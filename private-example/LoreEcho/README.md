# LoreEcho（私人 repo 的起始結構）

```
characters/<id>/character.toml   人物：名稱、個性（系統提示）、聲音（語音階段才用）、語言、
                                 全身圖的大小 figure_scale（預設 1.0）、figure_offset_y（預設 0）
characters/<id>/avatar/          頭像（泡泡）：idle listening thinking speaking（.png .svg .webp .jpg）
characters/<id>/full/            全身圖（展開視窗）：同樣四種狀態，從頭到腳、背景透明
settings.toml                    設定：目前的人物、叫出／收起的快捷鍵（預設 Alt+Space）
state.toml                       泡泡的位置、面板是否展開（App 自己寫）
tasks.toml                       （選用）改 le-chat 用哪個模型，格式同 lore-llm 的 tasks.toml 的 [override]
```

- 對話**不存檔**：只留在畫面上，按「新對話」或關掉 App 就沒了。
- 人物圖缺哪一張就用 `idle`；沒有 `full/` 時展開視窗用頭像；連頭像都沒有就顯示 App 圖示。人物圖不套主題色。
- `figure_scale`：1.0＝從頭到腳剛好放進視窗高度；大於 1 就放大，腳下超出視窗底邊被裁掉（像視覺小說的立繪），範圍 0.5–3。
  `figure_offset_y`：往下移視窗高度的幾成（負數往上），範圍 -0.5–0.5。
- 範本人物 `echo` 的圖是 App 圖示的簡化輪廓（頭像）和同一個人的全身（`figure_scale = 1.35`）。可以有多個人物，在齒輪的「一般」裡切換。
- 金鑰不放這裡（在系統鑰匙圈）；本機模型在 `LOCAL_LLM_HOME`。
