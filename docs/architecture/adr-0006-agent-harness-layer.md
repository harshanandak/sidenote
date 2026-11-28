# ADR 0006: Agent harness layer and notes pipeline

Status: accepted, 2026-11-08.

## Context

- **AI runs on the user's own local agents.** Today `runners.rs` and `runner_process.rs` support
  Claude Code, Codex and OpenCode by hand-built flags per CLI (`ask.rs::read_only_runner`).
- **The notes pipeline is PowerShell and Python** in `~/.config/hex/scripts`:
  - `hex-meeting-watch.ps1` finds finished meetings.
  - `hex-meeting-english.ps1` runs audio preparation (uv, sherpa-onnx), Hex transcription, OpenCode translation, Claude notes, Python render and publish, and Claude task classification.
  - Retries are marker files; progress is only a log.
- **Open security findings:** S01 (translation has tools), S05 (inherited environment) and S11 (job attached after start).
- **Background work is invisible to the user** (UX-05, UX-06, UX-08).

## Decision

**One trait for every agent task.**

```rust
pub trait Harness: Send + Sync {
    fn id(&self) -> &str;                       // "claude-code", "codex", "gemini", "opencode", "acp:<cmd>"
    fn probe(&self) -> Result<Capabilities>;    // installed, version, models, enforceable ToolPolicy
    fn run(&self, task: AgentTask, on: &dyn Fn(AgentEvent), cancel: &Cancel) -> Result<AgentOutput>;
}
pub struct AgentTask {
    pub kind: TaskKind,        // Ask, AskAll, QuickTitle, Translate, Notes, ClassifyTasks, DailyCheck
    pub prompt: String,
    pub reads: Vec<ReadGrant>, // files or roots the agent may read
    pub tools: ToolPolicy,     // None | ReadOnly
    pub model: Option<String>,
    pub output: OutputSpec,    // Text | Json(schema)
    pub timeout: Duration,
}
// AgentOutput = text, optional parsed JSON, and Provenance
// { harness, harness_version, model_reported, model_requested, duration_ms, tools_used }
```

**Two implementations.**
- **`AcpHarness`** uses the `agent-client-protocol` crate (Apache-2.0) and works with any agent in the ACP registry: Gemini CLI and OpenCode natively, Claude Code and Codex through their adapters (verify current adapter names).
  - It starts a session in a private scratch folder and streams updates into `AgentEvent`.
  - It answers permission requests from the task's `ToolPolicy`: deny every write and command, allow reads only inside `reads`.
  - It serves file reads only from granted paths and does not offer the terminal capability.
- **`CliHarness`** is today's `runner_process` path, kept for agents without ACP and for adapters that cannot enforce the policy.

**Enforcement rule.** A harness runs a task only if `probe` says it can enforce that task's
`ToolPolicy`; otherwise the app refuses with a plain message, as it refuses OpenCode for Ask
today. Translate and QuickTitle use `ToolPolicy::None` (closes S01).

**Containment for both.**
- Start the child suspended, attach the kill-on-close job, then resume (S11).
- Clean the environment (S05).
- Run in a private scratch folder, with a deadline per task.

**Settings.** `runners.json` moves into an `agents` section of `config.json` v2. Migration backs up the old file first. Detection lists installed agents and marks the rest "Not installed".

**Notes pipeline as a resumable Rust job runner.**

- **Job** = one meeting and one pipeline version. Each job gets a folder `local/jobs/<job-id>/` holding `job.json` and `status.json`. Nothing in it syncs.
- **Stages, in order:**
  1. detect
  2. quick_title
  3. prepare_audio
  4. transcribe
  5. translate
  6. notes
  7. render
  8. publish (opt-in)
  9. classify_tasks
  10. index
- **Each stage declares its inputs and outputs.** A stage is skipped when its outputs exist with a matching input hash. Outputs are written atomically (`store_io::atomic_write`), so a crash resumes at the stage that was running.
- **One worker** runs in the listener process, behind the existing named mutex. It starts on the Hex meeting-finished event, on startup catch-up, every 15 minutes and on Retry.
- **Retries:** 3 attempts, 15 minutes apart, then the job waits in a "Needs you" state with a Retry button.
- **Prompts** move into the repo as neutral minijinja templates. Users can override them in the data folder. Owner and company details become optional profile inputs.

**Migration path: replace the scripts one stage at a time.**
1. The Rust runner calls the existing scripts as stages and writes status from day one.
2. Port quick_title, notes, classify_tasks and translate to `Harness`.
3. Port render and publish to Rust with minijinja.
4. Decide on prepare_audio: sherpa-onnx Rust bindings, or keep uv Python as an optional "local speaker turns" extra.

**Status file contract v1** (also feeds the Background UI and toasts):

```json
{ "schema": 1, "job": "notes", "id": "0193...", "meeting_id": "0193...",
  "state": "queued|running|waiting|needs_user|failed|done",
  "stage": "translate", "stage_index": 5, "stage_count": 10,
  "progress": {"done": 3, "total": 8},
  "started_at": "...", "updated_at": "...", "heartbeat_at": "...", "attempt": 1,
  "last_error": {"stage": "translate", "message": "Codex timed out after 10 minutes", "at": "..."},
  "last_success_at": "..." }
```

- **Long-running services** (listener, sync, audio cleanup, daily check) write `local/status/<service>.json` with the same fields minus the meeting.
- **Writers** replace the file atomically and update `heartbeat_at` every 10 seconds while running. Readers treat a heartbeat older than 60 seconds as "Stopped unexpectedly".
- **Messages** are plain and redacted: no transcript text, no secrets, and the user profile path shortened.

## Amendment, 2026-11-28

Sidenote will own one in-process ACP harness, starting with OpenCode and then
Gemini CLI for eligible accounts. The first OpenCode profile will use inline
meeting context with tools disabled. Granted-file reads require qualification
of the agent's native tools and configuration. ACP capabilities cover client
RPCs only. Existing Claude Code and Codex routes keep their CLI transport.
There are no outbound meeting-event hooks.

Each reviewed free cloud route requires device-local permission for its purpose
and model. Unresolved routes and routes whose terms prohibit sensitive meeting
content are blocked. The current OpenCode Ask consent gate does not establish
read confinement for the legacy `plan` agent.

## Options rejected

- AG-UI: events from agent to UI, not control of a local CLI.
- A job queue library (apalis) or a SQLite queue: about 10 jobs a day, and status files are needed for the UI anyway.
- Keeping PowerShell and Python as the pipeline: Windows-only, unpinned dependencies, no status.
- Calling cloud LLM APIs from the app: breaks "AI runs on the user's own agents".
- ACP only: some agents or versions lack ACP or cannot enforce the policy.

## Consequences

- Ask, notes, quick title, translation and the daily check share one path for timeouts, cancel, provenance and policy.
- New agents arrive through the ACP registry with no new code.
- The pipeline becomes portable and visible, and closes S01, S05 and S11.
- Two harness implementations to maintain until ACP covers every supported agent.

## Revisit if

- ACP adds a standard tool-permission profile that all adapters enforce: drop per-adapter checks.
- Jobs exceed about 100 a day or need several workers: adopt a durable queue.

## First step

Roadmap lane F6: `src/core/status.rs` with the v1 schema, atomic writes and stale detection;
the listener writes `local/status/listener.json`.
