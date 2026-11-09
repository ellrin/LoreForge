# lore-store

資料分層與 space 掛載（設計稿 §3）。不認識任何 App。

- `Space` / `SpaceConfig`：`space.toml`（`id`、`name`、`kind = personal|household`、`members`、`[assets] backend = icloud|git`、`root`、`max_bytes`、`max_dim`）。
  - personal 只能有一位成員；household 的圖片必須放 git（她的 iPad 走 GitHub API、Apple ID 不同）。
  - `progress_dir(member)`：`progress/<member>/`，只給該 space 的成員。
- `Spaces`：同時掛載多個 space（id 不可重複）；`default_for(member)`。
- `check_flow(from, to, explicit)`：跨 space 只能透過明確的分享（複製）；個人資料不會自動流進 household。
- `AssetStore`：內容定址（sha256），存成 `<root>/ab/abcd….ext`。
  - `Backend::Icloud`：iCloud Drive 資料夾（預設 `~/Library/Mobile Documents/com~apple~CloudDocs/KnowledgeBase/assets`）。遇到 `.<name>.icloud` 佔位檔時經由可注入的 `CommandRunner` 執行 `brctl download` 並等待檔案出現。**not tested on real hardware**（測試用假的 runner 與暫存資料夾）。
  - `Backend::Git`：space 內的 `assets/`；點陣圖轉 webp（無損，`image` crate 沒有有損 webp 編碼器）、縮到 `max_dim`，超過 `max_bytes` 會再縮小，仍超過就拒絕。
  - `import_from`：分享時把圖片複製到另一個 store。
- `free_path`、`copy_missing`、`write_atomic`、`valid_member`、`expand_home`。

```sh
cargo test -p lore-store
```
