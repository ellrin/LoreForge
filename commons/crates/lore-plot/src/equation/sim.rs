//! Simulation scenes (docs/plot.md §11, docs/living-equations.md tier 2): an equation figure whose
//! `eq.sim` is a run, not a curve — a model fitted to a target by minimising `Σ wᵢ Lᵢ` (`fit1d`,
//! `fit2d`, `classes`) or a clustering algorithm stepping (`cluster`). The browser runs it
//! (`ui/plot/simcore.js`); here the spec is validated ([`problems`]), the term values of the 1D
//! and class scenes are computed the same way (shared cases `ui/plot/simtests.json`), the GIF
//! pacing is the same pure function ([`schedule`]) and a paper's LaTeX is matched to the scenes
//! that fit it ([`suggest`], no model).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::expr::{self, Env, Rng, Val};

pub const MAX_TERMS: usize = 8;
pub const MAX_COMPS: usize = 6;
pub const MAX_SAMPLES: usize = 2000;
pub const MAX_STEPS: usize = 5000;
/// Grid points of the 1D integrals (= simcore.js).
pub const GRID1: usize = 400;
const LN2PI: f64 = 1.837_877_066_409_345_5;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Comp {
    #[serde(default = "one")]
    pub w: f64,
    /// A number (1D) or `[x, y]` (2D).
    #[serde(default)]
    pub mu: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sigma: Option<f64>,
    /// 2D covariance `[sxx, sxy, syy]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cov: Option<Vec<f64>>,
}
fn one() -> f64 {
    1.0
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Target {
    #[serde(default)]
    pub mix: Vec<Comp>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Samples {
    #[serde(default)]
    pub n: usize,
    #[serde(default)]
    pub seed: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Term {
    pub kind: String,
    #[serde(default = "one")]
    pub w: f64,
    #[serde(default = "yes")]
    pub on: bool,
    /// The weight's symbol (`a`, `\lambda`): coloured by lore-notation when registered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `expr`: the term in the safe expression language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
    /// `smooth`: ε.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eps: Option<f64>,
    /// `anchor`: μ₀.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mu0: Option<f64>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Opt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lr: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<usize>,
}

/// A scripted weight change (GIFs; the live view ignores it).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub at: usize,
    pub w: Vec<f64>,
    /// Back to the starting θ at this step (two objectives from the same start in one run).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sim {
    /// fit1d | fit2d | classes | cluster
    pub scene: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples: Option<Samples>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub model: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub classes: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub cluster: Value,
    #[serde(default, rename = "box", skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Vec<f64>>,
    #[serde(default)]
    pub terms: Vec<Term>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opt: Option<Opt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan: Vec<Plan>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

/// The term kinds each scene offers (= `LoreSim.TERMS`).
pub fn kinds(scene: &str) -> &'static [&'static str] {
    match scene {
        "fit1d" => &["kl_pq", "ce", "kl_qp", "nll", "moment", "anchor", "entropy", "mse", "expr"],
        "classes" => &["ce", "smooth", "l2", "entropy", "expr"],
        "fit2d" => &["nll", "kl_pq", "kl_qp"],
        "cluster" => &["kmeans", "em", "ce", "l2", "smooth"],
        _ => &[],
    }
}
/// Names a user term may use (= `LoreSim.EXPR_NAMES`).
pub fn expr_names(scene: &str) -> &'static [&'static str] {
    match scene {
        "fit1d" => &["mu", "sigma"],
        "classes" => &["z", "q", "y", "K"],
        _ => &[],
    }
}

fn fin(v: f64) -> bool {
    v.is_finite()
}

impl Sim {
    fn k_classes(&self) -> (i64, i64) {
        (self.classes["K"].as_f64().unwrap_or(6.0).round() as i64, self.classes["y"].as_f64().unwrap_or(1.0).round() as i64)
    }
}

/// What makes a sim unusable. Empty = usable.
pub fn problems(sim: &Sim) -> Vec<String> {
    let mut out = Vec::new();
    let ks = kinds(&sim.scene);
    if ks.is_empty() {
        return vec![format!("不認得的場景「{}」", sim.scene)];
    }
    if sim.terms.is_empty() {
        out.push("沒有任何一項".into());
    }
    if sim.terms.len() > MAX_TERMS {
        out.push(format!("最多 {MAX_TERMS} 項"));
    }
    let method = sim.cluster["method"].as_str().unwrap_or("kmeans");
    for t in &sim.terms {
        if !ks.contains(&t.kind.as_str()) {
            out.push(format!("這個場景沒有「{}」", t.kind));
            continue;
        }
        if !(fin(t.w) && t.w >= 0.0 && t.w <= 1000.0) {
            out.push(format!("{} 的權重要在 0–1000", t.kind));
        }
        if sim.scene == "cluster" {
            let fits = match method {
                "kmeans" => t.kind == "kmeans",
                "em" => t.kind == "em",
                "ce" => matches!(t.kind.as_str(), "ce" | "l2" | "smooth"),
                _ => false,
            };
            if !fits {
                out.push(format!("「{}」不屬於 {method}", t.kind));
            }
        }
        if let Some(e) = t.eps
            && !(0.0..=1.0).contains(&e)
        {
            out.push("ε 要在 0–1".into());
        }
        if t.mu0.is_some_and(|m| !fin(m)) {
            out.push("μ₀ 不是數".into());
        }
        if t.kind == "expr" {
            let names = expr_names(&sim.scene);
            match t.expr.as_deref() {
                None | Some("") => out.push("自訂項沒有算式".into()),
                Some(src) => {
                    if let Err(e) = expr::parse(src, &|n| names.contains(&n)) {
                        out.push(format!("自訂項：{e}"));
                    }
                }
            }
        }
        if t.kind == "nll" && sim.scene == "fit1d" && sim.samples.as_ref().is_none_or(|s| s.n == 0) {
            out.push("樣本 NLL 要有樣本（samples.n）".into());
        }
    }
    if sim.scene == "cluster" && !matches!(method, "kmeans" | "em" | "ce") {
        out.push(format!("不認得的分群方法「{method}」"));
    }
    if let Some([a, b]) = sim.x
        && !(fin(a) && fin(b) && b > a)
    {
        out.push("橫軸範圍不對".into());
    }
    if let Some(b) = &sim.bbox
        && !(b.len() == 4 && b.iter().all(|v| fin(*v)) && b[1] > b[0] && b[3] > b[2])
    {
        out.push("範圍（box）要是 [x0, x1, y0, y1]".into());
    }
    if sim.scene != "classes" {
        let mix = sim.target.as_ref().map(|t| t.mix.as_slice()).unwrap_or(&[]);
        if mix.is_empty() || mix.len() > MAX_COMPS {
            out.push(format!("目標分佈要有 1–{MAX_COMPS} 個成分"));
        }
        if !mix.iter().any(|c| c.w > 0.0) || mix.iter().any(|c| !(fin(c.w) && c.w >= 0.0)) {
            out.push("成分的權重要 ≥ 0，至少一個 > 0".into());
        }
        let d2 = sim.scene != "fit1d";
        for c in mix {
            if d2 {
                let mu_ok = c.mu.as_array().is_some_and(|a| a.len() == 2 && a.iter().all(|v| v.as_f64().is_some_and(fin)));
                let cov_ok = c.cov.as_ref().is_some_and(|v| v.len() == 3 && v.iter().all(|x| fin(*x)) && v[0] > 0.0 && v[2] > 0.0 && v[0] * v[2] - v[1] * v[1] > 0.0);
                if !mu_ok || !cov_ok {
                    out.push("2D 成分要有 mu [x, y] 與正定的 cov [sxx, sxy, syy]".into());
                }
            } else if !c.mu.as_f64().is_some_and(fin) || !c.sigma.is_some_and(|s| fin(s) && s > 0.0) {
                out.push("1D 成分要有 mu 與 sigma > 0".into());
            }
        }
    } else {
        let (k, y) = sim.k_classes();
        if !(2..=12).contains(&k) || !(1..=k).contains(&y) {
            out.push("類別數 K 要在 2–12，y 在 1–K".into());
        }
    }
    if let Some(s) = &sim.samples
        && s.n > MAX_SAMPLES
    {
        out.push(format!("樣本最多 {MAX_SAMPLES} 個"));
    }
    if matches!(sim.scene.as_str(), "fit2d" | "cluster") {
        let k = sim.model["k"].as_f64().or(sim.cluster["k"].as_f64()).unwrap_or(3.0);
        if !(1.0..=8.0).contains(&k) {
            out.push("成分數 k 要在 1–8".into());
        }
    }
    if let Some(o) = &sim.opt {
        if o.lr.is_some_and(|lr| !(fin(lr) && lr > 0.0 && lr <= 10.0)) {
            out.push("學習率要在 (0, 10]".into());
        }
        if o.steps.is_some_and(|s| s == 0 || s > MAX_STEPS) {
            out.push(format!("步數要在 1–{MAX_STEPS}"));
        }
        if o.method.as_deref().is_some_and(|m| !matches!(m, "adam" | "gd")) {
            out.push("最佳化只能是 adam 或 gd".into());
        }
    }
    for p in &sim.plan {
        if p.w.len() > sim.terms.len() || p.w.iter().any(|w| !(fin(*w) && *w >= 0.0)) {
            out.push(format!("第 {} 步的權重不對", p.at));
        }
    }
    out
}

// ── values (fit1d, classes; = simcore.js) ───────────────────────────────

fn lse(a: &[f64]) -> f64 {
    let m = a.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if !m.is_finite() {
        return m;
    }
    m + a.iter().map(|v| (v - m).exp()).sum::<f64>().ln()
}
fn log_n1(x: f64, mu: f64, s: f64) -> f64 {
    let z = (x - mu) / s;
    -0.5 * z * z - s.ln() - 0.5 * LN2PI
}
/// A standard normal pair (Box–Muller over the shared mulberry32; = `LoreSim.normal`).
pub fn normal(r: &mut Rng) -> (f64, f64) {
    let (u1, u2) = (r.uniform(), r.uniform());
    let m = (-2.0 * u1.ln()).sqrt();
    (m * (2.0 * std::f64::consts::PI * u2).cos(), m * (2.0 * std::f64::consts::PI * u2).sin())
}

struct Mix1 {
    cs: Vec<(f64, f64, f64)>, // log w, mu, sigma
    w: Vec<f64>,
    mean: f64,
    std: f64,
}
fn mix1(t: Option<&Target>) -> Mix1 {
    let raw: Vec<(f64, f64, f64)> = t.map(|t| t.mix.as_slice()).unwrap_or(&[]).iter().map(|c| (c.w.max(0.0), c.mu.as_f64().unwrap_or(0.0), c.sigma.unwrap_or(1.0).max(1e-3))).collect();
    let tot: f64 = raw.iter().map(|c| c.0).sum::<f64>();
    let tot = if tot > 0.0 { tot } else { 1.0 };
    let w: Vec<f64> = raw.iter().map(|c| c.0 / tot).collect();
    let mean: f64 = raw.iter().zip(&w).map(|(c, w)| w * c.1).sum();
    let var: f64 = raw.iter().zip(&w).map(|(c, w)| w * (c.2 * c.2 + c.1 * c.1)).sum::<f64>() - mean * mean;
    Mix1 { cs: raw.iter().zip(&w).map(|(c, w)| (w.max(1e-300).ln(), c.1, c.2)).collect(), w, mean, std: var.max(0.0).sqrt() }
}
impl Mix1 {
    fn logp(&self, x: f64) -> f64 {
        lse(&self.cs.iter().map(|c| c.0 + log_n1(x, c.1, c.2)).collect::<Vec<_>>())
    }
    fn sample(&self, r: &mut Rng) -> f64 {
        let mut u = r.uniform();
        let mut k = 0;
        while k + 1 < self.cs.len() && u > self.w[k] {
            u -= self.w[k];
            k += 1;
        }
        self.cs[k].1 + self.cs[k].2 * normal(r).0
    }
}

/// Each term's value at θ (fit1d: `[μ, log σ]`; classes: the logits z). `None` = no value, and for
/// scenes the browser alone computes (fit2d, cluster).
pub fn values(sim: &Sim, theta: &[f64]) -> Vec<Option<f64>> {
    let terms = &sim.terms[..sim.terms.len().min(MAX_TERMS)];
    let names = expr_names(&sim.scene);
    let user = |t: &Term, env: &Env| -> Option<f64> {
        let node = expr::parse(t.expr.as_deref()?, &|n| names.contains(&n)).ok()?;
        match expr::eval(&node, env).ok()? {
            Val::S(v) => Some(v),
            Val::V(v) if v.len() == 1 => Some(v[0]),
            _ => None,
        }
    };
    match sim.scene.as_str() {
        "fit1d" => {
            let [x0, x1] = sim.x.unwrap_or([-6.0, 6.0]);
            let dx = (x1 - x0) / (GRID1 - 1) as f64;
            let xs: Vec<f64> = (0..GRID1).map(|i| x0 + i as f64 * dx).collect();
            let t = mix1(sim.target.as_ref());
            let lp: Vec<f64> = xs.iter().map(|x| t.logp(*x)).collect();
            let p: Vec<f64> = lp.iter().map(|v| v.exp()).collect();
            let (mu, s) = (theta[0], theta[1].exp());
            let lq: Vec<f64> = xs.iter().map(|x| log_n1(*x, mu, s)).collect();
            let smp: Vec<f64> = match &sim.samples {
                Some(sm) if sm.n > 0 => {
                    let mut r = Rng::new(sm.seed);
                    (0..sm.n.min(MAX_SAMPLES)).map(|_| t.sample(&mut r)).collect()
                }
                _ => vec![],
            };
            terms
                .iter()
                .map(|tm| {
                    let v = match tm.kind.as_str() {
                        "kl_pq" => (0..GRID1).filter(|&j| p[j] > 1e-300).map(|j| p[j] * (lp[j] - lq[j])).sum::<f64>() * dx,
                        "ce" => -(0..GRID1).map(|j| p[j] * lq[j]).sum::<f64>() * dx,
                        "kl_qp" => (0..GRID1).map(|j| (lq[j].exp(), j)).filter(|(q, _)| *q > 1e-300).map(|(q, j)| q * (lq[j] - lp[j])).sum::<f64>() * dx,
                        "nll" if !smp.is_empty() => -smp.iter().map(|x| log_n1(*x, mu, s)).sum::<f64>() / smp.len() as f64,
                        "moment" => (mu - t.mean).powi(2) + (s - t.std).powi(2),
                        "anchor" => (mu - tm.mu0.unwrap_or(0.0)).powi(2),
                        "entropy" => -(s.ln() + 0.5 * (1.0 + LN2PI)),
                        "mse" => (0..GRID1).map(|j| (lq[j].exp() - p[j]).powi(2)).sum::<f64>() * dx,
                        "expr" => {
                            let mut env = Env::new();
                            env.insert("mu".into(), Val::S(mu));
                            env.insert("sigma".into(), Val::S(s));
                            return user(tm, &env).filter(|v| v.is_finite());
                        }
                        _ => return None,
                    };
                    Some(v).filter(|v| v.is_finite())
                })
                .collect()
        }
        "classes" => {
            let (k, y) = sim.k_classes();
            let (k, y) = (k.clamp(2, 12) as usize, (y.clamp(1, k) - 1) as usize);
            let z: Vec<f64> = (0..k).map(|i| theta.get(i).copied().unwrap_or(0.0)).collect();
            let l = lse(&z);
            let lq: Vec<f64> = z.iter().map(|v| v - l).collect();
            let q: Vec<f64> = lq.iter().map(|v| v.exp()).collect();
            terms
                .iter()
                .map(|tm| {
                    let v = match tm.kind.as_str() {
                        "ce" => -lq[y],
                        "smooth" => {
                            let e = tm.eps.unwrap_or(0.1).clamp(0.0, 1.0);
                            -(0..k).map(|i| ((if i == y { 1.0 - e } else { 0.0 }) + e / k as f64) * lq[i]).sum::<f64>()
                        }
                        "l2" => 0.5 * z.iter().map(|v| v * v).sum::<f64>(),
                        "entropy" => q.iter().zip(&lq).filter(|(q, _)| **q > 0.0).map(|(q, l)| q * l).sum(),
                        "expr" => {
                            let mut env = Env::new();
                            env.insert("z".into(), Val::V(z.clone()));
                            env.insert("q".into(), Val::V(q.clone()));
                            env.insert("y".into(), Val::S((y + 1) as f64));
                            env.insert("K".into(), Val::S(k as f64));
                            return user(tm, &env).filter(|v| v.is_finite());
                        }
                        _ => return None,
                    };
                    Some(v).filter(|v| v.is_finite())
                })
                .collect()
        }
        _ => terms.iter().map(|_| None).collect(),
    }
}

// ── GIF pacing (= LoreSim.schedule) ──────────────────────────────────────

/// `frames` step indices: the cumulative visual change (`1 − mix`) mixed with plain time (`mix`),
/// sampled at equal spacing. `diffs[k]` = change from step k−1 to k (`diffs[0]` ignored).
pub fn schedule(diffs: &[f64], frames: usize, mix: f64) -> Vec<usize> {
    let n = diffs.len().saturating_sub(1);
    if n == 0 {
        return vec![0; frames];
    }
    let tot: f64 = diffs[1..].iter().map(|d| d.max(0.0)).sum();
    let w = if tot > 0.0 { 1.0 - mix } else { 0.0 };
    let mut cum = vec![0.0];
    let mut c = 0.0;
    for (k, d) in diffs.iter().enumerate().skip(1) {
        c += d.max(0.0);
        cum.push(if tot > 0.0 { w * c / tot } else { 0.0 } + (1.0 - w) * (k as f64 / n as f64));
    }
    let mut out = Vec::with_capacity(frames);
    let mut k = 0;
    for i in 0..frames {
        let t = if frames == 1 { 1.0 } else { i as f64 / (frames - 1) as f64 };
        while k < n && cum[k] < t - 1e-12 {
            k += 1;
        }
        out.push(k);
    }
    out
}

// ── which scenes fit a paper's equation (no model) ───────────────────────

/// Scene ids (from `equations.json` `scenes`) that fit `latex`, best first. Empty: a plain
/// function (tier 1) is the better view.
pub fn suggest(latex: &str) -> Vec<&'static str> {
    let s = latex.to_lowercase().replace(' ', "");
    let has = |k: &str| s.contains(k);
    let mut out: Vec<&'static str> = Vec::new();
    let mut add = |id: &'static str| {
        if !out.contains(&id) {
            out.push(id);
        }
    };
    // a weighted sum of loss terms: two or more \mathcal{L}, or a loss plus λ·(something)
    let losses = s.matches("\\mathcal{l}").count() + s.matches("\\ell_").count();
    let weighted = ["\\lambda", "\\beta", "\\alpha", "\\gamma"].iter().any(|w| has(w)) && s.contains('+');
    let kl = has("\\mathrm{kl}") || has("d_{kl}") || has("\\operatorname{kl}") || has("{kl}") || has("\\mathbb{kl}") || has("kl(");
    if kl {
        if weighted || losses >= 2 || s.contains('+') {
            add("sim-mix");
        }
        add("sim-modes");
    }
    if (losses >= 3 || (losses >= 2 && weighted)) && s.contains('+') {
        add("sim-mix");
    }
    let softmax = has("softmax") || has("\\frac{e^{z") || has("\\frac{\\exp(z") || has("\\sum_{j}e^{") || has("\\sum_je^{");
    if has("smooth") || (has("\\epsilon") || has("\\varepsilon")) && (has("/k") || has("{k}")) {
        add("sim-softmax");
    }
    if softmax && (has("w") || has("x_i")) && (has("\\sum_i") || has("\\sum_{i") || has("\\frac{1}{n}")) {
        add("sim-partition");
    }
    if softmax || has("y_k\\log") || has("y_{k}\\log") || has("y_c\\log") || has("\\sum_k") && has("\\log") && has("q_k") {
        add("sim-softmax");
    }
    let mixture = has("\\pi_j") || has("\\pi_k") || has("\\pi_{k}") || has("\\pi_{j}");
    if mixture && (has("\\mathcal{n}") || has("n(")) {
        add("sim-mixture");
        add("sim-em");
    }
    if (has("\\|x_i-") || has("\\|x_{i}-") || has("\\left\\|x_i-")) && (has("\\mu_") || has("c_")) && has("^2") && (has("\\min") || has("\\arg") || has("\\sum_{k") || has("\\sum_k")) {
        add("sim-kmeans");
    }
    if (has("\\logp_\\theta(x") || has("\\logp_{\\theta}(x") || has("\\logq_\\theta(x") || has("\\logq_{\\theta}(x")) && (has("\\sum_i") || has("\\sum_{i") || has("\\mathbb{e}")) {
        add("sim-mixture");
    }
    out
}

/// A catalogue scene by id: `{ id, label, latex, sim }`.
pub fn scene(id: &str) -> Option<Value> {
    super::catalogue()["scenes"].as_array()?.iter().find(|s| s["id"] == id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cases() -> Value {
        serde_json::from_str(include_str!("../../../../ui/plot/simtests.json")).unwrap()
    }

    #[test]
    fn shared_cases_match_the_browser() {
        for c in cases()["values"].as_array().unwrap() {
            let sim: Sim = serde_json::from_value(c["sim"].clone()).unwrap();
            assert!(problems(&sim).is_empty(), "{}: {:?}", c["name"], problems(&sim));
            let th: Vec<f64> = c["theta"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            let got = values(&sim, &th);
            for (i, e) in c["expect"].as_array().unwrap().iter().enumerate() {
                match e.as_f64() {
                    None => assert_eq!(got[i], None, "{} term {i}", c["name"]),
                    Some(e) => {
                        let g = got[i].unwrap_or_else(|| panic!("{} term {i}: no value", c["name"]));
                        assert!((g - e).abs() <= 1e-9_f64.max(e.abs() * 1e-9), "{} term {i} ({}): {g} vs {e}", c["name"], sim.terms[i].kind);
                    }
                }
            }
        }
    }

    #[test]
    fn forward_kl_on_the_grid_is_the_closed_form() {
        // KL(N(1, 0.8) ‖ N(0, 1.2))
        let sim: Sim = serde_json::from_value(serde_json::json!({ "scene": "fit1d", "x": [-6, 6], "target": { "mix": [{ "w": 1, "mu": 1, "sigma": 0.8 }] }, "terms": [{ "kind": "kl_pq" }, { "kind": "entropy" }] })).unwrap();
        let v = values(&sim, &[0.0, 1.2f64.ln()]);
        let exact = (1.2f64 / 0.8).ln() + (0.64 + 1.0) / (2.0 * 1.44) - 0.5;
        assert!((v[0].unwrap() - exact).abs() < 1e-7);
        // −H(N(·, 1.2))
        assert!((v[1].unwrap() + (1.2f64.ln() + 0.5 * (1.0 + LN2PI))).abs() < 1e-12);
    }

    #[test]
    fn every_catalogue_scene_is_valid() {
        let cat = super::super::catalogue();
        let scenes = cat["scenes"].as_array().unwrap();
        assert!(scenes.len() >= 7);
        for s in scenes {
            let sim: Sim = serde_json::from_value(s["sim"].clone()).unwrap();
            assert!(problems(&sim).is_empty(), "{}: {:?}", s["id"], problems(&sim));
            assert!(s["latex"].as_str().is_some_and(|l| !l.is_empty()));
            // the spec round-trips (unknown fields kept)
            assert_eq!(serde_json::to_value(&sim).unwrap()["scene"], s["sim"]["scene"]);
        }
        assert!(scene("sim-mix").is_some() && scene("nope").is_none());
    }

    #[test]
    fn bad_specs_are_refused() {
        let bad = |v: Value, want: &str| {
            let sim: Sim = serde_json::from_value(v).unwrap();
            let p = problems(&sim);
            assert!(p.iter().any(|m| m.contains(want)), "{want}: {p:?}");
        };
        let tgt = serde_json::json!({ "mix": [{ "w": 1, "mu": 0, "sigma": 1 }] });
        bad(serde_json::json!({ "scene": "warp" }), "不認得的場景");
        bad(serde_json::json!({ "scene": "fit1d", "target": tgt, "terms": [] }), "沒有任何一項");
        bad(serde_json::json!({ "scene": "fit1d", "target": tgt, "terms": [{ "kind": "kmeans" }] }), "沒有「kmeans」");
        bad(serde_json::json!({ "scene": "fit1d", "target": tgt, "terms": [{ "kind": "kl_pq", "w": -1 }] }), "權重");
        bad(serde_json::json!({ "scene": "fit1d", "target": tgt, "terms": [{ "kind": "expr", "expr": "constructor(mu)" }] }), "自訂項");
        bad(serde_json::json!({ "scene": "fit1d", "target": tgt, "terms": [{ "kind": "expr", "expr": "x + mu" }] }), "自訂項");
        bad(serde_json::json!({ "scene": "fit1d", "target": tgt, "terms": [{ "kind": "nll" }] }), "樣本");
        bad(serde_json::json!({ "scene": "fit1d", "target": { "mix": [{ "w": 1, "mu": 0, "sigma": -1 }] }, "terms": [{ "kind": "ce" }] }), "sigma");
        bad(serde_json::json!({ "scene": "fit2d", "target": { "mix": [{ "w": 1, "mu": [0, 0], "cov": [1, 2, 1] }] }, "terms": [{ "kind": "nll" }] }), "正定");
        bad(serde_json::json!({ "scene": "classes", "classes": { "K": 3, "y": 5 }, "terms": [{ "kind": "ce" }] }), "類別數");
        bad(serde_json::json!({ "scene": "cluster", "cluster": { "method": "kmeans" }, "target": { "mix": [{ "w": 1, "mu": [0, 0], "cov": [1, 0, 1] }] }, "terms": [{ "kind": "em" }] }), "不屬於");
        bad(serde_json::json!({ "scene": "fit1d", "target": tgt, "terms": [{ "kind": "ce" }], "opt": { "lr": 0, "steps": 99999 } }), "學習率");
        bad(serde_json::json!({ "scene": "fit1d", "target": tgt, "terms": [{ "kind": "ce" }], "plan": [{ "at": 3, "w": [1, 2] }] }), "第 3 步");
        bad(serde_json::json!({ "scene": "fit1d", "target": tgt, "samples": { "n": 99999 }, "terms": [{ "kind": "ce" }] }), "樣本最多");
    }

    #[test]
    fn schedule_is_paced_by_change_and_ends_on_the_last_step() {
        let mut d = vec![0.0];
        d.extend((0..100).map(|k| if k < 20 { 10.0 } else { 0.1 }));
        let s = schedule(&d, 40, 0.25);
        assert_eq!((s[0], *s.last().unwrap()), (0, 100));
        assert!(s.windows(2).all(|w| w[1] >= w[0]));
        assert!(s.iter().filter(|&&k| k <= 20).count() > 20);
        assert!(s.iter().filter(|&&k| k > 50).count() >= 4);
        assert_eq!(schedule(&[0.0; 5], 5, 0.25), vec![0, 1, 2, 3, 4]);
        // the browser's schedule gives the same frames (node runs simcore.test.js with the same input)
    }

    #[test]
    fn composite_losses_and_distribution_objectives_get_scenes() {
        let cases: &[(&str, &str)] = &[
            (r"\mathcal{L} = \mathcal{L}_{rec} + \beta\, D_{KL}(q_\phi(z|x) \| p(z))", "sim-mix"),
            (r"\mathrm{KL}(p \| q) = \int p(x) \log \frac{p(x)}{q(x)} dx", "sim-modes"),
            (r"\mathcal{L} = \mathcal{L}_{cls} + \lambda_1 \mathcal{L}_{box} + \lambda_2 \mathcal{L}_{mask}", "sim-mix"),
            (r"y^{LS}_k = y_k (1 - \epsilon) + \epsilon / K", "sim-softmax"),
            (r"p(x) = \sum_{k=1}^{K} \pi_k \mathcal{N}(x \mid \mu_k, \Sigma_k)", "sim-mixture"),
            (r"J = \sum_{i=1}^{N} \min_k \| x_i - \mu_k \|^2", "sim-kmeans"),
            (r"\mathcal{L} = -\frac{1}{N}\sum_i \log \mathrm{softmax}(W x_i)_{y_i}", "sim-partition"),
        ];
        for (tex, want) in cases {
            assert!(suggest(tex).contains(want), "{tex}: {:?}", suggest(tex));
        }
        // a single function: tier 1 is the better view
        for tex in [r"\mathrm{FL}(p_t) = -\alpha_t (1-p_t)^\gamma \log p_t", r"\mathrm{GELU}(x) = x \Phi(x)", r"\mathcal{L}_{\mathrm{Huber}} = \frac{1}{2} a^2"] {
            assert!(suggest(tex).is_empty(), "{tex}: {:?}", suggest(tex));
        }
    }
}
