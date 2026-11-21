// SPDX-License-Identifier: LGPL-2.1-or-later
// Adapted from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-model/src/lib.rs.
//! DisplayCaps data model.
//!
//! OS-neutral types for everything DisplayCaps reports: the probe [`ProbeScope`], display
//! endpoints and their per-layer capability sets and state, effective sets per presentation path,
//! path reports with hops, content requests and recommendations, findings, provenance and
//! confidence, and explicit unknowns. The JSON schema (`schema/`) is generated from these types;
//! see [`schema::snapshot_schema`].
//!
//! Nothing here assumes Linux: Linux-specific identifiers appear only as data inside
//! [`endpoint::CorrelationId`] variants and [`value::Source`] locators.

pub mod caps;
pub mod detail;
pub mod endpoint;
pub mod finding;
pub mod path;
pub mod redact;
pub mod schema;
pub mod scope;
pub mod snapshot;
pub mod value;

pub use caps::*;
pub use detail::*;
pub use endpoint::*;
pub use finding::*;
pub use path::*;
pub use redact::{RedactionMode, RedactionPolicy, Sensitive, SensitiveKind, SensitiveValue};
pub use scope::*;
pub use snapshot::*;
pub use value::*;

/// Version of the JSON schema this crate produces. Within major version 1, changes are additive
/// only (new optional fields, new enum variants); see `schema/README.md`.
pub const SCHEMA_VERSION: &str = "1.0.0";
