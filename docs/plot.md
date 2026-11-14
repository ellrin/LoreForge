# 繪圖工具（lore-plot）

作者 2026-11-13 的要求：論文與簡報的結果圖（長條、折線、散佈、ROC…）用 GUI 調整到好看，也能從資料＋一句話自動產生，再手動微調。
不是 Dock 上的 app，是共用套件：PaperComposer 與 SlideComposer 從文件裡打開、插入，選取圖時角落的「編輯」小按鈕重新編輯（雙擊不特別處理，2026-11-13 作者決定，和流程圖一樣）。

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
| `radar` | `x`（各軸名稱）、`series[]`: `y`（`ref` 參考線） | `fill`、`normalize` |
| `pie` | `x`（名稱）、`series[0].y` | `donut`、`labels`: direct／legend、`other`（併成 Other 的比例）、`total` |
| `area`／`dumbbell` | 同 line／bar（啞鈴：兩個系列） | `area`: fill／stack |
| `ridge`／`ecdf` | 同 box（每欄一組分布） | `overlap` |
| `calib`／`dca` | `series[]`: `score`＋`truth` | `bins`／`max` |
| `contour`／`surface`／`scatter3d` | `x`、`y`、`z`（規則格點；點雲可有 `group`） | `levels`、`azim`、`elev`、`walls` |

每條 `series` 可以有：`label`（可含 `$…$`）、`color`（不給＝色盤依序）、`marker`（o s ^ v D x + none）、
`dash`（solid dash dot dashdot）、`width`、`alpha`、`hidden`。

### 軸、圖例、註記

- `xaxis`／`yaxis`：`label`、`unit`（加在標籤後的括號裡）、`min`、`max`、`log`、`ticks`（數字陣列）、`fmt`（`.2f`、`%`、`.0%`）、
  `grid`（覆寫樣式）、`rotate`（刻度字旋轉）、`reverse`、`hidden`。
- `legend`：`show`、`pos`（best、upper right…、outside right、below）、`cols`、`frame`、`title`。
- `notes[]`：`text`（`x`,`y`，`coords`: data／axes）、`arrow`（`x`,`y` → `x2`,`y2`，可加 `text`）、
  `bracket`（顯著性括號：類別 `x` 到 `x2`，高 `y`，`text`: `*`、`**`、`n.s.`、`$p<0.01$`）、`hline`／`vline`（`y`／`x`，`label`、`dash`）。
- `inset`：放大小圖 `{ "x": [0, 0.1], "y": [0.6, 1], "at": [0.45, 0.35, 0.5, 0.5] }`（`at` ＝在軸內的位置與大小，軸的比例）。

## 3b. 格式（formats.json）

每種圖有 4–7 個格式（和色盤、樣式分開）：寬度、比例、圖例、附加元素。資料：`commons/ui/plot/formats.json`。
格式只是**建議**：依資料形狀排序（類別數、系列數、點數、AUC 差…），裡面的數字（≤4 類、8–20 類、AUC 差 0.02、圖例一列 ≤6 項…）都是可以改的預設值，
從不擋住任何格式——任何格式都能套在任何資料上，`plot-assist` 也可以在範圍外建議（說明理由）。
套用（`LorePlot.applyFormat`）只改這一格的外觀與圖的大小；資料對應（哪一欄是 x、哪些系列）保留，可以復原。

| 代號 | 圖 | 用途 | 寬 · 比例 | 說明 |
|---|---|---|---|---|
| B1 | bar | 少數類別、差異在 y | 單欄 · 1.33:1 | y 軸拉近時軸上有斷開記號，不會默默從非零開始 |
| B2 | bar | 很多類別、一個系列 | 雙欄 · 2.2:1 | 標籤太長時改橫向 |
| B3 | bar | 分組（類別 × 2–5 系列） | 雙欄 · 2.2:1 | 誤差線；圖例在上方一列；我們的方法最搶眼 |
| B4 | bar | 堆疊 | 單欄 · 0.83:1 | opts.mode = percent 改成百分比 |
| B5 | dumbbell | 成對（啞鈴） | 單欄 · 1:1 |  |
| B6 | bar | 每個指標一格（小倍數） | 雙欄 · 3.2:1 | 單位相同時共用 y 軸 |
| B7 | bar | 顯著性括號（* / ** / n.s.） | 單欄 · 1.33:1 | 疊在 B1／B3 上，高度自動錯開 |
| L1 | line | 訓練曲線 | 單欄 · 1.33:1 | 平滑線＋原始線 25% 透明；loss 圖例右上、accuracy 右下 |
| L2 | line | 參數掃描 | 單欄 · 1.33:1 | 一定有點；跨好幾個數量級時 x 用對數 |
| L3 | line | 很多系列（>6） | 雙欄 · 2.2:1 | 突出 1–2 條，其餘灰色 |
| L4 | line | 兩個指標共用 x | 單欄 · 0.71:1 |  |
| L5 | line | 平均 ± 標準差色帶 | 單欄 · 1.33:1 |  |
| L6 | line | 指標 vs 門檻＋操作點 | 單欄 · 1.33:1 | 操作點用註記移到實際位置 |
| S1 | scatter | 嵌入（t-SNE／UMAP） | 單欄 · 1:1 | 點的大小 ∝ 1/log n；5000 點以上透明 0.6 |
| S2 | scatter | 相關＋迴歸線＋r、p | 單欄 · 1.33:1 | 同單位時加 identity 線（opts.identity） |
| S3 | bland | Bland–Altman | 單欄 · 1.33:1 |  |
| S4 | scatter | 氣泡（大小欄） | 單欄 · 1.33:1 | 選大小欄（size） |
| S5 | scatter | 逐例 A vs B | 單欄 · 1:1 |  |
| S6 | scatter | 大量點（hexbin） | 單欄 · 1.33:1 |  |
| V1 | box | 2–5 組分布 | 單欄 · 1.33:1 | n 少於 100 時疊上抖動點 |
| V2 | box | 組 × 條件 | 雙欄 · 2.2:1 | 選分組欄（group）與條件欄（hue） |
| V3 | box | 很多組（橫向、依中位數排序） | 單欄 · 0.8:1 |  |
| V4 | box | 成對（每位受試者連線） | 單欄 · 1.33:1 |  |
| V5 | violin | 雨雲圖 | 單欄 · 1.33:1 |  |
| H1 | heatmap | 混淆矩陣 | 單欄 · 1:1 |  |
| H2 | heatmap | 相關矩陣（下三角） | 單欄 · 1:1 |  |
| H3 | heatmap | 結果表（方法 × 資料集） | 雙欄 · 2:1 | 每欄最好的加框（opts.best = col-min 時越小越好） |
| H4 | heatmap | 注意力圖 | 單欄 · 1:1 |  |
| H5 | heatmap | 兩個超參數的消融格 | 單欄 · 1:1 |  |
| R1 | roc | ROC（≤5 個方法） | 單欄 · 1:1 |  |
| R2 | roc | ROC＋左上放大 | 單欄 · 1:1 | AUC 差距小或曲線在左上重疊時 |
| R3 | pr | PR | 單欄 · 1:1 | 虛線＝盛行率 |
| R4 | roc | 每類／每資料集一格 | 雙欄 · 3:1 |  |
| R5 | calib | 校準＋數量直方 | 單欄 · 1:1 |  |
| R6 | dca | 決策曲線（全治療／不治療） | 單欄 · 1.33:1 |  |
| D1 | hist | 直方圖（Freedman–Diaconis） | 單欄 · 1.33:1 | opts.bins 可改組數 |
| D2 | hist | 2–3 組重疊＋KDE | 單欄 · 1.33:1 |  |
| D3 | ridge | 山脊圖 | 單欄 · 0.9:1 |  |
| D4 | ecdf | 累積分布（ECDF） | 單欄 · 1.33:1 |  |
| T1 | contour | 等高線（優先於曲面） | 單欄 · 1.1:1 | 規則格點的 x、y、z 欄 |
| T2 | surface | 曲面（固定視角） | 單欄 · 1:1 | 視角存在規格裡 |
| T3 | scatter3d | 小型 3D 點雲 | 單欄 · 1:1 |  |
| T4 | heatmap | 醫學影像三個正交切面 | 雙欄 · 3:1 | 三個表：axial、coronal、sagittal |
| P1 | pie | ≤5 片、直接標百分比 | 單欄 · 1:1 |  |
| P2 | pie | 甜甜圈（中間寫總數） | 單欄 · 1:1 |  |
| P3 | pie | 很多小片併成「Other」 | 單欄 · 1:1 |  |
| P4 | pie | 並排比較 | 雙欄 · 2.2:1 | 比較才是重點時，長條通常更清楚 |
| A1 | radar | 2–4 個方法 × 5–8 指標（正規化） | 單欄 · 1:1 |  |
| A2 | radar | 每個方法一格 | 雙欄 · 3:1 |  |
| A3 | radar | 單一方法 vs 平均／參考 | 單欄 · 1:1 |  |
| C1 | * | 2×2（a–d） | 雙欄 · 1:1 |  |
| C2 | * | 1×3 | 雙欄 · 3:1 |  |
| C3 | * | 左大＋右兩小 | 雙欄 · 1.6:1 |  |
| C4 | * | 共用色條 | 雙欄 · 3:1 |  |

期刊數字（`venues`，2026-11-13 查核；Nature 的最大高度現在是 170 mm，不是舊的 247 mm）：

| 期刊 | 單欄 / 雙欄 mm | 最大高 mm | 字 | 來源 |
|---|---|---|---|---|
| Nature | 89 / 183 | 170 | 5–7 pt | research-figure-guide.nature.com (building and exporting figure panels; preparing figures)（2026-11-13） |
| npj Digital Medicine | 89 / 183 | 170 | 5–7 pt | Nature Portfolio figure guide (the journal page needs a login; Nature numbers used)（2026-11-13） |
| IEEE TPAMI / TMI | 88.9 / 181.9 | — | 8–10 pt | IEEE author templates: 3.5 in / 7.16 in columns, 8 pt Times callouts（2026-11-13） |
| CVPR | 82.6 / 174.6 | — | 7–9 pt | CVPR author kit: 3.25 in columns, 6.875 in text width, 10 pt body, 9 pt captions（2026-11-13） |

共通規則（`rules`，也是可調的資料）：圖例先找資料最少的內側角落（4×4 佔用度：點、線段、長條、括號、小圖都算），
沒有就放圖外上方一列（≤ `legend_top_max` 項），再多就圖外右側；不加框時有 85% 白底；≤4 條線且有空間時可改成線尾直接標（`legend.pos = "direct"`）。
軸 4–7 個主刻度、5% 留白、單位寫在標籤、必要時才用科學記號；刻度字太擠時略過字、保留刻度。
優先順序：比較在印刷大小下讀得出來 → 不擋資料 → 最小字級 → 越少裝飾越好。

### 軸

`xaxis/yaxis.scale`：`linear`、`log`（log₁₀）、`symlog`（`linthresh`）、`stretch`（`segments: [[a, b, 權重], …]`，例如 `[[0.8, 1, 3]]` 把 0.8–1.0 拉長三倍；
權重 0 ＝斷軸，用來處理離群值）。分段交界與斷開處在軸上畫斷開記號 `//`。長條圖的值軸不從 0 開始時，斷開記號預設不畫，每張圖自己開（`opts.break_mark`，編輯器選項「斷開記號」；助手也可以建議）。
格線：`axes.grid`（none／y／x／both）、`grid_minor`＋`grid_density`（每個主格分幾格）、`grid_dash`、`grid_color`＋`grid_alpha`（預設黑 20%、0.4 pt），永遠畫在資料後面。

### 色盤

`colors.palette` 可以是顏色清單或 `palettes.json` 的名稱：Okabe–Ito、Paul Tol（bright／muted／vibrant）、重點一色＋灰，以及 20 個 Lore 主題配色的**靜態複本**
（`theme-<id>`：從 theme.css 複製一次、加深到白紙上對比 ≥ 3:1；圖永遠不跟 app 主題變）。
`series.ours = true`：這條用強調色（`colors.emphasis`，預設色盤第一色），其他系列向灰色淡化（`colors.mute`）；只有一個系列、「我們的」是一個類別時用 `opts.emphasis: ["Ours"]`（B1 會自動選名字是 Ours／Proposed 的類別）。每個樣式都套用同一規則。`opts.highlight` 指定要突出的系列，其他變灰。
公開樣式與標為色盲友善的色盤，前五色在紅綠、藍黃色盲模擬下兩兩 ΔE ≥ 10（測試檢查）。
顏色規則（`formats.json` 的 `rules.colour`，助手照著做）：強調色只給標為「我們的」系列或類別，且是整組最深的顏色；基準方法較淺、照資料順序；
只有一個系列又沒標「我們的」時用樣式的中性色（`colors.single`，可選 `opts.ramp` 依數值由淺到深），從不用強調色；不是方法比較的系列
（訓練／驗證、指標、類別）不用強調色，訓練曲線驗證實線、訓練虛線（L1）。ROC／PR 與嵌入圖不畫格線；放大小圖 0.4 pt 細框、淡色連線、不畫格線；
誤差線 0.6 pt、端點約 2 pt（`bars.err_width`、`bars.cap`）；盒鬚圖填色 60%（`box.alpha`）；有框圖例可用 0.3 pt 淺灰框（`legend.frame = "light"`、`frame_width`、`frame_color`）。

### 組合圖（一張圖 ＝ 版面＋多格）

`grid: { cols, rows, gap, labels, legend: top|right|bottom, sharey, sharex, share_color }`；每格 `span`（欄）與 `rowspan`（列），依序放進第一個放得下的位置。
編輯器：加一格（選種類，或複製這一格）、拖曳格的標籤排順序、每格自己的種類與資料、共用圖例／軸／色條、格標 a b c 自動、
頁面大小（單欄／1.5 欄／雙欄／整頁）、「併入其他圖…」（另一張 `.plot.json`；PaperComposer 列出這篇論文的其他圖）。

### 3D

只用在資料本身是 3D：`contour`（T1，優先）、`surface`（T2，視角 `opts.azim/elev` 存在規格裡）、`scatter3d`（T3），醫學影像用三個正交切面的熱圖（T4）。

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
- `font.math`：預設 `katex`＝`$…$` 裡的變數與方程式全部用 TeX 字型（作者 2026-11-13 決定）；`$…$` 外的文字用樣式的字型。`text`＝數學式裡的字母、數字改用圖的字型。
- 從 PaperComposer 打開時，論文對照表（lore-notation）的巨集交給 KaTeX，所以 `$\Lrec$` 這種登記過的巨集畫得出來。

## 6. 輸出

| 格式 | 怎麼做 | 用在 |
|---|---|---|
| PDF | headless Chrome 列印一頁（`@page` ＝圖的 mm 大小，邊界 0），字型內嵌、全部向量 | 論文 |
| SVG | 有 `pdftocairo` 就從 PDF 轉（字變路徑，任何軟體都一樣）；沒有就用編輯器的 SVG＋內嵌用到的 KaTeX 字型 | 論文、網頁 |
| PNG | 有 `pdftoppm` 就從 PDF 轉（`-r <dpi>`，和 PDF 一模一樣）；沒有就用 Chrome 截圖 | 簡報 |

- 規格永遠跟著圖：PaperComposer 是 `figures/<名稱>.plot.json`（和 PDF 同名）；PNG 把規格寫進自己的 `iTXt` 區塊（`lore-plot`），
  SVG 寫進 `<metadata>`。所以 SlideComposer 的圖片就算經過 .pptx 存檔再打開，仍能重新編輯。
- 沒有 Chrome：輸出失敗並說明（「需要 Chrome 才能輸出 PDF」），編輯與儲存規格不受影響。**輸出不在離線測試內**（需要瀏覽器）。

## 7. 自動產生（LLM 任務 `plot-propose`）

`commons/crates/lore-plot/tasks.toml`，類別 `create`，app 合併進自己的任務表（像 lore-flow）。
輸入：資料的欄名與前幾列、作者的一句話（「比較我們的方法與 4 個 baseline 的 Dice 和 HD95，分組長條加誤差線」）、可用的樣式 id。
輸出（JSON schema，清單都是封閉的）：圖的種類、用哪些欄、系列名稱、軸標籤（可含 `$…$`）、格數、樣式 id、圖說草稿。
模型不給顏色、大小、座標；這些來自樣式。回答再經 `Spec::validate`（欄名必須存在）才交給編輯器。

## 7b. 圖表助手（LLM 任務 `plot-assist`，類別 `tidy`）

編輯器上方「助手」打開右側的助手欄。助手**只輸出規格的值**（給現有的圖表函式用），從不寫程式：

- **幾種畫法**：目前的資料（＋一句話）→ 2–4 個不同種類或格式的圖，每個一句說明，縮圖並排，按「套用」換成那一個（可以復原）。資料原樣帶著，不改、不捨入。
- **照這句改**：「拉寬一點」「把 0.8–1.0 拉長」「legend 擋到曲線」「跟其他圖一致」→ 具體的修改 `[{ path, value, why }]`（規格裡的 JSON pointer，永遠不碰 `/data`），
  在伺服器套到副本上、檢查 `Spec::problems`，頁面一次套用（一步復原），並列出「改了什麼：前 → 後、為什麼」。勾「附上目前的圖」時把目前的圖（110 dpi PNG）
  交給能看圖的引擎（`plot-assist-vision`）。
- 最多問一個問題（資料意義不明時，例如某欄是什麼、越高越好嗎），其他自己決定並說明假設。

提示詞在 `commons/crates/lore-plot/prompts/assist.md`（公開、中性，不寫字型名稱），執行時填入 `{catalogue}`（formats.json 一格一行）、`{style_preset}`、`{venue}`，
以及作者自己的偏好：`private/shared/plot/assist.md`（自由文字）＋他的預設樣式（經 lore-home）。PaperComposer 與 SlideComposer 的任務表都合併了這三個任務
（`plot-propose`、`plot-assist`、`plot-assist-vision`），所以從論文重新打開圖 → 編輯器 → 助手 → 存回，會更新同一個圖檔。

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
| `POST /plot/api/assist` | `{ mode: alternatives｜adjust, spec, tables, request, style, svg, look }` → `{ alternatives[] ｜ spec＋changes[], note, question, engine }` |

頁面依序載入 `/plot/` 的 `plot.js`、`charts.js`、`charts3d.js`、`formats.js`、`editor.js`（`lore_plot::assets::SCRIPTS`）與 `editor.css`，然後
`LorePlot.edit({ spec, macros, title, target: "paper" | "slide", save: async ({ spec, svg }) => … })` 打開全螢幕編輯器（`<dialog>`），
按「完成」呼叫 `save`；`save` 由 app 寫檔。

### PaperComposer

- 論文右鍵選單「繪圖…」（在「新增圖…」旁）：編輯器 → 完成 → `POST /api/papers/{name}/plot`（`{ stem, spec, svg }`）
  → 伺服器輸出 `figures/<stem>.pdf`＋`figures/<stem>.plot.json` → 和「新增圖…」同一條路插入
  `\begin{figure}[t] \centering \includegraphics[width=\linewidth]{figures/<stem>.pdf} \caption{…} \label{fig:…} \end{figure}`
  （標籤由 `newLabel` 取、不重複；游標處插 `\ref{fig:…}`，不打「Figure 1」；需要時 main.tex 加 graphicx）。
- 點頁面上的圖：圖檔旁有 `.plot.json` 時角落出現「編輯」小按鈕（`ui/figedit.js`，流程圖的 `.flow.json` 同一個按鈕），按了打開編輯器；雙擊照舊；完成後覆寫同一個 PDF 與規格（有規格就代表是這個工具產生的檔），圖片重新載入。
- lore-notation：插入的區塊用登記的 `\label`／`\ref`，不動 `notation.tex`；`lore-plot` 的測試把產生的區塊放進合成論文跑 `check_document`。

### SlideComposer

- 上方「插入」選單「圖表」：編輯器（`target: "slide"`，預設樣式 `slide`）→ 完成 → `POST /api/decks/{name}/plot`（`{ spec, svg, dpi }`）
  → 伺服器輸出 PNG（預設 300 dpi，規格寫在 PNG 裡）存成 deck 的 asset → 插入圖片物件。
  PPTX 的圖片只放 PNG：lore-canvas 的 PPTX 輸出沒有 `svgBlip`＋PNG 備援，PowerPoint 不能直接顯示 SVG。
- 選取圖片：asset 是帶規格的 PNG 時選取框右上角有「編輯圖表」小按鈕（`SC_FIGURE_TOOLS`，流程圖同一個位置），按了打開編輯器；雙擊照舊；完成後同一張圖片換 asset（一個 `set_obj`，一步復原）。

## 9. 狀態與之後（2026-11-13；方程式見 §10）

已完成：規格、22 種圖、54 個格式（B1–C4）、組合圖、軸（log／symlog／分段拉伸／斷軸）、圖例佔用度、放大小圖、色盤（含 20 個主題的靜態複本）、
編輯器（格式、組合、助手）、匯入（CSV／TSV／JSON／XLSX／LaTeX 表）、輸出（PDF／SVG／PNG，規格跟著檔案）、PaperComposer 插入與重新編輯、
SlideComposer 插入與重新編輯、`plot-propose`／`plot-assist`。

沒有在真的環境測過：模型（自動、助手只用假模型測；`examples/serve.rs` 是假模型）、Windows／沒有 Chrome 的電腦。

之後：
- 圖說：存入論文時用圖的規格寫圖說草稿（現在是編輯器「圖說」欄或空白）。
- SlideComposer：簡報上的圖若要跟著投影片的配色，現在只有私人樣式 `mine-slide`；簡報風格包（style pack）的配色還沒接。
- 3D 曲面的軸刻度字在角落會擠在一起（只標最小／最大值）。
- 組合圖拖曳：現在拖的是格的標籤，不是圖上的格。
- 從 LoreReader 的論文 PDF 抓表格（lore-docsrc 的 PDF 表格）還沒接；LaTeX 表可以。


## 10. 會動的方程式（`kind: "equation"`，2026-11-13）

計畫與分層：`docs/living-equations.md`（這裡做的是第一層）。同一個編輯器、同一份規格、同一套 TeX 字形；PaperComposer、SlideComposer、LoreReader 都用它。

### 規格

一格 `{ "type": "equation", "eq": { … } }`（`equation` 不在 `TYPES` 裡：`plot-propose`／`plot-assist` 不會選它，`Spec::problems` 改查算式）：

| 欄位 | 內容 |
|---|---|
| `template` | 內建模板 id（`equations.json`），自訂算式時沒有 |
| `expr` | 安全算式語言（下方），模板也存一份，檔案自己完整 |
| `latex` / `tex` | 論文的 LaTeX（只顯示）／模板的 LaTeX |
| `x` | 橫軸變數 `{ name, label, min, max, color? }` |
| `vars[]` | `{ name, label, role: param｜vector, value, paper, min, max, step, log, integer, color? }`；`paper` ＝論文的值（滑桿上的標記、虛線曲線） |
| `output` | `curve`（預設）或 `bars`（算式給向量，例如 softmax） |
| `reduction[]` | 把論文記號化成小例子的假設（K = 3 類、N 個負樣本相同…） |
| `properties[]` | 可檢查的性質：`nonneg` `bounded` `zero_at` `min_at` `max_at` `increasing` `decreasing`（`var`、`at`）`symmetric` `odd` `equals`（`expr`）`sums_to`，`when` ＝假設的參數值 |
| `animate` | `{ var, frames, ms }`：GIF 掃的參數 |
| `family` | `{ var, values, on }`：一組參數值的曲線族 |
| `show` / `at_x` / `mark` / `yrange` | 曲線 `f`｜`dx`｜`d:<參數>`、敏感度讀值的點、在圖上標點、模板的 y 範圍 |

### 安全算式語言（`equation::expr`、`ui/plot/eqcore.js`）

自己的剖析器與求值器，兩份實作，`ui/plot/eqtests.json` 的案例兩邊都要過（Rust 測試＋node 跑 `ui/plot/tests/eqcore.test.js`）。沒有 eval／Function；名稱只從宣告的變數、綁定的索引、常數與函式表查（JS 用 Map，`constructor`、`__proto__` 查不到）。
運算子 `+ - * / ^`、比較（1／0）、`&& || !`、`v[i]`（從 1 起）、向量 `[a, b]`、`[條件]`（Iverson）；
`sum／prod／vec(k, 1, K, …)`、`let(名, 值, …)`、`E(x, normal|uniform|bernoulli|categorical(…), …)`（固定種子 256 次 Monte Carlo，兩邊抽到同樣的樣本）；
`log exp sqrt abs sign min max clamp pow if piecewise ind sg sigmoid softplus relu gelu silu tanh erf log1p`、向量 `sum mean len dot norm norm1 cosine softmax logsumexp argmax argmin`、
損失積木 `ce bce focal_w focal dice huber mse mae kl_gauss info_nce triplet smooth_ce hinge`。
上限：4000 字、巢狀 48 層、向量 4096、每次求值 200 萬步（巢狀加總與期望值都算進去）。

### 模板（`ui/plot/equations.json`）

CE、BCE、focal（γ, α）、soft Dice、CE＋Dice、Huber（δ）、MSE、MAE、KL（兩個高斯，方向 `q‖p`）、InfoNCE（τ、N、s⁻；封閉式 `log(1 + N e^{(s⁻−s⁺)/τ})`，和逐項加總相等由性質檢查）、triplet（margin）、
label smoothing（ε、K；最小值在 `1−ε+ε/K`）、softmax 溫度（長條）、ReLU／Leaky、GELU、SiLU、sigmoid、tanh。每個模板的性質由測試全部檢查，數值對照手算點。
模板也是 lore-notation 的一層（`equation::template_notation`：`eq:<id>` 方程式＋`v:<符號>` 符號，帶顏色角色），不是另一個方程式庫。

### 檢查（伺服器，模型的回答一律先過）

`equation::problems`：能剖析、名稱都有、範圍合理、目前與每個參數兩端在 201 點上大多有值、沒有 > 10⁹ 的數；擋住就不能用。
`check_properties`：逐條數值檢查，失敗照實顯示（✓／✗＋哪裡不成立），不擋。

### 模型任務 `equation-plot`（`tasks.toml`，類別 `create`，雲端優先）

輸入：論文的 LaTeX、符號表列（LoreReader symbols.json／對照表）、周圍的句子。輸出 JSON：模板＋論文的值，或自訂算式＋變數＋化簡假設＋2–5 個性質；`animate` ＝論文自己調的參數（消融、超參數段落），沒有就留空。
不能用時附理由重問一次，再不行把理由與草稿交回編輯器（手動）。自動放進投影片（`fit_for_auto`）只接受：模板、`animate` 來自論文且有論文的值、那個參數的符號或名稱真的出現在論文的調參／消融／超參數段落、性質全過。

### 編輯器

左：模板（損失／函數）、自訂、「從論文的式子」（LaTeX → 動起來）。右：算式（論文的 LaTeX、我們算的 LaTeX、算式、假設、✓✗）、參數滑桿（論文值標記）、敏感度（目前點的正規化 ∂f/∂θ 長條、曲線 f／∂f/∂x／∂f/∂θ）、動畫（參數、格數、ms、GIF、播放、曲線族），再來是一般的軸、圖例、樣式。
下方 **互動｜輸出**：互動 ＝ `eqlive.js`，app 裡看的版本，跟 app 主題（深色、發光曲線、改值時曲線平滑變形、點與切線跟著滑、2D／3D 曲面＋等高線＋沿 −∇f 滾的珠子，拖曳旋轉，只在有東西動時 requestAnimationFrame）；
輸出 ＝ 檔案的樣子（論文／簡報樣式，從不跟主題）。

**變數顏色（3Blue1Brown 式）**：每個變數一個固定的顏色角色（index，不是顏色）：函數本身 0、橫軸 1、參數 2、3…；lore-notation 的 `Entry.color` 可以指定（模板層 < `private/shared/notation.json` < 文件自己的 notation），
`/plot/api/equation/roles` 查（LoreReader 傳符號表、PaperComposer 傳論文的 notation、SlideComposer 傳簡報的 notation；同一個方程式裡角色不重複）。角色在 app 裡對到主題色，輸出時對到圖的樣式色盤；同一個符號在 LaTeX（`\textcolor`，mathsvg 照每個字的顏色攤平）、滑桿、曲線、敏感度長條、3D 軸都是同一色。
論文的圖（期刊）：字與軸標籤不上色（§2 的規則），曲線仍用角色色；簡報的圖：標籤也上色。論文曲線的圖例跟著文件語言：英文 "γ = 2 (paper)"、中文「γ = 2（論文）」（`eq.lang`；論文報告取簡報的語言）。

### GIF

瀏覽器用同一份 `plot.js` 畫每一格（`LorePlot.eq.frameSpecs`：參數來回掃、經過論文值時停 3 格且讀數用論文曲線的顏色、y 軸整段固定、論文曲線一直在、圖例一行在上方），
`POST /plot/api/gif` 在伺服器柵格化（Chrome 印多頁 PDF → `pdftoppm`；沒有 poppler 就一次截幾格再切開）並編碼：一個全域調色盤（最常見的顏色原樣保留，白與黑必在，抗鋸齒用 NeuQuant），線與字不抖色，無限循環；節奏、逐格延遲、只存變了的部分與熱圖抖色見 §11，
規格放在 `LOREPLOT1.0` 應用擴充區塊（`figure::spec_in_gif`）。預設 24 格（＋停格）、80 ms、200 dpi、寬最多 1200 px；簡報上的方程式圖 150 × 95 mm、刻度 16 pt、軸標 18 pt。

### 接到 app

- LoreReader：`/plot` 掛上（`lr-server` `plot_routes`），符號面板「式」每列的 ✦ 與段落裡式子的選單「動起來」→ `equation-plot` → 編輯器（互動）。模型不能用時編輯器帶著論文的 LaTeX 打開，手動選。
- PaperComposer：右鍵「會動的方程式…」→ 編輯器 → 存成 `figures/eq-<模板>.pdf`＋`.plot.json`（向量 PDF，和其他圖同一條插入路徑：`\label`、`\ref`、lore-notation 檢查）。要 PNG 用匯出。
- SlideComposer：工具列「會動的方程式」→ 編輯器 → 圖片是 GIF（帶規格；選取時「編輯圖表」重新打開）。論文報告：方法／損失頁的主要損失（`\mathcal{L}` 在左邊的式子，損失頁優先）走 `equation-plot`，通過 `fit_for_auto` 才在那頁式子下方放一張 GIF（一份簡報一張，220 × 95 mm，佔文字欄的寬），否則跳過並在警告裡說原因；微調視窗「會動的方程式 要／不要」。
  那頁的方程式也照同樣的角色上色（簡報樣式的色盤，和 GIF 一致）：編輯器看的 LaTeX 用 `\textcolor`，`.pptx` 的 OMML 在那些字的 `a:rPr` 加 `a:solidFill`；lore-notation 照樣讀到同樣的符號。
- `.pptx`：GIF 原樣存進 `ppt/media/*.gif`（`image/gif`），PowerPoint 播動畫；規格仍在 GIF 裡。

**沒有在真的環境測過**：真的模型回答 `equation-plot`；PowerPoint／Keynote 播放匯出的 GIF；Chrome／poppler 柵格化不在離線測試裡（`examples/serve.rs`＋headless Chrome 截圖手動檢查過）。

## 11. 模擬場景（`eq.sim`，會動的方程式第二層，2026-11-14）

計畫：`docs/living-equations.md`。同一種圖（`kind: "equation"`），`eq.sim` 有值時畫的不是曲線而是一次「跑」：每一步都真的算（Adam／GD、格點上的數值積分、EM、Lloyd），不是預錄。

### 規格（`eq.sim`）

| 欄位 | 內容 |
|---|---|
| `scene` | `fit1d`（1D 高斯 q 追目標 p）· `classes`（K 類 softmax 的長條）· `fit2d`（2D 高斯混合 q 追樣本）· `cluster`（k-means／GMM-EM／softmax 分區） |
| `target.mix[]` | 目標＝高斯混合：1D `{ w, mu, sigma }`，2D `{ w, mu: [x, y], cov: [sxx, sxy, syy] }`（可在畫面上拖） |
| `samples` | `{ n, seed }`：從目標抽樣（`nll`、2D、分群；固定種子，Rust 與 JS 抽到同樣的點） |
| `model` | 起點：1D `{ mu, sigma }`、類別 `{ z }`、2D／分群 `{ k, seed, sigma, mu? }` |
| `classes` / `cluster` / `x` / `box` | `{ K, y }`；`{ method: kmeans｜em｜ce, k }`；1D 範圍；2D 範圍 |
| `terms[]` | `{ kind, w, on, name, eps?, mu0?, expr?, color? }`：損失 = Σ wᵢ Lᵢ；`name` ＝權重的符號（`a`、`\lambda`），在 lore-notation 有登記顏色就用那個顏色 |
| `opt` | `{ method: adam｜gd, lr, steps }` |
| `plan[]` | `{ at, w[], reset? }`：GIF 用的權重變化（互動時不用）；`reset` ＝回到同一個起點（同一次跑比較兩種 loss） |
| `frames` / `ms` / `hold` | GIF 的格數、每格毫秒、最後停多久 |

項（`LoreSim.TERMS`、Rust `equation::sim::kinds`）：
`fit1d`：`kl_pq`（正向 KL）`ce`（交叉熵 −∫p log q）`kl_qp`（反向 KL）`nll`（樣本）`moment`（(μ−m)²+(σ−s)²）`anchor`（(μ−μ₀)²）`entropy`（−H(q)）`mse`（∫(q−p)²）`expr`（安全算式，名稱 `mu`、`sigma`）；
`classes`：`ce`（one-hot）`smooth`（label smoothing，ε）`l2` `entropy` `expr`（`z`、`q`、`y`、`K`）；`fit2d`：`nll` `kl_pq` `kl_qp`（36×36 格點）；`cluster`：`kmeans`、`em`，或 softmax 分區的 `ce` `l2` `smooth`。
梯度：中央差分（自訂項也一樣）。改權重、開關一項、加減項、拖目標：從目前的 θ 繼續，Adam 的動量重來（舊梯度的長記憶會讓它停住）；↺ 從頭。

### 檢查（Rust `equation::sim`）

`problems`（`equation::problems` 遇到 `sim` 改用它）：場景、項屬於這個場景、權重 0–1000、自訂項能剖析且只用允許的名稱、目標成分（σ > 0、2D 共變異數正定）、K／y、k、樣本數 ≤ 2000、學習率、步數 ≤ 5000、plan 的權重。
`values`：`fit1d` 與 `classes` 每一項的值，和 `simcore.js` 一樣算（400 點格點）；共用案例 `ui/plot/simtests.json` 兩邊都過（node `tests/simcore.test.js`、`cargo test -p lore-plot`）。
性質（node）：正向 KL／交叉熵的最佳解 μ = E_p[x]、σ² = Var_p；反向 KL 落在一個峰；label smoothing 收斂到平滑後的目標；權重 0 或關掉的項對整次跑沒有影響（θ 完全相同）；拉力 = −w ∂L/∂θ；k-means／EM 的目標不會變大；2D NLL 會降；`plan` 的 `reset` 後反向 KL 選一個峰。

### 畫面（`ui/plot/eqsim.js`）

左：動畫（目標 p ＝ `--accent-2`、模型 q ＝ `--accent`；發光線、點格）；q 的 μ 下方每一項一支拉力箭頭（−wᵢ∂Lᵢ/∂μ），σ 的拉力畫在 q 的半高處（↔ 變寬、→← 變窄）；2D 在每個成分的中心。
右：每一項一列（開關點、權重滑桿與數值、這一項的 LaTeX、拉力、加權後的值），下面是加權 loss 隨步數的堆疊圖（每項一色，負的部分在 0 以下），權重改過的地方一條虛線。項的顏色：lore-notation 登記的顏色角色優先，否則主題的 `--accent-3`、`--warn`、`--ok`、`--err`…
控制：▶／⏸、+1（走一步）、↺、1×／2×／4×、加一項（這個場景的項；自訂 = 在選單裡打算式，邊打邊檢查）；拖目標的峰（左右 = μ、上下 = 權重、滾輪 = 寬度）、2D 拖成分中心、類別圖點一條 = 正確類別。
效能（headless Chrome，1920×1080）：每個場景都 60 fps；每格計算＋繪圖 1D 約 2 ms、2D 混合 5 ms（再加正反兩個格點 KL 約 7 ms）、softmax 分區 10 ms；畫面的拉力與下一步共用同一次梯度。WKWebView 裡沒量過。

### 輸出

檔案用圖的樣式（白底、樣式的色盤，不發光，從不跟主題）：`CHARTS.equation` 遇到 `sim` 改由 `LorePlot.eqSim.chart` 在格子裡畫整頁（960×540 的虛擬頁：標題、動畫、項的列、堆疊圖），所以 PNG／PDF／編輯器的「輸出」都是 `plan` 跑完的最後一格（論文裡的模擬圖是一張靜態圖，作者決定要不要用）。
**GIF 節奏**（所有方程式 GIF）：`LoreSim.schedule(diffs, F, 0.25)`（Rust `sim::schedule` 同一個函式）：格與格之間的距離 = 75 % 畫面變化（每一步在小畫布上的像素差；曲線掃參數時用曲線的變化）＋ 25 % 時間，最後一格停 1.6 s（掃參數：兩個折返點各停 0.7 s、論文值那格停 4 倍）。`LorePlot.eq.gifFrames` → `{ frames, delays, dither }`，`POST /plot/api/gif`（SlideComposer 的 `/plot` 存檔、論文報告的自動 GIF 都用它）。
GIF 編碼：每格只存變了的矩形，矩形裡沒變的像素用透明索引（標題、列、軸不再每格重存）；熱圖矩形內加 4×4 Bayer 抖色（線與字照舊精確），熱圖先量化成 64 階，跨格沒變的像素就不用存。熱圖場景預設 60 格、寬 960 px，其他 100 格、寬 1200 px。量到的（簡報樣式，2026-11-14）：組合損失 0.86 MB、softmax 0.28 MB、k-means 65 KB、softmax 分區 1.2 MB、高斯混合 2.9 MB、第一層 focal 掃描 150 KB；做一張 GIF 要 25–40 秒（Chrome 印每一格＋poppler）。
KaTeX 字面在量字之前先載入（`LoreMathSvg.loadKatex` 等 `document.fonts.load`）：以前 ∑ 第一次用 KaTeX_Size1 時用備用字量寬度，下標會疊進 ∑ 裡。

### 接到 app

- 編輯器：左邊「模擬」一組（`equations.json` 的 `scenes`）；互動 ＝ 這個畫面（改的權重、項、目標存回規格），輸出 ＝ 檔案的樣子；右邊「模擬」：步數、學習率、GIF 格數與毫秒、GIF。
- `LorePlot.animateLatex`（LoreReader ✦、KnowledgeBase 卡片的式子）：組合損失或分佈的目標（`/plot/api/equation/scenes`，`sim::suggest`，不用模型）先問要哪個模擬或曲線。
- SlideComposer 放映「互動」：見 `SlideComposer/CLAUDE.md`（Show）。互動畫面的權重改動是 GIF 的起始權重，`plan` 的步驟照樣蓋過；從論文來的組合損失，場景用的是場景自己的項（論文的 LaTeX 與符號沒有對到項上，見 living-equations.md 提案 3）。目標分佈不能增減成分；寬度只能用滾輪。

