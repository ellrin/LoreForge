import AppKit
import Darwin

/// 程序排行：CPU%（兩次取樣之間的差值）與記憶體（依 App 合併）。
/// 只看得到「你這個使用者」的程序；root / 系統帳號的程序（kernel_task、WindowServer 等）macOS 不讓一般 App 讀。
final class ProcSampler {
    private struct Info { let path: String; let name: String; let appPath: String?; let appName: String? }

    private var prevCPU: [pid_t: UInt64] = [:]     // 累計 CPU 時間（ns）
    private var prevAt: UInt64 = 0                  // 取樣時間（ns）
    private var infoCache: [pid_t: Info] = [:]
    private let tbNumer: UInt64, tbDenom: UInt64
    private var pids = [pid_t](repeating: 0, count: 4096)

    init() {
        var tb = mach_timebase_info()
        mach_timebase_info(&tb)
        tbNumer = UInt64(tb.numer); tbDenom = UInt64(tb.denom)
    }

    func sample(topCPU: Int = 8, topApps: Int = 8) -> [String: Any] {
        let n = Int(proc_listallpids(&pids, Int32(pids.count * MemoryLayout<pid_t>.size)))
        let now = clock_gettime_nsec_np(CLOCK_UPTIME_RAW)
        let dt = prevAt > 0 ? Double(now - prevAt) : 0

        var cpuNow: [pid_t: UInt64] = [:]
        cpuNow.reserveCapacity(n)
        var cpuRows: [(pid_t, Double, UInt64)] = []
        var groups: [String: (name: String, appPath: String?, mem: UInt64, count: Int, pid: pid_t)] = [:]

        for pid in pids.prefix(max(0, n)) where pid > 0 {
            var ri = rusage_info_v4()
            let r = withUnsafeMutablePointer(to: &ri) {
                $0.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) { proc_pid_rusage(pid, RUSAGE_INFO_V4, $0) }
            }
            guard r == 0 else { continue }
            let cpuNs = (ri.ri_user_time &+ ri.ri_system_time) * tbNumer / tbDenom
            cpuNow[pid] = cpuNs
            if dt > 0, let p = prevCPU[pid], cpuNs >= p {
                let pct = Double(cpuNs - p) / dt * 100
                if pct >= 0.1 { cpuRows.append((pid, pct, ri.ri_phys_footprint)) }
            }
            guard let info = info(pid) else { continue }
            let key = info.appPath ?? "proc:" + info.name
            var g = groups[key] ?? (info.appName ?? info.name, info.appPath, 0, 0, pid)
            g.mem += ri.ri_phys_footprint
            g.count += 1
            groups[key] = g
        }

        // 清掉已結束程序的快取
        if infoCache.count > n + 200 { infoCache = infoCache.filter { cpuNow[$0.key] != nil } }
        prevCPU = cpuNow
        prevAt = now

        let cpu: [[String: Any]] = cpuRows.sorted { $0.1 > $1.1 }.prefix(topCPU).compactMap { pid, pct, mem in
            guard let i = info(pid) else { return nil }
            return ["pid": Int(pid), "name": i.name, "app": i.appName ?? NSNull(), "appPath": i.appPath ?? NSNull(),
                    "path": i.path, "cpu": pct, "mem": Double(mem)]
        }
        let mem: [[String: Any]] = groups.values.sorted { $0.mem > $1.mem }.prefix(topApps).map { g in
            ["name": g.name, "appPath": g.appPath ?? NSNull(), "mem": Double(g.mem), "count": g.count, "pid": Int(g.pid)]
        }
        return ["cpu": cpu, "mem": mem, "cores": ProcessInfo.processInfo.activeProcessorCount, "ready": dt > 0]
    }

    /// 執行檔路徑 → 所屬 App（取最外層的 .app，例如 Chrome Helper 會歸到 Google Chrome.app）
    private func info(_ pid: pid_t) -> Info? {
        if let c = infoCache[pid] { return c }
        var buf = [CChar](repeating: 0, count: 4096)
        guard proc_pidpath(pid, &buf, UInt32(buf.count)) > 0 else { return nil }
        let path = String(cString: buf)
        let name = (path as NSString).lastPathComponent
        var appPath: String?, appName: String?
        if let r = path.range(of: ".app/") {
            appPath = String(path[..<r.lowerBound]) + ".app"
            appName = FileManager.default.displayName(atPath: appPath!).replacingOccurrences(of: ".app", with: "")
        }
        let i = Info(path: path, name: name, appPath: appPath, appName: appName)
        infoCache[pid] = i
        return i
    }

    /// 切到該程序所屬的 App（已開就帶到前景，沒開就打開）
    static func activate(appPath: String?, pid: Int?) -> Bool {
        if let p = appPath {
            if let app = NSWorkspace.shared.runningApplications.first(where: { $0.bundleURL?.path == p }) {
                return app.activate()
            }
            return NSWorkspace.shared.open(URL(fileURLWithPath: p))
        }
        if let pid, let app = NSRunningApplication(processIdentifier: pid_t(pid)) { return app.activate() }
        return false
    }
}

// MARK: - 新聞：Google News RSS（依關鍵字搜尋，不需要爬網頁）

enum NewsFetcher {
    struct Item { var title = "", link = "", source = "", pubDate = "" }

    final class RSSParser: NSObject, XMLParserDelegate {
        var items: [Item] = []
        private var cur: Item?
        private var text = ""
        func parser(_ p: XMLParser, didStartElement e: String, namespaceURI: String?, qualifiedName: String?, attributes: [String: String] = [:]) {
            if e == "item" { cur = Item() }
            text = ""
        }
        func parser(_ p: XMLParser, foundCharacters s: String) { text += s }
        func parser(_ p: XMLParser, didEndElement e: String, namespaceURI: String?, qualifiedName: String?) {
            let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
            switch e {
            case "title": cur?.title = t
            case "link": cur?.link = t
            case "source": cur?.source = t
            case "pubDate": cur?.pubDate = t
            case "item": if let c = cur { items.append(c) }; cur = nil
            default: break
            }
        }
    }

    static func fetch(keyword: String, lang: String, limit: Int) async -> [[String: Any]] {
        let (hl, gl, ceid) = lang == "en" ? ("en-US", "US", "US:en") : ("zh-TW", "TW", "TW:zh-Hant")
        var c = URLComponents(string: "https://news.google.com/rss/search")!
        c.queryItems = [.init(name: "q", value: keyword), .init(name: "hl", value: hl),
                        .init(name: "gl", value: gl), .init(name: "ceid", value: ceid)]
        guard let url = c.url, let (data, _) = try? await URLSession.shared.data(from: url) else { return [] }
        let parser = XMLParser(data: data)
        let d = RSSParser()
        parser.delegate = d
        parser.parse()
        let df = DateFormatter()
        df.locale = Locale(identifier: "en_US_POSIX")
        df.dateFormat = "EEE, dd MMM yyyy HH:mm:ss zzz"
        return d.items.prefix(limit).map { it in
            var title = it.title
            // Google News 標題結尾是「 - 來源」，拿掉
            if !it.source.isEmpty, title.hasSuffix(" - " + it.source) { title = String(title.dropLast(it.source.count + 3)) }
            let ts = df.date(from: it.pubDate)?.timeIntervalSince1970 ?? 0
            return ["title": title, "link": it.link, "source": it.source, "time": ts * 1000]
        }
    }
}
