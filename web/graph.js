// Force-directed graph on a plain canvas (no library). Nodes are coloured by the
// member's familiarity state; tag nodes are small and hollow. Drag to move a
// node, drag the background to pan, wheel to zoom, click a card to open it.

const Graph = (() => {
  let canvas, ctx, nodes = [], edges = [], byId = new Map();
  let view = { x: 0, y: 0, k: 1 }, raf = 0, heat = 0, drag = null, onOpen = () => {}, hover = null;

  const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const colour = (n) => n.kind === "tag" ? css("--faint") : ({ learning: css("--warn"), known: css("--ok"), stale: css("--stale") })[n.state] ?? css("--faint");

  function init(el, open) {
    canvas = el;
    ctx = canvas.getContext("2d");
    onOpen = open;
    new ResizeObserver(() => { size(); draw(); }).observe(canvas);
    canvas.addEventListener("pointerdown", down);
    canvas.addEventListener("pointermove", move);
    canvas.addEventListener("pointerup", up);
    canvas.addEventListener("pointerleave", () => { hover = null; draw(); });
    canvas.addEventListener("wheel", (e) => {
      e.preventDefault();
      const p = toWorld(e);
      const k = Math.min(4, Math.max(0.2, view.k * (e.deltaY < 0 ? 1.1 : 0.9)));
      view.x = e.offsetX - p.x * k; view.y = e.offsetY - p.y * k; view.k = k;
      draw();
    }, { passive: false });
  }

  function size() {
    const r = canvas.getBoundingClientRect(), d = devicePixelRatio || 1;
    canvas.width = Math.max(1, r.width * d); canvas.height = Math.max(1, r.height * d);
    ctx.setTransform(d, 0, 0, d, 0, 0);
  }

  function set(data, center) {
    const old = byId;
    byId = new Map();
    const r = canvas.getBoundingClientRect();
    nodes = data.nodes.map((n, i) => {
      const o = old.get(n.id);
      const a = (i / Math.max(1, data.nodes.length)) * Math.PI * 2;
      const nn = { ...n, x: o?.x ?? Math.cos(a) * 160 + (Math.random() - .5) * 20, y: o?.y ?? Math.sin(a) * 160 + (Math.random() - .5) * 20, vx: 0, vy: 0, deg: 0 };
      byId.set(n.id, nn);
      return nn;
    });
    edges = data.edges.filter((e) => byId.has(e.from) && byId.has(e.to)).map((e) => ({ ...e, a: byId.get(e.from), b: byId.get(e.to) }));
    edges.forEach((e) => { e.a.deg++; e.b.deg++; });
    if (center && byId.has(center)) { const c = byId.get(center); c.x = 0; c.y = 0; c.center = true; }
    view = { x: r.width / 2, y: r.height / 2, k: 1 };
    size();
    heat = 300;
    loop();
  }

  function tick() {
    const rep = 2600;
    for (let i = 0; i < nodes.length; i++) {
      const a = nodes[i];
      for (let j = i + 1; j < nodes.length; j++) {
        const b = nodes[j];
        let dx = a.x - b.x, dy = a.y - b.y, d2 = dx * dx + dy * dy + .01;
        if (d2 > 250000) continue;
        const f = rep / d2, d = Math.sqrt(d2);
        dx /= d; dy /= d;
        a.vx += dx * f; a.vy += dy * f; b.vx -= dx * f; b.vy -= dy * f;
      }
      a.vx -= a.x * 0.004; a.vy -= a.y * 0.004;
    }
    for (const e of edges) {
      const len = e.origin === "tag" ? 60 : 110;
      const dx = e.b.x - e.a.x, dy = e.b.y - e.a.y, d = Math.sqrt(dx * dx + dy * dy) || 1;
      const f = (d - len) * 0.02;
      e.a.vx += dx / d * f; e.a.vy += dy / d * f; e.b.vx -= dx / d * f; e.b.vy -= dy / d * f;
    }
    for (const n of nodes) {
      if (n === drag?.node) continue;
      n.vx *= 0.82; n.vy *= 0.82;
      n.x += Math.max(-12, Math.min(12, n.vx)); n.y += Math.max(-12, Math.min(12, n.vy));
    }
  }

  function loop() {
    cancelAnimationFrame(raf);
    const step = () => {
      if (heat-- > 0) { tick(); draw(); raf = requestAnimationFrame(step); }
    };
    raf = requestAnimationFrame(step);
  }

  const radius = (n) => n.kind === "tag" ? 4 : 6 + Math.min(8, n.deg * 1.2);

  function draw() {
    if (!ctx) return;
    const r = canvas.getBoundingClientRect();
    ctx.clearRect(0, 0, r.width, r.height);
    ctx.save();
    ctx.translate(view.x, view.y); ctx.scale(view.k, view.k);
    const line = css("--line-strong"), accent = css("--accent-text");
    for (const e of edges) {
      ctx.beginPath();
      ctx.strokeStyle = e.origin === "link" ? accent : line;
      ctx.globalAlpha = e.origin === "tag" ? 0.35 : 0.8;
      ctx.lineWidth = (e.origin === "link" ? 1.6 : 1) / view.k;
      ctx.setLineDash(e.origin === "mention" ? [4 / view.k, 3 / view.k] : []);
      ctx.moveTo(e.a.x, e.a.y); ctx.lineTo(e.b.x, e.b.y); ctx.stroke();
    }
    ctx.setLineDash([]); ctx.globalAlpha = 1;
    ctx.font = `${12 / view.k}px ${css("--sans")}`;
    ctx.textAlign = "center";
    for (const n of nodes) {
      const rr = radius(n);
      ctx.beginPath(); ctx.arc(n.x, n.y, rr, 0, Math.PI * 2);
      if (n.kind === "tag") { ctx.strokeStyle = colour(n); ctx.lineWidth = 1.5 / view.k; ctx.stroke(); }
      else { ctx.fillStyle = colour(n); ctx.fill(); }
      if (n.center || n === hover) { ctx.strokeStyle = css("--text"); ctx.lineWidth = 2 / view.k; ctx.stroke(); }
      ctx.fillStyle = n.kind === "tag" ? css("--faint") : css("--text");
      if (view.k > 0.45 || n === hover) ctx.fillText(n.title, n.x, n.y + rr + 13 / view.k);
    }
    ctx.restore();
  }

  const toWorld = (e) => ({ x: (e.offsetX - view.x) / view.k, y: (e.offsetY - view.y) / view.k });
  const hit = (p) => nodes.find((n) => (n.x - p.x) ** 2 + (n.y - p.y) ** 2 <= (radius(n) + 4) ** 2);

  function down(e) {
    canvas.setPointerCapture(e.pointerId);
    const p = toWorld(e), n = hit(p);
    drag = { node: n, sx: e.offsetX, sy: e.offsetY, vx: view.x, vy: view.y, moved: false };
  }
  function move(e) {
    if (!drag) {
      const h = hit(toWorld(e)) ?? null;
      if (h !== hover) { hover = h; canvas.style.cursor = h ? "pointer" : "grab"; draw(); }
      return;
    }
    if (Math.abs(e.offsetX - drag.sx) + Math.abs(e.offsetY - drag.sy) > 3) drag.moved = true;
    if (drag.node) {
      const p = toWorld(e); drag.node.x = p.x; drag.node.y = p.y; heat = Math.max(heat, 30); loop();
    } else {
      view.x = drag.vx + e.offsetX - drag.sx; view.y = drag.vy + e.offsetY - drag.sy; draw();
    }
  }
  function up() {
    if (drag && !drag.moved && drag.node && drag.node.kind !== "tag") onOpen(drag.node.id);
    drag = null;
  }

  return { init, set, draw };
})();
