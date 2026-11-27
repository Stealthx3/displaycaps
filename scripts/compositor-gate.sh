#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# compositor-gate.sh — phase 5 gate: for the running desktop, compares the compositor layer with
# the desktop's own tool in the default state, after a scale change and after a mode change (each
# made with that desktop's own tool), then restores the defaults. Desktop guest only, inside the
# session (see desktop-session.sh):
#   scripts/compositor-gate.sh <kwin|sway|gnome|x11> [output]
set -euo pipefail
cd "$(dirname "$0")/.."
# swaymsg finds Sway through SWAYSOCK, which an SSH shell lacks.
if [ "${1:-}" = sway ] && [ -z "${SWAYSOCK:-}" ]; then
  SWAYSOCK="$(ls "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"/sway-ipc.*.sock 2>/dev/null | head -n 1)"; export SWAYSOCK
fi
desk="${1:?kwin|sway|gnome|x11}"; o="${2:-Virtual-1}"
settle() { sleep 3; }
set_state() { # $1: default | scale | mode
  case "$desk/$1" in
    kwin/default) kscreen-doctor "output.$o.mode.1280x800@75" "output.$o.scale.1" >/dev/null 2>&1 ;;
    kwin/scale) kscreen-doctor "output.$o.scale.1.5" >/dev/null 2>&1 ;;
    kwin/mode) kscreen-doctor "output.$o.mode.1920x1080@60" "output.$o.scale.1.25" >/dev/null 2>&1 ;;
    sway/default) swaymsg output "$o" mode 1280x800@74.994Hz scale 1 >/dev/null ;;
    sway/scale) swaymsg output "$o" scale 1.5 >/dev/null ;;
    sway/mode) swaymsg output "$o" mode 1920x1080@60Hz scale 1.25 >/dev/null ;;
    gnome/default) gdctl set --logical-monitor --primary --monitor "$o" --mode 1280x800@74.994 --scale 1 >/dev/null ;;
    gnome/scale) gdctl set --logical-monitor --primary --scale 1.25 --monitor "$o" --mode 1280x800@74.994 >/dev/null ;;
    gnome/mode) gdctl set --logical-monitor --primary --scale 1.5 --monitor "$o" --mode 1920x1080@60.000 >/dev/null ;;
    x11/default) xrandr --output "$o" --mode 1280x800 ;;
    x11/scale) return 1 ;;   # X11 has no per-output scale
    x11/mode) xrandr --output "$o" --mode 1920x1080 --rate 60 ;;
  esac
}
fail=0
for state in default scale mode; do
  set_state "$state" || continue
  settle
  bash scripts/compositor-diff.sh "$desk" "$state" || fail=1
done
set_state default || true
exit $fail
