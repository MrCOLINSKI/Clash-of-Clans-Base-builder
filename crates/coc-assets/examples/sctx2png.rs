//! Decodes an SCTX texture to a PNG.
//!
//! Usage: cargo run -p coc-assets --example sctx2png -- <in.sctx> <out.png>

use anyhow::{Context, Result};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    anyhow::ensure!(args.len() >= 3, "usage: sctx2png <in.sctx> <out.png>");

    let raw = std::fs::read(&args[1]).with_context(|| format!("reading {}", args[1]))?;
    let tex = coc_assets::sctx::decode(&raw)?;
    let meta = tex.meta.context("SCTX carried no readable metadata")?;

    println!(
        "{}: {}x{} format={} {} ASTC {}x{} blocks, {} bytes",
        args[1],
        meta.width,
        meta.height,
        meta.format,
        tex.block_count(),
        meta.footprint().0,
        meta.footprint().1,
        tex.payload.len()
    );

    let rgba = tex.to_rgba()?;
    let file = std::fs::File::create(&args[2])?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), meta.width as u32, meta.height as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgba)?;
    println!("wrote {}", args[2]);
    Ok(())
}
