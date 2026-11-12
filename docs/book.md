# 寫書：給自己的 review paper（規劃）

作者 2026-11-13：下一步要做寫書。記下來給之後的工作用。

## 目標

LoreReader 讀論文時，把**看不懂的部分自動整理成一份 review paper（書）**：為作者客製的背景介紹，例如醫學背景、新方法所依據的舊方法。論文報告（docs/paper-deck.md「以後」的「知識背景文件」）與 PaperComposer 首頁的「寫書」都是同一件事的入口。

## 流程（草案）

1. **從 LoreReader 一鍵開始**：這篇論文裡作者不懂的東西 —— 紅色名詞（`nouns.rs`：領域詞、知識庫沒有或「要學」）、卡住的段落（04 段落沒有「懂了」）、「問這篇」問過的問題。
2. **比對知識庫**：已經「記得」的不寫；「學習中」的簡短帶過。
3. **排順序**：依先備關係（先懂哪個才懂哪個；設計文件的 `prerequisite`）排章節。
4. **找資料**：LoreReader 讀過的論文與筆記、知識卡，再加上開放取用的文獻（OpenAlex / arXiv / Semantic Scholar）；每段附引用。
5. **寫成 LaTeX 文件**：存在作者選的資料夾（文件層，不放 private），在 PaperComposer 的「寫書」打開，可以改寫、匯出 PDF。必須與 `lore-notation` 相容（登記的巨集、`\ref{}`、`notation.tex`、跑檢查）。
6. **長時間工作**：跟論文報告一樣在背景跑、顯示進度、關掉視窗不中斷（SlideComposer 的 `jobs.rs` 要抽成共用的給 PaperComposer 用）。
7. **讀完回到知識庫**：閱讀時可以加卡；再比對，直到這篇勾選的詞都至少「學習中」。

## 需要先有的

- PaperComposer 的文件類型（設計文件 §6 `kinds/<id>/kind.toml`：`paper`、`book`、筆記）。
- 共用的工作執行器（從 SlideComposer `jobs.rs` 抽到 commons）。
- 作者私人資料的取用（few-shot 範例與檢索，放在 private，經 lore-home）。
