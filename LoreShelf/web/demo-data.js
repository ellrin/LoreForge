// LoreShelf fallback: a small synthetic catalogue answering the same routes as shelf-server,
// used only with ?demo or when /api/files is absent (screenshots and tests without a catalog).
// Generic names only; the 證件 items and the datasets are synthetic.
//
//   ShelfDemo.api(method, path, body) → Promise<json>
//     GET /api/files · /api/files/{id} · /api/neighbors/{id}?k=&layers=&cat= · /api/categories
//     GET /api/nearest?from=&cat= · /api/datasets/{id}/items?offset=&limit=
//     PUT /api/files/{id}/categories {categories} · POST /api/open|reveal/{id}
"use strict";
(function (root) {
  const N = 320;
  const DAY = 86400000, END = Date.UTC(2026, 10, 1);
  const TOPICS = [
    { folder: "論文", use: "paper", kinds: ["pdf", "pdf", "latex", "slides"], host: "arxiv.org", words: ["影像分割", "綜述", "深度學習", "資料集", "注意力", "評估"] },
    { folder: "課程", use: "homework", kinds: ["slides", "pdf", "document"], host: null, words: ["講義", "統計", "迴歸", "作業", "期末", "筆記"] },
    { folder: "旅行", use: "ticket", kinds: ["image", "image", "pdf", "spreadsheet"], host: null, words: ["行程", "機票", "地圖", "照片", "住宿", "預算"] },
    { folder: "帳單", use: "ticket", kinds: ["pdf", "image", "spreadsheet"], host: "mail.example.com", words: ["發票", "收據", "保險", "租約", "水電", "報帳"] },
    { folder: "程式", use: null, kinds: ["archive", "code", "project", "data"], host: "github.com", words: ["rust", "parser", "server", "工具", "範例", "備份"] },
    { folder: "影音", use: null, kinds: ["video", "audio", "image"], host: null, words: ["錄音", "會議", "演講", "剪輯", "片段", "訪談"] },
    { folder: "下載", use: null, kinds: ["installer", "archive", "ebook", "font"], host: "downloads.example.org", words: ["安裝檔", "字型", "電子書", "更新", "外掛", "手冊"] },
  ];
  // the same ids as lore_catalog::category (type ids, then use ids)
  const TYPES = [["pdf", "PDF"], ["document", "文件"], ["slides", "簡報"], ["spreadsheet", "試算表"], ["image", "圖片"], ["video", "影片"], ["archive", "壓縮檔"], ["code", "程式碼"], ["project", "專案"], ["dataset", "資料集"], ["package", "套件"]];
  const USES = [["paper", "論文"], ["id-doc", "證件"], ["homework", "作業"], ["ticket", "票券收據"]];
  const typeOf = (k) => ({ pdf: "pdf", document: "document", text: "document", ebook: "document", latex: "document", slides: "slides", spreadsheet: "spreadsheet", image: "image", video: "video", archive: "archive", code: "code", project: "project", dataset: "dataset", package: "package" })[k] ?? null;
  const EXT = { pdf: "pdf", latex: "tex", slides: "pptx", document: "docx", image: "jpg", spreadsheet: "xlsx", archive: "zip", code: "py", data: "json", video: "mp4", audio: "m4a", installer: "dmg", ebook: "epub", font: "otf", project: "", dataset: "" };
  const KL = { pdf: "PDF", latex: "LaTeX", slides: "簡報", document: "文件", image: "圖片", spreadsheet: "試算表", archive: "壓縮檔", code: "程式碼", data: "資料", video: "影片", audio: "音訊", installer: "安裝檔", ebook: "電子書", font: "字型", project: "專案", dataset: "資料集" };

  function rng(a) { return () => { a = (a + 0x6d2b79f5) >>> 0; let t = a; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }
  const iso = (ms) => new Date(ms).toISOString().slice(0, 19) + "Z";

  const r = rng(11);
  const items = [], extra = [];
  const push = (it, ex) => {
    const { use = [], ...rest } = it;
    items.push({ id: (0x10000000 + items.length * 7919).toString(16).padStart(16, "0"), seen_count: 1, ...rest, categories: [typeOf(it.kind), ...use].filter(Boolean), sensitive: use.includes("id-doc") });
    extra.push(ex);
  };
  for (let i = 0; i < N; i++) {
    const g = Math.floor(r() * TOPICS.length), T = TOPICS[g];
    const kind = T.kinds[Math.floor(r() * T.kinds.length)];
    const w1 = T.words[Math.floor(r() * T.words.length)], w2 = T.words[Math.floor(r() * T.words.length)];
    const t = END - Math.floor(r() * r() * 4 * 365) * DAY;
    const ym = iso(t).slice(0, 7);
    const stem = r() < 0.5 ? `${w1}_${w2}` : `${ym}_${w1}${r() < 0.4 ? "_v" + (1 + Math.floor(r() * 3)) : ""}`;
    const ext = EXT[kind];
    const host = T.host && r() < 0.7 ? T.host : null;
    const sub = r() < 0.5 ? w1 : "";
    push({ kind, name: ext ? `${stem}.${ext}` : `${w1}-${w2}`, ext, size: Math.floor(2e4 + r() * 3e7),
      times: { created: iso(t - DAY * 3), modified: iso(t), downloaded: host ? iso(t - DAY) : null, opened: r() < 0.4 ? iso(t + DAY * 20) : null, doc_created: null },
      download_host: host, use: T.use && r() < 0.8 ? [T.use] : [] },
      { folder: `~/文件/${T.folder}${sub ? "/" + sub : ""}`, words: [w1, w2], t, topic: g });
  }
  // synthetic 證件 (sensitive): pictured as a generic card, never a real document
  for (const [name, k] of [["證件_正面.jpg", "image"], ["證件_背面.jpg", "image"], ["護照_影本.pdf", "pdf"], ["居留證明_掃描.pdf", "pdf"]]) {
    const t = END - Math.floor(r() * 300) * DAY;
    push({ kind: k, name, ext: name.split(".").pop(), size: 4e5, times: { created: iso(t), modified: iso(t), downloaded: null, opened: null, doc_created: null }, download_host: null, use: ["id-doc"] },
      { folder: "~/文件/證件", words: ["證件", "掃描"], t, topic: 3 });
  }
  // synthetic datasets (one record each; their files are listed through /api/datasets/{id}/items)
  const DS = [
    { name: "影像資料集_A", count: 1240, ext: "png", folders: [["train", 980], ["val", 130], ["test", 130]], labels: ["labels.csv", "README.md"], words: ["資料集", "影像分割"] },
    { name: "錄音片段集", count: 612, ext: "wav", folders: [["raw", 400], ["clean", 212]], labels: ["transcript.tsv"], words: ["錄音", "片段"] },
  ];
  for (const d of DS) {
    const t = END - Math.floor(r() * 200) * DAY;
    push({ kind: "dataset", name: d.name, ext: "", size: d.count * 2.1e5, times: { created: iso(t), modified: iso(t), downloaded: null, opened: null, doc_created: null }, download_host: null, use: [] },
      { folder: `~/資料/${d.name}`, words: d.words, t, topic: d.ext === "png" ? 0 : 5, ds: d });
  }
  const byId = new Map(items.map((it, i) => [it.id, i]));

  const W = { name: 0.4, time: 0.25, kind: 0.15, host: 0.1, folder: 0.1 };
  function sim(a, b) {
    const A = extra[a], B = extra[b], ia = items[a], ib = items[b];
    const sa = new Set(A.words), shared = B.words.filter((w) => sa.has(w)).length;
    const name = shared ? shared / Math.max(A.words.length, B.words.length) : 0;
    const time = Math.exp(-Math.abs(A.t - B.t) / (60 * DAY));
    const kind = ia.kind === ib.kind ? 1 : 0, host = ia.download_host && ia.download_host === ib.download_host ? 1 : 0, folder = A.folder === B.folder ? 1 : 0;
    // a paper that names a dataset's words "mentions" it
    const mention = (ia.kind === "dataset" && ib.kind === "pdf" && shared) || (ib.kind === "dataset" && ia.kind === "pdf" && shared) ? 1 : 0;
    const w = { name: W.name * name, time: W.time * time, kind: W.kind * kind, host: W.host * host, folder: W.folder * folder, mention: 0.5 * mention };
    const score = Object.values(w).reduce((s, x) => s + x, 0);
    const parts = {}; for (const k in w) if (k !== "mention" || w[k]) parts[k] = score ? w[k] / score : 0;
    return { score, parts };
  }
  // cat=a,b: OR within a group, AND across groups (like lore_catalog::category::Filter)
  const parseCat = (s) => { const m = { type: new Set(), use: new Set() }; for (const x of (s || "").split(",").filter(Boolean)) { if (TYPES.some((t) => t[0] === x)) m.type.add(x); else if (USES.some((u) => u[0] === x)) m.use.add(x); } return m; };
  const matches = (i, c) => { const cs = items[i].categories; return (!c.type.size || cs.some((x) => c.type.has(x))) && (!c.use.size || cs.some((x) => c.use.has(x))); };
  function top(a, k, skip, c) {
    const out = [];
    for (let b = 0; b < items.length; b++) if (b !== a && !skip.has(b) && matches(b, c)) out.push([b, sim(a, b)]);
    out.sort((x, y) => y[1].score - x[1].score);
    return out.slice(0, k);
  }
  function neighbors(id, k, layers, c) {
    const a = byId.get(id); if (a == null) return null;
    const seen = new Set([a]), res = [];
    let front = [a];
    for (let L = 1; L <= layers; L++) {
      const next = [];
      const kk = L === 1 ? k : Math.max(2, Math.ceil(k / 4));
      for (const f of front) for (const [b, s] of top(f, kk, seen, c)) { seen.add(b); next.push(b); res.push({ id: items[b].id, score: +s.score.toFixed(4), layer: L, via: items[f].id, parts: s.parts }); }
      front = next.slice(0, L === 1 ? k : 0);
    }
    return res;
  }
  const categories = () => [...TYPES.map((t) => [...t, "type"]), ...USES.map((u) => [...u, "use"])].map(([id, label, group]) => ({ id, label, group, count: items.filter((it) => it.categories.includes(id)).length }));
  const dsOf = (i) => extra[i]?.ds;
  function members(d) { const out = []; for (const l of d.labels) out.push({ name: l, rel: l, size: 2e3 }); for (const [f, n] of d.folders) for (let j = 0; j < n; j++) { const name = `${String(j + 1).padStart(5, "0")}.${d.ext}`; out.push({ name, rel: `${f}/${name}`, size: 2e5 }); } return out.sort((a, b) => (a.rel < b.rel ? -1 : 1)); }

  async function api(method, path, body) {
    const u = new URL(path, "http://x");
    const p = u.pathname.split("/").filter(Boolean);
    const c = parseCat(u.searchParams.get("cat"));
    if (method === "GET" && p[1] === "files" && !p[2]) return items;
    if (method === "GET" && p[1] === "files") {
      const i = byId.get(p[2]); if (i == null) throw new Error("找不到這個項目");
      const it = items[i], E = extra[i];
      const d = dsOf(i);
      const what = d ? `${d.count.toLocaleString()} 個 ${d.ext.toUpperCase()} 檔` : `${E.words.join("、")}相關的${KL[it.kind] ?? it.kind}`;
      const v = { ...it, paths: [`${E.folder}${d ? "" : "/" + it.name}`], description: `${what}，修改於 ${it.times.modified.slice(0, 10)}。` };
      if (d) v.dataset = { files: d.count + d.labels.length, bytes: it.size, main_ext: d.ext, main_share: 0.99, image_sizes: [], meta_files: d.labels, subfolders: d.folders, more_subfolders: 0 };
      return v;
    }
    if (method === "PUT" && p[1] === "files" && p[3] === "categories") {
      const i = byId.get(p[2]), use = [...new Set(body?.categories ?? [])];
      items[i].categories = [typeOf(items[i].kind), ...use].filter(Boolean); items[i].sensitive = use.includes("id-doc");
      return { id: items[i].id, categories: use };
    }
    if (method === "GET" && p[1] === "neighbors") return neighbors(p[2], +(u.searchParams.get("k") || 12), +(u.searchParams.get("layers") || 2), c);
    if (method === "GET" && p[1] === "categories") return categories();
    if (method === "GET" && p[1] === "nearest") {
      const a = byId.get(u.searchParams.get("from")); if (a == null) throw new Error("找不到這個項目");
      const best = top(a, 1, new Set(), c)[0];
      if (!best) throw new Error("沒有符合的項目");
      return { item: items[best[0]], score: +best[1].score.toFixed(3), parts: best[1].parts };
    }
    if (method === "GET" && p[1] === "datasets" && p[3] === "items") {
      const d = dsOf(byId.get(p[2])); if (!d) throw new Error("找不到這個資料集");
      const m = members(d), off = +(u.searchParams.get("offset") || 0), lim = +(u.searchParams.get("limit") || 60);
      return { total: m.length, offset: off, items: m.slice(off, off + lim).map((x, k) => ({ index: off + k, ...x })) };
    }
    if (method === "POST" && (p[1] === "open" || p[1] === "reveal")) return { [p[1] === "open" ? "opened" : "revealed"]: true };
    throw new Error("not found");
  }

  root.ShelfDemo = { api, size: items.length };
})(typeof window !== "undefined" ? window : globalThis);
