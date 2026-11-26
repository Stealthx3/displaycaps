// SPDX-License-Identifier: LGPL-2.1-or-later
//! DisplayCaps link-budget engine: pure maths over modes and link limits (TMDS, FRL, DisplayPort,
//! DSC, chroma and bit depth). No I/O.
//!
//! - [`budget`]: data rates a format needs and links carry.
//! - [`choose`]: the format a driver ends up sending for a mode, given what the source, the sink
//!   and KMS allow, with every better option that was rejected and why. KMS reports what was
//!   requested, not what was negotiated, so the result is always *inferred*.
//!
//! The FRL budget is an approximation of the HDMI 2.1 capacity computation (16b/18b coding, a
//! fixed allowance for RS-FEC and packet overhead, blanking at a reduced cost); results near the
//! boundary carry little headroom and say so.

pub mod budget;
mod choose;

pub use choose::{Choice, FrlGate, LinkKind, Rejected, Request, SinkFormats, SourceLimits, choose};
