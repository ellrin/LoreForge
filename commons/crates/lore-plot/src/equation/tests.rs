//! Equation figures, offline: the expression language (shared cases with eqcore.js, unsafe
//! input), every template against hand-computed points and its own properties, derivatives
//! against analytic ones, the GIF encoder, validation failures, the `equation-plot` task with a
//! fake model (templates and new-style losses through the general path), the routes.

use std::sync::Mutex;

use serde_json::{Value, json};

use super::expr::{self, ExprError};
use super::*;

const CASES: &str = include_str!("../../../../ui/plot/eqtests.json");

fn env_of(vars: &Value) -> (Vec<String>, Env) {
    let mut names = vec![];
    let mut env = Env::new();
    if let Some(o) = vars.as_object() {
        for (k, v) in o {
            names.push(k.clone());
            env.insert(k.clone(), match v {
                Value::Array(a) => Val::V(a.iter().map(|x| x.as_f64().unwrap()).collect()),
                x => Val::S(x.as_f64().unwrap()),
            });
        }
    }
    (names, env)
}

#[test]
fn shared_cases_evaluate_to_the_hand_computed_values() {
    let cases: Value = serde_json::from_str(CASES).unwrap();
    for c in cases["values"].as_array().unwrap() {
        let src = c["expr"].as_str().unwrap();
        let (names, env) = env_of(&c["vars"]);
        let node = expr::parse(src, &|n| names.iter().any(|m| m == n)).unwrap_or_else(|e| panic!("{src}: {e}"));
        let got = expr::eval(&node, &env).unwrap_or_else(|e| panic!("{src}: {e}")).scalar().unwrap();
        match c["expect"].as_f64() {
            None => assert!(got.is_nan(), "{src}: {got} (expected no value)"),
            Some(want) => {
                let tol = c["tol"].as_f64().unwrap_or(1e-12);
                assert!((got - want).abs() <= tol.max(want.abs() * 1e-12), "{src}: {got} ≠ {want}");
            }
        }
    }
}

#[test]
fn unsafe_or_broken_input_is_refused() {
    let cases: Value = serde_json::from_str(CASES).unwrap();
    for c in cases["reject"].as_array().unwrap() {
        let src = c["expr"].as_str().unwrap();
        let names: Vec<String> = c["vars"].as_array().into_iter().flatten().map(|v| v.as_str().unwrap().to_string()).collect();
        assert!(expr::parse(src, &|n| names.iter().any(|m| m == n)).is_err(), "accepted: {src}");
    }
    for c in cases["budget"].as_array().unwrap() {
        let src = c["expr"].as_str().unwrap();
        let n = expr::parse_with(src, &[]).unwrap();
        assert!(expr::eval(&n, &Env::new()).is_err(), "no limit on: {src}");
    }
    // nesting and length caps (a recursive parser must not overflow the stack)
    let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    assert_eq!(expr::parse_with(&deep, &[]), Err(ExprError::TooLong));
    let deep = format!("{}1{}", "(".repeat(300), ")".repeat(300));
    assert_eq!(expr::parse_with(&deep, &[]), Err(ExprError::TooDeep));
    assert_eq!(expr::parse_with(&"-".repeat(400).to_string(), &[]).unwrap_err(), ExprError::TooDeep);
    assert_eq!(expr::parse_with(&format!("{}1{}", "[".repeat(300), "]".repeat(300)), &[]).unwrap_err(), ExprError::TooDeep);
    assert_eq!(expr::parse_with(&format!("{}x{}", "log(".repeat(100), ")".repeat(100)), &["x"]).unwrap_err(), ExprError::TooDeep);
    assert!(matches!(expr::parse_with("1 $ 2", &[]), Err(ExprError::Syntax(3, _))));
}

#[test]
fn monte_carlo_draws_the_same_samples_as_the_browser() {
    let cases: Value = serde_json::from_str(CASES).unwrap();
    for c in cases["mc"].as_array().unwrap() {
        let src = c["expr"].as_str().unwrap();
        let (names, env) = env_of(&c["vars"]);
        let n = expr::parse(src, &|x| names.iter().any(|m| m == x)).unwrap();
        let got = expr::eval_scalar(&n, &env).unwrap();
        let want = c["expect"].as_f64().unwrap();
        assert!((got - want).abs() <= 1e-12 * want.abs().max(1.0), "{src}: {got} ≠ {want}");
    }
}

#[test]
fn monte_carlo_is_deterministic() {
    let a = expr::eval_str("E(z, normal(mu, 1), z^2)", &[("mu", 0.5)]).unwrap();
    let b = expr::eval_str("E(z, normal(mu, 1), z^2)", &[("mu", 0.5)]).unwrap();
    assert_eq!(a, b);
    // common random numbers: the curve over mu is smooth (E[z²] = 1 + mu² up to MC noise, same draws)
    let c = expr::eval_str("E(z, normal(mu, 1), z^2)", &[("mu", 0.6)]).unwrap();
    assert!((c - a - (0.36 - 0.25)).abs() < 0.03, "{a} {c}");
}

fn tmpl(id: &str) -> (Eq, Node) {
    let eq = template_eq(id).unwrap_or_else(|| panic!("no template {id}"));
    let n = eq.parse().unwrap();
    (eq, n)
}

#[test]
fn every_template_is_valid_and_has_its_properties() {
    let ids = template_ids();
    assert!(ids.len() >= 17, "{ids:?}");
    for want in ["ce", "bce", "focal", "dice", "huber", "mse", "mae", "kl_gauss", "infonce", "triplet", "smooth_ce", "softmax_t", "relu", "gelu", "silu", "sigmoid", "tanh"] {
        assert!(ids.iter().any(|i| i == want), "missing {want}");
    }
    let cat = catalogue();
    for id in &ids {
        let (eq, _) = tmpl(id);
        assert_eq!(problems(&eq), Vec::<String>::new(), "{id}");
        assert!(eq.properties.len() >= 2, "{id}: at least two properties");
        for r in check_properties(&eq) {
            assert!(r.ok, "{id}: {} — {}", r.text, r.detail);
        }
        let t = cat["templates"].as_array().unwrap().iter().find(|t| t["id"] == id.as_str()).unwrap();
        assert!(!t["latex"].as_str().unwrap().is_empty() && !t["label"].as_str().unwrap().is_empty(), "{id}");
        for v in eq.vars.iter().filter(|v| v.role == "param") {
            let (a, b) = v.range().unwrap();
            let x = v.num().unwrap();
            assert!(a < b && x >= a && x <= b && v.step.unwrap() > 0.0, "{id}.{}", v.name);
        }
        if let Some(a) = &eq.animate {
            assert!(eq.param(&a.var).is_some(), "{id}: animates {}", a.var);
        }
        // the whole figure spec is valid too
        let spec = crate::Spec::from_value(spec_for(&eq, "clean")).unwrap();
        assert!(spec.is_equation());
        assert_eq!(spec.problems(), Vec::<String>::new(), "{id}");
    }
}

#[test]
fn templates_match_hand_computed_points() {
    let at = |id: &str, over: &[(&str, f64)]| {
        let (eq, n) = tmpl(id);
        value(&eq, &n, over).unwrap().unwrap()
    };
    let close = |a: f64, b: f64, tol: f64| assert!((a - b).abs() <= tol, "{a} vs {b}");
    close(at("ce", &[("p", 0.5)]), std::f64::consts::LN_2, 1e-12);
    close(at("bce", &[("p", 0.8), ("y", 1.0)]), 0.2231435513142097, 1e-12);
    close(at("bce", &[("p", 0.8), ("y", 0.0)]), 1.6094379124341003, 1e-12);
    close(at("focal", &[("p", 0.5)]), 0.04332169878499658, 1e-12); // γ = 2, α = 0.25
    close(at("focal", &[("p", 0.5), ("gamma", 0.0), ("alpha", 1.0)]), std::f64::consts::LN_2, 1e-12);
    close(at("dice", &[("p", 0.5)]), 1.0 - 21.0 / 39.0, 1e-12); // F 20, B 80, q 0.1, s 1
    close(at("huber", &[("r", 0.5)]), 0.125, 1e-12);
    close(at("huber", &[("r", -2.0)]), 1.5, 1e-12);
    close(at("mse", &[("r", -1.5)]), 2.25, 1e-12);
    close(at("mae", &[("r", -1.5)]), 1.5, 1e-12);
    close(at("kl_gauss", &[("mu1", 1.0)]), 0.5, 1e-12);
    close(at("kl_gauss", &[("mu1", 0.0), ("sigma1", 2.0)]), 0.8068528194400546, 1e-12);
    close(at("infonce", &[("spos", 0.5), ("tau", 0.1)]), 0.052500962056356286, 1e-12); // N 8, s⁻ 0
    close(at("triplet", &[("dap", 1.0), ("dan", 0.9)]), 0.3, 1e-12);
    close(at("smooth_ce", &[("p", 0.9)]), 0.5008609395783458, 1e-12); // ε 0.1, K 10
    close(at("relu", &[("x", -2.0), ("a", 0.1)]), -0.2, 1e-12);
    close(at("gelu", &[("x", 1.0)]), 0.8413447460685429, 1e-6);
    close(at("silu", &[("x", 1.0)]), 0.7310585786300049, 1e-12);
    close(at("sigmoid", &[("x", 0.0)]), 0.5, 1e-12);
    close(at("tanh", &[("x", 0.5)]), 0.46211715726000974, 1e-12);
    let (eq, n) = tmpl("softmax_t");
    let v = expr::eval(&n, &eq.env(&[("tau", 0.5)])).unwrap().as_vec();
    close(v[0], 0.8372831813099705, 1e-12);
    close(v.iter().sum(), 1.0, 1e-12);
}

#[test]
fn derivatives_match_the_analytic_ones() {
    let close = |a: f64, b: f64| assert!((a - b).abs() <= 1e-5 * b.abs().max(1.0), "{a} vs {b}");
    // CE: d/dp −log p = −1/p
    let (eq, n) = tmpl("ce");
    close(partial(&eq, &n, "p", &[("p", 0.3)]).unwrap().unwrap(), -1.0 / 0.3);
    // focal: ∂/∂γ [−α (1−p)^γ log p] = −α (1−p)^γ ln(1−p) log p
    let (eq, n) = tmpl("focal");
    let (p, g, a) = (0.4f64, 2.0f64, 0.25f64);
    close(partial(&eq, &n, "gamma", &[("p", p)]).unwrap().unwrap(), -a * (1.0 - p).powf(g) * (1.0 - p).ln() * p.ln());
    // and ∂/∂p = α[γ(1−p)^{γ−1} log p − (1−p)^γ / p]
    close(partial(&eq, &n, "p", &[("p", p)]).unwrap().unwrap(), a * (g * (1.0 - p).powf(g - 1.0) * p.ln() - (1.0 - p).powf(g) / p));
    // Huber, |r| > δ: ∂/∂δ = |r| − δ
    let (eq, n) = tmpl("huber");
    close(partial(&eq, &n, "delta", &[("r", 2.0)]).unwrap().unwrap(), 2.0 - 1.0);
    // InfoNCE = softplus(u), u = (s⁻ − s⁺)/τ + log N: ∂/∂τ = σ(u)·(s⁺ − s⁻)/τ²
    let (eq, n) = tmpl("infonce");
    let (sp, tau, nn) = (0.5f64, 0.07f64, 8.0f64);
    let u = (0.0 - sp) / tau + nn.ln();
    close(partial(&eq, &n, "tau", &[("spos", sp)]).unwrap().unwrap(), expr::sigmoid(u) * sp / (tau * tau));
    // 敏感度: normalised, the largest is ±1; at an easy example (p_t = 0.9) γ matters, signs right
    let (eq, n) = tmpl("focal");
    let s = sensitivity(&eq, &n, 0.9).unwrap();
    assert_eq!(s.len(), 2);
    assert!(s.iter().any(|x| (x.norm.abs() - 1.0).abs() < 1e-12));
    let g = s.iter().find(|x| x.name == "gamma").unwrap();
    let al = s.iter().find(|x| x.name == "alpha").unwrap();
    assert!(g.d < 0.0 && al.d > 0.0, "{s:?}");
    close(g.scaled, g.d * 5.0);
}

#[test]
fn sweep_is_a_ping_pong_without_stalls() {
    let (eq, _) = tmpl("focal");
    let g = eq.param("gamma").unwrap();
    let v = sweep_frames(g, 24);
    assert_eq!(v.len(), 24);
    assert_eq!(v[0], 0.0);
    assert_eq!(v.iter().filter(|x| **x == 5.0).count(), 1);
    assert!(v.windows(2).all(|w| w[0] != w[1]));
    assert!(v.last().unwrap() > &v[0], "the loop's last frame is not the first again");
    // log parameters sweep in log space
    let (eq, _) = tmpl("infonce");
    let t = sweep_frames(eq.param("tau").unwrap(), 9);
    assert!((t[2] / t[1] - t[1] / t[0]).abs() < 1e-9);
}

fn synthetic_frames(n: usize) -> Vec<gif::Rgba> {
    // white page, a black axis and a blue anti-aliased curve that moves
    (0..n)
        .map(|k| {
            let (w, h) = (120u32, 80u32);
            let mut px = vec![255u8; (w * h * 4) as usize];
            for y in 0..h {
                for x in 0..w {
                    let i = ((y * w + x) * 4) as usize;
                    if x == 10 || y == 70 {
                        px[i..i + 3].copy_from_slice(&[0, 0, 0]);
                    }
                    let cy = 60.0 - (x as f64 / 6.0 + k as f64).sin() * 20.0;
                    let d = (y as f64 - cy).abs();
                    if d < 2.0 {
                        let t = (1.0 - d / 2.0).clamp(0.0, 1.0);
                        let c = [0.0, 114.0, 178.0];
                        for ch in 0..3 {
                            px[i + ch] = (255.0 + (c[ch] - 255.0) * t).round() as u8;
                        }
                    }
                }
            }
            gif::Rgba { w, h, px }
        })
        .collect()
}

#[test]
fn gif_has_every_frame_loops_and_carries_the_spec() {
    let frames = synthetic_frames(24);
    let spec = r#"{"lore_plot":1,"kind":"equation"}"#;
    let bytes = gif::encode(&frames, 80, Some(spec)).unwrap();
    let info = gif::info(&bytes).unwrap();
    assert_eq!((info.w, info.h, info.frames), (120, 80, 24));
    assert_eq!(info.loops, Some(0), "loops forever");
    assert_eq!(info.delay_cs, Some(8));
    assert_eq!(gif::spec_from_gif(&bytes).as_deref(), Some(spec));
    // one global palette, white / black / the line colour exact
    let pal = gif::palette(&frames);
    assert!(pal.len() <= 255, "one index stays free for unchanged pixels");
    for c in [[255, 255, 255], [0, 0, 0], [0, 114, 178]] {
        assert!(pal.contains(&c), "{c:?}");
    }
    // decoded: the corner is pure white, the axis pure black
    let mut opts = ::gif::DecodeOptions::new();
    opts.set_color_output(::gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(std::io::Cursor::new(&bytes)).unwrap();
    let f = dec.read_next_frame().unwrap().unwrap();
    assert_eq!(&f.buffer[0..4], &[255, 255, 255, 255]);
    let i = ((70 * 120 + 50) * 4) as usize;
    assert_eq!(&f.buffer[i..i + 3], &[0, 0, 0]);
    // not a GIF / no spec
    assert!(gif::info(b"GIF89").is_none());
    assert_eq!(gif::spec_from_gif(&gif::encode(&frames[..2], 80, None).unwrap()), None);
    assert!(gif::encode(&[], 80, None).is_err());
}

#[test]
fn many_colours_still_fit_one_palette() {
    let (w, h) = (64u32, 64u32);
    let px: Vec<u8> = (0..w * h).flat_map(|i| [(i % 256) as u8, (i / 16 % 256) as u8, (i * 7 % 256) as u8, 255]).collect();
    let fr = vec![gif::Rgba { w, h, px }];
    let pal = gif::palette(&fr);
    assert!(pal.len() <= 256 && pal.len() > 100);
    let b = gif::encode(&fr, 50, None).unwrap();
    assert_eq!(gif::info(&b).unwrap().frames, 1);
}

#[test]
fn raster_resolution_is_capped() {
    assert_eq!(raster::dpi_for(85.0, 200, 1200), 200);
    assert!(raster::dpi_for(240.0, 300, 1200) as f64 * 240.0 / 25.4 <= 1200.0);
    let tools = crate::Tools::none();
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="85mm" height="60mm" viewBox="0 0 240 170"></svg>"#.to_string();
    assert!(matches!(raster::rasterize(&tools, &[svg], 200, 1200), Err(crate::export::ExportError::NoChrome)));
}

fn custom(expr: &str, x: (&str, f64, f64), vars: Value) -> Eq {
    Eq {
        expr: expr.into(),
        x: Some(Var { name: x.0.into(), role: "x".into(), min: Some(x.1), max: Some(x.2), ..Default::default() }),
        vars: serde_json::from_value(vars).unwrap(),
        ..Default::default()
    }
}

#[test]
fn validation_says_why() {
    let ok = custom("x^2", ("x", -1.0, 1.0), json!([]));
    assert!(problems(&ok).is_empty());
    let p = |e: &Eq| problems(e).join("；");
    assert!(p(&custom("x^", ("x", -1.0, 1.0), json!([]))).contains("算式"));
    assert!(p(&custom("y + 1", ("x", -1.0, 1.0), json!([]))).contains("y"));
    assert!(p(&custom("x", ("x", 1.0, -1.0), json!([]))).contains("範圍"));
    assert!(p(&custom("log(-x)", ("x", 0.1, 1.0), json!([]))).contains("有值"));
    assert!(p(&custom("exp(x)", ("x", 0.0, 100.0), json!([]))).contains("10⁹"));
    assert!(p(&custom("[x, x]", ("x", 0.0, 1.0), json!([]))).contains("向量"));
    assert!(p(&custom("a * x", ("x", 0.0, 1.0), json!([{ "name": "a", "role": "param", "value": 5, "min": 0, "max": 1, "step": 0.1 }]))).contains("不在範圍"));
    assert!(p(&custom("a * x", ("x", 0.0, 1.0), json!([{ "name": "a", "role": "param", "value": 1, "min": 0, "max": 2, "step": 0.1, "log": true }]))).contains("對數"));
    assert!(p(&custom("log(x - a)", ("x", 0.0, 1.0), json!([{ "name": "a", "role": "param", "value": -1, "min": -1, "max": 2, "step": 0.1 }]))).contains("a = 2"));
    let bars = Eq { expr: "softmax(z)".into(), output: Some("bars".into()), vars: serde_json::from_value(json!([{ "name": "z", "role": "vector", "value": [1, 2] }])).unwrap(), ..Default::default() };
    assert!(problems(&bars).is_empty());
    let bars2 = Eq { expr: "sum(z)".into(), ..bars.clone() };
    assert!(problems(&bars2).join("").contains("向量"));
    // a whole spec with a broken equation is not valid
    let mut v = spec_for(&custom("x^", ("x", 0.0, 1.0), json!([])), "clean");
    assert!(crate::Spec::from_value(v.clone()).unwrap().validate().is_err());
    v["panels"][0]["eq"] = json!(null);
    assert!(crate::Spec::from_value(v).unwrap().problems().join("").contains("eq"));
}

// ── the model task ──────────────────────────────────────────────────────

struct FakeLlm {
    replies: Mutex<Vec<Value>>,
    seen: Mutex<Vec<lore_llm::LlmRequest>>,
}

impl FakeLlm {
    fn new(r: Vec<Value>) -> FakeLlm {
        FakeLlm { replies: Mutex::new(r), seen: Mutex::new(vec![]) }
    }
}

#[async_trait::async_trait]
impl crate::propose::Llm for FakeLlm {
    async fn complete_json(&self, task: &str, req: lore_llm::LlmRequest) -> Result<(Value, String), String> {
        assert_eq!(task, propose::TASK);
        self.seen.lock().unwrap().push(req);
        let mut r = self.replies.lock().unwrap();
        if r.is_empty() { Err("no more".into()) } else { Ok((r.remove(0), "fake".into())) }
    }
}

fn ask(latex: &str) -> propose::Ask {
    propose::Ask { latex: latex.into(), symbols: vec!["$\\gamma$: focusing parameter".into()], context: "We set γ = 2 and α = 0.25.".into() }
}

#[tokio::test]
async fn template_match_takes_the_papers_values() {
    let llm = FakeLlm::new(vec![json!({
        "match": "template", "template": "focal",
        "paper_values": [{ "name": "gamma", "value": 2 }, { "name": "alpha", "value": 0.25 }],
        "reduction": [], "properties": [{ "kind": "nonneg", "text": "≥ 0" }], "animate": "gamma"
    })]);
    let pr = propose::propose(&llm, &ask("FL(p_t) = -\\alpha_t (1-p_t)^\\gamma \\log(p_t)")).await.unwrap();
    assert_eq!(pr.eq.template.as_deref(), Some("focal"));
    assert_eq!(pr.eq.param("gamma").unwrap().paper, Some(2.0));
    assert!(pr.eq.latex.contains("\\gamma"));
    assert_eq!(pr.report["ok"], true);
    assert!(pr.report["checks"].as_array().unwrap().iter().all(|c| c["ok"] == true));
    // the request carries the LaTeX, the symbols, the context and the closed schema
    let seen = llm.seen.lock().unwrap();
    let body = serde_json::to_string(&seen[0].messages).unwrap();
    assert!(body.contains("focusing parameter") && body.contains("γ = 2"));
    assert!(seen[0].system.contains("focal") && seen[0].system.contains("info_nce"));
    // a value outside the template's slider widens it
    let llm = FakeLlm::new(vec![json!({ "match": "template", "template": "infonce", "paper_values": [{ "name": "N", "value": 65536 }, { "name": "tau", "value": 0.2 }], "reduction": [], "properties": [] })]);
    let pr = propose::propose(&llm, &ask("InfoNCE")).await.unwrap();
    let n = pr.eq.param("N").unwrap();
    assert_eq!(n.num(), Some(65536.0));
    assert!(n.max.unwrap() >= 65536.0);
    assert!(pr.report["ok"] == true, "{}", pr.report);
}

/// New-style losses that are not templates: they go through the general path.
fn new_style() -> Vec<(&'static str, Value)> {
    vec![
        (
            "margin-scaled softmax with temperature",
            json!({ "match": "custom", "expr": "let(t, s * (c - m), logsumexp(vec(j, 1, K, if(j == 1, t, s * cneg))) - t)",
                "x": { "name": "c", "label": "$\\cos\\theta_y$", "min": -1, "max": 1 },
                "vars": [{ "name": "s", "role": "param", "value": 30, "min": 1, "max": 64, "step": 1 }, { "name": "m", "role": "param", "value": 0.35, "min": 0, "max": 0.8, "step": 0.05 },
                         { "name": "K", "role": "param", "value": 10, "min": 2, "max": 1000, "step": 1, "integer": true, "log": true }, { "name": "cneg", "role": "param", "value": 0, "min": -1, "max": 1, "step": 0.05 }],
                "reduction": ["$K - 1$ other classes all at cosine $c^-$"],
                "properties": [{ "kind": "nonneg", "text": "≥ 0" }, { "kind": "decreasing", "text": "↓ in cos" }, { "kind": "increasing", "var": "m", "text": "↑ in margin" },
                               { "kind": "equals", "expr": "softplus(s * cneg - s * (c - m) + log(K - 1))", "text": "closed form" }] }),
        ),
        (
            "polynomial expansion of CE (Poly-1)",
            json!({ "match": "custom", "expr": "ce(p) + eps * (1 - p)", "x": { "name": "p", "min": 0.001, "max": 1 },
                "vars": [{ "name": "eps", "role": "param", "value": 1, "min": -1, "max": 5, "step": 0.1 }],
                "reduction": ["one term of the Taylor expansion"],
                "properties": [{ "kind": "nonneg", "text": "≥ 0" }, { "kind": "zero_at", "value": 1, "text": "0 at p = 1" }, { "kind": "equals", "expr": "ce(p)", "when": [{ "name": "eps", "value": 0 }], "text": "CE when ε = 0" }, { "kind": "decreasing", "text": "↓" }] }),
        ),
        (
            "weighted Dice + BCE",
            json!({ "match": "custom", "expr": "let(P, [p, p, q, q], w * dice(P, [1, 1, 0, 0], 1) + (1 - w) * mean(bce(P, [1, 1, 0, 0])))",
                "x": { "name": "p", "min": 0.01, "max": 0.99 },
                "vars": [{ "name": "q", "role": "param", "value": 0.2, "min": 0.01, "max": 0.99, "step": 0.01 }, { "name": "w", "role": "param", "value": 0.5, "min": 0, "max": 1, "step": 0.05 }],
                "reduction": ["4 pixels: 2 foreground predicted $p$, 2 background predicted $q$"],
                "properties": [{ "kind": "nonneg", "text": "≥ 0" }, { "kind": "decreasing", "text": "↓ in p" }, { "kind": "increasing", "var": "q", "text": "↑ in q" }] }),
        ),
        (
            "expectation-based contrastive term",
            json!({ "match": "custom", "expr": "E(n, normal(0, sigma), softplus((sneg - (spos + n)) / tau))",
                "x": { "name": "spos", "min": -1, "max": 1 },
                "vars": [{ "name": "sigma", "role": "param", "value": 0.1, "min": 0, "max": 0.5, "step": 0.01 }, { "name": "tau", "role": "param", "value": 0.1, "min": 0.02, "max": 1, "step": 0.01, "log": true }, { "name": "sneg", "role": "param", "value": 0, "min": -1, "max": 1, "step": 0.05 }],
                "reduction": ["the positive similarity jittered by $\\mathcal{N}(0, \\sigma^2)$, 256 fixed draws"],
                "properties": [{ "kind": "nonneg", "text": "≥ 0" }, { "kind": "decreasing", "text": "↓ in s⁺" }, { "kind": "equals", "expr": "softplus((sneg - spos) / tau)", "when": [{ "name": "sigma", "value": 0 }], "text": "no jitter = logistic" }] }),
        ),
    ]
}

#[tokio::test]
async fn new_losses_go_through_the_general_path_and_pass_their_checks() {
    for (name, reply) in new_style() {
        let llm = FakeLlm::new(vec![reply]);
        let pr = propose::propose(&llm, &ask("\\mathcal{L} = …")).await.unwrap_or_else(|f| panic!("{name}: {}", f.reason));
        assert!(pr.eq.template.is_none(), "{name}");
        assert!(!pr.eq.reduction.is_empty(), "{name}");
        assert_eq!(pr.report["ok"], true, "{name}");
        for c in pr.report["checks"].as_array().unwrap() {
            assert_eq!(c["ok"], true, "{name}: {} {}", c["text"], c["detail"]);
        }
        assert!(pr.eq.animate.is_some(), "{name}");
        assert!(crate::Spec::from_value(spec_for(&pr.eq, "clean")).unwrap().validate().is_ok(), "{name}");
    }
}

#[tokio::test]
async fn a_wrong_expression_is_caught_by_its_properties() {
    // the margin's sign flipped: still evaluates, but "↑ in margin" is now false
    let (_, mut reply) = new_style().remove(0);
    reply["expr"] = json!("let(t, s * (c + m), logsumexp(vec(j, 1, K, if(j == 1, t, s * cneg))) - t)");
    let pr = propose::propose(&FakeLlm::new(vec![reply]), &ask("x")).await.unwrap();
    let checks = pr.report["checks"].as_array().unwrap();
    let bad: Vec<&Value> = checks.iter().filter(|c| c["ok"] == false).collect();
    assert!(bad.iter().any(|c| c["text"] == "↑ in margin"), "{checks:?}");
    assert!(bad.iter().any(|c| c["text"] == "closed form"), "{checks:?}");
    assert!(bad.iter().all(|c| !c["detail"].as_str().unwrap().is_empty()));
}

#[tokio::test]
async fn unusable_answers_are_retried_then_reported() {
    // first answer does not parse, second is fine
    let good = new_style().remove(1).1;
    let llm = FakeLlm::new(vec![json!({ "match": "custom", "expr": "ce(p) +", "x": { "name": "p", "min": 0.01, "max": 1 }, "vars": [], "reduction": [], "properties": [] }), good]);
    let pr = propose::propose(&llm, &ask("x")).await.unwrap();
    assert_eq!(pr.eq.expr, "ce(p) + eps * (1 - p)");
    let seen = llm.seen.lock().unwrap();
    assert!(seen[1].system.contains("could not be used"), "the retry says why");
    drop(seen);
    // both broken: the reason and the draft come back (the editor goes manual)
    let bad = json!({ "match": "custom", "expr": "log(p - 2)", "x": { "name": "p", "min": 0, "max": 1 }, "vars": [], "reduction": [], "properties": [] });
    let f = propose::propose(&FakeLlm::new(vec![bad.clone(), bad]), &ask("x")).await.unwrap_err();
    assert!(f.reason.contains("有值"), "{}", f.reason);
    assert_eq!(f.draft.unwrap().expr, "log(p - 2)");
    // unknown template
    let f = propose::propose(&FakeLlm::new(vec![json!({ "match": "template", "template": "nope", "reduction": [], "properties": [] }); 2]), &ask("x")).await.unwrap_err();
    assert!(f.reason.contains("nope"));
    // the model is down
    let f = propose::propose(&FakeLlm::new(vec![]), &ask("x")).await.unwrap_err();
    assert_eq!(f.reason, "no more");
}

#[test]
fn templates_are_guessed_from_latex() {
    assert_eq!(propose::guess_template("FL = -\\alpha (1-p_t)^\\gamma \\log p_t  focal loss").as_deref(), Some("focal"));
    assert_eq!(propose::guess_template("\\mathcal{L}_{InfoNCE} with temperature \\tau and negatives").as_deref(), Some("infonce"));
    assert_eq!(propose::guess_template("a + b = c"), None);
}

#[test]
fn the_task_is_in_the_table() {
    let t = crate::propose::task_table();
    let s = format!("{t:?}");
    assert!(s.contains("equation-plot"));
    assert!(crate::propose::TASKS.contains("[task.equation-plot]\nlabel = \"方程式動起來\"\nclass = \"create\""));
    // the data tasks never offer the equation type
    assert!(!crate::spec::TYPES.contains(&crate::spec::EQUATION));
}

#[test]
fn browser_core_passes_the_same_cases_under_node() {
    for f in ["eqcore.test.js", "eqcolors.test.js", "simcore.test.js", "formats.test.js", "dumbbell.test.js"] {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/plot/tests").join(f);
        match std::process::Command::new("node").arg(&script).output() {
            Ok(o) => assert!(o.status.success(), "node {f}:\n{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
            Err(_) => eprintln!("node not installed: {f} skipped"),
        }
    }
}

#[tokio::test]
async fn only_safe_answers_go_into_a_deck_unattended() {
    let tmpl = |animate: Option<&str>, paper: bool| {
        let mut r = json!({ "match": "template", "template": "focal", "reduction": [], "properties": [] });
        if paper {
            r["paper_values"] = json!([{ "name": "gamma", "value": 2 }]);
        }
        if let Some(a) = animate {
            r["animate"] = a.into();
        }
        r
    };
    let go = |r: Value| async move { propose::propose(&FakeLlm::new(vec![r]), &ask("FL")).await.unwrap() };
    let tuning = "We set $\\gamma = 2$; the ablation varies $\\gamma$.";
    assert!(propose::fit_for_auto(&go(tmpl(Some("gamma"), true)).await, tuning).is_ok());
    assert!(propose::fit_for_auto(&go(tmpl(Some("gamma"), true)).await, "the ablation varies γ").is_ok(), "the Unicode letter counts");
    assert!(propose::fit_for_auto(&go(tmpl(Some("gamma"), true)).await, "we train for 30 epochs").unwrap_err().contains("沒有提到"), "the paper never talks about tuning it");
    assert!(propose::fit_for_auto(&go(tmpl(None, true)).await, tuning).unwrap_err().contains("調哪個參數"), "no tuned parameter named");
    assert!(propose::fit_for_auto(&go(tmpl(Some("gamma"), false)).await, tuning).unwrap_err().contains("論文的值"));
    assert!(propose::fit_for_auto(&go(new_style().remove(1).1).await, tuning).unwrap_err().contains("自訂"));
    // a failing property blocks it too
    let mut bad = tmpl(Some("gamma"), true);
    bad["properties"] = json!([{ "kind": "increasing", "var": "gamma", "text": "wrong on purpose" }]);
    assert!(propose::fit_for_auto(&go(bad).await, tuning).unwrap_err().contains("wrong on purpose"));
}

/// In a paper an equation figure is a PDF + .plot.json inserted by the same figure block:
/// registered label, \ref, the caption's maths left alone; lore-notation finds nothing.
#[test]
fn an_equation_figure_block_passes_the_notation_checks() {
    use lore_notation::{CheckOptions, Document, Entry, Kind, Notation, check_document};
    let block = crate::figure::figure_block("figures/eq-focal.pdf", "Focal loss for $\\gamma \\in [0, 5]$; dashed: the value we use.", "fig:eq-focal", false);
    assert!(block.contains("$\\gamma \\in [0, 5]$"));
    let tex = format!("\\documentclass{{article}}\n\\begin{{document}}\n\\section{{Method}}\nLarger $\\gamma$ flattens easy examples (Fig.~\\ref{{fig:eq-focal}}).\n\n{block}\n\\end{{document}}\n");
    let mut reg = Notation::default();
    let mut e = Entry::new("fig:eq-focal", Kind::Figure);
    e.label = Some("fig:eq-focal".into());
    e.file = Some("figures/eq-focal.pdf".into());
    reg.upsert(e);
    let report = check_document(Document::from_str("main.tex", &tex), &reg, &CheckOptions::default());
    let about: Vec<_> = report.issues.iter().filter(|i| ["label_unreferenced", "label_duplicate", "ref_undefined", "hardcoded_reference"].contains(&i.code.as_str())).collect();
    assert!(about.is_empty(), "{about:#?}");
}

#[test]
fn templates_are_notation_entries_with_colour_roles() {
    let n = template_notation();
    let focal = n.get("eq:focal").expect("the focal template is a registry equation");
    assert_eq!(focal.kind(), lore_notation::Kind::Equation);
    assert!(focal.latex.as_deref().unwrap().contains("\\gamma"));
    assert!(focal.uses.len() >= 3);
    // every symbol it uses is an entry with a colour role (an index, never a literal colour)
    for u in &focal.uses {
        let e = n.get(u).unwrap();
        assert!(e.color.is_some_and(|c| c < 8), "{u}");
    }
    // symbols are clean LaTeX (no `$`, no labels' Chinese notes)
    for e in n.entries.iter().filter(|e| e.id.starts_with("v:")) {
        let t = e.latex.as_deref().unwrap();
        assert!(!t.contains('$') && !t.chars().any(|c| ('\u{3000}'..='\u{9fff}').contains(&c)), "{t}");
    }
    let roles = color_roles(&[]);
    let gamma = roles[&lore_notation::math::canonical("\\gamma")];
    let p = roles[&lore_notation::math::canonical("p_t")];
    assert_eq!(p, 1, "the input on x");
    assert!(gamma >= 2 && gamma != p);
    // a document's own colour wins (the same symbol, the same colour in every app)
    let mut doc = lore_notation::Notation::default();
    let mut e = lore_notation::Entry::new("g", lore_notation::Kind::Symbol);
    e.latex = Some("\\gamma".into());
    e.color = Some(5);
    doc.upsert(e);
    assert_eq!(color_roles(&[&doc])[&lore_notation::math::canonical("\\gamma")], 5);
    // the field is additive: an entry without it reads and writes as before
    let old = r#"{"version":1,"entries":[{"id":"x","kind":"symbol","latex":"x"}]}"#;
    let n = lore_notation::Notation::from_json(old).unwrap();
    assert!(n.entries[0].color.is_none());
    assert!(!serde_json::to_string(&n).unwrap().contains("color"));
}

/// Decode every frame onto one canvas (sub-rectangles composited, the way a viewer shows them).
fn composited(bytes: &[u8]) -> (Vec<Vec<u8>>, Vec<u16>, Vec<(u16, u16)>) {
    let mut opts = ::gif::DecodeOptions::new();
    opts.set_color_output(::gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(std::io::Cursor::new(bytes)).unwrap();
    let (w, h) = (dec.width() as usize, dec.height() as usize);
    let mut canvas = vec![0u8; w * h * 4];
    let (mut out, mut delays, mut sizes) = (vec![], vec![], vec![]);
    while let Some(f) = dec.read_next_frame().unwrap() {
        for y in 0..f.height as usize {
            for x in 0..f.width as usize {
                let s = (y * f.width as usize + x) * 4;
                let d = ((y + f.top as usize) * w + x + f.left as usize) * 4;
                if f.buffer[s + 3] != 0 {
                    canvas[d..d + 4].copy_from_slice(&f.buffer[s..s + 4]);
                }
            }
        }
        delays.push(f.delay);
        sizes.push((f.width, f.height));
        out.push(canvas.clone());
    }
    (out, delays, sizes)
}

#[test]
fn gif_stores_only_what_changed_and_keeps_per_frame_delays() {
    let frames = synthetic_frames(12);
    let bytes = gif::encode_with(&frames, 80, None, &gif::Opts { delays: (0..12).map(|i| if i == 11 { 1500 } else { 60 }).collect(), dither: vec![] }).unwrap();
    let (shown, delays, sizes) = composited(&bytes);
    assert_eq!(shown.len(), 12);
    assert_eq!(delays[0], 6);
    assert_eq!(delays[11], 150, "the end is held");
    assert_eq!(sizes[0], (120, 80));
    assert!(sizes[1..].iter().all(|&(w, h)| (w as u32) * (h as u32) < 120 * 80), "later frames are sub-rectangles: {sizes:?}");
    // composited, each frame shows that frame (the unchanged area kept from before is right too)
    for (k, f) in frames.iter().enumerate() {
        let err: u64 = shown[k].chunks_exact(4).zip(f.px.chunks_exact(4)).map(|(a, b)| (0..3).map(|c| (a[c] as i32 - b[c] as i32).unsigned_abs() as u64).sum::<u64>()).sum();
        assert!(err as f64 / (120.0 * 80.0 * 3.0) < 2.0, "frame {k}: mean error {}", err as f64 / (120.0 * 80.0 * 3.0));
    }
}

#[test]
fn dither_only_inside_the_heatmap_rectangle() {
    // left half: a smooth 2D gradient (many more than 256 colours); right half: a black line on white
    let (w, h) = (128u32, 64u32);
    let mut px = vec![255u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if x < 64 {
                px[i] = (40 + x * 3) as u8;
                px[i + 1] = (30 + y * 3) as u8;
                px[i + 2] = (90 + (x + y)) as u8;
            } else if y == 32 {
                px[i..i + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
    }
    let fr = vec![gif::Rgba { w, h, px: px.clone() }];
    let plain = composited(&gif::encode(&fr, 80, None).unwrap()).0.remove(0);
    let dith = composited(&gif::encode_with(&fr, 80, None, &gif::Opts { delays: vec![], dither: vec![[0.0, 0.0, 0.5, 1.0]] }).unwrap()).0.remove(0);
    // 4×4 block means over the gradient are closer to the source with the dither (no flat bands)
    let block_err = |img: &[u8]| -> f64 {
        let mut e = 0.0;
        for by in 0..h / 4 {
            for bx in 0..16 {
                for c in 0..3 {
                    let (mut a, mut b) = (0.0, 0.0);
                    for y in by * 4..by * 4 + 4 {
                        for x in bx * 4..bx * 4 + 4 {
                            let i = ((y * w + x) * 4 + c) as usize;
                            a += img[i] as f64;
                            b += px[i] as f64;
                        }
                    }
                    e += ((a - b) / 16.0).abs();
                }
            }
        }
        e
    };
    assert!(block_err(&dith) < block_err(&plain), "dither {} vs plain {}", block_err(&dith), block_err(&plain));
    // outside the rectangle the line and the paper stay exact
    for x in 64..w {
        let i = ((32 * w + x) * 4) as usize;
        assert_eq!(&dith[i..i + 3], &[0, 0, 0]);
        let j = ((10 * w + x) * 4) as usize;
        assert_eq!(&dith[j..j + 3], &[255, 255, 255]);
    }
}
