// 收句子 (docs/brain-hub.md §16): the author reads English sentences aloud, or types them, one after
// another; each goes to L-Brain's inbox at once (server: collect.rs, POST /api/collect) and is
// worked on there. Nothing is answered here: he keeps talking. A line above the input says how
// many went and how many wait for him in L-Brain's 收件. 「好了」 or the ✕ ends it.
// send() hands the text here instead of the chat (like find.js); a finished utterance is sent
// at once while this is on (voice.js), whatever 自動送出 says.
"use strict";

window.LoreEchoCollect = (() => {
  const C = { on: false, bar: null, sent: 0, counts: {}, label: "句型", timer: 0 };
  const el = (tag, attrs = {}, ...kids) => h(tag, attrs, ...kids);

  function paint() {
    if (!C.bar) return;
    const c = C.counts ?? {}, working = (c.waiting ?? 0) + (c.working ?? 0);
    C.bar.replaceChildren(
      el("span", { class: "cl-what" }, `收句子 → ${C.label}`),
      el("span", { class: "cl-n" }, `已送 ${C.sent}`),
      working ? el("span", { class: "cl-n" }, `處理中 ${working}`) : null,
      c.ready ? el("span", { class: "cl-n hot" }, `待確認 ${c.ready}`) : null,
      el("button", { type: "button", class: "ico cl-x", "aria-label": "結束收句子", onclick: stop }, "✕"));
  }
  async function refresh() {
    clearTimeout(C.timer);
    if (!C.on) return;
    try { const v = await api("GET", "/api/collect"); C.counts = v.counts ?? {}; C.label = v.label || C.label; paint(); } catch { /* the server restarts */ }
    C.timer = setTimeout(refresh, 4000);
  }
  async function start() {
    if (C.on) return;
    let v;
    try { v = await api("GET", "/api/collect"); } catch (e) { LoreShell.toast(e.message, { err: true }); return; }
    if (!v.brain) { say("L-Brain 沒有開著，句子沒有地方去。先打開 L-Brain。"); return; }
    C.on = true; C.sent = 0; C.counts = v.counts ?? {}; C.label = v.label || "句型";
    C.bar = el("div", { class: "collect-bar", role: "status" });
    $("form").before(C.bar);
    paint();
    say("唸句子給我，說完一句就送去 L-Brain 處理。說「好了」結束。");
    $("text").placeholder = "唸一句英文，或打字…";
    refresh();
  }
  function stop() {
    if (!C.on) return;
    C.on = false; clearTimeout(C.timer);
    C.bar?.remove(); C.bar = null;
    $("text").placeholder = "說點什麼…";
    say(C.sent ? `送了 ${C.sent} 句，到 L-Brain 的「收件」確認。` : "收句子結束。");
  }
  /** While on, a sent text is a sentence for the inbox, never a chat turn. */
  function take(text) {
    if (!C.on) return false;
    const t = (text ?? "").trim();
    if (!t) return true;
    if (/^(好了|結束|停|ok|done|stop)[。.!！]?$/i.test(t)) { stop(); return true; }
    api("POST", "/api/collect", { text: t, via: window.LoreVoice?.lastWasSpoken?.() ? "voice" : "typed", own: true })
      .then((v) => { C.sent += v.added ?? 0; C.counts = v.counts ?? C.counts; paint(); if (!v.added) LoreShell.toast("這一句已經在排隊了"); })
      .catch((e) => { LoreShell.toast(e.message, { err: true }); });   // not kept anywhere: he sees it and says it again
    return true;
  }
  /** 收進句型 on a selected passage: the selection L-Echo holds goes to the inbox. */
  async function selection() {
    try { const v = await api("POST", "/api/collect/selection", { own: true }); LoreShell.toast(v.added ? `已送去 L-Brain 處理（${v.added} 句）` : "這一句已經在排隊了"); }
    catch (e) { LoreShell.toast(e.message, { err: true }); }
  }
  return { get on() { return C.on; }, start, stop, toggle: () => (C.on ? stop() : start()), take, selection };
})();
