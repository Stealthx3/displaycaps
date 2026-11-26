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
- **2026-11-21** · change · Lab guest disks and base volumes live in libvirt pool VMDisks, not the default pool — _why:_ keep lab disks off the system drive
- **2026-11-21** · decision · edid-decode is the primary oracle; facts only di-edid-decode reports are listed as oracle splits, not failures — _why:_ edid-decode covers more blocks; libdisplay-info omits several _(by claude)_
- **2026-11-21** · decision · GTF standard timings resolved as edid-decode does — _why:_ matches the primary oracle _(by claude)_
- **2026-11-21** · decision · HDMI Forum VRRmax 0 is resolved by the preferred resolution's highest refresh, marked inferred — _why:_ never overstate a VRR range the TV does not state _(by claude)_
- **2026-11-21** · decision · Phase 2 fuzz run: 2 hours per target on the final code (about 905M executions, 0 crashes) _(by claude)_
- **2026-11-21** · decision · Gate 2.7's HTPC part met by the TV from AudioCaps' existing HTPC capture (Hisense, HEC 41008); the receiver was never captured and is out of scope — _why:_ maintainer ruled out the HTPC _(by claude)_
- **2026-11-26** · approval · Approved the phase 2 gate (TV identified from the existing HTPC capture, receiver out of scope); merge to main and start phase 3
- **2026-11-26** · decision · KMS is read through the pure-Rust drm crate (MIT) instead of linking libdrm — _why:_ same ioctls, no unsafe code outside the FFI crate, no system library to bundle; the plan listed libdrm as a linked library, so this is raised at the phase 3 gate _(by claude)_
- **2026-11-26** · decision · The KMS layer is read only inside a Wayland or X11 session; outside one it is unknown with reason no_session — _why:_ opening a primary node with no DRM master makes the process master implicitly _(by claude)_
- **2026-11-26** · decision · Mode, HDR and VRR changes are noticed by a read-only KMS re-read every 500 ms while a watch includes the KMS layer (ProbeOptions::kms_poll_interval, can be turned off) — _why:_ the kernel announces them only to the compositor; compositor events come in phase 5 _(by claude)_
- **2026-11-26** · decision · KMS client capabilities universal planes, atomic and aspect ratio are set on the library's own file descriptor; stereo 3D and writeback stay off — _why:_ visibility only; matches drm_info's view for the gate _(by claude)_
- **2026-11-26** · change · KMS change poll is opt-in (ProbeOptions::kms_poll_interval defaults to None), replacing the 500 ms default logged earlier today — _why:_ gate review: polling reopens cards and keeps a dGPU awake; principle 2, the app decides _(by claude)_
- **2026-11-26** · decision · A card is opened only when the process is in a Wayland/X11 session, seat0's active logind session is graphical (when logind state is readable), and one of the card's connectors drives a CRTC; other cards are unknown with a reason — _why:_ gate review: a session variable alone doesn't prove a compositor holds DRM master of every card _(by claude)_
- **2026-11-26** · decision · GETCONNECTOR is issued directly through drm-ffi with a mode buffer of at least one entry on every call — _why:_ the drm crate's retry loop sends count_modes=0 (a probe request) for a connector with no cached modes _(by claude)_
- **2026-11-26** · change · Lab: lab-displaycaps-desktop's virtio GPU has two heads, the second left disconnected, so passive checks cover a connector with no cached modes _(by claude)_
- **2026-11-26** · approval · Approved the phase 3 gate, including the drm crate instead of linking libdrm; merge to main and start phase 4
- **2026-11-26** · decision · Phase 4 gate 4.3 is simulated: the HTPC TV's real EDID (HDMI 2.0) plus a simulated HDMI 2.1 sink (the same EDID with FRL 4x12 and VIC 118 added), on amdgpu with FRL off and on (test htpc_4k_fallbacks_explained, displaycaps link) — _why:_ the HTPC is out of scope; raised for agreement at the phase 4 gate _(by claude)_
- **2026-11-26** · decision · Inferred driver policy: requested format at decreasing depth, then DSC, then YCbCr 4:2:0 when the format is automatic; TMDS before FRL — _why:_ approximates amdgpu and i915; always labelled inferred _(by claude)_
- **2026-11-26** · decision · FRL budget approximation: 16b/18b coding, 3% overhead allowance, blanking at 8 bits per pixel period; a 3x6 4K60 10-bit case within its margin is not asserted — _why:_ the exact HDMI 2.1 capacity algorithm isn't implemented; known 40G/48G 4K120 cases hold _(by claude)_
- **2026-11-26** · decision · amdgpu HDMI FRL gate: unsupported before Linux 7.2, on in 7.2-7.3 only with dcfeaturemask bit 0x400, on by default from 7.4 — _why:_ Phoronix reports on Linux 7.2 and 7.4; the CachyOS case _(by claude)_
- **2026-11-26** · change · Link layer: the GPU's TMDS, FRL, DP and DSC limits are unknown on the live system (no per-ASIC table until phase 7); the sink's limits are used and the layer is partial; simulations give the GPU's limits explicitly — _why:_ gate review: a per-driver table overstated older GPUs (e.g. Polaris has no FRL) _(by claude)_
- **2026-11-26** · change · When a better link format is rejected only for lack of knowledge (FRL state, a limit), the link format is unknown with both candidates named, not a guessed fallback — _why:_ gate review; unknown with a reason, never a guess _(by claude)_
- **2026-11-26** · change · DisplayPort link state is unknown (the link rate needs the DPCD); adapters are taken from the KMS subconnector property, the EDID heuristic only without it and then partial — _why:_ gate review: false adapters on DP monitors with an HDMI VSDB (seen in the workstation fixture) _(by claude)_
- **2026-11-26** · change · Driver policy: amdgpu turns DSC on before lowering depth, i915 lowers depth first (down to 6 bits on DP); HDMI 4:2:2 uses a 24-bit container; FRL blanking cost 4 bits per pixel period (calibrated: 4K60 10-bit RGB on 3x6) — _why:_ gate review _(by claude)_
- **2026-11-26** · action · Phase 4 fuzz run on the final code: 10 targets, 30 min each, about 363M executions, 0 crashes _(by claude)_
- **2026-11-26** · approval · Approved the phase 4 gate, including the simulated HTPC test for 4.3; merge to main and start phase 5
