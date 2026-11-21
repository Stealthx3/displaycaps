// SPDX-License-Identifier: LGPL-2.1-or-later
//! CTA-861-H Video Identification Codes (1-127 and 193-219) and HDMI VICs (1-4): the timing each
//! code stands for. The 1000/1001 rates (23.976, 29.97, 59.94 Hz and so on) share their code with
//! the integer rate and appear here at their nominal (integer) rate and clock, except the codes
//! the standard defines only at the fractional rate (VIC 1-3 and the other 480/576-line codes).

/// The timing a VIC stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VicTiming {
    /// Active width.
    pub width: u16,
    /// Active height (per frame).
    pub height: u16,
    /// Nominal refresh in millihertz (field rate for interlaced codes).
    pub refresh_millihz: u32,
    /// Interlaced.
    pub interlaced: bool,
    /// Pixel clock in kHz at the nominal refresh.
    pub pixel_clock_khz: u32,
    /// Total pixels per line.
    pub h_total: u16,
    /// Total lines per frame (both fields and the half line if interlaced).
    pub v_total: u16,
}

impl VicTiming {
    /// The pixel clock of the 1000/1001 variant of this code (used by a source at 23.976, 29.97,
    /// 59.94 Hz and so on), in kHz, rounded to nearest.
    pub fn pixel_clock_khz_1001(&self) -> u32 {
        ((u64::from(self.pixel_clock_khz) * 1000 + 500) / 1001) as u32
    }
}

/// (VIC, width, height, interlaced, pixel clock kHz, h total, v total per frame).
type Row = (u8, u16, u16, bool, u32, u16, u16);

static TABLE: [Row; 154] = [
    (1, 640, 480, false, 25175, 800, 525),
    (2, 720, 480, false, 27000, 858, 525),
    (3, 720, 480, false, 27000, 858, 525),
    (4, 1280, 720, false, 74250, 1650, 750),
    (5, 1920, 1080, true, 74250, 2200, 1125),
    (6, 1440, 480, true, 27000, 1716, 525),
    (7, 1440, 480, true, 27000, 1716, 525),
    (8, 1440, 240, false, 27000, 1716, 262),
    (9, 1440, 240, false, 27000, 1716, 262),
    (10, 2880, 480, true, 54000, 3432, 525),
    (11, 2880, 480, true, 54000, 3432, 525),
    (12, 2880, 240, false, 54000, 3432, 262),
    (13, 2880, 240, false, 54000, 3432, 262),
    (14, 1440, 480, false, 54000, 1716, 525),
    (15, 1440, 480, false, 54000, 1716, 525),
    (16, 1920, 1080, false, 148500, 2200, 1125),
    (17, 720, 576, false, 27000, 864, 625),
    (18, 720, 576, false, 27000, 864, 625),
    (19, 1280, 720, false, 74250, 1980, 750),
    (20, 1920, 1080, true, 74250, 2640, 1125),
    (21, 1440, 576, true, 27000, 1728, 625),
    (22, 1440, 576, true, 27000, 1728, 625),
    (23, 1440, 288, false, 27000, 1728, 312),
    (24, 1440, 288, false, 27000, 1728, 312),
    (25, 2880, 576, true, 54000, 3456, 625),
    (26, 2880, 576, true, 54000, 3456, 625),
    (27, 2880, 288, false, 54000, 3456, 312),
    (28, 2880, 288, false, 54000, 3456, 312),
    (29, 1440, 576, false, 54000, 1728, 625),
    (30, 1440, 576, false, 54000, 1728, 625),
    (31, 1920, 1080, false, 148500, 2640, 1125),
    (32, 1920, 1080, false, 74250, 2750, 1125),
    (33, 1920, 1080, false, 74250, 2640, 1125),
    (34, 1920, 1080, false, 74250, 2200, 1125),
    (35, 2880, 480, false, 108000, 3432, 525),
    (36, 2880, 480, false, 108000, 3432, 525),
    (37, 2880, 576, false, 108000, 3456, 625),
    (38, 2880, 576, false, 108000, 3456, 625),
    // VIC 39 is the CTA-861 exception: two 625-line fields, so 1250 lines per frame, not 2v+1.
    (39, 1920, 1080, true, 72000, 2304, 1250),
    (40, 1920, 1080, true, 148500, 2640, 1125),
    (41, 1280, 720, false, 148500, 1980, 750),
    (42, 720, 576, false, 54000, 864, 625),
    (43, 720, 576, false, 54000, 864, 625),
    (44, 1440, 576, true, 54000, 1728, 625),
    (45, 1440, 576, true, 54000, 1728, 625),
    (46, 1920, 1080, true, 148500, 2200, 1125),
    (47, 1280, 720, false, 148500, 1650, 750),
    (48, 720, 480, false, 54000, 858, 525),
    (49, 720, 480, false, 54000, 858, 525),
    (50, 1440, 480, true, 54000, 1716, 525),
    (51, 1440, 480, true, 54000, 1716, 525),
    (52, 720, 576, false, 108000, 864, 625),
    (53, 720, 576, false, 108000, 864, 625),
    (54, 1440, 576, true, 108000, 1728, 625),
    (55, 1440, 576, true, 108000, 1728, 625),
    (56, 720, 480, false, 108000, 858, 525),
    (57, 720, 480, false, 108000, 858, 525),
    (58, 1440, 480, true, 108000, 1716, 525),
    (59, 1440, 480, true, 108000, 1716, 525),
    (60, 1280, 720, false, 59400, 3300, 750),
    (61, 1280, 720, false, 74250, 3960, 750),
    (62, 1280, 720, false, 74250, 3300, 750),
    (63, 1920, 1080, false, 297000, 2200, 1125),
    (64, 1920, 1080, false, 297000, 2640, 1125),
    (65, 1280, 720, false, 59400, 3300, 750),
    (66, 1280, 720, false, 74250, 3960, 750),
    (67, 1280, 720, false, 74250, 3300, 750),
    (68, 1280, 720, false, 74250, 1980, 750),
    (69, 1280, 720, false, 74250, 1650, 750),
    (70, 1280, 720, false, 148500, 1980, 750),
    (71, 1280, 720, false, 148500, 1650, 750),
    (72, 1920, 1080, false, 74250, 2750, 1125),
    (73, 1920, 1080, false, 74250, 2640, 1125),
    (74, 1920, 1080, false, 74250, 2200, 1125),
    (75, 1920, 1080, false, 148500, 2640, 1125),
    (76, 1920, 1080, false, 148500, 2200, 1125),
    (77, 1920, 1080, false, 297000, 2640, 1125),
    (78, 1920, 1080, false, 297000, 2200, 1125),
    (79, 1680, 720, false, 59400, 3300, 750),
    (80, 1680, 720, false, 59400, 3168, 750),
    (81, 1680, 720, false, 59400, 2640, 750),
    (82, 1680, 720, false, 82500, 2200, 750),
    (83, 1680, 720, false, 99000, 2200, 750),
    (84, 1680, 720, false, 165000, 2000, 825),
    (85, 1680, 720, false, 198000, 2000, 825),
    (86, 2560, 1080, false, 99000, 3750, 1100),
    (87, 2560, 1080, false, 90000, 3200, 1125),
    (88, 2560, 1080, false, 118800, 3520, 1125),
    (89, 2560, 1080, false, 185625, 3300, 1125),
    (90, 2560, 1080, false, 198000, 3000, 1100),
    (91, 2560, 1080, false, 371250, 2970, 1250),
    (92, 2560, 1080, false, 495000, 3300, 1250),
    (93, 3840, 2160, false, 297000, 5500, 2250),
    (94, 3840, 2160, false, 297000, 5280, 2250),
    (95, 3840, 2160, false, 297000, 4400, 2250),
    (96, 3840, 2160, false, 594000, 5280, 2250),
    (97, 3840, 2160, false, 594000, 4400, 2250),
    (98, 4096, 2160, false, 297000, 5500, 2250),
    (99, 4096, 2160, false, 297000, 5280, 2250),
    (100, 4096, 2160, false, 297000, 4400, 2250),
    (101, 4096, 2160, false, 594000, 5280, 2250),
    (102, 4096, 2160, false, 594000, 4400, 2250),
    (103, 3840, 2160, false, 297000, 5500, 2250),
    (104, 3840, 2160, false, 297000, 5280, 2250),
    (105, 3840, 2160, false, 297000, 4400, 2250),
    (106, 3840, 2160, false, 594000, 5280, 2250),
    (107, 3840, 2160, false, 594000, 4400, 2250),
    (108, 1280, 720, false, 90000, 2500, 750),
    (109, 1280, 720, false, 90000, 2500, 750),
    (110, 1680, 720, false, 99000, 2750, 750),
    (111, 1920, 1080, false, 148500, 2750, 1125),
    (112, 1920, 1080, false, 148500, 2750, 1125),
    (113, 2560, 1080, false, 198000, 3750, 1100),
    (114, 3840, 2160, false, 594000, 5500, 2250),
    (115, 4096, 2160, false, 594000, 5500, 2250),
    (116, 3840, 2160, false, 594000, 5500, 2250),
    (117, 3840, 2160, false, 1188000, 5280, 2250),
    (118, 3840, 2160, false, 1188000, 4400, 2250),
    (119, 3840, 2160, false, 1188000, 5280, 2250),
    (120, 3840, 2160, false, 1188000, 4400, 2250),
    (121, 5120, 2160, false, 396000, 7500, 2200),
    (122, 5120, 2160, false, 396000, 7200, 2200),
    (123, 5120, 2160, false, 396000, 6000, 2200),
    (124, 5120, 2160, false, 742500, 6250, 2475),
    (125, 5120, 2160, false, 742500, 6600, 2250),
    (126, 5120, 2160, false, 742500, 5500, 2250),
    (127, 5120, 2160, false, 1485000, 6600, 2250),
    (193, 5120, 2160, false, 1485000, 5500, 2250),
    (194, 7680, 4320, false, 1188000, 11000, 4500),
    (195, 7680, 4320, false, 1188000, 10800, 4400),
    (196, 7680, 4320, false, 1188000, 9000, 4400),
    (197, 7680, 4320, false, 2376000, 11000, 4500),
    (198, 7680, 4320, false, 2376000, 10800, 4400),
    (199, 7680, 4320, false, 2376000, 9000, 4400),
    (200, 7680, 4320, false, 4752000, 10560, 4500),
    (201, 7680, 4320, false, 4752000, 8800, 4500),
    (202, 7680, 4320, false, 1188000, 11000, 4500),
    (203, 7680, 4320, false, 1188000, 10800, 4400),
    (204, 7680, 4320, false, 1188000, 9000, 4400),
    (205, 7680, 4320, false, 2376000, 11000, 4500),
    (206, 7680, 4320, false, 2376000, 10800, 4400),
    (207, 7680, 4320, false, 2376000, 9000, 4400),
    (208, 7680, 4320, false, 4752000, 10560, 4500),
    (209, 7680, 4320, false, 4752000, 8800, 4500),
    (210, 10240, 4320, false, 1485000, 12500, 4950),
    (211, 10240, 4320, false, 1485000, 13500, 4400),
    (212, 10240, 4320, false, 1485000, 11000, 4500),
    (213, 10240, 4320, false, 2970000, 12500, 4950),
    (214, 10240, 4320, false, 2970000, 13500, 4400),
    (215, 10240, 4320, false, 2970000, 11000, 4500),
    (216, 10240, 4320, false, 5940000, 13200, 4500),
    (217, 10240, 4320, false, 5940000, 11000, 4500),
    (218, 4096, 2160, false, 1188000, 5280, 2250),
    (219, 4096, 2160, false, 1188000, 4400, 2250),
];

/// The timing of a CTA-861 VIC (1-127 and 193-219), or `None` for reserved codes.
pub fn vic(code: u8) -> Option<VicTiming> {
    let &(_, width, height, interlaced, pixel_clock_khz, h_total, v_total) =
        TABLE.iter().find(|r| r.0 == code)?;
    let den = u64::from(h_total) * u64::from(v_total);
    let fields: u64 = if interlaced { 2 } else { 1 };
    let refresh_millihz =
        ((u64::from(pixel_clock_khz) * 1_000_000 * fields + den / 2) / den) as u32;
    Some(VicTiming {
        width,
        height,
        refresh_millihz,
        interlaced,
        pixel_clock_khz,
        h_total,
        v_total,
    })
}

/// The timing of an HDMI VIC (1-4): 3840x2160 at 30, 25 and 24 Hz, and 4096x2160 at 24 Hz.
pub fn hdmi_vic(code: u8) -> Option<VicTiming> {
    match code {
        1 => vic(95),
        2 => vic(94),
        3 => vic(93),
        4 => vic(98),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vic_39_is_exactly_50_hz() {
        let t = vic(39).unwrap();
        assert_eq!((t.width, t.height, t.interlaced, t.v_total), (1920, 1080, true, 1250));
        assert_eq!(t.refresh_millihz, 50_000);
    }

    #[test]
    fn table_is_complete() {
        assert_eq!(
            TABLE.iter().filter(|r| (1..=127).contains(&r.0)).count(),
            127
        );
        assert_eq!(
            TABLE.iter().filter(|r| (193..=219).contains(&r.0)).count(),
            27
        );
        for c in (1..=127u8).chain(193..=219) {
            assert!(vic(c).is_some(), "VIC {c}");
        }
        for c in [0u8, 128, 150, 192, 220, 255] {
            assert!(vic(c).is_none(), "VIC {c}");
        }
    }

    #[test]
    fn known_codes() {
        let t = vic(16).unwrap();
        assert_eq!((t.width, t.height, t.refresh_millihz), (1920, 1080, 60_000));
        assert_eq!(
            (t.pixel_clock_khz, t.h_total, t.v_total),
            (148_500, 2200, 1125)
        );
        assert!(!t.interlaced);
        assert_eq!(t.pixel_clock_khz_1001(), 148_352);
        let t = vic(5).unwrap();
        assert!(t.interlaced);
        assert_eq!(
            (t.width, t.height, t.refresh_millihz, t.v_total),
            (1920, 1080, 60_000, 1125)
        );
        let t = vic(1).unwrap();
        assert_eq!((t.width, t.height, t.refresh_millihz), (640, 480, 59_940));
        let t = vic(97).unwrap();
        assert_eq!((t.width, t.height, t.refresh_millihz), (3840, 2160, 60_000));
        assert_eq!(t.pixel_clock_khz, 594_000);
        let t = vic(118).unwrap();
        assert_eq!(
            (t.width, t.height, t.refresh_millihz),
            (3840, 2160, 120_000)
        );
        assert_eq!(t.pixel_clock_khz, 1_188_000);
        let t = vic(199).unwrap();
        assert_eq!((t.width, t.height), (7680, 4320));
        assert_eq!(t.pixel_clock_khz, 2_376_000);
        let t = vic(93).unwrap();
        assert_eq!((t.width, t.height, t.refresh_millihz), (3840, 2160, 24_000));
    }

    #[test]
    fn hdmi_vics() {
        let t = hdmi_vic(1).unwrap();
        assert_eq!((t.width, t.height, t.refresh_millihz), (3840, 2160, 30_000));
        assert_eq!(hdmi_vic(2).unwrap().refresh_millihz, 25_000);
        assert_eq!(hdmi_vic(3).unwrap().refresh_millihz, 24_000);
        let t = hdmi_vic(4).unwrap();
        assert_eq!((t.width, t.height, t.refresh_millihz), (4096, 2160, 24_000));
        assert!(hdmi_vic(0).is_none() && hdmi_vic(5).is_none());
    }
}
