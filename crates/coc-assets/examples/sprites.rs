//! Extracts individual sprites from a decoded SCTX atlas.
//!
//! Atlases pack many sprites — and many animation frames — onto one sheet with
//! transparent gutters between them. Connected regions of non-transparent
//! pixels therefore recover the individual sprite rectangles without needing
//! the `.sc` record stream that names them.
//!
//! Usage: cargo run -p coc-assets --example sprites -- <in.sctx> <outdir> [min_px]

use anyhow::{Context, Result};
use std::collections::VecDeque;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    anyhow::ensure!(a.len() >= 3, "usage: sprites <in.sctx> <outdir> [min_px]");
    let min_px: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(64);

    let raw = std::fs::read(&a[1])?;
    let tex = coc_assets::sctx::decode(&raw)?;
    let m = tex.meta.context("no metadata")?;
    let (w, h) = (m.width as usize, m.height as usize);
    let rgba = tex.to_rgba()?;
    std::fs::create_dir_all(&a[2])?;

    // Flood fill over opaque pixels. Alpha 8 rather than 0 ignores the faint
    // halo ASTC leaves around a sprite's edge, which would otherwise bridge
    // neighbouring sprites into one blob.
    let opaque = |i: usize| rgba[i * 4 + 3] > 8;
    let mut seen = vec![false; w * h];
    let mut boxes: Vec<(usize, usize, usize, usize, usize)> = Vec::new();

    for start in 0..w * h {
        if seen[start] || !opaque(start) {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (w, h, 0usize, 0usize, 0usize);
        let mut q = VecDeque::from([start]);
        seen[start] = true;
        while let Some(p) = q.pop_front() {
            let (x, y) = (p % w, p / w);
            x0 = x0.min(x); y0 = y0.min(y);
            x1 = x1.max(x); y1 = y1.max(y);
            n += 1;
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 { continue; }
                let np = ny as usize * w + nx as usize;
                if !seen[np] && opaque(np) {
                    seen[np] = true;
                    q.push_back(np);
                }
            }
        }
        if n >= min_px { boxes.push((x0, y0, x1, y1, n)); }
    }

    // Biggest first: the largest regions are the finished buildings, while the
    // long tail is projectiles and effect frames.
    boxes.sort_by_key(|b| std::cmp::Reverse(b.4));
    println!("{} sprites >= {min_px}px in {}x{}", boxes.len(), w, h);

    for (i, &(x0, y0, x1, y1, n)) in boxes.iter().take(24).enumerate() {
        let (sw, sh) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut crop = vec![0u8; sw * sh * 4];
        for y in 0..sh {
            let src = ((y0 + y) * w + x0) * 4;
            crop[y * sw * 4..(y + 1) * sw * 4].copy_from_slice(&rgba[src..src + sw * 4]);
        }
        let path = format!("{}/sprite_{i:02}_{sw}x{sh}.png", a[2]);
        let f = std::fs::File::create(&path)?;
        let mut e = png::Encoder::new(std::io::BufWriter::new(f), sw as u32, sh as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(&crop)?;
        println!("  {i:02}: {sw:4}x{sh:4} at ({x0},{y0}) {n} px -> {path}");
    }
    Ok(())
}
