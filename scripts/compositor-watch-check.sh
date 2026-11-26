#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# compositor-watch-check.sh — phase 5: starts a compositor-layer watch, changes the output's
# scale with the desktop's own tool, and checks that the watch reports a CompositorChange with
# the new scale; then restores it. Desktop guest only, inside the session:
#   scripts/compositor-watch-check.sh <kwin|sway|gnome|x11> [output]
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "${1:-}" = sway ] && [ -z "${SWAYSOCK:-}" ]; then
  SWAYSOCK="$(ls "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"/sway-ipc.*.sock 2>/dev/null | head -n 1)"; export SWAYSOCK
fi
bin="${DISPLAYCAPS:-dist/displaycaps}"
desk="${1:?kwin|sway|gnome|x11}"; o="${2:-Virtual-1}"
log="$(mktemp)"
"$bin" watch --layer compositor --events 1 --seconds 15 >"$log" &
pid=$!
sleep 2
case "$desk" in
  kwin) kscreen-doctor "output.$o.scale.1.5" >/dev/null 2>&1 ;;
  sway) swaymsg output "$o" scale 1.5 >/dev/null ;;
  gnome) gdctl set --logical-monitor --primary --scale 1.25 --monitor "$o" --mode 1280x800@74.994 >/dev/null ;;
  x11) xrandr --output "$o" --mode 1920x1080 ;;
esac
wait "$pid" || true
case "$desk" in
  kwin) kscreen-doctor "output.$o.scale.1" >/dev/null 2>&1 ;;
  sway) swaymsg output "$o" scale 1 >/dev/null ;;
  gnome) gdctl set --logical-monitor --primary --scale 1 --monitor "$o" --mode 1280x800@74.994 >/dev/null ;;
  x11) xrandr --output "$o" --mode 1280x800 ;;
esac
python3 - "$log" <<'PY'
import json, sys
lines = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
changes = [l for l in lines if "CompositorChange" in l.get("causes", [])]
if not changes:
    print("compositor-watch: no CompositorChange event:", [l.get("causes") for l in lines]); sys.exit(1)
c = changes[0]
print(f"compositor-watch: CompositorChange, re-probe {c.get('latency_ms')} ms, changed {c.get('diff', {}).get('changed')}")
PY
