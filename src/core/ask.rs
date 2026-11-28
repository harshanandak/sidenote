use super::{AskProgress, Runner};
use super::{Meeting, Message};
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
/// The Ask rules: a fixed opening and closing around your answer-style instructions.
pub(super) fn rules() -> String {
    super::agent_instructions::effective(&super::state_dir(), super::agent_instructions::Agent::Ask)
}
/// One-tap questions. The window shows the first four as chips and the rest under "More".
pub const SUGGESTIONS: &[&str] = &[
    "What did we decide?",
    "List my tasks",
    "Draft a follow-up message",
    "Write a TL;DR",
    "What do I need to do next?",
    "What is still unclear?",
    "Draft a short follow-up message",
];
/// The last twelve messages, each cut to 4,000 characters, as the prompt's conversation.
pub(super) fn chat(messages: &[Message]) -> String {
    messages[messages.len().saturating_sub(12)..]
        .iter()
        .map(|m| {
            format!(
                "{}: {}",
                if m.role == "user" { "Me" } else { "You" },
                m.text.chars().take(4000).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join(
            "

",
        )
}
pub fn prompt(rules: &str, meeting: &Meeting, messages: &[Message]) -> String {
    let chat = chat(messages);
    format!(
        "{rules}

<summary>
{}
</summary>

<transcript>
{}
</transcript>

Conversation so far (answer the last message from Me):

{chat}

You:",
        meeting.summary_raw, meeting.transcript_raw
    )
}
/// History must end with the owner's question.
pub(super) fn check_question(history: &[Message]) -> Result<(), String> {
    if history
        .last()
        .is_none_or(|m| m.role != "user" || m.text.trim().is_empty())
    {
        return Err("Ask needs a final user question in history".into());
    }
    Ok(())
}
/// Finds the configured runner. It does not remember it: the saved default is only what
/// Settings choose, so a runner picked for one question never becomes the next launch default.
pub(super) fn pick_runner(name: &str) -> Result<Runner, String> {
    super::list_runners()?
        .into_iter()
        .find(|r| r.name == name)
        .ok_or_else(|| "The selected runner is not configured".into())
}
/// Every Ask is read only, including if peer folders arrive while a runner is active.
pub(super) fn read_only_runner(mut runner: Runner) -> Result<Runner, String> {
    match super::runners::kind(&runner)? {
        "claude" => {
            if !runner.tools {
                return Ok(runner);
            }
            runner.args = [
                "-p",
                "--tools",
                "Read,Grep,Glob",
                "--allowedTools",
                "Read,Grep,Glob",
                "--strict-mcp-config",
                "--permission-prompts",
                "none",
                "--no-session-persistence",
                "--output-format",
                "stream-json",
                "--verbose",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
            if runner.model.is_some() {
                runner.args.extend(["--model".into(), "{model}".into()]);
            }
            runner.stdin = true;
            runner.tools = true;
            for root in [super::meetings_dir(), super::peers_dir()]
                .into_iter()
                .filter(|p| p.is_dir())
            {
                runner.args.push("--add-dir".into());
                runner.args.push(root.to_string_lossy().into_owned());
            }
        }
        "codex" => {
            runner.args = [
                "exec",
                "--sandbox",
                "read-only",
                "-c",
                "approval_policy=\"never\"",
                "--ephemeral",
                "--skip-git-repo-check",
                "--json",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
            if runner.model.is_some() {
                runner.args.extend(["--model".into(), "{model}".into()]);
            }
            runner.args.push("-".into());
            runner.stdin = true;
            runner.tools = true;
        }
        "opencode" => {
            runner.args = ["run", "--format", "json", "--agent", "plan"]
                .into_iter()
                .map(str::to_owned)
                .collect();
            if runner.model.is_some() {
                runner.args.extend(["--model".into(), "{model}".into()]);
            }
            runner.stdin = true;
        }
        _ => unreachable!(),
    }
    Ok(runner)
}

/// Prepares a transport without sending meeting content or making a model request.
pub fn prewarm(runner: Runner, options: &super::ask_options::Options, chat: String) {
    if let Ok(runner) = read_only_runner(runner).and_then(|r| options.runner(r)) {
        super::ask_session::warm(runner, chat);
    }
}
pub fn prewarm_meeting(
    runner: Runner,
    options: &super::ask_options::Options,
    chat: String,
) -> Result<std::sync::mpsc::Receiver<Result<(), String>>, String> {
    let runner = options.runner(read_only_runner(runner)?)?;
    Ok(super::ask_session::warm_meeting(runner, chat))
}

/// Sends the final progress event for a finished call and passes the result through.
pub(super) fn report(
    result: Result<String, String>,
    progress: &impl Fn(AskProgress),
) -> Result<String, String> {
    match &result {
        Ok(_) => progress(AskProgress::Finished),
        Err(message) => progress(AskProgress::Failed {
            message: message.clone(),
        }),
    }
    result
}
/// Blocking final-answer API. UI callers should use `spawn`.
pub fn ask(meeting_id: &str, history: &[Message], runner_name: &str) -> Result<String, String> {
    ask_with_progress(meeting_id, history, runner_name, |_| {})
}
pub fn ask_with_progress(
    meeting_id: &str,
    history: &[Message],
    runner_name: &str,
    progress: impl Fn(AskProgress),
) -> Result<String, String> {
    ask_with_options(
        meeting_id,
        history,
        runner_name,
        &Default::default(),
        progress,
    )
}

pub fn ask_with_options(
    meeting_id: &str,
    history: &[Message],
    runner_name: &str,
    options: &super::ask_options::Options,
    progress: impl Fn(AskProgress),
) -> Result<String, String> {
    let result = (|| {
        let valid = meeting_id.split_once('-').is_some_and(|(a, b)| {
            !a.is_empty() && !b.is_empty() && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
        });
        if !valid {
            return Err("Invalid meeting id".into());
        }
        check_question(history)?;
        let (meetings, _) = super::scan();
        let meeting = meetings
            .into_iter()
            .find(|m| m.id == meeting_id)
            .ok_or("Could not read the meeting summary or transcript")?;
        let folder = super::state_dir();
        std::fs::create_dir_all(&folder).map_err(|_| "Could not open app state folder")?;
        options.run(
            runner_name,
            options.prompt(prompt(&rules(), &meeting, history))?,
            meeting_id,
            &progress,
        )
    })();
    report(result, &progress)
}
/// Stops one running Ask. Clones share the flag: the window keeps one, the worker reads the other.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);
impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    /// Asks the worker to kill the CLI and give up. It lands within about 100 ms.
    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
/// The failure message of an Ask that its owner stopped. The window treats it as no failure.
pub const STOPPED: &str = "Stopped.";
thread_local! {
    static CURRENT: RefCell<Option<Cancel>> = const { RefCell::new(None) };
}
/// True on a worker whose own Ask was stopped by `Cancel::stop`.
pub(super) fn cancel_requested() -> bool {
    CURRENT.with(|c| c.borrow().as_ref().is_some_and(Cancel::stopped))
}
#[cfg(test)]
pub(super) fn with_cancel_for_test(cancel: Cancel, job: impl FnOnce()) {
    CURRENT.with(|current| *current.borrow_mut() = Some(cancel));
    job();
    CURRENT.with(|current| *current.borrow_mut() = None);
}
static STOP: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicUsize = AtomicUsize::new(0);
struct Running;
impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}
pub(super) fn stop_requested() -> bool {
    STOP.load(Ordering::SeqCst)
}
/// For app shutdown. Stops running Ask and Memory workers: the CLI and its children are killed, the
/// temporary prompt file is removed, and the call returns once the workers are done or `wait`
/// has passed. Returns false on timeout. A worker started afterwards stops at once.
pub fn cancel_all(wait: Duration) -> bool {
    STOP.store(true, Ordering::SeqCst);
    super::ask_session::close();
    let deadline = Instant::now() + wait;
    while RUNNING.load(Ordering::SeqCst) > 0 || !super::ask_session::closed() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}
/// Starts a meeting Ask on a worker and queues its final answer before the final progress event.
pub fn spawn(
    meeting_id: String,
    history: Vec<Message>,
    runner_name: String,
    cancel: Cancel,
    progress: impl Fn(AskProgress) + Send + 'static,
) -> std::sync::mpsc::Receiver<Result<String, String>> {
    spawn_with_options(
        meeting_id,
        history,
        runner_name,
        Default::default(),
        cancel,
        progress,
    )
}

pub fn spawn_with_options(
    meeting_id: String,
    history: Vec<Message>,
    runner_name: String,
    options: super::ask_options::Options,
    cancel: Cancel,
    progress: impl Fn(AskProgress) + Send + 'static,
) -> std::sync::mpsc::Receiver<Result<String, String>> {
    spawn_job(
        move |step| ask_with_options(&meeting_id, &history, &runner_name, &options, step),
        cancel,
        progress,
    )
}
pub fn live_prompt(rules: &str, history: &[Message]) -> String {
    format!(
        "{rules}\nAnswer only about this live meeting using its notes and transcript so far. Do not read other meetings.\n\nConversation so far:\n{}\n\nYou:",
        chat(history)
    )
}
pub fn spawn_live(
    key: String,
    history: Vec<Message>,
    runner: String,
    options: super::ask_options::Options,
    cancel: Cancel,
    progress: impl Fn(AskProgress) + Send + 'static,
) -> std::sync::mpsc::Receiver<Result<String, String>> {
    spawn_job(
        move |step| {
            check_question(&history)?;
            options.run(
                &runner,
                options.prompt(live_prompt(&rules(), &history))?,
                &key,
                &step,
            )
        },
        cancel,
        progress,
    )
}
/// Runs `job` on a worker that `cancel_all` waits for. Its answer is queued before the final
/// progress event, and the job's own final events are replaced by that one.
pub(super) fn spawn_job(
    job: impl FnOnce(&dyn Fn(AskProgress)) -> Result<String, String> + Send + 'static,
    cancel: Cancel,
    progress: impl Fn(AskProgress) + Send + 'static,
) -> std::sync::mpsc::Receiver<Result<String, String>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    spawn_cancellable(cancel, move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _update_activity = crate::updates::activity()
                .map_err(|_| "An update is being installed. Try again after restart.".to_owned())?;
            job(&|event| {
                if !matches!(event, AskProgress::Finished | AskProgress::Failed { .. }) {
                    progress(event);
                }
            })
        }))
        .unwrap_or_else(|_| Err("Ask stopped unexpectedly. Try again.".into()));
        let final_event = match &result {
            Ok(_) => AskProgress::Finished,
            Err(message) => AskProgress::Failed {
                message: message.clone(),
            },
        };
        let _ = sender.send(result);
        progress(final_event);
    });
    receiver
}

pub(super) fn spawn_cancellable(cancel: Cancel, job: impl FnOnce() + Send + 'static) {
    RUNNING.fetch_add(1, Ordering::SeqCst);
    let running = Running;
    std::thread::spawn(move || {
        let _running = running;
        CURRENT.with(|current| *current.borrow_mut() = Some(cancel));
        job();
    });
}
