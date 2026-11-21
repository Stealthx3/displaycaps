#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# Copied from AudioCaps v1.0.0 (commit 049d053) scripts/gen-header.sh; renamed for DisplayCaps.
# Regenerates include/displaycaps.h with cbindgen (a build tool, MPL-2.0; not linked).
# `--check` fails when the committed header is out of date.
set -euo pipefail
cd "$(dirname "$0")/.."
. ~/.cargo/env 2>/dev/null || true
command -v cbindgen >/dev/null || cargo install cbindgen --locked --version 0.29.2 >/dev/null
out=include/displaycaps.h
if [ "${1:-}" = --check ]; then
  tmp="$(mktemp)"; trap 'rm -f "$tmp"' EXIT
  cbindgen --quiet --config crates/displaycaps-ffi/cbindgen.toml --crate displaycaps-ffi --output "$tmp" crates/displaycaps-ffi
  diff -u "$out" "$tmp" && echo "header: current"
else
  cbindgen --quiet --config crates/displaycaps-ffi/cbindgen.toml --crate displaycaps-ffi --output "$out" crates/displaycaps-ffi
  echo "header: $out"
fi
