//! Invented UI states. No helper, Hex call, or worker process runs in these scenes.
use super::{
    State,
    ai_notes::{AiNotes, HELPERS},
    hex,
    notes_status::Job,
};
use rune::TextEdit;
use sidenote::core::{
    agent_instructions,
    notes::{Helper, JobState, NotesSettings},
};

pub fn prepare(s: &mut State) {
    super::sync_scene::settle(s);
    hex::scene(s, false);
    s.ai_notes = AiNotes {
        scene: true,
        settings: Some(NotesSettings {
            helper: Some(Helper::Claude),
            model: Helper::Claude.default_model().into(),
            enabled_at_ms: Some(1),
            ..NotesSettings::default()
        }),
        found: HELPERS.map(|h| (h, true)).into(),
        ..AiNotes::default()
    };
    s.sidebar_open.set(true);
    s.drawer_open.set(false);
    s.ask_open = false;
    s.config.features.daily_check = false;
    s.config.features.memory = false;
}
pub fn settings(s: &mut State, mode: &str) {
    prepare(s);
    s.hex_open = true;
    s.hex.section = hex::Section::AiNotes;
    match mode {
        "claude-only" => s.ai_notes.found = HELPERS.map(|h| (h, h == Helper::Claude)).into(),
        "instructions-fallback" => {
            let mut store = agent_instructions::Store::default();
            store.using_defaults_after_error = true;
            s.ai_notes.error = super::ai_notes::instructions_warning(&store);
        }
        "saved-codex" => s.ai_notes.settings.as_mut().unwrap().helper = Some(Helper::Codex),
        "saved-opencode" => s.ai_notes.settings.as_mut().unwrap().helper = Some(Helper::OpenCode),
        "privacy" => s.ai_notes.privacy_open = true,
        "off" | "none" => {
            let settings = s.ai_notes.settings.as_mut().unwrap();
            settings.helper = None;
            if mode == "off" {
                settings.engine = "off".into();
            }
        }
        "last-error" => {
            s.ai_notes.jobs.insert(
                "1800000000000-1".into(),
                Job {
                    state: state("backend-error"),
                    helper: Some(Helper::Claude),
                    updated_at: chrono::Utc::now().timestamp_millis() - 120_000,
                    request: None,
                    transcript_revision: None,
                    routing_fallbacks: Vec::new(),
                    translation: None,
                },
            );
        }
        "scripts" | "switch" => {
            let settings = s.ai_notes.settings.as_mut().unwrap();
            settings.engine = "scripts".into();
            settings.script = Some(std::env::temp_dir().join("example-notes.ps1"));
            if mode == "switch" {
                s.ai_notes.switch_confirm = Some(Helper::Claude);
            }
        }
        "testing" => {
            s.ai_notes.pending = Some("Testing fixed sample text...");
            s.ai_notes.testing = Some(0);
        }
        "worked" => {
            s.ai_notes
                .test_results
                .insert(0, "Worked in 8 s. Only fixed sample text was sent.".into());
        }
        "test-bad-answer" => {
            s.ai_notes
                .test_results
                .insert(0, super::ai_notes::test_error("invalid JSON"));
        }
        "test-error" => {
            s.ai_notes
                .test_results
                .insert(0, super::ai_notes::test_error("sign-in failed"));
        }
        "load-error" => {
            s.ai_notes.settings = None;
            s.ai_notes.error = Some(super::ai_notes::LOAD_ERROR.into());
        }
        "loading" => {
            s.ai_notes.settings = None;
            s.ai_notes.pending = Some("Loading Meeting summary...");
        }
        "helpers" => {
            super::scene_scroll(s, "notes-helper-0");
        }
        "options" => {
            s.ai_notes.settings.as_mut().unwrap().auto = false;
            s.ai_notes.name = TextEdit::new("Alex");
            super::scene_scroll(s, "notes-options");
        }
        "edited" => {
            let e = &mut s.ai_notes.editors[0];
            e.text = TextEdit::new(
                "Keep exact decisions and numbers. Give each task a clear owner. Use short sentences.",
            );
            e.saved = e.text.text.clone();
            e.edited_at = Some("2026-11-10T04:00:00Z".into());
        }
        "limit" => {
            s.ai_notes.editors[0].text =
                TextEdit::new("a".repeat(agent_instructions::MAX_CHARS + 1));
        }
        "reset" => s.ai_notes.editors[0].reset_confirm = true,
        "fixed-notes" | "fixed-tasks" => {
            let index = usize::from(mode == "fixed-tasks");
            super::prompt_library::select(s, index);
            s.ai_notes.library.open = [true; 4];
        }
        "save-error" => {
            s.ai_notes.editor_target = Some(0);
            s.ai_notes.error = Some(super::ai_notes::SAVE_ERROR.into());
        }
        _ => {}
    }
    if mode == "scripts" {
        super::scene_scroll(s, "notes-scripts-card");
    }
    if mode == "none" {
        s.ai_notes.found = HELPERS.map(|h| (h, false)).into();
    }
    if matches!(
        mode,
        "instructions" | "edited" | "limit" | "reset" | "fixed-notes" | "save-error"
    ) {
        super::scene_scroll(s, "agent-instructions");
    }
    if matches!(mode, "tasks" | "fixed-tasks") {
        super::scene_scroll(s, "instructions-1");
    }
    if matches!(
        mode,
        "worked" | "test-error" | "test-bad-answer" | "testing" | "switch"
    ) {
        super::scene_scroll(s, "notes-helper-0");
    }
}
pub fn today(s: &mut State, found: bool) {
    prepare(s);
    s.go_meetings();
    s.ai_notes.settings.as_mut().unwrap().helper = None;
    if !found {
        s.ai_notes.found = HELPERS.map(|h| (h, false)).into();
        super::scene_agents(s, &[]);
    }
}

pub fn first_open(s: &mut State) {
    today(s, false);
    s.meetings.clear();
    s.ai_notes.jobs.clear();
    s.rebuild_tasks();
    s.rebuild_nav();
    s.refresh_list();
    assert!(s.meetings.is_empty() && s.tasks.all.is_empty() && s.groups.is_empty());
}
pub fn state(name: &str) -> JobState {
    match name {
        "no-helper" | "no-notes" => JobState::NoHelper,
        "needs-click" => JobState::NeedsClick,
        "queued" => JobState::Queued,
        "running" => JobState::Running,
        "helper-missing" => JobState::HelperMissing,
        "backend-error" => JobState::BackendError {
            next_try_ms: Some(1794305400000),
            exit: Some(1),
        },
        "backend-manual" => JobState::BackendError {
            next_try_ms: None,
            exit: None,
        },
        "helper-limit" => JobState::HelperLimit { exit: Some(1) },
        "timeout" => JobState::TimedOut,
        "bad-output" => JobState::BadOutput,
        "fallback" => JobState::Done {
            fallback: true,
            tasks_failed: false,
        },
        "too-long" => JobState::TooLong { minutes: 360 },
        "typed-too-long" => JobState::ContextTooLong {
            typed_notes: true,
            glossary: false,
        },
        "glossary-too-long" => JobState::ContextTooLong {
            typed_notes: false,
            glossary: true,
        },
        "empty" => JobState::Empty,
        "tasks-failed" => JobState::Done {
            fallback: false,
            tasks_failed: true,
        },
        _ => JobState::Done {
            fallback: false,
            tasks_failed: false,
        },
    }
}
pub fn meeting(s: &mut State, name: &str) {
    prepare(s);
    let Some(mut m) = s.meetings.first().cloned() else {
        panic!("Notes scenes need a synthetic meeting fixture")
    };
    m.title = "Planning the launch".into();
    m.device = None;
    m.summary_raw = if matches!(name, "fallback" | "tasks-failed" | "done") {
        serde_json::json!({"made_by":{"helper":"claude","model":"claude-sonnet-5-5"}}).to_string()
    } else {
        String::new()
    };
    if !m.has_summary() {
        m.summary = Default::default();
    }
    if name == "empty" {
        m.lines.clear();
        m.transcript_raw.clear();
    }
    if name == "too-long" {
        m.minutes = Some(360);
    }
    m.notes = "Share the revised plan on Friday.".into();
    s.selected = Some(m.id.clone());
    s.ai_notes.jobs.insert(
        m.id.clone(),
        Job {
            state: state(name),
            helper: Some(Helper::Claude),
            updated_at: chrono::Utc::now().timestamp_millis() - 120_000,
            request: None,
            transcript_revision: None,
            routing_fallbacks: Vec::new(),
            translation: None,
        },
    );
    if name == "no-notes" {
        s.ai_notes.jobs.clear();
    }
    if matches!(name, "no-notes" | "no-helper") {
        s.ai_notes.settings.as_mut().unwrap().helper = None;
    }
    s.meetings = vec![m];
    s.rebuild_tasks();
    s.refresh_list();
    if name == "tasks-failed" {
        super::scene_scroll(s, "sec-todo");
    }
}
pub fn live(s: &mut State, name: &str) {
    meeting(s, name);
    let id = s.selected.take().unwrap();
    s.live = Some(
        super::live::Live::scene(super::live::Phase::Making, Some("Planning the launch"))
            .scene_pending(&id),
    );
    s.live_open = true;
    s.live_notes = TextEdit::new("Share the revised plan on Friday.");
}
pub fn list(s: &mut State) {
    prepare(s);
    s.ai_notes.settings.as_mut().unwrap().helper = None;
    let template = s.meetings.first().expect("Synthetic meeting").clone();
    let names = [
        "no-helper",
        "needs-click",
        "queued",
        "running",
        "helper-missing",
        "backend-error",
        "backend-manual",
        "timeout",
        "bad-output",
        "fallback",
        "too-long",
        "empty",
        "tasks-failed",
        "done",
    ];
    s.meetings = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut m = template.clone();
            m.id = format!("1800000000000-{i}");
            m.title = format!("Example meeting {}", i + 1);
            m.device = None;
            s.ai_notes.jobs.insert(
                m.id.clone(),
                Job {
                    state: state(name),
                    helper: Some(Helper::Claude),
                    updated_at: chrono::Utc::now().timestamp_millis() - 120_000,
                    request: None,
                    transcript_revision: None,
                    routing_fallbacks: Vec::new(),
                    translation: None,
                },
            );
            m
        })
        .collect();
    s.go_meetings();
    s.refresh_list();
}
