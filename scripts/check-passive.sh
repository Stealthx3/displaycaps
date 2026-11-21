#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# Adapted from AudioCaps v1.0.0 (commit 049d053) scripts/check-passive.sh: the display rules
# instead of the sound-device rules.
# check-passive.sh — runs a command (default: `displaycaps capture`) under strace, following child
# processes, and fails on anything that could change what is on screen:
#   - DRM ioctls that set state: SETCRTC, ATOMIC, SETPROPERTY / OBJ_SETPROPERTY, SETGAMMA, DIRTYFB,
#     page flips, cursor, SET_MASTER / DROP_MASTER, lease creation;
#   - a forced connector probe: DRM_IOCTL_MODE_GETCONNECTOR with count_modes=0 (what
#     drmModeGetConnector sends; drmModeGetConnectorCurrent never does);
#   - opening any /dev/i2c-* node (no DDC/CI or EDID-over-I2C in passive code), or any /dev/cec*;
#   - any write(2)/open for writing under /sys/class/drm or /sys/kernel/debug/dri.
# Prints the command's output on success.
#   - opening a primary /dev/dri/card* node outside a display session (the kernel would make the
#     process DRM master implicitly). Attempts count, even if they fail.
# Usage (in a guest, or on real hardware with the maintainer's OK):
#   bash scripts/check-passive.sh <label>
#   PASSIVE_CMD='displaycaps probe' bash scripts/check-passive.sh
set -euo pipefail
label="${1:-system}"
out="${CAPTURE_OUT:-$HOME/captures}"
trace="$(mktemp)"
mkdir -p "$out"
cmd=(displaycaps capture --out "$out" --label "$label")
# shellcheck disable=SC2206
[ -n "${PASSIVE_CMD:-}" ] && cmd=($PASSIVE_CMD)
strace -f -qq -v -xx -s 512 -e trace=open,openat,openat2,ioctl,write -o "$trace.named" "${cmd[@]}" >"$trace.out" || cmd_rc=$?
# strace -xx prints every string as hex, so paths and ioctl structs are decoded here before any
# rule is applied. Fails closed: a line that can't be decoded counts as a violation.
SESSION="$([ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ] && echo 1 || echo 0)" python3 - "$trace.named" <<'PY2'
import os, re, sys
HEX = re.compile(r'"((?:\\x[0-9a-f]{2})*)"(\.\.\.)?')
def dec(m): return bytes.fromhex(m.group(1).replace("\\x", ""))
SETTERS = re.compile(r"DRM_IOCTL_(MODE_(SETCRTC|ATOMIC|SETPROPERTY|OBJ_SETPROPERTY|SETGAMMA|DIRTYFB|PAGE_FLIP|CURSOR2?|CREATE_LEASE|REVOKE_LEASE|SETPLANE|RMFB|CLOSEFB)|SET_MASTER|DROP_MASTER)\b")
session = os.environ.get("SESSION") == "1"
bad, opens, ioctls, getconn, primary = [], 0, 0, 0, 0
for line in open(sys.argv[1], errors="replace"):
    m = re.search(r"\b(open|openat|openat2)\(", line)
    if m:
        opens += 1
        s = HEX.search(line)
        if not s:
            bad.append("undecodable open: " + line.strip()[:120]); continue
        path = dec(s).decode("utf-8", "replace")
        flags = line[s.end():]
        if re.match(r"/dev/(i2c-|cec)", path):
            bad.append(f"opened I2C/CEC node {path}")
        if re.match(r"/sys/(class/drm|kernel/debug/dri|devices/.*/drm/)", path) and re.search(r"O_(WRONLY|RDWR)", flags):
            bad.append(f"opened {path} for writing")
        if re.fullmatch(r"/dev/dri/card\d+", path):
            primary += 1
            if not session:
                bad.append(f"opened primary node {path} outside a display session (implicit DRM master)")
        continue
    if "ioctl(" in line and "DRM_IOCTL" in line:
        ioctls += 1
        if SETTERS.search(line):
            bad.append("state-changing ioctl: " + line.strip()[:120])
        if "DRM_IOCTL_MODE_GETCONNECTOR" in line:
            getconn += 1
            s = HEX.search(line)
            b = dec(s) if s else b""
            # struct drm_mode_get_connector: count_modes is the u32 at offset 32; 0 forces a probe.
            if len(b) < 36 or int.from_bytes(b[32:36], "little") == 0:
                bad.append("forced connector probe (count_modes=0) or undecodable: " + line.strip()[:100])
    elif re.search(r"\bwrite\(", line) and "/sys/" in line:
        bad.append("write under /sys: " + line.strip()[:100])
if bad:
    print("PASSIVE CHECK FAILED:"); [print("  " + x) for x in bad[:20]]; sys.exit(1)
print(f"passive check: ok ({opens} opens, {ioctls} DRM ioctls, {getconn} GETCONNECTOR, {primary} primary-node opens"
      f"{' inside a session' if session else ''}; no setters, no master, no forced probe, no I2C/CEC, no sysfs writes)")
PY2
rc=$?
[ $rc -eq 0 ] || { rm -f "$trace" "$trace.out" "$trace.named"; exit 1; }
[ "${cmd_rc:-0}" -eq 0 ] || echo "note: the traced command exited ${cmd_rc}"
cat "$trace.out"
rm -f "$trace" "$trace.out" "$trace.named"
