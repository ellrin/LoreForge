// Synthetic samples (no real results): one per format code (B1 … A3, C1–C4) plus composite pages.
//   await LorePlotSamples.all(style) → [{ id, label, spec }]   (needs plot.js, charts.js, formats.js)
//   LorePlotSamples.list(style)      → the same, synchronous, once the catalogue is loaded
(() => {
  const LP = window.LorePlot;
  let seed = 42;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  const gauss = () => Math.sqrt(-2 * Math.log(rnd() + 1e-12)) * Math.cos(2 * Math.PI * rnd());
  const r3 = (v) => Math.round(v * 1000) / 1000;
  const T = (columns, rows, name = "synthetic") => ({ name, columns, rows });
  const M3 = ["Baseline", "+ Pretrain", "Ours"];

  function data() {
    seed = 42;
    const d = {};
    d.bars = T(["metric", "Baseline", "Baseline_sd", "+ Pretrain", "+ Pretrain_sd", "Ours", "Ours_sd"], [["Dice", 0.78, 0.03, 0.81, 0.025, 0.86, 0.02], ["IoU", 0.66, 0.035, 0.69, 0.03, 0.75, 0.025], ["Sensitivity", 0.82, 0.04, 0.84, 0.03, 0.9, 0.02], ["Specificity", 0.91, 0.02, 0.92, 0.02, 0.94, 0.015]]);
    d.few = T(["method", "Dice"], [["U-Net", 0.842], ["nnU-Net", 0.861], ["TransUNet", 0.855], ["Ours", 0.884]]);
    const organs = ["Liver", "Spleen", "Left kidney", "Right kidney", "Pancreas", "Stomach", "Gallbladder", "Aorta", "Inferior vena cava", "Portal and splenic veins", "Esophagus", "Left adrenal gland", "Right adrenal gland", "Duodenum"];
    d.many = T(["organ", "Dice"], organs.map((o) => [o, r3(0.62 + rnd() * 0.33)]));
    d.stack = T(["site", "Normal", "Mild", "Moderate", "Severe"], [["Site A", 412, 120, 64, 18], ["Site B", 288, 96, 70, 31], ["Site C", 351, 140, 52, 12], ["Site D", 199, 77, 41, 22]]);
    d.pair = T(["dataset", "Before", "After"], ["Internal", "External A", "External B", "External C", "Low-dose", "Pediatric"].map((n) => { const a = 0.7 + rnd() * 0.15; return [n, r3(a), r3(a + 0.03 + rnd() * 0.07)]; }));
    const ep = Array.from({ length: 120 }, (_, k) => k + 1);
    d.train = T(["epoch", "Train", "Validation"], ep.map((e) => [e, r3(1.6 * Math.exp(-e / 25) + 0.15 + gauss() * 0.04), r3(1.6 * Math.exp(-e / 30) + 0.25 + gauss() * 0.05 + (e > 80 ? (e - 80) * 0.002 : 0))]));
    const lr = [1e-5, 3e-5, 1e-4, 3e-4, 1e-3, 3e-3, 1e-2, 3e-2];
    d.sweep = T(["learning rate", "Dice"], lr.map((v) => [v, r3(0.86 - 0.04 * Math.abs(Math.log10(v) + 3.3) ** 1.6 + gauss() * 0.004)]));
    d.runs = T(["epoch", ...Array.from({ length: 10 }, (_, k) => `seed ${k + 1}`)], Array.from({ length: 50 }, (_, e) => [e + 1, ...Array.from({ length: 10 }, (_, k) => r3(0.5 + 0.38 * (1 - Math.exp(-(e + 1) / (8 + k))) + gauss() * 0.008))]));
    d.two = T(["epoch", "Loss", "Accuracy"], ep.slice(0, 80).map((e) => [e, r3(1.2 * Math.exp(-e / 18) + 0.1 + gauss() * 0.02), r3(0.95 - 0.5 * Math.exp(-e / 15) + gauss() * 0.008)]));
    d.band = T(["fraction of labels", "Ours", "Ours_sd", "Baseline", "Baseline_sd"], [0.05, 0.1, 0.2, 0.4, 0.6, 0.8, 1].map((f) => [f, r3(0.62 + 0.25 * Math.sqrt(f)), r3(0.04 * (1 - f) + 0.01), r3(0.5 + 0.3 * Math.sqrt(f)), r3(0.05 * (1 - f) + 0.012)]));
    const th = Array.from({ length: 21 }, (_, k) => k / 20);
    d.thresh = T(["threshold", "Sensitivity", "Specificity"], th.map((t) => [t, r3(1 / (1 + Math.exp((t - 0.62) * 12))), r3(1 / (1 + Math.exp(-(t - 0.38) * 12)))]));
    d.emb = T(["dim1", "dim2", "class"], []);
    [["Normal", -3, 1.2, 600], ["Fracture", 2.6, 0.8, 500], ["Implant", 0, -2.8, 400]].forEach(([c, mx, my, n]) => { for (let i = 0; i < n; i++) d.emb.rows.push([r3(mx + gauss() * 1.1), r3(my + gauss() * 0.8), c]); });
    d.corr = T(["Manual volume (mL)", "Predicted volume (mL)"], Array.from({ length: 150 }, () => { const v = 20 + rnd() * 160; return [r3(v), r3(v * 0.96 + 3 + gauss() * 9)]; }));
    d.ba = T(["manual", "auto"], Array.from({ length: 80 }, () => { const v = 20 + rnd() * 60; return [r3(v), r3(v + 1.2 + gauss() * 2.5)]; }));
    d.bubble = T(["params (M)", "Dice", "GFLOPs", "family"], [["CNN", 7, 0.82, 40], ["CNN", 31, 0.85, 110], ["CNN", 65, 0.86, 260], ["Transformer", 22, 0.84, 90], ["Transformer", 88, 0.87, 410], ["Transformer", 150, 0.875, 720], ["Hybrid", 14, 0.85, 70], ["Hybrid", 41, 0.88, 190]].map(([f, p, dc, g]) => [p, dc, g, f]));
    d.percase = T(["Baseline Dice", "Ours Dice"], Array.from({ length: 90 }, () => { const a = 0.55 + rnd() * 0.4; return [r3(a), r3(Math.min(0.99, a + 0.03 + gauss() * 0.04))]; }));
    d.big = T(["x", "y"], Array.from({ length: 30000 }, () => { const a = gauss(), b = gauss(); return [r3(a), r3(0.6 * a + 0.8 * b)]; }));
    d.dist = T(M3, Array.from({ length: 60 }, () => [r3(0.78 + gauss() * 0.06), r3(0.81 + gauss() * 0.05), r3(0.86 + gauss() * 0.035)]));
    d.long = T(["site", "scanner", "Dice"], []);
    for (const s of ["Site A", "Site B", "Site C"]) for (const [sc, m] of [["1.5 T", 0.8], ["3 T", 0.85]]) for (let i = 0; i < 30; i++) d.long.rows.push([s, sc, r3(m + (s === "Site C" ? -0.04 : 0) + gauss() * 0.04)]);
    const regions = ["Frontal", "Parietal", "Temporal", "Occipital", "Insula", "Cingulate", "Thalamus", "Caudate", "Putamen", "Pallidum", "Hippocampus", "Amygdala"];
    d.manyg = T(["region", "Dice"], []);
    regions.forEach((r, k) => { const m = 0.6 + rnd() * 0.3; for (let i = 0; i < 25; i++) d.manyg.rows.push([r, r3(m + gauss() * 0.04)]); });
    d.paired = T(["Before", "After"], Array.from({ length: 20 }, () => { const a = 0.6 + rnd() * 0.25; return [r3(a), r3(a + 0.04 + gauss() * 0.03)]; }));
    d.cm = T(["true", "Normal", "Fracture", "Implant", "Other"], [["Normal", 412, 31, 7, 9], ["Fracture", 24, 388, 13, 6], ["Implant", 5, 9, 211, 4], ["Other", 11, 8, 6, 97]]);
    const vars = ["Age", "BMI", "Volume", "Density", "Score", "Dice"];
    const X = Array.from({ length: 200 }, () => { const z = gauss(); return vars.map((_, k) => z * (k % 2 ? 0.7 : -0.4) + gauss() * (0.6 + k * 0.1)); });
    const cc = (i, j) => { const a = X.map((r) => r[i]), b = X.map((r) => r[j]); const ma = a.reduce((s, v) => s + v, 0) / a.length, mb = b.reduce((s, v) => s + v, 0) / b.length; let sab = 0, saa = 0, sbb = 0; a.forEach((v, k) => { sab += (v - ma) * (b[k] - mb); saa += (v - ma) ** 2; sbb += (b[k] - mb) ** 2; }); return r3(sab / Math.sqrt(saa * sbb)); };
    d.corrm = T(["var", ...vars], vars.map((v, i) => [v, ...vars.map((_, j) => cc(i, j))]));
    d.results = T(["method", "BraTS", "KiTS", "LiTS", "ACDC"], [["U-Net", 0.812, 0.861, 0.794, 0.889], ["nnU-Net", 0.851, 0.884, 0.822, 0.912], ["TransUNet", 0.838, 0.871, 0.817, 0.905], ["Swin-UNETR", 0.846, 0.889, 0.819, 0.909], ["Ours", 0.863, 0.882, 0.836, 0.918]]);
    const att = Array.from({ length: 16 }, (_, i) => Array.from({ length: 16 }, (_, j) => r3(Math.exp(-((i - 6) ** 2 + (j - 9) ** 2) / 18) + 0.6 * Math.exp(-((i - 12) ** 2 + (j - 4) ** 2) / 8) + rnd() * 0.05)));
    d.att = T(["row", ...att[0].map((_, j) => String(j + 1))], att.map((r, i) => [String(i + 1), ...r]));
    const lrs = ["1e-5", "3e-5", "1e-4", "3e-4", "1e-3"], wds = ["0", "1e-5", "1e-4", "1e-3", "1e-2"];
    d.abl = T(["learning rate", ...wds], lrs.map((l, i) => [l, ...wds.map((_, j) => r3(0.86 - 0.012 * ((i - 2.2) ** 2 + (j - 1.6) ** 2 * 0.6) + rnd() * 0.004))]));
    const scores = (n, sep, k) => Array.from({ length: n }, () => { const y = rnd() < 0.42 ? 1 : 0; return [y, ...k.map((s) => r3(y * sep * s + gauss() * 0.6))]; });
    d.roc = T(["truth", "ResNet-50", "DenseNet-121", "Ours"], scores(400, 1.4, [0.9, 1.05, 1.5]));
    d.close = T(["truth", "Baseline", "Ours"], scores(500, 2.2, [1, 1.04]));
    d.perclass = T(["truth", "Normal", "Fracture", "Implant"], scores(300, 1.5, [1.4, 1.1, 1.7]));
    d.prob = T(["truth", "Ours", "Baseline"], Array.from({ length: 800 }, () => { const p = rnd(); const y = rnd() < p ? 1 : 0; return [y, r3(Math.min(0.999, Math.max(0.001, p * 0.9 + 0.05 + gauss() * 0.05))), r3(Math.min(0.999, Math.max(0.001, p ** 0.6 + gauss() * 0.08)))]; }));
    d.hist = T(["Dice"], Array.from({ length: 300 }, () => [r3(Math.min(0.99, 0.84 + gauss() * 0.06 - (rnd() < 0.1 ? 0.2 : 0)))]));
    d.ridge = T(["Week 1", "Week 2", "Week 4", "Week 8", "Week 12", "Week 24"], Array.from({ length: 120 }, () => [0, 1, 2, 3, 4, 5].map((k) => r3(40 + k * 6 + gauss() * (8 - k * 0.6)))));
    const grid = (n, f) => { const rows = []; for (let i = 0; i < n; i++) for (let j = 0; j < n; j++) { const x = -2 + (4 * i) / (n - 1), y = -2 + (4 * j) / (n - 1); rows.push([r3(x), r3(y), r3(f(x, y))]); } return rows; };
    const land = (x, y) => 0.6 * Math.log(1 + x * x + 2 * y * y) - 0.5 * Math.exp(-((x - 1) ** 2 + (y + 0.5) ** 2) * 2) - 0.35 * Math.exp(-((x + 1.1) ** 2 + (y - 0.8) ** 2) * 3);
    d.contour = T(["$\\theta_1$", "$\\theta_2$", "loss"], grid(31, land));
    d.surface = T(["$\\theta_1$", "$\\theta_2$", "loss"], grid(25, land));
    d.cloud = T(["x", "y", "z", "cluster"], []);
    [["A", 0, 0, 0], ["B", 3, 1, 2], ["C", 1, 3, -1]].forEach(([c, a, b, e]) => { for (let i = 0; i < 120; i++) d.cloud.rows.push([r3(a + gauss() * 0.7), r3(b + gauss() * 0.7), r3(e + gauss() * 0.7), c]); });
    const vol = (u, v, w) => Math.exp(-((u - 0.5) ** 2 + (v - 0.45) ** 2 + (w - 0.5) ** 2) * 9) + 0.6 * Math.exp(-((u - 0.62) ** 2 + (v - 0.6) ** 2 + (w - 0.42) ** 2) * 160) + 0.25 * (((u - 0.5) ** 2 + (v - 0.5) ** 2) < 0.2 ? 1 : 0);
    const slice = (fx) => { const n = 40; const rows = Array.from({ length: n }, (_, i) => [String(i), ...Array.from({ length: n }, (_, j) => r3(fx(i / (n - 1), j / (n - 1))))]); return T(["row", ...Array.from({ length: n }, (_, j) => String(j))], rows); };
    d.axial = { ...slice((i, j) => vol(j, i, 0.5)), name: "Axial" };
    d.coronal = { ...slice((i, j) => vol(j, 0.5, 1 - i)), name: "Coronal" };
    d.sagittal = { ...slice((i, j) => vol(0.5, j, 1 - i)), name: "Sagittal" };
    d.pie = T(["finding", "cases", "cases (external)"], [["Normal", 512, 301], ["Fracture", 248, 190], ["Implant", 131, 84], ["Other", 64, 61]]);
    d.pieMany = T(["diagnosis", "cases"], [["Normal", 820], ["Fracture", 310], ["Arthritis", 220], ["Implant", 140], ["Dislocation", 44], ["Tumour", 21], ["Infection", 18], ["Cyst", 14], ["Foreign body", 9], ["Other", 7]]);
    d.radar = T(["metric", "U-Net", "nnU-Net", "Ours"], [["Dice", 0.81, 0.85, 0.88], ["IoU", 0.69, 0.74, 0.79], ["Sens.", 0.83, 0.86, 0.9], ["Spec.", 0.92, 0.93, 0.95], ["PPV", 0.8, 0.84, 0.87], ["Speed", 0.9, 0.55, 0.75]]);
    d.radarRef = T(["metric", "Ours", "Mean of others"], d.radar.rows.map((r) => [r[0], r[3], r3((r[1] + r[2]) / 2)]));
    return d;
  }

  const base = (style, p, tables) => ({ lore_plot: 1, style, set: {}, size: null, grid: { cols: 1, rows: 1, gap: 5, labels: "none" }, data: tables, panels: [p], caption: "" });
  const ser = (names, err) => names.map((n) => (err ? { y: n, err: `${n}_sd`, label: n } : { y: n, label: n }));
  const ours = (list) => list.map((s) => (/Ours/.test(s.label ?? s.y ?? s.score ?? "") ? { ...s, ours: true } : s));

  /** The figure for one format: synthetic data, the format applied. */
  function sample(code, style, d, st) {
    const P = (type, extra, tbl = "t1") => ({ type, data: tbl, legend: {}, opts: {}, notes: [], ...extra });
    let spec;
    switch (code) {
      case "B1": spec = base(style, P("bar", { x: "method", series: [{ y: "Dice", label: "Dice" }], xaxis: { label: "" }, yaxis: { label: "Dice", min: 0.82, max: 0.9 } }), { t1: d.few }); break;
      case "B2": spec = base(style, P("bar", { x: "organ", series: [{ y: "Dice" }], xaxis: { label: "" }, yaxis: { label: "Dice" } }), { t1: d.many }); break;
      case "B3": case "B7": spec = base(style, P("bar", { x: "metric", series: ours(ser(M3, true)), xaxis: { label: "" }, yaxis: { label: "Score", min: 0.5, max: 1.0 } }), { t1: d.bars }); break;
      case "B4": spec = base(style, P("bar", { x: "site", series: ser(["Normal", "Mild", "Moderate", "Severe"]), xaxis: { label: "" }, yaxis: { label: "Cases" } }), { t1: d.stack }); break;
      case "B5": spec = base(style, P("dumbbell", { x: "dataset", series: ser(["Before", "After"]), xaxis: { label: "Dice" } }), { t1: d.pair }); break;
      case "B6": spec = base(style, P("bar", { x: "metric", series: ours(ser(M3, true)), xaxis: { label: "" }, yaxis: { label: "Score", min: 0.5, max: 1 } }), { t1: d.bars }); break;
      case "L1": spec = base(style, P("line", { x: "epoch", series: ser(["Train", "Validation"]), xaxis: { label: "Epoch" }, yaxis: { label: "Loss" } }), { t1: d.train }); break;
      case "L2": spec = base(style, P("line", { x: "learning rate", series: [{ y: "Dice", label: "Dice" }], xaxis: { label: "Learning rate" }, yaxis: { label: "Dice" }, legend: { show: false } }), { t1: d.sweep }); break;
      case "L3": spec = base(style, P("line", { x: "epoch", series: ser(d.runs.columns.slice(1)), xaxis: { label: "Epoch" }, yaxis: { label: "Validation Dice" } }), { t1: d.runs }); break;
      case "L4": spec = base(style, P("line", { x: "epoch", series: ser(["Loss", "Accuracy"]), xaxis: { label: "Epoch" }, yaxis: { label: "" }, legend: { show: false } }), { t1: d.two }); break;
      case "L5": spec = base(style, P("line", { x: "fraction of labels", series: ours(ser(["Ours", "Baseline"], true)), xaxis: { label: "Fraction of labelled data" }, yaxis: { label: "Dice" } }), { t1: d.band }); break;
      case "L6": spec = base(style, P("line", { x: "threshold", series: ser(["Sensitivity", "Specificity"]), xaxis: { label: "Decision threshold $t$" }, yaxis: { label: "Rate", min: 0, max: 1.02 } }), { t1: d.thresh }); break;
      case "S1": spec = base(style, P("scatter", { x: "dim1", y: "dim2", group: "class", series: [], xaxis: { label: "UMAP 1" }, yaxis: { label: "UMAP 2" } }), { t1: d.emb }); break;
      case "S2": spec = base(style, P("scatter", { x: "Manual volume (mL)", y: "Predicted volume (mL)", series: [], xaxis: { label: "Manual", unit: "mL" }, yaxis: { label: "Predicted", unit: "mL" } }), { t1: d.corr }); break;
      case "S3": spec = base(style, P("bland", { series: [{ y: "auto", y2: "manual" }], xaxis: { label: "Mean of methods", unit: "mm" }, yaxis: { label: "Auto − manual", unit: "mm" } }), { t1: d.ba }); break;
      case "S4": spec = base(style, P("scatter", { x: "params (M)", y: "Dice", size: "GFLOPs", group: "family", series: [], xaxis: { label: "Parameters", unit: "M" }, yaxis: { label: "Dice" } }), { t1: d.bubble }); break;
      case "S5": spec = base(style, P("scatter", { x: "Baseline Dice", y: "Ours Dice", series: [], xaxis: { label: "Baseline Dice" }, yaxis: { label: "Ours Dice" } }), { t1: d.percase }); break;
      case "S6": spec = base(style, P("scatter", { x: "x", y: "y", series: [], xaxis: { label: "Feature 1" }, yaxis: { label: "Feature 2" } }), { t1: d.big }); break;
      case "V1": spec = base(style, P("box", { series: ours(ser(M3)), xaxis: { label: "" }, yaxis: { label: "Dice" } }), { t1: d.dist }); break;
      case "V2": spec = base(style, P("box", { group: "site", hue: "scanner", y: "Dice", series: [], xaxis: { label: "" }, yaxis: { label: "Dice" } }), { t1: d.long }); break;
      case "V3": spec = base(style, P("box", { group: "region", y: "Dice", series: [], xaxis: { label: "Dice" }, yaxis: { label: "" } }), { t1: d.manyg }); break;
      case "V4": spec = base(style, P("box", { series: ser(["Before", "After"]), xaxis: { label: "" }, yaxis: { label: "Dice" } }), { t1: d.paired }); break;
      case "V5": spec = base(style, P("violin", { series: ours(ser(M3)), xaxis: { label: "" }, yaxis: { label: "Dice" } }), { t1: d.dist }); break;
      case "H1": spec = base(style, P("heatmap", { x: "true", series: [], xaxis: { label: "Predicted" }, yaxis: { label: "True" } }), { t1: d.cm }); break;
      case "H2": spec = base(style, P("heatmap", { x: "var", series: [], xaxis: { label: "" }, yaxis: { label: "" } }), { t1: d.corrm }); break;
      case "H3": spec = base(style, P("heatmap", { x: "method", series: [], xaxis: { label: "" }, yaxis: { label: "" } }), { t1: d.results }); break;
      case "H4": spec = base(style, P("heatmap", { x: "row", series: [], xaxis: { label: "" }, yaxis: { label: "" } }), { t1: d.att }); break;
      case "H5": spec = base(style, P("heatmap", { x: "learning rate", series: [], xaxis: { label: "Weight decay" }, yaxis: { label: "Learning rate" } }), { t1: d.abl }); break;
      case "R1": spec = base(style, P("roc", { series: ours(["ResNet-50", "DenseNet-121", "Ours"].map((s) => ({ score: s, truth: "truth", label: s }))), xaxis: { label: "False positive rate" }, yaxis: { label: "True positive rate" } }), { t1: d.roc }); break;
      case "R2": spec = base(style, P("roc", { series: ours(["Baseline", "Ours"].map((s) => ({ score: s, truth: "truth", label: s }))), xaxis: { label: "False positive rate" }, yaxis: { label: "True positive rate" } }), { t1: d.close }); break;
      case "R3": spec = base(style, P("pr", { series: ours(["ResNet-50", "DenseNet-121", "Ours"].map((s) => ({ score: s, truth: "truth", label: s }))), xaxis: { label: "Recall" }, yaxis: { label: "Precision" } }), { t1: d.roc }); break;
      case "R4": spec = base(style, P("roc", { series: ["Normal", "Fracture", "Implant"].map((s) => ({ score: s, truth: "truth", label: s })), xaxis: { label: "FPR" }, yaxis: { label: "TPR" } }), { t1: d.perclass }); break;
      case "R5": spec = base(style, P("calib", { series: ours([{ score: "Ours", truth: "truth", label: "Ours" }, { score: "Baseline", truth: "truth", label: "Baseline" }]), xaxis: { label: "Predicted probability" }, yaxis: { label: "Observed fraction" } }), { t1: d.prob }); break;
      case "R6": spec = base(style, P("dca", { series: ours([{ score: "Ours", truth: "truth", label: "Ours" }, { score: "Baseline", truth: "truth", label: "Baseline" }]), xaxis: { label: "Threshold probability" }, yaxis: { label: "Net benefit" } }), { t1: d.prob }); break;
      case "D1": spec = base(style, P("hist", { series: [{ y: "Dice" }], xaxis: { label: "Dice" }, yaxis: { label: "Cases" } }), { t1: d.hist }); break;
      case "D2": spec = base(style, P("hist", { series: ours(ser(["Baseline", "Ours"])), xaxis: { label: "Dice" }, yaxis: { label: "Density" } }), { t1: d.dist }); break;
      case "D3": spec = base(style, P("ridge", { series: ser(d.ridge.columns), xaxis: { label: "Lesion volume", unit: "mL" }, yaxis: { label: "" } }), { t1: d.ridge }); break;
      case "D4": spec = base(style, P("ecdf", { series: ours(ser(M3)), xaxis: { label: "Dice" }, yaxis: { label: "Cumulative fraction" } }), { t1: d.dist }); break;
      case "T1": spec = base(style, P("contour", { x: "$\\theta_1$", y: "$\\theta_2$", z: "loss", series: [], xaxis: { label: "$\\theta_1$" }, yaxis: { label: "$\\theta_2$" }, zaxis: { label: "$\\mathcal{L}$" } }), { t1: d.contour }); break;
      case "T2": spec = base(style, P("surface", { x: "$\\theta_1$", y: "$\\theta_2$", z: "loss", series: [], xaxis: { label: "$\\theta_1$" }, yaxis: { label: "$\\theta_2$" }, zaxis: { label: "$\\mathcal{L}$" } }), { t1: d.surface }); break;
      case "T3": spec = base(style, P("scatter3d", { x: "x", y: "y", z: "z", group: "cluster", series: [], xaxis: { label: "$z_1$" }, yaxis: { label: "$z_2$" }, zaxis: { label: "$z_3$" } }), { t1: d.cloud }); break;
      case "T4": spec = base(style, P("heatmap", { x: "row", series: [], xaxis: { label: "" }, yaxis: { label: "" } }), { t1: d.axial, t2: d.coronal, t3: d.sagittal }); break;
      case "P1": spec = base(style, P("pie", { x: "finding", series: [{ y: "cases" }] }), { t1: d.pie }); break;
      case "P2": spec = base(style, P("pie", { x: "finding", series: [{ y: "cases" }], opts: { total_label: "cases" } }), { t1: d.pie }); break;
      case "P3": spec = base(style, P("pie", { x: "diagnosis", series: [{ y: "cases" }] }), { t1: d.pieMany }); break;
      case "P4": spec = base(style, P("pie", { x: "finding", series: [{ y: "cases", label: "Internal" }, { y: "cases (external)", label: "External" }] }), { t1: d.pie }); break;
      case "A1": spec = base(style, P("radar", { x: "metric", series: ours(ser(["U-Net", "nnU-Net", "Ours"])) }), { t1: d.radar }); break;
      case "A2": spec = base(style, P("radar", { x: "metric", series: ser(["U-Net", "nnU-Net", "Ours"]), yaxis: { min: 0.4, max: 1 } }), { t1: d.radar }); break;
      case "A3": spec = base(style, P("radar", { x: "metric", series: ser(["Ours", "Mean of others"]), yaxis: { min: 0.4, max: 1 } }), { t1: d.radarRef }); break;
      default: return null;
    }
    let s = LP.applyFormat(spec, 0, code, st);
    // finishing touches a person would add
    if (code === "L1") s.panels[0].legend = { ...(s.panels[0].legend ?? {}), pos: "best" };
    if (code === "L6") s.panels[0].notes = [{ type: "point", x: 0.5, y: 0.802, text: "operating point" }];
    if (code === "S2") s.panels[0].opts.identity = true;
    if (code === "T4") s.panels.forEach((p) => { p.opts = { ...p.opts, cmap: "greys" }; });
    if (code === "P2") s.panels[0].opts.total_label = "cases";
    return s;
  }

  function composites(style, d, st) {
    const out = [];
    const P = (type, tbl, extra) => ({ type, data: tbl, legend: {}, opts: {}, notes: [], ...extra });
    const tables = { t1: d.bars, t2: d.train, t3: d.roc, t4: d.dist, t5: d.pie, t6: d.cm, t7: d.abl, t8: d.results };
    const fig = (panels, grid, size) => ({ lore_plot: 1, style, set: {}, size, grid: { gap: 5, labels: "a", ...grid }, data: tables, panels, caption: "" });
    const bar = P("bar", "t1", { x: "metric", series: ours(ser(M3, true)), xaxis: { label: "" }, yaxis: { label: "Score", min: 0.5, max: 1 }, legend: { pos: "above" } });
    const line = P("line", "t2", { x: "epoch", series: ser(["Train", "Validation"]), xaxis: { label: "Epoch" }, yaxis: { label: "Loss" }, opts: { smooth: 0.6 } });
    const roc = P("roc", "t3", { series: ours(["ResNet-50", "DenseNet-121", "Ours"].map((s) => ({ score: s, truth: "truth", label: s }))), xaxis: { label: "FPR" }, yaxis: { label: "TPR" } });
    const box = P("box", "t4", { series: ours(ser(M3)), xaxis: { label: "" }, yaxis: { label: "Dice" } });
    const pie = P("pie", "t5", { x: "finding", series: [{ y: "cases" }], opts: { labels: "direct" } });
    const heat = (t, extra) => P("heatmap", t, { x: tables[t].columns[0], series: [], xaxis: { label: "" }, yaxis: { label: "" }, opts: { colorbar: true, annot: "none", ...extra } });
    const W = st?.size?.widths ?? [85, 120, 175];
    out.push({ id: "C1", label: "C1 2×2（a–d）", spec: fig([bar, line, roc, box], { cols: 2, rows: 2 }, { w: W.at(-1), h: W.at(-1) * 0.82 }) });
    out.push({ id: "C2", label: "C2 1×3", spec: fig([bar, line, roc], { cols: 3, rows: 1 }, { w: W.at(-1), h: W.at(-1) / 3 }) });
    out.push({ id: "C3", label: "C3 左大＋右兩小", spec: fig([{ ...roc, span: 2, rowspan: 2 }, box, { ...line, legend: { show: false } }], { cols: 3, rows: 2 }, { w: W.at(-1), h: W.at(-1) / 1.6 }) });
    out.push({ id: "C4", label: "C4 共用色條", spec: fig([heat("t6", { annot: "value" }), heat("t7", { annot: "value", fmt: ".2f" }), heat("t8", { annot: "value", fmt: ".2f" })].map((p) => ({ ...p, opts: { ...p.opts, colorbar: true, fmt: p.opts.fmt } })), { cols: 3, rows: 1, share_color: false }, { w: W.at(-1), h: W.at(-1) / 3 }) });
    // a full page of line charts (3×3, shared legend and axes)
    const page = { t1: d.runs };
    const lines9 = Array.from({ length: 9 }, (_, k) => ({ type: "line", data: "t1", x: "epoch", series: ser(d.runs.columns.slice(1 + (k % 4), 4 + (k % 4))), title: `Fold ${k + 1}`, xaxis: { label: "Epoch" }, yaxis: { label: "Dice" }, legend: {}, opts: {}, notes: [] }));
    out.push({ id: "G1", label: "整頁 3×3 折線（共用圖例與軸）", spec: { lore_plot: 1, style, set: {}, size: { w: W.at(-1), h: Math.min(W.at(-1) * 1.1, st?.size?.max_h ?? 230) }, grid: { cols: 3, rows: 3, gap: 4, labels: "a", legend: "top", sharey: true, sharex: true }, data: page, panels: lines9, caption: "" } });
    out.push({ id: "G2", label: "左圓餅＋右長條", spec: fig([pie, { ...bar, span: 2 }], { cols: 3, rows: 1 }, { w: W.at(-1), h: W.at(-1) / 2.6 }) });
    out.push({ id: "G3", label: "一大兩小（上大下兩小）", spec: fig([{ ...line, span: 2 }, roc, box], { cols: 2, rows: 2 }, { w: W.at(-1), h: W.at(-1) * 0.85 }) });
    // axis demos: a stretched range, a broken axis for one outlier, symlog
    const sens = T(["epoch", "Baseline", "Ours"], Array.from({ length: 40 }, (_, k) => [k + 1, r3(Math.min(0.995, 0.5 + 0.45 * (1 - Math.exp(-(k + 1) / 6)) + gauss() * 0.006)), r3(Math.min(0.998, 0.55 + 0.44 * (1 - Math.exp(-(k + 1) / 5)) + gauss() * 0.005))]));
    out.push({ id: "X1", label: "分段拉伸：y 0–0.8 ×1、0.8–1.0 ×3", spec: { lore_plot: 1, style, set: {}, size: { w: W[0], h: W[0] * 0.75 }, grid: { cols: 1, rows: 1 }, data: { t1: sens }, panels: [{ type: "line", data: "t1", x: "epoch", series: ours(ser(["Baseline", "Ours"])), xaxis: { label: "Epoch" }, yaxis: { label: "Accuracy", min: 0.5, max: 1, scale: "stretch", segments: [[0.8, 1, 3]] }, legend: { pos: "lower right" }, opts: {}, notes: [] }], caption: "" } });
    const outl = T(["method", "Time (s)"], [["A", 1.2], ["B", 1.9], ["C", 2.4], ["D", 48.0]]);
    out.push({ id: "X2", label: "斷軸：一個離群值", spec: { lore_plot: 1, style, set: {}, size: { w: W[0], h: W[0] * 0.75 }, grid: { cols: 1, rows: 1 }, data: { t1: outl }, panels: [{ type: "bar", data: "t1", x: "method", series: [{ y: "Time (s)" }], xaxis: { label: "" }, yaxis: { label: "Inference time", unit: "s", min: 0, max: 50, scale: "stretch", segments: [[3, 46, 0]] }, legend: { show: false }, opts: { values: true }, notes: [] }], caption: "" } });
    const sym = T(["x", "Δ"], Array.from({ length: 41 }, (_, k) => { const x = k - 20; return [x, r3(Math.sign(x) * (Math.abs(x) ** 3) / 8)]; }));
    out.push({ id: "X3", label: "對稱對數 symlog", spec: { lore_plot: 1, style, set: {}, size: { w: W[0], h: W[0] * 0.75 }, grid: { cols: 1, rows: 1 }, data: { t1: sym }, panels: [{ type: "line", data: "t1", x: "x", series: [{ y: "Δ", label: "$\\Delta$" }], xaxis: { label: "$x$" }, yaxis: { label: "$\\Delta$", scale: "symlog", linthresh: 1 }, legend: { show: false }, opts: {}, notes: [] }], caption: "" } });
    return out;
  }

  function list(style = "clean", styles) {
    const d = data();
    const st = LP.styleFor(style, styles ?? [], {});
    const cat = LP.dataSync()?.formats?.formats ?? [];
    const out = [];
    for (const f of cat) {
      if (f.code.startsWith("C")) continue;
      const spec = sample(f.code, style, d, st);
      if (spec) out.push({ id: f.code, label: `${f.code} ${f.label}`, spec });
    }
    for (const c of composites(style, d, st)) out.push(c);
    return out;
  }
  window.LorePlotSamples = { list, all: async (style, styles) => { await LP.data(); return list(style, styles); }, data };
})();
