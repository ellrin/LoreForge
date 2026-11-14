//! Animated GIF of an equation figure: RGBA frames (rendered at print quality in the figure's
//! style) → one GIF with a single global palette, no dithering (smooth lines stay clean), white
//! and black kept exact, looping forever. The spec rides in an application extension
//! (`LOREPLOT1.0`), so a slide picture re-opens in the editor (PowerPoint keeps the GIF's bytes).

use std::collections::HashMap;

/// One frame, 8-bit RGBA, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

const APP_ID: &[u8; 11] = b"LOREPLOT1.0";

/// Colours over white (GIF has no partial alpha).
fn rgb(p: &[u8]) -> [u8; 3] {
    let a = p[3] as u32;
    let mix = |c: u8| ((c as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
    [mix(p[0]), mix(p[1]), mix(p[2])]
}

fn key(c: [u8; 3]) -> u32 {
    (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32
}

/// The global palette (≤ 256 RGB entries): the most frequent colours exactly (white, black, the
/// line colours), NeuQuant for the anti-aliasing in between.
pub fn palette(frames: &[Rgba]) -> Vec<[u8; 3]> {
    let mut count: HashMap<u32, u64> = HashMap::new();
    for f in frames {
        for p in f.px.chunks_exact(4) {
            *count.entry(key(rgb(p))).or_insert(0) += 1;
        }
    }
    let unpack = |k: u32| [(k >> 16) as u8, (k >> 8) as u8, k as u8];
    let mut by: Vec<(u32, u64)> = count.into_iter().collect();
    by.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut pal: Vec<[u8; 3]> = vec![[255, 255, 255], [0, 0, 0]];
    // ≤ 255 colours: one index stays free for "unchanged" (transparent) pixels of later frames
    if by.len() <= 253 {
        for (k, _) in &by {
            let c = unpack(*k);
            if !pal.contains(&c) {
                pal.push(c);
            }
        }
        return pal;
    }
    for (k, _) in by.iter().take(48) {
        let c = unpack(*k);
        if !pal.contains(&c) {
            pal.push(c);
        }
    }
    // the rest: NeuQuant over the remaining colours, weighted by how often they occur (capped)
    let mut sample: Vec<u8> = Vec::new();
    for (k, n) in by.iter().skip(48) {
        let c = unpack(*k);
        for _ in 0..(*n).clamp(1, 16) {
            sample.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        if sample.len() > 4 * 400_000 {
            break;
        }
    }
    let room = 255 - pal.len();
    if room >= 4 && sample.len() >= 4 * 4 {
        let nq = color_quant::NeuQuant::new(10, room, &sample);
        for c in nq.color_map_rgb().chunks_exact(3) {
            let c = [c[0], c[1], c[2]];
            if !pal.contains(&c) && pal.len() < 255 {
                pal.push(c);
            }
        }
    }
    pal
}

fn nearest(pal: &[[u8; 3]], c: [u8; 3]) -> u8 {
    let mut best = 0;
    let mut bd = u32::MAX;
    for (i, p) in pal.iter().enumerate() {
        let d = (0..3).map(|k| (p[k] as i32 - c[k] as i32).pow(2) as u32).sum::<u32>();
        if d < bd {
            bd = d;
            best = i;
            if d == 0 {
                break;
            }
        }
    }
    best as u8
}

/// How frames are timed and where a light ordered dither applies.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Opts {
    /// Per-frame delays (ms); missing entries use the common delay. GIF pacing ends with a hold.
    pub delays: Vec<u32>,
    /// Rectangles `[x, y, w, h]` in fractions of the frame (heatmaps): a 4×4 Bayer dither there
    /// against the palette's banding; everything else (lines, text) is mapped exactly.
    pub dither: Vec<[f64; 4]>,
}

const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
/// Amplitude of the dither (per channel, ± half of it).
const DITHER_AMP: f64 = 12.0;

/// Encode `frames` (all the size of the first; others are cropped / padded white).
pub fn encode(frames: &[Rgba], delay_ms: u32, spec_json: Option<&str>) -> Result<Vec<u8>, String> {
    encode_with(frames, delay_ms, spec_json, &Opts::default())
}

/// [`encode`] with per-frame delays and dithered rectangles.
pub fn encode_with(frames: &[Rgba], delay_ms: u32, spec_json: Option<&str>, opts: &Opts) -> Result<Vec<u8>, String> {
    let first = frames.first().ok_or("沒有任何一格")?;
    let (w, h) = (first.w, first.h);
    if w == 0 || h == 0 || w > 4000 || h > 4000 {
        return Err(format!("大小 {w}×{h} 不合理"));
    }
    let pal = palette(frames);
    // the index after the palette marks "same as the frame before" (transparent)
    let keep = pal.len() as u8;
    let mut flat: Vec<u8> = pal.iter().flatten().cloned().collect();
    flat.extend_from_slice(&[255, 0, 255]); // `keep`: never drawn
    while flat.len() < 6 {
        flat.extend_from_slice(&[0, 0, 0]);
    }
    let mut out = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut out, w as u16, h as u16, &flat).map_err(|e| e.to_string())?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
        if let Some(j) = spec_json {
            enc.write_raw_extension(gif::AnyExtension(0xFF), &[APP_ID, j.as_bytes()]).map_err(|e| e.to_string())?;
        }
        let mut cache: HashMap<u32, u8> = HashMap::new();
        let cs = |ms: u32| (ms / 10).clamp(2, 6000) as u16;
        let rects: Vec<[u32; 4]> = opts
            .dither
            .iter()
            .filter(|r| r.iter().all(|v| v.is_finite()))
            .map(|r| {
                let x0 = (r[0].clamp(0.0, 1.0) * w as f64).floor() as u32;
                let y0 = (r[1].clamp(0.0, 1.0) * h as f64).floor() as u32;
                [x0, y0, ((r[0] + r[2]).clamp(0.0, 1.0) * w as f64).ceil() as u32, ((r[1] + r[3]).clamp(0.0, 1.0) * h as f64).ceil() as u32]
            })
            .collect();
        let dithered = |x: u32, y: u32| rects.iter().any(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3]);
        let mut prev: Option<Vec<u8>> = None;
        for (fi, f) in frames.iter().enumerate() {
            let mut idx = Vec::with_capacity((w * h) as usize);
            for y in 0..h {
                for x in 0..w {
                    let mut c = if x < f.w && y < f.h {
                        let i = ((y * f.w + x) * 4) as usize;
                        rgb(&f.px[i..i + 4])
                    } else {
                        [255, 255, 255]
                    };
                    if !rects.is_empty() && dithered(x, y) {
                        let off = ((BAYER[(y % 4) as usize][(x % 4) as usize] as f64 + 0.5) / 16.0 - 0.5) * DITHER_AMP;
                        c = c.map(|v| (v as f64 + off).round().clamp(0.0, 255.0) as u8);
                    }
                    let k = key(c);
                    let ix = *cache.entry(k).or_insert_with(|| nearest(&pal, c));
                    idx.push(ix);
                }
            }
            // after the first frame only the rectangle that changed is stored (the rest stays on screen):
            // titles, rows and axes cost nothing per frame
            let (x0, y0, x1, y1) = match &prev {
                None => (0, 0, w, h),
                Some(p) => changed_rect(p, &idx, w, h).unwrap_or((0, 0, 1, 1)),
            };
            let sub: Vec<u8> = match &prev {
                None => idx.clone(),
                Some(p) => (y0..y1).flat_map(|y| (x0..x1).map(move |x| (y * w + x) as usize)).map(|i| if p[i] == idx[i] { keep } else { idx[i] }).collect(),
            };
            let mut fr = gif::Frame::from_indexed_pixels((x1 - x0) as u16, (y1 - y0) as u16, sub, prev.as_ref().map(|_| keep));
            fr.left = x0 as u16;
            fr.top = y0 as u16;
            fr.dispose = gif::DisposalMethod::Keep;
            fr.delay = cs(opts.delays.get(fi).copied().unwrap_or(delay_ms));
            enc.write_frame(&fr).map_err(|e| e.to_string())?;
            prev = Some(idx);
        }
    }
    Ok(out)
}

/// The smallest rectangle `(x0, y0, x1, y1)` where two index buffers differ (None: identical).
fn changed_rect(a: &[u8], b: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        let row = (y * w) as usize..((y + 1) * w) as usize;
        let (ra, rb) = (&a[row.clone()], &b[row]);
        if ra == rb {
            continue;
        }
        let first = ra.iter().zip(rb).position(|(p, q)| p != q).unwrap_or(0) as u32;
        let last = w - 1 - ra.iter().rev().zip(rb.iter().rev()).position(|(p, q)| p != q).unwrap_or(0) as u32;
        x0 = x0.min(first);
        x1 = x1.max(last + 1);
        y0 = y0.min(y);
        y1 = y + 1;
    }
    (x1 > x0).then_some((x0, y0, x1, y1))
}

/// What a GIF holds, read without decoding pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub w: u32,
    pub h: u32,
    pub frames: usize,
    /// NETSCAPE2.0 loop count (0 = forever), if any.
    pub loops: Option<u16>,
    /// centiseconds per frame (first frame's)
    pub delay_cs: Option<u16>,
    pub spec: Option<String>,
}

/// Walk the GIF's blocks.
pub fn info(b: &[u8]) -> Option<Info> {
    if b.len() < 13 || !(b.starts_with(b"GIF89a") || b.starts_with(b"GIF87a")) {
        return None;
    }
    let w = u16::from_le_bytes([b[6], b[7]]) as u32;
    let h = u16::from_le_bytes([b[8], b[9]]) as u32;
    let flags = b[10];
    let mut at = 13;
    if flags & 0x80 != 0 {
        at += 3 * (1 << ((flags & 7) + 1));
    }
    let sub_blocks = |mut at: usize| -> Option<(Vec<Vec<u8>>, usize)> {
        let mut out = vec![];
        loop {
            let n = *b.get(at)? as usize;
            at += 1;
            if n == 0 {
                return Some((out, at));
            }
            out.push(b.get(at..at + n)?.to_vec());
            at += n;
        }
    };
    let mut info = Info { w, h, frames: 0, loops: None, delay_cs: None, spec: None };
    loop {
        match *b.get(at)? {
            0x3B => return Some(info),
            0x21 => {
                let label = *b.get(at + 1)?;
                let (blocks, next) = sub_blocks(at + 2)?;
                if label == 0xFF && blocks.first().map(|x| x.as_slice()) == Some(b"NETSCAPE2.0") {
                    if let Some(d) = blocks.get(1).filter(|d| d.len() >= 3) {
                        info.loops = Some(u16::from_le_bytes([d[1], d[2]]));
                    }
                } else if label == 0xFF && blocks.first().map(|x| x.as_slice()) == Some(APP_ID.as_slice()) {
                    info.spec = String::from_utf8(blocks[1..].concat()).ok();
                } else if label == 0xF9 && info.delay_cs.is_none() {
                    if let Some(d) = blocks.first().filter(|d| d.len() >= 3) {
                        info.delay_cs = Some(u16::from_le_bytes([d[1], d[2]]));
                    }
                }
                at = next;
            }
            0x2C => {
                let lflags = *b.get(at + 9)?;
                at += 10;
                if lflags & 0x80 != 0 {
                    at += 3 * (1 << ((lflags & 7) + 1));
                }
                at += 1; // LZW minimum code size
                let (_, next) = sub_blocks(at)?;
                at = next;
                info.frames += 1;
            }
            _ => return None,
        }
    }
}

/// The spec a GIF made here carries.
pub fn spec_from_gif(b: &[u8]) -> Option<String> {
    info(b)?.spec
}

/// Decode a PNG into RGBA (frames from pdftoppm / Chrome).
pub fn decode_png(bytes: &[u8]) -> Result<Rgba, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("PNG 太大")?];
    let fi = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (fi.width, fi.height);
    let data = &buf[..fi.buffer_size()];
    let px: Vec<u8> = match fi.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("PNG 調色盤沒有展開".into()),
    };
    Ok(Rgba { w, h, px })
}
