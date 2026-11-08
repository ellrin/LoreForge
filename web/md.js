// Small Markdown + LaTeX renderer for card bodies (no build step, no network).
// Supports headings, paragraphs, lists, block quotes, fenced code, **bold**,
// *italic*, `code`, [links](url), ![images](url), $inline$ / $$display$$ math
// (KaTeX) and [[wikilinks]] (rendered as a.wikilink, the app sets the href; `resolve(name)` says
// whether a card exists). Everything else is escaped.

const MD = (() => {
  const esc = (s) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const safeUrl = (u) => (/^(https?:\/\/|\/api\/assets\/|#)/.test(u) ? u : "#");

  function tex(src, display) {
    if (!window.katex) return `<code>${esc(src)}</code>`;
    try {
      return katex.renderToString(src, { displayMode: display, throwOnError: false, strict: "ignore", output: "html" });
    } catch {
      return `<code>${esc(src)}</code>`;
    }
  }

  function inline(text, opts, slots) {
    // Protect code and math first so their contents are not touched.
    const keep = (html) => `\u0000${slots.push(html) - 1}\u0000`;
    let s = text
      .replace(/`([^`]+)`/g, (_, c) => keep(`<code>${esc(c)}</code>`))
      .replace(/\$\$([^$]+)\$\$/g, (_, m) => keep(tex(m, true)))
      // $…$ needs a non-space right inside both dollars and no digit right after the closing
      // one, so prices like "$5 and $10" stay text.
      .replace(/(^|[^\\$])\$(?=\S)([^$\n]*?\S)\$(?!\d)/g, (_, pre, m) => pre + keep(tex(m, false)));
    s = esc(s);
    s = s
      .replace(/\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (_, target, shown) => {
        const t = target.trim();
        const ok = opts.resolve ? opts.resolve(t) : true;
        return `<a class="wikilink${ok ? "" : " missing"}" href="#" data-target="${t}">${(shown ?? target).trim()}</a>`;
      })
      .replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (_, alt, url) => `<img alt="${alt}" src="${safeUrl(url.replace(/&amp;/g, "&"))}">`)
      .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_, t, url) => `<a href="${safeUrl(url.replace(/&amp;/g, "&"))}" target="_blank" rel="noopener">${t}</a>`)
      .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
      .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>");
    return s;
  }

  function render(src, opts = {}) {
    const slots = [];
    const lines = (src ?? "").replace(/\r\n?/g, "\n").split("\n");
    const out = [];
    let para = [];
    let list = null;
    const flushPara = () => {
      if (para.length) out.push(`<p>${inline(para.join("\n"), opts, slots).replace(/\n/g, "<br>")}</p>`);
      para = [];
    };
    const flushList = () => {
      if (list) out.push(`<${list.tag}>${list.items.map((i) => `<li>${inline(i, opts, slots)}</li>`).join("")}</${list.tag}>`);
      list = null;
    };
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      if (/^```/.test(line.trim())) {
        flushPara(); flushList();
        const code = [];
        while (++i < lines.length && !/^```/.test(lines[i].trim())) code.push(lines[i]);
        out.push(`<pre><code>${esc(code.join("\n"))}</code></pre>`);
        continue;
      }
      if (line.trim() === "$$") {
        flushPara(); flushList();
        const m = [];
        while (++i < lines.length && lines[i].trim() !== "$$") m.push(lines[i]);
        out.push(tex(m.join("\n"), true));
        continue;
      }
      let m;
      if ((m = /^(#{1,4})\s+(.*)$/.exec(line))) {
        flushPara(); flushList();
        const n = Math.min(m[1].length + 1, 4);
        out.push(`<h${n}>${inline(m[2], opts, slots)}</h${n}>`);
      } else if ((m = /^\s*([-*]|\d+[.)])\s+(.*)$/.exec(line))) {
        flushPara();
        const tag = /\d/.test(m[1]) ? "ol" : "ul";
        if (list && list.tag !== tag) flushList();
        list ??= { tag, items: [] };
        list.items.push(m[2]);
      } else if ((m = /^>\s?(.*)$/.exec(line))) {
        flushPara(); flushList();
        out.push(`<blockquote>${inline(m[1], opts, slots)}</blockquote>`);
      } else if (!line.trim()) {
        flushPara(); flushList();
      } else {
        flushList();
        para.push(line);
      }
    }
    flushPara(); flushList();
    return out.join("\n").replace(/\u0000(\d+)\u0000/g, (_, n) => slots[+n]);
  }

  return { render, esc };
})();
