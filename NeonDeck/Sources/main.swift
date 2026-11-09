import AppKit
import ServiceManagement

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private let deck = DeckController()
    private var statusItem: NSStatusItem!
    private let menu = NSMenu()

    func applicationDidFinishLaunching(_ notification: Notification) {
        deck.start()
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem.button?.image = NSImage(systemSymbolName: "square.grid.3x3.topleft.filled",
                                           accessibilityDescription: "NeonDeck")
        menu.delegate = self
        statusItem.menu = menu
    }

    // 已經在執行時再點 Dock / Launchpad / Finder 裡的圖示：切換顯示 / 隱藏桌面
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        deck.setVisible(!deck.window.isVisible)
        return false
    }

    // 每次打開選單時重建，確保主題 / 小工具清單與勾選狀態是最新的
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()

        let edit = item(deck.editMode ? "完成編輯" : "編輯版面", #selector(toggleEdit), "e")
        menu.addItem(edit)

        let add = NSMenuItem(title: "新增小工具", action: nil, keyEquivalent: "")
        let addMenu = NSMenu()
        for w in deck.listFiles("widgets", ext: "js") {
            let i = item(w, #selector(addWidget(_:)))
            i.representedObject = w
            addMenu.addItem(i)
        }
        add.submenu = addMenu
        menu.addItem(add)

        let theme = NSMenuItem(title: "主題", action: nil, keyEquivalent: "")
        let themeMenu = NSMenu()
        for t in deck.listFiles("themes", ext: "css") {
            let i = item(t, #selector(setTheme(_:)))
            i.representedObject = t
            i.state = t == deck.currentTheme ? .on : .off
            themeMenu.addItem(i)
        }
        theme.submenu = themeMenu
        menu.addItem(theme)

        menu.addItem(.separator())
        menu.addItem(item("重新載入桌面", #selector(reload), "r"))
        menu.addItem(item("重新計算儲存空間", #selector(refreshDisk)))
        menu.addItem(item("打開桌面資料夾", #selector(openFolder)))
        menu.addItem(.separator())

        let vis = item("顯示桌面", #selector(toggleVisible))
        vis.state = deck.window.isVisible ? .on : .off
        menu.addItem(vis)
        let login = item("登入時自動啟動", #selector(toggleLogin))
        login.state = SMAppService.mainApp.status == .enabled ? .on : .off
        menu.addItem(login)
        menu.addItem(.separator())
        menu.addItem(item("結束 NeonDeck", #selector(quit), "q"))
    }

    private func item(_ title: String, _ action: Selector, _ key: String = "") -> NSMenuItem {
        let i = NSMenuItem(title: title, action: action, keyEquivalent: key)
        i.target = self
        return i
    }

    @objc private func toggleEdit() { deck.setEditMode(!deck.editMode) }
    @objc private func addWidget(_ sender: NSMenuItem) { deck.addWidget(sender.representedObject as? String ?? "") }
    @objc private func setTheme(_ sender: NSMenuItem) { deck.setTheme(sender.representedObject as? String ?? "") }
    @objc private func reload() { deck.reload() }
    @objc private func refreshDisk() { deck.refreshDisk() }
    @objc private func openFolder() { NSWorkspace.shared.open(deck.root) }
    @objc private func toggleVisible() { deck.setVisible(!deck.window.isVisible) }
    @objc private func quit() { NSApp.terminate(nil) }

    @objc private func toggleLogin() {
        let svc = SMAppService.mainApp
        do {
            if svc.status == .enabled { try svc.unregister() } else { try svc.register() }
        } catch {
            let a = NSAlert()
            a.messageText = "無法設定登入項目"
            a.informativeText = "\(error.localizedDescription)\n\n請確認 NeonDeck.app 已安裝到「應用程式」資料夾，或到「系統設定 ▸ 一般 ▸ 登入項目」手動加入。"
            a.runModal()
        }
        if svc.status == .requiresApproval { SMAppService.openSystemSettingsLoginItems() }
    }
}

MainActor.assumeIsolated {
    let app = NSApplication.shared
    let delegate = AppDelegate()
    app.delegate = delegate
    app.setActivationPolicy(.accessory)
    app.run()
}
