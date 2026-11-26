// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-cli/src/capture.rs; the generic
// framework (staging, file copy with redaction, command recording, tarball) is kept, every audio
// collector is dropped and the DRM connector copy is new and minimal.
//! `displaycaps capture`: saves the files and command outputs that DisplayCaps reads into a
//! redacted tarball. Read-only and passive:
//! - only regular files under `/proc`, `/sys` and `/etc` are read, never `/dev`;
//! - commands may only query, with a timeout, and their output is redacted; every command used
//!   here is passive: `drm_info` uses `drmModeGetConnectorCurrent` (never a forced probe) and
//!   never calls SET_MASTER; it and `vulkaninfo` open primary DRM nodes, so they run only inside
//!   a display session whose compositor already holds master, `xrandr --current` never re-probes outputs, and nothing writes
//!   KMS, compositor or DDC/CI state. `scripts/check-passive.sh` checks a capture under strace.
//!
//! What it saves: os-release and the kernel version; every DRM card and connector under
//! `/sys/class/drm` (status, modes, enabled, dpms, EDID with the serial zeroed) and the GPU's PCI
//! ids and driver link; the display driver's module parameters (e.g. amdgpu's feature masks);
//! `drm_info -j`; the compositor's output state (`wayland-info`, `kscreen-doctor -j`,
//! `wlr-randr --json`, Mutter's `GetCurrentState`, `xrandr --current --verbose`, and this
//! library's own compositor facts, `displaycaps-compositor.json`); and `vainfo` and
//! `vulkaninfo --summary`. A tool that isn't installed is recorded as skipped.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use displaycaps_backends::manifest::{CAPTURE_FORMAT, CapturedCommand, CapturedSandbox, Manifest};
use displaycaps_parse::edid;

use crate::redact::{RULES, Redactor};

const MAX_FILE: u64 = 1 << 20;
const CMD_TIMEOUT: Duration = Duration::from_secs(5);

/// Options for a capture.
#[derive(Debug, Clone)]
pub struct Options {
    /// Directory to write the tarball into.
    pub out_dir: PathBuf,
    /// Label for the tarball name, e.g. `htpc`.
    pub label: String,
}

struct Staging {
    dir: PathBuf,
    redactor: Redactor,
    files: usize,
    seen: BTreeSet<PathBuf>,
    notes: Vec<String>,
}

impl Staging {
    fn dest(&self, live: &Path) -> PathBuf {
        self.dir
            .join("root")
            .join(live.strip_prefix("/").unwrap_or(live))
    }

    /// Copies one regular file (redacting it), or recreates one symlink with the same target.
    /// Device nodes are never opened.
    fn copy(&mut self, live: &Path) {
        if !self.seen.insert(live.to_path_buf()) || live.starts_with("/dev") {
            return;
        }
        let Ok(meta) = fs::symlink_metadata(live) else {
            return;
        };
        let dest = self.dest(live);
        if let Some(p) = dest.parent() {
            let _ = fs::create_dir_all(p);
        }
        if meta.file_type().is_symlink() {
            if let Ok(target) = fs::read_link(live) {
                let _ = symlink(target, &dest);
            }
            return;
        }
        if !meta.is_file() {
            return;
        }
        let mut buf = Vec::new();
        // procfs/sysfs report size 0 or 4096, so read with a cap instead of trusting the size.
        match fs::File::open(live).and_then(|f| f.take(MAX_FILE).read_to_end(&mut buf)) {
            Ok(_) => {}
            Err(e) => {
                if e.kind() != io::ErrorKind::PermissionDenied {
                    self.notes
                        .push(format!("unreadable: {} ({})", live.display(), e.kind()));
                }
                return;
            }
        }
        let data = if live.file_name().is_some_and(|n| n == "edid") {
            edid::redact_serials(&mut buf);
            buf
        } else if let Ok(text) = std::str::from_utf8(&buf) {
            self.redactor.text(text).into_bytes()
        } else {
            self.notes.push(format!(
                "binary file not redacted, skipped: {}",
                live.display()
            ));
            return;
        };
        if fs::write(&dest, data).is_ok() {
            self.files += 1;
        }
    }

    /// Copies a directory tree of regular files and symlinks (not following links), `depth`
    /// directory levels deep (1 copies only the files directly inside).
    fn copy_tree(&mut self, live: &Path, depth: u32) {
        if depth == 0 {
            return;
        }
        let Ok(entries) = fs::read_dir(live) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                self.copy_tree(&p, depth - 1);
            } else if ft.is_file() || ft.is_symlink() {
                self.copy(&p);
            }
        }
    }
}

/// Runs one query command with a timeout and stores its redacted output under `commands/`.
fn run_command(
    st: &mut Staging,
    name: &str,
    argv: &[&str],
    env: &[(&str, &str)],
) -> CapturedCommand {
    let mut rec = CapturedCommand {
        name: name.to_string(),
        argv: argv.iter().map(|s| s.to_string()).collect(),
        exit_code: None,
        output_file: None,
        skipped: None,
    };
    let mut cmd = Command::new(argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            rec.skipped = Some(format!("not run: {}", e.kind()));
            return rec;
        }
    };
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stdout.read_to_end(&mut b);
        b
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() > CMD_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    let out = reader.join().unwrap_or_default();
    match status {
        None => rec.skipped = Some("timed out after 5 s".into()),
        Some(s) => {
            rec.exit_code = s.code();
            let text = String::from_utf8_lossy(&out);
            let (file, data) = match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(mut v) => {
                    st.redactor.json(&mut v);
                    let mut s = serde_json::to_string_pretty(&v).unwrap_or_default();
                    s.push('\n');
                    (format!("{name}.json"), s)
                }
                Err(_) => (format!("{name}.txt"), st.redactor.text(&text)),
            };
            if !data.trim().is_empty()
                && fs::write(st.dir.join("commands").join(&file), data).is_ok()
            {
                rec.output_file = Some(file);
            }
        }
    }
    rec
}

/// Query commands, run with the capturing user's environment (so they reach the session).
const COMMANDS: &[(&str, &[&str])] = &[
    ("drm_info", &["drm_info", "-j"]),
    ("wayland-info", &["wayland-info"]),
    ("kscreen-doctor", &["kscreen-doctor", "-j"]),
    ("wlr-randr", &["wlr-randr", "--json"]),
    (
        "mutter-displayconfig",
        &[
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.gnome.Mutter.DisplayConfig",
            "--object-path",
            "/org/gnome/Mutter/DisplayConfig",
            "--method",
            "org.gnome.Mutter.DisplayConfig.GetCurrentState",
        ],
    ),
    ("xrandr", &["xrandr", "--current", "--verbose"]),
    ("vainfo", &["vainfo"]),
    ("vulkaninfo", &["vulkaninfo", "--summary"]),
];

/// Commands that open a primary DRM node (`/dev/dri/card*`).
const PRIMARY_NODE: &[&str] = &["drm_info", "vulkaninfo"];

/// The GPU behind a card: PCI ids, uevent, the driver link, and that driver's module parameters.
fn copy_gpu(st: &mut Staging, card: &Path) {
    let dev = card.join("device");
    for f in [
        "vendor",
        "device",
        "subsystem_vendor",
        "subsystem_device",
        "class",
        "revision",
        "uevent",
        "driver",
        "boot_vga",
    ] {
        st.copy(&dev.join(f));
    }
    if let Ok(real) = fs::canonicalize(&dev) {
        st.copy(&real.join("driver"));
    }
    let driver = fs::read_link(dev.join("driver"))
        .ok()
        .and_then(|t| t.file_name().map(|n| n.to_string_lossy().into_owned()));
    for m in driver
        .iter()
        .map(String::as_str)
        .chain(["drm", "drm_kms_helper"])
    {
        st.copy_tree(&Path::new("/sys/module").join(m).join("parameters"), 1);
    }
}

/// Runs a capture and returns the tarball path.
pub fn capture(opts: &Options) -> io::Result<PathBuf> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let stamp = crate::timefmt::stamp(now_ms / 1000);
    let base = format!("displaycaps-capture-{}-{stamp}", sanitize(&opts.label));
    fs::create_dir_all(&opts.out_dir)?;
    let dir = opts.out_dir.join(format!(".{base}.staging"));
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(dir.join("root"))?;
    fs::create_dir_all(dir.join("commands"))?;

    let host = fs::read_to_string("/proc/sys/kernel/hostname").ok();
    let user = std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("LOGNAME").ok());
    let mut commands = Vec::new();
    let mut st = Staging {
        dir: dir.clone(),
        redactor: Redactor::new(host.as_deref(), user.as_deref()),
        files: 0,
        seen: BTreeSet::new(),
        notes: vec![],
    };

    for f in [
        "/proc/sys/kernel/osrelease",
        "/etc/os-release",
        "/usr/lib/os-release",
        "/.flatpak-info",
    ] {
        st.copy(Path::new(f));
    }
    if let Ok(rd) = fs::read_dir("/sys/class/drm") {
        let mut names: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        names.sort();
        for p in names {
            let n = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Cards (card1) and connectors (card1-HDMI-A-1), not render nodes.
            if !n.starts_with("card") {
                continue;
            }
            st.copy(&p);
            if let Ok(real) = fs::canonicalize(&p) {
                st.copy_tree(&real, 1);
                if !n.contains('-') {
                    copy_gpu(&mut st, &real);
                }
            }
        }
        st.copy(Path::new("/sys/class/drm/version"));
    }
    // drm_info and vulkaninfo open primary /dev/dri/card* nodes. If no compositor holds DRM
    // master on a card, the kernel makes the first opener master implicitly, and closing it can
    // restore the console (a visible modeset). Run them only inside a display session.
    let session =
        std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    for (name, argv) in COMMANDS {
        if PRIMARY_NODE.contains(name) && !session {
            commands.push(CapturedCommand {
                name: name.to_string(),
                argv: argv.iter().map(|s| s.to_string()).collect(),
                exit_code: None,
                output_file: None,
                skipped: Some(
                    "no display session: would open a primary DRM node nothing holds master on"
                        .into(),
                ),
            });
            continue;
        }
        let rec = run_command(&mut st, name, argv, &[]);
        commands.push(rec);
    }
    // This library's own compositor reader, in-process, with serials and EDIDs redacted: the
    // facts fixtures replay.
    let name = displaycaps_backends::compositor::CAPTURED;
    let mut rec = CapturedCommand {
        name: name.to_string(),
        argv: vec!["(in-process)".into()],
        exit_code: None,
        output_file: None,
        skipped: None,
    };
    if session {
        match displaycaps_backends::compositor::live_facts(CMD_TIMEOUT) {
            Ok(f) => {
                let file = format!("{name}.json");
                let mut text = serde_json::to_string_pretty(&f).map_err(io::Error::other)?;
                text.push('\n');
                if fs::write(st.dir.join("commands").join(&file), st.redactor.text(&text)).is_ok() {
                    rec.exit_code = Some(0);
                    rec.output_file = Some(file);
                }
            }
            Err(e) => {
                rec.exit_code = Some(1);
                rec.skipped = Some(e);
            }
        }
    } else {
        rec.skipped = Some("no display session".into());
    }
    commands.push(rec);

    let manifest = Manifest {
        format: CAPTURE_FORMAT,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        captured_at_ms: now_ms,
        sandbox: CapturedSandbox {
            snap: std::env::var_os("SNAP_NAME").is_some(),
            flatpak: Path::new("/.flatpak-info").exists(),
        },
        file_count: st.files,
        commands,
        redaction: RULES.iter().map(|s| s.to_string()).collect(),
        notes: st.notes.clone(),
    };
    let mut m = serde_json::to_string_pretty(&manifest).map_err(io::Error::other)?;
    m.push('\n');
    fs::write(dir.join("manifest.json"), m)?;

    let tarball = opts.out_dir.join(format!("{base}.tar.gz"));
    crate::archive::write(&dir, &base, &tarball)?;
    fs::remove_dir_all(&dir)?;
    Ok(tarball)
}

fn sanitize(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() { "system".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_sanitized() {
        assert_eq!(sanitize("My HTPC #1"), "my-htpc--1");
        assert_eq!(sanitize(""), "system");
    }

    fn staging(name: &str) -> Staging {
        let dir =
            std::env::temp_dir().join(format!("displaycaps-cap-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("root")).unwrap();
        fs::create_dir_all(dir.join("commands")).unwrap();
        Staging {
            dir,
            redactor: Redactor::new(Some("lab-host"), None),
            files: 0,
            seen: BTreeSet::new(),
            notes: vec![],
        }
    }

    #[test]
    fn copy_redacts_text_and_zeroes_edid_serials() {
        let mut st = staging("copy");
        let src = st.dir.join("live");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("uevent"), "HOST=lab-host\n").unwrap();
        // A 128-byte base block with serial 0xDEADBEEF and a valid checksum.
        let mut edid_bytes = vec![0u8; 128];
        edid_bytes[..8].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 0]);
        edid_bytes[12..16].copy_from_slice(&[0xEF, 0xBE, 0xAD, 0xDE]);
        let sum: u8 = edid_bytes[..127]
            .iter()
            .fold(0u8, |a, b| a.wrapping_add(*b));
        edid_bytes[127] = 0u8.wrapping_sub(sum);
        fs::write(src.join("edid"), &edid_bytes).unwrap();
        st.copy(&src.join("uevent"));
        st.copy(&src.join("edid"));
        assert_eq!(st.files, 2);
        let text = fs::read_to_string(st.dest(&src.join("uevent"))).unwrap();
        assert_eq!(text, "HOST=redacted-host\n");
        let out = fs::read(st.dest(&src.join("edid"))).unwrap();
        assert_eq!(&out[12..16], &[0, 0, 0, 0]);
        assert_eq!(
            out.iter().fold(0u8, |a, b| a.wrapping_add(*b)),
            0,
            "checksum fixed"
        );
        let _ = fs::remove_dir_all(&st.dir);
    }

    #[test]
    fn device_nodes_are_never_copied() {
        let mut st = staging("dev");
        st.copy(Path::new("/dev/null"));
        st.copy(Path::new("/dev/dri/card0"));
        assert_eq!(st.files, 0);
        assert!(!st.dest(Path::new("/dev/null")).exists());
        let _ = fs::remove_dir_all(&st.dir);
    }

    #[test]
    fn copy_tree_takes_only_the_requested_depth() {
        let mut st = staging("tree");
        let src = st.dir.join("live");
        fs::create_dir_all(src.join("power")).unwrap();
        fs::write(src.join("status"), "connected\n").unwrap();
        fs::write(src.join("power/control"), "auto\n").unwrap();
        st.copy_tree(&src, 1);
        assert_eq!(st.files, 1);
        let _ = fs::remove_dir_all(&st.dir);
    }

    #[test]
    fn run_command_records_output_and_skips_missing_tools() {
        let mut st = staging("cmd");
        let rec = run_command(&mut st, "echo", &["echo", "host lab-host"], &[]);
        assert_eq!(rec.exit_code, Some(0));
        let file = rec.output_file.expect("output stored");
        let text = fs::read_to_string(st.dir.join("commands").join(file)).unwrap();
        assert_eq!(text, "host redacted-host\n");
        let rec = run_command(&mut st, "nope", &["/nonexistent/displaycaps-tool"], &[]);
        assert!(rec.skipped.is_some() && rec.exit_code.is_none());
        let _ = fs::remove_dir_all(&st.dir);
    }
}
