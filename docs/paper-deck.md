# 論文變簡報（paper → deck）

從 L-Paper 的論文或 L-Reader 的論文產生一份可上台的論文報告簡報。工作在 L-Slide 裡跑；L-Paper 與 L-Reader 上方列的「做成簡報」（共用的 `commons/ui/makedeck.js`）只是把論文交給 L-Slide 並打開這個工作；L-Slide 沒開時先把它打開（`lore-sc://`）、等它回應（最多 30 秒），等不到才顯示「開啟 L-Slide／重試」。

## 流程

按下去 → 長時間的整理（顯示進度）→ 微調視窗（長度／語言／底版、建議、大綱、最多 5 題，全部可略過）→ 產生投影片 → 排版用字微調 → 打開簡報（右側面板列出排版用字報告）。卡片上另有「直接產生」：跳過微調視窗。

- 開始的卡片（L-Reader／L-Paper 的「做成簡報」與 L-Slide 的「論文報告」都一樣）：長度（短 5–10 頁、標準 10–15、長 16 以上，提示用 16–24）、語言、**報告者／指導教授**、底版。報告者與指導教授從 L-Paper 存的人選（`GET /api/todeck/people`：讀 L-Paper 工作區的 `profile/authors.csv`，不啟動 L-Paper；只回 id 與姓名，e-mail 等不外流）；上次的選擇記在 L-Slide 的私人工作區（`paper-deck.json`），下次（含「直接產生」）預設同一組。
- 頁數算的是大綱的頁（含封面與致謝）；逐行出現多出來的複製頁不算。
- 做不到或降級的每一件事（圖沒有圖檔、式子改放截圖、超過字數、文獻放不下……）都列在工作的 `warnings`：進度畫面、微調視窗上方、以及打開簡報後右側「排版用字」最上面，不會默默略過。

- 微調視窗上方可以改長度、語言、底版。改長度或語言：只重做「大綱」這一步（進度畫面），做完回到微調視窗；同一個頁面內問題的答案、「我懂」、「記得」都保留（重跑時關掉視窗或重新載入就不保留），對大綱列的修改不保留（是新的大綱）。改底版不必重做（投影片那一步才用）。
- 「建議」：大綱寫完後模型再看一次大綱（加上作者的風格摘要與語氣範例），提出最多 5 個具體修改（加頁、刪頁、移動、改標題、改重點、換圖、合併、拆頁），每個附一行理由；逐項「套用」（改上面的大綱列）或「忽略」。沒有值得說的就沒有這一塊。
- 打開產生的簡報時，右側面板最上面是「排版用字」：每一項修正（第幾頁、改了什麼）可單獨「復原」，用詞一致的建議可逐項「套用」；都走編輯器的復原紀錄。之後從 AI 選單 →「排版用字」再打開。

## 工作（job）與階段

- 工作在伺服器端跑，id 在 `<L-Slide workspace>/cache/todeck/<job>/`（可刪的快取層，永不寫進 private）。
- 每個階段一個 JSON，原子寫入；完成的階段不重做。
- 關掉視窗不會停；頁面用 job id 重新接上。結束 app 會停；下次開啟時提示「上次做到第 N 步，繼續？」，從最後完成的階段接著做。
- 某階段檔案壞掉：只重做那一階段。

| 階段 | 產出 | 進度顯示 |
|---|---|---|
| `source` | 章節、圖、表、方程式（含 label）、參考文獻；L-Reader 的 PDF：模型逐頁看過並確認（見「素材」） | 圖 n/N、表 n、方程式 n |
| `notation` | 論文的對照表條目（沿用論文／L-Reader 的 id） | 條目 n |
| `terms` | 名詞與知識庫比對：熟／不熟 | 名詞 n → 不熟 m |
| `profile` | 作者簡報的風格摘要；更新語氣範例檔（見下） | — |
| `outline` | 每頁：種類、標題、用到的素材、重點、講者備註；接著「建議」 | 模型串流的字數／最後活動時間 |
| `review` | 作者在微調視窗的決定（可略過） | 等待作者 |
| `slides` | 投影片物件 | 第 n/N 頁 |
| `polish` | 排版用字微調（不改句子） | 檢查 n 項 |

- 呼叫模型的階段：取消綁在工作上（不是 HTTP 連線），「停止」會真的中止請求。
- 進度：階段清單（目前那一項亮起）、具體數字、每階段已花的時間、模型步驟顯示串流字數與最後活動時間，讓人看得出沒有當機。

## 本機模型的記憶體

本機模型是整台電腦共用的一個 llama-server（lore-llm `shared_runtime`，見 `docs/local-models.md`）：任何 app 需要時都能啟動，L-Slide 和其他 app 一樣經 `acquire` 使用。釋放記憶體：閒置 N 分鐘由任何 app 停掉（預設 10 分，在 齒輪 → 模型 設定）；「釋放」按鈕只在沒有其他 app 在用時能按；最後一個在用的 app 結束時停掉。

## 內容順序與風格

- 順序以作者的 seminar 簡報為準（設計文件 §7.1 的論文報告骨架）。
- `profile`：從作者私人資料夾的簡報（經 lore-home 唯讀讀取）抽出精簡的風格摘要，存在快取層：各種頁的頁數分配、標題長度與句型、條列數、Take home 的語氣、圖說與 `Ref:` 的習慣；頁尾標語、logo 等一律排除。摘要不進 repo。
- **語氣範例（2026-11-13 作者同意，取代「只送摘要」）**：除了摘要，也把作者自己以前簡報的句子當範例送進模型（Claude 或本機皆可），讓大綱的標題、條列、Take home 的寫法（用字、長度、中英混用、動詞開頭等）像作者。
  - 來源：只有作者自己的簡報（和摘要讀同一批：從作者自己的檔案匯入的，L-Slide 產生的不算）。
  - 排除：封面頁與致謝頁、頁尾等版面佔位、同一份簡報反覆出現的文字與圖（頁尾標語、機密標記、講者列、logo）、`Ref:` 與引用（et al.）、e-mail 與網址、機密／版權字樣、作者姓名與 id（`shared/profile.toml`）、`shared/deny-words.txt` 的每個字、只有數字的行、方程式與文獻條目、一兩個字的標籤。
  - 存放：作者的私人 repo（工具資料層，git 管）`private/SlideComposer/voice/examples.json`（經 lore-home 找到），每種頁的標題、條列、Take home、圖說各有上限。不放 `.claude`、skill、公開 repo 或程式常數。簡報有變（與摘要同一個 key，加上排除清單與版本）就重建；內容沒變不改寫檔案。key 存在快取層（`cache/deck-style/voice-key.json`）。工作的檢查點裡沒有範例句子。
  - 使用：`outline` 依論文（標題、關鍵字、摘要、章節名）挑每種頁最相近的幾句（同一句的變體只取一次），整段約 2600 字以內，接在系統提示的摘要後面，說明「學語氣、不抄內容」；範例語言與簡報語言不同時照樣參考語氣、仍用所選語言寫。「建議」用較短的一段（約 1200 字）。投影片與排版用字不再呼叫模型寫句子，所以只有這兩處。
  - 實作：`L-Slide/crates/server/src/deckvoice.rs`。

## 版面：一種字一個字級、同樣的位置

作者的規則（2026-11-19）：「字級不一致是大忌，每一頁該對齊要對齊」。

- 字型：`private/shared/fonts.toml` 的 `[deck]`（`latin`、`east_asian`，經 lore-home；公開範本留空 = 用底版自己的字型）。產生簡報時記在這份簡報自己身上（`deck.fonts`，也跟著 .pptx 的編輯狀態走），之後開啟、套標籤、排版用字都用這組字型；方程式維持數學字型。
- 每個文字標籤（標題、內文、圖說、表格、文獻、重點）整份只有一個字級；不再有「字多就換小一號」。放不下時不縮字：大綱的字數上限寫在提示裡（標題 48 字、每頁 5 行每行 90 字、有圖表的頁 3 行每行 80 字、重點 100 字），規劃後檢查，超過就請模型修正一次；還超過：行數多的拆成兩頁，其餘照寫並列進 `warnings`。排版用字微調對這種簡報也不縮字（`keep_sizes`）。
- 內容頁共用同一組框（`build.rs` `page_frames`）：標題一律用版面配置自己的標題框；內文從同一條線開始；`Ref:` 在 logo 右邊、頁碼左邊（從版面配置畫在底部的東西算出來，和作者自己的簡報同一個位置），只放整筆文獻、最多三行，放不下的不放並列進 `warnings`。
- 寬的圖放上面（至少佔內容寬度的八成）、文字在下；高的圖在左、文字在右。表格是原生表格、佔滿內容寬度。重點句（takeaway）：底版沒有深色帶時是一行「關鍵結果」強調字，緊接在內容下面。
- 封面：標題、期刊行、報告者／指導教授（底版的 `subtitle`）；不放關鍵字與自己的 `Ref:`。致謝頁：只有標題，置中。Take home 的編號由程式加（模型自己寫的「1.」會去掉）。
- 守門測試：`tests/todeck.rs` `assert_one_size_per_role`（排版用字微調後的簡報：每個標籤一個字級、內容頁標題框相同、內文起始線相同）。

## 素材：模型看過、確認過（L-Reader 的 PDF）

作者的規則（2026-11-19）：品質優先，不在乎 token；讓模型真的看 PDF 的頁面與截圖並確認，不用規則猜了就默默帶過。全部是 Opus → Sonnet，不走本機模型。實作：`todeck/extract.rs`（`source` 階段）。

1. **找**（`sc-locate-material`，每頁一次）：把每一頁的圖給模型，回答這頁的圖與表（編號、範圍、圖說）、獨立成行的方程式（編號、範圍、LaTeX）、第一頁的作者。原本的規則式偵測（lore-docsrc）與 L-Reader 文字層的 LaTeX 只當「提案」附上，請模型對著圖核對；文字層的 LaTeX 含控制字元（例如 U+0008）就不附。
2. **確認**（`sc-check-crop`）：每張圖／表的截圖再給模型看一次：是不是正好那一個、完整、沒有多切到別的（圖說、內文、隔壁的表）。有問題就依模型給的範圍重切再確認（最多看三次）；還是不過：照目前的範圍放，並列進 `warnings`。
3. **表格**：格子用 PDF 自己的文字排（`lore_docsrc::pdf::table`：確認過的範圍裡的字，依行與共同的空白欄分格），數字不是模型讀的；模型看到的列數／欄數只用來核對。排不出來或對不起來：放表格的截圖，並說明。
4. **方程式**（`sc-read-equation`）：把讀出的 LaTeX 排版成圖（KaTeX＋headless Chrome；沒有 Chrome 時模型直接讀 LaTeX，並說明），和論文的截圖一起給模型逐個符號比對；不一樣就用模型的修正再比一次。「排得出來」不算數。兩次都對不起來：放論文的截圖，並說明。
5. 模型沒看到的東西不放（偵測多抓的、文字層的碎片），列出哪些；一頁都看不了（沒有引擎）：沿用偵測的結果，並明說「沒有經過確認」。
6. 每個回答都存在這篇論文的快取（`cache/todeck/<sha>/verify/`，頁碼或截圖雜湊＋提示版本），停止再繼續、或同一篇再做一次都不會重問。`source` 的 `counts` 有 `calls`（這次問了幾次）與 `cached`。

寫大綱時模型也看得到圖：有圖檔的圖與以截圖呈現的表（縮小後）附在大綱的請求裡，素材清單標明每個圖／表／方程式能不能用（沒有圖檔的寫明不可用）；結果頁的表是原生表格、滿版寬，`key_row`（提案方法那一列）用底版的重點列底色（沒有就粗體）；同一張圖不會放在兩頁（主架構圖的分段講解除外），多出來的拿掉並列進 `warnings`。L-Paper 的 LaTeX 論文不經過這一段（圖檔、表格、方程式直接來自原始檔）。

## 每一頁都對著東西講（作者 2026-11-20）

一頁只有條列，看起來就是沒消化過的 AI 簡報。所以除了封面、Take home messages、結尾，每一頁都要有它在講的東西，規劃時先評估哪一種最適合那一頁（三種格式並列，不是誰替補誰）：

1. **論文的圖、表、方程式**（`figure`／`table`／`equations`／`kv`）。
2. **流程圖**（`flow` = 2–6 個方塊的標籤，依序）：步驟、年表（以前 → 轉變 → 這篇）、問題 → 原因 → 解法。組裝時用 lore-flow 畫成一張圖（原生形狀的群組，帶規格，「編輯流程圖」可以重新打開），方塊用流程圖的配色、文字用這份簡報的字型。
3. **空的圖框**（`suggest` = 建議放什麼圖）：流程圖取代不了的圖（照片、曲線）。圖框只在編輯畫面顯示「建議放：…」，放映、PDF、PPTX 不輸出。

每一頁有一句結論（`takeaway`，紅字），每一行都要通向它、說出要看圖上的什麼。標題寫這一頁要說的事，整份不重複。大綱不符合就退回重排一次（`plan::check_plan`），仍不符合的列在警告。

備忘稿（`notes`）每頁 2–4 句，補上投影片沒寫的解釋；語言由開始畫面的「備忘稿」決定（`zh`｜`en`，預設 `zh`），與內文語言無關。

主架構圖的逐行講解不複製投影片：每一行是一個文字框，各帶一個「出現」動畫（按一下出一行）；一頁一個標題，PDF 一次顯示全部。**沒有在真的 PowerPoint 上測過動畫。**

守衛：`tests/todeck.rs` 的 `assert_no_text_only_page`。

## 主架構圖：分段講解

作者要的樣子（2026-11-19）：流程圖要大，分成幾段；講其中一段時那一段清楚、其他很淡，講解的字在淡的那一側一行一行出現；再換下一段。不可以每頁都放一張小小的同一張圖。

- **模型真的看圖**（`sc-figure-parts`，`todeck/figparts.rs`，Opus → Sonnet）：大綱之前，把論文所有有圖檔的圖給模型看，回答哪一張是方法的主架構圖、要不要分段、沿哪個方向切、切在哪（切線位置是圖寬或圖高的比例）、每段的名稱與一句說明。切線再移到最近的空白處（`lore_docsrc::crop::ink_profile`／`snap_cut`）；找不到空白就照模型的位置並列進 `warnings`。
- 大綱照這些段落寫：總覽頁放整張圖（滿內容寬度＋圖說）；每一段一頁「方法」（`module` = 第幾段，最多 4 行、每行 60 字，不放方程式）；方程式放在 `module = 0` 的方法頁或損失頁（那些頁不再放這張圖）。模型判斷不必分段時：圖只在總覽頁出現一次。
- 投影片（`build.rs`）：整張圖放滿內容區（寬圖 = 滿寬；較高的圖放滿高度，寬度不到八成時列進 `warnings`），整張用底版的 `ghost.picture_alpha` 變淡，再把這一段原樣疊回去；講解的字放在比較大的那一塊淡區上（底下一層半透明的白，字才清楚）。每一行一張複製頁（`LineReveal` → `expand_reveals`：第 k 張有前 k 行），一份簡報可以有很多段這樣的頁。
- 微調的「方法拆幾個模組」（2–5）或換了主架構圖：`slides` 階段請模型依指定的段數再看一次圖，並把現有的方法頁分配到各段（結果存在工作資料夾 `figparts-<圖>-<段數>.json`，重做投影片不會再問）。
- 模型兩次都答不出可用的結果：平均切成指定的段數（沒指定就 2 段），並在 `warnings` 說明。沒有引擎而且還不知道主架構圖是哪一張：方法頁不放圖，也會說明。
- `outline` 的結果多了 `parts`（`{figure, split, axis, cuts, parts: [{label, about}], by, why}`）；`slides` 的結果有 `planned`（大綱頁數）、`parts`（段數）、`partsBy`（`model`／`equal`）、`calls`。

## 「不熟」標示（守備範圍）

- 定義：一個詞是「不熟」＝它是領域詞（縮寫、多字詞組、或知識卡的別名），而且知識庫沒有它，或知識卡狀態是「要學」。常用字永遠不算（沿用 L-Reader 的字頻過濾）。
- 範圍：詞、句（含不熟的詞且是背景／方法敘述）、頁（大部分內容不熟）。
- 呈現：新的強調標籤 `learn`。編輯時：詞與句是紅字加紅色點底線，頁是縮圖角落的紅色小標。上方一個切換「標示我不熟的／報告模式」。**匯出、列印、放映一律自動去掉**，不和「關鍵結果」的紅色粗體混淆。
- 微調視窗列出「這 N 個詞不在你的知識庫」，每個詞可勾「我懂」清掉標示；「同時存進知識庫（記得）」是另外的勾選，不自動。

## 排版用字微調（polish，草案）

只檢查與修正下列項目，不改寫句子：
1. 文字溢出框、字級低於標籤規定。
2. 每頁條列數超過風格摘要的上限。
3. 同一種頁的標題長度與句型一致。
4. 圖說與 `Ref:` 有沒有缺。
5. 每頁最多一個強調（關鍵結果紅色粗體）。
6. 用詞精確：同一概念全份簡報用同一個詞；符號用對照表登記的名稱與寫法；Fig./Figure 依簡報風格。
每一項修正都列在微調報告裡，可逐項復原。報告同時存在工作（`polish.json` 檢查點）與簡報資料夾（`decks/<name>/polish.json`）；編輯器讀後者，復原／套用走 `POST /api/decks/{name}/polish/{item}`（Rust 的編輯與復原紀錄）。

## 以後（記錄，尚未做）

- **知識背景文件**：把作者不熟的內容（例如醫學背景、某個新方法所依據的舊方法）抓出來，整理成一份為作者客製的背景介紹，等於「屬於自己的 review paper」，會是寫書功能的起點；投影片上不熟的部分連到這份文件。
- **圖重畫成原生流程圖**：`source` 階段的主架構圖可以選擇用 lore-flow 重畫成可編輯的流程圖，而不是截圖。

## API（L-Slide 伺服器）

實作：`commons/crates/lore-jobs`（通用的工作執行器，L-Slide 以 `sc_server::jobs` 轉出）與 `src/todeck/`（這條流程的各階段）。所有路徑在 L-Slide 的伺服器上；`/api/todeck*` 也回應 127.0.0.1 / localhost 的跨來源 GET/POST（`DELETE` 與 `PUT` 只給 L-Slide 自己的頁面）。時間是 Unix 毫秒。錯誤：HTTP 狀態碼 + `{ "error": "一行中文" }`。

### 工作物件（job）

`GET /api/todeck/jobs/{id}` 與其他工作路徑都回傳這個形狀：

```json
{
  "id": "j19a6f3c2b1d4",
  "title": "Two-Stage Grading of Synthetic Lesions",
  "created": 1762850000000,
  "state": "running",
  "direct": false,
  "step": 5,
  "doneSteps": 4,
  "total": 8,
  "seq": 42,
  "resumable": false,
  "error": null,
  "failure": null,
  "deck": null,
  "result": null,
  "stages": [
    { "id": "source",   "label": "讀論文",   "state": "done",    "model": false, "ms": 820, "started": 1762850000100,
      "counts": { "fig": 3, "figTotal": 4, "table": 2, "eq": 5, "sections": 6, "refs": 31 } },
    { "id": "notation", "label": "對照表",   "state": "done",    "model": false, "ms": 12,  "counts": { "entries": 14 } },
    { "id": "terms",    "label": "名詞",     "state": "done",    "model": false, "ms": 40,  "counts": { "terms": 37, "unknown": 6 } },
    { "id": "profile",  "label": "風格",     "state": "done",    "model": false, "ms": 5,   "counts": {} },
    { "id": "outline",  "label": "大綱",     "state": "running", "model": true,  "ms": 41200, "started": 1762850001000,
      "chars": 5310, "lastActivity": 1762850042100, "agoMs": 700, "counts": {} },
    { "id": "review",   "label": "微調",     "state": "pending", "model": false, "ms": 0, "counts": {} },
    { "id": "slides",   "label": "投影片",   "state": "pending", "model": false, "ms": 0, "counts": {} },
    { "id": "polish",   "label": "排版用字", "state": "pending", "model": false, "ms": 0, "counts": {} }
  ]
}
```

- `state`：`running`（在跑）· `review`（停在微調，等作者）· `done` · `failed` · `stopped`（按了停止）· `interrupted`（上次 app 結束時還沒做完：伺服器啟動後從快取讀到的 `running` / `review` 都變成這個）。
- `step`：目前（或下一個要做）的階段，從 1 起算 —「上次做到第 N 步，繼續？」的 N。`doneSteps`：已完成（含略過）的階段數。`resumable`：`stopped` / `failed` / `interrupted`。
- `stages[].state`：`pending` · `running`（`review` 等作者時也是 `running`）· `done` · `failed`。直接產生時 `review` 是 `done` 加 `"skipped": true`。
- `ms`：該階段已花的時間（取樣當下；進行中的頁面自己往上加）。`chars` / `lastActivity` / `agoMs` 只在 `model: true` 的階段：模型串流到目前的字數、最後收到字的時間、距今毫秒。
- `counts`：`source` `{fig, figTotal, table, eq, sections, refs}`（圖 n/N：`fig` 有圖檔的，`figTotal` 全部）· `notation` `{entries}` · `terms` `{terms, unknown}` · `profile` `{voice}`（語氣範例的句數）· `outline` `{slides, parts, suggestions}`（完成後）· `review` `{}` · `slides` `{slide, slides}`；`source` 與 `outline` 另有 `warnings`（幾件）、`outline` 有 `calls`（模型呼叫次數）（第 n/N 頁）· `polish` `{checks, fixes}`。
- `error`（`failed` 時）：一行中文。`failure`：`{ stage, status, error, alternative? }`，`alternative` 與 lore-llm 的錯誤內容相同（頁面據此提供「改用本機／改用 Claude」= 帶 `engine` 的 `resume`）。
- `deck` / `result`（`done` 時）：簡報資料夾名稱；`{ name, slides, planned, engine, parts, partsBy, modules, calls, notation, unknown, warnings, polish: {checks, fixes} }`（`unknown` = 標示的不熟名詞數）。`…/stages/polish` 是完整的微調報告（`{checks, fixes: [{slide, object, rule, what, before, after, changes}], items}`，`items` 含用詞一致的建議）。
- `options`：`{length, lang, style}`，目前的參數（微調視窗據此顯示）。
- `review`（只在 `state: "review"` 時）：

```json
{
  "outline": [ { "id": "s3", "kind": "overview", "title": "Big picture", "figs": ["fig1"], "point": "" } ],
  "figures": [ { "id": "fig1", "label": "圖 1", "caption": "…", "thumb": "/api/todeck/jobs/<id>/img/fig1.png" },
               { "id": "tab1", "label": "表 1", "caption": "…", "thumb": null } ],
  "unknown": [ { "term": "InfoNCE", "n": 4 } ],
  "suggestions": [ { "op": "retitle", "id": "s9", "title": "Main results", "reason": "結果頁標題太籠統" } ]
}
```

  `kind` 是頁的角色：`cover background challenge overview method loss training results ablation visual opinion takeaway thanks`。`figs` 是這頁用的圖／表 id（圖與表各最多一個）；`point` 是 take-away，沒有就是第一個要點。`thumb` 是同源的圖（LaTeX 的表沒有圖）。

  `suggestions`（`sc-outline-review`，Opus → Sonnet，`todeck/suggest.rs`；最多 5 個，可為空；模型不能用或回答不合格式就是空的，工作不會因此失敗）：每個是對大綱列的一個修改加一行 `reason`（繁中）。`op`：`add {to, kind, title, point, fig?}`（插在 `to` 那列後面，`""` = 最前面）· `remove {id}` · `move {id, to}` · `retitle {id, title}` · `point {id, point}` · `figure {id, fig}`（`""` = 不放圖）· `merge {id, with, title?}`（`with` 併入 `id` 並刪掉）· `split {id, title, point}`（同種類的第二頁接在後面）。id 都是這個 `outline` 的列；封面與致謝不會被改。頁面在自己的列上套用，送出的仍是下面的微調內容（新頁用沒有的 id，例如 `n1`）。

### 路徑

| Method | Path | Body | 回傳 |
|---|---|---|---|
| GET | `/api/todeck/papers` | | `{pc: [{path, title, name}], lr: [{sha, title, pdf}]}` |
| GET | `/api/todeck/people` | | `{people: [{id, name}], presenter, supervisor}`（L-Paper 的人選；後兩個是上次選的 id 或 `null`） |
| POST | `/api/todeck` | `{source: {kind: "pc", path} \| {kind: "lr", sha}, style?, length?, lang?, engine?, presenter?, supervisor?, mode: "review" \| "direct"}`（`presenter`／`supervisor` = 人選的 id，`""` = 不放，沒給 = 上次的選擇） | `{job: "<id>", …工作物件}`；伺服器在背景開始做 |
| POST | `/api/todeck`（沒有 `mode`） | 同上 | 舊的一次完成：直接產生並等做完，回 `result`；失敗回 `failure.status` 與 `{error, alternative?}`（給還沒改版的「做成簡報」） |
| POST | `/api/todeck/jobs` | 同上（或 `direct: true`） | 201，`{job, …工作物件}` |
| GET | `/api/todeck/jobs` | `?unfinished=1` 只列 `interrupted` / `stopped` / `failed` / `review` | `{jobs: [工作物件…]}`，新的在前 |
| GET | `/api/todeck/jobs/{id}` | | 工作物件 |
| GET | `/api/todeck/jobs/{id}/events?after={seq}` | | 長輪詢：`seq` 大於 `after` 時立刻回傳工作物件，否則最多等 25 秒。每秒 GET 一次也可以 |
| POST | `/api/todeck/jobs/{id}/stop` | | 工作物件（`stopped`）；進行中的模型請求被中止，工作的執行緒被釋放（`…/cancel` 同義） |
| POST | `/api/todeck/jobs/{id}/resume` | `{engine?: "local" \| "claude"}` | 工作物件；從第一個沒有檢查點的階段接著做；正在跑時 409 |
| POST | `/api/todeck/jobs/{id}/review` | 見下（`PUT` 同義） | 工作物件；不在 `review` 時 409 |
| POST | `/api/todeck/jobs/{id}/params` | `{length?, lang?, style?}` | 工作物件；只在 `review` 時（否則 409），值不認得 400。`length` 或 `lang` 變了：刪掉 `outline` 以後的檢查點，工作從 `outline` 重跑，做完再停在 `review`；只改 `style`：不重跑 |
| DELETE | `/api/todeck/jobs/{id}` | | 204；刪掉這個工作的快取資料夾（正在跑時 409） |
| GET | `/api/todeck/jobs/{id}/stages/{stage}` | | 該階段的檢查點內容（`outline`：`{plan: {venue, keywords, slides: [PlanSlide…]}, questions: [], parts, engine, suggestions, warnings, calls}`；`terms`：`{terms: [{term, unknown, n, kind, card}]}`）；沒做完 404 |
| GET | `/api/todeck/jobs/{id}/img/{file}` | | 論文的圖（PNG / JPEG） |

桌面版連結：`lore-sc://job/<id>` → `/?job=<id>`（`LoreLinks.open("sc", {job})`）。

### 微調（`POST …/{id}/review`）

```json
{ "skip": true }
```

或

```json
{
  "outline": [ { "id": "s3", "kind": "overview", "title": "…", "figs": ["fig1"], "point": "…" } ],
  "answers": { "audience": "lab", "mainFig": "fig2", "modules": 3, "opinion": false },
  "known": ["InfoNCE"],
  "remember": true
}
```

- `outline`：照頁面的順序；刪掉的列就是不要的頁；`id` 對回模型的頁（新頁可用沒有的 id，配 `kind`）；改了的 `point` 寫回 take-away（或第一個要點）。
- `answers`：每題可為 `null`。`mainFig` 換掉總覽與分段講解頁的圖（模型會再看一次新的圖）；`opinion: false` 拿掉「我的看法」頁，`true` 而沒有就加一頁空的。`modules`（2–5）= 主架構圖分幾段講（見「主架構圖：分段講解」）；`audience` 目前只記錄。
- `known`：勾了「我懂」的詞：送出時寫進 `terms` 的檢查點（`understood`），投影片就不標它們；`remember: true` 另外經 L-Brain 的 API 存成「記得」（它沒開就略過，訊息記在 `review.json` 的 `notes`）。投影片做好之後的「我懂」是 `PUT /api/decks/{name}/learn`。

### 檢查點

`<L-Slide workspace>/cache/todeck/<id>/`：`job.json`（參數、狀態、各階段時間與數字）、每個完成的階段一個 `<stage>.json`（`{stage, output, counts, elapsed_ms, finished}`）、`img/`（論文的圖）。全部原子寫入。檔案讀不到的階段重做，其他不動；`slides` 做到一半當掉時，下次先刪掉那份做一半的簡報（`slides.pending`）。

### 模型設定（`/api/engines`，每個 app 都有）

- `GET` 的 `local` 多了 `idle_minutes`（0 = 不自動釋放，預設 10）與 `can_release`（只有管理本機模型的 app ＝ L-Paper 是 `true`）。
- `PUT {local: {idle_minutes: 15}}`：存在 `private/shared/engine.toml` 的 `[local]`，各 app 共用；L-Paper 的 llama-server 每次檢查時讀它。
- `PUT {local: {release: true}}`：立刻停掉 llama-server；其他 app 回 400。
- 「使用中」以 llama-server 的 `/slots` 判斷（有 slot 在處理，或 task id 有變），所以 L-Slide 等直接打網址的請求也算；app 結束時照舊 `kill_now`。
