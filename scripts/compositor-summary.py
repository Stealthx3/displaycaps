#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-2.1-or-later
# compositor-summary.py — lab helper: condenses `displaycaps compositor-facts` (argv[1]) and a
# `displaycaps probe --layer compositor --layer environment` snapshot (argv[2]) for reading.
import json, sys

facts = json.load(open(sys.argv[1]))
w = facts.get("wayland", {}).get("ok")
if w:
    w["globals"] = len(w.get("globals", []))
    for k in ("wlr_heads", "kde_devices"):
        for d in w.get(k) or []:
            d["modes"] = len(d.get("modes", []))
            if d.get("edid_hex"):
                d["edid_hex"] = d["edid_hex"][:16] + "..."
m = facts.get("mutter", {}).get("ok")
if m:
    for mon in m["monitors"]:
        cur = [x for x in mon["modes"] if x.get("properties", {}).get("is-current")]
        mon["modes"] = f"{len(mon['modes'])} modes, current {cur[0]['id'] if cur else None}"
r = facts.get("randr", {}).get("ok")
if r:
    for o in r.get("outputs", []):
        o["modes"] = len(o.get("modes", []))
        if "EDID" in o.get("properties", {}):
            o["properties"]["EDID"] = o["properties"]["EDID"][:16] + "..."
print(json.dumps(facts, indent=1))
snap = json.load(open(sys.argv[2]))
print("session:", json.dumps(snap["environment"]["session"].get("value"), indent=1))
print("layers:", json.dumps(snap["layers"]))
for e in snap["endpoints"]:
    for l in e["layers"]:
        if l["layer"] != "compositor":
            continue
        d = (l.get("detail") or {}).copy()
        brief = {k: (v.get("value") if isinstance(v, dict) and "value" in v else v) for k, v in d.items()}
        modes = l.get("caps", {}).get("modes", {})
        print(e["id"], l["availability"], json.dumps(brief), "modes:", len(modes.get("value", [])) if isinstance(modes.get("value"), list) else modes)
    print("  correlation:", [c["id"] for c in e.get("correlation", []) if c["id"]["kind"] in ("wl_output", "mutter_monitor", "randr_output")])
