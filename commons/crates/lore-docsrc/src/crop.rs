//! Page renders (cached) and PNG crops, plus region operations on assets:
//! re-crop a sub-rectangle, split, white-mask, and turn semantic module
//! boxes into region assets.

use crate::boxer::ModuleBox;
use crate::error::{Error, Result};
use crate::model::{Asset, AssetCatalog, AssetKind, BBox};
use crate::pdf::poppler::Poppler;
use image::{GenericImageView, Rgba, RgbaImage};
use std::path::{Path, PathBuf};

/// Renders pages of one PDF into a cache folder and cuts crops out of them.
#[derive(Debug, Clone)]
pub struct Renderer {
    pub poppler: Poppler,
    pub pdf: PathBuf,
    /// `<cache>/pages/page-003@200.png` (and `…@100g.png` for gray renders).
    pub cache_dir: PathBuf,
}

impl Renderer {
    pub fn new(poppler: Poppler, pdf: &Path, cache_dir: &Path) -> Renderer {
        Renderer {
            poppler,
            pdf: pdf.to_path_buf(),
            cache_dir: cache_dir.to_path_buf(),
        }
    }

    /// Path of a page render relative to the cache folder.
    pub fn page_rel(page: u32, dpi: u32, gray: bool) -> String {
        format!(
            "pages/page-{page:03}@{dpi}{}.png",
            if gray { "g" } else { "" }
        )
    }

    /// Render a page once; later calls reuse the file.
    pub fn page_png(&self, page: u32, dpi: u32, gray: bool) -> Result<PathBuf> {
        let rel = Self::page_rel(page, dpi, gray);
        let path = self.cache_dir.join(&rel);
        if path.is_file() {
            return Ok(path);
        }
        std::fs::create_dir_all(path.parent().unwrap())?;
        let prefix = path.with_extension("");
        self.poppler
            .render_page(&self.pdf, page, dpi, gray, &prefix)
    }

    /// Crop `bbox` (points) of a page at `dpi`, painting `masks` white; writes `out`.
    /// Returns the pixel size.
    pub fn crop_png(
        &self,
        page: u32,
        bbox: &BBox,
        dpi: u32,
        masks: &[BBox],
        out: &Path,
    ) -> Result<(u32, u32)> {
        let src = image::open(self.page_png(page, dpi, false)?)?;
        let img = crop_image(&src.to_rgba8(), bbox, dpi, masks);
        if let Some(d) = out.parent() {
            std::fs::create_dir_all(d)?;
        }
        img.save(out)?;
        Ok(img.dimensions())
    }

    /// Render an asset's region (with its masks) to `<cache>/crops/<id>@<dpi>.png`.
    pub fn render_asset(&self, a: &Asset, dpi: u32) -> Result<PathBuf> {
        let (Some(page), Some(bbox)) = (a.page, a.bbox) else {
            return Err(Error::NoRegion(a.id.clone()));
        };
        let out = self.cache_dir.join(format!("crops/{}@{dpi}.png", a.id));
        self.crop_png(page, &bbox, dpi, &a.masks, &out)?;
        Ok(out)
    }
}

/// Cut `frac` (`[x0, y0, x1, y1]` as fractions of the page, top-left origin) out of a page
/// PNG, `pad` pixels around it; returns the PNG of the crop.
pub fn crop_png_fraction(png: &[u8], frac: [f64; 4], pad: u32) -> Result<Vec<u8>> {
    let page = image::load_from_memory(png)?.to_rgba8();
    let (w, h) = page.dimensions();
    let px = |f: f64, n: u32| (f.clamp(0.0, 1.0) * n as f64).round() as i64;
    let x0 = (px(frac[0].min(frac[2]), w) - pad as i64).clamp(0, w as i64 - 1) as u32;
    let y0 = (px(frac[1].min(frac[3]), h) - pad as i64).clamp(0, h as i64 - 1) as u32;
    let x1 = (px(frac[0].max(frac[2]), w) + pad as i64).clamp(x0 as i64 + 1, w as i64) as u32;
    let y1 = (px(frac[1].max(frac[3]), h) + pad as i64).clamp(y0 as i64 + 1, h as i64) as u32;
    let img = page.view(x0, y0, x1 - x0, y1 - y0).to_image();
    let mut buf = std::io::Cursor::new(vec![]);
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

/// Cut a box (points) out of a page image rendered at `dpi`; masks are painted white.
pub fn crop_image(page: &RgbaImage, bbox: &BBox, dpi: u32, masks: &[BBox]) -> RgbaImage {
    let s = dpi as f64 / 72.0;
    let (w, h) = page.dimensions();
    let x0 = ((bbox.x0 * s).floor().max(0.0) as u32).min(w.saturating_sub(1));
    let y0 = ((bbox.y0 * s).floor().max(0.0) as u32).min(h.saturating_sub(1));
    let x1 = ((bbox.x1 * s).ceil() as u32).clamp(x0 + 1, w);
    let y1 = ((bbox.y1 * s).ceil() as u32).clamp(y0 + 1, h);
    let mut img = page.view(x0, y0, x1 - x0, y1 - y0).to_image();
    for m in masks {
        let mx0 = ((m.x0 * s).floor() as i64 - x0 as i64).max(0) as u32;
        let my0 = ((m.y0 * s).floor() as i64 - y0 as i64).max(0) as u32;
        let mx1 = ((m.x1 * s).ceil() as i64 - x0 as i64).clamp(0, img.width() as i64) as u32;
        let my1 = ((m.y1 * s).ceil() as i64 - y0 as i64).clamp(0, img.height() as i64) as u32;
        for y in my0..my1 {
            for x in mx0..mx1 {
                img.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
    }
    img
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Cut with vertical lines (pieces left → right).
    Vertical,
    /// Cut with horizontal lines (pieces top → bottom).
    Horizontal,
}

fn child(cat: &AssetCatalog, parent: &Asset, suffix: &str, bbox: BBox) -> Asset {
    let mut a = Asset::new(
        cat.free_id(&format!("{}.{suffix}", parent.id)),
        AssetKind::Region,
    );
    a.parent = Some(parent.id.clone());
    a.page = parent.page;
    a.bbox = Some(bbox.rounded());
    a.section = parent.section.clone();
    a.source_ref = parent.source_ref.clone();
    a.masks = parent
        .masks
        .iter()
        .filter_map(|m| m.intersect(&bbox))
        .collect();
    a.confidence = parent.confidence;
    a
}

fn region(cat: &AssetCatalog, id: &str) -> Result<(Asset, BBox)> {
    let a = cat.require(id)?.clone();
    let b = a.bbox.ok_or_else(|| Error::NoRegion(id.into()))?;
    Ok((a, b))
}

/// Re-crop: a sub-rectangle given relative to the asset's top-left (points).
/// Adds a `Region` asset and returns its id.
pub fn recrop(cat: &mut AssetCatalog, id: &str, rel: BBox) -> Result<String> {
    let (a, b) = region(cat, id)?;
    let r = child(cat, &a, "r", b.sub(&rel));
    let rid = r.id.clone();
    cat.assets.push(r);
    Ok(rid)
}

/// Split an asset at offsets (points, relative to its top-left along the axis).
/// Adds one `Region` per piece, returns their ids in order.
pub fn split(cat: &mut AssetCatalog, id: &str, axis: Axis, cuts: &[f64]) -> Result<Vec<String>> {
    let (a, b) = region(cat, id)?;
    let len = match axis {
        Axis::Vertical => b.width(),
        Axis::Horizontal => b.height(),
    };
    let mut edges: Vec<f64> = cuts
        .iter()
        .copied()
        .filter(|c| *c > 0.0 && *c < len)
        .collect();
    edges.sort_by(|x, y| x.partial_cmp(y).unwrap());
    edges.insert(0, 0.0);
    edges.push(len);
    let mut ids = vec![];
    for (k, w) in edges.windows(2).enumerate() {
        let rel = match axis {
            Axis::Vertical => BBox::new(w[0], 0.0, w[1], b.height()),
            Axis::Horizontal => BBox::new(0.0, w[0], b.width(), w[1]),
        };
        let r = child(cat, &a, &format!("p{}", k + 1), b.sub(&rel));
        ids.push(r.id.clone());
        cat.assets.push(r);
    }
    Ok(ids)
}

/// Split into an even `rows × cols` grid (e.g. a panel of example images).
pub fn split_grid(
    cat: &mut AssetCatalog,
    id: &str,
    rows: usize,
    cols: usize,
) -> Result<Vec<String>> {
    let (a, b) = region(cat, id)?;
    let mut ids = vec![];
    for r in 0..rows.max(1) {
        for c in 0..cols.max(1) {
            let fx = |i: usize, n: usize| i as f64 / n.max(1) as f64;
            let bb = b.sub_fraction(fx(c, cols), fx(r, rows), fx(c + 1, cols), fx(r + 1, rows));
            let x = child(cat, &a, &format!("g{}-{}", r + 1, c + 1), bb);
            ids.push(x.id.clone());
            cat.assets.push(x);
        }
    }
    Ok(ids)
}

/// White-mask regions of an asset (relative to its top-left, points).
pub fn mask(cat: &mut AssetCatalog, id: &str, rel: &[BBox]) -> Result<()> {
    let (_, b) = region(cat, id)?;
    let a = cat.asset_mut(id).unwrap();
    a.masks.extend(rel.iter().map(|r| b.sub(r).rounded()));
    Ok(())
}

/// Turn module boxes (pixels of an image of the asset that is `img_w × img_h`)
/// into `Region` assets labelled with the module names. Returns their ids.
pub fn add_module_regions(
    cat: &mut AssetCatalog,
    id: &str,
    boxes: &[ModuleBox],
    img_w: u32,
    img_h: u32,
) -> Result<Vec<String>> {
    let (a, b) = region(cat, id)?;
    let (sx, sy) = (
        b.width() / img_w.max(1) as f64,
        b.height() / img_h.max(1) as f64,
    );
    let mut ids = vec![];
    for (k, m) in boxes.iter().enumerate() {
        let rel = BBox::new(
            m.bbox[0] * sx,
            m.bbox[1] * sy,
            m.bbox[2] * sx,
            m.bbox[3] * sy,
        );
        let mut r = child(cat, &a, &format!("m{}", k + 1), b.sub(&rel));
        r.label = Some(m.label.clone());
        r.text = m.role.clone();
        ids.push(r.id.clone());
        cat.assets.push(r);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_by_page_fraction() {
        let page = RgbaImage::from_pixel(200, 100, Rgba([255, 255, 255, 255]));
        let mut buf = std::io::Cursor::new(vec![]);
        page.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let out = crop_png_fraction(&buf.into_inner(), [0.25, 0.5, 0.75, 0.6], 2).unwrap();
        let img = image::load_from_memory(&out).unwrap();
        assert_eq!(img.dimensions(), (104, 14));
    }

    fn cat() -> AssetCatalog {
        let mut c = AssetCatalog::default();
        let mut f = Asset::new("fig1", AssetKind::Figure);
        f.page = Some(2);
        f.bbox = Some(BBox::new(100.0, 100.0, 300.0, 200.0));
        c.assets.push(f);
        c.assets.push(Asset::new("fig2", AssetKind::Figure));
        c
    }

    #[test]
    fn recrop_split_mask() {
        let mut c = cat();
        let r = recrop(&mut c, "fig1", BBox::new(0.0, 0.0, 100.0, 50.0)).unwrap();
        assert_eq!(r, "fig1.r");
        assert_eq!(
            c.asset(&r).unwrap().bbox,
            Some(BBox::new(100.0, 100.0, 200.0, 150.0))
        );
        assert_eq!(
            recrop(&mut c, "fig1", BBox::new(0.0, 0.0, 10.0, 10.0)).unwrap(),
            "fig1.r-2"
        );
        let parts = split(&mut c, "fig1", Axis::Vertical, &[50.0, 150.0]).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(
            c.asset(&parts[2]).unwrap().bbox,
            Some(BBox::new(250.0, 100.0, 300.0, 200.0))
        );
        let g = split_grid(&mut c, "fig1", 2, 2).unwrap();
        assert_eq!(
            c.asset(&g[3]).unwrap().bbox,
            Some(BBox::new(200.0, 150.0, 300.0, 200.0))
        );
        mask(&mut c, "fig1", &[BBox::new(10.0, 10.0, 20.0, 20.0)]).unwrap();
        assert_eq!(
            c.asset("fig1").unwrap().masks,
            vec![BBox::new(110.0, 110.0, 120.0, 120.0)]
        );
        // children made after the mask inherit the part of it they cover
        let r = recrop(&mut c, "fig1", BBox::new(0.0, 0.0, 15.0, 15.0)).unwrap();
        assert_eq!(
            c.asset(&r).unwrap().masks,
            vec![BBox::new(110.0, 110.0, 115.0, 115.0)]
        );
        assert!(matches!(
            recrop(&mut c, "fig2", BBox::new(0.0, 0.0, 1.0, 1.0)),
            Err(Error::NoRegion(_))
        ));
        assert!(matches!(
            recrop(&mut c, "nope", BBox::new(0.0, 0.0, 1.0, 1.0)),
            Err(Error::NoAsset(_))
        ));
    }

    #[test]
    fn module_boxes_to_regions() {
        let mut c = cat();
        let boxes = vec![ModuleBox {
            label: "encoder".into(),
            role: Some("model".into()),
            bbox: [0.0, 0.0, 400.0, 100.0],
        }];
        // image is 800×200 px for a 200×100 pt box → 0.25 pt/px horizontally, 0.5 vertically
        let ids = add_module_regions(&mut c, "fig1", &boxes, 800, 200).unwrap();
        let r = c.asset(&ids[0]).unwrap();
        assert_eq!(r.bbox, Some(BBox::new(100.0, 100.0, 200.0, 150.0)));
        assert_eq!(r.label.as_deref(), Some("encoder"));
        assert_eq!(r.kind, AssetKind::Region);
    }

    #[test]
    fn crop_with_mask() {
        // 144 dpi → 2 px per point; page 100×100 pt = 200×200 px, black
        let page = RgbaImage::from_pixel(200, 200, Rgba([0, 0, 0, 255]));
        let img = crop_image(
            &page,
            &BBox::new(10.0, 10.0, 60.0, 35.0),
            144,
            &[BBox::new(10.0, 10.0, 20.0, 20.0)],
        );
        assert_eq!(img.dimensions(), (100, 50));
        assert_eq!(img.get_pixel(5, 5), &Rgba([255, 255, 255, 255]));
        assert_eq!(img.get_pixel(25, 25), &Rgba([0, 0, 0, 255]));
    }
}
