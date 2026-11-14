//! The figure spec (`<name>.plot.json`, docs/plot.md §3): typed where the server needs to
//! look (data, columns, sizes), everything else kept as JSON so fields the editor adds
//! later survive a round trip through Rust.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Current spec version (`"lore_plot": 1`).
pub const VERSION: u32 = 1;

/// Chart types the renderer knows (`commons/ui/plot/plot.js` CHARTS) that draw a table.
/// `equation` (no table, `equation.js`) is [`EQUATION`], kept out of this list so the data
/// tasks (`plot-propose`, `plot-assist`) never offer it.
pub const TYPES: &[&str] = &[
    "line", "area", "bar", "dumbbell", "scatter", "bland", "box", "violin", "hist", "ridge", "ecdf", "heatmap", "roc", "pr", "calib", "dca", "km", "pie", "radar", "contour", "surface",
    "scatter3d",
];

/// The equation chart type (docs/plot.md §10).
pub const EQUATION: &str = "equation";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Table {
    /// Where it came from (file name), shown in the editor only.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub columns: Vec<String>,
    /// Cells: numbers, strings or null.
    pub rows: Vec<Vec<Value>>,
}

impl Table {
    pub fn has(&self, col: &str) -> bool {
        self.columns.iter().any(|c| c == col)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Size {
    /// mm (None: the style's)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    /// mm (None: the style's)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Series {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y2: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truth: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// label, color, marker, dash, width, group, hidden …
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Series {
    /// The columns this series reads.
    pub fn columns(&self) -> Vec<&str> {
        [&self.y, &self.err, &self.x, &self.y2, &self.score, &self.truth].into_iter().filter_map(|c| c.as_deref()).collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    #[serde(rename = "type")]
    pub kind: String,
    /// Table id in `Spec::data` (default: the first table).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// 3-D charts: the z column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<String>,
    /// Box / violin: the condition column (groups × conditions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue: Option<String>,
    #[serde(default)]
    pub series: Vec<Series>,
    /// title, xaxis, yaxis, legend, opts, notes, inset, span …
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Panel {
    /// What this chart type needs and the panel does not name (None = complete).
    pub fn missing(&self) -> Option<&'static str> {
        let has = |c: &Option<String>| c.as_deref().is_some_and(|v| !v.is_empty());
        let sers = |f: &dyn Fn(&Series) -> bool| !self.series.is_empty() && self.series.iter().all(f);
        let ok = match self.kind.as_str() {
            "line" | "area" | "bar" | "radar" => has(&self.x) && sers(&|s| has(&s.y)),
            "dumbbell" => has(&self.x) && self.series.len() >= 2 && sers(&|s| has(&s.y)),
            "pie" => has(&self.x) && self.series.first().is_some_and(|s| has(&s.y)),
            "scatter" => has(&self.x) && (has(&self.y) || self.series.first().is_some_and(|s| has(&s.y))),
            "bland" => self.series.first().is_some_and(|s| has(&s.y) && has(&s.y2)),
            "box" | "violin" | "hist" | "ridge" | "ecdf" => (has(&self.group) && (has(&self.y) || self.series.first().is_some_and(|s| has(&s.y)))) || sers(&|s| has(&s.y)),
            "roc" | "pr" => sers(&|s| (has(&s.score) && has(&s.truth)) || (has(&s.x) || has(&self.x)) && has(&s.y)),
            "calib" | "dca" => sers(&|s| has(&s.score) && has(&s.truth)),
            "km" => has(&self.x) && has(&self.event),
            "contour" | "surface" | "scatter3d" => has(&self.x) && has(&self.y) && (has(&self.z) || self.series.first().is_some_and(|s| has(&s.y))),
            _ => true,
        };
        (!ok).then_some("這種圖需要的欄還沒選")
    }

    pub fn columns(&self) -> Vec<&str> {
        let mut v: Vec<&str> = [&self.x, &self.y, &self.group, &self.size, &self.event, &self.z, &self.hue].into_iter().filter_map(|c| c.as_deref()).collect();
        for s in &self.series {
            v.extend(s.columns());
        }
        v
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    pub lore_plot: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// This figure's own style values (dotted keys, `font.size`).
    #[serde(default)]
    pub set: Map<String, Value>,
    #[serde(default)]
    pub size: Option<Size>,
    #[serde(default)]
    pub data: BTreeMap<String, Table>,
    #[serde(default)]
    pub panels: Vec<Panel>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caption: String,
    /// grid, margin …
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SpecError {
    #[error("不是圖的規格：{0}")]
    Parse(String),
    #[error("規格版本 {0} 比這個程式新，請更新")]
    Newer(u32),
    #[error("{0}")]
    Invalid(String),
}

impl Spec {
    /// Parse and upgrade (older versions → [`VERSION`]).
    pub fn from_json(s: &str) -> Result<Spec, SpecError> {
        let v: Value = serde_json::from_str(s).map_err(|e| SpecError::Parse(e.to_string()))?;
        Spec::from_value(v)
    }

    pub fn from_value(mut v: Value) -> Result<Spec, SpecError> {
        let ver = v.get("lore_plot").and_then(Value::as_u64).ok_or_else(|| SpecError::Parse("缺少 lore_plot 版本".into()))? as u32;
        if ver > VERSION {
            return Err(SpecError::Newer(ver));
        }
        migrate(&mut v, ver);
        serde_json::from_value(v).map_err(|e| SpecError::Parse(e.to_string()))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn table_of(&self, p: &Panel) -> Option<&Table> {
        match &p.data {
            Some(id) => self.data.get(id),
            None => self.data.values().next(),
        }
    }

    /// Structural problems, one line each (empty = usable): unknown chart types, missing
    /// tables or columns, sizes out of range.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.panels.is_empty() {
            out.push("沒有任何一格".into());
        }
        if let Some(s) = self.size
            && [s.w, s.h].into_iter().flatten().any(|v| !(10.0..=1000.0).contains(&v))
        {
            out.push(format!("大小 {}×{} mm 不合理（10–1000）", s.w.unwrap_or(0.0), s.h.unwrap_or(0.0)));
        }
        for (i, p) in self.panels.iter().enumerate() {
            let n = i + 1;
            // an equation panel has no table: its expression is checked instead
            if let Some(eq) = crate::equation::eq_of(p) {
                match eq {
                    Ok(eq) => out.extend(crate::equation::problems(&eq).into_iter().map(|w| format!("第 {n} 格：{w}"))),
                    Err(e) => out.push(format!("第 {n} 格：{e}")),
                }
                continue;
            }
            if !TYPES.contains(&p.kind.as_str()) {
                out.push(format!("第 {n} 格：不認得的種類 {}", p.kind));
            }
            let Some(t) = self.table_of(p) else {
                out.push(format!("第 {n} 格：找不到資料 {}", p.data.as_deref().unwrap_or("")));
                continue;
            };
            for c in p.columns() {
                if !t.has(c) {
                    out.push(format!("第 {n} 格：資料裡沒有「{c}」這欄"));
                }
            }
            if let Some(why) = p.missing() {
                out.push(format!("第 {n} 格：{why}"));
            }
        }
        out
    }

    /// An equation figure (`kind: "equation"`, or a panel of type `equation`).
    pub fn is_equation(&self) -> bool {
        self.rest.get("kind").and_then(Value::as_str) == Some("equation") || self.panels.iter().any(|p| p.kind == "equation")
    }

    /// An equation figure whose panel is a simulation scene (`eq.sim`, docs/plot.md §11).
    pub fn is_simulation(&self) -> bool {
        self.panels.iter().any(|p| p.kind == "equation" && p.rest.get("eq").is_some_and(|e| e.get("sim").is_some_and(|s| !s.is_null())))
    }

    pub fn validate(&self) -> Result<(), SpecError> {
        let p = self.problems();
        if p.is_empty() { Ok(()) } else { Err(SpecError::Invalid(p.join("；"))) }
    }
}

/// Upgrade an older spec in place (only version 1 exists so far).
fn migrate(v: &mut Value, from: u32) {
    if from < VERSION {
        v["lore_plot"] = VERSION.into();
    }
}
