# kb-core

知識庫核心（設計稿 §4）：檔案是唯一真相，SQLite 索引隨時可重建。

| 模組 | 內容 |
|---|---|
| `node` | 知識卡 JSON（§4.1）；未知欄位保留在 `extra`；`copies` 只存在個人 space |
| `id` | `n` + ULID（時間排序、同毫秒遞增） |
| `registry` | 內建種類／連結型別 + `kinds.toml` + `apps.toml`（App 宣告的種類與連結型別） |
| `tags` | `tags.json`：domains、標籤階層（`deep-learning/cnn`） |
| `progress` | 每人學習狀態（§4.2）與熟悉度狀態機（§4.4），間隔在 `Intervals` |
| `edges` | `[[wikilink]]` 解析；明確連結存在節點；索引保存雙向邊 |
| `index` | rusqlite（bundled，FTS5 `trigram` → 中日文可搜）：清單篩選、到期、最近、圖譜、時間軸 |
| `scan` | 術語掃描：Aho-Corasick（標題＋別名，不分大小寫、英文複數、CJK 不需字界），回傳 UTF-16 位置、狀態、是否第一次出現 |
| `llm` | `CardAssistant`（起草卡片、抽候選術語）＋ `MockAssistant`（僅測試）；`clean_source` / `draft_prompt` / `parse_draft`：真正的起草由 app 伺服器經 lore-llm 執行 |
| `kb` | `KnowledgeBase`：create / `create_as(space, fields, via, State::Known)`（新卡直接「記得」：`Progress::new_known`）/ update（revision+1、updated_at）/ trash / restore（從 `trash/` 復原最近一份）/ recall / encounter / share / scan / 查詢 / 資產 |
| `pending` | 待審區：每個 space 的 `pending/<id>.json`（`p`+ULID）。**不是卡片**：索引、清單、複習、搜尋都不讀。見下 |

### 待審區 API（LoreEcho 在 KnowledgeBase 沒開時直接呼叫）

只需要一個 `Space`（不必開 `KnowledgeBase`）；只寫 `pending/`（丟掉時移到 `trash/pending/`）：

```rust
let space = lore_store::Space::open(root)?;            // <private>/KnowledgeBase 的 space
let now = kb_core::SystemClock.now();
let cards = kb_core::pending::add(&space, now, vec![NewPending {
    title: "…".into(), summary: "…".into(), body: "…".into(), tags: vec![], kind: "concept".into(), // 空 = concept
    links: vec![/* Link { to: <既有卡 id>, rel: "related", .. } */], source: "對話標題".into(),
    from_conversation: Some("conv-id".into()), ..Default::default() }])?; // -> Vec<PendingCard>（含 id）
kb_core::pending::list(&space); get(&space, id); update(&space, id, NodeFields); discard(&space, id);
```

`KnowledgeBase` 上同樣有 `pending_add / pending_list / pending_get / pending_update / pending_discard`，另有
`pending_duplicates(space, &card)`（同標題或別名，不分大小寫）、`pending_accept(space, id, edits)`（成為正式卡，
狀態 learning，`sources` 記來源；目標不存在的連結丟掉、未知連結型別改 `related`、未知 kind 改 `concept`）、
`pending_merge(space, id, into_id, edits)`（併入既有卡：標題與別名成為別名、標籤／連結聯集、空摘要補上、不同內文接在後面）。
檔案欄位：卡片欄位（kind, title, aliases, lang, domains, tags, summary, body, links）＋ `source`、`created`、`from_conversation`。

## 規則

- 寫入一律指定 space；`share(id, to_space)` 是唯一的跨 space 路徑（複製、新 id、圖片複製到目標 store）。
- household 卡片的 `links[].to` 只能指向同一個 space 的卡片；household 不存 `copies`；分享時指向個人卡片的連結會被移除（目標已有副本時改指向副本）。
- 學習狀態只寫 `progress/<目前成員>/`；刪除只把檔案移到 `trash/`，`restore` 可移回。
- 狀態機：建立 → learning（+1 天）→ 記得 → streak 1（+3 天）→ 記得 → known（+30、+90 天維護，可關）；忘了／不熟 → streak 0、+1 天；known 180 天沒有活動 → stale，遇到時立刻到期。未到期就按「記得」只記錄、不推進（`count_early` 可改）。
- 時鐘可注入（`FixedClock`），測試完全確定。

```sh
cargo test -p kb-core
```
