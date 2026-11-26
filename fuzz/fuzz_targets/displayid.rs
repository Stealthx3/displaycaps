// SPDX-License-Identifier: LGPL-2.1-or-later
//! DisplayID sections: arbitrary bytes as one EDID extension block (tag 0x70 forced), every
//! accessor, and redaction through a full EDID: afterwards no product serial, tile serial,
//! ASCII serial block or ContainerID may survive.
#![no_main]

use displaycaps_parse::displayid::{DidBlockData, parse_displayid};
use displaycaps_parse::edid::{BLOCK, parse_edid, redact_serials};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut blk = data.to_vec();
    blk.resize(BLOCK, 0);
    blk[0] = 0x70;
    let d = parse_displayid(&blk);
    let _ = d.timings().count();
    let _ = (d.product_id(), d.tiles().count(), d.adaptive_sync().count());
    let _ = (d.dynamic_range(), d.parameters_v2(), d.interface_features());

    // A minimal valid base block announcing one extension, then the fuzzed DisplayID block.
    let mut edid = vec![0u8; BLOCK];
    edid[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
    edid[126] = 1;
    edid.extend_from_slice(&blk);
    let _ = redact_serials(&mut edid);
    let e = parse_edid(&edid).expect("a valid header parses");
    for d in &e.displayid {
        if let Some(p) = d.product_id() {
            assert_eq!(p.serial, 0, "product serial redacted");
        }
        assert!(d.tiles().all(|t| t.serial == 0), "tile serials redacted");
        for b in &d.blocks {
            match &b.data {
                DidBlockData::ContainerId(u) => assert_eq!(u, &[0u8; 16], "ContainerID redacted"),
                DidBlockData::Text(t) if b.tag == 0x0A => assert!(t.is_empty(), "ASCII serial redacted"),
                _ => {}
            }
        }
    }
});
