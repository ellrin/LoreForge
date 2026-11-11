// LoreEcho voice (design §7): the mic (press and hold, or 常駐), 朗讀回覆, the theme song, and the
// 人物 tab's 聲音 / 語速 / 主題曲 rows. Loaded after app.js; app.js calls the hooks in `LoreVoice`
// (all optional: without this file the page still works as text only).
//
// Server: GET /api/voice, GET /api/voice/voices, GET /api/voice/events (server-sent events),
// POST /api/voice/listen {continuous}, /listen/stop, /speak {text, voice, rate}, /stop,
// /open-settings {what}; GET/PUT /api/settings {talk, auto_send, read_aloud};
// GET/PUT/DELETE /api/characters/<id>/theme.

"use strict";

(() => {
  const $ = (id) => document.getElementById(id);
  const V = {
    available: false, why: "",
    talk: "hold", autoSend: true, readAloud: true,
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
    mic.after(aloud, play);
    icons(row);
    try {
      const [st, set] = await Promise.all([fetch("/api/voice").then((r) => r.json()), fetch("/api/settings").then((r) => r.json())]);
      V.available = !!st.available; V.why = st.why || "";
      V.talk = set.talk || "hold"; V.autoSend = set.auto_send !== false; V.readAloud = set.read_aloud !== false;
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
    on("speaking", (d) => {
      V.speaking = !!d.on; duck();
      if (V.speaking) app().setState?.("speaking");
      else { const w = V.waits; V.waits = []; w.forEach((f) => f()); }
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
    if (said) put(said); else put("");
    V.prefix = "";
    const all = $("text").value.trim();
    if (!all) return;
    if (V.autoSend) app().send?.();
    else $("text").focus();
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

  function setContinuous(on) {
    V.continuous = on; V.paused = false; paintMic();
    if (on) start(true); else stopListening();
  }

  // ── speaking ──────────────────────────────────────────────────
  function stopSpeaking() {
    if (!V.speaking) return false;
    post("/api/voice/stop").catch(() => {});
    return true;
  }

  /** Called when the reply text is complete: `done()` runs once the voice is quiet too. */
  function afterReply(spoke, done) {
    const resume = () => { done(); if (V.continuous && V.paused) { V.paused = false; start(true); } };
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
    V.waits = [];
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
      <label class="set-row"><span class="k">朗讀回覆</span><input type="checkbox" id="setReadAloud"></label>`;
    const save = (patch) => fetch("/api/settings", { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(patch) })
      .then(async (r) => { if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error || `${r.status}`); })
      .catch((e) => toast(e.message, { err: true }));
    const paint = () => sec.querySelectorAll("[data-talk]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.talk === V.talk)));
    sec.querySelectorAll("[data-talk]").forEach((b) => b.addEventListener("click", () => {
      if (V.continuous) setContinuous(false);
      V.talk = b.dataset.talk; paint(); paintMic(); save({ talk: V.talk });
    }));
    const auto = sec.querySelector("#setAutoSend"), aloud = sec.querySelector("#setReadAloud");
    auto.checked = V.autoSend; aloud.checked = V.readAloud;
    auto.addEventListener("change", () => { V.autoSend = auto.checked; save({ auto_send: auto.checked }); });
    aloud.addEventListener("change", () => setReadAloud(aloud.checked));
    paint();
    if (!V.available) sec.append(Object.assign(document.createElement("div"), { className: "set-empty", textContent: V.why }));
    el.append(sec);
  }

  // ── settings: 人物 → 聲音, 語速, 主題曲 ─────────────────────────
  const QUALITY = { premium: "高品質", enhanced: "加強版", default: "" };

  /** Rows for the 角色 section: `save(patch)` writes character.toml. */
  function characterRows(c, d, save, row) {
    const out = [];
    // 聲音: a picker of system voices (the character's language first) + 試聽
    const sel = document.createElement("select");
    sel.className = "set-input"; sel.setAttribute("aria-label", "聲音"); sel.style.flex = "1"; sel.style.minWidth = "0";
    sel.append(new Option("系統預設", ""));
    const test = document.createElement("button");
    test.type = "button"; test.className = "btn"; test.innerHTML = `<span data-icon="play"></span>試聽`;
    const rate = document.createElement("input");
    rate.type = "range"; rate.min = "0.5"; rate.max = "2"; rate.step = "0.05"; rate.value = String(d?.voice_rate ?? 1);
    rate.setAttribute("aria-label", "語速"); rate.style.flex = "1";
    const rateOut = document.createElement("span"); rateOut.className = "set-val";
    const showRate = () => { rateOut.textContent = `${Number(rate.value).toFixed(2)}×`; };
    showRate();
    if (!V.available) { sel.disabled = test.disabled = rate.disabled = true; sel.title = V.why; }
    else fetch("/api/voice/voices").then((r) => r.json()).then((v) => {
      for (const x of v.voices ?? []) sel.append(new Option(`${x.name}（${x.language}${QUALITY[x.quality] ? `・${QUALITY[x.quality]}` : ""}）`, x.id));
      if (d?.voice && ![...sel.options].some((o) => o.value === d.voice)) sel.append(new Option(`${d.voice}（這台沒有）`, d.voice));
      sel.value = d?.voice ?? "";
    }).catch(() => {});
    sel.value = d?.voice ?? "";
    sel.addEventListener("change", () => save({ voice: sel.value }));
    rate.addEventListener("input", showRate);
    rate.addEventListener("change", () => save({ voice_rate: Number(rate.value) }));
    test.addEventListener("click", () => {
      stopSpeaking();
      const line = `你好，我是${d?.name || c.name}。`;
      post("/api/voice/speak", { text: line, voice: sel.value, rate: Number(rate.value), character: c.id }).catch((e) => toast(e.message, { err: true }));
    });
    out.push(row("聲音", sel, test), row("語速", rate, rateOut));
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
    init, beforeSend, afterReply, escape, reset, characterChanged, buildGeneral, characterRows,
    /** Ask the server to read this reply aloud. */
    wantsSpeech: () => V.available && V.readAloud,
    stopSpeaking,
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init); else init();
})();
