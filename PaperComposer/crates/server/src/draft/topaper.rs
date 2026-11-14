//! From the discussion into the paper: the journal's habits (期刊寫法, docs/long-paper.md §D)
//! in each step's prompt, the abstract drafted like a section, and 放進論文 — the drafted
//! sections written into the paper's own section files (and the abstract into the title block),
//! so 審稿, 待補, 修訂 and the exports all read the same text.

use super::*;
use pc_core::venueprofile::{self as vp, Profile, Submission};

/// The paper's journal: its writing profile (when made for this journal) and its requirements.
#[derive(Default)]
pub(crate) struct Venue {
    pub profile: Option<Profile>,
    pub sub: Option<Submission>,
    /// `[word_limits] abstract`, `[limits] keywords_min / keywords` of the template.
    pub abstract_max: Option<usize>,
    pub keywords: (Option<usize>, Option<usize>),
}

pub(crate) fn venue_of(c: &Ctx) -> Venue {
    let Some(tpl) = meta::read_meta(&c.dir).journal else { return Venue::default() };
    let Some(j) = pc_core::journal::load(&c.ws.join("publishers"), &tpl) else { return Venue::default() };
    let sub = Submission::from_table(j.meta.submission.as_ref());
    let abstract_max = sub.as_ref().and_then(|s| s.abstract_words).map(|w| w as usize).or(j.meta.word_limits.abstract_);
    let kmin = sub.as_ref().and_then(|s| s.keywords_min).map(|w| w as usize).or(j.meta.limits.keywords_min.map(|x| x as usize));
    let kmax = sub.as_ref().and_then(|s| s.keywords_max).map(|w| w as usize).or(j.meta.limits.keywords.map(|x| x as usize));
    Venue { profile: vp::current(&c.ws, &tpl, j.meta.venue.as_ref()), sub, abstract_max, keywords: (kmin, kmax) }
}

impl Venue {
    /// For the outline prompt (`pc-draft-outline`).
    pub fn outline(&self) -> String {
        match (&self.profile, &self.sub) {
            (Some(p), sub) => format!("\n\n{}", vp::outline_block(p, sub.as_ref())),
            (None, Some(s)) => format!("\n\n{}", vp::submission_lines(s)),
            _ => String::new(),
        }
    }
    /// For one section's paragraphs (`pc-compose-paragraph` in step 7).
    pub fn section(&self, title: &str) -> String {
        self.profile.as_ref().and_then(|p| vp::prompt_block(p, title, self.sub.as_ref())).map(|b| format!("\n{b}\n")).unwrap_or_default()
    }
    /// For 預想選項 (`pc-draft-candidates`): a preference, never a change of the question.
    pub fn emphasis(&self) -> String {
        self.profile.as_ref().map(|p| format!("\n\n{}\n", vp::emphasis_block(p))).unwrap_or_default()
    }
    /// For 審稿 (`pc-draft-audit`): the journal's must-haves the model looks for.
    pub fn checklist(&self) -> String {
        let items = vp::audit_checklist(self.profile.as_ref(), self.sub.as_ref());
        let habits: Vec<String> = items.iter().filter(|i| i.source == "venue").map(|i| format!("- venue:{} — {} ({}/{} of the journal's articles; section: {})", i.id, i.label, i.count, i.of, i.section)).collect();
        if habits.is_empty() {
            return String::new();
        }
        format!(
            "\n\nThe journal's habits (most of its articles do these). Only when the draft does NOT do one, add a finding with kind \"venue\", \
the para where it belongs (or \"\"), and why in Traditional Chinese; never one for what the draft already does:\n{}\n",
            habits.join("\n")
        )
    }
}

// ── 摘要 ─────────────────────────────────────────────────

/// The abstract as one outline paragraph: the key value and the strongest claims, in the
/// order background → aim → methods → results → conclusion.
pub(crate) fn abstract_para(sess: &Session, key_value: &str) -> ds::OutlinePara {
    let mut en: Vec<String> = Vec::new();
    if !key_value.trim().is_empty() {
        en.push(format!("!Direction of the paper (do not quote): {key_value}"));
    }
    for c in &sess.claims {
        let line = if c.en.trim().is_empty() { c.zh.clone() } else { c.en.clone() };
        if !line.trim().is_empty() {
            en.push(format!("{} ({})", line.trim(), c.home));
        }
    }
    // what the outline's Methods and Results say, first point of each paragraph
    for o in sess.outline.iter().filter(|o| o.section == "Methods" || o.section == "Results") {
        for p in &o.paras {
            if let Some(first) = p.en.first() {
                en.push(format!("{} — {first}", o.section));
            }
        }
    }
    let pending: Vec<String> = sess.pending.iter().filter(|p| p.status == PendingStatus::Waiting).map(|p| p.id.clone()).collect();
    ds::OutlinePara {
        id: "abs".into(),
        kind: "Abstract".into(),
        plan: "background → gap and aim → materials and methods → main results with numbers → conclusion".into(),
        zh: "摘要：背景、目的、方法、主要結果（含數字）、結論，一段，不分小標。".into(),
        en,
        sources: sess.claims.iter().map(|c| c.id.clone()).collect(),
        cites: vec![],
        pending,
    }
}

/// The abstract's own rules, on top of the paragraph rules.
pub(crate) fn abstract_rules(v: &Venue) -> String {
    let (lo, hi) = abstract_words(v);
    format!(
        "\nThis is the ABSTRACT: one unstructured paragraph of {lo}–{hi} words, no headings, no citations, no equations, \
no figure or table references, every abbreviation spelled out at first use. Background in one or two sentences, then the aim, \
the materials and methods, the main results with their numbers, and the conclusion.\n"
    )
}

/// Words the abstract should have: the journal's limit (or 250), and 60 % of it below.
pub(crate) fn abstract_words(v: &Venue) -> (usize, usize) {
    let hi = v.abstract_max.unwrap_or(250).clamp(100, 400);
    (hi * 3 / 5, hi)
}

// ── 放進論文 ─────────────────────────────────────────────

/// The section file of the paper that holds the draft's `section` (by role: Methods ↔
/// "Materials and Methods", Conclusion ↔ "Conclusions").
pub(crate) fn paper_section(dir: &Path, section: &str) -> Option<paper::Section> {
    let want = vp::canonical_role(section);
    let all = pc_core::open::sections(dir);
    let role = |s: &paper::Section| vp::canonical_role(&pc_core::text::strip_latex(&s.title));
    all.iter().find(|s| role(s) == want).or_else(|| if want == "methods" { all.iter().find(|s| matches!(role(s), "data" | "experiments")) } else { None }).cloned()
}

/// A drafted section as the paragraphs of its paper section, the outline's subsections as
/// `\subsection{…}` before their first paragraph.
pub(crate) fn section_paragraphs(sess: &Session, d: &ds::DraftSection) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for p in &d.paras {
        let sub = sess.outline.iter().find(|o| o.section == d.section && o.paras.iter().any(|x| x.id == p.para)).and_then(|o| o.subsection.clone());
        if sub.is_some() && sub != current {
            out.push(format!("\\subsection{{{}}}", meta::tex_text(sub.as_deref().unwrap_or(""))));
        }
        current = sub;
        out.push(p.text.trim().to_string());
    }
    out
}

/// Where a new section goes: after the last paper section of an earlier draft section.
fn after_for(dir: &Path, section: &str) -> Option<String> {
    let at = ds::SECTIONS.iter().position(|s| *s == section)?;
    ds::SECTIONS[..at].iter().rev().find_map(|s| paper_section(dir, s)).map(|s| s.file)
}

/// The journal's title for a section the paper does not have yet ("Materials and Methods").
fn venue_title(v: &Venue, section: &str) -> String {
    let role = vp::canonical_role(section);
    v.profile
        .as_ref()
        .and_then(|p| p.sections.iter().find(|s| s.role == role))
        .and_then(|s| s.titles.first())
        .map(|t| vp::tex_safe_title(&t.text))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| section.to_string())
}

/// Write the abstract into the title block: `\abstract{…}` / `\begin{abstract}`, or the file a
/// `\abstract{\input{sections/01-abstract}}` reads.
pub(crate) fn write_abstract(dir: &Path, text: &str) -> Result<(), String> {
    let main_path = pc_core::open::main_file(dir);
    let main = std::fs::read_to_string(&main_path).map_err(|e| e.to_string())?;
    // an \abstract{…} the reader cannot parse (a bare % inside, a brace missing): never add a second one
    let has_cmd = pc_core::front::masked(&main).contains("\\abstract{") || main.contains("\\begin{abstract}");
    if has_cmd && !pc_core::front::fields(&main).iter().any(|f| f.field == "abstract") {
        return Err("main.tex 的摘要讀不出來（裡面可能有沒跳脫的 % 或少一個大括號）：先在編輯器修好摘要再放進論文".into());
    }
    let out = pc_core::front::set_field_in(dir, &main, "abstract", 0, text)?;
    if out != main {
        paper::write_atomic(&main_path, &out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A table, figure or algorithm block (`\begin{table}`, `\begin{figure*}`, …).
fn is_float(p: &str) -> bool {
    let t = p.trim_start();
    ["\\begin{table", "\\begin{figure", "\\begin{algorithm", "\\begin{sidewaystable", "\\begin{sidewaysfigure"].iter().any(|k| t.starts_with(k))
}

/// Does a section have text of its own (not only `\subsection{…}` headings)?
pub(crate) fn has_prose(s: &paper::Section) -> bool {
    s.paragraphs.iter().any(|p| {
        let t = p.trim();
        !t.is_empty() && !(t.starts_with("\\subsection") && t.lines().count() == 1) && !is_float(t)
    })
}

/// What 放進論文 did, for the window.
#[derive(serde::Serialize, Default)]
pub(crate) struct Placed {
    /// Section → the paper section's title it went into.
    pub sections: Vec<(String, String)>,
    /// Paper sections that had text before (kept in draft/archive/).
    pub replaced: Vec<String>,
}

/// Write the drafted sections (and the abstract) into the paper. Text already there is kept
/// in `draft/archive/<time>/` first; nothing else in the paper changes.
pub(crate) fn place(dir: &Path, ws_venue: &Venue, sess: &Session, only: Option<&str>) -> Result<Placed, String> {
    let mut done = Placed::default();
    // main.tex reads sections but none can be read: never add a second set after the back matter
    let main = std::fs::read_to_string(pc_core::open::main_file(dir)).unwrap_or_default();
    if pc_core::open::sections(dir).is_empty() && pc_core::front::masked(&main).contains("\\input{sections/") {
        return Err("讀不到論文的章節（main.tex 可能有沒成對的大括號或沒跳脫的 %）：先修好再放進論文".into());
    }
    let mut archive: Option<PathBuf> = None;
    let mut keep = |name: &str, text: &str| -> Result<(), String> {
        let a = match &archive {
            Some(a) => a.clone(),
            None => {
                let a = ds::new_archive(dir)?;
                archive = Some(a.clone());
                a
            }
        };
        std::fs::write(a.join(name), text).map_err(|e| e.to_string())
    };
    for d in sess.drafts.iter().filter(|d| only.is_none_or(|o| o == d.section)) {
        if d.section == "Abstract" {
            let text = d.paras.iter().map(|p| p.text.trim()).collect::<Vec<_>>().join("\n\n");
            let main = std::fs::read_to_string(pc_core::open::main_file(dir)).unwrap_or_default();
            let old = pc_core::front::fields(&main).into_iter().find(|f| f.field == "abstract").map(|f| f.value).unwrap_or_default();
            if !old.trim().is_empty() && !old.contains("\\input") {
                keep("abstract.tex", &old)?;
                done.replaced.push("Abstract".into());
            }
            write_abstract(dir, &text)?;
            done.sections.push((d.section.clone(), "Abstract".into()));
            continue;
        }
        let paras = section_paragraphs(sess, d);
        let mut target = match paper_section(dir, &d.section) {
            Some(s) => s,
            None => {
                let title = venue_title(ws_venue, &d.section);
                let id = paper::add_section(dir, &title, after_for(dir, &d.section).as_deref())?;
                pc_core::open::read_section(dir, &id).ok_or("新章節讀不到")?
            }
        };
        if has_prose(&target) {
            keep(&target.file.replace('/', "-"), &paper::render_section(&target))?;
            done.replaced.push(pc_core::text::strip_latex(&target.title));
        }
        // tables and figures already in the section (inserted from step 2, 流程圖, 繪圖) stay:
        // the drafted text refers to them with \ref
        let floats: Vec<String> = target.paragraphs.iter().filter(|p| is_float(p) && !paras.contains(p)).cloned().collect();
        target.paragraphs = paras;
        target.paragraphs.extend(floats);
        pc_core::open::write_section(dir, &target)?;
        done.sections.push((d.section.clone(), pc_core::text::strip_latex(&target.title)));
    }
    Ok(done)
}

/// The journal's naming for an empty paper: when every section file is empty, the sections are
/// laid out again from the 期刊寫法 skeleton (or, without one, left as they are).
pub(crate) fn relayout_if_empty(dir: &Path, v: &Venue) -> Result<bool, String> {
    let Some(p) = &v.profile else { return Ok(false) };
    let skeleton = vp::skeleton(p);
    if skeleton.is_empty() || pc_core::open::is_custom(dir) {
        return Ok(false);
    }
    let now = pc_core::open::sections(dir);
    if now.iter().any(has_prose) {
        return Ok(false);
    }
    let same = now.len() == skeleton.len() && now.iter().zip(&skeleton).all(|(a, b)| pc_core::text::strip_latex(&a.title) == pc_core::text::strip_latex(&b.title));
    if same {
        return Ok(false);
    }
    let mut after: Option<String> = None;
    let mut made = Vec::new();
    for s in &skeleton {
        let id = paper::add_section(dir, &s.title, after.as_deref().or(now.last().map(|x| x.file.as_str())))?;
        let mut sec = pc_core::open::read_section(dir, &id).ok_or("新章節讀不到")?;
        sec.paragraphs = s.subsections.iter().map(|x| format!("\\subsection{{{}}}", x.title)).collect();
        pc_core::open::write_section(dir, &sec)?;
        after = Some(id.clone());
        made.push(id);
    }
    for s in &now {
        paper::delete_section(dir, &s.file)?;
    }
    Ok(true)
}

// ── 審稿：期刊要求（程式檢查） ─────────────────────────────

fn words(t: &str) -> usize {
    pc_core::text::strip_latex(t).split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

/// The journal's requirements the code can check in the paper itself: the abstract's length,
/// the number of keywords, the required statements. Kind `venue`, in the author's words.
pub(crate) fn requirement_findings(dir: &Path, v: &Venue, sess: &Session) -> Vec<ds::Finding> {
    let mut out = Vec::new();
    let finding = |para: &str, why: String| ds::Finding { id: String::new(), kind: "venue".into(), para: para.into(), why, ..Default::default() };
    let main_path = pc_core::open::main_file(dir);
    let main = std::fs::read_to_string(&main_path).unwrap_or_default();
    let fields = pc_core::front::fields(&main);
    // the abstract: the paper's own (or its input file), else the drafted one
    let mut abs = fields.iter().find(|f| f.field == "abstract").map(|f| f.value.clone()).unwrap_or_default();
    if let Some(f) = abs.trim().strip_prefix("\\input{").and_then(|r| r.strip_suffix('}')) {
        let rel = if f.ends_with(".tex") { f.to_string() } else { format!("{f}.tex") };
        abs = std::fs::read_to_string(dir.join(rel)).unwrap_or_default();
    }
    if words(&abs) < 20 {
        if let Some(d) = sess.drafts.iter().find(|d| d.section == "Abstract") {
            abs = d.paras.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(" ");
        }
    }
    let (lo, hi) = abstract_words(v);
    let n = words(&abs);
    if n < 20 {
        out.push(finding("", "摘要還沒寫：到「草稿」生成 Abstract，再放進論文。".into()));
    } else if v.abstract_max.is_some() && (n > hi || n < lo) {
        out.push(finding("", format!("摘要 {n} 字；這本期刊要 {lo}–{hi} 字。")));
    }
    // a paper made by 新論文 starts with its folder name as the title
    let folder = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let title = fields.iter().find(|f| f.field == "title").map(|f| pc_core::text::strip_latex(&f.value)).unwrap_or_default();
    if title.trim().is_empty() || title.trim() == folder || title.trim() == "Article Title" {
        out.push(finding("", "還沒有標題（現在是資料夾名稱）：在論文上方的標題區改。".into()));
    }
    if fields.iter().any(|f| (f.field == "keywords" || f.field == "abstract") && pc_core::front::is_sample_text(&f.value)) {
        out.push(finding("", "摘要或關鍵字還是範本的示範文字。".into()));
    }
    if let (kmin, Some(kmax)) = v.keywords {
        let k = fields.iter().find(|f| f.field == "keywords").map(|f| f.value.split([',', ';']).filter(|x| !x.trim().is_empty()).count()).unwrap_or(0);
        let kmin = kmin.unwrap_or(1);
        if k < kmin || k > kmax {
            out.push(finding("", format!("關鍵字 {k} 個；這本期刊要 {kmin}–{kmax} 個。")));
        }
    }
    // placeholders left in the back matter: `[Funding.]`, `[Public datasets used …]`
    if let Some(at) = main.find("\\backmatter") {
        let back = &main[at..];
        let mut left: Vec<String> = Vec::new();
        let mut rest = back;
        while let Some(i) = rest.find('[') {
            let tail = &rest[i + 1..];
            let Some(j) = tail.find(']') else { break };
            let inner = &tail[..j];
            if inner.chars().count() >= 4 && inner.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && !inner.contains('\\') {
                left.push(inner.chars().take(40).collect());
            }
            rest = &tail[j + 1..];
        }
        if !left.is_empty() {
            out.push(finding("", format!("聲明還是預留文字（{} 處）：{}", left.len(), left.iter().take(4).cloned().collect::<Vec<_>>().join("、"))));
        }
    }
    if let Some(sub) = &v.sub {
        let all = pc_core::open::source_files(dir).iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect::<Vec<_>>().join("\n").to_lowercase();
        for st in &sub.required_statements {
            // the statement's heading words ("Ethics approval" → "ethic")
            let key: String = st.to_lowercase().split_whitespace().next().unwrap_or("").chars().take(5).collect();
            if !key.is_empty() && !all.contains(&key) {
                out.push(finding("", format!("投稿要求的聲明「{st}」還沒有。")));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pc_core::draft::session::{DraftPara, DraftSection, OutlinePara, OutlineSection};

    fn paper_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-topaper-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sections")).unwrap();
        std::fs::write(d.join("main.tex"), "\\documentclass{sn-jnl}\n\\begin{document}\n\\abstract{\\input{sections/01-abstract}}\n\\keywords{a, b}\n\\maketitle\n\\input{sections/02-introduction}\n\\input{sections/03-materials-and-methods}\n\\input{sections/04-results}\n\\backmatter\n\\bmhead{Funding}\n[Funding.]\n\\end{document}\n").unwrap();
        std::fs::write(d.join("sections/01-abstract.tex"), "Old abstract.\n").unwrap();
        std::fs::write(d.join("sections/02-introduction.tex"), "\\section{Introduction}\n").unwrap();
        std::fs::write(d.join("sections/03-materials-and-methods.tex"), "\\section{Materials and Methods}\n").unwrap();
        std::fs::write(d.join("sections/04-results.tex"), "\\section{Results}\nAn old sentence.\n\n\\begin{table}\n\\caption{Main.}\\label{tab:main}\n\\begin{tabular}{l}x\\end{tabular}\n\\end{table}\n").unwrap();
        d
    }

    fn draft(sec: &str, texts: &[&str]) -> DraftSection {
        DraftSection { section: sec.into(), paras: texts.iter().enumerate().map(|(i, t)| DraftPara { para: format!("{sec}{i}"), text: (*t).into(), ..Default::default() }).collect(), ..Default::default() }
    }

    #[test]
    fn drafts_go_into_the_paper_by_role_and_the_abstract_into_its_file() {
        let d = paper_dir("place");
        let mut s = Session::default();
        s.outline.push(OutlineSection { section: "Methods".into(), subsection: Some("Datasets".into()), paras: vec![OutlinePara { id: "Methods0".into(), ..Default::default() }], confirmed: false });
        s.drafts = vec![draft("Methods", &["We used two datasets."]), draft("Results", &["Table~\\ref{tab:main} shows it."]), draft("Conclusion", &["It works."]), draft("Abstract", &["We weight labels."])];
        let done = place(&d, &Venue::default(), &s, None).unwrap();
        assert_eq!(done.sections.iter().map(|(a, b)| format!("{a}>{b}")).collect::<Vec<_>>(), ["Methods>Materials and Methods", "Results>Results", "Conclusion>Conclusion", "Abstract>Abstract"]);
        let m = std::fs::read_to_string(d.join("sections/03-materials-and-methods.tex")).unwrap();
        assert!(m.contains("\\subsection{Datasets}") && m.contains("We used two datasets."), "{m}");
        let r = std::fs::read_to_string(d.join("sections/04-results.tex")).unwrap();
        assert!(r.contains("Table~\\ref{tab:main} shows it.") && r.contains("\\label{tab:main}") && !r.contains("An old sentence"), "the table stays: {r}");
        assert_eq!(done.replaced, ["Results"], "the old text was archived");
        // the abstract goes into the file \abstract reads; main.tex keeps its layout
        assert_eq!(std::fs::read_to_string(d.join("sections/01-abstract.tex")).unwrap().trim(), "We weight labels.");
        let main = std::fs::read_to_string(d.join("main.tex")).unwrap();
        assert!(main.contains("\\abstract{\\input{sections/01-abstract}}") && main.contains("\\input{sections/conclusion}"), "{main}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_journal_requirements_the_code_checks() {
        let d = paper_dir("req");
        let v = Venue { sub: Some(Submission { required_statements: vec!["Funding".into(), "Ethics approval".into()], ..Default::default() }), abstract_max: Some(250), keywords: (Some(4), Some(6)), ..Default::default() };
        let why: Vec<String> = requirement_findings(&d, &v, &Session::default()).into_iter().map(|f| f.why).collect();
        assert!(why.iter().any(|w| w.contains("摘要") && w.contains("還沒寫") || w.contains("摘要 2 字")), "{why:?}");
        assert!(why.iter().any(|w| w.contains("關鍵字 2 個")), "{why:?}");
        assert!(why.iter().any(|w| w.contains("Ethics approval")), "{why:?}");
        assert!(!why.iter().any(|w| w.contains("「Funding」")), "Funding is there: {why:?}");
        assert!(why.iter().any(|w| w.contains("預留文字")), "{why:?}");
        assert!(why.iter().any(|w| w.contains("還沒有標題")), "{why:?}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
