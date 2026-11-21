#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# oracles.sh — the lab-only EDID test oracles: edid-decode and libdisplay-info's di-edid-decode,
# built from pinned upstream sources into ~/oracles (never linked, never shipped, never on the host).
# Runs in the build guest only:
#   scripts/lab.sh run 'bash scripts/oracles.sh build'
#   scripts/lab.sh run 'bash scripts/oracles.sh versions'
#   scripts/lab.sh run 'bash scripts/oracles.sh run tests/fixtures/edid-corpus/<file>'   # both outputs
#   scripts/lab.sh run 'bash scripts/oracles.sh corpus'        # whole corpus, summary only
# The corpus is every *.bin under tests/fixtures/edid-corpus/ and tests/fixtures/parity/edid/, plus
# libdisplay-info's own test EDIDs from the pinned checkout (used in the lab only, not committed).
# Outputs go to target/oracles/ (git-ignored). Phase 2 diffs displaycaps-parse against them.
set -uo pipefail
cd "$(dirname "$0")/.."
O="$HOME/oracles"
# edid-decode moved into v4l-utils; pinned to the v4l-utils-1.32.0 release.
EDID_DECODE_REPO=https://git.linuxtv.org/v4l-utils.git
EDID_DECODE_TAG=v4l-utils-1.32.0
EDID_DECODE_COMMIT=5a666c7ce89c00d66aa8e53c8f098a0c6c401f91
LDI_REPO=https://gitlab.freedesktop.org/emersion/libdisplay-info.git
LDI_TAG=0.4.0
LDI_COMMIT=c67a3e9bedb05ab61c7443704a1a107e76254595
ED="$O/bin/edid-decode"
DI="$O/bin/di-edid-decode"

fetch() { # dir repo commit [tag]
  local dir="$1" repo="$2" commit="$3"
  [ -d "$dir/.git" ] || git clone -q "$repo" "$dir"
  git -C "$dir" fetch -q origin "$commit" 2>/dev/null || git -C "$dir" fetch -q origin
  git -C "$dir" -c advice.detachedHead=false checkout -q "$commit"
  [ "$(git -C "$dir" rev-parse HEAD)" = "$commit" ] || { echo "oracles: $dir is not at $commit" >&2; exit 1; }
}

case "${1:-}" in
  build)
    if ! dpkg -s meson ninja-build hwdata g++ >/dev/null 2>&1; then
      sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq meson ninja-build hwdata g++ python3 >/dev/null
    fi
    mkdir -p "$O/bin"
    rm -rf "$O/src/edid-decode"
    fetch "$O/src/v4l-utils" "$EDID_DECODE_REPO" "$EDID_DECODE_COMMIT"
    (cd "$O/src/v4l-utils" && rm -rf build && meson setup build -Dv4l-plugins=false -Dv4l-wrappers=false \
      -Dlibdvbv5=disabled -Dqvidcap=disabled -Dqv4l2=disabled -Dbpf=disabled -Djpeg=disabled \
      -Dgconv=disabled -Ddoxygen-doc=disabled >/dev/null && ninja -C build utils/edid-decode/edid-decode >/dev/null) || exit 1
    cp "$O/src/v4l-utils/build/utils/edid-decode/edid-decode" "$ED"
    fetch "$O/src/libdisplay-info" "$LDI_REPO" "$LDI_COMMIT"
    (cd "$O/src/libdisplay-info" && rm -rf build && meson setup build -Ddefault_library=static >/dev/null && ninja -C build >/dev/null) || exit 1
    cp "$O/src/libdisplay-info/build/di-edid-decode/di-edid-decode" "$DI"
    bash "$0" versions
    ;;
  versions)
    echo "edid-decode ($EDID_DECODE_TAG) $EDID_DECODE_COMMIT ($( [ -x "$ED" ] && echo built || echo missing))"
    echo "libdisplay-info $LDI_TAG $LDI_COMMIT ($( [ -x "$DI" ] && echo built || echo missing))"
    ;;
  run)
    f="${2:?edid file}"
    echo "== edid-decode"; "$ED" --check "$f"; echo "[exit $?]"
    echo "== di-edid-decode"; "$DI" < "$f"; echo "[exit $?]"
    ;;
  corpus)
    [ -x "$ED" ] && [ -x "$DI" ] || { echo "oracles: run 'oracles.sh build' first" >&2; exit 1; }
    out=target/oracles; rm -rf "$out"; mkdir -p "$out"
    mapfile -t files < <( { find tests/fixtures/edid-corpus tests/fixtures/parity/edid -name '*.bin' 2>/dev/null;
      find "$O/src/libdisplay-info/test/data" -name '*.edid' 2>/dev/null; } | sort)
    n=0; ed_ok=0; di_ok=0; conf=0; crashed=0
    for f in "${files[@]}"; do
      n=$((n + 1)); b="$(basename "$f")"; b="${b%.*}"
      case "$f" in "$O"/*) b="ldi-$b" ;; esac
      "$ED" --check "$f" > "$out/$b.edid-decode.txt" 2>&1; rc=$?
      [ $rc -eq 0 ] && ed_ok=$((ed_ok + 1)); [ $rc -ge 129 ] && [ $rc -le 159 ] && { crashed=$((crashed + 1)); echo "  edid-decode crashed ($rc) on $f"; }
      grep -qE '^(Failures|Warnings):' "$out/$b.edid-decode.txt" && conf=$((conf + 1))
      "$DI" < "$f" > "$out/$b.di-edid-decode.txt" 2>&1; rc=$?
      [ $rc -eq 0 ] && di_ok=$((di_ok + 1)); [ $rc -ge 129 ] && [ $rc -le 159 ] && { crashed=$((crashed + 1)); echo "  di-edid-decode crashed ($rc) on $f"; }
    done
    echo "oracles: $n EDIDs · edid-decode exit 0: $ed_ok · di-edid-decode exit 0: $di_ok · with conformity notes: $conf · crashes: $crashed · outputs in $out/"
    echo "oracles: non-zero exits without a crash mean the tool found conformity problems (expected for many test EDIDs)"
    [ $crashed -eq 0 ]
    ;;
  *) sed -n '3,12p' "$0"; exit 2 ;;
esac
