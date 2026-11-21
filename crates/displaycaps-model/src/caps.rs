// SPDX-License-Identifier: LGPL-2.1-or-later
//! Capability sets: what one layer (or the whole path, in an effective set) lets through.
//!
//! Every field is optional: `None` means the field group is outside the probe scope, and a
//! [`Value::Unknown`] means it was in scope but couldn't be read. Refresh rates are carried in
//! millihertz so that 23.976 Hz (`23976`) and 59.94 Hz (`59940`) stay exact; luminance in cd/m²
//! (nits).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::value::Value;

/// What one layer reports, grouped by [`crate::scope::FieldGroup`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CapabilitySet {
    /// Modes (field group `modes`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<Value<Vec<Mode>>>,
    /// Variable refresh range (field group `vrr`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vrr: Option<Value<VrrRange>>,
    /// Colour formats with their bit depths (field group `color_formats`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_formats: Option<Value<Vec<ColorFormatSupport>>>,
    /// HDR capabilities (field group `hdr`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr: Option<Value<HdrCapabilities>>,
    /// Colorimetry (field group `colorimetry`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorimetry: Option<Value<Vec<Colorimetry>>>,
    /// Quantization range (field group `quantization`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantization: Option<Value<QuantizationSupport>>,
    /// Link limits (field group `link`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Value<LinkLimits>>,
    /// Hardware decode profiles (field group `decode`, opt-in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decode: Option<Value<Vec<DecodeProfile>>>,
}

/// A refresh rate in millihertz (23.976 Hz is `23976`).
pub type MilliHertz = u32;

/// One video mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Mode {
    /// Active width in pixels.
    pub width: u16,
    /// Active height in lines (per frame, so 1080 for 1080i).
    pub height: u16,
    /// Vertical refresh in millihertz (field rate for interlaced modes).
    pub refresh_millihz: MilliHertz,
    /// Interlaced.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interlaced: bool,
    /// Full timing, when the source gives one (detailed timings, VIC tables, KMS modes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<Timing>,
    /// CTA-861 Video Identification Code, when the mode is a VIC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vic: Option<u8>,
    /// HDMI VIC (the 4K modes of the HDMI 1.4 VSDB), when the mode is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdmi_vic: Option<u8>,
    /// The sink's preferred (native) mode.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preferred: bool,
    /// Whether the mode can also, or only, be sent as YCbCr 4:2:0.
    #[serde(default, skip_serializing_if = "Ycbcr420::is_no")]
    pub ycbcr420: Ycbcr420,
    /// Where the mode was listed (EDID block types, KMS, compositor).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<ModeSource>,
}

/// A full video timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Timing {
    /// Pixel clock in kHz.
    pub pixel_clock_khz: u32,
    /// Total pixels per line, blanking included.
    pub h_total: u16,
    /// Total lines per frame, blanking included.
    pub v_total: u16,
    /// Horizontal sync polarity is positive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_sync_positive: Option<bool>,
    /// Vertical sync polarity is positive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v_sync_positive: Option<bool>,
}

/// YCbCr 4:2:0 support of a mode (CTA-861 4:2:0 video and capability map data blocks).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Ycbcr420 {
    /// Not as 4:2:0.
    #[default]
    No,
    /// As 4:2:0 in addition to the other formats.
    Also,
    /// Only as 4:2:0 (YCbCr 4:2:0 video data block).
    Only,
}

impl Ycbcr420 {
    /// Whether 4:2:0 is not supported (used to omit the default from JSON).
    pub fn is_no(&self) -> bool {
        matches!(self, Self::No)
    }
}

/// Where a mode was listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModeSource {
    /// EDID established timings.
    Established,
    /// EDID standard timings.
    Standard,
    /// EDID or CTA-861 detailed timing descriptor.
    Detailed,
    /// CTA-861 video data block (VIC).
    CtaVic,
    /// HDMI VSDB HDMI VIC.
    HdmiVic,
    /// DisplayID timing block.
    DisplayId,
    /// EDID CVT 3-byte codes.
    Cvt,
    /// The kernel's mode list for the connector.
    Kms,
    /// The compositor's mode list for the output.
    Compositor,
}

/// Variable refresh range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VrrRange {
    /// Lowest refresh in millihertz.
    pub min_millihz: MilliHertz,
    /// Highest refresh in millihertz.
    pub max_millihz: MilliHertz,
    /// Where the range comes from.
    pub kind: VrrKind,
}

impl VrrRange {
    /// Whether low-framerate compensation can work (max at least twice min).
    pub fn supports_lfc(&self) -> bool {
        u64::from(self.max_millihz) >= 2 * u64::from(self.min_millihz)
    }
}

/// Kinds of variable refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VrrKind {
    /// HDMI 2.1 VRR (HDMI Forum VSDB VRRmin/VRRmax).
    HdmiVrr,
    /// FreeSync / Adaptive-Sync from the EDID range limits or the AMD VSDB.
    AdaptiveSync,
    /// As reported by the kernel or compositor, origin not stated.
    Reported,
}

/// Colour encodings on the wire.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ColorFormat {
    /// RGB 4:4:4.
    Rgb,
    /// YCbCr 4:4:4.
    Ycbcr444,
    /// YCbCr 4:2:2.
    Ycbcr422,
    /// YCbCr 4:2:0.
    Ycbcr420,
}

/// One colour format and the bit depths per component it can carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ColorFormatSupport {
    /// The format.
    pub format: ColorFormat,
    /// Bits per component, ascending (e.g. `[8, 10, 12]`).
    pub bit_depths: Vec<u8>,
}

/// Electro-optical transfer functions a sink accepts (CTA-861 HDR static metadata block).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Eotf {
    /// Traditional gamma, SDR luminance range.
    TraditionalSdr,
    /// Traditional gamma, HDR luminance range.
    TraditionalHdr,
    /// SMPTE ST 2084 (PQ), used by HDR10, HDR10+ and Dolby Vision.
    Pq,
    /// Hybrid Log-Gamma (BT.2100 HLG).
    Hlg,
}

/// HDR formats an application can target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HdrFormat {
    /// PQ with static metadata (ST 2086 + MaxCLL/MaxFALL).
    Hdr10,
    /// Hybrid Log-Gamma.
    Hlg,
    /// HDR10+ dynamic metadata.
    Hdr10Plus,
    /// Dolby Vision.
    DolbyVision,
}

/// What a layer says about HDR.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HdrCapabilities {
    /// Transfer functions accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub eotfs: Vec<Eotf>,
    /// HDR formats this layer passes or the sink accepts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<HdrFormat>,
    /// Static metadata descriptor types supported (type 1 is ST 2086).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub static_metadata_types: Vec<u8>,
    /// Desired content max luminance (peak), in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_luminance_nits: Option<f32>,
    /// Desired content max frame-average luminance, in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_frame_average_nits: Option<f32>,
    /// Desired content min luminance, in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_luminance_nits: Option<f32>,
    /// Dynamic HDR detail (HDR10+ and Dolby Vision blocks).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dynamic: Vec<DynamicHdr>,
}

/// Dynamic-metadata HDR support as the sink describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "format", rename_all = "snake_case")]
pub enum DynamicHdr {
    /// HDR10+ (HDR10+ vendor-specific video data block).
    Hdr10Plus {
        /// Application version from the block.
        application_version: u8,
    },
    /// Dolby Vision (Dolby vendor-specific video data block).
    DolbyVision {
        /// Block version (0, 1 or 2).
        version: u8,
        /// Supports 12-bit 4:2:2 YUV (the standard DV interface).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        yuv422_12bit: Option<bool>,
        /// Low-latency (sink-led) Dolby Vision is supported.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        low_latency: Option<bool>,
        /// Target max luminance from the block, in nits.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_luminance_nits: Option<f32>,
    },
    /// Another dynamic metadata type from the HDR dynamic metadata data block.
    Other {
        /// Metadata type number.
        metadata_type: u16,
    },
}

/// Colorimetry, as in the CTA-861 colorimetry data block (and their KMS `Colorspace` peers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Colorimetry {
    /// BT.709 / sRGB (the default; never listed in the block).
    Default,
    /// xvYCC601.
    Xvycc601,
    /// xvYCC709.
    Xvycc709,
    /// sYCC601.
    Sycc601,
    /// opYCC601 (AdobeYCC601).
    OpYcc601,
    /// opRGB (AdobeRGB).
    OpRgb,
    /// BT.2020 constant luminance YCbCr.
    Bt2020Cycc,
    /// BT.2020 YCbCr.
    Bt2020Ycc,
    /// BT.2020 RGB.
    Bt2020Rgb,
    /// DCI-P3 (SMPTE RP 431-2).
    DciP3,
    /// ICtCp.
    Ictcp,
    /// ST 2113 RGB.
    St2113Rgb,
}

/// Quantization range of a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuantRange {
    /// Limited (16–235 for 8-bit).
    Limited,
    /// Full (0–255 for 8-bit).
    Full,
}

/// Quantization range support (CTA-861 video capability data block).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QuantizationSupport {
    /// The sink honours the RGB quantization range the source signals (QS bit).
    pub rgb_selectable: bool,
    /// The sink honours the YCC quantization range the source signals (QY bit).
    pub ycc_selectable: bool,
}

/// What a link (or one end of it) can carry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LinkLimits {
    /// Highest TMDS character rate in kHz (HDMI; 600000 for HDMI 2.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tmds_character_rate_khz: Option<u32>,
    /// Highest HDMI 2.1 FRL rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_frl: Option<FrlRate>,
    /// Highest DisplayPort link rate and lane count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_dp: Option<DpLink>,
    /// Display Stream Compression support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsc: Option<DscSupport>,
    /// Auto Low Latency Mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allm: Option<bool>,
    /// Highest bits per component the layer allows (e.g. KMS `max bpc` range end).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bpc: Option<u8>,
}

/// HDMI 2.1 Fixed Rate Link rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrlRate {
    /// Lanes (3 or 4).
    pub lanes: u8,
    /// Gbit/s per lane (3, 6, 8, 10 or 12).
    pub gbps_per_lane: u8,
}

impl FrlRate {
    /// Total raw rate in Gbit/s.
    pub fn total_gbps(&self) -> u32 {
        u32::from(self.lanes) * u32::from(self.gbps_per_lane)
    }
}

/// DisplayPort link rates.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DpRate {
    /// RBR, 1.62 Gbit/s per lane.
    Rbr,
    /// HBR, 2.7 Gbit/s per lane.
    Hbr,
    /// HBR2, 5.4 Gbit/s per lane.
    Hbr2,
    /// HBR3, 8.1 Gbit/s per lane.
    Hbr3,
    /// UHBR10, 10 Gbit/s per lane.
    Uhbr10,
    /// UHBR13.5, 13.5 Gbit/s per lane.
    Uhbr13_5,
    /// UHBR20, 20 Gbit/s per lane.
    Uhbr20,
}

/// A DisplayPort link configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DpLink {
    /// Lanes (1, 2 or 4).
    pub lanes: u8,
    /// Rate per lane.
    pub rate: DpRate,
}

/// Display Stream Compression support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DscSupport {
    /// DSC version, e.g. `1.2a`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Highest bits per component DSC can carry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bpc: Option<u8>,
    /// Native 4:2:0 compression supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_420: Option<bool>,
    /// Highest FRL rate with DSC (HDMI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_frl: Option<FrlRate>,
    /// Maximum slices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_slices: Option<u8>,
}

/// Video codecs for decode capabilities and content requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    /// MPEG-2 Video.
    Mpeg2,
    /// H.264 / AVC.
    H264,
    /// H.265 / HEVC.
    Hevc,
    /// VP8.
    Vp8,
    /// VP9.
    Vp9,
    /// AV1.
    Av1,
    /// H.266 / VVC.
    Vvc,
    /// VC-1.
    Vc1,
}

/// Decode APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecodeApi {
    /// VA-API.
    VaApi,
    /// Vulkan Video.
    VulkanVideo,
}

/// One hardware decode profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecodeProfile {
    /// API that reports it.
    pub api: DecodeApi,
    /// Codec.
    pub codec: VideoCodec,
    /// Profile name as the API names it, e.g. `Main10`, `Profile2`.
    pub profile: String,
    /// Bit depths decodable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bit_depths: Vec<u8>,
    /// Chroma formats decodable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chroma: Vec<ColorFormat>,
    /// Largest picture width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    /// Largest picture height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lfc_needs_twice_min() {
        let r = |min, max| VrrRange {
            min_millihz: min,
            max_millihz: max,
            kind: VrrKind::AdaptiveSync,
        };
        assert!(r(48_000, 120_000).supports_lfc());
        assert!(!r(48_000, 60_000).supports_lfc());
        assert!(r(40_000, 80_000).supports_lfc());
    }

    #[test]
    fn frl_total() {
        let f = FrlRate {
            lanes: 4,
            gbps_per_lane: 12,
        };
        assert_eq!(f.total_gbps(), 48);
    }

    #[test]
    fn mode_omits_defaults() {
        let m = Mode {
            width: 3840,
            height: 2160,
            refresh_millihz: 23_976,
            interlaced: false,
            timing: None,
            vic: Some(93),
            hdmi_vic: None,
            preferred: false,
            ycbcr420: Ycbcr420::No,
            sources: vec![ModeSource::CtaVic],
        };
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"width":3840,"height":2160,"refresh_millihz":23976,"vic":93,"sources":["cta_vic"]}"#
        );
    }
}
