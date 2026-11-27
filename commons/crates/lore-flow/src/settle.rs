//! What a figure's structure settles by itself, before anyone states anything: placement that
//! follows from who is joined to whom, what kind a thing is and where it was put on the grid,
//! never from how one particular figure happened to be laid out. Each rule leaves alone
//! whatever is already stated, and says what it settled.
//!
//! 1. A small thing nothing is joined to (a return arrow, a loss no line reaches) belongs to
//!    the nearest module: it is tied to it and takes no cell.
//! 2. A module joined to modules that stand in one column in several rows (parallel rows of
//!    the flow) stands beside those rows, in their middle; a module in its row joined to it by a data line stands there with it.
//! 3. In a row of things no line touches (headings, brackets) each lies over the columns nearer
//!    to it than to its neighbours in that row.
//! 4. A bracket is as wide as the columns it lies over; a brace as tall as its rows.
//! 5. A box or a line of words longer than [`LONG`] letters breaks in two near its middle.

use crate::pack;
use crate::spec::{EdgeKind, Graph, Node};
use serde_json::json;

/// The length of words (letters) over which a box's or a label's words are broken in two.
pub const LONG: usize = 22;

fn at(n: &Node) -> Option<(f64, f64)> {
    Some((n.num("col")?, n.num("lane")?))
}

/// The figure with what its structure settles written into it, and what was settled.
pub fn settle(g: &Graph) -> (Graph, Vec<String>) {
    let mut g = g.clone();
    let mut said: Vec<String> = Vec::new();
    let down = |g: &Graph, panel: &str| g.panels.iter().any(|p| p.id == panel && p.dir == "down");
    let touched = |g: &Graph, id: &str| g.edges.iter().any(|e| e.from == id || e.to == id);

    // 1. small things nothing is joined to
    let loose: Vec<usize> = (0..g.nodes.len())
        .filter(|i| {
            let n = &g.nodes[*i];
            (n.module == "loss" || n.module.starts_with("arrow")) && n.str("by").is_none() && !touched(&g, &n.id) && at(n).is_some()
        })
        .collect();
    for &i in &loose {
        let (c, l) = at(&g.nodes[i]).unwrap();
        let near = (0..g.nodes.len())
            .filter(|j| !loose.contains(j) && g.nodes[*j].panel == g.nodes[i].panel && touched(&g, &g.nodes[*j].id) && !pack::is_note(&g, *j))
            .filter_map(|j| at(&g.nodes[j]).map(|(x, y)| ((x - c).abs() + (y - l).abs(), j)))
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        if let Some((_, j)) = near {
            let to = g.nodes[j].id.clone();
            said.push(format!("{}：貼著 {}，不佔格子", g.nodes[i].id, to));
            g.nodes[i].params.insert("by".into(), json!(to));
        }
    }
    let object = |g: &Graph, i: usize| !pack::is_note(g, i) && at(&g.nodes[i]).is_some();

    // 2. beside the rows a module is joined to
    let rows_of = |g: &Graph, i: usize| -> (f64, f64) {
        let (_, l) = at(&g.nodes[i]).unwrap();
        (l, l + g.nodes[i].num("tall").unwrap_or(1.0).max(1.0) - 1.0)
    };
    let free = |g: &Graph, i: usize, lo: f64, hi: f64| -> bool {
        let (c, _) = at(&g.nodes[i]).unwrap();
        let c1 = c + g.nodes[i].num("span").unwrap_or(1.0).max(1.0) - 1.0;
        !(0..g.nodes.len()).any(|j| {
            if j == i || !object(g, j) || g.nodes[j].panel != g.nodes[i].panel {
                return false;
            }
            let (x, _) = at(&g.nodes[j]).unwrap();
            let x1 = x + g.nodes[j].num("span").unwrap_or(1.0).max(1.0) - 1.0;
            let (y0, y1) = rows_of(g, j);
            x <= c1 + 1e-6 && c <= x1 + 1e-6 && y0 <= hi + 1e-6 && lo <= y1 + 1e-6
        })
    };
    let flows = |k: EdgeKind| matches!(k, EdgeKind::Data | EdgeKind::Side);
    let first: Vec<Option<(f64, f64)>> = g.nodes.iter().map(at).collect();
    for i in 0..g.nodes.len() {
        if !object(&g, i) || g.nodes[i].params.contains_key("tall") || down(&g, &g.nodes[i].panel) {
            continue;
        }
        let (c, l) = at(&g.nodes[i]).unwrap();
        let id = g.nodes[i].id.clone();
        // (column, row) of the modules joined to it, left of it and right of it
        let mut sides: [Vec<(f64, f64)>; 2] = [Vec::new(), Vec::new()];
        for e in g.edges.iter().filter(|e| flows(e.kind) && (e.from == id || e.to == id)) {
            let other = if e.from == id { &e.to } else { &e.from };
            let Some(j) = g.node_index(other) else { continue };
            if !object(&g, j) || g.nodes[j].panel != g.nodes[i].panel {
                continue;
            }
            let (x, y) = first[j].unwrap();
            if x < c - 1e-6 {
                sides[0].push((x, y))
            } else if x > c + 1e-6 {
                sides[1].push((x, y))
            }
        }
        let mut lo = l;
        let mut hi = l;
        // parallel rows: modules of ONE column, in several rows, all joined to this one
        let mut own = false;
        for s in &sides {
            let mut xs: Vec<f64> = s.iter().map(|p| p.0).collect();
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            xs.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
            for x in xs {
                let mut ys: Vec<f64> = s.iter().filter(|p| (p.0 - x).abs() < 1e-6).map(|p| p.1).collect();
                ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
                ys.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
                // it stands in one of those rows itself (between two rows it is in their
                // middle already)
                if ys.len() >= 2 && ys.iter().any(|y| (*y - l).abs() < 1e-6) {
                    lo = if own { lo.min(ys[0]) } else { ys[0] };
                    hi = if own { hi.max(ys[ys.len() - 1]) } else { ys[ys.len() - 1] };
                    own = true;
                }
            }
        }
        // a data line to a module of its own row stays level: the middle of the rows is its row,
        // or no such line exists
        let level = ((lo + hi) / 2.0 - l).abs() < 1e-6
            || !g.edges.iter().any(|e| e.kind == EdgeKind::Data && (e.from == id || e.to == id) && g.node_index(if e.from == id { &e.to } else { &e.from }).is_some_and(|j| j != i && first[j].is_some_and(|p| (p.1 - l).abs() < 1e-6) && g.nodes[j].panel == g.nodes[i].panel));
        // a plain box can be as tall as the rows: every row's line into it is then straight,
        // wherever its own row lies among them
        let grows = g.nodes[i].module == "box" && !g.nodes[i].params.contains_key("h") && !g.nodes[i].params.contains_key("stretch");
        if !(hi - lo > 1e-6 && (level || grows)) {
            continue;
        }
        // a module joined to it that stands in its column, in those rows, moves half a column
        // on (it is the next step of that row, written into the same column)
        if !free(&g, i, lo, hi) {
            let blockers: Vec<usize> = (0..g.nodes.len())
                .filter(|j| *j != i && object(&g, *j) && g.nodes[*j].panel == g.nodes[i].panel && at(&g.nodes[*j]).is_some_and(|(x, y)| (x - c).abs() < 1e-6 && y >= lo - 1e-6 && y <= hi + 1e-6))
                .collect();
            let joined = |j: usize| g.edges.iter().any(|e| (e.from == id && e.to == g.nodes[j].id) || (e.to == id && e.from == g.nodes[j].id));
            if !blockers.is_empty() && blockers.iter().all(|j| joined(*j) && g.nodes[*j].num("tall").unwrap_or(1.0) <= 1.0 && g.nodes[*j].num("span").unwrap_or(1.0) <= 1.0) {
                for j in blockers {
                    let leaves = g.edges.iter().any(|e| e.from == id && e.to == g.nodes[j].id);
                    let x = c + if leaves { 0.5 } else { -0.5 };
                    said.push(format!("{}：移到 {id} 旁邊的半欄", g.nodes[j].id));
                    g.nodes[j].params.insert("col".into(), json!(x));
                }
            }
        }
        if hi - lo > 1e-6 && free(&g, i, lo, hi) {
            g.nodes[i].params.insert("lane".into(), json!(lo));
            g.nodes[i].params.insert("tall".into(), json!(hi - lo + 1.0));
            if grows {
                g.nodes[i].params.insert("stretch".into(), json!("tall"));
            }
            said.push(format!("{id}：站在第 {lo}–{hi} 列旁邊{}", if grows { "，跟那幾列一樣高" } else { "" }));
        }
    }
    // a module in the row of one that now stands beside several rows, joined to it by a data
    // line, stands there with it (the trunk of a figure stays one line)
    loop {
        let mut moved = false;
        for i in 0..g.nodes.len() {
            if !object(&g, i) || g.nodes[i].params.contains_key("tall") || down(&g, &g.nodes[i].panel) {
                continue;
            }
            let id = g.nodes[i].id.clone();
            let lead = g.edges.iter().filter(|e| e.kind == EdgeKind::Data && (e.from == id || e.to == id)).find_map(|e| {
                let j = g.node_index(if e.from == id { &e.to } else { &e.from })?;
                let tall = g.nodes[j].num("tall").filter(|t| *t > 1.0)?;
                (object(&g, j) && g.nodes[j].panel == g.nodes[i].panel && first[j].map(|p| p.1) == first[i].map(|p| p.1)).then(|| (at(&g.nodes[j]).unwrap().1, tall))
            });
            if let Some((lo, tall)) = lead {
                if free(&g, i, lo, lo + tall - 1.0) {
                    g.nodes[i].params.insert("lane".into(), json!(lo));
                    g.nodes[i].params.insert("tall".into(), json!(tall));
                    said.push(format!("{id}：跟著主線站在第 {lo}–{} 列旁邊", lo + tall - 1.0));
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }

    // 3. a row of things no line touches: each over the columns nearest to it
    let panels: Vec<String> = g.panels.iter().map(|p| p.id.clone()).collect();
    for p in &panels {
        if down(&g, p) {
            continue;
        }
        let here: Vec<usize> = (0..g.nodes.len()).filter(|i| &g.nodes[*i].panel == p && object(&g, *i)).collect();
        if !here.iter().any(|i| touched(&g, &g.nodes[*i].id)) {
            continue;
        }
        let mut cols: Vec<f64> = here.iter().flat_map(|i| { let (c, _) = at(&g.nodes[*i]).unwrap(); let s = g.nodes[*i].num("span").unwrap_or(1.0).max(1.0); [c, c + s - 1.0] }).collect();
        cols.sort_by(|a, b| a.partial_cmp(b).unwrap());
        cols.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        let mut lanes: Vec<f64> = here.iter().map(|i| at(&g.nodes[*i]).unwrap().1).collect();
        lanes.sort_by(|a, b| a.partial_cmp(b).unwrap());
        lanes.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        for l in lanes {
            let mut row: Vec<usize> = here.iter().copied().filter(|i| (at(&g.nodes[*i]).unwrap().1 - l).abs() < 1e-6 && g.nodes[*i].num("tall").unwrap_or(1.0) <= 1.0).collect();
            if row.len() < 2 || row.iter().any(|i| touched(&g, &g.nodes[*i].id) || g.nodes[*i].params.contains_key("span")) {
                continue;
            }
            // nothing else may reach into this row
            if here.iter().any(|i| !row.contains(i) && { let (y0, y1) = rows_of(&g, *i); y0 <= l + 1e-6 && l <= y1 + 1e-6 }) {
                continue;
            }
            row.sort_by(|a, b| at(&g.nodes[*a]).unwrap().0.partial_cmp(&at(&g.nodes[*b]).unwrap().0).unwrap());
            let cs: Vec<f64> = row.iter().map(|i| at(&g.nodes[*i]).unwrap().0).collect();
            for (k, &i) in row.iter().enumerate() {
                let from = if k == 0 { f64::MIN } else { (cs[k - 1] + cs[k]) / 2.0 };
                let to = if k + 1 == cs.len() { f64::MAX } else { (cs[k] + cs[k + 1]) / 2.0 };
                let mine: Vec<f64> = cols.iter().copied().filter(|c| *c > from + 1e-6 && *c <= to + 1e-6).collect();
                let (Some(a), Some(z)) = (mine.first(), mine.last()) else { continue };
                if z - a > 1e-6 {
                    g.nodes[i].params.insert("col".into(), json!(a));
                    g.nodes[i].params.insert("span".into(), json!(z - a + 1.0));
                    said.push(format!("{}：蓋在第 {a}–{z} 欄上", g.nodes[i].id));
                }
            }
        }
    }

    // 4. a bracket as wide as its columns, a brace as tall as its rows
    for n in &mut g.nodes {
        if n.params.contains_key("stretch") {
            continue;
        }
        if n.module == "bracket" && n.num("span").is_some_and(|s| s > 1.0) {
            n.params.insert("stretch".into(), json!(true));
            said.push(format!("{}：跟所括的欄一樣寬", n.id));
        } else if n.module == "brace" && n.num("tall").is_some_and(|t| t > 1.0) {
            n.params.insert("stretch".into(), json!("tall"));
            said.push(format!("{}：跟所括的列一樣高", n.id));
        }
    }

    // 5. long words in a box or a label break in two near the middle, outside maths
    for n in &mut g.nodes {
        if !matches!(n.module.as_str(), "box" | "label") || n.flag("vertical") || n.label.contains("\\\\") || n.label.chars().count() <= LONG {
            continue;
        }
        let chars: Vec<char> = n.label.chars().collect();
        let mut maths = false;
        let mut cuts: Vec<usize> = Vec::new();
        for (k, c) in chars.iter().enumerate() {
            if *c == '$' {
                maths = !maths;
            } else if *c == ' ' && !maths {
                cuts.push(k);
            }
        }
        let mid = chars.len() as f64 / 2.0;
        if let Some(k) = cuts.into_iter().min_by(|a, b| (*a as f64 - mid).abs().partial_cmp(&(*b as f64 - mid).abs()).unwrap()) {
            n.label = format!("{}\\\\{}", chars[..k].iter().collect::<String>(), chars[k + 1..].iter().collect::<String>());
            said.push(format!("{}：長標籤斷成兩行", n.id));
        }
    }
    (g, said)
}
