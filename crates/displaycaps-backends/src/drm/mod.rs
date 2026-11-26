// SPDX-License-Identifier: LGPL-2.1-or-later
//! The DRM backends: sysfs (cards, connectors, cached EDID, driver) and read-only KMS, joined into
//! endpoints for the probe scope.
//!
//! What is read, and when:
//! - `/sys/class/drm` is listed when any endpoint layer (sink chain, host hardware, kernel driver,
//!   KMS) is in scope. Connector attributes are the kernel's cached values.
//! - EDID bytes are read only for connected connectors in scope, and only when the sink chain
//!   layer is in scope.
//! - KMS is read only when the KMS layer is in scope, and on the live system only inside a display
//!   session (see [`kms_access`]). A fixture replays its captured `drm_info -j` output.
//! - Module parameters are read only for the card's driver and only with the `driver` field group.

pub(crate) mod kms;
mod kms_live;
pub mod link;
pub(crate) mod sysfs;
pub(crate) mod uevent;

use std::collections::BTreeMap;

use displaycaps_model::{
    Availability, CapabilitySet, Confidence, ConnectionStatus, CorrelationId, CorrelationKey,
    DriverDetail, Endpoint, EndpointId, FieldGroup, Gpu, HostDetail, LayerCapabilities,
    LayerDetail, LayerKind, ModuleParameter, ProbeScope, Provenance, Selection, SessionKind,
    Source, UnknownReason, Value,
};
use displaycaps_rules::quirks::QuirkSet;

use crate::environment::Env;
use crate::root::SystemRoot;
use crate::runtime::ProbeOptions;
use kms::{KmsCard, KmsConnector};
use sysfs::{SysCard, SysConnector};

/// The layers an endpoint is built from. If none is in scope, `/sys/class/drm` is not read.
pub(crate) const ENDPOINT_LAYERS: [LayerKind; 5] = [
    LayerKind::SinkChain,
    LayerKind::HostHardware,
    LayerKind::KernelDriver,
    LayerKind::Kms,
    LayerKind::Link,
];

/// Whether the link layer is in scope. It is worked out from the sink's EDID, KMS and the
/// driver's state, so those are read for it even when their own layers are not reported.
pub(crate) fn link_in_scope(scope: &ProbeScope) -> bool {
    scope.layers.includes(&LayerKind::Link)
}

/// Whether the scope needs the DRM backends at all.
pub(crate) fn in_scope(scope: &ProbeScope) -> bool {
    ENDPOINT_LAYERS.iter().any(|l| scope.layers.includes(l))
}

/// The sysfs tree: cards and connectors, or why `/sys/class/drm` couldn't be listed.
pub(crate) type SysTree = Result<(Vec<SysCard>, Vec<SysConnector>), (UnknownReason, String)>;

/// What sysfs gave.
#[derive(Debug, Default)]
pub(crate) struct SysRead {
    /// Cards and connectors, or why `/sys/class/drm` couldn't be listed.
    pub tree: Option<SysTree>,
    /// EDID bytes per connector name (connected connectors in scope only).
    pub edids: BTreeMap<String, Result<Vec<u8>, (UnknownReason, String)>>,
    /// Module parameters per card (driver detail or the link layer only).
    pub params: BTreeMap<String, Vec<ModuleParameter>>,
    /// The kernel release (driver detail or the link layer only).
    pub kernel: Option<String>,
}

/// What the KMS backend reads on one card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CardPlan {
    /// `card0`.
    pub card: String,
    /// KMS connector ids in scope; `None` reads every connector (sysfs has no `connector_id`).
    pub connectors: Option<Vec<u32>>,
    /// Connectors whose EDID blob is read: the sink is wanted and sysfs can't give the EDID.
    pub edid_for: Vec<u32>,
    /// Whether `HDR_OUTPUT_METADATA` blobs are read.
    pub hdr: bool,
}

/// One card's outcome.
#[derive(Debug, Clone)]
pub(crate) enum CardResult {
    /// Read.
    Read(KmsCard),
    /// Not opened, with the reason.
    Skipped(String, UnknownReason, String),
    /// Opening or reading failed.
    Failed(String, UnknownReason, String),
}

impl CardResult {
    fn card(&self) -> &str {
        match self {
            Self::Read(k) => &k.card,
            Self::Skipped(c, ..) | Self::Failed(c, ..) => c,
        }
    }
}

/// What KMS gave.
#[derive(Debug, Clone)]
pub(crate) enum KmsRead {
    /// Not read: the KMS layer is not in scope.
    NotInScope,
    /// Not readable, with the reason.
    Unreadable(UnknownReason, String),
    /// Per card in scope.
    Cards(Vec<CardResult>),
}

/// Whether KMS may be read here and how.
pub(crate) enum KmsAccess {
    /// Read the live cards (each card still has to pass [`kms_plan`]).
    Live,
    /// Replay the fixture's `drm_info -j` output.
    Fixture(Vec<u8>),
    /// Not readable.
    No(UnknownReason, String),
}

/// Whether the live KMS state may be read. Opening `/dev/dri/cardN` while no process is DRM
/// master of it makes the opener master implicitly (and its last close restores the console's
/// mode), so cards are opened only where a compositor holds master:
/// - the process is in a Wayland or X11 session (environment), and
/// - when logind's state is readable (`/run/systemd/seats/seat0`), seat0's active session is a
///   graphical one, so its compositor holds master now (not switched away to a text console).
///
/// Each card must also drive an output ([`kms_plan`]).
pub(crate) fn kms_access(root: &SystemRoot, env: &Env) -> KmsAccess {
    match root {
        SystemRoot::Fixture(_) => match root.captured_command("drm_info") {
            Some((_, Some(out))) => KmsAccess::Fixture(out),
            _ => KmsAccess::No(
                UnknownReason::NotExposed,
                "the capture has no drm_info output (taken outside a display session)".into(),
            ),
        },
        SystemRoot::Live => {
            if !matches!(
                crate::environment::session_kind(env),
                SessionKind::Wayland | SessionKind::X11
            ) {
                return KmsAccess::No(
                    UnknownReason::NoSession,
                    "no display session: opening /dev/dri/card* would make this process DRM master"
                        .into(),
                );
            }
            match seat0_graphical(root) {
                Some(false) => KmsAccess::No(
                    UnknownReason::NoSession,
                    "seat0's active session is not graphical (switched to a text console?): no \
                     compositor holds DRM master"
                        .into(),
                ),
                // Graphical, or logind's state isn't visible here (sandbox, no systemd).
                _ => KmsAccess::Live,
            }
        }
    }
}

/// Whether seat0's active session is a Wayland or X11 session, from logind's state files.
/// `None` when they can't be read.
fn seat0_graphical(root: &SystemRoot) -> Option<bool> {
    let field = |text: &str, key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(str::to_string)
    };
    let seat = root.read_string("/run/systemd/seats/seat0").ok()?;
    let Some(id) = field(&seat, "ACTIVE").filter(|s| !s.is_empty()) else {
        return Some(false);
    };
    if id.contains('/') || id.contains("..") {
        return Some(false);
    }
    let session = root
        .read_string(&format!("/run/systemd/sessions/{id}"))
        .ok()?;
    let graphical = matches!(field(&session, "TYPE").as_deref(), Some("wayland" | "x11"));
    let active = field(&session, "STATE").as_deref() == Some("active");
    Some(graphical && active)
}

/// The cards to read and what to read on each, from sysfs, for the scope. A card is left out when
/// none of its connectors is in scope; on the live system it is also skipped (not opened) when
/// none of its connectors drives a CRTC: a card that shows nothing has no compositor holding its
/// master, and opening it could also wake a runtime-suspended GPU.
pub(crate) fn kms_plan(
    root: &SystemRoot,
    scope: &ProbeScope,
    live: bool,
) -> Result<(Vec<CardPlan>, Vec<CardResult>), (UnknownReason, String)> {
    let (cards, conns) =
        sysfs::scan(root).map_err(|e| (io_reason(&e), format!("{}: {e}", sysfs::CLASS)))?;
    let sink =
        scope.layers.includes(&LayerKind::SinkChain) && wants_sink(scope) || link_in_scope(scope);
    let mut plans = vec![];
    let mut skipped = vec![];
    for card in &cards {
        let mine: Vec<&SysConnector> = conns.iter().filter(|c| c.card == card.name).collect();
        let in_scope: Vec<&&SysConnector> = mine
            .iter()
            .filter(|c| connector_in_scope(scope, c))
            .collect();
        if in_scope.is_empty() {
            continue;
        }
        if live && !mine.iter().any(|c| c.enabled == Some(true)) {
            skipped.push(CardResult::Skipped(
                card.name.clone(),
                UnknownReason::NotExposed,
                format!(
                    "{} drives no output: not opened (no compositor holds its DRM master)",
                    card.name
                ),
            ));
            continue;
        }
        let ids: Option<Vec<u32>> = in_scope.iter().map(|c| c.connector_id).collect();
        let edid_for = if sink && live {
            in_scope
                .iter()
                .filter(|c| c.status == Some(ConnectionStatus::Connected))
                .filter(|c| !root.can_open(&c.edid_path()))
                .filter_map(|c| c.connector_id)
                .collect()
        } else {
            vec![]
        };
        plans.push(CardPlan {
            card: card.name.clone(),
            connectors: ids,
            edid_for,
            hdr: scope.fields.includes(&FieldGroup::Hdr),
        });
    }
    Ok((plans, skipped))
}

fn io_reason(e: &std::io::Error) -> UnknownReason {
    match e.kind() {
        std::io::ErrorKind::NotFound => UnknownReason::NotExposed,
        std::io::ErrorKind::PermissionDenied => UnknownReason::NotGranted,
        _ => UnknownReason::ReadError,
    }
}

/// Reads one planned card.
pub(crate) fn read_planned(root: &SystemRoot, access: &KmsAccess, plan: &CardPlan) -> CardResult {
    match access {
        KmsAccess::No(r, d) => CardResult::Skipped(plan.card.clone(), *r, d.clone()),
        KmsAccess::Fixture(json) => match kms::from_drm_info(json) {
            Ok(cards) => match cards.into_iter().find(|c| c.card == plan.card) {
                Some(mut c) => {
                    if let Some(ids) = &plan.connectors {
                        c.connectors.retain(|k| ids.contains(&k.id));
                    }
                    CardResult::Read(c)
                }
                None => CardResult::Skipped(
                    plan.card.clone(),
                    UnknownReason::NotExposed,
                    "not in the captured drm_info output".into(),
                ),
            },
            Err(e) => CardResult::Failed(
                plan.card.clone(),
                UnknownReason::ParseError,
                format!("drm_info: {e}"),
            ),
        },
        KmsAccess::Live => {
            let _ = root;
            match kms_live::read_card(plan) {
                Ok(c) => CardResult::Read(c),
                Err(e) => {
                    let reason = match io_reason(&e) {
                        UnknownReason::NotExposed | UnknownReason::NotGranted => {
                            UnknownReason::NotGranted
                        }
                        r => r,
                    };
                    let hint = if reason == UnknownReason::NotGranted {
                        " (Flatpak: --device=dri; Snap: the opengl interface)"
                    } else {
                        ""
                    };
                    CardResult::Failed(
                        plan.card.clone(),
                        reason,
                        format!("/dev/dri/{}: {e}{hint}", plan.card),
                    )
                }
            }
        }
    }
}

/// Plans and reads every card in scope in one go (tools and the watch's change detection).
pub(crate) fn read_kms(root: &SystemRoot, access: &KmsAccess, scope: &ProbeScope) -> KmsRead {
    if let KmsAccess::No(r, d) = access {
        return KmsRead::Unreadable(*r, d.clone());
    }
    let live = matches!(access, KmsAccess::Live);
    match kms_plan(root, scope, live) {
        Err((r, d)) => KmsRead::Unreadable(r, d),
        Ok((plans, mut out)) => {
            out.extend(plans.iter().map(|p| read_planned(root, access, p)));
            KmsRead::Cards(out)
        }
    }
}

/// A fingerprint of the live KMS state of the scope's connectors (properties and modes; no blobs
/// read), for the watch's change detection. `None` when KMS may not be read now: the session is
/// re-checked on every call.
pub(crate) fn kms_signature(scope: &ProbeScope, env: &Env) -> Option<String> {
    let access = kms_access(&SystemRoot::Live, env);
    let mut scope = scope.clone();
    // Never read blobs for a fingerprint.
    scope.fields = displaycaps_model::Selection::Only(vec![]);
    match read_kms(&SystemRoot::Live, &access, &scope) {
        KmsRead::Cards(cards) => {
            let ok: Vec<_> = cards
                .into_iter()
                .filter_map(|c| match c {
                    CardResult::Read(k) => Some(k),
                    _ => None,
                })
                .collect();
            serde_json::to_string(&ok).ok()
        }
        _ => None,
    }
}

/// KMS as normalized fact lines, for comparing this crate's live read-only KMS reader with
/// `drm_info -j` (the phase 3 gate, `scripts/kms-diff.sh`, lab only). With `drm_info_json`, the
/// facts come from that output instead of the ioctls. Live reads go through the same checks as a
/// probe (display session, cards that drive an output). Lab tooling, not part of the API.
#[doc(hidden)]
pub fn kms_facts(drm_info_json: Option<&[u8]>) -> Result<Vec<String>, String> {
    let cards = match drm_info_json {
        Some(j) => kms::from_drm_info(j)?,
        None => {
            let access = kms_access(&SystemRoot::Live, &Env::Process);
            match read_kms(&SystemRoot::Live, &access, &ProbeScope::everything()) {
                KmsRead::Cards(c) => c
                    .into_iter()
                    .map(|c| match c {
                        CardResult::Read(k) => Ok(k),
                        CardResult::Skipped(n, r, d) | CardResult::Failed(n, r, d) => {
                            Err(format!("{n}: {r:?}: {d}"))
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                KmsRead::Unreadable(r, d) => return Err(format!("{r:?}: {d}")),
                KmsRead::NotInScope => return Err("not read".into()),
            }
        }
    };
    // The library reads only the CRTCs that drive a connector; compare drm_info on the same basis.
    let mut cards = cards;
    for c in &mut cards {
        let used: Vec<u32> = c.connectors.iter().filter_map(|k| k.crtc).collect();
        c.crtcs.retain(|k| used.contains(&k.id));
    }
    let mut out = std::collections::BTreeSet::new();
    let prop = |obj: &str, p: &kms::KmsProp| match &p.enum_name {
        Some(e) => format!("{obj} prop {} = {e}", p.name),
        // Blob ids are object ids that change with every new blob: compare presence only.
        None if p.name == "EDID"
            || p.name.ends_with("_ID") && p.name != "CRTC_ID"
            || p.name == "HDR_OUTPUT_METADATA"
            || p.name == "TILE"
            || p.name == "PATH" =>
        {
            format!(
                "{obj} prop {} {}",
                p.name,
                if p.raw == 0 { "unset" } else { "set" }
            )
        }
        None => format!("{obj} prop {} = {}", p.name, p.raw),
    };
    let mode = |m: &kms::KmsMode| {
        format!(
            "{}x{}@{} clk {} h {}/{}/{} v {}/{}/{} flags {:#x} type {:#x}",
            m.hdisplay,
            m.vdisplay,
            m.refresh_millihz(),
            m.clock_khz,
            m.hsync_start,
            m.hsync_end,
            m.htotal,
            m.vsync_start,
            m.vsync_end,
            m.vtotal,
            m.flags,
            m.mode_type
        )
    };
    for c in &cards {
        out.insert(format!("{} driver {}", c.card, c.driver));
        for k in &c.connectors {
            let o = format!("{} connector {}", c.card, k.id);
            out.insert(format!(
                "{o} type {} status {} crtc {:?}",
                k.type_code, k.status, k.crtc
            ));
            for m in &k.modes {
                out.insert(format!("{o} mode {}", mode(m)));
            }
            for p in &k.props {
                out.insert(prop(&o, p));
            }
        }
        for k in &c.crtcs {
            let o = format!("{} crtc {}", c.card, k.id);
            out.insert(format!(
                "{o} mode {}",
                k.mode.as_ref().map(mode).unwrap_or_else(|| "none".into())
            ));
            for p in &k.props {
                out.insert(prop(&o, p));
            }
        }
    }
    Ok(out.into_iter().collect())
}

/// Reads sysfs for the scope: the tree, then EDIDs and parameters as the scope asks.
pub(crate) fn read_sysfs(root: &SystemRoot, scope: &ProbeScope) -> SysRead {
    let mut out = SysRead::default();
    let tree = sysfs::scan(root).map_err(|e| (io_reason(&e), format!("{}: {e}", sysfs::CLASS)));
    if let Ok((cards, conns)) = &tree {
        if scope.layers.includes(&LayerKind::SinkChain) && wants_sink(scope) || link_in_scope(scope)
        {
            for c in conns.iter().filter(|c| connector_in_scope(scope, c)) {
                if c.status == Some(ConnectionStatus::Connected) {
                    out.edids.insert(
                        c.name.clone(),
                        root.read(&c.edid_path())
                            .map_err(|e| (io_reason(&e), e.to_string())),
                    );
                }
            }
        }
        let driver_state = scope.layers.includes(&LayerKind::KernelDriver)
            && scope.fields.includes(&FieldGroup::Driver)
            || link_in_scope(scope);
        if driver_state {
            out.kernel = root
                .read_string("/proc/sys/kernel/osrelease")
                .ok()
                .map(|s| s.trim().to_string());
        }
        if driver_state {
            for card in cards {
                let Some(driver) = &card.driver else { continue };
                for (module, param) in sysfs::display_parameters(driver) {
                    let path = format!("/sys/module/{module}/parameters/{param}");
                    if let Ok(v) = root.read_string(&path) {
                        let list = out.params.entry(card.name.clone()).or_default();
                        let name = format!("{module}.{param}");
                        if !list.iter().any(|p| p.name == name) {
                            list.push(ModuleParameter {
                                name,
                                value: v.trim().to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
    out.tree = Some(tree);
    out
}

fn wants_sink(scope: &ProbeScope) -> bool {
    [FieldGroup::Sink, FieldGroup::Identity]
        .iter()
        .any(|g| scope.fields.includes(g))
}

fn connector_in_scope(scope: &ProbeScope, c: &SysConnector) -> bool {
    let class = sysfs::class(&sysfs::connector_type(&c.kind));
    scope.includes_endpoint(class, &EndpointId(c.name.clone()))
}

/// Loads quirk data: the bundled pack, then `/etc/displaycaps/quirks/*.json`, then the app's
/// directories, later ones overriding earlier ones.
pub(crate) fn quirks(root: &SystemRoot, opts: &ProbeOptions) -> QuirkSet {
    let mut texts: Vec<(String, String)> = vec![];
    let sys = "/etc/displaycaps/quirks";
    if let Ok(entries) = root.read_dir(sys) {
        for e in entries.iter().filter(|e| e.name.ends_with(".json")) {
            let p = format!("{sys}/{}", e.name);
            if let Ok(t) = root.read_string(&p) {
                texts.push((p, t));
            }
        }
    }
    for dir in &opts.quirk_dirs {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        for p in files {
            if let Ok(t) = std::fs::read_to_string(&p) {
                texts.push((p.display().to_string(), t));
            }
        }
    }
    let bundled = QuirkSet::bundled();
    if texts.is_empty() {
        return bundled;
    }
    let mut packs: Vec<(String, &str)> =
        vec![("bundled".into(), displaycaps_rules::quirks::BUNDLED)];
    packs.extend(texts.iter().map(|(o, t)| (o.clone(), t.as_str())));
    QuirkSet::load(packs)
}

fn prov(layer: LayerKind, source: Source, at: u64) -> Provenance {
    Provenance {
        layer,
        source,
        read_at_ms: at,
    }
}

/// The KMS connector for a sysfs connector: by `connector_id` when sysfs has it, else by name.
fn kms_match<'a>(kms: &'a KmsRead, c: &SysConnector) -> Option<(&'a KmsCard, &'a KmsConnector)> {
    let KmsRead::Cards(cards) = kms else {
        return None;
    };
    let card = cards.iter().find_map(|r| match r {
        CardResult::Read(k) if k.card == c.card => Some(k),
        _ => None,
    })?;
    let conn = match c.connector_id {
        Some(id) => card.connectors.iter().find(|k| k.id == id),
        None => {
            // Without connector_id, names match only when the card has one connector of this type
            // and index 1: KMS type ids aren't in drm_info's output to compare.
            let same: Vec<_> = card
                .connectors
                .iter()
                .filter(|k| sysfs::kind_for_code(k.type_code) == Some(c.kind.as_str()))
                .collect();
            (same.len() == 1 && c.index == Some(1)).then(|| same[0])
        }
    }?;
    Some((card, conn))
}

/// Builds the endpoints in scope, and the availability of each endpoint layer.
pub(crate) fn endpoints(
    scope: &ProbeScope,
    sys: &SysRead,
    kms: &KmsRead,
    quirks: &QuirkSet,
    now: u64,
) -> (Vec<Endpoint>, BTreeMap<LayerKind, Availability>) {
    let mut avail = BTreeMap::new();
    let lay = |l: LayerKind| scope.layers.includes(&l);
    let field = |g: FieldGroup| scope.fields.includes(&g);
    let (cards, conns) = match &sys.tree {
        Some(Ok((c, n))) => (c.as_slice(), n.as_slice()),
        other => {
            let (reason, detail) = match other {
                Some(Err((r, e))) => (*r, e.clone()),
                _ => (UnknownReason::Timeout, "sysfs not read".into()),
            };
            for l in ENDPOINT_LAYERS.iter().filter(|l| lay(**l)) {
                // KMS (and the link inferred from it) has its own source.
                if !matches!(l, LayerKind::Kms | LayerKind::Link) {
                    avail.insert(
                        *l,
                        Availability::Unknown {
                            reason,
                            detail: Some(detail.clone()),
                        },
                    );
                }
            }
            for l in [LayerKind::Kms, LayerKind::Link] {
                if lay(l) {
                    avail.insert(l, kms_availability(kms));
                }
            }
            return (vec![], avail);
        }
    };
    let file = |p: String| Source::file(p);
    let mut out = vec![];
    let mut edid_trouble = false;
    for c in conns {
        let ctype = sysfs::connector_type(&c.kind);
        let ctype_for_chain = ctype.clone();
        let class = sysfs::class(&ctype);
        let id = EndpointId(c.name.clone());
        if !scope.includes_endpoint(class, &id) {
            continue;
        }
        let card = cards.iter().find(|k| k.name == c.card);
        let km = kms_match(kms, c);
        let mut ep = Endpoint {
            id,
            class,
            connector: Value::measured(ctype, prov(LayerKind::HostHardware, file(c.dir()), now)),
            status: match c.status {
                Some(s) => Value::measured(
                    s,
                    prov(
                        LayerKind::HostHardware,
                        file(format!("{}/status", c.dir())),
                        now,
                    ),
                ),
                None => Value::unknown(UnknownReason::ReadError, format!("{}/status", c.dir())),
            },
            name: None,
            correlation: vec![],
            sink: None,
            layers: vec![],
            effective: vec![],
            paths: vec![],
        };
        // The sink, from the cached EDID (sysfs first, then the KMS blob).
        let edid_src: Option<(&[u8], Source)> = match (
            sys.edids.get(&c.name),
            km.and_then(|(_, k)| k.prop("EDID"))
                .and_then(|p| p.blob.as_ref()),
        ) {
            (Some(Ok(raw)), _) if !raw.is_empty() => Some((raw.as_slice(), file(c.edid_path()))),
            (_, Some(b)) if !b.is_empty() => Some((
                b.as_slice(),
                Source::KmsProperty {
                    object: format!("connector {}", km.map(|(_, k)| k.id).unwrap_or(0)),
                    property: "EDID".into(),
                },
            )),
            _ => None,
        };
        // Every field, for the link and for devices in the chain; reported only through `sink`.
        let sink_full = (lay(LayerKind::Link) || lay(LayerKind::SinkChain))
            .then(|| {
                edid_src.as_ref().and_then(|(raw, src)| {
                    crate::sink::sink_record(raw, src.clone(), now, &Selection::All, quirks)
                })
            })
            .flatten();
        let mst = km
            .and_then(|(_, k)| k.prop("PATH"))
            .is_some_and(|p| p.raw != 0);
        let driver_name: Option<String> = km
            .map(|(k, _)| k.driver.clone())
            .filter(|d| !d.is_empty())
            .or_else(|| card.and_then(|k| k.driver.clone()));
        let params = sys.params.get(&c.card).cloned().unwrap_or_default();
        let frl = link::frl_gate(
            driver_name.as_deref(),
            sys.kernel.as_deref().and_then(link::kernel_version),
            &params,
        );
        let mut sink_key = None;
        if lay(LayerKind::SinkChain) {
            let mut layer = LayerCapabilities {
                layer: LayerKind::SinkChain,
                availability: Availability::Read,
                caps: CapabilitySet::default(),
                detail: None,
            };
            match (
                sys.edids.get(&c.name),
                km.and_then(|(_, k)| k.prop("EDID"))
                    .and_then(|p| p.blob.as_ref()),
            ) {
                (Some(Ok(raw)), _) if !raw.is_empty() => {
                    ep.sink = crate::sink::sink_record(
                        raw,
                        file(c.edid_path()),
                        now,
                        &scope.fields,
                        quirks,
                    );
                }
                (_, Some(blob)) if !blob.is_empty() => {
                    let src = Source::KmsProperty {
                        object: format!("connector {}", km.map(|(_, k)| k.id).unwrap_or(0)),
                        property: "EDID".into(),
                    };
                    ep.sink = crate::sink::sink_record(blob, src, now, &scope.fields, quirks);
                }
                (Some(Err((reason, e))), _) => {
                    edid_trouble = true;
                    layer.availability = Availability::Unknown {
                        reason: *reason,
                        detail: Some(format!("{}: {e}", c.edid_path())),
                    };
                }
                _ if c.status == Some(ConnectionStatus::Connected) && wants_sink(scope) => {
                    // Connected with an empty EDID: the kernel has none cached (e.g. a VM display
                    // without one, or a sink that didn't answer).
                    layer.availability = Availability::Unknown {
                        reason: UnknownReason::NotExposed,
                        detail: Some("the kernel has no EDID cached for this connector".into()),
                    };
                }
                _ => {}
            }
            if let Some(s) = &mut ep.sink {
                let devices = link::chain(&ctype_for_chain, sink_full.as_ref(), mst);
                if !devices.is_empty() {
                    s.chain = Value::inferred(
                        devices,
                        prov(
                            LayerKind::SinkChain,
                            Source::derived("connector type, sink EDID and MST path"),
                            now,
                        ),
                    );
                }
            }
            if let Some(s) = &ep.sink {
                sink_key = s.identity.sink_key.known().cloned();
                layer.caps = s.video.clone();
                if !field(FieldGroup::Sink) {
                    // Identity was asked for, the sink record was not.
                    ep.sink = None;
                    layer.caps = CapabilitySet::default();
                }
            }
            ep.layers.push(layer);
        }
        if lay(LayerKind::HostHardware) {
            let detail = field(FieldGroup::Hardware).then(|| {
                LayerDetail::HostHardware(HostDetail {
                    gpu: match card {
                        Some(k) => Value::measured(
                            Gpu {
                                card: k.name.clone(),
                                pci_vendor: k.pci_vendor,
                                pci_device: k.pci_device,
                                pci_slot: k.pci_slot.clone(),
                                vendor_name: k
                                    .pci_vendor
                                    .and_then(sysfs::vendor_name)
                                    .map(str::to_string),
                                driver: k.driver.clone(),
                            },
                            prov(
                                LayerKind::HostHardware,
                                file(format!("{}/{}/device", sysfs::CLASS, k.name)),
                                now,
                            ),
                        ),
                        None => Value::unknown(
                            UnknownReason::ReadError,
                            format!("{} not listed", c.card),
                        ),
                    },
                    connector_index: c.index,
                })
            });
            ep.layers.push(LayerCapabilities {
                layer: LayerKind::HostHardware,
                availability: Availability::Read,
                caps: CapabilitySet::default(),
                detail,
            });
        }
        if lay(LayerKind::KernelDriver) {
            let detail = field(FieldGroup::Driver).then(|| {
                let kms_driver = km.map(|(k, _)| k).filter(|k| !k.driver.is_empty());
                LayerDetail::KernelDriver(DriverDetail {
                    driver: match (kms_driver, card.and_then(|k| k.driver.clone())) {
                        (Some(k), _) => Value::measured(
                            k.driver.clone(),
                            prov(
                                LayerKind::KernelDriver,
                                match k.from {
                                    kms::KmsFrom::Live => Source::KmsProperty {
                                        object: k.card.clone(),
                                        property: "DRM_IOCTL_VERSION".into(),
                                    },
                                    kms::KmsFrom::DrmInfo => Source::Command {
                                        name: "drm_info".into(),
                                    },
                                },
                                now,
                            ),
                        ),
                        (None, Some(d)) => Value::measured(
                            d,
                            prov(
                                LayerKind::KernelDriver,
                                file(format!("{}/{}/device/driver", sysfs::CLASS, c.card)),
                                now,
                            ),
                        ),
                        (None, None) => {
                            Value::unknown(UnknownReason::NotExposed, "no driver bound".to_string())
                        }
                    },
                    parameters: Value::measured(
                        sys.params.get(&c.card).cloned().unwrap_or_default(),
                        prov(
                            LayerKind::KernelDriver,
                            file("/sys/module".to_string()),
                            now,
                        ),
                    ),
                    features: link::frl_feature(&frl, now).into_iter().collect(),
                })
            });
            ep.layers.push(LayerCapabilities {
                layer: LayerKind::KernelDriver,
                availability: Availability::Read,
                caps: CapabilitySet::default(),
                detail,
            });
        }
        if lay(LayerKind::Kms) {
            let layer = match (kms, km) {
                (_, Some((card, conn))) => {
                    let mut caps = CapabilitySet::default();
                    if field(FieldGroup::Modes) {
                        caps.modes = Some(Value::measured(
                            kms::modes(conn),
                            Provenance {
                                layer: LayerKind::Kms,
                                source: match card.from {
                                    kms::KmsFrom::Live => Source::KmsProperty {
                                        object: format!("connector {}", conn.id),
                                        property: "modes (cached, not probed)".into(),
                                    },
                                    kms::KmsFrom::DrmInfo => Source::Command {
                                        name: "drm_info".into(),
                                    },
                                },
                                read_at_ms: now,
                            },
                        ));
                    }
                    LayerCapabilities {
                        layer: LayerKind::Kms,
                        availability: Availability::Read,
                        caps,
                        detail: Some(LayerDetail::Kms(kms::kms_state(
                            card,
                            conn,
                            &scope.fields,
                            now,
                        ))),
                    }
                }
                (KmsRead::Cards(cards), None) => LayerCapabilities {
                    layer: LayerKind::Kms,
                    availability: match cards.iter().find(|r| r.card() == c.card) {
                        Some(CardResult::Skipped(_, r, d) | CardResult::Failed(_, r, d)) => {
                            Availability::Unknown {
                                reason: *r,
                                detail: Some(d.clone()),
                            }
                        }
                        _ => Availability::Unknown {
                            reason: UnknownReason::NotExposed,
                            detail: Some("no KMS connector matches this sysfs connector".into()),
                        },
                    },
                    caps: CapabilitySet::default(),
                    detail: None,
                },
                (other, None) => LayerCapabilities {
                    layer: LayerKind::Kms,
                    availability: kms_availability(other),
                    caps: CapabilitySet::default(),
                    detail: None,
                },
            };
            ep.layers.push(layer);
        }
        if lay(LayerKind::Link) {
            let kms_full = km.map(|(card, conn)| kms::kms_state(card, conn, &Selection::All, now));
            let layer = match (kms, &kms_full) {
                (_, Some(k)) => link::link_layer(&link::Inputs {
                    connector: &ctype_for_chain,
                    sink: sink_full.as_ref(),
                    kms: Some(k),
                    mst,
                    driver: driver_name.as_deref(),
                    frl: frl.clone(),
                    with_detail: field(FieldGroup::Link),
                    now,
                }),
                (other, None) => LayerCapabilities {
                    layer: LayerKind::Link,
                    availability: match kms_availability(other) {
                        Availability::Read | Availability::Partial => Availability::Unknown {
                            reason: UnknownReason::NotExposed,
                            detail: Some("no KMS state for this connector".into()),
                        },
                        a => a,
                    },
                    caps: CapabilitySet::default(),
                    detail: None,
                },
            };
            ep.layers.push(layer);
        }
        if field(FieldGroup::Identity) {
            let measured = |id| CorrelationKey {
                id,
                confidence: Confidence::Measured,
            };
            ep.correlation.push(measured(CorrelationId::DrmConnector {
                name: c.name.clone(),
            }));
            let kid = km.map(|(_, k)| k.id).or(c.connector_id);
            if let Some(id) = kid {
                ep.correlation.push(measured(CorrelationId::KmsConnector {
                    card: c.card.clone(),
                    id,
                }));
            }
            if let Some(crtc) = km.and_then(|(_, k)| k.crtc) {
                ep.correlation.push(measured(CorrelationId::KmsCrtc {
                    card: c.card.clone(),
                    id: crtc,
                }));
            }
            if let Some(key) = sink_key {
                ep.correlation
                    .push(measured(CorrelationId::SinkKey { key }));
            }
        }
        out.push(ep);
    }
    for l in [
        LayerKind::SinkChain,
        LayerKind::HostHardware,
        LayerKind::KernelDriver,
    ] {
        if lay(l) {
            let a = if l == LayerKind::SinkChain && edid_trouble {
                Availability::Partial
            } else {
                Availability::Read
            };
            avail.insert(l, a);
        }
    }
    if lay(LayerKind::Kms) {
        avail.insert(LayerKind::Kms, kms_availability(kms));
    }
    if lay(LayerKind::Link) {
        // Inferred from KMS: as readable as KMS is.
        avail.insert(LayerKind::Link, kms_availability(kms));
    }
    (out, avail)
}

fn kms_availability(kms: &KmsRead) -> Availability {
    match kms {
        KmsRead::NotInScope => Availability::Unknown {
            reason: UnknownReason::NotRequested,
            detail: None,
        },
        KmsRead::Unreadable(r, d) => Availability::Unknown {
            reason: *r,
            detail: Some(d.clone()),
        },
        KmsRead::Cards(c) => {
            let read = c
                .iter()
                .filter(|r| matches!(r, CardResult::Read(_)))
                .count();
            match c.iter().find(|r| !matches!(r, CardResult::Read(_))) {
                None => Availability::Read,
                Some(_) if read > 0 => Availability::Partial,
                Some(CardResult::Skipped(_, r, d) | CardResult::Failed(_, r, d)) => {
                    Availability::Unknown {
                        reason: *r,
                        detail: Some(d.clone()),
                    }
                }
                Some(CardResult::Read(_)) => Availability::Read,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempFixture;

    fn lab() -> SystemRoot {
        SystemRoot::fixture(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/lab-gnome"
        ))
        .unwrap()
    }

    #[test]
    fn seat0_session_type() {
        let f = TempFixture::new("ID=debian\n");
        let root = || SystemRoot::fixture(f.path()).unwrap();
        assert_eq!(seat0_graphical(&root()), None, "no logind state: unknown");
        let run = f.path().join("root/run/systemd");
        std::fs::create_dir_all(run.join("seats")).unwrap();
        std::fs::create_dir_all(run.join("sessions")).unwrap();
        std::fs::write(run.join("seats/seat0"), "IS_SEAT0=1\nACTIVE=3\n").unwrap();
        std::fs::write(run.join("sessions/3"), "TYPE=wayland\nSTATE=active\n").unwrap();
        assert_eq!(seat0_graphical(&root()), Some(true));
        std::fs::write(run.join("sessions/3"), "TYPE=tty\nSTATE=active\n").unwrap();
        assert_eq!(
            seat0_graphical(&root()),
            Some(false),
            "switched to a text console"
        );
        std::fs::write(run.join("seats/seat0"), "IS_SEAT0=1\nACTIVE=\n").unwrap();
        assert_eq!(seat0_graphical(&root()), Some(false));
        std::fs::write(run.join("seats/seat0"), "ACTIVE=../../etc/x\n").unwrap();
        assert_eq!(seat0_graphical(&root()), Some(false));
    }

    #[test]
    fn plan_follows_scope_and_skips_idle_cards() {
        let all = ProbeScope::everything();
        let (plans, skipped) = kms_plan(&lab(), &all, true).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].connectors, Some(vec![37]));
        assert!(plans[0].hdr);
        assert!(plans[0].edid_for.is_empty(), "the sysfs EDID is readable");
        // Out-of-scope endpoints: the card isn't planned at all.
        let ext = ProbeScope::nothing()
            .with_class(displaycaps_model::EndpointClass::External)
            .with_layer(LayerKind::Kms);
        assert_eq!(kms_plan(&lab(), &ext, true).unwrap().0.len(), 0);
        // A card that drives no output is never opened on the live system.
        let f = TempFixture::new("ID=debian\n");
        let c = f.path().join("root/sys/class/drm/card0-HDMI-A-1");
        std::fs::create_dir_all(f.path().join("root/sys/class/drm/card0")).unwrap();
        std::fs::create_dir_all(&c).unwrap();
        std::fs::write(c.join("status"), "connected\n").unwrap();
        std::fs::write(c.join("enabled"), "disabled\n").unwrap();
        std::fs::write(c.join("connector_id"), "98\n").unwrap();
        let root = SystemRoot::fixture(f.path()).unwrap();
        let (plans, skipped) = kms_plan(&root, &all, true).unwrap();
        assert!(plans.is_empty());
        assert!(
            matches!(&skipped[0], CardResult::Skipped(c, UnknownReason::NotExposed, _) if c == "card0")
        );
    }
}
