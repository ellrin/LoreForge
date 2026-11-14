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
| `export` | `render_spec(tools, spec, styles)`：沒有開頁面時用 headless Chrome 跑同一份 `plot.js` 畫出 SVG（PaperComposer 寫書用）；Chrome 列印成一頁向量 PDF；`pdftocairo -svg`（字變路徑）／`pdftoppm -r`；沒有 poppler 時用編輯器 SVG＋內嵌 KaTeX 字型、Chrome 截圖；PNG `iTXt`／SVG `<metadata>` 帶規格 |
| `figure` | `save_paper_figure`（`<stem>.pdf`＋`<stem>.plot.json`，作者自己的 PDF 永不覆蓋）、`spec_next_to`、`slide_png`、`spec_in_png`、`figure_block` |
| `propose` | LLM 任務 `plot-propose`（`tasks.toml`，類別 `create`）：資料＋一句話 → 規格；清單封閉、不決定顏色大小；作者的範例在 `private/shared/plot/examples/*.json` |
| `assist` | 圖表助手 `plot-assist`／`plot-assist-vision`（類別 `tidy`）：幾種畫法（2–4 個規格）或照一句話改（JSON pointer 修改，不碰資料，套用後檢查）；提示詞 `prompts/assist.md`，作者偏好 `private/shared/plot/assist.md` |
| `equation` | 會動的方程式（`kind: "equation"`，`../../../docs/plot.md` §10）：`expr` 安全算式語言（自己的剖析器，限制長度／巢狀／步數；`ui/plot/eqcore.js` 同一套，`ui/plot/eqtests.json` 兩邊都測）、模板（`ui/plot/equations.json`，也是 lore-notation 的一層：`template_notation`、`color_roles`）、`problems`／`check_properties`／`report`、`sensitivity`、`sweep_frames`、`spec_for`；`propose`（任務 `equation-plot`，`fit_for_auto` ＝能不能沒人看就放進投影片）；`gif`（全域調色盤編碼、`encode_with`：逐格延遲、只存變了的矩形與透明索引、熱圖矩形內 Bayer 抖色；`info`、`spec_from_gif`、`decode_png`）；`raster`（Chrome＋poppler 一次柵格化所有格，`gif_from_svgs_with`）；`sim`（模擬場景 `eq.sim`，§11：`problems`、`values`（共用案例 `ui/plot/simtests.json`）、`schedule`（GIF 節奏）、`suggest`（LaTeX → 場景，不用模型）、`scene`） |
| `router(Plot)` | `/plot/`、`/plot/{plot.js,charts.js,charts3d.js,formats.js,editor.js,editor.css,formats.json,palettes.json}`、`/plot/katex/*`、`/plot/api/{styles,import,export,propose,assist,gif}`、`/plot/api/equation/{check,propose,roles,scenes}`；`gif_from_body`（`GifBody`：SVG 格、`delays`、`dither`）、`export::render_equation_gif_frames` |

app 接上：

```rust
let plot = lore_plot::Plot::new(home.clone()).with_llm(Arc::new(MyLlm));   // MyLlm: lore_plot::propose::Llm
router = router.merge(lore_plot::router(plot));
tasks = tasks.merge(lore_plot::assist::task_table());   // plot-propose, plot-assist, plot-assist-vision
```

瀏覽器：`eqcore.js`（語言，DOM 以外）、`equation.js`（`equation` 圖、GIF 的每一格、編輯器的方程式區塊、變數顏色角色）、`eqlive.js`（app 裡的互動畫面：跟主題、曲線變形、3D 曲面與滾珠）、`simcore.js`（模擬的數值核心，DOM 以外，node 測試）、`eqsim.js`（模擬的即時畫面與檔案畫面、`LorePlot.animateLatex`、放映用的 `mount`／`mountCurve`）。
`cargo test -p lore-plot` 也用 node 跑 `ui/plot/tests/eqcore.test.js`、`eqcolors.test.js`（沒有 node 就略過）。

**沒有在離線測試裡的**：Chrome 輸出 PDF／PNG、poppler 轉 SVG／PNG（需要外部程式；`examples/export.rs` 手動檢查）。
Chrome 把可變字型（例如 STIX Two Text）內嵌成 Type 3（仍是向量）；靜態 TrueType 字型內嵌成 TrueType。
