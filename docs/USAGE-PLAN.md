# Usage Plan — DisplayCaps
_Standard: TOKEN-USAGE-OPTIMIZATION-STANDARD v1 · Created 2026-11-21 · Modelled on AudioCaps' plan_

## Routing (A)
| Task type | Tier | Why |
|---|---|---|
| Data model, schema v1, API shape, crate boundaries (task 1.4) | opus (highest tier in this plan) | every later phase builds on it; expensive to reverse |
| Link-budget maths design (TMDS/FRL/DP/DSC), path-report algebra, rule engine changes | opus | cross-layer reasoning, silent-loss bugs are subtle |
| Copying AudioCaps v1.0.0 code, renames, provenance headers | sonnet | mechanical but must stay faithful; tests catch drift |
| EDID/CTA/DisplayID block parsers | sonnet | spec-driven, oracle- and fuzz-checked; escalate oracle disagreements to opus |
| Backends (sysfs, KMS, Wayland, Mutter, RandR, VA-API, Vulkan) | sonnet | known APIs; escalate passivity or concurrency bugs to opus |
| Rule pack entries, quirk data, fixtures | sonnet | mechanical once the engine exists |
| Docs, changelog, decision-log entries | sonnet | routine writing |
| Lab build/test runs, oracle runs, fuzz triage | haiku | report failures only |
| Codebase inventory, dependency/license audit | haiku | conclusion only |
| Gate review before each phase hand-off | opus | high-stakes, unanchored review |

The main thread runs on whatever model the maintainer picks; work tiered below it is delegated to the agents
below when it is large enough to be worth a brief.

## Agents (B)
| Agent | Model | Tools | Used for |
|---|---|---|---|
| scout | haiku | Read, Grep, Glob | locate code, inventory fixtures, audit deps |
| runner | haiku | Bash | `scripts/lab.sh run` builds/tests; returns failures only |
| reviewer | opus | Read, Grep, Glob, Bash | gate review of a phase diff against its gate |

Defined in `.claude/agents/`. Small lookups stay in the main thread.

## Tools (C)
| Tool | Path | Scope | Replaces |
|---|---|---|---|
| progress | scripts/progress.mjs | shared (verbatim) | hand-written progress page |
| repo-check | scripts/repo-check.mjs | shared (verbatim) | manual repo audits, README progress block, log entries |
| lab | scripts/lab.sh | shared (verbatim, master v1.2; AudioCaps' local `put` addition not taken) | raw virsh/ssh for builds and tests |
| phone-alerts | scripts/phone-alerts.mjs | shared (verbatim) | manual notification setup |
| postmortem | scripts/postmortem.mjs | shared (verbatim) | hand-assembled post-mortem |
| gate | scripts/gate.sh | project (from AudioCaps) | fmt + clippy + test + deny + fuzz build + SPDX + provenance in one lab call |
| scan-history | scripts/scan-history.sh | project (from AudioCaps) | checking git history for IPs, MACs, plan docs, captures, unredacted EDIDs |
| lab-sync-back | scripts/lab-sync-back.sh | project (from AudioCaps) | copying generated files (lockfile, schema, fmt output, goldens) back from the guest |
| oracles | scripts/oracles.sh | project | building and running edid-decode / di-edid-decode in the lab |
| parity | scripts/parity-audiocaps.sh | project | regenerating AudioCaps' sink keys for the shared EDID vectors |
Candidates to watch: capture diffing, fixture redaction check, oracle-diff summariser (phase 2).

## Pacing (D)
| Session | Covers | Size | Checkpoint |
|---|---|---|---|
| P1-S1 | Git, standards, lab VM, workspace, copied code, model + schema, capture, oracles | L | phase 1 gate summary |
| P2 | EDID video parser (split by block family if long) | L ×2 | 2.3 and 2.7 |
| P3–P9 | One or two sessions per phase | M | each gate |
Never two L sessions back to back where limits are tight; Phase 2 is the largest step.

## Context (E)
- Never read by default: `target/`, `Cargo.lock`, `schema/*.json` (generated), `tests/fixtures/**`, `docs/progress/`, `docs/plan/*.pdf` (diagrams only), the AudioCaps repo outside its `v1.0.0` export in the scratchpad
- On-demand docs: `docs/plan/*`, `docs/standards/*`, `docs/architecture.md`, `docs/LAB.md`
- CLAUDE.md line budget: ~130 (project rules are dense; standards are referenced, not inlined)

## Log
- 2026-11-21 — Plan created from AudioCaps' plan. Task 1.4 (data model and schema v1) runs in the main thread on the top tier.
- 2026-11-21 — Copying the backends/rules/cli/ffi crates was delegated to one sonnet agent (≈290k tokens, 11 min); it worked from a scratchpad export of AudioCaps v1.0.0 and its own lab directory.
- 2026-11-21 — Gate review by the opus reviewer agent (≈100k tokens) found 11 issues, including two the gate scripts couldn't see (passive checker blind to hex paths; device UUID in a fixture). Keep it for every gate.
- 2026-11-21 — Phase 2: three sonnet implementer agents in parallel on disjoint parser files (timing/tables ≈234k tokens, CTA video ≈132k, DisplayID ≈187k), against a layout set in the main thread; integration, sink mapping and the differential harness in the main thread. The differential test found 3 real bugs the agents' own oracle checks missed (VIC 39, CTA-in-DisplayID, VIC 0) and 4 extractor gaps.
- 2026-11-26 — Phase 3 in the main thread (sysfs, KMS, watch, lab checks); one opus gate review (≈84k tokens) found 3 passivity risks the strace check couldn't see (session ≠ DRM master per card, a default KMS poll, a `count_modes = 0` path in the drm crate) and 8 smaller issues; all fixed before the gate. New tool: kms-diff.sh.
- 2026-11-26 — Phase 4 in the main thread (link maths and policy are top-tier work); one opus gate review (≈68k tokens) found 4 high findings (overconfident GPU limits, nvidia-drm name, unknowns turned into guesses, made-up DP results) and 9 smaller; all fixed. New tool: `displaycaps link`.
- 2026-11-26 — Phase 5 in the main thread; one gate review (≈93k tokens) found 3 high findings (wrong joins with 0-based X names, duplicate endpoints, serials in captured descriptions) and 14 smaller; all fixed or logged. New tools: compositor-facts, compositor-diff/-gate/-passive/-watch-check, compositor-oracle.py; the SSH agent refused twice mid-run (maintainer unlocked it).
- 2026-11-21 — New tools: oracles.sh, parity-vectors.mjs, parity-audiocaps.sh, check-passive.sh (DRM rules), desktop-session.sh.

## Handoff
Phase 5 at its gate on `feat/phase-5-compositors`: compositor layer (Wayland, KDE output device, wlr-output-management, color-management-v1, Mutter DisplayConfig, X11 RandR), compositor watch events, lab comparison with KWin, GNOME, Sway and Plasma on Xorg; waiting for the maintainer. Next: phase 6 (decode layer, opt-in). Lab: main running the fuzz run; desktop running.
