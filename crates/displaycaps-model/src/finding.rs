// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-model/src/finding.rs; access paths replaced by presentation paths; hints also keyed by compositor.
//! Diagnostic findings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::endpoint::EndpointId;
use crate::path::{HopKind, PresentationPath};
use crate::value::{Confidence, Provenance};

/// Stable finding code, e.g. `HDR_AVAILABLE_NOT_ENABLED`. Codes come from rule packs, so this is a
/// string rather than a closed enum; codes are never reused for a different meaning.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct FindingCode(pub String);

/// Severity of a finding.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Informational.
    Info,
    /// Quality or features are lost.
    Warning,
    /// The path doesn't work as the app expects.
    Error,
}

/// A diagnostic finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Finding {
    /// Stable code.
    pub code: FindingCode,
    /// Severity.
    pub severity: Severity,
    /// The endpoint it concerns, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<EndpointId>,
    /// The presentation path it concerns, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_path: Option<PresentationPath>,
    /// The hop where it happens.
    pub hop: HopKind,
    /// Evidence values.
    pub evidence: Vec<Evidence>,
    /// How sure the finding is.
    pub confidence: Confidence,
    /// Remediation hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<Hint>,
}

/// One evidence value behind a finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Evidence {
    /// What the value is, e.g. `sink.hdr.eotfs`.
    pub label: String,
    /// The value as JSON.
    pub value: serde_json::Value,
    /// Where it came from.
    pub provenance: Provenance,
}

/// A remediation hint, keyed by distro family and, for display findings, by compositor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Hint {
    /// What to do.
    pub text: String,
    /// The distro family the hint was written for; `None` for the generic fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distro_family: Option<crate::snapshot::DistroFamily>,
    /// The compositor the hint was written for; `None` when it applies to any compositor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compositor: Option<crate::snapshot::CompositorKind>,
}
