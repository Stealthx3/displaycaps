// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-parse/src/edid.rs; serial() added from
// crates/audiocaps-backends/src/assemble/sink.rs. Video blocks are added in phase 2.
//! EDID and CTA-861 extension parsing, plus serial redaction for the capture tool.
//!
//! [`parse_edid`] decodes the base block (identity, name) and every CTA-861 extension: audio data
//! blocks (SADs, raw bytes kept), speaker allocation, the HDMI and HDMI Forum vendor-specific
//! blocks (CEC physical address, latency, TMDS/FRL limits) and the list of other data blocks.
//! Bad checksums are reported, not rejected, because real sinks ship them.

use crate::ids::pnp_id;
use crate::sad::{Sad, parse_sads};

/// Size of one EDID block.
pub const BLOCK: usize = 128;
const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

/// Result of [`redact_serials`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdidRedaction {
    /// Not an EDID base block (empty, too short or bad header); bytes left untouched.
    NotEdid,
    /// Serial fields cleared and the base-block checksum fixed.
    Redacted {
        /// Whether a numeric serial (bytes 12–15) was non-zero.
        numeric_serial: bool,
        /// Whether a serial-number descriptor (tag 0xFF) was present.
        serial_descriptor: bool,
        /// Whether a DisplayID block carried a non-zero serial or ContainerID.
        displayid_serial: bool,
    },
}

/// Clears the numeric serial (bytes 12–15) and any serial-number display descriptor (tag 0xFF)
/// in the EDID base block, and in DisplayID extension blocks the product identification serial
/// (tag 0x00 / 0x20), the tiled-topology tile serial (0x12 / 0x28) and the ContainerID (0x29),
/// then fixes every
/// checksum it touched. Manufacturer, product code, name and other blocks are kept.
pub fn redact_serials(edid: &mut [u8]) -> EdidRedaction {
    if edid.len() < BLOCK || edid[..8] != HEADER {
        return EdidRedaction::NotEdid;
    }
    let numeric_serial = edid[12..16].iter().any(|&b| b != 0);
    edid[12..16].fill(0);
    let mut serial_descriptor = false;
    for off in [54, 72, 90, 108] {
        let d = &mut edid[off..off + 18];
        if d[0] == 0 && d[1] == 0 && d[2] == 0 && d[3] == 0xFF {
            serial_descriptor = true;
            let text = b"REDACTED\n    ";
            d[5..18].copy_from_slice(text);
        }
    }
    let sum: u32 = edid[..BLOCK - 1].iter().map(|&b| u32::from(b)).sum();
    edid[BLOCK - 1] = ((256 - (sum % 256)) % 256) as u8;
    let mut displayid_serial = false;
    for blk in edid[BLOCK..].as_chunks_mut::<BLOCK>().0 {
        if blk[0] == 0x70 && redact_displayid(blk) {
            displayid_serial = true;
        }
    }
    EdidRedaction::Redacted {
        numeric_serial,
        serial_descriptor,
        displayid_serial,
    }
}

/// Why an EDID could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdidError {
    /// Empty file: no sink connected.
    Empty,
    /// Shorter than one block.
    TooShort(usize),
    /// The 8-byte header is wrong.
    BadHeader,
}

impl std::fmt::Display for EdidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty EDID"),
            Self::TooShort(n) => write!(f, "EDID too short ({n} bytes)"),
            Self::BadHeader => write!(f, "bad EDID header"),
        }
    }
}

impl std::error::Error for EdidError {}

/// A decoded EDID.
#[derive(Debug, Clone, PartialEq)]
pub struct Edid {
    /// EDID version and revision, e.g. (1, 3).
    pub version: (u8, u8),
    /// Manufacturer id, big-endian as stored (bytes 8–9).
    pub manufacturer_id: u16,
    /// Three-letter PNP code, if valid.
    pub manufacturer: Option<String>,
    /// Product code (bytes 10–11, little-endian).
    pub product_code: u16,
    /// Week of manufacture (0 if not given, 255 = model year).
    pub week: u8,
    /// Year of manufacture or model year.
    pub year: u16,
    /// Display product name (descriptor 0xFC).
    pub name: Option<String>,
    /// Number of extension blocks the base block announces.
    pub extension_count: u8,
    /// Whether the base-block checksum is valid.
    pub checksum_ok: bool,
    /// The base block's video section: input, timings and descriptors.
    pub base: crate::timing::BaseVideo,
    /// Decoded CTA-861 extension blocks.
    pub cta: Vec<CtaBlock>,
    /// Decoded DisplayID extension blocks (tag 0x70).
    pub displayid: Vec<crate::displayid::DisplayId>,
    /// CTA-861 data blocks carried inside DisplayID sections (DisplayID data block 0x81),
    /// decoded like a CTA extension's video content.
    pub displayid_cta: Vec<crate::cta_video::CtaVideo>,
    /// Tags of extension blocks that are present but not decoded (0xF0 block map, …).
    pub other_extensions: Vec<u8>,
    /// Announced extension blocks missing from the buffer.
    pub missing_extensions: u8,
}

impl Edid {
    /// Every SAD across all CTA blocks, in order.
    pub fn sads(&self) -> impl Iterator<Item = &Sad> {
        self.cta.iter().flat_map(|c| c.sads.iter())
    }

    /// The HDMI VSDB, if any CTA block has one.
    pub fn hdmi(&self) -> Option<&HdmiVsdb> {
        self.cta.iter().find_map(|c| c.hdmi.as_ref())
    }

    /// The HDMI Forum VSDB, if any CTA block has one.
    pub fn hdmi_forum(&self) -> Option<&HdmiForumVsdb> {
        self.cta.iter().find_map(|c| c.hdmi_forum.as_ref())
    }

    /// The speaker allocation payload, if any CTA block has one.
    pub fn speaker_allocation(&self) -> Option<[u8; 3]> {
        self.cta.iter().find_map(|c| c.speaker_allocation)
    }
}

/// One CTA-861 extension block.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CtaBlock {
    /// Index of the block in the EDID (1 = first extension).
    pub block: u8,
    /// CTA revision.
    pub revision: u8,
    /// Sink supports basic audio (2-channel LPCM).
    pub basic_audio: bool,
    /// Whether the block checksum is valid.
    pub checksum_ok: bool,
    /// SADs from all audio data blocks, raw bytes kept.
    pub sads: Vec<Sad>,
    /// Speaker allocation data block payload.
    pub speaker_allocation: Option<[u8; 3]>,
    /// HDMI 1.x vendor-specific data block.
    pub hdmi: Option<HdmiVsdb>,
    /// HDMI Forum vendor-specific data block.
    pub hdmi_forum: Option<HdmiForumVsdb>,
    /// OUIs of all vendor-specific data blocks (tag 3), in order.
    pub vendor_ouis: Vec<u32>,
    /// Extended tags of all extended data blocks (tag 7), in order.
    pub extended_tags: Vec<u8>,
    /// The data block collection ran past its bounds.
    pub truncated: bool,
    /// Video content: SVDs, 4:2:0 maps, colour, HDR, HDMI 2.1 and vendor video blocks.
    pub video: crate::cta_video::CtaVideo,
}

/// HDMI Licensing VSDB (OUI 00-0C-03).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HdmiVsdb {
    /// CEC physical address, e.g. 0x2000 (input 2 of the root display).
    pub physical_address: u16,
    /// Supports ACP, ISRC1 and ISRC2 packets.
    pub supports_ai: bool,
    /// Maximum TMDS clock in MHz, if given.
    pub max_tmds_mhz: Option<u16>,
    /// Video latency in ms (progressive), if given.
    pub video_latency_ms: Option<Latency>,
    /// Audio latency in ms (progressive), if given.
    pub audio_latency_ms: Option<Latency>,
}

/// A latency field from the HDMI VSDB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Latency {
    /// Latency in ms.
    Ms(u16),
    /// The sink outputs no audio/video on this path (raw 255).
    NotSupported,
}

impl Latency {
    fn from_raw(v: u8) -> Option<Self> {
        match v {
            0 => None,
            255 => Some(Self::NotSupported),
            n => Some(Self::Ms((u16::from(n) - 1) * 2)),
        }
    }
}

/// HDMI Forum VSDB (OUI C4-5D-D8).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HdmiForumVsdb {
    /// Block version.
    pub version: u8,
    /// Maximum TMDS character rate in MHz (0 = use the HDMI VSDB value).
    pub max_tmds_mhz: u16,
    /// SCDC present.
    pub scdc: bool,
    /// Maximum FRL rate code (0 = FRL not supported, 1–6 = 3G3L … 12G4L).
    pub max_frl_rate: u8,
}

impl HdmiForumVsdb {
    /// Maximum FRL link rate in Gbit/s (total over all lanes), if FRL is supported.
    pub fn max_frl_gbps(&self) -> Option<u32> {
        match self.max_frl_rate {
            1 => Some(9),
            2 => Some(18),
            3 => Some(24),
            4 => Some(32),
            5 => Some(40),
            6 => Some(48),
            _ => None,
        }
    }
}

const OUI_HDMI: u32 = 0x00_0C03;
const OUI_HDMI_FORUM: u32 = 0xC4_5DD8;

fn checksum_ok(block: &[u8]) -> bool {
    block.iter().fold(0u8, |a, &b| a.wrapping_add(b)) == 0
}

/// Decodes an EDID with its extension blocks.
pub fn parse_edid(b: &[u8]) -> Result<Edid, EdidError> {
    if b.is_empty() {
        return Err(EdidError::Empty);
    }
    if b.len() < BLOCK {
        return Err(EdidError::TooShort(b.len()));
    }
    if b[..8] != HEADER {
        return Err(EdidError::BadHeader);
    }
    let manufacturer_id = u16::from_be_bytes([b[8], b[9]]);
    let mut name = None;
    for off in [54, 72, 90, 108] {
        let d = &b[off..off + 18];
        if d[0] == 0 && d[1] == 0 && d[3] == 0xFC {
            let t = &d[5..18];
            let t = match t.iter().position(|&c| c == b'\n') {
                Some(p) => &t[..p],
                None => t,
            };
            name = Some(String::from_utf8_lossy(t).trim_end().to_string());
        }
    }
    let extension_count = b[126];
    let available = (b.len() / BLOCK).saturating_sub(1);
    let n = usize::from(extension_count).min(available);
    let mut cta = Vec::new();
    let mut other_extensions = Vec::new();
    let mut displayid = Vec::new();
    let mut displayid_cta = Vec::new();
    for i in 1..=n {
        let blk = &b[i * BLOCK..(i + 1) * BLOCK];
        match blk[0] {
            0x02 => cta.push(parse_cta(blk, i as u8)),
            0x70 => {
                displayid.push(crate::displayid::parse_displayid(blk));
                displayid_cta.extend(displayid_cta_blocks(blk));
            }
            t => other_extensions.push(t),
        }
    }
    Ok(Edid {
        version: (b[18], b[19]),
        manufacturer_id,
        manufacturer: pnp_id(manufacturer_id),
        product_code: u16::from_le_bytes([b[10], b[11]]),
        week: b[16],
        year: 1990 + u16::from(b[17]),
        name,
        extension_count,
        checksum_ok: checksum_ok(&b[..BLOCK]),
        base: crate::timing::parse_base_video(&b[..BLOCK]),
        cta,
        displayid,
        displayid_cta,
        other_extensions,
        missing_extensions: extension_count.saturating_sub(n as u8),
    })
}

/// The CTA-861 data block collections inside a DisplayID section's 0x81 blocks, each decoded by
/// wrapping it in a synthetic CTA extension block (tag 0x02, revision 3, no detailed timings).
fn displayid_cta_blocks(blk: &[u8]) -> Vec<crate::cta_video::CtaVideo> {
    let mut out = Vec::new();
    let Some(&len) = blk.get(2) else {
        return out;
    };
    let end = (5 + usize::from(len))
        .min(blk.len().saturating_sub(1))
        .min(BLOCK - 1);
    let mut i = 5;
    while i + 3 <= end {
        let (tag, plen) = (blk[i], usize::from(blk[i + 2]));
        let payload = i + 3;
        if tag == 0 && plen == 0 || payload + plen > end {
            break;
        }
        if tag == 0x81 && plen > 0 && 4 + plen < BLOCK {
            let mut cta = vec![0u8; BLOCK];
            cta[0] = 0x02;
            cta[1] = 0x03;
            cta[2] = (4 + plen) as u8;
            cta[4..4 + plen].copy_from_slice(&blk[payload..payload + plen]);
            out.push(crate::cta_video::parse_cta_video(&cta));
        }
        i = payload + plen;
    }
    out
}

/// Zeroes the per-unit identifiers in a DisplayID extension block (EDID tag 0x70): the product
/// serial, the tiled-topology tile serial and the ContainerID. Then fixes the
/// DisplayID section checksum and the EDID block checksum. Returns whether a non-zero serial was
/// found. Malformed sections are left as they are.
fn redact_displayid(blk: &mut [u8]) -> bool {
    // blk[1] version, blk[2] bytes in section, blk[3] product type, blk[4] extension count,
    // data blocks from blk[5]; the section checksum follows the data blocks.
    let len = usize::from(blk[2]);
    let end = 5 + len;
    if end >= BLOCK - 1 {
        return false;
    }
    let mut found = false;
    let mut i = 5;
    while i + 3 <= end {
        let (tag, plen) = (blk[i], usize::from(blk[i + 2]));
        let payload = i + 3;
        if payload + plen > end {
            break;
        }
        // Per-unit identifiers: the product-ID serial (3-byte id, 2-byte product, 4-byte serial),
        // the tiled-topology tile serial (payload bytes 18..22) and the ContainerID UUID.
        let range = match tag {
            0x00 | 0x20 if plen >= 9 => Some(payload + 5..payload + 9),
            0x12 | 0x28 if plen >= 22 => Some(payload + 18..payload + 22),
            0x29 if plen >= 16 => Some(payload..payload + 16),
            _ => None,
        };
        if let Some(r) = range {
            let field = &mut blk[r];
            if field.iter().any(|&b| b != 0) {
                found = true;
                field.fill(0);
            }
        }
        i = payload + plen;
    }
    if found {
        let sum: u32 = blk[1..end].iter().map(|&b| u32::from(b)).sum();
        blk[end] = ((256 - (sum % 256)) % 256) as u8;
        let sum: u32 = blk[..BLOCK - 1].iter().map(|&b| u32::from(b)).sum();
        blk[BLOCK - 1] = ((256 - (sum % 256)) % 256) as u8;
    }
    found
}

/// The serial that feeds the sink key, exactly as AudioCaps v1.0.0 derives it
/// (`crates/audiocaps-backends/src/assemble/sink.rs`, `edid_serial`): the numeric serial (bytes
/// 12–15, little-endian, when non-zero) and every serial string descriptor (0xFF), joined by `/`.
/// Redacted EDIDs (serial bytes zeroed, descriptor text `REDACTED`) give `None`.
pub fn serial(raw: &[u8]) -> Option<String> {
    let mut parts = vec![];
    if raw.len() >= 16 {
        let n = u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]);
        if n != 0 {
            parts.push(n.to_string());
        }
    }
    for off in [54usize, 72, 90, 108] {
        let Some(d) = raw.get(off..off + 18) else {
            continue;
        };
        if d[..4] == [0, 0, 0, 0xff] {
            let s: String = d[5..]
                .iter()
                .take_while(|b| **b != b'\n')
                .map(|b| *b as char)
                .collect();
            let s = s.trim().to_string();
            if !s.is_empty() && s != "REDACTED" {
                parts.push(s);
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Decodes one 128-byte CTA-861 extension block.
pub fn parse_cta(blk: &[u8], index: u8) -> CtaBlock {
    let mut c = CtaBlock {
        block: index,
        ..CtaBlock::default()
    };
    if blk.len() < 4 {
        c.truncated = true;
        return c;
    }
    c.revision = blk[1];
    c.video = crate::cta_video::parse_cta_video(blk);
    c.basic_audio = blk[3] & 0x40 != 0;
    c.checksum_ok = blk.len() == BLOCK && checksum_ok(blk);
    // Byte 2: offset of the first detailed timing; data blocks live in 4..d. 0 means none.
    let d = usize::from(blk[2]);
    let end = if d == 0 { 4 } else { d.min(blk.len()) };
    if d != 0 && (d < 4 || d > blk.len()) {
        c.truncated = true;
    }
    let mut i = 4;
    while i < end {
        let tag = blk[i] >> 5;
        let len = usize::from(blk[i] & 0x1F);
        let start = i + 1;
        if start + len > end {
            c.truncated = true;
            break;
        }
        let p = &blk[start..start + len];
        match tag {
            1 => c.sads.extend(parse_sads(p)),
            3 if len >= 3 => {
                let oui = u32::from(p[0]) | (u32::from(p[1]) << 8) | (u32::from(p[2]) << 16);
                c.vendor_ouis.push(oui);
                match oui {
                    OUI_HDMI if c.hdmi.is_none() => c.hdmi = Some(parse_hdmi_vsdb(p)),
                    OUI_HDMI_FORUM if c.hdmi_forum.is_none() => {
                        c.hdmi_forum = Some(parse_hf_vsdb(p))
                    }
                    _ => {}
                }
            }
            4 if len >= 3 => c.speaker_allocation = Some([p[0], p[1], p[2]]),
            4 if len >= 1 => c.speaker_allocation = Some([p[0], 0, 0]),
            7 if len >= 1 => c.extended_tags.push(p[0]),
            _ => {}
        }
        i = start + len;
    }
    c
}

/// Decodes an HDMI VSDB payload (OUI included).
fn parse_hdmi_vsdb(p: &[u8]) -> HdmiVsdb {
    let at = |i: usize| p.get(i).copied();
    let mut h = HdmiVsdb {
        physical_address: match (at(3), at(4)) {
            (Some(a), Some(b)) => u16::from_be_bytes([a, b]),
            _ => 0xFFFF,
        },
        ..HdmiVsdb::default()
    };
    if let Some(f) = at(5) {
        h.supports_ai = f & 0x80 != 0;
    }
    h.max_tmds_mhz = at(6).filter(|&v| v != 0).map(|v| u16::from(v) * 5);
    if let Some(f) = at(7)
        && f & 0x80 != 0
    {
        h.video_latency_ms = at(8).and_then(Latency::from_raw);
        h.audio_latency_ms = at(9).and_then(Latency::from_raw);
    }
    h
}

/// Decodes an HDMI Forum VSDB payload (OUI included).
fn parse_hf_vsdb(p: &[u8]) -> HdmiForumVsdb {
    let at = |i: usize| p.get(i).copied().unwrap_or(0);
    HdmiForumVsdb {
        version: at(3),
        max_tmds_mhz: u16::from(at(4)) * 5,
        scdc: at(5) & 0x80 != 0,
        max_frl_rate: at(6) >> 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut e = vec![0u8; 256];
        e[..8].copy_from_slice(&HEADER);
        e[12..16].copy_from_slice(&[1, 2, 3, 4]);
        e[72..77].copy_from_slice(&[0, 0, 0, 0xFF, 0]);
        e[77..90].copy_from_slice(b"SN1234567\n   ");
        e[90..95].copy_from_slice(&[0, 0, 0, 0xFC, 0]);
        e[95..108].copy_from_slice(b"AVR-S660H\n   ");
        e[128] = 0x02; // CTA extension tag
        e
    }

    #[test]
    fn clears_serials_keeps_name_and_fixes_checksum() {
        let mut e = sample();
        let r = redact_serials(&mut e);
        assert_eq!(
            r,
            EdidRedaction::Redacted {
                numeric_serial: true,
                serial_descriptor: true,
                displayid_serial: false
            }
        );
        assert_eq!(&e[12..16], &[0, 0, 0, 0]);
        assert_eq!(&e[77..86], b"REDACTED\n");
        assert_eq!(&e[95..104], b"AVR-S660H");
        let sum: u32 = e[..128].iter().map(|&b| u32::from(b)).sum();
        assert_eq!(sum % 256, 0);
        assert_eq!(e[128], 0x02);
    }

    fn with_cta(dbc: &[u8]) -> Vec<u8> {
        let mut e = sample();
        e[126] = 1;
        e[90..95].copy_from_slice(&[0, 0, 0, 0xFC, 0]);
        e[8..12].copy_from_slice(&[0x20, 0xA3, 0x30, 0xA0]);
        let c = &mut e[128..];
        c[0] = 2;
        c[1] = 3;
        c[2] = (4 + dbc.len()) as u8;
        c[3] = 0x40;
        c[4..4 + dbc.len()].copy_from_slice(dbc);
        let sum = c[..127].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        c[127] = 0u8.wrapping_sub(sum);
        e
    }

    #[test]
    fn parses_cta_blocks() {
        let dbc = [
            0x26, 0x57, 0x06, 0x01, 0x0F, 0x7F, 0x07, // audio: E-AC-3 JOC, LPCM 8ch
            0x83, 0x4F, 0x00, 0x00, // speaker allocation
            0x6A, 0x03, 0x0C, 0x00, 0x20, 0x00, 0x80, 0x44, 0xA0, 0x0B,
            0x06, // HDMI VSDB + latency
            0x67, 0xD8, 0x5D, 0xC4, 0x01, 0x78, 0x80, 0x30, // HF-VSDB, FRL 3
            0xE2, 0x0F, 0x00, // extended tag 15
        ];
        let e = parse_edid(&with_cta(&dbc)).unwrap();
        assert_eq!(e.manufacturer.as_deref(), Some("HEC"));
        assert_eq!(e.product_code, 0xA030);
        assert_eq!(e.name.as_deref(), Some("AVR-S660H"));
        assert_eq!(e.cta.len(), 1);
        let c = &e.cta[0];
        assert!(c.basic_audio && c.checksum_ok && !c.truncated);
        assert_eq!(c.sads.len(), 2);
        assert_eq!(c.speaker_allocation, Some([0x4F, 0, 0]));
        let h = e.hdmi().unwrap();
        assert_eq!(h.physical_address, 0x2000);
        assert!(h.supports_ai);
        assert_eq!(h.max_tmds_mhz, Some(340));
        assert_eq!(h.video_latency_ms, Some(Latency::Ms(20)));
        assert_eq!(h.audio_latency_ms, Some(Latency::Ms(10)));
        let f = e.hdmi_forum().unwrap();
        assert_eq!(
            (f.max_tmds_mhz, f.scdc, f.max_frl_gbps()),
            (600, true, Some(24))
        );
        assert_eq!(c.vendor_ouis, vec![OUI_HDMI, OUI_HDMI_FORUM]);
        assert_eq!(c.extended_tags, vec![15]);
    }

    #[test]
    fn clears_displayid_product_serial() {
        let mut b = sample();
        b.truncate(BLOCK);
        b[126] = 1;
        let mut ext = vec![0u8; BLOCK];
        ext[0] = 0x70; // DisplayID extension
        ext[1] = 0x20; // DisplayID 2.0
        ext[2] = 12; // section length: one 3+9-byte product id block
        let blk = [
            0x20, 0x00, 9, 0x00, 0x00, 0x5e, 0x34, 0x12, 0xaa, 0xbb, 0xcc, 0xdd,
        ];
        ext[5..17].copy_from_slice(&blk);
        let fix = |e: &mut Vec<u8>| {
            let s: u32 = e[1..17].iter().map(|&x| u32::from(x)).sum();
            e[17] = ((256 - (s % 256)) % 256) as u8;
            let s: u32 = e[..127].iter().map(|&x| u32::from(x)).sum();
            e[127] = ((256 - (s % 256)) % 256) as u8;
        };
        fix(&mut ext);
        b.extend_from_slice(&ext);
        let r = redact_serials(&mut b);
        assert!(matches!(
            r,
            EdidRedaction::Redacted {
                displayid_serial: true,
                ..
            }
        ));
        let e = &b[BLOCK..];
        assert_eq!(&e[13..17], &[0, 0, 0, 0], "serial zeroed");
        assert_eq!(&e[5..13], &blk[..8], "id and product kept");
        assert_eq!(e[1..18].iter().map(|&x| u32::from(x)).sum::<u32>() % 256, 0);
        assert_eq!(e.iter().map(|&x| u32::from(x)).sum::<u32>() % 256, 0);
    }

    #[test]
    fn clears_tile_serial_and_container_id() {
        let mut b = sample();
        b.truncate(BLOCK);
        b[126] = 1;
        let mut ext = vec![0u8; BLOCK];
        ext[0] = 0x70;
        ext[1] = 0x20;
        // A 0x28 tiled-topology block (22-byte payload) and a 0x29 ContainerID (16 bytes).
        let mut tile = vec![0x28, 0x00, 22];
        tile.extend((1..=22u8).collect::<Vec<_>>());
        let mut cid = vec![0x29, 0x00, 16];
        cid.extend([0xAB; 16]);
        let body: Vec<u8> = tile.iter().chain(cid.iter()).copied().collect();
        ext[2] = body.len() as u8;
        ext[5..5 + body.len()].copy_from_slice(&body);
        let end = 5 + body.len();
        let s: u32 = ext[1..end].iter().map(|&x| u32::from(x)).sum();
        ext[end] = ((256 - (s % 256)) % 256) as u8;
        let s: u32 = ext[..127].iter().map(|&x| u32::from(x)).sum();
        ext[127] = ((256 - (s % 256)) % 256) as u8;
        b.extend_from_slice(&ext);
        assert!(matches!(
            redact_serials(&mut b),
            EdidRedaction::Redacted {
                displayid_serial: true,
                ..
            }
        ));
        let e = &b[BLOCK..];
        assert_eq!(&e[8 + 18..8 + 22], &[0, 0, 0, 0], "tile serial zeroed");
        assert_eq!(&e[8..8 + 18], &tile[3..21], "rest of the tile block kept");
        assert_eq!(&e[33..49], &[0u8; 16], "ContainerID zeroed");
        assert_eq!(
            e[1..end + 1].iter().map(|&x| u32::from(x)).sum::<u32>() % 256,
            0
        );
        assert_eq!(e.iter().map(|&x| u32::from(x)).sum::<u32>() % 256, 0);
    }

    #[test]
    fn truncated_and_missing() {
        let mut e = with_cta(&[0x3F, 1, 2]);
        assert!(parse_edid(&e).unwrap().cta[0].truncated);
        e[126] = 3;
        assert_eq!(parse_edid(&e).unwrap().missing_extensions, 2);
        assert_eq!(parse_edid(&e[..100]), Err(EdidError::TooShort(100)));
        assert_eq!(parse_edid(&[]), Err(EdidError::Empty));
        assert_eq!(parse_edid(&[1u8; 128]), Err(EdidError::BadHeader));
    }

    #[test]
    fn not_edid_untouched() {
        let mut e = vec![1u8; 10];
        assert_eq!(redact_serials(&mut e), EdidRedaction::NotEdid);
        assert_eq!(e, vec![1u8; 10]);
        let mut empty: Vec<u8> = vec![];
        assert_eq!(redact_serials(&mut empty), EdidRedaction::NotEdid);
    }
}
