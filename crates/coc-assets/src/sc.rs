//! The `SC` sprite container.
//!
//! Pre-3D art — which in this version means every building — ships in
//! Supercell's own `SC` format. Files pair as `name.sc` (structure) with
//! `name_<n>.sctx` (the texture atlases it samples).
//!
//! # Layout, verified against `sc/buildings.sc` at 18.400.11
//!
//! ```text
//! 0    "SC"        magic
//! 2    u32         version (6)
//! 16   u32         hash length (20)
//! 20   ..          hash
//! 12   u32         FlatBuffers root offset, relative to offset 12
//! ..   ..          FlatBuffers metadata: export names
//! ..   ZSTD frame  compressed object graph
//! ```
//!
//! The metadata region is FlatBuffers with the buffer base at **offset 12**,
//! and holds the export table: 3143 entries in `buildings.sc`, each a name
//! plus a constant type tag. The export id is the entry's index in that
//! vector, not a stored field.
//!
//! Everything else lives in a ZSTD frame that follows, located by magic. In
//! `buildings.sc` it decompresses from 4.0 MB to 29.7 MB and is itself a
//! FlatBuffers buffer, base at offset 4:
//!
//! | Field | Contents |
//! |---|---|
//! | 0 | vector of 3467 object names |
//! | 4 | offset into the point pool |
//! | 5 | point pool: `(x, y, t)` float triples, 1.93M entries |
//! | 6 | five banks, each a matrix vector and a colour-transform vector |
//!
//! The banks are chunked at the u16 boundary — 65 500-odd entries each —
//! because the indices referring to them are 16-bit. Matrices are six floats
//! (`a b c d tx ty`); colour transforms are RGBA multiply and add bytes.
//! Across the five banks there are 18 827 shape records, which lines up with
//! the number of sprites actually packed into the atlases (between 17 548 and
//! 20 591 opaque regions, depending on the size threshold).
//!
//! Texture entries sit near the tail: each carries `u16` width and height
//! followed by the atlas filename, e.g. 608 x 1004 and `buildings_0.sctx`,
//! matching that file's own SCTX header exactly.
//!
//! # What is NOT yet solved
//!
//! **Which sprite belongs to which building.** The chain
//! `export name -> object -> shape -> texture rectangle` is mapped as far as
//! the object name vector and the texture list, but the hop from a shape
//! record to its atlas rectangle is not. Until it is, a renderer cannot put
//! the right portrait on the right tower, and assigning sprites by any other
//! rule produces art that is real but wrong — a max-level X-Bow standing where
//! a level-2 Mortar belongs. See ASSUMPTIONS.md 6.7.

use crate::flatbuffers::{self, Table};
use anyhow::{Context, Result};

/// Magic identifying an SC container.
pub const MAGIC: &[u8; 2] = b"SC";
/// FlatBuffers base for the metadata region.
pub const META_FB_BASE: usize = 12;
/// ZSTD frame magic.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Container header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScHeader {
    /// Format version. Shipped art in 18.400.11 is version 6.
    pub version: u32,
    /// Length of the content hash that follows.
    pub hash_len: u32,
    pub len: usize,
}

/// One atlas referenced by an SC file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureRef {
    pub width: u32,
    pub height: u32,
    pub file: String,
}

/// A parsed SC container.
#[derive(Debug, Clone)]
pub struct Sc {
    pub header: ScHeader,
    /// Export names from the metadata region. Index is the export id.
    pub exports: Vec<String>,
    /// Object names from the decompressed graph.
    pub objects: Vec<String>,
    /// Atlases this file samples, in file order.
    pub textures: Vec<TextureRef>,
    /// Decompressed object graph, retained for further decoding.
    pub blob: Vec<u8>,
}

impl Sc {
    /// Export id for a name, which is its index in the export vector.
    pub fn export_id(&self, name: &str) -> Option<usize> {
        self.exports.iter().position(|e| e == name)
    }

    /// Index of a name in the object graph.
    pub fn object_id(&self, name: &str) -> Option<usize> {
        self.objects.iter().position(|o| o == name)
    }
}

pub fn is_sc(data: &[u8]) -> bool {
    data.len() >= 6 && &data[..2] == MAGIC
}

/// Reads the container header.
pub fn header(data: &[u8]) -> Result<ScHeader> {
    anyhow::ensure!(is_sc(data), "not an SC container");
    Ok(ScHeader {
        // Little-endian, like every other integer in these containers.
        version: u32::from_le_bytes(data[2..6].try_into()?),
        hash_len: data.get(16..20).map(|b| {
            u32::from_le_bytes(b.try_into().expect("4 bytes"))
        }).unwrap_or(0),
        len: data.len(),
    })
}

/// Parses an SC container: header, export table, and object graph.
pub fn decode(data: &[u8]) -> Result<Sc> {
    let header = header(data)?;

    let exports = read_names(data, META_FB_BASE, EXPORT_FIELD).unwrap_or_default();

    // The four magic bytes occur by chance inside the metadata, so candidates
    // are verified by actually decoding rather than trusting the first match.
    let (blob, _at) = find_and_decode_zstd(data)
        .context("no decodable ZSTD frame in SC container")?;

    let objects = read_names(&blob, BLOB_FB_BASE, OBJECT_FIELD).unwrap_or_default();
    let textures = read_textures(&blob);

    Ok(Sc {
        header,
        exports,
        objects,
        textures,
        blob,
    })
}

/// Scans for a ZSTD frame that genuinely decodes, returning it and its offset.
///
/// Two details matter here. The four magic bytes occur by chance inside the
/// metadata, so a candidate is only accepted once it actually decodes. And the
/// frame is followed by trailing bytes, so a whole-input decode reports
/// "unknown frame descriptor" *after* having produced the entire payload —
/// hence `single_frame`, which stops cleanly at the frame boundary.
fn find_and_decode_zstd(data: &[u8]) -> Option<(Vec<u8>, usize)> {
    use std::io::Read;
    let mut from = 0usize;
    while from + 4 <= data.len() {
        let rel = data[from..].windows(4).position(|w| w == ZSTD_MAGIC)?;
        let at = from + rel;
        if let Ok(mut dec) = zstd::stream::Decoder::new(&data[at..]) {
            // The object graph is tens of megabytes, well past the default
            // window limit.
            if dec.window_log_max(31).is_ok() {
                let mut out = Vec::new();
                if dec.single_frame().read_to_end(&mut out).is_ok() && out.len() > 1024 {
                    return Some((out, at));
                }
            }
        }
        from = at + 1;
    }
    None
}

/// Field holding the export vector in the metadata region.
const EXPORT_FIELD: usize = 10;
/// Field holding the object-name vector in the decompressed graph.
const OBJECT_FIELD: usize = 0;
/// FlatBuffers base within the decompressed graph.
const BLOB_FB_BASE: usize = 4;

/// Reads a vector of names, whether stored as tables wrapping a string or as
/// bare strings — the metadata and the graph differ.
fn read_names(buf: &[u8], base: usize, field: usize) -> Option<Vec<String>> {
    let root = flatbuffers::root(buf, base).ok()?;
    let (start, count) = root.vector_at(field).ok()??;
    let mut out = Vec::with_capacity(count.min(100_000));
    for k in 0..count.min(100_000) {
        let eo = start + k * 4;
        let rel = read_u32(buf, eo)? as usize;
        let p = eo.checked_add(rel)?;
        // A bare string: length prefix then bytes.
        if let Some(s) = read_string(buf, p) {
            out.push(s);
            continue;
        }
        // A table wrapping the string in its first field.
        if let Ok(t) = Table::at(buf, p) {
            out.push(t.str_at(0).ok().flatten().unwrap_or_default().to_string());
        } else {
            out.push(String::new());
        }
    }
    Some(out)
}

/// Finds atlas references by scanning for `.sctx` names and reading the
/// `u16` dimensions that precede each one.
fn read_textures(blob: &[u8]) -> Vec<TextureRef> {
    let mut out = Vec::new();
    let needle = b".sctx";
    let mut i = 0usize;
    while let Some(rel) = blob[i..].windows(needle.len()).position(|w| w == needle) {
        let end = i + rel + needle.len();
        // Walk back over the name to its length prefix.
        let mut start = end;
        while start > 0 && blob[start - 1] != 0 && end - start < 64 {
            start -= 1;
        }
        if start >= 4 {
            let len = read_u32(blob, start - 4).unwrap_or(0) as usize;
            if len == end - start && start >= 12 {
                let w = u16::from_le_bytes([blob[start - 12], blob[start - 11]]) as u32;
                let h = u16::from_le_bytes([blob[start - 10], blob[start - 9]]) as u32;
                if let Ok(file) = std::str::from_utf8(&blob[start..end]) {
                    out.push(TextureRef {
                        width: w,
                        height: h,
                        file: file.to_string(),
                    });
                }
            }
        }
        i = end;
    }
    out
}

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes(s.try_into().expect("4 bytes")))
}

fn read_string(b: &[u8], at: usize) -> Option<String> {
    let len = read_u32(b, at)? as usize;
    if len == 0 || len > 200 {
        return None;
    }
    let s = b.get(at + 4..at + 4 + len)?;
    let text = std::str::from_utf8(s).ok()?;
    text.chars()
        .all(|c| c.is_ascii_graphic() || c == '_')
        .then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_field_little_endian() {
        let mut blob = b"SC".to_vec();
        blob.extend_from_slice(&6u32.to_le_bytes());
        blob.extend_from_slice(&[0; 40]);
        assert_eq!(header(&blob).unwrap().version, 6);
    }

    #[test]
    fn rejects_a_non_sc_blob() {
        assert!(!is_sc(b"glTF"));
        assert!(header(b"gl").is_err());
        assert!(decode(b"not an sc file at all").is_err());
    }
}
