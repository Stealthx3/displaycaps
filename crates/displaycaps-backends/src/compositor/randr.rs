// SPDX-License-Identifier: LGPL-2.1-or-later
//! The live X11 RandR reader (libxcb through the `xcb` crate). Read-only and passive: it uses
//! `GetScreenResourcesCurrent`, never `GetScreenResources` (which makes the server re-probe every
//! output), reads output properties with `delete` and `pending` off, and sends no request that
//! changes state.

use std::collections::BTreeMap;

use displaycaps_model::UnknownReason;
use xcb::{Xid, randr, x};

use super::facts::{RandrFacts, RandrMode, RandrOutput, Read, to_hex};

/// Output properties worth reading (others are skipped to keep the read small).
const PROPERTIES: &[&str] = &[
    "EDID",
    "max bpc",
    "Colorspace",
    "Broadcast RGB",
    "vrr_capable",
    "link-status",
    "non-desktop",
    "content type",
    "TearFree",
];

/// Reads RandR from the X display `display` (e.g. `:0`); with `outputs` false only the server
/// and RandR versions.
pub(crate) fn read(display: &str, outputs: bool) -> Read<RandrFacts> {
    match read_inner(display, outputs) {
        Ok(f) => Read::Ok(f),
        Err((r, d)) => Read::err(r, d),
    }
}

type Fail = (UnknownReason, String);

fn e(err: impl std::fmt::Display) -> Fail {
    (UnknownReason::ReadError, err.to_string())
}

fn read_inner(display: &str, outputs: bool) -> Result<RandrFacts, Fail> {
    let (conn, screen) =
        xcb::Connection::connect_with_extensions(Some(display), &[xcb::Extension::RandR], &[])
            .map_err(|err| {
                (
                    UnknownReason::ServiceNotRunning,
                    format!("X display {display}: {err}"),
                )
            })?;
    let setup = conn.get_setup();
    let root = setup
        .roots()
        .nth(usize::try_from(screen).unwrap_or(0))
        .ok_or_else(|| e("no X screen"))?
        .root();
    let mut facts = RandrFacts {
        vendor: Some(setup.vendor().to_utf8().into_owned()),
        release: Some(setup.release_number()),
        ..RandrFacts::default()
    };
    let v = conn
        .wait_for_reply(conn.send_request(&randr::QueryVersion {
            major_version: 1,
            minor_version: 6,
        }))
        .map_err(|err| (UnknownReason::NotExposed, format!("RandR: {err}")))?;
    facts.version = [v.major_version(), v.minor_version()];
    if !outputs {
        return Ok(facts);
    }
    let res = conn
        .wait_for_reply(conn.send_request(&randr::GetScreenResourcesCurrent { window: root }))
        .map_err(e)?;
    let ts = res.config_timestamp();
    let modes: BTreeMap<u32, RandrMode> = res.modes().iter().map(|m| (m.id, mode(m))).collect();
    let mut atom_names = BTreeMap::new();
    for &output in res.outputs() {
        let info = conn
            .wait_for_reply(conn.send_request(&randr::GetOutputInfo {
                output,
                config_timestamp: ts,
            }))
            .map_err(e)?;
        let mut o = RandrOutput {
            name: String::from_utf8_lossy(info.name()).into_owned(),
            connection: match info.connection() {
                randr::Connection::Connected => "connected",
                randr::Connection::Disconnected => "disconnected",
                _ => "unknown",
            }
            .into(),
            modes: info
                .modes()
                .iter()
                .filter_map(|m| modes.get(&m.resource_id()).copied())
                .collect(),
            preferred: usize::from(info.num_preferred()),
            ..RandrOutput::default()
        };
        let crtc = info.crtc();
        if !crtc.is_none() {
            let c = conn
                .wait_for_reply(conn.send_request(&randr::GetCrtcInfo {
                    crtc,
                    config_timestamp: ts,
                }))
                .map_err(e)?;
            o.current = modes.get(&c.mode().resource_id()).copied();
        }
        let props = conn
            .wait_for_reply(conn.send_request(&randr::ListOutputProperties { output }))
            .map_err(e)?;
        for &atom in props.atoms() {
            let name = atom_name(&conn, &mut atom_names, atom)?;
            if !PROPERTIES.contains(&name.as_str()) {
                continue;
            }
            let p = conn
                .wait_for_reply(conn.send_request(&randr::GetOutputProperty {
                    output,
                    property: atom,
                    r#type: x::ATOM_ANY,
                    long_offset: 0,
                    long_length: 1024,
                    delete: false,
                    pending: false,
                }))
                .map_err(e)?;
            let text = if name == "EDID" {
                to_hex(p.data::<u8>())
            } else if p.r#type() == x::ATOM_ATOM {
                let mut names = vec![];
                for &a in p.data::<x::Atom>() {
                    names.push(atom_name(&conn, &mut atom_names, a)?);
                }
                names.join(",")
            } else {
                match p.format() {
                    32 => join(p.data::<u32>().iter().map(|v| *v as i32)),
                    16 => join(p.data::<u16>().iter()),
                    _ => join(p.data::<u8>().iter()),
                }
            };
            o.properties.insert(name, text);
        }
        facts.outputs.push(o);
    }
    Ok(facts)
}

fn join<T: ToString>(it: impl Iterator<Item = T>) -> String {
    it.map(|v| v.to_string()).collect::<Vec<_>>().join(",")
}

fn atom_name(
    conn: &xcb::Connection,
    cache: &mut BTreeMap<u32, String>,
    atom: x::Atom,
) -> Result<String, Fail> {
    if let Some(n) = cache.get(&atom.resource_id()) {
        return Ok(n.clone());
    }
    let r = conn
        .wait_for_reply(conn.send_request(&x::GetAtomName { atom }))
        .map_err(e)?;
    let n = r.name().to_utf8().into_owned();
    cache.insert(atom.resource_id(), n.clone());
    Ok(n)
}

fn mode(m: &randr::ModeInfo) -> RandrMode {
    let interlaced = m.mode_flags.contains(randr::ModeFlag::INTERLACE);
    let mut total = u64::from(m.htotal) * u64::from(m.vtotal);
    if m.mode_flags.contains(randr::ModeFlag::DOUBLE_SCAN) {
        total *= 2;
    }
    // Rounded like the kernel's drm_mode_vrefresh, so RandR and KMS agree.
    let mut mhz = (u64::from(m.dot_clock) * 1000 + total / 2)
        .checked_div(total)
        .unwrap_or(0);
    if interlaced {
        mhz *= 2;
    }
    RandrMode {
        width: m.width,
        height: m.height,
        refresh_mhz: u32::try_from(mhz).unwrap_or(u32::MAX),
        interlaced,
    }
}

/// Watches RandR on `display`: asks for this client's screen, output, CRTC and output-property
/// change notifications on the root window (that only sets this connection's event mask) and
/// sends `CompositorChange` for each batch, checking every `poll_every` (libxcb's fd can't be
/// borrowed without unsafe code, so the queue is polled). Stops when `stop` is set or the
/// connection ends.
pub(crate) fn watch(
    display: &str,
    tx: &std::sync::mpsc::Sender<crate::watch::Cause>,
    stop: &std::sync::atomic::AtomicBool,
    poll_every: std::time::Duration,
) {
    use std::sync::atomic::Ordering;
    let Ok((conn, screen)) =
        xcb::Connection::connect_with_extensions(Some(display), &[xcb::Extension::RandR], &[])
    else {
        return;
    };
    let Some(root) = conn
        .get_setup()
        .roots()
        .nth(usize::try_from(screen).unwrap_or(0))
        .map(|s| s.root())
    else {
        return;
    };
    let ck = conn.send_request_checked(&randr::SelectInput {
        window: root,
        enable: randr::NotifyMask::SCREEN_CHANGE
            | randr::NotifyMask::CRTC_CHANGE
            | randr::NotifyMask::OUTPUT_CHANGE
            | randr::NotifyMask::OUTPUT_PROPERTY,
    });
    if conn.check_request(ck).is_err() {
        return;
    }
    while !stop.load(Ordering::SeqCst) {
        let mut changed = false;
        loop {
            match conn.poll_for_event() {
                Ok(Some(_)) => changed = true,
                Ok(None) => break,
                Err(_) => return,
            }
        }
        if changed && tx.send(crate::watch::Cause::CompositorChange).is_err() {
            return;
        }
        std::thread::sleep(poll_every);
    }
}
