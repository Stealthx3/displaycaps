// SPDX-License-Identifier: LGPL-2.1-or-later
//! Kernel uevent messages (DRM hotplug).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    displaycaps_backends::fuzzing::uevent(data);
});
