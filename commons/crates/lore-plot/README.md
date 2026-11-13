# lore-plot — 繪圖工具

論文與簡報的結果圖：圖的規格（JSON）、資料匯入、樣式預設、輸出、自動產生，以及每個 app 掛上的 `/plot` 路由。
設計：`../../../docs/plot.md`。畫圖與編輯器在 `commons/ui/plot/`（`plot.js`、`editor.js`），本 crate 把它們編譯進程式。

```sh
cargo test -p lore-plot                                   # 離線：合成資料、暫存資料夾、假模型
cd commons/ui && python3 -m http.server                   # /plot/demo.html（編輯器）、/plot/gallery.html?style=nature（樣式一覽）
cargo run -p lore-plot --example export -- fig.svg fig.pdf   # 需要 Chrome；有 poppler 時 SVG／PNG 從 PDF 轉
cargo run -p lore-plot --example serve -- 7990            # /plot/ 編輯器＋假的模型（看助手的介面，不連任何引擎）
```

| 模組 | 內容 |
|---|---|
| `spec` | `Spec`（`lore_plot: 1`）、`Panel`、`Series`、`Table`；不認得的欄位保留；`problems()` 列出缺的資料欄、不認得的種類、大小 |
| `data` | `import(name, bytes)`：CSV、TSV、貼上的儲存格、JSON、XLSX（每個工作表）、LaTeX `tabular`（lore-docsrc）；`0.81 ± 0.02` 拆成 `欄` 與 `欄_sd` |
| `style` | 公開預設 `commons/ui/plot/styles/*.json`；私人 `private/shared/plot/styles/*.{json,toml}`（同 id 取代）與 `settings.toml`（`default_style`、`slide_style`、`[font] serif/sans` 排在字型清單最前面）；`base` 繼承 |
| `export` | Chrome 列印成一頁向量 PDF；`pdftocairo -svg`（字變路徑）／`pdftoppm -r`；沒有 poppler 時用編輯器 SVG＋內嵌 KaTeX 字型、Chrome 截圖；PNG `iTXt`／SVG `<metadata>` 帶規格 |
| `figure` | `save_paper_figure`（`<stem>.pdf`＋`<stem>.plot.json`，作者自己的 PDF 永不覆蓋）、`spec_next_to`、`slide_png`、`spec_in_png`、`figure_block` |
| `propose` | LLM 任務 `plot-propose`（`tasks.toml`，類別 `create`）：資料＋一句話 → 規格；清單封閉、不決定顏色大小；作者的範例在 `private/shared/plot/examples/*.json` |
| `assist` | 圖表助手 `plot-assist`／`plot-assist-vision`（類別 `tidy`）：幾種畫法（2–4 個規格）或照一句話改（JSON pointer 修改，不碰資料，套用後檢查）；提示詞 `prompts/assist.md`，作者偏好 `private/shared/plot/assist.md` |
| `router(Plot)` | `/plot/`、`/plot/{plot.js,charts.js,charts3d.js,formats.js,editor.js,editor.css,formats.json,palettes.json}`、`/plot/katex/*`、`/plot/api/{styles,import,export,propose,assist}` |

app 接上：

```rust
let plot = lore_plot::Plot::new(home.clone()).with_llm(Arc::new(MyLlm));   // MyLlm: lore_plot::propose::Llm
router = router.merge(lore_plot::router(plot));
tasks = tasks.merge(lore_plot::assist::task_table());   // plot-propose, plot-assist, plot-assist-vision
```

**沒有在離線測試裡的**：Chrome 輸出 PDF／PNG、poppler 轉 SVG／PNG（需要外部程式；`examples/export.rs` 手動檢查）。
Chrome 把可變字型（例如 STIX Two Text）內嵌成 Type 3（仍是向量）；靜態 TrueType 字型內嵌成 TrueType。
