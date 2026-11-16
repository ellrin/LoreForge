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

第 3 段 commit：2bbfa6d。

## 第 4 段：回答與要點回填

- `/answer`：立刻回應（串 → answered、`next` 前進、那一段 `busy = "fill"`），回填在背景跑 `pc-draft-fill`；
  選項的文字與打的字合成作者的回答。`skip` → skipped。已回答的題再答 → 400。`/note`：作者主動說的話 → `note` 串，同樣回填；
  掛在框上的 note 回填到框的第一段。
- 回填落地時重新讀檔、照 id 找段落（移動過沒關係）；`points` 只換掉 `from` = 這一串的，其他要點不動；串 → done、結論一行。
  段落在回填期間被刪 → 結果丟掉、串照樣結束（掛到框上；復原段落後再落地就照常寫回）。
- 追問：模型說 `gap` 且答案不是「沒有／還沒／不是…」才加，最多一個、追問不再追問；放在頁面現在顯示的那一題之後（不搶走 `next`）。
  「沒有／還沒有」照記成要點與結論，不再追問。
- 回答裡的寫法 → 寫法筆記（`notes::add_in`，group = 段落 id、章節 = 段落所在章對應的舊章名）；第 5 段的禁用字檢查會讀到。
- 回填失敗（沒有引擎）也會把串結束，結論 = 回答的第一行，不會讓頁面一直輪詢；App 關掉時留下的 answered 串，下次讀取時重新回填。
- 假模型可以設 `"_delay_ms": { "任務": 毫秒 }`，測試用它讓編輯發生在回填途中。

測試：`cargo test -p pc-core skeleton` → 28 passed；`cargo test -p pc-server --test skeleton_api` → 1 passed
（回答 → 回填中移動段落 → 要點落在新位置、移動保留；略過；「還沒有」不追問；回填中刪段 → 丟掉；框上的 note；兩次 GET 同 rev）。

第 4 段 commit：cafbc20。

## 第 5 段：逐段文字

- `draft.rs` 的逐章草稿拆成 `compose_env`（一章只讀一次：期刊寫法、作者規則與標點守門、對照表、記錄事實、其他章文字）與
  `compose_one`（一段：句型庫、風格、`compose_extra`、待補標記、對照表檢查與一次重寫、標點／句長、unicode → LaTeX、
  記錄編號外洩、禁用字、摘要長度，最後守門的安全修正）。舊的 `run_compose` 改成呼叫它們（HTTP 測試裡照舊跑過一次）。
- `/write { id }`：段落 id → 重寫那一段（就算已有文字）；框、章、`all` → 只寫沒有文字的段落，已有文字的一個位元組都不動
  （協調者說明 2，測試比對整段 JSON）。沒有可寫的 → 400。
- 背景：章與章同時（最多 3），同一章依序（後一段看得到前面寫好的），每段寫完立刻落地（`busy = "write"` 清掉、`state = text`、
  `job.done` +1）。段落在寫的途中被移動 → 寫到它現在的位置；被刪 → 不寫。
- 提示多加：框的目的、這段的一行、`link_prev`／`link_next`（要寫出轉折）、下一段要說什麼（不要先說）。`skeleton::plan::outline_para`
  把段落轉成舊大綱段落的形狀（say → zh、points → en），所以舊的檢查全部沿用。
- 「跳」：每章寫完，程式先挑（跟上一段沒有共同的依據、圖表、六個字母以上的詞，也沒有 `link_prev`），有才叫一次修補任務只問原因與改法。
- 寫法筆記：回答裡說的禁用字在那一段的文字裡被抓出來（測試：p3 有、p1 沒有）。
- 尚未做：新符號（registry 沒有的）目前只出現在 `problems` 的「對照表：」訊息裡，骨架沒有另存 `new_symbols` 清單（舊流程的「登記符號」按鈕讀 drafts）。

測試：`cargo test -p pc-core skeleton` → 30 passed；`cargo test -p pc-server --test skeleton_api` → 1 passed。

第 5 段 commit：73c5970。

## 第 6 段：細修與風格對照

- `/refine { para, preset }`：`wording` = lore-polish 的「用字」預設、`style` = 「我的風格」預設（同 潤飾 視窗：LaTeX 鎖住、
  改寫後過 lore-notation 檢查、寫進 polish.jsonl），`check` = 只有程式：作者 profile 的字詞檢查（有建議的才變成修改）與
  標點守門的機械修正。背景執行（`busy = "refine"`），完成時多一串 `refine`：句子對得上就一句一個 `Change`（`before` 是目前文字裡的一段），
  對不上就整段一個；沒有建議或沒通過檢查 → 這串直接 done，`conclusion` 說原因。
- `/change accept=true` 套用前確認 `before` 還在文字裡且只出現一次，否則 400 並說明。
- `GET /style`：只用程式。每章（有文字的）：每段字數、每段句數、每句字數（中位數）、句長四分位距、相鄰句長變化率、結論句比例、
  每段連接詞開頭的句數、禁用字、標點（有守門規則才有）。作者那一側只用程式已經有的：`checks.toml` 的段落範圍（字數、句數）、
  styleprofile（每句字數、每段字數／句數、結論句比例、他的連接詞清單）、寫法筆記與 vocabulary 的禁用字。
  沒有作者數字的（句長離散、變化率、連接詞）`mine = null`。只有單一典型值（styleprofile 的中位數）時寫成 `[v, v]`，
  `ok` = 差距在四分之一以內（這個容許值是程式的約定，留給作者決定）。超出範圍的段落列在 `paras`：字數、句數、
  「句長變化太小」（三句以上、句長全在三個字以內的差距；同樣是程式的約定）、禁用字、標點。
- 假模型也接上了 lore-polish（`PcHost::complete` 在 mock 時讀 `polish` 任務）。

測試：`cargo test -p pc-core skeleton` → 32 passed；`cargo test -p pc-server --test skeleton_api` → 1 passed；
`cargo test -p pc-server --lib polish` → 4 passed。

### 合約問題
- `/refine` 是背景工作：回應時那一段 `busy = "refine"`，`refine` 串在完成後才出現（頁面照常輪詢）。
- `StyleReport` 的數字是小數（例如 `17.0`），合約範例是整數；JSON 都是 number。

第 6 段 commit：113d99f。

## 第 7 段：主張、章節校正、摘要、審稿、放進論文改讀骨架

- `skeleton::project`：有骨架時，在呼叫當下把骨架做成舊步驟讀的形狀（不存檔，骨架是唯一來源）：
  章 → 大綱（role 對到五個舊章名；標題不同的，例如 Related work，成為 Introduction 下的 subsection）、
  有文字的段落 → drafts（舊 drafts 不讀，只有舊步驟寫的 Abstract 保留）、框 → 題組（purpose = explain）、段落 → 項目
  （say = label、points = stance、sources = evidence、cites = refs）。沒有骨架的 session 原封不動。
- `run_claims`、`run_review`、`run_compose`（摘要）、`run_audit`（含期刊要求的檢查）、`insert_drafts` 都改成讀 `reading(session)`。
- `/place { section }`：骨架的一章（或全部，含摘要）透過 `topaper::place` 放進論文，原有文字照舊先存檔；
  沒有可放的文字、或沒有放進任何章節 → 400（協調者說明 3），不會回報空的成功。

測試：`cargo test -p pc-core skeleton` → 33 passed；`cargo test -p pc-server --test skeleton_api` → 1 passed
（主張、摘要、審稿從骨架讀；放一章只動那一章；全部放；沒有文字 → 400）；`cargo test -p pc-server --lib draft` 見下。
（`cargo test -p pc-server --lib draft` → 25 passed。）

第 7 段 commit：bd48992。

## 第 8 段：隔離伺服器 `--mock` 與整段流程測試

- `cargo run -p pc-server --example isolated_server -- <空資料夾> [port] --mock crates/server/tests/fixtures/skeleton_mock.json`
  （port 預設 7990）。`--mock` 設 `PAPERCOMPOSER_LLM_MOCK` 與 `PAPERCOMPOSER_ENGINES_ISOLATED`：所有模型呼叫（草稿任務、
  逐段文字、lore-polish）都從檔案回答，不碰網路、`claude` CLI、本機模型；並在工作區加一篇合成的論文 `skeleton-demo`
  （`tests/fixtures/skeleton_seed.json`：Method A／B／C、內外部測試集、圖表規劃），頁面用
  `/api/papers/skeleton-demo/draft/skeleton` 開始。已在暫存資料夾實際啟動過一次（GET、排骨架、4 題、`next = q1`）。
- `tests/fixtures/skeleton_mock.json`：排骨架（10 段、4 題）、三次回填（第二次開一個追問與一條寫法筆記）、修補、逐段文字、
  用字潤飾、主張、校正、審稿的回覆。`_` 開頭的鍵（`_comment`、`_delay_ms`）不是任務。
- `tests/skeleton_flow.rs`：用同一個檔案走完：素材清單 → 排骨架 → 三個回答（各等回填落地）→ 略過 → 全部寫 → 用字 → 套用 → 風格對照。
- `pc_server::seed_skeleton_demo(workspace)`：範例與測試共用的種子。

測試：`cargo test -p pc-server --test skeleton_flow` → 1 passed。

第 8 段 commit：9bb9fdc。

## 收尾修正（審查後）

- `/write` 對框、章、`all`：每段輪到時重新讀檔，作者在等待期間打的文字照樣保留（落地時再檢查一次），只算進度。
  測試：寫 `all` 途中編輯排在後面的段落 → 文字不變；之後單獨 `/write` 那一段才重寫。
- 寫文字時，勾選的素材裡圖表規劃與論文都沒有的 label（作者新增的圖表、偽代碼、程式產生 label 的結果表）放進對照表檢查的
  脈絡（佔位圖表），`\ref` 不會被當成未定義。**沒有專門的測試**。放進論文時的佔位圖表仍然只看圖表規劃（見下方未做）。
- `/answer` 只接受問題（conflict／choice／missing 且有題目），edit／refine／note 串 → 400。
- 假模型模式下 `Llm::route()` 一律拒絕（「假模型模式：<任務> 沒有腳本」）：沒有腳本的功能絕不會走到真的引擎。
  有腳本的：`Llm::complete`／`complete_on`（所有討論草稿任務、骨架三個任務、逐段文字）與 lore-polish（用字、我的風格）。
  **沒有腳本、mock 時會回錯誤的**：流程圖（lore-flow）、繪圖（lore-plot）、期刊寫法、新範本、寫書（它們有自己的模型介面）。

## 最後一次全部測試

- `cargo test -p pc-core` → 261 passed。
- `cargo test -p pc-server --no-fail-fast` → lib 185 passed、1 failed；abort、book_api、flow_api、notes_api、open_api、
  pending_api、picked_api、skeleton_api、skeleton_flow、style_api、todeck_api、venue_api 各 1 passed。
  失敗的是 `shell::tests::the_shell_files_are_served_and_the_page_loads_them_in_order`：`ui/skeleton.js` 用了原生 `confirm()`
  （規則是改用 `LoreShell.ask`）。前端的檔案，不在我的範圍，沒有改。

### 合約問題（補）
- 刪掉整個框時，掛在裡面段落的已完成串改掛到「章」的 id（合約只說段落或框；框已經不在了）。
- 復原一個 `add_para` 時，掛在那一段上的串（例如作者的 note）會指向不存在的段落 id。頁面請把找不到 anchor 的串放在章末，或告訴我要怎麼處理。

## 留給作者決定／還沒做

- 風格對照的兩個程式約定：單一典型值的容許範圍（±25%）、「句長變化太小」（三句以上全在 3 個字差距內）。
- 素材清單不叫模型；`algorithm` 只能作者自己加；作者的想法（H…）預設勾選。
- 骨架沒有另存「新符號」清單（只在段落 problems 的「對照表：」訊息裡）。
- 放進論文的佔位圖表只看圖表規劃：作者在素材清單新增、圖表規劃沒有的圖表，放進論文後 `\ref` 會是未定義，要作者自己補上。
- `session.json` 現在帶著骨架與最多 50 步復原（每步一份章節快照），每次存檔整份重寫，舊的 `/draft` 視圖也會帶著它：
  長論文時檔案大小與存檔時間**沒有量過**。
- 規劃品質、回填品質、真模型的速度：都只用假回覆測過，**沒有在真的模型上跑過**。
