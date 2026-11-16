//! 骨架 routes: `/api/papers/{name}/draft/skeleton...` (contract: `docs/skeleton-api.md`).
//!
//! Every write goes through [`with_sk`]: the lock, a fresh load, a change by id, `rev` up when
//! anything changed, save. Background tasks (plan, fill, write, repair) never hold a session
//! across a model call: they read what they need, call the model, then patch the fresh session
//! by id (a paragraph moved meanwhile is found where it is now; one removed drops the result).
//! What runs is kept in [`LIVE`]: a `busy` paragraph or a `job` saved by a task that is not
//! running any more (the app quit) is settled when the skeleton is read.

use super::*;
use pc_core::draft::skeleton::{self as sk, inventory, Skeleton};
use std::collections::BTreeSet;

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/papers/{name}/draft/skeleton", get(get_view))
        .route("/api/papers/{name}/draft/skeleton/inventory", post(post_inventory))
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

/// The fill-in of an answered thread on a task of its own (stage 4).
fn spawn_fill(_s: &Shared, _c: &Ctx, _thread: String) {}
