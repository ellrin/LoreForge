// node web/tests/pagehand.test.js — the slides handed to L-Echo (`pageSlides`) and the slide shown
// after its ops were applied (`pagedSlide`), both in arrange.js, without a browser.
"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const web = path.join(__dirname, "..");
const ctx = vm.createContext({ console, Math, JSON, Map, Set, localStorage: { getItem: () => null } });
vm.runInContext(fs.readFileSync(path.join(web, "arrange.js"), "utf8") + "\n;globalThis.pageSlides = pageSlides; globalThis.pagedSlide = pagedSlide; globalThis.pickSlides = pickSlides;", ctx);
const paraText = (p) => p.runs.map((r) => r.text ?? "").join("");
const box = (id, ...paras) => ({ id, type: "text", frame: { x: 0, y: 0, w: 100, h: 100 }, text: { paras: paras.map((t) => ({ runs: [{ kind: "text", text: t }] })) } });
const plain = (v) => JSON.parse(JSON.stringify(v));

const slides = [
  { uid: "s1", objects: [{ ...box(1, "Cover"), ph: "ctrTitle" }] },
  { uid: "s2", objects: [box(2, "body only")] },
  { uid: "s3", objects: [{ ...box(1, " "), slot: "title" }, { type: "group", children: [{ ...box(4, "Method  overview"), slot: "title-2" }] }] },
  { uid: "s4", objects: [{ ...box(1, "Hidden"), ph: "title", hidden: true }, { ...box(2, "Results"), ph: "title" }] },
];

// id = uid, n = page number, title by the test selectionContext uses; deck order, whatever order was picked
assert.deepEqual(plain(ctx.pageSlides(slides, ["s3", "s1"], paraText)), [{ id: "s1", n: 1, title: "Cover" }, { id: "s3", n: 3, title: "Method overview" }]);
// no title object, and a hidden title does not count
assert.deepEqual(plain(ctx.pageSlides(slides, ["s2"], paraText)), [{ id: "s2", n: 2, title: "" }]);
assert.deepEqual(plain(ctx.pageSlides(slides, ["s4"], paraText)), [{ id: "s4", n: 4, title: "Results" }]);
// a slide that is gone is left out
assert.deepEqual(plain(ctx.pageSlides(slides, ["s9"], paraText)), []);

// the first slide an op names that is in the deck: changed, or inserted
assert.equal(ctx.pagedSlide([{ op: "set_obj", slide: "s3", id: 4, path: "/text", value: {} }, { op: "insert_slide", index: 3, slide: { uid: "s7" } }], [...slides, { uid: "s7" }]), "s3");
assert.equal(ctx.pagedSlide([{ op: "insert_slide", index: 1, slide: { uid: "s7", objects: [] } }], [...slides, { uid: "s7" }]), "s7");
// a deleted slide is not shown; nothing named = stay
assert.equal(ctx.pagedSlide([{ op: "delete_slide", slide: "s9" }, { op: "set_slide", slide: "s2", path: "/notes", value: "" }], slides), "s2");
assert.equal(ctx.pagedSlide([{ op: "delete_slide", slide: "s9" }], slides), null);

// the sorter's extra selection: ⌘-click toggles, the open slide is never an extra, Shift-click = the open slide through the clicked one
{
  const order = ["s1", "s2", "s3", "s4"], pick = (...a) => plain(ctx.pickSlides(order, ...a));
  assert.deepEqual(pick("s2", [], "s3", false), ["s3"]);
  assert.deepEqual(pick("s2", ["s3"], "s3", false), []);
  assert.deepEqual(pick("s2", ["s3"], "s2", false), ["s3"]);
  assert.deepEqual(pick("s2", ["s4"], "s3", true), ["s3"]);
  assert.deepEqual(pick("s3", [], "s1", true), ["s1", "s2"]);
}

console.log("pagehand.test.js ok");
