//! The system permissions an app needs (microphone, speech recognition, Personal Voice),
//! asked all at once the first time the desktop app opens; afterwards they change only in
//! 設定 → 一般 → 權限 (or System Settings).
//!
//! - [`Permission`], [`Status`]: the shared words (`granted | denied | not_determined | restricted |
//!   unsupported`).
//! - [`Provider`]: what can report and ask for some of them — lore-voice's `Voice` (microphone,
//!   speech, Personal Voice); fakes in tests.
//! - [`Permissions`]: the app's declared list over its providers. [`Permissions::request`] asks every
//!   `not_determined` one, one after another in the declared order, and never one that was
//!   answered (granted, denied, restricted): the system remembers the answer, so only the first
//!   launch shows prompts and no state file is needed.
//! - Launch: the live router registers its set ([`Permissions::register`]); the desktop app's
//!   `main` calls [`ask_at_launch`] once its window is up — a background thread, once per process.
//!   Tests, isolated routers and the plain dev server never call it.
//! - `web` (feature): `GET /api/permissions`, `POST /api/permissions/request`,
//!   `POST /api/permissions/open` ([`router`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// One system permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Microphone,
    Speech,
    PersonalVoice,
}

impl Permission {
    /// The order prompts come in.
    pub const ALL: [Permission; 3] = [Permission::Microphone, Permission::Speech, Permission::PersonalVoice];

    pub fn id(self) -> &'static str {
        match self {
            Permission::Microphone => "microphone",
            Permission::Speech => "speech",
            Permission::PersonalVoice => "personal_voice",
        }
    }

    /// `microphone`, `speech`, `personal_voice` (or `personal-voice`).
    pub fn parse(s: &str) -> Option<Permission> {
        match s {
            "microphone" => Some(Permission::Microphone),
            "speech" => Some(Permission::Speech),
            "personal_voice" | "personal-voice" => Some(Permission::PersonalVoice),
            _ => None,
        }
    }

    /// The name in 設定 → 一般 → 權限.
    pub fn label(self) -> &'static str {
        match self {
            Permission::Microphone => "麥克風",
            Permission::Speech => "語音辨識",
            Permission::PersonalVoice => "個人聲音",
        }
    }

    /// The System Settings page where the author changes it.
    pub fn settings_url(self) -> &'static str {
        match self {
            Permission::Microphone => "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
            Permission::Speech => "x-apple.systempreferences:com.apple.preference.security?Privacy_SpeechRecognition",
            Permission::PersonalVoice => "x-apple.systempreferences:com.apple.Accessibility-Settings.extension?PersonalVoice",
        }
    }
}

/// Where a permission stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Granted,
    Denied,
    /// Never asked: the system's prompt comes the next time it is requested.
    NotDetermined,
    /// Not the author's to give (parental controls, a device policy).
    Restricted,
    /// Not on this Mac (older macOS, no helper, not macOS).
    Unsupported,
}

impl Status {
    /// The helpers' words (also the older `authorized` / `undetermined`).
    pub fn from_word(s: &str) -> Status {
        match s {
            "granted" | "authorized" => Status::Granted,
            "denied" => Status::Denied,
            "not_determined" | "undetermined" | "notDetermined" => Status::NotDetermined,
            "restricted" => Status::Restricted,
            _ => Status::Unsupported,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Status::Granted => "granted",
            Status::Denied => "denied",
            Status::NotDetermined => "not_determined",
            Status::Restricted => "restricted",
            Status::Unsupported => "unsupported",
        }
    }
}

/// Something that knows some permissions (a helper behind lore-voice, a fake).
pub trait Provider: Send + Sync + 'static {
    /// The permissions it reports.
    fn covers(&self) -> Vec<Permission>;
    /// False: no engine here (no helper, not macOS) — its permissions are `unsupported` and nothing is asked.
    fn available(&self) -> bool;
    /// The current state; asks nothing.
    fn status(&self, p: Permission) -> Result<Status, String>;
    /// Show the system's prompt when `p` was never asked (blocks until the author answers), else
    /// just the state. Never asks again once answered.
    fn request(&self, p: Permission) -> Result<Status, String>;
}

/// One row of 設定 → 一般 → 權限.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Row {
    pub id: Permission,
    pub label: &'static str,
    pub status: Status,
}

type Opener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

struct Inner {
    declared: Vec<Permission>,
    providers: Vec<Arc<dyn Provider>>,
    /// The real app: System Settings may be opened and the launch may ask.
    live: bool,
    opener: Opener,
    /// One request at a time (the launch thread and 詢問 may overlap): never two prompts at once.
    asking: Mutex<()>,
}

/// The permissions one app declared, over its providers. Cheap to clone.
#[derive(Clone)]
pub struct Permissions {
    inner: Arc<Inner>,
}

impl Permissions {
    /// The app needs `declared` (asked in [`Permission::ALL`] order whatever the order here).
    pub fn new(declared: &[Permission]) -> Permissions {
        let mut d: Vec<Permission> = Permission::ALL.into_iter().filter(|p| declared.contains(p)).collect();
        d.dedup();
        Permissions { inner: Arc::new(Inner { declared: d, providers: vec![], live: true, opener: Arc::new(open_system), asking: Mutex::new(()) }) }
    }

    /// An app that needs none (its settings show no 權限 block).
    pub fn none() -> Permissions {
        Permissions::new(&[])
    }

    fn rebuild(self, f: impl FnOnce(&mut Inner)) -> Permissions {
        let mut inner = Arc::try_unwrap(self.inner).unwrap_or_else(|_| panic!("Permissions: build before sharing"));
        f(&mut inner);
        Permissions { inner: Arc::new(inner) }
    }

    /// Add a provider (the first one that covers a permission answers for it).
    pub fn with(self, p: Arc<dyn Provider>) -> Permissions {
        self.rebuild(|i| i.providers.push(p))
    }

    /// Tests and isolated routers: never opens System Settings, never asks at launch.
    pub fn isolated(self) -> Permissions {
        self.rebuild(|i| i.live = false)
    }

    /// How System Settings is opened (tests record the URL).
    pub fn with_opener(self, f: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static) -> Permissions {
        self.rebuild(|i| i.opener = Arc::new(f))
    }

    pub fn declared(&self) -> &[Permission] {
        &self.inner.declared
    }

    pub fn live(&self) -> bool {
        self.inner.live
    }

    fn provider(&self, p: Permission) -> Option<&Arc<dyn Provider>> {
        self.inner.providers.iter().find(|x| x.covers().contains(&p))
    }

    fn state(&self, p: Permission) -> Status {
        match self.provider(p) {
            Some(x) if x.available() => x.status(p).unwrap_or(Status::Unsupported),
            _ => Status::Unsupported,
        }
    }

    /// Every declared permission and where it stands (asks nothing).
    pub fn statuses(&self) -> Vec<Row> {
        self.inner.declared.iter().map(|&p| Row { id: p, label: p.label(), status: self.state(p) }).collect()
    }

    /// Ask every declared (or only `only`) permission that was never asked, one after another;
    /// answered ones are left alone. A failure stops the sequence (no prompt stacked on an
    /// unanswered one). Then the rows.
    pub fn request(&self, only: Option<Permission>) -> Vec<Row> {
        {
            let _one = self.inner.asking.lock().unwrap_or_else(|e| e.into_inner());
            for &p in &self.inner.declared {
                if only.is_some_and(|o| o != p) {
                    continue;
                }
                let Some(x) = self.provider(p).filter(|x| x.available()) else { continue };
                // read again just before asking: something else may have asked meanwhile
                match x.status(p) {
                    Ok(Status::NotDetermined) => {}
                    Ok(_) => continue,
                    Err(_) => break,
                }
                if x.request(p).is_err() {
                    break;
                }
            }
        }
        self.statuses()
    }

    /// Open the System Settings page of `p` (only in the real app); the URL either way.
    pub fn open(&self, p: Permission) -> Result<(bool, &'static str), String> {
        let url = p.settings_url();
        if !self.inner.live {
            return Ok((false, url));
        }
        (self.inner.opener)(url).map(|_| (true, url))
    }

    /// Whether the launch would ask anything, and the thread that does (`desktop`: this process
    /// is the desktop app).
    pub fn launch(&self, desktop: bool) -> Launch {
        if !desktop {
            return Launch::Skipped("not the desktop app");
        }
        if !self.inner.live {
            return Launch::Skipped("isolated");
        }
        if self.inner.declared.is_empty() {
            return Launch::Skipped("nothing declared");
        }
        if !self.inner.declared.iter().any(|&p| self.provider(p).is_some_and(|x| x.available())) {
            return Launch::Skipped("no helper");
        }
        let me = self.clone();
        match std::thread::Builder::new().name("lore-permissions".into()).spawn(move || me.request(None)) {
            Ok(h) => Launch::Started(h),
            Err(_) => Launch::Skipped("no thread"),
        }
    }

    /// This set is the one [`ask_at_launch`] asks (the live router of the app calls it).
    pub fn register(&self) {
        if self.inner.live && !self.inner.declared.is_empty() {
            *REGISTERED.lock().unwrap_or_else(|e| e.into_inner()) = Some(self.clone());
        }
    }
}

/// What [`Permissions::launch`] did.
pub enum Launch {
    /// Asking in the background; the rows when it is done.
    Started(std::thread::JoinHandle<Vec<Row>>),
    Skipped(&'static str),
}

static REGISTERED: Mutex<Option<Permissions>> = Mutex::new(None);
static ASKED: AtomicBool = AtomicBool::new(false);

/// The desktop app opened: ask every permission the app registered that was never asked, in a
/// background thread (the window does not wait). Once per process; does nothing when no set was
/// registered (tests, isolated, no helper).
pub fn ask_at_launch() {
    if ASKED.swap(true, Ordering::SeqCst) {
        return;
    }
    let set = REGISTERED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(p) = set {
        // even the checks (a helper's probe) stay off the caller's thread
        let _ = std::thread::Builder::new().name("lore-permissions-launch".into()).spawn(move || match p.launch(true) {
            Launch::Started(h) => {
                let _ = h.join();
            }
            Launch::Skipped(why) => eprintln!("permissions: not asked at launch ({why})"),
        });
    }
}

/// `/usr/bin/open <url>` on macOS; elsewhere an error.
fn open_system(url: &str) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("只有 macOS 有系統設定".into());
    }
    match std::process::Command::new("/usr/bin/open").arg(url).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("open: {s}")),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(feature = "web")]
pub use web::router;

#[cfg(feature = "web")]
mod web {
    //! | method | path | what |
    //! |---|---|---|
    //! | GET | /api/permissions | `{ permissions: [{ id, label, status }] }` (empty: the app declared none) |
    //! | POST | /api/permissions/request | `{ what? }`: ask the never-asked ones (or only `what`), then the same as GET |
    //! | POST | /api/permissions/open | `{ what }`: System Settings at that page → `{ opened, url }` |

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::{Value, json};

    use super::{Permission, Permissions, Row};

    /// `/api/permissions*` over `p`; a live set is also the one asked at launch.
    pub fn router<S: Clone + Send + Sync + 'static>(p: Permissions) -> Router<S> {
        p.register();
        Router::new()
            .route("/api/permissions", get(list))
            .route("/api/permissions/request", post(request))
            .route("/api/permissions/open", post(open))
            .with_state(p)
    }

    fn bad(status: StatusCode, msg: &str) -> Response {
        (status, Json(json!({ "error": msg }))).into_response()
    }

    fn rows(r: Vec<Row>) -> Response {
        Json(json!({ "permissions": r })).into_response()
    }

    fn what(body: &Option<Json<Value>>) -> Option<&str> {
        body.as_ref().and_then(|Json(b)| b["what"].as_str())
    }

    async fn list(State(p): State<Permissions>) -> Response {
        tokio::task::spawn_blocking(move || rows(p.statuses())).await.unwrap_or_else(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))
    }

    async fn request(State(p): State<Permissions>, body: Option<Json<Value>>) -> Response {
        let only = match what(&body) {
            None | Some("") => None,
            Some(w) => match Permission::parse(w).filter(|x| p.declared().contains(x)) {
                Some(x) => Some(x),
                None => return bad(StatusCode::BAD_REQUEST, "沒有這個權限"),
            },
        };
        // waits while the system's prompt is open
        tokio::task::spawn_blocking(move || rows(p.request(only))).await.unwrap_or_else(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))
    }

    async fn open(State(p): State<Permissions>, body: Option<Json<Value>>) -> Response {
        let Some(x) = what(&body).and_then(Permission::parse) else { return bad(StatusCode::BAD_REQUEST, "what = microphone | speech | personal_voice") };
        match p.open(x) {
            Ok((opened, url)) => Json(json!({ "opened": opened, "url": url })).into_response(),
            Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, &e),
        }
    }
}

/// A provider for tests: scripted states and answers, a shared log of every prompt shown.
pub mod fake {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::{Permission, Provider, Status};

    /// Every prompt shown, across fakes (to check the order between providers).
    pub type Log = Arc<Mutex<Vec<Permission>>>;

    pub struct FakeProvider {
        /// permission → (now, what the prompt answers)
        state: Mutex<HashMap<Permission, (Status, Status)>>,
        available: bool,
        log: Log,
        fail: Mutex<Option<Permission>>,
    }

    impl FakeProvider {
        pub fn new(log: Log, perms: &[(Permission, Status, Status)]) -> FakeProvider {
            FakeProvider { state: Mutex::new(perms.iter().map(|&(p, n, a)| (p, (n, a))).collect()), available: true, log, fail: Mutex::new(None) }
        }

        pub fn unavailable(log: Log, perms: &[Permission]) -> FakeProvider {
            FakeProvider { available: false, ..FakeProvider::new(log, &perms.iter().map(|&p| (p, Status::NotDetermined, Status::Granted)).collect::<Vec<_>>()) }
        }

        /// Asking for `p` fails (the helper stopped answering).
        pub fn fail_on(&self, p: Permission) {
            *self.fail.lock().unwrap() = Some(p);
        }

        /// Change a state from outside (System Settings).
        pub fn set(&self, p: Permission, now: Status) {
            if let Some(e) = self.state.lock().unwrap().get_mut(&p) {
                e.0 = now;
            }
        }
    }

    impl Provider for FakeProvider {
        fn covers(&self) -> Vec<Permission> {
            let s = self.state.lock().unwrap();
            Permission::ALL.into_iter().filter(|p| s.contains_key(p)).collect()
        }
        fn available(&self) -> bool {
            self.available
        }
        fn status(&self, p: Permission) -> Result<Status, String> {
            Ok(self.state.lock().unwrap().get(&p).map(|e| e.0).unwrap_or(Status::Unsupported))
        }
        fn request(&self, p: Permission) -> Result<Status, String> {
            if *self.fail.lock().unwrap() == Some(p) {
                return Err("fake: no answer".into());
            }
            let mut s = self.state.lock().unwrap();
            let Some(e) = s.get_mut(&p) else { return Ok(Status::Unsupported) };
            if e.0 == Status::NotDetermined {
                self.log.lock().unwrap().push(p);
                e.0 = e.1;
            }
            Ok(e.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeProvider, Log};
    use super::*;
    use Permission::*;
    use Status::*;

    fn voice(log: &Log, mic: Status, speech: Status, pv: Status) -> Arc<FakeProvider> {
        Arc::new(FakeProvider::new(log.clone(), &[(Microphone, mic, Granted), (Speech, speech, Denied), (PersonalVoice, pv, Granted)]))
    }

    #[test]
    fn words_map_to_states() {
        for (w, s) in [("granted", Granted), ("authorized", Granted), ("denied", Denied), ("not_determined", NotDetermined), ("undetermined", NotDetermined), ("restricted", Restricted), ("unsupported", Unsupported), ("whatever", Unsupported)] {
            assert_eq!(Status::from_word(w), s, "{w}");
        }
        for s in [Granted, Denied, NotDetermined, Restricted, Unsupported] {
            assert_eq!(Status::from_word(s.word()), s);
            assert_eq!(serde_json::to_value(s).unwrap(), serde_json::json!(s.word()));
        }
        for p in Permission::ALL {
            assert_eq!(Permission::parse(p.id()), Some(p));
            assert!(p.settings_url().starts_with("x-apple.systempreferences:"));
        }
        assert_eq!(Permission::parse("personal-voice"), Some(PersonalVoice));
        assert_eq!([Microphone, Speech, PersonalVoice].map(|p| p.label()), ["麥克風", "語音辨識", "個人聲音"]);
    }

    #[test]
    fn asks_only_the_never_asked_ones_in_order_and_never_again() {
        let log: Log = Log::default();
        // declared out of order: asked in the fixed order anyway
        let p = Permissions::new(&[PersonalVoice, Speech, Microphone]).with(voice(&log, NotDetermined, Granted, NotDetermined)).isolated();
        assert_eq!(p.declared(), [Microphone, Speech, PersonalVoice]);
        let rows = p.request(None);
        assert_eq!(*log.lock().unwrap(), [Microphone, PersonalVoice], "speech was answered already");
        assert_eq!(rows.iter().map(|r| r.status).collect::<Vec<_>>(), [Granted, Granted, Granted]);
        // a second time (the next launch): nothing is asked
        p.request(None);
        assert_eq!(log.lock().unwrap().len(), 2);
    }

    #[test]
    fn one_permission_and_failures() {
        let log: Log = Log::default();
        let v = voice(&log, NotDetermined, NotDetermined, NotDetermined);
        let p = Permissions::new(&[Microphone, Speech, PersonalVoice]).with(v.clone()).isolated();
        let rows = p.request(Some(Speech));
        assert_eq!(*log.lock().unwrap(), [Speech]);
        assert_eq!(rows[1].status, Denied);
        // a prompt that gets no answer stops the rest
        v.fail_on(Microphone);
        p.request(None);
        assert_eq!(*log.lock().unwrap(), [Speech]);
    }

    #[test]
    fn missing_or_unavailable_providers_are_unsupported() {
        let log: Log = Log::default();
        let p = Permissions::new(&[Microphone, Speech]).with(Arc::new(FakeProvider::unavailable(log.clone(), &[Microphone]))).isolated();
        assert_eq!(p.statuses().iter().map(|r| r.status).collect::<Vec<_>>(), [Unsupported, Unsupported]);
        p.request(None);
        assert!(log.lock().unwrap().is_empty());
        assert!(Permissions::none().statuses().is_empty());
    }

    #[test]
    fn the_launch_asks_only_in_the_live_desktop_app() {
        let log: Log = Log::default();
        let set = |live: bool| {
            let p = Permissions::new(&[Microphone, PersonalVoice]).with(voice(&log, NotDetermined, Granted, NotDetermined));
            if live { p.with_opener(|_| Ok(())) } else { p.isolated() }
        };
        assert!(matches!(set(true).launch(false), Launch::Skipped(_)), "tests and the dev server");
        assert!(matches!(set(false).launch(true), Launch::Skipped("isolated")));
        assert!(matches!(Permissions::none().launch(true), Launch::Skipped(_)));
        let off = Permissions::new(&[Microphone]).with(Arc::new(FakeProvider::unavailable(log.clone(), &[Microphone])));
        assert!(matches!(off.launch(true), Launch::Skipped("no helper")));
        assert!(log.lock().unwrap().is_empty());
        let Launch::Started(h) = set(true).launch(true) else { panic!("should ask") };
        let rows = h.join().unwrap();
        assert_eq!(*log.lock().unwrap(), [Microphone, PersonalVoice]);
        assert!(rows.iter().all(|r| r.status == Granted));
    }

    #[test]
    fn settings_open_only_in_the_live_app() {
        let opened = Arc::new(Mutex::new(vec![]));
        let o = opened.clone();
        let p = Permissions::new(&[Microphone]).with_opener(move |u| {
            o.lock().unwrap().push(u.to_string());
            Ok(())
        });
        assert_eq!(p.open(Microphone), Ok((true, "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")));
        assert_eq!(opened.lock().unwrap().len(), 1);
        assert_eq!(Permissions::new(&[Microphone]).isolated().open(Microphone).unwrap().0, false);
    }

    #[cfg(feature = "web")]
    #[tokio::test]
    async fn routes() {
        use axum::Router;
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use http_body_util::BodyExt;
        use serde_json::{Value, json};
        use tower::ServiceExt;

        async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
            let mut req = Request::builder().method(method).uri(uri);
            let body = match body {
                Some(v) => {
                    req = req.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
            let st = res.status();
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
        }

        let log: Log = Log::default();
        let app: Router = router(Permissions::new(&[Microphone, Speech, PersonalVoice]).with(voice(&log, NotDetermined, NotDetermined, Denied)).isolated());
        let (st, v) = call(&app, "GET", "/api/permissions", None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["permissions"], json!([
            { "id": "microphone", "label": "麥克風", "status": "not_determined" },
            { "id": "speech", "label": "語音辨識", "status": "not_determined" },
            { "id": "personal_voice", "label": "個人聲音", "status": "denied" },
        ]));
        let (_, v) = call(&app, "POST", "/api/permissions/request", Some(json!({ "what": "microphone" }))).await;
        assert_eq!(v["permissions"][0]["status"], "granted");
        assert_eq!(v["permissions"][1]["status"], "not_determined");
        let (_, v) = call(&app, "POST", "/api/permissions/request", None).await;
        assert_eq!(v["permissions"][1]["status"], "denied");
        assert_eq!(*log.lock().unwrap(), [Microphone, Speech], "Personal Voice was denied: never asked");
        assert_eq!(call(&app, "POST", "/api/permissions/request", Some(json!({ "what": "screen" }))).await.0, StatusCode::BAD_REQUEST, "unknown");
        let (st, v) = call(&app, "POST", "/api/permissions/open", Some(json!({ "what": "personal-voice" }))).await;
        assert_eq!((st, v["opened"].clone()), (StatusCode::OK, json!(false)));
        assert!(v["url"].as_str().unwrap().contains("PersonalVoice"));
        assert_eq!(call(&app, "POST", "/api/permissions/open", Some(json!({ "what": "x" }))).await.0, StatusCode::BAD_REQUEST);
        // an app that declared nothing: an empty list (the block is not shown)
        let none: Router = router(Permissions::none().isolated());
        assert_eq!(call(&none, "GET", "/api/permissions", None).await.1["permissions"], json!([]));
    }
}
