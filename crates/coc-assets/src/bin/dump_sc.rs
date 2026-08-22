//! `cargo run -p coc-assets --bin dump_sc -- <file.sc> <out.bin>`
//!
//! Writes the decompressed object graph so it can be explored outside Rust.
use anyhow::Result;
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&a[1])?;
    let sc = coc_assets::sc::decode(&data)?;
    eprintln!("exports {} objects {} textures {} blob {}",
        sc.exports.len(), sc.objects.len(), sc.textures.len(), sc.blob.len());
    std::fs::write(&a[2], &sc.blob)?;
    Ok(())
}
