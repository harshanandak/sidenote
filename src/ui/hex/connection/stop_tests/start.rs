use super::*;
use crate::ui::{live::Phase, theme, view};
use rune::{Env, Key, KeyEvent, Runtime, text::TextSystem};
use sidenote::{core::meeting_parts, updates::ListenerStatus};
const WIDE: (f32, f32) = (1334.0, 900.0);
const PHONE: (f32, f32) = (390.0, 844.0);
fn runtime(state: State, size: (f32, f32), dark: bool) -> Runtime<State> {
    let mut rt = Runtime::with_text(
        state,
        view,
        size,
        Env::default(),
        TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
    );
    rt.set_system_dark(dark);
    rt.settle_all();
    rt.frame(0.0);
    rt
}
fn tap(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.2);
}
fn element(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"))
}
fn type_text(rt: &mut Runtime<State>, text: &str) {
    rt.key(KeyEvent {
        key: Key::Text(text.into()),
        mods: rune::Mods::default(),
    });
    rt.frame(0.2);
}
fn click_new(rt: &mut Runtime<State>) {
    if rt.find("nav-new-menu").is_none() {
        tap(rt, "sidebar-toggle");
    }
    if rt.find("new-meeting").is_none() {
        tap(rt, "nav-new-menu");
    }
    tap(rt, "new-meeting");
}
fn dismiss_new_menu(rt: &mut Runtime<State>) {
    rt.key(Key::Escape.into());
    rt.frame(0.2);
    assert!(rt.find("new-meeting").is_none());
    if rt.state.drawer_open.get() {
        tap(rt, "sidebar-toggle");
    }
    assert!(!rt.state.drawer_open.get());
}
fn fixture_runtime() -> (Fixture, Runtime<State>) {
    fixture_runtime_at(WIDE, false)
}
fn fixture_runtime_at(size: (f32, f32), dark: bool) -> (Fixture, Runtime<State>) {
    let (api, mut state, _) = Fixture::start_idle();
    crate::ui::updates::settings_fixture(&mut state);
    state.listener_probed(
        Some(ListenerStatus {
            ready: true,
            ..Default::default()
        }),
        None,
    );
    let idle = command_status_sample(&state);
    state.hex_apply(Msg::ObservedStatus(idle));
    (api, runtime(state, size, dark))
}
fn finish_busy(rt: &mut Runtime<State>) {
    let until = Instant::now() + Duration::from_secs(15);
    while rt.state.hex_record_busy() {
        let event = rt
            .state
            .events_rx
            .recv_timeout(until.saturating_duration_since(Instant::now()))
            .expect("Hex command completion");
        rt.update(|s, _| s.apply_event(event));
    }
    rt.frame(0.2);
}
fn observed(rt: &mut Runtime<State>) {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        let event = rt
            .state
            .events_rx
            .recv_timeout(until.saturating_duration_since(Instant::now()))
            .expect("Hex status after the command reply");
        let hit = matches!(&event, Event::Hex(Msg::ObservedStatus(_)));
        rt.update(|s, _| s.apply_event(event));
        if hit {
            break;
        }
    }
    rt.frame(0.2);
}
fn deliver_status(rt: &mut Runtime<State>) {
    let sample = command_status_sample(&rt.state);
    rt.update(|s, _| s.hex_apply(Msg::ObservedStatus(sample)));
    rt.frame(0.2);
}
fn begin_idle(rt: &mut Runtime<State>) {
    click_new(rt);
    finish_busy(rt);
    observed(rt);
    deliver_status(rt);
}
fn write_notes(rt: &mut Runtime<State>, text: &str) {
    tap(rt, "live-notes-input");
    type_text(rt, text);
    assert_eq!(rt.state.live_notes.text, text);
}
fn no_running_controls(rt: &Runtime<State>) {
    for key in [
        "meeting-pause",
        "meeting-resume",
        "meeting-generate",
        "hex-pause",
        "hex-stop",
    ] {
        assert!(rt.find(key).is_none(), "{key} shown before ownership");
    }
}
fn start_fixture(api: &Fixture, state: &'static str, id: Option<&'static str>) {
    let mut data = api.data.lock().unwrap();
    data.start_records = false;
    data.meeting_state = Some(state);
    data.id = id;
}
fn session(rt: &Runtime<State>) -> meeting_parts::Meeting {
    rt.state
        .live
        .as_ref()
        .and_then(|live| live.session.clone())
        .expect("meeting session")
}
fn expire_pending(rt: &mut Runtime<State>) {
    rt.update(|s, _| {
        let m = s
            .live
            .as_mut()
            .and_then(|live| live.session.as_mut())
            .unwrap();
        m.pending_start_ms = Some(chrono::Utc::now().timestamp_millis() - 26_000);
    });
}
#[test]
fn recording_without_id_keeps_safety_stop() {
    if isolated_api_test("start::recording_without_id_keeps_safety_stop") {
        return;
    }
    let (api, mut rt) = fixture_runtime();
    start_fixture(&api, "idle", None);
    begin_idle(&mut rt);
    let first = session(&rt).id;
    rt.update(|s, _| s.new_meeting());
    finish_busy(&mut rt);
    assert_eq!(api.data.lock().unwrap().starts, 1);
    assert_eq!(session(&rt).id, first);
    rt.update(|s, _| s.go_meetings());
    rt.frame(0.2);
    assert!(element(&rt, "recording-open").focusable);
    assert!(rt.find("hex-stop").is_none());
    tap(&mut rt, "recording-open");
    write_notes(&mut rt, "notes before an idless recording");
    start_fixture(&api, "starting", None);
    deliver_status(&mut rt);
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Starting);
    no_running_controls(&rt);
    start_fixture(&api, "recording", None);
    expire_pending(&mut rt);
    deliver_status(&mut rt);
    assert_eq!(session(&rt).id, first);
    assert!(session(&rt).pending_start_ms.is_some());
    assert!(element(&rt, "hex-stop").focusable);
    tap(&mut rt, "hex-stop");
    finish_busy(&mut rt);
    api.assert_stops(1);
    assert_eq!(rt.state.live_notes.text, "notes before an idless recording");
}
#[test]
fn confirmed_recording_without_timing_attaches_after_invalid_id() {
    if isolated_api_test("start::confirmed_recording_without_timing_attaches_after_invalid_id") {
        return;
    }
    let (api, mut rt) = fixture_runtime();
    start_fixture(&api, "idle", None);
    begin_idle(&mut rt);
    write_notes(&mut rt, "exact notes without recorder timing");
    let before = session(&rt);
    let pending = before.pending_start_ms.unwrap();
    {
        let mut data = api.data.lock().unwrap();
        data.meeting_state = Some("recording");
        data.id = Some("bad");
        data.omit_elapsed = true;
    }
    deliver_status(&mut rt);
    assert!(session(&rt).pending_start_ms.is_some());
    assert!(session(&rt).parts.is_empty());
    assert!(element(&rt, "hex-stop").focusable);
    api.data.lock().unwrap().id = Some("300-3");
    deliver_status(&mut rt);
    let after = session(&rt);
    assert_eq!(after.id, before.id);
    assert_eq!(after.pending_start_ms, None);
    assert_eq!(after.parts.len(), 1);
    assert_eq!(
        (after.parts[0].id.as_str(), after.parts[0].started_ms),
        ("300-3", pending)
    );
    assert_eq!(
        rt.state.live_notes.text,
        "exact notes without recorder timing"
    );
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Recording);
    assert!(element(&rt, "meeting-pause").focusable);
    rt.update(|s, _| s.go_meetings());
    rt.frame(0.2);
    assert!(element(&rt, "hex-pause").focusable);
    assert!(rt.find("hex-stop").is_none());
}
#[test]
fn pending_resume_expires_after_fresh_idle() {
    if isolated_api_test("start::pending_resume_expires_after_fresh_idle") {
        return;
    }
    let (api, mut rt) = fixture_runtime();
    api.data.lock().unwrap().start_records = false;
    let root = sidenote::core::state_dir();
    let now = chrono::Utc::now().timestamp_millis();
    let mut saved = meeting_parts::Meeting::pending(&root, now - 60_000).unwrap();
    saved.resume(&root, "100-1", now - 60_000).unwrap();
    saved.pause(&root).unwrap();
    let folder = meeting_parts::inputs(&root).join(&saved.id);
    sidenote::core::live::write_notes(&folder, "old exact notes").unwrap();
    let meeting_id = saved.id.clone();
    rt.update(|s, _| {
        s.selected = Some(meeting_id.clone());
        s.meeting_resume();
        s.open_live();
    });
    finish_busy(&mut rt);
    observed(&mut rt);
    deliver_status(&mut rt);
    assert!(session(&rt).pending_start_ms.is_some());
    expire_pending(&mut rt);
    deliver_status(&mut rt);
    let after = session(&rt);
    assert_eq!(after.id, meeting_id);
    assert_eq!(after.pending_start_ms, None);
    assert_eq!(after.parts.len(), 1);
    assert_eq!(after.parts[0].id, "100-1");
    assert_eq!(rt.state.live_notes.text, "old exact notes");
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Paused);
    assert!(element(&rt, "meeting-resume").focusable);
    assert!(element(&rt, "meeting-generate").focusable);
    assert_eq!(api.data.lock().unwrap().starts, 1);
}
#[test]
fn late_start_error_preserves_attached_recording_and_notes() {
    if isolated_api_test("start::late_start_error_preserves_attached_recording_and_notes") {
        return;
    }
    let (api, mut rt) = fixture_runtime();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    {
        let mut data = api.data.lock().unwrap();
        data.start_failure = true;
        data.start_release = Some(release_rx);
        data.meeting_state = Some("starting");
        data.id = Some("300-3");
    }
    click_new(&mut rt);
    deliver_status(&mut rt);
    write_notes(&mut rt, "notes before the late error");
    let before = session(&rt).id;
    api.data.lock().unwrap().meeting_state = Some("recording");
    deliver_status(&mut rt);
    release_tx.send(()).unwrap();
    finish_busy(&mut rt);
    deliver_status(&mut rt);
    let after = session(&rt);
    assert_eq!(after.id, before);
    assert_eq!(after.parts.len(), 1);
    assert_eq!(after.parts[0].id, "300-3");
    assert_eq!(rt.state.live_notes.text, "notes before the late error");
    assert!(element(&rt, "meeting-pause").focusable);
}

fn meetings_pointer(rt: &mut Runtime<State>) {
    if rt.find("nav-meetings").is_none() {
        tap(rt, "sidebar-toggle");
    }
    tap(rt, "nav-meetings");
}

fn assert_editor_notes(rt: &mut Runtime<State>, text: &str) {
    assert_eq!(rt.state.live_notes.text, text);
    tap(rt, "live-notes-input");
    let editor = element(rt, "live-notes-input");
    assert!(editor.focusable && editor.focused);
    assert!(rt.find("ask-pill").is_some());
}

fn assert_ask_usable(rt: &mut Runtime<State>) {
    let [x, y, w, h] = element(rt, "ask-pill").rect;
    let [left, top, width, height] = element(rt, "ask-overlay").rect;
    assert!(w > 0.0 && h > 0.0 && x >= left && y >= top);
    assert!(x + w <= left + width + 0.5 && y + h <= top + height + 0.5);
    tap(rt, "ask-input");
    assert!(element(rt, "ask-input").focused);
}

fn reopen_pending(rt: &mut Runtime<State>, meeting_id: &str, notes: &str) {
    meetings_pointer(rt);
    assert!(!rt.state.on_live());
    assert!(element(rt, "recording-open").focusable);
    no_running_controls(rt);
    tap(rt, "recording-open");
    assert!(rt.state.on_live());
    let resumed = session(rt);
    assert_eq!(resumed.id, meeting_id);
    assert!(resumed.pending_start_ms.is_some());
    assert_editor_notes(rt, notes);
}

fn pending_idle_navigation(api: &Fixture, rt: &mut Runtime<State>) {
    const NOTES: &str = "Notes across both pending idle returns";
    rt.update(|s, _| crate::ui::scene_agents(s, crate::ui::ALL_AGENTS));
    rt.frame(0.2);
    let release = hold_start(api, false, false, "idle");
    click_new(rt);
    wait_starts(api, 1);
    let meeting_id = session(rt).id;
    write_notes(rt, NOTES);
    reopen_pending(rt, &meeting_id, NOTES);
    assert!(rt.state.hex.record_busy && !rt.state.hex_recording());
    assert_ask_usable(rt);

    release.send(()).unwrap();
    finish_busy(rt);
    observed(rt);
    assert!(rt.state.hex_stop_confirmed());
    reopen_pending(rt, &meeting_id, NOTES);
    assert_ask_usable(rt);
    assert_editor_notes(rt, NOTES);

    click_new(rt);
    finish_busy(rt);
    dismiss_new_menu(rt);
    assert_eq!(api.data.lock().unwrap().starts, 1);
    assert_eq!(session(rt).id, meeting_id);
}

fn no_id_variant(api: &Fixture, rt: &mut Runtime<State>, id: Option<&'static str>, omit: bool) {
    start_fixture(api, "idle", None);
    begin_idle(rt);
    let first = session(rt).id;
    tap(rt, "nav-search");
    tap(rt, "finder-row-0");
    click_new(rt);
    finish_busy(rt);
    assert_eq!(api.data.lock().unwrap().starts, 1);
    assert_eq!(session(rt).id, first);
    dismiss_new_menu(rt);
    write_notes(rt, "matrix no-id notes");
    start_fixture(api, "starting", None);
    deliver_status(rt);
    no_running_controls(rt);
    start_fixture(api, "recording", id);
    api.data.lock().unwrap().omit_id = omit;
    expire_pending(rt);
    deliver_status(rt);
    assert!(session(rt).pending_start_ms.is_some());
    assert!(element(rt, "hex-stop").focusable);
    let expired = rt.state.hex.status.clone().unwrap().unwrap();
    rt.update(|s, _| {
        s.hex_status_at(Ok(expired), Instant::now() - Duration::from_secs(11));
    });
    rt.frame(0.2);
    assert!(session(rt).pending_start_ms.is_some());
    assert!(element(rt, "hex-stop").focusable);
    meetings_pointer(rt);
    assert!(element(rt, "hex-stop").focusable);
    tap(rt, "hex-stop");
    finish_busy(rt);
    api.assert_stops(1);
    assert!(rt.state.hex_stop_confirmed());
    assert!(rt.state.live.is_some());
    assert_eq!(rt.state.live_notes.text, "matrix no-id notes");
}

fn timing_matrix(api: &Fixture, rt: &mut Runtime<State>) {
    start_fixture(api, "idle", None);
    begin_idle(rt);
    write_notes(rt, "matrix timing notes");
    let before = session(rt);
    let pending = before.pending_start_ms.unwrap();
    {
        let mut data = api.data.lock().unwrap();
        data.meeting_state = Some("recording");
        data.id = Some("bad");
        data.omit_elapsed = true;
    }
    deliver_status(rt);
    assert!(session(rt).parts.is_empty());
    assert!(element(rt, "hex-stop").focusable);
    api.data.lock().unwrap().id = Some("300-3");
    deliver_status(rt);
    let after = session(rt);
    assert_eq!(after.id, before.id);
    assert_eq!(after.pending_start_ms, None);
    assert_eq!(
        (after.parts[0].id.as_str(), after.parts[0].started_ms),
        ("300-3", pending)
    );
    assert_editor_notes(rt, "matrix timing notes");
    assert!(element(rt, "meeting-pause").focusable);
    meetings_pointer(rt);
    assert!(element(rt, "hex-pause").focusable);
    tap(rt, "hex-pause");
    finish_busy(rt);
    api.assert_stops(1);
    assert!(rt.state.hex_stop_confirmed());
    tap(rt, "recording-open");
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Paused);
    let paused = session(rt);
    assert_eq!(paused.id, before.id);
    assert_eq!(paused.parts.len(), 1);
    assert_eq!(paused.parts[0].id, "300-3");
    assert_editor_notes(rt, "matrix timing notes");
    assert!(element(rt, "meeting-resume").focusable);
    assert!(element(rt, "meeting-generate").focusable);
}

fn restore_paused(rt: &mut Runtime<State>) -> String {
    let root = sidenote::core::state_dir();
    let now = chrono::Utc::now().timestamp_millis();
    let mut saved = meeting_parts::Meeting::pending(&root, now - 60_000).unwrap();
    saved.resume(&root, "100-1", now - 60_000).unwrap();
    saved.pause(&root).unwrap();
    sidenote::core::live::write_notes(
        &meeting_parts::inputs(&root).join(&saved.id),
        "matrix old notes",
    )
    .unwrap();
    let hex_part = sidenote::core::meetings_dir().join("100-1");
    std::fs::create_dir_all(&hex_part).unwrap();
    std::fs::write(
        hex_part.join("manifest.json"),
        r#"{"status":"complete","duration_ms":1000}"#,
    )
    .unwrap();
    rt.update(|s, _| {
        s.hex.parts_restore_checked = false;
        s.live_sync();
    });
    rt.frame(0.2);
    assert_editor_notes(rt, "matrix old notes");
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Paused);
    assert_eq!(session(rt).id, saved.id);
    saved.id
}

fn resume_matrix(api: &Fixture, rt: &mut Runtime<State>) {
    let meeting_id = restore_paused(rt);
    let release = hold_start(api, false, false, "idle");
    tap(rt, "meeting-resume");
    wait_starts(api, 1);
    assert!(session(rt).pending_start_ms.is_some());
    release.send(()).unwrap();
    finish_busy(rt);
    observed(rt);
    deliver_status(rt);
    expire_pending(rt);
    deliver_status(rt);
    let after = session(rt);
    assert_eq!(
        (after.id.as_str(), after.pending_start_ms, after.parts.len()),
        (meeting_id.as_str(), None, 1)
    );
    assert_eq!(after.parts[0].id, "100-1");
    assert_editor_notes(rt, "matrix old notes");
    assert!(element(rt, "meeting-resume").focusable);
    assert!(element(rt, "meeting-generate").focusable);
    assert_eq!(api.data.lock().unwrap().starts, 1);
}

fn resume_attaches_new_part(api: &Fixture, rt: &mut Runtime<State>) {
    let meeting_id = restore_paused(rt);
    let release = hold_start(api, false, false, "idle");
    tap(rt, "meeting-resume");
    wait_starts(api, 1);
    start_fixture(api, "starting", Some("300-3"));
    deliver_status(rt);
    assert!(rt.state.hex.record_busy);
    assert_eq!(session(rt).parts.len(), 1);
    no_running_controls(rt);
    reopen_pending(rt, &meeting_id, "matrix old notes");
    no_running_controls(rt);

    start_fixture(api, "recording", Some("300-3"));
    deliver_status(rt);
    assert!(rt.state.hex.record_busy);
    let attached = session(rt);
    assert_eq!(attached.id, meeting_id);
    assert_eq!(attached.pending_start_ms, None);
    assert_eq!(attached.parts.len(), 2);
    assert_eq!(attached.parts[0].id, "100-1");
    assert_eq!(attached.parts[1].id, "300-3");

    release.send(()).unwrap();
    finish_busy(rt);
    observed(rt);
    assert_eq!(api.data.lock().unwrap().starts, 1);
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Recording);
    assert_editor_notes(rt, "matrix old notes");
    assert!(element(rt, "meeting-pause").focusable);
    meetings_pointer(rt);
    assert!(element(rt, "hex-pause").focusable);
    assert!(rt.find("hex-stop").is_none());
}

fn late_error_matrix(api: &Fixture, rt: &mut Runtime<State>, recording_before_error: bool) {
    let release_tx = hold_start(api, true, true, "starting");
    api.data.lock().unwrap().id = Some("300-3");
    click_new(rt);
    wait_starts(api, 1);
    deliver_status(rt);
    write_notes(rt, "matrix late-error notes");
    let before = session(rt).id;
    if recording_before_error {
        start_fixture(api, "recording", Some("300-3"));
        deliver_status(rt);
    }
    release_tx.send(()).unwrap();
    finish_busy(rt);
    assert!(rt.state.hex.error.is_some());
    deliver_status(rt);
    if !recording_before_error {
        let pending = session(rt);
        assert_eq!(pending.id, before);
        assert!(pending.pending_start_ms.is_some());
        assert!(pending.parts.is_empty());
        assert_starting(rt);
        assert_editor_notes(rt, "matrix late-error notes");
        start_fixture(api, "recording", Some("300-3"));
        deliver_status(rt);
    }
    let after = session(rt);
    assert_eq!(
        (
            after.id.as_str(),
            after.parts.len(),
            after.parts[0].id.as_str()
        ),
        (before.as_str(), 1, "300-3")
    );
    assert_eq!(after.pending_start_ms, None);
    assert_editor_notes(rt, "matrix late-error notes");
    assert!(element(rt, "meeting-pause").focusable);
    rt.update(|s, _| s.live_flush());
    let root = sidenote::core::state_dir();
    let notes = meeting_parts::inputs(&root)
        .join(&after.id)
        .join("notes.md");
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::fs::read_to_string(&notes).ok().as_deref() != Some("matrix late-error notes") {
        assert!(Instant::now() < deadline, "late-error notes were not saved");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_starts(api: &Fixture, expected: usize) {
    let until = Instant::now() + Duration::from_secs(2);
    while api.data.lock().unwrap().starts < expected {
        assert!(Instant::now() < until, "meeting-record did not arrive");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_retirement(rt: &mut Runtime<State>, path: &std::path::Path) {
    let until = Instant::now() + Duration::from_secs(2);
    while rt.state.live.is_some() {
        assert!(
            Instant::now() < until,
            "failed draft with notes at {path:?} was not retired"
        );
        rt.update_if(|s, _| s.live_tick());
        std::thread::sleep(Duration::from_millis(10));
    }
    rt.frame(0.2);
}

fn assert_no_finish_words(rt: &Runtime<State>) {
    for element in rt.elements() {
        for value in [element.key, element.text, element.label, element.help]
            .into_iter()
            .flatten()
        {
            assert!(!value.contains("Finish"));
        }
    }
}

fn assert_starting(rt: &Runtime<State>) {
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Starting);
    assert!(rt.find("live-notes-input").is_some());
    assert!(
        rt.elements().iter().any(
            |e| e.text.as_deref() == Some("Starting the recording. You can start writing now.")
        ),
        "current Starting copy is missing"
    );
    no_running_controls(rt);
    assert_no_finish_words(rt);
}

fn hold_start(
    api: &Fixture,
    failure: bool,
    records: bool,
    state: &'static str,
) -> mpsc::Sender<()> {
    let (release, wait) = mpsc::channel();
    let mut data = api.data.lock().unwrap();
    data.start_failure = failure;
    data.start_records = records;
    data.meeting_state = Some(state);
    data.start_release = Some(wait);
    release
}

fn record_new_meeting_waits_for_running_status(api: &Fixture, rt: &mut Runtime<State>) {
    let release = hold_start(api, false, true, "starting");
    click_new(rt);
    wait_starts(api, 1);
    assert_starting(rt);
    write_notes(rt, "Notes before confirmation");
    release.send(()).unwrap();
    finish_busy(rt);
    observed(rt);
    assert_starting(rt);
    click_new(rt);
    finish_busy(rt);
    assert_eq!(api.data.lock().unwrap().starts, 1);
    dismiss_new_menu(rt);
    api.data.lock().unwrap().meeting_state = Some("recording");
    deliver_status(rt);
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Recording);
    assert!(rt.find("meeting-pause").is_some());
    tap(rt, "meeting-pause");
    finish_busy(rt);
    assert_eq!(api.data.lock().unwrap().stops, 1);
}

fn starting_titlebar_controls(api: &Fixture, rt: &mut Runtime<State>) {
    let release = hold_start(api, false, false, "idle");
    click_new(rt);
    wait_starts(api, 1);
    release.send(()).unwrap();
    finish_busy(rt);
    observed(rt);
    assert_starting(rt);
    {
        let mut data = api.data.lock().unwrap();
        data.id = Some("300-3");
        data.meeting_state = Some("starting");
    }
    deliver_status(rt);
    assert_starting(rt);
    meetings_pointer(rt);
    assert!(rt.find("recording-open").is_some(), "live link missing");
    assert!(rt.find("hex-pause").is_none(), "starting offers Pause");
    assert!(rt.find("hex-stop").is_none());
    assert_no_finish_words(rt);
    tap(rt, "recording-open");
    assert!(rt.find("live-notes-input").is_some(), "live did not reopen");
    api.data.lock().unwrap().meeting_state = Some("recording");
    deliver_status(rt);
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Recording);
    meetings_pointer(rt);
    assert!(rt.find("hex-pause").is_some());
    assert!(rt.find("hex-stop").is_none(), "owned recording shows Stop");
    api.data.lock().unwrap().id = Some("400-4");
    deliver_status(rt);
    assert!(rt.find("hex-pause").is_none());
    assert!(rt.find("hex-stop").is_some());
}

fn error_idle_matrix(api: &Fixture, rt: &mut Runtime<State>) {
    let release = hold_start(api, true, false, "idle");
    click_new(rt);
    wait_starts(api, 1);
    tap(rt, "live-notes-input");
    type_text(rt, "Notes kept after a failed start");
    let first = session(rt).id;
    let root = sidenote::core::state_dir();
    let notes = meeting_parts::inputs(&root).join(&first).join("notes.md");
    release.send(()).unwrap();
    finish_busy(rt);
    assert!(rt.state.hex.error.is_some());
    deliver_status(rt);
    assert_eq!(rt.state.live_notes.text, "Notes kept after a failed start");
    no_running_controls(rt);
    assert_no_finish_words(rt);
    wait_for_retirement(rt, &notes);
    assert_eq!(
        std::fs::read_to_string(&notes).unwrap(),
        "Notes kept after a failed start"
    );
    assert!(rt.state.live.is_none(), "failed unstarted meeting remains");
    assert!(
        meeting_parts::list(&root)
            .unwrap()
            .iter()
            .all(|m| m.pending_start_ms.is_none()),
        "a duplicate pending session remains"
    );
    assert_eq!(api.data.lock().unwrap().starts, 1);
}

fn failed_start_retry_keeps_controls(api: &Fixture, rt: &mut Runtime<State>) {
    error_idle_matrix(api, rt);
    {
        let mut data = api.data.lock().unwrap();
        data.start_failure = false;
        data.start_records = true;
        data.meeting_state = Some("recording");
    }
    click_new(rt);
    wait_starts(api, 2);
    finish_busy(rt);
    observed(rt);
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Recording);
    let attached = session(rt);
    assert_eq!(attached.pending_start_ms, None);
    assert_eq!(attached.parts[0].id, "300-3");
    assert!(rt.find("meeting-pause").is_some());
    rt.update(|s, _| {
        s.hex_status_at(
            Err(HexError::Timeout),
            Instant::now() - Duration::from_secs(11),
        );
    });
    rt.frame(0.2);
    assert!(rt.state.hex_status_unknown());
    assert!(rt.find("meeting-pause").is_some());
    meetings_pointer(rt);
    assert!(rt.find("hex-pause").is_some());
    tap(rt, "recording-open");
    assert!(rt.find("meeting-pause").is_some(), "live did not reopen");
    tap(rt, "meeting-pause");
    finish_busy(rt);
    assert!(!rt.state.hex_stop_pending());
    assert!(!rt.state.hex_recording());
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Paused);
    assert!(rt.find("meeting-resume").is_some());
    let session = rt.state.live.as_ref().unwrap().session.as_ref().unwrap();
    assert!(!session.parts.is_empty(), "pause erased existing parts");
}

macro_rules! matrix_case {
    ($module:ident, $name:ident, $size:expr, $dark:expr, $body:ident $(, $arg:expr)*) => {
        #[test]
        fn $name() {
            if isolated_api_test(concat!("start::", stringify!($module), "::", stringify!($name))) {
                return;
            }
            let (api, mut rt) = fixture_runtime_at($size, $dark);
            super::$body(&api, &mut rt $(, $arg)*);
        }
    };
}

macro_rules! matrix_cases {
    ($module:ident, $body:ident $(, $arg:expr)*) => {
        mod $module {
            use super::*;
            matrix_case!($module, wide_light, WIDE, false, $body $(, $arg)*);
            matrix_case!($module, wide_dark, WIDE, true, $body $(, $arg)*);
            matrix_case!($module, phone_light, PHONE, false, $body $(, $arg)*);
            matrix_case!($module, phone_dark, PHONE, true, $body $(, $arg)*);
        }
    };
}

matrix_cases!(no_id_null, no_id_variant, None, false);
matrix_cases!(no_id_omitted, no_id_variant, None, true);
matrix_cases!(no_id_empty, no_id_variant, Some(""), false);
matrix_cases!(timing, timing_matrix);
matrix_cases!(resume, resume_matrix);
matrix_cases!(resume_attaches, resume_attaches_new_part);
matrix_cases!(pending_idle, pending_idle_navigation);
matrix_cases!(recording_then_error, late_error_matrix, true);
matrix_cases!(starting_then_error, late_error_matrix, false);
matrix_cases!(error_idle, error_idle_matrix);
matrix_cases!(record_new, record_new_meeting_waits_for_running_status);
matrix_cases!(starting_status, starting_titlebar_controls);
matrix_cases!(failed_start_and_resume, failed_start_retry_keeps_controls);
