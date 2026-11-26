// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-backends/src/environment.rs;
// sound-server presence replaced by the display session read from the probe's environment.
//! The environment layer: os-release, kernel, sandbox and the display session.
//!
//! The session is worked out from `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY` and `DISPLAY` as the probe
//! was given them (see [`crate::runtime::ProbeOptions::environment`]); no socket is connected to.
//! The compositor's name and protocols ([`displaycaps_model::SessionInfo::compositor`] and
//! `protocols`) come from the compositor backend (the Wayland registry, Mutter's D-Bus name or
//! the X server), which the probe runs for the environment layer too and fills in afterwards.

use std::collections::BTreeMap;

use displaycaps_model::{
    DistroFamily, EnvironmentInfo, LayerKind, OsInfo, Provenance, Sandbox, SessionInfo,
    SessionKind, Source, UnknownReason, Value,
};
use displaycaps_parse::os_release;

use crate::root::SystemRoot;
use crate::runtime::ProbeOptions;

/// The environment variables a probe may read.
#[derive(Debug, Clone)]
pub(crate) enum Env {
    /// The process environment (live system, nothing given by the application).
    Process,
    /// Exactly these variables.
    Given(BTreeMap<String, String>),
    /// None (a fixture, nothing given): environment variables are not captured.
    Absent,
}

impl Env {
    /// The environment this probe reads.
    pub(crate) fn for_probe(root: &SystemRoot, opts: &ProbeOptions) -> Self {
        match (&opts.environment, root) {
            (Some(v), _) => Self::Given(v.iter().cloned().collect()),
            (None, SystemRoot::Live) => Self::Process,
            (None, SystemRoot::Fixture(_)) => Self::Absent,
        }
    }

    /// A variable's value; empty counts as unset.
    pub(crate) fn get(&self, key: &str) -> Option<String> {
        let v = match self {
            Self::Process => std::env::var(key).ok(),
            Self::Given(m) => m.get(key).cloned(),
            Self::Absent => None,
        };
        v.filter(|v| !v.is_empty())
    }

    /// Whether anything can be read at all.
    fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

fn prov(layer: LayerKind, source: Source, at: u64) -> Provenance {
    Provenance {
        layer,
        source,
        read_at_ms: at,
    }
}

/// Reads the environment layer.
pub(crate) fn environment(root: &SystemRoot, env: &Env, now: u64) -> EnvironmentInfo {
    // Inside Flatpak, /etc/os-release describes the runtime; the host's is at /run/host.
    let candidates: &[&str] = if root.exists("/.flatpak-info") {
        &[
            "/run/host/os-release",
            "/etc/os-release",
            "/usr/lib/os-release",
        ]
    } else {
        &["/etc/os-release", "/usr/lib/os-release"]
    };
    let os = candidates
        .iter()
        .find_map(|p| root.read_string(p).ok().map(|t| (p, t)))
        .map(|(p, text)| {
            let kv = os_release::parse(&text);
            let id = os_release::get(&kv, "ID").unwrap_or("linux").to_string();
            let like = os_release::get(&kv, "ID_LIKE").unwrap_or("");
            let family = family_of(&id, like);
            Value::measured(
                OsInfo {
                    id,
                    version_id: os_release::get(&kv, "VERSION_ID").map(String::from),
                    family,
                },
                prov(LayerKind::Environment, Source::file(*p), now),
            )
        })
        .unwrap_or_else(|| Value::unknown(UnknownReason::NotExposed, Some("no os-release".into())));

    let kernel = match root.read_string("/proc/sys/kernel/osrelease") {
        Ok(k) => Value::measured(
            k.trim().to_string(),
            prov(
                LayerKind::Environment,
                Source::file("/proc/sys/kernel/osrelease"),
                now,
            ),
        ),
        Err(e) => Value::unknown(UnknownReason::ReadError, Some(e.to_string())),
    };

    EnvironmentInfo {
        os,
        kernel,
        sandbox: sandbox(root, env, now),
        session: session(env, now),
        graphics_driver: None,
    }
}

fn family_of(id: &str, like: &str) -> DistroFamily {
    let all: Vec<&str> = std::iter::once(id).chain(like.split_whitespace()).collect();
    let has = |x: &str| all.contains(&x);
    if has("debian") || has("ubuntu") {
        DistroFamily::Debian
    } else if has("fedora") || has("rhel") || has("centos") {
        DistroFamily::Fedora
    } else if has("arch") {
        DistroFamily::Arch
    } else if all.iter().any(|x| x.contains("suse")) {
        DistroFamily::Suse
    } else {
        DistroFamily::Other
    }
}

/// Whether the app runs in a snap: `SNAP_NAME` in the environment, or the capture's note for a
/// fixture without an environment.
pub(crate) fn in_snap(root: &SystemRoot, env: &Env) -> bool {
    match root {
        SystemRoot::Fixture(f) if env.is_absent() => f.manifest().sandbox.snap,
        _ => env.get("SNAP_NAME").is_some(),
    }
}

fn sandbox(root: &SystemRoot, env: &Env, now: u64) -> Value<Sandbox> {
    let p = prov(LayerKind::Environment, Source::file("/.flatpak-info"), now);
    match root.read_string("/.flatpak-info") {
        Ok(info) => {
            let (granted, missing) = crate::sandbox::flatpak(&info);
            return Value::measured(Sandbox::Flatpak { granted, missing }, p);
        }
        Err(e) if root.exists("/.flatpak-info") => {
            return Value::unknown(
                UnknownReason::ReadError,
                Some(format!(
                    "/.flatpak-info exists but can't be read: {}",
                    e.kind()
                )),
            );
        }
        Err(_) => {}
    }
    if in_snap(root, env) {
        let (connected, missing) = crate::sandbox::snap(root, env);
        return Value::inferred(
            Sandbox::Snap { connected, missing },
            prov(
                LayerKind::Environment,
                Source::derived("SNAP_NAME and the sockets and files the snap can reach"),
                now,
            ),
        );
    }
    Value::measured(Sandbox::None, p)
}

/// The session kind from `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY` and `DISPLAY` (`None` when the
/// environment is absent or names no display session).
pub(crate) fn session_kind(env: &Env) -> SessionKind {
    match env.get("XDG_SESSION_TYPE").as_deref() {
        Some("wayland") => SessionKind::Wayland,
        Some("x11") => SessionKind::X11,
        _ if env.get("WAYLAND_DISPLAY").is_some() => SessionKind::Wayland,
        _ if env.get("DISPLAY").is_some() => SessionKind::X11,
        _ => SessionKind::None,
    }
}

/// The display session, from `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY` and `DISPLAY`.
fn session(env: &Env, now: u64) -> Value<SessionInfo> {
    if env.is_absent() {
        return Value::unknown(
            UnknownReason::NotExposed,
            Some("environment variables are not captured".into()),
        );
    }
    let kind = session_kind(env);
    Value::inferred(
        SessionInfo {
            kind,
            compositor: None,
            protocols: vec![],
        },
        prov(
            LayerKind::Environment,
            Source::derived("XDG_SESSION_TYPE, WAYLAND_DISPLAY and DISPLAY"),
            now,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn given(vars: &[(&str, &str)]) -> Env {
        Env::Given(
            vars.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    fn kind(env: &Env) -> SessionKind {
        session(env, 0).known().expect("known").kind
    }

    #[test]
    fn families() {
        assert_eq!(family_of("ubuntu", "debian"), DistroFamily::Debian);
        assert_eq!(
            family_of("rocky", "rhel centos fedora"),
            DistroFamily::Fedora
        );
        assert_eq!(family_of("opensuse-tumbleweed", "suse"), DistroFamily::Suse);
        assert_eq!(family_of("nixos", ""), DistroFamily::Other);
    }

    #[test]
    fn session_kinds() {
        assert_eq!(
            kind(&given(&[("XDG_SESSION_TYPE", "wayland")])),
            SessionKind::Wayland
        );
        assert_eq!(
            kind(&given(&[("XDG_SESSION_TYPE", "x11"), ("DISPLAY", ":0")])),
            SessionKind::X11
        );
        // Without a session type the display variables decide, Wayland first.
        assert_eq!(
            kind(&given(&[
                ("WAYLAND_DISPLAY", "wayland-0"),
                ("DISPLAY", ":0")
            ])),
            SessionKind::Wayland
        );
        assert_eq!(kind(&given(&[("DISPLAY", ":1")])), SessionKind::X11);
        assert_eq!(
            kind(&given(&[("XDG_SESSION_TYPE", "tty")])),
            SessionKind::None
        );
        assert_eq!(kind(&given(&[("DISPLAY", "")])), SessionKind::None);
        assert!(matches!(
            session(&Env::Absent, 0),
            Value::Unknown {
                reason: UnknownReason::NotExposed,
                ..
            }
        ));
    }

    #[test]
    fn given_environment_is_all_a_probe_sees() {
        // A live probe with its own variables never reads the process environment.
        let opts = ProbeOptions::default().with_environment([("DISPLAY", ":3")]);
        let env = Env::for_probe(&SystemRoot::Live, &opts);
        assert_eq!(env.get("DISPLAY").as_deref(), Some(":3"));
        assert_eq!(env.get("PATH"), None);
    }
}
