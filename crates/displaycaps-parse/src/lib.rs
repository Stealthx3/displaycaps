// SPDX-License-Identifier: LGPL-2.1-or-later
// Adapted from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-parse/src/lib.rs.
//! DisplayCaps parsers: pure functions over bytes and text. No I/O, so every parser can be fuzzed
//! directly (see `fuzz/`).
//!
//! - [`edid`] — EDID base block and CTA-861 extensions; serial redaction for captures; the serial
//!   that feeds the sink key. Video blocks are added in phase 2.
//! - [`timing`] — detailed timings, base-block timings and descriptors, DMT, CVT and GTF.
//! - [`vic`] — CTA-861 VIC and HDMI VIC timing tables.
//! - [`cta_video`] — CTA-861 video, colour, HDR, HDMI 2.1 and vendor video blocks.
//! - [`displayid`] — DisplayID 1.3 and 2.x.
//! - [`sad`] — CTA-861 Short Audio Descriptors (kept so the combined library sees the same audio
//!   fields as AudioCaps).
//! - [`ids`] — PNP manufacturer and CEC vendor name tables.
//! - [`os_release`] — `/etc/os-release`.
//!
//! Parsers never panic on any input; malformed input yields an error, `None`, or a partial result
//! with a `truncated` flag.

pub mod cta_video;
pub mod displayid;
pub mod edid;
pub mod ids;
pub mod os_release;
pub mod sad;
pub mod timing;
pub mod vic;
