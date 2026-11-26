//! The baseplate: a figure is built on a grid of cells whose size is fixed before anything is
//! drawn (`docs/flow-blocks.md` §10, the author's layered build).
//!
//! A [`Format`] is a page format chosen up front (a two-column figure 16:9, a one-column figure
//! 5:6 …): it fixes how many cells the plate has and how large a cell is on the page, so text
//! and line weights are the same in every figure of a paper and nothing is shrunk to fit.
//! Blocks take whole cells (their picture, their words and a margin); rows and columns of the
//! plate are sized in whole cells and decide what is aligned with what; the room between them
//! is sized by the lines that have to pass. Text and lines are not bound to the grid.
//!
//! Layers, in build order: plate → base areas (panels) → modules' frames and lines → blocks →
//! words. This module measures and places; routing, drawing and the look checks stay where they
//! are (`route`, `build`, `look`).

use serde::{Deserialize, Serialize};

use crate::layout::{Layout, Rect};
use crate::spec::Graph;

/// A page format: what the finished figure has to fit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Format {
    pub id: String,
    /// Shown to the author.
    pub name: String,
    /// Width of the figure on the page, in points (a column or the full text width).
    pub width_pt: f64,
    /// Width : height.
    pub ratio: (u32, u32),
    /// Cells across. The cell is `width_pt / cols` on the page.
    pub cols: u32,
    /// The smallest text allowed on the page, in points.
    pub min_text_pt: f64,
}

impl Format {
    /// Size of one cell on the page, in points.
    pub fn cell_pt(&self) -> f64 {
        self.width_pt / self.cols.max(1) as f64
    }
    /// Cells down (the ratio applied to `cols`, rounded).
    pub fn rows(&self) -> u32 {
        ((self.cols as f64) * self.ratio.1 as f64 / self.ratio.0.max(1) as f64).round().max(1.0) as u32
    }
    /// The neutral formats every installation has; the author's own list (per journal) lies
    /// over them by id.
    pub fn builtin() -> Vec<Format> {
        vec![
            Format { id: "two-column-16x9".into(), name: "雙欄 16:9".into(), width_pt: 500.0, ratio: (16, 9), cols: 160, min_text_pt: 6.0 },
            Format { id: "one-column-5x6".into(), name: "單欄 5:6".into(), width_pt: 250.0, ratio: (5, 6), cols: 80, min_text_pt: 6.0 },
            Format { id: "slide-16x9".into(), name: "投影片 16:9".into(), width_pt: 960.0, ratio: (16, 9), cols: 160, min_text_pt: 12.0 },
        ]
    }
}

/// A rectangle of cells: column and row of its top-left cell, cells across and down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Cells {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// What one block needs on the plate, in cells: its picture, its words and its margin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Footprint {
    /// Index of the node in the graph.
    pub node: usize,
    pub w: u32,
    pub h: u32,
    /// How much it may be enlarged to fill spare room (1.0 = a block whose size is its words).
    pub grow: f64,
}

/// The area a figure needs, in cells, before it is placed: the sum the author asked to see.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Budget {
    /// Cells of the blocks' footprints.
    pub blocks: u32,
    /// Cells of the margins inside base areas and between them.
    pub margins: u32,
    /// Cells of the channels lines run in (from how many lines pass between neighbours).
    pub channels: u32,
    /// The least plate that holds an arrangement: cells across and down.
    pub least: (u32, u32),
}

/// One way of arranging the figure on the plate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// Which arrangement this is, for the author: `left-to-right`, `top-to-bottom`, `two-rows` …
    pub arrangement: String,
    /// The plate: cells across and down.
    pub plate: (u32, u32),
    /// Widths of the plate's columns and of the gaps between them, in cells (alternating: column,
    /// gap, column …), and the same for rows. Columns and rows decide alignment.
    pub columns: Vec<u32>,
    pub rows: Vec<u32>,
    /// Where each block sits (same order as the graph's nodes).
    pub blocks: Vec<Cells>,
    /// Where each base area (panel) sits (same order as the layout's panels).
    pub areas: Vec<Cells>,
    pub budget: Budget,
    /// Cells of the plate left empty, as a share (0..1).
    pub empty: f64,
}

/// Why a figure does not fit a format, in the author's terms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Misfit {
    /// Cells needed across and down by the tightest arrangement.
    pub need: (u32, u32),
    /// Cells the format has.
    pub have: (u32, u32),
    /// What would make it fit: another format's id, labels to break, a ratio that holds it.
    pub ways: Vec<String>,
}

/// Everything `place` needs to measure blocks (the same things `layout::layout_with` takes).
pub struct Measure<'a> {
    pub theme: &'a crate::theme::Theme,
    pub vars: &'a crate::text::Vars,
    pub library: &'a crate::modules::Library,
    pub image_size: &'a dyn Fn(&str) -> Option<(u32, u32)>,
}

/// The footprint of every block of the graph on a format's grid.
pub fn footprints(_g: &Graph, _f: &Format, _m: &Measure) -> Vec<Footprint> {
    unimplemented!("docs/flow-blocks.md §10: package plate")
}

/// The arrangements of the graph that fit the format, best first; or why none fits.
pub fn place(_g: &Graph, _f: &Format, _m: &Measure) -> Result<Vec<Placement>, Misfit> {
    unimplemented!("docs/flow-blocks.md §10: package plate")
}

/// A placement as the layout the router and the drawing take: every rectangle on cell borders,
/// the scale fixed by the format (never fitted afterwards).
pub fn to_layout(_g: &Graph, _f: &Format, _p: &Placement, _m: &Measure) -> Layout {
    unimplemented!("docs/flow-blocks.md §10: package plate")
}

/// A cell rectangle in the layout's coordinates (points at the layout's scale).
pub fn rect_of(c: Cells, cell: f64) -> Rect {
    Rect::new(c.x as f64 * cell, c.y as f64 * cell, c.w as f64 * cell, c.h as f64 * cell)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_format_fixes_the_plate() {
        let f = Format::builtin();
        let two = f.iter().find(|x| x.id == "two-column-16x9").unwrap();
        assert_eq!((two.cols, two.rows()), (160, 90));
        let one = f.iter().find(|x| x.id == "one-column-5x6").unwrap();
        assert_eq!((one.cols, one.rows()), (80, 96));
        // one cell is the same size on the page in both, so text and lines match across figures
        assert!((two.cell_pt() - one.cell_pt()).abs() < 1e-9);
        assert_eq!(rect_of(Cells { x: 2, y: 3, w: 10, h: 10 }, two.cell_pt()).w, 10.0 * two.cell_pt());
    }
}
