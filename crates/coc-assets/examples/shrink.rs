//! Downscales PNGs to a maximum width, preserving alpha.
//!
//! Building portraits ship at roughly 200px; the renderer draws them a few
//! dozen pixels wide, so shrinking cuts the embedded payload several-fold
//! with no visible loss.
//!
//! Usage: cargo run --release -p coc-assets --example shrink -- <indir> <outdir> [max_w]

use anyhow::Result;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    anyhow::ensure!(a.len() >= 3, "usage: shrink <indir> <outdir> [max_w]");
    let max_w: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(128);
    std::fs::create_dir_all(&a[2])?;

    let (mut done, mut before, mut after) = (0usize, 0u64, 0u64);
    for entry in std::fs::read_dir(&a[1])? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let raw = std::fs::read(&path)?;
        before += raw.len() as u64;

        let dec = png::Decoder::new(std::io::Cursor::new(&raw));
        let mut reader = match dec.read_info() {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf)?;
        let (w, h) = (info.width as usize, info.height as usize);
        let src = to_rgba(&buf[..info.buffer_size()], info.color_type, w, h);

        let (dw, dh) = if w > max_w {
            (max_w, (h * max_w).div_ceil(w).max(1))
        } else {
            (w, h)
        };

        // Box filter: average the source pixels covered by each destination
        // pixel, weighting colour by alpha so transparent edges do not bleed
        // dark halos into the sprite.
        let mut out = vec![0u8; dw * dh * 4];
        for y in 0..dh {
            for x in 0..dw {
                let (x0, x1) = ((x * w) / dw, (((x + 1) * w) / dw).max((x * w) / dw + 1));
                let (y0, y1) = ((y * h) / dh, (((y + 1) * h) / dh).max((y * h) / dh + 1));
                let (mut r, mut g, mut b, mut al, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
                for sy in y0..y1.min(h) {
                    for sx in x0..x1.min(w) {
                        let p = (sy * w + sx) * 4;
                        let a8 = src[p + 3] as u64;
                        r += src[p] as u64 * a8;
                        g += src[p + 1] as u64 * a8;
                        b += src[p + 2] as u64 * a8;
                        al += a8;
                        n += 1;
                    }
                }
                let d = (y * dw + x) * 4;
                if al > 0 {
                    out[d] = (r / al) as u8;
                    out[d + 1] = (g / al) as u8;
                    out[d + 2] = (b / al) as u8;
                }
                out[d + 3] = if n > 0 { (al / n) as u8 } else { 0 };
            }
        }

        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let dst = format!("{}/{name}", a[2]);
        let f = std::fs::File::create(&dst)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), dw as u32, dh as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Best);
        enc.write_header()?.write_image_data(&out)?;
        after += std::fs::metadata(&dst)?.len();
        done += 1;
    }
    println!("{done} images: {} KB -> {} KB", before / 1024, after / 1024);
    Ok(())
}

/// Expands whatever colour type the PNG used into straight RGBA.
fn to_rgba(buf: &[u8], ct: png::ColorType, w: usize, h: usize) -> Vec<u8> {
    let n = w * h;
    match ct {
        png::ColorType::Rgba => buf.to_vec(),
        png::ColorType::Rgb => {
            let mut o = vec![255u8; n * 4];
            for i in 0..n {
                o[i * 4..i * 4 + 3].copy_from_slice(&buf[i * 3..i * 3 + 3]);
            }
            o
        }
        png::ColorType::GrayscaleAlpha => {
            let mut o = vec![0u8; n * 4];
            for i in 0..n {
                let (v, a) = (buf[i * 2], buf[i * 2 + 1]);
                o[i * 4..i * 4 + 4].copy_from_slice(&[v, v, v, a]);
            }
            o
        }
        png::ColorType::Grayscale => {
            let mut o = vec![255u8; n * 4];
            for i in 0..n {
                let v = buf[i];
                o[i * 4..i * 4 + 3].copy_from_slice(&[v, v, v]);
            }
            o
        }
        png::ColorType::Indexed => vec![0u8; n * 4],
    }
}
