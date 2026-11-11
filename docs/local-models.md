# 本機模型共用（所有 Lore app 的共同規則）

目的：同一台電腦上，所有 Lore app（PaperComposer、SlideComposer、LoreReader、KnowledgeBase、LoreEcho、之後的 app）
用本機模型時，**同一時間只載入一個模型、只佔一份記憶體**。

做法：**不是**一個獨立的管家程式，而是 `lore-llm` 裡的一個模組（`shared_runtime`），每個 app 都內建、遵守同一套協定。
沒有任何 app 是「主人」；誰需要誰開，最後一個用完的人關。取代 `docs/loreecho.md` §8 的「獨立程序」提案（2026-11-11 作者改定）。

作者定案：**一次只留一個模型**（選項 a）。換模型要等目前的模型沒有請求在跑。

## 1. 資料放哪裡

| 內容 | 位置 | 分層 |
|---|---|---|
| 選的模型 id、閒置釋放分鐘數、走向 | `private/shared/engine.toml`（已有：`[engine] model`、`[local] idle_minutes`、`[route]`） | 工具設定（private） |
| 正在跑的 llama-server、誰在用、誰在等 | `<cache>/LoreForge/local-model.json`＋`local-model.lock`（`LOREFORGE_CACHE` 可改；和 `ports.json` 同一個資料夾） | 執行期（可刪的快取） |

`engine.toml` 的 `[engine] url` 不再寫入（網址是執行期資料，不屬於 private）；舊檔裡有的照讀一次後忽略。

## 2. `local-model.json`

```json
{
  "server": {
    "model": "qwen3.5-9b-q8",
    "url": "http://127.0.0.1:51234/v1",
    "pid": 4321,
    "started_by": { "app": "pc", "pid": 1200 },
    "since": 1794363123,
    "started_at": "Wed Nov 11 20:15:02 2026",
    "vision": false,
    "loading": true,
    "from": "gemma-4-12b-q8"
  },
  "leases":  [ { "app": "le", "pid": 1500, "since": 1794363200, "last_used": 1794363300, "priority": "interactive", "active": 1 } ],
  "wanting": [ { "app": "pc", "pid": 1200, "model": "gemma-4-31b-q4", "priority": "background", "since": 1794363310 } ],
  "last_activity": 1794363300
}
```

- `loading`（載入權重中，只在這段期間出現）、`from`（換掉的模型；有它就是「切換」而不是第一次「載入」）、`active`（這個程序正在跑的請求數）沒有值時不寫。換模型除了問 `/slots`，也看所有 lease 的 `active`。
- 所有讀寫都在 `local-model.lock` 的 `flock` 之下（持有者死掉時作業系統自動放開）。只支援 macOS（之後再談其他平台）。
- 每次讀取先清掉死掉的 pid（lease、wanting：`kill -0`）。`server` 另外驗證：`/health` 有回應**而且**該 pid 的指令是 llama-server、啟動時間和登記的一致（`ps -o comm=,lstart=`）；任一不符就當作已死、只刪登記，**絕不對沒驗證過的 pid 發訊號**。
- 檔案壞掉 → 當作空的，並用 `/health` 探測 `server.url`，還活著就補回。

## 3. 協定

**acquire(model, priority) → url**
1. 鎖住、清理。
2. 載入的就是這個模型而且活著 → 新增或更新自己的 lease，回傳 url。
3. 沒有在跑 → 自己啟動 llama-server（加 `--alias <模型 id>`，`/v1/models` 回報 catalog id，檔案壞掉時用來補回），登記 `server`（`loading: true`），等 `/health` 回 200 再加 lease、回傳 url；等待期間不持有鎖。不知道模型資料夾的 app 只能用別人開好的 server，輪到它啟動時回報錯誤。啟動規則：`setsid`（脫離 app 的程序群組，app 結束它不會跟著死）；stdout/stderr 寫到 `<cache>/LoreForge/llama-server.log`（每次啟動清空）；執行檔和模型檔從共用的 `LOCAL_LLM_HOME` 位置（`Layout::shared`）找，所以**任何 app 都能啟動**。下載模型仍只在 PaperComposer。
4. 載入的是別的模型 → 加入 `wanting`，解鎖，等待（下一節）。

**換模型（一次只留一個）**
- 只有在目前模型**沒有請求在跑**（問 llama-server `/slots`：沒有 slot 在處理）時才換。
- 等待順序：互動請求 → 背景請求；同級先來先服務（兩個互動請求要不同模型時，先到的先換）。背景請求等超過 120 秒升為互動級，不會一直餓著。（「前景 app 優先」第一版不做。）
- 一個模型載入後至少留 60 秒（最短駐留）。
- **保留期**：任何 lease 的 `last_used` 在 120 秒內（`hold_seconds`），對**背景**等待者來說這個模型仍算「在用」；互動等待者只要求沒有請求在跑。避免聊天和大模型每分鐘來回換。
- 輪到的人負責：停掉舊的 llama-server（用登記的 pid，先 SIGTERM，5 秒後 SIGKILL）、啟動新的、登記、移出 `wanting`。其他等待者每 0.5–2 秒（漸增）重查。
- 會自動換模型的情況只有兩種：該任務有指定別的模型（覆寫），或任務需要看圖而目前模型不能看圖（換成 `engine.toml` 的 `[engine] vision_model`；沒設就用模型目錄裡最小、能看圖的本機模型；都沒有就回報錯誤並附「改用 Claude」）。其他時候大家都用共用的預設模型，不會換、不會等。

**優先級怎麼決定**（`RouteOptions.priority`，預設依任務類別）：`ask` → 互動；`create`／`tidy` → 背景，但呼叫端有人在畫面上等結果時（生成按鈕、討論草稿的步驟、問這篇）傳 `interactive`。

**release**
- 每個請求結束更新 `last_used` 和 `last_activity`。
- app 結束時刪掉自己的 lease（當掉的 app 由下一次清理刪掉）。
- **最後一個活著的 lease 持有者結束時**，順便停掉 llama-server。

**閒置釋放**
- 任何一個 app 都會定期（每 30 秒）檢查：沒有請求在跑、`last_activity` 超過 `idle_minutes` → 停掉 llama-server、清掉 `server`。不是只有啟動它的那個 app 才會檢查。
- 所有 app 都當掉、留下孤兒 llama-server：下一個開啟的 app 清理時發現沒有活著的 lease 且閒置超時 → 停掉它。

**伺服器在用的時候死掉**：持有 lease 的請求收到連線拒絕 → 重新 acquire 一次（會重開），再失敗才回報錯誤。

## 4. 介面回饋（每個 app 用同樣的字）

引擎標籤、進度畫面、齒輪 → 模型都顯示同一組狀態：`載入模型中 · N 秒`、`等待 <app> 用完模型`、`切換模型中 · N 秒`、`已載入 <模型>`、`未載入`。不能只是轉圈。

## 5. 各 app 要改的地方

- **PaperComposer**：不再擁有模型。拿掉 `LocalRuntime` 的擁有權、`on_phase` 發布網址、結束時的 `kill_runtime`、`models::start/stop` 的直接啟停；下載模型照舊（在 PaperComposer 的齒輪 → 模型）。「選擇本機模型」只寫 `engine.toml` 的 model id。
- **SlideComposer、LoreReader、KnowledgeBase、LoreEcho**：本機請求一律經過 `acquire`，不再讀 `engine.toml` 的 url。
- **lore-engines（齒輪 → 模型）**：本機狀態從 `local-model.json` 讀（目前載入的模型、誰在用、誰在等）；「釋放」＝沒有其他 app 在用時才能按，否則顯示「X 正在用」。
- **lore-modeld**：獨立程式與 `lore-llm` 的 `broker.rs` 退役；`sched.rs` 的優先順序與升級邏輯移進 `shared_runtime`。`docs/loreecho.md` §8、§9.4、§10 第 1 階段改成指向本文件，`commons/CLAUDE.md` 的 crate 清單更新。

## 6. 舊東西怎麼對應

| 現有 | 之後 |
|---|---|
| `LocalRuntime`（每個程序一個 llama-server） | 只剩「啟動／停止一個 llama-server」的工具函式，由 `shared_runtime` 呼叫 |
| `LocalRuntime` 的閒置釋放、`watch_idle`、`release`（JR 加的） | 搬進 `shared_runtime`，任何 app 都會檢查 |
| PaperComposer 的 `on_phase` 發布網址、`Home::engine()/set_engine` 寫 url | 移除；`engine.toml` 只存 model id（舊 url 讀一次後忽略） |
| PaperComposer 的 `models::start/stop`、結束時 `kill_runtime` | start/stop 改呼叫 `shared_runtime`；結束時只刪自己的 lease（最後一個才停 server） |
| lore-engines `can_release`、`[local] idle_minutes` | `can_release` ＝沒有其他活著的 lease；idle 設定不變 |
| `local:current` | ＝ `engine.toml` 選的共用預設模型 id，用到時 acquire |
| lore-modeld、`broker.rs` | 退役 |

## 7. 不變式（維護時檢查）

- 同一時間最多一個 `server`。
- 清理後，每個 lease／wanting 的 pid 都還活著。
- 有 `server` ⇒ `/health` 有回應，否則下次清理就刪掉。
- 沒有任何 app 從 `private/` 讀模型網址。
- 範圍：只管 llama-server 模型；LoreEcho 的語音（Apple 內建）不受影響。

## 8. 選 (a) 的代價（作者已接受）

- PaperComposer 為某個任務載入另一個大模型時，LoreEcho 的對話會等它用完，或在它用完後才換回來。
- 換模型需要時間（大模型可能幾十秒），所以等待畫面一定要有進度。

## 9. 測試

用一個假的 llama-server（回應 `/health`、`/slots`、`/v1/models`、`/v1/chat/completions`，可模擬忙碌）和真的子程序：
- 兩個程序共用同一個 server；
- 啟動者結束，另一個照樣用；最後一個結束時 server 被停掉；
- 第三個要別的模型：slot 忙時等待，空了才換；互動請求排在背景請求前；背景請求等太久會升級；最短駐留生效；
- 非啟動者也能做閒置釋放；
- 鎖的持有者死掉、pid 殘留、檔案損毀、server 中途死掉後重新 acquire。

真的 llama-server 一律標 **not tested on real hardware**。
