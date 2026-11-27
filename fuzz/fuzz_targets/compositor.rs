// SPDX-License-Identifier: LGPL-2.1-or-later
//! Compositor facts (`commands/displaycaps-compositor.json` in fixtures) through the whole
//! compositor layer: identity, per-output views, matching and the layer itself.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    displaycaps_backends::fuzzing::compositor(data);
});
