# 論文變簡報（paper → deck）

從 PaperComposer 的論文或 LoreReader 的論文產生一份可上台的論文報告簡報。工作在 SlideComposer 裡跑；PaperComposer 與 LoreReader 上方列的「做成簡報」（共用的 `commons/ui/makedeck.js`）只是把論文交給 SlideComposer 並打開這個工作；SlideComposer 沒開時先把它打開（`lore-sc://`）、等它回應（最多 30 秒），等不到才顯示「開啟 SlideComposer／重試」。

## 流程

按下去 → 長時間的整理（顯示進度）→ 微調視窗（長度／語言／底版、建議、大綱、最多 4 題，全部可略過）→ 產生投影片 → 排版用字微調 → 打開簡報（右側面板列出排版用字報告）。卡片上另有「直接產生」：跳過微調視窗。

- 微調視窗上方可以改長度、語言、底版。改長度或語言：只重做「大綱」這一步（進度畫面），做完回到微調視窗；同一個頁面內問題的答案、「我懂」、「記得」都保留（重跑時關掉視窗或重新載入就不保留），對大綱列的修改不保留（是新的大綱）。改底版不必重做（投影片那一步才用）。
- 「建議」：大綱寫完後模型再看一次大綱（加上作者的風格摘要與語氣範例），提出最多 5 個具體修改（加頁、刪頁、移動、改標題、改重點、換圖、合併、拆頁），每個附一行理由；逐項「套用」（改上面的大綱列）或「忽略」。沒有值得說的就沒有這一塊。
- 打開產生的簡報時，右側面板最上面是「排版用字」：每一項修正（第幾頁、改了什麼）可單獨「復原」，用詞一致的建議可逐項「套用」；都走編輯器的復原紀錄。之後從 AI 選單 →「排版用字」再打開。

## 工作（job）與階段

- 工作在伺服器端跑，id 在 `<SlideComposer workspace>/cache/todeck/<job>/`（可刪的快取層，永不寫進 private）。
- 每個階段一個 JSON，原子寫入；完成的階段不重做。
- 關掉視窗不會停；頁面用 job id 重新接上。結束 app 會停；下次開啟時提示「上次做到第 N 步，繼續？」，從最後完成的階段接著做。
- 某階段檔案壞掉：只重做那一階段。

| 階段 | 產出 | 進度顯示 |
|---|---|---|
| `source` | 章節、圖、表、方程式（含 label）、參考文獻 | 圖 n/N、表 n、方程式 n |
| `notation` | 論文的對照表條目（沿用論文／LoreReader 的 id） | 條目 n |
| `terms` | 名詞與知識庫比對：熟／不熟 | 名詞 n → 不熟 m |
| `profile` | 作者簡報的風格摘要；更新語氣範例檔（見下） | — |
| `outline` | 每頁：種類、標題、用到的素材、重點、講者備註；接著「建議」 | 模型串流的字數／最後活動時間 |
| `review` | 作者在微調視窗的決定（可略過） | 等待作者 |
| `slides` | 投影片物件 | 第 n/N 頁 |
| `polish` | 排版用字微調（不改句子） | 檢查 n 項 |

- 呼叫模型的階段：取消綁在工作上（不是 HTTP 連線），「停止」會真的中止請求。
- 進度：階段清單（目前那一項亮起）、具體數字、每階段已花的時間、模型步驟顯示串流字數與最後活動時間，讓人看得出沒有當機。

## 本機模型的記憶體

本機模型是整台電腦共用的一個 llama-server（lore-llm `shared_runtime`，見 `docs/local-models.md`）：任何 app 需要時都能啟動，SlideComposer 和其他 app 一樣經 `acquire` 使用。釋放記憶體：閒置 N 分鐘由任何 app 停掉（預設 10 分，在 齒輪 → 模型 設定）；「釋放」按鈕只在沒有其他 app 在用時能按；最後一個在用的 app 結束時停掉。

## 內容順序與風格

- 順序以作者的 seminar 簡報為準（設計文件 §7.1 的論文報告骨架）。
- `profile`：從作者私人資料夾的簡報（經 lore-home 唯讀讀取）抽出精簡的風格摘要，存在快取層：各種頁的頁數分配、標題長度與句型、條列數、Take home 的語氣、圖說與 `Ref:` 的習慣；頁尾標語、logo 等一律排除。摘要不進 repo。
- **語氣範例（2026-11-13 作者同意，取代「只送摘要」）**：除了摘要，也把作者自己以前簡報的句子當範例送進模型（Claude 或本機皆可），讓大綱的標題、條列、Take home 的寫法（用字、長度、中英混用、動詞開頭等）像作者。
  - 來源：只有作者自己的簡報（和摘要讀同一批：從作者自己的檔案匯入的，SlideComposer 產生的不算）。
  - 排除：封面頁與致謝頁、頁尾等版面佔位、同一份簡報反覆出現的文字與圖（頁尾標語、機密標記、講者列、logo）、`Ref:` 與引用（et al.）、e-mail 與網址、機密／版權字樣、作者姓名與 id（`shared/profile.toml`）、`shared/deny-words.txt` 的每個字、只有數字的行、方程式與文獻條目、一兩個字的標籤。
  - 存放：作者的私人 repo（工具資料層，git 管）`private/SlideComposer/voice/examples.json`（經 lore-home 找到），每種頁的標題、條列、Take home、圖說各有上限。不放 `.claude`、skill、公開 repo 或程式常數。簡報有變（與摘要同一個 key，加上排除清單與版本）就重建；內容沒變不改寫檔案。key 存在快取層（`cache/deck-style/voice-key.json`）。工作的檢查點裡沒有範例句子。
  - 使用：`outline` 依論文（標題、關鍵字、摘要、章節名）挑每種頁最相近的幾句（同一句的變體只取一次），整段約 2600 字以內，接在系統提示的摘要後面，說明「學語氣、不抄內容」；範例語言與簡報語言不同時照樣參考語氣、仍用所選語言寫。「建議」用較短的一段（約 1200 字）。投影片與排版用字不再呼叫模型寫句子，所以只有這兩處。
  - 實作：`SlideComposer/crates/server/src/deckvoice.rs`。

## 「不熟」標示（守備範圍）

- 定義：一個詞是「不熟」＝它是領域詞（縮寫、多字詞組、或知識卡的別名），而且知識庫沒有它，或知識卡狀態是「要學」。常用字永遠不算（沿用 LoreReader 的字頻過濾）。
- 範圍：詞、句（含不熟的詞且是背景／方法敘述）、頁（大部分內容不熟）。
- 呈現：新的強調標籤 `learn`。編輯時：詞與句是紅字加紅色點底線，頁是縮圖角落的紅色小標。上方一個切換「標示我不熟的／報告模式」。**匯出、列印、放映一律自動去掉**，不和「關鍵結果」的紅色粗體混淆。
- 微調視窗列出「這 N 個詞不在你的知識庫」，每個詞可勾「我懂」清掉標示；「同時存進知識庫（記得）」是另外的勾選，不自動。

## 排版用字微調（polish，草案）

只檢查與修正下列項目，不改寫句子：
1. 文字溢出框、字級低於標籤規定。
2. 每頁條列數超過風格摘要的上限。
3. 同一種頁的標題長度與句型一致。
4. 圖說與 `Ref:` 有沒有缺。
5. 每頁最多一個強調（關鍵結果紅色粗體）。
6. 用詞精確：同一概念全份簡報用同一個詞；符號用對照表登記的名稱與寫法；Fig./Figure 依簡報風格。
每一項修正都列在微調報告裡，可逐項復原。報告同時存在工作（`polish.json` 檢查點）與簡報資料夾（`decks/<name>/polish.json`）；編輯器讀後者，復原／套用走 `POST /api/decks/{name}/polish/{item}`（Rust 的編輯與復原紀錄）。

## 以後（記錄，尚未做）

- **知識背景文件**：把作者不熟的內容（例如醫學背景、某個新方法所依據的舊方法）抓出來，整理成一份為作者客製的背景介紹，等於「屬於自己的 review paper」，會是寫書功能的起點；投影片上不熟的部分連到這份文件。
- **圖重畫成原生流程圖**：`source` 階段的主架構圖可以選擇用 lore-flow 重畫成可編輯的流程圖，而不是截圖。

## API（SlideComposer 伺服器）

實作：`commons/crates/lore-jobs`（通用的工作執行器，SlideComposer 以 `sc_server::jobs` 轉出）與 `src/todeck/`（這條流程的各階段）。所有路徑在 SlideComposer 的伺服器上；`/api/todeck*` 也回應 127.0.0.1 / localhost 的跨來源 GET/POST（`DELETE` 與 `PUT` 只給 SlideComposer 自己的頁面）。時間是 Unix 毫秒。錯誤：HTTP 狀態碼 + `{ "error": "一行中文" }`。

### 工作物件（job）

`GET /api/todeck/jobs/{id}` 與其他工作路徑都回傳這個形狀：

```json
{
  "id": "j19a6f3c2b1d4",
  "title": "Two-Stage Grading of Synthetic Lesions",
  "created": 1762850000000,
  "state": "running",
  "direct": false,
  "step": 5,
  "doneSteps": 4,
  "total": 8,
  "seq": 42,
  "resumable": false,
  "error": null,
  "failure": null,
  "deck": null,
  "result": null,
  "stages": [
    { "id": "source",   "label": "讀論文",   "state": "done",    "model": false, "ms": 820, "started": 1762850000100,
      "counts": { "fig": 3, "figTotal": 4, "table": 2, "eq": 5, "sections": 6, "refs": 31 } },
    { "id": "notation", "label": "對照表",   "state": "done",    "model": false, "ms": 12,  "counts": { "entries": 14 } },
    { "id": "terms",    "label": "名詞",     "state": "done",    "model": false, "ms": 40,  "counts": { "terms": 37, "unknown": 6 } },
    { "id": "profile",  "label": "風格",     "state": "done",    "model": false, "ms": 5,   "counts": {} },
    { "id": "outline",  "label": "大綱",     "state": "running", "model": true,  "ms": 41200, "started": 1762850001000,
      "chars": 5310, "lastActivity": 1762850042100, "agoMs": 700, "counts": {} },
    { "id": "review",   "label": "微調",     "state": "pending", "model": false, "ms": 0, "counts": {} },
    { "id": "slides",   "label": "投影片",   "state": "pending", "model": false, "ms": 0, "counts": {} },
    { "id": "polish",   "label": "排版用字", "state": "pending", "model": false, "ms": 0, "counts": {} }
  ]
}
```

- `state`：`running`（在跑）· `review`（停在微調，等作者）· `done` · `failed` · `stopped`（按了停止）· `interrupted`（上次 app 結束時還沒做完：伺服器啟動後從快取讀到的 `running` / `review` 都變成這個）。
- `step`：目前（或下一個要做）的階段，從 1 起算 —「上次做到第 N 步，繼續？」的 N。`doneSteps`：已完成（含略過）的階段數。`resumable`：`stopped` / `failed` / `interrupted`。
- `stages[].state`：`pending` · `running`（`review` 等作者時也是 `running`）· `done` · `failed`。直接產生時 `review` 是 `done` 加 `"skipped": true`。
- `ms`：該階段已花的時間（取樣當下；進行中的頁面自己往上加）。`chars` / `lastActivity` / `agoMs` 只在 `model: true` 的階段：模型串流到目前的字數、最後收到字的時間、距今毫秒。
- `counts`：`source` `{fig, figTotal, table, eq, sections, refs}`（圖 n/N：`fig` 有圖檔的，`figTotal` 全部）· `notation` `{entries}` · `terms` `{terms, unknown}` · `profile` `{voice}`（語氣範例的句數）· `outline` `{slides, boxes, suggestions}`（完成後）· `review` `{}` · `slides` `{slide, slides}`（第 n/N 頁）· `polish` `{checks, fixes}`。
- `error`（`failed` 時）：一行中文。`failure`：`{ stage, status, error, alternative? }`，`alternative` 與 lore-llm 的錯誤內容相同（頁面據此提供「改用本機／改用 Claude」= 帶 `engine` 的 `resume`）。
- `deck` / `result`（`done` 時）：簡報資料夾名稱；`{ name, slides, engine, boxes, notation, unknown, warnings, polish: {checks, fixes} }`（`unknown` = 標示的不熟名詞數）。`…/stages/polish` 是完整的微調報告（`{checks, fixes: [{slide, object, rule, what, before, after, changes}], items}`，`items` 含用詞一致的建議）。
- `options`：`{length, lang, style}`，目前的參數（微調視窗據此顯示）。
- `review`（只在 `state: "review"` 時）：

```json
{
  "outline": [ { "id": "s3", "kind": "overview", "title": "Big picture", "figs": ["fig1"], "point": "" } ],
  "figures": [ { "id": "fig1", "label": "圖 1", "caption": "…", "thumb": "/api/todeck/jobs/<id>/img/fig1.png" },
               { "id": "tab1", "label": "表 1", "caption": "…", "thumb": null } ],
  "unknown": [ { "term": "InfoNCE", "n": 4 } ],
  "suggestions": [ { "op": "retitle", "id": "s9", "title": "Main results", "reason": "結果頁標題太籠統" } ]
}
```

  `kind` 是頁的角色：`cover background challenge overview method loss training results ablation visual opinion takeaway thanks`。`figs` 是這頁用的圖／表 id（圖與表各最多一個）；`point` 是 take-away，沒有就是第一個要點。`thumb` 是同源的圖（LaTeX 的表沒有圖）。

  `suggestions`（`sc-outline-review`，class `create`，`todeck/suggest.rs`；最多 5 個，可為空；模型不能用或回答不合格式就是空的，工作不會因此失敗）：每個是對大綱列的一個修改加一行 `reason`（繁中）。`op`：`add {to, kind, title, point, fig?}`（插在 `to` 那列後面，`""` = 最前面）· `remove {id}` · `move {id, to}` · `retitle {id, title}` · `point {id, point}` · `figure {id, fig}`（`""` = 不放圖）· `merge {id, with, title?}`（`with` 併入 `id` 並刪掉）· `split {id, title, point}`（同種類的第二頁接在後面）。id 都是這個 `outline` 的列；封面與致謝不會被改。頁面在自己的列上套用，送出的仍是下面的微調內容（新頁用沒有的 id，例如 `n1`）。

### 路徑

| Method | Path | Body | 回傳 |
|---|---|---|---|
| GET | `/api/todeck/papers` | | `{pc: [{path, title, name}], lr: [{sha, title, pdf}]}` |
| POST | `/api/todeck` | `{source: {kind: "pc", path} \| {kind: "lr", sha}, style?, length?, lang?, engine?, mode: "review" \| "direct"}` | `{job: "<id>", …工作物件}`；伺服器在背景開始做 |
| POST | `/api/todeck`（沒有 `mode`） | 同上 | 舊的一次完成：直接產生並等做完，回 `result`；失敗回 `failure.status` 與 `{error, alternative?}`（給還沒改版的「做成簡報」） |
| POST | `/api/todeck/jobs` | 同上（或 `direct: true`） | 201，`{job, …工作物件}` |
| GET | `/api/todeck/jobs` | `?unfinished=1` 只列 `interrupted` / `stopped` / `failed` / `review` | `{jobs: [工作物件…]}`，新的在前 |
| GET | `/api/todeck/jobs/{id}` | | 工作物件 |
| GET | `/api/todeck/jobs/{id}/events?after={seq}` | | 長輪詢：`seq` 大於 `after` 時立刻回傳工作物件，否則最多等 25 秒。每秒 GET 一次也可以 |
| POST | `/api/todeck/jobs/{id}/stop` | | 工作物件（`stopped`）；進行中的模型請求被中止，工作的執行緒被釋放（`…/cancel` 同義） |
| POST | `/api/todeck/jobs/{id}/resume` | `{engine?: "local" \| "claude"}` | 工作物件；從第一個沒有檢查點的階段接著做；正在跑時 409 |
| POST | `/api/todeck/jobs/{id}/review` | 見下（`PUT` 同義） | 工作物件；不在 `review` 時 409 |
| POST | `/api/todeck/jobs/{id}/params` | `{length?, lang?, style?}` | 工作物件；只在 `review` 時（否則 409），值不認得 400。`length` 或 `lang` 變了：刪掉 `outline` 以後的檢查點，工作從 `outline` 重跑，做完再停在 `review`；只改 `style`：不重跑 |
| DELETE | `/api/todeck/jobs/{id}` | | 204；刪掉這個工作的快取資料夾（正在跑時 409） |
| GET | `/api/todeck/jobs/{id}/stages/{stage}` | | 該階段的檢查點內容（`outline`：`{plan: {venue, keywords, slides: [PlanSlide…]}, questions: [], boxes, engine, suggestions}`；`terms`：`{terms: [{term, unknown, n, kind, card}]}`）；沒做完 404 |
| GET | `/api/todeck/jobs/{id}/img/{file}` | | 論文的圖（PNG / JPEG） |

桌面版連結：`lore-sc://job/<id>` → `/?job=<id>`（`LoreLinks.open("sc", {job})`）。

### 微調（`POST …/{id}/review`）

```json
{ "skip": true }
```

或

```json
{
  "outline": [ { "id": "s3", "kind": "overview", "title": "…", "figs": ["fig1"], "point": "…" } ],
  "answers": { "audience": "lab", "mainFig": "fig2", "modules": 3, "opinion": false },
  "known": ["InfoNCE"],
  "remember": true
}
```

- `outline`：照頁面的順序；刪掉的列就是不要的頁；`id` 對回模型的頁（新頁可用沒有的 id，配 `kind`）；改了的 `point` 寫回 take-away（或第一個要點）。
- `answers`：每題可為 `null`。`mainFig` 換掉大架構與方法頁的圖（模組框隨之不用）；`opinion: false` 拿掉「我的看法」頁，`true` 而沒有就加一頁空的。`audience`、`modules` 目前只記錄。
- `known`：勾了「我懂」的詞：送出時寫進 `terms` 的檢查點（`understood`），投影片就不標它們；`remember: true` 另外經 KnowledgeBase 的 API 存成「記得」（它沒開就略過，訊息記在 `review.json` 的 `notes`）。投影片做好之後的「我懂」是 `PUT /api/decks/{name}/learn`。

### 檢查點

`<SlideComposer workspace>/cache/todeck/<id>/`：`job.json`（參數、狀態、各階段時間與數字）、每個完成的階段一個 `<stage>.json`（`{stage, output, counts, elapsed_ms, finished}`）、`img/`（論文的圖）。全部原子寫入。檔案讀不到的階段重做，其他不動；`slides` 做到一半當掉時，下次先刪掉那份做一半的簡報（`slides.pending`）。

### 模型設定（`/api/engines`，每個 app 都有）

- `GET` 的 `local` 多了 `idle_minutes`（0 = 不自動釋放，預設 10）與 `can_release`（只有管理本機模型的 app ＝ PaperComposer 是 `true`）。
- `PUT {local: {idle_minutes: 15}}`：存在 `private/shared/engine.toml` 的 `[local]`，各 app 共用；PaperComposer 的 llama-server 每次檢查時讀它。
- `PUT {local: {release: true}}`：立刻停掉 llama-server；其他 app 回 400。
- 「使用中」以 llama-server 的 `/slots` 判斷（有 slot 在處理，或 task id 有變），所以 SlideComposer 等直接打網址的請求也算；app 結束時照舊 `kill_now`。
