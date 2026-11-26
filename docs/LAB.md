# Lab — displaycaps
_Standard: VM-LAB-STANDARD v1.2 · Created 2026-11-21_

## Host (from lab.sh doctor)
- `lab.sh doctor`: OK (virt-install with cloud-init, virt-manager, KVM, LXC driver all present)
- Network: default (NAT) · Pool: VMDisks (a data drive; never libvirt's default pool, VM-LAB-STANDARD §4)
- Host meets the requirements; other projects' guests may be running, so keep one DisplayCaps guest up at a time.
- Real host details, if ever needed, go in git-ignored `docs/LAB.local.md` (PII-REDACTION-STANDARD §6).

## Guest decision (§2)
| Role | Kind | Rule | "Yes" answers | Note |
|---|---|---|---|---|
| main | vm | 2 | Q7, Q8 | Rust build, tests, cargo-deny, fuzz, test oracles; reverts keep the toolchain clean |
| desktop | vm | 2 | Q2, Q7, Q8 | virtio-gpu (KMS with an emulated EDID) running KWin, GNOME and Sway sessions for captures; needs systemd, a real DRM device and one snapshot per desktop |

## Guests (watch in virt-manager → QEMU/KVM)
| Guest | Kind | Base | vCPU/RAM | Purpose | Snapshots |
|---|---|---|---|---|---|
| lab-displaycaps-main | vm | lab-base-debian13 | 4 / 6 GB | build, tests, cargo-deny, fuzz build, edid-decode and di-edid-decode oracles | clean, provisioned |
| lab-displaycaps-desktop | vm | lab-base-debian13 | 4 / 8 GB | virtio-gpu (no virgl) with GNOME, Plasma (KWin; plasmashell needs GPU GL) and Sway via GDM autologin; captures | clean, desktops |

## Provisioning
- scripts/provision.sh: Rust toolchain (rustup, pinned by rust-toolchain.toml), build deps (pkg-config, clang, libdrm-dev, libwayland-dev, libdbus-1-dev, strace), cargo-deny, cargo-fuzz on the pinned nightly.

## Capture workflow (desktop guest)
- `LAB_ENV=lab.desktop.env scripts/lab.sh up desktop`, then per desktop: `line=$(bash scripts/desktop-session.sh <gnome|plasma|sway>) && eval "$line" && bash scripts/check-passive.sh lab-<desktop>` inside `lab.sh run -r desktop`.
- Pull with `scripts/lab.sh pull '/home/lab/captures/*' captures desktop` before any revert; shut the guest down before starting `main`.
- `desktop-session.sh` ends only lab's seat0 sessions (not the SSH one) and fails rather than capture the wrong desktop.

## Passive check (scripts/check-passive.sh)
- Proven in lab-displaycaps-desktop with one negative control per rule (2026-11-21): `modetest -c` (forced probe), `drm_info` with no session (implicit DRM master on a primary node), `cat /dev/i2c-0`, `cat /dev/cec0`, a `dd` write to a connector's sysfs `status`. Each fails; captures and in-session `drm_info` pass.

## KMS checks (phase 3, desktop guest)
- Binary: build in main (`cargo build --release -p displaycaps-cli`), `scripts/lab.sh pull /home/lab/src/target/release/displaycaps dist`, shut main down, start desktop, and copy `dist/displaycaps` into the guest's `/home/lab/src/dist/` (rsync over the lab key; `dist/` is git-ignored, so `lab.sh push` skips it).
- `scripts/kms-diff.sh <label>` inside a session: our ioctl read vs `drm_info -j`, through the same normalization.
- Hotplug: a sysfs `status` write sends no uevent; unbinding and rebinding the GPU's PCI driver in the guest (root, lab only) does (`remove`/`add` on the DRM devices). `vkms` isn't in the Debian 13 kernel.
- Mode change: `swaymsg output Virtual-1 mode …` inside the Sway session while `displaycaps watch --layer kms --kms-poll 500` runs (the poll is opt-in).
- The desktop guest's virtio GPU has two heads (2026-11-26); the second stays disconnected with no cached modes, so check-passive covers the GETCONNECTOR path that would otherwise send `count_modes = 0`.

## Compositor checks (phase 5, desktop guest)
- Sessions: `scripts/desktop-session.sh <gnome|plasma|sway|plasmax11>`; `plasmax11` is Plasma on Xorg (the RandR path; Xorg and kwin-x11 were added to the desktop guest on 2026-11-26 and to `provision-desktop.sh`). The script prints the session's environment, `DISPLAY` and `XAUTHORITY` for X11.
- `scripts/compositor-gate.sh <kwin|sway|gnome|x11>`: compares the compositor layer with the desktop's own tool (`kscreen-doctor -j`/`-o`, `swaymsg -t get_outputs`, `gdctl show`, `xrandr --current`, normalized by `scripts/compositor-oracle.py`) in the default state, after a scale change and after a mode change made with that tool, then restores the defaults. One comparison: `scripts/compositor-diff.sh <desktop> <label>`.
- `scripts/compositor-passive.sh <wayland|x11>`: logs every request the probe and a watch send (WAYLAND_DEBUG=1; dbus-monitor on the session bus for Mutter; an xtrace proxy for X11, with the lab user let in by `xhost +si:localuser` for the run) and fails on anything but the read-only set.
- `scripts/compositor-watch-check.sh <desktop>`: a scale change made with the desktop's tool must arrive as a `CompositorChange` watch event.
- `swaymsg` needs `SWAYSOCK`; the scripts find it under `$XDG_RUNTIME_DIR`.
- The virtio GPU and QEMU's EDID offer no HDR, VRR or RGB-range control, so those switches can't be flipped in the lab; their mapping is covered by unit tests over simulated facts (`compositor/tests.rs`).

## Commands
- Build: `scripts/lab.sh run 'cargo build --workspace'`
- Gate:  `scripts/lab.sh run 'bash scripts/gate.sh 2>&1 | tail -n 60'`
- Generated files back to the host (lockfile, schema, `cargo fmt` output, goldens): `scripts/lab-sync-back.sh Cargo.lock schema crates tests/fixtures`
- Watch: `scripts/lab.sh view`

## Host exceptions
- Real-hardware runs (workstation, HTPC) need a one-time OK from the maintainer per test (CLAUDE.md).
- 2026-11-21 — workstation capture: lab-built `dist/displaycaps capture` run once on the host under `scripts/check-passive.sh` (approved; passed: no DRM setters, no master, no forced probe, no I2C/CEC). `drm_info` isn't installed on the host, so the capture has no KMS dump.

## Log
- 2026-11-26 — Phase 5 gate: compositor-gate identical on GNOME (6 facts per state), KWin (9), Sway (5) and Plasma on X11 (3; no scale on X11), each in the default state and after a scale and a mode change; compositor-passive: Wayland 33 (GNOME), 36 (KWin) and 16 (Sway) requests, all read-only; Mutter got only GetCurrentState (2 calls: probe and watch); X11 38 requests, all read-only, no GetScreenResources. Watch: CompositorChange on all four; re-probe 3 ms (Sway), 60–78 ms (GNOME, KWin, X11) while the compositor applies the change (KWin's own answers take 40 ms at rest, 97 ms during the change, with software rendering). Probe of compositor + environment, median of 20: 6.7 ms GNOME, 19 ms KWin, 1.2 ms Sway.
- 2026-11-21 — doctor OK; Debian 13 base present. lab-displaycaps-main created and provisioned (Rust 1.99.1, cargo-deny 0.22.0, cargo-fuzz 0.13.2).
- 2026-11-26 — Phase 3 gate (after review fixes, two heads): kms-diff identical on GNOME, Plasma and Sway (46 facts each); check-passive ok for `probe` and `watch --kms-poll` in each session; seat-rule control: with seat0 switched to a text VT (VT_ACTIVATE ioctl as root; `chvt` isn't installed) and the session variables still set, `probe` opens no card and reports `no_session`. First run (one head): check-passive ok for `probe` and `watch` in each session and for `probe` outside a session (0 primary-node opens); hotplug via PCI unbind/bind and Sway mode changes reported by `displaycaps watch`. After the unbind test the card came back as `card1`; a reboot restores `card0`.
- 2026-11-21 — `scripts/lab.sh` is the master v1.2 copy; AudioCaps' local `put` command is not carried over (copy files with `lab.sh run` + the repo instead).
- 2026-11-21 — All lab guests' disks (main, desktop and their snapshots) and the base volume moved to pool VMDisks by the maintainer's request (done from another project's session); LAB_POOL=VMDisks.
