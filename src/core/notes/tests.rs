use super::{
    helper::{Failure, Run},
    *,
};
#[cfg(windows)]
mod corruption;
mod english_first;
mod idle;
mod passes;
#[cfg(windows)]
mod review;
#[cfg(windows)]
mod routing;
mod translated;
#[cfg(any(windows, target_os = "linux"))]
mod translation;
use crate::core::{
    agent_instructions::{self, Agent},
    store_io,
};
use std::{
    collections::VecDeque,
    fs,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "linux")]
#[test]
fn hardlinked_manifest_is_not_a_notes_input() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let outside = root.0.join("outside-manifest.json");
    fs::write(&outside, br#"{"status":"complete","duration_ms":30000}"#).unwrap();
    fs::remove_file(folder.join("manifest.json")).unwrap();
    fs::hard_link(outside, folder.join("manifest.json")).unwrap();
    assert!(queue::manifest(&folder).is_err());
}
#[cfg(target_os = "linux")]
#[test]
fn ancestor_swap_after_preflight_cannot_redirect_a_notes_read() {
    use std::os::unix::fs::symlink;
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let path = folder.join("manifest.json");
    crate::core::file_reads::checked_metadata(&path).unwrap();
    let parent = folder.parent().unwrap();
    fs::rename(parent, root.0.join("saved-meetings")).unwrap();
    symlink(root.0.join("saved-meetings"), parent).unwrap();
    assert!(
        crate::core::file_reads::nofollow(&path).is_err(),
        "ancestor swap bypassed preflight"
    );
    assert!(queue::manifest(&folder).is_err());
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sidenote-engine-test-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Fake {
    available: bool,
    glossary: Vec<String>,
    replies: VecDeque<Result<String, Failure>>,
    calls: Vec<(String, Duration)>,
    selections: Vec<(Helper, String)>,
}
impl Fake {
    fn new(replies: Vec<Result<String, Failure>>) -> Self {
        Self {
            available: true,
            glossary: Vec::new(),
            replies: replies.into(),
            calls: Vec::new(),
            selections: Vec::new(),
        }
    }
}
impl Run for Fake {
    fn glossary(&self) -> Vec<String> {
        self.glossary.clone()
    }
    fn installed(&self, _: Helper) -> bool {
        self.available
    }
    fn run(
        &mut self,
        helper: Helper,
        model: &str,
        prompt: String,
        timeout: Duration,
    ) -> Result<String, Failure> {
        self.selections.push((helper, model.into()));
        self.calls.push((prompt, timeout));
        self.replies.pop_front().expect("unexpected helper call")
    }
}
const ID: &str = "1793946600000-123";
const PRIVATE: &str = "The confidential fixture sentence must never appear in logs.";
#[cfg(any(windows, target_os = "linux"))]
fn routing_setup() -> (Temp, PathBuf, NotesSettings) {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    let mut config = crate::core::Config::default();
    config.notes = Some(settings.clone());
    config.pipeline.profile = "experimental".into();
    config.features.indic_engine = true;
    config.features.translation = true;
    config.labs.after_stop_language_routing = true;
    config.engine_settings.stages.routing_deadline_seconds = 360;
    let manifest = folder.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    m["ended_at_ms"] = chrono::Utc::now().timestamp_millis().into();
    fs::write(manifest, m.to_string()).unwrap();
    fs::write(
        root.0.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    (root, folder, settings)
}
fn settings_fixture() -> NotesSettings {
    NotesSettings {
        helper: Some(Helper::Claude),
        enabled_at_ms: Some(1),
        ..NotesSettings::default()
    }
}
fn save_fixture_settings(root: &Path, edited: &NotesSettings) -> std::io::Result<()> {
    let config: crate::core::Config = store_io::read(&root.join("config.json"))?;
    crate::core::Config::save_notes(root, &config.notes.unwrap_or_default(), edited)
}
fn meeting(root: &Path, text: &str) -> PathBuf {
    let folder = root.join("meetings").join(ID);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("manifest.json"),serde_json::json!({"status":"complete","title":"Delivery planning","created_at_ms":1793946600000_i64,"duration_ms":300000}).to_string()).unwrap();
    fs::write(
        folder.join("transcript.ndjson"),
        serde_json::json!({"start_ms":1000,"end_ms":2000,"source":"microphone","text":if text.is_empty() { String::new() } else { format!("{text} {}", vec!["synthetic";25].join(" ")) }})
            .to_string(),
    )
    .unwrap();
    fs::write(
        folder.join("quick-title.json"),
        r#"{"title":"Delivery planning"}"#,
    )
    .unwrap();
    if text.is_empty() {
        crate::core::recording_guard::keep_in(root, ID).unwrap();
    }
    folder
}
fn summary() -> String {
    serde_json::json!({"title":"Plan the delivery","context":[],"topics":[],"todo":[{"ref":"t1","title":"Send the report","description":"Share final figures.","owner":"You","at":["00:00:01"]}],"to_settle":[]}).to_string()
}
fn sorted() -> String {
    r#"{"meeting_project":null,"tasks":[{"i":0,"project":"Delivery","new":true}]}"#.into()
}

fn scripts_settings() -> NotesSettings {
    NotesSettings {
        engine: "scripts".into(),
        script: Some(PathBuf::from("C:/synthetic.ps1")),
        ..settings_fixture()
    }
}

#[test]
fn scripts_finish_creates_only_a_display_draft_then_final_replaces_it() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let mut fake = Fake::new(vec![Ok(summary())]);
    assert_eq!(
        draft::generate(
            &root.0,
            &root.0.join("meetings"),
            ID,
            &scripts_settings(),
            &mut fake
        )
        .unwrap(),
        draft::Outcome::Written
    );
    assert_eq!(fake.calls.len(), 1);
    let raw = draft::read(&root.0, &folder, ID).unwrap();
    let preview: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(preview["title"], "Plan the delivery");
    assert_eq!(preview["todo"], serde_json::json!([]));
    for name in [
        "english-summary.json",
        "english-transcript.md",
        "tasks.json",
    ] {
        assert!(!folder.join(name).exists(), "{name}");
    }
    for name in ["projects.json", "memory", "local/notes-jobs"] {
        assert!(!root.0.join(name).exists(), "{name}");
    }
    let pending = crate::core::Meeting::load_pending(&folder).unwrap();
    assert!(!pending.has_summary());
    assert!(pending.summary.todo.is_empty());
    assert!(
        crate::core::build_tasks(
            &[pending],
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
        )
        .is_empty()
    );
    assert!(
        crate::core::settle::Store::new(root.0.clone(), root.0.join("meetings"))
            .history()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        draft::generate(
            &root.0,
            &root.0.join("meetings"),
            ID,
            &scripts_settings(),
            &mut fake
        )
        .unwrap(),
        draft::Outcome::Skipped
    );
    fs::write(folder.join("english-summary.json"), summary()).unwrap();
    {
        let _scope = crate::core::file_reads::Scope::begin();
        assert!(draft::read(&root.0, &folder, ID).unwrap().is_empty());
        assert!(
            root.0
                .join(format!("local/draft-summaries/{ID}.json"))
                .exists()
        );
    }
    assert!(draft::read(&root.0, &folder, ID).unwrap().is_empty());
    assert!(
        !root
            .0
            .join(format!("local/draft-summaries/{ID}.json"))
            .exists()
    );
    assert_eq!(
        crate::core::Meeting::load(&folder).unwrap().summary.title,
        "Plan the delivery"
    );
}

#[test]
fn drafts_skip_app_off_short_and_unavailable_helpers() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let mut fake = Fake::new(vec![]);
    for engine in ["app", "off"] {
        let settings = NotesSettings {
            engine: engine.into(),
            ..scripts_settings()
        };
        assert_eq!(
            draft::generate(&root.0, &root.0.join("meetings"), ID, &settings, &mut fake).unwrap(),
            draft::Outcome::Skipped
        );
    }
    fake.available = false;
    assert_eq!(
        draft::generate(
            &root.0,
            &root.0.join("meetings"),
            ID,
            &scripts_settings(),
            &mut fake
        )
        .unwrap(),
        draft::Outcome::NoHelper
    );
    let settings = NotesSettings {
        helper: None,
        ..scripts_settings()
    };
    assert_eq!(
        draft::generate(&root.0, &root.0.join("meetings"), ID, &settings, &mut fake).unwrap(),
        draft::Outcome::NoHelper
    );
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(folder.join("manifest.json")).unwrap()).unwrap();
    manifest["duration_ms"] = 1000.into();
    fs::write(folder.join("manifest.json"), manifest.to_string()).unwrap();
    fake.available = true;
    assert_eq!(
        draft::generate(
            &root.0,
            &root.0.join("meetings"),
            ID,
            &scripts_settings(),
            &mut fake
        )
        .unwrap(),
        draft::Outcome::Skipped
    );
    assert!(fake.calls.is_empty());
    assert!(draft::read(&root.0, &folder, ID).unwrap().is_empty());
}

#[test]
fn final_arriving_during_draft_generation_wins() {
    struct FinalDuringCall {
        folder: PathBuf,
    }
    impl Run for FinalDuringCall {
        fn installed(&self, _: Helper) -> bool {
            true
        }
        fn run(&mut self, _: Helper, _: &str, _: String, _: Duration) -> Result<String, Failure> {
            fs::write(self.folder.join("english-summary.json"), summary()).unwrap();
            Ok(summary())
        }
    }
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let mut fake = FinalDuringCall {
        folder: folder.clone(),
    };
    assert_eq!(
        draft::generate(
            &root.0,
            &root.0.join("meetings"),
            ID,
            &scripts_settings(),
            &mut fake
        )
        .unwrap(),
        draft::Outcome::Skipped
    );
    assert!(draft::read(&root.0, &folder, ID).unwrap().is_empty());
}

#[test]
fn draft_recording_guard_also_rejects_too_few_words() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    fs::write(
        folder.join("transcript.ndjson"),
        r#"{"start_ms":1000,"text":"Two words"}"#,
    )
    .unwrap();
    let mut fake = Fake::new(vec![]);
    assert_eq!(
        draft::generate(
            &root.0,
            &root.0.join("meetings"),
            ID,
            &scripts_settings(),
            &mut fake
        )
        .unwrap(),
        draft::Outcome::Skipped
    );
    assert!(fake.calls.is_empty());
}
fn enqueue(root: &Path, settings: &NotesSettings, mode: Option<Request>) {
    queue::enqueue_in(root, &root.join("meetings"), ID, mode, settings).unwrap();
}
fn drain(root: &Path, settings: &NotesSettings, fake: &mut Fake) {
    queue::drain(root, &root.join("meetings"), fake, || Ok(settings.clone())).unwrap();
}

#[test]
fn busy_settings_and_authorization_preserve_job_and_prepared_translation() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    enqueue(&root.0, &settings, None);
    let path = root.0.join("local/notes-jobs").join(format!("{ID}.json"));
    let before = fs::read(&path).unwrap();
    let mut fake = Fake::new(vec![]);
    queue::drain(&root.0, &root.0.join("meetings"), &mut fake, || {
        Err(std::io::ErrorKind::WouldBlock.into())
    })
    .unwrap();
    assert!(fake.calls.is_empty());
    assert_eq!(fs::read(&path).unwrap(), before);
    let mut fake = Fake::new(vec![Err(Failure::Busy)]);
    drain(&root.0, &settings, &mut fake);
    let mut expected: queue::Job = serde_json::from_slice(&before).unwrap();
    expected.translation = Some(super::translation::Disposition {
        reason: "language_passes_not_available".into(),
        source: transcript::Source::Compatibility,
        segment_keys: vec![],
        snapshot: None,
    });
    assert_eq!(
        fs::read(&path).unwrap(),
        serde_json::to_vec_pretty(&expected).unwrap()
    );
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &settings, &mut fake);
    assert!(matches!(
        queue::read(&root.0, ID).unwrap().unwrap().state,
        JobState::Done {
            tasks_failed: false,
            ..
        }
    ));
}

#[test]
fn busy_task_authorization_retries_sorting_without_failing_the_job() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    enqueue(&root.0, &settings, None);
    let mut fake = Fake::new(vec![Ok(summary()), Err(Failure::Busy)]);
    drain(&root.0, &settings, &mut fake);
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert!(matches!(job.state, JobState::Queued));
    assert_eq!(job.attempts, 0);
    assert_eq!(job.request, None);
    assert!(job.sorting_pending);
    let mut fake = Fake::new(vec![Ok(sorted())]);
    drain(&root.0, &settings, &mut fake);
    assert!(matches!(
        state(&root.0),
        JobState::Done {
            tasks_failed: false,
            ..
        }
    ));
}

#[test]
fn unreadable_tasks_settings_after_publication_keep_notes_done() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    let mut config = crate::core::Config::default();
    config.notes = Some(settings.clone());
    fs::write(
        root.0.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    enqueue(&root.0, &settings, None);
    let mut first = Fake::new(vec![Ok(summary()), Err(Failure::Busy)]);
    drain(&root.0, &settings, &mut first);
    assert!(queue::read(&root.0, ID).unwrap().unwrap().sorting_pending);
    let published = fs::read(folder.join("english-summary.json")).unwrap();

    fs::write(root.0.join("config.json"), b"invalid synthetic config").unwrap();
    let mut retry = Fake::new(vec![]);
    drain(&root.0, &settings, &mut retry);
    assert_eq!(
        state(&root.0),
        JobState::Done {
            fallback: false,
            tasks_failed: true
        }
    );
    assert!(retry.calls.is_empty());
    assert_eq!(
        queue::read(&root.0, ID).unwrap().unwrap().message,
        "task settings could not be read"
    );
    assert_eq!(
        fs::read(folder.join("english-summary.json")).unwrap(),
        published
    );
}

#[test]
fn busy_sorting_retry_still_requires_auto_consent() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let mut settings = settings_fixture();
    enqueue(&root.0, &settings, None);
    let mut fake = Fake::new(vec![Ok(summary()), Err(Failure::Busy)]);
    drain(&root.0, &settings, &mut fake);
    settings.auto = false;
    let mut fake = Fake::new(vec![]);
    drain(&root.0, &settings, &mut fake);
    assert!(fake.calls.is_empty());
    assert!(matches!(state(&root.0), JobState::NeedsClick));
}
fn state(root: &Path) -> JobState {
    queue::read(root, ID).unwrap().unwrap().state
}

#[test]
fn unoffered_saved_helpers_never_run_and_have_a_fixed_reason() {
    assert!(Helper::Claude.offered());
    assert!(Helper::Codex.offered());
    {
        let helper = Helper::OpenCode;
        assert!(!helper.offered());
        let s: NotesSettings = serde_json::from_value(serde_json::json!({
            "helper": helper, "enabled_at_ms": 1
        }))
        .unwrap();
        assert_eq!(s.effective_helper(), None);
        assert_eq!(s.helper_reason(), Some(HELPER_NOT_OFFERED));
        let root = Temp::new();
        meeting(&root.0, PRIVATE);
        enqueue(&root.0, &s, Some(Request::MakeNotes));
        let mut fake = Fake::new(vec![]);
        drain(&root.0, &s, &mut fake);
        assert_eq!(state(&root.0), JobState::NoHelper);
        assert!(fake.calls.is_empty());
        assert_eq!(
            helper::Cli { root: None }.run(helper, "", String::new(), Duration::ZERO),
            Err(Failure::Consent)
        );
    }
}

#[test]
fn no_helper_or_no_consent_never_writes_meeting_files_or_calls_helper() {
    for s in [
        NotesSettings::default(),
        NotesSettings {
            enabled_at_ms: None,
            ..settings_fixture()
        },
    ] {
        let root = Temp::new();
        let folder = meeting(&root.0, PRIVATE);
        enqueue(&root.0, &s, None);
        let mut fake = Fake::new(vec![]);
        drain(&root.0, &s, &mut fake);
        assert_eq!(state(&root.0), JobState::NoHelper);
        assert!(fake.calls.is_empty());
        assert!(!folder.join("english-transcript.md").exists());
        assert!(!folder.join("english-summary.json").exists());
    }
}

#[test]
fn existing_script_transcript_is_preserved_and_used() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let original = "[00:00:01.000] You (English): Script transcript.\n";
    fs::write(folder.join("english-transcript.md"), original).unwrap();
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert_eq!(
        fs::read_to_string(folder.join("english-transcript.md")).unwrap(),
        original
    );
    assert!(fake.calls[0].0.contains("Script transcript."));
    assert!(!fake.calls[0].0.contains(PRIVATE));
}
#[test]
fn old_meeting_and_auto_off_need_click() {
    for s in [
        NotesSettings {
            enabled_at_ms: Some(i64::MAX),
            ..settings_fixture()
        },
        NotesSettings {
            auto: false,
            ..settings_fixture()
        },
    ] {
        let root = Temp::new();
        meeting(&root.0, PRIVATE);
        enqueue(&root.0, &s, None);
        let mut fake = Fake::new(vec![]);
        drain(&root.0, &s, &mut fake);
        assert_eq!(state(&root.0), JobState::NeedsClick);
        assert!(fake.calls.is_empty());
        assert!(
            !root
                .0
                .join("meetings")
                .join(ID)
                .join("english-transcript.md")
                .exists()
        );
    }
}
#[test]
fn explicit_click_allows_an_old_meeting_and_success_has_v2_and_mine() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let s = NotesSettings {
        enabled_at_ms: Some(i64::MAX),
        ..settings_fixture()
    };
    enqueue(&root.0, &s, Some(Request::MakeNotes));
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert_eq!(
        state(&root.0),
        JobState::Done {
            fallback: false,
            tasks_failed: false
        }
    );
    let summary: serde_json::Value =
        serde_json::from_slice(&fs::read(folder.join("english-summary.json")).unwrap()).unwrap();
    assert_eq!(summary["schema"], 2);
    assert_eq!(summary["todo"][0]["task"], summary["todo"][0]["title"]);
    assert_eq!(summary["todo"][0]["id"].as_str().unwrap().len(), 12);
    assert_eq!(summary["made_by"]["instructions"], "default");
    let tasks: crate::core::TasksFile =
        serde_json::from_slice(&fs::read(folder.join("tasks.json")).unwrap()).unwrap();
    assert_eq!(tasks.tasks[0].mine, Some(true));
    let history = crate::core::settle::Store::new(root.0.clone(), root.0.join("meetings"))
        .history()
        .unwrap();
    assert_eq!(history.len(), 2);
    assert!(
        history
            .iter()
            .any(|row| row.entry.kind == crate::core::settle::Kind::TaskProperties)
    );
    assert!(
        history
            .iter()
            .any(|row| row.entry.kind == crate::core::settle::Kind::ProjectAssignment)
    );
    assert!(history.iter().all(|row| row.entry.source_meeting_id == ID));
    assert_eq!(fake.calls.len(), 2);
    assert_eq!(fake.calls[0].1, helper::NOTES_TIMEOUT);
    assert_eq!(fake.calls[1].1, helper::TASKS_TIMEOUT);
}
#[test]
fn disabled_tasks_step_skips_sorting() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    let mut config = crate::core::Config::default();
    config.notes = Some(s.clone());
    config.engine_settings.steps.tasks = false;
    fs::write(
        root.0.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert_eq!(
        fake.calls
            .iter()
            .filter(|(_, timeout)| *timeout == helper::TASKS_TIMEOUT)
            .count(),
        0
    );
    assert_eq!(
        state(&root.0),
        JobState::Done {
            fallback: false,
            tasks_failed: false
        }
    );
    assert!(folder.join("english-summary.json").exists());
    assert!(!folder.join("tasks.json").exists());
}

#[test]
fn enabled_tasks_step_runs_sorting() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    let mut config = crate::core::Config::default();
    config.notes = Some(s.clone());
    config.engine_settings.steps.tasks = true;
    fs::write(
        root.0.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert_eq!(
        fake.calls
            .iter()
            .filter(|(_, timeout)| *timeout == helper::TASKS_TIMEOUT)
            .count(),
        1
    );
    assert_eq!(
        state(&root.0),
        JobState::Done {
            fallback: false,
            tasks_failed: false
        }
    );
    assert!(folder.join("tasks.json").exists());
}
#[test]
fn default_instructions_stop_after_two_invalid_answers() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![Ok("bad".into()), Ok("bad again".into())]);
    drain(&root.0, &s, &mut fake);
    assert_eq!(state(&root.0), JobState::BadOutput);
    assert_eq!(fake.calls.len(), 2);
    assert!(
        fake.calls[1]
            .0
            .contains("Your last answer could not be read:")
    );
    assert!(!folder.join("english-summary.json").exists());
}
#[test]
fn custom_ladder_falls_back_on_third_and_keeps_notes_on_tasks_failure() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    agent_instructions::save(&root.0, Agent::Notes, "CUSTOM NOTES").unwrap();
    agent_instructions::save(&root.0, Agent::Tasks, "CUSTOM TASKS").unwrap();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![
        Ok("bad".into()),
        Ok("bad".into()),
        Ok(summary()),
        Ok("bad".into()),
        Ok("bad".into()),
        Ok("bad".into()),
    ]);
    drain(&root.0, &s, &mut fake);
    assert_eq!(fake.calls.len(), 6);
    assert!(fake.calls[0].0.contains("CUSTOM NOTES"));
    assert!(fake.calls[1].0.contains("CUSTOM NOTES"));
    assert!(!fake.calls[2].0.contains("CUSTOM NOTES"));
    assert!(fake.calls[3].0.contains("CUSTOM TASKS"));
    assert!(!fake.calls[5].0.contains("CUSTOM TASKS"));
    assert_eq!(
        state(&root.0),
        JobState::Done {
            fallback: true,
            tasks_failed: true
        }
    );
    assert!(folder.join("english-summary.json").is_file());
    assert!(!folder.join("tasks.json").exists());
    assert_eq!(
        queue::read(&root.0, ID).unwrap().unwrap().instructions,
        "default-fallback"
    );
}
#[test]
fn too_long_does_not_call_helper_and_keeps_full_transcript() {
    let root = Temp::new();
    let text = "x".repeat(prompt::TRANSCRIPT_CAP + 1);
    let folder = meeting(&root.0, &text);
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![]);
    drain(&root.0, &s, &mut fake);
    assert!(matches!(state(&root.0), JobState::TooLong { .. }));
    assert!(fake.calls.is_empty());
    assert!(
        fs::read_to_string(folder.join("english-transcript.md"))
            .unwrap()
            .len()
            > prompt::TRANSCRIPT_CAP
    );
}
#[test]
fn first_part_is_explicit_and_caps_only_the_sent_material() {
    let root = Temp::new();
    let folder = meeting(&root.0, &"x".repeat(prompt::TRANSCRIPT_CAP + 1));
    let s = settings_fixture();
    enqueue(&root.0, &s, Some(Request::FirstPart));
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert!(matches!(state(&root.0), JobState::Done { .. }));
    assert!(
        fs::read_to_string(folder.join("english-transcript.md"))
            .unwrap()
            .len()
            > prompt::TRANSCRIPT_CAP
    );
}
#[test]
fn summary_is_never_overwritten_or_requeued_automatically() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    fs::write(folder.join("english-summary.json"), b"keep existing").unwrap();
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert!(matches!(job.state, JobState::Done { .. }));
    assert_eq!(job.attempts, 0);
    assert!(job.input_revision.is_some());
    drain(&root.0, &s, &mut Fake::new(vec![]));
    assert_eq!(
        fs::read(folder.join("english-summary.json")).unwrap(),
        b"keep existing"
    );
}
#[test]
fn one_worker_at_a_time() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let lock = store_io::lock(&root.0.join("local/notes-jobs/worker.lock")).unwrap();
    let mut fake = Fake::new(vec![]);
    drain(&root.0, &s, &mut fake);
    assert!(fake.calls.is_empty());
    assert_eq!(state(&root.0), JobState::Queued);
    drop(lock);
}

#[test]
fn existing_job_enqueue_does_not_wait_for_the_running_job_lock() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let _lock = store_io::lock(&root.0.join(format!("local/notes-jobs/{ID}.lock"))).unwrap();
    enqueue(&root.0, &s, None);
    enqueue(&root.0, &s, Some(Request::TryAgain));
    assert_eq!(state(&root.0), JobState::Queued);
}
#[test]
fn backend_retry_delays_are_fifteen_then_sixty_minutes_then_manual() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    for (attempt, delay) in [(1, Some(15 * 60_000)), (2, Some(60 * 60_000)), (3, None)] {
        let mut fake = Fake::new(vec![Err(Failure::Backend { exit: Some(1) })]);
        let before = chrono::Utc::now().timestamp_millis();
        drain(&root.0, &s, &mut fake);
        let mut job = queue::read(&root.0, ID).unwrap().unwrap();
        assert_eq!(job.attempts, attempt);
        let JobState::BackendError { next_try_ms, exit } = job.state else {
            panic!("expected backend error")
        };
        assert_eq!(exit, Some(1));
        match delay {
            Some(delay) => {
                assert!(
                    (before + delay..=chrono::Utc::now().timestamp_millis() + delay)
                        .contains(&next_try_ms.unwrap())
                );
                job.state = JobState::BackendError {
                    next_try_ms: Some(0),
                    exit,
                };
                job.next_try_at = Some(0);
                store_io::write(&root.0.join(format!("local/notes-jobs/{ID}.json")), &job).unwrap();
            }
            None => assert!(next_try_ms.is_none()),
        }
    }
}

#[test]
fn future_retry_releases_worker_and_runs_when_due_on_next_drain() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![Err(Failure::Backend { exit: None })]);
    drain(&root.0, &s, &mut fake);
    let path = root.0.join("local/notes-jobs/worker.lock");
    drop(store_io::lock(&path).expect("idle worker released its lock"));
    let mut job = queue::read(&root.0, ID).unwrap().unwrap();
    let retry = job.next_try_at.unwrap();
    assert!(retry > chrono::Utc::now().timestamp_millis());
    drain(&root.0, &s, &mut fake);
    assert_eq!(fake.calls.len(), 1);
    assert_eq!(
        queue::read(&root.0, ID).unwrap().unwrap().next_try_at,
        Some(retry)
    );
    job.state = JobState::BackendError {
        next_try_ms: Some(0),
        exit: None,
    };
    job.next_try_at = Some(0);
    store_io::write(&root.0.join(format!("local/notes-jobs/{ID}.json")), &job).unwrap();
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert!(matches!(state(&root.0), JobState::Done { .. }));
}
#[test]
fn timeout_and_missing_helper_do_not_retry_automatically() {
    for available in [true, false] {
        let root = Temp::new();
        meeting(&root.0, PRIVATE);
        let s = settings_fixture();
        enqueue(&root.0, &s, None);
        let mut fake = Fake::new(if available {
            vec![Err(Failure::TimedOut)]
        } else {
            vec![]
        });
        fake.available = available;
        drain(&root.0, &s, &mut fake);
        assert_eq!(
            state(&root.0),
            if available {
                JobState::TimedOut
            } else {
                JobState::HelperMissing
            }
        );
        let calls = fake.calls.len();
        drain(&root.0, &s, &mut fake);
        assert_eq!(fake.calls.len(), calls);
    }
}
#[test]
fn empty_transcript_is_terminal() {
    let root = Temp::new();
    meeting(&root.0, "");
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![]);
    drain(&root.0, &s, &mut fake);
    assert_eq!(state(&root.0), JobState::Empty);
    assert!(fake.calls.is_empty());
}
#[test]
fn privacy_failure_logs_never_include_meeting_or_helper_output() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![Ok(PRIVATE.into()), Ok(PRIVATE.into())]);
    drain(&root.0, &s, &mut fake);
    for path in [
        root.0.join("sidenote.log"),
        root.0.join(format!("local/notes-jobs/{ID}.json")),
    ] {
        assert!(!fs::read_to_string(path).unwrap().contains(PRIVATE));
    }
}
#[test]
fn migration_keeps_existing_scripts_and_new_users_start_without_consent() {
    let root = Temp::new();
    let script = root.0.join("watch.ps1");
    let fresh = migrated_settings(&script);
    assert_eq!(fresh.engine(), Engine::App);
    assert!(fresh.helper.is_none());
    assert!(fresh.enabled_at_ms.is_none());
    fs::write(&script, b"unused test fixture").unwrap();
    let migrated = migrated_settings(&script);
    assert_eq!(migrated.engine(), Engine::Scripts(script));
}
#[test]
fn scripts_and_off_never_enqueue_and_ids_cannot_escape_the_root() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    for engine in ["scripts", "off"] {
        let s = NotesSettings {
            engine: engine.into(),
            ..settings_fixture()
        };
        enqueue(&root.0, &s, None);
        assert!(queue::read(&root.0, ID).unwrap().is_none());
    }
    for id in ["../escape", "..\\escape", "C:\\temp", "", "abc"] {
        assert!(
            queue::enqueue_in(
                &root.0,
                &root.0.join("meetings"),
                id,
                None,
                &settings_fixture()
            )
            .is_err()
        );
    }
}
#[test]
fn settings_save_sets_consent_time_and_preserves_other_config_fields() {
    let root = Temp::new();
    fs::write(
        root.0.join("config.json"),
        r#"{"version":1,"features":{},"untouched":"value"}"#,
    )
    .unwrap();
    let mut s = settings_fixture();
    let before = chrono::Utc::now().timestamp_millis();
    save_fixture_settings(&root.0, &s).unwrap();
    let saved = crate::core::Config::load_in(&root.0)
        .unwrap()
        .notes
        .unwrap();
    assert!(saved.enabled_at_ms.unwrap() >= before);
    save_fixture_settings(&root.0, &s).unwrap();
    assert_eq!(
        crate::core::Config::load_in(&root.0)
            .unwrap()
            .notes
            .unwrap()
            .enabled_at_ms,
        saved.enabled_at_ms
    );
    let raw: serde_json::Value =
        serde_json::from_slice(&fs::read(root.0.join("config.json")).unwrap()).unwrap();
    assert_eq!(raw["untouched"], "value");
    s.engine = "off".into();
    save_fixture_settings(&root.0, &s).unwrap();
    assert!(
        crate::core::Config::load_in(&root.0)
            .unwrap()
            .notes
            .unwrap()
            .enabled_at_ms
            .is_none()
    );
}
#[test]
fn sort_again_keeps_notes_bytes_and_updates_only_tasks() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    let original = summary();
    fs::write(folder.join("english-summary.json"), &original).unwrap();
    enqueue(&root.0, &s, Some(Request::SortAgain));
    let mut fake = Fake::new(vec![Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert_eq!(
        fs::read_to_string(folder.join("english-summary.json")).unwrap(),
        original
    );
    assert!(folder.join("tasks.json").exists());
    assert_eq!(fake.calls.len(), 1);
}
#[test]
fn quick_title_failure_never_blocks_notes() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    fs::remove_file(folder.join("quick-title.json")).unwrap();
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![Err(Failure::TimedOut), Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert_eq!(fake.calls[0].1, helper::TITLE_TIMEOUT);
    assert_eq!(
        state(&root.0),
        JobState::Done {
            fallback: false,
            tasks_failed: false
        }
    );
}

#[test]
fn disabling_or_switching_helper_revokes_the_next_call() {
    let root = Temp::new();
    let mut s = settings_fixture();
    save_fixture_settings(&root.0, &s).unwrap();
    assert!(helper::authorized(&root.0, Helper::Claude).unwrap());
    assert!(!helper::authorized(&root.0, Helper::Codex).unwrap());
    s.engine = "off".into();
    save_fixture_settings(&root.0, &s).unwrap();
    assert!(!helper::authorized(&root.0, Helper::Claude).unwrap());
}

#[test]
fn config_lock_clash_is_retryable_and_never_revokes_consent() {
    let root = Temp::new();
    save_fixture_settings(&root.0, &settings_fixture()).unwrap();
    let lock = store_io::lock(&root.0.join("local/config.lock")).unwrap();
    assert!(helper::authorized(&root.0, Helper::Claude).unwrap());
    store_io::write(&root.0.join("config.json"), &crate::core::Config::default()).unwrap();
    assert_eq!(
        helper::authorized(&root.0, Helper::Claude).unwrap_err(),
        Failure::Busy
    );
    drop(lock);
    save_fixture_settings(&root.0, &settings_fixture()).unwrap();
    assert!(helper::authorized(&root.0, Helper::Claude).unwrap());
}

#[test]
fn corrupt_job_is_quarantined_without_stopping_healthy_work() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let path = root.0.join("local/notes-jobs/1-1.json");
    fs::write(&path, b"private broken JSON").unwrap();
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert!(matches!(state(&root.0), JobState::Done { .. }));
    assert!(!path.exists());
    let quarantined: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "quarantined")
        })
        .collect();
    assert_eq!(quarantined.len(), 1);
    assert_eq!(
        fs::read(quarantined[0].path()).unwrap(),
        b"private broken JSON"
    );
    assert!(
        !fs::read_to_string(root.0.join("sidenote.log"))
            .unwrap()
            .contains("private broken JSON")
    );
}

#[test]
fn invalid_instructions_do_not_stop_notes_or_settings_loading() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    save_fixture_settings(&root.0, &s).unwrap();
    fs::write(root.0.join("agent-instructions.json"), b"invalid JSON").unwrap();
    assert!(crate::core::Config::load_in(&root.0).is_ok());
    assert!(
        agent_instructions::load(&root.0)
            .unwrap()
            .using_defaults_after_error
    );
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert!(matches!(state(&root.0), JobState::Done { .. }));
    assert!(
        fake.calls[0]
            .0
            .contains(agent_instructions::default_base(Agent::Notes))
    );
}

#[test]
fn new_meeting_rechecks_previously_missing_helper() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![]);
    fake.available = false;
    drain(&root.0, &s, &mut fake);
    assert_eq!(state(&root.0), JobState::HelperMissing);
    let new_id = "1793946900000-2";
    let new = root.0.join("meetings").join(new_id);
    fs::create_dir_all(&new).unwrap();
    for name in ["manifest.json", "transcript.ndjson", "quick-title.json"] {
        fs::copy(folder.join(name), new.join(name)).unwrap();
    }
    queue::enqueue_in(&root.0, &root.0.join("meetings"), new_id, None, &s).unwrap();
    let mut fake = Fake::new(vec![
        Ok(summary()),
        Ok(sorted()),
        Ok(summary()),
        Ok(sorted()),
    ]);
    drain(&root.0, &s, &mut fake);
    assert!(matches!(state(&root.0), JobState::Done { .. }));
    assert!(matches!(
        queue::read(&root.0, new_id).unwrap().unwrap().state,
        JobState::Done { .. }
    ));
    assert_eq!(fake.calls.len(), 4);
}

#[test]
fn typed_notes_and_glossary_have_separate_character_caps_for_all_requests() {
    for request in [None, Some(Request::FirstPart), Some(Request::RedoTasks)] {
        for glossary in [false, true] {
            let root = Temp::new();
            let folder = meeting(&root.0, PRIVATE);
            if request == Some(Request::RedoTasks) {
                fs::write(folder.join("english-summary.json"), summary()).unwrap();
            }
            let s = settings_fixture();
            let mut fake = Fake::new(vec![]);
            if glossary {
                fake.glossary = vec!["é".repeat(prompt::GLOSSARY_CAP + 1)];
            } else {
                fs::write(folder.join("notes.md"), "é".repeat(prompt::TYPED_CAP + 1)).unwrap();
            }
            enqueue(&root.0, &s, request);
            drain(&root.0, &s, &mut fake);
            assert_eq!(
                state(&root.0),
                JobState::ContextTooLong {
                    typed_notes: !glossary,
                    glossary
                }
            );
            assert!(fake.calls.is_empty());
            fs::write(folder.join("notes.md"), "Short notes").unwrap();
            fake.glossary.clear();
            fake.replies = vec![Ok(summary()), Ok(sorted())].into();
            enqueue(&root.0, &s, Some(request.unwrap_or(Request::TryAgain)));
            drain(&root.0, &s, &mut fake);
            assert!(matches!(state(&root.0), JobState::Done { .. }));
        }
    }
    assert!(prompt::bounded_inputs(
        &"é".repeat(prompt::TYPED_CAP),
        &["é".repeat(prompt::GLOSSARY_CAP)]
    ));
}

#[test]
fn redo_sends_the_bounded_typed_notes_snapshot_even_if_the_file_changes() {
    struct EditingHelper {
        fake: Fake,
        notes: PathBuf,
    }
    impl Run for EditingHelper {
        fn installed(&self, _: Helper) -> bool {
            fs::write(&self.notes, "UNBOUNDED EDIT".repeat(prompt::TYPED_CAP)).unwrap();
            true
        }
        fn run(
            &mut self,
            helper: Helper,
            model: &str,
            prompt: String,
            timeout: Duration,
        ) -> Result<String, Failure> {
            self.fake.run(helper, model, prompt, timeout)
        }
    }
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    fs::write(folder.join("english-summary.json"), summary()).unwrap();
    fs::write(folder.join("notes.md"), "Bounded snapshot").unwrap();
    let s = settings_fixture();
    enqueue(&root.0, &s, Some(Request::RedoTasks));
    let mut helper = EditingHelper {
        fake: Fake::new(vec![Ok(summary()), Ok(sorted())]),
        notes: folder.join("notes.md"),
    };
    queue::drain(&root.0, &root.0.join("meetings"), &mut helper, || {
        Ok(s.clone())
    })
    .unwrap();
    assert!(matches!(state(&root.0), JobState::Done { .. }));
    assert!(helper.fake.calls[0].0.contains("Bounded snapshot"));
    assert!(!helper.fake.calls[0].0.contains("UNBOUNDED EDIT"));
    assert!(
        fs::read_to_string(folder.join("notes.md"))
            .unwrap()
            .chars()
            .count()
            > prompt::TYPED_CAP
    );
}
#[test]
fn oldest_meeting_runs_first_and_running_jobs_recover_after_a_crash() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    let older_id = "1793946000000-123";
    let older = root.0.join("meetings").join(older_id);
    fs::create_dir_all(&older).unwrap();
    for name in ["manifest.json", "transcript.ndjson"] {
        fs::copy(folder.join(name), older.join(name)).unwrap();
    }
    fs::write(
        older.join("quick-title.json"),
        r#"{"title":"Older meeting"}"#,
    )
    .unwrap();
    enqueue(&root.0, &s, None);
    queue::enqueue_in(&root.0, &root.0.join("meetings"), older_id, None, &s).unwrap();
    let mut job = queue::read(&root.0, ID).unwrap().unwrap();
    job.state = JobState::Running;
    store_io::write(&root.0.join(format!("local/notes-jobs/{ID}.json")), &job).unwrap();
    let mut fake = Fake::new(vec![
        Ok(summary()),
        Ok(sorted()),
        Ok(summary()),
        Ok(sorted()),
    ]);
    drain(&root.0, &s, &mut fake);
    assert!(fake.calls[0].0.contains("Title: Older meeting"));
    assert!(fake.calls[2].0.contains("Title: Delivery planning"));
    assert!(matches!(state(&root.0), JobState::Done { .. }));
    let calls = fake.calls.len();
    assert_eq!(
        queue::read(&root.0, ID).unwrap().unwrap().stage,
        crate::core::meeting_job::Stage::Tasks
    );
    let summary = fs::read(folder.join("english-summary.json")).unwrap();
    let mut interrupted = queue::read(&root.0, ID).unwrap().unwrap();
    interrupted.state = JobState::Running;
    store_io::write(
        &root.0.join(format!("local/notes-jobs/{ID}.json")),
        &interrupted,
    )
    .unwrap();
    enqueue(&root.0, &s, None);
    drain(&root.0, &s, &mut fake);
    assert_eq!(fake.calls.len(), calls);
    assert_eq!(
        fs::read(folder.join("english-summary.json")).unwrap(),
        summary
    );
}
#[test]
fn external_summary_published_during_a_call_is_preserved() {
    struct Race {
        folder: PathBuf,
        calls: usize,
    }
    impl Run for Race {
        fn installed(&self, _: Helper) -> bool {
            true
        }
        fn run(&mut self, _: Helper, _: &str, _: String, _: Duration) -> Result<String, Failure> {
            self.calls += 1;
            fs::write(self.folder.join("english-summary.json"), b"external notes").unwrap();
            Ok(summary())
        }
    }
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    enqueue(&root.0, &s, None);
    let mut race = Race {
        folder: folder.clone(),
        calls: 0,
    };
    queue::drain(&root.0, &root.0.join("meetings"), &mut race, || {
        Ok(s.clone())
    })
    .unwrap();
    assert_eq!(
        fs::read(folder.join("english-summary.json")).unwrap(),
        b"external notes"
    );
    assert_eq!(race.calls, 1);
    assert!(matches!(state(&root.0), JobState::Done { .. }));
}

#[test]
fn redo_tasks_only_replaces_todos_and_retains_unknown_note_fields() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let original = serde_json::json!({"title":"Keep this title","context":["Keep this context"],"topics":[],"todo":[{"task":"Send the report","owner":"You"}],"to_settle":[],"future":{"keep":true}});
    fs::write(folder.join("english-summary.json"), original.to_string()).unwrap();
    let store = crate::core::TaskStore::new(root.0.clone());
    store
        .update_override(ID, 0, "Send the report", |o| {
            o.status = Some(crate::core::TaskStatus::Review);
            o.owner = Some("Alex".into());
        })
        .unwrap();
    let s = settings_fixture();
    enqueue(&root.0, &s, Some(Request::RedoTasks));
    let mut fake=Fake::new(vec![Ok(serde_json::json!({"todo":[{"title":"Send the report","description":"Share the figures","subtasks":[{"title":"Read the figures"}]}]}).to_string()),Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    let updated: serde_json::Value =
        serde_json::from_slice(&fs::read(folder.join("english-summary.json")).unwrap()).unwrap();
    let mut expected = original;
    expected["todo"] = updated["todo"].clone();
    assert_eq!(updated, expected);
    assert_eq!(fake.calls.len(), 2);
    assert!(
        fake.calls
            .iter()
            .all(|(_, limit)| *limit == helper::TASKS_TIMEOUT)
    );
    let id = updated["todo"][0]["id"].as_str().unwrap();
    let overrides = store.load_overrides().unwrap();
    let edit = overrides.match_task(ID, id, "Send the report", 0).unwrap();
    assert_eq!(edit.status, Some(crate::core::TaskStatus::Review));
    assert_eq!(edit.owner.as_deref(), Some("Alex"));
    assert!(matches!(
        state(&root.0),
        JobState::Done {
            tasks_failed: false,
            ..
        }
    ));
}
#[test]
fn failed_redo_keeps_original_summary_and_overrides() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let original = summary();
    fs::write(folder.join("english-summary.json"), &original).unwrap();
    let s = settings_fixture();
    enqueue(&root.0, &s, Some(Request::RedoTasks));
    let mut fake = Fake::new(vec![Ok("bad".into()), Ok("bad again".into())]);
    drain(&root.0, &s, &mut fake);
    assert_eq!(
        fs::read_to_string(folder.join("english-summary.json")).unwrap(),
        original
    );
    assert_eq!(state(&root.0), JobState::BadOutput);
}

#[test]
fn redo_uses_existing_english_transcript_without_rewriting_it() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    fs::remove_file(folder.join("transcript.ndjson")).unwrap();
    let transcript = "[00:00:01.000] You (English): Send the paper proof.\n";
    fs::write(folder.join("english-transcript.md"), transcript).unwrap();
    fs::write(folder.join("english-summary.json"), summary()).unwrap();
    fs::write(folder.join("notes.md"), "Use the folded sample.").unwrap();
    let s = settings_fixture();
    enqueue(&root.0, &s, Some(Request::RedoTasks));
    let mut fake = Fake::new(vec![
        Ok(serde_json::json!({"todo":[{"title":"Send the paper proof"}]}).to_string()),
        Ok(sorted()),
    ]);
    drain(&root.0, &s, &mut fake);
    assert!(matches!(
        state(&root.0),
        JobState::Done {
            tasks_failed: false,
            ..
        }
    ));
    assert_eq!(
        fs::read_to_string(folder.join("english-transcript.md")).unwrap(),
        transcript
    );
    assert!(fake.calls[0].0.contains("Use the folded sample."));
}

#[test]
fn switching_from_unavailable_helper_requires_a_click_for_queued_older_meeting() {
    {
        let helper = Helper::OpenCode;
        let root = Temp::new();
        let folder = meeting(&root.0, PRIVATE);
        let t1 = 1;
        let t2 = 2;
        let mut s = NotesSettings {
            helper: Some(helper),
            enabled_at_ms: Some(t1),
            ..NotesSettings::default()
        };
        store_io::write(
            &root.0.join("config.json"),
            &serde_json::json!({"version":1,"features":{},"notes":s}),
        )
        .unwrap();
        store_io::write(
            &folder.join("manifest.json"),
            &serde_json::json!({"status":"complete","ended_at_ms":t2,"duration_ms":30000}),
        )
        .unwrap();
        enqueue(&root.0, &s, None);
        s.helper = Some(Helper::Claude);
        save_fixture_settings(&root.0, &s).unwrap();
        let saved = crate::core::Config::load_in(&root.0)
            .unwrap()
            .notes
            .unwrap();
        assert!(t1 < t2 && t2 < saved.enabled_at_ms.unwrap());
        let mut fake = Fake::new(vec![]);
        drain(&root.0, &saved, &mut fake);
        assert_eq!(state(&root.0), JobState::NeedsClick);
        assert!(fake.calls.is_empty());
        assert!(!folder.join("english-transcript.md").exists());
        s.helper = Some(helper);
        for auto in [false, true] {
            s.auto = auto;
            s.your_name = "Changed name".into();
            save_fixture_settings(&root.0, &s).unwrap();
            assert_eq!(
                crate::core::Config::load_in(&root.0)
                    .unwrap()
                    .notes
                    .unwrap()
                    .enabled_at_ms,
                None
            );
        }
    }
}

#[test]
fn meeting_standard_choice_uses_builtin_without_overwriting_custom_instructions() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let s = settings_fixture();
    agent_instructions::save(&root.0, Agent::Notes, "CUSTOM NOTES").unwrap();
    let mut prefs = crate::core::UiPrefs::default();
    prefs
        .meeting_summary_templates
        .insert(ID.into(), "standard".into());
    prefs.save(&root.0).unwrap();
    enqueue(&root.0, &s, None);
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert!(!fake.calls[0].0.contains("CUSTOM NOTES"));
    assert!(
        fake.calls[0]
            .0
            .contains(agent_instructions::default_base(Agent::Notes))
    );
    assert_eq!(
        queue::read(&root.0, ID).unwrap().unwrap().instructions,
        "default"
    );
    assert!(
        agent_instructions::load(&root.0)
            .unwrap()
            .get(Agent::Notes)
            .is_some()
    );
    let saved = fs::read(folder.join("english-summary.json")).unwrap();
    prefs
        .meeting_summary_templates
        .insert(ID.into(), "custom".into());
    prefs.save(&root.0).unwrap();
    enqueue(&root.0, &s, None);
    let mut no_calls = Fake::new(vec![]);
    drain(&root.0, &s, &mut no_calls);
    assert!(no_calls.calls.is_empty());
    assert_eq!(
        fs::read(folder.join("english-summary.json")).unwrap(),
        saved
    );
}

#[test]
fn automatic_short_and_unknown_recordings_never_enqueue_or_call_helpers() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    fs::write(
        folder.join("transcript.ndjson"),
        serde_json::json!({"text":PRIVATE}).to_string(),
    )
    .unwrap();
    store_io::write(
        &root
            .0
            .join("local/recording-guard")
            .join(format!("{ID}.json")),
        &serde_json::json!({"short":false,"kept":false,"detected_at":null}),
    )
    .unwrap();
    let settings = settings_fixture();
    enqueue(&root.0, &settings, None);
    assert!(queue::read(&root.0, ID).unwrap().is_none());
    assert!(!folder.join("english-transcript.md").exists());
    fs::write(folder.join("transcript.ndjson"), "{\"text\":").unwrap();
    store_io::write(
        &root
            .0
            .join("local/recording-guard")
            .join(format!("{ID}.json")),
        &serde_json::json!({"short":false,"kept":false,"detected_at":null}),
    )
    .unwrap();
    enqueue(&root.0, &settings, None);
    assert!(queue::read(&root.0, ID).unwrap().is_none());
}
#[test]
fn worker_rechecks_old_jobs_and_keep_queues_once() {
    let root = Temp::new();
    let folder = meeting(&root.0, &vec!["synthetic"; 25].join(" "));
    let guard_path = root
        .0
        .join("local/recording-guard")
        .join(format!("{ID}.json"));
    store_io::write(
        &guard_path,
        &serde_json::json!({"short":false,"kept":false,"detected_at":null}),
    )
    .unwrap();
    let settings = settings_fixture();
    enqueue(&root.0, &settings, None);
    assert_eq!(state(&root.0), JobState::Queued);
    fs::write(
        folder.join("transcript.ndjson"),
        r#"{"start_ms":1000,"end_ms":2000,"source":"microphone","text":"abandoned"}"#,
    )
    .unwrap();
    let mut fake = Fake::new(vec![]);
    drain(&root.0, &settings, &mut fake);
    assert!(fake.calls.is_empty());
    assert_eq!(state(&root.0), JobState::Queued);
    crate::core::recording_guard::keep_in(&root.0, ID).unwrap();
    enqueue(&root.0, &settings, None);
    let before = fs::read(root.0.join("local/notes-jobs").join(format!("{ID}.json"))).unwrap();
    crate::core::recording_guard::keep_in(&root.0, ID).unwrap();
    enqueue(&root.0, &settings, None);
    assert_eq!(
        fs::read(root.0.join("local/notes-jobs").join(format!("{ID}.json"))).unwrap(),
        before
    );
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &settings, &mut fake);
    assert_eq!(fake.calls.len(), 2);
    enqueue(&root.0, &settings, None);
    drain(&root.0, &settings, &mut fake);
    assert_eq!(fake.calls.len(), 2);
}
#[test]
fn hex_labs_original_transcript_fallback_contract() {
    let temp = Temp::new();
    let folder = temp.0.join("synthetic");
    fs::create_dir_all(folder.join("hex-original")).unwrap();
    let original = include_str!("../../../tests/fixtures/hex-labs/original.ndjson");
    fs::write(folder.join("hex-original/transcript.ndjson"), original).unwrap();
    fs::write(folder.join("transcript.ndjson"), "invalid replacement").unwrap();
    assert!(
        transcript::read(&folder)
            .unwrap()
            .contains("Synthetic original transcript")
    );
    fs::remove_file(folder.join("hex-original/transcript.ndjson")).unwrap();
    fs::write(folder.join("transcript.ndjson"), original).unwrap();
    assert!(
        transcript::read(&folder)
            .unwrap()
            .contains("Synthetic original transcript")
    );
}

#[test]
fn codex_uses_the_notes_parser_and_limit_auth_failures_are_terminal() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let s = NotesSettings {
        helper: Some(Helper::Codex),
        ..settings_fixture()
    };
    enqueue(&root.0, &s, Some(Request::MakeNotes));
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert!(matches!(state(&root.0), JobState::Done { .. }));
    for (failure, expected) in [
        (Failure::Limit { exit: Some(1) }, "HELPER_LIMIT"),
        (Failure::Auth { exit: Some(2) }, "HELPER_AUTH"),
    ] {
        let root = Temp::new();
        meeting(&root.0, PRIVATE);
        enqueue(&root.0, &s, Some(Request::MakeNotes));
        let mut fake = Fake::new(vec![Err(failure)]);
        drain(&root.0, &s, &mut fake);
        let job = queue::read(&root.0, ID).unwrap().unwrap();
        assert_eq!(serde_json::to_value(&job).unwrap()["state"], expected);
        assert!(job.next_try_at.is_none());
        assert!(
            !fs::read_to_string(root.0.join("sidenote.log"))
                .unwrap()
                .contains(PRIVATE)
        );
    }
}

#[test]
fn summary_model_defaults_and_custom_models_survive_settings_roundtrip() {
    let legacy: NotesSettings = serde_json::from_str(r#"{"helper":"claude"}"#).unwrap();
    assert!(legacy.model.is_empty());
    assert_eq!(legacy.effective_helper(), Some(Helper::Claude));
    let root = Temp::new();
    let baseline = settings_fixture();
    save_fixture_settings(&root.0, &baseline).unwrap();
    for model in ["gpt-6.1-terra", "my-custom-model"] {
        let edited = NotesSettings {
            helper: Some(Helper::Codex),
            model: model.into(),
            ..baseline.clone()
        };
        save_fixture_settings(&root.0, &edited).unwrap();
        let saved = crate::core::Config::load_in(&root.0)
            .unwrap()
            .notes
            .unwrap();
        assert_eq!(saved.helper, Some(Helper::Codex));
        assert_eq!(saved.model, model);
    }
}

#[test]
fn queued_restarted_and_manual_retries_keep_the_enqueued_selection() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let original = NotesSettings {
        model: "original-model".into(),
        ..settings_fixture()
    };
    enqueue(&root.0, &original, Some(Request::MakeNotes));
    let path = root.0.join(format!("local/notes-jobs/{ID}.json"));
    let mut job = queue::read(&root.0, ID).unwrap().unwrap();
    assert_eq!(job.helper, original.helper);
    assert_eq!(job.model, "original-model");
    job.state = JobState::Running;
    store_io::write(&path, &job).unwrap();
    let changed_model = NotesSettings {
        model: "different-model".into(),
        ..original.clone()
    };
    let mut fake = Fake::new(vec![Err(Failure::Backend { exit: Some(1) })]);
    drain(&root.0, &changed_model, &mut fake);
    assert_eq!(fake.selections, [(Helper::Claude, "original-model".into())]);
    let mut job = queue::read(&root.0, ID).unwrap().unwrap();
    job.state = JobState::BackendError {
        next_try_ms: Some(0),
        exit: Some(1),
    };
    store_io::write(&path, &job).unwrap();
    let changed_helper = NotesSettings {
        helper: Some(Helper::Codex),
        ..changed_model.clone()
    };
    let mut no_calls = Fake::new(vec![]);
    drain(&root.0, &changed_helper, &mut no_calls);
    assert!(no_calls.calls.is_empty());
    assert_eq!(state(&root.0), JobState::NoHelper);
    enqueue(&root.0, &changed_helper, Some(Request::TryAgain));
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert_eq!(job.helper, Some(Helper::Claude));
    assert_eq!(job.model, "original-model");
    let mut retry = Fake::new(vec![Err(Failure::TimedOut)]);
    drain(&root.0, &changed_model, &mut retry);
    assert_eq!(
        retry.selections,
        [(Helper::Claude, "original-model".into())]
    );
}

#[test]
fn only_explicit_codex_recovery_retargets_one_job_and_preserves_first_part() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let original = settings_fixture();
    enqueue(&root.0, &original, Some(Request::FirstPart));
    let mut fake = Fake::new(vec![Err(Failure::Limit { exit: Some(1) })]);
    drain(&root.0, &original, &mut fake);
    let path = root.0.join(format!("local/notes-jobs/{ID}.json"));
    let before = fs::read(&path).unwrap();
    let codex = NotesSettings {
        helper: Some(Helper::Codex),
        model: "codex-choice".into(),
        ..original
    };
    drain(&root.0, &codex, &mut Fake::new(vec![]));
    assert_eq!(fs::read(&path).unwrap(), before);
    enqueue(&root.0, &codex, Some(Request::UseCodex));
    let queued = fs::read(&path).unwrap();
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert_eq!(job.helper, Some(Helper::Codex));
    assert_eq!(job.model, "codex-choice");
    assert_eq!(job.request, Some(Request::FirstPart));
    assert_eq!(job.state, JobState::Queued);
    enqueue(&root.0, &codex, Some(Request::UseCodex));
    assert_eq!(fs::read(&path).unwrap(), queued);
    assert_eq!(
        fs::read_dir(root.0.join("local/notes-jobs"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|f| f.path().extension().is_some_and(|e| e == "json"))
            .count(),
        1
    );
}

#[test]
fn unavailable_linux_execution_is_terminal_and_not_a_backend_or_setup_error() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    enqueue(&root.0, &settings, Some(Request::MakeNotes));
    drain(
        &root.0,
        &settings,
        &mut Fake::new(vec![Err(Failure::LinuxUnavailable)]),
    );
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert_eq!(job.state, JobState::HelperUnavailableOnLinux);
    assert_eq!(job.message, "HELPER_UNAVAILABLE_LINUX");
    assert!(job.next_try_at.is_none());
}

#[test]
fn scripts_claim_defers_app_without_consuming_attempt_then_releases() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    queue::enqueue_in(
        &root.0,
        &root.0.join("meetings"),
        ID,
        Some(Request::MakeNotes),
        &settings,
    )
    .unwrap();
    let claim = crate::core::meeting_job::claim(&folder).unwrap();
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &settings, &mut fake);
    assert!(fake.calls.is_empty());
    assert_eq!(queue::read(&root.0, ID).unwrap().unwrap().attempts, 0);
    assert!(!folder.join("english-summary.json").exists());
    drop(claim);
    drain(&root.0, &settings, &mut fake);
    assert!(folder.join("english-summary.json").exists());
}

#[test]
fn old_jobs_default_to_pipeline_and_failure_keeps_the_current_stage() {
    let root = Temp::new();
    meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    enqueue(&root.0, &settings, None);
    let path = root.0.join(format!("local/notes-jobs/{ID}.json"));
    let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    old.as_object_mut().unwrap().remove("stage");
    store_io::write(&path, &old).unwrap();
    assert_eq!(
        queue::read(&root.0, ID).unwrap().unwrap().stage,
        crate::core::meeting_job::Stage::Pipeline
    );
    let mut fake = Fake::new(vec![Err(Failure::Auth { exit: Some(1) })]);
    drain(&root.0, &settings, &mut fake);
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert_eq!(job.stage, crate::core::meeting_job::Stage::Notes);
    assert!(matches!(job.state, JobState::HelperAuth { .. }));
    assert_eq!(
        job.state.failure_cause(),
        Some(crate::core::meeting_job::Cause::Auth)
    );
}

#[test]
fn summary_skip_covers_guard_policy_and_empty_jobs() {
    use crate::core::recording_guard::Eligibility::{Eligible, Short, Unknown};
    assert_eq!(summary_skip(Short, false, None), Some(SummarySkip::Short));
    assert_eq!(summary_skip(Eligible, true, None), Some(SummarySkip::Off));
    assert_eq!(
        summary_skip(Eligible, false, Some(&JobState::Empty)),
        Some(SummarySkip::Empty)
    );
    for eligibility in [Eligible, Unknown] {
        assert_eq!(summary_skip(eligibility, false, None), None);
    }
    for state in [
        JobState::Queued,
        JobState::Running,
        JobState::BackendError {
            next_try_ms: Some(i64::MAX),
            exit: None,
        },
        JobState::Done {
            fallback: false,
            tasks_failed: false,
        },
    ] {
        assert_eq!(summary_skip(Eligible, true, Some(&state)), None);
        assert_eq!(summary_skip(Eligible, false, Some(&state)), None);
        assert_eq!(
            summary_skip(Short, false, Some(&state)),
            Some(SummarySkip::Short)
        );
    }
}

#[test]
fn accidental_seventeen_second_call_has_no_job_and_a_terminal_skip() {
    let root = tempfile::tempdir().unwrap();
    let meetings = root.path().join("meetings");
    let folder = meetings.join(ID);
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("manifest.json"),
        r#"{"status":"complete","duration_ms":16700}"#,
    )
    .unwrap();
    fs::write(folder.join("transcript.ndjson"), "").unwrap();
    queue::enqueue_in(root.path(), &meetings, ID, None, &settings_fixture()).unwrap();
    assert!(queue::read(root.path(), ID).unwrap().is_none());
    let eligibility =
        crate::core::recording_guard::eligibility(root.path(), &meetings, ID).unwrap();
    let skipped = summary_skip(eligibility, false, None).unwrap();
    assert_eq!(skipped.heading(), "Summary skipped");
    assert!(skipped.detail().contains("Too short to summarize"));
    crate::core::recording_guard::keep_in(root.path(), ID).unwrap();
    let eligibility =
        crate::core::recording_guard::eligibility(root.path(), &meetings, ID).unwrap();
    assert_eq!(
        summary_skip(eligibility, false, Some(&JobState::Queued)),
        None
    );
}

#[cfg(all(windows, feature = "asr-runtime"))]
pub(crate) fn publish_public_routed(
    root: &Path,
    meetings: &Path,
    id: &str,
    expected: &str,
) -> StatusDetail {
    let settings = settings_fixture();
    save_fixture_settings(root, &settings).unwrap();
    queue::enqueue_in(root, meetings, id, None, &settings).unwrap();
    let summary = serde_json::json!({"title":"Public source regression","context":[],"topics":[],"todo":[],"to_settle":[]}).to_string();
    let mut helper = Fake::new(vec![
        Ok("Public English source".into()),
        Ok(summary),
        Ok(r#"{"meeting_project":null,"tasks":[]}"#.into()),
    ]);
    queue::drain(root, meetings, &mut helper, || Ok(settings.clone())).unwrap();
    assert!(
        meetings.join(id).join("english-summary.json").is_file(),
        "notes must publish from complete Hex coverage"
    );
    assert!(helper.calls[1].0.contains(expected));
    let status = status_record(root, id).unwrap();
    assert!(matches!(
        status.state,
        JobState::Done {
            fallback: false,
            tasks_failed: false
        }
    ));
    assert!(!helper.calls[1].0.contains(&status.routing_message()));
    status
}

#[test]
fn multipart_pause_resume_end_runs_once_over_ordered_parts() {
    use crate::core::meeting_parts::{self, Meeting, Phase};
    let root = Temp::new();
    let first = meeting(&root.0, "First part has the delivery quantities.");
    let hex = root.0.join("meetings");
    let second = hex.join("1793947600000-456");
    fs::create_dir_all(&second).unwrap();
    fs::write(second.join("manifest.json"), serde_json::json!({"status":"complete","created_at_ms":1793947600000_i64,"duration_ms":60000}).to_string()).unwrap();
    fs::write(second.join("transcript.ndjson"), serde_json::json!({"start_ms":1000,"end_ms":2000,"source":"system","text":"Second part confirms Friday."}).to_string()).unwrap();
    let mut session = Meeting::begin(&root.0, ID, 1793946600000).unwrap();
    session.refresh(&hex).unwrap();
    session.pause(&root.0).unwrap();
    let settings = settings_fixture();
    queue::enqueue_in(&root.0, &hex, ID, None, &settings).unwrap();
    assert!(
        queue::read(&root.0, ID).unwrap().is_none(),
        "Pause queued notes"
    );
    session
        .resume(&root.0, "1793947600000-456", 1793947600000)
        .unwrap();
    session.refresh(&hex).unwrap();
    session.pause(&root.0).unwrap();
    assert_eq!(
        session
            .parts
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        [ID, "1793947600000-456"]
    );
    assert_eq!(session.elapsed_ms(), 360000);
    assert_eq!(
        session.clone().restore(&root.0, &hex).unwrap().phase,
        Phase::Paused
    );
    let input = session.prepare_notes(&root.0, &hex).unwrap();
    let mut fake = Fake::new(vec![
        Ok(r#"{"title":"Delivery planning"}"#.into()),
        Ok(summary()),
        Ok(sorted()),
    ]);
    let inputs = meeting_parts::inputs(&root.0);
    queue::enqueue_in(&root.0, &inputs, ID, Some(Request::TryAgain), &settings).unwrap();
    queue::drain(&root.0, &hex, &mut fake, || Ok(settings.clone())).unwrap();
    assert_eq!(fake.calls.len(), 3);
    assert!(
        fake.calls[1].0.find("First part").unwrap() < fake.calls[1].0.find("Second part").unwrap()
    );
    assert!(session.notes_ready(&root.0).unwrap());
    let raw = fs::read_to_string(input.join("transcript.ndjson")).unwrap();
    assert!(raw.contains("301000"));
    let calls = fake.calls.len();
    queue::enqueue_in(&root.0, &inputs, ID, None, &settings).unwrap();
    queue::drain(&root.0, &hex, &mut fake, || Ok(settings.clone())).unwrap();
    assert_eq!(fake.calls.len(), calls, "End dispatched twice");
    session.notes_parts = session.requested_parts;
    session
        .resume(&root.0, "1793948600000-789", 1793948600000)
        .unwrap();
    assert!(session.stale());
    queue::drain(&root.0, &hex, &mut fake, || Ok(settings.clone())).unwrap();
    assert_eq!(fake.calls.len(), calls, "Resume regenerated notes");
    assert!(first.join("transcript.ndjson").is_file());
}

#[test]
fn resume_while_notes_are_queued_keeps_the_requested_snapshot() {
    use crate::core::meeting_parts::Meeting;
    let root = Temp::new();
    let first = meeting(&root.0, "First requested snapshot");
    let hex = root.0.join("meetings");
    let mut session = Meeting::begin(&root.0, ID, 1793946600000).unwrap();
    let folder = session.prepare_notes(&root.0, &hex).unwrap();
    let snapshot = fs::read(folder.join("transcript.ndjson")).unwrap();
    let manifest = fs::read(folder.join("manifest.json")).unwrap();
    let settings = settings_fixture();
    queue::enqueue_in(
        &root.0,
        folder.parent().unwrap(),
        ID,
        Some(Request::TryAgain),
        &settings,
    )
    .unwrap();
    let second = "1793947600000-456";
    fs::create_dir_all(hex.join(second)).unwrap();
    for name in ["manifest.json", "transcript.ndjson"] {
        fs::copy(first.join(name), hex.join(second).join(name)).unwrap();
    }
    session.resume(&root.0, second, 1793947600000).unwrap();
    session.pause(&root.0).unwrap();
    assert_eq!(fs::read(folder.join("manifest.json")).unwrap(), manifest);
    assert_eq!(
        session.prepare_notes(&root.0, &hex).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        fs::read(folder.join("transcript.ndjson")).unwrap(),
        snapshot
    );
    let mut fake = Fake::new(vec![
        Ok(r#"{"title":"Requested snapshot"}"#.into()),
        Ok(summary()),
        Ok(sorted()),
    ]);
    queue::drain(&root.0, &hex, &mut fake, || Ok(settings.clone())).unwrap();
    assert_eq!(fake.calls.len(), 3);
    assert_eq!(
        fake.calls[1].0.matches("First requested snapshot").count(),
        1
    );
    session.notes_parts = session.requested_parts;
    assert!(session.stale());
}

mod edited_summary;
