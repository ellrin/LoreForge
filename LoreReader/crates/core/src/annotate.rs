//! LoreReader's annotations written into a copy of a PDF as standard PDF annotations, and
//! read back from such a copy ("另存新檔（含筆記）" and the round trip after `private/` is
//! wiped). The source PDF is never modified: [`write`] returns new bytes.
//!
//! Encoding (one LoreReader annotation → one or two PDF annotations):
//! - every annotation with `rects` → `/Subtype /Highlight`: one `/QuadPoints` quad per rect,
//!   `/C` = its colour, `/AP` = the quads filled with blend mode Multiply (macOS Preview only
//!   draws annotations that carry an appearance stream), `/NM (lr:<id>)`, `/F 4` (print).
//!   A highlight's own note goes in its `/Contents`.
//! - `kind: "note"` (or no rects) → also a sticky note, `/Subtype /Text` with `/NM (lr:<id>/note)`
//!   beside the first rect, carrying the note in `/Contents`; the highlight then has none
//!   (one comment per note in Acrobat's and Preview's comment lists).
//! - The first of the two carries `/LoreReader` = the annotation as JSON (id, quote, rects,
//!   colour, dates), so a re-import is exact. The visible `/Contents` wins over the JSON note
//!   (a note edited in another viewer comes back edited).
//! - `kind: "text"` (a text box placed on the page) → `/Subtype /FreeText` with `/NM (lr:<id>)`,
//!   `/Rect` = the box (grown downwards when the text needs more room), `/Contents` = the
//!   text, `/DA`, and an `/AP` that draws the box and the wrapped text, upright on rotated
//!   pages (the form's `/Matrix` turns it with `/Rotate`). Its colours are fixed ([`TEXT_FILL`]).
//!   Latin-1 runs use Helvetica; every other character the non-embedded Adobe-CNS1 font
//!   `MSung-Light` (`/UniCNS-UTF16-H`), which Preview, Acrobat and poppler replace with a
//!   system CJK face: the file stays small and Chinese shows without regenerating.
//! - Document info `/LoreReaderSource` = sha256 of the paper the notes belong to.
//!
//! Re-exporting first removes every annotation whose `/NM` starts with `lr:`; annotations
//! made by other tools are left alone.
//!
//! Geometry: `rects` are fractions of the page as displayed (pdf.js: CropBox ∩ MediaBox,
//! turned by `/Rotate`), origin top-left. [`PageGeom`] maps them to PDF user space.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary};

use crate::papers::Annotation;
use crate::pdfops::{decode_text_string, flattened_page, load};
use crate::Result;

/// Document info key with the sha256 of the paper the notes belong to.
pub const SOURCE_KEY: &str = "LoreReaderSource";
/// `/NM` prefix of the annotations LoreReader writes.
pub const NM_PREFIX: &str = "lr:";
const DATA_KEY: &str = "LoreReader";
/// Sticky-note icon size in points.
const ICON: f64 = 16.0;

/// Text box: font size (pt), padding (pt), line height (× size), fill, border, ink. The web
/// box (notes.css) uses the same numbers so the wrap on screen and in the PDF agree.
pub const TEXT_PT: f64 = 10.0;
pub const TEXT_PAD: f64 = 4.0;
pub const TEXT_LEAD: f64 = 1.3;
pub const TEXT_FILL: [f64; 3] = [1.0, 0.973, 0.784];
const TEXT_LINE: [f64; 3] = [0.851, 0.725, 0.282];
const TEXT_INK: [f64; 3] = [0.133, 0.133, 0.133];

/// The colours of `papers::COLORS` (same as the web UI's `--hl-*`).
pub fn rgb(color: &str) -> [f64; 3] {
    match color {
        "green" => [0.561, 0.890, 0.533],
        "blue" => [0.549, 0.784, 1.0],
        "pink" => [1.0, 0.624, 0.812],
        _ => [1.0, 0.886, 0.290],
    }
}

fn nearest_color(c: &[f64]) -> String {
    let d = |n: &str| rgb(n).iter().zip(c).map(|(a, b)| (a - b).powi(2)).sum::<f64>();
    crate::papers::COLORS.iter().min_by(|a, b| d(a).total_cmp(&d(b))).unwrap_or(&"yellow").to_string()
}

/// A page's displayed box (CropBox ∩ MediaBox, user space) and its rotation (0/90/180/270).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGeom {
    pub bx: [f64; 4],
    pub rotate: i64,
}

impl PageGeom {
    fn wh(&self) -> (f64, f64) {
        (self.bx[2] - self.bx[0], self.bx[3] - self.bx[1])
    }

    /// Displayed size (points), width × height after rotation.
    pub fn display_size(&self) -> (f64, f64) {
        let (w, h) = self.wh();
        if self.rotate % 180 == 90 { (h, w) } else { (w, h) }
    }

    /// A point of the displayed page (fractions, top-left origin) in user space.
    pub fn to_user(&self, u: f64, v: f64) -> (f64, f64) {
        let [x0, y0, x1, y1] = self.bx;
        let (w, h) = self.wh();
        match self.rotate {
            90 => (x0 + v * w, y0 + u * h),
            180 => (x1 - u * w, y0 + v * h),
            270 => (x1 - v * w, y1 - u * h),
            _ => (x0 + u * w, y1 - v * h),
        }
    }

    /// The inverse of [`PageGeom::to_user`].
    pub fn to_frac(&self, x: f64, y: f64) -> (f64, f64) {
        let [x0, y0, x1, y1] = self.bx;
        let (w, h) = self.wh();
        match self.rotate {
            90 => ((y - y0) / h, (x - x0) / w),
            180 => ((x1 - x) / w, (y - y0) / h),
            270 => ((y1 - y) / h, (x1 - x) / w),
            _ => ((x - x0) / w, (y1 - y) / h),
        }
    }

    /// QuadPoints of a displayed rect: top-left, top-right, bottom-left, bottom-right as
    /// seen on screen (the order Acrobat writes for text running left to right).
    pub fn quad(&self, r: [f64; 4]) -> [(f64, f64); 4] {
        [self.to_user(r[0], r[1]), self.to_user(r[2], r[1]), self.to_user(r[0], r[3]), self.to_user(r[2], r[3])]
    }

    /// The displayed rect covered by four user-space points.
    pub fn rect_of(&self, pts: &[(f64, f64)]) -> [f64; 4] {
        let f: Vec<(f64, f64)> = pts.iter().map(|&(x, y)| self.to_frac(x, y)).collect();
        let c = |x: f64| x.clamp(0.0, 1.0);
        let (mut u0, mut v0, mut u1, mut v1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (u, v) in f {
            (u0, v0, u1, v1) = (u0.min(u), v0.min(v), u1.max(u), v1.max(v));
        }
        [c(u0), c(v0), c(u1), c(v1)]
    }
}

fn num(doc: &Document, o: &Object) -> Option<f64> {
    doc.dereference(o).ok()?.1.as_float().ok().map(f64::from)
}

fn rect4(doc: &Document, o: Option<&Object>) -> Option<[f64; 4]> {
    let a = doc.dereference(o?).ok()?.1.as_array().ok()?;
    if a.len() != 4 {
        return None;
    }
    let v: Vec<f64> = a.iter().filter_map(|x| num(doc, x)).collect();
    (v.len() == 4).then(|| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

/// The page's displayed box and rotation, as pdf.js computes them.
pub fn page_geom(doc: &Document, page: ObjectId) -> Result<PageGeom> {
    let d = flattened_page(doc, page)?;
    let media = rect4(doc, d.get(b"MediaBox").ok()).unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let bx = match rect4(doc, d.get(b"CropBox").ok()) {
        Some(c) => {
            let i = [c[0].max(media[0]), c[1].max(media[1]), c[2].min(media[2]), c[3].min(media[3])];
            if i[2] > i[0] && i[3] > i[1] { i } else { media }
        }
        None => media,
    };
    let r = d.get(b"Rotate").ok().and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_i64().ok()).unwrap_or(0);
    let r = r.rem_euclid(360);
    Ok(PageGeom { bx, rotate: if r % 90 == 0 { r } else { 0 } })
}

/// A PDF text string: plain literal for printable ASCII, else UTF-16BE with a BOM.
pub fn text_string(s: &str) -> Object {
    if s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        Object::string_literal(s)
    } else {
        let mut b = vec![0xFE, 0xFF];
        for u in s.encode_utf16() {
            b.extend(u.to_be_bytes());
        }
        Object::String(b, StringFormat::Hexadecimal)
    }
}

fn text_of(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<String> {
    let o = doc.dereference(d.get(key).ok()?).ok()?.1;
    Some(decode_text_string(o.as_str().ok()?))
}

fn reals(v: &[f64]) -> Object {
    Object::Array(v.iter().map(|&x| Object::Real(x as f32)).collect())
}

fn f(x: f64) -> String {
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.to_string() }
}

fn bbox(pts: &[(f64, f64)], pad: f64) -> [f64; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in pts {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    [x0 - pad, y0 - pad, x1 + pad, y1 + pad]
}

fn form(doc: &mut Document, bbox: [f64; 4], resources: Option<Dictionary>, ops: String) -> ObjectId {
    let mut d = dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "FormType" => 1,
        "BBox" => reals(&bbox), "Matrix" => reals(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
    };
    if let Some(r) = resources {
        d.set("Resources", r);
    }
    doc.add_object(Stream::new(d, ops.into_bytes()))
}

/// The page's `/Annots` entries (resolved array, cloned).
fn annots_of(doc: &Document, page: ObjectId) -> Vec<Object> {
    let Ok(d) = doc.get_dictionary(page) else { return vec![] };
    let Ok(a) = d.get(b"Annots") else { return vec![] };
    doc.dereference(a).ok().and_then(|(_, o)| o.as_array().ok().cloned()).unwrap_or_default()
}

fn annot_dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    doc.dereference(o).ok()?.1.as_dict().ok()
}

fn lr_nm(doc: &Document, d: &Dictionary) -> Option<String> {
    text_of(doc, d, b"NM").filter(|n| n.starts_with(NM_PREFIX))
}

/// Remove LoreReader's own annotations from every page; returns how many were removed.
fn strip_in(doc: &mut Document) -> usize {
    let mut removed = 0;
    for (_, page) in doc.get_pages() {
        let annots = annots_of(doc, page);
        let mut keep = Vec::with_capacity(annots.len());
        let mut gone: Vec<ObjectId> = vec![];
        for o in annots {
            match annot_dict(doc, &o) {
                Some(d) if lr_nm(doc, d).is_some() => {
                    removed += 1;
                    if let Object::Reference(id) = o {
                        gone.push(id);
                    }
                    if let Ok(Object::Reference(ap)) = d.get(b"AP").and_then(|ap| ap.as_dict()).and_then(|ap| ap.get(b"N")) {
                        gone.push(*ap);
                    }
                }
                _ => keep.push(o),
            }
        }
        if !gone.is_empty() || removed > 0 {
            for id in gone {
                doc.objects.remove(&id);
            }
            if let Ok(d) = doc.get_dictionary_mut(page) {
                if keep.is_empty() {
                    d.remove(b"Annots");
                } else {
                    d.set("Annots", Object::Array(keep));
                }
            }
        }
    }
    removed
}

/// Helvetica advance widths (1/1000 em) for ASCII 32..=126.
const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Drawn with Helvetica (WinAnsi) rather than the CJK font.
fn latin(c: char) -> bool {
    matches!(c as u32, 0x20..=0x7e | 0xa0..=0xff)
}

/// Advance width of `c` in 1/1000 em (CJK font: every glyph 1 em).
fn width(c: char) -> f64 {
    match c as u32 {
        n @ 0x20..=0x7e => HELV[(n - 0x20) as usize] as f64,
        0xa0..=0xff => 556.0,
        _ => 1000.0,
    }
}

fn text_width(s: &str) -> f64 {
    s.chars().map(width).sum::<f64>() * TEXT_PT / 1000.0
}

/// `text` broken into lines no wider than `max` points at [`TEXT_PT`]: at spaces between
/// Latin words, anywhere between CJK characters, inside a word only when it alone is too wide.
pub fn wrap(text: &str, max: f64) -> Vec<String> {
    let mut out = vec![];
    for para in text.split('\n') {
        // tokens: a Latin word with its trailing spaces, or one other character
        let mut toks: Vec<String> = vec![];
        for c in para.chars().filter(|c| !c.is_control()) {
            let last = toks.last().and_then(|t| t.chars().last());
            let join = match last {
                Some(l) if c == ' ' => latin(l),
                Some(l) => latin(c) && latin(l) && l != ' ',
                None => false,
            };
            if join {
                toks.last_mut().unwrap().push(c);
            } else {
                toks.push(c.to_string());
            }
        }
        let mut line = String::new();
        for t in toks {
            if line.is_empty() || text_width(&format!("{line}{}", t.trim_end())) <= max {
                line.push_str(&t);
            } else {
                out.push(line.trim_end().to_string());
                line = t.trim_start().to_string();
            }
            // A word wider than the box on its own: cut it.
            while text_width(line.trim_end()) > max && line.chars().count() > 1 {
                let chars: Vec<char> = line.chars().collect();
                let mut k = 1;
                while k < chars.len() && text_width(&chars[..=k].iter().collect::<String>()) <= max {
                    k += 1;
                }
                out.push(chars[..k].iter().collect());
                line = chars[k..].iter().collect();
            }
        }
        out.push(line.trim_end().to_string());
    }
    out
}

/// One line as content-stream operators, switching between Helvetica and the CJK font.
fn show_line(line: &str) -> String {
    let mut ops = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let l = latin(chars[i]);
        let mut j = i;
        while j < chars.len() && latin(chars[j]) == l {
            j += 1;
        }
        let hex: String = if l {
            chars[i..j].iter().map(|&c| format!("{:02X}", c as u32)).collect()
        } else {
            chars[i..j].iter().flat_map(|c| c.encode_utf16(&mut [0; 2]).to_vec()).map(|u| format!("{u:04X}")).collect()
        };
        ops += &format!("/{} {} Tf <{hex}> Tj ", if l { "Helv" } else { "CJK" }, f(TEXT_PT));
        i = j;
    }
    ops
}

/// The fonts of a text box's appearance: Helvetica and MSung-Light (Adobe-CNS1, not embedded).
fn text_fonts(doc: &mut Document) -> Dictionary {
    let desc = doc.add_object(dictionary! {
        "Type" => "FontDescriptor", "FontName" => "MSung-Light", "Flags" => 6,
        "FontBBox" => reals(&[-160.0, -249.0, 1015.0, 1071.0]), "ItalicAngle" => 0,
        "Ascent" => 880, "Descent" => -120, "CapHeight" => 880, "StemV" => 93,
    });
    let cid = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType0", "BaseFont" => "MSung-Light",
        "CIDSystemInfo" => dictionary! { "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("CNS1"), "Supplement" => 0 },
        "FontDescriptor" => desc, "DW" => 1000,
    });
    let cjk = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "MSung-Light", "Encoding" => "UniCNS-UTF16-H",
        "DescendantFonts" => vec![cid.into()],
    });
    let helv = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
    dictionary! { "Helv" => helv, "CJK" => cjk }
}

/// The form matrix that turns a box drawn upright (as displayed) into user space.
fn rotation_matrix(rotate: i64) -> [f64; 6] {
    match rotate {
        90 => [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
        180 => [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
        270 => [0.0, -1.0, 1.0, 0.0, 0.0, 0.0],
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    }
}

/// A text box's displayed rect (fractions) grown downwards to fit its wrapped text, and the lines.
pub fn text_layout(g: &PageGeom, r: [f64; 4], text: &str) -> ([f64; 4], Vec<String>) {
    let (dw, dh) = g.display_size();
    let (dw, dh) = (dw.max(1.0), dh.max(1.0));
    let w = ((r[2] - r[0]) * dw).max(2.0 * TEXT_PAD + TEXT_PT);
    let lines = wrap(text.trim_end(), w - 2.0 * TEXT_PAD);
    let need = 2.0 * TEXT_PAD + lines.len() as f64 * TEXT_PT * TEXT_LEAD;
    let h = ((r[3] - r[1]) * dh).max(need);
    let (x0, y0) = (r[0].clamp(0.0, 1.0), r[1].clamp(0.0, 1.0));
    ([x0, y0, x0 + w / dw, y0 + h / dh], lines)
}

fn add_freetext(doc: &mut Document, page: ObjectId, g: &PageGeom, a: &Annotation, data: &str) -> Option<ObjectId> {
    let (r, lines) = text_layout(g, *a.rects.first()?, &a.note);
    let (dw, dh) = g.display_size();
    let (w, h) = ((r[2] - r[0]) * dw, (r[3] - r[1]) * dh);
    let rect = bbox(&[g.to_user(r[0], r[1]), g.to_user(r[2], r[3])], 0.0);
    let c = |v: [f64; 3]| format!("{} {} {}", f(v[0]), f(v[1]), f(v[2]));
    let mut ops = format!("q {} rg 0 0 {} {} re f {} RG 0.75 w 0.375 0.375 {} {} re S Q\n", c(TEXT_FILL), f(w), f(h), c(TEXT_LINE), f(w - 0.75), f(h - 0.75));
    // Each line box is TEXT_LEAD × size; the baseline sits 0.8 em below the line box's
    // half-leading (the web box uses the same line height and padding).
    let lead = TEXT_PT * TEXT_LEAD;
    let first = h - TEXT_PAD - (lead - TEXT_PT) / 2.0 - 0.8 * TEXT_PT;
    ops += &format!("q 0 0 {} {} re W n BT {} rg {} TL 1 0 0 1 {} {} Tm\n", f(w), f(h), c(TEXT_INK), f(lead), f(TEXT_PAD), f(first));
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            ops += "T* ";
        }
        ops += &show_line(line);
        ops += "\n";
    }
    ops += "ET Q\n";
    let fonts = text_fonts(doc);
    let form = dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "FormType" => 1,
        "BBox" => reals(&[0.0, 0.0, w, h]), "Matrix" => reals(&rotation_matrix(g.rotate)),
        "Resources" => dictionary! { "Font" => fonts },
    };
    let ap = doc.add_object(Stream::new(form, ops.into_bytes()));
    let d = dictionary! {
        "Type" => "Annot", "Subtype" => "FreeText", "Rect" => reals(&rect),
        "Contents" => text_string(&a.note), "DA" => Object::string_literal(format!("/Helv {} Tf {} rg", f(TEXT_PT), c(TEXT_INK))),
        "Q" => 0, "C" => reals(&TEXT_FILL), "BS" => dictionary! { "W" => 0.75, "S" => "S" },
        "F" => 4, "P" => page, "NM" => text_string(&format!("{NM_PREFIX}{}", a.id)), "T" => text_string("LoreReader"),
        "AP" => dictionary! { "N" => ap }, DATA_KEY => text_string(data),
    };
    Some(doc.add_object(d))
}

fn add_highlight(doc: &mut Document, page: ObjectId, g: &PageGeom, a: &Annotation, contents: &str, data: Option<&str>) -> ObjectId {
    let quads: Vec<[(f64, f64); 4]> = a.rects.iter().map(|&r| g.quad(r)).collect();
    let pts: Vec<(f64, f64)> = quads.iter().flatten().copied().collect();
    let rect = bbox(&pts, 1.0);
    let [r, gg, b] = rgb(&a.color);
    let mut ops = format!("/G0 gs {} {} {} rg\n", f(r), f(gg), f(b));
    for q in &quads {
        // TL TR BR BL
        ops += &format!("{} {} m {} {} l {} {} l {} {} l h f\n", f(q[0].0), f(q[0].1), f(q[1].0), f(q[1].1), f(q[3].0), f(q[3].1), f(q[2].0), f(q[2].1));
    }
    let res = dictionary! { "ExtGState" => dictionary! { "G0" => dictionary! { "Type" => "ExtGState", "BM" => "Multiply", "CA" => 1, "ca" => 1 } } };
    let ap = form(doc, rect, Some(res), ops);
    let qp: Vec<f64> = pts.iter().flat_map(|&(x, y)| [x, y]).collect();
    let mut d = dictionary! {
        "Type" => "Annot", "Subtype" => "Highlight", "Rect" => reals(&rect), "QuadPoints" => reals(&qp),
        "C" => reals(&[r, gg, b]), "CA" => 1, "F" => 4, "P" => page,
        "NM" => text_string(&format!("{NM_PREFIX}{}", a.id)), "T" => text_string("LoreReader"),
        "AP" => dictionary! { "N" => ap },
    };
    if !contents.is_empty() {
        d.set("Contents", text_string(contents));
    }
    if let Some(j) = data {
        d.set(DATA_KEY, text_string(j));
    }
    doc.add_object(d)
}

fn add_sticky(doc: &mut Document, page: ObjectId, g: &PageGeom, a: &Annotation, data: Option<&str>) -> ObjectId {
    // Beside the first rect: in the left margin when there is room, else right of it.
    let (dw, dh) = g.display_size();
    // Viewers may draw their own, slightly larger icon from the top-left corner: keep 10 pt clear.
    let (iw, ih, gap) = (ICON / dw.max(1.0), ICON / dh.max(1.0), 10.0 / dw.max(1.0));
    let (u, v) = match a.rects.first() {
        Some(r) if r[0] - iw - gap >= 0.0 => (r[0] - iw - gap, r[1]),
        Some(r) => ((r[2] + 0.006).min(1.0 - iw), r[1]),
        None => (0.02, 0.02),
    };
    let v = v.clamp(0.0, (1.0 - ih).max(0.0));
    let rect = bbox(&[g.to_user(u, v), g.to_user(u + iw, v + ih)], 0.0);
    let [r, gg, b] = rgb(&a.color);
    let ops = format!(
        "0.35 0.35 0.35 RG 0.8 w {} {} {} rg 1.5 1.5 13 13 re B\n0.25 0.25 0.25 RG 1 w 4 11 m 12 11 l 4 8 m 12 8 l 4 5 m 9.5 5 l S\n",
        f(r), f(gg), f(b)
    );
    let ap = form(doc, [0.0, 0.0, ICON, ICON], None, ops);
    let mut d = dictionary! {
        "Type" => "Annot", "Subtype" => "Text", "Rect" => reals(&rect), "Name" => "Comment", "Open" => false,
        "C" => reals(&[r, gg, b]), "F" => 4, "P" => page,
        "NM" => text_string(&format!("{NM_PREFIX}{}/note", a.id)), "T" => text_string("LoreReader"),
        "Contents" => text_string(&a.note), "AP" => dictionary! { "N" => ap },
    };
    if let Some(j) = data {
        d.set(DATA_KEY, text_string(j));
    }
    doc.add_object(d)
}

fn set_info(doc: &mut Document, key: &str, value: &str) -> Result<()> {
    match doc.trailer.get(b"Info").ok().cloned() {
        Some(Object::Reference(id)) if doc.get_dictionary(id).is_ok() => {
            doc.get_dictionary_mut(id)?.set(key, text_string(value));
        }
        Some(Object::Dictionary(mut d)) => {
            d.set(key, text_string(value));
            doc.trailer.set("Info", d);
        }
        _ => {
            let id = doc.add_object(dictionary! { key => text_string(value) });
            doc.trailer.set("Info", id);
        }
    }
    Ok(())
}

/// A copy of `bytes` with `annots` written in as PDF annotations and `source_sha` in the
/// document info. LoreReader annotations already in `bytes` are replaced; others kept.
/// Annotations on pages the PDF does not have are skipped.
pub fn write(bytes: &[u8], annots: &[Annotation], source_sha: &str) -> Result<Vec<u8>> {
    let mut doc = load(bytes)?;
    strip_in(&mut doc);
    let pages = doc.get_pages();
    for a in annots {
        let Some(&page) = pages.get(&a.page) else { continue };
        let g = page_geom(&doc, page)?;
        let json = serde_json::to_string(a)?;
        if a.kind == "text" {
            if a.note.trim().is_empty() {
                continue;
            }
            if let Some(id) = add_freetext(&mut doc, page, &g, a, &json) {
                let mut list = annots_of(&doc, page);
                list.push(Object::Reference(id));
                doc.get_dictionary_mut(page)?.set("Annots", Object::Array(list));
            }
            continue;
        }
        let sticky = a.kind == "note" && !a.note.trim().is_empty() || a.rects.is_empty();
        let mut ids = vec![];
        if !a.rects.is_empty() {
            ids.push(add_highlight(&mut doc, page, &g, a, if sticky { "" } else { &a.note }, Some(&json)));
        }
        if sticky {
            ids.push(add_sticky(&mut doc, page, &g, a, ids.is_empty().then_some(json.as_str())));
        }
        let mut list = annots_of(&doc, page);
        list.extend(ids.into_iter().map(Object::Reference));
        doc.get_dictionary_mut(page)?.set("Annots", Object::Array(list));
    }
    set_info(&mut doc, SOURCE_KEY, source_sha)?;
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

/// What a PDF carries from LoreReader.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Embedded {
    /// `/LoreReaderSource`: the sha256 of the paper the notes were made on.
    pub source: Option<String>,
    pub annotations: Vec<Annotation>,
}

impl Embedded {
    pub fn is_empty(&self) -> bool {
        self.source.is_none() && self.annotations.is_empty()
    }
}

/// The `/LoreReaderSource` marker, if any (a valid sha256).
pub fn source_of(doc: &Document) -> Option<String> {
    let info = match doc.trailer.get(b"Info").ok()? {
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        Object::Dictionary(d) => d,
        _ => return None,
    };
    text_of(doc, info, SOURCE_KEY.as_bytes()).map(|s| s.trim().to_ascii_lowercase()).filter(|s| crate::library::valid_sha(s))
}

/// LoreReader's annotations and source marker in `bytes` (empty for any other PDF).
pub fn read(bytes: &[u8]) -> Result<Embedded> {
    let doc = load(bytes)?;
    let mut out: Vec<Annotation> = vec![];
    // id → the note's visible text (sticky note, or the highlight when it has no sticky)
    let mut visible: Vec<(String, bool, String)> = vec![];
    for (n, page) in doc.get_pages() {
        let g = page_geom(&doc, page)?;
        for o in annots_of(&doc, page) {
            let Some(d) = annot_dict(&doc, &o) else { continue };
            let Some(nm) = lr_nm(&doc, d) else { continue };
            let rest = &nm[NM_PREFIX.len()..];
            let (id, is_sticky) = match rest.strip_suffix("/note") {
                Some(id) => (id.to_string(), true),
                None => (rest.to_string(), false),
            };
            if id.is_empty() {
                continue;
            }
            visible.push((id.clone(), is_sticky, text_of(&doc, d, b"Contents").unwrap_or_default()));
            let parsed = text_of(&doc, d, DATA_KEY.as_bytes()).and_then(|j| serde_json::from_str::<Annotation>(&j).ok());
            let a = match parsed {
                Some(mut a) => {
                    a.id = id.clone();
                    a.page = n;
                    a
                }
                None if out.iter().any(|x| x.id == id) => {
                    if is_sticky && let Some(x) = out.iter_mut().find(|x| x.id == id) {
                        x.kind = "note".into();
                    }
                    continue;
                }
                None => {
                    // No JSON (another tool rewrote the annotation): rebuild from geometry.
                    if d.get(b"Subtype").and_then(|s| s.as_name()).is_ok_and(|s| s == b"FreeText") {
                        let Some(r) = rect4(&doc, d.get(b"Rect").ok()) else { continue };
                        let rects = vec![g.rect_of(&[(r[0], r[1]), (r[2], r[1]), (r[0], r[3]), (r[2], r[3])])];
                        out.push(serde_json::from_value(serde_json::json!({ "id": id, "page": n, "kind": "text", "rects": rects })).expect("annotation"));
                        continue;
                    }
                    let mut rects = vec![];
                    if let Some(q) = d.get(b"QuadPoints").ok().and_then(|q| doc.dereference(q).ok()).and_then(|(_, q)| q.as_array().ok()) {
                        let v: Vec<f64> = q.iter().filter_map(|x| num(&doc, x)).collect();
                        for c in v.as_chunks::<8>().0 {
                            rects.push(g.rect_of(&[(c[0], c[1]), (c[2], c[3]), (c[4], c[5]), (c[6], c[7])]));
                        }
                    }
                    let color = d.get(b"C").ok().and_then(|c| doc.dereference(c).ok()).and_then(|(_, c)| c.as_array().ok()).map(|c| c.iter().filter_map(|x| num(&doc, x)).collect::<Vec<_>>()).filter(|c| c.len() == 3).map(|c| nearest_color(&c)).unwrap_or_else(|| "yellow".into());
                    let ann: Annotation = serde_json::from_value(serde_json::json!({ "id": id, "page": n, "kind": if is_sticky { "note" } else { "highlight" }, "color": color, "rects": rects })).expect("annotation");
                    ann
                }
            };
            match out.iter_mut().find(|x| x.id == id) {
                // The highlight's rects are the anchor; a sticky only adds its note.
                Some(x) if is_sticky => x.kind = "note".into(),
                Some(_) => {}
                None => out.push(a),
            }
        }
    }
    // The visible /Contents wins: the sticky's when there is one, else the highlight's.
    for a in &mut out {
        let sticky = visible.iter().find(|(id, s, _)| *id == a.id && *s);
        let hl = visible.iter().find(|(id, s, _)| *id == a.id && !*s);
        if let Some((_, _, t)) = sticky.or(hl) {
            a.note = t.clone();
        }
    }
    Ok(Embedded { source: source_of(&doc), annotations: out })
}

/// `bytes` without LoreReader's annotations (other annotations and the info kept).
pub fn strip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut doc = load(bytes)?;
    strip_in(&mut doc);
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::papers::TextQuote;

    fn ann(id: &str, page: u32, kind: &str, color: &str, rects: Vec<[f64; 4]>, note: &str) -> Annotation {
        serde_json::from_value(serde_json::json!({
            "id": id, "page": page, "kind": kind, "color": color, "rects": rects, "note": note,
            "quote": TextQuote { exact: "Dice coefficient".into(), prefix: "the ".into(), suffix: " rewards".into() },
            "created_at": "2026-11-10T09:00:00+08:00", "updated_at": "2026-11-10T09:00:00+08:00",
        }))
        .unwrap()
    }

    /// The demo paper with page 2 turned 90° and page 3 given a CropBox and 270°.
    fn rotated_demo() -> Vec<u8> {
        let mut doc = Document::load_mem(&crate::synth::demo_paper()).unwrap();
        let pages = doc.get_pages();
        doc.get_dictionary_mut(pages[&2]).unwrap().set("Rotate", 90);
        let p3 = doc.get_dictionary_mut(pages[&3]).unwrap();
        p3.set("Rotate", -90);
        p3.set("CropBox", vec![36.into(), 50.into(), 576.into(), 742.into()]);
        let mut out = vec![];
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn geometry_round_trips_for_every_rotation() {
        for rotate in [0, 90, 180, 270] {
            let g = PageGeom { bx: [10.0, 20.0, 610.0, 820.0], rotate };
            for (u, v) in [(0.0, 0.0), (0.25, 0.75), (1.0, 1.0)] {
                let (x, y) = g.to_user(u, v);
                let (u2, v2) = g.to_frac(x, y);
                assert!((u - u2).abs() < 1e-9 && (v - v2).abs() < 1e-9, "{rotate}: {u},{v}");
            }
        }
        // The displayed top-left corner in user space.
        let g = |rotate| PageGeom { bx: [0.0, 0.0, 600.0, 800.0], rotate };
        assert_eq!(g(0).to_user(0.0, 0.0), (0.0, 800.0));
        assert_eq!(g(90).to_user(0.0, 0.0), (0.0, 0.0));
        assert_eq!(g(180).to_user(0.0, 0.0), (600.0, 0.0));
        assert_eq!(g(270).to_user(0.0, 0.0), (600.0, 800.0));
        assert_eq!(g(90).display_size(), (800.0, 600.0));
        // Moving right on screen on a 90° page goes up in user space.
        assert_eq!(g(90).to_user(1.0, 0.0), (0.0, 800.0));
    }

    #[test]
    fn write_read_rewrite_strip() {
        let src = rotated_demo();
        let doc = load(&src).unwrap();
        let pages = doc.get_pages();
        assert_eq!(page_geom(&doc, pages[&2]).unwrap().rotate, 90);
        assert_eq!(page_geom(&doc, pages[&3]).unwrap(), PageGeom { bx: [36.0, 50.0, 576.0, 742.0], rotate: 270 });

        let list = vec![
            ann("a1", 1, "highlight", "yellow", vec![[0.1, 0.2, 0.5, 0.23], [0.1, 0.24, 0.3, 0.27]], ""),
            ann("a2", 1, "note", "green", vec![[0.4, 0.5, 0.7, 0.53]], "要再查：Dice 與 Jaccard 的關係"),
            ann("a3", 2, "highlight", "blue", vec![[0.2, 0.3, 0.6, 0.34]], "旋轉頁"),
            ann("a4", 3, "note", "pink", vec![[0.15, 0.1, 0.5, 0.14]], "crop + 270"),
            ann("a9", 9, "highlight", "yellow", vec![[0.1, 0.1, 0.2, 0.2]], ""),
        ];
        let sha = "ab".repeat(32);
        let out = write(&src, &list, &sha).unwrap();
        let doc = load(&out).unwrap();
        let pages = doc.get_pages();
        let p1 = annots_of(&doc, pages[&1]);
        assert_eq!(p1.len(), 3, "a1, a2 highlight, a2 sticky");
        let hl = annot_dict(&doc, &p1[0]).unwrap();
        assert_eq!(hl.get(b"Subtype").unwrap().as_name().unwrap(), b"Highlight");
        assert_eq!(hl.get(b"QuadPoints").unwrap().as_array().unwrap().len(), 16);
        assert!(hl.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap().as_reference().is_ok());
        assert_eq!(text_of(&doc, hl, b"NM").as_deref(), Some("lr:a1"));
        let sticky = annot_dict(&doc, &p1[2]).unwrap();
        assert_eq!(sticky.get(b"Subtype").unwrap().as_name().unwrap(), b"Text");
        assert_eq!(text_of(&doc, sticky, b"Contents").as_deref(), Some("要再查：Dice 與 Jaccard 的關係"));
        assert!(text_of(&doc, annot_dict(&doc, &p1[1]).unwrap(), b"Contents").is_none(), "the note is on the sticky only");
        // Quads of the first rect, page 1 (Letter, no rotation): TL, TR, BL, BR.
        let q: Vec<f64> = hl.get(b"QuadPoints").unwrap().as_array().unwrap().iter().map(|x| x.as_float().unwrap() as f64).collect();
        let want = [61.2, 633.6, 306.0, 633.6, 61.2, 609.84, 306.0, 609.84];
        assert!(q[..8].iter().zip(want).all(|(a, b)| (a - b).abs() < 0.01), "{q:?}");
        // Rotated page 2: displayed top-left (0.2, 0.3) → user (0.3·612, 0.2·792).
        let p2 = annots_of(&doc, pages[&2]);
        let h2 = annot_dict(&doc, &p2[0]).unwrap();
        let q2: Vec<f64> = h2.get(b"QuadPoints").unwrap().as_array().unwrap().iter().map(|x| x.as_float().unwrap() as f64).collect();
        assert!((q2[0] - 183.6).abs() < 0.01 && (q2[1] - 158.4).abs() < 0.01, "{q2:?}");

        let e = read(&out).unwrap();
        assert_eq!(e.source.as_deref(), Some(sha.as_str()));
        assert_eq!(e.annotations.len(), 4, "page 9 does not exist");
        for (a, b) in e.annotations.iter().zip(&list) {
            assert_eq!(a, b);
        }

        // Re-export the export with a changed list: no duplicates, the foreign annotation stays.
        let mut doc = load(&out).unwrap();
        let p1 = doc.get_pages()[&1];
        let foreign = doc.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Square", "Rect" => reals(&[1.0, 1.0, 9.0, 9.0]), "NM" => text_string("other-tool") });
        let mut l = annots_of(&doc, p1);
        l.push(foreign.into());
        doc.get_dictionary_mut(p1).unwrap().set("Annots", l);
        let mut with_foreign = vec![];
        doc.save_to(&mut with_foreign).unwrap();
        let again = write(&with_foreign, &list[1..2], &sha).unwrap();
        let doc = load(&again).unwrap();
        let p1 = annots_of(&doc, doc.get_pages()[&1]);
        assert_eq!(p1.len(), 3, "the foreign square + a2's highlight and sticky");
        assert!(annots_of(&doc, doc.get_pages()[&2]).is_empty());
        assert_eq!(read(&again).unwrap().annotations, list[1..2].to_vec());

        // A note edited in another viewer comes back edited; JSON-less annotations are rebuilt.
        let mut doc = load(&again).unwrap();
        let ids: Vec<ObjectId> = annots_of(&doc, doc.get_pages()[&1]).iter().filter_map(|o| o.as_reference().ok()).collect();
        for id in ids {
            let d = doc.get_dictionary_mut(id).unwrap();
            d.remove(DATA_KEY.as_bytes());
            if d.get(b"Subtype").unwrap().as_name().unwrap() == b"Text" {
                d.set("Contents", text_string("改過的註記"));
            }
        }
        let mut edited = vec![];
        doc.save_to(&mut edited).unwrap();
        let e = read(&edited).unwrap();
        assert_eq!(e.annotations.len(), 1);
        let a = &e.annotations[0];
        assert_eq!((a.id.as_str(), a.kind.as_str(), a.color.as_str(), a.note.as_str()), ("a2", "note", "green", "改過的註記"));
        assert!(a.rects[0].iter().zip([0.4, 0.5, 0.7, 0.53]).all(|(x, y)| (x - y).abs() < 1e-3), "{:?}", a.rects);

        // strip removes ours, keeps the foreign one and the marker.
        let s = strip(&again).unwrap();
        let e = read(&s).unwrap();
        assert!(e.annotations.is_empty());
        assert_eq!(e.source.as_deref(), Some(sha.as_str()));
        let doc = load(&s).unwrap();
        assert_eq!(annots_of(&doc, doc.get_pages()[&1]).len(), 1);
        // The source is untouched; a plain PDF carries nothing.
        assert_eq!(src, rotated_demo());
        assert!(read(&crate::synth::demo_paper()).unwrap().is_empty());
    }

    fn text_note(id: &str, page: u32, r: [f64; 4], text: &str) -> Annotation {
        serde_json::from_value(serde_json::json!({
            "id": id, "page": page, "kind": "text", "rects": [r], "note": text,
            "created_at": "2026-11-11T09:00:00+08:00", "updated_at": "2026-11-11T09:00:00+08:00",
        }))
        .unwrap()
    }

    #[test]
    fn wrap_breaks_latin_at_spaces_and_cjk_anywhere() {
        assert_eq!(wrap("", 100.0), vec![""]);
        assert_eq!(wrap("a\n\nb", 100.0), vec!["a", "", "b"]);
        // 10 pt: "hello " ≈ 24.5 pt, so two words fit in 55 pt, three do not.
        assert_eq!(wrap("hello hello hello", 55.0), vec!["hello hello", "hello"]);
        // CJK: 10 pt per character.
        assert_eq!(wrap("一二三四五六七", 30.0), vec!["一二三", "四五六", "七"]);
        assert_eq!(wrap("看 Dice 係數", 45.0), vec!["看 Dice", "係數"]);
        // A word wider than the box is cut.
        let w = wrap("Supercalifragilistic", 40.0);
        assert!(w.len() > 1 && w.iter().all(|l| text_width(l) <= 40.0), "{w:?}");
        assert_eq!(w.concat(), "Supercalifragilistic");
    }

    /// Page `n` of `pdf` rendered by poppler at 72 dpi (1 px = 1 pt of the displayed page),
    /// as (width, height, rgb), or None when pdftoppm is not installed.
    fn render(pdf: &[u8], n: u32) -> Option<(usize, usize, Vec<u8>)> {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.pdf");
        std::fs::write(&f, pdf).unwrap();
        let out = dir.path().join("p");
        let ok = std::process::Command::new("pdftoppm")
            .args(["-r", "72", "-cropbox", "-f", &n.to_string(), "-l", &n.to_string(), "-singlefile"])
            .arg(&f)
            .arg(&out)
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?
            .success();
        assert!(ok, "pdftoppm failed");
        let b = std::fs::read(dir.path().join("p.ppm")).unwrap();
        // P6\n<w> <h>\n255\n<rgb…>
        let mut fields = vec![];
        let mut i = 0;
        while fields.len() < 4 {
            while b[i].is_ascii_whitespace() {
                i += 1;
            }
            let s = i;
            while !b[i].is_ascii_whitespace() {
                i += 1;
            }
            fields.push(String::from_utf8_lossy(&b[s..i]).to_string());
        }
        let (w, h) = (fields[1].parse().unwrap(), fields[2].parse().unwrap());
        Some((w, h, b[i + 1..].to_vec()))
    }

    /// Ink pixels (dark grey) inside the displayed rect `r` (fractions), shrunk by 2 px, and
    /// their bounding box in px relative to the rect's top-left; plus whether the note fill shows.
    fn ink_in(img: &(usize, usize, Vec<u8>), r: [f64; 4]) -> (usize, [usize; 4], bool) {
        let (w, h, px) = img;
        let (x0, y0) = ((r[0] * *w as f64) as usize + 2, (r[1] * *h as f64) as usize + 2);
        let (x1, y1) = ((r[2] * *w as f64) as usize - 2, (r[3] * *h as f64) as usize - 2);
        let (mut n, mut bb, mut fill) = (0, [usize::MAX, usize::MAX, 0, 0], false);
        for y in y0..y1.min(*h) {
            for x in x0..x1.min(*w) {
                let p = &px[(y * w + x) * 3..][..3];
                if p.iter().all(|&c| c < 110) {
                    n += 1;
                    bb = [bb[0].min(x - x0), bb[1].min(y - y0), bb[2].max(x - x0), bb[3].max(y - y0)];
                }
                fill |= p[0] > 245 && p[1] > 235 && (180..225).contains(&p[2]);
            }
        }
        (n, bb, fill)
    }

    #[test]
    fn text_notes_export_as_freetext_with_appearance() {
        let src = rotated_demo();
        let list = vec![
            text_note("t1", 1, [0.1, 0.86, 0.5, 0.9], "Check the Dice proof"),
            text_note("t2", 1, [0.55, 0.86, 0.9, 0.9], "要再查證明"),
            text_note("t3", 2, [0.1, 0.1, 0.45, 0.16], "旋轉頁 rotate"),
            text_note("t4", 3, [0.2, 0.8, 0.6, 0.86], "裁切與二七〇"),
            text_note("t5", 1, [0.1, 0.1, 0.2, 0.12], "  "),
        ];
        let sha = "cd".repeat(32);
        let out = write(&src, &list, &sha).unwrap();
        let doc = load(&out).unwrap();
        let pages = doc.get_pages();
        let p1 = annots_of(&doc, pages[&1]);
        assert_eq!(p1.len(), 2, "the blank note is not written");
        let d = annot_dict(&doc, &p1[1]).unwrap();
        assert_eq!(d.get(b"Subtype").unwrap().as_name().unwrap(), b"FreeText");
        assert_eq!(text_of(&doc, d, b"NM").as_deref(), Some("lr:t2"));
        assert_eq!(text_of(&doc, d, b"Contents").as_deref(), Some("要再查證明"));
        assert!(text_of(&doc, d, b"DA").unwrap().starts_with("/Helv 10 Tf"));
        let ap = doc.get_object(d.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap().as_reference().unwrap()).unwrap().as_stream().unwrap();
        let fonts = ap.dict.get(b"Resources").unwrap().as_dict().unwrap().get(b"Font").unwrap().as_dict().unwrap();
        let cjk = doc.get_dictionary(fonts.get(b"CJK").unwrap().as_reference().unwrap()).unwrap();
        assert_eq!(cjk.get(b"Encoding").unwrap().as_name().unwrap(), b"UniCNS-UTF16-H");
        // 要 = U+8981 in UTF-16BE inside the content.
        assert!(String::from_utf8_lossy(&ap.content).contains("/CJK 10 Tf <8981"), "{}", String::from_utf8_lossy(&ap.content));
        // Rect of t1 on the unrotated Letter page: x 61.2…, top at 792·(1 − 0.86).
        let r = rect4(&doc, annot_dict(&doc, &p1[0]).unwrap().get(b"Rect").ok()).unwrap();
        assert!((r[0] - 61.2).abs() < 0.01 && (r[3] - 110.88).abs() < 0.01, "{r:?}");
        // Rotated pages: the form turns with /Rotate.
        let m = |n: u32| {
            let a = annots_of(&doc, pages[&n]);
            let d = annot_dict(&doc, &a[0]).unwrap();
            let ap = doc.get_object(d.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap().as_reference().unwrap()).unwrap().as_stream().unwrap();
            ap.dict.get(b"Matrix").unwrap().as_array().unwrap().iter().map(|x| x.as_float().unwrap() as f64).collect::<Vec<_>>()
        };
        assert_eq!(m(2), vec![0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);
        assert_eq!(m(3), vec![0.0, -1.0, 1.0, 0.0, 0.0, 0.0]);

        // Round trip: exact through the JSON; without it, rebuilt from /Rect and /Contents.
        let e = read(&out).unwrap();
        assert_eq!(e.annotations, list[..4].to_vec());
        let mut doc = load(&out).unwrap();
        for (_, page) in doc.get_pages() {
            for o in annots_of(&doc, page) {
                let id = o.as_reference().unwrap();
                let d = doc.get_dictionary_mut(id).unwrap();
                d.remove(DATA_KEY.as_bytes());
                if text_of_raw(d, b"NM").as_deref() == Some("lr:t3") {
                    d.set("Contents", text_string("改過的文字"));
                }
            }
        }
        let mut bare = vec![];
        doc.save_to(&mut bare).unwrap();
        let e = read(&bare).unwrap();
        assert_eq!(e.annotations.len(), 4);
        for (a, b) in e.annotations.iter().zip(&list) {
            assert_eq!((a.id.as_str(), a.kind.as_str(), a.page), (b.id.as_str(), "text", b.page));
            // left/top/width as written (the height may have grown to fit the text)
            let (x, y) = (a.rects[0], b.rects[0]);
            assert!((x[0] - y[0]).abs() < 1e-3 && (x[1] - y[1]).abs() < 1e-3 && (x[2] - y[2]).abs() < 1e-3, "{x:?} vs {y:?}");
        }
        assert_eq!(e.annotations[2].note, "改過的文字");
        // Re-export replaces, strip removes.
        assert_eq!(read(&write(&out, &list[..1], &sha).unwrap()).unwrap().annotations, list[..1].to_vec());
        assert!(read(&strip(&out).unwrap()).unwrap().annotations.is_empty());

        // What a viewer shows (poppler; Preview uses the same /AP): ink inside every box,
        // one upright line at the top-left for the single-line notes.
        // CJK needs poppler-data's Adobe-CNS1 cMaps and a CJK system font: checked on macOS
        // (Homebrew poppler) or with LR_RENDER_CHECK=1, skipped elsewhere.
        if !cfg!(target_os = "macos") && std::env::var_os("LR_RENDER_CHECK").is_none() {
            eprintln!("rendering check skipped (not macOS; LR_RENDER_CHECK=1 to run it)");
            return;
        }
        let Some(img1) = render(&out, 1) else {
            eprintln!("pdftoppm not installed: rendering check skipped");
            return;
        };
        let geom = |n: u32| page_geom(&doc, doc.get_pages()[&n]).unwrap();
        for (img, a) in [(&img1, &list[0]), (&img1, &list[1]), (&render(&out, 2).unwrap(), &list[2]), (&render(&out, 3).unwrap(), &list[3])] {
            assert_eq!((img.0, img.1), { let (w, h) = geom(a.page).display_size(); (w.round() as usize, h.round() as usize) });
            let (r, lines) = text_layout(&geom(a.page), a.rects[0], &a.note);
            assert_eq!(lines.len(), 1, "{}", a.note);
            let (n, bb, fill) = ink_in(img, r);
            assert!(fill, "{}: no note fill", a.id);
            assert!(n > 25, "{}: {n} ink pixels", a.id);
            assert!(bb[2] - bb[0] > 3 * (bb[3] - bb[1]), "{}: not one horizontal line {bb:?}", a.id);
            assert!(bb[0] < 6 && bb[1] < 8, "{}: text not at the top-left {bb:?}", a.id);
        }
        // Nothing there in the plain PDF's render of the CJK box: the ink is the note's.
        let (n0, _, fill0) = ink_in(&render(&src, 1).unwrap(), text_layout(&geom(1), list[1].rects[0], &list[1].note).0);
        assert!(n0 == 0 && !fill0, "{n0}");
    }

    fn text_of_raw(d: &Dictionary, k: &[u8]) -> Option<String> {
        Some(decode_text_string(d.get(k).ok()?.as_str().ok()?))
    }
}
