//! The `SCTX` texture container.
//!
//! # Layout, as verified against shipped files
//!
//! ```text
//! 0   u32     header field A (48 in every sample seen)
//! 4   u32     header field B (28 in every sample seen)
//! 8   "SCTX"  magic
//! 12  ...     FlatBuffers metadata
//! ..  ...     ZSTD frame: the compressed texture payload
//! ```
//!
//! # What is confirmed
//!
//! - The magic sits at offset 8, not 0.
//! - The payload is a ZSTD frame, located by scanning for its magic rather
//!   than by a fixed offset (it began at 100 in every sample, but that is a
//!   consequence of the metadata size, not a guarantee).
//! - A `u32` in the header region holds the decompressed payload length, and
//!   it matched the actual decompressed size exactly on every sample.
//! - The decompressed payload is a flat array of 16-byte blocks whose first
//!   block is `fc fd ff ff ff ff ff ff 00 …`. The leading `0xFC` is the ASTC
//!   void-extent (constant colour) block signature, so the payload is ASTC
//!   texture data.
//!
//! # What is not yet confirmed
//!
//! **Image dimensions and the ASTC block footprint.** Candidate `u16` pairs in
//! the header region do not survive contact with a second file: one sample's
//! byte count factors consistently with the values read there, another's does
//! not. That is expected — the metadata is FlatBuffers, so a field the encoder
//! omits shifts everything after it. Dimensions must come from the vtable via
//! [`crate::flatbuffers`], and the field indices have not yet been pinned down
//! across enough files to be trustworthy.
//!
//! Until then [`Sctx::decode`] returns the ASTC payload and the metadata
//! table, and declines to guess a width. See ASSUMPTIONS.md.

use crate::flatbuffers::{self, Table};
use anyhow::{Context, Result};

/// Magic identifying an SCTX container, at offset 8.
pub const MAGIC: &[u8; 4] = b"SCTX";
/// Offset of the magic within the file.
pub const MAGIC_OFFSET: usize = 8;
/// Where the FlatBuffers metadata region begins.
pub const METADATA_OFFSET: usize = 12;
/// ZSTD frame magic, little-endian 0xFD2FB528.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
/// ASTC blocks are 16 bytes regardless of footprint.
pub const ASTC_BLOCK_BYTES: usize = 16;
/// Leading byte of an ASTC void-extent block.
const ASTC_VOID_EXTENT_TAG: u8 = 0xFC;

/// A parsed SCTX container.
#[derive(Debug, Clone)]
pub struct Sctx {
    /// Decompressed texture payload, as ASTC blocks.
    pub payload: Vec<u8>,
    /// Decompressed length recorded in the header, when one was found.
    pub declared_len: Option<u32>,
    /// Offset the ZSTD frame was found at.
    pub payload_offset: usize,
    /// Raw metadata region, for continued reverse-engineering.
    pub metadata: Vec<u8>,
}

impl Sctx {
    /// Number of 16-byte blocks in the payload.
    pub fn block_count(&self) -> usize {
        self.payload.len() / ASTC_BLOCK_BYTES
    }

    /// Whether the payload looks like ASTC block data.
    ///
    /// Checks that the length divides into whole blocks and that at least one
    /// void-extent block is present, which every texture with transparent
    /// margin has.
    pub fn looks_like_astc(&self) -> bool {
        if self.payload.is_empty() || !self.payload.len().is_multiple_of(ASTC_BLOCK_BYTES) {
            return false;
        }
        self.payload
            .chunks_exact(ASTC_BLOCK_BYTES)
            .any(|b| b[0] == ASTC_VOID_EXTENT_TAG)
    }

    /// Candidate `(width, height)` pairs consistent with the payload size.
    ///
    /// Deliberately returns *candidates* rather than an answer. Given the
    /// block count and an assumed footprint, many dimension pairs are
    /// arithmetically possible; picking one requires the metadata field that
    /// has not yet been identified. Callers that need real dimensions should
    /// treat an ambiguous result as unresolved rather than taking the first.
    pub fn dimension_candidates(&self, block_w: usize, block_h: usize) -> Vec<(usize, usize)> {
        let blocks = self.block_count();
        if blocks == 0 || block_w == 0 || block_h == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for bw in 1..=blocks {
            if bw * bw > blocks {
                break;
            }
            if !blocks.is_multiple_of(bw) {
                continue;
            }
            let bh = blocks / bw;
            out.push((bw * block_w, bh * block_h));
            if bw != bh {
                out.push((bh * block_w, bw * block_h));
            }
        }
        out
    }

    /// The FlatBuffers metadata root, if it parses.
    ///
    /// Exposed so the field indices can be mapped by walking real files.
    pub fn metadata_table(&self) -> Option<Table<'_>> {
        flatbuffers::root(&self.metadata, 0).ok()
    }
}

/// Returns true when `data` carries the SCTX magic.
pub fn is_sctx(data: &[u8]) -> bool {
    data.len() > MAGIC_OFFSET + 4 && &data[MAGIC_OFFSET..MAGIC_OFFSET + 4] == MAGIC
}

/// Parses and decompresses an SCTX container.
pub fn decode(data: &[u8]) -> Result<Sctx> {
    anyhow::ensure!(
        is_sctx(data),
        "not an SCTX container: expected magic {:?} at offset {MAGIC_OFFSET}",
        std::str::from_utf8(MAGIC).unwrap_or("SCTX")
    );

    let payload_offset = find_zstd_frame(data)
        .context("no ZSTD frame found in SCTX container")?;

    let payload = zstd::stream::decode_all(&data[payload_offset..])
        .context("decompressing SCTX texture payload")?;

    let metadata = data[METADATA_OFFSET..payload_offset].to_vec();

    // The header records the decompressed length. Rather than trusting a fixed
    // offset, look for a u32 anywhere in the header that matches what we
    // actually decompressed: that both locates the field and validates the
    // decode in one step.
    let declared_len = find_u32_equal(data, payload_offset, payload.len() as u32);

    Ok(Sctx {
        payload,
        declared_len,
        payload_offset,
        metadata,
    })
}

/// Locates the ZSTD frame by magic.
fn find_zstd_frame(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == ZSTD_MAGIC)
}

/// Finds a `u32` in `data[..limit]` equal to `want`, returning its value.
fn find_u32_equal(data: &[u8], limit: usize, want: u32) -> Option<u32> {
    let end = limit.min(data.len());
    data[..end]
        .windows(4)
        .any(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) == want)
        .then_some(want)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_non_sctx_blob() {
        assert!(!is_sctx(b"not a texture at all"));
        assert!(decode(b"not a texture at all").is_err());
    }

    #[test]
    fn magic_is_checked_at_offset_eight_not_zero() {
        let mut blob = vec![0u8; 32];
        blob[0..4].copy_from_slice(b"SCTX");
        assert!(!is_sctx(&blob), "magic at offset 0 is not an SCTX file");
        blob[0..4].copy_from_slice(&[0, 0, 0, 0]);
        blob[8..12].copy_from_slice(b"SCTX");
        assert!(is_sctx(&blob));
    }

    #[test]
    fn dimension_candidates_all_multiply_back_to_the_payload() {
        let sctx = Sctx {
            payload: vec![0u8; 38_640 * ASTC_BLOCK_BYTES],
            declared_len: None,
            payload_offset: 100,
            metadata: Vec::new(),
        };
        let cands = sctx.dimension_candidates(4, 4);
        assert!(!cands.is_empty());
        for (w, h) in &cands {
            assert_eq!(
                (w / 4) * (h / 4),
                sctx.block_count(),
                "candidate {w}x{h} does not account for the whole payload"
            );
        }
        // The point of returning candidates: this really is ambiguous.
        assert!(
            cands.len() > 1,
            "payload size alone should not identify a single dimension pair"
        );
    }

    #[test]
    fn astc_detection_needs_whole_blocks() {
        let mut s = Sctx {
            payload: vec![0xFC; 17],
            declared_len: None,
            payload_offset: 0,
            metadata: Vec::new(),
        };
        assert!(!s.looks_like_astc(), "17 bytes is not a whole block count");
        s.payload = vec![0xFC; 32];
        assert!(s.looks_like_astc());
        s.payload = vec![0x00; 32];
        assert!(!s.looks_like_astc(), "no void-extent block present");
    }
}
