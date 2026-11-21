# DisplayCaps — Roadmap to Linux 1.0

Public task list for the nine gated phases. The full design lives in the development plan (kept
outside git); this file only tracks progress. Each phase ends at a gate that the maintainer reviews before the
next phase starts. Ticks land in the same commit as the work.

Open items (not phase tasks):

- The HTPC is out of scope (lab-simulated tests only). Gates that name it (2.7 TV and receiver, 4.3 HTPC 4K fallbacks) need a simulated equivalent, agreed at those gates.

- Plan the combined library: name, repo, API shape, and what it adds beyond the two.
- Decide whether DisplayCaps' video EDID additions are back-ported to AudioCaps in a later release.
- Find an HDMI 2.1 FRL test sink, or accept fixture-only FRL coverage.
- Final library name, after a collision check, before 1.0.

## Phase 1 — Scaffold and copied foundation (about 2 weeks)

- [x] **1.1** Adopt the shared standards: repo layout, progress page, phone alerts, VM lab, usage plan, journal
- [x] **1.2** Cargo workspace with seven crates (model, parse, link, backends, rules, ffi, cli), pinned toolchain, rustfmt/clippy, SPDX headers, `deny.toml`
- [x] **1.3** Copy AudioCaps v1.0.0's generic code with provenance headers, renamed for DisplayCaps, with its tests
- [x] **1.4** Data model: display endpoints, presentation paths, layers, link state, path reports, findings; JSON schema v1
- [x] **1.5** Capture tool `displaycaps capture`: passive, read-only, EDID serials zeroed
- [x] **1.6** Captures: lab VMs (virtio-gpu with KWin, GNOME and Sway), workstation (HTPC skipped: lab-simulated tests only)
- [x] **1.7** EDID corpus and the lab oracle harness (edid-decode, di-edid-decode)
- [x] **1.8** Gate: copied code passes its tests; sink-key parity with AudioCaps; fmt, clippy, tests and cargo-deny clean; history clean

## Phase 2 — EDID video parser and sink (about 4 weeks)

- [x] **2.1** Timings: detailed, standard and established timings, range limits, CVT/GTF, VICs, 4:2:0 maps, HDMI VICs
- [x] **2.2** Colour and HDR: video capability, colorimetry, HDR static and dynamic metadata, Dolby Vision, HDR10+
- [x] **2.3** HDMI 2.1: full HDMI Forum VSDB and Sink Capability block, DSC, VRR, ALLM
- [x] **2.4** DisplayID 1.3 and 2.x
- [x] **2.5** Sink record and quirk packs
- [x] **2.6** Differential tests against both oracles over the corpus; fuzz targets for every block parser
- [ ] **2.7** Gate: corpus agrees with the oracles or every difference is triaged; the HTPC's TV identified from its existing capture (receiver: HTPC out of scope, see open items); fuzz runs clean

## Phase 3 — sysfs and KMS backend (about 3 weeks)

- [ ] **3.1** sysfs DRM reader: connectors, status, cached EDID
- [ ] **3.2** Read-only KMS: active mode, max bpc, Colorspace, color format, HDR metadata, Broadcast RGB, VRR, link status
- [ ] **3.3** Hotplug and mode-change events
- [ ] **3.4** Gate: matches drm_info; strace shows no modeset, DRM master or forced probe

## Phase 4 — Link budget engine (about 2 weeks)

- [ ] **4.1** TMDS, FRL, DP and DSC budget maths with table-driven tests
- [ ] **4.2** Inferred link state: chroma, bit depth, headroom, adapters and docks
- [ ] **4.3** Gate: HTPC 4K fallbacks explained, with FRL off and on

## Phase 5 — Compositors and X11 (about 4 weeks)

- [ ] **5.1** Wayland core outputs and color-management-v1 output image descriptions
- [ ] **5.2** wlr-output-management and KDE output device
- [ ] **5.3** Mutter DisplayConfig over D-Bus
- [ ] **5.4** X11 RandR fallback
- [ ] **5.5** Gate: matches KWin, GNOME and Sway settings in the lab

## Phase 6 — Decode layer, opt-in (about 2 weeks)

- [ ] **6.1** VA-API profiles, loaded lazily
- [ ] **6.2** Vulkan Video profiles, loaded lazily
- [ ] **6.3** Gate: matches vainfo and vulkaninfo; sandbox driver gap flagged

## Phase 7 — Path reports and rules (about 3 weeks)

- [ ] **7.1** Effective sets per presentation path
- [ ] **7.2** Path reports, content queries and best-mode recommendations
- [ ] **7.3** Declarative starter rule pack
- [ ] **7.4** Gate: each starter rule fires only when it should

## Phase 8 — DDC/CI and CEC reads (about 1 week)

- [ ] **8.1** DDC/CI VCP reads, never writes (opt-in)
- [ ] **8.2** CEC reads (opt-in)
- [ ] **8.3** Gate: opt-ins run only when the app asks

## Phase 9 — C ABI, docs, packaging (about 2 weeks)

- [ ] **9.1** C ABI with cbindgen header (`dc_` prefix), events by callback or pollable fd
- [ ] **9.2** Consumer docs: C, C++, Python, .NET, mpv mapping, SDL3 mapping
- [ ] **9.3** Flatpak and Snap permission templates
- [ ] **9.4** Gate: doc examples pass; sandbox release gate passes
