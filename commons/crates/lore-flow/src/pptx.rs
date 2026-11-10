//! A one-slide .pptx with a figure on it, through the usual lore-canvas export
//! (native shapes, attached connectors). For the command-line tool and tests;
//! an app inserts the objects into its own deck instead.

use std::collections::BTreeMap;

use lore_canvas::base::{Base, compile};
use lore_canvas::{Deck, Object, Slide, pptx_in, pptx_out};
use lore_ooxml::Deck as ODeck;

const LAYOUT: &str = "Blank";

fn blank(w: i64, h: i64) -> Result<Vec<u8>, String> {
    let json = serde_json::json!({
        "id": "flow-blank", "name": "Blank",
        "size": { "w": w, "h": h },
        "pages": { "content": { "layout": LAYOUT, "bg": { "kind": "solid", "color": "FFFFFF" } } }
    });
    let colors: BTreeMap<String, String> = BTreeMap::new();
    let base = Base::from_json(&json.to_string(), &colors)?;
    compile(&base, &colors, &|_: &str| None).map_err(|e| e.to_string())
}

/// The empty source deck and a canvas deck with one blank slide (uid `s1`).
pub fn blank_deck(w_pt: f64, h_pt: f64) -> Result<(ODeck, Deck), String> {
    let (w, h) = ((w_pt * 12_700.0).round() as i64, (h_pt * 12_700.0).round() as i64);
    let src = ODeck::from_bytes(&blank(w, h)?).map_err(|e| e.to_string())?;
    let mut deck = pptx_in::import(&src, None, None);
    let uid = deck.new_uid();
    deck.slides.push(Slide::new(uid, Some(LAYOUT.into())));
    Ok((src, deck))
}

/// A .pptx of one slide (`w_pt` × `h_pt`) holding `objects`. Connector ends on
/// module groups are pointed at the groups' port shapes first, so PowerPoint
/// keeps them attached.
pub fn single_slide(objects: &[Object], w_pt: f64, h_pt: f64) -> Result<Vec<u8>, String> {
    let (src, mut deck) = blank_deck(w_pt, h_pt)?;
    let mut objs = objects.to_vec();
    crate::build::retarget_for_pptx(&mut objs);
    deck.slides[0].objects = objs;
    pptx_out::export_bytes(&deck, &src, None, &|_: &str| None).map_err(|e| e.to_string())
}

/// Read a .pptx back into canvas slides (round-trip checks).
pub fn read(bytes: &[u8]) -> Result<Deck, String> {
    let od = ODeck::from_bytes(bytes).map_err(|e| e.to_string())?;
    Ok(pptx_in::import(&od, None, None))
}
