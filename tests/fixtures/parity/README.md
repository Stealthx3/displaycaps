# Sink-key parity vectors

Shared EDID test vectors that keep DisplayCaps' sink key identical to AudioCaps' for the same device
(`crates/displaycaps-parse/tests/parity.rs`).

| File | What it is |
| --- | --- |
| `edid/hisense-55qd5n.bin`, `qemu-virtual.bin`, `samsung-lc34g55t.bin`, `lg-fhd.bin` | Redacted EDIDs (serial bytes zeroed, checksum fixed) copied from AudioCaps v1.0.0's committed fixtures |
| `edid/synthetic-*.bin` | The Hisense EDID with made-up serials (numeric, string descriptor, both), an invalid manufacturer id, or no name, to cover every branch of the key. Not real devices |
| `audiocaps-v1.0.0.json` | What AudioCaps v1.0.0 itself reports for each vector, produced in the lab by `scripts/parity-audiocaps.sh` |

Regenerate the vectors with `node scripts/parity-vectors.mjs <audiocaps-v1.0.0 export>`, then the
AudioCaps results with `scripts/lab.sh run 'bash scripts/parity-audiocaps.sh'` and
`scripts/lab-sync-back.sh tests/fixtures`.
