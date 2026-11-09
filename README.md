# LoreForge

把讀過的東西鍛造成作品的一套本機工具：讀論文 → 整理進知識庫 → 寫論文／做簡報 → 複習記住。全部在自己的電腦上執行（Rust ＋ 網頁介面），資料是檔案、用 git 同步。

| 資料夾 | 用途 |
|---|---|
| [`commons/`](commons/) | 共用模組：LLM（Claude API、Claude Code 訂閱、地端模型）、git 同步、私人資料位置（`ym-home`）、知識庫核心、對照表、PPTX 讀寫、畫布、論文素材擷取 |
| [`PaperComposer/`](PaperComposer/) | 論文寫作：句型庫、期刊範本、LaTeX 匯入匯出、對照表校對 |
| [`KnowledgeBase/`](KnowledgeBase/) | 個人知識庫：一張卡一個 JSON 檔、連結與網路圖、熟悉度與複習提醒 |
| [`SlideComposer/`](SlideComposer/) | 任務式簡報編輯器：標籤字型、漸進揭露、無損 PPTX、輸出 PPTX／PDF |
| [`NeonDeck/`](NeonDeck/) | macOS 桌面小工具（含「最近學到」知識卡） |
| [`private-example/`](private-example/) | `private/` 的公開範本：每個 App 能打開的最小內容 |

每個 App 是獨立的 Cargo 工作區，以相對路徑引用 `../commons/crates/<name>`。

## 開始

```sh
sh scripts/install-hooks.sh                    # commit 前檢查（見下方「防止外洩」）
cd KnowledgeBase && cargo run -p kb-server     # http://127.0.0.1:7700
cd SlideComposer && cargo run -p sc-server     # http://127.0.0.1:7880
cd PaperComposer && cargo run -p pc-server     # 見 PaperComposer/README.md
```

第一次啟動時，知識庫與簡報編輯器會從 `private-example/<App>/` 複製一份到 `private/<App>/`（只補缺的檔案），之後只讀寫 `private/`。PaperComposer 有 `private/PaperComposer/` 就用它，否則先開示範資料，再由設定頁建立工作區。

## 私人資料：`private/`

程式碼在這個 repo（公開）；個人資料**不在這裡**。`private/` 整個被 `.gitignore` 排除，底下每個資料夾都是獨立的私人 git repo：

```
LoreForge/
  private-example/          ← 公開範本，結構和 private/ 一樣
  private/                  ← 不會進這個 repo
    shared/                 所有 App 共用的個人設定（LoreForge-private）
      profile.toml          你是誰（使用者 id、顯示名稱）
      fonts.toml            介面字型、簡報字型代替規則（授權字型、公司字型寫這裡）
      deny-words.txt        commit 前要擋的私人字詞
      docs/                 私人筆記、設計文件
    KnowledgeBase/          知識卡、學習狀態（KnowledgeBase-private）
    SlideComposer/          風格包、簡報（SlideComposer-private）
    PaperComposer/          句型、範本、論文（PaperComposer-private）
```

讀取順序：`private/shared/<檔案>` → 沒有才用 `private-example/shared/<檔案>`。要個人化就把範本複製到 `private/shared/` 再改，公開的程式碼和範本都不用動。例如公司或買來的字型：字型檔裝在系統裡，名稱與代替規則寫在 `private/shared/fonts.toml`，App 執行時才讀進頁面，公開 repo 裡不會出現。

**新的 App** 用 `commons/crates/ym-home` 取得這些位置：

```rust
let home = ym_home::Home::discover(Path::new(env!("CARGO_MANIFEST_DIR")));
let data = home.seed_app("MyApp")?;           // private/MyApp（第一次從 private-example/MyApp 建立）
let prefs = home.prefs();                     // private/shared 的 profile、fonts
let extra = home.shared_file("my-app.toml");  // 任何新的共用設定檔
```

再加一個 `private-example/MyApp/`（能打開就好）。網頁介面連結 `/prefs.css` 就會套用你的介面字型。環境變數 `LOREFORGE_HOME`、`LOREFORGE_PRIVATE` 可以把位置指到別處。

## 金鑰與模型放在哪裡

**金鑰、密碼、token 一律不在 LoreForge 裡**（public、private 都不放）。程式發現位置在 git repository 裡面時，會拒絕寫入金鑰檔或部署金鑰。

| 東西 | 位置 | 怎麼設定 |
|---|---|---|
| Claude API key | macOS 鑰匙圈：服務 `PaperComposer`，帳號 `anthropic-api-key` | PaperComposer 設定頁貼上；開發時用 `PaperComposer/scripts/dev.sh`（存在鑰匙圈 `PaperComposer-dev`，以環境變數 `ANTHROPIC_API_KEY` 傳給這次執行） |
| Claude 訂閱（Claude Code） | Claude Code 自己的登入（`claude` 指令，`~/.claude`） | 執行一次 `claude` 登入；LoreForge 不儲存 |
| Gmail SMTP 應用程式密碼 | 鑰匙圈：服務 `KnowledgeBase`，帳號 `smtp-password` | 知識庫設定頁；或環境變數 `KB_SMTP_PASSWORD`。鑰匙圈不能用時才存 `~/Library/Application Support/KnowledgeBase/credentials.toml`（只有自己可讀） |
| GitHub 同步 | 系統的 git 憑證管理員或 `~/.ssh`；部署金鑰在各 App 的設定資料夾（`~/Library/Application Support/<App>/deploy/`） | 一般 `git` 登入 |
| 地端模型（`.gguf`）＋ llama.cpp | `LOCAL_LLM_HOME`（`<home>/gguf/<owner>/<repo>/`、`<home>/runtimes/llama.cpp/`）；沒設定時 PaperComposer 用 `~/Library/Application Support/PaperComposer/models`、`…/llama.cpp` | 環境變數 `LOCAL_LLM_HOME`，或 PaperComposer 的 `config.toml` 裡 `local_llm_home = "…"`；模型在 PaperComposer 的模型頁下載 |
| App 設定 | `~/Library/Application Support/<App>/`（`config.toml`、`reminders.toml` 等） | 各 App 設定頁；檔案裡沒有密碼 |

## 防止外洩

```sh
sh scripts/install-hooks.sh
```

讓這個 repo 使用 `scripts/hooks/`，每次 commit 前檢查：

1. `private/` 底下的檔案一律擋下。
2. 新增的行含有 `private/shared/deny-words.txt` 裡的字詞（不分大小寫、整個字）就擋下。清單本身是私人的。
3. 超過 5 MB 的檔案擋下。
4. 有安裝 [gitleaks](https://github.com/gitleaks/gitleaks)（`brew install gitleaks`）時掃描金鑰。

全域的 `core.hooksPath` 若有 `commit-msg`，仍會照樣執行。

## 授權

MIT，見 [`LICENSE`](LICENSE)。

## 開發說明

本專案與 Claude（Anthropic）協作開發，程式碼在 claude.ai 與 Claude Code 中共同撰寫。
