# LoreReader workspace（私人 repo 的起始結構）

```
papers/<sha256>/paper.json         標題、原始檔名、DOI、加入時間、頁數（PDF 本身在書庫，不在這裡）
papers/<sha256>/annotations.json   螢光標記與註記（頁碼＋原文＋位置）
papers/<sha256>/quotes.json        引用句（PaperComposer 之後會讀）
papers/<sha256>/chat.json          問 Claude 的對話
known-words.txt                    你已經熟、不要標紅的英文字（一行一個或多個，# 之後是註解）
```

PDF 存在書庫（預設 `~/Documents/LoreReader/library`，在任何 git repo 之外），以 sha256 命名。
格式說明見 `LoreReader/README.md`。
