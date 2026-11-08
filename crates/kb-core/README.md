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
| `llm` | `CardAssistant`（起草卡片、抽候選術語）＋ `MockAssistant` — **ym-llm 整合點** |
| `kb` | `KnowledgeBase`：create / update（revision+1、updated_at）/ trash / restore（從 `trash/` 復原最近一份）/ recall / encounter / share / scan / 查詢 / 資產 |

## 規則

- 寫入一律指定 space；`share(id, to_space)` 是唯一的跨 space 路徑（複製、新 id、圖片複製到目標 store）。
- household 卡片的 `links[].to` 只能指向同一個 space 的卡片；household 不存 `copies`；分享時指向個人卡片的連結會被移除（目標已有副本時改指向副本）。
- 學習狀態只寫 `progress/<目前成員>/`；刪除只把檔案移到 `trash/`，`restore` 可移回。
- 狀態機：建立 → learning（+1 天）→ 記得 → streak 1（+3 天）→ 記得 → known（+30、+90 天維護，可關）；忘了／不熟 → streak 0、+1 天；known 180 天沒有活動 → stale，遇到時立刻到期。未到期就按「記得」只記錄、不推進（`count_early` 可改）。
- 時鐘可注入（`FixedClock`），測試完全確定。

```sh
cargo test -p kb-core
```
