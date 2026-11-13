//! 寫書: a book is a paper folder whose `paper.toml` says `kind = "book"`
//! (docs/book.md). It is made from a book template (`template.toml`
//! `kind = "book"` under `publishers/`), lives where the author chose, and is
//! edited like a paper: `main.tex` + `sections/` (one `\section` = one chapter),
//! `refs.bib`, `notation.tex` from lore-notation. What a paper has and a book
//! does not: the key value and the journal's rules (word, page limits).
//!
//! The public template is `private-example/PaperComposer/publishers/lecture-notes/`,
//! built into the app as well, so a workspace made before it existed still
//! gets it; a book template in the workspace's `publishers/` wins.

use crate::meta::{self, PaperMeta, KIND_BOOK};
use crate::paper;
use std::path::{Path, PathBuf};

/// The built-in template's folder name.
pub const TEMPLATE_ID: &str = "lecture-notes";

/// The built-in template's files (`publishers/lecture-notes/`).
pub const TEMPLATE_FILES: &[(&str, &str)] = &[
    ("sample.tex", include_str!("../../../../private-example/PaperComposer/publishers/lecture-notes/sample.tex")),
    ("lorenotes.sty", include_str!("../../../../private-example/PaperComposer/publishers/lecture-notes/lorenotes.sty")),
    ("refs.bib", include_str!("../../../../private-example/PaperComposer/publishers/lecture-notes/refs.bib")),
    ("template.toml", include_str!("../../../../private-example/PaperComposer/publishers/lecture-notes/template.toml")),
];

/// Is this publisher folder a book template?
pub fn is_book_template(dir: &Path) -> bool {
    #[derive(serde::Deserialize)]
    struct K {
        #[serde(default)]
        kind: String,
    }
    std::fs::read_to_string(dir.join("template.toml")).ok().and_then(|t| toml::from_str::<K>(&t).ok()).is_some_and(|k| k.kind == KIND_BOOK)
}

/// The workspace's book template: `publishers/lecture-notes/` when it is
/// one, else the first book template by folder name.
pub fn workspace_template(publishers: &Path) -> Option<PathBuf> {
    let own = publishers.join(TEMPLATE_ID);
    if is_book_template(&own) && own.join("sample.tex").is_file() {
        return Some(own);
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(publishers).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs.into_iter().find(|d| is_book_template(d) && d.join("sample.tex").is_file())
}

/// Write the built-in template into `dir` (a scratch folder).
pub fn write_builtin(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, text) in TEMPLATE_FILES {
        std::fs::write(dir.join(name), text)?;
    }
    Ok(())
}

/// A folder name from a book title: letters, digits, space, - _ . only.
pub fn folder_name(title: &str) -> String {
    let s: String = title.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') { c } else { ' ' }).collect();
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ").trim_start_matches('.').trim().chars().take(60).collect();
    let s = s.trim().to_string();
    if s.is_empty() { "book".into() } else { s }
}

/// A name for the new folder in `parent` that is not taken.
fn free_name(parent: &Path, base: &str) -> String {
    (1..).map(|k| if k == 1 { base.to_string() } else { format!("{base} {k}") }).find(|n| !parent.join(n).exists()).expect("a free name")
}

/// Make a book in a new folder under `parent`, named after `title`, from
/// the template folder `template`: main.tex + sections/ (the template's
/// sample split as for a paper), the template's style and refs.bib,
/// `notation.tex` (lore-notation, empty registry), paper.toml with
/// `kind = "book"`, and the title in main.tex. Returns the folder.
pub fn create(parent: &Path, title: &str, template: &Path) -> Result<PathBuf, String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return Err("請填寫書名".into());
    }
    if !parent.is_dir() {
        return Err(format!("找不到資料夾 {}", parent.display()));
    }
    let name = free_name(parent, &folder_name(&title));
    let dir = paper::create_paper_from_template_with(parent, &name, template, None)?;
    let done = (|| -> Result<(), String> {
        meta::write_meta(&dir, &PaperMeta { kind: KIND_BOOK.into(), ..Default::default() })?;
        lore_notation::write_notation_tex(&dir, &lore_notation::Notation::default(), &Default::default()).map_err(|e| e.to_string())?;
        let main_path = dir.join("main.tex");
        let main = std::fs::read_to_string(&main_path).map_err(|e| e.to_string())?;
        let main = crate::front::set_field(&main, "title", 0, &meta::tex_text(&title))?;
        paper::write_atomic(&main_path, &main).map_err(|e| e.to_string())
    })();
    if let Err(e) = done {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-book-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn folder_names_from_titles() {
        assert_eq!(folder_name("Contrastive learning: from scratch"), "Contrastive learning from scratch");
        assert_eq!(folder_name("對比學習／筆記"), "對比學習 筆記");
        assert_eq!(folder_name(" ../ "), "book");
    }

    #[test]
    fn the_builtin_template_is_a_book_template() {
        let d = tmp("tpl");
        write_builtin(&d.join(TEMPLATE_ID)).unwrap();
        std::fs::create_dir_all(d.join("ieee")).unwrap();
        std::fs::write(d.join("ieee/sample.tex"), "x").unwrap();
        assert!(is_book_template(&d.join(TEMPLATE_ID)) && !is_book_template(&d.join("ieee")));
        assert_eq!(workspace_template(&d), Some(d.join(TEMPLATE_ID)));
        assert_eq!(workspace_template(&d.join("ieee")), None);
        let sample = std::fs::read_to_string(d.join(TEMPLATE_ID).join("sample.tex")).unwrap();
        assert!(sample.starts_with("% !TEX program = xelatex\n"), "export picks XeLaTeX");
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Every file of the book, as lore-notation reads a document.
    fn files_of(dir: &Path) -> Vec<(String, String)> {
        let mut files = vec![("main.tex".to_string(), std::fs::read_to_string(dir.join("main.tex")).unwrap())];
        for f in paper::section_files(dir) {
            files.push((format!("sections/{f}"), std::fs::read_to_string(dir.join("sections").join(&f)).unwrap()));
        }
        files
    }

    #[test]
    fn a_new_book_is_a_paper_folder_marked_book() {
        let root = tmp("new");
        let tpl = root.join("tpl");
        write_builtin(&tpl).unwrap();
        let parent = root.join("Books");
        std::fs::create_dir_all(&parent).unwrap();
        assert!(create(&parent, "  ", &tpl).is_err());
        assert!(create(&root.join("missing"), "X", &tpl).is_err());
        let dir = create(&parent, "Contrastive learning & friends", &tpl).unwrap();
        assert_eq!(dir, parent.join("Contrastive learning friends"));
        let again = create(&parent, "Contrastive learning & friends", &tpl).unwrap();
        assert_eq!(again, parent.join("Contrastive learning friends 2"), "never into an existing folder");

        assert!(meta::read_meta(&dir).is_book());
        assert!(dir.join("paper.toml").is_file(), "PaperComposer's files at the top, as a paper made by the app");
        assert!(dir.join("lorenotes.sty").is_file() && dir.join("refs.bib").is_file());
        assert!(!dir.join("sample.tex").exists() && !dir.join("template.toml").exists());
        let notation = std::fs::read_to_string(dir.join("notation.tex")).unwrap();
        assert!(lore_notation::macros::is_generated(&notation), "{notation}");
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        assert!(main.starts_with("% !TEX program = xelatex\n"), "{main}");
        assert!(main.contains("\\title{Contrastive learning \\& friends}") && main.contains("\\input{notation}") && main.contains("\\bibliography{refs}"), "{main}");
        assert_eq!(paper::section_files(&dir).len(), 1);
        assert_eq!(crate::open::sections(&dir)[0].title, "Introduction");

        // lore-notation: the new book has nothing to fix
        use lore_notation::{check_document, CheckOptions, Document, Notation};
        let issues = check_document(Document::from_files(&files_of(&dir)), &Notation::default(), &CheckOptions::default()).issues;
        assert!(!issues.iter().any(|i| ["hardcoded_reference", "ref_undefined"].iter().any(|c| i.code.starts_with(c)) || i.code.starts_with("label_")), "{:?}", issues.iter().map(|i| &i.code).collect::<Vec<_>>());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
