# SlideComposer workspace（私人 repo 的起始結構）

```
styles/<id>/style.toml     風格包（標籤、色票、強調、元件、箭頭、變淡規則）＋ template.pptx
decks/<name>/deck.json     簡報模型（唯一真相；每次編輯後自動寫入）
decks/<name>/source.pptx   匯入的原檔或風格範本（不會被修改）
decks/<name>/assets/       插入的圖片（以內容雜湊命名）
decks/<name>/exports/      匯出的 .pptx / .pdf（永不覆蓋）
decks/<name>/cache/        OMML→LaTeX 快取（可刪）
```

風格包格式見 `commons/crates/lore-ooxml/src/style.rs`（`StylePack`）。
