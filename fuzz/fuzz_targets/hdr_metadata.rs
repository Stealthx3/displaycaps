// SPDX-License-Identifier: LGPL-2.1-or-later
//! `HDR_OUTPUT_METADATA` property blobs.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    displaycaps_backends::fuzzing::hdr_metadata(data);
});
