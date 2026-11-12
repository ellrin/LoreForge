//! A dataset's inside, read lazily with `symlink_metadata` (nothing hashed,
//! nothing opened except image headers for the dimension sample): the member
//! list (paged), a summary (main extension, sample image sizes, label /
//! metadata files at the top, first-level folders such as train/test or
//! class folders), and the 2×2 mosaic thumbnail.

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Extensions QuickLook can usually draw.
pub const IMAGE_EXTS: [&str; 10] = ["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "heic", "dcm"];
/// Top-level files listed as labels / metadata.
pub const META_EXTS: [&str; 8] = ["csv", "tsv", "json", "txt", "md", "xlsx", "yaml", "yml"];
pub const MAX_SUBFOLDERS: usize = 20;
pub const DIM_SAMPLES: usize = 5;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Member {
    pub name: String,
    /// Path relative to the dataset folder.
    pub rel: String,
    pub size: u64,
}

fn ext(name: &str) -> String {
    name.rsplit_once('.').filter(|(s, _)| !s.is_empty()).map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

pub fn is_image(name: &str) -> bool {
    IMAGE_EXTS.contains(&ext(name).as_str())
}

/// Every regular file below `root`, sorted by relative path.
pub fn members(root: &Path) -> Vec<Member> {
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<Member>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(md) = std::fs::symlink_metadata(e.path()) else { continue };
        if name == ".DS_Store" || md.file_type().is_symlink() || lore_catalog::scan::is_dataless(&md) || lore_catalog::scan::is_placeholder_name(&name) {
            continue;
        }
        if md.is_dir() {
            walk(root, &e.path(), out);
        } else if md.is_file() {
            let rel = e.path().strip_prefix(root).map(|r| r.to_string_lossy().to_string()).unwrap_or_else(|_| name.clone());
            out.push(Member { name, rel, size: md.len() });
        }
    }
}

/// `n` members spread evenly over the image members.
pub fn spread_images(members: &[Member], n: usize) -> Vec<usize> {
    let imgs: Vec<usize> = members.iter().enumerate().filter(|(_, m)| is_image(&m.name)).map(|(i, _)| i).collect();
    if imgs.is_empty() {
        return vec![];
    }
    let n = n.min(imgs.len());
    (0..n).map(|k| imgs[k * imgs.len() / n]).collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub files: usize,
    pub bytes: u64,
    pub main_ext: String,
    pub main_share: f64,
    /// `[width, height]` of a few evenly spaced images.
    pub image_sizes: Vec<[u32; 2]>,
    /// Names of label / metadata files at the top level.
    pub meta_files: Vec<String>,
    /// First-level folder names (≤ 20) and their file counts.
    pub subfolders: Vec<(String, usize)>,
    pub more_subfolders: usize,
}

pub fn summary(root: &Path, members: &[Member]) -> Summary {
    let mut exts: BTreeMap<String, usize> = BTreeMap::new();
    let mut subs: BTreeMap<String, usize> = BTreeMap::new();
    let mut meta = Vec::new();
    for m in members {
        *exts.entry(ext(&m.name)).or_default() += 1;
        match m.rel.split_once('/') {
            Some((top, _)) => *subs.entry(top.to_string()).or_default() += 1,
            None if META_EXTS.contains(&ext(&m.name).as_str()) => meta.push(m.name.clone()),
            None => {}
        }
    }
    let (main_ext, n) = exts.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(e, n)| (e.clone(), *n)).unwrap_or_default();
    let image_sizes = spread_images(members, DIM_SAMPLES)
        .into_iter()
        .filter_map(|i| lore_catalog::extract::image_size(&root.join(&members[i].rel)))
        .map(|(w, h)| [w, h])
        .collect();
    let more = subs.len().saturating_sub(MAX_SUBFOLDERS);
    Summary {
        files: members.len(),
        bytes: members.iter().map(|m| m.size).sum(),
        main_share: if members.is_empty() { 0.0 } else { (n as f64 * 1000.0 / members.len() as f64).round() / 1000.0 },
        main_ext,
        image_sizes,
        meta_files: meta,
        subfolders: subs.into_iter().take(MAX_SUBFOLDERS).collect(),
        more_subfolders: more,
    }
}

/// A 2×2 mosaic (`size`×`size`, transparent background) of up to four PNGs.
pub fn mosaic(pngs: &[Vec<u8>], size: u32) -> Option<Vec<u8>> {
    let tiles: Vec<image::DynamicImage> = pngs.iter().filter_map(|b| image::load_from_memory_with_format(b, image::ImageFormat::Png).ok()).take(4).collect();
    if tiles.is_empty() {
        return None;
    }
    let half = size / 2;
    let mut canvas = image::RgbaImage::new(size, size);
    for (k, t) in tiles.iter().enumerate() {
        let small = t.thumbnail(half, half).to_rgba8();
        let (x0, y0) = ((k as u32 % 2) * half, (k as u32 / 2) * half);
        let (dx, dy) = ((half - small.width()) / 2, (half - small.height()) / 2);
        image::imageops::overlay(&mut canvas, &small, (x0 + dx) as i64, (y0 + dy) as i64);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    canvas.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// The dataset folder: the first recorded path that still exists.
pub fn root_of(paths: &[String]) -> Option<PathBuf> {
    paths.iter().map(PathBuf::from).find(|p| p.is_dir() && std::fs::symlink_metadata(p).is_ok_and(|m| !m.file_type().is_symlink()))
}
