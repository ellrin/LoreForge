//! The model task `equation-plot` (class `create`, cloud first through `tasks.toml`): a paper's
//! LaTeX + its symbol table rows + the sentence around it → an equation figure. A catalogue
//! template is preferred (its expression is the verified one; the model only gives the paper's
//! values); anything else is written in the safe language with the reduction to a small case
//! stated and 2–5 checkable properties. Nothing is trusted: the answer is parsed, evaluated on
//! a grid and its properties tested ([`super::report`]); one retry with the reason, then the
//! reason goes back to the editor (manual).

use lore_llm::{LlmRequest, Message};
use serde_json::{Map, Value, json};

use super::{Eq, Property, Var, catalogue, problems, template_eq, template_ids};
use crate::propose::Llm;

pub const TASK: &str = "equation-plot";

pub const PROPERTY_KINDS: &[&str] = &["nonneg", "bounded", "zero_at", "min_at", "max_at", "increasing", "decreasing", "symmetric", "odd", "equals", "sums_to"];

/// What the paper says about the equation.
#[derive(Debug, Clone, Default)]
pub struct Ask {
    pub latex: String,
    /// Symbol table rows: "`\gamma`: focusing parameter (= 2)".
    pub symbols: Vec<String>,
    /// The sentence(s) around the equation.
    pub context: String,
}

#[derive(Debug, Clone)]
pub struct Proposal {
    pub eq: Eq,
    /// `{ ok, problems, checks }`
    pub report: Value,
    pub engine: String,
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct Failure {
    pub reason: String,
    /// The last (unusable) answer, for the editor's manual mode.
    pub draft: Option<Eq>,
}

fn kv_list() -> Value {
    json!({ "type": "array", "items": { "type": "object", "required": ["name", "value"], "properties": { "name": { "type": "string" }, "value": { "type": "number" } } } })
}

pub fn schema() -> Value {
    let mut ids: Vec<Value> = template_ids().into_iter().map(Value::from).collect();
    ids.push(Value::Null);
    json!({
        "type": "object",
        "required": ["match", "reduction", "properties"],
        "properties": {
            "match": { "type": "string", "enum": ["template", "custom"] },
            "template": { "type": ["string", "null"], "enum": ids },
            "paper_values": kv_list(),
            "expr": { "type": "string" },
            "output": { "type": "string", "enum": ["curve", "bars"] },
            "x": { "type": "object", "properties": { "name": { "type": "string" }, "label": { "type": "string" }, "min": { "type": "number" }, "max": { "type": "number" } } },
            "vars": { "type": "array", "items": { "type": "object", "required": ["name", "role"], "properties": {
                "name": { "type": "string" }, "label": { "type": "string" }, "role": { "type": "string", "enum": ["param", "vector"] },
                "value": { "type": ["number", "array"], "items": { "type": "number" } }, "min": { "type": "number" }, "max": { "type": "number" },
                "step": { "type": "number" }, "log": { "type": "boolean" }, "integer": { "type": "boolean" } } } },
            "reduction": { "type": "array", "items": { "type": "string" } },
            "properties": { "type": "array", "items": { "type": "object", "required": ["kind", "text"], "properties": {
                "kind": { "type": "string", "enum": PROPERTY_KINDS }, "var": { "type": "string" }, "value": { "type": ["number", "string"] },
                "lo": { "type": "number" }, "hi": { "type": "number" }, "expr": { "type": "string" }, "when": kv_list(), "at": kv_list(),
                "text": { "type": "string" } } } },
            "y_label": { "type": "string" },
            "animate": { "type": "string" },
            "note": { "type": "string" }
        }
    })
}

fn catalogue_text() -> String {
    let mut s = String::new();
    for t in catalogue()["templates"].as_array().into_iter().flatten() {
        let vars: Vec<String> = t["vars"].as_array().into_iter().flatten().map(|v| format!("{}={}", v["name"].as_str().unwrap_or(""), v["value"])).collect();
        s += &format!("- {} ({}): {}   x = {}; params: {}\n", t["id"].as_str().unwrap_or(""), t["label"].as_str().unwrap_or(""), t["latex"].as_str().unwrap_or(""), t["x"]["name"].as_str().unwrap_or("-"), if vars.is_empty() { "none".into() } else { vars.join(", ") });
    }
    s
}

fn system() -> String {
    format!(
        "You turn one equation from a research paper (mostly deep-learning losses and functions) into a small, exact, plottable simulation. Answer with JSON only, matching the schema.\n\n\
1. If the equation is one of these verified templates (same function, possibly other symbols), answer match = \"template\", the template id, and paper_values with the paper's own parameter values (by the template's parameter names). Prefer a template whenever it fits.\n{catalogue}\n\
2. Otherwise answer match = \"custom\" and write expr in this safe language (ASCII names, Greek letters spelled out: gamma, tau…):\n\
   numbers, + - * / ^, comparisons (1/0), && || !, v[i] (1-based), vectors [a, b, c], [cond] = Iverson bracket;\n\
   sum(k, 1, K, body), prod(k, 1, K, body), vec(k, 1, K, body), let(name, value, body), E(x, normal(m, s) | uniform(a, b) | bernoulli(p) | categorical(pvec), body) (fixed-seed Monte Carlo);\n\
   log exp sqrt abs sign min max clamp(x,a,b) pow if(c,a,b) piecewise(c1,v1,c2,v2,…,else) ind(c) sg(x) (stop-gradient = identity) sigmoid softplus relu gelu silu tanh erf log1p;\n\
   vectors: sum(v) mean(v) len(v) dot(u,v) norm(v) norm1(v) cosine(u,v) softmax(v) logsumexp(v) argmax(v);\n\
   loss blocks: ce(p) bce(p,y) focal_w(p,gamma) focal(p,gamma,alpha) dice(p,g[,s]) huber(r,delta) mse(r) mae(r) kl_gauss(mu1,sigma1,mu2,sigma2) info_nce(s_pos, s_negs_vector, tau) triplet(d_ap,d_an,m) smooth_ce(p,eps,K) hinge(x).\n\
   Compose new losses from these blocks. Reduce the paper's notation to a small concrete case (e.g. K = 3 classes, N = 8 negatives with equal similarity, 2-D embeddings) and say each assumption in reduction (short, may use $…$).\n\
   x: the one input that goes on the horizontal axis (the quantity whose effect matters most: a probability, a similarity, a residual, a distance), with a range where the expression is defined (open ends for log: 0.001 not 0).\n\
   vars: every other free name; role param = a slider (value = the paper's value, a sensible min/max around it, step; log = true for scales spanning decades like tau or N; integer = true for counts), role vector = fixed numbers. Labels are LaTeX between $…$ using the paper's symbols.\n\
   output = bars only when the expression gives a vector (e.g. a softmax over classes); then x is omitted.\n\
3. Always give 2–5 properties you expect the function to have, so a program can test them numerically: nonneg; bounded (lo, hi); zero_at (value); min_at / max_at (value: number or an expression over the variables); increasing / decreasing (var: x by default, or a parameter; at: x values to test a parameter at); symmetric / odd (value = centre); equals (expr = another expression, e.g. plain CE when gamma = 0, with when = parameter values); sums_to (bars). text = the property in a few words (Traditional Chinese, $…$ allowed). They must be true of the paper's function, not guesses about training.\n\
4. animate = the parameter the paper itself tunes or varies (its ablation, sensitivity study or hyperparameter choice, read from the context); empty if the paper tunes none. y_label = LaTeX of the output. note = one short sentence only if something is uncertain (else empty).\n\
Never invent values the paper does not give: then use the usual default and say so in reduction.",
        catalogue = catalogue_text()
    )
}

pub fn request(a: &Ask, retry_reason: Option<&str>) -> LlmRequest {
    let mut user = format!("Equation (LaTeX):\n{}\n", a.latex.trim());
    if !a.symbols.is_empty() {
        user += "\nSymbols from the paper:\n";
        for s in a.symbols.iter().take(40) {
            user += &format!("- {s}\n");
        }
    }
    if !a.context.trim().is_empty() {
        user += &format!("\nAround it in the paper:\n{}\n", a.context.chars().take(1500).collect::<String>());
    }
    let mut sys = system();
    if let Some(r) = retry_reason {
        sys += &format!("\n\nYour previous answer could not be used: {r}. Fix it.");
    }
    LlmRequest::chat(sys, vec![Message::user(user)]).json(schema()).max_tokens(4000)
}

fn kv(v: &Value) -> Map<String, Value> {
    v.as_array().into_iter().flatten().filter_map(|o| Some((o["name"].as_str()?.to_string(), o["value"].clone()))).collect()
}

fn property_of(p: &Value) -> Option<Property> {
    let kind = p["kind"].as_str()?.to_string();
    if !PROPERTY_KINDS.contains(&kind.as_str()) {
        return None;
    }
    Some(Property {
        kind,
        var: p["var"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
        value: p["value"].clone(),
        lo: p["lo"].as_f64(),
        hi: p["hi"].as_f64(),
        expr: p["expr"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
        when: kv(&p["when"]),
        at: kv(&p["at"]),
        text: p["text"].as_str().unwrap_or("").to_string(),
    })
}

/// The model's answer → an `eq` (not yet checked).
pub fn to_eq(reply: &Value, a: &Ask) -> Result<Eq, String> {
    let props: Vec<Property> = reply["properties"].as_array().into_iter().flatten().filter_map(property_of).collect();
    let reduction: Vec<String> = reply["reduction"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).filter(|s| !s.trim().is_empty()).collect();
    let mut eq = if reply["match"] == "template" {
        let id = reply["template"].as_str().ok_or("說是模板卻沒有模板 id")?;
        let mut eq = template_eq(id).ok_or_else(|| format!("沒有模板 {id}"))?;
        for (name, v) in kv(&reply["paper_values"]) {
            let Some(v) = v.as_f64() else { continue };
            if let Some(p) = eq.vars.iter_mut().find(|p| p.name == name && p.role == "param") {
                let v = if p.integer { v.round() } else { v };
                // the paper's value always fits on its slider
                if p.min.is_some_and(|m| v < m) {
                    p.min = Some(if p.log { v / 2.0 } else { v });
                }
                if p.max.is_some_and(|m| v > m) {
                    p.max = Some(if p.log { v * 2.0 } else { v });
                }
                p.value = json!(v);
                p.paper = Some(v);
            }
        }
        eq.reduction.extend(reduction);
        // the model's own expectations are tested too (template properties stay)
        let names = eq.names();
        eq.properties.extend(props.into_iter().filter(|p| p.var.as_ref().is_none_or(|v| names.contains(v))));
        eq
    } else {
        let expr = reply["expr"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("沒有算式")?.to_string();
        let output = reply["output"].as_str().filter(|o| *o == "bars").map(str::to_string);
        let x = if output.is_some() {
            None
        } else {
            let x = &reply["x"];
            Some(Var { name: x["name"].as_str().ok_or("沒有橫軸變數")?.to_string(), label: x["label"].as_str().unwrap_or("").to_string(), role: "x".into(), min: x["min"].as_f64(), max: x["max"].as_f64(), ..Default::default() })
        };
        let vars: Vec<Var> = reply["vars"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| {
                let name = v["name"].as_str()?.to_string();
                let role = v["role"].as_str().unwrap_or("param").to_string();
                let value = v["value"].clone();
                let paper = if role == "param" { value.as_f64() } else { None };
                Some(Var { name, label: v["label"].as_str().unwrap_or("").to_string(), role, value, paper, min: v["min"].as_f64(), max: v["max"].as_f64(), step: v["step"].as_f64(), log: v["log"].as_bool().unwrap_or(false), integer: v["integer"].as_bool().unwrap_or(false), color: None })
            })
            .collect();
        Eq { template: None, expr, x, vars, output, reduction, properties: props, ..Default::default() }
    };
    eq.latex = a.latex.trim().to_string();
    if let Some(v) = reply["animate"].as_str().filter(|v| eq.param(v).is_some()) {
        eq.animate = Some(super::Animate { var: v.to_string(), frames: None, ms: None });
        // the model read it from the paper (ablation / hyperparameter text), not a default
        eq.rest.insert("animate_paper".into(), true.into());
    }
    if eq.animate.is_none()
        && let Some(p) = eq.vars.iter().find(|v| v.role == "param")
    {
        eq.animate = Some(super::Animate { var: p.name.clone(), frames: None, ms: None });
    }
    if let Some(y) = reply["y_label"].as_str().filter(|s| !s.trim().is_empty()) {
        eq.rest.insert("y_label".into(), y.into());
    }
    Ok(eq)
}

/// Ask, check, retry once; never returns an `eq` that does not evaluate.
pub async fn propose(llm: &dyn Llm, a: &Ask) -> Result<Proposal, Failure> {
    let mut why: Option<String> = None;
    let mut draft: Option<Eq> = None;
    for _ in 0..2 {
        let (v, engine) = llm.complete_json(TASK, request(a, why.as_deref())).await.map_err(|e| Failure { reason: e, draft: draft.clone() })?;
        match to_eq(&v, a) {
            Ok(eq) => {
                let p = problems(&eq);
                if p.is_empty() {
                    let report = super::report(&eq);
                    return Ok(Proposal { eq, report, engine, note: v["note"].as_str().unwrap_or("").to_string() });
                }
                why = Some(p.join("；"));
                draft = Some(eq);
            }
            Err(e) => why = Some(e),
        }
    }
    Err(Failure { reason: why.unwrap_or_else(|| "模型的回答不能用".into()), draft })
}

/// Whether a proposal may go into a deck with nobody looking (論文報告): a catalogue template,
/// the parameter the paper tunes animated (its value from the paper, and its symbol or name
/// found in `tuning` = the paper's tuning / ablation / hyperparameter text), every check passing.
/// `Err` says why not.
pub fn fit_for_auto(p: &Proposal, tuning: &str) -> Result<(), String> {
    if p.eq.template.is_none() {
        return Err("不是內建的式子（自訂算式要作者確認）".into());
    }
    if p.report["ok"] != true {
        return Err("算式檢查沒過".into());
    }
    if let Some(c) = p.report["checks"].as_array().into_iter().flatten().find(|c| c["ok"] != true) {
        return Err(format!("性質不成立：{}", c["text"].as_str().unwrap_or("")));
    }
    let Some(a) = p.eq.animate.as_ref().filter(|_| p.eq.rest.get("animate_paper") == Some(&Value::Bool(true))) else {
        return Err("論文沒有說調哪個參數".into());
    };
    let v = match p.eq.param(&a.var) {
        Some(v) if v.paper.is_some() => v,
        _ => return Err(format!("{} 沒有論文的值", a.var)),
    };
    // the paper itself must talk about tuning it (the model's word is not enough)
    const GREEK: &[(&str, &str)] = &[("alpha", "α"), ("beta", "β"), ("gamma", "γ"), ("delta", "δ"), ("epsilon", "ε"), ("eps", "ε"), ("lambda", "λ"), ("mu", "μ"), ("sigma", "σ"), ("tau", "τ"), ("theta", "θ"), ("eta", "η"), ("rho", "ρ")];
    let tex = super::label_tex(&v.label);
    let base = tex.split('_').next().unwrap_or("").trim_start_matches('\\').to_string();
    let mut words: Vec<String> = vec![tex.clone(), format!("\\{}", v.name)];
    if v.name.chars().count() > 1 {
        words.push(v.name.clone());
    }
    for (n, g) in GREEK {
        if *n == v.name || *n == base {
            words.push(g.to_string());
            words.push(format!("\\{n}"));
        }
    }
    if words.iter().filter(|w| !w.trim().is_empty()).any(|w| tuning.contains(w.as_str())) {
        Ok(())
    } else {
        Err(format!("論文的調參／消融段落沒有提到 {}", a.var))
    }
}

/// A template guessed from the LaTeX alone (aliases in the catalogue), for places without a model.
pub fn guess_template(latex: &str) -> Option<String> {
    let low = latex.to_lowercase();
    let mut best: Option<(usize, String)> = None;
    for t in catalogue()["templates"].as_array()? {
        let n = t["aliases"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|a| low.contains(&a.to_lowercase())).count();
        if n > 0 && best.as_ref().is_none_or(|b| n > b.0) {
            best = Some((n, t["id"].as_str()?.to_string()));
        }
    }
    best.map(|b| b.1)
}
