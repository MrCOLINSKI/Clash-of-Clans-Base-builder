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
//! # The metadata is a size-prefixed FlatBuffers buffer
//!
//! The header is not three opaque words. It is the standard size-prefixed
//! FlatBuffers preamble, which is why reading dimensions at a fixed offset
//! appeared to work on one file and failed on the next:
//!
//! ```text
//! [0..4]   u32   size of the FlatBuffers region
//! [4..8]   u32   offset to the root table, relative to offset 4
//! [8..12]  char  file identifier, "SCTX"
//! ```
//!
//! So the buffer base is **offset 4**, not 0 and not 12. From there the root
//! table resolves normally and the fields are stable across every file tested:
//!
//! | Field | Meaning |
//! |---|---|
//! | 2 | image width in pixels |
//! | 3 | image height in pixels |
//! | 6 | pixel format enum |
//! | 7 | decompressed payload length |
//!
//! The block footprint follows from those: for every file tested,
//! `ceil(w/6) * ceil(h/6) * 16` equals the payload length exactly, so the
//! payload is **ASTC 6x6**.
//!
//! | File | Dimensions | Blocks | ceil(w/6) x ceil(h/6) |
//! |---|---|---|---|
//! | `chr_cannon_cart_0` | 1008 x 1376 | 38,640 | 168 x 230 |
//! | `chr_cannon_mortar_cart_0` | 2928 x 3058 | 248,880 | 488 x 510 |
//! | `buildings_0` | 608 x 1004 | 17,136 | 102 x 168 |
//! | `buildings_2` | 1632 x 2042 | 92,752 | 272 x 341 |

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

/// Where the size-prefixed FlatBuffers buffer begins.
///
/// Not 0 and not 12: a size-prefixed buffer puts its root offset at `[4..8]`,
/// so every offset inside the metadata is relative to byte 4.
pub const FB_BASE: usize = 4;

/// Vtable field indices, stable across every shipped file tested.
mod field {
    pub const WIDTH: usize = 2;
    pub const HEIGHT: usize = 3;
    pub const FORMAT: usize = 6;
    pub const PAYLOAD_LEN: usize = 7;
}

/// Metadata read out of the FlatBuffers header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub width: u32,
    pub height: u32,
    /// Pixel format enum. Observed values 5 and 12, both ASTC 6x6.
    pub format: u32,
    /// Decompressed payload length recorded in the header.
    pub payload_len: u32,
}

impl Meta {
    /// ASTC block footprint.
    ///
    /// Every shipped file tested is 6x6, confirmed by the block count matching
    /// the dimensions exactly. Returned as a pair so a future format that is
    /// not 6x6 has somewhere to go rather than silently decoding wrong.
    pub fn footprint(&self) -> (u32, u32) {
        (6, 6)
    }

    /// Blocks required for these dimensions at this footprint.
    pub fn expected_blocks(&self) -> u32 {
        let (bw, bh) = self.footprint();
        self.width.div_ceil(bw) * self.height.div_ceil(bh)
    }

    /// Whether the dimensions account for the payload exactly.
    ///
    /// This is the check that identified the footprint in the first place, so
    /// it doubles as a guard against a format change.
    pub fn is_consistent(&self, payload_bytes: usize) -> bool {
        self.expected_blocks() as usize * ASTC_BLOCK_BYTES == payload_bytes
    }
}

/// A parsed SCTX container.
#[derive(Debug, Clone)]
pub struct Sctx {
    /// Metadata, when the FlatBuffers header parsed.
    pub meta: Option<Meta>,
    /// Decompressed texture payload, as ASTC blocks.
    pub payload: Vec<u8>,
    /// Decompressed length recorded in the header, when one was found.
    pub declared_len: Option<u32>,
    /// Offset the payload was read from.
    pub payload_offset: usize,
    /// Whether the payload was ZSTD-compressed rather than stored raw.
    pub compressed: bool,
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
        if self.payload.is_empty() || self.payload.len() % ASTC_BLOCK_BYTES != 0 {
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
            if blocks % bw != 0 {
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

    /// Decodes the ASTC payload to 8-bit RGBA.
    ///
    /// Requires readable metadata, since ASTC carries no dimensions of its own
    /// — the block stream alone cannot say how wide the image is.
    pub fn to_rgba(&self) -> Result<Vec<u8>> {
        let m = self.meta.context("cannot decode without SCTX metadata")?;
        anyhow::ensure!(
            m.is_consistent(self.payload.len()),
            "dimensions {}x{} imply {} blocks but payload holds {}; \
             the block footprint may have changed",
            m.width,
            m.height,
            m.expected_blocks(),
            self.block_count()
        );

        let (bw, bh) = m.footprint();
        let footprint = astc_decode::Footprint::new(bw, bh);
        let mut out = vec![0u8; (m.width as usize) * (m.height as usize) * 4];
        let w = m.width as usize;
        let h = m.height as usize;

        astc_decode::astc_decode(
            std::io::Cursor::new(&self.payload),
            m.width,
            m.height,
            footprint,
            |x, y, px| {
                let (x, y) = (x as usize, y as usize);
                // The decoder walks whole blocks, so the final row and column
                // of blocks run past the image when dimensions are not a
                // multiple of the footprint.
                if x < w && y < h {
                    let i = (y * w + x) * 4;
                    out[i..i + 4].copy_from_slice(&px);
                }
            },
        )
        .map_err(|e| anyhow::anyhow!("ASTC decode failed: {e}"))?;
        Ok(out)
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

/// Parses an SCTX container, decompressing the payload if it is compressed.
///
/// Two payload modes ship in the same version. Which one a file uses is not
/// stated by a flag we can read yet, so it is detected: a ZSTD frame after the
/// metadata means compressed, and its absence means the ASTC blocks are stored
/// raw at the end of the file.
pub fn decode(data: &[u8]) -> Result<Sctx> {
    anyhow::ensure!(
        is_sctx(data),
        "not an SCTX container: expected magic {:?} at offset {MAGIC_OFFSET}",
        std::str::from_utf8(MAGIC).unwrap_or("SCTX")
    );

    let meta = read_meta(data);

    // The metadata region's own length tells us where the payload may start.
    let fb_end = (read_u32(data, 0).unwrap_or(0) as usize)
        .saturating_add(FB_BASE)
        .min(data.len());

    let (payload, payload_offset, compressed) = match find_zstd_frame(data, fb_end) {
        Some(off) => {
            let out = zstd::stream::decode_all(&data[off..])
                .context("decompressing SCTX texture payload")?;
            (out, off, true)
        }
        None => {
            // Stored raw. The blocks sit at the end of the file, so anchor to
            // the tail rather than guessing where the header stops.
            let want = meta
                .map(|m| m.payload_len as usize)
                .context("SCTX has neither a ZSTD frame nor readable metadata")?;
            anyhow::ensure!(
                want <= data.len(),
                "SCTX declares {want} payload bytes but the file is {}",
                data.len()
            );
            let off = data.len() - want;
            anyhow::ensure!(
                off >= fb_end,
                "SCTX payload would start at {off}, inside the {fb_end}-byte metadata region"
            );
            (data[off..].to_vec(), off, false)
        }
    };

    let metadata = data[METADATA_OFFSET..payload_offset.max(METADATA_OFFSET)].to_vec();

    if let Some(m) = meta {
        anyhow::ensure!(
            m.payload_len as usize == payload.len(),
            "SCTX header declares {} payload bytes but {} were read",
            m.payload_len,
            payload.len()
        );
    }

    let declared_len = meta.map(|m| m.payload_len);

    Ok(Sctx {
        meta,
        payload,
        declared_len,
        payload_offset,
        compressed,
        metadata,
    })
}

/// Locates a ZSTD frame at or after `from`.
fn find_zstd_frame(data: &[u8], from: usize) -> Option<usize> {
    if from >= data.len() {
        return None;
    }
    data[from..]
        .windows(4)
        .position(|w| w == ZSTD_MAGIC)
        .map(|i| i + from)
}

fn read_u32(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
}

/// Reads the FlatBuffers metadata header.
fn read_meta(data: &[u8]) -> Option<Meta> {
    let t = flatbuffers::root(data, FB_BASE).ok()?;
    Some(Meta {
        width: t.u16_at(field::WIDTH).ok()?? as u32,
        height: t.u16_at(field::HEIGHT).ok()?? as u32,
        format: t.u32_at(field::FORMAT).ok()?.unwrap_or(0),
        payload_len: t.u32_at(field::PAYLOAD_LEN).ok()??,
    })
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
            meta: None,
            payload: vec![0u8; 38_640 * ASTC_BLOCK_BYTES],
            declared_len: None,
            payload_offset: 100,
            compressed: true,
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
        // Payload size alone stays ambiguous, which is exactly why the real
        // dimensions are read from the FlatBuffers metadata instead. This
        // helper remains only for inspecting a file whose metadata is absent.
        assert!(
            cands.len() > 1,
            "payload size alone should not identify a single dimension pair"
        );
    }

    #[test]
    fn astc_detection_needs_whole_blocks() {
        let mut s = Sctx {
            meta: None,
            payload: vec![0xFC; 17],
            declared_len: None,
            payload_offset: 0,
            compressed: true,
            metadata: Vec::new(),
        };
        assert!(!s.looks_like_astc(), "17 bytes is not a whole block count");
        s.payload = vec![0xFC; 32];
        assert!(s.looks_like_astc());
        s.payload = vec![0x00; 32];
        assert!(!s.looks_like_astc(), "no void-extent block present");
    }
}
