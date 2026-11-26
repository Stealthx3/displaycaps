// SPDX-License-Identifier: LGPL-2.1-or-later
// Adapted from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-backends/src/lib.rs.
//! DisplayCaps system access.
//!
//! - [`root::SystemRoot`]: every reader works on the live system or a captured fixture tree.
//! - [`manifest`]: the capture manifest shared with the capture tool.
//! - [`runtime::Runtime`]: the probe thread with per-backend deadlines and snapshot generations.
//! - [`probe::probe`]: one scoped probe without deadlines (fixtures, tests, tools).
//! - [`watch`]: change events for a watched scope (manual or timed triggers; hotplug and
//!   compositor event sources arrive with their backends).
//!
//! The environment layer (os-release, kernel, sandbox, display session) is read here; the other
//! layers report `unknown` with reason `not_implemented` until their backends exist (phases 3 to
//! 6). Nothing outside the probe scope is read or reported.

mod environment;
pub mod manifest;
pub mod probe;
pub mod root;
pub mod runtime;
pub mod sink;
mod sandbox;
pub mod watch;

pub use probe::{probe, probe_with};
pub use root::SystemRoot;
pub use runtime::{Backend, ProbeOptions, Runtime};

#[cfg(test)]
pub(crate) mod testutil {
    //! A throwaway fixture directory for tests.

    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::manifest::{CAPTURE_FORMAT, CapturedSandbox, Manifest};

    static N: AtomicU32 = AtomicU32::new(0);

    /// A fixture directory removed on drop.
    pub(crate) struct TempFixture {
        pub dir: PathBuf,
    }

    impl TempFixture {
        /// Creates a fixture whose `/etc/os-release` is `os_release`.
        pub(crate) fn new(os_release: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "displaycaps-fixture-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(dir.join("root/etc")).expect("create fixture");
            std::fs::create_dir_all(dir.join("commands")).expect("create fixture");
            let manifest = Manifest {
                format: CAPTURE_FORMAT,
                tool_version: "test".into(),
                captured_at_ms: 1_000,
                sandbox: CapturedSandbox::default(),
                file_count: 1,
                commands: vec![],
                redaction: vec![],
                notes: vec![],
            };
            std::fs::write(
                dir.join("manifest.json"),
                serde_json::to_vec(&manifest).expect("manifest"),
            )
            .expect("write manifest");
            let f = Self { dir };
            f.set_os_release(os_release);
            f
        }

        /// Replaces `/etc/os-release` in the fixture.
        pub(crate) fn set_os_release(&self, text: &str) {
            std::fs::write(self.path().join("root/etc/os-release"), text).expect("write");
        }

        pub(crate) fn path(&self) -> &Path {
            &self.dir
        }
    }

    impl Drop for TempFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}
