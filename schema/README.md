<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
<!-- Adapted from AudioCaps v1.0.0 (commit 049d053) schema/README.md. -->
# DisplayCaps JSON schema

`displaycaps-snapshot-v1.schema.json` describes the JSON export of a snapshot (JSON Schema
2020-12). It is generated from the Rust model (`crates/displaycaps-model`) and checked by a test, so
it can't drift from the code. Print it with `displaycaps schema`. Its `$id` and extension keys
(`x-displaycaps-*`) are distinct from AudioCaps' so both libraries can live in one process.

## Versioning

- The file name carries the **major** version; `x-displaycaps-schema-version` and each snapshot's
  `schema_version` carry the full version (`1.0.0`).
- Within v1, changes are **additive only**: new optional fields, new enum variants, new finding
  codes. Consumers must ignore unknown fields and treat unknown enum values as "other".
- Removing or renaming a field, or changing its meaning, needs v2 (a new file next to this one).
- Regenerate after a model change: `UPDATE_EXPECT=1 cargo test -p displaycaps-model` in the lab VM.

## Units and conventions

- Refresh and frame rates are in **millihertz** (`23976` is 23.976 Hz), so film rates stay exact.
- Pixel clocks and TMDS character rates are in kHz; link data rates in kbit/s; luminance in nits.
- A field group outside the probe scope is **omitted**; a field in scope that couldn't be read is
  `{"state": "unknown", "reason": ..., "detail": ...}`. Nothing is ever reported as "unsupported"
  because it couldn't be read.
- Link state is always `confidence: inferred`: KMS reports what was requested, not what the link
  negotiated.

## Redaction rules

Sensitive strings are `Sensitive` objects: `{"kind": "...", "clear": "..."}` or
`{"kind": "...", "redacted": "..."}`. Exports are redacted unless the application opts out.

| Kind | Default | Exported as |
|---|---|---|
| `serial` | hash | `sha256:` + 16 hex digits (stable, so sink keys still match) |
| `device_name` | mask | `redacted` (only user-chosen names) |

Each snapshot records the policy used in `redaction`. Captures made by `displaycaps capture` are
redacted further before they reach disk: EDID serial fields are zeroed (serial string descriptors
become `REDACTED`) and the checksum fixed, and MAC and IP addresses, the host and user name and
JSON values under keys containing `serial` are replaced.
