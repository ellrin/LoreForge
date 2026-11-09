# 我的知識庫（personal space）

KnowledgeBase 的個人 space：只有你看得到，放在你自己的 private repo。

```
space.toml        id、名稱、kind = personal、成員、圖片位置（iCloud Drive）
kinds.toml        知識卡種類與額外欄位
apps.toml         各 App 宣告的種類與連結型別
tags.json         領域（domains）與標籤階層（deep-learning/cnn）
nodes/<id>.json   一張知識卡一個檔（只放內容）
progress/<你>/    你的學習狀態（記不記得），每張卡一個檔
trash/            刪除的卡片（只會移過來，不會真的刪）
```

- 圖片不進 git：存在 iCloud Drive 的 `KnowledgeBase/assets/`，以 sha256 命名。
- 個人卡片不會自動進入家庭 space；要分享請在卡片上按「分享到…」（複製一份）。
- SQLite 索引不在這裡：kb-server 每次啟動從這些檔案重建。
