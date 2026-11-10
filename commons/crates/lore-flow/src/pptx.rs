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
    single_slide_with(objects, w_pt, h_pt, &|_: &str| None)
}

/// As [`single_slide`], with the bytes of the picture files the objects name.
pub fn single_slide_with(objects: &[Object], w_pt: f64, h_pt: f64, pictures: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    let (src, mut deck) = blank_deck(w_pt, h_pt)?;
    let mut objs = objects.to_vec();
    crate::build::retarget_for_pptx(&mut objs);
    // a picture whose file is missing would be a broken picture in the file: a grey box instead
    fn missing(o: &mut Object, pictures: &dyn Fn(&str) -> Option<Vec<u8>>) {
        match &mut o.kind {
            lore_canvas::Kind::Picture(p) => {
                if p.asset.strip_prefix("file:").is_none_or(|f| pictures(f).is_none()) {
                    o.kind = lore_canvas::Kind::Shape(lore_canvas::SpObj { geom: lore_canvas::Geom::preset("rect"), fill: lore_canvas::Paint::solid("D9D9D9"), line: p.line.clone(), text: None, equation: false });
                }
            }
            lore_canvas::Kind::Group(g) => g.children.iter_mut().for_each(|c| missing(c, pictures)),
            _ => {}
        }
    }
    objs.iter_mut().for_each(|o| missing(o, pictures));
    deck.slides[0].objects = objs;
    pptx_out::export_bytes(&deck, &src, None, &|p: &str| pictures(p)).map_err(|e| e.to_string())
}

/// Pixel size of a PNG, JPEG or GIF.
pub fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    let be = |i: usize| -> Option<u32> { Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?)) };
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some((be(16)?, be(20)?));
    }
    if b.starts_with(b"GIF8") {
        return Some((u16::from_le_bytes([*b.get(6)?, *b.get(7)?]) as u32, u16::from_le_bytes([*b.get(8)?, *b.get(9)?]) as u32));
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let m = b[i + 1];
            if (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC {
                return Some((u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32, u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32));
            }
            if m == 0xD8 || (0xD0..=0xD7).contains(&m) || m == 0x01 || m == 0xFF {
                i += 2;
                continue;
            }
            i += 2 + u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        }
    }
    None
}

/// Read a .pptx back into canvas slides (round-trip checks).
pub fn read(bytes: &[u8]) -> Result<Deck, String> {
    let od = ODeck::from_bytes(bytes).map_err(|e| e.to_string())?;
    Ok(pptx_in::import(&od, None, None))
}
