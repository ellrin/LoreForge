//! 圖表規劃: which figures and tables the paper will have, proposed while the question groups
//! are planned (one call, in parallel with it): each with what it supports, the record items it
//! stands on (checked by code), the tool that makes it (流程圖 lore-flow, 繪圖 lore-plot, a
//! table, or the author's own image such as Grad-CAM) and 2–3 ways to draw it. The author keeps,
//! removes, picks a rendering or says their own; after 主張 the code ties each to the claims it
//! backs (none: flagged). The outline and the drafts refer to the kept ones by `\label`/`\ref`;
//! a result still to come stays `\pending`. The counts are checked against the journal's habit
//! (a warning, never a block).

use super::session::Session;
use crate::compose::{json_array, json_object, json_string, Prompt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const TASK: &str = "pc-draft-figures";
pub const TOOLS: &[&str] = &["flow", "plot", "table", "image"];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    #[default]
    Proposed,
    Kept,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PlannedFloat {
    /// F1, F2 … / T1, T2 … (the code's).
    #[serde(default)]
    pub id: String,
    /// `figure` or `table`.
    pub kind: String,
    /// `fig:…` / `tab:…`: what the drafts `\ref`.
    pub label: String,
    /// What it shows (for the author).
    pub title: String,
    /// The caption to start from (English).
    #[serde(default)]
    pub caption: String,
    /// What it supports in the paper's argument.
    #[serde(default)]
    pub supports: String,
    /// Record items it stands on (E2, R1, T3, H1, P1), only ones that exist.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// `flow`, `plot`, `table` or `image` (the author's own: Grad-CAM, example images).
    #[serde(default)]
    pub tool: String,
    /// 2–3 ways to draw it.
    #[serde(default)]
    pub options: Vec<String>,
    /// The rendering the author picked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen: Option<usize>,
    /// 「都不是，我自己說」: the author's own way.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub own: String,
    /// Pending results it waits for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
    /// Already in the paper (that float's label).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub existing: String,
    #[serde(default)]
    pub status: Status,
    /// Claims it backs (after 主張).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claims: Vec<String>,
    /// What the code found (no record evidence, no claim).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl PlannedFloat {
    pub fn kept(&self) -> bool {
        self.status != Status::Removed
    }
    /// How it will be drawn: the author's words, the chosen option, else the first.
    pub fn rendering(&self) -> &str {
        if !self.own.trim().is_empty() {
            return self.own.trim();
        }
        self.chosen.and_then(|k| self.options.get(k)).or(self.options.first()).map(String::as_str).unwrap_or("")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FigurePlan {
    pub items: Vec<PlannedFloat>,
    /// The journal's targets `[low, high]` (none: no journal data).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub figures: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tables: Option<[usize; 2]>,
    /// Being proposed in the background.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub running: bool,
    /// The last proposal failed (why).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub failed: String,
    /// The engine and how long the call took (seconds).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u64>,
}

impl FigurePlan {
    pub fn kept(&self) -> impl Iterator<Item = &PlannedFloat> {
        self.items.iter().filter(|i| i.kept())
    }

    /// The journal's habit against the plan (warnings, never a block).
    pub fn count_warnings(&self) -> Vec<String> {
        let mut out = vec![];
        for (kind, zh, target) in [("figure", "圖", self.figures), ("table", "表", self.tables)] {
            let Some([lo, hi]) = target else { continue };
            let n = self.kept().filter(|i| i.kind == kind).count();
            if n < lo {
                out.push(format!("{zh} {n} 個，期刊通常 {lo}–{hi} 個：少了"));
            } else if n > hi {
                out.push(format!("{zh} {n} 個，期刊通常 {lo}–{hi} 個：多了"));
            }
        }
        out
    }
}

pub fn schema() -> Value {
    let it = json_object(&[
        ("kind", json_string()),
        ("label", json_string()),
        ("title", json_string()),
        ("caption", json_string()),
        ("supports", json_string()),
        ("evidence", json_array(json_string())),
        ("tool", json_string()),
        ("options", json_array(json_string())),
        ("pending", json_array(json_string())),
        ("existing", json_string()),
    ]);
    json_object(&[("items", json_array(it))])
}

/// One call: the figures and tables from the record, the key value and the journal's habit.
/// `context` = the shared context (record, tables, pending results, what the paper has);
/// `venue` = [`crate::venueprofile::Profile::floats_block`] (may be empty).
pub fn prompt(context: &str, venue: &str, targets: Option<([usize; 2], [usize; 2])>) -> Prompt {
    let aim = match targets {
        Some((f, t)) => format!("Aim for {}–{} figures and {}–{} tables in total, counting those already in the paper.", f[0], f[1], t[0], t[1]),
        None => "Propose what the argument needs, usually 4–7 figures and 2–4 tables.".into(),
    };
    let system = format!(
        "You plan the figures and tables of a research paper from the study's experiment record, before the paper \
is written. Each serves the paper's argument (the key value) and the journal's habit. {aim}\n\
For each item give: `kind` figure or table; `label` (fig:short-name or tab:short-name, lowercase, hyphens); \
`title` = what it shows, for the author in Traditional Chinese (繁體中文); `caption` = a first caption in English \
(what is shown, no results); `supports` = which point of the argument it carries (繁體中文); `evidence` = ids of \
the record items it is made from (experiments E…, results tables R…, timeline steps T…, the author's thoughts H…, \
pending results P…), only ids listed below; `tool` = flow (a workflow or pipeline diagram), plot (a chart from \
numbers), table, or image (the author's own pictures: example images, Grad-CAM heatmaps); `options` = 2 or 3 ways \
to draw it, short and concrete (\"AUROC bars with 95% CI per finding\" vs \"ROC curves, one panel per site\"), in \
繁體中文; `pending` = pending ids it waits for; `existing` = the label of a table or figure already in the paper \
when the item is that one (keep it), else \"\".\n\
Figure 1 follows the journal's habit when it fits the study. Do not plan a figure the record cannot fill, except \
a workflow diagram of the method. Reply with JSON only."
    );
    let mut user = context.to_string();
    if !venue.trim().is_empty() {
        user += "\n## The journal's figures and tables\n";
        user += venue;
    }
    Prompt { system, user }
}

#[derive(Deserialize)]
struct Reply {
    items: Vec<PlannedFloat>,
}

fn slug(s: &str) -> String {
    let t: String = s.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    t.split('-').filter(|x| !x.is_empty()).take(6).collect::<Vec<_>>().join("-")
}

/// The model's list, checked: kinds and tools known, record ids that exist, pending ids that
/// exist, labels with the right prefix and unique (never one of the paper's other labels unless
/// it is that float), ids F1…/T1…, 2–3 options.
pub fn parse(reply: &str, known: &BTreeSet<String>, pending: &[String], paper_labels: &[String]) -> Result<Vec<PlannedFloat>, String> {
    let r: Reply = super::prompts::json_in(reply)?;
    let mut out: Vec<PlannedFloat> = vec![];
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let (mut nf, mut nt) = (0, 0);
    for mut it in r.items {
        it.kind = if it.kind.trim().eq_ignore_ascii_case("table") { "table".into() } else { "figure".into() };
        if it.title.trim().is_empty() {
            continue;
        }
        let prefix = if it.kind == "table" { "tab" } else { "fig" };
        it.existing = it.existing.trim().to_string();
        if !it.existing.is_empty() && !paper_labels.contains(&it.existing) {
            it.existing.clear();
        }
        let base = if !it.existing.is_empty() {
            it.existing.clone()
        } else {
            let name = it.label.split_once(':').map(|(_, n)| n).unwrap_or(&it.label);
            let s = slug(name);
            format!("{prefix}:{}", if s.is_empty() { slug(&it.caption) } else { s })
        };
        let mut label = base.clone();
        let mut k = 2;
        while taken.contains(&label) || (it.existing.is_empty() && paper_labels.contains(&label)) {
            label = format!("{base}-{k}");
            k += 1;
        }
        taken.insert(label.clone());
        it.label = label;
        if it.kind == "table" {
            nt += 1;
            it.id = format!("T{nt}");
        } else {
            nf += 1;
            it.id = format!("F{nf}");
        }
        let tool = it.tool.trim().to_lowercase();
        it.tool = if TOOLS.contains(&tool.as_str()) { tool } else if it.kind == "table" { "table".into() } else { "plot".into() };
        let before = it.evidence.len();
        it.evidence.retain(|e| known.contains(e.trim()));
        it.pending.retain(|p| pending.contains(p));
        for p in it.evidence.iter().filter(|e| pending.contains(e)).cloned().collect::<Vec<_>>() {
            if !it.pending.contains(&p) {
                it.pending.push(p);
            }
        }
        it.options.retain(|o| !o.trim().is_empty());
        it.options.truncate(3);
        it.warnings.clear();
        if it.evidence.is_empty() && it.tool != "flow" && it.existing.is_empty() {
            it.warnings.push(if before > 0 { "引用的紀錄項目不存在：先確認資料從哪裡來".into() } else { "紀錄裡找不到這張的資料".into() });
        }
        it.status = Status::Proposed;
        it.chosen = None;
        it.own.clear();
        it.claims.clear();
        out.push(it);
    }
    if out.is_empty() {
        return Err("沒有規劃出任何圖表".into());
    }
    Ok(out)
}

/// After 主張: tie each item to the claims it backs, by the record ids they share (a claim's
/// ledger items' evidence and its own evidence text) or its label; none = flagged.
pub fn reconcile(sess: &mut Session) {
    let claims: Vec<(String, BTreeSet<String>, String)> = sess
        .claims
        .iter()
        .map(|c| {
            let mut ids: BTreeSet<String> = sess.items.iter().filter(|i| c.items.contains(&i.id)).flat_map(|i| i.evidence.iter().cloned()).collect();
            ids.extend(c.evidence.split(|ch: char| !ch.is_ascii_alphanumeric()).filter(|w| w.len() >= 2).map(str::to_string));
            (c.id.clone(), ids, c.evidence.clone())
        })
        .collect();
    let Some(plan) = sess.figures.as_mut() else { return };
    for it in plan.items.iter_mut() {
        it.claims = claims.iter().filter(|(_, ids, ev)| it.evidence.iter().any(|e| ids.contains(e)) || ev.contains(&it.label)).map(|(id, _, _)| id.clone()).collect();
        it.warnings.retain(|w| !w.starts_with("沒有主張"));
        if it.claims.is_empty() && it.kept() && it.tool != "flow" && !claims.is_empty() {
            it.warnings.push("沒有主張用到這張：刪掉，或在論述裡補上它要支持的點".into());
        }
    }
}

/// The kept items for the outline and the drafts.
pub fn outline_block(sess: &Session) -> String {
    let Some(plan) = &sess.figures else { return String::new() };
    let kept: Vec<&PlannedFloat> = plan.kept().collect();
    if kept.is_empty() {
        return String::new();
    }
    let mut s = "\n## Planned figures and tables (the author's list; refer to each with \\ref{label}, never a typed number)\n".to_string();
    for it in kept {
        s += &format!("- {} [{}] {}: {} — supports: {}; drawn as: {}", it.label, it.kind, it.caption, it.title, it.supports, it.rendering());
        if !it.pending.is_empty() {
            s += &format!("; waits for {}", it.pending.join(", "));
        }
        s += "\n";
    }
    s
}

/// The planned floats a paragraph presents (by label), for its prompt.
pub fn paragraph_block(sess: &Session, labels: &[String]) -> String {
    let Some(plan) = &sess.figures else { return String::new() };
    let mine: Vec<&PlannedFloat> = plan.kept().filter(|i| labels.contains(&i.label)).collect();
    if mine.is_empty() {
        return String::new();
    }
    let mut s = "Figures and tables this paragraph presents (Fig.~\\ref{label} / Table~\\ref{label}; never write \"Figure 1\" or \"Table 2\"):\n".to_string();
    for it in mine {
        s += &format!("- {}: {}", it.label, it.caption);
        if !it.pending.is_empty() {
            s += &format!(" (its numbers wait for {}: mark them as pending)", it.pending.join(", "));
        }
        s += "\n";
    }
    s
}

/// Stand-in floats for the labels the paper does not have yet: the notation check resolves the
/// drafts' `\ref`s, and 放進論文 puts them in until the real figure or table replaces them.
/// English only (the author's options and title may be Chinese: pdflatex would stop on them).
pub fn placeholder(it: &PlannedFloat) -> String {
    let env = if it.kind == "table" { "table" } else { "figure" };
    let ascii = |t: &str| t.is_ascii() && !t.trim().is_empty();
    let from_label = it.label.split_once(':').map(|(_, n)| n).unwrap_or(&it.label).replace('-', " ");
    let caption = crate::meta::tex_text(if ascii(&it.caption) { it.caption.trim() } else { from_label.trim() });
    let boxed = format!("\\fbox{{\\parbox{{0.9\\linewidth}}{{\\centering\\vspace{{2em}}Planned {env} {}\\vspace{{2em}}}}}}", it.id);
    let cap = format!("\\caption{{{caption}}}\n\\label{{{}}}", it.label);
    // a table's caption above it, a figure's below
    if env == "table" {
        format!("\\begin{{table}}[t]\n{cap}\n\\centering\n{boxed}\n\\end{{table}}")
    } else {
        format!("\\begin{{figure}}[t]\n\\centering\n{boxed}\n{cap}\n\\end{{figure}}")
    }
}

/// A stand-in float in the paper (put there by 放進論文 for a planned figure or table).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StandIn {
    pub label: String,
    /// F1 / T2 (the plan's id).
    pub id: String,
    /// `figure` or `table`.
    pub kind: String,
    pub caption: String,
    /// The section file it is in.
    pub file: String,
}

fn stand_in_of(p: &str) -> Option<(String, String, String, String)> {
    let t = p.trim_start();
    let kind = if t.starts_with("\\begin{figure}") { "figure" } else if t.starts_with("\\begin{table}") { "table" } else { return None };
    let marker = format!("Planned {kind} ");
    let at = p.find(&marker)? + marker.len();
    let id: String = p[at..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    let arg = |cmd: &str| -> Option<String> {
        let i = p.find(cmd)? + cmd.len();
        let mut depth = 1usize;
        for (k, c) in p[i..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(p[i..i + k].to_string());
                    }
                }
                _ => {}
            }
        }
        None
    };
    Some((arg("\\label{")?, id, kind.to_string(), arg("\\caption{").unwrap_or_default()))
}

/// The stand-ins in the paper, in reading order.
pub fn stand_ins(paper: &std::path::Path) -> Vec<StandIn> {
    let mut out = vec![];
    for s in crate::open::sections(paper) {
        for p in &s.paragraphs {
            if let Some((label, id, kind, caption)) = stand_in_of(p) {
                out.push(StandIn { label, id, kind, caption, file: s.file.clone() });
            }
        }
    }
    out
}

/// Put `block` (the real figure or table, with the same `\label`) where the stand-in for
/// `label` is: the drafts' `\ref` stays valid. Error when there is no such stand-in.
pub fn replace_stand_in(paper: &std::path::Path, label: &str, block: &str) -> Result<String, String> {
    for mut s in crate::open::sections(paper) {
        let Some(k) = s.paragraphs.iter().position(|p| stand_in_of(p).is_some_and(|(l, ..)| l == label)) else { continue };
        s.paragraphs[k] = block.trim().to_string();
        crate::open::write_section(paper, &s)?;
        return Ok(s.file);
    }
    Err(format!("論文裡沒有 {label} 的預留框"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::session::{Claim, Item};

    fn known() -> BTreeSet<String> {
        ["E1", "E2", "R1", "P1"].iter().map(|s| s.to_string()).collect()
    }

    const REPLY: &str = r#"{"items":[
 {"kind":"figure","label":"fig:Study Workflow!","title":"研究流程","caption":"Study workflow.","supports":"方法","evidence":[],"tool":"flow","options":["由左到右","上下兩段"],"pending":[],"existing":""},
 {"kind":"figure","label":"fig:auc","title":"各發現的 AUROC","caption":"AUROC per finding.","supports":"主結果","evidence":["E2","R1","E9"],"tool":"plot","options":["長條加 CI","ROC 曲線","森林圖","第四種"],"pending":[],"existing":""},
 {"kind":"table","label":"tab:main","title":"主表","caption":"Main comparison.","supports":"主結果","evidence":["R1"],"tool":"table","options":["一張表"],"pending":["P1","P7"],"existing":"tab:main"},
 {"kind":"figure","label":"fig:auc","title":"外部驗證","caption":"External site.","supports":"推廣","evidence":["E7"],"tool":"chart","options":["a","b"],"pending":[],"existing":""},
 {"kind":"figure","label":"x","title":" ","caption":"","supports":"","evidence":[],"tool":"plot","options":[],"pending":[],"existing":""}
]}"#;

    #[test]
    fn the_list_is_checked_by_code() {
        let items = parse(REPLY, &known(), &["P1".into()], &["tab:main".into(), "fig:old".into()]).unwrap();
        assert_eq!(items.iter().map(|i| (i.id.as_str(), i.label.as_str())).collect::<Vec<_>>(), [("F1", "fig:study-workflow"), ("F2", "fig:auc"), ("T1", "tab:main"), ("F3", "fig:auc-2")]);
        assert_eq!(items[1].evidence, ["E2", "R1"], "an id the record does not have is dropped");
        assert_eq!(items[1].options.len(), 3);
        assert!(items[0].warnings.is_empty(), "a workflow diagram needs no record evidence");
        assert_eq!(items[2].pending, ["P1"]);
        assert_eq!(items[2].existing, "tab:main", "the paper's own table is that item");
        assert_eq!((items[3].tool.as_str(), items[3].warnings.len()), ("plot", 1), "unknown tool → plot; no evidence → warned");
        assert!(parse(r#"{"items":[]}"#, &known(), &[], &[]).is_err());
    }

    #[test]
    fn counts_claims_and_the_drafts() {
        let mut s = Session::default();
        let items = parse(REPLY, &known(), &["P1".into()], &["tab:main".into()]).unwrap();
        s.figures = Some(FigurePlan { items, figures: Some([4, 7]), tables: Some([2, 4]), ..Default::default() });
        let w = s.figures.as_ref().unwrap().count_warnings();
        assert_eq!(w, ["圖 3 個，期刊通常 4–7 個：少了", "表 1 個，期刊通常 2–4 個：少了"]);
        s.figures.as_mut().unwrap().items[3].status = Status::Removed;
        assert_eq!(s.figures.as_ref().unwrap().kept().count(), 3);
        // after 主張: a claim whose ledger item rests on E2 backs the AUROC figure; the main table's R1 is in a claim's evidence text
        s.items.push(Item { id: "i1".into(), group: "g1".into(), label: "x".into(), evidence: vec!["E2".into()], ..Default::default() });
        s.claims = vec![Claim { id: "c1".into(), zh: "z".into(), items: vec!["i1".into()], evidence: "AUROC 0.81".into(), ..Default::default() }];
        reconcile(&mut s);
        let p = s.figures.as_ref().unwrap();
        assert_eq!(p.items[1].claims, ["c1"]);
        assert!(p.items[2].warnings.iter().any(|w| w.starts_with("沒有主張")), "no claim uses the table");
        assert!(p.items[0].warnings.is_empty(), "a workflow diagram is not flagged");
        // the outline and the drafts reference the kept ones
        let o = outline_block(&s);
        assert!(o.contains("fig:auc [figure]") && o.contains("waits for P1") && !o.contains("外部驗證"));
        let b = paragraph_block(&s, &["fig:auc".into()]);
        assert!(b.contains("Fig.~\\ref{label}") && b.contains("- fig:auc: AUROC per finding."));
        assert!(paragraph_block(&s, &[]).is_empty());
        // a stand-in float carries the planned label (the \ref resolves until the real one replaces it)
        let ph = placeholder(&p.items[1]);
        assert!(ph.starts_with("\\begin{figure}[t]") && ph.contains("\\label{fig:auc}") && ph.contains("\\caption{AUROC per finding.}"), "{ph}");
        assert!(ph.is_ascii(), "never the Chinese options or title (pdflatex): {ph}");
        let added = PlannedFloat { id: "F9".into(), kind: "figure".into(), label: "fig:grad-cam".into(), title: "我自己的 Grad-CAM".into(), own: "四張並排".into(), ..Default::default() };
        let ph = placeholder(&added);
        assert!(ph.is_ascii() && ph.contains("\\caption{grad cam}") && ph.contains("Planned figure F9"), "{ph}");
        let mut it = p.items[1].clone();
        it.chosen = Some(1);
        assert_eq!(it.rendering(), "ROC 曲線");
        it.own = "我自己畫".into();
        assert_eq!(it.rendering(), "我自己畫");
    }

    #[test]
    fn a_stand_in_is_found_and_replaced_in_place_with_its_label() {
        let d = std::env::temp_dir().join(format!("pc-standin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sections")).unwrap();
        std::fs::write(d.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/results}\n\\end{document}\n").unwrap();
        let it = PlannedFloat { id: "F2".into(), kind: "figure".into(), label: "fig:auc".into(), title: "t".into(), caption: "AUROC per finding.".into(), ..Default::default() };
        let tab = PlannedFloat { id: "T1".into(), kind: "table".into(), label: "tab:main".into(), title: "t".into(), caption: "Main.".into(), ..Default::default() };
        std::fs::write(d.join("sections/results.tex"), format!("\\section{{Results}}\nFig.~\\ref{{fig:auc}} shows it.\n\n{}\n\n{}\n\nAfter.\n", placeholder(&it), placeholder(&tab))).unwrap();
        let si = stand_ins(&d);
        assert_eq!(si.iter().map(|x| (x.label.as_str(), x.id.as_str(), x.kind.as_str(), x.caption.as_str())).collect::<Vec<_>>(), [("fig:auc", "F2", "figure", "AUROC per finding."), ("tab:main", "T1", "table", "Main.")]);
        let block = "\\begin{figure}[t]\n\\centering\n\\includegraphics[width=\\linewidth]{figures/auc.pdf}\n\\caption{AUROC per finding.}\n\\label{fig:auc}\n\\end{figure}";
        assert_eq!(replace_stand_in(&d, "fig:auc", block).unwrap(), "results.tex");
        let r = std::fs::read_to_string(d.join("sections/results.tex")).unwrap();
        assert!(r.contains("figures/auc.pdf") && !r.contains("Planned figure F2") && r.matches("\\label{fig:auc}").count() == 1, "{r}");
        assert!(r.find("figures/auc.pdf").unwrap() < r.find("Planned table T1").unwrap() && r.contains("After."), "in place, the rest kept: {r}");
        assert_eq!(stand_ins(&d).len(), 1);
        assert!(replace_stand_in(&d, "fig:auc", block).is_err(), "no stand-in left for it");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_prompt_carries_the_journal_and_the_targets() {
        let p = prompt("CONTEXT", "Figure 1 is usually: study workflow", Some(([4, 7], [2, 4])));
        assert!(p.system.contains("Aim for 4–7 figures and 2–4 tables") && p.user.contains("Figure 1 is usually") && p.user.starts_with("CONTEXT"));
        assert_eq!(schema()["type"], "object");
    }
}
