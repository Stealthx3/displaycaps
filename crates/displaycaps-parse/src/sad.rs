// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-parse/src/sad.rs; raw SAD fields kept for the combined library; the mapping to
// AudioCaps' codec model removed.
//! CTA-861 Short Audio Descriptors (SADs), shared by the ELD and EDID parsers.
//!
//! A SAD is 3 bytes. The raw bytes are kept, so flags that the kernel's ELD text drops (E-AC-3 JOC,
//! MAT Atmos, extension type codes) survive.

/// Sample rates in SAD byte 2, bit 0 first.
pub const SAD_RATES_HZ: [u32; 7] = [32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

/// One Short Audio Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sad {
    /// The 3 raw bytes.
    pub raw: [u8; 3],
}

impl Sad {
    /// Wraps 3 raw bytes.
    pub fn from_bytes(b: [u8; 3]) -> Self {
        Self { raw: b }
    }

    /// Audio Format Code (1 = LPCM … 15 = extension).
    pub fn format_code(&self) -> u8 {
        (self.raw[0] >> 3) & 0x0F
    }

    /// Maximum channel count.
    pub fn channels(&self) -> u8 {
        (self.raw[0] & 0x07) + 1
    }

    /// Rate bitmask (byte 2, bits 0–6).
    pub fn rate_mask(&self) -> u8 {
        self.raw[1] & 0x7F
    }

    /// Supported sample rates in Hz, ascending.
    pub fn rates_hz(&self) -> Vec<u32> {
        (0..7)
            .filter(|i| self.raw[1] & (1 << i) != 0)
            .map(|i| SAD_RATES_HZ[i])
            .collect()
    }

    /// LPCM bit depths (format code 1 only).
    pub fn lpcm_bit_depths(&self) -> Vec<u8> {
        if self.format_code() != 1 {
            return Vec::new();
        }
        [16u8, 20, 24]
            .into_iter()
            .enumerate()
            .filter(|(i, _)| self.raw[2] & (1 << i) != 0)
            .map(|(_, d)| d)
            .collect()
    }

    /// Maximum bit rate in kbit/s (format codes 2–8: byte 3 × 8 kbit/s).
    pub fn max_bitrate_kbps(&self) -> Option<u32> {
        match self.format_code() {
            2..=8 if self.raw[2] != 0 => Some(u32::from(self.raw[2]) * 8),
            _ => None,
        }
    }

    /// Extension type code (format code 15 only).
    pub fn extension_code(&self) -> Option<u8> {
        (self.format_code() == 15).then_some(self.raw[2] >> 3)
    }
}

/// Splits a byte run into SADs; a trailing partial SAD is ignored.
pub fn parse_sads(bytes: &[u8]) -> Vec<Sad> {
    let (chunks, _) = bytes.as_chunks::<3>();
    chunks.iter().map(|c| Sad::from_bytes(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lpcm_8ch() {
        let s = Sad::from_bytes([0x0F, 0x7F, 0x07]);
        assert_eq!(s.format_code(), 1);
        assert_eq!(s.channels(), 8);
        assert_eq!(s.rates_hz(), SAD_RATES_HZ.to_vec());
        assert_eq!(s.lpcm_bit_depths(), vec![16, 20, 24]);
        assert_eq!(s.max_bitrate_kbps(), None);
    }

    #[test]
    fn ac3_bitrate_and_eac3() {
        let ac3 = Sad::from_bytes([0x15, 0x07, 0x50]);
        assert_eq!(ac3.format_code(), 2);
        assert_eq!(ac3.channels(), 6);
        assert_eq!(ac3.max_bitrate_kbps(), Some(640));
        let eac3 = Sad::from_bytes([0x57, 0x06, 0x01]);
        assert_eq!(eac3.format_code(), 10);
    }

    #[test]
    fn mat_and_extension() {
        let mat = Sad::from_bytes([0x67, 0x54, 0x01]);
        assert_eq!(mat.format_code(), 12);
        let ac4 = Sad::from_bytes([0x7F, 0x06, 12 << 3]);
        assert_eq!(ac4.extension_code(), Some(12));
    }

    #[test]
    fn partial_tail_ignored() {
        assert_eq!(parse_sads(&[1, 2, 3, 4, 5]).len(), 1);
    }
}
