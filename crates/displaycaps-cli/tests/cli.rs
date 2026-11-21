// SPDX-License-Identifier: LGPL-2.1-or-later
//! The `displaycaps` binary end to end: version, schema, a scoped live probe, and a capture that
//! imports back as a fixture and replays.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_displaycaps"))
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("displaycaps-cli-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn version_names_the_schema() {
    let out = bin().arg("version").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("displaycaps "), "{text}");
    assert!(text.contains("snapshot schema 1."), "{text}");
}

#[test]
fn schema_is_the_published_one() {
    let out = bin().arg("schema").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["$id"]
            .as_str()
            .unwrap()
            .contains("displaycaps-snapshot-v1")
    );
    let committed = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/displaycaps-snapshot-v1.schema.json"),
    )
    .unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), committed);
}

#[test]
fn scoped_probe_reports_only_the_scope() {
    let out = bin()
        .args(["probe", "--unbounded", "--layer", "environment"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let layers = v["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0]["layer"], "environment");
    assert!(v["environment"].is_object());
    assert_eq!(v["source"]["kind"], "live");
}

#[test]
fn probe_rejects_unknown_options_and_layers() {
    assert!(!bin().args(["probe", "--bogus"]).status().unwrap().success());
    assert!(
        !bin()
            .args(["probe", "--layer", "audio"])
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn capture_imports_and_replays() {
    let t = temp("capture");
    let out = bin()
        .args(["capture", "--label", "ci"])
        .arg("--out")
        .arg(&t)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tarball = PathBuf::from(String::from_utf8(out.stdout).unwrap().trim());
    assert!(tarball.is_file());
    let fixture = t.join("fixture");
    let out = bin()
        .arg("import")
        .arg(&tarball)
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        bin()
            .arg("check-redaction")
            .arg(&fixture)
            .status()
            .unwrap()
            .success()
    );
    let out = bin().arg("replay").arg(&fixture).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["source"]["kind"], "fixture");
    assert!(
        v["environment"]["os"]["state"] == "known" || v["environment"]["os"]["state"] == "unknown"
    );
    let _ = std::fs::remove_dir_all(&t);
}
