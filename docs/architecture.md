# Architecture

Crate boundaries as built. The design is in the development plan; this file tracks what exists.

| Crate | Role | Depends on | State |
| --- | --- | --- | --- |
| `displaycaps-model` | OS-neutral types and the versioned JSON schema: `Value<T>` with provenance and confidence, `ProbeScope`, display endpoints, sink record and identity (with the sink-key algorithm shared with AudioCaps), per-layer capability sets and state, presentation paths, effective sets, path reports, content requests, recommendations, findings, redaction | serde, schemars, sha2 | Schema v1 (phase 1) |
| `displaycaps-parse` | Pure parsers over bytes and text, no I/O: EDID base block (`timing`, with `dmt`, `cvt`, `vic` tables), CTA-861 audio (copied from AudioCaps) and video (`cta_video`), DisplayID (`displayid`), SADs, PNP ids, os-release | — | EDID video parser (phase 2), checked against edid-decode and di-edid-decode by `scripts/edid-diff.sh` |
| `displaycaps-link` | Pure link-budget maths: TMDS, FRL, DP, DSC, chroma and bit depth | model | Empty until phase 4 |
| `displaycaps-backends` | System access through `SystemRoot` (live `/` or a capture tree): runtime with per-backend deadlines, watch (manual, timer, DRM hotplug uevents, KMS change poll), environment and sandbox; `sink` builds the sink record from an EDID with quirk data; `drm` reads sysfs DRM (connectors, cached EDID, GPU, driver) and KMS through the `drm` crate's read-only ioctls (or a fixture's `drm_info -j`), and joins them into endpoints; Wayland, Mutter, RandR, VA-API and Vulkan backends arrive in phases 5–6 | model, parse, rules, drm, nix | Copied foundation (phase 1), sink record (phase 2), sysfs and KMS (phase 3) |
| `displaycaps-rules` | Declarative rule and quirk packs, bundled and overridable from `/etc/displaycaps/` or the app bundle | model | Engine copied (phase 1); starter rules in phase 7 |
| `displaycaps-ffi` | The C ABI (`dc_*`, `libdisplaycaps.so.1`, `include/displaycaps.h` from cbindgen) | backends, rules | Pattern copied (phase 1); full ABI in phase 9 |
| `displaycaps-cli` | `displaycaps` tool: probe, schema, passive capture | backends, rules | Phase 1 |

## Rules that shape the code

- **Scoped and passive.** Every probe takes a `ProbeScope`; nothing outside it is read. Readers
  never set KMS state, never take DRM master, never force a connector probe, never touch I2C or
  CEC unless the app opted in. `scripts/check-passive.sh` enforces this under strace.
- **KMS only where a compositor holds master.** Opening `/dev/dri/card*` when no process is DRM
  master would make this one master implicitly, so a card is opened only inside a Wayland or X11
  session whose logind session is seat0's active one (when logind's state is readable), and only
  if one of its connectors drives a CRTC. Otherwise the KMS layer is `unknown` with the reason.
  Only the connectors in scope (and the CRTCs driving them) are read; the change poll is opt-in. Connectors are read with `GETCONNECTOR` and a one-mode stub
  (the kernel's cached state, as `drmModeGetConnectorCurrent` does), issued directly through `drm-ffi` so `count_modes` is never 0. No libdrm is linked: the
  pure-Rust `drm` crate makes the same ioctls without `unsafe` in this workspace.
- **Unknown, never unsupported.** A layer that can't be read (sandbox, missing library, no
  session) is `unknown` with a reason.
- **Kept apart from AudioCaps.** Symbols `dc_*`, schema ids `displaycaps-*`, data paths
  `/usr/share/displaycaps/` and `/etc/displaycaps/`. The sink key is the one shared value;
  `crates/displaycaps-parse/tests/parity.rs` checks it against AudioCaps v1.0.0's own output.
- **Copied code names its source.** Each file copied from AudioCaps carries
  `Copied from AudioCaps v1.0.0 (commit 049d053) <path>` (or `Adapted from …`); `scripts/gate.sh`
  checks it.
