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
