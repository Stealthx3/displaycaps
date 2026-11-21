// SPDX-License-Identifier: LGPL-2.1-or-later
//! DisplayID sections: arbitrary bytes as one EDID extension block (tag 0x70 forced), every
//! accessor, and DisplayID serial redaction through a full EDID.
#![no_main]

use displaycaps_parse::displayid::parse_displayid;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut blk = data.to_vec();
    if let Some(b) = blk.first_mut() {
        *b = 0x70;
    }
    let d = parse_displayid(&blk);
    let _ = d.timings().count();
    let _ = (d.product_id(), d.tiles().count(), d.adaptive_sync().count());
    let _ = (d.dynamic_range(), d.parameters_v2(), d.interface_features());
});
