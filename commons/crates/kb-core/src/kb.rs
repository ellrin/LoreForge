//! [`KnowledgeBase`]: mounted spaces + index + clock + the current member. The only
//! writer of node and progress files (kb-server wraps it; other Rust apps may open
//! it read-only).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use ym_store::{AssetRef, CommandRunner, Space, SpaceKind, Spaces, SystemRunner, free_path, valid_member, write_atomic};

use crate::clock::{Clock, SystemClock, Time};
use crate::edges::Edge;
use crate::id::{node_id, valid_node_id};
use crate::index::{Filter, GraphQuery, GraphView, Index, NodeSummary, TimelineDay};
use crate::llm::{CardAssistant, DraftRequest, MockAssistant};
use crate::node::{CopyRef, Image, Link, Node, NodeFields};
use crate::progress::{Intervals, Outcome, Progress, Step};
use crate::registry::Registry;
use crate::scan::{ScanOptions, ScanSpan, Scanner};
use crate::tags::TagsFile;
use crate::{KbError, Result};

pub struct KbOptions {
    pub clock: Arc<dyn Clock>,
    pub intervals: Intervals,
    /// Runs `brctl` for iCloud placeholders.
    pub runner: Arc<dyn CommandRunner>,
    /// Card drafting / term extraction (ym-llm integration point; mock for now).
    pub assistant: Arc<dyn CardAssistant>,
    /// Keep the index in a file instead of memory (it is rebuilt at open either way).
    pub index_path: Option<PathBuf>,
}

impl Default for KbOptions {
    fn default() -> KbOptions {
        KbOptions { clock: Arc::new(SystemClock), intervals: Intervals::default(), runner: Arc::new(SystemRunner), assistant: Arc::new(MockAssistant), index_path: None }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceInfo {
    pub id: String,
    pub name: String,
    pub kind: SpaceKind,
    pub members: Vec<String>,
    /// The current member belongs to this space (can record learning state).
    pub is_member: bool,
    pub assets_backend: String,
    pub cards: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeView {
    pub space: String,
    pub node: Node,
    pub progress: Option<Progress>,
    pub backlinks: Vec<Edge>,
    pub outlinks: Vec<Edge>,
    /// `[[…]]` targets with no card yet.
    pub unresolved: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecallResult {
    pub progress: Progress,
    pub step: Step,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub spans: Vec<ScanSpan>,
    /// Terms without a card (only with `candidates: true`).
    pub candidates: Vec<String>,
    pub assistant: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RebuildReport {
    pub nodes: usize,
    pub progress: usize,
    /// Files that could not be read (skipped, never changed).
    pub errors: Vec<String>,
}

pub struct KnowledgeBase {
    spaces: Spaces,
    member: String,
    opts: KbOptions,
    index: Index,
    registries: HashMap<String, Registry>,
    tags: HashMap<String, TagsFile>,
    scanner: Option<Scanner>,
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json") && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn to_json(v: &impl Serialize) -> Result<Vec<u8>> {
    let mut s = serde_json::to_string_pretty(v)?;
    s.push('\n');
    Ok(s.into_bytes())
}

impl KnowledgeBase {
    pub fn open(spaces: Spaces, member: &str, opts: KbOptions) -> Result<KnowledgeBase> {
        if !valid_member(member) {
            return Err(KbError::Invalid(format!("member id `{member}`")));
        }
        let index = match &opts.index_path {
            Some(p) => Index::open(p)?,
            None => Index::open_in_memory()?,
        };
        let mut kb = KnowledgeBase { spaces, member: member.to_string(), opts, index, registries: HashMap::new(), tags: HashMap::new(), scanner: None };
        kb.rebuild()?;
        Ok(kb)
    }

    pub fn member(&self) -> &str {
        &self.member
    }
    pub fn spaces(&self) -> &Spaces {
        &self.spaces
    }
    pub fn now(&self) -> Time {
        self.opts.clock.now()
    }
    pub fn intervals(&self) -> &Intervals {
        &self.opts.intervals
    }
    pub fn assistant_name(&self) -> String {
        self.opts.assistant.name()
    }

    /// Re-read every space (registries, tags, nodes, this member's progress).
    pub fn rebuild(&mut self) -> Result<RebuildReport> {
        let mut report = RebuildReport::default();
        let now = self.now();
        self.index.clear()?;
        self.registries.clear();
        self.tags.clear();
        self.scanner = None;
        self.index.begin()?;
        let spaces: Vec<Space> = self.spaces.iter().cloned().collect();
        for space in &spaces {
            self.registries.insert(space.id().to_string(), Registry::load(&space.root)?);
            self.tags.insert(space.id().to_string(), TagsFile::load(&space.root).unwrap_or_else(|e| {
                report.errors.push(format!("{}/tags.json: {e}", space.id()));
                TagsFile::default()
            }));
            for path in json_files(&space.nodes_dir()) {
                match std::fs::read_to_string(&path).map_err(KbError::from).and_then(|s| serde_json::from_str::<Node>(&s).map_err(KbError::from)) {
                    Ok(n) if n.id.is_empty() => report.errors.push(format!("{}: no id", path.display())),
                    Ok(n) => {
                        if let Some(other) = self.index.space_of(&n.id)? {
                            report.errors.push(format!("{}: id {} already in space {other}", path.display(), n.id));
                            continue;
                        }
                        self.index.upsert_node(space.id(), &n)?;
                        report.nodes += 1;
                    }
                    Err(e) => report.errors.push(format!("{}: {e}", path.display())),
                }
            }
            if let Ok(dir) = space.progress_dir(&self.member) {
                for path in json_files(&dir) {
                    let id = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    match std::fs::read_to_string(&path).map_err(KbError::from).and_then(|s| serde_json::from_str::<Progress>(&s).map_err(KbError::from)) {
                        Ok(mut p) => {
                            // Stale is computed on read; the file changes at the next event.
                            p.refresh(now, &self.opts.intervals);
                            if self.index.space_of(&id)?.as_deref() == Some(space.id()) {
                                self.index.upsert_progress(&self.member, &id, &p)?;
                                report.progress += 1;
                            }
                        }
                        Err(e) => report.errors.push(format!("{}: {e}", path.display())),
                    }
                }
            }
        }
        self.index.commit()?;
        Ok(report)
    }

    pub fn spaces_info(&self) -> Result<Vec<SpaceInfo>> {
        let mut out = Vec::new();
        for s in self.spaces.iter() {
            let cards = self.index.list(&self.member, &Filter { space: Some(s.id().to_string()), limit: Some(5000), ..Default::default() }, self.now())?.len() as u64;
            out.push(SpaceInfo {
                id: s.id().to_string(),
                name: s.config.name.clone(),
                kind: s.kind(),
                members: s.config.members.clone(),
                is_member: s.is_member(&self.member),
                assets_backend: format!("{:?}", s.config.backend()).to_lowercase(),
                cards,
            });
        }
        Ok(out)
    }

    pub fn registry(&self, space: &str) -> Result<&Registry> {
        self.registries.get(space).ok_or_else(|| KbError::NotFound(format!("space `{space}`")))
    }

    pub fn tags(&self, space: &str) -> Result<&TagsFile> {
        self.tags.get(space).ok_or_else(|| KbError::NotFound(format!("space `{space}`")))
    }

    /// Tags used by cards (with counts), merged with `tags.json` entries.
    pub fn tag_counts(&self, space: Option<&str>) -> Result<Vec<(String, u64)>> {
        let mut counts: std::collections::BTreeMap<String, u64> = Default::default();
        for s in self.spaces.iter().filter(|s| space.is_none_or(|x| x == s.id())) {
            if let Some(t) = self.tags.get(s.id()) {
                for k in t.tags.keys() {
                    counts.entry(k.clone()).or_default();
                }
            }
        }
        for n in self.index.list(&self.member, &Filter { space: space.map(str::to_string), limit: Some(5000), ..Default::default() }, self.now())? {
            for t in n.tags {
                *counts.entry(t).or_default() += 1;
            }
        }
        Ok(counts.into_iter().collect())
    }

    fn space(&self, id: &str) -> Result<&Space> {
        Ok(self.spaces.require(id)?)
    }

    fn node_path(space: &Space, id: &str) -> PathBuf {
        space.nodes_dir().join(format!("{id}.json"))
    }

    /// The space a card is in.
    pub fn space_of(&self, id: &str) -> Result<String> {
        if !valid_node_id(id) {
            return Err(KbError::NotFound(format!("card `{id}`")));
        }
        self.index.space_of(id)?.ok_or_else(|| KbError::NotFound(format!("card `{id}`")))
    }

    /// Read a card from its file (the source of truth).
    pub fn node(&self, id: &str) -> Result<(String, Node)> {
        let sid = self.space_of(id)?;
        let path = Self::node_path(self.space(&sid)?, id);
        let node: Node = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        Ok((sid, node))
    }

    pub fn view(&self, id: &str) -> Result<NodeView> {
        let (space, node) = self.node(id)?;
        Ok(NodeView { progress: self.progress(id)?, backlinks: self.index.backlinks(id)?, outlinks: self.index.outlinks(id)?, unresolved: self.index.unresolved_mentions(id)?, space, node })
    }

    pub fn summary(&self, id: &str) -> Result<Option<NodeSummary>> {
        self.index.summary(&self.member, id)
    }

    fn progress_path(&self, space: &Space, id: &str) -> Result<PathBuf> {
        Ok(space.progress_dir(&self.member)?.join(format!("{id}.json")))
    }

    /// This member's learning state for a card (`None` if never recorded or not a
    /// member of the card's space).
    pub fn progress(&self, id: &str) -> Result<Option<Progress>> {
        let sid = self.space_of(id)?;
        let space = self.space(&sid)?;
        let Ok(path) = self.progress_path(space, id) else { return Ok(None) };
        if !path.is_file() {
            return Ok(None);
        }
        let mut p: Progress = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        p.refresh(self.now(), &self.opts.intervals);
        Ok(Some(p))
    }

    fn save_progress(&self, space: &Space, id: &str, p: &Progress) -> Result<()> {
        write_atomic(&self.progress_path(space, id)?, &to_json(p)?)?;
        self.index.upsert_progress(&self.member, id, p)
    }

    /// Checks before a card is written to `space` (design §3.2 included).
    fn validate(&self, space: &Space, node: &Node) -> Result<()> {
        if node.title.trim().is_empty() {
            return Err(KbError::Invalid("title is empty".into()));
        }
        let reg = self.registry(space.id())?;
        if !reg.has_kind(&node.kind) {
            return Err(KbError::Invalid(format!("unknown kind `{}` (add it to kinds.toml or apps.toml)", node.kind)));
        }
        for l in &node.links {
            if !reg.has_link_type(&l.rel) {
                return Err(KbError::Invalid(format!("unknown link type `{}`", l.rel)));
            }
            if !valid_node_id(&l.to) || l.to == node.id {
                return Err(KbError::Invalid(format!("link target `{}`", l.to)));
            }
            if space.is_household() && self.index.space_of(&l.to)?.as_deref() != Some(space.id()) {
                return Err(KbError::Forbidden(format!("a household card may only link to cards in the same space (`{}`)", l.to)));
            }
        }
        if space.is_household() && !node.copies.is_empty() {
            return Err(KbError::Forbidden("household cards never record where they were copied from".into()));
        }
        for img in &node.images {
            AssetRef::parse(&img.asset, &img.ext)?;
        }
        Ok(())
    }

    fn write_node(&mut self, space: &Space, node: &Node) -> Result<()> {
        write_atomic(&Self::node_path(space, &node.id), &to_json(node)?)?;
        self.index.upsert_node(space.id(), node)?;
        self.scanner = None;
        Ok(())
    }

    /// Create a card in `space` (always named: writes never guess a space). The
    /// current member's learning state starts as `learning`.
    pub fn create(&mut self, space_id: &str, fields: NodeFields, via: Option<&str>) -> Result<Node> {
        let space = self.space(space_id)?.clone();
        let now = self.now();
        let mut node = Node {
            id: node_id(now.timestamp_millis()),
            kind: "concept".into(),
            title: String::new(),
            aliases: vec![],
            lang: String::new(),
            domains: vec![],
            tags: vec![],
            summary: String::new(),
            body: String::new(),
            images: vec![],
            symbols: vec![],
            links: vec![],
            sources: vec![],
            external: vec![],
            copies: vec![],
            created_at: now,
            updated_at: now,
            revision: 1,
            extra: Default::default(),
        };
        fields.apply_to(&mut node);
        self.validate(&space, &node)?;
        if Self::node_path(&space, &node.id).exists() {
            return Err(KbError::Invalid(format!("{} already exists", node.id)));
        }
        self.write_node(&space, &node)?;
        if space.is_member(&self.member) {
            self.save_progress(&space, &node.id, &Progress::new(now, via, &self.opts.intervals))?;
        }
        Ok(node)
    }

    /// Edit a card: revision + 1, `updated_at` = now. With `expected_revision`, refuse
    /// if the file changed in between.
    pub fn update(&mut self, id: &str, fields: NodeFields, expected_revision: Option<u64>) -> Result<Node> {
        let (sid, mut node) = self.node(id)?;
        if let Some(r) = expected_revision && r != node.revision {
            return Err(KbError::Conflict { expected: r, actual: node.revision });
        }
        let space = self.space(&sid)?.clone();
        fields.apply_to(&mut node);
        node.revision += 1;
        node.updated_at = self.now();
        self.validate(&space, &node)?;
        self.write_node(&space, &node)?;
        Ok(node)
    }

    /// Move a card to `trash/nodes/` (and this member's learning state to
    /// `trash/progress/<member>/`). Nothing is deleted. Other members' learning state
    /// stays where it is (in household mode each member only commits their own folder).
    pub fn trash(&mut self, id: &str) -> Result<PathBuf> {
        let sid = self.space_of(id)?;
        let space = self.space(&sid)?.clone();
        let dir = space.trash_dir().join("nodes");
        std::fs::create_dir_all(&dir)?;
        let to = free_path(&dir, id, "json");
        std::fs::rename(Self::node_path(&space, id), &to)?;
        if let Ok(p) = self.progress_path(&space, id) && p.is_file() {
            let pdir = space.trash_dir().join("progress").join(&self.member);
            std::fs::create_dir_all(&pdir)?;
            std::fs::rename(&p, free_path(&pdir, id, "json"))?;
        }
        self.index.remove_node(id)?;
        self.index.remove_progress(&self.member, id)?;
        self.scanner = None;
        Ok(to)
    }

    /// Undo [`trash`](Self::trash): move the most recently trashed copy of a card (and this
    /// member's learning state, when it was trashed with it) back to `nodes/`. Refuses when
    /// a card with that id exists again. Returns the space and the card.
    pub fn restore(&mut self, id: &str) -> Result<(String, Node)> {
        if !valid_node_id(id) {
            return Err(KbError::NotFound(format!("card `{id}`")));
        }
        if self.index.space_of(id)?.is_some() {
            return Err(KbError::Invalid(format!("card `{id}` is not in the trash")));
        }
        // `free_path` names repeated trashings `<id>-2.json`, `<id>-3.json`…: take the last.
        let latest = |dir: &Path| -> Option<PathBuf> {
            let mut best: Option<(usize, PathBuf)> = None;
            for e in std::fs::read_dir(dir).ok()?.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                let Some(stem) = name.strip_suffix(".json") else { continue };
                let n = if stem == id {
                    1
                } else if let Some(k) = stem.strip_prefix(id).and_then(|r| r.strip_prefix('-')).and_then(|k| k.parse::<usize>().ok()) {
                    k
                } else {
                    continue;
                };
                if best.as_ref().is_none_or(|(b, _)| n > *b) {
                    best = Some((n, e.path()));
                }
            }
            best.map(|(_, p)| p)
        };
        let spaces: Vec<Space> = self.spaces.iter().cloned().collect();
        for space in &spaces {
            let Some(from) = latest(&space.trash_dir().join("nodes")) else { continue };
            let node: Node = serde_json::from_str(&std::fs::read_to_string(&from)?)?;
            let to = Self::node_path(space, id);
            if to.exists() {
                return Err(KbError::Invalid(format!("{} already exists", to.display())));
            }
            std::fs::create_dir_all(space.nodes_dir())?;
            std::fs::rename(&from, &to)?;
            self.index.upsert_node(space.id(), &node)?;
            self.scanner = None;
            if let Ok(p) = self.progress_path(space, id)
                && !p.exists()
                && let Some(pf) = latest(&space.trash_dir().join("progress").join(&self.member))
            {
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&pf, &p)?;
                let mut pr: Progress = serde_json::from_str(&std::fs::read_to_string(&p)?)?;
                pr.refresh(self.now(), &self.opts.intervals);
                self.index.upsert_progress(&self.member, id, &pr)?;
            }
            return Ok((space.id().to_string(), node));
        }
        Err(KbError::NotFound(format!("card `{id}` in the trash")))
    }

    /// Answer a check (記得 / 忘了 / 不熟). A card without learning state for this member
    /// (e.g. made by another household member) starts one first.
    pub fn recall(&mut self, id: &str, outcome: Outcome, via: Option<&str>) -> Result<RecallResult> {
        let sid = self.space_of(id)?;
        let space = self.space(&sid)?.clone();
        let now = self.now();
        let mut p = self.progress(id)?.unwrap_or_else(|| Progress::new(now, via, &self.opts.intervals));
        let step = p.recall(outcome, now, via, &self.opts.intervals);
        self.save_progress(&space, id, &p)?;
        Ok(RecallResult { progress: p, step })
    }

    /// The term was met while reading. Only cards that already have learning state.
    pub fn encounter(&mut self, id: &str, via: Option<&str>) -> Result<Option<Progress>> {
        let sid = self.space_of(id)?;
        let space = self.space(&sid)?.clone();
        let Some(mut p) = self.progress(id)? else { return Ok(None) };
        p.encounter(self.now(), via, &self.opts.intervals);
        self.save_progress(&space, id, &p)?;
        Ok(Some(p))
    }

    pub fn list(&self, f: &Filter) -> Result<Vec<NodeSummary>> {
        self.index.list(&self.member, f, self.now())
    }

    pub fn due(&self, limit: u32) -> Result<Vec<NodeSummary>> {
        self.index.due(&self.member, self.now(), limit)
    }

    pub fn recent(&self, days: i64, limit: u32) -> Result<Vec<NodeSummary>> {
        self.index.recent(&self.member, self.now(), days, limit)
    }

    pub fn graph(&self, center: Option<&str>, depth: u32, with_tags: bool, space: Option<&str>, limit: usize) -> Result<GraphView> {
        self.index.graph(&self.member, center, depth, with_tags, space, limit)
    }

    /// Global or local graph with every option (placeholders for unresolved `[[links]]`, …).
    pub fn graph_with(&self, q: &GraphQuery) -> Result<GraphView> {
        self.index.graph_with(&self.member, q)
    }

    pub fn timeline(&self, days: i64, space: Option<&str>) -> Result<Vec<TimelineDay>> {
        self.index.timeline(self.now(), days, space)
    }

    /// Find known terms in `text`.
    pub fn scan(&mut self, text: &str, opts: &ScanOptions) -> Result<ScanResult> {
        if self.scanner.is_none() {
            self.scanner = Some(Scanner::build(&self.index.names()?, &[])?);
        }
        let states = self.index.states(&self.member)?;
        let mut spans = self.scanner.as_ref().expect("built").scan(text, &states);
        if !opts.spaces.is_empty() {
            spans.retain(|s| opts.spaces.contains(&s.space));
        }
        if opts.record {
            let firsts: Vec<String> = spans.iter().filter(|s| s.first && s.state != "none").map(|s| s.node_id.clone()).collect();
            for id in firsts {
                self.encounter(&id, opts.via.as_deref().or(Some("scan")))?;
            }
        }
        let (candidates, assistant) = if opts.candidates {
            let known: Vec<String> = self.index.names()?.into_iter().map(|r| r.3.to_lowercase()).collect();
            let found: Vec<String> = spans.iter().map(|s| s.text.to_lowercase()).collect();
            let c = self.opts.assistant.extract_terms(text, &known)?.into_iter().filter(|t| !found.contains(&t.to_lowercase())).collect();
            (c, Some(self.opts.assistant.name()))
        } else {
            (vec![], None)
        };
        Ok(ScanResult { spans, candidates, assistant })
    }

    /// A card draft from context (not saved). ym-llm integration point.
    pub fn draft(&self, req: &DraftRequest) -> Result<NodeFields> {
        self.opts.assistant.draft_card(req)
    }

    /// Store an image (or PDF) in a space's asset store.
    pub fn put_asset(&self, space: &str, bytes: &[u8], ext: &str) -> Result<AssetRef> {
        Ok(self.space(space)?.assets(self.opts.runner.clone()).put(bytes, ext)?)
    }

    pub fn get_asset(&self, space: &str, r: &AssetRef) -> Result<Vec<u8>> {
        Ok(self.space(space)?.assets(self.opts.runner.clone()).get(r)?)
    }

    /// Personal cards that record copies (`copies`), for link remapping on share.
    fn copy_records(&self) -> Result<Vec<(String, CopyRef)>> {
        let mut out = Vec::new();
        for s in self.spaces.iter().filter(|s| s.kind() == SpaceKind::Personal) {
            for path in json_files(&s.nodes_dir()) {
                if let Ok(n) = std::fs::read_to_string(&path).map_err(KbError::from).and_then(|t| serde_json::from_str::<Node>(&t).map_err(KbError::from)) {
                    for c in n.copies {
                        out.push((n.id.clone(), c));
                    }
                }
            }
        }
        Ok(out)
    }

    /// Explicit share: copy a card into another space (design §3.2). The copy gets a
    /// new id; images are copied into the target's asset store (household: webp).
    /// Links are kept only when their target already exists in the target space
    /// (directly or as an earlier copy); a household copy never names personal ids.
    /// The personal side records the relation in `copies`.
    pub fn share(&mut self, id: &str, to_space: &str) -> Result<Node> {
        let (from_id, src) = self.node(id)?;
        let from = self.space(&from_id)?.clone();
        let to = self.space(to_space)?.clone();
        if from.id() == to.id() {
            return Err(KbError::Invalid("the card is already in that space".into()));
        }
        ym_store::check_flow(&from, &to, true)?;
        if !self.registry(to.id())?.has_kind(&src.kind) {
            return Err(KbError::Invalid(format!("space `{}` has no kind `{}`", to.id(), src.kind)));
        }
        let now = self.now();
        let records = self.copy_records()?;
        // Same card in the target space, if any: the target itself, or a recorded copy.
        let counterpart = |target: &str| -> Option<String> {
            if self.index.space_of(target).ok().flatten().as_deref() == Some(to.id()) {
                return Some(target.to_string());
            }
            for (owner, c) in &records {
                if owner == target && c.space == to.id() {
                    return Some(c.id.clone());
                }
                if c.id == target && c.space == from.id() && self.index.space_of(owner).ok().flatten().as_deref() == Some(to.id()) {
                    return Some(owner.clone());
                }
            }
            None
        };
        let links: Vec<Link> = src.links.iter().filter_map(|l| counterpart(&l.to).map(|t| Link { to: t, rel: l.rel.clone(), extra: l.extra.clone() })).collect();
        let (from_assets, to_assets) = (from.assets(self.opts.runner.clone()), to.assets(self.opts.runner.clone()));
        let mut images: Vec<Image> = Vec::new();
        for img in &src.images {
            let r = to_assets.import_from(&from_assets, &AssetRef::parse(&img.asset, &img.ext)?)?;
            images.push(Image { asset: r.asset, ext: r.ext, ..img.clone() });
        }
        let mut copy = Node { id: node_id(now.timestamp_millis()), links, images, copies: vec![], created_at: now, updated_at: now, revision: 1, ..src.clone() };
        if to.kind() == SpaceKind::Personal {
            copy.copies.push(CopyRef { space: from.id().to_string(), id: src.id.clone(), dir: "from".into(), at: now });
        }
        self.validate(&to, &copy)?;
        self.write_node(&to, &copy)?;
        if to.is_member(&self.member) {
            self.save_progress(&to, &copy.id, &Progress::new(now, Some("share"), &self.opts.intervals))?;
        }
        if from.kind() == SpaceKind::Personal {
            let mut s = src;
            s.copies.push(CopyRef { space: to.id().to_string(), id: copy.id.clone(), dir: "to".into(), at: now });
            s.revision += 1;
            s.updated_at = now;
            self.write_node(&from, &s)?;
        }
        Ok(copy)
    }
}
