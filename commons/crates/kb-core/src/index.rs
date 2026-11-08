//! SQLite index (rusqlite, bundled SQLite with FTS5). Never the source of truth: the
//! knowledge base rebuilds it from the files at start and updates it on every write.
//! Full-text search uses the FTS5 `trigram` tokenizer so CJK text (no spaces) is
//! searchable; queries shorter than three characters fall back to `LIKE`.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;

use chrono::{Duration, Utc};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::clock::Time;
use crate::edges::{Edge, name_key, wikilinks};
use crate::node::Node;
use crate::progress::Progress;

const SCHEMA: &str = r#"
DROP TABLE IF EXISTS nodes; DROP TABLE IF EXISTS node_tags; DROP TABLE IF EXISTS node_domains;
DROP TABLE IF EXISTS names; DROP TABLE IF EXISTS links; DROP TABLE IF EXISTS wikilinks;
DROP TABLE IF EXISTS progress; DROP TABLE IF EXISTS fts;
CREATE TABLE nodes(
  id TEXT PRIMARY KEY, space TEXT NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL,
  summary TEXT NOT NULL, aliases TEXT NOT NULL, tags TEXT NOT NULL, domains TEXT NOT NULL,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL, created_utc TEXT NOT NULL, updated_utc TEXT NOT NULL,
  created_day TEXT NOT NULL, updated_day TEXT NOT NULL, revision INTEGER NOT NULL, image TEXT);
CREATE TABLE node_tags(id TEXT NOT NULL, tag TEXT NOT NULL);
CREATE INDEX node_tags_id ON node_tags(id);
CREATE TABLE node_domains(id TEXT NOT NULL, domain TEXT NOT NULL);
CREATE TABLE names(id TEXT NOT NULL, space TEXT NOT NULL, name TEXT NOT NULL, key TEXT NOT NULL);
CREATE INDEX names_key ON names(key, space);
CREATE TABLE links(src TEXT NOT NULL, dst TEXT NOT NULL, rel TEXT NOT NULL);
CREATE INDEX links_dst ON links(dst);
CREATE TABLE wikilinks(src TEXT NOT NULL, space TEXT NOT NULL, key TEXT NOT NULL, name TEXT NOT NULL);
CREATE TABLE progress(member TEXT NOT NULL, id TEXT NOT NULL, state TEXT NOT NULL, streak INTEGER NOT NULL,
  next_at TEXT, next_utc TEXT, last_seen_utc TEXT, encounters INTEGER NOT NULL, score INTEGER NOT NULL,
  PRIMARY KEY(member, id));
CREATE VIRTUAL TABLE fts USING fts5(id UNINDEXED, title, aliases, summary, body, tokenize = 'trigram');
"#;

pub(crate) fn utc(t: &Time) -> String {
    t.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// One row of a card list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSummary {
    pub id: String,
    pub space: String,
    pub kind: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub tags: Vec<String>,
    pub domains: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
    /// First image (`{"asset", "ext"}`) for thumbnails.
    pub image: Option<serde_json::Value>,
    /// This member's state: learning | known | stale; `None` = no learning state yet.
    pub state: Option<String>,
    pub streak: u32,
    pub next_check_at: Option<String>,
    pub score: u32,
}

/// Card list filters. Empty fields do not filter.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Filter {
    pub space: Option<String>,
    pub kind: Option<String>,
    /// Tag or any tag below it (`deep-learning` matches `deep-learning/cnn`).
    pub tag: Option<String>,
    pub domain: Option<String>,
    /// learning | known | stale | none | due
    pub state: Option<String>,
    /// Full-text query over title, aliases, summary and body.
    pub q: Option<String>,
    /// Inclusive local days `YYYY-MM-DD` on `date_field`.
    pub since: Option<String>,
    pub until: Option<String>,
    /// created (default) | updated
    pub date_field: Option<String>,
    /// updated (default) | created | title | score
    pub sort: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// A graph node: a card, a `tag:<name>` node (`kind = "tag"`) or, with
/// [`GraphQuery::missing`], a placeholder for an unresolved `[[link]]`
/// (`id = "missing:<space>:<key>"`, `kind = "missing"`).
#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub space: String,
    /// This member's state (learning | known | stale); `None` = no learning state.
    pub state: Option<String>,
    pub score: u32,
    pub streak: u32,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub domains: Vec<String>,
    pub summary: String,
    /// RFC 3339 with the card's own offset; empty for tag and placeholder nodes.
    pub created_at: String,
    pub updated_at: String,
    /// Edges (links and mentions, not tag edges) touching this node in the returned view.
    pub degree: u32,
    /// Steps from the center in a local graph; absent in a global graph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hop: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GraphView {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
    /// Cards that matched (space filter, reachable from the center) before `limit`.
    pub total: usize,
    /// `limit` dropped cards.
    pub truncated: bool,
    /// The center actually used (absent when none was given or it is unknown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center: Option<String>,
}

/// What [`Index::graph_with`] returns.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GraphQuery {
    /// Local graph around this card; `None` = global graph.
    pub center: Option<String>,
    /// Steps from the center (both directions).
    pub depth: u32,
    /// Add `tag:<name>` nodes joined to their cards.
    pub with_tags: bool,
    pub space: Option<String>,
    /// Most cards returned (newest first in a global graph, nearest first in a local one).
    pub limit: usize,
    /// Add a placeholder node for every unresolved `[[link]]` (Obsidian's "not created yet").
    pub missing: bool,
}

impl Default for GraphQuery {
    fn default() -> GraphQuery {
        GraphQuery { center: None, depth: 2, with_tags: false, space: None, limit: 400, missing: false }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineItem {
    pub id: String,
    pub space: String,
    pub kind: String,
    pub title: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineDay {
    pub day: String,
    pub created: Vec<TimelineItem>,
    pub updated: Vec<TimelineItem>,
}

pub struct Index {
    conn: Connection,
}

const SUMMARY_COLS: &str = "n.id, n.space, n.kind, n.title, n.aliases, n.summary, n.tags, n.domains, n.created_at, n.updated_at, n.revision, n.image, p.state, p.streak, p.next_at, p.score";

fn row_summary(r: &rusqlite::Row<'_>) -> rusqlite::Result<NodeSummary> {
    let list = |i: usize| -> rusqlite::Result<Vec<String>> { Ok(serde_json::from_str(&r.get::<_, String>(i)?).unwrap_or_default()) };
    let image: Option<String> = r.get(11)?;
    Ok(NodeSummary {
        id: r.get(0)?,
        space: r.get(1)?,
        kind: r.get(2)?,
        title: r.get(3)?,
        aliases: list(4)?,
        summary: r.get(5)?,
        tags: list(6)?,
        domains: list(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
        revision: r.get::<_, i64>(10)? as u64,
        image: image.and_then(|s| serde_json::from_str(&s).ok()),
        state: r.get(12)?,
        streak: r.get::<_, Option<i64>>(13)?.unwrap_or(0) as u32,
        next_check_at: r.get(14)?,
        score: r.get::<_, Option<i64>>(15)?.unwrap_or(0) as u32,
    })
}

/// Escape a user query for FTS5: every word becomes a quoted phrase (AND-ed).
fn fts_query(q: &str) -> Option<String> {
    let words: Vec<String> = q.split_whitespace().filter(|w| w.chars().count() >= 3).map(|w| format!("\"{}\"", w.replace('"', "\"\""))).collect();
    let all_long = q.split_whitespace().all(|w| w.chars().count() >= 3);
    if words.is_empty() || !all_long { None } else { Some(words.join(" ")) }
}

impl Index {
    pub fn open_in_memory() -> Result<Index> {
        Index::init(Connection::open_in_memory()?)
    }

    /// A file-backed index (recreated from scratch: it is always rebuildable).
    pub fn open(path: &Path) -> Result<Index> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        Index::init(Connection::open(path)?)
    }

    fn init(conn: Connection) -> Result<Index> {
        conn.execute_batch(SCHEMA)?;
        Ok(Index { conn })
    }

    /// Empty every table.
    pub fn clear(&self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM nodes; DELETE FROM node_tags; DELETE FROM node_domains; DELETE FROM names; DELETE FROM links; DELETE FROM wikilinks; DELETE FROM progress; DELETE FROM fts;")?;
        Ok(())
    }

    pub fn begin(&self) -> Result<()> {
        self.conn.execute_batch("BEGIN")?;
        Ok(())
    }
    pub fn commit(&self) -> Result<()> {
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    pub fn upsert_node(&self, space: &str, n: &Node) -> Result<()> {
        self.remove_node(&n.id)?;
        let c = &self.conn;
        let image = n.images.first().map(|i| serde_json::json!({ "asset": i.asset, "ext": i.ext }).to_string());
        c.execute(
            "INSERT INTO nodes VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                n.id,
                space,
                n.kind,
                n.title,
                n.summary,
                serde_json::to_string(&n.aliases)?,
                serde_json::to_string(&n.tags)?,
                serde_json::to_string(&n.domains)?,
                n.created_at.to_rfc3339(),
                n.updated_at.to_rfc3339(),
                utc(&n.created_at),
                utc(&n.updated_at),
                n.created_at.date_naive().to_string(),
                n.updated_at.date_naive().to_string(),
                n.revision as i64,
                image,
            ],
        )?;
        for t in &n.tags {
            c.execute("INSERT INTO node_tags VALUES (?1, ?2)", params![n.id, t])?;
        }
        for d in &n.domains {
            c.execute("INSERT INTO node_domains VALUES (?1, ?2)", params![n.id, d])?;
        }
        for name in n.names() {
            c.execute("INSERT INTO names VALUES (?1, ?2, ?3, ?4)", params![n.id, space, name, name_key(name)])?;
        }
        for l in &n.links {
            c.execute("INSERT INTO links VALUES (?1, ?2, ?3)", params![n.id, l.to, l.rel])?;
        }
        for w in wikilinks(&n.body) {
            c.execute("INSERT INTO wikilinks VALUES (?1, ?2, ?3, ?4)", params![n.id, space, name_key(&w), w])?;
        }
        c.execute("INSERT INTO fts(id, title, aliases, summary, body) VALUES (?1, ?2, ?3, ?4, ?5)", params![n.id, n.title, n.aliases.join(" ; "), n.summary, n.body])?;
        Ok(())
    }

    pub fn remove_node(&self, id: &str) -> Result<()> {
        for t in ["nodes", "node_tags", "node_domains", "names", "fts"] {
            self.conn.execute(&format!("DELETE FROM {t} WHERE id = ?1"), [id])?;
        }
        self.conn.execute("DELETE FROM links WHERE src = ?1", [id])?;
        self.conn.execute("DELETE FROM wikilinks WHERE src = ?1", [id])?;
        Ok(())
    }

    pub fn space_of(&self, id: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT space FROM nodes WHERE id = ?1", [id], |r| r.get(0)).optional()?)
    }

    pub fn count(&self) -> Result<u64> {
        Ok(self.conn.query_row("SELECT count(*) FROM nodes", [], |r| r.get::<_, i64>(0))? as u64)
    }

    pub fn upsert_progress(&self, member: &str, id: &str, p: &Progress) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO progress VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                member,
                id,
                p.state.as_str(),
                p.streak as i64,
                p.next_check_at.map(|t| t.to_rfc3339()),
                p.next_check_at.as_ref().map(utc),
                p.last_seen_at.as_ref().map(utc),
                p.encounters as i64,
                p.score() as i64
            ],
        )?;
        Ok(())
    }

    pub fn remove_progress(&self, member: &str, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM progress WHERE member = ?1 AND id = ?2", [member, id])?;
        Ok(())
    }

    pub fn summary(&self, member: &str, id: &str) -> Result<Option<NodeSummary>> {
        let sql = format!("SELECT {SUMMARY_COLS} FROM nodes n LEFT JOIN progress p ON p.id = n.id AND p.member = ?1 WHERE n.id = ?2");
        Ok(self.conn.query_row(&sql, [member, id], row_summary).optional()?)
    }

    pub fn list(&self, member: &str, f: &Filter, now: Time) -> Result<Vec<NodeSummary>> {
        let mut sql = format!("SELECT {SUMMARY_COLS} FROM nodes n LEFT JOIN progress p ON p.id = n.id AND p.member = ? WHERE 1=1");
        let mut args: Vec<SqlValue> = vec![member.to_string().into()];
        let nonempty = |o: &Option<String>| o.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        if let Some(v) = nonempty(&f.space) {
            sql.push_str(" AND n.space = ?");
            args.push(v.into());
        }
        if let Some(v) = nonempty(&f.kind) {
            sql.push_str(" AND n.kind = ?");
            args.push(v.into());
        }
        if let Some(v) = nonempty(&f.tag) {
            sql.push_str(" AND EXISTS (SELECT 1 FROM node_tags t WHERE t.id = n.id AND (t.tag = ? OR substr(t.tag, 1, length(?) + 1) = ? || '/'))");
            args.extend([v.clone().into(), v.clone().into(), v.into()]);
        }
        if let Some(v) = nonempty(&f.domain) {
            sql.push_str(" AND EXISTS (SELECT 1 FROM node_domains d WHERE d.id = n.id AND (d.domain = ? OR substr(d.domain, 1, length(?) + 1) = ? || '/'))");
            args.extend([v.clone().into(), v.clone().into(), v.into()]);
        }
        if let Some(v) = nonempty(&f.state) {
            match v.as_str() {
                "none" => sql.push_str(" AND p.state IS NULL"),
                "due" => {
                    sql.push_str(" AND p.next_utc IS NOT NULL AND p.next_utc <= ?");
                    args.push(utc(&now).into());
                }
                s => {
                    sql.push_str(" AND p.state = ?");
                    args.push(s.to_string().into());
                }
            }
        }
        let day_col = if f.date_field.as_deref() == Some("updated") { "n.updated_day" } else { "n.created_day" };
        if let Some(v) = nonempty(&f.since) {
            sql.push_str(&format!(" AND {day_col} >= ?"));
            args.push(v.chars().take(10).collect::<String>().into());
        }
        if let Some(v) = nonempty(&f.until) {
            sql.push_str(&format!(" AND {day_col} <= ?"));
            args.push(v.chars().take(10).collect::<String>().into());
        }
        if let Some(q) = nonempty(&f.q) {
            match fts_query(&q) {
                Some(m) => {
                    sql.push_str(" AND n.id IN (SELECT id FROM fts WHERE fts MATCH ?)");
                    args.push(m.into());
                }
                None => {
                    // Short words (one CJK character, `CT`): substring match.
                    for w in q.split_whitespace() {
                        sql.push_str(" AND (instr(lower(n.title), ?) > 0 OR instr(lower(n.aliases), ?) > 0 OR instr(lower(n.summary), ?) > 0 OR n.id IN (SELECT id FROM fts WHERE instr(lower(body), ?) > 0))");
                        let w = w.to_lowercase();
                        args.extend([w.clone().into(), w.clone().into(), w.clone().into(), w.into()]);
                    }
                }
            }
        }
        sql.push_str(match f.sort.as_deref() {
            Some("created") => " ORDER BY n.created_utc DESC, n.id DESC",
            Some("title") => " ORDER BY lower(n.title) ASC",
            Some("score") => " ORDER BY coalesce(p.score, 0) ASC, n.updated_utc DESC",
            _ => " ORDER BY n.updated_utc DESC, n.id DESC",
        });
        sql.push_str(" LIMIT ? OFFSET ?");
        args.push((f.limit.unwrap_or(200).min(5000) as i64).into());
        args.push((f.offset.unwrap_or(0) as i64).into());
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(params_from_iter(args), row_summary)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Cards whose check is due for `member`, oldest first.
    pub fn due(&self, member: &str, now: Time, limit: u32) -> Result<Vec<NodeSummary>> {
        let sql = format!("SELECT {SUMMARY_COLS} FROM nodes n JOIN progress p ON p.id = n.id AND p.member = ?1 WHERE p.next_utc IS NOT NULL AND p.next_utc <= ?2 ORDER BY p.next_utc ASC LIMIT ?3");
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(params![member, utc(&now), limit as i64], row_summary)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Cards created or edited in the last `days` days, newest first.
    pub fn recent(&self, member: &str, now: Time, days: i64, limit: u32) -> Result<Vec<NodeSummary>> {
        let from = utc(&(now - Duration::days(days)));
        let sql = format!("SELECT {SUMMARY_COLS} FROM nodes n LEFT JOIN progress p ON p.id = n.id AND p.member = ?1 WHERE n.created_utc >= ?2 OR n.updated_utc >= ?2 ORDER BY n.updated_utc DESC LIMIT ?3");
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(params![member, from, limit as i64], row_summary)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Every edge between indexed cards: explicit links and resolved mentions.
    pub fn edges(&self) -> Result<Vec<Edge>> {
        let mut out = Vec::new();
        let mut st = self.conn.prepare("SELECT l.src, l.dst, l.rel FROM links l JOIN nodes a ON a.id = l.src JOIN nodes b ON b.id = l.dst")?;
        for r in st.query_map([], |r| Ok(Edge { from: r.get(0)?, to: r.get(1)?, rel: r.get(2)?, origin: "link".into() }))? {
            out.push(r?);
        }
        let mut st = self.conn.prepare("SELECT DISTINCT w.src, n.id FROM wikilinks w JOIN names n ON n.key = w.key AND n.space = w.space WHERE n.id != w.src")?;
        for r in st.query_map([], |r| Ok(Edge { from: r.get(0)?, to: r.get(1)?, rel: "mentions".into(), origin: "mention".into() }))? {
            out.push(r?);
        }
        Ok(out)
    }

    /// Edges pointing at `id` (reverse edges).
    pub fn backlinks(&self, id: &str) -> Result<Vec<Edge>> {
        Ok(self.edges()?.into_iter().filter(|e| e.to == id).collect())
    }

    /// Edges leaving `id` (explicit links resolved to indexed cards, mentions).
    pub fn outlinks(&self, id: &str) -> Result<Vec<Edge>> {
        Ok(self.edges()?.into_iter().filter(|e| e.from == id).collect())
    }

    /// Unresolved `[[…]]` targets of a card (no card with that title or alias yet).
    pub fn unresolved_mentions(&self, id: &str) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT w.key FROM wikilinks w WHERE w.src = ?1 AND NOT EXISTS (SELECT 1 FROM names n WHERE n.key = w.key AND n.space = w.space)")?;
        let rows = st.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }

    /// Graph around `center` (`depth` steps, both directions) or the whole graph.
    /// `with_tags` adds `tag:<name>` nodes joined to their cards.
    pub fn graph(&self, member: &str, center: Option<&str>, depth: u32, with_tags: bool, space: Option<&str>, limit: usize) -> Result<GraphView> {
        self.graph_with(member, &GraphQuery { center: center.map(str::to_string), depth, with_tags, space: space.map(str::to_string), limit, missing: false })
    }

    /// Unresolved `[[…]]` mentions as `(src, placeholder id, shown name, space)`.
    fn missing_mentions(&self) -> Result<Vec<(String, String, String, String)>> {
        let mut st = self.conn.prepare("SELECT w.src, w.space, w.key, w.name FROM wikilinks w WHERE NOT EXISTS (SELECT 1 FROM names n WHERE n.key = w.key AND n.space = w.space) ORDER BY w.rowid")?;
        let rows = st
            .query_map([], |r| {
                let (src, space, key, name): (String, String, String, String) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
                Ok((src, format!("missing:{space}:{key}"), name, space))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Global or local graph (see [`GraphQuery`]).
    pub fn graph_with(&self, member: &str, q: &GraphQuery) -> Result<GraphView> {
        let space = q.space.as_deref().filter(|s| !s.is_empty());
        let mut st = self.conn.prepare("SELECT n.id, n.title, n.kind, n.space, n.tags, p.state, p.score, p.streak, n.aliases, n.domains, n.summary, n.created_at, n.updated_at FROM nodes n LEFT JOIN progress p ON p.id = n.id AND p.member = ?1")?;
        let list = |s: String| -> Vec<String> { serde_json::from_str(&s).unwrap_or_default() };
        let mut all: HashMap<String, GraphNode> = st
            .query_map([member], |r| {
                Ok(GraphNode {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    kind: r.get(2)?,
                    space: r.get(3)?,
                    tags: list(r.get(4)?),
                    state: r.get(5)?,
                    score: r.get::<_, Option<i64>>(6)?.unwrap_or(0) as u32,
                    streak: r.get::<_, Option<i64>>(7)?.unwrap_or(0) as u32,
                    aliases: list(r.get(8)?),
                    domains: list(r.get(9)?),
                    summary: r.get(10)?,
                    created_at: r.get(11)?,
                    updated_at: r.get(12)?,
                    degree: 0,
                    hop: None,
                })
            })?
            .filter_map(|r| r.ok())
            .filter(|n| space.is_none_or(|s| n.space == s))
            .map(|n| (n.id.clone(), n))
            .collect();
        let mut edges: Vec<Edge> = self.edges()?.into_iter().filter(|e| all.contains_key(&e.from) && all.contains_key(&e.to)).collect();
        // Placeholders: not cards, so they never count against `limit` or `total`.
        let mut placeholders: HashSet<String> = HashSet::new();
        if q.missing {
            let mut seen_edges: HashSet<(String, String)> = HashSet::new();
            for (src, pid, name, sp) in self.missing_mentions()? {
                if !all.contains_key(&src) || !seen_edges.insert((src.clone(), pid.clone())) {
                    continue;
                }
                if placeholders.insert(pid.clone()) {
                    let blank = GraphNode { id: pid.clone(), title: name, kind: "missing".into(), space: sp, state: None, score: 0, streak: 0, tags: vec![], aliases: vec![], domains: vec![], summary: String::new(), created_at: String::new(), updated_at: String::new(), degree: 0, hop: None };
                    all.insert(pid.clone(), blank);
                }
                edges.push(Edge { from: src, to: pid, rel: "mentions".into(), origin: "mention".into() });
            }
        }
        let cards = all.len() - placeholders.len();
        let limit = q.limit.max(1);
        let mut keep: Vec<String>;
        let mut hops: HashMap<String, u32> = HashMap::new();
        let total;
        let center = q.center.as_deref().filter(|c| all.contains_key(*c) && !placeholders.contains(*c)).map(str::to_string);
        match &center {
            Some(c) => {
                let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
                for e in &edges {
                    adj.entry(&e.from).or_default().push(&e.to);
                    adj.entry(&e.to).or_default().push(&e.from);
                }
                let mut seen: HashSet<String> = HashSet::from([c.clone()]);
                let mut queue = VecDeque::from([(c.clone(), 0u32)]);
                let mut order: Vec<String> = Vec::new();
                while let Some((id, d)) = queue.pop_front() {
                    hops.insert(id.clone(), d);
                    order.push(id.clone());
                    if d >= q.depth || placeholders.contains(&id) {
                        continue;
                    }
                    for nb in adj.get(id.as_str()).into_iter().flatten() {
                        if seen.insert(nb.to_string()) {
                            queue.push_back((nb.to_string(), d + 1));
                        }
                    }
                }
                total = order.iter().filter(|id| !placeholders.contains(*id)).count();
                keep = order.iter().filter(|id| !placeholders.contains(*id)).take(limit).cloned().collect();
                let kept: HashSet<&String> = keep.iter().collect();
                let extra: Vec<String> = order.iter().filter(|id| placeholders.contains(*id) && edges.iter().any(|e| &e.to == *id && kept.contains(&e.from))).cloned().collect();
                keep.extend(extra);
            }
            None => {
                total = cards;
                let mut ids: Vec<&String> = all.keys().filter(|k| !placeholders.contains(*k)).collect();
                ids.sort();
                keep = ids.into_iter().rev().take(limit).cloned().collect();
                let kept: HashSet<&String> = keep.iter().collect();
                let mut extra: Vec<String> = edges.iter().filter(|e| placeholders.contains(&e.to) && kept.contains(&e.from)).map(|e| e.to.clone()).collect();
                extra.sort();
                extra.dedup();
                keep.extend(extra);
            }
        }
        let set: HashSet<&String> = keep.iter().collect();
        let kept_cards = keep.iter().filter(|k| !placeholders.contains(*k)).count();
        let view_edges: Vec<Edge> = edges.iter().filter(|e| set.contains(&e.from) && set.contains(&e.to)).cloned().collect();
        let mut degree: HashMap<&str, u32> = HashMap::new();
        for e in &view_edges {
            *degree.entry(e.from.as_str()).or_default() += 1;
            *degree.entry(e.to.as_str()).or_default() += 1;
        }
        let nodes: Vec<GraphNode> = keep
            .iter()
            .filter_map(|id| all.get(id).cloned())
            .map(|mut n| {
                n.degree = degree.get(n.id.as_str()).copied().unwrap_or(0);
                n.hop = hops.get(&n.id).copied();
                n
            })
            .collect();
        let mut view = GraphView { nodes, edges: view_edges, total, truncated: kept_cards < total, center };
        if q.with_tags {
            let mut tags: BTreeMap<String, ()> = BTreeMap::new();
            let mut tag_edges = Vec::new();
            for n in &view.nodes {
                for t in &n.tags {
                    tags.insert(t.clone(), ());
                    tag_edges.push(Edge { from: n.id.clone(), to: format!("tag:{t}"), rel: "tag".into(), origin: "tag".into() });
                }
            }
            for t in tags.into_keys() {
                view.nodes.push(GraphNode { id: format!("tag:{t}"), title: format!("#{t}"), kind: "tag".into(), space: String::new(), state: None, score: 0, streak: 0, tags: vec![], aliases: vec![], domains: vec![], summary: String::new(), created_at: String::new(), updated_at: String::new(), degree: 0, hop: None });
            }
            view.edges.extend(tag_edges);
        }
        Ok(view)
    }

    /// Per local day (the card's own offset): cards created and cards last edited, for
    /// the last `days` days, newest day first.
    pub fn timeline(&self, now: Time, days: i64, space: Option<&str>) -> Result<Vec<TimelineDay>> {
        let start = (now - Duration::days(days.max(1) - 1)).date_naive().to_string();
        let mut st = self.conn.prepare("SELECT id, space, kind, title, created_at, updated_at, created_day, updated_day, revision FROM nodes WHERE (created_day >= ?1 OR updated_day >= ?1) ORDER BY updated_utc DESC")?;
        let mut map: BTreeMap<String, TimelineDay> = BTreeMap::new();
        let rows = st.query_map([&start], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?, r.get::<_, String>(6)?, r.get::<_, String>(7)?, r.get::<_, i64>(8)?))
        })?;
        for row in rows {
            let (id, sp, kind, title, cat, uat, cday, uday, rev) = row?;
            if space.is_some_and(|s| s != sp) {
                continue;
            }
            let item = |at: &str| TimelineItem { id: id.clone(), space: sp.clone(), kind: kind.clone(), title: title.clone(), at: at.to_string() };
            if cday >= start {
                map.entry(cday.clone()).or_insert_with(|| TimelineDay { day: cday.clone(), created: vec![], updated: vec![] }).created.push(item(&cat));
            }
            if uday >= start && rev > 1 {
                map.entry(uday.clone()).or_insert_with(|| TimelineDay { day: uday.clone(), created: vec![], updated: vec![] }).updated.push(item(&uat));
            }
        }
        Ok(map.into_values().rev().collect())
    }

    /// `(id, space, title, name)` for every title and alias (term scanner input).
    pub fn names(&self) -> Result<Vec<(String, String, String, String)>> {
        let mut st = self.conn.prepare("SELECT n.id, n.space, x.title, n.name FROM names n JOIN nodes x ON x.id = n.id ORDER BY x.created_utc")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// This member's state per card (scanner colouring).
    pub fn states(&self, member: &str) -> Result<HashMap<String, String>> {
        let mut st = self.conn.prepare("SELECT id, state FROM progress WHERE member = ?1")?;
        let rows = st.query_map([member], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(rows)
    }

    /// Card ids that have a title or alias equal to `name` (case-insensitive), in `space`.
    pub fn find_by_name(&self, space: &str, name: &str) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT DISTINCT id FROM names WHERE key = ?1 AND space = ?2")?;
        let rows = st.query_map([name_key(name), space.to_string()], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }
}
