// SPDX-License-Identifier: LGPL-2.1-or-later
//! The format a driver sends for a mode, worked out from what the source, the sink and KMS allow.
//!
//! The policy approximates what amdgpu and i915 do when they pick a link format:
//! 1. the requested format (KMS `color format`, or RGB when it's automatic) at the highest depth
//!    `max bpc` and the sink allow, then lower depths in steps of 2 down to 8;
//! 2. DSC at the highest depth, when both ends support it (FRL or DisplayPort);
//! 3. YCbCr 4:2:0, when the format is automatic and the sink takes 4:2:0 for the mode, at the
//!    highest depth that fits.
//!
//! On HDMI, TMDS is used when the format fits the TMDS limit of both ends; otherwise FRL at the
//! highest rate both support, if the driver has FRL on. Every option above the chosen one is
//! returned as [`Rejected`] with the reason, which is what explains a fallback.

use displaycaps_model::{ColorFormat, DpLink, FrlRate, LinkSignal, Mode};

use crate::budget::{
    self, frl_capacity_kbps, frl_required_kbps, stream_kbps, tmds_character_rate_khz,
};

/// The kind of link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// HDMI (TMDS, and FRL when both ends and the driver allow it).
    Hdmi,
    /// DisplayPort (SST, or MST through a hub or dock).
    DisplayPort {
        /// Through an MST hub or dock.
        mst: bool,
    },
    /// A virtual output: no wire, no limits.
    Virtual,
}

/// Whether the driver has HDMI FRL on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrlGate {
    /// FRL is on.
    Enabled,
    /// The hardware or driver could, but FRL is off (e.g. amdgpu before Linux 7.4 without the
    /// feature-mask bit). The string says why.
    DisabledByDriver(String),
    /// The driver has no FRL support.
    Unsupported,
    /// Not known.
    Unknown,
}

/// What the source (GPU, driver and KMS) allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLimits {
    /// Highest TMDS character rate in kHz; `None` when unknown (the sink's limit is used alone).
    pub max_tmds_khz: Option<u32>,
    /// Highest FRL rate the hardware supports.
    pub max_frl: Option<FrlRate>,
    /// Whether the driver has FRL on.
    pub frl: FrlGate,
    /// Highest DisplayPort link; `None` when unknown (the sink's limit is used alone).
    pub max_dp: Option<DpLink>,
    /// Whether the source can compress with DSC.
    pub dsc: bool,
}

/// What the sink accepts for one mode.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SinkFormats {
    /// Depths per format for this mode (4:2:0 only if the sink takes 4:2:0 for the mode; RGB and
    /// 4:4:4 absent for a 4:2:0-only mode).
    pub formats: Vec<(ColorFormat, Vec<u8>)>,
    /// Highest TMDS character rate in kHz.
    pub max_tmds_khz: Option<u32>,
    /// Highest FRL rate.
    pub max_frl: Option<FrlRate>,
    /// Highest DisplayPort link.
    pub max_dp: Option<DpLink>,
    /// Whether the sink decodes DSC.
    pub dsc: bool,
}

impl SinkFormats {
    fn depths(&self, f: ColorFormat) -> &[u8] {
        self.formats
            .iter()
            .find(|(x, _)| *x == f)
            .map(|(_, d)| d.as_slice())
            .unwrap_or(&[])
    }
}

/// What KMS asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// `max bpc`.
    pub max_bpc: u8,
    /// `color format`, when forced (Linux 7.3+); `None` is automatic.
    pub color_format: Option<ColorFormat>,
}

/// The format the link carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The signal and its rate.
    pub signal: LinkSignal,
    /// Colour format on the wire.
    pub color_format: ColorFormat,
    /// Bits per component.
    pub bpc: u8,
    /// DSC rate in 1/16 bit per pixel, when compressed.
    pub dsc_bpp_x16: Option<u32>,
    /// Data rate needed, kbit/s.
    pub required_kbps: u64,
    /// Data rate the link carries, kbit/s.
    pub capacity_kbps: u64,
    /// Better options that were rejected, best first.
    pub rejected: Vec<Rejected>,
}

impl Choice {
    /// Spare capacity in 1/1000 of the link.
    pub fn headroom_permille(&self) -> u32 {
        if self.capacity_kbps == 0 {
            return 0;
        }
        u32::try_from(
            self.capacity_kbps.saturating_sub(self.required_kbps) * 1000 / self.capacity_kbps,
        )
        .unwrap_or(1000)
    }
}

/// An option that was not taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    /// Colour format tried.
    pub color_format: ColorFormat,
    /// Depth tried.
    pub bpc: u8,
    /// With DSC.
    pub dsc: bool,
    /// Why it was not taken.
    pub reason: String,
}

const DEPTHS: [u8; 4] = [16, 12, 10, 8];

fn name(f: ColorFormat) -> &'static str {
    match f {
        ColorFormat::Rgb => "RGB",
        ColorFormat::Ycbcr444 => "YCbCr 4:4:4",
        ColorFormat::Ycbcr422 => "YCbCr 4:2:2",
        ColorFormat::Ycbcr420 => "YCbCr 4:2:0",
    }
}

fn gbps(kbps: u64) -> String {
    format!(
        "{}.{:02} Gbit/s",
        kbps / 1_000_000,
        kbps % 1_000_000 / 10_000
    )
}

fn mhz(khz: u32) -> String {
    format!("{}.{} MHz", khz / 1000, khz % 1000 / 100)
}

fn frl_name(r: FrlRate) -> String {
    format!("FRL {}×{} Gbit/s", r.lanes, r.gbps_per_lane)
}

/// The best format for `mode`, or every option with its reason when nothing fits.
pub fn choose(
    mode: &Mode,
    link: LinkKind,
    source: &SourceLimits,
    sink: &SinkFormats,
    req: Request,
) -> Result<Choice, Vec<Rejected>> {
    let Some(t) = mode.timing else {
        return Err(vec![Rejected {
            color_format: req.color_format.unwrap_or(ColorFormat::Rgb),
            bpc: req.max_bpc,
            dsc: false,
            reason: "the mode has no timing (pixel clock unknown)".into(),
        }]);
    };
    let mut rejected = vec![];
    let formats: Vec<ColorFormat> = match req.color_format {
        Some(f) => vec![f],
        None => vec![ColorFormat::Rgb],
    };
    // 1. The requested format, depth by depth.
    for &f in &formats {
        if let Some(c) = try_depths(mode, &t, link, source, sink, req, f, false, &mut rejected) {
            return Ok(Choice { rejected, ..c });
        }
    }
    // 2. DSC.
    for &f in &formats {
        if let Some(c) = try_depths(mode, &t, link, source, sink, req, f, true, &mut rejected) {
            return Ok(Choice { rejected, ..c });
        }
    }
    // 3. 4:2:0 when the format is automatic.
    if req.color_format.is_none() {
        let f = ColorFormat::Ycbcr420;
        if sink.depths(f).is_empty() {
            rejected.push(Rejected {
                color_format: f,
                bpc: 8,
                dsc: false,
                reason: "the sink doesn't take YCbCr 4:2:0 for this mode".into(),
            });
        } else if let Some(c) =
            try_depths(mode, &t, link, source, sink, req, f, false, &mut rejected)
        {
            return Ok(Choice { rejected, ..c });
        }
    }
    Err(rejected)
}

#[allow(clippy::too_many_arguments)]
fn try_depths(
    mode: &Mode,
    t: &displaycaps_model::Timing,
    link: LinkKind,
    source: &SourceLimits,
    sink: &SinkFormats,
    req: Request,
    f: ColorFormat,
    dsc: bool,
    rejected: &mut Vec<Rejected>,
) -> Option<Choice> {
    let sink_depths = sink.depths(f);
    if sink_depths.is_empty() && link != LinkKind::Virtual {
        rejected.push(Rejected {
            color_format: f,
            bpc: 8,
            dsc,
            reason: format!("the sink doesn't take {} for this mode", name(f)),
        });
        return None;
    }
    if dsc && !(source.dsc && sink.dsc) {
        rejected.push(Rejected {
            color_format: f,
            bpc: req.max_bpc,
            dsc,
            reason: format!(
                "DSC needs both ends: source {}, sink {}",
                if source.dsc { "yes" } else { "no" },
                if sink.dsc { "yes" } else { "no" }
            ),
        });
        return None;
    }
    let mut depths: Vec<u8> = DEPTHS
        .iter()
        .copied()
        .filter(|d| *d <= req.max_bpc && (link == LinkKind::Virtual || sink_depths.contains(d)))
        .collect();
    if depths.is_empty() {
        depths.push(8);
    }
    // With DSC only the highest depth is tried: compression takes the place of depth reduction.
    if dsc {
        depths.truncate(1);
    }
    // Depth capped by the sink rather than by bandwidth: say so before the depths tried.
    let sink_max = sink_depths.iter().max().copied();
    if !dsc
        && let Some(m) = sink_max
        && m < req.max_bpc
        && link != LinkKind::Virtual
    {
        rejected.push(Rejected {
            color_format: f,
            bpc: req.max_bpc,
            dsc,
            reason: format!("the sink takes {} at {m} bits at most", name(f)),
        });
    }
    for &bpc in &depths {
        match fits(mode, t, link, source, sink, f, bpc, dsc) {
            Ok(c) => return Some(c),
            Err(reason) => rejected.push(Rejected {
                color_format: f,
                bpc,
                dsc,
                reason,
            }),
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn fits(
    mode: &Mode,
    t: &displaycaps_model::Timing,
    link: LinkKind,
    source: &SourceLimits,
    sink: &SinkFormats,
    f: ColorFormat,
    bpc: u8,
    dsc: bool,
) -> Result<Choice, String> {
    let base = |signal, required, capacity, dsc_bpp| Choice {
        signal,
        color_format: f,
        bpc,
        dsc_bpp_x16: dsc_bpp,
        required_kbps: required,
        capacity_kbps: capacity,
        rejected: vec![],
    };
    match link {
        LinkKind::Virtual => Ok(base(
            LinkSignal::Virtual,
            stream_kbps(t.pixel_clock_khz, f, bpc),
            0,
            None,
        )),
        LinkKind::Hdmi => {
            let tmds_limit = match (source.max_tmds_khz, sink.max_tmds_khz) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let mut why = vec![];
            if !dsc {
                match (
                    tmds_character_rate_khz(t.pixel_clock_khz, f, bpc),
                    tmds_limit,
                ) {
                    (Some(rate), Some(limit)) if rate <= limit => {
                        return Ok(base(
                            LinkSignal::Tmds {
                                character_rate_khz: rate,
                            },
                            budget::tmds_capacity_kbps(rate),
                            budget::tmds_capacity_kbps(limit),
                            None,
                        ));
                    }
                    (Some(rate), Some(limit)) => {
                        why.push(format!("TMDS needs {} (limit {})", mhz(rate), mhz(limit)))
                    }
                    (None, _) => why.push(format!("TMDS can't carry {} at {bpc} bits", name(f))),
                    (Some(_), None) => why.push("TMDS limit unknown".into()),
                }
            }
            // FRL.
            let common = match (source.max_frl, sink.max_frl) {
                (Some(a), Some(b)) => budget::common_frl(a, b),
                _ => None,
            };
            let frl_why = match (&source.frl, common, sink.max_frl) {
                (_, _, None) => Some("the sink has no FRL".to_string()),
                (FrlGate::Unsupported, ..) => Some("the driver has no FRL support".into()),
                (FrlGate::DisabledByDriver(w), ..) => {
                    Some(format!("FRL is off in the driver ({w})"))
                }
                (FrlGate::Unknown, ..) => Some("whether the driver has FRL on is unknown".into()),
                (FrlGate::Enabled, None, _) => Some("no FRL rate common to both ends".into()),
                (FrlGate::Enabled, Some(_), _) => None,
            };
            if let Some(w) = frl_why {
                why.push(w);
                return Err(why.join("; "));
            }
            let rate = common.expect("checked above");
            let cap = frl_capacity_kbps(rate);
            let need = frl_required_kbps(t, mode.width, mode.height, f, bpc);
            if dsc {
                let px = t.pixel_clock_khz;
                return match budget::dsc_bpp_x16(px, cap, f, bpc) {
                    Some(bpp) => Ok(base(
                        LinkSignal::Frl { rate },
                        u64::from(px) * u64::from(bpp) / 16,
                        cap,
                        Some(bpp),
                    )),
                    None => Err(format!(
                        "{} too slow even with DSC at 8 bpp",
                        frl_name(rate)
                    )),
                };
            }
            if need <= cap {
                Ok(base(LinkSignal::Frl { rate }, need, cap, None))
            } else {
                why.push(format!(
                    "{} carries {}, needs {}",
                    frl_name(rate),
                    gbps(cap),
                    gbps(need)
                ));
                Err(why.join("; "))
            }
        }
        LinkKind::DisplayPort { mst } => {
            let dp = match (source.max_dp, sink.max_dp) {
                (Some(a), Some(b)) => Some(DpLink {
                    lanes: a.lanes.min(b.lanes),
                    rate: a.rate.min(b.rate),
                }),
                (a, b) => a.or(b),
            };
            let Some(dp) = dp else {
                return Err("DisplayPort link rate unknown".into());
            };
            let cap = budget::dp_capacity_kbps(dp, mst);
            let signal = LinkSignal::DisplayPort { link: dp };
            if dsc {
                let px = t.pixel_clock_khz;
                return match budget::dsc_bpp_x16(px, cap, f, bpc) {
                    Some(bpp) => Ok(base(
                        signal,
                        u64::from(px) * u64::from(bpp) / 16,
                        cap,
                        Some(bpp),
                    )),
                    None => Err(format!("{} too slow even with DSC at 8 bpp", gbps(cap))),
                };
            }
            let need = stream_kbps(t.pixel_clock_khz, f, bpc);
            if need <= cap {
                Ok(base(signal, need, cap, None))
            } else {
                Err(format!(
                    "DisplayPort {}×{:?} carries {}, needs {}",
                    dp.lanes,
                    dp.rate,
                    gbps(cap),
                    gbps(need)
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ColorFormat::*;
    use displaycaps_model::{ModeSource, Timing, Ycbcr420};

    fn mode4k(clock: u32, refresh: u32) -> Mode {
        Mode {
            width: 3840,
            height: 2160,
            refresh_millihz: refresh,
            interlaced: false,
            timing: Some(Timing {
                pixel_clock_khz: clock,
                h_total: 4400,
                v_total: 2250,
                h_sync_positive: Some(true),
                v_sync_positive: Some(true),
            }),
            vic: None,
            hdmi_vic: None,
            preferred: false,
            ycbcr420: Ycbcr420::Also,
            sources: vec![ModeSource::CtaVic],
        }
    }

    /// An HDMI 2.0 TV (600 MHz, deep colour 10/12, 4:2:0 10/12) like the HTPC's Hisense.
    fn hdmi20_tv() -> SinkFormats {
        SinkFormats {
            formats: vec![
                (Rgb, vec![8, 10, 12]),
                (Ycbcr444, vec![8, 10, 12]),
                (Ycbcr422, vec![8, 10, 12]),
                (Ycbcr420, vec![8, 10, 12]),
            ],
            max_tmds_khz: Some(600_000),
            ..Default::default()
        }
    }

    /// An HDMI 2.1 TV with 48 Gbit/s FRL.
    fn hdmi21_tv() -> SinkFormats {
        SinkFormats {
            max_frl: Some(FrlRate {
                lanes: 4,
                gbps_per_lane: 12,
            }),
            ..hdmi20_tv()
        }
    }

    fn amd(frl: FrlGate) -> SourceLimits {
        SourceLimits {
            max_tmds_khz: Some(600_000),
            max_frl: Some(FrlRate {
                lanes: 4,
                gbps_per_lane: 12,
            }),
            frl,
            max_dp: None,
            dsc: true,
        }
    }

    fn auto(max_bpc: u8) -> Request {
        Request {
            max_bpc,
            color_format: None,
        }
    }

    #[test]
    fn hdmi20_4k60_hdr_falls_back_to_420_10bit() {
        // RGB 10 and 8... RGB 8 fits 594 MHz, so an automatic driver picks RGB 8 (bit depth lost).
        let c = choose(
            &mode4k(594_000, 60_000),
            LinkKind::Hdmi,
            &amd(FrlGate::Enabled),
            &hdmi20_tv(),
            auto(10),
        )
        .unwrap();
        assert_eq!((c.color_format, c.bpc), (Rgb, 8));
        assert_eq!(
            c.signal,
            LinkSignal::Tmds {
                character_rate_khz: 594_000
            }
        );
        let r = &c.rejected[0];
        assert_eq!((r.color_format, r.bpc), (Rgb, 10));
        assert!(
            r.reason.contains("TMDS needs 742.5 MHz (limit 600.0 MHz)"),
            "{}",
            r.reason
        );
        assert!(r.reason.contains("the sink has no FRL"), "{}", r.reason);
        // Forcing 4:2:0 (color format, Linux 7.3+) keeps 10 bits.
        let c = choose(
            &mode4k(594_000, 60_000),
            LinkKind::Hdmi,
            &amd(FrlGate::Enabled),
            &hdmi20_tv(),
            Request {
                max_bpc: 10,
                color_format: Some(Ycbcr420),
            },
        )
        .unwrap();
        assert_eq!((c.color_format, c.bpc), (Ycbcr420, 10));
        // 4:2:2 12-bit fits at 594 MHz too.
        let c = choose(
            &mode4k(594_000, 60_000),
            LinkKind::Hdmi,
            &amd(FrlGate::Enabled),
            &hdmi20_tv(),
            Request {
                max_bpc: 12,
                color_format: Some(Ycbcr422),
            },
        )
        .unwrap();
        assert_eq!((c.color_format, c.bpc), (Ycbcr422, 12));
    }

    #[test]
    fn film_rate_fits_444_10bit_on_hdmi20() {
        let c = choose(
            &mode4k(296_703, 23_976),
            LinkKind::Hdmi,
            &amd(FrlGate::Enabled),
            &hdmi20_tv(),
            auto(10),
        )
        .unwrap();
        assert_eq!((c.color_format, c.bpc), (Rgb, 10));
        assert!(c.rejected.is_empty());
    }

    #[test]
    fn hdmi21_4k120_frl_off_falls_to_420_8bit() {
        // The CachyOS case: FRL off by default → 4K120 only as 4:2:0 8-bit over TMDS.
        let off =
            FrlGate::DisabledByDriver("amdgpu before Linux 7.4 without dcfeaturemask 0x400".into());
        let c = choose(
            &mode4k(1_188_000, 120_000),
            LinkKind::Hdmi,
            &amd(off),
            &hdmi21_tv(),
            auto(10),
        )
        .unwrap();
        assert_eq!((c.color_format, c.bpc), (Ycbcr420, 8));
        assert_eq!(
            c.signal,
            LinkSignal::Tmds {
                character_rate_khz: 594_000
            }
        );
        assert!(
            c.rejected
                .iter()
                .any(|r| r.reason.contains("FRL is off in the driver"))
        );
        // FRL on: RGB 10-bit over FRL 4×12.
        let c = choose(
            &mode4k(1_188_000, 120_000),
            LinkKind::Hdmi,
            &amd(FrlGate::Enabled),
            &hdmi21_tv(),
            auto(10),
        )
        .unwrap();
        assert_eq!((c.color_format, c.bpc), (Rgb, 10));
        assert_eq!(
            c.signal,
            LinkSignal::Frl {
                rate: FrlRate {
                    lanes: 4,
                    gbps_per_lane: 12
                }
            }
        );
        assert!(c.headroom_permille() > 0);
    }

    #[test]
    fn nothing_fits_explains_every_option() {
        // 4K120 on an HDMI 2.0 TV without 4:2:0 for the mode: nothing.
        let mut tv = hdmi20_tv();
        tv.formats.retain(|(f, _)| *f != Ycbcr420);
        let e = choose(
            &mode4k(1_188_000, 120_000),
            LinkKind::Hdmi,
            &amd(FrlGate::Enabled),
            &tv,
            auto(8),
        )
        .unwrap_err();
        assert!(
            e.iter()
                .any(|r| r.reason.contains("doesn't take YCbCr 4:2:0"))
        );
        assert!(
            e.iter()
                .any(|r| r.dsc && r.reason.contains("DSC needs both ends"))
        );
    }

    #[test]
    fn dp_dsc_and_virtual() {
        let hbr3 = DpLink {
            lanes: 4,
            rate: displaycaps_model::DpRate::Hbr3,
        };
        let src = SourceLimits {
            max_dp: Some(hbr3),
            dsc: true,
            ..amd(FrlGate::Unsupported)
        };
        let sink = SinkFormats {
            max_dp: Some(hbr3),
            dsc: true,
            ..hdmi20_tv()
        };
        let mut m = mode4k(1_306_000, 144_000);
        m.ycbcr420 = Ycbcr420::No;
        let c = choose(
            &m,
            LinkKind::DisplayPort { mst: false },
            &src,
            &sink,
            auto(10),
        )
        .unwrap();
        assert_eq!((c.color_format, c.bpc), (Rgb, 10));
        assert_eq!(c.dsc_bpp_x16.map(|b| b / 16), Some(19));
        let v = choose(
            &mode4k(594_000, 60_000),
            LinkKind::Virtual,
            &src,
            &SinkFormats::default(),
            auto(8),
        )
        .unwrap();
        assert_eq!(v.signal, LinkSignal::Virtual);
    }
}
