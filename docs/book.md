# 寫書：給自己的 review（課程筆記）

作者 2026-11-13 決定的下一步。L-Reader 讀論文時，把**作者不懂的部分**寫成一份給自己的課程筆記；在 L-Paper 的「寫書」打開、改寫、匯出 PDF。論文報告的「知識背景文件」（docs/paper-deck.md「以後」）是同一件事。

## 作者的要求（2026-11-13）

- **格式**：課程講義筆記的樣子（參考 Stanford CS224n 的 lecture notes 的編排：課程式標題、關鍵詞、編號章節、定義與重點框、邊註）。模板是 LoreForge 自己寫的，不複製外部檔案。
- **從最基本開始**：每一章從**作者還不懂的名詞**講起；不懂的一定要解釋。懂不懂只看知識庫：`known`（記得）＝懂，不寫；其他（沒有卡、要學、`learning` 學習中）＝不懂，一定解釋（`learning` 可以短一點，但不能略過）。
- **語言**：英文為主，中文輔助（關鍵處中文說明、名詞第一次出現附中文）。
- **語氣**：教科書口吻，由淺入深，先直覺再式子。
- **有根據**：每個說法要有 reference（`\cite{…}`）；沒有來源的說法不寫。
- **上網找資料**：搜尋相關論文（OpenAlex / arXiv / Semantic Scholar 等，只取開放取用的全文）；用到的開放取用論文**下載並加進 L-Reader 書庫**。
- **一本書**：從一篇論文開始；之後讀別篇時可以「加進這本書」，已寫過的主題不重寫、只補新的章節。

## 流程（一個背景工作，`lore-jobs`）

入口：L-Reader 上方列「寫 review」（和「做成簡報」同一種卡片；L-Paper 沒開就打開它再等）→ 選「新書」或「加進…」、篇幅、語言比例 → 先看大綱／直接產生。L-Paper 首頁「寫書」列出書、可以從那裡開始或繼續。

| 階段 | 做什麼 |
|---|---|
| `source` | 論文素材（lore-docsrc 讀 L-Reader 的 PDF，唯讀）；從磁碟唯讀讀 L-Reader 的 `reading.json`（懂了的段落）與 `chat.json`（問過的問題），像 L-Slide 的 `reader.rs`。L-Reader 開著時送來的紅字名詞只是補充 |
| `terms` | 工作自己從論文素材算名詞（和 L-Slide 論文報告的 `terms` 同一套：`kbterms`，抽到 commons 共用），對知識庫；`known` 以外的全部進下一步。所以從 L-Paper 開始、或結束後繼續的工作也有名詞清單 |
| `plan` | 模型把要講的名詞整理成主題與章節，依先備關係排序（知識庫 `prerequisite` 邊優先），每章：要講的名詞、深度、要找的資料 |
| `research` | 知識卡、L-Reader 讀過的論文與引用句、上網搜尋開放取用論文 → 下載、加進 L-Reader 書庫、做成 BibTeX |
| `review` | 微調視窗（等作者）：章節勾選與順序、每章深度、語言比例、篇幅、建議（和論文報告同一套介面） |
| `write` | 逐章寫 LaTeX（模板、`\cite`、lore-notation 巨集與 `\ref{}`，`notation.tex` 不動） |
| `check` | lore-notation 檢查（這本書自己的 `notation.json`：由論文的 `symbols.json` 與全域登記建立，和論文報告一樣；`notation.tex` 不動）＋每個 `\cite` 都有 BibTeX 條目＋編譯 |

工作的檢查點在快取層；書本身（LaTeX 資料夾）是文件，存在作者選的地方（第一次問，之後記住預設位置），`paper.toml` 記 `kind = "book"`。書的清單記在 L-Paper 的設定資料夾 `books.toml`（`[[book]] path, title, papers = [sha…]`，像 `opened.toml`），首頁「寫書」據此列出。

**模板先看樣張**：模板做好後先編譯一頁樣張（標題頁、關鍵詞、定義框、邊註、名詞第一次出現的中英對照、一個 `\cite`）給作者看過，再寫章節。

## 上網找論文（`commons/crates/lore-research`）

- 來源：OpenAlex、arXiv、Semantic Scholar；只取開放取用全文（`is_oa` / `openAccessPdf` / arXiv）。
- 不自動帶作者的 e-mail（OpenAlex 的 `mailto`）：預設用一般的 client 名稱；要不要填由作者在設定裡決定。
- 下載到快取層的暫存資料夾，再經 L-Reader 的 `POST /api/papers/import {path}` 加進書庫。L-Reader 沒開：工作記下 `pending_imports`，頁面打開時用 `LoreLinks.ensure("lr")` 打開 L-Reader 補做，不會因為沒開而失敗。
- 沒網路：這一步只用知識卡與 L-Reader 的論文，工作照常完成並註明。
- 每筆資料都產生 BibTeX 條目（DOI / arXiv id 為準）。
- 測試不連網：OpenAlex / arXiv / S2 的回應都用假資料。

## API（L-Paper 伺服器）

`/api/book*` 與 `/api/info` 回應 127.0.0.1 / localhost 的跨來源請求（CORS），和 L-Slide 的 `/api/todeck*` 一樣，L-Reader 的按鈕才叫得到。錯誤：HTTP 狀態碼 + `{ "error": "一行中文" }`。工作物件的形狀與 docs/paper-deck.md §工作物件相同（`lore-jobs`），`review` 換成這條流程的內容。

| Method | Path | Body | 回傳 |
|---|---|---|---|
| GET | `/api/info` | — | 至少 `{ app: "pc", books: [{id, title, path, papers}] }` |
| GET | `/api/books` | — | `{ books: [{id, title, path, papers, modified, thumb}], folder }`（`books.toml`；`folder` 是新書預設的資料夾） |
| POST | `/api/books` | `{ title, folder }` | `{ id, path }`：從模板建立一本空的書（第一階段已有） |
| POST | `/api/book` | `{ source: {kind: "lr", sha}, book: "new" \| "<id>", length: "short"\|"standard"\|"long", lang: "en-zh"\|"en"\|"zh", mode: "review"\|"direct", extras?: {terms: [{term, n}], stuck: [段落 id], questions: [text]}, folder?: path, page?: "b5"\|"a4", engine?: "local"\|"subscription"\|"api" }` | `{ job: "<id>", …工作物件 }`（`folder` 沒給：上次新書的資料夾） |
| GET | `/api/book/jobs/{id}` | `?after=<seq>` 長輪詢 | 工作物件 |
| POST | `/api/book/jobs/{id}/review` | 作者在微調視窗的決定 | 工作物件 |
| POST | `/api/book/jobs/{id}/params` | `{length?, lang?, page?}` | 篇幅或語言變了：從 `plan` 重做；頁面大小只影響 `write` |
| POST | `/api/book/jobs/{id}/stop`、`/resume` | — | 工作物件 |
| GET | `/api/book/jobs/{id}/imports` | — | `{pending}`（頁面補做 L-Reader 匯入用） |
| POST | `/api/book/jobs/{id}/imports` | — | 頁面用 `LoreLinks.ensure("lr")` 打開 L-Reader 之後：伺服器再把等著的 PDF 交給它 → `{imported, pending, failed}` |
| GET | `/api/book/jobs/{id}/open` | — | 寫好的書在 `opened.toml` 的 id → `{id, path}`（頁面據此打開） |
| GET | `/api/book/jobs` | `?unfinished=1` | `{jobs}`（首頁「寫書」的進行中、「上次做到第 N 步，繼續？」） |
| DELETE | `/api/book/jobs/{id}` | — | 刪掉工作的快取資料夾 |

頁面：L-Paper 用 `?book-job=<id>` 打開工作畫面（LoreLinks `lore-pc://bookjob/<id>`）；完成後打開那本書。

## 第二階段（2026-11-13 完成）：寫書的工作

`L-Paper/crates/server/src/book/`（`pipeline.rs` 階段、`plan.rs`、`research.rs`、`write.rs`、`source.rs`、`model.rs`、`tools.rs`），頁面 `L-Paper/ui/bookjob.js`。

- **一樣的文件、一樣的路徑**：書就是 L-Paper 的文件，工作寫 LaTeX 只走論文本來的路：章節（`paper::add_section`、`render_section`）、圖與表（`floats.rs`：檔名、`\label`、圖說位置、`graphicx`；編輯器的 新增圖…／繪圖… 也改成呼叫同一個 `POST /api/papers/{name}/figure-block`；繪圖的圖是 lore-plot 的 `save_paper_figure`，留著 `.plot.json` 可以再用 繪圖… 修改）、表格（`textable` 的樣式與 `render`，和 新增表… 一樣）、文獻（進工作區的文獻庫 `refs/library.bib`，再由 `write_paper_bib` 產生書的 bib，和「文獻」一樣）、符號（`reader::register_into` 進書自己的 `notation.json`，和 L-Reader 面板的「插入」一樣）。寫好的書在編輯器裡和論文一樣改。
- **名詞**：`commons/crates/lore-learn`（從 L-Slide 的 `kbterms.rs` 抽出：找名詞、對知識庫、卡片的 `prerequisite` 連結；L-Slide 仍用它，行為不變）。`known` 以外都要講；`learning` 標成短一點。L-Reader 卡片送來的紅字名詞補進去。加進既有的書：書裡已經 `\term{…}` 過的名詞不再講。沒排進章節的名詞列在微調視窗。
- **大綱**：`pc-book-plan`（class create）；章節依知識庫的先備關係重排（`plan::order`）。`pc-book-suggest`：最多 5 個建議（改章名、深度、移動、不寫、合併、加一章），失敗就沒有。
- **找資料**：先查這篇論文的開放授權（OpenAlex 的 `license`；在 arXiv 上的論文以 arXiv 自己標的授權為準，因為 OpenAlex 把 arXiv 的版本一律標成 cc-by，arXiv 預設的授權不允許轉載）；書是作者自己讀書用的：所有 Creative Commons 授權（CC BY、BY-SA，也包括 BY-NC、BY-NC-SA、BY-NC-ND）、CC0、公眾領域的圖都可以放進書，**每張都引用來源並在圖說寫授權**（「Reproduced from \cite{…}, CC BY-NC 4.0.」，`write::reused_caption`）；授權不明、出版社授權、保留所有權利（含 arXiv 預設授權）的圖從不使用（2026-11-13 作者決定；寫論文仍用嚴格的規則 `figure_reuse_allowed_published`）。其他授權：有數字的圖改用 lore-plot 依論文的數字重畫（引用），示意圖用文字說明。流程圖等 L-Paper 有流程圖工具（和編輯器同一條路）再加。回答「太多請求」的來源，這個工作就不再問它（只重試一次、最多等 8 秒），不會卡在限流上。知識卡當脈絡（不能引用）。L-Reader 書庫裡標題含章節名詞的論文與作者的引用句。上網：每章最多 2 個查詢、取 2 篇開放取用論文，下載到快取、由伺服器交給 L-Reader；L-Reader 沒開就留在 `pending_imports`，頁面上「開啟 L-Reader」補做。沒網路照常完成並註明。
- **撰寫**：每章一次 `pc-book-chapter`（JSON：本文、圖、表）。只准引用這章的資料（不存在的 key 拿掉並列出）；圖用 `%%FIGURE:id` 位置與 `\ref{fig:id}`，換成真正的標籤；畫不出來的圖連同參照一起拿掉。每章跑對照表檢查（`notation::check`），手打的「Section 2」、不存在的 `\ref` 會請模型重寫一次，還有的列在結果。新符號登記為待確認。全書寫完後，沒有被引用的方程式標籤拿掉（連同只剩標籤的那一行）。`notation.tex` 只在它還是 lore-notation 產生的檔案時由 lore-notation 重寫。
- **檢查**：整本書的 lore-notation 檢查（`hardcoded_reference`、`ref_undefined`、`label_*`）、每個 `\cite` 都有條目、每個 `\includegraphics` 檔案都在、XeLaTeX 編譯（`export::build_pdf`，和匯出 PDF 一樣；沒有 TeX 就註明沒編譯）。
- **頁面**：`?book-job=<id>` 打開工作畫面（進度、微調、完成後「打開書」）；`?book-job=mock&at=run|review|done|interrupted` 不用模型看每個狀態。首頁「寫書」：進行中的工作、「從論文寫…」（L-Reader 的論文）、新書的頁面大小。
- **檢查點**：`<OS 快取>/PaperComposer/book/jobs/<id>/`（`PAPERCOMPOSER_CACHE` 可改），`write.state.json` 記已建立的章節與寫好的章，中斷後接著寫不重複。
- **書的版面（作者 2026-11-13 補充）**：`lorenotes.sty` 是書的版面：雙面、內外邊界對稱、邊註在外側（偶數頁在左、奇數頁在右）、左頁頁首書名、右頁頁首章名、頁碼在外側、標題頁單獨在第 1 頁（右頁）、摘要在標題頁背面、每章從右頁開始（前一頁空白不印頁首）。頁面大小 B5（預設）或 A4：`main.tex` 的 `\usepackage[b5]{lorenotes}`（`pc_core::book::page_of`／`set_page`；放在 main.tex 而不是 paper.toml，檔案自己就能在別處編譯）；新書表單、`POST /api/books {page}`、工作的 `page` 與微調視窗都可以選。

## 第一階段（2026-11-13 完成）：文件類型、書的清單、模板

已經有的：

- **文件類型**：`paper.toml` 的 `kind = "book"`（`pc_core::meta::PaperMeta::kind`，沒寫＝論文）。書沒有 Key value 列，也沒有期刊規則；其他（章節編輯、對照表、文獻、匯出）和論文相同。設計文件 §6 的 `kinds/` 沒做：目前只有 `book` 一種，差別只有上面兩項，等真的需要不同的檢查或大綱層級再做。
- **書的清單**：`books.toml`（L-Paper 的設定資料夾，`opened.toml` 旁邊；`[[book]] path, title, papers` 與 `last_folder`）。書也登記在 `opened.toml`，用那裡的 id 打開。`GET`／`POST /api/books`（`L-Paper/crates/server/src/books.rs`）。
- **首頁「寫書」**：書的清單（最近改過的在前）與「＋ 新書」（書名、資料夾：預設上次的；桌面版用系統對話框選）。建立時資料夾以書名命名、不覆蓋既有資料夾。
- **模板**：`private-example/PaperComposer/publishers/lecture-notes/`（`template.toml` 的 `kind = "book"`；`lorenotes.sty` 是自己寫的），也內建在 App（`pc_core::book`），工作區有自己的書模板時用工作區的。XeLaTeX（main.tex 第一行 `% !TEX program = xelatex`，匯出照這行選引擎），只用 BasicTeX 有的套件：沒有用 xeCJK、tcolorbox、titlesec（BasicTeX 沒有）；中文用 XeTeX 的字元類別自動切換字型，字型依序找這台電腦有的（Songti TC、PingFang TC 等 macOS 內建，Noto CJK、Fandol 為其他系統的備選）。指令：`\course` `\title` `\author` `\date` `\keywords`、`abstract`、`\term{英文}{中文}`、`\sidenote{…}`、`definition`／`keyidea`／`intuition`／`remark`、`natbib`（`\cite` → [1]）。
- **檢查**：新書通過 lore-notation 的檢查（`pc_core::book` 的測試）；有 XeLaTeX 的電腦上測試會實際編譯一本含名詞、邊註、各種框與中文的書。

## 依賴

- `commons/crates/lore-jobs`：共用的工作執行器（2026-11-13 已從 L-Slide 抽出）。
- L-Paper 文件類型（設計文件 §6）：先只做 `book`（`paper.toml` `kind`，沒有＝`paper`）。
- 上網找論文：共用 crate（搜尋、開放取用判斷、下載、BibTeX），L-Reader 書庫匯入走 L-Reader 的 `POST /api/papers/import`。
- 作者的私人資料（範例、檢索）放在 private，經 lore-home；不放 `.claude`、skill 或公開 repo。

## 統一入口（2026-11-17，作者決定）

寫書只有一條流程，也只有一個開始的地方：**開始面板**（`L-Paper/ui/booksetup.js`）。L-Paper 首頁「寫書」的「＋ 新書」、L-Reader 的「寫 review」都只是打開這個面板；L-Reader 會先把那篇論文和紅字名詞填好，面板上仍可增減。沒有第二個入口（原本的「從論文寫…」選單和 L-Reader 卡片上自己的選項都拿掉）。

作者的話：論文只是**起始關鍵字**的一種來源（論文裡不熟的名詞），用來找文獻；它在書裡和找到的文獻同等地位。建立一本書代表想學，所以不懂的名詞要**建立知識卡、標成不熟**。寫一本書很久，期間要能照常編輯別的論文。一本書裡一個符號只能有一個定義，引用的多篇論文各用各的符號，所以一律經過這本書的對照表（lore-notation）。

### 起點（可以混用，可以都沒有）

| 起點 | 內容 |
|---|---|
| `lr` | L-Reader 書庫的論文（sha） |
| `file` | 電腦裡的檔案（PDF） |
| `folder` | 一個資料夾：裡面的 PDF **全部**抽名詞，不讓作者勾，在背景一份一份做 |
| 關鍵字 | 自己打的名詞 |
| 知識卡 | 從 L-Brain 選的卡（不是「記得」的） |

都沒有：建立空白的書（和原本的「新書」一樣），不開工作。

### 流程的改動

| 階段 | 改動 |
|---|---|
| `source` | 每個起點一份素材。檔案與資料夾直接用 lore-docsrc 讀（只取文字，不裁圖），一次一份、每份做完就存檢查點（`counts.files`／`of`）。不是論文的檔案不用，結果裡註明幾份 |
| `terms` | 所有素材的名詞＋關鍵字＋選的卡，一起對知識庫 |
| `plan` | 沒有論文也能排（只有名詞）；多篇時給每篇的標題與摘要 |
| `research` | 一樣；**追一層**：找到的文獻（摘要與取得的文字）再抽一次名詞、對知識庫，不熟的掛在它所屬的章，預設不勾 |
| `review` | 章節下多一排「文獻裡的新名詞」可以勾；顯示會建立幾張知識卡，可以關掉 |
| `write` | 每寫完一章，這章教的名詞（沒有卡的）各建一張知識卡 |
| `check` | 一樣 |

- **格式**：`notes`（現在的講義，B5／A4）、`review1`（review paper 單欄）、`review2`（雙欄）。三種定義同一組指令（`\term`、`\sidenote`、`definition`…），只用 BasicTeX 有的套件；雙欄沒有邊註，`\sidenote` 變成腳註。模板先編譯一頁樣張給作者看。
- **知識卡**：只替寫出來的章節教過的名詞建；已有同名或同別名的卡不建。**每張卡各自上網查**（作者 2026-11-24：目的是補齊背景知識，知識建立一定要上網查；任務 `pc-book-card`，只有 Claude，沒有真的搜尋的回答不採用，查不到可靠說明就不建並列在結果）：中文一句摘要＋2–4 句說明（只照查到的內容、用自己的話，名詞寫成「中文（English）」），搜尋時帶書名、章名和這一章提到它的句子，讓查的是書裡的意思；來源是這本書和查到的網頁、`via = "papercomposer-book"`。開始面板預設開。建好後在 lore-learn 讀回來必須是**不熟**（`Outcome::Unfamiliar`），不是學習中。L-Brain 開著時走它的 API，沒開時用 kb-core 寫進成員的個人空間。建過的記在 `write.state.json`，中斷再繼續不重複。
- **符號**：這本書的 `notation.json` 是唯一的登記。多篇素材的 `symbols.json` 合併進去時，同一個符號兩種意思＝衝突：先登記的留著，後來的那篇在寫作時換成書裡的符號（提示詞帶著整份登記與「來源用別的字母時要換」），衝突列在結果的「待處理」。
- **隱私**：檔案與資料夾的路徑不進知識卡、LaTeX、bib、任何會同步的檔案，只留在工作的快取。資料夾裡不是論文的檔案（L-Shelf 開著時以它的 `cloud_ok` 為準；沒開時：lore-docsrc 讀得到標題和參考文獻的才算論文）一律不讀內容、不送給模型。
- **編輯別的論文**：工作在伺服器背景跑（本來就是）。L-Paper 上方列多一個進度小標（「寫書 3/8 章」），點了回到工作畫面。

### API 的改動

| Method | Path | Body | 回傳 |
|---|---|---|---|
| POST | `/api/book/setup` | `Setup`（可以只有一部分，也可以沒有 body） | `{ setup: "<id>" }`：先放著（記憶體，1 小時），給另一個 app 預填用；沒有 body＝預設值（面板自己先建一份，搜尋檔案才有地方加） |
| GET | `/api/book/setup/{id}` | — | 那份 `Setup`；`new` = 空的預設值。`file`／`folder` 起點多一個 `label`（檔名或資料夾名） |
| PUT | `/api/book/setup/{id}` | `Setup` | 存回面板上改過的設定（搜尋檔案加進來之前） |
| GET | `/api/book/options` | — | `{ books: [{id, title}], folder, formats: [{id, label, pages: ["b5","a4"]}], papers: [{sha, title, year}] }` |
| GET | `/api/book/cards?q=` | — | `{ cards: [{id, title, state}] }`：不是「記得」的卡，最多 30 |
| POST | `/api/book` | `Setup` | `{ job, …工作物件 }`（舊的 `source: {kind:"lr", sha}` 仍接受）。沒有任何起點：400 |
| POST | `/api/books` | `{ title, folder, page?, format? }` | 多了 `format` |
| POST | `/api/shell/dialog` | `{ kind: "pdf", start? }` | 系統的檔案對話框，可選多個 PDF → `{ paths: […], path: <第一個> }`；沒有桌面版：501 |
| POST | `/api/lore/picked` | （L-Echo 送來） | 搜尋檔案 `purpose: "寫書"`、`context: {setup}`：那個 PDF 加進該 setup 的 `sources`（`{kind: "file", path}`，不重複），回答檔名；面板再讀 `GET /api/book/setup/{id}` |

```jsonc
// Setup
{ "sources": [ { "kind": "lr", "sha": "…", "title": "…" }, { "kind": "file", "path": "/…/a.pdf" }, { "kind": "folder", "path": "/…/papers" } ],
  "keywords": ["contrastive learning"],
  "cards": ["<card id>"],
  "extras": { "terms": [{ "term": "…", "n": 3 }], "stuck": [], "questions": [] },   // L-Reader
  "book": "new",                 // or an opened.toml id (加進這本書)
  "title": "",                   // a new book's title; "" = the planner names it
  "folder": "/…/Books",
  "format": "notes",             // notes | review1 | review2
  "page": "b5",                  // notes only
  "length": "standard", "lang": "en-zh",
  "make_cards": true,
  "mode": "review" }             // review | direct
```

工作物件多的欄位：`options` 帶 `format`、`make_cards`、`sources`（每個 `{kind, label}`，`label` 是標題或檔名，不含路徑）、`keywords`；`source` 階段 `counts: {files, of, skipped}`；`review.chapters[].found: [{term, state, from}]`（追一層的新名詞，`from` = 那篇文獻的標題）與 `review.cards: <會建立幾張>`；`POST …/review` 的 body 多 `make_cards`；`result.cards: [{id, title}]`。

連結：`LoreLinks.open("pc", { booksetup: "<id>" })` → `lore-pc://booksetup/<id>` → `/?book-setup=<id>`。`?book-setup=mock` 不用伺服器看面板。

## 圖文書與先確認大綱（2026-11-24，作者的要求，待做）

作者的話（摘要）：建書時要先有大綱目錄讓我確定；先跟我做簡單的章節確認、互動式建立大綱，然後才生成。一開始要幫我「多」增加一些可能的關鍵字（例如論文裡有 XGBoost、random forest，我不熟），也要讓我自己新增。我要的書是**圖文書**：多一些算法解釋與圖示；找不到適合的圖、或流程直接說明就好的，用流程圖工具。語言可選，專有名詞有英文的要多標英文。找到的文獻裡的新名詞：找完資料後再停一次讓我勾（作者選 A）。

查到的現況：

- 名詞抽取漏掉單字的大小寫混合名詞與第一個字是一般英文的片語：合成句子「…compare with XGBoost and a random forest… XGBoost reached… the random forest…」只抽到 `support vector machine` 與 `AUC`，`XGBoost`、`random forest`、`gradient boosting`、`decision trees` 都沒有（lore-learn `find_terms`：片語要出現兩次且第一個字不是一般英文；單字只認縮寫）。
- 大綱的確認（`review`）在「找資料」之後；直接產生（`mode: direct`）不停。
- 書裡的圖只有：開放授權的論文圖（引用＋授權）、lore-plot 依數字重畫；示意與流程用文字。

要做（依序）：

1. **第一站：大綱與關鍵字**（`plan` 之後就停）。模型多提一些可能的關鍵字（論文裡出現但抽取漏掉的方法名、先備概念），每個都要指得出論文裡的句子或知識庫的先備連結，程式檢查；作者勾選、自己新增；大綱可以採用建議或打一句話重排。確定後才找資料與撰寫。面板的「直接產生」拿掉。
2. **第二站：文獻裡的新名詞**（`research` 之後再停一次，勾要不要加進書）。
3. **語言**：「中文」模式印成 **中文**（English）（`.sty` 的選項，不交換 `\term` 的參數：第一個參數是「已經講過」比對用的英文）；每章第一次出現都標英文、章名與大綱附英文；沒有通用中文譯名的直接用英文。
4. **圖文書**：大綱裡每章說明要放的圖：開放授權的論文圖、重畫的數據圖、流程圖（lore-flow，和 流程圖… 同一條路：`.flow.json`、`\ref`、符號進書的 `notation.json`），或空的圖位加建議。授權規則不放寬。
5. **時間軸與線性流程的版型**：見 docs/flow-figures.md §7。
