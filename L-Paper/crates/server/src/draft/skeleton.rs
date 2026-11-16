//! 骨架 routes: `/api/papers/{name}/draft/skeleton...` (contract: `docs/skeleton-api.md`).
//!
//! Every write goes through [`with_sk`]: the lock, a fresh load, a change by id, `rev` up when
//! anything changed, save. Background tasks (plan, fill, write, repair) never hold a session
//! across a model call: they read what they need, call the model, then patch the fresh session
//! by id (a paragraph moved meanwhile is found where it is now; one removed drops the result).
//! What runs is kept in [`LIVE`]: a `busy` paragraph or a `job` saved by a task that is not
//! running any more (the app quit) is settled when the skeleton is read.

use super::*;
use pc_core::draft::skeleton::{self as sk, inventory, plan as skp, Skeleton};
use std::collections::BTreeSet;

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/papers/{name}/draft/skeleton", get(get_view))
        .route("/api/papers/{name}/draft/skeleton/inventory", post(post_inventory))
        .route("/api/papers/{name}/draft/skeleton/plan", post(post_plan))
        .route("/api/papers/{name}/draft/skeleton/ops", post(post_ops))
        .route("/api/papers/{name}/draft/skeleton/save", post(post_save))
        .route("/api/papers/{name}/draft/skeleton/change", post(post_change))
        .route("/api/papers/{name}/draft/skeleton/convert", post(post_convert))
        .route("/api/papers/{name}/draft/skeleton/answer", post(post_answer))
        .route("/api/papers/{name}/draft/skeleton/note", post(post_note))
        .route("/api/papers/{name}/draft/skeleton/write", post(post_write))
        .route("/api/papers/{name}/draft/skeleton/refine", post(post_refine))
        .route("/api/papers/{name}/draft/skeleton/style", get(get_style))
}

// ── what runs ────────────────────────────────────────────

/// `<paper dir>#<what>` for every background task of this process (`job`, `para:p9`, `fill:q2`).
static LIVE: std::sync::Mutex<BTreeSet<String>> = std::sync::Mutex::new(BTreeSet::new());

fn live_key(dir: &Path, what: &str) -> String {
    format!("{}#{what}", dir.display())
}

fn is_live(dir: &Path, what: &str) -> bool {
    LIVE.lock().is_ok_and(|v| v.contains(&live_key(dir, what)))
}

/// Held while a task runs; dropped (also on a panic) it is not live any more.
pub(super) struct Held(String);

impl Drop for Held {
    fn drop(&mut self) {
        if let Ok(mut v) = LIVE.lock() {
            v.remove(&self.0);
        }
    }
}

/// Mark `what` as running; None when it already runs.
fn hold(dir: &Path, what: &str) -> Option<Held> {
    let k = live_key(dir, what);
    let mut v = LIVE.lock().ok()?;
    if !v.insert(k.clone()) {
        return None;
    }
    Some(Held(k))
}

/// A task saved as running that this process does not run: cut short. Returns the answered
/// threads whose fill-in has to run again.
fn settle(dir: &Path, k: &mut Skeleton) -> Vec<String> {
    if let Some(j) = k.job.as_mut().filter(|j| j.error.is_empty()) {
        if !is_live(dir, "job") {
            j.error = "上次沒有做完（App 關掉了）".into();
        }
    }
    for p in k.paras_mut() {
        if p.busy.is_some() && !is_live(dir, &format!("para:{}", p.id)) {
            p.busy = None;
        }
    }
    k.threads.iter().filter(|t| t.status == "answered" && !is_live(dir, &format!("fill:{}", t.id))).map(|t| t.id.clone()).collect()
}

// ── load, change, save ───────────────────────────────────

/// Lock, load, change the session (its skeleton is made when missing), and when the skeleton
/// changed: `rev` up, save. The closure's error leaves the file as it was.
pub(super) async fn with_sk<T>(dir: &Path, f: impl FnOnce(&mut Session) -> Result<T, String>) -> Result<(Session, T), Resp> {
    let _g = LOCK.lock().await;
    let mut sess = load(dir)?;
    let before = sess.skeleton.clone();
    let out = f(&mut sess).map_err(|e| bad(&e))?;
    if sess.skeleton != before {
        let old_rev = before.as_ref().map(|k| k.rev).unwrap_or(0);
        if let Some(k) = sess.skeleton.as_mut() {
            if k.rev <= old_rev {
                k.rev = old_rev + 1;
            }
        }
        save(dir, &sess)?;
    }
    Ok((sess, out))
}

/// The registry's numbered equations (label, what it is) for the inventory.
fn equations(c: &Ctx) -> Vec<(String, String)> {
    crate::notation::load(&c.ws, &c.dir)
        .entries
        .into_iter()
        .filter(|e| e.kind == Some(lore_notation::Kind::Equation))
        .filter_map(|e| {
            let label = e.label.clone().filter(|l| !l.trim().is_empty())?;
            let what = [e.meaning.clone(), e.caption.clone(), e.meaning_en.clone()].into_iter().flatten().find(|x| !x.trim().is_empty()).unwrap_or_default();
            Some((label, what))
        })
        .collect()
}

/// The page's view: the stored skeleton, or (none yet) the inventory the session gives now.
fn view_of(c: &Ctx, sess: &Session) -> sk::View {
    let targets = topaper::venue_of(c).profile.as_ref().map(inventory::targets);
    let can_convert = sk::convert::can_convert(sess);
    match &sess.skeleton {
        Some(k) => k.view(can_convert, targets),
        None => {
            let mut k = Skeleton::default();
            inventory::merge(&mut k, inventory::fresh(sess, &equations(c)));
            k.view(can_convert, targets)
        }
    }
}

fn reply(c: &Ctx, sess: &Session) -> Resp {
    Json(view_of(c, sess)).into_response()
}

/// Read: a stored skeleton gets the record's new inventory items and is settled (both only
/// written when something changed, so two reads with nothing in between give the same `rev`).
async fn read(s: &Shared, c: &Ctx) -> Result<Session, Resp> {
    let has = load(&c.dir)?.skeleton.is_some();
    if !has {
        return load(&c.dir);
    }
    let eqs = equations(c);
    let dir = c.dir.clone();
    let (sess, refill) = with_sk(&c.dir, |sess| {
        let fresh = inventory::fresh(sess, &eqs);
        let k = sess.skeleton.get_or_insert_with(Default::default);
        inventory::merge(k, fresh);
        Ok(settle(&dir, k))
    })
    .await?;
    for t in refill {
        spawn_fill(s, c, t);
    }
    Ok(sess)
}

async fn get_view(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(read(&s, &c).await);
    reply(&c, &sess)
}

// ── 素材清單 ─────────────────────────────────────────────

#[derive(Deserialize)]
struct InventoryBody {
    #[serde(default)]
    keep: Vec<String>,
    #[serde(default)]
    add: Vec<inventory::Added>,
}

async fn post_inventory(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<InventoryBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let eqs = equations(&c);
    let (sess, _) = tri!(
        with_sk(&c.dir, |sess| {
            let fresh = inventory::fresh(sess, &eqs);
            let k = sess.skeleton.get_or_insert_with(Default::default);
            inventory::merge(k, fresh);
            k.job = k.job.take().filter(|j| j.error.is_empty());
            inventory::choose(k, &b.keep, b.add)
        })
        .await
    );
    reply(&c, &sess)
}


// ── jobs ─────────────────────────────────────────────────

fn conflict(msg: &str) -> Resp {
    (StatusCode::CONFLICT, msg.to_string()).into_response()
}

fn job(kind: &str, label: &str, total: usize) -> sk::Job {
    sk::Job { kind: kind.into(), label: label.into(), done: 0, total, error: String::new() }
}

/// The job ends: gone when it worked, its error kept (shown until the next action) when not.
async fn end_job(dir: &Path, r: Result<(), String>) {
    let _ = with_sk(dir, |sess| {
        if let Some(k) = sess.skeleton.as_mut() {
            k.job = match r {
                Ok(()) => None,
                Err(e) => k.job.take().map(|j| sk::Job { error: if e.is_empty() { "沒有完成".into() } else { e }, ..j }),
            };
        }
        Ok(())
    })
    .await;
}

/// A finished job's error is cleared by the next action.
fn clear_failed(k: &mut Skeleton) {
    k.job = k.job.take().filter(|j| j.error.is_empty());
}

// ── 排骨架 ───────────────────────────────────────────────

/// The planner's prompt and what its reply is checked against, from the session as it is now.
/// `answers` = the interview so far (converting a session that has no outline yet).
fn plan_input(c: &Ctx, sess: &Session, answers: bool) -> (Prompt, skp::Known) {
    let (mat, mtext) = material_of(c);
    let k = sess.skeleton.clone().unwrap_or_default();
    let context = dp::context(&c.key_value, sess, &mtext);
    let turns = ds::read_turns(&c.dir);
    let answers_text = if answers { sk::convert::answers_block(sess, &turns) } else { String::new() };
    let style = style_text(&c.ws);
    let venue = topaper::venue_of(c).outline();
    let inv = inventory::block(&k);
    let prompt = skp::prompt(&skp::Input { context: &context, inventory: &inv, style: &style, venue: &venue, answers: &answers_text });
    let known = skp::Known {
        labels: k.kept_labels().into_iter().collect(),
        cites: ref_keys(&mat).into_iter().collect(),
        sources: ca::known_ids(sess),
        pending: sess.pending.iter().map(|p| p.id.clone()).collect(),
        groups: if answers { sk::convert::group_threads(sess, &turns).into_iter().map(|(g, _)| g).collect() } else { Default::default() },
    };
    (prompt, known)
}

/// Plan (or, `convert`, plan with the interview's answers and hang them on the paragraphs) on a
/// task of its own. The job is marked and held by the caller.
fn spawn_plan(s: &Shared, c: &Ctx, held: Held, convert: bool) {
    let (s2, c2) = (s.clone(), own(c));
    tokio::spawn(async move {
        let _held = held;
        let r = async {
            let sess = load(&c2.dir).map_err(|_| "讀不到 session".to_string())?;
            let (prompt, known) = plan_input(&c2, &sess, convert);
            let base = sess.skeleton.clone().unwrap_or_default();
            // checked on a copy (a reply that does not pass is asked again); applied below on the fresh one
            let (reply, _used) = engine::structured(&engines(&s2), skp::TASK, &prompt, skp::schema(), |x| {
                let mut k = base.clone();
                skp::parse(x, &known, &mut k).map(|_| x.to_string())
            })
            .await?;
            let turns = ds::read_turns(&c2.dir);
            with_sk(&c2.dir, |sess| {
                let snapshot = sess.clone();
                let k = sess.skeleton.get_or_insert_with(Default::default);
                let planned = skp::parse(&reply, &known, k)?;
                let uses = planned.uses.clone();
                skp::install(k, planned);
                if convert {
                    sk::convert::attach_answers(k, &snapshot, &turns, &uses);
                }
                Ok(())
            })
            .await
            .map_err(|_| "存不進骨架".to_string())?;
            Ok::<(), String>(())
        }
        .await;
        if let Err(e) = &r {
            eprintln!("排骨架: {e}");
        }
        end_job(&c2.dir, r).await;
    });
}

#[derive(Deserialize, Default)]
struct PlanBody {
    #[serde(default)]
    redo: bool,
}

async fn post_plan(State(s): State<Shared>, AxPath(name): AxPath<String>, body: Option<Json<PlanBody>>) -> Resp {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let c = tri!(ctx(&s, &name).await);
    let Some(held) = hold(&c.dir, "job") else { return conflict("還有工作在進行") };
    let exists = tri!(load(&c.dir)).skeleton.as_ref().is_some_and(|k| k.exists());
    if exists && !b.redo {
        return bad("已經有骨架了");
    }
    if exists {
        // 重排: the skeleton as it was goes to draft/archive/ first
        tri!(ds::snapshot(&c.dir).map_err(|e| bad(&e)));
    }
    let eqs = equations(&c);
    let (sess, _) = tri!(
        with_sk(&c.dir, |sess| {
            let fresh = inventory::fresh(sess, &eqs);
            let k = sess.skeleton.get_or_insert_with(Default::default);
            inventory::merge(k, fresh);
            k.job = Some(job("plan", "排骨架", 1));
            Ok(())
        })
        .await
    );
    spawn_plan(&s, &c, held, false);
    reply(&c, &sess)
}

/// 改用骨架: an old session → a skeleton (a copy archived first). With an outline: at once; in
/// the middle of the interview: planned afresh with the answers, then the answers hang on it.
async fn post_convert(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let Some(held) = hold(&c.dir, "job") else { return conflict("還有工作在進行") };
    let sess = tri!(load(&c.dir));
    if !sk::convert::can_convert(&sess) {
        return bad("沒有可以轉換的內容");
    }
    tri!(ds::snapshot(&c.dir).map_err(|e| bad(&e)));
    let turns = ds::read_turns(&c.dir);
    let eqs = equations(&c);
    let (sess, later) = tri!(
        with_sk(&c.dir, |sess| {
            let fresh = inventory::fresh(sess, &eqs);
            let converted = sk::convert::from_session(sess, &turns);
            let k = sess.skeleton.get_or_insert_with(Default::default);
            inventory::merge(k, fresh);
            match converted {
                Some(new) => {
                    k.sections = new.sections;
                    k.threads = new.threads;
                    k.undo.clear();
                    k.edits = 0;
                    k.sync_counters();
                    for (p, n) in &new.counters {
                        let c = k.counters.entry(p.clone()).or_insert(0);
                        *c = (*c).max(*n);
                    }
                    Ok(false)
                }
                None => {
                    k.job = Some(job("convert", "改用骨架", 1));
                    Ok(true)
                }
            }
        })
        .await
    );
    if later {
        spawn_plan(&s, &c, held, true);
    }
    reply(&c, &sess)
}

// ── edits ────────────────────────────────────────────────

#[derive(Deserialize)]
struct OpsBody {
    ops: Vec<sk::Op>,
}

/// The page's edits, in order, all or none; never a model call.
async fn post_ops(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<OpsBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let (sess, _) = tri!(
        with_sk(&c.dir, |sess| {
            let k = sess.skeleton.as_mut().filter(|k| k.exists()).ok_or("還沒有骨架")?;
            clear_failed(k);
            k.apply_all(b.ops)
        })
        .await
    );
    reply(&c, &sess)
}

#[derive(Deserialize)]
struct ChangeBody {
    thread: String,
    change: String,
    accept: bool,
}

async fn post_change(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<ChangeBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let (sess, _) = tri!(
        with_sk(&c.dir, |sess| {
            let k = sess.skeleton.as_mut().ok_or("還沒有骨架")?;
            clear_failed(k);
            k.decide(&b.thread, &b.change, b.accept)
        })
        .await
    );
    reply(&c, &sess)
}

/// 暫存: the touched paragraphs and their neighbours repaired in the background.
async fn post_save(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let Some(held) = hold(&c.dir, "job") else { return conflict("還有工作在進行") };
    let (sess, scope) = tri!(
        with_sk(&c.dir, |sess| {
            let k = sess.skeleton.as_mut().filter(|k| k.exists()).ok_or("還沒有骨架")?;
            clear_failed(k);
            let (paras, blocks) = k.repair_scope();
            if paras.is_empty() && blocks.is_empty() {
                k.clear_marks();
                return Ok(None);
            }
            for p in k.paras_mut().filter(|p| paras.contains(&p.id)) {
                p.busy = Some("repair".into());
            }
            k.job = Some(job("save", "暫存", 1));
            Ok(Some((paras, blocks)))
        })
        .await
    );
    let Some((paras, blocks)) = scope else { return reply(&c, &sess) };
    let holds: Vec<Held> = paras.iter().filter_map(|p| hold(&c.dir, &format!("para:{p}"))).collect();
    let (s2, c2) = (s.clone(), own(&c));
    let k0 = sess.skeleton.clone().unwrap_or_default();
    tokio::spawn(async move {
        let _held = (held, holds);
        let prompt = skp::repair_prompt(&c2.key_value, &k0, &paras, &blocks);
        let r = engine::structured(&engines(&s2), skp::REPAIR_TASK, &prompt, skp::repair_schema(), skp::parse_repair).await;
        let failed = r.as_ref().err().cloned();
        let saved = with_sk(&c2.dir, |sess| {
            let Some(k) = sess.skeleton.as_mut() else { return Ok(()) };
            for p in k.paras_mut().filter(|p| p.busy.as_deref() == Some("repair")) {
                p.busy = None;
            }
            if let Ok((reply, _)) = r {
                // paragraphs removed meanwhile are out of the scope
                let paras: Vec<String> = paras.iter().filter(|p| k.para(p).is_some()).cloned().collect();
                let blocks: Vec<String> = blocks.iter().filter(|b| k.block(b).is_some()).cloned().collect();
                skp::apply_repair(k, reply, &paras, &blocks);
            }
            Ok(())
        })
        .await;
        let res = match (failed, saved) {
            (Some(e), _) => Err(e),
            (None, Err(_)) => Err("存不進修補".to_string()),
            (None, Ok(_)) => Ok(()),
        };
        end_job(&c2.dir, res).await;
    });
    reply(&c, &sess)
}

// ── 補肉 ─────────────────────────────────────────────────

/// The paragraph an answer goes into: the anchor, or a block's first paragraph.
fn fill_target(k: &Skeleton, anchor: &str) -> Option<String> {
    if k.para(anchor).is_some() {
        return Some(anchor.to_string());
    }
    k.block(anchor).and_then(|b| b.paras.first()).map(|p| p.id.clone())
}

/// The fill-in of an answered thread on a task of its own: the reply is applied to the session
/// as it is then (the paragraph found by id). A failed call still closes the thread with the
/// answer as said (never asked again, never polled forever).
fn spawn_fill(s: &Shared, c: &Ctx, thread: String) {
    let Some(held) = hold(&c.dir, &format!("fill:{thread}")) else { return };
    let (s2, c2) = (s.clone(), own(c));
    tokio::spawn(async move {
        let _held = held;
        let Ok(sess) = load(&c2.dir) else { return };
        let Some(k) = sess.skeleton.clone() else { return };
        let Some(t) = k.thread(&thread).cloned() else { return };
        let target = fill_target(&k, &t.anchor);
        let _para = target.as_ref().and_then(|p| hold(&c2.dir, &format!("para:{p}")));
        let record = pc_core::draft::fallback::cut(&ds::records_text(&c2.dir, &sess.records, usize::MAX), 20_000);
        let prompt = skp::fill_prompt(&c2.key_value, &k, &t, &record);
        let r = engine::structured(&engines(&s2), skp::FILL_TASK, &prompt, skp::fill_schema(), skp::parse_fill).await;
        if let Err(e) = &r {
            eprintln!("要點回填 {thread}: {e}");
        }
        let guard_on = crate::workspace_guard(&s2).await.active();
        let ids = ca::known_ids(&sess);
        let _ = with_sk(&c2.dir, |sess| {
            let Some(k) = sess.skeleton.as_mut() else { return Ok(()) };
            if k.thread(&thread).is_none_or(|t| t.status != "answered") {
                return Ok(());
            }
            let mut reply = r.map(|(x, _)| x).unwrap_or_default();
            let notes = std::mem::take(&mut reply.writing_notes);
            let at = k.thread(&thread).filter(|t| !k.orphaned.contains(&t.id)).and_then(|t| fill_target(k, &t.anchor));
            // the paragraph where it is now (its busy mark cleared); removed meanwhile: dropped
            skp::apply_fill(k, &thread, reply, &ids);
            // how to write, said in the answer: 寫法筆記 on that paragraph
            if let Some(pid) = at {
                let section = k.section_of(&pid).map(|s| sk::old_section(s).to_string()).unwrap_or_default();
                if !notes.is_empty() {
                    pc_core::draft::notes::add_in(sess, &pid, &section, notes, guard_on);
                }
            }
            Ok(())
        })
        .await;
    });
}

#[derive(Deserialize)]
struct AnswerBody {
    thread: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    picked: Vec<String>,
    #[serde(default)]
    skip: bool,
}

/// An answer: at once the thread is answered and `next` moves on; the fill-in runs behind.
async fn post_answer(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<AnswerBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let (sess, fill) = tri!(
        with_sk(&c.dir, |sess| {
            let k = sess.skeleton.as_mut().ok_or("還沒有骨架")?;
            clear_failed(k);
            let t = k.thread(&b.thread).cloned().ok_or("沒有這一題")?;
            if t.status != "open" {
                return Err("這一題已經回答過了".into());
            }
            if b.skip {
                k.thread_mut(&b.thread).expect("checked").status = "skipped".into();
                return Ok(false);
            }
            // the options picked, then what was typed or said
            let mut said: Vec<String> = b.picked.iter().filter_map(|o| t.options.iter().find(|x| &x.id == o).map(|x| x.text.clone())).collect();
            if !b.text.trim().is_empty() {
                said.push(b.text.trim().to_string());
            }
            if said.is_empty() {
                return Err("先選一個或說說看".into());
            }
            let target = fill_target(k, &t.anchor);
            let x = k.thread_mut(&b.thread).expect("checked");
            x.messages.push(sk::Msg { who: "me".into(), text: said.join("。") });
            x.status = "answered".into();
            if let Some(p) = target.and_then(|p| k.para_mut(&p)) {
                p.busy = Some("fill".into());
            }
            Ok(true)
        })
        .await
    );
    if fill {
        spawn_fill(&s, &c, b.thread.clone());
    }
    reply(&c, &sess)
}

#[derive(Deserialize)]
struct NoteBody {
    anchor: String,
    text: String,
}

/// Something the author says about a paragraph or a block unasked: a `note` thread, folded in
/// like an answer.
async fn post_note(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<NoteBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    if b.text.trim().is_empty() {
        return bad("說說看");
    }
    let (sess, id) = tri!(
        with_sk(&c.dir, |sess| {
            let k = sess.skeleton.as_mut().filter(|k| k.exists()).ok_or("還沒有骨架")?;
            clear_failed(k);
            if !k.has(&b.anchor) {
                return Err("沒有這一段或這個框".into());
            }
            let target = fill_target(k, &b.anchor);
            let id = k.add_thread(sk::Thread { anchor: b.anchor.clone(), kind: "note".into(), status: "answered".into(), messages: vec![sk::Msg { who: "me".into(), text: b.text.trim().to_string() }], ..Default::default() });
            if let Some(p) = target.and_then(|p| k.para_mut(&p)) {
                p.busy = Some("fill".into());
            }
            Ok(id)
        })
        .await
    );
    spawn_fill(&s, &c, id);
    reply(&c, &sess)
}

// ── 文字 ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct WriteBody {
    id: String,
}

/// Write the text of a paragraph (again), or of every paragraph without text in a block, a
/// section or the whole paper, in the background: sections side by side, the paragraphs of a
/// section in order (each sees the ones before it), each landing as soon as it is written.
async fn post_write(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<WriteBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    if !has_writing_engine(&s, None).await {
        return bad("沒有可用的寫作引擎");
    }
    if s.loaded.read().await.profile.is_none() {
        return (StatusCode::SERVICE_UNAVAILABLE, "profile not loaded").into_response();
    }
    let Some(held) = hold(&c.dir, "job") else { return conflict("還有工作在進行") };
    let (sess, ids) = tri!(
        with_sk(&c.dir, |sess| {
            let k = sess.skeleton.as_mut().filter(|k| k.exists()).ok_or("還沒有骨架")?;
            clear_failed(k);
            let ids = skp::to_write(k, &b.id)?;
            if ids.is_empty() {
                return Err("沒有要寫的段落（已經有文字的段落請一段一段重寫）".into());
            }
            for p in k.paras_mut().filter(|p| ids.contains(&p.id)) {
                p.busy = Some("write".into());
            }
            k.job = Some(job("write", "寫文字", ids.len()));
            Ok(ids)
        })
        .await
    );
    let holds: Vec<Held> = ids.iter().filter_map(|p| hold(&c.dir, &format!("para:{p}"))).collect();
    // by section, in reading order
    let k0 = sess.skeleton.clone().unwrap_or_default();
    let mut by_section: Vec<(String, Vec<String>)> = vec![];
    for sec in &k0.sections {
        let mine: Vec<String> = sec.blocks.iter().flat_map(|b| b.paras.iter()).filter(|p| ids.contains(&p.id)).map(|p| p.id.clone()).collect();
        if !mine.is_empty() {
            by_section.push((sec.id.clone(), mine));
        }
    }
    let (s2, c2) = (s.clone(), own(&c));
    tokio::spawn(async move {
        let _held = (held, holds);
        use futures_util::StreamExt;
        let (sr, cr) = (&s2, &c2);
        let errors: Vec<String> = futures_util::stream::iter(by_section.into_iter().map(|(sec, ids)| async move { write_section(sr, cr, &sec, &ids).await }))
            .buffer_unordered(3)
            .collect::<Vec<Vec<String>>>()
            .await
            .into_iter()
            .flatten()
            .collect();
        // whatever did not land is not busy any more
        let _ = with_sk(&c2.dir, |sess| {
            if let Some(k) = sess.skeleton.as_mut() {
                for p in k.paras_mut().filter(|p| p.busy.as_deref() == Some("write")) {
                    p.busy = None;
                }
            }
            Ok(())
        })
        .await;
        end_job(&c2.dir, if errors.is_empty() { Ok(()) } else { Err(errors.join("；")) }).await;
    });
    reply(&c, &sess)
}

/// The paragraphs `ids` of section `sec`, one after the other; then the program's jump check on
/// the blocks written (the repair task says why and how to fix, one call). Returns the errors.
async fn write_section(s: &Shared, c: &Ctx, _sec: &str, ids: &[String]) -> Vec<String> {
    let mut errors = vec![];
    let mut blocks: Vec<String> = vec![];
    for id in ids {
        let Ok(sess) = load(&c.dir) else { break };
        let Some(k) = sess.skeleton.clone() else { break };
        // moved or removed meanwhile: written where it is now, or not at all
        let (Some(p), Some(section)) = (k.para(id).cloned(), k.section_of(id).cloned()) else { continue };
        let section_paras: Vec<&sk::Para> = section.blocks.iter().flat_map(|b| b.paras.iter()).collect();
        let at = section_paras.iter().position(|x| x.id == *id).unwrap_or(0);
        let earlier: Vec<String> = section_paras[..at].iter().map(|x| x.text.clone()).filter(|t| !t.trim().is_empty()).collect();
        let mut env = super::compose_env(s, c, &sess, sk::old_section(&section), None).await;
        // the other sections' text, for the notation checks
        env.other_drafts = k.sections.iter().filter(|x| x.id != section.id).map(|x| x.blocks.iter().flat_map(|b| b.paras.iter()).map(|p| p.text.clone()).collect::<Vec<_>>().join("\n\n")).collect();
        let more = skp::write_more(&k, id);
        let r = super::compose_one(s, c, &sess, &env, &skp::outline_para(&p), at, section_paras.len(), &earlier, &more).await;
        if r.is_err() {
            errors.push(format!("{id}：{}", r.as_ref().err().cloned().unwrap_or_default()));
        }
        let _ = with_sk(&c.dir, |sess| {
            let Some(k) = sess.skeleton.as_mut() else { return Ok(()) };
            let block = k.block_of(id).map(|b| b.id.clone());
            if let Some(x) = k.para_mut(id) {
                x.busy = None;
                if let Ok((d, _)) = &r {
                    x.text = d.text.clone();
                    x.problems = d.problems.clone();
                    x.state = sk::State::Text;
                    x.jump = None;
                }
            }
            if let Some(j) = k.job.as_mut() {
                j.done += 1;
            }
            if let Some(b) = block.filter(|b| !blocks.contains(b)) {
                blocks.push(b);
            }
            Ok(())
        })
        .await;
    }
    // too far from the paragraph before: the program finds them, the model only says why
    let k = load(&c.dir).ok().and_then(|x| x.skeleton);
    let Some(k) = k else { return errors };
    let cands: Vec<String> = blocks.iter().flat_map(|b| skp::jump_candidates(&k, b)).collect();
    if !cands.is_empty() {
        let prompt = skp::jump_prompt(&k, &cands);
        if let Ok((r, _)) = engine::structured(&engines(s), skp::REPAIR_TASK, &prompt, skp::repair_schema(), skp::parse_repair).await {
            let _ = with_sk(&c.dir, |sess| {
                if let Some(k) = sess.skeleton.as_mut() {
                    skp::apply_jumps(k, r, &cands);
                }
                Ok(())
            })
            .await;
        }
    }
    errors
}

// ── 細修 ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct RefineBody {
    para: String,
    preset: String,
}

/// 用字 / 我的風格 (lore-polish's presets, the same as the 潤飾 window: locked LaTeX, the
/// notation check of the rewrite) or 檢查 (the program's checks only): a `refine` thread of
/// proposed changes, made in the background (`busy = "refine"`).
async fn post_refine(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<RefineBody>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let preset = match b.preset.as_str() {
        "wording" => crate::polish::DICTION,
        "style" => crate::polish::VOICE,
        "check" => "check",
        _ => return bad("沒有這種細修"),
    };
    let Some(held) = hold(&c.dir, &format!("para:{}", b.para)) else { return conflict("這一段正在處理") };
    let (sess, (text, section)) = tri!(
        with_sk(&c.dir, |sess| {
            let k = sess.skeleton.as_mut().ok_or("還沒有骨架")?;
            clear_failed(k);
            let section = k.section_of(&b.para).map(|x| sk::old_section(x).to_string()).unwrap_or_default();
            let p = k.para_mut(&b.para).ok_or("沒有這一段")?;
            if p.text.trim().is_empty() {
                return Err("這一段還沒有文字".into());
            }
            p.busy = Some("refine".into());
            Ok((p.text.clone(), section))
        })
        .await
    );
    let (s2, c2, para, name) = (s.clone(), own(&c), b.para.clone(), name.clone());
    tokio::spawn(async move {
        let _held = held;
        let label = match preset {
            crate::polish::DICTION => "用字",
            crate::polish::VOICE => "我的風格",
            _ => "檢查",
        };
        let (changes, note): (Vec<sk::Change>, String) = if preset == "check" {
            let (words, fixed) = {
                let l = s2.loaded.read().await;
                let words: Vec<(String, String, String)> = l
                    .profile
                    .as_ref()
                    .map(|prof| pc_core::check_text(prof, &text, Some(&section)).into_iter().flat_map(|r| r.findings).filter_map(|f| Some((f.found?, f.suggest.unwrap_or_default(), f.message))).collect())
                    .unwrap_or_default();
                let rules = l.profile.as_ref().map(|p| p.rules.clone()).unwrap_or_default();
                let g = crate::guard::load(&c2.ws, Some(&c2.dir), &rules);
                let fixed = g.active().then(|| g.finish(&text).0).filter(|f| *f != text);
                (words, fixed)
            };
            let ch = sk::style::changes_from_checks(&text, &words, fixed.as_deref());
            let note = if ch.is_empty() { "檢查沒有找到要改的".to_string() } else { String::new() };
            (ch, note)
        } else {
            let body = lore_polish::ProposeBody { doc: name, text: text.clone(), preset: Some(preset.into()), scope: section.clone(), ..Default::default() };
            match lore_polish::propose(&crate::polish::PcHost(s2.clone()), body).await {
                Ok(p) if p.ok => {
                    let ch = sk::style::changes_from_rewrite(&text, &p.after, label);
                    let note = if ch.is_empty() { "沒有要改的".to_string() } else { String::new() };
                    (ch, note)
                }
                Ok(p) => (vec![], format!("建議沒有通過檢查：{}", p.problems.join("；"))),
                Err(e) => (vec![], format!("{label}沒有完成：{}", e.message())),
            }
        };
        let _ = with_sk(&c2.dir, |sess| {
            let Some(k) = sess.skeleton.as_mut() else { return Ok(()) };
            let Some(p) = k.para_mut(&para) else { return Ok(()) };
            p.busy = None;
            let t = sk::style::refine_thread(&para, changes, &note);
            k.add_thread(t);
            Ok(())
        })
        .await;
    });
    reply(&c, &sess)
}

/// 風格對照: the program's numbers per section next to the author's (no model).
async fn get_style(State(s): State<Shared>, AxPath(name): AxPath<String>) -> Resp {
    let c = tri!(ctx(&s, &name).await);
    let sess = tri!(load(&c.dir));
    let Some(k) = sess.skeleton.as_ref().filter(|k| k.exists()) else { return bad("還沒有骨架") };
    let ranges = crate::guard::paragraph_ranges(&c.ws);
    let l = s.loaded.read().await;
    let rules = l.profile.as_ref().map(|p| p.rules.clone()).unwrap_or_default();
    let guard = crate::guard::load(&c.ws, Some(&c.dir), &rules);
    let cache = if cfg!(test) { std::env::temp_dir().join("pc-test-cache") } else { crate::polish::cache_root() };
    let sp = l.profile.as_ref().map(|prof| pc_core::styleprofile::load_or_build(&prof.phrases, &prof.expressions, &prof.root, &c.ws, &pc_core::styleprofile::cache_file(&cache, &c.ws)));
    let mut banned: Vec<String> = sess.writing_notes.iter().flat_map(|n| n.banned.clone()).collect();
    banned.extend(l.profile.as_ref().map(|p| p.vocabulary.iter().map(|v| v.avoid.clone()).collect::<Vec<_>>()).unwrap_or_default());
    banned.sort();
    banned.dedup();
    let mine = |sec: &sk::Section| {
        let old = sk::old_section(sec);
        sk::style::Mine { range: ranges.get(old).copied(), profile: sp.as_ref().and_then(|p| p.sections.get(&pc_core::patterns::canonical_section(old))) }
    };
    Json(sk::style::report(k, &mine, &banned, &guard)).into_response()
}
