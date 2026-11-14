// 搜尋檔案 (find mode): another Lore app asked for a file (server: find.rs, GET /api/find polled).
// The panel takes the history's place; the author describes the file by voice or typing (send()
// hands the text here instead of the chat), LoreEcho lists up to 5 candidates from LoreShelf.
// A click sends that file to the asking app at once (several may be clicked). Words: 「第二個」 or a
// part of the name highlights a row and asks 「加入這份？」 — only 對／加入 sends it; anything else is a
// new description. 「好了」, Esc or closing the window ends find mode. Nothing is opened here.
"use strict";

window.LoreEchoFind = (() => {
  const F = { seq: null, req: null, items: [], focus: -1, added: new Set(), panel: null, busy: 0 };
  const el = (tag, attrs = {}, ...kids) => h(tag, attrs, ...kids);
  const NUM = { 一: 1, 二: 2, 兩: 2, 三: 3, 四: 4, 五: 5, "１": 1, "２": 2, "３": 3, "４": 4, "５": 5 };

  async function poll() {
    try {
      const v = await api("GET", "/api/find");
      if (v.find && v.seq !== F.seq) begin(v.find);
      else if (!v.find && F.req) finish();
      F.seq = v.seq;
    } catch { /* the server restarts */ }
    setTimeout(poll, 1200);
  }

  function begin(r) {
    window.LoreEchoActions?.close();
    if (!$("choices").hidden) toggleChoices(false);
    F.req = r; F.items = []; F.focus = -1; F.added = new Set();
    if (!S.expanded) show(true);
    mount();
    say(`要找哪一份${r.purpose}？說說看或打字描述。`);
    $("text").focus();
  }

  function mount() {
    F.panel?.remove();
    $("hist").hidden = true;
    const done = el("button", { type: "button", class: "btn small", onclick: () => end() }, "好了");
    F.list = el("ol", { class: "find-list", "aria-label": "找到的檔案" });
    F.panel = el("div", { class: "find" },
      el("div", { class: "find-head" }, el("span", { class: "find-title" }, `找檔案 · ${F.req.purpose}`), done),
      F.list);
    document.querySelector("#win .card").append(F.panel);
    render();
  }

  function render() {
    if (!F.list) return;
    if (!F.items.length) { F.list.replaceChildren(); return; }
    F.list.replaceChildren(...F.items.map((it, i) => {
      const added = F.added.has(it.id);
      const pic = it.thumb ? el("img", { class: "find-thumb", alt: "", loading: "lazy", src: `/api/find/thumb/${encodeURIComponent(it.id)}` }) : null;
      if (pic) pic.onerror = () => pic.replaceWith(el("span", { class: "find-thumb ext" }, it.ext || "?"));
      const row = el("button", { type: "button", class: `find-row${i === F.focus ? " on" : ""}${added ? " added" : ""}`, "aria-pressed": i === F.focus ? "true" : "false", onclick: () => add(i) },
        el("span", { class: "find-n" }, added ? "✓" : String(i + 1)),
        pic ?? el("span", { class: "find-thumb ext" }, it.ext || "?"),
        el("span", { class: "find-text" },
          el("span", { class: "find-name" }, it.name),
          el("span", { class: "find-meta" }, [it.folder, it.date].filter(Boolean).join(" · "))),
        i === F.focus && !added ? el("span", { class: "find-add" }, "加入") : null);
      return el("li", {}, row);
    }));
    F.list.querySelector(".find-row.on")?.scrollIntoView({ block: "nearest" });
  }

  function say(text, look = "") { S.line = null; setLine(text, look); }

  /** A sentence typed or said while finding; false when not in find mode (it goes to the chat). */
  function take(text) {
    if (!F.req || !text) return false;
    const t = text.replace(/[。！!？?，,\s]+$/u, "").trim();
    if (/^(好了|沒了|結束|不用了|完成|就這樣|done)$/iu.test(t) && F.focus < 0) { end(); return true; }
    if (/好了|結束找|不用找了/u.test(t)) { end(); return true; }
    if (F.focus >= 0) {
      if (/^(對|對的|是|是的|好|好啊|嗯|沒錯|就是|就是這份|就這份|加入|加|要|yes|ok|y)$/iu.test(t)) { add(F.focus); return true; }
      if (/^(不是|不對|不要|錯了|no|n)$/iu.test(t)) { F.focus = -1; render(); say("那是哪一份？再說說看。"); return true; }
    }
    const n = ordinal(t);
    if (n != null && n < F.items.length) { highlight(n); return true; }
    const m = byName(t);
    if (m >= 0) { highlight(m); return true; }
    search(t);
    return true;
  }

  function ordinal(t) {
    if (/最後(一|那)?(個|份)/u.test(t) && F.items.length) return F.items.length - 1;
    const m = t.match(/第\s*([一二兩三四五1-5１-５])\s*(個|份|項|筆|行)?/u) || t.match(/^([1-5１-５])$/u);
    if (!m) return null;
    return (NUM[m[1]] ?? Number(m[1])) - 1;
  }

  /** The one listed row a part of the sentence names (「有 CheXzero 的那份」), else -1. */
  function byName(t) {
    if (!F.items.length) return -1;
    const words = t.toLowerCase().replace(/(有|的|那份|那個|這份|這個|檔案|那一份|就是|叫做|叫)/gu, " ").split(/[\s，,。、「」"'()（）]+/u).filter((w) => w.length >= 2);
    if (!words.length) return -1;
    const score = F.items.map((it) => words.filter((w) => it.name.toLowerCase().includes(w)).length);
    const best = Math.max(...score);
    if (!best || score.filter((s) => s === best).length !== 1) return -1;
    // only when the words mostly point at names: a fresh description searches again
    return best * 2 >= words.length ? score.indexOf(best) : -1;
  }

  function highlight(i) {
    F.focus = i; render();
    const it = F.items[i];
    say(F.added.has(it.id) ? `「${it.name}」已經加入了。` : `第 ${i + 1} 份「${it.name}」，加入這份？`);
  }

  async function search(q) {
    const my = ++F.busy;
    F.focus = -1;
    say("找找看", "pending");
    setState("searching");
    try {
      const v = await api("GET", `/api/find/search?q=${encodeURIComponent(q)}`);
      if (my !== F.busy || !F.req) return;
      F.items = v.items || [];
      render();
      say(F.items.length ? `找到 ${F.items.length} 份。點一下就加入，或說第幾個。` : "沒有找到，換個說法？");
    } catch (e) {
      if (my === F.busy) say(e.message, "err");
    } finally {
      if (my === F.busy && S.state === "searching") setState("idle");
    }
  }

  async function add(i) {
    const it = F.items[i];
    if (!it || !F.req) return;
    if (F.added.has(it.id)) { highlight(i); return; }
    try {
      await api("POST", "/api/find/pick", { id: it.id });
      F.added.add(it.id);
      F.focus = -1; render();
      say(`已加入${F.req.purpose}：${it.name}`);
      setState("happy");
      setTimeout(() => { if (S.state === "happy") setState("idle"); }, 2500);
    } catch (e) { say(e.message, "err"); }
  }

  async function end() {
    if (!F.req) return;
    finish();
    try { await api("POST", "/api/find/end"); } catch { /* the app stops asking after a while */ }
  }

  function finish() {
    const had = F.req;
    F.req = null; F.items = []; F.focus = -1; F.busy++;
    F.panel?.remove(); F.panel = null; F.list = null;
    $("hist").hidden = false;
    if (had && S.expanded) say("好，不找了。");
  }

  let started = false;
  return {
    /** app.js, once boot() has opened or collapsed the window (a waiting request must not be ended by it). */
    start: () => { if (!started) { started = true; poll(); } },
    take,
    active: () => !!F.req,
    escape: () => { if (!F.req) return false; end(); return true; },
    closed: () => { if (F.req) end(); },
  };
})();
