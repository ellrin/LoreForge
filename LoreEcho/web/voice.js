// LoreEcho voice (design §7): the mic (press and hold, or 常駐), 朗讀回覆 and 文字跟著聲音, the
// theme song, and the 人物 tab's 聲音 / 個人聲音 / 語速 / 音調 / 主題曲 rows. Loaded after app.js; app.js calls the hooks in `LoreVoice`
// (all optional: without this file the page still works as text only).
//
// Server: GET /api/voice, GET /api/voice/voices, GET /api/voice/events (server-sent events),
// POST /api/voice/listen {continuous}, /listen/stop, /speak {text, voice, rate, pitch}, /stop,
// /open-settings {what}, /warm; GET/POST /api/voice/personal;
// GET/PUT /api/settings {talk, auto_send, read_aloud, text_follows};
// GET/PUT/DELETE /api/characters/<id>/theme.

"use strict";

(() => {
  const $ = (id) => document.getElementById(id);
  const V = {
    available: false, why: "",
    talk: "hold", autoSend: true, readAloud: true, follows: true,
    follow: null,         // the reply being revealed with the voice (文字跟著聲音)
    listening: false, speaking: false,
    holding: false,       // the mic is pressed (hold mode)
    continuous: false,    // 常駐 is on
    paused: false,        // 常駐, paused while a reply is written and spoken
    prefix: "",           // what was in the input before listening started
    waits: [],            // callbacks for "speaking is over"
    song: null,           // <audio> of the current character's theme
    songOn: false,
    fade: 0,
  };

  const post = (path, body) => fetch(path, { method: "POST", headers: body ? { "content-type": "application/json" } : {}, body: body ? JSON.stringify(body) : undefined })
    .then(async (r) => { if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error || `${r.status}`); return r.status === 204 ? {} : r.json(); });
  const btn = (id, icon, label) => { const b = document.createElement("button"); b.type = "button"; b.className = "ico"; b.id = id; b.setAttribute("aria-label", label); b.title = label; b.innerHTML = `<span data-icon="${icon}"></span>`; return b; };
  const icons = (el) => { if (typeof paintIcons === "function") paintIcons(el); };
  const toast = (m, o) => window.LoreShell?.toast(m, o);
  const app = () => window.LoreEcho ?? {};

  // ── boot ──────────────────────────────────────────────────────
  async function init() {
    const row = $("form");
    const mic = $("mic");
    const aloud = btn("aloud", "speaker", "朗讀回覆");
    const play = btn("songBtn", "play", "主題曲");
    play.hidden = true;
    mic.after(aloud);
    // the theme song belongs to the window, not the conversation: top bar, after the primary buttons
    const top = document.getElementById("actNew") ?? document.getElementById("actChoices");
    if (top) top.after(play); else mic.after(play);
    icons(row); icons(play.parentElement);
    try {
      const [st, set] = await Promise.all([fetch("/api/voice").then((r) => r.json()), fetch("/api/settings").then((r) => r.json())]);
      V.available = !!st.available; V.why = st.why || "";
      V.talk = set.talk || "hold"; V.autoSend = set.auto_send !== false; V.readAloud = set.read_aloud !== false; V.follows = set.text_follows !== false;
    } catch { V.available = false; V.why = "無法連到伺服器"; }
    paintMic(); paintAloud();
    aloud.hidden = !V.available;
    aloud.addEventListener("click", () => setReadAloud(!V.readAloud));
    play.addEventListener("click", toggleSong);
    wireMic(mic);
    if (V.available) listen();
    // a click on the figure stops the voice (a drag still moves the window)
    $("figure").addEventListener("click", () => { if (V.speaking) stopSpeaking(); });
    characterChanged(window.LoreEcho?.character?.());
  }

  function paintMic() {
    const mic = $("mic");
    mic.disabled = !V.available;
    const label = !V.available ? `語音：${V.why}` : V.talk === "continuous" ? (V.continuous ? "常駐聆聽中（按一下停止）" : "常駐聆聽（按一下開始）") : "按住說話";
    mic.setAttribute("aria-label", label); mic.title = label;
    mic.setAttribute("aria-pressed", String(V.listening || V.continuous));
    mic.classList.toggle("live", V.listening);
  }

  function paintAloud() {
    const b = $("aloud");
    b.setAttribute("aria-pressed", String(V.readAloud));
    b.title = V.readAloud ? "朗讀回覆：開" : "朗讀回覆：關";
    b.classList.toggle("off", !V.readAloud);
  }

  async function setReadAloud(on) {
    V.readAloud = on; paintAloud();
    if (!on) stopSpeaking();
    try { await fetch("/api/settings", { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify({ read_aloud: on }) }); } catch {}
  }

  // ── events from the server ────────────────────────────────────
  function listen() {
    const es = new EventSource("/api/voice/events");
    const on = (name, f) => es.addEventListener(name, (e) => { try { f(JSON.parse(e.data)); } catch {} });
    on("hello", (d) => { V.speaking = !!d.speaking; });
    on("listen_started", () => { V.listening = true; paintMic(); app().setState?.("listening"); duck(); });
    on("partial", (d) => { if (V.listening || V.holding || V.continuous) put(d.text); });
    on("final", (d) => finalText(d.text));
    on("listen_ended", () => {
      V.listening = false; paintMic(); duck();
      if (!V.continuous && app().state?.() === "listening" && !$("text").value.trim()) app().setState?.("idle");
    });
    on("speak_started", (d) => V.follow?.started(d.id));
    on("word", (d) => V.follow?.word(d));
    on("speak_finished", (d) => V.follow?.finished(d));
    on("speaking", (d) => {
      V.speaking = !!d.on; duck();
      if (!V.speaking) V.follow?.quiet();
      if (V.speaking) app().setState?.("speaking");
      else if (V.waits.length) { const w = V.waits; V.waits = []; w.forEach((f) => f()); }
      // e.g. 試聽: nothing waits; back to idle (not mid-reply: the queue may drain between sentences)
      else if (!app().busy?.() && app().state?.() === "speaking") app().setState?.("idle");
    });
    on("notice", (d) => toast(d.message));
    on("error", (d) => problem(d));
  }

  function problem(d) {
    V.holding = false;
    if (d.kind === "microphone" || d.kind === "speech") {
      V.continuous = false; V.listening = false; paintMic();
      const what = d.kind;
      toast(what === "microphone" ? "LoreEcho 沒有麥克風權限" : "LoreEcho 沒有語音辨識權限", {
        err: true, ms: 12000,
        action: { label: "開啟系統設定", run: () => post("/api/voice/open-settings", { what }).catch((e) => toast(e.message, { err: true })) },
      });
    } else if (/CancellationError|cancell?ed/i.test(d.message || "")) {
      V.listening = false; paintMic();               // stopping ends the stream this way: not an error
    } else if (d.message) {
      if (d.kind === "unavailable") { V.listening = false; V.speaking = false; paintMic(); }
      toast(d.message, { err: true });
    }
  }

  // ── listening ─────────────────────────────────────────────────
  /** The input shows what was typed before, then what is being said. */
  function put(text) {
    const t = $("text");
    t.value = V.prefix + (V.prefix && text ? " " : "") + (text || "");
    t.dispatchEvent(new Event("input"));
  }

  function finalText(text) {
    const said = (text || "").trim();
    put(said);
    const all = $("text").value.trim();
    if (!all) { V.prefix = ""; return; }
    if (!V.autoSend) { V.prefix = all; $("text").focus(); return; }   // the next sentence goes after it
    V.prefix = "";
    // still answering: send it as soon as the reply is done
    if (app().busy?.()) V.pendingSend = true; else app().send?.();
  }

  function start(continuous) {
    stopSpeaking();                       // talking over the character stops it
    V.prefix = $("text").value.trim();
    V.starting = post("/api/voice/listen", { continuous })
      .catch((e) => { V.holding = false; V.continuous = false; paintMic(); toast(e.message, { err: true }); });
    return V.starting;
  }

  // a quick press: the stop goes after the start has reached the server
  const stopListening = () => Promise.resolve(V.starting).then(() => post("/api/voice/listen/stop")).catch(() => {});

  function wireMic(mic) {
    // press and hold: pointer or Space/Enter held on the button
    const down = (e) => {
      if (!V.available || V.talk !== "hold" || V.holding) return;
      e.preventDefault();
      V.holding = true; start(false);
    };
    const up = () => { if (!V.holding) return; V.holding = false; stopListening(); };
    mic.addEventListener("pointerdown", (e) => { if (e.button === 0) { mic.setPointerCapture?.(e.pointerId); down(e); } });
    mic.addEventListener("pointerup", up);
    mic.addEventListener("pointercancel", up);
    mic.addEventListener("keydown", (e) => { if ((e.key === " " || e.key === "Enter") && !e.repeat) down(e); });
    mic.addEventListener("keyup", (e) => { if (e.key === " " || e.key === "Enter") up(); });
    // 常駐: a click turns it on and off
    mic.addEventListener("click", () => { if (V.talk === "continuous" && V.available) setContinuous(!V.continuous); });
  }

  /** The open window: get the voice ready so the first reply starts sooner (no sound). */
  function opened() {
    if (V.available && V.readAloud) post("/api/voice/warm", { character: V.char?.id }).catch(() => {});
  }

  function setContinuous(on) {
    V.continuous = on; V.paused = false; paintMic();
    if (on) start(true); else stopListening();
  }

  // ── 文字跟著聲音: the line shows what has been said ──────────────
  // The chat stream says where each queued sentence is (`say {id, start, end, spoken}`, UTF-16
  // units of the reply); the voice events say when it starts, which word is being said (system
  // voices: exact) and when it ends. Text between sentences that is not said (a Markdown rule)
  // shows when the next sentence starts. Nothing heard within WATCHDOG of the first text, the
  // voice stopping, or the reply ending quietly: the whole reply shows.
  const WATCHDOG = 4000;
  const CPS = 4.5;   // characters a second, for a sentence without word events

  function follow(speak, paint) {
    if (!speak || !V.follows || !V.available) return null;
    const F = {
      reply: "", shown: 0, spans: new Map(), early: new Set(), cur: null, begun: false, complete: false, off: false, dog: 0, tick: 0,
      draw() {
        if (V.follow !== F) return;
        let n = Math.min(F.shown, F.reply.length);
        if (n > 0 && n < F.reply.length && /[\ud800-\udbff]/.test(F.reply[n - 1])) n++;   // never half a character
        paint(F.reply.slice(0, n), n < F.reply.length || !F.complete ? "pending" : "");
      },
      to(n) { if (n > F.shown) { F.shown = n; F.draw(); } },
      all() {
        clearTimeout(F.dog); clearInterval(F.tick);
        F.off = true; F.shown = F.reply.length; F.complete = true;
        if (V.follow === F) { paint(F.reply, ""); V.follow = null; }
      },
      text(reply) {
        F.reply = reply;
        if (F.off) return paint(reply, "pending");
        if (!F.dog && !F.begun) F.dog = setTimeout(() => { if (!F.begun) F.all(); }, WATCHDOG);
        F.draw();
      },
      say(s) { F.spans.set(s.id, s); if (F.early.delete(s.id)) F.begin(s.id); },
      started(id) { if (F.spans.has(id)) F.begin(id); else F.early.add(id); },
      begin(id) {
        clearTimeout(F.dog); clearInterval(F.tick);
        const s = F.spans.get(id);
        F.begun = true;
        F.cur = { s, t0: performance.now(), words: false };
        F.to(s.start);
        // paced until the voice's own word events arrive
        const len = s.end - s.start, est = 1000 * Math.max(1, s.spoken) / (CPS * (V.rate || 1));
        F.tick = setInterval(() => {
          if (!F.cur || F.cur.s !== s || F.cur.words) return clearInterval(F.tick);
          F.to(s.start + Math.floor(len * Math.min(0.9, (performance.now() - F.cur.t0) / est)));
        }, 80);
      },
      word(d) {
        const c = F.cur;
        if (!c || c.s.id !== d.id) return;
        c.words = true;
        F.to(c.s.start + Math.round((c.s.end - c.s.start) * Math.min(1, (d.start + d.len) / Math.max(1, c.s.spoken))));
      },
      finished(d) {
        const s = F.spans.get(d.id);
        if (d.cancelled) return F.all();
        if (s) { F.to(s.end); if (F.cur?.s === s) { F.cur = null; clearInterval(F.tick); } }
      },
      quiet() { if (F.complete) F.all(); },
      /** The reply is complete (`reply` = all of it). */
      final(reply) {
        F.reply = reply; F.complete = true;
        if (F.off || (!V.speaking && !F.cur && [...F.spans.values()].every((s) => s.end <= F.shown))) F.all(); else F.draw();
      },
    };
    V.follow?.all();
    V.follow = F;
    return F;
  }

  // ── speaking ──────────────────────────────────────────────────
  function stopSpeaking() {
    if (!V.speaking) return false;
    post("/api/voice/stop").catch(() => {});
    return true;
  }

  /** Called when the reply text is complete: `done()` runs once the voice is quiet too. */
  function afterReply(spoke, done) {
    const resume = () => {
      done();
      if (V.pendingSend) { V.pendingSend = false; if ($("text").value.trim()) { app().send?.(); return; } }
      if (V.continuous && V.paused) { V.paused = false; start(true); }
    };
    if (!spoke) return resume();
    if (V.speaking) { V.waits.push(resume); return; }
    // the first sentence may still be on its way: wait a moment for "speaking"
    let fired = false;
    const once = () => { if (!fired) { fired = true; resume(); } };
    V.waits.push(once);
    setTimeout(() => { if (!V.speaking) { V.waits = V.waits.filter((f) => f !== once); once(); } }, 1500);
  }

  // ── theme song ────────────────────────────────────────────────
  const DUCK = 0.25;
  function songUrl(c) { return `/api/characters/${encodeURIComponent(c.id)}/theme?v=${Date.now()}`; }

  function characterChanged(c) {
    V.rate = c?.voice_rate ?? 1;
    const b = $("songBtn");
    const same = V.char && c && V.char.id === c.id && !!V.char.theme === !!c.theme;
    V.char = c;
    if (same) { if (V.song) V.song.loop = c.theme?.loop !== false; duck(); return; }   // e.g. a picture changed
    fadeOut();
    V.song = null; V.songOn = false;
    if (!b) return;
    b.hidden = !c?.theme;
    paintSong();
  }

  function paintSong() {
    const b = $("songBtn");
    b.setAttribute("aria-pressed", String(V.songOn));
    b.title = V.songOn ? "暫停主題曲" : "播放主題曲";
    b.setAttribute("aria-label", b.title);
    b.innerHTML = `<span data-icon="${V.songOn ? "pause" : "play"}"></span>`;
    icons(b);
  }

  const target = () => (V.char?.theme?.volume ?? 0.5) * (V.listening || V.speaking || V.holding ? DUCK : 1);

  function ramp(a, to, ms, then) {
    clearInterval(V.fade);
    const from = a.volume, t0 = performance.now();
    V.fade = setInterval(() => {
      const k = Math.min(1, (performance.now() - t0) / ms);
      a.volume = Math.max(0, Math.min(1, from + (to - from) * k));
      if (k >= 1) { clearInterval(V.fade); then?.(); }
    }, 30);
  }

  function duck() { if (V.song && V.songOn) ramp(V.song, target(), 300); }

  function toggleSong() {
    const c = V.char;
    if (!c?.theme) return;
    if (V.songOn) { V.songOn = false; paintSong(); if (V.song) ramp(V.song, 0, 400, () => V.song?.pause()); return; }
    if (!V.song) {
      V.song = new Audio(songUrl(c));
      V.song.addEventListener("error", () => { V.songOn = false; paintSong(); toast("這台播不了這個音樂格式", { err: true }); });
      V.song.addEventListener("ended", () => { V.songOn = false; paintSong(); });
    }
    V.song.loop = c.theme.loop !== false;
    V.song.volume = 0;
    V.songOn = true; paintSong();
    V.song.play().then(() => ramp(V.song, target(), 600)).catch(() => {});
  }

  /** Collapse or character switch: the song fades out. */
  function fadeOut() {
    const a = V.song;
    if (!a || a.paused) return;
    ramp(a, 0, 600, () => a.pause());
  }

  // ── hooks for app.js ──────────────────────────────────────────
  function beforeSend() {
    V.prefix = "";
    stopSpeaking();
    if (V.continuous && V.listening) { V.paused = true; stopListening(); }
  }

  /** Esc: stop the voice first (true when there was something to stop). */
  function escape() {
    if (V.holding || (V.listening && !V.continuous)) { V.holding = false; stopListening(); return true; }
    return stopSpeaking();
  }

  /** 新對話 / collapse: quiet. */
  function reset(collapsing) {
    stopSpeaking();
    V.follow?.all();
    V.waits = []; V.pendingSend = false; V.prefix = "";
    if (V.listening || V.continuous) { V.continuous = false; V.paused = false; V.holding = false; stopListening(); paintMic(); }
    if (collapsing) { fadeOut(); V.songOn = false; if ($("songBtn")) paintSong(); }
  }

  // ── settings: 一般 ─────────────────────────────────────────────
  function buildGeneral(el) {
    const sec = document.createElement("div");
    sec.className = "set-sec";
    sec.innerHTML = `<div class="set-label">語音</div>
      <div class="set-row"><span class="k">麥克風</span><span class="seg" role="group" aria-label="麥克風"><button type="button" data-talk="hold">按住說話</button><button type="button" data-talk="continuous">常駐</button></span></div>
      <label class="set-row"><span class="k">說完就送出</span><input type="checkbox" id="setAutoSend"></label>
      <label class="set-row"><span class="k">朗讀回覆</span><input type="checkbox" id="setReadAloud"></label>
      <label class="set-row"><span class="k">文字跟著聲音</span><input type="checkbox" id="setFollows"></label>`;
    const save = (patch) => fetch("/api/settings", { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(patch) })
      .then(async (r) => { if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error || `${r.status}`); })
      .catch((e) => toast(e.message, { err: true }));
    const paint = () => sec.querySelectorAll("[data-talk]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.talk === V.talk)));
    sec.querySelectorAll("[data-talk]").forEach((b) => b.addEventListener("click", () => {
      if (V.continuous) setContinuous(false);
      V.talk = b.dataset.talk; paint(); paintMic(); save({ talk: V.talk });
    }));
    const auto = sec.querySelector("#setAutoSend"), aloud = sec.querySelector("#setReadAloud"), follows = sec.querySelector("#setFollows");
    auto.checked = V.autoSend; aloud.checked = V.readAloud; follows.checked = V.follows;
    const paintFollows = () => { follows.disabled = !aloud.checked; follows.closest(".set-row").classList.toggle("dim", !aloud.checked); };
    paintFollows();
    auto.addEventListener("change", () => { V.autoSend = auto.checked; save({ auto_send: auto.checked }); });
    aloud.addEventListener("change", () => { setReadAloud(aloud.checked); paintFollows(); });
    follows.addEventListener("change", () => { V.follows = follows.checked; save({ text_follows: follows.checked }); });
    paint();
    if (!V.available) sec.append(Object.assign(document.createElement("div"), { className: "set-empty", textContent: V.why }));
    el.append(sec);
  }

  // ── settings: 人物 → 聲音, 個人聲音, 語速, 音調, 主題曲 ─────────────
  const QUALITY = { premium: "高品質", enhanced: "加強版", default: "" };
  const retro = (x) => x.id.includes(".eloquence.");
  const openSettings = (what) => post("/api/voice/open-settings", { what }).catch((e) => toast(e.message, { err: true }));

  /** The picker: 個人聲音 first, then the system voices (the character's language first, better
   *  first), the Eloquence ones last in 其他（機械音）. */
  function fillVoices(sel, voices, current) {
    sel.replaceChildren(new Option("系統預設", ""));
    const opt = (x) => new Option(`${x.name}（${x.language}${QUALITY[x.quality] ? `・${QUALITY[x.quality]}` : ""}）`, x.id);
    const group = (label, list) => {
      if (!list.length) return;
      const g = document.createElement("optgroup");
      g.label = label;
      g.append(...list.map(opt));
      sel.append(g);
    };
    group("個人聲音", voices.filter((x) => x.personal));
    sel.append(...voices.filter((x) => !x.personal && !retro(x)).map(opt));
    group("其他（機械音）", voices.filter((x) => !x.personal && retro(x)));
    if (current && ![...sel.options].some((o) => o.value === current)) sel.append(new Option(`${current}（這台沒有）`, current));
    sel.value = current ?? "";
  }

  /** 個人聲音: one line for the state, and the one thing to do about it. */
  function personalRow(row, reload) {
    const msg = document.createElement("span");
    msg.className = "set-note";
    const act = document.createElement("button");
    act.type = "button"; act.className = "btn";
    const holder = row("個人聲音", msg, act);
    holder.hidden = true;
    const paint = (p) => {
      holder.hidden = !p || p.status === "unsupported" || (p.status === "authorized" && p.voices > 0);
      if (holder.hidden) return;
      if (p.status === "not_determined") {
        msg.textContent = ""; act.textContent = "使用個人聲音";
        act.onclick = async () => {
          act.disabled = true;
          try { const r = await post("/api/voice/personal"); paint(r); if (r.voices) reload(); }
          catch (e) { toast(e.message, { err: true }); }
          act.disabled = false;
        };
      } else {
        msg.textContent = p.status === "denied" ? "LoreEcho 沒有個人聲音權限" : "還沒有個人聲音：系統設定 → 輔助使用 → 個人聲音";
        act.textContent = "開啟系統設定";
        act.onclick = () => openSettings("personal-voice");
      }
    };
    if (V.available) fetch("/api/voice/personal").then((r) => r.ok ? r.json() : null).then(paint).catch(() => {});
    return holder;
  }

  /** Rows for the 角色 section: `save(patch)` writes character.toml. */
  function characterRows(c, d, save, row) {
    const out = [];
    const sel = document.createElement("select");
    sel.className = "set-input"; sel.setAttribute("aria-label", "聲音"); sel.style.flex = "1"; sel.style.minWidth = "0";
    const test = document.createElement("button");
    test.type = "button"; test.className = "btn"; test.innerHTML = `<span data-icon="play"></span>試聽`;
    const slider = (label, min, max, value) => {
      const r = document.createElement("input");
      r.type = "range"; r.min = String(min); r.max = String(max); r.step = "0.05"; r.value = String(value);
      r.setAttribute("aria-label", label); r.style.flex = "1";
      const o = document.createElement("span"); o.className = "set-val";
      const show = () => { o.textContent = `${Number(r.value).toFixed(2)}×`; };
      show(); r.addEventListener("input", show);
      return [r, o];
    };
    const [rate, rateOut] = slider("語速", 0.5, 2, d?.voice_rate ?? 1);
    const [pitch, pitchOut] = slider("音調", 0.5, 2, d?.voice_pitch ?? 1);
    const note = document.createElement("span");
    note.className = "set-note";
    note.textContent = "更多聲音：系統設定 → 輔助使用 → 朗讀內容 → 管理聲音";
    const go = document.createElement("button");
    go.type = "button"; go.className = "btn"; go.textContent = "開啟系統設定";
    go.addEventListener("click", () => openSettings("voices"));
    const more = row("", note, go);
    const load = () => fetch("/api/voice/voices").then((r) => r.json()).then((v) => fillVoices(sel, v.voices ?? [], sel.value || d?.voice)).catch(() => {});
    fillVoices(sel, [], d?.voice);
    if (!V.available) { sel.disabled = test.disabled = rate.disabled = pitch.disabled = go.disabled = true; sel.title = V.why; }
    else load();
    sel.addEventListener("change", () => save({ voice: sel.value }));
    rate.addEventListener("change", () => { V.rate = Number(rate.value); save({ voice_rate: Number(rate.value) }); });
    pitch.addEventListener("change", () => save({ voice_pitch: Number(pitch.value) }));
    test.addEventListener("click", () => {
      stopSpeaking();
      const line = `你好，我是${d?.name || c.name}。`;
      post("/api/voice/speak", { text: line, voice: sel.value, rate: Number(rate.value), pitch: Number(pitch.value), character: c.id }).catch((e) => toast(e.message, { err: true }));
    });
    out.push(row("聲音", sel, test), more, personalRow(row, load), row("語速", rate, rateOut), row("音調", pitch, pitchOut));
    out.push(songRow(c, d, save, row));
    icons(out[0]);
    return out;
  }

  const AUDIO_ACCEPT = ".mp3,.m4a,.aac,.wav,.ogg,.opus,.flac,audio/*";

  function songRow(c, d, save, row) {
    const wrap = document.createElement("div");
    wrap.style.cssText = "display:flex;flex-wrap:wrap;gap:6px;align-items:center;flex:1;min-width:0";
    const holder = row("主題曲", wrap);
    let preview = null;
    const paint = (theme) => {
      preview?.pause(); preview = null;
      wrap.replaceChildren();
      const zone = document.createElement("button");
      zone.type = "button"; zone.className = "btn song-drop";
      zone.innerHTML = `<span data-icon="upload"></span>${theme ? "換一首…" : "選擇音樂…"}`;
      zone.title = "或把音樂檔拖到這裡（mp3、m4a、wav、ogg、flac，30 MB 以內）";
      const upload = async (file) => {
        if (!file) return;
        try {
          const r = await fetch(`/api/characters/${encodeURIComponent(c.id)}/theme`, { method: "PUT", body: file });
          if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error || `${r.status}`);
          await refresh();
        } catch (e) { toast(e.message, { err: true }); }
      };
      zone.addEventListener("click", () => {
        const f = Object.assign(document.createElement("input"), { type: "file", accept: AUDIO_ACCEPT, hidden: true });
        f.addEventListener("change", () => { upload(f.files[0]); f.remove(); });
        document.body.append(f); f.click();
      });
      for (const t of [zone, holder]) {
        t.addEventListener("dragover", (e) => { e.preventDefault(); zone.classList.add("over"); });
        t.addEventListener("dragleave", () => zone.classList.remove("over"));
        t.addEventListener("drop", (e) => { e.preventDefault(); zone.classList.remove("over"); upload(e.dataTransfer.files[0]); });
      }
      wrap.append(zone);
      if (theme) {
        const listen = document.createElement("button");
        listen.type = "button"; listen.className = "btn"; listen.innerHTML = `<span data-icon="play"></span>試聽`;
        listen.addEventListener("click", () => {
          if (preview && !preview.paused) { preview.pause(); listen.innerHTML = `<span data-icon="play"></span>試聽`; icons(listen); return; }
          preview = preview ?? new Audio(songUrl(c));
          preview.volume = theme.volume;
          preview.addEventListener("error", () => toast("這台播不了這個音樂格式", { err: true }), { once: true });
          preview.play().catch(() => {});
          listen.innerHTML = `<span data-icon="pause"></span>停止`; icons(listen);
        });
        const rm = document.createElement("button");
        rm.type = "button"; rm.className = "ico small"; rm.title = "移除主題曲"; rm.setAttribute("aria-label", "移除主題曲");
        rm.innerHTML = `<span data-icon="trash"></span>`;
        rm.addEventListener("click", async () => {
          if (!(await window.LoreShell.ask(rm, ["移除主題曲？"], [{ label: "取消", value: null }, { label: "移除", value: true, danger: true }]))) return;
          try {
            const r = await fetch(`/api/characters/${encodeURIComponent(c.id)}/theme`, { method: "DELETE" });
            if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error || `${r.status}`);
            await refresh();
          } catch (e) { toast(e.message, { err: true }); }
        });
        const loop = Object.assign(document.createElement("input"), { type: "checkbox", checked: theme.loop !== false });
        const loopL = document.createElement("label"); loopL.className = "song-opt"; loopL.append(loop, "循環");
        loop.addEventListener("change", () => Promise.resolve(save({ theme_loop: loop.checked })).then(() => app().reloadCharacters?.()));
        const vol = Object.assign(document.createElement("input"), { type: "range", min: "0", max: "1", step: "0.05", value: String(theme.volume) });
        vol.setAttribute("aria-label", "音量"); vol.className = "song-vol";
        vol.addEventListener("input", () => { if (preview) preview.volume = Number(vol.value); });
        vol.addEventListener("change", () => Promise.resolve(save({ theme_volume: Number(vol.value) })).then(() => app().reloadCharacters?.()));
        const volL = document.createElement("label"); volL.className = "song-opt"; volL.append("音量", vol);
        wrap.append(listen, rm, loopL, volL);
      }
      icons(wrap);
    };
    const refresh = async () => {
      const r = await fetch(`/api/characters/${encodeURIComponent(c.id)}`, { cache: "no-store" }).then((x) => x.json());
      await app().reloadCharacters?.();
      paint(r.theme);
    };
    paint(d?.theme ?? null);
    return holder;
  }

  window.LoreVoice = {
    init, beforeSend, afterReply, escape, reset, characterChanged, buildGeneral, characterRows, opened,
    /** 文字跟著聲音 for a reply: a controller (text, say, final, all) or null (show text at once). */
    follow,
    /** Ask the server to read this reply aloud. */
    wantsSpeech: () => V.available && V.readAloud,
    stopSpeaking,
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init); else init();
})();
