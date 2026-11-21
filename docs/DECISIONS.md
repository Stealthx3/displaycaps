# Decision & action log

User decisions and actions on this project, oldest first. Maintained by Claude per REPO-STRUCTURE-STANDARD;
once phase 1 adds the shared scripts, add entries with `node scripts/repo-check.mjs log <kind> "<text>" --why "<reason>"`.
Open items live in `docs/PLAN.md`.

- **2026-11-09** · action · Wrote the development plan as a sister library to AudioCaps (Claude Docs "DisplayCaps — Development Plan"), including an evaluation of forking libdisplay-info
- **2026-11-09** · decision · Carried over from AudioCaps: Rust core with a generated C header; LGPL-2.1-or-later; Linux first, Windows and macOS later; Flatpak and Snap compatibility required; information on-demand; the application decides; few languages, bundled use; Claude builds in libvirt VMs and the maintainer reviews each phase gate
- **2026-11-14** · action · Exported the development plan to PDF for review
- **2026-11-16** · decision · Sister library in its own repo, built on the AudioCaps design
- **2026-11-16** · decision · Code reuse: copy AudioCaps v1.0.0's generic code (core types, SystemRoot, capture, redaction, runtime, rule and quirk engine, sink identity and key) and keep it separate; no shared crate — _why:_ each library stays independently releasable
- **2026-11-16** · decision · Library family: a third repo will combine AudioCaps and DisplayCaps, so consumers pick AudioCaps, DisplayCaps or the combined library to suit their project
- **2026-11-16** · decision · EDID parsing: copy AudioCaps' Rust EDID parser and extend it with every video block; libdisplay-info is not linked or forked; edid-decode and libdisplay-info's di-edid-decode serve only as lab test oracles — _why:_ Rust only, no C dependency for parsing
- **2026-11-16** · decision · Phase gates: Claude Code stops at every gate and waits for approval (the AudioCaps "continue phase to phase" instruction does not apply here)
- **2026-11-16** · action · Updated the development plan for these decisions and for the AudioCaps 1.0.0 release; exported PDF and Markdown copies to `docs/plan/` (git-ignored)
- **2026-11-16** · action · Prepared the repository scaffold and the hand-over to Claude Code: README, LICENSE, NOTICE, changelog, decision log, roadmap, CLI mockup, `CLAUDE.md`, plus git-ignored `docs/plan/local-environment.md` and `docs/plan/claude-code-kickoff.md`
- **2026-11-21** · approval · One-time host exception: run displaycaps capture once on the workstation under scripts/check-passive.sh (phase 1.6)
- **2026-11-21** · approval · Create Stealthx3/displaycaps as a public GitHub repo and publish the progress page with GitHub Pages, like AudioCaps
- **2026-11-21** · approval · Install cbindgen 0.29.2 in the lab guest only (build tool, never linked) to generate include/displaycaps.h
- **2026-11-21** · action · Added PII-REDACTION-STANDARD to the project — _why:_ nothing in the project may identify the maintainer or the test environment
- **2026-11-21** · decision · Public identity is the GitHub handle Stealthx3 with its noreply address; this repo commits under it — _why:_ replaces the machine's git identity from CLAUDE.md
- **2026-11-21** · decision · Replace the maintainer's real name everywhere, including the NOTICE copyright (now Stealthx3), and absolute drive paths in CLAUDE.md
- **2026-11-21** · decision · Rewrite the local, never-pushed history to remove personal values before the first push
- **2026-11-21** · note · Lab host details stay out of docs/LAB.md; real values only in git-ignored docs/LAB.local.md
- **2026-11-21** · action · Workstation capture run once on the host under check-passive.sh (approved exception): passed; drm_info not installed there, so no KMS dump
- **2026-11-21** · action · Created the public GitHub repo Stealthx3/displaycaps and published the progress page with GitHub Pages (also deploys from feat/phase-* branches)
- **2026-11-21** · note · Repo check ignores tests/fixtures/ for folder-name case — _why:_ fixture trees mirror real sysfs names such as card1-HDMI-A-1 _(by claude)_
- **2026-11-21** · action · Rewrote the pushed phase-1 branch history to replace the workstation GPU device UUIDs in a fixture, and force-pushed — _why:_ a per-board hardware id leaked into commit 24c5fcd
- **2026-11-21** · note · GitHub still serves the old commit by its SHA until its own garbage collection; a purge needs a GitHub Support request
- **2026-11-21** · decision · Skip the HTPC: no capture or test on the production rig; use simulated tests in the lab only — _why:_ maintainer's instruction at the phase 1 gate
- **2026-11-21** · approval · Approved the phase 1 gate; merge to main and start phase 2
