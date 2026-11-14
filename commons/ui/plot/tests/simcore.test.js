// simcore.js under node: the shared term values (simtests.json, also checked by Rust `equation::sim`),
// and properties of real runs: forward KL / cross-entropy lands on the target's mean and variance,
// reverse KL picks one mode, label smoothing converges to the smoothed target, a weight of 0 changes
// nothing, k-means / EM never raise their objective, the GIF schedule is monotone and paced by change.
// Run: node commons/ui/plot/tests/simcore.test.js (cargo test -p lore-plot runs it when node is installed).
"use strict";
const path = require("path");
const assert = require("assert");
const L = require(path.join(__dirname, "..", "simcore.js"));
const cases = require(path.join(__dirname, "..", "simtests.json"));
const scenes = require(path.join(__dirname, "..", "equations.json")).scenes;
let n = 0;
const ok = (c, m) => { n++; assert.ok(c, m); };
const near = (a, b, tol, m) => ok(Math.abs(a - b) <= tol, `${m}: ${a} vs ${b}`);

// shared cases: term values at θ
for (const c of cases.values) {
  const run = L.create(c.sim);
  const got = run.values(c.theta);
  c.expect.forEach((e, i) => {
    if (e === null) ok(got[i] === null, `${c.name} term ${i}: ${got[i]} (expected no value)`);
    else near(got[i], e, Math.max(1e-9, Math.abs(e) * 1e-9), `${c.name} term ${i}`);
  });
}
// the scenes in the catalogue are valid and run
for (const s of scenes) {
  ok(L.problems(s.sim).length === 0, `${s.id}: ${L.problems(s.sim)}`);
  const run = L.create(s.sim).scripted().step(20);
  ok(run.hist.length === 21 && run.hist.every((h) => h.L.every((v) => v === null || Number.isFinite(v))), `${s.id} runs`);
}

const bimodal = { mix: [{ w: 0.5, mu: -2, sigma: 0.55 }, { w: 0.5, mu: 2, sigma: 0.55 }] };
const fit = (terms, steps = 900, model = { mu: 0.35, sigma: 0.5 }) => L.create({ scene: "fit1d", x: [-8, 8], target: bimodal, model, terms, opt: { lr: 0.05, steps } }).step(steps);
// forward KL (and cross-entropy): μ = E_p[x], σ² = Var_p
{
  const T = L.mix1(bimodal);
  for (const kind of ["kl_pq", "ce"]) {
    const r = fit([{ kind, w: 1 }]);
    near(r.theta[0], T.mean, 0.02, `${kind} μ`);
    near(Math.exp(2 * r.theta[1]), T.var, 0.05, `${kind} σ²`);
  }
  // reverse KL: one mode, narrow
  const r = fit([{ kind: "kl_qp", w: 1 }]);
  near(Math.abs(r.theta[0]), 2, 0.1, "reverse KL picks a mode");
  ok(Math.exp(r.theta[1]) < 0.8, "reverse KL is narrow");
}
// a weight of 0 (or a term switched off) has no effect on the run
{
  const a = fit([{ kind: "kl_pq", w: 1 }], 200);
  const b = fit([{ kind: "kl_pq", w: 1 }, { kind: "anchor", w: 0, mu0: -3 }], 200);
  const c = fit([{ kind: "kl_pq", w: 1 }, { kind: "kl_qp", w: 3, on: false }], 200);
  near(a.theta[0], b.theta[0], 1e-12, "w = 0"); near(a.theta[1], b.theta[1], 1e-12, "w = 0 σ");
  near(a.theta[0], c.theta[0], 1e-12, "off");
  ok(b.pulls()[1].every((x) => x === 0), "w = 0 ⇒ no pull");
}
// the pull is −w ∂L/∂θ: the anchor pulls μ toward μ0
{
  const r = L.create({ scene: "fit1d", target: bimodal, model: { mu: 1, sigma: 1 }, terms: [{ kind: "anchor", w: 2, mu0: -3 }] });
  near(r.pulls()[0][0], -2 * 2 * (1 - -3), 1e-4, "anchor pull");
}
// changing a weight continues from the current θ (no reset)
{
  const r = fit([{ kind: "kl_pq", w: 1 }, { kind: "kl_qp", w: 0 }], 100);
  const th = r.theta.slice();
  r.setWeight(1, 1);
  ok(r.theta[0] === th[0] && r.marks.includes(100), "weight change keeps θ and is marked");
  r.step(1);
  ok(r.stepNo === 101 && r.hist.length === 102, "continues");
}
// label smoothing converges to the smoothed target; one-hot keeps sharpening
{
  const sm = L.create({ scene: "classes", classes: { K: 6, y: 3 }, model: { z: [0.2, -0.3, 0, 0.9, -0.5, 0.3] }, terms: [{ kind: "smooth", w: 1, eps: 0.1 }], opt: { method: "gd", lr: 0.5 } }).step(3000);
  const q = sm.view().q, t = sm.targetBars();
  q.forEach((v, k) => near(v, t[k], 2e-3, `smoothed q${k}`));
  near(t[2], 0.9 + 0.1 / 6, 1e-12, "target bar");
  const oh = L.create({ scene: "classes", classes: { K: 6, y: 3 }, terms: [{ kind: "ce", w: 1 }], opt: { method: "gd", lr: 0.5 } }).step(3000);
  ok(oh.view().q[2] > 0.99, "one-hot → 1");
}
// a user term in the expression language: (mu - 1)^2 pulls μ to 1
{
  const r = L.create({ scene: "fit1d", target: bimodal, model: { mu: -1, sigma: 1 }, terms: [{ kind: "expr", w: 1, expr: "(mu - 1)^2" }], opt: { lr: 0.05 } }).step(600);
  near(r.theta[0], 1, 0.01, "expr term");
  ok(L.problems({ scene: "fit1d", terms: [{ kind: "expr", w: 1, expr: "constructor(1)" }] }).length === 1, "unknown names refused");
}
// k-means and EM never raise their objective
for (const method of ["kmeans", "em"]) {
  const r = L.create({ scene: "cluster", cluster: { method, k: 3 }, target: { mix: [{ w: 1, mu: [-2, -1], cov: [0.5, 0, 0.3] }, { w: 1, mu: [2, 1], cov: [0.3, 0.1, 0.4] }, { w: 1, mu: [1.5, -1.8], cov: [0.4, 0, 0.3] }] }, samples: { n: 300, seed: 5 }, terms: [{ kind: method, w: 1 }] }).step(30);
  const v = r.hist.map((h) => h.L[0]);
  ok(v.every((x, i) => i === 0 || x <= v[i - 1] + 1e-9), `${method} monotone: ${v.slice(0, 5)}`);
}
// 2D fit by NLL lowers the NLL
{
  const s = scenes.find((x) => x.id === "sim-mixture").sim;
  const r = L.create(s).step(60);
  ok(r.hist.at(-1).L[0] < r.hist[0].L[0] - 0.2, "2D NLL falls");
}
// GIF pacing: monotone, ends at the last step, more frames where the picture changes
{
  const diffs = [0, ...Array.from({ length: 100 }, (_, k) => (k < 20 ? 10 : 0.1))];
  const s = L.schedule(diffs, 40);
  ok(s[0] === 0 && s.at(-1) === 100 && s.every((v, i) => i === 0 || v >= s[i - 1]), "monotone, both ends");
  ok(s.filter((k) => k <= 20).length > 20, `busy start gets most frames (${s.filter((k) => k <= 20).length})`);
  ok(s.filter((k) => k > 50).length >= 4, "a settled tail still gets time");
  const flat = L.schedule([0, 0, 0, 0, 0], 5);
  ok(flat.join() === "0,1,2,3,4", "no change: plain time");
  const d = L.delays(10, 80, 1500);
  ok(d[9] === 1500 && d[0] === 80, "hold at the end");
}
// a plan step with reset: the second objective starts from the same θ (forward vs reverse KL in one run)
{
  const s = require(path.join(__dirname, "..", "equations.json")).scenes.find((x) => x.id === "sim-modes").sim;
  const r = L.create(s).scripted().step(s.opt.steps);
  const T = L.mix1(s.target);
  near(Math.abs(r.theta[0]), 2, 0.15, "after the reset, reverse KL settles on one mode");
  ok(r.marks.includes(160), "the switch is marked");
  ok(Math.abs(T.mean) < 0.5, "target mean");
}
// the audience follows the presenter: every row arrives (several per frame at 4×), weights ride along
{
  const spec = { scene: "fit1d", target: bimodal, model: { mu: 0.35, sigma: 0.5 }, terms: [{ kind: "kl_pq", w: 1 }, { kind: "kl_qp", w: 0 }] };
  const a = L.create(spec), b = L.create(spec);
  let sent = 0;
  const tick = () => { const m = { step: a.stepNo, theta: a.theta.slice(), from: sent, rows: a.hist.slice(sent), marks: a.marks.slice(), terms: JSON.parse(JSON.stringify(a.sim.terms)) }; sent = a.hist.length; b.follow(m); };
  for (let f = 0; f < 30; f++) { a.step(4); if (f === 10) { a.setWeight(1, 0.7, false); a.setWeight(1, 1, false); a.mark(); } tick(); }
  ok(b.hist.length === a.hist.length && b.stepNo === a.stepNo && b.theta.join() === a.theta.join(), "audience has every row");
  ok(b.marks.join() === a.marks.join() && a.marks.length === 1, `one mark per drag: ${a.marks}`);
  ok(b.sim.terms[1].w === 1, "weights ride along");
  // a removed term leaves the plan consistent
  const r = L.create({ ...spec, plan: [{ at: 5, w: [0.5, 1] }] });
  r.removeTerm(0);
  ok(r.sim.plan[0].w.join() === "1" && L.problems(r.sim).length === 0, "plan follows the terms");
}
console.log(`simcore.test.js ok (${n} checks)`);
