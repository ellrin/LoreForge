# LoreShelf

本機的檔案目錄：**不移動任何檔案**，用內容雜湊（sha256）認檔案，記下每個檔案在哪裡出現過、各種時間（建立、修改、下載、最後打開、文件內的日期）與下載網址，找出重複與只有一份的檔案。設計草稿在 `../docs/loreshelf.md`。

> 第一波：只讀掃描與報告。之後：內容抽取與標籤、3D 瀏覽、LoreEcho 與知識庫串接。

## 執行

```sh
cargo run --release -p loreshelf-cli -- scan ~/Downloads   # 只讀掃描，印出摘要與報告位置
cargo run --release -p loreshelf-cli -- report             # 最新報告的位置
cargo run -p shelf-server                              # 介面：http://127.0.0.1:7900
cargo test                                             # 離線測試（暫存資料夾、合成檔案）
```

- 目錄（每個內容一份紀錄、專案資料夾、報告）在 `~/Library/Application Support/LoreForge/LoreShelf`（或 `LORESHELF_HOME`），**只存在本機**：放在 iCloud Drive、這個 repo 或有遠端的 git 裡時會拒絕；它自己的 git 沒有遠端，推送一律被擋。
- 雜湊快取在 `~/Library/Caches/LoreForge/shelf/`，可以隨時刪除；沒變的檔案重掃時不會再讀。
- 掃描不寫入、不搬移、不改名，不跟隨符號連結，不下載 iCloud 上未下載的檔案，略過 `.DS_Store`。
- 有 `.git`、`main.tex`（或 `.tex`＋`.bib`）、`Cargo.toml`、`package.json`、`*.xcodeproj`、`pyproject.toml` 的資料夾算一個專案，不往內讀；同一個 git repo 的多個 clone 算同一筆。
- macOS 套件（`.app` 等）與資料集資料夾（超過 500 個檔、九成同一副檔名）各算一筆，只計檔數與大小。
- 目錄的本機 git 只記錄作者的修改（`edits/`），掃描結果不進 git。
- macOS 若拒絕讀取（例如「下載項目」的隱私權限），掃描會停下並說明；到「系統設定 → 隱私權與安全性 → 檔案與資料夾」允許終端機。

## 開發說明

本專案與 Claude（Anthropic）協作開發，程式碼在 Claude Code 中撰寫。
