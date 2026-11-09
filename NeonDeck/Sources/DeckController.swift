import AppKit
import WebKit
import UniformTypeIdentifiers
import EventKit

// MARK: - 使用者資料夾（主題 / 小工具 / 版面）

enum DeckPaths {
    static let defaultsKey = "deckRoot"

    /// 優先使用 `defaults write com.local.neondeck deckRoot <path>` 指定的資料夾（開發時指向專案的 desktop/），
    /// 否則用 ~/Library/Application Support/NeonDeck（第一次啟動時從 App 內建範本複製）。
    static func resolveRoot() -> URL {
        let fm = FileManager.default
        if let p = UserDefaults.standard.string(forKey: defaultsKey),
           fm.fileExists(atPath: (p as NSString).appendingPathComponent("index.html")) {
            return URL(fileURLWithPath: p, isDirectory: true)
        }
        let support = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("NeonDeck", isDirectory: true)
        if !fm.fileExists(atPath: support.appendingPathComponent("index.html").path),
           let bundled = Bundle.main.url(forResource: "desktop", withExtension: nil) {
            try? fm.createDirectory(at: support.deletingLastPathComponent(), withIntermediateDirectories: true)
            try? fm.removeItem(at: support)
            try? fm.copyItem(at: bundled, to: support)
        }
        return support
    }
}

// MARK: - deck:// 協定：把資料夾當成同源網站提供（ES module / 動態 import 才能正常運作）

final class DeckSchemeHandler: NSObject, WKURLSchemeHandler {
    let root: URL
    init(root: URL) { self.root = root.standardizedFileURL }

    func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        guard let url = task.request.url else { return }
        var path = url.path
        if path.isEmpty || path == "/" { path = "/index.html" }
        let file = root.appendingPathComponent(String(path.dropFirst())).standardizedFileURL
        guard file.path.hasPrefix(root.path), let data = try? Data(contentsOf: file) else {
            task.didFailWithError(URLError(.fileDoesNotExist)); return
        }
        let headers = ["Content-Type": Self.mime(file.pathExtension), "Cache-Control": "no-store"]
        let resp = HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: headers)!
        task.didReceive(resp)
        task.didReceive(data)
        task.didFinish()
    }

    func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}

    static func mime(_ ext: String) -> String {
        switch ext.lowercased() {
        case "html": return "text/html; charset=utf-8"
        case "js", "mjs": return "text/javascript; charset=utf-8"
        case "css": return "text/css; charset=utf-8"
        case "json": return "application/json; charset=utf-8"
        case "svg": return "image/svg+xml"
        default: return UTType(filenameExtension: ext)?.preferredMIMEType ?? "application/octet-stream"
        }
    }
}

// MARK: - 桌面視窗

final class DesktopWindow: NSWindow {
    override var canBecomeKey: Bool { true }   // 讓輸入框（新增提醒、鬧鐘）可以打字
    override var canBecomeMain: Bool { false }
}

// MARK: - 控制器：視窗 + WebView + JS 橋接 + 推送排程

@MainActor
final class DeckController: NSObject, WKScriptMessageHandlerWithReply, WKNavigationDelegate {
    let root: URL
    private(set) var window: DesktopWindow!
    private(set) var webView: WKWebView!
    let stats = StatsService()
    let eventKit = EventKitService()
    private var watcher: FileWatcher?
    private var timer: Timer?
    private var tickCount = 0
    private var wasActive = false
    private var ready = false
    private var subscriptions = Set<String>()
    private var lastWrittenLayout: Data?

    private(set) var editMode = false
    private(set) var currentTheme = "cyberpunk"
    var onStateChange: (() -> Void)?

    var layoutURL: URL { root.appendingPathComponent("layout.json") }

    override init() {
        root = DeckPaths.resolveRoot()
        super.init()
    }

    func start() {
        buildWindow()
        eventKit.onChange = { [weak self] in
            self?.send("calendarChanged", [:])
            self?.send("remindersChanged", [:])
        }
        watcher = FileWatcher(path: root.path) { [weak self] paths in
            MainActor.assumeIsolated { self?.filesChanged(paths) }
        }
        let t = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
        t.tolerance = 0.2
        RunLoop.main.add(t, forMode: .common)
        timer = t

        NotificationCenter.default.addObserver(forName: NSApplication.didChangeScreenParametersNotification,
                                               object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.fitToScreen() }
        }
        startPassthrough()
    }

    // MARK: 空白處穿透
    // 只有滑鼠在小工具上時視窗才接收事件；其他地方的點擊 / 右鍵直接交給 Finder 桌面。
    // 滑鼠在空白處：靠全域 mouseMoved 監聽（事件驅動，不輪詢）。
    // 滑鼠在小工具上：用 10Hz 輪詢偵測何時離開（只有懸停在小工具上時才會跑）。

    private var hitRects: [CGRect] = []
    private var captureAll = false          // 編輯模式：整個畫面都要能拖曳
    private var mouseMonitor: Any?
    private var hoverPoll: Timer?

    private func startPassthrough() {
        window.ignoresMouseEvents = true
        mouseMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.mouseMoved, .leftMouseDragged, .rightMouseDragged]) { [weak self] _ in
            MainActor.assumeIsolated { self?.updatePassthrough() }
        }
    }

    private func updatePassthrough() {
        let inside: Bool
        if captureAll {
            inside = true
        } else {
            let m = NSEvent.mouseLocation, f = window.frame
            let p = CGPoint(x: m.x - f.minX, y: f.maxY - m.y)   // 轉成網頁座標（左上為原點）
            inside = f.contains(m) && hitRects.contains { $0.insetBy(dx: -14, dy: -14).contains(p) }
        }
        if window.ignoresMouseEvents == inside { window.ignoresMouseEvents = !inside }

        if inside && !captureAll && hoverPoll == nil {
            let t = Timer(timeInterval: 0.1, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.updatePassthrough() }
            }
            RunLoop.main.add(t, forMode: .common)
            hoverPoll = t
        } else if (!inside || captureAll), let t = hoverPoll {
            t.invalidate(); hoverPoll = nil
        }
    }

    // MARK: Window

    private func buildWindow() {
        let cfg = WKWebViewConfiguration()
        cfg.setURLSchemeHandler(DeckSchemeHandler(root: root), forURLScheme: "deck")
        cfg.userContentController.addScriptMessageHandler(self, contentWorld: .page, name: "deck")
        cfg.mediaTypesRequiringUserActionForPlayback = []

        let wv = WKWebView(frame: .zero, configuration: cfg)
        wv.setValue(false, forKey: "drawsBackground")   // 讓主題可以用透明背景透出系統桌布
        wv.isInspectable = true                          // Safari ▸ 開發 ▸ 可以檢查元素、調 CSS
        wv.navigationDelegate = self
        webView = wv

        let w = DesktopWindow(contentRect: .zero, styleMask: [.borderless], backing: .buffered, defer: false)
        w.isOpaque = false
        w.backgroundColor = .clear
        w.hasShadow = false
        w.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopIconWindow)) + 1)
        w.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
        w.contentView = wv
        window = w
        fitToScreen()
        w.orderFrontRegardless()

        wv.load(URLRequest(url: URL(string: "deck://app/index.html")!))
    }

    func fitToScreen() {
        guard let screen = NSScreen.screens.first else { return }
        window.setFrame(screen.frame, display: true)
    }

    func setVisible(_ v: Bool) {
        if v { window.orderFrontRegardless() } else { window.orderOut(nil) }
    }

    func reload() {
        ready = false
        subscriptions.removeAll()
        hitRects = []; captureAll = false
        updatePassthrough()
        webView.reload()
    }

    // MARK: Swift → JS

    func send(_ channel: String, _ payload: Any) {
        guard ready,
              let data = try? JSONSerialization.data(withJSONObject: ["c": channel, "p": payload]),
              let json = String(data: data, encoding: .utf8) else { return }
        webView.evaluateJavaScript("window.__deck&&window.__deck.receive(\(json))", completionHandler: nil)
    }

    func setEditMode(_ on: Bool, notifyJS: Bool = true) {
        editMode = on
        if notifyJS { send("editMode", on) }
        if on { NSApp.activate(ignoringOtherApps: true); window.makeKey() }
        onStateChange?()
    }

    func addWidget(_ type: String) {
        send("addWidget", type)
        if !editMode { setEditMode(true) }
    }

    func setTheme(_ name: String) {
        currentTheme = name
        send("setTheme", name)
        onStateChange?()
    }

    func refreshDisk() { send("disk", stats.disk()) }

    // MARK: 排程

    private func tick() {
        tickCount += 1
        let visible = window.isVisible && window.occlusionState.contains(.visible)
        guard ready, visible, !subscriptions.isEmpty else { wasActive = false; return }

        if !wasActive {             // 從「看不見」恢復：重設基準，避免第一筆數據是長時間平均
            stats.prime()
            wasActive = true
            return
        }
        if subscriptions.contains("stats") { send("stats", stats.fast()) }
        if subscriptions.contains("procs"), tickCount % 3 == 0 { send("procs", stats.procs()) }
        if subscriptions.contains("slow"), tickCount % 30 == 0 { send("slow", stats.slow()) }
    }

    private func filesChanged(_ paths: [String]) {
        var reloadPage = false, cssChanged = false
        for p in paths {
            let ext = (p as NSString).pathExtension.lowercased()
            if ext == "css" { cssChanged = true }
            else if ext == "js" || ext == "html" { reloadPage = true }
            else if p.hasSuffix("/layout.json") {
                // 我們自己存的版面不觸發重新載入；手動改 layout.json 才觸發
                if let d = try? Data(contentsOf: layoutURL), d != lastWrittenLayout { reloadPage = true }
            }
        }
        if reloadPage { reload() }
        else if cssChanged { send("cssChanged", [:]) }
    }

    // MARK: 版面檔

    private func readLayout() -> Any {
        if let d = try? Data(contentsOf: layoutURL),
           let obj = try? JSONSerialization.jsonObject(with: d) as? [String: Any] {
            if let t = obj["theme"] as? String { currentTheme = t }
            return obj
        }
        return ["theme": "cyberpunk", "widgets": []]
    }

    private func writeLayout(_ obj: Any) -> Bool {
        guard JSONSerialization.isValidJSONObject(obj),
              let d = try? JSONSerialization.data(withJSONObject: obj, options: [.prettyPrinted, .sortedKeys]) else { return false }
        lastWrittenLayout = d
        if let t = (obj as? [String: Any])?["theme"] as? String { currentTheme = t }
        return (try? d.write(to: layoutURL, options: .atomic)) != nil
    }

    func listFiles(_ dir: String, ext: String) -> [String] {
        let url = root.appendingPathComponent(dir)
        let items = (try? FileManager.default.contentsOfDirectory(atPath: url.path)) ?? []
        return items.filter { $0.hasSuffix(".\(ext)") && !$0.hasPrefix("_") && !$0.hasPrefix(".") }
            .map { String($0.dropLast(ext.count + 1)) }
            .sorted()
    }

    // MARK: JS → Swift

    func userContentController(_ uc: WKUserContentController, didReceive message: WKScriptMessage,
                               replyHandler: @escaping @MainActor @Sendable (Any?, String?) -> Void) {
        guard let body = message.body as? [String: Any], let method = body["method"] as? String else {
            replyHandler(nil, "bad message"); return
        }
        let p = body["params"]
        let ps = p as? [String: Any] ?? [:]

        switch method {
        case "ready":
            ready = true
            let payload: [String: Any] = [
                "layout": readLayout(),
                "themes": listFiles("themes", ext: "css"),
                "widgets": listFiles("widgets", ext: "js"),
                "info": stats.staticInfo(),
                "screen": ["w": window.frame.width, "h": window.frame.height],
            ]
            replyHandler(payload, nil)
            onStateChange?()

        case "subscribe":
            subscriptions = Set(p as? [String] ?? [])
            stats.prime()
            if subscriptions.contains("disk") { send("disk", stats.disk()) }
            if subscriptions.contains("slow") { send("slow", stats.slow()) }
            if subscriptions.contains("procs") { send("procs", stats.procs()) }
            replyHandler(true, nil)

        case "saveLayout":
            replyHandler(writeLayout(p as Any), nil)
            onStateChange?()

        case "setEditMode":
            setEditMode((p as? Bool) ?? false, notifyJS: false)
            replyHandler(true, nil)

        case "hitRegions":
            captureAll = ps["editing"] as? Bool ?? false
            hitRects = (ps["rects"] as? [[Double]] ?? []).compactMap { r in
                r.count == 4 ? CGRect(x: r[0], y: r[1], width: r[2], height: r[3]) : nil
            }
            updatePassthrough()
            replyHandler(true, nil)

        case "refreshDisk":
            replyHandler(stats.disk(), nil)

        case "open":
            // 開啟 App / 檔案 / 資料夾 / 網址
            guard let target = p as? String else { replyHandler(false, nil); return }
            let url = target.hasPrefix("/") || target.hasPrefix("~")
                ? URL(fileURLWithPath: (target as NSString).expandingTildeInPath)
                : URL(string: target)
            replyHandler(url.map { NSWorkspace.shared.open($0) } ?? false, nil)

        case "appIcon":
            replyHandler(Shell.iconDataURL(path: (p as? String) ?? "") ?? NSNull(), nil)

        case "pickApps":
            NSApp.activate(ignoringOtherApps: true)
            let panel = NSOpenPanel()
            panel.allowsMultipleSelection = true
            panel.canChooseDirectories = true
            panel.directoryURL = URL(fileURLWithPath: "/Applications")
            panel.prompt = "加入"
            let picked = panel.runModal() == .OK ? panel.urls : []
            replyHandler(picked.map { u in
                ["path": u.path, "name": FileManager.default.displayName(atPath: u.path)
                    .replacingOccurrences(of: ".app", with: "")]
            }, nil)

        case "notify":
            Shell.notify(title: ps["title"] as? String ?? "NeonDeck",
                         body: ps["body"] as? String ?? "",
                         sound: ps["sound"] as? String)
            replyHandler(true, nil)

        case "access":
            let type: EKEntityType = (p as? String) == "reminders" ? .reminder : .event
            eventKit.requestAccess(type) { replyHandler($0, nil) }

        case "events":
            let s = Date(timeIntervalSince1970: (ps["start"] as? Double ?? 0) / 1000)
            let e = Date(timeIntervalSince1970: (ps["end"] as? Double ?? 0) / 1000)
            replyHandler(eventKit.events(start: s, end: e), nil)

        case "reminders":
            eventKit.reminders { replyHandler($0, nil) }

        case "setReminder":
            replyHandler(eventKit.setReminder(id: ps["id"] as? String ?? "",
                                              completed: ps["completed"] as? Bool ?? true), nil)

        case "addReminder":
            replyHandler(eventKit.addReminder(title: ps["title"] as? String ?? ""), nil)

        case "activateApp":
            replyHandler(ProcSampler.activate(appPath: ps["appPath"] as? String, pid: ps["pid"] as? Int), nil)

        case "activityMonitor":
            NSWorkspace.shared.open(URL(fileURLWithPath: "/System/Applications/Utilities/Activity Monitor.app"))
            replyHandler(true, nil)

        case "revealInFinder":
            if let path = p as? String { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)]) }
            replyHandler(true, nil)

        case "fetchNews":
            let keywords = ps["keywords"] as? [String] ?? []
            let lang = ps["lang"] as? String ?? "zh-TW"
            let limit = ps["limit"] as? Int ?? 8
            Task { @MainActor in
                var out: [[String: Any]] = []
                for k in keywords where !k.trimmingCharacters(in: .whitespaces).isEmpty {
                    out.append(["keyword": k, "items": await NewsFetcher.fetch(keyword: k, lang: lang, limit: limit)])
                }
                replyHandler(out, nil)
            }

        case "log":
            DeckLog.write(String(describing: p ?? ""))
            replyHandler(true, nil)

        default:
            replyHandler(nil, "unknown method: \(method)")
        }
    }

    // MARK: Navigation：外部連結交給系統開，不讓桌面頁面被導走

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                 decisionHandler: @escaping @MainActor @Sendable (WKNavigationActionPolicy) -> Void) {
        if let url = action.request.url, url.scheme != "deck", url.scheme != "about" {
            NSWorkspace.shared.open(url)
            decisionHandler(.cancel); return
        }
        decisionHandler(.allow)
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) { reload() }
}

// MARK: - 記錄檔：~/Library/Logs/NeonDeck.log（JS 錯誤也會寫進來）

enum DeckLog {
    static let url = FileManager.default.urls(for: .libraryDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("Logs/NeonDeck.log")

    static func write(_ msg: String) {
        let f = DateFormatter(); f.dateFormat = "yyyy-MM-dd HH:mm:ss"
        let line = "\(f.string(from: Date())) \(msg)\n"
        guard let data = line.data(using: .utf8) else { return }
        if let h = try? FileHandle(forWritingTo: url) {
            h.seekToEndOfFile(); h.write(data); try? h.close()
        } else {
            try? data.write(to: url)
        }
    }
}
