# 學習夥伴（暫名）設計草稿 — 2026-11-11

> 狀態：**草稿，待作者確認**。名稱、§9 的待決事項定了之後才開始寫程式。

一個個人學習助理：桌面右下角的圓形小圖示，點開後展開成一個有人物、有語音的聊天面板。
它接 Claude 或本機模型，**知道你知道什麼**（讀 KnowledgeBase），話不多、以中文為主、偏學習討論；
討論中的想法可以用「特化按鈕」整理後存成 PaperComposer 的筆記或 KnowledgeBase 的知識卡。

icon 草圖：`docs/assets/companion-logo-draft.svg`（賽博龐克機器人頭的輪廓：後梳的頭髮、圓形護目鏡、
額頭裂痕、下顎護板、小鬍子、高領；風格同其他 app 的 logo：深紫底、霓虹線條、發光、紅藍錯位）。

## 1. 外觀與視窗

| 狀態 | 樣子 |
|---|---|
| 收合 | 圓形泡泡（約 64px），預設在右下角，**可拖曳**，位置記在 private；常駐時外圈有麥克風收音的光環 |
| 展開 | 從泡泡長出的面板（約 380×560）：上半是人物（avatar），下半是對話；底部是輸入列、麥克風、特化按鈕 |
| 人物狀態 | `idle`／`listening`／`thinking`／`speaking` 四張圖（或一張圖＋動畫），說話時嘴型可先用簡單的開合切換 |

- Tauri 無邊框、透明、置頂的小視窗（不是一般 app 視窗）；系統匣／選單列有圖示可以叫出來。
- 介面配色走共用主題（`commons/ui/theme.css`），按鈕與提示用 `commons/ui` 的共用元件。人物圖本身不套主題色。
- 快捷鍵叫出／收起（例如 ⌥Space，可改）。

## 2. 對話

- **只記當次對話**：對話存在記憶體，關掉面板（或按「新對話」）就清掉，不寫長期記憶。
  - 唯一會落地的是你按了特化按鈕存下來的東西。
- 回答**偏短**：人物設定的系統提示要求 1–3 句、口語、繁體中文（`lore-llm` 的 `zhtw` 會再轉一次）；你說「詳細一點」才展開。
- 走 `lore-llm`：新任務類別放在「問答」（`ask`，自動＝有本機用本機），任務例如 `cp-chat`、`cp-note`、`cp-cards`。
  - 注意：Claude 走訂閱（`claude -p`）不能串流，一次回整段，語音會比較慢開口；走 API 或本機可以邊生邊念。

## 3. 「知道你知道什麼」— 接 KnowledgeBase

每一輪對話前，從你的話裡找出相關的知識卡，連同**你對它的熟悉度**一起放進提示：

1. `kb-core` 的 `scan`（術語掃描：標題＋別名）＋ FTS 搜尋，找出相關卡片。
2. 讀 `progress/<目前成員>/` 的狀態：`known`（你懂）、`learning`（在學）、`stale`（很久沒碰）、沒有卡（你不知道）。
3. 提示裡寫成：「使用者已懂：A、B（不必解釋）；正在學：C（可以考考他）；沒有卡：D（要從頭講）」，附卡片摘要（有字數上限）。

只讀不寫：讀的時候直接用 `kb-core` 開 `private/KnowledgeBase`（檔案是真相，索引可重建）。
畫面上會顯示這輪參考了哪幾張卡，點了用 `lore-kb://` 在 KnowledgeBase 打開。

## 4. 特化按鈕

討論到一半、有想法時按，**先給你預覽、你確認了才存**，絕不自動寫入。

| 按鈕 | 做什麼 | 存到哪 |
|---|---|---|
| 整理成筆記 | 把這段討論整理成完整筆記（問題、討論過程、結論、待查） | **PaperComposer 新增的「筆記」功能**（§5） |
| 做成知識卡 | 抽出 1–N 張卡的候選（標題、內容、標籤、連結到既有卡），逐張勾選 | KnowledgeBase（新卡預設 `learning`） |
| 記下靈感 | 一句話的快速記錄，不整理 | 筆記的「靈感」收件匣 |

寫入的路徑：對方 app 開著時走它的 API（`ports.json` 找埠，例如 KB 的 `POST /api/nodes`），
沒開時直接用 `kb-core`（KB）或寫進 PaperComposer 的筆記資料夾，下次開 app 時索引會補上。

## 5. PaperComposer 新增「筆記」

- 位置：`private/PaperComposer/notes/<日期>-<標題>.md`（或 `.tex`，見 §9），前面一段 front matter：
  來源（學習夥伴／手寫）、建立時間、相關的 KB 卡 id、標籤。
- PaperComposer 上方列加「筆記」：清單、閱讀、編輯；可以「插入到論文」（之後再做）。
- 牽涉 LaTeX 的部分照 `lore-notation`：方程式用登記過的巨集，產生的文字跑它的檢查。

## 6. 人物（全部在 private）

```
private/<App>/characters/<id>/
  character.toml   名稱、個性提示（話少、語氣）、預設聲音、語言
  avatar/          idle.png listening.png thinking.png speaking.png（或 sprite）
  voice/           聲音設定；自訓聲音的參考音檔或模型路徑
```

- 可以有多個人物，在齒輪裡切換；`private-example/<App>/` 只放一個中性的範本人物（用 icon 的輪廓當 avatar）。
- 上傳幾張圖 → 裁切成四種狀態（第一版手動指定，之後可以讓模型幫忙去背／挑表情）。

## 7. 語音

新的共用 crate `commons/crates/lore-voice`（`commons/CLAUDE.md` 早就留了名字，但還不存在）：

| 部分 | 第一版 | 之後 |
|---|---|---|
| 聽（STT） | whisper.cpp 本機（中文夠好）；或 macOS 內建語音辨識 | — |
| 說（TTS） | macOS 內建聲音（`AVSpeechSynthesizer`），零設定 | 本機可複製聲音的 TTS（例如 GPT-SoVITS／F5-TTS／CosyVoice 這類，用幾十秒參考音就能做中文聲線）；每個人物一個聲音 |
| 常駐 | 按鈕切換：麥克風一直開，VAD（偵測有人說話）切句；收合時只亮光環 | 喚醒詞（可選） |

- 預設是按住說話；常駐要你自己打開，關掉面板不會自動常駐。
- 測試只用假的 STT／TTS 引擎；真的引擎標 **not tested on real hardware**。
- 語音模型和本機 LLM 一樣放在 `LOCAL_LLM_HOME`，也受 §8 的記憶體管理。

## 8. 本機模型共用與優先權（對所有 app 都有用）

現況：`LocalRuntime` 在**一個程序內**保證只開一個 llama-server，由 PaperComposer 開；其他 app 讀
`private/shared/engine.toml` 的 url 連過去。沒有人管「誰先」，PaperComposer 關了模型也跟著停。

提案：**一個共用的模型管家**（小背景程序，暫名 `lore-modeld`，放 commons）

- 整台電腦只有它能開 llama-server（以及之後的 STT／TTS 模型）；任何 app 需要時把它叫起來（鎖檔在 `ports.json` 同一個快取資料夾），沒人用時依 `idle_minutes` 釋放。
- app 不再直接連 llama-server，而是透過管家送請求（同樣是 OpenAI 相容介面，`lore-llm` 只換 url，其他不變）。
- **優先權佇列**：
  1. 你**正在操作的 app**（視窗在前景；Tauri 的 focus 事件回報給管家）
  2. 互動式請求（對話、語音）高於背景批次（整理、翻譯、批次審稿）
  3. 同等級先來先服務
- 一次只跑一兩個請求（llama-server 的 slot 數）；背景工作在**兩個請求之間**讓位，不中途砍掉（要砍也只砍串流中的背景請求，之後重送）。
- 換模型的規則：目前載入的模型夠用就不換；前景有請求在等時不為背景工作換模型；需要看圖而目前模型不能看時才換。
- 記憶體不夠同時放 LLM＋TTS 時：LLM 優先，TTS 退回系統聲音（第一版就是這樣，不用額外管）。

這一塊獨立於學習夥伴，可以先做，PaperComposer／SlideComposer／LoreReader 都會受益。

## 9. 待作者決定

1. **名稱**（同時決定資料夾、`lore-<短名>://`、任務前綴）。
2. **筆記格式**：Markdown（好讀、好整理）還是 LaTeX（直接可貼進論文，要過 `lore-notation`）？
3. **新知識卡**：直接進 KB（`learning`），還是先進「待審」區，回 KB 再確認？
4. ~~模型管家~~ → 先採**獨立的小背景程序** `lore-modeld`（2026-11-11 開工，作者說「開始做」時採用建議值；要改再說）。
5. **平台**：只做 macOS（透明置頂視窗、系統聲音最順），還是也要 Windows？

## 10. 建議的施工順序

| 階段 | 內容 |
|---|---|
| 0 | 本稿＋icon 定案（icon 已減線條，表情照作者的參考圖：八字眉、半垂眼皮、下垂嘴角） |
| 1 | **進行中**：模型管家（`lore-modeld`）＋ `lore-llm` 改走它；PaperComposer 把開模型的工作交出去 |
| 2 | 新 app 骨架：泡泡／展開視窗、拖曳、文字對話、範本人物、齒輪（共用分頁） |
| 3 | 接 KB：相關卡＋熟悉度放進提示，顯示參考了哪些卡 |
| 4 | 特化按鈕：知識卡 → KB；PaperComposer 筆記功能 → 整理成筆記、記下靈感 |
| 5 | 語音：`lore-voice`（STT＋系統 TTS）、按住說話、常駐模式 |
| 6 | 自訂聲音、多人物、avatar 動畫 |
