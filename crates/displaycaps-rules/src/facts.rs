// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-rules/src/facts.rs; the generic
// container is kept, the audio fact extraction is dropped and a small display set is added.
//! Facts: a snapshot flattened into named values that rules compare. Each fact keeps the
//! provenance and confidence of the model value it came from.
//!
//! Names: `g.*` system-wide, `e.*` the endpoint being checked. A fact that can't be known is
//! simply absent; rules never fire on an absent fact. Display facts (modes, HDR, link budget) are
//! added with the starter rule pack in phase 7.
//!
//! | Fact | Meaning |
//! |---|---|
//! | `g.os.family` | `debian`, `fedora`, `arch`, `suse` or `other` |
//! | `g.sandbox.kind` | `none`, `flatpak` or `snap` |
//! | `g.sandbox.missing`, `g.sandbox.fixes` | missing permissions and their fixes |
//! | `g.sandbox.required_missing`, `g.sandbox.required_fixes` | the same, only for required ones |
//! | `g.session.kind` | `wayland`, `x11` or `none` |
//! | `g.compositor.kind` | the compositor, when identified |
//! | `e.class` | `external`, `internal` or `virtual` |
//! | `e.connector` | `hdmi`, `display_port`, `usb_c`, `edp`, ... |
//! | `e.status` | `connected`, `disconnected` or `unknown` |

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value as J, json};

use displaycaps_model::{
    Confidence, Endpoint, LayerKind, Provenance, Sandbox, Snapshot, Source, Value,
};

/// One fact.
#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    /// The value.
    pub value: J,
    /// Where it came from.
    pub provenance: Option<Provenance>,
    /// How sure it is.
    pub confidence: Confidence,
}

/// A set of facts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    map: BTreeMap<String, Fact>,
}

impl Facts {
    /// A fact by name.
    pub fn get(&self, name: &str) -> Option<&Fact> {
        self.map.get(name)
    }

    /// Sets a derived fact.
    pub fn set(&mut self, name: &str, value: J) {
        if !value.is_null() {
            self.map.insert(
                name.into(),
                Fact {
                    value,
                    provenance: None,
                    confidence: Confidence::Measured,
                },
            );
        }
    }

    /// Sets a fact from a model value; an unknown value leaves the fact absent.
    pub fn set_value<T>(&mut self, name: &str, v: &Value<T>, f: impl Fn(&T) -> J) {
        if let Value::Known {
            value,
            confidence,
            provenance,
        } = v
        {
            let j = f(value);
            if !j.is_null() {
                self.map.insert(
                    name.into(),
                    Fact {
                        value: j,
                        provenance: Some(provenance.clone()),
                        confidence: *confidence,
                    },
                );
            }
        }
    }

    /// Merges another set (later wins).
    pub fn extend(&mut self, other: &Facts) {
        for (k, v) in &other.map {
            self.map.insert(k.clone(), v.clone());
        }
    }

    /// The same facts without those whose name starts with `prefix`.
    pub fn without_prefix(&self, prefix: &str) -> Facts {
        Facts {
            map: self
                .map
                .iter()
                .filter(|(k, _)| !k.starts_with(prefix))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }

    /// All names (for documentation and tests).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    /// Builds facts from plain JSON (`{"e.connector": "hdmi", …}`), for rule tests.
    pub fn from_json(j: &J) -> Self {
        let mut f = Facts::default();
        if let Some(m) = j.as_object() {
            for (k, v) in m {
                f.set(k, v.clone());
            }
        }
        f
    }
}

/// The model's snake_case name of a unit-like enum value; for a value with data, the variant
/// name (`{"other": "x"}` gives `other`).
pub fn enum_name<T: Serialize>(v: &T) -> J {
    match serde_json::to_value(v) {
        Ok(s @ J::String(_)) => s,
        Ok(J::Object(m)) => m.keys().next().map_or(J::Null, |k| json!(k)),
        _ => J::Null,
    }
}

/// System-wide facts.
pub fn global(s: &Snapshot) -> Facts {
    let mut f = Facts::default();
    let Some(env) = &s.environment else {
        return f;
    };
    f.set_value("g.os.family", &env.os, |o| enum_name(&o.family));
    f.set_value("g.sandbox.kind", &env.sandbox, |sb| {
        json!(match sb {
            Sandbox::None => "none",
            Sandbox::Flatpak { .. } => "flatpak",
            Sandbox::Snap { .. } => "snap",
        })
    });
    let gaps = |sb: &Sandbox| match sb {
        Sandbox::Flatpak { missing, .. } | Sandbox::Snap { missing, .. } => missing.clone(),
        Sandbox::None => vec![],
    };
    f.set_value("g.sandbox.missing", &env.sandbox, |sb| {
        json!(gaps(sb).iter().map(|g| &g.permission).collect::<Vec<_>>())
    });
    f.set_value("g.sandbox.fixes", &env.sandbox, |sb| {
        json!(gaps(sb).iter().map(|g| &g.fix).collect::<Vec<_>>())
    });
    f.set_value("g.sandbox.required_missing", &env.sandbox, |sb| {
        json!(
            gaps(sb)
                .iter()
                .filter(|g| g.required)
                .map(|g| &g.permission)
                .collect::<Vec<_>>()
        )
    });
    f.set_value("g.sandbox.required_fixes", &env.sandbox, |sb| {
        json!(
            gaps(sb)
                .iter()
                .filter(|g| g.required)
                .map(|g| &g.fix)
                .collect::<Vec<_>>()
        )
    });
    f.set_value("g.session.kind", &env.session, |x| enum_name(&x.kind));
    f.set_value("g.compositor.kind", &env.session, |x| {
        x.compositor
            .as_ref()
            .map_or(J::Null, |c| enum_name(&c.kind))
    });
    f
}

/// Facts about one endpoint.
pub fn endpoint(e: &Endpoint) -> Facts {
    let mut f = Facts::default();
    f.set("e.class", enum_name(&e.class));
    f.set_value("e.connector", &e.connector, enum_name);
    f.set_value("e.status", &e.status, enum_name);
    f
}

/// A provenance for facts derived by the engine.
pub fn derived(layer: LayerKind, at: u64, what: &str) -> Provenance {
    Provenance {
        layer,
        source: Source::derived(what.to_string()),
        read_at_ms: at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use displaycaps_model::PermissionGap;
    use serde_json::json;

    #[test]
    fn set_get_extend_and_prefix() {
        let mut a = Facts::from_json(&json!({"g.x": 1, "e.y": 2, "n": null}));
        assert!(a.get("n").is_none(), "null facts are absent");
        assert_eq!(a.get("g.x").unwrap().value, json!(1));
        let b = Facts::from_json(&json!({"e.y": 3, "e.z": 4}));
        a.extend(&b);
        assert_eq!(a.get("e.y").unwrap().value, json!(3));
        let g = a.without_prefix("e.");
        assert_eq!(g.names().collect::<Vec<_>>(), ["g.x"]);
    }

    #[test]
    fn unknown_values_leave_facts_absent() {
        let mut f = Facts::default();
        let v: Value<u32> = Value::unknown(displaycaps_model::UnknownReason::NotExposed, None);
        f.set_value("e.n", &v, |n| json!(n));
        assert!(f.get("e.n").is_none());
    }

    #[test]
    fn enum_names() {
        assert_eq!(
            enum_name(&displaycaps_model::ConnectorType::DisplayPort),
            json!("display_port")
        );
        assert_eq!(
            enum_name(&displaycaps_model::ConnectorType::Other("X".into())),
            json!("other")
        );
    }

    #[test]
    fn sandbox_facts_split_required_from_optional() {
        let gap = |p: &str, required| PermissionGap {
            permission: p.into(),
            layers: vec![LayerKind::Kms],
            fix: format!("fix {p}"),
            required,
        };
        let s = crate::tests::snapshot_with_sandbox(Sandbox::Flatpak {
            granted: vec![],
            missing: vec![gap("--device=dri", true), gap("--talk-name=x", false)],
        });
        let g = global(&s);
        assert_eq!(g.get("g.sandbox.kind").unwrap().value, json!("flatpak"));
        assert_eq!(
            g.get("g.sandbox.missing").unwrap().value,
            json!(["--device=dri", "--talk-name=x"])
        );
        assert_eq!(
            g.get("g.sandbox.required_missing").unwrap().value,
            json!(["--device=dri"])
        );
        assert_eq!(
            g.get("g.sandbox.required_fixes").unwrap().value,
            json!(["fix --device=dri"])
        );
        assert_eq!(g.get("g.os.family").unwrap().value, json!("debian"));
        assert_eq!(g.get("g.session.kind").unwrap().value, json!("wayland"));
        assert!(g.get("g.compositor.kind").is_none());
    }
}
