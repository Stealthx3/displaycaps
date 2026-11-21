// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-rules/src/engine.rs; hops, layers
// and hint selection adapted to display (compositor-keyed hints, presentation paths).
//! The rule engine: declarative rules over [`Facts`].
//!
//! A rule pack is JSON: `{"format": 1, "rules": [...]}`. A rule:
//!
//! ```json
//! { "code": "SANDBOX_PERMISSION_MISSING", "severity": "warning", "hop": "environment",
//!   "scope": "system",
//!   "when": { "nonempty": "g.sandbox.required_missing" },
//!   "evidence": ["g.sandbox.kind", "g.sandbox.required_missing"],
//!   "hint": { "generic": "Grant {g.sandbox.required_missing} …", "debian": "…" } }
//! ```
//!
//! Expressions: `all`, `any`, `not`, `known`, `eq`, `ne`, `gt`, `ge`, `lt`, `le`, `contains`,
//! `nonempty`, `intersects`, `subset`, `not_subset`. Operands are fact names or literals; a string
//! starting with `$` is always a fact name (`"$e.sink.max_tmds_mhz"`), so literal strings can't be
//! mistaken for facts. A comparison with an absent fact is false, so `not` around it is true:
//! write `known` guards inside `not` when that matters.

use serde::Deserialize;
use serde_json::Value as J;

use displaycaps_model::{
    CompositorKind, Confidence, DistroFamily, Evidence, Finding, FindingCode, Hint, HopKind,
    Severity,
};

use crate::facts::{Facts, derived};

/// Version of the rule-pack format this build reads.
pub const RULE_PACK_FORMAT: u32 = 1;

/// The bundled starter pack.
pub const BUNDLED: &str = include_str!("../packs/rules-core.json");

/// Whether a rule looks at each endpoint or once at the whole system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    /// Once per endpoint in scope.
    #[default]
    Endpoint,
    /// Once for the system.
    System,
}

/// One rule.
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    /// Stable code.
    pub code: String,
    /// Severity.
    pub severity: Severity,
    /// The hop it concerns.
    pub hop: HopKind,
    /// Endpoint or system rule.
    #[serde(default)]
    pub scope: RuleScope,
    /// Condition.
    pub when: J,
    /// Facts reported as evidence.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Confidence when every evidence fact is measured (lowered to the evidence's otherwise).
    #[serde(default = "measured")]
    pub confidence: Confidence,
    /// Hints by compositor (`kwin`, `mutter`, `wlroots`, `hyprland`, `gamescope`, `weston`,
    /// `xorg`, `other`), by distro family (`generic`, `debian`, `fedora`, `arch`, `suse`).
    /// A compositor hint wins over a distro hint, which wins over `generic`.
    #[serde(default)]
    pub hint: std::collections::BTreeMap<String, String>,
    /// What the rule checks, for documentation.
    #[serde(default)]
    pub description: String,
    /// Report only the first endpoint it fires on (system-wide conditions seen per endpoint).
    #[serde(default)]
    pub once: bool,
    /// Facts the library doesn't produce yet; until it does, the rule can't fire.
    #[serde(default)]
    pub inputs_pending: Vec<String>,
}

fn measured() -> Confidence {
    Confidence::Measured
}

#[derive(Debug, Deserialize)]
struct Pack {
    format: u32,
    #[serde(default)]
    rules: Vec<J>,
}

/// A problem with one pack or rule; the rest still loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadIssue {
    /// The pack's origin.
    pub origin: String,
    /// What was wrong.
    pub detail: String,
}

/// Loaded rules, later packs overriding earlier ones by code.
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    /// Rules with their origin.
    pub rules: Vec<(Rule, String)>,
    /// Packs or rules that were skipped.
    pub issues: Vec<LoadIssue>,
}

impl RuleSet {
    /// Loads packs in order (bundled first, then overrides). A rule with an empty `when` (`null`)
    /// in a later pack disables the code.
    pub fn load<'a>(packs: impl IntoIterator<Item = (String, &'a str)>) -> Self {
        let mut set = RuleSet::default();
        for (origin, text) in packs {
            let pack: Pack = match serde_json::from_str(text) {
                Ok(p) => p,
                Err(e) => {
                    set.issues.push(LoadIssue {
                        origin,
                        detail: format!("not a rule pack: {e}"),
                    });
                    continue;
                }
            };
            if pack.format != RULE_PACK_FORMAT {
                set.issues.push(LoadIssue {
                    origin,
                    detail: format!(
                        "format {} (this build reads {RULE_PACK_FORMAT})",
                        pack.format
                    ),
                });
                continue;
            }
            for raw in pack.rules {
                match serde_json::from_value::<Rule>(raw) {
                    Ok(r) => {
                        if let Err(e) = check(&r.when) {
                            set.issues.push(LoadIssue {
                                origin: origin.clone(),
                                detail: format!("{}: {e}", r.code),
                            });
                            continue;
                        }
                        set.rules.retain(|(x, _)| x.code != r.code);
                        if !r.when.is_null() {
                            set.rules.push((r, origin.clone()));
                        }
                    }
                    Err(e) => set.issues.push(LoadIssue {
                        origin: origin.clone(),
                        detail: format!("rule skipped: {e}"),
                    }),
                }
            }
        }
        set
    }

    /// The bundled pack alone.
    pub fn bundled() -> Self {
        Self::load([("bundled".to_string(), BUNDLED)])
    }

    /// The rule with this code.
    pub fn rule(&self, code: &str) -> Option<&Rule> {
        self.rules.iter().map(|(r, _)| r).find(|r| r.code == code)
    }
}

const OPS: &[&str] = &[
    "all",
    "any",
    "not",
    "known",
    "eq",
    "ne",
    "gt",
    "ge",
    "lt",
    "le",
    "contains",
    "nonempty",
    "intersects",
    "subset",
    "not_subset",
];

/// Validates an expression's shape.
fn check(e: &J) -> Result<(), String> {
    if e.is_null() {
        return Ok(());
    }
    let m = e.as_object().ok_or("expression must be an object")?;
    if m.len() != 1 {
        return Err("expression must have exactly one operator".into());
    }
    let (op, arg) = m.iter().next().expect("one entry");
    if !OPS.contains(&op.as_str()) {
        return Err(format!("unknown operator {op}"));
    }
    match op.as_str() {
        "all" | "any" => arg
            .as_array()
            .ok_or(format!("{op} takes a list"))?
            .iter()
            .try_for_each(check),
        "not" => check(arg),
        "known" | "nonempty" => arg
            .as_str()
            .map(|_| ())
            .ok_or(format!("{op} takes a fact name")),
        _ => match arg.as_array() {
            Some(a) if a.len() == 2 && a[0].is_string() => Ok(()),
            _ => Err(format!("{op} takes [fact, value]")),
        },
    }
}

/// Resolves an operand: the first operand is always a fact name; later ones are facts when they
/// start with `$`, else literals.
fn operand<'a>(facts: &'a Facts, v: &'a J, first: bool) -> Option<&'a J> {
    match v.as_str() {
        Some(s) if first => facts.get(s.trim_start_matches('$')).map(|f| &f.value),
        Some(s) if s.starts_with('$') => facts.get(&s[1..]).map(|f| &f.value),
        _ => Some(v),
    }
}

fn as_list(v: &J) -> Vec<&J> {
    match v {
        J::Array(a) => a.iter().collect(),
        other => vec![other],
    }
}

fn num(v: &J) -> Option<f64> {
    v.as_f64()
}

/// Evaluates an expression. This is a closed interpreter over the operators listed in `OPS`
/// comparing JSON fact values; rule packs are data and can't run code.
pub fn eval(e: &J, facts: &Facts) -> bool {
    let Some((op, arg)) = e.as_object().and_then(|m| m.iter().next()) else {
        return false;
    };
    let pair = || -> Option<(&J, &J)> {
        let a = arg.as_array()?;
        Some((operand(facts, &a[0], true)?, operand(facts, &a[1], false)?))
    };
    match op.as_str() {
        "all" => arg
            .as_array()
            .is_some_and(|a| a.iter().all(|x| eval(x, facts))),
        "any" => arg
            .as_array()
            .is_some_and(|a| a.iter().any(|x| eval(x, facts))),
        "not" => !eval(arg, facts),
        "known" => arg.as_str().is_some_and(|n| facts.get(n).is_some()),
        "nonempty" => arg
            .as_str()
            .and_then(|n| facts.get(n))
            .is_some_and(|f| f.value.as_array().is_some_and(|a| !a.is_empty())),
        "eq" => pair().is_some_and(|(a, b)| a == b),
        "ne" => pair().is_some_and(|(a, b)| a != b),
        "gt" | "ge" | "lt" | "le" => pair().is_some_and(|(a, b)| match (num(a), num(b)) {
            (Some(x), Some(y)) => match op.as_str() {
                "gt" => x > y,
                "ge" => x >= y,
                "lt" => x < y,
                _ => x <= y,
            },
            _ => false,
        }),
        "contains" => pair().is_some_and(|(a, b)| as_list(a).contains(&b)),
        "intersects" => pair().is_some_and(|(a, b)| {
            let bl = as_list(b);
            as_list(a).iter().any(|x| bl.contains(x))
        }),
        "subset" => pair().is_some_and(|(a, b)| {
            let bl = as_list(b);
            as_list(a).iter().all(|x| bl.contains(x))
        }),
        "not_subset" => pair().is_some_and(|(a, b)| {
            let bl = as_list(b);
            as_list(a).iter().any(|x| !bl.contains(x))
        }),
        _ => false,
    }
}

fn text(v: &J) -> String {
    match v {
        J::String(s) => s.clone(),
        J::Array(a) => a.iter().map(text).collect::<Vec<_>>().join(", "),
        J::Null => String::new(),
        other => other.to_string(),
    }
}

/// Fills `{fact.name}` placeholders.
fn render(t: &str, facts: &Facts) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('}') {
            Some(j) => {
                let name = &after[..j];
                match facts.get(name) {
                    Some(f) => out.push_str(&text(&f.value)),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Applies one rule to a fact set. Returns the finding when it fires.
pub fn apply(
    rule: &Rule,
    facts: &Facts,
    family: Option<DistroFamily>,
    compositor: Option<CompositorKind>,
    at_ms: u64,
) -> Option<Finding> {
    if !eval(&rule.when, facts) {
        return None;
    }
    let layer = hop_layer(rule.hop);
    let mut confidence = rule.confidence;
    let evidence: Vec<Evidence> = rule
        .evidence
        .iter()
        .filter_map(|n| {
            let f = facts.get(n)?;
            if f.confidence == Confidence::Inferred {
                confidence = Confidence::Inferred;
            }
            Some(Evidence {
                label: n.clone(),
                value: f.value.clone(),
                provenance: f
                    .provenance
                    .clone()
                    .unwrap_or_else(|| derived(layer, at_ms, "rule engine facts")),
            })
        })
        .collect();
    let family_key = family.map(|f| match f {
        DistroFamily::Debian => "debian",
        DistroFamily::Fedora => "fedora",
        DistroFamily::Arch => "arch",
        DistroFamily::Suse => "suse",
        DistroFamily::Other => "generic",
    });
    let compositor_key = compositor.map(compositor_name);
    let hint = compositor_key
        .and_then(|k| rule.hint.get(k).map(|t| (t, None, compositor)))
        .or_else(|| {
            family_key.and_then(|k| {
                rule.hint
                    .get(k)
                    .map(|t| (t, family, None::<CompositorKind>))
            })
        })
        .or_else(|| rule.hint.get("generic").map(|t| (t, None, None)))
        .map(|(t, fam, comp)| Hint {
            text: render(t, facts),
            distro_family: fam,
            compositor: comp,
        });
    Some(Finding {
        code: FindingCode(rule.code.clone()),
        severity: rule.severity,
        endpoint: None,
        presentation_path: None,
        hop: rule.hop,
        evidence,
        confidence,
        hint,
    })
}

fn compositor_name(c: CompositorKind) -> &'static str {
    match c {
        CompositorKind::Kwin => "kwin",
        CompositorKind::Mutter => "mutter",
        CompositorKind::Wlroots => "wlroots",
        CompositorKind::Hyprland => "hyprland",
        CompositorKind::Gamescope => "gamescope",
        CompositorKind::Weston => "weston",
        CompositorKind::Xorg => "xorg",
        CompositorKind::Other => "other",
    }
}

fn hop_layer(h: HopKind) -> displaycaps_model::LayerKind {
    use displaycaps_model::LayerKind as L;
    match h {
        HopKind::Application | HopKind::WholePath | HopKind::Environment => L::Environment,
        HopKind::Decoder => L::Decode,
        HopKind::Renderer | HopKind::Compositor => L::Compositor,
        HopKind::KmsDriver => L::Kms,
        HopKind::Link => L::Link,
        HopKind::Sink | HopKind::SinkChain => L::SinkChain,
    }
}

/// A rule test case from a `*.cases.json` file: facts and the codes expected to fire.
#[derive(Debug, Deserialize)]
pub struct Case {
    /// What the case checks.
    pub name: String,
    /// Facts.
    pub facts: J,
    /// Codes that must fire.
    #[serde(default)]
    pub fires: Vec<String>,
    /// Codes that must not fire.
    #[serde(default)]
    pub silent: Vec<String>,
}

/// Runs every rule on a fact set and returns the codes that fired.
pub fn fired(set: &RuleSet, facts: &Facts) -> Vec<String> {
    set.rules
        .iter()
        .filter(|(r, _)| eval(&r.when, facts))
        .map(|(r, _)| r.code.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts(j: J) -> Facts {
        Facts::from_json(&j)
    }

    #[test]
    fn operators() {
        let f = facts(json!({"a": [1, 2], "b": [1, 2, 3], "n": 5, "s": "x", "e": []}));
        assert!(eval(&json!({"subset": ["a", "$b"]}), &f));
        assert!(eval(&json!({"not_subset": ["b", "$a"]}), &f));
        assert!(eval(&json!({"gt": ["n", 4]}), &f));
        assert!(!eval(&json!({"gt": ["missing", 4]}), &f));
        assert!(eval(&json!({"eq": ["s", "x"]}), &f));
        assert!(eval(&json!({"contains": ["a", 2]}), &f));
        assert!(!eval(&json!({"nonempty": "e"}), &f));
        assert!(eval(
            &json!({"all": [{"known": "n"}, {"not": {"known": "zz"}}]}),
            &f
        ));
        assert!(check(&json!({"bogus": 1})).is_err());
        assert!(check(&json!({"eq": [1, 2]})).is_err());
    }

    #[test]
    fn hints_render_and_pick_family() {
        let r: Rule = serde_json::from_value(json!({
            "code": "X", "severity": "info", "hop": "sink", "when": {"known": "a"},
            "evidence": ["a"], "hint": {"generic": "fix {a}", "debian": "apt {a}"}
        }))
        .unwrap();
        let f = facts(json!({"a": ["p", "q"]}));
        let x = apply(&r, &f, Some(DistroFamily::Debian), None, 0).unwrap();
        assert_eq!(x.hint.unwrap().text, "apt p, q");
        let y = apply(&r, &f, Some(DistroFamily::Arch), None, 0).unwrap();
        assert_eq!(y.hint.as_ref().unwrap().text, "fix p, q");
        assert_eq!(y.hint.unwrap().distro_family, None);
    }

    #[test]
    fn compositor_hint_wins_over_distro_and_generic() {
        let r: Rule = serde_json::from_value(json!({
            "code": "X", "severity": "info", "hop": "compositor", "when": {"known": "a"},
            "evidence": ["a"],
            "hint": {"generic": "fix", "debian": "apt", "kwin": "System Settings > Display"}
        }))
        .unwrap();
        let f = facts(json!({"a": 1}));
        let x = apply(
            &r,
            &f,
            Some(DistroFamily::Debian),
            Some(CompositorKind::Kwin),
            0,
        )
        .unwrap();
        let h = x.hint.unwrap();
        assert_eq!(h.text, "System Settings > Display");
        assert_eq!(h.compositor, Some(CompositorKind::Kwin));
        assert_eq!(h.distro_family, None);
        // A compositor without its own hint falls back to the distro hint.
        let y = apply(
            &r,
            &f,
            Some(DistroFamily::Debian),
            Some(CompositorKind::Mutter),
            0,
        )
        .unwrap();
        assert_eq!(y.hint.unwrap().text, "apt");
        assert_eq!(x.presentation_path, None);
    }

    #[test]
    fn evidence_carries_provenance_and_missing_facts_are_skipped() {
        let r: Rule = serde_json::from_value(json!({
            "code": "X", "severity": "warning", "hop": "link", "when": {"known": "a"},
            "evidence": ["a", "missing"]
        }))
        .unwrap();
        let f = facts(json!({"a": 3}));
        let x = apply(&r, &f, None, None, 42).unwrap();
        assert_eq!(x.evidence.len(), 1);
        assert_eq!(x.evidence[0].provenance.read_at_ms, 42);
        assert_eq!(
            x.evidence[0].provenance.layer,
            displaycaps_model::LayerKind::Link
        );
        assert_eq!(x.confidence, Confidence::Measured);
        assert!(x.hint.is_none());
    }
}
