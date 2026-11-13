# 繪圖工具（lore-plot）

作者 2026-11-13 的要求：論文與簡報的結果圖（長條、折線、散佈、ROC…）用 GUI 調整到好看，也能從資料＋一句話自動產生，再手動微調。
不是 Dock 上的 app，是共用套件：PaperComposer 與 SlideComposer 從文件裡打開、插入、雙擊重新編輯。

- 程式：`commons/crates/lore-plot`（圖的規格、資料匯入、樣式、輸出、LLM 任務、axum 路由）
  與 `commons/ui/plot/`（`plot.js` 畫圖、`editor.js` 編輯器、`demo.html`）。
- 圖的規格（spec）是一份 JSON；畫圖只有一個地方：瀏覽器裡的 `plot.js`（spec → SVG）。
  編輯器的畫面、輸出的 PDF／SVG／PNG 都是同一份 SVG，所以看到的就是輸出的。

## 1. 作者的圖（調查摘要）

作者的結果圖（私人調查在 `private/shared/plot/survey.md`）：一排三格（三個 backbone）的分組長條圖與虛線＋圓點折線、
2×3 的 ROC／PR（圖例寫 AUC，右上角放大小圖）、混淆矩陣加邊欄指標。淡色長條、同色系深色折線、y 方向虛線格線、
圖例有框。原本是 matplotlib PNG（330／600 dpi），要改成向量、字型一致（襯線體）、數學式。

## 2. 期刊的規定（樣式預設的根據）

| 樣式 | 欄寬（單欄／1.5／雙欄 mm） | 字 | 其他 |
|---|---|---|---|
| `nature` | 89 / 120 / 183，高 ≤ 170 | 無襯線，5–7 pt；格標 8 pt 粗體小寫 a b c | 有軸線與刻度，不要背景格線，色盲友善（Wong 2011），文字不上色，PDF 內嵌字型 |
| `npj` | 同 Nature Portfolio（npj Digital Medicine 的規定頁要登入，沿用 Nature 的數字） | 同上 | 同上 |
| `ieee`（TPAMI、TMI） | 88.9 / — / 181.9（3.5 in、7.16 in） | 襯線，8 pt 一致 | 線寬 ≥ 0.5 pt，600 dpi 線稿 |
| `cvpr` | 82.6 / — / 174.6（3.25 in、6.875 in） | 襯線，和內文同大（內文 10 pt、圖說 9 pt），圖裡 8–9 pt | 不能假設讀者會放大 |
| `clean` | 預設：單欄 85 mm | 襯線 8 pt | 中性的預設 |
| `slide` | 簡報：寬 240 mm | 無襯線 16 pt | 線粗、點大 |

色盲友善色盤：Okabe–Ito（Wong 2011）、Paul Tol bright／muted；熱圖：viridis、cividis、magma（感知均勻）。

## 3. 規格（`<名稱>.plot.json`）

```json
{
  "lore_plot": 1,
  "style": "clean",
  "set": { "font.size": 8, "axes.grid": "y" },
  "size": { "w": 85, "h": 60 },
  "grid": { "cols": 1, "rows": 1, "gap": 6, "labels": "a" },
  "data": { "t1": { "name": "results.csv", "columns": ["threshold", "base", "base_sd"], "rows": [[0.1, 0.81, 0.02]] } },
  "panels": [
    { "type": "bar", "data": "t1", "x": "threshold",
      "series": [{ "y": "base", "err": "base_sd", "label": "Baseline" }],
      "title": "", "xaxis": { "label": "IoU threshold" }, "yaxis": { "label": "$\\mathrm{Dice}$", "min": 0 },
      "legend": { "pos": "upper right" }, "opts": { "mode": "grouped" },
      "notes": [{ "type": "bracket", "x": 0, "x2": 2, "y": 0.95, "text": "**" }],
      "inset": null }
  ],
  "caption": ""
}
```

- 單位：`size` 是 mm；字是 pt；線寬是 pt。SVG 的 `viewBox` 以 pt 為單位、`width`/`height` 寫 mm。
- `style` ＝樣式預設的 id；`set` ＝這張圖自己改的值（點分隔的鍵，見 §4），GUI 的「樣式」區塊只寫 `set`，不改預設檔。
- `data` 跟著圖存（資料本身，不是檔案路徑），所以搬到別台電腦也能重新打開。`name` 只記來源檔名。
- `panels` 依 `grid` 由左到右、由上到下排；`span: 2` 佔兩格。格標（a、b、c）依 `grid.labels`：`a` `A` `(a)` `none`。
- 版本：`lore_plot` 是規格版本；讀到舊版由 `lore_plot::Spec::migrate` 升級；不認得的欄位保留。

### 圖的種類（`type`）

| type | 欄位 | 選項 `opts` |
|---|---|---|
| `line` | `x`；`series[]`: `y`、`err`（誤差，畫帶或誤差線） | `err_style`: band／bars；`step` |
| `bar` | `x`（類別）；`series[]`: `y`、`err` | `mode`: grouped／stacked；`horizontal`；`values`（長條上標數值）；`fmt` |
| `scatter` | `x`、`y`、`group`（類別欄，t-SNE／UMAP 群集）、`size` | `ellipse`（95% 共變異橢圓）、`hull`（凸包）、`alpha` |
| `box`／`violin` | `series[]`: `y`（每欄一組分布）；或 `group`＋`y`（長格式） | `points`（散點疊上）、`notch`、`mean` |
| `hist` | `series[]`: `y` | `bins`、`density`、`stacked` |
| `heatmap` | 第一欄是列名，其他欄是值（混淆矩陣同） | `cmap`、`values`、`fmt`、`normalize`: rows／none、`colorbar` |
| `roc`／`pr` | `series[]`: `x`、`y`（已算好的曲線）或 `score`＋`truth`（自己算） | `auc`（圖例加 AUC = 0.943）、`diagonal` |
| `bland` | `series[0]`: `y`（方法 A）、`y2`（方法 B） | `limits`（平均差 ± 1.96 SD） |
| `km` | `x`（時間）、`event`（1 = 事件，0 = 設限）、`group` | `censor`（設限記號） |
| `radar` | `x`（各軸名稱）、`series[]`: `y` | `fill` |

每條 `series` 可以有：`label`（可含 `$…$`）、`color`（不給＝色盤依序）、`marker`（o s ^ v D x + none）、
`dash`（solid dash dot dashdot）、`width`、`alpha`、`hidden`。

### 軸、圖例、註記

- `xaxis`／`yaxis`：`label`、`unit`（加在標籤後的括號裡）、`min`、`max`、`log`、`ticks`（數字陣列）、`fmt`（`.2f`、`%`、`.0%`）、
  `grid`（覆寫樣式）、`rotate`（刻度字旋轉）、`reverse`、`hidden`。
- `legend`：`show`、`pos`（best、upper right…、outside right、below）、`cols`、`frame`、`title`。
- `notes[]`：`text`（`x`,`y`，`coords`: data／axes）、`arrow`（`x`,`y` → `x2`,`y2`，可加 `text`）、
  `bracket`（顯著性括號：類別 `x` 到 `x2`，高 `y`，`text`: `*`、`**`、`n.s.`、`$p<0.01$`）、`hline`／`vline`（`y`／`x`，`label`、`dash`）。
- `inset`：放大小圖 `{ "x": [0, 0.1], "y": [0.6, 1], "at": [0.45, 0.35, 0.5, 0.5] }`（`at` ＝在軸內的位置與大小，軸的比例）。

## 4. 樣式預設（資料，不是程式）

公開預設：`commons/crates/lore-plot/styles/*.toml`；作者自己的：`private/shared/plot/styles/*.toml`（lore-home 的 shared 資料夾）。
同 id 時私人的優先。私人設定 `private/shared/plot/settings.toml`：

```toml
default_style = "mine"        # 新圖用的樣式
[font]
serif = ["<作者的字型>"]      # 排在每個襯線樣式的字型清單最前面
sans = []
```

樣式檔的鍵（`set` 用同樣的點分隔名稱，例如 `font.size`）：

```toml
id = "clean"
label = "簡潔"
[size]  w = 85, h = 60, widths = [85, 120, 175]
[font]  kind = "serif", family = ["STIX Two Text", "TeX Gyre Termes", "Liberation Serif", "Tinos", "serif"],
        size = 8, title = 8, label = 8, tick = 7, legend = 7, panel = 9, panel_weight = "bold", math = "text"
[axes]  line = 0.6, tick_len = 3, tick_dir = "out", spines = "lb", grid = "none", grid_dash = "dot", grid_color = "#c8c8c8", grid_width = 0.4
[lines] width = 1.2, marker = 3.5, markers = ["o", "s", "^", "D", "v"], dashes = ["solid"]
[bars]  width = 0.8, gap = 0.04, edge = 0, cap = 3, alpha = 1
[legend] frame = false, pos = "best", size = 7
[colors] palette = [...], lines = [...], cmap = "viridis", text = "#000000", axis = "#000000", background = "#ffffff"
```

- `colors.lines`（可省略）：折線用的深色，和長條的淡色 `palette` 一一對應（作者的習慣）。
- 字型：公開預設只寫自由授權的字（STIX Two Text：macOS 內建、OFL；TeX Gyre Termes：GUST 授權）。作者指定的字型只在私人設定。
  圖的顏色、字從不跟 app 的主題；編輯器的外框跟主題，圖永遠白底。

## 5. 數學式（LaTeX）

- 標題、軸標籤、圖例、註記都可以寫 `$…$`；用 commons 已 vendor 的 KaTeX（`commons/ui/notation-panel/vendor/katex`，0.16.47）。
- KaTeX 輸出是 HTML，不是 SVG。`plot.js` 把排好的 KaTeX 量好位置後**攤平成 SVG**：每段字是一個 `<text>`（字型 KaTeX_Main、KaTeX_Math…），
  分數線是 `<rect>`，根號等延伸符號是 KaTeX 自己的 `<svg>` 路徑。所以 SVG 裡沒有 `foreignObject`，PDF 裡數學式是向量字。
- `font.math = "text"`：數學式裡的字母、數字改用圖的字型（斜體），符號（∑、∫、括號）仍是 KaTeX 的字；`katex`：全部用 KaTeX 的字（Computer Modern 風格）。
- 從 PaperComposer 打開時，論文對照表（lore-notation）的巨集交給 KaTeX，所以 `$\Lrec$` 這種登記過的巨集畫得出來。

## 6. 輸出

| 格式 | 怎麼做 | 用在 |
|---|---|---|
| PDF | headless Chrome 列印一頁（`@page` ＝圖的 mm 大小，邊界 0），字型內嵌、全部向量 | 論文 |
| SVG | 有 `pdftocairo` 就從 PDF 轉（字變路徑，任何軟體都一樣）；沒有就用編輯器的 SVG＋內嵌用到的 KaTeX 字型 | 論文、網頁 |
| PNG | 有 `pdftoppm` 就從 PDF 轉（`-r <dpi>`，和 PDF 一模一樣）；沒有就用 Chrome 截圖 | 簡報 |

- 規格永遠跟著圖：PaperComposer 是 `figures/<名稱>.plot.json`（和 PDF 同名）；PNG 把規格寫進自己的 `iTXt` 區塊（`lore-plot`），
  SVG 寫進 `<metadata>`。所以 SlideComposer 的圖片就算經過 .pptx 存檔再打開，雙擊仍能重新編輯。
- 沒有 Chrome：輸出失敗並說明（「需要 Chrome 才能輸出 PDF」），編輯與儲存規格不受影響。**輸出不在離線測試內**（需要瀏覽器）。

## 7. 自動產生（LLM 任務 `plot-propose`）

`commons/crates/lore-plot/tasks.toml`，類別 `create`，app 合併進自己的任務表（像 lore-flow）。
輸入：資料的欄名與前幾列、作者的一句話（「比較我們的方法與 4 個 baseline 的 Dice 和 HD95，分組長條加誤差線」）、可用的樣式 id。
輸出（JSON schema，清單都是封閉的）：圖的種類、用哪些欄、系列名稱、軸標籤（可含 `$…$`）、格數、樣式 id、圖說草稿。
模型不給顏色、大小、座標；這些來自樣式。回答再經 `Spec::validate`（欄名必須存在）才交給編輯器。

## 8. 接到 app

### 共用部分

app 伺服器：`router = router.merge(lore_plot::router(lore_plot::Plot::new(home).with_llm(…)))`，提供：

| 路由 | 內容 |
|---|---|
| `GET /plot/`、`/plot/plot.js`、`/plot/editor.js`、`/plot/editor.css`、`/plot/katex/*` | 編輯器頁面與元件（編譯進程式，像 lore-shell） |
| `GET /plot/api/styles` | 樣式（公開＋私人，已套用私人字型）與預設樣式 id |
| `POST /plot/api/import?name=<檔名>` | 檔案內容（CSV、TSV、JSON、XLSX、含 tabular 的 .tex、貼上的文字）→ `{ tables: [{ name, columns, rows }] }` |
| `POST /plot/api/export` | `{ svg, spec, format: pdf/svg/png, dpi }` → 檔案 |
| `POST /plot/api/propose` | `{ tables, sentence, style }` → `{ spec, engine }` |

頁面：`/plot/plot.js` + `/plot/editor.js` + `/plot/editor.css`，然後
`LorePlot.edit({ spec, macros, title, target: "paper" | "slide", save: async ({ spec, svg }) => … })` 打開全螢幕編輯器（`<dialog>`），
按「完成」呼叫 `save`；`save` 由 app 寫檔。

### PaperComposer

- 論文右鍵選單「繪圖…」（在「新增圖…」旁）：編輯器 → 完成 → `POST /api/papers/{name}/plot`（`{ stem, spec, svg }`）
  → 伺服器輸出 `figures/<stem>.pdf`＋`figures/<stem>.plot.json` → 和「新增圖…」同一條路插入
  `\begin{figure}[t] \centering \includegraphics[width=\linewidth]{figures/<stem>.pdf} \caption{…} \label{fig:…} \end{figure}`
  （標籤由 `newLabel` 取、不重複；游標處插 `\ref{fig:…}`，不打「Figure 1」；需要時 main.tex 加 graphicx）。
- 雙擊頁面上的圖：圖檔旁有 `.plot.json` 才打開編輯器（沒有就照舊編輯段落）；完成後覆寫同一個 PDF 與規格（有規格就代表是這個工具產生的檔），圖片重新載入。
- lore-notation：插入的區塊用登記的 `\label`／`\ref`，不動 `notation.tex`；`lore-plot` 的測試把產生的區塊放進合成論文跑 `check_document`。

### SlideComposer

- 上方「插入」選單「圖表」：編輯器（`target: "slide"`，預設樣式 `slide`）→ 完成 → `POST /api/decks/{name}/plot`（`{ spec, svg, dpi }`）
  → 伺服器輸出 PNG（預設 300 dpi，規格寫在 PNG 裡）存成 deck 的 asset → 插入圖片物件。
  PPTX 的圖片只放 PNG：lore-canvas 的 PPTX 輸出沒有 `svgBlip`＋PNG 備援，PowerPoint 不能直接顯示 SVG。
- 雙擊圖片：asset 是帶規格的 PNG 才打開編輯器；完成後同一張圖片換 asset（一個 `set_obj`，一步復原）。

## 9. 狀態與之後

見 `commons/crates/lore-plot/README.md`。
