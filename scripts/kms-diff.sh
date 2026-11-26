#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# kms-diff.sh — phase 3 gate: compares DisplayCaps' live KMS read (its own read-only ioctls) with
# `drm_info -j` on the same system. Both go through the same normalization
# (`displaycaps kms-facts`, with and without `--drm-info`), so any difference is in the reading.
# Runs in the desktop guest only, inside a display session (the session's environment exported, see
# desktop-session.sh):
#   scripts/kms-diff.sh <label>
# Outputs in $HOME/kms-diff/; exits 1 on any difference. Blob properties (EDID, MODE_ID,
# HDR_OUTPUT_METADATA) are compared as set/unset: their ids change whenever a new blob is made.
set -euo pipefail
cd "$(dirname "$0")/.."
bin="${DISPLAYCAPS:-dist/displaycaps}"
label="${1:-session}"
out="$HOME/kms-diff"; mkdir -p "$out"
drm_info -j >"$out/$label.drm_info.json"
"$bin" kms-facts >"$out/$label.ours"
"$bin" kms-facts --drm-info "$out/$label.drm_info.json" >"$out/$label.drm_info"
n=$(wc -l <"$out/$label.ours")
if diff -u "$out/$label.drm_info" "$out/$label.ours" >"$out/$label.diff"; then
  echo "kms-diff $label: identical ($n facts)"
else
  echo "kms-diff $label: DIFFERENT (see $out/$label.diff)"; cat "$out/$label.diff"; exit 1
fi
