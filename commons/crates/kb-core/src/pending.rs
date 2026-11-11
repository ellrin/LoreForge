//! 待審區 (pending cards): candidate cards that another app (LoreEcho) proposes and the
//! author confirms in KnowledgeBase. One `pending/<id>.json` per card in each space
//! (id = `p` + ULID). Pending cards are **not** nodes: the index, the card list, review
//! and search never read `pending/`. Accepting creates a normal card (state `learning`)
//! and moves the file to `trash/pending/`; nothing is ever deleted.
//!
//! The free functions take only a [`Space`], so an app that does not run KnowledgeBase can
//! write candidates directly (`Space::open(root)` + [`add`]); they only ever write inside
//! `pending/` (and `trash/pending/` when a card is discarded). The same operations are on
//! [`KnowledgeBase`] (`pending_*`), which also does accept / merge.

use std::path::PathBuf;

use lore_store::{Space, free_path, write_atomic};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::clock::Time;
use crate::id::ulid;
use crate::kb::KnowledgeBase;
use crate::node::{Link, Node, NodeFields, Source};
use crate::{KbError, Result};

const CROCKFORD: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Pending ids are `p` + 26 Crockford characters (also the file name).
pub fn valid_pending_id(id: &str) -> bool {
    id.len() == 27 && id.starts_with('p') && id[1..].chars().all(|c| CROCKFORD.contains(c))
}

/// What a caller proposes. Everything but the title is optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NewPending {
    pub title: String,
    pub kind: String,
    pub aliases: Vec<String>,
    pub lang: String,
    pub domains: Vec<String>,
    pub tags: Vec<String>,
    pub summary: String,
    pub body: String,
    /// Links to existing cards (`to` = card id; dropped at accept time when the target is gone).
    pub links: Vec<Link>,
    /// Where it came from, one line (e.g. the conversation title).
    pub source: String,
    pub from_conversation: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A stored pending card: the card fields plus `source`, `created`, `from_conversation`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingCard {
    pub id: String,
    #[serde(default)]
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub source: String,
    pub created: Time,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_conversation: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl PendingCard {
    /// Title and aliases, trimmed, without empties.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.title.as_str()).chain(self.aliases.iter().map(|s| s.as_str())).map(str::trim).filter(|s| !s.is_empty())
    }

    /// The card fields this candidate would become, with `edits` laid over them.
    fn fields(&self, edits: NodeFields) -> NodeFields {
        let mut sources = vec![];
        if !self.source.trim().is_empty() || self.from_conversation.is_some() {
            sources.push(Source {
                kind: "loreecho".into(),
                reference: self.source.trim().to_string(),
                location: self.from_conversation.clone().unwrap_or_default(),
                extra: Map::new(),
            });
        }
        let mut extra = self.extra.clone();
        extra.extend(edits.extra);
        NodeFields {
            kind: Some(edits.kind.unwrap_or_else(|| self.kind.clone())),
            title: Some(edits.title.unwrap_or_else(|| self.title.clone())),
            aliases: Some(edits.aliases.unwrap_or_else(|| self.aliases.clone())),
            lang: Some(edits.lang.unwrap_or_else(|| self.lang.clone())),
            domains: Some(edits.domains.unwrap_or_else(|| self.domains.clone())),
            tags: Some(edits.tags.unwrap_or_else(|| self.tags.clone())),
            summary: Some(edits.summary.unwrap_or_else(|| self.summary.clone())),
            body: Some(edits.body.unwrap_or_else(|| self.body.clone())),
            links: Some(edits.links.unwrap_or_else(|| self.links.clone())),
            sources: Some(edits.sources.unwrap_or(sources)),
            images: edits.images,
            symbols: edits.symbols,
            external: edits.external,
            extra,
        }
    }
}

/// `<space>/pending/`
pub fn dir(space: &Space) -> PathBuf {
    space.file("pending")
}

fn path(space: &Space, id: &str) -> Result<PathBuf> {
    if !valid_pending_id(id) {
        return Err(KbError::NotFound(format!("pending card `{id}`")));
    }
    Ok(dir(space).join(format!("{id}.json")))
}

fn to_json(c: &PendingCard) -> Result<Vec<u8>> {
    let mut s = serde_json::to_string_pretty(c)?;
    s.push('\n');
    Ok(s.into_bytes())
}

/// Write candidates into `space`'s `pending/` (new ids; never overwrites). An empty title
/// is refused and nothing is written.
pub fn add(space: &Space, now: Time, cards: Vec<NewPending>) -> Result<Vec<PendingCard>> {
    if cards.iter().any(|c| c.title.trim().is_empty()) {
        return Err(KbError::Invalid("a pending card needs a title".into()));
    }
    let d = dir(space);
    std::fs::create_dir_all(&d)?;
    let mut out = Vec::new();
    for c in cards {
        let card = PendingCard {
            id: format!("p{}", ulid(now.timestamp_millis().max(0) as u64)),
            kind: if c.kind.trim().is_empty() { "concept".into() } else { c.kind.trim().to_string() },
            title: c.title.trim().to_string(),
            aliases: c.aliases.into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect(),
            lang: c.lang,
            domains: c.domains,
            tags: c.tags,
            summary: c.summary,
            body: c.body,
            links: c.links,
            source: c.source.trim().to_string(),
            created: now,
            from_conversation: c.from_conversation.filter(|s| !s.trim().is_empty()),
            extra: c.extra.into_iter().filter(|(k, _)| !["id", "created", "revision", "created_at", "updated_at"].contains(&k.as_str())).collect(),
        };
        let p = path(space, &card.id)?;
        if p.exists() {
            return Err(KbError::Invalid(format!("{} already exists", card.id)));
        }
        write_atomic(&p, &to_json(&card)?)?;
        out.push(card);
    }
    Ok(out)
}

/// One pending card.
pub fn get(space: &Space, id: &str) -> Result<PendingCard> {
    let p = path(space, id)?;
    let s = std::fs::read_to_string(&p).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { KbError::NotFound(format!("pending card `{id}`")) } else { e.into() })?;
    Ok(serde_json::from_str(&s)?)
}

/// Every pending card of `space`, oldest first. Unreadable files are skipped.
pub fn list(space: &Space) -> Vec<PendingCard> {
    let mut v: Vec<PendingCard> = std::fs::read_dir(dir(space))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json") && p.file_stem().is_some_and(|s| valid_pending_id(&s.to_string_lossy())))
                .filter_map(|p| serde_json::from_str::<PendingCard>(&std::fs::read_to_string(p).ok()?).ok())
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| a.created.cmp(&b.created).then(a.id.cmp(&b.id)));
    v
}

/// Replace a pending card's fields (the id and `created` stay).
pub fn update(space: &Space, id: &str, edits: NodeFields) -> Result<PendingCard> {
    let mut c = get(space, id)?;
    if let Some(v) = edits.title {
        if v.trim().is_empty() {
            return Err(KbError::Invalid("title is empty".into()));
        }
        c.title = v.trim().to_string();
    }
    if let Some(v) = edits.kind {
        c.kind = v;
    }
    if let Some(v) = edits.aliases {
        c.aliases = v;
    }
    if let Some(v) = edits.tags {
        c.tags = v;
    }
    if let Some(v) = edits.domains {
        c.domains = v;
    }
    if let Some(v) = edits.summary {
        c.summary = v;
    }
    if let Some(v) = edits.body {
        c.body = v;
    }
    if let Some(v) = edits.links {
        c.links = v;
    }
    write_atomic(&path(space, id)?, &to_json(&c)?)?;
    Ok(c)
}

/// Throw a pending card away: the file moves to `trash/pending/`.
pub fn discard(space: &Space, id: &str) -> Result<()> {
    let p = path(space, id)?;
    if !p.is_file() {
        return Err(KbError::NotFound(format!("pending card `{id}`")));
    }
    let d = space.trash_dir().join("pending");
    std::fs::create_dir_all(&d)?;
    std::fs::rename(&p, free_path(&d, id, "json"))?;
    Ok(())
}

fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// An existing card that has the same title or alias as a pending card.
#[derive(Debug, Clone, Serialize)]
pub struct Duplicate {
    pub id: String,
    pub title: String,
    pub summary: String,
}

fn union(into: &mut Vec<String>, add: &[String]) {
    for a in add {
        if !a.trim().is_empty() && !into.iter().any(|x| same(x, a)) {
            into.push(a.trim().to_string());
        }
    }
}

impl KnowledgeBase {
    /// Write candidates into a space's `pending/` (nothing else changes).
    pub fn pending_add(&self, space_id: &str, cards: Vec<NewPending>) -> Result<Vec<PendingCard>> {
        add(self.spaces().require(space_id)?, self.now(), cards)
    }

    /// Pending cards of one space (or every space), each with its space id.
    pub fn pending_list(&self, space_id: Option<&str>) -> Result<Vec<(String, PendingCard)>> {
        let mut out = Vec::new();
        for s in self.spaces().iter() {
            if space_id.is_some_and(|x| x != s.id()) {
                continue;
            }
            out.extend(list(s).into_iter().map(|c| (s.id().to_string(), c)));
        }
        if let Some(x) = space_id && self.spaces().get(x).is_none() {
            return Err(KbError::NotFound(format!("space `{x}`")));
        }
        Ok(out)
    }

    pub fn pending_get(&self, space_id: &str, id: &str) -> Result<PendingCard> {
        get(self.spaces().require(space_id)?, id)
    }

    pub fn pending_update(&self, space_id: &str, id: &str, edits: NodeFields) -> Result<PendingCard> {
        update(self.spaces().require(space_id)?, id, edits)
    }

    pub fn pending_discard(&self, space_id: &str, id: &str) -> Result<()> {
        discard(self.spaces().require(space_id)?, id)
    }

    /// Existing cards (same space) whose title or alias equals the pending card's title or
    /// alias, ignoring case.
    pub fn pending_duplicates(&self, space_id: &str, card: &PendingCard) -> Result<Vec<Duplicate>> {
        let names: Vec<&str> = card.names().collect();
        let all = self.list(&crate::Filter { space: Some(space_id.to_string()), limit: Some(100_000), ..Default::default() })?;
        Ok(all
            .into_iter()
            .filter(|n| std::iter::once(&n.title).chain(n.aliases.iter()).any(|x| names.iter().any(|m| same(m, x))))
            .map(|n| Duplicate { id: n.id, title: n.title, summary: n.summary })
            .collect())
    }

    /// Make the pending card a normal card (state `learning`), with `edits` over the
    /// candidate's fields, and move the file to `trash/pending/`. Links whose target is not
    /// a card of this space are dropped, an unknown link type becomes `related`, an unknown
    /// kind becomes `concept`.
    pub fn pending_accept(&mut self, space_id: &str, id: &str, edits: Option<NodeFields>) -> Result<Node> {
        let card = self.pending_get(space_id, id)?;
        let mut f = card.fields(edits.unwrap_or_default());
        self.sanitize(space_id, &mut f)?;
        let node = self.create(space_id, f, Some("loreecho"))?;
        discard(self.spaces().require(space_id)?, id)?;
        Ok(node)
    }

    /// Fold the pending card into an existing card instead of creating a new one: its title
    /// and aliases become aliases, tags/domains/links are added, an empty summary is filled,
    /// a differing body is appended after a blank line. Then the pending file is discarded.
    pub fn pending_merge(&mut self, space_id: &str, id: &str, into: &str, edits: Option<NodeFields>) -> Result<Node> {
        let card = self.pending_get(space_id, id)?;
        let (sid, target) = self.node(into)?;
        if sid != space_id {
            return Err(KbError::Forbidden("a pending card merges only into a card of its own space".into()));
        }
        let mut f = card.fields(edits.unwrap_or_default());
        self.sanitize(space_id, &mut f)?;
        let mut aliases = target.aliases.clone();
        union(&mut aliases, &[f.title.clone().unwrap_or_default()]);
        union(&mut aliases, f.aliases.as_deref().unwrap_or(&[]));
        let mut tags = target.tags.clone();
        union(&mut tags, f.tags.as_deref().unwrap_or(&[]));
        let mut domains = target.domains.clone();
        union(&mut domains, f.domains.as_deref().unwrap_or(&[]));
        let mut links = target.links.clone();
        for l in f.links.take().unwrap_or_default() {
            if l.to != target.id && !links.iter().any(|x| x.to == l.to && x.rel == l.rel) {
                links.push(l);
            }
        }
        let new_body = f.body.take().unwrap_or_default();
        let body = if new_body.trim().is_empty() || target.body.contains(new_body.trim()) {
            target.body.clone()
        } else if target.body.trim().is_empty() {
            new_body
        } else {
            format!("{}\n\n{}", target.body.trim_end(), new_body.trim())
        };
        let summary = if target.summary.trim().is_empty() { f.summary.take().unwrap_or_default() } else { target.summary.clone() };
        let mut sources = target.sources.clone();
        for s in f.sources.take().unwrap_or_default() {
            if !sources.contains(&s) {
                sources.push(s);
            }
        }
        let merged = NodeFields { aliases: Some(aliases), tags: Some(tags), domains: Some(domains), links: Some(links), body: Some(body), summary: Some(summary), sources: Some(sources), ..Default::default() };
        let node = self.update(into, merged, Some(target.revision))?;
        discard(self.spaces().require(space_id)?, id)?;
        Ok(node)
    }

    fn sanitize(&self, space_id: &str, f: &mut NodeFields) -> Result<()> {
        let reg = self.registry(space_id)?;
        if f.kind.as_deref().is_none_or(|k| !reg.has_kind(k)) {
            f.kind = Some("concept".into());
        }
        let links = f.links.take().unwrap_or_default();
        let mut kept = Vec::new();
        for mut l in links {
            if self.space_of(&l.to).ok().as_deref() != Some(space_id) {
                continue;
            }
            if !self.registry(space_id)?.has_link_type(&l.rel) {
                l.rel = "related".into();
            }
            kept.push(l);
        }
        f.links = Some(kept);
        if f.title.as_deref().is_none_or(|t| t.trim().is_empty()) {
            return Err(KbError::Invalid("title is empty".into()));
        }
        Ok(())
    }
}
