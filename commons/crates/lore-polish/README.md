# lore-polish

語句潤飾（`docs/long-paper.md` §A）：選取一段，說或打要怎麼改，模型只改這一段。PaperComposer、SlideComposer 共用，
頁面是 `commons/ui/polish.js`（`LorePolish`，由 lore-shell 提供）。

| 模組 | 內容 |
|---|---|
| `lock` | `lock(text, macros, literal)` → `Locked { text, slots }`：`\cite…`、`\ref`／`\eqref`／`\cref`…、`\label`、`$…$`、`$$…$$`、`\(…\)`、`\[…\]`、數學環境、`%` 註解、對照表的巨集（`macros_of(&Notation)`）、`literal`（SlideComposer 的換行）換成 `⟦1⟧`、`⟦2⟧`…；`Locked::restore(reply, …)` → `(文字, Vec<Problem>)`：少了、出現兩次、不認得的記號、模型自己打的 LaTeX 都是問題；`Locked::encode` 把前一輪的改寫用同樣的記號再鎖一次 |
| `prompt` | `request(&Input)`：選取（已鎖）、所在段落前後、附近段落、文件語言、作者句型、語氣區塊、寫作規則（有才加）、按讚的改寫、預設的區塊（`preset` 名稱＋`guidance`，有才加）、對話；回覆 `{ "rewrite": "…" }`（JSON schema） |
| `log` | 一次提議一行 JSON（`id, passage, time, instruction, before, after, applied, liked, engine, problems`）；`append`、`update`（換行後原子寫回）、`history`、`passage_of` |
| `liked` | 語調庫 `private/shared/polish/liked.jsonl`（`Home::shared_dir`，只讀這裡，不讀 private-example）；`add`、`read`、`examples(all, text, n)`（字元雙字組相似度，最像的先，每邊最多 600 字） |
| 根 | `Host`（app 提供：`doc(name, text) → Doc { log, macros, language, phrases, tone, style }`（`style`：作者的寫作規則，空白就不加；PaperComposer 給 `style/` 的筆記，SlideComposer 不給）、`complete(req, engine)`、`check(doc, 段落前, 段落後)`、`presets()` 與 `preset_block(doc, id, PresetPassage { text, prefix, suffix, scope })`，兩個都有預設＝沒有）；`Preset { id, label, instruction }`（`instruction` 只在伺服器）；`propose`、`applied`、`like`、`history`；`TASK = "polish"`、`task_table()`（本 crate 的 `tasks.toml`：類別 create，`claude:selected` → `local:current`） |
| `web`（feature，預設開） | `router(Arc<dyn Host>)`：`POST /api/polish`、`POST /api/polish/applied`、`POST /api/polish/like`、`GET /api/polish/history?doc=&passage=|text=`、`GET /api/polish/presets`（`[{ id, label }]`） |

**預設（presets）**：app 提供有名字的改寫（PaperComposer：「用字」「我的風格」；SlideComposer 目前沒有）。`polish.js` 把它們列成輸入框上方的按鈕，`LorePolish.open({ preset, scope })` 可以一開就按下；按下的預設跟著每次送出（`ProposeBody.preset`、`scope`），這時打的字可以空著、有打就當補充。模型拿到預設的 `instruction` 與 app 為這段給的區塊；LaTeX 一樣鎖住。記錄（與讚）只寫「「名稱」＋打的字」，不寫區塊內容。

規則：有任何問題（記號少了、多了、app 的檢查擋下）就不給套用，問題逐條顯示；每次提議都記一行；讚只寫進
私人 shared 資料夾，之後的潤飾取最像的 4 筆當範例。

`cargo test -p lore-polish`：鎖住／還原（掉了一個記號被擋並說出是哪個、出現兩次、多出來的 LaTeX、巨集不吃前綴、環境整段鎖）、
前一輪改寫重鎖、提示內容、記錄與歷史、讚只寫在暫存的 private/shared、整個流程（假的 Host）與路由、預設（指示與區塊送到模型、LaTeX 仍鎖住、記錄只有名稱、不認得的預設擋下、`/api/polish/presets` 不送出指示）。

**LaTeX documents.** `Host::latex()` (default `false`; PaperComposer `true`): the restored rewrite goes through
`lore_notation::unicode::to_latex`, so ≤ − ⁺ α never reach pdflatex. SlideComposer keeps plain text.
