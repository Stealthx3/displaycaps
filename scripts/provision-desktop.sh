#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# provision-desktop.sh — idempotent setup of lab-displaycaps-desktop: the standard Debian kernel
# (the cloud kernel has no DRM drivers, so no virtio-gpu), GDM with autologin, and the three
# desktops the captures cover (GNOME, Plasma/KWin, Sway), plus the read-only tools the capture
# records. Runs inside the desktop guest only:
#   LAB_ENV=lab.desktop.env scripts/lab.sh run -r desktop 'bash scripts/provision-desktop.sh'
# Then reboot the guest (lab.sh down desktop; up desktop) so the standard kernel and GDM start.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
sudo apt-get update -qq
sudo apt-get install -y -qq linux-image-amd64 >/dev/null
# Boot the standard kernel: newest non-cloud vmlinuz first in GRUB.
std="$(ls /boot/vmlinuz-*-amd64 2>/dev/null | grep -v cloud | sort -V | tail -n 1 || true)"
if [ -n "$std" ]; then
  ver="${std#/boot/vmlinuz-}"
  sudo sed -i "s|^GRUB_DEFAULT=.*|GRUB_DEFAULT=\"Advanced options for Debian GNU/Linux>Debian GNU/Linux, with Linux $ver\"|" /etc/default/grub
  sudo update-grub >/dev/null 2>&1
fi
sudo apt-get install -y -qq --no-install-recommends \
  gdm3 gnome-session gnome-shell \
  plasma-workspace kwin-wayland libkscreen-bin kscreen \
  sway foot \
  xserver-xorg-core xserver-xorg-input-libinput kwin-x11 xauth xtrace dbus-bin \
  wayland-utils wlr-randr drm-info libdrm-tests x11-xserver-utils libglib2.0-bin \
  vainfo vulkan-tools mesa-va-drivers mesa-vulkan-drivers \
  strace python3 dbus-user-session >/dev/null
sudo usermod -aG video,render lab
# GDM: automatic login as lab. The session is picked by scripts/desktop-session.sh.
sudo mkdir -p /etc/gdm3
sudo tee /etc/gdm3/daemon.conf >/dev/null <<'CONF'
[daemon]
AutomaticLoginEnable=true
AutomaticLogin=lab
WaylandEnable=true
CONF
systemctl get-default | grep -q graphical || sudo systemctl set-default graphical.target >/dev/null
echo "provision-desktop: $(uname -r) running; standard kernel: ${ver:-none}; reboot to use it"
