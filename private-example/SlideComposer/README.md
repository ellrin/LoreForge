# SlideComposer workspace（私人 repo 的起始結構）

```
styles/<id>/style.toml     底版的配色與標籤（標籤、色票、強調、元件、箭頭、變淡規則）
styles/<id>/base.json      畫布型底版：每個頁類的固定元素、起始物件、區域（App 編譯成範本，不另存檔）
styles/<id>/template.pptx  原封母片型底版（從 PPTX 抽出，沒有 base.json 時用這個）
decks/<name>/deck.json     簡報模型（唯一真相；每次編輯後自動寫入）
decks/<name>/source.pptx   匯入的原檔或風格範本（不會被修改）
decks/<name>/assets/       插入的圖片（以內容雜湊命名）
decks/<name>/exports/      匯出的 .pptx / .pdf（永不覆蓋）
decks/<name>/cache/        OMML→LaTeX 快取（可刪）
```

配色與標籤格式見 `commons/crates/lore-ooxml/src/style.rs`（`StylePack`），底版格式見 `commons/crates/lore-canvas/src/base.rs`（`Base`）。顏色在 base.json 裡寫成角色（`"@ink"`），角色的值在 style.toml 的 `[palette.colors]`。`styles/blank` 是中性的空白底版。
