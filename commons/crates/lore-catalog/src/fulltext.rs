//! The whole text of a file, in parts (a PDF's pages, a slide deck's slides, a Word document's
//! or a text file's blocks of paragraphs), cached per content in the cache tier
//! (`<cache>/LoreForge/shelf/fulltext/<sha256>.json`, wipeable). Filled by `loreshelf extract`
//! (separate from the scan: it is the slow part), read by the search index and the term pool.
//!
//! Never written into the catalog or any repo: the text of the author's files stays in the cache.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Files larger than this are not read.
pub const MAX_BYTES: u64 = 200 << 20;
/// Pages (or slides, or blocks) kept per file.
pub const MAX_PARTS: usize = 600;
/// Characters kept per file.
pub const MAX_CHARS: usize = 600_000;
/// A block of a Word document or a text file: about this many characters.
pub const BLOCK_CHARS: usize = 1_500;
/// Bumped when the reading changes: older files are read again.
pub const VERSION: u32 = 1;
/// Written into the folder when a run of [`fill`] has finished: the search index rebuilds after it.
pub const STAMP: &str = ".stamp";

/// One page, slide or block. `n` counts from 1 in its own unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub n: u32,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FullText {
    #[serde(default)]
    pub version: u32,
    /// `page`, `slide` or `block`.
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub parts: Vec<Part>,
    /// Why there is no text (`too large`, `unreadable`, `no text`), when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl FullText {
    pub fn dir(cache_dir: &Path) -> PathBuf {
        cache_dir.join("fulltext")
    }

    fn file(dir: &Path, sha: &str) -> PathBuf {
        dir.join(format!("{sha}.json"))
    }

    pub fn load(dir: &Path, sha: &str) -> Option<FullText> {
        serde_json::from_slice(&std::fs::read(Self::file(dir, sha)).ok()?).ok()
    }

    pub fn save(&self, dir: &Path, sha: &str) -> Result<(), crate::CatalogError> {
        let p = Self::file(dir, sha);
        let b = serde_json::to_vec(self).map_err(|e| crate::CatalogError::io(&p, std::io::Error::other(e)))?;
        crate::store::write_atomic(&p, &b)
    }

    /// All the parts' text, joined.
    pub fn joined(&self) -> String {
        self.parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    fn none(unit: &str, why: &str) -> FullText {
        FullText { version: VERSION, unit: unit.into(), parts: Vec::new(), note: Some(why.into()) }
    }
}

/// Is there anything to read in a file of this kind and name?
pub fn readable(kind: &str, name: &str) -> bool {
    if crate::category::secret_name(kind, name) {
        return false;
    }
    let ext = name.rsplit_once('.').map(|x| x.1.to_lowercase()).unwrap_or_default();
    match kind {
        "pdf" => true,
        "slides" => ext == "pptx" || ext == "pptm" || ext == "potx",
        "document" => ext == "docx" || ext == "docm" || ext == "dotx",
        "text" | "latex" => matches!(ext.as_str(), "txt" | "md" | "markdown" | "tex" | "bib"),
        _ => false,
    }
}

/// Read a file's whole text (never panics; a broken file gives a note instead of parts).
pub fn read(path: &Path, kind: &str, size: u64) -> FullText {
    let unit = match kind {
        "pdf" => "page",
        "slides" => "slide",
        _ => "block",
    };
    if size > MAX_BYTES {
        return FullText::none(unit, "too large");
    }
    let p = path.to_path_buf();
    let k = kind.to_string();
    let r = std::panic::catch_unwind(move || match k.as_str() {
        "pdf" => pdf_pages(&p),
        "slides" => pptx_slides(&p),
        "document" => docx_blocks(&p),
        _ => text_blocks(&p),
    });
    match r {
        Ok(Some(parts)) => {
            let parts = cap(parts);
            if parts.iter().all(|p| p.text.trim().is_empty()) {
                FullText::none(unit, "no text")
            } else {
                FullText { version: VERSION, unit: unit.into(), parts, note: None }
            }
        }
        _ => FullText::none(unit, "unreadable"),
    }
}

/// At most [`MAX_PARTS`] parts and [`MAX_CHARS`] characters; empty parts dropped; whitespace runs folded.
fn cap(parts: Vec<Part>) -> Vec<Part> {
    let mut left = MAX_CHARS;
    let mut out = Vec::new();
    for p in parts.into_iter().take(MAX_PARTS) {
        let t = fold(&p.text);
        if t.is_empty() {
            continue;
        }
        let n = t.chars().count();
        if n >= left {
            out.push(Part { n: p.n, text: t.chars().take(left).collect() });
            break;
        }
        left -= n;
        out.push(Part { n: p.n, text: t });
    }
    out
}

fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    out
}

fn pdf_pages(path: &Path) -> Option<Vec<Part>> {
    let doc = lopdf::Document::load(path).ok()?;
    let n = doc.get_pages().len() as u32;
    let mut out = Vec::new();
    for page in 1..=n.min(MAX_PARTS as u32) {
        let text = doc.extract_text(&[page]).unwrap_or_default();
        out.push(Part { n: page, text });
    }
    Some(out)
}

/// The text runs (`<a:t>` in slides, `<w:t>` in Word) of one XML part; `para` = the paragraph
/// element after which a line ends.
fn xml_text(xml: &str, run: &str, para: &str) -> String {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let (mut out, mut inside) = (String::new(), false);
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) if e.local_name().as_ref() == run => inside = true,
            Ok(Event::End(e)) if e.local_name().as_ref() == run => inside = false,
            Ok(Event::End(e)) if e.local_name().as_ref() == para => out.push('\n'),
            Ok(Event::Empty(e)) if matches!(e.local_name().as_ref(), "tab" | "br") => out.push(' '),
            Ok(Event::Text(t)) if inside => out.push_str(&t.xml10_content()),
            // `&amp;`, `&#x4E00;` …: entities come as their own events
            Ok(Event::GeneralRef(g)) if inside => {
                let name = g.xml10_content();
                match name.as_ref() {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    n => {
                        let code = n.strip_prefix("#x").and_then(|h| u32::from_str_radix(h, 16).ok()).or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()));
                        if let Some(c) = code.and_then(char::from_u32) {
                            out.push(c);
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn zip_entry(z: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut f = z.by_name(name).ok()?;
    let mut s = String::new();
    f.by_ref().take(64 << 20).read_to_string(&mut s).ok()?;
    Some(s)
}

fn pptx_slides(path: &Path) -> Option<Vec<Part>> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    // ppt/slides/slideN.xml, in N order (the deck's own order is in presentation.xml; N is close
    // enough for "which slide" and never wrong about the text)
    let mut ns: Vec<u32> = (0..z.len())
        .filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string()))
        .filter_map(|n| n.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok())
        .collect();
    ns.sort_unstable();
    let mut out = Vec::new();
    for n in ns {
        let mut text = zip_entry(&mut z, &format!("ppt/slides/slide{n}.xml")).map(|x| xml_text(&x, "t", "p")).unwrap_or_default();
        if let Some(notes) = zip_entry(&mut z, &format!("ppt/notesSlides/notesSlide{n}.xml")).map(|x| xml_text(&x, "t", "p")) {
            text.push('\n');
            text.push_str(&notes);
        }
        out.push(Part { n, text });
    }
    Some(out)
}

fn docx_blocks(path: &Path) -> Option<Vec<Part>> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let xml = zip_entry(&mut z, "word/document.xml")?;
    Some(blocks(&xml_text(&xml, "t", "p")))
}

fn text_blocks(path: &Path) -> Option<Vec<Part>> {
    let mut b = Vec::new();
    std::fs::File::open(path).ok()?.take((MAX_CHARS * 4) as u64).read_to_end(&mut b).ok()?;
    Some(blocks(&String::from_utf8_lossy(&b)))
}

/// Paragraphs gathered into blocks of about [`BLOCK_CHARS`] characters.
fn blocks(text: &str) -> Vec<Part> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        cur.push_str(line);
        cur.push('\n');
        if cur.chars().count() >= BLOCK_CHARS {
            out.push(Part { n: out.len() as u32 + 1, text: std::mem::take(&mut cur) });
        }
    }
    if !cur.trim().is_empty() {
        out.push(Part { n: out.len() as u32 + 1, text: cur });
    }
    out
}

/// What [`fill`] did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Filled {
    pub read: usize,
    pub with_text: usize,
    pub already: usize,
    pub missing: usize,
}

/// Read every catalogued file that has text and is not read yet (or was read by an older
/// [`VERSION`]); `progress(done, total)` after each. Only reads the files (never writes next to them).
pub fn fill(catalog_dir: &Path, cache_dir: &Path, mut progress: impl FnMut(usize, usize)) -> Result<Filled, crate::CatalogError> {
    let dir = FullText::dir(cache_dir);
    std::fs::create_dir_all(&dir).map_err(|e| crate::CatalogError::io(&dir, e))?;
    let files = crate::store::Catalog { dir: catalog_dir.to_path_buf() }.files_dir();
    let mut todo = Vec::new();
    let mut out = Filled::default();
    for e in std::fs::read_dir(&files).map_err(|e| crate::CatalogError::io(&files, e))?.flatten() {
        let Some(sha) = e.file_name().to_str().and_then(|n| n.strip_suffix(".json")).map(str::to_string) else { continue };
        let Some(rec) = crate::FileRecord::load(&files, &sha) else { continue };
        let name = rec.names.first().cloned().unwrap_or_default();
        if !readable(&rec.kind, &name) {
            // a secret read by an older version leaves the cache
            if rec.names.iter().any(|n| crate::category::secret_name(&rec.kind, n)) {
                let _ = std::fs::remove_file(FullText::file(&dir, &sha));
            }
            continue;
        }
        if FullText::load(&dir, &sha).is_some_and(|f| f.version >= VERSION) {
            out.already += 1;
            continue;
        }
        todo.push(rec);
    }
    let total = todo.len();
    for (i, rec) in todo.iter().enumerate() {
        // the first path that is still there and not a cloud placeholder
        let path = rec.seen_at.iter().map(|s| PathBuf::from(&s.path)).find(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file() && !crate::scan::is_dataless(&m)));
        match path {
            Some(p) => {
                let ft = read(&p, &rec.kind, rec.size);
                if !ft.parts.is_empty() {
                    out.with_text += 1;
                }
                ft.save(&dir, &rec.sha256)?;
                out.read += 1;
            }
            None => out.missing += 1,
        }
        progress(i + 1, total);
    }
    crate::store::write_atomic(&dir.join(STAMP), chrono::Utc::now().to_rfc3339().as_bytes())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(path: &Path, files: &[(&str, &str)]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, body) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn slides_in_order_with_their_notes() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("talk.pptx");
        let slide = |s: &str| format!(r#"<p:sld xmlns:a="a" xmlns:p="p"><p:txBody><a:p><a:r><a:t>{s}</a:t></a:r></a:p><a:p><a:r><a:t>second line</a:t></a:r></a:p></p:txBody></p:sld>"#);
        zip_with(&p, &[("ppt/slides/slide10.xml", &slide("Gait cycle")), ("ppt/slides/slide2.xml", &slide("Ground reaction force")), ("ppt/notesSlides/notesSlide2.xml", r#"<p:notes xmlns:a="a" xmlns:p="p"><a:p><a:r><a:t>say the 力學 part</a:t></a:r></a:p></p:notes>"#)]);
        let ft = read(&p, "slides", 1);
        assert_eq!(ft.unit, "slide");
        assert_eq!(ft.parts.iter().map(|p| p.n).collect::<Vec<_>>(), vec![2, 10]);
        assert_eq!(ft.parts[0].text, "Ground reaction force second line say the 力學 part");
    }

    #[test]
    fn word_paragraphs_become_blocks() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("notes.docx");
        let para = |s: &str| format!("<w:p><w:r><w:t>{s}</w:t></w:r><w:r><w:tab/><w:t xml:space=\"preserve\">tail</w:t></w:r></w:p>");
        let body: String = (0..200).map(|i| para(&format!("段落{i} 肌腱 tendon &amp; ligament"))).collect();
        zip_with(&p, &[("word/document.xml", &format!(r#"<w:document xmlns:w="w"><w:body>{body}</w:body></w:document>"#))]);
        let ft = read(&p, "document", 1);
        assert_eq!(ft.unit, "block");
        assert!(ft.parts.len() >= 3, "{}", ft.parts.len());
        assert!(ft.parts[0].text.starts_with("段落0 肌腱 tendon & ligament tail"));
        assert!(ft.joined().contains("段落199"));
    }

    #[test]
    fn text_files_and_broken_files() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a.md");
        std::fs::write(&p, "# Title\n\nsome   words\n").unwrap();
        assert_eq!(read(&p, "text", 20).parts, vec![Part { n: 1, text: "# Title some words".into() }]);
        let bad = t.path().join("bad.pptx");
        std::fs::write(&bad, "not a zip").unwrap();
        assert_eq!(read(&bad, "slides", 9).note.as_deref(), Some("unreadable"));
        assert_eq!(read(&bad, "pdf", MAX_BYTES + 1).note.as_deref(), Some("too large"));
        assert!(readable("slides", "x.PPTX") && !readable("slides", "x.key") && readable("latex", "a.tex") && !readable("image", "a.png"));
    }

    #[test]
    fn caps_and_folding() {
        let parts = (1..=MAX_PARTS as u32 + 5).map(|n| Part { n, text: format!("  p{n}\n\n x ") }).collect();
        let c = cap(parts);
        assert_eq!(c.len(), MAX_PARTS);
        assert_eq!(c[0].text, "p1 x");
        let big = vec![Part { n: 1, text: "字".repeat(MAX_CHARS + 10) }, Part { n: 2, text: "next".into() }];
        let c = cap(big);
        assert_eq!((c.len(), c[0].text.chars().count()), (1, MAX_CHARS));
    }
}
