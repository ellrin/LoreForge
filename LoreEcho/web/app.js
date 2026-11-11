// LoreEcho page: the bubble, the panel, the conversation (memory only).
// Needs theme.js, icons.js and shell.js (LoreShell) first. No build step.
//
// Server: GET/PUT /api/window, GET/PUT /api/settings, GET /api/characters,
// /api/characters/<id>/avatar/<state>, POST /api/chat (server-sent events).
// In the desktop app, PUT /api/window { expanded } also resizes the window, and the page
// drags the window itself through Tauri (window.__TAURI__, withGlobalTauri).

"use strict";

const $ = (id) => document.getElementById(id);
const S = {
  desktop: false,
  chars: [],
  char: null,          // { id, name, avatars }
  msgs: [],            // [{ role, text }] — never saved
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
    app: "le", brand: "LORE/ECHO", host: "#panel header.top",
    primary: [{ id: "actNew", icon: "plus", label: "新對話", run: newChat }],
    general: buildGeneral,
    keys: [{ label: "換行", keys: "⇧↵" }],
  });
  const collapse = h("button", { type: "button", class: "ico", id: "actCollapse", "aria-label": "收起 Esc", title: "收起成泡泡（對話會清掉）" }, h("span", { "data-icon": "chevron" }));
  collapse.addEventListener("click", () => show(false));
  S.shell.el.prepend(collapse);
  S.shell.slots.lead.append(h("span", { class: "who", id: "who", "data-tauri-drag-region": true }));
}

// ── characters ──────────────────────────────────────────────────
async function loadCharacters() {
  const c = await api("GET", "/api/characters");
  S.chars = c.characters;
  S.char = S.chars.find((x) => x.id === c.current) ?? S.chars[0] ?? null;
  $("who").textContent = S.char?.name ?? "LoreEcho";
  $("bubble").setAttribute("aria-label", `打開 LoreEcho（${S.char?.name ?? ""}）`);
  setState("idle");
  setImg($("bubbleImg"), $("bubbleIcon"), "idle");
}

const avatarUrl = (state) => `/api/characters/${encodeURIComponent(S.char.id)}/avatar/${state}`;

/** Show the character's picture for `state` (idle's when it has none; the icon when it has none at all). */
function setImg(img, icon, state) {
  const has = S.char?.avatars?.length > 0;
  img.hidden = !has; icon.hidden = has;
  if (!has) return;
  const s = S.char.avatars.includes(state) ? state : "idle";
  const url = avatarUrl(s);
  if (img.dataset.src !== url) { img.dataset.src = url; img.src = url; }
}

/** idle | listening | thinking | speaking */
function setState(state) {
  $("stage").dataset.state = state;
  setImg($("avatar"), $("avatarIcon"), state);
}

// ── bubble ⇄ panel ──────────────────────────────────────────────
async function show(expanded, save = true) {
  if (!expanded && S.expanded) {
    newChat();                                   // closing the panel ends the conversation (design §2)
    const p = $("panel");
    p.classList.remove("opening"); p.classList.add("closing");
    await new Promise((r) => setTimeout(r, matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 120));
    p.classList.remove("closing");
  }
  S.expanded = expanded;
  if (save) {
    try { await api("PUT", "/api/window", { expanded }); }
    catch (e) { if (expanded) LoreShell.toast(e.message, { err: true }); }
  }
  $("panel").hidden = !expanded;
  $("bubble").hidden = expanded;
  if (expanded) {
    const p = $("panel");
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

  $("form").addEventListener("submit", (e) => { e.preventDefault(); send(); });
  const t = $("text");
  t.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing && e.keyCode !== 229) { e.preventDefault(); send(); }
  });
  t.addEventListener("input", grow);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && S.expanded && !document.querySelector("dialog[open]") && !document.querySelector(".menu, .ask")) { e.preventDefault(); show(false); }
  });
}

function grow() {
  const t = $("text");
  t.style.height = "auto";
  t.style.height = `${Math.min(t.scrollHeight, 120)}px`;
}

// ── the conversation ────────────────────────────────────────────
function scrollDown() { const m = $("msgs"); m.scrollTop = m.scrollHeight; }

function addMsg(role, text) {
  $("empty").hidden = true;
  const li = h("li", { class: `msg ${role}` }, text);
  $("msgs").append(li);
  scrollDown();
  return li;
}

function newChat() {
  S.busy?.abort();
  S.busy = null;
  S.msgs = [];
  $("msgs").replaceChildren($("empty"));
  $("empty").hidden = false;
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
    S.msgs.push({ role: "user", text });
    addMsg("user", text);
  }
  const ctrl = new AbortController();
  S.busy = ctrl;
  $("send").disabled = true;
  setState("thinking");
  const li = addMsg("assistant pending", "");
  let reply = "";
  try {
    const r = await fetch("/api/chat", {
      method: "POST", headers: { "content-type": "application/json" }, signal: ctrl.signal,
      body: JSON.stringify({ messages: S.msgs, character: S.char?.id, engine }),
    });
    if (!r.ok || !(r.headers.get("content-type") || "").startsWith("text/event-stream")) {
      const data = await r.json().catch(() => ({}));
      li.remove();
      failed(data.error || `${r.status}`, data.alternative);
      return;
    }
    for await (const [ev, data] of events(r.body)) {
      if (ctrl.signal.aborted) return;
      if (ev === "start") { $("engine").textContent = data.label; $("engine").title = data.engine; $("engine").className = "engine chip"; $("engine").hidden = false; }
      else if (ev === "delta") { if (!reply) setState("speaking"); reply += data.text; li.textContent = reply; scrollDown(); }
      else if (ev === "done") { reply = data.text || reply; li.textContent = reply; }
      else if (ev === "error") { if (!reply) li.remove(); failed(data.error); return; }
    }
    li.classList.remove("pending");
    if (reply.trim()) S.msgs.push({ role: "assistant", text: reply });
    else li.remove();
  } catch (e) {
    if (ctrl.signal.aborted) return;
    if (!reply) li.remove();
    failed(e.message);
  } finally {
    li.classList.remove("pending");
    if (S.busy === ctrl) { S.busy = null; $("send").disabled = false; setState("idle"); }
  }
}

/** One error line in the conversation, with 「改用本機／改用 Claude」 when the server offers it. */
function failed(msg, alt) {
  const li = addMsg("err", msg);
  if (alt?.engine) {
    li.append(h("button", { type: "button", class: "btn", onclick: () => { li.remove(); send(alt.engine); } }, alt.label || "改用另一個"));
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
  sel.addEventListener("change", async () => {
    try {
      await api("PUT", "/api/settings", { character: sel.value });
      await loadCharacters();
      newChat();
      LoreShell.toast(`換成「${S.char?.name}」`);
    } catch (e) { LoreShell.toast(e.message, { err: true }); }
  });
  key.addEventListener("change", async () => {
    try {
      const s = await api("PUT", "/api/settings", { shortcut: key.value });
      key.value = s.shortcut;
      LoreShell.toast(s.shortcut ? `快捷鍵：${s.shortcut}` : "不用快捷鍵");
    } catch (e) { LoreShell.toast(e.message, { err: true }); }
  });
}
