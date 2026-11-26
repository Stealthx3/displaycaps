// SPDX-License-Identifier: LGPL-2.1-or-later
//! The sysfs DRM reader: cards and connectors under `/sys/class/drm`, with the kernel's cached
//! connector state. Every attribute read here is a cached value: `status`, `enabled`, `dpms`,
//! `modes` and `edid` never make the driver probe the connector (only a write to `status` would,
//! and nothing is ever written). EDID bytes are read only for connectors in scope.

use displaycaps_model::{ConnectionStatus, ConnectorType, EndpointClass};

use crate::root::SystemRoot;

/// The sysfs DRM class directory.
pub(crate) const CLASS: &str = "/sys/class/drm";

/// A DRM card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SysCard {
    /// `card0`.
    pub name: String,
    /// PCI vendor id, when the card is a PCI device.
    pub pci_vendor: Option<u16>,
    /// PCI device id.
    pub pci_device: Option<u16>,
    /// PCI slot, e.g. `0000:03:00.0`.
    pub pci_slot: Option<String>,
    /// The bound driver of the card's device, e.g. `amdgpu`.
    pub driver: Option<String>,
}

/// A DRM connector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SysConnector {
    /// `card0-HDMI-A-1`: the endpoint id and the DRM connector correlation key.
    pub name: String,
    /// `card0`.
    pub card: String,
    /// The type part of the name, e.g. `HDMI-A`.
    pub kind: String,
    /// The index part of the name, e.g. `1`.
    pub index: Option<u32>,
    /// `status`: the kernel's cached connection state.
    pub status: Option<ConnectionStatus>,
    /// `enabled`: whether the connector drives a CRTC.
    pub enabled: Option<bool>,
    /// `connector_id`: the KMS object id (Linux 6.2+).
    pub connector_id: Option<u32>,
    /// `modes`: the kernel's cached mode list, as `WxH` names.
    pub modes: Vec<String>,
}

impl SysConnector {
    /// The sysfs directory of this connector.
    pub fn dir(&self) -> String {
        format!("{CLASS}/{}", self.name)
    }

    /// Path of the cached EDID.
    pub fn edid_path(&self) -> String {
        format!("{}/edid", self.dir())
    }
}

/// Reads every card and connector. Fails only when `/sys/class/drm` can't be listed.
pub(crate) fn scan(root: &SystemRoot) -> std::io::Result<(Vec<SysCard>, Vec<SysConnector>)> {
    let entries = root.read_dir(CLASS)?;
    let mut cards = vec![];
    let mut conns = vec![];
    for e in entries {
        let Some(rest) = e.name.strip_prefix("card") else {
            continue;
        };
        match rest.split_once('-') {
            None if rest.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty() => {
                cards.push(card(root, &e.name));
            }
            Some((n, conn)) if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() => {
                conns.push(connector(root, &e.name, &format!("card{n}"), conn));
            }
            _ => {}
        }
    }
    Ok((cards, conns))
}

fn read_trim(root: &SystemRoot, path: &str) -> Option<String> {
    root.read_string(path).ok().map(|s| s.trim().to_string())
}

fn hex_u16(s: Option<String>) -> Option<u16> {
    s.and_then(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).ok())
}

fn basename(link: &str) -> Option<String> {
    link.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn card(root: &SystemRoot, name: &str) -> SysCard {
    let dev = format!("{CLASS}/{name}/device");
    let slot = root.read_link(&dev).ok().and_then(|l| basename(&l));
    // Only a PCI device has a vendor and device id: a slot looks like `0000:03:00.0`.
    let pci = slot.as_deref().is_some_and(is_pci_slot);
    SysCard {
        name: name.to_string(),
        pci_vendor: pci
            .then(|| hex_u16(read_trim(root, &format!("{dev}/vendor"))))
            .flatten(),
        pci_device: pci
            .then(|| hex_u16(read_trim(root, &format!("{dev}/device"))))
            .flatten(),
        pci_slot: slot.filter(|s| is_pci_slot(s)),
        driver: root
            .read_link(&format!("{dev}/driver"))
            .ok()
            .and_then(|l| basename(&l)),
    }
}

fn is_pci_slot(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 12
        && b[4] == b':'
        && b[7] == b':'
        && b[10] == b'.'
        && s.chars()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7 | 10) || c.is_ascii_hexdigit())
}

fn connector(root: &SystemRoot, name: &str, card: &str, conn: &str) -> SysConnector {
    let dir = format!("{CLASS}/{name}");
    let (kind, index) = match conn.rsplit_once('-') {
        Some((k, i)) if i.chars().all(|c| c.is_ascii_digit()) => (k.to_string(), i.parse().ok()),
        _ => (conn.to_string(), None),
    };
    SysConnector {
        name: name.to_string(),
        card: card.to_string(),
        kind,
        index,
        status: read_trim(root, &format!("{dir}/status")).map(|s| match s.as_str() {
            "connected" => ConnectionStatus::Connected,
            "disconnected" => ConnectionStatus::Disconnected,
            _ => ConnectionStatus::Unknown,
        }),
        enabled: read_trim(root, &format!("{dir}/enabled")).map(|s| s == "enabled"),
        connector_id: read_trim(root, &format!("{dir}/connector_id")).and_then(|s| s.parse().ok()),
        modes: root
            .read_string(&format!("{dir}/modes"))
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default(),
    }
}

/// The connector type for the type part of a connector name (`HDMI-A`, `DP`, `eDP`, ...), as the
/// kernel's `drm_connector_enum_list` names them.
pub(crate) fn connector_type(kind: &str) -> ConnectorType {
    match kind {
        "HDMI-A" | "HDMI-B" => ConnectorType::Hdmi,
        "DP" => ConnectorType::DisplayPort,
        "eDP" => ConnectorType::Edp,
        "DVI-I" | "DVI-D" | "DVI-A" => ConnectorType::Dvi,
        "VGA" => ConnectorType::Vga,
        "LVDS" => ConnectorType::Lvds,
        "DSI" => ConnectorType::Dsi,
        "USB" | "USB-C" => ConnectorType::UsbC,
        "Virtual" => ConnectorType::Virtual,
        "Writeback" => ConnectorType::Writeback,
        other => ConnectorType::Other(other.to_string()),
    }
}

/// The type part of a connector name for a KMS connector type code (`DRM_MODE_CONNECTOR_*`).
pub(crate) fn kind_for_code(code: u32) -> Option<&'static str> {
    Some(match code {
        0 => "Unknown",
        1 => "VGA",
        2 => "DVI-I",
        3 => "DVI-D",
        4 => "DVI-A",
        5 => "Composite",
        6 => "SVIDEO",
        7 => "LVDS",
        8 => "Component",
        9 => "DIN",
        10 => "DP",
        11 => "HDMI-A",
        12 => "HDMI-B",
        13 => "TV",
        14 => "eDP",
        15 => "Virtual",
        16 => "DSI",
        17 => "DPI",
        18 => "Writeback",
        19 => "SPI",
        20 => "USB",
        _ => return None,
    })
}

/// The endpoint class of a connector type.
pub(crate) fn class(t: &ConnectorType) -> EndpointClass {
    match t {
        ConnectorType::Edp | ConnectorType::Lvds | ConnectorType::Dsi => EndpointClass::Internal,
        ConnectorType::Virtual | ConnectorType::Writeback => EndpointClass::Virtual,
        ConnectorType::Other(k) if matches!(k.as_str(), "DPI" | "SPI") => EndpointClass::Internal,
        _ => EndpointClass::External,
    }
}

/// Vendor name for a PCI vendor id (display hardware only).
pub(crate) fn vendor_name(id: u16) -> Option<&'static str> {
    Some(match id {
        0x1002 => "AMD",
        0x8086 => "Intel",
        0x10de => "NVIDIA",
        0x1af4 => "Red Hat (virtio)",
        0x1234 => "QEMU",
        0x1b36 => "Red Hat (QEMU)",
        0x15ad => "VMware",
        0x1a03 => "ASPEED",
        0x102b => "Matrox",
        0x5143 => "Qualcomm",
        _ => return None,
    })
}

/// Module parameters that matter for display, per kernel module. Read only for the module the
/// card's driver uses, and only when driver detail is in scope.
pub(crate) fn display_parameters(driver: &str) -> &'static [(&'static str, &'static str)] {
    match driver {
        "amdgpu" => &[
            ("amdgpu", "dc"),
            ("amdgpu", "dcfeaturemask"),
            ("amdgpu", "dcdebugmask"),
            ("amdgpu", "freesync_video"),
            ("amdgpu", "abmlevel"),
            ("drm", "edid_firmware"),
        ],
        "i915" | "xe" => &[
            ("i915", "enable_psr"),
            ("i915", "enable_dpcd_backlight"),
            ("drm", "edid_firmware"),
        ],
        "nvidia" => &[
            ("nvidia_drm", "modeset"),
            ("nvidia_drm", "fbdev"),
            ("drm", "edid_firmware"),
        ],
        "nouveau" => &[("nouveau", "modeset"), ("drm", "edid_firmware")],
        _ => &[("drm", "edid_firmware")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_types() {
        assert_eq!(connector_type("HDMI-A"), ConnectorType::Hdmi);
        assert_eq!(connector_type("eDP"), ConnectorType::Edp);
        assert_eq!(class(&connector_type("eDP")), EndpointClass::Internal);
        assert_eq!(class(&connector_type("Virtual")), EndpointClass::Virtual);
        assert_eq!(class(&connector_type("DP")), EndpointClass::External);
        assert_eq!(kind_for_code(11), Some("HDMI-A"));
        assert!(is_pci_slot("0000:03:00.0"));
        assert!(!is_pci_slot("virtio0"));
    }

    #[test]
    fn lab_fixture() {
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/lab-gnome"
        );
        let root = SystemRoot::fixture(dir).unwrap();
        let (cards, conns) = scan(&root).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].name, "card0");
        assert_eq!(cards[0].pci_vendor, Some(0x1af4));
        assert_eq!(cards[0].driver.as_deref(), Some("virtio-pci"));
        assert_eq!(conns.len(), 1);
        let c = &conns[0];
        assert_eq!(
            (c.name.as_str(), c.kind.as_str(), c.index),
            ("card0-Virtual-1", "Virtual", Some(1))
        );
        assert_eq!(c.status, Some(ConnectionStatus::Connected));
        assert_eq!((c.enabled, c.connector_id), (Some(true), Some(37)));
        assert_eq!(c.modes.first().map(String::as_str), Some("1280x800"));
    }
}
