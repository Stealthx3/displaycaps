// SPDX-License-Identifier: LGPL-2.1-or-later
//! Live KMS reads: read-only ioctls only, through `drm-ffi` and the `drm` crate.
//!
//! - Connectors are read with `GETCONNECTOR` and a mode buffer of at least one entry on every
//!   call, so `count_modes` is never 0: the kernel returns its cached state and never probes the
//!   connector (libdrm's `drmModeGetConnectorCurrent`). The `drm` crate's own `get_connector`
//!   isn't used: its retry loop sends `count_modes = 0` for a connector with no cached modes.
//! - Nothing is set: no mode, no property, no master. The only per-file change is the client
//!   capabilities (universal planes, atomic, aspect ratio), which make atomic-only properties and
//!   the modes' picture aspect ratio visible to this file descriptor and affect nothing else.
//!   Stereo 3D and writeback connectors are left off: neither is a display the library reports.
//! - Only the connectors the plan names are read, with the CRTCs that drive them. Which cards may
//!   be opened at all is decided before (see `drm::kms_access` and `drm::kms_plan`).

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};

use drm::control::{Device as ControlDevice, ResourceHandle, property};
use drm::{ClientCapability, Device};
use drm_ffi::drm_mode_modeinfo;

use super::CardPlan;
use super::kms::{KmsCard, KmsConnector, KmsCrtc, KmsFrom, KmsMode, KmsProp};

/// An open primary node.
struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl Device for Card {}
impl ControlDevice for Card {}

/// Upper bound of the GETCONNECTOR retry loop (the mode list changing between calls).
const TRIES: usize = 4;

type ConnectorInfo = (drm_ffi::drm_mode_get_connector, Vec<drm_mode_modeinfo>);

/// `GETCONNECTOR` that never probes: `count_modes` is at least 1 on every call.
fn connector(card: &Card, id: u32) -> io::Result<ConnectorInfo> {
    let mut want = 1usize;
    for _ in 0..TRIES {
        // drm-ffi sends each buffer's capacity as its count (after reserving the counts of its
        // own sizing call), so the mode count it sends is never below this capacity, never 0.
        let mut modes: Vec<drm_mode_modeinfo> = Vec::with_capacity(want.max(1));
        let mut encoders: Vec<u32> = Vec::with_capacity(8);
        let info = drm_ffi::mode::get_connector(
            card.as_fd(),
            id,
            None,
            None,
            Some(&mut modes),
            Some(&mut encoders),
            false,
        )?;
        if info.count_modes as usize == modes.len() {
            return Ok((info, modes));
        }
        want = info.count_modes as usize;
    }
    Err(io::Error::other("mode list kept changing"))
}

/// Reads the plan's connectors on one card (`/dev/dri/cardN`).
pub(crate) fn read_card(plan: &CardPlan) -> io::Result<KmsCard> {
    let file = OpenOptions::new()
        .read(true)
        .open(format!("/dev/dri/{}", plan.card))?;
    let card = Card(file);
    // Visibility only; failures leave the legacy property set, which is still read.
    let _ = card.set_client_capability(ClientCapability::UniversalPlanes, true);
    let _ = card.set_client_capability(ClientCapability::Atomic, true);
    let _ = card.set_client_capability(ClientCapability::AspectRatio, true);
    let driver = card
        .get_driver()
        .map(|d| d.name().to_string_lossy().into_owned())
        .unwrap_or_default();
    let res = card.resource_handles()?;
    let mut connectors = vec![];
    let mut crtc_ids = vec![];
    for &h in res.connectors() {
        let id = u32::from(h);
        if plan
            .connectors
            .as_ref()
            .is_some_and(|ids| !ids.contains(&id))
        {
            continue;
        }
        let Ok((info, modes)) = connector(&card, id) else {
            continue;
        };
        let crtc = drm::control::from_u32::<drm::control::encoder::Handle>(info.encoder_id)
            .and_then(|e| card.get_encoder(e).ok())
            .and_then(|e| e.crtc())
            .map(u32::from);
        if let Some(c) = crtc {
            crtc_ids.push(c);
        }
        let mut blobs = vec![];
        if plan.hdr {
            blobs.push("HDR_OUTPUT_METADATA");
        }
        if plan.edid_for.contains(&id) {
            blobs.push("EDID");
        }
        connectors.push(KmsConnector {
            id,
            type_code: info.connector_type,
            status: info.connection,
            crtc,
            modes: modes.iter().map(mode).collect(),
            props: props(&card, h, &blobs),
        });
    }
    let mut crtcs = vec![];
    for &h in res.crtcs() {
        if !crtc_ids.contains(&u32::from(h)) {
            continue;
        }
        let Ok(info) = card.get_crtc(h) else { continue };
        crtcs.push(KmsCrtc {
            id: u32::from(h),
            mode: info.mode().map(|m| mode(&drm_mode_modeinfo::from(m))),
            props: props(&card, h, &[]),
        });
    }
    Ok(KmsCard {
        card: plan.card.clone(),
        driver,
        connectors,
        crtcs,
        from: KmsFrom::Live,
    })
}

fn mode(r: &drm_mode_modeinfo) -> KmsMode {
    // The raw struct: the `drm` crate's flag types drop bits they don't name (picture aspect
    // ratio among them).
    KmsMode {
        clock_khz: r.clock,
        hdisplay: r.hdisplay,
        hsync_start: r.hsync_start,
        hsync_end: r.hsync_end,
        htotal: r.htotal,
        vdisplay: r.vdisplay,
        vsync_start: r.vsync_start,
        vsync_end: r.vsync_end,
        vtotal: r.vtotal,
        vscan: r.vscan,
        flags: r.flags,
        mode_type: r.type_,
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
            && blobs.contains(&name.as_str()))
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
