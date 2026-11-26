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
  displaycaps watch [--layer NAME]... [--events N] [--seconds S] [--kms-poll MS]
      Watch the live system (default: every layer) and print one JSON line per change: its
      causes (hotplug, kms_change, ...), the re-probe latency and the diff. Stops after N events
      (default 1) or S seconds (default 60). --kms-poll turns on the KMS change poll.
  displaycaps link <edid-file> --mode WxH@HZ [--connector hdmi|dp] [--driver NAME]
                   [--kernel X.Y] [--param MODULE.NAME=VALUE]... [--max-bpc N]
                   [--color-format auto|rgb|444|422|420] [--sink-frl LANESxGBPS]
                   [--sink-vic N]... [--json]
      Simulate a link without hardware: the format the driver would send for that mode, and
      every better format with the reason it doesn't fit (defaults: hdmi, amdgpu, kernel 7.4,
      max bpc 10, automatic format). --sink-frl and --sink-vic simulate a sink with an FRL
      rate or CTA-861 modes its EDID lacks.
  displaycaps kms-facts [--drm-info FILE]
      Print KMS state as normalized fact lines, read with this library's read-only ioctls (only
      inside a display session), or from a `drm_info -j` file. Lab tooling for the KMS gate.
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
        Some("kms-facts") => cmd_kms_facts(&args[1..]),
        Some("watch") => cmd_watch(&args[1..]),
        Some("link") => cmd_link(&args[1..]),
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

fn cmd_kms_facts(args: &[String]) -> Result<(), String> {
    let json = match args {
        [] => None,
        [flag, file] if flag == "--drm-info" => {
            Some(std::fs::read(file).map_err(|e| format!("{file}: {e}"))?)
        }
        _ => return Err("usage: displaycaps kms-facts [--drm-info FILE]".into()),
    };
    for line in displaycaps_backends::kms_facts(json.as_deref())? {
        println!("{line}");
    }
    Ok(())
}

fn cmd_watch(args: &[String]) -> Result<(), String> {
    let mut scope = ProbeScope::everything();
    let mut layers: Vec<LayerKind> = vec![];
    let (mut events, mut seconds) = (1u32, 60u64);
    let mut opts = ProbeOptions::default();
    let mut it = args.iter();
    let num = |v: Option<&String>, what: &str| {
        v.and_then(|n| n.parse::<u64>().ok())
            .filter(|n| *n > 0)
            .ok_or(format!("{what} needs a positive number"))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--layer" => {
                let name = it.next().ok_or("--layer needs a name")?;
                layers.push(
                    serde_json::from_value(serde_json::Value::String(name.clone()))
                        .map_err(|_| format!("unknown layer {name}"))?,
                );
            }
            "--events" => events = u32::try_from(num(it.next(), "--events")?).unwrap_or(u32::MAX),
            "--seconds" => seconds = num(it.next(), "--seconds")?,
            "--kms-poll" => {
                opts = opts.with_kms_poll(Some(std::time::Duration::from_millis(num(
                    it.next(),
                    "--kms-poll",
                )?)))
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    if !layers.is_empty() {
        scope.layers = Selection::Only(layers);
    }
    let rt = Runtime::new(SystemRoot::Live).map_err(|e| e.to_string())?;
    let w = rt.watch(&scope, &opts).map_err(|e| e.to_string())?;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    // The first event is the initial snapshot.
    let first = w
        .recv_timeout(std::time::Duration::from_secs(seconds))
        .ok_or("no initial snapshot")?;
    eprintln!(
        "watch: initial snapshot, {} endpoints; waiting for changes",
        first.snapshot.endpoints.len()
    );
    let mut seen = 0;
    while seen < events {
        let left = end.saturating_duration_since(std::time::Instant::now());
        let Some(ev) = w.recv_timeout(left) else {
            return Err(format!("{seen} of {events} events before the timeout"));
        };
        seen += 1;
        println!(
            "{}",
            serde_json::json!({
                "causes": ev.causes.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>(),
                "latency_ms": ev.latency.as_secs_f64() * 1e3,
                "diff": ev.diff,
            })
        );
    }
    Ok(())
}

fn cmd_link(args: &[String]) -> Result<(), String> {
    use displaycaps_model::{ColorFormat, ConnectorType, FrlRate, LayerDetail, LinkSignal};
    let usage =
        "usage: displaycaps link <edid-file> --mode WxH@HZ [options] (see displaycaps --help)";
    let (file, rest) = args.split_first().ok_or(usage)?;
    let mut sc = displaycaps_backends::LinkScenario {
        edid: std::fs::read(file).map_err(|e| format!("{file}: {e}"))?,
        mode: (0, 0, 0),
        connector: ConnectorType::Hdmi,
        driver: "amdgpu".into(),
        kernel: "7.4".into(),
        params: vec![],
        max_bpc: 10,
        color_format: None,
        sink_frl: None,
        sink_vics: vec![],
    };
    let mut json = false;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--mode" => {
                let v = val()?;
                let (wh, hz) = v.split_once('@').ok_or("--mode WxH@HZ")?;
                let (w, h) = wh.split_once('x').ok_or("--mode WxH@HZ")?;
                let hz: f64 = hz.parse().map_err(|_| "--mode: refresh")?;
                sc.mode = (
                    w.parse().map_err(|_| "--mode: width")?,
                    h.parse().map_err(|_| "--mode: height")?,
                    (hz * 1000.0).round() as u32,
                );
            }
            "--connector" => {
                sc.connector = match val()?.as_str() {
                    "hdmi" => ConnectorType::Hdmi,
                    "dp" => ConnectorType::DisplayPort,
                    o => return Err(format!("--connector: {o}")),
                }
            }
            "--driver" => sc.driver = val()?,
            "--kernel" => sc.kernel = val()?,
            "--param" => {
                let v = val()?;
                let (n, x) = v.split_once('=').ok_or("--param MODULE.NAME=VALUE")?;
                sc.params.push((n.to_string(), x.to_string()));
            }
            "--max-bpc" => sc.max_bpc = val()?.parse().map_err(|_| "--max-bpc: number")?,
            "--color-format" => {
                sc.color_format = match val()?.as_str() {
                    "auto" => None,
                    "rgb" => Some(ColorFormat::Rgb),
                    "444" => Some(ColorFormat::Ycbcr444),
                    "422" => Some(ColorFormat::Ycbcr422),
                    "420" => Some(ColorFormat::Ycbcr420),
                    o => return Err(format!("--color-format: {o}")),
                }
            }
            "--sink-frl" => {
                let v = val()?;
                let (l, g) = v.split_once('x').ok_or("--sink-frl LANESxGBPS")?;
                sc.sink_frl = Some(FrlRate {
                    lanes: l.parse().map_err(|_| "--sink-frl: lanes")?,
                    gbps_per_lane: g.parse().map_err(|_| "--sink-frl: rate")?,
                });
            }
            "--sink-vic" => sc
                .sink_vics
                .push(val()?.parse().map_err(|_| "--sink-vic: number")?),
            "--json" => json = true,
            o => return Err(format!("unknown option {o}")),
        }
    }
    if sc.mode.0 == 0 {
        return Err(usage.into());
    }
    let layer = displaycaps_backends::explain_link(&sc)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&layer).expect("serializes")
        );
        return Ok(());
    }
    let Some(LayerDetail::Link(ls)) = &layer.detail else {
        return Err(format!("no link: {:?}", layer.availability));
    };
    let fmt = |f: &ColorFormat| match f {
        ColorFormat::Rgb => "RGB",
        ColorFormat::Ycbcr444 => "YCbCr 4:4:4",
        ColorFormat::Ycbcr422 => "YCbCr 4:2:2",
        ColorFormat::Ycbcr420 => "YCbCr 4:2:0",
    };
    let signal = match ls.signal.known() {
        Some(LinkSignal::Tmds { character_rate_khz }) => {
            format!("TMDS {:.1} MHz", f64::from(*character_rate_khz) / 1000.0)
        }
        Some(LinkSignal::Frl { rate }) => {
            format!("FRL {}x{} Gbit/s", rate.lanes, rate.gbps_per_lane)
        }
        Some(LinkSignal::DisplayPort { link }) => format!("DP {}x{:?}", link.lanes, link.rate),
        Some(LinkSignal::Virtual) => "virtual".into(),
        None => "none".into(),
    };
    match (ls.color_format.known(), ls.bit_depth.known()) {
        (Some(f), Some(b)) => {
            let need = ls
                .required_kbps
                .known()
                .map(|k| *k as f64 / 1e6)
                .unwrap_or(0.0);
            let cap = ls
                .capacity_kbps
                .known()
                .map(|k| *k as f64 / 1e6)
                .unwrap_or(0.0);
            println!(
                "link:     {} {b}-bit over {signal} ({need:.2} of {cap:.2} Gbit/s, inferred)",
                fmt(f)
            );
        }
        _ => println!("link:     nothing fits"),
    }
    for r in &ls.rejected {
        println!(
            "rejected: {} {}-bit{}: {}",
            fmt(&r.color_format),
            r.bit_depth,
            if r.dsc { " with DSC" } else { "" },
            r.reason
        );
    }
    for l in &ls.limited_by {
        println!("limited:  {l}");
    }
    Ok(())
}
