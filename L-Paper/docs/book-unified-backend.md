# 寫書統一入口：後端進度（2026-11-17）

契約：`docs/book.md` §統一入口。程式在 `crates/server/src/book/`（`setup.rs` 開始面板、`cards.rs` 知識卡，其餘是原本的階段）。

## 各階段

| 階段 | 做了什麼 | 怎麼檢查 |
|---|---|---|
| 1 路由與參數 | `POST /api/book/setup`（沒有 body＝預設值；記憶體 1 小時；id 是隨機 32 位 hex）、`GET`／`PUT /api/book/setup/{id}`（`new`＝預設值；`file`／`folder` 多 `label`）、`GET /api/book/options`、`GET /api/book/cards?q=`（不是 `known` 的卡，依標題排序，最多 30）。`POST /api/book` 收 `Setup`，舊的 `{source}` 照收；沒有起點 400，起點不對（路徑不在、不是 PDF、sha 不對）422。`Params` 多 `sources`、`keywords`、`cards`、`format`、`make_cards`、`title`；舊工作的 `sha` 讀成一個 `lr` 起點。工作物件的 `options` 多 `format`、`make_cards`、`sources: [{kind, label}]`、`keywords`。協調者追加：`POST /api/shell/dialog {kind: "pdf"}` 可多選（`{paths, path}`；桌面版的 hook 改回傳多個路徑）；搜尋檔案 `purpose: "寫書"`、`context: {setup}` 把 PDF 加進那份 setup（家目錄底下、只收 PDF、不重複，回答檔名）。 | `tests/book_api.rs`（setup 的建立、存回、搜尋檔案加入、options、cards、400、工作物件不含路徑）；`book::setup::tests`（3 個） |
| 2 多份素材 | `source.json` 變成 `{materials, files, of, skipped, notes}`（舊的一份 Material 照讀）。`lr` 照舊（第二篇之後的圖 id 是 `p2fig1`…，全書唯一）；`file` 與資料夾裡的 PDF（往下 4 層、略過隱藏、最多 300 份）一份一份用 lore-docsrc 只讀文字，每份做完寫 `source.files.json`（以檔案 sha256 為 key），重做不再讀。是不是論文：L-Shelf 開著且認得這個檔案（`GET /api/files/<sha256>`）就照它的 `cloud_ok`，`false` 的檔案連打開都不打開；否則 lore-docsrc 讀得到標題和參考文獻才算。素材不存路徑（只有檔名 `label`）。`terms` 讀全部素材＋關鍵字＋選的卡（作者自己指定的排在最前，不會被截掉）；`plan` 沒有論文也能排、多篇給每篇標題與摘要（摘要最多 12 篇）；每篇起始論文是一筆文獻，多篇時依名詞分給用到它的章（每章最多 6 篇）；沒有一章用到的不列為文獻（名詞已經讀過，結果裡註明篇數），一個大資料夾不會塞滿一章的提示詞或文獻庫；授權只對有圖的論文查；`books.toml` 的 `papers` 是所有 L-Reader 論文。 | `files_folders_and_keywords_start_a_book`、`a_keyword_only_book_plans_without_a_paper`、`an_older_job_reads_as_one_paper`，原本兩個整體測試照過 |
| 3 追一層 | 找資料之後，對找到的文獻（web 與 L-Reader 的，不含起始論文）保留的摘要與作者引用句跑同一套名詞抽取、對知識庫；不熟、書裡沒教、還沒出現過的掛在那篇文獻所屬的第一章，每章最多 8 個，存在 `research.json` 的 `found`（`retry_imports` 不會弄丟）。微調視窗 `review.chapters[].found: [{term, state, from}]`。 | `found_papers_offer_one_more_layer_of_terms` |
| 4 格式 | `notes`／`review1`／`review2`。新模板 `private-example/PaperComposer/publishers/review-paper/`、`review-paper-2col/`（`template.toml` 的 `format`），`lorereview.sty` 和 `lorenotes.sty` 同一組指令，只用 BasicTeX 的套件；`\sidenote` 是腳註，章就是一般的 `\section`；A4。內建在 App（`pc_core::book::FORMATS`、`write_builtin_format`），工作區有同名資料夾時用工作區的。`POST /api/books` 與工作收 `format`，頁面大小只給 `notes`。章節提示詞依書的格式（`pc_core::book::format_of` 從 main.tex 讀）寫「review paper 的一節」。 | `pc-core book::tests::every_format_has_a_template_with_the_same_commands`、`books::tests::the_review_formats_compile_with_xelatex`（這台有 XeLaTeX，兩種都編譯過）；一頁樣張見交接 |
| 5 知識卡 | `make_cards` 時，每章的 JSON 回覆多 `cards: [{term, summary, body}]`（只問這章教、沒有卡的名詞；繁中一句摘要＋2–4 句說明，不准引原文）。章寫好後跟章一起存進 `write.state.json`，緊接著建這一章的卡（建之前再查一次標題／別名），每建一張就存；重做不重複。L-Brain 開著：`GET /api/spaces` 選成員的 personal space → `POST /api/nodes`（`initial_state: learning`、`via: papercomposer-book`、`sources: [{type: "book", ref: 書名}]`）→ `POST /api/recall/{id} {outcome: "unfamiliar"}`；沒開：kb-core `create_as` + `recall(Outcome::Unfamiliar)`，和 L-Brain 的「標為不熟」一樣，lore-learn 讀回來是 `Familiar::Unknown`。`review.cards`＝大綱裡沒有卡的名詞數；`POST …/review` 的 `make_cards`（含 `skip`）改工作的設定；`result.cards: [{id, title}]`。舊的 `{source}` body（沒看過這個選項的呼叫者）預設不建卡。 | `a_book_makes_unfamiliar_cards_for_what_it_taught`（磁碟、不熟、來源只有書名、重做不重複、review.cards）、`cards_go_through_lbrain_while_it_runs`（API 的請求內容） |
| 6 一本書一份符號 | 各篇的 `symbols.json` 依序併進書的 `notation.json`（仍走 `reader::register_into`）。同一個符號（lore-notation 的 `find_latex`，正規化比對）意思不同＝衝突：先登記的留著，後來的不登記；別的字母表示已登記的量＝要換；兩種都不登記，用到它們的方程式也不登記。章節提示詞帶規則與每一筆換字／衝突；衝突列在 `result.problems`（kind `notation`）。`notation.tex` 照舊只在是 lore-notation 產生的檔時重寫。 | `several_papers_share_one_registry_of_symbols` |

## 和契約不同的地方

- `GET /api/book/options` 的 `formats[].pages`：review 兩種是 `["a4"]`（版面固定 A4），只有 `notes` 是 `["b5","a4"]`。
- 起點有誤（路徑不在、不是 PDF、不認得的種類）回 422，不是 400；只有「完全沒有起點」是 400（舊的測試本來就期待 422）。
- 「意思相同」怎麼判斷 lore-notation 沒有：用 `meaning_en`（沒有就 `meaning`）去掉大小寫、標點和 the/a/an/of 後完全相同。只有 L-Reader 的論文有 `symbols.json`，檔案與資料夾的素材沒有符號表。
- `/api/book/cards` 的 `state`：`unknown`（不熟）或 `learning`（學習中）。

## 留給作者決定（目前用最保守的做法）

- 自己選的單一 `file` 也要通過「是論文」的判斷（不是論文就不讀，結果註明）。
- 資料夾往下 4 層、最多 300 份 PDF；隱藏資料夾不讀。
- L-Shelf 開著但不認得這個檔案：退回「讀得到標題和參考文獻」的規則。
- 符號「意思相同」的判斷（見上）。
- 卡片建在成員的 personal space（沒有就第一個成員所在的 space）；來源寫 `{type: "book", ref: 書名}`。
- review 兩種格式只有 A4。
- 追一層的名詞只掛在那篇文獻的第一個章節，每章最多 8 個。
- 舊的 `{source}` body 預設不建卡；新的 Setup 預設建卡（契約的預設）。
- 範例伺服器（有沒有 `--mock` 都一樣）永遠用資料夾裡的假 L-Brain、不問開著的 L-Brain／L-Shelf。

## 沒有檢查到的

- 在 `source` 階段中途按停止再繼續（測了做完後重做不再讀檔，沒有測中途中斷）。
- `POST …/review` 帶 `make_cards` 經 HTTP（只測了 job 層的 `review.make_cards`）。
- 真的開著的 L-Brain、L-Shelf（只用假的 HTTP 回答）。
- 系統的 PDF 多選對話框（桌面版只編譯過）。

## 假模型（不用模型走一次頁面）

```sh
cargo run -p pc-server --example isolated_server -- <空資料夾> 7993 --mock crates/server/tests/fixtures/book_mock.json
```

在 `L-Paper/` 下執行。範例伺服器沒有 L-Reader 的論文可選（開始面板的「論文」是空的）；要試檔案，把任何論文 PDF 放進 `<空資料夾>/home/` 底下再用。資料夾裡會有 `KnowledgeBase/`（假的 L-Brain，成員 `ellrin`：softmax 記得、cosine similarity 學習中），卡片建在這裡；不連網、不問任何開著的 app。開始面板只打關鍵字（例如 `temperature scaling`）→ 大綱兩章、一個建議 → 微調 → 兩章內容、四張不熟的卡。腳本按順序用掉，同一個伺服器要再跑一次請重開。`book::tests::the_book_mock_fixture_runs_a_keyword_only_job` 確認這份腳本能走完。
