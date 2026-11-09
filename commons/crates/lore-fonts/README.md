# lore-fonts — 文件裡的字型在這台電腦上變成什麼

簡報或論文指定了字型（Calibri、微軟正黑體、公司字型），這台電腦不一定有。這個模組替每個字型名稱算出代替順序，以及實際用到哪一個：

1. 作者自己的規則：`private/shared/fonts.toml` 的 `[fallback]`（公司字型與授權字型只寫在這裡）
2. 內建預設 `src/defaults.toml`：先放字寬相同的開源字型（Calibri → Carlito、Cambria → Caladea、Arial → Liberation Sans），讓換行位置跟原檔一樣
3. 不認得的字型依名稱猜分類（無襯線、襯線、等寬、數學、中文黑體、明體、楷體），用該分類的順序。無襯線先試較窄的字（Calibri、Carlito、PT Sans），因為 Office 文件多半照這種字寬排版

「有」的意思：系統已安裝（macOS 的字型資料夾、Windows Fonts、fontconfig），或放在作者的私人字型資料夾 `private/shared/fonts/`。私人資料夾裡的字型檔由 App 以 `@font-face` 提供給畫面，不必安裝進系統，也不會進入公開 repo。

| API | 用途 |
|---|---|
| `Table::defaults()`、`Table::with_user(&fallback)` | 規則表（內建／加上作者的規則） |
| `Table::chain(name)`、`Table::stacks()` | 一個字型的完整代替順序／所有規則（給網頁組 CSS font stack） |
| `Installed::scan(private_dir)`、`Installed::from_names(..)` | 這台電腦有的字型（系統＋私人資料夾）／測試用 |
| `resolve(&table, &installed, name)` → `Resolution { used, how, chain, category, user_rule }` | `how`：`installed`、`private`、`substitute`、`missing` |
| `report(&table, &installed, names)` | 多個名稱一次算（略過主題代號 `+mj-lt` 與 CSS 關鍵字） |
| `font_faces_css(&installed, "/fonts/")`、`safe_font_file(name)` | 私人字型檔的 `@font-face`／可以提供的檔名 |

在其他電腦或沒有中文字型的環境，`missing` 會照實回報，App 應該讓作者看到，而不是默默換成系統預設字。
