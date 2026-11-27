//! What the one who draws says about a figure's placement, as a closed set of commands. Each is
//! a relation between named objects (A beside B, A in B's row, A over the columns from B to C,
//! a mark tied to a block), never a coordinate: [`apply`] works out the cell, the span, the tie,
//! and writes only what `pack`, `settle` and `lines` already read (`col`, `lane`, `span`,
//! `tall`, `stretch`, `by`, `side`, a line's sides, a break in a label). An agent or a small
//! model has one job here: to turn a sentence into one of these with the right names.
//!
//! A command answers with the figure and what it changed, in words; or with a [`Clash`] that
//! says why it cannot be done and who is in the way (an unknown name comes back with the names
//! nearest to it). What a command places is marked `stated`, and `settle` leaves it alone.

use crate::pack;
use crate::spec::{Graph, Node};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Way {
    Left,
    Right,
    Above,
    Below,
}

impl Way {
    fn word(self) -> &'static str {
        match self {
            Way::Left => "left",
            Way::Right => "right",
            Way::Above => "above",
            Way::Below => "below",
        }
    }
    fn zh(self) -> &'static str {
        match self {
            Way::Left => "左邊",
            Way::Right => "右邊",
            Way::Above => "上方",
            Way::Below => "下方",
        }
    }
    /// As a line's side is named.
    fn side(self) -> &'static str {
        match self {
            Way::Left => "left",
            Way::Right => "right",
            Way::Above => "top",
            Way::Below => "bottom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Axis {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fill {
    Wide,
    Tall,
    Both,
    None,
}

/// One thing said about the placement. `do` names it in its written form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "kebab-case")]
pub enum Command {
    /// A stands next to B on that side: in the neighbouring cell when it is free, else in a
    /// column (or row) of its own between B and what stands there.
    Beside { a: String, of: String, side: Way },
    /// A stands in B's row, or in B's column.
    Align { a: String, with: String, axis: Axis },
    /// A (a heading, a bracket) lies over the columns from one object to another.
    Over { a: String, from: String, to: String },
    /// A stands beside the rows from one object to another, in their middle; `fill` = as tall
    /// as those rows.
    BesideRows {
        a: String,
        from: String,
        to: String,
        #[serde(default)]
        fill: bool,
    },
    /// A mark (a loss, a return arrow, a note) belongs to a block: it is written beside it, on
    /// that side when one is said, and takes no cell.
    Tie {
        mark: String,
        to: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        side: Option<Way>,
    },
    /// A and B change places.
    Swap { a: String, b: String },
    /// A stands in the column and row said, as the figure's text counts them.
    Cell { a: String, col: f64, row: f64 },
    /// A takes the width of its columns, the height of its rows, both, or its own size again.
    Stretch { a: String, way: Fill },
    /// A's words break before this word (the only break they have).
    Break { a: String, before: String },
    /// A's words stand on one line.
    OneLine { a: String },
    /// The line from one block to another leaves, or arrives, on the side said.
    LineSide {
        from: String,
        to: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        leave: Option<Way>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arrive: Option<Way>,
    },
    /// What was said about A is taken back (its tie, span, height, stretch): the figure's own
    /// structure settles it again.
    Free { a: String },
}

/// Why a command cannot be done.
#[derive(Debug, Clone, PartialEq)]
pub struct Clash {
    /// In words, for the one who said it.
    pub why: String,
    /// The objects in the way, or the names nearest to one that is not known.
    pub who: Vec<String>,
}

fn clash(why: String, who: Vec<String>) -> Clash {
    Clash { why, who }
}

fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.to_lowercase().chars().collect(), b.to_lowercase().chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut prev = row[0];
        row[0] = i;
        for j in 1..=b.len() {
            let cur = row[j];
            row[j] = (prev + (a[i - 1] != b[j - 1]) as usize).min(row[j] + 1).min(row[j - 1] + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

fn find(g: &Graph, id: &str) -> Result<usize, Clash> {
    g.node_index(id).ok_or_else(|| {
        // the names nearest to it: by its own name, or by the words written on the object
        let mut near: Vec<(usize, String)> = g.nodes.iter().map(|n| (distance(id, &n.id).min(if n.label.to_lowercase().contains(&id.to_lowercase()) { 1 } else { usize::MAX }), n.id.clone())).collect();
        near.sort();
        let who: Vec<String> = near.into_iter().take(4).map(|x| x.1).collect();
        clash(format!("沒有叫 {id} 的物件，最接近的是 {}", who.join("、")), who)
    })
}

fn cell(n: &Node) -> Result<(f64, f64), Clash> {
    match (n.num("col"), n.num("lane")) {
        (Some(c), Some(l)) => Ok((c, l)),
        _ => Err(clash(format!("{} 沒有寫明欄和列，無法依它定位", n.id), vec![n.id.clone()])),
    }
}

/// The columns and rows a block takes: (first column, last column, first row, last row).
fn reach(n: &Node) -> Result<(f64, f64, f64, f64), Clash> {
    let (c, l) = cell(n)?;
    Ok((c, c + n.num("span").unwrap_or(1.0).max(1.0) - 1.0, l, l + n.num("tall").unwrap_or(1.0).max(1.0) - 1.0))
}

fn object(g: &Graph, i: usize) -> bool {
    !pack::is_note(g, i) && g.nodes[i].num("col").is_some() && g.nodes[i].num("lane").is_some()
}

fn same_panel(g: &Graph, ids: &[usize]) -> Result<(), Clash> {
    let p = &g.nodes[ids[0]].panel;
    match ids.iter().find(|i| &g.nodes[**i].panel != p) {
        Some(i) => Err(clash(format!("{} 和 {} 不在同一個區塊，欄列各算各的", g.nodes[ids[0]].id, g.nodes[*i].id), vec![g.nodes[ids[0]].id.clone(), g.nodes[*i].id.clone()])),
        None => Ok(()),
    }
}

/// The objects of `i`'s area that take a cell `i` takes.
fn in_the_way(g: &Graph, i: usize) -> Vec<String> {
    let Ok((c0, c1, l0, l1)) = reach(&g.nodes[i]) else { return Vec::new() };
    if !object(g, i) {
        return Vec::new();
    }
    (0..g.nodes.len())
        .filter(|j| *j != i && object(g, *j) && g.nodes[*j].panel == g.nodes[i].panel)
        .filter(|j| reach(&g.nodes[*j]).is_ok_and(|(x0, x1, y0, y1)| x0 <= c1 + 1e-6 && c0 <= x1 + 1e-6 && y0 <= l1 + 1e-6 && l0 <= y1 + 1e-6))
        .map(|j| g.nodes[j].id.clone())
        .collect()
}

fn checked(g: Graph, moved: &[usize], said: Vec<String>) -> Result<(Graph, Vec<String>), Clash> {
    for &i in moved {
        let who = in_the_way(&g, i);
        if !who.is_empty() {
            let (c0, c1, l0, l1) = reach(&g.nodes[i])?;
            let span = |a: f64, b: f64| if (a - b).abs() < 1e-6 { format!("{a}") } else { format!("{a}–{b}") };
            return Err(clash(format!("{} 要站的位置（第 {} 欄、第 {} 列）已經有 {}", g.nodes[i].id, span(c0, c1), span(l0, l1), who.join("、")), who));
        }
    }
    Ok((g, said))
}

fn put(n: &mut Node, k: &str, v: serde_json::Value) {
    n.params.insert(k.into(), v);
    n.params.insert("stated".into(), json!(true));
}

/// Does what the command says; the figure is returned with what changed, in words.
pub fn apply(g: &Graph, c: &Command) -> Result<(Graph, Vec<String>), Clash> {
    let mut g2 = g.clone();
    match c {
        Command::Beside { a, of, side } => {
            let (i, j) = (find(g, a)?, find(g, of)?);
            same_panel(g, &[i, j])?;
            let (c0, c1, l0, l1) = reach(&g.nodes[j])?;
            cell(&g.nodes[i])?;
            let across = matches!(side, Way::Left | Way::Right);
            let before = matches!(side, Way::Left | Way::Above);
            // the next column (or row) on that side that the area already has, else one step out
            let key = if across { "col" } else { "lane" };
            let edge = match side {
                Way::Left => c0,
                Way::Right => c1,
                Way::Above => l0,
                Way::Below => l1,
            };
            let next = (0..g.nodes.len())
                .filter(|k| *k != i && object(g, *k) && g.nodes[*k].panel == g.nodes[j].panel)
                .filter_map(|k| g.nodes[k].num(key))
                .filter(|v| if before { *v < edge - 1e-6 } else { *v > edge + 1e-6 })
                .min_by(|x, y| (x - edge).abs().partial_cmp(&(y - edge).abs()).unwrap());
            let step = if before { -1.0 } else { 1.0 };
            let mut tries: Vec<f64> = Vec::new();
            match next {
                // never further than the next stated column: beside means next to
                Some(v) if (v - edge).abs() <= 1.0 + 1e-6 => {
                    tries.push(v);
                    tries.push((v + edge) / 2.0);
                }
                Some(v) => {
                    tries.push(edge + step);
                    tries.push((v + edge) / 2.0);
                }
                None => tries.push(edge + step),
            }
            let other = if across { ("lane", l0) } else { ("col", c0) };
            let mut last: Option<Clash> = None;
            for t in tries {
                let mut g3 = g.clone();
                let n = &mut g3.nodes[i];
                n.params.remove("by");
                n.params.remove("side");
                // its own width (or height) ends at the neighbour when it stands before it
                let own = n.num(if across { "span" } else { "tall" }).unwrap_or(1.0).max(1.0);
                put(n, key, json!(if before { t - (own - 1.0) } else { t }));
                put(n, other.0, json!(other.1));
                let said = vec![format!("{a}：站到 {of} 的{}（第 {} 欄、第 {} 列）", side.zh(), n.num("col").unwrap(), n.num("lane").unwrap())];
                match checked(g3, &[i], said) {
                    Ok(r) => return Ok(r),
                    Err(e) => last = Some(e),
                }
            }
            Err(last.unwrap())
        }
        Command::Align { a, with, axis } => {
            let (i, j) = (find(g, a)?, find(g, with)?);
            same_panel(g, &[i, j])?;
            let (c, l) = cell(&g.nodes[j])?;
            cell(&g.nodes[i])?;
            let said = match axis {
                Axis::Row => {
                    put(&mut g2.nodes[i], "lane", json!(l));
                    match g.nodes[j].num("tall") {
                        Some(t) => put(&mut g2.nodes[i], "tall", json!(t)),
                        None => {
                            g2.nodes[i].params.remove("tall");
                        }
                    }
                    format!("{a}：和 {with} 同一列（第 {l} 列）")
                }
                Axis::Column => {
                    put(&mut g2.nodes[i], "col", json!(c));
                    format!("{a}：和 {with} 同一欄（第 {c} 欄）")
                }
            };
            checked(g2, &[i], vec![said])
        }
        Command::Over { a, from, to } => {
            let (i, j, k) = (find(g, a)?, find(g, from)?, find(g, to)?);
            same_panel(g, &[i, j, k])?;
            let (p, q) = (reach(&g.nodes[j])?, reach(&g.nodes[k])?);
            cell(&g.nodes[i])?;
            let (c0, c1) = (p.0.min(q.0), p.1.max(q.1));
            put(&mut g2.nodes[i], "col", json!(c0));
            put(&mut g2.nodes[i], "span", json!(c1 - c0 + 1.0));
            checked(g2, &[i], vec![format!("{a}：蓋在第 {c0}–{c1} 欄上（{from} 到 {to}）")])
        }
        Command::BesideRows { a, from, to, fill } => {
            let (i, j, k) = (find(g, a)?, find(g, from)?, find(g, to)?);
            same_panel(g, &[i, j, k])?;
            let (p, q) = (reach(&g.nodes[j])?, reach(&g.nodes[k])?);
            cell(&g.nodes[i])?;
            let (l0, l1) = (p.2.min(q.2), p.3.max(q.3));
            put(&mut g2.nodes[i], "lane", json!(l0));
            put(&mut g2.nodes[i], "tall", json!(l1 - l0 + 1.0));
            if *fill {
                put(&mut g2.nodes[i], "stretch", json!("tall"));
            }
            checked(g2, &[i], vec![format!("{a}：站在第 {l0}–{l1} 列旁邊（{from} 到 {to}）{}", if *fill { "，跟那幾列一樣高" } else { "" })])
        }
        Command::Tie { mark, to, side } => {
            let (i, j) = (find(g, mark)?, find(g, to)?);
            if i == j {
                return Err(clash(format!("{mark} 不能貼著自己"), vec![mark.clone()]));
            }
            // a block something is tied to is not itself tied back to it
            let mut at = j;
            for _ in 0..g.nodes.len() {
                match g.nodes[at].str("by").and_then(|b| g.node_index(b)) {
                    Some(b) if b == i => return Err(clash(format!("{to} 已經貼著 {mark}，不能互相貼"), vec![to.clone(), mark.clone()])),
                    Some(b) => at = b,
                    None => break,
                }
            }
            if g.edges.iter().any(|e| e.kind == crate::spec::EdgeKind::Data && (&e.from == mark || &e.to == mark)) {
                return Err(clash(format!("{mark} 在主線上，是物件不是標記"), vec![mark.clone()]));
            }
            put(&mut g2.nodes[i], "by", json!(to));
            match side {
                Some(s) => put(&mut g2.nodes[i], "side", json!(s.word())),
                None => {
                    g2.nodes[i].params.remove("side");
                }
            }
            Ok((g2, vec![format!("{mark}：貼在 {to} 的{}，不佔格子", side.map(|s| s.zh()).unwrap_or("旁邊"))]))
        }
        Command::Swap { a, b } => {
            let (i, j) = (find(g, a)?, find(g, b)?);
            same_panel(g, &[i, j])?;
            let (p, q) = (cell(&g.nodes[i])?, cell(&g.nodes[j])?);
            for (n, at) in [(i, q), (j, p)] {
                put(&mut g2.nodes[n], "col", json!(at.0));
                put(&mut g2.nodes[n], "lane", json!(at.1));
            }
            checked(g2, &[i, j], vec![format!("{a} 和 {b}：交換位置")])
        }
        Command::Cell { a, col, row } => {
            let i = find(g, a)?;
            let n = &mut g2.nodes[i];
            n.params.remove("by");
            n.params.remove("side");
            put(n, "col", json!(col));
            put(n, "lane", json!(row));
            checked(g2, &[i], vec![format!("{a}：第 {col} 欄、第 {row} 列")])
        }
        Command::Stretch { a, way } => {
            let i = find(g, a)?;
            let said = match way {
                Fill::None => {
                    g2.nodes[i].params.remove("stretch");
                    g2.nodes[i].params.insert("stated".into(), json!(true));
                    format!("{a}：回到自己的大小")
                }
                Fill::Wide => {
                    put(&mut g2.nodes[i], "stretch", json!("wide"));
                    format!("{a}：跟所佔的欄一樣寬")
                }
                Fill::Tall => {
                    put(&mut g2.nodes[i], "stretch", json!("tall"));
                    format!("{a}：跟所佔的列一樣高")
                }
                Fill::Both => {
                    put(&mut g2.nodes[i], "stretch", json!("both"));
                    format!("{a}：填滿所佔的欄和列")
                }
            };
            Ok((g2, vec![said]))
        }
        Command::Break { a, before } => {
            let i = find(g, a)?;
            let flat = g.nodes[i].label.replace("\\\\", " ");
            // the word's first place outside maths, at the start of a word
            let mut maths = false;
            let mut found: Option<usize> = None;
            for (k, ch) in flat.char_indices() {
                if ch == '$' {
                    maths = !maths;
                }
                if !maths && k > 0 && flat[k..].starts_with(before.as_str()) && flat[..k].ends_with(' ') {
                    found = Some(k);
                    break;
                }
            }
            let Some(k) = found else {
                return Err(clash(format!("{a} 的字（{flat}）裡沒有可以在「{before}」前斷行的地方"), vec![a.clone()]));
            };
            g2.nodes[i].label = format!("{}\\\\{}", flat[..k].trim_end(), &flat[k..]);
            g2.nodes[i].params.insert("stated".into(), json!(true));
            Ok((g2, vec![format!("{a}：在「{before}」前斷行")]))
        }
        Command::OneLine { a } => {
            let i = find(g, a)?;
            g2.nodes[i].label = g.nodes[i].label.replace("\\\\", " ");
            g2.nodes[i].params.insert("stated".into(), json!(true));
            Ok((g2, vec![format!("{a}：字寫成一行")]))
        }
        Command::LineSide { from, to, leave, arrive } => {
            find(g, from)?;
            find(g, to)?;
            let Some(e) = g.edges.iter().position(|e| &e.from == from && &e.to == to) else {
                let who: Vec<String> = g.edges.iter().filter(|e| &e.from == from || &e.to == from || &e.from == to || &e.to == to).map(|e| format!("{} → {}", e.from, e.to)).collect();
                return Err(clash(format!("沒有從 {from} 到 {to} 的線；和它們有關的線：{}", if who.is_empty() { "無".to_string() } else { who.join("、") }), who));
            };
            if leave.is_none() && arrive.is_none() {
                g2.edges[e].from_side.clear();
                g2.edges[e].to_side.clear();
                return Ok((g2, vec![format!("{from} → {to}：兩端的邊交還給程式決定")]));
            }
            let mut said = Vec::new();
            if let Some(s) = leave {
                g2.edges[e].from_side = s.side().into();
                said.push(format!("{from} → {to}：從 {from} 的{}出去", s.zh()));
            }
            if let Some(s) = arrive {
                g2.edges[e].to_side = s.side().into();
                said.push(format!("{from} → {to}：從 {to} 的{}進去", s.zh()));
            }
            Ok((g2, said))
        }
        Command::Free { a } => {
            let i = find(g, a)?;
            let had: Vec<&str> = ["by", "side", "span", "tall", "stretch", "stated"].into_iter().filter(|k| g2.nodes[i].params.remove(*k).is_some()).collect();
            if had.is_empty() {
                return Ok((g2, vec![format!("{a}：沒有說過什麼，不變")]));
            }
            checked(g2, &[i], vec![format!("{a}：收回說過的（{}），交還給結構決定", had.join("、"))])
        }
    }
}

/// Several commands in the order said. One that cannot be done is left out and its reason kept;
/// the others still apply.
pub fn apply_all(g: &Graph, cs: &[Command]) -> (Graph, Vec<String>, Vec<Clash>) {
    let (mut g, mut said, mut not) = (g.clone(), Vec::new(), Vec::new());
    for c in cs {
        match apply(&g, c) {
            Ok((n, s)) => {
                g = n;
                said.extend(s);
            }
            Err(e) => not.push(e),
        }
    }
    (g, said, not)
}
