// SPDX-License-Identifier: LGPL-2.1-or-later
// Adapted from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-backends/src/probe.rs; reduced to
// the environment layer. The other layers report unknown until their backends exist.
//! The scoped probe: runs the backends the scope asks for as tasks on the runtime's executor,
//! each with its own deadline, and assembles a [`Snapshot`].
//!
//! Only the environment layer has a backend so far. Every other layer in scope is reported
//! `unknown` with reason `not_implemented`; no endpoint is discovered, so `endpoints` is empty.
//! Nothing outside the scope is read: the environment is read only when the environment layer is
//! in scope, and a layer outside the scope is not mentioned at all.

use displaycaps_model::{
    Availability, EnvironmentInfo, LayerKind, ProbeScope, RedactionPolicy, SCHEMA_VERSION,
    Snapshot, SnapshotSource, SystemLayer, UnknownReason,
};

use crate::environment::{self, Env};
use crate::root::SystemRoot;
use crate::runtime::{self, Backend, ProbeOptions, Task};

/// Probes `root` within `scope` with no deadlines (fixtures, tests, tools). Applications use
/// [`crate::Runtime`], which applies per-backend deadlines.
pub fn probe(root: &SystemRoot, scope: &ProbeScope) -> Snapshot {
    probe_with(root, scope, &ProbeOptions::unbounded())
}

/// Probes `root` within `scope`, running the backends with the deadlines in `opts`.
pub fn probe_with(root: &SystemRoot, scope: &ProbeScope, opts: &ProbeOptions) -> Snapshot {
    probe_inner(root, scope, opts)
}

/// What the backends produced.
#[derive(Default)]
struct Results {
    environment: Option<EnvironmentInfo>,
    environment_timed_out: bool,
}

/// The environment backend: one step reads everything.
struct EnvTask<'a> {
    root: &'a SystemRoot,
    env: &'a Env,
    now: u64,
    out: Option<EnvironmentInfo>,
}

impl Task<Results> for EnvTask<'_> {
    fn backend(&self) -> Backend {
        Backend::Environment
    }

    fn step(&mut self) -> bool {
        self.out = Some(environment::environment(self.root, self.env, self.now));
        true
    }

    fn finish(self: Box<Self>, out: &mut Results, timed_out: bool) {
        out.environment_timed_out = timed_out;
        out.environment = self.out;
    }
}

pub(crate) fn probe_inner(root: &SystemRoot, scope: &ProbeScope, opts: &ProbeOptions) -> Snapshot {
    let now = root.now_ms();
    let env = Env::for_probe(root, opts);
    let mut res = Results::default();
    let mut tasks: Vec<Box<dyn Task<Results> + '_>> = Vec::new();
    if scope.layers.includes(&LayerKind::Environment) {
        tasks.push(Box::new(EnvTask {
            root,
            env: &env,
            now,
            out: None,
        }));
    }
    runtime::run(tasks, opts, &mut res);

    let layers = LayerKind::ALL
        .iter()
        .filter(|l| scope.layers.includes(l))
        .map(|&layer| SystemLayer {
            layer,
            availability: availability(layer, &res),
        })
        .collect();
    let mut snap = Snapshot {
        schema_version: SCHEMA_VERSION.to_string(),
        generation: 0,
        taken_at_ms: now,
        source: match root {
            SystemRoot::Live => SnapshotSource::Live,
            SystemRoot::Fixture(f) => SnapshotSource::Fixture {
                name: f.name().to_string(),
            },
        },
        scope: scope.clone(),
        redaction: RedactionPolicy::default(),
        layers,
        environment: res.environment,
        endpoints: vec![],
        content: opts.content.clone(),
        recommendations: vec![],
        findings: vec![],
    };
    snap.redact(&opts.redaction.clone().unwrap_or_default());
    snap
}

fn availability(layer: LayerKind, res: &Results) -> Availability {
    if layer != LayerKind::Environment {
        return Availability::Unknown {
            reason: UnknownReason::NotImplemented,
            detail: Some("no backend for this layer yet".into()),
        };
    }
    match &res.environment {
        _ if res.environment_timed_out => Availability::Unknown {
            reason: UnknownReason::Timeout,
            detail: Some("environment backend deadline".into()),
        },
        None => Availability::Unknown {
            reason: UnknownReason::Timeout,
            detail: Some("environment backend did not run".into()),
        },
        Some(e) => {
            let all = e.os.known().is_some()
                && e.kernel.known().is_some()
                && e.sandbox.known().is_some()
                && e.session.known().is_some();
            if all {
                Availability::Read
            } else {
                Availability::Partial
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Runtime;
    use crate::testutil::TempFixture;
    use displaycaps_model::{ContentRequest, SessionKind};

    const OS: &str = "ID=debian\nVERSION_ID=\"13\"\n";

    fn avail(s: &Snapshot, l: LayerKind) -> &Availability {
        &s.layers.iter().find(|x| x.layer == l).unwrap().availability
    }

    #[test]
    fn empty_scope_reads_and_reports_nothing() {
        let f = TempFixture::new(OS);
        let root = SystemRoot::fixture(f.path()).unwrap();
        let s = probe(&root, &ProbeScope::nothing());
        assert!(s.layers.is_empty() && s.environment.is_none() && s.endpoints.is_empty());
        assert!(s.findings.is_empty() && s.content.is_none());
        // A scope with fields and classes but no layers still reads no layer.
        let scope = ProbeScope::nothing()
            .with_class(displaycaps_model::EndpointClass::External)
            .with_field(displaycaps_model::FieldGroup::Hdr);
        assert!(probe(&root, &scope).layers.is_empty());
    }

    #[test]
    fn only_layers_in_scope_are_reported() {
        let f = TempFixture::new(OS);
        let root = SystemRoot::fixture(f.path()).unwrap();
        let scope = ProbeScope::nothing().with_layer(LayerKind::SinkChain);
        let s = probe(&root, &scope);
        assert_eq!(s.layers.len(), 1);
        assert_eq!(s.layers[0].layer, LayerKind::SinkChain);
        assert!(s.environment.is_none(), "environment layer is not in scope");
        assert!(matches!(
            avail(&s, LayerKind::SinkChain),
            Availability::Unknown {
                reason: UnknownReason::NotImplemented,
                ..
            }
        ));
    }

    #[test]
    fn environment_layer_with_the_given_environment() {
        let opts = ProbeOptions::unbounded().with_environment([
            ("XDG_SESSION_TYPE", "wayland"),
            ("WAYLAND_DISPLAY", "wayland-0"),
        ]);
        let scope = ProbeScope::nothing().with_layer(LayerKind::Environment);
        let s = probe_with(&SystemRoot::Live, &scope, &opts);
        let env = s.environment.as_ref().expect("environment in scope");
        assert_eq!(env.session.known().unwrap().kind, SessionKind::Wayland);
        assert!(env.session.known().unwrap().compositor.is_none());
        assert!(matches!(
            avail(&s, LayerKind::Environment),
            Availability::Read | Availability::Partial
        ));
        assert_eq!(s.source, SnapshotSource::Live);
    }

    #[test]
    fn everything_marks_unbuilt_layers_unknown() {
        let f = TempFixture::new(OS);
        let root = SystemRoot::fixture(f.path()).unwrap();
        let s = probe(&root, &ProbeScope::everything());
        assert_eq!(s.layers.len(), LayerKind::ALL.len());
        for l in LayerKind::ALL {
            match (l, avail(&s, l)) {
                (LayerKind::Environment, Availability::Partial) => {}
                (
                    _,
                    Availability::Unknown {
                        reason: UnknownReason::NotImplemented,
                        ..
                    },
                ) if l != LayerKind::Environment => {}
                (l, a) => panic!("{l:?}: {a:?}"),
            }
        }
        // The fixture has no environment variables: the session is unknown, not guessed.
        let env = s.environment.unwrap();
        assert_eq!(env.os.known().unwrap().id, "debian");
        assert!(env.session.known().is_none());
        assert_eq!(
            s.source,
            SnapshotSource::Fixture {
                name: f.path().file_name().unwrap().to_string_lossy().into_owned()
            }
        );
        assert_eq!(s.taken_at_ms, 1_000);
    }

    #[test]
    fn replay_is_deterministic() {
        let f = TempFixture::new(OS);
        let root = SystemRoot::fixture(f.path()).unwrap();
        let a = serde_json::to_string(&probe(&root, &ProbeScope::everything())).unwrap();
        let b = serde_json::to_string(&probe(&root, &ProbeScope::everything())).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn content_is_echoed_and_redaction_recorded() {
        let f = TempFixture::new(OS);
        let root = SystemRoot::fixture(f.path()).unwrap();
        let content = ContentRequest::default();
        let opts = ProbeOptions::unbounded()
            .with_content(content.clone())
            .with_deadline(Backend::Environment, std::time::Duration::from_secs(1));
        let s = probe_with(&root, &ProbeScope::nothing(), &opts);
        assert_eq!(s.content, Some(content));
        assert_eq!(s.redaction, RedactionPolicy::default());
    }

    #[test]
    fn runtime_publishes_generations() {
        let f = TempFixture::new(OS);
        let rt = Runtime::new(SystemRoot::fixture(f.path()).unwrap()).unwrap();
        let scope = ProbeScope::nothing().with_layer(LayerKind::Environment);
        let a = rt.probe(&scope, &ProbeOptions::default());
        let (b, diff) = rt.probe_diff(&scope, &ProbeOptions::default());
        assert_eq!((a.generation, b.generation), (1, 2));
        assert!(diff.expect("previous snapshot").is_empty());
        assert_eq!(rt.latest().unwrap().generation, 2);
    }
}
