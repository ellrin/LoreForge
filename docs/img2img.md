# 以圖產圖（image → images）與畫質提升

共用工具，放在 commons，任何 Lore app 都能用。第一批使用者：LoreEcho（新增角色）、SlideComposer（產生多張風格一致、關鍵語義不同的圖）。

## 用法

輸入一張參考圖（通常是一個角色）＋一組「變化」（表情、姿態、或任意文字描述），輸出每個變化各數張候選圖；作者挑一張，之後可選去背、畫質提升。

- **LoreEcho 新增角色**：一張主角圖 → 10 種狀態的大頭（表情）與全身（姿態）→ 每個狀態 4 張候選、挑 1 → 去背 → 放大 → 填進角色的 `avatar/`、`full/`。
- **SlideComposer**：選一張圖（或投影片上的圖）→ 「以圖產圖」→ 輸入幾個變化（例如同一個角色在不同場景、同一種圖示換不同主題）→ 候選圖 → 插入投影片。

## 模型（作者電腦 M 系列、32 GB；研究見 scratchpad 報告，授權以實際 LICENSE 為準）

| 工作 | 模型 | 備註 |
|---|---|---|
| 以圖產圖（保持同一角色的編輯） | Qwen-Image-Edit（Apache-2.0），MLX 4-bit（mflux），Lightning 4-step | 32 GB 只能 4-bit；執行時本機語言模型要讓位（`docs/local-models.md` 的一次一個模型） |
| 去背 | Apple Vision（內建）；邊緣不好時 BiRefNet | |
| 畫質提升 | Real-ESRGAN ncnn-vulkan（BSD-3），`realesrgan-x4plus-anime` | universal binary（arm64 原生） |

模型檔都是**第一次使用時由作者按下載**，放在共用模型資料夾（`[local] home`），不進 repo。

## 執行

- 跟本機語言模型同一套規則：同時只有一個大模型佔記憶體；生成前先讓語言模型讓位，生成完釋放。
- 生成是背景工作（照 `docs/paper-deck.md` 的工作模式：分步存檔、進度、停止、續做），每張圖顯示進度與預估時間。
- 永遠有「人挑選」這一步；自動流程只產生候選。

## 分階段

1. 畫質提升套件（Real-ESRGAN）：共用 crate ＋ 打包。
2. 以圖產圖核心：模型下載、生成、候選、背景工作、與一次一個模型的規則整合。
3. LoreEcho「從一張圖建立角色」。
4. SlideComposer「以圖產圖」插入。
