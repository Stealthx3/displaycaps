// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-backends/src/sandbox.rs;
// permissions mapped to display layers instead of audio ones, snap inference rewritten.
//! Sandbox permissions: what a Flatpak or Snap app was granted, and which permissions it lacks,
//! with the layers each one hides and the exact fix.
//!
//! | Permission | Hides |
//! |---|---|
//! | Flatpak `--device=dri` | KMS and the link (`kms`, `link`) |
//! | Flatpak `--socket=wayland`, `--socket=x11` or `--socket=fallback-x11` | the compositor |
//! | Flatpak `--talk-name=org.gnome.Mutter.DisplayConfig` | Mutter's output details (compositor; not required) |
//! | Snap `opengl` | KMS and the link |
//! | Snap `wayland`, `x11` | the compositor |
//! | Snap `hardware-observe` | the sink chain and host hardware (not required) |
//! | Snap `gpu-2404` | hardware decode (not required) |
//!
//! - Flatpak: read from `/.flatpak-info` (`[Context]` sockets and devices, `[Session Bus Policy]`).
//! - Snap: a library can't run `snapctl`, so connected plugs are inferred from what the snap can
//!   reach (the Wayland and X11 sockets, a connector's EDID attribute). They are reported with
//!   inferred confidence. A plug that can't be judged without opening a device node (`opengl`,
//!   `gpu-2404`) is neither reported connected nor missing; the KMS and decode backends add the
//!   gap when a read is refused.

use displaycaps_model::{LayerKind, PermissionGap};

use crate::environment::Env;
use crate::root::SystemRoot;

/// Granted permission lines and missing permissions of a Flatpak app.
pub(crate) fn flatpak(info: &str) -> (Vec<String>, Vec<PermissionGap>) {
    let mut section = "";
    let mut app = "<app-id>".to_string();
    let mut sockets: Vec<String> = vec![];
    let mut devices: Vec<String> = vec![];
    let mut session_bus: Vec<String> = vec![];
    let mut granted = vec![];
    for line in info.lines() {
        let t = line.trim();
        if let Some(s) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = match s {
                "Application" => "app",
                "Context" => "context",
                "Session Bus Policy" => "session-bus",
                _ => "",
            };
            continue;
        }
        let Some((k, v)) = t.split_once('=') else {
            continue;
        };
        let list = || -> Vec<String> {
            v.split(';')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(String::from)
                .collect()
        };
        match (section, k.trim()) {
            ("app", "name") => app = v.trim().to_string(),
            ("context", "sockets") => {
                sockets = list();
                granted.push(t.to_string());
            }
            ("context", "devices") => {
                devices = list();
                granted.push(t.to_string());
            }
            ("session-bus", name) => {
                // Only a policy that lets the app talk counts as a grant.
                if matches!(v.trim(), "talk" | "own") {
                    session_bus.push(name.to_string());
                    granted.push(format!("session-talk-name={name}"));
                }
            }
            _ => {}
        }
    }
    let has = |l: &[String], x: &str| l.iter().any(|y| y == x || y.starts_with(&format!("{x}:")));
    let fix =
        |arg: &str| format!("flatpak override --user {arg} {app} (packagers: finish-args {arg})");
    let wayland = has(&sockets, "wayland");
    let x11 = has(&sockets, "x11") || has(&sockets, "fallback-x11");
    let mut missing = vec![];
    if !has(&devices, "dri") && !has(&devices, "all") {
        missing.push(PermissionGap {
            permission: "--device=dri".into(),
            layers: vec![LayerKind::Kms, LayerKind::Link],
            fix: fix("--device=dri"),
            required: true,
        });
    }
    if !wayland {
        missing.push(PermissionGap {
            permission: "--socket=wayland".into(),
            layers: vec![LayerKind::Compositor],
            fix: fix("--socket=wayland"),
            // An app on X11 can still read the compositor's X11 side.
            required: !x11,
        });
    }
    if !wayland && !x11 {
        missing.push(PermissionGap {
            permission: "--socket=fallback-x11".into(),
            layers: vec![LayerKind::Compositor],
            fix: fix("--socket=fallback-x11"),
            required: false,
        });
    }
    if !session_bus
        .iter()
        .any(|n| n == "org.gnome.Mutter.DisplayConfig")
    {
        missing.push(PermissionGap {
            permission: "--talk-name=org.gnome.Mutter.DisplayConfig".into(),
            layers: vec![LayerKind::Compositor],
            fix: fix("--talk-name=org.gnome.Mutter.DisplayConfig"),
            required: false,
        });
    }
    (granted, missing)
}

/// What a snap plug opens: the layers it hides and whether basic operation needs it.
fn plug(name: &str) -> (Vec<LayerKind>, bool) {
    match name {
        "opengl" => (vec![LayerKind::Kms, LayerKind::Link], true),
        "wayland" | "x11" => (vec![LayerKind::Compositor], false),
        "hardware-observe" => (vec![LayerKind::SinkChain, LayerKind::HostHardware], false),
        "gpu-2404" => (vec![LayerKind::Decode], false),
        _ => (vec![], false),
    }
}

/// The unix socket path of the Wayland display, when it can be worked out.
fn wayland_socket(env: &Env) -> Option<String> {
    let display = env.get("WAYLAND_DISPLAY")?;
    if display.starts_with('/') {
        return Some(display);
    }
    let runtime = env.get("XDG_RUNTIME_DIR").filter(|r| r.starts_with('/'))?;
    Some(format!("{}/{display}", runtime.trim_end_matches('/')))
}

/// The unix socket path of a local X11 display (`:N` or `:N.S`).
fn x11_socket(env: &Env) -> Option<String> {
    let display = env.get("DISPLAY")?;
    let n = display.strip_prefix(':')?;
    let n = n.split('.').next()?;
    n.parse::<u32>()
        .ok()
        .map(|n| format!("/tmp/.X11-unix/X{n}"))
}

/// Inferred connected plugs and missing plugs of a Snap app.
pub(crate) fn snap(root: &SystemRoot, env: &Env) -> (Vec<String>, Vec<PermissionGap>) {
    let name = env.get("SNAP_NAME").unwrap_or_else(|| "<snap>".into());
    let mut connected = vec![];
    let mut missing = vec![];
    let mut check = |ok: Option<bool>, plug_name: &str| {
        let (layers, required) = plug(plug_name);
        match ok {
            Some(true) => connected.push(plug_name.to_string()),
            Some(false) => missing.push(PermissionGap {
                permission: plug_name.into(),
                layers,
                fix: format!("snap connect {name}:{plug_name}"),
                required,
            }),
            None => {}
        }
    };
    let live = matches!(root, SystemRoot::Live);
    // The display sockets: judged only when the session names one.
    check(
        live.then(|| wayland_socket(env))
            .flatten()
            .map(|p| root.is_socket(&p)),
        "wayland",
    );
    check(
        live.then(|| x11_socket(env))
            .flatten()
            .map(|p| root.is_socket(&p)),
        "x11",
    );
    // hardware-observe is what opens device attributes such as the DRM connectors' EDID (the
    // sink chain). Unknown when there is no connector. Only the attribute's readability is
    // tested; its content is dropped.
    let edid = root.read_dir("/sys/class/drm").ok().and_then(|es| {
        es.iter()
            .filter(|e| e.name.starts_with("card") && e.name.contains('-'))
            .map(|e| format!("/sys/class/drm/{}/edid", e.name))
            .find(|p| root.exists(p))
    });
    check(
        live.then_some(edid)
            .flatten()
            .map(|p| root.read(&p).is_ok()),
        "hardware-observe",
    );
    // opengl and gpu-2404 gate device nodes under /dev/dri; judging them would mean opening one.
    check(None, "opengl");
    check(None, "gpu-2404");
    (connected, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perms(m: &[PermissionGap]) -> Vec<&str> {
        m.iter().map(|x| x.permission.as_str()).collect()
    }

    #[test]
    fn flatpak_wayland_app() {
        let info = "[Application]\nname=org.example.Player\n\n[Context]\nshared=network;ipc;\nsockets=wayland;fallback-x11;pulseaudio;\ndevices=dri;\n";
        let (g, m) = flatpak(info);
        assert!(g.iter().any(|x| x.contains("dri")));
        assert_eq!(perms(&m), ["--talk-name=org.gnome.Mutter.DisplayConfig"]);
        assert!(!m[0].required);
        assert_eq!(m[0].layers, [LayerKind::Compositor]);
        assert!(m[0].fix.contains("org.example.Player"));
    }

    #[test]
    fn flatpak_everything_granted_and_nothing_granted() {
        let full = "[Application]\nname=a.b\n[Context]\nsockets=wayland;\ndevices=all;\n[Session Bus Policy]\norg.gnome.Mutter.DisplayConfig=talk\n";
        assert!(flatpak(full).1.is_empty());
        let none = "[Application]\nname=a.b\n[Context]\nshared=network;\n";
        let m = flatpak(none).1;
        assert_eq!(
            perms(&m),
            [
                "--device=dri",
                "--socket=wayland",
                "--socket=fallback-x11",
                "--talk-name=org.gnome.Mutter.DisplayConfig"
            ]
        );
        assert!(m[0].required && m[0].layers == [LayerKind::Kms, LayerKind::Link]);
        assert!(m[1].required && !m[2].required && !m[3].required);
    }

    #[test]
    fn flatpak_x11_only_makes_wayland_optional() {
        let x = "[Application]\nname=a.b\n[Context]\nsockets=x11;\ndevices=dri;\n[Session Bus Policy]\norg.gnome.Mutter.DisplayConfig=talk\n";
        let m = flatpak(x).1;
        assert_eq!(perms(&m), ["--socket=wayland"]);
        assert!(!m[0].required);
        // A policy that forbids talking is not a grant.
        let denied = "[Application]\nname=a.b\n[Context]\nsockets=wayland;\ndevices=dri;\n[Session Bus Policy]\norg.gnome.Mutter.DisplayConfig=none\n";
        assert_eq!(
            perms(&flatpak(denied).1),
            ["--talk-name=org.gnome.Mutter.DisplayConfig"]
        );
    }

    #[test]
    fn snap_plug_table() {
        assert_eq!(
            plug("opengl"),
            (vec![LayerKind::Kms, LayerKind::Link], true)
        );
        assert_eq!(plug("wayland"), (vec![LayerKind::Compositor], false));
        assert_eq!(plug("x11"), (vec![LayerKind::Compositor], false));
        assert_eq!(
            plug("hardware-observe"),
            (vec![LayerKind::SinkChain, LayerKind::HostHardware], false)
        );
        assert_eq!(plug("gpu-2404"), (vec![LayerKind::Decode], false));
    }

    #[test]
    fn display_sockets() {
        let env = |v: &[(&str, &str)]| {
            Env::Given(
                v.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            )
        };
        assert_eq!(
            wayland_socket(&env(&[
                ("WAYLAND_DISPLAY", "wayland-1"),
                ("XDG_RUNTIME_DIR", "/run/user/1000/")
            ])),
            Some("/run/user/1000/wayland-1".into())
        );
        assert_eq!(
            wayland_socket(&env(&[("WAYLAND_DISPLAY", "/tmp/w")])),
            Some("/tmp/w".into())
        );
        assert_eq!(
            wayland_socket(&env(&[("WAYLAND_DISPLAY", "wayland-0")])),
            None
        );
        assert_eq!(
            x11_socket(&env(&[("DISPLAY", ":1.0")])),
            Some("/tmp/.X11-unix/X1".into())
        );
        assert_eq!(x11_socket(&env(&[("DISPLAY", "host:0")])), None);
    }

    #[test]
    fn snap_reports_missing_display_sockets() {
        let env = Env::Given(
            [
                ("SNAP_NAME", "player"),
                ("WAYLAND_DISPLAY", "/nonexistent/wayland-test"),
                ("DISPLAY", ":99999"),
            ]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        );
        let (connected, missing) = snap(&SystemRoot::Live, &env);
        assert!(!connected.iter().any(|c| c == "wayland" || c == "x11"));
        let w = missing.iter().find(|m| m.permission == "wayland").unwrap();
        assert_eq!(w.fix, "snap connect player:wayland");
        assert_eq!(w.layers, [LayerKind::Compositor]);
        assert!(missing.iter().any(|m| m.permission == "x11"));
        // Plugs that would need a device node are never judged.
        assert!(!missing.iter().any(|m| m.permission == "opengl"));
        assert!(!missing.iter().any(|m| m.permission == "gpu-2404"));
    }
}
