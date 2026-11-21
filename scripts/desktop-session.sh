#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# desktop-session.sh — switches the desktop GDM logs lab into (gnome | plasma | sway) and waits
# until that compositor and its shell are up. Runs inside the desktop guest only:
#   LAB_ENV=lab.desktop.env scripts/lab.sh exec -r desktop 'bash scripts/desktop-session.sh sway'
# On success prints one `export ...` line a client needs to reach the session; exits 1 if the
# requested compositor isn't running after 5 minutes (so no capture runs against another desktop).
set -euo pipefail
want="${1:?gnome|plasma|sway}"
case "$want" in
  gnome) s=gnome; ready=(gnome-shell) ;;
  plasma) s=plasma; ready=(kwin_wayland) ;;   # plasmashell needs GPU GL; KWin holds the output state
  sway) s=sway; ready=(sway) ;;
  *) exit 2 ;;
esac
sudo systemctl stop gdm3
# End only lab's graphical sessions (seat0); its SSH sessions, including this one, stay.
for id in $(loginctl list-sessions --no-legend | awk '$3 == "lab" && $4 == "seat0" {print $1}'); do
  sudo loginctl terminate-session "$id" 2>/dev/null || true
done
for _ in $(seq 1 30); do pgrep -u lab -x 'gnome-shell|kwin_wayland|sway|plasmashell' >/dev/null || break; sleep 1; done
sudo pkill -KILL -u lab -x 'gnome-shell|kwin_wayland|sway|plasmashell' 2>/dev/null || true
sudo mkdir -p /var/lib/AccountsService/users
printf '[User]\nSession=%s\nXSession=%s\nSystemAccount=false\n' "$s" "$s" | sudo tee /var/lib/AccountsService/users/lab >/dev/null
sudo systemctl restart accounts-daemon
sudo systemctl start gdm3
up() { for p in "${ready[@]}"; do pgrep -u lab -x "$p" >/dev/null || return 1; done; }
for _ in $(seq 1 150); do up && break; sleep 2; done
up || { echo "desktop-session: $want did not start (waited 300 s)" >&2; exit 1; }
sleep 10   # let outputs settle
sock="$(basename "$(ls /run/user/1000/wayland-? 2>/dev/null | head -n 1)")"
[ -n "$sock" ] || { echo "desktop-session: no Wayland socket" >&2; exit 1; }
echo "desktop-session: $want up (${ready[*]})" >&2
echo "export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=$sock DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=$want"
