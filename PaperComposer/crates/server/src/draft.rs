//! 討論草稿 routes: `/api/papers/{name}/draft/...`.
//!
//! The session is plain JSON in the paper folder (pc_core::draft::session).
//! Model calls run without holding the lock; the session is re-read after the
//! call and the result applied to the fresh copy, so a slow model never
//! blocks the window or loses an edit made meanwhile.

use super::*;
use pc_core::draft::lens::Lens;
use pc_core::draft::prompts as dp;
use pc_core::draft::session::{self as ds, Phase, Session, Who};
use pc_core::draft::style_text;

/// Serialises read-modify-write of draft sessions.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Record text sent to the model is cut here.
const RECORD_MAX_CHARS: usize = 120_000;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/papers/{name}/draft", get(get_draft))
        .route("/api/papers/{name}/draft/topic", put(put_topic))
        .route("/api/papers/{name}/draft/records", post(upload_record))
        .route("/api/papers/{name}/draft/records/{file}", axum::routing::delete(delete_record))
        .route("/api/papers/{name}/draft/record", post(run_record))
        .route("/api/papers/{name}/draft/experiments", put(put_experiments))
        .route("/api/papers/{name}/draft/plan", post(run_plan).put(put_plan))
        .route("/api/papers/{name}/draft/turn", post(run_turn))
        .route("/api/papers/{name}/draft/group/{id}", post(group_action))
        .route("/api/papers/{name}/draft/claims", post(run_claims).put(put_claims))
        .route("/api/papers/{name}/draft/review", post(run_review).put(put_review))
        .route("/api/papers/{name}/draft/outline", post(run_outline).put(put_outline))
        .route("/api/papers/{name}/draft/compose", post(run_compose))
        .route("/api/papers/{name}/draft/audit", post(run_audit).put(put_audit))
        .route("/api/papers/{name}/draft/audit/{id}/ask", post(audit_ask))
        .route("/api/papers/{name}/draft/phase", put(put_phase))
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

fn load(dir: &Path) -> Result<Session, Resp> {
    Session::load(dir).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e).into_response())
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
    }))
    .into_response())
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

/// Run a model task, retrying once when the reply cannot be parsed.
async fn ask_model<T>(s: &Shared, task: &str, model_ref: Option<&str>, prompt: &Prompt, schema: serde_json::Value, parse: impl Fn(&str) -> Result<T, String>) -> Result<T, Resp> {
    let mut extra = None;
    let mut last = String::new();
    for _ in 0..2 {
        let reply = match model_ref {
            Some(r) => s.llm.complete_on(task, r, prompt, extra, Some(schema.clone())).await,
            None => s.llm.complete(task, None, prompt, extra, Some(schema.clone())).await,
        };
        match reply {
            Ok(r) => match parse(&r) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last = e;
                    extra = Some("Reply with the JSON object only, in the shape given.");
                }
            },
            Err(e) => return Err(bad(&e)),
        }
    }
    Err(bad(&last))
}

// ── Step 1 ──────────────────────────────────────────────

#[derive(Deserialize)]
struct TopicBody {
    #[serde(default)]
    reader_accepts: String,
    #[serde(default)]
    twist: String,
    #[serde(default)]
    lens: Option<String>,
}

async fn put_topic(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<TopicBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
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

// ── Step 2 ──────────────────────────────────────────────

#[derive(Deserialize)]
struct RecordQuery {
    file: String,
}

/// Text records are stored as they are; a PDF is stored as its text.
async fn upload_record(State(s): State<Shared>, AxPath(name): AxPath<String>, AxQuery(q): AxQuery<RecordQuery>, body: axum::body::Bytes) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    if body.is_empty() {
        return bad("檔案是空的");
    }
    let lower = q.file.to_lowercase();
    let (file, data): (String, Vec<u8>) = if lower.ends_with(".pdf") {
        match pdf_extract::extract_text_from_mem(&body) {
            Ok(t) if !t.trim().is_empty() => (format!("{}.txt", q.file.trim_end_matches(".pdf").trim_end_matches(".PDF")), t.into_bytes()),
            _ => return bad("讀不到這份 PDF 的文字（可能是掃描檔）"),
        }
    } else if [".xlsx", ".xls", ".docx", ".doc", ".pptx", ".numbers", ".pages"].iter().any(|e| lower.ends_with(e)) {
        return bad("請另存成 CSV、Markdown 或純文字再上傳（試算表：檔案 → 下載 → CSV）");
    } else if std::str::from_utf8(&body).is_err() {
        return bad("只接受文字檔（.md .txt .csv .tsv .json .log）或 PDF");
    } else {
        (q.file.clone(), body.to_vec())
    };
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    let rec = match ds::save_record(&c.dir, &file, &data) {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    sess.records.push(rec);
    if sess.phase < Phase::Record {
        sess.phase = Phase::Record;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
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
    let text = ds::records_text(&c.dir, &sess.records, RECORD_MAX_CHARS);
    let prompt = dp::record_prompt(&c.key_value, &lens, &text);
    let r = tri!(ask_model(&s, "pc-draft-record", None, &prompt, dp::record_schema(), |x| dp::parse_record(x, &lens)).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.record_summary = r.summary;
    sess.experiments = r.experiments;
    sess.timeline = r.timeline;
    if sess.phase < Phase::Plan {
        sess.phase = Phase::Plan;
    }
    tri!(save(&c.dir, &sess));
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
    let sess = tri!(load(&c.dir));
    let lens = tri!(lens_for(&c.ws, &sess));
    let prompt = dp::plan_prompt(&c.key_value, &sess, &lens, &style_text(&c.ws));
    let groups = tri!(ask_model(&s, "pc-draft-plan", None, &prompt, dp::plan_schema(), |x| dp::parse_plan(x, &lens)).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    if sess.groups.iter().any(|g| g.turns > 0) {
        return bad("已經開始對話了；要重新規劃請先在題組清單裡調整");
    }
    sess.set_plan(groups);
    if sess.phase < Phase::Interview {
        sess.phase = Phase::Interview;
    }
    tri!(save(&c.dir, &sess));
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
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── Step 4 ──────────────────────────────────────────────

#[derive(Deserialize)]
struct TurnBody {
    group: String,
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
    let (prompt, lens) = {
        let _g = LOCK.lock().await;
        let mut sess = tri!(load(&c.dir));
        let lens = tri!(lens_for(&c.ws, &sess));
        if sess.group(&b.group).is_none() {
            return bad("沒有這個題組");
        }
        let turns = ds::read_turns(&c.dir);
        let prompt = dp::turn_prompt(&c.key_value, &sess, &lens, &b.group, &turns, answer.as_deref());
        // The answer is kept even if the model call fails; sending it again
        // after a failure does not record it twice.
        let resent = answer.as_ref().is_some_and(|a| turns.iter().rev().find(|t| t.group == b.group).is_some_and(|t| t.who == Who::Me && &t.text == a));
        if let Some(a) = answer.as_ref().filter(|_| !resent) {
            if let Err(e) = ds::append_turn(&c.dir, &b.group, Who::Me, a, vec![]) {
                return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
            }
            if let Some(g) = sess.group_mut(&b.group) {
                g.turns += 1;
            }
        }
        sess.start_group(&b.group);
        if sess.phase < Phase::Interview {
            sess.phase = Phase::Interview;
        }
        tri!(save(&c.dir, &sess));
        (prompt, lens)
    };
    let r = tri!(ask_model(&s, "pc-draft-turn", mref.as_deref(), &prompt, dp::turn_schema(), dp::parse_turn).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.apply(&lens, &b.group, r.updates);
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
    if !say.is_empty() {
        if let Err(e) = ds::append_turn(&c.dir, &b.group, Who::Ai, &say, r.options) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    }
    tri!(save(&c.dir, &sess));
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
    match b.action.as_str() {
        "start" | "reopen" => {
            sess.start_group(&id);
        }
        "close" => sess.close_group(&id, false),
        "skip" => sess.close_group(&id, true),
        _ => return bad("unknown action"),
    }
    if sess.interview_finished() && sess.phase == Phase::Interview {
        sess.phase = Phase::Claims;
    }
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

// ── Step 4.5 and 5 ──────────────────────────────────────

async fn run_claims(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    let lens = tri!(lens_for(&c.ws, &sess));
    let prompt = dp::claims_prompt(&c.key_value, &sess, &lens);
    let claims = tri!(ask_model(&s, "pc-draft-claims", None, &prompt, dp::claims_schema(), dp::parse_claims).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.claims = claims;
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
    let prompt = dp::review_prompt(&c.key_value, &sess, &lens);
    let review = tri!(ask_model(&s, "pc-draft-review", None, &prompt, dp::review_schema(), dp::parse_review).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.review = Some(review);
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
    let prompt = dp::outline_prompt(&c.key_value, &sess, &lens, &style_text(&c.ws), only);
    let new = tri!(ask_model(&s, "pc-draft-outline", None, &prompt, dp::outline_schema(), dp::parse_outline).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    match only {
        Some((sec, _)) => dp::replace_section(&mut sess.outline, sec, new),
        None => {
            sess.outline = new;
            dp::number_paras(&mut sess.outline);
        }
    }
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

/// Draft one section paragraph by paragraph through the phrase bank (the
/// same tone lock as 寫作), with the outline's unit plan, the claim register
/// and the author's style notes as extra instructions.
async fn run_compose(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<ComposeDraftBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    if !ds::SECTIONS.contains(&b.section.as_str()) {
        return bad("未知的章節");
    }
    let sess = tri!(load(&c.dir));
    let paras: Vec<ds::OutlinePara> = sess.outline.iter().filter(|o| o.section == b.section).flat_map(|o| o.paras.clone()).collect();
    if paras.is_empty() {
        return bad("這個章節還沒有大綱");
    }
    if !has_writing_engine(&s, b.engine).await {
        return bad("沒有可用的寫作引擎");
    }
    let style = style_text(&c.ws);
    let mut out: Vec<ds::DraftPara> = vec![];
    for p in &paras {
        let points = if p.en.is_empty() { p.zh.clone() } else { p.en.iter().map(|x| format!("- {x}")).collect::<Vec<_>>().join("\n") };
        let req = ComposeRequest {
            points,
            section: bank_section(&b.section).into(),
            function: None,
            min_words: 60,
            max_words: 160,
            language: "en".into(),
            before: out.last().map(|d| d.text.clone()),
            after: None,
            key_value: Some(c.key_value.clone()).filter(|k| !k.is_empty()),
        };
        let (pts, refs, prompt) = {
            let l = s.loaded.read().await;
            let Some(prof) = &l.profile else { return (StatusCode::SERVICE_UNAVAILABLE, "profile not loaded").into_response() };
            let pts = compose::parse_points(&req.points);
            let refs = compose::reference_phrases(prof, &req, &pts);
            let prompt = compose::build_prompt(prof, &req, &pts, &refs);
            (pts, refs, prompt)
        };
        let earlier: Vec<String> = out.iter().map(|d| d.text.clone()).collect();
        let extra = dp::compose_extra(p, &sess, &b.section, &style, &earlier);
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
                    let text = d.sentences.iter().map(|x| x.text.trim()).collect::<Vec<_>>().join(" ");
                    if check.ok() || done.is_some() {
                        done = Some(ds::DraftPara { para: p.id.clone(), text, problems: check.problems });
                        break;
                    }
                    feedback = format!("{extra}\nYour previous answer had problems: {}. Fix them.", check.problems.join("; "));
                    done = Some(ds::DraftPara { para: p.id.clone(), text, problems: check.problems });
                }
                Err(e) => {
                    err = e;
                    feedback = format!("{extra}\nReply with the JSON object only.");
                }
            }
        }
        match done {
            Some(d) => out.push(d),
            None => return bad(&format!("{}：{err}", p.id)),
        }
    }
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.drafts.retain(|d| d.section != b.section);
    sess.drafts.push(ds::DraftSection { section: b.section.clone(), paras: out });
    sess.drafts.sort_by_key(|d| ds::SECTIONS.iter().position(|x| *x == d.section));
    if sess.phase < Phase::Draft {
        sess.phase = Phase::Draft;
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
    let prompt = dp::audit_prompt(&c.key_value, &sess, &lens, &style_text(&c.ws));
    let findings = tri!(ask_model(&s, "pc-draft-audit", None, &prompt, dp::audit_schema(), dp::parse_audit).await);
    let _g = LOCK.lock().await;
    let mut sess = tri!(load(&c.dir));
    sess.audit = findings;
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
    tri!(save(&c.dir, &sess));
    tri!(view(&c, &sess))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(t.task["pc-draft-audit"].fallback.is_none(), "the reviewer pass never falls back");
        for id in ["pc-draft-record", "pc-draft-plan", "pc-draft-claims", "pc-draft-review", "pc-draft-outline"] {
            assert_eq!(t.task[id].default, "claude:sonnet", "{id}");
        }
    }
}
