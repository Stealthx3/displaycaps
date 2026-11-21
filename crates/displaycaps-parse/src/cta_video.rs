// SPDX-License-Identifier: LGPL-2.1-or-later
//! CTA-861 video data blocks: video data blocks (SVDs), YCbCr 4:2:0 video and capability map
//! blocks, video capability, colorimetry, HDR static and dynamic metadata, the HDMI VSDB's video
//! part (deep colour, HDMI VICs), the full HDMI Forum VSDB and Sink Capability Data Structure,
//! Dolby Vision and HDR10+ vendor-specific video blocks, the AMD FreeSync VSDB, and the detailed
//! timings in the block. Filled in phase 2 (tasks 2.1–2.3).

use crate::timing::DetailedTiming;

/// The video content of one CTA-861 extension block.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CtaVideo {
    /// Detailed timings after the data block collection.
    pub detailed: Vec<DetailedTiming>,
}

/// Decodes the video content of one 128-byte CTA-861 extension block.
pub fn parse_cta_video(blk: &[u8]) -> CtaVideo {
    let mut v = CtaVideo::default();
    let d = usize::from(*blk.get(2).unwrap_or(&0));
    if d >= 4 {
        let mut off = d;
        while off + 18 <= blk.len().min(127) {
            match crate::timing::parse_dtd(&blk[off..off + 18]) {
                Some(t) => v.detailed.push(t),
                None => break,
            }
            off += 18;
        }
    }
    v
}
