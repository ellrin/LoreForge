# 論文變簡報（paper → deck）

從 PaperComposer 的論文或 LoreReader 的論文產生一份可上台的論文報告簡報。工作在 SlideComposer 裡跑；PaperComposer 的「做成簡報」只是把論文交給 SlideComposer 並打開這個工作。

## 流程

按下去 → 長時間的整理（顯示進度）→ 微調視窗（大綱＋最多 4 題，全部可略過）→ 產生投影片 → 排版用字微調 → 打開簡報。卡片上另有「直接產生」：跳過微調視窗。

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
| `profile` | 作者簡報的風格摘要（見下） | — |
| `outline` | 每頁：種類、標題、用到的素材、重點、講者備註 | 模型串流的字數／最後活動時間 |
| `review` | 作者在微調視窗的決定（可略過） | 等待作者 |
| `slides` | 投影片物件 | 第 n/N 頁 |
| `polish` | 排版用字微調（不改句子） | 檢查 n 項 |

- 呼叫模型的階段：取消綁在工作上（不是 HTTP 連線），「停止」會真的中止請求。
- 進度：階段清單（目前那一項亮起）、具體數字、每階段已花的時間、模型步驟顯示串流字數與最後活動時間，讓人看得出沒有當機。

## 本機模型的記憶體

本機模型由 PaperComposer 管理的 llama-server 提供（`private/shared/engine.toml` 公布網址），SlideComposer 只是使用者，不能關掉它。釋放記憶體由 lore-llm 的 `LocalRuntime` 負責：閒置 N 分鐘自動釋放（預設 10 分，在 齒輪 → 模型 設定），另有「釋放」按鈕；app 結束時照舊 `kill_now`。

## 內容順序與風格

- 順序以作者的 seminar 簡報為準（設計文件 §7.1 的論文報告骨架）。
- `profile`：從作者私人資料夾的簡報（經 lore-home 唯讀讀取）抽出精簡的風格摘要，存在快取層：各種頁的頁數分配、標題長度與句型、條列數、Take home 的語氣、圖說與 `Ref:` 的習慣；頁尾標語、logo 等一律排除。送進模型的是這份摘要，不是簡報本身。摘要不進 repo。

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
每一項修正都列在微調報告裡，可逐項復原。

## 以後（記錄，尚未做）

- **知識背景文件**：把作者不熟的內容（例如醫學背景、某個新方法所依據的舊方法）抓出來，整理成一份為作者客製的背景介紹，等於「屬於自己的 review paper」，會是寫書功能的起點；投影片上不熟的部分連到這份文件。
- **圖重畫成原生流程圖**：`source` 階段的主架構圖可以選擇用 lore-flow 重畫成可編輯的流程圖，而不是截圖。

## API（SlideComposer 伺服器）

所有路徑在 SlideComposer 的伺服器上（`/api/todeck*` 也回應 127.0.0.1 / localhost 的跨來源 GET/POST；`PUT …/review` 與 `DELETE` 只給 SlideComposer 自己的頁面）。時間一律是 Unix 毫秒。錯誤：HTTP 狀態碼 + `{ "error": "一行中文" }`。

### 工作物件（job）

`GET /api/todeck/jobs/{id}`、`POST /api/todeck/jobs`、`…/resume`、`…/cancel`、`PUT …/review` 都回傳這個形狀：

```json
{
  "id": "j18f3a2b4c5d",
  "created": 1762850000000,
  "title": "Two-Stage Grading of Synthetic Lesions",
  "source": { "kind": "pc", "path": "/…/papers/two-stage" },
  "options": { "length": "standard", "lang": "zh", "style": "blank", "engine": null },
  "direct": false,
  "status": "running",
  "step": 5,
  "done_steps": 4,
  "total": 8,
  "seq": 42,
  "stages": [
    { "id": "source",   "label": "讀論文",   "state": "done",    "model": false, "started": 1762850000100, "elapsed_ms": 820,
      "counts": { "figures": 3, "figures_total": 4, "tables": 2, "equations": 5, "sections": 6, "refs": 31 } },
    { "id": "notation", "label": "對照表",   "state": "done",    "model": false, "elapsed_ms": 12, "counts": { "entries": 14 } },
    { "id": "terms",    "label": "名詞",     "state": "done",    "model": false, "elapsed_ms": 40, "counts": { "terms": 37, "unknown": 6 } },
    { "id": "profile",  "label": "風格",     "state": "done",    "model": false, "elapsed_ms": 5,  "counts": {} },
    { "id": "outline",  "label": "大綱",     "state": "running", "model": true,  "started": 1762850001000, "elapsed_ms": 41200,
      "chars": 5310, "last_activity": 1762850042100, "counts": {} },
    { "id": "review",   "label": "微調",     "state": "pending", "model": false, "counts": {} },
    { "id": "slides",   "label": "投影片",   "state": "pending", "model": false, "counts": {} },
    { "id": "polish",   "label": "排版用字", "state": "pending", "model": false, "counts": {} }
  ],
  "error": null,
  "result": null
}
```

- `status`：`running`（在跑）· `waiting`（停在 `review` 等作者）· `done` · `failed` · `cancelled`（按了停止）· `stopped`（上次 app 結束時沒做完；伺服器啟動後從快取讀到的就是這個）。
- `step`：目前（或下一個要做）的階段，從 1 起算；`done_steps`：已完成（含略過）的階段數，「上次做到第 N 步」的 N 用它。
- `stages[].state`：`pending` · `running` · `waiting` · `done` · `skipped`（`review` 在直接產生時）· `failed` · `cancelled`。
- `elapsed_ms`：該階段已花的時間（進行中的會一直增加；頁面可用 `started` 自己算秒數）。`chars` / `last_activity` 只在 `model: true` 的階段出現：模型串流到目前的字數與最後一次收到字的時間。
- `counts` 各階段的鍵：`source` `{figures, figures_total, tables, equations, sections, refs}`（圖 n/N：`figures` 是有圖檔的，`figures_total` 是全部）· `notation` `{entries}` · `terms` `{terms, unknown}` · `profile` `{}` · `outline` `{slides, questions, boxes}`（完成後）· `review` `{}` · `slides` `{slide, total}`（第 n/N 頁）· `polish` `{checks, fixes}`。
- `error`（`failed` 時）：`{ "stage": "outline", "status": 503, "error": "一行中文", "alternative": {…} }`，`alternative` 與 lore-llm 的錯誤內容相同（頁面據此提供「改用本機／改用 Claude」，按下去就是帶 `engine` 的 `resume`）。
- `result`（`done` 時）：`{ "name": "<deck 資料夾>", "slides": 14, "engine": "Claude", "boxes": 3, "notation": 1, "warnings": [] }`。

### 路徑

| Method | Path | Body | 回傳 |
|---|---|---|---|
| GET | `/api/todeck/papers` | | `{pc: [{path, title, name}], lr: [{sha, title, pdf}]}`（不變） |
| POST | `/api/todeck/jobs` | `{source: {kind: "pc", path} \| {kind: "lr", sha}, length?, lang?, style?, engine?, direct?}` | 工作物件（201）；伺服器立刻在背景開始做 |
| GET | `/api/todeck/jobs` | | `{jobs: [工作物件…]}`，新的在前；`resumable` 的（`stopped` / `cancelled` / `failed` / `waiting`）就是要提示「上次做到第 N 步，繼續？」的 |
| GET | `/api/todeck/jobs/{id}` | | 工作物件 |
| GET | `/api/todeck/jobs/{id}/events?after={seq}` | | 長輪詢：工作的 `seq` 大於 `after` 時立刻回傳工作物件，否則最多等 25 秒（沒變化就回傳目前的）。頁面拿回傳的 `seq` 再問下一次。關掉頁面不影響工作 |
| GET | `/api/todeck/jobs/{id}/stages/{stage}` | | 該階段的產出（見下）；還沒做完 404 |
| POST | `/api/todeck/jobs/{id}/cancel` | | 工作物件（`cancelled`）；正在進行的模型請求會被中止 |
| POST | `/api/todeck/jobs/{id}/resume` | `{engine?: "local" \| "claude"}`（可省） | 工作物件；從最後完成的階段接著做（完成的階段不重做）；正在跑時 409 |
| PUT | `/api/todeck/jobs/{id}/review` | 見下 | 工作物件；寫下作者的決定並繼續做 |
| DELETE | `/api/todeck/jobs/{id}` | | 204；刪掉這個工作的快取資料夾（正在跑時 409） |
| POST | `/api/todeck` | 同 `POST /api/todeck/jobs`（一律直接產生） | 舊的一次完成：等工作做完才回 `result` 的內容；失敗時回 `error` 的狀態碼與 `{error, alternative?}` |

每個工作物件另有 `"resumable": true|false`。

### 階段產出（`…/stages/{stage}`）

- `outline`：`{ "plan": { "venue": "", "keywords": [], "slides": [PlanSlide…] }, "questions": [], "boxes": [{label, bbox: [x0,y0,x1,y1]}], "engine": "claude-code:…" }`。
  `PlanSlide` = `{role, title, bullets[], figure, table, equations[], takeaway, steps[], module, kv[[k,v]…], cites[], notes}`（`role` 是 `cover background challenge overview method loss training results ablation visual opinion takeaway thanks` 之一）。
  `questions`：`[{id, text, options[]}]`，最多 4 題（目前一律空陣列）。
- `terms`：`{ "terms": [{ "term": "…", "unknown": true, "card": null, "kind": "abbr" }] }`。
- `review`：作者送出的內容（直接產生時 `{ "skipped": true }`）。
- 其他階段的產出是內部格式，頁面不需要讀。

### 微調（`PUT …/review`）

```json
{
  "skipped": false,
  "plan": { "venue": "", "keywords": [], "slides": [ … 改過的 PlanSlide … ] },
  "answers": { "q1": "…" },
  "known": ["term a"],
  "remember": ["term a"]
}
```

全部可省：`skipped: true` 等於「全部略過」；沒有 `plan` 就用模型的大綱；`known` = 勾了「我懂」的詞（清掉標示）；`remember` = 另外勾了「同時存進知識庫」的詞。只有停在 `review`（`status: waiting`）的工作可以送，否則 409。

### 模型設定（`/api/engines`，每個 app 都有）

- `GET` 的 `local` 多了 `idle_minutes`（數字，0 = 不自動釋放，預設 10）與 `can_release`（只有管理本機模型的 app，也就是 PaperComposer，是 `true`）。
- `PUT {local: {idle_minutes: 15}}` 改閒置釋放的分鐘數（存在 `private/shared/engine.toml` 的 `[local]`，各 app 共用）。
- `PUT {local: {release: true}}` 立刻釋放本機模型（停掉 llama-server）；不是管理本機模型的 app 回 400。
