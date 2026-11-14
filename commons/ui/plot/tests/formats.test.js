// Changing a panel's chart type (formats.js LorePlot.changeType): what the author set is kept, and the
// axis labels follow the categories when they move to the y axis (dumbbell). A bar chart of scores per
// item turned into a dumbbell used to label the value axis with the category name and keep four series, two drawn.
// Run: node commons/ui/plot/tests/formats.test.js (cargo test -p lore-plot runs it when node is installed).
"use strict";
const path = require("path");
const fs = require("fs");
const assert = require("assert");
const clone = (o) => JSON.parse(JSON.stringify(o));
global.window = { LorePlot: { _: { isNum: (v) => typeof v === "number", num: Number, col: () => [], ncol: () => [], clone, trapz: () => 0, rocFrom: () => null }, dataSync: () => null, data: async () => ({}) } };
new Function(fs.readFileSync(path.join(__dirname, "..", "formats.js"), "utf8"))();
const LP = window.LorePlot;
// the starter panels charts.js makes (only what changeType reads)
LP.starterPanel = (type, t) => ({ type, data: "t1", x: t.columns[0], series: t.columns.slice(1).map((y) => ({ y, label: y })), legend: {}, notes: [], opts: {}, xaxis: { label: type === "dumbbell" ? "" : t.columns[0] }, yaxis: { label: "" } });
const tbl = { columns: ["Item", "Site A", "Site B", "Site C", "Diff (p)"], rows: [] };
const bar = { lore_plot: 1, data: { t1: tbl }, panels: [{ ...LP.starterPanel("bar", tbl), yaxis: { label: "Score" } }] };

const d = LP.changeType(bar, 0, "dumbbell").panels[0];
assert.strictEqual(d.type, "dumbbell");
assert.strictEqual(d.x, "Item");
assert.deepStrictEqual(d.yaxis, { label: "Item" }, "the categories' name goes down the y axis");
assert.deepStrictEqual(d.xaxis, { label: "Score" }, "the value label goes across");
assert.deepStrictEqual(d.series.map((s) => s.y), ["Site A", "Site B"], "a dumbbell keeps two series");

// and back: the labels return to their sides
const b = LP.changeType({ ...bar, panels: [d] }, 0, "bar").panels[0];
assert.deepStrictEqual([b.xaxis, b.yaxis], [{ label: "Item" }, { label: "Score" }]);
// a horizontal bar already has its categories on y: nothing swaps
const hb = { ...bar, panels: [{ ...bar.panels[0], opts: { horizontal: true }, xaxis: { label: "Score" }, yaxis: { label: "Item" } }] };
const d2 = LP.changeType(hb, 0, "dumbbell").panels[0];
assert.deepStrictEqual([d2.xaxis, d2.yaxis], [{ label: "Score" }, { label: "Item" }]);
// bar ↔ line: same sides
const l = LP.changeType(bar, 0, "line").panels[0];
assert.deepStrictEqual([l.xaxis, l.yaxis], [{ label: "Item" }, { label: "Score" }]);
console.log("formats: changeType ok");
