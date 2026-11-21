// SPDX-License-Identifier: LGPL-2.1-or-later
//! Timing formulas that give a standard timing, CVT timing code or range-limited mode a pixel
//! clock and blanking: VESA CVT 1.2 (standard blanking, reduced blanking v1 and v2 including the
//! 1000/1001 video-optimised variant) and VESA GTF (default and secondary curve). Pure arithmetic;
//! nonsensical inputs return `None`, never panic.

/// The blanking style of a computed or tabulated timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Blanking {
    /// CVT standard blanking (also used for GTF, DMT non-reduced and CTA timings).
    #[default]
    Standard,
    /// CVT reduced blanking v1 (160 pixel blank, 460 us minimum vertical blank).
    ReducedV1,
    /// CVT reduced blanking v2 (80 pixel blank, fixed 8-line sync, 1 line front porch).
    ReducedV2,
    /// CVT reduced blanking v2 video-optimised: the v2 pixel clock times 1000/1001.
    ReducedV2Video,
}

/// A timing computed by [`cvt`] or [`gtf`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CalcTiming {
    /// Pixel clock in kHz.
    pub pixel_clock_khz: u32,
    /// Active pixels per line (rounded to the formula's cell granularity), excluding borders.
    pub h_active: u32,
    /// Total pixels per line, borders included.
    pub h_total: u32,
    /// Horizontal front porch (negative only for degenerate GTF results).
    pub h_front: i32,
    /// Horizontal sync width.
    pub h_sync: u32,
    /// Horizontal back porch.
    pub h_back: i32,
    /// Active lines per frame, borders excluded.
    pub v_active: u32,
    /// Total lines per frame, borders included (both fields, plus the half line, if interlaced).
    pub v_total: u32,
    /// Vertical front porch (per field).
    pub v_front: u32,
    /// Vertical sync width.
    pub v_sync: u32,
    /// Vertical back porch (per field).
    pub v_back: u32,
    /// Left and right border pixels (each side).
    pub h_border: u32,
    /// Top and bottom border lines (each side).
    pub v_border: u32,
    /// Interlaced.
    pub interlaced: bool,
    /// Blanking style (`Standard` for GTF).
    pub blanking: Blanking,
}

impl CalcTiming {
    /// Vertical refresh (field rate if interlaced) in millihertz, rounded to nearest.
    pub fn refresh_millihz(&self) -> u32 {
        let den = u64::from(self.h_total) * u64::from(self.v_total);
        if den == 0 {
            return 0;
        }
        let fields: u64 = if self.interlaced { 2 } else { 1 };
        let num = u64::from(self.pixel_clock_khz) * 1_000_000 * fields;
        ((num + den / 2) / den) as u32
    }
}

const CELL_GRAN: f64 = 8.0;
const MARGIN_PERC: f64 = 1.8;
const MAX_DIM: u32 = 65_535;

fn valid_request(h: u32, v: u32, refresh_hz: f64) -> bool {
    (1..=MAX_DIM).contains(&h)
        && (1..=MAX_DIM).contains(&v)
        && (1.0..=10_000.0).contains(&refresh_hz)
}

fn finish(t: CalcTiming, clock_khz: f64) -> Option<CalcTiming> {
    if !(1.0..=4_000_000_000.0).contains(&clock_khz) {
        return None;
    }
    if t.h_total == 0 || t.v_total == 0 || t.h_total > 1_000_000 || t.v_total > 1_000_000 {
        return None;
    }
    Some(t)
}

/// The parameters of the GTF blanking-duty-cycle curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GtfParams {
    /// C, in percent.
    pub c: f64,
    /// M, in percent per kHz.
    pub m: f64,
    /// K.
    pub k: f64,
    /// J, in percent.
    pub j: f64,
}

impl GtfParams {
    /// The default (primary) curve: C 40, M 600, K 128, J 20.
    pub const DEFAULT: GtfParams = GtfParams {
        c: 40.0,
        m: 600.0,
        k: 128.0,
        j: 20.0,
    };

    /// The secondary curve of a range-limits descriptor, from its raw bytes: `c_half` is C in
    /// half percent, `m` is M in percent per kHz, `k` is K and `j_half` is J in half percent.
    pub fn secondary(c_half: u8, m: u16, k: u8, j_half: u8) -> GtfParams {
        GtfParams {
            c: f64::from(c_half) / 2.0,
            m: f64::from(m),
            k: f64::from(k),
            j: f64::from(j_half) / 2.0,
        }
    }
}

/// VESA GTF timing for a vertical refresh rate (`refresh_hz` is the frame rate; for interlaced
/// timings the field rate is double). `margins` adds the 1.8 % borders. `None` for impossible input.
pub fn gtf(
    h_active: u32,
    v_active: u32,
    refresh_hz: f64,
    interlaced: bool,
    margins: bool,
    p: GtfParams,
) -> Option<CalcTiming> {
    if !valid_request(h_active, v_active, refresh_hz) {
        return None;
    }
    let c_prime = ((p.c - p.j) * p.k / 256.0) + p.j;
    let m_prime = p.k / 256.0 * p.m;
    let h_rnd = (f64::from(h_active) / CELL_GRAN).round() * CELL_GRAN;
    let v_rnd = if interlaced {
        (f64::from(v_active) / 2.0).round()
    } else {
        f64::from(v_active)
    };
    let h_margin = if margins {
        (h_rnd * MARGIN_PERC / 100.0 / CELL_GRAN).round() * CELL_GRAN
    } else {
        0.0
    };
    let v_margin = if margins {
        (MARGIN_PERC / 100.0 * v_rnd).round()
    } else {
        0.0
    };
    let interlace = if interlaced { 0.5 } else { 0.0 };
    let total_active = h_rnd + h_margin * 2.0;
    let field_rate = if interlaced {
        refresh_hz * 2.0
    } else {
        refresh_hz
    };
    let h_period_est = ((1.0 / field_rate) - 550.0 / 1_000_000.0)
        / (v_rnd + v_margin * 2.0 + 1.0 + interlace)
        * 1_000_000.0;
    if !(h_period_est.is_finite() && h_period_est > 0.0) {
        return None;
    }
    let v_sync_bp = (550.0 / h_period_est).round();
    let total_v = v_rnd + v_margin * 2.0 + v_sync_bp + interlace + 1.0;
    let field_rate_est = 1.0 / h_period_est / total_v * 1_000_000.0;
    let h_period = h_period_est / (field_rate / field_rate_est);
    let duty = c_prime - (m_prime * h_period / 1000.0);
    if !(duty.is_finite() && duty < 100.0) {
        return None;
    }
    let h_blank =
        (total_active * duty / (100.0 - duty) / (2.0 * CELL_GRAN)).round() * 2.0 * CELL_GRAN;
    let total_pixels = total_active + h_blank;
    let pixel_freq = total_pixels / h_period;
    if !(h_blank.is_finite() && h_blank >= 0.0 && pixel_freq.is_finite()) {
        return None;
    }
    let h_sync = (8.0 / 100.0 * total_pixels / CELL_GRAN).round() * CELL_GRAN;
    let h_front = h_blank / 2.0 - h_sync;
    let field_lines = v_rnd + v_margin * 2.0 + v_sync_bp + 1.0;
    let v_total = if interlaced {
        2.0 * field_lines + 1.0
    } else {
        field_lines
    };
    let clock_khz = (1000.0 * pixel_freq).round();
    finish(
        CalcTiming {
            pixel_clock_khz: clock_khz as u32,
            h_active: h_rnd as u32,
            h_total: total_pixels as u32,
            h_front: h_front as i32,
            h_sync: h_sync as u32,
            h_back: (h_front + h_sync) as i32,
            v_active,
            v_total: v_total as u32,
            v_front: 1,
            v_sync: 3,
            v_back: (v_sync_bp - 3.0).max(0.0) as u32,
            h_border: h_margin as u32,
            v_border: v_margin as u32,
            interlaced,
            blanking: Blanking::Standard,
        },
        clock_khz,
    )
}

/// GTF with the default curve, progressive, no margins.
pub fn gtf_default(h_active: u32, v_active: u32, refresh_hz: f64) -> Option<CalcTiming> {
    gtf(
        h_active,
        v_active,
        refresh_hz,
        false,
        false,
        GtfParams::DEFAULT,
    )
}

/// VESA CVT 1.2 timing for a vertical refresh rate (frame rate; field rate is double if
/// interlaced). `margins` adds the 1.8 % borders. `None` for impossible input.
pub fn cvt(
    h_active: u32,
    v_active: u32,
    refresh_hz: f64,
    blanking: Blanking,
    interlaced: bool,
    margins: bool,
) -> Option<CalcTiming> {
    if !valid_request(h_active, v_active, refresh_hz) {
        return None;
    }
    let v2 = matches!(blanking, Blanking::ReducedV2 | Blanking::ReducedV2Video);
    let cell = if v2 { 1.0 } else { CELL_GRAN };
    let h_rnd = (f64::from(h_active) / cell).floor() * cell;
    let v_rnd = if interlaced {
        (f64::from(v_active) / 2.0).floor()
    } else {
        f64::from(v_active)
    };
    let h_margin = if margins {
        ((h_rnd * MARGIN_PERC / 100.0) / cell).floor() * cell
    } else {
        0.0
    };
    let v_margin = if margins {
        (MARGIN_PERC / 100.0 * v_rnd).floor()
    } else {
        0.0
    };
    let interlace = if interlaced { 0.5 } else { 0.0 };
    let total_active = h_rnd + h_margin * 2.0;
    let field_rate = if interlaced {
        refresh_hz * 2.0
    } else {
        refresh_hz
    };
    let clock_step = if v2 { 0.001 } else { 0.25 };
    let mut h_blank = if blanking == Blanking::ReducedV1 {
        160.0
    } else {
        80.0
    };
    let rb_v_fporch = if blanking == Blanking::ReducedV1 {
        3.0
    } else {
        1.0
    };
    let multiplier = if blanking == Blanking::ReducedV2Video {
        1000.0 / 1001.0
    } else {
        1.0
    };
    let mut h_sync = 32.0;
    let ratio_sync = {
        let (va, ha) = (u64::from(v_active), u64::from(h_active));
        // The vertical sync width follows the aspect ratio (CVT 1.2 table 3-2).
        if va * 4 / 3 == ha {
            4.0
        } else if va * 16 / 9 == ha {
            5.0
        } else if va * 16 / 10 == ha {
            6.0
        } else if (va % 4 == 0 && va * 5 / 4 == ha) || va * 15 / 9 == ha {
            7.0
        } else {
            10.0
        }
    };
    let v_sync = if v2 { 8.0 } else { ratio_sync };

    let (v_blank, v_sync_bp, pixel_freq) = if blanking == Blanking::Standard {
        let h_period_est = ((1.0 / field_rate) - 550.0 / 1_000_000.0)
            / (v_rnd + v_margin * 2.0 + 3.0 + interlace)
            * 1_000_000.0;
        if !(h_period_est.is_finite() && h_period_est > 0.0) {
            return None;
        }
        let mut v_sync_bp = (550.0 / h_period_est).floor() + 1.0;
        if v_sync_bp < v_sync + 7.0 {
            v_sync_bp = v_sync + 7.0;
        }
        let v_blank = v_sync_bp + 3.0;
        let mut duty = 30.0 - (300.0 * h_period_est / 1000.0);
        if duty < 20.0 {
            duty = 20.0;
        }
        h_blank =
            (total_active * duty / (100.0 - duty) / (2.0 * CELL_GRAN)).floor() * 2.0 * CELL_GRAN;
        let total_pixels = total_active + h_blank;
        h_sync = (total_pixels * 0.08 / CELL_GRAN).floor() * CELL_GRAN;
        let freq = (total_pixels / h_period_est / clock_step).floor() * clock_step;
        (v_blank, v_sync_bp, freq)
    } else {
        let min_vblank = 460.0;
        let h_period_est = ((1_000_000.0 / field_rate) - min_vblank) / (v_rnd + v_margin * 2.0);
        if !(h_period_est.is_finite() && h_period_est > 0.0) {
            return None;
        }
        let vbi_lines = (min_vblank / h_period_est).floor() + 1.0;
        let v_bporch = if blanking == Blanking::ReducedV1 {
            7.0
        } else {
            6.0
        };
        let min_vbi = rb_v_fporch + v_sync + v_bporch;
        let v_blank = if vbi_lines < min_vbi {
            min_vbi
        } else {
            vbi_lines
        };
        let total_v = v_blank + v_rnd + v_margin * 2.0 + interlace;
        let v_sync_bp = if blanking == Blanking::ReducedV1 {
            v_blank - rb_v_fporch
        } else {
            v_sync + v_bporch
        };
        let total_pixels = h_blank + total_active;
        let freq = field_rate * total_v * total_pixels * multiplier;
        (
            v_blank,
            v_sync_bp,
            ((freq / 1_000_000.0) / clock_step).floor() * clock_step,
        )
    };

    let v_back = v_sync_bp - v_sync;
    let v_front = v_blank - v_back - v_sync;
    let h_front = if v2 { 8.0 } else { h_blank / 2.0 - h_sync };
    let h_back = h_blank - h_front - h_sync;
    let field_lines = v_blank + v_rnd + v_margin * 2.0;
    let v_total = if interlaced {
        2.0 * field_lines + 1.0
    } else {
        field_lines
    };
    let clock_khz = (1000.0 * pixel_freq).round();
    finish(
        CalcTiming {
            pixel_clock_khz: clock_khz as u32,
            h_active: h_rnd as u32,
            h_total: (h_blank + total_active) as u32,
            h_front: h_front as i32,
            h_sync: h_sync as u32,
            h_back: h_back as i32,
            v_active,
            v_total: v_total as u32,
            v_front: v_front.max(0.0) as u32,
            v_sync: v_sync as u32,
            v_back: v_back.max(0.0) as u32,
            h_border: h_margin as u32,
            v_border: v_margin as u32,
            interlaced,
            blanking,
        },
        clock_khz,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cvt_1080p60_reduced_v1() {
        let t = cvt(1920, 1080, 60.0, Blanking::ReducedV1, false, false).unwrap();
        assert_eq!(t.pixel_clock_khz, 138_500);
        assert_eq!((t.h_total, t.v_total), (2080, 1111));
        assert_eq!((t.h_front, t.h_sync, t.h_back), (48, 32, 80));
        assert_eq!((t.v_front, t.v_sync, t.v_back), (3, 5, 23));
    }

    #[test]
    fn cvt_1440p60_standard() {
        let t = cvt(2560, 1440, 60.0, Blanking::Standard, false, false).unwrap();
        assert_eq!(t.pixel_clock_khz, 312_250);
        assert_eq!((t.h_total, t.v_total), (3488, 1493));
    }

    #[test]
    fn cvt_1080p60_standard() {
        let t = cvt(1920, 1080, 60.0, Blanking::Standard, false, false).unwrap();
        assert_eq!(t.pixel_clock_khz, 173_000);
        assert_eq!((t.h_total, t.v_total), (2576, 1120));
    }

    #[test]
    fn cvt_reduced_v2_matches_dmt_4096x2160() {
        let t = cvt(4096, 2160, 60.0, Blanking::ReducedV2, false, false).unwrap();
        assert_eq!(
            (t.pixel_clock_khz, t.h_total, t.v_total),
            (556_744, 4176, 2222)
        );
        let t = cvt(4096, 2160, 60.0, Blanking::ReducedV2Video, false, false).unwrap();
        assert_eq!(t.pixel_clock_khz, 556_188);
    }

    #[test]
    fn cvt_reproduces_dmt_rows() {
        for id in [0x16u8, 0x1b, 0x29, 0x2e, 0x39, 0x44, 0x4c] {
            let d = crate::dmt::by_id(id).unwrap();
            let t = cvt(
                u32::from(d.width),
                u32::from(d.height),
                60.0,
                Blanking::ReducedV1,
                false,
                false,
            )
            .unwrap();
            assert_eq!(t.pixel_clock_khz, d.pixel_clock_khz, "DMT {id:#x}");
            assert_eq!(t.h_total, u32::from(d.h_total), "DMT {id:#x}");
            assert_eq!(t.v_total, u32::from(d.v_total), "DMT {id:#x}");
        }
        for id in [0x17u8, 0x45, 0x2a, 0x4d, 0x3a] {
            let d = crate::dmt::by_id(id).unwrap();
            let t = cvt(
                u32::from(d.width),
                u32::from(d.height),
                60.0,
                Blanking::Standard,
                false,
                false,
            )
            .unwrap();
            assert_eq!(t.pixel_clock_khz, d.pixel_clock_khz, "DMT {id:#x}");
        }
    }

    #[test]
    fn gtf_known_values() {
        let t = gtf_default(1280, 1024, 60.0).unwrap();
        assert_eq!(t.pixel_clock_khz, 108_883);
        assert_eq!((t.h_total, t.v_total), (1712, 1060));
        let t = gtf_default(1024, 768, 60.0).unwrap();
        assert_eq!(t.pixel_clock_khz, 64_109);
    }

    #[test]
    fn interlaced_and_margins_do_not_panic() {
        assert!(cvt(1920, 1080, 60.0, Blanking::Standard, true, true).is_some());
        assert!(gtf(1920, 1080, 60.0, true, true, GtfParams::DEFAULT).is_some());
    }

    #[test]
    fn nonsense_returns_none() {
        assert!(cvt(0, 1080, 60.0, Blanking::Standard, false, false).is_none());
        assert!(cvt(1920, 1080, 0.0, Blanking::ReducedV1, false, false).is_none());
        assert!(cvt(1920, 1080, f64::NAN, Blanking::Standard, false, false).is_none());
        assert!(gtf_default(u32::MAX, u32::MAX, 60.0).is_none());
        assert!(gtf_default(1920, 1080, 10_000.0).is_none());
        for r in [1.0, 5000.0] {
            let _ = cvt(8, 8, r, Blanking::ReducedV2, false, false);
            let _ = gtf_default(8, 8, r);
        }
    }
}
