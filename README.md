<p align="center">
  <img src="apps/desktop/icons/logo.png" alt="PaperComposer logo" width="160">
</p>

# PaperComposer

本機運作的論文寫作編輯器：在排版好的畫面上直接編輯，用你自己的句型資料庫輔助寫作與檢查段落，輸出期刊要求的 LaTeX 或 Word。

> 開發初期。已完成：句型資料庫讀取與驗證、句型檢索、硬規則檢查、一次性段落生成（語氣鎖定）、桌面殼與介面雛形。

## 執行

需要 [Rust](https://rustup.rs)（1.85 以上）。

**桌面版**

```sh
cargo run -p papercomposer
```

- Windows：需要 Visual Studio Build Tools（C++）與 WebView2（Windows 10/11 已內建）。
- macOS：需要 Xcode Command Line Tools。
- 打包安裝檔：`cargo install tauri-cli --version "^2"`，再於 `apps/desktop/` 執行 `cargo tauri build`。Windows 產生 `.msi` / `.exe`，macOS 產生 `.app` / `.dmg`。
- 圖示：原始檔為 `apps/desktop/icons/logo.svg`，對應的 1024px 圖為 `logo.png`。換圖後於 `apps/desktop/` 執行 `cargo tauri icon icons/logo.png` 重新產生各尺寸。

**瀏覽器開發模式**（改介面時最快）

```sh
cargo run -p pc-server
```

開啟 <http://127.0.0.1:7878>。

沒有指定工作區時，兩種模式都使用內建示範資料（`workspace-template/`），右上角顯示「連接工作區」。

## 你的私人工作區

句型、模板、論文都是個人資料，**放在你自己的私有 repository**，不要放進這個 repo。結構如下（空白範本見 [`workspace-template/`](workspace-template/)；填好虛構內容的完整範例見 [`private-example/`](private-example/README.md)）：

```
your-private-repo/
  profile/      句型、用詞、規則、範例段落、公式片段
  publishers/   各出版社的 .cls、範例 .tex 與模板設定
  papers/       每篇論文一個資料夾（main.tex、sections/，以及 paper.toml：Key value 與作者）
  prompts/      自訂提示詞（可選）
  config.toml   App 記住的介面設定：淺色／深色、配色、紙張、縮放、引擎、上次開的論文
```

介面設定存在工作區的 `config.toml`（`[ui]`），跟著工作區一起同步，換一台電腦打開也是同樣的樣子；每次開啟時以它為準。

第一次開啟 App 會彈出 **AI 與同步** 視窗（之後點右上角的「AI」開啟），三個步驟都可略過：

1. **工作區與同步**：三選一，每位使用者都指向**自己的**私有 repo，App 不綁定任何特定帳號
   - 建立新的：先在 GitHub 建立一個空白的 **private** repo（旁邊的外連圖示會開啟建立頁面），貼上網址並選擇存放位置；從範本建立資料夾、`git init` 並 push。網址留空則只存在這台電腦
   - 從 repo 下載：第二台電腦用，clone 你的私有 repo；若偵測到是**公開** repo 會提醒先改成 private
   - 開啟資料夾：已經在電腦上的工作區；缺少的資料夾與範本檔會自動補上（不覆蓋既有檔案）
   - 需要同步時才會出現 Git 檢查：是否安裝 Git、commit 作者名稱與 email
2. **本機模型**：偵測記憶體（含 Mac 統一記憶體與 NVIDIA 顯示卡），標示推薦與放不下的模型。按「準備下載」先列出要下載的檔案、大小與來源，**確認後才下載**；下載完成自動啟動並設為預設
3. **Claude**：兩種連線方式，自己選、隨時可切換（右側引擎的 Claude 選單也能切換）
   - **登入帳號（訂閱）**：透過 [Claude Code](https://code.claude.com/docs/en/setup) 用你的 claude.ai Pro／Max 帳號，費用算在訂閱額度內。App 會偵測是否安裝、顯示官方安裝指令、按一下用瀏覽器登入並自動偵測完成。每次呼叫都在空的暫存資料夾執行 `claude -p`，關閉所有工具、不載入任何外掛設定，並移除 `ANTHROPIC_API_KEY` 以免改走 API 計費。只適合自己使用
   - **API key**：按外連圖示開啟 Anthropic Console 建立 key，貼回來儲存（只存在系統金鑰圈），按用量計費

   兩種方式都有「測試連線」，會標明走的是訂閱還是 API。寫作、拆解、段落檢查都使用右側選的引擎，檢索、提示詞與驗證都相同

也可以手動指定位置（擇一）：

- 環境變數 `PAPERCOMPOSER_WORKSPACE=/path/to/your-private-repo`
- 設定檔 `PaperComposer/config.toml` 寫入 `workspace_path = "/path/to/your-private-repo"`
  （Windows：`%APPDATA%`；macOS：`~/Library/Application Support/`；Linux：`~/.config/`）

設定檔在所有 repo 之外，路徑不會被 commit。欄位說明見 [`workspace-template/profile/README.md`](workspace-template/profile/README.md)。

## Git 同步

工作區是 Git repository 時（建議是你的私有 repo）：

- **開啟 App**：自動 pull。只做 fast-forward；兩邊都有新 commit 時會提示你按同步。
- **Ctrl+S**（或點狀態列的同步狀態）：commit 所有修改並 push。commit 訊息依修改的檔案自動產生，例如 `Update my-paper (method, result); profile (phrases)`。
- **另一台裝置先推了**：自動把你的修改接在遠端之後再 push；若兩邊改到同一句，**中止並還原**，你的修改仍保留在本機，不會自動合併。
- **commit 前檢查金鑰**：新增的內容看起來像 API key 或 token 時，拒絕 commit。

使用系統安裝的 `git`，登入交給你電腦上的 Git 憑證管理（Windows 為 Git Credential Manager），App 不保存 Git 密碼。作者名稱沿用你的 `git config user.name` 與 `user.email`。

## 新論文

論文選單的「新論文」開啟五個步驟的視窗。第 3 步的 Key value 填好才能往下；第 4、5 步可以留白，直接按「下一步」。「建立論文」只在最後一步出現，旁邊是資料夾名稱：依標題（沒有標題時用 Key value）自動產生，例如 `explainable-hip-fracture-detection-attention`，可以改，不會和現有的論文重名。

1. **版型**：選期刊或研討會的模板，或空白 article。
2. **作者**：從作者資料庫勾選、排順序，標記通訊作者與同等貢獻。資料庫在 `profile/authors.csv`（姓名、email、ORCID、所屬機構、角色）與 `profile/affiliations.csv`（系所、機構、城市、國家），姓與機構必填，其他可空著；在這裡新增的人會存進資料庫，下一篇直接選，也可以在「資料庫」視窗的「作者」「機構」分頁編輯。作者區塊依版型的寫法產生，不經過模型：Springer Nature（`\author[1]{\fnm{}\sur{}}`、`\affil`）、Elsevier（`elsarticle` 的 `\corref`；`cas-dc` 的 `\cormark` 與 `[orcid=]`）、IEEE 期刊（`\thanks{… is with …}`）、IEEE 會議（`\IEEEauthorblockN/A`）、LNCS（`\inst`、`\orcidID`、`\institute`），其他 class 用通用的 `\author{Name\\ 機構 \and …}`。範例裡的作者整段換掉，其餘不動。
3. **Key value 與大綱**：
   - **Key value 必填**：這篇論文通篇要強調的一件事，可以是願景（例如「提升 XX 效能」）。中文最多 50 字、英文最多 30 個單字，存在 `paper.toml`；論文上方會一直顯示，點一下可修改。匯入現有論文時也要填。
   - **大綱（選填，需要 Claude）**：條列章節與要說的事，Claude 依 Key value 給 3 個標題建議，並排出 section／subsection，每段最多兩句話說明這裡要寫什麼（寫在內文，之後自己改）。有期刊模板時依期刊的章節順序；勾選套用就取代模板的範例章節，前後的標題區與參考文獻區保留。

4. **圖表（選填）**：已經有的結果先放進來。
   - **表格**：從 Excel 或 Google 試算表選取儲存格直接貼上（也可以貼 CSV 或 Markdown 表格），第一列是表頭，下方即時預覽列數與欄數。選要放的章節（大綱架構的章節、模板的章節或空白論文的章節）和樣式（預設用這個期刊的：Springer Nature、Elsevier、IEEE、LNCS、CVPR，其他用 booktabs）。
   - **圖片**：PNG、JPG、PDF 或 EPS，存進論文的 `figures/`（檔名整理成 `fig1-原檔名.png`），以 `\includegraphics[width=\linewidth]` 放進 `figure` 環境。
   - **標題（caption）自動產生**：留白時，貼完表格或選好圖片後由右側選的寫作引擎（本機模型或 Claude）依表頭、列名寫一句標題，不寫結果與數字；也可以按標題旁的 AI 重寫。圖片只看得到檔名，檔名看不出內容時不會亂編，請自己填。建立時還空著的會先補上。
   - 可調整順序；建立時依序放在各章節的最後，產生 `\caption`、`\label{tab:…}`／`\label{fig:…}`，需要的套件自動加進 `main.tex`；之後在頁面上照常編輯。
5. **文獻（選填，需要 Claude）**：把想引用的文章一次貼上，可以夾雜備註（見下方「文獻」的「貼上想引用的文章」）。Claude 先算出有幾篇，再三篇一組查證、建檔、加標籤存進文獻庫，顯示進度條；可以邊匯入邊回到其他步驟。你對每篇的備註（例如「需注意 · Methods · 只支撐…」）會記成這篇論文的引用用途（`citations.csv`）。

**Key value 是生成與檢查的約束**：生成段落時放進規則；檢查加了一條軟規則 R106（`supports_key_value`），判斷段落、圖表、摘要、contribution、討論是否離題。它是黃燈：延伸討論不一定完全對齊主題，不必每段都綠燈。

**清單**：寫作工具列有「項目清單」（`itemize`）和「編號清單」（`enumerate`，例如 *Our contributions are as follows:* 後面列點）。

## 公式

紙張上的數學式用內建的 KaTeX 顯示（離線可用），支援 `$…$`、`\[…\]`、`equation`／`align` 等環境、自動編號與 `\eqref`，並會讀取論文 `main.tex` 與 `macros*.tex` 裡的 `\newcommand`、`\DeclareMathOperator`。這是編輯時的預覽，投稿用的 PDF 仍由 LaTeX 搭配期刊 cls 產生。

## LaTeX 與期刊模板

投稿用的 PDF 由 LaTeX 搭配期刊的 cls 產生，需要安裝 TeX：

| 系統 | 建議 |
|---|---|
| macOS | [MacTeX](https://www.tug.org/mactex/)（完整版，約 5 GB，期刊模板用到的套件都有） |
| Windows | [MiKTeX](https://miktex.org/)（缺套件時會自動下載）或 [TeX Live](https://tug.org/texlive/) |

**用精簡版（BasicTeX）的話**，要自己補套件。IEEE 與 MICCAI／LNCS 範本不用補；其他範本與表格樣式需要：

```sh
sudo tlmgr update --self
# Springer Nature
sudo tlmgr install sttools vruler breakurl apacite appendix wrapfig multirow ncctools rsfs
# CVPR
sudo tlmgr install silence enumitem cleveref currfile helvetic times courier mwe
```

不想用 sudo，也可以裝在自己的帳號：先 `tlmgr init-usertree`，再把 `sudo tlmgr install` 換成 `tlmgr --usermode install`。

**EPS 圖片**需要 Ghostscript 轉檔：macOS 執行 `brew install ghostscript`；Windows 的 TeX Live 已內建，MiKTeX 請另外安裝 [Ghostscript](https://ghostscript.com/releases/gsdnld.html)。PNG、JPG、PDF 圖片不需要。

**編譯時出現 `File 'xxx.sty' not found`**：用下面的指令找出它屬於哪個套件，再安裝：

```sh
tlmgr search --global --file /xxx.sty
```

**期刊模板**放在工作區的 `publishers/<名稱>/`，主要範例命名為 `sample.tex`（見 [`workspace-template/publishers/`](workspace-template/publishers/README.md)）。常用的官方來源：

| 期刊 | 來源 | 放法 |
|---|---|---|
| IEEE 期刊、會議 | [CTAN：IEEEtran](https://ctan.org/pkg/ieeetran) | `ieee-journal/`（`bare_jrnl.tex` → `sample.tex`）、`ieee-conference/`（`bare_conf.tex` → `sample.tex`），各附 `IEEEtran.cls`、`IEEEtran.bst`、`IEEEabrv.bib` |
| Springer Nature | [LaTeX author support](https://www.springernature.com/gp/authors/campaigns/latex-author-support) | `springer-nature/`（`sn-article.tex` → `sample.tex`），附 `sn-jnl.cls`、`bst/` |
| MICCAI（LNCS） | [CTAN：llncs](https://ctan.org/pkg/llncs)；範例論文請到 Springer 或 Overleaf 取得 | `miccai-lncs/`，附 `llncs.cls`、`splncs04.bst` |
| CVPR | [CVPR author kit](https://github.com/cvpr-org/author-kit) | `cvpr/`（`main.tex` 與其 `\input` 合併為 `sample.tex`），附 `cvpr.sty`、`ieeenat_fullname.bst` |
| Elsevier | [CTAN：elsarticle](https://ctan.org/pkg/elsarticle) | `elsevier/` |
| ACM | [CTAN：acmart](https://ctan.org/pkg/acmart) | `acm/` |

以上 IEEE、Springer Nature、MICCAI／LNCS、CVPR 範本已在 macOS + BasicTeX（補上述套件與 Ghostscript）實際編譯通過。

**表格**：論文裡的表格可以直接在畫面上編輯（選取、雙擊打字、增刪列欄、合併、加減線、字體），並套用 22 種樣式（各期刊官方範例的寫法、通用、數據、版面）。每種樣式都以實際編譯驗證；需要的套件會自動加進 `main.tex`。讀不懂的表格寫法會改用 LaTeX 原始碼編輯，不會被改寫。

**版型檢查**：選取表格時，App 會在背景用這篇論文自己的 class 和前置設定，讓 LaTeX 量出表格在四種字體大小下的實際寬度，以及期刊的 `\columnwidth`／`\textwidth`／`\textheight`（約 0.1–0.5 秒，需要安裝 TeX）。工具列顯示佔可用寬度的百分比；放不下或欄數超過樣式的限制時，依樣式規則找出第一個放得下的組合（先縮字體、再跨雙欄 `table*`、最後橫式），一鍵套用。欄數限制以單欄為準，跨雙欄時依可用寬度等比放寬；本來就是跨欄的樣式不會被改回單欄。

## 文獻

| 檔案（私人工作區，會同步） | 內容 |
|---|---|
| `refs/library.bib` | 所有文獻的 BibTeX（書目以這份為準） |
| `refs/notes.csv` | 每篇的一句話、簡介、標籤、DOI、PDF 檔名 |
| `profile/tags.csv` | 標籤清單（可自行編輯；沒有這個檔案時用內建的 16 個） |
| `papers/<論文>/citations.csv` | 每篇文獻在這篇論文裡用來說明什麼 |

- **文獻頁**（上方書架圖示）：搜尋、依標籤篩選、匯入 `.bib`、直接編輯一句話、簡介和標籤；有 PDF 的可一鍵開啟。
- **PDF 不上 git**：每台電腦在文獻頁設定自己的 PDF 資料夾（存在 App 設定檔，不在任何 repo）。App 依檔名、`<key>.pdf` 或檔名中的 DOI 找到對應的 PDF，找不到就顯示本機沒有。
- **匯入 PDF 或 DOI**：文獻頁會列出 PDF 資料夾裡還沒匯入的檔案。匯入時從 PDF 前兩頁讀出 DOI（跨行斷開的也會接回），書目向 Crossref（doi.org）取得，摘要依序向 OpenAlex、Semantic Scholar、PDF 本身的 Abstract 段落取得；**書目不經過模型**。模型只負責寫中文簡介、一句話，並從你的標籤清單挑標籤；頂會、頂刊依 `profile/venues.csv`（`pattern,tag`，沒有時用內建清單）判斷。抓到的標題若和 PDF 內容對不上（例如 DOI 被截斷而對到整本論文集），會標示並請你更正 DOI。每一篇都要你確認後才加入。
- **插入引用**：上方「插入 → 引用…」，搜尋或用標籤篩選、可多選，插在游標處（沒有在編輯時插在段落句尾），可順便記下用途。
- **用 Claude 找新文獻**（插入引用視窗的放大鏡；沒連 Claude 時是灰的）：預設請求是「幫我找關於『段落開頭』的文獻」，可再選標籤引導（回顧→只找 review；新算法→近 3 年；經典算法→依引用數排序；頂會／頂刊→依期刊清單篩選）。Claude 把請求轉成搜尋關鍵字，**文獻一律來自 OpenAlex**（都有 DOI、排除只有預印本的），Claude 再從候選中排序並寫一句為什麼適合；它不能新增或改寫候選。點標題開啟 DOI 頁面，有開放取用版本可直接開 PDF；「加入」會向 Crossref 取得書目、寫好簡介與標籤後放進文獻庫，並勾選待插入。
- **貼上想引用的文章**（文獻頁的清單圖示，或新論文的第 5 步；需要 Claude）：一次貼很多篇，參考文獻清單、標題、DOI、PMID、arXiv 編號都可以，中間夾雜的備註與小標題也沒關係。Claude 只負責拆成一篇一篇，抄下文字裡寫的書目欄位，以及你對每篇的備註（原文照抄），不從記憶補資料。**是否真的存在由資料庫判斷**：
  - 有 DOI：Crossref 上要對得上標題（你寫的簡短標題、或 Crossref 把副標題分開存的，都算對得上）
  - arXiv：向 arXiv 查證，書目用 arXiv 的 DOI（`10.48550/arXiv.…`）
  - PMID：向 PubMed 查證
  - 都沒有：用標題、作者、年份先搜 Crossref，再搜 PubMed（沒有 DOI 的臨床期刊文章多半在這裡），書目取自 PubMed 的紀錄

  找到的直接建檔（書目來自資料庫，簡介與標籤由模型寫），三篇同時進行並顯示進度。需要確認的列出候選讓你選；資料庫都沒有的（例如 Holm 1979，沒有 DOI 也不在 PubMed）可以按「用我貼的書目建檔」，BibTeX 會註明 *Bibliographic data as entered by the author*。在文獻頁使用時，可勾選把備註記到目前開啟的論文。
- **引用一定送得到 BibTeX，作者的 `.bib` 一律不改寫**：`\bibliography{…}` 列出的每個檔案都會讀取；文獻庫裡有、論文的 `.bib` 沒有的引用，寫到 App 管理的 `pc-refs.bib` 並自動加進 `\bibliography{…}`。論文沒有自己的 `.bib` 時，產生一個只含這篇引用的檔案。

## 匯入現有的論文

論文選單的模板欄選「匯入檔案…」或「arXiv…」：可匯入 `.zip`、`.tar.gz`、單一 `.tex`，或直接輸入 arXiv 編號／網址（下載作者上傳的 LaTeX 原始檔）。App 找出主檔（arXiv 依 `00README.json` 指定的檔案），展開 `\input`／`\include`／`\subfile`，每個 `\section`（含附錄）成為一個可編輯的章節檔；標題、摘要、參考文獻等章節之間的內容依原順序留在 `main.tex`。`%` 註解原樣保留，圖片、樣式檔、`.bib` 一起帶過來；只附 `.bbl` 的論文直接使用它。

以 4 篇 arXiv 論文實測（NeurIPS〈Attention Is All You Need〉、IEEE TMI、IEEE TSP、Springer BIT），匯出的 PDF 頁數與 arXiv 版完全相同，逐頁比對只有字型版本造成的細微差異。

**標題、作者、單位、摘要、關鍵字**也在頁面上雙擊編輯（左側導覽最上方的「標題與摘要」可跳到這裡）。編輯框裡是該欄位的 LaTeX 原文，存檔時只替換那一段，`main.tex` 其他內容一字不動；IEEE（`abstract`、`IEEEkeywords` 環境）與 Springer Nature（`\abstract{…}`、`\keywords{…}`、每人一個 `\author*[1]{…}`）的寫法都支援，註解掉的不算。還沒有摘要的論文會顯示「雙擊新增摘要」，寫入 `\maketitle` 之後。

**編輯畫面是預覽，PDF 才是成品**：編輯畫面用 KaTeX 與 App 自己的轉換呈現論文（方程式、圖片、表格、清單、定理、演算法、標題摘要、參考文獻），以便直接在頁面上修改，外觀接近但不保證與 PDF 完全相同。匯出 PDF 一律用期刊的 class 與真正的 LaTeX 編譯。編譯過一次後，畫面上的方程式、圖表、章節與引用編號會改用 LaTeX 算出的實際編號（讀取 `.aux`）。PDF 與 EPS 圖片用 Ghostscript（macOS 沒有時用內建的 `sips`）轉成預覽圖。

## 匯出

上方的下載圖示：點一下輸出 PDF（或按 Ctrl+P），滑鼠移上去可選 Word 或 LaTeX。檔案放在 `papers/<論文>/export/`，資料夾內附 `.gitignore`，**匯出的檔案不會上 git**；編譯的中間檔放在系統的快取資料夾，不會弄亂論文資料夾。

- **PDF**：用期刊自己的 class 編譯（pdflatex；`main.tex` 用了 `fontspec`、`xeCJK` 或寫了 `% !TEX program = xelatex` 時改用 XeLaTeX），自動跑 BibTeX／Biber 並重編到交互參照穩定。編譯前會重新產生這篇論文的 `.bib`。成功時顯示頁數；失敗時列出錯誤、出錯的原文、建議做法，點位置直接跳到那一段。找不到的引用、標籤和超出版面的行數也會列出。
- **Word**：需要 [Pandoc](https://pandoc.org/installing.html)（macOS：`brew install pandoc`）。引用依論文資料夾裡的 `.csl` 排版（從模板建立的論文會一起帶過來：IEEE、CVPR 用 `ieee.csl`，Springer Nature 用 `springer-basic-brackets.csl`，LNCS 用 `springer-lecture-notes-in-computer-science.csl`，取自 [CSL styles](https://github.com/citation-style-language/styles)）；有 `reference.docx` 時套用其中的字型與標題樣式。投稿請以 PDF 為準，Word 適合給共同作者修改。
  轉換前會處理 Pandoc 的常見困難：PDF／EPS 圖與省略副檔名的圖轉成 PNG；`subfigure`、`subfig`、`bbm`、`dsfont`、`mathabx` 等套件的指令給相容定義（論文自己定義的優先）；`.bib` 有語法錯誤時改用 App 解析出的乾淨副本（原檔不動）。以 4 篇 arXiv 論文實測，方程式轉成 Word 原生公式、表格、圖片、引用都在；少數寫法不標準的數學式（例如 `\underset{…}\mathop{…}` 第二個參數沒加大括號）會保留 LaTeX 原文並列在警告裡。
- **LaTeX**：產生 `export/<論文>-latex.zip`，可直接上傳投稿系統、arXiv 或 Overleaf：章節併回單一 `main.tex`，附上編譯產生的 `.bbl`、class／樣式檔、`.bib` 與圖片，不含 App 自己的檔案。所有檔案都放在最上層、沒有子資料夾（Editorial Manager 等投稿系統的要求），`\includegraphics{figures/x}` 等路徑自動改寫，同名的不同檔案會加上原資料夾名稱。會先完整編譯一次，確定能編譯才打包。實測 8 篇（4 篇 arXiv 論文與 4 個期刊範本）解壓到空資料夾單獨編譯，頁數與 App 輸出的 PDF 相同。

## 連接模型

### 一鍵安裝的本機模型

「AI 與同步」第 3 步會從 Hugging Face 下載 GGUF 模型、從 GitHub 下載對應平台的 [llama.cpp](https://github.com/ggml-org/llama.cpp)（Windows 用 Vulkan 版，支援 NVIDIA／AMD／Intel 顯示卡；Mac 用 Metal 版），存在本機資料夾（Windows：`%APPDATA%\PaperComposer`；macOS：`~/Library/Application Support/PaperComposer`；Linux：`~/.local/share/PaperComposer`）。App 啟動時自動開啟上次使用的模型，關閉時一併結束。

**和其他專案共用模型**：設定環境變數 `LOCAL_LLM_HOME`（或在 App 設定檔 `config.toml` 寫 `local_llm_home = "/path"`，從 Finder／開始選單開啟時讀不到 shell 變數），模型改存在 `<LOCAL_LLM_HOME>/gguf/<owner>/<repo>/<檔名>`、llama.cpp 存在 `<LOCAL_LLM_HOME>/runtimes/llama.cpp`。這和 `hf download <repo> --local-dir "$LOCAL_LLM_HOME/gguf/<repo>"` 的結構相同，用其他方式下載、而且在清單裡的模型會直接當成已安裝。

可選的模型列在 [`models/catalog.json`](models/catalog.json)，目前預設 Gemma 4 12B（Q8_0；記憶體較少時用 Q4_K_XL）。之後有新模型（例如 Gemma 5），在清單加一筆即可出現在精靈中：

```json
{
  "id": "gemma-5-xx-q8",
  "name": "Gemma 5 XX",
  "variant": "Q8_0",
  "summary": "新一代",
  "repo": "unsloth/gemma-5-xx-it-GGUF",
  "file_suffix": "Q8_0.gguf",
  "mmproj_suffix": "mmproj-BF16.gguf",
  "approx_gb": 13,
  "min_memory_gb": 24,
  "context": 32768,
  "license": "…",
  "vision": true
}
```

檔名由 Hugging Face 的檔案清單比對 `file_suffix` 找出，不需要寫完整網址。不想改 repo 時，可把自己的 `catalog.json` 放在上述本機資料夾的 `models/` 底下，會取代內建清單。

### 自己架設的模型

也可以使用任何 OpenAI 相容的端點，例如 llama.cpp、Ollama、LM Studio，或家裡的另一台電腦：

```toml
# config.toml
[engine]
url = "http://127.0.0.1:8080/v1"
model = "gemma-4-12b"
```

或用環境變數 `PAPERCOMPOSER_LLM_URL`、`PAPERCOMPOSER_LLM_MODEL`。未連接時，問答框仍會列出參考句型，並可複製完整提示詞到其他模型使用。

**Claude**：在「設定」貼上 API key，key 只存在系統金鑰圈（Windows 認證管理員、macOS 鑰匙圈），不寫入任何檔案；也可以用環境變數 `ANTHROPIC_API_KEY`。資料庫的「拆解」功能優先使用 Claude，未設定時改用本機模型。

## 專案結構

```
crates/core         資料庫讀取與驗證、句型檢索、段落檢查、生成提示詞與輸出驗證（不需模型）
crates/server       本機伺服器（只監聽 127.0.0.1），桌面版與瀏覽器模式共用
apps/desktop        Tauri 桌面殼：啟動伺服器並開啟視窗
ui/                 介面（HTML / CSS / JS）
workspace-template/ 私人工作區範本與示範資料（沒有指定工作區時使用）
private-example/    完整的私人工作區範例（虛構的作者、文獻、期刊與論文）
scripts/            Git hooks
```

## API

| 方法 | 路徑 | 說明 |
| --- | --- | --- |
| GET | `/api/workspace` | 工作區位置、論文與出版社清單 |
| GET | `/api/profile` | 資料庫名稱、筆數、格式問題 |
| POST | `/api/profile/reload` | 重新讀取資料庫 |
| GET | `/api/phrases?section=&function=&position=&text=&limit=` | 依欄位篩選並排序句型 |
| POST | `/api/check` `{ text, section }` | 逐段檢查 |
| GET | `/api/engine` | 模型連線狀態 |
| POST | `/api/compose` `{ points, section, function, min_words, max_words, language, before, after, key_value? }` | 一次性段落生成 |
| POST | `/api/check/soft` `{ text, section, before, after, engine, key_value? }` | 軟規則檢查（有 Key value 時才問 R106） |
| POST | `/api/papers` `{ name, template?, key_value, title?, authors?, outline?, tables? }` | 建立論文（Key value 必填；`tables` 為 `{ caption, section, data, style? }`，圖片為 `{ kind: "figure", caption, section, file }`） |
| POST | `/api/papers/{name}/figures?file=` | 上傳圖片到論文的 `figures/`（body 為檔案內容） |
| POST | `/api/wizard/caption` `{ kind, data?, file?, section?, key_value?, title?, engine? }` | 表格或圖片的標題建議（寫作引擎） |
| GET／PUT | `/api/settings` | 工作區 `config.toml` 的 `[ui]` 介面設定 |
| GET | `/api/papers/{name}` | 論文各章節與段落、`meta`（Key value 與作者） |
| PUT | `/api/papers/{name}/meta` `{ key_value }` | 修改 Key value |
| GET / POST | `/api/people` | 作者與機構清單；新增一位作者（可同時新增機構） |
| POST | `/api/wizard/outline` `{ template?, outline, key_value, engine }` | 大綱 → 標題建議與章節架構（Claude） |
| POST | `/api/refs/extract` `{ text, engine }` | 把貼上的文字拆成一篇一篇（Claude） |
| POST | `/api/refs/verify` `{ title, authors, year, doi, pmid, arxiv }` | 向 Crossref／arXiv／PubMed 查證一篇是否存在 |
| POST | `/api/refs/propose-record` `{ work \| guess, engine }` | 沒有 DOI 紀錄可取時，從 PubMed 紀錄或你貼的書目產生文獻庫條目 |
| PUT | `/api/papers/{name}/sections/{file}` `{ title, paragraphs }` | 儲存章節（一句一行） |
| GET / PUT | `/api/tables/{file}` | 讀寫資料庫 CSV（句型、片語、用詞、規則、範例、公式、作者、機構） |
| POST | `/api/papers/{name}/export` `{ kind: "pdf" \| "docx" \| "latex" }` | 匯出：頁數、錯誤（含位置）、找不到的引用與標籤 |
| POST | `/api/papers/{name}/export/open` `{ kind, reveal }` | 開啟匯出的檔案，或在資料夾中顯示 |
| POST | `/api/papers/import?name=&key_value=` （本文為 .zip／.tar.gz／.tex） | 匯入現有的 LaTeX 專案 |
| POST | `/api/papers/import-arxiv` `{ id, name?, key_value }` | 從 arXiv 匯入原始檔 |
| PUT | `/api/papers/{name}/front` `{ field, index, value }` | 修改標題區的欄位（title、author、affil、abstract、keywords） |
| GET | `/api/papers/{name}/figure?path=` | 圖片預覽（PDF／EPS 轉 PNG） |
| POST | `/api/decompose` `{ text, section }` | 把句子或段落拆成句型與片語建議 |
| GET | `/api/claude` | Claude 連線狀態 |
| POST | `/api/settings/claude` `{ key, model }` | 存入或移除 API key、選擇模型 |
| POST | `/api/open/{page}` | 在系統瀏覽器開啟固定頁面：`anthropic-keys`（建立 API key）、`github-new-repo`（建立 private repo） |
| GET | `/api/git/status` | 分支、遠端、未同步的檔案、領先或落後的 commit 數 |
| POST | `/api/git/pull` | 從遠端更新（只做 fast-forward） |
| POST | `/api/git/sync` `{ message? }` | commit 並 push |
| GET | `/api/setup/status` | AI 與同步：工作區、Git、記憶體、模型、Claude 狀態 |
| POST | `/api/setup/workspace` `{ path, init }` | 使用既有資料夾（可補上範本） |
| POST | `/api/setup/clone` `{ url, parent }` | clone 自己的私有 repo |
| POST | `/api/setup/new` `{ path, remote? }` | 從範本建立新工作區 |
| GET | `/api/setup/visibility?url=` | 查詢 GitHub repo 是否公開 |
| POST | `/api/setup/git-identity` `{ name, email }` | 設定 Git 作者 |
| POST | `/api/models/plan` `{ id }` | 列出要下載的檔案、大小與來源 |
| POST | `/api/models/install` `{ id }` | 下載並啟用模型 |
| GET | `/api/models/progress` | 下載進度 |
| POST | `/api/models/start` / `/api/models/stop` | 啟動或停止本機模型 |

## 防止洩漏個人資料

```sh
sh scripts/install-hooks.sh
```

安裝 commit 前的金鑰掃描（需要 [gitleaks](https://github.com/gitleaks/gitleaks)）。

## 開發說明

本專案與 Claude（Anthropic）協作開發，程式碼在 claude.ai 與 Claude Code 中共同撰寫。
