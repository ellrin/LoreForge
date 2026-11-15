//! Figures at the size the editor shows them. A paper's figures are often
//! 300-dpi plots thousands of pixels wide; the page shows them a few hundred
//! pixels wide, so the editor asks for a copy about twice its display width
//! (sharp on Retina screens) and the original is kept for export and print.
//!
//! PNG is decoded, shrunk and encoded here (flate2 only, no image crate);
//! JPEG and WebP go through `sips` (macOS) or ImageMagick when present, else
//! the original is served. Copies are cached by file, mtime and width.
//! Sizes are read from file headers so the page can reserve each figure's
//! aspect ratio before the image loads.

use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression, Crc};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
/// Larger images are served as they are rather than decoded here.
const MAX_PIXELS: u64 = 120_000_000;

fn be16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?)))
}
fn be32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn le16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?)))
}
fn le24(b: &[u8], i: usize) -> Option<u32> {
    let s = b.get(i..i + 3)?;
    Some(u32::from(s[0]) | u32::from(s[1]) << 8 | u32::from(s[2]) << 16)
}

/// Width and height from an image file's header (PDF/EPS in points, SVG in
/// user units): only the ratio matters to the page.
pub fn image_size(file: &Path) -> Option<(f64, f64)> {
    let ext = file.extension()?.to_string_lossy().to_lowercase();
    let b = std::fs::read(file).ok()?;
    let px = |s: Option<(u32, u32)>| s.filter(|&(w, h)| w > 0 && h > 0).map(|(w, h)| (f64::from(w), f64::from(h)));
    match ext.as_str() {
        "png" => px(png_size(&b)),
        "jpg" | "jpeg" => px(jpeg_size(&b)),
        "gif" => px((b.starts_with(b"GIF8")).then(|| (le16(&b, 6), le16(&b, 8))).and_then(|(w, h)| Some((w?, h?)))),
        "webp" => px(webp_size(&b)),
        "pdf" => pdf_size(&b),
        "eps" | "ps" => eps_size(&b),
        "svg" => svg_size(&String::from_utf8_lossy(&b)),
        _ => None,
    }
}

pub fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    (b.starts_with(PNG_SIG) && b.get(12..16) == Some(b"IHDR")).then_some(())?;
    Some((be32(b, 16)?, be32(b, 20)?))
}

fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    if !b.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        if m == 0xFF {
            i += 1;
            continue;
        }
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) {
            i += 2;
            continue;
        }
        let len = be16(b, i + 2)? as usize;
        // start of frame (not DHT, JPG, DAC)
        if (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC {
            return Some((be16(b, i + 7)?, be16(b, i + 5)?));
        }
        i += 2 + len;
    }
    None
}

fn webp_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.get(0..4) != Some(b"RIFF") || b.get(8..12) != Some(b"WEBP") {
        return None;
    }
    match b.get(12..16)? {
        b"VP8 " => Some((le16(b, 26)? & 0x3FFF, le16(b, 28)? & 0x3FFF)),
        b"VP8L" => {
            let (b1, b2, b3, b4) = (u32::from(*b.get(21)?), u32::from(*b.get(22)?), u32::from(*b.get(23)?), u32::from(*b.get(24)?));
            Some((1 + (((b2 & 0x3F) << 8) | b1), 1 + (((b4 & 0xF) << 10) | (b3 << 2) | ((b2 & 0xC0) >> 6))))
        }
        b"VP8X" => Some((1 + le24(b, 24)?, 1 + le24(b, 27)?)),
        _ => None,
    }
}

/// Four numbers after `key` (`/MediaBox [0 0 612 792]`, `%%BoundingBox: 0 0 400 300`).
fn box_after(text: &str, key: &str) -> Option<(f64, f64)> {
    let i = text.find(key)? + key.len();
    let rest = text[i..].trim_start().trim_start_matches(['[', ':']);
    let n: Vec<f64> = rest.split(|c: char| c.is_whitespace() || c == ']').filter(|s| !s.is_empty()).take(4).map(|s| s.parse().ok()).collect::<Option<_>>()?;
    let (w, h) = ((n.get(2)? - n.first()?).abs(), (n.get(3)? - n.get(1)?).abs());
    (w > 0.0 && h > 0.0).then_some((w, h))
}

/// First page box of a PDF, when its page objects are not compressed.
fn pdf_size(b: &[u8]) -> Option<(f64, f64)> {
    let text = String::from_utf8_lossy(b);
    box_after(&text, "/CropBox").or_else(|| box_after(&text, "/MediaBox"))
}

fn eps_size(b: &[u8]) -> Option<(f64, f64)> {
    let head = String::from_utf8_lossy(&b[..b.len().min(64 * 1024)]);
    box_after(&head, "%%HiResBoundingBox:").or_else(|| box_after(&head, "%%BoundingBox:"))
}

fn svg_size(s: &str) -> Option<(f64, f64)> {
    let tag = &s[s.find("<svg")?..];
    let tag = &tag[..tag.find('>')?];
    let attr = |name: &str| -> Option<&str> {
        let i = tag.find(&format!(" {name}=\""))? + name.len() + 3;
        Some(&tag[i..i + tag[i..].find('"')?])
    };
    if let Some(v) = attr("viewBox") {
        let n: Vec<f64> = v.split([' ', ',']).filter(|x| !x.is_empty()).filter_map(|x| x.parse().ok()).collect();
        if n.len() == 4 && n[2] > 0.0 && n[3] > 0.0 {
            return Some((n[2], n[3]));
        }
    }
    let num = |v: &str| v.trim_end_matches(|c: char| c.is_ascii_alphabetic()).parse::<f64>().ok().filter(|x| *x > 0.0);
    Some((num(attr("width")?)?, num(attr("height")?)?))
}

/// An image as 8-bit RGBA.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

/// Decode a non-interlaced PNG of any colour type and bit depth.
pub fn decode_png(b: &[u8]) -> Option<Rgba> {
    if !b.starts_with(PNG_SIG) {
        return None;
    }
    let (mut w, mut h, mut depth, mut ct) = (0u32, 0u32, 0u8, 0u8);
    let (mut plte, mut trns, mut idat) = (Vec::new(), Vec::new(), Vec::new());
    let mut i = 8;
    while i + 8 <= b.len() {
        let len = be32(b, i)? as usize;
        let kind = b.get(i + 4..i + 8)?;
        let data = b.get(i + 8..i + 8 + len)?;
        match kind {
            b"IHDR" => {
                w = be32(data, 0)?;
                h = be32(data, 4)?;
                depth = *data.get(8)?;
                ct = *data.get(9)?;
                if *data.get(12)? != 0 {
                    return None; // interlaced: served as it is
                }
            }
            b"PLTE" => plte = data.to_vec(),
            b"tRNS" => trns = data.to_vec(),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        i += 12 + len;
    }
    let channels: u32 = match ct {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return None,
    };
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS || ![1, 2, 4, 8, 16].contains(&depth) {
        return None;
    }
    let bits = channels * u32::from(depth);
    let stride = ((w as usize) * bits as usize).div_ceil(8);
    let bpp = (bits as usize).div_ceil(8).max(1);
    let mut raw = Vec::with_capacity((stride + 1) * h as usize);
    ZlibDecoder::new(&idat[..]).read_to_end(&mut raw).ok()?;
    if raw.len() < (stride + 1) * h as usize {
        return None;
    }
    let mut prev = vec![0u8; stride];
    let mut cur = vec![0u8; stride];
    let mut px = Vec::with_capacity(w as usize * h as usize * 4);
    let max = (1u32 << depth.min(8)) - 1;
    for y in 0..h as usize {
        let row = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        cur.copy_from_slice(&row[1..]);
        unfilter(row[0], &mut cur, &prev, bpp)?;
        // sample k of the row at the image's bit depth, as 8 bits
        let sample = |k: usize| -> u8 {
            match depth {
                16 => cur[k * 2],
                8 => cur[k],
                d => {
                    let bit = k * d as usize;
                    let v = (cur[bit / 8] >> (8 - d as usize - bit % 8)) & ((1u8 << d) - 1);
                    if ct == 3 { v } else { (u32::from(v) * 255 / max) as u8 }
                }
            }
        };
        // transparent colour (gray or RGB) at the original depth
        let raw16 = |k: usize| -> u16 { if depth == 16 { u16::from_be_bytes([cur[k * 2], cur[k * 2 + 1]]) } else { u16::from(sample(k)) } };
        for x in 0..w as usize {
            match ct {
                0 => {
                    let g = sample(x);
                    let a = if trns.len() >= 2 && depth >= 8 && raw16(x) == u16::from_be_bytes([trns[0], trns[1]]) { 0 } else { 255 };
                    px.extend_from_slice(&[g, g, g, a]);
                }
                2 => {
                    let (r, g, bl) = (sample(x * 3), sample(x * 3 + 1), sample(x * 3 + 2));
                    let clear = trns.len() >= 6 && depth >= 8 && (0..3).all(|c| raw16(x * 3 + c) == u16::from_be_bytes([trns[c * 2], trns[c * 2 + 1]]));
                    px.extend_from_slice(&[r, g, bl, if clear { 0 } else { 255 }]);
                }
                3 => {
                    let idx = sample(x) as usize;
                    let c = plte.get(idx * 3..idx * 3 + 3).unwrap_or(&[0, 0, 0]);
                    px.extend_from_slice(&[c[0], c[1], c[2], *trns.get(idx).unwrap_or(&255)]);
                }
                4 => {
                    let g = sample(x * 2);
                    px.extend_from_slice(&[g, g, g, sample(x * 2 + 1)]);
                }
                _ => px.extend_from_slice(&[sample(x * 4), sample(x * 4 + 1), sample(x * 4 + 2), sample(x * 4 + 3)]),
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    Some(Rgba { w, h, px })
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn unfilter(kind: u8, cur: &mut [u8], prev: &[u8], bpp: usize) -> Option<()> {
    for i in 0..cur.len() {
        let a = if i >= bpp { cur[i - bpp] } else { 0 };
        let c = if i >= bpp { prev[i - bpp] } else { 0 };
        let b = prev[i];
        cur[i] = cur[i].wrapping_add(match kind {
            0 => 0,
            1 => a,
            2 => b,
            3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
            4 => paeth(a, b, c),
            _ => return None,
        });
    }
    Some(())
}

/// Shrink to `tw` pixels wide (aspect kept) by averaging each source area,
/// with alpha-weighted colour so transparent edges do not darken.
pub fn shrink(img: &Rgba, tw: u32) -> Rgba {
    let tw = tw.clamp(1, img.w);
    let th = ((u64::from(img.h) * u64::from(tw) + u64::from(img.w) / 2) / u64::from(img.w)).max(1) as u32;
    let (sw, sh) = (img.w as usize, img.h as usize);
    let mut px = Vec::with_capacity(tw as usize * th as usize * 4);
    let xs: Vec<(usize, usize)> = (0..tw as usize).map(|x| (x * sw / tw as usize, ((x + 1) * sw / tw as usize).max(x * sw / tw as usize + 1))).collect();
    for y in 0..th as usize {
        let (y0, y1) = (y * sh / th as usize, ((y + 1) * sh / th as usize).max(y * sh / th as usize + 1));
        for &(x0, x1) in &xs {
            let mut s = [0u64; 4];
            for yy in y0..y1 {
                let row = &img.px[(yy * sw + x0) * 4..(yy * sw + x1) * 4];
                for p in row.chunks_exact(4) {
                    let a = u64::from(p[3]);
                    s[0] += u64::from(p[0]) * a;
                    s[1] += u64::from(p[1]) * a;
                    s[2] += u64::from(p[2]) * a;
                    s[3] += a;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            // fully transparent area: no colour to average
            px.extend_from_slice(&match (s[0].checked_div(s[3]), s[1].checked_div(s[3]), s[2].checked_div(s[3])) {
                (Some(r), Some(g), Some(b)) => [r as u8, g as u8, b as u8, ((s[3] + n / 2) / n) as u8],
                _ => [0, 0, 0, 0],
            });
        }
    }
    Rgba { w: tw, h: th, px }
}

/// Encode as PNG: RGB when fully opaque, else RGBA; each row filtered with
/// whichever of the standard filters leaves the smallest residue.
pub fn encode_png(img: &Rgba) -> Vec<u8> {
    let alpha = img.px.chunks_exact(4).any(|p| p[3] != 255);
    let bpp = if alpha { 4 } else { 3 };
    let stride = img.w as usize * bpp;
    let rows: Vec<Vec<u8>> = img
        .px
        .chunks_exact(img.w as usize * 4)
        .map(|r| if alpha { r.to_vec() } else { r.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect() })
        .collect();
    let mut raw = Vec::with_capacity((stride + 1) * rows.len());
    let zero = vec![0u8; stride];
    let mut out = vec![0u8; stride];
    for (y, row) in rows.iter().enumerate() {
        let prev = if y > 0 { &rows[y - 1] } else { &zero };
        let mut best = (u64::MAX, 0u8, Vec::new());
        for kind in 0..5u8 {
            for i in 0..stride {
                let a = if i >= bpp { row[i - bpp] } else { 0 };
                let c = if i >= bpp { prev[i - bpp] } else { 0 };
                let b = prev[i];
                out[i] = row[i].wrapping_sub(match kind {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                    _ => paeth(a, b, c),
                });
            }
            let cost: u64 = out.iter().map(|&v| u64::from((v as i8).unsigned_abs())).sum();
            if cost < best.0 {
                best = (cost, kind, out.clone());
            }
        }
        raw.push(best.1);
        raw.extend_from_slice(&best.2);
    }
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    let _ = z.write_all(&raw);
    let data = z.finish().unwrap_or_default();
    let mut png = PNG_SIG.to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&img.w.to_be_bytes());
    ihdr.extend_from_slice(&img.h.to_be_bytes());
    ihdr.extend_from_slice(&[8, if alpha { 6 } else { 2 }, 0, 0, 0]);
    for (kind, body) in [(&b"IHDR"[..], &ihdr[..]), (b"IDAT", &data[..]), (b"IEND", &[][..])] {
        png.extend_from_slice(&(body.len() as u32).to_be_bytes());
        png.extend_from_slice(kind);
        png.extend_from_slice(body);
        let mut crc = Crc::new();
        crc.update(kind);
        crc.update(body);
        png.extend_from_slice(&crc.sum().to_be_bytes());
    }
    png
}

/// A copy of the image `file` (already something a browser shows) at most
/// `width` pixels wide, cached; the file itself when it is no wider, cannot be
/// shrunk here, or is a vector (SVG).
pub async fn thumbnail(file: &Path, mime: &'static str, cache: &Path, width: u32) -> (PathBuf, &'static str) {
    let keep = (file.to_path_buf(), mime);
    let Ok(bytes) = std::fs::read(file) else { return keep };
    let size = match mime {
        "image/png" => png_size(&bytes),
        "image/jpeg" => jpeg_size(&bytes),
        "image/webp" => webp_size(&bytes),
        _ => None,
    };
    let Some((w, _)) = size else { return keep };
    if w <= width {
        return keep;
    }
    let stamp = std::fs::metadata(file).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(file, stamp, width), &mut h);
    let ext = if mime == "image/png" { "png" } else { "jpg" };
    let out = cache.join(format!("{:016x}-w{width}.{ext}", std::hash::Hasher::finish(&h)));
    if out.is_file() {
        return (out, if ext == "png" { "image/png" } else { "image/jpeg" });
    }
    if std::fs::create_dir_all(cache).is_err() {
        return keep;
    }
    if mime == "image/png" {
        let target = out.clone();
        let made = tokio::task::spawn_blocking(move || {
            let img = decode_png(&bytes)?;
            let png = encode_png(&shrink(&img, width));
            // write next to the target and rename, so a half-written file is never served
            let tmp = target.with_extension("part");
            std::fs::write(&tmp, png).ok()?;
            std::fs::rename(&tmp, &target).ok()
        })
        .await
        .ok()
        .flatten();
        return if made.is_some() { (out, "image/png") } else { keep };
    }
    // JPEG / WebP: the system's tools, when there are any
    let mut ok = false;
    if cfg!(target_os = "macos") {
        ok = crate::proc::cmd("sips")
            .args(["-s", "format", "jpeg", "--resampleWidth", &width.to_string()])
            .arg(file)
            .arg("--out")
            .arg(&out)
            .kill_on_drop(true)
            .output()
            .await
            .is_ok_and(|o| o.status.success());
    }
    if !ok {
        for tool in ["magick", "convert"] {
            let Some(exe) = crate::texmeasure::tool(tool) else { continue };
            ok = crate::proc::cmd(exe)
                .arg(file)
                .args(["-resize", &format!("{width}x"), "-quality", "88"])
                .arg(&out)
                .kill_on_drop(true)
                .output()
                .await
                .is_ok_and(|o| o.status.success());
            if ok {
                break;
            }
        }
    }
    if ok && out.is_file() { (out, "image/jpeg") } else { keep }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(w: u32, h: u32, alpha: bool) -> Rgba {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&[(x * 7) as u8, (y * 5) as u8, ((x + y) * 3) as u8, if alpha { (x * 11 % 256) as u8 } else { 255 }]);
            }
        }
        Rgba { w, h, px }
    }

    #[test]
    fn png_round_trip() {
        for alpha in [false, true] {
            let img = sample(37, 23, alpha);
            let png = encode_png(&img);
            assert_eq!(png_size(&png), Some((37, 23)));
            assert_eq!(decode_png(&png), Some(img));
        }
    }

    #[test]
    fn decodes_palette_and_low_bit_depth() {
        // 2x2 palette image, 2 bits per pixel: indices 0 1 / 2 3
        let mut raw = vec![0u8, 0b0001_0000, 0, 0b1011_0000];
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(&raw).unwrap();
        raw = z.finish().unwrap();
        let mut png = PNG_SIG.to_vec();
        let chunk = |png: &mut Vec<u8>, kind: &[u8], body: &[u8]| {
            png.extend_from_slice(&(body.len() as u32).to_be_bytes());
            png.extend_from_slice(kind);
            png.extend_from_slice(body);
            png.extend_from_slice(&[0, 0, 0, 0]);
        };
        chunk(&mut png, b"IHDR", &[0, 0, 0, 2, 0, 0, 0, 2, 2, 3, 0, 0, 0]);
        chunk(&mut png, b"PLTE", &[255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9]);
        chunk(&mut png, b"tRNS", &[255, 128]);
        chunk(&mut png, b"IDAT", &raw);
        chunk(&mut png, b"IEND", &[]);
        let img = decode_png(&png).unwrap();
        assert_eq!(img.px, vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 9, 9, 9, 255]);
    }

    #[test]
    fn shrinks_keeping_aspect_and_colour() {
        let mut img = sample(400, 300, false);
        img.px.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&[10, 20, 30, 255]));
        let t = shrink(&img, 100);
        assert_eq!((t.w, t.h), (100, 75));
        assert!(t.px.chunks_exact(4).all(|p| p == [10, 20, 30, 255]));
        // never wider than the original
        assert_eq!(shrink(&img, 900).w, 400);
    }

    #[test]
    fn reads_sizes_from_headers() {
        let d = std::env::temp_dir().join(format!("pc-thumb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.png"), encode_png(&sample(30, 20, false))).unwrap();
        std::fs::write(d.join("b.pdf"), "%PDF-1.4\n1 0 obj << /Type /Page /MediaBox [0 0 612 396] >>").unwrap();
        std::fs::write(d.join("c.eps"), "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 10 20 410 320\n").unwrap();
        std::fs::write(d.join("d.svg"), "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 200 100\">").unwrap();
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 0, 50, 0, 80, 3];
        jpg.extend_from_slice(&[0; 9]);
        std::fs::write(d.join("e.jpg"), jpg).unwrap();
        assert_eq!(image_size(&d.join("a.png")), Some((30.0, 20.0)));
        assert_eq!(image_size(&d.join("b.pdf")), Some((612.0, 396.0)));
        assert_eq!(image_size(&d.join("c.eps")), Some((400.0, 300.0)));
        assert_eq!(image_size(&d.join("d.svg")), Some((200.0, 100.0)));
        assert_eq!(image_size(&d.join("e.jpg")), Some((80.0, 50.0)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn thumbnails_are_cached_and_never_upscaled() {
        let d = std::env::temp_dir().join(format!("pc-thumb-t-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("big.png");
        std::fs::write(&f, encode_png(&sample(600, 400, false))).unwrap();
        let (small, mime) = thumbnail(&f, "image/png", &d.join("cache"), 256).await;
        assert_eq!(mime, "image/png");
        assert_ne!(small, f);
        assert_eq!(png_size(&std::fs::read(&small).unwrap()), Some((256, 171)));
        assert_eq!(thumbnail(&f, "image/png", &d.join("cache"), 256).await.0, small);
        assert_eq!(thumbnail(&f, "image/png", &d.join("cache"), 1024).await.0, f);
        let _ = std::fs::remove_dir_all(&d);
    }
}
