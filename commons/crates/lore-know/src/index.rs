//! SQLite index over the items: filter, sort, search, counts. Never the source of truth; it is
//! rebuilt from the items ([`Index::build`]) and kept current by [`Index::upsert`] / [`Index::delete`].
//!
//! Full-text search uses the FTS5 `trigram` tokenizer (CJK has no spaces), as kb-core does;
//! search words shorter than three characters fall back to `LIKE`.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, params, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::*;

const SCHEMA: &str = r#"
DROP TABLE IF EXISTS items; DROP TABLE IF EXISTS item_tags; DROP TABLE IF EXISTS fts;
CREATE TABLE items(
  id TEXT PRIMARY KEY, category TEXT NOT NULL, app TEXT NOT NULL, title TEXT NOT NULL, title_key TEXT NOT NULL,
  source TEXT NOT NULL, level TEXT NOT NULL, level_rank INTEGER NOT NULL, entity INTEGER NOT NULL,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL, used INTEGER NOT NULL, used_at TEXT NOT NULL,
  tag_count INTEGER NOT NULL, body TEXT NOT NULL);
CREATE INDEX items_category ON items(category);
CREATE TABLE item_tags(id TEXT NOT NULL, facet TEXT NOT NULL, tag TEXT NOT NULL);
CREATE INDEX item_tags_id ON item_tags(id);
CREATE INDEX item_tags_ft ON item_tags(facet, tag);
CREATE VIRTUAL TABLE fts USING fts5(id UNINDEXED, text, tokenize = 'trigram');
"#;

/// Sort keys of [`Query::sort`]: id and its label. Every key sorts by value, ascending unless
/// [`Query::desc`]; ties by title. For "recent" and "most" keys ask for `desc`.
pub const SORTS: &[(&str, &str)] = &[
    ("title", "名稱"),
    ("created", "最近新增"),
    ("updated", "最近修改"),
    ("used", "使用次數"),
    ("used_at", "最近使用"),
    ("tags", "標籤數"),
    ("level", "階層"),
    ("todo", "待製作在前"),
];

/// What a list asks for. Empty fields do not filter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Query {
    pub category: Option<String>,
    /// The dashboard section (the category's `app`).
    pub app: Option<String>,
    pub text: String,
    /// Facet → any of these tags; facets all have to match.
    pub tags: BTreeMap<String, Vec<String>>,
    /// `author` · `lore:<app>` · `external:<name>`.
    pub source: Option<String>,
    pub level: Option<String>,
    pub entity: Option<bool>,
    /// One of [`SORTS`]; empty = `title`.
    pub sort: String,
    pub desc: bool,
    /// 0 = no limit.
    pub limit: usize,
    pub offset: usize,
}

/// Numbers of one category.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Count {
    pub category: String,
    pub total: usize,
    /// Drawable / ordinary items (`entity`).
    pub entities: usize,
    /// To-be-made references (`!entity`).
    pub references: usize,
    /// Source key (as in [`Query::source`]) → items.
    pub by_source: BTreeMap<String, usize>,
    /// Figure level → items (items without a level are not counted).
    pub by_level: BTreeMap<String, usize>,
}

fn db<T>(r: rusqlite::Result<T>) -> Result<T> {
    r.map_err(|e| KnowError::Other(format!("索引出錯：{e}")))
}

fn source_key(s: &Source) -> String {
    if s.kind == "author" { "author".into() } else { format!("{}:{}", s.kind, s.name) }
}

fn level_rank(l: &str) -> i64 {
    match l {
        level::COMPONENT => 0,
        level::MODULE => 1,
        level::FIGURE => 2,
        _ => 3,
    }
}

/// Everything a search looks at: title, string field values, tags, triggers.
fn search_text(i: &Item) -> String {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut parts = vec![i.title.clone()];
    i.fields.values().for_each(|v| walk(v, &mut parts));
    parts.extend(i.all_tags().into_iter().map(String::from));
    parts.extend(i.triggers.iter().cloned());
    parts.join("\n")
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// The index.
pub struct Index {
    conn: Connection,
    /// Category id → section, for items of known categories.
    apps: HashMap<String, String>,
    order: Vec<String>,
}

impl Index {
    fn with(conn: Connection) -> Result<Index> {
        db(conn.execute_batch(SCHEMA))?;
        Ok(Index { conn, apps: HashMap::new(), order: vec![] })
    }

    /// An empty index in a file (rebuilt from scratch each time it is opened: it is only a cache).
    pub fn open(path: &Path) -> Result<Index> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        Index::with(db(Connection::open(path))?)
    }

    /// An index in memory holding `items`.
    pub fn build(items: &[Item], categories: &[Category]) -> Result<Index> {
        let mut ix = Index::with(db(Connection::open_in_memory())?)?;
        ix.rebuild(items, categories)?;
        Ok(ix)
    }

    /// Replace everything with `items`.
    pub fn rebuild(&mut self, items: &[Item], categories: &[Category]) -> Result<()> {
        db(self.conn.execute_batch(SCHEMA))?;
        self.apps = categories.iter().map(|c| (c.id.clone(), c.app.clone())).collect();
        self.order = categories.iter().map(|c| c.id.clone()).collect();
        let tx = db(self.conn.unchecked_transaction())?;
        for i in items {
            self.write(i)?;
        }
        db(tx.commit())
    }

    fn app_of(&self, category: &str) -> String {
        self.apps.get(category).cloned().unwrap_or_else(|| category.split('.').next().unwrap_or("").to_string())
    }

    fn write(&self, i: &Item) -> Result<()> {
        let body = search_text(i);
        db(self.conn.execute("DELETE FROM items WHERE id = ?1", [&i.id]))?;
        db(self.conn.execute("DELETE FROM item_tags WHERE id = ?1", [&i.id]))?;
        db(self.conn.execute("DELETE FROM fts WHERE id = ?1", [&i.id]))?;
        db(self.conn.execute(
            "INSERT INTO items(id, category, app, title, title_key, source, level, level_rank, entity, created_at, updated_at, used, used_at, tag_count, body)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                i.id,
                i.category,
                self.app_of(&i.category),
                i.title,
                i.title.to_lowercase(),
                source_key(&i.source),
                i.level,
                level_rank(&i.level),
                i.entity as i64,
                i.created_at,
                i.updated_at,
                i.used as i64,
                i.used_at,
                i.all_tags().len() as i64,
                body
            ],
        ))?;
        for (f, tags) in &i.tags {
            for t in tags {
                db(self.conn.execute("INSERT INTO item_tags(id, facet, tag) VALUES (?1,?2,?3)", params![i.id, f, t]))?;
            }
        }
        db(self.conn.execute("INSERT INTO fts(id, text) VALUES (?1,?2)", params![i.id, body]))?;
        Ok(())
    }

    /// Add the item or replace the one with its id.
    pub fn upsert(&self, i: &Item) -> Result<()> {
        let tx = db(self.conn.unchecked_transaction())?;
        self.write(i)?;
        db(tx.commit())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        db(self.conn.execute("DELETE FROM items WHERE id = ?1", [id]))?;
        db(self.conn.execute("DELETE FROM item_tags WHERE id = ?1", [id]))?;
        db(self.conn.execute("DELETE FROM fts WHERE id = ?1", [id]))?;
        Ok(())
    }

    fn filter(q: &Query) -> (String, Vec<SqlValue>) {
        let mut w: Vec<String> = vec![];
        let mut p: Vec<SqlValue> = vec![];
        let mut add = |clause: &str, v: SqlValue| {
            w.push(clause.to_string());
            p.push(v);
        };
        if let Some(c) = &q.category {
            add("category = ?", c.clone().into());
        }
        if let Some(a) = &q.app {
            add("app = ?", a.clone().into());
        }
        if let Some(s) = &q.source {
            add("source = ?", s.clone().into());
        }
        if let Some(l) = &q.level {
            add("level = ?", l.clone().into());
        }
        if let Some(e) = q.entity {
            add("entity = ?", (e as i64).into());
        }
        for (facet, tags) in &q.tags {
            if tags.is_empty() {
                continue;
            }
            let marks = vec!["?"; tags.len()].join(",");
            w.push(format!("EXISTS (SELECT 1 FROM item_tags t WHERE t.id = items.id AND t.facet = ? AND t.tag IN ({marks}))"));
            p.push(facet.clone().into());
            p.extend(tags.iter().map(|t| SqlValue::from(t.clone())));
        }
        for word in q.text.split_whitespace() {
            if word.chars().count() >= 3 {
                w.push("id IN (SELECT id FROM fts WHERE fts MATCH ?)".into());
                p.push(format!("\"{}\"", word.replace('"', "\"\"")).into());
            } else {
                w.push("body LIKE ? ESCAPE '\\'".into());
                p.push(format!("%{}%", like_escape(word)).into());
            }
        }
        let sql = if w.is_empty() { String::new() } else { format!(" WHERE {}", w.join(" AND ")) };
        (sql, p)
    }

    /// Matching item ids in order, and how many match before `limit` / `offset`.
    pub fn query(&self, q: &Query) -> Result<(Vec<String>, usize)> {
        let key = if q.sort.is_empty() { "title" } else { q.sort.as_str() };
        let col = match key {
            "title" => "title_key",
            "created" => "created_at",
            "updated" => "updated_at",
            "used" => "used",
            "used_at" => "used_at",
            "tags" => "tag_count",
            "level" => "level_rank",
            "todo" => "entity",
            other => return Err(KnowError::Invalid(format!("不認得的排序「{other}」"))),
        };
        let (wh, mut params) = Index::filter(q);
        let total: i64 = db(self.conn.query_row(&format!("SELECT COUNT(*) FROM items{wh}"), params_from_iter(params.iter()), |r| r.get(0)))?;
        let dir = if q.desc { "DESC" } else { "ASC" };
        let mut sql = format!("SELECT id FROM items{wh} ORDER BY {col} {dir}, title_key ASC, id ASC");
        if q.limit > 0 {
            sql.push_str(" LIMIT ? OFFSET ?");
            params.push((q.limit as i64).into());
            params.push((q.offset as i64).into());
        } else if q.offset > 0 {
            sql.push_str(" LIMIT -1 OFFSET ?");
            params.push((q.offset as i64).into());
        }
        let mut st = db(self.conn.prepare(&sql))?;
        let rows = db(st.query_map(params_from_iter(params.iter()), |r| r.get::<_, String>(0)))?;
        let ids = db(rows.collect::<rusqlite::Result<Vec<_>>>())?;
        Ok((ids, total as usize))
    }

    /// One [`Count`] per category the index was built with (in that order; empty ones too), then
    /// any other category that has items.
    pub fn counts(&self) -> Result<Vec<Count>> {
        let mut by: BTreeMap<String, Count> = BTreeMap::new();
        let mut st = db(self.conn.prepare("SELECT category, entity, source, level FROM items"))?;
        let rows = db(st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))))?;
        for row in rows {
            let (c, entity, source, level) = db(row)?;
            let n = by.entry(c.clone()).or_insert_with(|| Count { category: c, ..Default::default() });
            n.total += 1;
            if entity != 0 { n.entities += 1 } else { n.references += 1 }
            *n.by_source.entry(source).or_default() += 1;
            if !level.is_empty() {
                *n.by_level.entry(level).or_default() += 1;
            }
        }
        let mut out: Vec<Count> = vec![];
        for c in &self.order {
            out.push(by.remove(c).unwrap_or_else(|| Count { category: c.clone(), ..Default::default() }));
        }
        out.extend(by.into_values());
        Ok(out)
    }

    /// Facet → tag → number of items, over one category or all.
    pub fn tag_counts(&self, category: Option<&str>) -> Result<BTreeMap<String, BTreeMap<String, usize>>> {
        let mut out: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        let (sql, p): (&str, Vec<SqlValue>) = match category {
            Some(c) => ("SELECT t.facet, t.tag, COUNT(*) FROM item_tags t JOIN items i ON i.id = t.id WHERE i.category = ? GROUP BY t.facet, t.tag", vec![c.to_string().into()]),
            None => ("SELECT facet, tag, COUNT(*) FROM item_tags GROUP BY facet, tag", vec![]),
        };
        let mut st = db(self.conn.prepare(sql))?;
        let rows = db(st.query_map(params_from_iter(p.iter()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))))?;
        for row in rows {
            let (f, t, n) = db(row)?;
            out.entry(f).or_default().insert(t, n as usize);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(id: &str, app: &str, format: &str) -> Category {
        Category { id: id.into(), label: id.into(), app: app.into(), format: format.into(), origin: "lore".into(), writable: true, ..Default::default() }
    }

    fn cats() -> Vec<Category> {
        vec![cat("pc.patterns", "pc", format::PATTERN), cat("sc.figures", "sc", format::FIGURE), cat("my.notes", "my", format::NOTE)]
    }

    #[allow(clippy::too_many_arguments)]
    fn it(category: &str, local: &str, title: &str, source: Source, created: &str, updated: &str, used: u64, used_at: &str) -> Item {
        let mut i = Item::new(category, "x", title, source);
        i.id = item_id(category, local);
        i.created_at = created.into();
        i.updated_at = updated.into();
        i.used = used;
        i.used_at = used_at.into();
        i
    }

    fn sample() -> Vec<Item> {
        let mut a = it("my.notes", "a", "banana", Source::author(), "2026-01-03", "2026-01-01", 5, "2026-02-01");
        a.tag(facet::USE, "paper").tag(facet::USE, "slide").tag(facet::KIND, "timeline");
        let mut b = it("my.notes", "b", "Apple", Source::lore("pc"), "2026-01-01", "2026-01-03", 0, "");
        b.tag(facet::USE, "paper");
        let c = it("my.notes", "c", "cherry", Source::external("acme"), "2026-01-02", "2026-01-02", 9, "2026-01-15");
        let mut d = it("sc.figures", "d", "encoder", Source::lore("sc"), "2026-01-04", "2026-01-04", 1, "2026-03-01");
        d.level = level::MODULE.into();
        d.tag(facet::DOMAIN, "ml");
        let mut e = it("sc.figures", "e", "conv", Source::lore("sc"), "2026-01-05", "2026-01-05", 2, "2026-01-01");
        e.level = level::COMPONENT.into();
        e.entity = false;
        e.tag(facet::DOMAIN, "ml").tag(facet::DOMAIN, "web");
        let mut f = it("sc.figures", "f", "method", Source::lore("sc"), "2026-01-06", "2026-01-06", 0, "");
        f.level = level::FIGURE.into();
        vec![a, b, c, d, e, f]
    }

    fn ids(ix: &Index, q: Query) -> Vec<String> {
        ix.query(&q).unwrap().0.into_iter().map(|s| s.rsplit(':').next().unwrap().to_string()).collect()
    }

    fn sorted(ix: &Index, sort: &str, desc: bool) -> Vec<String> {
        ids(ix, Query { category: None, sort: sort.into(), desc, ..Default::default() })
    }

    #[test]
    fn every_sort_key_orders_as_documented() {
        let ix = Index::build(&sample(), &cats()).unwrap();
        assert_eq!(SORTS.len(), 8);
        // title: case-insensitive; ties never needed here
        assert_eq!(sorted(&ix, "title", false), ["b", "a", "c", "e", "d", "f"]);
        assert_eq!(sorted(&ix, "", false), sorted(&ix, "title", false));
        assert_eq!(sorted(&ix, "title", true), ["f", "d", "e", "c", "a", "b"]);
        // dates: ascending = oldest first; desc = the most recent first
        assert_eq!(sorted(&ix, "created", true), ["f", "e", "d", "a", "c", "b"]);
        assert_eq!(sorted(&ix, "updated", true), ["f", "e", "d", "b", "c", "a"]);
        assert_eq!(sorted(&ix, "created", false)[0], "b");
        // use count and last use (never used = empty = first when ascending)
        assert_eq!(sorted(&ix, "used", true), ["c", "a", "e", "d", "b", "f"]);
        assert_eq!(sorted(&ix, "used_at", true), ["d", "a", "c", "e", "b", "f"]);
        // tags: fewest first
        assert_eq!(sorted(&ix, "tags", false), ["c", "f", "b", "d", "e", "a"]);
        assert_eq!(sorted(&ix, "tags", true)[0], "a");
        // level: component < module < figure < no level
        assert_eq!(&sorted(&ix, "level", false)[..3], ["e", "d", "f"]);
        assert_eq!(&sorted(&ix, "level", true)[3..], ["f", "d", "e"]);
        // todo: references first
        assert_eq!(sorted(&ix, "todo", false)[0], "e");
        assert_eq!(sorted(&ix, "todo", true)[5], "e");
        assert!(matches!(ix.query(&Query { sort: "nope".into(), ..Default::default() }), Err(KnowError::Invalid(_))));
        // limit and offset, with the total before them
        let (page, total) = ix.query(&Query { sort: "title".into(), limit: 2, offset: 1, ..Default::default() }).unwrap();
        assert_eq!((page, total), (vec!["my.notes:a".to_string(), "my.notes:c".to_string()], 6));
        let (rest, _) = ix.query(&Query { offset: 4, ..Default::default() }).unwrap();
        assert_eq!(rest.len(), 2);
    }

    #[test]
    fn filters_by_category_tags_source_level_and_entity() {
        let ix = Index::build(&sample(), &cats()).unwrap();
        let q = |f: &dyn Fn(&mut Query)| {
            let mut q = Query { sort: "title".into(), ..Default::default() };
            f(&mut q);
            ids(&ix, q)
        };
        assert_eq!(q(&|q| q.category = Some("sc.figures".into())), ["e", "d", "f"]);
        assert_eq!(q(&|q| q.app = Some("my".into())), ["b", "a", "c"]);
        // any of the tags within a facet, all facets
        assert_eq!(q(&|q| { q.tags.insert(facet::USE.into(), vec!["paper".into(), "book".into()]); }), ["b", "a"]);
        assert_eq!(q(&|q| {
            q.tags.insert(facet::USE.into(), vec!["paper".into()]);
            q.tags.insert(facet::KIND.into(), vec!["timeline".into()]);
        }), ["a"]);
        assert_eq!(q(&|q| { q.tags.insert(facet::USE.into(), vec![]); }).len(), 6);
        assert_eq!(q(&|q| q.source = Some("author".into())), ["a"]);
        assert_eq!(q(&|q| q.source = Some("lore:pc".into())), ["b"]);
        assert_eq!(q(&|q| q.source = Some("external:acme".into())), ["c"]);
        assert_eq!(q(&|q| q.level = Some(level::MODULE.into())), ["d"]);
        assert_eq!(q(&|q| q.entity = Some(false)), ["e"]);
        assert_eq!(q(&|q| { q.entity = Some(true); q.category = Some("sc.figures".into()); }), ["d", "f"]);
        assert_eq!(q(&|q| { q.category = Some("my.notes".into()); q.source = Some("lore:sc".into()); }).len(), 0);
    }

    #[test]
    fn search_finds_latin_and_cjk_text_and_short_queries() {
        let mut a = it("my.notes", "a", "Softmax cross entropy", Source::author(), "", "", 0, "");
        a.set("body", "把分數轉成機率分布，再取對數");
        a.set("list", vec!["alpha", "beta"]);
        a.triggers.push("logit".into());
        let mut b = it("my.notes", "b", "Other", Source::author(), "", "", 0, "");
        b.set("body", "機率");
        b.tag(facet::USE, "slide-deck");
        let c = it("my.notes", "c", "100% sure_thing", Source::author(), "", "", 0, "");
        let ix = Index::build(&[a, b, c], &cats()).unwrap();
        let find = |t: &str| {
            let mut v = ids(&ix, Query { text: t.into(), sort: "title".into(), ..Default::default() });
            v.sort();
            v
        };
        assert_eq!(find("SOFTMAX"), ["a"]);
        assert_eq!(find("cross entropy"), ["a"]);
        assert_eq!(find("機率分布"), ["a"]);
        assert_eq!(find("機率"), ["a", "b"]);
        assert_eq!(find("機"), ["a", "b"]);
        assert_eq!(find("al"), ["a"]);
        assert_eq!(find("beta"), ["a"]);
        assert_eq!(find("logit"), ["a"]);
        assert_eq!(find("slide-deck"), ["b"]);
        assert_eq!(find("%"), ["c"]);
        assert_eq!(find("e_t"), ["c"]);
        assert_eq!(find("\"quote"), Vec::<String>::new());
        assert_eq!(find("zzzz"), Vec::<String>::new());
    }

    #[test]
    fn upsert_and_delete_keep_counts_right() {
        let ix = Index::build(&sample(), &cats()).unwrap();
        let by = |ix: &Index, c: &str| ix.counts().unwrap().into_iter().find(|n| n.category == c).unwrap();
        let n = by(&ix, "sc.figures");
        assert_eq!((n.total, n.entities, n.references), (3, 2, 1));
        assert_eq!((n.by_level[level::MODULE], n.by_level.len(), n.by_source["lore:sc"]), (1, 3, 3));
        assert_eq!(by(&ix, "pc.patterns").total, 0);
        assert_eq!(ix.counts().unwrap().iter().map(|c| c.category.as_str()).collect::<Vec<_>>(), ["pc.patterns", "sc.figures", "my.notes"]);
        // turn the reference into an entity, retitle another, drop a third
        let mut e = sample().remove(4);
        e.entity = true;
        e.title = "renamed".into();
        ix.upsert(&e).unwrap();
        ix.upsert(&e).unwrap();
        ix.delete("sc.figures:f").unwrap();
        ix.delete("sc.figures:missing").unwrap();
        let n = by(&ix, "sc.figures");
        assert_eq!((n.total, n.entities, n.references, n.by_level.len()), (2, 2, 0, 2));
        assert_eq!(ids(&ix, Query { text: "renamed".into(), ..Default::default() }), ["e"]);
        assert!(ids(&ix, Query { text: "conv".into(), ..Default::default() }).is_empty());
        assert_eq!(ix.query(&Query::default()).unwrap().1, 5);
        // an item of a category the index does not know still counts
        ix.upsert(&it("x.acme.w", "1", "w", Source::external("acme"), "", "", 0, "")).unwrap();
        assert_eq!(ix.counts().unwrap().last().unwrap().category, "x.acme.w");
        assert_eq!(ids(&ix, Query { app: Some("x".into()), ..Default::default() }), ["1"]);
        // a file index works too
        let d = tempfile::tempdir().unwrap();
        let f = Index::open(&d.path().join("sub/index.db")).unwrap();
        f.upsert(&e).unwrap();
        assert_eq!(f.query(&Query::default()).unwrap().1, 1);
    }

    #[test]
    fn tag_counts_per_facet() {
        let ix = Index::build(&sample(), &cats()).unwrap();
        let all = ix.tag_counts(None).unwrap();
        assert_eq!(all[facet::USE]["paper"], 2);
        assert_eq!(all[facet::DOMAIN]["ml"], 2);
        assert_eq!(all[facet::DOMAIN]["web"], 1);
        assert_eq!(all[facet::KIND]["timeline"], 1);
        let fig = ix.tag_counts(Some("sc.figures")).unwrap();
        assert_eq!(fig.keys().collect::<Vec<_>>(), [facet::DOMAIN]);
        assert_eq!(fig[facet::DOMAIN].len(), 2);
        assert!(ix.tag_counts(Some("pc.patterns")).unwrap().is_empty());
    }
}
