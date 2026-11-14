//! The author's notation registry (lore-notation) for generated text: what
//! the drafting and revision prompts are told (macros, labels,
//! abbreviations), and the registry's checks run on what they write.
//!
//! The registry is the author's one global file (`private/shared/notation.json`,
//! shared with the other apps; `profile/notation.json` only when LoreForge has
//! no private folder) merged with `papers/<p>/notation.json` (this paper); the
//! paper file wins. An older `profile/notation.json` is merged into the global
//! file once (never deleted; clashes are reported, the global row is kept).

use axum::{
    extract::{Path as UrlPath, State},
    http::header,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use lore_notation::{check_document, CheckOptions, Document, Entry, Ignored, Issue, Kind, Notation, Origin, RefWords};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::Shared;

/// Findings in generated text that must be fixed before it is kept.
const BLOCKING: &[&str] = &["hardcoded_reference", "ref_undefined"];
/// Findings worth showing the author next to the generated text.
const SHOWN: &[&str] = &[
    "hardcoded_reference",
    "ref_undefined",
    "ref_word_style",
    "abbrev_never_expanded",
    "abbrev_used_before_expansion",
    "undefined_symbol",
    "used_before_definition",
    "symbol_multiple_meanings",
];

/// The global registry file: `shared/notation.json` through lore-home; the
/// workspace's `profile/notation.json` when there is no shared folder.
pub fn global_path(workspace: &Path) -> PathBuf {
    shared_path().unwrap_or_else(|| old_path(workspace))
}

fn shared_path() -> Option<PathBuf> {
    crate::loreforge_home().shared_dir().map(|d| d.join("notation.json"))
}

fn old_path(workspace: &Path) -> PathBuf {
    workspace.join("profile/notation.json")
}

fn marker_path(workspace: &Path) -> PathBuf {
    workspace.join("profile/notation.migrated.json")
}

/// A row of the old profile file that differs from the global row it matches.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Clash {
    pub id: String,
    /// What stays (the global row).
    pub kept: Entry,
    /// What the old file had.
    pub old: Entry,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Migration {
    pub added: Vec<String>,
    pub same: Vec<String>,
    pub clashes: Vec<Clash>,
    /// The author has seen the clashes.
    #[serde(default)]
    pub seen: bool,
}

fn same_but_id(a: &Entry, b: &Entry) -> bool {
    let strip = |e: &Entry| {
        let mut v = serde_json::to_value(e).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            o.remove("id");
        }
        v
    };
    strip(a) == strip(b)
}

/// The global row that means the same thing as `e` (same id, same symbol LaTeX, same abbreviation).
fn counterpart<'a>(glob: &'a Notation, e: &Entry) -> Option<&'a Entry> {
    if let Some(g) = glob.get(&e.id) {
        return Some(g);
    }
    match e.kind() {
        Kind::Symbol | Kind::Operator => e.latex.as_deref().and_then(|l| glob.find_latex(l)).filter(|g| g.kind() == e.kind()),
        Kind::Abbrev => glob.entries.iter().find(|g| g.kind() == Kind::Abbrev && e.short.is_some() && g.short == e.short),
        _ => None,
    }
}

/// Merge `old` into `glob` (rows only the old file has are added; a global row always wins).
pub fn merge_old(glob: &mut Notation, old: &Notation) -> Migration {
    let mut m = Migration::default();
    for e in &old.entries {
        match counterpart(glob, e).cloned() {
            None => {
                m.added.push(e.id.clone());
                glob.entries.push(e.clone());
            }
            Some(g) if same_but_id(&g, e) => m.same.push(g.id.clone()),
            Some(g) => m.clashes.push(Clash { id: g.id.clone(), kept: g, old: e.clone() }),
        }
    }
    for c in &old.conventional {
        if !glob.conventional.contains(c) {
            glob.conventional.push(c.clone());
        }
    }
    for i in &old.ignored {
        if !glob.ignored.contains(i) {
            glob.ignored.push(i.clone());
        }
    }
    m
}

/// Is there an old `profile/notation.json` not yet merged into the global file?
fn pending(workspace: &Path) -> bool {
    shared_path().is_some() && old_path(workspace).is_file() && !marker_path(workspace).exists()
}

/// Once per workspace: merge the old profile file into the global one and
/// leave a marker (with the report) in `profile/`. Returns the report when it ran.
pub fn migrate(workspace: &Path) -> Result<Option<Migration>, String> {
    let Some(shared) = shared_path() else { return Ok(None) };
    migrate_files(&old_path(workspace), &shared, &marker_path(workspace))
}

pub fn migrate_files(old: &Path, shared: &Path, marker: &Path) -> Result<Option<Migration>, String> {
    if !old.is_file() || marker.exists() {
        return Ok(None);
    }
    let old_n = Notation::load(old)?;
    let mut glob = Notation::load(shared)?;
    let before = glob.to_json();
    let m = merge_old(&mut glob, &old_n);
    if glob.to_json() != before {
        glob.save(shared)?;
    }
    let text = serde_json::to_string_pretty(&m).map_err(|e| e.to_string())? + "\n";
    lore_notation::write_atomic(marker, &text).map_err(|e| format!("{}: {e}", marker.display()))?;
    Ok(Some(m))
}

fn migration_report(workspace: &Path) -> Option<Migration> {
    std::fs::read_to_string(marker_path(workspace)).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// The global registry as the apps see it. Before the migration has run it
/// already includes the old profile file (read only).
pub fn load_global(workspace: &Path) -> Notation {
    let mut g = Notation::load(&global_path(workspace)).unwrap_or_default();
    if pending(workspace) {
        if let Ok(old) = Notation::load(&old_path(workspace)) {
            merge_old(&mut g, &old);
        }
    }
    g
}

pub fn load(workspace: &Path, paper: &Path) -> Notation {
    let global = load_global(workspace);
    let doc = Notation::load(&pc_core::paper::app_dir(paper).join("notation.json")).unwrap_or_default();
    Notation::effective(&global, &doc)
}

/// The reference words of the paper's journal (`[ref_words]` in its
/// template, or cleveref when its sample loads it); None when the paper has
/// no journal or the journal sets nothing.
pub fn ref_words(workspace: &Path, paper: &Path) -> Option<RefWords> {
    let j = pc_core::meta::read_meta(paper).journal?;
    let rw = pc_core::journal::load(&workspace.join("publishers"), &j)?.effective_ref_words();
    (!rw.is_empty()).then_some(rw)
}

/// Check options for the paper: the defaults plus its journal's reference words.
pub fn options(workspace: &Path, paper: &Path) -> CheckOptions {
    CheckOptions { ref_words: ref_words(workspace, paper), ..Default::default() }
}

fn kind_entries(n: &Notation, k: Kind) -> Vec<&Entry> {
    n.entries.iter().filter(|e| e.kind() == k).collect()
}

/// Prompt text: the rules, then the registry (cut to a sensible size).
/// `words`: the journal's reference words, when it sets them.
pub fn describe(n: &Notation, words: Option<&RefWords>) -> String {
    let refs = match words {
        None => "- Refer to figures, tables, equations and sections only with \\ref{label} (Table~\\ref{tab:x}, Fig.~\\ref{fig:y}, \
Eq.~\\eqref{eq:z}); never type a number such as \"Table 1\" or \"Section 3\".\n"
            .to_string(),
        Some(w) if w.cleveref() => "- Refer to figures, tables, equations and sections only with \\cref{label} (\\Cref{label} at the start of a \
sentence), with no word before it: the journal's class prints \"Figure\", \"Table\", …; never type a number such as \"Table 1\" or \"Section 3\".\n"
            .to_string(),
        Some(w) => {
            let mut forms: Vec<String> = ["tab", "fig", "eq", "sec"].iter().map(|k| w.snippet(k, &format!("{k}:x"), false)).collect();
            for (k, _) in &w.sentence_start {
                forms.push(format!("{} at the start of a sentence", w.snippet(k, &format!("{k}:x"), true)));
            }
            format!(
                "- Refer to figures, tables, equations and sections only with \\ref{{label}}, written as the journal does: {}; \
never type a number such as \"Table 1\" or \"Section 3\".\n",
                forms.join(", ")
            )
        }
    };
    let mut s = String::from(
        "\n## The author's notation registry — follow it in every piece of LaTeX\n\
- Write a registered symbol with its macro when it has one (\\vw, not \\boldsymbol{w}), else with its registered LaTeX.\n",
    );
    s += &refs;
    s += "- Expand an abbreviation at its first use in the body unless the registry marks it conventional.\n\
- A symbol the registry does not have must be defined in the sentence where it first appears; prefer an existing one.\n\
- Every piece of math, even a single symbol or an assignment such as $\\lambda = 0.5$, goes inside $…$.\n";
    let line = |e: &Entry| -> String {
        let what = e.meaning_en.as_deref().or(e.meaning.as_deref()).unwrap_or("");
        match (e.macro_name(), e.latex.as_deref()) {
            (Some(m), Some(l)) => format!("\\{m} = {l} — {what}"),
            (Some(m), None) => format!("\\{m} — {what}"),
            (None, Some(l)) => format!("{l} — {what}"),
            (None, None) => what.to_string(),
        }
    };
    let mut add = |title: &str, rows: Vec<String>| {
        if !rows.is_empty() {
            s += &format!("{title}:\n");
            for r in rows.into_iter().take(80) {
                s += &format!("- {r}\n");
            }
        }
    };
    add("Symbols", kind_entries(n, Kind::Symbol).into_iter().map(line).collect());
    add("Operators", kind_entries(n, Kind::Operator).into_iter().map(line).collect());
    add(
        "Equations (label — body)",
        kind_entries(n, Kind::Equation).into_iter().filter_map(|e| Some(format!("{} — {}", e.label.as_deref()?, e.latex.as_deref().unwrap_or("")))).collect(),
    );
    add(
        "Figures and tables (label — caption)",
        n.entries
            .iter()
            .filter(|e| matches!(e.kind(), Kind::Figure | Kind::Table))
            .filter_map(|e| Some(format!("{} — {}", e.label.as_deref()?, e.caption.as_deref().unwrap_or(""))))
            .collect(),
    );
    add(
        "Abbreviations (short — long)",
        kind_entries(n, Kind::Abbrev)
            .into_iter()
            .filter_map(|e| Some(format!("{} — {}{}", e.short.as_deref()?, e.long.as_deref().unwrap_or("?"), e.macro_name().map(|m| format!(" (macro \\{m})")).unwrap_or_default())))
            .collect(),
    );
    if !n.conventional.is_empty() {
        s += &format!("Conventional (no definition or expansion needed): {}\n", n.conventional.join(", "));
    }
    s
}

/// What the registry's checks say about `text`, read in the context of the
/// paper (`main.tex` and its sections) plus `others` (generated text not in
/// the paper yet). Only findings located in `text` are returned.
pub struct Checked {
    pub blocking: Vec<String>,
    pub shown: Vec<String>,
    /// LaTeX of symbols the registry does not have.
    pub new_symbols: Vec<String>,
}

const FRAGMENT: &str = "sections/zz-generated.tex";

pub fn check(paper: &Path, n: &Notation, opt: &CheckOptions, others: &[String], text: &str) -> Checked {
    let rel = |p: &Path| p.strip_prefix(paper).unwrap_or(p).to_string_lossy().replace('\\', "/");
    let sources = pc_core::open::source_files(paper);
    let main_name = sources.first().map(|p| rel(p)).unwrap_or_else(|| "main.tex".into());
    let mut main = std::fs::read_to_string(pc_core::open::main_file(paper)).unwrap_or_else(|_| "\\documentclass{article}\n\\begin{document}\n\\end{document}\n".into());
    let mut files: Vec<(String, String)> = vec![];
    for p in sources.iter().skip(1) {
        if let Ok(t) = std::fs::read_to_string(p) {
            files.push((rel(p), t));
        }
    }
    let mut extra = String::new();
    for (i, o) in others.iter().enumerate() {
        let name = format!("sections/zz-other-{i}.tex");
        extra += &format!("\\input{{{}}}\n", name.trim_end_matches(".tex"));
        files.push((name, o.clone()));
    }
    extra += &format!("\\input{{{}}}\n", FRAGMENT.trim_end_matches(".tex"));
    files.push((FRAGMENT.into(), text.to_string()));
    match main.rfind("\\end{document}") {
        Some(i) => main.insert_str(i, &extra),
        None => main += &extra,
    }
    let mut all = vec![(main_name, main)];
    all.extend(files);
    let report = check_document(Document::from_files(&all), n, opt);
    let here = |i: &&Issue| i.locations.iter().any(|l| l.file == FRAGMENT);
    let msg = |i: &Issue| i.message.clone();
    let blocking = report.issues.iter().filter(here).filter(|i| BLOCKING.contains(&i.code.as_str())).map(msg).collect();
    let shown = report.issues.iter().filter(here).filter(|i| SHOWN.contains(&i.code.as_str())).map(msg).collect();
    let mut new_symbols: Vec<String> = report
        .issues
        .iter()
        .filter(here)
        .filter(|i| i.code == "unregistered_symbol")
        .flat_map(|i| if i.subjects.is_empty() { i.subject.clone().into_iter().collect() } else { i.subjects.clone() })
        .collect();
    new_symbols.sort();
    new_symbols.dedup();
    Checked { blocking, shown, new_symbols }
}

/// Add symbols to the paper's own registry, tagged `suggested` for review.
/// Returns how many were new.
pub fn register(paper: &Path, latex: &[String]) -> Result<usize, String> {
    let path = pc_core::paper::app_dir(paper).join("notation.json");
    let mut n = Notation::load(&path)?;
    let mut added = 0;
    for l in latex.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        if n.find_latex(l).is_some() {
            continue;
        }
        let mut e = Entry::new(&n.free_id("s"), Kind::Symbol);
        e.latex = Some(l.to_string());
        e.tags = vec!["suggested".into()];
        n.upsert(e);
        added += 1;
    }
    if added > 0 {
        n.save(&path)?;
    }
    Ok(added)
}

// ── 對照表 screen: the shared <notation-panel> over the global + paper registries ──

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/notation-panel.js", get(panel_js))
        .route("/api/papers/{name}/notation", get(notation_get).put(notation_put))
        .route("/api/papers/{name}/notation/seen", post(notation_seen))
}

/// The shared notation-panel Web Component (commons/ui), compiled in.
async fn panel_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], include_str!("../../../../commons/ui/notation-panel/notation-panel.js"))
}

fn glob_and_doc(workspace: &Path, paper: &Path) -> (Notation, Notation) {
    (load_global(workspace), Notation::load(&pc_core::paper::app_dir(paper).join("notation.json")).unwrap_or_default())
}

/// What the panel shows: merged rows (each with `origin`), the check report on
/// the whole paper, the commands the paper defines, and the migration note.
pub fn view(workspace: &Path, paper: &Path, glob: &Notation, doc: &Notation) -> Value {
    let eff = Notation::effective(glob, doc);
    let mut n = serde_json::to_value(&eff).unwrap_or(json!({}));
    n["entries"] = Value::Array(
        Notation::merged(glob, doc)
            .into_iter()
            .map(|m| {
                let mut e = serde_json::to_value(&m.entry).unwrap_or(json!({}));
                e["origin"] = json!(if m.origin == Origin::Global { "global" } else { "document" });
                e
            })
            .collect(),
    );
    let (report, macros) = match Document::load(paper) {
        Ok(d) => {
            let macros: Vec<String> = lore_notation::scan::document_macros(&d.text).keys().cloned().collect();
            (serde_json::to_value(check_document(d, &eff, &options(workspace, paper))).unwrap_or(Value::Null), macros)
        }
        Err(_) => (Value::Null, vec![]),
    };
    let migration = migration_report(workspace).filter(|m| !m.clashes.is_empty() && !m.seen);
    json!({ "notation": n, "report": report, "macros": macros, "migration": migration })
}

fn keep_in<T: PartialEq + Clone>(old: &[T], still: &[T]) -> Vec<T> {
    old.iter().filter(|x| still.contains(x)).cloned().collect()
}

/// Split what the panel sends back into the global file and the paper file.
/// Rows tagged `global` go to the global file (a global row hidden behind a
/// paper row of the same thing is left alone); everything else is the paper's.
pub fn split(panel: &Value, glob_old: &Notation, doc_old: &Notation) -> Result<(Notation, Notation), String> {
    let mut from_glob: Vec<Entry> = vec![];
    let mut from_doc: Vec<Entry> = vec![];
    for mut e in panel.get("entries").and_then(Value::as_array).cloned().unwrap_or_default() {
        let global = e.get("origin").and_then(Value::as_str) == Some("global");
        if let Some(o) = e.as_object_mut() {
            o.remove("origin");
        }
        let entry: Entry = serde_json::from_value(e).map_err(|e| e.to_string())?;
        if global { from_glob.push(entry) } else { from_doc.push(entry) }
    }
    let shown: Vec<String> = Notation::merged(glob_old, doc_old).into_iter().filter(|m| m.origin == Origin::Global).map(|m| m.entry.id).collect();
    let mut glob = glob_old.clone();
    glob.entries = glob_old
        .entries
        .iter()
        .filter_map(|g| if shown.contains(&g.id) { from_glob.iter().find(|e| e.id == g.id).cloned() } else { Some(g.clone()) })
        .collect();
    for e in from_glob {
        if glob_old.get(&e.id).is_none() {
            glob.entries.push(e);
        }
    }
    let mut doc = doc_old.clone();
    doc.entries = from_doc;
    let conv: Vec<String> = panel.get("conventional").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    glob.conventional = keep_in(&glob_old.conventional, &conv);
    doc.conventional = conv.into_iter().filter(|c| !glob_old.conventional.contains(c)).collect();
    let ign: Vec<Ignored> = panel.get("ignored").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    glob.ignored = keep_in(&glob_old.ignored, &ign);
    doc.ignored = ign.into_iter().filter(|i| !glob_old.ignored.contains(i)).collect();
    Ok((glob, doc))
}

type Resp = axum::response::Response;

async fn context(s: &Shared, name: &str) -> Result<(PathBuf, PathBuf), Resp> {
    let dir = crate::paper_dir(s, name).await?;
    let ws = s.loaded.read().await.workspace.clone().ok_or_else(|| crate::bad("no workspace"))?;
    Ok((ws, dir))
}

async fn notation_get(State(s): State<Shared>, UrlPath(name): UrlPath<String>) -> Result<Json<Value>, Resp> {
    let (ws, dir) = context(&s, &name).await?;
    migrate(&ws).map_err(|e| crate::bad(&e))?;
    let (g, d) = glob_and_doc(&ws, &dir);
    Ok(Json(view(&ws, &dir, &g, &d)))
}

async fn notation_put(State(s): State<Shared>, UrlPath(name): UrlPath<String>, Json(b): Json<Value>) -> Result<Json<Value>, Resp> {
    let (ws, dir) = context(&s, &name).await?;
    migrate(&ws).map_err(|e| crate::bad(&e))?;
    let (g0, d0) = glob_and_doc(&ws, &dir);
    let (g1, d1) = split(&b, &g0, &d0).map_err(|e| crate::bad(&e))?;
    if g1.to_json() != g0.to_json() {
        g1.save(&global_path(&ws)).map_err(|e| crate::bad(&e))?;
    }
    if d1.to_json() != d0.to_json() {
        d1.save(&pc_core::paper::app_dir(&dir).join("notation.json")).map_err(|e| crate::bad(&e))?;
    }
    let (g, d) = glob_and_doc(&ws, &dir);
    Ok(Json(view(&ws, &dir, &g, &d)))
}

/// The author has seen the migration clashes.
async fn notation_seen(State(s): State<Shared>, UrlPath(name): UrlPath<String>) -> Result<Json<Value>, Resp> {
    let (ws, _) = context(&s, &name).await?;
    if let Some(mut m) = migration_report(&ws) {
        m.seen = true;
        let text = serde_json::to_string_pretty(&m).map_err(|e| crate::bad(&e.to_string()))? + "\n";
        lore_notation::write_atomic(&marker_path(&ws), &text).map_err(|e| crate::bad(&e.to_string()))?;
    }
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paper() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("pc-notation-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(d.join("sections")).unwrap();
        std::fs::write(d.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/results}\n\\end{document}\n").unwrap();
        std::fs::write(
            d.join("sections/results.tex"),
            "\\section{Results}\nThe weight $\\vw$ scales each frame.\n\n\\begin{table}\n\\caption{Main comparison.}\n\\label{tab:main}\n\\begin{tabular}{l}\nx\n\\end{tabular}\n\\end{table}\n",
        )
        .unwrap();
        std::fs::write(
            d.join("notation.json"),
            r#"{"version":1,"entries":[{"id":"w","kind":"symbol","latex":"\\boldsymbol{w}","macro":"vw","meaning_en":"frame weight"},{"id":"t1","kind":"table","label":"tab:main","caption":"Main comparison"},{"id":"cv","kind":"abbrev","short":"CV","long":"cross-validation"}]}"#,
        )
        .unwrap();
        d
    }

    #[test]
    fn prompts_carry_the_rules_and_the_registry() {
        let p = paper();
        let n = load(&p, &p);
        let d = describe(&n, None);
        assert!(d.contains("Table~\\ref{tab:x}, Fig.~\\ref{fig:y}, Eq.~\\eqref{eq:z}"), "the default rule as before");
        assert!(d.contains("\\vw = \\boldsymbol{w} — frame weight"));
        let w = RefWords { fig: Some("Figure".into()), sentence_start: [("fig".to_string(), "Figure".to_string())].into(), ..Default::default() };
        let j = describe(&n, Some(&w));
        assert!(j.contains("Figure~\\ref{fig:x}") && j.contains("\\eqref{eq:x}") && !j.contains("Fig.~"), "{j}");
        let c = describe(&n, Some(&RefWords { mode: Some("cleveref".into()), ..Default::default() }));
        assert!(c.contains("\\cref{label}") && c.contains("\\Cref{label}"));
        assert!(d.contains("tab:main — Main comparison") && d.contains("CV — cross-validation"));
        assert!(d.contains("never type a number"));
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn drafted_pending_marks_pass_the_registry_checks() {
        // what the draft prompt asks for: marks with ids, a dependent sentence, a planned paragraph
        let p = paper();
        let n = Notation::default();
        let text = "On the external site, the weighted loss reaches a Dice of \\pending{P1}{external Dice}, \
which keeps it ahead of the baseline\\pendingdep{P1}.\n\n\\pendingpara{P2}{Per-site comparison; the table goes after the main one.}";
        let c = check(&p, &n, &CheckOptions::default(), &[], text);
        assert!(c.blocking.is_empty() && c.shown.is_empty() && c.new_symbols.is_empty(), "{:?} {:?}", c.blocking, c.shown);
        // a typed number still blocks, marks or not
        let c = check(&p, &n, &CheckOptions::default(), &[], "As Table 1 shows, Dice is \\pending{P1}{external Dice}.");
        assert!(!c.blocking.is_empty());
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn typed_references_block_and_new_symbols_are_found() {
        let p = paper();
        let n = load(&p, &p);
        let opt = CheckOptions::default();
        let bad = check(&p, &n, &opt, &[], "Table 1 shows that $\\vw$ and the new margin $\\gamma$ matter.\n");
        assert!(!bad.blocking.is_empty(), "a typed 'Table 1' must be fixed");
        let good = check(&p, &n, &opt, &[], "Table~\\ref{tab:main} shows that $\\vw$ matters.\n");
        assert!(good.blocking.is_empty(), "{:?}", good.blocking);
        // a journal that writes "Tab." shows the difference, without blocking
        let tab = CheckOptions { ref_words: Some(RefWords { tab: Some("Tab.".into()), ..Default::default() }), ..Default::default() };
        let j = check(&p, &n, &tab, &[], "Table~\\ref{tab:main} shows that $\\vw$ matters.\n");
        assert!(j.blocking.is_empty() && j.shown.iter().any(|m| m.contains("Tab.")), "{:?}", j.shown);
        assert_eq!(register(&p, &bad.new_symbols).unwrap(), bad.new_symbols.len());
        let reg = Notation::load(&p.join("notation.json")).unwrap();
        for s in &bad.new_symbols {
            assert!(reg.find_latex(s).is_some_and(|e| e.has_tag("suggested")));
        }
        assert_eq!(register(&p, &bad.new_symbols).unwrap(), 0, "registered once");
        let _ = std::fs::remove_dir_all(&p);
    }

    fn n(s: &str) -> Notation {
        Notation::from_json(s).unwrap()
    }

    #[test]
    fn migration_merges_once_keeps_the_old_file_and_reports_clashes() {
        let d = std::env::temp_dir().join(format!("pc-notation-mig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("profile")).unwrap();
        let (old, shared, marker) = (d.join("profile/notation.json"), d.join("shared/notation.json"), d.join("profile/notation.migrated.json"));
        let old_text = r#"{"version":1,"entries":[
            {"id":"w","kind":"symbol","latex":"\\boldsymbol{w}","macro":"vw","meaning_en":"weight"},
            {"id":"x","kind":"symbol","latex":"x","meaning_en":"input"},
            {"id":"cv","kind":"abbrev","short":"CV","long":"cross-validation"}],"conventional":["AI"]}"#;
        std::fs::write(&old, old_text).unwrap();
        std::fs::create_dir_all(d.join("shared")).unwrap();
        std::fs::write(&shared, r#"{"version":1,"entries":[{"id":"w","kind":"symbol","latex":"\\boldsymbol{w}","macro":"vw","meaning_en":"frame weight"},{"id":"x","kind":"symbol","latex":"x","meaning_en":"input"}]}"#).unwrap();
        let m = migrate_files(&old, &shared, &marker).unwrap().unwrap();
        assert_eq!(m.added, ["cv"]);
        assert_eq!(m.same, ["x"]);
        assert_eq!(m.clashes.len(), 1);
        assert_eq!(m.clashes[0].kept.meaning_en.as_deref(), Some("frame weight"), "the global row is kept");
        let g = Notation::load(&shared).unwrap();
        assert_eq!(g.entries.len(), 3);
        assert_eq!(g.get("w").unwrap().meaning_en.as_deref(), Some("frame weight"));
        assert_eq!(g.conventional, ["AI"]);
        assert_eq!(std::fs::read_to_string(&old).unwrap(), old_text, "the old file is never touched");
        // once: a row deleted from the global file does not come back
        let mut g2 = g.clone();
        g2.remove("cv");
        g2.save(&shared).unwrap();
        assert!(migrate_files(&old, &shared, &marker).unwrap().is_none());
        assert!(Notation::load(&shared).unwrap().get("cv").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn saving_splits_rows_between_the_global_and_the_paper_file() {
        let glob = n(r#"{"entries":[{"id":"g1","kind":"symbol","latex":"a","meaning_en":"A"},{"id":"g2","kind":"symbol","latex":"b","meaning_en":"B"},{"id":"g3","kind":"symbol","latex":"c","meaning_en":"C"}],"conventional":["AI"]}"#);
        // the paper overrides g3, so the panel does not show the global g3
        let doc = n(r#"{"entries":[{"id":"d3","kind":"symbol","latex":"c","meaning_en":"mine"}]}"#);
        let panel = json!({"entries":[
            {"id":"g1","kind":"symbol","latex":"a","meaning_en":"A edited","origin":"global"},
            {"id":"d3","kind":"symbol","latex":"c","meaning_en":"mine","origin":"document"},
            {"id":"new","kind":"symbol","latex":"q","origin":"document"},
            {"id":"ng","kind":"symbol","latex":"r","origin":"global"}],
            "conventional":["AI","BERT"]});
        let (g, d) = split(&panel, &glob, &doc).unwrap();
        let ids = |x: &Notation| x.entries.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&g), ["g1", "g3", "ng"], "g2 was deleted, g3 (hidden) is untouched");
        assert_eq!(g.get("g1").unwrap().meaning_en.as_deref(), Some("A edited"));
        assert_eq!(ids(&d), ["d3", "new"]);
        assert_eq!(g.conventional, ["AI"]);
        assert_eq!(d.conventional, ["BERT"]);
        assert!(d.entries.iter().all(|e| e.kind.is_some()));
    }

    #[test]
    fn the_paper_file_keeps_its_bytes_when_nothing_in_it_changed() {
        let p = paper();
        let before = std::fs::read_to_string(p.join("notation.json")).unwrap();
        let g = Notation::default();
        let d = Notation::load(&p.join("notation.json")).unwrap();
        let view = view(&p, &p, &g, &d);
        let (g1, d1) = split(&view["notation"], &g, &d).unwrap();
        assert_eq!(d1.to_json(), d.to_json());
        assert!(g1.entries.is_empty());
        assert_eq!(std::fs::read_to_string(p.join("notation.json")).unwrap(), before);
        assert!(view["report"].is_object(), "the paper is checked as a whole");
        let _ = std::fs::remove_dir_all(&p);
    }
}
