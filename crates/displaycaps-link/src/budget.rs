// SPDX-License-Identifier: LGPL-2.1-or-later
//! Data rates: what a format needs on each kind of link, and what each link carries. All rates
//! are in kbit/s, clocks in kHz.

use displaycaps_model::{ColorFormat, DpLink, DpRate, FrlRate, Timing};

/// Bits per pixel, in 1/1000 bit, for a colour format at `bpc` bits per component.
pub fn millibits_per_pixel(format: ColorFormat, bpc: u8) -> u64 {
    let c = u64::from(bpc) * 1000;
    match format {
        ColorFormat::Rgb | ColorFormat::Ycbcr444 => 3 * c,
        ColorFormat::Ycbcr422 => 2 * c,
        ColorFormat::Ycbcr420 => 3 * c / 2,
    }
}

/// HDMI TMDS character rate in kHz for a pixel clock, format and depth. `None` when TMDS can't
/// carry it (4:2:2 above 12 bits).
///
/// - RGB and 4:4:4: the clock times `bpc / 8` (deep colour: 1.25×, 1.5×, 2×).
/// - 4:2:2: always 1×, up to 12 bits (carried in a 12-bit container).
/// - 4:2:0: half the 4:4:4 rate.
pub fn tmds_character_rate_khz(pixel_clock_khz: u32, format: ColorFormat, bpc: u8) -> Option<u32> {
    let clk = u64::from(pixel_clock_khz);
    let rate = match format {
        ColorFormat::Rgb | ColorFormat::Ycbcr444 => clk * u64::from(bpc) / 8,
        ColorFormat::Ycbcr422 if bpc <= 12 => clk,
        ColorFormat::Ycbcr422 => return None,
        ColorFormat::Ycbcr420 => clk * u64::from(bpc) / 16,
    };
    // A rate beyond u32 kHz is far past any TMDS limit: saturate rather than call it uncarriable.
    Some(u32::try_from(rate).unwrap_or(u32::MAX))
}

/// TMDS carries 8 bits of video per 10-bit character on each of 3 data channels, so a character
/// rate of `r` kHz carries `24 r` kbit/s of video data.
pub fn tmds_capacity_kbps(character_rate_khz: u32) -> u64 {
    u64::from(character_rate_khz) * 24
}

/// FRL coding: 16b/18b.
const FRL_CODING: (u64, u64) = (16, 18);
/// Allowance for RS-FEC, super-block and packet overhead, in 1/1000 of the coded rate.
const FRL_OVERHEAD_PERMILLE: u64 = 30;
/// Bits a blanking pixel period costs on FRL (blanking is carried as control and data-island
/// characters, not as pixels). Calibrated against the HDMI Forum's FRL cases: 4K60 10-bit RGB on
/// 3×6, and 4K120 RGB at 8, 10 and 12 bits on 32, 40 and 48 Gbit/s.
const FRL_BLANK_BITS: u64 = 4;

/// Bits per pixel on HDMI, in 1/1000 bit: as [`millibits_per_pixel`], but 4:2:2 always fills a
/// 24-bit container (up to 12 bits per component).
pub fn hdmi_millibits_per_pixel(format: ColorFormat, bpc: u8) -> u64 {
    match format {
        ColorFormat::Ycbcr422 => 24_000,
        f => millibits_per_pixel(f, bpc),
    }
}

/// Video data an FRL link carries, in kbit/s.
pub fn frl_capacity_kbps(rate: FrlRate) -> u64 {
    let raw = u64::from(rate.total_gbps()) * 1_000_000;
    raw * FRL_CODING.0 / FRL_CODING.1 * (1000 - FRL_OVERHEAD_PERMILLE) / 1000
}

/// Video data a mode needs on FRL, in kbit/s: active pixels at full depth, blanking at a reduced
/// cost per pixel period.
pub fn frl_required_kbps(
    t: &Timing,
    h_active: u16,
    v_active: u16,
    format: ColorFormat,
    bpc: u8,
) -> u64 {
    let (ha, va) = (u64::from(h_active), u64::from(v_active));
    let (ht, vt) = (u64::from(t.h_total).max(ha), u64::from(t.v_total).max(va));
    if ht == 0 || vt == 0 {
        return 0;
    }
    let (ha, va, ht, vt) = (
        u128::from(ha),
        u128::from(va),
        u128::from(ht),
        u128::from(vt),
    );
    let active_mbits = ha * va * u128::from(hdmi_millibits_per_pixel(format, bpc));
    let blank_mbits = (ht * vt - ha * va) * u128::from(FRL_BLANK_BITS) * 1000;
    // bits per frame × frames per second; frames per second = clock / (ht × vt).
    let kbps = (active_mbits + blank_mbits) * u128::from(t.pixel_clock_khz) / (ht * vt) / 1000;
    u64::try_from(kbps).unwrap_or(u64::MAX)
}

/// HDMI 2.1 FRL rates in ascending order (FRL levels 1 to 6).
pub const FRL_LEVELS: [FrlRate; 6] = [
    FrlRate {
        lanes: 3,
        gbps_per_lane: 3,
    },
    FrlRate {
        lanes: 3,
        gbps_per_lane: 6,
    },
    FrlRate {
        lanes: 4,
        gbps_per_lane: 6,
    },
    FrlRate {
        lanes: 4,
        gbps_per_lane: 8,
    },
    FrlRate {
        lanes: 4,
        gbps_per_lane: 10,
    },
    FrlRate {
        lanes: 4,
        gbps_per_lane: 12,
    },
];

/// The FRL level (1–6) of a rate, `None` for a rate that isn't one.
pub fn frl_level(rate: FrlRate) -> Option<usize> {
    FRL_LEVELS.iter().position(|r| *r == rate).map(|i| i + 1)
}

/// The highest FRL rate both ends support.
pub fn common_frl(a: FrlRate, b: FrlRate) -> Option<FrlRate> {
    let lvl = frl_level(a)?.min(frl_level(b)?);
    Some(FRL_LEVELS[lvl - 1])
}

/// Per-lane rate of a DisplayPort link rate, in kbit/s.
pub fn dp_lane_kbps(rate: DpRate) -> u64 {
    match rate {
        DpRate::Rbr => 1_620_000,
        DpRate::Hbr => 2_700_000,
        DpRate::Hbr2 => 5_400_000,
        DpRate::Hbr3 => 8_100_000,
        DpRate::Uhbr10 => 10_000_000,
        DpRate::Uhbr13_5 => 13_500_000,
        DpRate::Uhbr20 => 20_000_000,
    }
}

/// Video data a DisplayPort link carries, in kbit/s: 8b/10b coding up to HBR3, 128b/132b for
/// UHBR; MST loses one of 64 time slots.
pub fn dp_capacity_kbps(link: DpLink, mst: bool) -> u64 {
    let raw = dp_lane_kbps(link.rate) * u64::from(link.lanes);
    let coded = match link.rate {
        DpRate::Uhbr10 | DpRate::Uhbr13_5 | DpRate::Uhbr20 => raw * 128 / 132,
        _ => raw * 8 / 10,
    };
    if mst { coded * 63 / 64 } else { coded }
}

/// Video data a mode needs on DisplayPort (or as a plain stream rate), in kbit/s: the pixel clock
/// times bits per pixel.
pub fn stream_kbps(pixel_clock_khz: u32, format: ColorFormat, bpc: u8) -> u64 {
    u64::from(pixel_clock_khz) * millibits_per_pixel(format, bpc) / 1000
}

/// Stream rate on a link: HDMI carries 4:2:2 in a 24-bit container, DisplayPort packs it.
pub fn link_stream_kbps(pixel_clock_khz: u32, format: ColorFormat, bpc: u8, hdmi: bool) -> u64 {
    let bits = if hdmi {
        hdmi_millibits_per_pixel(format, bpc)
    } else {
        millibits_per_pixel(format, bpc)
    };
    u64::from(pixel_clock_khz) * bits / 1000
}

/// DSC: the highest compressed rate (in 1/16 bit per pixel) that fits `capacity`, capped below
/// the uncompressed rate. `None` when even the 8 bpp floor doesn't fit.
pub fn dsc_bpp_x16(
    pixel_clock_khz: u32,
    capacity_kbps: u64,
    format: ColorFormat,
    bpc: u8,
) -> Option<u32> {
    if pixel_clock_khz == 0 {
        return None;
    }
    let fit = capacity_kbps * 16 / u64::from(pixel_clock_khz);
    let uncompressed = millibits_per_pixel(format, bpc) * 16 / 1000;
    let floor = match format {
        ColorFormat::Ycbcr420 => 6 * 16,
        _ => 8 * 16,
    };
    let bpp = fit.min(uncompressed.saturating_sub(1));
    (bpp >= floor).then(|| u32::try_from(bpp).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ColorFormat::*;

    /// CTA-861 4K timings (VIC 97, VIC 117/118).
    fn t4k(clock: u32) -> Timing {
        Timing {
            pixel_clock_khz: clock,
            h_total: 4400,
            v_total: 2250,
            h_sync_positive: Some(true),
            v_sync_positive: Some(true),
        }
    }

    #[test]
    fn tmds_rates_match_the_plan_table() {
        // 4K 23.976 Hz 10-bit 4:4:4: 297 MHz × 1.25 ≈ 371 MHz, fits 600.
        assert_eq!(tmds_character_rate_khz(296_703, Rgb, 10), Some(370_878));
        // 4K60 8-bit 4:4:4 is exactly 594 MHz.
        assert_eq!(tmds_character_rate_khz(594_000, Rgb, 8), Some(594_000));
        // 4K60 10-bit 4:4:4 needs 742.5 MHz: over HDMI 2.0.
        assert_eq!(tmds_character_rate_khz(594_000, Rgb, 10), Some(742_500));
        // 4:2:2 12-bit and 4:2:0 10-bit fit.
        assert_eq!(
            tmds_character_rate_khz(594_000, Ycbcr422, 12),
            Some(594_000)
        );
        assert_eq!(
            tmds_character_rate_khz(594_000, Ycbcr420, 10),
            Some(371_250)
        );
        // 4K120 4:2:0 8-bit is 594 MHz.
        assert_eq!(
            tmds_character_rate_khz(1_188_000, Ycbcr420, 8),
            Some(594_000)
        );
        assert_eq!(tmds_character_rate_khz(594_000, Ycbcr422, 16), None);
    }

    #[test]
    fn frl_known_cases() {
        let l = |lanes, g| FrlRate {
            lanes,
            gbps_per_lane: g,
        };
        let fits = |clk, f, bpc, r| {
            frl_required_kbps(&t4k(clk), 3840, 2160, f, bpc) <= frl_capacity_kbps(r)
        };
        // 4K120 10-bit RGB fits 40 Gbit/s FRL (the 40G OLEDs do it); 12-bit needs 48.
        assert!(fits(1_188_000, Rgb, 10, l(4, 10)));
        assert!(!fits(1_188_000, Rgb, 12, l(4, 10)));
        assert!(fits(1_188_000, Rgb, 12, l(4, 12)));
        // 4K120 8-bit RGB fits 32 Gbit/s; 10-bit doesn't.
        assert!(fits(1_188_000, Rgb, 8, l(4, 8)));
        assert!(!fits(1_188_000, Rgb, 10, l(4, 8)));
        // 4K60 10-bit RGB fits 3×6 (18 Gbit/s), as in the HDMI Forum's FRL table.
        assert!(fits(594_000, Rgb, 10, l(3, 6)));
        // HDMI 4:2:2 fills a 24-bit container: forced 4:2:2 at 4K120 needs 4×8, not 4×6.
        assert!(!fits(1_188_000, Ycbcr422, 8, l(4, 6)));
        assert!(fits(1_188_000, Ycbcr422, 8, l(4, 8)));
        // Hostile timings don't overflow.
        let big = Timing {
            pixel_clock_khz: u32::MAX,
            h_total: u16::MAX,
            v_total: u16::MAX,
            h_sync_positive: None,
            v_sync_positive: None,
        };
        assert!(frl_required_kbps(&big, u16::MAX, u16::MAX, Rgb, 16) > 0);
        assert_eq!(tmds_character_rate_khz(u32::MAX, Rgb, 16), Some(u32::MAX));
        assert_eq!(common_frl(l(4, 12), l(3, 6)), Some(l(3, 6)));
        assert_eq!(frl_level(l(4, 10)), Some(5));
    }

    #[test]
    fn dp_known_cases() {
        let hbr2 = DpLink {
            lanes: 4,
            rate: DpRate::Hbr2,
        };
        let hbr3 = DpLink {
            lanes: 4,
            rate: DpRate::Hbr3,
        };
        assert_eq!(dp_capacity_kbps(hbr2, false), 17_280_000);
        assert_eq!(dp_capacity_kbps(hbr3, false), 25_920_000);
        assert_eq!(
            dp_capacity_kbps(
                DpLink {
                    lanes: 4,
                    rate: DpRate::Uhbr20
                },
                false
            ),
            77_575_757
        );
        // 4K60 8-bit RGB (594 MHz, 14.26 Gbit/s) fits HBR2; 10-bit with CTA timing doesn't.
        assert!(stream_kbps(594_000, Rgb, 8) <= dp_capacity_kbps(hbr2, false));
        assert!(stream_kbps(594_000, Rgb, 10) > dp_capacity_kbps(hbr2, false));
        // 4K144 10-bit RGB (~1300 MHz CVT-RB2) needs DSC on HBR3: DSC at about 19 bpp fits.
        assert!(stream_kbps(1_306_000, Rgb, 10) > dp_capacity_kbps(hbr3, false));
        let bpp = dsc_bpp_x16(1_306_000, dp_capacity_kbps(hbr3, false), Rgb, 10).unwrap();
        assert_eq!(bpp / 16, 19);
        assert!(
            dsc_bpp_x16(4_000_000, 25_920_000, Rgb, 10).is_none(),
            "below the 8 bpp floor"
        );
        assert!(dp_capacity_kbps(hbr2, true) < dp_capacity_kbps(hbr2, false));
    }
}
