// SPDX-License-Identifier: LGPL-2.1-or-later
//! `drm_info -j` JSON (fixture KMS replay) and its mapping to KMS state.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    displaycaps_backends::fuzzing::drm_info(data);
});
