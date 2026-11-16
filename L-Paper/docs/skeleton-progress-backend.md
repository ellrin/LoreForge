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
