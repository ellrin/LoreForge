// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! LoreShelf's desktop shell. shelf-server runs in-process (or an already running one
//! answering `/api/health` on the configured port is reused); one transparent (window buttons over the page, no title bar),
//! resizable window with macOS vibrancy and rounded corners shows the page (it draws its own
//! translucent backdrop and drags the window by its top bar, `data-tauri-drag-region`).
//! First run: 85 % of the main screen's work area (no Dock, no menu bar), centred. Moves and
//! resizes are saved (debounced) in `<catalog>/window.json` with the screen's name and restored
//! on that screen (kept inside its work area; the main screen when it is gone). A regular app: Dock icon, app menu (隱藏, 結束 ⌘Q); a Dock click shows or hides
//! the window; the page hides it on Esc with `window.__TAURI__.core.invoke("hide")`.
//!
//! Activity for the page (render on demand, release WebGL when hidden): focus / blur,
//! minimise, hide and macOS occlusion (`NSWindow.occlusionState`, polled once a second on
//! the main thread) become `window.LoreShelf && LoreShelf.setActivity("active" | "blurred" | "hidden")`,
//! sent only when the state changes.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::utils::config::WindowEffectsConfig;
use tauri::window::{Effect, EffectState};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

/// How often visibility / occlusion is checked.
const POLL: Duration = Duration::from_secs(1);
/// Corner radius of the vibrancy view (the page rounds its own backdrop the same).
const CORNER: f64 = 14.0;

#[derive(Default)]
struct Desk {
    focused: AtomicBool,
    last: Mutex<String>,
    /// Bumped by every move / resize; a save runs only if no newer one came.
    moves: std::sync::atomic::AtomicU64,
    state_file: Mutex<Option<std::path::PathBuf>>,
}

const MIN_W: f64 = 900.0;
const MIN_H: f64 = 600.0;
/// Share of the work area the window takes the first time.
const FIRST_SHARE: f64 = 0.85;
const SAVE_AFTER: Duration = Duration::from_millis(500);
const STATE_FILE: &str = "window.json";

/// The window's place (logical pixels) and the screen it was on.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Place {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    #[serde(default)]
    screen: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// A monitor's work area (no Dock, no menu bar) in logical pixels.
fn work_of(m: &tauri::Monitor) -> Rect {
    let s = m.scale_factor();
    let wa = m.work_area();
    Rect { x: wa.position.x as f64 / s, y: wa.position.y as f64 / s, w: wa.size.width as f64 / s, h: wa.size.height as f64 / s }
}

/// The first-run place: `FIRST_SHARE` of the work area, centred.
fn first_place(work: Rect) -> Rect {
    let (w, h) = ((work.w * FIRST_SHARE).max(MIN_W.min(work.w)), (work.h * FIRST_SHARE).max(MIN_H.min(work.h)));
    Rect { x: work.x + (work.w - w) / 2.0, y: work.y + (work.h - h) / 2.0, w, h }
}

/// `r` no larger than `work` and moved inside it.
fn fit(r: Rect, work: Rect) -> Rect {
    let (w, h) = (r.w.min(work.w).max(MIN_W.min(work.w)), r.h.min(work.h).max(MIN_H.min(work.h)));
    let x = r.x.min(work.x + work.w - w).max(work.x);
    let y = r.y.min(work.y + work.h - h).max(work.y);
    Rect { x, y, w, h }
}

/// Where the window opens: the saved place on its screen (fitted), else the first-run place on the main screen.
fn opening_place(app: &AppHandle, saved: Option<Place>) -> Rect {
    let monitors = app.available_monitors().unwrap_or_default();
    let main = app.primary_monitor().ok().flatten().or_else(|| monitors.first().cloned());
    if let Some(p) = saved {
        let on = monitors
            .iter()
            .find(|m| p.screen.is_some() && m.name().map(|n| Some(n.as_str()) == p.screen.as_deref()).unwrap_or(false))
            .or_else(|| monitors.iter().find(|m| {
                let w = work_of(m);
                p.x + p.w / 2.0 >= w.x && p.x + p.w / 2.0 < w.x + w.w && p.y + p.h / 2.0 >= w.y && p.y + p.h / 2.0 < w.y + w.h
            }));
        if let Some(m) = on {
            return fit(Rect { x: p.x, y: p.y, w: p.w, h: p.h }, work_of(m));
        }
    }
    match main {
        Some(m) => first_place(work_of(&m)),
        None => Rect { x: 80.0, y: 80.0, w: 1280.0, h: 800.0 },
    }
}

fn load_place(file: &std::path::Path) -> Option<Place> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

/// Remember the window's place once it has rested.
fn moved(win: &WebviewWindow) {
    let app = win.app_handle().clone();
    let n = app.state::<Desk>().moves.fetch_add(1, Ordering::SeqCst) + 1;
    let win = win.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SAVE_AFTER);
        let desk = app.state::<Desk>();
        if desk.moves.load(Ordering::SeqCst) != n || win.is_minimized().unwrap_or(false) || win.is_fullscreen().unwrap_or(false) || !win.is_visible().unwrap_or(true) {
            return;
        }
        let (Ok(pos), Ok(size), Ok(scale)) = (win.outer_position(), win.inner_size(), win.scale_factor()) else { return };
        let (p, s) = (pos.to_logical::<f64>(scale), size.to_logical::<f64>(scale));
        let screen = win.current_monitor().ok().flatten().and_then(|m| m.name().cloned());
        let place = Place { x: p.x, y: p.y, w: s.width, h: s.height, screen };
        let Some(file) = desk.state_file.lock().unwrap().clone() else { return };
        if let Ok(b) = serde_json::to_vec_pretty(&place) {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = lore_catalog::store::write_atomic(&file, &b);
        }
    });
}

/// A shelf-server already answers on `addr`.
fn shelf_at(addr: SocketAddr) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let req = format!("GET /api/health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut body = String::new();
    let _ = s.read_to_string(&mut body);
    // only a server of this very build: an older copy still running lacks what the page asks for
    body.starts_with("HTTP/1.1 200") && body.contains("\"app\":\"LoreShelf\"") && body.contains(&format!("\"build\":{}", shelf_server::build_stamp()))
}

/// Start shelf-server on its own runtime thread; its address.
fn start_server(web: std::path::PathBuf, port: u16) -> Result<SocketAddr, Box<dyn std::error::Error>> {
    let catalog = lore_catalog::store::default_catalog_dir().ok_or("找不到目錄位置（請設定 LORESHELF_HOME）")?;
    let cache = lore_catalog::store::default_cache_dir().unwrap_or_else(|| std::env::temp_dir().join("loreshelf-cache"));
    let home = lore_home::Home::discover(&shelf_server::app_dir());
    let shared = shelf_server::AppState::new(home, web, catalog, cache, std::sync::Arc::new(shelf_server::SystemRunner));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            match shelf_server::bind(shared, port).await {
                Ok((addr, serve)) => {
                    let _ = tx.send(Ok(addr));
                    if let Err(e) = serve.await {
                        eprintln!("server stopped: {e}");
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        });
    });
    Ok(rx.recv()??)
}

/// Is the window covered by other windows (or off screen)? macOS only.
#[cfg(target_os = "macos")]
fn occluded(w: &WebviewWindow) -> bool {
    let Ok(p) = w.ns_window() else { return false };
    if p.is_null() {
        return false;
    }
    // SAFETY: a live NSWindow owned by this window, called on the main thread.
    let state: usize = unsafe { objc2::msg_send![&*(p as *mut objc2::runtime::AnyObject), occlusionState] };
    const VISIBLE: usize = 1 << 1; // NSWindowOcclusionStateVisible
    state & VISIBLE == 0
}

#[cfg(not(target_os = "macos"))]
fn occluded(_w: &WebviewWindow) -> bool {
    false
}

/// The page's activity now (main thread).
fn activity(w: &WebviewWindow) -> &'static str {
    let visible = w.is_visible().unwrap_or(true) && !w.is_minimized().unwrap_or(false) && !occluded(w);
    if !visible {
        "hidden"
    } else if w.app_handle().state::<Desk>().focused.load(Ordering::SeqCst) {
        "active"
    } else {
        "blurred"
    }
}

/// Tell the page when its activity changed.
fn report(w: &WebviewWindow) {
    let a = activity(w);
    let desk = w.app_handle().state::<Desk>();
    let mut last = desk.last.lock().unwrap();
    if *last != a {
        *last = a.to_string();
        let _ = w.eval(format!("window.LoreShelf && LoreShelf.setActivity({a:?})"));
    }
}

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

fn show(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        report(&w);
    }
}

fn hide_window(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        let _ = w.hide();
        report(&w);
    }
}

/// Dock click: hide when it is in front, otherwise bring it.
fn toggle(app: &AppHandle) {
    let Some(w) = main_window(app) else { return };
    if activity(&w) == "active" { hide_window(app) } else { show(app) }
}

/// The page's Esc: `window.__TAURI__.core.invoke("hide")`.
#[tauri::command]
fn hide(app: AppHandle) {
    hide_window(&app);
}

/// Where the window is on the screen, in points: `[x, y, w, h]` (the page seats LoreEcho in its
/// corner; a WebView does not know its own window's place).
#[tauri::command]
fn frame(app: AppHandle) -> Option<[f64; 4]> {
    let w = main_window(&app)?;
    let (pos, size, scale) = (w.outer_position().ok()?, w.outer_size().ok()?, w.scale_factor().ok()?);
    let (p, s) = (pos.to_logical::<f64>(scale), size.to_logical::<f64>(scale));
    Some([p.x, p.y, s.width, s.height])
}

/// The usual macOS menus: app (隱藏, 結束 ⌘Q), 編輯 (search box copy / paste), 視窗.
fn app_menu(app: &AppHandle) -> tauri::Result<()> {
    let quit = MenuItem::with_id(app, "app-quit", "結束 LoreShelf", true, Some("CmdOrCtrl+Q"))?;
    let appm = Submenu::with_items(
        app,
        "LoreShelf",
        true,
        &[
            &PredefinedMenuItem::hide(app, Some("隱藏 LoreShelf"))?,
            &PredefinedMenuItem::hide_others(app, Some("隱藏其他"))?,
            &PredefinedMenuItem::show_all(app, Some("全部顯示"))?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "編輯",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("復原"))?,
            &PredefinedMenuItem::redo(app, Some("重做"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("剪下"))?,
            &PredefinedMenuItem::copy(app, Some("拷貝"))?,
            &PredefinedMenuItem::paste(app, Some("貼上"))?,
            &PredefinedMenuItem::select_all(app, Some("全選"))?,
        ],
    )?;
    // custom items: the frameless window is minimised / hidden by us, not by AppKit
    let mini = MenuItem::with_id(app, "app-minimize", "最小化", true, Some("CmdOrCtrl+M"))?;
    let close = MenuItem::with_id(app, "app-hide-window", "收起", true, Some("CmdOrCtrl+W"))?;
    let front = MenuItem::with_id(app, "app-front", "顯示 LoreShelf", true, None::<&str>)?;
    let window = Submenu::with_items(app, "視窗", true, &[&mini, &close, &front])?;
    #[cfg(target_os = "macos")]
    let _ = window.set_as_windows_menu_for_nsapp();
    app.set_menu(Menu::with_items(app, &[&appm, &edit, &window])?)?;
    app.on_menu_event(|app, ev| match ev.id().as_ref() {
        "app-quit" => app.exit(0),
        "app-minimize" => {
            if let Some(w) = main_window(app) {
                let _ = w.minimize();
            }
        }
        "app-hide-window" => hide_window(app),
        "app-front" => show(app),
        _ => {}
    });
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(Desk::default())
        .invoke_handler(tauri::generate_handler![hide, frame])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Regular);

            // Bundled app: web/ ships as a resource. `cargo run`: the repository folder.
            let web = app
                .path()
                .resource_dir()
                .ok()
                .map(|d| d.join("web"))
                .filter(|w| w.join("index.html").exists())
                .unwrap_or_else(|| shelf_server::app_dir().join("web"));
            let port = std::env::var("LORESHELF_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(shelf_server::DEFAULT_PORT);
            let wanted = SocketAddr::from(([127, 0, 0, 1], port));
            let addr = if shelf_at(wanted) { wanted } else { start_server(web, port)? };
            lore_home::ports::register(shelf_server::PORT_NAME, addr.port());

            let state_file = lore_catalog::store::default_catalog_dir().map(|d| d.join(STATE_FILE));
            let r = opening_place(app.handle(), state_file.as_deref().and_then(load_place));
            *app.state::<Desk>().state_file.lock().unwrap() = state_file;
            let url = url::Url::parse(&format!("http://{addr}/"))?;
            let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("LoreShelf")
                .inner_size(r.w, r.h)
                .position(r.x, r.y)
                .min_inner_size(MIN_W, MIN_H)
                // the usual window buttons (close, minimise, full screen) over the page; no title bar
                .decorations(true)
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
                .transparent(true)
                .shadow(true)
                .resizable(true)
                .effects(WindowEffectsConfig { effects: vec![Effect::FullScreenUI], state: Some(EffectState::Active), radius: Some(CORNER), color: None, ..Default::default() })
                .build()?;
            app.state::<Desk>().focused.store(true, Ordering::SeqCst);
            let w2 = win.clone();
            win.on_window_event(move |ev| {
                match ev {
                    WindowEvent::Focused(f) => {
                        w2.app_handle().state::<Desk>().focused.store(*f, Ordering::SeqCst);
                        report(&w2);
                    }
                    WindowEvent::Moved(_) | WindowEvent::Resized(_) => moved(&w2),
                    // the red button closes for real: the app quits (the server and every picture with it)
                    WindowEvent::CloseRequested { .. } => w2.app_handle().exit(0),
                    _ => {}
                }
            });
            // Minimise, hide and occlusion have no event here: check once a second.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(POLL);
                    let h = handle.clone();
                    let ok = handle.run_on_main_thread(move || {
                        if let Some(w) = main_window(&h) {
                            report(&w);
                        }
                    });
                    if ok.is_err() {
                        break;
                    }
                }
            });
            app_menu(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building LoreShelf")
        .run(|app, event| match event {
            // a click on the Dock icon: show or hide the window
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => toggle(app),
            _ => {
                let _ = app;
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_place_and_fit() {
        let work = Rect { x: 0.0, y: 25.0, w: 1920.0, h: 1000.0 };
        let r = first_place(work);
        assert_eq!((r.w, r.h), (1632.0, 850.0));
        assert_eq!((r.x, r.y), (144.0, 100.0));
        assert!(r.y + r.h <= work.y + work.h, "never under the Dock");
        let f = fit(Rect { x: 1800.0, y: 900.0, w: 3000.0, h: 400.0 }, work);
        assert_eq!(f, Rect { x: 0.0, y: 425.0, w: 1920.0, h: 600.0 });
        let small = Rect { x: 0.0, y: 0.0, w: 800.0, h: 500.0 };
        assert_eq!(fit(Rect { x: 0.0, y: 0.0, w: 1000.0, h: 700.0 }, small), small, "a screen smaller than the minimum");
    }
}
