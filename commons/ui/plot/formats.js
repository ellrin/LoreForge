// LorePlot formats (formats.json, docs/plot.md §3b): every chart type has 4–7 formats (B1 … A3, C1–C4).
//   LorePlot.formatsFor(type)                 → formats of the chart family (+ composites)
//   LorePlot.suggestFormats(spec, pi)         → [{ code, score, why }] best first (never excludes one)
//   LorePlot.applyFormat(spec, pi, code, style) → a new spec (the panel's data mapping kept)
//   LorePlot.changeType(spec, pi, type)       → a new spec with panel pi as another chart type
//   LorePlot.dataShape(spec, pi)              → { categories, series, points, groups, err, classes, labelChars }
// The thresholds are the catalogue's tunable defaults: they rank suggestions only.
(() => {
  "use strict";
  const LP = window.LorePlot;
  const { isNum, num, col, ncol, clone, trapz, rocFrom } = LP._;
  const FAMILY = { bar: "B", dumbbell: "B", line: "L", area: "L", scatter: "S", bland: "S", box: "V", violin: "V", heatmap: "H", roc: "R", pr: "R", calib: "R", dca: "R", km: "R", hist: "D", ridge: "D", ecdf: "D", contour: "T", surface: "T", scatter3d: "T", pie: "P", radar: "A" };
  LP.FAMILY = FAMILY;
  const cat = () => LP.dataSync()?.formats ?? { formats: [], rules: {}, venues: {} };
  LP.catalogue = async () => (await LP.data()).formats;

  LP.formatsFor = function (type) {
    const fam = FAMILY[type] ?? "B";
    return cat().formats.filter((f) => f.code.startsWith(fam) || f.code.startsWith("C") || (fam === "H" && f.code === "T4"));
  };

  LP.dataShape = function (spec, pi) {
    const p = spec.panels?.[pi] ?? {};
    const t = spec.data?.[p.data] ?? Object.values(spec.data ?? {})[0] ?? { columns: [], rows: [] };
    const series = (p.series ?? []).filter((s) => !s.hidden);
    const cats = p.x ? col(t, p.x).map(String) : [];
    const groups = p.group ? new Set(col(t, p.group).map(String)).size : series.length;
    let aucGap = NaN;
    if ((p.type === "roc") && series.length > 1) {
      const aucs = series.map((s) => (s.score && s.truth ? rocFrom(ncol(t, s.score), ncol(t, s.truth)).auc : trapz(ncol(t, s.x ?? p.x), ncol(t, s.y)))).filter(isNum).sort((a, b) => b - a);
      if (aucs.length > 1) aucGap = aucs[0] - aucs[1];
    }
    const xs = p.x ? ncol(t, p.x).filter(isNum) : [];
    return {
      categories: p.type === "heatmap" ? t.rows.length : cats.length,
      series: series.length,
      points: t.rows.length,
      groups,
      classes: t.rows.length,
      err: series.some((s) => s.err),
      labelChars: Math.max(0, ...cats.map((c) => c.length)),
      decades: xs.length && Math.min(...xs) > 0 ? Math.log10(Math.max(...xs) / Math.min(...xs)) : 0,
      aucGap,
    };
  };

  const within = (v, r) => !Array.isArray(r) || (v >= r[0] && v <= r[1]);
  LP.suggestFormats = function (spec, pi) {
    const p = spec.panels?.[pi];
    if (!p) return [];
    const shape = LP.dataShape(spec, pi);
    const rules = cat().rules ?? {};
    return LP.formatsFor(p.type)
      .filter((f) => !f.code.startsWith("C"))
      .map((f) => {
        let score = f.chart === p.type ? 1 : 0.5;
        const why = [];
        for (const [k, r] of Object.entries(f.suggest ?? {})) {
          if (k === "err") { if (shape.err === r) { score += 1; why.push("有誤差欄"); } continue; }
          if (!isNum(shape[k])) continue;
          if (within(shape[k], r)) { score += 1; why.push(`${k} ${shape[k]}`); } else score -= 0.5;
        }
        if (f.code === "R2" && isNum(shape.aucGap) && shape.aucGap < (rules.inset_auc_gap ?? 0.02)) { score += 1.5; why.push(`AUC 差 ${shape.aucGap.toFixed(3)}`); }
        if (f.code === "L2" && shape.decades >= (rules.log_x_decades ?? 2)) { score += 1; why.push("x 跨數量級"); }
        if (f.code === "B2" && shape.labelChars > (rules.label_chars_horizontal ?? 12)) { score += 0.5; why.push("標籤長"); }
        return { code: f.code, label: f.label, score, why };
      })
      .sort((a, b) => b.score - a.score);
  };

  // ── changing chart type, keeping what the author set ───────────────────
  const SAME_SERIES = ["line", "area", "bar", "box", "violin", "hist", "radar", "ridge", "ecdf", "dumbbell"];
  LP.changeType = function (spec, pi, type) {
    const s = clone(spec);
    const p = s.panels[pi];
    const t = s.data[p.data] ?? Object.values(s.data)[0];
    const fresh = LP.starterPanel(type, t);
    const keep = { title: p.title, legend: p.legend, notes: p.notes, data: p.data, span: p.span, rowspan: p.rowspan };
    if (SAME_SERIES.includes(type) && SAME_SERIES.includes(p.type) && p.series?.length) fresh.series = p.series.map((q) => ({ ...q }));
    if (["line", "bar", "radar", "area", "dumbbell", "pie"].includes(type) && ["line", "bar", "radar", "area", "dumbbell", "pie"].includes(p.type)) { fresh.xaxis = p.xaxis; fresh.yaxis = p.yaxis; fresh.x = p.x; }
    if (type === "pie" && fresh.series.length > 1) fresh.series = fresh.series.slice(0, 1);
    s.panels[pi] = { ...fresh, ...Object.fromEntries(Object.entries(keep).filter(([, v]) => v !== undefined)), type };
    return s;
  };

  function deepMerge(a, b) {
    const out = clone(a) ?? {};
    for (const [k, v] of Object.entries(b ?? {})) out[k] = v && typeof v === "object" && !Array.isArray(v) && out[k] && typeof out[k] === "object" && !Array.isArray(out[k]) ? deepMerge(out[k], v) : clone(v);
    return out;
  }

  /** Width (mm) for "single" | "double" | "page" from the style's widths. */
  function widthOf(st, which) {
    const w = st?.size?.widths ?? [85, 120, 175];
    return which === "double" || which === "page" ? w.at(-1) : which === "onehalf" ? w[1] ?? w[0] : w[0];
  }

  LP.applyFormat = function (spec, pi, code, st) {
    const f = cat().formats.find((x) => x.code === code);
    if (!f) return spec;
    let s = clone(spec);
    const a = f.apply ?? {};
    // composites: arrange the panels (duplicating the current one when there are fewer)
    if (a.layout) {
      const cells = (f.spec.grid.cols ?? 1) * (f.spec.grid.rows ?? 1);
      const spans = a.spans ?? [];
      const need = spans.length ? spans.length : cells;
      while (s.panels.length < need) s.panels.push({ ...clone(s.panels[Math.min(pi, s.panels.length - 1)]), title: "" });
      s.panels.forEach((p, k) => { delete p.span; delete p.rowspan; if (spans[k]) { if (spans[k][0] > 1) p.span = spans[k][0]; if (spans[k][1] > 1) p.rowspan = spans[k][1]; } });
      s.grid = deepMerge(s.grid ?? {}, f.spec.grid);
    } else {
      if (f.chart !== "*" && f.chart !== s.panels[pi].type) s = LP.changeType(s, pi, f.chart);
      const p = s.panels[pi];
      // the format's panel values; notes are added, not replaced
      const fp = clone(f.panel ?? {});
      const addNotes = fp.notes;
      delete fp.notes;
      s.panels[pi] = deepMerge(p, fp);
      if (addNotes && !(p.notes ?? []).some((n) => n.type === addNotes[0].type)) s.panels[pi].notes = [...(p.notes ?? []), ...addNotes];
      if (f.set) s.set = { ...(s.set ?? {}), ...f.set };
      if (f.spec) s = deepMerge(s, f.spec);
      const q = s.panels[pi];
      const shape = LP.dataShape(s, pi);
      const rules = cat().rules ?? {};
      if (a.horizontal_if_label_chars && shape.labelChars > (rules.label_chars_horizontal ?? a.horizontal_if_label_chars) && !q.opts?.horizontal) {
        q.opts = { ...(q.opts ?? {}), horizontal: true };
        [q.xaxis, q.yaxis] = [q.yaxis ?? {}, q.xaxis ?? {}];
      }
      if (a.ours_last && q.series?.length && !q.series.some((x) => x.ours)) q.series[q.series.length - 1].ours = true;
      // one series: the category that is "ours" (by name, else the last one) is emphasised
      if (a.emphasis_category && (q.series?.length ?? 0) === 1 && !q.opts?.emphasis?.length) {
        const t = s.data[q.data] ?? Object.values(s.data)[0];
        const cats = col(t, q.x).map(String);
        const pick = cats.find((c) => /\b(ours|proposed)\b/i.test(c)) ?? cats.at(-1);
        if (pick != null) q.opts = { ...(q.opts ?? {}), emphasis: [pick] };
      }
      if (a.ref_last && q.series?.length > 1) q.series[q.series.length - 1].ref = true;
      if (a.highlight) q.opts = { ...(q.opts ?? {}), highlight: (q.series ?? []).slice(0, a.highlight).map((x) => x.label || x.y) };
      if (a.log_x_if_decades && shape.decades >= (rules.log_x_decades ?? a.log_x_if_decades)) q.xaxis = { ...(q.xaxis ?? {}), scale: "log" };
      if (a.brackets && q.series?.length > 1) {
        const ours = Math.max(0, q.series.findIndex((x) => x.ours));
        const si = ours || q.series.length - 1;
        const other = si === 0 ? 1 : 0;
        const cats = shape.categories || 1;
        q.notes = [...(q.notes ?? []).filter((n) => n.type !== "bracket"), ...Array.from({ length: Math.min(cats, 6) }, (_, k) => ({ type: "bracket", x: k, s: other, x2: k, s2: si, text: "*" }))];
      }
      if (a.split === "series" && (q.series?.length ?? 0) > 1) {
        const n = q.series.length;
        const base = clone(q);
        // each small multiple keeps the colour its series had in the combined chart
        const pal = LP._.colorsOf(st?.colors?.palette, LP._.ROOT_STYLE.colors.palette);
        const panels = q.series.map((ser, k) => ({ ...clone(base), series: [{ ...ser, color: ser.color ?? (q.type === "pie" ? undefined : pal[k % pal.length]) }], title: ser.label || ser.y || ser.score || "", legend: { ...(base.legend ?? {}) } }));
        s.panels.splice(pi, 1, ...panels);
        s.grid = { ...(s.grid ?? {}), cols: a.rows ? 1 : Math.min(n, 4), rows: a.rows ? n : Math.ceil(n / 4) };
      }
      if (a.split === "tables") {
        const ids = Object.keys(s.data);
        if (ids.length > 1) {
          const base = clone(q);
          s.panels.splice(pi, 1, ...ids.map((id) => ({ ...clone(base), data: id, title: s.data[id].name || id })));
          s.grid = { ...(s.grid ?? {}), cols: Math.min(ids.length, 3), rows: Math.ceil(ids.length / 3) };
        }
      }
    }
    // size: the venue width (from the style), the format's aspect
    const w = widthOf(st, f.width);
    const maxH = st?.size?.max_h ?? 170;
    s.size = { w: Math.round(w * 10) / 10, h: Math.round(Math.max(20, Math.min(maxH, w / (f.aspect || 1.333))) * 10) / 10 };
    s.format = s.panels.length > 1 && (a.layout || a.split) ? code : s.format;
    s.panels.forEach((p, k) => { if (k === pi || a.split) p.format = code; });
    return s;
  };
})();
