// LoreEcho 人物 tab → 角色設定: who the character is (all optional), saved as `profile` in
// character.toml (PUT /api/characters/<id> { profile }). The server builds the system prompt from it
// (chat.rs) and sends the example pairs as earlier turns.

"use strict";

window.LoreProfile = (() => {
  const el = (tag, attrs = {}, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v; else if (k === "style") e.style.cssText = v; else e.setAttribute(k, v === true ? "" : v);
    }
    e.append(...kids.flat().filter((x) => x != null));
    return e;
  };

  /** The 角色設定 section for detail `d`; `save(patch)` writes it. */
  function section(d, save) {
    const p = structuredClone(d?.profile ?? {});
    p.catchphrases ??= []; p.examples ??= [];
    const limit = d?.background_limit ?? 800;
    const write = () => save({ profile: p });
    const row = (k, ...kids) => el("div", { class: "set-row" }, el("span", { class: "k" }, k), ...kids);
    const input = (key, label, ph = "") => {
      const i = el("input", { class: "set-input", "aria-label": label, placeholder: ph, style: "flex:1;min-width:0" });
      i.value = p[key] ?? "";
      i.addEventListener("change", () => { p[key] = i.value; write(); });
      return row(label, i);
    };

    // 身分與背景: a story; the count shows how much reaches the model
    const bg = el("textarea", { class: "set-input", rows: 5, "aria-label": "身分與背景", style: "flex:1;min-width:0;resize:vertical" });
    bg.value = p.background ?? "";
    const count = el("div", { class: "profile-count" });
    const paintCount = () => {
      const n = [...bg.value.trim()].length;
      count.textContent = n > limit ? `${n} / ${limit}：超過的部分不會給模型` : `${n} / ${limit}`;
      count.classList.toggle("over", n > limit);
    };
    bg.addEventListener("input", paintCount);
    bg.addEventListener("change", () => { p.background = bg.value; write(); });
    paintCount();

    const phrases = el("textarea", { class: "set-input", rows: 2, "aria-label": "口頭禪", placeholder: "每行一個", style: "flex:1;min-width:0;resize:vertical" });
    phrases.value = p.catchphrases.join("\n");
    phrases.addEventListener("change", () => { p.catchphrases = phrases.value.split("\n").map((x) => x.trim()).filter(Boolean); write(); });

    // 範例對話: user / character pairs
    const pairs = el("div", { class: "profile-pairs" });
    const paintPairs = () => {
      pairs.replaceChildren(...p.examples.map((ex, i) => {
        const u = el("input", { class: "set-input", "aria-label": `範例 ${i + 1}：你說`, placeholder: "你說" });
        const r = el("input", { class: "set-input", "aria-label": `範例 ${i + 1}：角色回`, placeholder: "角色回" });
        u.value = ex.user ?? ""; r.value = ex.reply ?? "";
        u.addEventListener("change", () => { ex.user = u.value; write(); });
        r.addEventListener("change", () => { ex.reply = r.value; write(); });
        const rm = el("button", { type: "button", class: "ico small", "aria-label": `移除範例 ${i + 1}`, title: "移除" }, el("span", { "data-icon": "x" }));
        rm.addEventListener("click", () => { p.examples.splice(i, 1); paintPairs(); write(); });
        return el("div", { class: "profile-pair" }, u, r, rm);
      }));
      const add = el("button", { type: "button", class: "btn" }, el("span", { "data-icon": "plus" }), "加一組");
      add.disabled = p.examples.length >= 8;
      add.addEventListener("click", () => { p.examples.push({ user: "", reply: "" }); paintPairs(); pairs.querySelector(".profile-pair:last-of-type input")?.focus(); });
      pairs.append(add);
      if (typeof paintIcons === "function") paintIcons(pairs);
    };
    paintPairs();

    return el("div", { class: "set-sec" },
      el("div", { class: "set-label" }, "角色設定"),
      el("div", { class: "set-row" }, el("span", { class: "k" }, "身分與背景"), el("div", { style: "flex:1;min-width:0;display:flex;flex-direction:column;gap:2px" }, bg, count)),
      input("speech_style", "說話風格"),
      row("口頭禪", phrases),
      input("call_user", "怎麼稱呼你"),
      input("relationship", "和你的關係"),
      input("likes", "喜歡"),
      input("dislikes", "不喜歡"),
      el("div", { class: "set-row" }, el("span", { class: "k" }, "範例對話"), pairs));
  }

  return { section };
})();
