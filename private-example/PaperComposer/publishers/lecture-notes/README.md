# Lecture notes

PaperComposer 寫書的模板（`template.toml` 的 `kind = "book"`；不會出現在新論文的版型裡）。
`lorenotes.sty` 是 LoreForge 自己寫的，用 XeLaTeX 編譯，只用 BasicTeX 就有的套件。

| 指令 | |
|---|---|
| `\course{…}` `\title{…}` `\author{…}` `\date{…}` `\keywords{…}` | 標題區（課程、書名、作者、Keyphrases） |
| `\term{contrastive learning}{對比學習}` | 名詞第一次出現：英文粗體，後面附中文 |
| `\sidenote{…}` | 右邊的邊註 |
| `definition`（可加 `[名稱]`）、`keyidea`、`intuition`、`remark` | 定義框（編號）、重點、直覺、補充 |

中文字型：依序用這台電腦有的第一個（內文明體：Songti TC → LiSong Pro → Noto Serif CJK TC …；標題黑體：PingFang TC → Heiti TC → Noto Sans CJK TC …）。
要換字型，在 main.tex 的 `\usepackage{lorenotes}` 後面寫 `\lnCJKserif{字型A,字型B}`、`\lnCJKsans{…}`。

框（definition、keyidea、intuition）不會跨頁，框裡也不能放 `\sidenote`：每個框保持幾句話，邊註寫在框外。
`\term` 的中文用半形括號 `(對比學習)`；要全形，改 `lorenotes.sty` 裡 `\term` 那一行。
