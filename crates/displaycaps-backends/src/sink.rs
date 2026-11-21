// SPDX-License-Identifier: LGPL-2.1-or-later
// The identity part follows AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-backends/src/assemble/sink.rs
// (sink key, PNP name, serial); the video mapping is new for DisplayCaps.
//! The sink record: what the device at the far end of the cable says it is and can show, built
//! from its EDID (CTA-861 and DisplayID included) and quirk data.
//!
//! Every value read from the EDID is `measured`; values a quirk entry changes are `inferred` with a
//! `quirk` source. Field groups outside the scope are left out (`None`), never read. Devices in
//! between (receiver, adapter, dock) need the connector and other layers, so `chain` is unknown here.

use std::collections::BTreeMap;

use displaycaps_model::{
    CapabilitySet, ColorFormat, ColorFormatSupport, Colorimetry, DscSupport, DynamicHdr, EdidInfo,
    Eotf, FieldGroup, FrlRate, HdrCapabilities, HdrFormat, LayerKind, LinkLimits, Mode, ModeSource,
    Provenance, QuantizationSupport, QuirkEffect, RangeLimits, Selection, Sensitive, SensitiveKind,
    SinkIdentity, SinkRecord, Source, Timing, UnknownReason, Value, VrrKind, VrrRange, Ycbcr420,
    sink_key,
};
use displaycaps_parse::cta_video::CtaVideo;
use displaycaps_parse::edid::{Edid, parse_edid, serial};
use displaycaps_parse::timing::{self, DigitalInterface, ModeSource as PModeSource, VideoInput};
use displaycaps_parse::{dmt, ids, vic};
use displaycaps_rules::quirks::{QuirkSet, SinkKey};

/// Builds the sink record from raw EDID bytes, for the field groups in `fields`.
///
/// `source` names where the bytes came from (the sysfs `edid` file or the KMS blob) and `now` is
/// the read time. Returns `None` for an empty EDID (nothing connected); an EDID that can't be
/// parsed gives a record whose values are unknown with `parse_error`.
pub fn sink_record(
    raw: &[u8],
    source: Source,
    now: u64,
    fields: &Selection<FieldGroup>,
    quirks: &QuirkSet,
) -> Option<SinkRecord> {
    if raw.is_empty() {
        return None;
    }
    let prov = Provenance {
        layer: LayerKind::SinkChain,
        source,
        read_at_ms: now,
    };
    let e = match parse_edid(raw) {
        Ok(e) => e,
        Err(err) => return Some(unreadable(format!("{err:?}"))),
    };
    let want = |g: FieldGroup| fields.includes(&g);

    let identity = identity(&e, raw, &prov, now);
    let names: Vec<&str> = e.name.iter().map(String::as_str).collect();
    let applied = quirks.matching(&SinkKey {
        manufacturer: e.manufacturer.as_deref(),
        product_code: Some(e.product_code),
        model_names: names,
        cec_vendor: None,
    });

    let mut video = CapabilitySet::default();
    let ctas: Vec<&CtaVideo> = e
        .cta
        .iter()
        .map(|c| &c.video)
        .chain(e.displayid_cta.iter())
        .collect();
    if want(FieldGroup::Modes) {
        let mut modes = modes(&e, &ctas);
        for q in &applied {
            for eff in &q.effects {
                if let QuirkEffect::RemoveMode {
                    width,
                    height,
                    refresh_millihz,
                } = eff
                {
                    modes.retain(|m| {
                        (m.width, m.height, m.refresh_millihz)
                            != (*width, *height, *refresh_millihz)
                    });
                }
            }
        }
        video.modes = Some(Value::measured(modes, prov.clone()));
    }
    if want(FieldGroup::Vrr) {
        video.vrr = Some(match vrr(&e, &ctas) {
            Some(r) => Value::measured(r, prov.clone()),
            None => Value::unknown(
                UnknownReason::NotExposed,
                Some("the EDID advertises no variable refresh range".into()),
            ),
        });
    }
    if want(FieldGroup::ColorFormats) {
        video.color_formats = Some(Value::measured(color_formats(&e, &ctas), prov.clone()));
    }
    if want(FieldGroup::Hdr) {
        video.hdr = Some(hdr(&ctas, &applied, &prov, now));
    }
    if want(FieldGroup::Colorimetry) {
        video.colorimetry = Some(Value::measured(colorimetry(&ctas), prov.clone()));
    }
    if want(FieldGroup::Quantization) {
        video.quantization = Some(match ctas.iter().find_map(|c| c.capability.as_ref()) {
            Some(c) => Value::measured(
                QuantizationSupport {
                    rgb_selectable: c.qs,
                    ycc_selectable: c.qy,
                },
                prov.clone(),
            ),
            None => Value::unknown(
                UnknownReason::NotExposed,
                Some("no CTA-861 video capability data block".into()),
            ),
        });
    }
    let link = if want(FieldGroup::Link) {
        Value::measured(link(&e, &ctas), prov.clone())
    } else {
        Value::unknown(
            UnknownReason::NotRequested,
            Some("link field group not in scope".into()),
        )
    };
    let edid = Value::measured(edid_info(&e, raw), prov.clone());
    Some(SinkRecord {
        identity,
        edid,
        video,
        link,
        chain: Value::unknown(
            UnknownReason::NotImplemented,
            Some("devices in between are detected from the connector and link layers".into()),
        ),
        quirks: applied,
    })
}

fn unreadable(detail: String) -> SinkRecord {
    fn u<T>(d: &str) -> Value<T> {
        Value::unknown(UnknownReason::ParseError, Some(d.to_string()))
    }
    SinkRecord {
        identity: SinkIdentity {
            manufacturer_id: u(&detail),
            manufacturer_name: u(&detail),
            product_code: u(&detail),
            model_name: u(&detail),
            serial: None,
            sink_key: u(&detail),
        },
        edid: u(&detail),
        video: CapabilitySet::default(),
        link: u(&detail),
        chain: u(&detail),
        quirks: vec![],
    }
}

/// Sink identity and key, the same way AudioCaps v1.0.0 derives them (the parity test in
/// displaycaps-parse checks the key).
fn identity(e: &Edid, raw: &[u8], prov: &Provenance, now: u64) -> SinkIdentity {
    let serial = serial(raw);
    let key = sink_key(
        e.manufacturer.as_deref(),
        e.manufacturer_id.swap_bytes(),
        e.product_code,
        serial.as_deref(),
    );
    SinkIdentity {
        manufacturer_id: match &e.manufacturer {
            Some(m) => Value::measured(m.clone(), prov.clone()),
            None => Value::unknown(UnknownReason::ParseError, Some("invalid PNP id".into())),
        },
        manufacturer_name: match e.manufacturer.as_deref().and_then(ids::pnp_manufacturer) {
            Some(n) => Value::inferred(
                n.to_string(),
                Provenance {
                    layer: LayerKind::SinkChain,
                    source: Source::Table { name: "pnp".into() },
                    read_at_ms: now,
                },
            ),
            None => Value::unknown(
                UnknownReason::NotExposed,
                Some("manufacturer id not in the name table".into()),
            ),
        },
        product_code: Value::measured(e.product_code, prov.clone()),
        model_name: match &e.name {
            Some(n) => Value::measured(n.clone(), prov.clone()),
            None => Value::unknown(UnknownReason::NotExposed, None),
        },
        serial: serial.map(|s| Sensitive::new(SensitiveKind::Serial, s)),
        sink_key: Value::inferred(
            key,
            Provenance {
                layer: LayerKind::SinkChain,
                source: Source::derived("manufacturer, product and serial"),
                read_at_ms: now,
            },
        ),
    }
}

fn edid_info(e: &Edid, raw: &[u8]) -> EdidInfo {
    let mut extensions = vec![];
    let mut cta = e.cta.iter();
    let mut did = e.displayid.iter();
    let mut ok = e.checksum_ok;
    for blk in raw
        .chunks(timing_block())
        .skip(1)
        .take(usize::from(e.extension_count))
    {
        match blk.first() {
            Some(0x02) => {
                extensions.push("cta-861".to_string());
                ok &= cta.next().is_some_and(|c| c.checksum_ok);
            }
            Some(0x70) => {
                extensions.push("displayid".to_string());
                ok &= did.next().is_some_and(|d| d.checksum_valid);
            }
            Some(0xF0) => extensions.push("block-map".to_string()),
            Some(t) => extensions.push(format!("0x{t:02x}")),
            None => {}
        }
    }
    let manufactured = match (e.week, e.year) {
        (255, y) => Some(format!("model year {y}")),
        (0, y) => Some(format!("{y}")),
        (w, y) if w <= 54 => Some(format!("week {w} of {y}")),
        _ => None,
    };
    let interface = match &e.base.input {
        VideoInput::Digital(d) => match d.interface {
            DigitalInterface::HdmiA => Some("hdmi-a".to_string()),
            DigitalInterface::HdmiB => Some("hdmi-b".to_string()),
            DigitalInterface::DisplayPort => Some("displayport".to_string()),
            DigitalInterface::Dvi => Some("dvi".to_string()),
            DigitalInterface::Mddi => Some("mddi".to_string()),
            _ => None,
        },
        VideoInput::Analog(_) => Some("analog".to_string()),
    };
    EdidInfo {
        version: format!("{}.{}", e.version.0, e.version.1),
        length: raw.len() as u32,
        extensions,
        checksums_valid: ok && e.missing_extensions == 0,
        manufactured,
        interface,
        range_limits: e.base.range_limits().map(|r| RangeLimits {
            min_vertical_hz: r.min_vertical_hz,
            max_vertical_hz: r.max_vertical_hz,
            min_horizontal_khz: r.min_horizontal_khz,
            max_horizontal_khz: r.max_horizontal_khz,
            max_pixel_clock_khz: r.max_pixel_clock_khz,
            formula: r.formula.name().map(str::to_string),
        }),
    }
}

const fn timing_block() -> usize {
    displaycaps_parse::edid::BLOCK
}

type ModeKey = (u16, u16, u32, bool);

/// Adds a mode (or one more source for it) and returns its key.
fn put(
    map: &mut BTreeMap<ModeKey, Mode>,
    w: u32,
    h: u32,
    r: u32,
    il: bool,
    t: Option<Timing>,
    src: ModeSource,
) -> ModeKey {
    let k = (
        u16::try_from(w).unwrap_or(u16::MAX),
        u16::try_from(h).unwrap_or(u16::MAX),
        r,
        il,
    );
    let m = map.entry(k).or_insert_with(|| Mode {
        width: k.0,
        height: k.1,
        refresh_millihz: r,
        interlaced: il,
        timing: None,
        vic: None,
        hdmi_vic: None,
        preferred: false,
        ycbcr420: Ycbcr420::No,
        sources: vec![],
    });
    if m.timing.is_none() {
        m.timing = t;
    }
    if !m.sources.contains(&src) {
        m.sources.push(src);
    }
    k
}

/// All modes the sink lists, merged by (width, height, refresh, interlaced) with every source kept.
fn modes(e: &Edid, ctas: &[&CtaVideo]) -> Vec<Mode> {
    let mut map: BTreeMap<ModeKey, Mode> = BTreeMap::new();
    let mut preferred = None;
    for m in timing::modes(&e.base) {
        let src = match m.source {
            PModeSource::Established | PModeSource::EstablishedIii => ModeSource::Established,
            PModeSource::Standard => ModeSource::Standard,
            PModeSource::Detailed => ModeSource::Detailed,
            PModeSource::Cvt3 => ModeSource::Cvt,
        };
        let t = match (m.pixel_clock_khz, m.h_total, m.v_total) {
            (Some(c), Some(h), Some(v)) => Some(Timing {
                pixel_clock_khz: c,
                h_total: u16::try_from(h).unwrap_or(u16::MAX),
                v_total: u16::try_from(v).unwrap_or(u16::MAX),
                h_sync_positive: None,
                v_sync_positive: None,
            }),
            _ => None,
        };
        let k = put(
            &mut map,
            m.width,
            m.height,
            m.refresh_millihz,
            m.interlaced,
            t,
            src,
        );
        if src == ModeSource::Detailed && preferred.is_none() {
            preferred = Some(k);
        }
    }
    for c in ctas {
        for d in &c.detailed {
            put(
                &mut map,
                u32::from(d.h_active),
                d.height(),
                d.refresh_millihz(),
                d.interlaced,
                Some(Timing {
                    pixel_clock_khz: d.pixel_clock_khz,
                    h_total: u16::try_from(d.h_total()).unwrap_or(u16::MAX),
                    v_total: u16::try_from(d.v_total_frame()).unwrap_or(u16::MAX),
                    h_sync_positive: d.h_sync_positive,
                    v_sync_positive: d.v_sync_positive,
                }),
                ModeSource::Detailed,
            );
        }
        let y420_cap = c.y420_capable_vics();
        let y420_only: Vec<u8> = c.y420_only.iter().map(|s| s.vic).collect();
        for s in c.svds.iter().chain(c.y420_only.iter()) {
            let Some(t) = vic::vic(s.vic) else { continue };
            let k = put(
                &mut map,
                u32::from(t.width),
                u32::from(t.height),
                t.refresh_millihz,
                t.interlaced,
                Some(Timing {
                    pixel_clock_khz: t.pixel_clock_khz,
                    h_total: t.h_total,
                    v_total: t.v_total,
                    h_sync_positive: None,
                    v_sync_positive: None,
                }),
                ModeSource::CtaVic,
            );
            if let Some(m) = map.get_mut(&k) {
                m.vic.get_or_insert(s.vic);
                if y420_only.contains(&s.vic) {
                    m.ycbcr420 = Ycbcr420::Only;
                } else if y420_cap.contains(&s.vic) && m.ycbcr420 == Ycbcr420::No {
                    m.ycbcr420 = Ycbcr420::Also;
                }
            }
        }
        for code in c.hdmi.iter().flat_map(|h| h.hdmi_vics.iter()) {
            let Some(t) = vic::hdmi_vic(*code) else {
                continue;
            };
            let k = put(
                &mut map,
                u32::from(t.width),
                u32::from(t.height),
                t.refresh_millihz,
                false,
                Some(Timing {
                    pixel_clock_khz: t.pixel_clock_khz,
                    h_total: t.h_total,
                    v_total: t.v_total,
                    h_sync_positive: None,
                    v_sync_positive: None,
                }),
                ModeSource::HdmiVic,
            );
            if let Some(m) = map.get_mut(&k) {
                m.hdmi_vic.get_or_insert(*code);
            }
        }
    }
    for d in &e.displayid {
        for t in d.timings() {
            let timing = (t.pixel_clock_khz > 0).then(|| Timing {
                pixel_clock_khz: t.pixel_clock_khz,
                h_total: u16::try_from(t.h_total).unwrap_or(u16::MAX),
                v_total: u16::try_from(t.v_total).unwrap_or(u16::MAX),
                h_sync_positive: None,
                v_sync_positive: None,
            });
            let k = put(
                &mut map,
                t.width,
                t.height,
                t.refresh_millihz,
                t.interlaced,
                timing,
                ModeSource::DisplayId,
            );
            if t.preferred && preferred.is_none() {
                preferred = Some(k);
            }
        }
        for c in &d.timing_codes {
            if c.kind == displaycaps_parse::displayid::DidCodeKind::Dmt
                && let Some(t) = u8::try_from(c.code).ok().and_then(dmt::by_id)
            {
                put(
                    &mut map,
                    u32::from(t.width),
                    u32::from(t.height),
                    t.refresh_millihz,
                    t.interlaced,
                    Some(Timing {
                        pixel_clock_khz: t.pixel_clock_khz,
                        h_total: t.h_total,
                        v_total: t.v_total,
                        h_sync_positive: None,
                        v_sync_positive: None,
                    }),
                    ModeSource::DisplayId,
                );
            }
        }
    }
    if let Some(k) = preferred
        && let Some(m) = map.get_mut(&k)
    {
        m.preferred = true;
    }
    // Largest first, then highest refresh: the order apps usually want.
    let mut out: Vec<Mode> = map.into_values().collect();
    out.sort_by(|a, b| {
        (u32::from(b.width) * u32::from(b.height), b.refresh_millihz)
            .cmp(&(u32::from(a.width) * u32::from(a.height), a.refresh_millihz))
            .then(a.interlaced.cmp(&b.interlaced))
    });
    out
}

/// HDMI 2.1 VRR wins over FreeSync, which wins over DisplayID adaptive sync.
fn vrr(e: &Edid, ctas: &[&CtaVideo]) -> Option<VrrRange> {
    if let Some(hf) = ctas.iter().find_map(|c| c.hdmi_forum())
        && hf.vrr_min > 0
    {
        let max = if hf.vrr_max > 0 {
            u32::from(hf.vrr_max)
        } else {
            // VRRmax 0: the highest refresh of the modes the sink lists (HDMI 2.1 §10.3).
            let modes = modes(e, ctas);
            modes.iter().map(|m| m.refresh_millihz).max().unwrap_or(0) / 1000
        };
        return Some(VrrRange {
            min_millihz: u32::from(hf.vrr_min) * 1000,
            max_millihz: max * 1000,
            kind: VrrKind::HdmiVrr,
        });
    }
    if let Some(fs) = ctas.iter().find_map(|c| c.freesync.as_ref())
        && fs.min_hz > 0
        && fs.max_hz > fs.min_hz
    {
        return Some(VrrRange {
            min_millihz: u32::from(fs.min_hz) * 1000,
            max_millihz: u32::from(fs.max_hz) * 1000,
            kind: VrrKind::AdaptiveSync,
        });
    }
    e.displayid
        .iter()
        .flat_map(|d| d.adaptive_sync())
        .find(|a| a.min_refresh_millihz > 0 && a.max_refresh_millihz > a.min_refresh_millihz)
        .map(|a| VrrRange {
            min_millihz: a.min_refresh_millihz,
            max_millihz: a.max_refresh_millihz,
            kind: VrrKind::AdaptiveSync,
        })
}

fn color_formats(e: &Edid, ctas: &[&CtaVideo]) -> Vec<ColorFormatSupport> {
    let hdmi = ctas.iter().find_map(|c| c.hdmi.as_ref());
    let hf = ctas.iter().find_map(|c| c.hdmi_forum());
    let deep = |d30: bool, d36: bool, d48: bool| {
        let mut v = vec![8u8];
        for (on, b) in [(d30, 10u8), (d36, 12), (d48, 16)] {
            if on {
                v.push(b);
            }
        }
        v
    };
    let mut rgb = match hdmi {
        Some(h) => deep(h.dc_30bit, h.dc_36bit, h.dc_48bit),
        None => vec![8],
    };
    if let VideoInput::Digital(d) = &e.base.input
        && let Some(b) = d.bits_per_color
        && !rgb.contains(&b)
    {
        rgb.push(b);
        rgb.sort_unstable();
    }
    let mut out = vec![ColorFormatSupport {
        format: ColorFormat::Rgb,
        bit_depths: rgb.clone(),
    }];
    if ctas.iter().any(|c| c.ycbcr444) {
        out.push(ColorFormatSupport {
            format: ColorFormat::Ycbcr444,
            bit_depths: if hdmi.is_some_and(|h| h.dc_y444) {
                rgb.clone()
            } else {
                vec![8]
            },
        });
    }
    if ctas.iter().any(|c| c.ycbcr422) {
        // 4:2:2 carries up to 12 bits at the 8-bit rate on HDMI.
        out.push(ColorFormatSupport {
            format: ColorFormat::Ycbcr422,
            bit_depths: vec![8, 10, 12],
        });
    }
    if ctas.iter().any(|c| !c.y420_capable_vics().is_empty()) {
        out.push(ColorFormatSupport {
            format: ColorFormat::Ycbcr420,
            bit_depths: match hf {
                Some(h) => deep(h.dc_30bit_420, h.dc_36bit_420, h.dc_48bit_420),
                None => vec![8],
            },
        });
    }
    out
}

fn hdr(
    ctas: &[&CtaVideo],
    applied: &[displaycaps_model::AppliedQuirk],
    prov: &Provenance,
    now: u64,
) -> Value<HdrCapabilities> {
    let mut h = HdrCapabilities::default();
    let mut any = false;
    if let Some(s) = ctas.iter().find_map(|c| c.hdr_static.as_ref()) {
        any = true;
        for (on, eotf) in [
            (s.eotf_sdr, Eotf::TraditionalSdr),
            (s.eotf_hdr_gamma, Eotf::TraditionalHdr),
            (s.eotf_pq, Eotf::Pq),
            (s.eotf_hlg, Eotf::Hlg),
        ] {
            if on {
                h.eotfs.push(eotf);
            }
        }
        if s.eotf_pq {
            h.formats.push(HdrFormat::Hdr10);
        }
        if s.eotf_hlg {
            h.formats.push(HdrFormat::Hlg);
        }
        h.static_metadata_types = (0..8)
            .filter(|b| s.metadata_types & (1 << b) != 0)
            .map(|b| b + 1)
            .collect();
        h.max_luminance_nits = s.max_luminance_nits();
        h.max_frame_average_nits = s.max_frame_avg_nits();
        h.min_luminance_nits = s.min_luminance_nits();
    }
    for c in ctas {
        if let Some(p) = &c.hdr10_plus {
            any = true;
            h.formats.push(HdrFormat::Hdr10Plus);
            h.dynamic.push(DynamicHdr::Hdr10Plus {
                application_version: p.application_version,
            });
        }
        if let Some(dv) = &c.dolby_vision {
            any = true;
            h.formats.push(HdrFormat::DolbyVision);
            h.dynamic.push(DynamicHdr::DolbyVision {
                version: dv.version,
                yuv422_12bit: Some(dv.yuv422_12bit),
                low_latency: dv.low_latency,
                max_luminance_nits: dv.target_max_nits,
            });
        }
        for d in &c.hdr_dynamic {
            any = true;
            // Types 2 and 4 are HDR10+ and its successor; they are already listed via their VSVDB.
            h.dynamic.push(DynamicHdr::Other {
                metadata_type: d.kind,
            });
        }
    }
    h.formats.dedup();
    if !any {
        return Value::unknown(
            UnknownReason::NotExposed,
            Some("the EDID has no HDR static metadata or dynamic HDR block (SDR display)".into()),
        );
    }
    // Quirk data: a known real peak replaces a misreported one.
    for q in applied {
        for eff in &q.effects {
            if let QuirkEffect::PeakLuminance { nits } = eff {
                h.max_luminance_nits = Some(*nits);
                return Value::inferred(
                    h,
                    Provenance {
                        layer: LayerKind::SinkChain,
                        source: Source::Quirk { id: q.id.clone() },
                        read_at_ms: now,
                    },
                );
            }
        }
    }
    Value::measured(h, prov.clone())
}

fn colorimetry(ctas: &[&CtaVideo]) -> Vec<Colorimetry> {
    let mut out = vec![Colorimetry::Default];
    for c in ctas.iter().filter_map(|c| c.colorimetry.as_ref()) {
        for (on, v) in [
            (c.xvycc601, Colorimetry::Xvycc601),
            (c.xvycc709, Colorimetry::Xvycc709),
            (c.sycc601, Colorimetry::Sycc601),
            (c.opycc601, Colorimetry::OpYcc601),
            (c.oprgb, Colorimetry::OpRgb),
            (c.bt2020_cycc, Colorimetry::Bt2020Cycc),
            (c.bt2020_ycc, Colorimetry::Bt2020Ycc),
            (c.bt2020_rgb, Colorimetry::Bt2020Rgb),
            (c.ictcp, Colorimetry::Ictcp),
            (c.st2113_rgb, Colorimetry::St2113Rgb),
        ] {
            if on && !out.contains(&v) {
                out.push(v);
            }
        }
    }
    out
}

fn link(e: &Edid, ctas: &[&CtaVideo]) -> LinkLimits {
    let hdmi = ctas.iter().find_map(|c| c.hdmi.as_ref());
    let hf = ctas.iter().find_map(|c| c.hdmi_forum());
    let tmds = hf
        .map(|h| h.max_tmds_mhz)
        .filter(|m| *m > 0)
        .or_else(|| hdmi.and_then(|h| h.max_tmds_mhz))
        // An HDMI sink without a stated limit is held to the HDMI 1.x 165 MHz baseline.
        .or(hdmi.map(|_| 165));
    let frl = |r: &displaycaps_parse::cta_video::FrlRate| {
        r.lanes_and_gbps().map(|(lanes, gbps)| FrlRate {
            lanes,
            gbps_per_lane: gbps,
        })
    };
    let dsc = hf.filter(|h| h.dsc_1p2).map(|h| DscSupport {
        version: Some("1.2a".into()),
        max_bpc: Some(if h.dsc_16bpc {
            16
        } else if h.dsc_12bpc {
            12
        } else if h.dsc_10bpc {
            10
        } else {
            8
        }),
        native_420: Some(h.dsc_native_420),
        max_frl: frl(&h.dsc_max_frl),
        max_slices: h.dsc_max_slices().map(|(s, _)| s),
    });
    let max_bpc = color_formats(e, ctas)
        .iter()
        .flat_map(|c| c.bit_depths.iter().copied())
        .max();
    LinkLimits {
        max_tmds_character_rate_khz: tmds.map(|m| u32::from(m) * 1000),
        max_frl: hf.and_then(|h| frl(&h.max_frl)),
        max_dp: None,
        dsc,
        allm: hf.map(|h| h.allm),
        max_bpc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use displaycaps_model::Selection;

    fn hisense() -> Vec<u8> {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/parity/edid/hisense-55qd5n.bin"
        ))
        .unwrap()
    }

    fn rec(raw: &[u8], fields: Selection<FieldGroup>) -> SinkRecord {
        sink_record(
            raw,
            Source::file("/sys/class/drm/card1-HDMI-A-1/edid"),
            1,
            &fields,
            &QuirkSet::bundled(),
        )
        .unwrap()
    }

    #[test]
    fn hisense_tv_identity_and_video() {
        let r = rec(&hisense(), Selection::All);
        assert_eq!(
            r.identity.sink_key.known().map(String::as_str),
            Some("HEC-a030")
        );
        assert_eq!(
            r.identity.manufacturer_name.known().map(String::as_str),
            Some("Hisense")
        );
        let modes = r.video.modes.unwrap();
        let modes = modes.known().unwrap();
        let p = modes.iter().find(|m| m.preferred).unwrap();
        assert_eq!((p.width, p.height, p.refresh_millihz), (3840, 2160, 60_000));
        let m24 = modes
            .iter()
            .find(|m| (m.width, m.height, m.refresh_millihz) == (3840, 2160, 24_000))
            .unwrap();
        assert!(
            m24.sources.contains(&ModeSource::CtaVic) && m24.sources.contains(&ModeSource::HdmiVic)
        );
        let m60 = modes.iter().find(|m| m.vic == Some(97)).unwrap();
        assert_eq!(m60.ycbcr420, Ycbcr420::Also);
        let hdr = r.video.hdr.unwrap();
        let hdr = hdr.known().unwrap();
        for f in [
            HdrFormat::Hdr10,
            HdrFormat::Hlg,
            HdrFormat::Hdr10Plus,
            HdrFormat::DolbyVision,
        ] {
            assert!(hdr.formats.contains(&f), "{f:?}");
        }
        let link = r.link.known().unwrap();
        assert_eq!(link.max_tmds_character_rate_khz, Some(600_000));
        assert_eq!(link.max_frl, None);
        assert_eq!(link.allm, Some(true));
        let vrr = r.video.vrr.unwrap();
        let vrr = vrr.known().unwrap();
        assert_eq!((vrr.min_millihz, vrr.kind), (48_000, VrrKind::HdmiVrr));
        assert!(
            r.video
                .colorimetry
                .unwrap()
                .known()
                .unwrap()
                .contains(&Colorimetry::Bt2020Rgb)
        );
        let q = r.video.quantization.unwrap();
        assert!(q.known().unwrap().rgb_selectable);
        let fmts = r.video.color_formats.unwrap();
        let fmts = fmts.known().unwrap();
        assert!(
            fmts.iter()
                .any(|f| f.format == ColorFormat::Ycbcr420 && f.bit_depths.contains(&10))
        );
        let info = r.edid.known().unwrap();
        assert_eq!(info.version, "1.3");
        assert_eq!(info.extensions, vec!["cta-861".to_string()]);
        assert!(info.checksums_valid);
        assert_eq!(info.range_limits.as_ref().unwrap().max_vertical_hz, 75);
    }

    #[test]
    fn scope_limits_what_is_read() {
        let r = rec(
            &hisense(),
            Selection::Only(vec![FieldGroup::Sink, FieldGroup::Hdr]),
        );
        assert!(
            r.video.modes.is_none() && r.video.vrr.is_none() && r.video.color_formats.is_none()
        );
        assert!(r.video.hdr.is_some());
        assert!(matches!(
            r.link,
            Value::Unknown {
                reason: UnknownReason::NotRequested,
                ..
            }
        ));
    }

    #[test]
    fn empty_and_garbage() {
        let q = QuirkSet::bundled();
        assert!(sink_record(&[], Source::file("/x"), 0, &Selection::All, &q).is_none());
        let r = sink_record(&[1, 2, 3], Source::file("/x"), 0, &Selection::All, &q).unwrap();
        assert!(matches!(
            r.identity.sink_key,
            Value::Unknown {
                reason: UnknownReason::ParseError,
                ..
            }
        ));
    }

    #[test]
    fn quirk_peak_luminance_is_inferred_from_quirk() {
        let pack = r#"{"format":1,"entries":[{"id":"test-peak","description":"t","match":{"manufacturer":"HEC","product_code":41008},"effects":[{"effect":"peak_luminance","nits":700.0}]}]}"#;
        let q = QuirkSet::load([("test".to_string(), pack)]);
        let r = sink_record(&hisense(), Source::file("/x"), 0, &Selection::All, &q).unwrap();
        match r.video.hdr.unwrap() {
            Value::Known {
                value, provenance, ..
            } => {
                assert_eq!(value.max_luminance_nits, Some(700.0));
                assert_eq!(
                    provenance.source,
                    Source::Quirk {
                        id: "test-peak".into()
                    }
                );
            }
            Value::Unknown { .. } => panic!("hdr known"),
        }
        assert_eq!(r.quirks.len(), 1);
    }
}
