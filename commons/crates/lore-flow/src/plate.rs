//! The baseplate: a figure is built on a grid of cells whose size is fixed before anything is
//! drawn (`docs/flow-blocks.md` §10, the author's layered build).
//!
//! A [`Format`] is chosen up front: a width on the page and any ratio. The ratio fixes the
//! plate's shape; the plate is that ratio times the smallest whole multiplier that holds the
//! figure, so blocks are as large as the shape allows. How small the text then is on the page
//! (the width divided by the cells across) is reported, not a reason to fail.
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

/// A page format: two independent settings the author gives before anything is drawn. Any width
/// and any ratio are valid (two columns at 1:1, one column at 16:9 …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Format {
    /// Width of the figure on the page, in points (one column, two columns, or any width).
    pub width_pt: f64,
    /// Width : height. It fixes the plate's SHAPE only, not how much fits: the plate is this
    /// ratio times the smallest whole multiplier that holds the figure.
    pub ratio: (u32, u32),
    /// Text smaller than this on the page is reported (the figure is still drawn).
    pub min_text_pt: f64,
}

impl Format {
    pub fn new(width_pt: f64, ratio: (u32, u32)) -> Format {
        Format { width_pt, ratio: reduce(ratio), min_text_pt: 6.0 }
    }
    /// The plate for a multiplier: cells across and down.
    pub fn plate(&self, k: u32) -> (u32, u32) {
        let r = reduce(self.ratio);
        (r.0 * k.max(1), r.1 * k.max(1))
    }
    /// The smallest multiplier whose plate is at least `need` cells across and down.
    pub fn multiplier(&self, need: (u32, u32)) -> u32 {
        let r = reduce(self.ratio);
        need.0.div_ceil(r.0).max(need.1.div_ceil(r.1)).max(1)
    }
    /// Size of one cell on the page for a plate `cols` cells across, in points.
    pub fn cell_pt(&self, cols: u32) -> f64 {
        self.width_pt / cols.max(1) as f64
    }
}

/// A ratio in lowest terms (16:9 stays, 32:18 becomes 16:9, 0 counts as 1).
pub fn reduce(r: (u32, u32)) -> (u32, u32) {
    let (a, b) = (r.0.max(1), r.1.max(1));
    let mut g = (a, b);
    while g.1 != 0 {
        g = (g.1, g.0 % g.1);
    }
    (a / g.0, b / g.0)
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
    /// The smallest text on the page for this plate and the format's width, in points.
    pub text_pt: f64,
    /// When `text_pt` is under the format's limit: what would help (a wider width, labels that
    /// could break, less detail). Empty when the text is large enough.
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

/// The arrangements of the graph, best first, each on the smallest plate of the format's ratio
/// that holds it. `None` only for a graph without blocks.
pub fn place(_g: &Graph, _f: &Format, _m: &Measure) -> Option<Vec<Placement>> {
    unimplemented!("docs/flow-blocks.md §10: package plate")
}

/// A placement as the layout the router and the drawing take: every rectangle on cell borders.
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
    fn a_ratio_fixes_the_shape_and_the_need_fixes_the_size() {
        let f = Format::new(500.0, (16, 9));
        assert_eq!(f.plate(10), (160, 90));
        assert_eq!(f.plate(20), (320, 180));
        // a figure that needs 190 x 100 cells gets the smallest 16:9 plate that holds it
        let k = f.multiplier((190, 100));
        assert_eq!((k, f.plate(k)), (12, (192, 108)));
        // any ratio: two columns at 1:1, one column at 5:6, a ratio not in lowest terms
        assert_eq!(Format::new(500.0, (1, 1)).plate(Format::new(500.0, (1, 1)).multiplier((190, 100))), (190, 190));
        assert_eq!(Format::new(250.0, (5, 6)).plate(16), (80, 96));
        assert_eq!(Format::new(500.0, (32, 18)).ratio, (16, 9));
        // the cell on the page shrinks as the plate grows: legibility is a separate report
        assert!(f.cell_pt(320) < f.cell_pt(160));
        assert_eq!(rect_of(Cells { x: 2, y: 3, w: 10, h: 10 }, 3.0).w, 30.0);
    }
}
