// SPDX-License-Identifier: LGPL-2.1-or-later
//! Prints displaycaps-parse's view of an EDID as normalized fact lines, in the same format as
//! `scripts/edid_facts_oracle.py` produces from edid-decode and di-edid-decode, so the three can be
//! diffed (`scripts/edid-diff.sh`, lab only). Usage: `edid_facts FILE`.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use displaycaps_parse::displayid::DidCodeKind;
use displaycaps_parse::edid::parse_edid;
use displaycaps_parse::timing::{ModeSource, modes};
use displaycaps_parse::{dmt, vic};

fn hz(millihz: u32) -> String {
    format!("{}.{:03}", millihz / 1000, millihz % 1000)
}

fn mode(src: &str, w: u32, h: u32, il: bool, millihz: u32, pclk: Option<u32>) -> String {
    let mut s = format!(
        "mode {src} {w}x{h}{} {}",
        if il { "i" } else { "" },
        hz(millihz)
    );
    if let Some(k) = pclk.filter(|k| *k > 0) {
        let _ = write!(s, " pclk {k}");
    }
    s
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: edid_facts FILE");
    let raw = std::fs::read(&path).expect("readable file");
    let e = match parse_edid(&raw) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("parse error: {err:?}");
            std::process::exit(1);
        }
    };
    let mut f = BTreeSet::new();
    let mut add = |s: String| {
        f.insert(s);
    };
    add(format!("version {}.{}", e.version.0, e.version.1));
    if let Some(m) = &e.manufacturer {
        add(format!("mfg {m}"));
    }
    add(format!("product {}", e.product_code));
    if let Some(n) = e.base.product_name() {
        add(format!("name {n}"));
    }
    for m in modes(&e.base) {
        let src = match m.source {
            ModeSource::Established => "est",
            ModeSource::Standard => "std",
            ModeSource::Detailed => "dtd",
            ModeSource::Cvt3 => "cvt3",
            ModeSource::EstablishedIii => "est3",
        };
        add(mode(
            src,
            m.width,
            m.height,
            m.interlaced,
            m.refresh_millihz,
            m.pixel_clock_khz,
        ));
    }
    if let Some(r) = e.base.range_limits() {
        let mut s = format!(
            "range v {}-{} h {}-{}",
            r.min_vertical_hz, r.max_vertical_hz, r.min_horizontal_khz, r.max_horizontal_khz
        );
        if let Some(k) = r.max_pixel_clock_khz {
            let _ = write!(s, " pclk {}", k / 1000);
        }
        add(s);
    }
    let videos = e.cta.iter().map(|c| &c.video).chain(e.displayid_cta.iter());
    for v in videos {
        for t in &v.detailed {
            add(mode(
                "dtd",
                u32::from(t.h_active),
                t.height(),
                t.interlaced,
                t.refresh_millihz(),
                Some(t.pixel_clock_khz),
            ));
        }
        // VIC 0 is reserved; an SVD carrying it names no format.
        for s in v.svds.iter().filter(|s| s.vic != 0) {
            add(format!("vic {}", s.vic));
            if let Some(t) = vic::vic(s.vic) {
                add(mode(
                    "vic",
                    u32::from(t.width),
                    u32::from(t.height),
                    t.interlaced,
                    t.refresh_millihz,
                    Some(t.pixel_clock_khz),
                ));
            }
        }
        for s in &v.y420_only {
            add(format!("vic420only {}", s.vic));
        }
        if let Some(map) = &v.y420_cap_map {
            for (i, s) in v.svds.iter().enumerate() {
                let set = map.is_empty() || map.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0);
                if set {
                    add(format!("vic420cap {}", s.vic));
                }
            }
        }
        if let Some(cap) = &v.capability {
            add(format!("vcdb qy {}", u8::from(cap.qy)));
            add(format!("vcdb qs {}", u8::from(cap.qs)));
        }
        if let Some(c) = &v.colorimetry {
            for (on, name) in [
                (c.xvycc601, "xvYCC601"),
                (c.xvycc709, "xvYCC709"),
                (c.sycc601, "sYCC601"),
                (c.opycc601, "opYCC601"),
                (c.oprgb, "opRGB"),
                (c.bt2020_cycc, "BT2020cYCC"),
                (c.bt2020_ycc, "BT2020YCC"),
                (c.bt2020_rgb, "BT2020RGB"),
                (c.ictcp, "ICtCp"),
                (c.st2113_rgb, "ST2113RGB"),
            ] {
                if on {
                    add(format!("colorimetry {name}"));
                }
            }
        }
        if let Some(h) = &v.hdr_static {
            for (on, name) in [
                (h.eotf_sdr, "sdr"),
                (h.eotf_hdr_gamma, "hdr"),
                (h.eotf_pq, "pq"),
                (h.eotf_hlg, "hlg"),
            ] {
                if on {
                    add(format!("eotf {name}"));
                }
            }
        }
        if let Some(h) = &v.hdmi {
            if let Some(t) = h.max_tmds_mhz {
                add(format!("hdmi tmds {t}"));
            }
            for (on, n) in [
                (h.dc_30bit, "30"),
                (h.dc_36bit, "36"),
                (h.dc_48bit, "48"),
                (h.dc_y444, "y444"),
            ] {
                if on {
                    add(format!("hdmi dc {n}"));
                }
            }
            for code in &h.hdmi_vics {
                if let Some(t) = vic::hdmi_vic(*code) {
                    add(mode(
                        "hdmivic",
                        u32::from(t.width),
                        u32::from(t.height),
                        t.interlaced,
                        t.refresh_millihz,
                        Some(t.pixel_clock_khz),
                    ));
                }
            }
        }
        for hf in [&v.hdmi_forum_vsdb, &v.hdmi_forum_scdb]
            .into_iter()
            .flatten()
        {
            add(format!("hf version {}", hf.version));
            if hf.max_tmds_mhz > 0 {
                add(format!("hf tmds {}", hf.max_tmds_mhz));
            }
            if let Some((lanes, gbps)) = hf.max_frl.lanes_and_gbps() {
                add(format!("hf frl {lanes}x{gbps}"));
            }
            if hf.allm {
                add("hf allm".into());
            }
            if hf.vrr_min > 0 {
                add(format!("hf vrrmin {}", hf.vrr_min));
            }
            if hf.vrr_max > 0 {
                add(format!("hf vrrmax {}", hf.vrr_max));
            }
            if hf.dsc_1p2 {
                add("hf dsc 1.2a".into());
            }
            if hf.dsc_native_420 {
                add("hf dsc native420".into());
            }
        }
        if let Some(fs) = &v.freesync {
            add(format!("freesync min {}", fs.min_hz));
            add(format!("freesync max {}", fs.max_hz));
        }
        if let Some(dv) = &v.dolby_vision {
            add(format!("dv version {}", dv.version));
        }
        if let Some(p) = &v.hdr10_plus {
            add(format!("hdr10plus {}", p.application_version));
        }
    }
    for d in &e.displayid {
        add(format!(
            "displayid version {}.{}",
            d.version >> 4,
            d.version & 0x0F
        ));
        for t in d.timings() {
            add(mode(
                "did",
                t.width,
                t.height,
                t.interlaced,
                t.refresh_millihz,
                Some(t.pixel_clock_khz),
            ));
        }
        for c in &d.timing_codes {
            if c.kind == DidCodeKind::Dmt
                && let Some(t) = u8::try_from(c.code).ok().and_then(dmt::by_id)
            {
                add(mode(
                    "did",
                    u32::from(t.width),
                    u32::from(t.height),
                    t.interlaced,
                    t.refresh_millihz,
                    Some(t.pixel_clock_khz),
                ));
            }
        }
    }
    for l in f {
        println!("{l}");
    }
}
