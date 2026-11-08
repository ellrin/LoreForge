//! Consistency checks over a scanned document and its registry.
//!
//! Every issue has a stable `code`, a severity, a Traditional Chinese
//! message and locations (file, line, column, paragraph, sentence) the
//! editor can jump to. See the README for the list of codes.

use crate::abbrev;
use crate::math;
use crate::model::{Kind, Notation};
use crate::scan::{Area, ItemKind, Scan};
use crate::source::Loc;
use crate::symbols::{self, DefKind, SymbolInfo, Symbols};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub code: String,
    /// symbol | equation | abbrev | ref | style | registry
    pub category: String,
    pub severity: Severity,
    pub message: String,
    /// Symbol key, abbreviation, label or word the issue is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Registry entry id, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    pub locations: Vec<Loc>,
    /// Supporting places (e.g. where the definition finally comes).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<Loc>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolSummary {
    pub key: String,
    pub uses: usize,
    pub defined: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meaning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_use: Option<Loc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defined_at: Option<Loc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AbbrevSummary {
    pub short: String,
    pub uses: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub version: u32,
    pub files: Vec<String>,
    pub issues: Vec<Issue>,
    /// code → count
    pub summary: BTreeMap<String, usize>,
    pub symbols: Vec<SymbolSummary>,
    pub abbreviations: Vec<AbbrevSummary>,
}

impl Report {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }
    pub fn by_code(&self, code: &str) -> Vec<&Issue> {
        self.issues.iter().filter(|i| i.code == code).collect()
    }
}

#[derive(Debug, Clone)]
pub struct CheckOptions {
    /// Captions may define symbols (off: a caption that uses a symbol first is reported).
    pub caption_defines: bool,
    /// A display equation may use a symbol defined up to this many sentences later ("where …").
    pub grace_sentences: usize,
    /// Abbreviations that never need expanding.
    pub allow_abbrevs: Vec<String>,
    /// Report `\label`s of sections that are never referenced.
    pub section_labels: bool,
    /// Report abbreviations the abstract uses without expanding.
    pub abstract_abbrevs: bool,
    pub index_letters: Vec<String>,
    /// Cap on locations listed per issue.
    pub max_locations: usize,
}

impl Default for CheckOptions {
    fn default() -> Self {
        CheckOptions {
            caption_defines: false,
            grace_sentences: 3,
            allow_abbrevs: ["IEEE", "ID", "PDF", "URL", "USA", "UK", "HTML", "GPU", "CPU", "RGB", "DOI", "ISBN"].iter().map(|s| s.to_string()).collect(),
            section_labels: false,
            abstract_abbrevs: true,
            index_letters: math::DEFAULT_INDEX.iter().map(|s| s.to_string()).collect(),
            max_locations: 30,
        }
    }
}

struct Ctx<'a> {
    scan: &'a Scan,
    syms: &'a Symbols,
    reg: &'a Notation,
    opt: &'a CheckOptions,
    /// canonical latex → entry id
    reg_keys: BTreeMap<String, String>,
    conventional: BTreeSet<String>,
    issues: Vec<Issue>,
}

fn norm_meaning(m: &str) -> String {
    m.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|w| !w.is_empty() && !matches!(*w, "the" | "a" | "an" | "of" | "for" | "each" | "every" | "all" | "in" | "this"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl<'a> Ctx<'a> {
    fn occ_loc(&self, off: usize, area: Area, display: bool) -> Loc {
        let c = if display && matches!(area, Area::Body | Area::Abstract) { "equation" } else { area.as_str() };
        self.scan.loc(off, c)
    }

    fn push(&mut self, code: &str, category: &str, severity: Severity, message: String, subject: Option<String>, locations: Vec<Loc>) -> &mut Issue {
        let mut locations = locations;
        locations.truncate(self.opt.max_locations);
        let entry = subject.as_ref().and_then(|s| self.reg_keys.get(s).cloned());
        self.issues.push(Issue { code: code.into(), category: category.into(), severity, message, subject, entry, locations, related: vec![], details: vec![] });
        self.issues.last_mut().unwrap()
    }

    fn def_loc(&self, d: &symbols::Def) -> Loc {
        let seg = &self.scan.math[d.seg];
        let mut l = self.occ_loc(d.off, seg.ctx.area, seg.display);
        l.context = "definition".into();
        l
    }

    fn reg_meaning(&self, key: &str) -> Option<String> {
        let id = self.reg_keys.get(key)?;
        let e = self.reg.get(id)?;
        e.meaning.clone().or(e.meaning_en.clone())
    }

    fn symbols(&mut self) {
        let grace = self.opt.grace_sentences;
        let syms = self.syms;
        for info in syms.table.values() {
            if self.conventional.contains(&info.key) {
                continue;
            }
            let key = info.key.clone();
            if info.defs.is_empty() {
                let locs: Vec<Loc> = info.occs.iter().map(|o| self.occ_loc(o.off, o.area, o.display)).collect();
                let mut msg = format!("符號 ${key}$ 用了 {} 次，但正文沒有定義", info.occs.len());
                if let Some(m) = self.reg_meaning(&key) {
                    msg.push_str(&format!("（對照表：{m}）"));
                }
                self.push("undefined_symbol", "symbol", Severity::Warning, msg, Some(key.clone()), locs);
                continue;
            }
            let d = info.first_def().unwrap().clone();
            let early: Vec<&symbols::Occ> = info
                .occs
                .iter()
                .filter(|o| o.off < d.off && o.sentence != d.sentence && o.role != symbols::Role::Qualifier)
                .filter(|o| !(o.display && d.sentence >= o.sentence && d.sentence - o.sentence <= grace && o.area != Area::Caption))
                .collect();
            if !early.is_empty() {
                let in_caption = early.iter().any(|o| o.area == Area::Caption);
                let dl = self.def_loc(&d);
                let first = self.occ_loc(early[0].off, early[0].area, early[0].display);
                let msg = if in_caption {
                    format!("符號 ${key}$ 先出現在圖表說明（{}:{}），正文到 {}:{} 才定義", first.file, first.line, dl.file, dl.line)
                } else {
                    format!("符號 ${key}$ 先用後定義：第一次用在 {}:{}，到 {}:{} 才定義", first.file, first.line, dl.file, dl.line)
                };
                let locs = early.iter().map(|o| self.occ_loc(o.off, o.area, o.display)).collect();
                let iss = self.push("used_before_definition", "symbol", Severity::Warning, msg, Some(key.clone()), locs);
                iss.related.push(dl);
            }
            // defined twice with different content
            let mut contents: Vec<(String, &symbols::Def)> = Vec::new();
            for d in &info.defs {
                let c = match d.kind {
                    DefKind::Equals => d.rhs.clone().map(|r| format!("= {r}")),
                    DefKind::Denote if !d.shared => d.meaning.as_deref().map(norm_meaning).map(|m| format!("「{m}」")),
                    _ => None,
                };
                if let Some(c) = c
                    && !contents.iter().any(|(x, _)| *x == c) {
                        contents.push((c, d));
                    }
            }
            let eqs = contents.iter().filter(|(c, _)| c.starts_with('=')).count();
            let dens = contents.len() - eqs;
            if eqs >= 2 || dens >= 2 {
                let locs: Vec<Loc> = contents.iter().map(|(_, d)| self.def_loc(d)).collect();
                let details: Vec<String> = contents.iter().zip(&locs).map(|((c, _), l)| format!("{}:{}  ${key}$ {c}", l.file, l.line)).collect();
                let msg = format!("符號 ${key}$ 被定義了 {} 次，內容不同", contents.len());
                let iss = self.push("symbol_multiple_meanings", "symbol", Severity::Warning, msg, Some(key.clone()), locs);
                iss.details = details;
            }
            // defined but never used elsewhere
            if info.defs.iter().any(|d| d.kind.explicit()) {
                let def_segs: BTreeSet<usize> = info.defs.iter().map(|d| d.seg).collect();
                if info.occs.iter().all(|o| def_segs.contains(&o.seg)) {
                    let msg = format!("符號 ${key}$ 定義了但後文沒有再用到");
                    let l = self.def_loc(&d);
                    self.push("unused_definition", "symbol", Severity::Info, msg, Some(key.clone()), vec![l]);
                }
            }
        }
        self.letter_overloading();
        self.same_meaning();
    }

    /// One letter in several fonts with different meanings (bold m vs m),
    /// or a name subscript that is also a variable (`\lambda_m` vs `m`).
    fn letter_overloading(&mut self) {
        let syms = self.syms;
        let defined = |i: &SymbolInfo| i.defs.iter().any(|d| !matches!(d.kind, DefKind::Inherited | DefKind::Derived));
        let mut by_letter: BTreeMap<String, Vec<&SymbolInfo>> = BTreeMap::new();
        for info in syms.table.values() {
            if self.conventional.contains(&info.key) || !defined(info) {
                continue;
            }
            by_letter.entry(info.sample.letter()).or_default().push(info);
        }
        for (letter, infos) in by_letter {
            let fonts: BTreeSet<math::Font> = infos.iter().map(|i| i.sample.font).collect();
            let plain_bare = syms.table.get(&letter).filter(|i| defined(i) && i.sample.is_bare_plain());
            let clashes: Vec<&SymbolInfo> = match plain_bare {
                Some(_) => syms.table.values().filter(|i| i.sample.sub_letter.as_deref() == Some(letter.as_str())).collect(),
                None => vec![],
            };
            if fonts.len() < 2 && clashes.is_empty() {
                continue;
            }
            let mut variants: Vec<&SymbolInfo> = if fonts.len() >= 2 {
                // one representative per font: the defined variant without scripts first
                let mut v: Vec<&SymbolInfo> = Vec::new();
                for f in &fonts {
                    let mut c: Vec<&&SymbolInfo> = infos.iter().filter(|i| i.sample.font == *f).collect();
                    c.sort_by_key(|i| (i.sample.sub_name().is_some() || i.sample.sup_name().is_some() || !i.sample.accents.is_empty(), i.first_def().map(|d| d.off)));
                    v.push(c[0]);
                }
                v
            } else {
                vec![plain_bare.unwrap()]
            };
            for c in &clashes {
                if !variants.iter().any(|v| v.key == c.key) {
                    variants.push(c);
                }
            }
            let mut details = Vec::new();
            let mut locs = Vec::new();
            for v in &variants {
                let loc = v.first_def().map(|d| self.def_loc(d)).or_else(|| v.first_use().map(|o| self.occ_loc(o.off, o.area, o.display)));
                let what = if v.sample.sub_letter.as_deref() == Some(letter.as_str()) && v.sample.base != letter {
                    format!("下標 {letter}")
                } else {
                    symbols::font_name(v.sample.font).to_string()
                };
                let meaning = v.meaning().or_else(|| self.reg_meaning(&v.key)).unwrap_or_else(|| "（未說明）".into());
                if let Some(l) = &loc {
                    details.push(format!("${}$（{what}）：{meaning} — {}:{}", v.key, l.file, l.line));
                    locs.push(l.clone());
                }
            }
            let keys: Vec<String> = variants.iter().map(|v| format!("${}$", v.key)).collect();
            let msg = format!("字母 {letter} 有多種意思：{}", keys.join("、"));
            let iss = self.push("symbol_multiple_meanings", "symbol", Severity::Warning, msg, Some(letter.clone()), locs);
            iss.details = details;
        }
    }

    /// Different symbols the text describes with the same words.
    fn same_meaning(&mut self) {
        let syms = self.syms;
        let mut by: BTreeMap<String, Vec<(&SymbolInfo, &symbols::Def)>> = BTreeMap::new();
        for info in syms.table.values() {
            for d in &info.defs {
                let ok = !d.shared && (d.kind == DefKind::Denote || d.kind == DefKind::Apposition && d.meaning.as_deref().is_some_and(|m| m.split_whitespace().count() >= 2));
                if let (true, Some(m)) = (ok, &d.meaning) {
                    let n = norm_meaning(m);
                    if !n.is_empty() && !by.get(&n).is_some_and(|v| v.iter().any(|(i, _)| i.key == info.key)) {
                        by.entry(n).or_default().push((info, d));
                    }
                }
            }
        }
        for (m, v) in by {
            if v.len() < 2 {
                continue;
            }
            let locs: Vec<Loc> = v.iter().map(|(_, d)| self.def_loc(d)).collect();
            let keys: Vec<String> = v.iter().map(|(i, _)| format!("${}$", i.key)).collect();
            let msg = format!("「{m}」用了 {} 個不同符號：{}", v.len(), keys.join("、"));
            self.push("meaning_multiple_symbols", "symbol", Severity::Warning, msg, Some(m), locs);
        }
    }

    fn equations(&mut self) {
        let grace = self.opt.grace_sentences;
        for (si, seg) in self.scan.math.iter().enumerate() {
            if !seg.display || !matches!(seg.ctx.area, Area::Body | Area::Abstract) {
                continue;
            }
            let mut bad: Vec<String> = Vec::new();
            for info in self.syms.table.values() {
                if self.conventional.contains(&info.key) || !info.occs.iter().any(|o| o.seg == si) {
                    continue;
                }
                let ok = match info.first_def() {
                    None => false,
                    Some(d) => d.off <= seg.end || d.seg == si || (d.sentence >= seg.sentence && d.sentence - seg.sentence <= grace),
                };
                if !ok {
                    let later = info.first_def().map(|d| {
                        let l = self.def_loc(d);
                        format!("（{}:{} 才定義）", l.file, l.line)
                    });
                    bad.push(format!("${}${}", info.key, later.unwrap_or_default()));
                }
            }
            if bad.is_empty() {
                continue;
            }
            let l = self.scan.loc(seg.start, "equation");
            let name = seg.labels.first().map(|x| format!("方程式 {x}")).unwrap_or_else(|| format!("方程式（{}:{}）", l.file, l.line));
            let msg = format!("{name} 的變數未定義：{}", bad.join("、"));
            let subj = seg.labels.first().cloned();
            let iss = self.push("equation_undefined_variables", "equation", Severity::Warning, msg, subj, vec![l]);
            iss.details = bad;
        }
    }

    fn abbreviations(&mut self) -> BTreeMap<String, abbrev::AbbrevInfo> {
        let abbr = abbrev::analyze(self.scan);
        let reg_abbr: BTreeMap<String, &crate::model::Entry> =
            self.reg.entries.iter().filter(|e| e.kind() == Kind::Abbrev).filter_map(|e| e.short.clone().map(|s| (s, e))).collect();
        let has_abstract = self.scan.items.iter().any(|i| i.ctx.area == Area::Abstract);
        for (short, a) in &abbr {
            if self.opt.allow_abbrevs.iter().any(|x| x == short) || reg_abbr.get(short).is_some_and(|e| e.is_conventional()) {
                continue;
            }
            let entry = reg_abbr.get(short).map(|e| e.id.clone());
            let body_uses: Vec<&abbrev::Use> = a.uses.iter().filter(|u| u.area != Area::Abstract).collect();
            let body_exp: Vec<&abbrev::Expansion> = a.expansions.iter().filter(|e| e.area != Area::Abstract).collect();
            let loc_u = |s: &Self, u: &abbrev::Use| s.scan.loc(u.off, u.area.as_str());
            if !body_uses.is_empty() && body_exp.is_empty() {
                let hint = reg_abbr.get(short).and_then(|e| e.long.clone()).map(|l| format!("（對照表：{l}）")).unwrap_or_default();
                let msg = format!("縮寫 {short} 在正文用了 {} 次，但從未展開{hint}", body_uses.len());
                let locs = body_uses.iter().map(|u| loc_u(self, u)).collect();
                let iss = self.push("abbrev_never_expanded", "abbrev", Severity::Warning, msg, Some(short.clone()), locs);
                iss.entry = entry.clone();
            } else if let Some(first_exp) = body_exp.first() {
                let early: Vec<&&abbrev::Use> = body_uses.iter().filter(|u| u.off < first_exp.off).collect();
                if !early.is_empty() {
                    let el = self.scan.loc(first_exp.off, first_exp.area.as_str());
                    let msg = format!("縮寫 {short} 在展開（{}:{}「{}」）之前就用了 {} 次", el.file, el.line, first_exp.long, early.len());
                    let locs = early.iter().map(|u| loc_u(self, u)).collect();
                    let iss = self.push("abbrev_used_before_expansion", "abbrev", Severity::Warning, msg, Some(short.clone()), locs);
                    iss.related.push(el);
                    iss.entry = entry.clone();
                }
                if body_exp.len() >= 2 {
                    let locs: Vec<Loc> = body_exp.iter().map(|e| self.scan.loc(e.off, e.area.as_str())).collect();
                    let msg = format!("縮寫 {short} 在正文展開了 {} 次", body_exp.len());
                    let iss = self.push("abbrev_expanded_twice", "abbrev", Severity::Info, msg, Some(short.clone()), locs);
                    iss.entry = entry.clone();
                }
            }
            if self.opt.abstract_abbrevs && has_abstract {
                let abs_uses: Vec<&abbrev::Use> = a.uses.iter().filter(|u| u.area == Area::Abstract).collect();
                let abs_exp = a.expansions.iter().any(|e| e.area == Area::Abstract);
                if !abs_uses.is_empty() && !abs_exp {
                    let msg = format!("摘要用了縮寫 {short} 但沒有在摘要內展開");
                    let locs = abs_uses.iter().map(|u| loc_u(self, u)).collect();
                    let iss = self.push("abbrev_abstract", "abbrev", Severity::Info, msg, Some(short.clone()), locs);
                    iss.entry = entry.clone();
                }
            }
        }
        abbr
    }

    fn variants(&mut self) {
        for g in abbrev::spelling_variants(self.scan) {
            let words: Vec<String> = g.iter().map(|(w, _)| w.clone()).collect();
            let locs = g.iter().map(|(_, off)| self.scan.loc(*off, "body")).collect();
            let msg = format!("同一個詞有不同寫法：{}", words.join(" / "));
            self.push("spelling_variants", "style", Severity::Warning, msg, Some(words.join(" / ")), locs);
        }
    }

    fn references(&mut self) {
        let scan = self.scan;
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for l in &scan.labels {
            *seen.entry(l.key.as_str()).or_default() += 1;
        }
        let referenced: BTreeSet<&str> = scan.refs.iter().map(|r| r.key.as_str()).collect();
        let mut dup_done = BTreeSet::new();
        for l in &scan.labels {
            if seen[l.key.as_str()] > 1 && dup_done.insert(l.key.clone()) {
                let locs = scan.labels.iter().filter(|x| x.key == l.key).map(|x| scan.loc(x.off, x.ctx.area.as_str())).collect();
                self.push("label_duplicate", "ref", Severity::Error, format!("標籤 {} 重複定義", l.key), Some(l.key.clone()), locs);
            }
            let check = matches!(l.kind.as_str(), "fig" | "tab" | "eq") || l.kind == "sec" && self.opt.section_labels;
            if check && !referenced.contains(l.key.as_str()) {
                let what = match l.kind.as_str() {
                    "fig" => "圖",
                    "tab" => "表",
                    "eq" => "方程式",
                    _ => "章節",
                };
                let msg = format!("{what}標籤 {} 從未被引用", l.key);
                let loc = scan.loc(l.off, if l.kind == "eq" { "equation" } else { l.ctx.area.as_str() });
                self.push("label_unreferenced", "ref", Severity::Warning, msg, Some(l.key.clone()), vec![loc]);
            }
        }
        let labels: BTreeSet<&str> = scan.labels.iter().map(|l| l.key.as_str()).collect();
        let mut missing: BTreeMap<&str, Vec<Loc>> = BTreeMap::new();
        for r in &scan.refs {
            if !labels.contains(r.key.as_str()) {
                missing.entry(r.key.as_str()).or_default().push(scan.loc(r.off, r.ctx.area.as_str()));
            }
        }
        for (k, locs) in missing {
            self.push("ref_undefined", "ref", Severity::Error, format!("引用了不存在的標籤 {k}"), Some(k.to_string()), locs);
        }
        // hard-coded "Section III-C", "Fig. 2", "Table 1", "Eq. (3)"
        const WORDS: &[&str] = &[
            "Section", "Sections", "Sec", "Secs", "Fig", "Figs", "Figure", "Figures", "Table", "Tables", "Tab", "Eq", "Eqs", "Equation", "Equations",
            "Algorithm", "Appendix", "Chapter", "section", "figure", "table", "equation",
        ];
        let items = &scan.items;
        for (i, it) in items.iter().enumerate() {
            let ItemKind::Word(w) = &it.kind else { continue };
            if !WORDS.contains(&w.as_str()) {
                continue;
            }
            let mut j = i + 1;
            if matches!(items.get(j).map(|x| &x.kind), Some(ItemKind::Punct('.'))) {
                j += 1;
            }
            let mut paren = false;
            if matches!(items.get(j).map(|x| &x.kind), Some(ItemKind::Punct('('))) && w.starts_with("Eq") {
                j += 1;
                paren = true;
            }
            let Some(ItemKind::Word(n)) = items.get(j).map(|x| &x.kind) else { continue };
            if items[j].sentence != it.sentence {
                continue;
            }
            let numeric = n.chars().next().is_some_and(|c| c.is_ascii_digit()) && n.chars().all(|c| c.is_ascii_alphanumeric()) && n.len() <= 4;
            let roman = {
                let head = n.split('-').next().unwrap_or("");
                !head.is_empty() && head.chars().all(|c| matches!(c, 'I' | 'V' | 'X')) && n.split('-').nth(1).is_none_or(|t| t.len() <= 2 && t.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
            };
            let appendix = w == "Appendix" && n.len() == 1 && n.chars().all(|c| c.is_ascii_uppercase());
            if numeric || roman || appendix || paren && numeric {
                let end = items[j].end;
                let text = scan.doc.text[it.start..end].to_string();
                let msg = format!("「{}」是寫死的編號，請改用 \\ref / \\eqref", text.replace('~', " "));
                let loc = scan.loc(it.start, it.ctx.area.as_str());
                self.push("hardcoded_reference", "ref", Severity::Warning, msg, Some(text), vec![loc]);
            }
        }
        // the word in front of \ref for each label kind
        let kinds: BTreeMap<&str, &str> = scan.labels.iter().map(|l| (l.key.as_str(), l.kind.as_str())).collect();
        let mut forms: BTreeMap<(&str, String), Vec<usize>> = BTreeMap::new();
        for r in &scan.refs {
            let Some(kind) = kinds.get(r.key.as_str()) else { continue };
            if r.item == 0 {
                continue;
            }
            let mut k = r.item - 1;
            let dot = matches!(items[k].kind, ItemKind::Punct('.'));
            if dot && k > 0 {
                k -= 1;
            }
            if let ItemKind::Word(w) = &items[k].kind {
                // sentence-initial words follow other rules ("Figure 2 shows" vs "in Fig. 2")
                let first = scan.sentences.get(items[k].sentence).is_some_and(|s| s.items.start == k);
                if !first && w.chars().next().is_some_and(|c| c.is_uppercase()) {
                    forms.entry((kind, format!("{w}{}", if dot { "." } else { "" }))).or_default().push(r.off);
                }
            }
        }
        let mut per_kind: BTreeMap<&str, Vec<(String, Vec<usize>)>> = BTreeMap::new();
        for ((k, f), offs) in forms {
            per_kind.entry(k).or_default().push((f, offs));
        }
        for (_k, mut v) in per_kind {
            if v.len() < 2 {
                continue;
            }
            v.sort_by_key(|x| std::cmp::Reverse(x.1.len()));
            let names: Vec<String> = v.iter().map(|(f, o)| format!("{f}~\\ref（{} 次）", o.len())).collect();
            let locs = v[1..].iter().flat_map(|(_, o)| o.iter().map(|x| scan.loc(*x, "body"))).collect();
            self.push("ref_prefix_style", "style", Severity::Info, format!("引用前綴不一致：{}", names.join("、")), None, locs);
        }
    }

    fn captions(&mut self) {
        let scan = self.scan;
        for kind in ["figure", "table"] {
            struct C {
                off: usize,
                period: bool,
                upper: bool,
                title: bool,
                above: Option<bool>,
            }
            let mut cs: Vec<C> = Vec::new();
            for f in scan.floats.iter().filter(|f| f.kind == kind) {
                let Some(ci) = f.caption else { continue };
                let c = &scan.captions[ci];
                let raw = scan.doc.text[c.start..c.end].trim();
                let words: Vec<&str> = scan.items.iter().filter(|it| it.start >= c.start && it.end <= c.end).filter_map(|it| if let ItemKind::Word(w) = &it.kind { Some(w.as_str()) } else { None }).collect();
                let long: Vec<&&str> = words.iter().skip(1).filter(|w| w.len() > 3 && abbrev::acronyms(w).is_empty()).collect();
                let caps = long.iter().filter(|w| w.chars().next().is_some_and(|c| c.is_uppercase())).count();
                cs.push(C {
                    off: c.start,
                    period: raw.ends_with('.'),
                    upper: words.first().and_then(|w| w.chars().next()).is_none_or(|c| !c.is_lowercase()),
                    title: long.len() >= 3 && caps * 10 >= long.len() * 6,
                    above: f.tabular_at.map(|t| c.start < t),
                });
            }
            if cs.len() < 2 {
                continue;
            }
            let what = if kind == "figure" { "圖" } else { "表" };
            let rule = |name: &str, get: &dyn Fn(&C) -> Option<bool>, yes: &str, no: &str, this: &mut Self| {
                let vals: Vec<(usize, bool)> = cs.iter().filter_map(|c| get(c).map(|v| (c.off, v))).collect();
                let t = vals.iter().filter(|v| v.1).count();
                let f = vals.len() - t;
                if t == 0 || f == 0 {
                    return;
                }
                let minority = t < f;
                let locs: Vec<Loc> = vals.iter().filter(|v| v.1 == minority).map(|v| scan.loc(v.0, "caption")).collect();
                let msg = format!("{what}說明{name}不一致：{t} 個{yes}、{f} 個{no}");
                this.push("caption_style", "style", Severity::Info, msg, None, locs);
            };
            rule("結尾句點", &|c| Some(c.period), "有句點", "沒有", self);
            rule("開頭大寫", &|c| Some(c.upper), "大寫開頭", "小寫開頭", self);
            rule("大小寫風格", &|c| Some(c.title), "Title Case", "句子大小寫", self);
            rule("位置", &|c| c.above, "在表格上方", "在下方", self);
        }
    }

    fn registry(&mut self, abbr: &BTreeMap<String, abbrev::AbbrevInfo>) {
        let reg = self.reg;
        let scan = self.scan;
        let labels: BTreeSet<&str> = scan.labels.iter().map(|l| l.key.as_str()).collect();
        let existing: BTreeSet<String> = scan.doc_macros.clone();
        for p in reg.validate(&existing) {
            let msg = format!("對照表「{}」的 {}：{}", p.id, p.field, p.message);
            let iss = self.push("registry_invalid", "registry", Severity::Error, msg, None, vec![]);
            iss.entry = Some(p.id.clone());
        }
        for e in &reg.entries {
            let used = match e.kind() {
                Kind::Symbol => e.latex.as_deref().map(math::canonical).is_some_and(|k| self.syms.table.contains_key(&k)),
                Kind::Operator => e.latex.as_deref().is_some_and(|l| {
                    let c = math::canonical(l);
                    let name = c.trim_start_matches("\\operatorname{").trim_end_matches('}').trim_start_matches('\\').to_string();
                    self.syms.operators.contains_key(&name) || scan.math.iter().any(|m| m.src.contains(l.trim()))
                }),
                Kind::Abbrev => e.short.as_deref().is_some_and(|s| abbr.contains_key(s)),
                Kind::Figure | Kind::Table | Kind::Equation => match &e.label {
                    Some(l) => labels.contains(l.as_str()),
                    None => true,
                },
                Kind::Term => true,
            } || e.macro_name().is_some_and(|m| scan.doc.text[scan.doc.body.clone()].contains(&format!("\\{m}")));
            if !used && !e.is_conventional() {
                let what = e.latex.clone().or(e.short.clone()).or(e.label.clone()).unwrap_or_default();
                let msg = format!("對照表的「{}」（{}）在文件中沒有用到", e.id, what);
                let iss = self.push("unused_entry", "registry", Severity::Info, msg, None, vec![]);
                iss.entry = Some(e.id.clone());
            }
        }
        // registry: one symbol, several meanings / one meaning, several symbols
        let mut by_key: BTreeMap<String, Vec<&crate::model::Entry>> = BTreeMap::new();
        let mut by_meaning: BTreeMap<String, Vec<&crate::model::Entry>> = BTreeMap::new();
        for e in reg.entries.iter().filter(|e| matches!(e.kind(), Kind::Symbol | Kind::Operator)) {
            if let Some(l) = &e.latex {
                by_key.entry(math::canonical(l)).or_default().push(e);
            }
            if let Some(m) = e.meaning.as_deref().or(e.meaning_en.as_deref()) {
                let n = norm_meaning(m);
                if !n.is_empty() {
                    by_meaning.entry(n).or_default().push(e);
                }
            }
        }
        for (k, v) in by_key {
            let meanings: BTreeSet<String> = v.iter().filter_map(|e| e.meaning.as_deref().or(e.meaning_en.as_deref()).map(norm_meaning)).collect();
            if v.len() >= 2 && meanings.len() >= 2 {
                let ids: Vec<&str> = v.iter().map(|e| e.id.as_str()).collect();
                let msg = format!("對照表中 ${k}$ 有 {} 個意思（{}）", meanings.len(), ids.join("、"));
                let iss = self.push("symbol_multiple_meanings", "registry", Severity::Warning, msg, Some(k.clone()), vec![]);
                iss.details = v.iter().map(|e| format!("{}：{}", e.id, e.meaning.clone().or(e.meaning_en.clone()).unwrap_or_default())).collect();
            }
        }
        for (m, v) in by_meaning {
            let keys: BTreeSet<String> = v.iter().filter_map(|e| e.latex.as_deref().map(math::canonical)).collect();
            if keys.len() >= 2 {
                let list: Vec<String> = keys.iter().map(|k| format!("${k}$")).collect();
                let msg = format!("對照表中「{m}」對應 {} 個符號：{}", keys.len(), list.join("、"));
                let iss = self.push("meaning_multiple_symbols", "registry", Severity::Warning, msg, Some(m.clone()), vec![]);
                iss.details = v.iter().map(|e| e.id.clone()).collect();
            }
        }
        // used in the text but not registered (only once the registry has symbols)
        if reg.entries.iter().any(|e| e.kind() == Kind::Symbol) {
            for info in self.syms.table.values() {
                if !self.reg_keys.contains_key(&info.key) && !self.conventional.contains(&info.key) {
                    let locs: Vec<Loc> = info.occs.first().map(|o| self.occ_loc(o.off, o.area, o.display)).into_iter().collect();
                    self.push("unregistered_symbol", "registry", Severity::Info, format!("符號 ${}$ 尚未登記在對照表", info.key), Some(info.key.clone()), locs);
                }
            }
        }
    }
}

/// Run every check. `notation` is the effective registry (document + global,
/// see [`Notation::effective`]); pass an empty one for text-only checks.
pub fn check(scan: &Scan, notation: &Notation, opt: &CheckOptions) -> Report {
    let syms = symbols::analyze(scan, &symbols::SymbolOptions { caption_defines: opt.caption_defines });
    check_with(scan, &syms, notation, opt)
}

pub fn check_with(scan: &Scan, syms: &Symbols, notation: &Notation, opt: &CheckOptions) -> Report {
    let mut reg_keys = BTreeMap::new();
    let mut conventional = BTreeSet::new();
    for e in &notation.entries {
        if let (Some(l), true) = (&e.latex, matches!(e.kind(), Kind::Symbol | Kind::Operator)) {
            let k = math::canonical(l);
            if e.is_conventional() {
                conventional.insert(k.clone());
            }
            reg_keys.entry(k).or_insert(e.id.clone());
        }
    }
    let mut c = Ctx { scan, syms, reg: notation, opt, reg_keys, conventional, issues: Vec::new() };
    c.symbols();
    c.equations();
    let abbr = c.abbreviations();
    c.variants();
    c.references();
    c.captions();
    c.registry(&abbr);
    let mut issues = c.issues;
    issues.sort_by_key(|i| (i.severity, i.locations.first().map(|l| l.ord).unwrap_or(usize::MAX)));
    let mut summary = BTreeMap::new();
    for i in &issues {
        *summary.entry(i.code.clone()).or_insert(0) += 1;
    }
    let symbols = syms
        .table
        .values()
        .map(|i| SymbolSummary {
            key: i.key.clone(),
            uses: i.occs.len(),
            defined: !i.defs.is_empty(),
            meaning: i.meaning(),
            first_use: i.first_use().map(|o| scan.loc(o.off, scan.context_of(scan.math[o.seg].ctx, o.display))),
            defined_at: i.first_def().map(|d| scan.loc(d.off, "definition")),
            entry: notation.find_latex(&i.key).map(|e| e.id.clone()),
        })
        .collect();
    let abbreviations = abbr.values().map(|a| AbbrevSummary { short: a.short.clone(), uses: a.uses.len(), long: a.expansions.first().map(|e| e.long.clone()) }).collect();
    Report { version: 1, files: scan.doc.files.clone(), issues, summary, symbols, abbreviations }
}
