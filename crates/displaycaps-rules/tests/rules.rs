// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-rules/tests/rules.rs; rule count
// and override examples adapted to the DisplayCaps pack.
//! Rule pack gate: each bundled rule fires only when it should. Every rule has at least one case
//! where it fires and one near miss where it must stay silent (`packs/rules-core.cases.json`).

use displaycaps_rules::RuleSet;
use displaycaps_rules::engine::{Case, fired};
use displaycaps_rules::facts::Facts;

fn cases() -> Vec<Case> {
    let j: serde_json::Value =
        serde_json::from_str(include_str!("../packs/rules-core.cases.json")).unwrap();
    serde_json::from_value(j["cases"].clone()).unwrap()
}

#[test]
fn bundled_pack_loads_cleanly() {
    let r = RuleSet::bundled();
    assert!(r.issues.is_empty(), "{:?}", r.issues);
    assert_eq!(r.rules.len(), 1);
    for (rule, _) in &r.rules {
        assert!(
            rule.hint.contains_key("generic"),
            "{}: generic hint",
            rule.code
        );
        assert!(!rule.evidence.is_empty(), "{}: evidence", rule.code);
        assert!(!rule.description.is_empty(), "{}: description", rule.code);
    }
}

#[test]
fn every_rule_has_a_firing_case_and_a_near_miss() {
    let r = RuleSet::bundled();
    let cs = cases();
    for (rule, _) in &r.rules {
        assert!(
            cs.iter().any(|c| c.fires.contains(&rule.code)),
            "{}: no firing case",
            rule.code
        );
        assert!(
            cs.iter().any(|c| c.silent.contains(&rule.code)),
            "{}: no near miss",
            rule.code
        );
    }
}

#[test]
fn cases_hold() {
    let r = RuleSet::bundled();
    let mut bad = vec![];
    for c in cases() {
        let got = fired(&r, &Facts::from_json(&c.facts));
        for f in &c.fires {
            if !got.contains(f) {
                bad.push(format!("{}: {f} did not fire", c.name));
            }
        }
        for s in &c.silent {
            if got.contains(s) {
                bad.push(format!("{}: {s} fired", c.name));
            }
        }
        // Nothing else fires on a case built for one rule.
        for g in &got {
            if !c.fires.contains(g) {
                bad.push(format!("{}: unexpected {g}", c.name));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn empty_facts_fire_nothing() {
    assert!(fired(&RuleSet::bundled(), &Facts::default()).is_empty());
}

#[test]
fn override_disables_and_replaces() {
    let local = r#"{"format":1,"rules":[
        {"code":"SANDBOX_PERMISSION_MISSING","severity":"error","hop":"environment",
         "when":{"known":"g.sandbox.kind"},"evidence":["g.sandbox.kind"],
         "hint":{"generic":"x"},"description":"local"}]}"#;
    let r = RuleSet::load([
        ("bundled".to_string(), displaycaps_rules::engine::BUNDLED),
        ("/etc/displaycaps/rules.d/local.json".to_string(), local),
    ]);
    assert_eq!(
        r.rule("SANDBOX_PERMISSION_MISSING").unwrap().description,
        "local"
    );
    // A null `when` in a later pack disables the code.
    let off = r#"{"format":1,"rules":[
        {"code":"SANDBOX_PERMISSION_MISSING","severity":"info","hop":"environment","when":null}]}"#;
    let r = RuleSet::load([
        ("bundled".to_string(), displaycaps_rules::engine::BUNDLED),
        ("/etc/displaycaps/rules.d/off.json".to_string(), off),
    ]);
    assert!(r.rule("SANDBOX_PERMISSION_MISSING").is_none());
    // A rule with an unknown operator is skipped and reported; the rest of the pack stays.
    let bad = RuleSet::load([(
        "x".to_string(),
        r#"{"format":1,"rules":[{"code":"Y","severity":"info","hop":"sink","when":{"nope":1}}]}"#,
    )]);
    assert_eq!(bad.issues.len(), 1);
    assert!(bad.rules.is_empty());
}
