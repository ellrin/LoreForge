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

不會寫進 `private-example/`。金鑰不在這裡（見 `lore-secrets`）。
