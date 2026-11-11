// 論文報告 (home tile 01 開始): a PaperComposer paper or a LoreReader paper becomes a
// paper-report deck (POST /api/todeck, crates/server/src/todeck.rs), then opens here.
// PaperComposer's 做成簡報 calls the same route from its side.
"use strict";

const PD = { src: "pc", list: null, pick: null, busy: false };
const pdPref = (() => {
  let v = {};
  try { v = JSON.parse(localStorage.getItem("sc.todeck") ?? "{}") ?? {}; } catch {}
  return { get: (k, d) => v[k] ?? d, set: (k, x) => { v[k] = x; try { localStorage.setItem("sc.todeck", JSON.stringify(v)); } catch {} } };
})();

function pdSeg(el, key, def) {
  const cur = pdPref.get(key, def);
  el.querySelectorAll("button").forEach((b) => {
    b.setAttribute("aria-pressed", String(b.dataset.v === cur));
    b.onclick = () => { pdPref.set(key, b.dataset.v); el.querySelectorAll("button").forEach((x) => x.setAttribute("aria-pressed", String(x === b))); };
  });
}

async function openPaperDeck() {
  if (PD.busy) { $("#dlgPaper").showModal(); return; }
  pdSeg($("#pdLen"), "length", "standard");
  pdSeg($("#pdLang"), "lang", "zh");
  $$("#pdSrc button").forEach((b) => (b.onclick = () => { PD.src = b.dataset.src; PD.pick = null; pdPapers(); }));
  // bases: the same thumbnails as 新簡報
  const box = $("#pdBases");
  box.textContent = "";
  const styles = S.info?.style_list ?? (S.info?.styles ?? []).map((id) => ({ id, name: id, previews: 0 }));
  const want = styles.some((s) => s.id === pdPref.get("style")) ? pdPref.get("style") : styles[0]?.id;
  for (const st of styles) {
    const l = document.createElement("label");
    l.className = "base-opt";
    l.title = st.name;
    l.innerHTML = `<input type="radio" name="pdbase"><div class="thumb16"></div><span></span>`;
    const r = l.querySelector("input");
    r.value = st.id;
    r.checked = st.id === want;
    r.onchange = () => pdPref.set("style", st.id);
    l.querySelector("span").textContent = st.name;
    box.append(l);
    drawStyleCover(l.querySelector(".thumb16"), st.id).catch(() => l.querySelector(".thumb16").classList.add("blank"));
  }
  $("#dlgPaper").showModal();
  PD.list = null;
  await pdPapers();
}

async function pdPapers() {
  $$("#pdSrc button").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.src === PD.src)));
  const box = $("#pdPapers");
  if (!PD.list) {
    box.innerHTML = `<div class="empty">…</div>`;
    try { PD.list = await api("/api/todeck/papers"); } catch (e) { box.innerHTML = ""; toast(e.message, true); return; }
  }
  const items = (PD.src === "pc" ? PD.list.pc : PD.list.lr.filter((p) => p.pdf)) ?? [];
  box.textContent = "";
  if (!items.length) {
    const d = document.createElement("div");
    d.className = "empty";
    d.textContent = PD.src === "pc" ? "PaperComposer 沒有論文" : "LoreReader 沒有論文";
    box.append(d);
  }
  const key = (p) => p.path ?? p.sha;
  if (!items.some((p) => key(p) === PD.pick)) PD.pick = items[0] ? key(items[0]) : null;
  for (const p of items) {
    const b = document.createElement("button");
    b.type = "button";
    b.setAttribute("role", "radio");
    b.setAttribute("aria-checked", String(key(p) === PD.pick));
    b.innerHTML = `<span class="t"></span><span class="w"></span>`;
    b.querySelector(".t").textContent = p.title;
    b.querySelector(".w").textContent = p.name ?? "";
    b.title = p.title;
    b.onclick = () => { PD.pick = key(p); box.querySelectorAll("button").forEach((x) => x.setAttribute("aria-checked", String(x === b))); };
    b.ondblclick = () => pdCreate();
    box.append(b);
  }
  $("#pdGo").disabled = !PD.pick;
}

/** Make the deck; `engine` = the other engine after a route could not run (改用本機／改用 Claude). */
async function pdCreate(engine) {
  if (PD.busy || !PD.pick) return;
  const source = PD.src === "pc" ? { kind: "pc", path: PD.pick } : { kind: "lr", sha: PD.pick };
  const style = $("#pdBases input:checked")?.value;
  PD.busy = true;
  $("#pdGo").disabled = true;
  $("#pdBar").hidden = false;
  try {
    const r = await fetch("/api/todeck", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ source, style, length: pdPref.get("length", "standard"), lang: pdPref.get("lang", "zh"), engine }) });
    const v = await r.json().catch(() => ({}));
    if (!r.ok) {
      const alt = v.alternative;
      toast(humanError(v.error ?? `${r.status}`), { err: true, action: alt ? { label: alt.label, run: () => pdCreate(alt.engine) } : null, ms: 9000 });
      return;
    }
    $("#dlgPaper").close();
    openByName(v.name);
  } catch (e) {
    toast(e.message, true);
  } finally {
    PD.busy = false;
    $("#pdBar").hidden = true;
    $("#pdGo").disabled = !PD.pick;
  }
}

(() => {
  $("#tilePaper").onclick = openPaperDeck;
  $("#paperForm").onsubmit = (e) => { e.preventDefault(); pdCreate(); };
  $$("#dlgPaper .close").forEach((b) => (b.onclick = () => $("#dlgPaper").close()));
})();
