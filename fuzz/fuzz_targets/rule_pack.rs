// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) fuzz/fuzz_targets/rule_pack.rs; renamed for DisplayCaps.
//! Rule and quirk packs: arbitrary pack text loads without panicking, and every loaded rule
//! evaluates on arbitrary facts. Input: pack text, a NUL byte, facts JSON.
#![no_main]

use displaycaps_rules::RuleSet;
use displaycaps_rules::engine::fired;
use displaycaps_rules::facts::Facts;
use displaycaps_rules::quirks::QuirkSet;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (pack, facts) = match data.iter().position(|b| *b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &b"{}"[..]),
    };
    let pack = String::from_utf8_lossy(pack);
    let set = RuleSet::load([("fuzz".to_string(), pack.as_ref())]);
    let _ = QuirkSet::load([("fuzz".to_string(), pack.as_ref())]);
    if let Ok(j) = serde_json::from_slice::<serde_json::Value>(facts) {
        let f = Facts::from_json(&j);
        let _ = fired(&set, &f);
        let _ = fired(&RuleSet::bundled(), &f);
    }
});
