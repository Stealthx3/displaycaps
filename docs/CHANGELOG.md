# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Repository scaffold: README, license and notice, documentation index,
  roadmap, decision log, CLI mockup, and Claude Code hand-over instructions.
- Shared standards adopted (repo layout, progress page, phone alerts, VM lab,
  usage plan, post-mortem journal, PII redaction); public progress page.
- Cargo workspace with seven crates and the display data model with JSON
  schema v1 (`schema/displaycaps-snapshot-v1.schema.json`).
- Generic code copied from AudioCaps v1.0.0: SystemRoot, runtime, watch,
  environment and sandbox, rule and quirk engine, EDID parser, CLI and the
  `dc_` C ABI pattern (`include/displaycaps.h`, `libdisplaycaps.so.1`).
- Sink-key parity with AudioCaps v1.0.0 on shared EDID vectors.
- `displaycaps capture`: passive, read-only capture of DRM, driver, KMS,
  compositor and decode state with EDID serials zeroed; strace passive check.
- Golden-snapshot fixtures from GNOME, Plasma and Sway lab sessions and the
  workstation; EDID corpus; lab-only edid-decode and di-edid-decode oracles.
