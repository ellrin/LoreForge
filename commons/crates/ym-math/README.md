# ym-math

方程式轉換（設計稿 §7.4、§7.8、§13-2）：LaTeX ↔ OMML，用 **pandoc 當離線旁掛程式**（texmath 引擎）。

| API | 內容 |
|---|---|
| `Pandoc::default()` | `$YM_PANDOC` → `/opt/homebrew/bin/pandoc` → `/usr/local/bin/pandoc` → PATH 上的 `pandoc`；`Pandoc::new(path)` 自訂 |
| `Pandoc::version` / `available` | 找不到時回 `Error::PandocMissing`（訊息含安裝方式） |
| `latex_to_omml(latex, display)` | `$$…$$` → pandoc docx → 取出 `m:oMathPara` / `m:oMath`（Word 版 OMML） |
| `latex_to_pptx_math(latex, display, size_pt)` | 同上再轉成 PowerPoint 版：包在 `a14:m`，每個 `m:r` 加 Cambria Math 的 `a:rPr` |
| `omml_to_latex(el)` | Word 版、PowerPoint 版（`a14:m`）皆可：去掉 `a:rPr`、數學斜體碼位（𝐼、𝑡、𝐌）轉回 ASCII（粗體 → `m:sty b`）→ 包成最小 docx → pandoc → LaTeX |
| `pptx_to_word` / `word_to_pptx` | 兩種 OMML 方言互轉 |
| `equation_shape(id, name, rect, math, fallback_rel)` | PowerPoint 寫法的方程式文字方塊：`mc:AlternateContent`（a14 Choice ＋ 可選的圖片 Fallback） |
| `slide_equations(doc)` | 取出一頁所有 `a14:m` |
| `MathRenderer` / `NoRenderer` | Fallback 圖片（PNG/SVG）渲染掛勾；目前是 stub（TODO：KaTeX/MathJax 旁掛或 Typst） |

實測（4 份真實簡報，`examples/deck_equations.rs`）：93 個原生方程式 OMML→LaTeX 全部成功，再轉回 OMML 90/93。

測試不需要 pandoc 也會通過：需要 pandoc 的測試在找不到時直接略過。

```sh
cargo test -p ym-math
YM_DECKS="a.pptx:b.pptx" cargo run -p ym-math --example deck_equations
```
