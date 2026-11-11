// LoreEcho custom voices (Qwen3-TTS): 人物 → 聲音 gets a source picker before the system-voice rows.
// 描述 (default) = a voice designed from a description; 台灣口音 = takes cloned from a Taiwan system
// voice; 我的錄音 = the author's recording (consent first); 系統聲音 = Apple only.
// 產生 makes four takes; 選用 keeps one (the character then speaks every sentence in that voice).
// Server: /api/tts (status, job), /api/tts/download, /api/tts/cancel, /api/characters/{id}/voice…
// Wraps LoreVoice.characterRows (voice.js) and exportCharacter (app.js) without changing them.
(() => {
  const T = { status: null, job: null, poll: null, exportRef: {} };
  const enc = encodeURIComponent;
  const el = (tag, attrs = {}, ...kids) => {
    const n = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k.startsWith("on")) n.addEventListener(k.slice(2), v); else if (k === "class") n.className = v; else n.setAttribute(k, v === true ? "" : v);
    }
    n.append(...kids.flat().filter((x) => x != null));
    return n;
  };
  const icons = (n) => { if (typeof paintIcons === "function") paintIcons(n); };
  const toast = (m, o) => window.LoreShell?.toast(m, o);
  async function call(method, path, body) {
    const r = await fetch(path, { method, cache: "no-store", headers: body !== undefined ? { "content-type": "application/json" } : {}, body: body !== undefined ? JSON.stringify(body) : undefined });
    const v = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(v.error || `${r.status}`);
    return v;
  }
  const gb = (b) => `${(b / 1e9).toFixed(1)} GB`;
  const MODES = [["design", "描述"], ["accent", "台灣口音"], ["clone", "我的錄音"], ["system", "系統聲音"]];
  const EXAMPLE = "三十多歲男性，嗓音低沉略帶沙啞，語速慢，冷靜，台灣口音";

  let player = null;
  function play(url, btn) {
    if (player && !player.paused) { player.pause(); if (player._btn === btn) { player = null; return; } }
    player = new Audio(url); player._btn = btn;
    player.addEventListener("error", () => toast("播不了", { err: true }), { once: true });
    player.play().catch(() => {});
  }
  const playBtn = (label, run) => { const b = el("button", { type: "button", class: "btn", "aria-label": label }, el("span", { "data-icon": "play" }), label); b.addEventListener("click", () => run(b)); return b; };

  function characterRows(c, d, save, row) {
    const base = orig(c, d, save, row);
    // the system-voice picker is the fallback of the custom ones
    const k = base[0]?.querySelector(".k"); if (k) k.textContent = "系統聲音";
    const box = el("div", { class: "tts-box" });
    const src = el("select", { class: "set-input", "aria-label": "聲音來源" }, MODES.map(([v, t]) => el("option", { value: v }, t)));
    const srcRow = row("聲音", src);
    const pitch = el("input", { type: "range", min: "-6", max: "6", step: "0.5", "aria-label": "音高" });
    const pitchOut = el("span", { class: "set-val" });
    const pitchRow = row("音高", pitch, pitchOut);
    const showPitch = () => { const p = Number(pitch.value); pitchOut.textContent = p > 0 ? `+${p}` : `${p}`; };
    pitch.addEventListener("input", showPitch);
    let V = null;
    const put = async (patch) => { try { V = await call("PUT", `/api/characters/${enc(c.id)}/voice`, patch); paint(); } catch (e) { toast(e.message, { err: true }); } };
    pitch.addEventListener("change", () => put({ pitch: Number(pitch.value) }));
    src.addEventListener("change", () => put({ mode: src.value }));

    async function load() {
      try {
        [V, T.status] = await Promise.all([call("GET", `/api/characters/${enc(c.id)}/voice`), call("GET", "/api/tts")]);
        T.job = T.status.job; T.status = T.status.status;
      } catch { V = V ?? { mode: "system", pitch: 0, taiwan: true }; }
      paint();
      if (T.job?.running) watch();
    }

    function watch() {
      clearInterval(T.poll);
      T.poll = setInterval(async () => {
        if (!box.isConnected) { clearInterval(T.poll); return; }
        try { const v = await call("GET", "/api/tts"); T.status = v.status; T.job = v.job; } catch { return; }
        paint();
        if (!T.job.running) { clearInterval(T.poll); if (T.job.error) toast(T.job.error, { err: T.job.error !== "已取消" }); }
      }, 700);
    }

    function progress() {
      const j = T.job;
      const bar = el("progress", { max: String(j.total || 1), value: String(j.done || 0), class: "tts-bar" });
      const label = j.kind === "download" ? `${j.phase} · ${gb(j.done)} / ${gb(j.total)}` : `產生中 ${j.phase}`;
      const stop = el("button", { type: "button", class: "btn" }, "取消");
      stop.addEventListener("click", () => call("POST", "/api/tts/cancel").catch(() => {}));
      return el("div", { class: "tts-progress" }, bar, el("span", { class: "tts-note" }, label), stop);
    }

    function paint() {
      if (!V) return;
      src.value = V.mode; pitch.value = String(V.pitch ?? 0); showPitch();
      const custom = V.mode !== "system";
      pitchRow.hidden = !custom;
      box.replaceChildren();
      if (!custom) return;
      const st = T.status ?? {};
      const busy = T.job?.running;
      if (st.why) { box.append(row("", el("span", { class: "tts-note" }, st.why))); return; }
      if (T.job?.kind === "download" && busy) { box.append(row("", progress())); return; }
      if (!st.ready || (V.mode === "design" && !st.design)) {
        const b = el("button", { type: "button", class: "btn" }, el("span", { "data-icon": "download" }), `下載 Qwen3-TTS · ${gb(st.missing_bytes || 0)}`);
        b.addEventListener("click", async () => { try { T.job = await call("POST", "/api/tts/download"); paint(); watch(); } catch (e) { toast(e.message, { err: true }); } });
        box.append(row("", b, el("span", { class: "tts-note" }, "未下載：先用系統聲音")));
        icons(box);
        return;
      }
      if (V.mode === "design" || V.mode === "accent") paintMake(busy);
      else paintClone();
      paintChosen();
      icons(box);
    }

    function paintMake(busy) {
      // what is typed or picked survives the repaints while takes are being made
      const key = `${c.id}:${V.mode}`;
      T.draft ??= {};
      let input;
      if (V.mode === "design") {
        input = el("textarea", { class: "set-input", rows: "2", placeholder: EXAMPLE, "aria-label": "聲音描述" });
        input.value = T.draft[key] ?? (V.chosen?.kind === "design" ? V.chosen.description : "");
        box.append(row("描述", input));
      } else {
        input = el("select", { class: "set-input", "aria-label": "口音參考" }, el("option", { value: "" }, "自動（最好的台灣聲音）"));
        T.twVoices ??= fetch("/api/voice/voices").then((r) => r.json()).then((v) => (v.voices ?? []).filter((x) => x.language === "zh-TW")).catch(() => []);
        T.twVoices.then((list) => {
          for (const x of list) input.append(el("option", { value: x.id }, x.name));
          input.value = T.draft[key] ?? (V.chosen?.kind === "accent" ? V.chosen.apple_voice : "");
        });
        box.append(row("口音參考", input));
      }
      input.addEventListener(V.mode === "design" ? "input" : "change", () => { T.draft[key] = input.value; });
      const make = el("button", { type: "button", class: "btn", disabled: busy }, el("span", { "data-icon": "spark" }), "產生");
      make.addEventListener("click", async () => {
        const body = V.mode === "design" ? { kind: "design", description: input.value.trim() || EXAMPLE } : { kind: "accent", apple_voice: input.value };
        try { T.job = await call("POST", `/api/characters/${enc(c.id)}/voice/candidates`, body); paint(); watch(); } catch (e) { toast(e.message, { err: true }); }
      });
      const tw = el("input", { type: "checkbox", checked: V.taiwan !== false });
      tw.addEventListener("change", () => put({ taiwan: tw.checked }));
      const han = el("input", { type: "checkbox", checked: !!V.han });
      han.addEventListener("change", () => put({ han: han.checked }));
      box.append(row("", make, el("label", { class: "song-opt" }, tw, "台灣讀音"), el("label", { class: "song-opt" }, han, "和 讀 hàn")));
      const j = T.job;
      if (j?.kind === "candidates" && j.character === c.id && (j.running || j.source === V.mode)) {
        if (busy) box.append(row("", progress()));
        if (j.candidates?.length) {
          const takes = el("div", { class: "tts-takes" }, j.candidates.map((n) => {
            const use = el("button", { type: "button", class: "btn" }, "選用");
            use.addEventListener("click", async () => {
              try { V = await call("POST", `/api/characters/${enc(c.id)}/voice/pick`, { n }); T.picked = `${c.id}:${n}`; paint(); } catch (e) { toast(e.message, { err: true }); }
            });
            return el("span", { class: "tts-take" + (T.picked === `${c.id}:${n}` ? " on" : "") }, playBtn(String(n), (b) => play(`/api/characters/${enc(c.id)}/voice/candidates/${n}?t=${Date.now()}`, b)), use);
          }));
          box.append(row("候選", takes));
        }
      }
    }

    function paintClone() {
      const has = V.has_reference;
      const consent = el("input", { type: "checkbox", checked: !!V.clone?.consent });
      const text = el("input", { class: "set-input tts-text", value: V.clone?.text ?? "", placeholder: "錄音裡說的話（逐字）", "aria-label": "逐字稿" });
      text.addEventListener("change", () => put({ clone_text: text.value }));
      if (has) consent.addEventListener("change", () => put({ consent: consent.checked }));
      const zone = el("button", { type: "button", class: "btn song-drop", title: "wav、m4a、mp3，3–30 秒" }, el("span", { "data-icon": "upload" }), has ? `換錄音…（${(V.reference_seconds ?? 0).toFixed(1)} 秒）` : "選擇錄音…");
      const upload = async (file) => {
        if (!file) return;
        if (!consent.checked) { toast("先勾選「這是我的聲音或我有權使用」", { err: true }); return; }
        try {
          const r = await fetch(`/api/characters/${enc(c.id)}/voice/reference?consent=1&text=${enc(text.value)}`, { method: "PUT", body: file });
          const v = await r.json().catch(() => ({}));
          if (!r.ok) throw new Error(v.error || `${r.status}`);
          V = v; paint();
        } catch (e) { toast(e.message, { err: true }); }
      };
      zone.addEventListener("click", () => {
        const f = el("input", { type: "file", accept: ".wav,.m4a,.mp3,.aac,.flac,audio/*", hidden: true });
        f.addEventListener("change", () => { upload(f.files[0]); f.remove(); });
        document.body.append(f); f.click();
      });
      zone.addEventListener("dragover", (e) => { e.preventDefault(); zone.classList.add("over"); });
      zone.addEventListener("dragleave", () => zone.classList.remove("over"));
      zone.addEventListener("drop", (e) => { e.preventDefault(); zone.classList.remove("over"); upload(e.dataTransfer.files[0]); });
      box.append(row("", el("label", { class: "song-opt" }, consent, "這是我的聲音或我有權使用")));
      const parts = [zone];
      if (has) {
        parts.push(playBtn("聽", (b) => play(`/api/characters/${enc(c.id)}/voice/clip/reference?t=${Date.now()}`, b)));
        const rm = el("button", { type: "button", class: "ico small", title: "移除錄音", "aria-label": "移除錄音" }, el("span", { "data-icon": "trash" }));
        rm.addEventListener("click", async () => {
          if (!(await window.LoreShell.ask(rm, ["移除錄音？"], [{ label: "取消", value: null }, { label: "移除", value: true, danger: true }]))) return;
          try { V = await call("DELETE", `/api/characters/${enc(c.id)}/voice/reference`); paint(); } catch (e) { toast(e.message, { err: true }); }
        });
        parts.push(rm);
      }
      box.append(row("錄音", ...parts), row("逐字稿", text));
      if (has) {
        const inc = el("input", { type: "checkbox", checked: !!T.exportRef[c.id] });
        inc.addEventListener("change", () => { T.exportRef[c.id] = inc.checked; });
        box.append(row("", el("label", { class: "song-opt" }, inc, "匯出時包含錄音")));
      }
    }

    function paintChosen() {
      const using = V.speaking_with;
      const test = playBtn("試聽", () => {
        window.LoreVoice?.stopSpeaking?.();
        call("POST", "/api/voice/speak", { text: `你好，我是${d?.name || c.name}。今天想先從哪一段開始？`, character: c.id }).catch((e) => toast(e.message, { err: true }));
      });
      const note = using ? (V.mode === "clone" ? "使用錄音" : "已選用") : (V.mode === "clone" ? "要錄音、逐字稿和勾選" : "還沒選用：先用系統聲音");
      box.append(row("目前", el("span", { class: "tts-note" }, note), test));
    }

    load();
    // 聲音 (source) · its rows · 系統聲音 · 語速 · 音高 · 主題曲…
    return [srcRow, box, base[0], base[1], pitchRow, ...base.slice(2)].filter(Boolean);
  }

  let orig = null;
  function hook() {
    if (!window.LoreVoice || orig) return;
    orig = window.LoreVoice.characterRows;
    window.LoreVoice.characterRows = characterRows;
    // 匯出: the recording only when 「匯出時包含錄音」 is ticked
    if (typeof window.exportCharacter === "function") {
      const plain = window.exportCharacter;
      window.exportCharacter = async (c) => {
        if (!c || !T.exportRef[c.id]) return plain(c);
        try {
          const r = await fetch(`/api/characters/${enc(c.id)}/export?voice_ref=1`, { cache: "no-store" });
          if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error || `${r.status}`);
          const a = el("a", { href: URL.createObjectURL(await r.blob()), download: `${c.id}.zip` });
          document.body.append(a); a.click(); a.remove();
          toast(`已匯出 ${c.id}.zip（含錄音）`);
        } catch (e) { toast(e.message, { err: true }); }
      };
    }
  }
  hook();
  if (!orig) document.addEventListener("DOMContentLoaded", hook);
})();
