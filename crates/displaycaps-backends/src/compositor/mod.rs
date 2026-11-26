// SPDX-License-Identifier: LGPL-2.1-or-later
//! The compositor layer: what the session's compositor does with each output (enabled, mode,
//! scale, HDR and wide-colour switches, the output's image description, VRR policy, RGB range).
//!
//! Sources, read only when the compositor layer (or, for the compositor's name, the environment
//! layer) is in scope, and only those the session has:
//! - **Wayland** ([`wayland`]): the registry; `wl_output`; `wp_color_manager_v1` output image
//!   descriptions; `zwlr_output_manager_v1` heads (wlroots); `kde_output_device_v2` (KWin).
//! - **Mutter** ([`mutter`]): `org.gnome.Mutter.DisplayConfig.GetCurrentState` on the session bus,
//!   when the Wayland registry shows Mutter, or in an X11 session whose desktop is GNOME.
//! - **X11 RandR** ([`randr`]), in an X11 session.
//!
//! Each output is matched to its DRM connector by name (`HDMI-A-1`; Mutter's and the modesetting
//! driver's `HDMI-1` is the same connector), with EDID identity to settle ties; an output that
//! matches no connector becomes an endpoint of its own (`compositor-<name>`). A fixture replays
//! the facts a capture saved (`commands/displaycaps-compositor.json`).

pub mod facts;
mod mutter;
mod randr;
mod wayland;

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::Duration;

use displaycaps_model::{
    Availability, CompositorInfo, CompositorKind, CompositorOutput, Confidence, ConnectionStatus,
    ConnectorType, CorrelationId, CorrelationKey, Endpoint, EndpointClass, EndpointId, Eotf,
    FieldGroup, HdrCapabilities, ImageDescription, LayerCapabilities, LayerDetail, LayerKind, Mode,
    ModeSource, ProbeScope, Provenance, Sensitive, SensitiveKind, SessionKind, Source,
    UnknownReason, Value, VrrPolicy, Ycbcr420,
};

use crate::environment::{self, Env};
use crate::root::SystemRoot;
use facts::{CompositorFacts, ImageInfo, KdeDevice, MutterFacts, RandrFacts, Read, WaylandFacts};

/// The captured command name that holds a fixture's compositor facts.
pub const CAPTURED: &str = "displaycaps-compositor";

/// What the compositor backend needs to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Want {
    /// Not in scope.
    Nothing,
    /// Only the compositor's identity (the environment layer).
    Identity,
    /// Output state (the compositor layer).
    Outputs,
}

/// What the scope needs from the compositor.
pub(crate) fn want(scope: &ProbeScope) -> Want {
    if scope.layers.includes(&LayerKind::Compositor) {
        Want::Outputs
    } else if scope.layers.includes(&LayerKind::Environment) {
        Want::Identity
    } else {
        Want::Nothing
    }
}

/// Where the live sources are, worked out from the probe's environment.
#[derive(Debug, Clone, Default)]
pub(crate) struct LivePlan {
    wayland: Option<String>,
    x11: Option<String>,
    bus: Option<String>,
    gnome_x11: bool,
    outputs: bool,
    /// Inside Flatpak (Mutter's name may be hidden by the bus proxy).
    flatpak: bool,
    /// Read image descriptions (HDR or compositor fields in scope).
    images: bool,
    /// Read RandR's output settings properties (compositor fields in scope).
    props: bool,
}

impl LivePlan {
    /// Reads only what the scope's field groups need.
    pub(crate) fn limit(&mut self, scope: &ProbeScope) {
        let f = |g: FieldGroup| scope.fields.includes(&g);
        self.images = f(FieldGroup::Compositor) || f(FieldGroup::Hdr);
        self.props = f(FieldGroup::Compositor);
    }
}

/// The result: facts, or why there are none.
// Built once per probe and moved, never stored in bulk.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub(crate) enum CompRead {
    /// Facts from the sources that were read.
    Facts(CompositorFacts),
    /// Nothing could be read.
    Unreadable(UnknownReason, String),
}

/// How the backend gets its facts.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Access {
    /// Replay a fixture's facts.
    Fixture(CompRead),
    /// Read live.
    Live(LivePlan),
}

/// Plans the read for `root` and `env`.
pub(crate) fn access(root: &SystemRoot, env: &Env, want: Want) -> Access {
    match root {
        SystemRoot::Fixture(_) => Access::Fixture(match root.captured_command(CAPTURED) {
            Some((_, Some(bytes))) => match serde_json::from_slice::<CompositorFacts>(&bytes) {
                Ok(f) => CompRead::Facts(f),
                Err(e) => CompRead::Unreadable(UnknownReason::ParseError, e.to_string()),
            },
            _ => CompRead::Unreadable(
                UnknownReason::NotExposed,
                "the capture has no compositor facts".into(),
            ),
        }),
        SystemRoot::Live => {
            let kind = environment::session_kind(env);
            let mut p = LivePlan {
                outputs: want == Want::Outputs,
                bus: env.get("DBUS_SESSION_BUS_ADDRESS"),
                flatpak: root.exists("/.flatpak-info"),
                images: true,
                props: true,
                ..LivePlan::default()
            };
            match kind {
                SessionKind::Wayland => p.wayland = crate::sandbox::wayland_socket(env),
                SessionKind::X11 => {
                    p.x11 = env.get("DISPLAY");
                    p.gnome_x11 = env
                        .get("XDG_CURRENT_DESKTOP")
                        .is_some_and(|d| d.split(':').any(|s| s.eq_ignore_ascii_case("gnome")));
                }
                SessionKind::None => {}
            }
            if p.wayland.is_none() && p.x11.is_none() {
                return Access::Fixture(CompRead::Unreadable(
                    UnknownReason::NoSession,
                    match kind {
                        SessionKind::None => "no Wayland or X11 session in the environment",
                        _ => "the session's display socket can't be worked out",
                    }
                    .into(),
                ));
            }
            Access::Live(p)
        }
    }
}

/// Reads the live sources on a helper thread (each wait has its own deadline inside `budget`);
/// the backend task polls the returned channel.
pub(crate) fn start_live(plan: LivePlan, budget: Duration) -> mpsc::Receiver<CompRead> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("displaycaps-compositor".into())
        .spawn(move || {
            let _ = tx.send(read_live(&plan, budget));
        });
    if spawned.is_err() {
        let (tx, rx2) = mpsc::channel();
        let _ = tx.send(CompRead::Unreadable(
            UnknownReason::ReadError,
            "could not start the compositor reader".into(),
        ));
        return rx2;
    }
    rx
}

/// Reads the live sources in order: Wayland, then Mutter when the compositor is Mutter, or RandR
/// (and Mutter on GNOME) on X11.
pub(crate) fn read_live(plan: &LivePlan, budget: Duration) -> CompRead {
    // One deadline for every source, so a slow first source leaves the rest less time rather
    // than pushing the whole read past the backend's deadline.
    let deadline = std::time::Instant::now() + budget;
    let left = || deadline.saturating_duration_since(std::time::Instant::now());
    let mut f = CompositorFacts::default();
    let mutter = |f: &mut CompositorFacts| {
        let t = left();
        f.mutter = Some(if t.is_zero() {
            Read::err(UnknownReason::Timeout, "no time left for Mutter")
        } else if plan.outputs {
            mutter::read(plan.bus.as_deref(), t, plan.flatpak)
        } else {
            mutter::present(plan.bus.as_deref(), t, plan.flatpak)
        });
    };
    if let Some(sock) = &plan.wayland {
        let w = wayland::read(sock, plan.outputs, plan.images, left());
        let is_mutter = w.ok().is_some_and(|w| kind_of(w) == CompositorKind::Mutter);
        f.wayland = Some(w);
        // The registry already names Mutter; its state is needed only for output state.
        if is_mutter && plan.outputs {
            mutter(&mut f);
        }
    }
    if let Some(d) = &plan.x11 {
        f.randr = Some(randr::read(d, plan.outputs, plan.props));
        // On X11 only Mutter itself can say it runs, in both scopes alike.
        if plan.gnome_x11 {
            mutter(&mut f);
        }
    }
    CompRead::Facts(f)
}

/// Starts the compositor change source for a watch of `scope` (live system, compositor layer in
/// scope, a session in the environment): the Wayland connection's output events, or RandR's
/// change notifications on X11. Mutter's own settings (e.g. its HDR colour mode) change the
/// output's image description, which arrives over Wayland.
pub(crate) fn watch_source(
    env: &Env,
    tx: mpsc::Sender<crate::watch::Cause>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    poll_every: Duration,
) -> Option<std::thread::JoinHandle<()>> {
    let Access::Live(plan) = access(&SystemRoot::Live, env, Want::Outputs) else {
        return None;
    };
    std::thread::Builder::new()
        .name("displaycaps-compositor-watch".into())
        .spawn(move || {
            if let Some(sock) = &plan.wayland {
                wayland::watch(sock, &tx, &stop, poll_every);
            } else if let Some(d) = &plan.x11 {
                randr::watch(d, &tx, &stop, poll_every);
            }
        })
        .ok()
}

/// The live facts with serials redacted, for `displaycaps compositor-facts` and captures. Uses
/// the process environment.
pub fn live_facts(budget: Duration) -> Result<CompositorFacts, String> {
    let env = Env::Process;
    match access(&SystemRoot::Live, &env, Want::Outputs) {
        Access::Live(p) => match read_live(&p, budget) {
            CompRead::Facts(mut f) => {
                f.redact();
                Ok(f)
            }
            CompRead::Unreadable(r, d) => Err(format!("{r:?}: {d}")),
        },
        Access::Fixture(CompRead::Unreadable(r, d)) => Err(format!("{r:?}: {d}")),
        Access::Fixture(CompRead::Facts(f)) => Ok(f),
    }
}

// ----- identity --------------------------------------------------------------------------------

fn kind_of(w: &WaylandFacts) -> CompositorKind {
    let has = |p: &str| w.globals.iter().any(|g| g.interface.starts_with(p));
    if has("hyprland_") {
        CompositorKind::Hyprland
    } else if has("gamescope_") {
        CompositorKind::Gamescope
    } else if has("weston_") {
        CompositorKind::Weston
    } else if has("kde_output_device_v2") || has("org_kde_plasma_shell") {
        CompositorKind::Kwin
    } else if has("gtk_shell1") {
        CompositorKind::Mutter
    } else if has("zwlr_layer_shell_v1") || has("zwlr_output_manager_v1") {
        CompositorKind::Wlroots
    } else {
        CompositorKind::Other
    }
}

/// Globals worth naming in the session's protocol list.
const PROTOCOLS: &[&str] = &[
    "wl_output",
    "wp_color_manager_v1",
    "wp_color_representation_manager_v1",
    "frog_color_management_factory_v1",
    "xx_color_manager_v4",
    "zwlr_output_manager_v1",
    "kde_output_device_v2",
    "kde_output_management_v2",
    "wp_presentation",
    "wp_tearing_control_manager_v1",
    "wp_content_type_manager_v1",
    "wp_fifo_manager_v1",
    "wp_commit_timing_manager_v1",
    "zwp_linux_dmabuf_v1",
    "wp_drm_lease_device_v1",
];

/// The compositor and the protocols it offers, from whatever was read.
pub(crate) fn identity(f: &CompositorFacts) -> Option<(CompositorInfo, Vec<String>)> {
    let mut protocols = vec![];
    let mut info = None;
    if let Some(Read::Ok(w)) = &f.wayland {
        let kind = kind_of(w);
        info = Some(CompositorInfo {
            kind,
            name: name_of(kind).into(),
            version: None,
        });
        for p in PROTOCOLS {
            if let Some(v) = w
                .globals
                .iter()
                .filter(|g| g.interface == *p)
                .map(|g| g.version)
                .max()
            {
                protocols.push(format!("{p}@{v}"));
            }
        }
    }
    if let Some(Read::Ok(r)) = &f.randr {
        protocols.push(format!("RANDR {}.{}", r.version[0], r.version[1]));
        if info.is_none() {
            let xwayland = r.outputs.iter().any(|o| o.name.starts_with("XWAYLAND"));
            info = Some(if xwayland {
                CompositorInfo {
                    kind: CompositorKind::Other,
                    name: "Xwayland (a Wayland compositor seen through X11)".into(),
                    version: None,
                }
            } else {
                CompositorInfo {
                    kind: CompositorKind::Xorg,
                    name: r.vendor.clone().unwrap_or_else(|| "X server".into()),
                    version: r.release.map(xorg_version),
                }
            });
        }
    }
    if let Some(Read::Ok(_)) = &f.mutter {
        protocols.push(mutter::SERVICE.into());
        if !matches!(
            info,
            Some(CompositorInfo {
                kind: CompositorKind::Mutter,
                ..
            })
        ) {
            // GNOME on X11: Mutter is the window manager and owns the display configuration.
            info = Some(CompositorInfo {
                kind: CompositorKind::Mutter,
                name: name_of(CompositorKind::Mutter).into(),
                version: None,
            });
        }
    }
    info.map(|i| (i, protocols))
}

fn name_of(kind: CompositorKind) -> &'static str {
    match kind {
        CompositorKind::Kwin => "KWin",
        CompositorKind::Mutter => "Mutter",
        CompositorKind::Wlroots => "wlroots",
        CompositorKind::Hyprland => "Hyprland",
        CompositorKind::Gamescope => "Gamescope",
        CompositorKind::Weston => "Weston",
        CompositorKind::Xorg => "X server",
        CompositorKind::Other => "unknown Wayland compositor",
    }
}

/// X.Org's release number `MMmmppxx` as `MM.mm.pp`.
fn xorg_version(r: u32) -> String {
    format!(
        "{}.{}.{}",
        r / 10_000_000,
        r / 100_000 % 100,
        r / 1000 % 100
    )
}

// ----- per-output views ------------------------------------------------------------------------

/// One output as the compositor sources describe it, before it is matched to an endpoint.
#[derive(Debug, Clone)]
struct View {
    name: String,
    ids: Vec<CorrelationId>,
    model: Option<String>,
    edid: Option<Vec<u8>>,
    f: Fields,
    modes: Option<Value<Vec<Mode>>>,
}

/// Every reportable field, each optional, so sources can be layered.
#[derive(Debug, Clone, Default)]
struct Fields {
    enabled: Option<Value<bool>>,
    mode: Option<Value<Mode>>,
    scale: Option<Value<f32>>,
    hdr_enabled: Option<Value<bool>>,
    wide_color: Option<Value<bool>>,
    sdr_brightness: Option<Value<f32>>,
    image_description: Option<Value<ImageDescription>>,
    vrr_policy: Option<Value<VrrPolicy>>,
    rgb_range: Option<Value<String>>,
    max_bpc: Option<Value<u8>>,
    hdr: Option<Value<HdrCapabilities>>,
}

impl Fields {
    /// Fills every field this one lacks from `other`.
    fn or(mut self, other: Fields) -> Fields {
        macro_rules! fill {
            ($($f:ident),*) => { $( if self.$f.is_none() { self.$f = other.$f; } )* };
        }
        fill!(
            enabled,
            mode,
            scale,
            hdr_enabled,
            wide_color,
            sdr_brightness,
            image_description,
            vrr_policy,
            rgb_range,
            max_bpc,
            hdr
        );
        self
    }
}

struct Ctx {
    now: u64,
}

impl Ctx {
    fn prov(&self, source: Source) -> Provenance {
        Provenance {
            layer: LayerKind::Compositor,
            source,
            read_at_ms: self.now,
        }
    }
    fn wl(&self, interface: &str, event: &str) -> Provenance {
        self.prov(Source::Wayland {
            interface: interface.into(),
            event: event.into(),
        })
    }
    fn api(&self, api: &str, call: &str) -> Provenance {
        self.prov(Source::api(api, call))
    }
}

fn mode_of(width: i32, height: i32, mhz: u32, preferred: bool, interlaced: bool) -> Mode {
    Mode {
        width: u16::try_from(width.max(0)).unwrap_or(u16::MAX),
        height: u16::try_from(height.max(0)).unwrap_or(u16::MAX),
        refresh_millihz: mhz,
        interlaced,
        timing: None,
        vic: None,
        hdmi_vic: None,
        preferred,
        ycbcr420: Ycbcr420::No,
        sources: vec![ModeSource::Compositor],
    }
}

fn wl_mode(m: &facts::WlMode) -> Mode {
    mode_of(
        m.width,
        m.height,
        u32::try_from(m.refresh_mhz.max(0)).unwrap_or(0),
        m.preferred,
        false,
    )
}

/// Named primaries for the protocol's chromaticities (×1 000 000), within 0.002.
fn name_primaries(p: &[i32; 8]) -> String {
    const KNOWN: &[(&str, [i32; 8])] = &[
        (
            "srgb",
            [
                640_000, 330_000, 300_000, 600_000, 150_000, 60_000, 312_700, 329_000,
            ],
        ),
        (
            "bt2020",
            [
                708_000, 292_000, 170_000, 797_000, 131_000, 46_000, 312_700, 329_000,
            ],
        ),
        (
            "dci_p3",
            [
                680_000, 320_000, 265_000, 690_000, 150_000, 60_000, 314_000, 351_000,
            ],
        ),
        (
            "display_p3",
            [
                680_000, 320_000, 265_000, 690_000, 150_000, 60_000, 312_700, 329_000,
            ],
        ),
    ];
    KNOWN
        .iter()
        .find(|(_, k)| k.iter().zip(p).all(|(a, b)| (a - b).abs() <= 2_000))
        .map(|(n, _)| n.to_string())
        .unwrap_or_else(|| "custom".into())
}

fn image_description(i: &ImageInfo) -> ImageDescription {
    let primaries = i
        .primaries_named
        .clone()
        .or_else(|| i.primaries.as_ref().map(name_primaries))
        .unwrap_or_else(|| "unknown".into());
    let transfer = i
        .tf_named
        .clone()
        .or_else(|| {
            i.tf_power
                .map(|e| format!("power_{:.2}", f64::from(e) / 10_000.0))
        })
        .unwrap_or_else(|| "unknown".into());
    let target_primaries = i
        .target_primaries
        .as_ref()
        .map(name_primaries)
        .filter(|t| *t != primaries);
    ImageDescription {
        primaries,
        transfer,
        min_luminance_nits: i.luminances.map(|l| l[0] as f32 / 10_000.0),
        max_luminance_nits: i.luminances.map(|l| l[1] as f32),
        reference_luminance_nits: i.luminances.map(|l| l[2] as f32),
        target_primaries,
        target_max_luminance_nits: i.target_luminance.map(|l| l[1] as f32),
    }
}

fn is_hdr_tf(tf: &str) -> bool {
    matches!(tf, "st2084_pq" | "hlg")
}

/// Fields from an output's color-management image description.
fn cm_fields(c: &Ctx, img: &Option<Read<ImageInfo>>) -> Fields {
    let mut f = Fields::default();
    let p = || {
        c.wl(
            "wp_image_description_info_v1",
            "tf_named, primaries_named, luminances",
        )
    };
    match img {
        Some(Read::Ok(i)) => {
            let d = image_description(i);
            let hdr = is_hdr_tf(&d.transfer);
            f.hdr_enabled = Some(Value::inferred(hdr, p()));
            f.wide_color = Some(Value::inferred(
                d.primaries == "bt2020" || d.target_primaries.as_deref() == Some("bt2020"),
                p(),
            ));
            if hdr {
                if let Some(r) = d.reference_luminance_nits {
                    f.sdr_brightness = Some(Value::inferred(r, p()));
                }
                let eotf = if d.transfer == "hlg" {
                    Eotf::Hlg
                } else {
                    Eotf::Pq
                };
                f.hdr = Some(Value::inferred(
                    HdrCapabilities {
                        eotfs: vec![Eotf::TraditionalSdr, eotf],
                        max_luminance_nits: d.target_max_luminance_nits,
                        min_luminance_nits: i.target_luminance.map(|l| l[0] as f32 / 10_000.0),
                        max_frame_average_nits: i.target_max_fall.map(|v| v as f32),
                        ..HdrCapabilities::default()
                    },
                    p(),
                ));
            }
            f.image_description = Some(Value::measured(d, p()));
        }
        Some(Read::Err { reason, detail }) => {
            f.image_description = Some(Value::unknown(*reason, detail.clone()));
        }
        None => {}
    }
    f
}

fn kde_fields(c: &Ctx, d: &KdeDevice) -> (Fields, Option<Value<Vec<Mode>>>) {
    const I: &str = "kde_output_device_v2";
    let mut f = Fields::default();
    let caps = d.capabilities.unwrap_or(0);
    f.enabled = d.enabled.map(|e| Value::measured(e, c.wl(I, "enabled")));
    let on = d.enabled != Some(false);
    f.mode = d
        .current_mode
        .and_then(|i| d.modes.get(i))
        .filter(|_| on)
        .map(|m| Value::measured(wl_mode(m), c.wl(I, "current_mode")));
    f.scale = d
        .scale
        .filter(|_| on)
        .map(|s| Value::measured(s as f32, c.wl(I, "scale")));
    f.hdr_enabled = d
        .hdr
        .map(|h| Value::measured(h, c.wl(I, "high_dynamic_range")));
    f.wide_color = d
        .wide_color
        .map(|w| Value::measured(w, c.wl(I, "wide_color_gamut")));
    if d.hdr == Some(true) {
        f.sdr_brightness = d
            .sdr_brightness
            .map(|b| Value::measured(b as f32, c.wl(I, "sdr_brightness")));
    }
    if caps & facts::kde_cap::VRR != 0 {
        f.vrr_policy = d.vrr_policy.and_then(|v| {
            let p = match v {
                0 => VrrPolicy::Never,
                1 => VrrPolicy::Always,
                2 => VrrPolicy::Automatic,
                _ => return None,
            };
            Some(Value::measured(p, c.wl(I, "vrr_policy")))
        });
    }
    if caps & facts::kde_cap::RGB_RANGE != 0 {
        f.rgb_range = d.rgb_range.map(|r| {
            let n = match r {
                0 => "automatic",
                1 => "full",
                2 => "limited",
                _ => "unknown",
            };
            Value::measured(n.to_string(), c.wl(I, "rgb_range"))
        });
    }
    if d.capabilities.is_some() {
        let mut h = HdrCapabilities {
            eotfs: vec![Eotf::TraditionalSdr],
            ..HdrCapabilities::default()
        };
        if caps & facts::kde_cap::HDR != 0 {
            h.eotfs.push(Eotf::Pq);
        }
        let ov = d.brightness_overrides.unwrap_or([-1; 3]);
        let pick = |o: i32, m: Option<u32>| {
            if o >= 0 {
                Some(o as f32)
            } else {
                m.filter(|v| *v > 0).map(|v| v as f32)
            }
        };
        let md = d.brightness_metadata;
        h.max_luminance_nits = pick(ov[0], md.map(|m| m[0]));
        h.max_frame_average_nits = pick(ov[1], md.map(|m| m[1]));
        h.min_luminance_nits = pick(ov[2], md.map(|m| m[2])).map(|v| v / 10_000.0);
        f.hdr = Some(Value::measured(
            h,
            c.wl(I, "capabilities, brightness_metadata"),
        ));
    }
    let modes = (!d.modes.is_empty())
        .then(|| Value::measured(d.modes.iter().map(wl_mode).collect(), c.wl(I, "mode")));
    (f, modes)
}

fn wayland_views(c: &Ctx, w: &WaylandFacts, kind: CompositorKind) -> Vec<View> {
    let mut views: Vec<View> = vec![];
    fn find(views: &mut Vec<View>, name: &str) -> usize {
        match views.iter().position(|v| v.name == name) {
            Some(i) => i,
            None => {
                views.push(View {
                    name: name.into(),
                    ids: vec![],
                    model: None,
                    edid: None,
                    f: Fields::default(),
                    modes: None,
                });
                views.len() - 1
            }
        }
    }
    // KDE devices first (the most detail), then wlr heads, then wl_output.
    for d in w.kde_devices.iter().flatten() {
        let Some(name) = &d.name else { continue };
        let i = find(&mut views, name);
        let (f, modes) = kde_fields(c, d);
        let v = &mut views[i];
        v.f = std::mem::take(&mut v.f).or(f);
        v.modes = v.modes.take().or(modes);
        v.model = v.model.take().or_else(|| d.model.clone());
        v.edid = v
            .edid
            .take()
            .or_else(|| d.edid_hex.as_deref().and_then(facts::from_hex));
    }
    for h in w.wlr_heads.iter().flatten() {
        if h.name.is_empty() {
            continue;
        }
        const I: &str = "zwlr_output_head_v1";
        let i = find(&mut views, &h.name);
        let f = Fields {
            enabled: Some(Value::measured(h.enabled, c.wl(I, "enabled"))),
            mode: h
                .current_mode
                .and_then(|m| h.modes.get(m))
                .filter(|_| h.enabled)
                .map(|m| Value::measured(wl_mode(m), c.wl(I, "current_mode"))),
            scale: h
                .scale
                .filter(|_| h.enabled)
                .map(|s| Value::measured(s as f32, c.wl(I, "scale"))),
            vrr_policy: h.adaptive_sync.map(|a| {
                Value::measured(
                    if a {
                        VrrPolicy::Always
                    } else {
                        VrrPolicy::Never
                    },
                    c.wl(I, "adaptive_sync"),
                )
            }),
            ..Fields::default()
        };
        let modes = (!h.modes.is_empty())
            .then(|| Value::measured(h.modes.iter().map(wl_mode).collect(), c.wl(I, "mode")));
        let v = &mut views[i];
        v.f = std::mem::take(&mut v.f).or(f);
        v.modes = v.modes.take().or(modes);
        v.model = v.model.take().or_else(|| h.model.clone());
    }
    for o in &w.outputs {
        let Some(name) = &o.name else { continue };
        const I: &str = "wl_output";
        let i = find(&mut views, name);
        let base = Fields {
            // The compositor advertises a wl_output only for an output it shows.
            enabled: Some(Value::inferred(true, c.wl(I, "name"))),
            mode: o
                .mode
                .as_ref()
                .map(|m| Value::measured(wl_mode(m), c.wl(I, "mode"))),
            scale: o
                .scale
                .map(|s| Value::inferred(s as f32, c.wl(I, "scale (integer buffer scale)"))),
            ..Fields::default()
        };
        let v = &mut views[i];
        v.ids.push(CorrelationId::WlOutput { name: name.clone() });
        // Color management sits between the compositor's own API and wl_output.
        v.f = std::mem::take(&mut v.f)
            .or(cm_fields(c, &o.image_description))
            .or(base);
        v.model = v.model.take().or_else(|| o.model.clone());
    }
    if kind == CompositorKind::Wlroots || kind == CompositorKind::Other {
        let why = "the compositor exposes no HDR state (no color-management-v1 image description)";
        for v in &mut views {
            if v.f.hdr_enabled.is_none() {
                v.f.hdr_enabled = Some(Value::unknown(UnknownReason::NotExposed, why.to_string()));
            }
        }
    }
    views
}

fn mutter_views(c: &Ctx, m: &MutterFacts) -> Vec<View> {
    let p = || c.api(mutter::SERVICE, "GetCurrentState");
    let prop_u = |props: &BTreeMap<String, serde_json::Value>, k: &str| {
        props.get(k).and_then(|v| v.as_u64())
    };
    let mut out = vec![];
    for mon in &m.monitors {
        let logical = m
            .logical_monitors
            .iter()
            .find(|l| l.monitors.contains(&mon.connector));
        let current = mon.modes.iter().find(|md| {
            md.properties
                .get("is-current")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        });
        let flag = |md: &facts::MutterMode, k: &str| {
            md.properties
                .get(k)
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        };
        let mmode = |md: &facts::MutterMode| {
            mode_of(
                md.width,
                md.height,
                (md.refresh * 1000.0).round().max(0.0) as u32,
                flag(md, "is-preferred"),
                flag(md, "is-interlaced"),
            )
        };
        let color_mode = prop_u(&mon.properties, "color-mode");
        let mut f = Fields {
            enabled: Some(Value::measured(logical.is_some(), p())),
            mode: current
                .filter(|_| logical.is_some())
                .map(|md| Value::measured(mmode(md), p())),
            scale: logical.map(|l| Value::measured(l.scale as f32, p())),
            ..Fields::default()
        };
        if let Some(cm) = color_mode {
            f.hdr_enabled = Some(Value::measured(cm == 1, p()));
            f.wide_color = Some(Value::inferred(cm == 1, p()));
        }
        if let Some(modes) = mon
            .properties
            .get("supported-color-modes")
            .and_then(|v| v.as_array())
        {
            let mut h = HdrCapabilities {
                eotfs: vec![Eotf::TraditionalSdr],
                ..HdrCapabilities::default()
            };
            if modes.iter().any(|v| v.as_u64() == Some(1)) {
                h.eotfs.push(Eotf::Pq);
            }
            f.hdr = Some(Value::measured(h, p()));
        }
        if let Some(mode) = current.and_then(|md| md.properties.get("refresh-rate-mode")) {
            f.vrr_policy = match mode.as_str() {
                Some("variable") => Some(Value::measured(VrrPolicy::Always, p())),
                Some("fixed") => Some(Value::measured(VrrPolicy::Never, p())),
                _ => None,
            };
        }
        let modes = (!mon.modes.is_empty())
            .then(|| Value::measured(mon.modes.iter().map(mmode).collect(), p()));
        let mut serial = Sensitive::new(SensitiveKind::Serial, mon.serial.clone());
        // Replayed facts carry the capture's hash already.
        if mon.serial.starts_with("sha256:") {
            serial.value = displaycaps_model::SensitiveValue::Redacted(mon.serial.clone());
        }
        out.push(View {
            name: mon.connector.clone(),
            ids: vec![CorrelationId::MutterMonitor {
                connector: mon.connector.clone(),
                vendor: mon.vendor.clone(),
                product: mon.product.clone(),
                serial,
            }],
            model: Some(mon.product.clone()),
            edid: None,
            f,
            modes,
        });
    }
    out
}

fn randr_views(c: &Ctx, r: &RandrFacts) -> Vec<View> {
    let mut out = vec![];
    for o in r.outputs.iter().filter(|o| o.connection == "connected") {
        let prop = |k: &str| o.properties.get(k);
        let pp = |k: &str| c.api("RandR", &format!("GetOutputProperty {k}"));
        let rm = |m: &facts::RandrMode, preferred: bool| {
            mode_of(
                i32::from(m.width),
                i32::from(m.height),
                m.refresh_mhz,
                preferred,
                m.interlaced,
            )
        };
        let x11_hdr = "X11 has no HDR output path";
        // XWayland: the app sees the host Wayland compositor through X11 only (e.g. a Flatpak
        // with just the X11 socket), so the host's HDR state is out of reach, not off.
        let xwayland = o.name.starts_with("XWAYLAND");
        fn xw<T>() -> Value<T> {
            Value::unknown(
                UnknownReason::NotGranted,
                "XWayland output: the host Wayland compositor's HDR state isn't visible over X11 \
                 (needs the Wayland socket)"
                    .to_string(),
            )
        }
        let f = Fields {
            enabled: Some(Value::measured(
                o.current.is_some(),
                c.api("RandR", "GetOutputInfo"),
            )),
            mode: o
                .current
                .as_ref()
                .map(|m| Value::measured(rm(m, false), c.api("RandR", "GetCrtcInfo"))),
            hdr_enabled: Some(if xwayland {
                xw()
            } else {
                Value::inferred(false, c.prov(Source::derived(x11_hdr)))
            }),
            wide_color: prop("Colorspace")
                .map(|s| Value::inferred(s.contains("BT2020"), pp("Colorspace"))),
            // Named like KWin's setting: automatic, full or limited.
            rgb_range: prop("Broadcast RGB").map(|s| {
                let l = s.to_ascii_lowercase();
                let n = ["automatic", "full", "limited"]
                    .into_iter()
                    .find(|n| l.starts_with(n))
                    .map(str::to_string)
                    .unwrap_or(l);
                Value::measured(n, pp("Broadcast RGB"))
            }),
            max_bpc: prop("max bpc")
                .and_then(|s| s.split(',').next()?.trim().parse::<u8>().ok())
                .map(|b| Value::measured(b, pp("max bpc"))),
            hdr: Some(if xwayland {
                xw()
            } else {
                Value::inferred(
                    HdrCapabilities {
                        eotfs: vec![Eotf::TraditionalSdr],
                        ..HdrCapabilities::default()
                    },
                    c.prov(Source::derived(x11_hdr)),
                )
            }),
            ..Fields::default()
        };
        let modes = (!o.modes.is_empty()).then(|| {
            Value::measured(
                o.modes
                    .iter()
                    .enumerate()
                    .map(|(i, m)| rm(m, i < o.preferred))
                    .collect(),
                c.api("RandR", "GetOutputInfo"),
            )
        });
        out.push(View {
            name: o.name.clone(),
            ids: vec![CorrelationId::RandrOutput {
                name: o.name.clone(),
            }],
            model: None,
            edid: prop("EDID").and_then(|h| facts::from_hex(h)),
            f,
            modes,
        });
    }
    out
}

/// Merges views from several sources by output name; earlier sources win field by field.
fn merge(mut into: Vec<View>, more: Vec<View>) -> Vec<View> {
    for v in more {
        match into
            .iter_mut()
            .find(|x| drm_style(&x.name) == drm_style(&v.name))
        {
            Some(x) => {
                x.f = std::mem::take(&mut x.f).or(v.f);
                x.modes = x.modes.take().or(v.modes);
                x.ids.extend(v.ids);
                x.model = x.model.take().or(v.model);
                x.edid = x.edid.take().or(v.edid);
            }
            None => into.push(v),
        }
    }
    into
}

/// A compositor output name in DRM's form: Mutter and the modesetting driver write `HDMI-1` for
/// DRM's `HDMI-A-1`.
fn drm_style(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("HDMI-")
        && rest.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        return format!("HDMI-A-{rest}");
    }
    name.to_string()
}

/// The compositor kind behind the facts (for each output's `compositor` field).
fn output_kind(f: &CompositorFacts) -> CompositorKind {
    identity(f)
        .map(|(i, _)| i.kind)
        .unwrap_or(CompositorKind::Other)
}

fn views(f: &CompositorFacts, now: u64) -> Vec<View> {
    let c = Ctx { now };
    let kind = output_kind(f);
    // Mutter's own API is the most complete on GNOME; KWin's and wlroots' are read through
    // Wayland. RandR only in an X11 session.
    let mut v = vec![];
    if let Some(Read::Ok(m)) = &f.mutter {
        v = merge(v, mutter_views(&c, m));
    }
    if let Some(Read::Ok(w)) = &f.wayland {
        v = merge(v, wayland_views(&c, w, kind));
    }
    if let Some(Read::Ok(r)) = &f.randr {
        v = merge(v, randr_views(&c, r));
    }
    v
}

// ----- attaching to endpoints ------------------------------------------------------------------

/// The system-wide availability of the compositor layer.
pub(crate) fn availability(read: &CompRead) -> Availability {
    match read {
        CompRead::Unreadable(r, d) => Availability::Unknown {
            reason: *r,
            detail: Some(d.clone()),
        },
        CompRead::Facts(f) => {
            let srcs: Vec<Option<(UnknownReason, String)>> = [
                f.wayland.as_ref().map(err_of),
                f.mutter.as_ref().map(err_of),
                f.randr.as_ref().map(err_of),
            ]
            .into_iter()
            .flatten()
            .collect();
            let failed: Vec<_> = srcs.iter().flatten().collect();
            if srcs.is_empty() {
                Availability::Unknown {
                    reason: UnknownReason::NoSession,
                    detail: Some("no compositor source was read".into()),
                }
            } else if failed.is_empty() {
                Availability::Read
            } else if failed.len() == srcs.len() {
                Availability::Unknown {
                    reason: failed[0].0,
                    detail: Some(failed[0].1.clone()),
                }
            } else {
                Availability::Partial
            }
        }
    }
}

fn err_of<T>(r: &Read<T>) -> Option<(UnknownReason, String)> {
    match r {
        Read::Ok(_) => None,
        Read::Err { reason, detail } => Some((*reason, detail.clone())),
    }
}

/// The connector name in an endpoint id (`card1-HDMI-A-1` → `HDMI-A-1`).
fn connector_of(id: &EndpointId) -> &str {
    let s = id.0.as_str();
    match s.split_once('-') {
        Some((card, rest)) if card.starts_with("card") => rest,
        _ => s,
    }
}

/// Manufacturer, product code and serial number bytes (EDID bytes 8–15).
fn edid_id(edid: &[u8]) -> Option<&[u8]> {
    edid.get(8..16)
}

/// Whether an output's EDID names the same sink as an endpoint: against the endpoint's own EDID
/// (read for the join even when the sink isn't reported), else its sink record.
fn same_sink(edid: &[u8], e: &Endpoint, edids: &BTreeMap<String, Vec<u8>>) -> Option<bool> {
    let ours = edid_id(edid)?;
    if let Some(theirs) = edids.get(&e.id.0).and_then(|b| edid_id(b)) {
        return Some(ours == theirs);
    }
    let id = &e.sink.as_ref()?.identity;
    let product = u16::from_le_bytes([ours[2], ours[3]]);
    let m = u16::from_be_bytes([ours[0], ours[1]]);
    let pnp: String = [(m >> 10) & 31, (m >> 5) & 31, m & 31]
        .iter()
        .map(|c| char::from(b'@' + *c as u8))
        .collect();
    Some(
        id.product_code.known() == Some(&product)
            && id.manufacturer_id.known().map(String::as_str) == Some(pnp.as_str()),
    )
}

/// The endpoint a view belongs to: by connector name, checked against the sink's identity when
/// both sides have one (amdgpu's and nvidia's X drivers number from 0, so a matching name can be
/// another port), with identity alone to settle ties or find a renamed connector. An exact name
/// with nothing contradicting it is `measured`.
fn match_view(
    v: &View,
    eps: &[Endpoint],
    edids: &BTreeMap<String, Vec<u8>>,
) -> Option<(usize, Confidence)> {
    let want = drm_style(&v.name);
    let by_name: Vec<usize> = (0..eps.len())
        .filter(|&i| connector_of(&eps[i].id) == want)
        .collect();
    let identity = |i: usize| -> Option<bool> {
        let e = &eps[i];
        if let Some(edid) = &v.edid
            && let Some(same) = same_sink(edid, e, edids)
        {
            return Some(same);
        }
        let model = e.sink.as_ref().and_then(|s| s.identity.model_name.known());
        match (model, &v.model) {
            (Some(a), Some(b)) => Some(a == b),
            _ => None,
        }
    };
    let named = match by_name.len() {
        0 => None,
        1 => match identity(by_name[0]) {
            Some(false) => None,
            same => Some((
                by_name[0],
                if connector_of(&eps[by_name[0]].id) == v.name && same != Some(false) {
                    Confidence::Measured
                } else {
                    Confidence::Inferred
                },
            )),
        },
        _ => {
            let hits: Vec<_> = by_name
                .iter()
                .copied()
                .filter(|&i| identity(i) == Some(true))
                .collect();
            (hits.len() == 1).then(|| (hits[0], Confidence::Inferred))
        }
    };
    named.or_else(|| {
        let hits: Vec<usize> = (0..eps.len())
            .filter(|i| eps[*i].status.known() == Some(&ConnectionStatus::Connected))
            .filter(|&i| identity(i) == Some(true))
            .collect();
        (hits.len() == 1).then(|| (hits[0], Confidence::Inferred))
    })
}

fn class_of(name: &str) -> (EndpointClass, ConnectorType) {
    let n = name.to_ascii_lowercase();
    if n.starts_with("edp") {
        (EndpointClass::Internal, ConnectorType::Edp)
    } else if n.starts_with("lvds") {
        (EndpointClass::Internal, ConnectorType::Lvds)
    } else if n.starts_with("dsi") {
        (EndpointClass::Internal, ConnectorType::Dsi)
    } else if n.starts_with("hdmi") {
        (EndpointClass::External, ConnectorType::Hdmi)
    } else if n.starts_with("dp") || n.starts_with("displayport") {
        (EndpointClass::External, ConnectorType::DisplayPort)
    } else if n.starts_with("dvi") {
        (EndpointClass::External, ConnectorType::Dvi)
    } else if n.starts_with("vga") {
        (EndpointClass::External, ConnectorType::Vga)
    } else if n.starts_with("virtual")
        || n.starts_with("headless")
        || n.starts_with("wl-")
        || n.starts_with("x11-")
        || n.starts_with("xwayland")
    {
        (EndpointClass::Virtual, ConnectorType::Virtual)
    } else {
        (EndpointClass::External, ConnectorType::Other(name.into()))
    }
}

/// Adds the compositor layer (and its correlation keys) to `eps`, creating endpoints for outputs
/// that match no DRM connector. Returns the system-wide availability.
pub(crate) fn attach(
    eps: &mut Vec<Endpoint>,
    edids: &BTreeMap<String, Vec<u8>>,
    read: &CompRead,
    scope: &ProbeScope,
    now: u64,
) -> Availability {
    let sys = availability(read);
    let facts = match read {
        CompRead::Facts(f) if !matches!(sys, Availability::Unknown { .. }) => f,
        _ => {
            let (reason, detail) = match &sys {
                Availability::Unknown { reason, detail } => (*reason, detail.clone()),
                _ => (UnknownReason::ReadError, None),
            };
            for e in eps.iter_mut() {
                e.layers.push(LayerCapabilities {
                    layer: LayerKind::Compositor,
                    availability: Availability::Unknown {
                        reason,
                        detail: detail.clone(),
                    },
                    caps: Default::default(),
                    detail: None,
                });
            }
            return sys;
        }
    };
    let kind = output_kind(facts);
    let mut done = vec![false; eps.len()];
    let n = eps.len();
    for v in views(facts, now) {
        let idx = match match_view(&v, &eps[..n], edids) {
            // Two outputs for one connector (a renamed output found by identity, say): the
            // first keeps it; the other is not reported rather than as a second endpoint.
            Some((i, _)) if done[i] => continue,
            Some((i, conf)) => {
                done[i] = true;
                if scope.fields.includes(&FieldGroup::Identity) {
                    for id in &v.ids {
                        eps[i].correlation.push(CorrelationKey {
                            id: id.clone(),
                            confidence: conf,
                        });
                    }
                }
                i
            }
            _ => {
                let (class, ctype) = class_of(&v.name);
                let id = EndpointId(format!("compositor-{}", v.name));
                if !scope.includes_endpoint(class, &id) {
                    continue;
                }
                let c = Ctx { now };
                let src = Source::derived("the compositor's output list");
                eps.push(Endpoint {
                    id,
                    class,
                    connector: Value::inferred(ctype, c.prov(src.clone())),
                    status: Value::inferred(ConnectionStatus::Connected, c.prov(src)),
                    name: None,
                    correlation: if scope.fields.includes(&FieldGroup::Identity) {
                        v.ids
                            .iter()
                            .map(|id| CorrelationKey {
                                id: id.clone(),
                                confidence: Confidence::Measured,
                            })
                            .collect()
                    } else {
                        vec![]
                    },
                    sink: None,
                    layers: vec![],
                    effective: vec![],
                    paths: vec![],
                });
                eps.len() - 1
            }
        };
        let layer = layer_of(&v, kind, scope, now);
        eps[idx].layers.push(layer);
    }
    for (i, e) in eps.iter_mut().enumerate().take(n) {
        if !done[i] {
            e.layers.push(LayerCapabilities {
                layer: LayerKind::Compositor,
                availability: Availability::Unknown {
                    reason: UnknownReason::NotExposed,
                    detail: Some("the compositor lists no output for this connector".into()),
                },
                caps: Default::default(),
                detail: None,
            });
        }
    }
    sys
}

fn layer_of(v: &View, kind: CompositorKind, scope: &ProbeScope, now: u64) -> LayerCapabilities {
    let field = |g: FieldGroup| scope.fields.includes(&g);
    let f = &v.f;
    let mut caps = displaycaps_model::CapabilitySet::default();
    if field(FieldGroup::Modes) {
        caps.modes = Some(v.modes.clone().unwrap_or_else(|| {
            Value::unknown(
                UnknownReason::NotExposed,
                "the compositor states only the current mode".to_string(),
            )
        }));
    }
    if field(FieldGroup::Hdr) {
        caps.hdr = Some(f.hdr.clone().unwrap_or_else(|| {
            Value::unknown(
                UnknownReason::NotExposed,
                "the compositor states no HDR capability for this output".to_string(),
            )
        }));
    }
    let detail = field(FieldGroup::Compositor).then(|| {
        LayerDetail::Compositor(CompositorOutput {
            compositor: kind,
            output_name: Value::measured(
                v.name.clone(),
                Provenance {
                    layer: LayerKind::Compositor,
                    source: Source::derived("the compositor's output name"),
                    read_at_ms: now,
                },
            ),
            enabled: f.enabled.clone().unwrap_or_else(|| {
                Value::unknown(UnknownReason::NotExposed, "no enabled state".to_string())
            }),
            mode: f.mode.clone(),
            scale: f.scale.clone(),
            hdr_enabled: f.hdr_enabled.clone(),
            wide_color_enabled: f.wide_color.clone(),
            sdr_brightness_nits: f.sdr_brightness.clone(),
            image_description: f.image_description.clone(),
            vrr_policy: f.vrr_policy.clone(),
            rgb_range: f.rgb_range.clone(),
            max_bpc: f.max_bpc.clone(),
            hdr: f.hdr.clone(),
        })
    });
    let known = f.hdr_enabled.as_ref().is_some_and(|h| h.known().is_some())
        && f.enabled.as_ref().is_some_and(|e| e.known().is_some());
    LayerCapabilities {
        layer: LayerKind::Compositor,
        availability: if known {
            Availability::Read
        } else {
            Availability::Partial
        },
        caps,
        detail,
    }
}

/// Fuzzing entry: facts JSON through identity and the layer, against endpoints that share names
/// with typical outputs.
pub(crate) fn fuzz(data: &[u8]) {
    let Ok(f) = serde_json::from_slice::<CompositorFacts>(data) else {
        return;
    };
    let _ = identity(&f);
    let p = Provenance {
        layer: LayerKind::HostHardware,
        source: Source::derived("fuzz"),
        read_at_ms: 0,
    };
    let mut eps: Vec<Endpoint> = [
        "card0-HDMI-A-1",
        "card1-HDMI-A-1",
        "card0-DP-1",
        "card0-eDP-1",
    ]
    .iter()
    .map(|id| Endpoint {
        id: EndpointId(id.to_string()),
        class: EndpointClass::External,
        connector: Value::measured(ConnectorType::Hdmi, p.clone()),
        status: Value::measured(ConnectionStatus::Connected, p.clone()),
        name: None,
        correlation: vec![],
        sink: None,
        layers: vec![],
        effective: vec![],
        paths: vec![],
    })
    .collect();
    let mut f2 = f.clone();
    f2.redact();
    let none = BTreeMap::new();
    let _ = attach(
        &mut eps,
        &none,
        &CompRead::Facts(f),
        &ProbeScope::everything(),
        0,
    );
    let _ = attach(
        &mut vec![],
        &none,
        &CompRead::Facts(f2),
        &ProbeScope::everything(),
        0,
    );
}

#[cfg(test)]
mod tests;
