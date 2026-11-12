//! Server-side jobs: a list of stages run in order in the background, one checkpoint per
//! stage, progress for the page, cancel and resume. Generic: a [`Pipeline`] says what the
//! stages are and how each runs; 論文報告 (`todeck`) is the first one.
//!
//! - A job is a folder `<root>/<id>/`: `job.json` (options, status, per-stage time and
//!   counts) and one `<stage>.json` per finished stage (`{stage, output, counts, elapsed_ms,
//!   finished}`), all written atomically (`.tmp` + rename). Other files of the folder belong to
//!   the pipeline (pictures…). A folder without a readable `job.json` is not a job.
//! - A stage runs iff its file is missing or unreadable: finished stages are never redone, a
//!   corrupt one is redone alone. Stages read their inputs from earlier stages' files.
//! - The runner is a spawned task, not tied to any HTTP connection: closing the page never
//!   stops it. [`Job::cancel`] wakes the runner, which drops the stage's future (an in-flight
//!   model request is aborted with it) and records `stopped`.
//! - A `wait` stage (the author's review) ends the run with status `review`; [`Jobs::submit`]
//!   writes its file and starts the runner again. Created with `direct`, it is skipped.
//! - Jobs found on disk that were running or waiting when the process ended are
//!   `interrupted` (the page offers 「上次做到第 N 步，繼續？」).
//!
//! Wire shapes: docs/paper-deck.md "API".

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::{Notify, watch};

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Write `bytes` to `path` through a temporary file next to it and a rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    std::fs::create_dir_all(dir)?;
    static N: AtomicU32 = AtomicU32::new(0);
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

pub fn write_json(path: &Path, v: &impl Serialize) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(v).map_err(std::io::Error::other)?;
    write_atomic(path, &bytes)
}

/// A job id: lowercase letters and digits only (it is a folder name).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

#[derive(Debug, Clone, Copy)]
pub struct StageSpec {
    pub id: &'static str,
    /// Shown in the stage list.
    pub label: &'static str,
    /// Calls a model: the page shows streamed characters and the last activity.
    pub model: bool,
    /// Waits for the author ([`Jobs::submit`]); skipped when the job is `direct`.
    pub wait: bool,
}

/// A failed stage: an HTTP-like status, one line for the page and extra fields for the
/// error body (e.g. lore-llm's `alternative`).
#[derive(Debug, Clone)]
pub struct StageError {
    pub status: u16,
    pub message: String,
    pub extra: Value,
}

impl StageError {
    pub fn new(status: u16, message: impl Into<String>) -> StageError {
        StageError { status, message: message.into(), extra: Value::Null }
    }
}

impl From<String> for StageError {
    fn from(m: String) -> Self {
        StageError::new(500, m)
    }
}

pub type StageFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, StageError>> + Send + 'a>>;

/// What a kind of job does.
pub trait Pipeline: Send + Sync + 'static {
    fn stages(&self) -> &'static [StageSpec];
    /// Run one stage; the output is its checkpoint. Inputs come from [`StageCx::output`].
    fn run<'a>(&'a self, cx: &'a StageCx, stage: &'static str) -> StageFuture<'a>;
    /// Extra fields of the job object (the finished deck, the review material…).
    fn decorate(&self, _job: &Job, _snap: &mut Map<String, Value>) {}
}

/// Status of a job (the `state` of the job object).
pub mod status {
    pub const RUNNING: &str = "running";
    pub const REVIEW: &str = "review";
    pub const DONE: &str = "done";
    pub const FAILED: &str = "failed";
    /// Stopped by the author (停止).
    pub const STOPPED: &str = "stopped";
    /// The process ended while it was running or waiting.
    pub const INTERRUPTED: &str = "interrupted";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StageMeta {
    /// `pending` · `running` · `done` · `failed`.
    pub state: String,
    pub started: Option<u64>,
    pub elapsed_ms: u64,
    pub counts: Map<String, Value>,
    pub chars: Option<u64>,
    pub last_activity: Option<u64>,
    pub skipped: bool,
}

/// `job.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JobMeta {
    pub id: String,
    pub created: u64,
    pub title: String,
    /// What the pipeline needs to run (source, options…).
    pub params: Value,
    /// Skip `wait` stages.
    pub direct: bool,
    pub status: String,
    /// `{stage, status, error, …extra}` when failed.
    pub error: Option<Value>,
    pub stages: BTreeMap<String, StageMeta>,
}

/// Cancellation of the current run (reset when a run starts).
#[derive(Default)]
struct Cancel {
    flag: AtomicBool,
    notify: Notify,
}

impl Cancel {
    fn fire(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
    async fn wait(&self) {
        loop {
            let n = self.notify.notified();
            tokio::pin!(n);
            n.as_mut().enable();
            if self.flag.load(Ordering::SeqCst) {
                return;
            }
            n.await;
        }
    }
}

pub struct Job {
    pub id: String,
    pub dir: PathBuf,
    meta: Mutex<JobMeta>,
    seq: watch::Sender<u64>,
    cancel: Cancel,
    running: AtomicBool,
}

impl Job {
    pub fn meta(&self) -> JobMeta {
        self.meta.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn status(&self) -> String {
        self.meta.lock().unwrap_or_else(|e| e.into_inner()).status.clone()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn seq(&self) -> u64 {
        *self.seq.borrow()
    }

    fn bump(&self) {
        self.seq.send_modify(|v| *v += 1);
    }

    /// Change the meta, write `job.json` when `persist`, wake the pages.
    fn edit(&self, persist: bool, f: impl FnOnce(&mut JobMeta)) {
        let snapshot = {
            let mut m = self.meta.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut m);
            persist.then(|| m.clone())
        };
        if let Some(m) = snapshot
            && let Err(e) = write_json(&self.dir.join("job.json"), &m)
        {
            eprintln!("jobs: {}: {e}", self.id);
        }
        self.bump();
    }

    fn stage_edit(&self, stage: &str, persist: bool, f: impl FnOnce(&mut StageMeta)) {
        self.edit(persist, |m| f(m.stages.entry(stage.to_string()).or_default()));
    }

    /// Merge `v` (an object) into a stage's counts.
    pub fn stage_counts(&self, stage: &str, v: Value) {
        self.stage_edit(stage, false, |s| {
            if let Value::Object(m) = v {
                s.counts.extend(m);
            }
        });
    }

    pub fn stage_file(&self, stage: &str) -> PathBuf {
        self.dir.join(format!("{stage}.json"))
    }

    /// A finished stage's checkpoint (`None`: missing or unreadable).
    pub fn checkpoint(&self, stage: &str) -> Option<Value> {
        read_checkpoint(&self.stage_file(stage), stage)
    }

    /// A finished stage's output.
    pub fn output(&self, stage: &str) -> Option<Value> {
        self.checkpoint(stage).map(|mut c| c["output"].take())
    }

    /// Rewrite a finished stage's output (e.g. the author's 我懂 on the terms) and its counts;
    /// the rest of its checkpoint stays.
    pub fn replace_output(&self, stage: &str, output: Value, counts: Value) -> std::io::Result<()> {
        let mut cp = self.checkpoint(stage).ok_or_else(|| std::io::Error::other("no checkpoint"))?;
        cp["output"] = output;
        cp["counts"] = counts.clone();
        write_json(&self.stage_file(stage), &cp)?;
        self.stage_edit(stage, true, |s| {
            if let Value::Object(m) = counts {
                s.counts = m;
            }
        });
        Ok(())
    }

    /// Stop the current run (the stage's future is dropped; an in-flight request aborts).
    pub fn cancel(&self) {
        self.cancel.fire();
    }

    /// Wait until the job's sequence number passes `after` (or `timeout`).
    pub async fn changed_after(&self, after: u64, timeout: Duration) {
        let mut rx = self.seq.subscribe();
        let _ = tokio::time::timeout(timeout, async {
            while *rx.borrow_and_update() <= after {
                if rx.changed().await.is_err() {
                    return;
                }
            }
        })
        .await;
    }

    /// Wait until no run is going on.
    pub async fn settled(&self) {
        let mut rx = self.seq.subscribe();
        loop {
            rx.borrow_and_update();
            if !self.is_running() {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }

    /// The job object (docs/paper-deck.md "API"), without the pipeline's extra fields.
    pub fn snapshot(&self, specs: &[StageSpec]) -> Map<String, Value> {
        let m = self.meta();
        let now = now_ms();
        let mut done = 0;
        let mut current = None;
        let stages: Vec<Value> = specs
            .iter()
            .enumerate()
            .map(|(i, sp)| {
                let st = m.stages.get(sp.id).cloned().unwrap_or_default();
                let state = if st.state.is_empty() { "pending".to_string() } else { st.state.clone() };
                if state == "done" {
                    done += 1;
                } else if current.is_none() {
                    current = Some(i);
                }
                let live = state == "running" && m.status == status::RUNNING;
                let ms = st.elapsed_ms + if live { st.started.map_or(0, |s| now.saturating_sub(s)) } else { 0 };
                let mut o = json!({ "id": sp.id, "label": sp.label, "state": state, "model": sp.model, "ms": ms, "counts": st.counts, "started": st.started });
                if st.skipped {
                    o["skipped"] = true.into();
                }
                if sp.model && (st.chars.is_some() || state == "running") {
                    o["chars"] = st.chars.unwrap_or(0).into();
                    o["lastActivity"] = st.last_activity.into();
                    o["agoMs"] = st.last_activity.or(st.started).map(|t| now.saturating_sub(t)).into();
                }
                o
            })
            .collect();
        let err = m.error.clone();
        let resumable = matches!(m.status.as_str(), status::STOPPED | status::FAILED | status::INTERRUPTED);
        let mut out = Map::new();
        out.insert("id".into(), m.id.clone().into());
        out.insert("title".into(), m.title.clone().into());
        out.insert("created".into(), m.created.into());
        out.insert("state".into(), m.status.clone().into());
        out.insert("direct".into(), m.direct.into());
        out.insert("step".into(), (current.unwrap_or(specs.len().saturating_sub(1)) + 1).into());
        out.insert("doneSteps".into(), done.into());
        out.insert("total".into(), specs.len().into());
        out.insert("seq".into(), self.seq().into());
        out.insert("resumable".into(), resumable.into());
        out.insert("error".into(), err.as_ref().and_then(|e| e["error"].as_str()).map(String::from).into());
        out.insert("failure".into(), err.unwrap_or(Value::Null));
        out.insert("stages".into(), stages.into());
        out
    }
}

fn read_checkpoint(path: &Path, stage: &str) -> Option<Value> {
    let v: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (v["stage"] == stage && v.get("output").is_some()).then_some(v)
}

/// What a stage can do while it runs.
pub struct StageCx {
    pub job: Arc<Job>,
    pub stage: &'static str,
}

impl StageCx {
    pub fn dir(&self) -> &Path {
        &self.job.dir
    }
    pub fn params(&self) -> Value {
        self.job.meta().params
    }
    /// An earlier stage's output (its checkpoint file).
    pub fn output(&self, stage: &str) -> Result<Value, StageError> {
        self.job.output(stage).ok_or_else(|| StageError::new(500, format!("缺少「{stage}」的結果")))
    }
    /// Set this stage's counts (merged with the ones already set).
    pub fn counts(&self, v: Value) {
        self.job.stage_counts(self.stage, v);
    }
    /// [`StageCx::counts`] from a blocking thread: a `'static` handle.
    pub fn counter(&self) -> impl Fn(Value) + Send + Sync + 'static {
        let (job, stage) = (self.job.clone(), self.stage);
        move |v| job.stage_counts(stage, v)
    }
    /// A model stage's streamed characters so far (and now as the last activity).
    pub fn chars(&self, n: u64) {
        self.job.stage_edit(self.stage, false, |s| {
            s.chars = Some(n);
            s.last_activity = Some(now_ms());
        });
    }
    /// The job's title (once the source is read).
    pub fn title(&self, t: &str) {
        let t = t.to_string();
        self.job.edit(true, |m| m.title = t);
    }
}

/// The jobs of one folder.
pub struct Jobs {
    pub root: PathBuf,
    pipeline: Arc<dyn Pipeline>,
    map: Mutex<BTreeMap<String, Arc<Job>>>,
}

impl Jobs {
    /// The jobs in `root` (read once: running or waiting ones become `interrupted`).
    pub fn open(root: PathBuf, pipeline: Arc<dyn Pipeline>) -> Arc<Jobs> {
        let jobs = Arc::new(Jobs { root: root.clone(), pipeline, map: Mutex::new(BTreeMap::new()) });
        for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let id = e.file_name().to_string_lossy().to_string();
            if !valid_id(&id) {
                continue;
            }
            let Some(mut meta) = std::fs::read(e.path().join("job.json")).ok().and_then(|b| serde_json::from_slice::<JobMeta>(&b).ok()) else { continue };
            if meta.id != id {
                continue;
            }
            if matches!(meta.status.as_str(), status::RUNNING | status::REVIEW) {
                meta.status = status::INTERRUPTED.into();
            }
            let job = jobs.job_from(meta);
            jobs.reconcile(&job);
            jobs.map.lock().unwrap_or_else(|e| e.into_inner()).insert(id, job);
        }
        jobs
    }

    pub fn specs(&self) -> &'static [StageSpec] {
        self.pipeline.stages()
    }

    fn job_from(&self, meta: JobMeta) -> Arc<Job> {
        let (tx, _) = watch::channel(1);
        Arc::new(Job { id: meta.id.clone(), dir: self.root.join(&meta.id), meta: Mutex::new(meta), seq: tx, cancel: Cancel::default(), running: AtomicBool::new(false) })
    }

    /// Stage states from the files: a stage is done iff its checkpoint reads.
    fn reconcile(&self, job: &Job) {
        let specs = self.specs();
        let files: Vec<Option<Value>> = specs.iter().map(|s| job.checkpoint(s.id)).collect();
        job.edit(false, |m| {
            for (sp, cp) in specs.iter().zip(files) {
                let st = m.stages.entry(sp.id.to_string()).or_default();
                match cp {
                    Some(c) => {
                        st.state = "done".into();
                        st.skipped = c["skipped"].as_bool().unwrap_or(false);
                        st.elapsed_ms = c["elapsed_ms"].as_u64().unwrap_or(st.elapsed_ms);
                        if let Some(Value::Object(cn)) = c.get("counts") {
                            st.counts = cn.clone();
                        }
                    }
                    None if st.state == "done" || st.state == "running" => st.state = "pending".into(),
                    None => {}
                }
            }
        });
    }

    pub fn get(&self, id: &str) -> Option<Arc<Job>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Arc<Job>> {
        let mut v: Vec<Arc<Job>> = self.map.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        v.sort_by_key(|j| std::cmp::Reverse((j.meta().created, j.id.clone())));
        v
    }

    /// The job object with the pipeline's fields.
    pub fn view(&self, job: &Job) -> Value {
        let mut m = job.snapshot(self.specs());
        self.pipeline.decorate(job, &mut m);
        Value::Object(m)
    }

    /// A new job, started at once.
    pub fn create(self: &Arc<Self>, params: Value, direct: bool, title: &str) -> std::io::Result<Arc<Job>> {
        static N: AtomicU32 = AtomicU32::new(0);
        let id = format!("j{:x}{:x}", now_ms(), N.fetch_add(1, Ordering::Relaxed) % 4096);
        let meta = JobMeta { id: id.clone(), created: now_ms(), title: title.into(), params, direct, status: status::RUNNING.into(), ..Default::default() };
        let job = self.job_from(meta);
        std::fs::create_dir_all(&job.dir)?;
        write_json(&job.dir.join("job.json"), &job.meta())?;
        self.map.lock().unwrap_or_else(|e| e.into_inner()).insert(id, job.clone());
        self.start(&job);
        Ok(job)
    }

    /// Run from the first stage without a checkpoint. `false` when a run is going on.
    pub fn start(self: &Arc<Self>, job: &Arc<Job>) -> bool {
        if job.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        job.cancel.flag.store(false, Ordering::SeqCst);
        self.reconcile(job);
        job.edit(true, |m| {
            m.status = status::RUNNING.into();
            m.error = None;
        });
        let (me, job) = (self.clone(), job.clone());
        tokio::spawn(async move {
            me.run(&job).await;
            job.running.store(false, Ordering::SeqCst);
            job.bump();
        });
        true
    }

    /// Change the job's parameters (before a resume), e.g. another engine.
    pub fn set_params(&self, job: &Job, f: impl FnOnce(&mut Value)) {
        job.edit(true, |m| f(&mut m.params));
    }

    /// Redo from `stage` on: its checkpoint and every later one are removed, then the run starts
    /// again (finished earlier stages are kept). `Err` while a run is going on.
    pub fn redo_from(self: &Arc<Self>, job: &Arc<Job>, stage: &str) -> Result<(), String> {
        if job.is_running() {
            return Err("工作正在進行".into());
        }
        let specs = self.specs();
        let at = specs.iter().position(|s| s.id == stage).ok_or_else(|| format!("沒有這個階段：{stage}"))?;
        for sp in &specs[at..] {
            match std::fs::remove_file(job.stage_file(sp.id)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        job.edit(true, |m| {
            for sp in &specs[at..] {
                m.stages.insert(sp.id.to_string(), StageMeta { state: "pending".into(), ..Default::default() });
            }
        });
        self.start(job);
        Ok(())
    }

    /// The author's input for the waiting stage: written as its checkpoint, then the run
    /// goes on. `Err` when the job is not waiting there.
    pub fn submit(self: &Arc<Self>, job: &Arc<Job>, stage: &str, output: Value) -> Result<(), String> {
        if job.is_running() || job.status() != status::REVIEW {
            return Err("這個工作沒有在等微調".into());
        }
        let skipped = output["skipped"].as_bool().unwrap_or(false);
        let cp = json!({ "stage": stage, "output": output, "counts": {}, "elapsed_ms": 0, "finished": now_ms(), "skipped": skipped });
        write_json(&job.stage_file(stage), &cp).map_err(|e| e.to_string())?;
        self.start(job);
        Ok(())
    }

    /// 停止: cancel the run and wait (up to 5 s) until the worker is released; a job waiting
    /// for the author becomes `stopped` too.
    pub async fn stop(&self, job: &Job) {
        job.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(5), job.settled()).await;
        if !job.is_running() && job.status() == status::REVIEW {
            job.edit(true, |m| m.status = status::STOPPED.into());
        }
    }

    /// Remove a job and its folder (not while it runs).
    pub fn delete(&self, job: &Job) -> Result<(), String> {
        if job.is_running() {
            return Err("工作還在進行".into());
        }
        self.map.lock().unwrap_or_else(|e| e.into_inner()).remove(&job.id);
        std::fs::remove_dir_all(&job.dir).map_err(|e| e.to_string())
    }

    async fn run(&self, job: &Arc<Job>) {
        let direct = job.meta().direct;
        for sp in self.specs() {
            if job.checkpoint(sp.id).is_some() {
                job.stage_edit(sp.id, false, |s| s.state = "done".into());
                continue;
            }
            if job.cancel.flag.load(Ordering::SeqCst) {
                job.edit(true, |m| m.status = status::STOPPED.into());
                return;
            }
            if sp.wait {
                if direct {
                    let cp = json!({ "stage": sp.id, "output": { "skipped": true }, "counts": {}, "elapsed_ms": 0, "finished": now_ms(), "skipped": true });
                    if let Err(e) = write_json(&job.stage_file(sp.id), &cp) {
                        return self.fail(job, sp.id, StageError::new(500, e.to_string()));
                    }
                    job.stage_edit(sp.id, true, |s| {
                        s.state = "done".into();
                        s.skipped = true;
                    });
                    continue;
                }
                job.edit(true, |m| {
                    m.status = status::REVIEW.into();
                    let s = m.stages.entry(sp.id.to_string()).or_default();
                    s.state = "running".into();
                    s.started.get_or_insert(now_ms());
                });
                return;
            }
            let started = now_ms();
            job.stage_edit(sp.id, true, |s| {
                *s = StageMeta { state: "running".into(), started: Some(started), ..Default::default() };
            });
            let cx = StageCx { job: job.clone(), stage: sp.id };
            let r = tokio::select! {
                r = self.pipeline.run(&cx, sp.id) => Some(r),
                _ = job.cancel.wait() => None,
            };
            let elapsed = now_ms().saturating_sub(started);
            match r {
                None => {
                    job.edit(true, |m| {
                        m.status = status::STOPPED.into();
                        let s = m.stages.entry(sp.id.to_string()).or_default();
                        s.state = "pending".into();
                        s.elapsed_ms = elapsed;
                    });
                    return;
                }
                Some(Err(e)) => {
                    job.stage_edit(sp.id, false, |s| s.elapsed_ms = elapsed);
                    return self.fail(job, sp.id, e);
                }
                Some(Ok(output)) => {
                    let counts = job.meta().stages.get(sp.id).map(|s| s.counts.clone()).unwrap_or_default();
                    let cp = json!({ "stage": sp.id, "output": output, "counts": counts, "elapsed_ms": elapsed, "finished": now_ms() });
                    if let Err(e) = write_json(&job.stage_file(sp.id), &cp) {
                        return self.fail(job, sp.id, StageError::new(500, format!("無法寫入進度：{e}")));
                    }
                    job.stage_edit(sp.id, true, |s| {
                        s.state = "done".into();
                        s.elapsed_ms = elapsed;
                    });
                }
            }
        }
        job.edit(true, |m| m.status = status::DONE.into());
    }

    fn fail(&self, job: &Job, stage: &str, e: StageError) {
        let mut err = match e.extra {
            Value::Object(m) => Value::Object(m),
            _ => json!({}),
        };
        err["stage"] = stage.into();
        err["status"] = e.status.into();
        err["error"] = e.message.into();
        job.edit(true, |m| {
            m.status = status::FAILED.into();
            m.error = Some(err);
            m.stages.entry(stage.to_string()).or_default().state = "failed".into();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// Three stages, `b` waits on a gate when `block_b` is set; counts runs per stage.
    struct Fake {
        runs: [AtomicUsize; 4],
        block_b: AtomicBool,
        gate: Notify,
        dropped: AtomicBool,
    }

    const SPECS: &[StageSpec] = &[
        StageSpec { id: "a", label: "A", model: false, wait: false },
        StageSpec { id: "b", label: "B", model: true, wait: false },
        StageSpec { id: "r", label: "R", model: false, wait: true },
        StageSpec { id: "c", label: "C", model: false, wait: false },
    ];

    struct OnDrop<'a>(&'a AtomicBool);
    impl Drop for OnDrop<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    impl Pipeline for Fake {
        fn stages(&self) -> &'static [StageSpec] {
            SPECS
        }
        fn run<'a>(&'a self, cx: &'a StageCx, stage: &'static str) -> StageFuture<'a> {
            Box::pin(async move {
                let i = SPECS.iter().position(|s| s.id == stage).unwrap();
                self.runs[i].fetch_add(1, Ordering::SeqCst);
                match stage {
                    "a" => Ok(json!({ "n": 1 })),
                    "b" => {
                        let a = cx.output("a")?;
                        cx.chars(5);
                        if self.block_b.load(Ordering::SeqCst) {
                            let _g = OnDrop(&self.dropped);
                            self.gate.notified().await;
                        }
                        Ok(json!({ "n": a["n"].as_i64().unwrap() + 1 }))
                    }
                    _ => Ok(json!({ "b": cx.output("b")?["n"], "r": cx.output("r")? })),
                }
            })
        }
    }

    fn fake(block: bool) -> Arc<Fake> {
        Arc::new(Fake { runs: Default::default(), block_b: AtomicBool::new(block), gate: Notify::new(), dropped: AtomicBool::new(false) })
    }

    async fn until(job: &Job, f: impl Fn(&JobMeta) -> bool) {
        for _ in 0..400 {
            if f(&job.meta()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out: {:?}", job.meta());
    }

    #[tokio::test]
    async fn stages_run_in_order_and_review_waits_or_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = fake(false);
        let jobs = Jobs::open(dir.path().to_path_buf(), p.clone());
        let job = jobs.create(json!({}), false, "t").unwrap();
        until(&job, |m| m.status == status::REVIEW).await;
        job.settled().await;
        let v = jobs.view(&job);
        assert_eq!(v["state"], "review");
        assert_eq!(v["step"], 3);
        assert_eq!(v["stages"][2]["state"], "running");
        assert!(job.checkpoint("r").is_none());
        jobs.submit(&job, "r", json!({ "known": ["x"] })).unwrap();
        until(&job, |m| m.status == status::DONE).await;
        assert_eq!(job.output("c").unwrap(), json!({ "b": 2, "r": { "known": ["x"] } }));
        assert!(jobs.submit(&job, "r", json!({})).is_err(), "only a waiting job takes a review");

        // direct: the review is skipped
        let d = jobs.create(json!({}), true, "t").unwrap();
        until(&d, |m| m.status == status::DONE).await;
        assert_eq!(d.output("r").unwrap(), json!({ "skipped": true }));
        let v = jobs.view(&d);
        assert_eq!(v["stages"][2]["skipped"], true);
        assert_eq!(v["doneSteps"], 4);
    }

    #[tokio::test]
    async fn cancel_drops_the_stage_and_resume_redoes_only_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = fake(true);
        let jobs = Jobs::open(dir.path().to_path_buf(), p.clone());
        let job = jobs.create(json!({}), true, "t").unwrap();
        until(&job, |m| m.stages.get("b").is_some_and(|s| s.chars == Some(5))).await;
        assert_eq!(jobs.view(&job)["stages"][1]["chars"], 5);
        job.cancel();
        tokio::time::timeout(Duration::from_secs(2), job.settled()).await.expect("the worker is released");
        assert!(p.dropped.load(Ordering::SeqCst), "the blocked call was dropped");
        assert_eq!(job.status(), status::STOPPED);
        assert!(jobs.view(&job)["resumable"].as_bool().unwrap());
        p.block_b.store(false, Ordering::SeqCst);
        assert!(jobs.start(&job));
        until(&job, |m| m.status == status::DONE).await;
        assert_eq!(p.runs[0].load(Ordering::SeqCst), 1, "a finished stage is not redone");
        assert_eq!(p.runs[1].load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_killed_process_resumes_from_its_checkpoints() {
        let dir = tempfile::tempdir().unwrap();
        let p = fake(true);
        let jobs = Jobs::open(dir.path().to_path_buf(), p.clone());
        let job = jobs.create(json!({ "x": 1 }), true, "t").unwrap();
        until(&job, |m| m.stages.get("b").is_some_and(|s| s.state == "running")).await;
        let a_before = std::fs::read(job.stage_file("a")).unwrap();
        // "kill": a new process reads the same folder; the old one's work is gone
        let p2 = fake(false);
        let again = Jobs::open(dir.path().to_path_buf(), p2.clone());
        let j2 = again.get(&job.id).unwrap();
        let v = again.view(&j2);
        assert_eq!(v["state"], "interrupted");
        assert_eq!(v["doneSteps"], 1);
        assert_eq!(v["step"], 2, "上次做到第 2 步");
        assert_eq!(again.list().len(), 1);
        assert!(again.start(&j2));
        until(&j2, |m| m.status == status::DONE).await;
        assert_eq!(p2.runs[0].load(Ordering::SeqCst), 0, "stage a came from its checkpoint");
        assert_eq!(std::fs::read(j2.stage_file("a")).unwrap(), a_before);
        assert_eq!(j2.meta().params["x"], 1);
        job.cancel();
    }

    #[tokio::test]
    async fn a_corrupt_stage_file_is_redone_alone() {
        let dir = tempfile::tempdir().unwrap();
        let p = fake(false);
        let jobs = Jobs::open(dir.path().to_path_buf(), p.clone());
        let job = jobs.create(json!({}), true, "t").unwrap();
        until(&job, |m| m.status == status::DONE).await;
        job.settled().await;
        std::fs::write(job.stage_file("b"), b"{ not json").unwrap();
        let c_before = std::fs::read(job.stage_file("c")).unwrap();
        let again = Jobs::open(dir.path().to_path_buf(), p.clone());
        let j2 = again.get(&job.id).unwrap();
        assert_eq!(again.view(&j2)["stages"][1]["state"], "pending");
        assert!(again.start(&j2));
        until(&j2, |m| m.status == status::DONE).await;
        let runs: Vec<usize> = p.runs.iter().map(|r| r.load(Ordering::SeqCst)).collect();
        assert_eq!(runs, vec![1, 2, 0, 1], "only b again");
        assert_eq!(j2.output("b").unwrap()["n"], 2);
        assert_eq!(std::fs::read(j2.stage_file("c")).unwrap(), c_before);
    }

    /// 微調's 語言／長度: the job goes back to `b`, later stages are redone, earlier ones kept,
    /// and it waits for the author again.
    #[tokio::test]
    async fn redo_from_reruns_only_that_stage_and_waits_again() {
        let dir = tempfile::tempdir().unwrap();
        let p = fake(false);
        let jobs = Jobs::open(dir.path().to_path_buf(), p.clone());
        let job = jobs.create(json!({ "lang": "zh" }), false, "t").unwrap();
        until(&job, |m| m.status == status::REVIEW).await;
        job.settled().await;
        jobs.set_params(&job, |v| v["lang"] = "en".into());
        jobs.redo_from(&job, "b").unwrap();
        until(&job, |m| m.status == status::REVIEW && m.stages.get("b").is_some_and(|s| s.state == "done")).await;
        job.settled().await;
        let runs: Vec<usize> = p.runs.iter().map(|r| r.load(Ordering::SeqCst)).collect();
        assert_eq!(runs, vec![1, 2, 0, 0], "a kept, b again, still waiting before c");
        assert_eq!(job.meta().params["lang"], "en");
        assert!(jobs.redo_from(&job, "nope").is_err());
    }

    #[test]
    fn ids_are_folder_names() {
        assert!(valid_id("j18f3a2b4c5d1"));
        assert!(!valid_id("../x") && !valid_id("") && !valid_id("J1") && !valid_id("a/b"));
    }
}
