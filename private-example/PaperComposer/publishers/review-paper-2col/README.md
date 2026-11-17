# Review paper, two columns

L-Paper 寫書的模板之一（格式 `review2`，雙欄；`review-paper/` 是單欄 `review1`）。`template.toml` 的 `kind = "book"`，不會出現在新論文的版型裡。
`lorereview.sty` 是 LoreForge 自己寫的，用 XeLaTeX 編譯，只用 BasicTeX 就有的套件；頁面 A4。

指令和講義（`lecture-notes/` 的 `lorenotes.sty`）一樣，所以同一本書換格式不用改內文：

| 指令 | |
|---|---|
| `\course{…}` `\title{…}` `\author{…}` `\date{…}` `\keywords{…}` | 標題區（在第一頁上方，雙欄時跨兩欄） |
| `\term{contrastive learning}{對比學習}` | 名詞第一次出現：英文粗體，後面附中文 |
| `\sidenote{…}` | review paper 沒有邊註：變成腳註 |
| `definition`（可加 `[名稱]`）、`keyidea`、`intuition`、`remark` | 定義框（編號）、重點、直覺、補充 |

書的一章就是一個 `\section`，接在前一節後面，不換頁。中文字型和講義相同（`\lnCJKserif{…}`、`\lnCJKsans{…}` 可換）。
