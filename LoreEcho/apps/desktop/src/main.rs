// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! LoreEcho's desktop shell: the local server runs in-process on a free port; one small
//! frameless, transparent, always-on-top window shows the page. Collapsed it is a 64 px
//! bubble, expanded a 620×620 window (the character's full body on the left, standing over
//! the dialogue panel on the right; the rest is transparent) whose bottom-right corner is
//! the bubble's (moved back on screen when it would not fit). The page drags the window itself (Tauri's
//! `startDragging`) and asks for the panel through `PUT /api/window`, which reaches this
//! file as a [`le_server::DesktopEvent`]. Moves are saved (debounced) as the bubble's
//! place in `private/LoreEcho/state.toml`. A tray / menu-bar item and a global shortcut
//! (settings.toml, default Alt+Space) show and hide the window.

use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use le_server::store::WindowPatch;
use le_server::{DesktopEvent, DesktopHook, Shared};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Monitor, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const BUBBLE: f64 = 64.0;
/// The open window: figure + dialogue panel. The page lays them out in this box; the same
/// numbers are `--win-w` / `--win-h` in `web/styles.css` (a test checks they agree).
const WIN_W: f64 = 620.0;
const WIN_H: f64 = 620.0;
/// Distance of the default bubble from the screen's corner.
const MARGIN: f64 = 24.0;
/// Moves within this time after the app placed the window itself are not the user's.
const QUIET: Duration = Duration::from_millis(400);
/// Save a dragged position once the window has rested this long.
const SAVE_AFTER: Duration = Duration::from_millis(400);

/// A rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// The bubble's default place: the bottom-right corner of the work area.
fn default_bubble(work: Rect) -> (f64, f64) {
    (work.x + work.w - BUBBLE - MARGIN, work.y + work.h - BUBBLE - MARGIN)
}

/// `r` moved (not resized) so that it lies inside `work` (top-left wins when it cannot).
fn keep_inside(r: Rect, work: Rect) -> Rect {
    let x = r.x.min(work.x + work.w - r.w).max(work.x);
    let y = r.y.min(work.y + work.h - r.h).max(work.y);
    Rect { x, y, ..r }
}

/// The open window grown from the bubble at `b`: same bottom-right corner, kept on screen.
fn panel_for(b: (f64, f64), work: Rect) -> Rect {
    keep_inside(Rect { x: b.0 + BUBBLE - WIN_W, y: b.1 + BUBBLE - WIN_H, w: WIN_W, h: WIN_H }, work)
}

/// Where the bubble goes when the open window at `p` collapses (its bottom-right corner).
fn bubble_from_panel(p: Rect) -> (f64, f64) {
    (p.x + p.w - BUBBLE, p.y + p.h - BUBBLE)
}

/// A monitor's work area in logical pixels.
fn work_of(m: &Monitor) -> Rect {
    let s = m.scale_factor();
    let wa = m.work_area();
    Rect { x: wa.position.x as f64 / s, y: wa.position.y as f64 / s, w: wa.size.width as f64 / s, h: wa.size.height as f64 / s }
}

#[derive(Default)]
struct Desk {
    shared: Mutex<Option<Shared>>,
    addr: Mutex<Option<SocketAddr>>,
    /// Until when moves are the app's own.
    quiet_until: Mutex<Option<Instant>>,
    /// Bumped by every user move; a save runs only if no newer move came.
    moves: AtomicU64,
    shortcut: Mutex<Option<Shortcut>>,
}

fn shared(app: &AppHandle) -> Option<Shared> {
    app.state::<Desk>().shared.lock().unwrap().clone()
}

/// The work area around the logical point `(x, y)`: the monitor that contains it, else the
/// primary one. `None` without any monitor information.
fn work_at(app: &AppHandle, at: Option<(f64, f64)>) -> Option<Rect> {
    let monitors = app.available_monitors().unwrap_or_default();
    if let Some((x, y)) = at
        && let Some(r) = monitors.iter().map(work_of).find(|r| r.contains(x + BUBBLE / 2.0, y + BUBBLE / 2.0))
    {
        return Some(r);
    }
    app.primary_monitor().ok().flatten().or_else(|| monitors.into_iter().next()).map(|m| work_of(&m))
}

/// The bubble's saved place, back on a screen (the default corner when there is none).
fn bubble_place(app: &AppHandle) -> (f64, f64) {
    let w = shared(app).map(|s| s.window()).unwrap_or_default();
    let saved = w.x.zip(w.y);
    let Some(work) = work_at(app, saved) else { return saved.unwrap_or((100.0, 100.0)) };
    match saved {
        Some(b) => {
            let r = keep_inside(Rect { x: b.0, y: b.1, w: BUBBLE, h: BUBBLE }, work);
            (r.x, r.y)
        }
        None => default_bubble(work),
    }
}

/// Size and place the window for the bubble or the panel.
fn place(win: &WebviewWindow, expanded: bool) {
    let app = win.app_handle();
    let b = bubble_place(app);
    let r = match (expanded, work_at(app, Some(b))) {
        (true, Some(work)) => panel_for(b, work),
        (true, None) => Rect { x: b.0 + BUBBLE - WIN_W, y: b.1 + BUBBLE - WIN_H, w: WIN_W, h: WIN_H },
        (false, _) => Rect { x: b.0, y: b.1, w: BUBBLE, h: BUBBLE },
    };
    *app.state::<Desk>().quiet_until.lock().unwrap() = Some(Instant::now() + QUIET);
    let _ = win.set_size(LogicalSize::new(r.w, r.h));
    let _ = win.set_position(LogicalPosition::new(r.x, r.y));
    if expanded {
        let _ = win.set_focus();
    }
}

/// The user moved the window: remember the bubble's place once it rests.
fn moved(win: &WebviewWindow) {
    let app = win.app_handle().clone();
    let desk = app.state::<Desk>();
    if desk.quiet_until.lock().unwrap().is_some_and(|t| Instant::now() < t) {
        return;
    }
    let n = desk.moves.fetch_add(1, Ordering::SeqCst) + 1;
    let win = win.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SAVE_AFTER).await;
        if app.state::<Desk>().moves.load(Ordering::SeqCst) != n {
            return;
        }
        let (Ok(pos), Ok(size), Ok(scale)) = (win.outer_position(), win.outer_size(), win.scale_factor()) else { return };
        let (p, s) = (pos.to_logical::<f64>(scale), size.to_logical::<f64>(scale));
        let r = Rect { x: p.x, y: p.y, w: s.width, h: s.height };
        let b = if r.w > BUBBLE * 1.5 { bubble_from_panel(r) } else { (r.x, r.y) };
        if let Some(sh) = shared(&app)
            && let Err(e) = sh.save_window(&WindowPatch { x: Some(b.0), y: Some(b.1), expanded: None })
        {
            eprintln!("loreecho: {e}");
        }
    });
}

fn toggle(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
    } else {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Register `keys` as the show / hide shortcut (replacing the old one). "" = none.
fn set_shortcut(app: &AppHandle, keys: &str) {
    let gs = app.global_shortcut();
    let desk = app.state::<Desk>();
    let mut cur = desk.shortcut.lock().unwrap();
    if let Some(old) = cur.take() {
        let _ = gs.unregister(old);
    }
    if keys.trim().is_empty() {
        return;
    }
    match keys.parse::<Shortcut>() {
        Ok(sc) => match gs.register(sc) {
            Ok(()) => *cur = Some(sc),
            Err(e) => eprintln!("loreecho: shortcut {keys}: {e}"),
        },
        Err(e) => eprintln!("loreecho: shortcut {keys}: {e}"),
    }
}

fn on_desktop_event(app: &AppHandle, e: DesktopEvent) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || match e {
        DesktopEvent::Expanded(on) => {
            if let Some(w) = app2.get_webview_window("main") {
                place(&w, on);
            }
        }
        DesktopEvent::Shortcut(keys) => set_shortcut(&app2, &keys),
    });
}

fn tray(app: &AppHandle) -> tauri::Result<()> {
    let toggle_item = MenuItem::with_id(app, "toggle", "顯示／隱藏 LoreEcho", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "結束 LoreEcho", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle_item, &quit])?;
    let mut b = TrayIconBuilder::with_id("main")
        .tooltip("LoreEcho")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "toggle" => toggle(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                toggle(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.build(app)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _sc, ev| {
                    if ev.state == ShortcutState::Pressed {
                        toggle(app);
                    }
                })
                .build(),
        )
        .manage(Desk::default())
        .setup(|app| {
            // a menu-bar companion, not a Dock app
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let mut config = le_server::Config::load(&[])?;
            config.port = 0;
            // Bundled app: web/ ships as a resource. `cargo run`: the repository folder.
            if let Some(web) = app.path().resource_dir().ok().map(|d| d.join("web")).filter(|w| w.join("index.html").exists()) {
                config.web = web;
            }
            let handle = app.handle().clone();
            let hook: DesktopHook = std::sync::Arc::new(move |e| on_desktop_event(&handle, e));
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                rt.block_on(async move {
                    match le_server::bind(config, Some(hook)).await {
                        Ok((addr, shared, serve)) => {
                            let _ = tx.send(Ok((addr, shared)));
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
            let (addr, shared) = rx.recv()??;
            lore_home::ports::register(le_server::config::SHORT, addr.port());
            let state = shared.window();
            let keys = shared.settings().shortcut;
            {
                let desk = app.state::<Desk>();
                *desk.shared.lock().unwrap() = Some(shared);
                *desk.addr.lock().unwrap() = Some(addr);
            }

            let url = url::Url::parse(&format!("http://{addr}/"))?;
            let b = bubble_place(app.handle());
            let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("LoreEcho")
                .inner_size(BUBBLE, BUBBLE)
                .position(b.0, b.1)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .visible_on_all_workspaces(true)
                .skip_taskbar(true)
                .resizable(false)
                .build()?;
            if state.expanded {
                place(&win, true);
            }
            let w2 = win.clone();
            win.on_window_event(move |ev| {
                if let WindowEvent::Moved(_) = ev {
                    moved(&w2);
                }
            });
            tray(app.handle())?;
            set_shortcut(app.handle(), &keys);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building LoreEcho")
        .run(|_, event| {
            // quitting: this app's hold on the shared local model goes (the last user stops it)
            if let tauri::RunEvent::Exit = event {
                le_server::llm::quit();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0.0, y: 0.0, w: 1440.0, h: 875.0 };

    #[test]
    fn default_place_is_the_bottom_right_corner() {
        assert_eq!(default_bubble(SCREEN), (1440.0 - 64.0 - 24.0, 875.0 - 64.0 - 24.0));
    }

    #[test]
    fn the_panel_grows_from_the_bubble_and_stays_on_screen() {
        let b = default_bubble(SCREEN);
        let p = panel_for(b, SCREEN);
        assert_eq!(bubble_from_panel(p), b, "same bottom-right corner");
        // a bubble at the top-left: the panel is pushed back on screen
        let p = panel_for((10.0, 10.0), SCREEN);
        assert_eq!((p.x, p.y, p.w, p.h), (0.0, 0.0, WIN_W, WIN_H));
        // a second monitor to the right
        let right = Rect { x: 1440.0, y: 0.0, w: 1920.0, h: 1080.0 };
        let p = panel_for((1450.0, 500.0), right);
        assert!(p.x >= right.x && p.y + p.h <= right.y + right.h);
    }

    #[test]
    fn the_wide_window_keeps_the_corner_when_it_fits() {
        // a bubble in the middle of the screen: no push, the corner is shared exactly
        let b = (900.0, 700.0);
        let p = panel_for(b, SCREEN);
        assert_eq!((p.x, p.y, p.w, p.h), (900.0 + 64.0 - WIN_W, 700.0 + 64.0 - WIN_H, WIN_W, WIN_H));
        // a bubble near the left edge: pushed right only, bottom edge still the bubble's
        let p = panel_for((100.0, 700.0), SCREEN);
        assert_eq!((p.x, p.y + p.h), (0.0, 700.0 + 64.0));
        // near the top: pushed down only
        let p = panel_for((900.0, 40.0), SCREEN);
        assert_eq!((p.x + p.w, p.y), (900.0 + 64.0, 0.0));
        // a work area with a menu bar and a Dock: stays inside it
        let work = Rect { x: 0.0, y: 25.0, w: 1440.0, h: 800.0 };
        let p = panel_for(default_bubble(work), work);
        assert!(p.x >= work.x && p.y >= work.y && p.x + p.w <= work.x + work.w && p.y + p.h <= work.y + work.h);
        assert_eq!(bubble_from_panel(p), default_bubble(work));
    }

    #[test]
    fn the_page_uses_the_same_window_size() {
        let css = include_str!("../../../web/styles.css");
        assert!(css.contains(&format!("--win-w: {}px", WIN_W)) && css.contains(&format!("--win-h: {}px", WIN_H)), "web/styles.css --win-w / --win-h");
    }

    #[test]
    fn keep_inside_moves_only() {
        let r = keep_inside(Rect { x: 2000.0, y: -50.0, w: 64.0, h: 64.0 }, SCREEN);
        assert_eq!(r, Rect { x: 1440.0 - 64.0, y: 0.0, w: 64.0, h: 64.0 });
    }

    #[test]
    fn capability_and_config() {
        let cap = include_str!("../capabilities/default.json");
        assert!(cap.contains("core:window:allow-start-dragging") && cap.contains("http://127.0.0.1:*"));
        let conf = include_str!("../tauri.conf.json");
        assert!(conf.contains("\"macOSPrivateApi\": true") && conf.contains("\"withGlobalTauri\": true"));
    }
}
