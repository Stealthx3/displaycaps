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
- EDID video parser: detailed, standard, established and CVT timings with the
  DMT, CVT, GTF and VIC tables; range limits; CTA-861 video capability,
  colorimetry, HDR static and dynamic metadata, Dolby Vision, HDR10+, 4:2:0
  maps, the full HDMI Forum VSDB and SCDB (FRL, DSC, VRR, ALLM) and FreeSync;
  DisplayID 1.3 and 2.x, including CTA blocks inside DisplayID.
- Sink record from the EDID with quirk data (`displaycaps sink <edid-file>`).
- Differential tests against edid-decode and di-edid-decode over 171 EDIDs,
  and fuzz targets for every block parser.
- Endpoints from sysfs DRM: connectors with their cached status, the sink
  record from the cached EDID, GPU and PCI ids, driver and display-related
  module parameters, and correlation keys (DRM connector, KMS connector and
  CRTC, sink key).
- Read-only KMS layer: active mode, the cached mode list, `max bpc`,
  `Colorspace`, `color format`, `HDR_OUTPUT_METADATA`, `Broadcast RGB`,
  `vrr_capable`, `VRR_ENABLED`, `content type`, `link-status` and panel
  orientation. Read only where a compositor holds DRM master (never as master);
  fixtures replay captured `drm_info -j` output.
- Watch events for DRM hotplug (kernel uevents) and, opt-in, KMS changes
  (mode, HDR, VRR) with `ProbeOptions::kms_poll_interval`.
- `displaycaps watch` and `displaycaps kms-facts` (lab tooling).
- Link-budget engine (`displaycaps-link`): TMDS, FRL, DisplayPort and DSC
  rates, and the format a driver sends for a mode (depth reduction, then DSC,
  then YCbCr 4:2:0) with every better format it rejected and why.
- Inferred link layer per endpoint: signal (TMDS, FRL, DP, virtual), colour
  format, bit depth, required rate, capacity and rejected formats, from the
  sink EDID, KMS and the driver's limits and HDMI FRL gate (amdgpu by kernel
  version and `dcfeaturemask`); DP-to-HDMI adapters and MST hubs reported as
  devices in the sink chain.
- `displaycaps link`: simulate a link from an EDID, mode and driver.
