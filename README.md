# KnowledgeBase

本機運作的個人知識庫：一張知識卡一個 JSON 檔，記錄「我記不記得」，讓其他 App（PaperComposer、SlideComposer、JELearner、NeonDeck 小工具）查詢術語、提醒複習。設計見 `../../docs/ecosystem-design.md` §4。

> 開發中（第 1 波 A1：知識庫核心）。已完成：`ym-store`（space、iCloud／git 圖片）、`kb-core`（知識卡、連結、標籤、學習狀態機、SQLite/FTS5 索引、術語掃描）、kb-server API、最小網頁介面（清單、編輯器、複習、時間軸、圖譜）。
> 尚未完成：Tauri 桌面 App、接上 `ym-llm`（目前起草卡片與候選術語用離線 mock）、NeonDeck 小工具／通知／Email 摘要（第 2 波）、家庭 space 的實際掛載。

## 執行

需要 [Rust](https://rustup.rs)（1.85 以上）。

```sh
cargo run -p kb-server
```

開啟 <http://127.0.0.1:7700>（只監聽本機）。預設掛載旁邊的 `../private`（你的個人 space）。

其他 space 或使用者：

```sh
cargo run -p kb-server -- --space ~/path/to/space --space ~/JELearner/private/knowledge --member ellrin
KB_SPACES=/a:/b KB_MEMBER=ellrin PORT=7700 cargo run -p kb-server
```

也可以寫在 `~/Library/Application Support/KnowledgeBase/config.toml`：

```toml
member = "ellrin"
spaces = ["~/Documents/KnowledgeBase/private"]
[intervals]          # 複習間隔（天），都可省略
first_check_days = 1
second_check_days = 3
maintenance_days = [30, 90]
maintenance = true
stale_after_days = 180
```

## 資料

- **personal space**（只有你）：`space.toml`、`nodes/`、`progress/<你>/`、`kinds.toml`、`apps.toml`、`tags.json`；圖片在 iCloud Drive（`KnowledgeBase/assets/`，sha256 命名）。範本在 `workspace-template/`。
- **household space**（家人共用）：放在 JELearner 共用 repo 的 `knowledge/`，`kind = "household"`，圖片轉 webp 存在 git。每個人的學習狀態在自己的 `progress/<成員>/`。
- 個人卡片**不會自動**進入家庭 space；在卡片上按「分享到…」才會複製一份（新 id，指向個人卡片的連結不帶過去）。
- SQLite 索引在記憶體中，每次啟動從檔案重建；工具列的「重建索引」可在手動改檔後重讀。

## 熟悉度

建立 → 學習中（隔天問）→ 記得 → 3 天後再問 → 記得 → **記得了**。忘了或標「不熟」就從頭來。記得的卡片 30、90 天後再確認一次（可關），180 天沒碰過會變成「久未複習」，下次遇到時再問。

## API（摘要）

| | |
|---|---|
| `GET/POST /api/nodes`、`GET/PUT/DELETE /api/nodes/{id}` | 清單（`q`、`kind`、`tag`、`domain`、`state`、`space`、`since`/`until`、`sort`）、新增（必須指定 `space`）、編輯（`revision` 衝突回 409）、移到垃圾桶 |
| `POST /api/scan` | 術語掃描：`{text, candidates?, record?}` → 位置（UTF-16）、卡片、狀態、是否第一次出現 |
| `GET /api/recall/due`、`POST /api/recall/{id}` | 到期卡片；作答 `remembered` / `forgot` / `unfamiliar` |
| `GET /api/recent`、`/api/timeline`、`/api/graph` | 最近、每天新增修改、圖譜 |
| `POST /api/assets?space=&ext=`、`GET /api/assets/{space}/{hash.ext}` | 圖片 |
| `GET /api/spaces`、`/api/registry`、`POST /api/nodes/{id}/share`、`POST /api/draft`、`POST /api/reindex` | |

CORS 只允許本機（`127.0.0.1`、`localhost`、`tauri://`、`file://`／`null`，NeonDeck 的 WKWebView 用得到）。

## 開發

```sh
cargo test                       # 伺服器 API 測試（離線）
cd ../../commons && cargo test -p kb-core -p ym-store
```

開發說明：程式碼由作者與 Claude Code 一起撰寫。
