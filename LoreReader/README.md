# LoreReader

本機運作的 PDF 論文閱讀器。目標是用最短時間弄清楚一篇論文裡自己需要掌握什麼：沒學過的術語標成紅色、已在知識庫的術語顯示熟悉度；讀的時候存「引用句」（之後在 PaperComposer 引用）、加註記、翻譯、問 Claude。知識庫只放真正消化過的東西，所以**只有按「加入知識庫」才會建立卡片**，不會自動寫入。

> 原型 v0。已完成：書庫（repo 外、sha256 命名）、pdf.js 閱讀（文字層、縮放、搜尋、延遲繪製、縮圖）、不破壞原檔的頁面操作（刪除、複製、拖曳排序、從其他 PDF 插入、合併）、列印、螢光標記與註記、選取文字右鍵選單（複製、翻譯、問 Claude、存成引用句、加入知識庫）、知識標示（紅色＝沒有卡片）、從知識庫卡片連回 PDF 那一頁。
> 不在範圍：編輯 PDF 內容、投影片精靈（之後在 SlideComposer）、變數／方程式擷取、把註記寫進 PDF 檔（之後可做）。

## 執行

需要 [Rust](https://rustup.rs)（1.85 以上）。

```sh
cargo run -p lr-server            # http://127.0.0.1:7890
cargo test                        # 全部離線（合成 PDF、假的 KnowledgeBase 與本機模型）
cargo run -p lr-core --example synth_pdf -- demo /tmp/demo.pdf   # 示範用的合成論文（3 頁）
```

Mac App：在 LoreForge 根目錄執行 `scripts/install-apps.sh LoreReader`（`apps/desktop` 的 Tauri 殼）。App 用 7890 埠（知識庫卡片的連結指向這裡）；7890 已經有 lr-server 在跑時直接開那一個，被別的程式佔用時改用任意空埠（這時知識庫的連結到不了這個視窗）。App 登記為 PDF 檢視器：在 Finder 開啟的 PDF（雙擊、「打開檔案的應用程式」）會加入書庫並直接打開；要讓它成為預設的 PDF App，在任一 PDF 的「取得資訊 → 打開檔案的應用程式」選 LoreReader 再按「全部更改…」。圖示原始檔為 `apps/desktop/icons/logo.svg`（PaperComposer 的霓虹風格；換圖方式同 SlideComposer）。

知識標示、加入知識庫與卡片摘要需要 KnowledgeBase 伺服器在執行（預設 `http://127.0.0.1:7700`）；沒在執行時介面會直接說，紅色候選詞仍然會標。

## 書庫：PDF 不進 git

- 書庫在 **repo 之外**，預設 `~/Documents/LoreReader/library`；匯出（另存新檔）在 `~/Documents/LoreReader/exports`。書庫或匯出資料夾若在任何 git 工作目錄裡，程式會拒絕使用並說明原因。
- 開啟任何地方的 PDF（按「開啟 PDF」、拖進視窗或輸入路徑）都會**複製**進書庫，以內容的 sha256 命名（`ab/abcd….pdf`，同 KnowledgeBase 的圖片），原檔不動；同一個檔案再開一次不會重複。
- 每篇論文的資料在私人工作區 `private/LoreReader/papers/<sha256>/`（`lore-home` 第一次從 `private-example/LoreReader/` 建立）。寫入都是原子寫入（暫存檔＋改名）。
- 設定檔 `~/Library/Application Support/LoreReader/config.toml`（或 `$LOREREADER_HOME/config.toml`），也可用環境變數：

```toml
library = "~/Documents/LoreReader/library"   # LOREREADER_LIBRARY
exports = "~/Documents/LoreReader/exports"   # LOREREADER_EXPORTS
kb_url = "http://127.0.0.1:7700"             # LOREREADER_KB_URL
claude_model = "…"                           # 問 Claude 用的模型（不寫＝catalog 預設）
[engine]                                     # 翻譯用的本機模型（OpenAI 相容伺服器）
url = "http://127.0.0.1:8080/v1"             # LOREREADER_LLM_URL
model = "qwen3.5-9b-q8"                      # LOREREADER_LLM_MODEL
```

私人工作區的位置可用 `LOREREADER_WORKSPACE` 改到別處（測試、示範用）。

## 資料格式

每個檔案都有 `schema` 版本；不認得的欄位會保留。

| 檔案 | 內容 |
|---|---|
| `paper.json` | `schema: "lorereader.paper/1"`、`sha256`、`title`（PDF 的 Title，沒有就用檔名）、`original_name`、`doi`（前兩頁找到的第一個 DOI，沒有為 `null`）、`pages`、`added_at`、`derived_from`（頁面操作產生的新 PDF：`{op, from: [sha256…]}`） |
| `annotations.json` | `schema: "lorereader.annotations/1"`，`annotations: [{id, page, kind: "highlight" \| "note", color: yellow\|green\|blue\|pink, quote: {exact, prefix, suffix}, rects: [[x0,y0,x1,y1]…], note, created_at, updated_at}]`。`rects` 是相對於頁面的比例（左上為原點），所以任何縮放都對得上；`quote` 是原文與前後文，之後換渲染方式也能重新定位 |
| `quotes.json` | 引用句。`schema: "lorereader.quotes/1"`，`quotes: [{id, text, page, note, created_at}]`。`note` 是一行備註（可空白） |
| `chat.json` | 問 Claude 的對話。`schema: "lorereader.chat/1"`，`messages: [{role: user\|assistant, text, attachments: [{text, page, context}], at, engine}]` |

`GET /api/quotes` 回傳所有論文的引用句（每筆加上 `sha256`、`paper_title`、`doi`），給 PaperComposer 之後讀取；PaperComposer 那邊尚未實作。

之後的「論文符號／方程式表」也會放在 `papers/<sha256>/`（與 lore-notation 的登錄欄位相同，加上論文範圍、等級與誰確認的），目前的檔名不會和它衝突。

## 閱讀

- 左邊縮圖、中間頁面、右邊面板（註記／引用句／對話）；窄螢幕時縮圖與面板變成抽屜。深色／淺色與其他 App 相同（右上角切換）。
- pdf.js 文字層可以選取文字；縮放、符合寬度、頁碼跳頁、全文搜尋（⌘/Ctrl＋F，Enter 下一個）。只繪製視窗附近的頁面，縮圖捲到才畫。

## 頁面操作（不破壞原檔）

上方工具列「頁面」：複製這一頁、刪除這一頁、從其他 PDF 插入頁面（選一份 PDF → 點選要的頁，依點選順序 → 指定插在第幾頁之後）；左邊縮圖可以拖曳排序。變更先暫存（標題旁顯示「頁面已變更」），按「另存新檔」才用 Rust（`lopdf`）產生**新的 PDF**：存進書庫，同時複製一份到匯出資料夾（同名就加 `-2`、`-3`，不覆蓋）。原本的 PDF 永遠不動。書庫頁的「合併 PDF…」依點選順序把幾份 PDF 接成一份新的。

新 PDF 不保留原檔的書籤（outline）、表單與具名目的地；頁面上的連結註解保留。

## 列印

工具列的印表機按鈕：

- **Mac App**：開一個列印視窗（`print.html`，每頁以約 216 dpi 繪製，紙張大小用 PDF 第一頁的尺寸），繪製完成後顯示 macOS 的系統列印對話框（WKWebView 的原生列印），可以選任何已安裝的印表機或存成 PDF。
- **瀏覽器**：開新分頁 `print.html?sha=…&auto=1`，繪製完成後呼叫 `window.print()`。

> 本機檢查只確認了列印頁的版面（headless Chromium 以列印模式輸出 3 頁、Letter 尺寸）。**實體印表機與 Mac App 的原生列印對話框都還沒在真的 Mac 上測過。**

## 選取文字的右鍵選單

- **複製**。
- **螢光標記**（黃、綠、藍、粉紅）與**加註記…**：存在 `annotations.json`，右邊「註記」分頁列出，點一下跳到那裡。
- **翻譯**：lore-llm 任務 `lr-translate`，先用本機模型（`local:current`），連不上才用 Claude（`claude:selected`），與 PaperComposer 的順序相同。結果在小視窗顯示，可以「加入知識庫」。
- **問 Claude**：把選取的文字、頁碼和所在段落附到右邊的對話（任務 `lr-chat` → `claude:selected`，只用 Claude）。對話存在這篇論文的 `chat.json`。
- **存成引用句…**：句子（可修剪）＋一行備註，存在 `quotes.json`；右邊「引用句」分頁列出。
- **加入知識庫…**：呼叫 KnowledgeBase 的 `POST /api/nodes`。標題預填選取的文字，摘要自己寫（可按「用模型起草摘要」呼叫 KnowledgeBase 的 `/api/draft`）。**初始狀態**可選「學習中」（預設：明天問、3 天後再問）或「已掌握」（已經熟的詞，直接記為「記得」，30 天後再確認）。來源記為 `{type: "pdf", ref: <sha256>, where: "p.<頁>", title: <論文標題>}`，內文附上所在的句子。

## 知識標示（原型）

工具列的標靶按鈕打開／關閉，旁邊有圖例：

- **紅色**：可能是專業術語、但知識庫裡沒有卡片。
- **淡琥珀色**：有卡片，學習中或久未複習。
- **細綠底線**：有卡片，已經記得。

每一頁的文字送到 KnowledgeBase 的 `POST /api/scan`（不記錄「遇到」），找出已有卡片的詞與狀態。紅色候選詞：KnowledgeBase 的候選輸出目前還是離線 mock，所以 LoreReader 用自己的規則（`lr-core` 的 `terms`，不呼叫模型）：

1. 括號裡的縮寫與它前面拼得出縮寫的全名（`Spectral Boundary Loss (SBL)`），之後再出現的縮寫也算；
2. 兩個以上大寫字母的詞（`BERT`、`ResNet`、`U-Net`）；
3. 大寫開頭的多字詞組（中間可夾 of、and、for…）；整行都是大寫開頭的標題行不算；
4. 有連字號、而且其中一段不是常用字（或很長）的詞（`cross-entropy`）；
5. 5 個字母以上、不在常用英文字表（約 2,600 字，處理複數、-ed、-ing、-ly 等變化）裡的字。

References／Bibliography 之後的文字、網址、DOI、e-mail 不標。

**已知限制**：沒有詞性和語意，人名、地名（Hausdorff、Sobel、作者名）會被當成少見字；句首的大寫字用小寫判斷；常用字表很短，比較少見但普通的英文字也會標紅；英式／美式拼法不在字表裡的會標；PDF 文字層的連字、斷行連字號會把字拆開；數字加單位、希臘字母不處理。自己已經熟的字寫進 `private/LoreReader/known-words.txt`（一行一個或多個）就不會再標。

點紅色詞：「加入知識庫」或「翻譯」。點有卡片的詞：顯示卡片摘要與熟悉度，可在 KnowledgeBase 開啟。參考文獻不會自動加入知識庫；「遇到」也不會自動記錄（之後可做成選項）。

## 從知識庫開啟 PDF

`GET /open?sha=<sha256>&page=<n>`：書庫裡有這份 PDF 就開到那一頁；沒有就顯示「不在這台電腦的書庫裡」。KnowledgeBase 卡片的來源若是 `type = "pdf"`，在卡片頁右側「來源」變成這個連結（位址由 KnowledgeBase 的 `reader_url` 設定，預設 `http://127.0.0.1:7890`）。

## API（摘要）

| | |
|---|---|
| `GET /api/info` | 書庫、匯出、工作區位置與錯誤、KnowledgeBase 位址、是否有本機模型、是否是 Mac App |
| `GET /api/papers`、`POST /api/papers/import?name=` | 論文清單；匯入（內容是 PDF，或 JSON `{path}`） |
| `GET/PUT /api/papers/{sha}`、`GET /api/papers/{sha}/pdf` | 論文資料／改標題；PDF 本身 |
| `GET/POST /api/papers/{sha}/annotations`、`PUT/DELETE …/annotations/{id}` | 註記 |
| `GET/POST /api/papers/{sha}/quotes`、`DELETE …/quotes/{id}`、`GET /api/quotes` | 引用句（單篇／全部） |
| `GET/POST/DELETE /api/papers/{sha}/chat` | 對話（送出 `{text, attachments}` 會回覆並存檔） |
| `POST /api/compose` | `{pages: [{sha, page}], title?, op?}` → 新 PDF（書庫＋匯出） |
| `POST /api/translate` | `{text, context?}` |
| `POST /api/terms` | `{text}` → 已有卡片的詞（KnowledgeBase）＋紅色候選詞（UTF-16 位置） |
| `GET /api/kb/status`、`POST /api/kb/cards`、`POST /api/kb/draft`、`GET /api/kb/nodes/{id}` | KnowledgeBase（建立卡片只在明確操作時） |
| `POST /api/print`、`POST /api/print/ready?id=` | Mac App 的原生列印 |
| `GET /open?sha=&page=` | 從其他 App 開到某一頁 |

## 第三方元件

- [pdf.js](https://github.com/mozilla/pdf.js)（Mozilla，Apache-2.0）5.7.284，放在 `web/vendor/pdfjs/`（`LICENSE` 隨附）。標準字型：Foxit（BSD 風格授權，`standard_fonts/LICENSE_FOXIT`）與 Liberation（SIL OFL 1.1，`standard_fonts/LICENSE_LIBERATION`）。文字層的 CSS 取自 pdf.js 的 `pdf_viewer.css`。
- [lopdf](https://github.com/J-F-Liu/lopdf)（MIT）：頁面操作。

## 沒有在真的硬體上測過

- Mac App（Tauri）：這台 Linux 機器沒有 WebKitGTK，`apps/desktop` 無法編譯；原生列印對話框、實體印表機、7890 埠的處理都沒測。
- 真的本機模型與 Claude（測試用假的 OpenAI 相容伺服器；問 Claude 只測了「沒有可用模型」的錯誤）。
- 真實論文（只用合成 PDF）。

## 開發說明

本專案與 Claude（Anthropic）協作開發，程式碼在 claude.ai 與 Claude Code 中共同撰寫。
