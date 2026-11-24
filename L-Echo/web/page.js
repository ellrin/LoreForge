// 頁面修改 (page mode): another Lore app handed over one or two pages of a deck (server: page.rs,
// GET /api/page polled). The panel takes the history's place: which page(s), then the result as
// before / after pictures with what the check found, and 套用 · 再改 · 復原. He says what to change
// in the dialogue box's own field (typed or dictated; send() hands the sentence here, never to the
// chat). The edit is the asking app's (a model that also looks at the rendered page), proxied under
// /api/page/edit; the model is the header menu's own choice for this job (S.models.page, Claude only).
// A sentence sent while a proposal is shown and not applied revises that proposal; after 套用 the
// next sentence is a new edit of the same page(s).
// Esc: the larger picture, then the wait, then a pending 再改, then page mode itself.
// 新對話, collapse or a text selection end it.
"use strict";

window.LoreEchoPage = (() => {
  const P = {
    req: null, ended: null, panel: null, body: null, notes: null, foot: null, scope: null, zoom: null,
    phase: "idle",       // idle | result | applied
    res: null,           // the proposal shown: { id, before, after, findings, warnings }
    ver: 0,              // bumped per answer: a revision keeps its picture URLs
    said: "",            // his last instruction (back in the field when the proposal dies)
    side: "after",       // the narrow layout's 前／後
    armed: false,        // 再改 was pressed
    wait: null,          // { ctrl, t0, tick } while an edit runs
    busy: false,         // 套用 / 復原 running
    seq: 0, prev: null, mic: null,
  };
  const box = () => $("box");
  const HOLD = (n) => (n > 1 ? "怎麼改這兩頁？" : "怎麼改這一頁？");

  /** fetch with the status kept (409 and 410 mean different things here). */
  async function call(path, body, signal) {
    const r = await fetch(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body ?? {}), cache: "no-store", signal });
    const data = await r.json().catch(() => ({}));
    return { ok: r.ok, status: r.status, data, error: data.error || `${r.status}` };
  }

  async function poll() {
    try {
      const v = await api("GET", "/api/page");
      // (an ended request may still be on the server for a moment: not a new one)
      if (v.open && v.request !== P.req?.request) { if (v.request !== P.ended) begin(v); }
      else if (!v.open && P.req) finish();
      else if (v.open && JSON.stringify(v.slides) !== JSON.stringify(P.req.slides)) { Object.assign(P.req, v); paintScope(); }   // an applied split renumbered them
    } catch { /* the server restarts */ }
    setTimeout(poll, 1200);
  }

  function begin(r) {
    finish();
    window.LoreEchoActions?.close();
    if (window.LoreEchoSel?.active()) window.LoreEchoSel.end();   // the server already dropped it
    if (!$("choices").hidden) toggleChoices(false);
    P.req = r;
    P.prev = { text: $("line").textContent, cls: $("line").className, line: S.line };
    if (!S.expanded) show(true);
    mount();
    S.chip?.actual("");                            // the chip is this job's model now
    $("text").focus();
  }

  function mount() {
    $("hist").hidden = true;
    P.scope = h("div", { class: "pg-scope" });
    const flip = h("div", { class: "seg pg-flip", role: "group", "aria-label": "顯示" },
      [["before", "前"], ["after", "後"]].map(([id, t]) => h("button", { type: "button", "data-side": id, onclick: () => { P.side = id; paintSide(); } }, t)));
    const close = h("button", { type: "button", class: "ico small", "aria-label": "結束 Esc", title: "結束 Esc", onclick: () => end() }, h("span", { "data-icon": "x" }));
    P.body = h("div", { class: "pg-body" });
    P.notes = h("div", { class: "pg-notes" });
    P.foot = h("div", { class: "pg-foot" });
    P.panel = h("div", { class: "pg", id: "pg" }, h("div", { class: "pg-head" }, P.scope, flip, close), P.body, P.notes, P.foot);
    document.querySelector("#win .card").append(P.panel);
    box().classList.add("pg-on");
    paintIcons(P.panel);
    paintScope();
    render();
  }

  function paintScope() {
    const s = P.req.slides ?? [];
    const titles = s.map((x) => (x.title ?? "").trim()).filter(Boolean);
    const text = `第 ${s.map((x) => x.n).join("、")} 頁${titles.length ? `：${titles.join(" · ")}` : ""}`;
    P.scope.textContent = text;
    P.scope.title = [P.req.source, text].filter((x) => x && x.trim()).join("\n");
    hold();
  }
  function hold() { $("text").placeholder = P.armed ? "再改什麼？" : HOLD(P.req?.slides?.length ?? 1); }

  /** A picture of the proposal, through L-Echo: the last segment of the app's URL. */
  const src = (url) => `/api/page/edit/${encodeURIComponent(P.res.id)}/image/${encodeURIComponent(String(url).split("?")[0].split("/").pop())}?v=${P.ver}`;

  /** Which page a finding is about, as the author counts them. */
  function pageOf(f) {
    const s = P.req.slides.find((x) => x.id === f.slide);
    if (s) return `第 ${s.n} 頁`;
    if (typeof f.slide === "number") return `第 ${f.slide} 頁`;
    return f.slide ? String(f.slide) : "";
  }

  function render() {
    if (!P.panel) return;
    const r = P.res;
    P.panel.classList.toggle("full", !!r);
    P.panel.dataset.phase = P.phase;
    if (!r) { P.body.replaceChildren(); P.notes.replaceChildren(); P.foot.replaceChildren(); P.foot.hidden = true; return; }
    const stack = (side, label, urls) => h("div", { class: "pg-stack", "data-side": side },
      h("span", { class: "pg-cap" }, label),
      h("div", { class: "pg-set" }, (urls ?? []).map((u) => h("button", { type: "button", class: "pg-pic", "aria-label": `${label}（放大）`, onclick: () => zoom(src(u), label) }, h("img", { src: src(u), alt: label, draggable: "false" })))));
    const finds = (r.findings ?? []).filter((f) => f?.text);
    P.body.replaceChildren(h("div", { class: "pg-pics" }, stack("before", "前", r.before), stack("after", "後", r.after)));
    // under the pictures and outside their scroller: what the check found is always in sight
    P.notes.replaceChildren(...[
      finds.length ? h("ul", { class: "pg-finds", "aria-label": "檢查發現的問題" }, finds.map((f) => h("li", { title: f.kind || null }, h("span", { "data-icon": "alert" }), h("span", {}, pageOf(f) ? h("b", {}, pageOf(f)) : null, f.text)))) : null,
      ...(r.warnings ?? []).filter(Boolean).map((w) => h("div", { class: "pg-warn" }, w)),
    ].filter(Boolean));
    const applied = P.phase === "applied";
    P.foot.hidden = false;
    P.foot.replaceChildren(...[
      applied && r.undo !== false ? h("button", { type: "button", class: "btn small", "data-act": "undo", onclick: undo }, h("span", { "data-icon": "undo" }), "復原") : null,
      h("span", { class: "pg-gap", title: (applied && r.message) || null }, applied ? r.message : null),   // the app's own words, beside 已套用 (no toast: it would cover the field)
      applied ? null : h("button", { type: "button", class: "btn small", "data-act": "again", "aria-pressed": String(P.armed), onclick: again }, "再改"),
      h("button", { type: "button", class: "btn small primary", "data-act": "apply", disabled: applied, onclick: apply }, applied ? "已套用" : "套用"),
    ].filter(Boolean));
    paintIcons(P.panel);
    paintSide();
    lock();
  }
  function paintSide() {
    if (!P.panel) return;
    P.panel.dataset.side = P.side;
    for (const b of P.panel.querySelectorAll(".pg-flip button")) b.setAttribute("aria-pressed", String(b.dataset.side === P.side));
  }
  /** Nothing is pressed while an edit, 套用 or 復原 runs. */
  function lock() {
    const on = !!P.wait || P.busy;
    for (const b of P.foot?.querySelectorAll(".btn") ?? []) b.disabled = on || (b.dataset.act === "apply" && P.phase === "applied");
  }

  // ── the dialogue line: only progress and problems ──
  function say(text, look = "") {
    S.line = null;
    box().classList.toggle("pg-say", !!text);
    setLine(text, look);
  }

  /** send() asks: in page mode the sentence is the instruction (true = taken, the chat gets nothing). */
  function take() {
    if (!P.req) return false;
    if (P.wait || P.busy) return true;             // one thing at a time; what he typed stays
    const text = $("text").value.trim();
    if (text) run(text);
    return true;
  }

  async function run(instruction) {
    const my = ++P.seq, req = P.req;
    const revise = P.phase === "result" && P.res?.id;   // a proposal he has not applied: the same one, changed
    P.said = instruction;
    $("text").value = ""; grow();
    arm(false);
    const ctrl = new AbortController();
    const t0 = Date.now();
    const tell = () => say(`修改中 · ${Math.round((Date.now() - t0) / 1000)} 秒`);
    P.wait = { ctrl, tick: setInterval(() => { tell(); if (Date.now() - t0 > 8000 && S.state === "thinking") setState("waiting"); }, 1000) };
    fields(false);
    lock();
    setState("thinking");
    tell();
    let out = null;
    try {
      out = revise
        ? await call(`/api/page/edit/${encodeURIComponent(P.res.id)}/revise`, { instruction }, ctrl.signal)
        : await call("/api/page/edit", { instruction, engine: S.models.page || undefined }, ctrl.signal);
    } catch (e) {
      out = ctrl.signal.aborted ? null : { ok: false, error: e.message };
    }
    if (my !== P.seq || P.req !== req) return;      // cancelled, or page mode moved on: a late answer is dropped
    settle();
    if (!out) return;
    if (!out.ok || !out.data.id) { back(out.error); return; }
    P.res = out.data; P.ver++;
    P.phase = "result"; P.side = "after";
    say("");
    render();
    P.body.scrollTop = 0;
  }

  /** The wait is over (answer, problem or Esc): the field is his again. */
  function settle() {
    if (!P.wait) return;
    clearInterval(P.wait.tick);
    P.wait = null;
    fields(true);
    lock();
    if (S.state === "thinking" || S.state === "waiting") setState("idle");
    $("text").focus();
  }

  /** A problem: said in the line, his words back in the field. */
  function back(message) {
    say(message, "err");
    if (!$("text").value.trim() && P.said) { $("text").value = P.said; grow(); }
  }

  /** Esc while waiting: the page stops waiting. The server call is abandoned, not stopped: the app
   *  may still finish the proposal, which nobody asks for again. */
  function cancel() {
    if (!P.wait) return;
    P.seq++;
    P.wait.ctrl.abort();
    settle();
    say("");
    if (!$("text").value.trim() && P.said) { $("text").value = P.said; grow(); }
  }

  function fields(on) {
    $("text").disabled = !on; $("send").disabled = !on;
    // (voice.js repaints the mic by itself; take() ignores a sentence that arrives meanwhile)
    if (!on) { P.mic = $("mic").disabled; $("mic").disabled = true; } else if (P.mic != null) { $("mic").disabled = P.mic; P.mic = null; }
  }

  function arm(on) {
    P.armed = !!on;
    P.foot?.querySelector('[data-act="again"]')?.setAttribute("aria-pressed", String(P.armed));
    hold();
  }
  function again() { arm(true); $("text").focus(); }

  async function apply() {
    if (P.phase !== "result" || P.busy || P.wait) return;
    const req = P.req, id = P.res.id;
    P.busy = true; lock(); arm(false);
    let out;
    try { out = await call(`/api/page/edit/${encodeURIComponent(id)}/apply`); } catch (e) { out = { ok: false, status: 0, error: e.message }; }
    if (P.req !== req) return;
    P.busy = false;
    if (out.ok) {
      P.phase = "applied";
      say("");
      P.res.message = out.data.message || "";
      render();
    } else if (out.status === 410) {               // the app closed the request: page mode is over
      P.ended = req.request;
      finish();
      S.line = null; setLine(out.error, "err");
    } else if (out.status === 409) {               // the slides changed meanwhile: this proposal is dead
      P.res = null; P.phase = "idle";
      render();
      back(out.error);
      $("text").focus();
    } else { lock(); say(out.error, "err"); }
  }

  async function undo() {
    if (P.phase !== "applied" || P.busy || P.wait) return;
    const req = P.req, id = P.res.id;
    P.busy = true; lock();
    let out;
    try { out = await call(`/api/page/edit/${encodeURIComponent(id)}/undo`); } catch (e) { out = { ok: false, status: 0, error: e.message }; }
    if (P.req !== req) return;
    P.busy = false;
    if (out.ok) {                                  // the pages are as before: his words are his to say again
      P.res = null; P.phase = "idle";
      say("");
      render();
      if (!$("text").value.trim() && P.said) { $("text").value = P.said; grow(); }
      $("text").focus();
    } else {
      if (out.status === 409) P.res.undo = false;  // something else was edited since: no undo from here
      render();
      say(out.error, "err");
    }
  }

  // ── a picture, larger, inside the window ──
  function zoom(url, label) {
    unzoom();
    P.zoom = h("div", { class: "pg-zoom", role: "dialog", "aria-label": label, onclick: unzoom }, h("img", { src: url, alt: label, draggable: "false" }));
    $("win").append(P.zoom);
  }
  function unzoom() { P.zoom?.remove(); P.zoom = null; }

  async function end() {
    if (!P.req) return;
    P.ended = P.req.request;
    finish();
    try { await api("POST", "/api/page/end"); } catch { /* the app stops waiting after a while */ }
  }

  function finish() {
    if (!P.req) return;
    P.seq++;
    if (P.wait) { P.wait.ctrl.abort(); settle(); }
    unzoom();
    P.req = null; P.res = null; P.phase = "idle"; P.armed = false; P.busy = false; P.said = ""; P.side = "after";
    P.panel?.remove(); P.panel = null; P.body = null; P.notes = null; P.foot = null; P.scope = null;
    $("hist").hidden = false;
    box().classList.remove("pg-on", "pg-say");
    // the line as it was before the pages came (新對話 writes its own right after)
    if (P.prev) { S.line = P.prev.line; $("line").className = P.prev.cls; $("line").textContent = P.prev.text; P.prev = null; }
    $("text").placeholder = "說點什麼…";
    S.chip?.actual("");                            // the chip is the chat's model again
  }

  /** Esc: the larger picture, the wait, a pending 再改, then page mode. */
  function escape() {
    if (!P.req) return false;
    if (P.zoom) unzoom();
    else if (P.wait) cancel();
    else if (P.armed) arm(false);
    else end();
    return true;
  }

  let started = false;
  return {
    /** app.js, once boot() has opened or collapsed the window. */
    start: () => { if (!started) { started = true; poll(); } },
    take, escape, end: () => end(),
    active: () => !!P.req,
  };
})();
