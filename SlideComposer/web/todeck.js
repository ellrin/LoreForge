// 論文報告 (home tile 01 開始): a PaperComposer paper or a LoreReader paper becomes a
// paper-report deck (POST /api/todeck, crates/server/src/todeck.rs), then opens here.
// PaperComposer's 做成簡報 calls the same route from its side.
// The job view (progress, then the review window) is below the dialog code: the work runs
// on the server as a job (docs/paper-deck.md); this page only watches it, so closing never stops it.
"use strict";

const PD = { src: "pc", list: null, pick: null, busy: false };
const pdPref = (() => {
  let v = {};
  try { v = JSON.parse(localStorage.getItem("sc.todeck") ?? "{}") ?? {}; } catch {}
  return { get: (k, d) => v[k] ?? d, set: (k, x) => { v[k] = x; try { localStorage.setItem("sc.todeck", JSON.stringify(v)); } catch {} } };
})();

function pdSeg(el, key, def) {
  const cur = pdPref.get(key, def);
  el.querySelectorAll("button").forEach((b) => {
    b.setAttribute("aria-pressed", String(b.dataset.v === cur));
    b.onclick = () => { pdPref.set(key, b.dataset.v); el.querySelectorAll("button").forEach((x) => x.setAttribute("aria-pressed", String(x === b))); };
  });
}

async function openPaperDeck() {
  if (PD.busy) { $("#dlgPaper").showModal(); return; }
  pdSeg($("#pdLen"), "length", "standard");
  pdSeg($("#pdLang"), "lang", "zh");
  $$("#pdSrc button").forEach((b) => (b.onclick = () => { PD.src = b.dataset.src; PD.pick = null; pdPapers(); }));
  // bases: the same thumbnails as 新簡報
  const box = $("#pdBases");
  box.textContent = "";
  const styles = S.info?.style_list ?? (S.info?.styles ?? []).map((id) => ({ id, name: id, previews: 0 }));
  const want = styles.some((s) => s.id === pdPref.get("style")) ? pdPref.get("style") : styles[0]?.id;
  for (const st of styles) {
    const l = document.createElement("label");
    l.className = "base-opt";
    l.title = st.name;
    l.innerHTML = `<input type="radio" name="pdbase"><div class="thumb16"></div><span></span>`;
    const r = l.querySelector("input");
    r.value = st.id;
    r.checked = st.id === want;
    r.onchange = () => pdPref.set("style", st.id);
    l.querySelector("span").textContent = st.name;
    box.append(l);
    drawStyleCover(l.querySelector(".thumb16"), st.id).catch(() => l.querySelector(".thumb16").classList.add("blank"));
  }
  $("#dlgPaper").showModal();
  PD.list = null;
  await pdPapers();
}

async function pdPapers() {
  $$("#pdSrc button").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.src === PD.src)));
  const box = $("#pdPapers");
  if (!PD.list) {
    box.innerHTML = `<div class="empty">…</div>`;
    try { PD.list = await api("/api/todeck/papers"); } catch (e) { box.innerHTML = ""; toast(e.message, true); return; }
  }
  const items = (PD.src === "pc" ? PD.list.pc : PD.list.lr.filter((p) => p.pdf)) ?? [];
  box.textContent = "";
  if (!items.length) {
    const d = document.createElement("div");
    d.className = "empty";
    d.textContent = PD.src === "pc" ? "PaperComposer 沒有論文" : "LoreReader 沒有論文";
    box.append(d);
  }
  const key = (p) => p.path ?? p.sha;
  if (!items.some((p) => key(p) === PD.pick)) PD.pick = items[0] ? key(items[0]) : null;
  for (const p of items) {
    const b = document.createElement("button");
    b.type = "button";
    b.setAttribute("role", "radio");
    b.setAttribute("aria-checked", String(key(p) === PD.pick));
    b.innerHTML = `<span class="t"></span><span class="w"></span>`;
    b.querySelector(".t").textContent = p.title;
    b.querySelector(".w").textContent = p.name ?? "";
    b.title = p.title;
    b.onclick = () => { PD.pick = key(p); box.querySelectorAll("button").forEach((x) => x.setAttribute("aria-checked", String(x === b))); };
    b.ondblclick = () => pdCreate();
    box.append(b);
  }
  $("#pdGo").disabled = !PD.pick;
}

/** Make the deck; `engine` = the other engine after a route could not run (改用本機／改用 Claude). */
async function pdCreate(engine, mode = "review") {
  if (PD.busy || !PD.pick) return;
  const source = PD.src === "pc" ? { kind: "pc", path: PD.pick } : { kind: "lr", sha: PD.pick };
  const style = $("#pdBases input:checked")?.value;
  PD.busy = true;
  $("#pdGo").disabled = true;
  $("#pdBar").hidden = false;
  try {
    const r = await fetch("/api/todeck", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ source, style, length: pdPref.get("length", "standard"), lang: pdPref.get("lang", "zh"), engine, mode }) });
    const v = await r.json().catch(() => ({}));
    if (!r.ok) {
      const alt = v.alternative;
      toast(humanError(v.error ?? `${r.status}`), { err: true, action: alt ? { label: alt.label, run: () => pdCreate(alt.engine, mode) } : null, ms: 9000 });
      return;
    }
    $("#dlgPaper").close();
    if (v.job) TDJ.open(v.job); else openByName(v.name);
  } catch (e) {
    toast(e.message, true);
  } finally {
    PD.busy = false;
    $("#pdBar").hidden = true;
    $("#pdGo").disabled = !PD.pick;
  }
}

(() => {
  $("#tilePaper").onclick = openPaperDeck;
  // two ways in: 先看大綱 (the primary: the review window) and 直接產生 (defaults, no review)
  const go = $("#pdGo");
  go.firstChild.textContent = "先看大綱";
  const direct = document.createElement("button");
  direct.type = "button";
  direct.className = "btn small";
  direct.id = "pdDirect";
  direct.textContent = "直接產生";
  direct.onclick = () => pdCreate(undefined, "direct");
  go.before(direct);
  new MutationObserver(() => { direct.disabled = go.disabled; }).observe(go, { attributes: true, attributeFilter: ["disabled"] });
  $("#paperForm").onsubmit = (e) => { e.preventDefault(); pdCreate(); };
  $$("#dlgPaper .close").forEach((b) => (b.onclick = () => $("#dlgPaper").close()));
})();

// ───────────────────────── the job view ─────────────────────────
// Wire shapes (docs/paper-deck.md §API) are kept in `TDAPI` only; `TDMOCK` stands in with the same
// methods for ?job=mock (&at=run|model|review|done|interrupted) so every state can be looked at.
const TD_STAGES = [
  ["source", "讀論文"], ["notation", "對照表"], ["terms", "名詞"], ["profile", "風格"],
  ["outline", "大綱"], ["review", "微調"], ["slides", "投影片"], ["polish", "排版"],
];
const TD_KINDS = { cover: "封面", background: "背景", challenge: "問題", overview: "總覽", method: "方法", loss: "損失", training: "訓練細節", results: "結果", ablation: "消融", visual: "視覺化", opinion: "我的看法", takeaway: "Take home", thanks: "致謝" };

const TDAPI = {
  async j(path, opt) {
    const r = await fetch(path, opt);
    const v = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(humanError(v.error ?? `${r.status}`));
    return v;
  },
  post(path, body) { return this.j(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body ?? {}) }); },
  get: (id) => TDAPI.j(`/api/todeck/jobs/${encodeURIComponent(id)}`),
  list: () => TDAPI.j("/api/todeck/jobs?unfinished=1").then((v) => v.jobs ?? v),
  stop: (id) => TDAPI.post(`/api/todeck/jobs/${encodeURIComponent(id)}/stop`),
  resume: (id) => TDAPI.post(`/api/todeck/jobs/${encodeURIComponent(id)}/resume`),
  discard: (id) => TDAPI.j(`/api/todeck/jobs/${encodeURIComponent(id)}`, { method: "DELETE" }),
  review: (id, body) => TDAPI.post(`/api/todeck/jobs/${encodeURIComponent(id)}/review`, body),
};

// A synthetic job on a client clock: stages advance by time, the review stage waits for the author.
const TDMOCK = (() => {
  const at = new URL(location.href).searchParams.get("at") ?? "run";
  let t0 = Date.now(), reviewed = null, stopped = false;
  const svg = (c, n) => `data:image/svg+xml,${encodeURIComponent(`<svg xmlns='http://www.w3.org/2000/svg' width='64' height='40'><rect width='64' height='40' fill='${c}'/><text x='32' y='25' font-size='14' text-anchor='middle' fill='#fff' font-family='sans-serif'>${n}</text></svg>`)}`;
  const figures = [1, 2, 3, 4, 5, 6, 7, 8].map((n) => ({ id: `fig${n}`, label: `圖 ${n}`, thumb: svg(["#4a7", "#58b", "#a65", "#86a"][n % 4], n) }));
  const outline = [
    ["title", "封面", [], "題目、作者、日期"], ["background", "研究背景", ["fig1"], "為什麼這個問題重要"],
    ["problem", "現有方法的缺口", [], "三個限制"], ["method", "整體架構", ["fig2"], "四個模組串成一條流程"],
    ["method", "模組一：編碼器", ["fig3"], "把輸入壓成固定長度的表示"], ["method", "模組二：對齊", ["fig4", "fig5"], "對齊損失的設計"],
    ["result", "主要結果", ["fig6"], "比基線高 4.2 個百分點"], ["result", "消融實驗", ["fig7"], "每個模組都有貢獻"],
    ["discussion", "限制與討論", [], "資料量小、只測一個領域"], ["conclusion", "Take home", [], "三句話帶走"],
  ].map(([kind, title, figs, point], i) => ({ id: `s${i + 1}`, kind, title, figs, point }));
  const unknown = ["contrastive loss", "InfoNCE", "ablation", "temperature τ", "hard negative", "MoCo", "projection head", "linear probe", "cosine schedule", "EMA teacher", "BYOL", "SimCLR"].map((term, i) => ({ term, n: 12 - i }));
  const title = "Contrastive Learning for Sparse Signals";
  function snapshot() {
    if (at === "interrupted") return { id: "mock", title, state: "interrupted", step: 4, stages: TD_STAGES.map(([id], i) => ({ id, state: i < 3 ? "done" : "pending", ms: i < 3 ? 4000 : 0 })) };
    const dur = [6, 3, 5, 2, 14];
    const el = at === "review" || at === "done" ? 1e6 : at === "model" ? 17 + (Date.now() - t0) / 1000 : (Date.now() - t0) / 1000;
    let acc = 0, cur = null;
    const st = dur.map((d, i) => {
      const s = { id: TD_STAGES[i][0], state: "pending", ms: 0, f: Math.min(1, Math.max(0, (el - acc) / d)) };
      if (el >= acc + d) { s.state = "done"; s.ms = d * 1000; }
      else if (cur == null) { cur = i; s.state = "running"; s.ms = (el - acc) * 1000; }
      acc += d;
      return s;
    });
    st[0].counts = { fig: Math.round(8 * st[0].f), figTotal: 8, table: Math.round(3 * st[0].f), eq: Math.round(5 * st[0].f) };
    st[1].counts = { entries: Math.round(17 * st[1].f) };
    st[2].counts = { terms: Math.round(42 * st[2].f), unknown: Math.round(12 * st[2].f) };
    if (st[4].state === "running") { st[4].chars = Math.round(1840 * st[4].f); st[4].agoMs = 700 + (Math.floor(el) % 3) * 900; }
    const reviewing = cur == null;
    const out = { id: "mock", title, state: reviewing ? "review" : "running", stage: reviewing ? "review" : TD_STAGES[cur][0] };
    out.stages = TD_STAGES.map(([id], i) => st[i] ?? { id, state: "pending", ms: 0 });
    if (reviewing) { out.stages[5] = { id: "review", state: "running", ms: 0 }; out.review = { outline, figures, unknown }; }
    if (reviewed || at === "done") {
      out.state = "done";
      out.deck = "demo";
      out.stages = TD_STAGES.map(([id]) => ({ id, state: "done", ms: 5000, counts: id === "slides" ? { slide: 12, slides: 12 } : id === "polish" ? { checks: 23 } : undefined }));
    }
    if (stopped) out.state = "stopped";
    return out;
  }
  return {
    get: async () => snapshot(),
    list: async () => [],
    stop: async () => { stopped = true; },
    resume: async () => { stopped = false; t0 = Date.now(); },
    discard: async () => {},
    review: async (_id, body) => { reviewed = body; window.__tdReview = body; },
  };
})();
const tdApi = () => (new URL(location.href).searchParams.get("job") === "mock" || TDJ.id === "mock" ? TDMOCK : TDAPI);

const TDJ = (() => {
  const E = (tag, attrs = {}, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v;
      else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat()) if (c != null && c !== false) e.append(c);
    return e;
  };
  const J = { id: null, snap: null, at: 0, timer: 0, root: null, mode: "", busy: false, live: false };
  const fmt = (ms) => { const s = Math.max(0, Math.round(ms / 1000)); return s < 60 ? `${s} 秒` : `${Math.floor(s / 60)} 分 ${String(s % 60).padStart(2, "0")} 秒`; };
  const ic = (name) => { const s = document.createElement("span"); s.dataset.icon = name; s.style.display = "inline-flex"; return s; };
  const paint = (el) => { try { paintIcons(el); } catch {} };

  function countText(st) {
    const c = st.counts;
    if (!c) return "";
    const p = [];
    if (c.figTotal != null) p.push(`圖 ${c.fig ?? 0}/${c.figTotal}`);
    if (c.table != null) p.push(`表 ${c.table}`);
    if (c.eq != null) p.push(`方程式 ${c.eq}`);
    if (c.entries != null) p.push(`條目 ${c.entries}`);
    if (c.terms != null) p.push(c.unknown != null ? `名詞 ${c.terms} → 不熟 ${c.unknown}` : `名詞 ${c.terms}`);
    if (c.slides != null) p.push(`第 ${c.slide ?? 0}/${c.slides} 頁`);
    if (c.checks != null) p.push(`檢查 ${c.checks} 項`);
    return p.join(" · ");
  }

  function shell() {
    if (J.root) return J.root;
    J.root = E("section", { class: "tdj", id: "tdJob", "aria-label": "論文報告" });
    document.body.append(J.root);
    return J.root;
  }

  function head(snap) {
    const h = E("header", { class: "tdj-head" },
      E("h2", {}, snap?.title ?? "論文報告"),
      E("span", { class: "grow", style: "flex:1" }),
      E("button", { type: "button", class: "ico close", "aria-label": "關閉 Esc", onclick: close }, ic("x")));
    paint(h);
    return h;
  }
  const spacer = () => E("span", { style: "flex:1" });
  const stepOf = (snap) => snap.stages.filter((s) => s.state === "done").length + 1;

  // ── progress ──
  function paintProgress() {
    const snap = J.snap;
    const root = shell();
    const now = Date.now() - J.at;
    const list = E("ol", { class: "tdj-stages" });
    for (const [id, name] of TD_STAGES) {
      const st = snap.stages.find((x) => x.id === id) ?? { id, state: "pending", ms: 0 };
      const run = st.state === "running" && snap.state === "running";
      const parts = [];
      const ct = countText(st);
      if (ct) parts.push(E("span", {}, ct));
      if (run && st.chars != null) {
        parts.push(E("span", {}, `${st.chars.toLocaleString()} 字`));
        const ago = (st.agoMs ?? 0) + now;
        parts.push(E("span", { class: ago > 30000 ? "stale" : "" }, `${Math.max(1, Math.round(ago / 1000))} 秒前還在寫`));
      }
      const el = st.state === "done" ? st.ms : st.state === "running" ? st.ms + (run ? now : 0) : 0;
      list.append(E("li", { class: `tdj-st ${st.state}`, "aria-current": run ? "step" : null },
        E("span", { class: "mk", "aria-hidden": "true" }, st.state === "done" ? ic("check") : ""),
        E("span", { class: "nm" }, name),
        E("span", { class: "ct" }, ...parts.flatMap((x, i) => (i ? [E("i", {}, "·"), x] : [x]))),
        E("span", { class: "tm" }, el > 0 ? fmt(el) : "")));
    }
    const foot = E("footer", { class: "tdj-foot" }, spacer());
    const msg = snap.state === "failed" ? snap.error ?? "沒有完成" : snap.state === "stopped" ? "已停止" : snap.state === "interrupted" ? `上次做到第 ${snap.step ?? stepOf(snap)} 步` : "";
    if (snap.state === "running") foot.append(E("button", { type: "button", class: "btn danger", onclick: stop }, "停止"));
    else if (snap.state === "done") foot.append(E("button", { type: "button", class: "btn primary", onclick: () => openByName(snap.deck) }, "打開簡報"));
    else foot.append(E("button", { type: "button", class: "btn", onclick: discard }, "捨棄"), E("button", { type: "button", class: "btn primary", onclick: resume }, "繼續"));
    root.replaceChildren(head(snap), E("div", { class: "tdj-body" }, list, msg ? E("p", { class: "tdj-msg" + (snap.state === "failed" ? " err" : "") }, msg) : null), foot);
    paint(root);
    J.mode = "progress";
  }

  async function stop() { try { await tdApi().stop(J.id); } catch (e) { toast(e.message, true); } poll(); }
  async function resume() { try { await tdApi().resume(J.id); } catch (e) { toast(e.message, true); } J.live = true; poll(); }
  async function discard() { try { await tdApi().discard(J.id); } catch (e) { toast(e.message, true); } close(); }

  // ── review window ──
  function paintReview() {
    const snap = J.snap, rv = snap.review ?? { outline: [], figures: [], unknown: [] };
    const root = shell();
    const rows = rv.outline.map((r) => ({ ...r, figs: [...(r.figs ?? [])] }));
    const figs = new Map(rv.figures.map((f) => [f.id, f]));
    const ans = { audience: null, mainFig: null, modules: null, opinion: null };
    const known = new Set();
    let remember = false;

    const body = E("div", { class: "tdj-body wide" });
    const bOut = LoreShell.block({ n: "01", title: "大綱", count: String(rows.length), key: "tdj.outline" });
    const bQ = LoreShell.block({ n: "02", title: "問題", key: "tdj.q" });
    const bT = LoreShell.block({ n: "03", title: `這 ${rv.unknown.length} 個詞不在你的知識庫`, key: "tdj.terms" });

    const list = E("ol", { class: "tdj-outline" });
    function paintRows() {
      list.textContent = "";
      bOut.setCount(String(rows.length));
      rows.forEach((r, i) => {
        const th = E("span", { class: "th" });
        for (const id of r.figs) {
          const f = figs.get(id);
          th.append(E("button", { type: "button", class: "fig", title: `${f?.label ?? id}`, "aria-label": `換圖 ${f?.label ?? id}`, onclick: (e) => swap(e.currentTarget, r, id) }, f?.thumb ? E("img", { src: f.thumb, alt: f.label ?? id }) : f?.label ?? id));
        }
        if (!r.figs.length && rv.figures.length) th.append(E("button", { type: "button", class: "fig add", "aria-label": "加圖", onclick: (e) => swap(e.currentTarget, r, null) }, ic("plus")));
        list.append(E("li", { class: "row" },
          E("span", { class: "mv" },
            E("button", { type: "button", class: "ico tiny", "aria-label": "上移", disabled: i === 0, onclick: () => { [rows[i - 1], rows[i]] = [rows[i], rows[i - 1]]; paintRows(); } }, ic("up")),
            E("button", { type: "button", class: "ico tiny down", "aria-label": "下移", disabled: i === rows.length - 1, onclick: () => { [rows[i + 1], rows[i]] = [rows[i], rows[i + 1]]; paintRows(); } }, ic("up"))),
          E("span", { class: "kd" }, TD_KINDS[r.kind] ?? r.kind),
          E("input", { class: "ti", value: r.title, "aria-label": "標題", onchange: (e) => (r.title = e.target.value) }),
          th,
          E("input", { class: "pt", value: r.point ?? "", "aria-label": "重點", onchange: (e) => (r.point = e.target.value) }),
          E("button", { type: "button", class: "ico tiny", "aria-label": "刪除", onclick: () => { rows.splice(i, 1); paintRows(); } }, ic("trash"))));
      });
      paint(list);
    }
    function swap(anchor, r, cur) {
      const items = rv.figures.map((f) => ({ label: f.label, current: f.id === cur, run: () => { if (cur == null) r.figs.push(f.id); else r.figs[r.figs.indexOf(cur)] = f.id; paintRows(); } }));
      if (cur != null) items.push({ label: "移除", run: () => { r.figs = r.figs.filter((x) => x !== cur); paintRows(); } });
      LoreShell.menu(anchor, items);
    }
    bOut.body.append(list);
    paintRows();

    // four questions, every one skippable (tap the chosen one again to clear it)
    const q = (label, ctl) => E("div", { class: "tdj-q" }, E("span", { class: "ql" }, label), ctl);
    const seg = (opts, key) => {
      const s = E("div", { class: "seg", role: "radiogroup" });
      for (const [v, t] of opts) s.append(E("button", { type: "button", "aria-pressed": "false", "data-v": String(v), onclick: () => {
        ans[key] = ans[key] === v ? null : v;
        s.querySelectorAll("button").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.v === String(ans[key]))));
      } }, t));
      return s;
    };
    const figSel = E("select", { class: "tdj-sel", "aria-label": "主架構圖", onchange: (e) => (ans.mainFig = e.target.value || null) }, E("option", { value: "" }, "—"), ...rv.figures.map((f) => E("option", { value: f.id }, f.label)));
    bQ.body.append(
      q("聽眾", seg([["lab", "實驗室組會"], ["seminar", "研討會"], ["defense", "口試"]], "audience")),
      q("主架構圖", figSel),
      q("方法拆幾個模組", seg([[2, "2"], [3, "3"], [4, "4"], [5, "5"]], "modules")),
      q("我的看法頁", seg([[true, "要"], [false, "不要"]], "opinion")));

    const tl = E("ul", { class: "tdj-terms" });
    for (const t of rv.unknown) {
      tl.append(E("li", {}, E("label", {}, E("input", { type: "checkbox", onchange: (e) => (e.target.checked ? known.add(t.term) : known.delete(t.term)) }), E("span", { class: "tm" }, t.term), t.n ? E("span", { class: "n" }, `×${t.n}`) : null, E("span", { class: "kn" }, "我懂"))));
    }
    bT.body.append(tl, E("label", { class: "tdj-rem" }, E("input", { type: "checkbox", onchange: (e) => (remember = e.target.checked) }), "同時存進知識庫（記得）"));
    if (!rv.unknown.length) bT.style.display = "none";

    body.append(bOut, bQ, bT);
    const submit = async (skip) => {
      if (J.busy) return;
      J.busy = true;
      try {
        await tdApi().review(J.id, skip ? { skip: true } : { outline: rows.map((r) => ({ id: r.id, kind: r.kind, title: r.title, figs: r.figs, point: r.point })), answers: ans, known: [...known], remember });
        J.mode = "";
        J.live = true;
        poll();
      } catch (e) { toast(e.message, true); } finally { J.busy = false; }
    };
    root.replaceChildren(head(snap), body, E("footer", { class: "tdj-foot" }, spacer(), E("button", { type: "button", class: "btn", onclick: () => submit(true) }, "略過"), E("button", { type: "button", class: "btn primary", onclick: () => submit(false) }, "確定")));
    paint(root);
    J.mode = "review";
  }

  // ── polling ──
  async function poll() {
    clearTimeout(J.timer);
    if (!J.id) return;
    try {
      const snap = await tdApi().get(J.id);
      J.snap = snap; J.at = Date.now();
      if (snap.state === "running") J.live = true;
      if (snap.state === "review") { if (J.mode !== "review") paintReview(); }
      else {
        paintProgress();
        if (snap.state === "done" && snap.deck && J.live) { J.live = false; const id = J.id; setTimeout(() => J.id === id && openByName(snap.deck), 1200); }
      }
      if (snap.state === "running" || snap.state === "review") J.timer = setTimeout(poll, 1000);
    } catch (e) {
      shell().replaceChildren(head(null), E("div", { class: "tdj-body" }, E("p", { class: "tdj-msg err" }, e.message)), E("footer", { class: "tdj-foot" }, spacer(), E("button", { type: "button", class: "btn primary", onclick: poll }, "重試")));
      J.mode = "";
    }
  }
  // between polls only the clock text moves
  setInterval(() => { if (J.id && J.mode === "progress" && J.snap?.state === "running") paintProgress(); }, 1000);

  function open(id) {
    J.id = id; J.snap = null; J.mode = ""; J.live = false;
    try { history.replaceState(null, "", `?job=${encodeURIComponent(id)}`); } catch {}
    shell().hidden = false;
    document.addEventListener("keydown", esc, true);
    poll();
  }
  const esc = (e) => { if (e.key === "Escape" && J.root && !J.root.hidden && !document.querySelector("dialog[open], .ask, .menu")) { e.preventDefault(); close(); } };
  function close() {
    clearTimeout(J.timer);
    J.id = null; J.mode = "";
    if (J.root) J.root.hidden = true;
    document.removeEventListener("keydown", esc, true);
    try { const u = new URL(location.href); u.searchParams.delete("job"); u.searchParams.delete("at"); history.replaceState(null, "", u); } catch {}
  }

  // ── start-up: ?job=<id>, else 「上次做到第 N 步，繼續？」 for a job a quit left unfinished ──
  async function boot() {
    const u = new URL(location.href);
    const id = u.searchParams.get("job");
    if (id) { open(id); return; }
    let jobs = [];
    try { jobs = (await TDAPI.list()).filter((j) => j.state === "interrupted"); } catch {}
    if (u.searchParams.get("at") === "interrupted") jobs = [{ id: "mock", step: 4 }];
    const j = jobs[0];
    if (!j) return;
    const pick = await LoreShell.form({
      title: "論文報告",
      build: (b) => b.append(E("p", {}, `上次做到第 ${j.step ?? "?"} 步，繼續？`)),
      actions: [{ id: "drop", label: "捨棄", run: async () => { try { await (j.id === "mock" ? TDMOCK : TDAPI).discard(j.id); } catch {} } }, { id: "go", label: "繼續", primary: true }],
    });
    if (pick === "go") { J.id = j.id; try { await tdApi().resume(j.id); } catch (e) { toast(e.message, true); } open(j.id); J.live = true; }
  }
  setTimeout(boot, 400);
  return { open, close, get id() { return J.id; } };
})();
