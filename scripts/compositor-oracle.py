#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-2.1-or-later
# compositor-oracle.py — phase 5 gate helper: turns a desktop's own settings tool output and a
# DisplayCaps snapshot into the same normalized fact lines, one per output and setting:
#   compositor-oracle.py native <kwin|sway|gnome|x11> <tool-output>... > native.facts
#   compositor-oracle.py ours <snapshot.json> <kwin|sway|gnome|x11> > ours.facts
# Only the settings the desktop's tool states are compared, so each side prints that subset.
# Refresh rates are compared at 0.01 Hz, scales at 0.01.
import json
import re
import sys

KEYS = {
    "kwin": ["enabled", "mode", "scale", "modes", "hdr", "hdr_capable", "wcg", "vrr", "rgb"],
    "sway": ["enabled", "mode", "scale", "modes", "vrr"],
    "gnome": ["enabled", "mode", "scale", "modes", "hdr", "hdr_capable"],
    "x11": ["enabled", "mode", "modes"],
}


def hz(mhz):
    return f"{mhz / 1000:.2f}"


def emit(facts, desktop):
    for name in sorted(facts):
        for k in KEYS[desktop]:
            if k in facts[name]:
                print(f"{name} {k} {facts[name][k]}")


def ansi(s):
    return re.sub(r"\x1b\[[0-9;]*m", "", s)


def native_kwin(json_file, text_file):
    d = json.load(open(json_file))
    out = {}
    for o in d["outputs"]:
        if not o.get("connected"):
            continue
        f = out.setdefault(o["name"], {})
        f["enabled"] = str(o["enabled"]).lower()
        modes = {m["id"]: m for m in o["modes"]}
        m = modes.get(o.get("currentModeId"))
        if o["enabled"] and m:
            f["mode"] = f"{m['size']['width']}x{m['size']['height']}@{m['refreshRate']:.2f}"
            f["scale"] = f"{float(o['scale']):.2f}"
        f["modes"] = str(len(o["modes"]))
    name = None
    for line in ansi(open(text_file).read()).splitlines():
        mo = re.match(r"Output: \d+ (\S+)", line)
        if mo:
            name = mo.group(1)
            continue
        if name not in out:
            continue
        mo = re.match(r"\s+(Vrr|RgbRange|HDR|Wide Color Gamut): (.*)", line)
        if not mo:
            continue
        key, val = mo.group(1), mo.group(2).strip().lower()
        f = out[name]
        if key == "Vrr":
            f["vrr"] = val
        elif key == "RgbRange":
            f["rgb"] = val
        elif key == "HDR":
            f["hdr"] = "true" if val == "enabled" else "false"
            f["hdr_capable"] = "false" if val == "incapable" else "true"
        else:
            f["wcg"] = "true" if val == "enabled" else "false"
    return out


def native_sway(json_file):
    out = {}
    for o in json.load(open(json_file)):
        f = out.setdefault(o["name"], {})
        f["enabled"] = str(o.get("active", False)).lower()
        if o.get("active"):
            c = o["current_mode"]
            f["mode"] = f"{c['width']}x{c['height']}@{hz(c['refresh'])}"
            f["scale"] = f"{float(o['scale']):.2f}"
        f["modes"] = str(len(o.get("modes", [])))
        a = o.get("adaptive_sync_status")
        if a:
            f["vrr"] = "always" if a == "enabled" else "never"
    return out


def native_gnome(brief_file, verbose_file):
    out = {}
    lines = open(brief_file).read().splitlines()
    mon = None
    section = None
    logical_scale = None
    for line in lines:
        mo = re.search(r"Monitor (\S+) \(", line)
        if mo and "Logical" not in line:
            mon = mo.group(1)
            out.setdefault(mon, {"enabled": "false"})
            section = "monitor"
            continue
        if "Logical monitor #" in line:
            section = "logical"
            logical_scale = None
            continue
        if section == "monitor" and mon:
            mo = re.search(r"(\d+)x(\d+)@([\d.]+)", line)
            if mo:
                out[mon]["mode"] = f"{mo.group(1)}x{mo.group(2)}@{float(mo.group(3)):.2f}"
        if section == "logical":
            mo = re.search(r"Scale: ([\d.]+)", line)
            if mo:
                logical_scale = float(mo.group(1))
            mo = re.search(r"└──(\S+) \(", line)
            if mo and mo.group(1) in out:
                out[mo.group(1)]["enabled"] = "true"
                if logical_scale is not None:
                    out[mo.group(1)]["scale"] = f"{logical_scale:.2f}"
    mon = None
    for line in open(verbose_file).read().splitlines():
        mo = re.search(r"Monitor (\S+) \(", line)
        if mo and "Logical" not in line:
            mon = mo.group(1)
            continue
        if mon not in out:
            continue
        mo = re.search(r"Modes \((\d+)\)", line)
        if mo:
            out[mon]["modes"] = mo.group(1)
        mo = re.search(r"└──color-mode ⇒\s+(\S+)|├──color-mode ⇒\s+(\S+)", line)
        if mo:
            out[mon]["hdr"] = "true" if (mo.group(1) or mo.group(2)) == "bt2100" else "false"
        mo = re.search(r"supported-color-modes ⇒\s+\[(.*)\]", line)
        if mo:
            out[mon]["hdr_capable"] = "true" if "bt2100" in mo.group(1) else "false"
    for f in out.values():
        if f.get("enabled") != "true":
            f.pop("mode", None)
    return out


def native_x11(text_file):
    out = {}
    name = None
    for line in open(text_file).read().splitlines():
        mo = re.match(r"(\S+) (connected|disconnected)", line)
        if mo:
            name = mo.group(1) if mo.group(2) == "connected" else None
            if name:
                out[name] = {
                    "enabled": "true" if re.search(r" \d+x\d+\+\d+\+\d+", line) else "false",
                    "modes": "0",
                }
            continue
        if not name or not line.startswith("   "):
            continue
        parts = line.split()
        res = parts[0]
        rates = re.findall(r"([\d.]+)([ *+]*)", " ".join(parts[1:]))
        out[name]["modes"] = str(int(out[name]["modes"]) + len(rates))
        for r, flags in rates:
            if "*" in flags:
                out[name]["mode"] = f"{res}@{float(r):.2f}"
    return out


def ours(snapshot_file, desktop):
    snap = json.load(open(snapshot_file))
    out = {}
    v = lambda x: x.get("value") if isinstance(x, dict) and x.get("state") != "unknown" else None
    for e in snap["endpoints"]:
        for l in e["layers"]:
            if l["layer"] != "compositor" or not l.get("detail"):
                continue
            d = l["detail"]
            name = v(d["output_name"])
            f = out.setdefault(name, {})
            f["enabled"] = str(v(d["enabled"])).lower()
            m = v(d.get("mode") or {})
            if m and v(d["enabled"]):
                f["mode"] = f"{m['width']}x{m['height']}@{hz(m['refresh_millihz'])}"
            s = v(d.get("scale") or {})
            if s is not None and v(d["enabled"]):
                f["scale"] = f"{s:.2f}"
            modes = v(l.get("caps", {}).get("modes") or {})
            if modes is not None:
                f["modes"] = str(len(modes))
            h = v(d.get("hdr_enabled") or {})
            if h is not None:
                f["hdr"] = str(h).lower()
            caps = v(d.get("hdr") or {})
            if caps is not None:
                f["hdr_capable"] = str("pq" in caps.get("eotfs", [])).lower()
            w = v(d.get("wide_color_enabled") or {})
            if w is not None:
                f["wcg"] = str(w).lower()
            vrr = v(d.get("vrr_policy") or {})
            f["vrr"] = vrr if vrr else "incapable"
            rgb = v(d.get("rgb_range") or {})
            f["rgb"] = rgb if rgb else "unknown"
    if desktop == "x11":
        # xrandr --current lists connected outputs only; so does the compositor layer.
        pass
    return out


def main():
    if sys.argv[1] == "native":
        desktop, files = sys.argv[2], sys.argv[3:]
        facts = {
            "kwin": native_kwin,
            "sway": native_sway,
            "gnome": native_gnome,
            "x11": native_x11,
        }[desktop](*files)
    else:
        desktop = sys.argv[3]
        facts = ours(sys.argv[2], desktop)
    emit(facts, desktop)


if __name__ == "__main__":
    main()
