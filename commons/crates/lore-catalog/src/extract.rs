//! Small facts read from inside a file, cached per content in the cache tier
//! (`<cache>/LoreForge/shelf/extract/<sha256>.json`, wipeable): a PDF's page
//! count and the text of its first [`TEXT_PAGES`] pages (lopdf, size-capped,
//! panic-guarded), an image's pixel size (header only). Filled by the scan
//! for new contents; read by the categories and the dataset ↔ paper links.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const TEXT_PAGES: u32 = 2;
/// PDFs larger than this are not parsed for text.
pub const TEXT_MAX_BYTES: u64 = 60 << 20;
/// Characters of text kept.
pub const TEXT_MAX_CHARS: usize = 20_000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Extract {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<u32>,
    /// Text of the first pages (may be empty for a scan).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

impl Extract {
    pub fn dir(cache_dir: &Path) -> PathBuf {
        cache_dir.join("extract")
    }

    pub fn load(dir: &Path, sha: &str) -> Option<Extract> {
        serde_json::from_slice(&std::fs::read(dir.join(format!("{sha}.json"))).ok()?).ok()
    }

    pub fn save(&self, dir: &Path, sha: &str) -> Result<(), crate::CatalogError> {
        let p = dir.join(format!("{sha}.json"));
        let b = serde_json::to_vec(self).map_err(|e| crate::CatalogError::io(&p, std::io::Error::other(e)))?;
        crate::store::write_atomic(&p, &b)
    }
}

/// Facts of one file by kind (`pdf`, `image`; others give an empty extract).
pub fn extract(path: &Path, kind: &str, size: u64) -> Extract {
    match kind {
        "pdf" => {
            let (pages, text) = pdf_text(path, size);
            Extract { pages, text, ..Extract::default() }
        }
        "image" => match image_size(path) {
            Some((w, h)) => Extract { width: Some(w), height: Some(h), ..Extract::default() },
            None => Extract::default(),
        },
        _ => Extract::default(),
    }
}

/// (page count, text of the first pages).
pub fn pdf_text(path: &Path, size: u64) -> (Option<u32>, Option<String>) {
    if size > TEXT_MAX_BYTES {
        return (None, None);
    }
    let p = path.to_path_buf();
    let r = std::panic::catch_unwind(move || {
        let doc = lopdf::Document::load(&p).ok()?;
        let n = doc.get_pages().len() as u32;
        let pages: Vec<u32> = (1..=TEXT_PAGES.min(n)).collect();
        let text = doc.extract_text(&pages).unwrap_or_default();
        let text: String = text.chars().take(TEXT_MAX_CHARS).collect();
        Some((n, text))
    });
    match r {
        Ok(Some((n, t))) => (Some(n), Some(t)),
        _ => (None, None),
    }
}

/// Pixel size from the header of a PNG, JPEG, GIF, BMP or WebP (reads at most 256 KiB).
pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    let mut b = Vec::new();
    std::fs::File::open(path).ok()?.take(256 << 10).read_to_end(&mut b).ok()?;
    image_size_bytes(&b)
}

pub fn image_size_bytes(b: &[u8]) -> Option<(u32, u32)> {
    let be32 = |i: usize| -> Option<u32> { Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?)) };
    let le16 = |i: usize| -> Option<u32> { Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32) };
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(16)?, be32(20)?));
    }
    if b.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if b.starts_with(b"BM") {
        let w = i32::from_le_bytes(b.get(18..22)?.try_into().ok()?);
        let h = i32::from_le_bytes(b.get(22..26)?.try_into().ok()?);
        return Some((w.unsigned_abs(), h.unsigned_abs()));
    }
    if b.len() > 30 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return match &b[12..16] {
            b"VP8X" => Some((1 + (b[24] as u32 | (b[25] as u32) << 8 | (b[26] as u32) << 16), 1 + (b[27] as u32 | (b[28] as u32) << 8 | (b[29] as u32) << 16))),
            b"VP8 " => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            b"VP8L" => {
                let v = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
                Some(((v & 0x3fff) + 1, ((v >> 14) & 0x3fff) + 1))
            }
            _ => None,
        };
    }
    if b.starts_with(&[0xff, 0xd8]) {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xff {
                i += 1;
                continue;
            }
            let m = b[i + 1];
            if m == 0xd8 || m == 0x01 || (0xd0..=0xd7).contains(&m) || m == 0xff {
                i += if m == 0xff { 1 } else { 2 };
                continue;
            }
            let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
            if matches!(m, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
                let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}
