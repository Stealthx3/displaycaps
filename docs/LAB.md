# Lab — displaycaps
_Standard: VM-LAB-STANDARD v1.2 · Created 2026-11-21_

## Host (from lab.sh doctor)
- `lab.sh doctor`: OK (virt-install with cloud-init, virt-manager, KVM, LXC driver all present)
- Network: default (NAT) · Pool: default
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

## Commands
- Build: `scripts/lab.sh run 'cargo build --workspace'`
- Gate:  `scripts/lab.sh run 'bash scripts/gate.sh 2>&1 | tail -n 60'`
- Generated files back to the host (lockfile, schema, `cargo fmt` output, goldens): `scripts/lab-sync-back.sh Cargo.lock schema crates tests/fixtures`
- Watch: `scripts/lab.sh view`

## Host exceptions
- Real-hardware runs (workstation, HTPC) need a one-time OK from the maintainer per test (CLAUDE.md).
- 2026-11-21 — workstation capture: lab-built `dist/displaycaps capture` run once on the host under `scripts/check-passive.sh` (approved; passed: no DRM setters, no master, no forced probe, no I2C/CEC). `drm_info` isn't installed on the host, so the capture has no KMS dump.

## Log
- 2026-11-21 — doctor OK; Debian 13 base present. lab-displaycaps-main created and provisioned (Rust 1.99.1, cargo-deny 0.22.0, cargo-fuzz 0.13.2).
- 2026-11-21 — `scripts/lab.sh` is the master v1.2 copy; AudioCaps' local `put` command is not carried over (copy files with `lab.sh run` + the repo instead).
