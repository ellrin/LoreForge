# 骨架 — 後端進度（pc-core / pc-server）

設計：`docs/long-paper.md` §G；合約：`docs/skeleton-api.md`；假資料：`crates/core/tests/fixtures/skeleton_view.json`。

## 第 1 段：資料、操作、復原、轉換

- `pc_core::draft::skeleton`：`Skeleton`（存在 session 的 `skeleton` 欄位，舊 session 沒有它照常載入，也不會寫出空欄位）、
  `View`（合約的形狀，null 與空陣列照寫）、`Section` `Block` `Para` `Thread` `Change` `Inv` `Job` `Targets` `Op`。
- id 由存在骨架裡的計數器給（s b p q n e r v 各一個），刪掉的 id 不再用。
- 操作（add_para add_block remove move edit reason done undo）都是純函式，在伺服器的鎖裡套用。
  復原最多 50 步；結構操作（新增、刪除、移動）復原時只把位置還原，段落裡背景寫進去的東西（要點、文字）保留；
  edit 復原只還原它改的欄位。刪除與移動讓前後段變成「接縫」（`seams`，不在 View 裡），暫存時一起修補。
- 轉換：題組 → 框（同一題組連續的段落一個框），大綱段落 → 段（沿用 p 編號），草稿 → `text`，
  每個有回答的題組 → 一串 `note`（done，結論 = 題組整理的 conclusion，沒有就 explain），掛在題組的第一段。
  還沒有大綱的 session：`answers_block` 把已回答的問答交給規劃者，`attach_answers` 依規劃者回的 `answers` 掛上去。
- 存檔：`session::snapshot`（從 rewind 抽出來，rewind 改呼叫它），轉換與重排前先存一份到 `draft/archive/`。
- 寫法筆記：`notes::add_in`（指定章節）；`for_paragraph` 也認「說在這一段上」的筆記（group = 段落 id）。

測試：`cargo test -p pc-core skeleton` → 19 passed；`cargo test -p pc-core session` → 15 passed。

第 1 段 commit：f6d0637。補充（協調者的合約說明 4）：刪除段落時，掛在上面還開著的問題拿掉，其他的串改掛到框（刪整個框時掛到章）；
復原時一起回來。測試 `removing_a_paragraph_drops_its_open_questions_and_keeps_the_rest_on_the_block`；`cargo test -p pc-core skeleton` → 20 passed。

## 第 2 段：素材清單

- `skeleton::inventory`：不叫模型，從已經有的東西組出來：圖表規劃（表、圖；工具是 flow 的算 `flow`）、
  圖表規劃沒涵蓋的結果表、有結果的實驗與作者的想法（`finding`）、對照表（lore-notation）裡有 label 的編號公式（`equation`）。
  `algorithm` 只有作者自己加。依據編號對照記錄核對，不存在的丟掉。
- 作者勾選之前，清單每次照記錄重算（同一來源的 id 不變）；勾選之後清單是作者的：記錄有新東西才加進來，作者的選擇不動，
  來源消失的項目拿掉（作者自己加的留著）。
- 作者新增的圖、表、流程、公式、偽代碼標 `pending`（還要作者做出來），label 由標題的英文字產生，沒有英文字就用 id（`fig:v9`）。
- `targets`：期刊寫法的 `float_targets()`，公式用同樣的算法從 `equations` 的平均算；沒有期刊資料是 null。
- 伺服器：`crates/server/src/draft/skeleton.rs`（draft.rs 的子模組，共用 LOCK、ctx、load/save）。所有寫入經過 `with_sk`：
  鎖、重新讀檔、照 id 改、有變才 `rev`+1 並存檔；兩次 GET 中間沒變就是同一個 `rev`。
  還沒有骨架的 session 讀取時不寫檔（清單用算的）。
- 路由：GET ``、POST `/inventory`。

測試：`cargo test -p pc-core skeleton` → 22 passed；`cargo test -p pc-server --test skeleton_api` → 1 passed（HTTP：清單、兩次讀取相同、勾選、新增、錯誤不改東西）。

第 2 段 commit：42bfc29。

## 第 3 段：規劃、修補、任務、假模型

- 任務（`crates/server/tasks.toml`）：`pc-draft-skeleton` 預設 `claude:opus`、沒有 class、沒有備援（本機模型永遠不是候選）；
  Opus 安全擋下時照舊由 `Attempts::failed` 改 Sonnet（`skeleton_plan_refused_by_opus_goes_to_sonnet`）。
  `pc-draft-skeleton-repair` = `claude:sonnet` → `local:current`，class `create`。
  `pc-draft-fill` = `claude:haiku` → `local:current`，class `create`（`ask` 會把本機排第一，測試裡證明了這一點）。
  每個任務一個「第一個候選」測試（`llm.rs`，用共用設定的 `router::plan`，和 `discuss_never_routes_to_the_local_model` 同法）。
- 規劃（`skeleton::plan`）：輸入 = 主題、`prompts::context()`（整理過的記錄、論文的圖表與參考文獻、對照表）、勾選的素材、
  風格段落類型（`style_text`）、期刊寫法（`Venue::outline`）、轉換時的已回答問答。提示要求先排結果 → 討論 → 方法 → 介紹／相關研究／結論，
  回傳照論文順序（程式也依 role 排序）。每段 `points` = 記錄已經支持的要點（協調者說明 1），有要點的段落是 `points` 狀態。
  程式核對：label 只留勾選的素材、cites 只留論文的 key、sources 只留記錄有的 id、pending 只留有的、空段空框空章拿掉、
  `of` 由模型的 key 換成真的 id、問題只留三種且掛在段落上、選項最多 3、最多 15 題（模型的順序 = 重要性）、
  問題與選項裡的內部編號（turn N、第 N 輪、記錄 id、模型的 key）由程式刪掉。
- 修補（暫存）：只送被動到的段落、接縫、與前後鄰居；回覆裡範圍外的段落與框一律不套用（測試逐段比對 JSON 位元組相同）；
  每項修改是一個 applied 的 `Change`，依目標分成 `edit` 串；`/change accept=false` 還原。修補進行中又被改的段落保留標記。
- 背景工作：`/plan`、`/convert`（沒有大綱時）、`/save` 在背景跑，`job` 存在骨架裡；同一篇只能有一個 job（409）。
  App 關掉留下的 job → 讀取時標成錯誤；`busy` 的段落沒有在跑 → 清掉。
- 假模型：`PAPERCOMPOSER_LLM_MOCK=<檔案>` 時 `Llm` 的 `complete`／`complete_on` 都從檔案回答（每個任務照順序，最後一個重複），
  `ready()` 一律 true；不碰網路、`claude` CLI、本機模型。
- 路由：`/plan`、`/ops`（全部套用或全部不套用）、`/save`、`/change`、`/convert`。

測試：`cargo test -p pc-core skeleton` → 25 passed；`cargo test -p pc-server --lib -- draft::engine llm::` → 23 passed；
`cargo test -p pc-server --test skeleton_api` → 1 passed（排骨架 → 編輯 → 暫存 → 還原 → 有大綱的轉換 → 訪談中途的轉換）。

### 合約問題
- `job.error` 有值時 job 仍然非 null（頁面照合約會一直輪詢）。建議：頁面看到 `error` 非空就停止輪詢並顯示；下一個動作（ops、change、
  inventory、save、plan）會清掉失敗的 job。
- `/convert` 沒有大綱時是背景工作（`job.kind = "convert"`），回應時 `exists` 還是 false。
- 轉換來的訪談串 `kind = "note"`、`status = "done"`（合約沒有專給轉換的種類）。
