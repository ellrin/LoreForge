# 寫書：給自己的 review（課程筆記）

作者 2026-11-13 決定的下一步。LoreReader 讀論文時，把**作者不懂的部分**寫成一份給自己的課程筆記；在 PaperComposer 的「寫書」打開、改寫、匯出 PDF。論文報告的「知識背景文件」（docs/paper-deck.md「以後」）是同一件事。

## 作者的要求（2026-11-13）

- **格式**：課程講義筆記的樣子（參考 Stanford CS224n 的 lecture notes 的編排：課程式標題、關鍵詞、編號章節、定義與重點框、邊註）。模板是 LoreForge 自己寫的，不複製外部檔案。
- **從最基本開始**：每一章從**作者還不懂的名詞**講起；不懂的一定要解釋。懂不懂只看知識庫：`known`（記得）＝懂，不寫；其他（沒有卡、要學、`learning` 學習中）＝不懂，一定解釋（`learning` 可以短一點，但不能略過）。
- **語言**：英文為主，中文輔助（關鍵處中文說明、名詞第一次出現附中文）。
- **語氣**：教科書口吻，由淺入深，先直覺再式子。
- **有根據**：每個說法要有 reference（`\cite{…}`）；沒有來源的說法不寫。
- **上網找資料**：搜尋相關論文（OpenAlex / arXiv / Semantic Scholar 等，只取開放取用的全文）；用到的開放取用論文**下載並加進 LoreReader 書庫**。
- **一本書**：從一篇論文開始；之後讀別篇時可以「加進這本書」，已寫過的主題不重寫、只補新的章節。

## 流程（一個背景工作，`lore-jobs`）

入口：LoreReader 上方列「寫 review」（和「做成簡報」同一種卡片；PaperComposer 沒開就打開它再等）→ 選「新書」或「加進…」、篇幅、語言比例 → 先看大綱／直接產生。PaperComposer 首頁「寫書」列出書、可以從那裡開始或繼續。

| 階段 | 做什麼 |
|---|---|
| `source` | 論文素材（lore-docsrc 讀 LoreReader 的 PDF，唯讀）；從磁碟唯讀讀 LoreReader 的 `reading.json`（懂了的段落）與 `chat.json`（問過的問題），像 SlideComposer 的 `reader.rs`。LoreReader 開著時送來的紅字名詞只是補充 |
| `terms` | 工作自己從論文素材算名詞（和 SlideComposer 論文報告的 `terms` 同一套：`kbterms`，抽到 commons 共用），對知識庫；`known` 以外的全部進下一步。所以從 PaperComposer 開始、或結束後繼續的工作也有名詞清單 |
| `plan` | 模型把要講的名詞整理成主題與章節，依先備關係排序（知識庫 `prerequisite` 邊優先），每章：要講的名詞、深度、要找的資料 |
| `research` | 知識卡、LoreReader 讀過的論文與引用句、上網搜尋開放取用論文 → 下載、加進 LoreReader 書庫、做成 BibTeX |
| `review` | 微調視窗（等作者）：章節勾選與順序、每章深度、語言比例、篇幅、建議（和論文報告同一套介面） |
| `write` | 逐章寫 LaTeX（模板、`\cite`、lore-notation 巨集與 `\ref{}`，`notation.tex` 不動） |
| `check` | lore-notation 檢查（這本書自己的 `notation.json`：由論文的 `symbols.json` 與全域登記建立，和論文報告一樣；`notation.tex` 不動）＋每個 `\cite` 都有 BibTeX 條目＋編譯 |

工作的檢查點在快取層；書本身（LaTeX 資料夾）是文件，存在作者選的地方（第一次問，之後記住預設位置），`paper.toml` 記 `kind = "book"`。書的清單記在 PaperComposer 的設定資料夾 `books.toml`（`[[book]] path, title, papers = [sha…]`，像 `opened.toml`），首頁「寫書」據此列出。

**模板先看樣張**：模板做好後先編譯一頁樣張（標題頁、關鍵詞、定義框、邊註、名詞第一次出現的中英對照、一個 `\cite`）給作者看過，再寫章節。

## 上網找論文（`commons/crates/lore-research`）

- 來源：OpenAlex、arXiv、Semantic Scholar；只取開放取用全文（`is_oa` / `openAccessPdf` / arXiv）。
- 不自動帶作者的 e-mail（OpenAlex 的 `mailto`）：預設用一般的 client 名稱；要不要填由作者在設定裡決定。
- 下載到快取層的暫存資料夾，再經 LoreReader 的 `POST /api/papers/import {path}` 加進書庫。LoreReader 沒開：工作記下 `pending_imports`，頁面打開時用 `LoreLinks.ensure("lr")` 打開 LoreReader 補做，不會因為沒開而失敗。
- 沒網路：這一步只用知識卡與 LoreReader 的論文，工作照常完成並註明。
- 每筆資料都產生 BibTeX 條目（DOI / arXiv id 為準）。
- 測試不連網：OpenAlex / arXiv / S2 的回應都用假資料。

## API（PaperComposer 伺服器）

`/api/book*` 與 `/api/info` 回應 127.0.0.1 / localhost 的跨來源請求（CORS），和 SlideComposer 的 `/api/todeck*` 一樣，LoreReader 的按鈕才叫得到。錯誤：HTTP 狀態碼 + `{ "error": "一行中文" }`。工作物件的形狀與 docs/paper-deck.md §工作物件相同（`lore-jobs`），`review` 換成這條流程的內容。

| Method | Path | Body | 回傳 |
|---|---|---|---|
| GET | `/api/info` | — | 至少 `{ app: "pc", books: [{id, title, path, papers}] }` |
| GET | `/api/books` | — | 書的清單（`books.toml`） |
| POST | `/api/book` | `{ source: {kind: "lr", sha}, book: "new" \| "<id>", length: "short"\|"standard"\|"long", lang: "en-zh"\|"en"\|"zh", mode: "review"\|"direct", extras?: {terms: [{term, n}], stuck: [段落 id], questions: [text]}, folder?: path }` | `{ job: "<id>", …工作物件 }` |
| GET | `/api/book/jobs/{id}` | `?after=<seq>` 長輪詢 | 工作物件 |
| POST | `/api/book/jobs/{id}/review` | 作者在微調視窗的決定 | 工作物件 |
| POST | `/api/book/jobs/{id}/params` | `{length?, lang?}` | 從 `plan` 重做 |
| POST | `/api/book/jobs/{id}/stop`、`/resume` | — | 工作物件 |
| GET | `/api/book/jobs/{id}/imports` | — | `pending_imports`（頁面補做 LoreReader 匯入用） |

頁面：PaperComposer 用 `?book-job=<id>` 打開工作畫面（LoreLinks `lore-pc://bookjob/<id>`）；完成後打開那本書。

## 依賴

- `commons/crates/lore-jobs`：共用的工作執行器（2026-11-13 已從 SlideComposer 抽出）。
- PaperComposer 文件類型（設計文件 §6）：先只做 `book`（`paper.toml` `kind`，沒有＝`paper`）。
- 上網找論文：共用 crate（搜尋、開放取用判斷、下載、BibTeX），LoreReader 書庫匯入走 LoreReader 的 `POST /api/papers/import`。
- 作者的私人資料（範例、檢索）放在 private，經 lore-home；不放 `.claude`、skill 或公開 repo。
