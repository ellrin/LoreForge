# 骨架 — 前端進度

Design: `docs/long-paper.md` §G. Contract: `L-Paper/docs/skeleton-api.md`.

## 做完的

- `ui/skeleton.js`, `ui/skeleton.css`: 素材、骨架（框、段落、拖曳、Alt+↑↓、＋、復原）、邊欄（提問、補充、修補、細修）、風格對照。
- `ui/skeleton-demo.html`: the screen on the synthetic fixture with a pretend server in the page.
  `python3 -m http.server 8765` from the repo root → `/L-Paper/ui/skeleton-demo.html`
  (`?at=b4`, `?tab=inventory|style`, `?empty=1`, `?do=answer,write,fold,note,narrow`, `?theme=<id>`).
- 入口：討論草稿視窗上方的「骨架」。
- 畫面只換有變動的節點（`morph`）：背景更新時正在打字的框不會被換掉。
  檢查：`skeleton-demo.html?at=b4&probe=typing`，標題顯示 `probe typing: ok`。
- 共用：麥克風與繪圖工具用討論草稿的（`PCDraft.micButton`, `PCDraft.makeFloat`）；toast 與確認用 `LoreShell`。

## 還沒在真的環境試

- 拖曳的手感、長骨架的捲動、WKWebView（桌面 App）裡的表現。
- 中文輸入法組字時背景更新（只用程式檢查過節點沒有被換掉）。
- 和真的伺服器接起來（等後端）。

## 沒做／留給作者

- 段落文字的顯示是這裡自己的簡單轉換（`\ref`、`\cite`、`$…$`、`\pending`）；L-Paper 沒有現成的行內 LaTeX 顯示可以共用。
- 素材清單的公式、偽代碼、表格還沒有接到各自的工具（流程圖與繪圖有）。
