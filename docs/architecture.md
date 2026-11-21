# Architecture

Crate boundaries as built. The design is in the development plan; this file tracks what exists.

| Crate | Role | Depends on | State |
| --- | --- | --- | --- |
| `displaycaps-model` | OS-neutral types and the versioned JSON schema: `Value<T>` with provenance and confidence, `ProbeScope`, display endpoints, sink record and identity (with the sink-key algorithm shared with AudioCaps), per-layer capability sets and state, presentation paths, effective sets, path reports, content requests, recommendations, findings, redaction | serde, schemars, sha2 | Schema v1 (phase 1) |
| `displaycaps-parse` | Pure parsers over bytes and text, no I/O: EDID base block and CTA-861 (copied from AudioCaps; video blocks in phase 2), SADs, PNP ids, os-release | — | Copied foundation (phase 1) |
| `displaycaps-link` | Pure link-budget maths: TMDS, FRL, DP, DSC, chroma and bit depth | model | Empty until phase 4 |
| `displaycaps-backends` | System access through `SystemRoot` (live `/` or a capture tree): runtime with per-backend deadlines, watch, environment and sandbox; sysfs DRM, KMS, Wayland, Mutter, RandR, VA-API and Vulkan backends arrive in phases 3–6 | model, parse | Copied foundation (phase 1) |
| `displaycaps-rules` | Declarative rule and quirk packs, bundled and overridable from `/etc/displaycaps/` or the app bundle | model | Engine copied (phase 1); starter rules in phase 7 |
| `displaycaps-ffi` | The C ABI (`dc_*`, `libdisplaycaps.so.1`, `include/displaycaps.h` from cbindgen) | backends, rules | Pattern copied (phase 1); full ABI in phase 9 |
| `displaycaps-cli` | `displaycaps` tool: probe, schema, passive capture | backends, rules | Phase 1 |

## Rules that shape the code

- **Scoped and passive.** Every probe takes a `ProbeScope`; nothing outside it is read. Readers
  never set KMS state, never take DRM master, never force a connector probe, never touch I2C or
  CEC unless the app opted in. `scripts/check-passive.sh` enforces this under strace.
- **Unknown, never unsupported.** A layer that can't be read (sandbox, missing library, no
  session) is `unknown` with a reason.
- **Kept apart from AudioCaps.** Symbols `dc_*`, schema ids `displaycaps-*`, data paths
  `/usr/share/displaycaps/` and `/etc/displaycaps/`. The sink key is the one shared value;
  `crates/displaycaps-parse/tests/parity.rs` checks it against AudioCaps v1.0.0's own output.
- **Copied code names its source.** Each file copied from AudioCaps carries
  `Copied from AudioCaps v1.0.0 (commit 049d053) <path>` (or `Adapted from …`); `scripts/gate.sh`
  checks it.
