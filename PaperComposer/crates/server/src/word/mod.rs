//! 開啟 Word…: a .docx becomes a LaTeX paper folder; the .docx itself is
//! never written (it is kept as `.papercomposer/reference.docx` so a Word
//! export keeps its page setup).
//!
//! Pandoc reads the file once (`-f docx+styles --track-changes=all`, as
//! JSON). The tree is cleaned here (`ast`): Word heading styles pandoc does
//! not know (`Heading1`) become headings, typed section numbers become
//! `sec:N` labels, an equation in a two-column table with a typed `(N)`
//! becomes an `equation` with `\label{eq:N}`, figure and table captions
//! lose their typed "Figure N." and get `fig:N` / `tab:N`, and the
//! reference list is taken out (it becomes refs.bib). Tracked changes and
//! comments are turned into markers, then the tree is written twice: with
//! every change accepted (the paper) and with every change rejected (what
//! the co-author started from). Comparing the two section by section gives
//! the review's changes (`review.json`, `source: "word"`, with each change's
//! Word author and time); the comment markers give its comments. Typed
//! citations `[n]` become `\cite{refn}` and typed "Fig. 1" / "Table 1" /
//! "Eq. (1)" / "Section 2" become `\ref`s to those labels (`refs`).

pub mod api;
pub mod ast;
pub mod bibparse;
pub mod out;
pub mod refs;
pub mod synth;

use crate::texmeasure::tool;
use pc_core::authors::Author;
use pc_core::meta::{self, PaperMeta};
use pc_core::paper::{self, Section};
use pc_core::review::{self, Comment, Review};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What the conversion made, for the card shown after it.
#[derive(Debug, Default, serde::Serialize)]
pub struct Converted {
    pub dir: PathBuf,
    pub sections: usize,
    pub figures: usize,
    pub equations: usize,
    pub references: usize,
    pub changes: usize,
    pub comments: usize,
    /// Findings of the notation registry's reference checks on the result.
    pub notation: Vec<String>,
}

const TEMPLATE: &str = r#"\documentclass{article}
\usepackage{amsmath,amssymb}
\usepackage{graphicx}
\usepackage{longtable,booktabs,array,calc}
\usepackage{hyperref}
\providecommand{\tightlist}{\setlength{\itemsep}{0pt}\setlength{\parskip}{0pt}}
\providecommand{\pandocbounded}[1]{#1}
\title{$title$}
\author{$for(author)$$author$$sep$ \and $endfor$}
\date{}
\begin{document}
\maketitle
$if(abstract)$
\begin{abstract}
$abstract$
\end{abstract}
$endif$
$body$

\bibliographystyle{unsrt}
\bibliography{refs}
\end{document}
"#;

async fn pandoc(args: &[String], dir: &Path) -> Result<Vec<u8>, String> {
    let exe = tool("pandoc").ok_or("pandoc-missing")?;
    let out = tokio::time::timeout(Duration::from_secs(180), tokio::process::Command::new(exe).args(args).current_dir(dir).kill_on_drop(true).output())
        .await
        .map_err(|_| "pandoc 逾時".to_string())?
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("pandoc：{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

pub fn has_pandoc() -> bool {
    tool("pandoc").is_some()
}

/// The folder a .docx converts into by default: `<name> (LaTeX)` beside it.
pub fn default_dest(docx: &Path) -> PathBuf {
    let stem = docx.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "paper".into());
    docx.with_file_name(format!("{stem} (LaTeX)"))
}

/// Convert `docx` into a new paper folder `dest` (missing or empty).
pub async fn convert(docx: &Path, dest: &Path, people: &[Author]) -> Result<Converted, String> {
    if dest.exists() && std::fs::read_dir(dest).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("{} 已經有檔案，請選一個新的資料夾", dest.display()));
    }
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let work = std::env::temp_dir().join(format!("papercomposer-word-{}-{n}", std::process::id()));
    let res = convert_in(docx, dest, people, &work).await;
    let _ = std::fs::remove_dir_all(&work);
    if res.is_err() {
        // leave nothing half made
        if dest.join("main.tex").exists() {
            let _ = std::fs::remove_dir_all(dest);
        }
    }
    res
}

async fn convert_in(docx: &Path, dest: &Path, people: &[Author], work: &Path) -> Result<Converted, String> {
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(work).map_err(io)?;
    let input = work.join("in.docx");
    std::fs::copy(docx, &input).map_err(|e| format!("讀不到 {}：{e}", docx.display()))?;
    let json = pandoc(
        &["-f", "docx+styles", "--track-changes=all", "--extract-media=.", "-t", "json", "-s", "in.docx"].map(String::from),
        work,
    )
    .await?;
    let mut doc: serde_json::Value = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    let found = ast::clean(&mut doc);
    let marks = ast::mark(&mut doc);
    std::fs::write(work.join("template.latex"), TEMPLATE).map_err(io)?;
    let mut tex = Vec::new();
    for mode in [ast::Mode::Accept, ast::Mode::Reject] {
        let mut d = doc.clone();
        ast::resolve(&mut d, mode);
        ast::tables_to_tex(&mut d);
        let file = format!("{mode:?}.json").to_lowercase();
        std::fs::write(work.join(&file), serde_json::to_vec(&d).map_err(|e| e.to_string())?).map_err(io)?;
        let out = pandoc(&["-f", "json", "-t", "latex", "--wrap=none", "-s", "--template=template.latex", &file].map(String::from), work).await?;
        let text = String::from_utf8_lossy(&out).to_string();
        tex.push(refs::rewrite(&text, &found.labels, found.references.len()));
    }
    let (accepted, rejected) = (tex.remove(0), tex.remove(0));

    // the paper: main.tex with its figures, split into sections
    let src = work.join("src");
    std::fs::create_dir_all(src.join("figures")).map_err(io)?;
    std::fs::write(src.join("main.tex"), &accepted).map_err(io)?;
    for (from, to) in &found.images {
        std::fs::copy(work.join(from), src.join(to)).map_err(|e| format!("{from}: {e}"))?;
    }
    std::fs::write(src.join("refs.bib"), refs::bibtex(&found.references)).map_err(io)?;
    paper::import_into(dest, &src)?;
    let app = dest.join(paper::APP_DIR);
    std::fs::create_dir_all(&app).map_err(io)?;
    std::fs::copy(docx, app.join("reference.docx")).map_err(io)?;

    // sections as written, markers taken out; the rejected version beside them
    let mut review = Review::default();
    let mut comments = 0;
    // Word's threads (commentsExtended.xml): replies and done comments
    let threads = out::comment_threads(&std::fs::read(docx).unwrap_or_default());
    let mut by_word_id: HashMap<String, usize> = HashMap::new();
    let mut replies: Vec<(String, review::Reply)> = Vec::new();
    let before = paper::split_sections(&rejected);
    let files = paper::section_files(dest);
    for (i, file) in files.iter().enumerate() {
        let path = dest.join("sections").join(file);
        let mut sec = paper::parse_section(file, &std::fs::read_to_string(&path).map_err(io)?);
        let (paras, after_marks, found_comments) = unmark(&sec.paragraphs, true);
        sec.paragraphs = paras;
        paper::write_atomic(&path, &paper::render_section(&sec)).map_err(io)?;
        let sec = paper::parse_section(file, &std::fs::read_to_string(&path).map_err(io)?);
        for c in found_comments {
            let Some(info) = marks.comments.get(&c.id) else { continue };
            let (parent, done) = threads.get(&c.id).cloned().unwrap_or((None, false));
            if let Some(p) = parent {
                replies.push((p, review::Reply { id: review::new_id("r"), author: info.author.clone(), at: info.date.clone(), text: info.text.clone(), extra: Default::default() }));
                continue;
            }
            by_word_id.insert(c.id.clone(), review.comments.len());
            review.comments.push(Comment {
                id: review::new_id("m"),
                author: info.author.clone(),
                at: info.date.clone(),
                file: file.clone(),
                index: c.para,
                exact: c.exact,
                prefix: c.prefix,
                suffix: c.suffix,
                text: info.text.clone(),
                replies: vec![],
                resolved: done,
                orphan: false,
                source: Some("word".into()),
                extra: Default::default(),
            });
            comments += 1;
        }
        // the same section of the rejected version: by place, when the
        // headings line up (a heading changed in review leaves it out)
        let Some(old) = before.get(i).filter(|b| before.len() == files.len() && norm_title(&b.title) == norm_title(&sec.title)) else { continue };
        let rendered = paper::parse_section(file, &paper::render_section(&Section { paragraphs: unmark(&old.paragraphs, false).0, ..old.clone() }));
        let (_, before_marks, _) = unmark(&old.paragraphs, false);
        let was = review.changes.len();
        review.tracking = true;
        review::record(&mut review, file, &rendered.paragraphs, &sec.paragraphs, "", "");
        review.tracking = false;
        // each change gets the Word author and time of its paragraph
        for c in review.changes.iter_mut().skip(was) {
            let k = c.after.as_deref().and_then(|t| after_marks.get(&review::norm(t))).or_else(|| c.before.as_deref().and_then(|t| before_marks.get(&review::norm(t))));
            let who = k.and_then(|k| marks.changes.get(*k)).or(marks.changes.first());
            if let Some(w) = who {
                c.author = w.author.clone();
                c.at = w.date.clone();
            }
            c.source = Some("word".into());
        }
    }
    for (parent, reply) in replies {
        match by_word_id.get(&parent) {
            Some(&k) => review.comments[k].replies.push(reply),
            None => {}
        }
    }
    for c in &mut review.comments {
        c.replies.sort_by(|a, b| a.at.cmp(&b.at));
    }
    review::save(dest, &review)?;

    // paper.toml: the profile's people named as authors
    let authors = pc_core::authors::match_block(&std::fs::read_to_string(dest.join("main.tex")).unwrap_or_default(), people);
    meta::write_meta(dest, &PaperMeta { key_value: String::new(), journal: None, authors })?;

    Ok(Converted {
        dir: dest.to_path_buf(),
        sections: files.len(),
        figures: found.images.len(),
        equations: found.labels.iter().filter(|l| l.starts_with("eq:")).count(),
        references: found.references.len(),
        changes: review.changes.len(),
        comments,
        notation: reference_findings(dest, &Default::default()),
    })
}

fn norm_title(t: &str) -> String {
    t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The notation registry's reference checks on the converted paper: typed
/// numbers left ("Table 1" with no table 1 found) and \ref to no label.
pub fn reference_findings(dir: &Path, registry: &lore_notation::Notation) -> Vec<String> {
    let mut files = vec![("main.tex".to_string(), std::fs::read_to_string(dir.join("main.tex")).unwrap_or_default())];
    for f in paper::section_files(dir) {
        files.push((format!("sections/{f}"), std::fs::read_to_string(dir.join("sections").join(&f)).unwrap_or_default()));
    }
    let doc = lore_notation::Document::from_files(&files);
    let report = lore_notation::check_document(doc, registry, &Default::default());
    report.issues.iter().filter(|i| ["hardcoded_reference", "ref_undefined"].contains(&i.code.as_str())).map(|i| i.message.clone()).collect()
}

#[derive(Debug, PartialEq)]
struct FoundComment {
    id: String,
    para: usize,
    exact: String,
    prefix: String,
    suffix: String,
}

/// Take the markers out of paragraphs: returns the clean paragraphs (empty
/// ones dropped), the change marker of each paragraph (by normalised text),
/// and, when `comments`, where each comment sits.
fn unmark(paras: &[String], comments: bool) -> (Vec<String>, HashMap<String, usize>, Vec<FoundComment>) {
    let mut out = Vec::new();
    let mut marks = HashMap::new();
    let mut found = Vec::new();
    for p in paras {
        let mut clean = String::new();
        let mut change = None;
        let mut starts: Vec<(String, usize)> = Vec::new();
        let mut ends: Vec<(String, usize)> = Vec::new();
        let mut it = p.char_indices().peekable();
        while let Some((_, c)) = it.next() {
            if matches!(c, ast::C_START | ast::C_END | ast::CHANGE) {
                let mut id = String::new();
                for (_, d) in it.by_ref() {
                    if d == ast::CLOSE {
                        break;
                    }
                    id.push(d);
                }
                match c {
                    ast::C_START => starts.push((id, clean.len())),
                    ast::C_END => ends.push((id, clean.len())),
                    _ => change = id.parse::<usize>().ok(),
                }
                continue;
            }
            clean.push(c);
        }
        // a marker on a line of its own leaves a line end behind
        let clean = clean.trim_end().to_string();
        if clean.trim().is_empty() {
            continue;
        }
        if comments {
            for (id, s) in &starts {
                let s = (*s).min(clean.len());
                let e = ends.iter().find(|(i, _)| i == id).map(|(_, e)| *e).unwrap_or(clean.len()).clamp(s, clean.len());
                let take = |t: &str, n: usize, back: bool| -> String {
                    let v: Vec<char> = t.chars().collect();
                    if back { v[v.len().saturating_sub(n)..].iter().collect() } else { v.iter().take(n).collect() }
                };
                found.push(FoundComment {
                    id: id.clone(),
                    para: out.len(),
                    exact: clean[s..e].to_string(),
                    prefix: take(&clean[..s], 32, true),
                    suffix: take(&clean[e..], 32, false),
                });
            }
        }
        if let Some(k) = change {
            marks.insert(review::norm(&clean), k);
        }
        out.push(clean);
    }
    (out, marks, found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_come_out_and_say_where_comments_are() {
        let p = format!("We study {}0{}Section~\\ref{{sec:2}} describes it{}0{} and more.{}3{}", ast::C_START, ast::CLOSE, ast::C_END, ast::CLOSE, ast::CHANGE, ast::CLOSE);
        let alone = format!("{}4{}", ast::CHANGE, ast::CLOSE);
        let (paras, marks, found) = unmark(&[p, alone, "Plain.".into()], true);
        assert_eq!(paras, ["We study Section~\\ref{sec:2} describes it and more.", "Plain."]);
        assert_eq!(marks.get("We study Section~\\ref{sec:2} describes it and more."), Some(&3));
        assert_eq!(found, [FoundComment { id: "0".into(), para: 0, exact: "Section~\\ref{sec:2} describes it".into(), prefix: "We study ".into(), suffix: " and more.".into() }]);
    }

    /// The synthetic Word paper end to end (needs pandoc; skipped without it).
    #[tokio::test]
    async fn converts_a_word_paper_with_tracked_changes_and_comments() {
        if !has_pandoc() {
            eprintln!("pandoc not installed: skipped");
            return;
        }
        let root = std::env::temp_dir().join(format!("pc-word-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let docx = root.join("Widgets.docx");
        let bytes = synth::docx();
        std::fs::write(&docx, &bytes).unwrap();
        let dest = default_dest(&docx);
        assert_eq!(dest, root.join("Widgets (LaTeX)"));
        let people = vec![Author { id: "a1".into(), given: "Ellrin".into(), family: "Example".into(), ..Default::default() }];
        let c = convert(&docx, &dest, &people).await.unwrap();
        assert_eq!(std::fs::read(&docx).unwrap(), bytes, "the .docx is never written");
        assert!(dest.join(".papercomposer/reference.docx").is_file());
        assert_eq!((c.sections, c.figures, c.equations, c.references), (3, 1, 1, 3), "{c:?}");

        let main = std::fs::read_to_string(dest.join("main.tex")).unwrap();
        assert!(main.contains("\\title{Synthetic Study of Folded Widgets}") && main.contains("\\bibliography{refs}"), "{main}");
        let all: String = paper::section_files(&dest).iter().map(|f| std::fs::read_to_string(dest.join("sections").join(f)).unwrap()).collect();
        assert!(all.contains("\\section{Introduction}\\label{sec:1}"), "{all}");
        assert!(all.contains("\\cite{ref1}") && all.contains("\\cite{ref2,ref3}"), "{all}");
        assert!(all.contains("Section~\\ref{sec:2}") && all.contains("Fig.~\\ref{fig:1}") && all.contains("Table~\\ref{tab:1}") && all.contains("Eq.~\\eqref{eq:1}"), "{all}");
        assert!(all.contains("\\begin{equation}") && all.contains("\\label{eq:1}") && all.contains("\\label{fig:1}") && all.contains("\\label{tab:1}"), "{all}");
        assert!(all.contains("figures/image1.png") && dest.join("figures/image1.png").is_file());
        assert!(all.contains("\\begin{table}[t]\n\\centering\n\\caption{Widgets and their stiffness.}\n\\label{tab:1}\n\\begin{tabular}{ll}\n\\toprule\nWidget & Stiffness \\\\\n\\midrule\nA & 12 \\\\\n\\bottomrule"), "{all}");
        assert!(!all.contains("Figure 1.") && !all.contains("Table~\\ref{tab:1}.") && !all.contains("References"), "{all}");
        // the accepted text is the paper
        assert!(all.contains("We study folded widgets") && !all.contains("very simple") && all.contains("added in review") && !all.contains("was removed"), "{all}");
        assert!(!all.chars().any(|c| ('\u{E000}'..='\u{E0FF}').contains(&c)), "no markers left");
        let bib = std::fs::read_to_string(dest.join("refs.bib")).unwrap();
        assert!(bib.contains("@article{ref1,") && bib.contains("title = {Folded widgets}") && bib.contains("year = {2020}"), "{bib}");
        assert!(bib.contains("@inproceedings{ref3,") && bib.contains("booktitle = {Proceedings of Widget Day}"), "{bib}");

        // review.json: three changes by daidai, one comment
        let r = review::load(&dest);
        assert!(dest.join(".papercomposer/review.json").is_file());
        assert_eq!(r.changes.len(), 3, "{:#?}", r.changes);
        assert!(r.changes.iter().all(|c| c.author == "daidai" && c.source.as_deref() == Some("word")));
        let edit = r.changes.iter().find(|c| c.before.is_some() && c.after.is_some()).unwrap();
        assert!(edit.before.as_deref().unwrap().contains("very simple") && edit.after.as_deref().unwrap().contains("We study folded"));
        assert_eq!(edit.at, "2026-11-01T10:00:00Z");
        assert!(r.changes.iter().any(|c| c.before.is_none() && c.after.as_deref().is_some_and(|a| a.contains("added in review"))));
        assert!(r.changes.iter().any(|c| c.after.is_none() && c.before.as_deref().is_some_and(|b| b.contains("was removed"))));
        assert_eq!(r.comments.len(), 1);
        let m = &r.comments[0];
        assert_eq!((m.author.as_str(), m.text.as_str(), m.source.as_deref()), ("daidai", "Please cite a second source here.", Some("word")));
        assert!(m.exact.contains("describes it"), "{m:?}");
        // the comment is found where it is, and every change can be placed
        let mut again = r.clone();
        let sec = paper::section_files(&dest)[0].clone();
        let paras = paper::load_paper(&dest)[0].paragraphs.clone();
        review::reanchor(&mut again, &sec, &paras);
        assert!(!again.comments[0].orphan);
        // rejecting the edit puts the co-author's sentence back
        let back = review::rejected(&paras, &r.changes, edit).unwrap();
        assert!(back.iter().any(|p| p.contains("very simple")));

        // authors from the profile, no key value yet
        let m = meta::read_meta(&dest);
        assert_eq!(m.authors.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["a1"]);
        // lore-notation: no typed reference left, no \ref without its label
        assert!(c.notation.is_empty(), "{:?}", c.notation);
        // a second conversion into the same folder is refused
        assert!(convert(&docx, &dest, &people).await.is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
