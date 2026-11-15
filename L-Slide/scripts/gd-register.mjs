// Registers gd-loader.mjs (Node --import entry for import-gd-recipes.mjs).
import { register } from "node:module";
register("./gd-loader.mjs", import.meta.url);
