//! Supercell asset decompression.
//!
//! Assets on `game-assets.clashofclans.com` are served in one of three
//! compression formats, optionally behind a signature header. Format is
//! detected by magic bytes, never by file extension.
//!
//! Observed on game version 18.400.11: every `logic/*.csv` file is
//! `Sig:` + LZMA. The LZHAM and ZSTD paths are implemented against the
//! documented framing but are not exercised by the current asset set.

use thiserror::Error;

/// Length of the `Sig:` signature header that prefixes signed assets.
///
/// Layout is a 4-byte `Sig:` magic followed by a 64-byte signature.
const SIG_HEADER_LEN: usize = 68;

/// LZMA1 `alone` properties byte upper bound: `(4 * 5 + 4) * 9 + 8`.
const LZMA_MAX_PROP: u8 = 224;

#[derive(Debug, Error)]
pub enum DecompressError {
    #[error("input is too short to identify ({0} bytes)")]
    TooShort(usize),
    #[error("LZMA properties byte {0} is out of range (max {LZMA_MAX_PROP})")]
    BadLzmaProps(u8),
    #[error("LZMA decode failed: {0}")]
    Lzma(String),
    #[error("ZSTD decode failed: {0}")]
    Zstd(String),
    #[error(
        "asset uses LZHAM (SCLZ) compression, which this build cannot decode. \
         No pure-Rust LZHAM decoder is available; see ASSUMPTIONS.md. \
         dict_size_log2={dict_size_log2}, uncompressed_size={uncompressed_size}"
    )]
    Lzham {
        dict_size_log2: u8,
        uncompressed_size: u32,
    },
}

/// Which compression format an asset blob uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Raw LZMA1 with Supercell's truncated 4-byte size field.
    Lzma,
    /// LZHAM, framed as `SCLZ` + dict size + uncompressed size.
    Lzham,
    /// Standard ZSTD frame.
    Zstd,
    /// Already-decoded CSV text.
    Plain,
}

/// Result of decompressing an asset, retaining how it was encoded.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub bytes: Vec<u8>,
    pub format: Format,
    /// Whether a `Sig:` header was stripped before decoding.
    pub was_signed: bool,
}

/// Strips the `Sig:` header if present, returning the payload.
fn strip_signature(data: &[u8]) -> (&[u8], bool) {
    if data.len() > SIG_HEADER_LEN && &data[..4] == b"Sig:" {
        (&data[SIG_HEADER_LEN..], true)
    } else {
        (data, false)
    }
}

/// Identifies the compression format of an (unsigned) payload.
pub fn detect(payload: &[u8]) -> Result<Format, DecompressError> {
    if payload.len() < 4 {
        return Err(DecompressError::TooShort(payload.len()));
    }
    if &payload[..4] == b"SCLZ" {
        return Ok(Format::Lzham);
    }
    // ZSTD frame magic, little-endian 0xFD2FB528.
    if payload[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        return Ok(Format::Zstd);
    }
    // Decoded CSVs start with a quoted header cell.
    if payload.starts_with(b"\"Name\"") || payload.starts_with(b"\"name\"") {
        return Ok(Format::Plain);
    }
    Ok(Format::Lzma)
}

/// Decompresses a Supercell asset blob.
///
/// Handles the optional `Sig:` header, then dispatches on magic bytes.
pub fn decompress(data: &[u8]) -> Result<Decoded, DecompressError> {
    let (payload, was_signed) = strip_signature(data);
    let format = detect(payload)?;
    let bytes = match format {
        Format::Plain => payload.to_vec(),
        Format::Zstd => zstd::stream::decode_all(payload)
            .map_err(|e| DecompressError::Zstd(e.to_string()))?,
        Format::Lzham => {
            // Framing is documented even though we cannot decode it, so the
            // error carries enough detail to decode out-of-band if needed.
            let dict_size_log2 = payload[4];
            let uncompressed_size = u32::from_le_bytes([
                payload[5], payload[6], payload[7], payload[8],
            ]);
            return Err(DecompressError::Lzham {
                dict_size_log2,
                uncompressed_size,
            });
        }
        Format::Lzma => decompress_lzma(payload)?,
    };
    Ok(Decoded {
        bytes,
        format,
        was_signed,
    })
}

/// Decodes Supercell's malformed-header LZMA.
///
/// A standard LZMA1 `alone` header is 5 property bytes followed by an 8-byte
/// little-endian uncompressed size. Supercell truncates that size field to 4
/// bytes, so four zero bytes are inserted at offset 9 to restore a well-formed
/// header before handing it to a conventional decoder.
fn decompress_lzma(payload: &[u8]) -> Result<Vec<u8>, DecompressError> {
    if payload.len() < 9 {
        return Err(DecompressError::TooShort(payload.len()));
    }
    let props = payload[0];
    if props > LZMA_MAX_PROP {
        return Err(DecompressError::BadLzmaProps(props));
    }

    let mut fixed = Vec::with_capacity(payload.len() + 4);
    fixed.extend_from_slice(&payload[..9]);
    fixed.extend_from_slice(&[0u8; 4]);
    fixed.extend_from_slice(&payload[9..]);

    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&fixed[..]);
    lzma_rs::lzma_decompress(&mut cursor, &mut out)
        .map_err(|e| DecompressError::Lzma(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_zstd_magic() {
        let blob = [0x28, 0xB5, 0x2F, 0xFD, 0, 0, 0, 0];
        assert_eq!(detect(&blob).unwrap(), Format::Zstd);
    }

    #[test]
    fn detects_lzham_magic() {
        let blob = *b"SCLZ\x12\x00\x00\x00\x00";
        assert_eq!(detect(&blob).unwrap(), Format::Lzham);
    }

    #[test]
    fn detects_plain_csv() {
        assert_eq!(detect(b"\"Name\",\"TID\"").unwrap(), Format::Plain);
    }

    #[test]
    fn falls_back_to_lzma() {
        let blob = [0x5D, 0x00, 0x00, 0x04, 0x00];
        assert_eq!(detect(&blob).unwrap(), Format::Lzma);
    }

    #[test]
    fn strips_signature_header() {
        let mut blob = b"Sig:".to_vec();
        blob.extend_from_slice(&[0u8; 64]);
        blob.extend_from_slice(b"\"Name\",\"TID\"");
        let (payload, signed) = strip_signature(&blob);
        assert!(signed);
        assert_eq!(payload, b"\"Name\",\"TID\"");
    }

    #[test]
    fn rejects_out_of_range_lzma_props() {
        let blob = [0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(matches!(
            decompress_lzma(&blob),
            Err(DecompressError::BadLzmaProps(0xFF))
        ));
    }

    #[test]
    fn lzham_error_reports_framing() {
        let mut blob = b"SCLZ".to_vec();
        blob.push(18); // dict_size_log2
        blob.extend_from_slice(&1234u32.to_le_bytes());
        match decompress(&blob) {
            Err(DecompressError::Lzham {
                dict_size_log2,
                uncompressed_size,
            }) => {
                assert_eq!(dict_size_log2, 18);
                assert_eq!(uncompressed_size, 1234);
            }
            other => panic!("expected Lzham error, got {other:?}"),
        }
    }
}
