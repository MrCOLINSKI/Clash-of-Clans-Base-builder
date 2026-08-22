//! The legacy `SC` sprite container.
//!
//! Pre-3D art, and still used for UI and 2D effects, ships in Supercell's own
//! `SC` format: a header followed by shape, movie-clip, and texture-reference
//! records. Files pair up as `name.sc` (geometry and animation) with
//! `name_<n>.sctx` (the texture atlas they sample).
//!
//! Only the container header is parsed here. The record stream is a separate
//! reverse-engineering effort tracked in ASSUMPTIONS.md; what this gives us
//! today is reliable identification and version reporting, so an extraction
//! run can report exactly what it could not yet handle instead of failing
//! opaquely.

use anyhow::Result;

/// Magic identifying an SC container, at offset 0.
pub const MAGIC: &[u8; 2] = b"SC";

/// A recognised SC container header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScHeader {
    /// Format version. Shipped art in 18.400.11 is version 6.
    pub version: u32,
    /// Total file length in bytes.
    pub len: usize,
}

/// Returns true when `data` carries the SC magic.
pub fn is_sc(data: &[u8]) -> bool {
    data.len() >= 6 && &data[..2] == MAGIC
}

/// Reads the container header.
pub fn header(data: &[u8]) -> Result<ScHeader> {
    anyhow::ensure!(is_sc(data), "not an SC container");
    // Little-endian, like every other integer in these containers. The
    // shipped bytes are `53 43 06 00 00 00`, i.e. "SC" then 6.
    let version = u32::from_le_bytes([data[2], data[3], data[4], data[5]]);
    Ok(ScHeader {
        version,
        len: data.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_field() {
        // Shipped files begin "SC" followed by a little-endian version.
        let mut blob = b"SC".to_vec();
        blob.extend_from_slice(&6u32.to_le_bytes());
        blob.extend_from_slice(&[0; 32]);
        assert_eq!(header(&blob).unwrap().version, 6);
    }

    #[test]
    fn rejects_a_non_sc_blob() {
        assert!(!is_sc(b"glTF"));
        assert!(header(b"glTF____").is_err());
    }
}
