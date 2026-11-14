// 潤飾: a small conversation about a selected passage — say (hold the mic) or type how to
// rewrite it, see the rewrite as a word diff, 套用 / 再改 / 👍. Served as `/polish.js` by
// `lore_shell::FILES`; loaded after shell.js. The server side is lore-polish's `/api/polish*`
// (LaTeX locked, every proposal logged, liked rewrites into the tone library) and lore-voice's
// `/api/voice*`. An app mounts its defaults once and opens it on a selection:
//
//   LorePolish.mount({
//     doc: () => state.paper?.name,          // the document id the server knows
//     language: () => "en",                  // listening and the model's language (optional)
//     engine: () => chosenEngine(),          // the app's engine choice (optional)
//     onLike: (after, anchor) => {},         // after 👍 (PaperComposer: 收藏句型 on the rewrite)
//   });
// The app's presets (GET /api/polish/presets, e.g. PaperComposer's 用字 and 我的風格) show as
// buttons beside the free instruction; one pressed goes with every send, the typed words add to it.
//   LorePolish.open({
//     anchor,                                // an element or a DOMRect: where the dialog goes
//     text, prefix, suffix, context,         // the passage, the rest of its paragraph, nearby text
//     keep: [],                              // more literal strings the model must keep (optional)
//     scope: "",                             // where the passage sits (a section title; optional)
//     preset: "",                            // a preset id to start pressed (optional)
//     apply: async (after) => true,          // put it in as one undo step; a string = why not
//   });
"use strict";

window.LorePolish = (() => {
  let defaults = {};
  let card = null, cur = null, voiceOk = null, events = null, presetsP = null;

  function h(tag, attrs = {}, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v;
      else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat()) if (c != null && c !== false) e.append(c);
    return e;
  }
  const ib = (name, label, onclick, cls = "ico small") => {
    const b = h("button", { type: "button", class: cls, "aria-label": label, onclick });
    b.append(icon(name));
    return b;
  };

  async function call(path, body, method) {
    const r = await fetch(path, body === undefined ? { method: method ?? "GET" } : { method: method ?? "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    const t = await r.text();
    let v = null;
    try { v = t ? JSON.parse(t) : null; } catch {}
    if (!r.ok) throw new Error(v?.error ?? (t || String(r.status)));
    return v;
  }

  // ── word diff (CJK by character) ──
  const tokens = (s) => s.match(/\s+|[　-〿㐀-鿿豈-﫿＀-￯]|[^\s　-〿㐀-鿿豈-﫿＀-￯]+/g) ?? [];
  function diff(a, b) {
    const x = tokens(a), y = tokens(b);
    if (x.length * y.length > 400000) return [["del", a], ["ins", b]];
    const n = x.length, m = y.length;
    const L = Array.from({ length: n + 1 }, () => new Uint16Array(m + 1));
    for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) L[i][j] = x[i] === y[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
    const out = [];
    const push = (k, t) => { const l = out[out.length - 1]; if (l && l[0] === k) l[1] += t; else out.push([k, t]); };
    let i = 0, j = 0;
    while (i < n && j < m) {
      if (x[i] === y[j]) { push("same", x[i]); i++; j++; }
      else if (L[i + 1][j] >= L[i][j + 1]) push("del", x[i++]);
      else push("ins", y[j++]);
    }
    while (i < n) push("del", x[i++]);
    while (j < m) push("ins", y[j++]);
    return out;
  }
  function diffNode(a, b) {
    const box = h("div", { class: "lp-text" });
    for (const [k, t] of diff(a, b)) box.append(k === "same" ? t : h(k, { class: `lp-${k}` }, t));
    return box;
  }

  const CSS = `
.lp-card { position: fixed; z-index: 70; width: 440px; max-width: calc(100vw - 16px); max-height: calc(100vh - 16px); overflow: auto; background: var(--panel); color: var(--text); border: 1px solid var(--line-strong); border-top: 2px solid var(--accent-text); border-radius: var(--r); box-shadow: 0 10px 30px rgba(0,0,0,.18); padding: 10px 12px 12px; display: grid; gap: 8px; font-size: 13px; }
.lp-head { display: flex; align-items: center; gap: 6px; font-weight: 600; }
.lp-head svg.i { color: var(--accent-text); width: 16px; height: 16px; }
.lp-head .lp-grow { flex: 1; }
.lp-text { white-space: pre-wrap; line-height: 1.55; padding: 8px 10px; border: 1px solid var(--line); border-radius: var(--r); background: var(--surface); max-height: 40vh; overflow: auto; overflow-wrap: anywhere; }
.lp-ins { background: var(--accent-soft); color: var(--accent-text); text-decoration: none; border-radius: 2px; }
.lp-del { color: var(--err); text-decoration: line-through; }
.lp-problems { color: var(--err); font-size: 12px; display: grid; gap: 2px; }
.lp-hist { display: flex; flex-wrap: wrap; gap: 4px; align-items: center; }
.lp-hist button { font: 500 11px var(--mono); min-width: 24px; height: 22px; padding: 0 6px; border: 1px solid var(--line-strong); border-radius: var(--r); background: var(--surface); color: var(--muted); }
.lp-hist button:hover { color: var(--text); background: var(--surface-hover); }
.lp-hist button[aria-pressed="true"] { color: var(--text); border-color: var(--accent-text); background: var(--accent-soft); }
.lp-hist button.applied { border-style: dashed; }
.lp-presets { display: flex; flex-wrap: wrap; gap: 6px; }
.lp-presets button[aria-pressed="true"] { color: var(--text); border-color: var(--accent-text); background: var(--accent-soft); }
.lp-ask { display: flex; gap: 6px; align-items: flex-end; }
.lp-ask textarea { flex: 1; min-height: 34px; max-height: 120px; resize: vertical; font: inherit; padding: 6px 8px; border: 1px solid var(--line-strong); border-radius: var(--r); background: var(--bg); color: var(--text); }
.lp-ask .lp-mic.on { color: var(--accent-text); background: var(--accent-soft); }
.lp-foot { display: flex; gap: 6px; align-items: center; justify-content: flex-end; }
.lp-foot .lp-like[aria-pressed="true"] { color: var(--accent-text); background: var(--accent-soft); }
.lp-busy { height: 3px; border-radius: 2px; background: var(--accent-soft); overflow: hidden; }
.lp-busy::before { content: ""; display: block; width: 35%; height: 100%; background: var(--accent-text); animation: lp-run 1.4s ease-in-out infinite; }
@keyframes lp-run { from { transform: translateX(-100%); } to { transform: translateX(290%); } }
@media (prefers-reduced-motion: reduce) { .lp-busy::before { animation: none; width: 100%; opacity: .5; } }
`;
  function style() {
    if (document.getElementById("lore-polish-css")) return;
    document.head.append(h("style", { id: "lore-polish-css" }, CSS));
  }

  // the language to listen in: the document's, else the passage's own (mostly Han → zh-TW)
  const listenLang = (l, text = "") => {
    if (/^zh/i.test(l ?? "")) return "zh-TW";
    if (/^en/i.test(l ?? "")) return "en-US";
    if (l) return l;
    const han = (text.match(/[\u3400-\u9fff]/g) ?? []).length;
    return han * 3 > text.replace(/\s/g, "").length ? "zh-TW" : "en-US";
  };

  function place(anchor) {
    if (!card) return;
    const r = anchor?.getBoundingClientRect ? anchor.getBoundingClientRect() : anchor ?? { left: innerWidth / 2, right: innerWidth / 2, top: 80, bottom: 80 };
    const w = card.offsetWidth, ht = card.offsetHeight;
    const below = r.bottom + 8 + ht < innerHeight - 8;
    card.style.top = `${Math.max(8, below ? r.bottom + 8 : Math.max(8, r.top - 8 - ht))}px`;
    card.style.left = `${Math.max(8, Math.min(r.left, innerWidth - w - 8))}px`;
  }

  function close() {
    if (!card) return;
    if (cur?.listening) call("/api/voice/listen/stop", {}).catch(() => {});
    events?.close(); events = null;
    card.remove(); card = null; cur = null;
    document.removeEventListener("keydown", esc, true);
  }
  const esc = (e) => { if (e.key === "Escape" && card) { e.preventDefault(); e.stopPropagation(); close(); } };

  /** The current proposal on screen: the diff, problems, the buttons. */
  function show() {
    const c = cur;
    const p = c.shown;
    c.view.replaceChildren(p ? diffNode(c.text, p.after) : h("div", { class: "lp-text" }, c.text));
    c.problems.replaceChildren(...(p?.problems ?? []).map((m) => h("div", {}, m)));
    c.foot.hidden = !p;
    c.applyBtn.disabled = !p || !p.ok;
    c.applyBtn.title = p && !p.ok ? (p.problems ?? []).join("\n") : "";
    c.likeBtn.setAttribute("aria-pressed", String(!!p?.liked));
    c.hist.replaceChildren(...c.items.map((it, k) => {
      const b = h("button", { type: "button", "aria-pressed": String(it === p), class: it.applied ? "applied" : null, title: it.instruction, onclick: () => { c.shown = it; show(); } }, String(k + 1));
      return b;
    }));
    c.hist.hidden = !c.items.length || (c.items.length === 1 && c.items[0] === p);
  }

  function busy(on) {
    cur.busy.hidden = !on;
    cur.send.disabled = on;
  }

  async function send() {
    const c = cur;
    const instruction = c.input.value.trim();
    const preset = c.presets.find((x) => x.id === c.preset);
    if (!instruction && !preset) { c.input.focus(); return; }
    // what was asked, as the author sees it (a preset by its label)
    const asked = preset ? `「${preset.label}」${instruction}` : instruction;
    busy(true);
    try {
      const p = await call("/api/polish", {
        doc: c.doc, text: c.text, prefix: c.o.prefix ?? "", suffix: c.o.suffix ?? "", context: c.o.context ?? "",
        language: c.lang, instruction, turns: c.turns, passage: c.passage, engine: defaults.engine?.() ?? undefined, keep: c.o.keep ?? [],
        preset: preset?.id ?? null, scope: c.o.scope ?? "",
      });
      if (cur !== c) return;
      c.passage = p.passage;
      const item = { ...p, instruction: asked, liked: false, applied: false };
      c.turns.push({ instruction: asked, after: p.after });
      c.items.push(item);
      c.shown = item;
      c.input.value = "";
      show();
    } catch (e) {
      LoreShell.toast(e.message, { err: true, ms: 6000 });
    } finally { if (cur === c) busy(false); }
  }

  async function apply() {
    const c = cur, p = c?.shown;
    if (!p?.ok) return;
    const r = await c.o.apply(p.after);
    if (r !== true) { if (r) LoreShell.toast(String(r), { err: true, ms: 6000 }); return; }
    call("/api/polish/applied", { doc: c.doc, id: p.id }).catch(() => {});
    close();
  }

  async function like() {
    const c = cur, p = c?.shown;
    if (!p || p.liked) return;
    try {
      await call("/api/polish/like", { doc: c.doc, id: p.id });
      p.liked = true;
      show();
      defaults.onLike?.(p.after, c.likeBtn);
    } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
  }

  // ── hold to talk ──
  async function voice() {
    if (voiceOk == null) {
      try { voiceOk = !!(await call("/api/voice")).available; } catch { voiceOk = false; }
    }
    return voiceOk;
  }
  function listenTo() {
    if (events) return;
    events = new EventSource("/api/voice/events");
    const text = (e) => { try { return JSON.parse(e.data).text ?? ""; } catch { return ""; } };
    events.addEventListener("partial", (e) => { if (cur?.listening) cur.input.value = cur.base + text(e); });
    events.addEventListener("final", (e) => { if (!cur) return; cur.input.value = (cur.base + text(e)).trimStart(); cur.input.focus(); });
    events.addEventListener("error", (e) => {
      let v = {};
      try { v = JSON.parse(e.data ?? "{}"); } catch {}
      if (!v.kind) return; // the stream itself closing
      if (v.kind === "microphone" || v.kind === "speech" || v.kind === "unavailable") { voiceOk = false; if (cur) cur.mic.hidden = true; }
      // a refused permission is changed in System Settings, never asked again
      const perm = v.kind === "microphone" || v.kind === "speech" ? v.kind : null;
      LoreShell.toast(v.message ?? "語音無法使用", { err: true, action: perm && LoreShell.permissions ? { label: "開啟系統設定", run: () => LoreShell.permissions.open(perm) } : undefined });
    });
  }
  async function talk(on) {
    const c = cur;
    if (!c) return;
    if (on) {
      listenTo();
      c.base = c.input.value ? `${c.input.value.trimEnd()} ` : "";
      c.listening = true;
      c.mic.classList.add("on");
      try { await call("/api/voice/listen", { continuous: false, language: listenLang(c.lang, c.text) }); }
      catch (e) { c.listening = false; c.mic.classList.remove("on"); voiceOk = false; c.mic.hidden = true; LoreShell.toast(e.message, { err: true }); }
    } else if (c.listening) {
      c.listening = false;
      c.mic.classList.remove("on");
      call("/api/voice/listen/stop", {}).catch(() => {});
    }
  }

  /** The app's presets, asked once per page. */
  function presets() {
    presetsP ??= call("/api/polish/presets").then((v) => (Array.isArray(v) ? v : [])).catch(() => []);
    return presetsP;
  }
  function drawPresets(c) {
    c.presetBox.replaceChildren(...c.presets.map((x) => h("button", {
      type: "button", class: "btn small", "aria-pressed": String(c.preset === x.id),
      onclick: () => { c.preset = c.preset === x.id ? null : x.id; drawPresets(c); c.input.focus(); },
    }, x.label)));
    c.presetBox.hidden = !c.presets.length;
  }

  async function open(o) {
    close();
    style();
    const doc = o.doc ?? defaults.doc?.();
    if (!doc || !o.text?.trim()) { LoreShell.toast("先選取要潤飾的文字"); return; }
    const c = cur = { o, doc, text: o.text, lang: o.language ?? defaults.language?.() ?? "", turns: [], items: [], shown: null, passage: null, presets: [], preset: o.preset || null };
    c.presetBox = h("div", { class: "lp-presets", hidden: true });
    c.view = h("div");
    c.problems = h("div", { class: "lp-problems" });
    c.hist = h("div", { class: "lp-hist" });
    c.input = h("textarea", { rows: "2", placeholder: "怎麼改？", "aria-label": "怎麼改" });
    c.input.addEventListener("keydown", (e) => {
      if (e.isComposing) return;
      if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); }
    });
    c.mic = ib("mic", "按住說話", null, "ico small lp-mic");
    c.mic.hidden = true;
    c.mic.addEventListener("pointerdown", (e) => { e.preventDefault(); c.mic.setPointerCapture?.(e.pointerId); talk(true); });
    for (const ev of ["pointerup", "pointercancel"]) c.mic.addEventListener(ev, () => talk(false));
    c.send = ib("send", "送出 ↵", send);
    c.busy = h("div", { class: "lp-busy", hidden: true });
    c.applyBtn = h("button", { type: "button", class: "btn small primary", onclick: apply }, "套用");
    c.likeBtn = ib("thumbUp", "讚", like, "ico small lp-like");
    c.foot = h("div", { class: "lp-foot", hidden: true },
      c.likeBtn,
      h("button", { type: "button", class: "btn small", onclick: () => c.input.focus() }, "再改"),
      c.applyBtn);
    card = h("div", { class: "lp-card", role: "dialog", "aria-label": "潤飾" },
      h("div", { class: "lp-head" }, icon("polish"), h("span", {}, "潤飾"), h("span", { class: "lp-grow" }), ib("x", "關閉 Esc", close)),
      c.view, c.problems, c.hist, c.presetBox,
      h("div", { class: "lp-ask" }, c.input, c.mic, c.send),
      c.busy, c.foot);
    document.body.append(card);
    document.addEventListener("keydown", esc, true);
    show();
    place(o.anchor);
    c.input.focus();
    // earlier proposals of this passage, and the mic when there is a voice
    voice().then((ok) => { if (cur === c) c.mic.hidden = !ok; });
    presets().then((list) => {
      if (cur !== c) return;
      c.presets = list;
      if (c.preset && !list.some((x) => x.id === c.preset)) c.preset = null;
      drawPresets(c);
      place(o.anchor);
      // opened from a toolbar preset (用字 / 我的風格): that is the request, run it now
      if (c.preset && o.preset) send();
    });
    try {
      const r = await call(`/api/polish/history?doc=${encodeURIComponent(doc)}&text=${encodeURIComponent(o.text)}`);
      if (cur !== c || c.items.length) return;
      c.passage = r.passage;
      c.items = (r.items ?? []).map((x) => ({ ...x, ok: !(x.problems ?? []).length }));
      show();
      place(o.anchor);
    } catch {}
  }

  function mount(o = {}) { defaults = { ...defaults, ...o }; }

  return { mount, open, close, diff, isOpen: () => !!card };
})();
