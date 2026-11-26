// SPDX-License-Identifier: LGPL-2.1-or-later
//! The live Mutter reader: one `org.gnome.Mutter.DisplayConfig.GetCurrentState` call on the
//! session bus. It never calls `ApplyMonitorsConfig` or any other method, and never activates the
//! service (the call is made only when the name has an owner). The zbus pattern follows AudioCaps'
//! BlueZ reader (zbus is async; the call runs on zbus's own runtime helper).

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use displaycaps_model::UnknownReason;
use serde_json::{Map, Value};
use zbus::wire::Variant;

use super::facts::{MutterFacts, MutterLogical, MutterMode, MutterMonitor, Read};

pub(crate) const SERVICE: &str = "org.gnome.Mutter.DisplayConfig";
pub(crate) const PATH: &str = "/org/gnome/Mutter/DisplayConfig";

type Props<'a> = HashMap<String, Variant<'a>>;
type Spec = (String, String, String, String);
type ModeT<'a> = (String, i32, i32, f64, f64, Vec<f64>, Props<'a>);
type MonitorT<'a> = (Spec, Vec<ModeT<'a>>, Props<'a>);
type LogicalT<'a> = (i32, i32, f64, u32, bool, Vec<Spec>, Props<'a>);
type StateT<'a> = (u32, Vec<MonitorT<'a>>, Vec<LogicalT<'a>>, Props<'a>);

/// Reads Mutter's current state from the session bus at `address` (or the default one).
/// `sandboxed`: inside Flatpak, where the bus proxy hides names the app may not talk to.
pub(crate) fn read(address: Option<&str>, budget: Duration, sandboxed: bool) -> Read<MutterFacts> {
    match zruntime::block_on(read_async(address, budget, sandboxed, true)) {
        Ok(f) => Read::Ok(f),
        Err((r, d)) => Read::err(r, d),
    }
}

/// Whether Mutter owns its DisplayConfig name, without calling it (the compositor's identity
/// for the environment layer). An owned name gives empty facts.
pub(crate) fn present(
    address: Option<&str>,
    budget: Duration,
    sandboxed: bool,
) -> Read<MutterFacts> {
    match zruntime::block_on(read_async(address, budget, sandboxed, false)) {
        Ok(f) => Read::Ok(f),
        Err((r, d)) => Read::err(r, d),
    }
}

type Fail = (UnknownReason, String);

async fn read_async(
    address: Option<&str>,
    budget: Duration,
    sandboxed: bool,
    state: bool,
) -> Result<MutterFacts, Fail> {
    let builder = match address {
        Some(a) => zbus::connection::Builder::address(a),
        None => zbus::connection::Builder::session(),
    };
    let conn = builder.method_timeout(budget).build().await.map_err(|e| {
        (
            UnknownReason::NotGranted,
            format!("no access to the session bus: {e}"),
        )
    })?;
    let dbus = zbus::fdo::DBusProxy::new(&conn)
        .await
        .map_err(|e| (UnknownReason::ReadError, e.to_string()))?;
    let name = zbus::names::BusName::try_from(SERVICE)
        .map_err(|e| (UnknownReason::ReadError, e.to_string()))?;
    match dbus.name_has_owner(name).await {
        Ok(true) => {}
        // Flatpak's bus proxy answers "no owner" for a name the app may not talk to.
        Ok(false) if sandboxed => {
            return Err((
                UnknownReason::NotGranted,
                format!(
                    "{SERVICE} is not visible from the sandbox (needs --talk-name={SERVICE}, or \
                     Mutter is not running)"
                ),
            ));
        }
        Ok(false) => {
            return Err((
                UnknownReason::ServiceNotRunning,
                format!("{SERVICE} has no owner (not GNOME, or Mutter is not running)"),
            ));
        }
        Err(e) => return Err((UnknownReason::NotGranted, e.to_string())),
    }
    if !state {
        return Ok(MutterFacts::default());
    }
    let msg = conn
        .call_method(Some(SERVICE), PATH, Some(SERVICE), "GetCurrentState", &())
        .await
        .map_err(|e| {
            let s = e.to_string();
            let reason = if s.contains("AccessDenied") || s.contains("ServiceUnknown") {
                UnknownReason::NotGranted
            } else if s.contains("imeout") {
                UnknownReason::Timeout
            } else {
                UnknownReason::ReadError
            };
            (reason, s)
        })?;
    let body = msg.body();
    let state: StateT<'_> = body
        .deserialize()
        .map_err(|e| (UnknownReason::ParseError, e.to_string()))?;
    Ok(from_state(state))
}

fn props(p: &Props<'_>) -> BTreeMap<String, Value> {
    p.iter().map(|(k, v)| (k.clone(), json(v))).collect()
}

fn from_state((serial, monitors, logical, global): StateT<'_>) -> MutterFacts {
    MutterFacts {
        serial,
        monitors: monitors
            .into_iter()
            .map(
                |((connector, vendor, product, serial), modes, p)| MutterMonitor {
                    connector,
                    vendor,
                    product,
                    serial,
                    modes: modes
                        .into_iter()
                        .map(
                            |(id, width, height, refresh, preferred_scale, _, p)| MutterMode {
                                id,
                                width,
                                height,
                                refresh,
                                preferred_scale,
                                properties: props(&p),
                            },
                        )
                        .collect(),
                    properties: props(&p),
                },
            )
            .collect(),
        logical_monitors: logical
            .into_iter()
            .map(
                |(_, _, scale, transform, primary, specs, _)| MutterLogical {
                    scale,
                    transform,
                    primary,
                    monitors: specs.into_iter().map(|s| s.0).collect(),
                },
            )
            .collect(),
        properties: props(&global),
    }
}

/// A D-Bus value as plain JSON.
fn json(v: &Variant<'_>) -> Value {
    use Variant as Z;
    match v {
        Z::U8(x) => Value::from(*x),
        Z::Bool(x) => Value::from(*x),
        Z::I16(x) => Value::from(*x),
        Z::U16(x) => Value::from(*x),
        Z::I32(x) => Value::from(*x),
        Z::U32(x) => Value::from(*x),
        Z::I64(x) => Value::from(*x),
        Z::U64(x) => Value::from(*x),
        Z::F64(x) => Value::from(*x),
        Z::Str(s) => Value::from(s.as_str()),
        Z::ObjectPath(p) => Value::from(p.as_str()),
        Z::Signature(s) => Value::from(s.to_string()),
        Z::Variant(inner) => json(inner),
        Z::Array(a) => Value::Array(a.iter().map(json).collect()),
        Z::Dict(d) => {
            let mut m = Map::new();
            for (k, v) in d.iter() {
                let key = match json(k) {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                m.insert(key, json(v));
            }
            Value::Object(m)
        }
        Z::Structure(s) => Value::Array(s.fields().iter().map(json).collect()),
        _ => Value::Null,
    }
}
