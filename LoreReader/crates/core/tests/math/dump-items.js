// Regenerate tests/math/paperN.items.json: the pdf.js text items of a compiled synthetic
// paper, in the same shape the reader page posts to /api/papers/{sha}/read.
//
//   pdflatex paper1.tex
//   NODE_PATH=$(npm root -g) node dump-items.js paper1.pdf paper1.items.json
//
// Needs Playwright with Chromium. Serves LoreReader/web (for the vendored pdf.js) and the
// PDF on a temporary local port. Only for synthetic papers: never commit a real paper's text.
const http = require("http");
const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright");

const [pdfPath, outPath] = process.argv.slice(2);
const web = path.resolve(__dirname, "../../../../web");
const page = `<!doctype html><script type="module">
import * as pdfjs from "/vendor/pdfjs/pdf.min.mjs";
pdfjs.GlobalWorkerOptions.workerSrc = "/vendor/pdfjs/pdf.worker.min.mjs";
window.dump = async () => {
  const doc = await pdfjs.getDocument({ url: "/paper.pdf", isEvalSupported: false, standardFontDataUrl: "/vendor/pdfjs/standard_fonts/" }).promise;
  const pages = [];
  for (let n = 1; n <= doc.numPages; n++) {
    const pg = await doc.getPage(n);
    const tc = await pg.getTextContent();
    await pg.getOperatorList();
    let text = ""; const items = []; const fonts = {};
    for (const it of tc.items) {
      if (it.str === undefined) continue;
      const f = it.fontName;
      if (!(f in fonts)) { let o = null; try { o = pg.commonObjs.has(f) ? pg.commonObjs.get(f) : null; } catch {} fonts[f] = { name: o?.name ?? "", italic: !!o?.italic, bold: !!o?.bold }; }
      const s = text.length; text += it.str; const t = it.transform;
      items.push({ s, e: text.length, str: it.str, font: fonts[f].name, italic: fonts[f].italic, bold: fonts[f].bold, size: Math.hypot(t[2], t[3]) || Math.abs(t[0]), x: t[4], y: t[5], w: it.width, h: it.height, eol: !!it.hasEOL });
      if (it.hasEOL) text += "\\n";
    }
    const vp = pg.getViewport({ scale: 1 });
    pages.push({ page: n, text, items, width: vp.width, height: vp.height });
  }
  return { pages };
};
window.ready = true;
</script>`;

const types = { ".mjs": "text/javascript", ".js": "text/javascript", ".pdf": "application/pdf", ".html": "text/html" };
const server = http.createServer((req, res) => {
  const url = decodeURIComponent(req.url.split("?")[0]);
  if (url === "/") { res.writeHead(200, { "content-type": "text/html" }); return res.end(page); }
  const file = url === "/paper.pdf" ? path.resolve(pdfPath) : path.join(web, path.normalize(url));
  if (url !== "/paper.pdf" && !file.startsWith(web)) { res.writeHead(403); return res.end(); }
  fs.readFile(file, (err, data) => {
    if (err) { res.writeHead(404); return res.end(); }
    res.writeHead(200, { "content-type": types[path.extname(file)] ?? "application/octet-stream" });
    res.end(data);
  });
});
server.listen(0, "127.0.0.1", async () => {
  const b = await chromium.launch();
  const p = await b.newPage();
  await p.goto(`http://127.0.0.1:${server.address().port}/`);
  await p.waitForFunction(() => window.ready);
  fs.writeFileSync(outPath, JSON.stringify(await p.evaluate(() => window.dump())));
  await b.close();
  server.close();
});
