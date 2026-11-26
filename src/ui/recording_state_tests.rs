//! The recording state of the meeting page, through the real page: the pause control, the live
//! dot and the line over the notes. Each test runs in its own process on an isolated data
//! folder, with invented meeting text only. Nothing here talks to Hex.
use super::*;
use rune::{Env, Runtime, text::TextSystem};
use sidenote::core::meeting_parts::{self, Part, Phase as SavedPhase};
use sidenote::hex_engine::{EngineState, ModelState, Recording};

const MEETING: &str = "1800000000000-1";
const OTHER: &str = "1800000000002-2";

/// Relaunches the named test in a child process; true in the parent, which has then checked it.
fn in_parent(name: &str) -> bool {
    const FLAG: &str = "SIDENOTE_RECORDING_STATE_TEST";
    if std::env::var(FLAG).ok().as_deref() == Some(name) {
        return false;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("ui::recording_state_tests::{name}"),
            "--nocapture",
        ])
        .env(FLAG, name);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{name} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

/// The saved-meeting page over two invented meetings. The first is held in memory by a live scene
/// in `phase`, so no press reaches Hex. No parts file is written: a held meeting leaves the list.
fn page(phase: SavedPhase, size: (f32, f32)) -> Runtime<State> {
    crate::test_paths::isolate();
    for id in [MEETING, OTHER] {
        let folder = core::meetings_dir().join(id);
        std::fs::create_dir_all(&folder).unwrap();
        let summary = serde_json::json!({
            "title": "Beta launch plan",
            "context": ["The team met to plan the beta launch."],
            "topics": [{"title": "Launch", "points": [{"type": "decided", "text": "Ship the beta on Friday."}]}],
        });
        std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
    }
    let parts = meeting_parts::Meeting {
        id: MEETING.into(),
        parts: vec![Part {
            id: MEETING.into(),
            started_ms: 1800000000000,
            duration_ms: 724000,
        }],
        phase,
        ..Default::default()
    };
    let mut state = State::new();
    if phase == SavedPhase::Recording {
        let mut live = live::Live::scene(live::Phase::Recording, None);
        live.session = Some(parts);
        state.live = Some(live);
    }
    let mut rt = Runtime::with_text(
        state,
        view,
        size,
        Env::default(),
        TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
    );
    rt.state.selected = Some(MEETING.into());
    rt.state.load_saved();
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn element(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"))
}

fn tap(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.2);
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: rune::Mods::default(),
    });
    rt.frame(0.2);
}

/// Hex says the synthetic meeting records.
fn hex_records(rt: &mut Runtime<State>) {
    rt.update(|s, _| {
        s.hex.status = Some(Ok(EngineState {
            running: true,
            version: "test".into(),
            protocol: 4,
            recording: Some(Recording {
                id: Some(MEETING.into()),
                title: None,
                since: None,
                elapsed_seconds: Some(724),
            }),
            meeting_state: "recording".into(),
            dictating: Some(false),
            capture_state: "idle".into(),
            model: ModelState {
                id: None,
                name: None,
                language: None,
                installed: None,
                preparing: None,
                downloaded_bytes: None,
            },
            labs: Default::default(),
        }));
    });
    rt.frame(0.2);
}

const NOTE: &str = "Recording. These notes update when you pause.";

/// The texts inside the line over the notes; empty when the page has no such line.
fn note_texts(rt: &Runtime<State>) -> Vec<String> {
    let Some(root) = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("meeting-recording-note"))
        .map(|e| e.path)
    else {
        return Vec::new();
    };
    rt.elements()
        .into_iter()
        .filter(|e| e.path.starts_with(&format!("{root}/")))
        .filter_map(|e| e.text)
        .collect()
}

#[test]
fn the_notes_say_they_update_on_pause_only_while_recording() {
    if in_parent("the_notes_say_they_update_on_pause_only_while_recording") {
        return;
    }
    let mut rt = page(SavedPhase::Recording, DEFAULT_SIZE);
    // The line is the first thing in the notes: 16 above the Context heading, in its column.
    assert_eq!(note_texts(&rt), [NOTE]);
    let (note, context) = (
        element(&rt, "meeting-recording-note").rect,
        element(&rt, "sec-context").rect,
    );
    assert_eq!(context[1] - (note[1] + note[3]), 16.0);
    assert_eq!(note[0], context[0]);
    let dot = rt
        .elements()
        .into_iter()
        .find(|e| {
            e.path
                .starts_with(&element(&rt, "meeting-recording-note").path)
                && e.radius == 4.0
        })
        .expect("the live dot");
    assert_eq!(dot.fill, theme::palette(true).danger);
    // Pause finishes: the same page, a moment later, has no line.
    rt.update(|s, _| s.parts_paused());
    rt.frame(0.2);
    assert!(
        rt.find("meeting-recording-note").is_none(),
        "still shown after Pause"
    );
    assert!(rt.find("sec-context").is_some());
    // A meeting that has ended never shows it either.
    rt.update(|s, _| {
        let live = s.live.as_mut().unwrap();
        live.session.as_mut().unwrap().phase = SavedPhase::Ended;
    });
    rt.frame(0.2);
    assert!(
        rt.find("meeting-recording-note").is_none(),
        "shown for an ended meeting"
    );
    // At 390 wide the line stays inside the page.
    let rt = page(SavedPhase::Recording, (390.0, 900.0));
    let note = element(&rt, "meeting-recording-note").rect;
    assert!(note[0] >= 0.0 && note[0] + note[2] <= 390.0, "{note:?}");
}

/// The caption under the meta row (or the live page's header), joined; None when it is absent.
fn caption(rt: &Runtime<State>) -> Option<String> {
    let root = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("meeting-recorded-time"))?
        .path;
    let texts: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| e.path == root || e.path.starts_with(&format!("{root}/")))
        .filter_map(|e| e.text)
        .collect();
    Some(texts.join(" "))
}

#[test]
fn the_recorded_caption_is_hidden_while_recording_and_back_on_pause_on_both_pages() {
    if in_parent("the_recorded_caption_is_hidden_while_recording_and_back_on_pause_on_both_pages") {
        return;
    }
    // The pill already shows the clock, so the meeting page says "recording" in two places only.
    let mut rt = page(SavedPhase::Recording, DEFAULT_SIZE);
    assert!(rt.find("meeting-recording-note").is_some());
    assert_eq!(
        caption(&rt),
        None,
        "caption shown while recording (meeting page)"
    );
    rt.update(|s, _| s.parts_paused());
    rt.frame(0.2);
    let paused = caption(&rt).expect("no caption after Pause (meeting page)");
    assert!(paused.starts_with("Paused \u{b7} "), "{paused}");
    // The live page takes its status from the same function.
    let mut rt = page(SavedPhase::Recording, DEFAULT_SIZE);
    rt.update(|s, _| {
        s.selected = None;
        s.live_open = true;
    });
    rt.frame(0.2);
    assert!(
        rt.find("live-notes-input").is_some(),
        "the live page is not shown"
    );
    assert_eq!(
        caption(&rt),
        None,
        "caption shown while recording (live page)"
    );
    rt.update(|s, _| s.parts_paused());
    rt.frame(0.2);
    let paused = caption(&rt).expect("no caption after Pause (live page)");
    assert!(paused.starts_with("Paused \u{b7} "), "{paused}");
}

#[test]
fn recording_clock_stays_visible_on_the_live_page() {
    if in_parent("recording_clock_stays_visible_on_the_live_page") {
        return;
    }
    let mut rt = page(SavedPhase::Recording, DEFAULT_SIZE);
    // The recording entry point lives in the sidebar "+ New" menu now (old: a top-bar button).
    tap(&mut rt, "nav-new-menu");
    assert!(rt.find("new-meeting").is_some());
    tap(&mut rt, "nav-new-menu");
    hex_records(&mut rt);
    assert_eq!(element(&rt, "top-bar").rect[3], 48.0);
    assert!(rt.find("new-meeting").is_none());
    assert!(rt.find("ask-pill").is_some());
    assert!(rt.find("meeting-pause").is_some());
    assert!(
        rt.find("recording-open").is_none(),
        "the recording meeting still has the top-right recording pill"
    );

    rt.update(|s, _| s.meeting_ui.tabs = true);
    rt.frame(0.2);
    assert!(rt.find("recording-open").is_some());

    rt.update(|s, _| {
        s.meeting_ui.tabs = false;
        s.selected = Some(OTHER.into());
        s.load_saved();
    });
    rt.frame(0.2);
    assert!(rt.find("recording-pill").is_some());
    assert_eq!(&element(&rt, "hex-pause").rect[2..], &[24.0, 24.0]);
    assert!(rt.find("hex-stop").is_none());

    rt.update(|s, _| s.open_live());
    rt.frame(0.2);
    assert!(rt.find("meeting-pause").is_some());
    let clock = element(&rt, "recording-open");
    assert_eq!(
        clock.label.as_deref(),
        Some("Recording 12:04. Open the live meeting")
    );
    assert!(
        rt.find("recording-pill").is_none(),
        "the live page shows a second recording action"
    );
    rt.update(|s, _| {
        s.go_home();
        s.set_sidebar(false);
        s.config.features.people = true;
        s.config.features.memory = true;
        s.config.features.daily_check = true;
        s.config.features.annotations = true;
    });
    rt.frame(0.2);
    assert!(rt.find("nav-meetings").is_some());
    let toggles = rt
        .elements()
        .into_iter()
        .filter(|e| e.key.as_deref() == Some("sidebar-toggle"))
        .count();
    assert_eq!(toggles, 1);
    assert_eq!(element(&rt, "nav-search").fill, rune::Color::TRANSPARENT);
    #[cfg(windows)]
    assert!(element(&rt, "open-deleted-meetings").rect[1] >= 48.0);
    tap(&mut rt, "sidebar-toggle");
    rt.update(|s, _| s.hex.status = Some(Err(sidenote::hex_engine::HexError::Timeout)));
    rt.frame(0.2);
    // The menu holds the entry point, so it must be opened before the row shows.
    tap(&mut rt, "nav-new-menu");
    assert!(rt.find("new-meeting").is_some());
    assert!(rt.find("search").is_none());
    for place in [
        "nav-meetings",
        "view-tasks",
        "nav-people",
        "view-memory",
        "nav-annotations",
        "view-check",
    ] {
        assert!(rt.find(place).is_some(), "missing {place}");
    }
    // Shut the menu, then fold the sidebar with the new top-row control.
    tap(&mut rt, "nav-new-menu");
    tap(&mut rt, "nav-collapse");
    assert!(rt.find("nav-meetings").is_none());
}

#[test]
fn the_live_dot_in_the_top_right_is_danger_red_not_amber() {
    if in_parent("the_live_dot_in_the_top_right_is_danger_red_not_amber") {
        return;
    }
    let mut rt = page(SavedPhase::Recording, DEFAULT_SIZE);
    hex_records(&mut rt);
    rt.update(|s, _| {
        s.selected = Some(OTHER.into());
        s.load_saved();
    });
    rt.frame(0.2);
    let dot = element(&rt, "recording-live-dot");
    assert_eq!((dot.rect[2], dot.rect[3]), (8.0, 8.0));
    assert!(dot.path.starts_with(&element(&rt, "recording-open").path));
    let p = theme::palette(true);
    assert_eq!(dot.fill, p.danger);
    assert_ne!(dot.fill, p.warn);
    for (dark, classic_red, olive_red) in
        [(true, "#f29b99", "#f4a19a"), (false, "#a92b30", "#b42318")]
    {
        let classic = theme::classic_palette(dark);
        let olive = theme::palette(dark);
        assert_eq!(classic.danger, rune::color::hex(classic_red).unwrap());
        assert_eq!(olive.danger, rune::color::hex(olive_red).unwrap());
        assert_ne!(olive.danger, olive.warn);
    }
}

#[test]
fn pause_icon_is_distinct_from_stop_and_works_by_mouse_and_keyboard() {
    if in_parent("pause_icon_is_distinct_from_stop_and_works_by_mouse_and_keyboard") {
        return;
    }
    let mut rt = page(SavedPhase::Recording, DEFAULT_SIZE);
    hex_records(&mut rt);
    let pause = element(&rt, "meeting-pause");
    assert_eq!((pause.rect[2], pause.rect[3]), (32.0, 32.0));
    assert_eq!(
        (pause.role, pause.label.as_deref(), pause.help.as_deref()),
        ("button", Some("Pause recording"), Some("Pause recording"))
    );
    let dot = element(&rt, "dock-recording-dot");
    assert!(dot.path.starts_with(&pause.path));
    assert_eq!(dot.fill, theme::palette(true).danger);
    rt.update(|s, _| s.hex.status = None);
    rt.frame(0.0);
    let unavailable = element(&rt, "meeting-pause");
    assert_eq!(unavailable.label.as_deref(), Some("Pause recording"));
    assert_eq!(
        unavailable.help.as_deref(),
        Some(
            "Pause recording
Hex is not running. Stop it from Hex if it is recording."
        )
    );
    assert!(!unavailable.focusable);
    tap(&mut rt, "meeting-pause");
    assert!(
        rt.state.hex.error.is_none(),
        "disabled Pause handled a click"
    );
    hex_records(&mut rt);
    rt.update(|s, _| {
        s.selected = Some(OTHER.into());
        s.load_saved();
    });
    rt.frame(0.2);
    let managed = element(&rt, "hex-pause");
    assert!(managed.label.as_deref().unwrap().starts_with("Pause"));
    assert!(rt.find("hex-stop").is_none());
    assert!(rt.find("meeting-pause").is_none() && rt.find("dock-recording-dot").is_none());

    rt.update(|s, _| {
        let Some(Ok(status)) = &mut s.hex.status else {
            unreachable!()
        };
        status.recording.as_mut().unwrap().id = Some(OTHER.into());
    });
    rt.frame(0.2);
    assert!(rt.find("hex-pause").is_none());
    let stop = element(&rt, "hex-stop");
    let stop_name = stop.label.unwrap();
    assert!(stop_name.starts_with("Stop"), "{stop_name}");
    assert!(!stop_name.contains("Pause"), "{stop_name}");
    assert!(rt.find("meeting-pause").is_none() && rt.find("dock-recording-dot").is_none());
    hex_records(&mut rt);
    rt.update(|s, _| {
        s.selected = Some(MEETING.into());
        s.load_saved();
    });
    rt.frame(0.2);
    // Assert the command's capability check so the test needs no running Hex process.
    rt.update(|s, _| s.hex_apply(hex::Msg::Caps("test".into(), Ok(Default::default()))));
    let unsupported = Some("This Hex build cannot stop. Stop it from Hex.");
    assert!(rt.state.hex.error.is_none());
    tap(&mut rt, "meeting-pause");
    assert_eq!(
        rt.state.hex.error, unsupported,
        "mouse press did not reach Pause"
    );
    // Keyboard: the button takes focus with a visible ring and Space presses it. Enter presses
    // only rows and links in the toolkit, so it is not part of the contract for a button.
    rt.update(|s, _| s.hex.error = None);
    rt.update(|_, tx| tx.focus("meeting-pause", true));
    rt.frame(0.0);
    let pause = element(&rt, "meeting-pause");
    assert!(pause.focusable && pause.focused && pause.focus_visible);
    press(&mut rt, Key::Text(" ".into()));
    assert_eq!(rt.state.hex.error, unsupported, "Space did not reach Pause");
}
