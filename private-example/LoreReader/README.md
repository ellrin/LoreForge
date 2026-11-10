# LoreReader workspace（私人 repo 的起始結構）

```
papers/<sha256>/paper.json         標題、原始檔名、DOI、加入時間、頁數（PDF 本身在書庫，不在這裡）
papers/<sha256>/annotations.json   螢光標記與註記（頁碼＋原文＋位置）
papers/<sha256>/quotes.json        引用句（PaperComposer 之後會讀）
papers/<sha256>/chat.json          問 Claude 的對話
papers/<sha256>/export.json        上一次「另存新檔（含筆記）」存到哪裡、當時的註記版本
papers/<sha256>/nouns.json         名詞清單的模型分類（快取，一篇一次）
ignore-words.txt                   忽略清單：不要列進「名詞」、不要標紅的字（右鍵「這不是術語」會寫進來）
models.json                        「模型」選單：每個工作用地端或 Claude 的哪一個
```

PDF 存在書庫（預設 `~/Documents/LoreReader/library`，在任何 git repo 之外），以 sha256 命名。
格式說明見 `LoreReader/README.md`。
