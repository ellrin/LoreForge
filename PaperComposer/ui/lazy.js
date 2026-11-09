// Drawing formulas and figures only near the view. The page and the
// navigator put their text up at once; tex.js leaves each formula and image
// as a placeholder of the size it will take, and this file draws them when
// they come within about one screen above or below what is shown (an
// IntersectionObserver with a 100% margin), so a book opens and edits as
// quickly as a short paper and scrolling never meets an undrawn formula.
//
// A placeholder rarely has exactly the size of what replaces it. On the page,
// a change above the view would move what is being read, so the view is held
// on its first visible block while placeholders are drawn (browsers' own
// scroll anchoring is turned off for the page in styles.css; WebKit has none).
// Jumps (navigator, findings, errors) draw around the target first and then
// scroll; while a smooth scroll runs nothing else is drawn, so the target
// does not move under it. Printing draws everything first.

const PCLazy = (() => {
  const views = new Map(); // container -> { scroller, blocks, anchored, io }
  let paused = false;
  let jumps = 0;

  /** Draw the pending blocks of `container` as they near the view of `scroller`.
   *  `blocks(container)` lists the elements watched; `anchored` holds the view. */
  function attach(container, scroller, blocks, anchored) {
    const v = { container, scroller, blocks, anchored, io: null };
    views.set(container, v);
    return v;
  }

  function observer(v) {
    v.io?.disconnect();
    v.io = new IntersectionObserver((entries) => {
      if (paused) return;
      const hits = entries.filter((e) => e.isIntersecting).map((e) => e.target);
      if (!hits.length) return;
      hits.forEach((t) => v.io.unobserve(t));
      draw(v, hits);
    }, { root: v.scroller, rootMargin: "100% 0px 100% 0px" });
    return v.io;
  }

  /** Start watching (again) every block of the container that has placeholders. */
  function watch(container, only) {
    const v = views.get(container);
    if (!v) return;
    const io = only && v.io ? v.io : observer(v);
    // the page watches every block (each is marked .seen once drawn); the navigator only those with formulas
    for (const b of only ?? v.blocks(container)) if (v.anchored ? !b.classList.contains("seen") : PCTex.hasPending(b)) io.observe(b);
  }

  // A block drawn once is .seen: styles.css lets the browser skip laying it
  // out while it is far from the view (content-visibility: auto), keeping the
  // size it last had, so a long book scrolls as lightly as a short paper and
  // nothing moves when it comes back.
  // Its height now is the size kept until the browser has drawn it with that
  // rule (else a block skipped at once would take a default height).
  function seen(blocks) {
    const page = blocks.filter((b) => b.parentElement?.id === "paper" && !b.classList.contains("seen"));
    if (!page.length) return;
    // in the page's own CSS pixels, to the fraction (the page may be zoomed)
    const paper = page[0].parentElement, z = paper.getBoundingClientRect().width / (paper.offsetWidth || 1) || 1;
    // the content box: contain-intrinsic-size does not count padding and border
    const box = (b) => { const c = getComputedStyle(b); return parseFloat(c.paddingTop) + parseFloat(c.paddingBottom) + parseFloat(c.borderTopWidth) + parseFloat(c.borderBottomWidth); };
    const hs = page.map((b) => Math.max(0, b.getBoundingClientRect().height / z - box(b))); // all reads, then all writes
    page.forEach((b, i) => { b.style.containIntrinsicBlockSize = `auto ${hs[i]}px`; b.classList.add("seen"); });
  }

  function draw(v, targets) {
    if (v.anchored) hold(v, () => { targets.forEach((t) => PCTex.hydrate(t)); seen(targets); });
    else targets.forEach((t) => PCTex.hydrate(t));
  }

  /** The first block of the container whose bottom is below the top of the view. */
  function firstVisible(v) {
    const kids = v.container.children;
    const top = v.scroller.getBoundingClientRect().top;
    let lo = 0, hi = kids.length - 1, found = null;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (kids[mid].getBoundingClientRect().bottom > top) { found = kids[mid]; hi = mid - 1; } else lo = mid + 1;
    }
    return found;
  }

  /** Run fn without moving what is shown at the top of the view. */
  function hold(v, fn) {
    const a = firstVisible(v);
    const before = a?.getBoundingClientRect().top;
    fn();
    if (!a?.isConnected) return;
    const d = a.getBoundingClientRect().top - before;
    if (Math.abs(d) >= 0.5) v.scroller.scrollTop += d;
  }

  /** The container's child holding el. */
  function blockOf(v, el) {
    let b = el;
    while (b && b.parentElement !== v.container) b = b.parentElement;
    return b;
  }

  /** Draw el's block and the blocks within a screen above and below it. */
  function reveal(el) {
    const v = [...views.values()].find((x) => x.container.contains(el));
    if (!v) return;
    const b = blockOf(v, el);
    if (!b) return;
    const screen = v.scroller.clientHeight || innerHeight;
    const drawn = [b];
    PCTex.hydrate(b);
    for (const dir of ["previousElementSibling", "nextElementSibling"]) {
      let n = b[dir], h = 0;
      while (n && h < screen) { PCTex.hydrate(n); drawn.push(n); h += n.offsetHeight; n = n[dir]; }
    }
    seen(drawn);
  }

  /** Scroll to el (scrollIntoView options) with it and its surroundings drawn. */
  function jump(el, opts = { block: "center", behavior: "smooth" }) {
    if (!el) return;
    reveal(el);
    const v = [...views.values()].find((x) => x.container.contains(el));
    if (!v || opts.behavior !== "smooth") { el.scrollIntoView(opts); return; }
    // nothing is drawn while the smooth scroll runs; afterwards, watch again
    paused = true;
    const sc = v.scroller;
    const id = ++jumps;
    let timer = null;
    const done = () => {
      sc.removeEventListener("scroll", wait);
      clearTimeout(timer);
      if (id !== jumps) return; // a later jump took over
      jumps++;
      paused = false;
      views.forEach((x) => watch(x.container));
    };
    const wait = () => { clearTimeout(timer); timer = setTimeout(done, 160); };
    sc.addEventListener("scroll", wait, { passive: true });
    wait();
    setTimeout(done, 2500); // in any case
    el.scrollIntoView(opts);
  }

  /** Draw everything under container now (printing, selecting all). */
  function drawAll(container) {
    PCTex.hydrate(container);
    views.get(container)?.io?.disconnect();
  }

  addEventListener("beforeprint", () => {
    views.forEach((v) => drawAll(v.container));
    const paper = document.getElementById("paper");
    if (paper) PCTex.fullImages(paper);
  });

  /** Draw what is in view (and a screen around it) now, before the next paint. */
  function revealView(container) {
    const v = views.get(container);
    const a = v && firstVisible(v);
    if (a) hold(v, () => reveal(a));
  }

  return { attach, watch, reveal, revealView, jump, drawAll, hold: (container, fn) => { const v = views.get(container); if (v) hold(v, fn); else fn(); } };
})();
