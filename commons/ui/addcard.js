// 加入知識庫 for selected words, in any Lore app: L-Reader's 一鍵加卡 — POST /api/lore/card
// (`lore_cards::router`): a web-searched summary, then the card (L-Brain's API while it runs,
// else its folder). Served as `/addcard.js` by `lore_shell::FILES`; loaded after shell.js.
//
//   LoreAddCard.add({ title: selectedText, source: documentTitle });
"use strict";

window.LoreAddCard = (() => {
  const toast = (msg, opt) => LoreShell.toast(msg, opt);

  /** The words as a card title: one line, no quotes or trailing punctuation around them, a
   *  bracket only when it is closed ("transform (FFT)," → "transform (FFT)"), ≤ 120 characters. */
  function clean(text) {
    let t = String(text ?? "").replace(/\s+/g, " ").trim();
    for (let prev = ""; prev !== t;) {
      prev = t;
      t = t.replace(/^["'“”‘’]+|["'“”‘’.,，。:：;；!?！？]+$/g, "").trim();
      const open = (t.match(/[(（[]/g) ?? []).length, close = (t.match(/[)）\]]/g) ?? []).length;
      if (open > close) t = t.replace(/^[(（[]/, "").replace(/[(（[]$/, "").trim();
      if (close > open) t = t.replace(/[)）\]]$/, "").replace(/^[)）\]]/, "").trim();
    }
    return t.slice(0, 120);
  }

  async function add({ title, source = "" } = {}) {
    const words = clean(title);
    if (!words) { toast("先選取要加入的字詞"); return null; }
    toast(`上網查「${words}」…`);
    const r = await fetch("/api/lore/card", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ title: words, source }) }).catch(() => null);
    const v = r ? await r.json().catch(() => ({})) : { error: "沒有回應" };
    if (!r?.ok) { toast(v.error ?? `加入失敗（${r?.status ?? "—"}）`, { err: true }); return null; }
    toast(`已加入知識庫：${v.node.title}`, { action: { label: "開啟", run: () => LoreLinks.open("kb", { node: v.node.id }) }, ms: 6000 });
    return v.node;
  }

  return { add, clean };
})();
