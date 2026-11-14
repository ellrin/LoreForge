//! Equation figures (`kind: "equation"`, docs/plot.md §10, docs/living-equations.md tier 1).
//!
//! A panel `{ "type": "equation", "eq": { … } }` plots an expression of the safe language
//! ([`expr`]) over its input on x, with parameters as sliders. The same catalogue of verified
//! templates (`commons/ui/plot/equations.json`) is read here and by the browser
//! (`eqcore.js` evaluates, `equation.js` draws and edits). The server side checks what the browser
//! will draw: it parses, evaluates on a grid, rejects non-finite or absurd output ([`problems`]),
//! tests the stated properties numerically ([`check_properties`]) and turns frames into a GIF
//! ([`gif`]). The model task `equation-plot` ([`propose`]) proposes an `eq`; it is never trusted
//! before these checks.

pub mod expr;
pub mod gif;
pub mod propose;
pub mod raster;
pub mod sim;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use expr::{Env, Node, Val};

/// The template catalogue (shared with the browser).
pub const CATALOGUE: &str = include_str!("../../../../ui/plot/equations.json");

/// Grid points for checks (the browser draws with the same count).
pub const GRID: usize = 201;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Var {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// "x" | "param" | "vector" | "fixed"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// A number (param, fixed) or a list (vector).
    #[serde(default)]
    pub value: Value,
    /// The value the paper uses (marked on the slider; reset goes there).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub log: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub integer: bool,
    /// Colour role (an index into the palette in force; docs/plot.md §10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u8>,
}

impl Var {
    /// Its colour role, when set.
    pub fn rest_color(&self) -> Option<u8> {
        self.color
    }
    pub fn set_color(&mut self, c: u8) {
        self.color = Some(c);
    }
    pub fn num(&self) -> Option<f64> {
        self.value.as_f64()
    }
    pub fn range(&self) -> Option<(f64, f64)> {
        Some((self.min?, self.max?))
    }
    /// `n` values over the range (log-spaced when `log`), integers rounded.
    pub fn sweep(&self, n: usize) -> Vec<f64> {
        let Some((a, b)) = self.range() else { return vec![] };
        let n = n.max(2);
        (0..n)
            .map(|i| {
                let t = i as f64 / (n - 1) as f64;
                let v = if self.log && a > 0.0 && b > 0.0 { (a.ln() + t * (b.ln() - a.ln())).exp() } else { a + t * (b - a) };
                if self.integer { v.round() } else { v }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Property {
    /// nonneg | bounded | zero_at | min_at | max_at | increasing | decreasing | symmetric | odd | equals | sums_to
    pub kind: String,
    /// The variable it is about (default: the input on x).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub var: Option<String>,
    /// A number or an expression over the variables.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lo: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hi: Option<f64>,
    /// equals: the other expression.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
    /// Parameter values the property assumes.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub when: Map<String, Value>,
    /// Input values (x) a parameter monotonicity is tested at.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub at: Map<String, Value>,
    /// The sentence shown (may hold $…$).
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Animate {
    pub var: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<usize>,
    /// Milliseconds per frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Family {
    pub var: String,
    pub values: Vec<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on: bool,
}

/// The `eq` of an equation panel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Eq {
    /// The catalogue template it came from (None: custom).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Empty for a simulation (`sim`).
    #[serde(default)]
    pub expr: String,
    /// The paper's LaTeX (shown beside ours; never evaluated).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub latex: String,
    /// Template LaTeX (ours) when a template is used.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<Var>,
    #[serde(default)]
    pub vars: Vec<Var>,
    /// "curve" (default) | "bars" (the expression gives a vector)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// How the paper's notation was reduced to a small concrete case.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reduction: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<Property>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animate: Option<Animate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<Family>,
    /// A simulation scene instead of a curve (docs/plot.md §11, [`sim`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sim: Option<sim::Sim>,
    /// editor state and anything newer (show, at_x, checks …)
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Eq {
    pub fn bars(&self) -> bool {
        self.output.as_deref() == Some("bars")
    }
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.vars.iter().map(|v| v.name.clone()).collect();
        if let Some(x) = &self.x
            && !self.bars()
        {
            v.push(x.name.clone());
        }
        v
    }
    pub fn param(&self, name: &str) -> Option<&Var> {
        self.vars.iter().find(|v| v.name == name)
    }
    pub fn parse(&self) -> Result<Node, expr::ExprError> {
        self.parse_other(&self.expr)
    }
    pub fn parse_other(&self, src: &str) -> Result<Node, expr::ExprError> {
        let names = self.names();
        expr::parse(src, &|n| names.iter().any(|m| m == n))
    }
    /// Variables at their current values, `over` applied on top.
    pub fn env(&self, over: &[(&str, f64)]) -> Env {
        let mut env = Env::new();
        for v in &self.vars {
            let val = match &v.value {
                Value::Array(a) => Val::V(a.iter().filter_map(Value::as_f64).collect()),
                other => {
                    let x = other.as_f64().unwrap_or(f64::NAN);
                    Val::S(if v.integer { x.round() } else { x })
                }
            };
            env.insert(v.name.clone(), val);
        }
        if let Some(x) = &self.x
            && !self.bars()
        {
            env.insert(x.name.clone(), Val::S(x.min.unwrap_or(0.0)));
        }
        for (k, v) in over {
            let integer = self.param(k).is_some_and(|p| p.integer);
            env.insert(k.to_string(), Val::S(if integer { v.round() } else { *v }));
        }
        env
    }
    /// x grid of `n` points.
    pub fn xs(&self, n: usize) -> Vec<f64> {
        let Some(x) = &self.x else { return vec![] };
        let (a, b) = (x.min.unwrap_or(0.0), x.max.unwrap_or(1.0));
        (0..n).map(|i| a + (b - a) * i as f64 / (n - 1).max(1) as f64).collect()
    }
}

/// The equation of a panel, if it is one.
pub fn eq_of(panel: &crate::Panel) -> Option<Result<Eq, String>> {
    if panel.kind != "equation" {
        return None;
    }
    Some(match panel.rest.get("eq") {
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| format!("eq 欄位不對：{e}")),
        None => Err("沒有 eq".into()),
    })
}

// ── catalogue ───────────────────────────────────────────────────────────

pub fn catalogue() -> Value {
    serde_json::from_str(CATALOGUE).expect("equations.json parses")
}

pub fn template_ids() -> Vec<String> {
    catalogue()["templates"].as_array().into_iter().flatten().filter_map(|t| t["id"].as_str().map(str::to_string)).collect()
}

/// The template's `eq` (paper values not yet applied).
pub fn template_eq(id: &str) -> Option<Eq> {
    let t = catalogue()["templates"].as_array()?.iter().find(|t| t["id"] == id)?.clone();
    let mut x: Option<Var> = t.get("x").and_then(|v| serde_json::from_value(v.clone()).ok());
    if let Some(x) = x.as_mut() {
        x.role = "x".into();
    }
    Some(Eq {
        template: Some(id.to_string()),
        expr: t["expr"].as_str()?.to_string(),
        latex: String::new(),
        tex: t["latex"].as_str().unwrap_or("").to_string(),
        x,
        vars: serde_json::from_value(t["vars"].clone()).unwrap_or_default(),
        output: t["output"].as_str().map(str::to_string),
        reduction: serde_json::from_value(t["reduction"].clone()).unwrap_or_default(),
        properties: serde_json::from_value(t["properties"].clone()).unwrap_or_default(),
        animate: serde_json::from_value(t["animate"].clone()).ok(),
        family: serde_json::from_value(t["family"].clone()).ok(),
        sim: None,
        rest: Map::new(),
    })
}

/// An equation figure on a slide (mm; equation.js `SLIDE_W/H`): with the slide style's type it
/// reads at projection size.
pub const SLIDE_MM: (f64, f64) = (150.0, 95.0);

/// The catalogue as lore-notation entries (the shared registry, no separate equation library):
/// one `equation` entry per template (`eq:<id>`, its LaTeX, `uses` = its symbols) and one `symbol`
/// entry per variable LaTeX (`v:<canonical>`), with a colour role (`color`): the output 0, the input
/// on x 1, parameters 2, 3, … in order. A symbol met again in a later template keeps its first role.
/// This is the neutral public layer; the author's global registry and the paper's / deck's own
/// notation are read over it (their `color` wins).
pub fn template_notation() -> lore_notation::Notation {
    use lore_notation::{Entry, Kind, Notation};
    let mut n = Notation::default();
    for id in template_ids() {
        let Some(eq) = template_eq(&id) else { continue };
        let mut uses = vec![];
        let mut sym = |tex: &str, meaning: &str, role: u8, n: &mut Notation| {
            // the maths of a label (`$q$（背景）` → `q`, `類別 $k$` → `k`)
            let tex = match (tex.find('$'), tex.rfind('$')) {
                (Some(a), Some(b)) if b > a => tex[a + 1..b].trim().to_string(),
                _ => tex.trim().to_string(),
            };
            if tex.is_empty() || tex.contains('$') || tex.chars().any(|c| ('\u{3000}'..='\u{9fff}').contains(&c) || ('\u{ff00}'..='\u{ffef}').contains(&c)) {
                return;
            }
            let key = format!("v:{}", lore_notation::math::canonical(&tex));
            if n.get(&key).is_none() {
                let mut e = Entry::new(&key, Kind::Symbol);
                e.latex = Some(tex);
                e.meaning_en = Some(meaning.to_string()).filter(|m| !m.is_empty());
                e.color = Some(role);
                e.tags = vec!["conventional".into(), "lore-plot".into()];
                n.upsert(e);
            }
            uses.push(key);
        };
        if let Some(x) = &eq.x {
            sym(&x.label, &x.name, 1, &mut n);
        }
        for (k, v) in eq.vars.iter().filter(|v| v.role == "param").enumerate() {
            sym(&v.label, &v.name, 2 + (k as u8 % 6), &mut n);
        }
        let mut e = Entry::new(&format!("eq:{id}"), Kind::Equation);
        e.latex = Some(eq.tex.clone());
        e.uses = uses;
        e.color = Some(0);
        e.tags = vec!["lore-plot".into()];
        n.upsert(e);
    }
    n
}

/// Colour roles by symbol LaTeX (canonical): the templates' layer, then each of `layers` over it
/// (the author's global registry, the paper's / deck's notation): the last one that sets `color` wins.
pub fn color_roles(layers: &[&lore_notation::Notation]) -> std::collections::BTreeMap<String, u8> {
    let base = template_notation();
    let mut out = std::collections::BTreeMap::new();
    for n in std::iter::once(&base).chain(layers.iter().copied()) {
        for e in &n.entries {
            if let (Some(tex), Some(c)) = (e.latex.as_deref(), e.color) {
                out.insert(lore_notation::math::canonical(tex), c);
            }
        }
    }
    out
}

/// The maths of a label: `$q$（背景）` → `q`.
pub fn label_tex(label: &str) -> String {
    match (label.find('$'), label.rfind('$')) {
        (Some(a), Some(b)) if b > a => label[a + 1..b].trim().to_string(),
        _ => label.trim().to_string(),
    }
}

/// Write colour roles onto the variables that have none (the same rule as equation.js
/// `assignRoles` + `distinctRoles`): from the registry layers by symbol, then distinct within
/// the equation (the function 0; a taken role moves to the lowest free one). A figure made
/// with nobody looking (論文報告) keeps these, so a later re-edit does not shift its colours.
pub fn assign_roles(eq: &mut Eq, layers: &[&lore_notation::Notation]) {
    let roles = color_roles(layers);
    let mut used = std::collections::BTreeSet::from([0u8]);
    let x_default = 1u8;
    let mut k = 0u8;
    let take = |v: &mut Var, default: u8, used: &mut std::collections::BTreeSet<u8>| {
        let from_reg = roles.get(&lore_notation::math::canonical(&label_tex(if v.label.is_empty() { &v.name } else { &v.label }))).copied();
        let mut r = v.rest_color().or(from_reg).unwrap_or(default);
        if used.contains(&r) {
            r = (0..6u8).find(|c| !used.contains(c)).unwrap_or(r);
        }
        used.insert(r);
        v.set_color(r);
    };
    if let Some(x) = eq.x.as_mut() {
        take(x, x_default, &mut used);
    }
    for v in eq.vars.iter_mut().filter(|v| v.role == "param") {
        take(v, 2 + k % 4, &mut used);
        k += 1;
    }
}

/// A whole figure spec for an equation (`kind: "equation"`, one panel, no data).
pub fn spec_for(eq: &Eq, style: &str) -> Value {
    let t = eq.template.as_deref().and_then(|id| catalogue()["templates"].as_array().and_then(|a| a.iter().find(|t| t["id"] == id).cloned())).unwrap_or(Value::Null);
    let ylabel = eq.rest.get("y_label").and_then(Value::as_str).or(t["y"]["label"].as_str()).unwrap_or("$f$");
    let yaxis = json!({ "label": ylabel });
    let xlabel = match &eq.x {
        Some(x) if !eq.bars() => if x.label.is_empty() { format!("${}$", x.name) } else { x.label.clone() },
        _ => t["x"]["label"].as_str().unwrap_or("").to_string(),
    };
    let mut eqv = serde_json::to_value(eq).unwrap_or_default();
    if eqv["tex"].is_null() {
        eqv["tex"] = Value::String(eq.tex.clone());
    }
    // the template's y range is for the function itself (the editor drops it for derivatives)
    if eqv.get("yrange").is_none() && (t["y"]["min"].is_number() || t["y"]["max"].is_number()) {
        eqv["yrange"] = json!([t["y"]["min"], t["y"]["max"]]);
    }
    if eq.bars() && eqv.get("x").is_none() {
        eqv["xlabel"] = xlabel.clone().into();
    }
    json!({
        "lore_plot": crate::spec::VERSION,
        "kind": "equation",
        "style": style,
        "set": if style.contains("slide") { json!({ "font.tick": 16, "font.label": 18, "legend.size": 16, "axes.max_ticks": 5 }) } else { json!({}) },
        "size": if style.contains("slide") { json!({ "w": SLIDE_MM.0, "h": SLIDE_MM.1 }) } else { Value::Null },
        "grid": { "cols": 1, "rows": 1, "gap": 5, "labels": "none" },
        "data": {},
        "panels": [{ "type": "equation", "eq": eqv, "title": "", "xaxis": { "label": xlabel }, "yaxis": yaxis, "legend": { "pos": "best" }, "opts": {}, "notes": [] }],
        "caption": "",
    })
}

// ── evaluation ──────────────────────────────────────────────────────────

/// f over the x grid at the current values (+ `over`); None where not a finite scalar.
pub fn curve(eq: &Eq, node: &Node, xs: &[f64], over: &[(&str, f64)]) -> Result<Vec<Option<f64>>, expr::ExprError> {
    let xn = eq.x.as_ref().map(|x| x.name.clone()).unwrap_or_default();
    let mut env = eq.env(over);
    let mut out = Vec::with_capacity(xs.len());
    for &x in xs {
        env.insert(xn.clone(), Val::S(x));
        let v = expr::eval(node, &env)?;
        out.push(v.scalar().filter(|y| y.is_finite()));
    }
    Ok(out)
}

/// One value (None: not a finite scalar).
pub fn value(eq: &Eq, node: &Node, over: &[(&str, f64)]) -> Result<Option<f64>, expr::ExprError> {
    Ok(expr::eval(node, &eq.env(over))?.scalar().filter(|y| y.is_finite()))
}

/// ∂f/∂name at the point (central difference inside the variable's range).
pub fn partial(eq: &Eq, node: &Node, name: &str, over: &[(&str, f64)]) -> Result<Option<f64>, expr::ExprError> {
    let env = eq.env(over);
    let Some(at) = env.get(name).and_then(Val::scalar) else { return Ok(None) };
    let range = if eq.x.as_ref().is_some_and(|x| x.name == name) { eq.x.as_ref().and_then(Var::range) } else { eq.param(name).and_then(Var::range) };
    let (lo, hi) = range.unwrap_or((at - 1.0, at + 1.0));
    let h = ((hi - lo).abs() * 1e-4).max(1e-9);
    let (a, b) = ((at - h).max(lo), (at + h).min(hi));
    if b <= a {
        return Ok(None);
    }
    let mut o: Vec<(&str, f64)> = over.iter().filter(|(k, _)| *k != name).cloned().collect();
    o.push((name, a));
    let fa = value(eq, node, &o)?;
    o.pop();
    o.push((name, b));
    let fb = value(eq, node, &o)?;
    Ok(match (fa, fb) {
        (Some(fa), Some(fb)) => Some((fb - fa) / (b - a)),
        _ => None,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sens {
    pub name: String,
    /// ∂f/∂θ
    pub d: f64,
    /// ∂f/∂θ × (θ range): how much f moves over the parameter's whole range (linearised)
    pub scaled: f64,
    /// scaled / max |scaled| ∈ [−1, 1]
    pub norm: f64,
}

/// 敏感度: each parameter's normalised partial derivative at x = `x0`.
pub fn sensitivity(eq: &Eq, node: &Node, x0: f64) -> Result<Vec<Sens>, expr::ExprError> {
    let xn = eq.x.as_ref().map(|x| x.name.clone()).unwrap_or_default();
    let mut out = Vec::new();
    for p in eq.vars.iter().filter(|v| v.role == "param") {
        let over = [(xn.as_str(), x0)];
        let d = partial(eq, node, &p.name, &over)?.unwrap_or(0.0);
        let span = p.range().map(|(a, b)| b - a).unwrap_or(1.0);
        out.push(Sens { name: p.name.clone(), d, scaled: d * span, norm: 0.0 });
    }
    let m = out.iter().map(|s| s.scaled.abs()).fold(0.0, f64::max);
    for s in &mut out {
        s.norm = if m > 0.0 { s.scaled / m } else { 0.0 };
    }
    Ok(out)
}

/// The values a GIF sweeps through: up the range and back (ping-pong), `n` frames in all,
/// each end shown once (so the loop has no stall).
pub fn sweep_frames(v: &Var, n: usize) -> Vec<f64> {
    let n = n.max(2);
    let up = n / 2 + 1;
    let vals = v.sweep(up);
    let mut out = vals.clone();
    out.extend(vals[1..vals.len() - 1].iter().rev().take(n - up));
    out
}

// ── checks ──────────────────────────────────────────────────────────────

/// What blocks a figure: the expression does not parse, names are missing, ranges are
/// broken, or the values are not finite / absurd. Empty = usable.
pub fn problems(eq: &Eq) -> Vec<String> {
    if let Some(s) = &eq.sim {
        return sim::problems(s);
    }
    let mut out = Vec::new();
    let node = match eq.parse() {
        Ok(n) => n,
        Err(e) => return vec![format!("算式：{e}")],
    };
    if !eq.bars() {
        match &eq.x {
            None => out.push("沒有橫軸變數".into()),
            Some(x) => match x.range() {
                Some((a, b)) if a.is_finite() && b.is_finite() && b > a => {}
                _ => out.push(format!("橫軸 {} 的範圍不對", x.name)),
            },
        }
    }
    for v in &eq.vars {
        if v.role == "vector" {
            if !v.value.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|x| x.as_f64().is_some_and(f64::is_finite))) {
                out.push(format!("{} 要是一串數字", v.name));
            }
            continue;
        }
        let Some(val) = v.num().filter(|x| x.is_finite()) else {
            out.push(format!("{} 沒有數值", v.name));
            continue;
        };
        if v.role == "param" {
            match v.range() {
                Some((a, b)) if a.is_finite() && b.is_finite() && b > a => {
                    if val < a - 1e-12 || val > b + 1e-12 {
                        out.push(format!("{} = {val} 不在範圍 {a}–{b} 內", v.name));
                    }
                    if v.log && a <= 0.0 {
                        out.push(format!("{} 是對數滑桿，範圍要大於 0", v.name));
                    }
                }
                _ => out.push(format!("{} 的範圍不對", v.name)),
            }
        }
    }
    if !out.is_empty() {
        return out;
    }
    if eq.bars() {
        match expr::eval(&node, &eq.env(&[])) {
            Ok(Val::V(v)) if v.iter().all(|x| x.is_finite()) && v.len() <= 64 => {}
            Ok(Val::V(_)) => out.push("長條的值不是有限的數（或超過 64 條）".into()),
            Ok(Val::S(_)) => out.push("長條圖的算式要給一串數（向量）".into()),
            Err(e) => out.push(format!("算式：{e}")),
        }
        return out;
    }
    let xs = eq.xs(GRID);
    match expr::eval(&node, &eq.env(&[])) {
        Ok(Val::V(v)) if v.len() > 1 => {
            out.push("算式的結果是向量：改成長條圖，或對它取 sum／mean".into());
            return out;
        }
        Err(e) => {
            out.push(format!("算式：{e}"));
            return out;
        }
        _ => {}
    }
    let ys = match curve(eq, &node, &xs, &[]) {
        Ok(y) => y,
        Err(e) => return vec![format!("算式：{e}")],
    };
    let finite: Vec<f64> = ys.iter().flatten().cloned().collect();
    if finite.len() * 10 < ys.len() * 9 {
        out.push(format!("目前的參數下只有 {}/{} 個點有值（範圍或定義域不對？）", finite.len(), ys.len()));
    }
    if finite.iter().any(|y| y.abs() > 1e9) {
        out.push("數值大到不合理（超過 10⁹），範圍可能不對".into());
    }
    // each parameter at its ends still gives a curve
    for p in eq.vars.iter().filter(|v| v.role == "param") {
        for end in [p.min, p.max].into_iter().flatten() {
            match curve(eq, &node, &xs, &[(&p.name, end)]) {
                Ok(ys) => {
                    let n = ys.iter().flatten().count();
                    if n * 2 < ys.len() {
                        out.push(format!("{} = {end} 時大多沒有值", p.name));
                    }
                }
                Err(e) => out.push(format!("{} = {end}：{e}", p.name)),
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub kind: String,
    pub text: String,
    pub ok: bool,
    /// Why it failed / what was measured.
    #[serde(default)]
    pub detail: String,
}

fn num_or_expr(eq: &Eq, v: &Value, over: &[(&str, f64)]) -> Result<f64, String> {
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(|| "不是數".into()),
        Value::String(s) => {
            let node = eq.parse_other(s).map_err(|e| e.to_string())?;
            value(eq, &node, over).map_err(|e| e.to_string())?.ok_or_else(|| format!("「{s}」沒有值"))
        }
        _ => Err("缺少 value".into()),
    }
}

fn overrides(m: &Map<String, Value>) -> Vec<(String, f64)> {
    m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect()
}

/// Test each stated property numerically (failures reported, never hidden).
pub fn check_properties(eq: &Eq) -> Vec<CheckResult> {
    if eq.sim.is_some() {
        return vec![];
    }
    eq.properties.iter().map(|p| check_one(eq, p)).collect()
}

pub fn check_one(eq: &Eq, p: &Property) -> CheckResult {
    let r = check_inner(eq, p);
    CheckResult {
        kind: p.kind.clone(),
        text: if p.text.is_empty() { p.kind.clone() } else { p.text.clone() },
        ok: r.is_ok(),
        detail: match r {
            Ok(d) => d,
            Err(d) => d,
        },
    }
}

fn check_inner(eq: &Eq, p: &Property) -> Result<String, String> {
    let node = eq.parse().map_err(|e| e.to_string())?;
    let when_owned = overrides(&p.when);
    let when: Vec<(&str, f64)> = when_owned.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    if let Some(name) = when.iter().map(|w| w.0).find(|k| !eq.names().iter().any(|n| n == k)) {
        return Err(format!("沒有變數 {name}"));
    }
    if eq.bars() {
        let v = expr::eval(&node, &eq.env(&when)).map_err(|e| e.to_string())?.as_vec();
        return match p.kind.as_str() {
            "sums_to" => {
                let t = num_or_expr(eq, &p.value, &when)?;
                let s: f64 = v.iter().sum();
                if (s - t).abs() <= 1e-9 * t.abs().max(1.0) { Ok(format!("和 = {s:.6}")) } else { Err(format!("和 = {s:.6}，不是 {t}")) }
            }
            "bounded" => {
                let (lo, hi) = (p.lo.unwrap_or(f64::NEG_INFINITY), p.hi.unwrap_or(f64::INFINITY));
                match v.iter().find(|y| !(**y >= lo - 1e-9 && **y <= hi + 1e-9)) {
                    None => Ok(String::new()),
                    Some(y) => Err(format!("有一條是 {y:.4}")),
                }
            }
            "nonneg" => {
                if v.iter().all(|y| *y >= -1e-12) { Ok(String::new()) } else { Err("有負的".into()) }
            }
            k => Err(format!("長條圖不能檢查「{k}」")),
        };
    }
    let x = eq.x.as_ref().ok_or("沒有橫軸變數")?;
    let (xa, xb) = x.range().ok_or("橫軸沒有範圍")?;
    let xs = eq.xs(GRID);
    let ys = curve(eq, &node, &xs, &when).map_err(|e| e.to_string())?;
    let finite: Vec<f64> = ys.iter().flatten().cloned().collect();
    let scale = finite.iter().fold(0.0f64, |m, y| m.max(y.abs())).max(1.0);
    let tol = 1e-6 * scale;
    let at = |xv: f64| -> Result<Option<f64>, String> {
        let mut o = when.clone();
        o.push((x.name.as_str(), xv));
        value(eq, &node, &o).map_err(|e| e.to_string())
    };
    match p.kind.as_str() {
        "nonneg" => match xs.iter().zip(&ys).find(|(_, y)| y.is_some_and(|y| y < -tol)) {
            None => Ok(format!("最小 {:.4}", finite.iter().cloned().fold(f64::INFINITY, f64::min))),
            Some((xv, y)) => Err(format!("{} = {xv:.3} 時是 {:.4}", x.name, y.unwrap())),
        },
        "bounded" => {
            let (lo, hi) = (p.lo.unwrap_or(f64::NEG_INFINITY), p.hi.unwrap_or(f64::INFINITY));
            match xs.iter().zip(&ys).find(|(_, y)| y.is_some_and(|y| y < lo - tol || y > hi + tol)) {
                None => Ok(String::new()),
                Some((xv, y)) => Err(format!("{} = {xv:.3} 時是 {:.4}", x.name, y.unwrap())),
            }
        }
        "zero_at" => {
            let v = num_or_expr(eq, &p.value, &when)?;
            match at(v)? {
                Some(y) if y.abs() <= tol => Ok(format!("f({v:.3}) = {y:.2e}")),
                Some(y) => Err(format!("f({v:.3}) = {y:.4}")),
                None => Err(format!("f({v:.3}) 沒有值")),
            }
        }
        "min_at" | "max_at" => {
            let v = num_or_expr(eq, &p.value, &when)?;
            if v < xa - 1e-9 || v > xb + 1e-9 {
                return Err(format!("{v:.3} 不在橫軸範圍內"));
            }
            let fine = 2001;
            let xs2: Vec<f64> = (0..fine).map(|i| xa + (xb - xa) * i as f64 / (fine - 1) as f64).collect();
            let ys2 = curve(eq, &node, &xs2, &when).map_err(|e| e.to_string())?;
            let mut best: Option<(f64, f64)> = None;
            for (xv, y) in xs2.iter().zip(&ys2) {
                if let Some(y) = y {
                    let better = match best {
                        None => true,
                        Some((_, b)) => (p.kind == "min_at" && *y < b) || (p.kind == "max_at" && *y > b),
                    };
                    if better {
                        best = Some((*xv, *y));
                    }
                }
            }
            let (bx, by) = best.ok_or("沒有值")?;
            let step = (xb - xa) / (fine - 1) as f64;
            // a flat bottom counts: the value at v is as low (high) as the extreme
            let fv = at(v)?;
            let flat = fv.is_some_and(|f| (f - by).abs() <= 1e-9 * scale);
            if (bx - v).abs() <= (2.0 * step).max(0.002 * (xb - xa)) || flat {
                Ok(format!("在 {bx:.4}"))
            } else {
                Err(format!("其實在 {bx:.4}（f = {by:.4}）"))
            }
        }
        "increasing" | "decreasing" => {
            let sign = if p.kind == "increasing" { 1.0 } else { -1.0 };
            let var = p.var.clone().unwrap_or_else(|| x.name.clone());
            let series: Vec<(String, Vec<Option<f64>>, Vec<f64>)> = if var == x.name {
                vec![(String::new(), ys.clone(), xs.clone())]
            } else {
                let pv = eq.param(&var).ok_or_else(|| format!("沒有參數 {var}"))?;
                let vals = pv.sweep(41);
                let points: Vec<f64> = match p.at.get(&x.name).and_then(Value::as_f64) {
                    Some(v) => vec![v],
                    None => [0.1, 0.3, 0.5, 0.7, 0.9].iter().map(|t| xa + t * (xb - xa)).collect(),
                };
                let mut s = vec![];
                for xv in points {
                    let mut ys = vec![];
                    for v in &vals {
                        let mut o: Vec<(&str, f64)> = when.iter().filter(|w| w.0 != var).cloned().collect();
                        o.push((x.name.as_str(), xv));
                        o.push((var.as_str(), *v));
                        ys.push(value(eq, &node, &o).map_err(|e| e.to_string())?);
                    }
                    s.push((format!("{} = {xv:.3}", x.name), ys, vals.clone()));
                }
                s
            };
            let mut moved = false;
            for (label, ys, xs) in &series {
                let pts: Vec<(f64, f64)> = xs.iter().zip(ys).filter_map(|(a, b)| Some((*a, (*b)?))).collect();
                if pts.len() < 2 {
                    return Err("沒有足夠的值".into());
                }
                for w in pts.windows(2) {
                    let d = (w[1].1 - w[0].1) * sign;
                    if d < -tol {
                        let at = if label.is_empty() { String::new() } else { format!("（{label}）") };
                        return Err(format!("{var} 從 {:.3} 到 {:.3} 時 f 反而{}{at}", w[0].0, w[1].0, if sign > 0.0 { "變小" } else { "變大" }));
                    }
                    if d > tol {
                        moved = true;
                    }
                }
            }
            if moved { Ok(String::new()) } else { Err("完全沒有變".into()) }
        }
        "symmetric" | "odd" => {
            let c = if p.value.is_null() { 0.0 } else { num_or_expr(eq, &p.value, &when)? };
            let r = (c - xa).min(xb - c);
            if r <= 0.0 {
                return Err(format!("{c:.3} 不在橫軸範圍內"));
            }
            let fc = at(c)?.unwrap_or(0.0);
            for i in 1..=50 {
                let d = r * i as f64 / 50.0;
                let (Some(a), Some(b)) = (at(c + d)?, at(c - d)?) else { continue };
                let bad = if p.kind == "symmetric" { (a - b).abs() > tol } else { ((a - fc) + (b - fc)).abs() > tol };
                if bad {
                    return Err(format!("f({:.3}) = {a:.4}，f({:.3}) = {b:.4}", c + d, c - d));
                }
            }
            Ok(String::new())
        }
        "equals" => {
            let other = eq.parse_other(p.expr.as_deref().ok_or("缺少比較的算式")?).map_err(|e| format!("比較的算式：{e}"))?;
            let zs = curve(eq, &other, &xs, &when).map_err(|e| e.to_string())?;
            let mut n = 0;
            let mut worst = 0.0f64;
            for ((xv, a), b) in xs.iter().zip(&ys).zip(&zs) {
                if let (Some(a), Some(b)) = (a, b) {
                    n += 1;
                    let e = (a - b).abs() / a.abs().max(b.abs()).max(1.0);
                    if e > 1e-6 {
                        return Err(format!("{} = {xv:.3} 時 {a:.5} ≠ {b:.5}", x.name));
                    }
                    worst = worst.max(e);
                }
            }
            if n * 10 < xs.len() * 9 { Err("大多數點沒有值".into()) } else { Ok(format!("最大相對差 {worst:.1e}")) }
        }
        k => Err(format!("不認得的性質「{k}」")),
    }
}

/// `problems` + property checks, as the routes answer them.
pub fn report(eq: &Eq) -> Value {
    let problems = problems(eq);
    let checks = if problems.is_empty() { check_properties(eq) } else { vec![] };
    json!({ "ok": problems.is_empty(), "problems": problems, "checks": checks })
}

#[cfg(test)]
mod tests;
