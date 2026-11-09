# LoreForge

把讀過的東西鍛造成作品的一套本機工具：讀論文 → 整理進知識庫 → 寫論文／做簡報 → 複習記住。全部在自己的電腦上執行（Rust ＋ 網頁介面），資料是檔案、用 git 同步。

| 資料夾 | 用途 |
|---|---|
| [`commons/`](commons/) | 共用模組：LLM（Claude API、Claude Code 訂閱、地端模型）、git 同步、知識庫核心、對照表、PPTX 讀寫、畫布、論文素材擷取 |
| [`PaperComposer/`](PaperComposer/) | 論文寫作：句型庫、期刊範本、LaTeX 匯入匯出、對照表校對 |
| [`KnowledgeBase/`](KnowledgeBase/) | 個人知識庫：一張卡一個 JSON 檔、連結與網路圖、熟悉度與複習提醒 |
| [`SlideComposer/`](SlideComposer/) | 任務式簡報編輯器：標籤字型、漸進揭露、無損 PPTX、輸出 PPTX／PDF |
| [`NeonDeck/`](NeonDeck/) | macOS 桌面小工具（含「最近學到」知識卡） |

每個 App 是獨立的 Cargo 工作區，以相對路徑引用 `../commons/crates/<name>`。

## 私人資料

程式碼在這個 repo（公開）；個人與家庭共用的資料**不在這裡**。每個 App 的資料是各自獨立的私人 git repo，放在 `private/` 底下（整個資料夾被 `.gitignore` 排除）：

```
LoreForge/
  private/                 ← 不會進這個 repo
    PaperComposer/         句型、範本、論文（自己的 private repo）
    KnowledgeBase/         知識卡、學習狀態（自己的 private repo）
    SlideComposer/         風格包、簡報（自己的 private repo）
    docs/                  設計筆記（自己的 private repo）
    deny-words.txt         commit 前要擋的私人字詞（每行一個正規表示式）
```

App 預設在 `../private/<App>` 找資料，也可以用各 App README 的環境變數或設定檔指到別處。

## 開始

```sh
sh scripts/install-hooks.sh          # commit 前檢查：擋 private/、私人字詞、大檔、金鑰
cd commons && cargo test             # 共用模組
cd KnowledgeBase && cargo run -p kb-server    # http://127.0.0.1:7700
```

各 App 的執行方式見它自己的 README。

## 開發說明

本專案與 Claude（Anthropic）協作開發，程式碼在 claude.ai 與 Claude Code 中共同撰寫。
