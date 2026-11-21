// SPDX-License-Identifier: LGPL-2.1-or-later
// AppliedQuirk and PermissionGap copied from AudioCaps v1.0.0 (commit 049d053)
// crates/audiocaps-model/src/detail.rs; QuirkEffect redefined for video. The per-layer state types
// are new for DisplayCaps.
//! Layer detail: state that a capability set can't express (the active mode, KMS properties, the
//! inferred link, compositor switches), plus quirk effects and sandbox permission gaps.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::caps::{
    ColorFormat, Colorimetry, DpLink, Eotf, FrlRate, HdrCapabilities, MilliHertz, Mode, QuantRange,
};
use crate::snapshot::CompositorKind;
use crate::value::Value;

/// State and detail per layer.
// The variants are plain data in a JSON model; boxing would change the public type for no gain.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "layer", rename_all = "snake_case")]
pub enum LayerDetail {
    /// Host hardware behind the connector.
    HostHardware(HostDetail),
    /// Kernel driver state.
    KernelDriver(DriverDetail),
    /// KMS state of the connector and its CRTC.
    Kms(KmsState),
    /// The link as worked out from the other layers.
    Link(LinkState),
    /// The compositor's view of the output.
    Compositor(CompositorOutput),
}

/// GPU and connector hardware.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HostDetail {
    /// The GPU this connector belongs to.
    pub gpu: Value<Gpu>,
    /// Connector index on the card for its type, e.g. `1` for `HDMI-A-1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_index: Option<u32>,
}

/// A GPU.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Gpu {
    /// DRM card, e.g. `card1`.
    pub card: String,
    /// PCI vendor id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pci_vendor: Option<u16>,
    /// PCI device id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pci_device: Option<u16>,
    /// PCI slot, e.g. `0000:03:00.0` (bus topology, not personal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pci_slot: Option<String>,
    /// Vendor name from the id table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_name: Option<String>,
    /// Kernel driver, e.g. `amdgpu`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
}

/// Kernel driver state that affects the display path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DriverDetail {
    /// Driver name, e.g. `amdgpu`, `i915`, `virtio_gpu`.
    pub driver: Value<String>,
    /// Module parameters that matter for display (e.g. `dcfeaturemask`), as read.
    pub parameters: Value<Vec<ModuleParameter>>,
    /// Feature gates worked out from the parameters and kernel version (e.g. HDMI FRL).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<DriverFeature>,
}

/// One module parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModuleParameter {
    /// Name, e.g. `dcfeaturemask`.
    pub name: String,
    /// Value as the kernel prints it.
    pub value: String,
}

/// A driver feature that gates a display capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DriverFeature {
    /// Feature, e.g. `hdmi_frl`.
    pub name: String,
    /// Whether it is enabled.
    pub enabled: Value<bool>,
}

/// KMS state of one connector, read with read-only ioctls. Never set, never forced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KmsState {
    /// The active mode, when the connector drives a CRTC.
    pub active_mode: Value<Option<Mode>>,
    /// `max bpc`: the current value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bpc: Option<Value<u8>>,
    /// `Colorspace` property, as named by the driver (e.g. `BT2020_RGB`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorspace: Option<Value<String>>,
    /// `color format` property (Linux 7.3+), when the driver has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_format: Option<Value<ColorFormat>>,
    /// `HDR_OUTPUT_METADATA` blob, decoded; `None` inside the value when unset (SDR).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr_output_metadata: Option<Value<Option<HdrMetadata>>>,
    /// `Broadcast RGB` (`Automatic`, `Full`, `Limited 16:235`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast_rgb: Option<Value<String>>,
    /// `vrr_capable` on the connector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vrr_capable: Option<Value<bool>>,
    /// `VRR_ENABLED` on the CRTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vrr_enabled: Option<Value<bool>>,
    /// `content type`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<Value<String>>,
    /// `link-status` (`Good` or `Bad`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_status: Option<Value<String>>,
    /// `panel orientation`, for internal panels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_orientation: Option<Value<String>>,
}

/// HDR static metadata (CTA-861 infoframe type 1), as sent to the sink.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HdrMetadata {
    /// Transfer function signalled.
    pub eotf: Eotf,
    /// Mastering display max luminance, in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_mastering_nits: Option<f32>,
    /// Mastering display min luminance, in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_mastering_nits: Option<f32>,
    /// MaxCLL, in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cll_nits: Option<u16>,
    /// MaxFALL, in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fall_nits: Option<u16>,
}

/// The signal on the wire, worked out from the mode, the properties and the link limits.
/// Always `inferred`: KMS reports what was requested, not always what was negotiated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LinkState {
    /// TMDS, FRL or DisplayPort, and its rate.
    pub signal: Value<LinkSignal>,
    /// Colour format on the wire.
    pub color_format: Value<ColorFormat>,
    /// Bits per component on the wire.
    pub bit_depth: Value<u8>,
    /// Quantization range on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Value<QuantRange>>,
    /// Whether DSC is in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsc: Option<Value<bool>>,
    /// Data rate the signal needs, in kbit/s.
    pub required_kbps: Value<u64>,
    /// Data rate the link can carry, in kbit/s.
    pub capacity_kbps: Value<u64>,
    /// Devices between the GPU and the sink that limit the link (adapter, dock, MST).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limited_by: Vec<String>,
}

/// The physical signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkSignal {
    /// HDMI TMDS.
    Tmds {
        /// TMDS character rate in kHz.
        character_rate_khz: u32,
    },
    /// HDMI 2.1 FRL.
    Frl {
        /// The FRL rate.
        rate: FrlRate,
    },
    /// DisplayPort.
    DisplayPort {
        /// Lanes and rate.
        link: DpLink,
    },
    /// A virtual link (VM display, writeback): no wire, no limits.
    Virtual,
}

/// The compositor's view of one output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompositorOutput {
    /// Which compositor reported it.
    pub compositor: CompositorKind,
    /// The output's name in the compositor, e.g. `HDMI-A-1`.
    pub output_name: Value<String>,
    /// The output is enabled.
    pub enabled: Value<bool>,
    /// The current mode as the compositor reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Value<Mode>>,
    /// Scale factor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Value<f32>>,
    /// HDR mode switched on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr_enabled: Option<Value<bool>>,
    /// Wide colour gamut (BT.2020 signalling) switched on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wide_color_enabled: Option<Value<bool>>,
    /// SDR content brightness in nits, when HDR is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdr_brightness_nits: Option<Value<f32>>,
    /// The output's image description (color-management-v1, or the compositor's own API).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_description: Option<Value<ImageDescription>>,
    /// VRR policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vrr_policy: Option<Value<VrrPolicy>>,
    /// RGB range setting the compositor applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb_range: Option<Value<String>>,
    /// Max bits per component the compositor requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bpc: Option<Value<u8>>,
    /// HDR capabilities as the compositor reports them for the output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr: Option<Value<HdrCapabilities>>,
}

/// An image description: what the compositor will turn content into for this output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImageDescription {
    /// Named primaries, e.g. `srgb`, `bt2020`, or `custom`.
    pub primaries: String,
    /// Named transfer function, e.g. `srgb`, `st2084_pq`, `bt1886`.
    pub transfer: String,
    /// Min luminance, in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_luminance_nits: Option<f32>,
    /// Max (peak) luminance, in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_luminance_nits: Option<f32>,
    /// Reference white luminance, in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_luminance_nits: Option<f32>,
    /// Target colour volume primaries, when different from `primaries`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_primaries: Option<String>,
    /// Target max luminance, in nits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_max_luminance_nits: Option<f32>,
}

/// When the compositor uses variable refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VrrPolicy {
    /// Never.
    Never,
    /// Always.
    Always,
    /// Only for fullscreen surfaces that ask for it (games, video).
    Automatic,
}

/// What a quirk entry corrects or adds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AppliedQuirk {
    /// Entry id, e.g. `lg-cx-zero-peak`.
    pub id: String,
    /// What the entry corrects or adds.
    pub description: String,
    /// The pack it came from: `bundled`, or the directory that overrode it.
    pub origin: String,
    /// Where the facts come from (spec sheet, measurement, bug report).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Effects.
    pub effects: Vec<QuirkEffect>,
}

/// One effect of a quirk entry. Effects are reported, and values they change are marked
/// `inferred` with a `quirk` source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum QuirkEffect {
    /// A capability no EDID field carries, e.g. `dolby_vision_low_latency_only`.
    Feature {
        /// Feature name.
        name: String,
    },
    /// The sink's real peak luminance, when the EDID misreports it.
    PeakLuminance {
        /// Nits.
        nits: f32,
    },
    /// An advertised mode the sink can't actually show.
    RemoveMode {
        /// Width.
        width: u16,
        /// Height.
        height: u16,
        /// Refresh in millihertz.
        refresh_millihz: MilliHertz,
    },
    /// An HDR format the sink advertises but handles badly, or doesn't advertise but accepts.
    HdrFormat {
        /// The transfer function.
        eotf: Eotf,
        /// Whether the sink really accepts it.
        supported: bool,
    },
    /// Colorimetry the sink accepts but doesn't advertise.
    AddColorimetry {
        /// The colorimetry.
        colorimetry: Colorimetry,
    },
    /// The EDID is fixed or copied (splitters, extractors), so it describes another device.
    FixedEdid,
    /// A note for diagnostics.
    Note {
        /// Text.
        text: String,
    },
}

/// A sandbox permission the app lacks, with what it hides and the fix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PermissionGap {
    /// The permission, e.g. `--device=dri`, `hardware-observe` (Snap plug).
    pub permission: String,
    /// Layers it hides.
    pub layers: Vec<crate::scope::LayerKind>,
    /// What the user or packager runs or adds, e.g. `flatpak override --user --device=dri <app>`.
    pub fix: String,
    /// Needed for basic operation (else it only adds detail).
    pub required: bool,
}
