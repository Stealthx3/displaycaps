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
strace -f -qq -v -xx -s 96 -e trace=open,openat,openat2,ioctl,write -o "$trace.named" "${cmd[@]}" >"$trace.out"
opens="$(grep -cE 'open(at2?)?\(' "$trace.named" || true)"
fail=0
setters="$(grep -E 'DRM_IOCTL_(MODE_(SETCRTC|ATOMIC|SETPROPERTY|OBJ_SETPROPERTY|SETGAMMA|DIRTYFB|PAGE_FLIP|CURSOR2?|CREATE_LEASE|REVOKE_LEASE|SETPLANE|RMFB|CLOSEFB)|SET_MASTER|DROP_MASTER)' "$trace.named" || true)"
if [ -n "$setters" ]; then echo "PASSIVE CHECK FAILED: state-changing DRM ioctls:"; echo "$setters" | head -n 20; fail=1; fi
# strace prints struct drm_mode_get_connector as raw bytes; count_modes is the u32 at offset 32 of
# the buffer passed in. 0 is what drmModeGetConnector sends (the kernel then re-probes the
# connector); drmModeGetConnectorCurrent always sends 1 with a stack buffer.
forced="$(grep 'DRM_IOCTL_MODE_GETCONNECTOR' "$trace.named" | python3 -c '
import re, sys
for line in sys.stdin:
    m = re.search(r"GETCONNECTOR, \"((?:\\x[0-9a-f]{2})+)", line)
    if not m:
        print("undecodable: " + line.strip()); continue
    b = bytes.fromhex(m.group(1).replace("\\x", ""))
    if len(b) < 36 or int.from_bytes(b[32:36], "little") == 0:
        print(line.strip()[:120])
' || true)"
if [ -n "$forced" ]; then echo "PASSIVE CHECK FAILED: forced connector probes (count_modes=0):"; echo "$forced" | head -n 10; fail=1; fi
i2c="$(grep -E '"/dev/(i2c-|cec)' "$trace.named" || true)"
if [ -n "$i2c" ]; then echo "PASSIVE CHECK FAILED: opened I2C or CEC nodes:"; echo "$i2c" | head -n 10; fail=1; fi
sysw="$(grep -E '"/sys/(class/drm|kernel/debug/dri)[^"]*", O_(WRONLY|RDWR)' "$trace.named" || true)"
if [ -n "$sysw" ]; then echo "PASSIVE CHECK FAILED: opened DRM sysfs/debugfs for writing:"; echo "$sysw" | head -n 10; fail=1; fi
getconn="$(grep -c 'DRM_IOCTL_MODE_GETCONNECTOR' "$trace.named" || true)"
drmio="$(grep -c 'DRM_IOCTL' "$trace.named" || true)"
[ "$fail" = 0 ] || { rm -f "$trace" "$trace.out" "$trace.named"; exit 1; }
echo "passive check: ok ($opens opens, $drmio DRM ioctls traced, $getconn GETCONNECTOR, none forcing a probe; no setters, no master, no I2C/CEC)"
cat "$trace.out"
rm -f "$trace" "$trace.out" "$trace.named"
