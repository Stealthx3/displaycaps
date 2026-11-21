#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# edid-diff.sh — differential test of displaycaps-parse against both lab oracles (edid-decode and
# di-edid-decode) over the whole corpus. Runs in the build guest only:
#   scripts/lab.sh run 'bash scripts/edid-diff.sh'           # summary; exit 1 on untriaged diffs
#   scripts/lab.sh run 'bash scripts/edid-diff.sh FILE'      # one EDID, full fact diff
# Facts are normalized lines (scripts/edid_facts_oracle.py for the oracles; the edid_facts example
# for displaycaps-parse). A difference is accepted only when tests/fixtures/edid-corpus/triage.tsv
# lists it: <corpus file name> TAB <fact line, prefixed + (only ours) or - (edid-decode has it, we lack it)> TAB <reason>.
# Where the two oracles disagree and we match one of them, the case is listed as an oracle split.
# Facts are a set per EDID (not per block): two blocks stating the same fact count once.
# Not compared yet: DisplayID tiles, adaptive-sync and parameter blocks, HDMI 3D, Dolby Vision
# primaries (unit tests cover them).
# Corpus: committed EDIDs (tests/fixtures/edid-corpus, tests/fixtures/parity/edid) plus the oracles'
# own test data from their pinned checkouts (lab only, not committed). Outputs in target/oracles/diff/.
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export LC_ALL=C.UTF-8
cd "$(dirname "$0")/.."
O="$HOME/oracles"; ED="$O/bin/edid-decode"; DI="$O/bin/di-edid-decode"
out=target/oracles/diff; raw=target/oracles/raw
mkdir -p "$out" "$raw"
cargo build -q --release -p displaycaps-parse --example edid_facts || exit 1
OURS=target/release/examples/edid_facts
# edid-decode's data files are hex dumps: convert once to binary.
for f in "$O"/src/v4l-utils/utils/edid-decode/data/*; do
  b="$raw/ed-$(basename "$f").bin"; [ -s "$b" ] || "$ED" -o raw "$f" "$b" >/dev/null 2>&1 || rm -f "$b"
done
for f in "$O"/src/libdisplay-info/test/data/*.edid; do cp -n "$f" "$raw/ldi-$(basename "${f%.edid}").bin"; done
if [ $# -gt 0 ]; then files=("$@"); else
  mapfile -t files < <(ls tests/fixtures/edid-corpus/*.bin tests/fixtures/parity/edid/*.bin "$raw"/*.bin 2>/dev/null)
fi
triage=tests/fixtures/edid-corpus/triage.tsv
n=0; clean=0; agree=0; untriaged=0; oracle_split=0
: > "$out/summary.tsv"
for f in "${files[@]}"; do
  n=$((n + 1)); name="$(basename "$f" .bin)"
  "$ED" --skip-hex-dump "$f" 2>/dev/null | python3 scripts/edid_facts_oracle.py > "$out/$name.ed"
  "$DI" < "$f" 2>/dev/null | python3 scripts/edid_facts_oracle.py > "$out/$name.di"
  "$OURS" "$f" > "$out/$name.ours" 2>"$out/$name.err" || echo "!parse-failed" >> "$out/$name.ours"
  # An empty primary-oracle result would turn every fact into a silent split: fail instead.
  [ -s "$out/$name.ed" ] || echo "!edid-decode-empty" >> "$out/$name.ours"
  if ! cmp -s "$out/$name.ed" "$out/$name.di"; then oracle_split=$((oracle_split + 1)); fi
  # edid-decode is the primary oracle: a fact it reports that we lack is a difference ("-"), and so
  # is a fact we report that neither oracle has ("+"). Facts only di-edid-decode reports, and facts
  # we share with one oracle only, are oracle splits: listed in $out/$name.split for review, not
  # failures (di-edid-decode decodes fewer blocks and computes some legacy timings differently).
  sort -u "$out/$name.ed" "$out/$name.di" > "$out/$name.any"
  { comm -23 "$out/$name.ed" "$out/$name.ours" | sed 's/^/- /'; comm -13 "$out/$name.any" "$out/$name.ours" | sed 's/^/+ /'; } > "$out/$name.diff2"
  { comm -12 "$out/$name.ours" "$out/$name.ed" | comm -23 - "$out/$name.di" | sed 's/^/edid-decode only: /'
    comm -12 "$out/$name.ours" "$out/$name.di" | comm -23 - "$out/$name.ed" | sed 's/^/di-edid-decode only: /'
    comm -23 "$out/$name.di" "$out/$name.ours" | comm -23 - "$out/$name.ed" | sed 's/^/di-edid-decode only, we lack: /'; } > "$out/$name.split"
  left=0
  while IFS= read -r d; do
    [ -z "$d" ] && continue
    if [ -f "$triage" ] && awk -F'\t' -v n="$name" -v d="$d" '$1==n && $2==d {f=1} END {exit !f}' "$triage"; then continue; fi
    left=$((left + 1)); echo "$name	$d" >> "$out/summary.tsv"
  done < "$out/$name.diff2"
  [ -s "$out/$name.diff2" ] || clean=$((clean + 1))
  [ $left -eq 0 ] && agree=$((agree + 1)) || untriaged=$((untriaged + left))
  if [ $# -gt 0 ]; then echo "== $name"; cat "$out/$name.diff2"; fi
done
splits="$(cat "$out"/*.split 2>/dev/null | wc -l)"
if [ $# -eq 0 ]; then
  echo "edid-diff: oracle-split facts to review: $splits (kinds: $(cat "$out"/*.split 2>/dev/null | sed -E 's/^([^:]+): ([a-z0-9]+ ?[a-z0-9]*).*/\1 \2/' | sort | uniq -c | sort -rn | head -n 8 | awk '{c=$1; $1=""; printf "%s%s ×%s", (n++?"; ":""), substr($0,2), c}'))"
  echo "edid-diff: $n EDIDs · identical facts: $clean · agree after triage: $agree · untriaged differences: $untriaged · oracles disagree with each other on $oracle_split (we side with one: see $out/*.split)"
  [ $untriaged -gt 0 ] && { echo "top untriaged fact kinds:"; cut -f2 "$out/summary.tsv" | awk '{print $1, $2, $3}' | sort | uniq -c | sort -rn | head -n 15; }
fi
[ $untriaged -eq 0 ]
