//! What the dashboard shows (docs/brain-hub.md §8, the chosen cover): the whole and how it
//! splits into families, what each family holds, what was added week by week, what the steps
//! used, where the gaps are. The shapes here are the FROZEN contract between the hub, L-Brain's
//! server and its page; `Hub::stats` fills them.
//!
//! A **family** is a row of the dashboard: the categories that share a `Category::group` under
//! one app section (a category without a group is its own family). Knowledge cards are a family
//! the server adds itself (they live in kb-core, not in the hub).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::types::*;

/// A named count inside a family (`句型 440`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Part {
    /// The category id to open (or "" when the part is not a category of its own, e.g. a level).
    pub category: String,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Family {
    /// Stable key: `<app>/<group>` or the category id.
    pub id: String,
    pub label: String,
    /// The dashboard section: a Lore app id, `my`, `x`, `kb`.
    pub app: String,
    /// The category ids in it, in registry order (the first is what the row opens).
    pub categories: Vec<String>,
    pub count: usize,
    pub parts: Vec<Part>,
    /// Items created in each of the last 12 weeks (Monday to Sunday), oldest first; the last is this week.
    pub weeks: Vec<usize>,
    /// Created in the last 30 days.
    pub added_30d: usize,
    /// Items a step used in the last 30 days (distinct items).
    pub used_30d: usize,
    /// One short line when something is missing (Traditional Chinese), else "": references still
    /// to make, no step uses it, not shared with Lore.
    pub gap: String,
    /// An outside application's independent set.
    pub independent: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepUse {
    /// lore-llm task id.
    pub task: String,
    /// Distinct items it used in the last 30 days.
    pub items: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub total: usize,
    pub families: Vec<Family>,
    /// The Monday (`YYYY-MM-DD`) of each of the 12 weeks in `Family::weeks`, oldest first.
    pub week_starts: Vec<String>,
    pub steps: Vec<StepUse>,
    /// Open proposals waiting for the author.
    pub proposals: usize,
    pub untagged: usize,
    /// Lore and hand-made categories no step uses.
    pub unconnected: Vec<String>,
    /// Outside applications and how many items each gave.
    pub sources: Vec<Part>,
}

// ── Hub::stats ───────────────────────────────────────────────────────────────

/// The day an item was created (its own local date), or `None` when `created_at` cannot be read.
fn created_day(item: &Item) -> Option<NaiveDate> {
    let s = item.created_at.trim();
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.date_naive()).or_else(|| s.get(..10).and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()))
}

/// The family a category belongs to: its key (`<app>/<group>` or the category id) and label.
fn family_key(c: &Category) -> (String, String) {
    if c.group.trim().is_empty() { (c.id.clone(), c.label.clone()) } else { (format!("{}/{}", c.app, c.group), c.group.clone()) }
}

/// The Mondays of the 12 weeks ending with the week of `today`, oldest first.
fn week_mondays(today: NaiveDate) -> Vec<NaiveDate> {
    let this = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    (0..12).map(|i| this - Duration::days(7 * (11 - i))).collect()
}

/// Which of the 12 weeks `day` falls in (`None` before the first or after this week).
fn week_of(day: NaiveDate, mondays: &[NaiveDate]) -> Option<usize> {
    let n = (day - mondays[0]).num_days();
    (0..84).contains(&n).then_some((n / 7) as usize)
}

impl crate::hub::Hub {
    /// The dashboard's figures as of `today`: families in registry order with their parts, the
    /// weekly additions, the 30-day use from `<hub>/usage.jsonl`, and the gaps.
    pub fn stats(&self, today: NaiveDate) -> crate::Result<Stats> {
        let reg = self.store.registry();
        let mondays = week_mondays(today);
        let since = today - Duration::days(29); // the last 30 days, today included
        let usage = crate::reader::usage(self.store.hub(), since);

        let mut items: BTreeMap<String, Vec<Item>> = BTreeMap::new();
        for c in reg.all() {
            items.insert(c.id.clone(), self.store.list(&c.id)?);
        }

        // families in registry order
        let mut order: Vec<String> = vec![];
        let mut members: BTreeMap<String, (String, Vec<&Category>)> = BTreeMap::new();
        for c in reg.all() {
            let (key, label) = family_key(c);
            if !members.contains_key(&key) {
                order.push(key.clone());
            }
            members.entry(key).or_insert((label, vec![])).1.push(c);
        }

        let mut families = vec![];
        for key in &order {
            let (label, cats) = &members[key];
            let first = cats[0];
            let all: Vec<&Item> = cats.iter().flat_map(|c| items[&c.id].iter()).collect();
            let has_figure = cats.iter().any(|c| c.format == format::FIGURE);
            let (parts, to_make) = if has_figure {
                let figs: Vec<&Item> = cats.iter().filter(|c| c.format == format::FIGURE).flat_map(|c| items[&c.id].iter()).collect();
                let at = |lv: &str| figs.iter().filter(|i| i.entity && i.level == lv).count();
                // a reference is made once a drawable figure names it in `made_from`
                let drawable: BTreeSet<&str> = figs.iter().filter(|i| i.entity).map(|i| i.id.as_str()).collect();
                let made_from: BTreeSet<&str> = figs.iter().filter(|i| i.entity).map(|i| i.field("made_from")).filter(|m| !m.is_empty()).collect();
                // (`contains` alone says nothing: a reference lists the drawable parts it is made of
                // long before anyone has made it)
                let _ = &drawable;
                let to_make = figs.iter().filter(|i| !i.entity && !made_from.contains(i.id.as_str())).count();
                let part = |label: &str, count: usize| Part { category: first.id.clone(), label: label.into(), count };
                (
                    vec![part("流程圖", at(level::FIGURE)), part("模塊", at(level::MODULE)), part("元件", at(level::COMPONENT)), part("待製作", to_make)],
                    to_make,
                )
            } else {
                (cats.iter().map(|c| Part { category: c.id.clone(), label: c.label.clone(), count: items[&c.id].len() }).collect(), 0)
            };
            let mut weeks = vec![0usize; 12];
            let mut added_30d = 0;
            for i in &all {
                let Some(d) = created_day(i) else { continue };
                if let Some(w) = week_of(d, &mondays) {
                    weeks[w] += 1;
                }
                if d >= since && d <= today {
                    added_30d += 1;
                }
            }
            let ids: BTreeSet<&str> = cats.iter().map(|c| c.id.as_str()).collect();
            let used: BTreeSet<&str> =
                usage.iter().flat_map(|(_, _, v)| v.iter()).map(String::as_str).filter(|id| split_id(id).is_some_and(|(c, _)| ids.contains(c))).collect();
            let independent = cats.iter().any(|c| c.independent());
            let gap = if to_make > 0 {
                format!("{to_make} 個待製作")
            } else if !independent && !cats.iter().any(|c| c.connected()) {
                "沒有步驟在用".into()
            } else if independent {
                "不與 Lore 共用".into()
            } else {
                String::new()
            };
            families.push(Family {
                id: key.clone(),
                label: label.clone(),
                app: first.app.clone(),
                categories: cats.iter().map(|c| c.id.clone()).collect(),
                count: all.len(),
                parts,
                weeks,
                added_30d,
                used_30d: used.len(),
                gap,
                independent,
            });
        }

        // the steps
        let mut by_task: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (_, task, ids) in &usage {
            by_task.entry(task.as_str()).or_default().extend(ids.iter().map(String::as_str));
        }
        let mut steps: Vec<StepUse> = by_task.into_iter().map(|(t, s)| StepUse { task: t.into(), items: s.len() }).collect();
        steps.sort_by(|a, b| b.items.cmp(&a.items).then_with(|| a.task.cmp(&b.task)));

        let every: Vec<&Item> = items.values().flatten().collect();
        let mut given: BTreeMap<&str, usize> = BTreeMap::new();
        for i in every.iter().filter(|i| i.source.is_external()) {
            *given.entry(i.source.name.as_str()).or_default() += 1;
        }
        let mut sources: Vec<Part> = given.into_iter().map(|(n, c)| Part { category: String::new(), label: n.into(), count: c }).collect();
        sources.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));

        Ok(Stats {
            total: every.len(),
            families,
            week_starts: mondays.iter().map(|d| d.format("%Y-%m-%d").to_string()).collect(),
            steps,
            proposals: self.proposals.count_open(),
            untagged: every.iter().filter(|i| i.all_tags().is_empty()).count(),
            unconnected: reg.all().iter().filter(|c| c.origin != "external" && !c.connected()).map(|c| c.label.clone()).collect(),
            sources,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Hub;
    use crate::intake::{Batch, Entry, Target};
    use std::path::{Path, PathBuf};

    const MANIFEST: &str = r#"
[[category]]
id = "t.rules"
label = "規則"
app = "t"
group = "準則"
format = "rule"
tasks = ["draft"]
writable = true
[category.native]
kind = "csv"
app_dir = "App"
path = "rules.csv"

[[category]]
id = "t.words"
label = "用詞"
app = "t"
group = "準則"
format = "wording"
writable = true
[category.native]
kind = "csv"
app_dir = "App"
path = "words.csv"

[[category]]
id = "t.flow"
label = "流程圖"
app = "t"
group = "圖"
format = "figure"
writable = true
[category.native]
kind = "dir"
app_dir = "App"
path = "flow"

[[category]]
id = "t.refs"
label = "參考圖"
app = "t"
group = "圖"
format = "figure"
writable = true
[category.native]
kind = "dir"
app_dir = "App"
path = "refs"

[[category]]
id = "t.alone"
label = "單獨一類"
app = "t"
format = "note"
writable = true
tasks = ["polish"]
"#;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn open(hub: &Path, root: &Path) -> Hub {
        let dirs = BTreeMap::from([("App".to_string(), root.to_path_buf())]);
        Hub::open(hub.to_path_buf(), &[MANIFEST], &dirs, &[]).unwrap()
    }

    /// An item written straight as a hub record with the given creation time.
    fn record(hub: &Path, cat: &str, local: &str, created: &str, source: Source, tag: Option<&str>) {
        let mut i = Item::new(cat, if cat == "t.alone" { format::NOTE } else { format::RULE }, local, source);
        i.set("message", local).set("body", local);
        i.created_at = created.into();
        if let Some(t) = tag {
            i.tag(facet::KIND, t);
        }
        crate::util::write_atomic(&hub.join("items").join(cat).join(format!("{local}.json")), serde_json::to_string(&i).unwrap().as_bytes()).unwrap();
    }

    fn fam<'a>(s: &'a Stats, id: &str) -> &'a Family {
        s.families.iter().find(|f| f.id == id).unwrap_or_else(|| panic!("{id} in {:?}", s.families.iter().map(|f| &f.id).collect::<Vec<_>>()))
    }

    fn setup() -> (tempfile::TempDir, tempfile::TempDir) {
        let (hub, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        // the app's own data: two rules, one word, one reference
        std::fs::write(root.path().join("rules.csv"), "id,type,message,section,tags\nR1,hard,Be short.,,\nR2,soft,Be plain.,,\n").unwrap();
        std::fs::write(root.path().join("words.csv"), "avoid,prefer,note\nutilize,use,\n").unwrap();
        std::fs::create_dir_all(root.path().join("refs/f01")).unwrap();
        std::fs::write(root.path().join("refs/f01/desc.md"), "# f01 Overview\n\nTwo rows.\n").unwrap();
        std::fs::create_dir_all(root.path().join("refs/f02")).unwrap();
        std::fs::write(root.path().join("refs/f02/desc.md"), "level: module\n\n# f02 Encoder\n").unwrap();
        (hub, root)
    }

    #[test]
    fn families_group_categories_and_count_parts() {
        let (hub, root) = setup();
        let h = open(hub.path(), root.path());
        let s = h.stats(d("2026-11-24")).unwrap();
        assert_eq!(s.families.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["t/準則", "t/圖", "t.alone"]);
        let f = fam(&s, "t/準則");
        assert_eq!((f.label.as_str(), f.app.as_str(), f.count), ("準則", "t", 3));
        assert_eq!(f.categories, ["t.rules", "t.words"]);
        assert_eq!(f.parts, vec![Part { category: "t.rules".into(), label: "規則".into(), count: 2 }, Part { category: "t.words".into(), label: "用詞".into(), count: 1 }]);
        let a = fam(&s, "t.alone");
        assert_eq!((a.label.as_str(), a.count, a.parts.len()), ("單獨一類", 0, 1));
        assert_eq!(s.total, 5);
        assert_eq!(s.week_starts.len(), 12);
        assert!(s.steps.is_empty() && s.proposals == 0 && s.sources.is_empty());
    }

    #[test]
    fn figures_split_by_level_and_to_make() {
        let (hub, root) = setup();
        let h = open(hub.path(), root.path());
        let s = h.stats(d("2026-11-24")).unwrap();
        let f = fam(&s, "t/圖");
        let got: Vec<(&str, usize, &str)> = f.parts.iter().map(|p| (p.label.as_str(), p.count, p.category.as_str())).collect();
        // references are never entities: both pictures wait in 待製作, whatever their level
        assert_eq!(got, [("流程圖", 0, "t.flow"), ("模塊", 0, "t.flow"), ("元件", 0, "t.flow"), ("待製作", 2, "t.flow")]);
        assert_eq!(f.gap, "2 個待製作");
        assert_eq!(f.count, 2);
    }

    #[test]
    fn weeks_are_monday_to_sunday_ending_this_week() {
        let (hub, root) = setup();
        // 2026-11-24 is a Tuesday: this week starts Monday 11-23, the first of the 12 on 09-07
        record(hub.path(), "t.alone", "a", "2026-11-23T00:10:00+08:00", Source::author(), None);
        record(hub.path(), "t.alone", "b", "2026-11-22T23:50:00+08:00", Source::author(), None); // Sunday: last week
        record(hub.path(), "t.alone", "c", "2026-09-07T09:00:00+08:00", Source::author(), None); // first week
        record(hub.path(), "t.alone", "d", "2026-09-06T09:00:00+08:00", Source::author(), None); // before the window
        record(hub.path(), "t.alone", "e", "2026-11-30T09:00:00+08:00", Source::author(), None); // after this week
        record(hub.path(), "t.alone", "f", "not a date", Source::author(), None);
        let h = open(hub.path(), root.path());
        let s = h.stats(d("2026-11-24")).unwrap();
        assert_eq!(s.week_starts[0], "2026-09-07");
        assert_eq!(s.week_starts[11], "2026-11-23");
        let w = &fam(&s, "t.alone").weeks;
        assert_eq!(w.len(), 12);
        assert_eq!((w[0], w[10], w[11]), (1, 1, 1), "{w:?}");
        assert_eq!(w.iter().sum::<usize>(), 3, "d, e and f are in no week");
        assert_eq!(fam(&s, "t.alone").count, 6);
        assert_eq!(fam(&s, "t.alone").added_30d, 2, "a and b; c is older than 30 days, e is in the future");
        // a Sunday is the end of its own week
        let sun = h.stats(d("2026-11-29")).unwrap();
        assert_eq!(sun.week_starts[11], "2026-11-23");
    }

    #[test]
    fn use_and_gaps_are_reported() {
        let (hub, root) = setup();
        let mut h = open(hub.path(), root.path());
        let day = d("2026-11-24");
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        h.record_use_by("draft", &ids(&["t.rules:R1", "t.rules:R2"]), d("2026-11-20")).unwrap();
        h.record_use_by("draft", &ids(&["t.rules:R1"]), d("2026-11-22")).unwrap();
        h.record_use_by("polish", &ids(&["t.alone:x", "t.rules:R1"]), d("2026-11-23")).unwrap();
        h.record_use_by("old", &ids(&["t.rules:R2"]), d("2026-09-01")).unwrap();
        let s = h.stats(day).unwrap();
        assert_eq!(fam(&s, "t/準則").used_30d, 2, "R1 and R2, each once however often");
        assert_eq!(fam(&s, "t.alone").used_30d, 1);
        assert_eq!(s.steps, vec![StepUse { task: "draft".into(), items: 2 }, StepUse { task: "polish".into(), items: 2 }]);
        assert_eq!(fam(&s, "t/準則").gap, "", "a step reads it");
        assert_eq!(fam(&s, "t.alone").gap, "");
        // no category of the figure family has a step, but its gap is the references to make
        assert_eq!(fam(&s, "t/圖").gap, "2 個待製作");
        assert_eq!(s.unconnected, ["用詞", "流程圖", "參考圖"], "labels of the categories no step uses");
        // an independent set: never connected, says so
        let e = Entry { id: "1".into(), title: "x".into(), fields: serde_json::from_value(serde_json::json!({"body": "y"})).unwrap(), ..Default::default() };
        let b = Batch { lore_know: 1, source: "acme".into(), target: Target::Independent { name: "Ideas".into(), format: format::NOTE.into() }, items: vec![e] };
        let rep = h.import(&b, false).unwrap();
        let s = h.stats(day).unwrap();
        let f = fam(&s, "x/acme");
        assert!(f.independent && f.gap == "不與 Lore 共用", "{f:?}");
        assert_eq!(f.categories, [rep.category]);
        assert!(!s.unconnected.iter().any(|l| l == "Ideas"));
        // a family with no step and nothing to make is called out
        h.store.registry_mut().add_manual("Mine", format::NOTE, "", "").unwrap();
        let s = h.stats(day).unwrap();
        assert_eq!(fam(&s, "my.mine").gap, "沒有步驟在用");
    }

    #[test]
    fn sources_and_untagged_are_counted() {
        let (hub, root) = setup();
        record(hub.path(), "t.alone", "tagged", "2026-11-20T09:00:00+08:00", Source::author(), Some("timeline"));
        record(hub.path(), "t.alone", "plain", "2026-11-20T09:00:00+08:00", Source::author(), None);
        let mut h = open(hub.path(), root.path());
        let entry = |id: &str| {
            let mut e = Entry { id: id.into(), title: format!("T{id}"), ..Default::default() };
            e.fields.insert("message".into(), format!("m{id}").into());
            e
        };
        let batch = |source: &str, n: usize| Batch {
            lore_know: 1,
            source: source.into(),
            target: Target::Align { category: "t.rules".into() },
            items: (0..n).map(|i| entry(&i.to_string())).collect(),
        };
        h.import(&batch("acme", 3), false).unwrap();
        h.import(&batch("beta", 1), false).unwrap();
        let s = h.stats(d("2026-11-24")).unwrap();
        assert_eq!(s.sources, vec![Part { category: String::new(), label: "acme".into(), count: 3 }, Part { category: String::new(), label: "beta".into(), count: 1 }]);
        // untagged: 2 rules + 1 word + 2 references + 1 of the two notes + 4 aligned outside rules
        assert_eq!(s.untagged, 2 + 1 + 2 + 1 + 4);
        assert_eq!(s.total, 2 + 1 + 2 + 2 + 4);
        assert_eq!(fam(&s, "t/準則").count, 3 + 4);
        let _: PathBuf = hub.path().to_path_buf();
    }

    #[test]
    fn the_family_counts_it_as_a_figure() {
        const M: &str = "[[category]]\nid = \"t.refs\"\nlabel = \"參考圖\"\napp = \"t\"\ngroup = \"圖\"\nformat = \"figure\"\nwritable = true\n[category.native]\nkind = \"dir\"\napp_dir = \"App\"\npath = \"refs\"\n\n[[category]]\nid = \"t.specs\"\nlabel = \"做好的\"\napp = \"t\"\ngroup = \"圖\"\nformat = \"figure\"\nwritable = true\n";
        let (hub, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        for f in ["f01", "f02"] {
            std::fs::create_dir_all(root.path().join("refs").join(f)).unwrap();
            std::fs::write(root.path().join("refs").join(f).join("desc.md"), format!("# {f} Pipeline\n\nBoxes.\n")).unwrap();
        }
        let dirs = BTreeMap::from([("App".to_string(), root.path().to_path_buf())]);
        let mut h = Hub::open(hub.path().to_path_buf(), &[M], &dirs, &[]).unwrap();
        let parts = |h: &Hub| -> Vec<(String, usize)> { fam(&h.stats(d("2026-11-24")).unwrap(), "t/圖").parts.iter().map(|p| (p.label.clone(), p.count)).collect() };
        assert_eq!(parts(&h), [("流程圖".to_string(), 0), ("模塊".to_string(), 0), ("元件".to_string(), 0), ("待製作".to_string(), 2)]);
        // a whole figure made from f01: it counts under 流程圖, and f01 is no longer to make
        let spec = serde_json::json!({ "lore_flow": 1, "dsl": "fig demo\nx: box \"A\"\n" });
        let made = h.put(crate::formats::figure::spec_item("t.specs", "Made", &spec, Source::author())).unwrap();
        assert_eq!(parts(&h)[3].1, 2, "not linked yet: both still wait");
        // a reference that lists a drawable part is not made by that: it only says what it is built of
        let mut r = h.store.get("t.refs:f02").unwrap().unwrap();
        r.contains.push(made.id.clone());
        h.put(r).unwrap();
        assert_eq!(parts(&h)[3].1, 2, "listing a part makes nothing");
        // the figure names the reference it was made from: that one is no longer to make
        let mut m = h.store.get(&made.id).unwrap().unwrap();
        m.set("made_from", "t.refs:f01");
        h.put(m).unwrap();
        let s = h.stats(d("2026-11-24")).unwrap();
        let f = fam(&s, "t/圖");
        assert_eq!(f.parts.iter().map(|p| (p.label.as_str(), p.count)).collect::<Vec<_>>(), [("流程圖", 1), ("模塊", 0), ("元件", 0), ("待製作", 1)]);
        assert_eq!(f.gap, "1 個待製作");
    }
}
