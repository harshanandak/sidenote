use super::super::*;
mod api;
#[cfg(windows)]
mod recovery;
use api::Fixture;
#[cfg(all(windows, feature = "recorder-ready-proof"))]
mod readiness;
use std::sync::atomic::Ordering;

fn command_status_sample(state: &State) -> StatusSample {
    sample_status(|| Ok(state.hex.engine.status_for_command().unwrap()))
}

fn isolated_api_test(name: &str) -> bool {
    if std::env::var("SIDENOTE_HEX_API_TEST").as_deref() == Ok(name) {
        return false;
    }
    let root = tempfile::Builder::new()
        .prefix("live-replacement-")
        .tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts"))
        .unwrap();
    let mut exe = std::env::current_exe().unwrap();
    if name
        == "recorder_ready_installed_preparation_disables_the_real_control_until_validation_finishes"
    {
        std::fs::create_dir_all(root.path().join("data")).unwrap();
        let current = root.path().join("current");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(root.path().join("Update.exe"), []).unwrap();
        std::fs::write(current.join("sq.version"), "<package><metadata><id>Fixture</id><version>1.0.0</version><mainExe>proof.exe</mainExe></metadata></package>").unwrap();
        let copied = current.join("proof.exe");
        std::fs::copy(exe, &copied).unwrap();
        exe = copied;
    }
    let mut command = std::process::Command::new(exe);
    command
        .args([
            "--exact",
            &format!("ui::hex::connection::stop_tests::{name}"),
            "--nocapture",
        ])
        .env("SIDENOTE_HEX_API_TEST", name)
        .env("SIDENOTE_DATA_DIR", root.path().join("data"))
        .env("SIDENOTE_STATE_DIR", root.path().join("state"))
        .env("SIDENOTE_HEX_DIR", root.path().join("hex"))
        .env("SIDENOTE_PEERS_DIR", root.path().join("peers"));
    if name
        == "recorder_ready_installed_preparation_disables_the_real_control_until_validation_finishes"
    {
        command
            .env("SIDENOTE_RECORDER_READY_TEST", root.path())
            .env(
                "SIDENOTE_LISTENER_TASK",
                "Sidenote recorder readiness fixture",
            );
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    assert!(
        command.status().unwrap().success(),
        "isolated Hex API test failed"
    );
    true
}

fn pending_notes(state: &mut State, api: &Fixture, text: &str) -> u64 {
    use crate::ui::live::Phase;
    state.live.as_mut().unwrap().phase = Phase::Making;
    api.data.lock().unwrap().id = None;
    let idle = state.hex.engine.status_for_command().unwrap();
    state.hex_status_at(Ok(idle), Instant::now() - Duration::from_secs(11));
    assert!(state.hex_status_unknown());
    state.live_notes = rune::TextEdit::new(text);
    state.live_tick();
    if !text.is_empty() {
        assert_eq!(
            state.live.as_ref().unwrap().saving,
            crate::ui::live::Saving::Pending
        );
    }
    api.data.lock().unwrap().id = Some("200-2");
    state.live.as_ref().unwrap().recording_id
}

fn active_recheck(state: &mut State) {
    // Linux can read this status. The New meeting action itself remains unavailable.
    #[cfg(target_os = "linux")]
    state.hex_recheck_start();
    #[cfg(not(target_os = "linux"))]
    state.new_meeting();
    Fixture::finish(state);
}

#[test]
fn active_recheck_preserves_pending_notes() {
    if isolated_api_test("active_recheck_preserves_pending_notes") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    let old = pending_notes(&mut state, &api, "Synthetic notes inside autosave delay");
    active_recheck(&mut state);
    assert_eq!(api.data.lock().unwrap().starts, 0);
    let recovery = sidenote::core::state_dir().join("unsaved-notes");
    let files: Vec<_> = std::fs::read_dir(recovery)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(
        std::fs::read_to_string(files[0].path()).unwrap(),
        "Synthetic notes inside autosave delay"
    );
    assert_ne!(state.live.as_ref().unwrap().recording_id, old);
    assert_eq!(
        state.live.as_ref().unwrap().phase,
        crate::ui::live::Phase::Recording
    );
    assert!(
        state
            .hex
            .error
            .unwrap()
            .contains("No new meeting was started")
    );
}

#[test]
fn active_recheck_failed_save_keeps_pending_notes() {
    if isolated_api_test("active_recheck_failed_save_keeps_pending_notes") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    let old = pending_notes(&mut state, &api, "Synthetic notes on failed save");
    let recovery = sidenote::core::state_dir().join("unsaved-notes");
    std::fs::create_dir_all(recovery.parent().unwrap()).unwrap();
    std::fs::write(&recovery, "synthetic blocker").unwrap();
    active_recheck(&mut state);
    assert_eq!(api.data.lock().unwrap().starts, 0);
    let live = state.live.as_ref().unwrap();
    assert_eq!(live.recording_id, old);
    assert_eq!(live.phase, crate::ui::live::Phase::Making);
    assert_eq!(state.live_notes.text, "Synthetic notes on failed save");
    assert!(live.close_failed);
    assert!(matches!(live.saving, crate::ui::live::Saving::Failed(_)));
    assert!(state.on_live());
}

#[test]
fn active_recheck_replaces_empty_page() {
    if isolated_api_test("active_recheck_replaces_empty_page") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    let old = pending_notes(&mut state, &api, "");
    active_recheck(&mut state);
    assert_eq!(api.data.lock().unwrap().starts, 0);
    assert_ne!(state.live.as_ref().unwrap().recording_id, old);
    assert_eq!(
        state.live.as_ref().unwrap().phase,
        crate::ui::live::Phase::Recording
    );
    assert!(!state.live.as_ref().unwrap().close_failed);
}

#[cfg(windows)]
fn saved_removal_notes(text: &str) {
    let recovery = sidenote::core::state_dir().join("unsaved-notes");
    assert!(
        std::fs::read_dir(recovery)
            .unwrap()
            .any(|entry| { std::fs::read_to_string(entry.unwrap().path()).unwrap() == text }),
        "latest notes must be durable before removing or leaving the page"
    );
}

#[cfg(windows)]
fn assert_recovery_close(state: &mut State, id: &str, text: &str) {
    use sidenote::core::{self, meeting_parts};
    assert!(state.live_close());
    assert!(!state.on_live(), "restored storage must permit navigation");
    let root = core::state_dir();
    assert!(meeting_parts::read(&root, id).unwrap().is_none());
    assert!(!meeting_parts::inputs(&root).join(id).exists());
    saved_removal_notes(text);
    let mut restarted = State::new();
    let idle = command_status_sample(state);
    restarted.hex_apply(Msg::ObservedStatus(idle));
    assert!(restarted.live.is_none());
}

#[cfg(windows)]
fn start_removal_cases(failed_start: bool) {
    use crate::ui::live::{Phase, Saving};
    use sidenote::core::{self, meeting_parts};
    for (text, fail_save) in [
        ("Synthetic pending Start notes", true),
        ("Synthetic pending Start notes", false),
        ("", false),
    ] {
        let (api, mut state, _) = Fixture::start();
        let (release, wait) = std::sync::mpsc::channel();
        {
            let mut data = api.data.lock().unwrap();
            data.id = None;
            data.start_failure = failed_start;
            data.start_records = false;
            data.start_release = Some(wait);
        }
        let idle = command_status_sample(&state);
        state.hex_apply(Msg::ObservedStatus(idle));
        state.new_meeting();
        assert!(state.hex.record_busy);
        let live = state.live.as_ref().unwrap();
        assert_eq!(live.phase, Phase::Starting);
        let page = live.recording_id;
        let id = live.session.as_ref().unwrap().id.clone();
        let root = core::state_dir();
        let folder = meeting_parts::inputs(&root).join(&id);
        let notes = folder.join("notes.md");
        if !failed_start {
            release.send(()).unwrap();
            Fixture::finish(&mut state);
            assert!(state.hex.error.is_none());
            assert!(
                state.live.is_some(),
                "idle before START_WAIT must keep the page"
            );
            state.live.as_mut().unwrap().age_past_start_timeout();
        }
        state.live_notes = rune::TextEdit::new(text);
        state.live_tick();
        if !text.is_empty() {
            assert_eq!(state.live.as_ref().unwrap().saving, Saving::Pending);
        }
        assert!(!notes.exists());
        let recovery = root.join("unsaved-notes");
        if fail_save {
            std::fs::create_dir(&notes).unwrap();
            std::fs::create_dir_all(recovery.parent().unwrap()).unwrap();
            std::fs::write(&recovery, "synthetic recovery blocker").unwrap();
        }
        let (saved, wake) = std::sync::mpsc::channel();
        state.waker = Some(std::sync::Arc::new(move || {
            let _ = saved.send(());
        }));
        let (scan, dirty) = crate::ui::refresh::observing_handle();
        state.scan_tx = Some(scan);
        if failed_start {
            release.send(()).unwrap();
            Fixture::finish(&mut state);
            assert!(
                state.hex.error.is_some(),
                "apply the real failed Start reply"
            );
        } else {
            let idle = command_status_sample(&state);
            assert!(idle.1.as_ref().unwrap().recording.is_none());
            state.hex_apply(Msg::ObservedStatus(idle));
        }
        let save_waker = state.waker.take();
        if !text.is_empty() {
            wake.recv_timeout(Duration::from_secs(2)).unwrap();
            state.live_tick();
        }
        assert_eq!(api.data.lock().unwrap().starts, 1);
        if fail_save {
            let live = state.live.as_ref().expect("failed save must keep the page");
            assert_eq!(live.recording_id, page);
            assert_eq!(live.phase, Phase::Starting);
            assert_eq!(state.live_notes.text, text);
            assert!(matches!(live.saving, Saving::Failed(_)));
            assert!(state.on_live());
            assert_eq!(live.session.as_ref().unwrap().id, id);
            assert!(notes.is_dir());
            assert!(meeting_parts::read(&root, &id).unwrap().is_some());
            assert!(!state.live_close(), "blocked storage must refuse close");
            std::fs::remove_file(&recovery).unwrap();
            let edited = "Synthetic notes edited after storage recovery";
            state.live_notes = rune::TextEdit::new(edited);
            state.waker = save_waker;
            state.live_flush();
            wake.recv_timeout(Duration::from_secs(2)).unwrap();
            state.waker = None;
            state.live_tick();
            assert!(state.on_live() && notes.is_dir());
            saved_removal_notes(edited);
            assert_recovery_close(&mut state, &id, edited);
        } else {
            assert!(state.live.is_none(), "saved or empty page may be removed");
            assert!(!state.on_live());
            if !text.is_empty() {
                assert_eq!(std::fs::read_to_string(notes).unwrap(), text);
                let record = meeting_parts::read(&root, &id).unwrap().unwrap();
                assert_eq!(record.phase, meeting_parts::Phase::Ended);
                assert!(record.pending_start_ms.is_none() && record.parts.is_empty());
                assert!(dirty.load(Ordering::Acquire));
                let mut restarted = State::new();
                let idle = command_status_sample(&state);
                restarted.hex_apply(Msg::ObservedStatus(idle));
                assert!(restarted.live.is_none());
                let listed = restarted.meetings.iter().find(|m| m.id == id).unwrap();
                assert_eq!(listed.notes, text);
                assert!(crate::ui::notes_status::job(&restarted, listed).is_none());
            } else {
                assert!(meeting_parts::read(&root, &id).unwrap().is_none());
                assert!(!folder.exists());
            }
        }
    }
}

#[cfg(windows)]
#[test]
fn start_timeout_preserves_pending_notes_before_removal() {
    if isolated_api_test("start_timeout_preserves_pending_notes_before_removal") {
        return;
    }
    start_removal_cases(false);
}

#[cfg(windows)]
fn refused_start_timeout(name: &str, restore_primary: bool) {
    if isolated_api_test(name) {
        return;
    }
    let (api, mut state, _) = Fixture::start_idle();
    api.data.lock().unwrap().start_records = false;
    let idle = command_status_sample(&state);
    state.hex_apply(Msg::ObservedStatus(idle));
    state.new_meeting();
    assert!(state.hex.record_busy);
    Fixture::finish(&mut state);
    assert!(state.hex.error.is_none());
    let live = state.live.as_mut().unwrap();
    assert_eq!(live.phase, crate::ui::live::Phase::Starting);
    live.age_past_start_timeout();
    let page = live.recording_id;
    let id = live.session.as_ref().unwrap().id.clone();

    let root = sidenote::core::state_dir();
    let notes = sidenote::core::meeting_parts::inputs(&root)
        .join(&id)
        .join(sidenote::core::live::NOTES_FILE);
    let recovery = root.join("unsaved-notes");
    std::fs::create_dir(&notes).unwrap();
    std::fs::write(&recovery, "synthetic recovery blocker").unwrap();
    let text = "Synthetic notes after a refused close";
    state.live_notes = rune::TextEdit::new(text);
    assert!(!state.live_close(), "both note destinations are blocked");

    let idle = command_status_sample(&state);
    assert!(idle.1.as_ref().unwrap().recording.is_none());
    state.hex_apply(Msg::ObservedStatus(idle));
    let live = state.live.as_ref().expect("failed save must keep the page");
    assert_eq!(live.recording_id, page);
    assert!(live.close_failed);
    assert_eq!(state.live_notes.text, text);

    std::fs::remove_file(&recovery).unwrap();
    if restore_primary {
        std::fs::remove_dir(&notes).unwrap();
    }
    let (saved, wake) = std::sync::mpsc::channel();
    state.waker = Some(std::sync::Arc::new(move || {
        let _ = saved.send(());
    }));
    state.live_flush();
    wake.recv_timeout(Duration::from_secs(2)).unwrap();
    state.waker = None;
    state.live_tick();
    if restore_primary {
        assert_eq!(std::fs::read_to_string(&notes).unwrap(), text);
    } else {
        saved_removal_notes(text);
    }

    let idle = command_status_sample(&state);
    assert!(idle.1.as_ref().unwrap().recording.is_none());
    state.hex_apply(Msg::ObservedStatus(idle));
    if !restore_primary {
        let live = state.live.as_ref().unwrap();
        assert_eq!(live.session.as_ref().unwrap().id, id);
        assert!(state.on_live() && notes.is_dir());
        assert_recovery_close(&mut state, &id, text);
        return;
    }
    assert!(
        state.live.is_none(),
        "durable notes must let the timeout finish"
    );
    if let Ok(preserved) = std::fs::read_to_string(&notes) {
        assert_eq!(preserved, text);
    } else {
        saved_removal_notes(text);
    }
    if restore_primary {
        let mut restarted = State::new();
        let idle = command_status_sample(&state);
        restarted.hex_apply(Msg::ObservedStatus(idle));
        assert!(
            restarted.live.is_none(),
            "a finished draft must not reopen on restart"
        );
        let listed = restarted
            .meetings
            .iter()
            .find(|meeting| meeting.id == id)
            .expect("saved draft must be discoverable in the meeting list");
        assert_eq!(listed.notes, text);
        assert!(crate::ui::notes_status::job(&restarted, listed).is_none());
        assert!(crate::ui::notes_status::read(&root, std::slice::from_ref(listed)).is_empty());
        restarted.select(&id, &mut rune::Tx::default());
        assert!(matches!(
            restarted.meeting_tab,
            crate::ui::meeting::Tab::Notes
        ));
        let mut view = rune::Runtime::new(
            restarted,
            crate::ui::view,
            (960.0, 720.0),
            rune::Env::default(),
        );
        view.frame(0.0);
        let elements = view.elements();
        assert!(
            elements
                .iter()
                .any(|element| element.text.as_deref() == Some(text))
        );
        assert!(!elements.iter().any(|element| matches!(
            element.key.as_deref(),
            Some("meeting-resume" | "meeting-generate")
        )));
    }
}

#[cfg(windows)]
#[test]
fn failed_start_close_keeps_autosaved_primary_and_newer_recovery() {
    if isolated_api_test("failed_start_close_keeps_autosaved_primary_and_newer_recovery") {
        return;
    }
    use sidenote::core::{self, meeting_parts};
    let (api, mut state, _) = Fixture::start_idle();
    let (release, wait) = std::sync::mpsc::channel();
    {
        let mut data = api.data.lock().unwrap();
        data.start_failure = true;
        data.start_release = Some(wait);
    }
    let idle = command_status_sample(&state);
    state.hex_apply(Msg::ObservedStatus(idle));
    state.new_meeting();
    assert!(state.hex.record_busy);
    let primary = "Earlier synthetic notes";
    let newer = "Newer synthetic notes\nKeep every line";
    state.live_notes = rune::TextEdit::new(primary);
    state.live_tick();
    let live = state.live.as_mut().unwrap();
    live.age_past_notes_autosave();
    let id = live.session.as_ref().unwrap().id.clone();
    let root = core::state_dir();
    let notes = meeting_parts::inputs(&root).join(&id).join("notes.md");
    let (saved, wake) = std::sync::mpsc::channel();
    state.waker = Some(std::sync::Arc::new(move || {
        let _ = saved.send(());
    }));
    state.live_tick();
    wake.recv_timeout(Duration::from_secs(2)).unwrap();
    state.live_tick();
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), primary);

    let permissions = std::fs::metadata(&notes).unwrap().permissions();
    let mut read_only = permissions.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&notes, read_only).unwrap();
    state.live_notes = rune::TextEdit::new(newer);
    release.send(()).unwrap();
    Fixture::finish(&mut state);
    assert!(state.hex.error.is_some());
    wake.recv_timeout(Duration::from_secs(2)).unwrap();
    state.waker = None;
    state.live_tick();
    assert!(state.on_live());
    saved_removal_notes(newer);
    let closed = state.live_close();
    std::fs::set_permissions(&notes, permissions).unwrap();
    assert!(closed);
    assert!(!state.on_live());
    let record = meeting_parts::read(&root, &id).unwrap().unwrap();
    assert_eq!(record.phase, meeting_parts::Phase::Ended);
    assert!(record.pending_start_ms.is_none() && record.parts.is_empty());

    let mut restarted = State::new();
    let idle = command_status_sample(&state);
    restarted.hex_apply(Msg::ObservedStatus(idle));
    assert!(restarted.live.is_none());
    let listed = restarted.meetings.iter().find(|m| m.id == id).unwrap();
    assert_eq!(listed.notes, primary);
    assert!(crate::ui::notes_status::job(&restarted, listed).is_none());
    restarted.select(&id, &mut rune::Tx::default());
    let mut view = rune::Runtime::new(
        restarted,
        crate::ui::view,
        (960.0, 720.0),
        rune::Env::default(),
    );
    view.frame(0.0);
    assert!(!view.elements().iter().any(|element| matches!(
        element.key.as_deref(),
        Some("meeting-resume" | "meeting-generate")
    )));
    saved_removal_notes(newer);
}

#[cfg(windows)]
#[test]
fn failed_resume_keeps_the_recorded_part_paused() {
    if isolated_api_test("failed_resume_keeps_the_recorded_part_paused") {
        return;
    }
    use sidenote::core::{self, meeting_parts};
    let (api, mut state, _) = Fixture::start_idle();
    api.data.lock().unwrap().start_failure = true;
    let part = core::meetings_dir().join("100-1");
    std::fs::create_dir_all(&part).unwrap();
    std::fs::write(part.join("manifest.json"), r#"{"status":"complete"}"#).unwrap();
    let root = core::state_dir();
    meeting_parts::Meeting::begin(&root, "100-1", 100)
        .unwrap()
        .pause(&root)
        .unwrap();
    state.hex.parts_restore_checked = false;
    let idle = command_status_sample(&state);
    state.hex_apply(Msg::ObservedStatus(idle));
    state.meeting_resume();
    Fixture::finish(&mut state);
    assert_eq!(api.data.lock().unwrap().starts, 1);
    assert_eq!(
        state.live.as_ref().unwrap().phase,
        crate::ui::live::Phase::Paused
    );
    let record = meeting_parts::read(&root, "100-1").unwrap().unwrap();
    assert_eq!(record.phase, meeting_parts::Phase::Paused);
    assert!(record.pending_start_ms.is_none());
    assert_eq!(record.parts.len(), 1);
}

#[cfg(windows)]
#[test]
fn refused_close_resumes_start_timeout_after_notes_recovery() {
    refused_start_timeout(
        "refused_close_resumes_start_timeout_after_notes_recovery",
        false,
    );
}

#[cfg(windows)]
#[test]
fn refused_close_resumes_start_timeout_after_primary_save() {
    refused_start_timeout(
        "refused_close_resumes_start_timeout_after_primary_save",
        true,
    );
}

#[test]
fn refused_close_save_failure_log_contains_only_route_and_kind() {
    if isolated_api_test("refused_close_save_failure_log_contains_only_route_and_kind") {
        return;
    }
    crate::test_paths::isolate();
    let root = sidenote::core::state_dir();
    let meeting = root.join("synthetic-private-meeting-123");
    let recovery = root.join("synthetic-blocked-recovery");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&recovery, "synthetic blocker").unwrap();
    let kind = std::fs::create_dir_all(&recovery).unwrap_err().kind();

    assert!(
        sidenote::core::live::preserve_notes(
            Some(meeting),
            "Synthetic private draft",
            &recovery.join("draft.md"),
        )
        .is_err()
    );
    let log = std::fs::read_to_string(sidenote::core::log_path()).unwrap_or_default();
    let messages: Vec<_> = log
        .lines()
        .filter_map(|line| line.split_once(" listener: ").map(|(_, message)| message))
        .collect();
    assert_eq!(
        messages,
        [
            "live notes save failed route=meeting_folder kind=NotFound".to_owned(),
            format!("live notes save failed route=recovery_copy kind={kind:?}"),
        ]
    );
}

#[cfg(windows)]
#[test]
fn failed_start_preserves_pending_notes_before_removal() {
    if isolated_api_test("failed_start_preserves_pending_notes_before_removal") {
        return;
    }
    start_removal_cases(true);
}

fn no_id_stop(phase: &'static str) {
    use crate::ui::live::Phase;

    let (api, mut state, now) = Fixture::start();
    {
        let mut data = api.data.lock().unwrap();
        data.omit_id = true;
        data.meeting_state = Some(phase);
    }
    let status = command_status_sample(&state);
    state.hex_apply(Msg::ObservedStatus(status));
    assert_eq!(recording_id(&state), None);
    {
        let mut data = api.data.lock().unwrap();
        data.commands.clear();
        data.delay = Duration::from_millis(1000);
    }
    state.stop_during_feed_read();
    Fixture::finish(&mut state);
    {
        let mut data = api.data.lock().unwrap();
        assert_eq!(data.stops, 1, "no-ID {phase} must send exactly one Stop");
        assert!(
            data.commands
                .starts_with(&["status".into(), "meeting-stop".into()])
        );
        assert_eq!(data.id, None);
        data.delay = Duration::ZERO;
    }
    Fixture::stop_status(&mut state);
    if state.hex_stop_pending() {
        let idle = command_status_sample(&state);
        state.hex_apply(Msg::ObservedStatus(idle));
        Fixture::stop_status(&mut state);
    }
    assert!(state.hex.error.is_none());
    assert_eq!(state.live.as_ref().unwrap().phase, Phase::Making);
    state.hex_status_at(Err(HexError::Timeout), now + Duration::from_secs(11));
    assert_eq!(state.live.as_ref().unwrap().phase, Phase::Making);

    state.hex_check_settings();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let event = state
            .events_rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("Settings status after Stop");
        if let Event::Hex(msg @ Msg::SettingsStatus(_)) = event {
            state.hex_apply(msg);
            break;
        }
    }
    assert!(matches!(&state.hex.status, Some(Ok(status)) if status.meeting_state == "idle"));
    assert!(!state.hex_recording());
    assert_eq!(state.live.as_ref().unwrap().phase, Phase::Making);
}

#[test]
fn no_id_hex_stops_after_confirming_recording_through_ui_and_engine() {
    if isolated_api_test("no_id_hex_stops_after_confirming_recording_through_ui_and_engine") {
        return;
    }
    no_id_stop("recording");
}

#[test]
fn no_id_hex_stops_after_confirming_starting_through_ui_and_engine() {
    if isolated_api_test("no_id_hex_stops_after_confirming_starting_through_ui_and_engine") {
        return;
    }
    no_id_stop("starting");
}

#[test]
fn no_id_hex_idle_or_transcribing_does_not_stop_through_ui_and_engine() {
    if isolated_api_test("no_id_hex_idle_or_transcribing_does_not_stop_through_ui_and_engine") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    api.data.lock().unwrap().omit_id = true;
    for phase in ["idle", "transcribing"] {
        api.data.lock().unwrap().meeting_state = Some("recording");
        let status = command_status_sample(&state);
        state.hex_apply(Msg::ObservedStatus(status));
        api.data.lock().unwrap().meeting_state = Some(phase);
        Fixture::stop(&mut state);
        assert_eq!(api.data.lock().unwrap().stops, 0);
        let message = state.hex.error.unwrap();
        if phase == "idle" {
            assert_eq!(message, "The recording already ended in Hex.");
        } else {
            assert_eq!(message, "Recording stopped. Hex is still transcribing.");
            state.hex_status_at(
                Err(HexError::Timeout),
                Instant::now() + Duration::from_secs(11),
            );
            assert!(state.hex_status_unknown());
            Fixture::stop_status(&mut state);
            assert!(!state.hex_status_unknown());
        }
    }
    api.data.lock().unwrap().meeting_state = Some("idle");
    let idle = command_status_sample(&state);
    state.hex_apply(Msg::ObservedStatus(idle));
    Fixture::stop_status(&mut state);
    Fixture::stop(&mut state);
    assert_eq!(api.data.lock().unwrap().stops, 0);
    assert_eq!(state.hex.error, Some("Hex says no meeting is recording."));
}

#[test]
fn unchanged_stop_reply_shows_hex_idle_message_through_ui_and_engine() {
    if isolated_api_test("unchanged_stop_reply_shows_hex_idle_message_through_ui_and_engine") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    api.data.lock().unwrap().stop_changed = false;
    Fixture::stop(&mut state);
    assert_eq!(api.data.lock().unwrap().stops, 1);
    assert_eq!(state.hex.error, Some("Hex says no meeting is recording."));
}

#[test]
fn stale_stop_refuses_replacement_recording_through_ui_and_engine() {
    if isolated_api_test("stale_stop_refuses_replacement_recording_through_ui_and_engine") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    api.data.lock().unwrap().id = Some("200-2");
    Fixture::stop(&mut state);
    let (stops, id) = {
        let data = api.data.lock().unwrap();
        (data.stops, data.id)
    };
    assert_eq!(stops, 0, "must not send generic Stop for replacement B");
    assert_eq!(id, Some("200-2"), "B must keep recording");
    let message = state.hex.error.unwrap();
    assert!(message.contains("recording a different meeting"));
    assert!(message.contains("Stop it from Hex"));
}

#[test]
fn expired_header_stop_reconciles_each_hex_outcome() {
    if isolated_api_test("expired_header_stop_reconciles_each_hex_outcome") {
        return;
    }
    fn header(state: &State, _: &mut rune::Cx<State>) -> rune::El {
        let mut layer = Vec::new();
        let actions =
            crate::ui::nav::actions(state, crate::ui::theme::palette(false), false, &mut layer);
        rune_kit::root(actions, layer)
    }
    for phase in ["idle", "transcribing", "recording", "timeout"] {
        let (api, mut state, now) = Fixture::start();
        state.hex_status_at(Err(HexError::Timeout), now + Duration::from_secs(11));
        state.live_open = false;
        {
            let mut data = api.data.lock().unwrap();
            data.meeting_state = Some(phase);
            data.id = (phase == "recording").then_some("200-2");
            data.status_timeout = phase == "timeout";
        }
        let mut ui = rune::Runtime::new(state, header, (600.0, 160.0), rune::Env::default());
        ui.frame(0.0);
        let stop = ui
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("hex-stop"))
            .unwrap();
        let [x, y, w, h] = stop.rect;
        let (x, y) = (x + w / 2.0, y + h / 2.0);
        assert_eq!(
            crate::ui::nav::recording_words(&ui.state, false),
            "Recording status unknown"
        );
        ui.pointer_move(x, y);
        ui.pointer_down(x, y);
        ui.pointer_up(x, y);
        ui.frame(0.0);
        assert!(ui.state.hex.record_busy, "{phase}");
        ui.update(|state, _| Fixture::finish(state));
        ui.frame(0.0);
        let state = &ui.state;
        let unknown = phase == "timeout";
        let active = unknown || phase == "recording";
        assert_eq!(state.hex.connection.expired(), unknown, "{phase}");
        let words = crate::ui::nav::recording_words(state, false);
        assert_eq!(words == "Recording status unknown", unknown, "{phase}");
        assert_eq!(state.ask_live_meeting(), active, "{phase}");
        assert_eq!(state.hex_stop_pending(), phase == "transcribing");
        assert_eq!(
            ui.elements()
                .iter()
                .any(|e| e.key.as_deref() == Some("hex-stop") && e.focusable),
            active,
            "{phase}"
        );
        assert_eq!(api.data.lock().unwrap().stops, 0, "{phase}");
        if phase == "recording" {
            assert_eq!(recording_id(state).as_deref(), Some("200-2"));
            assert!(words.starts_with("Recording "));
        }
    }
}

#[test]
fn stale_stop_uses_command_budget_with_cached_capabilities() {
    if isolated_api_test("stale_stop_uses_command_budget_with_cached_capabilities") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    api.data.lock().unwrap().delay = Duration::from_millis(1000);
    Fixture::stop(&mut state);
    let (stops, id) = {
        let data = api.data.lock().unwrap();
        (data.stops, data.id)
    };
    assert_eq!(stops, 1, "Stop must survive a status slower than 750 ms");
    assert_eq!(id, None);
    assert!(state.hex.error.is_none());
}

#[test]
fn stop_keeps_the_identity_captured_by_the_rendered_button() {
    if isolated_api_test("stop_keeps_the_identity_captured_by_the_rendered_button") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    let displayed = recording_id(&state);
    api.data.lock().unwrap().id = Some("200-2");
    let newer = command_status_sample(&state);
    state.hex_apply(Msg::ObservedStatus(newer));
    state.hex_record_stop(displayed);
    let (stops, id) = {
        let data = api.data.lock().unwrap();
        (data.stops, data.id)
    };
    assert_eq!(stops, 0);
    assert_eq!(id, Some("200-2"));
    assert!(state.hex.error.unwrap().contains("changed before Stop"));
    assert!(state.hex.error.unwrap().contains("Stop it from Hex"));
}

#[cfg(windows)]
#[test]
fn expired_recording_is_unknown_and_new_meeting_rechecks_then_starts() {
    if isolated_api_test("expired_recording_is_unknown_and_new_meeting_rechecks_then_starts") {
        return;
    }
    let (api, mut state, now) = Fixture::start();
    state.hex_status_at(Err(HexError::Timeout), now + Duration::from_secs(11));
    assert_eq!(
        crate::ui::nav::recording_words(&state, false),
        "Recording status unknown"
    );
    assert!(
        indicator(&state, crate::ui::theme::palette(false))
            .1
            .contains("status unknown")
    );
    assert_eq!(
        start_blocked(&state),
        None,
        "stale observations must not permanently block New meeting"
    );
    assert_eq!(stop_blocked(&state), None);
    {
        let mut data = api.data.lock().unwrap();
        data.id = None;
        data.delay = Duration::from_millis(1000);
    }
    state.new_meeting();
    Fixture::finish(&mut state);
    let starts = api.data.lock().unwrap().starts;
    assert_eq!(
        starts, 1,
        "New meeting must recheck, then send Start after a confirmed idle reply"
    );
    assert!(state.hex.error.is_none());
}

#[test]
fn newer_watcher_status_supersedes_delayed_stop_snapshots() {
    if isolated_api_test("newer_watcher_status_supersedes_delayed_stop_snapshots") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    let old = sample_status(|| state.hex.engine.status());
    api.data.lock().unwrap().meeting_state = Some("transcribing");
    state.hex_begin_stop(0, Instant::now());
    let deadline = state.hex.stop.as_ref().unwrap().deadline.unwrap();
    state.hex_apply(Msg::ObservedStatus(sample_status(|| {
        state.hex.engine.status()
    })));
    for stale in [
        Msg::Done(
            Action::Stop(0, Some(Box::new(old.clone()))),
            Err(HexError::Timeout),
        ),
        Msg::StopStatus(deadline, old),
    ] {
        state.hex_apply(stale);
        assert!(!state.hex_recording());
        assert!(!state.ask_live_meeting());
    }
}

#[test]
fn superseded_stop_refusal_keeps_transcribing_meeting_pending() {
    if isolated_api_test("superseded_stop_refusal_keeps_transcribing_meeting_pending") {
        return;
    }
    use crate::ui::live::Phase;

    let (api, mut state, _) = Fixture::start();
    let part = sidenote::core::meetings_dir().join("100-1");
    std::fs::create_dir_all(&part).unwrap();
    std::fs::write(part.join("manifest.json"), r#"{"status":"recording"}"#).unwrap();
    state.hex_apply(Msg::ObservedStatus(command_status_sample(&state)));
    assert!(state.live.as_ref().unwrap().session.is_some());
    {
        let mut data = api.data.lock().unwrap();
        data.id = None;
        data.meeting_state = Some("transcribing");
    }
    state.meeting_pause();
    assert!(state.hex_stop_pending());

    // Hold the real Stop reply until a later watcher observation has been applied.
    let deadline = Instant::now() + Duration::from_secs(2);
    let refusal = loop {
        let event = state
            .events_rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("Stop refusal");
        if let Event::Hex(msg @ Msg::Done(Action::Stop(..), _)) = event {
            break msg;
        }
    };
    let Msg::Done(Action::Stop(_, Some(preflight)), Err(HexError::Failed { code, .. })) = &refusal
    else {
        panic!("expected Stop refusal with a preflight status");
    };
    assert_eq!(code.as_deref(), Some("recording-changed"));
    assert_eq!(preflight.1.as_ref().unwrap().meeting_state, "transcribing");
    let watcher = sample_status(|| state.hex.engine.status());
    assert!(watcher.0 > preflight.0);
    let watcher_sent = watcher.0;
    state.hex_apply(Msg::ObservedStatus(watcher));
    assert_eq!(state.hex.last_status_request, Some(watcher_sent));
    assert_eq!(state.hex.state().unwrap().meeting_state, "transcribing");
    state.hex_apply(refusal);

    assert!(state.hex_stop_pending());
    assert_eq!(
        recorder_start_block_reason(&state),
        Some("Check that Hex has finished the previous recording.")
    );
    state.meeting_resume();
    state.new_meeting();
    let live = state.live.as_ref().unwrap();
    assert_eq!(live.phase, Phase::Recording);
    assert!(live.session.as_ref().unwrap().pending_start_ms.is_none());
    assert!(!state.hex.record_busy);
    assert_eq!(api.data.lock().unwrap().starts, 0);
    Fixture::stop_status(&mut state);
    assert!(state.hex_stop_pending());
}

#[test]
fn late_status_replies_do_not_renew_recording_freshness() {
    if isolated_api_test("late_status_replies_do_not_renew_recording_freshness") {
        return;
    }
    let (_api, mut state, _) = Fixture::start();
    assert!(state.hex_running());
    let status = state.hex.engine.status_for_command().unwrap();
    let late = (Instant::now() - Duration::from_secs(11), Ok(status));
    state.hex_begin_stop(0, Instant::now());
    let deadline = state.hex.stop.as_ref().unwrap().deadline.unwrap();

    for (source, msg) in [
        ("watcher", Msg::ObservedStatus(late.clone())),
        ("Stop completion", Msg::StopStatus(deadline, late.clone())),
        (
            "Stop preflight",
            Msg::Done(
                Action::Stop(0, Some(Box::new(late))),
                Err(HexError::Unsupported),
            ),
        ),
    ] {
        state.hex_apply(msg);
        assert!(
            state.hex.status.as_ref().is_some_and(Result::is_ok),
            "{source}"
        );
        assert!(state.hex_status_unknown(), "{source}");
        assert!(!state.hex_running(), "{source}");
        state.hex_apply(Msg::ObservedStatus(sample_status(|| {
            Err(HexError::Timeout)
        })));
        assert!(state.hex_status_unknown(), "{source} after timeout");
        assert_eq!(recording_id(&state).as_deref(), Some("100-1"), "{source}");
    }
}

#[test]
fn successful_snapshot_expires_without_another_poll() {
    if isolated_api_test("successful_snapshot_expires_without_another_poll") {
        return;
    }
    let (_api, mut state, _) = Fixture::start();
    let good = state.hex.engine.status_for_command().unwrap();
    state.hex_status_at(Ok(good), Instant::now() - Duration::from_secs(11));
    assert!(state.hex_status_unknown());
    assert!(!state.hex_running());
    assert_eq!(
        crate::ui::nav::recording_words(&state, false),
        "Recording status unknown"
    );
    assert_eq!(recorder_start_block_reason(&state), None);
    assert_eq!(stop_blocked(&state), None);
}

#[cfg(target_os = "linux")]
#[test]
fn new_meeting_refuses_on_linux_without_rechecking_or_replacing_pending_notes() {
    if isolated_api_test(
        "new_meeting_refuses_on_linux_without_rechecking_or_replacing_pending_notes",
    ) {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    let notes = "Synthetic pending notes on Linux";
    let page = pending_notes(&mut state, &api, notes);
    api.data.lock().unwrap().commands.clear();

    let refusal = "Recording is not available on Linux yet.";
    assert_eq!(start_blocked(&state), Some(refusal));
    let mut runtime =
        rune::Runtime::new(state, meeting_control, (500.0, 160.0), rune::Env::default());
    runtime.frame(0.0);
    let elements = runtime.elements();
    let control = elements
        .iter()
        .find(|el| el.key.as_deref() == Some("new-meeting"))
        .unwrap();
    assert_eq!(control.help.as_deref(), Some(refusal));
    assert!(!control.focusable);
    assert!(
        !elements
            .iter()
            .any(|el| el.text.as_deref() == Some(refusal))
    );
    runtime.state.new_meeting();
    let state = &runtime.state;
    let live = state.live.as_ref().unwrap();
    assert!(state.hex_status_unknown());
    assert!(!state.hex_record_busy());
    assert_eq!(live.recording_id, page);
    assert_eq!(live.phase, crate::ui::live::Phase::Making);
    assert_eq!(live.saving, crate::ui::live::Saving::Pending);
    assert_eq!(state.live_notes.text, notes);
    assert!(!sidenote::core::state_dir().join("unsaved-notes").exists());
    assert!(api.data.lock().unwrap().commands.is_empty());
}

#[test]
fn settings_check_uses_the_bounded_status_call_without_starting_a_meeting() {
    if isolated_api_test("settings_check_uses_the_bounded_status_call_without_starting_a_meeting") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    state.hex.settings_status_at = None;
    state.hex_check_settings();
    assert!(state.hex.settings_status_pending);
    while state.hex.settings_status_pending {
        if let Event::Hex(msg) = state
            .events_rx
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
        {
            state.hex_apply(msg);
        }
    }
    assert!(state.hex_running());
    assert!(state.hex.settings_status_at.is_some());
    assert_eq!(api.data.lock().unwrap().starts, 0);
}

fn finished_refresh_view(view: &str) {
    use crate::ui::{Event, refresh};
    use sidenote::core;
    let (api, mut state, _) = Fixture::start();
    let root = core::meetings_dir();
    let now = chrono::Utc::now().timestamp_millis();
    let write = |id: &str, title: &str, started: i64, summary: bool| {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), serde_json::json!({"status":"complete","created_at_ms":started,
            "ended_at_ms":started+60_000,"duration_ms":60_000,"title":"Meeting at 15:14","title_source":"default"}).to_string()).unwrap();
        if summary {
            std::fs::write(
                dir.join("english-summary.json"),
                serde_json::json!({"title":title}).to_string(),
            )
            .unwrap();
        }
    };
    write("10-1", "Earlier workshop", now - 68 * 60_000, true);
    let apply = |state: &mut State| {
        let options = state.refresh_options();
        state.apply_event(Event::Refreshed(refresh::Result {
            generation: core::mutation_generation(),
            options,
            data: Some(Box::new(refresh::Data::read(options))),
        }));
    };
    apply(&mut state);
    state.live = None;
    state.live_open = false;
    match view {
        "meeting" => state.selected = Some("10-1".into()),
        "memory" => {
            state.config.features.memory = true;
            state.memory_open = true;
        }
        "sidebar" | "home" | "list" => state.go_home(),
        _ => panic!("unknown view"),
    }
    let (handle, dirty) = refresh::observing_handle();
    state.scan_tx = Some(handle);
    write("100-1", "Latest workshop", now - 2 * 60_000, false);
    api.data.lock().unwrap().id = None;
    let idle = command_status_sample(&state);
    state.apply_event(Event::Hex(Msg::ObservedStatus(idle)));
    assert!(
        dirty.load(Ordering::Acquire),
        "meeting-finished observation did not invalidate the catalog in {view}"
    );
    apply(&mut state);
    assert_eq!(state.meetings[0].id, "100-1", "{view}");
    assert!(
        state
            .groups
            .iter()
            .flat_map(|g| &g.items)
            .any(|i| state.meetings[i.index].id == "100-1")
    );
    assert_eq!(
        state
            .ask_settings
            .last_live
            .as_ref()
            .unwrap()
            .hint
            .as_deref(),
        Some("100-1")
    );
    write("100-1", "Latest workshop", now - 2 * 60_000, true);
    apply(&mut state);
    assert_eq!(
        state.meetings[0].title, "Latest workshop",
        "notes did not replace the default title in {view}"
    );
    if view == "meeting" {
        assert_eq!(
            state.current().unwrap().title,
            "Earlier workshop",
            "Do not redirect a reader browsing an older meeting"
        );
    }
}

#[test]
fn finished_event_refreshes_meetings_list() {
    if !isolated_api_test("finished_event_refreshes_meetings_list") {
        finished_refresh_view("list");
    }
}
#[test]
fn finished_event_refreshes_home() {
    if !isolated_api_test("finished_event_refreshes_home") {
        finished_refresh_view("home");
    }
}
#[test]
fn finished_event_refreshes_meeting_catalog_without_redirecting_the_reader() {
    if !isolated_api_test("finished_event_refreshes_meeting_catalog_without_redirecting_the_reader")
    {
        finished_refresh_view("meeting");
    }
}
#[test]
fn finished_event_refreshes_memory_catalog() {
    if !isolated_api_test("finished_event_refreshes_memory_catalog") {
        finished_refresh_view("memory");
    }
}
#[test]
fn finished_event_refreshes_sidebar_catalog() {
    if !isolated_api_test("finished_event_refreshes_sidebar_catalog") {
        finished_refresh_view("sidebar");
    }
}
#[test]
fn page_notice_does_not_follow_navigation_to_memory() {
    if isolated_api_test("page_notice_does_not_follow_navigation_to_memory") {
        return;
    }
    let (_api, mut state, _) = Fixture::start();
    state.config.features.memory = true;
    state.notice = Some("The transcript has no line for that time.".into());
    state.open_memory();
    assert!(state.on_memory());
    assert!(state.notice.is_none());
}

#[test]
fn preview_title_replaces_the_default_heading_without_publishing_a_final_summary() {
    if isolated_api_test(
        "preview_title_replaces_the_default_heading_without_publishing_a_final_summary",
    ) {
        return;
    }
    let (_api, _state, _) = Fixture::start();
    let folder = sidenote::core::meetings_dir().join("200-2");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("manifest.json"),
        r#"{"status":"complete","title":"Meeting at 15:14","title_source":"default"}"#,
    )
    .unwrap();
    let drafts = sidenote::core::state_dir().join("local/draft-summaries");
    std::fs::create_dir_all(&drafts).unwrap();
    std::fs::write(
        drafts.join("200-2.json"),
        r#"{"title":"Workshop launch plan","context":["Synthetic preview"]}"#,
    )
    .unwrap();
    let meeting = sidenote::core::Meeting::load_pending(&folder).unwrap();
    assert_eq!(meeting.title, "Workshop launch plan");
    assert!(meeting.summary.title.is_empty());
    assert!(!meeting.has_summary());
    std::fs::write(
        folder.join("english-summary.json"),
        r#"{"title":"Final workshop decisions"}"#,
    )
    .unwrap();
    assert_eq!(
        sidenote::core::Meeting::load(&folder).unwrap().title,
        "Final workshop decisions"
    );
}
#[cfg(windows)]
#[test]
fn meeting_pause_resume_uses_hex_commands_and_keeps_one_meeting() {
    if isolated_api_test("meeting_pause_resume_uses_hex_commands_and_keeps_one_meeting") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    let hex = sidenote::core::meetings_dir();
    for (id, status, duration) in [("100-1", "recording", 60000), ("300-3", "recording", 30000)] {
        let folder = hex.join(id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("manifest.json"),
            serde_json::json!({"status":status,"created_at_ms":100,"duration_ms":duration})
                .to_string(),
        )
        .unwrap();
        std::fs::write(folder.join("transcript.ndjson"),serde_json::json!({"start_ms":0,"end_ms":1000,"text":"Synthetic meeting part","source":"microphone"}).to_string()).unwrap();
    }
    state.live_notes = rune::TextEdit::new("Synthetic personal notes survive Pause.");
    assert!(state.live_close());
    let good = state.hex.engine.status_for_command().unwrap();
    state.hex_status_at(Ok(good), Instant::now());
    state.parts_sync();
    assert_eq!(
        state
            .live
            .as_ref()
            .unwrap()
            .session
            .as_ref()
            .unwrap()
            .parts
            .len(),
        1
    );
    use crate::ui::live::Phase;
    let mut transcribing = state.hex.engine.status_for_command().unwrap();
    transcribing.recording = None;
    transcribing.meeting_state = "transcribing".into();
    api.data.lock().unwrap().stop_state = Some("transcribing");
    state.meeting_pause();
    assert!(state.hex_stop_pending());
    let manifest = hex.join("100-1/manifest.json");
    std::fs::write(
        &manifest,
        r#"{"status":"transcribing","duration_ms":90000}"#,
    )
    .unwrap();
    state.hex_apply(Msg::ObservedStatus(sample_status(|| {
        Ok(transcribing.clone())
    })));
    state.live_sync();
    state.parts_sync();
    state.meeting_pause();
    state.meeting_resume();
    state.meeting_generate();
    state.hex_record_stop(None);
    let live = state.live.as_ref().unwrap();
    assert_eq!(live.phase, Phase::Recording);
    assert_eq!(live.session.as_ref().unwrap().parts[0].duration_ms, 60000);
    Fixture::finish(&mut state);
    Fixture::stop_status(&mut state);
    assert_eq!(api.data.lock().unwrap().stops, 1);
    state.hex_apply(Msg::ObservedStatus(sample_status(|| Ok(transcribing))));
    assert!(state.hex_stop_pending());
    assert!(state.hex_stop_tick(Instant::now() + Duration::from_secs(31), &mut Tx::default()));
    assert!(state.hex_stop_unconfirmed());
    state.parts_sync();
    let live = state.live.as_ref().unwrap();
    assert_eq!(live.session.as_ref().unwrap().parts[0].duration_ms, 60000);
    api.data.lock().unwrap().meeting_state = None;
    assert!(
        !sidenote::core::state_dir()
            .join("local/notes-jobs")
            .exists()
    );
    std::fs::write(&manifest, r#"{"status":"complete","duration_ms":90000}"#).unwrap();
    state.hex_recheck_stop();
    Fixture::stop_status(&mut state);
    let live = state.live.as_ref().unwrap();
    assert_eq!(live.phase, Phase::Paused);
    assert_eq!(live.session.as_ref().unwrap().parts[0].duration_ms, 90000);
    let folder = sidenote::core::meeting_parts::inputs(&sidenote::core::state_dir()).join("100-1");
    let paused = sidenote::core::Meeting::load_pending(&folder).unwrap();
    assert!(
        crate::ui::notes_status::job(&state, &paused).is_none(),
        "paused meeting offered a notes job"
    );
    std::fs::write(
        folder.join("english-summary.json"),
        "{\"title\":\"Synthetic existing notes\"}",
    )
    .unwrap();
    {
        let session = state.live.as_mut().unwrap().session.as_mut().unwrap();
        session.notes_parts = 1;
        session.requested_parts = 1;
        session.save(&sidenote::core::state_dir()).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.md")).unwrap(),
        "Synthetic personal notes survive Pause."
    );
    state.live = None;
    state.hex.parts_restore_checked = false;
    let idle = state.hex.engine.status_for_command().unwrap();
    state.hex_status_at(Ok(idle), Instant::now());
    assert_eq!(
        state.live.as_ref().unwrap().phase,
        crate::ui::live::Phase::Paused
    );
    state.meeting_resume();
    Fixture::finish(&mut state);
    let recording = state.hex.engine.status_for_command().unwrap();
    state.hex_status_at(Ok(recording), Instant::now());
    let session = state.live.as_ref().unwrap().session.as_ref().unwrap();
    assert_eq!(session.id, "100-1");
    assert!(session.stale());
    assert_eq!(
        session
            .parts
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["100-1", "300-3"]
    );
    assert_eq!(api.data.lock().unwrap().starts, 1);
    assert_eq!(
        api.data.lock().unwrap().stops,
        1,
        "restored pause stopped the resumed part"
    );
    assert!(
        !sidenote::core::state_dir()
            .join("local/notes-jobs")
            .exists()
    );
}

#[cfg(windows)]
#[test]
fn recorder_ready_new_meeting_logs_one_correlated_action_through_recheck() {
    if isolated_api_test("recorder_ready_new_meeting_logs_one_correlated_action_through_recheck") {
        return;
    }
    let (api, mut state, _) = Fixture::start_idle();
    let idle = state.hex.engine.status_for_command().unwrap();
    state.hex_status_at(Ok(idle), Instant::now() - Duration::from_secs(11));
    let (release, wait) = std::sync::mpsc::channel();
    api.data.lock().unwrap().start_release = Some(wait);
    state.new_meeting();
    let mut runtime =
        rune::Runtime::new(state, meeting_control, (500.0, 160.0), rune::Env::default());
    runtime.frame(0.0);
    assert!(
        runtime
            .elements()
            .iter()
            .any(|e| e.text.as_deref() == Some("Checking Hex before New meeting"))
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let event = runtime
            .state
            .events_rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        let rechecked = matches!(&event, Event::Hex(Msg::RecheckStart(_)));
        runtime.update(|state, _| state.apply_event(event));
        if rechecked {
            break;
        }
    }
    runtime.frame(0.0);
    assert!(
        runtime
            .elements()
            .iter()
            .any(|e| e.text.as_deref() == Some("Working")),
        "busy={} blocked={:?} error={:?}",
        runtime.state.hex_record_busy(),
        start_blocked(&runtime.state),
        runtime.state.hex.error
    );
    release.send(()).unwrap();
    Fixture::finish(&mut runtime.state);
    assert_eq!(api.data.lock().unwrap().starts, 1);
    let log = std::fs::read_to_string(sidenote::core::log_path()).unwrap();
    let actions: Vec<_> = log
        .lines()
        .filter(|line| line.contains("hex new_meeting action "))
        .collect();
    assert_eq!(actions.len(), 4, "{actions:?}");
    for (line, stage) in actions
        .iter()
        .zip(["entry", "recheck_done", "request_sent", "completed"])
    {
        assert!(
            line.contains(&format!(
                "seq=1 stage={stage} role=ui pid={}",
                std::process::id()
            )),
            "{line}"
        );
    }
    assert!(log.contains("hex new_meeting availability reason="));
    assert!(!log.contains("hex new_meeting blocked"));
}

#[cfg(any(windows, target_os = "linux"))]
fn meeting_control(state: &State, _: &mut rune::Cx<State>) -> rune::El {
    let mut layer = Vec::new();
    let button = crate::ui::nav::new_meeting_button(
        state,
        crate::ui::theme::palette(false),
        false,
        &mut layer,
    );
    rune_kit::root(button, layer)
}

#[cfg(all(windows, feature = "recorder-ready-proof"))]
#[test]
fn recorder_ready_installed_preparation_disables_the_real_control_until_validation_finishes() {
    if isolated_api_test(
        "recorder_ready_installed_preparation_disables_the_real_control_until_validation_finishes",
    ) {
        return;
    }
    readiness::run();
}
