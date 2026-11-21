# CLAUDE.md — DisplayCaps

Standing instructions for Claude Code working in this repository. Read this
file, `docs/DECISIONS.md`, `docs/PLAN.md` and the full plan in
`docs/plan/DisplayCaps-Development-Plan.md` (the PDF next to it has the
diagrams) before changing anything. Once phase 1 creates it, read
`docs/USAGE-PLAN.md` too.

## What this is

DisplayCaps (working name) is a library that applications bundle to learn what
the display path can really show, and where anything is lost on the way: from
the TV, monitor or AV receiver (EDID, DDC/CI, CEC) back through the HDMI/DP
link, GPU driver and KMS, the compositor (Wayland or X11) and the video decoder,
to the app. It is AudioCaps' sister library; a third, combined library is
planned. Linux 1.0 first; Windows and macOS later on the same OS-neutral core.

## Decisions that are settled (do not reopen without asking)

- **Language:** one Rust core plus a cbindgen-generated C header. No other
  implementation languages, no maintained per-language bindings, no published
  Rust crate. Consumers bundle the shared library and follow project docs.
- **License:** LGPL-2.1-or-later. Every source file carries
  `SPDX-License-Identifier: LGPL-2.1-or-later`. Dual MIT/Apache-2.0 crates are
  taken under MIT. Apache-2.0-only, GPL-only or unknown licenses need the maintainer's
  approval.
- **Sandboxes:** must work fully in-process inside Flatpak and Snap. No host
  service or daemon. A layer that can't be read is `unknown` with a reason,
  never "unsupported".
- **Code reuse:** copy AudioCaps v1.0.0's generic code (core types, SystemRoot,
  capture, redaction, runtime, rule and quirk engine, sink identity and key, C
  ABI pattern) into this repo and maintain it here. No shared crate, no git
  dependency on AudioCaps. Each copied file names its AudioCaps source path and
  commit in a header comment. **Never modify the AudioCaps repo**
  (the sibling checkout `../audiocaps`); read it at tag `v1.0.0`.
- **EDID parsing:** copy AudioCaps' Rust EDID parser and extend it with every
  video block. Do not link or fork libdisplay-info. edid-decode and
  libdisplay-info's `di-edid-decode` run only in the lab as test oracles.
- **Library family:** keep everything distinct from AudioCaps so the combined
  library can hold both in one process: C symbols `dc_*`, soname
  `libdisplaycaps.so.1`, header `displaycaps.h`, schema ids, data paths
  (`/usr/share/displaycaps/`, `/etc/displaycaps/`). A parity test keeps the
  sink key identical to AudioCaps' for the same EDID.
- **Crates:** `displaycaps-model`, `displaycaps-parse`, `displaycaps-link`,
  `displaycaps-backends`, `displaycaps-rules`, `displaycaps-ffi`,
  `displaycaps-cli`.
- **Rules and quirk data:** declarative packs bundled with each release,
  overridable from a system directory or the app's bundle. Never fetched from
  the network.

Plan defaults marked "Proposed" in the decision table of the plan (read-only
rule, recommend-never-act, 1.0 scope, first consumers, name) are followed as
written; raise any change at a gate.

## Guiding principles (apply to every API and implementation choice)

1. **Information on-demand.** Probe and return only the scope the application
   asks for. Nothing outside the scope is read, cached or reported.
2. **The application decides.** Decode checks, DDC/CI, CEC and other opt-ins
   run only when the app calls them, and do exactly what was asked.
3. **Passive by default.** Never set a mode or any KMS property, never take DRM
   master, never force a connector probe (use `drmModeGetConnectorCurrent`,
   never `drmModeGetConnector`), never read EDID over I2C, never write DDC/CI,
   never change compositor settings (HDR, VRR, scale). What is on screen must
   never flicker or change.
4. **Recommend, never act.** Mode and HDR recommendations are returned to the
   app; the app or compositor applies them.

## Working rules

- **Phases and gates.** Work follows the roadmap in `docs/PLAN.md` (9 phases to
  Linux 1.0). Finish a phase on its own branch, meet its gate, then **stop**:
  summarize what was built and how the gate was verified, and wait for
  the maintainer's approval before starting the next phase. This holds for every
  phase.
- **Ask before:** reopening a settled decision, adding a dependency with a new
  license family, installing system packages on the host, touching the host's
  display or compositor configuration, creating or publishing anything outside
  this repo (GitHub repo, Pages, releases), or anything irreversible.
- **Notify:** when a gate is reached or a decision needs the maintainer, send a phone
  push notification per PHONE-ALERTS-STANDARD; otherwise say so prominently at
  the end of the response.
- **Dev and test environment:** build and test only inside libvirt/KVM VMs or
  LXC containers on this machine (visible in Virtual Machine Manager), through
  `scripts/lab.sh` (see `docs/LAB.md`). Guests are named `lab-displaycaps-*`;
  reuse the existing base volumes; keep one guest running at a time unless the
  host has room. Real-hardware tests on the host or the HTPC need a one-time
  exception from the maintainer per test.
- **Standards:** copies of the shared ai-assets standards live in
  `docs/standards/` and apply to this project: REPO-STRUCTURE-STANDARD.md,
  PROGRESS-BAR-STANDARD.md, PHONE-ALERTS-STANDARD.md, VM-LAB-STANDARD.md,
  TOKEN-USAGE-OPTIMIZATION-STANDARD.md, POST-MORTEM-STANDARD.md,
  PII-REDACTION-STANDARD.md (masters in the
  shared ai-assets folder; its path is in `docs/plan/local-environment.md`). Shared scripts are copied
  verbatim and not edited per project. If a standard conflicts with this file,
  ask.
- Build/test only via `scripts/lab.sh` (see `docs/LAB.md`). Guests are
  watchable in virt-manager. `scripts/gate.sh` runs the quality bar in the lab.
- Phone alerts: see PHONE-ALERTS-STANDARD; hooks installed, check with
  `node scripts/phone-alerts.mjs check`.
- Progress: tick `docs/PLAN.md` in the same commit as the work, then run
  `node scripts/progress.mjs` and `node scripts/repo-check.mjs progress`.
- Usage plan: `docs/USAGE-PLAN.md` (read the handoff note first). Append one
  line per session to `docs/postmortem/JOURNAL.md`.

## Repository conventions

- Branches: `main` is always releasable; work on `<type>/<kebab-case>` branches
  (phase work: `feat/phase-N-<topic>`; also `fix/`, `docs/`, `chore/`).
  Conventional Commit messages (`feat:`, `fix:`, `docs:`, `test:`, `chore:`,
  `refactor:`). Commit as the public identity in `.pii-redaction.json` (set in
  this repo's git config); never with a personal name or email.
- Every user decision or action goes in `docs/DECISIONS.md` (oldest first).
  User-facing changes update `docs/CHANGELOG.md` under Unreleased.
- Auxiliary docs live in `docs/`. The README follows REPO-STRUCTURE-STANDARD §5.
- **Never commit** anything under `docs/plan/`, raw captures (`/captures/`),
  unredacted EDIDs, secrets, passwords, IP addresses or MAC addresses. Only
  redacted fixtures go into `tests/fixtures/` (EDID serial bytes zeroed and the
  checksum fixed).
- Quality bar before any gate: `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, `cargo deny check`, and fuzz targets building for every parser.

## Where things are

| Path | Contents |
| --- | --- |
| `docs/plan/DisplayCaps-Development-Plan.md` / `.pdf` | Full plan: goals, architecture, probe sources, data model, link budget, rule catalogue, API, sandbox matrix, EDID decision, roadmap, testing (git-ignored) |
| `docs/plan/local-environment.md` | Machine, lab and test-rig details (git-ignored) |
| `docs/plan/claude-code-kickoff.md` | The kickoff prompt that started development (git-ignored) |
| `docs/DECISIONS.md` | Decision & action log |
| `docs/PLAN.md` | Roadmap checkboxes and open items (drives the progress page) |
| `../audiocaps` | AudioCaps, the source of the copied code (read-only) |
| `README.md` | Public-facing summary |
