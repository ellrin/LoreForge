# lore-shell

四個 LoreForge app 共用的外殼檔案（`commons/ui/icons.js`、`base.css`、`shell.js`），編進 crate，
讓每個伺服器提供同一份。設計與介面：`docs/shell.md`（F 節）。

| API | 用途 |
|---|---|
| `ICONS_JS`、`BASE_CSS`、`SHELL_JS` | 三個檔的內容 |
| `FILES` | `(路徑, content-type, 內容)`：`/icons.js`、`/base.css`、`/shell.js`、`/makedeck.js`（做成簡報）、`/makebook.js`（寫 review，`LoreMakeBook.mount({ before, source, key, extras })`） |
| `icon_names(src)`、`icon_aliases(src)` | 讀 icons.js 的圖示名稱與舊名對照（測試用） |

`cargo test -p lore-shell`：圖示沒有重複、每個 SVG 合法、舊名與改名都指到現有圖示、
四個 app 的舊 icons.js 與 HTML／JS 用到的名稱都在、base.css 只用 theme.css 的 token、
齒輪的共用分頁與上方列右側的順序。
