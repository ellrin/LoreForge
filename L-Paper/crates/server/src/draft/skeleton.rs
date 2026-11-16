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

/// The fill-in of an answered thread on a task of its own (stage 4).
fn spawn_fill(_s: &Shared, _c: &Ctx, _thread: String) {}
