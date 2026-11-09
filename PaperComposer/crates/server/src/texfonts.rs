//! Fonts a paper names for XeLaTeX / LuaLaTeX (`\setmainfont`, `\setCJKmainfont`,
//! `\newfontfamily`, `\setmathfont` …) and what they become on this machine,
//! through `lore-fonts` (the author's `[fallback]` rules in
//! `private/shared/fonts.toml`, metric-compatible defaults, category chains).
//!
//! - A font this machine has (system, TeX's own font tree, or the private font
//!   folder `private/shared/fonts/`) is left as it is.
//! - A missing font with a substitute here: the compile runs on **copies** of
//!   the files that name it, with the substitute's name; the author's files
//!   (main.tex, a preamble, notation.tex …) are never changed. The export
//!   report says what was replaced.
//! - Nothing in the chain is here: the export stops before compiling and says so.
//!
//! Fonts from the private folder are named by file (`{File.ttf}[Path=…,
//! BoldFont=…]`) in the copy, so XeTeX finds them on every platform without
//! installing them (on macOS XeTeX looks names up through Core Text, which does
//! not see that folder). `OSFONTDIR` also points at the folder, for LuaTeX's
//! name lookup and kpathsea file lookups.
//!
//! **Not tested on real hardware**: no TeX engine is installed where this was
//! written; the parsing and rewriting are unit-tested with synthetic .tex.

use lore_fonts::{How, Installed, Resolution, Table};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Font commands whose (last) mandatory argument is a font name.
const CMDS: &[&str] = &[
    "setmainfont",
    "setromanfont",
    "setsansfont",
    "setmonofont",
    "setmathfont",
    "newfontfamily",
    "setCJKmainfont",
    "setCJKsansfont",
    "setCJKmonofont",
    "newCJKfontfamily",
];

/// One font named in a file.
#[derive(Debug, Clone, PartialEq)]
pub struct FontUse {
    /// File relative to the paper folder (`main.tex`, `preamble.tex`, `mystyle.sty`).
    pub file: String,
    /// 1-based line of the command.
    pub line: u32,
    /// Command without the backslash (`setmainfont`).
    pub cmd: String,
    pub name: String,
    /// Byte range of the name inside its braces.
    name_at: Range<usize>,
    /// Byte index just after the `[` of options following the name, if any.
    opts_after: Option<usize>,
    /// Byte index just after the `}` closing the name.
    name_end: usize,
    /// All option text (before and after the name), to spot `Path=`.
    opts: String,
}

impl FontUse {
    /// A font given as a file (`texgyretermes-regular.otf`, `Path=`) or built
    /// from a macro: not a family name to look up.
    pub fn is_file_or_macro(&self) -> bool {
        let n = self.name.to_ascii_lowercase();
        self.name.contains(['\\', '#'])
            || [".otf", ".ttf", ".ttc", ".otc", ".pfb", ".woff", ".woff2"].iter().any(|e| n.ends_with(e))
            || self.opts.contains("Path")
            || self.opts.contains("Extension")
    }
}

/// The text with comments blanked out (same byte offsets).
fn mask_comments(tex: &str) -> String {
    let b = tex.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'%' => {
                while i < b.len() && b[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    // Only ASCII bytes inside comments were replaced, each by a space, and
    // whole comments are blanked, so the text stays valid UTF-8.
    String::from_utf8(out).unwrap_or_else(|_| tex.to_string())
}

/// Skip spaces and single line breaks (not a blank line: xparse stops there).
fn skip_space(b: &[u8], mut i: usize) -> usize {
    let mut newlines = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t' || b[i] == b'\n' || b[i] == b'\r') {
        if b[i] == b'\n' {
            newlines += 1;
            if newlines > 1 {
                break;
            }
        }
        i += 1;
    }
    i
}

/// Index just after the group or bracket opening at `i`, braces balanced.
fn close(b: &[u8], i: usize, open: u8, shut: u8) -> Option<usize> {
    let mut depth = 0i32;
    let mut braces = 0i32;
    let mut j = i;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 1,
            b'{' if open != b'{' => braces += 1,
            b'}' if open != b'{' => braces -= 1,
            c if c == open && braces == 0 => depth += 1,
            c if c == shut && braces == 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(j + 1);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Every font name `tex` sets with the commands above (comments ignored).
pub fn find_fonts(file: &str, tex: &str) -> Vec<FontUse> {
    let m = mask_comments(tex);
    let b = m.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_alphabetic() {
            j += 1;
        }
        let word = &m[i + 1..j];
        i = j.max(i + 1);
        let Some(cmd) = CMDS.iter().find(|c| **c == word) else { continue };
        let mut opts = String::new();
        let mut k = skip_space(b, j);
        // \newCJKfontfamily[tag]\cs …
        if *cmd == "newCJKfontfamily" && b.get(k) == Some(&b'[') {
            let Some(e) = close(b, k, b'[', b']') else { continue };
            k = skip_space(b, e);
        }
        // \newfontfamily\cs or {\cs}
        if cmd.starts_with("new") {
            if b.get(k) == Some(&b'\\') {
                k += 1;
                while k < b.len() && b[k].is_ascii_alphabetic() {
                    k += 1;
                }
            } else if b.get(k) == Some(&b'{') {
                let Some(e) = close(b, k, b'{', b'}') else { continue };
                k = e;
            } else {
                continue;
            }
            k = skip_space(b, k);
        }
        if b.get(k) == Some(&b'[') {
            let Some(e) = close(b, k, b'[', b']') else { continue };
            opts.push_str(&m[k + 1..e - 1]);
            k = skip_space(b, e);
        }
        if b.get(k) != Some(&b'{') {
            continue;
        }
        let Some(e) = close(b, k, b'{', b'}') else { continue };
        let raw = &tex[k + 1..e - 1];
        let lead = raw.len() - raw.trim_start().len();
        let name_at = k + 1 + lead..k + 1 + lead + raw.trim().len();
        let after = skip_space(b, e);
        let opts_after = if b.get(after) == Some(&b'[') {
            if let Some(oe) = close(b, after, b'[', b']') {
                opts.push(',');
                opts.push_str(&m[after + 1..oe - 1]);
            }
            Some(after + 1)
        } else {
            None
        };
        out.push(FontUse {
            file: file.to_string(),
            line: tex[..start].matches('\n').count() as u32 + 1,
            cmd: cmd.to_string(),
            name: tex[name_at.clone()].to_string(),
            name_at,
            opts_after,
            name_end: e,
            opts,
        });
        i = e;
    }
    out
}

/// main.tex and the local files its preamble loads (`\input`, `\include`,
/// `\usepackage` of a .sty in the paper folder), relative paths, no repeats.
pub fn preamble_files(paper: &Path) -> Vec<String> {
    let mut files = vec!["main.tex".to_string()];
    let mut k = 0;
    while k < files.len() && files.len() < 40 {
        let Ok(text) = std::fs::read_to_string(paper.join(&files[k])) else {
            k += 1;
            continue;
        };
        let m = mask_comments(&text);
        let head = if k == 0 { m.split("\\begin{document}").next().unwrap_or(&m) } else { &m };
        for (cmd, ext) in [("\\input{", "tex"), ("\\include{", "tex"), ("\\usepackage{", "sty"), ("\\RequirePackage{", "sty")] {
            for part in head.split(cmd).skip(1) {
                let Some(arg) = part.split('}').next() else { continue };
                for a in arg.split(',').map(str::trim).filter(|a| !a.is_empty() && !a.contains("..")) {
                    let rel = if a.ends_with(&format!(".{ext}")) { a.to_string() } else { format!("{a}.{ext}") };
                    if paper.join(&rel).is_file() && !files.contains(&rel) {
                        files.push(rel);
                    }
                }
            }
        }
        k += 1;
    }
    files
}

/// What happens to one font.
#[derive(Debug, Clone, PartialEq)]
pub enum Fix {
    /// The font is here: nothing to do.
    Keep,
    /// Use another family name.
    Rename(String),
    /// Use font files from the private folder (by file name and `Path=`).
    Files { name: String, opts: String },
    /// Nothing in the chain is here.
    Missing,
}

/// Pick regular / bold / italic / bold italic files of `family` from the
/// private folder.
fn private_files(installed: &Installed, family: &str, dir: &Path) -> Option<Fix> {
    let faces: Vec<_> = installed.private.iter().filter(|p| p.families.iter().any(|f| f.eq_ignore_ascii_case(family))).collect();
    let pick = |italic: bool, weight: i32| {
        faces.iter().filter(|f| f.italic == italic).min_by_key(|f| (f.weight as i32 - weight).abs()).map(|f| f.file.clone())
    };
    let regular = pick(false, 400).or_else(|| pick(true, 400))?;
    let mut dir = dir.to_string_lossy().replace('\\', "/");
    if !dir.ends_with('/') {
        dir.push('/');
    }
    let mut opts = vec![format!("Path={{{dir}}}")];
    let distinct = |f: Option<String>, w: i32, it: bool| {
        f.filter(|f| *f != regular && faces.iter().any(|x| x.file == *f && x.italic == it && (x.weight as i32 >= 600) == (w >= 600)))
    };
    if let Some(f) = distinct(pick(false, 700), 700, false) {
        opts.push(format!("BoldFont={{{f}}}"));
    }
    if let Some(f) = distinct(pick(true, 400), 400, true) {
        opts.push(format!("ItalicFont={{{f}}}"));
    }
    if let Some(f) = distinct(pick(true, 700), 700, true) {
        opts.push(format!("BoldItalicFont={{{f}}}"));
    }
    Some(Fix::Files { name: regular, opts: opts.join(",") })
}

/// Decide what to do with `u` on this machine.
pub fn fix_for(r: &Resolution, installed: &Installed, private_dir: Option<&Path>) -> Fix {
    let Some(used) = &r.used else { return Fix::Missing };
    let in_private = installed.private.iter().any(|p| p.families.iter().any(|f| f.eq_ignore_ascii_case(used)));
    if in_private {
        if let Some(f) = private_dir.and_then(|d| private_files(installed, used, d)) {
            return f;
        }
    }
    match r.how {
        How::Installed | How::Private => Fix::Keep,
        How::Substitute => Fix::Rename(used.clone()),
        How::Missing => Fix::Missing,
    }
}

/// Apply fixes to `tex` (uses must come from `find_fonts` on this text).
pub fn rewrite(tex: &str, fixes: &[(&FontUse, &Fix)]) -> String {
    // edits as (at, remove up to, insert), applied back to front
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (u, f) in fixes {
        match f {
            Fix::Rename(n) => edits.push((u.name_at.start, u.name_at.end, n.clone())),
            Fix::Files { name, opts } => {
                edits.push((u.name_at.start, u.name_at.end, name.clone()));
                match u.opts_after {
                    Some(at) => edits.push((at, at, format!("{opts},"))),
                    None => edits.push((u.name_end, u.name_end, format!("[{opts}]"))),
                }
            }
            Fix::Keep | Fix::Missing => {}
        }
    }
    edits.sort_by(|a, b| b.0.cmp(&a.0));
    let mut s = tex.to_string();
    for (at, to, ins) in edits {
        s.replace_range(at..to, &ins);
    }
    s
}

/// Fonts this machine has for TeX: the system's, TeX's own OpenType/TrueType
/// trees (`kpsewhich -var-value`), and the private folder. Scanned once.
pub fn installed(private_dir: Option<&Path>) -> &'static Installed {
    static FONTS: std::sync::OnceLock<Installed> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let mut i = Installed::scan(private_dir);
        if let Some(kpse) = crate::texmeasure::tool("kpsewhich") {
            for var in ["TEXMFDIST", "TEXMFLOCAL", "TEXMFHOME"] {
                let Ok(o) = crate::proc::std_cmd(&kpse).arg(format!("-var-value={var}")).output() else { continue };
                let root = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
                for sub in ["fonts/opentype", "fonts/truetype"] {
                    if root.join(sub).is_dir() {
                        i.add_system_dir(&root.join(sub));
                    }
                }
            }
        }
        i
    })
}

/// What the compile needs after the font check.
#[derive(Debug, Default)]
pub struct Prepared {
    /// The rewritten main.tex to compile instead of the author's, if it changed.
    pub main: Option<PathBuf>,
    /// Folder holding rewritten copies; put first on `TEXINPUTS`.
    pub copies: Option<PathBuf>,
    /// Lines for the export report.
    pub notes: Vec<String>,
    /// Fonts with nothing to stand in: (file, line, message).
    pub missing: Vec<(String, u32, String)>,
}

/// Check every font the paper names and write copies with substitutes into
/// `work/.fonts/` when needed (a dot folder: the build folder mirrors the paper's own folders). The paper's own files are only read.
pub fn prepare(paper: &Path, work: &Path, table: &Table, installed: &Installed, private_dir: Option<&Path>) -> std::io::Result<Prepared> {
    let mut p = Prepared::default();
    let copies = work.join(".fonts");
    let _ = std::fs::remove_dir_all(&copies);
    for rel in preamble_files(paper) {
        let Ok(text) = std::fs::read_to_string(paper.join(&rel)) else { continue };
        let uses: Vec<FontUse> = find_fonts(&rel, &text).into_iter().filter(|u| !u.is_file_or_macro()).collect();
        let mut fixes: Vec<(&FontUse, Fix)> = Vec::new();
        for u in &uses {
            let r = lore_fonts::resolve(table, installed, &u.name);
            let f = fix_for(&r, installed, private_dir);
            match &f {
                Fix::Missing => p.missing.push((
                    rel.clone(),
                    u.line,
                    format!("這台電腦沒有字型「{}」，代替順序（{}）裡也一個都沒有", u.name, r.chain.join(" → ")),
                )),
                Fix::Rename(n) => p.notes.push(format!(
                    "字型代替：{rel} 第 {} 行「{}」→「{n}」{}（只改編譯用的副本，原檔不變）",
                    u.line,
                    u.name,
                    if r.user_rule { "，依 fonts.toml 的規則" } else { "" }
                )),
                Fix::Files { name, .. } if !r.used.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(&u.name)) => p.notes.push(format!(
                    "字型代替：{rel} 第 {} 行「{}」→ 私人字型資料夾的 {name}（只改編譯用的副本，原檔不變）",
                    u.line, u.name
                )),
                _ => {}
            }
            if matches!(f, Fix::Rename(_) | Fix::Files { .. }) {
                fixes.push((u, f));
            }
        }
        if fixes.is_empty() {
            continue;
        }
        let refs: Vec<(&FontUse, &Fix)> = fixes.iter().map(|(u, f)| (*u, f)).collect();
        let out = copies.join(&rel);
        if let Some(d) = out.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&out, rewrite(&text, &refs))?;
        p.copies = Some(copies.clone());
        if rel == "main.tex" {
            p.main = Some(out);
        }
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_fonts::PrivateFace;

    const PREAMBLE: &str = r"\documentclass{article}
\usepackage{fontspec}
\usepackage{xeCJK}
\usepackage{unicode-math}
% \setmainfont{Commented Out}
\setmainfont{Calibri}[Ligatures=TeX]
\setsansfont[Scale=0.9]{ Brand Sans }
\setmonofont{texgyrecursor-regular.otf}
\setCJKmainfont{芫荽}
\setCJKsansfont[BoldFont={Noto Sans CJK TC Bold}]{微軟正黑體}
\newfontfamily\headfont{Some Display Face}
\newCJKfontfamily[kai]\kaishu{標楷體}
\newfontfamily{\code}[Scale=0.8]
  {Consolas}
\setmathfont{Cambria Math}
\input{notation}
\begin{document}
\setmainfont{Not In Preamble}
\end{document}
";

    #[test]
    fn finds_every_font_command_and_skips_comments() {
        let u = find_fonts("main.tex", PREAMBLE);
        let names: Vec<(&str, &str, u32)> = u.iter().map(|x| (x.cmd.as_str(), x.name.as_str(), x.line)).collect();
        assert_eq!(
            names,
            [
                ("setmainfont", "Calibri", 6),
                ("setsansfont", "Brand Sans", 7),
                ("setmonofont", "texgyrecursor-regular.otf", 8),
                ("setCJKmainfont", "芫荽", 9),
                ("setCJKsansfont", "微軟正黑體", 10),
                ("newfontfamily", "Some Display Face", 11),
                ("newCJKfontfamily", "標楷體", 12),
                ("newfontfamily", "Consolas", 13),
                ("setmathfont", "Cambria Math", 15),
                ("setmainfont", "Not In Preamble", 18),
            ]
        );
        assert!(u[2].is_file_or_macro() && !u[0].is_file_or_macro());
        assert!(find_fonts("x.tex", r"\setmainfont[Path=fonts/]{Foo}").iter().all(|u| u.is_file_or_macro()));
        assert!(find_fonts("x.tex", r"\setmainfontx{Foo} \setmainfont\fontname").is_empty());
    }

    fn here() -> Installed {
        let mut i = Installed::from_names(["Carlito", "Noto Sans CJK TC", "STIX Two Math", "DejaVu Sans Mono", "Iansui"]);
        i.private = vec![
            PrivateFace { families: vec!["Iansui".into(), "芫荽".into()], file: "Iansui-Regular.ttf".into(), weight: 400, italic: false },
            PrivateFace { families: vec!["Iansui".into(), "芫荽".into()], file: "Iansui-Bold.ttf".into(), weight: 700, italic: false },
        ];
        i
    }

    #[test]
    fn rewrites_a_copy_with_substitutes_and_private_files() {
        let mut rules = std::collections::BTreeMap::new();
        rules.insert("Brand Sans".to_string(), vec!["Brand Sans".to_string(), "Calibri".to_string(), "Carlito".to_string()]);
        let table = Table::with_user(&rules);
        let inst = here();
        let dir = Path::new("/fonts");
        let uses = find_fonts("main.tex", PREAMBLE);
        let fixes: Vec<(&FontUse, Fix)> = uses
            .iter()
            .filter(|u| !u.is_file_or_macro())
            .map(|u| (u, fix_for(&lore_fonts::resolve(&table, &inst, &u.name), &inst, Some(dir))))
            .collect();
        let by = |n: &str| fixes.iter().find(|(u, _)| u.name == n).map(|(_, f)| f.clone()).unwrap();
        assert_eq!(by("Calibri"), Fix::Rename("Carlito".into()));
        assert_eq!(by("Brand Sans"), Fix::Rename("Carlito".into()), "the author's rule");
        assert_eq!(by("微軟正黑體"), Fix::Rename("Noto Sans CJK TC".into()));
        assert_eq!(by("Cambria Math"), Fix::Rename("STIX Two Math".into()));
        assert_eq!(by("芫荽"), Fix::Files { name: "Iansui-Regular.ttf".into(), opts: "Path={/fonts/},BoldFont={Iansui-Bold.ttf}".into() });
        let refs: Vec<(&FontUse, &Fix)> = fixes.iter().map(|(u, f)| (*u, f)).collect();
        let out = rewrite(PREAMBLE, &refs);
        assert!(out.contains(r"\setmainfont{Carlito}[Ligatures=TeX]"));
        assert!(out.contains(r"\setsansfont[Scale=0.9]{ Carlito }"));
        assert!(out.contains(r"\setmonofont{texgyrecursor-regular.otf}"), "file names stay");
        assert!(out.contains(r"\setCJKmainfont{Iansui-Regular.ttf}[Path={/fonts/},BoldFont={Iansui-Bold.ttf}]"));
        assert!(out.contains(r"\setCJKsansfont[BoldFont={Noto Sans CJK TC Bold}]{Noto Sans CJK TC}"));
        assert!(out.contains(r"\setmathfont{STIX Two Math}"));
        assert!(out.contains(r"% \setmainfont{Commented Out}"));
        assert!(out.contains(r"\input{notation}"));
        assert_eq!(out.lines().count(), PREAMBLE.lines().count(), "line numbers in errors stay right");
        // options after the name get the private-file keys first
        let t = r"\setCJKmainfont{芫荽}[AutoFakeSlant]";
        let u = find_fonts("x", t);
        let f = fix_for(&lore_fonts::resolve(&table, &inst, "芫荽"), &inst, Some(dir));
        assert_eq!(rewrite(t, &[(&u[0], &f)]), r"\setCJKmainfont{Iansui-Regular.ttf}[Path={/fonts/},BoldFont={Iansui-Bold.ttf},AutoFakeSlant]");
    }

    fn paper(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-texfonts-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sections")).unwrap();
        d
    }

    #[test]
    fn prepare_copies_only_what_changes_and_leaves_the_paper_alone() {
        let d = paper("prep");
        let main = "\\documentclass{article}\n\\usepackage{fontspec}\n\\input{preamble}\n\\usepackage{mystyle,amsmath}\n\\input{notation}\n\\begin{document}\n\\input{sections/01-intro}\n\\end{document}\n";
        let notation = "% generated\n\\newcommand{\\ribmask}{M_{\\mathrm{rib}}}\n";
        std::fs::write(d.join("main.tex"), main).unwrap();
        std::fs::write(d.join("preamble.tex"), "\\setmainfont{Calibri}\n").unwrap();
        std::fs::write(d.join("mystyle.sty"), "\\setmonofont{DejaVu Sans Mono}\n").unwrap();
        std::fs::write(d.join("notation.tex"), notation).unwrap();
        std::fs::write(d.join("sections/01-intro.tex"), "\\section{Intro}\nText $\\ribmask$.\n").unwrap();
        assert_eq!(preamble_files(&d), ["main.tex", "preamble.tex", "notation.tex", "mystyle.sty"]);
        let work = d.join("build");
        let p = prepare(&d, &work, &Table::defaults(), &here(), None).unwrap();
        assert!(p.main.is_none(), "main.tex names no font");
        assert_eq!(std::fs::read_to_string(work.join(".fonts/preamble.tex")).unwrap(), "\\setmainfont{Carlito}\n");
        assert!(!work.join(".fonts/mystyle.sty").exists() && !work.join(".fonts/notation.tex").exists());
        assert_eq!(p.notes.len(), 1);
        assert!(p.notes[0].contains("「Calibri」→「Carlito」"), "{:?}", p.notes);
        assert!(p.missing.is_empty());
        // the author's files are untouched
        assert_eq!(std::fs::read_to_string(d.join("preamble.tex")).unwrap(), "\\setmainfont{Calibri}\n");
        assert_eq!(std::fs::read_to_string(d.join("notation.tex")).unwrap(), notation);
        assert_eq!(std::fs::read_to_string(d.join("main.tex")).unwrap(), main);

        // nothing in the chain: reported, with where
        std::fs::write(d.join("main.tex"), main.replace("\\input{preamble}", "\\setCJKmainfont{華康某體}")).unwrap();
        let p = prepare(&d, &work, &Table::defaults(), &Installed::from_names(["Carlito", "DejaVu Sans Mono"]), None).unwrap();
        assert_eq!(p.missing.len(), 1);
        assert_eq!((p.missing[0].0.as_str(), p.missing[0].1), ("main.tex", 3));
        assert!(p.missing[0].2.contains("華康某體"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn notation_checks_see_the_same_paper_after_the_font_rewrite() {
        use lore_notation::{check_document, CheckOptions, Document, Notation};
        let d = paper("notation");
        let main = "\\documentclass{article}\n\\usepackage{fontspec}\n\\setmainfont{Calibri}\n\\input{notation}\n\\begin{document}\n\\input{sections/01-intro}\n\\end{document}\n";
        std::fs::write(d.join("main.tex"), main).unwrap();
        std::fs::write(d.join("notation.tex"), "\\newcommand{\\ribmask}{M_{\\mathrm{rib}}}\n").unwrap();
        std::fs::write(d.join("sections/01-intro.tex"), "\\section{Intro}\nWe use the CNN here, see Table 1 and $\\ribmask x$.\n").unwrap();
        let work = d.join("build");
        let p = prepare(&d, &work, &Table::defaults(), &here(), None).unwrap();
        let copy = p.main.expect("main.tex rewritten");
        // the compiled copy next to the same other files
        let d2 = paper("notation2");
        std::fs::copy(&copy, d2.join("main.tex")).unwrap();
        std::fs::copy(d.join("notation.tex"), d2.join("notation.tex")).unwrap();
        std::fs::copy(d.join("sections/01-intro.tex"), d2.join("sections/01-intro.tex")).unwrap();
        let reg = Notation::default();
        let codes = |dir: &Path| {
            let r = check_document(Document::load(dir).unwrap(), &reg, &CheckOptions::default());
            r.issues.iter().map(|i| (i.code.clone(), i.message.clone())).collect::<Vec<_>>()
        };
        let (a, b) = (codes(&d), codes(&d2));
        assert!(!a.is_empty(), "the synthetic text has something to report");
        assert_eq!(a, b);
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&d2);
    }
}
