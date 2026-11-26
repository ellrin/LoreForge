// node web/tests/sites.test.js — the editor's connection sites (`sites(o)` in app.js) against the numbers
// lore-canvas produces (commons/crates/lore-canvas/tests/data/sites.json, written by the Rust test
// `write_sites_vectors`). Every shape has sixteen, in the same order, within 1 EMU.
"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const src = fs.readFileSync(path.join(__dirname, "..", "app.js"), "utf8");
const between = (a, b) => src.slice(src.indexOf(a) + a.length, src.indexOf(b));
const rot = src.slice(src.indexOf("function rotPt"), src.indexOf("function bbox"));
const code = [rot, between("// sites:path-begin", "// sites:path-end"), between("// sites:begin", "// sites:end"), "globalThis.sites = sites;"].join("\n");
const ctx = vm.createContext({ Math, console });
vm.runInContext(code, ctx);

const file = path.join(__dirname, "..", "..", "..", "commons", "crates", "lore-canvas", "tests", "data", "sites.json");
const cases = JSON.parse(fs.readFileSync(file, "utf8"));
assert.ok(cases.length > 100, "vector present");
for (const c of cases) {
  const o = { id: 1, type: "shape", frame: c.frame, geom: { prst: c.prst, adj: c.adj } };
  const got = ctx.sites(o);
  const label = `${c.prst} ${JSON.stringify(c.frame)} ${JSON.stringify(c.adj)}`;
  assert.equal(got.length, c.sites.length, label);
  c.sites.forEach(([x, y], i) => {
    assert.ok(Math.abs(got[i][0] - x) <= 1 && Math.abs(got[i][1] - y) <= 1, `${label} site ${i}: ${got[i]} vs ${[x, y]}`);
  });
}

// an unknown preset keeps four, a text box sixteen, a custom path one per vertex
const f = { x: 0, y: 0, w: 400, h: 400 };
assert.equal(ctx.sites({ type: "shape", frame: f, geom: { prst: "wave" } }).length, 4);
assert.equal(ctx.sites({ type: "text", frame: f, geom: { prst: "rect" } }).length, 16);
assert.equal(ctx.sites({ type: "shape", frame: f, geom: { prst: "custom", path: "M0 0 L1 0 L0.5 1 Z" } }).length, 3);
// a group with a child named `port` (a flow block) connects at the port's rectangle, mapped from
// the group's child space: same numbers as lore-canvas's group_with_a_port_connects_at_the_port
const g = { type: "group", frame: { x: 1000, y: 2000, w: 400, h: 600 }, ch: { x: 0, y: 0, w: 200, h: 300 },
  children: [{ name: "flow:f/n/port", type: "shape", frame: { x: 0, y: 50, w: 200, h: 100 }, geom: { prst: "rect" } }] };
const gs = ctx.sites(g);
assert.deepEqual(Array.from(gs[0], Math.round), [1200, 2100], "top middle of the port");
assert.deepEqual(Array.from(gs[2], Math.round), [1200, 2300], "bottom middle of the port");
console.log(`sites.test.js: ${cases.length} shapes ok`);
