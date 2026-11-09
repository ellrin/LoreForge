//! Editing operations: small JSON-patch-like commands with exact inverses,
//! grouped into transactions for undo/redo. Every op is validated against the
//! style rules (tags, palette) before it is committed; a failing transaction
//! leaves the deck unchanged.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::geom;
use crate::model::*;
use crate::style::{RuleError, StyleSystem};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Replace the value at `path` (JSON pointer inside the object; "" = whole object).
    SetObj {
        slide: String,
        id: u32,
        #[serde(default)]
        path: String,
        value: Value,
    },
    /// Insert an object into the slide (or into group `parent`) at `index` (z-order, 0 = back).
    InsertObj {
        slide: String,
        #[serde(default)]
        parent: Option<u32>,
        index: usize,
        obj: Object,
    },
    DeleteObj { slide: String, id: u32 },
    /// Move an object to another z-index within its parent.
    MoveObj { slide: String, id: u32, index: usize },
    /// Replace a slide field (`/hidden`, `/notes`, `/anims`, `/layout`).
    SetSlide {
        slide: String,
        path: String,
        value: Value,
    },
    InsertSlide { index: usize, slide: Slide },
    DeleteSlide { slide: String },
    MoveSlide { slide: String, index: usize },
    /// Macro: group these top-level objects into a new group with id `id`.
    Group {
        slide: String,
        ids: Vec<u32>,
        #[serde(default)]
        id: Option<u32>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Macro: dissolve a top-level group, children go to slide space.
    Ungroup { slide: String, id: u32 },
}

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("找不到投影片 {0}")]
    NoSlide(String),
    #[error("找不到物件 {0}")]
    NoObject(u32),
    #[error("路徑無效：{0}")]
    BadPath(String),
    #[error("資料格式錯誤：{0}")]
    Bad(String),
    #[error("{0}")]
    Rule(#[from] RuleError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Txn {
    pub label: String,
    pub ops: Vec<Op>,
    pub inverse: Vec<Op>,
}

/// What a transaction touched (so the UI refreshes only those slides).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Applied {
    pub slides: Vec<String>,
    /// Slide list changed (insert/delete/move).
    pub structure: bool,
    pub label: String,
}

impl Applied {
    fn touch(&mut self, uid: &str) {
        if !self.slides.iter().any(|s| s == uid) {
            self.slides.push(uid.to_string());
        }
    }
}

#[derive(Debug, Clone)]
pub struct Editor {
    pub deck: Deck,
    pub style: Option<StyleSystem>,
    pub undo: Vec<Txn>,
    pub redo: Vec<Txn>,
    /// Bumped on every change (apply / undo / redo).
    pub rev: u64,
    pub limit: usize,
}

fn set_ptr(root: &mut Value, path: &str, value: Value) -> Result<Value, OpError> {
    if path.is_empty() {
        return Ok(std::mem::replace(root, value));
    }
    // allow creating a missing last key in an object
    let (parent, last) = path
        .rsplit_once('/')
        .ok_or_else(|| OpError::BadPath(path.into()))?;
    let last = last.replace("~1", "/").replace("~0", "~");
    let p = if parent.is_empty() {
        Some(root)
    } else {
        root.pointer_mut(parent)
    }
    .ok_or_else(|| OpError::BadPath(path.into()))?;
    match p {
        Value::Object(m) => Ok(m.insert(last, value).unwrap_or(Value::Null)),
        Value::Array(a) => {
            let i: usize = last.parse().map_err(|_| OpError::BadPath(path.into()))?;
            let slot = a.get_mut(i).ok_or_else(|| OpError::BadPath(path.into()))?;
            Ok(std::mem::replace(slot, value))
        }
        _ => Err(OpError::BadPath(path.into())),
    }
}

impl Editor {
    pub fn new(deck: Deck, style: Option<StyleSystem>) -> Self {
        Editor {
            deck,
            style,
            undo: vec![],
            redo: vec![],
            rev: 0,
            limit: 200,
        }
    }

    fn slide_mut(&mut self, uid: &str) -> Result<&mut Slide, OpError> {
        self.deck
            .slide_mut(uid)
            .ok_or_else(|| OpError::NoSlide(uid.into()))
    }

    /// Expand macros (group/ungroup) into primitive ops against the current state.
    fn expand(&self, op: Op) -> Result<Vec<Op>, OpError> {
        match op {
            Op::Group {
                slide,
                ids,
                id,
                name,
            } => {
                let s = self
                    .deck
                    .slide(&slide)
                    .ok_or_else(|| OpError::NoSlide(slide.clone()))?;
                let mut picked: Vec<(usize, Object)> = Vec::new();
                for i in &ids {
                    let (parent, idx) = s.locate(*i).ok_or(OpError::NoObject(*i))?;
                    if parent.is_some() {
                        return Err(OpError::Bad("只能群組最上層物件".into()));
                    }
                    picked.push((idx, s.objects[idx].clone()));
                }
                if picked.len() < 2 {
                    return Err(OpError::Bad("至少選兩個物件才能群組".into()));
                }
                picked.sort_by_key(|(i, _)| *i);
                let top = picked.last().map(|(i, _)| *i).unwrap_or(0);
                let children: Vec<Object> = picked.iter().map(|(_, o)| o.clone()).collect();
                let b = geom::bounds(children.iter().map(|o| &o.frame));
                let gid = id.unwrap_or_else(|| s.max_id() + 1);
                let g = Object {
                    id: gid,
                    name: name.unwrap_or_else(|| format!("群組 {gid}")),
                    frame: b.clone(),
                    hidden: false,
                    ph: None,
                    kind: Kind::Group(GroupObj { ch: b, children }),
                    src: None,
                };
                let mut out: Vec<Op> = ids
                    .iter()
                    .map(|i| Op::DeleteObj {
                        slide: slide.clone(),
                        id: *i,
                    })
                    .collect();
                out.push(Op::InsertObj {
                    slide,
                    parent: None,
                    index: top + 1 - picked.len(),
                    obj: g,
                });
                Ok(out)
            }
            Op::Ungroup { slide, id } => {
                let s = self
                    .deck
                    .slide(&slide)
                    .ok_or_else(|| OpError::NoSlide(slide.clone()))?;
                let (parent, idx) = s.locate(id).ok_or(OpError::NoObject(id))?;
                if parent.is_some() {
                    return Err(OpError::Bad("只能解散最上層群組".into()));
                }
                let o = &s.objects[idx];
                let Kind::Group(g) = &o.kind else {
                    return Err(OpError::Bad("不是群組".into()));
                };
                let mut out = vec![Op::DeleteObj {
                    slide: slide.clone(),
                    id,
                }];
                for (j, c) in g.children.iter().enumerate() {
                    let mut c = c.clone();
                    geom::child_to_parent(&o.frame, &g.ch, &mut c);
                    out.push(Op::InsertObj {
                        slide: slide.clone(),
                        parent: None,
                        index: idx + j,
                        obj: c,
                    });
                }
                Ok(out)
            }
            op => Ok(vec![op]),
        }
    }

    fn check(&self, old: Option<&Value>, new: &Value) -> Result<(), OpError> {
        if let Some(st) = &self.style {
            st.check_change(old, new)?;
        }
        Ok(())
    }

    /// Apply one primitive op; returns its inverse.
    fn apply_one(&mut self, op: &Op, a: &mut Applied, validate: bool) -> Result<Op, OpError> {
        match op {
            Op::SetObj {
                slide,
                id,
                path,
                value,
            } => {
                let s = self.slide_mut(slide)?;
                let o = s.find_mut(*id).ok_or(OpError::NoObject(*id))?;
                let mut v = serde_json::to_value(&*o).map_err(|e| OpError::Bad(e.to_string()))?;
                let before = if validate { Some(v.clone()) } else { None };
                let old = set_ptr(&mut v, path, value.clone())?;
                if let Some(b) = &before {
                    self.check(Some(b), &v)?;
                }
                let mut no: Object =
                    serde_json::from_value(v).map_err(|e| OpError::Bad(e.to_string()))?;
                no.id = *id;
                let o = self.slide_mut(slide)?.find_mut(*id).unwrap();
                *o = no;
                a.touch(slide);
                Ok(Op::SetObj {
                    slide: slide.clone(),
                    id: *id,
                    path: path.clone(),
                    value: old,
                })
            }
            Op::InsertObj {
                slide,
                parent,
                index,
                obj,
            } => {
                if validate {
                    let v = serde_json::to_value(obj).map_err(|e| OpError::Bad(e.to_string()))?;
                    self.check(None, &v)?;
                }
                let s = self.slide_mut(slide)?;
                if s.find(obj.id).is_some() {
                    return Err(OpError::Bad(format!("物件 id {} 重複", obj.id)));
                }
                let list = s
                    .list_mut(*parent)
                    .ok_or(OpError::NoObject(parent.unwrap_or(0)))?;
                let i = (*index).min(list.len());
                list.insert(i, obj.clone());
                a.touch(slide);
                Ok(Op::DeleteObj {
                    slide: slide.clone(),
                    id: obj.id,
                })
            }
            Op::DeleteObj { slide, id } => {
                let s = self.slide_mut(slide)?;
                let (parent, idx) = s.locate(*id).ok_or(OpError::NoObject(*id))?;
                let list = s.list_mut(parent).unwrap();
                let obj = list.remove(idx);
                a.touch(slide);
                Ok(Op::InsertObj {
                    slide: slide.clone(),
                    parent,
                    index: idx,
                    obj,
                })
            }
            Op::MoveObj { slide, id, index } => {
                let s = self.slide_mut(slide)?;
                let (parent, idx) = s.locate(*id).ok_or(OpError::NoObject(*id))?;
                let list = s.list_mut(parent).unwrap();
                let o = list.remove(idx);
                let to = (*index).min(list.len());
                list.insert(to, o);
                a.touch(slide);
                Ok(Op::MoveObj {
                    slide: slide.clone(),
                    id: *id,
                    index: idx,
                })
            }
            Op::SetSlide { slide, path, value } => {
                let s = self.slide_mut(slide)?;
                let mut v = serde_json::to_value(&*s).map_err(|e| OpError::Bad(e.to_string()))?;
                if path.is_empty() || path.starts_with("/objects") || path.starts_with("/uid") {
                    return Err(OpError::BadPath(path.clone()));
                }
                let old = set_ptr(&mut v, path, value.clone())?;
                let ns: Slide = serde_json::from_value(v).map_err(|e| OpError::Bad(e.to_string()))?;
                *self.slide_mut(slide)? = ns;
                a.touch(slide);
                Ok(Op::SetSlide {
                    slide: slide.clone(),
                    path: path.clone(),
                    value: old,
                })
            }
            Op::InsertSlide { index, slide } => {
                if self.deck.slide(&slide.uid).is_some() {
                    return Err(OpError::Bad(format!("投影片 {} 已存在", slide.uid)));
                }
                if validate {
                    let v = serde_json::to_value(slide).map_err(|e| OpError::Bad(e.to_string()))?;
                    self.check(None, &v)?;
                }
                // keep uid counter ahead of any inserted uid
                if let Some(n) = slide.uid.strip_prefix('s').and_then(|n| n.parse::<u64>().ok()) {
                    self.deck.next_uid = self.deck.next_uid.max(n + 1);
                }
                let i = (*index).min(self.deck.slides.len());
                self.deck.slides.insert(i, slide.clone());
                a.structure = true;
                a.touch(&slide.uid);
                Ok(Op::DeleteSlide {
                    slide: slide.uid.clone(),
                })
            }
            Op::DeleteSlide { slide } => {
                let i = self
                    .deck
                    .slide_index(slide)
                    .ok_or_else(|| OpError::NoSlide(slide.clone()))?;
                let s = self.deck.slides.remove(i);
                a.structure = true;
                Ok(Op::InsertSlide { index: i, slide: s })
            }
            Op::MoveSlide { slide, index } => {
                let i = self
                    .deck
                    .slide_index(slide)
                    .ok_or_else(|| OpError::NoSlide(slide.clone()))?;
                let s = self.deck.slides.remove(i);
                let to = (*index).min(self.deck.slides.len());
                self.deck.slides.insert(to, s);
                a.structure = true;
                Ok(Op::MoveSlide {
                    slide: slide.clone(),
                    index: i,
                })
            }
            Op::Group { .. } | Op::Ungroup { .. } => Err(OpError::Bad("macro not expanded".into())),
        }
    }

    fn run(&mut self, ops: &[Op], a: &mut Applied, validate: bool) -> Result<Vec<Op>, OpError> {
        let mut inverse = Vec::new();
        for op in ops {
            match self.apply_one(op, a, validate) {
                Ok(inv) => inverse.push(inv),
                Err(e) => {
                    // roll back
                    let mut tmp = Applied::default();
                    for inv in inverse.iter().rev() {
                        let _ = self.apply_one(inv, &mut tmp, false);
                    }
                    return Err(e);
                }
            }
        }
        Ok(inverse)
    }

    /// Apply a transaction (validated, atomic). Connectors attached to moved
    /// shapes are re-routed inside the same transaction.
    pub fn apply(&mut self, ops: Vec<Op>, label: &str) -> Result<Applied, OpError> {
        let mut prim = Vec::new();
        for op in ops {
            // expand against the state after the previous ops: apply as we go
            prim.push(op);
        }
        let mut a = Applied {
            label: label.into(),
            ..Default::default()
        };
        let mut done_ops = Vec::new();
        let mut inverse: Vec<Op> = Vec::new();
        for op in prim {
            let ex = match self.expand(op) {
                Ok(x) => x,
                Err(e) => {
                    let mut tmp = Applied::default();
                    for inv in inverse.iter().rev() {
                        let _ = self.apply_one(inv, &mut tmp, false);
                    }
                    return Err(e);
                }
            };
            match self.run(&ex, &mut a, true) {
                Ok(inv) => {
                    inverse.extend(inv);
                    done_ops.extend(ex);
                }
                Err(e) => {
                    let mut tmp = Applied::default();
                    for inv in inverse.iter().rev() {
                        let _ = self.apply_one(inv, &mut tmp, false);
                    }
                    return Err(e);
                }
            }
        }
        // connector re-routing (connectors attached to moved/resized objects)
        let mut moved: std::collections::BTreeMap<String, std::collections::BTreeSet<u32>> = Default::default();
        for op in &done_ops {
            match op {
                Op::SetObj { slide, id, path, .. } if path.is_empty() || path.starts_with("/frame") => {
                    moved.entry(slide.clone()).or_default().insert(*id);
                }
                Op::SetObj { slide, value, path, .. } if path.starts_with("/start") || path.starts_with("/end") => {
                    if let Some(id) = value.get("id").and_then(|v| v.as_u64()) {
                        moved.entry(slide.clone()).or_default().insert(id as u32);
                    }
                }
                Op::InsertObj { slide, obj, .. } => {
                    moved.entry(slide.clone()).or_default().insert(obj.id);
                    if let Kind::Connector(c) = &obj.kind {
                        for a in [&c.start, &c.end].into_iter().flatten() {
                            moved.entry(slide.clone()).or_default().insert(a.id);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut extra = Vec::new();
        for (uid, ids) in &moved {
            if let Some(s) = self.deck.slide(uid) {
                for (id, f) in geom::reroute(s, ids) {
                    if ids.contains(&id) && !matches!(s.find(id).map(|o| &o.kind), Some(Kind::Connector(_))) {
                        continue;
                    }
                    extra.push(Op::SetObj {
                        slide: uid.clone(),
                        id,
                        path: "/frame".into(),
                        value: serde_json::to_value(f).unwrap(),
                    });
                }
            }
        }
        if !extra.is_empty() {
            let inv = self.run(&extra, &mut a, false)?;
            inverse.extend(inv);
            done_ops.extend(extra);
        }
        inverse.reverse();
        self.undo.push(Txn {
            label: label.into(),
            ops: done_ops,
            inverse,
        });
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.rev += 1;
        Ok(a)
    }

    pub fn undo(&mut self) -> Option<Applied> {
        let t = self.undo.pop()?;
        let mut a = Applied {
            label: t.label.clone(),
            ..Default::default()
        };
        match self.run(&t.inverse, &mut a, false) {
            Ok(_) => {
                self.redo.push(t);
                self.rev += 1;
                Some(a)
            }
            Err(_) => None,
        }
    }

    pub fn redo(&mut self) -> Option<Applied> {
        let t = self.redo.pop()?;
        let mut a = Applied {
            label: t.label.clone(),
            ..Default::default()
        };
        match self.run(&t.ops, &mut a, false) {
            Ok(_) => {
                self.undo.push(t);
                self.rev += 1;
                Some(a)
            }
            Err(_) => None,
        }
    }

    pub fn can_undo(&self) -> Option<&str> {
        self.undo.last().map(|t| t.label.as_str())
    }
    pub fn can_redo(&self) -> Option<&str> {
        self.redo.last().map(|t| t.label.as_str())
    }
}
