# LoreEcho（私人 repo 的起始結構）

```
characters/<id>/character.toml   人物：名稱、個性（系統提示）、聲音（語音階段才用）、語言
characters/<id>/avatar/          人物圖：idle listening thinking speaking（.png .svg .webp .jpg）
settings.toml                    設定：目前的人物、叫出／收起的快捷鍵（預設 Alt+Space）
state.toml                       泡泡的位置、面板是否展開（App 自己寫）
tasks.toml                       （選用）改 le-chat 用哪個模型，格式同 lore-llm 的 tasks.toml 的 [override]
```

- 對話**不存檔**：只留在畫面上，按「新對話」或關掉 App 就沒了。
- 人物圖缺哪一張就用 `idle`；連 `idle` 都沒有就顯示 App 圖示。人物圖不套主題色。
- 範本人物 `echo` 的圖是 App 圖示的簡化輪廓。可以有多個人物，在齒輪的「一般」裡切換。
- 金鑰不放這裡（在系統鑰匙圈）；本機模型在 `LOCAL_LLM_HOME`。
