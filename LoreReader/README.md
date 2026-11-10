# LoreReader

本機運作的 PDF 論文閱讀器。目標是用最短時間弄清楚一篇論文裡自己需要掌握什麼：沒學過的術語標成紅色、已在知識庫的術語顯示熟悉度；讀的時候存「引用句」（之後在 PaperComposer 引用）、加註記、翻譯、問 Claude。知識庫只放真正消化過的東西，所以**只有按「加入知識庫」才會建立卡片**，不會自動寫入。

> 原型 v0。已完成：書庫（repo 外、sha256 命名）、pdf.js 閱讀（文字層、縮放、搜尋、延遲繪製、縮圖）、不破壞原檔的頁面操作（刪除、複製、拖曳排序、從其他 PDF 插入、合併）、列印、螢光標記與註記、選取文字右鍵選單（複製、翻譯、問 Claude、存成引用句、加入知識庫）、知識標示（紅色＝沒有卡片）、從知識庫卡片連回 PDF 那一頁。
> 閱讀輔助（第二階段）：名詞整理（整篇詞表對知識庫、模型一次分類、忽略清單）、每個工作可選模型。
> 不在範圍：編輯 PDF 內容、投影片精靈（之後在 SlideComposer）、把註記寫進 PDF 檔（之後可做）。

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
| `nouns.json` | 名詞分類的快取。`schema: "lorereader.nouns/1"`，`categories: {<詞的正規化 key>: 分類}`，`asked: [已經送給模型的 key]`，`engine`，`classified_at`。名詞清單本身每次開檔重算（全文不寫進工作區） |

工作區根目錄另有 `ignore-words.txt`（忽略清單，一行一個字或詞組，`#` 之後是註解）與 `models.json`（`{schema: "lorereader.models/1", choices: {<任務>: auto\|local\|haiku\|sonnet\|opus}}`）。

`GET /api/quotes` 回傳所有論文的引用句（每筆加上 `sha256`、`paper_title`、`doi`），給 PaperComposer 之後讀取；PaperComposer 那邊尚未實作。

之後的「論文符號／方程式表」也會放在 `papers/<sha256>/`（與 lore-notation 的登錄欄位相同，加上論文範圍、等級與誰確認的），目前的檔名不會和它衝突。

## 閱讀

- 左邊縮圖、中間頁面、右邊面板（名詞／註記／引用句／對話）；窄螢幕時縮圖與面板變成抽屜。深色／淺色與其他 App 相同（右上角切換）。
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

## 名詞整理與知識標示（A 線：要學的名詞）

打開論文後，頁面把**全文**（pdf.js 的文字、字型、位置）送給伺服器一次（`POST /api/papers/{sha}/read`，只放在記憶體，不寫進工作區），伺服器照作者的做法整理出整篇的名詞清單（`lr-core` 的 `nouns`）：

1. **詞表**：References／Bibliography 之後不算，取出每個詞與每次出現的位置（頁碼＋在該頁文字裡的位置）。
2. **先比對多字詞**：文中連續 2–4 個字的詞組拿去對 KnowledgeBase 卡片的標題與別名（`GET /api/nodes`，只讀），所以 `cross entropy`、`Dice coefficient`、`U-Net` 會整組對到；跨行的詞組也算。括號裡的縮寫和前面拼得出它的全名（`Spectral Boundary Loss (SBL)`）也算一組，之後的 `SBL` 算在同一組。剩下的拆成單字；有連字號的複合詞（`label-smoothing`）保持一組，除非每一段都是一般字。
3. **詞形還原**：大小寫、複數（含 `matrices`、`analyses` 這類不規則的）、連字號與空格、英式／美式拼法（`normalisation`、`colour`、`tumour`、`centre`、`labelled`、`haem-`、`oedema`…）、連字（ﬁ）、彎引號、**行尾斷字**（`segmen-` 換行 `tation`：全文別處出現過 `segmentation`，或下一段是小寫而且全文沒有 `segmen-tation` 這種寫法，就接回去）。
4. **扣掉**：功能詞（`function_words.txt`）、學術通用詞（`academic_words.txt`：method、propose、significant…）、日常英文（`common_words.txt`，約 2,600 字；但 `TERMLIKE` 裡在論文中是術語的字不扣，例如 regression、variance、attention、loss、boundary）、以及**忽略清單**。縮寫（兩個以上大寫字母）不扣。
5. **對知識庫**：記得、學習中／久未複習、沒有卡片。
6. **沒有卡片的詞整批交給模型分類，一篇只呼叫一次**（任務 `lr-classify-terms`，預設 `local:current`，用 JSON schema）：方法、指標、資料集、統計、臨床／解剖、一般學術詞、人名、其他術語。結果存在 `papers/<sha256>/nouns.json`，問過的詞不會再自動送出（「分類」按鈕可以再送一次還沒有分類的）。沒有模型時清單照樣能用，只是顯示「未分類」。
7. 右鍵（或清單裡的眼睛按鈕）「**這不是術語**」寫進 `private/LoreReader/ignore-words.txt`，清單與內文立刻重畫。舊的 `known-words.txt` 第一次啟動時會自動搬進 `ignore-words.txt`（兩個都有時接在後面），之後只讀新檔名。懂的專有名詞不要放忽略清單：加入知識庫並選「已掌握」。

一篇約 8,000 字的合成論文會整理成約 270 個詞（測試 `long_paper_reduces_to_a_few_hundred_units`）。

**右邊「名詞」分頁**：欄位是詞、分類、知識庫狀態、次數、第一次出現的頁碼；可以依每一欄排序，依分類、狀態篩選，也可以打字篩選。點一個詞：文中**所有出現的位置**都會亮起來並跳到第一個，再點一下（或 ↓）跳到下一個。每一列可以「加入知識庫」（初始狀態「學習中／已掌握」）或「這不是術語」。

**內文顏色**（工具列的標靶按鈕打開／關閉）也來自這份清單：

- **紅色**：專業術語、沒有卡片。分類是「一般學術詞」或「人名」的只列在清單、不標紅；還沒分類的用紅色虛線。
- **淡琥珀色**：有卡片，學習中、久未複習或還沒有學習紀錄。
- **細綠底線**：有卡片，已經記得。

點有顏色的詞：分類、次數、第一次出現的頁碼；可以看所有出現位置、翻譯、「這不是術語」、加入知識庫；有卡片的顯示卡片摘要，可在 KnowledgeBase 開啟。參考文獻不會自動加入知識庫；「遇到」也不會記錄（只讀卡片清單，不呼叫 `/api/scan`）。

**已知限制**：人名、地名要靠模型分類才不會標紅；日常英文字表裡、又不在 `TERMLIKE` 的字（例如 network、image、model）不會列出，若某個領域需要，請告訴 Claude 加進 `TERMLIKE`；希臘字母與單一字母是變數，屬於「符號」，不在名詞清單；數字加單位不處理。

### 模型選單

工具列的 ✦ 按鈕：名詞分類、段落分析、問 Claude、翻譯，各自選「自動」（照 `tasks.toml` 的預設順序）、地端、Haiku、Sonnet 或 Opus，做法同 PaperComposer 的模型選單（`complete_on`）。選了某一個就只用那一個，用不了會直接說原因，不會偷偷改用別的。選擇存在 `private/LoreReader/models.json`。

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
| `POST /api/papers/{sha}/read` | 頁面送來的全文（每頁的文字與 pdf.js 項目：字型名稱、字級、位置）→ 名詞清單 |
| `GET /api/papers/{sha}/nouns`、`POST …/nouns/classify` | 名詞清單（每個詞的 key、寫法、來源、知識庫狀態、分類、次數、出現位置 `[頁, 起, 迄]`）；`{again?}` → 一次 `lr-classify-terms` |
| `POST/DELETE /api/ignore` | `{term}` → 加進／移出忽略清單 |
| `GET/PUT /api/models` | 模型選單：`{task, model}` |
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
