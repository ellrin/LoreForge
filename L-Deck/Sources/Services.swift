import AppKit
import EventKit
import UserNotifications

// MARK: - 系統數據 → JSON 字典

final class StatsService {
    let sampler = SystemSampler()
    let procSampler = ProcSampler()
    let info = StaticInfo.load()

    func prime() {
        sampler.resetCPU(); sampler.resetNet()
        _ = sampler.cpu(); _ = sampler.network()
    }

    /// 每秒：網速 / 記憶體 / CPU
    func fast() -> [String: Any] {
        var d: [String: Any] = [:]
        if let c = sampler.cpu() { d["cpu"] = c }
        if let n = sampler.network() {
            d["down"] = n.down; d["up"] = n.up
            d["rx"] = Double(n.rxTotal); d["tx"] = Double(n.txTotal)
        }
        let m = sampler.memory()
        d["mem"] = ["used": Double(m.used), "total": Double(m.total), "app": Double(m.app),
                    "wired": Double(m.wired), "compressed": Double(m.compressed), "swap": Double(m.swapUsed)]
        d["pressure"] = sampler.pressureLevel()
        var la = [Double](repeating: 0, count: 3)
        getloadavg(&la, 3)
        d["load"] = la
        d["uptime"] = Date().timeIntervalSince(info.boot)
        return d
    }

    /// 每 3 秒：CPU 正在處理的程序 + 記憶體大戶（依 App 合併）
    func procs() -> [String: Any] { procSampler.sample() }

    /// 每 30 秒：電池 / IP / 溫度狀態
    func slow() -> [String: Any] {
        var d: [String: Any] = [:]
        if let b = SystemSampler.battery() {
            d["battery"] = ["percent": b.percent, "charging": b.charging, "ac": b.onAC]
        }
        d["ip"] = SystemSampler.localIP() ?? NSNull()
        let t: String
        switch ProcessInfo.processInfo.thermalState {
        case .nominal: t = "nominal"
        case .fair: t = "fair"
        case .serious: t = "serious"
        case .critical: t = "critical"
        @unknown default: t = "unknown"
        }
        d["thermal"] = t
        return d
    }

    /// 只在啟動 / 手動要求時計算
    func disk() -> [String: Any] {
        guard let d = SystemSampler.disk() else { return [:] }
        return ["free": Double(d.free), "total": Double(d.total), "at": Date().timeIntervalSince1970 * 1000]
    }

    func staticInfo() -> [String: Any] {
        ["host": info.host, "os": info.os, "chip": info.chip, "cores": info.cores,
         "boot": info.boot.timeIntervalSince1970 * 1000, "memTotal": Double(sampler.totalMemory)]
    }
}

// MARK: - 日曆 / 提醒事項

final class EventKitService {
    let store = EKEventStore()
    var onChange: (() -> Void)?

    init() {
        NotificationCenter.default.addObserver(forName: .EKEventStoreChanged, object: store, queue: .main) { [weak self] _ in
            self?.onChange?()
        }
    }

    private func statusString(_ type: EKEntityType) -> String {
        switch EKEventStore.authorizationStatus(for: type) {
        case .fullAccess: return "granted"
        case .notDetermined: return "notDetermined"
        case .writeOnly: return "writeOnly"
        default: return "denied"
        }
    }

    func requestAccess(_ type: EKEntityType, _ done: @escaping (String) -> Void) {
        guard EKEventStore.authorizationStatus(for: type) == .notDetermined else {
            done(statusString(type)); return
        }
        let finish: (Bool, Error?) -> Void = { [weak self] _, _ in
            DispatchQueue.main.async { done(self?.statusString(type) ?? "denied") }
        }
        if type == .event { store.requestFullAccessToEvents(completion: finish) }
        else { store.requestFullAccessToReminders(completion: finish) }
    }

    func events(start: Date, end: Date) -> [[String: Any]] {
        guard EKEventStore.authorizationStatus(for: .event) == .fullAccess else { return [] }
        let pred = store.predicateForEvents(withStart: start, end: end, calendars: nil)
        return store.events(matching: pred).map { e in
            [
                "id": e.eventIdentifier ?? UUID().uuidString,
                "title": e.title ?? "",
                "start": e.startDate.timeIntervalSince1970 * 1000,
                "end": e.endDate.timeIntervalSince1970 * 1000,
                "allDay": e.isAllDay,
                "calendar": e.calendar?.title ?? "",
                "color": hex(e.calendar?.cgColor),
                "location": e.location ?? "",
            ]
        }
    }

    func reminders(_ done: @escaping ([[String: Any]]) -> Void) {
        guard EKEventStore.authorizationStatus(for: .reminder) == .fullAccess else { done([]); return }
        let pred = store.predicateForIncompleteReminders(withDueDateStarting: nil, ending: nil, calendars: nil)
        store.fetchReminders(matching: pred) { [weak self] rs in
            let items: [[String: Any]] = (rs ?? []).map { r in
                var d: [String: Any] = [
                    "id": r.calendarItemIdentifier,
                    "title": r.title ?? "",
                    "list": r.calendar?.title ?? "",
                    "color": self?.hex(r.calendar?.cgColor) ?? "",
                    "priority": r.priority,
                ]
                if let due = r.dueDateComponents, let date = Calendar.current.date(from: due) {
                    d["due"] = date.timeIntervalSince1970 * 1000
                    d["dueHasTime"] = due.hour != nil
                }
                return d
            }
            DispatchQueue.main.async { done(items) }
        }
    }

    func setReminder(id: String, completed: Bool) -> Bool {
        guard let r = store.calendarItem(withIdentifier: id) as? EKReminder else { return false }
        r.isCompleted = completed
        return (try? store.save(r, commit: true)) != nil
    }

    func addReminder(title: String) -> Bool {
        guard let cal = store.defaultCalendarForNewReminders() else { return false }
        let r = EKReminder(eventStore: store)
        r.title = title
        r.calendar = cal
        return (try? store.save(r, commit: true)) != nil
    }

    private func hex(_ c: CGColor?) -> String {
        guard let c, let rgb = c.converted(to: CGColorSpace(name: CGColorSpace.sRGB)!, intent: .defaultIntent, options: nil),
              let comps = rgb.components, comps.count >= 3 else { return "" }
        return String(format: "#%02X%02X%02X", Int(comps[0] * 255), Int(comps[1] * 255), Int(comps[2] * 255))
    }
}

// MARK: - App 圖示 / 通知

enum Shell {
    private static var iconCache: [String: String] = [:]

    static func iconDataURL(path: String, size: Int = 96) -> String? {
        if let c = iconCache[path] { return c }
        guard FileManager.default.fileExists(atPath: path) else { return nil }
        let img = NSWorkspace.shared.icon(forFile: path)
        guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
                                         bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                                         colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) else { return nil }
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
        img.draw(in: NSRect(x: 0, y: 0, width: size, height: size))
        NSGraphicsContext.restoreGraphicsState()
        guard let png = rep.representation(using: .png, properties: [:]) else { return nil }
        let url = "data:image/png;base64," + png.base64EncodedString()
        iconCache[path] = url
        return url
    }

    /// 打開「行事曆」；有日期時用 AppleScript 切到那天（`view` = "day view" / "month view"）。
    /// 第一次會問自動化權限；被拒絕或腳本失敗時只打開 App。
    static func showCalendar(at: Date?, view: String) {
        let app = URL(fileURLWithPath: "/System/Applications/Calendar.app")
        guard let at else { NSWorkspace.shared.open(app); return }
        let c = Calendar.current.dateComponents([.year, .month, .day], from: at)
        let src = """
        set d to current date
        set day of d to 1
        set year of d to \(c.year!)
        set month of d to \(c.month!)
        set day of d to \(c.day!)
        tell application "Calendar"
          activate
          switch view to \(view)
          view calendar at d
        end tell
        """
        var err: NSDictionary?
        NSAppleScript(source: src)?.executeAndReturnError(&err)
        if let err {
            DeckLog.write("showCalendar: \(err)")
            NSWorkspace.shared.open(app)
        }
    }

    private static var notifAsked = false

    static func notify(title: String, body: String, sound: String?) {
        if let sound, let s = NSSound(named: NSSound.Name(sound)) { s.stop(); s.play() }
        if title.isEmpty { return }   // 只播音效（鬧鐘重複響鈴用）
        let center = UNUserNotificationCenter.current()
        let post = {
            let c = UNMutableNotificationContent()
            c.title = title; c.body = body
            center.add(UNNotificationRequest(identifier: UUID().uuidString, content: c, trigger: nil))
        }
        if notifAsked { post(); return }
        notifAsked = true
        center.requestAuthorization(options: [.alert, .sound]) { ok, _ in if ok { post() } }
    }
}

// MARK: - 檔案監看（熱更新）

final class FileWatcher {
    private var stream: FSEventStreamRef?
    private let handler: ([String]) -> Void

    init(path: String, handler: @escaping ([String]) -> Void) {
        self.handler = handler
        var ctx = FSEventStreamContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(),
                                       retain: nil, release: nil, copyDescription: nil)
        let cb: FSEventStreamCallback = { _, info, _, paths, _, _ in
            guard let info else { return }
            let me = Unmanaged<FileWatcher>.fromOpaque(info).takeUnretainedValue()
            let arr = unsafeBitCast(paths, to: NSArray.self) as? [String] ?? []
            me.handler(arr)
        }
        let flags = FSEventStreamCreateFlags(kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagFileEvents)
        guard let s = FSEventStreamCreate(nil, cb, &ctx, [path] as CFArray,
                                          FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.25, flags) else { return }
        stream = s
        FSEventStreamSetDispatchQueue(s, .main)
        FSEventStreamStart(s)
    }

    deinit {
        if let s = stream { FSEventStreamStop(s); FSEventStreamInvalidate(s); FSEventStreamRelease(s) }
    }
}
