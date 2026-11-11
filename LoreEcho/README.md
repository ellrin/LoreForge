# LoreEcho

個人學習夥伴：桌面右下角一顆圓形小泡泡，點開變成像視覺小說一樣的視窗：左邊站著人物的全身圖，右邊是對話。接 Claude 或本機模型（經過 `lore-llm`），回答偏短、以繁體中文為主；只記這一次的對話，不寫進任何檔案。設計見 `../docs/loreecho.md`。

> 開發中（階段 2：骨架）。已完成：泡泡／展開視窗（無邊框、透明、置頂、可拖曳、位置記在 private）、選單列圖示、全域快捷鍵、文字對話（串流）、範本人物、齒輪（一般、外觀、模型、快捷鍵）、讀 KnowledgeBase 的卡（只讀；每輪附上相關的卡與熟悉度，「從知識庫抽兩張」「今天要複習什麼」「考我」「有哪些…的卡」「記住這個」，回答下方「引用：…」可開卡；記得／忘了只在 KnowledgeBase 開著時經它的 API 記下）。
> 之後：特化按鈕（階段 4）、語音（階段 5，麥克風按鈕目前是停用的預留位置）。

## 執行

```sh
cargo run -p le-server        # 瀏覽器模式：http://127.0.0.1:7860
cargo run -p loreecho         # 桌面泡泡（Tauri）
```

資料在 `../private/LoreEcho/`，第一次啟動時從 `../private-example/LoreEcho/` 複製（之後不會再寫進範本）：

```
characters/<id>/character.toml   名稱、個性、聲音（之後）、語言、全身圖的大小（figure_scale、figure_offset_y）
characters/<id>/avatar/          頭像：idle listening thinking speaking（泡泡用；缺的用 idle）
characters/<id>/full/            全身圖：同樣四種狀態，從頭到腳、去背（展開視窗用；缺的用 full/idle，
                                 完全沒有 full/ 就用頭像）
settings.toml                    目前的人物、快捷鍵（預設 Alt+Space）
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
    整理成筆記、做成知識卡、記下靈感——下一階段才接上，現在按了只提示「下一階段」。
  - 右下：對話框，上緣是人物的名牌（點了換人物，和齒輪 →「一般」同一個設定），框裡是人物目前這句話（串流顯示），
    下面是輸入列。
  - 瀏覽器視窗很窄（寬 < 520 或高 < 380）時不顯示人物。
- 拖曳上方列、人物或對話紀錄的空白處移動視窗；左上角的箭頭或 Esc 收起（收起時對話會清掉）。
- 「＋ 新對話」清掉目前的對話。Enter 送出，Shift+Enter 換行。
- 邊緣：拖曳展開視窗的左邊、上邊或左上角調整大小（420–1000 px，記在 private，和泡泡位置一起）。
- Dock：像一般 App 有 Dock 圖示，點它會顯示泡泡或把視窗帶到最前面；⌘M 最小化、⌘H 隱藏、⌘Q 結束。
- 選單列圖示：左鍵顯示／隱藏，右鍵選單可以結束。快捷鍵在齒輪 →「一般」改。

## API

| 方法 | 路徑 | 說明 |
|---|---|---|
| GET | `/api/health` | 狀態 |
| GET/PUT | `/api/theme`，`/theme.css`，`/theme.js` | 共用主題 |
| GET/PUT | `/api/engines` | 共用的模型設定 |
| GET/PUT | `/api/window` | `{ x, y, expanded }` |
| GET/PUT | `/api/settings` | `{ character, shortcut }` |
| GET | `/api/characters` | 人物清單與目前的人物（每個人物：`avatars`、`full` 有圖的狀態，`full` 空＝沒有全身圖；`figure: { scale, offset_y }`） |
| GET | `/api/characters/{id}/avatar/{state}` | 頭像（缺的狀態用 idle） |
| GET | `/api/characters/{id}/full/{state}` | 全身圖（`full/<state>` → `full/idle` → 該狀態的頭像） |
| POST | `/api/chat` | `{ messages, character , kb? }` → 串流回答（server-sent events；有知識庫時先送 `search`、`cards`） |
| POST | `/api/kb/recall` | `{ id, outcome }` → 經 KnowledgeBase 的 API 記下複習結果（它沒開時回一句說明） |

## 圖示

原始檔為 `apps/desktop/icons/logo.svg`（作者核可的草圖）。`logo.png`（1024px）用 Chromium 轉出，其他尺寸、`icon.ico`、`icon.icns` 由 `logo.png` 縮出；有 tauri-cli 時也可以在 `apps/desktop/` 執行 `cargo tauri icon icons/logo.png -o icons` 重新產生（只保留 `tauri.conf.json` 列出的檔案）。

## 開發說明

本專案與 Claude（Anthropic）協作開發，程式碼在 claude.ai 與 Claude Code 中共同撰寫。
