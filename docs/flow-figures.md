# 流程圖與論文圖（規劃）

作者 2026-11-13 的要求，記下來給之後的工作用。程式在 `commons/crates/lore-flow`（L1 圖的內容 → L2 位置與連線 → L3 lore-canvas 物件）與共用編輯器 `commons/ui/flow/`；SlideComposer 已接上（§5），PaperComposer 是下一步（§6）。

## 1. 共用功能，不屬於任何一個 app

- 流程圖是 Lore 的共用功能：SlideComposer、PaperComposer（寫論文時畫方法圖）都用同一套；之後其他 app 需要也接同一套。
- 介面（討論、版面選擇、預覽、圖例）做成共用元件（`commons/ui`），模型呼叫走 lore-llm 的共用任務，範例與模板放作者的私人資料夾（`private/shared/flow/` 或各 app 的 private，經 lore-home），不進公開 repo。

## 2. 圖本身的操作

- 流程圖的物件可以組成群組，變成「一張圖」；群組後固定長寬比放大縮小（字、線寬、箭頭一起縮放）。
- 任何物件或群組可以另存成圖片（PNG，之後加 SVG / PDF），在 SlideComposer 與 PaperComposer 都可以。
- 已有：L3 的模組是可拆開的群組；`lore-flow draw … --png` 從命令列出 PNG；SlideComposer 的「一張圖」與「另存圖片」（§5）。

## 3. 從題目到圖的步驟（照作者畫圖的習慣）

每一步都是討論（模型提問與建議，作者決定），不是一次生成：

1. **出發點**：論文題目與方法（PaperComposer 的論文、LoreReader 讀過的論文或一段描述）。
2. **大綱**：先討論方法的大綱。
3. **分幾個 block**。
4. **重點是模型還是流程**。
5. **模型細節要到多細**（只到模組名稱，或到層、維度）。
6. **哪些變數、方程式要出現**（用 lore-notation 的登記：符號與 `\ref{}`，不手打）。
7. **每個 block 的輸入、輸出是什麼，block 本身是什麼**。
8. **結果是一份線性的列表**，每個 block 一條：
   ```
   block 1: a  -> b  -> c  -> d
   block 2: a2 -> b2 -> c2 -> d2
   ```
   這份列表就是 L1（lore-flow 的文字格式 `fig … / panel … / x -> y`），可以直接看、直接改。

然後是**排版與顏色**，在輸出前決定：

9. **排版**：全部左到右、分成直的 block、左邊兩個小的右邊一個大的……（版面模板，需要很多個；作者會再增加）。
10. **顏色與樣式**：依 block 種類上色、線的樣式（實線、虛線）、模組畫法。
11. **圖下方的說明（圖例）**：例如「綠色虛線＝…」「粉色 block＝conv block」；要不要放、放哪些，由作者決定。
12. **生成**：產生可編輯的原生物件（SlideComposer 的投影片物件；PaperComposer 輸出圖檔或 TikZ，之後再定）。

已有（2026-11-13）：上面 1–12 全部，見 §5。要補：PaperComposer 端的接法（§6）。

## 4. 之後（下下下階段，不急）

- 完整的論文繪圖工具：論文常見的圖都要能畫（流程圖、架構圖、結果圖表、示意圖、比較圖格……）。
- 美術風格可客製（作者自己的風格包，像 SlideComposer 的風格版型）。

## 5. 現況（2026-11-13）

**共用編輯器**（像繪圖工具一樣是共用功能）：`lore_flow::web::router` 掛在 app 的 `/flow/*`，頁面 `LoreFlow.edit({ spec, doc, frame, target, source, sources, save })`
打開全螢幕編輯器，外框和繪圖編輯器同一套（上方列：流程圖 · 名稱 · 復原／重做 · 助手 · 下載 SVG · 取消 · 插入投影片／存入論文）。

- 左邊：步驟 1–8。每一步「問模型」＝一次 `flow-discuss`（類別 create，JSON，每步封閉的回答格式）：模型說兩三句、給 2–4 個選項（★ 是建議），作者點選或自己寫，「下一步」自動問下一步。
  6 變數與方程式：模型從對照表提出符號（KaTeX 顯示，新的標「新」），作者勾選，也可以「從對照表加…」。7 每個 block 一張小卡（輸入、輸出、是什麼）。
  8 線性列表：「產生列表」＝ `sc-flow-graph`（討論的決定 → L1），每個 block 一行 `block 1 (…): a -> b -> c`。
- 右邊：01 列表（L1 文字，直接改）· 02 版面（每個模板用目前這張圖畫縮圖；每個 block 的方向 自動／→／↓）· 03 顏色與線（block 種類：顏色＋名稱，指定給 block 或模組；
  每種線：顏色、實線／虛線…、意思）· 04 圖例（自動／自選／不放；自選＝勾選要放的列、改文字，圖下方或上方）。
- 助手（`flow-assist`，類別 tidy）：「照這句改」只改規格的值（`/dirs/b`、`/legend/pos`…，伺服器先畫一次確認畫得出來），列出改了什麼；「幾種版面」給 2–4 個同內容的版面縮圖，「套用」。
- 規格（`FlowSpec`，`lore_flow: 1`）：L1 文字＋版面、方向、主題、block 種類與顏色、線的樣式與意思、圖例、框的大小、討論的決定。圖的顏色從不跟 app 主題。
- 資料：公開中性的 20 個版面模板（`commons/crates/lore-flow/layouts.json`）、9 個 block 色（`figure::PALETTE`）；作者自己的放 `private/shared/flow/`
  （`layouts/`、`themes/`、`modules/`、`palette.json`、`examples/*.json` 給列表那一步當範例、`assist.md` 作者的習慣），各 app 的 `private/<App>/flow/` 也會讀（SlideComposer 原有的 `templates/`、`themes/`）。

**SlideComposer**：工具列「流程圖」與 AI 選單「流程圖」打開編輯器（出發點預設帶入 論文 面板開著的那篇的題目與摘要）；「插入投影片」＝伺服器畫出原生物件
（形狀、連接線、文字，數學式是 Office Math），整張圖是**一張圖**：一個群組，固定長寬比，放大縮小時字、線寬、箭頭一起縮放（`Op::Scale`，一步復原），
規格掛在群組上（`deck.json` 與 .pptx 的編輯器狀態區，存檔再打開仍可重新編輯）。選取圖時選取框右上角的「編輯流程圖」小按鈕或 排列 →「編輯流程圖」重新打開（雙擊照舊改字，2026-11-13 作者決定），完成後原地換掉（同位置、同寬，一步復原）。
圖裡的新變數登記到這份簡報的 `notation.json`。任何物件或群組：排列 →「一張圖」（多選＝組成一張圖；單一群組＝開／關鎖定比例）、「另存圖片」PNG／SVG／PDF（存到簡報的 exports/）。

沒有在真的環境測過：真的模型（測試與截圖都是假模型）、PowerPoint 打開含一張圖的 .pptx（鎖定比例旗標是 `a:grpSpLocks noChangeAspect`）。

## 6. PaperComposer（2026-11-13 已接上；論文與書同一條路）

和「新增圖…／繪圖…」同一條路，不另做一套：

1. **入口**：論文右鍵選單「流程圖…」（在「繪圖…」旁，`ui/context.js` 的 `add("graph", "流程圖…", …)`）；`ui/figflow.js`（像 `figplot.js`）載入 `/flow/editor.css`、`/flow/editor.js`，
   `LoreFlow.edit({ target: "paper", doc: <論文名稱>, frame: [版心寬 pt, 高], source: 論文題目＋摘要＋方法一節（lore-docsrc 讀 main.tex）, sources: LoreReader 的論文, save })`。
2. **伺服器**：`router.merge(lore_flow::web::router(Flow::new(home).with_llm(FlowLlm).with_notation(<這篇論文的對照表>).with_app_dir(private/PaperComposer/flow)))`，
   任務表合併 `lore_flow::llm::task_table()`（`flow-discuss`、`flow-assist`、`sc-flow-graph`）；`FlowLlm` 和 `plot.rs` 的 `PlotLlm` 是同一個轉接（兩個 trait 形狀相同）。
3. **存檔**：`POST /api/papers/{name}/flow`（`{stem, replace, spec, svg}`）→ `figures/<stem>.pdf`＋`figures/<stem>.flow.json`（規格，含討論）。
   PDF 的數學式要是 TeX 字形，不是預覽 SVG 的 Unicode：編輯器在「存入論文」時把圖畫成 SVG、`$…$` 交給 KaTeX 攤平成 SVG（`plot.js` 已有的做法，抽成共用函式），
   伺服器用 `lore_plot::export::pdf`（Chrome）出 PDF、有 `pdftocairo` 時也出 SVG。登記過的巨集交給 KaTeX（`/api/papers/{name}/plot/macros` 同一份）。
   `figures/` 裡沒有 `.flow.json` 的 PDF 是作者自己的檔，永遠不覆寫（同繪圖的規則）。
4. **插入**：`placeFile(at, was, "figures/<stem>.pdf", caption, stem)` → `figure-block`（`floats.rs`）：`\begin{figure}… \includegraphics… \caption{…} \label{fig:…}`，
   標籤由伺服器取、不重複；游標處插 `\ref{fig:…}`，從不打「Figure 1」；需要時 main.tex 加 graphicx。
   圖說先用各格標題（`(a) …; (b) ….`，存檔回應的 `caption`），作者再改；不會插入空的 `\caption{}`。
   出發點：lore-docsrc 讀不到期刊 class 的 `\abstract{…}` 時，題目與摘要改由論文自己的標題欄位補上（2026-11-14）。
5. **對照表**：圖裡的新變數（`vars_new`）用 `lore_flow::notation::register_new` 寫進這篇論文自己的對照表（標 `flow-draft`，不寫全域、不動 `notation.tex`）；
   圖裡的標籤用 `@id` 指向登記的符號，登記過的巨集在畫圖前展開。插入的區塊跑 lore-notation 的 `check_document`。
6. **重新編輯**：點頁面上的圖，旁邊有 `.flow.json`（或 `.plot.json`）時圖角出現「編輯」小按鈕（`ui/figedit.js`），打開那個工具的編輯器，完成後覆寫同一個 PDF 與規格；雙擊照舊。
7. **測試**（離線、合成論文、假模型）：存檔產生 PDF 規格兩個檔（沒有 Chrome 時只檢查規格與錯誤訊息）、作者自己的 PDF 不被覆寫、插入的區塊通過 `check_document`、`\ref` 不是打字的編號、新變數只進論文的對照表。

做好的（2026-11-13）：`PaperComposer/crates/server/src/flow.rs`、`ui/figflow.js`、`ui/figedit.js`；出發點＝題目、摘要、方法一節（lore-docsrc 讀 main.tex；書讀 `\chapter`）；
數學式：`/flow/api/svg` 帶 `tex: true` 時每行數學標 `data-tex`，`LoreFlow.paperSvg` 用共用的 `commons/ui/mathsvg.js`（從 `plot.js` 抽出來，繪圖工具也用它）換成 TeX 字形；
PDF 由伺服器用 lore-plot 的 Chrome 輸出。繪圖工具也不會覆寫流程圖的 PDF（`.flow.json` 算已占用）。測試：`tests/flow_api.rs`（論文與書各跑一次，假的 PDF，不開瀏覽器）。
