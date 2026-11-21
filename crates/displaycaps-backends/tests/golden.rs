// SPDX-License-Identifier: LGPL-2.1-or-later
// Adapted from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-backends/tests/golden.rs; display
// scope (no directions) in the scope test.
//! Golden snapshots: every fixture under `tests/fixtures/` replays into a stable snapshot stored
//! as `tests/fixtures/<name>.snapshot.json`. Re-bless after an intended change with
//! `UPDATE_EXPECT=1 cargo test -p displaycaps-backends --test golden`.

use std::path::Path;

use displaycaps_backends::{SystemRoot, probe};
use displaycaps_model::ProbeScope;

fn fixtures() -> Vec<std::path::PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut v: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/fixtures exists")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("manifest.json").is_file())
        .collect();
    v.sort();
    v
}

#[test]
fn every_fixture_replays_into_its_golden_snapshot() {
    let all = fixtures();
    assert!(
        all.len() >= 4,
        "lab-gnome, lab-plasma, lab-sway and workstation fixtures"
    );
    for dir in all {
        let root = SystemRoot::fixture(&dir).expect("fixture opens");
        let snap = probe(&root, &ProbeScope::everything());
        let mut json = serde_json::to_string_pretty(&snap).expect("serializes");
        json.push('\n');
        let golden = dir.with_extension("snapshot.json");
        expect_test::expect_file![golden].assert_eq(&json);
    }
}

#[test]
fn replay_is_deterministic() {
    for dir in fixtures() {
        let root = SystemRoot::fixture(&dir).expect("fixture opens");
        let a = serde_json::to_string(&probe(&root, &ProbeScope::everything())).unwrap();
        let b = serde_json::to_string(&probe(&root, &ProbeScope::everything())).unwrap();
        assert_eq!(a, b, "{}", dir.display());
    }
}

#[test]
fn golden_snapshots_deserialize_into_the_model() {
    for dir in fixtures() {
        let golden = dir.with_extension("snapshot.json");
        if let Ok(text) = std::fs::read_to_string(&golden) {
            let _: displaycaps_model::Snapshot =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", golden.display()));
        }
    }
}

#[test]
fn scope_limits_what_is_reported() {
    use displaycaps_model::{EndpointClass, LayerKind};
    for dir in fixtures() {
        let root = SystemRoot::fixture(&dir).unwrap();
        let scope = ProbeScope::nothing()
            .with_class(EndpointClass::External)
            .with_layer(LayerKind::SinkChain);
        let s = probe(&root, &scope);
        assert!(s.environment.is_none(), "{}", dir.display());
        assert!(s.findings.is_empty());
        assert!(
            s.endpoints
                .iter()
                .all(|e| e.layers.iter().all(|l| l.layer == LayerKind::SinkChain))
        );
    }
}
