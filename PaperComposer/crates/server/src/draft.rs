//! 討論草稿 routes: `/api/papers/{name}/draft/...`.
//!
//! The session is plain JSON in the paper folder (pc_core::draft::session).
//! Model calls run without holding the lock; the session is re-read after the
//! call and the result applied to the fresh copy, so a slow model never
//! blocks the window or loses an edit made meanwhile.
//!
//! Each step runs on Claude when it is available and on the local model
//! otherwise ([`engine`]); the engine that ran is kept in the session and
//! shown next to the step.

pub(crate) mod engine;
mod topaper;

use super::*;
use pc_core::draft::candidates::{self as ca, Prep};
use pc_core::draft::discuss as dz;
use pc_core::draft::lens::Lens;
use pc_core::draft::pending::{self as pd, Direction, PendingStatus};
use pc_core::draft::material::{self, Material};
use pc_core::draft::prompts as dp;
use pc_core::draft::revise::{self as rv, Revision, RevisionStatus};
use pc_core::draft::session::{self as ds, Phase, Session, Who};
use pc_core::draft::style_text;
use pc_core::draft::venue;

/// Serialises read-modify-write of draft sessions.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());


pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/papers/{name}/draft", get(get_draft))
        .route("/api/papers/{name}/draft/topic", put(put_topic))
        .route("/api/papers/{name}/draft/records", post(upload_record))
        .route("/api/papers/{name}/draft/records/text", post(record_from_text))
        .route("/api/papers/{name}/draft/records/{file}", axum::routing::delete(delete_record))
        .route("/api/papers/{name}/draft/record", post(run_record))
        .route("/api/papers/{name}/draft/experiments", put(put_experiments))
        .route("/api/papers/{name}/draft/plan", post(run_plan).put(put_plan))
        .route("/api/papers/{name}/draft/turn", post(run_turn))
        .route("/api/papers/{name}/draft/group/{id}", post(group_action))
        .route("/api/papers/{name}/draft/items/{id}/exclude", post(exclude_item))
        .route("/api/papers/{name}/draft/figures", post(run_figures).put(put_figures))
        .route("/api/papers/{name}/draft/notes", put(put_notes))
        .route("/api/papers/{name}/draft/notes/keep", post(keep_notes))
        .route("/api/papers/{name}/draft/discuss", post(discuss_open))
        .route("/api/papers/{name}/draft/discuss/{id}", post(discuss_say))
        .route("/api/papers/{name}/draft/discuss/{id}/answer", post(discuss_answer))
        .route("/api/papers/{name}/draft/discuss/{id}/close", post(discuss_close))
        .route("/api/papers/{name}/draft/claims", post(run_claims).put(put_claims))
        .route("/api/papers/{name}/draft/review", post(run_review).put(put_review))
        .route("/api/papers/{name}/draft/outline", post(run_outline).put(put_outline))
        .route("/api/papers/{name}/draft/compose", post(run_compose))
        .route("/api/papers/{name}/draft/insert", post(insert_drafts))
        .route("/api/papers/{name}/draft/audit", post(run_audit).put(put_audit))
        .route("/api/papers/{name}/draft/audit/{id}/ask", post(audit_ask))
        .route("/api/papers/{name}/draft/phase", put(put_phase))
        .route("/api/papers/{name}/draft/restart", post(restart))
        .route("/api/papers/{name}/draft/rewind", post(rewind))
        .route("/api/papers/{name}/draft/templates", get(template_choices))
        .route("/api/papers/{name}/draft/template", put(put_template))
        .route("/api/papers/{name}/draft/floats", post(add_floats))
        .route("/api/papers/{name}/draft/tables/{id}/insert", post(insert_table))
        .route("/api/papers/{name}/draft/revisions", post(new_revision))
        .route("/api/papers/{name}/draft/notation/register", post(register_symbols))
        .route("/api/papers/{name}/draft/revisions/{id}/plan", post(plan_revision))
        .route("/api/papers/{name}/draft/revisions/{id}/changes/{cid}", post(decide_change))
        .route("/api/papers/{name}/draft/revisions/{id}/status", post(revision_status))
        .route("/api/papers/{name}/draft/pending", get(pending_list).put(put_pending))
        .route("/api/papers/{name}/draft/pending/{id}/fill", post(fill_pending))
}

type Resp = axum::response::Response;

struct Ctx {
    dir: PathBuf,
    ws: PathBuf,
    key_value: String,
}

async fn ctx(s: &Shared, name: &str) -> Result<Ctx, Resp> {
    let dir = paper_dir(s, name).await?;
    let ws = s.loaded.read().await.workspace.clone().ok_or_else(|| bad("no workspace"))?;
    let key_value = meta::read_meta(&dir).key_value;
    Ok(Ctx { dir, ws, key_value })
}

/// The paper's tables, figures and references, and the same as prompt text.
/// …plus the author's notation registry and its rules, so every step writes
/// with the registered macros and labels.
fn material_of(c: &Ctx) -> (Material, String) {
    let m = material::load(&c.ws, &c.dir);
    let text = material::describe(&m) + &crate::notation::describe(&crate::notation::load(&c.ws, &c.dir), crate::notation::ref_words(&c.ws, &c.dir).as_ref());
    (m, text)
}

fn ref_keys(m: &Material) -> Vec<String> {
    m.refs.iter().map(|r| r.key.clone()).collect()
}

fn load(dir: &Path) -> Result<Session, Resp> {
    let mut s = Session::load(dir).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e).into_response())?;
    settle_stale_prep(dir, &mut s);
    // a figure plan cut short (the app quit) is not running any more
    if let Some(f) = s.figures.as_mut().filter(|f| f.running) {
        let live = FIGURING.lock().map(|v| v.contains(&dir.display().to_string())).unwrap_or(true);
        if !live {
            f.running = false;
            f.failed = "上次沒有做完（App 關掉了）".into();
        }
    }
    Ok(s)
}

fn save(dir: &Path, sess: &Session) -> Result<(), Resp> {
    sess.save(dir).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e).into_response())
}

/// The session's lens: its own choice, else `mine` / the first lens in the
/// workspace, else the built-in one.
fn lens_for(ws: &Path, sess: &Session) -> Result<Lens, Resp> {
    let avail = Lens::available(ws);
    let name = if !sess.lens.is_empty() {
        sess.lens.clone()
    } else if avail.iter().any(|n| n == "mine") {
        "mine".into()
    } else {
        avail.first().cloned().unwrap_or_else(|| "default".into())
    };
    Lens::load(ws, &name).map_err(|e| bad(&e))
}

/// Everything the window shows.
fn view(c: &Ctx, sess: &Session) -> Result<Resp, Resp> {
    let lens = lens_for(&c.ws, sess)?;
    let states: Vec<_> = sess.groups.iter().map(|g| sess.group_state(&lens, &g.id)).collect();
    Ok(Json(serde_json::json!({
        "key_value": c.key_value,
        // the journal template (step 1 picks it; 期刊寫法 and the requirements follow it)
        "journal": meta::read_meta(&c.dir).journal,
        "session": sess,
        "turns": ds::read_turns(&c.dir),
        "lens": {
            "name": lens.name,
            "available": Lens::available(&c.ws),
            "scale": lens.scale,
            "modes": lens.modes,
            "flags": lens.flags,
            "max_turns": lens.stop.max_turns_per_group,
        },
        "states": states,
        "has_style": !style_text(&c.ws).trim().is_empty(),
        // 待補清單: the paper's marks (and the draft's, before it is copied in)
        "pending_report": pending_report(c, sess),
        "material": material::load(&c.ws, &c.dir),
        // 圖表規劃's stand-ins already in the paper (the plan offers 用流程圖做／用繪圖做 on them)
        "stand_ins": pc_core::draft::figures::stand_ins(&c.dir),
    }))
    .into_response())
}

fn pending_report(c: &Ctx, sess: &Session) -> Vec<pd::ReportEntry> {
    let mut all = pd::locate(&c.dir);
    // the session's draft counts only until it is in the paper, and never once filled
    let in_paper: std::collections::BTreeSet<String> = all.iter().map(|p| p.id.clone()).collect();
    let filled = |id: &str| sess.pending.iter().any(|p| p.id == id && p.status == PendingStatus::Filled);
    all.extend(pd::draft_places(sess).into_iter().filter(|p| !in_paper.contains(&p.id) && !filled(&p.id)));
    pd::report_from(&all, sess)
}

// ── 預想選項 (in the background, never inside a turn) ───────

/// Groups whose candidates a task of this process is preparing now. A group
/// saved as `running` that is not here was cut off (the app quit): it shows
/// as failed, with 重試.
static PREPARING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn prep_key(dir: &Path, group: &str) -> String {
    format!("{}#{group}", dir.display())
}

fn settle_stale_prep(dir: &Path, sess: &mut Session) {
    let live = PREPARING.lock().map(|v| v.clone()).unwrap_or_default();
    let items = sess.items.clone();
    for g in &mut sess.groups {
        if g.prep == Some(Prep::Running) && !live.contains(&prep_key(dir, &g.id)) {
            // cut short (the app quit): the items that got their options keep them
            let some = items.iter().any(|i| i.group == g.id && i.candidates_tried);
            g.prep = Some(if some { Prep::Done } else { Prep::Failed });
        }
    }
}

/// Mark the groups whose items want candidates as being prepared; returns them.
fn mark_prep(sess: &mut Session, lens: &Lens, groups: &[String]) -> Vec<String> {
    let mut out = vec![];
    for g in groups {
        if sess.group(g).is_some_and(|x| x.prep == Some(Prep::Running)) || ca::targets(sess, lens, std::slice::from_ref(g)).is_empty() {
            continue;
        }
        if let Some(x) = sess.group_mut(g) {
            x.prep = Some(Prep::Running);
            out.push(g.clone());
        }
    }
    out
}

/// One batched candidates call for `groups` (already marked by [`mark_prep`],
/// the session saved), on a task of its own: the caller's reply never waits on it.
fn spawn_candidates(s: &Shared, c: &Ctx, groups: Vec<String>) {
    if groups.is_empty() {
        return;
    }
    let s2 = s.clone();
    let c = Ctx { dir: c.dir.clone(), ws: c.ws.clone(), key_value: c.key_value.clone() };
    let keys: Vec<String> = groups.iter().map(|g| prep_key(&c.dir, g)).collect();
    if let Ok(mut v) = PREPARING.lock() {
        v.extend(keys.iter().cloned());
    }
    tokio::spawn(async move {
        // the items, in pieces of ca::BATCH (a long plan never outgrows one reply); each piece is
        // kept as it lands, a failed piece marks its items tried (重試 asks again)
        let ready = async {
            let sess = Session::load(&c.dir)?;
            let lens = lens_for(&c.ws, &sess).map_err(|_| "lens".to_string())?;
            let ids: Vec<String> = ca::targets(&sess, &lens, &groups).iter().map(|i| i.id.clone()).collect();
            let record = ds::records_text(&c.dir, &sess.records, usize::MAX);
            let (_, mtext) = material_of(&c);
            let mtext = mtext + &topaper::venue_of(&c).emphasis();
            Ok::<_, String>((sess, lens, ids, record, mtext))
        }
        .await;
        let mut ok = ready.is_ok();
        if let Ok((sess, lens, ids, record, mtext)) = ready {
            // group by group in plan order: the first groups are ready while the later ones wait
            let order = |id: &String| sess.items.iter().find(|i| &i.id == id).and_then(|i| groups.iter().position(|g| *g == i.group)).unwrap_or(usize::MAX);
            let mut ids = ids;
            ids.sort_by_key(order);
            let mut done_ids: Vec<String> = Vec::new();
            // three pieces at a time (one Opus reply takes minutes); results land in plan order
            use futures_util::StreamExt;
            let pieces: Vec<Vec<String>> = ids.chunks(ca::BATCH).map(|p| p.to_vec()).collect();
            let (sess_r, lens_r, rec_r, mt_r, kv_r, s_r) = (&sess, &lens, &record, &mtext, &c.key_value, &s2);
            let mut results = futures_util::stream::iter(pieces.into_iter().map(|piece| async move {
                let items: Vec<&ds::Item> = sess_r.items.iter().filter(|i| piece.contains(&i.id)).collect();
                let res = engine::candidates(&engines(s_r), kv_r, sess_r, lens_r, &items, rec_r, mt_r).await;
                (piece, res)
            }))
            .buffered(3);
            while let Some((piece, res)) = results.next().await {
                let piece = piece.as_slice();
                let _g = LOCK.lock().await;
                let Ok(mut now) = Session::load(&c.dir) else { break };
                match &res {
                    Ok((checked, _)) => {
                        ca::apply(&mut now, piece, checked);
                    }
                    Err(e) => {
                        eprintln!("預想選項: {e}");
                        ca::mark_tried(&mut now, piece);
                        ok = false;
                    }
                }
                // a group whose items all had their turn is ready now (the window looks every 3 s)
                done_ids.extend(piece.iter().cloned());
                if res.is_ok() {
                    for g in &groups {
                        let mine: Vec<&String> = ids.iter().filter(|id| sess.items.iter().any(|i| &i.id == *id && i.group == *g)).collect();
                        if mine.iter().all(|id| done_ids.contains(id)) {
                            if let Some(x) = now.group_mut(g) {
                                if x.prep == Some(Prep::Running) {
                                    x.prep = Some(Prep::Done);
                                }
                            }
                        }
                    }
                }
                let _ = now.save(&c.dir);
            }
        }
        let _g = LOCK.lock().await;
        if let Ok(mut v) = PREPARING.lock() {
            v.retain(|k| !keys.contains(k));
        }
        let Ok(mut sess) = Session::load(&c.dir) else { return };
        for g in &groups {
            if let Some(x) = sess.group_mut(g) {
                x.prep = Some(if ok { Prep::Done } else { Prep::Failed });
            }
        }
        let _ = sess.save(&c.dir);
    });
}

macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(r) => return r,
        }
    };
}

async fn get_draft(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    tri!(view(&c, &sess))
}

/// The app's engines for the draft steps, with the task table as it is now
/// (the user's tasks.toml included).
/// The engines in a fixed order (the wizard's engine choice first, then the
/// local model): what a step that used to be Claude-only tries.
struct Ordered<'a> {
    inner: engine::Live<'a>,
    order: Vec<String>,
}

#[async_trait::async_trait]
impl engine::Engines for Ordered<'_> {
    fn candidates(&self, _task: &str, _picked: Option<&str>) -> Vec<String> {
        self.order.clone()
    }
    async fn complete(&self, task: &str, model_ref: &str, prompt: &Prompt, extra: Option<&str>, schema: serde_json::Value) -> Result<String, String> {
        self.inner.complete(task, model_ref, prompt, extra, schema).await
    }
    async fn local_context(&self, task: &str) -> Option<u32> {
        self.inner.local_context(task).await
    }
}

/// One structured step for the wizard and the reference search: Claude on
/// the chosen route first (subscription or API), then the local model when
/// Claude is not available or gives nothing usable. Choosing 本機 asks the
/// local model only. A small local model gets the prompt fitted to its
/// context and its reply repaired (see `engine::ask`).
pub(crate) async fn claude_then_local<T>(s: &Shared, task: &str, choice: EngineChoice, prompt: &Prompt, schema: &serde_json::Value, parse: impl Fn(&str) -> Result<T, String>) -> Result<T, String> {
    let order = match choice {
        EngineChoice::Local => vec!["local:current".to_string()],
        c => vec![c.model_ref(), "local:current".to_string()],
    };
    let e = Ordered { inner: engines(s), order };
    let mut at = engine::Attempts::new(&e, task, None);
    while let Some(c) = at.next(&e).await {
        match engine::ask(&e, task, &c, prompt, schema, &parse).await {
            Ok(v) => return Ok(v),
            Err(f) => at.failed(&c, f),
        }
    }
    Err(at.error())
}

fn engines(s: &Shared) -> engine::Live<'_> {
    let user = config_path().map(|p| p.with_file_name("tasks.toml"));
    let table = Llm::task_table(user.as_deref()).or_else(|_| Llm::task_table(None)).unwrap_or_default();
    engine::Live { llm: &s.llm, table }
}

// ── Step 1 ──────────────────────────────────────────────

#[derive(Deserialize)]
struct TopicBody {
    /// The paper's key value: asked here, kept in paper.toml.
    #[serde(default)]
    key_value: Option<String>,
    #[serde(default)]
    reader_accepts: String,
    #[serde(default)]
    twist: String,
    #[serde(default)]
    lens: Option<String>,
}

async fn put_topic(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<TopicBody>) -> Resp {
    let mut c = tri!(ctx(&s, &name).await);
    if let Some(kv) = &b.key_value {
        let kv = match meta::check_key_value(kv) {
            Ok(k) => k,
            Err(e) => return bad(&e),
        };
        let mut m = meta::read_meta(&c.dir);
        if m.key_value != kv {
            m.key_value = kv.clone();
            if let Err(e) = meta::write_meta(&c.dir, &m) {
                return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
            }
        }
        c.key_value = kv;
    }
    if c.key_value.trim().is_empty() {
        return bad("請填寫 Key value：這篇論文通篇要強調的一件事");
    }
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.hook = ds::Hook { reader_accepts: b.reader_accepts.trim().into(), twist: b.twist.trim().into() };
    if let Some(l) = b.lens.filter(|l| paper::safe_name(l)) {
        sess.lens = l;
    }
    if sess.phase < Phase::Record {
        sess.phase = Phase::Record;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct PhaseBody {
    phase: Phase,
}

async fn put_phase(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<PhaseBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.phase = b.phase;
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── 重來 ────────────────────────────────────────────────

/// 重新開始: the whole draft/ folder goes to draft/archive/<time>/.
async fn restart(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    if let Err(e) = ds::restart(&c.dir) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    let sess = tri!(load(&c.dir));
    tri!(view(&c, &sess))
}

/// 從這步重來: back to a step; later steps' results are cleared (a copy
/// of the session goes to draft/archive/ first).
async fn rewind(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<PhaseBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    match ds::rewind(&c.dir, b.phase) {
        Ok(sess) => tri!(view(&c, &sess)),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

// ── Step 2 ──────────────────────────────────────────────

#[derive(Deserialize)]
struct RecordQuery {
    file: String,
}

/// A record file's bytes as stored: text as it is, a PDF as its text; formats to convert first are refused.
fn record_bytes(name: &str, body: &[u8]) -> Result<(String, Vec<u8>), String> {
    if body.is_empty() {
        return Err("檔案是空的".into());
    }
    let lower = name.to_lowercase();
    if lower.ends_with(".pdf") {
        match pdf_extract::extract_text_from_mem(body) {
            Ok(t) if !t.trim().is_empty() => Ok((format!("{}.txt", name.trim_end_matches(".pdf").trim_end_matches(".PDF")), t.into_bytes())),
            _ => Err("讀不到這份 PDF 的文字（可能是掃描檔）".into()),
        }
    } else if [".xlsx", ".xls", ".docx", ".doc", ".pptx", ".numbers", ".pages"].iter().any(|e| lower.ends_with(e)) {
        Err("請另存成 CSV、Markdown 或純文字再上傳（試算表：檔案 → 下載 → CSV）".into())
    } else if std::str::from_utf8(body).is_err() {
        Err("只接受文字檔（.md .txt .csv .tsv .json .log）或 PDF".into())
    } else {
        Ok((name.to_string(), body.to_vec()))
    }
}

/// Store record files (already converted) in the session.
async fn store_records(c: &Ctx, files: Vec<(String, Vec<u8>)>) -> Resp {
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    for (file, data) in files {
        let rec = match ds::save_record(&c.dir, &file, &data) {
            Ok(r) => r,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        };
        sess.records.retain(|r| r.file != rec.file);
        sess.records.push(rec);
    }
    if sess.phase < Phase::Record {
        sess.phase = Phase::Record;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct RecordText {
    text: String,
}

/// A record the author typed or pasted: stored as `手寫紀錄-<time>.md`.
async fn record_from_text(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<RecordText>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    if b.text.trim().is_empty() {
        return bad("沒有內容");
    }
    let stamp: String = lore_polish::log::now().chars().take(19).filter(|c| c.is_ascii_digit() || *c == 'T').collect::<String>().replace('T', "-");
    let file = format!("手寫紀錄-{stamp}.md");
    store_records(&c, vec![(file, b.text.into_bytes())]).await
}

/// Text records are stored as they are; a PDF is stored as its text.
async fn upload_record(State(s): State<Shared>, AxPath(name): AxPath<String>, AxQuery(q): AxQuery<RecordQuery>, body: axum::body::Bytes) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    match record_bytes(&q.file, &body) {
        Ok(f) => store_records(&c, vec![f]).await,
        Err(e) => bad(&e),
    }
}

/// Largest record file read from a path.
const RECORD_MAX_BYTES: u64 = 20 * 1024 * 1024;

/// A typed or pasted path (`~/…`, quoted, `file://…`) as a path inside the home folder.
fn record_path(raw: &str, home: &Path) -> Result<PathBuf, String> {
    let mut s = raw.trim().trim_matches(|c| c == '"' || c == '\'' ).trim().to_string();
    if let Some(rest) = s.strip_prefix("file://") {
        s = percent_decode(rest);
    }
    let s = s.replace("\\ ", " ");
    let p = if s == "~" { home.to_path_buf() } else if let Some(r) = s.strip_prefix("~/") { home.join(r) } else { PathBuf::from(&s) };
    if !p.is_absolute() {
        return Err("請貼完整路徑（/Users/… 或 ~/…）".into());
    }
    let real = p.canonicalize().map_err(|_| format!("找不到：{s}"))?;
    let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    if !real.starts_with(&home) {
        return Err("只讀家目錄底下的檔案".into());
    }
    Ok(real)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The record files at a path: the file itself, or a folder's files (not its subfolders),
/// with the ones that cannot be records listed by name and reason.
fn record_files_at(p: &Path) -> Result<(Vec<(String, Vec<u8>)>, Vec<String>), String> {
    let paths: Vec<PathBuf> = if p.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).filter(|x| x.is_file() && !x.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'))).collect();
        v.sort();
        v
    } else {
        vec![p.to_path_buf()]
    };
    let (mut ok, mut skipped) = (vec![], vec![]);
    for f in paths {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        // any name: save_record turns it into a safe one
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len > RECORD_MAX_BYTES {
            skipped.push(format!("{name}：超過 20 MB"));
            continue;
        }
        match std::fs::read(&f).map_err(|e| e.to_string()).and_then(|b| record_bytes(&name, &b)) {
            Ok(x) => ok.push(x),
            Err(e) => skipped.push(format!("{name}：{e}")),
        }
    }
    Ok((ok, skipped))
}

/// 搜尋檔案 (LoreEcho, `lore_shell::findfile`): a file the author chose for 實驗紀錄 becomes a
/// record of paper `name`. Only inside `home`; a folder gives its files. `Ok` = what LoreEcho says.
pub(crate) async fn record_picked(s: &Shared, name: &str, path: &Path, home: &Path) -> Result<String, String> {
    let c = ctx(s, name).await.map_err(|_| "找不到這篇論文".to_string())?;
    let p = record_path(&path.to_string_lossy(), home)?;
    let (files, skipped) = record_files_at(&p)?;
    if files.is_empty() {
        return Err(if skipped.is_empty() { "這裡沒有檔案".to_string() } else { format!("不能用：{}", skipped.join("；")) });
    }
    let names: Vec<String> = files.iter().map(|f| f.0.clone()).collect();
    let r = store_records(&c, files).await;
    if !r.status().is_success() {
        let b = axum::body::to_bytes(r.into_body(), 64 * 1024).await.unwrap_or_default();
        return Err(String::from_utf8_lossy(&b).trim().to_string());
    }
    Ok(if skipped.is_empty() { names.join("、") } else { format!("{}（略過 {} 個）", names.join("、"), skipped.len()) })
}

async fn delete_record(State(s): State<Shared>, AxPath((name, file)): AxPath<(String, String)>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    if !paper::safe_name(&file) {
        return bad("invalid file");
    }
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.records.retain(|r| r.file != file);
    let _ = std::fs::remove_file(ds::draft_dir(&c.dir).join(ds::RECORDS_DIR).join(&file));
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

async fn run_record(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    if sess.records.is_empty() {
        return bad("先上傳實驗記錄");
    }
    let lens = tri!(lens_for(&c.ws, &sess));
    let text = ds::records_text(&c.dir, &sess.records, usize::MAX);
    let (_, mtext) = material_of(&c);
    let (r, used) = tri!(engine::record(&engines(&s), &c.key_value, &lens, &text, &mtext).await.map_err(|e| bad(&e)));
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.engines.insert("record".into(), used);
    sess.record_summary = r.summary;
    sess.experiments = r.experiments;
    sess.timeline = r.timeline;
    sess.thoughts = r.thoughts;
    // Tables already put into the paper stay marked; new ones replace the rest.
    let kept: Vec<ds::ResultTable> = sess.tables.iter().filter(|t| t.inserted).cloned().collect();
    let mut tables = kept;
    for mut t in r.tables {
        if tables.iter().any(|k| k.caption == t.caption) {
            continue;
        }
        t.id = format!("R{}", tables.len() + 1);
        tables.push(t);
    }
    sess.tables = tables;
    sess.conflicts = r.conflicts;
    // 待補: items follow their tables; `?` in the record becomes a pending item
    let tables = sess.tables.clone();
    pd::rebind_tables(&mut sess.pending, &tables);
    let exps = sess.experiments.clone();
    pd::rebind_experiments(&mut sess.pending, &exps);
    pd::from_record(&mut sess.pending, &exps, &tables);
    // results already filled stay in the record read again
    let filled: Vec<String> = sess.pending.iter().filter(|p| p.status == PendingStatus::Filled).map(|p| p.id.clone()).collect();
    for id in &filled {
        pd::write_back(&mut sess, id);
    }
    // the record now has a waiting result: its 修訂 is made, the conversation settles the direction
    let mut prep = vec![];
    for k in pd::arrived(&sess) {
        // the record's words go into the request, not into the paper: the values come from the author
        let line = sess.pending[k].experiment.as_ref().and_then(|e| sess.experiments.iter().find(|x| &x.id == e)).map(|e| format!("{} {} {}", e.title, e.metrics, e.result)).unwrap_or_default();
        let it = &mut sess.pending[k];
        it.status = PendingStatus::Filled;
        it.direction = Some(Direction::Unsure);
        prep.extend(open_fill(&c, &mut sess, k));
        if let Some(r) = sess.pending[k].revision.clone().and_then(|r| sess.revisions.iter_mut().find(|x| x.id == r)) {
            r.request = format!("{}。記錄：{}", r.request, line.trim());
        }
    }
    pd::sync_group(&mut sess);
    if sess.phase < Phase::Plan {
        sess.phase = Phase::Plan;
    }
    tri!(save(&c.dir, &sess));
    spawn_candidates(&s, &c, prep);
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct ExperimentsBody {
    #[serde(default)]
    summary: Option<String>,
    experiments: Vec<ds::Experiment>,
    timeline: Vec<ds::TimelineStep>,
}

async fn put_experiments(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<ExperimentsBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    if let Some(x) = b.summary {
        sess.record_summary = x;
    }
    sess.experiments = b.experiments;
    sess.timeline = b.timeline;
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── Step 3 ──────────────────────────────────────────────

async fn run_plan(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    // 圖表規劃 at the same time (its own task; the plan never waits for it)
    if sess_figures_idle(&c) {
        spawn_figures(&s, &c);
    }
    let sess = tri!(load(&c.dir));
    let lens = tri!(lens_for(&c.ws, &sess));
    let (_, mtext) = material_of(&c);
    let (groups, used) = tri!(engine::plan(&engines(&s), &c.key_value, &sess, &lens, &style_text(&c.ws), &mtext).await.map_err(|e| bad(&e)));
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    if sess.groups.iter().any(|g| g.turns > 0) {
        return bad("已經開始對話了；要重新規劃請先在題組清單裡調整");
    }
    sess.engines.insert("plan".into(), used);
    sess.set_plan(groups);
    pd::sync_group(&mut sess);
    if sess.phase < Phase::Interview {
        sess.phase = Phase::Interview;
    }
    // 預想選項 for every group, before the interview starts
    let all: Vec<String> = sess.groups.iter().map(|g| g.id.clone()).collect();
    let prep = mark_prep(&mut sess, &lens, &all);
    tri!(save(&c.dir, &sess));
    spawn_candidates(&s, &c, prep);
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct PlanEdit {
    /// Groups in the new order; groups left out are removed with their items.
    groups: Vec<ds::Group>,
    /// New or edited items (by id; a new item has an empty id).
    #[serde(default)]
    items: Vec<ds::Item>,
}

async fn put_plan(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<PlanEdit>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let mut groups = vec![];
    for mut g in b.groups {
        if !ds::SECTIONS.contains(&g.section.as_str()) {
            return bad("未知的章節");
        }
        if let Some(old) = sess.group(&g.id) {
            // Keep what the conversation produced; only the plan's fields change.
            let (title, purpose, section) = (g.title, g.purpose, g.section);
            g = ds::Group { title, purpose, section, ..old.clone() };
        } else {
            g.id = format!("g{}", sess.groups.len() + groups.len() + 1);
            while sess.group(&g.id).is_some() || groups.iter().any(|x: &ds::Group| x.id == g.id) {
                g.id = format!("{}x", g.id);
            }
            g.status = ds::GroupStatus::Pending;
            g.turns = 0;
        }
        groups.push(g);
    }
    let keep: Vec<String> = groups.iter().map(|g| g.id.clone()).collect();
    sess.items.retain(|i| keep.contains(&i.group));
    sess.groups = groups;
    for mut it in b.items {
        if !keep.contains(&it.group) || it.label.trim().is_empty() {
            continue;
        }
        match sess.items.iter_mut().find(|x| !it.id.is_empty() && x.id == it.id) {
            Some(x) => {
                x.label = it.label;
                x.role = it.role;
            }
            None => {
                it.id = sess.new_item_id();
                sess.items.push(it);
            }
        }
    }
    if sess.active.as_ref().is_some_and(|a| !keep.contains(a)) {
        sess.active = None;
    }
    // the 待補項目 group comes back while results wait
    pd::sync_group(&mut sess);
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── Step 4 ──────────────────────────────────────────────

#[derive(Deserialize)]
struct TurnBody {
    group: String,
    /// Candidates the author chose for this answer (ids of the item asked about).
    #[serde(default)]
    picked: Vec<String>,
    /// The item they belong to, when chosen from the ledger (default: the item the last question was about).
    #[serde(default)]
    item: Option<String>,
    /// None: open the group with its first question.
    #[serde(default)]
    answer: Option<String>,
    /// "haiku" | "sonnet" | "opus" | "local"; none = the task's own order.
    #[serde(default)]
    model: Option<String>,
}

fn model_ref(m: Option<&str>) -> Result<Option<String>, Resp> {
    match m.map(str::trim).filter(|m| !m.is_empty() && *m != "auto") {
        None => Ok(None),
        Some("local") => Ok(Some("local:current".into())),
        Some(a @ ("haiku" | "sonnet" | "opus")) => Ok(Some(format!("claude:{a}"))),
        Some(_) => Err(bad("未知的模型")),
    }
}

async fn run_turn(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<TurnBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let mref = tri!(model_ref(b.model.as_deref()));
    let answer = b.answer.map(|a| a.trim().to_string()).filter(|a| !a.is_empty());
    let (mat, mut mtext) = material_of(&c);
    let prompt = {
        let _g = LOCK.lock().await;
        let mut sess = tri!(load(&c.dir));
        if let Some(r) = sess.revisions.iter().find(|r| r.group == b.group) {
            match r.pending.as_deref().and_then(|p| sess.pending.iter().find(|x| x.id == p)) {
                // 回填: what came in, the author's explanation, every dependent sentence
                Some(item) => {
                    let rep = pending_report(&c, &sess).into_iter().find(|e| e.id == item.id);
                    mtext += &pd::turn_context(item, rep.as_ref());
                    mtext += &rv::turn_context(r, &c.dir);
                }
                None => mtext += &rv::turn_context(r, &c.dir),
            }
        }
        let lens = tri!(lens_for(&c.ws, &sess));
        if sess.group(&b.group).is_none() {
            return bad("沒有這個題組");
        }
        let turns = ds::read_turns(&c.dir);
        // a choice among the prepared candidates: the item asked about last
        let about = b
            .item
            .clone()
            .filter(|i| sess.items.iter().any(|x| &x.id == i && x.group == b.group))
            .unwrap_or_else(|| turns.iter().rev().find(|t| t.group == b.group && t.who == Who::Ai).map(|t| t.about.clone()).unwrap_or_default());
        let (answer, picked) = ca::resolve_answer(&sess, &about, answer, b.picked.clone());
        let shown = answer.as_ref().map(|a| if picked.is_empty() { a.clone() } else { format!("{a}\n(picked {} for {about})", picked.join(", ")) });
        let prompt = dp::turn_prompt(&c.key_value, &sess, &lens, &b.group, &turns, shown.as_deref(), &mtext);
        // The answer is kept even if the model call fails; sending it again
        // after a failure does not record it twice.
        let resent = answer.as_ref().is_some_and(|a| turns.iter().rev().find(|t| t.group == b.group).is_some_and(|t| t.who == Who::Me && &t.text == a));
        let mut me_turn = None;
        if let Some(a) = answer.as_ref().filter(|_| !resent) {
            let t = ds::Turn { n: 0, group: b.group.clone(), who: Who::Me, text: a.clone(), options: vec![], engine: String::new(), about: if picked.is_empty() { String::new() } else { about.clone() }, picked: picked.clone() };
            match ds::append_turn_with(&c.dir, t) {
                Ok(t) => me_turn = Some(t.n),
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
            }
            if let Some(g) = sess.group_mut(&b.group) {
                g.turns += 1;
            }
            ca::note_picked(&mut sess, &about, &picked);
        }
        sess.start_group(&b.group);
        if sess.phase < Phase::Interview {
            sess.phase = Phase::Interview;
        }
        tri!(save(&c.dir, &sess));
        (prompt, lens, me_turn, answer)
    };
    let (prompt, lens, me_turn, answer) = prompt;
    let (mut r, used) = tri!(engine::turn(&engines(&s), mref.as_deref(), &prompt).await.map_err(|e| bad(&e)));
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    // a question back: not an answer. It leaves the interview for 深度討論 (the model's
    // explanation opens it); nothing of it is graded or kept as material
    if r.discuss {
        dp::drop_grading(&mut r);
        let last_q = ds::read_turns(&c.dir).into_iter().rev().find(|t| t.group == b.group && t.who == Who::Ai);
        if let Some(n) = me_turn {
            if let Err(e) = ds::remove_turn(&c.dir, n) {
                return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
            }
            if let Some(g) = sess.group_mut(&b.group) {
                g.turns = g.turns.saturating_sub(1);
            }
        }
        let (question, item) = last_q.map(|t| (t.text, t.about)).unwrap_or_default();
        let d = pc_core::draft::discuss::open(&mut sess, &b.group, &item, &question);
        if let Some(a) = answer.as_ref() {
            d.messages.push(dz::Message { who: Who::Me, text: a.clone() });
        }
        if !r.reply.trim().is_empty() {
            d.messages.push(dz::Message { who: Who::Ai, text: r.reply.trim().to_string() });
        }
        tri!(save(&c.dir, &sess));
        return tri!(view(&c, &sess));
    }
    // a real answer: a discussion still open on this group's question is over (never reopened by itself)
    for d in sess.discussions.iter_mut().filter(|d| d.group == b.group && d.status == dz::Status::Open) {
        d.status = dz::Status::Closed;
    }
    let known = ref_keys(&mat);
    sess.apply(&lens, &b.group, &known, r.updates);
    if let Err(e) = dp::add_ref_notes(&c.dir, &r.ref_notes, &known) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    dp::apply_pending_notes(&mut sess, &r.pending_notes);
    // 寫法筆記: how he said to write, in his words (a punctuation wish against his rule is flagged)
    let guard_on = crate::workspace_guard(&s).await.active();
    pc_core::draft::notes::add(&mut sess, &b.group, std::mem::take(&mut r.writing_notes), guard_on);
    if let Some(g) = sess.group_mut(&b.group) {
        let keep = |new: String, old: &str| if new.trim().is_empty() { old.to_string() } else { new };
        let old = g.summary.clone();
        g.summary = ds::GroupSummary {
            explain: keep(r.summary.explain, &old.explain),
            motive: keep(r.summary.motive, &old.motive),
            conclusion: keep(r.summary.conclusion, &old.conclusion),
            reader_accepts: keep(r.summary.reader_accepts, &old.reader_accepts),
            twist: keep(r.summary.twist, &old.twist),
        };
    }
    let say = [r.reply.trim(), r.question.trim()].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join("\n");
    // the question's item: its prepared candidates are the choices (the turn's options are dropped)
    let about = r.item.filter(|i| sess.items.iter().any(|x| &x.id == i && x.group == b.group)).unwrap_or_default();
    let options = if sess.items.iter().any(|x| x.id == about && !x.candidates.is_empty()) { vec![] } else { r.options };
    if !say.is_empty() {
        let t = ds::Turn { n: 0, group: b.group.clone(), who: Who::Ai, text: say, options, engine: used.label.clone(), about, picked: vec![] };
        if let Err(e) = ds::append_turn_with(&c.dir, t) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    }
    // items the answer added: their candidates come in the background (the turn's own options meanwhile)
    let prep = mark_prep(&mut sess, &lens, std::slice::from_ref(&b.group));
    tri!(save(&c.dir, &sess));
    spawn_candidates(&s, &c, prep);
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct GroupBody {
    /// start | close | skip | reopen
    action: String,
}

async fn group_action(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>, Json(b): Json<GroupBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    if sess.group(&id).is_none() {
        return bad("沒有這個題組");
    }
    let mut prep = vec![];
    match b.action.as_str() {
        "start" | "reopen" => {
            sess.start_group(&id);
            if let Ok(lens) = lens_for(&c.ws, &sess) {
                prep = mark_prep(&mut sess, &lens, std::slice::from_ref(&id));
            }
        }
        // 重試 the options of this group's items that have none
        "prepare" => {
            ca::retry(&mut sess, &id);
            if let Ok(lens) = lens_for(&c.ws, &sess) {
                prep = mark_prep(&mut sess, &lens, std::slice::from_ref(&id));
            }
        }
        "close" => sess.close_group(&id, false),
        "skip" => sess.close_group(&id, true),
        _ => return bad("unknown action"),
    }
    if sess.interview_finished() && sess.phase == Phase::Interview {
        sess.phase = Phase::Claims;
    }
    tri!(save(&c.dir, &sess));
    spawn_candidates(&s, &c, prep);
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct ExcludeBody {
    #[serde(default = "yes")]
    excluded: bool,
}
fn yes() -> bool {
    true
}

/// 這題不用寫進論文: the item is never asked again, never in the ledger, the claims or the draft.
async fn exclude_item(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>, Json(b): Json<ExcludeBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let Some(it) = sess.items.iter_mut().find(|i| i.id == id) else { return bad("沒有這個項目") };
    it.excluded = b.excluded;
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── 圖表規劃 ─────────────────────────────────────────────

fn sess_figures_idle(c: &Ctx) -> bool {
    load(&c.dir).map(|s| !s.figures.as_ref().is_some_and(|f| f.running)).unwrap_or(false)
}

/// Papers whose figure plan a task of this process is making now; a plan saved as running that
/// is not here was cut off (the app quit): it shows as failed, with 重新規劃圖表.
static FIGURING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Labels of the tables and figures already in the paper.
fn paper_labels(c: &Ctx) -> Vec<String> {
    material::floats(&c.dir).into_iter().map(|f| f.id).collect()
}

/// One call on a task of its own: the list lands in the session when it is ready (the window
/// looks again); the author's choices on an earlier list are kept for items with the same label.
fn spawn_figures(s: &Shared, c: &Ctx) {
    let s2 = s.clone();
    let c = Ctx { dir: c.dir.clone(), ws: c.ws.clone(), key_value: c.key_value.clone() };
    let key = c.dir.display().to_string();
    if let Ok(mut v) = FIGURING.lock() {
        v.push(key.clone());
    }
    tokio::spawn(async move {
        struct Done(String);
        impl Drop for Done {
            fn drop(&mut self) {
                if let Ok(mut v) = FIGURING.lock() {
                    v.retain(|k| *k != self.0);
                }
            }
        }
        let _done = Done(key);
        {
            let _g = LOCK.lock().await;
            let Ok(mut sess) = Session::load(&c.dir) else { return };
            let plan = sess.figures.get_or_insert_with(Default::default);
            plan.running = true;
            plan.failed.clear();
            let _ = sess.save(&c.dir);
        }
        let t0 = std::time::Instant::now();
        let sess = match Session::load(&c.dir) { Ok(x) => x, Err(_) => return };
        let venue = topaper::venue_of(&c);
        let targets = venue.profile.as_ref().map(|p| p.float_targets());
        let floats = venue.profile.as_ref().map(|p| p.floats_block()).unwrap_or_default();
        let (_, mtext) = material_of(&c);
        let context = dp::context(&c.key_value, &sess, &mtext);
        let prompt = pc_core::draft::figures::prompt(&context, &floats, targets);
        let known = ca::known_ids(&sess);
        let pending: Vec<String> = sess.pending.iter().filter(|p| p.status == PendingStatus::Waiting).map(|p| p.id.clone()).collect();
        let res = engine::figures(&engines(&s2), &prompt, &known, &pending, &paper_labels(&c)).await;
        let secs = t0.elapsed().as_secs();
        eprintln!("圖表規劃: {secs} s");
        let _g = LOCK.lock().await;
        let Ok(mut now) = Session::load(&c.dir) else { return };
        let old = now.figures.take().unwrap_or_default();
        let mut plan = pc_core::draft::figures::FigurePlan { figures: targets.map(|t| t.0), tables: targets.map(|t| t.1), seconds: Some(secs), ..Default::default() };
        match res {
            Ok((mut items, used)) => {
                for it in items.iter_mut() {
                    if let Some(o) = old.items.iter().find(|o| o.label == it.label) {
                        it.status = o.status;
                        it.own = o.own.clone();
                        it.chosen = o.chosen.filter(|k| *k < it.options.len());
                    }
                }
                plan.items = items;
                plan.engine = used.label;
            }
            Err(e) => {
                plan.items = old.items;
                plan.failed = e;
            }
        }
        now.figures = Some(plan);
        if !now.claims.is_empty() {
            pc_core::draft::figures::reconcile(&mut now);
        }
        let _ = now.save(&c.dir);
    });
}

/// 重新規劃圖表 (or the first time, when the plan already exists).
async fn run_figures(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    if !sess_figures_idle(&c) {
        return bad("圖表規劃還在進行");
    }
    spawn_figures(&s, &c);
    // the task marks it running at once; the window looks again
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let sess = tri!(load(&c.dir));
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct FiguresEdit {
    items: Vec<pc_core::draft::figures::PlannedFloat>,
}

/// The author's choices: keep / remove, the rendering, their own words, a new item.
async fn put_figures(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<FiguresEdit>) -> Resp {
    use pc_core::draft::figures::{PlannedFloat, Status};
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let plan = sess.figures.get_or_insert_with(Default::default);
    let mut out: Vec<PlannedFloat> = vec![];
    for mut e in b.items {
        match plan.items.iter().find(|x| !e.id.is_empty() && x.id == e.id) {
            // only the author's fields change
            Some(x) => {
                let mut x = x.clone();
                x.status = e.status;
                x.chosen = e.chosen.filter(|k| *k < x.options.len());
                x.own = e.own.trim().to_string();
                out.push(x);
            }
            None if !e.title.trim().is_empty() => {
                // an item the author adds: kept, drawn their way
                let kind = if e.kind == "table" { "table" } else { "figure" };
                let n = plan.items.iter().chain(out.iter()).filter(|x| x.kind == kind).count() + 1;
                e.id = format!("{}{n}", if kind == "table" { "T" } else { "F" });
                e.kind = kind.into();
                let slug: String = e.title.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ').collect::<String>().split_whitespace().take(4).collect::<Vec<_>>().join("-").to_lowercase();
                e.label = format!("{}:{}", if kind == "table" { "tab" } else { "fig" }, if slug.is_empty() { e.id.to_lowercase() } else { slug });
                if e.tool.is_empty() {
                    e.tool = if kind == "table" { "table".into() } else { "image".into() };
                }
                if e.own.trim().is_empty() {
                    e.own = e.title.clone();
                }
                e.status = Status::Kept;
                out.push(e);
            }
            None => {}
        }
    }
    plan.items = out;
    if !sess.claims.is_empty() {
        pc_core::draft::figures::reconcile(&mut sess);
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── 寫法筆記 ─────────────────────────────────────────────

#[derive(Deserialize)]
struct NotesEdit {
    notes: Vec<pc_core::draft::notes::WritingNote>,
}

/// The author edits the notes: text, scope, 以後也用; a note left out is deleted.
async fn put_notes(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<NotesEdit>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let mut out = vec![];
    for e in b.notes {
        let Some(old) = sess.writing_notes.iter().find(|n| n.id == e.id) else { continue };
        let text = e.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        let mut n = old.clone();
        n.text = text;
        n.scope = if matches!(e.scope.as_str(), "paper" | "section" | "paragraph") { e.scope } else { n.scope };
        n.keep = e.keep;
        n.banned = e.banned.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        out.push(n);
    }
    sess.writing_notes = out;
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

/// 存進我的寫法: the notes marked 以後也用 into the workspace's style/ (only on this click).
async fn keep_notes(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let (file, n) = tri!(pc_core::draft::notes::keep(&c.ws, &name, &pc_core::venueprofile::today(), &mut sess).map_err(|e| bad(&e)));
    tri!(save(&c.dir, &sess));
    let _ = file;
    let mut v = tri!(view(&c, &sess));
    if n > 0 {
        v.headers_mut().insert("x-kept", axum::http::HeaderValue::from(n as u64));
    }
    v
}

// ── 深度討論 ─────────────────────────────────────────────

#[derive(Deserialize)]
struct DiscussOpen {
    group: String,
}

/// Open (or reopen) the discussion of the group's current question.
async fn discuss_open(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<DiscussOpen>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    if sess.group(&b.group).is_none() {
        return bad("沒有這個題組");
    }
    let Some(q) = ds::read_turns(&c.dir).into_iter().rev().find(|t| t.group == b.group && t.who == Who::Ai) else { return bad("這一組還沒有問題") };
    dz::open(&mut sess, &b.group, &q.about, &q.text);
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct DiscussSay {
    text: String,
}

/// The author says something; the discussion model (Sonnet) replies.
async fn discuss_say(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>, Json(b): Json<DiscussSay>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let text = b.text.trim().to_string();
    if text.is_empty() {
        return bad("沒有內容");
    }
    let prompt = {
        let _g = LOCK.lock().await;
        let mut sess = tri!(load(&c.dir));
        let Some(d) = sess.discussions.iter_mut().find(|d| d.id == id && d.status == dz::Status::Open) else { return bad("這個討論已經結束") };
        // resent after a failure: not twice
        if !d.messages.last().is_some_and(|m| m.who == Who::Me && m.text == text) {
            d.messages.push(dz::Message { who: Who::Me, text });
        }
        let d = d.clone();
        tri!(save(&c.dir, &sess));
        let record = ds::records_text(&c.dir, &sess.records, usize::MAX);
        dz::prompt(&c.key_value, &sess, &record, &ds::read_turns(&c.dir), &d)
    };
    let (reply, _) = tri!(engine::discuss(&engines(&s), &prompt).await.map_err(|e| bad(&e)));
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    if let Some(d) = sess.discussions.iter_mut().find(|d| d.id == id) {
        d.messages.push(dz::Message { who: Who::Ai, text: reply });
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

/// 整理成這題的回答: the author's answer from the discussion, for them to edit (nothing is sent yet).
async fn discuss_answer(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    let Some(d) = sess.discussions.iter().find(|d| d.id == id) else { return bad("沒有這個討論") };
    if !d.messages.iter().any(|m| m.who == Who::Me) {
        return bad("還沒有討論內容");
    }
    let p = dz::answer_prompt(&c.key_value, &sess, d);
    let (answer, _) = tri!(engine::discuss_answer(&engines(&s), &p).await.map_err(|e| bad(&e)));
    Json(serde_json::json!({ "answer": answer })).into_response()
}

#[derive(Deserialize)]
struct DiscussClose {
    /// The confirmed answer: sent through the interview as the answer to the discussed item.
    #[serde(default)]
    answer: Option<String>,
    /// 這題不用寫進論文
    #[serde(default)]
    exclude: bool,
    #[serde(default)]
    model: Option<String>,
}

/// End a discussion: with the author's confirmed answer (a normal turn, graded like any
/// answer), without one, or with the item left out of the paper.
async fn discuss_close(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>, Json(b): Json<DiscussClose>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let answer = b.answer.as_deref().map(str::trim).filter(|a| !a.is_empty()).map(str::to_string);
    let d = {
        let _g = LOCK.lock().await;
        let mut sess = tri!(load(&c.dir));
        let Some(d) = sess.discussions.iter_mut().find(|d| d.id == id) else { return bad("沒有這個討論") };
        d.status = if answer.is_some() { dz::Status::Answered } else { dz::Status::Closed };
        if let Some(a) = &answer {
            d.answer = a.clone();
        }
        let d = d.clone();
        if b.exclude {
            if let Some(it) = sess.items.iter_mut().find(|i| i.id == d.item) {
                it.excluded = true;
            }
        }
        tri!(save(&c.dir, &sess));
        d
    };
    match answer {
        Some(a) => run_turn(State(s), AxPath(name), Json(TurnBody { group: d.group.clone(), picked: vec![], item: None, answer: Some(dz::as_answer(&d, &a)), model: b.model })).await,
        None => {
            let sess = tri!(load(&c.dir));
            tri!(view(&c, &sess))
        }
    }
}

// ── Step 4.5 and 5 ──────────────────────────────────────

async fn run_claims(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    let lens = tri!(lens_for(&c.ws, &sess));
    let (mat, mtext) = material_of(&c);
    let (mut claims, used) = tri!(engine::claims(&engines(&s), &c.key_value, &sess, &lens, &mtext).await.map_err(|e| bad(&e)));
    let known = ref_keys(&mat);
    for c in &mut claims {
        c.refs.retain(|k| known.contains(k));
    }
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.claims = claims;
    // 圖表規劃: each figure and table against the claims it backs
    pc_core::draft::figures::reconcile(&mut sess);
    sess.engines.insert("claims".into(), used);
    if sess.phase < Phase::Review {
        sess.phase = Phase::Review;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct ClaimsBody {
    claims: Vec<ds::Claim>,
}

async fn put_claims(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<ClaimsBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let mut claims: Vec<ds::Claim> = b.claims.into_iter().filter(|c| !c.zh.trim().is_empty()).collect();
    let mut n = claims.iter().filter_map(|c| c.id.strip_prefix('c')?.parse::<usize>().ok()).max().unwrap_or(0);
    for c in &mut claims {
        if c.id.is_empty() {
            n += 1;
            c.id = format!("c{n}");
        }
    }
    sess.claims = claims;
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

async fn run_review(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    let lens = tri!(lens_for(&c.ws, &sess));
    let (review, used) = tri!(engine::review(&engines(&s), &c.key_value, &sess, &lens).await.map_err(|e| bad(&e)));
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.review = Some(review);
    sess.engines.insert("review".into(), used);
    if sess.phase < Phase::Template {
        sess.phase = Phase::Template;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct ReviewBody {
    /// Suggestion id → accepted.
    decisions: std::collections::HashMap<String, Option<bool>>,
}

async fn put_review(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<ReviewBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    if let Some(r) = &mut sess.review {
        for x in &mut r.suggestions {
            if let Some(d) = b.decisions.get(&x.id) {
                x.accepted = *d;
            }
        }
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── ★ 版型 ──────────────────────────────────────────────

/// Every template in the workspace, as the suggester sees it; `except` is
/// the paper itself (its own template is not a habit).
fn candidates(ws: &Path, except: &Path) -> Vec<venue::Candidate> {
    let mut uses: std::collections::HashMap<String, usize> = Default::default();
    for e in std::fs::read_dir(ws.join("papers")).into_iter().flatten().flatten() {
        if e.path() != except {
            if let Some(j) = meta::read_meta(&e.path()).journal {
                *uses.entry(j).or_default() += 1;
            }
        }
    }
    let publishers = ws.join("publishers");
    pc_core::journal::list(&publishers)
        .into_iter()
        .map(|j| venue::Candidate {
            sections: pc_core::journal::read_sample(&publishers.join(&j.id)).map(|x| crate::wizard::sample_sections(&x)).unwrap_or_default(),
            uses: uses.get(&j.id).copied().unwrap_or(0),
            kind: j.meta.kind,
            id: j.id,
        })
        .collect()
}

async fn template_choices(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    let suggested = venue::suggest(&sess, &candidates(&c.ws, &c.dir), 3);
    Json(serde_json::json!({ "current": meta::read_meta(&c.dir).journal, "suggested": suggested })).into_response()
}

/// Put the paper on `template` (None = the plain article), with its
/// authors written again from the profile in the new class's style.
pub(crate) async fn set_paper_template(s: &Shared, ws: &Path, dir: &Path, template: Option<&str>) -> Result<(), Resp> {
    let people = resolve_people(s, &meta::read_meta(dir).authors).await.map_err(|e| bad(&e))?;
    retemplate(ws, dir, template, people).map_err(|e| bad(&e)).map(|_| ())
}

/// The section files keep every word; the old main.tex is kept in
/// draft/archive/. False when the paper is on that template already.
fn retemplate(ws: &Path, dir: &Path, template: Option<&str>, people: Option<(Vec<Person>, Vec<Affiliation>)>) -> Result<bool, String> {
    if let Some(t) = template {
        if !paper::safe_name(t) || !ws.join("publishers").join(t).join("sample.tex").is_file() {
            return Err("找不到這個版型".into());
        }
    }
    let mut m = meta::read_meta(dir);
    if m.journal.as_deref() == template {
        return Ok(false);
    }
    // A LaTeX project opened as it is keeps its own main file and structure.
    if pc_core::open::is_custom(dir) {
        return Err("這篇是原地開啟的 LaTeX 專案，版型請在它自己的主檔裡改；按「稍後」繼續".into());
    }
    // Already on this journal's class (a main file set up by hand: its declarations, its blind
    // switch): the journal is only recorded, main.tex is not rebuilt.
    if let Some(t) = template
        && paper::same_class(dir, &ws.join("publishers").join(t))
    {
        m.journal = Some(t.to_string());
        meta::write_meta(dir, &m)?;
        return Ok(true);
    }
    let old = paper::apply_template(dir, template.map(|t| ws.join("publishers").join(t)).as_deref())?;
    let keep = ds::new_archive(dir)?;
    std::fs::write(keep.join("main.tex"), old).map_err(|e| e.to_string())?;
    if let Some((p, a)) = people {
        let path = dir.join("main.tex");
        let main = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        paper::write_atomic(&path, &authors::apply(&main, &p, &a)).map_err(|e| e.to_string())?;
    }
    m.journal = template.map(String::from);
    meta::write_meta(dir, &m)?;
    Ok(true)
}

#[derive(Deserialize)]
struct TemplateBody {
    /// A folder under publishers/; none = the plain article.
    #[serde(default)]
    template: Option<String>,
    /// 稍後: keep the paper's template and go on.
    #[serde(default)]
    later: bool,
    /// Chosen in step 1 (主題): the session stays where it is.
    #[serde(default)]
    stay: bool,
}

async fn put_template(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<TemplateBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    if !b.later {
        let t = b.template.as_deref().map(str::trim).filter(|t| !t.is_empty());
        tri!(set_paper_template(&s, &c.ws, &c.dir, t).await);
        // an empty paper takes the journal's own sections (期刊寫法 skeleton)
        if let Err(e) = topaper::relayout_if_empty(&c.dir, &topaper::venue_of(&c)) {
            eprintln!("版型：章節沒有重排：{e}");
        }
    }
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    if sess.phase < Phase::Outline && !b.stay {
        sess.phase = Phase::Outline;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── Step 6 ──────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct OutlineBody {
    /// Revise only this section, following `comment`.
    #[serde(default)]
    section: Option<String>,
    #[serde(default)]
    comment: Option<String>,
}

async fn run_outline(State(s): State<Shared>, AxPath(name): AxPath<String>, body: Option<Json<OutlineBody>>) -> Resp {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    let lens = tri!(lens_for(&c.ws, &sess));
    let only = match (&b.section, &b.comment) {
        (Some(sec), comment) => {
            if !ds::SECTIONS.contains(&sec.as_str()) {
                return bad("未知的章節");
            }
            Some((sec.as_str(), comment.as_deref().unwrap_or("").trim()))
        }
        _ => None,
    };
    let (mat, mtext) = material_of(&c);
    let style = style_text(&c.ws) + &topaper::venue_of(&c).outline() + &crate::guard::outline_sizes(&crate::guard::paragraph_ranges(&c.ws));
    let (mut new, used) = tri!(engine::outline(&engines(&s), &c.key_value, &sess, &lens, &style, only, &mtext).await.map_err(|e| bad(&e)));
    let known = ref_keys(&mat);
    for p in new.iter_mut().flat_map(|o| o.paras.iter_mut()) {
        p.cites.retain(|k| known.contains(k));
    }
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    match only {
        Some((sec, _)) => dp::replace_section(&mut sess.outline, sec, new),
        None => {
            sess.outline = new;
            dp::number_paras(&mut sess.outline);
        }
    }
    sess.engines.insert("outline".into(), used);
    if sess.phase < Phase::Outline {
        sess.phase = Phase::Outline;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct OutlineEdit {
    outline: Vec<ds::OutlineSection>,
}

async fn put_outline(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<OutlineEdit>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.outline = b.outline.into_iter().filter(|o| ds::SECTIONS.contains(&o.section.as_str())).collect();
    dp::number_paras(&mut sess.outline);
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── Step 7 ──────────────────────────────────────────────

#[derive(Deserialize)]
struct ComposeDraftBody {
    section: String,
    #[serde(default)]
    engine: Option<EngineChoice>,
}

/// Section names in the phrase bank ("Method", "Result").
fn bank_section(s: &str) -> &str {
    match s {
        "Methods" => "Method",
        "Results" => "Result",
        other => other,
    }
}

/// The record's text given to each paragraph (a long record is cut).
const FACTS_CHARS: usize = 12_000;

/// Draft one section paragraph by paragraph through the phrase bank (the
/// same tone lock as 寫作), with the outline's unit plan, the claim register
/// and the author's style notes as extra instructions.
async fn run_compose(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<ComposeDraftBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let is_abstract = b.section == "Abstract";
    if !ds::SECTIONS.contains(&b.section.as_str()) && !is_abstract {
        return bad("未知的章節");
    }
    let sess = tri!(load(&c.dir));
    let venue = topaper::venue_of(&c);
    let paras: Vec<ds::OutlinePara> = if is_abstract {
        if sess.claims.is_empty() && sess.outline.is_empty() {
            return bad("先完成主張登記與大綱");
        }
        vec![topaper::abstract_para(&sess, &c.key_value)]
    } else {
        sess.outline.iter().filter(|o| o.section == b.section).flat_map(|o| o.paras.clone()).collect()
    };
    if paras.is_empty() {
        return bad("這個章節還沒有大綱");
    }
    let (min_words, max_words) = if is_abstract { topaper::abstract_words(&venue) } else { (60, 160) };
    // the author's punctuation, sentence and paragraph rules (style/checks.toml; off by default)
    let rules = s.loaded.read().await.profile.as_ref().map(|p| p.rules.clone()).unwrap_or_default();
    let guard = crate::guard::load(&c.ws, Some(&c.dir), &rules);
    let range = if is_abstract { None } else { crate::guard::paragraph_ranges(&c.ws).get(&b.section).copied() };
    // the journal's habits for this section (期刊寫法), and the abstract's own rules
    let venue_extra = venue.section(&b.section) + &if is_abstract { topaper::abstract_rules(&venue) } else { String::new() };
    if !has_writing_engine(&s, b.engine).await {
        return bad("沒有可用的寫作引擎");
    }
    let notation = crate::notation::load(&c.ws, &c.dir);
    let check_opt = crate::notation::options(&c.ws, &c.dir);
    let style = style_text(&c.ws) + &crate::notation::describe(&notation, check_opt.ref_words.as_ref());
    let (mat, _) = material_of(&c);
    // the record's facts (counts, datasets, settings) and every results table: what the draft
    // used to ask the author with [Author to confirm …] although the record had it
    let facts = dp::record_facts(&sess, &ds::records_text(&c.dir, &sess.records, usize::MAX), FACTS_CHARS)
        + &material::describe(&Material { floats: mat.floats.clone(), refs: vec![] });
    // the values the record already has: a sentence of only these never waits for a pending result
    let known_values = ds::records_text(&c.dir, &sess.records, usize::MAX) + &sess.tables.iter().map(|t| t.tsv()).collect::<Vec<_>>().join("\n");
    // Other sections already drafted count as context for the notation checks.
    let other_drafts: Vec<String> = sess.drafts.iter().filter(|d| d.section != b.section).map(|d| d.paras.iter().map(|p| p.text.clone()).collect::<Vec<_>>().join("\n\n")).collect();
    let mut new_symbols: Vec<String> = vec![];
    let mut out: Vec<ds::DraftPara> = vec![];
    let (guide, _) = pc_core::patterns::load(&c.ws);
    for (k, p) in paras.iter().enumerate() {
        // the guide's sentence structures that fit this paragraph (not the author's voice)
        let guide_block = pc_core::patterns::block(&pc_core::patterns::select(&guide, &pc_core::patterns::Situation { section: &b.section, function: &p.kind, index: k, count: paras.len() }));
        let points = if p.en.is_empty() { p.zh.clone() } else { p.en.iter().map(|x| format!("- {x}")).collect::<Vec<_>>().join("\n") };
        // within the section's range, a different slice for each paragraph (no three alike)
        let (min_words, max_words) = range.map(|r| r.aim(k)).unwrap_or((min_words, max_words));
        let req = ComposeRequest {
            points,
            section: bank_section(&b.section).into(),
            function: None,
            min_words,
            max_words,
            language: "en".into(),
            before: out.last().map(|d| d.text.clone()),
            after: None,
            key_value: Some(c.key_value.clone()).filter(|k| !k.is_empty()),
            index: Some(k),
            count: Some(paras.len()),
        };
        let (pts, refs, prompt) = {
            let l = s.loaded.read().await;
            let Some(prof) = &l.profile else { return (StatusCode::SERVICE_UNAVAILABLE, "profile not loaded").into_response() };
            let pts = compose::parse_points(&req.points);
            let refs = compose::reference_phrases(prof, &req, &pts);
            // the style notes go in through compose_extra below, not twice
            let prompt = compose::with_patterns(compose::build_prompt(prof, &req, &pts, &refs, ""), &guide_block);
            (pts, refs, prompt)
        };
        let earlier: Vec<String> = out.iter().map(|d| d.text.clone()).collect();
        let extra = dp::compose_extra(p, &sess, &b.section, &style, &earlier, &mat.refs, &facts) + &venue_extra + &guard.rule.prompt_line();
        let mut feedback = extra.clone();
        let mut done = None;
        let mut err = String::new();
        for _ in 0..2 {
            let reply = match writing_complete(&s, "pc-compose-paragraph", b.engine, &prompt, Some(&feedback), Some(compose::reply_schema())).await {
                Ok(r) => r,
                Err(e) => return bad(&e),
            };
            match compose::parse_draft(&reply) {
                Ok(mut d) => {
                    let l = s.loaded.read().await;
                    let Some(prof) = &l.profile else { break };
                    let check = compose::check_draft(prof, &req, &pts, &refs, &mut d);
                    drop(l);
                    // ≤, −, ⁺… as LaTeX before the checks, so pdflatex never stops on them
                    let text = lore_notation::unicode::prose_to_latex(&d.sentences.iter().map(|x| x.text.trim()).collect::<Vec<_>>().join(" "));
                    // dependency marks only for the results this paragraph states or depends on
                    // not in the abstract and conclusion: a conclusion there quoting a known number still rests on the result
                    let known = if matches!(b.section.as_str(), "Abstract" | "Conclusion") { "" } else { known_values.as_str() };
                    // a pending result the outline names must be marked: checked before the pruning
                    let unmarked_before = pd::missing_marks(&sess, &p.pending, &text);
                    let text = pd::prune_deps(&sess, &p.pending, &text, known);
                    let over = pd::overmarked(&text);
                    // The registry's checks, in the context of the paper and the drafts so far.
                    let mut ctx_texts = other_drafts.clone();
                    // planned figures and tables the paper does not have yet: their labels resolve
                    ctx_texts.extend(sess.figures.iter().flat_map(|f| f.kept()).filter(|f| !mat.floats.iter().any(|x| x.id == f.label)).map(pc_core::draft::figures::placeholder));
                    ctx_texts.push(out.iter().map(|x| x.text.clone()).collect::<Vec<_>>().join("\n\n"));
                    let nc = crate::notation::check(&c.dir, &notation, &check_opt, &ctx_texts, &text);
                    let mut problems = check.problems.clone();
                    problems.extend(nc.shown.iter().map(|m| format!("對照表：{m}")));
                    // a pending result the outline names must be marked, never written as a number
                    let unmarked = unmarked_before;
                    problems.extend(unmarked.iter().map(|id| format!("待補 {id} 沒有標記")));
                    // the record's ids (E1, T2…) are the discussion's, never the paper's
                    // only ids the record really has (a vertebra "T12" or a slice "P1" in a paper is prose)
                    let known = ca::known_ids(&sess);
                    let leaked: Vec<String> = dp::leaked_ids(&text).into_iter().filter(|id| known.contains(id)).collect();
                    problems.extend(leaked.iter().map(|id| format!("草稿寫出了記錄的編號 {id}")));
                    // the abstract's word limit is the journal's, counted, not trusted
                    let n_words = pc_core::text::strip_latex(&text).split_whitespace().count();
                    let too_long = is_abstract && n_words > max_words;
                    if too_long {
                        problems.push(format!("摘要 {n_words} 字，超過 {max_words}"));
                    }
                    problems.extend(over.iter().map(|(id, n)| format!("待補 {id} 標了 {n} 句依賴句")));
                    // his own words from the interview: a word he banned is a hard failure
                    let banned = pc_core::draft::notes::banned_in(&pc_core::draft::notes::for_paragraph(&sess, &b.section, p), &text);
                    problems.extend(banned.iter().map(|w| format!("用了訪談裡說不要的「{w}」")));
                    // the punctuation and sentence rules, and the section's paragraph length
                    let broken = guard.check(&text);
                    let length = range.and_then(|r| r.problem(&text));
                    if (check.ok() && nc.blocking.is_empty() && unmarked.is_empty() && leaked.is_empty() && !too_long && over.is_empty() && broken.is_empty() && length.is_none() && banned.is_empty()) || done.is_some() {
                        if let Some(l) = &length {
                            problems.push(format!("段落長度：{l}"));
                        }
                        new_symbols.extend(nc.new_symbols.clone());
                        done = Some(ds::DraftPara { pending: lore_notation::pending::ids(&text).into_iter().collect(), para: p.id.clone(), text, problems });
                        break;
                    }
                    let mut why: Vec<String> = check.problems.clone();
                    why.extend(nc.blocking.iter().cloned());
                    why.extend(unmarked.iter().map(|id| format!("pending result {id} is not marked: use \\pending{{{id}}}{{…}} for its number and end every sentence that depends on it with \\pendingdep{{{id}}}")));
                    why.extend(leaked.iter().map(|id| format!("the text names the record's id {id}: the reader never saw the record; describe the experiment or table in words (or \\ref the table)")));
                    why.extend(over.iter().map(|(id, n)| format!("{n} sentences end in \\pendingdep{{{id}}}: keep it only on the sentences that state {id}'s value or conclude from it, at most {}; a sentence with only known numbers is not dependent", pd::MAX_DEPS)));
                    if !broken.is_empty() {
                        why.push(lore_notation::punct::repair_note(&broken));
                    }
                    if !banned.is_empty() {
                        why.push(format!("the author said never to use: {}", banned.join(", ")));
                    }
                    if let Some(l) = &length {
                        why.push(format!("this paragraph has {l}"));
                    }
                    if too_long {
                        why.push(format!("the abstract has {n_words} words; the journal allows at most {max_words}: cut it to {min_words}–{max_words} words"));
                    }
                    feedback = format!("{extra}\nYour previous answer had problems: {}. Fix them.", why.join("; "));
                    new_symbols.extend(nc.new_symbols.clone());
                    done = Some(ds::DraftPara { pending: lore_notation::pending::ids(&text).into_iter().collect(), para: p.id.clone(), text, problems });
                }
                Err(e) => {
                    err = e;
                    feedback = format!("{extra}\nReply with the JSON object only.");
                }
            }
        }
        match done {
            Some(mut d) => {
                // after the repair: the safe mechanical fixes; what is left is flagged, never hidden
                let (t, still) = guard.finish(&d.text);
                d.text = t;
                d.problems.extend(crate::guard::flags(&still));
                out.push(d)
            }
            None => return bad(&format!("{}：{err}", p.id)),
        }
    }
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.drafts.retain(|d| d.section != b.section);
    new_symbols.sort();
    new_symbols.dedup();
    let engine = match resolve_engine(&s, b.engine).await {
        Some(EngineChoice::Local) => "本機".to_string(),
        Some(_) => pc_core::draft::fallback::engine_label(&format!("claude:{}", s.llm.model())),
        None => String::new(),
    };
    sess.drafts.push(ds::DraftSection { section: b.section.clone(), paras: out, new_symbols, engine, placed: false });
    sess.drafts.sort_by_key(|d| ds::SECTIONS.iter().position(|x| *x == d.section));
    if sess.phase < Phase::Draft {
        sess.phase = Phase::Draft;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[derive(Deserialize, Default)]
struct InsertDraftBody {
    /// One drafted section ("Methods", "Abstract"); none = every drafted section.
    #[serde(default)]
    section: Option<String>,
}

/// 放進論文: the drafts into the paper's section files and the abstract into the title block.
async fn insert_drafts(State(s): State<Shared>, AxPath(name): AxPath<String>, body: Option<Json<InsertDraftBody>>) -> Resp {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    if sess.drafts.is_empty() {
        return bad("還沒有草稿");
    }
    let placed = tri!(topaper::place(&c.dir, &topaper::venue_of(&c), &sess, b.section.as_deref()).map_err(|e| bad(&e)));
    for d in sess.drafts.iter_mut().filter(|d| placed.sections.iter().any(|(x, _)| *x == d.section)) {
        d.placed = true;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── Step 8 ──────────────────────────────────────────────

async fn run_audit(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    if sess.drafts.is_empty() {
        return bad("還沒有草稿");
    }
    let lens = tri!(lens_for(&c.ws, &sess));
    let (_, mtext) = material_of(&c);
    let venue = topaper::venue_of(&c);
    let (findings, used) = tri!(engine::audit(&engines(&s), &c.key_value, &sess, &lens, &(style_text(&c.ws) + &venue.checklist()), &mtext).await.map_err(|e| bad(&e)));
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    // marked pending text is never a missing number or an overclaim
    sess.audit = pd::filter_findings(findings, &sess);
    // the journal's requirements, checked by the code (abstract length, keywords, statements)
    let n = sess.audit.len();
    for (k, mut f) in topaper::requirement_findings(&c.dir, &venue, &sess).into_iter().enumerate() {
        f.id = format!("r{}", n + k + 1);
        sess.audit.push(f);
    }
    // 圖表規劃 against the journal's habit (warnings)
    let n = sess.audit.len();
    for (k, w) in sess.figures.as_ref().map(|f| f.count_warnings()).unwrap_or_default().into_iter().enumerate() {
        sess.audit.push(ds::Finding { id: format!("g{}", n + k + 1), kind: "venue".into(), para: String::new(), quote: String::new(), why: format!("圖表規劃：{w}"), ask: String::new(), resolved: false });
    }
    // the author's punctuation and sentence rules over the whole paper, his own text too (warnings: he decides)
    let rules = s.loaded.read().await.profile.as_ref().map(|p| p.rules.clone()).unwrap_or_default();
    let n = sess.audit.len();
    for (k, (at, why)) in crate::guard::audit(&crate::guard::load(&c.ws, Some(&c.dir), &rules), &c.dir).into_iter().enumerate() {
        sess.audit.push(ds::Finding { id: format!("u{}", n + k + 1), kind: "punctuation".into(), para: at, quote: String::new(), why, ask: String::new(), resolved: false });
    }
    sess.engines.insert("audit".into(), used);
    sess.phase = Phase::Audit;
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct AuditEdit {
    /// Finding id → resolved.
    resolved: std::collections::HashMap<String, bool>,
}

async fn put_audit(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<AuditEdit>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    for f in &mut sess.audit {
        if let Some(r) = b.resolved.get(&f.id) {
            f.resolved = *r;
        }
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

/// Put a finding's question back to the author: a 審稿追問 group in the
/// section of the paragraph, with the question as its first turn.
async fn audit_ask(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let Some(f) = sess.audit.iter().find(|f| f.id == id).cloned() else { return bad("沒有這項審稿意見") };
    let section = sess
        .outline
        .iter()
        .find(|o| o.paras.iter().any(|p| p.id == f.para))
        .map(|o| o.section.clone())
        .unwrap_or_else(|| "Discussion".into());
    let gid = format!("a{}", f.id);
    if sess.group(&gid).is_none() {
        sess.groups.push(ds::Group {
            id: gid.clone(),
            section,
            title: format!("審稿追問：{}", f.kind),
            purpose: f.why.clone(),
            ..Default::default()
        });
        let iid = sess.new_item_id();
        sess.items.push(ds::Item { id: iid, group: gid.clone(), label: if f.quote.is_empty() { f.why.clone() } else { f.quote.clone() }, role: "audit".into(), ..Default::default() });
        let q = if f.ask.trim().is_empty() { format!("{}\n這裡要怎麼處理？", f.why) } else { format!("{}\n{}", f.why, f.ask) };
        if let Err(e) = ds::append_turn(&c.dir, &gid, Who::Ai, &q, vec![]) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    }
    sess.start_group(&gid);
    sess.phase = Phase::Interview;
    let prep = lens_for(&c.ws, &sess).map(|l| mark_prep(&mut sess, &l, std::slice::from_ref(&gid))).unwrap_or_default();
    tri!(save(&c.dir, &sess));
    spawn_candidates(&s, &c, prep);
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct RegisterBody {
    symbols: Vec<String>,
}

/// Put symbols a draft or a revision introduced into the paper's registry
/// (tagged `suggested`; the author fills in the meaning in the notation panel).
async fn register_symbols(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<RegisterBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    if let Err(e) = crate::notation::register(&c.dir, &b.symbols) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    for d in &mut sess.drafts {
        d.new_symbols.retain(|x| !b.symbols.contains(x));
    }
    for r in &mut sess.revisions {
        if let Some(p) = &mut r.plan {
            p.new_symbols.retain(|x| !b.symbols.contains(x));
        }
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── 修訂 ────────────────────────────────────────────────

#[derive(Deserialize)]
struct NewRevision {
    /// What has to be added, in the author's (or the reviewer's) words.
    request: String,
    /// Section where it probably belongs (for the conversation's grouping).
    #[serde(default)]
    section: Option<String>,
    /// Library keys to add to this paper's references.
    #[serde(default)]
    refs: Vec<String>,
    /// A figure already uploaded to figures/.
    #[serde(default)]
    figure: Option<String>,
}

fn short(s: &str, n: usize) -> String {
    let t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > n { t.chars().take(n).collect::<String>() + "…" } else { t }
}

/// A revision request becomes a conversation group (`v1`, `v2`, …); its new
/// references are added to the paper's citations.csv so every step knows them.
async fn new_revision(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<NewRevision>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let request = b.request.trim().to_string();
    if request.is_empty() {
        return bad("請寫下要補什麼");
    }
    if let Some(f) = &b.figure {
        if !paper::safe_name(f) || !c.dir.join("figures").join(f).is_file() {
            return bad("找不到這個圖檔，請先上傳");
        }
    }
    let section = b.section.filter(|x| ds::SECTIONS.contains(&x.as_str())).unwrap_or_else(|| "Results".into());
    let lib: Vec<String> = bib::parse(&std::fs::read_to_string(c.ws.join("refs/library.bib")).unwrap_or_default()).0.into_iter().map(|e| e.key).collect();
    let refs: Vec<String> = b.refs.into_iter().filter(|k| lib.contains(k)).collect();
    if !refs.is_empty() {
        let path = pc_core::paper::citations_file(&c.dir);
        let mut cit = bib::read_citations(&path);
        for k in &refs {
            cit.entry(k.clone()).or_insert_with(|| format!("修訂：{}", short(&request, 40)));
        }
        if let Err(e) = bib::write_citations(&path, &cit) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let n = sess.revisions.len() + 1;
    let (id, mut gid) = (format!("v{n}"), format!("v{n}"));
    while sess.group(&gid).is_some() {
        gid.push('x');
    }
    sess.groups.push(ds::Group { id: gid.clone(), section, title: format!("修訂：{}", short(&request, 24)), purpose: request.clone(), ..Default::default() });
    let iid = sess.new_item_id();
    sess.items.push(ds::Item { id: iid, group: gid.clone(), label: short(&request, 60), role: "revision".into(), refs: refs.clone(), ..Default::default() });
    sess.revisions.push(Revision { id, request, group: gid.clone(), refs, figure: b.figure, status: RevisionStatus::Open, plan: None, pending: None });
    sess.start_group(&gid);
    let prep = lens_for(&c.ws, &sess).map(|l| mark_prep(&mut sess, &l, std::slice::from_ref(&gid))).unwrap_or_default();
    tri!(save(&c.dir, &sess));
    spawn_candidates(&s, &c, prep);
    tri!(view(&c, &sess))
}

/// The concrete changes for a revision (Opus), from its conversation.
async fn plan_revision(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    let Some(r) = sess.revisions.iter().find(|r| r.id == id).cloned() else { return bad("沒有這個修訂") };
    let fill = r.pending.as_deref().and_then(|p| sess.pending.iter().find(|x| x.id == p)).cloned();
    // 回填 as expected, nothing to write: the code fills the numbers, no model
    let code = fill.as_ref().map(|i| pd::fill_changes(&c.dir, i));
    if let (Some(item), Some((changes, effects))) = (&fill, &code)
        && item.direction == Some(Direction::Matches)
        && !effects.iter().any(|e| e.contains("預留的段落"))
    {
        let mut changes = changes.clone();
        for (k, ch) in changes.iter_mut().enumerate() {
            ch.id = format!("x{}", k + 1);
        }
        if changes.is_empty() {
            return bad("論文裡找不到這一項的待補記號");
        }
        let plan = rv::RevisionPlan { summary: format!("{} 符合預期：只換數字（{} 處）", item.id, changes.len()), changes, effects: effects.clone(), new_symbols: vec![], engine: String::new() };
        let _g = LOCK.lock().await;
        let mut sess = tri!(load(&c.dir));
        if let Some(x) = sess.revisions.iter_mut().find(|x| x.id == id) {
            x.plan = Some(plan);
            x.status = RevisionStatus::Planned;
        }
        tri!(save(&c.dir, &sess));
        return tri!(view(&c, &sess));
    }
    let conversation = ds::read_turns(&c.dir)
        .iter()
        .filter(|t| t.group == r.group)
        .map(|t| format!("{}: {}", if t.who == Who::Ai { "Q" } else { "A" }, t.text))
        .collect::<Vec<_>>()
        .join("\n");
    let summary = sess.group(&r.group).map(|g| format!("\nSummary: {} / {} / {}", g.summary.explain, g.summary.motive, g.summary.conclusion)).unwrap_or_default();
    let (_, mtext) = material_of(&c);
    let mtext = mtext + &fill.as_ref().map(pd::plan_rule).unwrap_or_default() + &crate::workspace_guard(&s).await.rule.prompt_line();
    let (mut plan, used) = tri!(engine::revise(&engines(&s), &c.key_value, &sess, &r, &c.dir, &mtext, &style_text(&c.ws), &(conversation + &summary)).await.map_err(|e| bad(&e)));
    plan.engine = used.label;
    // the model's rewritten paragraphs keep to the author's punctuation rule (mechanical fixes;
    // what is left is said in the plan). The code's own number fills below are not touched.
    {
        let rules = s.loaded.read().await.profile.as_ref().map(|p| p.rules.clone()).unwrap_or_default();
        let guard = crate::guard::load(&c.ws, Some(&c.dir), &rules);
        for ch in plan.changes.iter_mut() {
            let (t, still) = guard.finish(&ch.text);
            ch.text = t;
            plan.effects.extend(crate::guard::flags(&still).into_iter().map(|f| format!("{}：{f}", ch.file)));
        }
    }
    // 回填: numbers from the code, every paragraph with a mark covered, dependents flagged
    if let (Some(item), Some((changes, effects))) = (&fill, code) {
        plan.changes = pd::merge_plan(item, std::mem::take(&mut plan.changes), changes);
        plan.effects.extend(effects);
    }
    for ch in plan.changes.iter_mut() {
        ch.text = lore_notation::unicode::prose_to_latex(&ch.text);
    }
    // The registry's checks on every change; findings join the effects.
    let notation = crate::notation::load(&c.ws, &c.dir);
    let check_opt = crate::notation::options(&c.ws, &c.dir);
    for ch in &plan.changes {
        let nc = crate::notation::check(&c.dir, &notation, &check_opt, &[], &ch.text);
        plan.effects.extend(nc.shown.iter().map(|m| format!("{}：對照表：{m}", ch.id)));
        plan.new_symbols.extend(nc.new_symbols);
    }
    plan.new_symbols.sort();
    plan.new_symbols.dedup();
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    if let Some(x) = sess.revisions.iter_mut().find(|x| x.id == id) {
        x.plan = Some(plan);
        x.status = RevisionStatus::Planned;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct Decide {
    apply: bool,
}

async fn decide_change(State(s): State<Shared>, AxPath((name, id, cid)): AxPath<(String, String, String)>, Json(b): Json<Decide>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let Some(r) = sess.revisions.iter_mut().find(|x| x.id == id) else { return bad("沒有這個修訂") };
    let Some(plan) = r.plan.as_mut() else { return bad("還沒有修改方案") };
    let Some(ch) = plan.changes.iter_mut().find(|x| x.id == cid) else { return bad("沒有這項修改") };
    if ch.applied == Some(true) {
        return bad("已經套用過了");
    }
    if b.apply {
        // the review sees this save too (tracked changes, comment anchors)
        let _review = crate::REVIEW_LOCK.lock().await;
        let paras = |d: &std::path::Path| pc_core::open::read_section(d, &ch.file).map(|s| s.paragraphs).unwrap_or_default();
        let old = paras(&c.dir);
        if let Err(e) = rv::apply(&c.dir, ch) {
            return bad(&e);
        }
        crate::review_saved(&c.dir, &ch.file, &old, &paras(&c.dir));
        if let Err(e) = pd::ensure_tex(&c.dir).and_then(|_| pd::release_tex(&c.dir)) {
            return bad(&e);
        }
    }
    ch.applied = Some(b.apply);
    if plan.changes.iter().all(|x| x.applied.is_some()) {
        r.status = RevisionStatus::Done;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct RevStatus {
    status: RevisionStatus,
}

async fn revision_status(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>, Json(b): Json<RevStatus>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let Some(r) = sess.revisions.iter_mut().find(|x| x.id == id) else { return bad("沒有這個修訂") };
    r.status = b.status;
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct FloatsBody {
    /// Tables pasted from a spreadsheet and figures already uploaded to
    /// figures/, each put at the end of its section in this order.
    tables: Vec<TableSpec>,
}

/// Results the author already has, straight into the paper (the new-paper
/// wizard's 圖表 step, on an existing paper).
async fn add_floats(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<FloatsBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    if b.tables.is_empty() {
        return bad("沒有圖表");
    }
    if let Err(e) = add_tables(&c.dir, meta::read_meta(&c.dir).journal.as_deref(), &b.tables) {
        return bad(&e);
    }
    let sess = tri!(load(&c.dir));
    tri!(view(&c, &sess))
}

#[derive(Deserialize, Default)]
struct InsertBody {
    /// Section title to put it in; default: the Results section.
    #[serde(default)]
    section: Option<String>,
}

/// Put a table built from the record at the end of a section, in the
/// journal's table style (the same path as the new-paper wizard's tables).
async fn insert_table(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>, body: Option<Json<InsertBody>>) -> Resp {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let Some(t) = sess.tables.iter().find(|t| t.id == id).cloned() else { return bad("沒有這個表格") };
    let spec = TableSpec {
        kind: "table".into(),
        caption: lore_notation::unicode::prose_to_latex(&t.caption),
        section: b.section.unwrap_or_else(|| "Results".into()),
        // the record's −0.039 and ≤ as LaTeX: the table compiles with pdflatex
        data: pd::table_tsv(&t, &sess.pending).lines().map(lore_notation::unicode::prose_to_latex).collect::<Vec<_>>().join("\n"),
        file: String::new(),
        style: None,
    };
    if let Err(e) = add_tables(&c.dir, meta::read_meta(&c.dir).journal.as_deref(), &[spec]) {
        return bad(&e);
    }
    // pending cells compile in the draft (pending.tex, \input in the preamble)
    if let Err(e) = pd::ensure_tex(&c.dir) {
        return bad(&e);
    }
    if let Some(x) = sess.tables.iter_mut().find(|x| x.id == id) {
        x.inserted = true;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── 待補 ────────────────────────────────────────────────

async fn pending_list(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    Json(serde_json::json!({ "items": sess.pending, "report": pending_report(&c, &sess) })).into_response()
}

#[derive(Deserialize)]
struct PendingBody {
    items: Vec<pd::PendingItem>,
    /// Results tables with a row or column added for a pending result
    /// (cells are text; a table already in the paper is not changed).
    #[serde(default)]
    tables: Option<Vec<ds::ResultTable>>,
}

/// The author's 等資料 list (record step): new items get ids, the interview's
/// notes and results stay; the 待補項目 group follows.
async fn put_pending(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<PendingBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    for i in &b.items {
        if !i.id.is_empty() && !lore_notation::pending::valid_id(&i.id) {
            return bad("待補編號只能用英數字");
        }
    }
    if let Some(new) = b.tables {
        for t in new {
            if let Some(x) = sess.tables.iter_mut().find(|x| x.id == t.id && !x.inserted) {
                if t.rows.iter().all(|r| r.len() == t.columns.len()) {
                    x.columns = t.columns;
                    x.rows = t.rows;
                }
            }
        }
    }
    sess.pending = pd::merge_edit(&sess.pending, b.items);
    let exps = sess.experiments.clone();
    for i in &mut sess.pending {
        i.experiment_title = i.experiment.as_deref().and_then(|e| exps.iter().find(|x| x.id == e)).map(|e| e.title.clone()).unwrap_or_default();
    }
    let tables = sess.tables.clone();
    for i in &mut sess.pending {
        if let Some(t) = i.table.as_deref().and_then(|t| tables.iter().find(|x| x.id == t)) {
            i.table_caption = t.caption.clone();
        } else {
            i.table = None;
            i.cells.clear();
        }
    }
    pd::sync_group(&mut sess);
    let prep = lens_for(&c.ws, &sess).map(|l| mark_prep(&mut sess, &l, &[pd::GROUP_ID.to_string()])).unwrap_or_default();
    tri!(save(&c.dir, &sess));
    spawn_candidates(&s, &c, prep);
    tri!(view(&c, &sess))
}

#[derive(Deserialize)]
struct FillBody {
    /// The result as the author says it (`0.74`, `Dice 0.74 ± 0.02`).
    #[serde(default)]
    result: String,
    /// Values per cell / per number mark.
    #[serde(default)]
    values: std::collections::BTreeMap<String, String>,
    direction: Direction,
}

/// 回填: the result came in. The item is filled and a 修訂 is made from it,
/// carrying the interview's explanation and every place and dependent; the
/// conversation and the plan go through 修訂 as for any request.
async fn fill_pending(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>, Json(b): Json<FillBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    if b.result.trim().is_empty() && b.values.values().all(|v| v.trim().is_empty()) {
        return bad("請填結果");
    }
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let Some(k) = sess.pending.iter().position(|x| x.id == id) else { return bad("沒有這個待補項目") };
    {
        let it = &mut sess.pending[k];
        it.result = b.result.trim().to_string();
        // values given before and not typed again stay
        for (k, v) in b.values {
            let v = v.trim().to_string();
            if v.is_empty() {
                it.values.remove(&k);
            } else {
                it.values.insert(k, v);
            }
        }
        it.direction = Some(b.direction);
        it.status = PendingStatus::Filled;
    }
    let prep = open_fill(&c, &mut sess, k);
    tri!(save(&c.dir, &sess));
    spawn_candidates(&s, &c, prep);
    tri!(view(&c, &sess))
}

/// The 修訂 of a filled item (made, or the open one updated); the record gets
/// the result. A conversation opens unless the result is as expected. Returns
/// the groups whose candidates to prepare.
fn open_fill(c: &Ctx, sess: &mut Session, k: usize) -> Vec<String> {
    let id = sess.pending[k].id.clone();
    // the record has the result now (the experiment's `?`, the table's cells)
    pd::write_back(sess, &id);
    let item = sess.pending[k].clone();
    let rep = pending_report(c, sess).into_iter().find(|e| e.id == id);
    // one 修訂 per item: filled again = the same request, updated
    let existing = item.revision.as_deref().and_then(|r| sess.revisions.iter().position(|x| x.id == r && x.status != RevisionStatus::Done));
    let gid = match existing {
        Some(at) => {
            let r = &mut sess.revisions[at];
            r.request = pd::request_text(&item, rep.as_ref());
            r.plan = None;
            r.status = RevisionStatus::Open;
            r.group.clone()
        }
        None => {
            let n = sess.revisions.len() + 1;
            let (rid, mut gid) = (format!("v{n}"), format!("v{n}"));
            while sess.group(&gid).is_some() {
                gid.push('x');
            }
            let section = rep.as_ref().and_then(|r| r.places.first()).map(|p| p.section.clone()).filter(|x| ds::SECTIONS.contains(&x.as_str())).unwrap_or_else(|| "Results".into());
            sess.groups.push(ds::Group { id: gid.clone(), section, title: format!("回填：{} {}", item.id, short(&item.what, 20)), purpose: pd::request_text(&item, rep.as_ref()), ..Default::default() });
            let iid = sess.new_item_id();
            sess.items.push(ds::Item { id: iid, group: gid.clone(), label: format!("{}：{}", item.id, short(&item.what, 50)), role: "revision".into(), ..Default::default() });
            sess.revisions.push(pd::revision_for(&item, &rid, &gid, rep.as_ref()));
            sess.pending[k].revision = Some(rid);
            gid
        }
    };
    pd::sync_group(sess);
    // as expected = numbers only, nothing to discuss; otherwise the conversation gets its options
    if item.direction == Some(Direction::Matches) {
        return vec![];
    }
    sess.start_group(&gid);
    lens_for(&c.ws, sess).map(|l| mark_prep(sess, &l, std::slice::from_ref(&gid))).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pasted_path_reads_a_file_or_a_folders_files() {
        let home = std::env::temp_dir().join(format!("pc-recpath-{}", std::process::id()));
        let dir = home.join("Shelf/Run 1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("log.md"), "E1 lr 1e-4, AUC 0.81").unwrap();
        std::fs::write(dir.join("看板 (Copy) - x.md"), "H1 想法").unwrap();
        std::fs::write(dir.join("table.csv"), "a,b\n1,2\n").unwrap();
        std::fs::write(dir.join("sheet.xlsx"), [0u8, 159, 1]).unwrap();
        std::fs::write(dir.join(".DS_Store"), [0u8]).unwrap();
        // ~/, quotes, a backslash-escaped space, file:// with %20 all name the same place
        for raw in ["~/Shelf/Run 1/log.md", "'~/Shelf/Run 1/log.md'", "~/Shelf/Run\\ 1/log.md"] {
            let p = record_path(raw, &home).unwrap();
            assert!(p.ends_with("log.md"), "{raw}");
        }
        let url = format!("file://{}", home.join("Shelf/Run%201/log.md").display());
        assert!(record_path(&url, &home).unwrap().ends_with("log.md"));
        // a folder: its files, the spreadsheet named as skipped, dot files ignored
        let (ok, skipped) = record_files_at(&record_path("~/Shelf/Run 1", &home).unwrap()).unwrap();
        assert_eq!(ok.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["log.md", "table.csv", "看板 (Copy) - x.md"]);
        // a pasted path with spaces, parentheses and Chinese, quoted the way Finder copies it
        let p = record_path("'~/Shelf/Run 1/看板 (Copy) - x.md'", &home).unwrap();
        assert_eq!(record_files_at(&p).unwrap().0.len(), 1);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].starts_with("sheet.xlsx"));
        // outside the home folder, relative, or missing: refused
        assert!(record_path("/etc/hosts", &home).is_err());
        assert!(record_path("Shelf/Run 1", &home).is_err());
        assert!(record_path("~/nope.md", &home).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_new_template_keeps_the_text_and_the_notation_checks() {
        use lore_notation::{check_document, CheckOptions, Document};
        let ws = std::env::temp_dir().join(format!("pc-draft-tpl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let tpl = ws.join("publishers/demo-conf");
        std::fs::create_dir_all(&tpl).unwrap();
        std::fs::write(tpl.join("template.toml"), "name = \"Demo Conference\"\nkind = \"conference\"\n").unwrap();
        std::fs::write(tpl.join("sample.tex"), "\\documentclass{article}\n\\usepackage{cite}\n\\begin{document}\n\\title{T}\\maketitle\n\\section{Introduction}\nSample.\n\\section{Method}\nSample.\n\\section{Results}\nSample.\n\\bibliography{refs}\n\\end{document}\n").unwrap();
        let dir = paper::create_paper(&ws.join("papers"), "P").unwrap();
        // another paper on the same template: the author's habit
        let other = paper::create_paper(&ws.join("papers"), "Q").unwrap();
        meta::write_meta(&other, &meta::PaperMeta { journal: Some("demo-conf".into()), ..Default::default() }).unwrap();
        std::fs::write(dir.join("notation.json"), r#"{"version":1,"entries":[{"id":"w","kind":"symbol","latex":"\\boldsymbol{w}","macro":"vw","meaning_en":"frame weight"},{"id":"t1","kind":"table","label":"tab:main","caption":"Main comparison"}]}"#).unwrap();
        std::fs::write(dir.join("notation.tex"), "\\newcommand{\\vw}{\\boldsymbol{w}}\n").unwrap();
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap().replace("\\usepackage{amsmath,amssymb}\n", "\\usepackage{amsmath,amssymb}\n\\input{notation}\n");
        std::fs::write(dir.join("main.tex"), main).unwrap();
        std::fs::write(dir.join("sections/result.tex"), "\\section{Result}\n\nTable~\\ref{tab:main} shows that $\\vw$ matters.\n\n\\begin{table}\n\\caption{Main comparison.}\n\\label{tab:main}\n\\begin{tabular}{l}\nx\n\\end{tabular}\n\\end{table}\n").unwrap();
        let checks = |d: &Path| {
            let mut files = vec![("main.tex".to_string(), std::fs::read_to_string(d.join("main.tex")).unwrap())];
            for f in paper::section_files(d) {
                files.push((format!("sections/{f}"), std::fs::read_to_string(d.join("sections").join(&f)).unwrap()));
            }
            let n = crate::notation::load(&ws, d);
            let mut codes: Vec<String> = check_document(Document::from_files(&files), &n, &CheckOptions::default()).issues.iter().map(|i| format!("{} {}", i.code, i.message)).collect();
            codes.sort();
            codes
        };
        let before = checks(&dir);
        assert!(!before.iter().any(|c| c.starts_with("ref_undefined") || c.starts_with("hardcoded_reference")), "{before:?}");
        let text = std::fs::read_to_string(dir.join("sections/result.tex")).unwrap();

        assert!(retemplate(&ws, &dir, Some("demo-conf"), None).unwrap());
        assert_eq!(meta::read_meta(&dir).journal.as_deref(), Some("demo-conf"));
        assert_eq!(std::fs::read_to_string(dir.join("sections/result.tex")).unwrap(), text);
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        assert!(main.contains("\\usepackage{cite}") && main.contains("\\input{notation}") && main.contains("\\input{sections/result}"), "{main}");
        assert_eq!(checks(&dir), before, "the registry's checks say the same after the change");
        let archived: Vec<_> = std::fs::read_dir(ds::draft_dir(&dir).join(ds::ARCHIVE_DIR)).unwrap().flatten().collect();
        assert!(archived[0].path().join("main.tex").is_file(), "the old main.tex is kept");
        assert!(!retemplate(&ws, &dir, Some("demo-conf"), None).unwrap(), "same template: nothing to do");
        assert!(retemplate(&ws, &dir, Some("../x"), None).is_err() && retemplate(&ws, &dir, Some("nope"), None).is_err());

        // the suggester sees the template, its sections and the other paper's habit
        let c = candidates(&ws, &dir);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].kind.as_str(), c[0].uses), ("conference", 1));
        assert_eq!(c[0].sections, ["Introduction", "Method", "Results"]);
        assert_eq!(venue::suggest(&Session::default(), &c, 3), ["demo-conf"]);

        // back to the plain article
        assert!(retemplate(&ws, &dir, None, None).unwrap());
        assert_eq!(meta::read_meta(&dir).journal, None);
        assert_eq!(checks(&dir), before);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn a_paper_set_up_by_hand_on_the_journal_class_only_records_the_journal() {
        let ws = std::env::temp_dir().join(format!("pc-draft-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let tpl = ws.join("publishers/jx");
        std::fs::create_dir_all(&tpl).unwrap();
        std::fs::write(tpl.join("template.toml"), "name = \"Journal X\"\n\n[limits]\nanonymous = true\n\n[submission]\nrequired_statements = [\"Funding\"]\n").unwrap();
        std::fs::write(tpl.join("sample.tex"), "%% \\documentclass{other}\n\\documentclass[pdflatex]{sn-jnl}\n\\begin{document}\n\\maketitle\n\\section{Introduction}\nSample.\n\\backmatter\n\\bmhead{Acknowledgements}\nAcknowledgements are not compulsory.\n\\bibliography{x}\n\\end{document}\n").unwrap();
        let dir = ws.join("papers/P");
        std::fs::create_dir_all(dir.join("sections")).unwrap();
        let own = "\\documentclass[pdflatex,sn-vancouver-num]{sn-jnl}\n\\newcommand{\\blinded}[2]{\\ifdefined\\blindmode #2\\else #1\\fi}\n\\begin{document}\n\\ifdefined\\blindmode\\else\n\\author{A}\n\\fi\n\\maketitle\n\\input{sections/01-introduction}\n\\backmatter\n\\bmhead{Funding}\n\\blinded{No funding.}{Removed.}\n\\bibliography{x}\n\\end{document}\n";
        std::fs::write(dir.join("main.tex"), own).unwrap();
        std::fs::write(dir.join("sections/01-introduction.tex"), "\\section{Introduction}\nOurs.\n").unwrap();
        assert!(retemplate(&ws, &dir, Some("jx"), None).unwrap());
        assert_eq!(meta::read_meta(&dir).journal.as_deref(), Some("jx"));
        assert_eq!(std::fs::read_to_string(dir.join("main.tex")).unwrap(), own, "his main.tex is not rebuilt");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn a_preparation_cut_off_shows_as_failed() {
        let dir = std::env::temp_dir().join(format!("pc-prep-{}", std::process::id()));
        let mut s = Session::default();
        s.groups.push(ds::Group { id: "g1".into(), prep: Some(Prep::Running), ..Default::default() });
        s.groups.push(ds::Group { id: "g2".into(), prep: Some(Prep::Running), ..Default::default() });
        PREPARING.lock().unwrap().push(prep_key(&dir, "g2"));
        settle_stale_prep(&dir, &mut s);
        assert_eq!(s.groups[0].prep, Some(Prep::Failed), "saved as running, no task here: the app quit");
        assert_eq!(s.groups[1].prep, Some(Prep::Running), "a task of this process is on it");
        PREPARING.lock().unwrap().retain(|k| k != &prep_key(&dir, "g2"));
    }

    #[test]
    fn model_picker_maps_to_refs() {
        assert_eq!(model_ref(None).unwrap(), None);
        assert_eq!(model_ref(Some("auto")).unwrap(), None);
        assert_eq!(model_ref(Some("local")).unwrap().as_deref(), Some("local:current"));
        assert_eq!(model_ref(Some("sonnet")).unwrap().as_deref(), Some("claude:sonnet"));
        assert!(model_ref(Some("gpt")).is_err());
        assert_eq!(bank_section("Methods"), "Method");
    }

    #[test]
    fn draft_tasks_route_as_intended() {
        let t = Llm::task_table(None).unwrap();
        assert_eq!(t.task["pc-draft-turn"].default, "claude:haiku");
        assert_eq!(t.task["pc-draft-turn"].fallback.as_deref(), Some("local:current"));
        assert_eq!(t.task["pc-draft-audit"].default, "claude:opus");
        assert_eq!(t.task["pc-draft-audit"].fallback.as_deref(), Some("local:current"), "without Claude the reviewer pass runs locally");
        assert_eq!(t.task["pc-draft-record"].default, "claude:opus", "reading tables out of a record is Opus work");
        assert_eq!(t.task["pc-draft-revise"].default, "claude:opus", "editing an existing paper is Opus work");
        assert_eq!(t.task["pc-draft-candidates"].default, "claude:opus", "預想選項 think on Opus, before the interview");
        for id in ["pc-draft-plan", "pc-draft-claims", "pc-draft-review", "pc-draft-outline"] {
            assert_eq!(t.task[id].default, "claude:sonnet", "{id}");
        }
    }
}
