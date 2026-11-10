// Node loader for import-gd-recipes.mjs: GraphicDesigner's TypeScript imports
// modules without extensions and JSON without import attributes (Vite style).
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

export async function resolve(spec, ctx, next) {
  if ((spec.startsWith(".") || spec.startsWith("/")) && !/\.[a-z]+$/i.test(spec)) {
    for (const ext of [".ts", "/index.ts"]) {
      const url = new URL(spec + ext, ctx.parentURL);
      if (existsSync(fileURLToPath(url))) return next(url.href, ctx);
    }
  }
  return next(spec, ctx);
}


export async function load(url, ctx, next) {
  if (url.endsWith(".ts") && url.includes("/source/src/")) {
    const { readFile } = await import("node:fs/promises");
    const src = (await readFile(fileURLToPath(url), "utf8")).replace(/import\.meta\.glob(<[^()]*>)?\(/g, "((..._a) => ({}))(");
    return { format: "module-typescript", source: src, shortCircuit: true };
  }
  if (url.endsWith(".json")) {
    const { readFile } = await import("node:fs/promises");
    return { format: "module", source: `export default ${await readFile(fileURLToPath(url), "utf8")};`, shortCircuit: true };
  }
  return next(url, ctx);
}
