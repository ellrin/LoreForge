// LoreShelf scene helpers: the on-demand frame scheduler and the document pictures.
// The scene: a few layer canvases (app.js) and ~25 DOM nodes moved with transforms.
//
//   Scene.frames(draw, { fps })   one rAF per invalidate(); draw(dt) returns true to keep
//                                 animating; the fps cap skips frames inside rAF (no timers)
//   Scene.paint(canvas, item, { w, h, big })   a document picture for a kind (its own colours,
//                                 never the theme: like a thumbnail in the other apps)
//   Scene.aspect(kind)            width / height of that picture
"use strict";
(function (root) {
  function frames(draw, opt) {
    // raf: the scheduled frame id; DRAWING while draw() runs. An invalidate() during draw() only
    // notes `again`: it must not schedule a second frame (that doubled the work every frame).
    const DRAWING = -1;
    let raf = 0, last = 0, lastDraw = 0, paused = false, again = false;
    const tick = (now) => {
      raf = 0;
      if (paused) return;
      const fps = opt.fps();
      if (fps < 60 && lastDraw && now - lastDraw < 1000 / fps - 2) { raf = requestAnimationFrame(tick); return; }
      const dt = Math.min(64, last ? now - last : 16);
      last = now; lastDraw = now;
      raf = DRAWING; again = false;
      let keep = false;
      try { keep = draw(dt, now); } finally { raf = 0; }
      if ((keep || again) && !paused) raf = requestAnimationFrame(tick);
      else last = 0;
    };
    return {
      invalidate() {
        if (raf === DRAWING) { again = true; return; }
        if (!raf && !paused) raf = requestAnimationFrame(tick);
      },
      pause(on) { paused = on; if (on && raf > 0) { cancelAnimationFrame(raf); raf = 0; last = 0; } },
      get running() { return raf !== 0; },
    };
  }

  // ── document pictures ──
  // kind → [shape, colour, badge]; shapes: page, slide, photo, film, wave, zip, box, editor, book, folder, cube, stack, glyph
  const KIND = {
    pdf: ["page", "#c4473a", "PDF"], latex: ["page", "#2f7a5a", "TeX"], document: ["page", "#2f5fb3", "DOC"],
    text: ["page", "#6b6f7a", "TXT"], data: ["page", "#7a5ab0", "{ }"], spreadsheet: ["sheet", "#1f8a4c", "XLS"],
    slides: ["slide", "#d4731f", "PPT"], image: ["photo", "#4b78b8", ""], video: ["film", "#6a3fb0", ""],
    audio: ["wave", "#138a8a", ""], archive: ["zip", "#6a6e7e", "ZIP"], installer: ["box", "#5d6b82", "DMG"],
    code: ["editor", "#2b2f3a", "</>"], ebook: ["book", "#8a5a2b", ""], font: ["glyph", "#3d3d46", "Aa"],
    project: ["folder", "#3a7bd5", ""], package: ["cube", "#7a6a55", ""], dataset: ["mosaic", "#2f8a7a", ""],
    projects: ["folder", "#3a7bd5", ""], packages: ["cube", "#7a6a55", ""], datasets: ["mosaic", "#2f8a7a", ""],
  };
  const aspect = (kind) => ({ mosaic: 1, slide: 16 / 10, photo: 4 / 3, film: 16 / 10, wave: 4 / 3, folder: 5 / 4, cube: 1, stack: 1, box: 1, glyph: 1, book: 0.8 })[(KIND[kind] ?? ["page"])[0]] ?? 0.76;
  const SANS = '"Noto Sans TC", -apple-system, "PingFang TC", sans-serif';
  function seed(s) { let h = 2166136261; for (const ch of String(s)) h = Math.imul(h ^ ch.charCodeAt(0), 16777619); return () => { h = Math.imul(h ^ (h >>> 13), 0x5bd1e995) >>> 0; return (h & 0xffff) / 0x10000; }; }
  const rr = (c, x, y, w, h, r) => { c.beginPath(); c.roundRect(x, y, w, h, r); };

  function paint(cv, item, { w, h, big = false }) {
    const dpr = Math.min(2, devicePixelRatio || 1);
    cv.width = Math.round(w * dpr); cv.height = Math.round(h * dpr);
    const c = cv.getContext("2d");
    c.setTransform(dpr, 0, 0, dpr, 0, 0);
    let [shape, col, badge] = KIND[item.kind] ?? ["page", "#6b6f7a", (item.ext || "").toUpperCase().slice(0, 4)];
    if (item.sensitive) shape = "idcard";   // a generic card, never the document itself
    const r = seed(item.id), u = Math.min(w, h) / 100;
    const ink = "#24231f", line = "#cfccc3", paper = "#fbfaf6";
    const title = (x, y, maxW, size, lines = 2) => {
      if (!big) return;
      c.fillStyle = ink; c.font = `700 ${size}px ${SANS}`; c.textBaseline = "top";
      const name = item.name.replace(/\.[^.]{1,8}$/, "");
      let row = "", n = 0;
      for (const ch of name) {
        if (c.measureText(row + ch).width > maxW) { c.fillText(row, x, y + n * size * 1.25); row = ch; if (++n >= lines) return; } else row += ch;
      }
      if (row) c.fillText(row, x, y + n * size * 1.25);
    };
    const badgeText = (x, y, size, color = col) => { if (!badge) return; c.fillStyle = color; c.font = `800 ${size}px ${SANS}`; c.textAlign = "center"; c.textBaseline = "middle"; c.fillText(badge, x, y); c.textAlign = "left"; };
    const lines = (x, y, ww, n, gap, th) => { c.fillStyle = line; for (let i = 0; i < n; i++) c.fillRect(x, y + i * gap, ww * (i % 5 === 4 ? 0.55 : 0.85 + r() * 0.15), th); };
    c.save();
    rr(c, 0, 0, w, h, Math.max(3, 5 * u)); c.clip();
    switch (shape) {
      case "page": case "sheet": {
        c.fillStyle = paper; c.fillRect(0, 0, w, h);
        c.fillStyle = col; c.fillRect(0, 0, w, big ? h * 0.035 : h * 0.26);
        if (big) {
          title(w * 0.1, h * 0.08, w * 0.8, Math.max(12, w * 0.07));
          if (shape === "sheet") { c.strokeStyle = "#b9d9c5"; c.lineWidth = 1; for (let i = 0; i <= 12; i++) { c.beginPath(); c.moveTo(w * 0.1, h * (0.3 + i * 0.05)); c.lineTo(w * 0.9, h * (0.3 + i * 0.05)); c.stroke(); } for (let j = 0; j <= 4; j++) { c.beginPath(); c.moveTo(w * (0.1 + j * 0.2), h * 0.3); c.lineTo(w * (0.1 + j * 0.2), h * 0.9); c.stroke(); } }
          else lines(w * 0.1, h * 0.3, w * 0.8, 14, h * 0.045, Math.max(2, h * 0.012));
          badgeText(w * 0.85, h * 0.93, Math.max(10, w * 0.06));
        } else {
          badgeText(w / 2, h * 0.13, 22 * u, "#fff");
          if (shape === "sheet") { c.strokeStyle = "#9cc9ac"; c.lineWidth = 1.2; for (let i = 0; i < 5; i++) { c.beginPath(); c.moveTo(w * 0.14, h * (0.38 + i * 0.12)); c.lineTo(w * 0.86, h * (0.38 + i * 0.12)); c.stroke(); } c.beginPath(); c.moveTo(w * 0.42, h * 0.38); c.lineTo(w * 0.42, h * 0.86); c.moveTo(w * 0.64, h * 0.38); c.lineTo(w * 0.64, h * 0.86); c.stroke(); }
          else lines(w * 0.14, h * 0.38, w * 0.72, 5, h * 0.11, h * 0.045);
        }
        break;
      }
      case "slide": {
        c.fillStyle = paper; c.fillRect(0, 0, w, h);
        c.fillStyle = col; c.fillRect(0, 0, w, h * 0.24);
        if (big) { c.save(); c.fillStyle = "#fff"; c.font = `700 ${Math.max(12, h * 0.09)}px ${SANS}`; c.textBaseline = "middle"; c.fillText(item.name.replace(/\.[^.]{1,8}$/, "").slice(0, 22), w * 0.06, h * 0.12); c.restore(); }
        else badgeText(w / 2, h * 0.12, 20 * u, "#fff");
        lines(w * 0.08, h * 0.36, w * 0.45, 4, h * 0.13, h * 0.05);
        c.fillStyle = col + "66"; c.fillRect(w * 0.6, h * 0.36, w * 0.32, h * 0.5);
        break;
      }
      case "photo": {
        const hue = Math.floor(r() * 360);
        const g = c.createLinearGradient(0, 0, 0, h); g.addColorStop(0, `hsl(${hue},55%,70%)`); g.addColorStop(1, `hsl(${(hue + 40) % 360},45%,88%)`);
        c.fillStyle = g; c.fillRect(0, 0, w, h);
        for (let k = 0; k < 3; k++) { c.fillStyle = `hsl(${(hue + 120 + k * 25) % 360},${30 + k * 10}%,${48 - k * 10}%)`; c.beginPath(); c.moveTo(0, h); for (let i = 0; i <= 8; i++) c.lineTo((w * i) / 8, h * (0.58 + k * 0.12) - r() * h * 0.16); c.lineTo(w, h); c.fill(); }
        c.fillStyle = "#fff9"; c.beginPath(); c.arc(w * 0.72, h * 0.26, Math.min(w, h) * 0.09, 0, 7); c.fill();
        break;
      }
      case "mosaic": {   // a dataset: four of its pictures
        const g = 3 * u;
        for (let k = 0; k < 4; k++) {
          const x = (k % 2) * (w / 2), y = Math.floor(k / 2) * (h / 2), hue = Math.floor(r() * 360);
          const gr = c.createLinearGradient(0, y, 0, y + h / 2); gr.addColorStop(0, `hsl(${hue},45%,62%)`); gr.addColorStop(1, `hsl(${(hue + 50) % 360},40%,84%)`);
          c.fillStyle = gr; c.fillRect(x + g / 2, y + g / 2, w / 2 - g, h / 2 - g);
          c.fillStyle = `hsl(${(hue + 160) % 360},30%,40%)`; c.beginPath(); c.moveTo(x + g, y + h / 2 - g); c.lineTo(x + w * 0.2, y + h * 0.28); c.lineTo(x + w * 0.32, y + h * 0.4); c.lineTo(x + w / 2 - g, y + h * 0.3); c.lineTo(x + w / 2 - g, y + h / 2 - g); c.fill();
        }
        break;
      }
      case "idcard": {
        c.fillStyle = "#e6edf3"; c.fillRect(0, 0, w, h);
        c.fillStyle = "#c3ced8"; rr(c, w * 0.08, h * 0.2, w * 0.3, h * 0.55, 4); c.fill();
        c.fillStyle = "#b9c4ce"; for (let i = 0; i < 4; i++) c.fillRect(w * 0.46, h * (0.24 + i * 0.13), w * (0.42 - i * 0.06), h * 0.05);
        c.fillStyle = "#9aa7b3"; c.fillRect(0, h * 0.86, w, h * 0.14);
        break;
      }
      case "film": {
        c.fillStyle = "#1d1b24"; c.fillRect(0, 0, w, h);
        c.fillStyle = "#3a3646"; for (let i = 0; i < 8; i++) { c.fillRect(w * (0.04 + i * 0.12), h * 0.04, w * 0.07, h * 0.08); c.fillRect(w * (0.04 + i * 0.12), h * 0.88, w * 0.07, h * 0.08); }
        c.fillStyle = col; c.beginPath(); c.moveTo(w * 0.4, h * 0.3); c.lineTo(w * 0.66, h * 0.5); c.lineTo(w * 0.4, h * 0.7); c.closePath(); c.fill();
        break;
      }
      case "wave": {
        c.fillStyle = "#e9f4f3"; c.fillRect(0, 0, w, h);
        c.fillStyle = col; const n = 22; for (let i = 0; i < n; i++) { const a = (0.15 + 0.7 * Math.abs(Math.sin(i * 0.7 + r() * 2))) * h * 0.6; c.fillRect(w * (0.08 + (i * 0.84) / n), h / 2 - a / 2, w * 0.024, a); }
        break;
      }
      case "zip": {
        c.fillStyle = "#e9e5da"; c.fillRect(0, 0, w, h);
        c.fillStyle = col; c.fillRect(w * 0.44, 0, w * 0.12, h * 0.6);
        c.fillStyle = "#e9e5da"; for (let i = 0; i < 9; i++) c.fillRect(w * (i % 2 ? 0.44 : 0.5), h * (0.03 + i * 0.065), w * 0.06, h * 0.03);
        c.fillStyle = col; rr(c, w * 0.39, h * 0.58, w * 0.22, h * 0.12, 3); c.fill();
        badgeText(w / 2, h * 0.84, big ? Math.max(12, w * 0.09) : 18 * u);
        break;
      }
      case "box": {
        c.fillStyle = "#e6e8ec"; c.fillRect(0, 0, w, h);
        c.fillStyle = col; c.beginPath(); c.arc(w / 2, h * 0.42, Math.min(w, h) * 0.26, 0, 7); c.fill();
        c.fillStyle = "#e6e8ec"; c.beginPath(); c.arc(w / 2, h * 0.42, Math.min(w, h) * 0.07, 0, 7); c.fill();
        badgeText(w / 2, h * 0.83, big ? Math.max(12, w * 0.09) : 18 * u);
        break;
      }
      case "editor": {
        c.fillStyle = col; c.fillRect(0, 0, w, h);
        const cs = ["#7fd1ff", "#ffcf6b", "#c792ea", "#9be28a"];
        for (let i = 0; i < (big ? 16 : 6); i++) { c.fillStyle = cs[i % 4]; c.fillRect(w * (0.1 + (i % 3) * 0.06), h * (0.12 + i * (big ? 0.05 : 0.1)), w * (0.2 + r() * 0.45), Math.max(2, h * (big ? 0.016 : 0.04))); }
        if (!big) badgeText(w / 2, h * 0.84, 18 * u, "#fff");
        break;
      }
      case "book": {
        c.fillStyle = col; c.fillRect(0, 0, w, h);
        c.fillStyle = "#00000033"; c.fillRect(0, 0, w * 0.1, h);
        c.fillStyle = "#f3e6cf"; c.fillRect(w * 0.22, h * 0.2, w * 0.62, h * 0.18);
        if (big) title(w * 0.24, h * 0.22, w * 0.58, Math.max(11, w * 0.06), 2);
        break;
      }
      case "glyph": {
        c.fillStyle = paper; c.fillRect(0, 0, w, h);
        c.fillStyle = col; c.font = `500 ${Math.min(w, h) * 0.5}px Georgia, serif`; c.textAlign = "center"; c.textBaseline = "middle"; c.fillText("Aa", w / 2, h / 2); c.textAlign = "left";
        break;
      }
      case "folder": case "cube": case "stack": {
        c.fillStyle = "#eef1f6"; c.fillRect(0, 0, w, h);
        c.fillStyle = col;
        if (shape === "folder") { rr(c, w * 0.1, h * 0.18, w * 0.35, h * 0.2, 4); c.fill(); rr(c, w * 0.1, h * 0.28, w * 0.8, h * 0.56, 6); c.fill(); }
        else if (shape === "cube") { c.beginPath(); c.moveTo(w * 0.5, h * 0.14); c.lineTo(w * 0.84, h * 0.32); c.lineTo(w * 0.84, h * 0.7); c.lineTo(w * 0.5, h * 0.88); c.lineTo(w * 0.16, h * 0.7); c.lineTo(w * 0.16, h * 0.32); c.closePath(); c.fill(); c.strokeStyle = "#eef1f6"; c.lineWidth = Math.max(1.5, 2 * u); c.beginPath(); c.moveTo(w * 0.16, h * 0.32); c.lineTo(w * 0.5, h * 0.5); c.lineTo(w * 0.84, h * 0.32); c.moveTo(w * 0.5, h * 0.5); c.lineTo(w * 0.5, h * 0.88); c.stroke(); }
        else { for (let i = 2; i >= 0; i--) { c.beginPath(); c.ellipse(w / 2, h * (0.3 + i * 0.2), w * 0.32, h * 0.1, 0, 0, 7); c.fill(); c.fillRect(w * 0.18, h * (0.3 + i * 0.2), w * 0.64, h * 0.12); } }
        if (big) { c.fillStyle = "#fff"; c.font = `700 ${Math.max(12, w * 0.07)}px ${SANS}`; c.textAlign = "center"; c.textBaseline = "middle"; c.fillText(item.name.slice(0, 16), w / 2, h * (shape === "folder" ? 0.58 : 0.5)); c.textAlign = "left"; }
        break;
      }
    }
    c.restore();
    return cv;
  }


  root.Scene = { frames, paint, aspect, KIND };
})(window);
