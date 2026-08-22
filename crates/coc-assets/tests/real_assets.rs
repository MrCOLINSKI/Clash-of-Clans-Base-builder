//! Decoders exercised against real shipped art files.
//!
//! The unit tests cover synthetic containers. These cover the actual bytes
//! Supercell serves, which is where assumptions about a format quietly turn
//! out to be wrong — the SCTX width field being the clearest example.

use coc_assets::{glb, identify, sc, sctx, Kind};
use std::path::PathBuf;

fn sample(name: &str) -> Option<Vec<u8>> {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "..",
        "data",
        "assets",
        "samples",
        name,
    ]
    .iter()
    .collect();
    std::fs::read(path).ok()
}

#[test]
fn identifies_each_shipped_container() {
    if let Some(d) = sample("chr_cannon_cart_0.sctx") {
        assert_eq!(identify(&d), Kind::Sctx);
    }
    if let Some(d) = sample("chr_cannon_cart.sc") {
        assert_eq!(identify(&d), Kind::Sc);
    }
    if let Some(d) = sample("alchemist_default_geo.ingame.glb") {
        assert_eq!(identify(&d), Kind::Glb);
    }
}

#[test]
fn sctx_payload_decompresses_to_its_declared_length() {
    let Some(d) = sample("chr_cannon_cart_0.sctx") else {
        return;
    };
    let t = sctx::decode(&d).expect("SCTX decodes");

    // The declared length is located by matching it against what actually
    // decompressed, so a match here validates both the field and the decode.
    assert_eq!(
        t.declared_len,
        Some(t.payload.len() as u32),
        "header should record the decompressed payload length"
    );
    assert_eq!(t.payload.len(), 618_240, "known size for this sample");
    assert_eq!(t.payload_offset, 100);
}

#[test]
fn sctx_payload_is_astc_block_data() {
    let Some(d) = sample("chr_cannon_cart_0.sctx") else {
        return;
    };
    let t = sctx::decode(&d).expect("SCTX decodes");

    assert!(t.looks_like_astc(), "payload should be ASTC blocks");
    assert_eq!(t.block_count(), 618_240 / 16);

    // The first block is a void-extent block: a fully transparent margin,
    // which every packed sprite atlas has.
    assert_eq!(
        &t.payload[..8],
        &[0xFC, 0xFD, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        "expected an ASTC void-extent block at the start"
    );
}

#[test]
fn sctx_dimensions_remain_genuinely_ambiguous_from_size_alone() {
    let Some(d) = sample("chr_cannon_cart_0.sctx") else {
        return;
    };
    let t = sctx::decode(&d).expect("SCTX decodes");

    // This is the open question, pinned as a test so it is not forgotten:
    // payload size admits many dimension pairs, so the real width must come
    // from the FlatBuffers metadata rather than arithmetic.
    let cands = t.dimension_candidates(4, 4);
    assert!(
        cands.len() > 1,
        "if this ever returns one candidate, dimensions became derivable"
    );
    // 672x920 accounts for the payload exactly, but so do others.
    assert!(cands.contains(&(672, 920)) || cands.contains(&(920, 672)));
}

#[test]
fn glb_is_gltf_shaped_but_carries_a_flatbuffers_descriptor() {
    let Some(d) = sample("alchemist_default_geo.ingame.glb") else {
        return;
    };
    let g = glb::decode(&d).expect("glTF container parses");

    assert_eq!(g.version, 2, "standard glTF 2.0 header");
    assert_eq!(
        g.declared_len as usize,
        d.len(),
        "declared length should match the file"
    );

    // The reason a stock glTF loader cannot read these.
    assert!(g.is_supercell_variant());
    assert_eq!(g.descriptor().expect("descriptor").kind_str(), "FLA2");

    // The binary chunk is ordinary glTF payload and is the largest part.
    let bin = g.binary().expect("BIN chunk present");
    assert!(
        bin.data.len() > g.descriptor().unwrap().data.len(),
        "geometry should dominate the descriptor"
    );
}

#[test]
fn glb_descriptor_names_the_supercell_format_and_skeleton() {
    let Some(d) = sample("alchemist_default_geo.ingame.glb") else {
        return;
    };
    let g = glb::decode(&d).expect("parses");
    let strings = g.descriptor_strings(5);

    assert!(
        strings.iter().any(|s| s.contains("SC_odin_format")),
        "descriptor should name Supercell's own format"
    );
    // Joint names confirm the descriptor carries the scene graph and skinning
    // data that glTF would normally put in its JSON chunk.
    assert!(
        strings.iter().any(|s| s.contains("_03_s")),
        "expected skeleton joint names, got {:?}",
        &strings[..strings.len().min(10)]
    );
}

#[test]
fn glb_descriptor_parses_as_flatbuffers() {
    let Some(d) = sample("alchemist_default_geo.ingame.glb") else {
        return;
    };
    let g = glb::decode(&d).expect("parses");
    let t = g
        .descriptor_table()
        .expect("FLA2 chunk should parse as a FlatBuffers root table");

    // Structure is readable even without the schema: this is what makes
    // mapping the fields tractable.
    assert!(t.field_count() > 0);
    assert!(
        !t.present_fields().is_empty(),
        "root table should have populated fields"
    );
}

#[test]
fn sc_header_reports_version_six() {
    let Some(d) = sample("chr_cannon_cart.sc") else {
        return;
    };
    let h = sc::header(&d).expect("SC header parses");
    assert_eq!(h.version, 6, "shipped art in 18.400.11 is SC v6");
    assert_eq!(h.len, d.len());
}

#[test]
fn decoders_never_panic_on_truncated_input() {
    for name in [
        "chr_cannon_cart_0.sctx",
        "chr_cannon_cart.sc",
        "alchemist_default_geo.ingame.glb",
    ] {
        let Some(d) = sample(name) else { continue };
        // Every prefix must produce an error, never a panic.
        for cut in [0, 1, 4, 8, 11, 12, 16, 64, 99, 101, 1024] {
            if cut >= d.len() {
                continue;
            }
            let head = &d[..cut];
            let _ = sctx::decode(head);
            let _ = glb::decode(head);
            let _ = sc::header(head);
            let _ = identify(head);
        }
    }
}
