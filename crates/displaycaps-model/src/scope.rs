// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-model/src/scope.rs; layers, field
// groups and paths replaced with the display ones, directions removed (displays are outputs).
//! Probe scope: what the application asks for. Information on-demand is built into the API shape:
//! every probe takes a [`ProbeScope`], and nothing outside it is read, cached or reported.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::endpoint::{EndpointClass, EndpointId};
use crate::path::PresentationPath;

/// A selection of items: everything, or only the listed ones. An empty `Only` selects nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "select", content = "items", rename_all = "snake_case")]
pub enum Selection<T> {
    /// Every item.
    All,
    /// Only these items.
    Only(Vec<T>),
}

impl<T: PartialEq> Selection<T> {
    /// Whether `item` is selected.
    pub fn includes(&self, item: &T) -> bool {
        match self {
            Self::All => true,
            Self::Only(items) => items.contains(item),
        }
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Only(items) if items.is_empty())
    }
}

/// The eight layers DisplayCaps reads, from the far end of the cable to the app's sandbox.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    /// The connected sink and anything between it and the GPU (EDID; DDC/CI and CEC opt-in).
    SinkChain,
    /// Host hardware: GPU, PCI ids, connector types, link rates the hardware supports.
    HostHardware,
    /// Kernel driver: driver name, module parameters (e.g. the amdgpu feature mask), kernel.
    KernelDriver,
    /// KMS state read with read-only ioctls: active mode, max bpc, colorspace, HDR metadata, VRR.
    Kms,
    /// The HDMI/DP link, worked out from the layers above (always inferred).
    Link,
    /// The compositor: HDR, wide colour, image description, VRR policy, scale (Wayland or X11).
    Compositor,
    /// Hardware video decode (opt-in): VA-API and Vulkan Video profiles.
    Decode,
    /// Environment: session type, sandbox, permissions, distro, driver sources.
    Environment,
}

impl LayerKind {
    /// Every layer, in order from sink to app.
    pub const ALL: [LayerKind; 8] = [
        Self::SinkChain,
        Self::HostHardware,
        Self::KernelDriver,
        Self::Kms,
        Self::Link,
        Self::Compositor,
        Self::Decode,
        Self::Environment,
    ];
}

/// Groups of fields an application can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldGroup {
    /// Endpoint identity, names, connector type and correlation keys.
    Identity,
    /// Modes: resolution, refresh, timing, VICs, preferred mode.
    Modes,
    /// Variable refresh rate range and state.
    Vrr,
    /// Colour formats (RGB, YCbCr 4:4:4/4:2:2/4:2:0) and bit depths.
    ColorFormats,
    /// HDR: transfer functions, static and dynamic metadata, luminance.
    Hdr,
    /// Colorimetry (BT.2020, DCI-P3, xvYCC, ...).
    Colorimetry,
    /// Quantization range (limited or full).
    Quantization,
    /// Link limits and link state: TMDS clock, FRL, DP rates, DSC.
    Link,
    /// Hardware decode profiles (only filled by the opt-in decode check).
    Decode,
    /// Sink record (identity, video capabilities, devices in between, quirks).
    Sink,
    /// Effective sets per presentation path.
    Effective,
    /// Path reports with hops.
    Paths,
    /// Diagnostic findings.
    Findings,
    /// Driver state: driver, module parameters, kernel.
    Driver,
    /// Host hardware detail: GPU, PCI ids, connectors.
    Hardware,
    /// Compositor state: HDR and wide-colour switches, image description, scale, VRR policy.
    Compositor,
}

/// What a probe may read and report. Build it with [`ProbeScope::nothing`] and the `with_*`
/// methods, or start from [`ProbeScope::everything`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProbeScope {
    /// Endpoint classes to consider.
    pub endpoint_classes: Selection<EndpointClass>,
    /// Specific endpoints (by id) to consider.
    pub endpoints: Selection<EndpointId>,
    /// Layers to read.
    pub layers: Selection<LayerKind>,
    /// Field groups to report.
    pub fields: Selection<FieldGroup>,
    /// Presentation paths to compute effective sets and path reports for.
    pub presentation_paths: Selection<PresentationPath>,
}

impl ProbeScope {
    /// Selects nothing. Add what the application needs with the `with_*` methods.
    pub fn nothing() -> Self {
        Self {
            endpoint_classes: Selection::Only(vec![]),
            endpoints: Selection::All,
            layers: Selection::Only(vec![]),
            fields: Selection::Only(vec![]),
            presentation_paths: Selection::Only(vec![]),
        }
    }

    /// Selects everything the library can read (passive probes only; opt-ins stay separate calls).
    pub fn everything() -> Self {
        Self {
            endpoint_classes: Selection::All,
            endpoints: Selection::All,
            layers: Selection::All,
            fields: Selection::All,
            presentation_paths: Selection::All,
        }
    }

    /// Adds an endpoint class.
    pub fn with_class(mut self, class: EndpointClass) -> Self {
        push(&mut self.endpoint_classes, class);
        self
    }

    /// Restricts to one more specific endpoint.
    pub fn with_endpoint(mut self, id: EndpointId) -> Self {
        match &mut self.endpoints {
            Selection::All => self.endpoints = Selection::Only(vec![id]),
            Selection::Only(v) => v.push(id),
        }
        self
    }

    /// Adds a layer.
    pub fn with_layer(mut self, layer: LayerKind) -> Self {
        push(&mut self.layers, layer);
        self
    }

    /// Adds a field group.
    pub fn with_field(mut self, field: FieldGroup) -> Self {
        push(&mut self.fields, field);
        self
    }

    /// Adds a presentation path.
    pub fn with_presentation_path(mut self, path: PresentationPath) -> Self {
        push(&mut self.presentation_paths, path);
        self
    }

    /// Whether an endpoint of this class and id is in scope.
    pub fn includes_endpoint(&self, class: EndpointClass, id: &EndpointId) -> bool {
        self.endpoint_classes.includes(&class) && self.endpoints.includes(id)
    }
}

fn push<T: PartialEq>(sel: &mut Selection<T>, item: T) {
    if let Selection::Only(items) = sel
        && !items.contains(&item)
    {
        items.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_selects_nothing() {
        let s = ProbeScope::nothing();
        assert!(!s.layers.includes(&LayerKind::SinkChain));
        assert!(!s.includes_endpoint(
            EndpointClass::External,
            &EndpointId("drm:card1-HDMI-A-1".into())
        ));
    }

    #[test]
    fn builder_adds_only_what_was_asked() {
        let s = ProbeScope::nothing()
            .with_class(EndpointClass::External)
            .with_layer(LayerKind::SinkChain)
            .with_field(FieldGroup::Hdr);
        assert!(s.layers.includes(&LayerKind::SinkChain));
        assert!(!s.layers.includes(&LayerKind::Compositor));
        assert!(s.fields.includes(&FieldGroup::Hdr));
        assert!(!s.fields.includes(&FieldGroup::Modes));
        assert!(!s.includes_endpoint(
            EndpointClass::Internal,
            &EndpointId("drm:card0-eDP-1".into())
        ));
    }

    #[test]
    fn endpoint_restriction() {
        let id = EndpointId("drm:card1-HDMI-A-1".into());
        let s = ProbeScope::everything().with_endpoint(id.clone());
        assert!(s.includes_endpoint(EndpointClass::External, &id));
        assert!(!s.includes_endpoint(
            EndpointClass::External,
            &EndpointId("drm:card1-DP-1".into())
        ));
    }

    #[test]
    fn eight_layers_sink_to_app() {
        assert_eq!(LayerKind::ALL.len(), 8);
        assert_eq!(LayerKind::ALL[0], LayerKind::SinkChain);
        assert_eq!(LayerKind::ALL[7], LayerKind::Environment);
    }
}
