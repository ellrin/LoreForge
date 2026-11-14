// Variable colour roles (equation.js): the same spec gives every symbol the same INDEX whether it is drawn
// for the app theme (interactive view) or for a deck / paper palette (exports) — only the palette differs.
// A role set on the variable (from the lore-notation registry) wins over the default by position.
// Run: node commons/ui/plot/tests/eqcolors.test.js (cargo test -p lore-plot runs it when node is installed).
"use strict";
const path = require("path");
const fs = require("fs");
const assert = require("assert");
const E = require(path.join(__dirname, "..", "eqcore.js"));
// equation.js needs only these of plot.js at load time
global.window = { LoreEq: E, LorePlot: { _: { r2: (v) => Math.round(v * 100) / 100, isNum: (v) => typeof v === "number" && Number.isFinite(v), clone: (o) => JSON.parse(JSON.stringify(o)), markerPath: () => "", dashArray: () => "" }, CHARTS: {}, base: "" } };
new Function(fs.readFileSync(path.join(__dirname, "..", "equation.js"), "utf8"))();
const LP = window.LorePlot;
const cat = require(path.join(__dirname, "..", "equations.json"));
const theme = ["#2de2e6", "#ffd23f", "#ff3fb4", "#3ee8b5", "#ff9f43", "#ff6b86"];
const deck = ["#0072B2", "#E69F00", "#009E73", "#D55E00", "#CC79A7", "#56B4E9"];
const index = (colors, pal) => Object.fromEntries(Object.entries(colors).map(([k, c]) => [k, pal.indexOf(c)]));
let n = 0;
for (const t of cat.templates) {
  const eq = LP.eq.fromTemplate(t);
  const a = index(LP.eqColors.colorsFor(eq, theme), theme);
  const b = index(LP.eqColors.colorsFor(eq, deck), deck);
  assert.deepStrictEqual(a, b, t.id);
  assert.ok(Object.values(a).every((i) => i >= 0), t.id);
  // the function itself, the input and each parameter: distinct roles (up to the palette's size)
  const ps = (eq.vars ?? []).filter((v) => v.role === "param").map((v) => a[v.name]);
  if (eq.x) assert.ok(!ps.includes(a[eq.x.name]) && a[eq.x.name] !== a.__out, t.id);
  n++;
}
// a role from the registry (var.color) is the same in both renders and wins over the position
const focal = LP.eq.fromTemplate(cat.templates.find((t) => t.id === "focal"));
focal.vars.find((v) => v.name === "gamma").color = 4;
assert.strictEqual(LP.eqColors.colorsFor(focal, theme).gamma, theme[4]);
assert.strictEqual(LP.eqColors.colorsFor(focal, deck).gamma, deck[4]);
// LaTeX colour-coding uses those colours for the symbols' own spellings
const syms = LP.eqColors.symbolsFor(focal, LP.eqColors.colorsFor(focal, deck));
assert.match(E.colorTex("-\\alpha_{t}(1-p_{t})^{\\gamma}\\log p_{t}", syms), new RegExp(`\\\\textcolor\\{${deck[4]}\\}\\{\\\\gamma\\}`));
// registry roles that collide (two parameters both "role 2") end up distinct within the equation
const custom = { expr: "gamma * x + tau", x: { name: "x", label: "$x$", min: 0, max: 1 }, vars: [{ name: "gamma", label: "$\\gamma$", role: "param", value: 1, min: 0, max: 2, step: 0.1, color: 2 }, { name: "tau", label: "$\\tau$", role: "param", value: 1, min: 0.1, max: 2, step: 0.1, color: 2 }] };
LP.eq.distinctRoles(custom);
const roles = ["__out", "x", "gamma", "tau"].map((k) => LP.eqColors.roleOf(custom, k));
assert.strictEqual(new Set(roles).size, 4, JSON.stringify(roles));
assert.strictEqual(LP.eqColors.roleOf(custom, "gamma"), 2, "the first keeps its registry role");
// the GIF's frame sequence: ping-pong, the paper's value held `hold` frames each crossing, none at an end
const g = { min: 0, max: 5, step: 0.1, paper: 2 };
const seq = LP.eq.sweepWithHold(g, 24, 3);
assert.strictEqual(seq.length, 24 + 2 * 3);
assert.strictEqual(seq.filter((v) => v === 2).length, 2 * 3);
const atEnd = LP.eq.sweepWithHold({ ...g, paper: 0 }, 24, 3);
assert.strictEqual(atEnd.length, 24, "a paper value at the end of the range is not held");
console.log(`eqcolors: ${n} templates, same index in theme and deck palettes`);
