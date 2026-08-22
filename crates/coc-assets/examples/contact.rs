//! Builds contact sheets of the largest sprites across many atlases.
//!
//! Sprite identity is not recoverable from the `.sc` metadata yet, so the
//! fallback is to look at them. This decodes every atlas given, extracts
//! sprites by flood-filling opaque regions, ranks them by area and tiles the
//! biggest into labelled sheets.
//!
//! Usage: cargo run --release -p coc-assets --example contact -- <outdir> <a.sctx>...

use anyhow::{Context, Result};
use std::collections::VecDeque;

struct Sprite {
    atlas: usize,
    idx: usize,
    w: usize,
    h: usize,
    px: Vec<u8>,
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    anyhow::ensure!(a.len() >= 3, "usage: contact <outdir> <atlas.sctx>...");
    let outdir = &a[1];
    std::fs::create_dir_all(outdir)?;

    let mut sprites: Vec<Sprite> = Vec::new();
    for (ai, path) in a[2..].iter().enumerate() {
        let Ok(raw) = std::fs::read(path) else { continue };
        let Ok(tex) = coc_assets::sctx::decode(&raw) else { continue };
        let Some(m) = tex.meta else { continue };
        let Ok(rgba) = tex.to_rgba() else { continue };
        let (w, h) = (m.width as usize, m.height as usize);
        for (i, (x0, y0, x1, y1)) in regions(&rgba, w, h, 6000).into_iter().enumerate() {
            let (sw, sh) = (x1 - x0 + 1, y1 - y0 + 1);
            // Skip long thin strips: those are effect frames, not buildings.
            // Building sprites sit in a fairly narrow band: a 3x3 is roughly
            // 150px wide at the art's native tile size, a 4x4 around 200.
            // Outside that range are effect frames, UI cards and scenery.
            let (lo, hi) = size_band();
            if sw < lo || sh < lo || sw > hi || sh > hi {
                continue;
            }
            // Buildings are wider than tall or close to square-ish; skip
            // extreme aspect ratios, which are beams and trails.
            let ar = sw as f32 / sh as f32;
            if !(0.55..=2.2).contains(&ar) {
                continue;
            }
            let mut px = vec![0u8; sw * sh * 4];
            for y in 0..sh {
                let src = ((y0 + y) * w + x0) * 4;
                px[y * sw * 4..(y + 1) * sw * 4].copy_from_slice(&rgba[src..src + sw * 4]);
            }
            sprites.push(Sprite { atlas: ai, idx: i, w: sw, h: sh, px });
        }
    }
    // Group by footprint width so similar-sized buildings sit together.
    sprites.sort_by_key(|s| (std::cmp::Reverse(s.w / 20), std::cmp::Reverse(s.h)));
    println!("{} sprites collected", sprites.len());

    const COLS: usize = 8;
    const ROWS: usize = 5;
    const CELL: usize = 176;
    let per = COLS * ROWS;
    for (sheet, chunk) in sprites.chunks(per).enumerate().take(8) {
        let (sw, sh) = (COLS * CELL, ROWS * CELL);
        // Mid grey ground: Clash art is dark-outlined, so it reads on grey.
        let mut buf = vec![0u8; sw * sh * 4];
        for p in buf.chunks_exact_mut(4) {
            p.copy_from_slice(&[44, 48, 56, 255]);
        }
        for (k, s) in chunk.iter().enumerate() {
            let (cx, cy) = ((k % COLS) * CELL, (k / COLS) * CELL);
            // Contain-fit into the cell.
            let scale = ((CELL - 16) as f32 / s.w as f32).min((CELL - 16) as f32 / s.h as f32);
            let (dw, dh) = ((s.w as f32 * scale) as usize, (s.h as f32 * scale) as usize);
            let ox = cx + (CELL - dw) / 2;
            let oy = cy + (CELL - dh) / 2;
            for y in 0..dh {
                for x in 0..dw {
                    let sxp = (x as f32 / scale) as usize;
                    let syp = (y as f32 / scale) as usize;
                    if sxp >= s.w || syp >= s.h {
                        continue;
                    }
                    let sp = (syp * s.w + sxp) * 4;
                    let al = s.px[sp + 3] as u32;
                    if al == 0 {
                        continue;
                    }
                    let dp = ((oy + y) * sw + ox + x) * 4;
                    for c in 0..3 {
                        let src = s.px[sp + c] as u32;
                        let dst = buf[dp + c] as u32;
                        buf[dp + c] = ((src * al + dst * (255 - al)) / 255) as u8;
                    }
                }
            }
            // Cell index strip: three pixel-blocks encoding position in the sheet.
            for i in 0..=(k % COLS) {
                mark(&mut buf, sw, cx + 4 + i * 5, cy + 4, 4, [90, 200, 210, 255]);
            }
            for i in 0..=(k / COLS) {
                mark(&mut buf, sw, cx + 4, cy + 12 + i * 5, 4, [230, 170, 60, 255]);
            }
        }
        let path = format!("{outdir}/sheet{sheet}.png");
        let f = std::fs::File::create(&path).context("creating sheet")?;
        let mut e = png::Encoder::new(std::io::BufWriter::new(f), sw as u32, sh as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(&buf)?;
        println!("sheet{sheet}: {} sprites -> {path}", chunk.len());
        for (k, s) in chunk.iter().enumerate() {
            println!("   r{} c{}  atlas={} idx={} {}x{}", k / COLS, k % COLS, s.atlas, s.idx, s.w, s.h);
        }
    }
    Ok(())
}

/// Sprite size band to keep, overridable for experimentation.
fn size_band() -> (usize, usize) {
    let get = |k: &str, d: usize| {
        std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
    };
    (get("SPRITE_MIN", 90), get("SPRITE_MAX", 300))
}

fn mark(buf: &mut [u8], sw: usize, x: usize, y: usize, n: usize, c: [u8; 4]) {
    for dy in 0..n {
        for dx in 0..n {
            let p = ((y + dy) * sw + x + dx) * 4;
            if p + 4 <= buf.len() {
                buf[p..p + 4].copy_from_slice(&c);
            }
        }
    }
}

/// Bounding boxes of connected opaque regions of at least `min_px` pixels.
fn regions(rgba: &[u8], w: usize, h: usize, min_px: usize) -> Vec<(usize, usize, usize, usize)> {
    let opaque = |i: usize| rgba[i * 4 + 3] > 8;
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    for start in 0..w * h {
        if seen[start] || !opaque(start) {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (w, h, 0usize, 0usize, 0usize);
        let mut q = VecDeque::from([start]);
        seen[start] = true;
        while let Some(p) = q.pop_front() {
            let (x, y) = (p % w, p / w);
            x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y); n += 1;
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 { continue; }
                let np = ny as usize * w + nx as usize;
                if !seen[np] && opaque(np) { seen[np] = true; q.push_back(np); }
            }
        }
        if n >= min_px { out.push((x0, y0, x1, y1)); }
    }
    out
}
