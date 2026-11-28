//! Ask preferences and context. Summary runners never pass through this module.
use super::{Runner, agent_consent, file_reads, live, runners};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Thinking {
    Low,
    #[default]
    Medium,
    High,
}
impl Thinking {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
    fn argument(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}
pub fn meeting_thinking_default() -> Thinking {
    Thinking::Low
}
pub fn meeting_agent_default() -> String {
    "Codex".into()
}
pub fn supports_thinking(runner: &Runner) -> bool {
    matches!(runners::kind(runner), Ok("claude" | "codex"))
}
pub fn agent_name(runner: &Runner) -> &'static str {
    match runners::kind(runner) {
        Ok("claude") => "Claude Code",
        Ok("codex") => "Codex",
        _ => "OpenCode",
    }
}

#[derive(Clone, Debug)]
pub struct LiveSource {
    pub hint: Option<String>,
    pub start_ms: Option<i64>,
}
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub connection: Option<super::provider::Settings>,
    pub thinking: Thinking,
    pub model: Option<String>,
    pub meeting: bool,
    pub live: Option<LiveSource>,
    pub live_notes: Option<String>,
    pub meeting_agent: bool,
    pub fallback: Option<Fallback>,
}
#[derive(Clone, Debug)]
pub struct Fallback {
    pub runner: String,
    pub model: Option<String>,
    pub thinking: Thinking,
}
#[derive(Debug)]
enum RunFailure {
    Runner(String),
    Consent(agent_consent::Refusal),
}
impl<T: Into<String>> From<T> for RunFailure {
    fn from(error: T) -> Self {
        Self::Runner(error.into())
    }
}

fn send(
    root: &Path,
    runner: &Runner,
    prompt: String,
    run: impl FnOnce(String, Option<File>) -> Result<String, String>,
) -> Result<String, RunFailure> {
    let guard = if runners::kind(runner)? == "opencode" {
        Some(agent_consent::authorize(root, runner.model.as_deref()).map_err(RunFailure::Consent)?)
    } else {
        None
    };
    run(prompt, guard).map_err(RunFailure::Runner)
}

impl Options {
    pub(super) fn run(
        &self,
        name: &str,
        prompt: String,
        chat: &str,
        progress: &impl Fn(super::AskProgress),
    ) -> Result<String, String> {
        if let Some(settings) = &self.connection {
            return super::provider::chat(settings.clone(), prompt, progress);
        }
        let execute = |name: &str, options: &Self| {
            let runner = if options.meeting_agent {
                super::list_runners()?
                    .into_iter()
                    .find(|r| r.name == name)
                    .ok_or("The selected runner is not configured")?
            } else {
                super::ask::pick_runner(name)?
            };
            let runner = options.runner(super::ask::read_only_runner(runner)?)?;
            send(
                &super::state_dir(),
                &runner,
                prompt.clone(),
                |prompt, guard| {
                    super::ask_session::run_authorized(&runner, prompt, chat, progress, guard)
                },
            )
        };
        self.try_fallback(name, progress, execute)
    }
    fn try_fallback(
        &self,
        name: &str,
        progress: &impl Fn(super::AskProgress),
        mut execute: impl FnMut(&str, &Self) -> Result<String, RunFailure>,
    ) -> Result<String, String> {
        let mut result = execute(name, self);
        if matches!(result, Err(RunFailure::Runner(_)))
            && !super::ask::cancel_requested()
            && !super::ask::stop_requested()
            && let Some(fallback) = &self.fallback
        {
            progress(super::AskProgress::Fallback {
                runner: fallback.runner.clone(),
            });
            progress(super::AskProgress::Text {
                text: String::new(),
            });
            result = execute(
                &fallback.runner,
                &Self {
                    model: fallback.model.clone(),
                    thinking: fallback.thinking,
                    meeting_agent: true,
                    ..Default::default()
                },
            );
        }
        result.map_err(|failure| match failure {
            RunFailure::Runner(message) => message,
            RunFailure::Consent(refusal) => refusal.message().into(),
        })
    }
    pub(super) fn runner(&self, mut runner: Runner) -> Result<Runner, String> {
        if let Some(model) = &self.model {
            runner.model = Some(model.clone());
            if !runner.args.iter().any(|a| a == "--model") {
                runner
                    .args
                    .splice(1..1, ["--model".into(), "{model}".into()]);
            }
        }
        match runners::kind(&runner)? {
            "claude" => {
                let mut args = runner.args.into_iter();
                runner.args = Vec::new();
                while let Some(arg) = args.next() {
                    if matches!(arg.as_str(), "--output-format" | "--effort") {
                        args.next();
                    } else if arg != "--include-partial-messages" && arg != "--verbose" {
                        runner.args.push(arg);
                    }
                }
                runner.args.extend([
                    "--effort".into(),
                    self.thinking.argument().into(),
                    "--output-format".into(),
                    "stream-json".into(),
                    "--include-partial-messages".into(),
                    "--verbose".into(),
                ]);
            }
            "codex" => {
                runner.args.splice(
                    1..1,
                    [
                        "-c".into(),
                        format!("model_reasoning_effort={}", self.thinking.argument()),
                    ],
                );
            }
            _ => {}
        }
        Ok(runner)
    }
    pub(super) fn prompt(&self, prompt: String) -> Result<String, String> {
        self.prompt_in(prompt, &super::meetings_dir())
    }
    fn prompt_in(&self, mut prompt: String, root: &Path) -> Result<String, String> {
        let boundary = prompt.find("\n\n").unwrap_or(prompt.len());
        let mut instructions = String::new();
        if self.meeting {
            instructions.push_str("\nDuring this meeting or its follow-up, answer in at most 5 lines. Lead with the answer. Use the live transcript so far when supplied.");
        }
        if self.live.is_some() {
            instructions.push_str("\nIf the live transcript is empty, no speech is available yet. Do not infer the current meeting from older meetings.");
        }
        prompt.insert_str(boundary, &instructions);
        if let Some(notes) = &self.live_notes {
            prompt.push_str("\n<live_notes>\n");
            prompt.push_str(notes);
            prompt.push_str("\n</live_notes>\n");
        }
        if let Some(source) = &self.live {
            let state = super::state_dir();
            let suppressed = super::meeting_delete::suppressed_ids(&state)
                .map_err(|_| "Could not read deleted meeting IDs. Try again.")?;
            let folder = live::find_folder(root, source.hint.as_deref(), source.start_ms);
            let folder = folder.or_else(|| {
                if source.hint.is_some() {
                    return None;
                }
                let start = source.start_ms?;
                let (_, recent) =
                    recent_meeting_for(root, chrono::Local::now().timestamp_millis(), Some(start))?;
                live::find_folder(root, recent.hint.as_deref(), recent.start_ms)
            });
            prompt.push_str("\n<live_transcript>\n");
            if let Some(folder) = folder.filter(|folder| {
                let id = folder.file_name().unwrap_or_default().to_string_lossy();
                !suppressed.contains(id.as_ref())
                    && !super::recording_guard::hidden(&state, folder, &id)
            }) {
                match live::read_rolling(&folder, None)
                    .map_err(|_| "Could not read the live transcript. Try again.")?
                {
                    live::Read::Lines(lines, _) => {
                        for line in lines {
                            let secs = line.seconds;
                            prompt.push_str(&format!(
                                "[{:02}:{:02}:{:02}] {}: {}\n",
                                secs / 3600,
                                secs / 60 % 60,
                                secs % 60,
                                if line.you { "You" } else { "Them" },
                                line.text
                            ));
                        }
                    }
                    _ => {
                        let path = folder.join("english-transcript.md");
                        match file_reads::read(&path, live::ROLLING_MAX_BYTES) {
                            Ok(bytes) => prompt.push_str(&String::from_utf8_lossy(&bytes)),
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(_) => {
                                return Err(
                                    "Could not read the meeting transcript. Try again.".into()
                                );
                            }
                        }
                    }
                }
            }
            prompt.push_str("</live_transcript>\n");
        }
        Ok(prompt)
    }
}

pub fn in_meeting(recording: bool, ended_ms: Option<i64>, now_ms: i64) -> bool {
    recording || ended_ms.is_some_and(|end| (0..=600_000).contains(&now_ms.saturating_sub(end)))
}
/// Includes meetings whose summary has not been written yet. Ignores active and future records.
pub fn recent_meeting(root: &Path, now_ms: i64) -> Option<(i64, LiveSource)> {
    recent_meeting_for(root, now_ms, None)
}
fn recent_meeting_for(
    root: &Path,
    now_ms: i64,
    start_ms: Option<i64>,
) -> Option<(i64, LiveSource)> {
    let state = super::state_dir();
    let suppressed = super::meeting_delete::suppressed_ids(&state).ok()?;
    let entries = file_reads::entries(root).ok()?;
    let mut meetings = entries.iter().filter_map(|entry| {
        let name = entry.file_name();
        if suppressed.contains(name.to_string_lossy().as_ref()) {
            return None;
        }
        let folder = entry.path();
        if !file_reads::directory(&folder) {
            return None;
        }
        let bytes = file_reads::read(&folder.join("manifest.json"), 1024 * 1024).ok()?;
        let m: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        if super::recording_guard::hidden(
            &super::state_dir(),
            &folder,
            &entry.file_name().to_string_lossy(),
        ) {
            return None;
        }
        if start_ms.is_some_and(|start| {
            m["created_at_ms"]
                .as_i64()
                .is_none_or(|made| made.abs_diff(start) > 90_000)
        }) {
            return None;
        }
        if matches!(m["status"].as_str(), Some("recording" | "starting")) {
            return None;
        }
        let end = m["ended_at_ms"].as_i64().or_else(|| {
            let duration = m["duration_ms"]
                .as_f64()
                .filter(|d| d.is_finite() && *d >= 0.0)?;
            Some(m["created_at_ms"].as_i64()?.saturating_add(duration as i64))
        })?;
        in_meeting(false, Some(end), now_ms).then(|| {
            (
                end,
                LiveSource {
                    hint: Some(entry.file_name().to_string_lossy().into_owned()),
                    start_ms: m["created_at_ms"].as_i64(),
                },
            )
        })
    });
    if start_ms.is_some() {
        let only = meetings.next()?;
        meetings.next().is_none().then_some(only)
    } else {
        meetings.max_by_key(|(end, _)| *end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opencode_ask_checks_consent_before_each_recipient_gets_prompt_bytes() {
        use agent_consent::Refusal;
        let root = tempfile::tempdir().unwrap();
        let mut runner = Runner {
            name: "Synthetic OpenCode".into(),
            program: "opencode".into(),
            model: Some("opencode/big-pickle".into()),
            args: Vec::new(),
            stdin: true,
            tools: true,
        };
        let options = Options {
            fallback: Some(Fallback {
                runner: "fallback".into(),
                model: None,
                thinking: Thinking::Low,
            }),
            ..Default::default()
        };
        let terms = agent_consent::terms(runner.model.as_deref()).unwrap();
        let marker = "synthetic meeting marker";
        let mut received = Vec::new();
        for (permission, expected_bytes) in
            [(None, 0), (Some(true), marker.len()), (Some(false), 0)]
        {
            if let Some(allow) = permission {
                agent_consent::set_permission(root.path(), terms, allow).unwrap();
            }
            received.clear();
            let result = options.try_fallback(&runner.name, &|_| {}, |name, _| {
                assert_eq!(name, runner.name, "privacy refusal must skip fallback");
                send(root.path(), &runner, marker.into(), |prompt, guard| {
                    assert!(guard.is_some());
                    received.extend_from_slice(prompt.as_bytes());
                    Ok("answer".into())
                })
            });
            assert_eq!(received.len(), expected_bytes);
            assert_eq!(result.is_ok(), permission == Some(true));
            if permission == Some(true) {
                assert_eq!(received, marker.as_bytes());
                assert!(matches!(
                    agent_consent::authorize(root.path(), Some("opencode/beam")),
                    Err(Refusal::Permission(_))
                ));
            }
        }
        for (model, refusal) in [
            (Some("other/example-free"), Refusal::UnreviewedFree),
            (None, Refusal::Unresolved),
            (Some("other/unreviewed"), Refusal::Unresolved),
        ] {
            runner.model = model.map(str::to_owned);
            assert!(
                matches!(send(root.path(), &runner, marker.into(), |_, _| panic!("unexpected send")),
                Err(RunFailure::Consent(actual)) if actual == refusal)
            );
        }
        runner.model = Some("opencode/nemotron-3.5-lightning".into());
        let Err(Refusal::Restricted(restricted)) = agent_consent::terms(runner.model.as_deref())
        else {
            panic!("expected restricted terms")
        };
        assert!(agent_consent::set_permission(root.path(), restricted, true).is_err());
        let result = options.try_fallback("Codex", &|_| {}, |name, _| {
            if name == "Codex" {
                return Err("Synthetic CLI failure".into());
            }
            assert_eq!(name, "fallback");
            send(root.path(), &runner, marker.into(), |_, _| {
                panic!("unexpected send")
            })
        });
        assert_eq!(
            result.unwrap_err(),
            Refusal::Restricted(restricted).message()
        );
        std::fs::write(root.path().join("local/agent-consent.json"), b"{ broken").unwrap();
        assert!(matches!(
            agent_consent::authorize(root.path(), Some(terms.model)),
            Err(Refusal::Permission(_))
        ));
    }

    #[test]
    fn endpoint_failures_never_enter_the_cli_fallback() {
        let options = Options {
            connection: Some(super::super::provider::Settings {
                enabled: true,
                ..super::super::provider::Settings::openrouter()
            }),
            fallback: Some(Fallback {
                runner: "must never execute".into(),
                model: None,
                thinking: Thinking::Low,
            }),
            ..Default::default()
        };
        let result = options.run(
            "unused CLI",
            "Synthetic content".into(),
            "synthetic",
            &|_| panic!("Unauthorized endpoint must emit no execution or fallback events"),
        );
        assert!(result.unwrap_err().contains("Opt Ask in"));
    }
    #[test]
    fn current_live_prompt_includes_notes_and_transcript_without_other_meetings() {
        let root =
            std::env::temp_dir().join(format!("meeting-agent-live-context-{}", std::process::id()));
        let current = root.join("1800000000000-1");
        let other = root.join("1800000000001-1");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            current.join("english-transcript.md"),
            "[00:04] Maya will prepare the Friday prototype.",
        )
        .unwrap();
        std::fs::write(
            other.join("english-summary.md"),
            "Unrelated synthetic budget decision",
        )
        .unwrap();
        let options = Options {
            meeting: true,
            live: Some(LiveSource {
                hint: Some("1800000000000-1".into()),
                start_ms: None,
            }),
            live_notes: Some("Typed notes: use paper cards".into()),
            ..Default::default()
        };
        let history = [super::super::Message {
            role: "user".into(),
            text: "What did we decide?".into(),
            ..Default::default()
        }];
        let prompt = options
            .prompt_in(
                super::super::ask::live_prompt(
                    &super::super::agent_instructions::default_prompt(
                        super::super::agent_instructions::Agent::Ask,
                    ),
                    &history,
                ),
                &root,
            )
            .unwrap();
        assert!(prompt.contains("Typed notes: use paper cards"));
        assert!(prompt.contains("Maya will prepare"));
        assert!(prompt.contains("Do not read other meetings"));
        assert!(!prompt.contains("Unrelated synthetic budget decision"));
        let key = super::super::live_ask_key("../synthetic meeting");
        assert!(super::super::persistence::is_live_ask_key(&key));
        assert!(!super::super::persistence::is_live_ask_key("live-../../"));
    }
    #[test]
    fn waiting_for_codex_warmup_does_not_select_the_fallback() {
        let options = Options {
            fallback: Some(Fallback {
                runner: "slower default".into(),
                model: None,
                thinking: Thinking::Medium,
            }),
            ..Default::default()
        };
        let events = std::cell::RefCell::new(Vec::new());
        let mut calls = Vec::new();
        let result = options.try_fallback(
            "Codex",
            &|event| events.borrow_mut().push(event),
            |name, _| {
                calls.push(name.to_owned());
                events
                    .borrow_mut()
                    .push(super::super::AskProgress::StartingCodex);
                Ok("Codex answered after warming".into())
            },
        );
        assert_eq!(result.unwrap(), "Codex answered after warming");
        assert_eq!(calls, ["Codex"]);
        assert_eq!(*events.borrow(), [super::super::AskProgress::StartingCodex]);
    }
    #[test]
    fn fallback_retries_once_with_normal_preferences_and_clears_partial_text() {
        use std::cell::RefCell;
        let options = Options {
            thinking: Thinking::Low,
            meeting_agent: true,
            fallback: Some(Fallback {
                runner: "normal".into(),
                model: Some("normal-model".into()),
                thinking: Thinking::Medium,
            }),
            ..Default::default()
        };
        for primary_error in [
            "not installed",
            "not signed in",
            "failed after partial text",
        ] {
            let calls = RefCell::new(Vec::new());
            let events = RefCell::new(Vec::new());
            let result = options.try_fallback(
                "Codex",
                &|event| events.borrow_mut().push(event),
                |name, options| {
                    calls.borrow_mut().push(name.to_owned());
                    if name == "Codex" {
                        return Err(primary_error.into());
                    }
                    assert_eq!(options.model.as_deref(), Some("normal-model"));
                    assert_eq!(options.thinking, Thinking::Medium);
                    assert!(options.fallback.is_none());
                    Ok("saved question answered".into())
                },
            );
            assert_eq!(result.unwrap(), "saved question answered");
            assert_eq!(*calls.borrow(), ["Codex", "normal"]);
            assert!(
                matches!(&events.borrow()[0], super::super::AskProgress::Fallback { runner } if runner == "normal")
            );
            assert!(
                matches!(&events.borrow()[1], super::super::AskProgress::Text { text } if text.is_empty())
            );
        }
        let cancel = super::super::ask::Cancel::new();
        cancel.stop();
        super::super::ask::with_cancel_for_test(cancel, || {
            let result =
                options.try_fallback("Codex", &|_| panic!("Cancelled Ask retried"), |_, _| {
                    Err(super::super::ask::STOPPED.into())
                });
            assert_eq!(result.unwrap_err(), super::super::ask::STOPPED);
        });
    }
    #[test]
    fn no_id_follow_up_uses_the_matching_finished_transcript_and_rejects_ambiguous_folders() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("ask-no-id-{}", std::process::id()));
        let start = chrono::Local::now().timestamp_millis() - 120_000;
        let make = |suffix| {
            let folder = root.join(format!("{start}-{suffix}"));
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join("manifest.json"),
                serde_json::json!({
                    "status":"complete", "created_at_ms":start, "ended_at_ms":start+60_000
                })
                .to_string(),
            )
            .unwrap();
            std::fs::write(
                folder.join("english-transcript.md"),
                "[00:00:44] You: Check the draft",
            )
            .unwrap();
        };
        make(1);
        let mut options = Options {
            meeting: true,
            live: Some(LiveSource {
                hint: None,
                start_ms: Some(start),
            }),
            ..Default::default()
        };
        assert!(
            options
                .prompt_in("Question".into(), &root)
                .unwrap()
                .contains("Check the draft")
        );
        make(2);
        assert!(
            !options
                .prompt_in("Question".into(), &root)
                .unwrap()
                .contains("Check the draft")
        );
        options.live.as_mut().unwrap().hint = Some("missing-1".into());
        assert!(
            !options
                .prompt_in("Question".into(), &root)
                .unwrap()
                .contains("Check the draft")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn restart_finds_unsummarised_recent_meeting_and_prompt_reads_latest_transcript() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("ask-context-{}", std::process::id()));
        let folder = root.join("1000-1");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("manifest.json"),
            r#"{"status":"transcribing","created_at_ms":1000,"duration_ms":2000}"#,
        )
        .unwrap();
        std::fs::write(
            folder.join("transcript.rolling.ndjson"),
            "{\"source\":\"microphone\",\"start_ms\":42000,\"text\":\"Ship on Friday\"}\n",
        )
        .unwrap();
        let (end, source) = recent_meeting(&root, 4000).unwrap();
        assert_eq!(end, 3000);
        let options = Options {
            meeting: true,
            live: Some(source),
            ..Default::default()
        };
        let prompt = options.prompt_in("Question".into(), &root).unwrap();
        assert!(prompt.contains("at most 5 lines"));
        assert!(prompt.contains("[00:00:42] You: Ship on Friday"));
        std::fs::write(
            folder.join("transcript.rolling.ndjson"),
            "{\"source\":\"system\",\"start_ms\":43000,\"text\":\"Move it to Monday\"}\n",
        )
        .unwrap();
        assert!(
            options
                .prompt_in("Question".into(), &root)
                .unwrap()
                .contains("Move it to Monday")
        );
        std::fs::remove_file(folder.join("transcript.rolling.ndjson")).unwrap();
        std::fs::write(
            folder.join("english-transcript.md"),
            "[00:00:44] You: Final transcript",
        )
        .unwrap();
        assert!(
            options
                .prompt_in("Question".into(), &root)
                .unwrap()
                .contains("Final transcript")
        );
        assert!(recent_meeting(&root, 603001).is_none());
        assert!(
            !Options::default()
                .prompt_in("Question".into(), &root)
                .unwrap()
                .contains("5 lines")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn meeting_window_includes_ten_minutes_but_not_future_end_times() {
        assert!(in_meeting(true, None, 0));
        assert!(in_meeting(false, Some(1000), 1000));
        assert!(in_meeting(false, Some(1000), 601000));
        assert!(!in_meeting(false, Some(1000), 601001));
        assert!(!in_meeting(false, Some(1001), 1000));
        assert!(!in_meeting(false, None, 1000));
    }
    #[test]
    fn every_ask_level_maps_to_real_flags_without_changing_stored_runners() {
        let runners: Vec<Runner> =
            serde_json::from_str(include_str!("../../default-runners.json")).unwrap();
        for thinking in Thinking::ALL {
            for original in &runners {
                let runner = super::super::ask::read_only_runner(original.clone()).unwrap();
                let before = runner.args.clone();
                let options = Options {
                    thinking,
                    model: Some("chosen-model".into()),
                    ..Default::default()
                };
                let prepared = options.runner(runner).unwrap();
                runners::validate(&prepared).unwrap();
                assert_eq!(prepared.model.as_deref(), Some("chosen-model"));
                match runners::kind(original).unwrap() {
                    "claude" => {
                        assert!(
                            prepared
                                .args
                                .windows(2)
                                .any(|p| p == ["--effort", thinking.argument()])
                        );
                        assert!(
                            prepared
                                .args
                                .iter()
                                .any(|a| a == "--include-partial-messages")
                        );
                    }
                    "codex" => assert!(
                        prepared
                            .args
                            .contains(&format!("model_reasoning_effort={}", thinking.argument()))
                    ),
                    _ => assert_eq!(prepared.args, before),
                }
                assert!(
                    !original
                        .args
                        .iter()
                        .any(|a| a == "--effort" || a.starts_with("model_reasoning_effort="))
                );
            }
        }
    }
}
