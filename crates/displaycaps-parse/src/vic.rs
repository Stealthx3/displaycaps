// SPDX-License-Identifier: LGPL-2.1-or-later
//! CTA-861 Video Identification Codes and HDMI VICs: the timing each code stands for.
//! Filled in phase 2 (task 2.1).

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
    /// Total lines per frame.
    pub v_total: u16,
}

/// The timing of a CTA-861 VIC (1–127 and 193–219), or `None` for reserved codes.
pub fn vic(_code: u8) -> Option<VicTiming> {
    None
}

/// The timing of an HDMI VIC (1–4).
pub fn hdmi_vic(_code: u8) -> Option<VicTiming> {
    None
}
