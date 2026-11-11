//! Who goes next, and when the model may change. Pure: no I/O, no clock of its own
//! (every call takes `now`), so every rule is unit-tested.
//!
//! Order of waiting requests (design companion §8):
//!
//! 1. requests from the **focused app** (its window is in front),
//! 2. then **interactive** before **background**,
//! 3. then first come, first served.
//!
//! At most [`Config::slots`] requests run at once (default [`DEFAULT_SLOTS`]). A running
//! request is never cut (no cancellation in this phase): a background request yields
//! *between* requests only.
//!
//! Model changes ([`swap_decision`]): the loaded model is kept when the request asks for
//! it; a background request never makes the model change while a request from the
//! focused app or an interactive request is waiting; a change waits until the running
//! requests have finished, and nothing overtakes a request that waits for one.
//!
//! Starvation guard: a background request that has waited [`Config::promote_after`]
//! (default [`DEFAULT_PROMOTE_AFTER`], two minutes) is promoted one level, to
//! interactive (it does not become a focused-app request).

use lore_llm::broker::Priority;
use std::time::{Duration, Instant};

/// Requests running at once (llama-server's slots).
pub const DEFAULT_SLOTS: usize = 1;
/// A background request waiting this long counts as interactive.
pub const DEFAULT_PROMOTE_AFTER: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Requests running at once (at least 1).
    pub slots: usize,
    /// Promote a background request after waiting this long (`None`: never).
    pub promote_after: Option<Duration>,
}

impl Default for Config {
    fn default() -> Self {
        Config { slots: DEFAULT_SLOTS, promote_after: Some(DEFAULT_PROMOTE_AFTER) }
    }
}

/// One request, waiting or running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    /// `X-Lore-App` (any short name).
    pub app: String,
    pub priority: Priority,
    /// Catalog id of the model it needs.
    pub model: String,
    /// When it arrived.
    pub since: Instant,
}

/// What starting a request means for the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Swap {
    /// The loaded model is the one asked for.
    Keep,
    /// Nothing is loaded: load it.
    Load,
    /// Another model is loaded and nothing runs: change it.
    Change,
    /// A change (or load) is needed but requests are running: wait for them.
    WaitForRunning,
    /// A background request would change the loaded model while a more urgent one waits.
    Blocked,
}

/// The model-change rule. `background`: the request's effective priority;
/// `urgent_waiting`: another request from the focused app or an interactive one waits.
pub fn swap_decision(loaded: Option<&str>, want: &str, background: bool, urgent_waiting: bool, running: usize) -> Swap {
    if loaded == Some(want) {
        return Swap::Keep;
    }
    // loading into empty memory is not a change: nothing is taken from anyone
    if background && urgent_waiting && loaded.is_some() {
        return Swap::Blocked;
    }
    if running > 0 {
        return Swap::WaitForRunning;
    }
    if loaded.is_none() { Swap::Load } else { Swap::Change }
}

/// A request that may start now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub id: u64,
    pub model: String,
    pub swap: Swap,
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    pub config: Config,
    focused: Option<String>,
    queue: Vec<Request>,
    running: Vec<Request>,
    /// The model the next requests are planned against.
    loaded: Option<String>,
    next_id: u64,
}

impl Scheduler {
    pub fn new(config: Config) -> Scheduler {
        Scheduler { config, focused: None, queue: Vec::new(), running: Vec::new(), loaded: None, next_id: 1 }
    }

    /// The app whose window is in front (`None`: none of ours).
    pub fn set_focus(&mut self, app: Option<&str>) {
        self.focused = app.map(str::trim).filter(|a| !a.is_empty()).map(str::to_string);
    }

    pub fn focused(&self) -> Option<&str> {
        self.focused.as_deref()
    }

    /// What is really loaded (after a release or an idle unload).
    pub fn set_loaded(&mut self, model: Option<&str>) {
        self.loaded = model.map(str::to_string);
    }

    pub fn loaded(&self) -> Option<&str> {
        self.loaded.as_deref()
    }

    /// Queue a request; returns its id.
    pub fn enqueue(&mut self, app: &str, priority: Priority, model: &str, now: Instant) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.queue.push(Request { id, app: app.to_string(), priority, model: model.to_string(), since: now });
        id
    }

    /// The request is over (finished, failed, or its client went away while waiting).
    pub fn done(&mut self, id: u64) {
        self.queue.retain(|r| r.id != id);
        self.running.retain(|r| r.id != id);
    }

    pub fn waiting(&self) -> &[Request] {
        &self.queue
    }

    pub fn running(&self) -> &[Request] {
        &self.running
    }

    fn is_focused(&self, r: &Request) -> bool {
        self.focused.as_deref() == Some(r.app.as_str())
    }

    /// The priority after the starvation guard.
    pub fn effective(&self, r: &Request, now: Instant) -> Priority {
        match (r.priority, self.config.promote_after) {
            (Priority::Background, Some(after)) if now.saturating_duration_since(r.since) >= after => Priority::Interactive,
            (p, _) => p,
        }
    }

    /// Waiting requests in the order they would start.
    pub fn order(&self, now: Instant) -> Vec<&Request> {
        let mut v: Vec<&Request> = self.queue.iter().collect();
        v.sort_by_key(|r| (!self.is_focused(r), self.effective(r, now) != Priority::Interactive, r.since, r.id));
        v
    }

    /// The next request that may start now, moved to running; `None` when the slots are
    /// full, nothing waits, or the first in line waits for a model change.
    pub fn next(&mut self, now: Instant) -> Option<Start> {
        if self.running.len() >= self.config.slots.max(1) {
            return None;
        }
        let mut pick = None;
        for r in self.order(now) {
            let background = self.effective(r, now) == Priority::Background;
            let urgent = self.queue.iter().any(|o| o.id != r.id && (self.is_focused(o) || self.effective(o, now) == Priority::Interactive));
            match swap_decision(self.loaded.as_deref(), &r.model, background, urgent, self.running.len()) {
                Swap::Blocked => continue,
                Swap::WaitForRunning => return None,
                s => {
                    pick = Some((r.id, s));
                    break;
                }
            }
        }
        let (id, swap) = pick?;
        let i = self.queue.iter().position(|r| r.id == id).expect("in the queue");
        let r = self.queue.remove(i);
        self.loaded = Some(r.model.clone());
        let start = Start { id, model: r.model.clone(), swap };
        self.running.push(r);
        Some(start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Priority::{Background as Bg, Interactive as Ia};

    fn s(slots: usize) -> Scheduler {
        Scheduler::new(Config { slots, promote_after: None })
    }

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    /// Start everything that may start now and finish it at once; the order of ids.
    fn drain(sc: &mut Scheduler, now: Instant) -> Vec<u64> {
        let mut out = Vec::new();
        while let Some(st) = sc.next(now) {
            out.push(st.id);
            sc.done(st.id);
        }
        out
    }

    #[test]
    fn same_level_is_first_come_first_served() {
        let t = Instant::now();
        let mut sc = s(1);
        let a = sc.enqueue("kb", Bg, "m", at(t, 0));
        let b = sc.enqueue("lr", Bg, "m", at(t, 1));
        let c = sc.enqueue("kb", Bg, "m", at(t, 2));
        assert_eq!(drain(&mut sc, at(t, 3)), vec![a, b, c]);
    }

    #[test]
    fn interactive_goes_before_background() {
        let t = Instant::now();
        let mut sc = s(1);
        let a = sc.enqueue("kb", Bg, "m", at(t, 0));
        let b = sc.enqueue("lr", Ia, "m", at(t, 1));
        let c = sc.enqueue("sc", Bg, "m", at(t, 2));
        let d = sc.enqueue("pc", Ia, "m", at(t, 3));
        assert_eq!(drain(&mut sc, at(t, 4)), vec![b, d, a, c]);
    }

    #[test]
    fn the_focused_app_goes_first_even_with_background_work() {
        let t = Instant::now();
        let mut sc = s(1);
        sc.set_focus(Some("pc"));
        let other_ia = sc.enqueue("lr", Ia, "m", at(t, 0));
        let pc_bg = sc.enqueue("pc", Bg, "m", at(t, 1));
        let pc_ia = sc.enqueue("pc", Ia, "m", at(t, 2));
        let other_bg = sc.enqueue("kb", Bg, "m", at(t, 3));
        assert_eq!(drain(&mut sc, at(t, 4)), vec![pc_ia, pc_bg, other_ia, other_bg]);
    }

    #[test]
    fn a_focus_change_reorders_the_queue() {
        let t = Instant::now();
        let mut sc = s(1);
        sc.set_focus(Some("pc"));
        let pc = sc.enqueue("pc", Bg, "m", at(t, 0));
        let sc_req = sc.enqueue("sc", Bg, "m", at(t, 1));
        sc.set_focus(Some("sc"));
        assert_eq!(sc.focused(), Some("sc"));
        assert_eq!(drain(&mut sc, at(t, 2)), vec![sc_req, pc]);
        sc.set_focus(Some("  "));
        assert_eq!(sc.focused(), None, "blank = none");
    }

    #[test]
    fn at_most_the_slot_count_runs() {
        let t = Instant::now();
        let mut sc = s(1);
        let a = sc.enqueue("kb", Bg, "m", t);
        let b = sc.enqueue("kb", Bg, "m", t);
        assert_eq!(sc.next(t).map(|x| x.id), Some(a));
        assert_eq!(sc.next(t), None, "one slot");
        sc.done(a);
        assert_eq!(sc.next(t).map(|x| x.id), Some(b));

        let mut two = s(2);
        let a = two.enqueue("kb", Bg, "m", t);
        let b = two.enqueue("kb", Bg, "m", t);
        let c = two.enqueue("kb", Bg, "m", t);
        assert_eq!(two.next(t).map(|x| x.id), Some(a));
        assert_eq!(two.next(t).map(|x| (x.id, x.swap)), Some((b, Swap::Keep)), "same model: runs alongside");
        assert_eq!(two.next(t), None);
        assert_eq!(two.running().len(), 2);
        two.done(b);
        assert_eq!(two.next(t).map(|x| x.id), Some(c));

        // 0 slots is treated as 1.
        let mut zero = s(0);
        zero.enqueue("kb", Bg, "m", t);
        assert!(zero.next(t).is_some());
    }

    #[test]
    fn running_background_work_is_not_cut_it_yields_between_requests() {
        let t = Instant::now();
        let mut sc = s(1);
        let bg1 = sc.enqueue("kb", Bg, "m", at(t, 0));
        let bg2 = sc.enqueue("kb", Bg, "m", at(t, 1));
        assert_eq!(sc.next(at(t, 2)).unwrap().id, bg1);
        sc.set_focus(Some("pc"));
        let fg = sc.enqueue("pc", Ia, "m", at(t, 3));
        assert_eq!(sc.next(at(t, 4)), None, "bg1 keeps running");
        assert_eq!(sc.running()[0].id, bg1);
        sc.done(bg1);
        assert_eq!(drain(&mut sc, at(t, 5)), vec![fg, bg2], "the foreground request goes before the next background one");
    }

    #[test]
    fn the_loaded_model_is_kept_or_loaded_or_changed() {
        let t = Instant::now();
        let mut sc = s(1);
        sc.enqueue("kb", Bg, "a", t);
        assert_eq!(sc.next(t).unwrap().swap, Swap::Load);
        assert_eq!(sc.loaded(), Some("a"));
        sc.done(1);
        sc.enqueue("kb", Bg, "a", t);
        assert_eq!(sc.next(t).unwrap().swap, Swap::Keep);
        sc.done(2);
        sc.enqueue("kb", Bg, "b", t);
        assert_eq!(sc.next(t).unwrap().swap, Swap::Change);
        assert_eq!(sc.loaded(), Some("b"));
        sc.done(3);
        sc.set_loaded(None); // released
        sc.enqueue("kb", Bg, "b", t);
        assert_eq!(sc.next(t).unwrap().swap, Swap::Load);
    }

    #[test]
    fn a_change_waits_for_the_running_requests_and_nothing_overtakes_it() {
        let t = Instant::now();
        let mut sc = s(2);
        let a1 = sc.enqueue("kb", Bg, "a", at(t, 0));
        assert_eq!(sc.next(at(t, 1)).unwrap().id, a1);
        let b = sc.enqueue("pc", Ia, "b", at(t, 2));
        let a2 = sc.enqueue("kb", Ia, "a", at(t, 3));
        assert_eq!(sc.next(at(t, 4)), None, "b needs a change: waits, and a2 does not slip in");
        sc.done(a1);
        let st = sc.next(at(t, 5)).unwrap();
        assert_eq!((st.id, st.swap), (b, Swap::Change));
        assert_eq!(sc.next(at(t, 6)), None, "a2 now needs a change back: waits for b");
        sc.done(b);
        assert_eq!(sc.next(at(t, 7)).map(|x| (x.id, x.swap)), Some((a2, Swap::Change)));
    }

    #[test]
    fn background_work_never_changes_the_model_while_urgent_work_waits() {
        let t = Instant::now();
        let mut sc = s(1);
        sc.set_loaded(Some("a"));
        sc.set_focus(Some("pc"));
        // The focused app's background request needs b; another app's interactive one uses a.
        let pc_bg = sc.enqueue("pc", Bg, "b", at(t, 0));
        let lr_ia = sc.enqueue("lr", Ia, "a", at(t, 1));
        let st = sc.next(at(t, 2)).unwrap();
        assert_eq!((st.id, st.swap), (lr_ia, Swap::Keep), "the interactive one runs on the loaded model first");
        sc.done(lr_ia);
        let st = sc.next(at(t, 3)).unwrap();
        assert_eq!((st.id, st.swap), (pc_bg, Swap::Change), "then the change");

        // A background request that needs a change, behind nothing urgent, may change.
        let mut sc = s(1);
        sc.set_loaded(Some("a"));
        let kb = sc.enqueue("kb", Bg, "b", t);
        sc.enqueue("lr", Bg, "a", t);
        assert_eq!(sc.next(t).map(|x| (x.id, x.swap)), Some((kb, Swap::Change)));

        // While the focused app's background request is blocked, the interactive one
        // goes first, and may change the model itself.
        let mut sc = s(1);
        sc.set_loaded(Some("a"));
        sc.set_focus(Some("pc"));
        let pc_bg = sc.enqueue("pc", Bg, "b", at(t, 0));
        let lr_ia = sc.enqueue("lr", Ia, "c", at(t, 1));
        assert_eq!(sc.next(at(t, 2)).map(|x| (x.id, x.swap)), Some((lr_ia, Swap::Change)));
        sc.done(lr_ia);
        assert_eq!(sc.next(at(t, 3)).map(|x| x.id), Some(pc_bg));
    }

    #[test]
    fn swap_rule_table() {
        assert_eq!(swap_decision(Some("a"), "a", true, true, 1), Swap::Keep);
        assert_eq!(swap_decision(None, "a", false, false, 0), Swap::Load);
        assert_eq!(swap_decision(Some("a"), "b", false, false, 0), Swap::Change);
        assert_eq!(swap_decision(Some("a"), "b", false, true, 0), Swap::Change, "interactive may change with others waiting");
        assert_eq!(swap_decision(Some("a"), "b", true, true, 0), Swap::Blocked);
        assert_eq!(swap_decision(None, "b", true, true, 0), Swap::Load, "nothing loaded: not a change");
        assert_eq!(swap_decision(Some("a"), "b", true, false, 0), Swap::Change);
        assert_eq!(swap_decision(Some("a"), "b", false, false, 1), Swap::WaitForRunning);
        assert_eq!(swap_decision(Some("a"), "b", true, false, 2), Swap::WaitForRunning);
    }

    #[test]
    fn starving_background_work_is_promoted_one_level() {
        let t = Instant::now();
        let mut sc = Scheduler::new(Config { slots: 1, promote_after: Some(Duration::from_secs(60)) });
        assert_eq!(Config::default().promote_after, Some(DEFAULT_PROMOTE_AFTER));
        let old_bg = sc.enqueue("kb", Bg, "m", at(t, 0));
        let ia = sc.enqueue("lr", Ia, "m", at(t, 30_000));
        // Before the limit: interactive first.
        assert_eq!(sc.order(at(t, 59_000)).iter().map(|r| r.id).collect::<Vec<_>>(), vec![ia, old_bg]);
        // After: the old background request is interactive and older.
        assert_eq!(sc.effective(&sc.waiting()[0].clone(), at(t, 60_000)), Ia);
        assert_eq!(sc.order(at(t, 60_000)).iter().map(|r| r.id).collect::<Vec<_>>(), vec![old_bg, ia]);
        // One level only: the focused app still goes first.
        sc.set_focus(Some("pc"));
        let pc_bg = sc.enqueue("pc", Bg, "m", at(t, 61_000));
        assert_eq!(drain(&mut sc, at(t, 61_000)), vec![pc_bg, old_bg, ia]);
    }

    #[test]
    fn a_promoted_request_may_change_the_model() {
        let t = Instant::now();
        let mut sc = Scheduler::new(Config { slots: 1, promote_after: Some(Duration::from_secs(10)) });
        sc.set_loaded(Some("a"));
        let bg = sc.enqueue("kb", Bg, "b", at(t, 0));
        let ia = sc.enqueue("lr", Ia, "a", at(t, 5_000));
        assert_eq!(sc.next(at(t, 6_000)).map(|x| x.id), Some(ia), "not yet promoted");
        sc.done(ia);
        let ia2 = sc.enqueue("lr", Ia, "a", at(t, 7_000));
        assert_eq!(sc.next(at(t, 11_000)).map(|x| (x.id, x.swap)), Some((bg, Swap::Change)), "promoted, older than ia2");
        sc.done(bg);
        assert_eq!(sc.next(at(t, 12_000)).map(|x| x.id), Some(ia2));
    }

    #[test]
    fn a_waiting_request_can_be_withdrawn() {
        let t = Instant::now();
        let mut sc = s(1);
        let a = sc.enqueue("kb", Bg, "m", t);
        let b = sc.enqueue("kb", Bg, "m", t);
        sc.done(a);
        assert_eq!(sc.waiting().len(), 1);
        assert_eq!(drain(&mut sc, t), vec![b]);
        assert!(sc.waiting().is_empty() && sc.running().is_empty());
    }
}
