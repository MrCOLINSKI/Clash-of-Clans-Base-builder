//! Decoders for Clash of Clans art containers.
//!
//! Separate from `coc-data` on purpose: balance data is required for the
//! simulator to be correct at all, whereas art is required only for the
//! renderer to look right. Nothing in the simulator depends on this crate, so
//! a container format changing under us degrades presentation rather than
//! invalidating results.
//!
//! # Formats
//!
//! | Extension | Count in 18.400.11 | Status |
//! |---|---|---|
//! | `.glb` | 3079 | Container parsed; `FLA2` descriptor is FlatBuffers, schema unmapped |
//! | `.sctx` | 1833 | Container parsed, ZSTD payload decoded, ASTC confirmed; dimensions unmapped |
//! | `.sc` | 728 | Header parsed; record stream unmapped |
//!
//! See `docs/ART_FORMATS.md` for the reverse-engineering notes behind each.

pub mod flatbuffers;
pub mod glb;
pub mod sc;
pub mod sctx;

/// Which art container a blob holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// glTF binary, possibly with Supercell's `FLA2` descriptor.
    Glb,
    /// `SCTX` texture container.
    Sctx,
    /// Legacy `SC` sprite container.
    Sc,
    Unknown,
}

/// Identifies a container by magic bytes.
///
/// Order matters: `SCTX` files carry their magic at offset 8 and could begin
/// with anything, so they are tested before the offset-0 formats.
pub fn identify(data: &[u8]) -> Kind {
    if sctx::is_sctx(data) {
        Kind::Sctx
    } else if glb::is_glb(data) {
        Kind::Glb
    } else if sc::is_sc(data) {
        Kind::Sc
    } else {
        Kind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_by_magic_not_extension() {
        let mut sctx = vec![0u8; 16];
        sctx[8..12].copy_from_slice(b"SCTX");
        assert_eq!(identify(&sctx), Kind::Sctx);

        let mut glb = b"glTF".to_vec();
        glb.extend_from_slice(&[0; 16]);
        assert_eq!(identify(&glb), Kind::Glb);

        let mut sc = b"SC".to_vec();
        sc.extend_from_slice(&6u32.to_le_bytes());
        assert_eq!(identify(&sc), Kind::Sc);

        assert_eq!(identify(b"something else entirely"), Kind::Unknown);
    }
}
