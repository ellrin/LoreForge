//! A table's cells from the PDF's own text: the words inside a box of a page, grouped into
//! rows (by their line) and columns (by the blank gutters all rows share). The text of every
//! cell is what the PDF contains, so numbers are never somebody's reading of a picture.

use super::layout::{TextPage, Word};
use super::poppler::Poppler;
use crate::error::Result;
use std::path::Path;

/// The text pages of a PDF (`pdftotext -bbox-layout`), words with their boxes in points.
pub fn text_pages(pdf: &Path) -> Result<Vec<TextPage>> {
    let p = Poppler::discover();
    p.require()?;
    super::layout::parse_bbox_layout(&p.bbox_layout(pdf)?)
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

/// The cells of the table inside `frac` (`[x0, y0, x1, y1]`, fractions of the page):
/// `None` when the box holds no table-like text (fewer than two rows or two columns).
pub fn grid_in(page: &TextPage, frac: [f64; 4]) -> Option<Vec<Vec<String>>> {
    let (x0, y0, x1, y1) = (frac[0] * page.width, frac[1] * page.height, frac[2] * page.width, frac[3] * page.height);
    let inside = |w: &Word| {
        let (cx, cy) = ((w.bbox.x0 + w.bbox.x1) / 2.0, (w.bbox.y0 + w.bbox.y1) / 2.0);
        cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1 && !w.text.trim().is_empty()
    };
    let mut words: Vec<&Word> = page.lines.iter().flat_map(|l| l.words.iter()).filter(|w| inside(w)).collect();
    if words.len() < 4 {
        return None;
    }
    let hmed = median(words.iter().map(|w| w.bbox.y1 - w.bbox.y0).collect()).max(1.0);
    // rows: words whose centres lie on one line
    words.sort_by(|a, b| ((a.bbox.y0 + a.bbox.y1), a.bbox.x0).partial_cmp(&((b.bbox.y0 + b.bbox.y1), b.bbox.x0)).unwrap_or(std::cmp::Ordering::Equal));
    let mut rows: Vec<Vec<&Word>> = vec![];
    let mut row_y = f64::NEG_INFINITY;
    for w in words {
        let cy = (w.bbox.y0 + w.bbox.y1) / 2.0;
        if rows.is_empty() || cy - row_y > hmed * 0.6 {
            rows.push(vec![]);
            row_y = cy;
        }
        rows.last_mut().unwrap().push(w);
    }
    for r in &mut rows {
        r.sort_by(|a, b| a.bbox.x0.partial_cmp(&b.bbox.x0).unwrap_or(std::cmp::Ordering::Equal));
    }
    if rows.len() < 2 {
        return None;
    }
    // columns: the gutters (almost) no row writes in, wider than a word space
    let char_w = median(rows.iter().flatten().map(|w| (w.bbox.x1 - w.bbox.x0) / w.text.chars().count().max(1) as f64).collect()).max(0.5);
    let min_gap = char_w * 1.7;
    let step = (char_w / 4.0).max(0.25);
    let n = (((x1 - x0) / step).ceil() as usize).max(1);
    let mut cover = vec![0u32; n];
    for r in &rows {
        let mut row = vec![false; n];
        for (i, w) in r.iter().enumerate() {
            // a word and the ordinary space after it belong together
            let end = match r.get(i + 1) {
                Some(nx) if nx.bbox.x0 - w.bbox.x1 < min_gap => nx.bbox.x0,
                _ => w.bbox.x1,
            };
            let (a, b) = ((((w.bbox.x0 - x0) / step).floor().max(0.0)) as usize, (((end - x0) / step).ceil().max(0.0)) as usize);
            for c in row.iter_mut().take(b.min(n)).skip(a.min(n)) {
                *c = true;
            }
        }
        for (i, c) in row.iter().enumerate() {
            cover[i] += *c as u32;
        }
    }
    let few = (rows.len() as f64 * 0.12).floor() as u32;
    let mut cols: Vec<(f64, f64)> = vec![];
    let mut start: Option<usize> = None;
    let mut blank = 0usize;
    for i in 0..=n {
        let used = i < n && cover[i] > few;
        match (used, start) {
            (true, None) => {
                start = Some(i);
                blank = 0;
            }
            (true, Some(_)) => blank = 0,
            (false, Some(s)) => {
                blank += 1;
                if blank as f64 * step >= min_gap || i == n {
                    cols.push((x0 + s as f64 * step, x0 + (i + 1 - blank) as f64 * step));
                    start = None;
                }
            }
            (false, None) => {}
        }
    }
    if cols.len() < 2 {
        return None;
    }
    let col_of = |w: &Word| {
        let cx = (w.bbox.x0 + w.bbox.x1) / 2.0;
        cols.iter().position(|(a, b)| cx >= *a && cx <= *b).unwrap_or_else(|| {
            // in a gutter (a cell that runs over): the column that starts nearest to its left edge
            cols.iter().enumerate().min_by(|a, b| (a.1.0 - w.bbox.x0).abs().partial_cmp(&(b.1.0 - w.bbox.x0).abs()).unwrap_or(std::cmp::Ordering::Equal)).map(|(i, _)| i).unwrap_or(0)
        })
    };
    let mut grid: Vec<Vec<String>> = vec![];
    for r in &rows {
        let mut cells = vec![String::new(); cols.len()];
        // a cell that runs over its column keeps its words together: a word follows the one
        // before it when only an ordinary space separates them
        let mut prev: Option<(usize, f64)> = None;
        for w in r {
            let c = match prev {
                Some((pc, px1)) if w.bbox.x0 - px1 < min_gap => pc,
                _ => col_of(w),
            };
            if !cells[c].is_empty() {
                cells[c].push(' ');
            }
            cells[c].push_str(w.text.trim());
            prev = Some((c, w.bbox.x1));
        }
        if cells.iter().any(|c| !c.is_empty()) {
            grid.push(cells);
        }
    }
    (grid.len() >= 2).then_some(grid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BBox;
    use crate::pdf::layout::Line;

    fn page(rows: &[&[(&str, f64)]]) -> TextPage {
        // 6 pt per character, rows 14 pt apart from y = 100
        let mut lines = vec![];
        for (ri, r) in rows.iter().enumerate() {
            let y = 100.0 + ri as f64 * 14.0;
            let words: Vec<Word> = r.iter().map(|(t, x)| Word { bbox: BBox::new(*x, y, *x + t.chars().count() as f64 * 6.0, y + 10.0), text: t.to_string() }).collect();
            lines.push(Line { bbox: BBox::new(50.0, y, 500.0, y + 10.0), words, block: 0 });
        }
        // body text under the table, outside the box
        lines.push(Line { bbox: BBox::new(50.0, 300.0, 500.0, 310.0), words: vec![Word { bbox: BBox::new(50.0, 300.0, 110.0, 310.0), text: "Afterwards".into() }], block: 1 });
        TextPage { number: 1, width: 600.0, height: 800.0, lines, blocks: vec![] }
    }

    #[test]
    fn cells_come_from_the_words_in_the_box() {
        let p = page(&[
            &[("Method", 60.0), ("Site", 250.0), ("A", 280.0), ("Site", 400.0), ("B", 430.0)],
            &[("Baseline", 60.0), ("80.7", 250.0), ("(74.0,", 280.0), ("87.4)", 322.0), ("45.0", 400.0), ("(23.7,", 430.0), ("62.6)", 472.0)],
            &[("Ours", 60.0), ("+", 90.0), ("queue,", 102.0), ("long", 144.0), ("name", 174.0), ("78.1", 250.0), ("(71.0,", 280.0), ("84.6)", 322.0), ("71.6", 400.0), ("(59.8,", 430.0), ("83.3)", 472.0)],
        ]);
        let g = grid_in(&p, [0.05, 0.11, 0.95, 0.19]).unwrap();
        assert_eq!(g, vec![
            vec!["Method".to_string(), "Site A".into(), "Site B".into()],
            vec!["Baseline".into(), "80.7 (74.0, 87.4)".into(), "45.0 (23.7, 62.6)".into()],
            vec!["Ours + queue, long name".into(), "78.1 (71.0, 84.6)".into(), "71.6 (59.8, 83.3)".into()],
        ]);
        // a box over plain text is no table
        assert!(grid_in(&p, [0.05, 0.36, 0.95, 0.40]).is_none());
    }
}
