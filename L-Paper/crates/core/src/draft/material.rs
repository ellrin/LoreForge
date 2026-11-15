//! What the paper already holds, for the draft interview: its tables and
//! figures (from the section files) and its references (the paper's
//! citations with what each is used for, plus the library's notes and tags).
//! Nothing here is written; the interview only reads.

use crate::bib;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Float {
    /// "table" or "figure".
    pub kind: String,
    /// `\label{...}`, or "Table 2" / "Figure 1" by position when unlabelled.
    pub id: String,
    #[serde(default)]
    pub caption: String,
    /// Title of the section it sits in.
    #[serde(default)]
    pub section: String,
    /// Table cells as plain text (first row = header); empty for figures.
    #[serde(default)]
    pub grid: Vec<Vec<String>>,
    /// The image file of a figure.
    #[serde(default)]
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RefInfo {
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub year: String,
    #[serde(default)]
    pub doi: String,
    #[serde(default)]
    pub one_liner: String,
    /// Tag labels (not ids), as the author sees them.
    #[serde(default)]
    pub tags: Vec<String>,
    /// What the author said this reference is for in this paper.
    #[serde(default)]
    pub purpose: String,
    /// Already cited in the paper's LaTeX.
    #[serde(default)]
    pub cited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Material {
    pub floats: Vec<Float>,
    pub refs: Vec<RefInfo>,
}

/// LaTeX to plain text, without leftover grouping braces.
fn plain(s: &str) -> String {
    // Symbols that carry meaning in captions and cells, before math is dropped.
    let mut t = s.to_string();
    for (from, to) in [
        ("\\times", "×"), ("\\pm", "±"), ("\\leq", "≤"), ("\\le", "≤"), ("\\geq", "≥"), ("\\ge", "≥"),
        ("\\lambda", "λ"), ("\\sigma", "σ"), ("\\mu", "μ"), ("\\alpha", "α"), ("\\beta", "β"), ("\\%", "%"), ("--", "–"),
    ] {
        t = t.replace(from, to);
    }
    let t = t.replace('$', "");
    crate::text::strip_latex(&t).replace(['{', '}'], "").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn arg_after(s: &str, cmd: &str) -> Option<String> {
    let i = s.find(cmd)? + cmd.len();
    let rest = s[i..].trim_start();
    let rest = if rest.starts_with('[') { &rest[rest.find(']')? + 1..] } else { rest };
    let rest = rest.trim_start().strip_prefix('{')?;
    let mut depth = 1usize;
    for (k, c) in rest.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[..k].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Tables and figures in the paper's sections, in reading order.
pub fn floats(paper: &Path) -> Vec<Float> {
    let mut out = vec![];
    let (mut nt, mut nf) = (0, 0);
    for sec in crate::open::sections(paper) {
        let title = plain(&sec.title);
        let text = sec.paragraphs.join("\n\n");
        let mut rest = text.as_str();
        while let Some(start) = rest.find("\\begin{") {
            let after = &rest[start + 7..];
            let Some(close) = after.find('}') else { break };
            let env = &after[..close];
            let base = env.trim_end_matches('*');
            let end_tag = format!("\\end{{{env}}}");
            let Some(end) = rest[start..].find(&end_tag) else { break };
            let block = &rest[start..start + end + end_tag.len()];
            rest = &rest[start + end + end_tag.len()..];
            let kind = match base {
                "table" | "sidewaystable" => "table",
                "figure" | "sidewaysfigure" => "figure",
                _ => continue,
            };
            let caption = arg_after(block, "\\caption").map(|c| plain(&c)).unwrap_or_default();
            let label = arg_after(block, "\\label").unwrap_or_default();
            let (id, grid, file) = if kind == "table" {
                nt += 1;
                let grid = crate::textable::parse(block)
                    .map(|t| t.rows.iter().map(|r| r.iter().filter(|c| !c.covered).map(|c| plain(&c.text)).collect()).collect())
                    .unwrap_or_default();
                (if label.is_empty() { format!("Table {nt}") } else { label }, grid, String::new())
            } else {
                nf += 1;
                let file = arg_after(block, "\\includegraphics").unwrap_or_default();
                (if label.is_empty() { format!("Figure {nf}") } else { label }, vec![], file)
            };
            out.push(Float { kind: kind.into(), id, caption, section: title.clone(), grid, file });
        }
    }
    out
}

/// The paper's references: every key in `citations.csv` or cited in the
/// LaTeX, with the library's title, year, DOI, one-liner and tag labels.
pub fn refs(workspace: &Path, paper: &Path) -> Vec<RefInfo> {
    let purposes = bib::read_citations(&crate::paper::citations_file(paper));
    let tex: String = crate::open::source_files(paper)
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let cited = bib::cited_keys(&tex);
    let entries: BTreeMap<String, bib::Entry> = bib::parse(&std::fs::read_to_string(workspace.join("refs/library.bib")).unwrap_or_default())
        .0
        .into_iter()
        .map(|e| (e.key.clone(), e))
        .collect();
    let notes: BTreeMap<String, bib::Note> = bib::read_notes(&workspace.join("refs/notes.csv")).into_iter().map(|n| (n.key.clone(), n)).collect();
    let labels: BTreeMap<String, String> = bib::read_tags(&workspace.join("profile/tags.csv")).into_iter().map(|t| (t.id, t.label)).collect();
    let mut keys: Vec<String> = purposes.keys().cloned().collect();
    for k in &cited {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }
    keys.into_iter()
        .map(|key| {
            let e = entries.get(&key);
            let n = notes.get(&key);
            let doi = n.map(|n| n.doi.clone()).filter(|d| !d.is_empty()).or_else(|| e.and_then(|e| e.get("doi")).map(str::to_string)).unwrap_or_default();
            RefInfo {
                title: e.map(|e| e.title()).unwrap_or_default(),
                year: e.and_then(|e| e.get("year")).unwrap_or("").to_string(),
                doi,
                one_liner: n.map(|n| n.one_liner.clone()).unwrap_or_default(),
                tags: n.map(|n| n.tags.iter().map(|t| labels.get(t).cloned().unwrap_or_else(|| t.clone())).collect()).unwrap_or_default(),
                purpose: purposes.get(&key).cloned().unwrap_or_default(),
                cited: cited.contains(&key),
                key,
            }
        })
        .collect()
}

pub fn load(workspace: &Path, paper: &Path) -> Material {
    Material { floats: floats(paper), refs: refs(workspace, paper) }
}

/// Compact prompt text: tables with their cells (cut), figures by caption,
/// references with tags and purpose.
pub fn describe(m: &Material) -> String {
    let mut s = String::new();
    if !m.floats.is_empty() {
        s += "\n## Tables and figures already in the paper\n";
        for f in &m.floats {
            s += &format!("- [{}] {} ({}, in {}): {}\n", f.kind, f.id, if f.kind == "table" { "table" } else { f.file.as_str() }, f.section, f.caption);
            for row in f.grid.iter().take(14) {
                s += &format!("    | {} |\n", row.join(" | "));
            }
            if f.grid.len() > 14 {
                s += &format!("    … {} more rows\n", f.grid.len() - 14);
            }
        }
    }
    if !m.refs.is_empty() {
        s += "\n## The paper's references (key — title, year; tags; the author's note on what it is for)\n";
        for r in &m.refs {
            s += &format!(
                "- {} — {} ({}); tags: {}; {}{}\n",
                r.key,
                if r.title.is_empty() { "?" } else { r.title.as_str() },
                r.year,
                if r.tags.is_empty() { "-".into() } else { r.tags.join(", ") },
                if r.purpose.is_empty() { "no note" } else { r.purpose.as_str() },
                if r.one_liner.is_empty() { String::new() } else { format!("; gist: {}", r.one_liner) }
            );
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_floats_and_references_from_a_paper() {
        let ws = std::env::temp_dir().join(format!("pc-material-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let p = ws.join("papers/p");
        std::fs::create_dir_all(p.join("sections")).unwrap();
        std::fs::create_dir_all(ws.join("refs")).unwrap();
        std::fs::create_dir_all(ws.join("profile")).unwrap();
        std::fs::write(p.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/results}\n\\end{document}\n").unwrap();
        std::fs::write(
            p.join("sections/results.tex"),
            "\\section{Results}\nAs in prior work~\\cite{lee2020}, Table~\\ref{tab:main} compares.\n\n\\begin{table}[t]\n\\caption{Main {comparison} (5$\\times$5-fold, $\\lambda$--sweep)}\n\\label{tab:main}\n\\begin{tabular}{lc}\n\\toprule\nMethod & Dice \\\\\n\\midrule\nLinear & 0.704 \\\\\nFuzzy & 0.720 \\\\\n\\bottomrule\n\\end{tabular}\n\\end{table}\n\n\\begin{figure}\n\\includegraphics[width=\\linewidth]{figures/fig1-curve.png}\n\\caption{Training curves}\n\\end{figure}\n",
        )
        .unwrap();
        std::fs::write(p.join("citations.csv"), "key,purpose\nkim2019,Intro · 支撐「排除難影像」是常見做法\n").unwrap();
        std::fs::write(ws.join("refs/library.bib"), "@article{kim2019,\n  title = {Excluding {Hard} Frames},\n  year = {2019},\n  doi = {10.5555/kim}\n}\n").unwrap();
        std::fs::write(ws.join("refs/notes.csv"), "key,summary,one_liner,tags,doi,pdf\nkim2019,,排除低品質影格,medical;classical_algorithm,,\n").unwrap();
        let m = load(&ws, &p);
        assert_eq!(m.floats.len(), 2);
        assert_eq!((m.floats[0].id.as_str(), m.floats[0].caption.as_str()), ("tab:main", "Main comparison (5×5-fold, λ–sweep)"));
        assert_eq!(m.floats[0].grid[2], ["Fuzzy", "0.720"]);
        assert_eq!((m.floats[1].id.as_str(), m.floats[1].file.as_str()), ("Figure 1", "figures/fig1-curve.png"));
        assert_eq!(m.refs.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["kim2019", "lee2020"]);
        let k = &m.refs[0];
        assert_eq!((k.title.as_str(), k.year.as_str(), k.doi.as_str()), ("Excluding Hard Frames", "2019", "10.5555/kim"));
        assert_eq!(k.tags, ["醫學", "經典算法"], "tag ids become the author's labels");
        assert!(!k.cited && m.refs[1].cited);
        let d = describe(&m);
        assert!(d.contains("| Fuzzy | 0.720 |") && d.contains("支撐「排除難影像」"));
        let _ = std::fs::remove_dir_all(&ws);
    }
}
