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

fn view_as(rt: &mut Runtime<State>, size: (f32, f32), dark: bool) {
    rt.set_viewport(size, 1.0);
    rt.set_system_dark(dark);
    rt.settle_all();
    rt.frame(0.0);
}

fn both_views(rt: &mut Runtime<State>, check: impl Fn(&Runtime<State>)) {
    for size in [WIDE, PHONE] {
        for dark in [false, true] {
            view_as(rt, size, dark);
            check(rt);
        }
    }
}

fn tap(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.2);
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    rt.key(KeyEvent {
        key: Key::Text(text.into()),
        mods: rune::Mods::default(),
    });
    rt.frame(0.2);
}

fn open_new_menu(rt: &mut Runtime<State>) {
    if rt.find("nav-new-menu").is_none() {
        tap(rt, "sidebar-toggle");
    }
    if rt.find("new-meeting").is_none() {
        tap(rt, "nav-new-menu");
    }
}

fn click_new(rt: &mut Runtime<State>) {
    open_new_menu(rt);
    tap(rt, "new-meeting");
}

fn fixture_runtime(size: (f32, f32), dark: bool) -> (Fixture, Runtime<State>) {
    let (api, mut state, _) = Fixture::start_idle();
    crate::ui::updates::settings_fixture(&mut state);
    let listener = ListenerStatus {
        ready: true,
        ..Default::default()
    };
    state.listener_probed(Some(listener), None);
    let idle = command_status_sample(&state);
    state.hex_apply(Msg::ObservedStatus(idle));
    (api, runtime(state, size, dark))
}

fn finish_busy(rt: &mut Runtime<State>) {
    while rt.state.hex.record_busy {
        let event = rt
            .state
            .events_rx
            .recv_timeout(Duration::from_secs(15))
            .expect("Hex command completion");
        rt.state.apply_event(event);
    }
    rt.frame(0.2);
}

fn observed(rt: &mut Runtime<State>) {
    loop {
        let event = rt
            .state
            .events_rx
            .recv_timeout(Duration::from_secs(15))
            .expect("Hex status after the command reply");
        let hit = matches!(&event, Event::Hex(Msg::ObservedStatus(_)));
        rt.state.apply_event(event);
        if hit {
            break;
        }
    }
    rt.frame(0.2);
}

fn deliver_status(rt: &mut Runtime<State>) {
    let sample = command_status_sample(&rt.state);
    rt.state.hex_apply(Msg::ObservedStatus(sample));
    rt.frame(0.2);
}

fn wait_starts(api: &Fixture, expected: usize) {
    let until = Instant::now() + Duration::from_secs(2);
    while api.data.lock().unwrap().starts < expected {
        assert!(Instant::now() < until, "meeting-record did not arrive");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_file(path: &std::path::Path, text: &str) {
    let until = Instant::now() + Duration::from_secs(2);
    while std::fs::read_to_string(path).ok().as_deref() != Some(text) {
        assert!(Instant::now() < until, "notes were not saved at {path:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
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

fn assert_no_running_controls(rt: &Runtime<State>) {
    for key in [
        "meeting-pause",
        "meeting-resume",
        "meeting-generate",
        "hex-pause",
        "hex-stop",
    ] {
        assert!(rt.find(key).is_none(), "{key} shown before recording");
    }
    assert_no_finish_words(rt);
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
    assert_no_running_controls(rt);
}

fn set_start_fixture(
    api: &Fixture,
    release: std::sync::mpsc::Receiver<()>,
    failure: bool,
    records: bool,
    state: &'static str,
) {
    let mut data = api.data.lock().unwrap();
    data.start_failure = failure;
    data.start_records = records;
    data.meeting_state = Some(state);
    data.start_release = Some(release);
}

#[test]
fn record_new_meeting_waits_for_running_status() {
    if isolated_api_test("start::record_new_meeting_waits_for_running_status") {
        return;
    }
    let (api, mut rt) = fixture_runtime(PHONE, true);
    let (release, wait) = mpsc::channel();
    set_start_fixture(&api, wait, false, true, "starting");
    click_new(&mut rt);
    wait_starts(&api, 1);
    assert_starting(&rt);
    tap(&mut rt, "live-notes-input");
    type_text(&mut rt, "Notes before confirmation");
    assert_eq!(rt.state.live_notes.text, "Notes before confirmation");
    release.send(()).unwrap();
    finish_busy(&mut rt);
    observed(&mut rt);
    both_views(&mut rt, assert_starting);
    view_as(&mut rt, WIDE, false);
    open_new_menu(&mut rt);
    tap(&mut rt, "new-meeting");
    assert_eq!(api.data.lock().unwrap().starts, 1);
    tap(&mut rt, "nav-new-menu");
    api.data.lock().unwrap().meeting_state = Some("recording");
    deliver_status(&mut rt);
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Recording);
    both_views(&mut rt, |rt| {
        assert!(rt.find("meeting-pause").is_some());
    });
    tap(&mut rt, "meeting-pause");
    finish_busy(&mut rt);
    assert_eq!(api.data.lock().unwrap().stops, 1);
}

#[test]
fn starting_status_does_not_offer_titlebar_pause_after_navigation() {
    if isolated_api_test("start::starting_status_does_not_offer_titlebar_pause_after_navigation") {
        return;
    }
    let (api, mut rt) = fixture_runtime(WIDE, false);
    let (release, wait) = mpsc::channel();
    set_start_fixture(&api, wait, false, false, "idle");
    click_new(&mut rt);
    wait_starts(&api, 1);
    release.send(()).unwrap();
    finish_busy(&mut rt);
    observed(&mut rt);
    assert_starting(&rt);
    {
        let mut data = api.data.lock().unwrap();
        data.id = Some("300-3");
        data.meeting_state = Some("starting");
    }
    deliver_status(&mut rt);
    both_views(&mut rt, assert_starting);
    view_as(&mut rt, WIDE, false);
    tap(&mut rt, "nav-meetings");
    assert!(rt.find("recording-open").is_some(), "live link missing");
    assert!(rt.find("hex-pause").is_none(), "starting offers Pause");
    assert!(rt.find("hex-stop").is_none());
    assert_no_finish_words(&rt);
    tap(&mut rt, "recording-open");
    assert!(rt.find("live-notes-input").is_some(), "live did not reopen");
    api.data.lock().unwrap().meeting_state = Some("recording");
    deliver_status(&mut rt);
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Recording);
    tap(&mut rt, "nav-meetings");
    assert!(rt.find("hex-pause").is_some());
    assert!(rt.find("hex-stop").is_none(), "owned recording shows Stop");
    api.data.lock().unwrap().id = Some("400-4");
    deliver_status(&mut rt);
    assert!(rt.find("hex-pause").is_none());
    assert!(rt.find("hex-stop").is_some());
}

#[test]
fn failed_start_and_resume_keep_notes_and_recovery_controls() {
    if isolated_api_test("start::failed_start_and_resume_keep_notes_and_recovery_controls") {
        return;
    }
    let (api, mut rt) = fixture_runtime(WIDE, false);
    let (release, wait) = mpsc::channel();
    set_start_fixture(&api, wait, true, false, "idle");
    click_new(&mut rt);
    wait_starts(&api, 1);
    tap(&mut rt, "live-notes-input");
    type_text(&mut rt, "Notes kept after a failed start");
    let first = rt
        .state
        .live
        .as_ref()
        .unwrap()
        .session
        .as_ref()
        .unwrap()
        .id
        .clone();
    let root = sidenote::core::state_dir();
    let notes = meeting_parts::inputs(&root).join(&first).join("notes.md");
    release.send(()).unwrap();
    finish_busy(&mut rt);
    assert!(rt.state.hex.error.is_some());
    assert_eq!(rt.state.live_notes.text, "Notes kept after a failed start");
    assert_no_running_controls(&rt);
    both_views(&mut rt, assert_no_running_controls);
    view_as(&mut rt, WIDE, false);
    wait_file(&notes, "Notes kept after a failed start");
    rt.state.live_tick();
    rt.frame(0.2);
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
    {
        let mut data = api.data.lock().unwrap();
        data.start_failure = false;
        data.start_records = true;
        data.meeting_state = Some("recording");
    }
    click_new(&mut rt);
    wait_starts(&api, 2);
    finish_busy(&mut rt);
    observed(&mut rt);
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Recording);
    assert!(rt.find("meeting-pause").is_some());
    rt.state.hex_status_at(
        Err(HexError::Timeout),
        Instant::now() - Duration::from_secs(11),
    );
    rt.frame(0.2);
    assert!(rt.state.hex_status_unknown());
    assert!(rt.find("meeting-pause").is_some());
    tap(&mut rt, "nav-meetings");
    assert!(rt.find("hex-pause").is_some());
    tap(&mut rt, "recording-open");
    assert!(rt.find("meeting-pause").is_some(), "live did not reopen");
    tap(&mut rt, "meeting-pause");
    finish_control(&mut rt.state);
    rt.frame(0.2);
    assert_eq!(rt.state.live.as_ref().unwrap().phase, Phase::Paused);
    assert!(rt.find("meeting-resume").is_some());
    let session = rt.state.live.as_ref().unwrap().session.as_ref().unwrap();
    assert!(!session.parts.is_empty(), "pause erased existing parts");
    both_views(&mut rt, |rt| {
        assert!(rt.find("meeting-resume").is_some());
    });
}
