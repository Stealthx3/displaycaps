#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# Copied from AudioCaps v1.0.0 (commit 049d053) scripts/gate.sh; adds the provenance-header check.
# gate.sh — the phase quality bar in one call: fmt, clippy, tests, cargo-deny, fuzz build,
# SPDX headers, provenance headers on code copied from AudioCaps.
# Runs inside the build guest: scripts/lab.sh run 'bash scripts/gate.sh'
# Prints one line per check plus the failing output (trimmed); exit 1 if any check fails.
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
fail=0
check() {
  local name="$1"; shift
  local out; out="$("$@" 2>&1)"; local rc=$?
  if [ $rc -eq 0 ]; then echo "ok   $name"; else echo "FAIL $name"; echo "$out" | tail -n 40; fail=1; fi
}
check "cargo fmt --check" cargo fmt --all --check
check "cargo clippy -D warnings" cargo clippy --workspace --all-targets -- -D warnings
check "cargo test" cargo test --workspace --no-fail-fast -q
check "cargo deny check" cargo deny check
# Fuzz harness: separate workspace on a pinned nightly; every parser target must build.
check "fuzz targets build" bash -c 'cd fuzz && cargo fuzz build -O'
check "cargo deny check (fuzz)" bash -c 'cd fuzz && cargo deny check'
# Verbatim copies of shared ai-assets scripts are exempt (they must not be edited per project).
VERBATIM='scripts/(lab\.sh|progress\.mjs|repo-check\.mjs|phone-alerts\.mjs|postmortem\.mjs)$'
spdx_missing="$(find crates scripts -type f \( -name '*.rs' -o -name '*.sh' -o -name '*.mjs' -o -name '*.toml' \) -print | grep -Ev "$VERBATIM" | xargs grep -L 'SPDX-License-Identifier: LGPL-2.1-or-later' || true)"
spdx_missing="$spdx_missing$(for f in Cargo.toml deny.toml rust-toolchain.toml rustfmt.toml clippy.toml; do grep -q 'SPDX-License-Identifier' "$f" || echo "$f"; done)"
if [ -z "$spdx_missing" ]; then echo "ok   SPDX headers"; else echo "FAIL SPDX headers"; echo "$spdx_missing"; fail=1; fi
# Code copied from AudioCaps names its source and commit (CLAUDE.md, "Code reuse").
prov_bad="$(grep -rlE '(Copied|Adapted) from AudioCaps' crates scripts fuzz --include='*.rs' --include='*.sh' --include='*.toml' 2>/dev/null | xargs -r grep -LE '(Copied|Adapted) from AudioCaps v1\.0\.0 \(commit 049d053\)' || true)"
if [ -z "$prov_bad" ]; then echo "ok   provenance headers ($(grep -rlE '(Copied|Adapted) from AudioCaps' crates scripts fuzz 2>/dev/null | wc -l) copied files)"; else echo "FAIL provenance headers"; echo "$prov_bad"; fail=1; fi
[ $fail -eq 0 ] && echo "gate: PASS" || echo "gate: FAIL"
exit $fail
