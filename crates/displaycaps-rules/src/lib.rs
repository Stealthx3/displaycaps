// SPDX-License-Identifier: LGPL-2.1-or-later
// Adapted from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-rules/src/lib.rs; the audio
// analysis (access paths, can-play queries) is dropped, the findings pass is kept.
//! DisplayCaps declarative data and analysis.
//!
//! - [`quirks`]: sink quirk data, matched by manufacturer, product and model name.
//! - [`facts`] and [`engine`]: declarative rule packs over facts derived from a snapshot; each rule
//!   that fires becomes a [`displaycaps_model::Finding`] with evidence and a compositor- or
//!   distro-aware hint. The starter display rule pack arrives in phase 7.
//!
//! Packs are bundled with each release and can be overridden from a system directory or the app's
//! bundle; nothing is fetched from the network. This crate does no I/O: callers pass pack text in.

pub mod engine;
pub mod facts;
pub mod quirks;

pub use engine::{RULE_PACK_FORMAT, RuleSet};

use displaycaps_model::{FieldGroup, Snapshot};

use crate::engine::RuleScope;

/// Computes the findings the snapshot's scope asks for and stores them in `s.findings`. System
/// rules run once; endpoint rules run once per endpoint in the snapshot (a rule marked `once`
/// reports only the first endpoint it fires on).
pub fn analyze(s: &mut Snapshot, rules: &RuleSet) {
    if !s.scope.fields.includes(&FieldGroup::Findings) {
        return;
    }
    let g = facts::global(s);
    let family = s
        .environment
        .as_ref()
        .and_then(|e| e.os.known())
        .map(|o| o.family);
    let compositor = s
        .environment
        .as_ref()
        .and_then(|e| e.session.known())
        .and_then(|x| x.compositor.as_ref())
        .map(|c| c.kind);
    let at = s.taken_at_ms;
    let mut findings = vec![];
    for (rule, _) in &rules.rules {
        if rule.scope == RuleScope::System {
            if let Some(f) = engine::apply(rule, &g, family, compositor, at) {
                findings.push(f);
            }
            continue;
        }
        for e in &s.endpoints {
            let mut f = g.clone();
            f.extend(&facts::endpoint(e));
            if let Some(mut x) = engine::apply(rule, &f, family, compositor, at) {
                x.endpoint = Some(e.id.clone());
                findings.push(x);
                if rule.once {
                    break;
                }
            }
        }
    }
    findings.sort_by(|a, b| {
        (std::cmp::Reverse(a.severity), &a.code.0, &a.endpoint).cmp(&(
            std::cmp::Reverse(b.severity),
            &b.code.0,
            &b.endpoint,
        ))
    });
    s.findings = findings;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use displaycaps_model::{
        ConnectionStatus, ConnectorType, DistroFamily, Endpoint, EndpointClass, EndpointId,
        EnvironmentInfo, LayerKind, OsInfo, ProbeScope, Provenance, RedactionPolicy,
        SCHEMA_VERSION, Sandbox, SessionInfo, SessionKind, Severity, SnapshotSource, Source, Value,
    };

    fn prov() -> Provenance {
        Provenance {
            layer: LayerKind::Environment,
            source: Source::derived("test"),
            read_at_ms: 0,
        }
    }

    pub(crate) fn snapshot_with_sandbox(sandbox: Sandbox) -> Snapshot {
        Snapshot {
            schema_version: SCHEMA_VERSION.into(),
            generation: 1,
            taken_at_ms: 7,
            source: SnapshotSource::Live,
            scope: ProbeScope::everything(),
            redaction: RedactionPolicy::default(),
            layers: vec![],
            environment: Some(EnvironmentInfo {
                os: Value::measured(
                    OsInfo {
                        id: "debian".into(),
                        version_id: None,
                        family: DistroFamily::Debian,
                    },
                    prov(),
                ),
                kernel: Value::measured("6.12".into(), prov()),
                sandbox: Value::measured(sandbox, prov()),
                session: Value::inferred(
                    SessionInfo {
                        kind: SessionKind::Wayland,
                        compositor: None,
                        protocols: vec![],
                    },
                    prov(),
                ),
                graphics_driver: None,
            }),
            endpoints: vec![],
            content: None,
            recommendations: vec![],
            findings: vec![],
        }
    }

    fn endpoint(id: &str, connector: ConnectorType) -> Endpoint {
        Endpoint {
            id: EndpointId(id.into()),
            class: EndpointClass::External,
            connector: Value::measured(connector, prov()),
            status: Value::measured(ConnectionStatus::Connected, prov()),
            name: None,
            correlation: vec![],
            sink: None,
            layers: vec![],
            effective: vec![],
            paths: vec![],
        }
    }

    #[test]
    fn bundled_sandbox_rule_fires_on_a_missing_required_permission() {
        let gap = displaycaps_model::PermissionGap {
            permission: "--device=dri".into(),
            layers: vec![LayerKind::Kms, LayerKind::Link],
            fix: "flatpak override --user --device=dri org.example.Player".into(),
            required: true,
        };
        let mut s = snapshot_with_sandbox(Sandbox::Flatpak {
            granted: vec![],
            missing: vec![gap],
        });
        analyze(&mut s, &RuleSet::bundled());
        assert_eq!(s.findings.len(), 1);
        let f = &s.findings[0];
        assert_eq!(f.code.0, "SANDBOX_PERMISSION_MISSING");
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.endpoint, None);
        assert!(f.hint.as_ref().unwrap().text.contains("--device=dri"));
        assert!(f.evidence.iter().any(|e| e.label == "g.sandbox.kind"));

        // Not sandboxed: nothing fires.
        let mut s = snapshot_with_sandbox(Sandbox::None);
        analyze(&mut s, &RuleSet::bundled());
        assert!(s.findings.is_empty());
    }

    #[test]
    fn findings_are_computed_only_when_in_scope() {
        let mut s = snapshot_with_sandbox(Sandbox::Flatpak {
            granted: vec![],
            missing: vec![displaycaps_model::PermissionGap {
                permission: "--device=dri".into(),
                layers: vec![LayerKind::Kms],
                fix: "x".into(),
                required: true,
            }],
        });
        s.scope = ProbeScope::nothing().with_field(FieldGroup::Hdr);
        analyze(&mut s, &RuleSet::bundled());
        assert!(s.findings.is_empty());
    }

    #[test]
    fn endpoint_rules_run_per_endpoint_and_once_stops_early() {
        let pack = r#"{"format":1,"rules":[
            {"code":"PER_ENDPOINT","severity":"info","hop":"sink","when":{"eq":["e.connector","hdmi"]},"evidence":["e.connector"]},
            {"code":"ONCE","severity":"warning","hop":"sink","once":true,"when":{"eq":["e.connector","hdmi"]},"evidence":["e.connector"]}]}"#;
        let rules = RuleSet::load([("t".to_string(), pack)]);
        assert!(rules.issues.is_empty(), "{:?}", rules.issues);
        let mut s = snapshot_with_sandbox(Sandbox::None);
        s.endpoints = vec![
            endpoint("drm:card1-DP-1", ConnectorType::DisplayPort),
            endpoint("drm:card1-HDMI-A-1", ConnectorType::Hdmi),
            endpoint("drm:card1-HDMI-A-2", ConnectorType::Hdmi),
        ];
        analyze(&mut s, &rules);
        let got: Vec<_> = s
            .findings
            .iter()
            .map(|f| (f.code.0.as_str(), f.endpoint.as_ref().unwrap().0.as_str()))
            .collect();
        // Sorted: higher severity first, then code, then endpoint.
        assert_eq!(
            got,
            [
                ("ONCE", "drm:card1-HDMI-A-1"),
                ("PER_ENDPOINT", "drm:card1-HDMI-A-1"),
                ("PER_ENDPOINT", "drm:card1-HDMI-A-2"),
            ]
        );
    }
}
