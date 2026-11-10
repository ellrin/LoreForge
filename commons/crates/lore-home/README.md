# lore-home

LoreForge App 找私人資料的地方。

```
LoreForge/
  private-example/<App>/    公開的最小範本（App 第一次啟動時複製）
  private-example/shared/   共用設定的範本
  private/<App>/            這個 App 的私人資料（自己的 private repo）
  private/shared/           所有 App 共用的個人設定（profile.toml、fonts.toml、…）
```

| API | 用途 |
|---|---|
| `Home::discover(path)` | 從 App 內的任一路徑往上找 LoreForge（`LOREFORGE_HOME`、`LOREFORGE_PRIVATE` 可覆寫） |
| `home.seed_app("App")` | `private/App`；不存在時從 `private-example/App` 複製（只補缺檔，不覆蓋） |
| `home.shared_file("x.toml")` | `private/shared/x.toml`，沒有才用範本 |
| `home.prefs()` | `Profile`（`id`、`name`）與 `Fonts`（`ui.sans/serif/mono`、`fallback`） |
| `fonts.css()` | 介面字型的 CSS 變數（`--sans`、`--serif`、`--mono`），App 在 `/prefs.css` 提供 |

## 主題（`lore_home::theme`）

所有 App 共用一個配色，選擇存在 `private/shared/ui.toml`（`theme = "<id>"`），在任何一個 App 改了，其他 App 回到前景時就跟著換。
配色表在 `commons/ui/theme.css`（20 組：深／淺各 10 組，每邊 5 組單色＋5 組搭配色，與 JELearner 相同），頁面端在 `commons/ui/theme.js`。

| API | 用途 |
|---|---|
| `theme::CSS`、`theme::script(&home)` | App 的 `/theme.css`、`/theme.js`（後者前面帶目前的選擇，第一次繪製就是對的顏色） |
| `home.theme()`、`home.set_theme(id)` | `GET`／`PUT /api/theme` 讀寫的值；沒選過是 `None`（頁面顯示 `theme::DEFAULT`） |
| `theme::THEMES`、`theme::nearest(id, light)` | 合法的 id；舊版的 orange／night／terminal 換成最接近的主題 |

頁面：`<link rel="stylesheet" href="theme.css">`、`<script src="theme.js"></script>` 放在 `<head>`，設定裡用 `LoreTheme.picker(元素)`。
測試檢查三處的 id 一致，以及 `theme.css` 開頭寫的對比規則。

## 本機模型（`lore_home::engine`）

所有 App 共用的本機模型伺服器存在 `private/shared/engine.toml`：

```toml
[engine]
url = "http://127.0.0.1:8080/v1"   # OpenAI 相容伺服器（llama-server、Ollama、LM Studio）
model = "qwen3.5-9b-q8"            # 可省略
```

| API | 用途 |
|---|---|
| `home.engine()` | 有設定（`url` 不是空的）才是 `Some`；App 自己的 `[engine]` 優先，沒有才用這個。每次用都重讀（伺服器可能換埠） |
| `home.set_engine(url, model)` | 管理本機模型的 App（PaperComposer）在作者選定／啟動模型時寫入；`url = None` 清掉。檔案裡其他表格保留 |
| `home.engine_settings()` | 整個檔：`[engine]`、`[claude] via／model`、`[route]`（類別 → 走向）、`[overrides]`（任務 → 走向）；不認得的值略過（＝自動） |
| `home.set_claude(via, model)`、`set_route(class, route)`、`set_override(task, route)` | 改一項、其他保留；`via` ∈ `VIAS`、類別 ∈ `CLASSES`、走向 ∈ `ROUTES`；覆寫設成 `auto`／`None` 就刪掉那行 |

同一個檔還有 Claude 連線與走向（由 `lore-engines` 的 `/api/engines` 讀寫，lore-llm 依它排先後）：

```toml
[claude]
via = "subscription"   # subscription | api（金鑰不在這裡）
model = "sonnet"       # catalog 別名或 id

[route]                # create 創作與審稿 · tidy 整理與格式 · ask 問答
create = "auto"        # auto | cloud | local
tidy = "auto"
ask = "auto"

[overrides]            # 個別任務，優先於類別
pc-draft-turn = "local"
```

不會寫進 `private-example/`。金鑰不在這裡（見 `lore-secrets`）。
