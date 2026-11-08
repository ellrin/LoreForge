// Upload a template (LaTeX or Word) and let Claude turn it into a
// PaperComposer template: name it, upload the files (and optionally the
// journal's PDF to compare with), watch Claude work and tell it what to
// change, preview against the PDF, then save it into the template library.

const PCConvert = (() => {
  let job = null;
  let from = 0;
  let timer = null;
  let host = null;

  const enc = () => encodeURIComponent(job.id);
  const ready = () => !!(state.engines?.subscription?.configured || state.engines?.api?.configured);

  /** Draw the converter into `box` (the templates window's main pane). */
  async function open(box) {
    host = box;
    clearInterval(timer);
    const last = store.get("pc.convertJob", null);
    if (last) {
      try { const r = await api(`/api/convert/${encodeURIComponent(last)}`); job = r.job; return draw(r); }
      catch { store.set("pc.convertJob", null); }
    }
    job = null;
    nameStep();
  }

  function head(title) {
    host.innerHTML = "";
    const h = el("div", "tpl-head");
    h.append(el("h3", "", title));
    const badge = el("span", "cv-badge", "Claude");
    badge.title = "需要連接 Claude";
    h.append(badge);
    host.append(h);
  }

  function nameStep() {
    head("上傳模板");
    if (!ready()) {
      const p = el("p", "empty", "需要先連接 Claude。");
      const go = el("button", "ghost", "設定 Claude");
      go.onclick = () => Setup.open("claude");
      host.append(p, go);
      return;
    }
    const name = el("input");
    name.placeholder = "例如 IEEE Transactions on Medical Imaging、碩士論文";
    const kind = el("div", "seg");
    let k = "journal";
    [["journal", "期刊"], ["conference", "研討會"], ["custom", "自訂"]].forEach(([v, label]) => {
      const b = el("button", "", label);
      b.setAttribute("aria-pressed", String(v === k));
      b.onclick = () => { k = v; kind.querySelectorAll("button").forEach((x) => x.setAttribute("aria-pressed", String(x === b))); };
      kind.append(b);
    });
    const go = el("button", "primary small", "下一步");
    go.onclick = async () => {
      if (!name.value.trim()) { name.focus(); return; }
      try {
        job = await api("/api/convert", { name: name.value, kind: k });
        store.set("pc.convertJob", job.id);
        from = 0;
        draw(await api(`/api/convert/${enc()}`));
      } catch (e) { flash(e.message, 6000); }
    };
    name.onkeydown = (e) => { if (e.key === "Enter") go.click(); };
    const form = el("div", "tpl-form");
    const f1 = el("label", "tf"); f1.append(el("span", "lbl", "名稱"), name);
    const f2 = el("div", "tf"); f2.append(el("span", "lbl", "類別"), kind);
    form.append(f1, f2);
    const acts = el("div", "tpl-actions");
    acts.append(go);
    host.append(form, acts);
    name.focus();
  }

  async function upload(role, files) {
    for (const f of files) {
      const r = await fetch(`/api/convert/${enc()}/upload?role=${role}&name=${encodeURIComponent(f.name)}`, { method: "POST", body: f });
      if (!r.ok) { flash(await r.text(), 6000); break; }
    }
    draw(await api(`/api/convert/${enc()}`));
  }

  function picker(role, accept, multiple) {
    const i = el("input");
    i.type = "file";
    i.accept = accept;
    i.multiple = multiple;
    i.onchange = () => upload(role, [...i.files]);
    i.click();
  }

  function draw(r) {
    job = r.job;
    head(job.name);
    const a = r.analysis ?? {};
    const KIND = { journal: "期刊", conference: "研討會", custom: "自訂" };
    host.querySelector(".tpl-head").append(el("span", "tpl-sub", KIND[job.kind] ?? ""));

    // uploads
    const up = el("div", "cv-up");
    const src = el("div", "cv-box");
    const srcHead = el("div", "cv-box-head");
    srcHead.append(el("b", "", "模板檔案"));
    const tip = el("button", "help", "?");
    tip.dataset.tip = "LaTeX：整個模板資料夾壓成 zip，或選取所有檔案（主檔、.cls、.sty、.bst、圖片、.bib）。Word：期刊的 .docx 範本。";
    srcHead.append(tip);
    const addSrc = el("button", "ghost small", "上傳…");
    addSrc.onclick = () => picker("source", ".zip,.tar.gz,.tgz,.tex,.cls,.sty,.bst,.bib,.clo,.cfg,.png,.jpg,.pdf,.eps,.docx", true);
    srcHead.append(addSrc);
    src.append(srcHead);
    if (a.files?.length) {
      const what = a.source === "latex" ? `LaTeX · 主檔 ${a.main}` : a.source === "word" ? "Word 範本" : "找不到主檔";
      src.append(el("div", `cv-state ${a.source === "none" ? "bad" : "ok"}`, `${what} · ${a.files.length} 個檔案`));
      if (a.missing?.length) src.append(el("div", "cv-state bad", `缺少：${a.missing.join("、")}`));
    } else src.append(el("div", "cv-state", "還沒有檔案"));
    const ref = el("div", "cv-box");
    const refHead = el("div", "cv-box-head");
    refHead.append(el("b", "", "比對用 PDF"), el("span", "tpl-sub", "選填"));
    const addRef = el("button", "ghost small", a.reference ? "更換…" : "上傳…");
    addRef.onclick = () => picker("reference", ".pdf", false);
    refHead.append(addRef);
    ref.append(refHead, el("div", `cv-state ${a.reference ? "ok" : ""}`, a.reference ? "已上傳" : "期刊排版好的論文 PDF，Claude 會照著比對版面"));
    up.append(src, ref);
    host.append(up);

    // conversation
    const log = el("div", "cv-log");
    log.id = "cvLog";
    host.append(log);
    const bar = el("div", "cv-bar");
    const say = el("textarea");
    say.rows = 2;
    say.placeholder = job.started ? "告訴 Claude 要改什麼，例如：改成雙欄、標題要置中" : "補充說明（選填）";
    const go = el("button", "primary small", job.started ? "送出" : "開始轉換");
    const stop = el("button", "ghost small", "停止");
    stop.hidden = job.status !== "running";
    go.disabled = job.status === "running" || a.source === "none" || !a.files?.length;
    go.onclick = async () => {
      go.disabled = true;
      try {
        job = await api(`/api/convert/${enc()}/run`, { instruction: say.value });
        say.value = "";
        poll();
      } catch (e) { flash(e.message, 8000); go.disabled = false; }
    };
    say.onkeydown = (e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) go.click(); };
    stop.onclick = async () => { await api(`/api/convert/${enc()}/stop`, {}); poll(); };
    bar.append(say, go, stop);
    host.append(bar);

    // preview and save
    const out = el("div", "cv-out");
    out.id = "cvOut";
    const check = el("button", "ghost", "預覽");
    check.onclick = () => preview(out, check);
    const save = el("button", "primary small", "存進模板庫");
    save.onclick = () => saveIt(r.files ?? []);
    const fresh = el("button", "ghost", "重新開始");
    fresh.onclick = () => { if (confirm("放下這次轉換，重新開始？")) { store.set("pc.convertJob", null); clearInterval(timer); job = null; nameStep(); } };
    const acts = el("div", "tpl-actions");
    acts.append(check, save, fresh);
    host.append(acts, out);
    from = 0;
    poll();
  }

  async function poll() {
    clearInterval(timer);
    const tick = async () => {
      if (!job || !document.getElementById("cvLog")) { clearInterval(timer); return; }
      let r;
      try { r = await api(`/api/convert/${enc()}/events?from=${from}`); } catch { return; }
      const log = $("#cvLog");
      const near = log.scrollHeight - log.scrollTop - log.clientHeight < 40;
      r.events.forEach((e) => log.append(el("div", `cv-ev ${e.kind}`, e.text)));
      from = r.next;
      if (near) log.scrollTop = log.scrollHeight;
      const running = r.status === "running";
      log.classList.toggle("busy", running);
      const bar = log.nextElementSibling;
      bar.querySelector(".primary").disabled = running;
      bar.querySelector(".ghost").hidden = !running;
      if (r.error && !log.querySelector(".cv-ev.fail")) log.append(el("div", "cv-ev err fail", r.error));
      if (!running) {
        clearInterval(timer);
        job.status = r.status;
        bar.querySelector(".primary").textContent = "送出";
        bar.querySelector("textarea").placeholder = "告訴 Claude 要改什麼，例如：改成雙欄、標題要置中";
      }
    };
    await tick();
    timer = setInterval(tick, 1500);
  }

  async function preview(out, btn) {
    btn.disabled = true;
    out.innerHTML = "";
    out.append(el("div", "cv-state", "編譯中…"));
    try {
      const r = await api(`/api/convert/${enc()}/check`, {});
      out.innerHTML = "";
      out.append(el("div", `cv-state ${r.ok ? "ok" : "bad"}`, r.ok ? `PaperComposer 可以用這個模板 · ${r.pages} 頁` : `無法編譯：${r.errors?.[0]?.message ?? ""}`));
      if (r.ok) {
        const pages = el("div", "cv-pages");
        const t = Date.now();
        for (let n = 1; n <= 2; n++) {
          const pair = el("div", "cv-pair");
          [["reference", "期刊"], ["output", "轉換結果"]].forEach(([which, label]) => {
            const fig = el("figure");
            const img = el("img");
            img.src = `/api/convert/${enc()}/page?which=${which}&n=${n}&t=${t}`;
            img.onerror = () => fig.remove();
            fig.append(img, el("figcaption", "", `${label} p.${n}`));
            pair.append(fig);
          });
          pages.append(pair);
        }
        out.append(pages);
      }
    } catch (e) { out.innerHTML = ""; flash(e.message, 6000); }
    btn.disabled = false;
  }

  async function saveIt() {
    const r = await api(`/api/convert/${enc()}`);
    const files = r.files ?? [];
    if (!files.includes("sample.tex")) return flash("還沒有 sample.tex：請先讓 Claude 完成轉換", 6000);
    if (!confirm(`存進模板庫的檔案（${files.length} 個）：\n\n${files.join("\n")}`)) return;
    try {
      const s = await api(`/api/convert/${enc()}/save`, {});
      store.set("pc.convertJob", null);
      store.set("pc.template", s.id);
      try { state.workspace = await api("/api/workspace"); } catch {}
      flash("已存進模板庫");
      refreshGit();
      PCTemplates.open(s.id);
    } catch (e) { flash(e.message, 6000); }
  }

  return { open };
})();
