// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-rules/src/quirks.rs; match gains
// substring model names, effects are the display QuirkEffect, bundled pack and tests replaced.
//! Sink quirk data: facts about specific sinks that no EDID or CEC field carries, or that a sink
//! misreports. Packs are JSON (`packs/quirks-core.json` is bundled); later packs override entries
//! with the same id. Entries match by PNP manufacturer id plus a product code or a model name
//! (EDID monitor name or CEC OSD name; exact, or a case-insensitive substring), or by CEC vendor id
//! plus a model name.

use serde::Deserialize;

use displaycaps_model::{AppliedQuirk, QuirkEffect};

/// Version of the quirk-pack format this build reads.
pub const QUIRK_PACK_FORMAT: u32 = 1;

/// The bundled pack.
pub const BUNDLED: &str = include_str!("../packs/quirks-core.json");

#[derive(Debug, Deserialize)]
struct Pack {
    format: u32,
    #[serde(default)]
    entries: Vec<serde_json::Value>,
}

/// One quirk entry.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Entry {
    /// Stable id.
    pub id: String,
    /// What the entry corrects or adds.
    pub description: String,
    /// Where the facts come from.
    #[serde(default)]
    pub reference: Option<String>,
    /// Match criteria.
    #[serde(rename = "match")]
    pub matcher: Match,
    /// Effects.
    pub effects: Vec<QuirkEffect>,
}

/// Match criteria: the manufacturer (or CEC vendor) and at least one more identifier must match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Match {
    /// PNP manufacturer id, e.g. `DON`.
    #[serde(default)]
    pub manufacturer: Option<String>,
    /// EDID product code.
    #[serde(default)]
    pub product_code: Option<u16>,
    /// Model names (EDID monitor name or CEC OSD name), compared case-insensitively.
    #[serde(default)]
    pub model_names: Vec<String>,
    /// Fragments of a model name, matched case-insensitively anywhere in it (for families whose
    /// EDID name carries a suffix, e.g. `LG TV SSCR2`).
    #[serde(default)]
    pub model_name_contains: Vec<String>,
    /// CEC vendor id (OUI, hex).
    #[serde(default)]
    pub cec_vendor: Option<String>,
}

/// What is known about a sink, for matching.
#[derive(Debug, Clone, Default)]
pub struct SinkKey<'a> {
    /// PNP manufacturer id.
    pub manufacturer: Option<&'a str>,
    /// EDID product code.
    pub product_code: Option<u16>,
    /// Model names seen (EDID name, CEC OSD name).
    pub model_names: Vec<&'a str>,
    /// CEC vendor id (hex).
    pub cec_vendor: Option<&'a str>,
}

impl Match {
    fn matches(&self, k: &SinkKey<'_>) -> bool {
        let maker = match (&self.manufacturer, k.manufacturer) {
            (Some(m), Some(km)) if m.eq_ignore_ascii_case(km) => true,
            _ => {
                matches!((&self.cec_vendor, k.cec_vendor), (Some(v), Some(kv)) if v.eq_ignore_ascii_case(kv))
            }
        };
        if !maker {
            return false;
        }
        let product = self.product_code.is_some() && self.product_code == k.product_code;
        let name = self.model_names.iter().any(|n| {
            k.model_names
                .iter()
                .any(|kn| kn.trim().eq_ignore_ascii_case(n))
        });
        let fragment = self.model_name_contains.iter().any(|n| {
            let n = n.to_ascii_lowercase();
            !n.is_empty()
                && k.model_names
                    .iter()
                    .any(|kn| kn.to_ascii_lowercase().contains(&n))
        });
        product || name || fragment
    }
}

/// A problem with one pack or entry; the rest still loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadIssue {
    /// The pack's origin.
    pub origin: String,
    /// What was wrong.
    pub detail: String,
}

/// Loaded quirk entries, later origins overriding earlier ones by id.
#[derive(Debug, Clone, Default)]
pub struct QuirkSet {
    entries: Vec<(Entry, String)>,
    /// Packs or entries that were skipped.
    pub issues: Vec<LoadIssue>,
}

impl QuirkSet {
    /// Loads packs in order (bundled first, then overrides). Each is `(origin, json text)`.
    pub fn load<'a>(packs: impl IntoIterator<Item = (String, &'a str)>) -> Self {
        let mut set = QuirkSet::default();
        for (origin, text) in packs {
            let pack: Pack = match serde_json::from_str(text) {
                Ok(p) => p,
                Err(e) => {
                    set.issues.push(LoadIssue {
                        origin,
                        detail: format!("not a quirk pack: {e}"),
                    });
                    continue;
                }
            };
            if pack.format != QUIRK_PACK_FORMAT {
                set.issues.push(LoadIssue {
                    origin,
                    detail: format!(
                        "format {} (this build reads {QUIRK_PACK_FORMAT})",
                        pack.format
                    ),
                });
                continue;
            }
            for raw in pack.entries {
                match serde_json::from_value::<Entry>(raw) {
                    Ok(e) => {
                        set.entries.retain(|(x, _)| x.id != e.id);
                        set.entries.push((e, origin.clone()));
                    }
                    Err(err) => set.issues.push(LoadIssue {
                        origin: origin.clone(),
                        detail: format!("entry skipped: {err}"),
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

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries matching a sink, as applied quirks.
    pub fn matching(&self, key: &SinkKey<'_>) -> Vec<AppliedQuirk> {
        self.entries
            .iter()
            .filter(|(e, _)| e.matcher.matches(key))
            .map(|(e, origin)| AppliedQuirk {
                id: e.id.clone(),
                description: e.description.clone(),
                origin: origin.clone(),
                reference: e.reference.clone(),
                effects: e.effects.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_pack_loads_and_matches() {
        let q = QuirkSet::bundled();
        assert!(q.issues.is_empty(), "{:?}", q.issues);
        assert!(!q.is_empty());
        let k = SinkKey {
            manufacturer: Some("GSM"),
            model_names: vec!["LG TV SSCR2"],
            ..SinkKey::default()
        };
        let m = q.matching(&k);
        assert_eq!(m[0].id, "lg-cx-zero-peak-luminance");
        assert_eq!(m[0].origin, "bundled");
        assert_eq!(m[0].reference.as_deref(), Some("KDE bug 495557"));
        assert!(matches!(m[0].effects[0], QuirkEffect::Note { .. }));
        // The name match is case-insensitive.
        let k = SinkKey {
            manufacturer: Some("gsm"),
            model_names: vec!["lg tv"],
            ..SinkKey::default()
        };
        assert_eq!(q.matching(&k).len(), 1);
        // Manufacturer alone is not enough.
        let k = SinkKey {
            manufacturer: Some("GSM"),
            ..SinkKey::default()
        };
        assert!(q.matching(&k).is_empty());
        // The name alone is not enough either.
        let k = SinkKey {
            manufacturer: Some("SAM"),
            model_names: vec!["LG TV SSCR2"],
            ..SinkKey::default()
        };
        assert!(q.matching(&k).is_empty());
        // Another LG monitor line does not carry "LG TV".
        let k = SinkKey {
            manufacturer: Some("GSM"),
            model_names: vec!["LG ULTRAGEAR"],
            ..SinkKey::default()
        };
        assert!(q.matching(&k).is_empty());
    }

    #[test]
    fn product_code_and_cec_vendor_matching() {
        let pack = r#"{"format":1,"entries":[
            {"id":"by-code","description":"d","match":{"manufacturer":"ABC","product_code":4660},"effects":[{"effect":"fixed_edid"}]},
            {"id":"by-cec","description":"d","match":{"cec_vendor":"00E091","model_names":["Box"]},"effects":[]}]}"#;
        let q = QuirkSet::load([("t".to_string(), pack)]);
        assert!(q.issues.is_empty(), "{:?}", q.issues);
        let k = SinkKey {
            manufacturer: Some("ABC"),
            product_code: Some(4660),
            ..SinkKey::default()
        };
        assert_eq!(q.matching(&k)[0].effects, [QuirkEffect::FixedEdid]);
        let k = SinkKey {
            cec_vendor: Some("00e091"),
            model_names: vec!["box"],
            ..SinkKey::default()
        };
        assert_eq!(q.matching(&k)[0].id, "by-cec");
        let k = SinkKey {
            manufacturer: Some("ABC"),
            product_code: Some(1),
            ..SinkKey::default()
        };
        assert!(q.matching(&k).is_empty());
    }

    #[test]
    fn overrides_and_bad_packs() {
        let over = r#"{"format":1,"entries":[{"id":"lg-cx-zero-peak-luminance","description":"local",
            "match":{"manufacturer":"GSM","product_code":4660},"effects":[]},
            {"id":"broken"}]}"#;
        let q = QuirkSet::load([
            ("bundled".to_string(), BUNDLED),
            ("/etc/displaycaps/quirks.d/x.json".to_string(), over),
            ("bad".to_string(), "not json"),
            ("future".to_string(), r#"{"format":9,"entries":[]}"#),
        ]);
        assert_eq!(q.len(), 1);
        assert_eq!(q.issues.len(), 3);
        let k = SinkKey {
            manufacturer: Some("GSM"),
            product_code: Some(4660),
            ..SinkKey::default()
        };
        let m = q.matching(&k);
        assert_eq!(
            (m[0].description.as_str(), m[0].origin.as_str()),
            ("local", "/etc/displaycaps/quirks.d/x.json")
        );
    }
}
