#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# compositor-diff.sh — phase 5 gate: compares DisplayCaps' compositor layer with the desktop's own
# settings tool on the same session (kscreen-doctor for KWin, swaymsg for Sway, gdctl for GNOME,
# xrandr for X11), through scripts/compositor-oracle.py. Runs in the desktop guest only, inside
# a display session (the session's environment exported, see desktop-session.sh):
#   scripts/compositor-diff.sh <kwin|sway|gnome|x11> <label>
# Outputs in $HOME/compositor-diff/; exits 1 on any difference.
set -euo pipefail
cd "$(dirname "$0")/.."
# swaymsg finds Sway through SWAYSOCK, which an SSH shell lacks.
if [ "${1:-}" = sway ] && [ -z "${SWAYSOCK:-}" ]; then
  SWAYSOCK="$(ls "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"/sway-ipc.*.sock 2>/dev/null | head -n 1)"; export SWAYSOCK
fi
bin="${DISPLAYCAPS:-dist/displaycaps}"
desk="${1:?kwin|sway|gnome|x11}"; label="${2:-default}"
out="$HOME/compositor-diff/$desk-$label"; mkdir -p "$out"
case "$desk" in
  kwin) kscreen-doctor -j >"$out/native.json" 2>/dev/null
        kscreen-doctor -o >"$out/native.txt" 2>/dev/null
        native=("$out/native.json" "$out/native.txt") ;;
  sway) swaymsg -r -t get_outputs >"$out/native.json"; native=("$out/native.json") ;;
  gnome) gdctl show >"$out/native.txt"; gdctl show --verbose >"$out/native-verbose.txt"
         native=("$out/native.txt" "$out/native-verbose.txt") ;;
  x11) xrandr --current >"$out/native.txt"; native=("$out/native.txt") ;;
  *) exit 2 ;;
esac
"$bin" probe --layer compositor >"$out/snapshot.json"
python3 scripts/compositor-oracle.py native "$desk" "${native[@]}" >"$out/native.facts"
python3 scripts/compositor-oracle.py ours "$out/snapshot.json" "$desk" >"$out/ours.facts"
n=$(wc -l <"$out/ours.facts")
if diff -u "$out/native.facts" "$out/ours.facts" >"$out/diff"; then
  echo "compositor-diff $desk/$label: identical ($n facts)"
else
  echo "compositor-diff $desk/$label: DIFFERENT"; cat "$out/diff"; exit 1
fi
