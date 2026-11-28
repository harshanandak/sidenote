//! One Ask transport, owned by a worker. No meeting text is written to disk or command lines.
use super::{AskProgress, Runner, ask, runner_process::Output, runners};
use crate::platform::CommandExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const IDLE: Duration = Duration::from_secs(600);
const MEETING_IDLE: Duration = Duration::from_secs(120);
const WARMUP: &str = "Reply exactly Ready. This is a synthetic connection warm-up. Do not use tools, read files, or take actions.";
const CAP: usize = 4 * 1024 * 1024;
static ACTOR: OnceLock<mpsc::Sender<Work>> = OnceLock::new();
static CLOSED: AtomicBool = AtomicBool::new(true);
static WARMING: AtomicUsize = AtomicUsize::new(0);
static CATALOG: Mutex<BTreeMap<String, Vec<Model>>> = Mutex::new(BTreeMap::new());
static CATALOG_VERSION: AtomicU64 = AtomicU64::new(0);
static OPENCODE_DISCOVERY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
}

pub fn catalog_version() -> u64 {
    CATALOG_VERSION.load(Ordering::Relaxed)
}
pub fn models(runner: &Runner) -> Vec<Model> {
    let kind = runners::kind(runner).unwrap_or("");
    if let Ok(catalog) = CATALOG.lock()
        && let Some(models) = catalog.get(kind)
    {
        return models.clone();
    }
    let defaults: &[(&str, &str)] = match kind {
        "claude" => &[
            ("claude-sonnet-5-5", "Sonnet 5.5"),
            ("claude-opus-5-5", "Opus 5.5"),
            ("claude-haiku-5-5", "Haiku 5.5"),
        ],
        "codex" => &[
            ("gpt-6.1-sol", "GPT-6.1 Sol"),
            ("gpt-6.1-terra", "GPT-6.1 Terra"),
            ("gpt-6-luna", "GPT-6 Luna"),
        ],
        _ => &[
            ("openai/gpt-6.1-terra-fast", "GPT-6.1 Terra Fast"),
            ("anthropic/claude-sonnet-5-5", "Sonnet 5.5 via OpenCode"),
        ],
    };
    defaults
        .iter()
        .map(|(id, name)| Model {
            id: (*id).into(),
            name: (*name).into(),
        })
        .collect()
}
fn remember_models(kind: &str, values: &[Value]) {
    let mut models = Vec::new();
    for value in values.iter().take(256) {
        let Some(id) = value
            .get("resolvedModel")
            .or_else(|| value.get("model"))
            .or_else(|| value.get("value"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if id.is_empty()
            || id.len() > 200
            || id.chars().any(char::is_control)
            || models.iter().any(|m: &Model| m.id == id)
        {
            continue;
        }
        let name = value
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(id);
        models.push(Model {
            id: id.into(),
            name: name.chars().filter(|c| !c.is_control()).take(120).collect(),
        });
    }
    if !models.is_empty()
        && let Ok(mut catalog) = CATALOG.lock()
        && catalog.get(kind) != Some(&models)
    {
        catalog.insert(kind.into(), models);
        CATALOG_VERSION.fetch_add(1, Ordering::Relaxed);
    }
}

/// OpenCode exposes a plain model list, rather than the two streaming protocols.
pub fn discover(runner: Runner) {
    if runners::kind(&runner) != Ok("opencode") || OPENCODE_DISCOVERY.swap(true, Ordering::SeqCst) {
        return;
    }
    thread::spawn(move || {
        let result = (|| {
            let mut child = crate::platform::worker_command(runners::executable(&runner).ok()?)
                .ok()?
                .arg("models")
                .creation_flags(0x08000000)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            let job = match crate::platform::ProcessJob::attach(&child) {
                Ok(job) => job,
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            };
            let mut stdout = child.stdout.take()?;
            let reader = thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stdout
                    .by_ref()
                    .take((CAP + 1) as u64)
                    .read_to_end(&mut bytes);
                bytes
            });
            let deadline = Instant::now() + Duration::from_secs(30);
            let success = loop {
                if ask::stop_requested() || Instant::now() >= deadline {
                    break false;
                }
                if let Ok(Some(status)) = child.try_wait() {
                    break status.success();
                }
                thread::sleep(Duration::from_millis(100));
            };
            drop(job);
            let _ = child.kill();
            let _ = child.wait();
            let bytes = reader.join().ok()?;
            if !success || bytes.len() > CAP {
                return None;
            }
            let text = String::from_utf8(bytes).ok()?;
            let values: Vec<Value> = text
                .split_whitespace()
                .filter(|s| {
                    s.contains('/')
                        && s.len() <= 200
                        && s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || "/._-:#".contains(c))
                })
                .map(|id| json!({"model":id,"displayName":id}))
                .collect();
            remember_models("opencode", &values);
            Some(())
        })();
        if result.is_none() {
            OPENCODE_DISCOVERY.store(false, Ordering::SeqCst);
        }
    });
}

#[derive(Clone, PartialEq, Eq)]
struct Key {
    runner: Runner,
    chat: String,
}
enum Reply {
    Progress(AskProgress),
    Result(Result<String, String>),
}
struct Warmup {
    ready: SyncSender<Result<(), String>>,
}
impl Warmup {
    fn new(ready: SyncSender<Result<(), String>>) -> Self {
        WARMING.fetch_add(1, Ordering::SeqCst);
        Self { ready }
    }
}
impl Drop for Warmup {
    fn drop(&mut self) {
        WARMING.fetch_sub(1, Ordering::SeqCst);
    }
}
fn starting(runner: &Runner) -> Option<AskProgress> {
    (runners::kind(runner) == Ok("codex") && WARMING.load(Ordering::SeqCst) > 0)
        .then_some(AskProgress::StartingCodex)
}
enum Work {
    Warm(Key, Option<Warmup>),
    Release,
    Reset(String),
    Ask {
        key: Key,
        prompt: String,
        cancel: std::sync::Arc<AtomicBool>,
        reply: SyncSender<Reply>,
    },
    Recording(bool),
    Close,
}

fn actor() -> &'static mpsc::Sender<Work> {
    ACTOR.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        CLOSED.store(false, Ordering::SeqCst);
        thread::spawn(move || {
            serve(rx);
            CLOSED.store(true, Ordering::SeqCst);
        });
        tx
    })
}
pub fn warm(runner: Runner, chat: String) {
    if !ask::stop_requested()
        && runner.stdin
        && matches!(runners::kind(&runner), Ok("claude" | "codex"))
    {
        let _ = actor().send(Work::Warm(Key { runner, chat }, None));
    }
}
/// Prime only Codex, using synthetic text. Receiving readiness never blocks the UI.
pub fn warm_meeting(runner: Runner, chat: String) -> Receiver<Result<(), String>> {
    let (ready, receiver) = mpsc::sync_channel(1);
    if !ask::stop_requested() && runner.stdin && runners::kind(&runner) == Ok("codex") {
        let _ = actor().send(Work::Warm(Key { runner, chat }, Some(Warmup::new(ready))));
    } else {
        let _ = ready.send(Err("Codex is not available for warming.".into()));
    }
    receiver
}
pub fn recording(active: bool) {
    if !ask::stop_requested() {
        let _ = actor().send(Work::Recording(active));
    }
}
pub fn reset(chat: &str) {
    if let Some(actor) = ACTOR.get() {
        let _ = actor.send(Work::Reset(chat.into()));
    }
}
/// Release the idle transport after any current answer has completed.
pub fn release() {
    if let Some(actor) = ACTOR.get() {
        let _ = actor.send(Work::Release);
    }
}
pub(super) fn close() {
    if let Some(actor) = ACTOR.get() {
        let _ = actor.send(Work::Close);
    }
}
pub(super) fn closed() -> bool {
    CLOSED.load(Ordering::SeqCst)
}

/// Stands in for the helper, so a test can drive the real Ask path with no helper installed.
/// Debug builds and tests only: a release build does not contain it.
#[cfg(debug_assertions)]
pub static STAND_IN: Mutex<Option<StandIn>> = Mutex::new(None);
#[cfg(debug_assertions)]
pub type StandIn = fn(&Runner, &str, &dyn Fn(AskProgress)) -> Result<String, String>;

/// Blocking on an Ask worker, never the UI thread. Cancellation also stops an in-flight transport.
pub fn run(
    runner: &Runner,
    prompt: String,
    chat: &str,
    progress: impl Fn(AskProgress),
) -> Result<String, String> {
    if runners::kind(runner) == Ok("opencode") {
        let runner = super::ask::read_only_runner(runner.clone())?;
        let guard = super::agent_consent::authorize(&super::state_dir(), runner.model.as_deref())
            .map_err(super::agent_consent::Refusal::message)?;
        return run_authorized(&runner, prompt, chat, progress, Some(guard));
    }
    run_authorized(runner, prompt, chat, progress, None)
}

pub(super) fn run_authorized(
    runner: &Runner,
    prompt: String,
    chat: &str,
    progress: impl Fn(AskProgress),
    consent_guard: Option<std::fs::File>,
) -> Result<String, String> {
    #[cfg(debug_assertions)]
    if let Some(stand_in) = *STAND_IN.lock().unwrap() {
        progress(AskProgress::Started {
            runner: runner.name.clone(),
        });
        return stand_in(runner, &prompt, &progress);
    }
    let _activity = crate::updates::activity()
        .map_err(|_| "An update is being installed. Try again after restart.")?;
    if let Some(guard) = consent_guard {
        use super::runner_process::{Invocation, run_classified};
        return run_classified(
            Invocation::OpenCodeAsk(runner, guard),
            prompt,
            &super::state_dir(),
            Duration::from_secs(600),
            false,
            progress,
        )
        .map_err(|error| error.message);
    }
    if !runner.stdin || !matches!(runners::kind(runner)?, "claude" | "codex") {
        return super::run_runner(runner, prompt, &super::state_dir(), progress);
    }
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    if let Some(event) = starting(runner) {
        progress(event);
    }
    let (tx, rx) = mpsc::sync_channel(8);
    actor()
        .send(Work::Ask {
            key: Key {
                runner: runner.clone(),
                chat: chat.into(),
            },
            prompt,
            cancel: cancel.clone(),
            reply: tx,
        })
        .map_err(|_| "The agent session stopped. Restart Sidenote and try again.")?;
    let began = Instant::now();
    loop {
        if ask::stop_requested() || ask::cancel_requested() {
            cancel.store(true, Ordering::SeqCst);
            return Err(ask::STOPPED.into());
        }
        if began.elapsed() >= IDLE {
            cancel.store(true, Ordering::SeqCst);
            return Err("The agent exceeded its time limit.".into());
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Reply::Progress(event)) => progress(event),
            Ok(Reply::Result(result)) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("The agent session closed. Try again.".into()),
        }
    }
}

struct Lifetime {
    recording: bool,
    ended: Option<Instant>,
    activity: Instant,
}
impl Lifetime {
    fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.activity) >= if self.recording { MEETING_IDLE } else { IDLE }
            || (!self.recording
                && self
                    .ended
                    .is_some_and(|end| now.duration_since(end) >= IDLE))
    }
    fn recording(&mut self, recording: bool, now: Instant) {
        if self.recording && !recording {
            self.ended = Some(now);
        }
        if recording {
            self.ended = None;
        }
        self.recording = recording;
    }
}

fn serve(rx: Receiver<Work>) {
    let mut lifetime = Lifetime {
        recording: false,
        ended: None,
        activity: Instant::now(),
    };
    let mut desired: Option<Key> = None;
    let mut session: Option<Session> = None;
    let mut retry_at = Instant::now();
    while !ask::stop_requested() {
        let work = rx.recv_timeout(Duration::from_secs(1));
        match work {
            Ok(Work::Close) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Work::Release) => {
                session = None;
                desired = None;
            }
            Ok(Work::Recording(active)) => {
                if active != lifetime.recording {
                    session = None;
                    desired = None;
                }
                lifetime.recording(active, Instant::now());
                continue;
            }
            Ok(Work::Reset(chat)) => {
                if desired.as_ref().is_some_and(|key| key.chat == chat) {
                    session = None;
                    lifetime.activity = Instant::now();
                    retry_at = Instant::now();
                }
            }
            Ok(Work::Warm(key, ready)) => {
                lifetime.activity = Instant::now();
                if desired.as_ref() != Some(&key) {
                    session = None;
                    retry_at = Instant::now();
                }
                desired = Some(key.clone());
                if let Some(warmup) = ready {
                    let cancel = AtomicBool::new(false);
                    let result = (|| {
                        if session.as_mut().is_none_or(|s| !s.alive()) {
                            session = Some(Session::start(&key, &cancel)?);
                        }
                        session.as_mut().unwrap().prime(&key.runner, &cancel)
                    })();
                    if result.is_err() {
                        session = None;
                        desired = None;
                    }
                    lifetime.activity = Instant::now();
                    let _ = warmup.ready.send(result);
                }
            }
            Ok(Work::Ask {
                key,
                prompt,
                cancel,
                reply,
            }) => {
                if cancel.load(Ordering::SeqCst) {
                    continue;
                }
                lifetime.activity = Instant::now();
                if desired.as_ref() != Some(&key) {
                    session = None;
                }
                desired = Some(key.clone());
                let result = (|| {
                    if session.as_mut().is_none_or(|s| !s.alive()) {
                        session = Some(Session::start(&key, &cancel)?);
                    }
                    let progress = |event| {
                        let _ = reply.send(Reply::Progress(event));
                    };
                    session
                        .as_mut()
                        .unwrap()
                        .turn(&key.runner, prompt, &cancel, &progress)
                })();
                if result.is_err() {
                    session = None;
                    retry_at = Instant::now() + Duration::from_secs(5);
                }
                lifetime.activity = Instant::now();
                let _ = reply.send(Reply::Result(result));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if lifetime.expired(Instant::now()) {
            session = None;
            desired = None;
            lifetime.ended = None;
        }
        if session.as_mut().is_some_and(|s| !s.alive()) {
            session = None;
        }
        if session.is_none()
            && Instant::now() >= retry_at
            && let Some(key) = &desired
        {
            match Session::start(key, &AtomicBool::new(false)) {
                Ok(warm) => session = Some(warm),
                Err(_) => retry_at = Instant::now() + Duration::from_secs(5),
            }
        }
    }
    drop(session);
}

enum Stream {
    Event(Value, usize),
    Failed,
    Closed,
}
struct Session {
    child: Child,
    job: Option<crate::platform::ProcessJob>,
    input: Option<SyncSender<Vec<u8>>>,
    events: Option<Receiver<Stream>>,
    threads: Vec<JoinHandle<()>>,
    kind: String,
    thread_id: Option<String>,
    next_id: u64,
    answered: bool,
    /// The model the agent reported when it opened the conversation, if it did.
    model: Option<String>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.events.take();
        self.input.take();
        self.job.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
impl Session {
    fn start(key: &Key, cancel: &AtomicBool) -> Result<Self, String> {
        runners::validate(&key.runner)?;
        #[cfg(target_os = "linux")]
        if cfg!(target_os = "linux") {
            return Err(runners::NETWORK_UNAVAILABLE.into());
        }
        let kind = runners::kind(&key.runner)?;
        let mut command = Command::new(runners::executable(&key.runner)?);
        let root = super::state_dir();
        std::fs::create_dir_all(&root).map_err(|_| "Could not open the app data folder.")?;
        if kind == "claude" {
            command.args(
                key.runner
                    .args
                    .iter()
                    .map(|a| a.replace("{model}", key.runner.model.as_deref().unwrap_or(""))),
            );
            command.args(["--input-format", "stream-json"]);
        } else {
            command.args(["app-server", "--stdio"]);
        }
        command.current_dir(root);
        let mut session = Self::spawn(command, kind)?;
        session.initialize(&key.runner, cancel)?;
        Ok(session)
    }
    fn spawn(mut command: Command, kind: &str) -> Result<Self, String> {
        crate::platform::contain_command(&mut command).map_err(|error| error.to_string())?;
        let mut child = command
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "Could not start the agent session.")?;
        let job = match crate::platform::ProcessJob::attach(&child) {
            Ok(job) => job,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Could not contain the agent session.".into());
            }
        };
        let stdout = child.stdout.take().ok_or("Could not open agent output.")?;
        let stderr = child.stderr.take().ok_or("Could not open agent status.")?;
        Self::from_child(child, Some(job), stdout, stderr, kind)
    }
    fn from_child(
        mut child: Child,
        job: Option<crate::platform::ProcessJob>,
        output: impl Read + Send + 'static,
        mut status: impl Read + Send + 'static,
        kind: &str,
    ) -> Result<Self, String> {
        let mut stdin = child.stdin.take().ok_or("Could not open agent input.")?;
        let (events_tx, events) = mpsc::sync_channel(16);
        let (input, input_rx) = mpsc::sync_channel::<Vec<u8>>(1);
        let writer_tx = events_tx.clone();
        let writer = thread::spawn(move || {
            for bytes in input_rx {
                if stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_err() {
                    let _ = writer_tx.send(Stream::Failed);
                    break;
                }
            }
        });
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                match reader
                    .by_ref()
                    .take((CAP + 1) as u64)
                    .read_until(b'\n', &mut bytes)
                {
                    Ok(0) => break,
                    Ok(_) if bytes.len() <= CAP => {
                        let event = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                        if events_tx.send(Stream::Event(event, bytes.len())).is_err() {
                            return;
                        }
                    }
                    _ => {
                        let _ = events_tx.send(Stream::Failed);
                        return;
                    }
                }
            }
            let _ = events_tx.send(Stream::Closed);
        });
        let errors = thread::spawn(move || {
            let _ = std::io::copy(&mut status, &mut std::io::sink());
        });
        Ok(Self {
            child,
            job,
            input: Some(input),
            events: Some(events),
            threads: vec![writer, reader, errors],
            kind: kind.into(),
            thread_id: None,
            next_id: 0,
            answered: false,
            model: None,
        })
    }
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    fn send(&self, event: Value) -> Result<(), String> {
        let mut bytes =
            serde_json::to_vec(&event).map_err(|_| "Could not prepare the agent question.")?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err("The meeting is too large for this question.".into());
        }
        bytes.push(b'\n');
        self.input
            .as_ref()
            .ok_or("Agent input closed.")?
            .send(bytes)
            .map_err(|_| "The agent session closed. Try again.".into())
    }
    fn event(
        &self,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<Option<(Value, usize)>, String> {
        if ask::stop_requested() || cancel.load(Ordering::SeqCst) {
            return Err(ask::STOPPED.into());
        }
        if Instant::now() >= deadline {
            return Err("The agent session timed out. Try again.".into());
        }
        match self
            .events
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_millis(100))
        {
            Ok(Stream::Event(event, bytes)) => Ok(Some((event, bytes))),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            _ => Err("The agent session closed. Check its sign-in and try again.".into()),
        }
    }
    fn rpc(&mut self, method: &str, params: Value, cancel: &AtomicBool) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let request_id = id.to_string();
        if self.kind == "claude" {
            self.send(
                json!({"type":"control_request", "request_id":id.to_string(), "request":params}),
            )?;
        } else {
            self.send(json!({"id":id, "method":method, "params":params}))?;
        }
        let deadline = Instant::now() + Duration::from_secs(40);
        let mut bytes = 0;
        loop {
            let Some((event, size)) = self.event(deadline, cancel)? else {
                continue;
            };
            bytes += size;
            if bytes > CAP {
                return Err("The agent exceeded its output limit.".into());
            }
            if self.kind == "claude" {
                if event["type"] == "control_response"
                    && event["response"]["request_id"].as_str() == Some(request_id.as_str())
                {
                    if event["response"]["subtype"] == "error" {
                        return Err("The agent could not initialize this session.".into());
                    }
                    return Ok(event["response"]["response"].clone());
                }
            } else if event["id"] == id {
                if event.get("error").is_some() {
                    return Err("The agent rejected this session request. Check the chosen model and sign-in.".into());
                }
                return Ok(event["result"].clone());
            } else {
                self.decline_request(&event)?;
            }
        }
    }
    fn initialize(&mut self, runner: &Runner, cancel: &AtomicBool) -> Result<(), String> {
        if self.kind == "claude" {
            let response = self.rpc("initialize", json!({"subtype":"initialize"}), cancel)?;
            if let Some(models) = response["models"].as_array() {
                remember_models("claude", models);
            }
        } else {
            self.rpc("initialize", json!({"clientInfo":{"name":"sidenote","version":env!("CARGO_PKG_VERSION")}, "capabilities":{"experimentalApi":true}}), cancel)?;
            self.send(json!({"method":"initialized","params":{}}))?;
            let mut cursor = Value::Null;
            let mut models = Vec::new();
            for _ in 0..8 {
                let response =
                    self.rpc("model/list", json!({"limit":100,"cursor":cursor}), cancel)?;
                if let Some(page) = response["data"].as_array() {
                    models.extend(page.iter().cloned());
                }
                cursor = response["nextCursor"].clone();
                if cursor.is_null() {
                    break;
                }
            }
            remember_models("codex", &models);
            let response = self.rpc("thread/start", json!({"cwd":super::state_dir(),"model":runner.model,"ephemeral":true,"sandbox":"read-only","approvalPolicy":"never"}), cancel)?;
            self.thread_id = response["thread"]["id"].as_str().map(str::to_owned);
            self.model = response["model"].as_str().and_then(runners::reported_model);
            if self.thread_id.is_none() {
                return Err("The agent did not open a conversation.".into());
            }
        }
        Ok(())
    }
    fn decline_request(&self, event: &Value) -> Result<(), String> {
        if self.kind == "codex" && event.get("id").is_some() && event.get("method").is_some() {
            self.send(json!({"id":event["id"],"error":{"code":-32601,"message":"Ask does not approve interactive requests."}}))?;
        }
        Ok(())
    }
    fn turn(
        &mut self,
        runner: &Runner,
        prompt: String,
        cancel: &AtomicBool,
        progress: &impl Fn(AskProgress),
    ) -> Result<String, String> {
        self.turn_until(runner, prompt, cancel, progress, Instant::now() + IDLE)
    }
    fn prime(&mut self, runner: &Runner, cancel: &AtomicBool) -> Result<(), String> {
        if !self.answered {
            self.turn_until(
                runner,
                WARMUP.into(),
                cancel,
                &|_| {},
                Instant::now() + Duration::from_secs(30),
            )?;
        }
        Ok(())
    }
    fn turn_until(
        &mut self,
        runner: &Runner,
        prompt: String,
        cancel: &AtomicBool,
        progress: &impl Fn(AskProgress),
        deadline: Instant,
    ) -> Result<String, String> {
        progress(AskProgress::Started {
            runner: runner.name.clone(),
        });
        if let Some(model) = self.model.clone() {
            progress(AskProgress::Model { model });
        }
        let mut turn_request = None;
        if self.kind == "claude" {
            self.send(json!({"type":"user","message":{"role":"user","content":prompt}}))?;
        } else {
            let effort = runner
                .args
                .iter()
                .find_map(|arg| arg.strip_prefix("model_reasoning_effort="))
                .unwrap_or("medium");
            self.next_id += 1;
            turn_request = Some(self.next_id);
            self.send(json!({"id":self.next_id,"method":"turn/start","params":{"threadId":self.thread_id,"effort":effort,"input":[{"type":"text","text":prompt}]}}))?;
        }
        let mut output = Output::default();
        let mut bytes = 0;
        let mut text_item = None;
        loop {
            let Some((event, size)) = self.event(deadline, cancel)? else {
                continue;
            };
            bytes += size;
            if bytes > CAP {
                return Err("The agent exceeded its output limit.".into());
            }
            if self.kind == "claude" {
                output.line(&event.to_string(), "claude", true, progress);
                if event["type"] == "result" {
                    if output.failed {
                        return Err("The agent could not answer. Check the selected model and sign-in, then try again.".into());
                    }
                    break;
                }
            } else {
                if event.get("id").and_then(Value::as_u64) == turn_request
                    && event.get("error").is_some()
                {
                    return Err("The agent rejected this question. Check the selected model and thinking level.".into());
                }
                self.decline_request(&event)?;
                let params = &event["params"];
                match event["method"].as_str() {
                    Some("item/agentMessage/delta") => {
                        if let Some(text) = params["delta"].as_str() {
                            if text_item.as_ref() != Some(&params["itemId"]) {
                                output.final_text.clear();
                                text_item = Some(params["itemId"].clone());
                            }
                            output.final_text.push_str(text);
                            progress(AskProgress::Text {
                                text: output.final_text.clone(),
                            });
                        }
                    }
                    Some("item/completed") => {
                        let item = &params["item"];
                        if item["type"] == "agentMessage"
                            && let Some(text) = item["text"].as_str()
                        {
                            output.final_text = text.into();
                            progress(AskProgress::Text { text: text.into() });
                        }
                    }
                    Some("turn/completed") => {
                        if params["turn"]["status"] != "completed" {
                            return Err("The agent could not finish the answer. Try again.".into());
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
        if output.final_text.trim().is_empty() {
            return Err("The agent returned no answer. Try again.".into());
        }
        self.answered = true;
        Ok(output.final_text.trim().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_persistent_sessions_refuse_agents_before_executable_lookup() {
        let configured: Vec<Runner> =
            serde_json::from_str(include_str!("../../default-runners.json")).unwrap();
        for runner in configured {
            let key = Key {
                runner,
                chat: "synthetic".into(),
            };
            assert_eq!(
                Session::start(&key, &AtomicBool::new(false)).err().unwrap(),
                runners::NETWORK_UNAVAILABLE
            );
        }
    }
    #[test]
    fn queued_warmup_reports_starting_for_codex_and_releases_on_drop() {
        let runners: Vec<Runner> =
            serde_json::from_str(include_str!("../../default-runners.json")).unwrap();
        let codex = runners
            .iter()
            .find(|r| runners::kind(r) == Ok("codex"))
            .unwrap();
        let claude = runners
            .iter()
            .find(|r| runners::kind(r) == Ok("claude"))
            .unwrap();
        let (ready, _) = mpsc::sync_channel(1);
        assert!(starting(codex).is_none());
        let warmup = Warmup::new(ready);
        assert_eq!(starting(codex), Some(AskProgress::StartingCodex));
        assert!(starting(claude).is_none());
        drop(warmup);
        assert!(starting(codex).is_none());
    }
    #[test]
    fn warm_lifetime_covers_recording_and_expires_at_either_ten_minute_limit() {
        let now = Instant::now();
        let mut lifetime = Lifetime {
            recording: false,
            ended: None,
            activity: now,
        };
        assert!(!lifetime.expired(now + IDLE - Duration::from_millis(1)));
        assert!(lifetime.expired(now + IDLE));
        lifetime.recording(true, now);
        assert!(!lifetime.expired(now + MEETING_IDLE - Duration::from_millis(1)));
        assert!(lifetime.expired(now + MEETING_IDLE));
        assert!(lifetime.expired(now + IDLE * 3));
        lifetime.activity = now + IDLE * 3;
        assert!(!lifetime.expired(now + IDLE * 3));
        lifetime.recording(false, now + IDLE * 3);
        lifetime.activity = now + IDLE * 3;
        assert!(!lifetime.expired(now + IDLE * 4 - Duration::from_millis(1)));
        lifetime.activity = now + IDLE * 4;
        assert!(lifetime.expired(now + IDLE * 4));
    }
    #[test]
    fn a_different_chat_or_model_cannot_inherit_a_warm_conversation() {
        let runner: Runner =
            serde_json::from_str::<Vec<Runner>>(include_str!("../../default-runners.json"))
                .unwrap()
                .remove(0);
        let a = Key {
            runner: runner.clone(),
            chat: "meeting-one".into(),
        };
        let b = Key {
            runner: runner.clone(),
            chat: "meeting-two".into(),
        };
        assert!(a != b);
        let mut c = a.clone();
        c.runner.model = Some("other-model".into());
        assert!(a != c);
    }
    #[test]
    fn streaming_session_reuses_one_process_then_recovers_from_death_and_cancels() {
        if std::env::var_os("SIDENOTE_ASK_PROTOCOL_FIXTURE").is_some() {
            agent_fixture();
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let fixture = |kind| {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "core::ask_session::tests::streaming_session_reuses_one_process_then_recovers_from_death_and_cancels",
                ])
                .current_dir(root.path())
                .env_clear()
                .env("SIDENOTE_DATA_DIR", root.path())
                .env("SIDENOTE_ASK_PROTOCOL_FIXTURE", "1")
                .creation_flags(0x08000000)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            // Libtest writes to stdout, even when the fixture owns the protocol.
            let output = child.stderr.take().unwrap();
            let status = child.stdout.take().unwrap();
            Session::from_child(child, None, output, status, kind).unwrap()
        };
        for kind in ["claude", "codex"] {
            let runner =
                serde_json::from_str::<Vec<Runner>>(include_str!("../../default-runners.json"))
                    .unwrap()
                    .into_iter()
                    .find(|r| runners::kind(r) == Ok(kind))
                    .unwrap();
            let cancel = AtomicBool::new(false);
            let mut session = fixture(kind);
            session.initialize(&runner, &cancel).unwrap();
            assert!(!session.answered);
            let pid = session.child.id();
            let events = std::cell::RefCell::new(Vec::new());
            let first = session
                .turn(&runner, "first".into(), &cancel, &|e| {
                    events.borrow_mut().push(e)
                })
                .unwrap();
            assert_eq!(first, "answer 1");
            assert_eq!(
                session
                    .turn(&runner, "second".into(), &cancel, &|_| {})
                    .unwrap(),
                "answer 2"
            );
            assert_eq!(session.child.id(), pid);
            let last_request = session.next_id;
            session.prime(&runner, &cancel).unwrap();
            assert_eq!(
                session.next_id, last_request,
                "An answered session was primed twice"
            );
            assert!(
                events
                    .borrow()
                    .iter()
                    .any(|e| matches!(e,AskProgress::Text {text} if text=="answer "))
            );
            // Each transport reports the model the agent named, and only that.
            let reported: Vec<_> = events
                .borrow()
                .iter()
                .filter_map(|e| match e {
                    AskProgress::Model { model } => Some(model.clone()),
                    _ => None,
                })
                .collect();
            assert_eq!(reported, [format!("fixture-{kind}-model")]);
            assert!(
                session
                    .turn(&runner, "die".into(), &cancel, &|_| {})
                    .is_err()
            );
            drop(session);
            let mut restarted = fixture(kind);
            restarted.initialize(&runner, &cancel).unwrap();
            assert_eq!(
                restarted
                    .turn(&runner, "new".into(), &cancel, &|_| {})
                    .unwrap(),
                "answer 1"
            );
            let cancelled = AtomicBool::new(true);
            assert_eq!(
                restarted
                    .turn(&runner, "cancel".into(), &cancelled, &|_| {})
                    .unwrap_err(),
                ask::STOPPED
            );
            if kind == "codex" {
                let mut warming = fixture(kind);
                warming.initialize(&runner, &cancel).unwrap();
                warming.prime(&runner, &cancel).unwrap();
                assert!(warming.answered);
                let last_request = warming.next_id;
                warming.prime(&runner, &cancel).unwrap();
                assert_eq!(warming.next_id, last_request);
                assert_eq!(
                    warming
                        .turn(&runner, "after warm-up".into(), &cancel, &|_| {})
                        .unwrap(),
                    "answer 2"
                );
            }
        }
    }

    fn agent_fixture() {
        let stdin = std::io::stdin();
        let mut output = std::io::stderr().lock();
        let mut turn = 0;
        for line in stdin.lock().lines() {
            let event: Value = serde_json::from_str(&line.unwrap()).unwrap();
            let method = event["method"].as_str().unwrap_or("");
            let responses = if event["type"] == "control_request" {
                vec![
                    json!({"type":"control_response","response":{"subtype":"success","request_id":event["request_id"],"response":{"models":[]}}}),
                ]
            } else if method == "initialize" {
                vec![json!({"id":event["id"],"result":{}})]
            } else if method == "model/list" {
                vec![json!({"id":event["id"],"result":{"data":[],"nextCursor":null}})]
            } else if method == "thread/start" {
                assert_eq!(event["params"]["sandbox"], "read-only");
                assert_eq!(event["params"]["approvalPolicy"], "never");
                assert_eq!(event["params"]["ephemeral"], true);
                vec![
                    json!({"id":event["id"],"result":{"thread":{"id":"fixture-thread"},"model":"fixture-codex-model"}}),
                ]
            } else if event["type"] == "user" || method == "turn/start" {
                if event["message"]["content"] == "die"
                    || event["params"]["input"][0]["text"] == "die"
                {
                    std::process::exit(3);
                }
                turn += 1;
                if method == "turn/start" {
                    vec![
                        json!({"method":"item/agentMessage/delta","params":{"threadId":"fixture-thread","itemId":turn.to_string(),"delta":"answer "}}),
                        json!({"id":event["id"],"result":{"turn":{"id":turn.to_string()}}}),
                        json!({"method":"item/agentMessage/delta","params":{"threadId":"fixture-thread","itemId":turn.to_string(),"delta":turn.to_string()}}),
                        json!({"method":"turn/completed","params":{"turn":{"status":"completed"}}}),
                    ]
                } else {
                    vec![
                        json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"answer "}}}),
                        json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":turn.to_string()}}}),
                        json!({"type":"assistant","message":{"model":"fixture-claude-model","content":[{"type":"text","text":format!("answer {turn}")}]}}),
                        json!({"type":"result","result":format!("answer {turn}"),"is_error":false}),
                    ]
                }
            } else {
                continue;
            };
            for response in responses {
                writeln!(output, "{response}").unwrap();
                output.flush().unwrap();
            }
        }
    }
}
