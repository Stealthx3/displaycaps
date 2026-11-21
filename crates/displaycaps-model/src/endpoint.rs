// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-model/src/endpoint.rs; sink
// identity, the sink-key algorithm (from crates/audiocaps-backends/src/assemble/sink.rs),
// correlation keys, layer capabilities and availability kept; endpoints redefined as display
// connectors and the sink record extended with video fields.
//! Display endpoints: one connector with whatever is plugged into it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::caps::{CapabilitySet, LinkLimits};
use crate::detail::{AppliedQuirk, LayerDetail};
use crate::path::{EffectiveSet, PathReport};
use crate::redact::{Redact, RedactionPolicy, Sensitive, stable_hash};
use crate::scope::LayerKind;
use crate::value::{Confidence, UnknownReason, Value};

/// Stable endpoint identifier, e.g. `drm:card1-HDMI-A-1`. Stable across probes on the same system
/// while the hardware and configuration stay the same.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct EndpointId(pub String);

/// Endpoint class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndpointClass {
    /// External connectors: HDMI, DisplayPort, USB-C alt mode, DVI, VGA (also through adapters,
    /// docks and MST hubs; one endpoint per sink).
    External,
    /// Internal panels: eDP, LVDS, DSI.
    Internal,
    /// Virtual outputs: VM displays, headless outputs, network streaming, writeback.
    Virtual,
}

/// The physical connector type, as the driver names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorType {
    /// HDMI type A or B.
    Hdmi,
    /// DisplayPort (also USB-C alt mode, which KMS reports as DisplayPort).
    DisplayPort,
    /// USB-C, when the driver reports it as such.
    UsbC,
    /// DVI-I, DVI-D or DVI-A.
    Dvi,
    /// VGA.
    Vga,
    /// Embedded DisplayPort.
    Edp,
    /// LVDS.
    Lvds,
    /// MIPI DSI.
    Dsi,
    /// A virtual connector (virtio-gpu, QXL, VKMS).
    Virtual,
    /// A writeback connector (never a display; listed for completeness).
    Writeback,
    /// Something else, named as the driver names it.
    Other(String),
}

/// Whether something is plugged in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    /// A sink is connected.
    Connected,
    /// Nothing is connected.
    Disconnected,
    /// The driver can't tell.
    Unknown,
}

/// One display endpoint with everything reported about it (within the probe scope).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Endpoint {
    /// Stable identifier.
    pub id: EndpointId,
    /// Class.
    pub class: EndpointClass,
    /// Connector type.
    pub connector: Value<ConnectorType>,
    /// Connection status (from the kernel's cached state; never a forced probe).
    pub status: Value<ConnectionStatus>,
    /// Display name, e.g. the compositor's output description. Product names are not personal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<Value<String>>,
    /// Keys that join this endpoint across layers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub correlation: Vec<CorrelationKey>,
    /// The connected sink, when one is connected and the sink field group is in scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sink: Option<SinkRecord>,
    /// Capabilities and state as each layer reports them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerCapabilities>,
    /// What survives every layer, per presentation path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effective: Vec<EffectiveSet>,
    /// Path reports, per presentation path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<PathReport>,
}

/// A key that identifies this endpoint in one layer, with how sure the join is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CorrelationKey {
    /// The key.
    pub id: CorrelationId,
    /// Measured when the layer states the key directly; inferred when matched heuristically.
    pub confidence: Confidence,
}

/// Identifiers used to join an endpoint across layers (and, through the DRM connector and sink
/// key, with AudioCaps' HDMI/DP audio endpoints).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CorrelationId {
    /// DRM connector, e.g. `card1-HDMI-A-1` (the same key AudioCaps uses).
    DrmConnector {
        /// Connector name.
        name: String,
    },
    /// KMS connector object id on a card.
    KmsConnector {
        /// Card, e.g. `card1`.
        card: String,
        /// Connector object id.
        id: u32,
    },
    /// KMS CRTC driving the connector.
    KmsCrtc {
        /// Card, e.g. `card1`.
        card: String,
        /// CRTC object id.
        id: u32,
    },
    /// Wayland `wl_output` name, e.g. `HDMI-A-1`.
    WlOutput {
        /// Output name.
        name: String,
    },
    /// Mutter DisplayConfig monitor spec.
    MutterMonitor {
        /// Connector, e.g. `HDMI-1`.
        connector: String,
        /// Vendor (PNP id).
        vendor: String,
        /// Product.
        product: String,
        /// Serial (redacted on export by default).
        serial: Sensitive,
    },
    /// X11 RandR output, e.g. `HDMI-A-0`.
    RandrOutput {
        /// Output name.
        name: String,
    },
    /// The sink key (shared with AudioCaps for the same device).
    SinkKey {
        /// The key.
        key: String,
    },
}

/// What the sink at the far end of the cable says it is and can do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SinkRecord {
    /// Identity of the sink, with the same fields and key as AudioCaps' sink identity.
    pub identity: SinkIdentity,
    /// EDID facts that aren't capabilities: version, extensions, checksum state.
    pub edid: Value<EdidInfo>,
    /// Video capabilities the sink advertises (EDID, CTA-861 and DisplayID blocks).
    pub video: CapabilitySet,
    /// Link limits the sink states (HDMI and HDMI Forum VSDBs, DisplayID).
    pub link: Value<LinkLimits>,
    /// Devices in between (receiver, splitter, adapter, dock), when known.
    pub chain: Value<Vec<ChainDevice>>,
    /// Quirk-data entries that matched this sink, with their effects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<AppliedQuirk>,
}

/// Identity of a sink. Same fields and key algorithm as AudioCaps' `SinkIdentity`, so the combined
/// library can join a display endpoint with its HDMI/DP audio endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SinkIdentity {
    /// Three-letter PNP manufacturer ID, e.g. `HEC`.
    pub manufacturer_id: Value<String>,
    /// Manufacturer name resolved from the PNP ID.
    pub manufacturer_name: Value<String>,
    /// Product code.
    pub product_code: Value<u16>,
    /// Monitor name, e.g. `HISENSE`.
    pub model_name: Value<String>,
    /// Serial number (redacted on export by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<Sensitive>,
    /// Stable key apps can store settings under; see [`sink_key`].
    pub sink_key: Value<String>,
}

/// The sink key: `<PNP id>-<product hex>-<serial hash>`, or `<PNP id>-<product hex>` without a
/// serial, or `<raw manufacturer hex>-<product hex>` when the manufacturer id isn't valid PNP.
///
/// Copied from AudioCaps v1.0.0 (`assemble/sink.rs`); the parity test in `displaycaps-parse`
/// checks it against AudioCaps' own output. `raw_manufacturer` is the manufacturer id as AudioCaps
/// reads it from the ELD (the two EDID bytes 8–9 as a little-endian number).
pub fn sink_key(
    pnp: Option<&str>,
    raw_manufacturer: u16,
    product: u16,
    serial: Option<&str>,
) -> String {
    match (pnp, serial) {
        (Some(m), Some(s)) => format!(
            "{m}-{product:04x}-{}",
            stable_hash(s).trim_start_matches("sha256:")
        ),
        (Some(m), None) => format!("{m}-{product:04x}"),
        (None, _) => format!("{raw_manufacturer:04x}-{product:04x}"),
    }
}

/// EDID facts that describe the data rather than the display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EdidInfo {
    /// EDID version, e.g. `1.4`.
    pub version: String,
    /// Length in bytes.
    pub length: u32,
    /// Extension block tags in order, e.g. `["cta-861", "displayid"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// Every block's checksum is valid.
    pub checksums_valid: bool,
    /// Week and year of manufacture, or model year, when stated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufactured: Option<String>,
    /// Digital input interface from the base block (`hdmi-a`, `displayport`, ...), when stated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
}

/// A device between the GPU and the display (receiver, splitter, extractor, adapter, dock).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChainDevice {
    /// What kind of device.
    pub kind: ChainDeviceKind,
    /// Its identity, when it has one of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<SinkIdentity>,
    /// How sure the detection is.
    pub confidence: Confidence,
    /// Why it was detected, e.g. `HDMI sink EDID on a DP connector`.
    pub evidence: String,
}

/// Kinds of device in the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChainDeviceKind {
    /// AV receiver or soundbar passing video through.
    Receiver,
    /// HDMI splitter or switch.
    Splitter,
    /// Audio extractor or EDID emulator (often a fixed EDID).
    Extractor,
    /// DP-to-HDMI (or similar) protocol converter.
    Adapter,
    /// Dock or USB-C hub.
    Dock,
    /// DisplayPort MST hub.
    MstHub,
}

/// One layer's view of an endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayerCapabilities {
    /// The layer.
    pub layer: LayerKind,
    /// Whether the layer could be read.
    pub availability: Availability,
    /// What the layer lets through. Fields outside the probe scope are omitted.
    pub caps: CapabilitySet,
    /// State and detail that a capability set can't express, when in scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<LayerDetail>,
}

/// Whether a layer could be read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Availability {
    /// Fully read.
    Read,
    /// Partly read; see the unknown fields.
    Partial,
    /// Not readable.
    Unknown {
        /// Why.
        reason: UnknownReason,
        /// Detail, e.g. the missing permission.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

impl Redact for Endpoint {
    fn redact(&mut self, policy: &RedactionPolicy) {
        self.correlation.redact(policy);
        self.sink.redact(policy);
    }
}

impl Redact for CorrelationKey {
    fn redact(&mut self, policy: &RedactionPolicy) {
        if let CorrelationId::MutterMonitor { serial, .. } = &mut self.id {
            serial.redact(policy);
        }
    }
}

impl Redact for SinkRecord {
    fn redact(&mut self, policy: &RedactionPolicy) {
        self.identity.redact(policy);
        self.chain.redact(policy);
    }
}

impl Redact for ChainDevice {
    fn redact(&mut self, policy: &RedactionPolicy) {
        self.identity.redact(policy);
    }
}

impl Redact for SinkIdentity {
    fn redact(&mut self, policy: &RedactionPolicy) {
        self.serial.redact(policy);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redact::{SensitiveKind, SensitiveValue};

    #[test]
    fn sink_key_forms() {
        assert_eq!(sink_key(Some("HEC"), 0xa320, 0xa030, None), "HEC-a030");
        let k = sink_key(Some("HEC"), 0xa320, 0xa030, Some("12345"));
        assert!(k.starts_with("HEC-a030-"));
        assert_eq!(k.len(), "HEC-a030-".len() + 16);
        assert_eq!(sink_key(None, 0x0000, 0x0001, Some("x")), "0000-0001");
    }

    #[test]
    fn redaction_reaches_serials() {
        let mut id = SinkIdentity {
            manufacturer_id: Value::unknown(UnknownReason::NotExposed, None),
            manufacturer_name: Value::unknown(UnknownReason::NotExposed, None),
            product_code: Value::unknown(UnknownReason::NotExposed, None),
            model_name: Value::unknown(UnknownReason::NotExposed, None),
            serial: Some(Sensitive::new(SensitiveKind::Serial, "SN1")),
            sink_key: Value::unknown(UnknownReason::NotExposed, None),
        };
        id.redact(&RedactionPolicy::default());
        assert_eq!(
            id.serial.unwrap().value,
            SensitiveValue::Redacted(stable_hash("SN1"))
        );
    }
}
