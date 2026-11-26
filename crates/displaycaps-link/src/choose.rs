// SPDX-License-Identifier: LGPL-2.1-or-later
//! The format a driver sends for a mode, worked out from what the source, the sink and KMS allow.
//!
//! The policy approximates the drivers:
//! 1. the requested format (KMS `color format`, or RGB when automatic) at the highest depth
//!    `max bpc` and the sink allow;
//! 2. then either DSC at that depth before lower depths (amdgpu: [`SourceLimits::dsc_first`]) or
//!    lower depths before DSC (i915), down to 8 bits on HDMI and 6 on DisplayPort;
//! 3. YCbCr 4:2:0, when the format is automatic and the sink takes 4:2:0 for the mode.
//!
//! On HDMI, TMDS is used when the format fits the TMDS limit of both ends; otherwise FRL at the
//! highest rate both support, if the driver has FRL on. Every option above the chosen one is
//! returned as [`Rejected`] with the reason. An option rejected only because something is unknown
//! (the driver's FRL state, a link rate) is marked [`Rejected::unknown`]: the chosen format is then
//! a lower bound, not the answer ([`Choice::uncertain`]).

use displaycaps_model::{ColorFormat, DpLink, DpRate, FrlRate, LinkSignal, Mode, Timing};

use crate::budget::{
    self, frl_capacity_kbps, frl_required_kbps, link_stream_kbps, tmds_character_rate_khz,
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
    /// FRL could work but the driver has it off. The string says why.
    DisabledByDriver(String),
    /// The driver has no FRL support.
    Unsupported,
    /// Not known.
    Unknown,
}

/// What the source (GPU, driver and KMS) allows. `None` means unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLimits {
    /// Highest TMDS character rate in kHz.
    pub max_tmds_khz: Option<u32>,
    /// Highest FRL rate the hardware supports.
    pub max_frl: Option<FrlRate>,
    /// Whether the driver has FRL on.
    pub frl: FrlGate,
    /// Highest DisplayPort link.
    pub max_dp: Option<DpLink>,
    /// Whether the source can compress with DSC.
    pub dsc: Option<bool>,
    /// The driver turns DSC on at the requested depth before it lowers the depth (amdgpu).
    pub dsc_first: bool,
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
    /// Highest FRL rate with DSC (HDMI).
    pub dsc_max_frl: Option<FrlRate>,
    /// Highest depth DSC carries.
    pub dsc_max_bpc: Option<u8>,
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

    /// Whether a better option was rejected only because something is unknown: the chosen
    /// format is then a lower bound.
    pub fn uncertain(&self) -> bool {
        self.rejected.iter().any(|r| r.unknown)
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
    /// Rejected for lack of knowledge, not because it doesn't fit.
    pub unknown: bool,
}

const DEPTHS: [u8; 5] = [16, 12, 10, 8, 6];

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

/// Why an option failed, and whether only for lack of knowledge.
type Miss = (String, bool);

struct Ctx<'a> {
    mode: &'a Mode,
    t: Timing,
    link: LinkKind,
    source: &'a SourceLimits,
    sink: &'a SinkFormats,
    req: Request,
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
            unknown: true,
        }]);
    };
    let cx = Ctx {
        mode,
        t,
        link,
        source,
        sink,
        req,
    };
    let mut rejected = vec![];
    let first = req.color_format.unwrap_or(ColorFormat::Rgb);
    if let Some(c) = cx.format(first, &mut rejected) {
        return Ok(Choice { rejected, ..c });
    }
    if req.color_format.is_none() {
        let f = ColorFormat::Ycbcr420;
        if sink.depths(f).is_empty() {
            rejected.push(Rejected {
                color_format: f,
                bpc: 8,
                dsc: false,
                reason: "the sink doesn't take YCbCr 4:2:0 for this mode".into(),
                unknown: false,
            });
        } else if let Some(c) = cx.format(f, &mut rejected) {
            return Ok(Choice { rejected, ..c });
        }
    }
    Err(rejected)
}

impl Ctx<'_> {
    fn min_depth(&self) -> u8 {
        match self.link {
            LinkKind::DisplayPort { .. } => 6,
            _ => 8,
        }
    }

    /// One format: depths and DSC in the driver's order.
    fn format(&self, f: ColorFormat, rejected: &mut Vec<Rejected>) -> Option<Choice> {
        let virt = self.link == LinkKind::Virtual;
        let sink_depths = self.sink.depths(f);
        if sink_depths.is_empty() && !virt {
            rejected.push(Rejected {
                color_format: f,
                bpc: self.req.max_bpc,
                dsc: false,
                reason: format!("the sink doesn't take {} for this mode", name(f)),
                unknown: false,
            });
            return None;
        }
        let mut depths: Vec<u8> = DEPTHS
            .iter()
            .copied()
            .filter(|d| {
                *d <= self.req.max_bpc
                    && *d >= self.min_depth()
                    && (virt || sink_depths.contains(d))
            })
            .collect();
        if depths.is_empty() {
            depths.push(
                sink_depths
                    .iter()
                    .copied()
                    .min()
                    .unwrap_or(8)
                    .max(self.min_depth()),
            );
        }
        if let Some(m) = sink_depths.iter().max().copied()
            && m < self.req.max_bpc
            && !virt
        {
            rejected.push(Rejected {
                color_format: f,
                bpc: self.req.max_bpc,
                dsc: false,
                reason: format!("the sink takes {} at {m} bits at most", name(f)),
                unknown: false,
            });
        }
        let top = depths[0];
        let try_one = |bpc: u8, dsc: bool, rejected: &mut Vec<Rejected>| -> Option<Choice> {
            match self.fits(f, bpc, dsc) {
                Ok(c) => Some(c),
                Err((reason, unknown)) => {
                    rejected.push(Rejected {
                        color_format: f,
                        bpc,
                        dsc,
                        reason,
                        unknown,
                    });
                    None
                }
            }
        };
        if let Some(c) = try_one(top, false, rejected) {
            return Some(c);
        }
        // DSC only when both ends are known to do it; an unknown source is a rejection for lack
        // of knowledge below.
        let dsc_possible = !virt && self.source.dsc == Some(true) && self.sink.dsc;
        if self.source.dsc_first
            && dsc_possible
            && let Some(c) = try_one(top, true, rejected)
        {
            return Some(c);
        }
        for &bpc in &depths[1..] {
            if let Some(c) = try_one(bpc, false, rejected) {
                return Some(c);
            }
        }
        if !self.source.dsc_first || !dsc_possible {
            if dsc_possible {
                if let Some(c) = try_one(top, true, rejected) {
                    return Some(c);
                }
            } else if !virt {
                rejected.push(Rejected {
                    color_format: f,
                    bpc: top,
                    dsc: true,
                    reason: format!(
                        "DSC needs both ends: source {}, sink {}",
                        match self.source.dsc {
                            Some(true) => "yes",
                            Some(false) => "no",
                            None => "unknown",
                        },
                        if self.sink.dsc { "yes" } else { "no" }
                    ),
                    unknown: self.source.dsc.is_none() && self.sink.dsc,
                });
            }
        }
        None
    }

    fn base(
        &self,
        signal: LinkSignal,
        f: ColorFormat,
        bpc: u8,
        required: u64,
        capacity: u64,
        dsc: Option<u32>,
    ) -> Choice {
        Choice {
            signal,
            color_format: f,
            bpc,
            dsc_bpp_x16: dsc,
            required_kbps: required,
            capacity_kbps: capacity,
            rejected: vec![],
        }
    }

    fn fits(&self, f: ColorFormat, bpc: u8, dsc: bool) -> Result<Choice, Miss> {
        let t = &self.t;
        let px = t.pixel_clock_khz;
        match self.link {
            LinkKind::Virtual => Ok(self.base(
                LinkSignal::Virtual,
                f,
                bpc,
                budget::stream_kbps(px, f, bpc),
                0,
                None,
            )),
            LinkKind::Hdmi => self.hdmi(f, bpc, dsc),
            LinkKind::DisplayPort { mst } => {
                let dp = match (self.source.max_dp, self.sink.max_dp) {
                    (Some(a), Some(b)) => DpLink {
                        lanes: a.lanes.min(b.lanes),
                        rate: a.rate.min(b.rate),
                    },
                    _ => return Err(("the DisplayPort link rate is unknown".into(), true)),
                };
                let mut cap = budget::dp_capacity_kbps(dp, mst);
                let signal = LinkSignal::DisplayPort { link: dp };
                if dsc {
                    // DSC on 8b/10b links needs FEC (about 2.4%).
                    if !matches!(dp.rate, DpRate::Uhbr10 | DpRate::Uhbr13_5 | DpRate::Uhbr20) {
                        cap = cap * 976 / 1000;
                    }
                    let bpc = self.sink.dsc_max_bpc.map_or(bpc, |m| bpc.min(m));
                    return match budget::dsc_bpp_x16(px, cap, f, bpc) {
                        Some(bpp) => Ok(self.base(
                            signal,
                            f,
                            bpc,
                            u64::from(px) * u64::from(bpp) / 16,
                            cap,
                            Some(bpp),
                        )),
                        None => Err((
                            format!("{} too slow even with DSC at 8 bpp", gbps(cap)),
                            false,
                        )),
                    };
                }
                let need = link_stream_kbps(px, f, bpc, false);
                if need <= cap {
                    Ok(self.base(signal, f, bpc, need, cap, None))
                } else {
                    Err((
                        format!(
                            "DisplayPort {}×{:?} carries {}, needs {}",
                            dp.lanes,
                            dp.rate,
                            gbps(cap),
                            gbps(need)
                        ),
                        false,
                    ))
                }
            }
        }
    }

    fn hdmi(&self, f: ColorFormat, bpc: u8, dsc: bool) -> Result<Choice, Miss> {
        let (t, source, sink) = (&self.t, self.source, self.sink);
        let mut why = vec![];
        let mut unknown = false;
        if !dsc {
            let limit = match (source.max_tmds_khz, sink.max_tmds_khz) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            match (tmds_character_rate_khz(t.pixel_clock_khz, f, bpc), limit) {
                (Some(rate), Some(limit)) if rate <= limit => {
                    return Ok(self.base(
                        LinkSignal::Tmds {
                            character_rate_khz: rate,
                        },
                        f,
                        bpc,
                        budget::tmds_capacity_kbps(rate),
                        budget::tmds_capacity_kbps(limit),
                        None,
                    ));
                }
                (Some(rate), Some(limit)) => {
                    why.push(format!("TMDS needs {} (limit {})", mhz(rate), mhz(limit)));
                }
                (None, _) => why.push(format!("TMDS can't carry {} at {bpc} bits", name(f))),
                (Some(_), None) => {
                    why.push("the TMDS limit is unknown".into());
                    unknown = true;
                }
            }
        }
        // FRL.
        let frl_miss = match (&source.frl, sink.max_frl, source.max_frl) {
            (_, None, _) => Some(("the sink has no FRL".to_string(), false)),
            (FrlGate::Unsupported, ..) => Some(("the driver has no FRL support".into(), false)),
            (FrlGate::DisabledByDriver(w), ..) => {
                Some((format!("FRL is off in the driver ({w})"), false))
            }
            (FrlGate::Unknown, ..) => {
                Some(("whether the driver has FRL on is unknown".into(), true))
            }
            (FrlGate::Enabled, _, None) => {
                Some(("the GPU's highest FRL rate is unknown".into(), true))
            }
            (FrlGate::Enabled, Some(_), Some(_)) => None,
        };
        if let Some((w, u)) = frl_miss {
            why.push(w);
            return Err((why.join("; "), unknown || u));
        }
        let mut rate = budget::common_frl(
            source.max_frl.expect("checked"),
            sink.max_frl.expect("checked"),
        )
        .ok_or(("no FRL rate common to both ends".to_string(), false))?;
        if dsc {
            if let Some(d) = sink.dsc_max_frl {
                rate = budget::common_frl(rate, d).unwrap_or(rate);
            }
            let cap = frl_capacity_kbps(rate);
            let bpc = sink.dsc_max_bpc.map_or(bpc, |m| bpc.min(m));
            let px = t.pixel_clock_khz;
            return match budget::dsc_bpp_x16(px, cap, f, bpc) {
                Some(bpp) => Ok(self.base(
                    LinkSignal::Frl { rate },
                    f,
                    bpc,
                    u64::from(px) * u64::from(bpp) / 16,
                    cap,
                    Some(bpp),
                )),
                None => Err((
                    format!("{} too slow even with DSC at 8 bpp", frl_name(rate)),
                    false,
                )),
            };
        }
        let cap = frl_capacity_kbps(rate);
        let need = frl_required_kbps(t, self.mode.width, self.mode.height, f, bpc);
        if need <= cap {
            Ok(self.base(LinkSignal::Frl { rate }, f, bpc, need, cap, None))
        } else {
            why.push(format!(
                "{} carries {}, needs {}",
                frl_name(rate),
                gbps(cap),
                gbps(need)
            ));
            Err((why.join("; "), false))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ColorFormat::*;
    use displaycaps_model::{ModeSource, Ycbcr420};

    fn mode(w: u16, h: u16, clock: u32, ht: u16, vt: u16, refresh: u32) -> Mode {
        Mode {
            width: w,
            height: h,
            refresh_millihz: refresh,
            interlaced: false,
            timing: Some(Timing {
                pixel_clock_khz: clock,
                h_total: ht,
                v_total: vt,
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

    fn mode4k(clock: u32, refresh: u32) -> Mode {
        mode(3840, 2160, clock, 4400, 2250, refresh)
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
            dsc: Some(true),
            dsc_first: true,
        }
    }

    fn auto(max_bpc: u8) -> Request {
        Request {
            max_bpc,
            color_format: None,
        }
    }

    #[test]
    fn hdmi20_4k60_hdr_drops_depth_not_chroma() {
        // RGB 10 needs 742.5 MHz; an automatic driver lowers the depth before it subsamples:
        // RGB 8 at 594 MHz.
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
        assert!(!c.uncertain());
        // Forcing 4:2:0 (color format, Linux 7.3+) keeps 10 bits; 4:2:2 12-bit fits too.
        let forced = |f, bpc| {
            choose(
                &mode4k(594_000, 60_000),
                LinkKind::Hdmi,
                &amd(FrlGate::Enabled),
                &hdmi20_tv(),
                Request {
                    max_bpc: bpc,
                    color_format: Some(f),
                },
            )
            .unwrap()
        };
        let c = forced(Ycbcr420, 10);
        assert_eq!((c.color_format, c.bpc), (Ycbcr420, 10));
        let c = forced(Ycbcr422, 12);
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
        assert!(!c.uncertain());
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
    fn unknown_frl_makes_the_answer_a_lower_bound() {
        let c = choose(
            &mode4k(1_188_000, 120_000),
            LinkKind::Hdmi,
            &amd(FrlGate::Unknown),
            &hdmi21_tv(),
            auto(10),
        )
        .unwrap();
        assert_eq!((c.color_format, c.bpc), (Ycbcr420, 8));
        assert!(
            c.uncertain(),
            "RGB over FRL was rejected only for lack of knowledge"
        );
        let mut src = amd(FrlGate::Enabled);
        src.max_frl = None;
        let c = choose(
            &mode4k(1_188_000, 120_000),
            LinkKind::Hdmi,
            &src,
            &hdmi21_tv(),
            auto(10),
        )
        .unwrap();
        assert!(c.uncertain());
    }

    #[test]
    fn nothing_fits_explains_every_option() {
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
    fn dsc_order_follows_the_driver() {
        // A 4K120 CVT-RB2 monitor (~1046 MHz) on 4×HBR3: 10-bit needs 31.4 Gbit/s, 8-bit 25.1.
        let hbr3 = DpLink {
            lanes: 4,
            rate: DpRate::Hbr3,
        };
        let mut m = mode(3840, 2160, 1_046_000, 3920, 2222, 120_000);
        m.ycbcr420 = Ycbcr420::No;
        let sink = SinkFormats {
            max_dp: Some(hbr3),
            dsc: true,
            ..hdmi20_tv()
        };
        let amdgpu = SourceLimits {
            max_dp: Some(hbr3),
            ..amd(FrlGate::Unsupported)
        };
        let c = choose(
            &m,
            LinkKind::DisplayPort { mst: false },
            &amdgpu,
            &sink,
            auto(10),
        )
        .unwrap();
        assert_eq!(
            (c.color_format, c.bpc),
            (Rgb, 10),
            "amdgpu: DSC at the requested depth"
        );
        assert!(c.dsc_bpp_x16.is_some());
        let i915 = SourceLimits {
            dsc_first: false,
            ..amdgpu
        };
        let c = choose(
            &m,
            LinkKind::DisplayPort { mst: false },
            &i915,
            &sink,
            auto(10),
        )
        .unwrap();
        assert_eq!(
            (c.color_format, c.bpc, c.dsc_bpp_x16),
            (Rgb, 8, None),
            "i915: depth first"
        );
    }

    #[test]
    fn six_bit_panels_and_virtual() {
        let hbr2 = DpLink {
            lanes: 2,
            rate: DpRate::Hbr2,
        };
        let panel = SinkFormats {
            formats: vec![(Rgb, vec![6, 8])],
            max_dp: Some(hbr2),
            ..Default::default()
        };
        let src = SourceLimits {
            max_dp: Some(hbr2),
            dsc: Some(false),
            dsc_first: false,
            ..amd(FrlGate::Unsupported)
        };
        let m = mode(1920, 1080, 138_500, 2080, 1111, 60_000);
        let c = choose(
            &m,
            LinkKind::DisplayPort { mst: false },
            &src,
            &panel,
            auto(6),
        )
        .unwrap();
        assert_eq!(c.bpc, 6);
        let v = choose(
            &mode4k(594_000, 60_000),
            LinkKind::Virtual,
            &src,
            &SinkFormats::default(),
            auto(8),
        )
        .unwrap();
        assert_eq!(v.signal, LinkSignal::Virtual);
        let e = choose(
            &m,
            LinkKind::DisplayPort { mst: false },
            &SourceLimits {
                max_dp: None,
                ..src
            },
            &panel,
            auto(8),
        )
        .unwrap_err();
        // Every RGB option fails for lack of knowledge, none for bandwidth.
        assert!(
            e.iter()
                .filter(|r| r.color_format == Rgb && !r.dsc)
                .all(|r| r.unknown)
        );
    }
}
