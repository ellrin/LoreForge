//! 期刊寫法: how one journal writes, learned from a handful of its own open-access articles
//! (docs/long-paper.md §D). One profile per **journal template** (never per base format), kept in
//! the private workspace at `venues/<template id>/profile.toml` with a readable `profile.md`.
//!
//! Everything here is pure: the server downloads and extracts the articles, a model labels each
//! paragraph with a content category (one call per article, [`label_request`] / [`parse_labels`]),
//! and this module computes everything else in code — section titles and order, words, shares
//! per section and per content category (averaged over the articles, with the spread), paragraph
//! moves, structure habits, figure / table / equation counts, citation density, recurring short
//! phrases (n-grams found in several articles; never a sentence) — and turns a profile into
//! prompt blocks, an audit checklist and a paper skeleton for drafting.
//!
//! What a venue's articles show are the **venue's** patterns, never the author's voice.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const SCHEMA: u32 = 1;
/// How many articles a profile is made from.
pub const ARTICLES: usize = 5;
/// A profile older than this is shown with 更新.
pub const STALE_DAYS: i64 = 365;
/// Where profiles live in the workspace.
pub const DIR: &str = "venues";

// ── Identity and submission requirements (template.toml `[venue]`, `[submission]`) ──

/// Which journal (or conference) a template is: its OpenAlex source, confirmed once by the author.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Identity {
    /// OpenAlex source id (`S1234567`).
    pub openalex: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub issn_l: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub issn: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub publisher: String,
    /// OpenAlex's source type (`journal`, `conference`, …).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// `YYYY-MM-DD` the author confirmed it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub confirmed: String,
}

impl Identity {
    /// From the template's `[venue]` table; a table of the wrong shape is no identity
    /// (the template itself stays readable: `journal::Meta` keeps the table raw).
    pub fn from_table(t: Option<&toml::Table>) -> Option<Identity> {
        let id: Identity = toml::Value::Table(t?.clone()).try_into().ok()?;
        (!id.openalex.trim().is_empty()).then_some(id)
    }
    pub fn to_table(&self) -> toml::Table {
        match toml::Value::try_from(self) {
            Ok(toml::Value::Table(t)) => t,
            _ => toml::Table::new(),
        }
    }
}

/// What the journal requires at submission (`[submission]`, filled from the journal's own
/// guidelines by the author's research; refreshed with the profile about once a year).
/// Read leniently: a key of an unexpected type is left out, unknown keys are kept in `other`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Submission {
    pub class: String,
    pub class_options: String,
    pub pages: Option<i64>,
    pub words: Option<i64>,
    pub abstract_words: Option<i64>,
    /// `structured` (Background / Methods / Results / Conclusions …) or `unstructured`, or the headings.
    pub abstract_structure: Vec<String>,
    pub keywords_min: Option<i64>,
    pub keywords_max: Option<i64>,
    pub highlights: Option<bool>,
    /// Statements every submission must carry (`data availability`, `ethics`, `conflict of interest`, …).
    pub required_statements: Vec<String>,
    /// Sections the journal asks for by name.
    pub required_sections: Vec<String>,
    pub sources: Vec<String>,
    /// `YYYY-MM-DD` the requirements were last checked.
    pub checked: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub other: BTreeMap<String, Value>,
}

impl Submission {
    pub fn from_table(t: Option<&toml::Table>) -> Option<Submission> {
        let t = t?;
        let mut s = Submission::default();
        let str_of = |v: &toml::Value| v.as_str().map(|x| x.trim().to_string());
        let list = |v: &toml::Value| -> Vec<String> {
            match v {
                toml::Value::String(x) => vec![x.trim().to_string()],
                toml::Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(|y| y.trim().to_string())).filter(|y| !y.is_empty()).collect(),
                toml::Value::Table(m) => m.values().filter_map(|x| x.as_str().map(|y| y.trim().to_string())).collect(),
                _ => vec![],
            }
        };
        for (k, v) in t {
            match k.as_str() {
                "class" => s.class = str_of(v).unwrap_or_default(),
                "class_options" => s.class_options = str_of(v).unwrap_or_default(),
                "pages" => s.pages = v.as_integer(),
                "words" => s.words = v.as_integer(),
                "abstract_words" => s.abstract_words = v.as_integer(),
                "abstract_structure" => s.abstract_structure = list(v),
                "keywords_min" => s.keywords_min = v.as_integer(),
                "keywords_max" | "keywords" => s.keywords_max = v.as_integer(),
                "highlights" => s.highlights = v.as_bool(),
                "required_statements" => s.required_statements = list(v),
                "required_sections" => s.required_sections = list(v),
                "sources" => s.sources = list(v),
                "checked" => s.checked = str_of(v).unwrap_or_else(|| v.to_string()),
                _ => {
                    if let Ok(j) = serde_json::to_value(v) {
                        s.other.insert(k.clone(), j);
                    }
                }
            }
        }
        Some(s)
    }
}

/// A template that is a base format (hidden from the picker; journals are built on it): marked
/// `role = "base"`, or other templates are `based_on` it and it is not a journal itself (no
/// `[venue]`, no `role = "journal"`). `role = "umbrella"` (a series or a multi-conference
/// event) is hidden too: the author makes the specific journal from it.
pub fn hidden_role(role: &str, has_children: bool, has_identity: bool) -> bool {
    matches!(role.trim(), "base" | "umbrella") || (has_children && !has_identity && role.trim() != "journal")
}

/// A venue name for searching: no year, no bracketed notes (`ICLR 2027` → `ICLR`,
/// `Elsevier CAS（通用，雙欄）` → `Elsevier CAS`).
pub fn search_name(name: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in name.chars() {
        match c {
            '(' | '（' | '[' => depth += 1,
            ')' | '）' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace()
        .filter(|w| !(w.len() == 4 && w.chars().all(|c| c.is_ascii_digit()) && (w.starts_with("19") || w.starts_with("20"))))
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Dates ──

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYY-MM-DD` → days since 1970-01-01.
pub fn day_number(date: &str) -> Option<i64> {
    let mut it = date.trim().get(..10)?.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
}

pub fn date_of(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Today (UTC) as `YYYY-MM-DD`.
pub fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    date_of(secs.div_euclid(86400))
}

/// Older than [`STALE_DAYS`] on `today` (an unreadable date counts as old).
pub fn is_stale(made: &str, today: &str) -> bool {
    match (day_number(made), day_number(today)) {
        (Some(a), Some(b)) => b - a > STALE_DAYS,
        _ => true,
    }
}

/// Articles published from this date on (about two years back).
pub fn from_date(today: &str) -> String {
    date_of(day_number(today).unwrap_or(0) - 730)
}

// ── Content categories and section roles ──

/// One content category a paragraph is labelled with.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Category {
    pub id: String,
    pub label: String,
    /// What it covers (for the model).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub about: String,
    /// One of the base set; venue-specific ones are found by the model (`x-…`).
    pub base: bool,
}

/// The base set (id, label, what it covers).
pub const BASE_CATEGORIES: &[(&str, &str, &str)] = &[
    ("value", "臨床／應用價值", "why it matters in practice: clinical or application value, impact on patients or users"),
    ("motivation", "問題與動機", "the problem, the gap in existing methods, why it is hard"),
    ("prior", "前人研究", "describing or citing prior work"),
    ("contribution", "貢獻與目的", "what this paper does: aims, contributions, hypotheses, paper organisation"),
    ("method", "方法描述", "how the proposed method works, in words"),
    ("equations", "公式與推導", "equations, definitions, derivations, losses"),
    ("dataset", "資料集", "the data: sources, cohorts, inclusion, labelling, splits"),
    ("setup", "實驗設定", "implementation details, training, metrics, baselines chosen, statistics used"),
    ("results", "結果", "reporting the results"),
    ("ablation", "消融實驗", "ablation or sensitivity studies"),
    ("comparison", "與其他方法比較", "comparing with other methods or prior results"),
    ("discussion", "討論：優缺點", "interpreting results: strengths and weaknesses, why it works, pros and cons"),
    ("limitations", "限制", "limitations of the study"),
    ("future", "未來工作", "future work"),
    ("conclusion", "結論", "summing up the findings"),
    ("statements", "聲明", "ethics, consent, data / code availability, funding, conflicts, author contributions"),
];

pub fn base_categories() -> Vec<Category> {
    BASE_CATEGORIES.iter().map(|(id, label, about)| Category { id: id.to_string(), label: label.to_string(), about: about.to_string(), base: true }).collect()
}

/// Section roles (id, label), in the usual order.
pub const ROLES: &[(&str, &str)] = &[
    ("abstract", "摘要"),
    ("introduction", "引言"),
    ("related", "相關研究"),
    ("background", "背景"),
    ("methods", "方法"),
    ("data", "資料"),
    ("experiments", "實驗"),
    ("results", "結果"),
    ("discussion", "討論"),
    ("limitations", "限制"),
    ("conclusion", "結論"),
    ("statements", "聲明"),
    ("appendix", "附錄"),
    ("other", "其他"),
];

pub fn role_label(id: &str) -> &'static str {
    ROLES.iter().find(|(r, _)| *r == id).map(|(_, l)| *l).unwrap_or("其他")
}

/// The role a heading names (`Materials and Methods` → methods), from its words.
pub fn canonical_role(title: &str) -> &'static str {
    let t = clean_title(title).to_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| t.contains(w));
    if has(&["availability", "acknowledg", "funding", "declaration", "conflict of interest", "competing interest", "author contribution", "ethic", "consent", "credit author"]) {
        "statements"
    } else if has(&["abstract"]) {
        "abstract"
    } else if has(&["introduc"]) {
        "introduction"
    } else if has(&["related work", "literature", "prior work", "previous work", "state of the art", "related studies"]) {
        "related"
    } else if has(&["limitation"]) && !has(&["result", "discussion"]) {
        "limitations"
    } else if has(&["result"]) {
        "results"
    } else if has(&["discussion"]) {
        "discussion"
    } else if has(&["conclu", "summary", "future work", "outlook"]) {
        "conclusion"
    } else if has(&["appendix", "supplement"]) {
        "appendix"
    } else if has(&["background", "preliminar", "problem formulation", "problem statement"]) {
        "background"
    } else if has(&["dataset", "data set", "data collection", "cohort", "participants", "patients", "subjects", "data and", "data "]) || t.trim() == "data" {
        "data"
    } else if has(&["experiment", "evaluation", "implementation", "setup", "settings", "training details"]) {
        "experiments"
    } else if has(&["method", "approach", "proposed", "framework", "model", "algorithm", "architecture", "materials", "design"]) {
        "methods"
    } else {
        "other"
    }
}

/// Body roles: what the proportions are taken over.
fn is_body(role: &str) -> bool {
    !matches!(role, "abstract" | "statements" | "appendix")
}

// ── What the server hands in ──

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SecText {
    /// 1 = section, 2 = subsection, 3 = subsubsection.
    pub level: u8,
    pub title: String,
    pub paragraphs: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Caption {
    /// `figure` or `table`.
    pub kind: String,
    pub number: String,
    pub text: String,
}

/// One article as extracted (sections in reading order).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ArticleText {
    pub doi: String,
    pub title: String,
    pub year: Option<i32>,
    /// The licence of the copy read (`cc-by`, …).
    pub licence: String,
    /// The copy read (its URL) and where it was (`publisher`, `pmc`, `repository`, `arxiv`).
    pub url: String,
    pub copy: String,
    pub sections: Vec<SecText>,
    pub captions: Vec<Caption>,
    /// The section index (into `sections`) of each numbered equation; None = not placed.
    pub equations: Vec<Option<usize>>,
}

impl ArticleText {
    /// Paragraphs in order with their section index.
    pub fn paragraphs(&self) -> Vec<(usize, &str)> {
        self.sections.iter().enumerate().flat_map(|(i, s)| s.paragraphs.iter().map(move |p| (i, p.as_str()))).collect()
    }
    /// Enough to learn from: some sections with prose.
    pub fn usable(&self) -> bool {
        let words: usize = self.sections.iter().flat_map(|s| &s.paragraphs).map(|p| words(p)).sum();
        self.sections.iter().filter(|s| s.level <= 1).count() >= 3 && words >= 1500
    }
}

pub fn words(s: &str) -> usize {
    s.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

/// Citation marks in a paragraph: numbered (`[3]`, `[4, 7–9]`; a bracket with a 0 or a decimal is
/// an interval, not a citation) and author–year (`(Name et al., 2021; Other, 2019a)`,
/// `Name et al. (2020)`). Superscript numbers cannot be told from the text: 0.
pub fn citations(p: &str) -> usize {
    let mut n = 0;
    let b: Vec<char> = p.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '['
            && let Some(j) = b[i + 1..].iter().position(|c| *c == ']')
        {
            let inner: String = b[i + 1..i + 1 + j].iter().collect();
            let nums: Vec<i64> = inner.split([',', '-', '–', '—']).filter_map(|x| x.trim().parse().ok()).collect();
            if !inner.trim().is_empty() && inner.chars().all(|c| c.is_ascii_digit() || matches!(c, ',' | ' ' | '-' | '–' | '—')) && !nums.contains(&0) {
                n += inner
                    .split(',')
                    .map(|part| {
                        let r: Vec<i64> = part.split(['-', '–', '—']).filter_map(|x| x.trim().parse().ok()).collect();
                        if r.len() == 2 && r[1] > r[0] && r[1] - r[0] < 50 { (r[1] - r[0] + 1) as usize } else { 1 }
                    })
                    .sum::<usize>();
            }
            i += j + 1;
        }
        i += 1;
    }
    if n > 0 {
        return n;
    }
    // author–year: a year after ", " or "(" that closes a citation (`)`, `;`, a letter suffix)
    let mut k = 0;
    for (at, _) in p.match_indices(|c: char| c == ',' || c == '(') {
        let rest = &p[at + 1..];
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        let year: String = rest.chars().take(4).collect();
        let after = rest.chars().nth(4);
        if year.len() == 4 && (year.starts_with("19") || year.starts_with("20")) && year.chars().all(|c| c.is_ascii_digit()) && matches!(after, Some(')' | ';' | 'a' | 'b' | 'c')) {
            k += 1;
        }
    }
    k
}

// ── The model's labels ──

/// What the labelling call returns for one article (validated against the article).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Labels {
    /// Per section index: its role (top-level sections; subsections inherit).
    pub roles: Vec<String>,
    /// Per paragraph (in [`ArticleText::paragraphs`] order): a category id.
    pub cats: Vec<String>,
    /// Venue-specific categories this article needed.
    pub extra: Vec<Category>,
    /// Content points the article makes (ids of [`CONTENT_POINTS`]).
    pub points: Vec<String>,
}

/// Points only reading can tell (id, label, what it means).
pub const CONTENT_POINTS: &[(&str, &str, &str)] = &[
    ("pros_cons", "討論優缺點", "the discussion names both strengths and weaknesses of the method"),
    ("clinical_relevance", "臨床／應用意義", "an explicit statement of clinical or practical relevance"),
    ("contributions_list", "條列貢獻", "the introduction lists the contributions (bullets or numbered)"),
    ("statistical_tests", "統計檢定", "results are tested for significance (p-values, confidence intervals)"),
    ("comparison_sota", "和最新方法比較", "compared with recent state-of-the-art methods"),
    ("external_validation", "外部驗證", "evaluated on an external or independent dataset"),
    ("reproducibility", "可重現細節", "enough implementation detail to reproduce (hyper-parameters, code, hardware)"),
];

/// Max characters of one paragraph sent to the model (words are counted in code on the full text).
const PARA_CHARS: usize = 700;

/// The labelling request for one article: (system, user, schema). `known` = venue-specific
/// categories earlier articles already found (so five calls do not invent five names).
pub fn label_request(a: &ArticleText, known: &[Category]) -> (String, String, Value) {
    let mut cats = base_categories();
    cats.extend(known.iter().cloned());
    let cat_lines: Vec<String> = cats.iter().map(|c| format!("- {}: {}", c.id, if c.about.is_empty() { &c.label } else { &c.about })).collect();
    let roles: Vec<&str> = ROLES.iter().map(|(r, _)| *r).collect();
    let points: Vec<String> = CONTENT_POINTS.iter().map(|(id, _, about)| format!("- {id}: {about}")).collect();
    let system = format!(
        "You read one published research article and label its structure. You never rewrite or quote it.\n\
         For every paragraph give one content category id:\n{}\n\
         If a paragraph clearly needs a category this venue uses that is not listed, add it to `extra` with an id starting with `x-` and a short Traditional Chinese label (at most 2 new ones), and use that id.\n\
         For every section give its role, one of: {}.\n\
         List in `points` which of these the article does:\n{}\n\
         Reply with JSON only.",
        cat_lines.join("\n"),
        roles.join(", "),
        points.join("\n")
    );
    let mut user = String::new();
    let mut p = 0;
    for (i, s) in a.sections.iter().enumerate() {
        user.push_str(&format!("\n## [S{i}] {}{}\n", "#".repeat(s.level.saturating_sub(1) as usize), s.title));
        for para in &s.paragraphs {
            let cut: String = para.chars().take(PARA_CHARS).collect();
            user.push_str(&format!("[P{p}] {cut}{}\n", if cut.len() < para.len() { " …" } else { "" }));
            p += 1;
        }
    }
    let schema = json!({
        "type": "object",
        "required": ["sections", "paragraphs"],
        "properties": {
            "sections": { "type": "array", "items": { "type": "object", "required": ["s", "role"], "properties": { "s": { "type": "integer" }, "role": { "type": "string" } } } },
            "paragraphs": { "type": "array", "items": { "type": "object", "required": ["p", "c"], "properties": { "p": { "type": "integer" }, "c": { "type": "string" } } } },
            "extra": { "type": "array", "items": { "type": "object", "required": ["id", "label"], "properties": { "id": { "type": "string" }, "label": { "type": "string" } } } },
            "points": { "type": "array", "items": { "type": "string" } }
        }
    });
    (system, format!("Article sections and paragraphs:{user}"), schema)
}

/// The model's reply checked against the article: unknown ids become `other` / the heading's
/// own role, missing paragraphs are filled from their neighbours (or `method`), extra
/// categories must start with `x-`.
pub fn parse_labels(v: &Value, a: &ArticleText, known: &[Category]) -> Labels {
    let mut extra: Vec<Category> = Vec::new();
    for e in v["extra"].as_array().into_iter().flatten() {
        let id = e["id"].as_str().unwrap_or("").trim().to_lowercase();
        let label = e["label"].as_str().unwrap_or("").trim().to_string();
        if id.starts_with("x-") && id.len() > 2 && !label.is_empty() && extra.len() < 2 && !known.iter().any(|k| k.id == id) {
            extra.push(Category { id, label, about: String::new(), base: false });
        }
    }
    let valid: BTreeSet<String> = base_categories().into_iter().chain(known.iter().cloned()).chain(extra.iter().cloned()).map(|c| c.id).collect();
    let n_par = a.paragraphs().len();
    let mut cats: Vec<Option<String>> = vec![None; n_par];
    for x in v["paragraphs"].as_array().into_iter().flatten() {
        let (Some(p), Some(c)) = (x["p"].as_u64(), x["c"].as_str()) else { continue };
        let c = c.trim().to_lowercase();
        if (p as usize) < n_par && valid.contains(&c) {
            cats[p as usize] = Some(c);
        }
    }
    // gaps: the previous paragraph's category, else the next one's, else method
    let mut filled: Vec<String> = Vec::with_capacity(n_par);
    for i in 0..n_par {
        let c = cats[i].clone().or_else(|| filled.last().cloned()).or_else(|| cats[i..].iter().flatten().next().cloned()).unwrap_or_else(|| "method".into());
        filled.push(c);
    }
    let mut roles: Vec<String> = a.sections.iter().map(|s| canonical_role(&s.title).to_string()).collect();
    for x in v["sections"].as_array().into_iter().flatten() {
        let (Some(i), Some(r)) = (x["s"].as_u64(), x["role"].as_str()) else { continue };
        let r = r.trim().to_lowercase();
        if (i as usize) < roles.len() && ROLES.iter().any(|(id, _)| *id == r) && r != "other" {
            roles[i as usize] = r;
        }
    }
    let points = v["points"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|p| p.trim().to_string()).filter(|p| CONTENT_POINTS.iter().any(|(id, _, _)| id == p)).collect::<BTreeSet<_>>().into_iter().collect();
    Labels { roles, cats: filled, extra, points }
}

/// Labels from the headings alone (no model): roles from headings, every paragraph of a role
/// its usual category. Used when no engine answers, so a profile is still made (marked).
pub fn heading_labels(a: &ArticleText) -> Labels {
    let roles: Vec<String> = top_roles(a, &a.sections.iter().map(|s| canonical_role(&s.title).to_string()).collect::<Vec<_>>());
    let cats = a
        .paragraphs()
        .iter()
        .map(|(si, _)| {
            match roles[*si].as_str() {
                "introduction" => "motivation",
                "related" | "background" => "prior",
                "data" => "dataset",
                "experiments" => "setup",
                "results" => "results",
                "discussion" => "discussion",
                "limitations" => "limitations",
                "conclusion" => "conclusion",
                "statements" => "statements",
                _ => "method",
            }
            .to_string()
        })
        .collect();
    Labels { roles, cats, ..Default::default() }
}

/// Subsections take their section's role (a top-level heading's own role wins).
fn top_roles(a: &ArticleText, roles: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(roles.len());
    let mut current = "other".to_string();
    for (i, s) in a.sections.iter().enumerate() {
        if s.level <= 1 {
            current = roles.get(i).cloned().unwrap_or_else(|| "other".into());
        }
        out.push(current.clone());
    }
    out
}

// ── One article, measured ──

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RolePart {
    pub role: String,
    pub title: String,
    /// 0..1 among the article's body sections.
    pub position: f64,
    pub words: usize,
    pub citations: usize,
    pub equations: usize,
    pub subsections: Vec<String>,
    /// Words per category.
    pub cats: BTreeMap<String, usize>,
    /// Paragraph categories in order, repeats merged.
    pub moves: Vec<String>,
}

/// What one article shows, all in code.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Measured {
    pub body_words: usize,
    pub parts: Vec<RolePart>,
    pub figures: usize,
    pub tables: usize,
    pub equations: usize,
    /// Habit ids this article shows.
    pub habits: Vec<String>,
    pub cat_words: BTreeMap<String, usize>,
}

fn overview_caption(t: &str) -> bool {
    let t = t.to_lowercase();
    ["overview", "pipeline", "framework", "architecture", "workflow", "flowchart", "flow chart", "schematic", "illustration of the proposed", "proposed method", "proposed approach"].iter().any(|w| t.contains(w))
}

/// A contribution list: bullets, or numbered items `1)`, `2)` / `(i)`, `(ii)` in order and
/// not part of a reference ("Fig. 1)", "(Smith, 2021)", "Eq. (1)").
fn bulleted(paras: &[&str]) -> bool {
    let refs = ["fig", "fig.", "figure", "table", "tab.", "eq", "eq.", "equation", "section", "sec.", "step", "stage", "et", "al.", "al.,"];
    let items = |p: &str| -> Vec<String> {
        let w: Vec<&str> = p.split_whitespace().collect();
        (0..w.len())
            .filter(|&i| i == 0 || !refs.contains(&w[i - 1].trim_start_matches('(').to_lowercase().as_str()))
            .filter_map(|i| {
                let t = w[i];
                let core = t.strip_prefix('(').unwrap_or(t).strip_suffix(')')?;
                let ok = (core.len() == 1 && core.chars().all(|c| c.is_ascii_digit())) || matches!(core, "i" | "ii" | "iii" | "iv" | "a" | "b" | "c");
                ok.then(|| core.to_string())
            })
            .collect()
    };
    let in_order = |v: &[String]| v.windows(2).any(|x| (x[0] == "1" && x[1] == "2") || (x[0] == "i" && x[1] == "ii") || (x[0] == "a" && x[1] == "b"));
    paras.iter().any(|p| p.matches('•').count() >= 2 || in_order(&items(p))) || paras.windows(3).any(|w| w.iter().all(|p| p.trim_start().starts_with(['•', '▪', '◦', '–'])))
}

pub fn measure(a: &ArticleText, l: &Labels) -> Measured {
    let roles = top_roles(a, &l.roles);
    let pars = a.paragraphs();
    let mut m = Measured::default();
    // body sections in order, one part per top-level section
    let mut part_of: Vec<Option<usize>> = vec![None; a.sections.len()];
    for (i, s) in a.sections.iter().enumerate() {
        if s.level <= 1 || m.parts.is_empty() {
            m.parts.push(RolePart { role: roles[i].clone(), title: s.title.trim().to_string(), ..Default::default() });
        } else if !s.title.trim().is_empty() {
            m.parts.last_mut().unwrap().subsections.push(s.title.trim().to_string());
        }
        part_of[i] = Some(m.parts.len() - 1);
    }
    for (pi, (si, text)) in pars.iter().enumerate() {
        let Some(k) = part_of[*si] else { continue };
        let w = words(text);
        let cat = l.cats.get(pi).cloned().unwrap_or_else(|| "method".into());
        let part = &mut m.parts[k];
        part.words += w;
        part.citations += citations(text);
        *part.cats.entry(cat.clone()).or_default() += w;
        if part.moves.last() != Some(&cat) {
            part.moves.push(cat.clone());
        }
        if is_body(&part.role) {
            *m.cat_words.entry(cat).or_default() += w;
        }
    }
    for e in a.equations.iter().flatten() {
        if let Some(Some(k)) = part_of.get(*e) {
            m.parts[*k].equations += 1;
        }
    }
    m.equations = a.equations.len();
    m.figures = a.captions.iter().filter(|c| c.kind == "figure").map(|c| &c.number).collect::<BTreeSet<_>>().len();
    m.tables = a.captions.iter().filter(|c| c.kind == "table").map(|c| &c.number).collect::<BTreeSet<_>>().len();
    // merge parts of the same role that follow each other (a heading split by the extractor)
    let body: Vec<usize> = (0..m.parts.len()).filter(|&i| is_body(&m.parts[i].role)).collect();
    let nb = body.len().max(2) - 1;
    for (j, &i) in body.iter().enumerate() {
        m.parts[i].position = j as f64 / nb as f64;
    }
    m.body_words = body.iter().map(|&i| m.parts[i].words).sum();

    // habits
    let top: Vec<&str> = m.parts.iter().map(|p| p.role.as_str()).collect();
    let mut h: BTreeSet<&str> = BTreeSet::new();
    if top.contains(&"related") {
        h.insert("related_separate");
    } else if let Some(intro) = m.parts.iter().find(|p| p.role == "introduction")
        && (intro.cats.get("prior").copied().unwrap_or(0) * 4 >= intro.words.max(1) || intro.subsections.iter().any(|s| canonical_role(s) == "related"))
    {
        h.insert("related_in_intro");
    }
    if a.sections.iter().any(|s| canonical_role(&s.title) == "limitations") {
        h.insert("limitations_heading");
    }
    if top.contains(&"discussion") && top.contains(&"conclusion") {
        h.insert("discussion_and_conclusion");
    }
    if a.sections.iter().any(|s| s.level <= 1 && { let t = s.title.to_lowercase(); t.contains("result") && t.contains("discussion") }) {
        h.insert("results_discussion_merged");
    }
    if top.contains(&"data") {
        h.insert("data_section");
    }
    let intro_paras: Vec<&str> = pars.iter().filter(|(si, _)| roles[*si] == "introduction").map(|(_, t)| *t).collect();
    if bulleted(&intro_paras) {
        h.insert("contributions_bulleted");
    }
    if a.captions.iter().find(|c| c.kind == "figure" && c.number.trim_matches(|c: char| !c.is_alphanumeric()) == "1").is_some_and(|c| overview_caption(&c.text)) {
        h.insert("fig1_overview");
    }
    let low_titles: Vec<String> = a.sections.iter().map(|s| s.title.to_lowercase()).collect();
    let all_text = || pars.iter().map(|(_, t)| t.to_lowercase()).collect::<Vec<_>>().join(" ");
    if low_titles.iter().any(|t| t.contains("data availability") || t.contains("availability of data")) || all_text().contains("data availability") {
        h.insert("data_availability");
    }
    if low_titles.iter().any(|t| t.contains("code availability")) || all_text().contains("github.com") || all_text().contains("code is available") || all_text().contains("code are available") {
        h.insert("code_availability");
    }
    if low_titles.iter().any(|t| t.contains("ethic") || t.contains("consent")) || all_text().contains("institutional review board") || all_text().contains("ethics committee") {
        h.insert("ethics_statement");
    }
    if m.cat_words.contains_key("limitations") {
        h.insert("limitations");
    }
    if m.cat_words.contains_key("future") {
        h.insert("future_work");
    }
    if m.cat_words.contains_key("ablation") {
        h.insert("ablation");
    }
    if m.cat_words.contains_key("value") {
        h.insert("clinical_relevance");
    }
    for p in &l.points {
        h.insert(CONTENT_POINTS.iter().find(|(id, _, _)| id == p).map(|(id, _, _)| *id).unwrap_or("other"));
    }
    if h.contains("contributions_bulleted") {
        h.insert("contributions_list");
    }
    h.remove("other");
    m.habits = h.into_iter().map(String::from).collect();
    m
}

/// Habits (id, label, kind): `structure` (how the paper is arranged) or `content` /
/// `statement` (what it must say; these make the audit checklist).
pub const HABITS: &[(&str, &str, &str)] = &[
    ("related_separate", "相關研究獨立成節", "structure"),
    ("related_in_intro", "前人研究寫在引言裡", "structure"),
    ("data_section", "資料獨立成節", "structure"),
    ("results_discussion_merged", "結果與討論合併", "structure"),
    ("discussion_and_conclusion", "討論與結論分開", "structure"),
    ("limitations_heading", "限制有自己的標題", "structure"),
    ("contributions_bulleted", "貢獻用條列", "structure"),
    ("fig1_overview", "圖 1 是整體流程", "structure"),
    ("contributions_list", "列出貢獻", "content"),
    ("clinical_relevance", "寫出臨床／應用意義", "content"),
    ("pros_cons", "討論優缺點", "content"),
    ("limitations", "寫出限制", "content"),
    ("future_work", "寫出未來工作", "content"),
    ("ablation", "消融實驗", "content"),
    ("comparison_sota", "和最新方法比較", "content"),
    ("statistical_tests", "統計檢定", "content"),
    ("external_validation", "外部驗證", "content"),
    ("reproducibility", "可重現細節", "content"),
    ("data_availability", "資料取得聲明", "statement"),
    ("code_availability", "程式碼取得", "statement"),
    ("ethics_statement", "倫理審查聲明", "statement"),
];

// ── Recurring phrases (code only) ──

const STOP: &[&str] = &[
    "the", "a", "an", "of", "to", "in", "on", "for", "and", "or", "with", "by", "as", "at", "from", "is", "are", "was", "were", "be", "been", "that", "which", "this", "these", "those", "it", "its", "we", "our", "their", "they", "than", "can", "has", "have", "had", "not", "also", "such", "between", "into", "using", "used", "each", "both", "all", "more", "most", "other", "may", "will", "while", "where", "when", "there", "here", "then", "however", "thus",
];
const BAD_EDGE_START: &[&str] = &["and", "or", "of", "which", "that", "than", "is", "are", "was", "were", "be", "been", "by", "with", "from"];
const BAD_EDGE_END: &[&str] = &["the", "a", "an", "of", "to", "and", "or", "in", "on", "for", "with", "by", "at", "from", "as", "is", "are", "was", "were", "be", "that", "which", "this", "its", "our", "their"];

fn tokens(sentence: &str) -> Vec<String> {
    sentence
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\''))
        .map(|w| w.trim_matches(|c: char| c == '-' || c == '\'').to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// A short phrase used in several articles of the venue, and the situation it is used in.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Phrase {
    pub text: String,
    /// The category of the paragraphs it appears in most.
    pub situation: String,
    /// In how many of the articles.
    pub articles: usize,
}

/// 2–5-word n-grams found in at least `min_articles` articles; maximal ones only (a shorter
/// n-gram inside a kept longer one with the same count is dropped), at most `limit`.
pub fn recurring_phrases(arts: &[(&ArticleText, &Labels)], min_articles: usize, limit: usize) -> Vec<Phrase> {
    let mut df: HashMap<Vec<String>, BTreeSet<usize>> = HashMap::new();
    let mut sit: HashMap<Vec<String>, HashMap<String, usize>> = HashMap::new();
    for (ai, (a, l)) in arts.iter().enumerate() {
        for (pi, (_, p)) in a.paragraphs().iter().enumerate() {
            let cat = l.cats.get(pi).cloned().unwrap_or_default();
            if cat == "statements" {
                continue;
            }
            for sentence in p.split(['.', ';', ':', '?', '!', '(', ')', '[', ']']) {
                let t = tokens(sentence);
                for n in 2..=5 {
                    for w in t.windows(n) {
                        if w.iter().any(|x| x.chars().any(|c| c.is_ascii_digit()) || x.len() > 24) {
                            continue;
                        }
                        if BAD_EDGE_START.contains(&w[0].as_str()) || BAD_EDGE_END.contains(&w[n - 1].as_str()) {
                            continue;
                        }
                        if !w.iter().any(|x| x.len() >= 4 && !STOP.contains(&x.as_str())) {
                            continue;
                        }
                        df.entry(w.to_vec()).or_default().insert(ai);
                        *sit.entry(w.to_vec()).or_default().entry(cat.clone()).or_default() += 1;
                    }
                }
            }
        }
    }
    let mut cands: Vec<(Vec<String>, usize)> = df.into_iter().map(|(k, v)| (k, v.len())).filter(|(_, n)| *n >= min_articles.max(2)).collect();
    cands.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(b.1.cmp(&a.1)).then(a.0.cmp(&b.0)));
    let mut kept: Vec<(Vec<String>, usize)> = Vec::new();
    for (k, n) in cands {
        let inside = kept.iter().any(|(big, bn)| *bn >= n && big.windows(k.len()).any(|w| w == k.as_slice()));
        if !inside {
            kept.push((k, n));
        }
    }
    kept.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.len().cmp(&a.0.len())).then(a.0.cmp(&b.0)));
    kept.into_iter()
        .take(limit)
        .map(|(k, n)| {
            let situation = sit.get(&k).and_then(|m| m.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(c, _)| c.clone())).unwrap_or_default();
            Phrase { text: k.join(" "), situation, articles: n }
        })
        .collect()
}

// ── The profile ──

/// Mean and spread over the articles that have the thing.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Stat {
    pub mean: f64,
    pub min: f64,
    pub max: f64,
}

impl Stat {
    pub fn of(v: &[f64]) -> Stat {
        if v.is_empty() {
            return Stat::default();
        }
        let r = |x: f64| (x * 1000.0).round() / 1000.0;
        Stat { mean: r(v.iter().sum::<f64>() / v.len() as f64), min: r(v.iter().cloned().fold(f64::INFINITY, f64::min)), max: r(v.iter().cloned().fold(f64::NEG_INFINITY, f64::max)) }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Count {
    pub text: String,
    pub n: usize,
}

/// A content category's share inside a section (or the whole body).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Point {
    pub category: String,
    pub label: String,
    /// Share of the section's words (0..1), over the articles whose section has it.
    pub share: Stat,
    /// In how many of the articles' sections.
    pub present: usize,
}

/// A paragraph move: a category and where in the section it usually comes (0 = first).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Move {
    pub category: String,
    pub label: String,
    pub position: f64,
    pub present: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SectionProfile {
    pub role: String,
    pub label: String,
    /// The actual titles, most used first.
    pub titles: Vec<Count>,
    /// In how many articles it is a section of its own.
    pub present: usize,
    /// Mean place among the body sections (0 = first).
    pub position: f64,
    /// Share of the body's words.
    pub share: Stat,
    pub words: Stat,
    /// Citations per 100 words.
    pub citation_density: Stat,
    /// Share of the article's equations.
    pub equation_share: Stat,
    /// Subsection titles used in at least two articles (or all of one, when only one has it).
    pub subsections: Vec<Count>,
    pub points: Vec<Point>,
    pub moves: Vec<Move>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Habit {
    pub id: String,
    pub label: String,
    /// `structure`, `content` or `statement`.
    pub kind: String,
    pub count: usize,
    pub of: usize,
}

impl Habit {
    /// Most of the articles show it (≥ 60 %).
    pub fn usual(&self) -> bool {
        self.of > 0 && self.count * 5 >= self.of * 3
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ArticleUsed {
    pub doi: String,
    pub title: String,
    pub year: Option<i32>,
    pub licence: String,
    /// `publisher`, `pmc`, `repository`, `arxiv`.
    pub copy: String,
    pub words: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub schema: u32,
    pub template: String,
    pub venue: String,
    pub openalex: String,
    pub issn_l: String,
    /// `YYYY-MM-DD`.
    pub made: String,
    /// The engine that labelled the paragraphs (`headings` = no model, from headings only).
    pub engine: String,
    /// `article` or `review`.
    pub kind: String,
    pub articles: Vec<ArticleUsed>,
    /// Why other candidates were not used (counts by reason).
    pub skipped: BTreeMap<String, usize>,
    pub body_words: Stat,
    pub figures: Stat,
    pub tables: Stat,
    pub equations: Stat,
    pub sections: Vec<SectionProfile>,
    /// Content categories over the whole body.
    pub overall: Vec<Point>,
    pub categories: Vec<Category>,
    pub habits: Vec<Habit>,
    pub phrases: Vec<Phrase>,
}

fn cat_label(cats: &[Category], id: &str) -> String {
    cats.iter().find(|c| c.id == id).map(|c| c.label.clone()).unwrap_or_else(|| id.to_string())
}

/// The profile from the articles and their labels (all in code).
pub fn build(template: &str, id: &Identity, kind: &str, made: &str, engine: &str, arts: &[(ArticleText, Labels)]) -> Profile {
    let mut categories = base_categories();
    for (_, l) in arts {
        for e in &l.extra {
            if !categories.iter().any(|c| c.id == e.id) {
                categories.push(e.clone());
            }
        }
    }
    let ms: Vec<Measured> = arts.iter().map(|(a, l)| measure(a, l)).collect();
    let n = ms.len();
    let f = |v: Vec<f64>| Stat::of(&v);
    let mut p = Profile {
        schema: SCHEMA,
        template: template.into(),
        venue: id.name.clone(),
        openalex: id.openalex.clone(),
        issn_l: id.issn_l.clone(),
        made: made.into(),
        engine: engine.into(),
        kind: kind.into(),
        articles: arts.iter().zip(&ms).map(|((a, _), m)| ArticleUsed { doi: a.doi.clone(), title: a.title.clone(), year: a.year, licence: a.licence.clone(), copy: a.copy.clone(), words: m.body_words }).collect(),
        body_words: f(ms.iter().map(|m| m.body_words as f64).collect()),
        figures: f(ms.iter().map(|m| m.figures as f64).collect()),
        tables: f(ms.iter().map(|m| m.tables as f64).collect()),
        equations: f(ms.iter().map(|m| m.equations as f64).collect()),
        ..Default::default()
    };
    // sections by role
    for (role, label) in ROLES {
        if *role == "other" || *role == "abstract" {
            continue;
        }
        let parts: Vec<(&Measured, Vec<&RolePart>)> = ms.iter().map(|m| (m, m.parts.iter().filter(|x| x.role == *role).collect::<Vec<_>>())).filter(|(_, v)| !v.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }
        let mut titles: BTreeMap<String, usize> = BTreeMap::new();
        let mut subs: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let (mut share, mut wds, mut dens, mut eqs, mut pos) = (vec![], vec![], vec![], vec![], vec![]);
        let mut cat_share: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        let mut moves: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        for (ai, (m, ps)) in parts.iter().enumerate() {
            let w: usize = ps.iter().map(|x| x.words).sum();
            let cites: usize = ps.iter().map(|x| x.citations).sum();
            let eq: usize = ps.iter().map(|x| x.equations).sum();
            for x in ps {
                *titles.entry(clean_title(&x.title)).or_default() += 1;
                for s in &x.subsections {
                    subs.entry(clean_title(s)).or_default().insert(ai);
                }
            }
            share.push(if m.body_words > 0 { w as f64 / m.body_words as f64 } else { 0.0 });
            wds.push(w as f64);
            dens.push(if w > 0 { cites as f64 * 100.0 / w as f64 } else { 0.0 });
            if m.equations > 0 {
                eqs.push(eq as f64 / m.equations as f64);
            }
            pos.push(ps[0].position);
            let mut cw: BTreeMap<&str, usize> = BTreeMap::new();
            for x in ps {
                for (c, k) in &x.cats {
                    *cw.entry(c).or_default() += k;
                }
            }
            for (c, k) in cw {
                cat_share.entry(c.to_string()).or_default().push(if w > 0 { k as f64 / w as f64 } else { 0.0 });
            }
            let seq: Vec<&String> = ps.iter().flat_map(|x| &x.moves).collect();
            let mut seen: BTreeSet<&String> = BTreeSet::new();
            for (k, c) in seq.iter().enumerate() {
                if seen.insert(c) {
                    moves.entry(c.to_string()).or_default().push(if seq.len() > 1 { k as f64 / (seq.len() - 1) as f64 } else { 0.0 });
                }
            }
        }
        let np = parts.len();
        let mut titles: Vec<Count> = titles.into_iter().map(|(text, n)| Count { text, n }).collect();
        titles.sort_by(|a, b| b.n.cmp(&a.n).then(a.text.cmp(&b.text)));
        let mut subsections: Vec<Count> = subs.into_iter().map(|(text, s)| Count { text, n: s.len() }).filter(|c| c.n >= 2 || np == 1).collect();
        subsections.sort_by(|a, b| b.n.cmp(&a.n).then(a.text.cmp(&b.text)));
        subsections.truncate(8);
        let mut points: Vec<Point> = cat_share
            .into_iter()
            .map(|(c, v)| {
                // a section without the category counts as 0 in the mean
                let mut all = v.clone();
                all.resize(np, 0.0);
                let st = Stat::of(&all);
                Point { label: cat_label(&categories, &c), category: c, share: Stat { mean: st.mean, min: st.min, max: st.max }, present: v.len() }
            })
            .collect();
        points.sort_by(|a, b| b.share.mean.partial_cmp(&a.share.mean).unwrap_or(std::cmp::Ordering::Equal).then(a.category.cmp(&b.category)));
        let mut mv: Vec<Move> = moves
            .into_iter()
            .filter(|(_, v)| v.len() * 2 >= np)
            .map(|(c, v)| Move { label: cat_label(&categories, &c), position: Stat::of(&v).mean, present: v.len(), category: c })
            .collect();
        mv.sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap_or(std::cmp::Ordering::Equal).then(a.category.cmp(&b.category)));
        p.sections.push(SectionProfile {
            role: role.to_string(),
            label: label.to_string(),
            titles,
            present: np,
            position: Stat::of(&pos).mean,
            share: Stat::of(&share),
            words: Stat::of(&wds),
            citation_density: Stat::of(&dens),
            equation_share: Stat::of(&eqs),
            subsections,
            points,
            moves: mv,
        });
    }
    p.sections.sort_by(|a, b| {
        let body = |s: &SectionProfile| !is_body(&s.role);
        body(a).cmp(&body(b)).then(a.position.partial_cmp(&b.position).unwrap_or(std::cmp::Ordering::Equal))
    });
    // the body overall
    let mut over: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for m in &ms {
        for (c, w) in &m.cat_words {
            over.entry(c.clone()).or_default().push(if m.body_words > 0 { *w as f64 / m.body_words as f64 } else { 0.0 });
        }
    }
    p.overall = over
        .into_iter()
        .map(|(c, v)| {
            let mut all = v.clone();
            all.resize(n, 0.0);
            Point { label: cat_label(&categories, &c), share: Stat::of(&all), present: v.len(), category: c }
        })
        .collect();
    p.overall.sort_by(|a, b| b.share.mean.partial_cmp(&a.share.mean).unwrap_or(std::cmp::Ordering::Equal));
    p.habits = HABITS
        .iter()
        .map(|(id, label, kind)| Habit { id: id.to_string(), label: label.to_string(), kind: kind.to_string(), count: ms.iter().filter(|m| m.habits.iter().any(|h| h == id)).count(), of: n })
        .filter(|h| h.count > 0)
        .collect();
    let refs: Vec<(&ArticleText, &Labels)> = arts.iter().map(|(a, l)| (a, l)).collect();
    p.phrases = recurring_phrases(&refs, if n >= 4 { 3 } else { 2 }, 30);
    p.categories = categories.into_iter().filter(|c| p.overall.iter().any(|o| o.category == c.id) || c.base).collect();
    p
}

/// `2.1 Materials and Methods` → `Materials and Methods`; `INTRODUCTION` → `Introduction`.
pub fn clean_title(t: &str) -> String {
    let t = t.trim();
    let t = t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ');
    let t = match t.split_once(['.', ' ']) {
        Some((head, rest)) if !head.is_empty() && head.len() <= 4 && head.chars().all(|c| matches!(c, 'I' | 'V' | 'X')) => rest.trim_start_matches(['.', ' ']),
        _ => t,
    };
    let t = t.trim();
    if t.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase()) && t.chars().any(char::is_alphabetic) {
        let low = t.to_lowercase();
        let small = ["and", "of", "the", "in", "for", "on", "to", "with", "a", "an"];
        low.split(' ')
            .enumerate()
            .map(|(i, w)| if i > 0 && small.contains(&w) { w.to_string() } else { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        t.to_string()
    }
}

// ── Storage ──

pub fn dir(workspace: &Path, template: &str) -> PathBuf {
    workspace.join(DIR).join(template)
}

pub fn load(workspace: &Path, template: &str) -> Option<Profile> {
    if !crate::paper::safe_name(template) {
        return None;
    }
    let t = std::fs::read_to_string(dir(workspace, template).join("profile.toml")).ok()?;
    toml::from_str(&t).ok()
}

/// The profile of a template **for the journal it is now**: `None` when there is none, when
/// the template has no confirmed `[venue]`, or when the profile was made for another journal
/// (the author changed which journal the template is). Every reader goes through this.
pub fn current(workspace: &Path, template: &str, venue: Option<&toml::Table>) -> Option<Profile> {
    let id = Identity::from_table(venue)?;
    load(workspace, template).filter(|p| p.openalex == id.openalex)
}

/// `profile.toml` (the data) and `profile.md` (to read).
pub fn save(workspace: &Path, p: &Profile) -> Result<PathBuf, String> {
    if !crate::paper::safe_name(&p.template) {
        return Err("模板名稱不正確".into());
    }
    let d = dir(workspace, &p.template);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let t = toml::to_string(p).map_err(|e| e.to_string())?;
    crate::paper::write_atomic(&d.join("profile.toml"), &t).map_err(|e| e.to_string())?;
    crate::paper::write_atomic(&d.join("profile.md"), &markdown(p)).map_err(|e| e.to_string())?;
    Ok(d)
}

fn pct(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

fn spread(s: &Stat) -> String {
    format!("{}（{}–{}）", pct(s.mean), pct(s.min), pct(s.max))
}

pub fn markdown(p: &Profile) -> String {
    let n = p.articles.len();
    let mut s = format!("# 期刊寫法：{}\n\n{} 由 {} 篇開放取用文章整理（{}）。這是期刊的寫法，不是作者的語氣。\n\n", p.venue, p.made, n, if p.engine == "headings" { "只看標題" } else { &p.engine });
    s.push_str(&format!("全文約 {:.0} 字（{:.0}–{:.0}）；圖 {:.1}、表 {:.1}、編號公式 {:.1}。\n\n", p.body_words.mean, p.body_words.min, p.body_words.max, p.figures.mean, p.tables.mean, p.equations.mean));
    s.push_str("## 章節\n\n");
    for sec in &p.sections {
        let titles: Vec<String> = sec.titles.iter().map(|t| format!("{}（{}/{}）", t.text, t.n, n)).collect();
        s.push_str(&format!("### {}（{}/{}）\n\n標題：{}\n\n篇幅 {}，約 {:.0} 字", sec.label, sec.present, n, titles.join("、"), spread(&sec.share), sec.words.mean));
        if has_citations(p) {
            s.push_str(&format!("，每百字引用 {:.1}", sec.citation_density.mean));
        }
        if sec.equation_share.mean > 0.0 {
            s.push_str(&format!("，公式 {}", pct(sec.equation_share.mean)));
        }
        s.push_str("\n\n");
        if !sec.subsections.is_empty() {
            s.push_str(&format!("小節：{}\n\n", sec.subsections.iter().map(|c| format!("{}（{}）", c.text, c.n)).collect::<Vec<_>>().join("、")));
        }
        if !sec.moves.is_empty() {
            s.push_str(&format!("段落順序：{}\n\n", sec.moves.iter().map(|m| m.label.clone()).collect::<Vec<_>>().join(" → ")));
        }
        for pt in &sec.points {
            s.push_str(&format!("- {} {}（{}/{}）\n", pt.label, spread(&pt.share), pt.present, sec.present));
        }
        s.push('\n');
    }
    s.push_str("## 習慣\n\n");
    for h in &p.habits {
        s.push_str(&format!("- {}：{}/{}\n", h.label, h.count, h.of));
    }
    s.push_str("\n## 常見用語（情境）\n\n");
    for ph in &p.phrases {
        s.push_str(&format!("- “{}” — {}（{}/{}）\n", ph.text, cat_label(&p.categories, &ph.situation), ph.articles, n));
    }
    s.push_str("\n## 文章\n\n");
    for a in &p.articles {
        s.push_str(&format!("- {} ({}) doi:{} — {}\n", a.title, a.year.map(|y| y.to_string()).unwrap_or_default(), a.doi, a.licence));
    }
    s
}

// ── Choosing the articles ──

/// Order candidate works by closeness to the paper's topic (`topic`: the paper's own keywords,
/// compared here, never sent): overlap of their title and keywords, then newest first.
pub fn rank(cands: &[(String, Vec<String>, String)], topic: &[String]) -> Vec<usize> {
    let words = |s: &str| -> BTreeSet<String> { tokens(s).into_iter().filter(|w| w.len() >= 3 && !STOP.contains(&w.as_str())).collect() };
    let want: BTreeSet<String> = topic.iter().flat_map(|t| words(t)).collect();
    let mut idx: Vec<(usize, usize, &str)> = cands
        .iter()
        .enumerate()
        .map(|(i, (title, kws, date))| {
            let have: BTreeSet<String> = words(title).into_iter().chain(kws.iter().flat_map(|k| words(k))).collect();
            (i, want.intersection(&have).count(), date.as_str())
        })
        .collect();
    idx.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(a.2)).then(a.0.cmp(&b.0)));
    idx.into_iter().map(|(i, _, _)| i).collect()
}

// ── For drafting and the audit (hook points; see docs/long-paper.md §D) ──

const HEADER: &str = "These are the habits of the target venue, measured from its own open-access articles. They describe the venue, not the author's voice: follow the arrangement and proportions, use the phrases only where they fit, never copy sentences.";

/// The section of the profile a heading of the author's paper belongs to.
pub fn section_for<'a>(p: &'a Profile, title: &str) -> Option<&'a SectionProfile> {
    let ct = clean_title(title).to_lowercase();
    p.sections.iter().find(|s| s.titles.iter().any(|t| t.text.to_lowercase() == ct)).or_else(|| {
        let r = canonical_role(title);
        p.sections.iter().find(|s| s.role == r)
    })
}

/// Citations could be told from the text (numbered or author–year; superscripts cannot).
pub fn has_citations(p: &Profile) -> bool {
    p.sections.iter().any(|s| s.citation_density.mean > 0.0)
}

fn section_lines(p: &Profile, s: &SectionProfile) -> String {
    let n = p.articles.len();
    let density = if has_citations(p) { format!(", {:.1} citations per 100 words", s.citation_density.mean) } else { String::new() };
    let mut out = format!(
        "Section \"{}\" (in {}/{} articles; usual titles: {}): about {} of the body (range {}–{}), ~{:.0} words{density}.",
        s.titles.first().map(|t| t.text.as_str()).unwrap_or(&s.label),
        s.present,
        n,
        s.titles.iter().take(3).map(|t| format!("\"{}\" {}/{}", t.text, t.n, n)).collect::<Vec<_>>().join(", "),
        pct(s.share.mean),
        pct(s.share.min),
        pct(s.share.max),
        s.words.mean,
    );
    if s.equation_share.mean > 0.05 {
        out.push_str(&format!(" Holds about {} of the equations.", pct(s.equation_share.mean)));
    }
    if !s.subsections.is_empty() {
        out.push_str(&format!("\n  Subsections often used: {}.", s.subsections.iter().take(6).map(|c| format!("\"{}\"", c.text)).collect::<Vec<_>>().join(", ")));
    }
    if !s.moves.is_empty() {
        out.push_str(&format!("\n  Paragraph moves in order: {}.", s.moves.iter().map(|m| m.category.as_str()).collect::<Vec<_>>().join(" → ")));
    }
    let pts: Vec<String> = s.points.iter().filter(|x| x.share.mean >= 0.03).map(|x| format!("{} {} ({}–{})", x.category, pct(x.share.mean), pct(x.share.min), pct(x.share.max))).collect();
    if !pts.is_empty() {
        out.push_str(&format!("\n  Key points with target share of this section: {}.", pts.join(", ")));
    }
    out
}

/// One section's block for the paragraph / section drafting prompt: titles to use, the moves,
/// key points with target proportions, the venue's must-haves that belong here, phrases.
pub fn prompt_block(p: &Profile, section_title: &str, sub: Option<&Submission>) -> Option<String> {
    let s = section_for(p, section_title)?;
    let mut out = format!("{HEADER}\nVenue: {}.\n{}", p.venue, section_lines(p, s));
    let musts: Vec<String> = p.habits.iter().filter(|h| h.usual() && h.kind != "structure" && habit_section(&h.id) == s.role).map(|h| format!("{} ({}/{} articles)", h.id, h.count, h.of)).collect();
    if !musts.is_empty() {
        out.push_str(&format!("\n  Most of the venue's articles include here: {}.", musts.join(", ")));
    }
    if let Some(sub) = sub {
        let req: Vec<&String> = sub.required_sections.iter().filter(|r| canonical_role(r) == s.role).collect();
        if !req.is_empty() {
            out.push_str(&format!("\n  Required by the journal: {}.", req.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(", ")));
        }
    }
    let ids: BTreeSet<&str> = s.points.iter().map(|x| x.category.as_str()).collect();
    let ph: Vec<String> = p.phrases.iter().filter(|x| ids.contains(x.situation.as_str())).take(8).map(|x| format!("\"{}\" ({})", x.text, x.situation)).collect();
    if !ph.is_empty() {
        out.push_str(&format!("\n  Phrases common in this venue for these situations: {}.", ph.join(", ")));
    }
    Some(out)
}

/// The whole-paper block for the outline: sections in the venue's order with titles, lengths,
/// moves and proportions, the structure habits, the counts of figures / tables / equations.
pub fn outline_block(p: &Profile, sub: Option<&Submission>) -> String {
    // no caption read in any article is the PDF reader's miss, not a journal without figures: left out
    let floats = if p.figures.max > 0.0 || p.tables.max > 0.0 {
        format!("; figures ~{:.0}, tables ~{:.0}, numbered equations ~{:.0}", p.figures.mean, p.tables.mean, p.equations.mean)
    } else {
        String::new()
    };
    let mut out = format!(
        "{HEADER}\nVenue: {} ({} articles, {}). Body ~{:.0} words ({:.0}–{:.0}){floats}.\n",
        p.venue,
        p.articles.len(),
        p.made,
        p.body_words.mean,
        p.body_words.min,
        p.body_words.max,
    );
    for s in p.sections.iter().filter(|s| is_body(&s.role) && s.present * 2 >= p.articles.len()) {
        out.push_str(&format!("- {}\n", section_lines(p, s)));
    }
    let st: Vec<String> = p.habits.iter().filter(|h| h.kind == "structure").map(|h| format!("{} {}/{}", h.id, h.count, h.of)).collect();
    if !st.is_empty() {
        out.push_str(&format!("Structure habits: {}.\n", st.join(", ")));
    }
    if let Some(s) = sub {
        out.push_str(&submission_lines(s));
    }
    out
}

/// What the venue emphasises (for the candidate answers step): the categories that take the
/// most space, and the content points most of its articles make.
pub fn emphasis_block(p: &Profile) -> String {
    let top: Vec<String> = p.overall.iter().take(6).map(|x| format!("{} {}", x.category, pct(x.share.mean))).collect();
    let musts: Vec<String> = p.habits.iter().filter(|h| h.usual() && h.kind == "content").map(|h| format!("{} {}/{}", h.id, h.count, h.of)).collect();
    format!("The venue {} spends its words mostly on: {}. Most of its articles make these points: {}. Prefer candidate answers that serve these.", p.venue, top.join(", "), if musts.is_empty() { "none in particular".into() } else { musts.join(", ") })
}

pub fn submission_lines(s: &Submission) -> String {
    let mut v = Vec::new();
    if let Some(w) = s.words {
        v.push(format!("at most {w} words"));
    }
    if let Some(pg) = s.pages {
        v.push(format!("at most {pg} pages"));
    }
    if let Some(w) = s.abstract_words {
        v.push(format!("abstract ≤ {w} words"));
    }
    if !s.abstract_structure.is_empty() {
        v.push(format!("abstract: {}", s.abstract_structure.join(" / ")));
    }
    if !s.required_sections.is_empty() {
        v.push(format!("required sections: {}", s.required_sections.join(", ")));
    }
    if !s.required_statements.is_empty() {
        v.push(format!("required statements: {}", s.required_statements.join(", ")));
    }
    if v.is_empty() { String::new() } else { format!("Submission requirements: {}.\n", v.join("; ")) }
}

/// The section a content point belongs in (for the per-section block and the audit's place).
pub fn habit_section(id: &str) -> &'static str {
    match id {
        "contributions_list" | "clinical_relevance" => "introduction",
        "pros_cons" | "limitations" => "discussion",
        "future_work" => "conclusion",
        "ablation" | "comparison_sota" | "statistical_tests" | "external_validation" => "results",
        "reproducibility" => "experiments",
        "data_availability" | "code_availability" | "ethics_statement" => "statements",
        _ => "",
    }
}

/// One check of the audit.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CheckItem {
    pub id: String,
    pub label: String,
    /// `submission` (the journal requires it) or `venue` (most of its articles do it).
    pub source: String,
    /// Where it usually goes (a role id; empty = anywhere).
    pub section: String,
    pub count: usize,
    pub of: usize,
    pub required: bool,
}

/// The audit checklist: the submission's required statements and sections (required), then
/// the venue's usual content points, statements and structure habits (with how many of the
/// articles show them: "5/5 articles discuss pros & cons").
pub fn audit_checklist(p: Option<&Profile>, sub: Option<&Submission>) -> Vec<CheckItem> {
    let mut v = Vec::new();
    if let Some(s) = sub {
        for st in &s.required_statements {
            v.push(CheckItem { id: format!("req:{st}"), label: st.clone(), source: "submission".into(), section: "statements".into(), required: true, ..Default::default() });
        }
        for sec in &s.required_sections {
            v.push(CheckItem { id: format!("sec:{sec}"), label: sec.clone(), source: "submission".into(), section: canonical_role(sec).into(), required: true, ..Default::default() });
        }
    }
    if let Some(p) = p {
        for h in p.habits.iter().filter(|h| h.usual()) {
            v.push(CheckItem { id: h.id.clone(), label: h.label.clone(), source: "venue".into(), section: habit_section(&h.id).into(), count: h.count, of: h.of, required: false });
        }
    }
    v
}

/// A heading from a PDF made safe for a LaTeX section title: typographic punctuation as
/// ASCII, other non-ASCII characters dropped, LaTeX specials escaped (`meta::tex_text`).
pub fn tex_safe_title(t: &str) -> String {
    let mut out = String::new();
    for c in clean_title(t).chars() {
        match c {
            '‘' | '’' | '′' => out.push('\''),
            '“' | '”' => out.push('"'),
            '–' => out.push_str("--"),
            '—' => out.push_str("---"),
            '\\' | '{' | '}' | '^' | '~' => {}
            c if c.is_ascii() => out.push(c),
            _ => {}
        }
    }
    crate::meta::tex_text(&out.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The paper's skeleton in the venue's own arrangement and naming: the sections most of its
/// articles have (in their usual order, with the usual title), and their common subsections.
/// No intent text: the section files start empty (the moves and shares reach the model through
/// [`outline_block`] / [`prompt_block`], never the LaTeX).
pub fn skeleton(p: &Profile) -> Vec<crate::meta::OutlineSection> {
    let n = p.articles.len().max(1);
    let ok = |t: &str| !t.is_empty() && t.len() <= 80;
    p.sections
        .iter()
        .filter(|s| is_body(&s.role) && s.present * 2 >= n)
        .map(|s| crate::meta::OutlineSection {
            title: s.titles.first().map(|t| tex_safe_title(&t.text)).filter(|t| ok(t)).unwrap_or_else(|| default_title(&s.role).into()),
            intent: String::new(),
            subsections: s.subsections.iter().filter(|c| c.n * 2 >= s.present.max(2)).map(|c| tex_safe_title(&c.text)).filter(|t| ok(t)).take(6).map(|title| crate::meta::OutlineSub { title, intent: String::new() }).collect(),
        })
        .collect()
}

fn default_title(role: &str) -> &'static str {
    match role {
        "introduction" => "Introduction",
        "related" => "Related Work",
        "background" => "Background",
        "methods" => "Methods",
        "data" => "Data",
        "experiments" => "Experiments",
        "results" => "Results",
        "discussion" => "Discussion",
        "limitations" => "Limitations",
        "conclusion" => "Conclusion",
        _ => "Section",
    }
}

/// A compact line for the template picker: the usual sections in order, then the length.
pub fn summary(p: &Profile) -> String {
    let n = p.articles.len().max(1);
    let secs: Vec<String> = p.sections.iter().filter(|s| is_body(&s.role) && s.present * 2 >= n).map(|s| s.titles.first().map(|t| t.text.clone()).unwrap_or_else(|| s.label.clone())).collect();
    format!("{} · 約 {:.0} 字", secs.join(" → "), (p.body_words.mean / 100.0).round() * 100.0)
}

#[cfg(test)]
#[path = "venueprofile_tests.rs"]
mod tests;
