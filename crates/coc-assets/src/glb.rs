//! Supercell's glTF-binary variant.
//!
//! The 3D models under `sc3d/` use a standard glTF 2.0 binary container — the
//! `glTF` magic, the version and length fields, and the chunk framing are all
//! to spec, and the `BIN` chunk holds ordinary vertex and index data.
//!
//! The descriptor chunk is not. Where the specification requires a chunk typed
//! `JSON`, these files carry one typed **`FLA2`** containing FlatBuffers. Any
//! off-the-shelf glTF loader therefore rejects them, which is the single
//! reason importing this art is a reverse-engineering job rather than a
//! library call.
//!
//! Strings recovered from the `FLA2` chunk of a character model include
//! `SC_odin_format`, `bounds`, `parent`, and skeleton joint names such as
//! `L_index_03_s` — so the chunk describes a scene graph and skinning data in
//! the same role glTF's JSON would play.

use crate::flatbuffers::{self, Table};
use anyhow::{Context, Result};

/// glTF binary magic.
pub const MAGIC: &[u8; 4] = b"glTF";
/// Supercell's descriptor chunk type, replacing the spec's `JSON`.
pub const CHUNK_FLA2: [u8; 4] = *b"FLA2";
/// The spec's descriptor chunk type, accepted when present.
pub const CHUNK_JSON: [u8; 4] = *b"JSON";
/// Binary buffer chunk type; the trailing byte is a space in the spec.
pub const CHUNK_BIN: [u8; 4] = *b"BIN\0";

/// One chunk of a glTF binary file.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub kind: [u8; 4],
    /// Offset of the chunk's data within the file.
    pub offset: usize,
    pub data: Vec<u8>,
}

impl Chunk {
    pub fn kind_str(&self) -> String {
        String::from_utf8_lossy(&self.kind).trim_end_matches('\0').to_string()
    }
}

/// A parsed glTF binary container.
#[derive(Debug, Clone)]
pub struct Glb {
    pub version: u32,
    /// Total length declared in the header.
    pub declared_len: u32,
    pub chunks: Vec<Chunk>,
}

impl Glb {
    /// The descriptor chunk, whether it is `FLA2` or a spec-compliant `JSON`.
    pub fn descriptor(&self) -> Option<&Chunk> {
        self.chunks
            .iter()
            .find(|c| c.kind == CHUNK_FLA2 || c.kind == CHUNK_JSON)
    }

    /// The binary buffer chunk, which is standard glTF regardless of the
    /// descriptor's encoding.
    pub fn binary(&self) -> Option<&Chunk> {
        self.chunks.iter().find(|c| c.kind == CHUNK_BIN)
    }

    /// Whether this file uses Supercell's FlatBuffers descriptor rather than
    /// the spec's JSON, and so cannot be read by a stock glTF loader.
    pub fn is_supercell_variant(&self) -> bool {
        self.chunks.iter().any(|c| c.kind == CHUNK_FLA2)
    }

    /// The descriptor's FlatBuffers root, when it is an `FLA2` chunk.
    pub fn descriptor_table(&self) -> Option<Table<'_>> {
        let c = self.chunks.iter().find(|c| c.kind == CHUNK_FLA2)?;
        flatbuffers::root(&c.data, 0).ok()
    }

    /// Printable ASCII runs in the descriptor, for reverse-engineering.
    ///
    /// The joint and property names are stored as plain strings, so this is
    /// the quickest way to see what a model actually declares.
    pub fn descriptor_strings(&self, min_len: usize) -> Vec<String> {
        let Some(c) = self.descriptor() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut cur = Vec::new();
        for &b in &c.data {
            if (0x20..0x7f).contains(&b) {
                cur.push(b);
            } else {
                if cur.len() >= min_len {
                    out.push(String::from_utf8_lossy(&cur).into_owned());
                }
                cur.clear();
            }
        }
        if cur.len() >= min_len {
            out.push(String::from_utf8_lossy(&cur).into_owned());
        }
        out
    }
}

/// Returns true when `data` carries the glTF binary magic.
pub fn is_glb(data: &[u8]) -> bool {
    data.len() >= 12 && &data[..4] == MAGIC
}

/// Parses the chunk structure of a glTF binary file.
pub fn decode(data: &[u8]) -> Result<Glb> {
    anyhow::ensure!(is_glb(data), "not a glTF binary container");

    let version = read_u32(data, 4).context("reading glTF version")?;
    let declared_len = read_u32(data, 8).context("reading glTF length")?;

    let mut chunks = Vec::new();
    let mut off = 12usize;
    while off + 8 <= data.len() {
        let len = read_u32(data, off)? as usize;
        let kind_raw = read_u32(data, off + 4)?.to_le_bytes();
        let start = off + 8;
        let end = start.checked_add(len).filter(|e| *e <= data.len()).with_context(|| {
            format!("chunk at {off} declares {len} bytes, past the end of a {}-byte file", data.len())
        })?;
        chunks.push(Chunk {
            kind: kind_raw,
            offset: start,
            data: data[start..end].to_vec(),
        });
        // Chunks are 4-byte aligned.
        off = end.next_multiple_of(4);
        if len == 0 {
            break;
        }
    }

    anyhow::ensure!(!chunks.is_empty(), "glTF container has no chunks");
    Ok(Glb {
        version,
        declared_len,
        chunks,
    })
}

fn read_u32(data: &[u8], at: usize) -> Result<u32> {
    let bytes = data
        .get(at..at + 4)
        .with_context(|| format!("need 4 bytes at {at}, file is {}", data.len()))?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("4 bytes")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // length patched below
        for (kind, data) in chunks {
            b.extend_from_slice(&(data.len() as u32).to_le_bytes());
            b.extend_from_slice(kind);
            b.extend_from_slice(data);
            while !b.len().is_multiple_of(4) {
                b.push(0);
            }
        }
        let n = b.len() as u32;
        b[8..12].copy_from_slice(&n.to_le_bytes());
        b
    }

    #[test]
    fn parses_chunk_framing() {
        let blob = build(&[
            (CHUNK_FLA2, vec![1, 2, 3, 4]),
            (CHUNK_BIN, vec![9; 16]),
        ]);
        let g = decode(&blob).expect("parses");
        assert_eq!(g.version, 2);
        assert_eq!(g.chunks.len(), 2);
        assert_eq!(g.binary().unwrap().data.len(), 16);
    }

    #[test]
    fn identifies_the_supercell_descriptor_variant() {
        let sc = decode(&build(&[(CHUNK_FLA2, vec![0; 8])])).unwrap();
        assert!(sc.is_supercell_variant());
        assert_eq!(sc.descriptor().unwrap().kind_str(), "FLA2");

        let spec = decode(&build(&[(CHUNK_JSON, b"{}".to_vec())])).unwrap();
        assert!(
            !spec.is_supercell_variant(),
            "a spec-compliant JSON chunk is not the Supercell variant"
        );
        assert_eq!(spec.descriptor().unwrap().kind_str(), "JSON");
    }

    #[test]
    fn rejects_a_chunk_running_past_the_end() {
        let mut blob = build(&[(CHUNK_BIN, vec![0; 8])]);
        // Overstate the chunk length.
        blob[12..16].copy_from_slice(&9999u32.to_le_bytes());
        assert!(decode(&blob).is_err());
    }

    #[test]
    fn rejects_a_non_glb_blob() {
        assert!(!is_glb(b"nope"));
        assert!(decode(b"nope not a model file").is_err());
    }

    #[test]
    fn extracts_strings_from_the_descriptor() {
        let mut payload = vec![0u8; 4];
        payload.extend_from_slice(b"SC_odin_format");
        payload.push(0);
        payload.extend_from_slice(b"L_index_03_s");
        let g = decode(&build(&[(CHUNK_FLA2, payload)])).unwrap();
        let strings = g.descriptor_strings(5);
        assert!(strings.iter().any(|s| s.contains("SC_odin_format")));
        assert!(strings.iter().any(|s| s.contains("L_index_03_s")));
    }
}
