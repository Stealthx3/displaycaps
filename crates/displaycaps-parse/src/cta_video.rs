// SPDX-License-Identifier: LGPL-2.1-or-later
//! CTA-861 video data blocks: video data blocks (SVDs), YCbCr 4:2:0 video and capability map
//! blocks, video capability, colorimetry, HDR static and dynamic metadata, the HDMI VSDB's video
//! part (deep colour, HDMI VICs), the full HDMI Forum VSDB and Sink Capability Data Structure,
//! Dolby Vision and HDR10+ vendor-specific video blocks, the AMD FreeSync VSDB, and the detailed
//! timings in the block. Pure functions over bytes; never panics on any input.

use crate::timing::DetailedTiming;

const OUI_HDMI: u32 = 0x000C03;
const OUI_HDMI_FORUM: u32 = 0xC4_5DD8;
const OUI_AMD: u32 = 0x00001A;
const OUI_DOLBY: u32 = 0x00D046;
const OUI_HDR10PLUS: u32 = 0x90_848B;

/// A Short Video Descriptor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Svd {
    /// CTA-861 video identification code (1..=255; native flag removed).
    pub vic: u8,
    /// Native format (CTA-861-F rule: codes 129..=192 are VIC 1..=64 flagged native).
    pub native: bool,
}

impl Svd {
    /// Decodes one SVD byte.
    pub fn from_byte(b: u8) -> Svd {
        if (129..=192).contains(&b) {
            Svd {
                vic: b & 0x7F,
                native: true,
            }
        } else {
            Svd {
                vic: b,
                native: false,
            }
        }
    }
}

/// Video capability data block (extended tag 0).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VideoCapability {
    /// YCC quantization range selectable (QY).
    pub qy: bool,
    /// RGB quantization range selectable (QS).
    pub qs: bool,
    /// Scan behaviour of PT formats (0 none, 1 over, 2 under, 3 both).
    pub s_pt: u8,
    /// Scan behaviour of IT formats.
    pub s_it: u8,
    /// Scan behaviour of CE formats.
    pub s_ce: u8,
}

/// Colorimetry data block (extended tag 5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Colorimetry {
    /// xvYCC601.
    pub xvycc601: bool,
    /// xvYCC709.
    pub xvycc709: bool,
    /// sYCC601.
    pub sycc601: bool,
    /// opYCC601.
    pub opycc601: bool,
    /// opRGB (Adobe RGB).
    pub oprgb: bool,
    /// BT.2020 cYCC.
    pub bt2020_cycc: bool,
    /// BT.2020 YCC.
    pub bt2020_ycc: bool,
    /// BT.2020 RGB.
    pub bt2020_rgb: bool,
    /// Gamut boundary description metadata profiles MD0..MD3 (bit 0 is profile P0).
    pub metadata_profiles: u8,
    /// Default RGB colorimetry may be signalled.
    pub default_rgb: bool,
    /// sRGB may be signalled.
    pub srgb: bool,
    /// ICtCp (BT.2100).
    pub ictcp: bool,
    /// SMPTE ST 2113 RGB (P3-D65 and P3-DCI).
    pub st2113_rgb: bool,
}

impl Colorimetry {
    /// DCI-P3 family support, which CTA-861 signals as ST 2113 RGB.
    pub fn dci_p3(&self) -> bool {
        self.st2113_rgb
    }
}

/// HDR static metadata data block (extended tag 6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HdrStatic {
    /// Traditional gamma, SDR luminance range.
    pub eotf_sdr: bool,
    /// Traditional gamma, HDR luminance range.
    pub eotf_hdr_gamma: bool,
    /// SMPTE ST 2084 (PQ).
    pub eotf_pq: bool,
    /// Hybrid Log-Gamma.
    pub eotf_hlg: bool,
    /// Static metadata types bitmap (bit 0 is type 1).
    pub metadata_types: u8,
    /// Raw desired content max luminance code.
    pub max_luminance_raw: Option<u8>,
    /// Raw desired content max frame-average luminance code.
    pub max_frame_avg_raw: Option<u8>,
    /// Raw desired content min luminance code.
    pub min_luminance_raw: Option<u8>,
}

impl HdrStatic {
    /// Static metadata type 1 is supported.
    pub fn type1(&self) -> bool {
        self.metadata_types & 1 != 0
    }

    /// Desired content max luminance in nits (50 * 2^(cv/32)).
    pub fn max_luminance_nits(&self) -> Option<f32> {
        self.max_luminance_raw.map(luminance_nits)
    }

    /// Desired content max frame-average luminance in nits.
    pub fn max_frame_avg_nits(&self) -> Option<f32> {
        self.max_frame_avg_raw.map(luminance_nits)
    }

    /// Desired content min luminance in nits (max * (cv/255)^2 / 100); needs the max code too.
    pub fn min_luminance_nits(&self) -> Option<f32> {
        let max = self.max_luminance_nits()?;
        let cv = f32::from(self.min_luminance_raw?);
        Some(max * (cv / 255.0) * (cv / 255.0) / 100.0)
    }
}

/// Converts a luminance code value to nits (50 * 2^(cv/32)).
pub fn luminance_nits(cv: u8) -> f32 {
    50.0 * (f32::from(cv) / 32.0).exp2()
}

/// Converts a 12-bit PQ (SMPTE ST 2084) code to nits.
pub fn pq_to_nits(pq: u16) -> f32 {
    let v = f64::from(pq.min(4095)) / 4095.0;
    let (m1, m2) = (2610.0 / 16384.0, 128.0 * (2523.0 / 4096.0));
    let (c1, c2, c3) = (
        3424.0 / 4096.0,
        32.0 * (2413.0 / 4096.0),
        32.0 * (2392.0 / 4096.0),
    );
    let e = v.powf(1.0 / m2);
    let n = ((e - c1).max(0.0) / (c2 - c3 * e)).powf(1.0 / m1);
    (n * 10000.0) as f32
}

/// One HDR dynamic metadata entry (extended tag 7).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HdrDynamicEntry {
    /// Metadata type (1 = SMPTE ST 2094-1, 2 = ST 2094-10, 3 = ST 2094-20, 4 = ST 2094-40...).
    pub kind: u16,
    /// Version (low nibble of the first support byte), if present.
    pub version: Option<u8>,
    /// First support-flags byte as stored (type 2 carries the SL-HDR flags in bits 4..6).
    pub flags: Option<u8>,
}

/// A CIE 1931 xy chromaticity.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Chromaticity {
    /// x.
    pub x: f32,
    /// y.
    pub y: f32,
}

/// Dolby Vision primaries (white is only present in version 0).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DolbyPrimaries {
    /// Red.
    pub red: Chromaticity,
    /// Green.
    pub green: Chromaticity,
    /// Blue.
    pub blue: Chromaticity,
    /// White (version 0 only).
    pub white: Option<Chromaticity>,
}

/// Dolby Vision vendor-specific video data block (OUI 00-D0-46).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DolbyVision {
    /// VSVDB version (0, 1 or 2).
    pub version: u8,
    /// Supports YUV 4:2:2 12-bit.
    pub yuv422_12bit: bool,
    /// Supports 2160p60 (versions 0 and 1).
    pub supports_2160p60: bool,
    /// Supports global dimming.
    pub global_dimming: bool,
    /// Supports backlight control (version 2).
    pub backlight_control: bool,
    /// Display management major version.
    pub dm_major: u8,
    /// Display management minor version (version 0 only; later versions say "x").
    pub dm_minor: Option<u8>,
    /// Target minimum PQ code (versions 0 and 2).
    pub target_min_pq: Option<u16>,
    /// Target maximum PQ code (versions 0 and 2).
    pub target_max_pq: Option<u16>,
    /// Target minimum luminance in nits.
    pub target_min_nits: Option<f32>,
    /// Target maximum luminance in nits.
    pub target_max_nits: Option<f32>,
    /// Version 1: P3-D65 colorimetry rather than BT.709.
    pub colorimetry_p3: Option<bool>,
    /// Version 1: standard plus low-latency interface (else standard only).
    pub low_latency: Option<bool>,
    /// Version 2: interface code (0 LL, 1 LL + LL-HDMI, 2 std + LL, 3 std + LL + LL-HDMI).
    pub interface: Option<u8>,
    /// Version 2: bits for YUV 4:4:4 (0 none, 10 or 12).
    pub yuv444_bits: Option<u8>,
    /// Version 2: backlight minimum luminance in nits.
    pub backlight_min_luma: Option<u16>,
    /// Primaries.
    pub primaries: Option<DolbyPrimaries>,
}

/// HDR10+ vendor-specific video data block (OUI 90-84-8B).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hdr10Plus {
    /// Application version.
    pub application_version: u8,
    /// Full-frame peak luminance index.
    pub full_frame_peak_index: u8,
    /// Peak luminance index.
    pub peak_index: u8,
}

/// AMD FreeSync vendor-specific data block (OUI 00-00-1A).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FreeSync {
    /// Block version.
    pub version: u8,
    /// Feature capability byte (bit 6: replay).
    pub feature_caps: u8,
    /// Minimum refresh rate in Hz.
    pub min_hz: u8,
    /// Maximum refresh rate in Hz.
    pub max_hz: u8,
    /// FreeSync 1.x flags byte.
    pub flags_1x: Option<u8>,
    /// FreeSync 2 flags byte.
    pub flags_2x: Option<u8>,
    /// Maximum luminance in nits (FreeSync 2).
    pub max_luminance_nits: Option<f32>,
    /// Minimum luminance in nits (FreeSync 2).
    pub min_luminance_nits: Option<f32>,
}

/// HDMI Licensing VSDB, video part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HdmiVideo {
    /// 48-bit deep colour.
    pub dc_48bit: bool,
    /// 36-bit deep colour.
    pub dc_36bit: bool,
    /// 30-bit deep colour.
    pub dc_30bit: bool,
    /// Deep colour in YCbCr 4:4:4.
    pub dc_y444: bool,
    /// Dual-link DVI.
    pub dvi_dual: bool,
    /// Maximum TMDS clock in MHz.
    pub max_tmds_mhz: Option<u16>,
    /// HDMI_Video_present was set.
    pub video_present: bool,
    /// Image size code (0 none, 1 aspect only, 2 1 cm units, 3 5 cm units).
    pub image_size: u8,
    /// HDMI VICs (4K and similar formats).
    pub hdmi_vics: Vec<u8>,
    /// 3D present.
    pub three_d_present: bool,
    /// 3D_multi_present (0 none, 1 structure-all, 2 structure-all plus mask).
    pub three_d_multi: u8,
    /// 3D_structure_ALL bitmap.
    pub three_d_structure_all: Option<u16>,
    /// 3D_MASK bitmap.
    pub three_d_mask: Option<u16>,
}

/// An HDMI 2.1 fixed rate link code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrlRate(pub u8);

impl FrlRate {
    /// Lane count and Gbit/s per lane of the best mode: 1=3x3, 2=3x6, 3=4x6, 4=4x8, 5=4x10,
    /// 6=4x12. `None` for 0 (unsupported) and unknown codes.
    pub fn lanes_and_gbps(&self) -> Option<(u8, u8)> {
        match self.0 {
            1 => Some((3, 3)),
            2 => Some((3, 6)),
            3 => Some((4, 6)),
            4 => Some((4, 8)),
            5 => Some((4, 10)),
            6 => Some((4, 12)),
            _ => None,
        }
    }

    /// Number of lanes.
    pub fn lanes(&self) -> Option<u8> {
        self.lanes_and_gbps().map(|v| v.0)
    }

    /// Gbit/s per lane.
    pub fn gbps_per_lane(&self) -> Option<u8> {
        self.lanes_and_gbps().map(|v| v.1)
    }

    /// Total raw link rate in Gbit/s.
    pub fn total_gbps(&self) -> Option<u16> {
        self.lanes_and_gbps()
            .map(|(l, g)| u16::from(l) * u16::from(g))
    }
}

/// HDMI Forum VSDB / Sink Capability Data Structure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HdmiForum {
    /// Version.
    pub version: u8,
    /// Payload bytes available from the version byte on (fields past the end read as clear).
    pub payload_len: u8,
    /// Maximum TMDS character rate in MHz (0 = not specified).
    pub max_tmds_mhz: u16,
    /// SCDC present.
    pub scdc_present: bool,
    /// SCDC read request capable.
    pub rr_capable: bool,
    /// Cable status supported.
    pub cable_status: bool,
    /// Colour content bits per component indication.
    pub ccbpci: bool,
    /// Scrambling supported at 340 Mcsc or below.
    pub lte_340mcsc_scramble: bool,
    /// 3D independent view.
    pub independent_view: bool,
    /// 3D dual view.
    pub dual_view: bool,
    /// 3D OSD disparity.
    pub osd_disparity: bool,
    /// Maximum FRL rate.
    pub max_frl: FrlRate,
    /// UHD VIC.
    pub uhd_vic: bool,
    /// 48-bit deep colour in 4:2:0.
    pub dc_48bit_420: bool,
    /// 36-bit deep colour in 4:2:0.
    pub dc_36bit_420: bool,
    /// 30-bit deep colour in 4:2:0.
    pub dc_30bit_420: bool,
    /// FAPA end extended.
    pub fapa_end_extended: bool,
    /// QMS.
    pub qms: bool,
    /// M_delta.
    pub m_delta: bool,
    /// CinemaVRR (deprecated).
    pub cinema_vrr: bool,
    /// Negative M_vrr values (CNMVRR).
    pub cnm_vrr: bool,
    /// Fast Vactive.
    pub fva: bool,
    /// Auto Low-Latency Mode.
    pub allm: bool,
    /// FAPA start location.
    pub fapa_start_location: bool,
    /// VRRmin in Hz (0 = not specified).
    pub vrr_min: u8,
    /// VRRmax in Hz (0 = not specified).
    pub vrr_max: u16,
    /// DSC 1.2a.
    pub dsc_1p2: bool,
    /// Compressed video transport for 4:2:0.
    pub dsc_native_420: bool,
    /// QMS TFRmax.
    pub qms_tfr_max: bool,
    /// QMS TFRmin.
    pub qms_tfr_min: bool,
    /// Any valid 1/16 bpp.
    pub dsc_all_bpp: bool,
    /// 16 bpc DSC.
    pub dsc_16bpc: bool,
    /// 12 bpc DSC.
    pub dsc_12bpc: bool,
    /// 10 bpc DSC.
    pub dsc_10bpc: bool,
    /// DSC max slices code (0..=15).
    pub dsc_max_slices_code: u8,
    /// DSC max FRL rate.
    pub dsc_max_frl: FrlRate,
    /// Total chunk size in KiB (1 + code); `None` when the code is 0.
    pub dsc_total_chunk_kbytes: Option<u8>,
}

impl HdmiForum {
    /// DSC maximum slices and pixel clock per slice (MHz before the Ksliceadjust factor).
    /// Unknown codes above 7 read as 7; code 0 is unsupported.
    pub fn dsc_max_slices(&self) -> Option<(u8, u16)> {
        match self.dsc_max_slices_code {
            0 => None,
            1 => Some((1, 340)),
            2 => Some((2, 340)),
            3 => Some((4, 340)),
            4 => Some((8, 340)),
            5 => Some((8, 400)),
            6 => Some((12, 400)),
            7 => Some((16, 400)),
            // 8–15 are reserved.
            _ => None,
        }
    }
}

/// The video content of one CTA-861 extension block.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CtaVideo {
    /// Sink underscans IT video formats by default (header byte 3, bit 7).
    pub underscan: bool,
    /// Supports YCbCr 4:4:4 (header byte 3, bit 5).
    pub ycbcr444: bool,
    /// Supports YCbCr 4:2:2 (header byte 3, bit 4).
    pub ycbcr422: bool,
    /// Number of native detailed timings (header byte 3, bits 0..3).
    pub native_dtds: u8,
    /// SVDs of all video data blocks, in order.
    pub svds: Vec<Svd>,
    /// VICs of the YCbCr 4:2:0-only video data block (extended tag 14).
    pub y420_only: Vec<Svd>,
    /// YCbCr 4:2:0 capability map; `Some(empty)` means every SVD is 4:2:0 capable.
    pub y420_cap_map: Option<Vec<u8>>,
    /// Video capability data block.
    pub capability: Option<VideoCapability>,
    /// Colorimetry data block.
    pub colorimetry: Option<Colorimetry>,
    /// HDR static metadata data block.
    pub hdr_static: Option<HdrStatic>,
    /// HDR dynamic metadata entries.
    pub hdr_dynamic: Vec<HdrDynamicEntry>,
    /// Video format preference data block (raw short video references).
    pub format_preference: Vec<u8>,
    /// Dolby Vision VSVDB.
    pub dolby_vision: Option<DolbyVision>,
    /// HDR10+ VSVDB.
    pub hdr10_plus: Option<Hdr10Plus>,
    /// HDMI Licensing VSDB video part.
    pub hdmi: Option<HdmiVideo>,
    /// HDMI Forum VSDB.
    pub hdmi_forum_vsdb: Option<HdmiForum>,
    /// HDMI Forum Sink Capability Data Structure (extended tag 0x79).
    pub hdmi_forum_scdb: Option<HdmiForum>,
    /// AMD FreeSync VSDB.
    pub freesync: Option<FreeSync>,
    /// HDMI Forum EEODB: EDID extension block count.
    pub eeodb_ext_blocks: Option<u8>,
    /// Every extended tag seen (including those decoded above, e.g. 32 InfoFrame).
    pub extended_tags: Vec<u8>,
    /// Extended tags this module does not decode.
    pub unknown_extended_tags: Vec<u8>,
    /// Detailed timings after the data block collection.
    pub detailed: Vec<DetailedTiming>,
}

impl CtaVideo {
    /// The HDMI Forum data, from the VSDB or else the SCDB.
    pub fn hdmi_forum(&self) -> Option<&HdmiForum> {
        self.hdmi_forum_vsdb
            .as_ref()
            .or(self.hdmi_forum_scdb.as_ref())
    }

    /// VICs that may be sent as YCbCr 4:2:0: the 4:2:0-only list plus the SVDs selected by the
    /// capability map (all of them when the map is present but empty).
    pub fn y420_capable_vics(&self) -> Vec<u8> {
        let mut out: Vec<u8> = self.y420_only.iter().map(|s| s.vic).collect();
        match &self.y420_cap_map {
            Some(m) if m.is_empty() => out.extend(self.svds.iter().map(|s| s.vic)),
            Some(m) => {
                for (i, s) in self.svds.iter().enumerate() {
                    if m.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0) {
                        out.push(s.vic);
                    }
                }
            }
            None => {}
        }
        out
    }
}

/// Decodes the video content of one 128-byte CTA-861 extension block.
pub fn parse_cta_video(blk: &[u8]) -> CtaVideo {
    let mut v = CtaVideo::default();
    if let Some(&b3) = blk.get(3) {
        v.underscan = b3 & 0x80 != 0;
        v.ycbcr444 = b3 & 0x20 != 0;
        v.ycbcr422 = b3 & 0x10 != 0;
        v.native_dtds = b3 & 0x0F;
    }
    let d = usize::from(*blk.get(2).unwrap_or(&0));
    let end = if d == 0 { 4 } else { d.min(blk.len()) };
    let mut i = 4;
    while i < end {
        let len = usize::from(blk[i] & 0x1F);
        let start = i + 1;
        if start + len > end {
            break;
        }
        data_block(&mut v, blk[i] >> 5, &blk[start..start + len]);
        i = start + len;
    }
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

fn data_block(v: &mut CtaVideo, tag: u8, p: &[u8]) {
    match tag {
        2 => v.svds.extend(p.iter().map(|&b| Svd::from_byte(b))),
        3 if p.len() >= 3 => {
            let oui = u32::from(p[0]) | (u32::from(p[1]) << 8) | (u32::from(p[2]) << 16);
            match oui {
                OUI_HDMI if v.hdmi.is_none() => v.hdmi = Some(parse_hdmi(p)),
                OUI_HDMI_FORUM if v.hdmi_forum_vsdb.is_none() => {
                    v.hdmi_forum_vsdb = Some(parse_hdmi_forum(&p[3..]))
                }
                OUI_AMD if v.freesync.is_none() => v.freesync = parse_freesync(&p[3..]),
                _ => {}
            }
        }
        7 if !p.is_empty() => extended(v, p[0], &p[1..]),
        _ => {}
    }
}

fn extended(v: &mut CtaVideo, tag: u8, q: &[u8]) {
    v.extended_tags.push(tag);
    let at = |i: usize| q.get(i).copied();
    match tag {
        0 => {
            if let Some(b) = at(0) {
                v.capability = Some(VideoCapability {
                    qy: b & 0x80 != 0,
                    qs: b & 0x40 != 0,
                    s_pt: (b >> 4) & 3,
                    s_it: (b >> 2) & 3,
                    s_ce: b & 3,
                });
            }
        }
        1 => vsvdb(v, q),
        5 => {
            if let Some(a) = at(0) {
                let b = at(1).unwrap_or(0);
                v.colorimetry = Some(Colorimetry {
                    xvycc601: a & 1 != 0,
                    xvycc709: a & 2 != 0,
                    sycc601: a & 4 != 0,
                    opycc601: a & 8 != 0,
                    oprgb: a & 0x10 != 0,
                    bt2020_cycc: a & 0x20 != 0,
                    bt2020_ycc: a & 0x40 != 0,
                    bt2020_rgb: a & 0x80 != 0,
                    metadata_profiles: b & 0x0F,
                    default_rgb: b & 0x10 != 0,
                    srgb: b & 0x20 != 0,
                    ictcp: b & 0x40 != 0,
                    st2113_rgb: b & 0x80 != 0,
                });
            }
        }
        6 => {
            if let (Some(e), Some(t)) = (at(0), at(1)) {
                v.hdr_static = Some(HdrStatic {
                    eotf_sdr: e & 1 != 0,
                    eotf_hdr_gamma: e & 2 != 0,
                    eotf_pq: e & 4 != 0,
                    eotf_hlg: e & 8 != 0,
                    metadata_types: t,
                    max_luminance_raw: at(2),
                    max_frame_avg_raw: at(3),
                    min_luminance_raw: at(4),
                });
            }
        }
        7 => {
            let mut r = q;
            while r.len() >= 3 {
                let n = usize::from(r[0]);
                if n < 2 || r.len() < n + 1 {
                    break;
                }
                let kind = u16::from(r[1]) | (u16::from(r[2]) << 8);
                let flags = (n > 2).then(|| r[3]);
                v.hdr_dynamic.push(HdrDynamicEntry {
                    kind,
                    version: flags.map(|f| f & 0x0F),
                    flags,
                });
                r = &r[n + 1..];
            }
        }
        13 => v.format_preference = q.to_vec(),
        14 => v.y420_only.extend(q.iter().map(|&b| Svd::from_byte(b))),
        15 => v.y420_cap_map = Some(q.to_vec()),
        0x78 => v.eeodb_ext_blocks = at(0),
        0x79 if q.len() >= 2 => v.hdmi_forum_scdb = Some(parse_hdmi_forum(&q[2..])),
        32 => {}
        _ => v.unknown_extended_tags.push(tag),
    }
}

/// Extended tag 1: vendor-specific video data block; `q` starts at the OUI.
fn vsvdb(v: &mut CtaVideo, q: &[u8]) {
    if q.len() < 3 {
        return;
    }
    let oui = u32::from(q[0]) | (u32::from(q[1]) << 8) | (u32::from(q[2]) << 16);
    let x = &q[3..];
    match oui {
        OUI_DOLBY if v.dolby_vision.is_none() => v.dolby_vision = parse_dolby(x),
        OUI_HDR10PLUS if v.hdr10_plus.is_none() => {
            if let Some(&b) = x.first() {
                v.hdr10_plus = Some(Hdr10Plus {
                    application_version: b & 3,
                    full_frame_peak_index: (b >> 2) & 3,
                    peak_index: b >> 4,
                });
            }
        }
        _ => v.unknown_extended_tags.push(1),
    }
}

fn parse_dolby(x: &[u8]) -> Option<DolbyVision> {
    let g = |i: usize| x.get(i).copied().map(u16::from);
    let x0 = g(0)?;
    let ver = (x0 >> 5) as u8 & 7;
    let mut d = DolbyVision {
        version: ver,
        yuv422_12bit: x0 & 1 != 0,
        ..DolbyVision::default()
    };
    match ver {
        0 if x.len() >= 17 => {
            let v = |i: usize| g(i).unwrap_or(0);
            d.supports_2160p60 = x0 & 2 != 0;
            d.global_dimming = x0 & 4 != 0;
            let dm = v(16);
            d.dm_major = (dm >> 4) as u8;
            d.dm_minor = Some((dm & 0xF) as u8);
            let min = (v(14) << 4) | (v(13) >> 4);
            let max = (v(15) << 4) | (v(13) & 0xF);
            d.target_min_pq = Some(min);
            d.target_max_pq = Some(max);
            d.target_min_nits = Some(pq_to_nits(min));
            d.target_max_nits = Some(pq_to_nits(max));
            let pt = |a: usize, b: usize, c: usize| Chromaticity {
                x: f32::from((v(a) >> 4) | (v(b) << 4)) / 4096.0,
                y: f32::from((v(a) & 0xF) | (v(c) << 4)) / 4096.0,
            };
            d.primaries = Some(DolbyPrimaries {
                red: pt(1, 2, 3),
                green: pt(4, 5, 6),
                blue: pt(7, 8, 9),
                white: Some(pt(10, 11, 12)),
            });
        }
        1 if x.len() >= 4 => {
            let v = |i: usize| g(i).unwrap_or(0);
            d.supports_2160p60 = x0 & 2 != 0;
            d.global_dimming = v(1) & 1 != 0;
            d.dm_major = (((x0 >> 2) & 7) + 2) as u8;
            d.colorimetry_p3 = Some(v(2) & 1 != 0);
            d.low_latency = Some(v(3) & 1 != 0);
            let lm = f32::from(v(2) >> 1) / 127.0;
            d.target_min_nits = Some(lm * lm);
            d.target_max_nits = Some(100.0 + f32::from(v(1) >> 1) * 50.0);
            if x.len() == 10 {
                let c = |a: usize, b: usize| Chromaticity {
                    x: f32::from(v(a)) / 256.0,
                    y: f32::from(v(b)) / 256.0,
                };
                d.primaries = Some(DolbyPrimaries {
                    red: c(4, 5),
                    green: c(6, 7),
                    blue: c(8, 9),
                    white: None,
                });
            } else if x.len() >= 7 {
                let (xmin, ymin) = (0.625f32, 0.25f32);
                let xs = (0.74609375 - xmin) / 31.0;
                let ys = (0.37109375 - ymin) / 31.0;
                let ry = ((v(6) & 7) << 2) | (v(4) & 1) | ((v(5) & 1) << 1);
                let red = Chromaticity {
                    x: xmin + xs * f32::from(v(6) >> 3),
                    y: ymin + ys * f32::from(ry),
                };
                let gxs = 0.49609375 / 127.0;
                let gys = (0.99609375 - 0.5) / 127.0;
                let green = Chromaticity {
                    x: gxs * f32::from(v(4) >> 1),
                    y: 0.5 + gys * f32::from(v(5) >> 1),
                };
                let bxs = (0.15234375 - 0.125) / 7.0;
                let bys = (0.05859375 - 0.03125) / 7.0;
                let blue = Chromaticity {
                    x: 0.125 + bxs * f32::from(v(3) >> 5),
                    y: 0.03125 + bys * f32::from((v(3) >> 2) & 7),
                };
                d.primaries = Some(DolbyPrimaries {
                    red,
                    green,
                    blue,
                    white: None,
                });
            }
        }
        2 if x.len() >= 7 => {
            let v = |i: usize| g(i).unwrap_or(0);
            d.backlight_control = x0 & 2 != 0;
            d.global_dimming = v(1) & 4 != 0;
            d.dm_major = (((x0 >> 2) & 7) + 2) as u8;
            d.backlight_min_luma = Some(25 + (v(1) & 3) * 25);
            d.interface = Some((v(2) & 3) as u8);
            d.yuv444_bits = Some(match ((v(3) & 1) << 1) | (v(4) & 1) {
                1 => 10,
                2 => 12,
                _ => 0,
            });
            let min = 20 * (v(1) >> 3);
            let max = 2055 + 65 * (v(2) >> 3);
            d.target_min_pq = Some(min);
            d.target_max_pq = Some(max);
            d.target_min_nits = Some(pq_to_nits(min));
            d.target_max_nits = Some(pq_to_nits(max));
            d.primaries = Some(DolbyPrimaries {
                red: Chromaticity {
                    x: 0.625 + f32::from(v(5) >> 3) / 256.0,
                    y: 0.25 + f32::from(v(6) >> 3) / 256.0,
                },
                green: Chromaticity {
                    x: f32::from(v(3) >> 1) / 256.0,
                    y: 0.5 + f32::from(v(4) >> 1) / 256.0,
                },
                blue: Chromaticity {
                    x: 0.125 + f32::from(v(5) & 7) / 256.0,
                    y: 0.03125 + f32::from(v(6) & 7) / 256.0,
                },
                white: None,
            });
        }
        _ => {}
    }
    Some(d)
}

fn parse_freesync(x: &[u8]) -> Option<FreeSync> {
    if x.len() < 4 {
        return None;
    }
    let mut f = FreeSync {
        version: x[0],
        feature_caps: x[1],
        min_hz: x[2],
        max_hz: x[3],
        flags_1x: x.get(4).copied(),
        ..FreeSync::default()
    };
    if x.len() >= 10 {
        f.flags_2x = Some(x[5]);
        let max = luminance_nits(x[6]);
        let cv = f32::from(x[7]) / 255.0;
        f.max_luminance_nits = Some(max);
        f.min_luminance_nits = Some(max * cv * cv / 100.0);
    }
    Some(f)
}

/// Decodes an HDMI Licensing VSDB payload (OUI included).
fn parse_hdmi(p: &[u8]) -> HdmiVideo {
    let at = |i: usize| p.get(i).copied();
    let mut h = HdmiVideo::default();
    if let Some(f) = at(5) {
        h.dc_48bit = f & 0x40 != 0;
        h.dc_36bit = f & 0x20 != 0;
        h.dc_30bit = f & 0x10 != 0;
        h.dc_y444 = f & 0x08 != 0;
        h.dvi_dual = f & 0x01 != 0;
    }
    h.max_tmds_mhz = at(6).filter(|&v| v != 0).map(|v| u16::from(v) * 5);
    let Some(f) = at(7) else { return h };
    let mut i = 8;
    if f & 0x80 != 0 {
        i += 2;
        if f & 0x40 != 0 {
            i += 2;
        }
    }
    if f & 0x20 == 0 {
        return h;
    }
    h.video_present = true;
    let (Some(a), Some(b)) = (at(i), at(i + 1)) else {
        return h;
    };
    h.three_d_present = a & 0x80 != 0;
    h.three_d_multi = (a >> 5) & 3;
    h.image_size = (a >> 3) & 3;
    let vic_len = usize::from(b >> 5);
    i += 2;
    for k in 0..vic_len {
        if let Some(v) = at(i + k) {
            h.hdmi_vics.push(v);
        }
    }
    i += vic_len;
    if h.three_d_present && matches!(h.three_d_multi, 1 | 2) {
        if let (Some(x), Some(y)) = (at(i), at(i + 1)) {
            h.three_d_structure_all = Some(u16::from_be_bytes([x, y]));
        }
        if h.three_d_multi == 2
            && let (Some(x), Some(y)) = (at(i + 2), at(i + 3))
        {
            h.three_d_mask = Some(u16::from_be_bytes([x, y]));
        }
    }
    h
}

/// Decodes the HDMI Forum VSDB / SCDB fields; `x[0]` is the version byte.
fn parse_hdmi_forum(x: &[u8]) -> HdmiForum {
    let g = |i: usize| x.get(i).copied().unwrap_or(0);
    let mut h = HdmiForum {
        version: g(0),
        payload_len: x.len().min(255) as u8,
        max_tmds_mhz: u16::from(g(1)) * 5,
        scdc_present: g(2) & 0x80 != 0,
        rr_capable: g(2) & 0x40 != 0,
        cable_status: g(2) & 0x20 != 0,
        ccbpci: g(2) & 0x10 != 0,
        lte_340mcsc_scramble: g(2) & 0x08 != 0,
        independent_view: g(2) & 0x04 != 0,
        dual_view: g(2) & 0x02 != 0,
        osd_disparity: g(2) & 0x01 != 0,
        max_frl: FrlRate(g(3) >> 4),
        uhd_vic: g(3) & 0x08 != 0,
        dc_48bit_420: g(3) & 0x04 != 0,
        dc_36bit_420: g(3) & 0x02 != 0,
        dc_30bit_420: g(3) & 0x01 != 0,
        ..HdmiForum::default()
    };
    if x.len() > 4 {
        let b = g(4);
        h.fapa_end_extended = b & 0x80 != 0;
        h.qms = b & 0x40 != 0;
        h.m_delta = b & 0x20 != 0;
        h.cinema_vrr = b & 0x10 != 0;
        h.cnm_vrr = b & 0x08 != 0;
        h.fva = b & 0x04 != 0;
        h.allm = b & 0x02 != 0;
        h.fapa_start_location = b & 0x01 != 0;
    }
    if x.len() > 5 {
        h.vrr_min = g(5) & 0x3F;
        h.vrr_max = (u16::from(g(5) & 0xC0) << 2) | u16::from(g(6));
    }
    if x.len() > 7 {
        let b = g(7);
        h.dsc_1p2 = b & 0x80 != 0;
        h.dsc_native_420 = b & 0x40 != 0;
        h.qms_tfr_max = b & 0x20 != 0;
        h.qms_tfr_min = b & 0x10 != 0;
        h.dsc_all_bpp = b & 0x08 != 0;
        h.dsc_16bpc = b & 0x04 != 0;
        h.dsc_12bpc = b & 0x02 != 0;
        h.dsc_10bpc = b & 0x01 != 0;
        h.dsc_max_slices_code = g(8) & 0x0F;
        h.dsc_max_frl = FrlRate(g(8) >> 4);
        let c = g(9) & 0x3F;
        h.dsc_total_chunk_kbytes = (x.len() > 9 && c != 0).then(|| 1 + c);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a 128-byte CTA block from raw data blocks (header bytes included) and byte 3.
    fn cta(b3: u8, blocks: &[&[u8]]) -> Vec<u8> {
        let mut v = vec![0u8; 128];
        v[0] = 0x02;
        v[1] = 3;
        v[3] = b3;
        let mut i = 4;
        for b in blocks {
            v[i..i + b.len()].copy_from_slice(b);
            i += b.len();
        }
        v[2] = i as u8;
        v
    }

    /// A data block with the given tag and payload.
    fn db(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![(tag << 5) | payload.len() as u8];
        v.extend_from_slice(payload);
        v
    }

    /// An extended data block.
    fn ext(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![tag];
        p.extend_from_slice(payload);
        db(7, &p)
    }

    #[test]
    fn header_and_svds() {
        let b = cta(0xB2, &[&db(2, &[16 | 0x80, 4, 193, 97, 129])]);
        let v = parse_cta_video(&b);
        assert!(v.underscan && v.ycbcr444 && v.ycbcr422);
        assert_eq!(v.native_dtds, 2);
        assert_eq!(
            v.svds,
            vec![
                Svd {
                    vic: 16,
                    native: true
                },
                Svd {
                    vic: 4,
                    native: false
                },
                Svd {
                    vic: 193,
                    native: false
                },
                Svd {
                    vic: 97,
                    native: false
                },
                Svd {
                    vic: 1,
                    native: true
                },
            ]
        );
    }

    #[test]
    fn video_capability_and_colorimetry() {
        let b = cta(0, &[&ext(0, &[0b1110_0110]), &ext(5, &[0xE5, 0xE1])]);
        let v = parse_cta_video(&b);
        let c = v.capability.unwrap();
        assert!(c.qy && c.qs);
        assert_eq!((c.s_pt, c.s_it, c.s_ce), (2, 1, 2));
        let k = v.colorimetry.unwrap();
        assert!(k.xvycc601 && !k.xvycc709 && k.sycc601 && k.bt2020_ycc && k.bt2020_rgb);
        assert!(k.bt2020_cycc && !k.oprgb && !k.opycc601);
        assert_eq!(k.metadata_profiles, 1);
        assert!(k.srgb && k.ictcp && k.st2113_rgb && k.dci_p3() && !k.default_rgb);
        assert_eq!(v.extended_tags, vec![0, 5]);
    }

    #[test]
    fn hdr_static_and_dynamic() {
        let b = cta(
            0,
            &[
                &ext(6, &[0x0F, 0x01, 96, 64, 0]),
                &ext(7, &[3, 4, 0, 0x01, 2, 1, 0]),
            ],
        );
        let v = parse_cta_video(&b);
        let h = v.hdr_static.unwrap();
        assert!(h.eotf_sdr && h.eotf_hdr_gamma && h.eotf_pq && h.eotf_hlg && h.type1());
        assert_eq!(h.max_luminance_nits(), Some(400.0));
        assert!((h.max_frame_avg_nits().unwrap() - 200.0).abs() < 0.01);
        assert_eq!(h.min_luminance_nits(), Some(0.0));
        assert_eq!(v.hdr_dynamic.len(), 2);
        assert_eq!(v.hdr_dynamic[0].kind, 4);
        assert_eq!(v.hdr_dynamic[0].version, Some(1));
        assert_eq!(v.hdr_dynamic[1].kind, 1);
        assert_eq!(v.hdr_dynamic[1].version, None);
        let h2 = HdrStatic {
            max_luminance_raw: Some(96),
            min_luminance_raw: Some(51),
            ..HdrStatic::default()
        };
        assert!((h2.min_luminance_nits().unwrap() - 0.16).abs() < 0.001);
    }

    #[test]
    fn short_blocks_do_not_panic() {
        for t in [0u8, 1, 5, 6, 7, 13, 14, 15, 32, 0x78, 0x79, 0x7F] {
            let b = cta(0, &[&ext(t, &[])]);
            let _ = parse_cta_video(&b);
        }
        let b = cta(0, &[&ext(6, &[0x04])]);
        assert!(parse_cta_video(&b).hdr_static.is_none());
        let b = cta(
            0,
            &[&db(3, &[0x03, 0x0C]), &db(3, &[0x03, 0x0C, 0x00, 0, 0])],
        );
        let v = parse_cta_video(&b);
        assert!(v.hdmi.is_some());
        assert!(parse_cta_video(&[]).svds.is_empty());
        assert!(parse_cta_video(&[2, 3, 200, 0]).svds.is_empty());
        let mut b = cta(0, &[&db(2, &[1, 2, 3])]);
        b[4] = (2 << 5) | 31;
        assert!(parse_cta_video(&b).svds.is_empty());
    }

    #[test]
    fn y420_blocks_and_map() {
        let b = cta(
            0,
            &[
                &db(2, &[16, 97, 101, 4]),
                &ext(14, &[96, 122]),
                &ext(15, &[0b0000_0110]),
            ],
        );
        let v = parse_cta_video(&b);
        assert_eq!(v.y420_only.len(), 2);
        assert_eq!(v.y420_capable_vics(), vec![96, 122, 97, 101]);
        let all = cta(0, &[&db(2, &[16, 97]), &ext(15, &[])]);
        assert_eq!(parse_cta_video(&all).y420_capable_vics(), vec![16, 97]);
        let none = cta(0, &[&db(2, &[16, 97])]);
        assert!(parse_cta_video(&none).y420_capable_vics().is_empty());
        let beyond = cta(0, &[&db(2, &[16]), &ext(15, &[0x80, 0x01])]);
        assert!(parse_cta_video(&beyond).y420_capable_vics().is_empty());
    }

    #[test]
    fn hdmi_vsdb_video_part() {
        let p = [
            0x03,
            0x0C,
            0x00,
            0x10,
            0x00,        // OUI, phys addr
            0x78,        // DC 48/36/30/Y444
            0x44,        // 340 MHz
            0x20 | 0x80, // video present + latency present
            0x10,
            0x20,               // latency
            0xC0 | 0x40 | 0x08, // 3D present, multi=2, image size 1
            0x40 | 0x04,        // 2 VICs, 3D len 4
            0x01,
            0x03, // HDMI VICs
            0x00,
            0x07,
            0x00,
            0x03, // structure all, mask
        ];
        let v = parse_cta_video(&cta(0, &[&db(3, &p)]));
        let h = v.hdmi.unwrap();
        assert!(h.dc_48bit && h.dc_36bit && h.dc_30bit && h.dc_y444 && !h.dvi_dual);
        assert_eq!(h.max_tmds_mhz, Some(340));
        assert!(h.video_present && h.three_d_present);
        assert_eq!(h.three_d_multi, 2);
        assert_eq!(h.image_size, 1);
        assert_eq!(h.hdmi_vics, vec![1, 3]);
        assert_eq!(h.three_d_structure_all, Some(7));
        assert_eq!(h.three_d_mask, Some(3));
    }

    #[test]
    fn hdmi_forum_full() {
        let hf = [
            1,
            120,
            0xC9,
            0x6F,
            0xE5,
            0x40 | 0x01,
            0x78,
            0xFF,
            0x46,
            0x05,
        ];
        let mut p = vec![0xD8, 0x5D, 0xC4];
        p.extend_from_slice(&hf);
        let v = parse_cta_video(&cta(0, &[&db(3, &p)]));
        let h = v.hdmi_forum().unwrap();
        assert_eq!(h.version, 1);
        assert_eq!(h.max_tmds_mhz, 600);
        assert!(h.scdc_present && h.rr_capable && !h.cable_status && h.lte_340mcsc_scramble);
        assert!(!h.ccbpci && !h.independent_view && !h.dual_view && h.osd_disparity);
        assert_eq!(h.max_frl, FrlRate(6));
        assert_eq!(h.max_frl.lanes_and_gbps(), Some((4, 12)));
        assert_eq!(h.max_frl.total_gbps(), Some(48));
        assert!(h.uhd_vic && h.dc_48bit_420 && h.dc_36bit_420 && h.dc_30bit_420);
        assert!(h.fapa_end_extended && h.qms && h.m_delta && !h.cinema_vrr);
        assert!(!h.cnm_vrr && h.fva && !h.allm && h.fapa_start_location);
        assert_eq!(h.vrr_min, 1);
        assert_eq!(h.vrr_max, 0x100 | 0x78);
        assert!(h.dsc_1p2 && h.dsc_native_420 && h.qms_tfr_max && h.qms_tfr_min);
        assert!(h.dsc_all_bpp && h.dsc_16bpc && h.dsc_12bpc && h.dsc_10bpc);
        assert_eq!(h.dsc_max_slices(), Some((12, 400)));
        assert_eq!(h.dsc_max_frl, FrlRate(4));
        assert_eq!(h.dsc_total_chunk_kbytes, Some(6));
    }

    #[test]
    fn hdmi_forum_scdb_short_and_frl() {
        let v = parse_cta_video(&cta(0, &[&ext(0x79, &[0, 0, 1, 60, 0x80, 0x10])]));
        let h = v.hdmi_forum_scdb.unwrap();
        assert_eq!(h.max_tmds_mhz, 300);
        assert!(h.scdc_present);
        assert_eq!(h.max_frl, FrlRate(1));
        assert_eq!(h.max_frl.lanes(), Some(3));
        assert_eq!(h.max_frl.gbps_per_lane(), Some(3));
        assert_eq!(h.dsc_max_slices(), None);
        assert!(!h.allm);
        assert_eq!(FrlRate(0).lanes_and_gbps(), None);
        assert_eq!(FrlRate(9).total_gbps(), None);
        let v = parse_cta_video(&cta(0, &[&db(3, &[0xD8, 0x5D, 0xC4, 1])]));
        assert_eq!(v.hdmi_forum_vsdb.unwrap().payload_len, 1);
        let v = parse_cta_video(&cta(
            0,
            &[&ext(0x78, &[3]), &ext(32, &[1, 2]), &ext(0x40, &[])],
        ));
        assert_eq!(v.eeodb_ext_blocks, Some(3));
        assert_eq!(v.extended_tags, vec![0x78, 32, 0x40]);
        assert_eq!(v.unknown_extended_tags, vec![0x40]);
    }

    #[test]
    fn dolby_v0() {
        let mut x = vec![0x46, 0xD0, 0x00, 0x07]; // OUI, version 0, 422, 2160p60, GD
        x.extend_from_slice(&[0x00; 16]);
        x[3 + 13] = 0x12; // min PQ low nibble 1, max low nibble 2
        x[3 + 14] = 0x00;
        x[3 + 15] = 0x80;
        x[3 + 16] = 0x42;
        x[3 + 1] = 0x00;
        x[3 + 2] = 0xA0;
        x[3 + 3] = 0x30;
        let v = parse_cta_video(&cta(0, &[&ext(1, &x)]));
        let d = v.dolby_vision.unwrap();
        assert_eq!(d.version, 0);
        assert!(d.yuv422_12bit && d.supports_2160p60 && d.global_dimming);
        assert_eq!((d.dm_major, d.dm_minor), (4, Some(2)));
        assert_eq!(d.target_min_pq, Some(1));
        assert_eq!(d.target_max_pq, Some((0x80 << 4) | 2));
        let r = d.primaries.unwrap().red;
        assert!((r.x - (0xA00 as f32 / 4096.0)).abs() < 1e-6);
        assert!((r.y - (0x300 as f32 / 4096.0)).abs() < 1e-6);
        assert!(d.primaries.unwrap().white.is_some());
        // Too short for version 0: only the version is known.
        let s = parse_cta_video(&cta(0, &[&ext(1, &[0x46, 0xD0, 0x00, 0x01, 2])]));
        let d = s.dolby_vision.unwrap();
        assert!(d.yuv422_12bit && d.primaries.is_none());
    }

    #[test]
    fn dolby_v1_and_v2() {
        // Version 1, 12 bytes after OUI is not 10: unique primaries; use 10 for explicit.
        let mut x = vec![0x46, 0xD0, 0x00];
        x.extend_from_slice(&[0x23, 0x0C, 0x05, 0x01, 0x80, 0x80, 0x40, 0x40, 0x20, 0x20]);
        let v = parse_cta_video(&cta(0, &[&ext(1, &x)]));
        let d = v.dolby_vision.unwrap();
        assert_eq!(d.version, 1);
        assert!(d.yuv422_12bit && d.supports_2160p60 && !d.global_dimming);
        assert_eq!(d.dm_major, 2);
        assert_eq!(d.colorimetry_p3, Some(true));
        assert_eq!(d.low_latency, Some(true));
        assert_eq!(d.target_max_nits, Some(100.0 + 6.0 * 50.0));
        assert_eq!(d.primaries.unwrap().red, Chromaticity { x: 0.5, y: 0.5 });

        let mut x = vec![0x46, 0xD0, 0x00];
        x.extend_from_slice(&[0x47, 0x1D, 0x0B, 0x03, 0x00, 0x08, 0x08]);
        let v = parse_cta_video(&cta(0, &[&ext(1, &x)]));
        let d = v.dolby_vision.unwrap();
        assert_eq!(d.version, 2);
        assert!(d.backlight_control && d.yuv422_12bit && d.global_dimming);
        assert_eq!(d.dm_major, 3);
        assert_eq!(d.backlight_min_luma, Some(50));
        assert_eq!(d.interface, Some(3));
        assert_eq!(d.yuv444_bits, Some(12));
        assert_eq!(d.target_min_pq, Some(20 * 3));
        assert_eq!(d.target_max_pq, Some(2055 + 65));
        assert!((100.0..1000.0).contains(&d.target_max_nits.unwrap()));
        assert!((d.primaries.unwrap().red.x - (0.625 + 1.0 / 256.0)).abs() < 1e-6);
    }

    #[test]
    fn hdr10plus_and_freesync() {
        let v = parse_cta_video(&cta(
            0,
            &[
                &ext(1, &[0x8B, 0x84, 0x90, 0x49]),
                &db(
                    3,
                    &[
                        0x1A, 0x00, 0x00, 1, 0x40, 48, 144, 0xE6, 4, 98, 0x40, 0, 98, 0,
                    ],
                ),
            ],
        ));
        let h = v.hdr10_plus.unwrap();
        assert_eq!(
            (h.application_version, h.full_frame_peak_index, h.peak_index),
            (1, 2, 4)
        );
        let f = v.freesync.unwrap();
        assert_eq!((f.version, f.min_hz, f.max_hz), (1, 48, 144));
        assert_eq!(f.flags_1x, Some(0xE6));
        assert_eq!(f.flags_2x, Some(4));
        assert!((f.max_luminance_nits.unwrap() - 417.71).abs() < 0.01);
        let short = parse_cta_video(&cta(0, &[&db(3, &[0x1A, 0x00, 0x00, 1, 0])]));
        assert!(short.freesync.is_none());
    }

    #[test]
    fn format_preference_and_pq() {
        let v = parse_cta_video(&cta(0, &[&ext(13, &[16, 97])]));
        assert_eq!(v.format_preference, vec![16, 97]);
        assert!((pq_to_nits(4095) - 10000.0).abs() < 1.0);
        assert_eq!(pq_to_nits(0), 0.0);
    }

    #[test]
    fn detailed_timings_kept() {
        let mut b = cta(0, &[&db(2, &[16])]);
        let off = usize::from(b[2]);
        // 1920x1080 @ 60: clock 148.5 MHz.
        let dtd = [
            0x02, 0x3A, 0x80, 0x18, 0x71, 0x38, 0x2D, 0x40, 0x58, 0x2C, 0x45, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x1E,
        ];
        b[off..off + 18].copy_from_slice(&dtd);
        let v = parse_cta_video(&b);
        assert_eq!(v.detailed.len(), 1);
        assert_eq!(v.detailed[0].h_active, 1920);
    }
}
