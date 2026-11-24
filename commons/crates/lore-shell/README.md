# lore-shell

四個 LoreForge app 共用的外殼檔案（`commons/ui/icons.js`、`base.css`、`shell.js`），編進 crate，
讓每個伺服器提供同一份。設計與介面：`docs/shell.md`（F 節）。

| API | 用途 |
|---|---|
| `ICONS_JS`、`BASE_CSS`、`SHELL_JS` | 三個檔的內容 |
| `FILES` | `(路徑, content-type, 內容)`：`/icons.js`、`/base.css`、`/shell.js`、`/makedeck.js`（做成簡報）、`/makebook.js`（寫 review，`LoreMakeBook.mount({ before, source, key, extras })`）、`/polish.js`（潤飾，`LorePolish.mount({ doc, language, engine, onLike })`＋`LorePolish.open({ anchor, text, prefix, suffix, context, keep, apply })`；伺服器掛 lore-polish 與 lore-voice 的路由）、`/findfile.js`（搜尋檔案，`LoreFindFile.button({ purpose, exts, context, onAdded })`）、`/selection.js`（選取的文字交給 L-Echo，`LoreSelection.watch({ gather, apply })`＋`LoreSelection.send()`） |
| `findfile`（feature `web`） | 搜尋檔案一律交給 L-Echo：`Finds::new(app, on_pick)` ＋ `router(finds)` ＝ `POST /api/lore/find {purpose, exts, context}`（發 request id、必要時 `open -g -an L-Echo`、交給 L-Echo 的 `POST /api/find`）、`GET /api/lore/find/{request}`（`{open, added}`，頁面輪詢）、`POST /api/lore/picked {request, path \| end}`（只收開著的 request、副檔名要符合；交給 app 的 callback，`Pick { purpose, context, path }`）；L-Echo 那邊用 `send_back`；`with_ask` 給測試 |
| `selection`（feature `web`） | 選取的文字交給 L-Echo、改寫送回頁面套用：`Selections::new(app)` ＋ `router(sel)` ＝ `POST /api/lore/selection {doc, source, text, prefix, suffix, context, scope, keep, language?, readonly?, launch}`（`readonly: true` ＝ 不能改的文件（PDF），L-Echo 不給改寫、不送回；發 request id、加上 `app`、`port` 交給 L-Echo 的 `POST /api/selection`；`launch: false` 只找已開的 L-Echo；連不到 503 並丟掉；同一份 doc 新的選取結束舊的；text 上限 20,000 字、context／prefix／suffix 8,000）、`GET /api/lore/selection/{request}`（`{open, pending: [{seq, after, polish}], error}`，頁面輪詢）、`POST /api/lore/selection/{request}/done {seq, ok, error?}`、`POST /api/lore/selected {request, after, polish? \| end}`（等頁面最多 15 秒：`{ok, message}`／409 頁面拒絕／410 已結束／504 沒回應）；L-Echo 那邊用 `send_selected`、`end_selection`；`with_ports`、`with_echo`、`with_wait` 給測試 |
| `icon_names(src)`、`icon_aliases(src)` | 讀 icons.js 的圖示名稱與舊名對照（測試用） |
| `permissions` | 系統權限：`Permission`（microphone、speech、personal_voice）、`Status`（granted、denied、not_determined、restricted、unsupported）、`Provider`（lore-voice 的 `Voice`、`fake::FakeProvider`）、`Permissions::new(&[…]).with(provider)`（`.isolated()`：測試／隔離，不開系統設定、不在啟動時問）；`request(None)` 依固定順序只問從沒問過的、一次一個、失敗就停；`router(p)`（feature `web`）＝ `GET /api/permissions`、`POST /api/permissions/request {what?}`、`POST /api/permissions/open {what}`，並把這組登記成啟動時要問的；桌面 App 視窗開好後呼叫 `ask_at_launch()`（背景執行緒，每個行程一次） |

第一次打開會一次詢問 App 需要的所有權限；之後到 設定 → 一般 → 權限（只有宣告了權限的 App 才有這一塊：L-Paper、L-Slide、L-Echo）。
功能 `web` 預設開；lore-voice 只用型別（`default-features = false`）。

`cargo test -p lore-shell`：圖示沒有重複、每個 SVG 合法、舊名與改名都指到現有圖示、
四個 app 的舊 icons.js 與 HTML／JS 用到的名稱都在、base.css 與 polish.js 只用 theme.css 的 token、
齒輪的共用分頁與上方列右側的順序、一般 → 權限只在有宣告時出現；權限的順序、只問沒問過的、啟動只在桌面 App、路由（假的 provider）；選取：轉交（假的 L-Echo）、套用／拒絕／逾時／結束、新選取取代舊的、L-Echo 沒開、長度上限。
