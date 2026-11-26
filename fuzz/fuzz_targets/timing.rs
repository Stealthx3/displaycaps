// SPDX-License-Identifier: LGPL-2.1-or-later
//! Base-block timings and descriptors, mode resolution, and the CVT/GTF formulas with arbitrary
//! parameters (first 9 bytes pick the formula inputs; the rest is a base block).
#![no_main]

use displaycaps_parse::cvt::{Blanking, GtfParams, cvt, gtf};
use displaycaps_parse::timing::{modes, parse_base_video, parse_dtd};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() >= 9 {
        let w = u32::from(u16::from_le_bytes([data[0], data[1]]));
        let h = u32::from(u16::from_le_bytes([data[2], data[3]]));
        let r = f64::from(u16::from_le_bytes([data[4], data[5]])) / 64.0;
        let b = match data[6] % 4 {
            0 => Blanking::Standard,
            1 => Blanking::ReducedV1,
            2 => Blanking::ReducedV2,
            _ => Blanking::ReducedV2Video,
        };
        let (il, mg) = (data[7] & 1 != 0, data[7] & 2 != 0);
        let _ = cvt(w, h, r, b, il, mg);
        let _ = gtf(w, h, r, il, mg, GtfParams::DEFAULT);
        let _ = data[8];
    }
    let base = data.get(9..).unwrap_or_default();
    let _ = parse_dtd(base);
    let v = parse_base_video(base);
    let _ = (v.range_limits(), v.product_name(), modes(&v).len());
});
