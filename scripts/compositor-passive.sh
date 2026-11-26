#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# compositor-passive.sh — phase 5 gate: checks that DisplayCaps' compositor reads send only
# read-only requests. Desktop guest only, inside a session (see desktop-session.sh):
#   scripts/compositor-passive.sh <wayland|x11>
# - Wayland: the client's own protocol log (WAYLAND_DEBUG=1) may hold only these requests:
#   wl_display.get_registry/sync, wl_registry.bind, wp_color_manager_v1.get_output/destroy,
#   wp_color_management_output_v1.get_image_description/destroy,
#   wp_image_description_v1.get_information/destroy.
# - D-Bus (on GNOME): the only method called on org.gnome.Mutter.DisplayConfig is
#   GetCurrentState (dbus-monitor on the session bus).
# - X11: an xtrace proxy logs every request; allowed: QueryExtension, RandR QueryVersion,
#   GetScreenResourcesCurrent, GetOutputInfo, GetCrtcInfo, ListOutputProperties,
#   GetOutputProperty (delete and pending off), RandR SelectInput (the watch: this connection's
#   own event mask), GetAtomName, and the connection's own setup. GetScreenResources (a forced
#   re-probe) fails.
# Runs a probe of the compositor and environment layers, and a short watch (both sessions).
set -euo pipefail
cd "$(dirname "$0")/.."
bin="${DISPLAYCAPS:-dist/displaycaps}"
kind="${1:?wayland|x11}"
out="$HOME/compositor-passive/$kind"; mkdir -p "$out"
fail=0
if [ "$kind" = wayland ]; then
  mon=""
  if [ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] && command -v dbus-monitor >/dev/null; then
    dbus-monitor --session "type='method_call'" >"$out/dbus.log" 2>&1 &
    mon=$!; sleep 1
  fi
  WAYLAND_DEBUG=1 "$bin" probe --layer compositor --layer environment >"$out/probe.json" 2>"$out/wayland.log"
  WAYLAND_DEBUG=1 timeout 3 "$bin" watch --layer compositor --events 1 --seconds 2 >/dev/null 2>>"$out/wayland.log" || true
  [ -n "$mon" ] && { sleep 1; kill "$mon" 2>/dev/null || true; wait "$mon" 2>/dev/null || true; }
  python3 - "$out/wayland.log" <<'PY' || fail=1
import re, sys
allowed = {
    ("wl_display", "get_registry"), ("wl_display", "sync"), ("wl_registry", "bind"),
    ("wp_color_manager_v1", "get_output"), ("wp_color_manager_v1", "destroy"),
    ("wp_color_management_output_v1", "get_image_description"),
    ("wp_color_management_output_v1", "destroy"),
    ("wp_image_description_v1", "get_information"), ("wp_image_description_v1", "destroy"),
}
n, bad = 0, []
for line in open(sys.argv[1], errors="replace"):
    m = re.search(r"-> (\w+)@\d+\.(\w+)\(", line)
    if not m:
        continue
    n += 1
    if (m.group(1), m.group(2)) not in allowed:
        bad.append(line.strip())
if n == 0:
    print("compositor-passive: no Wayland requests logged (WAYLAND_DEBUG not honoured?)"); sys.exit(1)
if bad:
    print("compositor-passive: disallowed Wayland requests:"); print("\n".join(bad)); sys.exit(1)
print(f"compositor-passive wayland: {n} requests, all read-only")
PY
  if [ -s "$out/dbus.log" ]; then
    mutter=$(grep "^method call" "$out/dbus.log" | grep "org.gnome.Mutter.DisplayConfig" || true)
    calls=$(printf '%s' "$mutter" | grep -c . || true)
    others=$(printf '%s' "$mutter" | grep -v "member=GetCurrentState" || true)
    if [ -n "$others" ]; then echo "compositor-passive: disallowed D-Bus calls:"; echo "$others"; fail=1
    else echo "compositor-passive dbus: $calls call(s) to Mutter, all GetCurrentState"; fi
  fi
else
  command -v xtrace >/dev/null || { echo "compositor-passive: xtrace not installed" >&2; exit 2; }
  # The proxied client can't present the session's cookie for the fake display, so the lab
  # user is let in by name for the run (guest only) and removed again.
  xhost +si:localuser:"$(id -un)" >/dev/null
  xtrace -n -s -D :9 -d "$DISPLAY" -o "$out/x11.log" -- env DISPLAY=:9 "$bin" probe --layer compositor --layer environment >"$out/probe.json" 2>/dev/null || true
  # The watch: SelectInput sets only this connection's event mask.
  xtrace -n -s -D :9 -d "$DISPLAY" -o "$out/x11-watch.log" -- env DISPLAY=:9 timeout 3 "$bin" watch --layer compositor --events 1 --seconds 2 >/dev/null 2>&1 || true
  cat "$out/x11-watch.log" >>"$out/x11.log"
  xhost -si:localuser:"$(id -un)" >/dev/null
  python3 - "$out/x11.log" <<'PY' || fail=1
import re, sys
allowed = {"QueryExtension", "RANDR-QueryVersion", "RANDR-GetScreenResourcesCurrent",
           "RANDR-GetOutputInfo", "RANDR-GetCrtcInfo", "RANDR-ListOutputProperties",
           "RANDR-GetOutputProperty", "RANDR-SelectInput", "GetAtomName", "BIG-REQUESTS-Enable",
           "GetInputFocus"}
n, bad = 0, []
for line in open(sys.argv[1], errors="replace"):
    m = re.search(r"(?:([\w-]+)-)?Request\([\d,]+\): (\w+)", line)
    if not m:
        continue
    n += 1
    req = f"{m.group(1)}-{m.group(2)}" if m.group(1) else m.group(2)
    if req not in allowed or (req == "RANDR-GetOutputProperty" and ("delete=true" in line or "pending=true" in line)):
        bad.append(line.strip())
if n == 0:
    print("compositor-passive: no X11 requests logged"); sys.exit(1)
if bad:
    print("compositor-passive: disallowed X11 requests:"); print("\n".join(bad[:20])); sys.exit(1)
print(f"compositor-passive x11: {n} requests, all read-only")
PY
fi
exit $fail
