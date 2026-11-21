#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# parity-audiocaps.sh — regenerates tests/fixtures/parity/audiocaps-v1.0.0.json: the sink identity
# AudioCaps v1.0.0 itself reports for each shared EDID vector. Runs in the build guest only:
#   scripts/lab.sh run 'bash scripts/parity-audiocaps.sh' && scripts/lab-sync-back.sh tests/fixtures
# AudioCaps is cloned from its public repo at tag v1.0.0 (commit checked) into ~/oracles; the local
# AudioCaps checkout is never touched. Each vector goes into a copy of AudioCaps' own
# synthetic-hdmi-earc-cec fixture (EDID file replaced, ELD manufacturer/product patched to match,
# as the kernel would) and is probed with `audiocaps replay`.
set -euo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cd "$(dirname "$0")/.."
REPO=https://github.com/Stealthx3/audiocaps
COMMIT=049d053f4f590667a1706fa001c2e75611bb9ff5
AC="$HOME/oracles/audiocaps-v1.0.0"
if [ ! -d "$AC/.git" ]; then
  git clone -q --depth 1 --branch v1.0.0 "$REPO" "$AC"
fi
[ "$(git -C "$AC" rev-parse HEAD)" = "$COMMIT" ] || { echo "parity: AudioCaps checkout is not $COMMIT" >&2; exit 1; }
if ! dpkg -s libasound2-dev libpipewire-0.3-dev libpulse-dev libjack-jackd2-dev >/dev/null 2>&1; then
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq libasound2-dev libpipewire-0.3-dev \
    libspa-0.2-dev libpulse-dev libjack-jackd2-dev >/dev/null
fi
(cd "$AC" && cargo build -q --release -p audiocaps-cli 2>&1 | tail -n 5)
BIN="$AC/target/release/audiocaps"
"$BIN" --version
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
out=tests/fixtures/parity/audiocaps-v1.0.0.json
python3 - "$BIN" "$AC/tests/fixtures/synthetic-hdmi-earc-cec" "$work" tests/fixtures/parity/edid "$out" "$COMMIT" <<'PY'
import json, os, shutil, subprocess, sys, glob
binary, template, work, vecdir, out, commit = sys.argv[1:]
edid_rel = "root/sys/devices/pci0000:00/0000:00:01.0/0000:01:00.0/drm/card0/card0-HDMI-A-1/edid"
results = {}
for vec in sorted(glob.glob(os.path.join(vecdir, "*.bin"))):
    name = os.path.basename(vec)[:-4]
    fx = os.path.join(work, name)
    shutil.copytree(template, fx, symlinks=True)
    b = open(vec, "rb").read()
    open(os.path.join(fx, edid_rel), "wb").write(b)
    alsa = os.path.join(fx, "commands/alsa.json")
    j = json.load(open(alsa))
    for c in j["cards"][0]["controls"]:
        if c["name"] == "ELD":
            c["values"][16:20] = list(b[8:12])   # manufacturer + product, as drm_edid_to_eld copies them
    json.dump(j, open(alsa, "w"))
    snap = json.loads(subprocess.run([binary, "replay", fx], check=True, capture_output=True, text=True).stdout)
    ep = next(e for e in snap["endpoints"] if e["id"] == "alsa:card0/pcm3p")
    ident = ep["sink"]["identity"]
    val = lambda v: v.get("value") if v.get("state") == "known" else None
    results[name] = {
        "sink_key": val(ident["sink_key"]),
        "manufacturer_id": val(ident["manufacturer_id"]),
        "product_code": val(ident["product_code"]),
        "model_name": val(ident["model_name"]),
    }
doc = {"generator": "scripts/parity-audiocaps.sh", "audiocaps": {"version": "1.0.0", "commit": commit},
       "note": "Sink identity as AudioCaps v1.0.0 reports it for each vector in edid/ (ELD model-name fallback applies when the EDID has none).",
       "vectors": results}
open(out, "w").write(json.dumps(doc, indent=2) + "\n")
print(f"parity: {len(results)} vectors -> {out}")
for k, v in results.items(): print(f"  {k}: {v['sink_key']}")
PY
