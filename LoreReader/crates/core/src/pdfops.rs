//! Non-destructive page operations. Every operation is a [`compose`]: a new PDF whose
//! pages are a list of (source PDF, page number) picks. The sources are never modified.
//!
//! - delete pages: every page but those
//! - duplicate a page: that page twice
//! - reorder: the pages in the new order
//! - insert pages from another PDF at a position: picks from two sources
//! - merge: all pages of several sources
//!
//! Each picked page becomes a new page object (a copy of the source page dictionary with
//! its inherited `Resources` / `MediaBox` / `CropBox` / `Rotate` made explicit) under one
//! new page tree, so a page used twice is two valid pages that share their content
//! streams and resources. Outlines, named destinations and forms of the sources are not
//! carried over (v0); unreachable objects are pruned.

use std::collections::BTreeMap;

use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};

use crate::{Error, Result};

/// One page of the output: `page` (1-based) of `sources[src]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Pick {
    pub src: usize,
    pub page: u32,
}

pub(crate) fn load(bytes: &[u8]) -> Result<Document> {
    let doc = Document::load_mem(bytes)?;
    if doc.is_encrypted() {
        return Err(Error::Pdf("這份 PDF 有加密，不能做頁面操作".into()));
    }
    Ok(doc)
}

pub fn page_count(bytes: &[u8]) -> Result<u32> {
    Ok(load(bytes)?.get_pages().len() as u32)
}

/// Plain text of one page (lopdf's extractor; good enough for synthetic PDFs, DOIs and
/// checks — the reader itself uses pdf.js's text layer).
pub fn page_text(bytes: &[u8], page: u32) -> Result<String> {
    Ok(load(bytes)?.extract_text(&[page])?)
}

/// `/Title` from the document info, when it is a non-empty string.
pub fn info_title(bytes: &[u8]) -> Option<String> {
    let doc = Document::load_mem(bytes).ok()?;
    let info = match doc.trailer.get(b"Info").ok()? {
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        Object::Dictionary(d) => d,
        _ => return None,
    };
    let raw = info.get(b"Title").ok()?.as_str().ok()?;
    let t = decode_text_string(raw);
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// PDF text string: UTF-16BE with BOM, else PDFDocEncoding (≈ Latin-1).
pub(crate) fn decode_text_string(raw: &[u8]) -> String {
    if raw.starts_with(&[0xFE, 0xFF]) {
        let u: Vec<u16> = raw[2..].chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&u)
    } else if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(&raw[3..]).into_owned()
    } else {
        raw.iter().map(|&b| b as char).collect()
    }
}

/// The first DOI in `text` (`10.<4–9 digits>/<suffix>`), trailing punctuation trimmed.
pub fn find_doi(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(off) = text[i..].find("10.") {
        let s = i + off;
        let mut j = s + 3;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        let digits = j - (s + 3);
        if (4..=9).contains(&digits) && j < b.len() && b[j] == b'/' {
            let mut k = j + 1;
            while k < b.len() && !b[k].is_ascii_whitespace() && b[k] != b'"' && b[k] != b'<' && b[k] != b'>' {
                k += 1;
            }
            // Text extraction may glue the next line on ("…2026Abstract"): cut where a
            // lowercase letter or digit meets a capitalised word.
            let sb = &b[..k];
            let mut cut = k;
            for m in j + 2..k.saturating_sub(1) {
                if (sb[m - 1].is_ascii_lowercase() || sb[m - 1].is_ascii_digit()) && sb[m].is_ascii_uppercase() && sb[m + 1].is_ascii_lowercase() {
                    cut = m;
                    break;
                }
            }
            let doi = text[s..cut].trim_end_matches(['.', ',', ';', ')', ']']);
            if doi.len() > j - s + 1 {
                return Some(doi.to_string());
            }
        }
        i = s + 3;
    }
    None
}

/// DOI from the first two pages.
pub fn doi_of(bytes: &[u8]) -> Option<String> {
    let doc = Document::load_mem(bytes).ok()?;
    let n = doc.get_pages().len() as u32;
    (1..=n.min(2)).find_map(|p| doc.extract_text(&[p]).ok().and_then(|t| find_doi(&t)))
}

const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// The page dictionary with inherited attributes copied in from its ancestors.
pub(crate) fn flattened_page(doc: &Document, id: ObjectId) -> Result<Dictionary> {
    let mut page = doc.get_dictionary(id)?.clone();
    let mut parent = page.get(b"Parent").ok().and_then(|p| p.as_reference().ok());
    let mut guard = 0;
    while let Some(pid) = parent {
        guard += 1;
        if guard > 64 {
            break;
        }
        let Ok(node) = doc.get_dictionary(pid) else { break };
        for key in INHERITED {
            if !page.has(key)
                && let Ok(v) = node.get(key)
            {
                page.set(key.to_vec(), v.clone());
            }
        }
        parent = node.get(b"Parent").ok().and_then(|p| p.as_reference().ok());
    }
    if !page.has(b"MediaBox") {
        page.set("MediaBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
    }
    if !page.has(b"Resources") {
        page.set("Resources", dictionary! {});
    }
    Ok(page)
}

/// A new PDF made of `picks` from `sources`.
pub fn compose(sources: &[&[u8]], picks: &[Pick]) -> Result<Vec<u8>> {
    if picks.is_empty() {
        return Err(Error::Invalid("新的 PDF 至少要有一頁".into()));
    }
    let mut out = Document::with_version("1.7");
    let mut next_id = 1;
    // Per source: page number → object id in `out`.
    let mut page_maps: Vec<BTreeMap<u32, ObjectId>> = Vec::new();
    for (i, bytes) in sources.iter().enumerate() {
        let mut doc = load(bytes).map_err(|e| Error::Pdf(format!("第 {} 份來源：{e}", i + 1)))?;
        doc.renumber_objects_with(next_id);
        next_id = doc.max_id + 1;
        page_maps.push(doc.get_pages());
        for (id, obj) in doc.objects {
            out.objects.insert(id, obj);
        }
    }
    out.max_id = next_id;
    let pages_id = out.new_object_id();
    let mut kids: Vec<Object> = Vec::with_capacity(picks.len());
    for (n, p) in picks.iter().enumerate() {
        let map = page_maps.get(p.src).ok_or_else(|| Error::Invalid(format!("第 {} 頁：沒有第 {} 份來源", n + 1, p.src + 1)))?;
        let src_id = *map.get(&p.page).ok_or_else(|| Error::Invalid(format!("來源 {} 沒有第 {} 頁（共 {} 頁）", p.src + 1, p.page, map.len())))?;
        let mut page = flattened_page(&out, src_id)?;
        page.set("Parent", pages_id);
        // Annotations stay with the page but lose their back-pointer to the old page.
        if let Ok(Object::Array(annots)) = page.get(b"Annots").cloned() {
            for a in annots.iter().filter_map(|a| a.as_reference().ok()) {
                if let Ok(d) = out.get_dictionary_mut(a) {
                    d.remove(b"P");
                }
            }
        }
        kids.push(out.add_object(page).into());
    }
    let count = kids.len() as i64;
    out.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = out.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    out.trailer.set("Root", catalog);
    out.prune_objects();
    out.renumber_objects();
    let mut bytes = Vec::new();
    out.save_to(&mut bytes)?;
    Ok(bytes)
}

/// Every page of `bytes` except `delete` (1-based).
pub fn without_pages(n: u32, delete: &[u32]) -> Vec<Pick> {
    (1..=n).filter(|p| !delete.contains(p)).map(|page| Pick { src: 0, page }).collect()
}

/// All pages, with `page` repeated right after itself.
pub fn with_duplicate(n: u32, page: u32) -> Vec<Pick> {
    (1..=n).flat_map(|p| if p == page { vec![Pick { src: 0, page: p }; 2] } else { vec![Pick { src: 0, page: p }] }).collect()
}

/// Source 0's pages with `pages` of source 1 inserted after page `after` (0 = at the start).
pub fn with_inserted(n: u32, after: u32, pages: &[u32]) -> Vec<Pick> {
    let mut out: Vec<Pick> = (1..=after.min(n)).map(|page| Pick { src: 0, page }).collect();
    out.extend(pages.iter().map(|&page| Pick { src: 1, page }));
    out.extend((after.min(n) + 1..=n).map(|page| Pick { src: 0, page }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::numbered_pdf;

    fn labels(bytes: &[u8]) -> Vec<String> {
        let n = page_count(bytes).unwrap();
        (1..=n).map(|p| page_text(bytes, p).unwrap().split_whitespace().collect::<Vec<_>>().join(" ")).collect()
    }

    #[test]
    fn delete_duplicate_reorder_insert_merge() {
        let a = numbered_pdf(4, "A");
        let b = numbered_pdf(3, "B");
        let del = compose(&[&a], &without_pages(4, &[2, 4])).unwrap();
        assert_eq!(labels(&del), ["A Page 1", "A Page 3"]);
        let dup = compose(&[&a], &with_duplicate(4, 2)).unwrap();
        assert_eq!(labels(&dup), ["A Page 1", "A Page 2", "A Page 2", "A Page 3", "A Page 4"]);
        let order: Vec<Pick> = [4, 1, 3, 2].iter().map(|&page| Pick { src: 0, page }).collect();
        assert_eq!(labels(&compose(&[&a], &order).unwrap()), ["A Page 4", "A Page 1", "A Page 3", "A Page 2"]);
        let ins = compose(&[&a, &b], &with_inserted(4, 1, &[3, 1])).unwrap();
        assert_eq!(labels(&ins), ["A Page 1", "B Page 3", "B Page 1", "A Page 2", "A Page 3", "A Page 4"]);
        let all: Vec<Pick> = (1..=4).map(|page| Pick { src: 0, page }).chain((1..=3).map(|page| Pick { src: 1, page })).collect();
        let merged = compose(&[&a, &b], &all).unwrap();
        assert_eq!(page_count(&merged).unwrap(), 7);
        assert_eq!(labels(&merged)[4], "B Page 1");
        // The sources are untouched and the output reloads cleanly.
        assert_eq!(a, numbered_pdf(4, "A"));
        assert!(Document::load_mem(&merged).unwrap().get_pages().values().all(|_| true));
    }

    #[test]
    fn bad_picks_are_refused() {
        let a = numbered_pdf(2, "A");
        assert!(compose(&[&a], &[]).is_err());
        assert!(compose(&[&a], &[Pick { src: 0, page: 3 }]).unwrap_err().to_string().contains("第 3 頁"));
        assert!(compose(&[&a], &[Pick { src: 1, page: 1 }]).is_err());
        assert!(compose(&[b"not a pdf"], &[Pick { src: 0, page: 1 }]).is_err());
    }

    #[test]
    fn title_and_doi() {
        let d = crate::synth::demo_paper();
        assert_eq!(info_title(&d).as_deref(), Some("A Synthetic Study of Overlap Metrics"));
        assert_eq!(doi_of(&d).as_deref(), Some("10.5555/lorereader.demo.2026"));
        assert_eq!(find_doi("see https://doi.org/10.1000/xyz123)."), Some("10.1000/xyz123".into()));
        assert_eq!(find_doi("version 10.2 of 10.12/ab"), None);
        assert_eq!(find_doi("10.1109/TMI.2020.12345Introduction"), Some("10.1109/TMI.2020.12345".into()));
        assert_eq!(page_count(&d).unwrap(), 3);
    }
}
