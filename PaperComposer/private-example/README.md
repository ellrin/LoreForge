# 私人工作區範例

這個資料夾示範**你自己的私有 repo** 長什麼樣子：每個檔案都是真的會用到的格式、填了內容，但人名、機構、期刊、文獻全部是虛構的（DOI 用 `10.5555` 測試前綴）。照著它建立自己的私有 repo，再把內容換成自己的。

```
private-example/
  config.toml                 App 記住的介面設定（淺色／深色、配色、縮放、上次開的論文…）
  profile/                    寫作資料庫
    profile.toml              資料庫名稱、語言
    phrases.csv               句型：章節、功能、位置、{佔位} 模板、例句
    expressions.csv           轉折詞、搭配詞、短語，附用法
    exemplars.csv             整段範例（你的語氣），附出處
    vocabulary.csv            避免的字與替代
    rules.csv                 段落檢查規則
    equations.csv             常用公式
    authors.csv               合作者（建立論文時勾選）
    affiliations.csv          機構
  publishers/
    example-journal/          一個期刊模板
      template.toml           名稱、簡稱、出版社、類別、作者須知、投稿規定
      sample.tex              期刊範例（新論文從它產生）
      refs.bib
  papers/
    retina-triage/            一篇論文
      paper.toml              Key value 與作者（對應 authors.csv 的 id）
      main.tex                標題、作者、摘要、關鍵字、\input 各章節、參考文獻
      sections/*.tex          一句一行，段落之間空一行
      refs.bib
      figures/overview.png
  prompts/                    自訂提示詞（選填）
```

## 用它建立自己的工作區

1. 在 GitHub 建立一個空白的 **private** repo，例如 `you/PaperComposer-private`。
2. 複製這個資料夾並推上去：

   ```sh
   cp -R private-example ~/PaperComposer-private
   cd ~/PaperComposer-private
   git init && git add -A && git commit -m "Start my workspace"
   git remote add origin https://github.com/you/PaperComposer-private.git
   git push -u origin main
   ```

3. 在 App 的「設定 → 工作區」選「開啟資料夾」，指向 `~/PaperComposer-private`。
4. 換成自己的內容：
   - `profile/`：在 App 的「資料庫」視窗編輯，或用試算表開 CSV（存成 UTF-8）。句型最好來自你自己寫過、潤稿過的論文。
   - `publishers/`：從期刊官網下載 LaTeX 模板放進來，或在 App 的「模板 → 上傳模板」讓 Claude 轉換。
   - `papers/`：範例論文可以刪掉，用「新論文」建立自己的。
   - `config.toml`：不用手動改，在 App 裡切換外觀就會更新。

之後在 App 裡按 Ctrl+S 就會 commit 並 push，另一台電腦開啟時自動 pull。

**不要把真實的個人資料放進公開 repo。** 這個範例在公開 repo 裡，是因為裡面沒有任何真實資料；期刊的 `.cls`／`.bst` 可能有自己的授權，也請放在私有 repo。
