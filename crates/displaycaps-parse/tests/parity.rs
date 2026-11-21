// SPDX-License-Identifier: LGPL-2.1-or-later
//! Sink-key parity with AudioCaps v1.0.0. For every shared EDID vector in
//! `tests/fixtures/parity/edid/`, DisplayCaps must derive the same sink identity that AudioCaps
//! itself reported (`tests/fixtures/parity/audiocaps-v1.0.0.json`, produced by running the
//! AudioCaps v1.0.0 binary in the lab: `scripts/parity-audiocaps.sh`). The combined library joins
//! a display endpoint and its HDMI/DP audio endpoint on this key.

use std::path::Path;

use displaycaps_model::sink_key;
use displaycaps_parse::edid::{parse_edid, serial};
use displaycaps_parse::ids::pnp_id;

fn fixtures() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/parity")
}

#[test]
fn sink_identity_matches_audiocaps() {
    let doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures().join("audiocaps-v1.0.0.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(doc["audiocaps"]["version"], "1.0.0");
    let vectors = doc["vectors"].as_object().unwrap();
    let mut files: Vec<_> = std::fs::read_dir(fixtures().join("edid"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert_eq!(
        files.len(),
        vectors.len(),
        "every vector has an AudioCaps result"
    );
    assert!(files.len() >= 9);
    for f in files {
        let name = f.file_stem().unwrap().to_str().unwrap();
        let expect = &vectors[name];
        let raw = std::fs::read(&f).unwrap();
        let edid = parse_edid(&raw).unwrap();
        // AudioCaps reads the manufacturer from the ELD: EDID bytes 8–9 as a little-endian number.
        let raw_mfg = edid.manufacturer_id.swap_bytes();
        let pnp = pnp_id(edid.manufacturer_id);
        let key = sink_key(
            pnp.as_deref(),
            raw_mfg,
            edid.product_code,
            serial(&raw).as_deref(),
        );
        assert_eq!(
            Some(key.as_str()),
            expect["sink_key"].as_str(),
            "{name}: sink key"
        );
        assert_eq!(
            pnp.as_deref(),
            expect["manufacturer_id"].as_str(),
            "{name}: manufacturer"
        );
        assert_eq!(
            u64::from(edid.product_code),
            expect["product_code"].as_u64().unwrap(),
            "{name}: product"
        );
        // AudioCaps falls back to the ELD's monitor name when the EDID has none; DisplayCaps
        // reads the EDID only, so compare names only where the EDID carries one.
        if let Some(n) = &edid.name {
            assert_eq!(
                Some(n.as_str()),
                expect["model_name"].as_str(),
                "{name}: model"
            );
        }
    }
}
