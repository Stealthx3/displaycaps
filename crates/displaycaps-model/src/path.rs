// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-model/src/path.rs; effective sets,
// limits, path reports, hops and losses kept in shape; access paths replaced by presentation
// paths, stream formats by video formats, and content requests and recommendations added.
//! Presentation paths, effective sets, path reports, content requests and recommendations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::caps::{
    CapabilitySet, ColorFormat, Colorimetry, HdrFormat, MilliHertz, Mode, QuantRange, VideoCodec,
};
use crate::endpoint::Availability;
use crate::finding::FindingCode;
use crate::scope::{FieldGroup, LayerKind};
use crate::value::Value;

/// How an application puts pictures on the screen. The effective set differs per path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PresentationPath {
    /// A Wayland surface with a color-management-v1 image description (the HDR path on Wayland).
    WaylandColorManaged,
    /// A plain Wayland surface (sRGB, SDR).
    Wayland,
    /// An X11 window (SDR; no HDR path for apps).
    X11,
    /// Direct KMS: the app is DRM master (kiosk, Gamescope session) and sets modes itself.
    DirectKms,
}

impl PresentationPath {
    /// Every path.
    pub const ALL: [PresentationPath; 4] = [
        Self::WaylandColorManaged,
        Self::Wayland,
        Self::X11,
        Self::DirectKms,
    ];
}

/// What survives every layer for one presentation path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EffectiveSet {
    /// The presentation path.
    pub presentation_path: PresentationPath,
    /// Whether the path could be evaluated.
    pub availability: Availability,
    /// The effective capabilities.
    pub caps: CapabilitySet,
    /// Which layer limits which field group.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limited_by: Vec<Limit>,
}

/// One layer limiting one field group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Limit {
    /// The limiting layer.
    pub layer: LayerKind,
    /// The limited field group.
    pub field: FieldGroup,
}

/// The hops from the application's content to the panel for one presentation path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PathReport {
    /// The presentation path.
    pub presentation_path: PresentationPath,
    /// The content the report was computed for, when the application gave one; otherwise the
    /// report describes the desktop as it is now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentRequest>,
    /// Hops in order, application first.
    pub hops: Vec<Hop>,
    /// Index into `hops` of the bottleneck, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottleneck: Option<u16>,
}

/// Kinds of hop, from the content to the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HopKind {
    /// The application and its content.
    Application,
    /// The video decoder.
    Decoder,
    /// The renderer and tone mapper (what the player renders to).
    Renderer,
    /// The compositor (Wayland or X11).
    Compositor,
    /// KMS and the kernel driver.
    KmsDriver,
    /// The HDMI/DP link.
    Link,
    /// The sink.
    Sink,
    /// Devices between the GPU and the display (receiver, splitter, adapter, dock).
    SinkChain,
    /// The sandbox or session around the app.
    Environment,
    /// The whole path (for findings that are not about one hop).
    WholePath,
}

/// One hop in a path report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Hop {
    /// Kind of hop.
    pub kind: HopKind,
    /// Short label, e.g. `KWin 6.5`, `HDMI TMDS 594 MHz`.
    pub label: String,
    /// Format carried at this hop.
    pub format: Value<VideoFormat>,
    /// Data rate carried, in kbit/s (link hops).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_kbps: Option<Value<u64>>,
    /// Highest data rate this hop can carry, in kbit/s (link hops).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ceiling_kbps: Option<Value<u64>>,
    /// What is lost at this hop.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub losses: Vec<Loss>,
}

/// A picture format at a hop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VideoFormat {
    /// Width in pixels.
    pub width: u16,
    /// Height in lines.
    pub height: u16,
    /// Refresh or frame rate in millihertz.
    pub rate_mhz: MilliHertz,
    /// Colour format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_format: Option<ColorFormat>,
    /// Bits per component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit_depth: Option<u8>,
    /// HDR format, `None` for SDR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr: Option<HdrFormat>,
    /// Colorimetry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorimetry: Option<Colorimetry>,
    /// Quantization range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<QuantRange>,
}

/// Something lost at a hop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Loss {
    /// Kind of loss.
    pub kind: LossKind,
    /// Detail, e.g. `10 → 8 bit`.
    pub detail: String,
    /// The finding that explains it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding: Option<FindingCode>,
}

/// Kinds of loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LossKind {
    /// Resolution reduced or scaled.
    Resolution,
    /// Refresh doesn't match the content, or is lower than asked.
    Refresh,
    /// Bits per component reduced.
    BitDepth,
    /// Chroma subsampled (4:2:2 or 4:2:0).
    Chroma,
    /// Quantization range changed or mismatched.
    Range,
    /// HDR tone-mapped to SDR.
    Hdr,
    /// Dynamic HDR (Dolby Vision, HDR10+) dropped to static.
    DynamicHdr,
    /// Wide gamut mapped to a smaller one.
    Gamut,
    /// Variable refresh not used.
    Vrr,
    /// Hardware decode not available.
    Decode,
}

/// What an application intends to show, for content queries, path reports and recommendations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContentRequest {
    /// Codec, for decode checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<VideoCodec>,
    /// Codec profile, e.g. `Main10`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Bits per component of the content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit_depth: Option<u8>,
    /// Width in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u16>,
    /// Height in lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u16>,
    /// Frame rate in millihertz (23.976 fps is `23976`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_rate_mhz: Option<MilliHertz>,
    /// HDR format, `None` for SDR content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr: Option<HdrFormat>,
    /// Colorimetry, e.g. BT.2020.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorimetry: Option<Colorimetry>,
    /// Chroma format the content needs on the wire to look right (4:4:4 for desktop text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_chroma: Option<ColorFormat>,
}

/// A recommendation for content on a path. DisplayCaps returns it; the app or the compositor
/// applies it. DisplayCaps never changes a mode or setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Recommendation {
    /// The endpoint it is for.
    pub endpoint: crate::endpoint::EndpointId,
    /// The presentation path it is for.
    pub presentation_path: PresentationPath,
    /// The mode to switch to (a mode the sink lists).
    pub mode: Mode,
    /// Colour format the link would carry.
    pub color_format: ColorFormat,
    /// Bits per component the link would carry.
    pub bit_depth: u8,
    /// HDR format to render for, `None` to render SDR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr: Option<HdrFormat>,
    /// Peak luminance to tone-map to, in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_peak_nits: Option<f32>,
    /// What the recommendation gives up, if anything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compromises: Vec<Loss>,
}
