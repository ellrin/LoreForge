# LoreForge 共同介面（定案 2026-11-11）

四個工具：PaperComposer（PC）、SlideComposer（SC）、LoreReader（LR）、KnowledgeBase（KB）。
原則：同一套產品；共同功能放在同一個位置；不常用的收進齒輪；數值以 SC 為準。

## A. 外框標準（每個位置一列）

| 位置 | 標準 | 現況差異 |
|---|---|---|
| 上方列 | 高 48px，左右 16px，間距 16px | KB 52px；LR 間距 8px |
| 內文字 | 14px；圖示鈕 32px | PC 13px；KB 15px、圖示鈕 36px |
| 窄視窗 | 1100px 變抽屜，760px 藏品牌字 | LR 640px；KB 900px |
| 上方列左→右 | ［窄：左抽屜］· 品牌（按了回首頁）· **文件選單 `名稱 ▾`** · **狀態標籤**（已儲存／儲存中／未儲存／同步中／未同步，存檔和同步合成一個）· 本工具的 2–4 個主要按鈕 · **AI（固定在主要按鈕最後一格）** … 右側最多 3＋2：**對照表 · 匯出 ▾（含列印）· 齒輪（永遠最右）**；復原／重做只在編輯文件時出現 | 見 B 表 |
| 搜尋 | ⌘F 四個都一樣；搜尋列固定從工具膠囊下方出現；KB 本來就是搜尋為主，保留上方的搜尋框 | PC、LR 位置不同；SC 沒有 |
| 視覺重點 | 每個畫面只有一個主色實心按鈕（PC 生成、KB 儲存；SC、LR 沒有） | — |
| 名稱與圖示 | 同一功能同一名稱同一圖示：對照表（不叫方程式與變數）、首頁（不叫封面／書庫）、另存新檔 | 目前三種叫法 |
| 第二列 | 浮在內容上方的圓角工具膠囊（只放本工具的編輯工具） | LR 是整排工具列；KB 沒有 |
| 左側 | 導覽（大綱／投影片／頁面縮圖／卡片篩選） | 已大致一致 |
| 右側 | 編號區塊 `01 名稱`，附 ? 和 ± | LR 是 6 個分頁；KB 沒有右側 |
| 狀態列 | 位置 · 模型 · 縮放（存檔狀態只在上方列） | LR、KB 沒有 |
| 首頁 | 一塊置中面板：`01 開始`（小磚）＋ `02 最近` | LR 是書庫卡片牆；KB 沒有首頁 |

KB 套用：上方列、齒輪、提示與詢問框、跨 app 連結、狀態列（只在編輯卡片時）。不套用：文件選單、復原。

## B. 功能 × 工具（有／位置 → 怎麼改）

| 功能 | PC | SC | LR | KB |
|---|---|---|---|---|
| 齒輪 | 有，最右 → 保留 | 有，倒數第二 → 移到最右、改成共用設定視窗 | **沒有 → 新增** | 在左側導覽 → 移到右上角視窗 |
| 外觀（主題、紙張） | 在齒輪裡 → 保留 | 齒輪＋太陽鈕 → **拿掉太陽鈕** | 太陽鈕 → **收進齒輪** | 設定頁 → 收進齒輪 |
| 模型（本機／Claude） | 齒輪＋右側「引擎」＋狀態列 | **沒有**（流程圖 AI 需要）→ 新增 | 工具列「模型」鈕 → 收進齒輪，「你的領域」移到「一般」 | **沒有** → 新增 |
| 對照表 | 只能從警告打開 → **上方列固定按鈕** | 有，上方列 → 保留 | 「符號」分頁 → 改用共用對照表，可「加入全域」 | 沒有 → 顯示全域對照表 |
| 狀態標籤 | 同步雲朵 → **併成狀態標籤** | 有 → 當標準 | 「頁面未儲存」→ 狀態標籤，**點它＝另存新檔（含筆記）**（LR 的儲存就是另存，private 是暫存區） | 「未儲存」→ 統一樣式 |
| 另存新檔 | 沒有 → 放進文件選單 | 在文件選單 → 保留 | 上方列按鈕 → 放進文件選單 | 不適用 |
| 文件選單（最近／開啟／新增） | 名稱只能回封面 → 改成選單 | 有 → 當標準 | 只有標題文字 → 改成選單（最近、書庫、加入 PDF） | 不適用（卡片沒有「文件」） |
| 匯出／列印 | 匯出有，沒有列印 → 加列印 | 匯出有，`print.html` 沒按鈕 → 加列印 | 列印、另存是兩顆鈕 → 併成匯出選單 | 沒有 → 卡片匯出 Markdown／列印 |
| 復原／重做 | 沒有（互補，段落層級） | 有 | 沒有（互補：註記的復原） | 只有刪除後的復原提示 |
| 搜尋／取代 | 有 | **沒有 → 新增**（搜整份簡報的文字） | 有搜尋 | 有搜尋 |
| ? 說明 | 有 | 有 | 沒有 → 新增 | 用另一種樣式 → 統一 |
| 快捷鍵清單 | 都沒有 → 齒輪裡加一頁，由按鈕標籤自動產生 | 同 | 同 | 同 |
| 縮放 | 狀態列 | 狀態列 | 上方列 → 移到狀態列 | 只有圖譜 |
| 同步（git） | 有 → 併進狀態標籤 | 資料在 git 時才出現同步狀態 | 同 | 同 |
| 提示訊息／詢問框 | 還有系統原生 `confirm()` → 改共用 | 自己的一套 | 自己的一套 | 還有原生 `confirm()` → 改共用 |
| 在其他 Lore app 開啟 | 開 LR 會跑到瀏覽器 | 同 | 只能用網址 | 有 `lore-kb://` → 四個都用 `lore-<app>://` |

## C. 共用程式（改一份，四個都跟著變）

| 檔案 | 內容 | 現況 |
|---|---|---|
| `commons/ui/icons.js` | 四套圖示合成一套 | 四份各自複製（65／99／108／55 個） |
| `commons/ui/base.css` | 按鈕、切換鈕、標籤、選單、視窗、提示、區塊標題、狀態列、抽屜 | 四份 `styles.css` 各寫一次 |
| `commons/ui/shell.js` | 上方列、文件選單、抽屜、齒輪視窗外殼、提示、詢問框、跨 app 連結 | 四份各寫一次 |
| 齒輪共用分頁 | 一般 · 外觀 · 模型 · 同步 · 快捷鍵，再加一頁本工具專屬 | PC 有分頁；SC 只有一頁；KB 是頁面；LR 沒有 |
| 模型設定 | 存在 `private/shared/`（跟主題一樣），所有 app 讀同一份；金鑰放同一個鑰匙圈項目；舊位置照讀、不刪 | 四個 app 各一份設定檔、各一個鑰匙圈項目 |

## D. 模型（齒輪 → 模型，一頁兩段）

**連線**（所有 app 共用，存在 `private/shared/`，跟主題一樣）：本機模型（目前使用、下載／切換）、Claude（訂閱或金鑰、偏好模型）。每個連線旁標「看圖 ✓／✗」。

**走向**（依任務類別，不依 app）：每列一個 `自動 ｜ 雲 ｜ 地`。

| 類別 | 預設 | 任務 |
|---|---|---|
| 創作與審稿 | 雲 → 地 | pc-compose-paragraph、pc-soft-check、pc-outline、pc-caption、pc-draft-plan／claims／review／outline／audit／revise／record、流程圖資料流、簡報設計建議 |
| 整理與格式 | 地 → 雲 | kb-summarize-card、kb-extract-terms、lr-translate、lr-classify-terms、lr-paragraph、pc-decompose、pc-cite-extract、pc-ref-queries、pc-ref-rank、pc-ref-notes、pc-template-rules |
| 問答 | 自動（有地用地，否則雲） | pc-draft-turn、lr-chat |

每個 `tasks.toml` 的任務加一個 `class =`；lore-llm 依共用設定和類別決定先後。個別功能旁的引擎選擇保留在功能旁，統一成同一個小標籤（預設「自動」，並顯示實際用了哪個）。

## E. 已定案的細節

- ⌘S＝儲存（四個都一樣）；同步在背景，狀態併進狀態標籤。
- LR 右側改成編號區塊：名詞、註記預設展開；符號、段落、引用句預設收起；問這篇是上方列的 AI 鈕。
- 同一篇論文在不同工具裡內容要一致：從 LR 帶進來的符號與方程式，自動登記到這份文件（論文或簡報）的對照表，id 沿用 LR 的 `symbols.json`；全域對照表不自動寫入。PC 目前只複製 LaTeX，要改成自動登記。

## F. 共用外殼的介面（`commons/ui`，2026-11-11）

示範：`cd commons/ui && python3 -m http.server`，開 `/demo/shell.html`（`?theme=<id>`，API 是假的）。
伺服器透過 `lore-shell` crate 提供四個檔：`lore_shell::FILES` → `/icons.js`、`/base.css`、`/shell.js`、`/makedeck.js`（「做成簡報」按鈕與卡片，`LoreMakeDeck.mount({ before, source, key })`，有論文的 app 掛在 AI 鈕前面：PaperComposer、LoreReader）。
頁面載入順序：`theme.css → base.css → styles.css`；`theme.js → icons.js → shell.js → app.js`。
app 自己的 `icons.js` 拿掉（兩份都宣告 `ICONS`）。`cargo test -p lore-shell` 檢查圖示沒有重複、四個 app 用到的名稱都在、base.css 只用 theme.css 的 token。

### 圖示（`icons.js`）

四套的聯集，一個圖形一個名稱；API 不變（`ICONS`、`data-icon`、`paintIcons`、`icon`、`iconButton`）。
同一功能畫法不同時用 SC（其次 LR）的畫法、名稱不變（file、pdf、save、image、star、moon、fit）。
舊名照用：`ICON_ALIASES`（export→download、list→listUl、listOrdered→listOl、panelRight→panel）。
同名不同意思要改呼叫處：`ICON_RENAMES`（SC／LR 的 z 順序 back→sendBack，LR insert→insertPage、quote→excerpt、merge→mergeFiles，PC 重新生成 redo→recast）。
共同位置的圖示：首頁 home · 文件選單 chevron · 對照表 book · 匯出 download · 列印 printer · 另存新檔 save · 開啟 folder · AI spark · 設定 gear · 復原 undo · 重做 redo · 搜尋 search · 同步 cloud／cloudOff · 抽屜 panelLeft／panel。

### 外殼（`shell.js`）

```js
const shell = LoreShell.mount({
  app, brand: "SLIDE/COMPOSER", home,                       // 品牌（按了回首頁）
  picker: { icon, name, recent: async () => [...], actions: [...] },   // 文件選單；最後自動加「首頁」
  status: { onClick(state) },                               // 狀態標籤可點時做什麼（LR：另存新檔）
  primary: [{ id, icon, label, run | menu }],               // 1–4 個；ai 固定在最後一格
  ai: { icon: "spark", label, run | menu },
  notation, export: [...含列印],                            // 右側：對照表 · 匯出▾ · 齒輪（固定，不收別的）
  undo: { undo, redo },                                     // 只在 shell.editing(true) 時出現
  drawers: { left, leftLabel, leftIcon, right, rightLabel },  // ≤1100px 變抽屜，≤760px 藏品牌字
  search, save,                                             // ⌘F、⌘S；⌘, 開設定
  general(el), look(el), sync(el), settingsTabs: [{ id, label, build }],   // 齒輪裡本工具的內容
  modelSetup(kind),                                         // 模型頁「下載…／登入…」交回 app（金鑰只在 app 裡處理）
  keys: [{ label, keys }],                                  // 沒有按鈕的快捷鍵
  host,                                                     // 要填的 <header>（預設在 body 最前面新建）
});
`resize: { left: "#sel", right: "#sel", defaults: { left, right }, min, max }`：左右面板內緣的拖曳把手（>1100px；游標 col-resize，滑過／拖曳時顯示細線，連點兩下還原，聚焦後 ←/→ 微調、Shift 加大、Home 還原）。寬度寫在 `<html>` 的 `--left-w` / `--right-w`，app 的 grid 用 `var(--left-w, 預設)`；每個 app 各自記在 localStorage `lore.panels.<app>`；每次變動在 document 送 `lore-resize`（LR、SC 的符合寬度跟著重排）。預設範圍 180–480px，`min`／`max` 可給數字或 `{ left, right }`。`shell.resize.set(side, px)` / `.reset(side)`。
shell.slots.lead / primary / right   // 給本工具的小東西（SC 字型標籤、KB 使用者）
shell.setName(n) · shell.setStatus(state, { label, title, action }) · shell.editing(on) · shell.setUndo({ undo, redo }) · shell.drawer(w, on) · shell.home(on)（首頁：只留品牌、抽屜鈕、齒輪）
LoreShell.toast(msg, { err, action: { label, run }, ms })
LoreShell.ask(anchor, lines, buttons) → Promise（值或 null；Esc、點外面＝null）
LoreShell.form({ title, build(body, dlg), actions: [{ label, primary, run }] }) → Promise（小表單對話框）
LoreShell.menu(anchor, items)（anchor＝元素，或右鍵的 `{ x, y }`；元素被隱藏時，例如首頁的文件鈕，改掛在品牌下；`LoreShell.place(el, anchor)` 給 app 自己的浮層用）· LoreShell.settings(tab) · LoreShell.block({ n, title, count, help, open, key })
LoreShell.engineChip({ task, value, used, onChange }) · LoreShell.shortcuts()
LoreLinks.open(app, params) · LoreLinks.url(app, params) · LoreLinks.ports
```

`<notation-panel>`（`commons/ui/notation-panel`）：顏色預設取主題 token（`--bg --text --muted --line --panel --accent-text --err --warn --accent-soft`），沒有 token 才用內建亮／暗色，仍可用 `--np-*` 覆寫；`readonly` 屬性（或單列 `entry.readonly`）＝不能改、不能刪（面板層級也不顯示「新增」）；`panel.actions = [{ id, label, title, show(e), disabled(e), run(e) }]` 每列加按鈕，並送出 `notation-action`。

狀態：`saved` 已儲存、`saving` 儲存中、`dirty` 未儲存、`syncing` 同步中、`unsynced` 未同步、`error`（app 給字，例如 SC「已在別處修改」）。
`dirty`／`unsynced`／`error` 可點（`status.onClick`），或每次 `setStatus` 自帶 `action`。
提示：帶按鈕的提示會留滿它的時間，一般提示疊在它下面（各自只取代同類的上一則），下一則提示不會把它蓋掉。
模型頁：`/api/engines` 沒回應（404）時，仍顯示 app 的「下載…／登入…」（`modelSetup`）。
提示、詢問框、選單放在開著的 dialog 裡（不然被齒輪視窗蓋住）。`.help[data-tip]` 由外殼統一處理，app 拿掉自己的。

快捷鍵頁由按鈕的 `aria-label` 產生：標籤最後一段是按鍵（`復原 ⌘Z`、`選取 V`、`粗體　Ctrl+B`、`關閉 Esc`），或放在括號裡（`復原「移動」(⌘Z)`，「…」會略過）。括號裡只認真正的按鍵（⌘⇧⌥⌃、`Ctrl+B`、`F5`、`Esc`、`↵`）；單一字母、數字或說明文字不算，說明請放 `title`。
例外用 `data-keys="⌘K"`；分組依位置（上方列、工具、左側、右側、狀態列、其他），或 `data-keys-group`。

### 跨 app 連結（`LoreLinks`）

| app | 連結 | http 備援（開發用預設埠） |
|---|---|---|
| pc | `lore-pc://open?path=<檔案>` | `http://127.0.0.1:7878/?open=<檔案>` |
| sc | `lore-sc://deck/<名稱>` | `http://127.0.0.1:7880/?deck=<名稱>` |
| lr | `lore-lr://paper/<sha>?page=<n>` | `http://127.0.0.1:7890/?paper=<sha>&page=<n>` |
| kb | `lore-kb://node/<id>` | `http://127.0.0.1:7700/#/node/<id>` |

`lore-<app>://` 不帶路徑＝開那個 app 的首頁。`open()` 依序：桌面版注入的 `window.LoreDesktop.openUrl(url)` →
本 app 伺服器的 `POST /api/links/open {url}`（只接受 `lore-(pc|sc|lr|kb)://`，交給系統開啟，成功 204）→ 新視窗開 http 備援。
桌面版用的是空閒埠：各桌面 app 綁埠時把 `{port, pid}` 寫進 OS 快取資料夾的 `LoreForge/ports.json`（lore-home `ports`，死掉的 pid 忽略），
`GET /api/links/ports`（lore-engines）回傳還在跑的 `{ pc, sc, lr, kb }`，`LoreLinks` 啟動時取一次並覆寫 `LoreLinks.ports`
（`LoreLinks.live` ＝ 實際在跑的、`refresh()`、`ready`）。`LoreLinks.ensure(app, { timeout, onWait })`：沒在跑就用 scheme 開起來，等埠出現（預設 20 秒），回傳埠或 null。
四個桌面 app 的 Info.plist 都註冊了 `lore-<app>://`，`RunEvent::Opened` 把它轉成頁面（`/?open=`、`/?deck=`、`/?paper=&page=`、`/#/node/`）。

### 模型：`GET/PUT /api/engines`

設定存在 `private/shared/engine.toml`（lore-home `Home::engine()`／`set_engine_model()`：`[engine]` 只存模型 id；正在跑的 llama-server 是執行期資料，見 `docs/local-models.md`），建議加兩個表：

```toml
[engine]            # 已有：本機模型
url = "http://127.0.0.1:8080/v1"
model = "qwen3.5-9b-q8"

[claude]            # 建議：Claude 連線（不放金鑰；金鑰在鑰匙圈）
via = "subscription"   # subscription | api
model = "sonnet"       # catalog 別名

[route]             # 建議：依任務類別的走向
create = "auto"     # auto | cloud | local
tidy = "auto"
ask = "auto"
```

| 類別 id | 名稱 | 自動的先後 |
|---|---|---|
| `create` | 創作與審稿 | 雲 → 地 |
| `tidy` | 整理與格式 | 地 → 雲 |
| `ask` | 問答 | 地 → 雲（本機可用就用本機） |

走向：`auto`＝上表的先後，第一個不能用就換另一個；`cloud`＝只用 Claude；`local`＝只用本機（不能用就報錯，不換）。
功能旁的引擎標籤是 app 自己的覆寫（存在 `private/<App>/`，例如 LR 的 `models.json`），優先於類別走向；`auto`＝跟類別走。

`GET /api/engines` →

```json
{
  "local":  { "state": "ready", "url": "http://127.0.0.1:8080/v1", "model": "qwen3.5-9b-q8", "label": "Qwen 3.5 9B", "vision": false,
              "models": [{ "id": "qwen3.5-9b-q8", "label": "Qwen 3.5 9B", "vision": false }] },
  "claude": { "state": "ready", "via": "subscription", "vias": { "subscription": true, "api": false }, "model": "sonnet", "vision": true,
              "models": [{ "id": "sonnet", "label": "Sonnet", "vision": true }] },
  "classes": [{ "id": "create", "label": "創作與審稿", "route": "auto", "auto": ["claude", "local"] }],
  "tasks": [{ "id": "pc-draft-turn", "label": "討論草稿", "class": "ask", "last": "local:qwen3.5-9b-q8" }],
  "overrides": { "pc-draft-turn": "auto" }
}
```

`local.status` 的字：`未載入 · 用到時自動載入`、`已載入 · <app、app> 正在用`、載入／切換／等待（§4）。`claude.vision`＝只有走金鑰（`via=api`）且有金鑰才 ✓，訂閱 ✗（標題「訂閱不能傳圖」）。
`tasks[]` 另有 `last_label`（「本機 Gemma 4 12B」）和 `last_at`（Unix 秒）；模型頁每個走向列顯示該類別最近一次「上次：… · N 分鐘前」（只算本 app 的任務，記在行程內）。
`engineChip({ live: true })`：送出前顯示下一次會用的（自動 · 本機 Gemma 4 12B，依覆寫／類別走向／可用性），回答後 `chip.actual(名稱)` 顯示實際用的。

`state`：`ready` 可用、`starting` 啟動中、`off` 未連線、`missing` 未安裝、`error`。`vision`＝能看圖。`tasks` 只列本 app 的任務；`last`＝上次實際用的模型（標籤顯示用）。
`PUT /api/engines` 送要改的部分：`{ local: { model }, claude: { via, model }, routes: { create: "cloud" }, overrides: { "<task>": "local" } }`，回傳完整的 GET 內容；
值不對 → 400 `{ "error": "一行中文" }`。沒有這個路由（404）時模型頁只顯示「—」。登入、輸入金鑰、下載模型不經過這個介面（`modelSetup` 交回 app）。

## 順序

先做外殼再補功能：互補功能需要外殼先把位置留好。

1. 一個子代理先做共用外殼（C 表），其他人等它。
2. 四個子代理同時把四個 app 換上外殼（各管各的 app）。
3. 互補功能（PC／LR 復原、SC 搜尋、LR／KB 狀態列、列印）各開一個子代理。
4. 流程圖 AI 的入口放在 SC 標準外殼裡的固定位置。
