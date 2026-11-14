//! Which app `POST /api/open/{id}` hands a file to. The author has no Word or
//! PowerPoint: his documents open in his own Lore apps (`open -a <App> <path>`).
//! An app not in `/Applications` → the system default (`open <path>`), as for
//! every type not in the table. An `.app` is never launched (lib.rs reveals it).

use std::path::Path;

/// One place: extension (lower case) → the Lore app that reads it.
/// Legacy `.key` / `.ppt` / `.doc` are not here on purpose: no Lore app reads
/// them (SlideComposer reads only .pptx, PaperComposer only .docx), so they
/// go to the system default.
pub const BY_EXT: &[(&str, &str)] = &[
    ("pdf", "LoreReader"),
    ("pptx", "SlideComposer"),
    ("docx", "PaperComposer"), // its 開啟 Word…: an earlier conversion opens again
    ("tex", "PaperComposer"),  // 開啟 LaTeX…: the .tex's folder
];

/// Where installed apps are looked for.
pub const APPLICATIONS: &str = "/Applications";

/// `/Applications/<app>.app` exists.
pub fn installed(app: &str) -> bool {
    Path::new(APPLICATIONS).join(format!("{app}.app")).is_dir()
}

/// The Lore app for an item, and the path to hand it: a file by its
/// extension; a LaTeX project → PaperComposer with its main `.tex` (it opens
/// the folder; handing a bare folder to `open -a` is not tested on real hardware, so never done).
pub fn target(kind: &str, ext: &str, path: &Path) -> Option<(&'static str, std::path::PathBuf)> {
    if kind == "project" {
        return latex_main(path).map(|tex| ("PaperComposer", tex));
    }
    if kind == "dataset" || kind == "package" {
        return None;
    }
    let ext = ext.to_ascii_lowercase();
    BY_EXT.iter().find(|(e, _)| *e == ext).map(|(_, app)| (*app, path.to_path_buf()))
}

/// A LaTeX project by the scan's own rule (`project::markers`: `main.tex`, or a
/// `.tex` with a `.bib`): its `main.tex`, else its first `.tex` by name.
fn latex_main(dir: &Path) -> Option<std::path::PathBuf> {
    let entries: Vec<(String, bool)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path().is_dir()))
        .collect();
    if !lore_catalog::project::markers(&entries).iter().any(|m| m == "latex") {
        return None;
    }
    let mut tex: Vec<&String> = entries.iter().filter(|(n, d)| !d && n.ends_with(".tex")).map(|(n, _)| n).collect();
    tex.sort_by_key(|n| (n.as_str() != "main.tex", n.as_str()));
    tex.first().map(|n| dir.join(n))
}

/// The `open` arguments: `-a <App> <path>` when that app is installed, else `<path>`.
/// Only macOS has `open -a`; elsewhere always the plain path (`xdg-open`).
pub fn args(kind: &str, ext: &str, path: &Path, installed: &dyn Fn(&str) -> bool) -> (Vec<String>, Option<&'static str>) {
    let plain = || (vec![path.to_string_lossy().to_string()], None);
    if !cfg!(target_os = "macos") {
        return plain();
    }
    match target(kind, ext, path) {
        Some((app, p)) if installed(app) => (vec!["-a".into(), app.into(), p.to_string_lossy().to_string()], Some(app)),
        _ => plain(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &str) -> bool {
        true
    }

    #[test]
    fn each_row_and_the_fallback() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let f = |name: &str| t.path().join(name);
        let a = |kind: &str, ext: &str, p: &Path| args(kind, ext, p, &all);
        let s = |p: &Path| p.to_string_lossy().to_string();
        assert_eq!(a("pdf", "pdf", &f("a.pdf")).0, ["-a", "LoreReader", &s(&f("a.pdf"))]);
        assert_eq!(a("slides", "PPTX", &f("a.PPTX")).0, ["-a", "SlideComposer", &s(&f("a.PPTX"))]);
        assert_eq!(a("document", "docx", &f("a.docx")).0, ["-a", "PaperComposer", &s(&f("a.docx"))]);
        assert_eq!(a("latex", "tex", &f("a.tex")).0, ["-a", "PaperComposer", &s(&f("a.tex"))]);
        // legacy formats no Lore app reads, and everything else: the system default
        for (kind, ext) in [("slides", "key"), ("slides", "ppt"), ("document", "doc"), ("latex", "bib"), ("image", "png")] {
            let p = f(&format!("a.{ext}"));
            assert_eq!(a(kind, ext, &p), (vec![s(&p)], None), "{ext}");
        }
        // a LaTeX project: its main.tex, else its first .tex; a project without one stays default
        let proj = f("talk");
        std::fs::create_dir_all(&proj).unwrap();
        assert_eq!(a("project", "", &proj).0, [s(&proj)]);
        std::fs::write(proj.join("b.tex"), "x").unwrap();
        std::fs::write(proj.join("a.tex"), "x").unwrap();
        assert_eq!(a("project", "", &proj).0, [s(&proj)], "a .tex alone is not a LaTeX project");
        std::fs::write(proj.join("refs.bib"), "x").unwrap();
        assert_eq!(a("project", "", &proj).0, ["-a", "PaperComposer", &s(&proj.join("a.tex"))]);
        std::fs::write(proj.join("main.tex"), "x").unwrap();
        assert_eq!(a("project", "", &proj).0, ["-a", "PaperComposer", &s(&proj.join("main.tex"))]);
        // datasets and packages are never handed to a Lore app
        assert_eq!(a("dataset", "png", &proj).0, [s(&proj)]);
        // the app not installed: the system default
        let none = |_: &str| false;
        assert_eq!(args("pdf", "pdf", &f("a.pdf"), &none), (vec![s(&f("a.pdf"))], None));
        let only_reader = |app: &str| app == "LoreReader";
        assert_eq!(args("slides", "pptx", &f("a.pptx"), &only_reader).0, [s(&f("a.pptx"))]);
        assert_eq!(args("pdf", "pdf", &f("a.pdf"), &only_reader).1, Some("LoreReader"));
    }
}
