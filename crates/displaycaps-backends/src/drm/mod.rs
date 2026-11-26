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
pub(crate) mod sysfs;
pub(crate) mod uevent;

use std::collections::BTreeMap;

use displaycaps_model::{
    Availability, CapabilitySet, Confidence, ConnectionStatus, CorrelationId, CorrelationKey,
    DriverDetail, Endpoint, EndpointId, FieldGroup, Gpu, HostDetail, LayerCapabilities,
    LayerDetail, LayerKind, ModuleParameter, ProbeScope, Provenance, SessionKind, Source,
    UnknownReason, Value,
};
use displaycaps_rules::quirks::QuirkSet;

use crate::environment::Env;
use crate::root::SystemRoot;
use crate::runtime::ProbeOptions;
use kms::{KmsCard, KmsConnector};
use sysfs::{SysCard, SysConnector};

/// The layers an endpoint is built from. If none is in scope, `/sys/class/drm` is not read.
pub(crate) const ENDPOINT_LAYERS: [LayerKind; 4] = [
    LayerKind::SinkChain,
    LayerKind::HostHardware,
    LayerKind::KernelDriver,
    LayerKind::Kms,
];

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
    pub edids: BTreeMap<String, Result<Vec<u8>, String>>,
    /// Module parameters per module (driver detail only).
    pub params: BTreeMap<String, Vec<ModuleParameter>>,
}

/// What KMS gave.
#[derive(Debug, Clone)]
pub(crate) enum KmsRead {
    /// Not read: the KMS layer is not in scope.
    NotInScope,
    /// Not readable, with the reason.
    Unreadable(UnknownReason, String),
    /// Cards read (a card that failed is listed with its error).
    Cards(Vec<Result<KmsCard, (String, String)>>),
}

/// Whether KMS may be read here and how. On the live system the primary node is opened only
/// inside a Wayland or X11 session (the compositor is DRM master; opening the node with no master
/// would make this process master implicitly).
pub(crate) enum KmsAccess {
    /// Read the live cards.
    Live,
    /// Replay the fixture's `drm_info -j` output.
    Fixture(Vec<u8>),
    /// Not readable.
    No(UnknownReason, String),
}

pub(crate) fn kms_access(root: &SystemRoot, env: &Env) -> KmsAccess {
    match root {
        SystemRoot::Fixture(_) => match root.captured_command("drm_info") {
            Some((_, Some(out))) => KmsAccess::Fixture(out),
            _ => KmsAccess::No(
                UnknownReason::NotExposed,
                "the capture has no drm_info output (taken outside a display session)".into(),
            ),
        },
        SystemRoot::Live => match crate::environment::session_kind(env) {
            SessionKind::Wayland | SessionKind::X11 => KmsAccess::Live,
            _ => KmsAccess::No(
                UnknownReason::NoSession,
                "no display session: opening /dev/dri/card* would make this process DRM master"
                    .into(),
            ),
        },
    }
}

/// Reads KMS for the cards on this system.
pub(crate) fn read_kms(root: &SystemRoot, access: KmsAccess, blobs: &[&str]) -> KmsRead {
    match access {
        KmsAccess::No(r, d) => KmsRead::Unreadable(r, d),
        KmsAccess::Fixture(json) => match kms::from_drm_info(&json) {
            Ok(cards) => KmsRead::Cards(cards.into_iter().map(Ok).collect()),
            Err(e) => KmsRead::Unreadable(UnknownReason::ParseError, format!("drm_info: {e}")),
        },
        KmsAccess::Live => {
            let names: Vec<String> = match root.read_dir(sysfs::CLASS) {
                Ok(v) => v
                    .into_iter()
                    .map(|e| e.name)
                    .filter(|n| {
                        n.strip_prefix("card")
                            .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
                    })
                    .collect(),
                Err(e) => {
                    return KmsRead::Unreadable(
                        UnknownReason::ReadError,
                        format!("{}: {e}", sysfs::CLASS),
                    );
                }
            };
            let cards: Vec<_> = names
                .iter()
                .map(|n| kms_live::read_card(n, blobs).map_err(|e| (n.clone(), e.to_string())))
                .collect();
            if !cards.is_empty() && cards.iter().all(Result::is_err) {
                let (reason, what) = match &cards[0] {
                    Err((_, e)) if e.contains("ermission") || e.contains("No such file") => (
                        UnknownReason::NotGranted,
                        "no access to /dev/dri (Flatpak: --device=dri; Snap: the opengl interface)",
                    ),
                    _ => (UnknownReason::ReadError, "every card failed"),
                };
                let detail = cards
                    .iter()
                    .filter_map(|c| c.as_ref().err().map(|(n, e)| format!("{n}: {e}")))
                    .collect::<Vec<_>>()
                    .join("; ");
                return KmsRead::Unreadable(reason, format!("{what}: {detail}"));
            }
            KmsRead::Cards(cards)
        }
    }
}

/// A fingerprint of the live KMS state of every card (properties and modes; no blobs read), for
/// the watch's change detection. Read-only, like every KMS read.
pub(crate) fn kms_signature() -> Option<String> {
    match read_kms(&SystemRoot::Live, KmsAccess::Live, &[]) {
        KmsRead::Cards(cards) => {
            let ok: Vec<_> = cards.into_iter().flatten().collect();
            serde_json::to_string(&ok).ok()
        }
        _ => None,
    }
}

/// KMS as normalized fact lines, for comparing this crate's live read-only KMS reader with
/// `drm_info -j` (the phase 3 gate, `scripts/kms-diff.sh`, lab only). With `drm_info_json`, the
/// facts come from that output instead of the ioctls. Live reads need a display session.
pub fn kms_facts(drm_info_json: Option<&[u8]>) -> Result<Vec<String>, String> {
    let cards = match drm_info_json {
        Some(j) => kms::from_drm_info(j)?,
        None => match read_kms(&SystemRoot::Live, KmsAccess::Live, &[]) {
            KmsRead::Cards(c) => c
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|(n, e)| format!("{n}: {e}"))?,
            KmsRead::Unreadable(r, d) => return Err(format!("{r:?}: {d}")),
            KmsRead::NotInScope => return Err("not read".into()),
        },
    };
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
    let tree = sysfs::scan(root).map_err(|e| {
        let reason = match e.kind() {
            std::io::ErrorKind::NotFound => UnknownReason::NotExposed,
            std::io::ErrorKind::PermissionDenied => UnknownReason::NotGranted,
            _ => UnknownReason::ReadError,
        };
        (reason, format!("{}: {e}", sysfs::CLASS))
    });
    if let Ok((cards, conns)) = &tree {
        if scope.layers.includes(&LayerKind::SinkChain) && wants_sink(scope) {
            for c in conns.iter().filter(|c| connector_in_scope(scope, c)) {
                if c.status == Some(ConnectionStatus::Connected) {
                    out.edids.insert(
                        c.name.clone(),
                        root.read(&c.edid_path()).map_err(|e| e.to_string()),
                    );
                }
            }
        }
        if scope.layers.includes(&LayerKind::KernelDriver)
            && scope.fields.includes(&FieldGroup::Driver)
        {
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
    let card = cards.iter().flatten().find(|k| k.card == c.card)?;
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
                // KMS has its own source; it stays readable without sysfs.
                if *l != LayerKind::Kms {
                    avail.insert(
                        *l,
                        Availability::Unknown {
                            reason,
                            detail: Some(detail.clone()),
                        },
                    );
                }
            }
            if lay(LayerKind::Kms) {
                avail.insert(LayerKind::Kms, kms_availability(kms));
            }
            return (vec![], avail);
        }
    };
    let file = |p: String| Source::file(p);
    let mut out = vec![];
    let mut edid_trouble = false;
    for c in conns {
        let ctype = sysfs::connector_type(&c.kind);
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
                (Some(Err(e)), _) => {
                    edid_trouble = true;
                    layer.availability = Availability::Unknown {
                        reason: UnknownReason::NotGranted,
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
                    features: vec![],
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
                (KmsRead::Cards(_), None) => LayerCapabilities {
                    layer: LayerKind::Kms,
                    availability: Availability::Unknown {
                        reason: UnknownReason::NotExposed,
                        detail: Some("no KMS connector matches this sysfs connector".into()),
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
        KmsRead::Cards(c) if c.iter().any(Result::is_err) => Availability::Partial,
        KmsRead::Cards(_) => Availability::Read,
    }
}
