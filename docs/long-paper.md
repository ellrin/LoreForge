# 論文（長）與 語句潤飾（規劃）

作者 2026-11-13 的要求。兩件事都是 Lore 的共用功能接到 PaperComposer（和 SlideComposer），不另做一套：
編輯器、模板、圖表、繪圖、流程圖、對照表（lore-notation）、文獻、知識庫、模型路由（lore-llm）、背景工作（lore-jobs）、
找文獻（lore-research）、語音（lore-voice）都用現有的。

## A. 語句潤飾（先做：作者這週就要寫論文）

**是什麼**：選取一段（幾句話）→ 小對話 → 照作者的意見改這一段。作者用講的（或打字）說要怎麼改：
哪些詞保留、連接詞增減替換、結尾太冗……。模型給改寫，作者看差異，「套用」、再說一次繼續改、或放棄。

- **共用元件** `commons/ui/polish.js`（`LorePolish.mount({ source, apply, … })`，像 `makedeck.js`），由 `lore-shell` 提供。
  PaperComposer：選取文字時寫作工具列與右鍵選單多「潤飾」（和「收藏句型」同一個選取對應，不另寫）。
  SlideComposer：文字框裡選取時同樣的入口。套用＝一步復原。
- **語音**：`lore-voice` 的路由從 LoreEcho 抽成 crate 裡的共用 router（`lore_voice::web::router`，像 `lore_flow::web::router`），
  LoreEcho、PaperComposer、SlideComposer 都掛同一個。按住說話（`continuous: false`），講完的字進輸入框，可以再改再送。
  沒有語音時（非 macOS、權限沒開）只用打字。
- **模型**：lore-llm 任務 `polish`（類別 create，預設雲端；設定的「模型」可改）。帶：選取的文字、前後文、文件語言、
  作者的句型庫（PC `phrases.csv`／`expressions.csv`；SC `voice/examples.json`）、按過讚的改寫（few-shot）。
- **LaTeX 不能壞**：`\cite{}`、`\ref{}`、`\label{}`、`$…$`、登記的巨集在送出前換成鎖住的記號，回來後伺服器比對，
  少了或改了就不給套用並說哪個；套用前再跑 lore-notation 的 `check_document`。SlideComposer 的數學式同樣鎖住。
- **暫存與記錄**：每一次提議都記一列（指示、原文、改寫、套用了沒、讚）：
  論文 `draft/polish.jsonl`（同 `draft/` 其他檔，跟著論文走），簡報 `decks/<name>/polish-log.jsonl`。
  小對話視窗可以看這段的前幾次版本，點一下拿回來比。
- **讚**：改寫旁的 👍。按讚的「原文 → 改寫＋你的指示」存進私人的語調庫 `private/shared/polish/liked.jsonl`
  （兩個 app 共用，經 lore-home；不放 `.claude/`、skill、公開 repo）：
  1. 之後潤飾與生成段落時當範例（「作者喜歡這樣改」）；
  2. 同時開出「收藏句型」的拆解，勾選後加進句型庫（PC 現有的 `/api/phrases/add`，不另存一份）。

**已做（2026-11-13）**：
- `commons/crates/lore-polish`：鎖住／還原（`⟦n⟧`；少了、重複、多出 LaTeX 都不給套用並說是哪個）、任務 `polish`（自己的 `tasks.toml`，兩個 app 合併進任務表與「模型」分頁）、
  每次提議一行的記錄、語調庫 `private/shared/polish/liked.jsonl`（只讀寫私人 shared；取最像的 4 筆當範例）、`/api/polish*` 路由（app 提供 `Host`）。
- `commons/ui/polish.js`（`LorePolish.mount({ doc, language, engine, onLike })`＋`LorePolish.open({ anchor, text, prefix, suffix, context, keep, apply })`，lore-shell 提供）：
  差異顯示、套用／再改／👍／關閉、這段之前的版本 1 2 3…、按住說話（沒有語音就不出現）。
- `lore_voice::web::router`：LoreEcho 改用它（路徑與內容不變），PaperComposer、SlideComposer 也掛；兩個桌面版照 LoreEcho 打包語音小程式、Info.plist 加麥克風／語音辨識說明。
- PaperComposer：寫作工具列與右鍵「潤飾」；選取對應用右鍵選單同一套（`PCContext.locate`，新增 `sourceRange` 包兩端）；
  套用在段落編輯器裡 `insertText`（⌘Z 一步）；套用前跑對照表檢查（`replace_check` → `check_document`，只擋改寫新增的）；記錄 `draft/polish.jsonl`；👍 開「收藏句型」。
- SlideComposer：編輯文字框時工具列與右鍵「潤飾」，同一段落；方程式 `$latex$`、換行鎖住；套用 = 一筆「潤飾」交易；記錄 `decks/<name>/polish-log.jsonl`；語氣用 `voice/examples.json`。
- 還沒在真的硬體上試：麥克風與權限提示、真模型的改寫品質。

## B. 論文（長）——畢業論文、review 論文

**和論文同一個東西**：「新論文」旁多一個「新論文（長）」；`paper.toml` 寫 `length = "long"`，模板、編輯器、
圖表／繪圖／流程圖／對照表／文獻／匯出全部一樣（同書的規則：不單獨開發）。只有 AI 討論流程不同。

**討論流程**（延伸「討論草稿」的 `draft/session.json`＋`transcript.jsonl`，可隨時中斷接著做）：

1. **初始化**：Key value（重點）、核心價值、大概要寫的章節。AI 用一個角色（例如指導教授、審稿人；角色寫在私人的 `lens/`）
   問作者想先討論哪些章節。
2. **發散**：作者想寫的全部講出來，題目越多越好；AI 只記錄、補問、不刪。
3. **收斂**：對照 Key value 修剪；作者堅持的保留；定下章節與每章主題。
4. **每章簡介與大綱**。
5. **細部內容**（分章、可以跳著做）：討論細節、要放的 reference、實驗表格。這時兩件事**同時**在背景跑（lore-jobs）：
   - **筆記**：討論中隨時記下每章的重點與關鍵字（`draft/chapters/<id>.md`），可以直接改；
   - **查文獻**：用每章的關鍵字上網找（lore-research，開放取用），找到的列成那一章的候選文獻，作者勾選才進 `refs.bib`／`citations.csv`（寫下用途）。
6. **逐段草稿**：一章確認後，每一段用討論的方式寫：先說這段要講什麼 → 生成（句型庫、語氣）→ 用 A 的潤飾小對話修。

**用講的**：整個討論視窗可以按住說話（同 A 的語音），長篇的討論以雲端模型為主。

**雙語（中文一版、英文一版）**：同一個資料夾兩份 `main.tex`（例如 `main-en.tex`、`main-zh.tex`），共用圖、表、文獻、對照表；
段落一對一配對（段落編號）。改了一邊，另一邊那一段標「待同步」，按一下由模型照改過的那段重譯（鎖住 LaTeX 同 A）；
不會自動覆寫另一邊。

## C. 寫作三階段（2026-11-13）

作者的說法：1) 初稿——寫出想表達的（動機、事實、自己的看法），照文法規則；2) 用字——確定用詞恰當；
3) 個人風格——自己的開頭與結尾、論證順序、結尾用語、主旨佔段落的比例。全部用現有的功能接起來：

| 階段 | 入口 | 帶給模型的 |
|---|---|---|
| 1 草稿 | 寫作「生成」、討論草稿第 7 步 | 要點（動機、事實、看法都要留下）、`style/10-writing.md`、句型庫（語氣鎖定）、`style/patterns.toml` 中**符合這段情境**的指南句構 |
| 2 用字 | 寫作工具列「用字」（整段）、潤飾小對話的「用字」 | 只改用詞、不動句構與順序；程式檢查在這段的「原文 → 建議」；寫作規則 |
| 3 我的風格 | 寫作工具列「我的風格」（整段）、小對話的同名按鈕 | 該章節的風格摘要（開頭、結尾、結尾用語、連接詞、論證順序、主旨比例與位置、長度）＋同章 2–3 段他自己的範例段落 |

**指南不是作者的語氣**：`_thesis-statement.md` 這類寫作指南的例句不是作者的句子。它們只放在 `style/patterns.toml`
（`source = "guide"`），只當「這種情境適合這種句構」：程式依章節／段落功能／位置挑（Introduction 最後一段或 contribution → 論點句；
比較 → compare_two；每段第一句 → 主題句；摘要不加），標題寫明「來自寫作指南、不是作者語氣、只借結構」。
擋住的地方：`style_text` 不送它；「收藏句型」遇到指南句（或標 `guide` 的列）略過；潤飾的句型與生成的句型庫檢索略過標 `guide` 的列；
讚只記預設名稱；我的風格的摘要與範例只取作者自己的列（排除 `guide` 標記與指南句）；SlideComposer 的 deckvoice 只讀他的簡報。

**Key value 檢查**：Key value 當論點句判斷（任務 `pc-thesis-check`，一次小呼叫、JSON schema），固定的問題代碼（中文標籤）＋1–2 個改寫，
點了才用；標準取 `style/_thesis-statement.md`（有上限），沒有就用公開程式裡的通用標準（不含範例）。研究論文的論點要「能用證據支持」。

**風格摘要放哪**：完全由句型庫算出（不需模型），放在快取層（系統快取資料夾 `PaperComposer/style-profile/`），
鍵是三個 CSV＋`patterns.toml` 的內容；內容沒變不重寫。工作區沒有「這篇是作者自己的論文」的標記，所以論文本身不讀（生成的草稿也就不會回流）。

**已做（2026-11-13）**：`pc_core::patterns`、`pc_core::thesis`、`pc_core::styleprofile`；lore-polish 的預設（`Preset`、`/api/polish/presets`、`polish.js` 按鈕）；
PaperComposer 的「用字」「我的風格」、Key value 檢查（討論草稿第 1 步、論文上方列、匯入）；作者私人的 `style/patterns.toml`。SlideComposer 有預設機制，不提供預設。
還沒在真的硬體上試：真模型的判斷與改寫品質。

## D. 期刊寫法（2026-11-14）

作者的說法：模板就是要投的期刊；格式（class、選項、字數上限）是查得到的小事，模板真正要帶的是**這本期刊的習慣與章節安排**。
選了期刊後，抓這本期刊約 5 篇近兩年的開放取用文章，算出章節標題、各章的重點與**比例**（臨床價值、公式說明、前人研究、資料集……各佔多少）、
大家都用的短語、必備的論點（例如一定要寫優缺點），大約一年更新一次。

**一律以期刊為單位**：一本期刊一個模板（`publishers/<id>/`），它自己的 `[venue]`（是哪一本，OpenAlex 來源，作者確認一次）、
`[submission]`（投稿要求，作者那邊查好的資料）與期刊寫法（`venues/<id>/profile.toml` + `profile.md`）。
出版社的通用模板（有別的模板 `based_on` 它，自己沒有 `[venue]`；或 `role = "base"`）是**基礎格式**：保留、不出現在選單，只當新期刊的底；
系列／多會議的總稱（`role = "umbrella"`）也不出現。已經用通用模板的論文照常能用，「整理與文獻」卡片的期刊列多一個「選期刊」，按了才換（`draft::set_paper_template`，章節檔每個字保留，舊 main.tex 存進 `draft/archive/`）。

**流程**
1. 選單裡每本期刊多一行它的寫法（章節順序 · 約幾字）。點一本 → 下方的「期刊寫法」面板。
2. 沒確認過是哪一本：面板顯示「哪一本期刊？」，用模板名稱（去掉年份與括號）搜 OpenAlex 來源，作者點一個 → 寫進 template.toml `[venue]`。筆形按鈕可以換。
3. 「整理寫法」→ 背景工作（lore-jobs：找文章 → 下載與讀取 → 標記段落 → 整理寫法），面板顯示進度、可以停止與繼續；作者可以繼續做別的事。超過 12 個月的寫法按鈕變成「更新」，沒有彈窗。
4. 搜尋框打一個不在清單裡的名字 → 「新增期刊「…」」：選 OpenAlex 來源 → 依 `venues/requirements.toml`（作者的投稿要求資料，見下）或出版社名稱（含縮寫：Institute of Electrical and Electronics Engineers → IEEE）挑基礎格式 → 建立這本期刊自己的模板（`role = "journal"`、`[venue]`、有資料時 `[submission]`）→ 自動開始整理寫法。挑不出基礎格式時列出基礎格式讓作者點一個。同一本（同 OpenAlex id，或同名而還沒確認的模板）不會建第二個。
5. 用期刊模板建新論文：有期刊寫法時，章節（含常見小節）照這本期刊的安排與命名產生（`venueprofile::skeleton`，標題轉成 LaTeX 安全的 ASCII，章節檔是空的）；還沒有時是一般的空章節（Introduction、Related work、Method、Result、Discussion、Conclusion，和以前一樣），並在背景開始整理（會用網路與模型）。

**抓哪些文章**（`lore-research::venue`）：OpenAlex `works?filter=primary_location.source.id:<S…>,is_oa:true,type:article,from_publication_date:<兩年前>&sort=publication_date:desc&per-page=40`
（作者的論文是 review 時 `type:review`）。只送期刊的 OpenAlex id（來源搜尋時是期刊名稱）；論文自己的關鍵字只在本機排序候選（標題、OpenAlex 關鍵字的重疊），Key value、標題、作者、e-mail 從不送出，`mailto` 預設關。
每篇依序試：出版社的正式版 → PubMed Central → 其他典藏的正式版 → 接受稿 → arXiv（預印本可能和期刊版不同，所以最後；授權用 arXiv 自己標的）。
**只用有標明授權的開放取用副本**，記下讀的是哪一份與它的授權。下載到快取（`<系統快取>/PaperComposer/venue/pdf/`，可以整個刪）；抽出的文字只在工作的快取資料夾，期刊寫法裡沒有全文（只有標題、數字、比例、5 個字以內的短語）。
下載到的是網頁、PDF 讀不出章節的就換下一篇，直到 5 篇；跳過的理由與篇數記在 `skipped`。

**怎麼算**（`pc_core::venueprofile`，全部程式算，模型只標記）：
- 讀 PDF：lore-docsrc（`render_crops = false`），章節、段落、圖表說明、編號公式（含所在章節）。
- 模型（任務 `pc-venue-profile`，預設 `claude:sonnet`，備援 `local:current`，一篇一次呼叫）只回答：每段的內容類別（基本 16 類：臨床／應用價值、問題與動機、前人研究、貢獻與目的、方法描述、公式與推導、資料集、實驗設定、結果、消融、比較、討論：優缺點、限制、未來工作、結論、聲明；這本期刊另外需要的最多 2 個 `x-…`，之後的文章沿用）、每章的角色、文章做了哪些論點（優缺點、臨床意義、條列貢獻、統計檢定、和最新方法比較、外部驗證、可重現細節）。回答先比對文章再用：不認得的類別、缺的段落由程式補。
- 程式算：每章字數與佔全文比例、每章各類別比例（各篇比例的平均與範圍，不是字數加總）、段落順序（moves，例如引言：臨床價值 → 問題 → 前人 → 貢獻）、常見小節與標題的實際寫法（Materials and Methods 4/5）、圖／表／編號公式數量與公式集中在哪章、每百字引用數（編號 `[n]` 與作者–年份；上標引用讀不出來時整個不提）、
  結構習慣（相關研究獨立成節或寫在引言、資料獨立成節、結果與討論合併、討論與結論分開、限制有自己的標題、貢獻條列、圖 1 是整體流程）、必備論點與聲明（資料取得、程式碼、倫理）——每一項都記「5 篇中幾篇」；
  常見短語：2–5 個字、至少 3 篇都出現的 n-gram（取最長的），記它最常出現的段落類別（情境），從不存句子。

**給草稿與審稿用的接點**（pc-core，討論草稿的檔案由另一個工作改，這裡只提供函式）：

| 函式 | 給誰 | 內容 |
|---|---|---|
| `venueprofile::current(ws, template, journal.meta.venue.as_ref())` | 都要先拿到 `Profile` | `None` = 還沒有寫法、模板沒確認是哪一本、或寫法是為另一本期刊做的（換過期刊）：照舊 |
| `Submission::from_table(journal.meta.submission.as_ref())` | 同上 | 投稿要求（寬鬆讀取） |
| `venueprofile::outline_block(&p, sub)` | 大綱（`draft/prompts.rs` 的 outline 提示詞、`pc-draft-outline`）| 章節順序與命名、各章字數與比例、段落順序、結構習慣、投稿要求 |
| `venueprofile::prompt_block(&p, section_title, sub)` | 逐段／逐章草稿（`pc-draft-*` 的 section 提示詞、`/api/compose`） | 這一章的標題、moves、各類別目標比例（含範圍）、這章常見的必備論點（幾篇）、這種情境的短語；`None` = 這本期刊沒有這種章 |
| `venueprofile::emphasis_block(&p)` | Opus 的「預想選項」（`pc-draft-candidates`） | 期刊花最多篇幅的類別、多數文章都會做的論點；只當偏好，不改問題 |
| `venueprofile::audit_checklist(Some(&p), sub)` | 審稿（`pc-draft-audit`）| `CheckItem { id, label, source: submission｜venue, section, count, of, required }`：投稿要求的聲明與章節（必須）＋期刊多數文章（≥ 60 %）的習慣，例如「5/5 篇討論優缺點，你的草稿沒有」 |
| `venueprofile::skeleton(&p)` | 套用版型（draft 的 `PUT /template` 之後）| `Vec<OutlineSection>`，同新論文 |

建議的掛法（後續工作）：`draft/prompts.rs` 組 outline／section／audit 提示詞時，從 `paper.toml` 的 `journal` 取模板 id，`current` 一次，把對應的 block 加在提示詞的「期刊」段；
審稿把 `audit_checklist` 每一項變成固定的問題代碼（`venue:<id>`），只在沒寫到時提出。區塊開頭已經寫明「這是期刊的習慣、不是作者的語氣、不抄句子」。
`GET /api/venue/t/{tpl}/prompt?section=` 可以直接看這些區塊長什麼樣子。

**投稿要求的資料**（作者那邊整理，放 `venues/requirements.toml`；`POST /api/venue/requirements/apply` 寫進各模板的 `[submission]` 與 `role`）：

```toml
[template."<模板 id>"]                 # 已有的模板
base = "<基礎格式 id>"                 # 選填
role = "journal"                       # 選填：base／umbrella 讓它不出現在選單
[template."<模板 id>".submission]
class = "…"                            # 以下全部選填，型別錯的那一項略過、其他照讀
class_options = "…"
pages = 10
words = 7000
abstract_words = 250
abstract_structure = ["Background", "Methods", "Results", "Conclusions"]   # 或 "unstructured"
keywords_min = 3
keywords_max = 6
highlights = true
required_statements = ["Data availability", "Ethics approval", "Conflict of interest"]
required_sections = ["Limitations"]
sources = ["https://… 作者須知的網址"]
checked = "2026-11-14"                 # 超過 12 個月面板把投稿要求標成舊的

[journal."<ISSN-L、ISSN 或 OpenAlex id>"]   # 還沒有模板的期刊（新增期刊時用）
base = "<基礎格式 id>"
[journal."<…>".submission]
…
```

**跨 app（提案，作者決定）**
- LoreReader：下載的 5 篇要不要進書庫？現在不進（只在快取）。提案：面板的文章清單每篇一個「加入 LoreReader」，用 lore-research 的 `import_into_reader`（LoreReader 沒開就排隊）。
- KnowledgeBase：期刊寫法做成一張卡（期刊名、章節、必備論點）？提案：只在作者按「存成知識卡」時寫，避免把期刊資料混進學習卡。
- SlideComposer：研討會的投影片風格（頁數、版面）不在 OpenAlex 裡；提案之後另做（研討會的開放論文集 CVF、OpenReview 不在 OpenAlex 的開放取用標示裡，這輪不寫爬蟲，面板會說找不到）。
- 寫書：寫書用的是書的模板，不是期刊；不接。
- lore-polish：期刊短語當潤飾的建議（「這本期刊常說 …」）？提案：潤飾小對話多一個「期刊用語」預設，只列這段情境（類別）的短語，作者點了才用。
- 討論草稿的「預想選項」（另一個工作在做）：`emphasis_block` 可以讓候選答案偏向期刊重視的內容；只提案，不改它的檔案。

**作者可以改的預設**
- 5 篇、近兩年、研究文章（review 論文才抓 review）；每本期刊一年更新一次（不提醒，只換按鈕）。
- 「多數」= 5 篇中至少 3 篇（60 %），審稿清單只放多數文章有的習慣。
- `[submission]` 和既有的 `[limits]`／`[word_limits]`／`[requires]` 有重疊（字數、頁數、關鍵字數、highlights）。目前兩邊各自保留，字數狀態列照舊讀 `[word_limits]`；要不要以 `[submission]` 為準、或套用時同步過去，請作者決定。
- arXiv 副本只在其他副本都拿不到時用；授權記 arXiv 自己標的（常是 arXiv 的非專屬授權）。

**限制**：OpenAlex 的 `type:article` 與期刊 id 篩選、出版社 PDF 連結常給網頁（會換下一篇）、PMC 的 PDF 連結可能要驗證，都還沒有在真的網路上試過；
上標引用讀不出（引用密度就不提）；研討會論文集多半不在 OpenAlex 的開放取用標示裡。

## E. 待補（還在等資料的結果）與預想選項（2026-11-14）

作者的情況：實驗做完九成，剩下一成在等資料，但論文一定要有它們；要先寫，並且標出哪裡還沒好。
「一張表還差一個結果（一列、一欄或一格）」要能在討論時說明，修訂時照這個說明補。

**記錄**（第 2 步）：每一項待補（`session.json` 的 `pending`，編號 P1、P2…）寫量什麼、等什麼資料、預期方向、預計何時、屬於哪個實驗；
結果表格點一格、標題（整欄）或第一欄（整列）就標成那一項等資料，也可以加一列／一欄「等資料」。記錄裡寫 `?` 的結果與格子自動變成待補。
重新整理記錄後，表格依標題找回來；已回填的數字留在記錄裡；記錄裡等到的數字會自動開一個回填修訂（方向在對話裡說）。

**對話**（第 3、4 步）：有待補時多一組「待補項目」（`wp`），每一項問：是什麼、為什麼必要、預期什麼、結果相反時要改什麼；
作者的話濃縮存在那一項（`note`），四項都有才算這組完成。

**草稿**（第 6、7 步）：大綱每段標它依賴哪幾項（`pending`）；草稿照預期方向寫（不寫 may／might），用 lore-notation 登記的記號：
`\pending{P1}{external Dice}`（數字）、`\pendingcell{P1}{Ours / Dice}`（表格一格）、`\pendingpara{P2}{這段會寫什麼、放哪張表}`（整段）、
`\pendingdep{P1}`（依賴它的句子，包括摘要、討論、結論）。記號帶編號，所以「哪一句依賴哪一項」直接從論文原始檔讀出，不會因為複製貼上而失聯。
大綱說某段依賴 P1、草稿卻沒有標記時，那段會重寫一次（避免編出數字）。草稿 PDF 裡記號是框起來的深橘色（`#C2400D`，文件色，不跟介面主題），
Word 草稿是 `[P1: …]` 文字。對照表檢查不會把記號當成縮寫、符號或手打的參照；潤飾會鎖住記號。

**審稿**（第 8 步）：開頭是待補清單（每一項在哪、哪些句子依賴它）；有記號的句子不會被說成「缺數字」或「宣稱過度」（提示裡說、程式也再濾一次）。

**回填**（第 9 步，用修訂，不另做一套）：「填結果」→ 每個數字與每一格的值 → 符合預期／與預期相反／不確定 → 回填。
- 符合預期：程式自己做修改方案（只換數字、拿掉依賴記號，摘要也改），不叫模型；逐項套用。
- 相反或不確定：開一組修訂對話，帶著當時的說明與所有依賴句；修改方案由模型改寫依賴的句子，數字一律由程式填，依賴過的段落都標「要改寫」，沒被模型改到的也列出來。
- 最後一個記號填完，`\input{pending}` 自動拿掉。

**投稿版**：匯出選單「PDF（投稿版）」與 LaTeX 壓縮檔在還有記號時不輸出，逐段列出還缺什麼（點了跳到那一段）；一般 PDF、Word 是草稿版，照常輸出。
論文畫面底下的狀態列有「待補 N」，點一下跳到下一個有記號的段落。

**預想選項**：題組規劃完，背景用 Opus（`pc-draft-candidates`，可在「模型」改）讀實驗記錄與 Key value，替每個要問動機、理由、觀察、解讀或待補預期的項目
想 2–3 個可能的答案，每個附依據（E3、T2、H1、R1、P1）。程式檢查：依據要真的存在、文字裡的數字要和記錄一字不差，否則丟掉。
對話時那一項的選項出現在問題下面：點一個（或兩個合併）放進回答框可以再改，按 1／2／3 也行，說「第二個」也行；「都不是，我自己說」就自己講。
**問答本身不用 Opus**：每一題仍是 Haiku／Sonnet（照設定），選項只在對話開始前批次準備（一次最多 12 項，一份計畫通常 1–3 次 Opus 呼叫），
新加的題組（待補項目、修訂、審稿追問）開始前再準備一次；每一項只準備一次，沒準備好時對話照常，可按「重試」。

**已做（2026-11-14）**：上面全部。測試：lore-notation（記號、檢查不誤報、pdflatex 實際編譯有無 xcolor 兩種）、lore-polish（鎖住）、
lore-docsrc（純文字標記）、PaperComposer（舊 session 載入、提示內容、依賴清單、審稿過濾、回填的程式方案與相反時的標記、投稿版擋下、
Word 草稿、預想選項的依據與數字檢查、選了哪個有記錄、對話從不叫預想選項的任務）、SlideComposer（論文報告保留記號）。
**還沒在真的模型上試**：Opus 的選項品質、Haiku 會不會把問題對到正確的項目（`item`）、模型改寫相反結果的品質。

### 跨 App

- **已接**：SlideComposer 論文報告——表格的待補格顯示 `[pending]`、文字的數字顯示 `[pending: …]`、依賴句結尾留 `[pending]`，規劃提示要求照抄記號、不寫數字或結論；
  lore-polish 鎖住四種記號；lore-docsrc 的純文字轉換保留記號。
- **提議（等作者決定）**：
  1. SlideComposer 的待補格／字用簡報底版的強調色或虛線框（現在是純文字 `[pending]`）；回填後一鍵更新已做好的簡報。
  2. 論文報告的「Take home messages」遇到依賴待補的主張時整條略過，而不是帶記號。
  3. KnowledgeBase：待補項目的「預期 vs 實際」在回填後記成一張卡（自己的預測準不準，給寫書或回顧用）。
  4. LoreReader：讀到別人論文的相同指標時，提示和自己的待補項目比較。
  5. lore-notation 的對照表面板列出待補記號（目前只有 PaperComposer 的狀態列與審稿清單）。
  6. LoreEcho：語音記下「P1 的資料到了：0.74」直接回填。

## F. 實跑（2026-11-14，投稿首稿前）

用 6 篇目標期刊的開放取用文章當「自己的研究」，在暫存工作區從頭跑一次 新論文 → 討論草稿 → 匯出（真模型、真網路），每一步截圖、計時，
邊跑邊修。改了什麼（都有測試）：

- **期刊先選**：第 1 步「投稿期刊」（和 Key value 一起）。清單點一本就收成一行＋它的期刊寫法；按「儲存，下一步」才套用版型
  （`PUT /draft/template {stay: true}`，不跳步）；空白的論文換成這本期刊的章節（`venueprofile::skeleton`）。之後換期刊先問。
  ★ 版型 步驟保留（舊 session 照常），第 5 步校正有期刊時直接到大綱。
- **期刊寫法接進每一步**（`draft/topaper.rs` 的 `Venue`）：大綱提示詞加 `outline_block`、逐段草稿加這一章的 `prompt_block`、
  預想選項加 `emphasis_block`、審稿加 `audit_checklist`（多數文章有、你沒有的才提，類別「期刊要求」）。另外程式檢查：
  摘要字數、關鍵字個數、投稿要求的聲明在不在、聲明還是不是 `[…]` 預留文字。
- **摘要**：第 7 步最後一章是 Abstract（一段、不分小標、字數照期刊）；**放進論文**把每章寫進論文的章節檔（依角色對到
  「Materials and Methods」「Conclusions」，沒有的章照期刊命名新增），摘要寫進 `\abstract{…}`／`abstract` 環境／它 `\input` 的檔。
  以前草稿只留在 `session.json`（只有「複製」），審稿、待補、匯出讀不到。
- **摘要不是章節**：大綱裡的 Abstract／Keywords 進標題區，不會變成 `\section{Abstract}`；`\abstract{\input{sections/01-abstract}}`
  裡的檔不算章節（以前這種寫法讓 `front.rs` panic，論文打不開）。範本的示範摘要在用大綱建論文時清掉。
- **期刊的聲明**：Springer 範本的示範後記（"Acknowledgements are not compulsory…"、各期刊政策連結）換成 `[submission] required_statements`，
  雙盲期刊的致謝／經費／倫理／作者貢獻寫成 `\blinded{…}{Removed for blind review.}`（作者手寫的雙盲寫法）。
- **雙盲匯出**：`[limits] anonymous` 的期刊，「PDF（投稿版）」同時輸出完整版與盲審版（`export/<名>-blind.pdf`，
  `front::blind_main`：`\blindmode` 打開、作者／單位／e-mail 指令移除，作者自己已經用 `\ifdefined\blindmode` 的不動），
  再用 pdftotext 檢查盲審版裡沒有作者姓名、e-mail、單位，有就不給過。
- **Unicode**：生成、潤飾、修改方案的 ≤ − ⁺ α × ° 等一律換成 LaTeX（`lore_notation::unicode::to_latex`；`$…$` 裡用裸指令）。
- **Opus 拒答**：Opus 的安全檢查偶爾擋下正常的實驗記錄（"safeguards flagged this message"），改用 Sonnet 再試，不落到本機模型。
- **文獻**：貼上的參考文獻清單每篇都加進本篇文獻（沒寫用途時先用一句簡介）；Crossref／PubMed 回 429 時等一下再試（以前 23 篇裡 9 篇失敗）。
- **OpenAlex 額度**：沒有 API key 時 OpenAlex 每個 IP 一天共用一份免費額度，用完回 429 與很長的 Retry-After；
  現在直接說「今天的免費額度用完了，約 N 小時後恢復」，不空等。期刊寫法多一個「用手邊的 PDF…」：選這本期刊的幾篇 PDF，不用網路。
- **PDF 讀章節**：Springer 期刊的章節標題沒有編號、只是字比較大，lore-docsrc 以前一個都讀不到；現在沒有編號的文章用字級判斷（6 篇裡 2–3 篇讀得出，仍待改）。
- **記錄裡的 `?`**：整理實驗記錄只把記錄自己寫 `?`／等資料的結果當待補；記錄沒提到的格子是 `—`。
- **自己設定好的論文**：已經在這本期刊 class 上、自己寫了 `\blinded` 與聲明的 main.tex，選期刊只記下期刊，不重排（以前會用範本重建、丟掉作者的聲明與盲審開關）。
- **pdflatex 不認得的字元**：匯出錯誤說要改成什麼，「全部換成 LaTeX，再編譯」一鍵處理論文檔案（逐段）。
- **預想選項**：一次三批 Opus、依題組順序先好先用；被中斷時已經準備好的保留，不再顯示「重試」。
- **小錯**：`window.PCNotation`、`window.PCOpen` 沒有設（「登記到對照表」打不開對照表）；期刊寫法面板讀不出圖表時不寫成 0；`venues/requirements.toml` 在打開面板時套用。

- **模型寫的 `%`**：草稿寫「39.1%」，LaTeX 把後面整行當註解（字不見、摘要的大括號沒關）；生成、潤飾、修改方案一律跳脫成 `\%`（`unicode::prose_to_latex`）。
  標題區讀摘要檔改用欄位判斷，不再數大括號；放進論文讀不到章節或摘要時不寫、說原因（以前會在後記後面多加一組章節、多一個摘要）。
- **大綱用期刊的章名**（Materials and Methods、Conclusions）以前整章被丟掉；現在依角色對回。
- **摘要字數**：生成時數字數，超過期刊上限重寫一次；投稿版的提醒列出字數、資料夾名稱當標題、範本的關鍵字、「[Author to confirm]」與聲明的預留文字（不擋，作者決定）。
- **記錄的編號**（E1、T2…，記錄裡真的有的）出現在草稿裡會重寫一次。
- **文獻的 `&amp;`**：出版社的 BibTeX 常帶 HTML 實體，寫進論文的 .bib 時換成 LaTeX；編譯前清掉上一次的 .bbl（一筆壞掉的書目以前會讓 BibTeX 永遠跑不到）。
- **表格**：所有表格格子的 − ≤ ± 換成 LaTeX（貼上的試算表、記錄整理出的結果表）。
- **流程圖的出發點**：lore-docsrc 讀不了期刊 class 的專案時，用論文自己的標題、摘要與方法一節。

還沒做（記下來）：草稿常把已知數字也標成待補、依賴句標太多（一項 11–27 句）；「與預期相反」的回填沒有改到摘要裡的論點句；寫作面板的「生成」還沒帶期刊寫法（`/api/compose` 不知道論文）；PDF 讀圖表標題（Springer 的「Fig. 1」）仍讀不到；
6 篇裡有 3 篇讀不出章節；作者名字當成章節標題（「B 作者名」通訊作者記號）。

## 施工順序

1. A 語句潤飾（PaperComposer 先，SlideComposer 同一個元件接上）。
2. B 的「新論文（長）」入口＋初始化／發散／收斂。
3. B 的細部內容＋背景筆記與查文獻。
4. B 的逐段草稿、雙語同步。

## 預設的決定（作者可改）

- 雙語：改一邊 → 另一邊標「待同步」，按了才重譯（不自動覆寫）。
- 語音：作者用講的，模型回文字；要不要唸出來是設定裡的開關，預設關。
- 讚的語調庫兩個 app 共用一份（`private/shared/polish/`）。
- 待補：投稿版 = 「PDF（投稿版）」與 LaTeX 壓縮檔；一般 PDF、Word 是草稿版。草稿匯出會在 `main.tex` 前言加 `\input{pending}`
  並寫 `pending.tex`（lore-notation 產生，作者自己的同名檔不覆蓋），最後一個記號填完時拿掉 `\input`。
- 待補：記錄裡寫 `?` 自動變待補項目；草稿裡記號的說明（`\pending` 第二個參數）是英文。
- 預想選項：每項 2–3 個，每份計畫最多 12 項一次 Opus；每一項只自動準備一次。
