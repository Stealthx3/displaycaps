// SPDX-License-Identifier: LGPL-2.1-or-later
//! Video timings shared by every EDID structure: detailed timing descriptors (base block, CTA-861
//! and DisplayID type I share the layout), the base block's video section (input definition,
//! established, standard and detailed timings, display descriptors including range limits and
//! CVT 3-byte codes), the VESA DMT table, and the CVT and GTF formulas that give standard timings
//! a pixel clock. Pure functions over bytes; never panics on any input.

use crate::cvt;
use crate::dmt;

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

/// Analog video input definition (EDID byte 0x14, bit 7 clear).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnalogInput {
    /// Signal level standard, 0-3: 0.700/0.300 V, 0.714/0.286 V, 1.000/0.400 V, 0.700/0.000 V
    /// (video/sync, peak-to-peak 1.000, 1.000, 1.400, 0.700).
    pub signal_level: u8,
    /// Blank-to-black setup (pedestal) expected.
    pub blank_to_black_setup: bool,
    /// Separate sync supported.
    pub separate_sync: bool,
    /// Composite sync (on horizontal) supported.
    pub composite_sync: bool,
    /// Sync on green supported.
    pub sync_on_green: bool,
    /// Serration on the vertical sync supported.
    pub vsync_serration: bool,
}

/// The digital interface of an EDID 1.4 digital display.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DigitalInterface {
    /// Not defined (also every pre-1.4 digital EDID).
    #[default]
    Undefined,
    /// DVI.
    Dvi,
    /// HDMI-a.
    HdmiA,
    /// HDMI-b.
    HdmiB,
    /// MDDI.
    Mddi,
    /// DisplayPort.
    DisplayPort,
    /// A reserved code.
    Reserved(u8),
}

/// Digital video input definition (EDID byte 0x14, bit 7 set).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DigitalInput {
    /// Bits per colour channel (6, 8, 10, 12, 14 or 16); `None` when undefined or before EDID 1.4.
    pub bits_per_color: Option<u8>,
    /// The interface (EDID 1.4 only).
    pub interface: DigitalInterface,
    /// DFP 1.x compatible TMDS (EDID 1.2 and 1.3, bit 0).
    pub dfp_compatible: bool,
}

/// The video input definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoInput {
    /// An analog display.
    Analog(AnalogInput),
    /// A digital display.
    Digital(DigitalInput),
}

impl Default for VideoInput {
    fn default() -> Self {
        VideoInput::Digital(DigitalInput::default())
    }
}

impl VideoInput {
    /// Whether this is a digital input.
    pub fn is_digital(&self) -> bool {
        matches!(self, VideoInput::Digital(_))
    }
}

/// The maximum image size or aspect ratio (bytes 0x15 and 0x16).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ScreenSize {
    /// Both bytes zero (or one zero before EDID 1.4): variable or undefined.
    #[default]
    Variable,
    /// Maximum horizontal and vertical image size in centimetres.
    Cm {
        /// Horizontal size in cm.
        width: u8,
        /// Vertical size in cm.
        height: u8,
    },
    /// Landscape aspect ratio, width over height (EDID 1.4, vertical byte zero).
    AspectLandscape(f32),
    /// Portrait aspect ratio, width over height (EDID 1.4, horizontal byte zero).
    AspectPortrait(f32),
}

/// The colour type or supported colour encodings (feature byte 0x18, bits 4:3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DisplayColor {
    /// Undefined.
    #[default]
    Undefined,
    /// Monochrome or grayscale.
    Monochrome,
    /// RGB colour display.
    Rgb,
    /// Non-RGB colour display.
    NonRgb,
    /// Digital EDID 1.4: RGB 4:4:4 plus the listed YCrCb encodings.
    Digital {
        /// YCrCb 4:4:4 supported.
        ycbcr444: bool,
        /// YCrCb 4:2:2 supported.
        ycbcr422: bool,
    },
}

/// The feature support byte (0x18), decoded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Features {
    /// The raw byte.
    pub raw: u8,
    /// DPMS standby.
    pub dpms_standby: bool,
    /// DPMS suspend.
    pub dpms_suspend: bool,
    /// DPMS active-off.
    pub dpms_active_off: bool,
    /// Colour type or encodings.
    pub color: DisplayColor,
    /// The default (sRGB) colour space is the primary colour space.
    pub srgb_default: bool,
    /// The first detailed timing is the preferred timing (always true from EDID 1.4).
    pub has_preferred_timing: bool,
    /// EDID 1.4: the preferred timing includes the native pixel format and refresh. Before 1.4
    /// this repeats the preferred-timing bit.
    pub preferred_includes_native: bool,
    /// EDID 1.4: continuous frequency. Before 1.4: GTF timings supported within the range limits.
    pub continuous_frequency: bool,
}

/// A CIE 1931 chromaticity coordinate.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Xy {
    /// x.
    pub x: f32,
    /// y.
    pub y: f32,
}

/// The display's primaries and white point (bytes 0x19-0x22, 10-bit fixed point).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Chromaticity {
    /// Red.
    pub red: Xy,
    /// Green.
    pub green: Xy,
    /// Blue.
    pub blue: Xy,
    /// White point.
    pub white: Xy,
}

/// Established timings I and II (bytes 0x23-0x25).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EstablishedTimings {
    /// Byte 0x23: 720x400@70, 720x400@88, 640x480@60, 640x480@67, 640x480@72, 640x480@75,
    /// 800x600@56, 800x600@60 (bit 7 first).
    pub byte_23: u8,
    /// Byte 0x24: 800x600@72, 800x600@75, 832x624@75, 1024x768i@87, 1024x768@60, @70, @75,
    /// 1280x1024@75 (bit 7 first).
    pub byte_24: u8,
    /// Byte 0x25: bit 7 is 1152x870@75, the rest is the manufacturer's timings byte.
    pub manufacturer: u8,
}

/// The aspect ratio of a standard timing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StdAspect {
    /// 1:1 (code 00 before EDID 1.3).
    #[default]
    Ratio1x1,
    /// 16:10 (code 00 from EDID 1.3).
    Ratio16x10,
    /// 4:3.
    Ratio4x3,
    /// 5:4.
    Ratio5x4,
    /// 16:9.
    Ratio16x9,
}

impl StdAspect {
    /// Width and height of the ratio.
    pub fn ratio(self) -> (u16, u16) {
        match self {
            StdAspect::Ratio1x1 => (1, 1),
            StdAspect::Ratio16x10 => (16, 10),
            StdAspect::Ratio4x3 => (4, 3),
            StdAspect::Ratio5x4 => (5, 4),
            StdAspect::Ratio16x9 => (16, 9),
        }
    }
}

/// A standard timing (two bytes in the base block or a 0xFA descriptor).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StandardTiming {
    /// The two bytes as stored.
    pub code: [u8; 2],
    /// Horizontal active pixels.
    pub h_active: u16,
    /// Vertical active lines (from the aspect ratio, truncated).
    pub v_active: u16,
    /// Aspect ratio.
    pub aspect: StdAspect,
    /// Vertical refresh in Hz (60-123).
    pub refresh_hz: u8,
}

/// An aspect ratio named by a CVT range descriptor or CVT timing code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CvtAspect {
    /// 4:3.
    R4x3,
    /// 16:9.
    R16x9,
    /// 16:10.
    R16x10,
    /// 5:4.
    R5x4,
    /// 15:9.
    R15x9,
}

impl CvtAspect {
    /// Width and height of the ratio.
    pub fn ratio(self) -> (u32, u32) {
        match self {
            CvtAspect::R4x3 => (4, 3),
            CvtAspect::R16x9 => (16, 9),
            CvtAspect::R16x10 => (16, 10),
            CvtAspect::R5x4 => (5, 4),
            CvtAspect::R15x9 => (15, 9),
        }
    }
}

/// The set of aspect ratios a CVT range descriptor supports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CvtAspects {
    /// 4:3.
    pub r4x3: bool,
    /// 16:9.
    pub r16x9: bool,
    /// 16:10.
    pub r16x10: bool,
    /// 5:4.
    pub r5x4: bool,
    /// 15:9.
    pub r15x9: bool,
}

/// The CVT fields of a range-limits descriptor (bytes 11-17).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CvtRange {
    /// CVT version (major, minor), byte 11.
    pub version: (u8, u8),
    /// Pixel clock precision: the real maximum clock is the descriptor's maximum minus this many
    /// quarter-MHz steps.
    pub clock_offset_quarter_mhz: u8,
    /// Maximum active pixels per line; 0 when unlimited.
    pub max_h_active: u16,
    /// Supported aspect ratios.
    pub aspects: CvtAspects,
    /// Preferred aspect ratio; `None` for a reserved code.
    pub preferred_aspect: Option<CvtAspect>,
    /// Standard blanking supported.
    pub standard_blanking: bool,
    /// Reduced blanking supported.
    pub reduced_blanking: bool,
    /// Horizontal shrink supported.
    pub h_shrink: bool,
    /// Horizontal stretch supported.
    pub h_stretch: bool,
    /// Vertical shrink supported.
    pub v_shrink: bool,
    /// Vertical stretch supported.
    pub v_stretch: bool,
    /// Preferred vertical refresh in Hz; 0 when not stated.
    pub preferred_refresh_hz: u8,
}

/// The secondary GTF curve of a range-limits descriptor (bytes 12-17).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SecondaryGtf {
    /// Start of the secondary curve in kHz of horizontal frequency.
    pub start_khz: u16,
    /// C in percent.
    pub c: f32,
    /// M in percent per kHz.
    pub m: u16,
    /// K.
    pub k: u8,
    /// J in percent.
    pub j: f32,
}

impl SecondaryGtf {
    /// The curve as GTF parameters.
    pub fn params(&self) -> cvt::GtfParams {
        cvt::GtfParams {
            c: f64::from(self.c),
            m: f64::from(self.m),
            k: f64::from(self.k),
            j: f64::from(self.j),
        }
    }
}

/// The timing formula a range-limits descriptor names (byte 10).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum RangeFormula {
    /// Default GTF supported.
    #[default]
    DefaultGtf,
    /// Range limits only (EDID 1.4).
    RangeLimitsOnly,
    /// Secondary GTF curve; `None` when the six curve bytes are all zero.
    SecondaryGtf(Option<SecondaryGtf>),
    /// CVT, with its fields.
    Cvt(CvtRange),
    /// Any other byte value.
    Unknown(u8),
}

impl RangeFormula {
    /// The name the model's `RangeLimits::formula` uses: `default_gtf`, `range_only`,
    /// `secondary_gtf` or `cvt`; `None` for an unknown byte.
    pub fn name(&self) -> Option<&'static str> {
        match self {
            RangeFormula::DefaultGtf => Some("default_gtf"),
            RangeFormula::RangeLimitsOnly => Some("range_only"),
            RangeFormula::SecondaryGtf(_) => Some("secondary_gtf"),
            RangeFormula::Cvt(_) => Some("cvt"),
            RangeFormula::Unknown(_) => None,
        }
    }
}

/// A Display Range Limits descriptor (0xFD), with the EDID 1.4 +255 offsets already applied.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RangeLimitsDescriptor {
    /// Lowest vertical rate in Hz.
    pub min_vertical_hz: u16,
    /// Highest vertical rate in Hz.
    pub max_vertical_hz: u16,
    /// Lowest horizontal rate in kHz.
    pub min_horizontal_khz: u16,
    /// Highest horizontal rate in kHz.
    pub max_horizontal_khz: u16,
    /// Highest pixel clock in kHz (the descriptor stores units of 10 MHz); `None` when zero.
    pub max_pixel_clock_khz: Option<u32>,
    /// The named timing formula.
    pub formula: RangeFormula,
}

impl RangeLimitsDescriptor {
    /// The CVT fields, if the descriptor names CVT.
    pub fn cvt(&self) -> Option<&CvtRange> {
        match &self.formula {
            RangeFormula::Cvt(c) => Some(c),
            _ => None,
        }
    }
}

/// One colour point of a 0xFB descriptor.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ColorPoint {
    /// The white point index.
    pub index: u8,
    /// The white point.
    pub white: Xy,
    /// Gamma; `None` when defined in an extension block (0xFF).
    pub gamma: Option<f32>,
}

/// Display colour management data (0xF9): the a3 and a2 polynomial coefficients.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ColorManagement {
    /// Version.
    pub version: u8,
    /// Red a3.
    pub red_a3: f32,
    /// Red a2.
    pub red_a2: f32,
    /// Green a3.
    pub green_a3: f32,
    /// Green a2.
    pub green_a2: f32,
    /// Blue a3.
    pub blue_a3: f32,
    /// Blue a2.
    pub blue_a2: f32,
}

/// A refresh rate a CVT timing code can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CvtRate {
    /// 50 Hz, standard blanking.
    Hz50,
    /// 60 Hz, standard blanking.
    Hz60,
    /// 75 Hz, standard blanking.
    Hz75,
    /// 85 Hz, standard blanking.
    Hz85,
    /// 60 Hz, reduced blanking.
    Hz60Reduced,
}

impl CvtRate {
    /// The refresh in Hz.
    pub fn hz(self) -> u32 {
        match self {
            CvtRate::Hz50 => 50,
            CvtRate::Hz60 | CvtRate::Hz60Reduced => 60,
            CvtRate::Hz75 => 75,
            CvtRate::Hz85 => 85,
        }
    }

    /// Whether the rate uses reduced blanking.
    pub fn reduced(self) -> bool {
        self == CvtRate::Hz60Reduced
    }
}

/// One 3-byte CVT timing code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CvtTimingCode {
    /// Addressable lines.
    pub v_active: u16,
    /// Addressable pixels per line (from the aspect ratio, multiple of 8).
    pub h_active: u16,
    /// Aspect ratio.
    pub aspect: CvtAspect,
    /// Supported rates in the order 50, 60, 75, 85 Hz, 60 Hz reduced blanking.
    pub rates: [bool; 5],
    /// The preferred rate, if the code is consistent.
    pub preferred: Option<CvtRate>,
}

impl CvtTimingCode {
    /// The supported rates, in the order 50, 60, 75, 85, 60 reduced.
    pub fn supported(&self) -> Vec<CvtRate> {
        const ALL: [CvtRate; 5] = [
            CvtRate::Hz50,
            CvtRate::Hz60,
            CvtRate::Hz75,
            CvtRate::Hz85,
            CvtRate::Hz60Reduced,
        ];
        ALL.iter()
            .zip(self.rates.iter())
            .filter(|(_, on)| **on)
            .map(|(r, _)| *r)
            .collect()
    }
}

/// The 0xF8 descriptor: up to four CVT 3-byte timing codes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CvtCodes {
    /// Version byte (1 for the defined layout).
    pub version: u8,
    /// The codes that are not all zero.
    pub codes: Vec<CvtTimingCode>,
}

/// Established timings III (0xF7): bits for 44 DMT timings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EstablishedIii(pub [u8; 6]);

/// DMT IDs of the established timings III bits, most significant bit of byte 6 first.
const EST3_DMT: [u8; 44] = [
    0x01, 0x02, 0x03, 0x07, 0x0e, 0x0c, 0x13, 0x15, // byte 6
    0x16, 0x17, 0x18, 0x19, 0x20, 0x21, 0x23, 0x25, // byte 7
    0x27, 0x2e, 0x2f, 0x30, 0x31, 0x29, 0x2a, 0x2b, // byte 8
    0x2c, 0x39, 0x3a, 0x3b, 0x3c, 0x33, 0x34, 0x35, // byte 9
    0x36, 0x37, 0x3e, 0x3f, 0x41, 0x42, 0x44, 0x45, // byte 10
    0x46, 0x47, 0x49, 0x4a, // byte 11, bits 7-4
];

impl EstablishedIii {
    /// The DMT IDs of the set bits.
    pub fn dmt_ids(&self) -> Vec<u8> {
        EST3_DMT
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                self.0
                    .get(i / 8)
                    .is_some_and(|b| b & (0x80 >> (i % 8)) != 0)
            })
            .map(|(_, id)| *id)
            .collect()
    }
}

/// A display descriptor of the base block (an 18-byte slot whose first two bytes are zero).
#[derive(Debug, Clone, PartialEq)]
pub enum Descriptor {
    /// 0xFF: display product serial number text.
    SerialNumber(String),
    /// 0xFE: unspecified text.
    Text(String),
    /// 0xFC: display product name.
    ProductName(String),
    /// 0xFD: display range limits.
    RangeLimits(RangeLimitsDescriptor),
    /// 0xFB: colour point data (one or two points).
    ColorPoints(Vec<ColorPoint>),
    /// 0xFA: six more standard timings.
    StandardTimings(Vec<StandardTiming>),
    /// 0xF9: display colour management data.
    ColorManagement(ColorManagement),
    /// 0xF8: CVT 3-byte timing codes.
    CvtCodes(CvtCodes),
    /// 0xF7: established timings III.
    EstablishedIii(EstablishedIii),
    /// 0x10: dummy descriptor.
    Dummy,
    /// Any other tag (manufacturer-specific 0x00-0x0F, EPI 0x0E, reserved), kept as its tag.
    Other {
        /// The descriptor tag (byte 3).
        tag: u8,
    },
}

/// The base block's video section.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BaseVideo {
    /// EDID version and revision (bytes 18 and 19).
    pub version: (u8, u8),
    /// Video input definition.
    pub input: VideoInput,
    /// Maximum image size or aspect ratio.
    pub screen_size: ScreenSize,
    /// Display gamma; `None` when defined in an extension block (0xFF).
    pub gamma: Option<f32>,
    /// Feature support.
    pub features: Features,
    /// Chromaticity coordinates.
    pub chromaticity: Chromaticity,
    /// Established timings I and II.
    pub established: EstablishedTimings,
    /// The standard timings in use (unused 0x0101 slots omitted), in order.
    pub standard: Vec<StandardTiming>,
    /// Detailed timings from the four descriptor slots, in order (the first is preferred).
    pub detailed: Vec<DetailedTiming>,
    /// The display descriptors from the four descriptor slots, in order.
    pub descriptors: Vec<Descriptor>,
}

impl BaseVideo {
    /// The first range-limits descriptor.
    pub fn range_limits(&self) -> Option<&RangeLimitsDescriptor> {
        self.descriptors.iter().find_map(|d| match d {
            Descriptor::RangeLimits(r) => Some(r),
            _ => None,
        })
    }

    /// The display product name text (0xFC).
    pub fn product_name(&self) -> Option<&str> {
        self.descriptors.iter().find_map(|d| match d {
            Descriptor::ProductName(s) => Some(s.as_str()),
            _ => None,
        })
    }

    /// The serial number text descriptor (0xFF).
    pub fn serial_text(&self) -> Option<&str> {
        self.descriptors.iter().find_map(|d| match d {
            Descriptor::SerialNumber(s) => Some(s.as_str()),
            _ => None,
        })
    }

    /// The preferred timing: the first detailed timing.
    pub fn preferred(&self) -> Option<&DetailedTiming> {
        self.detailed.first()
    }

    /// Whether the display signals CVT support through its range limits (EDID 1.4).
    pub fn supports_cvt(&self) -> bool {
        self.version >= (1, 4) && self.range_limits().is_some_and(|r| r.cvt().is_some())
    }
}

fn xy(x_hi: u8, x_lo: u8, y_hi: u8, y_lo: u8) -> Xy {
    let x = (u16::from(x_hi) << 2) | u16::from(x_lo & 3);
    let y = (u16::from(y_hi) << 2) | u16::from(y_lo & 3);
    Xy {
        x: f32::from(x) / 1024.0,
        y: f32::from(y) / 1024.0,
    }
}

fn parse_standard_timing(b1: u8, b2: u8, minor: u8) -> Option<StandardTiming> {
    if b1 <= 0x01 {
        return None;
    }
    let h_active = (u16::from(b1) + 31) * 8;
    let aspect = match b2 >> 6 {
        0 if minor >= 3 => StdAspect::Ratio16x10,
        0 => StdAspect::Ratio1x1,
        1 => StdAspect::Ratio4x3,
        2 => StdAspect::Ratio5x4,
        _ => StdAspect::Ratio16x9,
    };
    let (rw, rh) = aspect.ratio();
    Some(StandardTiming {
        code: [b1, b2],
        h_active,
        v_active: (u32::from(h_active) * u32::from(rh) / u32::from(rw)) as u16,
        aspect,
        refresh_hz: (b2 & 0x3f) + 60,
    })
}

fn descriptor_text(d: &[u8]) -> String {
    let mut s = String::new();
    for &b in d.get(5..18).unwrap_or(&[]) {
        if b == 0x0a || b == 0x00 || b == 0xff {
            break;
        }
        s.push(if (0x20..0x7f).contains(&b) {
            char::from(b)
        } else {
            '.'
        });
    }
    s.trim_end().to_string()
}

fn cvt_aspect_bits(v: u8) -> CvtAspect {
    match v & 3 {
        0 => CvtAspect::R4x3,
        1 => CvtAspect::R16x9,
        2 => CvtAspect::R16x10,
        _ => CvtAspect::R15x9,
    }
}

fn parse_range_limits(d: &[u8; 18], minor: u8) -> RangeLimitsDescriptor {
    let (mut v_max_off, mut v_min_off, mut h_max_off, mut h_min_off) = (0u16, 0u16, 0u16, 0u16);
    if minor >= 4 {
        if d[4] & 0x02 != 0 {
            v_max_off = 255;
            if d[4] & 0x01 != 0 {
                v_min_off = 255;
            }
        }
        if d[4] & 0x08 != 0 {
            h_max_off = 255;
            if d[4] & 0x04 != 0 {
                h_min_off = 255;
            }
        }
    }
    let formula = match d[10] {
        0x00 => RangeFormula::DefaultGtf,
        0x01 => RangeFormula::RangeLimitsOnly,
        0x02 => {
            if d[12..18].iter().all(|&b| b == 0) {
                RangeFormula::SecondaryGtf(None)
            } else {
                RangeFormula::SecondaryGtf(Some(SecondaryGtf {
                    start_khz: u16::from(d[12]) * 2,
                    c: f32::from(d[13]) / 2.0,
                    m: u16::from_le_bytes([d[14], d[15]]),
                    k: d[16],
                    j: f32::from(d[17]) / 2.0,
                }))
            }
        }
        0x04 => RangeFormula::Cvt(CvtRange {
            version: (d[11] >> 4, d[11] & 0x0f),
            clock_offset_quarter_mhz: d[12] >> 2,
            max_h_active: ((u16::from(d[12] & 3) << 8) | u16::from(d[13])) * 8,
            aspects: CvtAspects {
                r4x3: d[14] & 0x80 != 0,
                r16x9: d[14] & 0x40 != 0,
                r16x10: d[14] & 0x20 != 0,
                r5x4: d[14] & 0x10 != 0,
                r15x9: d[14] & 0x08 != 0,
            },
            preferred_aspect: match d[15] >> 5 {
                0 => Some(CvtAspect::R4x3),
                1 => Some(CvtAspect::R16x9),
                2 => Some(CvtAspect::R16x10),
                3 => Some(CvtAspect::R5x4),
                4 => Some(CvtAspect::R15x9),
                _ => None,
            },
            standard_blanking: d[15] & 0x08 != 0,
            reduced_blanking: d[15] & 0x10 != 0,
            h_shrink: d[16] & 0x80 != 0,
            h_stretch: d[16] & 0x40 != 0,
            v_shrink: d[16] & 0x20 != 0,
            v_stretch: d[16] & 0x10 != 0,
            preferred_refresh_hz: d[17],
        }),
        other => RangeFormula::Unknown(other),
    };
    RangeLimitsDescriptor {
        min_vertical_hz: u16::from(d[5]) + v_min_off,
        max_vertical_hz: u16::from(d[6]) + v_max_off,
        min_horizontal_khz: u16::from(d[7]) + h_min_off,
        max_horizontal_khz: u16::from(d[8]) + h_max_off,
        max_pixel_clock_khz: (d[9] != 0).then(|| u32::from(d[9]) * 10_000),
        formula,
    }
}

fn parse_cvt_codes(d: &[u8; 18]) -> CvtCodes {
    let mut out = CvtCodes {
        version: d[5],
        codes: Vec::new(),
    };
    for i in 0..4 {
        let c = &d[6 + i * 3..9 + i * 3];
        if c.iter().all(|&b| b == 0) {
            continue;
        }
        let v_active = ((u16::from(c[0]) | (u16::from(c[1] & 0xf0) << 4)) + 1) * 2;
        let aspect = cvt_aspect_bits(c[1] >> 2);
        let (rw, rh) = aspect.ratio();
        let h_active = 8 * ((u32::from(v_active) * rw / rh) / 8);
        let mut pref = usize::from((c[2] >> 5) & 3);
        if pref == 1 && c[2] & 1 != 0 {
            pref = 4;
        }
        let rates = [
            c[2] & 0x10 != 0,
            c[2] & 0x08 != 0,
            c[2] & 0x04 != 0,
            c[2] & 0x02 != 0,
            c[2] & 0x01 != 0,
        ];
        const ALL: [CvtRate; 5] = [
            CvtRate::Hz50,
            CvtRate::Hz60,
            CvtRate::Hz75,
            CvtRate::Hz85,
            CvtRate::Hz60Reduced,
        ];
        out.codes.push(CvtTimingCode {
            v_active,
            h_active: h_active as u16,
            aspect,
            rates,
            preferred: rates.get(pref).copied().filter(|&on| on).map(|_| ALL[pref]),
        });
    }
    out
}

fn parse_color_points(d: &[u8; 18]) -> Vec<ColorPoint> {
    let point = |i: usize, lo: usize, hx: usize, hy: usize, g: usize| ColorPoint {
        index: d[i],
        white: xy(d[hx], d[lo] >> 2, d[hy], d[lo]),
        gamma: (d[g] != 0xff).then(|| (f32::from(d[g]) + 100.0) / 100.0),
    };
    let mut v = vec![point(5, 6, 7, 8, 9)];
    if d[10] != 0 {
        v.push(point(10, 11, 12, 13, 14));
    }
    v
}

fn parse_color_management(d: &[u8; 18]) -> ColorManagement {
    let c = |i: usize| f32::from(i16::from_le_bytes([d[i], d[i + 1]])) / 100.0;
    ColorManagement {
        version: d[5],
        red_a3: c(6),
        red_a2: c(8),
        green_a3: c(10),
        green_a2: c(12),
        blue_a3: c(14),
        blue_a2: c(16),
    }
}

fn parse_descriptor(d: &[u8; 18], minor: u8) -> Descriptor {
    match d[3] {
        0xff => Descriptor::SerialNumber(descriptor_text(d)),
        0xfe => Descriptor::Text(descriptor_text(d)),
        0xfc => Descriptor::ProductName(descriptor_text(d)),
        0xfd => Descriptor::RangeLimits(parse_range_limits(d, minor)),
        0xfb => Descriptor::ColorPoints(parse_color_points(d)),
        0xfa => Descriptor::StandardTimings(
            (0..6)
                .filter_map(|i| parse_standard_timing(d[5 + i * 2], d[6 + i * 2], minor))
                .collect(),
        ),
        0xf9 => Descriptor::ColorManagement(parse_color_management(d)),
        0xf8 => Descriptor::CvtCodes(parse_cvt_codes(d)),
        0xf7 => {
            let mut b = [0u8; 6];
            b.copy_from_slice(&d[6..12]);
            Descriptor::EstablishedIii(EstablishedIii(b))
        }
        0x10 => Descriptor::Dummy,
        tag => Descriptor::Other { tag },
    }
}

fn parse_input(b: u8, minor: u8) -> VideoInput {
    if b & 0x80 != 0 {
        let mut input = DigitalInput::default();
        if minor >= 4 {
            input.bits_per_color = match (b >> 4) & 7 {
                0 | 7 => None,
                n => Some(n * 2 + 4),
            };
            input.interface = match b & 0x0f {
                0 => DigitalInterface::Undefined,
                1 => DigitalInterface::Dvi,
                2 => DigitalInterface::HdmiA,
                3 => DigitalInterface::HdmiB,
                4 => DigitalInterface::Mddi,
                5 => DigitalInterface::DisplayPort,
                n => DigitalInterface::Reserved(n),
            };
        } else if minor >= 2 {
            input.dfp_compatible = b & 1 != 0;
        }
        VideoInput::Digital(input)
    } else {
        VideoInput::Analog(AnalogInput {
            signal_level: (b >> 5) & 3,
            blank_to_black_setup: b & 0x10 != 0,
            separate_sync: b & 0x08 != 0,
            composite_sync: b & 0x04 != 0,
            sync_on_green: b & 0x02 != 0,
            vsync_serration: b & 0x01 != 0,
        })
    }
}

fn parse_features(b: u8, digital: bool, minor: u8) -> Features {
    let color = if digital && minor >= 4 {
        DisplayColor::Digital {
            ycbcr444: b & 0x08 != 0,
            ycbcr422: b & 0x10 != 0,
        }
    } else {
        match (b >> 3) & 3 {
            0 => DisplayColor::Monochrome,
            1 => DisplayColor::Rgb,
            2 => DisplayColor::NonRgb,
            _ => DisplayColor::Undefined,
        }
    };
    Features {
        raw: b,
        dpms_standby: b & 0x80 != 0,
        dpms_suspend: b & 0x40 != 0,
        dpms_active_off: b & 0x20 != 0,
        color,
        srgb_default: b & 0x04 != 0,
        has_preferred_timing: minor >= 4 || b & 0x02 != 0,
        preferred_includes_native: b & 0x02 != 0,
        continuous_frequency: b & 0x01 != 0,
    }
}

/// Decodes the video section of a 128-byte EDID base block. A slice shorter than 128 bytes gives
/// the all-default value.
pub fn parse_base_video(base: &[u8]) -> BaseVideo {
    let Some(x) = base.get(..128).and_then(|s| <&[u8; 128]>::try_from(s).ok()) else {
        return BaseVideo::default();
    };
    let minor = x[19];
    let input = parse_input(x[0x14], minor);
    let screen_size = if x[0x15] != 0 && x[0x16] != 0 {
        ScreenSize::Cm {
            width: x[0x15],
            height: x[0x16],
        }
    } else if minor >= 4 && x[0x15] != 0 {
        ScreenSize::AspectLandscape((f32::from(x[0x15]) + 99.0) / 100.0)
    } else if minor >= 4 && x[0x16] != 0 {
        ScreenSize::AspectPortrait(100.0 / (f32::from(x[0x16]) + 99.0))
    } else {
        ScreenSize::Variable
    };
    let mut v = BaseVideo {
        version: (x[18], x[19]),
        input,
        screen_size,
        gamma: (x[0x17] != 0xff).then(|| (f32::from(x[0x17]) + 100.0) / 100.0),
        features: parse_features(x[0x18], input.is_digital(), minor),
        chromaticity: Chromaticity {
            red: xy(x[0x1b], x[0x19] >> 6, x[0x1c], x[0x19] >> 4),
            green: xy(x[0x1d], x[0x19] >> 2, x[0x1e], x[0x19]),
            blue: xy(x[0x1f], x[0x1a] >> 6, x[0x20], x[0x1a] >> 4),
            white: xy(x[0x21], x[0x1a] >> 2, x[0x22], x[0x1a]),
        },
        established: EstablishedTimings {
            byte_23: x[0x23],
            byte_24: x[0x24],
            manufacturer: x[0x25],
        },
        ..BaseVideo::default()
    };
    for i in 0..8 {
        if let Some(t) = parse_standard_timing(x[0x26 + i * 2], x[0x27 + i * 2], minor) {
            v.standard.push(t);
        }
    }
    for off in [54usize, 72, 90, 108] {
        let Some(d) = <&[u8; 18]>::try_from(&x[off..off + 18]).ok() else {
            continue;
        };
        match parse_dtd(d) {
            Some(t) => v.detailed.push(t),
            None => v.descriptors.push(parse_descriptor(d, minor)),
        }
    }
    v
}

/// Where a resolved mode was listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSource {
    /// Established timings I or II.
    Established,
    /// A standard timing (base block or 0xFA descriptor).
    Standard,
    /// A detailed timing descriptor.
    Detailed,
    /// A CVT 3-byte timing code.
    Cvt3,
    /// Established timings III.
    EstablishedIii,
}

/// How a resolved mode's timing was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeBasis {
    /// A row of the DMT table.
    Dmt,
    /// The CVT formula.
    Cvt,
    /// The GTF formula.
    Gtf,
    /// The descriptor carries the full timing.
    Detailed,
    /// A fixed legacy timing (IBM, Apple) listed by the established timings.
    Fixed,
    /// No formula applies (EDID before 1.2): only size and refresh are known.
    Unspecified,
}

/// A mode the base block lists, with its timing resolved where possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedMode {
    /// Active width.
    pub width: u32,
    /// Active height (per frame).
    pub height: u32,
    /// Vertical refresh (field rate if interlaced) in millihertz.
    pub refresh_millihz: u32,
    /// Interlaced.
    pub interlaced: bool,
    /// Pixel clock in kHz.
    pub pixel_clock_khz: Option<u32>,
    /// Total pixels per line.
    pub h_total: Option<u32>,
    /// Total lines per frame.
    pub v_total: Option<u32>,
    /// Where the mode was listed.
    pub source: ModeSource,
    /// How the timing was obtained.
    pub basis: ModeBasis,
    /// The DMT ID when the mode is a DMT row.
    pub dmt_id: Option<u8>,
}

fn from_dmt(d: &dmt::DmtTiming, source: ModeSource) -> ResolvedMode {
    ResolvedMode {
        width: u32::from(d.width),
        height: u32::from(d.height),
        refresh_millihz: d.refresh_millihz,
        interlaced: d.interlaced,
        pixel_clock_khz: Some(d.pixel_clock_khz),
        h_total: Some(u32::from(d.h_total)),
        v_total: Some(u32::from(d.v_total)),
        source,
        basis: ModeBasis::Dmt,
        dmt_id: Some(d.id),
    }
}

fn from_calc(t: &cvt::CalcTiming, source: ModeSource, basis: ModeBasis) -> ResolvedMode {
    ResolvedMode {
        width: t.h_active,
        height: t.v_active,
        refresh_millihz: t.refresh_millihz(),
        interlaced: t.interlaced,
        pixel_clock_khz: Some(t.pixel_clock_khz),
        h_total: Some(t.h_total),
        v_total: Some(t.v_total),
        source,
        basis,
        dmt_id: None,
    }
}

/// (width, height, pixel clock kHz, h total, v total) of the non-DMT established timings.
const IBM_720X400_70: (u32, u32, u32, u32, u32) = (720, 400, 28_320, 900, 449);
const IBM_720X400_88: (u32, u32, u32, u32, u32) = (720, 400, 35_500, 900, 449);
const APPLE_640X480_67: (u32, u32, u32, u32, u32) = (640, 480, 30_240, 864, 525);
const APPLE_832X624_75: (u32, u32, u32, u32, u32) = (832, 624, 57_284, 1152, 667);
const APPLE_1152X870_75: (u32, u32, u32, u32, u32) = (1152, 870, 100_000, 1456, 915);

enum Est {
    Dmt(u8),
    Fixed((u32, u32, u32, u32, u32)),
}

/// The established timings I and II bits, byte 0x23 bit 7 first.
const EST12: [Est; 17] = [
    Est::Fixed(IBM_720X400_70),
    Est::Fixed(IBM_720X400_88),
    Est::Dmt(0x04),
    Est::Fixed(APPLE_640X480_67),
    Est::Dmt(0x05),
    Est::Dmt(0x06),
    Est::Dmt(0x08),
    Est::Dmt(0x09),
    Est::Dmt(0x0a),
    Est::Dmt(0x0b),
    Est::Fixed(APPLE_832X624_75),
    Est::Dmt(0x0f),
    Est::Dmt(0x10),
    Est::Dmt(0x11),
    Est::Dmt(0x12),
    Est::Dmt(0x24),
    Est::Fixed(APPLE_1152X870_75),
];

fn unspecified(w: u32, h: u32, hz: u32, source: ModeSource) -> ResolvedMode {
    ResolvedMode {
        width: w,
        height: h,
        refresh_millihz: hz.saturating_mul(1000),
        interlaced: false,
        pixel_clock_khz: None,
        h_total: None,
        v_total: None,
        source,
        basis: ModeBasis::Unspecified,
        dmt_id: None,
    }
}

fn resolve_standard(st: &StandardTiming, base: &BaseVideo) -> ResolvedMode {
    let src = ModeSource::Standard;
    if let Some(d) = dmt::by_std_code(st.code[0], st.code[1]) {
        return from_dmt(d, src);
    }
    let (w, h, hz) = (
        u32::from(st.h_active),
        u32::from(st.v_active),
        f64::from(st.refresh_hz),
    );
    // The EDID 1.4 text lets a CVT range-limits descriptor imply CVT for standard timings, but
    // edid-decode (the lab oracle) always lists the GTF result from EDID 1.2 on; follow it so
    // results compare equal. Callers wanting the CVT reading can call `cvt::cvt` themselves.
    if base.version >= (1, 2)
        && let Some(t) = cvt::gtf_default(w, h, hz)
    {
        return from_calc(&t, src, ModeBasis::Gtf);
    }
    unspecified(w, h, u32::from(st.refresh_hz), src)
}

/// Every mode the base block lists, resolved to a timing: established timings I/II (bit order),
/// standard timings, detailed timings, then the descriptors in slot order (established III, CVT
/// codes, 0xFA standard timings). Established timings resolve through DMT (or the fixed legacy
/// timings), standard timings through DMT and otherwise through GTF (EDID 1.2 and later, as
/// edid-decode lists them; earlier versions keep only size and refresh), CVT codes through CVT
/// (reduced blanking v1 for the "60 Hz RB" bit).
pub fn modes(base: &BaseVideo) -> Vec<ResolvedMode> {
    let mut out = Vec::new();
    let e = &base.established;
    for (i, est) in EST12.iter().enumerate() {
        let on = match i {
            0..=7 => e.byte_23 & (0x80 >> i) != 0,
            8..=15 => e.byte_24 & (0x80 >> (i - 8)) != 0,
            _ => e.manufacturer & 0x80 != 0,
        };
        if !on {
            continue;
        }
        match est {
            Est::Dmt(id) => {
                if let Some(d) = dmt::by_id(*id) {
                    out.push(from_dmt(d, ModeSource::Established));
                }
            }
            Est::Fixed((w, h, clk, ht, vt)) => {
                let den = u64::from(*ht) * u64::from(*vt);
                out.push(ResolvedMode {
                    width: *w,
                    height: *h,
                    refresh_millihz: ((u64::from(*clk) * 1_000_000 + den / 2) / den) as u32,
                    interlaced: false,
                    pixel_clock_khz: Some(*clk),
                    h_total: Some(*ht),
                    v_total: Some(*vt),
                    source: ModeSource::Established,
                    basis: ModeBasis::Fixed,
                    dmt_id: None,
                });
            }
        }
    }
    for st in &base.standard {
        out.push(resolve_standard(st, base));
    }
    for t in &base.detailed {
        out.push(ResolvedMode {
            width: u32::from(t.h_active),
            height: t.height(),
            refresh_millihz: t.refresh_millihz(),
            interlaced: t.interlaced,
            pixel_clock_khz: Some(t.pixel_clock_khz),
            h_total: Some(t.h_total()),
            v_total: Some(t.v_total_frame()),
            source: ModeSource::Detailed,
            basis: ModeBasis::Detailed,
            dmt_id: None,
        });
    }
    for d in &base.descriptors {
        match d {
            Descriptor::EstablishedIii(e3) => {
                for id in e3.dmt_ids() {
                    if let Some(t) = dmt::by_id(id) {
                        out.push(from_dmt(t, ModeSource::EstablishedIii));
                    }
                }
            }
            Descriptor::CvtCodes(c) => {
                for code in &c.codes {
                    for rate in code.supported() {
                        let blanking = if rate.reduced() {
                            cvt::Blanking::ReducedV1
                        } else {
                            cvt::Blanking::Standard
                        };
                        if let Some(t) = cvt::cvt(
                            u32::from(code.h_active),
                            u32::from(code.v_active),
                            f64::from(rate.hz()),
                            blanking,
                            false,
                            false,
                        ) {
                            out.push(from_calc(&t, ModeSource::Cvt3, ModeBasis::Cvt));
                        }
                    }
                }
            }
            Descriptor::StandardTimings(list) => {
                for st in list {
                    out.push(resolve_standard(st, base));
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DTD_1080P60: [u8; 18] = [
        0x02, 0x3A, 0x80, 0x18, 0x71, 0x38, 0x2D, 0x40, 0x58, 0x2C, 0x45, 0x00, 0x40, 0x84, 0x63,
        0x00, 0x00, 0x1E,
    ];

    /// A base block of the given EDID minor version carrying the four descriptor slots; fields
    /// that tests need are set to the sRGB-like values, everything else is zero (checksum is not
    /// checked by the parser).
    fn block(minor: u8, input: u8, features: u8, slots: [[u8; 18]; 4]) -> [u8; 128] {
        let mut b = [0u8; 128];
        b[..8].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 0]);
        b[18] = 1;
        b[19] = minor;
        b[0x14] = input;
        b[0x15] = 0x3C;
        b[0x16] = 0x21;
        b[0x17] = 0x78;
        b[0x18] = features;
        b[0x19..0x23]
            .copy_from_slice(&[0xEE, 0x91, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54]);
        for i in 0..8 {
            b[0x26 + 2 * i] = 1;
            b[0x27 + 2 * i] = 1;
        }
        for (i, s) in slots.iter().enumerate() {
            b[54 + 18 * i..72 + 18 * i].copy_from_slice(s);
        }
        b
    }

    fn desc(tag: u8, payload: &[u8]) -> [u8; 18] {
        let mut d = [0u8; 18];
        d[3] = tag;
        d[4..4 + payload.len()].copy_from_slice(payload);
        d
    }

    fn text_desc(tag: u8, text: &str) -> [u8; 18] {
        let mut d = [0x20u8; 18];
        d[..3].fill(0);
        d[3] = tag;
        d[4] = 0;
        let t = text.as_bytes();
        d[5..5 + t.len()].copy_from_slice(t);
        if t.len() < 13 {
            d[5 + t.len()] = 0x0a;
        }
        d
    }

    const EMPTY: [u8; 18] = [0; 18];

    #[test]
    fn cea_1080p60_dtd() {
        // 1920x1080@60, 148.5 MHz (CTA-861 VIC 16 timing).
        let d = DTD_1080P60;
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

    #[test]
    fn real_edid_matches_edid_decode() {
        let b = include_bytes!("../../../tests/fixtures/parity/edid/samsung-lc34g55t.bin");
        let v = parse_base_video(&b[..128]);
        assert_eq!(v.version, (1, 4));
        assert_eq!(
            v.input,
            VideoInput::Digital(DigitalInput {
                bits_per_color: Some(10),
                interface: DigitalInterface::DisplayPort,
                dfp_compatible: false,
            })
        );
        assert_eq!(
            v.screen_size,
            ScreenSize::Cm {
                width: 80,
                height: 34
            }
        );
        assert_eq!(v.gamma, Some(2.2));
        assert_eq!(
            v.features.color,
            DisplayColor::Digital {
                ycbcr444: true,
                ycbcr422: true
            }
        );
        assert!(v.features.dpms_active_off && !v.features.dpms_standby);
        assert!(v.features.preferred_includes_native && v.features.continuous_frequency);
        assert!(!v.features.srgb_default);
        // edid-decode: Red 0.6484, 0.3359; White 0.3076, 0.3291.
        assert!((v.chromaticity.red.x - 0.6484).abs() < 1e-4);
        assert!((v.chromaticity.red.y - 0.3359).abs() < 1e-4);
        assert!((v.chromaticity.white.x - 0.3076).abs() < 1e-4);
        assert_eq!(v.product_name(), Some("LC34G55T"));
        assert_eq!(v.serial_text(), Some("REDACTED"));
        // Range limits only, 60-165 Hz, horizontal 258-258 kHz through the +255 offsets.
        let r = v.range_limits().unwrap();
        assert_eq!(
            (
                r.min_vertical_hz,
                r.max_vertical_hz,
                r.min_horizontal_khz,
                r.max_horizontal_khz
            ),
            (60, 165, 258, 258)
        );
        assert_eq!(r.max_pixel_clock_khz, Some(880_000));
        assert_eq!(r.formula, RangeFormula::RangeLimitsOnly);
        assert_eq!(r.formula.name(), Some("range_only"));
        // DTD 1: 3440x1440 99.981604 Hz 543.5 MHz.
        let p = v.preferred().unwrap();
        assert_eq!(
            (p.h_active, p.height(), p.pixel_clock_khz),
            (3440, 1440, 543_500)
        );
        assert_eq!(p.refresh_millihz(), 99_982);
        let m = modes(&v);
        assert_eq!(m.len(), 23);
        // Standard timing 0x81 0x00: 1280x800@60 through DMT 0x1c (59.810 Hz).
        let m800 = m.iter().find(|m| m.dmt_id == Some(0x1c)).unwrap();
        assert_eq!(
            (
                m800.width,
                m800.height,
                m800.refresh_millihz,
                m800.pixel_clock_khz
            ),
            (1280, 800, 59_810, Some(83_500))
        );
        assert_eq!(m800.source, ModeSource::Standard);
        let last = m.last().unwrap();
        assert_eq!(last.source, ModeSource::Detailed);
        assert_eq!((last.h_total, last.v_total), (Some(3600), Some(1510)));
        // The IBM and Apple established timings.
        let fixed: Vec<_> = m.iter().filter(|m| m.basis == ModeBasis::Fixed).collect();
        assert_eq!(fixed.len(), 4);
        assert_eq!(fixed[0].refresh_millihz, 70_082);
    }

    #[test]
    fn synthetic_edid_14_cvt_codes_established_iii_and_std_descriptor() {
        // Cross-checked against edid-decode: every mode below appears in its listing.
        let slots = [
            DTD_1080P60,
            desc(
                0xF8,
                &[
                    0, 1, 0x1B, 0x24, 0x29, 0x77, 0x58, 0x1A, 0x00, 0x00, 0x00, 0xE0, 0x80, 0x08,
                ],
            ),
            desc(
                0xF7,
                &[0x00, 0xAA, 0x55, 0xFF, 0x0F, 0xF0, 0x0F, 0, 0, 0, 0, 0, 0],
            ),
            desc(
                0xFA,
                &[
                    0x01, 0x01, 0xD1, 0xC0, 0x61, 0x40, 0x31, 0x59, 0x4C, 0xA3, 0x4C, 0xC1,
                ],
            ),
        ];
        let mut b = block(4, 0xA5, 0xEE, slots);
        b[0x23] = 0x21;
        b[0x24] = 0x08;
        b[0x25] = 0x80;
        for (i, (a, c)) in [(0xD1u8, 0xC0u8), (0x45, 0x61), (0x95, 0x00), (0x81, 0x4F)]
            .iter()
            .enumerate()
        {
            b[0x26 + 2 * i] = *a;
            b[0x27 + 2 * i] = *c;
        }
        let v = parse_base_video(&b);
        assert_eq!(v.standard.len(), 4);
        assert_eq!(v.detailed.len(), 1);
        assert_eq!(v.descriptors.len(), 3);
        let m = modes(&v);
        let find = |src: ModeSource, w: u32, h: u32, hz_milli: u32| {
            m.iter()
                .find(|m| {
                    m.source == src && (m.width, m.height, m.refresh_millihz) == (w, h, hz_milli)
                })
                .copied()
        };
        // Established I/II: 640x480@60, 800x600@60, 1024x768@60 and 1152x870@75.
        assert_eq!(
            m.iter()
                .filter(|m| m.source == ModeSource::Established)
                .count(),
            4
        );
        // Standard timings: DMT, then GTF for the rest (edid-decode lists GTF from EDID 1.2).
        let gtf = find(ModeSource::Standard, 800, 600, 93_001).unwrap();
        assert_eq!(gtf.basis, ModeBasis::Gtf);
        assert_eq!(
            (gtf.pixel_clock_khz, gtf.h_total, gtf.v_total),
            (Some(63_108), Some(1072), Some(633))
        );
        let gtf = find(ModeSource::Standard, 1280, 960, 75_000).unwrap();
        assert_eq!(gtf.pixel_clock_khz, Some(129_859));
        assert_eq!(
            find(ModeSource::Standard, 1920, 1080, 60_000)
                .unwrap()
                .dmt_id,
            Some(0x52)
        );
        // CVT codes: 1080p at 60 Hz standard and 60 Hz reduced blanking, 4480x2800 at 50/60/85.
        let c = find(ModeSource::Cvt3, 1920, 1080, 59_963).unwrap();
        assert_eq!(c.pixel_clock_khz, Some(173_000));
        let c = find(ModeSource::Cvt3, 1920, 1080, 59_934).unwrap();
        assert_eq!(c.pixel_clock_khz, Some(138_500));
        assert_eq!(
            find(ModeSource::Cvt3, 4480, 2800, 49_979)
                .unwrap()
                .pixel_clock_khz,
            Some(894_500)
        );
        assert_eq!(
            find(ModeSource::Cvt3, 4480, 2800, 84_978)
                .unwrap()
                .pixel_clock_khz,
            Some(1_571_500)
        );
        assert_eq!(
            find(ModeSource::Cvt3, 6056, 4546, 59_989)
                .unwrap()
                .pixel_clock_khz,
            Some(2_404_750)
        );
        assert_eq!(m.iter().filter(|m| m.source == ModeSource::Cvt3).count(), 6);
        // Established III: 24 DMT timings.
        assert_eq!(
            m.iter()
                .filter(|m| m.source == ModeSource::EstablishedIii)
                .count(),
            24
        );
        assert_eq!(
            find(ModeSource::EstablishedIii, 1280, 768, 59_995)
                .unwrap()
                .dmt_id,
            Some(0x16)
        );
        // The 0xFA descriptor's five timings.
        assert_eq!(
            m.iter()
                .filter(|m| m.source == ModeSource::Standard)
                .count(),
            4 + 5
        );
        let d = find(ModeSource::Standard, 760, 475, 109_000).unwrap();
        assert_eq!(d.pixel_clock_khz, Some(55_154));
        // CVT code details.
        let Descriptor::CvtCodes(codes) = &v.descriptors[0] else {
            panic!()
        };
        assert_eq!(codes.version, 1);
        assert_eq!(codes.codes[0].aspect, CvtAspect::R16x9);
        assert_eq!(codes.codes[0].preferred, Some(CvtRate::Hz60Reduced));
        assert_eq!(
            codes.codes[1].supported(),
            vec![CvtRate::Hz50, CvtRate::Hz60, CvtRate::Hz85]
        );
        assert_eq!(codes.codes[1].preferred, Some(CvtRate::Hz50));
    }

    #[test]
    fn synthetic_edid_13_secondary_gtf_color_points_and_management() {
        let slots = [
            DTD_1080P60,
            desc(
                0xFD,
                &[
                    0, 0x30, 0x4B, 0x1E, 0x50, 0x1A, 0x02, 0x00, 0x3C, 0x50, 0x0B, 0x02, 0x80, 0x28,
                ],
            ),
            desc(
                0xFB,
                &[1, 0x0A, 0x50, 0x54, 0x78, 2, 0x05, 0x4F, 0x5A, 0xFF],
            ),
            desc(
                0xF9,
                &[
                    1, 0x10, 0x00, 0xF0, 0xFF, 0x20, 0x01, 0x00, 0x80, 0x05, 0x00, 0x07, 0x00,
                ],
            ),
        ];
        let v = parse_base_video(&block(3, 0x80, 0xEE, slots));
        // EDID 1.3: digital with no depth or interface, colour type RGB, preferred timing bit.
        assert_eq!(v.input, VideoInput::Digital(DigitalInput::default()));
        assert_eq!(v.features.color, DisplayColor::Rgb);
        assert!(v.features.srgb_default && v.features.has_preferred_timing);
        assert!(!v.features.continuous_frequency);
        // Secondary GTF, no EDID 1.4 offsets.
        let r = v.range_limits().unwrap();
        assert_eq!(
            (
                r.min_vertical_hz,
                r.max_vertical_hz,
                r.min_horizontal_khz,
                r.max_horizontal_khz
            ),
            (48, 75, 30, 80)
        );
        assert_eq!(r.max_pixel_clock_khz, Some(260_000));
        let RangeFormula::SecondaryGtf(Some(g)) = r.formula else {
            panic!()
        };
        assert_eq!(
            (g.start_khz, g.c, g.m, g.k, g.j),
            (120, 40.0, 523, 128, 20.0)
        );
        let p = g.params();
        assert_eq!((p.c, p.m, p.k, p.j), (40.0, 523.0, 128.0, 20.0));
        let Descriptor::ColorPoints(cp) = &v.descriptors[1] else {
            panic!()
        };
        assert_eq!(cp.len(), 2);
        assert_eq!(cp[0].index, 10);
        assert!((cp[0].white.x - 0.3281).abs() < 1e-4 && (cp[0].white.y - 0.4687).abs() < 1e-4);
        assert!((cp[0].gamma.unwrap() - 1.02).abs() < 1e-4);
        assert_eq!(cp[1].index, 5);
        // Display colour management data as edid-decode prints it.
        let Descriptor::ColorManagement(cm) = &v.descriptors[2] else {
            panic!()
        };
        assert_eq!(cm.version, 16);
        assert!((cm.red_a3 + 40.96).abs() < 1e-3 && (cm.red_a2 - 84.47).abs() < 1e-3);
        assert!((cm.green_a3 - 0.01).abs() < 1e-3 && (cm.green_a2 - 14.08).abs() < 1e-3);
        assert!((cm.blue_a3 - 17.92).abs() < 1e-3 && cm.blue_a2.abs() < 1e-3);
    }

    #[test]
    fn range_limit_offsets_and_cvt_fields() {
        // EDID 1.4: max V +255 and max H +255 (flags 0x0A); min V +255 and min H +255 too (0x0F).
        let d = |flags: u8| {
            desc(
                0xFD,
                &[
                    flags, 48, 100, 30, 200, 30, 0x04, 0x11, 0x11, 0x40, 0xE8, 0x38, 0xC0, 60,
                ],
            )
        };
        let r = |minor, flags| {
            *parse_base_video(&block(
                minor,
                0xA5,
                0xEE,
                [DTD_1080P60, d(flags), EMPTY, EMPTY],
            ))
            .range_limits()
            .unwrap()
        };
        let a = r(4, 0x0A);
        assert_eq!(
            (
                a.min_vertical_hz,
                a.max_vertical_hz,
                a.min_horizontal_khz,
                a.max_horizontal_khz
            ),
            (48, 355, 30, 455)
        );
        let a = r(4, 0x0F);
        assert_eq!(
            (
                a.min_vertical_hz,
                a.max_vertical_hz,
                a.min_horizontal_khz,
                a.max_horizontal_khz
            ),
            (303, 355, 285, 455)
        );
        // Min offset flags without the max flag do nothing; offsets are ignored before 1.4.
        let a = r(4, 0x05);
        assert_eq!((a.max_vertical_hz, a.max_horizontal_khz), (100, 200));
        let a = r(3, 0x0F);
        assert_eq!((a.max_vertical_hz, a.max_horizontal_khz), (100, 200));
        // The CVT fields.
        let c = r(4, 0).cvt().copied().unwrap();
        assert_eq!(c.version, (1, 1));
        assert_eq!(c.clock_offset_quarter_mhz, 4);
        assert_eq!(c.max_h_active, 2560);
        assert_eq!(
            c.aspects,
            CvtAspects {
                r4x3: true,
                r16x9: true,
                r16x10: true,
                r5x4: false,
                r15x9: true
            }
        );
        assert_eq!(c.preferred_aspect, Some(CvtAspect::R16x9));
        assert!(c.standard_blanking && c.reduced_blanking);
        assert!(c.h_shrink && c.h_stretch && !c.v_shrink && !c.v_stretch);
        assert_eq!(c.preferred_refresh_hz, 60);
        assert_eq!(r(4, 0).formula.name(), Some("cvt"));
        assert_eq!(r(4, 0).max_pixel_clock_khz, Some(300_000));
    }

    #[test]
    fn text_and_other_descriptors() {
        let v = parse_base_video(&block(
            3,
            0x80,
            0x0E,
            [
                text_desc(0xFF, "SN-0001"),
                text_desc(0xFE, "free text"),
                text_desc(0xFC, "Test Panel"),
                desc(0x10, &[]),
            ],
        ));
        assert_eq!(v.detailed.len(), 0);
        assert_eq!(
            v.descriptors,
            vec![
                Descriptor::SerialNumber("SN-0001".into()),
                Descriptor::Text("free text".into()),
                Descriptor::ProductName("Test Panel".into()),
                Descriptor::Dummy,
            ]
        );
        let v = parse_base_video(&block(
            3,
            0x80,
            0x0E,
            [desc(0x0E, &[]), desc(0x05, &[]), desc(0x77, &[]), EMPTY],
        ));
        // An all-zero slot is a descriptor with tag 0.
        assert_eq!(
            v.descriptors,
            vec![
                Descriptor::Other { tag: 0x0E },
                Descriptor::Other { tag: 0x05 },
                Descriptor::Other { tag: 0x77 },
                Descriptor::Other { tag: 0 },
            ]
        );
        // A full-width name with no terminator, and non-ASCII bytes.
        let mut d = text_desc(0xFC, "ABCDEFGHIJKLM");
        d[6] = 0x81;
        let v = parse_base_video(&block(3, 0x80, 0x0E, [d, EMPTY, EMPTY, EMPTY]));
        assert_eq!(v.product_name(), Some("A.CDEFGHIJKLM"));
    }

    #[test]
    fn input_size_gamma_and_features_decoding() {
        let slots = [DTD_1080P60, EMPTY, EMPTY, EMPTY];
        // Analog: 0.714 V level, pedestal, separate + composite sync.
        let mut b = block(3, 0x20 | 0x10 | 0x08 | 0x04 | 0x01, 0x0A, slots);
        let v = parse_base_video(&b);
        assert_eq!(
            v.input,
            VideoInput::Analog(AnalogInput {
                signal_level: 1,
                blank_to_black_setup: true,
                separate_sync: true,
                composite_sync: true,
                sync_on_green: false,
                vsync_serration: true,
            })
        );
        assert!(!v.input.is_digital());
        assert_eq!(v.features.color, DisplayColor::Rgb);
        // Digital 1.4: bit depth codes and interfaces, no depth for 000 and 111.
        for (code, bits) in [
            (1u8, Some(6u8)),
            (2, Some(8)),
            (3, Some(10)),
            (4, Some(12)),
            (5, Some(14)),
            (6, Some(16)),
            (0, None),
            (7, None),
        ] {
            let v = parse_base_video(&block(4, 0x80 | (code << 4) | 2, 0x18, slots));
            let VideoInput::Digital(d) = v.input else {
                panic!()
            };
            assert_eq!(
                (d.bits_per_color, d.interface),
                (bits, DigitalInterface::HdmiA)
            );
            assert_eq!(
                v.features.color,
                DisplayColor::Digital {
                    ycbcr444: true,
                    ycbcr422: true
                }
            );
        }
        let ifaces = [
            (0u8, DigitalInterface::Undefined),
            (1, DigitalInterface::Dvi),
            (2, DigitalInterface::HdmiA),
            (3, DigitalInterface::HdmiB),
            (4, DigitalInterface::Mddi),
            (5, DigitalInterface::DisplayPort),
            (9, DigitalInterface::Reserved(9)),
        ];
        for (c, i) in ifaces {
            let VideoInput::Digital(d) = parse_base_video(&block(4, 0x90 | c, 0, slots)).input
            else {
                panic!()
            };
            assert_eq!(d.interface, i);
        }
        // EDID 1.2/1.3 digital: DFP 1.x bit only.
        let VideoInput::Digital(d) = parse_base_video(&block(2, 0x81, 0, slots)).input else {
            panic!()
        };
        assert!(d.dfp_compatible && d.interface == DigitalInterface::Undefined);
        // Screen size: cm, landscape ratio, portrait ratio, variable; gamma 0xFF.
        b[0x15] = 0;
        b[0x16] = 0;
        assert_eq!(parse_base_video(&b).screen_size, ScreenSize::Variable);
        b[19] = 4;
        b[0x15] = 79; // (79 + 99) / 100 = 1.78
        let ScreenSize::AspectLandscape(r) = parse_base_video(&b).screen_size else {
            panic!()
        };
        assert!((r - 1.78).abs() < 1e-4);
        b[0x15] = 0;
        b[0x16] = 1; // 100 / 100 = 1.0
        let ScreenSize::AspectPortrait(r) = parse_base_video(&b).screen_size else {
            panic!()
        };
        assert!((r - 1.0).abs() < 1e-4);
        b[19] = 3;
        assert_eq!(parse_base_video(&b).screen_size, ScreenSize::Variable);
        b[0x17] = 0xFF;
        assert_eq!(parse_base_video(&b).gamma, None);
        // Features: preferred timing is always set from 1.4 and only by the bit before.
        let f = parse_base_video(&block(4, 0xA5, 0x00, slots)).features;
        assert!(f.has_preferred_timing && !f.preferred_includes_native);
        let f = parse_base_video(&block(3, 0x80, 0x00, slots)).features;
        assert!(!f.has_preferred_timing);
        let f = parse_base_video(&block(3, 0x80, 0xE7, slots)).features;
        assert!(f.dpms_standby && f.dpms_suspend && f.dpms_active_off && f.continuous_frequency);
        assert_eq!(f.color, DisplayColor::Monochrome);
    }

    #[test]
    fn chromaticity_is_ten_bit_fixed_point() {
        let v = parse_base_video(&block(4, 0xA5, 0xEE, [DTD_1080P60, EMPTY, EMPTY, EMPTY]));
        let c = v.chromaticity;
        // The sRGB primaries: 0.640/0.330, 0.300/0.600, 0.150/0.060, white 0.3127/0.3290.
        for (got, want) in [
            (c.red.x, 0.640),
            (c.red.y, 0.330),
            (c.green.x, 0.300),
            (c.green.y, 0.600),
            (c.blue.x, 0.150),
            (c.blue.y, 0.060),
            (c.white.x, 0.3127),
            (c.white.y, 0.3290),
        ] {
            assert!((got - want).abs() < 0.002, "{got} vs {want}");
        }
    }

    #[test]
    fn standard_timing_decoding_and_aspect_by_version() {
        // 0x81 0x00: 1280 wide, aspect code 0 is 16:10 from EDID 1.3 and 1:1 before.
        let st = |minor| parse_standard_timing(0x81, 0x00, minor).unwrap();
        assert_eq!(
            (st(3).h_active, st(3).v_active, st(3).refresh_hz),
            (1280, 800, 60)
        );
        assert_eq!(st(3).aspect, StdAspect::Ratio16x10);
        assert_eq!((st(2).h_active, st(2).v_active), (1280, 1280));
        assert_eq!(st(2).aspect.ratio(), (1, 1));
        let t = parse_standard_timing(0xD1, 0xFF, 4).unwrap();
        assert_eq!(
            (t.h_active, t.v_active, t.refresh_hz, t.aspect),
            (1920, 1080, 123, StdAspect::Ratio16x9)
        );
        assert!(parse_standard_timing(0x01, 0x01, 4).is_none());
        assert!(parse_standard_timing(0x00, 0x00, 4).is_none());
        // Before EDID 1.2 there is no formula: only size and refresh.
        let mut b = block(1, 0x80, 0, [DTD_1080P60, EMPTY, EMPTY, EMPTY]);
        b[0x26] = 0x45;
        b[0x27] = 0x61;
        let m = modes(&parse_base_video(&b));
        assert_eq!(m[0].basis, ModeBasis::Unspecified);
        assert_eq!(
            (
                m[0].width,
                m[0].height,
                m[0].refresh_millihz,
                m[0].pixel_clock_khz
            ),
            (800, 600, 93_000, None)
        );
    }

    #[test]
    fn established_iii_bits() {
        let e = EstablishedIii([0x80, 0x01, 0, 0, 0, 0x10]);
        assert_eq!(e.dmt_ids(), vec![0x01, 0x25, 0x4a]);
        assert!(EstablishedIii([0; 6]).dmt_ids().is_empty());
        assert_eq!(EstablishedIii([0xFF; 6]).dmt_ids().len(), 44);
    }

    #[test]
    fn short_and_random_input_never_panics() {
        for n in 0..=130usize {
            let b = vec![0x5Au8; n];
            let v = parse_base_video(&b);
            let _ = modes(&v);
        }
        assert_eq!(parse_base_video(&[0u8; 127]), BaseVideo::default());
        let mut x: u32 = 0x1234_5678;
        for _ in 0..4000 {
            let mut b = [0u8; 128];
            for v in b.iter_mut() {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *v = (x >> 24) as u8;
            }
            // Bias the descriptor tags so every kind is exercised.
            if x & 1 == 0 {
                for s in [54usize, 72, 90, 108] {
                    b[s] = 0;
                    b[s + 1] = 0;
                    b[s + 3] = [0xF7u8, 0xF8, 0xFA, 0xFB, 0xFC, 0xFD, 0xFE, 0xFF, 0x10, 0xF9]
                        [(x >> 8) as usize % 10];
                }
            }
            b[19] = (x >> 16) as u8 % 6;
            let v = parse_base_video(&b);
            let _ = modes(&v);
        }
    }
}
