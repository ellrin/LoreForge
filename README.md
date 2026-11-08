# SlideComposer

任務式簡報編輯器：選「任務版型」（例如論文報告）＋「風格版型」（從 PPTX 收集的母片、標籤字型、色票、元件），放入論文等素材，快速產生初稿，再在畫布上細修，輸出 PPTX／PDF。與個人知識庫、對照表串聯。

目前為起始階段：PPTX 無損讀寫與風格包格式在 `../../commons/crates/ym-ooxml`，方程式轉換在 `ym-math`。設計見 `../../docs/ecosystem-design.md` §7。

個人的風格包、任務版型與簡報放在另一個私人 repo（`SlideComposer-private`），不放在這裡。

## 開發說明

本專案與 Claude（Anthropic）協作開發，程式碼在 claude.ai 與 Claude Code 中共同撰寫。
