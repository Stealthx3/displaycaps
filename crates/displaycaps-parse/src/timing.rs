// SPDX-License-Identifier: LGPL-2.1-or-later
//! Video timings shared by every EDID structure: detailed timing descriptors (base block, CTA-861
//! and DisplayID type I share the layout), the base block's video section (input definition,
//! established, standard and detailed timings, display descriptors including range limits and
//! CVT 3-byte codes), the VESA DMT table, and the CVT and GTF formulas that give standard timings
//! a pixel clock. Pure functions over bytes; never panics on any input.

/// A detailed timing (18-byte descriptor with a non-zero pixel clock).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetailedTiming {
    /// Pixel clock in kHz (the descriptor stores units of 10 kHz).
    pub pixel_clock_khz: u32,
    /// Horizontal active pixels.
    pub h_active: u16,
    /// Horizontal blanking pixels.
    pub h_blank: u16,
    /// Horizontal front porch.
    pub h_front: u16,
    /// Horizontal sync width.
    pub h_sync: u16,
    /// Vertical active lines (per field for interlaced timings).
    pub v_active: u16,
    /// Vertical blanking lines.
    pub v_blank: u16,
    /// Vertical front porch.
    pub v_front: u16,
    /// Vertical sync width.
    pub v_sync: u16,
    /// Image width in mm.
    pub h_size_mm: u16,
    /// Image height in mm.
    pub v_size_mm: u16,
    /// Horizontal border pixels.
    pub h_border: u8,
    /// Vertical border lines.
    pub v_border: u8,
    /// Interlaced.
    pub interlaced: bool,
    /// Stereo mode bits (0 = none).
    pub stereo: u8,
    /// Horizontal sync polarity is positive, for digital separate sync.
    pub h_sync_positive: Option<bool>,
    /// Vertical sync polarity is positive, for digital separate sync.
    pub v_sync_positive: Option<bool>,
}

impl DetailedTiming {
    /// Total pixels per line.
    pub fn h_total(&self) -> u32 {
        u32::from(self.h_active) + u32::from(self.h_blank)
    }

    /// Total lines per frame (both fields for interlaced timings).
    pub fn v_total_frame(&self) -> u32 {
        let v = u32::from(self.v_active) + u32::from(self.v_blank);
        if self.interlaced { 2 * v + 1 } else { v }
    }

    /// Active lines per frame (doubled for interlaced timings).
    pub fn height(&self) -> u32 {
        u32::from(self.v_active) * if self.interlaced { 2 } else { 1 }
    }

    /// Vertical refresh (field rate for interlaced) in millihertz, rounded to nearest.
    pub fn refresh_millihz(&self) -> u32 {
        let h = u64::from(self.h_total());
        let v = u64::from(u32::from(self.v_active) + u32::from(self.v_blank));
        if h == 0 || v == 0 {
            return 0;
        }
        let num = u64::from(self.pixel_clock_khz) * 1_000_000;
        let den = h * v;
        let field = (num + den / 2) / den;
        // Interlaced: two fields of (v + 0.5) lines each per frame; field rate = clock / (h * (v + 0.5)).
        if self.interlaced {
            let den = h * (2 * v + 1);
            ((2 * num + den / 2) / den) as u32
        } else {
            field as u32
        }
    }
}

/// Decodes an 18-byte detailed timing descriptor. `None` when the pixel clock is 0 (a display
/// descriptor) or the slice is shorter than 18 bytes.
pub fn parse_dtd(d: &[u8]) -> Option<DetailedTiming> {
    let d = d.get(..18)?;
    let clock = u16::from_le_bytes([d[0], d[1]]);
    if clock == 0 {
        return None;
    }
    let hi = |b: u8, shift: u8| u16::from((b >> shift) & 0x0F) << 8;
    let h_active = u16::from(d[2]) | hi(d[4], 4);
    let h_blank = u16::from(d[3]) | hi(d[4], 0);
    let v_active = u16::from(d[5]) | hi(d[7], 4);
    let v_blank = u16::from(d[6]) | hi(d[7], 0);
    let h_front = u16::from(d[8]) | (u16::from((d[11] >> 6) & 0x03) << 8);
    let h_sync = u16::from(d[9]) | (u16::from((d[11] >> 4) & 0x03) << 8);
    let v_front = u16::from(d[10] >> 4) | (u16::from((d[11] >> 2) & 0x03) << 4);
    let v_sync = u16::from(d[10] & 0x0F) | (u16::from(d[11] & 0x03) << 4);
    let h_size_mm = u16::from(d[12]) | hi(d[14], 4);
    let v_size_mm = u16::from(d[13]) | hi(d[14], 0);
    let flags = d[17];
    let (h_pos, v_pos) = if flags & 0x18 == 0x18 {
        // Digital separate sync: bit 2 vertical polarity, bit 1 horizontal polarity.
        (Some(flags & 0x02 != 0), Some(flags & 0x04 != 0))
    } else {
        (None, None)
    };
    Some(DetailedTiming {
        pixel_clock_khz: u32::from(clock) * 10,
        h_active,
        h_blank,
        h_front,
        h_sync,
        v_active,
        v_blank,
        v_front,
        v_sync,
        h_size_mm,
        v_size_mm,
        h_border: d[15],
        v_border: d[16],
        interlaced: flags & 0x80 != 0,
        stereo: ((flags >> 4) & 0x06) | (flags & 0x01),
        h_sync_positive: h_pos,
        v_sync_positive: v_pos,
    })
}

/// The base block's video section. Filled by [`parse_base_video`] (phase 2, task 2.1).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BaseVideo {
    /// Detailed timings from the four descriptor slots, in order (the first is preferred).
    pub detailed: Vec<DetailedTiming>,
}

/// Decodes the video section of a 128-byte EDID base block.
pub fn parse_base_video(base: &[u8]) -> BaseVideo {
    let mut v = BaseVideo::default();
    for off in [54usize, 72, 90, 108] {
        if let Some(t) = base.get(off..off + 18).and_then(parse_dtd) {
            v.detailed.push(t);
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cea_1080p60_dtd() {
        // 1920x1080@60, 148.5 MHz (CTA-861 VIC 16 timing).
        let d = [
            0x02, 0x3A, 0x80, 0x18, 0x71, 0x38, 0x2D, 0x40, 0x58, 0x2C, 0x45, 0x00, 0x40, 0x84,
            0x63, 0x00, 0x00, 0x1E,
        ];
        let t = parse_dtd(&d).unwrap();
        assert_eq!(t.pixel_clock_khz, 148_500);
        assert_eq!(
            (t.h_active, t.h_blank, t.v_active, t.v_blank),
            (1920, 280, 1080, 45)
        );
        assert_eq!((t.h_front, t.h_sync, t.v_front, t.v_sync), (88, 44, 4, 5));
        assert_eq!(t.refresh_millihz(), 60_000);
        assert_eq!(
            (t.h_sync_positive, t.v_sync_positive),
            (Some(true), Some(true))
        );
        assert!(parse_dtd(&[0u8; 18]).is_none());
        assert!(parse_dtd(&d[..10]).is_none());
    }
}
