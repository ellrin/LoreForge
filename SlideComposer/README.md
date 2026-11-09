# SlideComposer

任務式簡報編輯器：選「任務版型」（例如論文報告）＋「風格版型」（從 PPTX 收集的母片、標籤字型、色票、元件），放入論文等素材，快速產生初稿，再在畫布上細修，輸出 PPTX／PDF。與個人知識庫、對照表串聯。設計見 `../private/shared/docs/ecosystem-design.md`（作者的私人筆記） §7。

目前完成 **B3：畫布編輯器 v1**（Rust 核心 `commons/crates/lore-canvas` ＋ axum 伺服器 ＋ 純 JS DOM/SVG 編輯器）。生成流程（C3）尚未開始。

## 執行

```sh
cargo run -p sc-server            # http://127.0.0.1:7880 ，workspace 預設為 ../private/SlideComposer
SLIDECOMPOSER_WORKSPACE=/path/to/SlideComposer-private \
SLIDECOMPOSER_DECKS=/tmp/decks cargo run --release -p sc-server -- --port 7880
cargo test                        # 全部離線（合成測資）
```

- `SLIDECOMPOSER_WORKSPACE`：風格包與簡報所在的私人 repo（`styles/`、`decks/`）。
- `SLIDECOMPOSER_DECKS`：另指簡報資料夾（例如把真實簡報放在暫存區，不進任何 repo）。
- `SLIDECOMPOSER_CHROME`：PDF 用的 Chrome/Chromium 路徑（預設找 `/Applications/Google Chrome.app`）。
- 方程式需要 pandoc（`lore-math` 旁掛；`LORE_PANDOC` 可指定路徑）。

## 架構

| 位置 | 內容 |
|---|---|
| `../commons/crates/lore-canvas` | 場景模型（投影片／物件／文字段落與 run＝標籤＋強調）、風格規則、可復原的操作、幾何（連接點、黏著、對齊）、漸進揭露產生器、元件、PPTX ↔ 畫布轉換（未改動的物件保留原 XML，未改動的投影片逐位元組相同） |
| `crates/core`（`sc-core`） | workspace、簡報資料夾（`deck.json`＋`source.pptx`＋`assets/`）、匯入、存檔、匯出 PPTX、Chrome 列印 PDF、OMML↔LaTeX 快取 |
| `crates/server`（`sc-server`） | axum，127.0.0.1:7880，JSON API（見 `crates/server/src/lib.rs` 開頭的表） |
| `web/` | 編輯器（無建置步驟）：`render.js` 投影片 → DOM/SVG（編輯畫面、縮圖、列印共用）、`app.js` 互動、`panels.js` 側欄與對話框、`print.html` PDF 路由 |

只掛載目前這一頁；縮圖依投影片物件快取。所有編輯都送到伺服器變成一筆交易（驗證標籤與色票規則 → 套用 → 自動存 `deck.json`），復原／重做在 Rust 端。

## 編輯器功能（v1）

- 投影片排序（拖曳）、新增／複製／刪除／隱藏（備用頁）、講者備註、版面配置切換。
- 畫布：16:9、縮放、格線、吸附（物件邊緣／中心、投影片邊緣／中心；按住 Alt 暫停）、單選／多選（Shift、框選；框選可從投影片外的灰色區開始，⌘ 拖曳在物件上也框選）、移動、縮放（旋轉後也正確）、旋轉（Shift 15°）、對齊／均分、圖層順序、群組／取消群組、複製／貼上／再製、方向鍵微調（連按合併成一步復原）、移動時黏著的連接線即時跟隨、⌘Z／⇧⌘Z；視窗 ≤1024px 時投影片清單與側欄收成抽屜。
- 文字：雙擊進入 contenteditable（只有編輯時才接受輸入法）；**只能選標籤**（不能自由設字型字級，標籤有範圍時可在範圍內調字級）；強調標籤（key／note／concept-n／strong，⌘B＝strong）；項目符號、編號、兩層階層（Tab）、對齊含左右對齊、行距、段前、垂直對齊。
- 插入：矩形、圓角矩形、橢圓、三角形、梯形、多邊形、直線、箭頭、連接線（直線／肘形／曲線，可黏在圖形連接點上，圖形移動時自動跟著走）、風格元件（編號圓圈、模型梯形、標籤卡、重點框、語意遮罩、各種箭頭樣式）、圖片（裁切把手、透明度、外框、白色遮罩）、表格（編輯儲存格、插入／刪除列欄、底色）、方程式（LaTeX → KaTeX 預覽 → pandoc 轉原生 OMML）。
- 漸進揭露：選步驟（每步一組物件）→ 產生 N 頁，其他步驟依風格包變淡（圖片透明、文字灰、線條淡、方程式灰）；或議程模式（每個第一層項目一頁、當前項目加粗）；設定面板停靠在側欄，設定時仍可點選投影片上的物件。
- 動畫：5 種進場（出現、淡入、飛入、擦去、縮放），順序、觸發、時間。
- 匯出：PPTX（`exports/`，不覆蓋）、PDF（headless Chrome 列印 `print.html`）。
- 字型代替（`lore-fonts`）：簡報用到、這台電腦沒有的字型，畫面與 PDF 依序改用作者規則（`private/shared/fonts.toml` 的 `[fallback]`）、字寬相同的開源字（Calibri → Carlito）、同類字型（不認得的無襯線字先試較窄的 Carlito、PT Sans）。上方列出現「字型代替 N」時，點開看每個字型變成什麼；紅色表示這台電腦連代替字都沒有。字型檔放在 `private/shared/fonts/` 就能直接用，不必安裝；公司字型或授權字型的名稱只寫在 private。
- 方程式與文字同樣大小（PowerPoint 的原生公式不放大），避免一行混著文字與公式時溢出換行。

## 已知限制

見 `CLAUDE.md` 的「Gaps」。

## 開發說明

本專案與 Claude（Anthropic）協作開發，程式碼在 claude.ai 與 Claude Code 中共同撰寫。
