// LoreEcho page: the bubble, the open window (character + conversation), memory only.
// Needs theme.js, icons.js and shell.js (LoreShell) first. No build step.
//
// Server: GET/PUT /api/window, GET/PUT /api/settings, GET /api/characters,
// /api/characters/<id>/avatar/<state> (portrait: the bubble, name tag), /api/characters/<id>/full/<state>
// (full body: the open window; both follow the state's fallback chain), the 人物 tab's PUT/DELETE of
// those, POST /api/characters (+ /import, /<id>/export), POST /api/chat (server-sent events).
// In the desktop app, PUT /api/window { expanded } also resizes the window, and the page
// drags the window itself through Tauri (window.__TAURI__, withGlobalTauri).

"use strict";

const $ = (id) => document.getElementById(id);
const S = {
  desktop: false,
  chars: [],
  states: [],          // the ten states, from the server
  chains: {},          // state → [state, …, "idle"]: what a state without a picture shows
  rev: 0,              // bumped when a picture changes (cache-buster)
  state: "idle",
  timers: { wait: 0, mood: 0 },
  char: null,          // { id, name, avatars, full, figure: { scale, offset_y } }
  msgs: [],            // [{ role, text }] — never saved
  line: null,          // the reply shown in the dialogue box (moves to the history on the next turn)
  busy: null,          // AbortController of the running reply
  expanded: false,
  shell: null,
};

function h(tag, attrs = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const c of kids.flat()) if (c != null) el.append(c);
  return el;
}

async function api(method, path, body) {
  const r = await fetch(path, { method, headers: body ? { "content-type": "application/json" } : {}, body: body ? JSON.stringify(body) : undefined, cache: "no-store" });
  const data = await r.json().catch(() => ({}));
  if (!r.ok) { const e = new Error(data.error || `${r.status}`); e.body = data; throw e; }
  return data;
}

const tauriWindow = () => window.__TAURI__?.window?.getCurrentWindow?.() ?? null;

// ── boot ────────────────────────────────────────────────────────
boot();
async function boot() {
  mountShell();
  paintIcons();
  wire();
  try {
    const [health, win] = await Promise.all([api("GET", "/api/health"), api("GET", "/api/window")]);
    S.desktop = !!health.desktop;
    setWinSize(win.w, win.h);
    wireResize();
    await loadCharacters();
    show(!!win.expanded, false);
  } catch (e) {
    show(true, false);
    LoreShell.toast(`無法連到 LoreEcho 伺服器：${e.message}`, { err: true });
  }
}

function mountShell() {
  S.shell = LoreShell.mount({
    app: "le", brand: "LORE/ECHO", host: "#win header.top",
    primary: [
      { id: "actChoices", icon: "listUl", label: "功能列表", run: () => toggleChoices() },
      { id: "actNew", icon: "plus", label: "新對話", run: newChat },
    ],
    general: buildGeneral,
    settingsTabs: [{ id: "character", label: "人物", build: buildCharacters }],
    keys: [{ label: "換行", keys: "⇧↵" }, { label: "功能列表：上一個／下一個", keys: "↑ ↓" }],
  });
  const collapse = h("button", { type: "button", class: "ico", id: "actCollapse", "aria-label": "收起 Esc", title: "收起成泡泡（對話會清掉）" }, h("span", { "data-icon": "chevron" }));
  collapse.addEventListener("click", () => show(false));
  S.shell.el.prepend(collapse);
  // which model answers: the next reply's (自動 · 本機 Gemma 4 12B), then the one that answered; click = 自動／雲／地
  S.chip = LoreShell.engineChip({ task: "le-chat", live: true, label: "這次回答用的模型" });
  S.chip.id = "engine";
  S.shell.slots.lead.append(S.chip);
}

// ── characters ──────────────────────────────────────────────────
async function loadCharacters() {
  const c = await api("GET", "/api/characters");
  S.chars = c.characters;
  S.states = c.states ?? [];
  S.chains = c.chains ?? {};
  S.char = S.chars.find((x) => x.id === c.current) ?? S.chars[0] ?? null;
  const name = S.char?.name ?? "LoreEcho";
  $("who").textContent = name;
  S.rev++;
  $("nametag").setAttribute("aria-label", `${name}（換人物）`);
  $("bubble").setAttribute("aria-label", `打開 LoreEcho（${name}）`);
  const f = S.char?.figure ?? {};
  $("win").style.setProperty("--fig-scale", f.scale ?? 1);
  $("win").style.setProperty("--fig-dy", f.offset_y ?? 0);
  $("figure").classList.toggle("portrait", !(S.char?.full?.length > 0));
  setState("idle");
  setImg($("bubbleImg"), $("bubbleIcon"), "avatar", "idle");
  // fetch the other states' pictures now, so a state change does not flicker
  for (const st of S.states) for (const kind of ["full", "avatar"]) { const u = picUrl(kind, st); if (u) new Image().src = u; }
}

/** The picture of `kind` (avatar = portrait, full = full body) for `state`, or null when the
 *  character has no pictures at all. The server resolves a missing state along its chain
 *  (and a character without full bodies shows its portrait in the open window). */
function picUrl(kind, state) {
  const c = S.char;
  if (!c || !(c.avatars?.length || c.full?.length)) return null;
  return `/api/characters/${encodeURIComponent(c.id)}/${kind}/${state}?v=${S.rev}`;
}

/** Show the character's picture (the app icon when it has none at all). */
function setImg(img, icon, kind, state) {
  const url = picUrl(kind, state);
  img.hidden = !url; icon.hidden = !!url;
  if (url && img.dataset.src !== url) {
    img.dataset.src = url; img.src = url;
    // never draw a picture taller than its own pixels (a small raster would blur)
    img.onload = () => { img.style.maxHeight = img.naturalHeight ? `${img.naturalHeight}px` : ""; };
  }
}

/** One of the ten states (idle listening thinking speaking happy unsure surprised waiting
 *  searching noting): the full body, and the small portrait on the name tag. */
function setState(state) {
  S.state = state;
  $("figure").dataset.state = state;
  setImg($("full"), $("fullIcon"), "full", state);
  const url = picUrl("avatar", state);
  $("whoImg").hidden = !url;
  if (url && $("whoImg").dataset.src !== url) { $("whoImg").dataset.src = url; $("whoImg").src = url; }
}

/** Show `state` while `work()` runs, then go back (hook for the moments that have no signal
 *  yet: searching while KnowledgeBase is asked, noting while a note is written). */
async function whileState(state, work) {
  const before = S.state;
  setState(state);
  try { return await work(); }
  finally { if (S.state === state) setState(S.busy ? "thinking" : before === "listening" ? "idle" : before); }
}
window.LoreEcho = { setState, whileState };

/** A simple cue in a finished reply: happy | unsure | surprised | null. */
function moodOf(text) {
  if (/不確定|不太確定|不太清楚|不一定|我猜|也許|說不準|不知道/.test(text)) return "unsure";
  if (/^(咦|哇|欸|啊)|沒想到|竟然|居然|真的嗎|嚇/.test(text)) return "surprised";
  if (/太好了|恭喜|很棒|答對|沒錯|說得對|成功了|做得好/.test(text)) return "happy";
  return null;
}

/** After a reply: show its mood for a few seconds, then idle. */
function settle(mood) {
  clearTimeout(S.timers.mood);
  setState(mood ?? "idle");
  if (mood) S.timers.mood = setTimeout(() => { if (!S.busy && S.state === mood) setState("idle"); }, 4000);
}

/** Switch to character `id` (settings.toml, same as 齒輪 → 一般); a new conversation starts. */
async function switchCharacter(id) {
  if (id === S.char?.id) return;
  try {
    await api("PUT", "/api/settings", { character: id });
    await loadCharacters();
    newChat();
    LoreShell.toast(`換成「${S.char?.name}」`);
  } catch (e) { LoreShell.toast(e.message, { err: true }); }
}

/** The name tag's menu: every character, the current one marked. */
function characterMenu() {
  const items = S.chars.map((c) => ({ label: c.name === c.id ? c.id : `${c.name}（${c.id}）`, current: c.id === S.char?.id, run: () => switchCharacter(c.id) }));
  items.push("-", { icon: "gear", label: "人物設定…", run: () => LoreShell.settings("character") });
  LoreShell.menu($("nametag"), items);
}

// ── 功能列表: choice rows (design §4), wired in a later phase ─────
function choiceRows() { return [...document.querySelectorAll("#choices .choice")]; }

function toggleChoices(on = $("choices").hidden) {
  $("choices").hidden = !on;
  $("actChoices")?.setAttribute("aria-pressed", String(on));
  if (on) choiceRows()[0]?.focus();
}

function wireChoices() {
  for (const b of choiceRows()) {
    b.addEventListener("click", () => LoreShell.toast(`「${b.querySelector(".label").textContent}」下一階段才會有`));
    b.addEventListener("mouseenter", () => b.focus({ preventScroll: true }));
  }
  $("choices").addEventListener("keydown", (e) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const rows = choiceRows();
    const i = rows.indexOf(document.activeElement);
    rows[(i + (e.key === "ArrowDown" ? 1 : rows.length - 1)) % rows.length].focus();
  });
}

// ── the open window's size (420–1000 px, remembered with the bubble's place) ──
const WIN_MIN = 420, WIN_MAX = 1000;
const clampWin = (v) => Math.min(WIN_MAX, Math.max(WIN_MIN, Math.round(v)));
function setWinSize(w, h) {
  const root = document.documentElement.style;
  if (w) root.setProperty("--win-w", `${clampWin(w)}px`);
  if (h) root.setProperty("--win-h", `${clampWin(h)}px`);
}
// The window keeps its bottom-right corner, so its left and top edges (and their corner) are
// the grips; the desktop app resizes the window itself (PUT /api/window {w, h}).
function wireResize() {
  const win = $("win");
  for (const edge of ["l", "t", "tl"]) win.append(h("div", { class: `grip grip-${edge}`, "data-edge": edge, "aria-hidden": "true" }));
  let drag = null, queued = false;
  const push = () => {
    queued = false;
    if (drag) api("PUT", "/api/window", { w: drag.w, h: drag.h }).catch(() => {});
  };
  win.addEventListener("pointerdown", (e) => {
    const g = e.target.closest?.(".grip");
    if (!g || e.button !== 0) return;
    e.preventDefault();
    g.setPointerCapture(e.pointerId);
    drag = { edge: g.dataset.edge, x: e.screenX, y: e.screenY, w0: win.offsetWidth, h0: win.offsetHeight, w: win.offsetWidth, h: win.offsetHeight };
  });
  win.addEventListener("pointermove", (e) => {
    if (!drag) return;
    if (drag.edge.includes("l")) drag.w = clampWin(drag.w0 - (e.screenX - drag.x));
    if (drag.edge.includes("t")) drag.h = clampWin(drag.h0 - (e.screenY - drag.y));
    setWinSize(drag.w, drag.h);
    if (!queued) { queued = true; requestAnimationFrame(push); }
  });
  const end = () => { if (drag) { push(); drag = null; } };
  win.addEventListener("pointerup", end);
  win.addEventListener("pointercancel", end);
  // the desktop window resized from outside (the system's own edge drag): the page follows it
  if (S.desktop) addEventListener("resize", () => { if (S.expanded && !drag) setWinSize(innerWidth, innerHeight); });
}

// ── bubble ⇄ open window ────────────────────────────────────────
async function show(expanded, save = true) {
  if (!expanded && S.expanded) {
    newChat();                                   // closing the window ends the conversation (design §2)
    const p = $("win");
    p.classList.remove("opening"); p.classList.add("closing");
    await new Promise((r) => setTimeout(r, matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 120));
    p.classList.remove("closing");
  }
  S.expanded = expanded;
  if (save) {
    try { await api("PUT", "/api/window", { expanded }); }
    catch (e) { if (expanded) LoreShell.toast(e.message, { err: true }); }
  }
  $("win").hidden = !expanded;
  $("bubble").hidden = expanded;
  if (expanded) {
    S.chip?.refresh();
    const p = $("win");
    p.classList.remove("closing"); p.classList.add("opening");
    setTimeout(() => p.classList.remove("opening"), 200);
    $("text").focus();
    scrollDown();
  }
}

function wire() {
  // the bubble: a short press opens, a drag moves the window
  const b = $("bubble");
  let down = null;
  b.addEventListener("pointerdown", (e) => { if (e.button === 0) down = { x: e.screenX, y: e.screenY, dragged: false }; });
  b.addEventListener("pointermove", (e) => {
    if (!down || down.dragged || Math.hypot(e.screenX - down.x, e.screenY - down.y) < 4) return;
    const w = tauriWindow();
    if (w) { down.dragged = true; w.startDragging().catch(() => {}); }
  });
  b.addEventListener("pointerup", () => { if (down && !down.dragged) show(true); down = null; });
  b.addEventListener("click", (e) => { if (e.detail === 0) show(true); });   // keyboard

  $("nametag").addEventListener("click", characterMenu);
  wireChoices();
  $("form").addEventListener("submit", (e) => { e.preventDefault(); send(); });
  const t = $("text");
  t.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing && e.keyCode !== 229) { e.preventDefault(); send(); }
  });
  t.addEventListener("input", grow);
  // the character listens while the message is being written (the voice phase will use the same state for recording)
  t.addEventListener("input", () => { if (!S.busy) setState(t.value.trim() ? "listening" : "idle"); });
  t.addEventListener("blur", () => { if (!S.busy && S.state === "listening") setState("idle"); });
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !S.expanded || document.querySelector("dialog[open]") || document.querySelector(".menu, .ask")) return;
    e.preventDefault();
    if (!$("choices").hidden) { toggleChoices(false); $("text").focus(); }
    else show(false);
  });
}

function grow() {
  const t = $("text");
  t.style.height = "auto";
  t.style.height = `${Math.min(t.scrollHeight, 120)}px`;
}

// ── the conversation: earlier turns in the history, the current line in the box ──
function scrollDown() { const m = $("hist"); m.scrollTop = m.scrollHeight; $("line").scrollTop = $("line").scrollHeight; }

function addHist(role, text) {
  const li = h("li", { class: role }, text);
  $("hist").append(li);
  scrollDown();
  return li;
}

/** The dialogue box: `text` with a look (""/pending/err/empty). */
function setLine(text, look = "") {
  const l = $("line");
  l.className = `line ${look}`.trim();
  l.textContent = text;
  l.scrollTop = l.scrollHeight;
  return l;
}

const GREETING = "想聊什麼都可以。對話只留在這個視窗裡，按「新對話」或收起就清掉。";

function newChat() {
  S.busy?.abort();
  S.busy = null;
  clearTimeout(S.timers.wait); clearTimeout(S.timers.mood);
  S.msgs = [];
  S.line = null;
  window.LoreEchoKb?.clear();
  $("hist").replaceChildren();
  setLine(GREETING, "empty");
  S.chip.actual("");
  setState("idle");
  $("send").disabled = false;
  if (S.expanded) $("text").focus();
}

async function send(engine) {
  if (S.busy) return;
  if (!engine) {
    const text = $("text").value.trim();
    if (!text) return;
    $("text").value = ""; grow();
    if (S.line) { addHist("assistant", S.line); S.line = null; }   // the last reply joins the history
    S.msgs.push({ role: "user", text });
    addHist("user", text);
  }
  const ctrl = new AbortController();
  S.busy = ctrl;
  $("send").disabled = true;
  clearTimeout(S.timers.mood);
  setState("thinking");
  clearTimeout(S.timers.wait);
  S.timers.wait = setTimeout(() => { if (S.busy === ctrl && !reply) setState("waiting"); }, 8000);   // a long wait for the model
  setLine("", "pending");
  S.chip.actual("");   // until this reply says which model answers, the prediction shows
  S.chip.busy(true);
  let reply = "";
  try {
    const r = await fetch("/api/chat", {
      method: "POST", headers: { "content-type": "application/json" }, signal: ctrl.signal,
      body: JSON.stringify({ messages: S.msgs, character: S.char?.id, engine, kb: window.LoreEchoKb?.ask() }),
    });
    if (!r.ok || !(r.headers.get("content-type") || "").startsWith("text/event-stream")) {
      const data = await r.json().catch(() => ({}));
      failed(data.error || `${r.status}`, data.alternative);
      return;
    }
    for await (const [ev, data] of events(r.body)) {
      if (ctrl.signal.aborted) return;
      if (ev === "search" || ev === "cards") window.LoreEchoKb?.event(ev, data);   // kbcards.js
      else if (ev === "start") { S.chip.actual(data.label); }
      else if (ev === "delta") { if (!reply) { clearTimeout(S.timers.wait); setState("speaking"); } reply += data.text; setLine(reply, "pending"); }
      else if (ev === "done") { reply = data.text || reply; }
      else if (ev === "error") { if (reply) keep(reply); failed(data.error); return; }
    }
    if (reply.trim()) keep(reply);
    else failed("沒有收到回答");
  } catch (e) {
    if (ctrl.signal.aborted) return;
    if (reply) keep(reply);
    failed(e.message);
  } finally {
    clearTimeout(S.timers.wait);
    S.chip.busy(false);
    if (S.busy === ctrl) { S.busy = null; $("send").disabled = false; settle(reply.trim() ? moodOf(reply) : null); }
  }
}

/** A finished (or cut-off) reply: part of the conversation, shown in the box. */
function keep(reply) {
  S.msgs.push({ role: "assistant", text: reply });
  S.line = reply;
  setLine(reply);
}

/** A problem: in the box when nothing was said, else under the history; with
 *  「改用本機／改用 Claude」 when the server offers it. */
function failed(msg, alt) {
  let el;
  if (S.line && S.msgs.at(-1)?.role === "assistant") el = addHist("err", msg);
  else el = setLine(msg, "err");
  if (alt?.engine) {
    el.append(h("button", { type: "button", class: "btn", onclick: (e) => { e.currentTarget.remove(); send(alt.engine); } }, alt.label || "改用另一個"));
  }
}

/** Server-sent events of a fetch body: yields [event, data]. */
async function* events(body) {
  const reader = body.getReader();
  const dec = new TextDecoder();
  let buf = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += dec.decode(value, { stream: true }).replace(/\r\n/g, "\n");
    let i;
    while ((i = buf.indexOf("\n\n")) >= 0) {
      const block = buf.slice(0, i); buf = buf.slice(i + 2);
      let ev = "message", data = "";
      for (const line of block.split("\n")) {
        if (line.startsWith("event:")) ev = line.slice(6).trim();
        else if (line.startsWith("data:")) data += line.slice(5).replace(/^ /, "");
      }
      if (!data) continue;            // keep-alive comments
      try { yield [ev, JSON.parse(data)]; } catch { /* not ours */ }
    }
  }
}

// ── settings: 一般 ───────────────────────────────────────────────
async function buildGeneral(el) {
  el.append($("tplGeneral").content.cloneNode(true));
  const key = el.querySelector("#setShortcut");
  try { key.value = (await api("GET", "/api/settings")).shortcut ?? ""; } catch {}
  key.addEventListener("change", async () => {
    try {
      const s = await api("PUT", "/api/settings", { shortcut: key.value });
      key.value = s.shortcut;
      LoreShell.toast(s.shortcut ? `快捷鍵：${s.shortcut}` : "不用快捷鍵");
    } catch (e) { LoreShell.toast(e.message, { err: true }); }
  });
}

// ── settings: 人物 (design §6) ───────────────────────────────────
const STATE_LABEL = { idle: "待機", listening: "聽", thinking: "想", speaking: "說", happy: "開心", unsure: "不確定", surprised: "驚訝", waiting: "等待", searching: "查知識庫", noting: "記筆記" };
const ROWS = [["avatar", "縮圖"], ["full", "對話人物"]];
const label = (c) => (c.name === c.id ? c.id : `${c.name}（${c.id}）`);

async function buildCharacters(el) {
  el.replaceChildren();
  const c = S.char;
  const sec = (title, ...kids) => h("div", { class: "set-sec" }, h("div", { class: "set-label" }, title), ...kids);
  const row = (k, ...kids) => h("div", { class: "set-row" }, h("span", { class: "k" }, k), ...kids);
  const rebuild = () => buildCharacters(el);
  const pickFile = (accept, onFile) => {
    const f = h("input", { type: "file", accept, hidden: true });
    f.addEventListener("change", () => { if (f.files[0]) onFile(f.files[0]); f.remove(); });
    document.body.append(f); f.click();
  };

  // 切換人物 · 新增 · 匯出 · 匯入
  const sel = h("select", { class: "set-input", "aria-label": "人物" }, S.chars.map((x) => h("option", { value: x.id }, label(x))));
  sel.value = c?.id ?? "";
  sel.addEventListener("change", async () => { await switchCharacter(sel.value); rebuild(); });
  const btn = (icon, text, run, attrs = {}) => h("button", { type: "button", class: "btn", onclick: run, ...attrs }, h("span", { "data-icon": icon }), text);
  const bar = h("div", { class: "set-row" }, sel,
    btn("plus", "新增人物", (e) => newCharacter(e.currentTarget, rebuild)),
    btn("download", "匯出", () => exportCharacter(c), { disabled: !c || c.id === "default" }),
    btn("upload", "匯入", () => pickFile(".zip,application/zip", (f) => importCharacter(f, rebuild))));
  el.append(sec("切換人物", bar));
  if (!c || c.id === "default") { el.append(h("div", { class: "set-empty" }, "沒有人物。")); paintIcons(el); return; }

  // name · persona · voice
  const d = await api("GET", `/api/characters/${encodeURIComponent(c.id)}`).catch(() => null);
  const save = async (patch) => {
    try { await api("PUT", `/api/characters/${encodeURIComponent(c.id)}`, patch); if (patch.name != null) await loadCharacters(); }
    catch (e) { LoreShell.toast(e.message, { err: true }); }
  };
  const name = h("input", { class: "set-input", value: d?.name ?? c.name, "aria-label": "名稱", style: "flex:1" });
  name.addEventListener("change", () => save({ name: name.value }));
  const persona = h("textarea", { class: "set-input", rows: 4, "aria-label": "個性", style: "flex:1;resize:vertical" }, d?.persona ?? "");
  persona.addEventListener("change", () => save({ persona: persona.value }));
  const voice = h("input", { class: "set-input", value: d?.voice ?? "", "aria-label": "聲音", placeholder: "之後的階段", disabled: true, style: "flex:1" });
  el.append(sec("角色", row("名稱", name), row("個性", persona), row("聲音", voice)));

  // the pictures: one row of drop zones per kind
  for (const [kind, title] of ROWS) {
    const grid = h("div", { class: "pic-grid" + (kind === "full" ? " tall" : ""), role: "group", "aria-label": title });
    el.append(sec(title, grid));
    const paint = () => {
      grid.replaceChildren(...S.states.map((st) => cell(kind, st, c, paint, rebuildPics)));
      paintIcons(grid);
    };
    el._paints = [...(el._paints ?? []), paint];
    paint();
  }
  function rebuildPics() { for (const p of el._paints) p(); }
  paintIcons(el);
}

/** One drop zone: preview (or the fallback, dimmed), the state's name, 選擇圖片… / 移除. */
function cell(kind, st, c, paint, paintAll) {
  const own = (kind === "avatar" ? c.avatars : c.full)?.includes(st);
  const via = own ? null : S.chains[st]?.find((x) => x !== st && (kind === "avatar" ? c.avatars : c.full)?.includes(x));
  const url = (own || via) ? `/api/characters/${encodeURIComponent(c.id)}/${kind}/${own ? st : via}?v=${S.rev}` : null;
  const zone = h("div", { class: "pic-zone" + (own ? " own" : ""), tabindex: 0, role: "button", "aria-label": `${STATE_LABEL[st]}：選擇圖片`, title: via ? `沒有圖，用「${STATE_LABEL[via]}」` : own ? "" : "沒有圖" },
    url ? h("img", { src: url, alt: "", draggable: "false" }) : h("span", { class: "none", "data-icon": "image" }));
  const upload = async (file) => {
    if (!file) return;
    try {
      const r = await fetch(`/api/characters/${encodeURIComponent(c.id)}/${kind}/${st}`, { method: "PUT", body: file });
      if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error || `${r.status}`);
      await refreshChars(); paintAll();
    } catch (e) { LoreShell.toast(e.message, { err: true }); }
  };
  const choose = () => {
    const f = h("input", { type: "file", accept: ".png,.jpg,.jpeg,.webp,.svg,image/png,image/jpeg,image/webp,image/svg+xml", hidden: true });
    f.addEventListener("change", () => { upload(f.files[0]); f.remove(); });
    document.body.append(f); f.click();
  };
  zone.addEventListener("click", choose);
  zone.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); choose(); } });
  zone.addEventListener("dragover", (e) => { e.preventDefault(); zone.classList.add("over"); });
  zone.addEventListener("dragleave", () => zone.classList.remove("over"));
  zone.addEventListener("drop", (e) => { e.preventDefault(); zone.classList.remove("over"); upload(e.dataTransfer.files[0]); });
  const rm = h("button", { type: "button", class: "ico small", "aria-label": `移除${STATE_LABEL[st]}的圖`, title: "移除", disabled: !own }, h("span", { "data-icon": "trash" }));
  rm.addEventListener("click", async () => {
    if (!(await LoreShell.ask(rm, [`移除「${STATE_LABEL[st]}」的圖？`], [{ label: "取消", value: null }, { label: "移除", value: true, danger: true }]))) return;
    try { await api("DELETE", `/api/characters/${encodeURIComponent(c.id)}/${kind}/${st}`); await refreshChars(); paintAll(); }
    catch (e) { LoreShell.toast(e.message, { err: true }); }
  });
  return h("div", { class: "pic-cell" }, zone,
    h("div", { class: "pic-foot" }, h("span", { class: "pic-name" }, STATE_LABEL[st], via ? h("small", {}, `→ ${STATE_LABEL[via]}`) : null), rm));
}

/** Reload the list after a picture changed (pictures on screen follow). */
async function refreshChars() {
  const id = S.char?.id;
  await loadCharacters();
  if (id && S.char?.id !== id) S.char = S.chars.find((x) => x.id === id) ?? S.char;
}

function newCharacter(anchor, done) {
  let input;
  LoreShell.form({
    title: "新增人物",
    build: (body) => { input = h("input", { class: "set-input", "aria-label": "名稱", placeholder: "名稱", style: "width:100%" }); body.append(input); setTimeout(() => input.focus(), 0); },
    actions: [{ label: "取消" }, {
      label: "新增", primary: true,
      run: async () => {
        try {
          const d = await api("POST", "/api/characters", { name: input.value });
          await switchCharacter(d.id); done();
        } catch (e) { LoreShell.toast(e.message, { err: true }); return false; }
      },
    }],
  });
}

async function exportCharacter(c) {
  try {
    const r = await fetch(`/api/characters/${encodeURIComponent(c.id)}/export`, { cache: "no-store" });
    if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error || `${r.status}`);
    const a = h("a", { href: URL.createObjectURL(await r.blob()), download: `${c.id}.zip` });
    document.body.append(a); a.click(); a.remove();
    LoreShell.toast(`已匯出 ${c.id}.zip`);
  } catch (e) { LoreShell.toast(e.message, { err: true }); }
}

async function importCharacter(file, done) {
  try {
    const r = await fetch("/api/characters/import", { method: "POST", body: file });
    const d = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(d.error || `${r.status}`);
    await switchCharacter(d.id); done();
    LoreShell.toast(`已匯入「${d.name}」`);
  } catch (e) { LoreShell.toast(e.message, { err: true }); }
}
