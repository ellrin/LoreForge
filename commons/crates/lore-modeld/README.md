# lore-modeld

共用的本機模型管家（設計見 `docs/loreecho.md` §8）。整台電腦只有它開 llama-server；每個 App 不再自己開，
而是把本機模型的請求送給它（一樣是 OpenAI 相容介面），由它決定誰先。

```toml
lore-modeld = { path = "../commons/crates/lore-modeld" }   # 只有要自己架管家（或測試）才需要；App 用 lore-llm 的 broker
```

```sh
cargo build -p lore-modeld          # target/debug/lore-modeld
lore-modeld [--home <LoreForge 資料夾>] [--gguf <dir> --runtime <dir>]
            [--linger <秒>] [--slots <n>] [--promote-after <秒，0 = 不升級>]
```

- 只聽 `127.0.0.1`，隨機空 port；把 `{port, pid}` 寫在 `<快取資料夾>/modeld.json`（和 `ports.json` 同一個資料夾，
  `LOREFORGE_CACHE` 可改）。啟動時若檔案裡的 pid 還活著、`GET /api/status` 也回應，就安靜結束（印出那一個的網址）。
  同時啟動兩個時，用 `modeld.lock` 排隊，只會留下一個。
- 擁有**一個** `lore_llm::local::LocalRuntime`（沿用它開 llama-server、`/slots` 判斷忙碌、閒置釋放）；
  閒置時間跟著共用設定 `private/shared/engine.toml` 的 `[local] idle_minutes`（`lore-engines` 的 `idle_minutes`）。
- 模型資料夾：`--gguf` + `--runtime`（某個 App 自己的資料夾），否則 `LOCAL_LLM_HOME`。
- 模型已釋放、沒有請求、而且 `--linger`（預設 5 分鐘，`DEFAULT_LINGER`）內沒有任何 App 跟它說話，就自己結束並刪掉 `modeld.json`。
  `SIGTERM`／Ctrl-C 也一樣乾淨結束（llama-server 跟著關）。

## API

| 方法 | 路徑 | 說明 |
|---|---|---|
| `POST` | `/v1/chat/completions` | OpenAI 相容。`model` = 模型目錄的 id；管家確保這個模型已載入再轉給 llama-server。串流（SSE）逐塊轉送，非串流原樣回傳。錯誤用 OpenAI 的 `{ "error": { "message" } }`（缺 `model` 400、模型不能啟動 503、llama-server 沒回應 502） |
| `GET` | `/v1/models` | 可用的模型（模型資料夾裡已下載的），`loaded` 標出目前載入的 |
| `POST` | `/api/focus` | `{ "app": "pc" }`：目前在前景的 App（`null` = 都不是）；App 在視窗取得焦點時呼叫 |
| `GET` | `/api/status` | `{ service: "lore-modeld", pid, loaded, phase, focused, queue: [{ app, priority, effective, model, waiting_ms }], running, running_apps, slots }`；`queue` 依將要執行的順序 |
| `POST` | `/api/release` | 立刻釋放模型（204）；有請求在跑時拒絕（409 `{ error }`） |

請求標頭：

| 標頭 | 值 |
|---|---|
| `X-Lore-App` | `pc`、`sc`、`lr`、`kb`、`le`…（任何短名都可以；沒有 = `unknown`） |
| `X-Lore-Priority` | `interactive`（有人在等：對話、語音）或 `background`（整理、翻譯、批次審稿）；沒有或不認得 = `background` |

## 排程規則（`sched`，純函式、沒有 I/O）

1. 前景 App 的請求最先（不論互動或背景）。
2. 其次互動式先於背景。
3. 同等級先來先服務。

- 同時最多跑 `slots` 個（預設 1，`DEFAULT_SLOTS`）。
- 正在跑的背景請求**不中途砍掉**（這一階段沒有取消）；讓位發生在兩個請求之間。
- 換模型（`swap_decision`）：
  - 要的就是目前載入的模型 → 不換（`Keep`）。
  - 沒有模型載入 → 直接載入（`Load`；載入空的記憶體不算換）。
  - 背景請求要換掉已載入的模型，而前景 App 或任何互動式請求正在等 → 不換（`Blocked`），先讓後面能跑的請求跑。
  - 要換模型時，等正在跑的請求都結束（`WaitForRunning`），而且排在它後面的請求不能插隊（不然永遠換不成）。
- 防餓死：背景請求等超過 `promote_after`（預設 **2 分鐘**，`DEFAULT_PROMOTE_AFTER`）升一級，當作互動式
  （不會升成前景 App 的請求）；升級後也算互動式，可以換模型。

## 檔案

| 檔案 | 內容 |
|---|---|
| `src/sched.rs` | `Scheduler`（`enqueue`、`next(now)`、`done`、`set_focus`、`set_loaded`、`order`、`effective`）、`swap_decision`、`Config` |
| `src/server.rs` | `Broker`（排程＋上游）、`router(broker)`、`Ticket`（請求結束或用戶端離開時釋放位置） |
| `src/upstream.rs` | `trait Upstream { ensure, loaded, release, phase, models }`；`LocalUpstream`（真的 `LocalRuntime`） |
| `src/lib.rs` | `serve(Options, upstream, shutdown) -> Outcome::{AlreadyRunning(url), Stopped}`、`DEFAULT_LINGER` |
| `src/main.rs` | `lore-modeld` 執行檔 |
| `tests/broker.rs` | 假的 llama-server（axum：`/v1/models`、`/v1/chat/completions` 串流與非串流、`/slots`）＋假上游 |

App 端（找到或啟動管家、帶標頭、`focus`）在 `lore-llm` 的 `lore_llm::broker`。

## 測試

`cargo test -p lore-modeld`：排程的每條規則（單元測試）；經過假 llama-server 的轉送（串流逐塊、非串流、錯誤）、
請求到達順序（前景 → 互動 → 背景、跑到一半不被砍）、換模型等正在跑的請求、釋放 409／204、用戶端放棄時離開佇列、
單一實例與閒置結束（暫存資料夾的 state 檔）、真的執行檔用 `LOREFORGE_CACHE` 第二次啟動會安靜結束。

## Not tested on real hardware

- 用真的 llama-server 透過管家載入、換模型、閒置釋放（`LocalUpstream` → `LocalRuntime`；測試只用假的上游）。
- 多個 `slots`（llama-server `--parallel`）：`LocalRuntime` 目前用預設的 slot 數啟動。
- Windows 上的 detached 啟動與 `pid_alive`；macOS 上由 App bundle 啟動管家。
