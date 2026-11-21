#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# Adapted from AudioCaps v1.0.0 (commit 049d053) scripts/fuzz.sh: DisplayCaps targets and seeds.
# fuzz.sh — seed, run and summarise the cargo-fuzz targets. Runs inside the build guest:
#   scripts/lab.sh run 'bash scripts/fuzz.sh build'
#   scripts/lab.sh run 'bash scripts/fuzz.sh seed'
#   scripts/lab.sh run 'bash scripts/fuzz.sh start 86400'   # detached; all targets in parallel
#   scripts/lab.sh run 'bash scripts/fuzz.sh summary'
# Corpora, logs and crash artifacts stay under fuzz/ (git-ignored).
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cd "$(dirname "$0")/.."
TARGETS=(edid cta_video displayid timing os_release rule_pack)
F=fuzz

seed_files() { # target dir pattern...
  local t="$1"; shift
  mkdir -p "$F/corpus/$t"
  find tests/fixtures -type f \( "$@" \) -size +0 -print0 |
    while IFS= read -r -d '' p; do cp "$p" "$F/corpus/$t/seed-$(sha256sum "$p" | cut -c1-16)"; done
}

case "${1:-}" in
  build)
    (cd "$F" && cargo fuzz build -O 2>&1 | tail -n 20) && echo "fuzz build: ok"
    ;;
  seed)
    # EDIDs: the committed corpus and parity vectors, plus the oracles' test data when present (lab).
    edids=$(ls tests/fixtures/edid-corpus/*.bin tests/fixtures/parity/edid/*.bin target/oracles/raw/*.bin 2>/dev/null)
    for t in edid timing; do
      mkdir -p "$F/corpus/$t"
      for p in $edids; do
        if [ "$t" = timing ]; then { printf '\x00\x0f\x70\x08\x00\x0f\x00\x00\x00'; head -c 128 "$p"; } > "$F/corpus/$t/seed-$(sha256sum "$p" | cut -c1-16)"
        else cp "$p" "$F/corpus/$t/seed-$(sha256sum "$p" | cut -c1-16)"; fi
      done
    done
    tmp="$(mktemp)"
    for t in cta_video displayid; do
      mkdir -p "$F/corpus/$t"
      tag=$([ "$t" = cta_video ] && echo 02 || echo 70)
      for p in $edids; do
        n=$(( $(stat -c %s "$p") / 128 ))
        for i in $(seq 1 $((n - 1))); do
          dd if="$p" of="$tmp" bs=128 skip="$i" count=1 status=none
          [ "$(head -c1 "$tmp" | od -An -tx1 | tr -d ' ')" = "$tag" ] && cp "$tmp" "$F/corpus/$t/seed-$(sha256sum "$tmp" | cut -c1-16)"
        done
      done
    done
    rm -f "$tmp"
    mkdir -p "$F/corpus/os_release"
    find tests/fixtures -name os-release -size +0 | while read -r p; do cp "$p" "$F/corpus/os_release/seed-$(sha256sum "$p" | cut -c1-16)"; done
    mkdir -p "$F/corpus/rule_pack"; for p in crates/displaycaps-rules/packs/*.json; do cp "$p" "$F/corpus/rule_pack/seed-$(basename "$p")"; done
    for t in "${TARGETS[@]}"; do echo "$t: $(ls "$F/corpus/$t" 2>/dev/null | wc -l) seeds"; done
    ;;
  start)
    secs="${2:?seconds}"
    mkdir -p "$F/logs"
    date -u +%FT%TZ > "$F/logs/started"
    echo "$secs" > "$F/logs/duration"
    for t in "${TARGETS[@]}"; do
      mkdir -p "$F/corpus/$t" "$F/artifacts/$t"
      # Redirect the whole subshell (not just cargo) so nothing holds the caller's stdout open,
      # and setsid so the run survives the SSH session that started it.
      (cd "$F" && exec setsid nohup cargo fuzz run -O "$t" "corpus/$t" -- \
        -max_total_time="$secs" -timeout=10 -rss_limit_mb=2048 -print_final_stats=1 \
        -artifact_prefix="artifacts/$t/") < /dev/null > "$F/logs/$t.log" 2>&1 &
    done
    echo "started ${#TARGETS[@]} targets for ${secs}s"
    ;;
  summary)
    [ -f "$F/logs/started" ] && echo "started $(cat "$F/logs/started"), duration $(cat "$F/logs/duration")s"
    fail=0
    for t in "${TARGETS[@]}"; do
      log="$F/logs/$t.log"
      running=$(pgrep -f "release/$t " >/dev/null && echo running || echo stopped)
      last=$(grep -E '^#[0-9]+' "$log" 2>/dev/null | tail -n 1 | awk '{print $1, $3, $4, $(NF-3), $(NF-2), $(NF-1), $NF}')
      execs=$(grep -E 'stat::number_of_executed_units' "$log" 2>/dev/null | awk '{print $2}')
      crashes=$(find "$F/artifacts/$t" -type f 2>/dev/null | wc -l)
      [ "$crashes" -gt 0 ] && fail=1
      grep -qE 'ERROR: libFuzzer|panicked at' "$log" 2>/dev/null && fail=1
      echo "$t: $running, crashes=$crashes, ${execs:+execs=$execs, }last: ${last:-none}"
    done
    [ $fail -eq 0 ] && echo "fuzz: CLEAN" || echo "fuzz: FAILURES (see fuzz/artifacts, fuzz/logs)"
    exit $fail
    ;;
  stop)
    pkill -f 'cargo-fuzz fuzz run'; pkill -f "release/($(IFS='|'; echo "${TARGETS[*]}")) " && echo stopped || echo "nothing running"
    ;;
  *)
    sed -n '3,7p' "$0"; exit 2 ;;
esac
