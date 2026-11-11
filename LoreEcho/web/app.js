// LoreEcho page: the bubble, the open window (character + conversation), memory only.
// Needs theme.js, icons.js and shell.js (LoreShell) first. No build step.
//
// Server: GET/PUT /api/window, GET/PUT /api/settings, GET /api/characters,
// /api/characters/<id>/avatar/<state> (portrait: the bubble), /api/characters/<id>/full/<state>
// (full body: the open window), POST /api/chat (server-sent events).
// In the desktop app, PUT /api/window { expanded } also resizes the window, and the page
// drags the window itself through Tauri (window.__TAURI__, withGlobalTauri).

"use strict";

const $ = (id) => document.getElementById(id);
const S = {
  desktop: false,
  chars: [],
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
    keys: [{ label: "換行", keys: "⇧↵" }, { label: "功能列表：上一個／下一個", keys: "↑ ↓" }],
  });
  const collapse = h("button", { type: "button", class: "ico", id: "actCollapse", "aria-label": "收起 Esc", title: "收起成泡泡（對話會清掉）" }, h("span", { "data-icon": "chevron" }));
  collapse.addEventListener("click", () => show(false));
  S.shell.el.prepend(collapse);
  S.shell.slots.lead.append(h("span", { class: "engine", id: "engine", hidden: true }));
}

// ── characters ──────────────────────────────────────────────────
async function loadCharacters() {
  const c = await api("GET", "/api/characters");
  S.chars = c.characters;
  S.char = S.chars.find((x) => x.id === c.current) ?? S.chars[0] ?? null;
  const name = S.char?.name ?? "LoreEcho";
  $("who").textContent = name;
  $("nametag").setAttribute("aria-label", `${name}（換人物）`);
  $("bubble").setAttribute("aria-label", `打開 LoreEcho（${name}）`);
  const f = S.char?.figure ?? {};
  $("win").style.setProperty("--fig-scale", f.scale ?? 1);
  $("win").style.setProperty("--fig-dy", f.offset_y ?? 0);
  $("figure").classList.toggle("portrait", !(S.char?.full?.length > 0));
  setState("idle");
  setImg($("bubbleImg"), $("bubbleIcon"), "avatar", "idle");
  // fetch the other states' full bodies now, so a state change does not flicker
  for (const st of ["listening", "thinking", "speaking"]) { const u = picUrl("full", st); if (u) new Image().src = u; }
}

/** The picture of `kind` (avatar = portrait, full = full body) for `state`, or null when the
 *  character has none. A missing state is idle's; a character without full bodies shows its
 *  portrait in the open window (the server's full/ route falls back the same way). */
function picUrl(kind, state) {
  const c = S.char;
  if (!c) return null;
  const useFull = kind === "full" && c.full?.length > 0;
  const own = (useFull ? c.full : c.avatars) ?? [];
  if (!own.length) return null;
  const s = own.includes(state) ? state : "idle";
  return `/api/characters/${encodeURIComponent(c.id)}/${useFull ? "full" : "avatar"}/${s}`;
}

/** Show the character's picture (the app icon when it has none at all). */
function setImg(img, icon, kind, state) {
  const url = picUrl(kind, state);
  img.hidden = !url; icon.hidden = !!url;
  if (url && img.dataset.src !== url) { img.dataset.src = url; img.src = url; }
}

/** idle | listening | thinking | speaking */
function setState(state) {
  $("figure").dataset.state = state;
  setImg($("full"), $("fullIcon"), "full", state);
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
  items.push("-", { icon: "gear", label: "人物設定…", run: () => LoreShell.settings("general") });
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
  S.msgs = [];
  S.line = null;
  $("hist").replaceChildren();
  setLine(GREETING, "empty");
  $("engine").hidden = true;
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
  setState("thinking");
  setLine("", "pending");
  let reply = "";
  try {
    const r = await fetch("/api/chat", {
      method: "POST", headers: { "content-type": "application/json" }, signal: ctrl.signal,
      body: JSON.stringify({ messages: S.msgs, character: S.char?.id, engine }),
    });
    if (!r.ok || !(r.headers.get("content-type") || "").startsWith("text/event-stream")) {
      const data = await r.json().catch(() => ({}));
      failed(data.error || `${r.status}`, data.alternative);
      return;
    }
    for await (const [ev, data] of events(r.body)) {
      if (ctrl.signal.aborted) return;
      if (ev === "start") { $("engine").textContent = data.label; $("engine").title = data.engine; $("engine").className = "engine chip"; $("engine").hidden = false; }
      else if (ev === "delta") { if (!reply) setState("speaking"); reply += data.text; setLine(reply, "pending"); }
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
    if (S.busy === ctrl) { S.busy = null; $("send").disabled = false; setState("idle"); }
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
  const sel = el.querySelector("#setCharacter");
  const key = el.querySelector("#setShortcut");
  for (const c of S.chars) sel.append(h("option", { value: c.id }, c.name === c.id ? c.id : `${c.name}（${c.id}）`));
  sel.value = S.char?.id ?? "";
  try { key.value = (await api("GET", "/api/settings")).shortcut ?? ""; } catch {}
  sel.addEventListener("change", () => switchCharacter(sel.value));
  key.addEventListener("change", async () => {
    try {
      const s = await api("PUT", "/api/settings", { shortcut: key.value });
      key.value = s.shortcut;
      LoreShell.toast(s.shortcut ? `快捷鍵：${s.shortcut}` : "不用快捷鍵");
    } catch (e) { LoreShell.toast(e.message, { err: true }); }
  });
}
