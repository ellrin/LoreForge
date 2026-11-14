# shared（所有 App 共用的私人設定，起始範本）

複製到 `private/shared/` 再改；那裡是自己的私人 repo，不在公開 repo 裡。

```
profile.toml     你是誰（id、name），以及 field = 你的領域與程度（名詞分類判斷「基礎」用）
fonts.toml       介面字型
deny-words.txt   commit 前擋下的字
my-vocab.json    我的詞彙：自己論文裡的詞與詞組（LoreReader 標「我的論文」時建立；
                 每個詞記下來源：論文 sha256，或 import:<名稱> 的匯入清單），之後
                 SlideComposer、PaperComposer 也會讀；這些詞在 LoreReader 算「記得」
polish/liked.jsonl  語調庫：潤飾時按 👍 的改寫，一行一筆 {before, after, instruction, app, time}；
                 App 自己建立，之後潤飾拿最像的幾筆當範例（這裡不放範例）
```
