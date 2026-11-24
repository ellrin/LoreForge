//! What applies to a step and its content, written into the prompt (docs/brain-hub.md §7).
//!
//! A category records the steps (lore-llm task ids) that use it; an item may carry `triggers`.
//! When a step runs it asks [`select`] which of its categories' items apply to the content,
//! [`render`]s them in their format's shape and records [`used_ids`]. Everything here is pure:
//! no I/O and no clock, so a result depends only on its arguments.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::types::{Category, Format, Item};

// ── step 1: matching ─────────────────────────────────────────────────────────

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x2FDF | 0x3000..=0x303F | 0x3040..=0x30FF | 0x3100..=0x312F | 0x31A0..=0x31FF
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF
        | 0x20000..=0x2FA1F)
}

/// Occurrences of `trigger` in `text` (not overlapping), case-insensitive.
///
/// * Latin letters and digits: whole words, plus the simple plural (`+s`): `timeline` matches
///   `timelines` but not `timelined`.
/// * A trigger with a CJK character: a substring.
/// * Several words: any run of whitespace between them.
/// * An empty (or blank) trigger never matches.
pub fn hits(text: &str, trigger: &str) -> usize {
    let tokens: Vec<Vec<char>> = trigger.split_whitespace().map(|t| t.to_lowercase().chars().collect()).collect();
    if tokens.is_empty() {
        return 0;
    }
    let cjk = tokens.iter().flatten().any(|c| is_cjk(*c));
    let first_alnum = tokens[0][0].is_alphanumeric();
    let last_alnum = tokens.last().and_then(|t| t.last()).is_some_and(|c| c.is_alphanumeric());
    let text: Vec<char> = text.to_lowercase().chars().collect();

    // Where a match of all tokens starting at `i` ends (exclusive).
    let match_at = |i: usize| -> Option<usize> {
        let mut p = i;
        for (k, tok) in tokens.iter().enumerate() {
            if k > 0 {
                let start = p;
                while p < text.len() && text[p].is_whitespace() {
                    p += 1;
                }
                if p == start {
                    return None;
                }
            }
            if text.len() < p + tok.len() || text[p..p + tok.len()] != tok[..] {
                return None;
            }
            p += tok.len();
        }
        Some(p)
    };

    let mut n = 0;
    let mut i = 0;
    while i < text.len() {
        if !cjk && first_alnum && i > 0 && text[i - 1].is_alphanumeric() {
            i += 1;
            continue;
        }
        let Some(mut end) = match_at(i) else {
            i += 1;
            continue;
        };
        if !cjk && last_alnum {
            let boundary = |e: usize| e >= text.len() || !text[e].is_alphanumeric();
            if !boundary(end) {
                // the simple plural: one `s`, then a boundary
                if text.get(end) == Some(&'s') && boundary(end + 1) {
                    end += 1;
                } else {
                    i += 1;
                    continue;
                }
            }
        }
        n += 1;
        i = end.max(i + 1);
    }
    n
}

// ── step 2: selection ────────────────────────────────────────────────────────

/// What a step asks for.
#[derive(Debug, Clone, Default)]
pub struct Ask<'a> {
    /// The lore-llm task id of the step.
    pub task: &'a str,
    /// The content the step works on (what triggers are looked for in).
    pub text: &'a str,
    /// At most this many items per category; 0 = no limit.
    pub per_category: usize,
    /// Facet → any of these tags. Empty = no tag filter.
    pub tags: BTreeMap<String, Vec<String>>,
}

/// An item chosen for a step, and why.
#[derive(Debug, Clone)]
pub struct Picked<'a> {
    pub item: &'a Item,
    /// Total trigger hits; 0 for an item without triggers.
    pub score: usize,
    /// The triggers that hit.
    pub why: Vec<String>,
}

fn tags_match(item: &Item, want: &BTreeMap<String, Vec<String>>) -> bool {
    want.iter().filter(|(_, v)| !v.is_empty()).all(|(facet, any)| {
        item.tags.get(facet).is_some_and(|have| have.iter().any(|h| any.iter().any(|w| h.trim().eq_ignore_ascii_case(w.trim()))))
    })
}

/// The items that apply to `ask`, grouped by category (in the order of `cats`).
///
/// Rules: only categories whose `tasks` contain `ask.task` and that are not independent; a
/// reference figure (`entity == false`) is never selected; an item without triggers always
/// applies, one with triggers only when at least one hits; the tag filter keeps items that have
/// any of the asked tags in every asked facet. Order: triggered items by score (then `used`,
/// then title), then the untriggered by `used` (descending), then title; `per_category` cuts that
/// list. A category with nothing picked is left out.
pub fn select<'a>(cats: &[Category], items: &'a [Item], ask: &Ask) -> Vec<(Category, Vec<Picked<'a>>)> {
    let mut out = vec![];
    for cat in cats {
        if cat.independent() || !cat.tasks.iter().any(|t| t == ask.task) {
            continue;
        }
        let mut picked: Vec<Picked<'a>> = vec![];
        for item in items.iter().filter(|i| i.category == cat.id) {
            if !item.entity || !tags_match(item, &ask.tags) {
                continue;
            }
            let mut why = vec![];
            let mut score = 0;
            for t in &item.triggers {
                let h = hits(ask.text, t);
                if h > 0 {
                    score += h;
                    why.push(t.trim().to_string());
                }
            }
            let has_triggers = item.triggers.iter().any(|t| !t.trim().is_empty());
            if has_triggers && score == 0 {
                continue;
            }
            picked.push(Picked { item, score, why });
        }
        picked.sort_by(|a, b| {
            b.score.cmp(&a.score).then(b.item.used.cmp(&a.item.used)).then_with(|| a.item.title.cmp(&b.item.title)).then_with(|| a.item.id.cmp(&b.item.id))
        });
        if ask.per_category > 0 {
            picked.truncate(ask.per_category);
        }
        if !picked.is_empty() {
            out.push((cat.clone(), picked));
        }
    }
    out
}

// ── step 3: into the prompt ──────────────────────────────────────────────────

/// The formats by id.
pub type Formats = BTreeMap<String, Box<dyn Format>>;

fn heading(cat: &Category) -> &str {
    if cat.label.trim().is_empty() { &cat.id } else { cat.label.trim() }
}

/// The prompt text: per category a `### <label>` heading, then each item's [`Format::prompt`]
/// on its own entry (one per line group, no blank line between items); blocks are separated by
/// a blank line. A category whose format is not in `formats` is skipped ([`unknown_formats`]);
/// nothing picked gives "" so a prompt without know-how stays unchanged.
pub fn render(picked: &[(Category, Vec<Picked>)], formats: &Formats) -> String {
    let mut blocks = vec![];
    for (cat, list) in picked {
        let Some(f) = formats.get(&cat.format) else { continue };
        let entries: Vec<String> = list.iter().map(|p| f.prompt(p.item).trim_end().to_string()).filter(|s| !s.is_empty()).collect();
        if entries.is_empty() {
            continue;
        }
        blocks.push(format!("### {}\n{}", heading(cat), entries.join("\n")));
    }
    blocks.join("\n\n")
}

/// Format ids used by picked categories that `formats` lacks (sorted, once each).
pub fn unknown_formats(picked: &[(Category, Vec<Picked>)], formats: &Formats) -> Vec<String> {
    let mut v: Vec<String> = picked.iter().map(|(c, _)| c.format.clone()).filter(|f| !formats.contains_key(f)).collect();
    v.sort();
    v.dedup();
    v
}

/// Ids of the picked items, each once, in order. Pass only what was actually rendered when some
/// formats were unknown (see [`without_unknown`]).
pub fn used_ids(picked: &[(Category, Vec<Picked>)]) -> Vec<String> {
    let mut seen = vec![];
    for (_, list) in picked {
        for p in list {
            if !seen.contains(&p.item.id) {
                seen.push(p.item.id.clone());
            }
        }
    }
    seen
}

/// `picked` without the categories whose format `render` would skip.
pub fn without_unknown<'a>(picked: &[(Category, Vec<Picked<'a>>)], formats: &Formats) -> Vec<(Category, Vec<Picked<'a>>)> {
    picked.iter().filter(|(c, _)| formats.contains_key(&c.format)).cloned().collect()
}

// ── step 4: the report ───────────────────────────────────────────────────────

/// One line of what a job shows the author: what was used and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Used {
    /// The category's label.
    pub category: String,
    pub title: String,
    pub id: String,
    /// The triggers that hit (empty: applied without triggers).
    pub why: Vec<String>,
    /// "" for the author or Lore, else the outside application's name.
    pub source: String,
}

pub fn report(picked: &[(Category, Vec<Picked>)]) -> Vec<Used> {
    picked
        .iter()
        .flat_map(|(cat, list)| {
            list.iter().map(move |p| Used {
                category: heading(cat).to_string(),
                title: p.item.title.clone(),
                id: p.item.id.clone(),
                why: p.why.clone(),
                source: if p.item.source.is_external() { p.item.source.name.clone() } else { String::new() },
            })
        })
        .collect()
}

// ── step 5: wiring guard ─────────────────────────────────────────────────────

/// Lore and manual categories no step uses (未接上). Independent sets are not listed.
pub fn unconnected(cats: &[Category]) -> Vec<&Category> {
    cats.iter().filter(|c| !c.independent() && !c.connected()).collect()
}

/// Task id → ids of the (non-independent) categories it reads, in the order given.
pub fn tasks_of(cats: &[Category]) -> BTreeMap<String, Vec<String>> {
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in cats.iter().filter(|c| !c.independent()) {
        for t in &c.tasks {
            let v = m.entry(t.clone()).or_default();
            if !v.contains(&c.id) {
                v.push(c.id.clone());
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{format, FormatDef, Source};

    // ── step 1 ──
    #[test]
    fn latin_triggers_match_whole_words_and_plurals() {
        assert_eq!(hits("A Timeline, two timelines; TIMELINE.", "timeline"), 3);
        assert_eq!(hits("timelined and subtimeline", "timeline"), 0);
        assert_eq!(hits("a model2 and model", "model"), 1);
        assert_eq!(hits("anything", ""), 0);
        assert_eq!(hits("anything", "   "), 0);
        assert_eq!(hits("x", "timeline"), 0);
    }

    #[test]
    fn cjk_triggers_match_as_substrings() {
        assert_eq!(hits("這是時間軸圖與時間軸說明", "時間軸"), 2);
        assert_eq!(hits("a時間軸b", "時間軸"), 1);
        assert_eq!(hits("沒有", "時間軸"), 0);
    }

    #[test]
    fn multi_word_triggers_ignore_spacing_and_case() {
        assert_eq!(hits("The data   Flow\n diagram, a data flow", "data flow"), 2);
        assert_eq!(hits("dataflow", "data flow"), 0);
        assert_eq!(hits("data flows", "Data  Flow"), 1);
    }

    // ── fakes ──
    fn cat(id: &str, fmt: &str, origin: &str, tasks: &[&str]) -> Category {
        Category { id: id.into(), label: format!("L {id}"), format: fmt.into(), origin: origin.into(), tasks: tasks.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }
    fn item(cat: &str, local: &str, title: &str, triggers: &[&str]) -> Item {
        let mut i = Item::new(cat, format::RULE, title, Source::author());
        i.id = format!("{cat}:{local}");
        i.triggers = triggers.iter().map(|s| s.to_string()).collect();
        i
    }
    fn ask<'a>(task: &'a str, text: &'a str) -> Ask<'a> {
        Ask { task, text, ..Default::default() }
    }

    struct Fake;
    impl Format for Fake {
        fn def(&self) -> FormatDef {
            FormatDef { id: "rule".into(), ..Default::default() }
        }
        fn line(&self, item: &Item) -> String {
            item.title.clone()
        }
        fn prompt(&self, item: &Item) -> String {
            format!("- {}", item.title)
        }
    }
    fn formats() -> Formats {
        let mut f = Formats::new();
        f.insert("rule".into(), Box::new(Fake));
        f
    }

    // ── step 2 ──
    #[test]
    fn only_the_steps_categories_are_read() {
        let cats = [cat("a.x", "rule", "lore", &["t1"]), cat("a.y", "rule", "lore", &["t2"]), cat("a.z", "rule", "manual", &[])];
        let items = [item("a.x", "1", "X", &[]), item("a.y", "1", "Y", &[]), item("a.z", "1", "Z", &[])];
        let r = select(&cats, &items, &ask("t1", ""));
        assert_eq!(used_ids(&r), ["a.x:1"]);
        assert!(select(&cats, &items, &ask("other", "")).is_empty());
    }

    #[test]
    fn an_independent_set_is_never_selected() {
        let cats = [cat("x.app.words", "rule", "external", &["t1"])];
        let items = [item("x.app.words", "1", "W", &[])];
        assert!(select(&cats, &items, &ask("t1", "")).is_empty());
    }

    #[test]
    fn aligned_external_items_are_selected_with_the_categorys_own() {
        let cats = [cat("my.rules", "rule", "manual", &["t1"])];
        let own = item("my.rules", "1", "Own", &[]);
        let mut ext = item("my.rules", "2", "Aligned", &[]);
        ext.source = Source::external("SomeApp");
        let items = [own, ext];
        let r = select(&cats, &items, &ask("t1", ""));
        assert_eq!(r.len(), 1);
        assert_eq!(used_ids(&r), ["my.rules:2", "my.rules:1"]); // by title: Aligned, Own
    }

    #[test]
    fn items_with_triggers_need_a_hit_and_come_first() {
        let cats = [cat("a.x", "rule", "lore", &["t1"])];
        let mut plain = item("a.x", "p", "Plain", &[]);
        plain.used = 9;
        let one = item("a.x", "1", "One hit", &["timeline"]);
        let two = item("a.x", "2", "Two hits", &["timeline", "網路"]);
        let none = item("a.x", "3", "No hit", &["flowchart"]);
        let items = [plain, one, two, none];
        let r = select(&cats, &items, &ask("t1", "A timeline of a 網路, timelines."));
        assert_eq!(used_ids(&r), ["a.x:2", "a.x:1", "a.x:p"]);
        assert_eq!(r[0].1[0].score, 3);
        assert_eq!(r[0].1[0].why, ["timeline", "網路"]);
        assert_eq!(r[0].1[2].score, 0);
        // without triggers: `used` breaks ties, then the title
        let mut a = item("a.x", "a", "B", &[]);
        a.used = 1;
        let b = item("a.x", "b", "A", &[]);
        let c = item("a.x", "c", "C", &[]);
        let items = [b, c, a];
        assert_eq!(used_ids(&select(&cats, &items, &ask("t1", ""))), ["a.x:a", "a.x:b", "a.x:c"]);
    }

    #[test]
    fn references_are_never_selected() {
        let cats = [cat("a.fig", "figure", "lore", &["t1"])];
        let mut r = item("a.fig", "1", "Reference", &[]);
        r.entity = false;
        let e = item("a.fig", "2", "Entity", &[]);
        let items = [r, e];
        assert_eq!(used_ids(&select(&cats, &items, &ask("t1", ""))), ["a.fig:2"]);
    }

    #[test]
    fn the_tag_filter_and_the_cut_apply() {
        let cats = [cat("a.x", "rule", "lore", &["t1"])];
        let mut a = item("a.x", "a", "A", &[]);
        a.tag("use", "paper").tag("kind", "timeline");
        let mut b = item("a.x", "b", "B", &[]);
        b.tag("use", "slide");
        let mut c = item("a.x", "c", "C", &[]);
        c.tag("use", "paper");
        let items = [a, b, c];
        let mut q = ask("t1", "");
        q.tags.insert("use".into(), vec!["paper".into(), "book".into()]);
        assert_eq!(used_ids(&select(&cats, &items, &q)), ["a.x:a", "a.x:c"]);
        q.tags.insert("kind".into(), vec!["timeline".into()]);
        assert_eq!(used_ids(&select(&cats, &items, &q)), ["a.x:a"]);
        q.tags.clear();
        q.per_category = 2;
        assert_eq!(used_ids(&select(&cats, &items, &q)), ["a.x:a", "a.x:b"]);
        q.tags.insert("use".into(), vec!["nothing".into()]);
        assert!(select(&cats, &items, &q).is_empty());
    }

    // ── step 3 ──
    #[test]
    fn blocks_are_headed_by_the_category() {
        let cats = [cat("a.x", "rule", "lore", &["t1"]), cat("a.y", "rule", "lore", &["t1"]), cat("a.u", "mystery", "lore", &["t1"])];
        let items = [item("a.x", "1", "One", &[]), item("a.x", "2", "Two", &[]), item("a.y", "1", "Three", &[]), item("a.u", "1", "Lost", &[])];
        let r = select(&cats, &items, &ask("t1", ""));
        assert_eq!(render(&r, &formats()), "### L a.x\n- One\n- Two\n\n### L a.y\n- Three");
        assert_eq!(unknown_formats(&r, &formats()), ["mystery"]);
        assert_eq!(used_ids(&without_unknown(&r, &formats())), ["a.x:1", "a.x:2", "a.y:1"]);
    }

    #[test]
    fn nothing_picked_renders_nothing() {
        assert_eq!(render(&[], &formats()), "");
        assert!(unknown_formats(&[], &formats()).is_empty());
        assert!(used_ids(&[]).is_empty());
    }

    #[test]
    fn used_ids_list_every_item_once() {
        let c = cat("a.x", "rule", "lore", &["t1"]);
        let i = item("a.x", "1", "One", &[]);
        let j = item("a.x", "2", "Two", &[]);
        let picked = vec![
            (c.clone(), vec![Picked { item: &i, score: 0, why: vec![] }, Picked { item: &j, score: 0, why: vec![] }]),
            (c, vec![Picked { item: &i, score: 0, why: vec![] }]),
        ];
        assert_eq!(used_ids(&picked), ["a.x:1", "a.x:2"]);
    }

    // ── step 4 ──
    #[test]
    fn the_report_names_what_was_used_and_why() {
        let cats = [cat("my.rules", "rule", "manual", &["t1"])];
        let own = item("my.rules", "1", "Own", &["timeline"]);
        let mut ext = item("my.rules", "2", "Aligned", &[]);
        ext.source = Source::external("SomeApp");
        let items = [own, ext];
        let r = select(&cats, &items, &ask("t1", "a timeline"));
        let rep = report(&r);
        assert_eq!(rep.len(), 2);
        assert_eq!(rep[0], Used { category: "L my.rules".into(), title: "Own".into(), id: "my.rules:1".into(), why: vec!["timeline".into()], source: String::new() });
        assert_eq!((rep[1].source.as_str(), rep[1].why.len()), ("SomeApp", 0));
        assert!(serde_json::to_string(&rep).unwrap().contains("\"why\":[\"timeline\"]"));
    }

    // ── step 5 ──
    #[test]
    fn unconnected_categories_are_found() {
        let cats = [cat("a.x", "rule", "lore", &["t1", "t2"]), cat("a.y", "rule", "lore", &[]), cat("my.z", "rule", "manual", &[]), cat("x.s.w", "rule", "external", &[]), cat("my.w", "rule", "manual", &["t1"])];
        let un: Vec<&str> = unconnected(&cats).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(un, ["a.y", "my.z"]);
        let m = tasks_of(&cats);
        assert_eq!(m["t1"], ["a.x", "my.w"]);
        assert_eq!(m["t2"], ["a.x"]);
        assert_eq!(m.len(), 2);
    }
}
