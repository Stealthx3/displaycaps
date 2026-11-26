// SPDX-License-Identifier: LGPL-2.1-or-later
//! KMS state as plain data ([`KmsCard`]), from one of two sources: the live read-only ioctls
//! (`kms_live`) or a fixture's captured `drm_info -j` output ([`from_drm_info`]). Both go through
//! the same mapping into the model ([`kms_state`], [`modes`]), which is also what the phase 3 gate
//! compares against drm_info on a live system.

use displaycaps_model::{
    ColorFormat, Eotf, FieldGroup, HdrMetadata, KmsState, LayerKind, Mode, ModeSource, Provenance,
    Selection, Source, Timing, UnknownReason, Value,
};
use serde::Serialize;

/// One card's KMS objects.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct KmsCard {
    /// `card0`.
    pub card: String,
    /// Kernel driver name from `DRM_IOCTL_VERSION`, e.g. `amdgpu`.
    pub driver: String,
    /// Connectors.
    pub connectors: Vec<KmsConnector>,
    /// CRTCs.
    pub crtcs: Vec<KmsCrtc>,
    /// Where the data came from.
    #[serde(skip)]
    pub from: KmsFrom,
}

/// Where KMS data came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum KmsFrom {
    /// Read-only ioctls on the live system.
    #[default]
    Live,
    /// A fixture's `drm_info -j` output.
    DrmInfo,
}

/// A connector.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct KmsConnector {
    /// Object id.
    pub id: u32,
    /// `DRM_MODE_CONNECTOR_*`.
    pub type_code: u32,
    /// 1 connected, 2 disconnected, 3 unknown.
    pub status: u32,
    /// The CRTC it drives (through its encoder), if any.
    pub crtc: Option<u32>,
    /// The kernel's cached mode list (never re-probed).
    pub modes: Vec<KmsMode>,
    /// Properties.
    pub props: Vec<KmsProp>,
}

/// A CRTC.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct KmsCrtc {
    /// Object id.
    pub id: u32,
    /// The mode it scans out, if any.
    pub mode: Option<KmsMode>,
    /// Properties.
    pub props: Vec<KmsProp>,
}

/// A KMS mode (`struct drm_mode_modeinfo`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct KmsMode {
    pub clock_khz: u32,
    pub hdisplay: u16,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub vdisplay: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub vscan: u16,
    pub flags: u32,
    pub mode_type: u32,
}

/// A property and its current value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct KmsProp {
    /// Name, e.g. `max bpc`.
    pub name: String,
    /// Raw value (for a blob, the blob id; 0 is no blob).
    pub raw: u64,
    /// For an enum, the name of the current value.
    pub enum_name: Option<String>,
    /// For a blob that is read (EDID, HDR_OUTPUT_METADATA), its bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<Vec<u8>>,
    /// For a blob a fixture decoded already (drm_info's JSON form), the decoded HDR metadata.
    #[serde(skip)]
    pub hdr: Option<Option<HdrMetadata>>,
}

const FLAG_PHSYNC: u32 = 1;
const FLAG_NHSYNC: u32 = 2;
const FLAG_PVSYNC: u32 = 4;
const FLAG_NVSYNC: u32 = 8;
const FLAG_INTERLACE: u32 = 0x10;
const FLAG_DBLSCAN: u32 = 0x20;
const TYPE_PREFERRED: u32 = 0x8;

impl KmsMode {
    /// The vertical refresh in millihertz (field rate for interlaced modes), as the kernel's
    /// `drm_mode_vrefresh` works it out, but without rounding to whole hertz.
    pub fn refresh_millihz(&self) -> u32 {
        let mut den = u64::from(self.htotal) * u64::from(self.vtotal);
        if den == 0 {
            return 0;
        }
        let mut num = u64::from(self.clock_khz) * 1_000_000;
        if self.flags & FLAG_INTERLACE != 0 {
            num *= 2;
        }
        if self.flags & FLAG_DBLSCAN != 0 {
            den *= 2;
        }
        if self.vscan > 1 {
            den *= u64::from(self.vscan);
        }
        u32::try_from((num + den / 2) / den).unwrap_or(u32::MAX)
    }

    /// The model's mode.
    pub fn to_mode(&self) -> Mode {
        let il = self.flags & FLAG_INTERLACE != 0;
        let pol = |p: u32, n: u32| match (self.flags & p != 0, self.flags & n != 0) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        };
        Mode {
            width: self.hdisplay,
            height: self.vdisplay,
            refresh_millihz: self.refresh_millihz(),
            interlaced: il,
            timing: Some(Timing {
                pixel_clock_khz: self.clock_khz,
                h_total: self.htotal,
                v_total: self.vtotal,
                h_sync_positive: pol(FLAG_PHSYNC, FLAG_NHSYNC),
                v_sync_positive: pol(FLAG_PVSYNC, FLAG_NVSYNC),
            }),
            vic: None,
            hdmi_vic: None,
            preferred: self.mode_type & TYPE_PREFERRED != 0,
            ycbcr420: Default::default(),
            sources: vec![ModeSource::Kms],
        }
    }
}

impl KmsConnector {
    /// A property by name.
    pub fn prop(&self, name: &str) -> Option<&KmsProp> {
        self.props.iter().find(|p| p.name == name)
    }
}

impl KmsCrtc {
    /// A property by name.
    pub fn prop(&self, name: &str) -> Option<&KmsProp> {
        self.props.iter().find(|p| p.name == name)
    }
}

/// Decodes an `HDR_OUTPUT_METADATA` blob (`struct hdr_output_metadata` with a type-1
/// `hdr_metadata_infoframe`). `None` for a blob too short or of another metadata type.
pub(crate) fn decode_hdr_metadata(b: &[u8]) -> Option<HdrMetadata> {
    if b.len() < 30 {
        return None;
    }
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let metadata_type = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    if metadata_type != 0 {
        return None;
    }
    let eotf = match b[4] {
        0 => Eotf::TraditionalSdr,
        1 => Eotf::TraditionalHdr,
        2 => Eotf::Pq,
        3 => Eotf::Hlg,
        _ => return None,
    };
    let nz = |v: u16| (v != 0).then_some(v);
    Some(HdrMetadata {
        eotf,
        max_mastering_nits: nz(u16_at(22)).map(f32::from),
        // In units of 0.0001 cd/m².
        min_mastering_nits: nz(u16_at(24)).map(|v| f32::from(v) / 10_000.0),
        max_cll_nits: nz(u16_at(26)),
        max_fall_nits: nz(u16_at(28)),
    })
}

fn color_format(name: &str) -> Option<ColorFormat> {
    match name
        .to_ascii_lowercase()
        .replace([' ', '_', ':'], "")
        .as_str()
    {
        "rgb" | "rgb444" => Some(ColorFormat::Rgb),
        "ycbcr444" => Some(ColorFormat::Ycbcr444),
        "ycbcr422" => Some(ColorFormat::Ycbcr422),
        "ycbcr420" => Some(ColorFormat::Ycbcr420),
        _ => None,
    }
}

/// The KMS state of one connector for the field groups in scope. Properties the driver doesn't
/// have are left out (`None`), never guessed.
pub(crate) fn kms_state(
    card: &KmsCard,
    conn: &KmsConnector,
    fields: &Selection<FieldGroup>,
    now: u64,
) -> KmsState {
    let crtc = conn
        .crtc
        .and_then(|id| card.crtcs.iter().find(|c| c.id == id));
    let prov = |object: String, property: &str| Provenance {
        layer: LayerKind::Kms,
        source: match card.from {
            KmsFrom::Live => Source::KmsProperty {
                object,
                property: property.to_string(),
            },
            KmsFrom::DrmInfo => Source::Command {
                name: "drm_info".into(),
            },
        },
        read_at_ms: now,
    };
    let cobj = || format!("connector {}", conn.id);
    let want = |g: FieldGroup| fields.includes(&g);
    let cprop = |name: &str| conn.prop(name).map(|p| (p, prov(cobj(), name)));
    let enum_val = |name: &str| {
        cprop(name).map(|(p, pv)| match &p.enum_name {
            Some(n) => Value::measured(n.clone(), pv),
            None => Value::unknown(UnknownReason::ParseError, format!("{name}: no enum name")),
        })
    };
    let active_mode = match crtc {
        None => Value::measured(None, prov(cobj(), "CRTC_ID")),
        Some(c) => {
            // A CRTC with ACTIVE = 0 keeps its mode but scans nothing out.
            let active = c.prop("ACTIVE").is_none_or(|p| p.raw != 0);
            Value::measured(
                c.mode.as_ref().filter(|_| active).map(KmsMode::to_mode),
                prov(format!("crtc {}", c.id), "MODE_ID"),
            )
        }
    };
    KmsState {
        active_mode: if want(FieldGroup::Modes) {
            active_mode
        } else {
            Value::unknown(UnknownReason::NotRequested, None)
        },
        max_bpc: want(FieldGroup::ColorFormats)
            .then(|| cprop("max bpc"))
            .flatten()
            .map(|(p, pv)| match u8::try_from(p.raw) {
                Ok(v) => Value::measured(v, pv),
                Err(_) => Value::unknown(
                    UnknownReason::ParseError,
                    "max bpc out of range".to_string(),
                ),
            }),
        colorspace: want(FieldGroup::Colorimetry)
            .then(|| enum_val("Colorspace"))
            .flatten(),
        color_format: want(FieldGroup::ColorFormats)
            .then(|| cprop("color format"))
            .flatten()
            .map(
                |(p, pv)| match p.enum_name.as_deref().and_then(color_format) {
                    Some(f) => Value::measured(f, pv),
                    None => Value::unknown(
                        UnknownReason::NotExposed,
                        format!(
                            "color format is {:?}: the driver chooses",
                            p.enum_name.as_deref().unwrap_or("?")
                        ),
                    ),
                },
            ),
        hdr_output_metadata: want(FieldGroup::Hdr)
            .then(|| cprop("HDR_OUTPUT_METADATA"))
            .flatten()
            .map(|(p, pv)| {
                if p.raw == 0 {
                    return Value::measured(None, pv);
                }
                if let Some(h) = &p.hdr {
                    return Value::measured(h.clone(), pv);
                }
                match p.blob.as_deref().map(decode_hdr_metadata) {
                    Some(Some(h)) => Value::measured(Some(h), pv),
                    Some(None) => Value::unknown(
                        UnknownReason::ParseError,
                        "HDR_OUTPUT_METADATA blob not decodable".to_string(),
                    ),
                    None => Value::unknown(
                        UnknownReason::ReadError,
                        "HDR_OUTPUT_METADATA blob not read".to_string(),
                    ),
                }
            }),
        broadcast_rgb: want(FieldGroup::Quantization)
            .then(|| enum_val("Broadcast RGB"))
            .flatten(),
        vrr_capable: want(FieldGroup::Vrr)
            .then(|| cprop("vrr_capable"))
            .flatten()
            .map(|(p, pv)| Value::measured(p.raw != 0, pv)),
        vrr_enabled: want(FieldGroup::Vrr)
            .then(|| crtc.and_then(|c| c.prop("VRR_ENABLED").map(|p| (c, p))))
            .flatten()
            .map(|(c, p)| {
                Value::measured(p.raw != 0, prov(format!("crtc {}", c.id), "VRR_ENABLED"))
            }),
        content_type: want(FieldGroup::Compositor)
            .then(|| enum_val("content type"))
            .flatten(),
        link_status: want(FieldGroup::Link)
            .then(|| enum_val("link-status"))
            .flatten(),
        panel_orientation: want(FieldGroup::Identity)
            .then(|| enum_val("panel orientation"))
            .flatten(),
    }
}

/// The connector's cached KMS mode list as model modes.
pub(crate) fn modes(conn: &KmsConnector) -> Vec<Mode> {
    conn.modes.iter().map(KmsMode::to_mode).collect()
}

/// Reads a fixture's `drm_info -j` output. Unknown or malformed entries are skipped.
pub(crate) fn from_drm_info(json: &[u8]) -> Result<Vec<KmsCard>, String> {
    use serde_json::Value as J;
    let doc: J = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    let obj = doc.as_object().ok_or("drm_info: not an object")?;
    let u = |v: &J, k: &str| v.get(k).and_then(J::as_u64).unwrap_or(0);
    let mode = |m: &J| -> Option<KmsMode> {
        let n = |k: &str| u16::try_from(u(m, k)).ok();
        Some(KmsMode {
            clock_khz: u32::try_from(u(m, "clock")).ok()?,
            hdisplay: n("hdisplay")?,
            hsync_start: n("hsync_start")?,
            hsync_end: n("hsync_end")?,
            htotal: n("htotal")?,
            vdisplay: n("vdisplay")?,
            vsync_start: n("vsync_start")?,
            vsync_end: n("vsync_end")?,
            vtotal: n("vtotal")?,
            vscan: n("vscan")?,
            flags: u32::try_from(u(m, "flags")).ok()?,
            mode_type: u32::try_from(u(m, "type")).ok()?,
        })
    };
    let props = |o: &J| -> Vec<KmsProp> {
        let Some(map) = o.get("properties").and_then(J::as_object) else {
            return vec![];
        };
        map.iter()
            .map(|(name, p)| {
                let raw = u(p, "raw_value");
                let enum_name = p.get("spec").and_then(J::as_array).and_then(|spec| {
                    spec.iter()
                        .find(|e| e.get("value").and_then(J::as_u64) == Some(raw))
                        .and_then(|e| e.get("name").and_then(J::as_str))
                        .map(str::to_string)
                });
                let hdr = (name == "HDR_OUTPUT_METADATA" && raw != 0)
                    .then(|| p.get("data").map(drm_info_hdr));
                KmsProp {
                    name: name.clone(),
                    raw,
                    enum_name,
                    blob: None,
                    hdr: hdr.flatten(),
                }
            })
            .collect()
    };
    let mut cards = vec![];
    for (path, dev) in obj {
        let Some(card) = path.rsplit('/').next().filter(|c| c.starts_with("card")) else {
            continue;
        };
        let arr = |k: &str| {
            dev.get(k)
                .and_then(J::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let encoders = arr("encoders");
        let enc_crtc = |id: u64| {
            encoders
                .iter()
                .find(|e| u(e, "id") == id)
                .map(|e| u(e, "crtc_id"))
                .filter(|&c| c != 0)
                .and_then(|c| u32::try_from(c).ok())
        };
        let connectors = arr("connectors")
            .iter()
            .map(|c| KmsConnector {
                id: u32::try_from(u(c, "id")).unwrap_or(0),
                type_code: u32::try_from(u(c, "type")).unwrap_or(0),
                status: u32::try_from(u(c, "status")).unwrap_or(3),
                crtc: enc_crtc(u(c, "encoder_id")),
                modes: c
                    .get("modes")
                    .and_then(J::as_array)
                    .map(|m| m.iter().filter_map(mode).collect())
                    .unwrap_or_default(),
                props: props(c),
            })
            .collect();
        let crtcs = arr("crtcs")
            .iter()
            .map(|c| KmsCrtc {
                id: u32::try_from(u(c, "id")).unwrap_or(0),
                // drm_info prints an all-zero mode for a CRTC without one.
                mode: c.get("mode").and_then(mode).filter(|m| m.clock_khz != 0),
                props: props(c),
            })
            .collect();
        cards.push(KmsCard {
            card: card.to_string(),
            driver: dev
                .pointer("/driver/name")
                .and_then(J::as_str)
                .unwrap_or_default()
                .to_string(),
            connectors,
            crtcs,
            from: KmsFrom::DrmInfo,
        });
    }
    Ok(cards)
}

/// drm_info's decoded `HDR_OUTPUT_METADATA` (its `data` object). The key names follow the kernel
/// struct; a form that doesn't match gives `None` (not decodable).
fn drm_info_hdr(d: &serde_json::Value) -> Option<HdrMetadata> {
    let inner = d.get("hdmi_metadata_type1").unwrap_or(d);
    let n = |k: &str| inner.get(k).and_then(serde_json::Value::as_u64);
    let eotf = match n("eotf")? {
        0 => Eotf::TraditionalSdr,
        1 => Eotf::TraditionalHdr,
        2 => Eotf::Pq,
        3 => Eotf::Hlg,
        _ => return None,
    };
    let nz16 = |k: &str| n(k).filter(|&v| v != 0).and_then(|v| u16::try_from(v).ok());
    Some(HdrMetadata {
        eotf,
        max_mastering_nits: nz16("max_display_mastering_luminance").map(f32::from),
        min_mastering_nits: nz16("min_display_mastering_luminance")
            .map(|v| f32::from(v) / 10_000.0),
        max_cll_nits: nz16("max_cll"),
        max_fall_nits: nz16("max_fall"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<KmsCard> {
        let p = format!(
            "{}/../../tests/fixtures/{name}/commands/drm_info.json",
            env!("CARGO_MANIFEST_DIR")
        );
        from_drm_info(&std::fs::read(p).unwrap()).unwrap()
    }

    #[test]
    fn drm_info_fixture() {
        let cards = fixture("lab-gnome");
        assert_eq!(cards.len(), 1);
        let c = &cards[0];
        assert_eq!(
            (c.card.as_str(), c.driver.as_str()),
            ("card0", "virtio_gpu")
        );
        let conn = &c.connectors[0];
        assert_eq!(
            (conn.id, conn.type_code, conn.status, conn.crtc),
            (37, 15, 1, Some(36))
        );
        assert_eq!(
            conn.prop("link-status").unwrap().enum_name.as_deref(),
            Some("Good")
        );
        let s = kms_state(c, conn, &Selection::All, 5);
        let m = s.active_mode.known().unwrap().as_ref().unwrap();
        assert_eq!((m.width, m.height), (1280, 800));
        // 107.3 MHz / (1728 × 828) = 74.9937 Hz.
        assert_eq!(m.refresh_millihz, 74_994);
        assert_eq!(
            s.link_status.unwrap().known().map(String::as_str),
            Some("Good")
        );
        assert_eq!(s.vrr_enabled.unwrap().known(), Some(&false));
        assert!(
            s.max_bpc.is_none(),
            "virtio has no max bpc: omitted, not guessed"
        );
        assert!(!modes(conn).is_empty());
    }

    #[test]
    fn scoped_fields() {
        let cards = fixture("lab-gnome");
        let s = kms_state(
            &cards[0],
            &cards[0].connectors[0],
            &Selection::Only(vec![FieldGroup::Vrr]),
            5,
        );
        assert!(s.active_mode.known().is_none());
        assert!(s.link_status.is_none() && s.vrr_enabled.is_some());
    }

    #[test]
    fn hdr_blob() {
        let mut b = vec![0u8; 32];
        b[4] = 2; // PQ
        b[22..24].copy_from_slice(&1000u16.to_le_bytes());
        b[24..26].copy_from_slice(&50u16.to_le_bytes());
        b[26..28].copy_from_slice(&800u16.to_le_bytes());
        b[28..30].copy_from_slice(&400u16.to_le_bytes());
        let h = decode_hdr_metadata(&b).unwrap();
        assert_eq!(h.eotf, Eotf::Pq);
        assert_eq!(h.max_mastering_nits, Some(1000.0));
        assert_eq!(h.min_mastering_nits, Some(0.005));
        assert_eq!((h.max_cll_nits, h.max_fall_nits), (Some(800), Some(400)));
        assert!(decode_hdr_metadata(&b[..20]).is_none());
        b[0] = 1;
        assert!(
            decode_hdr_metadata(&b).is_none(),
            "only static metadata type 1"
        );
    }

    #[test]
    fn properties_map() {
        let p = |name: &str, raw: u64, e: Option<&str>| KmsProp {
            name: name.into(),
            raw,
            enum_name: e.map(str::to_string),
            blob: None,
            hdr: None,
        };
        let card = KmsCard {
            card: "card1".into(),
            driver: "amdgpu".into(),
            connectors: vec![KmsConnector {
                id: 98,
                type_code: 11,
                status: 1,
                crtc: Some(77),
                modes: vec![],
                props: vec![
                    p("max bpc", 10, None),
                    p("Colorspace", 9, Some("BT2020_RGB")),
                    p("Broadcast RGB", 0, Some("Automatic")),
                    p("color format", 0, Some("AUTO")),
                    p("HDR_OUTPUT_METADATA", 0, None),
                    p("vrr_capable", 1, None),
                ],
            }],
            crtcs: vec![KmsCrtc {
                id: 77,
                mode: None,
                props: vec![p("ACTIVE", 1, None), p("VRR_ENABLED", 1, None)],
            }],
            from: KmsFrom::Live,
        };
        let s = kms_state(&card, &card.connectors[0], &Selection::All, 1);
        assert_eq!(s.max_bpc.unwrap().known(), Some(&10));
        assert_eq!(
            s.colorspace.unwrap().known().map(String::as_str),
            Some("BT2020_RGB")
        );
        assert_eq!(
            s.broadcast_rgb.unwrap().known().map(String::as_str),
            Some("Automatic")
        );
        assert!(
            s.color_format.unwrap().known().is_none(),
            "AUTO is the driver's choice"
        );
        assert_eq!(s.hdr_output_metadata.unwrap().known(), Some(&None));
        assert_eq!(s.vrr_capable.unwrap().known(), Some(&true));
        assert_eq!(s.vrr_enabled.unwrap().known(), Some(&true));
        assert_eq!(s.active_mode.known(), Some(&None));
        assert!(s.panel_orientation.is_none());
    }
}
