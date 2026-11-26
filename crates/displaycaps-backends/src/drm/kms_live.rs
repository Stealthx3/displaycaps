// SPDX-License-Identifier: LGPL-2.1-or-later
//! Live KMS reads through the `drm` crate: read-only ioctls only.
//!
//! - Connectors are read with `GETCONNECTOR` and a one-mode stub (the `drm` crate's
//!   `force_probe = false`, the same request as libdrm's `drmModeGetConnectorCurrent`): the
//!   kernel returns its cached state and never probes the connector.
//! - Nothing is set: no mode, no property, no master. The only per-file change is the client
//!   capabilities (universal planes, atomic), which make atomic-only properties visible to this
//!   file descriptor and affect nothing else.
//! - The primary node is opened only inside a display session, where the compositor is DRM master
//!   already. Outside one the kernel would make this process master implicitly, so the caller
//!   doesn't open it at all (see `drm::kms_readable`).

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};

use drm::control::{Device as ControlDevice, ResourceHandle, property};
use drm::{ClientCapability, Device};

use super::kms::{BLOBS_READ, KmsCard, KmsConnector, KmsCrtc, KmsFrom, KmsMode, KmsProp};

/// An open primary node.
struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl Device for Card {}
impl ControlDevice for Card {}

/// Reads one card (`/dev/dri/cardN`). `blobs` names the blob properties whose bytes are read
/// (EDID only when the sink is in scope; HDR metadata only when HDR is).
pub(crate) fn read_card(name: &str, blobs: &[&str]) -> io::Result<KmsCard> {
    let file = OpenOptions::new()
        .read(true)
        .open(format!("/dev/dri/{name}"))?;
    let card = Card(file);
    // Visibility only; failures leave the legacy property set, which is still read.
    let _ = card.set_client_capability(ClientCapability::UniversalPlanes, true);
    let _ = card.set_client_capability(ClientCapability::Atomic, true);
    let driver = card
        .get_driver()
        .map(|d| d.name().to_string_lossy().into_owned())
        .unwrap_or_default();
    let res = card.resource_handles()?;
    let mut connectors = vec![];
    for &h in res.connectors() {
        let Ok(info) = card.get_connector(h, false) else {
            continue;
        };
        let crtc = info
            .current_encoder()
            .and_then(|e| card.get_encoder(e).ok())
            .and_then(|e| e.crtc())
            .map(u32::from);
        connectors.push(KmsConnector {
            id: u32::from(h),
            type_code: u32::from(info.interface()),
            status: u32::from(info.state()),
            crtc,
            modes: info.modes().iter().map(mode).collect(),
            props: props(&card, h, blobs),
        });
    }
    let mut crtcs = vec![];
    for &h in res.crtcs() {
        let Ok(info) = card.get_crtc(h) else { continue };
        crtcs.push(KmsCrtc {
            id: u32::from(h),
            mode: info.mode().as_ref().map(mode),
            props: props(&card, h, &[]),
        });
    }
    Ok(KmsCard {
        card: name.to_string(),
        driver,
        connectors,
        crtcs,
        from: KmsFrom::Live,
    })
}

fn mode(m: &drm::control::Mode) -> KmsMode {
    let (hdisplay, vdisplay) = m.size();
    let (hsync_start, hsync_end, htotal) = m.hsync();
    let (vsync_start, vsync_end, vtotal) = m.vsync();
    KmsMode {
        clock_khz: m.clock(),
        hdisplay,
        hsync_start,
        hsync_end,
        htotal,
        vdisplay,
        vsync_start,
        vsync_end,
        vtotal,
        vscan: m.vscan(),
        flags: m.flags().bits(),
        mode_type: m.mode_type().bits(),
    }
}

fn props<T: ResourceHandle>(card: &Card, h: T, blobs: &[&str]) -> Vec<KmsProp> {
    let Ok(set) = card.get_properties(h) else {
        return vec![];
    };
    let mut out = vec![];
    for (&ph, &raw) in set.iter() {
        let Ok(info) = card.get_property(ph) else {
            continue;
        };
        let name = info.name().to_string_lossy().into_owned();
        let enum_name = match info.value_type() {
            property::ValueType::Enum(values) => values
                .get_value_from_raw_value(raw)
                .map(|v| v.name().to_string_lossy().into_owned()),
            _ => None,
        };
        let blob = (matches!(info.value_type(), property::ValueType::Blob)
            && raw != 0
            && blobs.contains(&name.as_str())
            && BLOBS_READ.contains(&name.as_str()))
        .then(|| card.get_property_blob(raw).ok())
        .flatten();
        out.push(KmsProp {
            name,
            raw,
            enum_name,
            blob,
            hdr: None,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
