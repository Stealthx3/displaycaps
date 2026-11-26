// SPDX-License-Identifier: LGPL-2.1-or-later
//! The link-budget engine over arbitrary modes and limits: no panic, no overflow, and a chosen
//! format never needs more than its link carries.
#![no_main]

use displaycaps_link::{FrlGate, LinkKind, Request, SinkFormats, SourceLimits, budget, choose};
use displaycaps_model::{ColorFormat, DpLink, DpRate, Mode, Timing, Ycbcr420};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|d: &[u8]| {
    if d.len() < 24 {
        return;
    }
    let u16_at = |i: usize| u16::from_le_bytes([d[i], d[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    let fmt = |b: u8| match b % 4 {
        0 => ColorFormat::Rgb,
        1 => ColorFormat::Ycbcr444,
        2 => ColorFormat::Ycbcr422,
        _ => ColorFormat::Ycbcr420,
    };
    let mode = Mode {
        width: u16_at(0),
        height: u16_at(2),
        refresh_millihz: 60_000,
        interlaced: false,
        timing: Some(Timing {
            pixel_clock_khz: u32_at(4),
            h_total: u16_at(8),
            v_total: u16_at(10),
            h_sync_positive: None,
            v_sync_positive: None,
        }),
        vic: None,
        hdmi_vic: None,
        preferred: false,
        ycbcr420: Ycbcr420::Also,
        sources: vec![],
    };
    let frl = budget::FRL_LEVELS[usize::from(d[12]) % 6];
    let dp = DpLink {
        lanes: [1, 2, 4][usize::from(d[13]) % 3],
        rate: [DpRate::Rbr, DpRate::Hbr2, DpRate::Hbr3, DpRate::Uhbr20][usize::from(d[13] >> 4) % 4],
    };
    let link = match d[14] % 4 {
        0 => LinkKind::Hdmi,
        1 => LinkKind::DisplayPort { mst: false },
        2 => LinkKind::DisplayPort { mst: true },
        _ => LinkKind::Virtual,
    };
    let source = SourceLimits {
        max_tmds_khz: (d[15] & 1 != 0).then(|| u32_at(16)),
        max_frl: (d[15] & 2 != 0).then_some(frl),
        frl: [FrlGate::Enabled, FrlGate::Unsupported, FrlGate::Unknown][usize::from(d[15] >> 2) % 3].clone(),
        max_dp: (d[15] & 32 != 0).then_some(dp),
        dsc: d[15] & 64 != 0,
    };
    let depths: Vec<u8> = [8u8, 10, 12, 16].into_iter().filter(|b| d[20] & (1 << (b / 4)) != 0).collect();
    let sink = SinkFormats {
        formats: (0..4).filter(|i| d[21] & (1 << i) != 0).map(|i| (fmt(i), depths.clone())).collect(),
        max_tmds_khz: (d[22] & 1 != 0).then(|| u32_at(16) / 2),
        max_frl: (d[22] & 2 != 0).then_some(frl),
        max_dp: (d[22] & 4 != 0).then_some(dp),
        dsc: d[22] & 8 != 0,
    };
    let req = Request {
        max_bpc: d[23] % 20,
        color_format: (d[23] & 0x80 != 0).then(|| fmt(d[23])),
    };
    if let Ok(c) = choose(&mode, link, &source, &sink, req)
        && link != LinkKind::Virtual
    {
        assert!(c.required_kbps <= c.capacity_kbps, "{c:?}");
    }
});
