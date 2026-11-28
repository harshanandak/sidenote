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

fn assert_editor_notes(rt: &Runtime<State>, text: &str) {
    assert_eq!(rt.state.live_notes.text, text);
    assert!(element(rt, "live-notes-input").focusable);
    assert!(rt.find("ask-pill").is_some());
}

fn no_id_matrix(api: &Fixture, rt: &mut Runtime<State>) {
    no_id_variant(api, rt, None, false);
}
fn no_id_omitted(api: &Fixture, rt: &mut Runtime<State>) {
    no_id_variant(api, rt, None, true);
}
fn no_id_empty(api: &Fixture, rt: &mut Runtime<State>) {
    no_id_variant(api, rt, Some(""), false);
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
    observed(rt);
    api.assert_stops(1);
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
    observed(rt);
    api.assert_stops(1);
    tap(rt, "recording-open");
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Paused);
    assert_editor_notes(rt, "matrix timing notes");
    assert!(element(rt, "meeting-resume").focusable);
    assert!(element(rt, "meeting-generate").focusable);
}

fn resume_matrix(api: &Fixture, rt: &mut Runtime<State>) {
    api.data.lock().unwrap().start_records = false;
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
    let meeting_id = saved.id.clone();
    rt.update(|s, _| {
        s.hex.parts_restore_checked = false;
        s.live_sync();
    });
    rt.frame(0.2);
    assert_editor_notes(rt, "matrix old notes");
    tap(rt, "meeting-resume");
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
    assert_editor_notes(rt, "matrix old notes");
    assert!(element(rt, "meeting-resume").focusable);
    assert!(element(rt, "meeting-generate").focusable);
}

fn late_error_matrix(api: &Fixture, rt: &mut Runtime<State>) {
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    {
        let mut data = api.data.lock().unwrap();
        data.start_failure = true;
        data.start_release = Some(release_rx);
        data.meeting_state = Some("starting");
        data.id = Some("300-3");
    }
    click_new(rt);
    deliver_status(rt);
    write_notes(rt, "matrix late-error notes");
    let before = session(rt).id;
    api.data.lock().unwrap().meeting_state = Some("recording");
    deliver_status(rt);
    release_tx.send(()).unwrap();
    finish_busy(rt);
    observed(rt);
    let after = session(rt);
    assert_eq!(
        (
            after.id.as_str(),
            after.parts.len(),
            after.parts[0].id.as_str()
        ),
        (before.as_str(), 1, "300-3")
    );
    assert_editor_notes(rt, "matrix late-error notes");
    assert!(element(rt, "meeting-pause").focusable);
}

macro_rules! matrix_case {
    ($name:ident, $body:ident, $size:expr, $dark:expr) => {
        #[test]
        fn $name() {
            if isolated_api_test(concat!("start::", stringify!($name))) {
                return;
            }
            let (api, mut rt) = fixture_runtime_at($size, $dark);
            $body(&api, &mut rt);
        }
    };
}
matrix_case!(no_id_null_wide_light, no_id_matrix, WIDE, false);
matrix_case!(no_id_null_wide_dark, no_id_matrix, WIDE, true);
matrix_case!(no_id_null_phone_light, no_id_matrix, PHONE, false);
matrix_case!(no_id_null_phone_dark, no_id_matrix, PHONE, true);
matrix_case!(no_id_omitted_wide_light, no_id_omitted, WIDE, false);
matrix_case!(no_id_omitted_wide_dark, no_id_omitted, WIDE, true);
matrix_case!(no_id_omitted_phone_light, no_id_omitted, PHONE, false);
matrix_case!(no_id_omitted_phone_dark, no_id_omitted, PHONE, true);
matrix_case!(no_id_empty_wide_light, no_id_empty, WIDE, false);
matrix_case!(no_id_empty_wide_dark, no_id_empty, WIDE, true);
matrix_case!(no_id_empty_phone_light, no_id_empty, PHONE, false);
matrix_case!(no_id_empty_phone_dark, no_id_empty, PHONE, true);
matrix_case!(timing_wide_light, timing_matrix, WIDE, false);
matrix_case!(timing_wide_dark, timing_matrix, WIDE, true);
matrix_case!(timing_phone_light, timing_matrix, PHONE, false);
matrix_case!(timing_phone_dark, timing_matrix, PHONE, true);
matrix_case!(resume_wide_light, resume_matrix, WIDE, false);
matrix_case!(resume_wide_dark, resume_matrix, WIDE, true);
matrix_case!(resume_phone_light, resume_matrix, PHONE, false);
matrix_case!(resume_phone_dark, resume_matrix, PHONE, true);
matrix_case!(late_error_wide_light, late_error_matrix, WIDE, false);
matrix_case!(late_error_wide_dark, late_error_matrix, WIDE, true);
matrix_case!(late_error_phone_light, late_error_matrix, PHONE, false);
matrix_case!(late_error_phone_dark, late_error_matrix, PHONE, true);

fn error_idle_matrix(api: &Fixture, rt: &mut Runtime<State>) {
    {
        let mut data = api.data.lock().unwrap();
        data.start_failure = true;
        data.start_records = false;
        data.meeting_state = Some("idle");
    }
    click_new(rt);
    finish_busy(rt);
    observed(rt);
    deliver_status(rt);
    assert!(rt.state.live.is_none());
    assert_eq!(api.data.lock().unwrap().starts, 1);
}
matrix_case!(error_idle_wide_light, error_idle_matrix, WIDE, false);
matrix_case!(error_idle_wide_dark, error_idle_matrix, WIDE, true);
matrix_case!(error_idle_phone_light, error_idle_matrix, PHONE, false);
matrix_case!(error_idle_phone_dark, error_idle_matrix, PHONE, true);
