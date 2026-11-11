# LoreEcho

個人學習夥伴：桌面右下角一顆圓形小泡泡，點開變成像視覺小說一樣的視窗：左邊站著人物的全身圖，右邊是對話。接 Claude 或本機模型（經過 `lore-llm`），回答偏短、以繁體中文為主；只記這一次的對話，不寫進任何檔案。設計見 `../docs/loreecho.md`。

> 開發中（階段 2：骨架＋語音）。已完成：泡泡／展開視窗（無邊框、透明、置頂、可拖曳、位置記在 private）、選單列圖示、全域快捷鍵、文字對話（串流）、範本人物、齒輪（一般、外觀、模型、快捷鍵、人物）、語音（macOS：按住說話／常駐、朗讀回覆）、人物的主題曲與角色設定、讀 KnowledgeBase 的卡（只讀；每輪附上相關的卡與熟悉度，「從知識庫抽兩張」「今天要複習什麼」「考我」「有哪些…的卡」「記住這個」，回答下方「引用：…」可開卡；記得／忘了只在 KnowledgeBase 開著時經它的 API 記下）、功能列表（整理成筆記、做成知識卡、記下靈感，先預覽、確認才存）。

## 執行

```sh
cargo run -p le-server        # 瀏覽器模式：http://127.0.0.1:7860
cargo run -p loreecho         # 桌面泡泡（Tauri）
```

資料在 `../private/LoreEcho/`，第一次啟動時從 `../private-example/LoreEcho/` 複製（之後不會再寫進範本）：

```
characters/<id>/character.toml   名稱、個性、語言、聲音（voice＝系統聲音 id，voice_rate＝語速倍數）、
                                 角色設定（background speech_style catchphrases call_user relationship
                                 likes dislikes [[examples]]）、主題曲（theme_loop、theme_volume）、
                                 全身圖的大小（figure_scale、figure_offset_y）
characters/<id>/theme.<ext>      主題曲（mp3 m4a aac wav ogg opus flac，30 MB 以內，可以沒有）
characters/<id>/avatar/          頭像：idle listening thinking speaking（泡泡用；缺的用 idle）
characters/<id>/full/            全身圖：同樣四種狀態，從頭到腳、去背（展開視窗用；缺的用 full/idle，
                                 完全沒有 full/ 就用頭像）
settings.toml                    目前的人物、快捷鍵（預設 Alt+Space）、麥克風（talk = hold｜continuous）、
                                 說完就送出（auto_send）、朗讀回覆（read_aloud）
state.toml                       泡泡的位置、是否展開
```

模型：齒輪 →「模型」，和其他 LoreForge App 共用（`private/shared/engine.toml`）。對話是任務 `le-chat`，屬於「問答」：有本機模型就用本機，否則用 Claude。Claude 走訂閱時不能串流，回答會一次出現。也可以用 `LOREECHO_LLM_URL` 指定一個 OpenAI 相容的本機伺服器。

## 畫面

- 泡泡（64px）：人物的 idle 頭像裁成圓形。點一下打開；按住拖曳可以移動。
- 展開視窗（620×620，透明、只畫人物和對話框；右下角和泡泡同一個角，放不下時推回螢幕內）：
  - 左：人物全身圖（等待回答時 thinking、串流時 speaking、其他時候 idle），頭在視窗上緣附近，
    稍微壓在右邊對話欄的左緣上（那一條不放文字）。`character.toml` 的 `figure_scale`
    （預設 1.0＝從頭到腳剛好放進視窗高度；範本 回聲 是 1.35，腳下超出視窗底邊被裁掉，像立繪）、
    `figure_offset_y`（往下移視窗高度的幾成，-0.5–0.5）。只有頭像的人物顯示頭像。
  - 右上：上方列（收起 · 回答的模型 · 功能列表 · 新對話 · 齒輪）＋這次對話的較早幾輪（小字，你的話靠右）。
  - 功能列表：按上方列的清單鈕（再按或 Esc 收回）展開成選項列，左邊是說明、右邊是選項；↑↓ 移動、Enter 選。
    - 整理成筆記：模型（任務 `le-note`，整理與格式）把這段對話整理成 LaTeX 筆記（問題、討論過程、結論、待查），
      在歷史區的位置預覽（預覽／LaTeX 可切換、可改標題、標籤、相關卡），下方列出 lore-notation 的檢查結果，
      有阻擋性問題（例如 `\ref` 到不存在的標籤）時不能存。存到 PaperComposer：它開著時經它的 `POST /api/notes`，
      沒開時直接寫 `private/PaperComposer/notes/<日期>-<標題>.tex`（不覆蓋，撞名加 `-2`）。
    - 做成知識卡：模型（任務 `le-cards`）挑出 1–N 張候選卡，可連到你已有的卡；逐張勾選、修改，「送到待審」＝
      KnowledgeBase 開著時經它的 `POST /api/pending`，沒開時用 kb-core 寫進你個人空間的 `pending/`。不會變成正式卡，要回 KnowledgeBase 確認。
    - 記下靈感：一句話，不用模型，存成 `kind: idea` 的筆記。
    - 整理時人物顯示「noting」；存好後跳一行提示，附「打開」。
  - 右下：對話框，上緣是人物的名牌（點了換人物，和齒輪 →「一般」同一個設定），框裡是人物目前這句話（串流顯示），
    下面是輸入列。
  - 瀏覽器視窗很窄（寬 < 520 或高 < 380）時不顯示人物。
- 拖曳上方列、人物或對話紀錄的空白處移動視窗；左上角的箭頭或 Esc 收起（收起時對話會清掉）。
- 「＋ 新對話」清掉目前的對話。Enter 送出，Shift+Enter 換行。
- 邊緣：拖曳展開視窗的左邊、上邊或左上角調整大小（420–1000 px，記在 private，和泡泡位置一起）。
- Dock：像一般 App 有 Dock 圖示，點它會顯示泡泡或把視窗帶到最前面；⌘M 最小化、⌘H 隱藏、⌘Q 結束。
- 選單列圖示：左鍵顯示／隱藏，右鍵選單可以結束。快捷鍵在齒輪 →「一般」改。

## 語音（macOS）

- 輸入列左邊：麥克風、朗讀回覆（喇叭，開／關）、主題曲（人物有主題曲才出現；播放／暫停）。
- 麥克風預設「按住說話」：按住時人物是 listening，說的話即時出現在輸入框（接在原本打的字後面），放開就送出。
  齒輪 →「一般」→ 語音 可以改成「常駐」（按一下開、再按一下關，說完一句停一下就送出；回答和朗讀時先停聽，結束後再接著聽），
  也可以關掉「說完就送出」（改成留在輸入框等你按 ↵）。
- 朗讀回覆：回答一邊串流一邊一句一句念出來（第一句念到第一個逗號就先開始），人物在念完之前都是 speaking；Esc 或點人物停止。
  「文字跟著聲音」（齒輪 → 一般 → 語音，預設開）：對話框的字跟著聲音一句一句出現，像視覺小說；完整的回覆照樣留在對話紀錄。
- 聽：Apple `SpeechTranscriber`（裝置上辨識，這台 macOS 支援中文（台灣））；不支援或模型還沒下載時用 `SFSpeechRecognizer`。
  說：系統聲音（齒輪 → 人物 → 聲音，中文（台灣）排前面，標出加強版／高品質，機械音放最後的「其他（機械音）」；
  系統設定 → 輔助使用 → 朗讀內容 → 管理聲音 可以免費下載更好的聲音）。語速、音調（0.5–2 倍）每個人物各自存。
  個人聲音（macOS 14+）：按「使用個人聲音」才會問權限；允許後自己的聲音排在最上面；沒有做過的話到 系統設定 → 輔助使用 → 個人聲音。
- 第一次按麥克風時 macOS 會問麥克風（和必要時語音辨識）權限；拒絕過的話會出現一行提示和「開啟系統設定」。
- 主題曲：齒輪 → 人物 → 主題曲（選擇或拖進音樂檔、試聽、移除、循環、音量）。只在按播放時才播；說話、聆聽時自動調小聲，收起或換人物時淡出。
  WebKit 播不了 ogg／opus 時會提示。
- 角色設定：齒輪 → 人物 → 角色設定（身分與背景、說話風格、口頭禪、怎麼稱呼你、和你的關係、喜歡／不喜歡、範例對話）。
  全部選填；系統提示依序是個性、名字、這些設定、固定的簡短規則；範例對話當成前幾輪對話送出。
  身分與背景只送前 800 字（欄位下面有字數）。
- 實作：`../commons/crates/lore-voice`（Swift 輔助程式 `lore-voice-helper`，打包在 App 的 `Contents/MacOS/`）。

## API

| 方法 | 路徑 | 說明 |
|---|---|---|
| GET | `/api/health` | 狀態 |
| GET/PUT | `/api/theme`，`/theme.css`，`/theme.js` | 共用主題 |
| GET/PUT | `/api/engines` | 共用的模型設定 |
| GET/PUT | `/api/window` | `{ x, y, expanded }` |
| GET/PUT | `/api/settings` | `{ character, shortcut, talk, auto_send, read_aloud, text_follows }` |
| GET | `/api/characters` | 人物清單與目前的人物（每個人物：`avatars`、`full` 有圖的狀態，`full` 空＝沒有全身圖；`figure: { scale, offset_y }`） |
| GET | `/api/characters/{id}/avatar/{state}` | 頭像（缺的狀態用 idle） |
| GET | `/api/characters/{id}/full/{state}` | 全身圖（`full/<state>` → `full/idle` → 該狀態的頭像） |
| GET/PUT | `/api/characters/{id}` | 名稱、個性、聲音、語速、主題曲設定、角色設定（`profile`） |
| GET/PUT/DELETE | `/api/characters/{id}/theme` | 主題曲 |
| POST | `/api/chat` | `{ messages, character, kb?, speak? }` → 串流回答（server-sent events；有知識庫時先送 `search`、`cards`；`speak` = 一邊念出來） |
| POST | `/api/kb/recall` | `{ id, outcome }` → 經 KnowledgeBase 的 API 記下複習結果（它沒開時回一句說明） |
| GET | `/api/voice`，`/api/voice/voices`，`/api/voice/events` | 語音狀態、系統聲音、語音事件（server-sent events） |
| POST | `/api/voice/listen`，`/listen/stop`，`/speak`，`/stop`，`/open-settings`，`/warm` | 聽、停、說（試聽，可帶 `pitch`）、停止說、開系統設定（隱私、個人聲音、朗讀內容）、打開視窗時先載入聲音 |
| GET/POST | `/api/voice/personal` | 個人聲音的權限 `{ status, voices }`；POST = 使用個人聲音（從沒問過才會跳系統提示） |

## 圖示

原始檔為 `apps/desktop/icons/logo.svg`（作者核可的草圖）。`logo.png`（1024px）用 Chromium 轉出，其他尺寸、`icon.ico`、`icon.icns` 由 `logo.png` 縮出；有 tauri-cli 時也可以在 `apps/desktop/` 執行 `cargo tauri icon icons/logo.png -o icons` 重新產生（只保留 `tauri.conf.json` 列出的檔案）。

## 開發說明

本專案與 Claude（Anthropic）協作開發，程式碼在 claude.ai 與 Claude Code 中共同撰寫。
