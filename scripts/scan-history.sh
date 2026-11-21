#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# Copied from AudioCaps v1.0.0 (commit 049d053) scripts/scan-history.sh; adds the unredacted-EDID check.
# scan-history.sh — checks every commit reachable from all refs for things that must never be
# committed: plan documents, raw captures, IPv4/MAC addresses, private keys, .env files and EDIDs
# whose serial fields are not redacted.
# Prints one line per problem and "history: clean" or "history: N problem(s)". Runs on the host
# (reads git only).
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
problems=0
report() { echo "  $1"; problems=$((problems + 1)); }
paths="$(git log --all --name-only --pretty=format: | sort -u)"
while read -r p; do
  [ -z "$p" ] && continue
  case "$p" in
    docs/plan/*|captures/*|*.capture.tar*|*Development-Plan*) report "forbidden path in history: $p" ;;
    .env|.env.*|*.pem|*.key|id_rsa*|id_ed25519*) report "secret-looking file in history: $p" ;;
  esac
done <<< "$paths"
revs="$(git rev-list --all)"
allow="$(grep -vE '^(#|$)' scripts/scan-history.allow 2>/dev/null || true)"
allowed() { [ -n "$allow" ] && grep -qE -f <(echo "$allow") <<< "$1"; }
# IPv4 outside loopback/0.0.0.0 and libvirt's documented default subnet inside the verbatim VM
# lab standard; MAC addresses not in the redacted 02: form.
ips="$(git grep -IhoE '\b(25[0-5]|2[0-4][0-9]|1?[0-9]?[0-9])(\.(25[0-5]|2[0-4][0-9]|1?[0-9]?[0-9])){3}\b' $revs -- . ':!docs/standards/VM-LAB-STANDARD.md' 2>/dev/null | sort -u | grep -vE '^(127\.0\.0\.1|0\.0\.0\.0)$' || true)"
for ip in $ips; do allowed "$ip" && continue; report "IPv4 address in history: $ip"; done
macs="$(git grep -IhoiE '\b[0-9a-f]{2}([:_-])[0-9a-f]{2}(\1[0-9a-f]{2}){4}\b' $revs 2>/dev/null | sort -u | grep -viE '^02' || true)"
for m in $macs; do allowed "$m" && continue; report "MAC address in history: $m"; done
keys="$(git grep -Il 'PRIVATE KEY-----' $revs -- . ':!docs/standards/*' ':!scripts/scan-history.sh' ':!scripts/postmortem.mjs' 2>/dev/null | head -5 || true)"
[ -n "$keys" ] && report "private key material in: $keys"
# EDIDs (any blob starting with the EDID header) must have the serial number zeroed and no serial
# string descriptor other than REDACTED. The parity vectors named synthetic-* carry made-up serials.
for rev in $revs; do
  git ls-tree -r "$rev" | awk '$2 == "blob" {print $3, $4}'
done | sort -u | while read -r blob p; do
  case "$p" in tests/fixtures/parity/edid/synthetic-*) continue ;; esac
  [ "$(git cat-file -s "$blob")" -ge 128 ] || continue
  [ "$(git cat-file blob "$blob" | head -c 8 | od -An -tx1 | tr -d ' \n')" = "00ffffffffffff00" ] || continue
  git cat-file blob "$blob" | python3 -c '
import sys
b = sys.stdin.buffer.read(128)
bad = b[12:16] != bytes(4)
for o in (54, 72, 90, 108):
    d = b[o:o+18]
    if d[:4] == bytes([0, 0, 0, 0xff]) and d[5:].split(b"\n")[0].strip() not in (b"", b"REDACTED"):
        bad = True
sys.exit(1 if bad else 0)' || echo "unredacted EDID in history: $p"
done > "${TMPDIR:-/tmp}/scan-edid.$$"
while read -r line; do report "${line}"; done < "${TMPDIR:-/tmp}/scan-edid.$$"; rm -f "${TMPDIR:-/tmp}/scan-edid.$$"
[ $problems -eq 0 ] && echo "history: clean" || { echo "history: $problems problem(s)"; exit 1; }
