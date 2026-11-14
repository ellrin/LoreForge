// eqcore.js under node: the shared cases (eqtests.json) that lore-plot's Rust evaluator also passes,
// unsafe input refused, Monte Carlo identical to Rust's, the LaTeX of the parsed tree, the eq helpers.
// Run: node commons/ui/plot/tests/eqcore.test.js (cargo test -p lore-plot runs it when node is installed).
"use strict";
const path = require("path");
const assert = require("assert");
const E = require(path.join(__dirname, "..", "eqcore.js"));
const cases = require(path.join(__dirname, "..", "eqtests.json"));
const cat = require(path.join(__dirname, "..", "equations.json"));
let n = 0;
const ok = (c, m) => { n++; assert.ok(c, m); };

for (const c of cases.values) {
  const vars = c.vars ?? {};
  const ast = E.parse(c.expr, Object.keys(vars));
  let got = E.evaluate(ast, vars);
  if (Array.isArray(got) && got.length === 1) got = got[0];
  if (c.expect === null) ok(Number.isNaN(got), `${c.expr}: ${got} (expected no value)`);
  else {
    const tol = Math.max(c.tol ?? 1e-12, Math.abs(c.expect) * 1e-12);
    ok(Math.abs(got - c.expect) <= tol, `${c.expr}: ${got} ≠ ${c.expect}`);
  }
}
for (const c of cases.reject) {
  let threw = false;
  try { E.parse(c.expr, c.vars ?? []); } catch { threw = true; }
  ok(threw, `accepted: ${c.expr}`);
}
for (const c of cases.budget) {
  let threw = false;
  try { E.evaluate(E.parse(c.expr, []), {}); } catch { threw = true; }
  ok(threw, `no limit on: ${c.expr}`);
}
for (const deep of ["(".repeat(300) + "1" + ")".repeat(300), "-".repeat(400) + "1", "[".repeat(300) + "1" + "]".repeat(300)]) {
  assert.throws(() => E.parse(deep, []), /巢狀太深/);
  n++;
}
assert.throws(() => E.parse("(".repeat(5000) + "1" + ")".repeat(5000), []), /太長/);
// names never reach object properties
for (const bad of ["constructor", "__proto__", "prototype", "valueOf"]) assert.throws(() => E.evaluate({ t: "name", n: bad }, {}), /不認得/);

// Monte Carlo: the same draws as Rust (cases.mc: values the Rust test asserts too)
for (const c of cases.mc ?? []) {
  const got = E.evaluate(E.parse(c.expr, Object.keys(c.vars ?? {})), c.vars ?? {});
  ok(Math.abs(got - c.expect) <= 1e-12 * Math.max(1, Math.abs(c.expect)), `mc ${c.expr}: ${got} ≠ ${c.expect}`);
}

// LaTeX of the parsed tree
const tex = (s, names, labels) => E.toLatex(E.parse(s, names), labels);
assert.strictEqual(tex("-alpha * (1 - p)^gamma * log(p)", ["alpha", "p", "gamma"], { p: "$p_t$" }), "-\\alpha\\,{\\left(1 - p_t\\right)}^{\\gamma}\\,\\log\\left(p_t\\right)");
assert.strictEqual(tex("sum(j, 1, N, exp(s / tau))", ["N", "s", "tau"]), "\\sum_{j=1}^{N} e^{\\frac{s}{\\tau}}");
assert.match(tex("E(n, normal(0, sigma), softplus(x + n))", ["sigma", "x"]), /\\mathbb\{E\}_\{n \\sim \\mathcal\{N\}/);
assert.match(tex("piecewise(x < 0, 0, x)", ["x"]), /\\begin\{cases\}/);
assert.strictEqual(tex("(a + b) * c", ["a", "b", "c"]), "\\left(a + b\\right)\\,c");
assert.strictEqual(tex("a - (b - c)", ["a", "b", "c"]), "a - \\left(b - c\\right)");
n += 6;
// colour-coding: the paper's spellings (p_{t} vs p_t), longest first, no partial commands
assert.strictEqual(E.colorTex("\\alpha_{t}(1-p_{t})^{\\gamma}\\gammax", [{ tex: "p_t", color: "#0af" }, { tex: "\\gamma", color: "#f0a" }]), "\\alpha_{t}(1-\\textcolor{#0af}{p_{t}})^{\\textcolor{#f0a}{\\gamma}}\\gammax");
assert.match(tex("gamma * p", ["gamma", "p"]), /gamma/);
assert.strictEqual(E.toLatex(E.parse("gamma * p", ["gamma", "p"]), {}, { gamma: "#f0a" }), "\\textcolor{#f0a}{\\gamma}\\,p");
n += 3;

// eq helpers on every template: same hand points as the Rust test, properties are the server's
const T = (id) => {
  const t = cat.templates.find((x) => x.id === id);
  return { expr: t.expr, x: t.x, vars: t.vars, output: t.output };
};
const at = (id, over) => { const eq = T(id); return E.eq.value(eq, E.eq.compile(eq), over); };
const close = (a, b, tol = 1e-12) => ok(Math.abs(a - b) <= tol, `${a} vs ${b}`);
close(at("focal", { p: 0.5 }), 0.04332169878499658);
close(at("dice", { p: 0.5 }), 1 - 21 / 39);
close(at("infonce", { spos: 0.5, tau: 0.1 }), 0.052500962056356286);
close(at("smooth_ce", { p: 0.9 }), 0.5008609395783458);
close(at("gelu", { x: 1 }), 0.8413447460685429, 1e-6);
for (const t of cat.templates) {
  const eq = T(t.id);
  const ast = E.eq.compile(eq);
  if (eq.output === "bars") { const v = E.evaluate(ast, E.eq.env(eq)); ok(Array.isArray(v) && Math.abs(v.reduce((a, b) => a + b) - 1) < 1e-12, t.id); continue; }
  const ys = E.eq.curve(eq, ast, E.eq.xs(eq));
  ok(ys.filter((y) => y != null).length >= 190, `${t.id}: values`);
}
// derivatives: focal ∂/∂γ analytic
{
  const eq = T("focal"), ast = E.eq.compile(eq);
  const p = 0.4, g = 2, a = 0.25;
  close(E.eq.partial(eq, ast, "gamma", { p }), -a * (1 - p) ** g * Math.log(1 - p) * Math.log(p), 1e-6);
  const s = E.eq.sensitivity(eq, ast, 0.9);
  ok(s.some((x) => Math.abs(Math.abs(x.norm) - 1) < 1e-12) && s.find((x) => x.name === "gamma").d < 0, "sensitivity");
}
// ping-pong frames = Rust
{
  const v = T("focal").vars[0];
  const f = E.eq.sweepFrames(v, 24);
  ok(f.length === 24 && f[0] === 0 && f.filter((x) => x === 5).length === 1, "sweep");
}
console.log(`eqcore: ${n} checks passed`);
