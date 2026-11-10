# NeonDeck — 自己設計的 macOS 桌面

整個桌面是一張畫布：小工具（JS）＋主題（CSS）都是你自己寫的檔案，存檔就即時生效。

```
Sources/          Swift 外殼：桌面視窗、系統數據、日曆/提醒、開 App、通知
desktop/
  index.html
  layout.json     版面（哪些小工具、位置、大小、各自設定、目前主題）；個人設定，不進 git
  core/           deck.js 版面引擎 · util.js 共用函式 · base.css 結構 · mock.js 預覽用假資料
  themes/         ★ 美術風格：cyberpunk.css、mono.css …
  widgets/        ★ 小工具：sysmon / clock / alarm / calendar（含提醒事項待辦）/ cards（知識卡輪播）/ news / launcher …；_template.js 是範本
```

## 安裝 / 更新
```bash
./build.sh install     # 編譯 → ~/Applications/NeonDeck.app → 啟動；桌面資料夾指向本專案 desktop/（在 LoreForge/NeonDeck 執行）
```
選單列的 ▦ 圖示：編輯版面（⌘E）、新增小工具、切換主題、重新載入、登入時自動啟動、結束。

## 日常使用
- **編輯版面**：拖曳移動、右下角縮放、✕ 移除、Esc 結束。版面自動存回 `layout.json`。
- **多螢幕 / 不同解析度**：版面依解析度各存一份。沒排過的螢幕會借用最接近的版面等比例縮放顯示；在那台螢幕上調整後就另存一份，不影響其他螢幕。
- **改風格**：編輯 `themes/*.css` 存檔 → 不用重新載入就會套用。先改 `:root` 的色票，大部分畫面就會跟著變。
- **寫新主題**：複製一份 css 改名，選單 ▸ 主題 就看得到。
- **寫新小工具**：複製 `widgets/_template.js` 改名（不要 `_` 開頭）→ 選單 ▸ 新增小工具。
- **小工具個別設定**：直接改 `layout.json` 裡的 `config`（例如 `"hideHeader": true`、`"title": "..."`、`"className": "my-style"` 讓主題針對它）。手動改 layout.json 會自動重新載入。

## 除錯
- **用 Chrome 設計主題**：`cd desktop && python3 -m http.server 8765` → 開 http://localhost:8765（可加 `?theme=mono`），用 DevTools 調整，資料是假的。
- **檢查 App 裡的實際頁面**：Safari ▸ 設定 ▸ 進階 ▸ 顯示「開發」選單 → 開發 ▸ 你的 Mac ▸ NeonDeck。
- **錯誤記錄**：`tail -f ~/Library/Logs/NeonDeck.log`（JS 錯誤也會寫進來）。

## 資源消耗設計
- 系統數據全部用 syscall 直接讀，不開子程序；儀表板被全部遮住或隱藏時暫停取樣。
- 只有掛著的小工具宣告需要的頻道才會計算（例如沒有 sysmon 就完全不取樣）。
- 網速 / 記憶體 / CPU 每秒、記憶體大戶 5 秒、電池 / IP 30 秒、硬碟容量開機算一次（選單可手動重算）。
- 主題避免無限循環的 CSS 動畫（那會持續吃 GPU）。

## 已知限制
- 目前只鋪在主螢幕。
- 鬧鐘需要 NeonDeck 在執行中、Mac 沒有睡眠。
- App 是本機簽章（ad-hoc），每次重新編譯後 macOS 可能會再問一次行事曆 / 提醒事項權限。
