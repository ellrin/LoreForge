//! The knowledge hub's items that outside applications aligned to L-Paper's writing categories
//! (`pc.patterns`, `pc.expressions`, `pc.wording`, `pc.exemplars`; docs/brain-hub.md), turned
//! into the profile's own row types and appended after the author's own rows. One merge point:
//! the server calls [`merge`] when it loads the profile (`pc-server::know`).
//!
//! `pc-core` does not depend on `lore-know`: the server hands over plain [`Row`]s (an item's
//! columns as text). What this file never does:
//! - touch the author's rows, their order or their priority (outside rows come last);
//! - merge `pc.rules`: an outside application must not change what L-Paper's checks enforce;
//! - let an outside row into the CSV files (ids start with [`PREFIX`], the table writers refuse them)
//!   or into the derived 我的風格 (`styleprofile::own` drops them);
//! - guess a missing column: such an item is skipped and counted.

use std::collections::{BTreeMap, BTreeSet};

use crate::profile::{Expression, Phrase, Profile, VocabEntry, POSITIONS};
use crate::styleprofile::Exemplar;

/// Every outside row's id starts with this (`x:<source>:<the item's own id>`); the author's ids
/// (`P017`, `X003`, `E1` …) never do.
pub const PREFIX: &str = "x:";

/// The id of an outside row.
pub fn outside_id(source: &str, local: &str) -> String {
    format!("{PREFIX}{}:{}", clean(source), local.trim())
}

/// Whether an id (a phrase id, a table cell) belongs to an outside row.
pub fn is_outside_id(id: &str) -> bool {
    id.trim_start().starts_with(PREFIX)
}

/// A source's name as one id segment.
fn clean(s: &str) -> String {
    let t: String = s.trim().chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' }).collect();
    if t.is_empty() { "source".into() } else { t }
}

/// One outside item as the columns of the matching CSV (`lore-know`'s `formats/table.rs` uses the
/// same names): the plain input of this module.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    /// The outside application's name.
    pub source: String,
    /// The item's own id inside its category (the part after the last `:`).
    pub local: String,
    pub fields: BTreeMap<String, String>,
}

impl Row {
    fn get(&self, k: &str) -> &str {
        self.fields.get(k).map(|s| s.trim()).unwrap_or("")
    }
}

/// What the hub holds for the four categories.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Aligned {
    /// `pc.patterns` → phrases.
    pub patterns: Vec<Row>,
    /// `pc.expressions`.
    pub expressions: Vec<Row>,
    /// `pc.wording` → vocabulary.
    pub wording: Vec<Row>,
    /// `pc.exemplars`.
    pub exemplars: Vec<Row>,
}

impl Aligned {
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty() && self.expressions.is_empty() && self.wording.is_empty() && self.exemplars.is_empty()
    }
}

/// The profile's row types made from outside rows, and how many rows were left out.
#[derive(Debug, Clone, Default)]
pub struct Converted {
    pub phrases: Vec<Phrase>,
    pub expressions: Vec<Expression>,
    pub vocabulary: Vec<VocabEntry>,
    pub exemplars: Vec<Exemplar>,
    /// Rows lacking a required column (or a second row with an id already taken).
    pub skipped: usize,
}

/// The `priority` of a pattern row when it carries a valid one (1 to 5).
fn priority_of(r: &Row) -> Option<u8> {
    r.get("priority").parse::<u8>().ok().filter(|p| (1..=5).contains(p))
}

/// Convert rows into the profile's own types. A phrase needs what the author's own phrase rows
/// need (`profile::validate_phrase`): section, function, a valid position and a template; an
/// expression its text; a wording row its `avoid`; an exemplar its text. Anything else is skipped
/// and counted, never filled in. `default_priority` is for a pattern that carries none.
pub fn convert(a: &Aligned, default_priority: u8) -> Converted {
    let mut c = Converted::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut take = |c: &mut Converted, r: &Row| -> Option<String> {
        if r.local.trim().is_empty() {
            c.skipped += 1;
            return None;
        }
        let id = outside_id(&r.source, &r.local);
        if !seen.insert(id.clone()) {
            c.skipped += 1;
            return None;
        }
        Some(id)
    };
    for r in &a.patterns {
        let ok = ["section", "function", "template"].iter().all(|k| !r.get(k).is_empty()) && POSITIONS.contains(&r.get("position")) && r.get("template").matches('{').count() == r.get("template").matches('}').count();
        let Some(id) = ok.then(|| take(&mut c, r)).flatten() else {
            if !ok {
                c.skipped += 1;
            }
            continue;
        };
        c.phrases.push(Phrase {
            id,
            section: r.get("section").into(),
            function: r.get("function").into(),
            position: r.get("position").into(),
            template: r.get("template").into(),
            example: r.get("example").into(),
            tags: vec![],
            priority: priority_of(r).unwrap_or(default_priority),
        });
    }
    for r in &a.expressions {
        let ok = !r.get("text").is_empty();
        let Some(id) = ok.then(|| take(&mut c, r)).flatten() else {
            if !ok {
                c.skipped += 1;
            }
            continue;
        };
        c.expressions.push(Expression {
            id,
            kind: r.get("kind").into(),
            text: r.get("text").into(),
            usage: r.get("usage").into(),
            section: r.get("section").into(),
            example: r.get("example").into(),
            tags: vec![],
        });
    }
    for r in &a.wording {
        if r.get("avoid").is_empty() {
            c.skipped += 1;
            continue;
        }
        c.vocabulary.push(VocabEntry { avoid: r.get("avoid").into(), prefer: r.get("prefer").into(), note: r.get("note").into(), source: clean(&r.source) });
    }
    for r in &a.exemplars {
        let ok = !r.get("text").is_empty();
        let Some(id) = ok.then(|| take(&mut c, r)).flatten() else {
            if !ok {
                c.skipped += 1;
            }
            continue;
        };
        c.exemplars.push(Exemplar { id, section: r.get("section").into(), function: r.get("function").into(), text: r.get("text").into(), source: r.get("source").into() });
    }
    c
}

/// What [`merge`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeReport {
    pub added_phrases: usize,
    pub added_expressions: usize,
    pub added_wording: usize,
    pub added_exemplars: usize,
    /// Rows left out for lacking a column.
    pub skipped: usize,
    /// Outside wording whose `avoid` the author already has (the author's row stays): the words.
    pub wording_kept_authors: Vec<String>,
    /// The outside applications that gave at least one row now in the profile, sorted.
    pub sources: Vec<String>,
}

impl MergeReport {
    pub fn added(&self) -> usize {
        self.added_phrases + self.added_expressions + self.added_wording + self.added_exemplars
    }
}

/// Append outside rows after the author's own. Order and priority of the author's rows are
/// untouched; outside patterns without a priority of their own take the lowest priority already
/// in the bank (3, the loader's default, for an empty bank). A wording row whose `avoid` the
/// author has (any case) is not added and is reported; `pc.rules` is never merged.
pub fn merge(profile: &mut Profile, aligned: Aligned) -> MergeReport {
    let mut rep = MergeReport::default();
    if aligned.is_empty() {
        return rep;
    }
    let lowest = profile.phrases.iter().map(|p| p.priority).min().unwrap_or(3);
    let c = convert(&aligned, lowest);
    rep.skipped = c.skipped;
    let mut sources: BTreeSet<String> = BTreeSet::new();
    let have_ids = |profile: &Profile| -> BTreeSet<String> { profile.phrases.iter().map(|p| p.id.clone()).chain(profile.expressions.iter().map(|e| e.id.clone())).collect() };
    let ids = have_ids(profile);
    let src_of = |id: &str| id.strip_prefix(PREFIX).and_then(|r| r.split(':').next()).unwrap_or("").to_string();
    for p in c.phrases {
        if ids.contains(&p.id) {
            rep.skipped += 1;
            continue;
        }
        sources.insert(src_of(&p.id));
        profile.phrases.push(p);
        rep.added_phrases += 1;
    }
    for e in c.expressions {
        if ids.contains(&e.id) {
            rep.skipped += 1;
            continue;
        }
        sources.insert(src_of(&e.id));
        profile.expressions.push(e);
        rep.added_expressions += 1;
    }
    let mut avoids: BTreeSet<String> = profile.vocabulary.iter().map(|v| v.avoid.trim().to_lowercase()).collect();
    for v in c.vocabulary {
        if !avoids.insert(v.avoid.trim().to_lowercase()) {
            rep.wording_kept_authors.push(v.avoid.clone());
            continue;
        }
        sources.insert(v.source.clone());
        profile.vocabulary.push(v);
        rep.added_wording += 1;
    }
    let mut have_ex: BTreeSet<String> = profile.outside_exemplars.iter().map(|e| e.id.clone()).collect();
    for e in c.exemplars {
        if !have_ex.insert(e.id.clone()) {
            rep.skipped += 1;
            continue;
        }
        sources.insert(src_of(&e.id));
        profile.outside_exemplars.push(e);
        rep.added_exemplars += 1;
    }
    rep.sources = sources.into_iter().collect();
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Manifest, Rule, RuleKind};

    fn row(source: &str, local: &str, f: &[(&str, &str)]) -> Row {
        Row { source: source.into(), local: local.into(), fields: f.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn pattern(source: &str, local: &str, template: &str) -> Row {
        row(source, local, &[("section", "Introduction"), ("function", "gap"), ("position", "opening"), ("template", template), ("example", "e.g. text")])
    }

    fn author() -> Profile {
        let ph = |id: &str, pr: u8| Phrase { id: id.into(), section: "Result".into(), function: "finding".into(), position: "body".into(), template: format!("{{x}} {id}."), example: String::new(), tags: vec!["paper1".into()], priority: pr };
        Profile {
            root: Default::default(),
            manifest: Manifest { name: "t".into(), schema_version: 1, language: "en".into() },
            phrases: vec![ph("P1", 5), ph("P2", 2)],
            expressions: vec![Expression { id: "X1".into(), kind: "connective".into(), text: "Moreover,".into(), usage: String::new(), section: String::new(), example: String::new(), tags: vec![] }],
            vocabulary: vec![VocabEntry { avoid: "utilize".into(), prefer: "use".into(), note: String::new(), source: String::new() }],
            rules: vec![Rule { id: "R1".into(), kind: RuleKind::Hard, scope: "paragraph".into(), check: "max_commas".into(), threshold: "3".into(), severity: "warning".into(), message: "m".into() }],
            outside_exemplars: vec![],
            authors: vec![],
            affiliations: vec![],
            variants: Vec::new(),
            style_checks: Default::default(),
        }
    }

    fn snapshot(p: &Profile) -> String {
        serde_json::to_string(p).unwrap() + &format!("{:?}{:?}", p.vocabulary, p.outside_exemplars)
    }

    #[test]
    fn items_become_rows_with_the_same_columns() {
        let a = Aligned {
            patterns: vec![{
                let mut r = pattern("wordbox", "7", "Little is known about {x}.");
                r.fields.insert("priority".into(), "4".into());
                r
            }],
            expressions: vec![row("wordbox", "e1", &[("kind", "connective"), ("text", "Hence,"), ("usage", "result"), ("section", "Any"), ("example", "Hence, it works.")])],
            wording: vec![row("wordbox", "w", &[("avoid", "very"), ("prefer", "markedly"), ("note", "n")])],
            exemplars: vec![row("wordbox", "x1", &[("section", "Result"), ("function", "finding"), ("text", "Recall rose."), ("source", "Someone (2020)")])],
        };
        let c = convert(&a, 1);
        assert_eq!(c.skipped, 0);
        let p = &c.phrases[0];
        assert_eq!((p.section.as_str(), p.function.as_str(), p.position.as_str(), p.template.as_str(), p.example.as_str(), p.priority), ("Introduction", "gap", "opening", "Little is known about {x}.", "e.g. text", 4));
        let e = &c.expressions[0];
        assert_eq!((e.kind.as_str(), e.text.as_str(), e.usage.as_str(), e.section.as_str(), e.example.as_str()), ("connective", "Hence,", "result", "Any", "Hence, it works."));
        let v = &c.vocabulary[0];
        assert_eq!((v.avoid.as_str(), v.prefer.as_str(), v.note.as_str(), v.source.as_str()), ("very", "markedly", "n", "wordbox"));
        let x = &c.exemplars[0];
        assert_eq!((x.section.as_str(), x.function.as_str(), x.text.as_str(), x.source.as_str()), ("Result", "finding", "Recall rose.", "Someone (2020)"));
        // no priority on the row: the one given
        let c = convert(&Aligned { patterns: vec![pattern("wordbox", "8", "A {b}.")], ..Default::default() }, 2);
        assert_eq!(c.phrases[0].priority, 2);
    }

    #[test]
    fn an_item_missing_a_column_is_skipped_and_counted() {
        let a = Aligned {
            patterns: vec![
                row("wordbox", "1", &[("template", "Only a template.")]),
                row("wordbox", "2", &[("section", "Introduction"), ("function", "gap"), ("position", "middle"), ("template", "Bad position.")]),
                pattern("wordbox", "3", "Fine {x}."),
                pattern("wordbox", "", "No local id."),
            ],
            expressions: vec![row("wordbox", "e", &[("kind", "phrase")])],
            wording: vec![row("wordbox", "w", &[("prefer", "x")])],
            exemplars: vec![row("wordbox", "x", &[("section", "Result")])],
        };
        let c = convert(&a, 3);
        assert_eq!(c.skipped, 6);
        assert_eq!((c.phrases.len(), c.expressions.len(), c.vocabulary.len(), c.exemplars.len()), (1, 0, 0, 0));
        let mut p = author();
        let rep = merge(&mut p, a);
        assert_eq!((rep.skipped, rep.added_phrases, rep.added()), (6, 1, 1));
    }

    #[test]
    fn outside_ids_never_collide_with_the_authors() {
        let a = Aligned { patterns: vec![pattern("wordbox", "P1", "Same local id as an author row.")], expressions: vec![row("wordbox", "X1", &[("text", "Moreover,")])], ..Default::default() };
        let mut p = author();
        let rep = merge(&mut p, a);
        assert_eq!((rep.added_phrases, rep.added_expressions), (1, 1));
        let ids: Vec<&str> = p.phrases.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["P1", "P2", "x:wordbox:P1"]);
        assert_eq!(p.expressions[1].id, "x:wordbox:X1");
        assert!(is_outside_id("x:wordbox:P1") && !is_outside_id("P1") && !is_outside_id("X001") && !is_outside_id(""));
        // two sources with the same local id stay apart
        assert_ne!(outside_id("a", "1"), outside_id("b", "1"));
        // a duplicate of the same source and id is counted, not doubled
        let mut p = author();
        let rep = merge(&mut p, Aligned { patterns: vec![pattern("s", "1", "A {x}."), pattern("s", "1", "B {x}.")], ..Default::default() });
        assert_eq!((rep.added_phrases, rep.skipped), (1, 1));
    }

    #[test]
    fn the_authors_rows_come_first_and_unchanged() {
        let before = author();
        let mut p = author();
        let a = Aligned {
            patterns: vec![pattern("wordbox", "7", "Little is known about {x}."), {
                let mut r = pattern("wordbox", "8", "Own priority {x}.");
                r.fields.insert("priority".into(), "5".into());
                r
            }],
            expressions: vec![row("wordbox", "e", &[("text", "Hence,")])],
            wording: vec![row("wordbox", "w", &[("avoid", "very"), ("prefer", "markedly")])],
            exemplars: vec![row("wordbox", "x", &[("text", "Recall rose.")])],
        };
        let rep = merge(&mut p, a);
        assert_eq!((rep.added_phrases, rep.added_expressions, rep.added_wording, rep.added_exemplars), (2, 1, 1, 1));
        assert_eq!(rep.sources, ["wordbox"]);
        assert_eq!(&p.phrases[..2], &before.phrases[..]);
        assert_eq!(&p.expressions[..1], &before.expressions[..]);
        assert_eq!(&p.vocabulary[..1], &before.vocabulary[..]);
        assert_eq!(p.rules.len(), before.rules.len());
        // the lowest priority present (2) unless the item carries one
        assert_eq!((p.phrases[2].priority, p.phrases[3].priority), (2, 5));
        assert_eq!(p.vocabulary[1].source, "wordbox");
        assert_eq!(p.outside_exemplars[0].id, "x:wordbox:x");
    }

    #[test]
    fn the_authors_wording_wins_over_an_outside_one() {
        let mut p = author();
        let a = Aligned { wording: vec![row("wordbox", "1", &[("avoid", "Utilize"), ("prefer", "employ")]), row("wordbox", "2", &[("avoid", "very"), ("prefer", "markedly")])], ..Default::default() };
        let rep = merge(&mut p, a);
        assert_eq!(rep.wording_kept_authors, ["Utilize"]);
        assert_eq!(rep.added_wording, 1);
        let v: Vec<(&str, &str)> = p.vocabulary.iter().map(|v| (v.avoid.as_str(), v.prefer.as_str())).collect();
        assert_eq!(v, [("utilize", "use"), ("very", "markedly")]);
    }

    #[test]
    fn merging_nothing_changes_nothing() {
        let mut p = author();
        let before = snapshot(&p);
        let rep = merge(&mut p, Aligned::default());
        assert_eq!(rep, MergeReport::default());
        assert_eq!(snapshot(&p), before);
        // rows that all fail conversion change nothing either
        let rep = merge(&mut p, Aligned { patterns: vec![row("w", "1", &[("template", "x")])], ..Default::default() });
        assert_eq!((rep.added(), rep.skipped), (0, 1));
        assert_eq!(snapshot(&p), before);
    }
}
