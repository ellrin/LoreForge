import Foundation
import Darwin
import IOKit.ps

// MARK: - 資料結構

struct MemInfo {
    var used: UInt64 = 0        // App + Wired + Compressed（與「活動監視器」的「已使用記憶體」一致）
    var total: UInt64 = 0
    var app: UInt64 = 0
    var wired: UInt64 = 0
    var compressed: UInt64 = 0
    var swapUsed: UInt64 = 0
}

struct NetSample {
    var down: Double            // bytes/s
    var up: Double
    var rxTotal: UInt64         // 開機以來累計
    var txTotal: UInt64
}

struct DiskInfo {
    var free: Int64
    var total: Int64
}

struct BatteryInfo {
    var percent: Int
    var charging: Bool
    var onAC: Bool
}

struct ProcInfo: Identifiable {
    var id: pid_t
    var name: String
    var footprint: UInt64
}

/// 只在啟動時讀一次的靜態資訊
struct StaticInfo {
    let host: String
    let os: String
    let chip: String
    let cores: String
    let boot: Date

    static func load() -> StaticInfo {
        let v = ProcessInfo.processInfo.operatingSystemVersion
        var os = "macOS \(v.majorVersion).\(v.minorVersion)"
        if v.patchVersion > 0 { os += ".\(v.patchVersion)" }

        let chip = sysctlString("machdep.cpu.brand_string") ?? "Unknown"

        // 依效能層級組出 "6P+6E" 之類的字串
        var cores = "\(ProcessInfo.processInfo.processorCount)C"
        if let levels = sysctlInt("hw.nperflevels"), levels > 0 {
            var parts: [String] = []
            for i in 0..<levels {
                guard let n = sysctlInt("hw.perflevel\(i).physicalcpu") else { continue }
                let name = sysctlString("hw.perflevel\(i).name") ?? "L\(i)"
                parts.append("\(n)\(name.prefix(1))")
            }
            if !parts.isEmpty { cores = parts.joined(separator: "+") }
        }

        var tv = timeval()
        var sz = MemoryLayout<timeval>.size
        sysctlbyname("kern.boottime", &tv, &sz, nil, 0)
        let boot = Date(timeIntervalSince1970: Double(tv.tv_sec) + Double(tv.tv_usec) / 1e6)

        let host = Host.current().localizedName ?? ProcessInfo.processInfo.hostName
        return StaticInfo(host: host, os: os, chip: chip, cores: cores, boot: boot)
    }
}

func sysctlString(_ name: String) -> String? {
    var size = 0
    guard sysctlbyname(name, nil, &size, nil, 0) == 0, size > 0 else { return nil }
    var buf = [CChar](repeating: 0, count: size)
    guard sysctlbyname(name, &buf, &size, nil, 0) == 0 else { return nil }
    return String(cString: buf)
}

func sysctlInt(_ name: String) -> Int? {
    var v: Int32 = 0
    var size = MemoryLayout<Int32>.size
    guard sysctlbyname(name, &v, &size, nil, 0) == 0 else { return nil }
    return Int(v)
}

// MARK: - 取樣器（全部走 syscall，不開子程序）

final class SystemSampler {
    private let host = mach_host_self()
    private let pageSize: UInt64
    let totalMemory: UInt64

    private var prevTicks: (UInt32, UInt32, UInt32, UInt32)?
    private var prevNet: (rx: UInt64, tx: UInt64, t: TimeInterval)?
    private var netBuf = [UInt8]()
    private var ifCache: [UInt16: Bool] = [:]

    init() {
        var ps: vm_size_t = 0
        host_page_size(host, &ps)
        pageSize = UInt64(ps)

        var mem: UInt64 = 0
        var sz = MemoryLayout<UInt64>.size
        sysctlbyname("hw.memsize", &mem, &sz, nil, 0)
        totalMemory = mem
    }

    func resetCPU() { prevTicks = nil }
    func resetNet() { prevNet = nil }

    /// 0...1，第一次呼叫只建立基準，回傳 nil
    func cpu() -> Double? {
        var info = host_cpu_load_info()
        var count = mach_msg_type_number_t(MemoryLayout<host_cpu_load_info>.stride / MemoryLayout<integer_t>.stride)
        let kr = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                host_statistics(host, HOST_CPU_LOAD_INFO, $0, &count)
            }
        }
        guard kr == KERN_SUCCESS else { return nil }
        let t = info.cpu_ticks // (user, system, idle, nice)
        defer { prevTicks = t }
        guard let p = prevTicks else { return nil }
        let user = Double(t.0 &- p.0), sys = Double(t.1 &- p.1)
        let idle = Double(t.2 &- p.2), nice = Double(t.3 &- p.3)
        let total = user + sys + idle + nice
        return total > 0 ? (user + sys + nice) / total : 0
    }

    func memory() -> MemInfo {
        var s = vm_statistics64()
        var count = mach_msg_type_number_t(MemoryLayout<vm_statistics64>.stride / MemoryLayout<integer_t>.stride)
        _ = withUnsafeMutablePointer(to: &s) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                host_statistics64(host, HOST_VM_INFO64, $0, &count)
            }
        }
        let internalPages = UInt64(s.internal_page_count)
        let purgeable = UInt64(s.purgeable_count)
        let app = (internalPages > purgeable ? internalPages - purgeable : 0) * pageSize
        let wired = UInt64(s.wire_count) * pageSize
        let comp = UInt64(s.compressor_page_count) * pageSize

        var swap = xsw_usage()
        var sz = MemoryLayout<xsw_usage>.size
        sysctlbyname("vm.swapusage", &swap, &sz, nil, 0)

        return MemInfo(used: app + wired + comp, total: totalMemory,
                       app: app, wired: wired, compressed: comp, swapUsed: swap.xsu_used)
    }

    /// 1 = normal, 2 = warning, 4 = critical
    func pressureLevel() -> Int {
        sysctlInt("kern.memorystatus_vm_pressure_level") ?? 1
    }

    /// 用 NET_RT_IFLIST2 拿 64-bit 計數器（getifaddrs 的 if_data 是 32-bit，超過 4GB 會溢位）
    func network() -> NetSample? {
        var mib: [Int32] = [CTL_NET, PF_ROUTE, 0, 0, NET_RT_IFLIST2, 0]
        var len = 0
        guard sysctl(&mib, 6, nil, &len, nil, 0) == 0 else { return nil }
        if netBuf.count < len { netBuf = [UInt8](repeating: 0, count: len + 1024) }
        let ok = netBuf.withUnsafeMutableBytes { sysctl(&mib, 6, $0.baseAddress, &len, nil, 0) }
        guard ok == 0 else { return nil }

        var rx: UInt64 = 0, tx: UInt64 = 0
        netBuf.withUnsafeBytes { raw in
            var off = 0
            while off + MemoryLayout<if_msghdr>.size <= len {
                let h = raw.loadUnaligned(fromByteOffset: off, as: if_msghdr.self)
                if h.ifm_msglen == 0 { break }
                if Int32(h.ifm_type) == RTM_IFINFO2,
                   off + MemoryLayout<if_msghdr2>.size <= len {
                    let h2 = raw.loadUnaligned(fromByteOffset: off, as: if_msghdr2.self)
                    if isPhysical(h2.ifm_index) {
                        rx &+= h2.ifm_data.ifi_ibytes
                        tx &+= h2.ifm_data.ifi_obytes
                    }
                }
                off += Int(h.ifm_msglen)
            }
        }

        let now = ProcessInfo.processInfo.systemUptime
        defer { prevNet = (rx, tx, now) }
        guard let p = prevNet, now > p.t else { return nil }
        let dt = now - p.t
        let down = rx >= p.rx ? Double(rx - p.rx) / dt : 0
        let up = tx >= p.tx ? Double(tx - p.tx) / dt : 0
        return NetSample(down: down, up: up, rxTotal: rx, txTotal: tx)
    }

    /// 只算 en*（Wi-Fi / 乙太網路），排除 lo0、utun(VPN 會重複計算)、awdl 等
    private func isPhysical(_ idx: UInt16) -> Bool {
        if let v = ifCache[idx] { return v }
        var name = [CChar](repeating: 0, count: Int(IF_NAMESIZE) + 1)
        var ok = false
        if if_indextoname(UInt32(idx), &name) != nil {
            ok = String(cString: name).hasPrefix("en")
        }
        ifCache[idx] = ok
        return ok
    }

    /// 記憶體用量前幾名（phys_footprint，與活動監視器相同口徑）
    func topMemoryProcesses(limit: Int = 3) -> [ProcInfo] {
        let n = proc_listallpids(nil, 0)
        guard n > 0 else { return [] }
        var pids = [pid_t](repeating: 0, count: Int(n) + 64)
        let got = proc_listallpids(&pids, Int32(pids.count * MemoryLayout<pid_t>.size))
        guard got > 0 else { return [] }

        var results: [(pid_t, UInt64)] = []
        results.reserveCapacity(Int(got))
        for pid in pids.prefix(Int(got)) where pid > 0 {
            var ri = rusage_info_v4()
            let r = withUnsafeMutablePointer(to: &ri) {
                $0.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) {
                    proc_pid_rusage(pid, RUSAGE_INFO_V4, $0)
                }
            }
            if r == 0 { results.append((pid, ri.ri_phys_footprint)) }
        }
        results.sort { $0.1 > $1.1 }

        return results.prefix(limit).map { pid, fp in
            var buf = [CChar](repeating: 0, count: 256)
            proc_name(pid, &buf, UInt32(buf.count))
            let name = String(cString: buf)
            return ProcInfo(id: pid, name: name.isEmpty ? "pid \(pid)" : name, footprint: fp)
        }
    }

    // 以下為低頻 / 一次性

    static func disk() -> DiskInfo? {
        let url = URL(fileURLWithPath: "/")
        let keys: Set<URLResourceKey> = [.volumeTotalCapacityKey, .volumeAvailableCapacityForImportantUsageKey]
        guard let v = try? url.resourceValues(forKeys: keys),
              let total = v.volumeTotalCapacity,
              let free = v.volumeAvailableCapacityForImportantUsage else { return nil }
        return DiskInfo(free: free, total: Int64(total))
    }

    static func battery() -> BatteryInfo? {
        guard let blob = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),
              let list = IOPSCopyPowerSourcesList(blob)?.takeRetainedValue() as? [CFTypeRef] else { return nil }
        for ps in list {
            guard let d = IOPSGetPowerSourceDescription(blob, ps)?.takeUnretainedValue() as? [String: Any],
                  let cap = d[kIOPSCurrentCapacityKey] as? Int,
                  let max = d[kIOPSMaxCapacityKey] as? Int, max > 0 else { continue }
            let charging = d[kIOPSIsChargingKey] as? Bool ?? false
            let onAC = (d[kIOPSPowerSourceStateKey] as? String) == kIOPSACPowerValue
            return BatteryInfo(percent: cap * 100 / max, charging: charging, onAC: onAC)
        }
        return nil
    }

    static func localIP() -> String? {
        var ifap: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&ifap) == 0, let first = ifap else { return nil }
        defer { freeifaddrs(ifap) }
        for p in sequence(first: first, next: { $0.pointee.ifa_next }) {
            let ifa = p.pointee
            guard let sa = ifa.ifa_addr, sa.pointee.sa_family == UInt8(AF_INET),
                  String(cString: ifa.ifa_name).hasPrefix("en") else { continue }
            var hostBuf = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(sa, socklen_t(sa.pointee.sa_len), &hostBuf, socklen_t(hostBuf.count),
                           nil, 0, NI_NUMERICHOST) == 0 {
                return String(cString: hostBuf)
            }
        }
        return nil
    }
}
