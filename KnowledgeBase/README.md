# KnowledgeBase

本機運作的個人知識庫：一張知識卡一個 JSON 檔，記錄「我記不記得」，讓其他 App（PaperComposer、SlideComposer、JELearner、NeonDeck 小工具）查詢術語、提醒複習。設計見 `../private/shared/docs/ecosystem-design.md`（作者的私人筆記） §4。

> 開發中（第 1 波 A1：知識庫核心）。已完成：`lore-store`（space、iCloud／git 圖片）、`kb-core`（知識卡、連結、標籤、學習狀態機、SQLite/FTS5 索引、術語掃描）、kb-server API、最小網頁介面（清單、編輯器、複習、時間軸、圖譜）。
> 第 2 波 B2：NeonDeck 小工具（`NeonDeck/desktop/widgets/knowledge.js`）、到期通知、每日 e-mail 摘要、登入自動啟動腳本。
> 尚未完成：候選術語仍用離線 mock（起草卡片已接上 `lore-llm`）、家庭 space 的實際掛載。

## 執行

需要 [Rust](https://rustup.rs)（1.85 以上）。

```sh
cargo run -p kb-server
```

開啟 <http://127.0.0.1:7700>（只監聽本機）。預設掛載`../private/KnowledgeBase`（你的個人 space）。

Mac App：在 LoreForge 根目錄執行 `scripts/install-apps.sh KnowledgeBase`（`apps/desktop` 的 Tauri 殼）。App 用 7700 埠（NeonDeck 小工具讀這個埠）；如果 7700 已經有 kb-server 在跑（例如用下方的 launchd 腳本開機啟動），視窗就直接開那一個，不另外啟動。App 自己啟動伺服器時，提醒排程只在 App 開著時運作。圖示原始檔為 `apps/desktop/icons/logo.svg`（換圖方式同 SlideComposer）。

其他 space 或使用者：

```sh
cargo run -p kb-server -- --space ~/path/to/space --space ~/JELearner/private/knowledge --member ellrin
KB_SPACES=/a:/b KB_MEMBER=ellrin PORT=7700 cargo run -p kb-server
```

也可以寫在 `~/Library/Application Support/KnowledgeBase/config.toml`：

```toml
member = "ellrin"
spaces = ["~/Documents/KnowledgeBase/private"]
reader_url = "http://127.0.0.1:7890"   # LoreReader：卡片來源是 PDF 時連到這裡（也可用 KB_READER_URL）
[intervals]          # 複習間隔（天），都可省略
first_check_days = 1
second_check_days = 3
maintenance_days = [30, 90]
maintenance = true
stale_after_days = 180
```

## 提醒：通知與每日 e-mail（設計 §4.5）

在網頁的「設定」（<http://127.0.0.1:7700/#/settings>）設定；存在 `~/Library/Application Support/KnowledgeBase/reminders.toml`（或 `$KB_HOME`），不在任何 repo 內。

- **到期通知**：時間預設 09:00、21:00。NeonDeck 的「knowledge」小工具到了時段會向伺服器認領，有到期卡片就跳通知；NeonDeck 沒開時，伺服器等 2 分鐘沒人認領就用 `osascript` 自己通知（可關）。同一時段只通知一次；Mac 睡著錯過，醒來補一次。
- **每日 e-mail 摘要**：到期的問題（答案在信末）、最近學到、今日數字；純文字＋HTML。Gmail：`smtp.gmail.com`、587、STARTTLS，帳號填完整地址，密碼用「應用程式密碼」，按「儲存密碼」：存到鑰匙圈（服務 `KnowledgeBase`、帳號 `smtp-password`）；鑰匙圈無法使用時改存 `credentials.toml`（設定資料夾內、只有你的帳號能讀），設定頁會顯示實際存在哪裡。`KB_SMTP_PASSWORD` 可暫時覆蓋。每天只寄一次（`reminders-state.json` 記錄寄出日期），錯過時間醒來補寄一次，失敗 15 分鐘後重試、一天最多 3 次。「預覽」與「寄測試信」不算當天那封。
- **登入時自動啟動**：`scripts/install-launchd.sh`（`--print` 只印 plist，`--uninstall` 移除）。會編譯 release、把執行檔複製到 `~/Library/Application Support/KnowledgeBase/bin/`，寫入 `~/Library/LaunchAgents/local.knowledgebase.kb-server.plist` 並載入。
- 網址路由：`#/node/<id>`、`#/new`、`#/review`、`#/timeline`、`#/graph`、`#/graph/<id>`（局部圖譜）、`#/settings`（NeonDeck 與 e-mail 的連結用）。

## 資料

- **personal space**（只有你）：`space.toml`、`nodes/`、`progress/<你>/`、`kinds.toml`、`apps.toml`、`tags.json`；圖片在 iCloud Drive（`KnowledgeBase/assets/`，sha256 命名）。範本在 `workspace-template/`。
- **household space**（家人共用）：放在 JELearner 共用 repo 的 `knowledge/`，`kind = "household"`，圖片轉 webp 存在 git。每個人的學習狀態在自己的 `progress/<成員>/`。
- 個人卡片**不會自動**進入家庭 space；在卡片上按「分享到…」才會複製一份（新 id，指向個人卡片的連結不帶過去）。
- SQLite 索引在記憶體中，每次啟動從檔案重建；工具列的「重建索引」可在手動改檔後重讀。

## 外觀

設定頁的「外觀」選配色，所有 App 共用（存在 `private/shared/ui.toml`）。卡片的閱讀頁用主題的紙面色。

## 起草摘要

卡片編輯器的「用模型起草摘要」（`POST /api/draft`）經過 `lore-llm`，任務 `kb-summarize-card`：先用本機模型（`config.toml` 的 `[engine]` 或 `KB_LLM_URL`），沒有時用 Claude。引用的 PDF 頁碼與論文標題一併放進提示，行尾斷開的連字號會接回去。草稿只回傳給編輯器，**不會自動儲存**，改過再存。回覆標明實際用的引擎；沒有任何引擎時回 503 並說明原因。

## 圖譜

類似 Obsidian 的知識網路圖（`web/graph.js`，純 canvas，沒有外部套件）。

- **全部／局部**：全部 = 所有卡片；局部 = 以一張卡為中心，深度 1–3 層（標題列的滑桿）。卡片頁按「在圖譜中查看」或網址 `#/graph/<id>` 直接開局部圖譜。
- **節點**：顏色 = 熟悉度（學習中、記得、久未複習、尚無紀錄），也可改成依領域、標籤群組（第一層）、種類或空間著色（固定 8 色，其餘歸「其他」）；大小 = 連結數；形狀 = 種類（概念 ●、方法 ■、指標／方程式 ◆、資料集 ▲、解剖 ⬢、論文 ⬟）。虛線空心圈 = 內文 `[[連結]]` 指到還沒建立的卡片，點它可直接建立。可開「標籤節點」看共同標籤（含階層 `a/b → a`）。
- **連結**：先備知識 = 橘色箭頭（A → B：A 需要先懂 B），對照 = 虛線，相關 = 細線，內文提到 = 最細，其他有方向的型別（是一種、屬於、例子、用到…）畫箭頭。每種型別可個別隱藏。
- **操作**：滑過節點會亮起它和鄰居、顯示摘要；點一下開側邊資訊（連入／連出、開啟卡片、局部圖譜）；⌘／Ctrl＋點直接開卡片；拖曳節點會固定位置，雙擊已固定的節點取消固定（未固定的則開卡片）；滾輪或雙指縮放、拖曳背景平移、「全部置中」（F）。搜尋框輸入即標出符合的節點，Enter 跳過去。
- **設定面板**（右上的滑桿圖示）：篩選種類、熟悉度、空間、標籤（勾上層 = 含所有下層）、建立日期區間、孤立節點；連結型別；著色方式、節點大小、文字出現門檻；力度（中心力、排斥力、連結距離、連結力，可重設）。設定存在瀏覽器 `localStorage`（`kb.graph`），篩選條件不保存。
- **看知識怎麼長出來**（▶）：依建立時間播放，卡片一張張長出來；下方時間軸可拖曳。
- **效能**：Barnes–Hut 四元樹排斥力、批次繪製、只畫畫面內的東西、依 `devicePixelRatio` 繪製；佈局穩定、分頁或檢視被隱藏時就停止計算。實測（headless Chrome、2× 螢幕）：360 張卡／1,000 條連結約 0.4 秒排好；2,000 張卡／6,000 條連結約 0.7 秒排好，拖曳時維持 60 fps。

## 熟悉度

建立 → 學習中（隔天問）→ 記得 → 3 天後再問 → 記得 → **記得了**。忘了或標「不熟」就從頭來。記得的卡片 30、90 天後再確認一次（可關），180 天沒碰過會變成「久未複習」，下次遇到時再問。

## API（摘要）

| | |
|---|---|
| `GET/POST /api/nodes`、`GET/PUT/DELETE /api/nodes/{id}` | 清單（`q`、`kind`、`tag`、`domain`、`state`、`space`、`since`/`until`、`sort`）、新增（必須指定 `space`；可選 `initial_state`：`learning`（預設）或 `known` ＝已經熟的詞，直接成為「記得」，第一次確認在 30 天後，關掉維持確認時不排）、編輯（`revision` 衝突回 409）、移到垃圾桶 |
| `POST /api/nodes/{id}/restore` | 從垃圾桶復原（最近一次刪除的那份，連同學習狀態）；網頁刪除後的「復原」 |
| `POST /api/scan` | 術語掃描：`{text, candidates?, record?}` → 位置（UTF-16）、卡片、狀態、是否第一次出現 |
| `GET /api/recall/due`、`POST /api/recall/{id}` | 到期卡片；作答 `remembered` / `forgot` / `unfamiliar` |
| `GET /api/recent`、`/api/timeline` | 最近、每天新增修改 |
| `GET /api/graph` | 圖譜：`center`＋`depth`（局部，預設 2，最多 6）、`space`、`limit`（預設 400 張最新的卡）、`full=true`（全部，上限 20,000）、`missing=true`（未建立的 `[[連結]]` 佔位節點，`kind = "missing"`）、`tags=true`（標籤節點）。節點含 `kind`、`state`、`score`、`streak`、`tags`、`aliases`、`domains`、`summary`、`created_at`、`updated_at`、`degree`、`hop`（局部圖譜的距離）；另回傳 `total`、`truncated`、`center` |
| `POST /api/assets?space=&ext=`、`GET /api/assets/{space}/{hash.ext}` | 圖片 |
| `GET /api/health` | 狀態、成員、`reader_url`（LoreReader 的位址，卡片來源 `type = "pdf"` 連到 `<reader_url>/open?sha=…&page=…`；`KB_READER_URL` 或 config.toml 的 `reader_url`，預設 `http://127.0.0.1:7890`） |
| `GET /api/spaces`、`/api/registry`、`POST /api/nodes/{id}/share`、`POST /api/reindex` | 空間、各 App 的登錄；複製到家庭 space（新 id）；重建索引 |
| `POST /api/draft` | 起草摘要（見上）：`{term, context, kind?, lang?, page?, source?, engine?}` → `{draft, engine, assistant}` |
| `POST /api/nodes/{id}/encounter` | 「遇到」：記一次出現，可附來源 `{via}` → `{progress}` |
| `GET/PUT /api/theme` | 共用配色（外觀） |
| `GET /api/stats` | 今日數字（今天新增、待複習、7 天內記住、各狀態張數）＋通知時間 |
| `GET/PUT /api/reminders`、`PUT /api/reminders/password` | 提醒設定、狀態、密碼是否已設定（密碼不會回傳） |
| `GET /api/reminders/preview?format=html\|text\|json`、`POST /api/reminders/test-email`、`POST /api/reminders/test-notify` | 預覽／測試 |
| `POST /api/reminders/claim` | `{by}` 認領目前的通知時段（NeonDeck 用；同一時段只有第一個拿到 `claimed: true`） |

CORS 只允許本機（`127.0.0.1`、`localhost`、`tauri://`、`file://`／`null`、NeonDeck 的 `deck://`）。

## 開發

```sh
cargo test                       # 伺服器 API 測試（離線；SMTP 用測試內的假伺服器，不碰鑰匙圈、不跳通知）
cd ../commons && cargo test -p kb-core -p lore-store
```

開發說明：程式碼由作者與 Claude Code 一起撰寫。
