// SPDX-License-Identifier: LGPL-2.1-or-later
//! The live Wayland reader: one client connection, read-only. It lists the registry, and when
//! output state is asked for binds `wl_output`, `wp_color_manager_v1`, `zwlr_output_manager_v1`
//! and `kde_output_device_v2` and reads what they send. It never creates a surface, never builds
//! an output configuration and never sends a request that changes state; the only objects it
//! creates are the color-management output and image-description objects needed to read an
//! output's image description, and it destroys them again. Every wait has a deadline.

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use displaycaps_model::UnknownReason;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_callback, wl_output, wl_registry};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child,
};
use wayland_protocols::wp::color_management::v1::client::{
    wp_color_management_output_v1 as cmo, wp_color_manager_v1 as cm,
    wp_image_description_info_v1 as iinfo, wp_image_description_v1 as idesc,
};
use wayland_protocols_plasma::output_device::v2::client::{
    kde_output_device_mode_v2 as kdem, kde_output_device_v2 as kde,
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_head_v1 as wlrh, zwlr_output_manager_v1 as wlrm, zwlr_output_mode_v1 as wlrmode,
};

use super::facts::{
    ColorManager, Global, ImageInfo, KdeDevice, Read, WaylandFacts, WlMode, WlOutput, WlrHead,
    to_hex,
};

/// Highest versions this reader handles.
const WL_OUTPUT: u32 = 4;
const COLOR_MANAGER: u32 = 2;
const WLR_MANAGER: u32 = 4;
const KDE_DEVICE: u32 = 11;

/// Reads the compositor at `socket`. With `outputs` false only the registry is listed.
pub(crate) fn read(socket: &str, outputs: bool, budget: Duration) -> Read<WaylandFacts> {
    let deadline = Instant::now() + budget;
    match connect(socket).and_then(|conn| session(&conn, outputs, deadline)) {
        Ok(f) => Read::Ok(f),
        Err((r, d)) => Read::err(r, d),
    }
}

type Fail = (UnknownReason, String);

fn connect(socket: &str) -> Result<Connection, Fail> {
    let stream = UnixStream::connect(socket).map_err(|e| {
        let reason = match e.kind() {
            std::io::ErrorKind::PermissionDenied => UnknownReason::NotGranted,
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                UnknownReason::ServiceNotRunning
            }
            _ => UnknownReason::ReadError,
        };
        (reason, format!("{socket}: {e}"))
    })?;
    Connection::from_socket(stream).map_err(|e| (UnknownReason::ReadError, e.to_string()))
}

/// The objects bound for output state.
struct Bound {
    wl_outputs: Vec<wl_output::WlOutput>,
    manager: Option<cm::WpColorManagerV1>,
}

/// Lists the registry, then (with `outputs`) binds the output protocols and reads their state.
fn bind(
    conn: &Connection,
    q: &mut EventQueue<State>,
    st: &mut State,
    outputs: bool,
    deadline: Instant,
) -> Result<Bound, Fail> {
    let qh = q.handle();
    let reg = conn.display().get_registry(&qh, ());
    roundtrip(conn, q, st, deadline)?;
    let mut wl_outputs = vec![];
    let mut manager = None;
    if !outputs {
        return Ok(Bound {
            wl_outputs,
            manager,
        });
    }
    for (name, iface, ver) in st.globals.clone() {
        match iface.as_str() {
            "wl_output" => {
                let v = ver.min(WL_OUTPUT);
                let i = st.outputs.len();
                st.outputs.push(WlOutput {
                    version: v,
                    ..WlOutput::default()
                });
                wl_outputs.push(reg.bind::<wl_output::WlOutput, _, _>(name, v, &qh, i));
            }
            "wp_color_manager_v1" if manager.is_none() => {
                let v = ver.min(COLOR_MANAGER);
                st.cm = Some(ColorManager {
                    version: v,
                    ..ColorManager::default()
                });
                manager = Some(reg.bind::<cm::WpColorManagerV1, _, _>(name, v, &qh, ()));
            }
            "zwlr_output_manager_v1" if st.wlr.is_none() => {
                st.wlr = Some(vec![]);
                reg.bind::<wlrm::ZwlrOutputManagerV1, _, _>(name, ver.min(WLR_MANAGER), &qh, ());
            }
            "kde_output_device_v2" => {
                let v = ver.min(KDE_DEVICE);
                let i = st.kde.len();
                st.kde.push(KdeDevice {
                    version: v,
                    ..KdeDevice::default()
                });
                reg.bind::<kde::KdeOutputDeviceV2, _, _>(name, v, &qh, i);
            }
            _ => {}
        }
    }
    roundtrip(conn, q, st, deadline)?;
    Ok(Bound {
        wl_outputs,
        manager,
    })
}

fn session(conn: &Connection, outputs: bool, deadline: Instant) -> Result<WaylandFacts, Fail> {
    let mut q: EventQueue<State> = conn.new_event_queue();
    let qh = q.handle();
    let mut st = State::default();
    let Bound {
        wl_outputs,
        manager,
    } = bind(conn, &mut q, &mut st, outputs, deadline)?;
    let mut facts = WaylandFacts {
        globals: st
            .globals
            .iter()
            .map(|(_, i, v)| Global {
                interface: i.clone(),
                version: *v,
            })
            .collect(),
        ..WaylandFacts::default()
    };
    if !outputs {
        return Ok(facts);
    }

    // Each output's image description: get_output, get_image_description, wait for ready, then
    // get_information. Objects are destroyed once read.
    if let Some(m) = &manager {
        let mut pending = vec![];
        for (i, o) in wl_outputs.iter().enumerate() {
            let cmo = m.get_output(o, &qh, i);
            let d = cmo.get_image_description(&qh, i);
            pending.push((cmo, d));
        }
        roundtrip(conn, &mut q, &mut st, deadline)?;
        for (i, (_, d)) in pending.iter().enumerate() {
            if st.ready.get(&i) == Some(&true) {
                d.get_information(&qh, i);
            }
        }
        roundtrip(conn, &mut q, &mut st, deadline)?;
        for (o, d) in pending {
            d.destroy();
            o.destroy();
        }
        m.destroy();
        let _ = conn.flush();
    }

    facts.outputs = std::mem::take(&mut st.outputs);
    facts.color_manager = st.cm.take();
    facts.wlr_heads = st.wlr.take();
    if !st.kde.is_empty() {
        facts.kde_devices = Some(std::mem::take(&mut st.kde));
    }
    Ok(facts)
}

/// Watches the compositor at `socket`: binds the output protocols (and each output's
/// color-management object, for `image_description_changed`) and sends `cause` whenever any of
/// them reports a change, or an output global comes or goes. Stops when `stop` is set or the
/// connection ends. Read-only, like [`read`].
pub(crate) fn watch(
    socket: &str,
    tx: &std::sync::mpsc::Sender<crate::watch::Cause>,
    stop: &std::sync::atomic::AtomicBool,
    poll_every: Duration,
) {
    use std::sync::atomic::Ordering;
    let Ok(conn) = connect(socket) else { return };
    let mut q: EventQueue<State> = conn.new_event_queue();
    let qh = q.handle();
    let mut st = State::default();
    let Ok(b) = bind(
        &conn,
        &mut q,
        &mut st,
        true,
        Instant::now() + Duration::from_secs(5),
    ) else {
        return;
    };
    let _cm_outputs: Vec<_> = b
        .manager
        .iter()
        .flat_map(|m| {
            b.wl_outputs
                .iter()
                .enumerate()
                .map(|(i, o)| m.get_output(o, &qh, i))
                .collect::<Vec<_>>()
        })
        .collect();
    if roundtrip(
        &conn,
        &mut q,
        &mut st,
        Instant::now() + Duration::from_secs(5),
    )
    .is_err()
    {
        return;
    }
    let mut seen = st.events;
    let ms = u16::try_from(poll_every.as_millis().max(1)).unwrap_or(u16::MAX);
    while !stop.load(Ordering::SeqCst) {
        if q.dispatch_pending(&mut st).is_err() || q.flush().is_err() {
            return;
        }
        if st.events != seen {
            seen = st.events;
            if tx.send(crate::watch::Cause::CompositorChange).is_err() {
                return;
            }
        }
        let Some(guard) = q.prepare_read() else {
            continue;
        };
        let ready = {
            let mut fds = [PollFd::new(guard.connection_fd(), PollFlags::POLLIN)];
            poll(&mut fds, PollTimeout::from(ms)).unwrap_or(0)
        };
        if ready > 0 {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return,
            }
        }
    }
}

/// `wl_display.sync`, then dispatch until its callback fires or the deadline passes.
fn roundtrip(
    conn: &Connection,
    q: &mut EventQueue<State>,
    st: &mut State,
    deadline: Instant,
) -> Result<(), Fail> {
    let err = |e: &dyn std::fmt::Display| (UnknownReason::ReadError, e.to_string());
    st.seq += 1;
    let seq = st.seq;
    conn.display().sync(&q.handle(), seq);
    loop {
        q.dispatch_pending(st).map_err(|e| err(&e))?;
        if st.synced >= seq {
            return Ok(());
        }
        q.flush().map_err(|e| err(&e))?;
        let Some(guard) = q.prepare_read() else {
            continue;
        };
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err((
                UnknownReason::Timeout,
                "the compositor did not answer".into(),
            ));
        }
        let ms = u16::try_from(left.as_millis().max(1)).unwrap_or(u16::MAX);
        let ready = {
            let mut fds = [PollFd::new(guard.connection_fd(), PollFlags::POLLIN)];
            poll(&mut fds, PollTimeout::from(ms)).map_err(|e| err(&e))?
        };
        if ready == 0 {
            return Err((
                UnknownReason::Timeout,
                "the compositor did not answer".into(),
            ));
        }
        match guard.read() {
            Ok(_) => {}
            Err(wayland_client::backend::WaylandError::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(err(&e)),
        }
    }
}

#[derive(Default)]
struct State {
    /// Events from output objects and the registry (a watch compares this count).
    events: u64,
    seq: u32,
    synced: u32,
    globals: Vec<(u32, String, u32)>,
    outputs: Vec<WlOutput>,
    cm: Option<ColorManager>,
    /// Image description readiness per output index.
    ready: HashMap<usize, bool>,
    /// Image information being collected per output index.
    info: HashMap<usize, ImageInfo>,
    wlr: Option<Vec<WlrHead>>,
    heads: HashMap<ObjectId, usize>,
    wlr_modes: HashMap<ObjectId, (usize, usize)>,
    kde: Vec<KdeDevice>,
    kde_modes: HashMap<ObjectId, (usize, usize)>,
}

fn raw<T: Into<u32>>(w: WEnum<T>) -> u32 {
    match w {
        WEnum::Value(v) => v.into(),
        WEnum::Unknown(u) => u,
    }
}

fn named(table: &[&str], v: u32) -> String {
    table
        .get(v as usize)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("unknown_{v}"))
}

/// `wp_color_manager_v1.transfer_function` entry names by value.
pub(crate) const TF: &[&str] = &[
    "",
    "bt1886",
    "gamma22",
    "gamma28",
    "st240",
    "ext_linear",
    "log_100",
    "log_316",
    "xvycc",
    "srgb",
    "ext_srgb",
    "st2084_pq",
    "st428",
    "hlg",
    "compound_power_2_4",
];
/// `wp_color_manager_v1.primaries` entry names by value.
pub(crate) const PRIMARIES: &[&str] = &[
    "",
    "srgb",
    "pal_m",
    "pal",
    "ntsc",
    "generic_film",
    "bt2020",
    "cie1931_xyz",
    "dci_p3",
    "display_p3",
    "adobe_rgb",
];
const FEATURES: &[&str] = &[
    "icc_v2_v4",
    "parametric",
    "set_primaries",
    "set_tf_power",
    "set_luminances",
    "set_mastering_display_primaries",
    "extended_target_volume",
    "windows_scrgb",
    "windows_bt2100",
];
const INTENTS: &[&str] = &[
    "perceptual",
    "relative",
    "saturation",
    "absolute",
    "relative_bpc",
    "absolute_no_adaptation",
];

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        st: &mut Self,
        _: &wl_registry::WlRegistry,
        ev: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        st.events += 1;
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = ev
        {
            st.globals.push((name, interface, version));
        }
    }
}

impl Dispatch<wl_callback::WlCallback, u32> for State {
    fn event(
        st: &mut Self,
        _: &wl_callback::WlCallback,
        ev: wl_callback::Event,
        seq: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = ev {
            st.synced = st.synced.max(*seq);
        }
    }
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(
        st: &mut Self,
        _: &wl_output::WlOutput,
        ev: wl_output::Event,
        i: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        st.events += 1;
        let Some(o) = st.outputs.get_mut(*i) else {
            return;
        };
        match ev {
            wl_output::Event::Geometry { make, model, .. } => {
                o.make = Some(make);
                o.model = Some(model);
            }
            wl_output::Event::Mode {
                flags,
                width,
                height,
                refresh,
            } => {
                let f = match flags {
                    WEnum::Value(f) => f,
                    WEnum::Unknown(u) => wl_output::Mode::from_bits_truncate(u),
                };
                if f.contains(wl_output::Mode::Current) || o.mode.is_none() {
                    o.mode = Some(WlMode {
                        width,
                        height,
                        refresh_mhz: refresh,
                        preferred: f.contains(wl_output::Mode::Preferred),
                    });
                }
            }
            wl_output::Event::Scale { factor } => o.scale = Some(factor),
            wl_output::Event::Name { name } => o.name = Some(name),
            wl_output::Event::Description { description } => o.description = Some(description),
            _ => {}
        }
    }
}

impl Dispatch<cm::WpColorManagerV1, ()> for State {
    fn event(
        st: &mut Self,
        _: &cm::WpColorManagerV1,
        ev: cm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        st.events += 1;
        let Some(c) = &mut st.cm else { return };
        match ev {
            cm::Event::SupportedIntent { render_intent } => {
                c.intents.push(named(INTENTS, raw(render_intent)))
            }
            cm::Event::SupportedFeature { feature } => {
                c.features.push(named(FEATURES, raw(feature)))
            }
            cm::Event::SupportedTfNamed { tf } => c.transfer_functions.push(named(TF, raw(tf))),
            cm::Event::SupportedPrimariesNamed { primaries } => {
                c.primaries.push(named(PRIMARIES, raw(primaries)))
            }
            _ => {}
        }
    }
}

impl Dispatch<cmo::WpColorManagementOutputV1, usize> for State {
    fn event(
        st: &mut Self,
        _: &cmo::WpColorManagementOutputV1,
        _: cmo::Event,
        _: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `image_description_changed`: the probe reads one state; a watch re-probes.
        st.events += 1;
    }
}

impl Dispatch<idesc::WpImageDescriptionV1, usize> for State {
    fn event(
        st: &mut Self,
        _: &idesc::WpImageDescriptionV1,
        ev: idesc::Event,
        i: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        st.events += 1;
        match ev {
            idesc::Event::Ready { .. } | idesc::Event::Ready2 { .. } => {
                st.ready.insert(*i, true);
            }
            idesc::Event::Failed { cause, msg } => {
                st.ready.insert(*i, false);
                if let Some(o) = st.outputs.get_mut(*i) {
                    o.image_description = Some(Read::err(
                        UnknownReason::NotExposed,
                        format!("image description failed (cause {}): {msg}", raw(cause)),
                    ));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<iinfo::WpImageDescriptionInfoV1, usize> for State {
    fn event(
        st: &mut Self,
        _: &iinfo::WpImageDescriptionInfoV1,
        ev: iinfo::Event,
        i: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        st.events += 1;
        if let iinfo::Event::Done = ev {
            let info = st.info.remove(i).unwrap_or_default();
            if let Some(o) = st.outputs.get_mut(*i) {
                o.image_description = Some(Read::Ok(info));
            }
            return;
        }
        let info = st.info.entry(*i).or_default();
        match ev {
            iinfo::Event::IccFile { .. } => info.icc = true, // the fd is closed on drop
            iinfo::Event::Primaries {
                r_x,
                r_y,
                g_x,
                g_y,
                b_x,
                b_y,
                w_x,
                w_y,
            } => info.primaries = Some([r_x, r_y, g_x, g_y, b_x, b_y, w_x, w_y]),
            iinfo::Event::PrimariesNamed { primaries } => {
                info.primaries_named = Some(named(PRIMARIES, raw(primaries)))
            }
            iinfo::Event::TfPower { eexp } => info.tf_power = Some(eexp),
            iinfo::Event::TfNamed { tf } => info.tf_named = Some(named(TF, raw(tf))),
            iinfo::Event::Luminances {
                min_lum,
                max_lum,
                reference_lum,
            } => info.luminances = Some([min_lum, max_lum, reference_lum]),
            iinfo::Event::TargetPrimaries {
                r_x,
                r_y,
                g_x,
                g_y,
                b_x,
                b_y,
                w_x,
                w_y,
            } => info.target_primaries = Some([r_x, r_y, g_x, g_y, b_x, b_y, w_x, w_y]),
            iinfo::Event::TargetLuminance { min_lum, max_lum } => {
                info.target_luminance = Some([min_lum, max_lum])
            }
            iinfo::Event::TargetMaxCll { max_cll } => info.target_max_cll = Some(max_cll),
            iinfo::Event::TargetMaxFall { max_fall } => info.target_max_fall = Some(max_fall),
            _ => {}
        }
    }
}

impl Dispatch<wlrm::ZwlrOutputManagerV1, ()> for State {
    fn event(
        st: &mut Self,
        _: &wlrm::ZwlrOutputManagerV1,
        ev: wlrm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        st.events += 1;
        if let wlrm::Event::Head { head } = ev {
            let heads = st.wlr.get_or_insert_with(Vec::new);
            st.heads.insert(head.id(), heads.len());
            heads.push(WlrHead::default());
        }
    }

    event_created_child!(State, wlrm::ZwlrOutputManagerV1, [
        wlrm::EVT_HEAD_OPCODE => (wlrh::ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<wlrh::ZwlrOutputHeadV1, ()> for State {
    fn event(
        st: &mut Self,
        head: &wlrh::ZwlrOutputHeadV1,
        ev: wlrh::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        st.events += 1;
        let Some(&hi) = st.heads.get(&head.id()) else {
            return;
        };
        let Some(h) = st.wlr.as_mut().and_then(|v| v.get_mut(hi)) else {
            return;
        };
        match ev {
            wlrh::Event::Name { name } => h.name = name,
            wlrh::Event::Description { description } => h.description = Some(description),
            wlrh::Event::Make { make } => h.make = Some(make),
            wlrh::Event::Model { model } => h.model = Some(model),
            wlrh::Event::SerialNumber { serial_number } => h.serial = Some(serial_number),
            wlrh::Event::Enabled { enabled } => h.enabled = enabled != 0,
            wlrh::Event::Scale { scale } => h.scale = Some(scale),
            wlrh::Event::Mode { mode } => {
                st.wlr_modes.insert(mode.id(), (hi, h.modes.len()));
                h.modes.push(WlMode::default());
            }
            wlrh::Event::CurrentMode { mode } => {
                h.current_mode = st.wlr_modes.get(&mode.id()).map(|&(_, m)| m);
            }
            wlrh::Event::AdaptiveSync { state } => {
                h.adaptive_sync = Some(raw(state) == 1);
            }
            _ => {}
        }
    }

    event_created_child!(State, wlrh::ZwlrOutputHeadV1, [
        wlrh::EVT_MODE_OPCODE => (wlrmode::ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<wlrmode::ZwlrOutputModeV1, ()> for State {
    fn event(
        st: &mut Self,
        mode: &wlrmode::ZwlrOutputModeV1,
        ev: wlrmode::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        st.events += 1;
        let Some(&(h, m)) = st.wlr_modes.get(&mode.id()) else {
            return;
        };
        let Some(md) = st
            .wlr
            .as_mut()
            .and_then(|v| v.get_mut(h))
            .and_then(|h| h.modes.get_mut(m))
        else {
            return;
        };
        match ev {
            wlrmode::Event::Size { width, height } => {
                md.width = width;
                md.height = height;
            }
            wlrmode::Event::Refresh { refresh } => md.refresh_mhz = refresh,
            wlrmode::Event::Preferred => md.preferred = true,
            _ => {}
        }
    }
}

impl Dispatch<kde::KdeOutputDeviceV2, usize> for State {
    fn event(
        st: &mut Self,
        _: &kde::KdeOutputDeviceV2,
        ev: kde::Event,
        i: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        st.events += 1;
        let i = *i;
        let Some(d) = st.kde.get_mut(i) else { return };
        match ev {
            kde::Event::Geometry { make, model, .. } => {
                d.make = Some(make);
                d.model = Some(model);
            }
            kde::Event::Mode { mode } => {
                st.kde_modes.insert(mode.id(), (i, d.modes.len()));
                d.modes.push(WlMode::default());
            }
            kde::Event::CurrentMode { mode } => {
                d.current_mode = st.kde_modes.get(&mode.id()).map(|&(_, m)| m);
            }
            kde::Event::Scale { factor } => d.scale = Some(factor),
            kde::Event::Edid { raw } => {
                d.edid_hex = super::facts::from_base64(&raw).map(|b| to_hex(&b));
            }
            kde::Event::Enabled { enabled } => d.enabled = Some(enabled != 0),
            kde::Event::Capabilities { flags } => {
                d.capabilities = Some(match flags {
                    WEnum::Value(f) => f.bits(),
                    WEnum::Unknown(u) => u,
                })
            }
            kde::Event::VrrPolicy { vrr_policy } => d.vrr_policy = Some(raw(vrr_policy)),
            kde::Event::RgbRange { rgb_range } => d.rgb_range = Some(raw(rgb_range)),
            kde::Event::Name { name } => d.name = Some(name),
            kde::Event::HighDynamicRange { hdr_enabled } => d.hdr = Some(hdr_enabled != 0),
            kde::Event::SdrBrightness { sdr_brightness } => d.sdr_brightness = Some(sdr_brightness),
            kde::Event::WideColorGamut { wcg_enabled } => d.wide_color = Some(wcg_enabled != 0),
            kde::Event::BrightnessMetadata {
                max_peak_brightness,
                max_frame_average_brightness,
                min_brightness,
            } => {
                d.brightness_metadata = Some([
                    max_peak_brightness,
                    max_frame_average_brightness,
                    min_brightness,
                ])
            }
            kde::Event::BrightnessOverrides {
                max_peak_brightness,
                max_average_brightness,
                min_brightness,
            } => {
                d.brightness_overrides =
                    Some([max_peak_brightness, max_average_brightness, min_brightness])
            }
            kde::Event::ColorProfileSource { source } => d.color_profile_source = Some(raw(source)),
            _ => {}
        }
    }

    event_created_child!(State, kde::KdeOutputDeviceV2, [
        kde::EVT_MODE_OPCODE => (kdem::KdeOutputDeviceModeV2, ()),
    ]);
}

impl Dispatch<kdem::KdeOutputDeviceModeV2, ()> for State {
    fn event(
        st: &mut Self,
        mode: &kdem::KdeOutputDeviceModeV2,
        ev: kdem::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        st.events += 1;
        let Some(&(d, m)) = st.kde_modes.get(&mode.id()) else {
            return;
        };
        let Some(md) = st.kde.get_mut(d).and_then(|d| d.modes.get_mut(m)) else {
            return;
        };
        match ev {
            kdem::Event::Size { width, height } => {
                md.width = width;
                md.height = height;
            }
            kdem::Event::Refresh { refresh } => md.refresh_mhz = refresh,
            kdem::Event::Preferred => md.preferred = true,
            _ => {}
        }
    }
}
