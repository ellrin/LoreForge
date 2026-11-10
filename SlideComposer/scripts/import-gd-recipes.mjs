// Build web/recipes.json (版型 catalog) from GraphicDesigner's recipes, using
// GraphicDesigner's own composition generators and recipe resolver, so block
// positions and guide lines are exactly what that app computes.
//
//   node --import ./scripts/gd-register.mjs scripts/import-gd-recipes.mjs <path to GraphicDesigner> [out.json]
//
// Only 16:9 recipes and recipes for any canvas are taken. Coordinates are
// fractions of the slide (0..1). GraphicDesigner: MIT, © Ellrin.
import { readFileSync, readdirSync, writeFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const gd = resolve(process.argv[2] ?? "");
const out = process.argv[3] ?? new URL("../web/recipes.json", import.meta.url).pathname;
const src = join(gd, "source/src");
if (!existsSync(join(src, "core/recipes.ts"))) { console.error("usage: import-gd-recipes.mjs <GraphicDesigner folder>"); process.exit(2); }
const imp = (p) => import(pathToFileURL(join(src, p)).href);

// registries without Vite's import.meta.glob
async function registry(dir) {
  const { buildRegistry } = await imp("core/registry.ts");
  const gens = {}, params = {}, metas = {};
  for (const id of readdirSync(join(src, dir), { withFileTypes: true }).filter((d) => d.isDirectory()).map((d) => d.name)) {
    const base = `./${id}/`;
    gens[`${base}generate.ts`] = await imp(`${dir}/${id}/generate.ts`);
    const p = join(src, dir, id, "params.json");
    params[`${base}params.json`] = existsSync(p) ? JSON.parse(readFileSync(p, "utf8")) : {};
    metas[`${base}meta.json`] = JSON.parse(readFileSync(join(src, dir, id, "meta.json"), "utf8"));
  }
  return buildRegistry(gens, params, metas);
}
const comps = await registry("layers/1-composition/templates");
const guides = await registry("layers/2-guides/templates");
const { resolveRecipe, cutOf } = await imp("core/recipes.ts");

const W = 1600, H = 900;
const n = (v) => Math.round(v * 10000) / 10000;
const P = (p) => ({ x: n(p.x / W), y: n(p.y / H) });
function prim(p, visual) {
  const s = { weight: p.weight ?? "main", dashed: !!p.dashed, visual: !!visual, arrow: !!p.arrow };
  if (p.kind === "line") return { kind: "line", a: P(p.a), b: P(p.b), ...s };
  if (p.kind === "polyline") return { kind: "polyline", points: p.points.map(P), closed: !!p.closed, ...s };
  if (p.kind === "circle") return { kind: "circle", c: P(p.c), rx: n(p.r / W), ry: n(p.r / H), ...s };
  return null; // labels are not drawn on slides
}

const recipes = [];
for (const group of readdirSync(join(src, "recipes"))) {
  for (const f of readdirSync(join(src, "recipes", group)).filter((x) => x.endsWith(".json"))) {
    const r = { ...JSON.parse(readFileSync(join(src, "recipes", group, f), "utf8")), group, id: `${group}/${f.replace(/\.json$/, "")}` };
    // 16:9 and any-canvas recipes; golden-ratio ones (spiral, φ grid) for every canvas,
    // recomputed on 16:9 — they are written relative to their composition
    const golden = group === "golden-spiral" || group === "phi-grid";
    if (r.canvas && r.canvas !== "16x9" && !golden) continue;
    if (r.portrait) continue;
    const res = resolveRecipe(r, { w: W, h: H }, comps, guides);
    const nc = res.compositions.length;
    recipes.push({
      id: r.id, group, name: r.name, description: r.description ?? "", order: r.order ?? 99,
      cut: cutOf(r, res.blocks),
      composition: r.compositions.map((c) => comps.find((t) => t.id === c.template)?.meta.name ?? c.template),
      blocks: res.blocks.map((b) => ({
        name: b.name, role: b.role, shape: b.shape, filled: b.filled, panel: b.panel ?? null,
        rect: { x: n(b.rect.x / W), y: n(b.rect.y / H), w: n(b.rect.w / W), h: n(b.rect.h / H) },
        points: b.points ? b.points.map(P) : null,
      })),
      guides: res.outputs.flatMap((o, i) => o.primitives.map((p) => prim(p, i >= nc)).filter(Boolean)),
      anchors: res.outputs.slice(0, nc).flatMap((o) => o.anchors.map((a) => ({ ...P(a), label: a.label ?? "" }))),
    });
  }
}
recipes.sort((a, b) => a.group.localeCompare(b.group) || a.order - b.order);

// every composition on the whole slide with its default parameters: guide lines for
// layouts that only name a composition (the author's own arrangements)
const { computeInstance } = await imp("core/instances.ts");
const { IDENTITY } = await imp("core/transform.ts");
const compositions = {};
for (const t of comps) {
  const inst = { uid: t.id, templateId: t.id, params: structuredClone(t.defaults), orientation: { ...IDENTITY }, visible: true, frame: { kind: "canvas" } };
  const o = computeInstance(t, inst, { x: 0, y: 0, w: W, h: H }, true);
  compositions[t.id] = { name: t.meta.name, guides: o.primitives.map((p) => prim(p, false)).filter(Boolean), anchors: o.anchors.map((a) => ({ ...P(a), label: a.label ?? "" })) };
}
writeFileSync(out, JSON.stringify({ source: "GraphicDesigner (MIT, © Ellrin)", compositions, recipes }, null, 0) + "\n");
console.log(`${recipes.length} recipes → ${out}`);
