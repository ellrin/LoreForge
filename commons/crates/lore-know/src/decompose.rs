//! K8: a picture of a flow figure, split into the figure, its modules and their components, and
//! proposed for the database (docs/brain-hub.md §2 rules 3 and 6, §4.2, §4.3, §7).
//!
//! This module owns no model call and decodes no picture: it builds the request text and the JSON
//! schema ([`prompt`], [`schema`]), cleans the model's JSON reply ([`parse`]), matches the pieces
//! to what the library already has ([`key`], [`find`]) and turns the result into a [`Proposal`]
//! ([`plan`]). What the library already has is linked, never duplicated; what it cannot draw is a
//! **reference** to make (`entity = false`), never an invented drawable entity.
//!
//! The reply's shape: the figure contains every module of the reply; each module names its
//! components; `components` lists every component once (a name a module uses but the list forgot
//! is added with an empty description by [`parse`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::types::{facet, format, level, Change, Item, KnowError, Proposal, Result, Source};

// ── the reply ────────────────────────────────────────────────────────────────

fn nullable<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// One thing seen in the picture, at any of the three sizes.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Piece {
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(deserialize_with = "nullable")]
    pub description: String,
    /// The field (`facet::DOMAIN`): machine learning, web ...
    #[serde(deserialize_with = "nullable")]
    pub domain: Vec<String>,
    /// The kind of structure (`facet::KIND`): architecture, data flow, timeline, network ...
    #[serde(deserialize_with = "nullable")]
    pub kind: Vec<String>,
    /// Other names it goes by (used for matching only).
    #[serde(deserialize_with = "nullable")]
    pub also: Vec<String>,
}

/// A module and the names of its components.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Module {
    #[serde(flatten)]
    pub piece: Piece,
    #[serde(deserialize_with = "nullable")]
    pub components: Vec<String>,
}

/// The model's reply. The figure contains every module listed here.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Reply {
    pub figure: Piece,
    #[serde(deserialize_with = "nullable")]
    pub modules: Vec<Module>,
    #[serde(deserialize_with = "nullable")]
    pub components: Vec<Piece>,
}

fn piece_schema(extra: Option<Value>) -> Value {
    let strs = json!({"type": "array", "items": {"type": "string"}});
    let mut props = json!({
        "name": {"type": "string"},
        "description": {"type": "string"},
        "domain": strs,
        "kind": strs,
        "also": strs,
    });
    if let Some(Value::Object(m)) = extra {
        for (k, v) in m {
            props[k] = v;
        }
    }
    json!({"type": "object", "properties": props, "required": ["name"]})
}

/// The JSON schema of [`Reply`].
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "figure": piece_schema(None),
            "modules": {"type": "array", "items": piece_schema(Some(json!({
                "components": {"type": "array", "items": {"type": "string"}}
            })))},
            "components": {"type": "array", "items": piece_schema(None)},
        },
        "required": ["figure"],
    })
}

fn clean_list(v: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for s in v.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(s)) {
            out.push(s.to_string());
        }
    }
    out
}

fn clean_piece(p: &Piece) -> Piece {
    Piece {
        name: p.name.trim().to_string(),
        description: p.description.trim().to_string(),
        domain: clean_list(&p.domain),
        kind: clean_list(&p.kind),
        also: clean_list(&p.also),
    }
}

fn union(a: &mut Vec<String>, b: &[String]) {
    a.extend(b.iter().cloned());
    *a = clean_list(a);
}

/// Merge `b` into `a` (the same thing named twice): union of tags and other names, the longer description.
fn merge_piece(a: &mut Piece, b: &Piece) {
    if b.description.chars().count() > a.description.chars().count() {
        a.description = b.description.clone();
    }
    union(&mut a.domain, &b.domain);
    union(&mut a.kind, &b.kind);
    union(&mut a.also, &b.also);
}

/// Read and clean the model's reply: trimmed, pieces without a name dropped, a component or module
/// named twice merged, a component only a module names added with an empty description.
pub fn parse(v: &Value) -> Result<Reply> {
    let raw: Reply = serde_json::from_value(v.clone()).map_err(|e| KnowError::Invalid(format!("看不懂模型回的拆圖結果：{e}")))?;
    let figure = clean_piece(&raw.figure);
    if figure.name.is_empty() {
        return Err(KnowError::Invalid("模型的回覆裡這張圖沒有名稱，沒辦法拆。請再試一次，或在備註裡說明這張圖是什麼".into()));
    }
    // components, merged by name key (the first spelling is kept)
    let mut components: Vec<Piece> = vec![];
    for p in raw.components.iter().map(clean_piece).filter(|p| !key(&p.name).is_empty()) {
        let k = key(&p.name);
        match components.iter_mut().find(|c| key(&c.name) == k) {
            Some(c) => merge_piece(c, &p),
            None => components.push(p),
        }
    }
    // modules, merged by name key; their component names point at the canonical component names
    let mut modules: Vec<Module> = vec![];
    for m in &raw.modules {
        let piece = clean_piece(&m.piece);
        if key(&piece.name).is_empty() {
            continue;
        }
        let mut names: Vec<String> = vec![];
        for n in m.components.iter().map(|s| s.trim()).filter(|s| !key(s).is_empty()) {
            let k = key(n);
            let canon = match components.iter().find(|c| key(&c.name) == k) {
                Some(c) => c.name.clone(),
                None => {
                    components.push(Piece { name: n.to_string(), ..Default::default() });
                    n.to_string()
                }
            };
            if !names.iter().any(|x| key(x) == k) {
                names.push(canon);
            }
        }
        match modules.iter_mut().find(|x| key(&x.piece.name) == key(&piece.name)) {
            Some(x) => {
                merge_piece(&mut x.piece, &piece);
                for n in names {
                    if !x.components.iter().any(|y| key(y) == key(&n)) {
                        x.components.push(n);
                    }
                }
            }
            None => modules.push(Module { piece, components: names }),
        }
    }
    Ok(Reply { figure, modules, components })
}

// ── the request ──────────────────────────────────────────────────────────────

/// At most this many library names are listed in a request.
const LISTED: usize = 150;

const SYSTEM: &str = "\
You are given a picture of a flow figure (for example a method diagram from a paper, a network \
diagram or a timeline). Split it into three sizes.

1. figure: the whole picture. Give it a short name and say in one or two sentences what it shows.
2. modules: the larger blocks it is built from (for example an encoder, a decoder, an attention \
block, a client side, a server side). The figure contains every module you list.
3. components: the smallest reusable parts the modules are made of (for example a conv layer, a \
pooling layer, a node that adds two inputs, a LAN, a WAN, a load balancer). List each component \
once, and name in each module the components it uses.

Rules:
- Prefer the smallest reusable components, and use the real names as they are used in the field \
(conv layer, encoder, LAN ...), not names you make up for this picture.
- Describe only what you can see in the picture. Do not add parts that are not drawn, and do not \
guess what the authors might have meant.
- Give every piece several tags from three kinds: domain (the field, for example machine learning \
or web), kind (the kind of structure, for example architecture, data flow, timeline, network) and, \
when clear, use. Put domain tags in `domain` and structure tags in `kind`.
- The user message lists what the library already has as `name [level]`. Whenever a piece IS one of \
those things, reuse that name exactly instead of inventing a new one. Put other names it goes by \
in `also`.
- Reply with one JSON value that follows the given schema and nothing else.";

/// The request: `(system, user)`. The user text lists the library's figure items as `name [level]`
/// (entities first, then by level and name; at most 150) and the author's note when given.
pub fn prompt(existing: &[Item], note: &str) -> (String, String) {
    let rank = |l: &str| match l {
        level::COMPONENT => 0,
        level::MODULE => 1,
        level::FIGURE => 2,
        _ => 3,
    };
    let mut items: Vec<&Item> = existing.iter().filter(|i| i.format == format::FIGURE && !i.title.trim().is_empty()).collect();
    items.sort_by_key(|i| (!i.entity, rank(&i.level), key(&i.title), i.title.clone(), i.id.clone()));
    let mut user = String::from("The library already has these figure items (name [level]):\n");
    if items.is_empty() {
        user.push_str("(none)\n");
    }
    for i in items.iter().take(LISTED) {
        let l = if i.level.is_empty() { "?" } else { i.level.as_str() };
        user.push_str(&format!("{} [{}]\n", i.title.trim(), l));
    }
    if items.len() > LISTED {
        user.push_str(&format!("(and {} more, not listed)\n", items.len() - LISTED));
    }
    let note = note.trim();
    if !note.is_empty() {
        user.push_str(&format!("\nThe author's note about this picture:\n{note}\n"));
    }
    user.push_str("\nSplit the attached picture into figure, modules and components.");
    (SYSTEM.to_string(), user)
}

// ── matching to the library ──────────────────────────────────────────────────

/// A name as compared: lowercase, ASCII punctuation and hyphens as spaces, spaces collapsed, a
/// trailing plural `s` dropped from each word of 4 or more letters (not `ss`).
pub fn key(name: &str) -> String {
    let lower = name.to_lowercase();
    let spaced: String = lower.chars().map(|c| if c.is_ascii_punctuation() { ' ' } else { c }).collect();
    spaced
        .split_whitespace()
        .map(|w| {
            if w.chars().count() >= 4 && w.ends_with('s') && !w.ends_with("ss") { &w[..w.len() - 1] } else { w }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The existing figure item of level `lvl` that `piece` is: its title key equals the key of the
/// piece's name or of one of its other names. An entity is preferred to a reference.
pub fn find<'a>(piece: &Piece, lvl: &str, existing: &'a [Item]) -> Option<&'a Item> {
    let keys: Vec<String> = std::iter::once(piece.name.as_str()).chain(piece.also.iter().map(String::as_str)).map(key).filter(|k| !k.is_empty()).collect();
    let mut hits = existing.iter().filter(|i| i.format == format::FIGURE && i.level == lvl && keys.contains(&key(&i.title)));
    let first = hits.next()?;
    if first.entity {
        return Some(first);
    }
    Some(hits.find(|i| i.entity).unwrap_or(first))
}

// ── the proposal ─────────────────────────────────────────────────────────────

struct Planner<'a> {
    existing: &'a [Item],
    category: &'a str,
    by: &'a str,
    changes: Vec<Change>,
    /// stored id → index of its `update` change (several pieces may name one item)
    updates: BTreeMap<String, usize>,
    next: usize,
    added: usize,
    linked: usize,
}

impl Planner<'_> {
    /// Place a piece and return the id of the item it is (stored, or temporary). `contains` are
    /// the ids of its parts.
    fn place(&mut self, p: &Piece, lvl: &str, contains: Vec<String>, image: &str) -> String {
        match find(p, lvl, self.existing) {
            Some(have) => {
                self.linked += 1;
                self.link(have, p, contains);
                have.id.clone()
            }
            None => {
                self.next += 1;
                self.added += 1;
                let mut it = Item::new(self.category, format::FIGURE, &p.name, Source::lore(self.by));
                it.id = format!("new:{}", self.next);
                it.level = lvl.into();
                it.entity = false;
                it.set("what", "reference").set("description", p.description.as_str());
                if !image.is_empty() {
                    it.set("image", image);
                }
                for t in &p.domain {
                    it.tag(facet::DOMAIN, t);
                }
                for t in &p.kind {
                    it.tag(facet::KIND, t);
                }
                it.contains = contains;
                it.tidy();
                let note = match lvl {
                    level::COMPONENT => "新的元件（待製作）",
                    level::MODULE => "新的模塊（待製作）",
                    _ => "新的流程圖（參考圖，待製作）",
                };
                self.changes.push(Change { op: "add".into(), item: Some(it.clone()), note: note.into(), ..Default::default() });
                it.id
            }
        }
    }

    /// Add what an existing item lacks (tags the reply brings, parts it did not list) as one `update`.
    fn link(&mut self, have: &Item, p: &Piece, contains: Vec<String>) {
        let known = self.updates.get(&have.id).copied();
        let mut it = match known {
            Some(i) => self.changes[i].item.clone().expect("update carries an item"),
            None => have.clone(),
        };
        let before = it.clone();
        let mut notes: Vec<String> = vec![];
        for (f, tags) in [(facet::DOMAIN, &p.domain), (facet::KIND, &p.kind)] {
            let have_tags = it.tags.get(f).cloned().unwrap_or_default();
            let new: Vec<&String> = tags.iter().filter(|t| !have_tags.iter().any(|h| h.eq_ignore_ascii_case(t))).collect();
            if !new.is_empty() {
                notes.push(format!("補上標籤 {}", new.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("、")));
                for t in new {
                    it.tag(f, t);
                }
            }
        }
        let added: Vec<String> = contains.into_iter().filter(|c| !it.contains.contains(c)).collect();
        if !added.is_empty() {
            notes.push(format!("加入 {} 個下一層", added.len()));
            it.contains.extend(added);
        }
        it.tidy();
        if it == before {
            return; // only linked: nothing to change
        }
        let note = notes.join("；");
        match known {
            Some(i) => {
                let ch = &mut self.changes[i];
                ch.note = format!("{}；{}", ch.note, note);
                ch.item = Some(it);
            }
            None => {
                self.updates.insert(have.id.clone(), self.changes.len());
                self.changes.push(Change { op: "update".into(), item: Some(it), note, ..Default::default() });
            }
        }
    }
}

/// The proposal for a reply: new pieces as `add` (references to make, with a temporary id
/// `new:<n>` the parents name in `contains`), existing pieces linked by their stored id (an
/// `update` only when the reply brings tags or parts they lack). Order: components, modules, figure.
/// Never an entity, never a `remove`. `picture` is the picture's file name (may be empty).
pub fn plan(reply: &Reply, existing: &[Item], category: &str, by: &str, picture: &str) -> Proposal {
    let mut pl = Planner { existing, category, by, changes: vec![], updates: BTreeMap::new(), next: 0, added: 0, linked: 0 };
    let mut comp_ids: Vec<(String, String)> = vec![]; // (key, id)
    for c in &reply.components {
        let id = pl.place(c, level::COMPONENT, vec![], "");
        comp_ids.push((key(&c.name), id));
    }
    let mut module_ids: Vec<String> = vec![];
    for m in &reply.modules {
        let parts: Vec<String> = m.components.iter().filter_map(|n| comp_ids.iter().find(|(k, _)| *k == key(n)).map(|(_, id)| id.clone())).collect();
        module_ids.push(pl.place(&m.piece, level::MODULE, parts, ""));
    }
    pl.place(&reply.figure, level::FIGURE, module_ids, picture);
    let title = format!("{}：{} 個新增、{} 個連到已有的", reply.figure.name, pl.added, pl.linked);
    Proposal { id: String::new(), by: by.into(), title, changes: pl.changes, state: "open".into(), created_at: String::new() }
}

// ── the one-line answer ──────────────────────────────────────────────────────

/// What a proposal adds, for L-Echo's one-line answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub new_components: usize,
    pub new_modules: usize,
    /// Existing items the proposal links to, counted from the changes: items updated, and stored
    /// ids named in a new item's `contains`. An existing item that is linked without any change
    /// and has no new parent is not visible here; the proposal's title has the exact count.
    pub linked: usize,
    pub new_figure: bool,
}

pub fn summary(p: &Proposal) -> Summary {
    let mut s = Summary::default();
    let mut linked: Vec<&str> = vec![];
    for c in &p.changes {
        let Some(it) = &c.item else { continue };
        match c.op.as_str() {
            "add" => {
                match it.level.as_str() {
                    level::COMPONENT => s.new_components += 1,
                    level::MODULE => s.new_modules += 1,
                    level::FIGURE => s.new_figure = true,
                    _ => {}
                }
                linked.extend(it.contains.iter().map(String::as_str).filter(|id| !id.starts_with("new:")));
            }
            "update" => linked.push(it.id.as_str()),
            _ => {}
        }
    }
    linked.sort_unstable();
    linked.dedup();
    s.linked = linked.len();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn existing(id: &str, title: &str, lvl: &str, entity: bool) -> Item {
        let mut i = Item::new("my.figs", format::FIGURE, title, Source::author());
        i.id = id.into();
        i.level = lvl.into();
        i.entity = entity;
        i.revision = 3;
        i
    }

    fn reply() -> Reply {
        parse(&json!({
            "figure": {"name": "Image classifier", "description": "A small network.", "domain": ["machine learning"], "kind": ["architecture"]},
            "modules": [
                {"name": "Encoder", "description": "Reads the image.", "domain": ["machine learning"], "components": ["Conv layer", "Pooling layer"]},
                {"name": "Head", "components": ["Linear layer"]}
            ],
            "components": [
                {"name": "Conv layer", "description": "Convolution.", "domain": ["machine learning"], "kind": ["architecture"]},
                {"name": "Pooling layer"},
                {"name": "Linear layer", "domain": ["machine learning"]}
            ]
        }))
        .unwrap()
    }

    // ── step 1 ──

    #[test]
    fn a_reply_parses_and_is_cleaned() {
        let r = parse(&json!({
            "figure": {"name": "  Pipeline ", "description": " x ", "domain": [" web ", "", "web"], "kind": null},
            "modules": [{"name": " ", "components": ["a"]}, {"name": "Client", "components": [" Browser ", ""]}],
            "components": [{"name": "Browser", "also": [" web browser "]}, {"name": ""}, {"description": "no name"}]
        }))
        .unwrap();
        assert_eq!(r.figure.name, "Pipeline");
        assert_eq!(r.figure.description, "x");
        assert_eq!(r.figure.domain, vec!["web"]);
        assert!(r.figure.kind.is_empty());
        assert_eq!(r.modules.len(), 1);
        assert_eq!(r.modules[0].components, vec!["Browser"]);
        assert_eq!(r.components.len(), 1);
        assert_eq!(r.components[0].also, vec!["web browser"]);
        let e = parse(&json!({"figure": {"name": "  "}})).unwrap_err().to_string();
        assert!(e.contains("沒有名稱"), "{e}");
        assert!(parse(&json!({"modules": []})).is_err());
        let s = schema();
        assert_eq!(s["required"][0], "figure");
        assert_eq!(s["properties"]["modules"]["items"]["required"][0], "name");
        assert_eq!(s["properties"]["components"]["items"]["required"][0], "name");
    }

    #[test]
    fn duplicate_components_are_merged() {
        let r = parse(&json!({
            "figure": {"name": "F"},
            "components": [
                {"name": "Conv layer", "description": "short", "domain": ["ml"], "kind": ["architecture"]},
                {"name": "conv-layers", "description": "a longer description", "domain": ["vision", "ml"], "also": ["CNN layer"]}
            ]
        }))
        .unwrap();
        assert_eq!(r.components.len(), 1);
        let c = &r.components[0];
        assert_eq!(c.name, "Conv layer");
        assert_eq!(c.description, "a longer description");
        assert_eq!(c.domain, vec!["ml", "vision"]);
        assert_eq!(c.kind, vec!["architecture"]);
        assert_eq!(c.also, vec!["CNN layer"]);
    }

    #[test]
    fn a_module_may_name_a_component_the_list_forgot() {
        let r = parse(&json!({
            "figure": {"name": "F"},
            "modules": [{"name": "M", "components": ["Adder", "adders", "Gate"]}],
            "components": [{"name": "Gate", "description": "d"}]
        }))
        .unwrap();
        assert_eq!(r.modules[0].components, vec!["Adder", "Gate"]);
        let adder = r.components.iter().find(|c| c.name == "Adder").unwrap();
        assert!(adder.description.is_empty());
        assert_eq!(r.components.len(), 2);
    }

    // ── step 2 ──

    #[test]
    fn the_request_lists_the_library_by_level() {
        let ex = vec![
            existing("a:1", "Encoder", level::MODULE, true),
            existing("a:2", "Conv layer", level::COMPONENT, true),
            existing("a:3", "Sketch", level::FIGURE, false),
            existing("a:4", "Old module", level::MODULE, false),
            Item::new("my.notes", format::NOTE, "Not a figure", Source::author()),
        ];
        let (sys, user) = prompt(&ex, "  from a vision paper ");
        assert!(sys.contains("JSON") && sys.contains("reuse that name"));
        let lines: Vec<&str> = user.lines().filter(|l| l.ends_with(']')).collect();
        assert_eq!(lines, vec!["Conv layer [component]", "Encoder [module]", "Old module [module]", "Sketch [figure]"]);
        assert!(!user.contains("Not a figure"));
        assert!(user.contains("from a vision paper"));
        assert!(!prompt(&ex, "").1.contains("note about"));
        let many: Vec<Item> = (0..200).map(|n| existing(&format!("a:{n}"), &format!("Part {n:03}"), level::COMPONENT, true)).collect();
        assert_eq!(prompt(&many, "").1.lines().filter(|l| l.ends_with("[component]")).count(), 150);
    }

    #[test]
    fn the_request_is_stable_for_the_same_library() {
        let a = vec![existing("a:1", "B", level::MODULE, true), existing("a:2", "A", level::MODULE, true), existing("a:3", "C", level::COMPONENT, false)];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(prompt(&a, "n"), prompt(&b, "n"));
        assert_eq!(prompt(&a, "n"), prompt(&a, "n"));
    }

    // ── step 3 ──

    fn p(name: &str, also: &[&str]) -> Piece {
        Piece { name: name.into(), also: also.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn names_match_across_case_hyphens_and_plurals() {
        assert_eq!(key("  Conv-Layers, "), "conv layer");
        assert_eq!(key("conv layer"), "conv layer");
        assert_eq!(key("LAN"), "lan");
        assert_eq!(key("Class"), "class");
        let ex = vec![existing("a:1", "Conv layer", level::COMPONENT, true), existing("a:2", "Local area network", level::COMPONENT, true)];
        assert_eq!(find(&p("CONV-LAYERS", &[]), level::COMPONENT, &ex).unwrap().id, "a:1");
        assert_eq!(find(&p("LAN", &["Local Area Networks"]), level::COMPONENT, &ex).unwrap().id, "a:2");
        assert!(find(&p("Pooling layer", &[]), level::COMPONENT, &ex).is_none());
        assert!(find(&p("", &[]), level::COMPONENT, &ex).is_none());
    }

    #[test]
    fn the_level_must_agree() {
        let ex = vec![existing("a:1", "Encoder", level::MODULE, true)];
        assert!(find(&p("Encoder", &[]), level::COMPONENT, &ex).is_none());
        assert!(find(&p("Encoder", &[]), level::MODULE, &ex).is_some());
        let mut not_fig = existing("a:2", "Gate", level::COMPONENT, true);
        not_fig.format = format::NOTE.into();
        assert!(find(&p("Gate", &[]), level::COMPONENT, &[not_fig]).is_none());
    }

    #[test]
    fn an_entity_is_preferred_to_a_reference() {
        let ex = vec![existing("a:1", "Gate", level::COMPONENT, false), existing("a:2", "gate", level::COMPONENT, true), existing("a:3", "Gates", level::COMPONENT, true)];
        assert_eq!(find(&p("Gate", &[]), level::COMPONENT, &ex).unwrap().id, "a:2");
        assert_eq!(find(&p("Gate", &[]), level::COMPONENT, &ex[..1]).unwrap().id, "a:1");
    }

    // ── step 4 ──

    #[test]
    fn new_pieces_are_references_never_entities() {
        let pr = plan(&reply(), &[], "my.figs", "le", "fig3.png");
        assert_eq!(pr.changes.len(), 6);
        assert!(pr.changes.iter().all(|c| c.op == "add" && !c.note.is_empty()));
        let levels: Vec<&str> = pr.changes.iter().map(|c| c.item.as_ref().unwrap().level.as_str()).collect();
        assert_eq!(levels, ["component", "component", "component", "module", "module", "figure"]);
        for c in &pr.changes {
            let it = c.item.as_ref().unwrap();
            assert!(!it.entity && it.field("what") == "reference" && it.format == format::FIGURE);
            assert!(it.id.starts_with("new:") && it.category == "my.figs" && it.source == Source::lore("le"));
        }
        let fig = pr.changes[5].item.as_ref().unwrap();
        assert_eq!(fig.field("image"), "fig3.png");
        assert_eq!(fig.tags[facet::DOMAIN], vec!["machine learning"]);
        assert_eq!(fig.tags[facet::KIND], vec!["architecture"]);
        assert_eq!((pr.by.as_str(), pr.state.as_str(), pr.id.as_str(), pr.created_at.as_str()), ("le", "open", "", ""));
        let ids: Vec<&str> = pr.changes.iter().map(|c| c.item.as_ref().unwrap().id.as_str()).collect();
        assert_eq!(ids, ["new:1", "new:2", "new:3", "new:4", "new:5", "new:6"]);
        assert_eq!(plan(&reply(), &[], "c", "le", "").changes[5].item.as_ref().unwrap().fields.get("image"), None);
    }

    #[test]
    fn existing_pieces_are_linked_not_duplicated() {
        let mut conv = existing("my.figs:c1", "conv layers", level::COMPONENT, true);
        conv.tag(facet::DOMAIN, "machine learning").tag(facet::KIND, "architecture");
        let pr = plan(&reply(), std::slice::from_ref(&conv), "my.figs", "le", "");
        assert_eq!(pr.changes.len(), 5);
        assert!(!pr.changes.iter().any(|c| c.item.as_ref().unwrap().title.to_lowercase().starts_with("conv")));
        let enc = pr.changes.iter().map(|c| c.item.as_ref().unwrap()).find(|i| i.title == "Encoder").unwrap();
        assert!(enc.contains.contains(&"my.figs:c1".to_string()));
        assert!(pr.changes.iter().all(|c| c.op != "remove"));
    }

    #[test]
    fn parents_contain_their_parts_by_temporary_or_stored_id() {
        let mut conv = existing("my.figs:c1", "Conv layer", level::COMPONENT, true);
        conv.tag(facet::DOMAIN, "machine learning").tag(facet::KIND, "architecture");
        let pr = plan(&reply(), &[conv], "my.figs", "le", "");
        let by = |t: &str| pr.changes.iter().map(|c| c.item.as_ref().unwrap()).find(|i| i.title == t).unwrap();
        assert_eq!(by("Encoder").contains, vec!["my.figs:c1".to_string(), by("Pooling layer").id.clone()]);
        assert_eq!(by("Head").contains, vec![by("Linear layer").id.clone()]);
        assert_eq!(by("Image classifier").contains, vec![by("Encoder").id.clone(), by("Head").id.clone()]);
        assert!(by("Pooling layer").contains.is_empty());
    }

    #[test]
    fn missing_tags_on_an_existing_item_become_an_update() {
        let mut conv = existing("my.figs:c1", "Conv layer", level::COMPONENT, true);
        conv.tag(facet::DOMAIN, "machine learning");
        let pr = plan(&reply(), &[conv.clone()], "my.figs", "le", "");
        let up: Vec<&Change> = pr.changes.iter().filter(|c| c.op == "update").collect();
        assert_eq!(up.len(), 1);
        let it = up[0].item.as_ref().unwrap();
        assert_eq!((it.id.as_str(), it.revision), ("my.figs:c1", 3));
        assert_eq!(it.tags[facet::DOMAIN], vec!["machine learning"]);
        assert_eq!(it.tags[facet::KIND], vec!["architecture"]);
        assert!(up[0].note.contains("architecture"), "{}", up[0].note);
        assert!(it.entity, "an existing entity stays an entity");
        // an existing figure of the same name gets its parts added, not a second figure
        let mut fig = existing("my.figs:f1", "image classifier", level::FIGURE, true);
        fig.contains = vec!["my.figs:old".into()];
        let pr = plan(&reply(), &[fig], "my.figs", "le", "p.png");
        assert!(!pr.changes.iter().any(|c| c.op == "add" && c.item.as_ref().unwrap().level == level::FIGURE));
        let up = pr.changes.last().unwrap();
        assert_eq!(up.op, "update");
        let c = &up.item.as_ref().unwrap().contains;
        assert_eq!(c[0], "my.figs:old");
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn the_title_counts_what_is_new_and_what_is_linked() {
        let ex = vec![existing("a:1", "Conv layer", level::COMPONENT, true), existing("a:2", "Encoder", level::MODULE, true)];
        let pr = plan(&reply(), &ex, "my.figs", "le", "");
        assert_eq!(pr.title, "Image classifier：4 個新增、2 個連到已有的");
    }

    // ── step 5 ──

    #[test]
    fn the_summary_counts_by_level() {
        let s = summary(&plan(&reply(), &[], "c", "le", ""));
        assert_eq!(s, Summary { new_components: 3, new_modules: 2, linked: 0, new_figure: true });
        let ex = vec![existing("a:1", "Conv layer", level::COMPONENT, true), existing("a:2", "Encoder", level::MODULE, true)];
        let s = summary(&plan(&reply(), &ex, "c", "le", ""));
        assert_eq!((s.new_components, s.new_modules, s.new_figure), (2, 1, true));
        assert_eq!(s.linked, 2, "{s:?}");
    }
}
