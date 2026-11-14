// KnowledgeBase web UI: card list with filters, card editor (Markdown + KaTeX
// preview, image paste/drop, link picker), review, timeline, graph, settings
// (對照表, reminders in the gear). Talks only to kb-server's JSON API (same origin).
// Routes (other apps link here, e.g. NeonDeck and the e-mail digest):
//   #/node/<id>  #/new  #/review  #/timeline  #/graph  #/graph/<id> (local graph)  #/settings (opens the gear)  #/cards

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
  const text = await res.text().catch(() => "");
  let data = null;
  try { data = text ? JSON.parse(text) : {}; } catch { data = null; }
  if (!res.ok) {
    // axum answers a body it cannot read (wrong type, number out of range) with 400/422 plain text.
    const e = new Error(data?.error ?? (text || res.statusText));
    e.status = res.status;
    e.alternative = data?.alternative ?? null;
    e.category = data?.category ?? (res.status === 422 || (res.status === 400 && !data) ? "format" : res.status === 404 ? "not_found" : res.status >= 500 ? "internal" : "invalid");
    throw e;
  }
  return data ?? {};
}

/** Short message at the bottom (the shell's). `action`: { label, run } adds a button (e.g. 復原). */
const toast = (msg, err = false, action = null) => LoreShell.toast(msg, { err, action });
const fail = (e) => toast(errorText(e), true);
const hasCJK = (s) => /[\u3400-\u9fff]/.test(s ?? "");
/** Drop a trailing raw (English) detail the server keeps for troubleshooting: 「…（Connection refused）」. */
const plainZh = (s) => (s ?? "").replace(/（([^（）]*)）$/, (m, inner) => (hasCJK(inner) ? m : ""));
/** Field names in serde / validation messages → labels on the settings page. */
const FIELD_LABEL = { smtp_port: "連接埠", smtp_host: "SMTP 主機", recent_days: "「最近學到」天數", max_due: "最多幾題", send_at: "寄送時間", times: "通知時間", grace_minutes: "等待分鐘", to: "收件人", from: "寄件人", user: "帳號", security: "加密", revision: "版本" };
/** A message in Chinese only: the server's Chinese messages as they are, known English ones translated. */
function errorText(e) {
  if (e instanceof TypeError || (!e.status && !e.category)) return "連不到知識庫伺服器，請確認 kb-server 有在執行。";
  const msg = e.message ?? "";
  switch (e.category) {
    case "conflict": return "這張卡在別處被修改過，請重新開啟後再存。";
    case "not_found": return "找不到這張卡，可能已經移到垃圾桶。";
    case "format": {
      const f = Object.keys(FIELD_LABEL).find((k) => new RegExp(`\\b${k}\\b`).test(msg));
      const range = /invalid value|out of range|too large/.test(msg);
      return `格式不對：${f ? `「${FIELD_LABEL[f]}」${range ? "的數值超出範圍" : "的內容不對"}` : "有欄位的內容不對"}`;
    }
    case "forbidden": return /household/.test(msg) ? "家庭空間的卡片只能連到同一個空間的卡片。" : "不允許這個操作。";
    case "smtp": return hasCJK(msg) ? `寄信失敗：${plainZh(msg)}` : "寄信失敗，請檢查 SMTP 設定。";
    case "notify": return "無法顯示 macOS 通知。";
  }
  if (hasCJK(msg)) return msg;
  if (/title is empty|term is empty/.test(msg)) return "請輸入標題。";
  if (/context is empty/.test(msg)) return "請貼上原文。";
  if (/unknown kind/.test(msg)) return "不認得這個種類，請到 kinds.toml 新增。";
  if (/unknown link type/.test(msg)) return "不認得這個連結類型。";
  if (/link target/.test(msg)) return "連結的目標不對（不能連到自己）。";
  if (/not in the trash/.test(msg)) return "這張卡已經復原了。";
  if (/already exists/.test(msg)) return "已經有同一張卡，無法復原。";
  if (/empty upload/.test(msg)) return "檔案是空的。";
  if (/asset|ext/.test(msg)) return "不支援這種檔案。";
  return e.status >= 500 ? "知識庫伺服器發生錯誤，請稍後再試。" : "資料不正確，無法處理。";
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
  due: [], reviewAt: 0, reveal: false, busy: false, round: { remembered: 0, forgot: 0, known: 0 },
  graphCenter: null,
};

const kindLabel = (id) => S.reg.kinds.find((k) => k.id === id)?.label ?? id;
// Domains carry the colour (--g1…--g12) everywhere: card list and graph. The slot is the
// domain's place in the registry (alphabetical), so a colour never moves when cards change.
const domainLabel = (id) => S.reg?.domains?.[id]?.label ?? id;
const domainSlot = (id) => { const i = Object.keys(S.reg?.domains ?? {}).indexOf(id); return i < 0 ? null : i % 12; };
const relLabel = (id) => S.reg.link_types.find((k) => k.id === id)?.label ?? id;
const spaceName = (id) => S.spaces.find((s) => s.id === id)?.name ?? id;

// ── Boot ───────────────────────────────────────────────
async function boot() {
  mountShell();
  paintIcons();
  try {
    const sp = await api("GET", "/api/spaces");
    S.member = sp.member; S.spaces = sp.spaces;
    $("memberChip").textContent = S.member;
    S.reg = await api("GET", "/api/registry");
  } catch (e) { fail(e); return; }
  api("GET", "/api/health").then((hl) => { S.readerUrl = hl.reader_url; }).catch(() => {});
  fillSelect($("fKind"), [["", "全部"], ...S.reg.kinds.map((k) => [k.id, k.label])]);
  fillSelect($("edKind"), S.reg.kinds.map((k) => [k.id, k.label]));
  fillSelect($("fSpace"), [["", "全部"], ...S.spaces.map((s) => [s.id, s.name])]);
  fillSelect($("edSpace"), S.spaces.map((s) => [s.id, `${s.name}${s.kind === "household" ? "（家庭）" : ""}`]));
  fillSelect($("pickerRel"), S.reg.link_types.map((l) => [l.id, l.label]));
  renderTags();
  renderSpacesFoot();
  loadFilters();
  try { await refreshNames(); } catch (e) { fail(e); }
  wire();
  if (!route()) setView("cards");
  window.addEventListener("hashchange", route);
  refreshDue();
  refreshPending();
  setInterval(refreshPending, 20000);
  window.addEventListener("focus", refreshPending);
}

// ── Routes ─────────────────────────────────────────────
const VIEWS = ["cards", "review", "pending", "timeline", "graph"];
/** Show what the URL hash names; false when it names nothing. */
function route() {
  const m = location.hash.match(/^#\/(\w+)(?:\/([\w-]+))?/);
  if (!m) return false;
  // Leaving the editor with unsaved changes: ask first; on "no" put the address back.
  const target = m[1] === "node" ? `#/node/${m[2]}` : m[1] === "new" ? "#/new" : null;
  if (S.view === "editor" && edHash() !== location.hash && isDirty()) {
    // Unsaved changes: put the address back, ask, and go on to where it was headed.
    const wanted = location.hash;
    history.replaceState(null, "", edHash());
    askLeave().then((ok) => { if (ok) location.hash = wanted; });
    return true;
  }
  if (m[1] === "settings") { LoreShell.settings(); setHash(S.view === "editor" ? edHash() : `#/${S.view}`); return true; }
  if (m[1] === "node" && m[2]) { if (target !== edHash() || S.view !== "editor") openEditor(m[2]); return true; }
  if (m[1] === "new") { if (S.view !== "editor" || S.ed?.id || S.ed?.pending) openEditor(null); return true; }
  if (m[1] === "pending" && m[2]) { if (S.view !== "editor" || S.ed?.pending?.id !== m[2]) openPending(m[2]); return true; }
  if (m[1] === "graph") { S.graphCenter = m[2] ?? null; setView("graph"); return true; }
  if (VIEWS.includes(m[1])) { setView(m[1]); return true; }
  return false;
}
const setHash = (h) => { if (location.hash !== h) history.replaceState(null, "", h); };

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
    refreshDue(); refreshPending();
  } catch (e) { fail(e); }
}

// ── Views ──────────────────────────────────────────────
function setView(v) {
  if (S.view === "editor" && v !== "editor" && isDirty()) {
    setHash(edHash());
    askLeave().then((ok) => { if (ok) showView(v); });
    return false;
  }
  return showView(v);
}
function showView(v) {
  S.view = v;
  document.querySelectorAll(".view").forEach((s) => (s.hidden = s.dataset.view !== v));
  const navFor = v === "editor" ? (S.ed?.pending ? "pending" : "cards") : v;
  document.querySelectorAll("#views button").forEach((b) => b.toggleAttribute("aria-current", b.dataset.view === navFor));
  document.querySelectorAll("#views button[aria-current]").forEach((b) => b.setAttribute("aria-current", "page"));
  $("filters").hidden = !(v === "cards");
  shell.drawer("left", false);
  paintStatus();
  if (v !== "graph") Graph.hide();
  if (v === "cards") loadCards();
  if (v === "review") loadReview();
  if (v === "pending") loadPending();
  if (v === "timeline") loadTimeline();
  if (v === "graph") loadGraph();
  if (VIEWS.includes(v)) setHash(v === "graph" && S.graphCenter ? `#/graph/${S.graphCenter}` : `#/${v}`);
  return true;
}

function wire() {
  $("views").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-view]");
    if (!b) return;
    if (b.dataset.view === "graph" && S.view === "editor" && S.ed?.id) S.graphCenter = S.ed.id;
    setView(b.dataset.view);
  });
  for (const id of FILTERS) $(id).addEventListener("change", () => { saveFilters(); loadCards(); });
  $("fDateField").addEventListener("click", (e) => {
    const b = e.target.closest("button"); if (!b) return;
    setDateField(b.dataset.v);
    saveFilters(); loadCards();
  });
  let qt;
  $("q").addEventListener("input", () => { clearTimeout(qt); qt = setTimeout(() => { if (S.view !== "cards") setView("cards"); else loadCards(); }, 200); });
  // 外觀 is the shell's (the theme all LoreForge apps share)
  document.addEventListener("lore-theme", () => { if (S.view === "graph") Graph.draw(); });
  wireEditor();
  $("tlDays").onchange = loadTimeline;
  Graph.init($("graphWrap"), {
    open: (id) => openEditor(id),
    create: (title) => newFromTitle(title),
    newCard: () => openEditor(null),
    onCenter: (id) => { S.graphCenter = id; setHash(id ? `#/graph/${id}` : "#/graph"); },
    info: (t) => ($("graphInfo").textContent = t),
    kindLabel, relLabel, spaceName, domainLabel, domainSlot, fail, toast,
    tagLabel: (t) => S.reg?.tags?.[t]?.label ?? t.split("/").pop(),
    domains: () => Object.keys(S.reg?.domains ?? {}),
  });
  wireNotation();
  document.addEventListener("keydown", reviewKeys);
}

// ── Card list ──────────────────────────────────────────
const FILTERS = ["fTime", "fState", "fKind", "fTag", "fSpace", "fSort"];
function setDateField(v) {
  S.dateField = v === "updated" ? "updated" : "created";
  $("fDateField").querySelectorAll("button").forEach((x) => x.setAttribute("aria-pressed", String(x.dataset.v === S.dateField)));
}
/** Filters survive a reload (this browser only). */
function saveFilters() {
  const f = Object.fromEntries(FILTERS.map((id) => [id, $(id).value]));
  f.dateField = S.dateField;
  try { localStorage.setItem("kb.filters", JSON.stringify(f)); } catch {}
}
function loadFilters() {
  let f = null;
  try { f = JSON.parse(localStorage.getItem("kb.filters") ?? "null"); } catch {}
  if (!f || typeof f !== "object") return;
  for (const id of FILTERS) {
    const sel = $(id), v = f[id];
    if (typeof v === "string" && [...sel.options].some((o) => o.value === v)) sel.value = v;
  }
  setDateField(f.dateField);
}
function clearFilters() {
  for (const id of FILTERS) $(id).value = id === "fSort" ? "updated" : "";
  $("q").value = "";
  saveFilters(); loadCards();
}

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
      const none = !S.spaces.some((s) => s.cards > 0);
      list.replaceChildren(none
        ? h("div", { class: "empty" },
            h("b", {}, "還沒有知識卡"),
            h("button", { class: "btn primary", type: "button", onclick: () => openEditor(null) }, icon("plus"), "新增第一張卡"),
            h("span", { class: "muted small" }, "小訣竅：在內文寫 [[標題]] 就會連到其他卡片，圖譜會跟著長出來。"))
        : h("div", { class: "empty" },
            h("b", {}, "沒有符合條件的卡片"),
            h("button", { class: "btn", type: "button", onclick: clearFilters }, "清除篩選")));
      return;
    }
    list.replaceChildren(...cardIndex(nodes));
  } catch (e) { fail(e); }
}

// The card list is an index: what I know, grouped by domain, one line per card. Cards that
// need attention (learning, stale, never reviewed) come first; the summary shows on hover.
const domainOf = (n) => n.domains?.[0] ?? null;
const dcStyle = (d) => { const i = d == null ? null : domainSlot(d); return i == null ? null : `--dc: var(--g${i + 1})`; };
function cardIndex(nodes) {
  const out = [];
  const due = nodes.filter((n) => (n.state ?? "none") !== "known");
  if (due.length && due.length < nodes.length) out.push(idxSection("待複習", null, due, true));
  const groups = new Map();
  for (const n of nodes) { const d = domainOf(n); (groups.get(d) ?? groups.set(d, []).get(d)).push(n); }
  const order = [...groups.entries()].sort((a, b) => (a[0] == null) - (b[0] == null) || b[1].length - a[1].length);
  for (const [d, list] of order) out.push(idxSection(d == null ? "未分類" : domainLabel(d), d, list, false));
  return out;
}
function idxSection(label, d, list, attention) {
  return h("section", { class: `idx${attention ? " due" : ""}`, style: dcStyle(d) },
    h("h3", {}, h("i", {}), label, h("span", {}, list.length)),
    h("ul", {}, list.map((n) => h("li", {}, idxRow(n, attention)))));
}
function idxRow(n, attention) {
  const st = n.state ?? "none";
  const b = h("button", { type: "button", onclick: () => { unpeek(); openEditor(n.id); } },
    attention ? h("i", { class: `st-${st}` }) : null,
    h("span", { class: "t" }, n.title),
    h("span", { class: "k" }, kindLabel(n.kind)));
  b.addEventListener("pointerenter", (e) => e.pointerType === "mouse" && peek(n, b));
  b.addEventListener("pointerleave", unpeek);
  b.addEventListener("focus", () => peek(n, b));
  b.addEventListener("blur", unpeek);
  return b;
}
let peekEl = null;
function peek(n, anchor) {
  if (!peekEl) { peekEl = h("div", { class: "peek", role: "tooltip" }); document.body.append(peekEl); }
  const d = domainOf(n), st = n.state ?? "none";
  peekEl.setAttribute("style", dcStyle(d) ?? "");
  peekEl.replaceChildren(
    h("b", {}, n.title),
    n.aliases.length ? h("div", { class: "al" }, n.aliases.join(" · ")) : null,
    n.summary ? h("p", {}, n.summary) : null,
    h("div", { class: "m" }, d ? h("span", { class: "dm" }, domainLabel(d)) : null, h("span", {}, kindLabel(n.kind)), h("span", {}, STATE_LABEL[st]),
      S.spaces.length > 1 ? h("span", {}, spaceName(n.space)) : null));
  peekEl.hidden = false;
  const r = anchor.getBoundingClientRect(), w = peekEl.offsetWidth, ph = peekEl.offsetHeight;
  const x = r.right + 12 + w < innerWidth ? r.right + 12 : Math.max(8, r.left - 12 - w);
  peekEl.style.left = `${x}px`;
  peekEl.style.top = `${Math.max(8, Math.min(r.top - 6, innerHeight - ph - 8))}px`;
}
function unpeek() { if (peekEl) peekEl.hidden = true; }

// ── Editor ─────────────────────────────────────────────
// Unsaved changes: the form is compared with a snapshot taken when the card was opened or
// saved. While it differs, the head shows 「未儲存」, a draft is kept in localStorage
// (`kb.draft.<id|new>`, offered back next time) and leaving asks first.
const draftKey = (id) => (id == null && S.ed?.pending ? `kb.draft.${S.ed.pending.id}` : `kb.draft.${id ?? "new"}`);
const edHash = () => (S.ed?.pending ? `#/pending/${S.ed.pending.id}` : S.ed?.id ? `#/node/${S.ed.id}` : "#/new");
const edState = () => JSON.stringify({ ...fieldsFromForm(), space: $("edSpace").value });
const isDirty = () => S.view === "editor" && !!S.ed && !S.ed.leaving && S.ed.snap != null && edState() !== S.ed.snap;
function markClean() {
  if (!S.ed) return;
  S.ed.snap = edState();
  try { localStorage.removeItem(draftKey(S.ed.id)); } catch {}
  updateDirty();
}
function updateDirty() {
  const d = isDirty();
  paintStatus();
  if (d) try { localStorage.setItem(draftKey(S.ed.id), JSON.stringify({ at: new Date().toISOString(), revision: S.ed.revision, state: edState() })); } catch {}
}
/** Ask before leaving unsaved changes (next to the status chip); yes = leave, the draft stays in localStorage. */
async function askLeave() {
  const ok = await LoreShell.ask(document.querySelector(".fstate:not([hidden])") ?? $("edBack"),
    ["這張卡有未儲存的修改，確定要離開嗎？", "修改已暫存成草稿，下次打開這張卡可以還原"],
    [{ label: "留下", value: false }, { label: "離開", value: true, primary: true }]);
  if (ok !== true) return false;
  if (S.ed) S.ed.leaving = true;
  return true;
}
/** The one status chip, only while a card is open: 未儲存 / 儲存中 / 已儲存. */
function paintStatus() {
  if (!shell) return;
  if (S.view !== "editor" || !S.ed) shell.setStatus(null);
  else if (S.ed.saving) shell.setStatus("saving");
  else if (isDirty()) shell.setStatus("dirty");
  else if (S.ed.id) shell.setStatus("saved");
  else shell.setStatus(null);
}
window.addEventListener("beforeunload", (e) => { if (isDirty()) { e.preventDefault(); e.returnValue = ""; } });

function offerDraft() {
  let d = null;
  try { d = JSON.parse(localStorage.getItem(draftKey(S.ed.id)) ?? "null"); } catch {}
  const bar = $("edDraftBar");
  if (!d?.state || d.state === S.ed.snap) { bar.hidden = true; return; }
  const when = new Date(d.at);
  bar.replaceChildren(
    h("span", {}, `有 ${when.getMonth() + 1}/${when.getDate()} ${fmtTime(d.at)} 沒存的草稿${d.revision != null && d.revision !== S.ed.revision ? "（之後卡片又改過）" : ""}`),
    h("button", { type: "button", class: "btn small", onclick: () => { applyState(JSON.parse(d.state)); bar.hidden = true; updateDirty(); } }, "還原草稿"),
    h("button", { type: "button", class: "btn small ghost", onclick: () => { try { localStorage.removeItem(draftKey(S.ed.id)); } catch {} bar.hidden = true; } }, "捨棄"));
  bar.hidden = false;
}
function applyState(st) {
  $("edTitle").value = st.title ?? ""; $("edKind").value = st.kind ?? "concept";
  $("edAliases").value = (st.aliases ?? []).join(", "); $("edTags").value = (st.tags ?? []).join(", "); $("edDomains").value = (st.domains ?? []).join(", ");
  $("edSummary").value = st.summary ?? ""; $("edBody").value = st.body ?? "";
  if (!S.ed.id && st.space) $("edSpace").value = st.space;
  S.ed.images = st.images ?? []; S.ed.links = st.links ?? [];
  renderEdLists(); renderPreview();
}

function wireEditor() {
  $("edBack").onclick = () => setView(S.ed?.pending ? "pending" : "cards");
  $("edDiscard").onclick = discardPending;
  $("edSave").onclick = saveCard;
  $("edForm").addEventListener("submit", (e) => { e.preventDefault(); saveCard(); });
  let pt;
  $("edForm").addEventListener("input", () => { clearTimeout(pt); pt = setTimeout(() => { renderPreview(); updateDirty(); }, 120); });
  // Enter in the title saves (not while an IME is composing); ⌘S is the shell's.
  $("edTitle").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing && e.keyCode !== 229) { e.preventDefault(); saveCard(); }
  });
  $("edSpace").onchange = renderPreview;
  $("edDelete").onclick = deleteCard;
  $("edShare").onclick = shareCard;
  $("edGraph").onclick = () => { if (S.ed?.id) { S.graphCenter = S.ed.id; setView("graph"); } };
  // Images pasted or dropped into the body go in at the caret as ![](…) (and into the card's image list).
  const body = $("edBody");
  body.addEventListener("paste", (e) => {
    const files = [...(e.clipboardData?.files ?? [])].filter((f) => f.type.startsWith("image/"));
    if (files.length) { e.preventDefault(); files.forEach((f) => uploadImage(f, true)); }
  });
  body.addEventListener("dragover", (e) => { if ([...e.dataTransfer.types].includes("Files")) { e.preventDefault(); body.classList.add("drop"); } });
  body.addEventListener("dragleave", () => body.classList.remove("drop"));
  body.addEventListener("drop", (e) => {
    body.classList.remove("drop");
    const files = [...e.dataTransfer.files];
    if (files.length) { e.preventDefault(); body.focus(); files.forEach((f) => uploadImage(f, true)); }
  });
  $("edAddImage").onclick = () => $("edImageFile").click();
  $("edImageFile").onchange = (e) => { [...e.target.files].forEach((f) => uploadImage(f)); e.target.value = ""; };
  $("edInsertLink").onclick = () => pick("wiki");
  $("edAddLink").onclick = () => pick("link");
  $("draftGo").onclick = async (e) => {
    e.preventDefault();
    runDraft();
  };
  async function runDraft(engine) {
    const go = $("draftGo"), err = $("draftErr");
    err.hidden = true; go.disabled = true; go.textContent = "起草中…";
    try {
      const { draft, assistant } = await api("POST", "/api/draft", { term: $("draftTerm").value, context: $("draftCtx").value, kind: $("edKind").value, engine });
      if (!$("edTitle").value.trim()) $("edTitle").value = draft.title ?? "";
      if (!$("edSummary").value.trim()) $("edSummary").value = draft.summary ?? "";
      if (!$("edBody").value.trim()) $("edBody").value = draft.body ?? "";
      $("draftDlg").close();
      renderPreview(); updateDirty();
      toast(`草稿已填入（${assistant}）`);
    } catch (er) { // the dialog stays open with the source text
      err.textContent = errorText(er); err.hidden = false;
      if (er.alternative) toast(errorText(er), true, { label: er.alternative.label, run: () => runDraft(er.alternative.engine) });
    }
    go.disabled = false; go.replaceChildren(icon("spark"), "起草");
  }
  let pq;
  $("pickerQ").addEventListener("input", () => { clearTimeout(pq); pq = setTimeout(searchPicker, 150); });
}

async function openEditor(id) {
  if (S.view === "editor" && edHash() !== (id ? `#/node/${id}` : "#/new") && isDirty() && !(await askLeave())) { setHash(edHash()); return; }
  if (S.view === "editor" && S.ed) S.ed.leaving = true;
  pendingMode(null);
  setView("editor");
  setHash(id ? `#/node/${id}` : "#/new");
  const isNew = !id;
  $("edDraftBar").hidden = true;
  $("edDelete").hidden = isNew; $("edGraph").hidden = isNew; $("edShareWrap").hidden = true;
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
    S.ed.snap = edState(); offerDraft(); paintStatus();
    $("edTitle").focus();
    return;
  }
  try {
    const v = await api("GET", `/api/nodes/${id}`);
    const n = v.node;
    S.ed = { id: n.id, space: v.space, revision: n.revision, images: n.images ?? [], links: n.links ?? [], snap: null };
    $("edTitle").value = n.title; $("edAliases").value = n.aliases.join(", ");
    $("edKind").value = n.kind; $("edSpace").value = v.space;
    $("edTags").value = n.tags.join(", "); $("edDomains").value = n.domains.join(", ");
    $("edSummary").value = n.summary; $("edBody").value = n.body;
    $("edMeta").textContent = `建立 ${n.created_at.slice(0, 10)} · 修改 ${n.updated_at.slice(0, 10)} · 第 ${n.revision} 版`;
    const others = S.spaces.filter((s) => s.id !== v.space);
    if (others.length) {
      fillSelect($("edShareTo"), others.map((s) => [s.id, s.name]));
      $("edShareWrap").hidden = false;
    }
    renderEdLists(); renderPreview();
    renderSide(v);
    S.ed.snap = edState(); offerDraft(); paintStatus();
  } catch (e) { fail(e); S.ed = null; setView("cards"); }
}

function renderSide(v) {
  // One row per card even when it links here twice (e.g. 先備知識 + 內文提到): one chip per relation.
  const by = new Map();
  for (const b of v.backlinks) {
    const rels = by.get(b.from) ?? by.set(b.from, []).get(b.from);
    const r = b.rel === "mentions" ? "內文提到" : relLabel(b.rel);
    if (!rels.includes(r)) rels.push(r);
  }
  const back = [...by].map(([id, rels]) => h("li", {}, h("a", { href: `#/node/${id}` }, S.titles.get(id) ?? id), ...rels.map((r) => h("span", { class: "rel-chip" }, r))));
  const srcs = (v.node.sources ?? []).map(sourceItem);
  $("edBacklinks").replaceChildren(...(srcs.length ? [h("h4", {}, "來源"), h("ul", {}, srcs)] : []), ...(back.length || v.unresolved.length ? [
    back.length ? h("h4", {}, `反向連結（${back.length}）`) : null,
    back.length ? h("ul", {}, back) : null,
    v.unresolved.length ? h("h4", {}, "尚未建立的 [[連結]]") : null,
    v.unresolved.length ? h("ul", {}, v.unresolved.map((t) => h("li", {}, h("button", { type: "button", class: "linkish", onclick: () => newFromTitle(t) }, `＋ ${t}`)))) : null,
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

/** A card source. `pdf` sources (LoreReader: ref = the PDF's sha256, where = "p.<n>")
 *  link to LoreReader's /open, at `reader_url` from /api/health. */
function sourceItem(src) {
  const label = [src.title || src.ref, src.where].filter(Boolean).join(" · ");
  if (src.type === "pdf" && /^[0-9a-f]{64}$/.test(src.ref ?? "")) {
    const page = /(\d+)/.exec(src.where ?? "")?.[1] ?? "1";
    const base = S.readerUrl ?? "http://127.0.0.1:7890";
    return h("li", {}, h("a", { href: `${base}/open?sha=${src.ref}&page=${page}`, target: "_blank", rel: "noopener", title: "在 LoreReader 開啟" }, label), h("span", { class: "rel-chip" }, "PDF"));
  }
  return h("li", { class: "muted" }, src.type ? `${src.type}：${label}` : label);
}

function newFromTitle(t) {
  openEditor(null).then(() => { if (S.view === "editor" && !S.ed?.id) { $("edTitle").value = t; renderPreview(); updateDirty(); } });
}

function renderEdLists() {
  const space = $("edSpace").value;
  $("edImages").replaceChildren(...S.ed.images.map((img, i) => h("figure", {},
    h("img", { src: assetUrl(space, img), alt: img.caption ?? "" }),
    h("input", { value: img.caption ?? "", placeholder: "圖說", oninput: (e) => { img.caption = e.target.value; renderPreview(); } }),
    h("input", { value: img.credit ?? "", placeholder: "來源", oninput: (e) => { img.credit = e.target.value; } }),
    iconButton("x", "移除圖片", () => { S.ed.images.splice(i, 1); renderEdLists(); renderPreview(); updateDirty(); }))));
  $("edLinks").replaceChildren(...S.ed.links.map((l, i) => h("li", {},
    h("span", { class: "rel" }, relLabel(l.rel)), S.titles.get(l.to) ?? l.to,
    iconButton("x", "移除連結", () => { S.ed.links.splice(i, 1); renderEdLists(); updateDirty(); }))));
  if (!S.ed.links.length) $("edLinks").append(h("li", { class: "muted" }, "沒有明確連結（內文的 [[標題]] 會自動成為「提到」連結）"));
}

/** Card images as figures, except the ones the body already shows inline (pasted at the caret). */
const figures = (space, images, body) => images.filter((img) => !body.includes(assetUrl(space, img)))
  .map((img) => `<figure><img src="${assetUrl(space, img)}" alt=""><figcaption>${MD.esc(img.caption ?? "")}${img.credit ? ` · ${MD.esc(img.credit)}` : ""}</figcaption></figure>`);
/** [[wikilinks]] become real links: existing cards → #/node/<id>, missing ones → a new card. */
function wireWikilinks(root) {
  root.querySelectorAll("a.wikilink").forEach((w) => {
    const id = S.names.get(w.dataset.target.toLowerCase());
    if (id) { w.href = `#/node/${id}`; w.title = `開啟「${w.dataset.target}」`; }
    else { w.title = `建立「${w.dataset.target}」`; w.onclick = (e) => { e.preventDefault(); newFromTitle(w.dataset.target); }; }
  });
}

function renderPreview() {
  const space = $("edSpace").value;
  const title = $("edTitle").value.trim();
  const aliases = splitList($("edAliases").value);
  const resolve = (t) => S.names.has(t.toLowerCase());
  const body = $("edBody").value;
  const html = [
    `<h1>${MD.esc(title || "（未命名）")}</h1>`,
    aliases.length ? `<div class="p-al">${MD.esc(aliases.join(" · "))}</div>` : "",
    $("edSummary").value.trim() ? `<div class="p-sum">${MD.render($("edSummary").value, { resolve })}</div>` : "",
    MD.render(body, { resolve }),
    ...figures(space, S.ed?.images ?? [], body),
  ].join("");
  const p = $("edPreview");
  p.innerHTML = html;
  wireWikilinks(p);
}

async function uploadImage(file, atCaret = false) {
  const space = $("edSpace").value;
  const ext = (file.name.match(/\.([a-z0-9]+)$/i)?.[1] ?? file.type.split("/")[1] ?? "png").toLowerCase();
  const ta = $("edBody"), at = [ta.selectionStart, ta.selectionEnd];
  try {
    const r = await api("POST", `/api/assets?space=${encodeURIComponent(space)}&ext=${encodeURIComponent(ext)}`, undefined, file);
    const img = { asset: r.asset, ext: r.ext, caption: "", credit: "", marks: [] };
    if (!S.ed.images.some((x) => x.asset === img.asset)) S.ed.images.push(img);
    if (atCaret) {
      const v = ta.value, before = v.slice(0, at[0]);
      const ins = `${before && !before.endsWith("\n") ? "\n" : ""}![](${assetUrl(space, img)})\n`;
      ta.setRangeText(ins, at[0], at[1], "end");
    }
    renderEdLists(); renderPreview(); updateDirty();
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
  if (!S.ed || S.ed.saving) return;
  if (S.ed.pending) return acceptPending();
  if (!$("edTitle").value.trim()) { toast("請輸入標題", true); $("edTitle").focus(); return; }
  S.ed.saving = true; $("edSave").disabled = true; paintStatus();
  try {
    let id;
    const wasNew = !S.ed.id;
    if (S.ed.id) {
      const r = await api("PUT", `/api/nodes/${S.ed.id}`, { revision: S.ed.revision, ...fieldsFromForm() });
      id = r.node.id;
    } else {
      const r = await api("POST", "/api/nodes", { space: $("edSpace").value, via: "kb", ...fieldsFromForm() });
      id = r.node.id;
    }
    markClean();
    if (wasNew) try { localStorage.removeItem(draftKey(null)); } catch {}
    toast("已儲存");
    await refreshMeta();
    S.ed.leaving = true;
    await openEditor(id);
  } catch (e) { fail(e); }
  finally { S.ed && (S.ed.saving = false); $("edSave").disabled = false; paintStatus(); }
}

/** Delete = move to the trash; the toast offers 復原 (no confirm dialog). */
async function deleteCard() {
  if (!S.ed?.id) return;
  const id = S.ed.id, title = $("edTitle").value;
  try {
    await api("DELETE", `/api/nodes/${id}`);
    try { localStorage.removeItem(draftKey(id)); } catch {}
    S.ed.leaving = true;
    await refreshMeta(); setView("cards");
    toast(`已把「${title}」移到垃圾桶`, false, { label: "復原", run: () => restoreCard(id, title) });
  } catch (e) { fail(e); }
}

async function restoreCard(id, title) {
  try {
    await api("POST", `/api/nodes/${id}/restore`);
    toast(`已復原「${title}」`);
    await refreshMeta();
    if (S.view === "cards") loadCards();
  } catch (e) { fail(e); }
}

async function shareCard() {
  const to = $("edShareTo").value;
  const target = S.spaces.find((s) => s.id === to);
  const lines = [`把「${$("edTitle").value}」分享（複製）到「${target?.name ?? to}」？`];
  if (target?.kind === "household") lines.push("會複製一份到家庭空間（新的 id，圖片轉成 webp 存進共用 repo）；指向個人卡片的連結不會帶過去。");
  if (!(await LoreShell.ask($("edShare"), lines, [{ label: "取消", value: null }, { label: "分享", value: true, primary: true }]))) return;
  try { const r = await api("POST", `/api/nodes/${S.ed.id}/share`, { to_space: to }); toast(`已分享到 ${target?.name ?? to}`); await refreshMeta(); S.ed.leaving = true; openEditor(r.node.id); }
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
  renderPreview(); updateDirty();
}

// ── Review ─────────────────────────────────────────────
async function refreshDue() {
  try { const { nodes } = await api("GET", "/api/recall/due?limit=200"); S.due = nodes; $("cDue").textContent = nodes.length || ""; }
  catch {}
}

async function loadReview() {
  await refreshDue();
  S.reviewAt = 0; S.reveal = false; S.busy = false;
  S.round = { remembered: 0, forgot: 0, known: 0 };
  renderReview();
}

let reviewSeq = 0;
async function renderReview() {
  const seq = ++reviewSeq;
  const box = $("review");
  const n = S.due[S.reviewAt];
  $("reviewInfo").textContent = S.due.length && n ? `${S.reviewAt + 1} / ${S.due.length}` : "";
  if (!n) { box.replaceChildren(reviewDone()); return; }
  const answer = h("div", { class: "answer paper" });
  if (S.reveal) {
    try {
      const v = await api("GET", `/api/nodes/${n.id}`);
      if (seq !== reviewSeq) return;
      answer.innerHTML = (v.node.summary ? `<div class="p-sum">${MD.render(v.node.summary)}</div>` : "") + MD.render(v.node.body, { resolve: (t) => S.names.has(t.toLowerCase()) })
        + figures(v.space, v.node.images ?? [], v.node.body).join("");
      wireWikilinks(answer);
    } catch (e) { fail(e); }
  }
  const dis = S.busy || null;
  box.replaceChildren(h("div", { class: "rv-card" },
    h("div", { class: "q" }, `${kindLabel(n.kind)} · ${STATE_LABEL[n.state ?? "none"]}${n.streak ? ` · 已連續記得 ${n.streak} 次` : ""}`),
    h("h3", {}, `還記得「${n.title}」嗎？`),
    n.aliases.length ? h("div", { class: "al" }, n.aliases.join(" · ")) : null,
    S.reveal ? answer : null,
    h("div", { class: "rv-actions" },
      h("button", { class: "btn primary", disabled: dis, onclick: () => answerCard("remembered") }, "記得 ", h("kbd", {}, "1")),
      h("button", { class: "btn", disabled: dis, onclick: () => answerCard("forgot") }, "忘了 ", h("kbd", {}, "2")),
      S.reveal ? null : h("button", { class: "btn", disabled: dis, onclick: () => { S.reveal = true; renderReview(); } }, "看答案 ", h("kbd", {}, "空白鍵")))),
    h("div", { class: "rv-left" }, h("a", { href: `#/node/${n.id}` }, "開啟卡片")));
}

/** End of a round (or nothing due): what happened, a small cheer, where to go next. */
function reviewDone() {
  const r = S.round, done = r.remembered + r.forgot;
  const next = (label, ico, fn, primary) => h("button", { type: "button", class: `btn${primary ? " primary" : ""}`, onclick: fn }, icon(ico), label);
  if (!done) {
    return h("div", { class: "empty rv-done" },
      h("b", {}, S.due.length ? "這一輪複習完了" : "目前沒有到期的卡片"),
      h("span", { class: "muted small" }, "新卡片建立後隔天會出現在這裡。"),
      h("div", { class: "rv-next" }, next("新增知識卡", "plus", () => openEditor(null), true), next("看圖譜", "graph", () => setView("graph"))));
  }
  return h("div", { class: "rv-done done" },
    h("div", { class: "rv-cheer", "aria-hidden": "true" }, icon("check")),
    h("h3", {}, "這一輪複習完了！"),
    h("div", { class: "rv-sum" },
      h("div", {}, h("b", {}, String(r.remembered)), h("span", {}, "記得")),
      h("div", {}, h("b", {}, String(r.forgot)), h("span", {}, "再練一次")),
      r.known ? h("div", {}, h("b", {}, String(r.known)), h("span", {}, "新記住")) : null),
    h("div", { class: "rv-next" },
      next("看今天學了什麼", "calendar", () => setView("timeline"), true),
      next("新增知識卡", "plus", () => openEditor(null)),
      next("回到知識卡", "cards", () => setView("cards"))));
}

async function answerCard(outcome) {
  const n = S.due[S.reviewAt];
  if (!n || S.busy) return;           // one answer per card, even with a fast double key or click
  S.busy = true;
  document.querySelectorAll("#review .rv-actions button").forEach((b) => (b.disabled = true));
  try {
    const r = await api("POST", `/api/recall/${n.id}`, { outcome, via: "kb-review" });
    const p = r.progress;
    toast(r.step.became_known ? `「${n.title}」已經記住了` : outcome === "remembered" ? `連續記得 ${p.streak} 次，下次 ${fmtDay(p.next_check_at)}` : `沒關係，${fmtDay(p.next_check_at)} 再問一次`);
    S.round[outcome === "remembered" ? "remembered" : "forgot"]++;
    if (r.step.became_known) S.round.known++;
    S.reviewAt++; S.reveal = false;
    $("cDue").textContent = Math.max(0, S.due.length - S.reviewAt) || "";
  } catch (e) { fail(e); }
  finally { S.busy = false; renderReview(); }
}

function reviewKeys(e) {
  if (S.view !== "review" || e.metaKey || e.ctrlKey || e.altKey || e.target.closest?.("input, textarea, select, dialog")) return;
  if (e.repeat) { if (e.key === " ") e.preventDefault(); return; }   // a held key answers once
  if (e.key === "1") answerCard("remembered");
  else if (e.key === "2") answerCard("forgot");
  // Space on a focused button / link is that control's own click.
  else if (e.key === " " && !e.target.closest?.("button, a") && !S.reveal && !S.busy) { e.preventDefault(); S.reveal = true; renderReview(); }
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
function loadGraph() {
  Graph.show(S.graphCenter);
}

// ── The shell: top bar, gear, 對照表, 匯出 ─────────────
let shell = null;
function mountShell() {
  const inEditor = () => S.view === "editor" && !!S.ed;
  shell = LoreShell.mount({
    app: "kb", resize: { left: "#left", defaults: { left: 248 } }, brand: "KNOWLEDGE/BASE", host: "header.top",
    home: () => setView("cards"),
    status: { onClick: () => saveCard() },
    primary: [{ id: "actNew", icon: "plus", label: "新增知識卡", run: () => openEditor(null) }],
    ai: { id: "actDraft", icon: "spark", label: "起草摘要", run: openDraft },
    notation: openNotation,
    export: [
      { icon: "download", label: "匯出 Markdown", run: exportMarkdown, get disabled() { return !inEditor(); } },
      { icon: "printer", label: "列印", run: () => window.print(), get disabled() { return !inEditor(); } },
    ],
    drawers: { left: "#left", leftLabel: "導覽", leftIcon: "panelLeft" },
    search: () => { $("q").focus(); $("q").select(); },
    save: () => { if (inEditor()) saveCard(); },
    general: buildGeneral,
    settingsTabs: [{ id: "notify", label: "提醒", build: buildNotify }, { id: "email", label: "郵件", build: buildEmail }],
    keys: [{ label: "複習：記得", keys: "1", group: "其他" }, { label: "複習：忘了", keys: "2", group: "其他" }, { label: "複習：看答案", keys: "Space", group: "其他" }],
  });
  // 搜尋 and the current user sit in the lead slot
  shell.slots.lead.append(
    h("label", { class: "search" }, h("span", { "data-icon": "search" }), h("input", { id: "q", type: "search", placeholder: "搜尋標題、別名、摘要、內文…", autocomplete: "off", "aria-label": "搜尋" })),
    h("span", { class: "chip", id: "memberChip", title: "目前使用者" }));
  paintIcons(shell.slots.lead);
}

/** 起草摘要: the draft dialog for the card being edited (opens a new card first when none is). */
async function openDraft() {
  if (!(S.view === "editor" && S.ed)) { await openEditor(null); if (S.view !== "editor") return; }
  $("draftTerm").value = $("edTitle").value;
  $("draftDlg").showModal();
}

// 匯出: the card in the editor as Markdown; 列印 prints its sheet (print rules in styles.css).
function cardMarkdown() {
  const f = fieldsFromForm();
  const out = [`# ${f.title.trim() || "未命名"}`];
  if (f.aliases.length) out.push("", `別名：${f.aliases.join("、")}`);
  if (f.summary.trim()) out.push("", ...f.summary.trim().split("\n").map((l) => `> ${l}`));
  if (f.body.trim()) out.push("", f.body.trim());
  return `${out.join("\n")}\n`;
}
function exportMarkdown() {
  if (!S.ed) return;
  const name = ($("edTitle").value.trim() || "未命名").replace(/[\\/:*?"<>|]+/g, "-");
  const url = URL.createObjectURL(new Blob([cardMarkdown()], { type: "text/markdown;charset=utf-8" }));
  const a = h("a", { href: url, download: `${name}.md` });
  document.body.append(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
}

// ── 對照表: the global registry (private/shared/notation.json) in the shared <notation-panel> ──
const NT = { timer: 0 };
async function openNotation() {
  try { $("ntPanel").notation = await api("GET", "/api/notation"); } catch (e) { fail(e); return; }
  $("ntState").hidden = true;
  $("dlgNotation").showModal();
}
function wireNotation() {
  const p = $("ntPanel"), st = $("ntState");
  const save = () => {
    clearTimeout(NT.timer);
    st.hidden = false; st.textContent = "未儲存";
    NT.timer = setTimeout(async () => {
      try { await api("PUT", "/api/notation", p.notation); st.textContent = "已儲存"; }
      catch (e) { fail(e); }
    }, 500);
  };
  for (const ev of ["notation-change", "notation-add", "notation-delete"]) p.addEventListener(ev, save);
  $("dlgNotation").querySelector(".close").onclick = () => $("dlgNotation").close();
}

// ── Gear: 一般 (today's numbers, 重建索引), 提醒, 郵件 ─────
const SEC_PORT = { starttls: 587, tls: 465 };
const NUM_RANGE = { ePort: [1, 65535], eDays: [1, 60], eMax: [1, 200] };
const EMAIL_RE = /^[^\s@<>()",;]+@[^\s@<>()",;]+\.[^\s@<>()",;]+$/;
const PW_SOURCE = { keychain: "存在鑰匙圈", env: "來自環境變數 KB_SMTP_PASSWORD", file: "存在設定資料夾的 credentials.toml（鑰匙圈無法使用時的備用，只有你的帳號能讀）" };

async function buildGeneral(el) {
  const strip = h("div", { class: "stat-strip", "aria-label": "今日數字" });
  const file = h("p", { class: "muted small" });
  const btn = h("button", { type: "button", class: "btn", id: "actReindex", onclick: reindex }, icon("refresh"), "重建索引");
  el.append(strip, h("div", { class: "set-actions" }, btn), file);
  try {
    const [r, st] = await Promise.all([api("GET", "/api/reminders"), api("GET", "/api/stats")]);
    S.reminders = r;
    const n = st.stats;
    strip.replaceChildren(...[["今天新增", n.new_today], ["待複習", n.due], ["7 天內記住", n.known_week], ["全部卡片", n.total]]
      .map(([k, v]) => h("div", {}, h("b", {}, v), h("span", {}, k))));
    file.textContent = r.settings_file ? `設定檔：${r.settings_file}（不在任何 repo 內）` : "";
  } catch (e) { fail(e); }
}

async function reindex(e) {
  const b = e.currentTarget; b.classList.add("busy"); b.disabled = true;
  try { const r = await api("POST", "/api/reindex"); toast(`已重建索引：${r.nodes} 張卡片${r.errors.length ? `，${r.errors.length} 個檔案無法讀取` : ""}`); await refreshMeta(); }
  catch (er) { fail(er); } finally { b.classList.remove("busy"); b.disabled = false; }
}

async function loadReminders() {
  S.reminders = await api("GET", "/api/reminders");
}

async function buildNotify(el) {
  el.append($("tplNotify").content.cloneNode(true));
  paintIcons(el);
  $("setNotify").addEventListener("submit", (e) => { e.preventDefault(); saveSettings(); });
  for (const f of el.querySelectorAll("input")) f.addEventListener("input", () => fieldError(f, null));
  $("nTimes").addEventListener("input", () => warnTimes());
  $("nTimes").addEventListener("keydown", (e) => { if (e.key === "Enter" && !e.isComposing && e.keyCode !== 229) { e.preventDefault(); saveSettings(); } });
  $("nTest").onclick = async () => {
    try { const r = await api("POST", "/api/reminders/test-notify"); toast(`已送出通知：${r.title}`); }
    catch (e) { fail(e); }
  };
  try { await loadReminders(); renderNotify(); } catch (e) { fail(e); }
}

async function buildEmail(el) {
  el.append($("tplEmail").content.cloneNode(true));
  paintIcons(el);
  $("setEmail").addEventListener("submit", (e) => { e.preventDefault(); saveSettings(); });
  $("eSecurity").onchange = () => {
    const p = SEC_PORT[$("eSecurity").value], cur = +$("ePort").value;
    if (p && (!cur || cur === 465 || cur === 587)) $("ePort").value = p;
  };
  $("eEnabled").addEventListener("change", syncEmailFields);
  // Typing clears that field's error; leaving a number field pulls it into range.
  for (const f of el.querySelectorAll("input")) f.addEventListener("input", () => fieldError(f, null));
  for (const id of ["eDays", "eMax"]) $(id).addEventListener("change", () => clampField($(id)));
  $("ePwSave").onclick = async () => {
    const pw = $("ePw").value;
    if (!pw && !(await LoreShell.ask($("ePwSave"), ["密碼欄是空的：要刪除已存的 SMTP 密碼嗎？"], [{ label: "取消", value: null }, { label: "刪除", value: true, primary: true, danger: true }]))) return;
    try {
      const r = await api("PUT", "/api/reminders/password", { password: pw });
      $("ePw").value = "";
      const where = r.password?.source ?? r.saved;
      toast(!pw ? "已刪除密碼" : where === "keychain" ? "密碼已存到鑰匙圈" : "鑰匙圈無法使用，密碼改存在設定資料夾的 credentials.toml（只有你的帳號能讀）");
      await loadReminders(); renderEmail();
    } catch (e) { fail(e); }
  };
  $("eTest").onclick = async (ev) => {
    const b = ev.currentTarget;
    if (b.disabled) return;
    b.disabled = true; b.classList.add("busy");
    try {
      if (!(await saveSettings(true))) return;
      const r = await api("POST", "/api/reminders/test-email");
      toast(`已寄出測試信到 ${r.to.join("、")}`);
    } catch (e) { fail(e); try { await loadReminders(); renderEmail(); } catch {} } finally { b.disabled = false; b.classList.remove("busy"); }
  };
  try { await loadReminders(); renderEmail(); } catch (e) { fail(e); }
}

/** E-mail fields only matter when the digest is on (the toggle stays visible). */
function syncEmailFields() { if ($("eFields")) $("eFields").hidden = !$("eEnabled").checked; }

function clampField(el) {
  const [lo, hi] = NUM_RANGE[el.id];
  if (el.value === "") return;
  const v = Math.round(+el.value);
  if (Number.isFinite(v)) el.value = Math.min(hi, Math.max(lo, v));
}

/** Show (msg) or clear (null) an error under a field. */
function fieldError(el, msg) {
  el.setCustomValidity(msg ?? "");
  el.toggleAttribute("aria-invalid", !!msg);
  const id = `${el.id}Err`;
  let p = document.getElementById(id);
  if (!msg) { p?.remove(); if (el.getAttribute("aria-describedby") === id) el.removeAttribute("aria-describedby"); return; }
  if (!p) { p = h("span", { id, class: "field-err" }); el.closest("label")?.append(p) ?? el.after(p); }
  p.textContent = msg;
  el.setAttribute("aria-describedby", id);
}

/** Notification times: 「9:5」「21：00」 are fine; returns [normalised list, bad entry]. */
function parseTimes(raw) {
  const out = [];
  for (const t of splitList(raw)) {
    const m = /^(\d{1,2})[:：](\d{1,2})$/.exec(t);
    if (!m || +m[1] > 23 || +m[2] > 59) return [null, t];
    out.push(`${m[1].padStart(2, "0")}:${m[2].padStart(2, "0")}`);
  }
  return [[...new Set(out)].sort(), null];
}
function warnTimes() {
  if (!$("nTimes")) return;
  const empty = !splitList($("nTimes").value).length;
  $("nTimesWarn").hidden = !empty;
}

/** Client-side checks, Chinese messages next to the field; focuses the first problem. */
function validateSettings(forTest = false) {
  document.querySelectorAll("#setNotify [aria-invalid], #setEmail [aria-invalid]").forEach((el) => fieldError(el, null));
  for (const id of ["eDays", "eMax"]) if ($(id)) clampField($(id));
  const errs = [];
  if ($("nTimes")) {
    const [, badTime] = parseTimes($("nTimes").value);
    if (badTime) errs.push([$("nTimes"), `「${badTime}」看不懂，請寫成 09:00 這樣，用逗號分隔`]);
  }
  if ($("eHost")) validateEmail(errs, forTest);
  for (const [el, m] of errs) fieldError(el, m);
  if (errs.length) {
    const el = errs[0][0];
    if (el.closest("#eFields")?.hidden) $("eFields").hidden = false;   // show, without turning the digest on
    el.focus();
    toast(errs.length > 1 ? `有 ${errs.length} 個欄位要修正` : errs[0][1], true);
    return false;
  }
  return true;
}

function validateEmail(errs, forTest) {
  const enabled = $("eEnabled").checked || forTest;
  const port = $("ePort").value;
  if (port !== "" && !(+port >= 1 && +port <= 65535 && Number.isInteger(+port))) errs.push([$("ePort"), "連接埠要是 1–65535 的數字（Gmail 用 587）"]);
  if (enabled && !$("eHost").value.trim()) errs.push([$("eHost"), "請填 SMTP 主機，例如 smtp.gmail.com"]);
  const user = $("eUser").value.trim(), from = $("eFrom").value.trim();
  if (user && !EMAIL_RE.test(user)) errs.push([$("eUser"), "帳號要是完整的 e-mail 地址"]);
  if (enabled && !user && !from) errs.push([$("eUser"), "請填 SMTP 帳號（也會當成寄件人）"]);
  if (from && !EMAIL_RE.test(from)) errs.push([$("eFrom"), "寄件人要是有效的 e-mail 地址"]);
  const to = splitList($("eTo").value);
  const badTo = to.find((a) => !EMAIL_RE.test(a));
  if (badTo) errs.push([$("eTo"), `「${badTo}」不是有效的 e-mail 地址`]);
  else if (enabled && !to.length) errs.push([$("eTo"), "請填收件人"]);
  if (enabled && !$("eAt").value) errs.push([$("eAt"), "請選寄送時間"]);
}

function settingsFromForm() {
  const n = (id, d) => { const v = Math.round(+$(id).value); return $(id).value !== "" && Number.isFinite(v) ? v : d; };
  const cur = S.reminders.settings;
  return {
    notify: !$("nTimes") ? cur.notify : {
      enabled: $("nEnabled").checked,
      times: parseTimes($("nTimes").value)[0] ?? splitList($("nTimes").value),
      grace_minutes: cur.notify.grace_minutes ?? 2,
    },
    email: !$("eHost") ? cur.email : {
      enabled: $("eEnabled").checked,
      smtp_host: $("eHost").value.trim(), smtp_port: n("ePort", 587), security: $("eSecurity").value,
      user: $("eUser").value.trim(), from: $("eFrom").value.trim(), to: splitList($("eTo").value),
      send_at: $("eAt").value || "07:30", skip_empty: $("eSkip").checked,
      recent_days: n("eDays", 7), max_due: n("eMax", 30),
    },
  };
}

async function saveSettings(forTest = false) {
  if (!validateSettings(forTest)) return false;
  try {
    S.reminders = await api("PUT", "/api/reminders", settingsFromForm());
    renderNotify(); renderEmail();
    if (!forTest) toast(S.reminders.settings.notify.times.length ? "設定已儲存" : "設定已儲存（沒有通知時間，不會跳到期通知）");
    return true;
  } catch (e) {
    toast(e.category === "format" ? `設定${errorText(e)}` : errorText(e), true);
    return false;
  }
}

function fmtWhen(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  return isNaN(d) ? iso : `${d.getMonth() + 1}/${d.getDate()} ${fmtTime(iso)}`;
}

function renderNotify() {
  if (!$("nTimes") || !S.reminders) return;
  const { settings: s, state: st } = S.reminders;
  document.querySelectorAll("#setNotify [aria-invalid]").forEach((el) => fieldError(el, null));
  $("nEnabled").checked = s.notify.enabled;
  $("nTimes").value = s.notify.times.join(", ");
  warnTimes();
  const by = { neondeck: "NeonDeck", "kb-server": "知識庫" }[st.notify_last_by] ?? st.notify_last_by;
  $("nState").textContent = st.notify_last_slot ? `上次通知時段：${st.notify_last_slot.replace("T", " ")}（${by}）${st.notify_last_error ? " · 上次通知沒有成功" : ""}` : "";
}

function renderEmail() {
  if (!$("eHost") || !S.reminders) return;
  const { settings: s, state: st, password: pw } = S.reminders;
  document.querySelectorAll("#setEmail [aria-invalid]").forEach((el) => fieldError(el, null));
  const e = s.email;
  $("eEnabled").checked = e.enabled;
  syncEmailFields();
  $("eHost").value = e.smtp_host; $("ePort").value = e.smtp_port; $("eSecurity").value = e.security;
  $("eUser").value = e.user; $("eFrom").value = e.from; $("eTo").value = e.to.join(", ");
  $("eAt").value = e.send_at; $("eDays").value = e.recent_days; $("eMax").value = e.max_due; $("eSkip").checked = e.skip_empty;
  const src = PW_SOURCE[pw.source] ?? pw.source;
  $("ePwState").replaceChildren(pw.set ? h("span", { class: "ok-text" }, `✓ 已設定密碼 · ${src}`) : h("span", {}, "還沒有設定密碼"));
  const parts = [];
  if (st.email_last_sent) parts.push(`上次寄送：${st.email_last_sent}${st.email_last_result?.startsWith("skipped") ? "（沒有內容，略過）" : ""}`);
  if (st.email_last_error) parts.push(h("span", { class: "err-text", title: st.email_last_error }, `上次失敗（${fmtWhen(st.email_last_attempt)}）：${hasCJK(st.email_last_error) ? plainZh(st.email_last_error) : "寄信失敗，請檢查 SMTP 設定"}`));
  if (e.enabled) parts.push(`每天 ${e.send_at} 寄出；Mac 睡著錯過時，醒來後補寄一次。`);
  $("eState").replaceChildren(...parts.flatMap((p, i) => (i ? [" · ", p] : [p])));
}

boot();

// ── 待審 (pending cards from LoreEcho) ──────────────────
// Files live in <space>/pending/; they are not cards (not in the list, search or review) until
// 確認. The editor opens in 待審 mode: 確認 (primary) makes a normal card, 丟掉 throws it away,
// and a card with the same title or alias as an existing one offers 合併到既有卡.
S.pending = [];
async function refreshPending() {
  try {
    const { pending } = await api("GET", "/api/pending");
    S.pending = pending;
    $("cPending").textContent = pending.length || "";
    $("navPending").hidden = !pending.length;
    if (S.view === "pending") renderPending();
  } catch {}
}
async function loadPending() { await refreshPending(); renderPending(); }
function renderPending() {
  $("pendingInfo").textContent = S.pending.length ? `${S.pending.length} 張` : "";
  const list = $("pendingList");
  if (!S.pending.length) { list.replaceChildren(h("div", { class: "empty" }, h("b", {}, "沒有待審的卡"))); return; }
  list.replaceChildren(h("ul", {}, S.pending.map((c) => h("li", {}, h("button", { type: "button", onclick: () => { location.hash = `#/pending/${c.id}`; } },
    h("span", { class: "t" }, c.title),
    c.duplicates.length ? h("span", { class: "dup", title: `已有同名卡：${c.duplicates.map((d) => d.title).join("、")}` }, "重複") : null,
    h("span", { class: "src" }, c.source || "—"),
    h("span", { class: "when mono" }, fmtWhen(c.created)))))));
}

/** Editor chrome for a pending card (`c`) or back to normal (null). */
function pendingMode(c) {
  $("edDiscard").hidden = !c;
  $("edSave").lastChild.textContent = c ? "確認" : "儲存";
  $("edDupBar").hidden = true;
  if (c) { $("edDelete").hidden = true; $("edGraph").hidden = true; $("edShareWrap").hidden = true; $("edDraftBar").hidden = true; }
}

async function openPending(id) {
  if (S.view === "editor" && edHash() !== `#/pending/${id}` && isDirty() && !(await askLeave())) { setHash(edHash()); return; }
  if (S.view === "editor" && S.ed) S.ed.leaving = true;
  await refreshPending();
  const c = S.pending.find((x) => x.id === id);
  if (!c) { toast("這張待審卡已經不在了", true); S.ed = null; setView("pending"); return; }
  S.ed = { id: null, space: c.space, revision: null, images: [], links: c.links ?? [], pending: c };
  setView("editor");
  setHash(`#/pending/${id}`);
  pendingMode(c);
  $("edHeading").textContent = "待審";
  $("edSpace").disabled = true; $("edSpace").value = c.space;
  $("edTitle").value = c.title; $("edAliases").value = (c.aliases ?? []).join(", ");
  $("edKind").value = S.reg.kinds.some((k) => k.id === c.kind) ? c.kind : "concept";
  $("edTags").value = (c.tags ?? []).join(", "); $("edDomains").value = (c.domains ?? []).join(", ");
  $("edSummary").value = c.summary ?? ""; $("edBody").value = c.body ?? "";
  $("edMeta").textContent = [c.source, fmtWhen(c.created)].filter(Boolean).join(" · ");
  $("edBacklinks").replaceChildren(); $("edProgress").replaceChildren();
  renderEdLists(); renderPreview();
  S.ed.snap = edState(); paintStatus();
  if (c.duplicates.length) {
    const bar = $("edDupBar");
    bar.replaceChildren(
      h("span", {}, `已有同名卡：${c.duplicates.map((d) => d.title).join("、")}`),
      ...c.duplicates.map((d) => h("button", { type: "button", class: "btn small", onclick: () => mergePending(d) }, c.duplicates.length > 1 ? `合併到「${d.title}」` : "合併到既有卡")));
    bar.hidden = false;
  }
}

async function acceptPending() {
  if (!$("edTitle").value.trim()) { toast("請輸入標題", true); $("edTitle").focus(); return; }
  await finishPending(() => api("POST", `/api/pending/${S.ed.pending.id}/accept`, fieldsFromForm()), "已加入知識卡");
}
async function mergePending(d) {
  await finishPending(() => api("POST", `/api/pending/${S.ed.pending.id}/accept`, { ...fieldsFromForm(), merge_into: d.id }), `已合併到「${d.title}」`, true);
}
async function finishPending(call, msg, openAfter = false) {
  if (!S.ed?.pending || S.ed.saving) return;
  S.ed.saving = true; $("edSave").disabled = true; paintStatus();
  try {
    const r = await call();
    markClean(); S.ed.leaving = true;
    toast(msg);
    await refreshMeta();
    if (openAfter) await openEditor(r.node.id);
    else setView(S.pending.length ? "pending" : "cards");
  } catch (e) { fail(e); }
  finally { S.ed && (S.ed.saving = false); $("edSave").disabled = false; paintStatus(); }
}
async function discardPending() {
  if (!S.ed?.pending) return;
  const ok = await LoreShell.ask($("edDiscard"), ["丟掉這張待審卡？"], [{ label: "留下", value: false }, { label: "丟掉", value: true, primary: true }]);
  if (ok !== true) return;
  try {
    await api("DELETE", `/api/pending/${S.ed.pending.id}`);
    S.ed.leaving = true;
    try { localStorage.removeItem(draftKey(null)); } catch {}
    await refreshPending();
    toast("已丟掉");
    setView(S.pending.length ? "pending" : "cards");
  } catch (e) { fail(e); }
}
