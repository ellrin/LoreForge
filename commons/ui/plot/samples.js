// Synthetic sample figures (no real results): the demo, the gallery and the tests use them.
// LorePlotSamples.list() → [{ id, label, spec }]
(() => {
  let seed = 42;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  const gauss = () => Math.sqrt(-2 * Math.log(rnd() + 1e-12)) * Math.cos(2 * Math.PI * rnd());
  const r3 = (v) => Math.round(v * 1000) / 1000;

  function list(style = "clean") {
    seed = 42;
    const methods = ["Baseline", "+ Pretrain", "Ours"];
    // grouped bars with error bars
    const bars = {
      columns: ["metric", "Baseline", "Baseline_sd", "+ Pretrain", "+ Pretrain_sd", "Ours", "Ours_sd"],
      rows: [["Dice", 0.78, 0.03, 0.81, 0.025, 0.86, 0.02], ["IoU", 0.66, 0.035, 0.69, 0.03, 0.75, 0.025], ["Sensitivity", 0.82, 0.04, 0.84, 0.03, 0.9, 0.02], ["Specificity", 0.91, 0.02, 0.92, 0.02, 0.94, 0.015]],
    };
    // lines over a threshold
    const th = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6];
    const line = { columns: ["threshold", ...methods], rows: th.map((t, i) => [t, r3(0.93 - i * i * 0.025), r3(0.9 - i * i * 0.022), r3(0.98 - i * i * 0.015)]) };
    // embedding clusters
    const emb = { columns: ["dim1", "dim2", "class"], rows: [] };
    [["Normal", -2, 1], ["Fracture", 2, 0.5], ["Implant", 0, -2.2]].forEach(([c, mx, my]) => {
      for (let i = 0; i < 60; i++) emb.rows.push([r3(mx + gauss() * 0.8), r3(my + gauss() * 0.6), c]);
    });
    // distributions
    const dist = { columns: ["Baseline", "+ Pretrain", "Ours"], rows: [] };
    for (let i = 0; i < 80; i++) dist.rows.push([r3(0.78 + gauss() * 0.06), r3(0.81 + gauss() * 0.05), r3(0.86 + gauss() * 0.035)]);
    // ROC from scores
    const roc = { columns: ["truth", "Baseline", "Ours"], rows: [] };
    for (let i = 0; i < 300; i++) {
      const y = rnd() < 0.45 ? 1 : 0;
      roc.rows.push([y, r3(y * 0.9 + gauss() * 0.55), r3(y * 1.6 + gauss() * 0.55)]);
    }
    // confusion matrix
    const cm = { columns: ["true", "Normal", "Fracture", "Implant"], rows: [["Normal", 412, 31, 7], ["Fracture", 24, 388, 13], ["Implant", 5, 9, 211]] };
    // Bland–Altman
    const ba = { columns: ["manual", "auto"], rows: [] };
    for (let i = 0; i < 60; i++) { const v = 20 + rnd() * 60; ba.rows.push([r3(v), r3(v + 1.2 + gauss() * 2.5)]); }
    // Kaplan–Meier
    const km = { columns: ["months", "event", "group"], rows: [] };
    for (const [g, rate] of [["Low risk", 0.02], ["High risk", 0.06]]) for (let i = 0; i < 60; i++) { const t = r3(-Math.log(rnd() + 1e-9) / rate); km.rows.push([Math.min(t, 48), t < 48 && rnd() > 0.15 ? 1 : 0, g]); }
    // histogram
    const hist = { columns: ["Baseline", "Ours"], rows: dist.rows.map((r) => [r[0], r[2]]) };
    // radar
    const radar = { columns: ["metric", "Baseline", "Ours"], rows: [["Dice", 0.78, 0.86], ["IoU", 0.66, 0.75], ["Sens.", 0.82, 0.9], ["Spec.", 0.91, 0.94], ["PPV", 0.8, 0.88]] };

    const base = (p, data, extra = {}) => ({ lore_plot: 1, style, set: {}, size: null, grid: { cols: 1, rows: 1, gap: 5, labels: "none" }, data: { t1: data }, panels: [p], caption: "", ...extra });
    const ser = (names, err) => names.map((n) => (err ? { y: n, err: `${n}_sd`, label: n } : { y: n, label: n }));
    return [
      { id: "bar", label: "分組長條＋誤差線＋顯著性", spec: base({ type: "bar", data: "t1", x: "metric", series: ser(methods, true), xaxis: { label: "" }, yaxis: { label: "Score", min: 0.5, max: 1.0 }, legend: { pos: "above", cols: 3 }, opts: { mode: "grouped" }, notes: [{ type: "bracket", x: 0, s: 0, x2: 0, s2: 2, text: "**" }, { type: "bracket", x: 1, s: 1, x2: 1, s2: 2, text: "*" }] }, bars) },
      { id: "line", label: "折線（數學式標籤）", spec: base({ type: "line", data: "t1", x: "threshold", series: ser(methods), xaxis: { label: "IoU threshold $\\tau$" }, yaxis: { label: "Sensitivity $S(\\tau)$", min: 0, max: 1.05 }, legend: { pos: "lower left" }, opts: {} }, line) },
      { id: "scatter", label: "群集（t-SNE 風格，95% 橢圓）", spec: base({ type: "scatter", data: "t1", x: "dim1", y: "dim2", group: "class", series: [], xaxis: { label: "t-SNE 1" }, yaxis: { label: "t-SNE 2" }, legend: { pos: "upper right" }, opts: { ellipse: true } }, emb) },
      { id: "violin", label: "小提琴圖＋散點", spec: base({ type: "violin", data: "t1", series: ser(methods), xaxis: { label: "" }, yaxis: { label: "$\\mathrm{Dice}$" }, opts: { points: true } }, dist) },
      { id: "box", label: "盒鬚圖", spec: base({ type: "box", data: "t1", series: ser(methods), xaxis: { label: "" }, yaxis: { label: "Dice" }, opts: { mean: true }, notes: [{ type: "bracket", x: 0, x2: 2, y: 1.0, text: "$p<0.01$" }] }, dist) },
      { id: "roc", label: "ROC（圖例含 AUC，放大小圖）", spec: base({ type: "roc", data: "t1", series: [{ score: "Baseline", truth: "truth", label: "Baseline" }, { score: "Ours", truth: "truth", label: "Ours" }], xaxis: { label: "False positive rate" }, yaxis: { label: "True positive rate" }, legend: { pos: "lower right" }, opts: { auc: true, diagonal: true }, inset: { x: [0, 0.2], y: [0.6, 1], at: [0.38, 0.38, 0.4, 0.38] } }, roc) },
      { id: "heatmap", label: "混淆矩陣", spec: base({ type: "heatmap", data: "t1", x: "true", series: [], xaxis: { label: "Predicted" }, yaxis: { label: "True" }, opts: { cmap: "blues", values: true, colorbar: true } }, cm) },
      { id: "hist", label: "直方圖", spec: base({ type: "hist", data: "t1", series: ser(["Baseline", "Ours"]), xaxis: { label: "Dice" }, yaxis: { label: "Count" }, opts: { bins: 14 } }, hist) },
      { id: "bland", label: "Bland–Altman", spec: base({ type: "bland", data: "t1", series: [{ y: "auto", y2: "manual" }], xaxis: { label: "Mean of methods", unit: "mm" }, yaxis: { label: "Auto − manual", unit: "mm" } }, ba) },
      { id: "km", label: "Kaplan–Meier", spec: base({ type: "km", data: "t1", x: "months", event: "event", group: "group", series: [], xaxis: { label: "Time", unit: "months" }, yaxis: { label: "Survival probability" }, legend: { pos: "lower left" } }, km) },
      { id: "radar", label: "雷達圖", spec: base({ type: "radar", data: "t1", x: "metric", series: ser(["Baseline", "Ours"]), yaxis: { min: 0.5, max: 1 }, legend: { pos: "upper right" } }, radar) },
      {
        id: "panels", label: "多格 a/b/c（論文風格的一排三格）",
        spec: {
          lore_plot: 1, style, set: {}, size: { w: 175, h: 55 }, grid: { cols: 3, rows: 1, gap: 6, labels: "a" }, caption: "",
          data: { t1: bars, t2: line, t3: roc },
          panels: [
            { type: "bar", data: "t1", x: "metric", series: ser(methods, true), title: "Segmentation", yaxis: { label: "Score", min: 0.5, max: 1 }, legend: { show: false }, opts: {} },
            { type: "line", data: "t2", x: "threshold", series: ser(methods), title: "Localisation", xaxis: { label: "Threshold $\\tau$" }, yaxis: { label: "Sensitivity", min: 0, max: 1.05 }, legend: { pos: "lower left" }, opts: {} },
            { type: "roc", data: "t3", series: [{ score: "Baseline", truth: "truth", label: "Baseline" }, { score: "Ours", truth: "truth", label: "Ours" }], title: "Classification", xaxis: { label: "FPR" }, yaxis: { label: "TPR" }, legend: { pos: "lower right" }, opts: { auc: true } },
          ],
        },
      },
    ];
  }
  window.LorePlotSamples = { list };
})();
