# LoreReader

本機運作的 PDF 論文閱讀器。目標是用最短時間弄清楚一篇論文裡自己需要掌握什麼：沒學過的術語標成紅色、已在知識庫的術語顯示熟悉度；讀的時候存「引用句」（之後在 PaperComposer 引用）、加註記、翻譯、問 Claude。知識庫只放真正消化過的東西，所以**只有按「加入知識庫」才會建立卡片**，不會自動寫入。

> 原型 v0。已完成：書庫（repo 外、sha256 命名）、pdf.js 閱讀（文字層、縮放、搜尋、延遲繪製、縮圖）、不破壞原檔的頁面操作（刪除、複製、拖曳排序、從其他 PDF 插入、合併）、列印、螢光標記與註記、選取文字右鍵選單（複製、翻譯、問 Claude、存成引用句、加入知識庫）、知識標示（紅色＝沒有卡片）、從知識庫卡片連回 PDF 那一頁。
> 閱讀輔助（第二階段）：名詞整理（整篇詞表對知識庫、模型一次分類、忽略清單）、符號與方程式表（不靠模型）、讀懂段落（背景分析、卡住的標籤、懂了的進度）與問 Claude 的段落脈絡、每個工作可選模型。
> 做成簡報：上方列的投影片鈕把這篇交給 SlideComposer 的「論文報告」（長度、語言、底版，先看大綱或直接產生）；SlideComposer 沒開會先打開它。
> 不在範圍：編輯 PDF 內容、投影片精靈（在 SlideComposer）、把註記寫進 PDF 檔（之後可做）。

## 執行

需要 [Rust](https://rustup.rs)（1.85 以上）。

```sh
cargo run -p lr-server            # http://127.0.0.1:7890
cargo test                        # 全部離線（合成 PDF、假的 KnowledgeBase 與本機模型）
cargo run -p lr-core --example synth_pdf -- demo /tmp/demo.pdf   # 示範用的合成論文（3 頁）
cargo run -p lr-server --example isolated_server -- <空資料夾> [埠] [KB 位址]   # 不需要模型、鑰匙圈或 claude CLI，只用來試介面
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

沒寫 `[engine]` 時，本機模型用所有 LoreForge App 共用的選擇（`private/shared/engine.toml` 的 `[engine] model`，每次呼叫重讀），跑在整台電腦共用的一個 llama-server 上（誰需要誰開、最後一個用完的關，見 `docs/local-models.md`），同一個本機模型不必在每個 App 各設一次。

私人工作區的位置可用 `LOREREADER_WORKSPACE` 改到別處（測試、示範用）。

## 資料格式

每個檔案都有 `schema` 版本；不認得的欄位會保留。

| 檔案 | 內容 |
|---|---|
| `paper.json` | `schema: "lorereader.paper/1"`、`sha256`、`title`（PDF 的 Title，沒有就用檔名）、`original_name`、`doi`（前兩頁找到的第一個 DOI，沒有為 `null`）、`pages`、`added_at`、`derived_from`（頁面操作產生的新 PDF：`{op, from: [sha256…]}`）、`original_path`（從 Finder 或路徑開啟時原檔的位置，存檔對話框預設在它旁邊）、`source_sha`（這份 PDF 是 LoreReader 存出的「含筆記」檔，對應原本那篇的 sha256） |
| `annotations.json` | `schema: "lorereader.annotations/1"`，`annotations: [{id, page, kind: "highlight" \| "note" \| "text", color: yellow\|green\|blue\|pink, quote: {exact, prefix, suffix}, rects: [[x0,y0,x1,y1]…], note, created_at, updated_at}]`。`rects` 是相對於頁面的比例（左上為原點），所以任何縮放都對得上；`quote` 是原文與前後文，之後換渲染方式也能重新定位。`kind: "text"` 是放在頁面任意位置的文字框：一個 rect 是框，`note` 是文字，`quote` 空白 |
| `quotes.json` | 引用句。`schema: "lorereader.quotes/1"`，`quotes: [{id, text, page, note, created_at}]`。`note` 是一行備註（可空白） |
| `export.json` | 上一次「另存新檔（含筆記）」：`schema: "lorereader.export/1"`、`hash`（當時註記的 sha256）、`path`（存到哪裡；瀏覽器下載為 `null`）、`at`、`declined`（關閉時按「不儲存」的那一版註記） |
| `chat.json` | 問 Claude 的對話。`schema: "lorereader.chat/1"`，`messages: [{role: user\|assistant, text, attachments: [{text, page, context}], at, engine, paragraph?, chip?: {kind: term\|symbol\|equation, id, label}, context?}]`（`context` 是附給模型的段落脈絡，重播歷史時用） |
| `symbols.json` | 符號表。`schema: "lorereader.symbols/1"`，`sha256`，`generated_at`，`entries`：lore-notation 登錄的欄位（`id`、`kind: "symbol"`、`latex`、`meaning`、`meaning_en`、`defined_at`、`tags`…）加上 `scope`（論文 sha256）、`grade`（`certain` ✅／`defined` 🟢／`inferred` 🟡／`conflict` 🔴／`unresolved` ⚪）、`resolved_by`（`program`／`local`／`claude`／`author`）、`evidence: [{quote, page, by, at?, meaning?}]`、`occurrences: [{page, s, e, bbox, latex, eq?}]`、`first: {page, sentence}`、`equations: [id]`、`note`；`equations`：lore-notation 的 `equation` 條目（`id`、`label: "(3)"`、`latex`、`uses`）加上 `page`、`number`、`bbox`、`s`、`e`。`bbox` 是頁面比例、左上為原點 |
| `private/shared/my-vocab.json` | 我的詞彙（各 App 共用）。`schema: "lore.my-vocab/1"`，`sources: [sha256 或 "import:<名稱>"]`，`entries: [{key, text, sources}]` |
| `paragraphs.json` | 段落分析的快取。`schema: "lorereader.paragraphs/1"`，`analyses: {<段落 id>: {role, equations: [{ref, inputs, outputs, target, effect}], motive, results: [{metric, comparison, values, difference, test, significance}], engine, at}}`。段落 id 是段落文字的雜湊，重新開檔不變；段落文字本身不存 |
| `reading.json` | 「懂了」。`schema: "lorereader.reading/1"`，`understood: [段落 id]` |
| `nouns.json` | 名詞分類的快取。`schema: "lorereader.nouns/1"`，`categories: {<詞的正規化 key>: 分類}`，`asked: [已經送給模型的 key]`，`engine`，`classified_at`。名詞清單本身每次開檔重算（全文不寫進工作區） |

工作區根目錄另有 `ignore-words.txt`（忽略清單，一行一個字或詞組，`#` 之後是註解）與 `models.json`（`{schema: "lorereader.models/1", choices: {<任務>: auto\|local\|haiku\|sonnet\|opus}}`）。

`GET /api/quotes` 回傳所有論文的引用句（每筆加上 `sha256`、`paper_title`、`doi`）。PaperComposer 的「讀過的」與 SlideComposer 的「論文」面板不經過這個 API，它們直接唯讀讀取書庫與論文資料夾（兩邊的 `crates/server/src/reader.rs`）。

之後的「論文符號／方程式表」也會放在 `papers/<sha256>/`（與 lore-notation 的登錄欄位相同，加上論文範圍、等級與誰確認的），目前的檔名不會和它衝突。

## 閱讀

- 外框是所有 LoreForge App 共用的外殼（`commons/ui` 的 icons.js、base.css、shell.js，`docs/shell.md`）：上方列＝品牌（回首頁）· 文件選單（最近的論文、加入 PDF…、合併 PDF…、改標題…、另存新檔（含筆記）…、首頁）· 狀態標籤 · 標示名詞 · 搜尋 · 做成簡報 · 問這篇（AI）… 右邊 對照表 · 匯出▾（另存新檔（含筆記）、列印）· 齒輪。
- 左邊縮圖、中間頁面、右邊編號區塊（01 名詞、02 註記 預設展開；03 符號、04 段落、05 引用句、06 問這篇 預設收起，上方列的「問這篇」打開 06）；≤1100px 縮圖與面板變成抽屜。
- 齒輪：一般（你的領域、書庫與匯出資料夾、KnowledgeBase 網址）、外觀（配色，與其他 App 共用）、模型（共用的連線與走向，`/api/engines`）、快捷鍵。
- pdf.js 文字層可以選取文字；下方狀態列：頁碼跳頁、縮小／實際大小／放大、符合寬度。⌘F 在頁面上方開搜尋列（Enter 下一個、⇧Enter 上一個、Esc 關閉）。只繪製視窗附近的頁面，縮圖捲到才畫。

## 頁面操作（不破壞原檔）

左邊縮圖欄上方的三顆按鈕：複製這一頁、刪除這一頁、從其他 PDF 插入頁面（選一份 PDF → 點選要的頁，依點選順序 → 指定插在第幾頁之後）；左邊縮圖可以拖曳排序。變更先暫存（狀態標籤顯示「頁面未儲存」，點它＝另存…），按縮圖欄的「另存…」才用 Rust（`lopdf`）產生**新的 PDF**：存進書庫，同時複製一份到匯出資料夾（同名就加 `-2`、`-3`，不覆蓋）。原本的 PDF 永遠不動。書庫頁的「合併 PDF…」依點選順序把幾份 PDF 接成一份新的。

新 PDF 不保留原檔的書籤（outline）、表單與具名目的地；頁面上的連結註解保留。

## 另存新檔（含筆記）

`private/` 可能被清掉，所以註記可以寫進一份**新的 PDF**（原檔永遠不動）：這就是 LoreReader 的「儲存」。從文件選單或匯出▾ 的「另存新檔（含筆記）…」、⌘S，或點狀態標籤：

- 註記改過、還沒存進 PDF 時，狀態標籤顯示「未儲存」；存過之後「已儲存」。
- 第一次：Mac App 開系統的存檔對話框，預設在原檔旁邊（不知道原檔在哪就是「文件」），檔名 `<原檔名> (筆記).pdf`；瀏覽器直接下載。
- 之後：選單「儲存」（覆寫上一次存的那份，先寫暫存檔再改名）或「另存新檔…」。只會覆寫 LoreReader 自己存出的檔案，不會覆寫原檔、書庫或 git repo 裡的檔。
- 「頁面未儲存」（頁面操作）時要先在縮圖欄「另存…」頁面；筆記未儲存看狀態標籤。
- **關閉視窗或 ⌘Q**（Mac App）：有沒存進 PDF 的註記時，每篇問一次「儲存筆記到新的 PDF？」：儲存…／不儲存／取消。存過的直接覆寫那份；「不儲存」之後同一版註記不再問；「取消」不關。

寫法（任何 PDF 閱讀器都看得到）：每個螢光是 `/Highlight` 註解（`/QuadPoints`、顏色、外觀串流 `/AP`，Preview 才畫得出來）；「註記」另外在旁邊加一個便利貼（`/Text`，內容是註記文字）；文字框是 `/FreeText`（`/Contents`、`/DA`、畫好框與文字的 `/AP`，旋轉頁也是正的；英數用 Helvetica，中文用不內嵌的 Adobe-CNS1 字型 `MSung-Light`，Preview／Acrobat／poppler 會換成系統的中文字型；顏色固定，不跟主題）。每個註解有 `/NM (lr:<id>)` 和整筆註記的 JSON，再存一次會先移除 `lr:` 開頭的註解（別的工具加的不動）。文件資訊裡的 `/LoreReaderSource` 是原本那篇的 sha256。位置換算用 CropBox（沒有就 MediaBox）與 `/Rotate`。

**再開啟這份檔案**（例如 `private/` 被清掉之後，從 Finder 打開）：註記依 id 合併回來（不重複），這份檔案記為「上一次存的」。原本那篇還在書庫 → 就是那篇（同一個 sha256，知識庫連結與引用句不變）；不在 → 去掉 LoreReader 註解後的 PDF 成為一篇新的論文，`source_sha` 指向原本那篇，`/open?sha=<原本的>` 也會開到它。在別的閱讀器改過的註記文字以 PDF 上看得到的為準。

## 列印

匯出▾ 的「列印」：

- **Mac App**：開一個列印視窗（`print.html`，每頁以約 216 dpi 繪製，紙張大小用 PDF 第一頁的尺寸），繪製完成後顯示 macOS 的系統列印對話框（WKWebView 的原生列印），可以選任何已安裝的印表機或存成 PDF。
- **瀏覽器**：開新分頁 `print.html?sha=…&auto=1`，繪製完成後呼叫 `window.print()`。

> 本機檢查只確認了列印頁的版面（headless Chromium 以列印模式輸出 3 頁、Letter 尺寸）。**實體印表機與 Mac App 的原生列印對話框都還沒在真的 Mac 上測過。**

## 選取文字的右鍵選單

- **複製**。
- **螢光標記**（黃、綠、藍、粉紅）與**加註記…**：存在 `annotations.json`，右邊「註記」分頁列出，點一下跳到那裡。
- **文字框**：在頁面任何地方（文字、空白、圖）點兩下，就在那裡開一個文字框直接打字；Enter 換行，⌘↵ 或點外面存，空的框不留（Esc 丟掉）。點一下編輯、拖曳移動、右下角調大小、選取時按 × 或 ⌫ 刪除，⌘Z 復原。點兩下不再反白；點三下只選那一行。
- **翻譯**：lore-llm 任務 `lr-translate`，先用本機模型（`local:current`），連不上才用 Claude（`claude:selected`），與 PaperComposer 的順序相同。結果在小視窗顯示，可以「加入知識庫」。
- **問 Claude**：把選取的文字、頁碼和所在段落附到右邊的對話（任務 `lr-chat` → `claude:selected`；送出旁的引擎標籤可以改）。對話存在這篇論文的 `chat.json`。
- **存成引用句…**：句子（可修剪）＋一行備註，存在 `quotes.json`；右邊「引用句」分頁列出。
- **加入知識庫…**：呼叫 KnowledgeBase 的 `POST /api/nodes`。標題預填選取的文字，摘要自己寫（可按「用模型起草摘要」呼叫 KnowledgeBase 的 `/api/draft`，論文標題與頁碼一併送出）。**初始狀態**可選「學習中」（預設：明天問、3 天後再問）或「已掌握」（已經熟的詞，直接記為「記得」，30 天後再確認）。來源記為 `{type: "pdf", ref: <sha256>, where: "p.<頁>", title: <論文標題>}`，內文附上所在的句子。

## 名詞整理與知識標示（A 線：要學的名詞）

打開論文後，頁面把**全文**（pdf.js 的文字、字型、位置）送給伺服器一次（`POST /api/papers/{sha}/read`，只放在記憶體，不寫進工作區），伺服器照作者的做法整理出整篇的名詞清單（`lr-core` 的 `nouns`）：

1. **詞表**：References／Bibliography 之後不算，取出每個詞與每次出現的位置（頁碼＋在該頁文字裡的位置）。投稿稿件左邊的**行號**（固定 x 的一欄、往下遞增的整數）先換成空白（位置不變），所以不會黏成 `the275`，搜尋、引用句、段落分析也看不到；投稿系統的字樣（`Click here to view linked References`、`--Manuscript Draft--`、`Editorial Manager`…）與每頁重複的頁首頁尾不算詞。
2. **先比對多字詞**：文中連續 2–4 個字的詞組拿去對 KnowledgeBase 卡片的標題與別名（`GET /api/nodes`，只讀），所以 `cross entropy`、`Dice coefficient`、`U-Net` 會整組對到；跨行的詞組也算。括號裡的縮寫和前面拼得出它的全名（`Spectral Boundary Loss (SBL)`）也算一組，之後的 `SBL` 算在同一組。剩下的拆成單字；有連字號的複合詞（`label-smoothing`）保持一組，除非每一段都是一般字。
3. **詞形還原**：大小寫、複數（含 `matrices`、`analyses` 這類不規則的）、連字號與空格、英式／美式拼法（`normalisation`、`colour`、`tumour`、`centre`、`labelled`、`haem-`、`oedema`…）、連字（ﬁ）、彎引號、**行尾斷字**（`segmen-` 換行 `tation`：全文別處出現過 `segmentation`，或下一段是小寫而且全文沒有 `segmen-tation` 這種寫法，就接回去）。
4. **扣掉**：功能詞（`function_words.txt`）、學術通用詞（`academic_words.txt`：method、propose、significant…）、日常英文（`common_words.txt`，約 2,600 字；但 `TERMLIKE` 裡在論文中是術語的字不扣，例如 regression、variance、attention、loss、boundary）、最常見的英文字（`english_common.txt`，SCOWL 20 級以內，約 11,000 個字形：hip、biology、institute…）、以及**忽略清單**。縮寫（兩個以上大寫字母）不扣；全大寫的一般字（`TAIWAN`、`LAKE`）不算縮寫。次常見的字（`english_familiar.txt`，SCOWL 35 級：fracture、pelvic、auxiliary…）照樣送去分類，但**分類之前不列出**。也不列：人名、單位與標題頁的字（大寫開頭、全文沒有小寫寫法、多半出現在每個字都大寫的行或 `et al.` 前面：作者、`I-Fang`、`Liao2`、學校與期刊名）、編號（`ABCD-D-24-01234R4`）、網址／e-mail／檔名裡的字、序數與容量（`43rd`、`24GB`）、黏著數字的一般字。字表來自 SCOWL（http://wordlist.aspell.net/ ，授權在 `english_words.LICENSE`，可自由使用與散布，須保留版權聲明）。
5. **對知識庫**：記得、學習中／久未複習、沒有卡片。
6. **沒有卡片的詞整批交給模型分類，一篇只呼叫一次**（任務 `lr-classify-terms`：先地端模型、沒有時用 Haiku；JSON schema）：方法、指標、資料集、統計、臨床／解剖、其他術語、**基礎科學詞**（這個領域的專家早就會的科學、數學、工程詞，例如 pixel、vector、variance）、一般學術詞、人名。提示裡附上**讀者的領域與程度**（齒輪 → 一般的「你的領域」，存在共用的 `private/shared/profile.toml` 的 `field`，其他 App 也看得到；不寫就用預設 "researcher in medical image analysis and deep learning"；舊的 `private/LoreReader/reading.toml` 會自動搬過去），以及讀者已經記得的術語（知識庫「記得」的卡片標題、「我的論文」裡的術語）當程度參考。結果存在 `papers/<sha256>/nouns.json`；分類或提示的版本、或領域改了，快取就作廢重來。**還沒分類的詞不標色**，清單上顯示「未分類」。
7. 右鍵（或清單裡的眼睛按鈕）「**忽略**」寫進 `private/LoreReader/ignore-words.txt`，清單與內文立刻重畫。舊的 `known-words.txt` 第一次啟動時會自動搬進 `ignore-words.txt`（兩個都有時接在後面），之後只讀新檔名。懂的專有名詞不要放忽略清單：加入知識庫並選「記得」。

一篇約 8,000 字的合成論文會整理成約 270 個詞（測試 `long_paper_reduces_to_a_few_hundred_units`）。

**右邊「名詞」分頁**：欄位是詞、分類、知識庫狀態、次數、第一次出現的頁碼；可以依每一欄排序，依分類、狀態篩選，也可以打字篩選。點一個詞：文中**所有出現的位置**都會亮起來並跳到第一個，再點一下（或 ↓）跳到下一個。每一列可以「加入知識庫…」（狀態「學習中／記得」）或「忽略」。上方「● 要學 · ● 學習中 · ● 記得」可以點來篩選；「要學」＝專業術語而且沒有卡片（分頁上的數字也是它）。

**內文顏色**（上方列的標靶按鈕「標示名詞」打開／關閉）也來自這份清單：

- **紅色（要學）**：模型判為方法／指標／資料集／統計／臨床解剖／其他術語、而且沒有卡片、也不在「我的論文」裡的詞。基礎科學詞、一般學術詞、人名與還沒分類的詞都不標紅；名詞清單把基礎科學詞收成最後一組。
- **淡琥珀色（學習中）**：有卡片，學習中、久未複習或還沒有學習紀錄。
- **細綠底線（記得）**：有卡片、已經記得；或出現在「我的論文」裡（清單上加註「用過」）。知識庫裡「學習中」的卡片仍然是淡琥珀色。

**我的論文**：書庫卡片右上角的星號把一篇論文標成自己的論文（存在 `paper.json` 的 `mine`）。標記時從全文（References 之前）建出自己的詞彙：所有詞的正規化形式、重複出現的 2–4 字詞組、對到知識庫標題或別名的詞組，存在共用的 `private/shared/my-vocab.json`（各 App 共用；每個詞記下來源：論文 sha256，或 `import:<名稱>` 的匯入清單；取消標記只重建那篇論文的部分，匯入的保留；舊的 `private/LoreReader/my-vocab.json` 第一次會自動併進去）。從自己論文的段落文字（不需要 PDF）建匯入清單：`cargo run -p lr-core --example my_vocab_import -- out.json <資料夾>…`（每個資料夾一個 `paragraphs.jsonl`）。這些詞算「記得」，也當名詞分類的程度參考；它和忽略清單是分開的。

點有顏色的詞：分類、次數、第一次出現的頁碼；可以看所有出現位置、翻譯、「忽略」、加入知識庫；有卡片的顯示卡片摘要，可在 KnowledgeBase 開啟。參考文獻不會自動加入知識庫；「遇到」也不會記錄（只讀卡片清單，不呼叫 `/api/scan`）。

**已知限制**：標題頁與作者行以外的人名、地名（內文、參考文獻前的致謝）要靠模型分類才不會標紅；日常英文字表裡、又不在 `TERMLIKE` 的字（例如 network、image、model）不會列出，若某個領域需要，請告訴 Claude 加進 `TERMLIKE`；希臘字母與單一字母是變數，屬於「符號」，不在名詞清單；數字加單位不處理。

## 符號與方程式（第一步：不靠模型）

開檔時用同一份全文（每個 pdf.js 文字項目的字型名稱、字級、基線位置）找出數學，不呼叫任何模型（`lr-core` 的 `symbols`）：

- **行內數學**：數學字型（Computer／Latin Modern 的 CMMI、CMSY、CMEX、MSBM…、STIX／STIX Two Math、Cambria Math、XITS、TeX Gyre／New CM Math、Euler）、Unicode 數學字母與希臘字母，或正體內文裡單獨一個斜體（粗體）字母，都算數學。比前一個數學字小、而且升高或降低的是上下標，更小的再巢狀（`\mathbb{P}_{\theta_{t}}`、`\mathbb{P}_{\theta^{\prime}}`）；疊在字母上方的 ˆ ˜ ¯ ˙ ¨ 依位置變成 `\hat{}` 等；數學之間的 `= ( ) , |` 與 `log`、`max` 這類字接起來。
- **字形 → LaTeX**：內建表 `crates/core/src/glyphs.toml`（希臘字母、運算子、關係符號、重音）加上 Unicode 數學字母（𝐱 → `\mathbf{x}`、𝒙 → `\boldsymbol{x}`、𝓛 → `\mathcal{L}`…）；沒有 ToUnicode 的 TeX 字型用字型編碼對照（CMMI 的 OML、CMSY 的 OMS、CMR 的 OT1、CMEX 的大型運算子，例如舊版 pdfTeX 把 `\epsilon` 輸出成字碼 15）。**自己的對照**：`private/shared/glyphs.toml`（各 App 共用）與 `private/LoreReader/glyphs.toml`（「符號」分頁的齒輪按鈕可以直接編輯；存檔後按「重新偵測」），只寫要蓋過的條目。
- **獨立方程式**：大部分是數學、在欄內大致置中、右邊有 `(n)` 的一行；上下相鄰的置中數學行（分數、多行對齊）併成同一條；編號單獨在兩行中間也認得。記錄頁碼、範圍（bbox）與編號，LaTeX 盡力而為。
- **符號**：每段數學的 LaTeX 用 lore-notation 自己的解析器讀出符號，所以 key 與 lore-notation 一致（`\bm{x}_i` → `\boldsymbol{x}`，索引字母 i、j、k… 不算）。ℝ、𝔼 這類慣用符號標「慣用」、排在最後。
- **定義句**：規則比對「where ⟦x⟧ is/denotes/represents …」「⟦x⟧ denotes …」「let ⟦x⟧ be …」「we denote … by ⟦x⟧」（「is/are」後面要接 the/a/an）。**只有附上原文那一句與頁碼時才給 🟢**。同一篇裡同一個符號找到兩種不同的定義是 🔴 衝突（不是和作者自己的 notation 比）。沒找到是 ⚪。程式不給 ✅ 和 🟡（✅ 留給作者或原始檔，🟡 留給之後的模型推測）。

**右邊「符號」分頁**：每個符號用 KaTeX 顯示，附等級、意思、定義原句與頁碼、出現次數與頁碼（點一下跳過去並閃一下位置）、用到它的方程式；可依等級篩選；最後列出獨立方程式。每一筆都能編輯（LaTeX、中文意思、原文定義、等級、備註），作者改過的記為 `resolved_by: author`，重新偵測不會蓋掉；也可以「新增」偵測漏掉的符號。**滑鼠停在內文的變數上**會出現小視窗，顯示這段數學裡每個符號的定義。

**準確度**（`tests/math_ground_truth.rs`）：三篇合成 LaTeX 論文（`crates/core/tests/math/paperN.tex`，pdflatex 編譯，pdf.js 項目存成 JSON；PDF 不留），以 lore-notation 掃描原始檔得到的符號為標準答案：精確率 1.00、召回率 0.98（82／84；漏掉 `\Delta_n` 與 `\mathbf{0}`）；有編號的方程式 7／7。`cargo run -p lr-core --example symbols_json -- 頁面.json --tex 原始檔.tex` 可以拿自己論文的 LaTeX 原始檔算同樣的數字。

**已知限制**：分數 `\frac{a}{b}` 在行內常被讀成上下標；大型運算子上下的極限（`\sum_{i=1}^{N}`）方向可能顛倒；括號後面的下標（`(\ldots)_{t}`）接不上；寬的 `\bar`、`\widehat` 若是畫線而不是字形就抓不到；同一個字母在 CM 字型裡 `\phi` 與 `\varphi` 分不出來（都當 `\phi`）；圖說等小字級裡的上下標偶爾漏掉；無編號的獨立方程式會偵測得比實際多。

## 讀懂段落（B 線）與「問 Claude」的分工

開檔後把全文切成**段落**（分析用 `lr-paragraph`：先地端模型，沒有時用 Haiku）（pdf.js 的行：粗體或較大、較短、常有編號的行是章節標題；行距變大、首行縮排、句子結束的短行開始新段；獨立方程式留在所屬的段落裡、讀成［式 (n)］，方程式後面的「where …」接在同一段；沒結束的段落接到下一頁），然後在背景依序跑 `lr-paragraph`：**方法段與結果段先跑**，跑完一段就顯示一段，結果存在 `papers/<sha256>/paragraphs.json`，下次開檔不再跑。**不提供整段的中文摘要或翻譯**，只標出卡住的地方。

「段落」分頁，每一段：

- **角色**：背景、缺口、方法步驟、結果、討論、其他。
- **卡住的地方（標籤）**：這段裡 A 線的紅字（沒有卡片的術語）、符號表裡的符號（附等級）與獨立方程式。每個標籤有三個動作：**看定義**（知識庫卡片或符號表）、**問 Claude**、**加入知識庫**。
- **方法段**：每條方程式的「翻譯」（輸入、輸出、作用目標、作用結果）與這一步的動機（原文有寫才有）。
- **結果段**：指標、比較對象、數值、差距、檢定、顯著性，原文沒有的格子留空。
- **「這段我卡住了」**（整段和所有標籤一起問）與 **「懂了」**：懂了存在 `papers/<sha256>/reading.json`，上方顯示進度（例如 20/32），「下一段還沒懂的」跳到下一段；這和知識庫的「記得」無關。

**問 Claude 自動附上**：論文標題、章節、段落原文（含頁碼）、卡住的項目和它在本篇的已知定義（符號的意思、等級與原文定義句；術語的分類與知識庫狀態），以及**作者在這段裡的知識庫狀態**（已掌握、學習中、沒有卡片），讓 Claude 用作者懂的詞解釋不懂的。預設問法「只解釋我卡住的點，不要翻譯整段」，可以改寫。

**回答下方**：

- **存進符號表…**：選這段的一個符號，填意思與等級（Claude 的回答當依據），記為 `resolved_by: claude`，回答存成 `evidence`；之後重新偵測不會蓋掉。
- **存成註記**：存進 `annotations.json`（`paragraph` 欄位記段落 id），在「註記」分頁看得到。
- **加入知識庫…**：可選學習中或記得（同右鍵選單的「加入知識庫…」）。

對話依段落歸檔（`chat.json` 的 `paragraph`、`chip`、`context`），「對話」分頁可以切換「全部」或「這段」，重讀時看得到當時卡在哪、怎麼解開。沒有模型時段落與標籤照樣列出，上方說明原因。

### 引擎標籤

名詞分類（01 名詞）、段落分析（04 段落）、問這篇（送出旁）、翻譯（譯文下方）各有共用外殼的引擎標籤：「自動」（跟齒輪 → 模型的類別走向）、雲、地；存成共用 `private/shared/engine.toml` 的 `[overrides]`（`PUT /api/engines`，lore-engines）。舊的 `private/LoreReader/models.json`（以前的模型選單：地端、Haiku、Sonnet、Opus，嚴格、不換）照讀，標籤顯示「雲 · 名稱」或「地」，改一次標籤就清掉。用不了時錯誤附上 lore-llm 的「改用本機／改用 Claude」，問這篇的錯誤框有這顆按鈕。選了某一個就只用那一個，用不了會直接說原因，不會偷偷改用別的。選擇存在 `private/LoreReader/models.json`。

## 從知識庫開啟 PDF

`GET /open?sha=<sha256>&page=<n>`：書庫裡有這份 PDF 就開到那一頁；沒有就顯示「不在這台電腦的書庫裡」。KnowledgeBase 卡片的來源若是 `type = "pdf"`，在卡片頁右側「來源」變成這個連結（位址由 KnowledgeBase 的 `reader_url` 設定，預設 `http://127.0.0.1:7890`）。

## API（摘要）

| | |
|---|---|
| `POST /api/kb/open` | KnowledgeBase 沒開時打開它（macOS `open -a KnowledgeBase`，只限本機的 KB；測試與 isolated 不會開）；頁面之後每幾秒檢查一次，連上就重畫名詞清單 |
| `GET /api/info` | 書庫、匯出、工作區位置與錯誤、KnowledgeBase 位址、是否有本機模型、是否是 Mac App |
| `GET /api/papers`、`POST /api/papers/import?name=` | 論文清單；匯入（內容是 PDF，或 JSON `{path}`） |
| `GET/PUT /api/papers/{sha}`、`GET /api/papers/{sha}/pdf` | 論文資料／改標題；PDF 本身 |
| `GET/POST /api/papers/{sha}/annotations`、`PUT/DELETE …/annotations/{id}` | 註記（PUT `{color?, note?, rects?}`） |
| `GET/POST /api/papers/{sha}/quotes`、`DELETE …/quotes/{id}`、`GET /api/quotes` | 引用句（單篇／全部） |
| `GET/POST/DELETE /api/papers/{sha}/chat` | 對話（送出 `{text, attachments, paragraph?, chip?: {kind, id}}` 會回覆並存檔；有 `paragraph` 時伺服器附上段落脈絡） |
| `POST /api/compose` | `{pages: [{sha, page}], title?, op?}` → 新 PDF（書庫＋匯出） |
| `POST /api/translate` | `{text, context?}` |
| `POST /api/papers/{sha}/read` | 頁面送來的全文（每頁的文字與 pdf.js 項目：字型名稱、字級、位置）→ 名詞清單、符號，以及找到的行號 `line_numbers: [[頁, 起, 迄]]`（頁面拿來排除在搜尋與選取之外） |
| `GET /api/papers/{sha}/nouns`、`POST …/nouns/classify` | 名詞清單（每個詞的 key、寫法、來源、知識庫狀態、分類、次數、出現位置 `[頁, 起, 迄]`）；`{again?}` → 一次 `lr-classify-terms` |
| `GET/POST /api/papers/{sha}/symbols`、`PUT …/symbols/{id}`、`POST …/symbols/rebuild` | 符號表；新增；修改 `{latex?, meaning?, meaning_en?, grade?, note?, by?, evidence?, page?}`（作者的修改優先）；重新偵測 |
| `GET /api/papers/{sha}/paragraphs`、`POST …/paragraphs/run`、`POST …/paragraphs/{id}/understood` | 段落（標籤、分析、懂了）與背景分析的進度；重跑 `{again?}`；`{on}` |
| `GET/PUT /api/glyphs` | 自己的字形 → LaTeX 對照（`private/LoreReader/glyphs.toml`） |
| `POST/DELETE /api/ignore` | `{term}` → 加進／移出忽略清單 |
| `GET/PUT /api/engines`、`POST /api/links/open` | 齒輪的模型頁與引擎標籤、開其他 Lore App（lore-engines，所有 App 共用） |
| `GET/PUT /api/models` | 舊的模型選單（引擎標籤改一次就清掉）：`{task, model: auto\|local\|cloud\|haiku\|sonnet\|opus}` |
| `POST /api/papers/{sha}/symbols/{id}/global` | 對照表的「加入全域」：這一列寫進共用的 `private/shared/notation.json`（全域已有同一個符號 → 409） |
| `GET /api/notation/global` | 全域對照表 |
| `GET/PUT /api/theme` | 共用配色（外觀） |
| `GET/PUT /api/reader` | 讀者的領域與程度 `{field}`（名詞分類用；存在 `private/shared/profile.toml`） |
| `POST /api/papers/{sha}/mine` | `{on, pages?}`：「我的論文」標記與重建 my-vocab |
| `GET /api/kb/status`、`POST /api/kb/cards`、`POST /api/kb/draft`、`GET /api/kb/nodes/{id}` | KnowledgeBase（建立卡片只在明確操作時） |
| `GET/POST /api/papers/{sha}/export` | 含筆記的 PDF：狀態（`dirty`、上一次存的檔）／存檔 `{path}`、`{last: true}`（覆寫上一次）、`{choose: true}`（Mac App 的存檔對話框） |
| `POST /api/papers/{sha}/export.pdf`、`POST …/export/decline`、`GET /api/dirty` | 下載（瀏覽器）／關閉時「不儲存」／還沒存進 PDF 的論文 |
| `POST /api/print`、`POST /api/print/ready?id=` | Mac App 的原生列印 |
| `GET /open?sha=&page=` | 從其他 App 開到某一頁 |

## 第三方元件

- [KaTeX](https://katex.org)（MIT）0.16.47，放在 `web/vendor/katex/`（與 PaperComposer 相同；`LICENSE` 隨附）：符號與方程式的顯示。
- [pdf.js](https://github.com/mozilla/pdf.js)（Mozilla，Apache-2.0）5.7.284，放在 `web/vendor/pdfjs/`（`LICENSE` 隨附）。標準字型：Foxit（BSD 風格授權，`standard_fonts/LICENSE_FOXIT`）與 Liberation（SIL OFL 1.1，`standard_fonts/LICENSE_LIBERATION`）。文字層的 CSS 取自 pdf.js 的 `pdf_viewer.css`。
- [lopdf](https://github.com/J-F-Liu/lopdf)（MIT）：頁面操作、把註記寫進 PDF。
- [tauri-plugin-dialog](https://github.com/tauri-apps/plugins-workspace)（MIT／Apache-2.0）：Mac App 的存檔與確認對話框。

## 沒有在真的硬體上測過

- Mac App（Tauri）：這台 Linux 機器沒有 WebKitGTK，`apps/desktop` 無法編譯；原生列印對話框、實體印表機、7890 埠的處理都沒測。
- 存檔對話框、關閉視窗／⌘Q 時的「儲存筆記到新的 PDF？」都只編譯過，沒有點過。Dock 的「結束」、登出不會問（走 `terminate:`）。在 Preview 改過、再存的含筆記 PDF 能不能讀回來也沒測（要看 Preview 是否保留 `/NM` 和 `/LoreReaderSource`）。
- 真的本機模型與 Claude（測試用假的 OpenAI 相容伺服器；問 Claude 只測了「沒有可用模型」的錯誤）。
- 真實論文（只用合成 PDF）。

## 開發說明

本專案與 Claude（Anthropic）協作開發，程式碼在 claude.ai 與 Claude Code 中共同撰寫。
