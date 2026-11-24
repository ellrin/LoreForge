// 預想選項 and 按住說話: the interview's candidate answers as cards, and the mic that fills a
// text field. The model proposes 2–3 candidates from the record; the author picks one (or
// two, they combine), edits, or dictates. Served as `/choices.js` by `lore_shell::FILES`;
// load it after icons.js and shell.js (the mic uses `iconButton` and `LoreShell.toast`).
//
//   LoreChoices.cards(list, {
//     onPick: (item, index) => {},          // a card was clicked
//     numbered: true,                       // 1 2 3 on the cards (false: a ＋ on each)
//     picked: (item, index) => false,       // is this card already taken (drawn "on")
//     disabled: false,                      // the cards cannot be clicked now
//     evidenceTitle: (id) => "",            // the tooltip of an evidence tag
//   })  → <div class="df-cands">
//
//   list: [{ text, evidence?: [id, …], …any fields the app needs back in onPick/picked }]
//   `text` is the candidate; a long one reads as its first clause in bold, the rest quieter;
//   each `evidence` id is a small tag under the card (its title = evidenceTitle(id)).
//   Picking by 1 / 2 / 3 on the keyboard is the page's own (it knows when its answer box is
//   being typed in) and calls the same onPick.
//
//   LoreChoices.mic(field, onFinal, opts)  → the hold-to-talk button, hidden when the machine
//   has no voice (lore-voice's shared /api/voice*). The words go into `field` as said and the
//   field is marked `data-typed` and gets an `input` event, so a poll's redraw keeps it like
//   typed text; `onFinal(text)` runs when the phrase ends. opts.language (default "zh-TW").
//   Several fields may each have a mic; one phrase is heard at a time.
//
// Colours are the theme's tokens only; the rules are injected once, when the script loads (a page may draw `.df-ev` evidence tags itself).
"use strict";

window.LoreChoices = (() => {
  const CSS = `
.df-cands { display: flex; flex-direction: column; gap: 6px; margin-top: 10px; }
.df-cand { display: grid; grid-template-columns: 18px 1fr; gap: 2px 8px; align-items: baseline; padding: 9px 12px; border: 1px solid var(--line-strong); border-radius: var(--r); background: var(--panel); color: var(--text); text-align: left; font: 13px/1.5 var(--sans); white-space: normal; cursor: pointer; }
.df-cand:hover { border-color: var(--accent-text); }
.df-cand.on { border-color: var(--accent-text); background: var(--accent-soft); }
.df-cand .n { font: 600 11px var(--mono); color: var(--faint); }
.df-cand .ev { grid-column: 2; display: flex; flex-wrap: wrap; gap: 4px; opacity: .7; }
.df-ev { padding: 0 5px; border: 1px solid var(--line); border-radius: 6px; font: 10.5px var(--mono); color: var(--muted); cursor: help; }
.df-cand .t { line-height: 1.55; }
.df-cand .t b { display: block; font-weight: 600; }
.df-cand .t .more { display: block; color: var(--muted); font-size: 12.5px; margin-top: 2px; }
.df-mic { align-self: flex-start; }
.df-mic.on { color: var(--accent-text); background: var(--accent-soft); border-color: var(--accent-text); }
`;
  function style() {
    if (document.getElementById("lore-choices-css")) return;
    const s = document.createElement("style");
    s.id = "lore-choices-css";
    s.textContent = CSS;
    document.head.append(s);
  }
  const el = (tag, cls, text) => { const n = document.createElement(tag); if (cls) n.className = cls; if (text != null) n.textContent = text; return n; };

  function cards(list, { onPick, numbered = true, picked, disabled = false, evidenceTitle } = {}) {
    style();
    const box = el("div", "df-cands");
    list.forEach((c, k) => {
      const b = el("button", `df-cand${picked?.(c, k) ? " on" : ""}`);
      b.disabled = !!disabled;
      // a long choice reads as its point, then the rest quieter
      const m = c.text.match(/^(.{8,}?[。；：]|.{8,60}?，)(.+)$/s);
      const tx = el("span", "t");
      if (m && c.text.length > 44) tx.append(el("b", "", m[1].replace(/[；：，]$/, "")), el("span", "more", m[2].trim()));
      else tx.textContent = c.text;
      b.append(el("span", "n", numbered ? String(k + 1) : "＋"), tx);
      if (c.evidence?.length) {
        const ev = el("span", "ev");
        c.evidence.forEach((e) => { const x = el("span", "df-ev", e); x.title = evidenceTitle ? evidenceTitle(e) : e; ev.append(x); });
        b.append(ev);
      }
      b.onclick = () => onPick?.(c, k);
      box.append(b);
    });
    return box;
  }

  async function call(path, body) {
    const r = await fetch(path, body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
    const raw = await r.text();
    if (!r.ok) throw new Error(raw || String(r.status));
    return raw ? JSON.parse(raw) : null;
  }

  let voiceOk = null, vEvents = null, vCur = null;
  function mic(field, onFinal, opts = {}) {
    style();
    const b = iconButton("mic", "按住說話", () => {});
    b.classList.add("df-mic");
    b.hidden = true;
    (async () => {
      if (voiceOk == null) { try { voiceOk = !!(await call("/api/voice")).available; } catch { voiceOk = false; } }
      b.hidden = !voiceOk;
    })();
    const text = (e) => { try { return JSON.parse(e.data).text ?? ""; } catch { return ""; } };
    const put = (t) => {
      if (!vCur) return;
      const f = vCur.field;
      f.value = (vCur.base + t).trimStart();
      f.dataset.typed = "1";
      f.dispatchEvent(new Event("input"));
    };
    const start = async (e) => {
      e.preventDefault();
      if (!vEvents) {
        vEvents = new EventSource("/api/voice/events");
        vEvents.addEventListener("partial", (x) => put(text(x)));
        vEvents.addEventListener("final", (x) => { const t = text(x); put(t); const c = vCur; c?.field.focus(); c?.onFinal?.(t); });
        vEvents.addEventListener("error", (x) => { let v = {}; try { v = JSON.parse(x.data ?? "{}"); } catch {} if (v.kind) { voiceOk = false; LoreShell.toast(v.message ?? "語音無法使用", { err: true }); } });
      }
      vCur = { field, onFinal, base: field.value ? `${field.value.trimEnd()} ` : "" };
      b.classList.add("on");
      try { await call("/api/voice/listen", { continuous: false, language: opts.language ?? "zh-TW" }); } catch (err) { b.classList.remove("on"); LoreShell.toast(err.message, { err: true }); }
    };
    const stop = () => { if (!b.classList.contains("on")) return; b.classList.remove("on"); call("/api/voice/listen/stop", {}).catch(() => {}); };
    b.onpointerdown = start;
    b.onpointerup = stop;
    b.onpointerleave = stop;
    return b;
  }

  style();
  return { cards, mic };
})();
