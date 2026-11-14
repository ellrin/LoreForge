// Slide show.
//   single     full screen in this window
//   presenter  this window = 簡報者檢視 (current + next slide, notes, timer);
//              the audience window (window.open; the desktop app puts it full
//              screen on the extended display) follows over a BroadcastChannel
//   audience   that other window: shows what the presenter shows
// Keys: → Space Enter click = next step; ← = back; number+Enter = go to;
// Home/End; L = laser pointer; pinch (trackpad) or ⌘+/⌘− = zoom, two-finger
// scroll pans while zoomed, 0 = back to 100 %; B = black screen; Esc = end.
// G = 選頁 (presenter only): every slide as a thumbnail, click / Enter jumps, Esc
// closes; the audience sees nothing until a slide is chosen.
// Hidden (備用) slides are skipped (the picker shows them dimmed; choosing one shows it).
// 互動 (I, or the button; only on a slide with an equation figure — a GIF that carries a lore-plot equation
// spec): the slide gives way to the figure's live view in the Lore theme (commons/ui/plot/eqsim.js). The
// presenter (or the single screen) gets the controls and runs the simulation; the audience window only
// renders the state it is sent ({ live } messages). Esc or the button returns to the slide. The view lives
// in SHOW.live and is put back into the stage whenever showRender empties it.
"use strict";

const SHOW = {
  on: false, mode: "single", list: [], i: 0, step: 0, groups: [],
  root: null, stage: null, node: null, bar: null, black: false, end: false,
  laser: { on: false, x: 0.5, y: 0.5, seen: false }, zoom: { k: 1, x: 0.5, y: 0.5 },
  typed: "", chan: null, aud: null, t0: 0, paused: 0, pauseAt: 0, timer: 0, notesSize: 24,
  pick: null, pickOpen: false, pickTypedTimer: 0, thumbs: null, // 選頁: its grid (built once per window) and uid → rendered slide, kept for the show
  live: null, eqs: new Map(), // 互動: { el, view, uid, spec } while on; asset → its equation spec (null: none)
};
/** This window was opened as the audience window (it closes when the show ends). */
const AUD_WINDOW = new URL(location.href).searchParams.has("audience");
const showChan = () => {
  if (!SHOW.chan) {
    SHOW.chan = new BroadcastChannel(`sc-show:${location.origin}`);
    SHOW.chan.onmessage = (e) => onShowMessage(e.data ?? {});
  }
  return SHOW.chan;
};

// ── Start / end ─────────────────────────────────────────────────────────────
function showList() { return S.deck.slides.filter((s) => !s.hidden); }

function startShow(fromCurrent = true, mode = "single") {
  if (!S.deck || SHOW.on) return;
  SHOW.list = showList();
  if (!SHOW.list.length) return;
  SHOW.i = fromCurrent ? Math.max(0, SHOW.list.findIndex((s) => s.uid === S.cur)) : 0;
  Object.assign(SHOW, { on: true, step: 0, black: false, end: false, typed: "", zoom: { k: 1, x: 0.5, y: 0.5 }, thumbs: new Map() });
  SHOW.laser.on = false;
  document.addEventListener("keydown", showKey, true);
  addEventListener("resize", showFit);
  if (mode === "presenter") {
    SHOW.t0 = Date.now(); SHOW.paused = 0; SHOW.pauseAt = 0;
    becomePresenter();
    // the audience window; the desktop app puts it full screen on the other display
    const u = new URL(location.href);
    u.search = `?deck=${encodeURIComponent(S.name)}&audience=1`;
    SHOW.aud = window.open(u.href, "sc-audience", "popup,width=1280,height=720");
  } else {
    setRoot("single");
    document.addEventListener("fullscreenchange", showFsChange);
    SHOW.root.requestFullscreen?.().catch(() => {});
    showRender();
  }
}

/** Replace this window's show surface with one for `mode`. */
function setRoot(mode) {
  SHOW.root?.remove();
  clearInterval(SHOW.timer);
  SHOW.pick = null; SHOW.pickOpen = false;
  SHOW.mode = mode;
  const root = document.createElement("div");
  root.className = mode === "presenter" ? "presenter" : "show";
  root.tabIndex = -1;
  SHOW.root = root;
  SHOW.bar = null;
  if (mode === "presenter") buildConsole(root);
  else SHOW.stage = root;
  if (mode === "single") buildBar(root);
  document.body.append(root);
  if (mode === "audience") {
    // clicks here drive the presenter
    root.addEventListener("click", () => showChan().postMessage({ cmd: "next" }));
    root.addEventListener("contextmenu", (e) => { e.preventDefault(); showChan().postMessage({ cmd: "prev" }); });
    root.addEventListener("wheel", (e) => { e.preventDefault(); if (!e.ctrlKey) wheelStep(e, (cmd) => showChan().postMessage({ cmd })); }, { passive: false });
  } else wireStage(SHOW.stage);
  root.focus();
}

function becomePresenter(state) {
  if (state) applyState(state);
  liveDrop();
  setRoot("presenter");
  SHOW.timer = setInterval(tick, 500);
  tick();
  showChan();
  showRender(SHOW.step);
  broadcast();
  if (state?.live) liveOpen(state.live.spec, state.live.state);
  // 選頁's thumbnails: rendered while nothing else happens, so the first G is instant
  (window.requestIdleCallback ?? ((f) => setTimeout(f, 400)))(() => { if (SHOW.on && SHOW.mode === "presenter") pickThumbs(); });
}
function becomeAudience(state) {
  if (state) applyState(state);
  liveDrop();
  setRoot("audience");
  showChan();
  showRender(SHOW.step);
  if (state?.live) liveOpen(state.live.spec, state.live.state);
}
function snapshot() {
  return { uid: SHOW.list[SHOW.i]?.uid, step: SHOW.step, black: SHOW.black, endScreen: SHOW.end, laser: { ...SHOW.laser }, zoom: { ...SHOW.zoom },
    t0: SHOW.t0, paused: SHOW.paused, pauseAt: SHOW.pauseAt, notesSize: SHOW.notesSize,
    live: SHOW.live ? { spec: SHOW.live.spec, uid: SHOW.live.uid, state: SHOW.live.view.state() } : null };
}
function applyState(m) {
  const i = SHOW.list.findIndex((s) => s.uid === m.uid);
  if (i >= 0) SHOW.i = i;
  Object.assign(SHOW, { step: m.step ?? 0, black: !!m.black, end: !!m.endScreen, laser: m.laser ?? SHOW.laser, zoom: m.zoom ?? SHOW.zoom });
  if (m.t0) Object.assign(SHOW, { t0: m.t0, paused: m.paused ?? 0, pauseAt: m.pauseAt ?? 0, notesSize: m.notesSize ?? SHOW.notesSize });
}

/** ⇄: the two windows trade roles (the presenter view moves to the other display). */
function swapScreens() {
  if (SHOW.mode !== "presenter") return;
  const st = snapshot();
  showChan().postMessage({ swap: true, state: st });
  becomeAudience(st);
  // this click is the user gesture full screen needs
  if (!document.fullscreenElement) SHOW.root.requestFullscreen?.().catch(() => {});
}

function onShowMessage(m) {
  if (!SHOW.on) return;
  if (m.end) { endShow(true); return; }
  if (m.swap) { if (SHOW.mode === "audience") becomePresenter(m.state); return; }
  if (m.cmd) { if (SHOW.mode === "presenter") showCommand(m.cmd); return; }
  if ("live" in m) { if (SHOW.mode === "audience") liveFollow(m.live); return; }
  if (!m.uid || SHOW.mode !== "audience") return;
  const i = showIndexOf(m.uid);
  const changed = i !== SHOW.i || m.step < SHOW.step || !SHOW.node || !!m.black !== SHOW.black || !!m.endScreen !== SHOW.end;
  Object.assign(SHOW, { black: !!m.black, end: !!m.endScreen, laser: m.laser, zoom: m.zoom });
  if (i >= 0) SHOW.i = i;
  if (changed) showRender(m.step);
  else { while (SHOW.step < m.step) playGroup(); showFit(); }
}

function endShow(fromOther = false) {
  if (!SHOW.on) return;
  SHOW.on = false;
  clearInterval(SHOW.timer);
  document.removeEventListener("keydown", showKey, true);
  removeEventListener("resize", showFit);
  document.removeEventListener("fullscreenchange", showFsChange);
  if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
  if (!fromOther && SHOW.chan) SHOW.chan.postMessage({ end: true });
  try { SHOW.aud?.close(); } catch {}
  SHOW.aud = null;
  liveDrop();
  const uid = SHOW.list[SHOW.i]?.uid;
  SHOW.root?.remove();
  SHOW.root = SHOW.stage = SHOW.node = SHOW.bar = SHOW.pick = null;
  SHOW.pickOpen = false;
  SHOW.thumbs = null;
  if (AUD_WINDOW) { closeAudienceWindow(); return; }
  if (uid && uid !== S.cur) gotoSlide(uid);
}
/** Close this (audience) window: the desktop app closes it when the title says so; a browser lets the opener's popup close itself. */
function closeAudienceWindow() {
  document.title = "sc:close";
  setTimeout(() => window.close(), 50);
}
// Esc in real full screen is taken by the browser (it leaves full screen first): in 互動 that Esc leaves 互動 only
function showFsChange() {
  if (document.fullscreenElement || !SHOW.on || SHOW.mode !== "single") return;
  if (SHOW.live) { liveClose(); return; }
  // the same Esc may already have closed 互動 by its keydown: that one does not end the show too
  if (performance.now() - (SHOW.liveClosedAt ?? -1e9) < 600) return;
  endShow();
}

/** The window opened by 簡報者檢視: starts as the audience, asks for the current state. */
function startAudience() {
  SHOW.list = showList();
  SHOW.on = true;
  document.title = `${S.name} · 放映`;
  document.addEventListener("keydown", showKey, true);
  addEventListener("resize", showFit);
  becomeAudience();
  showChan().postMessage({ cmd: "hello" });
  // closing this window (⌘W) ends the show
  addEventListener("pagehide", () => { if (SHOW.on) showChan().postMessage({ cmd: "end" }); });
}

// ── Presenter console ───────────────────────────────────────────────────────
function buildConsole(root) {
  root.innerHTML = `
    <div class="pv-main"><div class="pv-stage"></div></div>
    <aside class="pv-side">
      <div class="pv-label">下一頁</div><div class="pv-next"></div>
      <div class="pv-label pv-notes-head">備忘稿<span class="grow"></span>
        <button class="ico small" data-a="smaller" data-icon="zoomOut" aria-label="字小一點"></button>
        <button class="ico small" data-a="bigger" data-icon="zoomIn" aria-label="字大一點"></button></div>
      <div class="pv-notes"></div>
    </aside>
    <footer class="pv-bar">
      <span class="pv-timer">00:00</span>
      <button class="ico small" data-a="pause" data-icon="clock" aria-label="暫停／繼續計時"></button>
      <button class="ico small" data-a="reset" data-icon="refresh" aria-label="重新計時"></button>
      <span class="pv-clock"></span>
      <button class="ghost" data-a="swap"><span data-icon="recast"></span>對調螢幕</button>
      <span class="grow"></span>
      <button class="ico" data-a="pick" data-icon="slides" aria-label="選頁 (G)"></button>
      <span class="pv-pos"></span>
      <button class="ghost pv-live" data-a="live" hidden><span data-icon="sliders"></span>互動</button>
      <span class="grow"></span>
      <button class="ico" data-a="laser" data-icon="target" aria-label="雷射筆"></button>
      <button class="ico" data-a="zoomout" data-icon="zoomOut" aria-label="縮小"></button>
      <button class="ico" data-a="zoom0" data-icon="fit" aria-label="100%"></button>
      <button class="ico" data-a="zoomin" data-icon="zoomIn" aria-label="放大"></button>
      <button class="ico" data-a="black" data-icon="eyeOff" aria-label="黑畫面"></button>
      <span class="vsep"></span>
      <button class="ico" data-a="prev" data-icon="back" aria-label="上一步"></button>
      <button class="ico" data-a="next" data-icon="play" aria-label="下一步"></button>
      <button class="ghost" data-a="end">結束</button>
    </footer>`;
  paintIcons(root);
  SHOW.stage = root.querySelector(".pv-stage");
  root.querySelector(".pv-bar").addEventListener("click", (e) => { const a = e.target.closest("[data-a]")?.dataset.a; if (a) { e.stopPropagation(); showCommand(a); } });
  root.querySelector(".pv-notes-head").addEventListener("click", (e) => { const a = e.target.closest("[data-a]")?.dataset.a; if (a) showCommand(a); });
}
/** Single-screen controls: everything reachable without shortcuts. */
const BAR = `
  <button class="ico" data-a="prev" data-icon="back" aria-label="上一步"></button>
  <span class="sb-pos"></span>
  <button class="ico" data-a="next" data-icon="play" aria-label="下一步"></button>
  <span class="vsep"></span>
  <button class="ico sb-live" data-a="live" data-icon="sliders" aria-label="互動 (I)" hidden></button>
  <button class="ico" data-a="laser" data-icon="target" aria-label="雷射筆"></button>
  <button class="ico" data-a="zoomout" data-icon="zoomOut" aria-label="縮小"></button>
  <button class="ico" data-a="zoom0" data-icon="fit" aria-label="100%"></button>
  <button class="ico" data-a="zoomin" data-icon="zoomIn" aria-label="放大"></button>
  <button class="ico" data-a="black" data-icon="eyeOff" aria-label="黑畫面"></button>
  <span class="vsep"></span>
  <button class="ghost" data-a="end">結束</button>`;
function buildBar(root) {
  const bar = document.createElement("div");
  bar.className = "show-bar";
  bar.innerHTML = BAR;
  paintIcons(bar);
  root.append(bar);
  SHOW.bar = bar;
  bar.addEventListener("click", (e) => { e.stopPropagation(); const a = e.target.closest("[data-a]")?.dataset.a; if (a) showCommand(a); });
  bar.addEventListener("pointermove", (e) => e.stopPropagation());
  let idle = 0;
  const wake = () => { bar.classList.add("up"); clearTimeout(idle); idle = setTimeout(() => { if (!bar.matches(":hover")) bar.classList.remove("up"); }, 2500); };
  root.addEventListener("pointermove", wake);
  wake();
}
function barState() {
  const bar = SHOW.bar;
  if (!bar || !SHOW.root?.contains(bar)) return;
  const sl = SHOW.list[SHOW.i];
  bar.querySelector(".sb-pos").textContent = `${slideNo(sl.uid)} / ${S.deck.slides.length}`;
  bar.querySelector('[data-a="laser"]').setAttribute("aria-pressed", String(SHOW.laser.on));
  bar.querySelector('[data-a="black"]').setAttribute("aria-pressed", String(SHOW.black));
  bar.querySelector('[data-a="zoom0"]').disabled = SHOW.zoom.k <= 1.001;
  liveButton(bar.querySelector('[data-a="live"]'));
}

function tick() {
  const el = SHOW.root?.querySelector(".pv-timer");
  if (!el) return;
  const ms = (SHOW.pauseAt || Date.now()) - SHOW.t0 - SHOW.paused;
  const s = Math.max(0, Math.floor(ms / 1000));
  const p = (n) => String(n).padStart(2, "0");
  el.textContent = s >= 3600 ? `${Math.floor(s / 3600)}:${p(Math.floor(s / 60) % 60)}:${p(s % 60)}` : `${p(Math.floor(s / 60))}:${p(s % 60)}`;
  el.classList.toggle("paused", !!SHOW.pauseAt);
  const d = new Date();
  SHOW.root.querySelector(".pv-clock").textContent = `${p(d.getHours())}:${p(d.getMinutes())}`;
}
function consoleSide() {
  if (SHOW.mode !== "presenter") return;
  const sl = SHOW.list[SHOW.i], nx = SHOW.list[SHOW.i + 1];
  const next = SHOW.root.querySelector(".pv-next");
  next.textContent = "";
  if (SHOW.step < SHOW.groups.length) next.append(Object.assign(document.createElement("div"), { className: "pv-more", textContent: `本頁還有 ${SHOW.groups.length - SHOW.step} 個動畫` }));
  if (nx) {
    const n = SC.renderSlide(nx, ctxFor(nx, { slideNo: slideNo(nx.uid), learn: false })); // 不熟 marks never show
    n.classList.add("pv-thumb");
    next.append(n);
    const W = SC.px(S.deck.size.w);
    requestAnimationFrame(() => { n.style.transform = `scale(${next.clientWidth / W})`; next.style.height = `${(next.clientWidth / W) * SC.px(S.deck.size.h)}px`; });
  } else next.append(Object.assign(document.createElement("div"), { className: "pv-more", textContent: "最後一頁" }));
  const notes = SHOW.root.querySelector(".pv-notes");
  notes.textContent = sl.notes || "";
  notes.style.fontSize = `${SHOW.notesSize}px`;
  SHOW.root.querySelector(".pv-pos").textContent = `${slideNo(sl.uid)} / ${S.deck.slides.length}`;
  SHOW.root.querySelector('[data-a="laser"]').setAttribute("aria-pressed", String(SHOW.laser.on));
  SHOW.root.querySelector('[data-a="black"]').setAttribute("aria-pressed", String(SHOW.black));
  liveButton(SHOW.root.querySelector('[data-a="live"]'));
}

// ── Rendering ───────────────────────────────────────────────────────────────
/** Click groups: each click-triggered animation starts a group; "with" / "after" join it. */
function animGroups(sl) {
  const groups = [];
  for (const a of sl.anims ?? []) {
    if (a.trigger === "click" || !groups.length) groups.push([a]);
    else groups.at(-1).push(a);
  }
  return groups;
}

function showRender(step = 0, animate = false) {
  const stage = SHOW.stage;
  stage.textContent = "";
  if (SHOW.bar && SHOW.mode === "single") stage.append(SHOW.bar);
  stage.classList.toggle("black", SHOW.black || SHOW.end);
  const sl = SHOW.list[SHOW.i];
  const node = SC.renderSlide(sl, ctxFor(sl, { slideNo: slideNo(sl.uid), learn: false })); // 不熟 marks never show
  node.classList.add("show-slide");
  stage.append(node);
  const dot = document.createElement("div");
  dot.className = "laser";
  stage.append(dot);
  // 互動 stays over its slide (another slide closes it); emptying the stage must not lose it
  if (SHOW.live && SHOW.live.uid !== sl.uid) liveClose();
  if (SHOW.live) stage.append(SHOW.live.el);
  SHOW.node = node;
  SHOW.groups = animGroups(sl);
  for (const g of SHOW.groups) for (const a of g) node.querySelector(`.obj[data-id="${a.obj}"]`)?.classList.add("show-pending");
  SHOW.step = 0;
  while (SHOW.step < step) playGroup(animate);
  showFit();
  consoleSide();
}

function playGroup(animate = true) {
  const g = SHOW.groups[SHOW.step++];
  if (!g) return;
  let wait = 0;
  for (const a of g) {
    const el = SHOW.node.querySelector(`.obj[data-id="${a.obj}"]`);
    if (!el) continue;
    if (a.trigger === "after") wait += a.duration_ms;
    const go = () => {
      el.classList.remove("show-pending");
      if (!animate || a.effect === "appear") return;
      el.style.setProperty("--dur", `${a.duration_ms}ms`);
      el.classList.add(`show-${a.effect}`);
    };
    const delay = (a.delay_ms ?? 0) + (a.trigger === "after" ? wait - a.duration_ms : 0);
    if (animate && delay > 0) setTimeout(go, delay); else go();
  }
}

/** Fit the slide into its stage, then apply zoom (k around x, y in slide fractions) and the laser dot. */
function showGeom() {
  const st = SHOW.stage;
  const W = SC.px(S.deck.size.w), H = SC.px(S.deck.size.h);
  const bw = st.clientWidth, bh = st.clientHeight;
  const k0 = Math.min(bw / W, bh / H), z = SHOW.zoom.k, k = k0 * z;
  let tx = (bw - W * k0) / 2, ty = (bh - H * k0) / 2;
  if (z > 1) {
    tx = bw / 2 - SHOW.zoom.x * W * k;
    ty = bh / 2 - SHOW.zoom.y * H * k;
    // keep the slide covering the stage
    tx = Math.min(Math.max(tx, bw - W * k - Math.max(0, (bw - W * k0) / 2)), Math.max(0, (bw - W * k0) / 2));
    ty = Math.min(Math.max(ty, bh - H * k - Math.max(0, (bh - H * k0) / 2)), Math.max(0, (bh - H * k0) / 2));
  }
  return { W, H, k, tx, ty };
}
function showFit() {
  if (!SHOW.node) return;
  const g = showGeom();
  SHOW.node.style.transform = `translate(${g.tx}px, ${g.ty}px) scale(${g.k})`;
  const dot = SHOW.stage.querySelector(".laser");
  if (dot) {
    dot.hidden = !SHOW.laser.on || !SHOW.laser.seen;
    dot.style.transform = `translate(${g.tx + SHOW.laser.x * g.W * g.k}px, ${g.ty + SHOW.laser.y * g.H * g.k}px)`;
  }
  SHOW.stage.classList.toggle("lasering", SHOW.laser.on);
  SHOW.stage.classList.toggle("zoomed", SHOW.zoom.k > 1.001);
  barState();
}
/** A pointer position → slide fractions. */
function showFrac(ev) {
  const g = showGeom();
  const r = SHOW.stage.getBoundingClientRect();
  return [(ev.clientX - r.left - g.tx) / (g.W * g.k), (ev.clientY - r.top - g.ty) / (g.H * g.k)];
}

function broadcast() {
  if (SHOW.mode !== "presenter") return;
  const sl = SHOW.list[SHOW.i];
  showChan().postMessage({ uid: sl.uid, step: SHOW.step, black: SHOW.black, endScreen: SHOW.end, laser: { ...SHOW.laser }, zoom: { ...SHOW.zoom } });
}

// ── Navigation ──────────────────────────────────────────────────────────────
function showNext() {
  if (SHOW.black) { SHOW.black = false; showRender(SHOW.step); broadcast(); return; }
  if (SHOW.step < SHOW.groups.length) { playGroup(); consoleSide(); broadcast(); return; }
  if (SHOW.i < SHOW.list.length - 1) { SHOW.i++; SHOW.zoom = { k: 1, x: 0.5, y: 0.5 }; showRender(0); broadcast(); return; }
  if (!SHOW.end) { SHOW.end = true; showRender(SHOW.step); broadcast(); return; }
  endShow();
}
function showPrev() {
  if (SHOW.end) { SHOW.end = false; showRender(SHOW.groups.length); broadcast(); return; }
  if (SHOW.i > 0) { SHOW.i--; SHOW.zoom = { k: 1, x: 0.5, y: 0.5 }; showRender(animGroups(SHOW.list[SHOW.i]).length); broadcast(); }
}
function showGo(i) {
  SHOW.end = false;
  SHOW.i = Math.max(0, Math.min(SHOW.list.length - 1, i));
  SHOW.zoom = { k: 1, x: 0.5, y: 0.5 };
  showRender(0);
  broadcast();
}
function setZoom(k, at) {
  const z = SHOW.zoom, nk = Math.min(6, Math.max(1, k));
  if (at && nk > 1) {
    // the point under the fingers stays put
    const f = z.k / nk;
    z.x = at[0] + (z.x - at[0]) * f;
    z.y = at[1] + (z.y - at[1]) * f;
  }
  z.k = nk;
  if (nk === 1) { z.x = 0.5; z.y = 0.5; }
  z.x = Math.min(1, Math.max(0, z.x));
  z.y = Math.min(1, Math.max(0, z.y));
  showFit();
  broadcast();
}

function showCommand(a) {
  switch (a) {
    case "next": showNext(); break;
    case "prev": showPrev(); break;
    case "end": endShow(); break;
    case "laser": SHOW.laser.on = !SHOW.laser.on; showFit(); consoleSide(); broadcast(); break;
    case "black": SHOW.black = !SHOW.black; showRender(SHOW.step); broadcast(); break;
    case "zoom0": setZoom(1); break;
    case "zoomin": setZoom(SHOW.zoom.k * 1.5); break;
    case "zoomout": setZoom(SHOW.zoom.k / 1.5); break;
    case "pause":
      if (SHOW.pauseAt) { SHOW.paused += Date.now() - SHOW.pauseAt; SHOW.pauseAt = 0; } else SHOW.pauseAt = Date.now();
      tick(); break;
    case "reset": SHOW.t0 = Date.now(); SHOW.paused = 0; SHOW.pauseAt = SHOW.pauseAt ? Date.now() : 0; tick(); break;
    case "bigger": SHOW.notesSize = Math.min(48, SHOW.notesSize + 2); consoleSide(); break;
    case "smaller": SHOW.notesSize = Math.max(12, SHOW.notesSize - 2); consoleSide(); break;
    case "hello": broadcast(); if (SHOW.live) showChan().postMessage({ live: { ...SHOW.live.view.state(), open: SHOW.live.spec, uid: SHOW.live.uid } }); break;
    case "swap": swapScreens(); break;
    case "pick": if (SHOW.pickOpen) closePicker(); else openPicker(); break;
    case "live": if (SHOW.live) liveClose(); else liveStart(); break;
    case "liveoff": if (SHOW.live) liveClose(); break;
  }
}

function showKey(ev) {
  if (!SHOW.on) return;
  // typing in 互動 (a custom term's formula) is typing, not show keys
  if (SHOW.live && ev.target?.closest?.(".show-live") && ev.target.matches?.("input[type=text], textarea")) return;
  ev.stopPropagation();
  if (SHOW.mode === "audience") {
    // this window has the focus: its keys drive the presenter (in 互動, Esc only leaves it)
    if (SHOW.live) { if (ev.key === "Escape") { ev.preventDefault(); showChan().postMessage({ cmd: "liveoff" }); } return; }
    const cmd = { ArrowRight: "next", ArrowDown: "next", " ": "next", Enter: "next", PageDown: "next", ArrowLeft: "prev", ArrowUp: "prev", PageUp: "prev", Escape: "end" }[ev.key];
    if (cmd) { ev.preventDefault(); showChan().postMessage({ cmd }); }
    return;
  }
  const k = ev.key, mod = ev.metaKey || ev.ctrlKey;
  if (SHOW.pickOpen) { pickKey(ev); return; }
  if (SHOW.live) { liveKey(ev); return; }
  if (k.toLowerCase() === "i" && !mod && liveFigure()) { ev.preventDefault(); liveStart(); return; }
  if (k.toLowerCase() === "g" && !mod && SHOW.mode === "presenter") { ev.preventDefault(); openPicker(); return; }
  if (mod && (k === "=" || k === "+")) { ev.preventDefault(); setZoom(SHOW.zoom.k * 1.25); return; }
  if (mod && k === "-") { ev.preventDefault(); setZoom(SHOW.zoom.k / 1.25); return; }
  if (/^\d$/.test(k) && !mod) {
    if (k === "0" && !SHOW.typed) { ev.preventDefault(); setZoom(1); return; }
    SHOW.typed += k; ev.preventDefault(); return;
  }
  if (k === "Enter" && SHOW.typed) { const n = +SHOW.typed; SHOW.typed = ""; const i = SHOW.list.findIndex((s) => slideNo(s.uid) === n); if (i >= 0) showGo(i); ev.preventDefault(); return; }
  SHOW.typed = "";
  const key = k.toLowerCase();
  if (["ArrowRight", "ArrowDown", "PageDown", " ", "Enter"].includes(k) || key === "n") { ev.preventDefault(); showNext(); }
  else if (["ArrowLeft", "ArrowUp", "PageUp", "Backspace"].includes(k) || key === "p") { ev.preventDefault(); showPrev(); }
  else if (k === "Home") { ev.preventDefault(); showGo(0); }
  else if (k === "End") { ev.preventDefault(); showGo(SHOW.list.length - 1); }
  else if (key === "l") { ev.preventDefault(); showCommand("laser"); }
  else if (key === "b" || k === ".") { ev.preventDefault(); showCommand("black"); }
  else if (k === "Escape") { ev.preventDefault(); if (SHOW.zoom.k > 1) setZoom(1); else endShow(); }
}

// ── 選頁 (presenter): all slides at a glance ───────────────────────────────
// The thumbnails are the show's own renders (no 不熟 marks), made once per show and
// kept; the grid is built once per window and only re-marked and re-sized when it opens.
// Like a light table: as many columns as make the thumbnails largest while the whole deck
// fits the window; past PICK_MIN wide it scrolls instead. Hidden slides are shown dimmed
// and can be chosen (PowerPoint's "see all slides"): the show then includes them.
const PICK_MIN = 180, PICK_GAP = 14, PICK_LABEL = 26, PICK_PAD = 28;
function pickThumbs() {
  SHOW.thumbs ??= new Map(); // a window that became the presenter by 對調螢幕
  for (const sl of S.deck.slides) {
    if (SHOW.thumbs.has(sl.uid)) continue;
    const n = SC.renderSlide(sl, ctxFor(sl, { slideNo: slideNo(sl.uid), learn: false }));
    n.classList.add("pk-slide");
    SHOW.thumbs.set(sl.uid, n);
  }
}
/** Columns and thumbnail width for n slides in a w × h area (pure; the largest that fits, else the minimum). */
function pickLayout(n, w, h, aspect) {
  let best = { cols: 1, cw: 0 };
  for (let c = 1; c <= n; c++) {
    const r = Math.ceil(n / c);
    const cw = Math.min((w - (c - 1) * PICK_GAP) / c, ((h - (r - 1) * PICK_GAP) / r - PICK_LABEL) / aspect);
    if (cw > best.cw) best = { cols: c, cw };
  }
  if (best.cw >= PICK_MIN) return { cols: best.cols, cw: Math.floor(best.cw) };
  const cols = Math.max(1, Math.floor((w + PICK_GAP) / (PICK_MIN + PICK_GAP)));
  return { cols, cw: Math.floor((w - (cols - 1) * PICK_GAP) / cols) };
}
function pickFit() {
  const el = SHOW.pick;
  if (!el) return;
  const aspect = S.deck.size.h / S.deck.size.w;
  const { cols, cw } = pickLayout(S.deck.slides.length, el.clientWidth - 2 * PICK_PAD, el.clientHeight - 2 * PICK_PAD, aspect);
  el.style.setProperty("--pk-cols", cols);
  el.style.setProperty("--pk-w", `${cw}px`);
  el.style.setProperty("--pk-h", `${Math.round(cw * aspect)}px`);
  const k = cw / SC.px(S.deck.size.w);
  for (const n of SHOW.thumbs.values()) n.style.transform = `scale(${k})`;
}
function openPicker() {
  if (SHOW.mode !== "presenter" || !SHOW.root) return;
  if (!SHOW.pick) {
    pickThumbs();
    const el = document.createElement("div");
    el.className = "pv-pick";
    el.setAttribute("role", "dialog");
    el.setAttribute("aria-label", "選頁");
    el.innerHTML = `<span class="pk-typed" hidden></span><button class="ico pk-close" data-icon="x" aria-label="關閉 (Esc)"></button><div class="pk-grid" role="listbox"></div>`;
    paintIcons(el);
    const grid = el.querySelector(".pk-grid");
    S.deck.slides.forEach((sl, i) => {
      const c = document.createElement("button");
      c.className = "pk-cell";
      c.dataset.uid = sl.uid;
      c.setAttribute("role", "option");
      c.classList.toggle("hid", !!sl.hidden);
      c.innerHTML = `<div class="pk-frame"></div><span class="pk-no">${i + 1}</span>`;
      c.querySelector(".pk-frame").append(SHOW.thumbs.get(sl.uid));
      if (sl.hidden) c.append(Object.assign(document.createElement("span"), { className: "pk-hid", textContent: "隱藏" }));
      grid.append(c);
    });
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      if (e.target.closest(".pk-close") || e.target === el) { closePicker(); return; }
      const c = e.target.closest(".pk-cell");
      if (c) pickGo(c.dataset.uid);
    });
    el.addEventListener("pointermove", (e) => { const c = e.target.closest(".pk-cell"); if (c) pickFocus(c, false); });
    el.addEventListener("wheel", (e) => e.stopPropagation(), { passive: true });
    SHOW.pick = el;
  }
  const cur = SHOW.list[SHOW.i]?.uid;
  let focus = null;
  for (const c of SHOW.pick.querySelectorAll(".pk-cell")) {
    const on = c.dataset.uid === cur;
    c.classList.toggle("cur", on);
    c.setAttribute("aria-selected", String(on));
    if (on) focus = c;
  }
  pickTyped("");
  SHOW.pickOpen = true;
  SHOW.root.append(SHOW.pick);
  pickFit();
  addEventListener("resize", pickFit);
  focus ??= SHOW.pick.querySelector(".pk-cell");
  pickFocus(focus, false);
  focus?.scrollIntoView({ block: "center" });
}
function closePicker() {
  SHOW.pick?.remove();
  SHOW.pickOpen = false;
  removeEventListener("resize", pickFit);
  clearTimeout(SHOW.pickTypedTimer);
  SHOW.root?.focus();
}
/** The keyboard's cell: an outline ring, distinct from the current slide's filled border. */
function pickFocus(c, scroll = true) {
  if (!c) return;
  SHOW.pick.querySelector(".pk-cell.focus")?.classList.remove("focus");
  c.classList.add("focus");
  c.focus({ preventScroll: true });
  if (scroll) c.scrollIntoView({ block: "nearest" });
}
/** Digits typed so far (number + Enter), shown in a small badge; cleared after a pause. */
function pickTyped(t) {
  SHOW.typed = t;
  const b = SHOW.pick?.querySelector(".pk-typed");
  if (b) { b.hidden = !t; b.textContent = t; }
  clearTimeout(SHOW.pickTypedTimer);
  if (t) SHOW.pickTypedTimer = setTimeout(() => pickTyped(""), 2500);
}
/** A hidden slide chosen in 選頁 joins this show's list, in deck order (both windows do this). */
function showIndexOf(uid) {
  let i = SHOW.list.findIndex((s) => s.uid === uid);
  if (i >= 0) return i;
  const at = S.deck.slides.findIndex((s) => s.uid === uid);
  if (at < 0) return -1;
  i = SHOW.list.findIndex((s) => S.deck.slides.findIndex((x) => x.uid === s.uid) > at);
  if (i < 0) i = SHOW.list.length;
  SHOW.list.splice(i, 0, S.deck.slides[at]);
  return i;
}
function pickGo(uid) {
  closePicker();
  const i = showIndexOf(uid);
  if (i >= 0 && (i !== SHOW.i || SHOW.end)) showGo(i);
}
/** Keys while the picker is open: arrows move, Enter jumps, Esc / G close; digits + Enter jump to that page number. */
function pickKey(ev) {
  const k = ev.key;
  const cells = [...SHOW.pick.querySelectorAll(".pk-cell")];
  const at = Math.max(0, cells.findIndex((c) => c.classList.contains("focus")));
  const cols = +SHOW.pick.style.getPropertyValue("--pk-cols") || 1;
  const move = (d) => { const j = Math.min(cells.length - 1, Math.max(0, at + d)); pickFocus(cells[j]); };
  if (/^\d$/.test(k)) {
    ev.preventDefault();
    const t = (SHOW.typed + k).replace(/^0+/, "");
    pickTyped(t);
    if (cells[+t - 1]) pickFocus(cells[+t - 1]);
    return;
  }
  if (k === "Backspace") { ev.preventDefault(); pickTyped(SHOW.typed.slice(0, -1)); return; }
  const typed = SHOW.typed;
  pickTyped("");
  if (k === "Escape" || k.toLowerCase() === "g") { ev.preventDefault(); closePicker(); }
  else if (k === "Enter" || k === " ") {
    ev.preventDefault();
    const c = typed ? cells[+typed - 1] : cells[at];
    if (c) pickGo(c.dataset.uid);
  }
  else if (k === "ArrowRight") { ev.preventDefault(); move(1); }
  else if (k === "ArrowLeft") { ev.preventDefault(); move(-1); }
  else if (k === "ArrowDown") { ev.preventDefault(); move(cols); }
  else if (k === "ArrowUp") { ev.preventDefault(); move(-cols); }
  else if (k === "Home") { ev.preventDefault(); pickFocus(cells[0]); }
  else if (k === "End") { ev.preventDefault(); pickFocus(cells.at(-1)); }
  else if (k === "Tab") { ev.preventDefault(); move(ev.shiftKey ? -1 : 1); }
  else ev.preventDefault();
}

/** Mouse wheel / two-finger scroll = next / previous step: one step per gesture
 *  (deltas add up past a threshold, then a short pause so a fling is one page). */
const WHEEL = { acc: 0, until: 0 };
function wheelStep(e, run) {
  const now = performance.now();
  if (now < WHEEL.until) { WHEEL.acc = 0; return; }
  const d = Math.abs(e.deltaY) >= Math.abs(e.deltaX) ? e.deltaY : e.deltaX;
  WHEEL.acc += e.deltaMode === 1 ? d * 16 : d;
  if (Math.abs(WHEEL.acc) < 40) return;
  run(WHEEL.acc > 0 ? "next" : "prev");
  WHEEL.acc = 0;
  WHEEL.until = now + 450;
}

/** Pointer, laser, pinch zoom and two-finger pan on the live slide. */
function wireStage(st) {
  // zoomed: drag with the mouse to pan (laser or not); a drag is not a click
  let drag = null, dragged = false;
  st.addEventListener("pointerdown", (e) => {
    dragged = false;
    if (e.button !== 0 || SHOW.zoom.k <= 1.001 || e.target.closest(".show-bar")) return;
    drag = { x: e.clientX, y: e.clientY, zx: SHOW.zoom.x, zy: SHOW.zoom.y };
    st.setPointerCapture(e.pointerId);
    st.classList.add("dragging");
  });
  const stop = () => { drag = null; st.classList.remove("dragging"); };
  st.addEventListener("pointerup", stop);
  st.addEventListener("pointercancel", stop);
  st.addEventListener("pointermove", (e) => {
    if (drag) {
      const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
      if (Math.hypot(dx, dy) > 4) dragged = true;
      const g = showGeom();
      SHOW.zoom.x = drag.zx - dx / (g.W * g.k);
      SHOW.zoom.y = drag.zy - dy / (g.H * g.k);
      setZoom(SHOW.zoom.k);
    }
    if (!SHOW.laser.on) return;
    const [x, y] = showFrac(e);
    Object.assign(SHOW.laser, { x, y, seen: true });
    showFit();
    broadcast();
  });
  st.addEventListener("pointerleave", () => { if (SHOW.laser.on) { SHOW.laser.seen = false; showFit(); broadcast(); } });
  st.addEventListener("click", (e) => { if (e.button === 0 && SHOW.zoom.k <= 1.001 && !dragged) showNext(); });
  st.addEventListener("contextmenu", (e) => { e.preventDefault(); showPrev(); });
  // Chrome / trackpad pinch arrives as ctrl+wheel; plain wheel pans while zoomed
  st.addEventListener("wheel", (e) => {
    e.preventDefault();
    if (e.ctrlKey) { setZoom(SHOW.zoom.k * Math.exp(-e.deltaY * 0.01), showFrac(e)); return; }
    if (SHOW.zoom.k <= 1.001) { wheelStep(e, (cmd) => showCommand(cmd)); return; }
    if (SHOW.zoom.k > 1) {
      const g = showGeom();
      SHOW.zoom.x += e.deltaX / (g.W * g.k);
      SHOW.zoom.y += e.deltaY / (g.H * g.k);
      setZoom(SHOW.zoom.k);
    }
  }, { passive: false });
  // Safari / WKWebView pinch
  let g0 = 1;
  st.addEventListener("gesturestart", (e) => { e.preventDefault(); g0 = SHOW.zoom.k; });
  st.addEventListener("gesturechange", (e) => { e.preventDefault(); setZoom(g0 * e.scale, showFrac(e)); });
  st.addEventListener("gestureend", (e) => e.preventDefault());
}

// ── 互動: the live view of an equation figure ───────────────────────────────
/** Equation figures on a slide: GIF pictures whose embedded spec is a lore-plot equation (looked up once per asset). */
function liveFigure(sl = SHOW.list[SHOW.i]) {
  for (const o of sl?.objects ?? []) {
    if (o.type !== "picture" || !/\.gif$/i.test(o.asset ?? "")) continue;
    const v = SHOW.eqs.get(o.asset);
    if (v === undefined) {
      SHOW.eqs.set(o.asset, null);
      fetch(`${deckApi("/plot")}?asset=${encodeURIComponent(o.asset)}`)
        .then((r) => (r.ok ? r.json() : null))
        .then((j) => { const sp = j?.spec; if (sp?.panels?.some((p) => p.type === "equation")) { SHOW.eqs.set(o.asset, sp); consoleSide(); barState(); } })
        .catch(() => {});
      continue;
    }
    if (v) return v;
  }
  return null;
}
function liveButton(b) {
  if (!b) return;
  b.hidden = !liveFigure() && !SHOW.live;
  b.setAttribute("aria-pressed", String(!!SHOW.live));
}
async function liveStart() {
  const spec = liveFigure();
  if (!spec || SHOW.mode === "audience") return;
  await liveOpen(spec);
}
/** The live layer in this window: controls for the presenter / single screen, render-only for the audience. */
async function liveOpen(spec, state) {
  if (!window.LorePlot?.eqSim) { try { await window.SCPlot?.load(); } catch { return; } }
  if (!window.LorePlot?.eqSim || !SHOW.on) return;
  liveDrop();
  const el = document.createElement("div");
  el.className = "show-live";
  // its clicks, keys and wheel stay here (no slide change, no laser)
  for (const t of ["click", "pointerdown", "pointermove", "wheel", "contextmenu"]) el.addEventListener(t, (e) => e.stopPropagation());
  const role = SHOW.mode === "audience" ? "view" : SHOW.mode === "presenter" ? "control" : "full";
  const sim = spec.panels.find((p) => p.type === "equation")?.eq?.sim;
  const send = (m) => { if (SHOW.mode === "presenter") showChan().postMessage({ live: m }); };
  const o = { role, onState: send, onClose: () => liveClose() };
  const view = sim ? LorePlot.eqSim.mount(el, spec, o) : LorePlot.eqSim.mountCurve(el, spec, o);
  SHOW.live = { el, view, spec, uid: SHOW.list[SHOW.i]?.uid };
  SHOW.stage.append(el);
  SHOW.root?.classList.add("live");
  if (state) view.apply(state);
  send({ ...view.state(), open: spec, uid: SHOW.live.uid });
  showFit();
  consoleSide();
}
/** Audience: follow the presenter's live state (null: back to the slide). */
async function liveFollow(m) {
  if (!m) { liveClose(true); return; }
  if (m.open) { SHOW.liveOpening = true; try { await liveOpen(m.open, m); } finally { SHOW.liveOpening = false; } return; }
  if (SHOW.liveOpening) return; // the open's state comes with it; ticks before that would leave a gap
  // a tick that starts past what this window has (it joined late, or missed one): ask for the whole state again
  if (SHOW.live && m.t === "tick" && m.from > (SHOW.live.view.run?.()?.hist.length ?? 0)) { showChan().postMessage({ cmd: "hello" }); return; }
  SHOW.live?.view.apply(m);
}
/** Remove the layer (no message). */
function liveDrop() {
  if (!SHOW.live) return;
  try { SHOW.live.view.destroy(); } catch {}
  SHOW.live.el.remove();
  SHOW.live = null;
  SHOW.root?.classList.remove("live");
}
function liveClose(fromOther = false) {
  if (!SHOW.live) return;
  liveDrop();
  SHOW.liveClosedAt = performance.now();
  if (SHOW.mode === "presenter" && !fromOther) showChan().postMessage({ live: null });
  if (SHOW.mode === "audience" || !SHOW.stage) return;
  showFit();
  consoleSide();
}
function liveKey(ev) {
  const k = ev.key;
  if (k === "Escape" || (k.toLowerCase() === "i" && !ev.metaKey && !ev.ctrlKey)) { ev.preventDefault(); liveClose(); return; }
  if (k === " ") { ev.preventDefault(); SHOW.live.view.toggle?.(); }
}
