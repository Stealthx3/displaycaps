// SPDX-License-Identifier: LGPL-2.1-or-later
//! DisplayID 1.3 and 2.x sections inside EDID extension blocks (tag 0x70): timing types, tiled
//! topology, display parameters, product identification, adaptive sync and interface features.
//!
//! [`parse_displayid`] takes one 128-byte EDID extension block (`blk[0] == 0x70`). The DisplayID
//! section starts at `blk[1]`: version, number of data-block bytes, product type (1.x) or primary
//! use case (2.x), extension count, then the data blocks, then the section checksum (summing
//! `blk[1..=5 + bytes]` gives 0). Every data block is recorded by tag, decoded when the crate
//! knows the layout. Timings of every kind also land in one flat list ([`DisplayId::timings`]).
//! The parser never panics: short or inconsistent input sets `truncated` / `malformed` flags and
//! keeps whatever decoded cleanly.

use crate::vic;

/// Largest value of the section's byte count that fits in one EDID block.
const MAX_SECTION_BYTES: usize = 121;

/// How a [`DidTiming`] was described in the section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DidTimingType {
    /// Type I detailed timing (data block 0x03, 10 kHz units).
    I,
    /// Type II detailed timing (data block 0x04).
    II,
    /// Type III short timing (data block 0x05, CVT formula).
    III,
    /// Type IV enumerated DMT/VIC code (data block 0x06).
    IV,
    /// Type V short timing (data block 0x11, CVT formula).
    V,
    /// Type VI detailed timing (data block 0x13, 1 kHz units).
    VI,
    /// Type VII detailed timing (data block 0x22, 1 kHz units).
    VII,
    /// Type VIII enumerated timing code (data block 0x23).
    VIII,
    /// Type IX formula-based timing (data block 0x24).
    IX,
    /// Type X formula-based timing (data block 0x2A).
    X,
    /// CTA-861 timing bitmap (data block 0x08).
    CtaBitmap,
    /// A timing code listed in a stereo interface block (0x10 or 0x27).
    StereoCode,
}

/// Kind of an enumerated timing code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DidCodeKind {
    /// VESA DMT ID.
    Dmt,
    /// CTA-861 VIC.
    Vic,
    /// HDMI VIC.
    HdmiVic,
}

/// One enumerated timing code (DMT ID, VIC or HDMI VIC), as listed, resolved or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DidTimingCode {
    /// What the code numbers.
    pub kind: DidCodeKind,
    /// The code.
    pub code: u16,
    /// The block type that listed it.
    pub timing_type: DidTimingType,
}

/// Blanking and sync details of a detailed timing (types I, II, VI, VII).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DidDetail {
    /// Horizontal blanking in pixels.
    pub h_blank: u32,
    /// Horizontal front porch in pixels.
    pub h_front: u32,
    /// Horizontal sync width in pixels.
    pub h_sync: u32,
    /// Vertical blanking in lines (per frame).
    pub v_blank: u32,
    /// Vertical front porch in lines.
    pub v_front: u32,
    /// Vertical sync width in lines.
    pub v_sync: u32,
    /// Horizontal sync polarity is positive.
    pub h_sync_positive: bool,
    /// Vertical sync polarity is positive.
    pub v_sync_positive: bool,
}

/// One timing from any DisplayID timing block, in a uniform shape.
///
/// Formula-based timings (types III, V, IX, X) carry the nominal refresh but no pixel clock or
/// totals (those are 0) because they depend on the CVT variant the source picks.
#[derive(Debug, Clone, PartialEq)]
pub struct DidTiming {
    /// Active width in pixels.
    pub width: u32,
    /// Active height in lines (per frame; 0 when an undefined aspect ratio hides it).
    pub height: u32,
    /// Refresh in millihertz (field rate for interlaced timings).
    pub refresh_millihz: u32,
    /// Interlaced.
    pub interlaced: bool,
    /// Marked as the preferred timing.
    pub preferred: bool,
    /// Pixel clock in kHz, 0 when not described by the timing.
    pub pixel_clock_khz: u32,
    /// Total pixels per line, 0 when not described by the timing.
    pub h_total: u32,
    /// Total lines per frame, 0 when not described by the timing.
    pub v_total: u32,
    /// Which timing type described it.
    pub timing_type: DidTimingType,
    /// Stereo: 0 none, 1 stereo, 2 stereo depends on user action, 3 reserved.
    pub stereo: u8,
    /// Aspect ratio as (horizontal, vertical) when the timing states one.
    pub aspect: Option<(u16, u16)>,
    /// Blanking and sync details, for detailed timings.
    pub detail: Option<DidDetail>,
}

/// Product identification (data block 0x00 or 0x20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidProductId {
    /// Vendor identifier bytes (IEEE OUI in 1.x, as stored).
    pub oui: [u8; 3],
    /// Product code.
    pub product_code: u16,
    /// Serial number (0 when unset).
    pub serial: u32,
    /// Week of manufacture, or 0xFF when `year` is a model year.
    pub week: u8,
    /// Year of manufacture (2000 + stored byte).
    pub year: u16,
    /// Product name (may be empty).
    pub name: String,
}

/// DisplayID 1.x display parameters (data block 0x01).
#[derive(Debug, Clone, PartialEq)]
pub struct DidParameters {
    /// Horizontal image size in 0.1 mm.
    pub width_dmm: u16,
    /// Vertical image size in 0.1 mm.
    pub height_dmm: u16,
    /// Horizontal pixel count.
    pub h_pixels: u16,
    /// Vertical pixel count.
    pub v_pixels: u16,
    /// Feature flags: bit 7 audio on video interface, 6 separate audio inputs, 5 audio input
    /// override, 4 power management, 3 fixed timing, 2 fixed pixel format, 1 ACP/ISRC packets,
    /// 0 de-interlacing.
    pub feature_flags: u8,
    /// Gamma, `None` when undefined.
    pub gamma: Option<f32>,
    /// Native pixel aspect ratio (width / height).
    pub aspect: f32,
    /// Bits per colour of the native display.
    pub bpc_native: u8,
    /// Bits per colour overall (dynamic range).
    pub bpc_overall: u8,
}

/// DisplayID 1.x colour characteristics (data block 0x02).
#[derive(Debug, Clone, PartialEq)]
pub struct DidColorCharacteristics {
    /// Coordinates are CIE 1976 (u', v') instead of CIE 1931 (x, y).
    pub cie_1976: bool,
    /// Temporal instead of spatial colour.
    pub temporal: bool,
    /// Associated transfer characteristics identifier (0 for none).
    pub transfer_id: u8,
    /// Standard colour space id when no primaries are listed.
    pub color_space: Option<u8>,
    /// Primaries (x, y).
    pub primaries: Vec<(f32, f32)>,
    /// White points (x, y).
    pub white_points: Vec<(f32, f32)>,
}

/// Video timing range limits (data block 0x09).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidRangeLimits {
    /// Minimum pixel clock in kHz.
    pub min_pixel_clock_khz: u32,
    /// Maximum pixel clock in kHz.
    pub max_pixel_clock_khz: u32,
    /// Minimum horizontal frequency in kHz.
    pub min_h_khz: u8,
    /// Maximum horizontal frequency in kHz.
    pub max_h_khz: u8,
    /// Minimum horizontal blanking in pixels.
    pub min_h_blank: u16,
    /// Minimum vertical refresh in Hz.
    pub min_v_hz: u8,
    /// Maximum vertical refresh in Hz.
    pub max_v_hz: u8,
    /// Minimum vertical blanking in lines.
    pub min_v_blank: u16,
    /// Supports interlaced timings.
    pub interlaced: bool,
    /// Supports CVT.
    pub cvt: bool,
    /// Supports CVT reduced blanking.
    pub cvt_reduced: bool,
    /// Discrete frequency display device.
    pub discrete: bool,
}

/// Display device data (data block 0x0C).
#[derive(Debug, Clone, PartialEq)]
pub struct DidDisplayDevice {
    /// Device technology code (e.g. 0x15 active-matrix IPS, 0x50 OLED).
    pub technology: u8,
    /// Operating mode byte (high nibble: mode; bit 3 backlight, bit 2 intensity).
    pub operating_mode: u8,
    /// Native width in pixels (0 when unset).
    pub native_width: u32,
    /// Native height in pixels (0 when unset).
    pub native_height: u32,
    /// Aspect ratio (width / height).
    pub aspect: f32,
    /// Orientation byte: bits 7-6 default orientation, 5-4 rotation, 3-2 zero pixel, 1-0 scan.
    pub orientation: u8,
    /// Sub-pixel layout code.
    pub subpixel: u8,
    /// Horizontal pixel pitch in mm.
    pub pitch_h_mm: f32,
    /// Vertical pixel pitch in mm.
    pub pitch_v_mm: f32,
    /// Colour bit depth.
    pub bit_depth: u8,
    /// Response time in ms.
    pub response_ms: u8,
    /// Response time is for white-to-black transitions.
    pub response_white_to_black: bool,
}

/// Interface power sequencing (data block 0x0D), all in milliseconds.
#[derive(Debug, Clone, PartialEq)]
pub struct DidPowerSequencing {
    /// T1 minimum.
    pub t1_min_ms: f32,
    /// T1 maximum.
    pub t1_max_ms: f32,
    /// T2 maximum.
    pub t2_max_ms: f32,
    /// T3 maximum.
    pub t3_max_ms: f32,
    /// T4 minimum.
    pub t4_min_ms: f32,
    /// T5 minimum.
    pub t5_min_ms: f32,
    /// T6 minimum.
    pub t6_min_ms: f32,
}

/// Transfer characteristics (data block 0x0E).
#[derive(Debug, Clone, PartialEq)]
pub struct DidTransfer {
    /// Identifier tying it to a colour characteristics block.
    pub id: u8,
    /// First curve is the white curve.
    pub first_is_white: bool,
    /// Curves as cumulative percent samples ending at 100 (sampled form).
    pub curves: Vec<Vec<f32>>,
    /// Four-parameter curves (A0, A1, A2, A3, gamma byte) when that form is used.
    pub params: Vec<[u8; 5]>,
}

/// Display interface (data block 0x0F).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidInterface {
    /// Interface type code (0 analog, 1 LVDS, 2 TMDS, 4 DVI-D, 7 HDMI-A, 8 HDMI-B, 0xA DisplayPort,
    /// ...).
    pub interface_type: u8,
    /// Sub-type (analog) or number of links (digital).
    pub links: u8,
    /// Interface version (major, minor).
    pub version: (u8, u8),
    /// Supported bits per colour, RGB, as a mask: bit 0 = 6 bpc, 1 = 8, 2 = 10, 3 = 12, 4 = 14, 5 = 16.
    pub bpc_rgb: u8,
    /// Same mask for YCbCr 4:4:4.
    pub bpc_ycbcr444: u8,
    /// Mask for YCbCr 4:2:2: bit 0 = 8 bpc, 1 = 10, 2 = 12, 3 = 14, 4 = 16.
    pub bpc_ycbcr422: u8,
    /// Content protection: 0 none, 1 HDCP, 2 DTCP, 3 DPCP.
    pub content_protection: u8,
    /// Content protection version (major, minor).
    pub cp_version: (u8, u8),
    /// Spread spectrum byte (bits 7-6 type, 3-0 percent x 10).
    pub spread_spectrum: u8,
    /// Interface-specific bytes that follow.
    pub type_flags: Vec<u8>,
}

/// Stereo interface (data block 0x10 or 0x27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidStereo {
    /// Stereo interface method code.
    pub method: u8,
    /// Method parameter bytes.
    pub params: Vec<u8>,
}

/// Tiled display topology (data block 0x12 or 0x28).
#[derive(Debug, Clone, PartialEq)]
pub struct DidTile {
    /// Capability byte as stored.
    pub capabilities: u8,
    /// The whole tiled display is one physical enclosure.
    pub single_enclosure: bool,
    /// Number of horizontal tiles.
    pub h_tiles: u8,
    /// Number of vertical tiles.
    pub v_tiles: u8,
    /// This tile's horizontal location (0-based).
    pub h_location: u8,
    /// This tile's vertical location (0-based).
    pub v_location: u8,
    /// Tile width in pixels.
    pub tile_width: u32,
    /// Tile height in pixels.
    pub tile_height: u32,
    /// Bezel sizes in pixels (top, bottom, right, left) when the block carries them.
    pub bezels: Option<[f32; 4]>,
    /// Tiled display vendor id bytes.
    pub vendor: [u8; 3],
    /// Tiled display product code.
    pub product_code: u16,
    /// Tiled display serial number.
    pub serial: u32,
}

/// DisplayID 2.x display parameters (data block 0x21).
#[derive(Debug, Clone, PartialEq)]
pub struct DidParametersV2 {
    /// Horizontal image size in mm.
    pub width_mm: f32,
    /// Vertical image size in mm.
    pub height_mm: f32,
    /// Native horizontal pixels.
    pub native_width: u16,
    /// Native vertical pixels.
    pub native_height: u16,
    /// Scan orientation code (0 = left to right, top to bottom).
    pub scan_orientation: u8,
    /// Luminance information: 0 minimum guaranteed, 1 guidance for the source.
    pub luminance_info: u8,
    /// Coordinates are CIE 1976 instead of CIE 1931.
    pub cie_1976: bool,
    /// Speakers are integrated.
    pub speakers_integrated: bool,
    /// Native primaries (x, y).
    pub primaries: [(f32, f32); 3],
    /// Native white point (x, y).
    pub white_point: (f32, f32),
    /// Native maximum luminance, full coverage, cd/m2.
    pub max_luminance_full: Option<f32>,
    /// Native maximum luminance, 10 % rectangular coverage, cd/m2.
    pub max_luminance_10pct: Option<f32>,
    /// Native minimum luminance, cd/m2.
    pub min_luminance: Option<f32>,
    /// Native bits per colour (None when not defined).
    pub native_bpc: Option<u8>,
    /// Device technology: 0 not specified, 1 active-matrix LCD, 2 OLED.
    pub technology: u8,
    /// Dark theme preferred (revision 1 and later).
    pub dark_theme: bool,
    /// Native gamma EOTF, `None` when not defined.
    pub gamma: Option<f32>,
}

/// Dynamic video timing range limits (data block 0x25): the adaptive-sync range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidDynamicRange {
    /// Minimum pixel clock in kHz.
    pub min_pixel_clock_khz: u32,
    /// Maximum pixel clock in kHz.
    pub max_pixel_clock_khz: u32,
    /// Minimum vertical refresh in millihertz.
    pub min_refresh_millihz: u32,
    /// Maximum vertical refresh in millihertz.
    pub max_refresh_millihz: u32,
    /// Seamless dynamic video timing change supported.
    pub seamless: bool,
}

/// Display interface features (data block 0x26).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidInterfaceFeatures {
    /// RGB bpc mask (bit 0 = 6 bpc, 1 = 8, 2 = 10, 3 = 12, 4 = 14, 5 = 16).
    pub bpc_rgb: u8,
    /// YCbCr 4:4:4 bpc mask (same bits).
    pub bpc_ycbcr444: u8,
    /// YCbCr 4:2:2 bpc mask (bit 0 = 8 bpc, 1 = 10, 2 = 12, 3 = 14, 4 = 16).
    pub bpc_ycbcr422: u8,
    /// YCbCr 4:2:0 bpc mask (same bits as 4:2:2).
    pub bpc_ycbcr420: u8,
    /// Minimum pixel rate for 4:2:0 in units of 74.25 MHz (0 when not stated).
    pub min_420_rate_74mhz: u8,
    /// Audio sample-rate mask (bit 0 = 32 kHz, 1 = 44.1, 2 = 48).
    pub audio_rates: u8,
    /// Colour space and EOTF combination mask 1 (bit 0 sRGB, 1 BT.601, 2 BT.709/BT.1886,
    /// 3 Adobe RGB, 4 DCI-P3, 5 BT.2020, 6 BT.2020 with SMPTE ST 2084).
    pub combos1: u8,
    /// Combination mask 2 (reserved by the standard).
    pub combos2: u8,
    /// Additional (colour space, EOTF) pairs as stored (high nibble, low nibble).
    pub additional: Vec<(u8, u8)>,
}

/// One adaptive sync descriptor (data block 0x2B).
#[derive(Debug, Clone, PartialEq)]
pub struct DidAdaptiveSync {
    /// Flags byte as stored.
    pub flags: u8,
    /// Range is the panel's native range.
    pub native_range: bool,
    /// Seamless transitions supported.
    pub seamless: bool,
    /// Maximum single frame duration increase in ms.
    pub max_duration_increase_ms: f32,
    /// Maximum single frame duration decrease in ms.
    pub max_duration_decrease_ms: f32,
    /// Minimum refresh in millihertz.
    pub min_refresh_millihz: u32,
    /// Maximum refresh in millihertz.
    pub max_refresh_millihz: u32,
}

/// The decoded content of a data block.
#[derive(Debug, Clone, PartialEq)]
pub enum DidBlockData {
    /// Product identification (0x00, 0x20).
    ProductId(DidProductId),
    /// 1.x display parameters (0x01).
    Parameters(DidParameters),
    /// Colour characteristics (0x02).
    ColorCharacteristics(DidColorCharacteristics),
    /// Detailed or formula timings; they are in [`DisplayId::timings`].
    Timings {
        /// Number of timings the block contributed.
        count: usize,
    },
    /// Enumerated timing codes (0x06, 0x07, 0x08, 0x23); see [`DisplayId::timing_codes`].
    TimingCodes {
        /// Number of codes the block listed.
        count: usize,
    },
    /// Video timing range limits (0x09).
    RangeLimits(DidRangeLimits),
    /// ASCII text (0x0A product serial string, 0x0B general string).
    Text(String),
    /// Display device data (0x0C).
    DisplayDevice(DidDisplayDevice),
    /// Interface power sequencing (0x0D).
    PowerSequencing(DidPowerSequencing),
    /// Transfer characteristics (0x0E).
    Transfer(DidTransfer),
    /// Display interface (0x0F).
    Interface(DidInterface),
    /// Stereo interface (0x10, 0x27).
    Stereo(DidStereo),
    /// Tiled display topology (0x12, 0x28).
    Tiled(DidTile),
    /// 2.x display parameters (0x21).
    ParametersV2(DidParametersV2),
    /// Dynamic video timing range limits (0x25).
    DynamicRange(DidDynamicRange),
    /// Display interface features (0x26).
    InterfaceFeatures(DidInterfaceFeatures),
    /// ContainerID (0x29).
    ContainerId([u8; 16]),
    /// Adaptive sync descriptors (0x2B).
    AdaptiveSync(Vec<DidAdaptiveSync>),
    /// Vendor-specific (0x7E, 0x7F) with the vendor id bytes.
    Vendor {
        /// Vendor identifier (OUI) bytes.
        oui: [u8; 3],
    },
    /// CTA-861 DisplayID data block (0x81); the payload is not decoded here.
    CtaData {
        /// Payload length in bytes.
        len: usize,
    },
    /// Known or unknown tag that is recorded but not decoded (also used when malformed).
    Raw,
}

/// One data block of a DisplayID section.
#[derive(Debug, Clone, PartialEq)]
pub struct DidBlock {
    /// Data block tag.
    pub tag: u8,
    /// Revision and flags byte (bits 2-0 revision).
    pub rev_flags: u8,
    /// Payload length in bytes, as stored.
    pub payload_len: u8,
    /// The payload was too short or inconsistent for this block's layout.
    pub malformed: bool,
    /// What the payload decoded to.
    pub data: DidBlockData,
}

/// One DisplayID section (one EDID extension block with tag 0x70).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayId {
    /// DisplayID version byte, e.g. `0x13` or `0x20`.
    pub version: u8,
    /// Product type (1.x) or primary use case (2.x).
    pub product_type: u8,
    /// Number of data-block bytes the section declares (clamped to 121).
    pub section_bytes: u8,
    /// Number of extension sections following the base section.
    pub extension_count: u8,
    /// The section checksum adds up to zero.
    pub checksum_valid: bool,
    /// The block was too short, or a data block ran past the section end.
    pub truncated: bool,
    /// Data blocks in order.
    pub blocks: Vec<DidBlock>,
    /// Every timing from every timing block, in order.
    pub timings: Vec<DidTiming>,
    /// Every enumerated timing code, in order.
    pub timing_codes: Vec<DidTimingCode>,
}

impl DisplayId {
    /// All timings of the section in one list.
    pub fn timings(&self) -> impl Iterator<Item = &DidTiming> {
        self.timings.iter()
    }

    /// The first block with this tag.
    pub fn block(&self, tag: u8) -> Option<&DidBlock> {
        self.blocks.iter().find(|b| b.tag == tag)
    }

    /// Whether the section has a block with this tag.
    pub fn has_block(&self, tag: u8) -> bool {
        self.block(tag).is_some()
    }

    /// The product identification block, if present.
    pub fn product_id(&self) -> Option<&DidProductId> {
        self.blocks.iter().find_map(|b| match &b.data {
            DidBlockData::ProductId(p) => Some(p),
            _ => None,
        })
    }

    /// Tiled topology blocks.
    pub fn tiles(&self) -> impl Iterator<Item = &DidTile> {
        self.blocks.iter().filter_map(|b| match &b.data {
            DidBlockData::Tiled(t) => Some(t),
            _ => None,
        })
    }

    /// Adaptive sync ranges (data block 0x2B), in millihertz.
    pub fn adaptive_sync(&self) -> impl Iterator<Item = &DidAdaptiveSync> {
        self.blocks
            .iter()
            .filter_map(|b| match &b.data {
                DidBlockData::AdaptiveSync(v) => Some(v.iter()),
                _ => None,
            })
            .flatten()
    }

    /// Dynamic video timing range limits (data block 0x25), if present.
    pub fn dynamic_range(&self) -> Option<&DidDynamicRange> {
        self.blocks.iter().find_map(|b| match &b.data {
            DidBlockData::DynamicRange(d) => Some(d),
            _ => None,
        })
    }

    /// 2.x display parameters, if present.
    pub fn parameters_v2(&self) -> Option<&DidParametersV2> {
        self.blocks.iter().find_map(|b| match &b.data {
            DidBlockData::ParametersV2(d) => Some(d),
            _ => None,
        })
    }

    /// Display interface features (data block 0x26), if present.
    pub fn interface_features(&self) -> Option<&DidInterfaceFeatures> {
        self.blocks.iter().find_map(|b| match &b.data {
            DidBlockData::InterfaceFeatures(d) => Some(d),
            _ => None,
        })
    }
}

/// Supported bits per colour for an RGB or YCbCr 4:4:4 mask (6, 8, 10, 12, 14, 16).
pub fn bpc_list_444(mask: u8) -> Vec<u8> {
    [6u8, 8, 10, 12, 14, 16]
        .iter()
        .enumerate()
        .filter(|(i, _)| mask & (1 << i) != 0)
        .map(|(_, &b)| b)
        .collect()
}

/// Supported bits per colour for a YCbCr 4:2:2 or 4:2:0 mask (8, 10, 12, 14, 16).
pub fn bpc_list_4xx(mask: u8) -> Vec<u8> {
    [8u8, 10, 12, 14, 16]
        .iter()
        .enumerate()
        .filter(|(i, _)| mask & (1 << i) != 0)
        .map(|(_, &b)| b)
        .collect()
}

fn b(p: &[u8], i: usize) -> u8 {
    p.get(i).copied().unwrap_or(0)
}

fn le16(p: &[u8], i: usize) -> u32 {
    u32::from(b(p, i)) | (u32::from(b(p, i + 1)) << 8)
}

fn le24(p: &[u8], i: usize) -> u32 {
    le16(p, i) | (u32::from(b(p, i + 2)) << 16)
}

fn le32(p: &[u8], i: usize) -> u32 {
    le16(p, i) | (le16(p, i + 2) << 16)
}

fn aspect_code(c: u8) -> Option<(u16, u16)> {
    match c {
        0 => Some((1, 1)),
        1 => Some((5, 4)),
        2 => Some((4, 3)),
        3 => Some((15, 9)),
        4 => Some((16, 9)),
        5 => Some((16, 10)),
        6 => Some((64, 27)),
        7 => Some((256, 135)),
        _ => None,
    }
}

/// Refresh in millihertz from pixel clock and totals; `frame_lines` are all lines of one frame.
fn refresh(clock_khz: u32, h_total: u32, v_total: u32, interlaced: bool) -> u32 {
    let den = u64::from(h_total) * u64::from(v_total);
    if den == 0 {
        return 0;
    }
    let fields = if interlaced { 2 } else { 1 };
    ((u64::from(clock_khz) * 1_000_000 * fields + den / 2) / den) as u32
}

/// 12-bit chromaticity pair at `i` (3 bytes) as (x, y).
fn chroma(p: &[u8], i: usize) -> (f32, f32) {
    let x = u32::from(b(p, i)) | (u32::from(b(p, i + 1) & 0x0F) << 8);
    let y = u32::from(b(p, i + 1) >> 4) | (u32::from(b(p, i + 2)) << 4);
    (x as f32 / 4096.0, y as f32 / 4096.0)
}

/// 16-bit DisplayID luminance (5-bit exponent, 10-bit mantissa) in cd/m2.
fn luminance(v: u32) -> Option<f32> {
    if v & 0x8000 != 0 {
        return None;
    }
    // IEEE 754 half precision (sign bit already rejected): exponent 0 is subnormal (no implicit
    // leading 1, scale 2^-14), exponent 31 is infinity or NaN.
    let e = (v & 0x7C00) >> 10;
    let m = (v & 0x3FF) as f32;
    match e {
        0 => Some(2f32.powi(-14) * m / 1024.0),
        31 => None,
        _ => Some(2f32.powi(e as i32 - 15) * (1.0 + m / 1024.0)),
    }
}

fn text(p: &[u8]) -> String {
    p.iter()
        .take_while(|&&c| c != 0)
        .map(|&c| {
            if (0x20..0x7F).contains(&c) {
                c as char
            } else {
                '?'
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

struct Out<'a> {
    id: &'a mut DisplayId,
}

/// Decodes a DisplayID EDID extension block (tag 0x70).
pub fn parse_displayid(blk: &[u8]) -> DisplayId {
    let mut id = DisplayId {
        version: b(blk, 1),
        product_type: b(blk, 3),
        extension_count: b(blk, 4),
        ..DisplayId::default()
    };
    if blk.len() < 5 {
        id.truncated = true;
        return id;
    }
    let mut len = usize::from(blk[2]);
    if len > MAX_SECTION_BYTES {
        len = MAX_SECTION_BYTES;
        id.truncated = true;
    }
    id.section_bytes = len as u8;
    let end = 5 + len;
    match blk.get(1..=end) {
        Some(s) => id.checksum_valid = s.iter().fold(0u8, |a, &c| a.wrapping_add(c)) == 0,
        None => id.truncated = true,
    }
    let section_end = end.min(blk.len());
    let mut out = Out { id: &mut id };
    let mut i = 5;
    while i + 3 <= section_end {
        let (tag, rev, plen) = (blk[i], blk[i + 1], usize::from(blk[i + 2]));
        if tag == 0 && plen == 0 {
            break; // padding
        }
        let start = i + 3;
        if start + plen > section_end {
            out.id.truncated = true;
            break;
        }
        out.block(tag, rev, &blk[start..start + plen]);
        i = start + plen;
    }
    id
}

impl Out<'_> {
    fn push_timing(&mut self, t: DidTiming) {
        self.id.timings.push(t);
    }

    fn code(&mut self, kind: DidCodeKind, code: u16, ty: DidTimingType) {
        self.id.timing_codes.push(DidTimingCode {
            kind,
            code,
            timing_type: ty,
        });
        let t = match kind {
            DidCodeKind::Vic => u8::try_from(code).ok().and_then(vic::vic),
            DidCodeKind::HdmiVic => u8::try_from(code).ok().and_then(vic::hdmi_vic),
            DidCodeKind::Dmt => None,
        };
        if let Some(t) = t {
            self.push_timing(DidTiming {
                width: u32::from(t.width),
                height: u32::from(t.height),
                refresh_millihz: t.refresh_millihz,
                interlaced: t.interlaced,
                preferred: false,
                pixel_clock_khz: t.pixel_clock_khz,
                h_total: u32::from(t.h_total),
                v_total: u32::from(t.v_total),
                timing_type: ty,
                stereo: 0,
                aspect: None,
                detail: None,
            });
        }
    }

    fn block(&mut self, tag: u8, rev: u8, p: &[u8]) {
        let timings_before = self.id.timings.len();
        let codes_before = self.id.timing_codes.len();
        let mut malformed = false;
        let data = self.decode(tag, rev, p, &mut malformed);
        let data = match data {
            DidBlockData::Timings { .. } => DidBlockData::Timings {
                count: self.id.timings.len() - timings_before,
            },
            DidBlockData::TimingCodes { .. } => DidBlockData::TimingCodes {
                count: self.id.timing_codes.len() - codes_before,
            },
            d => d,
        };
        self.id.blocks.push(DidBlock {
            tag,
            rev_flags: rev,
            payload_len: p.len() as u8,
            malformed,
            data,
        });
    }

    fn decode(&mut self, tag: u8, rev: u8, p: &[u8], bad: &mut bool) -> DidBlockData {
        let need = |n: usize, bad: &mut bool| -> bool {
            if p.len() < n {
                *bad = true;
                false
            } else {
                true
            }
        };
        let block_rev = rev & 7;
        match tag {
            0x00 | 0x20 => {
                if !need(11, bad) {
                    return DidBlockData::Raw;
                }
                let n = usize::from(b(p, 11));
                let name = p.get(12..).map(|r| text(&r[..n.min(r.len())]));
                DidBlockData::ProductId(DidProductId {
                    oui: [p[0], p[1], p[2]],
                    product_code: le16(p, 3) as u16,
                    serial: le32(p, 5),
                    week: p[9],
                    year: 2000 + u16::from(p[10]),
                    name: name.unwrap_or_default(),
                })
            }
            0x01 => {
                if !need(12, bad) {
                    return DidBlockData::Raw;
                }
                DidBlockData::Parameters(DidParameters {
                    width_dmm: le16(p, 0) as u16,
                    height_dmm: le16(p, 2) as u16,
                    h_pixels: le16(p, 4) as u16,
                    v_pixels: le16(p, 6) as u16,
                    feature_flags: p[8],
                    gamma: (p[9] != 0xFF).then(|| (f32::from(p[9]) + 100.0) / 100.0),
                    aspect: (f32::from(p[10]) + 100.0) / 100.0,
                    bpc_native: (p[11] & 0x0F) + 1,
                    bpc_overall: (p[11] >> 4) + 1,
                })
            }
            0x02 => {
                if !need(1, bad) {
                    return DidBlockData::Raw;
                }
                let nw = usize::from(p[0] & 0x0F);
                let np = usize::from((p[0] >> 4) & 7);
                let mut off = 1;
                let mut color_space = None;
                if np == 0 {
                    color_space = p.get(1).copied();
                    off = 2;
                }
                if p.len() < off + 3 * (np + nw) {
                    *bad = true;
                }
                let get = |from: usize, n: usize| -> Vec<(f32, f32)> {
                    (0..n)
                        .filter(|k| from + 3 * k + 3 <= p.len())
                        .map(|k| chroma(p, from + 3 * k))
                        .collect()
                };
                DidBlockData::ColorCharacteristics(DidColorCharacteristics {
                    cie_1976: rev & 0x80 != 0,
                    temporal: p[0] & 0x80 != 0,
                    transfer_id: (rev >> 3) & 0x0F,
                    color_space,
                    primaries: get(off, np),
                    white_points: get(off + 3 * np, nw),
                })
            }
            0x03 | 0x22 => {
                let vii = tag == 0x22;
                let sz = if vii {
                    20 + usize::from((rev >> 4) & 7)
                } else {
                    20
                };
                if p.len() < sz {
                    *bad = true;
                }
                for d in p.chunks_exact(sz) {
                    self.timing_i_vii(d, vii, block_rev);
                }
                DidBlockData::Timings { count: 0 }
            }
            0x04 => {
                if !p.len().is_multiple_of(11) {
                    *bad = true;
                }
                for d in p.as_chunks::<11>().0.iter() {
                    self.timing_ii(d);
                }
                DidBlockData::Timings { count: 0 }
            }
            0x05 => {
                if !p.len().is_multiple_of(3) {
                    *bad = true;
                }
                for d in p.as_chunks::<3>().0.iter() {
                    self.timing_iii(d);
                }
                DidBlockData::Timings { count: 0 }
            }
            0x06 => {
                let kind = code_kind(rev >> 6);
                for &c in p {
                    self.code(kind, u16::from(c), DidTimingType::IV);
                }
                DidBlockData::TimingCodes { count: 0 }
            }
            0x07 => {
                for i in 0..p.len().min(10) * 8 {
                    if p[i / 8] & (1 << (i % 8)) != 0 {
                        self.code(DidCodeKind::Dmt, i as u16 + 1, DidTimingType::IV);
                    }
                }
                DidBlockData::TimingCodes { count: 0 }
            }
            0x08 => {
                for i in 0..p.len().min(8) * 8 {
                    if p[i / 8] & (1 << (i % 8)) != 0 {
                        self.code(DidCodeKind::Vic, i as u16 + 1, DidTimingType::CtaBitmap);
                    }
                }
                DidBlockData::TimingCodes { count: 0 }
            }
            0x23 => {
                let kind = code_kind(rev >> 6);
                if rev & 0x08 != 0 {
                    for c in p.as_chunks::<2>().0.iter() {
                        self.code(kind, le16(c, 0) as u16, DidTimingType::VIII);
                    }
                } else {
                    for &c in p {
                        self.code(kind, u16::from(c), DidTimingType::VIII);
                    }
                }
                DidBlockData::TimingCodes { count: 0 }
            }
            0x09 => {
                if !need(15, bad) {
                    return DidBlockData::Raw;
                }
                DidBlockData::RangeLimits(DidRangeLimits {
                    min_pixel_clock_khz: (le24(p, 0) + 1) * 10,
                    max_pixel_clock_khz: (le24(p, 3) + 1) * 10,
                    min_h_khz: p[6],
                    max_h_khz: p[7],
                    min_h_blank: le16(p, 8) as u16,
                    min_v_hz: p[10],
                    max_v_hz: p[11],
                    min_v_blank: le16(p, 12) as u16,
                    interlaced: p[14] & 0x80 != 0,
                    cvt: p[14] & 0x40 != 0,
                    cvt_reduced: p[14] & 0x20 != 0,
                    discrete: p[14] & 0x10 != 0,
                })
            }
            0x0A | 0x0B => DidBlockData::Text(text(p)),
            0x0C => {
                if !need(13, bad) {
                    return DidBlockData::Raw;
                }
                let w = le16(p, 2);
                let h = le16(p, 4);
                DidBlockData::DisplayDevice(DidDisplayDevice {
                    technology: p[0],
                    operating_mode: p[1],
                    native_width: if w != 0 { w + 1 } else { 0 },
                    native_height: if h != 0 { h + 1 } else { 0 },
                    aspect: (f32::from(b(p, 6)) + 100.0) / 100.0,
                    orientation: b(p, 7),
                    subpixel: b(p, 8),
                    pitch_h_mm: f32::from(b(p, 9)) / 100.0,
                    pitch_v_mm: f32::from(b(p, 10)) / 100.0,
                    bit_depth: b(p, 11) & 0x0F,
                    response_ms: b(p, 12) & 0x7F,
                    response_white_to_black: b(p, 12) & 0x80 != 0,
                })
            }
            0x0D => {
                if !need(6, bad) {
                    return DidBlockData::Raw;
                }
                DidBlockData::PowerSequencing(DidPowerSequencing {
                    t1_min_ms: f32::from(p[0] >> 4) / 10.0,
                    t1_max_ms: f32::from(p[0] & 0x0F) * 2.0,
                    t2_max_ms: f32::from(p[1] & 0x3F) * 2.0,
                    t3_max_ms: f32::from(p[2] & 0x3F) * 2.0,
                    t4_min_ms: f32::from(p[3] & 0x7F) * 10.0,
                    t5_min_ms: f32::from(p[4] & 0x3F) * 10.0,
                    t6_min_ms: f32::from(p[5] & 0x3F) * 10.0,
                })
            }
            0x0E => {
                if !need(1, bad) {
                    return DidBlockData::Raw;
                }
                let four = p[0] & 0x20 != 0;
                let mut t = DidTransfer {
                    id: rev >> 4,
                    first_is_white: p[0] & 0x80 != 0,
                    curves: vec![],
                    params: vec![],
                };
                let mut off = 1;
                while off < p.len() {
                    let n = usize::from(p[off]);
                    if four {
                        match p.get(off + 1..off + 6) {
                            Some(s) => t.params.push([s[0], s[1], s[2], s[3], s[4]]),
                            None => {
                                *bad = true;
                                break;
                            }
                        }
                        off += 6;
                    } else {
                        // n samples; the last (always 100 %) is not stored.
                        let stored = n.saturating_sub(1);
                        let Some(s) = p.get(off + 1..off + 1 + stored) else {
                            *bad = true;
                            break;
                        };
                        let mut sum = 0f32;
                        let mut c: Vec<f32> = s
                            .iter()
                            .map(|&v| {
                                sum += f32::from(v);
                                sum * 100.0 / 1023.0
                            })
                            .collect();
                        c.push(100.0);
                        t.curves.push(c);
                        off += 1 + stored;
                    }
                }
                DidBlockData::Transfer(t)
            }
            0x0F => {
                if !need(10, bad) {
                    return DidBlockData::Raw;
                }
                DidBlockData::Interface(DidInterface {
                    interface_type: p[0] >> 4,
                    links: p[0] & 0x0F,
                    version: (p[1] >> 4, p[1] & 0x0F),
                    bpc_rgb: p[2],
                    bpc_ycbcr444: p[3],
                    bpc_ycbcr422: p[4],
                    content_protection: p[5] & 0x0F,
                    cp_version: (p[6] >> 4, p[6] & 0x0F),
                    spread_spectrum: p[7],
                    type_flags: p[8..].to_vec(),
                })
            }
            0x10 | 0x27 => {
                if !need(2, bad) {
                    return DidBlockData::Raw;
                }
                let n = usize::from(p[0]);
                let params_end = (1 + n).min(p.len());
                if 1 + n > p.len() {
                    *bad = true;
                }
                let s = DidStereo {
                    method: p[1],
                    params: p[2.min(params_end)..params_end].to_vec(),
                };
                if rev & 0x40 != 0 {
                    let mut pos = 1 + n;
                    while pos < p.len() {
                        let cnt = usize::from(p[pos] & 0x1F);
                        let kind = code_kind(p[pos] >> 6);
                        let Some(codes) = p.get(pos + 1..pos + 1 + cnt) else {
                            *bad = true;
                            break;
                        };
                        for &c in codes {
                            self.code(kind, u16::from(c), DidTimingType::StereoCode);
                        }
                        pos += 1 + cnt;
                    }
                }
                DidBlockData::Stereo(s)
            }
            0x11 => {
                if !p.len().is_multiple_of(7) {
                    *bad = true;
                }
                for d in p.as_chunks::<7>().0.iter() {
                    self.timing_v(d);
                }
                DidBlockData::Timings { count: 0 }
            }
            0x12 | 0x28 => {
                if !need(22, bad) {
                    return DidBlockData::Raw;
                }
                let caps = p[0];
                let pm = f32::from(p[8]);
                DidBlockData::Tiled(DidTile {
                    capabilities: caps,
                    single_enclosure: caps & 0x80 != 0,
                    h_tiles: ((p[1] >> 4) | ((p[3] >> 2) & 0x30)) + 1,
                    v_tiles: ((p[1] & 0x0F) | (p[3] & 0x30)) + 1,
                    h_location: (p[2] >> 4) | (((p[3] >> 2) & 3) << 4),
                    v_location: (p[2] & 0x0F) | ((p[3] & 3) << 4),
                    tile_width: le16(p, 4) + 1,
                    tile_height: le16(p, 6) + 1,
                    bezels: (caps & 0x40 != 0).then(|| {
                        [
                            pm * f32::from(p[9]) / 10.0,
                            pm * f32::from(p[10]) / 10.0,
                            pm * f32::from(p[11]) / 10.0,
                            pm * f32::from(p[12]) / 10.0,
                        ]
                    }),
                    vendor: [p[13], p[14], p[15]],
                    product_code: le16(p, 16) as u16,
                    serial: le32(p, 18),
                })
            }
            0x13 => {
                let mut off = 0;
                while off + 14 <= p.len() {
                    let sz = if p[off + 2] & 0x40 != 0 { 17 } else { 14 };
                    match p.get(off..off + sz) {
                        Some(d) => self.timing_vi(d),
                        None => {
                            *bad = true;
                            break;
                        }
                    }
                    off += sz;
                }
                if off != p.len() {
                    *bad = true;
                }
                DidBlockData::Timings { count: 0 }
            }
            0x21 => {
                if !need(29, bad) {
                    return DidBlockData::Raw;
                }
                let mm = if rev & 0x80 != 0 { 1.0 } else { 0.1 };
                let nb = p[27] & 7;
                DidBlockData::ParametersV2(DidParametersV2 {
                    width_mm: le16(p, 0) as f32 * mm,
                    height_mm: le16(p, 2) as f32 * mm,
                    native_width: le16(p, 4) as u16,
                    native_height: le16(p, 6) as u16,
                    scan_orientation: p[8] & 7,
                    luminance_info: (p[8] >> 3) & 3,
                    cie_1976: p[8] & 0x40 != 0,
                    speakers_integrated: p[8] & 0x80 == 0,
                    primaries: [chroma(p, 9), chroma(p, 12), chroma(p, 15)],
                    white_point: chroma(p, 18),
                    max_luminance_full: luminance(le16(p, 21)),
                    max_luminance_10pct: luminance(le16(p, 23)),
                    min_luminance: luminance(le16(p, 25)),
                    native_bpc: match nb {
                        1..=6 => Some([6u8, 8, 10, 12, 14, 16][usize::from(nb) - 1]),
                        _ => None,
                    },
                    technology: (p[27] >> 4) & 7,
                    dark_theme: block_rev >= 1 && p[27] & 0x80 != 0,
                    gamma: (p[28] != 0xFF).then(|| (f32::from(p[28]) + 100.0) / 100.0),
                })
            }
            0x24 => {
                if !p.len().is_multiple_of(6) {
                    *bad = true;
                }
                for d in p.as_chunks::<6>().0.iter() {
                    self.timing_ix_x(d, DidTimingType::IX, 6);
                }
                DidBlockData::Timings { count: 0 }
            }
            0x25 => {
                if !need(9, bad) {
                    return DidBlockData::Raw;
                }
                let max = if block_rev >= 1 {
                    u32::from(p[7]) + (u32::from(p[8] & 3) << 8)
                } else {
                    u32::from(p[7])
                };
                DidBlockData::DynamicRange(DidDynamicRange {
                    min_pixel_clock_khz: le24(p, 0) + 1,
                    max_pixel_clock_khz: le24(p, 3) + 1,
                    min_refresh_millihz: u32::from(p[6]) * 1000,
                    max_refresh_millihz: max * 1000,
                    seamless: p[8] & 0x80 != 0,
                })
            }
            0x26 => {
                if !need(1, bad) {
                    return DidBlockData::Raw;
                }
                let n = usize::from(b(p, 8));
                let mut additional = vec![];
                for k in 0..n {
                    match p.get(9 + k) {
                        Some(&v) if k < 7 => additional.push((v >> 4, v & 0x0F)),
                        _ => {
                            *bad = true;
                            break;
                        }
                    }
                }
                if p.len() < 9 {
                    *bad = true;
                }
                DidBlockData::InterfaceFeatures(DidInterfaceFeatures {
                    bpc_rgb: b(p, 0),
                    bpc_ycbcr444: b(p, 1),
                    bpc_ycbcr422: b(p, 2),
                    bpc_ycbcr420: b(p, 3),
                    min_420_rate_74mhz: b(p, 4),
                    audio_rates: b(p, 5),
                    combos1: b(p, 6),
                    combos2: b(p, 7),
                    additional,
                })
            }
            0x29 => {
                if !need(16, bad) {
                    return DidBlockData::Raw;
                }
                let mut u = [0u8; 16];
                u.copy_from_slice(&p[..16]);
                DidBlockData::ContainerId(u)
            }
            0x2A => {
                let sz = 6 + usize::from((rev >> 4) & 7);
                if sz > 8 || !p.len().is_multiple_of(sz) {
                    *bad = true;
                }
                if sz <= 8 {
                    for d in p.chunks_exact(sz) {
                        self.timing_ix_x(d, DidTimingType::X, sz);
                    }
                }
                DidBlockData::Timings { count: 0 }
            }
            0x2B => {
                let sz = 6 + usize::from((rev >> 4) & 7);
                if !p.len().is_multiple_of(sz) {
                    *bad = true;
                }
                DidBlockData::AdaptiveSync(
                    p.chunks_exact(sz)
                        .map(|d| DidAdaptiveSync {
                            flags: d[0],
                            native_range: d[0] & 1 != 0,
                            seamless: d[0] & 0x10 == 0,
                            max_duration_increase_ms: f32::from(d[1]) / 4.0,
                            max_duration_decrease_ms: f32::from(d[5]) / 4.0,
                            min_refresh_millihz: u32::from(d[2]) * 1000,
                            max_refresh_millihz: (1
                                + u32::from(d[3])
                                + ((u32::from(d[4]) & 3) << 8))
                                * 1000,
                        })
                        .collect(),
                )
            }
            0x7E | 0x7F => {
                if !need(3, bad) {
                    return DidBlockData::Raw;
                }
                DidBlockData::Vendor {
                    oui: [p[0], p[1], p[2]],
                }
            }
            0x81 => DidBlockData::CtaData { len: p.len() },
            _ => DidBlockData::Raw,
        }
    }

    /// Type I (10 kHz units) and type VII (1 kHz units) 20-byte detailed timings.
    fn timing_i_vii(&mut self, d: &[u8], vii: bool, block_rev: u8) {
        let unit = if vii { 1 } else { 10 };
        let clock = unit * (1 + le24(d, 0));
        let flags = b(d, 3);
        let h_active = 1 + le16(d, 4);
        let h_blank = 1 + le16(d, 6);
        let h_front = 1 + (le16(d, 8) & 0x7FFF);
        let h_sync = 1 + le16(d, 10);
        let v_active = 1 + le16(d, 12);
        let v_blank = 1 + le16(d, 14);
        let v_front = 1 + (le16(d, 16) & 0x7FFF);
        let v_sync = 1 + le16(d, 18);
        let interlaced = flags & 0x10 != 0;
        let (h_total, v_total) = (h_active + h_blank, v_active + v_blank);
        self.push_timing(DidTiming {
            width: h_active,
            height: v_active,
            refresh_millihz: refresh(clock, h_total, v_total, interlaced),
            interlaced,
            preferred: block_rev < 2 && flags & 0x80 != 0,
            pixel_clock_khz: clock,
            h_total,
            v_total,
            timing_type: if vii {
                DidTimingType::VII
            } else {
                DidTimingType::I
            },
            stereo: (flags >> 5) & 3,
            aspect: aspect_code(flags & 0x0F),
            detail: Some(DidDetail {
                h_blank,
                h_front,
                h_sync,
                v_blank,
                v_front,
                v_sync,
                h_sync_positive: b(d, 9) & 0x80 != 0,
                v_sync_positive: b(d, 17) & 0x80 != 0,
            }),
        });
    }

    fn timing_ii(&mut self, d: &[u8]) {
        let clock = 10 * (1 + le24(d, 0));
        let flags = b(d, 3);
        let h_active = 8 + 8 * (u32::from(b(d, 4)) | (u32::from(b(d, 5) & 1) << 8));
        let h_blank = 8 + 8 * u32::from((b(d, 5) & 0xFE) >> 1);
        let h_front = 8 + 8 * u32::from(b(d, 6) >> 4);
        let h_sync = 8 + 8 * u32::from(b(d, 6) & 0x0F);
        let v_active = 1 + (u32::from(b(d, 7)) | (u32::from(b(d, 8) & 0x0F) << 8));
        let v_blank = 1 + u32::from(b(d, 9));
        let v_front = 1 + u32::from(b(d, 10) >> 4);
        let v_sync = 1 + u32::from(b(d, 10) & 0x0F);
        let interlaced = flags & 0x10 != 0;
        let (h_total, v_total) = (h_active + h_blank, v_active + v_blank);
        self.push_timing(DidTiming {
            width: h_active,
            height: v_active,
            refresh_millihz: refresh(clock, h_total, v_total, interlaced),
            interlaced,
            preferred: flags & 0x80 != 0,
            pixel_clock_khz: clock,
            h_total,
            v_total,
            timing_type: DidTimingType::II,
            stereo: (flags >> 5) & 3,
            aspect: None,
            detail: Some(DidDetail {
                h_blank,
                h_front,
                h_sync,
                v_blank,
                v_front,
                v_sync,
                h_sync_positive: flags & 0x08 != 0,
                v_sync_positive: flags & 0x04 != 0,
            }),
        });
    }

    fn timing_iii(&mut self, d: &[u8]) {
        let aspect = aspect_code(d[0] & 0x0F);
        let width = 8 + 8 * u32::from(d[1]);
        let height = aspect.map_or(0, |(h, v)| width * u32::from(v) / u32::from(h));
        self.push_timing(DidTiming {
            width,
            height,
            refresh_millihz: (1 + u32::from(d[2] & 0x7F)) * 1000,
            interlaced: d[2] & 0x80 != 0,
            preferred: d[0] & 0x80 != 0,
            pixel_clock_khz: 0,
            h_total: 0,
            v_total: 0,
            timing_type: DidTimingType::III,
            stereo: 0,
            aspect,
            detail: None,
        });
    }

    fn timing_v(&mut self, d: &[u8]) {
        self.push_timing(DidTiming {
            width: 1 + le16(d, 2),
            height: 1 + le16(d, 4),
            refresh_millihz: (1 + u32::from(d[6])) * 1000,
            interlaced: false,
            preferred: d[0] & 0x80 != 0,
            pixel_clock_khz: 0,
            h_total: 0,
            v_total: 0,
            timing_type: DidTimingType::V,
            stereo: (d[0] >> 5) & 3,
            aspect: None,
            detail: None,
        });
    }

    fn timing_ix_x(&mut self, d: &[u8], ty: DidTimingType, sz: usize) {
        let extra = if sz > 6 {
            u32::from(b(d, 6) & 3) << 8
        } else {
            0
        };
        self.push_timing(DidTiming {
            width: 1 + le16(d, 1),
            height: 1 + le16(d, 3),
            refresh_millihz: (1 + u32::from(b(d, 5)) + extra) * 1000,
            interlaced: false,
            preferred: false,
            pixel_clock_khz: 0,
            h_total: 0,
            v_total: 0,
            timing_type: ty,
            stereo: (b(d, 0) >> 5) & 3,
            aspect: None,
            detail: None,
        });
    }

    fn timing_vi(&mut self, d: &[u8]) {
        let clock = 1 + le16(d, 0) + (u32::from(b(d, 2) & 0x3F) << 16);
        let h_active = 1 + u32::from(b(d, 3)) + (u32::from(b(d, 4) & 0x3F) << 8);
        let v_active = 1 + u32::from(b(d, 5)) + (u32::from(b(d, 6) & 0x3F) << 8);
        let h_blank = 1 + u32::from(b(d, 7)) + (u32::from(b(d, 9) & 0x0F) << 8);
        let h_front = 1 + u32::from(b(d, 8)) + (u32::from(b(d, 9) & 0xF0) << 4);
        let h_sync = 1 + u32::from(b(d, 10));
        let v_blank = 1 + u32::from(b(d, 11));
        let v_front = 1 + u32::from(b(d, 12));
        let v_sync = 1 + u32::from(b(d, 13) & 0x0F);
        let interlaced = b(d, 13) & 0x80 != 0;
        let (h_total, v_total) = (h_active + h_blank, v_active + v_blank);
        self.push_timing(DidTiming {
            width: h_active,
            height: v_active,
            refresh_millihz: refresh(clock, h_total, v_total, interlaced),
            interlaced,
            preferred: b(d, 2) & 0x80 != 0,
            pixel_clock_khz: clock,
            h_total,
            v_total,
            timing_type: DidTimingType::VI,
            stereo: (b(d, 13) >> 5) & 3,
            aspect: None,
            detail: Some(DidDetail {
                h_blank,
                h_front,
                h_sync,
                v_blank,
                v_front,
                v_sync,
                h_sync_positive: b(d, 4) & 0x80 != 0,
                v_sync_positive: b(d, 6) & 0x80 != 0,
            }),
        });
    }
}

fn code_kind(v: u8) -> DidCodeKind {
    match v & 3 {
        1 => DidCodeKind::Vic,
        2 => DidCodeKind::HdmiVic,
        _ => DidCodeKind::Dmt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a 128-byte extension block from data blocks, with valid checksums.
    fn section(version: u8, ptype: u8, blocks: &[Vec<u8>]) -> Vec<u8> {
        let data: Vec<u8> = blocks.concat();
        let mut blk = vec![0u8; 128];
        blk[0] = 0x70;
        blk[1] = version;
        blk[2] = data.len() as u8;
        blk[3] = ptype;
        blk[5..5 + data.len()].copy_from_slice(&data);
        let end = 5 + data.len();
        let s: u32 = blk[1..end].iter().map(|&x| u32::from(x)).sum();
        blk[end] = ((256 - (s % 256)) % 256) as u8;
        let s: u32 = blk[..127].iter().map(|&x| u32::from(x)).sum();
        blk[127] = ((256 - (s % 256)) % 256) as u8;
        blk
    }

    fn db(tag: u8, rev: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![tag, rev, payload.len() as u8];
        v.extend_from_slice(payload);
        v
    }

    /// 1920x1080 at 60 Hz: 2200 x 1125 total, 148.5 MHz.
    fn type1_1080p(unit_khz: u32, flags: u8) -> Vec<u8> {
        let c = 148_500 / unit_khz - 1;
        let mut d = vec![c as u8, (c >> 8) as u8, (c >> 16) as u8, flags];
        for v in [
            1920u32 - 1,
            280 - 1,
            88 - 1,
            44 - 1,
            1080 - 1,
            45 - 1,
            4 - 1,
            5 - 1,
        ] {
            // active, blank, front, sync for h then v; front carries the polarity bit.
            d.extend_from_slice(&[v as u8, (v >> 8) as u8]);
        }
        d[9] |= 0x80; // h sync positive
        d[17] |= 0x80; // v sync positive
        d
    }

    #[test]
    fn header_and_checksum() {
        let blk = section(0x13, 3, &[db(0x0B, 0, b"hello")]);
        let d = parse_displayid(&blk);
        assert_eq!((d.version, d.product_type), (0x13, 3));
        assert!(d.checksum_valid && !d.truncated);
        assert_eq!(d.section_bytes, 8);
        assert_eq!(d.blocks.len(), 1);
        assert_eq!(d.blocks[0].data, DidBlockData::Text("hello".into()));
        let mut bad = blk.clone();
        bad[5 + 8] ^= 0xFF;
        assert!(!parse_displayid(&bad).checksum_valid);
    }

    #[test]
    fn section_overrun_is_truncated() {
        let mut blk = section(0x13, 3, &[db(0x0B, 0, b"hello")]);
        blk[7] = 50; // payload length beyond the section
        let d = parse_displayid(&blk);
        assert!(d.truncated);
        assert!(d.blocks.is_empty());
        let mut blk = section(0x13, 3, &[]);
        blk[2] = 200;
        assert!(parse_displayid(&blk).truncated);
        assert!(parse_displayid(&[0x70, 0x13]).truncated);
    }

    #[test]
    fn product_id_both_versions() {
        let mut p = vec![
            0x00, 0x00, 0x5E, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12, 10, 24, 4,
        ];
        p.extend_from_slice(b"Test");
        for tag in [0x00u8, 0x20] {
            let blk = section(if tag == 0 { 0x13 } else { 0x20 }, 3, &[db(tag, 0, &p)]);
            let d = parse_displayid(&blk);
            let pid = d.product_id().expect("product id");
            assert_eq!(pid.oui, [0, 0, 0x5E]);
            assert_eq!(pid.product_code, 0x1234);
            assert_eq!(pid.serial, 0x1234_5678);
            assert_eq!((pid.week, pid.year, pid.name.as_str()), (10, 2024, "Test"));
        }
        let d = parse_displayid(&section(0x13, 3, &[db(0x00, 0, &p[..6])]));
        assert!(d.blocks[0].malformed);
        assert!(d.product_id().is_none());
    }

    #[test]
    fn parameters_v1() {
        let p = [
            0x60, 0x09, 0x84, 0x05, 0x80, 0x07, 0x38, 0x04, 0x84, 120, 78, 0x77,
        ];
        let d = parse_displayid(&section(0x13, 3, &[db(0x01, 0, &p)]));
        let DidBlockData::Parameters(x) = &d.blocks[0].data else {
            panic!("not parameters");
        };
        assert_eq!((x.width_dmm, x.height_dmm), (2400, 1412));
        assert_eq!((x.h_pixels, x.v_pixels), (1920, 1080));
        assert_eq!(x.feature_flags, 0x84);
        assert!((x.gamma.unwrap() - 2.2).abs() < 1e-4);
        assert!((x.aspect - 1.78).abs() < 1e-4);
        assert_eq!((x.bpc_native, x.bpc_overall), (8, 8));
        let d = parse_displayid(&section(0x13, 3, &[db(0x01, 0, &p[..5])]));
        assert!(d.blocks[0].malformed);
    }

    #[test]
    fn type1_refresh_and_fields() {
        let t = type1_1080p(10, 0x84); // preferred, 16:9
        let d = parse_displayid(&section(0x13, 3, &[db(0x03, 0, &t)]));
        let tm: Vec<_> = d.timings().collect();
        assert_eq!(tm.len(), 1);
        let t = tm[0];
        assert_eq!((t.width, t.height), (1920, 1080));
        assert_eq!(t.refresh_millihz, 60_000);
        assert_eq!(
            (t.h_total, t.v_total, t.pixel_clock_khz),
            (2200, 1125, 148_500)
        );
        assert!(t.preferred && !t.interlaced);
        assert_eq!(t.aspect, Some((16, 9)));
        assert_eq!(t.timing_type, DidTimingType::I);
        let dt = t.detail.unwrap();
        assert_eq!(
            (dt.h_front, dt.h_sync, dt.v_front, dt.v_sync),
            (88, 44, 4, 5)
        );
        assert!(dt.h_sync_positive && dt.v_sync_positive);
        assert_eq!(d.blocks[0].data, DidBlockData::Timings { count: 1 });
    }

    #[test]
    fn type1_two_descriptors_and_short_payload() {
        let mut t = type1_1080p(10, 0x04);
        t.extend(type1_1080p(10, 0x04));
        let d = parse_displayid(&section(0x13, 3, &[db(0x03, 0, &t)]));
        assert_eq!(d.timings().count(), 2);
        let d = parse_displayid(&section(0x13, 3, &[db(0x03, 0, &t[..19])]));
        assert_eq!(d.timings().count(), 0);
        assert!(d.blocks[0].malformed);
    }

    #[test]
    fn type7_refresh_and_size_field() {
        let t = type1_1080p(1, 0x04);
        let d = parse_displayid(&section(0x20, 3, &[db(0x22, 0, &t)]));
        let t = d.timings().next().expect("timing");
        assert_eq!(t.refresh_millihz, 60_000);
        assert_eq!(t.pixel_clock_khz, 148_500);
        assert_eq!(t.timing_type, DidTimingType::VII);
        // Descriptor size grows with bits 6-4 of the revision byte.
        let mut t2 = type1_1080p(1, 0x04);
        t2.extend([0xEE, 0xEE]);
        let mut both = t2.clone();
        both.extend(t2);
        let d = parse_displayid(&section(0x20, 3, &[db(0x22, 0x20, &both)]));
        assert_eq!(d.timings().count(), 2);
    }

    #[test]
    fn interlaced_gives_field_rate() {
        // 1080i at 50 Hz fields: 74.25 MHz, 2640 x 1125 frame lines.
        let c = 7425u32 - 1;
        let mut t = vec![c as u8, (c >> 8) as u8, 0, 0x14];
        for v in [1919u32, 719, 527, 43, 1079, 44, 3, 4] {
            t.extend_from_slice(&[v as u8, (v >> 8) as u8]);
        }
        let d = parse_displayid(&section(0x13, 3, &[db(0x03, 0, &t)]));
        let t = d.timings().next().unwrap();
        assert!(t.interlaced);
        assert_eq!(t.refresh_millihz, 50_000);
    }

    #[test]
    fn type2_type3_type5_type6() {
        // Type II: 1024x768 (h 8+8*127, blank 8+8*40), 65 MHz.
        let c = 6500u32 - 1;
        let t2 = [
            c as u8,
            (c >> 8) as u8,
            0,
            0x80,
            127,
            (40 << 1) as u8,
            0x21,
            0xFF,
            ((768 - 1) >> 8) as u8,
            37,
            0x32,
        ];
        let d = parse_displayid(&section(0x13, 3, &[db(0x04, 0, &t2)]));
        let t = d.timings().next().unwrap();
        assert_eq!(
            (t.width, t.height, t.timing_type),
            (1024, 768, DidTimingType::II)
        );
        assert!(t.preferred);
        assert_eq!(t.h_total, 1024 + 328);
        // Type III: preferred, 16:9, width 1280, 60 Hz.
        let t3 = [0x84, (1280 / 8 - 1) as u8, 59];
        let d = parse_displayid(&section(0x13, 3, &[db(0x05, 0, &t3)]));
        let t = d.timings().next().unwrap();
        assert_eq!((t.width, t.height, t.refresh_millihz), (1280, 720, 60_000));
        assert!(t.preferred);
        // Type V.
        let t5 = [0x80, 0, 0x7F, 0x07, 0x37, 0x04, 119];
        let d = parse_displayid(&section(0x13, 3, &[db(0x11, 0, &t5)]));
        let t = d.timings().next().unwrap();
        assert_eq!(
            (t.width, t.height, t.refresh_millihz),
            (1920, 1080, 120_000)
        );
        // Type VI: 1920x1080, 148.5 MHz in 1 kHz units.
        let c = 148_500u32 - 1;
        let t6 = [
            c as u8,
            (c >> 8) as u8,
            (c >> 16) as u8 | 0x80,
            (1919 & 0xFF) as u8,
            (1919 >> 8) as u8,
            (1079 & 0xFF) as u8,
            (1079 >> 8) as u8,
            ((280 - 1) & 0xFF) as u8,
            87,
            ((280 - 1) >> 8) as u8,
            43,
            44,
            3,
            4,
        ];
        let d = parse_displayid(&section(0x13, 3, &[db(0x13, 0, &t6)]));
        let t = d.timings().next().unwrap();
        assert_eq!((t.width, t.height, t.refresh_millihz), (1920, 1080, 60_000));
        assert!(t.preferred);
        assert_eq!(t.timing_type, DidTimingType::VI);
    }

    #[test]
    fn type9_and_type10() {
        let t9 = [0x01, 0x7F, 0x07, 0x37, 0x04, 143];
        let d = parse_displayid(&section(0x20, 3, &[db(0x24, 0, &t9)]));
        let t = d.timings().next().unwrap();
        assert_eq!(
            (t.width, t.height, t.refresh_millihz),
            (1920, 1080, 144_000)
        );
        let t10 = [0x03, 0x7F, 0x07, 0x37, 0x04, 0xFF, 0x01];
        let d = parse_displayid(&section(0x20, 3, &[db(0x2A, 0x10, &t10)]));
        let t = d.timings().next().unwrap();
        assert_eq!(
            (t.width, t.height, t.refresh_millihz),
            (1920, 1080, 512_000)
        );
        assert_eq!(t.timing_type, DidTimingType::X);
        let d = parse_displayid(&section(0x20, 3, &[db(0x2A, 0x70, &t10)]));
        assert!(d.blocks[0].malformed);
    }

    #[test]
    fn timing_codes_and_bitmaps() {
        let d = parse_displayid(&section(
            0x13,
            3,
            &[db(0x06, 0x40, &[16, 35]), db(0x07, 0, &[0b101, 0x80])],
        ));
        let codes: Vec<_> = d.timing_codes.iter().map(|c| (c.kind, c.code)).collect();
        assert_eq!(
            codes,
            vec![
                (DidCodeKind::Vic, 16),
                (DidCodeKind::Vic, 35),
                (DidCodeKind::Dmt, 1),
                (DidCodeKind::Dmt, 3),
                (DidCodeKind::Dmt, 16)
            ]
        );
        assert_eq!(d.blocks[0].data, DidBlockData::TimingCodes { count: 2 });
        let d = parse_displayid(&section(0x20, 3, &[db(0x23, 0x08, &[0x2C, 0x01, 5, 0])]));
        let codes: Vec<_> = d.timing_codes.iter().map(|c| c.code).collect();
        assert_eq!(codes, vec![300, 5]);
        let d = parse_displayid(&section(0x13, 3, &[db(0x08, 0, &[0b11])]));
        assert_eq!(d.timing_codes.len(), 2);
    }

    #[test]
    fn range_limits_and_dynamic_range() {
        let mut p = vec![];
        p.extend_from_slice(&[0x00, 0x00, 0x00]); // min 0.01 MHz units: (0+1)*10 kHz
        p.extend_from_slice(&[0xE7, 0x39, 0x00]); // 14823+1 = 14824 -> 148.24 MHz
        p.extend_from_slice(&[30, 135, 0x50, 0x00, 48, 144, 0x1E, 0x00, 0xE0]);
        let d = parse_displayid(&section(0x13, 3, &[db(0x09, 0, &p)]));
        let DidBlockData::RangeLimits(r) = &d.blocks[0].data else {
            panic!()
        };
        assert_eq!(
            (r.min_pixel_clock_khz, r.max_pixel_clock_khz),
            (10, 148_240)
        );
        assert_eq!((r.min_v_hz, r.max_v_hz, r.min_h_blank), (48, 144, 80));
        assert!(r.interlaced && r.cvt && r.cvt_reduced && !r.discrete);
        // Dynamic range 2.x rev 1: max refresh uses 10 bits, seamless in bit 7.
        let p = [0x00, 0x00, 0x00, 0xFF, 0xFF, 0x03, 48, 0x2C, 0x81];
        let d = parse_displayid(&section(0x20, 3, &[db(0x25, 1, &p)]));
        let r = d.dynamic_range().expect("range");
        assert_eq!(
            (r.min_refresh_millihz, r.max_refresh_millihz),
            (48_000, 300_000)
        );
        assert!(r.seamless);
        assert_eq!((r.min_pixel_clock_khz, r.max_pixel_clock_khz), (1, 262_144));
        let d = parse_displayid(&section(0x20, 3, &[db(0x25, 0, &p)]));
        assert_eq!(d.dynamic_range().unwrap().max_refresh_millihz, 44_000);
        let d = parse_displayid(&section(0x20, 3, &[db(0x25, 0, &p[..5])]));
        assert!(d.blocks[0].malformed);
    }

    #[test]
    fn adaptive_sync_descriptors() {
        let p = [0x01, 8, 48, 143, 0, 4, 0x00, 4, 30, 59, 0, 0];
        let d = parse_displayid(&section(0x20, 3, &[db(0x2B, 0, &p)]));
        let a: Vec<_> = d.adaptive_sync().collect();
        assert_eq!(a.len(), 2);
        assert!(a[0].native_range && a[0].seamless);
        assert_eq!(
            (a[0].min_refresh_millihz, a[0].max_refresh_millihz),
            (48_000, 144_000)
        );
        assert!((a[0].max_duration_increase_ms - 2.0).abs() < 1e-6);
        assert_eq!(
            (a[1].min_refresh_millihz, a[1].max_refresh_millihz),
            (30_000, 60_000)
        );
        let d = parse_displayid(&section(0x20, 3, &[db(0x2B, 0, &p[..7])]));
        assert!(d.blocks[0].malformed);
        assert_eq!(d.adaptive_sync().count(), 1);
    }

    #[test]
    fn tiled_topology() {
        let mut p = vec![
            0x80 | 0x40 | 0x01,
            0x11,
            0x10,
            0x00,
            0xFF,
            0x0F,
            0x37,
            0x04,
            10,
            5,
            6,
            7,
            8,
        ];
        p.extend_from_slice(&[b'A', b'B', b'C', 0x34, 0x12, 0x01, 0, 0, 0]);
        assert_eq!(p.len(), 22);
        let d = parse_displayid(&section(0x13, 3, &[db(0x12, 0, &p)]));
        let t = d.tiles().next().expect("tile");
        assert_eq!((t.h_tiles, t.v_tiles), (2, 2));
        assert_eq!((t.h_location, t.v_location), (1, 0));
        assert_eq!((t.tile_width, t.tile_height), (4096, 1080));
        assert!(t.single_enclosure);
        assert_eq!(t.bezels, Some([5.0, 6.0, 7.0, 8.0]));
        assert_eq!(t.vendor, *b"ABC");
        assert_eq!((t.product_code, t.serial), (0x1234, 1));
        let d = parse_displayid(&section(0x13, 3, &[db(0x12, 0, &p[..21])]));
        assert!(d.blocks[0].malformed);
    }

    #[test]
    fn interface_and_features() {
        let p = [0xA2, 0x12, 0x0E, 0x0E, 0x07, 0x01, 0x22, 0x00, 0x01, 0x02];
        let d = parse_displayid(&section(0x13, 3, &[db(0x0F, 0, &p)]));
        let DidBlockData::Interface(i) = &d.blocks[0].data else {
            panic!()
        };
        assert_eq!((i.interface_type, i.links, i.version), (0xA, 2, (1, 2)));
        assert_eq!(bpc_list_444(i.bpc_rgb), vec![8, 10, 12]);
        assert_eq!(bpc_list_4xx(i.bpc_ycbcr422), vec![8, 10, 12]);
        assert_eq!((i.content_protection, i.cp_version), (1, (2, 2)));
        assert_eq!(i.type_flags, vec![1, 2]);
        // 0x26: RGB 8/10, 444 8/10, 422 8, 420 none, audio 48 kHz, one extra combination.
        let p = [0x06, 0x06, 0x01, 0x00, 0x00, 0x04, 0x41, 0x00, 0x01, 0x68];
        let d = parse_displayid(&section(0x20, 3, &[db(0x26, 0, &p)]));
        let f = d.interface_features().expect("features");
        assert_eq!(bpc_list_444(f.bpc_rgb), vec![8, 10]);
        assert_eq!(f.audio_rates, 4);
        assert_eq!(f.combos1, 0x41);
        assert_eq!(f.additional, vec![(6, 8)]);
        // Too many additional combos for the payload.
        let d = parse_displayid(&section(
            0x20,
            3,
            &[db(0x26, 0, &[0, 0, 0, 0, 0, 0, 0, 0, 3, 0x11])],
        ));
        assert!(d.blocks[0].malformed);
    }

    #[test]
    fn parameters_v2_values() {
        let mut p = vec![0u8; 29];
        p[..4].copy_from_slice(&[0x60, 0x09, 0x84, 0x05]); // 240.0 x 141.2 mm
        p[4..8].copy_from_slice(&[0x00, 0x0F, 0xC0, 0x08]); // 3840 x 2240? use 2160 below
        p[6..8].copy_from_slice(&[0x70, 0x08]);
        p[8] = 0x40;
        // Luminance 400 cd/m2: 400 = 2^8 * 1.5625 -> exp 23 -> field 23, mantissa 0.5625*1024=576.
        let lum = (23u16 << 10) | 576;
        p[21..23].copy_from_slice(&lum.to_le_bytes());
        p[27] = 0x03 | (2 << 4) | 0x80; // 10 bpc, OLED, dark theme
        p[28] = 120;
        let d = parse_displayid(&section(0x20, 3, &[db(0x21, 1, &p)]));
        let v = d.parameters_v2().expect("params");
        assert!((v.width_mm - 240.0).abs() < 1e-3);
        assert_eq!((v.native_width, v.native_height), (3840, 2160));
        assert!(v.cie_1976 && v.speakers_integrated);
        assert!((v.max_luminance_full.unwrap() - 400.0).abs() < 0.5);
        assert_eq!(v.native_bpc, Some(10));
        assert_eq!(v.technology, 2);
        assert!(v.dark_theme);
        assert!((v.gamma.unwrap() - 2.2).abs() < 1e-4);
        let d = parse_displayid(&section(0x20, 3, &[db(0x21, 1, &p[..28])]));
        assert!(d.blocks[0].malformed);
    }

    #[test]
    fn misc_blocks() {
        let d = parse_displayid(&section(
            0x20,
            3,
            &[
                db(0x29, 0, &[7u8; 16]),
                db(0x7E, 0, &[0x3A, 0x02, 0x92, 1, 2]),
                db(0x81, 0, &[0x03, 0x0C, 0x00, 0x10]),
                db(0x2C, 1, &[0u8; 10]),
                db(0x55, 0, &[1, 2, 3]),
            ],
        ));
        assert_eq!(d.blocks.len(), 5);
        assert_eq!(d.blocks[0].data, DidBlockData::ContainerId([7; 16]));
        assert_eq!(
            d.blocks[1].data,
            DidBlockData::Vendor {
                oui: [0x3A, 0x02, 0x92]
            }
        );
        assert_eq!(d.blocks[2].data, DidBlockData::CtaData { len: 4 });
        assert_eq!(d.blocks[3].data, DidBlockData::Raw);
        assert_eq!(d.blocks[4].tag, 0x55);
        assert!(d.has_block(0x2C));
    }

    #[test]
    fn device_power_color_transfer() {
        let p = [
            0x15, 0x50, 0x7F, 0x07, 0x37, 0x04, 78, 0x00, 0x01, 15, 15, 0x08, 0x85,
        ];
        let p: Vec<u8> = p.to_vec();
        let d = parse_displayid(&section(0x13, 3, &[db(0x0C, 0, &p)]));
        let DidBlockData::DisplayDevice(x) = &d.blocks[0].data else {
            panic!()
        };
        assert_eq!((x.native_width, x.native_height), (1920, 1080));
        assert_eq!(
            (x.bit_depth, x.response_ms, x.response_white_to_black),
            (8, 5, true)
        );
        let d = parse_displayid(&section(0x13, 3, &[db(0x0D, 0, &[0x24, 5, 5, 3, 2, 2])]));
        let DidBlockData::PowerSequencing(x) = &d.blocks[0].data else {
            panic!()
        };
        assert!((x.t1_min_ms - 0.2).abs() < 1e-6 && x.t1_max_ms == 8.0 && x.t4_min_ms == 30.0);
        // Colour characteristics: sRGB-style three primaries and one white point.
        let mut c = vec![0x33u8];
        c.extend([0x40u8; 18]);
        let d = parse_displayid(&section(0x13, 3, &[db(0x02, 0, &c)]));
        let DidBlockData::ColorCharacteristics(x) = &d.blocks[0].data else {
            panic!()
        };
        assert_eq!((x.primaries.len(), x.white_points.len()), (3, 3));
        assert!(!d.blocks[0].malformed);
        let d = parse_displayid(&section(0x13, 3, &[db(0x02, 0, &c[..6])]));
        assert!(d.blocks[0].malformed);
        // Transfer: one curve of 4 samples (3 stored) and a four-parameter form.
        let d = parse_displayid(&section(
            0x13,
            3,
            &[db(0x0E, 0x10, &[0x00, 4, 100, 100, 100])],
        ));
        let DidBlockData::Transfer(x) = &d.blocks[0].data else {
            panic!()
        };
        assert_eq!(x.id, 1);
        assert_eq!(x.curves[0].len(), 4);
        assert!((x.curves[0][3] - 100.0).abs() < 1e-6);
        let d = parse_displayid(&section(
            0x13,
            3,
            &[db(0x0E, 0, &[0x20, 5, 1, 2, 3, 4, 120])],
        ));
        let DidBlockData::Transfer(x) = &d.blocks[0].data else {
            panic!()
        };
        assert_eq!(x.params, vec![[1, 2, 3, 4, 120]]);
    }

    #[test]
    fn stereo_block_with_codes() {
        // method 1 (field sequential), one param byte, then one VIC group of two codes.
        let p = [2, 1, 1, 0x82, 16, 31];
        let d = parse_displayid(&section(0x13, 3, &[db(0x10, 0x40, &p)]));
        let DidBlockData::Stereo(s) = &d.blocks[0].data else {
            panic!()
        };
        assert_eq!((s.method, s.params.clone()), (1, vec![1]));
        assert_eq!(d.timing_codes.len(), 2);
        assert_eq!(d.timing_codes[0].kind, DidCodeKind::HdmiVic);
    }

    #[test]
    fn padding_stops_parsing_and_agrees_with_redactor_layout() {
        // Product id, then zero padding inside the declared section length.
        let p = [0, 0, 0x5E, 1, 0, 9, 9, 9, 9, 1, 2, 0];
        let ext = section(0x13, 3, &[db(0x00, 0, &p), vec![0; 6]]);
        let d = parse_displayid(&ext);
        assert_eq!(d.blocks.len(), 1);
        assert!(d.checksum_valid);
        assert_eq!(d.product_id().unwrap().serial, 0x0909_0909);
        // edid.rs's redactor clears the same serial and keeps the section checksum valid.
        let mut edid = vec![0u8; 128];
        edid[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        edid[126] = 1;
        edid.extend_from_slice(&ext);
        crate::edid::redact_serials(&mut edid);
        let d = parse_displayid(&edid[128..]);
        assert_eq!(d.product_id().unwrap().serial, 0);
        assert!(d.checksum_valid);
        assert_eq!(d.product_id().unwrap().product_code, 1);
    }

    #[test]
    fn never_panics_on_truncation_and_noise() {
        let blk = section(
            0x20,
            3,
            &[
                db(
                    0x20,
                    0,
                    &[0, 0, 0x5E, 1, 0, 9, 9, 9, 9, 1, 2, 2, b'a', b'b'],
                ),
                db(0x22, 0, &type1_1080p(1, 0)),
                db(0x2B, 0, &[1, 2, 3, 4, 5, 6]),
                db(0x12, 0, &[0u8; 22]),
                db(0x13, 0, &[0u8; 14]),
                db(0x0E, 0, &[0x20, 9, 9]),
            ],
        );
        for n in 0..=blk.len() {
            let _ = parse_displayid(&blk[..n]);
        }
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        };
        for _ in 0..4000 {
            let mut b2 = blk.clone();
            for _ in 0..6 {
                let i = usize::from(next()) % b2.len();
                b2[i] = next();
            }
            let _ = parse_displayid(&b2);
            let mut rnd: Vec<u8> = (0..128).map(|_| next()).collect();
            rnd[1] = 0x20;
            rnd[2] = next() % 122;
            let _ = parse_displayid(&rnd);
        }
    }
}
