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

所有 App 共用的本機模型**選擇**存在 `private/shared/engine.toml`（只有 catalog id；正在跑的
llama-server 在哪裡、誰在用，是執行期資料，由 lore-llm `shared_runtime` 放在快取資料夾，見
`docs/local-models.md`）：

```toml
[engine]
model = "qwen3.5-9b-q8"            # 共用預設模型（local:current）
vision_model = "gemma-4-12b-q8"    # 可省略：需要看圖而預設模型不能看圖時改用
```

| API | 用途 |
|---|---|
| `home.engine()` | 有 `model` 或 `vision_model` 才是 `Some`。舊檔的 `url` 讀到也忽略 |
| `home.set_engine_model(model)`、`set_vision_model(model)` | 齒輪 → 模型選定時寫入；`None`／空字串刪掉。順便刪掉舊的 `url`，其他鍵與表格保留 |
| `home.local_home()`、`set_local_home(path)` | `[local] home`＝這台電腦的模型資料夾（PaperComposer 啟動時寫入；`LOCAL_LLM_HOME` 之後的備援，給從 Finder 開的 app）。值相同就不寫檔 |
| `home.forget_engine_url()` | 刪掉舊的 `[engine] url`（沒有就不寫檔）；lore-engines 掛上時呼叫一次 |
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
