// SPDX-License-Identifier: LGPL-2.1-or-later
//! What each compositor source reported, as plain data: the live readers fill these, captures
//! save them (`commands/displaycaps-compositor.json`) and fixtures replay them. Serials and EDIDs
//! are redacted before a capture is written ([`CompositorFacts::redact`]).

use std::collections::BTreeMap;

use displaycaps_model::UnknownReason;
use serde::{Deserialize, Serialize};

/// Everything the compositor sources gave. A source that wasn't asked is `None`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompositorFacts {
    /// The Wayland registry and the output protocols read through it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wayland: Option<Read<WaylandFacts>>,
    /// Mutter's `org.gnome.Mutter.DisplayConfig.GetCurrentState`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutter: Option<Read<MutterFacts>>,
    /// X11 RandR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub randr: Option<Read<RandrFacts>>,
}

/// A source's result: its facts, or why it couldn't be read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Read<T> {
    /// Read.
    Ok(T),
    /// Not read.
    Err {
        /// Why.
        reason: UnknownReason,
        /// Detail.
        detail: String,
    },
}

impl<T> Read<T> {
    /// The facts, when read.
    pub fn ok(&self) -> Option<&T> {
        match self {
            Self::Ok(t) => Some(t),
            Self::Err { .. } => None,
        }
    }

    /// A failure.
    pub fn err(reason: UnknownReason, detail: impl Into<String>) -> Self {
        Self::Err {
            reason,
            detail: detail.into(),
        }
    }
}

/// One Wayland global.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Global {
    /// Interface name.
    pub interface: String,
    /// Advertised version.
    pub version: u32,
}

/// The Wayland side.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WaylandFacts {
    /// Every global the registry advertised (interface and version only).
    pub globals: Vec<Global>,
    /// `wl_output`s, when output state was asked for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<WlOutput>,
    /// `wp_color_manager_v1`, when offered and asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_manager: Option<ColorManager>,
    /// `zwlr_output_manager_v1` heads, when offered and asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wlr_heads: Option<Vec<WlrHead>>,
    /// `kde_output_device_v2` devices, when offered and asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kde_devices: Option<Vec<KdeDevice>>,
}

/// A mode as Wayland protocols state it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WlMode {
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Refresh in mHz (0 when the protocol gives none).
    pub refresh_mhz: i32,
    /// Marked preferred.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preferred: bool,
}

/// One `wl_output`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WlOutput {
    /// Bound version.
    pub version: u32,
    /// `name` (v4), e.g. `HDMI-A-1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `description` (v4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `geometry` make.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub make: Option<String>,
    /// `geometry` model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The current mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<WlMode>,
    /// Integer buffer scale (v2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<i32>,
    /// The output's image description from `wp_color_management_output_v1`, when read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_description: Option<Read<ImageInfo>>,
}

/// `wp_color_manager_v1`: what clients may use. Names are the protocol's enum entry names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorManager {
    /// Bound version.
    pub version: u32,
    /// `supported_feature`.
    pub features: Vec<String>,
    /// `supported_tf_named`.
    pub transfer_functions: Vec<String>,
    /// `supported_primaries_named`.
    pub primaries: Vec<String>,
    /// `supported_intent`.
    pub intents: Vec<String>,
}

/// `wp_image_description_info_v1`. Chromaticities are ×1 000 000, luminances as the protocol
/// sends them (`min` in 0.0001 cd/m², the rest in cd/m²).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInfo {
    /// `primaries_named`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primaries_named: Option<String>,
    /// `primaries`: r, g, b, white x/y.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primaries: Option<[i32; 8]>,
    /// `tf_named`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tf_named: Option<String>,
    /// `tf_power` exponent ×10 000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tf_power: Option<u32>,
    /// `luminances`: min (0.0001 cd/m²), max, reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub luminances: Option<[u32; 3]>,
    /// `target_primaries`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_primaries: Option<[i32; 8]>,
    /// `target_luminance`: min (0.0001 cd/m²), max.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_luminance: Option<[u32; 2]>,
    /// `target_max_cll`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_max_cll: Option<u32>,
    /// `target_max_fall`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_max_fall: Option<u32>,
    /// An ICC file was sent (its contents are not read).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub icc: bool,
}

/// One `zwlr_output_head_v1`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WlrHead {
    /// `name`.
    pub name: String,
    /// `description`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `make`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub make: Option<String>,
    /// `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `serial_number` (a hash after redaction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// `enabled`.
    pub enabled: bool,
    /// `mode` objects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<WlMode>,
    /// Index of `current_mode` in `modes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_mode: Option<usize>,
    /// `scale`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// `adaptive_sync` (v4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adaptive_sync: Option<bool>,
}

/// One `kde_output_device_v2`. Enum values are the protocol's numbers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KdeDevice {
    /// Bound version.
    pub version: u32,
    /// `name` (v2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `geometry` make.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub make: Option<String>,
    /// `geometry` model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `serial_number` (a hash after redaction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// `edid`, as the raw bytes in hex (serial zeroed after redaction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edid_hex: Option<String>,
    /// `enabled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// `mode` objects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<WlMode>,
    /// Index of `current_mode` in `modes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_mode: Option<usize>,
    /// `scale`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// `capabilities` flags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<u32>,
    /// `vrr_policy`: 0 never, 1 always, 2 automatic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vrr_policy: Option<u32>,
    /// `rgb_range`: 0 automatic, 1 full, 2 limited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb_range: Option<u32>,
    /// `high_dynamic_range` (v3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr: Option<bool>,
    /// `wide_color_gamut` (v3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wide_color: Option<bool>,
    /// `sdr_brightness` in nits (v3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdr_brightness: Option<u32>,
    /// `brightness_metadata` (v6): max peak, max frame average (nits), min (0.0001 nits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness_metadata: Option<[u32; 3]>,
    /// `brightness_overrides` (v6), -1 for no override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness_overrides: Option<[i32; 3]>,
    /// `color_profile_source` (v7): 0 sRGB, 1 ICC, 2 EDID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_profile_source: Option<u32>,
}

/// KDE capability flags.
pub(crate) mod kde_cap {
    pub const VRR: u32 = 0x2;
    pub const RGB_RANGE: u32 = 0x4;
    pub const HDR: u32 = 0x8;
}

/// Mutter's `GetCurrentState`. Property maps hold the variants as plain JSON.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MutterFacts {
    /// Configuration serial.
    pub serial: u32,
    /// Monitors.
    pub monitors: Vec<MutterMonitor>,
    /// Logical monitors.
    pub logical_monitors: Vec<MutterLogical>,
    /// Global properties.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, serde_json::Value>,
}

/// One Mutter monitor.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MutterMonitor {
    /// Connector, e.g. `HDMI-1`.
    pub connector: String,
    /// Vendor (PNP id).
    pub vendor: String,
    /// Product.
    pub product: String,
    /// Serial (a hash after redaction).
    pub serial: String,
    /// Modes.
    pub modes: Vec<MutterMode>,
    /// Properties (`display-name`, `is-builtin`, `color-mode`, ...).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, serde_json::Value>,
}

/// One Mutter mode.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MutterMode {
    /// Mode id.
    pub id: String,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
    /// Refresh in Hz.
    pub refresh: f64,
    /// Preferred scale.
    pub preferred_scale: f64,
    /// Properties (`is-current`, `is-preferred`, `is-interlaced`, `refresh-rate-mode`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, serde_json::Value>,
}

/// One Mutter logical monitor.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MutterLogical {
    /// Scale.
    pub scale: f64,
    /// Transform.
    pub transform: u32,
    /// Primary.
    pub primary: bool,
    /// Connectors of the monitors it shows.
    pub monitors: Vec<String>,
}

/// X11 RandR.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RandrFacts {
    /// Server vendor string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// Server release number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<u32>,
    /// RandR version (major, minor).
    pub version: [u32; 2],
    /// Outputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<RandrOutput>,
}

/// A RandR mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandrMode {
    /// Width.
    pub width: u16,
    /// Height.
    pub height: u16,
    /// Refresh in mHz, from the dot clock and totals.
    pub refresh_mhz: u32,
    /// Interlaced.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interlaced: bool,
}

/// One RandR output.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RandrOutput {
    /// Name, e.g. `HDMI-1` (modesetting) or `HDMI-A-0` (amdgpu).
    pub name: String,
    /// `connected`, `disconnected` or `unknown`.
    pub connection: String,
    /// The output's modes; the first `preferred` are preferred.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<RandrMode>,
    /// Number of preferred modes.
    #[serde(default)]
    pub preferred: usize,
    /// The CRTC's current mode, when the output drives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<RandrMode>,
    /// Output properties: atom and integer values as text, `EDID` as hex.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
}

impl CompositorFacts {
    /// Hashes serials and zeroes EDID serials, so the facts can be saved in a capture.
    pub fn redact(&mut self) {
        let hash = |s: &mut String| {
            if !s.is_empty() && !s.starts_with("sha256:") {
                *s = displaycaps_model::redact::stable_hash(s);
            }
        };
        let edid = |h: &mut String| {
            if let Some(mut b) = from_hex(h) {
                displaycaps_parse::edid::redact_serials(&mut b);
                *h = to_hex(&b);
            }
        };
        // Serials also appear inside free-text fields (wlroots' default description is
        // "make model serial (name)"): cut them out there first.
        let mut serials: Vec<String> = vec![];
        if let Some(Read::Ok(w)) = &self.wayland {
            serials.extend(
                w.wlr_heads
                    .iter()
                    .flatten()
                    .filter_map(|h| h.serial.clone()),
            );
            serials.extend(
                w.kde_devices
                    .iter()
                    .flatten()
                    .filter_map(|d| d.serial.clone()),
            );
        }
        if let Some(Read::Ok(m)) = &self.mutter {
            serials.extend(m.monitors.iter().map(|m| m.serial.clone()));
        }
        serials.retain(|s| s.len() >= 3 && !s.starts_with("sha256:"));
        let scrub = |t: &mut String| {
            for s in &serials {
                if t.contains(s.as_str()) {
                    *t = t.replace(s.as_str(), "<serial>");
                }
            }
        };
        if let Some(Read::Ok(w)) = &mut self.wayland {
            for o in &mut w.outputs {
                o.description.iter_mut().for_each(scrub);
                o.model.iter_mut().for_each(scrub);
            }
            for h in w.wlr_heads.iter_mut().flatten() {
                h.description.iter_mut().for_each(scrub);
                h.model.iter_mut().for_each(scrub);
                h.serial.iter_mut().for_each(hash);
            }
            for d in w.kde_devices.iter_mut().flatten() {
                d.model.iter_mut().for_each(scrub);
                d.serial.iter_mut().for_each(hash);
                d.edid_hex.iter_mut().for_each(edid);
            }
        }
        if let Some(Read::Ok(m)) = &mut self.mutter {
            for mon in &mut m.monitors {
                if let Some(serde_json::Value::String(n)) = mon.properties.get_mut("display-name") {
                    scrub(n);
                }
                hash(&mut mon.serial);
            }
        }
        if let Some(Read::Ok(r)) = &mut self.randr {
            for o in &mut r.outputs {
                if let Some(h) = o.properties.get_mut("EDID") {
                    edid(h);
                }
            }
        }
    }
}

/// Lower-case hex.
pub(crate) fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Bytes from hex; `None` if malformed.
pub(crate) fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Standard base64 (with or without padding); `None` if malformed.
pub(crate) fn from_base64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let s: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let s = match s.iter().position(|&c| c == b'=') {
        Some(p) if s[p..].iter().all(|&c| c == b'=') && s.len() - p <= 2 => &s[..p],
        Some(_) => return None,
        None => &s[..],
    };
    if s.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for chunk in s.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= val(c)? << (18 - 6 * i);
        }
        let bytes = acc.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_hex() {
        assert_eq!(from_base64("AP8A/w==").unwrap(), [0, 255, 0, 255]);
        assert_eq!(from_base64("AP8A/w").unwrap(), [0, 255, 0, 255]);
        assert_eq!(from_base64("TWFu").unwrap(), b"Man");
        assert_eq!(from_base64("TWE=").unwrap(), b"Ma");
        assert!(from_base64("T").is_none() && from_base64("T=W=").is_none());
        assert!(from_base64("@@@@").is_none());
        assert_eq!(from_hex(&to_hex(&[1, 0xab])).unwrap(), [1, 0xab]);
        assert!(from_hex("abc").is_none() && from_hex("zz").is_none());
    }

    #[test]
    fn redaction_hashes_serials() {
        let mut f = CompositorFacts {
            mutter: Some(Read::Ok(MutterFacts {
                monitors: vec![MutterMonitor {
                    serial: "SN123".into(),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        f.wayland = Some(Read::Ok(WaylandFacts {
            outputs: vec![WlOutput {
                description: Some("Make Model AB12345 (DP-1)".into()),
                ..Default::default()
            }],
            wlr_heads: Some(vec![WlrHead {
                description: Some("Make Model AB12345 (DP-1)".into()),
                serial: Some("AB12345".into()),
                ..Default::default()
            }]),
            ..Default::default()
        }));
        f.redact();
        let w = f.wayland.as_ref().unwrap().ok().unwrap();
        assert_eq!(
            w.outputs[0].description.as_deref(),
            Some("Make Model <serial> (DP-1)")
        );
        let h = &w.wlr_heads.as_ref().unwrap()[0];
        assert_eq!(h.description.as_deref(), Some("Make Model <serial> (DP-1)"));
        assert!(h.serial.as_ref().unwrap().starts_with("sha256:"));
        let s = &f.mutter.as_ref().unwrap().ok().unwrap().monitors[0].serial;
        assert!(s.starts_with("sha256:"));
        let once = s.clone();
        f.redact();
        assert_eq!(f.mutter.unwrap().ok().unwrap().monitors[0].serial, once);
    }
}
