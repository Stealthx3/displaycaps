// SPDX-License-Identifier: LGPL-2.1-or-later
//! CTA-861 video blocks: arbitrary bytes as one extension block, and the helpers that interpret
//! the decoded blocks.
#![no_main]

use displaycaps_parse::cta_video::parse_cta_video;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let v = parse_cta_video(data);
    let _ = v.y420_capable_vics();
    if let Some(h) = v.hdmi_forum() {
        let _ = (h.max_frl.lanes_and_gbps(), h.max_frl.total_gbps(), h.dsc_max_slices());
    }
    if let Some(h) = &v.hdr_static {
        let _ = (h.max_luminance_nits(), h.min_luminance_nits(), h.max_frame_avg_nits());
    }
});
