// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) fuzz/fuzz_targets/edid.rs; SAD codec mapping
// replaced by the raw SAD accessors; the sink-key serial added.
//! EDID + CTA-861, the sink-key serial, and serial redaction (which must keep a parsable EDID
//! parsable and leave no serial behind).
#![no_main]

use displaycaps_parse::edid::{parse_edid, redact_serials, serial};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let parsed = parse_edid(data);
    if let Ok(e) = &parsed {
        for s in e.sads() {
            let _ = (s.format_code(), s.channels(), s.rates_hz(), s.lpcm_bit_depths());
            let _ = (s.max_bitrate_kbps(), s.extension_code());
        }
        let _ = (e.hdmi(), e.hdmi_forum().map(|f| f.max_frl_gbps()));
    }
    let _ = serial(data);
    let mut copy = data.to_vec();
    let _ = redact_serials(&mut copy);
    assert_eq!(copy.len(), data.len());
    if let Ok(before) = parsed {
        let after = parse_edid(&copy).expect("redaction keeps an EDID parsable");
        assert_eq!(before.cta, after.cta, "redaction must not touch extension blocks");
        assert_eq!(serial(&copy), None, "redaction leaves no sink-key serial");
    }
});
