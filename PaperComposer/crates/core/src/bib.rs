//! Reference library: BibTeX entries plus the writer's own notes.
//!
//! In the private workspace:
//! - `refs/library.bib`  every reference, standard BibTeX (the bibliographic truth)
//! - `refs/notes.csv`    per reference: summary, one-line note, tags, DOI, PDF file name
//! - `profile/tags.csv`  the tag vocabulary
//! - `papers/<p>/citations.csv`  what each reference is used for in that paper
//!
//! PDFs are not stored: only a file name, looked up in a folder each machine
//! configures for itself.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub kind: String,
    pub key: String,
    /// Lower-case field name → raw value (without the outer braces/quotes).
    pub fields: BTreeMap<String, String>,
}

impl Entry {
    pub fn get(&self, f: &str) -> Option<&str> {
        self.fields.get(f).map(|s| s.as_str())
    }
    /// Title without LaTeX braces.
    pub fn title(&self) -> String {
        clean(self.get("title").unwrap_or(""))
    }
    pub fn year(&self) -> String {
        self.get("year").unwrap_or("").to_string()
    }
    /// Journal or conference.
    pub fn venue(&self) -> String {
        clean(self.get("journal").or(self.get("booktitle")).or(self.get("publisher")).unwrap_or(""))
    }
    /// "Smith et al." style short author list.
    pub fn authors_short(&self) -> String {
        let a = self.get("author").unwrap_or("");
        let names: Vec<&str> = a.split(" and ").map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        let last = |n: &str| -> String {
            let n = clean(n);
            match n.split_once(',') {
                Some((l, _)) => l.trim().to_string(),
                None => n.split_whitespace().last().unwrap_or("").to_string(),
            }
        };
        match names.len() {
            0 => String::new(),
            1 => last(names[0]),
            2 => format!("{} & {}", last(names[0]), last(names[1])),
            _ => format!("{} et al.", last(names[0])),
        }
    }
}

/// Strip TeX braces and collapse spaces, for display.
pub fn clean(s: &str) -> String {
    s.replace(['{', '}'], "").split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── parsing ────────────────────────────────────────────

/// Parse BibTeX. `@string`, `@comment` and `@preamble` are skipped; a broken
/// entry is reported and the rest still parse.
pub fn parse(src: &str) -> (Vec<Entry>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let b = src.as_bytes();
    let mut i = 0;
    while let Some(at) = src[i..].find('@') {
        i += at + 1;
        let kind_end = src[i..].find(|c: char| !c.is_ascii_alphanumeric()).map(|k| i + k).unwrap_or(src.len());
        let kind = src[i..kind_end].to_lowercase();
        let mut j = kind_end;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= b.len() || (b[j] != b'{' && b[j] != b'(') {
            continue;
        }
        let close = if b[j] == b'{' { b'}' } else { b')' };
        // find the matching close, honouring nested braces
        let mut depth = 0i32;
        let mut end = None;
        for (k, &c) in b.iter().enumerate().skip(j) {
            if c == b'{' || (c == b'(' && close == b')' && k == j) {
                depth += 1;
            } else if c == b'}' || (c == close && close == b')') {
                depth -= 1;
                if depth == 0 {
                    end = Some(k);
                    break;
                }
            }
        }
        let Some(end) = end else {
            errors.push(format!("@{kind}: not closed"));
            break;
        };
        let body = &src[j + 1..end];
        i = end + 1;
        if matches!(kind.as_str(), "string" | "comment" | "preamble") {
            continue;
        }
        match parse_entry(&kind, body) {
            Ok(e) => out.push(e),
            Err(e) => errors.push(e),
        }
    }
    (out, errors)
}

fn parse_entry(kind: &str, body: &str) -> Result<Entry, String> {
    let (key, rest) = body.split_once(',').ok_or_else(|| format!("@{kind}: no key"))?;
    let key = key.trim().to_string();
    if key.is_empty() || key.contains(char::is_whitespace) {
        return Err(format!("@{kind}: bad key '{key}'"));
    }
    let mut fields = BTreeMap::new();
    let mut s = rest;
    loop {
        s = s.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        if s.is_empty() {
            break;
        }
        let eq = s.find('=').ok_or_else(|| format!("{key}: field without '='"))?;
        let name = s[..eq].trim().to_lowercase();
        let mut v = s[eq + 1..].trim_start();
        let mut value = String::new();
        // value: {..}, ".." or a bare word, joined with #
        loop {
            if v.starts_with('{') {
                let mut depth = 0;
                let mut end = 0;
                for (k, c) in v.char_indices() {
                    if c == '{' {
                        depth += 1;
                    } else if c == '}' {
                        depth -= 1;
                        if depth == 0 {
                            end = k;
                            break;
                        }
                    }
                }
                if end == 0 {
                    return Err(format!("{key}: unclosed brace in {name}"));
                }
                value += &v[1..end];
                v = &v[end + 1..];
            } else if let Some(rest) = v.strip_prefix('"') {
                let mut depth = 0;
                let mut end = None;
                for (k, c) in rest.char_indices() {
                    match c {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        '"' if depth == 0 => {
                            end = Some(k);
                            break;
                        }
                        _ => {}
                    }
                }
                let end = end.ok_or_else(|| format!("{key}: unclosed quote in {name}"))?;
                value += &rest[..end];
                v = &rest[end + 1..];
            } else {
                let end = v.find(|c: char| c == ',' || c == '#' || c.is_whitespace()).unwrap_or(v.len());
                value += &v[..end];
                v = &v[end..];
            }
            let t = v.trim_start();
            if let Some(more) = t.strip_prefix('#') {
                v = more.trim_start();
            } else {
                v = t;
                break;
            }
        }
        fields.insert(name, value.split_whitespace().collect::<Vec<_>>().join(" "));
        s = v;
    }
    Ok(Entry { kind: kind.to_string(), key, fields })
}

// ── writing ────────────────────────────────────────────

const FIELD_ORDER: &[&str] = &["author", "title", "journal", "booktitle", "year", "volume", "number", "pages", "publisher", "doi", "url"];

pub fn write_entry(e: &Entry) -> String {
    let mut names: Vec<&String> = e.fields.keys().collect();
    names.sort_by_key(|n| (FIELD_ORDER.iter().position(|o| o == n).unwrap_or(FIELD_ORDER.len()), n.to_string()));
    let mut out = format!("@{}{{{},\n", e.kind, e.key);
    for n in names {
        out += &format!("  {} = {{{}}},\n", n, e.fields[n]);
    }
    out += "}\n";
    out
}

pub fn write(entries: &[Entry]) -> String {
    entries.iter().map(write_entry).collect::<Vec<_>>().join("\n")
}

// ── notes, tags, citations ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Note {
    pub key: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub one_liner: String,
    /// Tag ids.
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub doi: String,
    /// File name inside this machine's PDF folder.
    #[serde(default)]
    pub pdf: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tag {
    pub id: String,
    pub label: String,
}

pub const DEFAULT_TAGS: &[(&str, &str)] = &[
    ("medical", "醫學"),
    ("classical_algorithm", "經典算法"),
    ("new_algorithm", "新算法"),
    ("traditional_application", "傳統應用"),
    ("new_application", "新應用"),
    ("top_conference", "頂會"),
    ("top_journal", "頂刊"),
    ("review", "回顧"),
    ("dataset", "資料集"),
    ("benchmark", "基準"),
    ("theory", "理論"),
    ("baseline", "比較基準"),
    ("sota", "SOTA"),
    ("open_code", "有程式碼"),
    ("clinical", "臨床"),
    ("our_prior_work", "我們的前作"),
];

fn csv_reader(path: &Path) -> Option<csv::Reader<std::fs::File>> {
    csv::ReaderBuilder::new().flexible(true).from_path(path).ok()
}

pub fn read_notes(path: &Path) -> Vec<Note> {
    let Some(mut r) = csv_reader(path) else { return Vec::new() };
    r.records()
        .flatten()
        .filter_map(|rec| {
            let f = |i: usize| rec.get(i).unwrap_or("").to_string();
            let key = f(0);
            (!key.is_empty()).then(|| Note {
                key,
                summary: f(1),
                one_liner: f(2),
                tags: f(3).split(';').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect(),
                doi: f(4),
                pdf: f(5),
            })
        })
        .collect()
}

pub fn write_notes(path: &Path, notes: &[Note]) -> std::io::Result<()> {
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(["key", "summary", "one_liner", "tags", "doi", "pdf"])?;
    for n in notes {
        w.write_record([&n.key, &n.summary, &n.one_liner, &n.tags.join(";"), &n.doi, &n.pdf])?;
    }
    let bytes = w.into_inner().map_err(|e| std::io::Error::other(e.to_string()))?;
    crate::paper::write_atomic(path, &String::from_utf8_lossy(&bytes))
}

pub fn read_tags(path: &Path) -> Vec<Tag> {
    let from_file: Vec<Tag> = csv_reader(path)
        .map(|mut r| {
            r.records()
                .flatten()
                .filter_map(|rec| {
                    let id = rec.get(0)?.trim().to_string();
                    (!id.is_empty()).then(|| Tag { label: rec.get(1).unwrap_or(&id).trim().to_string(), id })
                })
                .collect()
        })
        .unwrap_or_default();
    if from_file.is_empty() {
        DEFAULT_TAGS.iter().map(|(id, l)| Tag { id: id.to_string(), label: l.to_string() }).collect()
    } else {
        from_file
    }
}

pub fn write_default_tags(path: &Path) -> std::io::Result<()> {
    let mut s = String::from("id,label\n");
    for (id, l) in DEFAULT_TAGS {
        s += &format!("{id},{l}\n");
    }
    crate::paper::write_atomic(path, &s)
}

/// key → what it is used for in one paper.
pub fn read_citations(path: &Path) -> BTreeMap<String, String> {
    csv_reader(path)
        .map(|mut r| r.records().flatten().filter_map(|rec| Some((rec.get(0)?.to_string(), rec.get(1).unwrap_or("").to_string()))).collect())
        .unwrap_or_default()
}

pub fn write_citations(path: &Path, c: &BTreeMap<String, String>) -> std::io::Result<()> {
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(["key", "purpose"])?;
    for (k, v) in c {
        w.write_record([k, v])?;
    }
    let bytes = w.into_inner().map_err(|e| std::io::Error::other(e.to_string()))?;
    crate::paper::write_atomic(path, &String::from_utf8_lossy(&bytes))
}

/// Keys cited in LaTeX: \cite, \citep, \citet, \parencite, ... with optional
/// [..] arguments and comma lists.
pub fn cited_keys(tex: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = tex;
    while let Some(i) = rest.find("\\") {
        rest = &rest[i + 1..];
        let name: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        if !name.ends_with("cite") && !matches!(name.as_str(), "citep" | "citet" | "citealp" | "citeauthor" | "citeyear" | "nocite") && !name.contains("cite") {
            continue;
        }
        let mut s = rest[name.len()..].trim_start_matches('*');
        while let Some(r) = s.trim_start().strip_prefix('[') {
            match r.find(']') {
                Some(e) => s = &r[e + 1..],
                None => break,
            }
        }
        let s = s.trim_start();
        if let Some(r) = s.strip_prefix('{') {
            if let Some(e) = r.find('}') {
                for k in r[..e].split(',') {
                    let k = k.trim().to_string();
                    // "*" is \nocite{*}; "..." is placeholder text in samples
                    if !k.is_empty() && !k.chars().all(|c| c == '*' || c == '.') && !out.contains(&k) {
                        out.push(k);
                    }
                }
            }
        }
    }
    out
}

/// A paper's refs.bib: the library entries it cites, in citation order.
/// Returns the keys that are cited but missing from the library.
pub fn paper_bib(library: &[Entry], cited: &[String]) -> (String, Vec<String>) {
    let mut chosen = Vec::new();
    let mut missing = Vec::new();
    for k in cited {
        match library.iter().find(|e| &e.key == k) {
            Some(e) => chosen.push(e.clone()),
            None => missing.push(k.clone()),
        }
    }
    (write(&chosen), missing)
}

/// Find a reference's PDF in this machine's folder: the recorded file name,
/// then `<key>.pdf`, then a file name containing the DOI suffix.
pub fn find_pdf(dir: &Path, note: &Note) -> Option<PathBuf> {
    let direct = |name: &str| (!name.is_empty() && !name.contains("..")).then(|| dir.join(name)).filter(|p| p.is_file());
    if let Some(p) = direct(&note.pdf).or_else(|| direct(&format!("{}.pdf", note.key))) {
        return Some(p);
    }
    let doi_tail = note.doi.rsplit('/').next().unwrap_or("").to_lowercase();
    if doi_tail.len() < 4 {
        return None;
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pdf"))
            && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_lowercase().contains(&doi_tail))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIB: &str = r#"
@string{tmi = "IEEE TMI"}
% a comment
@article{ronneberger2015unet,
  title = {{U-Net}: Convolutional Networks for Biomedical Image Segmentation},
  author = {Ronneberger, Olaf and Fischer, Philipp and Brox, Thomas},
  booktitle = "MICCAI",
  year = 2015,
  pages = {234--241},
  doi = {10.1007/978-3-319-24574-4_28}
}
@inproceedings{he2016resnet, author={Kaiming He and Xiangyu Zhang}, title={Deep Residual Learning}, year={2016}, booktitle={CVPR}}
@article{broken, title = {unclosed
"#;

    #[test]
    fn parses_common_bibtex() {
        let (e, errors) = parse(BIB);
        assert_eq!(e.len(), 2, "{errors:?}");
        assert_eq!(e[0].key, "ronneberger2015unet");
        assert_eq!(e[0].title(), "U-Net: Convolutional Networks for Biomedical Image Segmentation");
        assert_eq!(e[0].year(), "2015");
        assert_eq!(e[0].venue(), "MICCAI");
        assert_eq!(e[0].authors_short(), "Ronneberger et al.");
        assert_eq!(e[1].authors_short(), "He & Zhang");
        assert_eq!(errors.len(), 1, "broken entry reported, others kept");
        // written and read back unchanged
        let (again, _) = parse(&write(&e));
        assert_eq!(again, e);
    }

    #[test]
    fn finds_cited_keys() {
        let tex = r"As shown \cite{a,b} and \citep[p.~3]{c}; see \citet*{a} and \parencite[see][]{d}. \nocite{*}";
        assert_eq!(cited_keys(tex), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn builds_a_paper_bib_in_citation_order() {
        let (lib, _) = parse(BIB);
        let (bib, missing) = paper_bib(&lib, &["he2016resnet".into(), "nope".into(), "ronneberger2015unet".into()]);
        assert!(bib.find("he2016resnet").unwrap() < bib.find("ronneberger2015unet").unwrap());
        assert_eq!(missing, vec!["nope"]);
    }

    #[test]
    fn notes_round_trip_and_pdf_lookup() {
        let dir = std::env::temp_dir().join(format!("pc-bib-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("pdfs")).unwrap();
        let notes = vec![Note {
            key: "k1".into(),
            summary: "A summary, with comma".into(),
            one_liner: "Introduces U-Net.".into(),
            tags: vec!["medical".into(), "classical_algorithm".into()],
            doi: "10.1007/978-3-319-24574-4_28".into(),
            pdf: String::new(),
        }];
        write_notes(&dir.join("notes.csv"), &notes).unwrap();
        assert_eq!(read_notes(&dir.join("notes.csv")), notes);
        assert!(find_pdf(&dir.join("pdfs"), &notes[0]).is_none());
        std::fs::write(dir.join("pdfs/unet_978-3-319-24574-4_28.pdf"), "").unwrap();
        assert!(find_pdf(&dir.join("pdfs"), &notes[0]).is_some(), "found by DOI in file name");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
