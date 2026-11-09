# commons

各個 App（PaperComposer、JELearner、KnowledgeBase、SlideComposer、NeonDeck 小工具，以及之後的新 App）共用的 Rust 模組與網頁元件。每個模組都不知道任何 App 的存在，App 以路徑依賴引用（從 App 的 `public/` 工作區：`../commons/crates/<name>`）。

整體設計見 `../private/shared/docs/ecosystem-design.md`（作者的私人筆記）。

## 模組

| crate | 用途 |
|---|---|
| `lore-llm` | LLM 抽象：Claude API（串流、web search、看圖）、Claude Code 訂閱、llama-server／OpenAI 相容端點、Mock；模型清單、地端模型、依任務選模型（`tasks.toml`）、Keychain 金鑰 |
| `lore-secrets` | 金鑰存在系統鑰匙圈（鑰匙圈不能用時才存成只有自己可讀的檔案，且絕不寫進 git repository） |
| `lore-home` | LoreForge 的私人資料位置：`private/<App>`（從 `private-example/` 建立）、共用個人設定 `private/shared`（profile、fonts…） |
| `lore-gitsync` | git 同步：只 fast-forward、絕不 force push、commit 前掃描金鑰、家人模式（只提交指定路徑） |
| `lore-store` | 資料分層與 space（個人／家庭）、內容定址圖片（iCloud Drive 或 git） |
| `kb-core` | 知識庫核心：知識卡、連結、標籤、每人學習狀態、索引、術語掃描 |
| `lore-notation` | 對照表：符號／方程式／圖／表／縮寫的登記、巨集、插入與校對 |
| `lore-ooxml` | PPTX 無損讀寫、風格範本、進場動畫、風格包 |
| `lore-math` | LaTeX ↔ OMML（以 pandoc 為旁掛程式） |
| `lore-canvas` | 簡報畫布：場景模型、標籤字型規則、可復原操作、漸進揭露、PPTX ↔ 畫布 |
| `lore-docsrc` | 論文素材擷取：從 arXiv LaTeX 或 PDF 找出圖、表、方程式並裁切 |

`ui/` 是不需建置的 Web Components（例如 `notation-panel/`），各自附 `demo.html`。

各 crate 的 API 與遷移說明在它自己的 `README.md`。

## 指令

```sh
cargo test                 # 全部，離線
cargo test -p <crate>      # 單一模組
sh ../scripts/install-hooks.sh   # 安裝 LoreForge 的 commit 前檢查（見根目錄 README）
```

## 開發說明

本專案與 Claude（Anthropic）協作開發，程式碼在 claude.ai 與 Claude Code 中共同撰寫。
