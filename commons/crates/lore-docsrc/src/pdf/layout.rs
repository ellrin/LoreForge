//! Text geometry from `pdftotext -bbox-layout`, column layout, and the
//! "ink" (non-white pixels) of a page render.

use crate::error::{Error, Result};
use crate::model::BBox;
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub bbox: BBox,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub bbox: BBox,
    pub words: Vec<Word>,
    /// Block index within the page.
    pub block: usize,
}

impl Line {
    pub fn text(&self) -> String {
        self.words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
    pub fn height(&self) -> f64 {
        self.bbox.height()
    }
    /// Largest horizontal gap between consecutive words.
    pub fn max_gap(&self) -> f64 {
        self.words
            .windows(2)
            .map(|w| w[1].bbox.x0 - w[0].bbox.x1)
            .fold(0.0, f64::max)
    }
    /// Share of words that look like prose (≥ 2 letters, mostly alphabetic).
    pub fn wordy_ratio(&self) -> f64 {
        if self.words.is_empty() {
            return 0.0;
        }
        let n = self
            .words
            .iter()
            .filter(|w| {
                let letters = w.text.chars().filter(|c| c.is_alphabetic()).count();
                letters >= 2 && letters * 10 >= w.text.chars().count() * 7
            })
            .count();
        n as f64 / self.words.len() as f64
    }
    /// Text with letter-spaced headings collapsed (`A B S T R A C T` → `ABSTRACT`).
    /// Small caps split by pdftotext (`R EFERENCES`, `IV. R ESULTS`) are rejoined.
    pub fn squeezed(&self) -> String {
        if self.words.len() >= 4 && self.words.iter().all(|w| w.text.chars().count() == 1) {
            self.words.iter().map(|w| w.text.as_str()).collect()
        } else {
            fix_small_caps(&self.text())
        }
    }
}

static SMALL_CAPS_HYPHEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Z]) -([A-Z])").unwrap());
static SMALL_CAPS_PLURAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Z]{2,}) S\b").unwrap());
static SMALL_CAPS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Z]) ([A-Z]{2,})").unwrap());

/// `N UMBERS OF S UBJECTS` → `NUMBERS OF SUBJECTS`, only when the text is mostly upper case.
pub fn fix_small_caps(s: &str) -> String {
    let letters: Vec<char> = s.chars().filter(|c| c.is_alphabetic()).collect();
    let upper = letters.iter().filter(|c| c.is_uppercase()).count();
    if letters.is_empty() || upper * 10 < letters.len() * 6 {
        return s.to_string();
    }
    let t = SMALL_CAPS.replace_all(s, "$1$2");
    let t = SMALL_CAPS_HYPHEN.replace_all(&t, "$1-$2");
    SMALL_CAPS_PLURAL.replace_all(&t, "${1}S").into_owned()
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextPage {
    /// 1-based.
    pub number: u32,
    pub width: f64,
    pub height: f64,
    pub lines: Vec<Line>,
    /// Block bounding boxes (index = `Line::block`).
    pub blocks: Vec<BBox>,
}

static ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(\w+)="([^"]*)""#).unwrap());
static WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<word ([^>]*)>(.*?)</word>"#).unwrap());

fn attrs(s: &str) -> std::collections::HashMap<String, f64> {
    ATTR.captures_iter(s)
        .filter_map(|c| Some((c[1].to_string(), c[2].parse::<f64>().ok()?)))
        .collect()
}

fn bbox_of(s: &str) -> Option<BBox> {
    let a = attrs(s);
    Some(BBox::new(
        *a.get("xMin")?,
        *a.get("yMin")?,
        *a.get("xMax")?,
        *a.get("yMax")?,
    ))
}

pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find(';').filter(|j| *j <= 8) else {
            out.push('&');
            rest = after;
            continue;
        };
        let ent = &after[..j];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16)
                .ok()
                .and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &after[j + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Parse the XHTML that `pdftotext -bbox-layout` writes.
pub fn parse_bbox_layout(xml: &str) -> Result<Vec<TextPage>> {
    let mut pages: Vec<TextPage> = vec![];
    for raw in xml.lines() {
        let l = raw.trim();
        if let Some(rest) = l.strip_prefix("<page ") {
            let a = attrs(rest);
            let (Some(w), Some(h)) = (a.get("width"), a.get("height")) else {
                return Err(Error::parse(
                    "pdftotext -bbox-layout",
                    "page 沒有 width/height",
                ));
            };
            pages.push(TextPage {
                number: pages.len() as u32 + 1,
                width: *w,
                height: *h,
                lines: vec![],
                blocks: vec![],
            });
        } else if let Some(rest) = l.strip_prefix("<block ") {
            if let (Some(p), Some(b)) = (pages.last_mut(), bbox_of(rest)) {
                p.blocks.push(b);
            }
        } else if let Some(rest) = l.strip_prefix("<line ") {
            if let (Some(p), Some(b)) = (pages.last_mut(), bbox_of(rest)) {
                let block = p.blocks.len().saturating_sub(1);
                p.lines.push(Line {
                    bbox: b,
                    words: vec![],
                    block,
                });
            }
        } else if l.starts_with("<word ") {
            for c in WORD.captures_iter(l) {
                if let (Some(p), Some(b)) = (pages.last_mut(), bbox_of(&c[1]))
                    && let Some(line) = p.lines.last_mut()
                {
                    line.words.push(Word {
                        bbox: b,
                        text: unescape(&c[2]),
                    });
                }
            }
        }
    }
    for p in pages.iter_mut() {
        p.lines.retain(|l| !l.words.is_empty());
    }
    Ok(pages)
}

/// Document-wide column geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Columns {
    /// (x0, x1) per column, left to right.
    pub cols: Vec<(f64, f64)>,
    /// Median height of prose lines (≈ body font size × 1.2).
    pub line_height: f64,
}

fn percentile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * q).round() as usize]
}

impl Columns {
    pub fn detect(pages: &[TextPage]) -> Columns {
        let width = pages.first().map(|p| p.width).unwrap_or(612.0);
        let mid = width / 2.0;
        let long: Vec<&Line> = pages
            .iter()
            .flat_map(|p| p.lines.iter())
            .filter(|l| l.words.len() >= 3 && l.bbox.width() > 0.25 * width)
            .collect();
        let mut heights: Vec<f64> = long
            .iter()
            .filter(|l| l.words.len() >= 5)
            .map(|l| l.height())
            .collect();
        let line_height = if heights.is_empty() {
            10.0
        } else {
            percentile(&mut heights, 0.5)
        };
        let left: Vec<&&Line> = long.iter().filter(|l| l.bbox.x1 < mid + 2.0).collect();
        let right: Vec<&&Line> = long.iter().filter(|l| l.bbox.x0 > mid - 2.0).collect();
        let n = long.len().max(1) as f64;
        if left.len() as f64 > 0.25 * n && right.len() as f64 > 0.25 * n {
            let mut lx0: Vec<f64> = left.iter().map(|l| l.bbox.x0).collect();
            let mut lx1: Vec<f64> = left.iter().map(|l| l.bbox.x1).collect();
            let mut rx0: Vec<f64> = right.iter().map(|l| l.bbox.x0).collect();
            let mut rx1: Vec<f64> = right.iter().map(|l| l.bbox.x1).collect();
            Columns {
                cols: vec![
                    (percentile(&mut lx0, 0.1), percentile(&mut lx1, 0.9)),
                    (percentile(&mut rx0, 0.1), percentile(&mut rx1, 0.9)),
                ],
                line_height,
            }
        } else {
            let mut x0: Vec<f64> = long.iter().map(|l| l.bbox.x0).collect();
            let mut x1: Vec<f64> = long.iter().map(|l| l.bbox.x1).collect();
            let (a, b) = if long.is_empty() {
                (0.1 * width, 0.9 * width)
            } else {
                (percentile(&mut x0, 0.05), percentile(&mut x1, 0.95))
            };
            Columns {
                cols: vec![(a, b)],
                line_height,
            }
        }
    }

    pub fn two(&self) -> bool {
        self.cols.len() == 2
    }

    /// Middle of the gutter (two columns).
    pub fn gutter_mid(&self) -> Option<f64> {
        self.two().then(|| (self.cols[0].1 + self.cols[1].0) / 2.0)
    }

    pub fn full(&self) -> (f64, f64) {
        (self.cols[0].0, self.cols.last().unwrap().1)
    }

    /// Does this box cross the gutter (a full-width element)?
    pub fn spans(&self, b: &BBox) -> bool {
        match self.gutter_mid() {
            Some(g) => b.x0 < g - 6.0 && b.x1 > g + 6.0,
            None => true,
        }
    }

    /// Column index for a box (by its centre); `None` = spans both.
    pub fn column_of(&self, b: &BBox) -> Option<usize> {
        if !self.two() {
            return Some(0);
        }
        if self.spans(b) {
            return None;
        }
        Some(if b.center_x() < self.gutter_mid().unwrap() {
            0
        } else {
            1
        })
    }

    /// Horizontal span (x0, x1) of a box's column (or the full width).
    pub fn span_of(&self, b: &BBox) -> (f64, f64) {
        match self.column_of(b) {
            Some(i) => self.cols[i],
            None => self.full(),
        }
    }
}

/// Dark pixels of a grayscale page render, with a summed-area table for fast counts.
#[derive(Debug, Clone)]
pub struct Ink {
    pub w: usize,
    pub h: usize,
    /// Pixels per PDF point.
    pub scale: f64,
    sat: Vec<u32>,
}

impl Ink {
    /// Pixels darker than `threshold` (0–255) count as ink.
    pub fn from_luma(w: usize, h: usize, px: &[u8], scale: f64, threshold: u8) -> Ink {
        let mut sat = vec![0u32; (w + 1) * (h + 1)];
        for y in 0..h {
            let mut row = 0u32;
            for x in 0..w {
                row += (px[y * w + x] < threshold) as u32;
                sat[(y + 1) * (w + 1) + x + 1] = sat[y * (w + 1) + x + 1] + row;
            }
        }
        Ink { w, h, scale, sat }
    }

    pub fn from_png(path: &Path, scale: f64) -> Result<Ink> {
        let img = image::open(path)?.to_luma8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        Ok(Ink::from_luma(w, h, img.as_raw(), scale, 200))
    }

    fn px(&self, v: f64, max: usize) -> usize {
        ((v * self.scale).round().max(0.0) as usize).min(max)
    }

    fn count_px(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> u32 {
        if x1 <= x0 || y1 <= y0 {
            return 0;
        }
        let w = self.w + 1;
        self.sat[y1 * w + x1] + self.sat[y0 * w + x0]
            - self.sat[y0 * w + x1]
            - self.sat[y1 * w + x0]
    }

    /// Ink pixels inside a box (points).
    pub fn count(&self, b: &BBox) -> u32 {
        self.count_px(
            self.px(b.x0, self.w),
            self.px(b.y0, self.h),
            self.px(b.x1, self.w),
            self.px(b.y1, self.h),
        )
    }

    /// Tight box around the ink inside `region` (rows/columns with fewer than
    /// `min_px` dark pixels are ignored).
    pub fn bbox_in(&self, region: &BBox, min_px: u32) -> Option<BBox> {
        let (x0, y0, x1, y1) = (
            self.px(region.x0, self.w),
            self.px(region.y0, self.h),
            self.px(region.x1, self.w),
            self.px(region.y1, self.h),
        );
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let rows: Vec<usize> = (y0..y1)
            .filter(|&y| self.count_px(x0, y, x1, y + 1) >= min_px)
            .collect();
        let cols: Vec<usize> = (x0..x1)
            .filter(|&x| self.count_px(x, y0, x + 1, y1) >= min_px)
            .collect();
        let (ry0, ry1) = (*rows.first()?, *rows.last()? + 1);
        let (cx0, cx1) = (*cols.first()?, *cols.last()? + 1);
        let s = self.scale;
        Some(BBox::new(
            cx0 as f64 / s,
            ry0 as f64 / s,
            cx1 as f64 / s,
            ry1 as f64 / s,
        ))
    }

    /// Horizontal bands of ink inside `region`: runs of inked rows, merged
    /// across gaps smaller than `merge_gap` points. Returns (y0, y1) in points.
    pub fn bands(&self, region: &BBox, merge_gap: f64) -> Vec<(f64, f64)> {
        let (x0, y0, x1, y1) = (
            self.px(region.x0, self.w),
            self.px(region.y0, self.h),
            self.px(region.x1, self.w),
            self.px(region.y1, self.h),
        );
        let s = self.scale;
        let gap_px = (merge_gap * s).round() as usize;
        let mut out: Vec<(usize, usize)> = vec![];
        let mut y = y0;
        while y < y1 {
            if self.count_px(x0, y, x1, y + 1) > 0 {
                let start = y;
                while y < y1 && self.count_px(x0, y, x1, y + 1) > 0 {
                    y += 1;
                }
                match out.last_mut() {
                    Some(last) if start - last.1 <= gap_px => last.1 = y,
                    _ => out.push((start, y)),
                }
            } else {
                y += 1;
            }
        }
        out.into_iter()
            .map(|(a, b)| (a as f64 / s, b as f64 / s))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const SAMPLE: &str = r#"<doc>
  <page width="600.000000" height="800.000000">
    <flow>
      <block xMin="50" yMin="100" xMax="290" yMax="122">
        <line xMin="50" yMin="100" xMax="290" yMax="110">
          <word xMin="50" yMin="100" xMax="80" yMax="110">Fig.</word>
          <word xMin="83" yMin="100" xMax="90" yMax="110">1.</word>
          <word xMin="93" yMin="100" xMax="290" yMax="110">A&amp;B &lt;x&gt;</word>
        </line>
      </block>
    </flow>
  </page>
  <page width="600.000000" height="800.000000">
  </page>
</doc>"#;

    #[test]
    fn parses_pages_lines_words() {
        let p = parse_bbox_layout(SAMPLE).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].lines.len(), 1);
        assert_eq!(p[0].lines[0].text(), "Fig. 1. A&B <x>");
        assert_eq!(p[0].lines[0].max_gap(), 3.0);
        assert_eq!(p[0].blocks.len(), 1);
        assert_eq!(p[1].number, 2);
    }

    #[test]
    fn ink_boxes_and_bands() {
        // 100×100 px at 1 px/pt; ink at rows 10..20 and 23..25 and 60..70, cols 30..50
        let mut px = vec![255u8; 100 * 100];
        for y in (10..20).chain(23..25).chain(60..70) {
            for x in 30..50 {
                px[y * 100 + x] = 0;
            }
        }
        let ink = Ink::from_luma(100, 100, &px, 1.0, 200);
        assert_eq!(ink.count(&BBox::new(0.0, 0.0, 100.0, 100.0)), 20 * 22);
        assert_eq!(
            ink.bbox_in(&BBox::new(0.0, 0.0, 100.0, 50.0), 1),
            Some(BBox::new(30.0, 10.0, 50.0, 25.0))
        );
        assert_eq!(
            ink.bands(&BBox::new(0.0, 0.0, 100.0, 100.0), 4.0),
            vec![(10.0, 25.0), (60.0, 70.0)]
        );
        assert_eq!(ink.bands(&BBox::new(0.0, 0.0, 100.0, 100.0), 1.0).len(), 3);
    }

    #[test]
    fn letter_spaced_heading() {
        let w = |t: &str, x: f64| Word {
            bbox: BBox::new(x, 0.0, x + 5.0, 8.0),
            text: t.into(),
        };
        let l = Line {
            bbox: BBox::new(0.0, 0.0, 60.0, 8.0),
            words: "ABSTRACT"
                .chars()
                .enumerate()
                .map(|(i, c)| w(&c.to_string(), i as f64 * 7.0))
                .collect(),
            block: 0,
        };
        assert_eq!(l.squeezed(), "ABSTRACT");
        assert_eq!(fix_small_caps("IV. R ESULTS"), "IV. RESULTS");
        assert_eq!(
            fix_small_caps("N UMBERS OF S UBJECTS IN E ACH G ROUP"),
            "NUMBERS OF SUBJECTS IN EACH GROUP"
        );
        assert_eq!(fix_small_caps("A Study of X"), "A Study of X");
        assert_eq!(
            fix_small_caps("D ETAILS OF S UB -D ATASETS OF CSCL S"),
            "DETAILS OF SUB-DATASETS OF CSCLS"
        );
    }
}
