# private-example

`private/` 的公開範本：每個 App 能打開所需的**最小**內容，裡面幾乎是空的。結構和 `private/` 一模一樣：

```
private-example/              （公開，在 LoreForge repo 裡）
  shared/                     所有 App 共用的個人設定
    profile.toml              你是誰（使用者 id、顯示名稱）
    fonts.toml                介面字型、簡報字型的代替規則（授權字型寫在 private）
    deny-words.txt            commit 前要擋的私人字詞
    docs/                     私人筆記、設計文件
  KnowledgeBase/              知識庫的空白 space
  SlideComposer/              簡報：styles/、decks/
  PaperComposer/              論文：profile/、papers/、publishers/、prompts/
```

## 怎麼用

- **App 第一次啟動**時，如果 `private/<App>/` 不存在，會從這裡複製一份過去（只補缺的檔案，不覆蓋），之後只讀寫 `private/`。**沒有任何程式會寫進 `private-example/`。**
- **共用設定**（`shared/`）：App 先讀 `private/shared/<檔案>`，沒有才用這裡的。要改就複製到 `private/shared/` 再改。
- `private/` 底下每個資料夾都是獨立的私人 git repo：

  | 資料夾 | 建議的 GitHub repo（private） |
  |---|---|
  | `private/shared/` | `LoreForge-private` |
  | `private/KnowledgeBase/` | `KnowledgeBase-private` |
  | `private/SlideComposer/` | `SlideComposer-private` |
  | `private/PaperComposer/` | `PaperComposer-private` |

## 新的 App 怎麼用私人資料

用 `commons/crates/lore-home`：

```rust
let home = lore_home::Home::discover(Path::new(env!("CARGO_MANIFEST_DIR")));
let data = home.seed_app("MyApp")?;          // private/MyApp（第一次從 private-example/MyApp 建立）
let prefs = home.prefs();                    // private/shared 的 profile、fonts
let notes = home.shared_file("my-app.toml"); // 任何共用設定檔：private 優先，否則用範本
```

再在這裡加一個 `private-example/MyApp/`（能打開就好的最小內容）。

## 這裡絕對不放

金鑰、密碼、token（在系統鑰匙圈）、地端模型（在 `LOCAL_LLM_HOME`）、真實的論文、簡報、知識卡、個人字詞。詳見根目錄 README 的「金鑰與模型放在哪裡」。
