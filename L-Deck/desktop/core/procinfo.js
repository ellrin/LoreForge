// 程序說明對照表：「這是什麼、為什麼佔用」。可以自己加。
// 依序比對，第一個符合的生效。match 比對的是執行檔名稱。

const RULES = [
  // ---- App 的子程序（Chrome / Electron / VS Code / Claude 都是這種架構）----
  { match: /Helper \(Renderer\)/, what: '網頁 / 視窗內容的渲染程序', why: '某個分頁或視窗正在跑動畫、影片或大量腳本' },
  { match: /Helper \(GPU\)/, what: 'GPU 繪圖加速程序', why: '畫面有影片、動畫或 3D 內容' },
  { match: /Helper \(Plugin\)/, what: '擴充套件 / 外掛程序', why: '擴充套件在背景工作（例如 VS Code 的語言伺服器、Git）' },
  { match: /Helper \(Alerts\)|Helper \(Aperitif\)/, what: '背景輔助程序', why: '處理通知、網路或其他背景工作' },
  { match: /Helper$/, what: 'App 的輔助程序', why: '處理網路、擴充或背景工作' },

  // ---- 系統 ----
  { match: /^WindowServer$/, what: 'macOS 繪製所有視窗與畫面', why: '外接螢幕、動畫、半透明效果會讓它變忙' },
  { match: /^kernel_task$/, what: '系統核心', why: '機器過熱時會故意佔用 CPU 來降溫' },
  { match: /^(mds|mds_stores|mdworker|mdworker_shared)$/, what: 'Spotlight 搜尋索引', why: '有大量新檔案或剛更新系統，正在建立索引' },
  { match: /^(photoanalysisd|mediaanalysisd)$/, what: '照片分析（人臉、物件辨識）', why: '照片圖庫有新照片，閒置時會在背景分析' },
  { match: /^(bird|cloudd|fileproviderd)$/, what: 'iCloud 同步', why: '正在上傳或下載 iCloud 雲碟的檔案' },
  { match: /^backupd/, what: 'Time Machine 備份', why: '正在備份' },
  { match: /^(softwareupdated|installd)$/, what: '系統更新 / 安裝', why: '正在下載或安裝更新' },
  { match: /^coreaudiod$/, what: '系統音訊', why: '正在播放或錄製聲音' },
  { match: /^(assistantd|siriknowledged|intelligenceplatformd|modelmanagerd)$/, what: 'Siri / Apple Intelligence', why: '正在處理語音或下載、整理裝置上的模型' },
  { match: /^swcd$/, what: '共享網路憑證服務', why: '剛安裝 App 或更新網域關聯資料' },
  { match: /^MTLCompilerService$/, what: 'GPU 著色器編譯', why: 'App 第一次開啟或更新後在編譯繪圖程式' },
  { match: /^(nsurlsessiond)$/, what: '背景下載', why: 'App 在背景下載檔案' },
  { match: /^(node|python\d*|ruby|java|R)$/, what: '程式語言執行環境', why: '某個腳本或開發伺服器正在執行' },
  { match: /^claude$/, what: 'Claude Code（終端機）', why: '正在執行任務' },
];

/** name: 執行檔名稱；app: 所屬 App 名稱（可能為 null） */
export function explain(name, app) {
  for (const r of RULES) if (r.match.test(name)) return { what: r.what, why: r.why };
  if (app && name.replace(/\.app$/, '') === app) return { what: `${app} 主程式`, why: '這個 App 本身正在工作' };
  if (app) return { what: `${app} 的子程序`, why: '' };
  return { what: '背景程序', why: '' };
}

/** 一行短說明（列表用） */
export function shortWhat(name, app) {
  const e = explain(name, app);
  return app && !e.what.includes(app) ? `${app} · ${e.what}` : e.what;
}
