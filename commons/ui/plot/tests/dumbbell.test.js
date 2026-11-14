// The dumbbell chart (charts.js): `opts.sort` orders the categories by the second value of each pair,
// top to bottom (the assistant asked for it and nothing moved).
// Run: node commons/ui/plot/tests/dumbbell.test.js (cargo test -p lore-plot runs it when node is installed).
"use strict";
const path = require("path");
const fs = require("fs");
const assert = require("assert");
global.window = {};
global.document = { createElement: () => ({ style: {} }), fonts: { ready: Promise.resolve() }, head: { append() {} }, querySelector: () => null };
for (const f of ["../../mathsvg.js", "../plot.js", "../charts.js"]) new Function(fs.readFileSync(path.join(__dirname, f), "utf8"))();
const LP = window.LorePlot;
const tbl = { columns: ["Item", "Before", "After"], rows: [["Alpha", 0.762, 0.801], ["Beta", 0.727, 0.519], ["Gamma", 0.832, 0.794], ["Delta", 0.714, null]] };
const plan = (opts) => LP.CHARTS.dumbbell.prepare({ type: "dumbbell", x: "Item", series: [{ y: "Before" }, { y: "After" }], opts }, tbl, LP._.ROOT_STYLE);
assert.deepStrictEqual(plan({}).y.cats, ["Alpha", "Beta", "Gamma", "Delta"], "unsorted: the table's order");
assert.deepStrictEqual(plan({ sort: "asc" }).y.cats, ["Delta", "Beta", "Gamma", "Alpha"], "a missing value first when ascending");
assert.deepStrictEqual(plan({ sort: "desc" }).y.cats, ["Alpha", "Gamma", "Beta", "Delta"]);
// the points move with their category
const p = plan({ sort: "asc" });
assert.deepStrictEqual(p.points.filter(([, k]) => k === 1).map(([v]) => v).sort(), [0.519, 0.727]);
console.log("dumbbell: sort ok");
