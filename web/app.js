// KnowledgeBase web UI: card list with filters, card editor (Markdown + KaTeX
// preview, image paste/drop, link picker), review, timeline, graph.
// Talks only to kb-server's JSON API (same origin).

const $ = (id) => document.getElementById(id);

/** Build an element: h("div", { class: "x", onclick }, "text", child…). */
function h(tag, attrs = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs ?? {})) {
    if (v == null || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = v;
    else if (k === "html") el.innerHTML = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const c of kids.flat()) if (c != null && c !== false) el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  return el;
}

async function api(method, path, body, raw) {
  const opt = { method, headers: {} };
  if (raw) opt.body = raw;
  else if (body !== undefined) { opt.body = JSON.stringify(body); opt.headers["content-type"] = "application/json"; }
  const res = await fetch(path, opt);
  const data = await res.json().catch(() => ({}));
  if (!res.ok) { const e = new Error(data.error ?? res.statusText); e.status = res.status; e.category = data.category; throw e; }
  return data;
}

let toastTimer;
function toast(msg, err = false) {
  const t = $("toast");
  t.textContent = msg; t.className = err ? "toast err" : "toast"; t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), err ? 5000 : 2600);
}
const fail = (e) => toast(errorText(e), true);
function errorText(e) {
  const m = { conflict: "這張卡在別處被修改過，請重新開啟後再存。", forbidden: "不允許：", not_found: "找不到：" }[e.category];
  if (e.category === "conflict") return m;
  return (m ?? "") + e.message;
}

const STATE_LABEL = { learning: "學習中", known: "記得", stale: "久未複習", none: "尚無紀錄" };
const stateChip = (s) => h("span", { class: "st" }, h("i", { class: `st-${s ?? "none"}` }), STATE_LABEL[s ?? "none"]);
const fmtDay = (iso) => { const d = new Date(iso); return `${d.getMonth() + 1}/${d.getDate()}`; };
const fmtTime = (iso) => { const d = new Date(iso); return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`; };
const localDay = (offsetDays = 0) => { const d = new Date(); d.setDate(d.getDate() - offsetDays); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`; };
const splitList = (s) => s.split(/[,，、]/).map((x) => x.trim()).filter(Boolean);
const assetUrl = (space, img) => `/api/assets/${encodeURIComponent(space)}/${img.asset.replace(/^sha256:/, "")}.${img.ext}`;

const S = {
  member: "", spaces: [], reg: { kinds: [], link_types: [], tag_counts: [] },
  view: "cards", dateField: "created",
  titles: new Map(), // id → title
  names: new Map(), // lowercased title/alias → id
  ed: null, // { id, space, revision, images, links, extra }
  due: [], reviewAt: 0, reveal: false,
  graphCenter: null,
};

const kindLabel = (id) => S.reg.kinds.find((k) => k.id === id)?.label ?? id;
const relLabel = (id) => S.reg.link_types.find((k) => k.id === id)?.label ?? id;
const spaceName = (id) => S.spaces.find((s) => s.id === id)?.name ?? id;

// ── Boot ───────────────────────────────────────────────
async function boot() {
  paintIcons();
  try {
    const sp = await api("GET", "/api/spaces");
    S.member = sp.member; S.spaces = sp.spaces;
    $("memberChip").textContent = S.member;
    S.reg = await api("GET", "/api/registry");
  } catch (e) { fail(e); return; }
  fillSelect($("fKind"), [["", "全部"], ...S.reg.kinds.map((k) => [k.id, k.label])]);
  fillSelect($("edKind"), S.reg.kinds.map((k) => [k.id, k.label]));
  fillSelect($("fSpace"), [["", "全部"], ...S.spaces.map((s) => [s.id, s.name])]);
  fillSelect($("edSpace"), S.spaces.map((s) => [s.id, `${s.name}${s.kind === "household" ? "（家庭）" : ""}`]));
  fillSelect($("pickerRel"), S.reg.link_types.map((l) => [l.id, l.label]));
  renderTags();
  renderSpacesFoot();
  await refreshNames();
  wire();
  setView("cards");
  refreshDue();
}

function fillSelect(sel, pairs) {
  sel.replaceChildren(...pairs.map(([v, t]) => h("option", { value: v }, t)));
}

function renderTags() {
  const cur = $("fTag").value;
  fillSelect($("fTag"), [["", "全部"], ...S.reg.tag_counts.map((t) => [t.tag, `${"　".repeat(t.tag.split("/").length - 1)}${t.tag.split("/").pop()}（${t.count}）`])]);
  $("fTag").value = cur;
}

function renderSpacesFoot() {
  $("spacesFoot").replaceChildren(...S.spaces.map((s) => h("div", { class: `sp ${s.kind}`, title: `${s.kind === "household" ? "家庭共用" : "個人"} · 圖片：${s.assets_backend}` }, h("i"), `${s.name} · ${s.cards} 張`)));
  $("cCards").textContent = S.spaces.reduce((a, s) => a + s.cards, 0) || "";
}

async function refreshNames() {
  const { nodes } = await api("GET", "/api/nodes?limit=5000&sort=title");
  S.titles.clear(); S.names.clear();
  for (const n of nodes) {
    S.titles.set(n.id, n.title);
    for (const name of [n.title, ...n.aliases]) S.names.set(name.toLowerCase(), n.id);
  }
}

async function refreshMeta() {
  try {
    const [sp, reg] = await Promise.all([api("GET", "/api/spaces"), api("GET", "/api/registry")]);
    S.spaces = sp.spaces; S.reg = reg;
    renderSpacesFoot(); renderTags();
    await refreshNames();
    refreshDue();
  } catch (e) { fail(e); }
}

// ── Views ──────────────────────────────────────────────
function setView(v) {
  S.view = v;
  document.querySelectorAll(".view").forEach((s) => (s.hidden = s.dataset.view !== v));
  document.querySelectorAll("#views button").forEach((b) => b.toggleAttribute("aria-current", b.dataset.view === v || (v === "editor" && b.dataset.view === "cards")));
  document.querySelectorAll("#views button[aria-current]").forEach((b) => b.setAttribute("aria-current", "page"));
  $("filters").hidden = !(v === "cards");
  $("left").classList.remove("open");
  if (v === "cards") loadCards();
  if (v === "review") loadReview();
  if (v === "timeline") loadTimeline();
  if (v === "graph") loadGraph();
}

function wire() {
  $("views").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-view]");
    if (!b) return;
    if (b.dataset.view === "graph" && S.view === "editor" && S.ed?.id) S.graphCenter = S.ed.id;
    setView(b.dataset.view);
  });
  for (const id of ["fTime", "fState", "fKind", "fTag", "fSpace", "fSort"]) $(id).addEventListener("change", loadCards);
  $("fDateField").addEventListener("click", (e) => {
    const b = e.target.closest("button"); if (!b) return;
    S.dateField = b.dataset.v;
    $("fDateField").querySelectorAll("button").forEach((x) => x.setAttribute("aria-pressed", x === b));
    loadCards();
  });
  let qt;
  $("q").addEventListener("input", () => { clearTimeout(qt); qt = setTimeout(() => { if (S.view !== "cards") setView("cards"); else loadCards(); }, 200); });
  $("actNew").onclick = () => openEditor(null);
  $("actNav").onclick = () => $("left").classList.toggle("open");
  $("actMode").onclick = () => {
    const m = document.documentElement.dataset.mode === "dark" ? "light" : "dark";
    document.documentElement.dataset.mode = m;
    try { localStorage.setItem("kb.mode", m); } catch {}
    if (S.view === "graph") Graph.draw();
  };
  $("actReindex").onclick = async (e) => {
    const b = e.currentTarget; b.classList.add("busy");
    try { const r = await api("POST", "/api/reindex"); toast(`已重建索引：${r.nodes} 張卡片${r.errors.length ? `，${r.errors.length} 個檔案無法讀取` : ""}`); await refreshMeta(); setView(S.view); }
    catch (er) { fail(er); } finally { b.classList.remove("busy"); }
  };
  wireEditor();
  $("tlDays").onchange = loadTimeline;
  $("gTags").onchange = loadGraph;
  $("gDepth").onchange = loadGraph;
  $("gAll").onclick = () => { S.graphCenter = null; loadGraph(); };
  Graph.init($("graph"), (id) => openEditor(id));
  document.addEventListener("keydown", reviewKeys);
}

// ── Card list ──────────────────────────────────────────
async function loadCards() {
  const p = new URLSearchParams();
  const add = (k, v) => v && p.set(k, v);
  add("q", $("q").value.trim());
  add("state", $("fState").value); add("kind", $("fKind").value); add("tag", $("fTag").value); add("space", $("fSpace").value);
  add("sort", $("fSort").value);
  const days = +$("fTime").value;
  if (days) { p.set("since", localDay(days - 1)); p.set("date_field", S.dateField); }
  p.set("limit", "500");
  try {
    const { nodes } = await api("GET", `/api/nodes?${p}`);
    $("cardsInfo").textContent = `${nodes.length} 張`;
    const list = $("cardList");
    if (!nodes.length) {
      list.replaceChildren(h("div", { class: "empty" }, p.toString() === "sort=updated&limit=500" ? "還沒有知識卡。按右上角 ＋ 新增第一張。" : "沒有符合條件的卡片。"));
      return;
    }
    list.replaceChildren(...nodes.map(cardEl));
  } catch (e) { fail(e); }
}

function cardEl(n) {
  return h("button", { class: "card", onclick: () => openEditor(n.id) },
    n.image ? h("img", { class: "thumb", alt: "", loading: "lazy", src: assetUrl(n.space, n.image) }) : null,
    h("div", { class: "t" }, n.title, stateChip(n.state)),
    n.aliases.length ? h("div", { class: "al" }, n.aliases.join(" · ")) : null,
    n.summary ? h("div", { class: "s" }, n.summary) : null,
    h("div", { class: "meta" },
      h("span", { class: "tag kind" }, kindLabel(n.kind)),
      S.spaces.length > 1 ? h("span", { class: "tag space" }, spaceName(n.space)) : null,
      ...n.tags.map((t) => h("span", { class: "tag" }, `#${t}`))));
}

// ── Editor ─────────────────────────────────────────────
function wireEditor() {
  $("edBack").onclick = () => setView("cards");
  $("edSave").onclick = saveCard;
  $("edForm").addEventListener("submit", (e) => { e.preventDefault(); saveCard(); });
  let pt;
  $("edForm").addEventListener("input", () => { clearTimeout(pt); pt = setTimeout(renderPreview, 120); });
  $("edSpace").onchange = renderPreview;
  $("edDelete").onclick = deleteCard;
  $("edShare").onclick = shareCard;
  const body = $("edBody");
  body.addEventListener("paste", (e) => {
    const files = [...(e.clipboardData?.files ?? [])].filter((f) => f.type.startsWith("image/"));
    if (files.length) { e.preventDefault(); files.forEach(uploadImage); }
  });
  body.addEventListener("dragover", (e) => { if ([...e.dataTransfer.types].includes("Files")) { e.preventDefault(); body.classList.add("drop"); } });
  body.addEventListener("dragleave", () => body.classList.remove("drop"));
  body.addEventListener("drop", (e) => {
    body.classList.remove("drop");
    const files = [...e.dataTransfer.files];
    if (files.length) { e.preventDefault(); files.forEach(uploadImage); }
  });
  $("edAddImage").onclick = () => $("edImageFile").click();
  $("edImageFile").onchange = (e) => { [...e.target.files].forEach(uploadImage); e.target.value = ""; };
  $("edInsertLink").onclick = () => pick("wiki");
  $("edAddLink").onclick = () => pick("link");
  $("edDraft").onclick = () => { $("draftTerm").value = $("edTitle").value; $("draftDlg").showModal(); };
  $("draftGo").onclick = async (e) => {
    e.preventDefault();
    try {
      const { draft, assistant } = await api("POST", "/api/draft", { term: $("draftTerm").value, context: $("draftCtx").value, kind: $("edKind").value });
      if (!$("edTitle").value.trim()) $("edTitle").value = draft.title ?? "";
      if (!$("edSummary").value.trim()) $("edSummary").value = draft.summary ?? "";
      if (!$("edBody").value.trim()) $("edBody").value = draft.body ?? "";
      $("draftDlg").close();
      renderPreview();
      toast(`草稿已填入（${assistant}）`);
    } catch (er) { fail(er); }
  };
  let pq;
  $("pickerQ").addEventListener("input", () => { clearTimeout(pq); pq = setTimeout(searchPicker, 150); });
}

async function openEditor(id) {
  setView("editor");
  const isNew = !id;
  $("edDelete").hidden = isNew; $("edShare").hidden = true; $("edShareTo").hidden = true;
  $("edSpace").disabled = !isNew;
  $("edHeading").textContent = isNew ? "新增知識卡" : "編輯知識卡";
  $("edBacklinks").replaceChildren(); $("edProgress").replaceChildren();
  if (isNew) {
    const def = S.spaces.find((s) => s.kind === "personal" && s.is_member) ?? S.spaces[0];
    S.ed = { id: null, space: def?.id, revision: null, images: [], links: [], };
    for (const f of ["edTitle", "edAliases", "edTags", "edDomains", "edSummary", "edBody"]) $(f).value = "";
    $("edKind").value = "concept";
    $("edSpace").value = def?.id ?? "";
    $("edMeta").textContent = "";
    renderEdLists(); renderPreview();
    $("edTitle").focus();
    return;
  }
  try {
    const v = await api("GET", `/api/nodes/${id}`);
    const n = v.node;
    S.ed = { id: n.id, space: v.space, revision: n.revision, images: n.images ?? [], links: n.links ?? [] };
    $("edTitle").value = n.title; $("edAliases").value = n.aliases.join(", ");
    $("edKind").value = n.kind; $("edSpace").value = v.space;
    $("edTags").value = n.tags.join(", "); $("edDomains").value = n.domains.join(", ");
    $("edSummary").value = n.summary; $("edBody").value = n.body;
    $("edMeta").textContent = `建立 ${n.created_at.slice(0, 10)} · 修改 ${n.updated_at.slice(0, 10)} · 第 ${n.revision} 版`;
    const others = S.spaces.filter((s) => s.id !== v.space);
    if (others.length) {
      fillSelect($("edShareTo"), others.map((s) => [s.id, s.name]));
      $("edShare").hidden = false; $("edShareTo").hidden = false;
    }
    renderEdLists(); renderPreview();
    renderSide(v);
  } catch (e) { fail(e); setView("cards"); }
}

function renderSide(v) {
  const link = (id, extra) => h("li", {}, h("a", { onclick: () => openEditor(id) }, S.titles.get(id) ?? id), extra ? ` · ${extra}` : "");
  const back = v.backlinks.map((b) => link(b.from, b.rel === "mentions" ? "內文提到" : relLabel(b.rel)));
  $("edBacklinks").replaceChildren(...(back.length || v.unresolved.length ? [
    back.length ? h("h4", {}, `反向連結（${back.length}）`) : null,
    back.length ? h("ul", {}, back) : null,
    v.unresolved.length ? h("h4", {}, "尚未建立的 [[連結]]") : null,
    v.unresolved.length ? h("ul", {}, v.unresolved.map((t) => h("li", {}, h("a", { onclick: () => newFromTitle(t) }, `＋ ${t}`)))) : null,
  ].filter(Boolean) : []));
  const p = v.progress;
  if (!p) { $("edProgress").replaceChildren(h("h4", {}, "學習狀態"), h("div", { class: "muted" }, "尚無紀錄（複習時作答即開始記錄）")); return; }
  const ev = { created: "建立", remembered: "記得", forgot: "忘了", unfamiliar: "標為不熟", encountered: "遇到", known: "成為「記得」", stale: "久未複習", shared: "分享" };
  $("edProgress").replaceChildren(
    h("h4", {}, "學習狀態"),
    h("div", {}, stateChip(p.state), ` · 連續記得 ${p.streak} 次 · 遇到 ${p.encounters} 次`),
    h("div", { class: "muted" }, p.next_check_at ? `下次確認：${p.next_check_at.slice(0, 10)}` : "不需再確認"),
    h("ul", {}, p.log.slice(-5).reverse().map((l) => h("li", { class: "muted" }, `${l.at.slice(0, 10)} ${ev[l.event] ?? l.event}${l.early ? "（未到期，不計）" : ""}${l.via ? ` · ${l.via}` : ""}`))));
}

function newFromTitle(t) {
  openEditor(null).then(() => { $("edTitle").value = t; renderPreview(); });
}

function renderEdLists() {
  const space = $("edSpace").value;
  $("edImages").replaceChildren(...S.ed.images.map((img, i) => h("figure", {},
    h("img", { src: assetUrl(space, img), alt: img.caption ?? "" }),
    h("input", { value: img.caption ?? "", placeholder: "圖說", oninput: (e) => { img.caption = e.target.value; renderPreview(); } }),
    h("input", { value: img.credit ?? "", placeholder: "來源", oninput: (e) => { img.credit = e.target.value; } }),
    iconButton("x", "移除圖片", () => { S.ed.images.splice(i, 1); renderEdLists(); renderPreview(); }))));
  $("edLinks").replaceChildren(...S.ed.links.map((l, i) => h("li", {},
    h("span", { class: "rel" }, relLabel(l.rel)), S.titles.get(l.to) ?? l.to,
    iconButton("x", "移除連結", () => { S.ed.links.splice(i, 1); renderEdLists(); }))));
  if (!S.ed.links.length) $("edLinks").append(h("li", { class: "muted" }, "沒有明確連結（內文的 [[標題]] 會自動成為「提到」連結）"));
}

function renderPreview() {
  const space = $("edSpace").value;
  const title = $("edTitle").value.trim();
  const aliases = splitList($("edAliases").value);
  const resolve = (t) => S.names.has(t.toLowerCase());
  const html = [
    `<h1>${MD.esc(title || "（未命名）")}</h1>`,
    aliases.length ? `<div class="p-al">${MD.esc(aliases.join(" · "))}</div>` : "",
    $("edSummary").value.trim() ? `<div class="p-sum">${MD.render($("edSummary").value, { resolve })}</div>` : "",
    MD.render($("edBody").value, { resolve }),
    ...(S.ed?.images ?? []).map((img) => `<figure><img src="${assetUrl(space, img)}" alt=""><figcaption>${MD.esc(img.caption ?? "")}${img.credit ? ` · ${MD.esc(img.credit)}` : ""}</figcaption></figure>`),
  ].join("");
  const p = $("edPreview");
  p.innerHTML = html;
  p.querySelectorAll(".wikilink").forEach((w) => (w.onclick = () => {
    const id = S.names.get(w.dataset.target.toLowerCase());
    id ? openEditor(id) : newFromTitle(w.dataset.target);
  }));
}

async function uploadImage(file) {
  const space = $("edSpace").value;
  const ext = (file.name.match(/\.([a-z0-9]+)$/i)?.[1] ?? file.type.split("/")[1] ?? "png").toLowerCase();
  try {
    const r = await api("POST", `/api/assets?space=${encodeURIComponent(space)}&ext=${encodeURIComponent(ext)}`, undefined, file);
    S.ed.images.push({ asset: r.asset, ext: r.ext, caption: "", credit: "", marks: [] });
    renderEdLists(); renderPreview();
    toast("已加入圖片");
  } catch (e) { fail(e); }
}

function fieldsFromForm() {
  return {
    title: $("edTitle").value, kind: $("edKind").value,
    aliases: splitList($("edAliases").value), tags: splitList($("edTags").value), domains: splitList($("edDomains").value),
    summary: $("edSummary").value, body: $("edBody").value,
    images: S.ed.images, links: S.ed.links,
  };
}

async function saveCard() {
  if (!$("edTitle").value.trim()) { toast("請輸入標題", true); $("edTitle").focus(); return; }
  try {
    let id;
    if (S.ed.id) {
      const r = await api("PUT", `/api/nodes/${S.ed.id}`, { revision: S.ed.revision, ...fieldsFromForm() });
      id = r.node.id;
    } else {
      const r = await api("POST", "/api/nodes", { space: $("edSpace").value, via: "kb", ...fieldsFromForm() });
      id = r.node.id;
    }
    toast("已儲存");
    await refreshMeta();
    openEditor(id);
  } catch (e) { fail(e); }
}

async function deleteCard() {
  if (!S.ed?.id || !confirm(`把「${$("edTitle").value}」移到垃圾桶？（檔案移到 space 的 trash/，不會刪除）`)) return;
  try { await api("DELETE", `/api/nodes/${S.ed.id}`); toast("已移到垃圾桶"); await refreshMeta(); setView("cards"); }
  catch (e) { fail(e); }
}

async function shareCard() {
  const to = $("edShareTo").value;
  const target = S.spaces.find((s) => s.id === to);
  const note = target?.kind === "household" ? "\n\n會複製一份到家庭空間（新的 id，圖片轉成 webp 存進共用 repo）；指向個人卡片的連結不會帶過去。" : "";
  if (!confirm(`把「${$("edTitle").value}」分享（複製）到「${target?.name ?? to}」？${note}`)) return;
  try { const r = await api("POST", `/api/nodes/${S.ed.id}/share`, { to_space: to }); toast(`已分享到 ${target?.name ?? to}`); await refreshMeta(); openEditor(r.node.id); }
  catch (e) { fail(e); }
}

// ── Picker ([[wikilink]] or explicit link) ─────────────
let pickMode = "wiki";
function pick(mode) {
  pickMode = mode;
  $("pickerTitle").textContent = mode === "wiki" ? "插入 [[連結]]" : "新增明確連結";
  $("pickerRel").hidden = mode === "wiki";
  $("pickerQ").value = "";
  $("picker").showModal();
  searchPicker();
}

async function searchPicker() {
  const q = $("pickerQ").value.trim();
  const p = new URLSearchParams({ limit: "30", sort: q ? "title" : "updated" });
  if (q) p.set("q", q);
  // Household cards may only link inside their own space.
  const space = $("edSpace").value;
  if (pickMode === "link" && S.spaces.find((s) => s.id === space)?.kind === "household") p.set("space", space);
  try {
    const { nodes } = await api("GET", `/api/nodes?${p}`);
    const list = nodes.filter((n) => n.id !== S.ed?.id);
    $("pickerList").replaceChildren(...list.map((n) => h("li", {}, h("button", { type: "button", onclick: () => picked(n) },
      h("span", {}, n.title), h("span", { class: "tag kind" }, kindLabel(n.kind)), n.aliases.length ? h("span", { class: "al" }, n.aliases.join(" · ")) : null))));
    if (!list.length) $("pickerList").append(h("li", { class: "muted" }, q ? `沒有「${q}」` : "還沒有其他卡片"));
  } catch (e) { fail(e); }
}

function picked(n) {
  $("picker").close();
  if (pickMode === "wiki") {
    const ta = $("edBody"), s = ta.selectionStart, e = ta.selectionEnd;
    const ins = `[[${n.title}]]`;
    ta.setRangeText(ins, s, e, "end");
    ta.focus();
  } else {
    const rel = $("pickerRel").value;
    if (!S.ed.links.some((l) => l.to === n.id && l.rel === rel)) S.ed.links.push({ to: n.id, rel });
    S.titles.set(n.id, n.title);
    renderEdLists();
  }
  renderPreview();
}

// ── Review ─────────────────────────────────────────────
async function refreshDue() {
  try { const { nodes } = await api("GET", "/api/recall/due?limit=200"); S.due = nodes; $("cDue").textContent = nodes.length || ""; }
  catch {}
}

async function loadReview() {
  await refreshDue();
  S.reviewAt = 0; S.reveal = false;
  renderReview();
}

async function renderReview() {
  const box = $("review");
  const n = S.due[S.reviewAt];
  $("reviewInfo").textContent = S.due.length ? `${Math.min(S.reviewAt + 1, S.due.length)} / ${S.due.length}` : "";
  if (!n) {
    box.replaceChildren(h("div", { class: "empty" }, S.due.length ? "這一輪複習完了。" :"目前沒有到期的卡片。新卡片建立後隔天會出現在這裡。"));
    return;
  }
  const answer = h("div", { class: "answer paper" });
  if (S.reveal) {
    try {
      const v = await api("GET", `/api/nodes/${n.id}`);
      answer.innerHTML = (v.node.summary ? `<div class="p-sum">${MD.render(v.node.summary)}</div>` : "") + MD.render(v.node.body, { resolve: (t) => S.names.has(t.toLowerCase()) })
        + (v.node.images ?? []).map((img) => `<figure><img src="${assetUrl(v.space, img)}" alt=""><figcaption>${MD.esc(img.caption ?? "")}</figcaption></figure>`).join("");
    } catch (e) { fail(e); }
  }
  box.replaceChildren(h("div", { class: "rv-card" },
    h("div", { class: "q" }, `${kindLabel(n.kind)} · ${STATE_LABEL[n.state ?? "none"]}${n.streak ? ` · 已連續記得 ${n.streak} 次` : ""}`),
    h("h3", {}, `還記得「${n.title}」嗎？`),
    n.aliases.length ? h("div", { class: "al" }, n.aliases.join(" · ")) : null,
    S.reveal ? answer : null,
    h("div", { class: "rv-actions" },
      h("button", { class: "btn primary", onclick: () => answerCard("remembered") }, "記得 ", h("kbd", {}, "1")),
      h("button", { class: "btn", onclick: () => answerCard("forgot") }, "忘了 ", h("kbd", {}, "2")),
      S.reveal ? null : h("button", { class: "btn", onclick: () => { S.reveal = true; renderReview(); } }, "看答案 ", h("kbd", {}, "空白鍵")))),
    h("div", { class: "rv-left" }, h("a", { href: "#", onclick: (e) => { e.preventDefault(); openEditor(n.id); } }, "開啟卡片")));
}

async function answerCard(outcome) {
  const n = S.due[S.reviewAt];
  if (!n) return;
  try {
    const r = await api("POST", `/api/recall/${n.id}`, { outcome, via: "kb-review" });
    const p = r.progress;
    toast(r.step.became_known ? `「${n.title}」已經記住了` : outcome === "remembered" ? `連續記得 ${p.streak} 次，下次 ${fmtDay(p.next_check_at)}` : `沒關係，${fmtDay(p.next_check_at)} 再問一次`);
    S.reviewAt++; S.reveal = false;
    $("cDue").textContent = Math.max(0, S.due.length - S.reviewAt) || "";
    renderReview();
  } catch (e) { fail(e); }
}

function reviewKeys(e) {
  if (S.view !== "review" || e.target.closest("input, textarea, select, dialog")) return;
  if (e.key === "1") answerCard("remembered");
  else if (e.key === "2") answerCard("forgot");
  else if (e.key === " " && !S.reveal) { e.preventDefault(); S.reveal = true; renderReview(); }
}

// ── Timeline ───────────────────────────────────────────
async function loadTimeline() {
  try {
    const { days } = await api("GET", `/api/timeline?days=${$("tlDays").value}`);
    const box = $("timeline");
    if (!days.length) { box.replaceChildren(h("div", { class: "empty" }, "這段期間沒有新增或修改的卡片。")); return; }
    const wd = "日一二三四五六";
    box.replaceChildren(...days.map((d) => {
      const date = new Date(`${d.day}T00:00:00`);
      const item = (it, upd) => h("button", { onclick: () => openEditor(it.id) }, h("span", { class: `ev${upd ? " upd" : ""}` }, upd ? "修改" : "新增"), it.title, h("span", { class: "tag kind" }, kindLabel(it.kind)), h("span", { class: "at" }, fmtTime(it.at)));
      return h("div", { class: "tl-day" },
        h("div", { class: "d" }, h("b", {}, `${date.getMonth() + 1}/${date.getDate()}`), `週${wd[date.getDay()]} · ${d.created.length} 新 ${d.updated.length} 改`),
        h("div", { class: "tl-items" }, ...d.created.map((x) => item(x, false)), ...d.updated.map((x) => item(x, true))));
    }));
  } catch (e) { fail(e); }
}

// ── Graph ──────────────────────────────────────────────
async function loadGraph() {
  const p = new URLSearchParams({ depth: $("gDepth").value, tags: $("gTags").checked });
  if (S.graphCenter) p.set("center", S.graphCenter);
  try {
    const g = await api("GET", `/api/graph?${p}`);
    const cards = g.nodes.filter((n) => n.kind !== "tag").length;
    $("graphInfo").textContent = `${S.graphCenter ? `以「${S.titles.get(S.graphCenter) ?? ""}」為中心 · ` : ""}${cards} 張卡片 · ${g.edges.filter((e) => e.origin !== "tag").length} 條連結`;
    requestAnimationFrame(() => Graph.set(g, S.graphCenter));
  } catch (e) { fail(e); }
}

boot();
