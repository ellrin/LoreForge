// LoreMathSvg: text with $…$ maths laid out once with KaTeX in a hidden element, then flattened
// into SVG — <text> runs, <rect> rules (fraction bars) and KaTeX's own <svg> paths — so the
// result has no HTML in it and prints with TeX glyphs (Chrome PDF, poppler SVG/PNG).
// Shared by the plotting tool (ui/plot/plot.js) and the flow-figure editor (ui/flow/editor.js).
//
//   await LoreMathSvg.loadKatex(base)          base = the folder of katex.min.js / katex.min.css
//   LoreMathSvg.measure(str, font, macros)     font = { family, size (pt), weight, italic, math: "katex"|"text", color }
//     → { w, h, asc, desc, body, place(x, y, { anchor, valign, rotate, part }) → svg string }
//   LoreMathSvg.hasMath(str) · .htmlOf(str, macros) · .ensureHost() · .pieces(str)
(() => {
  "use strict";
  if (window.LoreMathSvg) return;
  const S = 8; // measuring scale (sub-pixel precision for small type)
  const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  const r2 = (v) => Math.round(v * 100) / 100;
  let host = null;
  const cache = new Map();
  let katexLoading = null;
  function ensureHost() {
    if (host?.isConnected) return host;
    host = document.createElement("div");
    host.className = "lp-measure";
    host.setAttribute("aria-hidden", "true");
    host.style.cssText = "position:absolute;left:-30000px;top:0;white-space:nowrap;pointer-events:none;contain:layout style";
    document.body.append(host);
    if (!document.getElementById("lp-measure-css")) {
      const st = document.createElement("style");
      st.id = "lp-measure-css";
      st.textContent = `.lp-measure .lp-t{display:inline-block;line-height:1;white-space:pre}
.lp-measure .lp-t .katex{font-size:1.1em;line-height:1;text-rendering:geometricPrecision}
.lp-measure .lp-t.lp-mtext .katex{font-size:1em;font-family:var(--lp-font)}
.lp-measure .lp-t.lp-mtext .katex .mathnormal,.lp-measure .lp-t.lp-mtext .katex .mathit{font-family:var(--lp-font);font-style:italic}
.lp-measure .lp-t.lp-mtext .katex .mathrm,.lp-measure .lp-t.lp-mtext .katex .textrm{font-family:var(--lp-font);font-style:normal}
.lp-measure .lp-t.lp-mtext .katex .mathbf,.lp-measure .lp-t.lp-mtext .katex .textbf{font-family:var(--lp-font);font-weight:bold;font-style:normal}
.lp-measure .lp-t.lp-mtext .katex .boldsymbol{font-family:var(--lp-font);font-weight:bold;font-style:italic}`;
      document.head.append(st);
    }
    return host;
  }
  const hasMath = (s) => /(^|[^\\])\$/.test(s);
  // KaTeX's faces load lazily: a formula measured before its face arrived (∑ in KaTeX_Size1 on first use) keeps the
  // fallback's widths while the SVG names the real face — sub/superscripts then land inside the glyph. So the faces
  // are loaded before anything is measured, and whatever was measured before is measured again.
  const FACES = ["KaTeX_Main", "italic 1em KaTeX_Math", "KaTeX_Size1", "KaTeX_Size2", "KaTeX_AMS", "KaTeX_Caligraphic", "italic 1em KaTeX_Main", "bold 1em KaTeX_Main"];
  let facesReady = null;
  function loadFaces() {
    if (facesReady) return facesReady;
    if (!document.fonts?.load) return (facesReady = Promise.resolve());
    facesReady = Promise.all(FACES.map((f) => document.fonts.load(f.includes("1em") ? f.replace("1em", "20px") : `20px ${f}`, "∑∫x").catch(() => []))).then(() => cache.clear());
    return facesReady;
  }
  function loadKatex(base) {
    if (window.katex && document.querySelector("link[data-lp-katex], link[href*='katex']")) return loadFaces();
    if (katexLoading) return katexLoading;
    katexLoading = new Promise((ok) => {
      let pending = 2;
      const done = () => { if (--pending === 0) ok(); };
      const have = document.querySelector("link[data-lp-katex], link[href*='katex.min.css']");
      if (!have) {
        const l = document.createElement("link");
        l.rel = "stylesheet";
        l.href = base + "katex.min.css";
        l.dataset.lpKatex = "1";
        l.onload = l.onerror = done;
        document.head.append(l);
      } else done();
      if (window.katex) return done();
      const s = document.createElement("script");
      s.src = base + "katex.min.js";
      s.onload = done;
      s.onerror = done;
      document.head.append(s);
    }).then(loadFaces);
    return katexLoading;
  }
  /** Split on unescaped $…$ into [{math:bool, s}]. */
  function pieces(str) {
    const out = [];
    let buf = "", math = false;
    for (let i = 0; i < str.length; i++) {
      const c = str[i];
      if (c === "\\" && str[i + 1] === "$") { buf += math ? "\\$" : "$"; i++; continue; }
      if (c === "$") { if (buf) out.push({ math, s: buf }); buf = ""; math = !math; continue; }
      buf += c;
    }
    if (buf) out.push({ math: math && false, s: math ? "$" + buf : buf });
    return out;
  }
  function htmlOf(str, macros) {
    return pieces(str)
      .map((p) => {
        if (!p.math) return esc(p.s);
        if (!window.katex) return esc(p.s);
        try {
          return window.katex.renderToString(p.s, { throwOnError: false, output: "html", macros: { ...(macros ?? {}) }, strict: "ignore" });
        } catch {
          return esc(p.s);
        }
      })
      .join("");
  }
  const famAttr = (f) => f.replace(/"/g, "'");
  /**
   * Lay out `str` in font `f` ({ family, size, weight, italic, math, color }) and flatten it.
   * Returns { w, asc, desc, place(x, y, { anchor, rotate, valign }) → svg string }.
   */
  function measure(str, f, macros) {
    str = String(str ?? "");
    const key = JSON.stringify([str, f, macros ? Object.keys(macros).length : 0]);
    if (cache.has(key)) return cache.get(key);
    const h = ensureHost();
    const span = document.createElement("span");
    span.className = "lp-t" + (f.math === "text" ? " lp-mtext" : "");
    span.style.cssText = `font-family:${f.family};--lp-font:${f.family};font-size:${f.size * S}px;font-weight:${f.weight ?? 400};font-style:${f.italic ? "italic" : "normal"};color:${f.color ?? "#000"}`;
    span.innerHTML = htmlOf(str, macros) || "​";
    h.append(span);
    const box = span.getBoundingClientRect();
    const probe = document.createElement("span");
    probe.style.cssText = "display:inline-block;width:0;height:0;vertical-align:baseline;padding:0;margin:0;border:0";
    span.insertBefore(probe, span.firstChild);
    const base = probe.getBoundingClientRect().bottom;
    probe.remove();
    const X = (v) => r2((v - box.left) / S), Y = (v) => r2((v - base) / S);
    const parts = [];
    // text runs
    // \textcolor{…}{…} (equation figures colour each variable): a node's own colour when it differs from the run's
    const runInk = getComputedStyle(span).color;
    const own = (c) => (c && c !== runInk ? c : f.color ?? "#000");
    const walker = document.createTreeWalker(span, NodeFilter.SHOW_TEXT);
    const nodes = [];
    for (let n = walker.nextNode(); n; n = walker.nextNode()) if (n.nodeValue && n.nodeValue !== "​" && n.nodeValue.trim() !== "") nodes.push(n);
    for (const n of nodes) {
      const parent = n.parentElement;
      const cs = getComputedStyle(parent);
      if (cs.display === "none") continue;
      const range = document.createRange();
      range.selectNodeContents(n);
      const rr = range.getClientRects()[0] ?? range.getBoundingClientRect();
      parent.insertBefore(probe, n);
      const bl = probe.getBoundingClientRect().bottom;
      probe.remove();
      const fs = r2(parseFloat(cs.fontSize) / S);
      const wt = cs.fontWeight === "400" || cs.fontWeight === "normal" ? "" : ` font-weight="${cs.fontWeight}"`;
      const it = cs.fontStyle === "normal" ? "" : ` font-style="${cs.fontStyle}"`;
      parts.push(`<text x="${X(rr.left)}" y="${Y(bl)}" font-family="${famAttr(cs.fontFamily)}" font-size="${fs}"${wt}${it} fill="${own(cs.color)}" xml:space="preserve">${esc(n.nodeValue)}</text>`);
    }
    // rules (fraction bars, over/underlines) and KaTeX's own svg paths
    for (const el of span.querySelectorAll("*")) {
      if (el.tagName.toLowerCase() === "svg") {
        const r = el.getBoundingClientRect();
        let clip = el.parentElement;
        while (clip && clip !== span && getComputedStyle(clip).overflow !== "hidden") clip = clip.parentElement;
        const c = clip && clip !== span ? clip.getBoundingClientRect() : r;
        const inner = el.cloneNode(true);
        inner.setAttribute("x", r2((r.left - c.left) / S));
        inner.setAttribute("y", r2((r.top - c.top) / S));
        inner.setAttribute("width", r2(r.width / S));
        inner.setAttribute("height", r2(r.height / S));
        inner.removeAttribute("style");
        inner.setAttribute("fill", own(getComputedStyle(el).color));
        const xml = new XMLSerializer().serializeToString(inner).replace(/ xmlns="http:\/\/www\.w3\.org\/2000\/svg"/, "");
        parts.push(`<svg x="${X(c.left)}" y="${Y(c.top)}" width="${r2(c.width / S)}" height="${r2(c.height / S)}" overflow="hidden">${xml}</svg>`);
        continue;
      }
      if (el.closest("svg")) continue;
      const cs = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      for (const side of ["Top", "Bottom"]) {
        const w = parseFloat(cs[`border${side}Width`]);
        if (w > 0 && cs[`border${side}Style`] !== "none" && r.width > 0) {
          const y = side === "Top" ? r.top : r.bottom - w;
          parts.push(`<rect x="${X(r.left)}" y="${Y(y)}" width="${r2(r.width / S)}" height="${r2(w / S)}" fill="${own(cs[`border${side}Color`])}"/>`);
        }
      }
      for (const side of ["Left", "Right"]) {
        const w = parseFloat(cs[`border${side}Width`]);
        if (w > 0 && cs[`border${side}Style`] !== "none" && r.height > 0) {
          const x = side === "Left" ? r.left : r.right - w;
          parts.push(`<rect x="${X(x)}" y="${Y(r.top)}" width="${r2(w / S)}" height="${r2(r.height / S)}" fill="${own(cs[`border${side}Color`])}"/>`);
        }
      }
    }
    span.remove();
    const empty = !str.trim();
    const out = {
      w: empty ? 0 : r2(box.width / S),
      asc: empty ? 0 : r2((base - box.top) / S),
      desc: empty ? 0 : r2((box.bottom - base) / S),
      body: parts.join(""),
    };
    out.h = r2(out.asc + out.desc);
    /** anchor: start | middle | end; valign: baseline | middle | top | bottom (before rotation). */
    out.place = (x, y, o = {}) => {
      if (empty) return "";
      const dx = o.anchor === "middle" ? -out.w / 2 : o.anchor === "end" ? -out.w : 0;
      const dy = o.valign === "middle" ? (out.asc - out.desc) / 2 : o.valign === "top" ? out.asc : o.valign === "bottom" ? -out.desc : 0;
      const rot = o.rotate ? ` rotate(${o.rotate})` : "";
      const part = o.part ? ` data-part="${esc(o.part)}"` : "";
      return `<g transform="translate(${r2(x)} ${r2(y)})${rot} translate(${r2(dx)} ${r2(dy)})"${part}>${out.body}</g>`;
    };
    cache.set(key, out);
    if (cache.size > 4000) cache.delete(cache.keys().next().value);
    return out;
  }

  window.LoreMathSvg = { S, esc, r2, ensureHost, loadKatex, pieces, htmlOf, hasMath, measure };
})();
