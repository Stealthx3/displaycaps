// SPDX-License-Identifier: LGPL-2.1-or-later
//! The inferred link: what the HDMI or DisplayPort link carries for the active mode, worked out
//! by `displaycaps-link` from the sink record, KMS (`max bpc`, `color format`), the driver's
//! limits and feature gates, and the connector (adapters, MST). Every value is *inferred*.
//!
//! Driver limits and gates are a small table here; they move to the declarative data packs with
//! the rule engine (phase 7).

use displaycaps_link::{FrlGate, LinkKind, Request, SinkFormats, SourceLimits};
use displaycaps_model::{
    Availability, CapabilitySet, ChainDevice, ChainDeviceKind, ColorFormat, Confidence,
    ConnectorType, DriverFeature, FrlRate, KmsState, LayerCapabilities, LayerDetail, LayerKind,
    LinkLimits, LinkState, Mode, ModuleParameter, Provenance, RejectedFormat, SinkRecord, Source,
    UnknownReason, Value, Ycbcr420,
};

/// Kernel version `(major, minor)` from a release string like `7.2.3-arch1-1`.
pub(crate) fn kernel_version(release: &str) -> Option<(u32, u32)> {
    let mut it = release.split(|c: char| !c.is_ascii_digit());
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

fn param<'a>(params: &'a [ModuleParameter], name: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.value.as_str())
}

fn parse_mask(v: &str) -> Option<u64> {
    let v = v.trim();
    match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => v.parse().ok(),
    }
}

/// amdgpu's `dcfeaturemask` bit that turns HDMI FRL on in Linux 7.2 and 7.3.
const AMDGPU_DC_FRL: u64 = 0x400;

/// Whether the driver has HDMI 2.1 FRL on.
///
/// - amdgpu: FRL arrived in Linux 7.2, off unless `dcfeaturemask` has bit 0x400; on by default
///   since Linux 7.4 (sources: Phoronix, "Initial AMDGPU HDMI 2.1 FRL support merged for Linux
///   7.2" and "Patches ready for AMDGPU HDMI 2.1 enabled by default with Linux 7.4").
/// - i915, xe, nouveau: no native HDMI FRL.
/// - nvidia: the proprietary driver supports FRL on HDMI 2.1 hardware; which board this is isn't
///   known here, so unknown.
pub(crate) fn frl_gate(
    driver: Option<&str>,
    kernel: Option<(u32, u32)>,
    params: &[ModuleParameter],
) -> FrlGate {
    match driver {
        Some("amdgpu") => match kernel {
            None => FrlGate::Unknown,
            Some(k) if k < (7, 2) => FrlGate::Unsupported,
            Some(k) if k < (7, 4) => {
                match param(params, "amdgpu.dcfeaturemask").and_then(parse_mask) {
                    Some(m) if m & AMDGPU_DC_FRL != 0 => FrlGate::Enabled,
                    Some(_) => FrlGate::DisabledByDriver(format!(
                        "amdgpu on Linux {}.{} turns FRL on only with dcfeaturemask bit 0x400",
                        k.0, k.1
                    )),
                    None => FrlGate::Unknown,
                }
            }
            Some(_) => FrlGate::Enabled,
        },
        Some("i915" | "xe" | "nouveau") => FrlGate::Unsupported,
        _ => FrlGate::Unknown,
    }
}

/// The source's link limits for a driver. `None` for a driver this table doesn't know.
pub(crate) fn source_limits(driver: Option<&str>, frl: FrlGate) -> Option<SourceLimits> {
    let frl48 = Some(FrlRate {
        lanes: 4,
        gbps_per_lane: 12,
    });
    let (tmds, max_frl, dsc) = match driver? {
        "amdgpu" => (600_000, frl48, true),
        "nvidia" => (600_000, frl48, true),
        "i915" | "xe" => (594_000, None, true),
        "nouveau" => (600_000, None, false),
        _ => return None,
    };
    Some(SourceLimits {
        max_tmds_khz: Some(tmds),
        max_frl,
        frl,
        max_dp: None,
        dsc,
    })
}

/// The `hdmi_frl` driver feature for the driver detail.
pub(crate) fn frl_feature(gate: &FrlGate, at: u64) -> Option<DriverFeature> {
    let prov = Provenance {
        layer: LayerKind::KernelDriver,
        source: Source::derived("driver, kernel version and module parameters"),
        read_at_ms: at,
    };
    let enabled = match gate {
        FrlGate::Enabled => Value::inferred(true, prov),
        FrlGate::DisabledByDriver(_) | FrlGate::Unsupported => Value::inferred(false, prov),
        FrlGate::Unknown => return None,
    };
    Some(DriverFeature {
        name: "hdmi_frl".into(),
        enabled,
    })
}

/// Devices the connector reveals between the GPU and the sink.
pub(crate) fn chain(
    connector: &ConnectorType,
    sink: Option<&SinkRecord>,
    mst: bool,
) -> Vec<ChainDevice> {
    let mut out = vec![];
    let hdmi_sink = sink.is_some_and(is_hdmi_sink);
    if matches!(connector, ConnectorType::DisplayPort | ConnectorType::UsbC) && hdmi_sink {
        out.push(ChainDevice {
            kind: ChainDeviceKind::Adapter,
            identity: None,
            confidence: Confidence::Inferred,
            evidence: "HDMI sink EDID on a DisplayPort connector".into(),
        });
    }
    if mst {
        out.push(ChainDevice {
            kind: ChainDeviceKind::MstHub,
            identity: None,
            confidence: Confidence::Inferred,
            evidence: "the connector has an MST path".into(),
        });
    }
    out
}

/// An HDMI sink: its EDID states an HDMI TMDS limit (HDMI or HDMI Forum VSDB).
fn is_hdmi_sink(s: &SinkRecord) -> bool {
    s.link
        .known()
        .is_some_and(|l| l.max_tmds_character_rate_khz.is_some() || l.max_frl.is_some())
}

/// What the sink accepts for `mode` (its formats and depths, 4:2:0 as the mode allows).
fn sink_formats(sink: &SinkRecord, mode: &Mode) -> SinkFormats {
    let limits = sink.link.known().cloned().unwrap_or_default();
    let all = sink
        .video
        .color_formats
        .as_ref()
        .and_then(|v| v.known())
        .cloned()
        .unwrap_or_default();
    // The sink's own entry for this mode (4:2:0 per mode), matched by size and refresh.
    let y420 = sink
        .video
        .modes
        .as_ref()
        .and_then(|m| m.known())
        .and_then(|modes| {
            modes.iter().find(|m| {
                m.width == mode.width
                    && m.height == mode.height
                    && m.interlaced == mode.interlaced
                    && m.refresh_millihz.abs_diff(mode.refresh_millihz) <= 500
            })
        })
        .map(|m| m.ycbcr420)
        .unwrap_or(Ycbcr420::No);
    let mut formats = vec![];
    for f in [
        ColorFormat::Rgb,
        ColorFormat::Ycbcr444,
        ColorFormat::Ycbcr422,
        ColorFormat::Ycbcr420,
    ] {
        let depths = all
            .iter()
            .find(|c| c.format == f)
            .map(|c| c.bit_depths.clone());
        let allowed = match (f, y420) {
            (ColorFormat::Ycbcr420, Ycbcr420::No) => false,
            (ColorFormat::Ycbcr420, _) => true,
            (_, Ycbcr420::Only) => false,
            _ => true,
        };
        if !allowed {
            continue;
        }
        match depths {
            Some(d) => formats.push((f, d)),
            // 4:2:0 allowed for the mode but no depth list: 8 bits.
            None if f == ColorFormat::Ycbcr420 => formats.push((f, vec![8])),
            None => {}
        }
    }
    SinkFormats {
        formats,
        max_tmds_khz: limits.max_tmds_character_rate_khz,
        max_frl: limits.max_frl,
        max_dp: limits.max_dp,
        dsc: limits.dsc.is_some(),
    }
}

/// Everything the link layer of one endpoint is worked out from.
pub(crate) struct Inputs<'a> {
    pub connector: &'a ConnectorType,
    pub sink: Option<&'a SinkRecord>,
    /// KMS state with every field (read for the link whatever the field scope).
    pub kms: Option<&'a KmsState>,
    pub mst: bool,
    pub driver: Option<&'a str>,
    pub frl: FrlGate,
    pub with_detail: bool,
    pub now: u64,
}

/// The link layer for one endpoint.
pub(crate) fn link_layer(i: &Inputs<'_>) -> LayerCapabilities {
    let unknown = |reason, detail: &str| LayerCapabilities {
        layer: LayerKind::Link,
        availability: Availability::Unknown {
            reason,
            detail: Some(detail.to_string()),
        },
        caps: CapabilitySet::default(),
        detail: None,
    };
    let prov = Provenance {
        layer: LayerKind::Link,
        source: Source::derived(
            "active mode, max bpc and color format (KMS), sink EDID, driver limits",
        ),
        read_at_ms: i.now,
    };
    let adapter = matches!(
        i.connector,
        ConnectorType::DisplayPort | ConnectorType::UsbC
    ) && i.sink.is_some_and(is_hdmi_sink);
    let mut limited_by = vec![];
    let kind = match i.connector {
        ConnectorType::Hdmi => LinkKind::Hdmi,
        ConnectorType::DisplayPort | ConnectorType::UsbC if adapter => {
            limited_by.push("DP-to-HDMI adapter (its TMDS limit is not known)".to_string());
            LinkKind::Hdmi
        }
        ConnectorType::DisplayPort | ConnectorType::UsbC | ConnectorType::Edp => {
            if i.mst {
                limited_by
                    .push("MST hub or dock (bandwidth shared with other streams)".to_string());
            }
            LinkKind::DisplayPort { mst: i.mst }
        }
        ConnectorType::Virtual => LinkKind::Virtual,
        _ => {
            return unknown(
                UnknownReason::NotImplemented,
                "link maths cover HDMI, DisplayPort and virtual connectors",
            );
        }
    };
    let Some(kms) = i.kms else {
        return unknown(
            UnknownReason::NotExposed,
            "the link needs the KMS state (active mode, max bpc)",
        );
    };
    let Some(mode) = kms.active_mode.known().cloned().flatten() else {
        return match kms.active_mode.known() {
            Some(None) => unknown(UnknownReason::NotExposed, "the connector drives no mode"),
            _ => unknown(UnknownReason::NotExposed, "the active mode is not known"),
        };
    };
    let sink = match (kind, i.sink) {
        (LinkKind::Virtual, _) => SinkFormats::default(),
        (_, Some(s)) => sink_formats(s, &mode),
        (_, None) => return unknown(UnknownReason::NotExposed, "the link needs the sink's EDID"),
    };
    let mut source = match (kind, source_limits(i.driver, i.frl.clone())) {
        (LinkKind::Virtual, _) => SourceLimits {
            max_tmds_khz: None,
            max_frl: None,
            frl: FrlGate::Unsupported,
            max_dp: None,
            dsc: false,
        },
        (_, Some(s)) => s,
        (_, None) => SourceLimits {
            max_tmds_khz: None,
            max_frl: None,
            frl: i.frl.clone(),
            max_dp: None,
            dsc: false,
        },
    };
    if adapter {
        // The adapter, not the GPU, sets the TMDS limit, and it doesn't do FRL unless it is a PCON
        // we can't see.
        source.max_tmds_khz = None;
        source.frl = FrlGate::Unknown;
    }
    let req = Request {
        // Drivers without `max bpc` send 8 bits.
        max_bpc: kms
            .max_bpc
            .as_ref()
            .and_then(|v| v.known())
            .copied()
            .unwrap_or(8),
        color_format: kms.color_format.as_ref().and_then(|v| v.known()).copied(),
    };
    let effective = LinkLimits {
        max_tmds_character_rate_khz: match (source.max_tmds_khz, sink.max_tmds_khz) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        },
        max_frl: match (&source.frl, source.max_frl, sink.max_frl) {
            (FrlGate::Enabled, Some(a), Some(b)) => displaycaps_link::budget::common_frl(a, b),
            _ => None,
        },
        max_dp: None,
        dsc: None,
        allm: None,
        max_bpc: Some(req.max_bpc),
    };
    let caps = CapabilitySet {
        link: Some(Value::inferred(effective, prov.clone())),
        ..Default::default()
    };
    let rejected = |v: Vec<displaycaps_link::Rejected>| -> Vec<RejectedFormat> {
        v.into_iter()
            .map(|r| RejectedFormat {
                color_format: r.color_format,
                bit_depth: r.bpc,
                dsc: r.dsc,
                reason: r.reason,
            })
            .collect()
    };
    match displaycaps_link::choose(&mode, kind, &source, &sink, req) {
        Ok(c) => {
            let dp_unknown = matches!(kind, LinkKind::DisplayPort { .. }) && c.capacity_kbps == 0;
            let state = LinkState {
                signal: Value::inferred(c.signal.clone(), prov.clone()),
                color_format: Value::inferred(c.color_format, prov.clone()),
                bit_depth: Value::inferred(c.bpc, prov.clone()),
                range: None,
                dsc: Some(Value::inferred(c.dsc_bpp_x16.is_some(), prov.clone())),
                required_kbps: Value::inferred(c.required_kbps, prov.clone()),
                capacity_kbps: if dp_unknown || kind == LinkKind::Virtual {
                    Value::unknown(UnknownReason::NotExposed, "no link capacity".to_string())
                } else {
                    Value::inferred(c.capacity_kbps, prov.clone())
                },
                limited_by,
                rejected: rejected(c.rejected),
            };
            LayerCapabilities {
                layer: LayerKind::Link,
                availability: Availability::Read,
                caps,
                detail: i.with_detail.then_some(LayerDetail::Link(state)),
            }
        }
        Err(all) => {
            let why = all
                .first()
                .map(|r| r.reason.clone())
                .unwrap_or_else(|| "no format fits".into());
            fn u<T>(d: &str) -> Value<T> {
                Value::unknown(UnknownReason::NotExposed, d.to_string())
            }
            let state = LinkState {
                signal: u(&why),
                color_format: u(&why),
                bit_depth: u(&why),
                range: None,
                dsc: None,
                required_kbps: u(&why),
                capacity_kbps: u(&why),
                limited_by,
                rejected: rejected(all),
            };
            LayerCapabilities {
                layer: LayerKind::Link,
                availability: Availability::Partial,
                caps,
                detail: i.with_detail.then_some(LayerDetail::Link(state)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, v: &str) -> ModuleParameter {
        ModuleParameter {
            name: name.into(),
            value: v.into(),
        }
    }

    fn htpc(mode: (u16, u16, u32), kernel: &str, mask: &str, sim_frl: bool) -> LinkState {
        let edid = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/parity/edid/hisense-55qd5n.bin"
        ))
        .unwrap();
        let sc = Scenario {
            edid,
            mode,
            connector: ConnectorType::Hdmi,
            driver: "amdgpu".into(),
            kernel: kernel.into(),
            params: vec![("amdgpu.dcfeaturemask".into(), mask.into())],
            max_bpc: 10,
            color_format: None,
            sink_frl: sim_frl.then_some(FrlRate {
                lanes: 4,
                gbps_per_lane: 12,
            }),
            sink_vics: if sim_frl { vec![118] } else { vec![] },
        };
        match explain(&sc).unwrap().detail {
            Some(LayerDetail::Link(l)) => l,
            other => panic!("{other:?}"),
        }
    }

    /// Phase 4 gate, simulated (the HTPC is out of scope): the HTPC's TV (its real EDID, HDMI
    /// 2.0) and an RX 9070 XT on amdgpu, with FRL off and on.
    #[test]
    fn htpc_4k_fallbacks_explained() {
        use displaycaps_model::LinkSignal;
        let fmt = |l: &LinkState| {
            (
                *l.color_format.known().unwrap(),
                *l.bit_depth.known().unwrap(),
            )
        };
        // The real TV, 4K60 HDR: 10-bit RGB needs 742.5 MHz; the TV has no FRL whatever the driver
        // does, so the driver drops to 8 bits.
        for kernel in ["7.2.3", "7.4.0"] {
            let l = htpc((3840, 2160, 60_000), kernel, "0x402", false);
            assert_eq!(fmt(&l), (ColorFormat::Rgb, 8));
            assert!(
                l.rejected[0].reason.contains("742.5 MHz")
                    && l.rejected[0].reason.contains("the sink has no FRL")
            );
        }
        // 4K 23.976: 10-bit RGB fits HDMI 2.0.
        let l = htpc((3840, 2160, 23_976), "7.2.3", "0x2", false);
        assert_eq!(fmt(&l), (ColorFormat::Rgb, 10));
        assert!(l.rejected.is_empty());
        // A simulated HDMI 2.1 TV (48 Gbit/s FRL, 4K120): FRL off → 4:2:0 8-bit over TMDS ...
        let l = htpc((3840, 2160, 120_000), "7.2.3", "0x2", true);
        assert_eq!(fmt(&l), (ColorFormat::Ycbcr420, 8));
        assert_eq!(
            l.signal.known(),
            Some(&LinkSignal::Tmds {
                character_rate_khz: 594_000
            })
        );
        assert!(
            l.rejected
                .iter()
                .any(|r| r.reason.contains("dcfeaturemask bit 0x400"))
        );
        // ... FRL on (the mask bit, or Linux 7.4) → RGB 10-bit over FRL.
        for (k, m) in [("7.2.3", "0x402"), ("7.4.0", "0x2")] {
            let l = htpc((3840, 2160, 120_000), k, m, true);
            assert_eq!(fmt(&l), (ColorFormat::Rgb, 10));
            assert!(matches!(l.signal.known(), Some(LinkSignal::Frl { .. })));
            assert!(l.rejected.is_empty());
        }
    }

    #[test]
    fn amdgpu_frl_gate() {
        let d = Some("amdgpu");
        assert_eq!(frl_gate(d, Some((7, 1)), &[]), FrlGate::Unsupported);
        assert!(matches!(
            frl_gate(d, Some((7, 2)), &[p("amdgpu.dcfeaturemask", "2")]),
            FrlGate::DisabledByDriver(_)
        ));
        assert_eq!(
            frl_gate(d, Some((7, 3)), &[p("amdgpu.dcfeaturemask", "1026")]),
            FrlGate::Enabled
        );
        assert_eq!(
            frl_gate(d, Some((7, 3)), &[p("amdgpu.dcfeaturemask", "0x402")]),
            FrlGate::Enabled
        );
        assert_eq!(frl_gate(d, Some((7, 3)), &[]), FrlGate::Unknown);
        assert_eq!(frl_gate(d, Some((7, 4)), &[]), FrlGate::Enabled);
        assert_eq!(
            frl_gate(Some("i915"), Some((7, 4)), &[]),
            FrlGate::Unsupported
        );
        assert_eq!(
            frl_gate(Some("nvidia"), Some((7, 4)), &[]),
            FrlGate::Unknown
        );
        assert_eq!(kernel_version("7.2.3-arch1-1"), Some((7, 2)));
        assert_eq!(kernel_version("6.12.117+deb13-amd64"), Some((6, 12)));
    }
}

/// A link scenario for `displaycaps link` and the phase 4 gate: an EDID, a mode, a connector and
/// a source driver, simulated without hardware. Lab tooling, not part of the API.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct Scenario {
    /// Raw EDID of the sink.
    pub edid: Vec<u8>,
    /// Mode: width, height, refresh in millihertz (matched against the sink's modes ±0.5 Hz).
    pub mode: (u16, u16, u32),
    /// Connector type.
    pub connector: ConnectorType,
    /// Source driver, e.g. `amdgpu`.
    pub driver: String,
    /// Kernel release, e.g. `7.3.0`.
    pub kernel: String,
    /// Module parameters as `module.name = value`.
    pub params: Vec<(String, String)>,
    /// KMS `max bpc`.
    pub max_bpc: u8,
    /// KMS `color format`, `None` for automatic.
    pub color_format: Option<ColorFormat>,
    /// Simulate a sink with this FRL rate (the EDID's own is used otherwise).
    pub sink_frl: Option<FrlRate>,
    /// Simulate a sink that also lists these CTA-861 VICs (4:2:0 allowed for them).
    pub sink_vics: Vec<u8>,
}

/// Works out the link for a [`Scenario`]: the layer, or why the mode isn't in the EDID.
#[doc(hidden)]
pub fn explain(s: &Scenario) -> Result<LayerCapabilities, String> {
    use displaycaps_model::Selection;
    let mut sink = crate::sink::sink_record(
        &s.edid,
        Source::Command {
            name: "displaycaps link".into(),
        },
        0,
        &Selection::All,
        &displaycaps_rules::quirks::QuirkSet::bundled(),
    )
    .ok_or("empty EDID")?;
    if let Some(frl) = s.sink_frl
        && let Some(l) = sink.link.known_mut()
    {
        l.max_frl = Some(frl);
    }
    if let Some(modes) = sink.video.modes.as_mut().and_then(|m| m.known_mut()) {
        for &n in &s.sink_vics {
            let Some(v) = displaycaps_parse::vic::vic(n) else {
                return Err(format!("VIC {n} is not in the CTA-861 table"));
            };
            modes.push(Mode {
                width: v.width,
                height: v.height,
                refresh_millihz: v.refresh_millihz,
                interlaced: v.interlaced,
                timing: Some(displaycaps_model::Timing {
                    pixel_clock_khz: v.pixel_clock_khz,
                    h_total: v.h_total,
                    v_total: v.v_total,
                    h_sync_positive: None,
                    v_sync_positive: None,
                }),
                vic: Some(n),
                hdmi_vic: None,
                preferred: false,
                ycbcr420: Ycbcr420::Also,
                sources: vec![displaycaps_model::ModeSource::CtaVic],
            });
        }
    }
    let (w, h, r) = s.mode;
    let mode = sink
        .video
        .modes
        .as_ref()
        .and_then(|m| m.known())
        .and_then(|modes| {
            modes
                .iter()
                .filter(|m| m.width == w && m.height == h && !m.interlaced && m.timing.is_some())
                .min_by_key(|m| m.refresh_millihz.abs_diff(r))
                .filter(|m| m.refresh_millihz.abs_diff(r) <= 500)
                .cloned()
        })
        .ok_or_else(|| {
            format!(
                "{w}x{h} at {}.{:03} Hz is not in the EDID",
                r / 1000,
                r % 1000
            )
        })?;
    let prov = Provenance {
        layer: LayerKind::Kms,
        source: Source::Command {
            name: "displaycaps link (simulated)".into(),
        },
        read_at_ms: 0,
    };
    let kms = KmsState {
        active_mode: Value::measured(Some(mode), prov.clone()),
        max_bpc: Some(Value::measured(s.max_bpc, prov.clone())),
        colorspace: None,
        color_format: s.color_format.map(|f| Value::measured(f, prov.clone())),
        hdr_output_metadata: None,
        broadcast_rgb: None,
        vrr_capable: None,
        vrr_enabled: None,
        content_type: None,
        link_status: None,
        panel_orientation: None,
    };
    let params: Vec<ModuleParameter> = s
        .params
        .iter()
        .map(|(n, v)| ModuleParameter {
            name: n.clone(),
            value: v.clone(),
        })
        .collect();
    let frl = frl_gate(Some(&s.driver), kernel_version(&s.kernel), &params);
    Ok(link_layer(&Inputs {
        connector: &s.connector,
        sink: Some(&sink),
        kms: Some(&kms),
        mst: false,
        driver: Some(&s.driver),
        frl,
        with_detail: true,
        now: 0,
    }))
}
