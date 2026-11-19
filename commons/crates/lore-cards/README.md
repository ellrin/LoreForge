# lore-cards

加入知識庫 from any Lore app, whether L-Brain is open or not.

| API | 用途 |
|---|---|
| `Brain::system(&home)` | 這台電腦的 L-Brain：ports.json 的 `kb` 或 127.0.0.1:7700；空間與成員照 L-Brain 自己的讀法（`KB_SPACES`、它的 config.toml、lore-home） |
| `Brain::at(url, spaces, member)`、`Brain::none()` | 固定位址與空間（測試、`KB_URL`）；完全沒有 L-Brain（隔離範例） |
| `brain.create(body)` | 新卡，body 與 L-Brain `POST /api/nodes` 相同（沒寫 `space` = 個人空間）→ `{ node, via: kb \| file }` |
| `brain.recall(id, outcome, via)`、`view(id)`、`update(id, body)` | 記得／忘了／不熟、讀一張卡、修改（帶 `revision`，改過了 → 409） |
| `search::summary(host, term)` | L-Reader 一鍵加卡的摘要：一定要上網查（task `lore-card`，只用 Claude），沒查到來源或 `found: false` 就不建卡；只送字詞和作者的領域 |
| `router(brain, host, via)` | `POST /api/lore/card {title, source?}`：上網查 → 摘要 → 卡（學習中），給 `commons/ui/addcard.js` 的 `LoreAddCard.add` |

規則：L-Brain 有回應就只走它的 API；沒有在聽才直接用 kb-core 寫它的資料夾（L-Brain 下次開啟時從檔案重建索引）。
L-Brain 回應錯誤或逾時是錯誤，不會改寫檔案（同時只有一個寫入者）。兩條路都不做 git 同步（L-Brain 本身也不做）。
`source` 只放文件標題，不放路徑；文件內容不送給模型。

`cargo run -p lore-cards --example offline_card -- <空資料夾> <標題>` 在合成的空間寫一張卡，再用
`cargo run -p kb-server --example isolated_server -- <同一資料夾> <port>` 在 L-Brain 裡看到它。
