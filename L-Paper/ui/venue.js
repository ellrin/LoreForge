// 期刊寫法: a journal template's habits, learned from about five of its own
// open-access articles (venueprof.rs, docs/long-paper.md §D).
//   PCVenue.panel({ template, paper })   the panel under the template list:
//       which journal (confirmed once) → 整理寫法 → progress → sections with a
//       proportion bar each, habits with counts, phrases; 更新 after a year
//   PCVenue.newJournal({ query, onDone })   a journal of its own from a name
//   PCVenue.summary(id)                  one line for the picker (sync, cached)
//   PCVenue.loadSummaries()              fills that cache (returns a promise)
//   PCVenue.chooseJournal(paper)         選期刊 for a paper on a base format
const PCVenue = (() => {
  let sums = null;
  let sumsP = null;

  function loadSummaries(force) {
    if (force) { sums = null; sumsP = null; }
    sumsP ??= api("/api/venue/summaries").then((v) => (sums = v || {})).catch(() => (sums = {}));
    return sumsP;
  }
  const summary = (id) => sums?.[id] ?? null;

  // the shared theme's accents (never document colours): one hue per family of categories,
  // lighter shades for the later members; the segment's hint names the category
  const FAMILIES = [
    ["--accent", ["value", "motivation", "prior"], "背景與前人"],
    ["--accent-2", ["contribution", "method", "equations"], "方法"],
    ["--accent-3", ["dataset", "setup"], "資料與設定"],
    ["--ok", ["results", "comparison", "ablation"], "結果"],
    ["--warn", ["discussion", "limitations", "future", "conclusion"], "討論與結論"],
  ];
  const SHADES = [100, 62, 36, 22];
  function tone(cat) {
    for (const [hue, ids] of FAMILIES) {
      const i = ids.indexOf(cat);
      if (i >= 0) return i ? `color-mix(in srgb, var(${hue}) ${SHADES[i]}%, var(--panel))` : `var(${hue})`;
    }
    return "var(--muted)"; // statements and the venue's own categories
  }
  const pct = (x) => `${Math.round(x * 100)}%`;
  const num = (x) => Math.round(x).toLocaleString();

  /** A stacked bar of a section's key points. */
  function bar(points, cats) {
    const b = el("div", "vp-bar");
    const total = points.reduce((s, p) => s + p.share.mean, 0) || 1;
    points.filter((p) => p.share.mean >= 0.02).forEach((p) => {
      const seg = el("span");
      seg.style.width = `${(p.share.mean / total) * 100}%`;
      seg.style.background = tone(p.category);
      seg.dataset.tip = `${p.label} ${pct(p.share.mean)}（${pct(p.share.min)}–${pct(p.share.max)}）`;
      seg.title = seg.dataset.tip;
      b.append(seg);
    });
    return b;
  }

  function panel(opts) {
    const box = el("div", "vp");
    const tpl = opts.template;
    if (!tpl) return box;
    let alive = true;
    const draw = async (v) => {
      if (!v) {
        try { v = await api(`/api/venue/t/${encodeURIComponent(tpl)}`); }
        catch { box.replaceChildren(); return; } // the demo folder, an unknown template: nothing to show
      }
      if (!alive) return;
      box.replaceChildren(...view(v));
      if (v.job?.state === "running") follow(v.job);
    };
    const follow = async (job) => {
      let seq = job.seq;
      while (alive && box.isConnected) {
        let j;
        try { j = await api(`/api/venue/jobs/${job.job || job.id}?after=${seq}`); } catch { return; }
        seq = j.seq;
        if (j.state !== "running") { loadSummaries(true); draw(); return; }
        const p = box.querySelector(".vp-progress");
        if (p) p.replaceWith(progress(j));
      }
    };
    const make = async () => {
      try {
        const j = await api(`/api/venue/t/${encodeURIComponent(tpl)}/make`, { paper: opts.paper ?? null });
        draw();
        return j;
      } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
    };

    // 用手邊的 PDF: the author's own PDFs of this journal (no OpenAlex, no download)
    const fromPdfs = () => {
      const f = el("input");
      f.type = "file"; f.accept = "application/pdf,.pdf"; f.multiple = true; f.hidden = true;
      const b = el("button", "ghost small", "用手邊的 PDF…");
      b.title = "選這本期刊的 3–6 篇 PDF（找不到網路上的文章時）";
      b.onclick = () => f.click();
      f.onchange = async () => {
        if (!f.files.length) return;
        b.disabled = true; b.textContent = "上傳中…";
        try {
          const local = [];
          for (const file of f.files) {
            const r = await fetch(`/api/venue/t/${encodeURIComponent(tpl)}/pdf?file=${encodeURIComponent(file.name)}`, { method: "POST", body: file });
            if (!r.ok) throw new Error(`${file.name}：${await r.text()}`);
            local.push(await r.json());
          }
          await api(`/api/venue/t/${encodeURIComponent(tpl)}/make`, { paper: opts.paper ?? null, local });
          draw();
        } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); b.disabled = false; b.textContent = "用手邊的 PDF…"; }
      };
      const w = el("span", "vp-pdfs");
      w.append(b, f);
      return w;
    };

    function head(title, ...right) {
      const h = el("div", "vp-head");
      h.append(el("b", "", title), el("span", "grow"), ...right);
      return h;
    }

    function progress(j) {
      const st = j.stages.find((s) => s.state === "running") ?? j.stages[j.step - 1] ?? j.stages[0];
      const c = st?.counts ?? {};
      const n = c.articles != null ? ` ${c.articles}/5` : c.labelled != null ? ` ${c.labelled}/${c.of}` : "";
      const p = el("div", "vp-progress");
      const meter = el("div", "vp-steps");
      j.stages.forEach((s) => meter.append(el("span", s.state === "done" ? "done" : s.state === "running" ? "on" : "")));
      p.append(meter, el("span", "", `${st?.label ?? ""}${n}…`));
      const stop = el("button", "ghost small", "停止");
      stop.onclick = async () => { try { await api(`/api/venue/jobs/${j.job || j.id}/stop`, {}); draw(); } catch {} };
      p.append(stop);
      return p;
    }

    function view(v) {
      if (v.template.hidden) return [];
      // which journal is it? (once)
      if (!v.identity) return identify(v);
      if (v.job?.state === "running") return [head("期刊寫法"), progress(v.job)];
      const out = [];
      const p = v.profile;
      const again = el("button", v.stale || !p ? "primary small" : "ghost small", p ? (v.stale ? "更新" : "重新整理") : "整理寫法");
      if (!p) again.prepend(icon("spark"));
      again.onclick = () => { again.disabled = true; make(); };
      const change = iconButton("pen", `換期刊（${v.identity.name}）`, () => box.replaceChildren(...identify(v, true)));
      change.classList.add("vp-pen");
      out.push(head("期刊寫法", p ? el("span", "vp-meta", `${p.made} · ${p.articles.length} 篇`) : el("span"), change, fromPdfs(), again));
      if (v.job && ["failed", "stopped", "interrupted"].includes(v.job.state)) {
        const r = el("div", "vp-err");
        r.append(el("span", "", v.job.error || "沒有完成"));
        const go = el("button", "ghost small", "繼續");
        go.onclick = async () => { try { await api(`/api/venue/jobs/${v.job.job || v.job.id}/resume`, {}); draw(); } catch (e) { LoreShell.toast(e.message, { err: true }); } };
        r.append(go);
        out.push(r);
      }
      if (!p) return out;
      const n = p.articles.length;
      const cats = p.categories;
      const body = p.sections.filter((s) => !["statements", "appendix"].includes(s.role));
      const rows = el("div", "vp-secs");
      body.forEach((s) => {
        // a row opens to the numbers: key points with share and range, the order of the
        // paragraphs, the subsections, the titles used
        const d = el("details", "vp-sec");
        const sum = el("summary");
        sum.append(el("span", "vp-title", s.titles[0]?.text ?? s.label), el("span", "vp-n", s.present < n ? `${s.present}/${n}` : ""), bar(s.points, cats), el("span", "vp-words", num(s.words.mean)));
        d.append(sum);
        const more = el("div", "vp-more");
        const pts = el("ul", "vp-points");
        s.points.filter((x) => x.share.mean >= 0.02).forEach((x) => {
          const li = el("li");
          const dot = el("i");
          dot.style.background = tone(x.category);
          li.append(dot, el("span", "", x.label), el("span", "vp-pct", pct(x.share.mean)), el("span", "vp-meta", `${pct(x.share.min)}–${pct(x.share.max)}`));
          pts.append(li);
        });
        more.append(pts);
        if (s.moves.length > 1) more.append(el("div", "vp-line", s.moves.map((m) => m.label).join(" → ")));
        if (s.subsections.length) more.append(el("div", "vp-line vp-meta", s.subsections.map((x) => `${x.text} ${x.n}/${n}`).join(" · ")));
        if (s.titles.length > 1) more.append(el("div", "vp-line vp-meta", s.titles.map((x) => `${x.text} ${x.n}/${n}`).join(" · ")));
        d.append(more);
        rows.append(d);
      });
      out.push(rows);
      // legend: the five families (the open row names each category)
      const used = new Set(body.flatMap((s) => s.points.filter((x) => x.share.mean >= 0.02).map((x) => x.category)));
      const leg = el("div", "vp-legend");
      FAMILIES.forEach(([hue, ids, name]) => {
        if (!ids.some((id) => used.has(id))) return;
        const k = el("span");
        const d = el("i");
        d.style.background = `var(${hue})`;
        k.append(d, name);
        leg.append(k);
      });
      out.push(leg);
      // habits the rows do not already show; pairs merged
      const H = Object.fromEntries((p.habits || []).map((h) => [h.id, h]));
      const usual = (h) => h && h.count * 5 >= h.of * 3;
      const shown = new Set(["related_separate", "related_in_intro", "data_section", "results_discussion_merged", "discussion_and_conclusion", "contributions_bulleted", "limitations_heading"]);
      const chips = (p.habits || []).filter((h) => usual(h) && !shown.has(h.id)).map((h) => {
        let text = `${h.label} ${h.count}/${h.of}`;
        if (h.id === "contributions_list" && usual(H.contributions_bulleted)) text = `${h.label}（條列 ${H.contributions_bulleted.count}）${h.count}/${h.of}`;
        if (h.id === "limitations" && usual(H.limitations_heading)) text = `${h.label}（獨立標題 ${H.limitations_heading.count}）${h.count}/${h.of}`;
        return el("span", `vp-chip ${h.kind}`, text);
      });
      if (chips.length) {
        const hb = el("div", "vp-chips");
        hb.append(...chips);
        out.push(hb);
      }
      // no caption read anywhere = the PDF reader missed them: say nothing rather than 0
      const floats = p.figures.max > 0 || p.tables.max > 0 ? ` · 圖 ${p.figures.mean.toFixed(0)} · 表 ${p.tables.mean.toFixed(0)} · 公式 ${p.equations.mean.toFixed(0)}` : "";
      const nums = el("div", "vp-meta", `約 ${num(p.body_words.mean)} 字${floats}`);
      out.push(nums);
      if (p.phrases?.length) {
        const d = el("details", "vp-phrases");
        d.append(el("summary", "", "常見用語"));
        const ul = el("ul");
        p.phrases.slice(0, 20).forEach((x) => { const li = el("li"); li.append(el("span", "", x.text), el("span", "vp-meta", `${cats.find((c) => c.id === x.situation)?.label ?? ""} ${x.articles}/${n}`)); ul.append(li); });
        d.append(ul);
        out.push(d);
      }
      if (v.submission) {
        const s = v.submission;
        const bits = [s.words && `${s.words} 字`, s.pages && `${s.pages} 頁`, s.abstract_words && `摘要 ${s.abstract_words} 字`, ...(s.required_statements || [])].filter(Boolean);
        if (bits.length) out.push(el("div", `vp-meta${v.submission_stale ? " vp-old" : ""}`, `投稿要求：${bits.join(" · ")}`));
      }
      return out;
    }

    // ── which journal ──
    function identify(v, changing) {
      const back = el("button", "ghost small", "取消");
      back.onclick = () => draw(v);
      const h = changing ? head("哪一本期刊？", back) : head("哪一本期刊？", fromPdfs());
      const q = el("input", "np-input vp-q");
      q.type = "search";
      q.value = changing ? v.identity.name : v.guess;
      const list = el("div", "vp-cands");
      let t = 0;
      const search = async () => {
        const my = ++t;
        list.replaceChildren(el("span", "vp-meta", "搜尋中…"));
        let r;
        try { r = await api(`/api/venue/sources?q=${encodeURIComponent(q.value)}`); }
        catch (e) { if (my === t) list.replaceChildren(el("span", "vp-meta", e.message)); return; }
        if (my !== t) return;
        list.replaceChildren();
        if (!r.sources.length) list.append(el("span", "vp-meta", "沒有找到"));
        r.sources.forEach((s) => {
          const b = el("button", "vp-cand");
          b.append(el("span", "name", s.name), el("span", "sub", [s.publisher, s.issn_l, s.kind === "conference" ? "研討會" : ""].filter(Boolean).join(" · ")));
          b.onclick = async () => {
            b.disabled = true;
            try { draw(await api(`/api/venue/t/${encodeURIComponent(tpl)}/identity`, { source: s }, "PUT")); }
            catch (e) { LoreShell.toast(e.message, { err: true }); b.disabled = false; }
          };
          list.append(b);
        });
      };
      let deb;
      q.oninput = () => { clearTimeout(deb); deb = setTimeout(search, 400); };
      search();
      return [h, q, list];
    }

    draw();
    // the panel is dropped with the list; stop following then
    const mo = new MutationObserver(() => { if (!box.isConnected) { alive = false; mo.disconnect(); } });
    setTimeout(() => box.parentNode && mo.observe(document.body, { childList: true, subtree: true }));
    return box;
  }

  /** A journal of its own: pick the journal (OpenAlex), then its template is made on the
   *  right base format and its 期刊寫法 starts. opts: query, onDone(id), onCancel(). */
  function newJournal(opts) {
    const box = el("div", "vp vp-new");
    const h = el("div", "vp-head");
    const back = el("button", "ghost small", "取消");
    back.onclick = () => opts.onCancel?.();
    h.append(el("b", "", "新增期刊"), el("span", "grow"), back);
    const q = el("input", "np-input vp-q");
    q.type = "search";
    q.value = opts.query ?? "";
    const list = el("div", "vp-cands");
    const create = async (s, base) => {
      list.replaceChildren(el("span", "vp-meta", "建立中…"));
      let r = null;
      try {
        const res = await fetch("/api/venue/journals", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ source: s, base: base ?? null }) });
        r = await res.json();
      } catch (e) { list.replaceChildren(el("span", "vp-meta", e.message)); return; }
      if (r?.id) { loadSummaries(true); opts.onDone?.(r.id); return; }
      // no base format known for this publisher: pick one
      if (!r?.bases?.length) { list.replaceChildren(el("span", "vp-meta", r?.error || "沒有建立")); return; }
      list.replaceChildren(el("span", "vp-meta", "格式"));
      r.bases.forEach((b) => { const btn = el("button", "vp-cand"); btn.append(el("span", "name", b.name), el("span", "sub", b.publisher || "")); btn.onclick = () => create(s, b.id); list.append(btn); });
    };
    let t = 0;
    const search = async () => {
      const my = ++t;
      if (q.value.trim().length < 2) { list.replaceChildren(); return; }
      list.replaceChildren(el("span", "vp-meta", "搜尋中…"));
      let r;
      try { r = await api(`/api/venue/sources?q=${encodeURIComponent(q.value)}`); }
      catch (e) { if (my === t) list.replaceChildren(el("span", "vp-meta", e.message)); return; }
      if (my !== t) return;
      list.replaceChildren();
      if (!r.sources.length) list.append(el("span", "vp-meta", "沒有找到"));
      r.sources.forEach((s) => {
        const b = el("button", "vp-cand");
        b.append(el("span", "name", s.name), el("span", "sub", [s.publisher, s.issn_l, s.kind === "conference" ? "研討會" : ""].filter(Boolean).join(" · ")));
        b.onclick = () => create(s);
        list.append(b);
      });
    };
    let deb;
    q.oninput = () => { clearTimeout(deb); deb = setTimeout(search, 400); };
    search();
    box.append(h, q, list);
    setTimeout(() => q.focus());
    return box;
  }

  /** 選期刊: move a paper on a base format onto a journal template. */
  async function chooseJournal(paper, onDone) {
    let tpls = [];
    try { tpls = (await api("/api/templates")).filter((j) => j.kind !== "book" && !j.hidden); } catch {}
    const d = el("dialog", "vp-dialog");
    let picked = "";
    const ok = el("button", "primary small", "套用");
    ok.disabled = true;
    const cancel = el("button", "ghost small", "取消");
    cancel.onclick = () => d.close();
    ok.onclick = async () => {
      ok.disabled = true;
      try {
        await api(`/api/venue/paper/${encodeURIComponent(paper)}/journal`, { template: picked });
        d.close();
        onDone?.(picked);
      } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); ok.disabled = false; }
    };
    const foot = el("div", "vp-foot");
    foot.append(el("span", "grow"), cancel, ok);
    d.append(el("h3", "", "選期刊"), PCNew.templateList({ tpls, selected: "", paper, journalsOnly: true, onPick: (v) => { picked = v; ok.disabled = !v; } }), foot);
    d.addEventListener("close", () => d.remove());
    document.body.append(d);
    d.showModal();
  }

  return { panel, newJournal, summary, loadSummaries, chooseJournal };
})();
