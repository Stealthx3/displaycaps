#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# desktop-session.sh — picks the desktop GDM logs lab into (gnome | plasma | sway), restarts GDM
# and waits until that compositor's Wayland socket is up. Runs inside the desktop guest only:
#   LAB_ENV=lab.desktop.env scripts/lab.sh exec -r desktop 'bash scripts/desktop-session.sh sway'
# Prints the environment a client needs to reach the session.
set -euo pipefail
want="${1:?gnome|plasma|sway}"
case "$want" in gnome) s=gnome; p=gnome-shell ;; plasma) s=plasma; p=kwin_wayland ;; sway) s=sway; p=sway ;; *) exit 2 ;; esac
sudo mkdir -p /var/lib/AccountsService/users
printf '[User]\nSession=%s\nSystemAccount=false\n' "$s" | sudo tee /var/lib/AccountsService/users/lab >/dev/null
sudo systemctl restart gdm3
for _ in $(seq 1 60); do
  pgrep -u lab -x "$p" >/dev/null && ls /run/user/1000/wayland-* >/dev/null 2>&1 && break
  sleep 2
done
pgrep -u lab -x "$p" >/dev/null || { echo "desktop-session: $p did not start" >&2; exit 1; }
sock="$(basename "$(ls /run/user/1000/wayland-? 2>/dev/null | head -n 1)")"
echo "desktop-session: $want up ($p)"
echo "export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=$sock DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus XDG_SESSION_TYPE=wayland"
