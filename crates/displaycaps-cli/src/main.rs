// SPDX-License-Identifier: LGPL-2.1-or-later
// Adapted from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-cli/src/main.rs; the audio
// subcommands (alsa-dump, server-dump, cec-dump, watch) and opt-in flags are dropped.
//! `displaycaps` command-line tool.

mod archive;
mod capture;
mod redact;
mod timefmt;

use std::path::PathBuf;
use std::process::ExitCode;

use displaycaps_backends::{ProbeOptions, Runtime, SystemRoot, probe};
use displaycaps_model::{EndpointId, LayerKind, ProbeScope, Selection};

const USAGE: &str = "\
displaycaps — DisplayCaps command-line tool

USAGE:
  displaycaps version
  displaycaps --version
      Print the tool version and the snapshot schema version.
  displaycaps probe [--endpoint ID]... [--layer NAME]... [--unbounded] [--repeat N] [--quiet]
      Probe the live system through the runtime (per-backend deadlines, 150 ms default) and
      print the last snapshot as JSON. --endpoint limits the scope to those endpoints; --layer
      limits it to those layers (sink_chain, host_hardware, kernel_driver, kms, link, compositor,
      decode, environment); --unbounded waits for every backend; --repeat probes N times and
      prints each probe's time to stderr; --quiet skips the JSON.
  displaycaps capture [--out DIR] [--label NAME]
      Save a redacted, passive capture of this system as a .tar.gz (default DIR: ./captures).
  displaycaps import <capture.tar.gz> <fixture-dir>
      Extract a capture into a fixture directory and check its redaction.
  displaycaps check-redaction <fixture-dir>
      Scan a fixture for MAC or IP addresses that should have been redacted.
  displaycaps redact-fixture <fixture-dir>
      Re-apply the redaction rules to an extracted fixture (after a rule change).
  displaycaps replay <fixture-dir>
      Probe a fixture with the full scope and print the snapshot as JSON.
  displaycaps schema
      Print the JSON schema of the snapshot (v1).
  displaycaps sink <edid-file> [--keep-serial]
      Decode an EDID file into the sink record (JSON) with the bundled quirk data. The serial is
      redacted unless --keep-serial is given.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("capture") => cmd_capture(&args[1..]),
        Some("import") if args.len() == 3 => cmd_import(&args[1], &args[2]),
        Some("check-redaction") if args.len() == 2 => cmd_check(&args[1]),
        Some("redact-fixture") if args.len() == 2 => archive::redact_fixture(args[1].as_ref())
            .map_err(|e| e.to_string())
            .map(|n| println!("{n} files re-redacted")),
        Some("replay") if args.len() == 2 => SystemRoot::fixture(&args[1])
            .map_err(|e| e.to_string())
            .map(|r| print_snapshot(&r)),
        Some("version" | "--version" | "-V") => {
            println!(
                "displaycaps {} (snapshot schema {})",
                env!("CARGO_PKG_VERSION"),
                displaycaps_model::SCHEMA_VERSION
            );
            Ok(())
        }
        Some("probe") => cmd_probe(&args[1..]),
        Some("schema") => {
            print!("{}", displaycaps_model::schema::snapshot_schema());
            Ok(())
        }
        Some("sink") if args.len() >= 2 => {
            cmd_sink(&args[1], args[2..].iter().any(|a| a == "--keep-serial"))
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("displaycaps: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_snapshot(root: &SystemRoot) {
    let snap = probe(root, &ProbeScope::everything());
    println!(
        "{}",
        serde_json::to_string_pretty(&snap).expect("snapshot serializes")
    );
}

fn cmd_sink(path: &str, keep_serial: bool) -> Result<(), String> {
    use displaycaps_model::redact::Redact;
    let raw = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let quirks = displaycaps_rules::quirks::QuirkSet::bundled();
    let mut rec = displaycaps_backends::sink::sink_record(
        &raw,
        displaycaps_model::Source::file(path),
        0,
        &displaycaps_model::Selection::All,
        &quirks,
    )
    .ok_or("empty EDID: nothing connected")?;
    if !keep_serial {
        rec.redact(&displaycaps_model::RedactionPolicy::default());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&rec).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn cmd_probe(args: &[String]) -> Result<(), String> {
    let mut scope = ProbeScope::everything();
    let mut layers: Vec<LayerKind> = vec![];
    let mut opts = ProbeOptions::default();
    let mut repeat = 1u32;
    let mut quiet = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--endpoint" => {
                let id = it.next().ok_or("--endpoint needs an id")?;
                scope = scope.with_endpoint(EndpointId(id.clone()));
            }
            "--layer" => {
                let name = it.next().ok_or("--layer needs a name")?;
                layers.push(
                    serde_json::from_value(serde_json::Value::String(name.clone()))
                        .map_err(|_| format!("unknown layer {name}"))?,
                );
            }
            "--unbounded" => opts = ProbeOptions::unbounded(),
            "--repeat" => {
                repeat = it
                    .next()
                    .and_then(|n| n.parse().ok())
                    .filter(|n| *n > 0)
                    .ok_or("--repeat needs a positive number")?
            }
            "--quiet" => quiet = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    if !layers.is_empty() {
        scope.layers = Selection::Only(layers);
    }
    let rt = Runtime::new(SystemRoot::Live).map_err(|e| e.to_string())?;
    let mut last = None;
    for i in 0..repeat {
        let t = std::time::Instant::now();
        let snap = rt.probe(&scope, &opts);
        if repeat > 1 {
            eprintln!(
                "probe {}: {:.2} ms, {} endpoints",
                i + 1,
                t.elapsed().as_secs_f64() * 1000.0,
                snap.endpoints.len()
            );
        }
        last = Some(snap);
    }
    if !quiet && let Some(snap) = last {
        println!(
            "{}",
            serde_json::to_string_pretty(&*snap).expect("snapshot serializes")
        );
    }
    Ok(())
}

fn cmd_capture(args: &[String]) -> Result<(), String> {
    let mut opts = capture::Options {
        out_dir: PathBuf::from("captures"),
        label: "system".into(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => opts.out_dir = it.next().ok_or("--out needs a directory")?.into(),
            "--label" => opts.label = it.next().ok_or("--label needs a name")?.clone(),
            other => return Err(format!("unknown option {other}")),
        }
    }
    let path = capture::capture(&opts).map_err(|e| e.to_string())?;
    println!("{}", path.display());
    Ok(())
}

fn cmd_import(tarball: &str, dest: &str) -> Result<(), String> {
    let n = archive::extract(tarball.as_ref(), dest.as_ref()).map_err(|e| e.to_string())?;
    SystemRoot::fixture(dest).map_err(|e| format!("not a capture: {e}"))?;
    let leaks = archive::check_redaction(dest.as_ref()).map_err(|e| e.to_string())?;
    if !leaks.is_empty() {
        return Err(format!("redaction check failed:\n  {}", leaks.join("\n  ")));
    }
    println!("imported {n} files into {dest}; redaction check passed");
    Ok(())
}

fn cmd_check(dir: &str) -> Result<(), String> {
    let leaks = archive::check_redaction(dir.as_ref()).map_err(|e| e.to_string())?;
    if leaks.is_empty() {
        println!("redaction check passed");
        Ok(())
    } else {
        Err(format!("redaction check failed:\n  {}", leaks.join("\n  ")))
    }
}
