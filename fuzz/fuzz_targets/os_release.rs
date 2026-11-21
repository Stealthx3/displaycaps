// SPDX-License-Identifier: LGPL-2.1-or-later
// Adapted from AudioCaps v1.0.0 (commit 049d053) fuzz/fuzz_targets/procfs.rs (its os-release arm).
//! `os-release` text.
#![no_main]

use displaycaps_parse::os_release;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(os_release::parse(&String::from_utf8_lossy(data)));
});
