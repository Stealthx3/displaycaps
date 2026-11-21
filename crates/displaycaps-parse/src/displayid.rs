// SPDX-License-Identifier: LGPL-2.1-or-later
//! DisplayID 1.3 and 2.x sections inside EDID extension blocks (tag 0x70): timing types, tiled
//! topology, display parameters, product identification, adaptive sync and interface features.
//! Filled in phase 2 (task 2.4).

/// One DisplayID section (one EDID extension block with tag 0x70).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayId {
    /// DisplayID version byte, e.g. `0x13` or `0x20`.
    pub version: u8,
}

/// Decodes a DisplayID EDID extension block (tag 0x70).
pub fn parse_displayid(blk: &[u8]) -> DisplayId {
    DisplayId {
        version: blk.get(1).copied().unwrap_or(0),
    }
}
