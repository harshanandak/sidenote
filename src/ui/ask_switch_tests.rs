//! The Ask answer label and the selectable summary, through the real pages. Each test runs in
//! its own process on an isolated data folder, with invented meeting text only.
use super::*;
use rune::{Env, Runtime, text::TextSystem};

const MEETING: &str = "1800000000000-1";
const CONTEXT: &str = "The team met to plan the beta launch.";
const POINT: &str = "Ship the beta on Friday.";

/// Relaunches the named test in a child process; true in the parent, which has then checked it.
fn in_parent(name: &str) -> bool {
    const FLAG: &str = "SIDENOTE_ASK_V3_TEST";
    if std::env::var(FLAG).ok().as_deref() == Some(name) {
        return false;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("ui::ask_switch_tests::{name}"),
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

/// The meeting page over one invented meeting that has a structured summary.
fn page() -> Runtime<State> {
    page_at(DEFAULT_SIZE)
}

fn page_at(size: (f32, f32)) -> Runtime<State> {
    page_with(size, || {})
}

/// `page_at`, with `stored` run after the meeting folder is written and before the app reads its
/// meetings, so a test can leave other records beside it.
fn page_with(size: (f32, f32), stored: impl FnOnce()) -> Runtime<State> {
    crate::test_paths::isolate();
    let folder = core::meetings_dir().join(MEETING);
    std::fs::create_dir_all(&folder).unwrap();
    let summary = serde_json::json!({
        "title": "Beta launch plan",
        "context": [CONTEXT],
        "topics": [{"title": "Launch", "points": [{"type": "decided", "text": POINT}]}],
    });
    std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
    stored();
    let mut state = State::new();
    state.ai_notes.jobs = notes_status::read(&core::state_dir(), &state.meetings);
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

#[test]
fn a_context_line_and_a_point_can_be_selected_and_copied() {
    if in_parent("a_context_line_and_a_point_can_be_selected_and_copied") {
        return;
    }
    let mut rt = page();
    for (key, len, want) in [
        ("sec-context", 7, "Context"),
        ("ctx-0", CONTEXT.len(), CONTEXT),
        ("meeting-point-text-pt-0-0", POINT.len(), POINT),
    ] {
        rt.update(|_, tx| tx.select_text(key, 0..len));
        rt.frame(0.0);
        assert_eq!(rt.selected_text().as_deref(), Some(want), "{key}");
    }
}

fn page_with_cited_point() -> Runtime<State> {
    page_with_cited_point_at(DEFAULT_SIZE)
}

fn page_with_cited_point_at(size: (f32, f32)) -> Runtime<State> {
    page_with(size, || {
        let folder = core::meetings_dir().join(MEETING);
        let summary = serde_json::json!({
            "title": "Beta launch plan",
            "topics": [{"title": "Launch", "points": [{
                "type": "decided", "text": POINT, "at": ["unknown", "00:27"]
            }]}],
        });
        std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
        std::fs::write(
            folder.join("english-transcript.md"),
            "[00:00:06.000] Maya (English): Ready?\n[00:00:27.000] Tom (English): Ship the beta on Friday.\n",
        )
        .unwrap();
    })
}

#[test]
fn kb_copy_rows_keep_mono_for_paths_and_commands() {
    if in_parent("kb_copy_rows_keep_mono_for_paths_and_commands") {
        return;
    }
    const VALUES: [&str; 2] = ["docs/v1/api_keys.md", "git show HEAD:docs/v1/api_keys.md"];
    fn fixture(s: &State, _: &mut rune::Cx<State>) -> rune::El {
        let p = theme::palette(false);
        col().children(VALUES.into_iter().enumerate().flat_map(|(i, value)| {
            [
                check::source_row(s, value, format!("copy-{i}"), p),
                rune::text(value)
                    .font(roles::META.style().mono())
                    .self_start(),
            ]
        }))
    }
    crate::test_paths::isolate();
    let mut rt = Runtime::with_text(
        State::new(),
        fixture,
        (640.0, 240.0),
        Env::default(),
        TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
    );
    rt.frame(0.0);
    let elements = rt.elements();
    for value in VALUES {
        let widths = elements
            .iter()
            .filter(|element| element.text.as_deref() == Some(value))
            .map(|element| element.rect[2])
            .collect::<Vec<_>>();
        assert_eq!(widths.len(), 2, "{value}");
        assert!((widths[0] - widths[1]).abs() < 0.5, "{value}: {widths:?}");
    }
}

#[test]
fn point_timestamp_uses_its_own_figures_and_stays_clickable() {
    if in_parent("point_timestamp_uses_its_own_figures_and_stays_clickable") {
        return;
    }
    const PROSE: &str = "Review on 1 December";
    fn fixture(_: &State, _: &mut rune::Cx<State>) -> rune::El {
        let p = theme::palette(false);
        col().children(
            [
                ("plain", None),
                ("early", Some("11:11")),
                ("late", Some("22:22")),
                ("narrow", Some("11:11")),
            ]
            .into_iter()
            .map(|(key, at)| {
                let point = core::Point {
                    text: PROSE.into(),
                    at: at.into_iter().map(str::to_string).collect(),
                    ..Default::default()
                };
                let row = widgets::point_row(&point, key.into(), None, false, p);
                if key == "narrow" { row.w(150.0) } else { row }
            }),
        )
    }
    let state = page_with(DEFAULT_SIZE, || {
        std::fs::write(
            core::meetings_dir()
                .join(MEETING)
                .join("english-transcript.md"),
            "[00:11:11.000] Maya (English): Review the date.\n",
        )
        .unwrap();
    })
    .state;
    let mut rt = Runtime::with_text(
        state,
        fixture,
        (480.0, 240.0),
        Env::default(),
        TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
    );
    rt.frame(0.0);
    let elements = rt.elements();
    let prose = elements
        .iter()
        .filter(|element| element.text.as_deref() == Some(PROSE))
        .collect::<Vec<_>>();
    assert_eq!(
        prose.len(),
        4,
        "each point renders prose in its own element"
    );
    for element in &prose[..3] {
        assert!((element.rect[2] - prose[0].rect[2]).abs() < 0.5);
    }
    let time = |key| {
        elements
            .iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap()
            .rect
    };
    let early = time("early-time");
    let late = time("late-time");
    assert!((early[2] - late[2]).abs() < 0.5);
    assert!(early[0] >= prose[1].rect[0] + prose[1].rect[2]);
    assert!(early[1] < prose[1].rect[1] + prose[1].rect[3]);
    let narrow = time("narrow-time");
    assert!(narrow[1] >= prose[3].rect[1] + prose[3].rect[3]);
    assert!(narrow[0] + narrow[2] <= 150.0);
    tap(&mut rt, "early-time");
    assert_eq!(rt.state.highlight, Some(0));
    assert_eq!(rt.state.back.as_deref(), Some("early"));
}

#[test]
fn meeting_source_time_reveals_on_hover_and_focus() {
    if in_parent("meeting_source_time_reveals_on_hover_and_focus") {
        return;
    }
    let mut rt = page_with_cited_point();
    let source = "meeting-source-pt-0-0";
    let row = "meeting-point-pt-0-0";
    assert_eq!(rt.presented(source).unwrap().1, 0.0);
    let (x, y) = rt.find("meeting-point-text-pt-0-0").unwrap();
    rt.pointer_move(x, y);
    rt.frame(0.2);
    assert_source_revealed(&rt, source, row);
    rt.pointer_leave();
    rt.frame(0.2);
    assert_eq!(rt.presented(source).unwrap().1, 0.0);

    rt.update(|_, tx| tx.focus(source, true));
    rt.frame(0.2);
    assert_source_revealed(&rt, source, row);
}

#[test]
fn meeting_source_click_opens_the_cited_transcript_line() {
    if in_parent("meeting_source_click_opens_the_cited_transcript_line") {
        return;
    }
    let mut rt = page_with_cited_point();
    let (x, y) = rt.find("meeting-point-text-pt-0-0").unwrap();
    rt.pointer_move(x, y);
    rt.frame(0.2);
    tap(&mut rt, "meeting-source-pt-0-0");
    assert!(rt.state.transcript_open);
    assert_eq!(rt.state.highlight, Some(1));
    assert!(rt.find("line-1").is_some());
}

fn assert_source_revealed(rt: &Runtime<State>, source: &str, row: &str) {
    let elements = rt.elements();
    let time = elements
        .iter()
        .find(|e| e.key.as_deref() == Some(source))
        .unwrap();
    assert_eq!(time.opacity, 1.0);
    assert_eq!(time.label.as_deref(), Some("Open source at 0:27"));
    let row = elements
        .iter()
        .find(|e| e.key.as_deref() == Some(row))
        .unwrap();
    assert_eq!(row.hover, 1.0);
}

/// A summary with every point kind, in an order that is not the display order.
fn page_with_every_kind() -> Runtime<State> {
    page_with(DEFAULT_SIZE, || {
        let folder = core::meetings_dir().join(MEETING);
        let summary = serde_json::json!({
            "title": "Beta launch plan",
            "context": [CONTEXT, "Two questions stayed open."],
            "topics": [
                {"title": "Launch", "points": [
                    {"type": "fact", "text": "Copy is with Priya.", "at": ["00:27"]},
                    {"type": "open", "text": "Whether to ask for the work email later."},
                    {"type": "decided", "text": POINT},
                    {"type": "response", "text": "Priya will send it Thursday."},
                    {"type": "decided", "text": "Hold the review on Monday."},
                ]},
                {"title": "Survey", "points": [{"type": "fact", "text": "Four in ten skip it."}]},
            ],
            "verify": ["The sample is 48 people."],
        });
        std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
    })
}

fn rect_of(rt: &Runtime<State>, key: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"))
        .rect
}

#[test]
fn meeting_notes_body_follows_the_spacing_and_bullet_rules() {
    if in_parent("meeting_notes_body_follows_the_spacing_and_bullet_rules") {
        return;
    }
    let rt = page_with_every_kind();
    let row = |i: usize| rect_of(&rt, &format!("meeting-point-pt-0-{i}"));
    // Bullets sit 27 px apart: a 23 px line plus 2 px of padding above and below.
    let mut ys: Vec<f32> = (0..5).map(|i| row(i)[1]).collect();
    ys.sort_by(f32::total_cmp);
    for i in 1..5 {
        assert_eq!(ys[i] - ys[i - 1], 27.0, "row {i}");
        assert_eq!(row(i)[3], 27.0, "row {i}");
    }
    // Every heading is one role: the same height, 8 px above its first row, 28 px below the
    // last row of the section before.
    let context = rect_of(&rt, "sec-context");
    let launch = rect_of(&rt, "topic-1");
    let survey = rect_of(&rt, "topic-2");
    let verify = rect_of(&rt, "sec-verify");
    assert!(
        [launch[3], survey[3], verify[3]]
            .iter()
            .all(|h| *h == context[3])
    );
    assert_eq!(ys[0] - (launch[1] + launch[3]), 8.0);
    assert_eq!(
        launch[1] - (rect_of(&rt, "ctx-0")[1] + rect_of(&rt, "ctx-0")[3]),
        28.0
    );
    assert_eq!(survey[1] - (ys[4] + 27.0), 28.0);
    // Context is one paragraph: both lines are in ctx-0, and no second block exists.
    assert!(rt.find("ctx-1").is_none());
    let context = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("ctx-0"))
        .and_then(|e| e.text);
    assert_eq!(
        context.as_deref(),
        Some(&*format!("{CONTEXT} Two questions stayed open."))
    );
    // Bullets are 5 px, 11 px in from the row edge; the text starts 26 px in (46 for a reply).
    // A decision has an ink dot, an open point a ring (no fill), the rest a grey dot.
    let elements = rt.elements();
    let by_key = |key: &str| {
        elements
            .iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("missing {key}"))
    };
    for (i, text_x) in [(0, 26.0), (1, 26.0), (2, 26.0), (3, 46.0), (4, 26.0)] {
        let (row, dot) = (row(i), by_key(&format!("meeting-bullet-pt-0-{i}")));
        let text = rect_of(&rt, &format!("meeting-point-text-pt-0-{i}"));
        assert_eq!((dot.rect[2], dot.rect[3]), (5.0, 5.0), "dot {i}");
        let dx = if i == 3 { 20.0 } else { 0.0 };
        assert_eq!(dot.rect[0] - row[0], 11.0 + dx, "dot x {i}");
        assert_eq!(text[0] - row[0], text_x, "text x {i}");
        assert_eq!(
            dot.fill.a == 0.0,
            i == 1,
            "only the open point is a ring: {i}"
        );
        // First line is 23 px tall, so its centre is 2 + 11.5 below the row top.
        assert_eq!(dot.rect[1] + 2.5 - row[1], 13.5, "dot y {i}");
    }
    let fill = |i: usize| by_key(&format!("meeting-bullet-pt-0-{i}")).fill;
    assert_eq!(fill(2), fill(4), "decision dots share one colour");
    assert_eq!(fill(0), fill(3), "fact and reply dots share one colour");
    assert_ne!(fill(0), fill(2), "a decision dot differs from a fact dot");
}

/// The summary indexes of the first topic's `count` rows, top to bottom.
fn shown_order(rt: &Runtime<State>, count: usize) -> Vec<usize> {
    let mut shown: Vec<(f32, usize)> = (0..count)
        .map(|i| (rect_of(rt, &format!("meeting-point-pt-0-{i}"))[1], i))
        .collect();
    shown.sort_by(|a, b| a.0.total_cmp(&b.0));
    shown.into_iter().map(|(_, i)| i).collect()
}

#[test]
fn meeting_replies_stay_under_the_row_they_answer() {
    if in_parent("meeting_replies_stay_under_the_row_they_answer") {
        return;
    }
    // Fact, decision and a chain of two replies, open point and its reply. Sorting decisions first
    // must move each reply with its row, not leave it behind among the facts.
    let rt = page_with(DEFAULT_SIZE, || {
        let folder = core::meetings_dir().join(MEETING);
        let summary = serde_json::json!({
            "title": "Beta launch plan",
            "topics": [{"title": "Launch", "points": [
                {"type": "fact", "text": "Copy is with Priya."},
                {"type": "decided", "text": POINT},
                {"type": "response", "text": "Priya will send it Thursday."},
                {"type": "response", "text": "Maya will check it first."},
                {"type": "open", "text": "Whether to ask for the work email later."},
                {"type": "response", "text": "Tom thinks it can wait."},
            ]}],
        });
        std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
    });
    assert_eq!(shown_order(&rt, 6), [1, 2, 3, 0, 4, 5]);
}

#[test]
fn meeting_notes_lead_each_topic_with_its_decisions() {
    if in_parent("meeting_notes_lead_each_topic_with_its_decisions") {
        return;
    }
    let rt = page_with_every_kind();
    // Summary order is fact, open, decided, response, decided: shown as the two decisions
    // (summary rows 2 and 4) with the reply (3) kept under the first, then the fact (0), then
    // the open point (1).
    assert_eq!(shown_order(&rt, 5), [2, 3, 4, 0, 1]);
    // The bold lead block is the decisions only.
    let elements = rt.elements();
    let ink = |i: usize| {
        let key = format!("meeting-point-text-pt-0-{i}");
        elements
            .iter()
            .find(|e| e.key.as_deref() == Some(&key))
            .unwrap()
            .text_color
    };
    assert_eq!(ink(2), ink(4));
    for plain in [0, 1, 3] {
        assert_ne!(
            ink(plain),
            ink(2),
            "only decisions use the ink colour: {plain}"
        );
    }
    // A topic with no decisions keeps the summary's order.
    assert!(rect_of(&rt, "meeting-point-pt-1-0")[1] > rect_of(&rt, "topic-2")[1]);
}

#[test]
fn meeting_source_time_works_by_keyboard_in_the_new_rows() {
    if in_parent("meeting_source_time_works_by_keyboard_in_the_new_rows") {
        return;
    }
    let mut rt = page_with_cited_point();
    // The pill is hidden, 21 px high, 6 px in from the row's right edge, and reachable by Tab.
    let (row, pill) = (
        rect_of(&rt, "meeting-point-pt-0-0"),
        rect_of(&rt, "meeting-source-pt-0-0"),
    );
    assert_eq!(pill[3], 21.0);
    assert_eq!(row[0] + row[2] - (pill[0] + pill[2]), 6.0);
    rt.update(|_, tx| tx.focus("meeting-source-pt-0-0", true));
    rt.frame(0.2);
    rt.key(KeyEvent {
        key: Key::Text(" ".into()),
        mods: rune::Mods::default(),
    });
    rt.frame(0.2);
    assert!(rt.state.transcript_open);
    assert_eq!(rt.state.highlight, Some(1));
}

#[test]
fn meeting_model_hint_reveals_on_hover_and_keyboard_focus() {
    if in_parent("meeting_model_hint_reveals_on_hover_and_keyboard_focus") {
        return;
    }
    let mut rt = page();
    rt.update(|s, _| {
        scene_agents(s, ALL_AGENTS);
        s.ask_open = true;
    });
    rt.frame(0.0);
    let hint = "ask-model";
    let tip = "help-hint-model-tip";
    assert_eq!(rt.presented(tip).unwrap().1, 0.0);
    let (x, y) = rt.find(hint).unwrap();
    rt.pointer_move(x, y);
    rt.frame(0.2);
    assert_eq!(rt.presented(tip).unwrap().1, 1.0);
    rt.pointer_leave();
    rt.frame(0.2);
    assert_eq!(rt.presented(tip).unwrap().1, 0.0);
    rt.update(|_, tx| tx.focus(hint, true));
    rt.frame(0.2);
    assert_eq!(rt.presented(tip).unwrap().1, 1.0);
}

#[test]
fn meeting_meta_facts_and_popover_work_by_keyboard() {
    if in_parent("meeting_meta_facts_and_popover_work_by_keyboard") {
        return;
    }
    assert!(page().find("meeting-facts").is_none());
    let start = 1_793_955_600_000_i64;
    let mut rt = page_with((390.0, 900.0), || {
        let folder = core::meetings_dir().join(MEETING);
        let state = core::state_dir();
        std::fs::create_dir_all(&state).unwrap();
        for (path, value) in [
            (
                folder.join("manifest.json"),
                serde_json::json!({"status":"complete","created_at_ms":start,"duration_ms":1320000}),
            ),
            (
                folder.join("english-summary.json"),
                serde_json::json!({"title":"Beta launch plan","context":[CONTEXT],"people":["Maya","Tom"]}),
            ),
            (
                folder.join("tasks.json"),
                serde_json::json!({"meeting_project":"Atlas 2"}),
            ),
            (
                state.join("config.json"),
                serde_json::json!({"version":1,"features":{"projects":true}}),
            ),
            (
                state.join("projects.json"),
                serde_json::json!({"projects":["Atlas 2"]}),
            ),
        ] {
            std::fs::write(path, value.to_string()).unwrap();
        }
    });
    let project = format!("meeting-project-{MEETING}");
    let more = format!("more-meeting-{MEETING}-button");
    let date = chrono::DateTime::from_timestamp_millis(start)
        .unwrap()
        .with_timezone(&chrono::Local);
    let facts = format!("Date: {}. Duration: 22 min", date.format("%-d %b"));
    rt.update(|s, tx| {
        let deadline = Instant::now() + Duration::from_secs(2);
        while s.areas.refresh_pending {
            let event = s
                .events_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("area worker finished");
            s.apply_event(event);
        }
        tx.focus("segment0", true);
    });
    rt.frame(0.0);
    let press = |rt: &mut Runtime<State>, key| {
        rt.key(KeyEvent {
            key,
            mods: rune::Mods::default(),
        });
        rt.frame(0.2);
    };
    // Tab reaches both controls before the facts below them.
    for (key, name) in [
        (more.as_str(), "More actions"),
        ("meeting-facts", facts.as_str()),
        ("meeting-people", "2 people"),
        (&project, "Suggested project: Atlas 2. Change it"),
        ("meeting-status", "Transcript status: English only"),
    ] {
        press(&mut rt, Key::Tab);
        let item = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("missing {key}"));
        assert!(item.focusable && item.focused, "{key}");
        assert!(item.focus_visible, "{key}");
        assert_eq!(item.label.as_deref(), Some(name), "{key}");
        assert_eq!(item.rect[3], 28.0, "{key}");
        assert!(
            item.rect[0] >= 0.0 && item.rect[0] + item.rect[2] <= 390.0,
            "{key}"
        );
    }
    let focused = |rt: &Runtime<State>, key| {
        rt.elements()
            .iter()
            .any(|e| e.focused && e.key.as_deref() == Some(key))
    };
    press(&mut rt, Key::Text(" ".into()));
    assert!(rt.find("meeting-english-only").is_some());
    assert!(focused(&rt, "meeting-status-settings"));
    press(&mut rt, Key::Escape);
    assert!(rt.find("meeting-english-only").is_none());
    assert!(focused(&rt, "meeting-status"));
}

/// Three tasks and two open questions: one task with a detail, one with a guessed date and one
/// with neither owner nor date; one question waits on someone.
fn page_with_tasks(size: (f32, f32)) -> Runtime<State> {
    tasks_page(size, false)
}

/// `page_with_tasks`, optionally with a context line and a topic above the tasks.
fn tasks_page(size: (f32, f32), notes: bool) -> Runtime<State> {
    let mut rt = page_with(size, || {
        let folder = core::meetings_dir().join(MEETING);
        let mut summary = serde_json::json!({
            "title": "Beta launch plan",
            "todo": [
                {"task": "Send the survey results to the team", "title": "Send the survey results",
                 "description": "Send the survey results to the team",
                 "owner": "Maya", "due": "2026-11-27"},
                {"task": "Share the pricing page copy", "title": "Share the pricing page copy",
                 "owner": "Priya", "due": "2026-12-01", "due_default": true},
                {"task": "Book five customer tests", "title": "Book five customer tests"},
            ],
            "to_settle": [
                {"question": "Should the work email move to the end of sign-up?", "waits_on": "Priya"},
                {"question": "Who owns the survey follow-up?"},
            ],
        });
        if notes {
            summary["context"] = serde_json::json!([CONTEXT]);
            summary["topics"] = serde_json::json!([{"title": "Launch", "points": [
                {"type": "decided", "text": POINT},
                {"type": "fact", "text": "Copy is with Priya."}]}]);
        }
        std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
        let state = core::state_dir();
        std::fs::create_dir_all(&state).unwrap();
        let config = serde_json::json!({"version":1,"features":{"tasks":true}});
        std::fs::write(state.join("config.json"), config.to_string()).unwrap();
    });
    // The tasks come from the area worker's snapshot.
    rt.update(|s, _| {
        s.refresh_areas();
        let deadline = Instant::now() + Duration::from_secs(2);
        while s.areas.refresh_pending {
            let event = s
                .events_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("area worker finished");
            s.apply_event(event);
        }
    });
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

fn rail_checkbox(rt: &Runtime<State>, index: usize) -> rune::ElementInfo {
    let path = element(rt, &format!("meeting-task-{index}")).path + "/";
    rt.elements()
        .into_iter()
        .find(|e| e.path.starts_with(&path) && e.role == "checkbox")
        .unwrap()
}

#[test]
fn meeting_tasks_column_is_plain_rows_that_keep_every_fact() {
    if in_parent("meeting_tasks_column_is_plain_rows_that_keep_every_fact") {
        return;
    }
    let mut rt = page_with_tasks(DEFAULT_SIZE);
    rt.update(|s, _| s.tasks.today = chrono::NaiveDate::from_ymd_opt(2026, 11, 27).unwrap());
    rt.settle_all();
    rt.frame(0.0);
    // No card: the lists have neither a rounded box nor a fill.
    for key in ["meeting-tasks", "meeting-questions"] {
        let list = element(&rt, key);
        assert_eq!((list.radius, list.fill.a), (0.0, 0.0), "{key}");
    }
    // Each row: 8 all round, with a 24 px checkbox target around its 16 px mark.
    let row = |i: usize| element(&rt, &format!("meeting-task-{i}")).rect;
    for i in 0..3 {
        let tick = rail_checkbox(&rt, i).rect;
        let path = element(&rt, &format!("meeting-task-{i}")).path + "/";
        let title = rt
            .elements()
            .into_iter()
            .find(|e| e.path.starts_with(&path) && e.key.as_deref() == Some("title"))
            .unwrap()
            .rect;
        assert_eq!((tick[2], tick[3]), (24.0, 24.0), "check target {i}");
        assert_eq!(tick[0] - row(i)[0], 4.0, "tick {i}");
        assert_eq!(
            (title[0] - row(i)[0], title[1] - row(i)[1]),
            (34.0, 8.0),
            "title {i}"
        );
        // The text column is 300 wide: the row is 316 with 8 of padding each side.
        assert_eq!(row(i)[2], 316.0, "row width {i}");
    }
    // 4 px from the label to the first row, 28 between the lists.
    let label = element(&rt, "meeting-task-count").rect;
    assert_eq!(label[3], 22.0);
    assert_eq!(row(0)[1] - (label[1] + label[3]), 4.0);
    let questions = element(&rt, "meeting-questions").rect;
    let tasks = element(&rt, "meeting-tasks").rect;
    assert_eq!(questions[1] - 4.0 - 22.0 - (tasks[1] + tasks[3]), 28.0);
    assert_eq!(element(&rt, "meeting-question-0").rect[0], row(0)[0]);
    // The rail shows the title and plain metadata. The description lives in detail.
    let elements = rt.elements();
    let texts: Vec<_> = elements.iter().filter_map(|e| e.text.as_deref()).collect();
    for want in [
        "Send the survey results",
        "Maya \u{b7} Today",
        "Share the pricing page copy",
        "Priya \u{b7} 1 Dec \u{b7} guessed",
        "Book five customer tests",
        "No owner",
        "Should the work email move to the end of sign-up?",
        "Waiting on Priya",
        "Who owns the survey follow-up?",
    ] {
        assert!(texts.contains(&want), "missing {want}");
    }
    assert!(!texts.contains(&"Send the survey results to the team"));
    assert!(!texts.iter().any(|text| text.contains("No date")));
}

#[test]
fn meeting_tasks_column_fits_a_narrow_window() {
    if in_parent("meeting_tasks_column_fits_a_narrow_window") {
        return;
    }
    let rt = page_with_tasks((390.0, 900.0));
    for i in 0..3 {
        let row = element(&rt, &format!("meeting-task-{i}")).rect;
        assert!(
            (16.0..=17.0).contains(&row[0]) && (373.0..=374.0).contains(&(row[0] + row[2])),
            "row {i}: {row:?}"
        );
        let tick = rail_checkbox(&rt, i).rect;
        assert_eq!(
            (tick[0], tick[2], tick[3]),
            (row[0], 32.0, 32.0),
            "tick {i}"
        );
    }
}

fn wait_for_tick(rt: &mut Runtime<State>, index: usize, done: bool) {
    rt.update(|s, _| {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !s
            .tasks
            .all
            .iter()
            .any(|t| t.index == index && t.done == done)
        {
            let event = s
                .events_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("tick saved");
            s.apply_event(event);
        }
    });
    rt.frame(0.2);
}

#[test]
fn meeting_task_rows_work_by_keyboard_and_checking_one_saves() {
    if in_parent("meeting_task_rows_work_by_keyboard_and_checking_one_saves") {
        return;
    }
    let press = |rt: &mut Runtime<State>, key| {
        rt.key(KeyEvent {
            key,
            mods: rune::Mods::default(),
        });
        rt.frame(0.2);
    };
    let mut rt = page_with_tasks(DEFAULT_SIZE);
    rt.update(|_, tx| tx.focus("meeting-task-0", true));
    rt.frame(0.0);
    let tick = rail_checkbox(&rt, 0);
    assert!(!tick.focusable && !tick.focused);
    assert_eq!(tick.role, "checkbox");
    assert_eq!(
        tick.label.as_deref(),
        Some("Mark Send the survey results done")
    );
    let open = element(&rt, "meeting-task-0");
    assert!(open.focusable && open.focused && open.focus_visible);
    assert_eq!(open.role, "button");
    press(&mut rt, Key::Tab);
    assert!(element(&rt, "meeting-task-1").focused);
    press(&mut rt, Key::Up);
    assert!(element(&rt, "meeting-task-0").focus_visible);
    // Space saves the tick and leaves the row in place for this visit.
    press(&mut rt, Key::Text(" ".into()));
    wait_for_tick(&mut rt, 0, true);
    assert_eq!(element(&rt, "meeting-task-0").rect, open.rect);
    assert_eq!(
        rail_checkbox(&rt, 0).label.as_deref(),
        Some("Mark Send the survey results open")
    );
    rt.update(|s, tx| {
        s.go_home();
        s.select(MEETING, tx);
    });
    rt.frame(0.2);
    assert!(rt.find("meeting-task-0").is_none());
    rt.update(|_, tx| tx.focus("meeting-done", true));
    rt.frame(0.0);
    press(&mut rt, Key::Enter);
    let [x, y, w, h] = rail_checkbox(&rt, 0).rect;
    rt.pointer_move(x + w / 2.0, y + h / 2.0);
    rt.pointer_down(x + w / 2.0, y + h / 2.0);
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    rt.frame(0.2);
    wait_for_tick(&mut rt, 0, false);
    assert_eq!(
        rail_checkbox(&rt, 0).label.as_deref(),
        Some("Mark Send the survey results done")
    );
    rt.update(|_, tx| tx.focus("meeting-task-0", true));
    rt.frame(0.0);
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.view, super::tasks::View::Tasks);
    assert_eq!(
        rt.state.tasks.panel,
        Some(super::tasks::Panel::Task((MEETING.into(), 0)))
    );
}

#[test]
fn meeting_tasks_label_names_the_sides_only_when_they_will_has_tasks() {
    if in_parent("meeting_tasks_label_names_the_sides_only_when_they_will_has_tasks") {
        return;
    }
    let mut rt = page_with_tasks(DEFAULT_SIZE);
    let inside = |rt: &Runtime<State>, key: &str| -> Vec<String> {
        let root = element(rt, key).path;
        rt.elements()
            .into_iter()
            .filter(|e| e.path.starts_with(&format!("{root}/")))
            .filter_map(|e| e.text)
            .collect()
    };
    // Everyone is on our side: one label, "Tasks 3", and no side labels.
    assert_eq!(inside(&rt, "meeting-task-count")[..2], ["Tasks", "3"]);
    assert!(rt.find("meeting-ours-task-count").is_none());
    assert!(rt.find("meeting-external-task-count").is_none());
    // Priya owns one task and is on the other side: "Tasks 3" over "Ours 2" and "They will 1".
    rt.update(|s, _| {
        s.people
            .prefs
            .people_sides
            .insert(core::people::key("Priya"), core::people::Side::External);
    });
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(inside(&rt, "meeting-task-count")[..2], ["Tasks", "3"]);
    assert_eq!(inside(&rt, "meeting-ours-task-count"), ["Ours", "2"]);
    assert_eq!(
        inside(&rt, "meeting-external-task-count"),
        ["They will", "1"]
    );
    // The first side label sits 4 under the Tasks label's row; 28 between the two lists.
    let (tasks, ours, theirs) = (
        element(&rt, "meeting-task-count").rect,
        element(&rt, "meeting-ours-task-count").rect,
        element(&rt, "meeting-external-task-count").rect,
    );
    let list = element(&rt, "meeting-tasks").rect;
    assert_eq!(ours[1] - (tasks[1] + tasks[3]), 4.0);
    assert_eq!(
        theirs[1] - (list[1] + list[3]),
        28.0,
        "theirs {theirs:?}, list {list:?}"
    );
}

#[test]
fn an_answer_names_the_helper_and_model_that_wrote_it_and_old_ones_still_load() {
    if in_parent("an_answer_names_the_helper_and_model_that_wrote_it_and_old_ones_still_load") {
        return;
    }
    let mut rt = page();
    let said = |runner: Option<&str>, model: Option<&str>, fallback: bool| Message {
        role: "assistant".into(),
        text: "Done.".into(),
        runner: runner.map(str::to_owned),
        model: model.map(str::to_owned),
        fallback,
        ..Default::default()
    };
    // Asked for a model, the helper reported none: the label must not claim the request.
    let requested_only = Message {
        requested_model: Some("gpt-6.1-sol".into()),
        ..said(Some("Codex"), None, false)
    };
    let old: Message =
        serde_json::from_str(r#"{"role":"assistant","text":"Older answer."}"#).unwrap();
    assert!(old.runner.is_none() && old.model.is_none() && !old.fallback);
    assert!(!serde_json::to_string(&old).unwrap().contains("runner"));
    let messages = vec![
        old,
        said(Some("Codex"), Some("gpt-6.1-sol"), false),
        said(Some("OpenCode"), None, false),
        said(Some("Claude Sonnet"), None, true),
        requested_only,
    ];
    rt.update(|s, _| s.saved.messages = messages);
    rt.settle_all();
    rt.frame(0.0);
    let label = |i: usize| ask::answer_label(&rt.state, &rt.state.saved.messages[i]);
    assert_eq!(label(0), None);
    assert!(label(1).unwrap().starts_with("Codex"), "{:?}", label(1));
    assert_eq!(label(2).as_deref(), Some("OpenCode (model not reported)"));
    assert_eq!(
        label(3).as_deref(),
        Some("Claude Code (model not reported) \u{b7} fallback")
    );
    assert_eq!(label(4).as_deref(), Some("Codex (model not reported)"));
    assert!(rt.find("answer-model-0").is_none());
    for i in 1..5 {
        assert!(
            rt.find(&format!("answer-model-{i}")).is_some(),
            "answer {i}"
        );
    }
}

#[test]
fn a_fallback_answer_is_saved_with_the_helper_that_wrote_it() {
    if in_parent("a_fallback_answer_is_saved_with_the_helper_that_wrote_it") {
        return;
    }
    let mut rt = page();
    let s = &mut rt.state;
    let mut run = AskRun::stand_in("Codex", &[], 0);
    run.model = Some("gpt-6.1-sol".into());
    run.requested_model = Some("gpt-6.1-sol".into());
    run.fallback_model = Some("claude-sonnet-5-5".into());
    s.asks.insert(MEETING.into(), run);
    let progress = |s: &mut State, event| {
        s.apply_event(Event::Progress {
            id: MEETING.into(),
            generation: 0,
            event,
        })
    };
    progress(
        s,
        AskProgress::Model {
            model: "gpt-6.1-sol-reported".into(),
        },
    );
    assert_eq!(
        s.asks[MEETING].reported_model.as_deref(),
        Some("gpt-6.1-sol-reported")
    );
    progress(
        s,
        AskProgress::Fallback {
            runner: "Claude Sonnet".into(),
        },
    );
    // The first helper's report does not carry over, and the request is now the fallback's.
    assert_eq!(s.asks[MEETING].reported_model, None);
    assert_eq!(
        s.asks[MEETING].requested_model.as_deref(),
        Some("claude-sonnet-5-5")
    );
    progress(
        s,
        AskProgress::Model {
            model: "claude-sonnet-5-5-20261101".into(),
        },
    );
    let mut run = s.asks.remove(MEETING).unwrap();
    let (tx, rx) = mpsc::channel();
    tx.send(Ok("Here is the answer.".to_string())).unwrap();
    run.rx = rx;
    s.finish_ask(MEETING, run, None);
    let saved = s.saved.messages.last().unwrap();
    assert_eq!(saved.runner.as_deref(), Some("Claude Sonnet"));
    assert_eq!(saved.model.as_deref(), Some("claude-sonnet-5-5-20261101"));
    assert_eq!(saved.requested_model.as_deref(), Some("claude-sonnet-5-5"));
    assert!(saved.fallback);
}

#[test]
fn a_connected_endpoint_answer_names_the_model_it_reported_or_says_it_did_not() {
    if in_parent("a_connected_endpoint_answer_names_the_model_it_reported_or_says_it_did_not") {
        return;
    }
    let mut rt = page();
    let s = &mut rt.state;
    for (reported, want) in [
        (
            Some("served-model-1"),
            "Chat completions \u{b7} served-model-1",
        ),
        (None, "Chat completions (model not reported)"),
    ] {
        let mut run = AskRun::stand_in(core::provider::NAME, &[], 0);
        run.requested_model = Some("asked-model".into());
        s.asks.insert(MEETING.into(), run);
        if let Some(model) = reported {
            s.apply_event(Event::Progress {
                id: MEETING.into(),
                generation: 0,
                event: AskProgress::Model {
                    model: model.into(),
                },
            });
        }
        let mut run = s.asks.remove(MEETING).unwrap();
        let (tx, rx) = mpsc::channel();
        tx.send(Ok("An answer.".to_string())).unwrap();
        run.rx = rx;
        s.finish_ask(MEETING, run, None);
        let saved = s.saved.messages.last().unwrap();
        assert_eq!(saved.runner.as_deref(), Some(core::provider::NAME));
        assert_eq!(saved.requested_model.as_deref(), Some("asked-model"));
        assert_eq!(saved.model.as_deref(), reported);
        assert_eq!(ask::answer_label(s, saved).as_deref(), Some(want));
    }
}

const STOP_QUESTION: &str = "When does the beta ship?";
const PARTIAL_ANSWER: &str = "The beta ships on Friday.";

fn start_stop_fixture(rt: &mut Runtime<State>) {
    rt.update(|s, _| {
        scene_agents(s, ALL_AGENTS);
        let (saved, message_id) = s
            .store
            .append_message_identified(
                MEETING,
                Message {
                    role: "user".into(),
                    text: STOP_QUESTION.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        s.saved = saved;
        s.question = TextEdit::default();
        s.ask_open = true;
        let mut run = AskRun::stand_in("Codex", &[], 0);
        run.submitted_message_id = Some(message_id);
        run.requested_model = Some("requested-model".into());
        s.asks.insert(MEETING.into(), run);
    });
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
}

fn ask_fixture_progress(rt: &mut Runtime<State>, event: AskProgress) {
    rt.update(|s, _| {
        s.apply_event(Event::Progress {
            id: MEETING.into(),
            generation: 0,
            event,
        });
    });
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
}

fn worker_reports_stop(rt: &mut Runtime<State>) {
    ask_fixture_progress(
        rt,
        AskProgress::Failed {
            message: core::ask::STOPPED.into(),
        },
    );
}

fn assert_no_ask_alert(rt: &Runtime<State>) {
    assert!(!rt.state.ask_errors.contains_key(MEETING));
    assert!(!rt.elements().iter().any(|e| {
        e.role == "alert"
            && e.text
                .as_deref()
                .is_some_and(|text| text.starts_with("Could not answer."))
    }));
}

fn stopped_label_count(rt: &Runtime<State>) -> usize {
    rt.elements()
        .iter()
        .filter(|e| e.text.as_deref() == Some("Stopped"))
        .count()
}

#[test]
fn stop_keeps_the_streamed_answer_and_marks_it_in_the_thread() {
    if in_parent("stop_keeps_the_streamed_answer_and_marks_it_in_the_thread") {
        return;
    }
    let mut rt = page();
    start_stop_fixture(&mut rt);
    ask_fixture_progress(
        &mut rt,
        AskProgress::Model {
            model: "reported-model".into(),
        },
    );
    ask_fixture_progress(
        &mut rt,
        AskProgress::Text {
            text: PARTIAL_ANSWER.into(),
        },
    );
    assert!(rt.find("ask-streaming-answer").is_some());
    assert_eq!(stopped_label_count(&rt), 0);
    tap(&mut rt, "ask-stop");
    assert!(rt.state.asks[MEETING].cancel.stopped());
    worker_reports_stop(&mut rt);

    let saved = rt.state.store.load(MEETING).unwrap();
    let [question, answer] = saved.messages.as_slice() else {
        panic!("expected the question and one partial answer, got {saved:?}");
    };
    assert_eq!(
        (question.role.as_str(), question.text.as_str()),
        ("user", STOP_QUESTION)
    );
    assert_eq!(
        (answer.role.as_str(), answer.text.as_str()),
        ("assistant", PARTIAL_ANSWER)
    );
    assert!(answer.stopped);
    assert_eq!(answer.runner.as_deref(), Some("Codex"));
    assert_eq!(answer.model.as_deref(), Some("reported-model"));
    assert_eq!(answer.requested_model.as_deref(), Some("requested-model"));
    assert_eq!(serde_json::to_value(answer).unwrap()["stopped"], true);
    assert!(rt.state.question.text.is_empty());
    assert_no_ask_alert(&rt);
    assert!(rt.find("ask-streaming-answer").is_none());
    assert_eq!(stopped_label_count(&rt), 1);
    let answer_path = element(&rt, "msg-1").path + "/";
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.path.starts_with(&answer_path) && e.text.as_deref() == Some("Stopped"))
    );
}

#[test]
fn stop_without_answer_text_returns_the_question_to_the_box() {
    if in_parent("stop_without_answer_text_returns_the_question_to_the_box") {
        return;
    }
    let mut rt = page();
    for text in [None, Some(" \n\t ")] {
        start_stop_fixture(&mut rt);
        if let Some(text) = text {
            let text = text.into();
            ask_fixture_progress(&mut rt, AskProgress::Text { text });
        }
        tap(&mut rt, "ask-stop");
        assert!(rt.state.asks[MEETING].cancel.stopped());
        worker_reports_stop(&mut rt);

        assert!(rt.state.store.load(MEETING).unwrap().messages.is_empty());
        assert_eq!(rt.state.question.text, STOP_QUESTION);
        assert_no_ask_alert(&rt);
        assert!(rt.find("ask-thread").is_none());
        assert_eq!(stopped_label_count(&rt), 0);
    }
}

#[test]
fn old_and_completed_answers_show_no_stop_label() {
    if in_parent("old_and_completed_answers_show_no_stop_label") {
        return;
    }
    let mut rt = page_with(DEFAULT_SIZE, || {
        let root = core::state_dir();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(format!("{MEETING}.json")),
            r#"{"messages":[{"role":"assistant","text":"An older answer."}]}"#,
        )
        .unwrap();
    });
    let old = &rt.state.saved.messages[0];
    assert_eq!(old.text, "An older answer.");
    assert!(!old.stopped);
    assert!(rt.find("msg-0").is_some());
    assert_eq!(stopped_label_count(&rt), 0);

    start_stop_fixture(&mut rt);
    let (tx, rx) = mpsc::channel();
    tx.send(Ok("A finished answer.".into())).unwrap();
    rt.state.asks.get_mut(MEETING).unwrap().rx = rx;
    ask_fixture_progress(&mut rt, AskProgress::Finished);

    let saved = rt.state.store.load(MEETING).unwrap();
    let answer = saved.messages.last().unwrap();
    assert_eq!(answer.text, "A finished answer.");
    assert!(!answer.stopped);
    assert!(
        serde_json::to_value(answer)
            .unwrap()
            .get("stopped")
            .is_none()
    );
    assert!(rt.find("msg-2").is_some());
    assert_eq!(stopped_label_count(&rt), 0);
}

#[test]
fn deletion_discards_streamed_text_even_after_the_user_pressed_stop() {
    if in_parent("deletion_discards_streamed_text_even_after_the_user_pressed_stop") {
        return;
    }
    use sidenote::core::meeting_delete::{Eligibility, Storage, read_visibility};

    let mut rt = page_with(DEFAULT_SIZE, || {
        let manifest = core::meetings_dir().join(MEETING).join("manifest.json");
        std::fs::write(manifest, br#"{"status":"complete"}"#).unwrap();
    });
    start_stop_fixture(&mut rt);
    let run = rt.state.asks.get_mut(MEETING).unwrap();
    run.live = true;
    run.visibility = *read_visibility();
    ask_fixture_progress(
        &mut rt,
        AskProgress::Text {
            text: PARTIAL_ANSWER.into(),
        },
    );
    tap(&mut rt, "ask-stop");
    assert!(rt.state.asks[MEETING].cancel.stopped());
    Storage::new(&core::meetings_dir(), &rt.state.store.root)
        .unwrap()
        .delete(
            MEETING,
            "Synthetic".into(),
            1,
            |_| Ok(Eligibility::Ready),
            |_| Ok(()),
        )
        .unwrap();
    worker_reports_stop(&mut rt);

    assert!(!rt.state.asks.contains_key(MEETING));
    assert!(rt.state.store.load(MEETING).unwrap().messages.is_empty());
    assert_eq!(stopped_label_count(&rt), 0);
}

#[test]
fn cloud_recovery_discards_streamed_text_and_keeps_the_retry_question() {
    if in_parent("cloud_recovery_discards_streamed_text_and_keeps_the_retry_question") {
        return;
    }
    let mut rt = page();
    start_stop_fixture(&mut rt);
    rt.state.asks.get_mut(MEETING).unwrap().runner = core::provider::NAME.into();
    ask_fixture_progress(
        &mut rt,
        AskProgress::Text {
            text: PARTIAL_ANSWER.into(),
        },
    );
    rt.update(|s, _| s.connection_recovery(2));

    assert!(!rt.state.asks.contains_key(MEETING));
    assert!(rt.state.store.load(MEETING).unwrap().messages.is_empty());
    assert_eq!(rt.state.question.text, STOP_QUESTION);
    assert_no_ask_alert(&rt);
}

/// At 390 wide the Ask panel keeps the page's 16 point gutter, Hide and Clear keep their own
/// room inside it, the caption shrinks to what is left, and the page fades into the bar rather
/// than being cut off at its edge.
#[test]
fn at_390_wide_the_ask_header_keeps_clear_off_the_edge_and_the_page_fades_out() {
    if in_parent("at_390_wide_the_ask_header_keeps_clear_off_the_edge_and_the_page_fades_out") {
        return;
    }
    let mut rt = page_at((390.0, 844.0));
    let said = |role: &str, text: &str| Message {
        role: role.into(),
        text: text.into(),
        ..Default::default()
    };
    rt.update(|s, _| {
        s.saved.messages = vec![
            said("user", "What did we decide about the launch date?"),
            said("assistant", "The beta ships on Friday."),
            said("user", "And who owns the follow up?"),
            said("assistant", "Maya will send the draft."),
        ]
    });
    // The panel rises in on its first frame; let that finish so the rects are the resting ones.
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    let rect =
        |rt: &Runtime<State>, key: &str| rt.presented(key).unwrap_or_else(|| panic!("{key}")).0;
    let (overlay, thread) = (rect(&rt, "ask-overlay"), rect(&rt, "ask-thread"));
    assert_eq!(
        (
            thread[0] - overlay[0],
            overlay[0] + overlay[2] - (thread[0] + thread[2])
        ),
        (16.0, 16.0),
        "the panel keeps the 16 px gutter on both sides"
    );
    let (caption, hide, clear) = (
        rect(&rt, "ask-caption"),
        rect(&rt, "ask-toggle"),
        rect(&rt, "ask-clear"),
    );
    let panel_right = thread[0] + thread[2];
    assert!(
        clear[0] + clear[2] <= panel_right - 4.0,
        "Clear touches the edge"
    );
    assert!(
        caption[0] + caption[2] <= hide[0] && hide[0] + hide[2] <= clear[0],
        "the caption runs under Hide or Clear"
    );
    let scroll = rect(&rt, "meeting-scroll");
    assert_eq!(scroll[1] + scroll[3], overlay[1] + overlay[3]);
    assert!(rt.find("meeting-scroll-fade").is_none());
}

#[test]
fn the_model_picker_shows_on_a_meeting_the_live_screen_and_a_new_chat() {
    if in_parent("the_model_picker_shows_on_a_meeting_the_live_screen_and_a_new_chat") {
        return;
    }
    let mut rt = page();
    rt.update(|s, _| scene_agents(s, ALL_AGENTS));
    rt.frame(0.0);
    assert!(rt.find("ask-model").is_none());
    assert!(rt.find("ask-options-menu").is_none());
    rt.update(|s, _| s.ask_open = true);
    rt.frame(0.0);
    assert!(rt.find("ask-options").is_some());
    assert!(
        rt.state.selected.is_some() && rt.find("ask-model").is_some(),
        "meeting"
    );
    rt.update(|s, _| s.selected = None);
    rt.frame(0.0);
    assert!(rt.find("ask-model").is_some(), "new chat");
    rt.update(|s, _| scene_live(s, "", &[], false));
    rt.frame(0.0);
    assert!(
        rt.state.on_live() && rt.find("ask-model").is_some(),
        "live screen"
    );
}

#[test]
fn meeting_ticks_keep_the_all_meetings_chat() {
    if in_parent("meeting_ticks_keep_the_all_meetings_chat") {
        return;
    }
    let mut rt = page_with_tasks(DEFAULT_SIZE);
    let all = core::ALL_KEY;
    let global = Message {
        role: "user".into(),
        text: "Across meetings".into(),
        ..Default::default()
    };
    rt.update(|s, _| {
        s.meeting_ui.tabs = true;
        s.store.append_message(all, global.clone()).unwrap();
    });
    rt.frame(0.0);
    tap(&mut rt, "tick-0");
    wait_for_tick(&mut rt, 0, true);
    let checked = element(&rt, "tick-0").fill;
    rt.update(|s, _| s.set_ask_scope(true));
    rt.frame(0.0);
    assert_eq!(element(&rt, "tick-0").fill, checked);
    tap(&mut rt, "tick-0");
    wait_for_tick(&mut rt, 0, false);
    assert_eq!(rt.state.ask_key(), all);
    assert_eq!(rt.state.saved.messages, [global]);
}

#[test]
fn ask_follows_the_open_meeting_and_only_shows_a_prompt_in_an_empty_editor() {
    if in_parent("ask_follows_the_open_meeting_and_only_shows_a_prompt_in_an_empty_editor") {
        return;
    }
    for size in [DEFAULT_SIZE, (390.0, 844.0)] {
        let mut rt = page_at(size);
        rt.update(|s, _| {
            scene_agents(s, ALL_AGENTS);
            s.ask_open = true;
        });
        rt.frame(0.0);
        assert!(rect_of(&rt, "ask-input")[2] >= 90.0);
        let chip = |rt: &Runtime<State>| element(rt, "ask-prompt").label.unwrap();
        assert_eq!(rt.state.ask_key(), MEETING);
        assert_eq!(chip(&rt), "Ask options, What did we decide?");
        rt.update(|s, _| s.set_ask_scope(true));
        rt.frame(0.0);
        assert_eq!(rt.state.ask_key(), core::ALL_KEY);
        rt.update(|s, _| s.go_home());
        rt.frame(0.0);
        assert_eq!(chip(&rt), "Ask options, List my tasks");
        rt.update(|s, _| s.question = TextEdit::new("A draft"));
        rt.frame(0.0);
        assert!(rt.find("ask-prompt").is_none());
        rt.update(|s, _| {
            s.question = TextEdit::default();
            scene_live(s, "", &[], false);
            s.hex.status = Some(Ok(hex_engine_recording()));
            use sidenote::hex_engine::{Feature, Support};
            let caps = [(Feature::RecordStop, Support::Supported)].into();
            s.hex_apply(hex::Msg::Caps("test".into(), Ok(caps)));
        });
        rt.frame(0.0);
        assert!(rt.state.ask_current_live());
        assert_eq!(chip(&rt), "Ask options, What did I miss");
        assert_eq!(
            element(&rt, "hex-stop").label.as_deref(),
            Some("Stop recording")
        );
        assert!(rect_of(&rt, "hex-stop")[0] < rect_of(&rt, "ask-pill")[0]);
        rt.update(|s, _| {
            s.asks
                .insert(s.ask_key(), AskRun::stand_in("Codex", &[], 0));
        });
        rt.frame(0.0);
        assert!(rt.find("ask-prompt").is_none());
        rt.update(|s, _| s.go_home());
        rt.frame(0.0);
        assert_eq!(rt.state.ask_key(), core::ALL_KEY);
        assert_eq!(chip(&rt), "Ask options, List my tasks");
    }
}

#[test]
fn a_pick_applies_to_the_next_question_and_default_follows_settings() {
    if in_parent("a_pick_applies_to_the_next_question_and_default_follows_settings") {
        return;
    }
    let mut rt = page();
    rt.update(|s, _| scene_agents(s, ALL_AGENTS));
    let groups = ask_settings::picker_groups(&rt.state);
    assert!(
        groups.len() >= 2 && groups.iter().all(|g| !g.2.is_empty()),
        "{groups:?}"
    );
    let before = rt.state.ask_runner_name().map(str::to_owned);
    rt.update(|s, _| s.choose_ask_pick("Codex".into(), Some("gpt-test".into())));
    assert_eq!(rt.state.ask_runner_name(), Some("Codex"));
    assert_eq!(rt.state.ask_model().as_deref(), Some("gpt-test"));
    assert!(ask::picker_label(&rt.state).contains("Codex"));
    rt.update(|s, _| s.choose_ask_default());
    assert_eq!(rt.state.ask_runner_name().map(str::to_owned), before);
}

// The helper stand-in (`core::ask_session::STAND_IN`) only exists with debug assertions, so
// everything that installs it is compiled the same way. A release test build then has no way
// to reach a real helper and no unresolved symbol.
#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

/// Opened by the test to let helper A answer.
#[cfg(debug_assertions)]
static GATE: AtomicBool = AtomicBool::new(false);
/// Set by helper A as soon as it is running: the test switches only after this.
#[cfg(debug_assertions)]
static ENTERED: AtomicBool = AtomicBool::new(false);
#[cfg(debug_assertions)]
static CALLS: std::sync::Mutex<Vec<(String, Option<String>)>> = std::sync::Mutex::new(Vec::new());
/// How long the fake helper and the test wait for each other.
#[cfg(debug_assertions)]
const WAIT: Duration = Duration::from_secs(20);

/// Makes the fake Codex fail at once, as an agent that is not installed does.
#[cfg(debug_assertions)]
static CODEX_FAILS: AtomicBool = AtomicBool::new(false);
/// What the fake Claude Sonnet reports as the model that answered.
#[cfg(debug_assertions)]
const SONNET_REPORTS: &str = "claude-sonnet-5-5-20261101";

/// The fake helper: records what it was started with and reports a model the way a real one
/// would: Codex a resolved name that differs from the request, Claude Sonnet a dated name,
/// OpenCode none. Helper A (Codex) says it has started, then holds its answer until the gate
/// opens, so the test can switch while it is still answering.
#[cfg(debug_assertions)]
fn fake_helper(
    runner: &core::Runner,
    _prompt: &str,
    progress: &dyn Fn(AskProgress),
) -> Result<String, String> {
    CALLS
        .lock()
        .unwrap()
        .push((runner.name.clone(), runner.model.clone()));
    if runner.name == "Codex" && CODEX_FAILS.load(SeqCst) {
        return Err("Codex is not available".into());
    }
    match runner.name.as_str() {
        "Codex" => progress(AskProgress::Model {
            model: format!("{}-resolved", runner.model.clone().unwrap_or_default()),
        }),
        "Claude Sonnet" => progress(AskProgress::Model {
            model: SONNET_REPORTS.into(),
        }),
        _ => {}
    }
    if runner.name == "Codex" {
        ENTERED.store(true, SeqCst);
        let end = Instant::now() + WAIT;
        while !GATE.load(SeqCst) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        if !GATE.load(SeqCst) {
            return Err("the gate never opened".into());
        }
    }
    Ok(format!("answer from {}", runner.name))
}

#[cfg(debug_assertions)]
fn wait_until_entered() {
    let end = Instant::now() + WAIT;
    while !ENTERED.load(SeqCst) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(ENTERED.load(SeqCst), "helper A never started");
}

#[cfg(debug_assertions)]
fn first_model(s: &State, agent: &str) -> (String, Option<String>) {
    let group = ask_settings::picker_groups(s)
        .into_iter()
        .find(|g| g.0 == agent)
        .unwrap();
    (group.1, group.2.into_iter().find_map(|m| m.1))
}

/// Applies events until the answer for the open meeting has been saved.
#[cfg(debug_assertions)]
fn finish_answer(rt: &mut Runtime<State>) {
    while rt.state.asks.contains_key(MEETING) {
        let event = rt
            .state
            .events_rx
            .recv_timeout(Duration::from_secs(25))
            .expect("no answer");
        rt.update(|s, _| s.apply_event(event));
    }
}

#[test]
fn tab_from_the_ask_editor_focuses_the_model_picker() {
    if in_parent("tab_from_the_ask_editor_focuses_the_model_picker") {
        return;
    }
    let mut rt = page();
    rt.set_full_keyboard_access(false);
    rt.update(|s, tx| {
        scene_agents(s, ALL_AGENTS);
        s.ask_open = true;
        tx.focus("ask-input", true);
    });
    rt.frame(0.0);
    assert!(rt.state.question.focused);

    rt.key(KeyEvent {
        key: Key::Tab,
        mods: rune::Mods::default(),
    });
    rt.frame(0.0);
    assert!(!rt.state.question.focused);
    rt.update_if(|s, tx| poll(s, 0.0, tx));
    rt.frame(0.0);
    rt.update_if(|s, tx| {
        assert_eq!(tx.focused(), Some("ask-model"));
        assert!(s.ask_open);
        false
    });
}

#[test]
fn reduced_motion_opens_the_ask_panel_without_moving_the_pill() {
    if in_parent("reduced_motion_opens_the_ask_panel_without_moving_the_pill") {
        return;
    }
    let mut state = State::new();
    scene_agents(&mut state, ALL_AGENTS);
    let mut rt = Runtime::with_text(
        state,
        |s, cx| {
            cx.reduced_motion(true);
            view(s, cx)
        },
        DEFAULT_SIZE,
        Env::default(),
        TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
    );
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    let closed = rect_of(&rt, "ask-options");
    let pill = rect_of(&rt, "ask-pill");
    rt.update(|_, tx| tx.focus("ask-input", true));
    rt.frame(0.0);
    rt.update(|s, tx| s.ask_open = ask::focus_open(s, tx));
    rt.frame(0.016);
    let opened = rect_of(&rt, "ask-options");
    assert_eq!((opened[1], opened[3]), (closed[1], closed[3]));
    assert_eq!(rect_of(&rt, "ask-pill"), pill);
}

#[cfg(debug_assertions)]
#[test]
fn ask_pressing_send_and_a_suggestion_keeps_the_editor_focused() {
    if in_parent("ask_pressing_send_and_a_suggestion_keeps_the_editor_focused") {
        return;
    }
    *core::ask_session::STAND_IN.lock().unwrap() = Some(fake_helper);
    GATE.store(true, SeqCst);
    let mut rt = page();
    rt.update(|s, tx| {
        scene_agents(s, ALL_AGENTS);
        s.ask_open = true;
        tx.focus("ask-input", true);
    });
    rt.frame(0.0);
    assert!(rt.state.question.focused);
    rt.key(KeyEvent {
        key: Key::Text("Summarize the plan".into()),
        mods: rune::Mods::default(),
    });
    rt.frame(0.2);
    tap(&mut rt, "ask-send");
    assert!(rt.state.asks.contains_key(MEETING));
    assert!(rt.state.question.focused, "Send kept the editor's focus");
    rt.key(KeyEvent {
        key: Key::Text("x".into()),
        mods: rune::Mods::default(),
    });
    rt.frame(0.0);
    assert_eq!(
        rt.state.question.text, "x",
        "the editor still receives keys"
    );
    finish_answer(&mut rt);
    rt.update(|s, _| s.question.text.clear());
    rt.update(|s, _| s.thread_hidden = true);
    rt.frame(0.2);
    tap(&mut rt, "ask-options");
    tap(&mut rt, "ask-options-item0");
    assert!(rt.state.asks.contains_key(MEETING));
    assert!(
        rt.state.question.focused,
        "The suggestion kept the editor's focus"
    );
    finish_answer(&mut rt);
}

#[cfg(debug_assertions)]
#[test]
fn switching_helper_mid_answer_labels_each_answer_and_runs_the_next_on_the_new_one() {
    if in_parent("switching_helper_mid_answer_labels_each_answer_and_runs_the_next_on_the_new_one")
    {
        return;
    }
    *core::ask_session::STAND_IN.lock().unwrap() = Some(fake_helper);
    let mut rt = page();
    rt.update(|s, _| scene_agents(s, ALL_AGENTS));
    let (a, a_model) = first_model(&rt.state, "Codex");
    let (b, b_model) = first_model(&rt.state, "OpenCode");
    rt.update(|s, _| s.choose_ask_pick(a.clone(), a_model.clone()));
    rt.update(|s, _| s.send("What do I need to do next?"));
    let run = &rt.state.asks[MEETING];
    assert_eq!((run.runner.as_str(), &run.model), (a.as_str(), &a_model));
    // Switch only once A has really started and is holding its answer.
    wait_until_entered();
    assert!(!GATE.load(SeqCst), "A must still be answering");
    assert_eq!(*CALLS.lock().unwrap(), vec![(a.clone(), a_model.clone())]);
    rt.update(|s, _| s.choose_ask_pick(b.clone(), b_model.clone()));
    assert!(ask::picker_label(&rt.state).starts_with("Next answer: "));
    assert_eq!(rt.state.asks[MEETING].runner, a);
    GATE.store(true, SeqCst);
    finish_answer(&mut rt);
    let first = rt.state.saved.messages.last().unwrap().clone();
    assert_eq!(first.text, format!("answer from {a}"));
    // Codex reported a resolved name that differs from the request: the answer keeps both.
    let resolved = format!("{}-resolved", a_model.clone().unwrap());
    assert_eq!(
        (
            first.runner.as_deref(),
            first.model.as_deref(),
            &first.requested_model
        ),
        (Some(a.as_str()), Some(resolved.as_str()), &a_model)
    );
    assert!(
        ask::answer_label(&rt.state, &first)
            .unwrap()
            .starts_with("Codex")
    );
    rt.update(|s, _| s.send("And after that?"));
    assert_eq!(rt.state.asks[MEETING].runner, b);
    finish_answer(&mut rt);
    let second = rt.state.saved.messages.last().unwrap().clone();
    assert_eq!(second.text, format!("answer from {b}"));
    // OpenCode reported no model: the request is kept, and the label does not claim it.
    assert_eq!(
        (
            second.runner.as_deref(),
            &second.model,
            &second.requested_model
        ),
        (Some(b.as_str()), &None, &b_model)
    );
    assert_eq!(
        ask::answer_label(&rt.state, &second).as_deref(),
        Some("OpenCode (model not reported)")
    );
    assert!(rt.state.ask_settings.pick.is_none());
    // Reload the saved thread from disk: both answers and their labels are unchanged.
    let before = rt.state.saved.messages.clone();
    rt.update(|s, _| s.load_saved());
    assert_eq!(rt.state.saved.messages, before);
    let labels: Vec<_> = rt
        .state
        .saved
        .messages
        .iter()
        .filter(|m| m.role == "assistant")
        .map(|m| ask::answer_label(&rt.state, m))
        .collect();
    assert!(labels[0].as_ref().unwrap().starts_with("Codex"));
    assert_eq!(labels[1].as_deref(), Some("OpenCode (model not reported)"));
    let calls = CALLS.lock().unwrap().clone();
    assert_eq!(calls, vec![(a, a_model), (b, b_model)]);
}

#[cfg(debug_assertions)]
#[test]
fn a_fallback_answer_through_the_real_ask_names_the_helper_that_answered_and_survives_reload() {
    if in_parent(
        "a_fallback_answer_through_the_real_ask_names_the_helper_that_answered_and_survives_reload",
    ) {
        return;
    }
    *core::ask_session::STAND_IN.lock().unwrap() = Some(fake_helper);
    CODEX_FAILS.store(true, SeqCst);
    let mut rt = page();
    rt.update(|s, _| scene_agents(s, ALL_AGENTS));
    // A live meeting asks Codex first and falls back to the normal helper when it fails.
    rt.update(|s, _| {
        s.ask_settings.recording = true;
        s.meeting_agent_transition();
    });
    assert_eq!(rt.state.ask_runner_name(), Some("Codex"));
    let fallback_request = rt.state.normal_ask_model();
    rt.update(|s, _| s.send("What did we decide?"));
    assert_eq!(rt.state.notice, None, "the question did not save");
    finish_answer(&mut rt);
    assert_eq!(rt.state.notice, None, "the answer did not save");
    let calls: Vec<_> = CALLS.lock().unwrap().iter().map(|c| c.0.clone()).collect();
    assert_eq!(calls, ["Codex", "Claude Sonnet"]);
    let answer = rt.state.saved.messages.last().unwrap().clone();
    assert_eq!(answer.text, "answer from Claude Sonnet");
    assert_eq!(answer.runner.as_deref(), Some("Claude Sonnet"));
    assert_eq!(answer.model.as_deref(), Some(SONNET_REPORTS));
    assert_eq!(answer.requested_model, fallback_request);
    assert!(answer.fallback);
    // After a reload from disk the label still names the helper that answered.
    rt.update(|s, _| s.load_saved());
    let saved = rt.state.saved.messages.last().unwrap();
    assert_eq!(saved, &answer);
    let label = ask::answer_label(&rt.state, saved).unwrap();
    assert!(
        label.starts_with("Claude Code") && label.ends_with("fallback"),
        "{label}"
    );
}

#[cfg(debug_assertions)]
#[test]
fn a_one_question_pick_leaves_the_saved_default_alone_after_submit_and_restart() {
    if in_parent("a_one_question_pick_leaves_the_saved_default_alone_after_submit_and_restart") {
        return;
    }
    *core::ask_session::STAND_IN.lock().unwrap() = Some(fake_helper);
    GATE.store(true, SeqCst);
    let mut rt = page();
    rt.update(|s, _| scene_agents(s, ALL_AGENTS));
    // The Settings default is Claude Sonnet, saved the way the Settings control saves it.
    rt.update(|s, _| s.choose_normal_ask_runner("Claude Sonnet".into()));
    let saved = || core::last_used_runner().unwrap();
    assert_eq!(saved().as_deref(), Some("Claude Sonnet"));
    // Pick Codex for one question and send it through the real Ask path.
    let (codex, codex_model) = first_model(&rt.state, "Codex");
    rt.update(|s, _| s.choose_ask_pick(codex.clone(), codex_model.clone()));
    rt.update(|s, _| s.send("What do I need to do next?"));
    assert_eq!(rt.state.notice, None, "the first question did not save");
    finish_answer(&mut rt);
    assert_eq!(rt.state.notice, None, "the first answer did not save");
    assert_eq!(*CALLS.lock().unwrap(), vec![(codex, codex_model)]);
    assert_eq!(rt.state.runner.as_deref(), Some("Claude Sonnet"));
    assert!(rt.state.ask_settings.pick.is_none());
    // After submit, and as a restart reads it, the default is still the Settings choice.
    assert_eq!(saved().as_deref(), Some("Claude Sonnet"));
    let restarted = default_runner(&rt.state.runners, &rt.state.runner_found, saved());
    assert_eq!(restarted.as_deref(), Some("Claude Sonnet"));
    // The next question with no pick goes to the default, not to Codex.
    rt.update(|s, _| s.send("And after that?"));
    assert_eq!(rt.state.notice, None, "the second question did not save");
    assert_eq!(rt.state.asks[MEETING].runner, "Claude Sonnet");
    finish_answer(&mut rt);
    assert_eq!(rt.state.notice, None, "the second answer did not save");
    assert_eq!(saved().as_deref(), Some("Claude Sonnet"));
}

/// A saved-thread write only reads the device id, so another holder of `device.lock` (the area
/// worker that `refresh_areas` starts) must not make the question or the answer fail to save.
#[cfg(all(debug_assertions, any(windows, target_os = "linux")))]
#[test]
fn a_question_and_its_answer_save_while_another_thread_holds_the_device_lock() {
    if in_parent("a_question_and_its_answer_save_while_another_thread_holds_the_device_lock") {
        return;
    }
    *core::ask_session::STAND_IN.lock().unwrap() = Some(fake_helper);
    GATE.store(true, SeqCst);
    let mut rt = page();
    rt.update(|s, _| scene_agents(s, ALL_AGENTS));
    rt.update(|s, _| s.choose_normal_ask_runner("Claude Sonnet".into()));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let held = options.open(core::state_dir().join("device.lock")).unwrap();
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        assert_eq!(
            unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
    }
    rt.update(|s, _| s.send("What did we decide?"));
    assert_eq!(rt.state.notice, None);
    assert!(
        rt.state.asks.contains_key(MEETING),
        "the question was not sent"
    );
    let question = rt.state.saved.messages.last().unwrap();
    assert_eq!(
        (question.role.as_str(), question.text.as_str()),
        ("user", "What did we decide?")
    );
    finish_answer(&mut rt);
    assert_eq!(rt.state.notice, None);
    let answer = rt.state.saved.messages.last().unwrap();
    assert_eq!(answer.role, "assistant");
    drop(held);
}

#[cfg(all(debug_assertions, target_os = "linux"))]
#[test]
fn retiring_memory_shares_the_wait_between_two_contended_locks() {
    use sidenote::core::memory::{MemoryStore, RecordStatus};
    use std::{
        fs::{self, OpenOptions},
        os::fd::AsRawFd,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    if in_parent("retiring_memory_shares_the_wait_between_two_contended_locks") {
        return;
    }
    let mut rt = page();
    let root = core::state_dir();
    let store = MemoryStore::default();
    let id = "synthetic-lock-proof";
    fs::create_dir_all(root.join("memory")).unwrap();
    fs::write(
        root.join("memory/records.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "records": [{
                "id": id, "type": "decision", "text": "Synthetic claim", "sources": [],
                "status": "active", "basis": "owner_confirmed",
                "updated_at": "2026-11-09T00:00:00Z", "source_fingerprint": "synthetic"
            }],
            "proposals": [], "events": [],
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        store.list_records().unwrap()[0].status,
        RecordStatus::Active
    );

    let hold = |path: std::path::PathBuf| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        file
    };
    let first = hold(root.join("local/memory/records.lock"));
    let hash = blake3::hash(b"memory/records.json").to_hex();
    let second = hold(root.join(format!("local/locks/{hash}.lock")));
    let (retiring, wait_for_retire) = mpsc::channel();
    let mut elapsed = Duration::ZERO;
    thread::scope(|scope| {
        scope.spawn(move || {
            wait_for_retire.recv().unwrap();
            thread::sleep(Duration::from_millis(75));
            drop(first);
        });
        rt.update(|s, _| {
            retiring.send(()).unwrap();
            let start = Instant::now();
            s.memory_retire(id);
            elapsed = start.elapsed();
        });
    });
    assert_eq!(
        rt.state.memory.errors.get(id).map(String::as_str),
        Some(super::edit_worker::SaveError::Busy.message())
    );
    // Allow for filesystem work and scheduling outside the 200 ms lock budget.
    assert!(
        elapsed < Duration::from_millis(250),
        "retire took {elapsed:?}"
    );
    drop(second);
    assert_eq!(
        store.list_records().unwrap()[0].status,
        RecordStatus::Active
    );
}

/// The dock lines up with the notes column and its prompt sits above it.
fn the_bar_is_centered(name: &str, size: (f32, f32)) {
    if in_parent(name) {
        return;
    }
    let mut rt = page_at(size);
    rt.update(|s, _| {
        scene_agents(s, ALL_AGENTS);
        s.ask_open = true;
    });
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    let rect = |key: &str| {
        rt.presented(key)
            .unwrap_or_else(|| panic!("{key} at {size:?}"))
            .0
    };
    let (pane, dock, chip, pill) = (
        rect("meeting-title"),
        rect("meeting-bottom-bar"),
        rect("ask-prompt"),
        rect("ask-pill"),
    );
    assert_eq!(pill[0], pane[0]);
    assert!(pill[2] <= dock[2]);
    assert_eq!(pill[3], 70.0);
    assert_eq!(chip[0], pill[0]);
    assert!(chip[1] + chip[3] <= pill[1]);
}

#[test]
fn the_ask_bar_is_centered_at_390_wide() {
    the_bar_is_centered("the_ask_bar_is_centered_at_390_wide", (390.0, 844.0));
}

#[test]
fn the_ask_bar_is_centered_at_the_default_width() {
    the_bar_is_centered("the_ask_bar_is_centered_at_the_default_width", DEFAULT_SIZE);
}

#[test]
fn meeting_grid_is_a_44_strip_640_notes_40_gap_and_300_tasks() {
    if in_parent("meeting_grid_is_a_44_strip_640_notes_40_gap_and_300_tasks") {
        return;
    }
    let mut rt = page_with_tasks((1334.0, 900.0));
    rt.settle_all();
    rt.frame(0.0);
    let rect =
        |rt: &Runtime<State>, key: &str| rt.presented(key).unwrap_or_else(|| panic!("{key}")).0;
    let (grid, strip, main, tasks) = (
        rect(&rt, "meeting-grid"),
        rect(&rt, "meeting-grid-strip"),
        rect(&rt, "meeting-main"),
        rect(&rt, "meeting-tasks-column"),
    );
    assert_eq!(strip[2], 44.0, "strip");
    assert_eq!(main[2], 640.0, "notes column");
    assert_eq!(tasks[2], 300.0, "tasks column");
    assert_eq!(
        strip[0] + strip[2],
        main[0],
        "the strip sits left of the notes"
    );
    assert_eq!(tasks[0] - (main[0] + main[2]), 40.0, "gap");
    // Centred with 16 kept at the right, and nothing past the window.
    let left = grid[0] - nav::width(&rt.state);
    let right = 1334.0 - (grid[0] + grid[2]);
    assert_eq!((grid[2], right - left), (1024.0, 16.0), "grid {grid:?}");
    // The header's title and controls are in the notes column.
    let head = rect(&rt, "meeting-header");
    assert_eq!((head[0], head[2]), (main[0], 640.0), "header");
    assert_eq!(rect(&rt, "ask-pill")[2], 640.0, "Ask bar");
    assert!(rt.find("meeting-scroll-fade").is_none());
    // The transcript sheet rises inside the notes column.
    rt.update(|s, _| s.transcript_open = true);
    rt.frame(0.2);
    rt.settle_all();
    let sheet = rect(&rt, "meeting-transcript-surface");
    assert_eq!((sheet[0], sheet[2]), (main[0], 640.0), "transcript sheet");
}

/// Long notes: the last row ends below where the old 150 fade began, and is still clear of it.
#[test]
fn the_dock_leaves_the_last_notes_row_clear() {
    if in_parent("the_dock_leaves_the_last_notes_row_clear") {
        return;
    }
    let long = || {
        let folder = core::meetings_dir().join(MEETING);
        let point = |kind: &str, text: &str| serde_json::json!({"type": kind, "text": text});
        let summary = serde_json::json!({
            "title": "Beta launch plan",
            "context": ["The team reviewed the new onboarding flow. The launch date and the pricing copy were the two open questions."],
            "topics": [
                {"title": "Launch date", "points": [
                    point("decided", "Launch in the second week of December, if the copy is done."),
                    point("decided", "Hold the launch review on Monday 1 December."),
                    point("fact", "The pricing page copy is still with Priya.")]},
                {"title": "Onboarding flow", "points": [
                    point("decided", "Cut step three down to one screen."),
                    point("fact", "Step three loses the most people in the survey."),
                    point("fact", "Test the shorter step with five customers.")]},
                {"title": "Survey results", "points": [
                    point("fact", "Four in ten people skip the account question."),
                    point("fact", "Whether to ask for the work email later.")]},
            ],
            "verify": ["The survey sample is 48 people, so treat the percentages as rough."],
        });
        std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
    };
    let mut rt = page_with((1334.0, 900.0), long);
    rt.settle_all();
    rt.frame(0.0);
    let rect =
        |rt: &Runtime<State>, key: &str| rt.presented(key).unwrap_or_else(|| panic!("{key}")).0;
    let (last, dock) = (rect(&rt, "verify-0"), rect(&rt, "ask-overlay"));
    let scroll = rect(&rt, "meeting-scroll");
    assert!(
        last[1] + last[3] > scroll[1] + scroll[3] - 150.0,
        "the scene must reach into the old 150 fade"
    );
    assert!(
        last[1] + last[3] <= dock[1],
        "the last notes row {last:?} sits under the dock {dock:?}"
    );
    // The answer adds a panel without introducing a fade.
    rt.update(|s, _| {
        s.saved.messages = ["What did we decide?", "The beta ships on Friday."]
            .iter()
            .zip(["user", "assistant"])
            .map(|(text, role)| Message {
                role: role.into(),
                text: (*text).into(),
                ..Default::default()
            })
            .collect()
    });
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    assert!(rect(&rt, "ask-thread")[3] > 0.0);
    assert!(rt.find("meeting-scroll-fade").is_none());
}

/// The saved meeting at `size` with a date and duration, two people, a project and the English-only
/// status: every item the meta row can show.
fn page_with_every_meta_item(size: (f32, f32)) -> Runtime<State> {
    page_with_meta(size, 1_793_955_600_000, 22, &["Maya", "Tom"], "Atlas 2")
}

fn page_with_meta(
    size: (f32, f32),
    start: i64,
    minutes: u64,
    people: &[&str],
    project: &str,
) -> Runtime<State> {
    let mut rt = page_with(size, || {
        let folder = core::meetings_dir().join(MEETING);
        let state = core::state_dir();
        std::fs::create_dir_all(&state).unwrap();
        for (path, value) in [
            (
                folder.join("manifest.json"),
                serde_json::json!({"status":"complete","created_at_ms":start,"duration_ms":minutes * 60_000}),
            ),
            (
                folder.join("english-summary.json"),
                serde_json::json!({"title":"Beta launch plan","context":[CONTEXT],"people":people}),
            ),
            (
                folder.join("tasks.json"),
                serde_json::json!({"meeting_project":project}),
            ),
            (
                state.join("config.json"),
                serde_json::json!({"version":1,"features":{"projects":true}}),
            ),
            (
                state.join("projects.json"),
                serde_json::json!({"projects":[project]}),
            ),
        ] {
            std::fs::write(path, value.to_string()).unwrap();
        }
    });
    rt.update(|s, _| {
        let deadline = Instant::now() + Duration::from_secs(2);
        while s.areas.refresh_pending {
            let event = s
                .events_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("area worker finished");
            s.apply_event(event);
        }
    });
    rt.settle_all();
    rt.frame(0.0);
    rt
}

#[test]
fn the_meta_facts_show_a_date_alone_a_duration_alone_or_both_with_a_dot_between() {
    if in_parent("the_meta_facts_show_a_date_alone_a_duration_alone_or_both_with_a_dot_between") {
        return;
    }
    let start = 1_793_955_600_000_i64;
    let date = chrono::DateTime::from_timestamp_millis(start)
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%-d %b")
        .to_string();
    let both = serde_json::json!({"status":"complete","created_at_ms":start,"duration_ms":1320000});
    let date_only = serde_json::json!({"status":"complete","created_at_ms":start});
    let duration_only = serde_json::json!({"status":"complete","duration_ms":1320000});
    let cases = [
        (both, format!("Date: {date}. Duration: 22 min"), true),
        (date_only, format!("Date: {date}"), false),
        (duration_only, "Duration: 22 min".to_string(), false),
    ];
    for (manifest, label, dot) in cases {
        let rt = page_with(DEFAULT_SIZE, || {
            let path = core::meetings_dir().join(MEETING).join("manifest.json");
            std::fs::write(path, manifest.to_string()).unwrap();
        });
        let facts = element(&rt, "meeting-facts");
        assert_eq!(facts.label.as_deref(), Some(label.as_str()));
        // The dot sits between a date and a duration, and only then.
        assert_eq!(rt.find("meeting-fact-separator").is_some(), dot, "{label}");
        let shown: Vec<_> = rt
            .elements()
            .into_iter()
            .filter(|e| e.path.starts_with(&format!("{}/", facts.path)))
            .filter_map(|e| e.text)
            .collect();
        let want: Vec<&str> = label
            .trim_start_matches("Date: ")
            .trim_start_matches("Duration: ")
            .split(". Duration: ")
            .collect();
        assert_eq!(shown, want, "{label}");
    }
}

#[test]
fn a_phone_page_has_a_24_title_16_gutters_and_facts_below_the_controls() {
    if in_parent("a_phone_page_has_a_24_title_16_gutters_and_facts_below_the_controls") {
        return;
    }
    let rt = page_with_every_meta_item((390.0, 900.0));
    let project = format!("meeting-project-{MEETING}");
    let more = format!("more-meeting-{MEETING}-button");
    let r = |key: &str| rect_of(&rt, key);
    // The page starts at the sidebar's 1 point divider; the gutter is 16 from there.
    let left = r("meeting-pane")[0] + 16.0;
    // The title is 24 on a 30 line.
    let title = r("meeting-title");
    assert_eq!((title[0], title[3]), (left, 30.0), "title {title:?}");
    // Line one: the switch and the menu at opposite gutters.
    let (view, facts, menu) = (r("meeting-view"), r("meeting-facts"), r(&more));
    assert_eq!(menu[1], view[1], "line one");
    assert_eq!(
        menu[0] + menu[2],
        390.0 - 16.0,
        "the menu is at the right gutter"
    );
    assert_eq!(view[0], left, "the switch is at the left gutter");
    // The facts start 6 below the controls and wrap in reading order, ending with the section
    // rail's slot.
    let (people, chip, status, slot) = (
        r("meeting-people"),
        r(&project),
        r("meeting-status"),
        r("meeting-sections-slot"),
    );
    assert_eq!((facts[0], facts[1]), (left, view[1] + view[3] + 6.0));
    for (before, after) in [
        (facts, people),
        (people, chip),
        (chip, status),
        (status, slot),
    ] {
        assert!(
            after[1] >= before[1] + before[3]
                || (after[1] >= before[1] && after[0] >= before[0] + before[2]),
            "{after:?} follows {before:?}"
        );
    }
    assert_eq!(r("sec-context")[0], left, "notes gutter");
    assert_inside(&rt, 390.0);
    // Every fact stays reachable: each is on the page with its name.
    for key in [
        "meeting-facts",
        "meeting-people",
        "meeting-status",
        &project,
        &more,
    ] {
        assert!(element(&rt, key).label.is_some(), "{key}");
    }
}

#[test]
fn the_tasks_sit_28_below_the_notes_on_a_phone() {
    if in_parent("the_tasks_sit_28_below_the_notes_on_a_phone") {
        return;
    }
    let rt = tasks_page((390.0, 900.0), true);
    let last = rect_of(&rt, "meeting-point-pt-0-1");
    let tasks = rect_of(&rt, "meeting-actions");
    assert_eq!(tasks[1] - (last[1] + last[3]), 28.0, "tasks {tasks:?}");
    assert!(rt.find("meeting-grid").is_none());
}

#[test]
fn the_hidden_time_pill_leaves_the_whole_row_to_the_text_on_a_phone() {
    if in_parent("the_hidden_time_pill_leaves_the_whole_row_to_the_text_on_a_phone") {
        return;
    }
    // Bullet column 20, row padding 6 each side.
    let space = |rt: &Runtime<State>| {
        let row = rect_of(rt, "meeting-point-pt-0-0");
        (
            row,
            rect_of(rt, "meeting-point-text-pt-0-0")[2],
            row[2] - 32.0,
        )
    };
    let rt = page_with_cited_point_at((390.0, 900.0));
    let (row, text, free) = space(&rt);
    assert_eq!(text, free, "the text has the row's whole width");
    // The pill is still 6 from the row's right edge, and the pointer still reaches it.
    let pill = rect_of(&rt, "meeting-source-pt-0-0");
    assert_eq!(
        (pill[3], row[0] + row[2] - (pill[0] + pill[2])),
        (21.0, 6.0)
    );
    // On the wide page the pill is in the row, so the text has less.
    let (_, text, free) = space(&page_with_cited_point());
    assert!(text < free, "wide: {text} of {free}");
}

#[test]
fn the_ask_bar_starts_on_the_notes_edge_wherever_the_strip_is_on() {
    if in_parent("the_ask_bar_starts_on_the_notes_edge_wherever_the_strip_is_on") {
        return;
    }
    // 800 wide with no sidebar, and 1000 wide with it (a page of 751): both are `narrow` and both
    // have room for the strip and the 640 column, so the bar sits on the column's edge.
    for size in [(800.0, 900.0), (1000.0, 900.0)] {
        let rt = page_at(size);
        assert!(rt.find("meeting-grid").is_some(), "{size:?}: no strip");
        let (pill, main) = (
            rect_of(&rt, "meeting-bottom-bar"),
            rect_of(&rt, "meeting-main"),
        );
        assert_eq!(
            pill[0], main[0],
            "{size:?}: the Ask bar is off the notes edge"
        );
        assert_eq!(
            pill[2], main[2],
            "{size:?}: the Ask bar is not the column's width"
        );
    }
}

/// What Hex reports while it records the invented meeting.
fn hex_engine_recording() -> sidenote::hex_engine::EngineState {
    sidenote::hex_engine::EngineState {
        running: true,
        version: "test".into(),
        protocol: 4,
        recording: Some(sidenote::hex_engine::Recording {
            id: Some(MEETING.into()),
            title: None,
            since: None,
            elapsed_seconds: Some(724),
        }),
        meeting_state: "recording".into(),
        dictating: Some(false),
        capture_state: "idle".into(),
        model: sidenote::hex_engine::ModelState {
            id: None,
            name: None,
            language: None,
            installed: None,
            preparing: None,
            downloaded_bytes: None,
        },
        labs: Default::default(),
    }
}

#[test]
fn the_phone_menu_shares_the_switch_row_with_short_and_long_facts() {
    if in_parent("the_phone_menu_shares_the_switch_row_with_short_and_long_facts") {
        return;
    }
    use chrono::TimeZone;

    let more = format!("more-meeting-{MEETING}-button");
    let project_key = format!("meeting-project-{MEETING}");
    let project = "Atlas Partner Operations and Launch Readiness";
    let people = [
        "Maya", "Tom", "Amir", "Asha", "Eli", "Inez", "Jin", "Lea", "Noor", "Ravi", "Rosa", "Zoe",
    ];
    let start = chrono::Local
        .with_ymd_and_hms(2026, 11, 28, 12, 0, 0)
        .unwrap()
        .timestamp_millis();
    for long in [false, true] {
        // The recording rail uses 57 points, leaving pages of 333 and 350.
        for (window, rail) in [
            (390.0, true),
            (340.0, false),
            (407.0, true),
            (360.0, false),
            (364.0, false),
            (365.0, false),
            (380.0, false),
            (390.0, false),
            (420.0, false),
        ] {
            let mut rt = if long {
                page_with_meta((window, 900.0), start, 105, &people, project)
            } else {
                page_with_every_meta_item((window, 900.0))
            };
            if rail {
                rt.update(|s, _| s.hex.status = Some(Ok(hex_engine_recording())));
                rt.frame(0.2);
                rt.settle_all();
            }
            let page = window - if rail { 57.0 } else { 0.0 };
            let (view, menu) = (rect_of(&rt, "meeting-view"), rect_of(&rt, &more));
            assert_eq!(
                menu[1], view[1],
                "page {page}, long facts {long}: the menu is on a line by itself"
            );
            if long {
                let facts = element(&rt, "meeting-facts");
                let people = element(&rt, "meeting-people");
                let project_chip = element(&rt, &project_key);
                assert_eq!(
                    facts.label.as_deref(),
                    Some("Date: 28 Nov. Duration: 105 min")
                );
                assert_eq!(people.label.as_deref(), Some("12 people"));
                assert!(project_chip.label.as_deref().unwrap().contains(project));
                let pane = rect_of(&rt, "meeting-pane");
                for fact in [&facts, &people, &project_chip] {
                    let [x, y, w, h] = fact.rect;
                    assert!(y >= view[1] + view[3], "page {page}: {:?}", fact.label);
                    assert!(
                        w > 0.0
                            && h > 0.0
                            && x >= pane[0]
                            && x + w <= pane[0] + pane[2]
                            && y >= pane[1]
                            && y + h <= pane[1] + pane[3],
                        "page {page}: {:?} {:?} is outside the page {pane:?}",
                        fact.label,
                        fact.rect
                    );
                }
            }
            assert_inside(&rt, window);
        }
    }
}

#[test]
fn the_rail_mic_is_danger_red_not_amber_while_hex_records_on_a_phone() {
    if in_parent("the_rail_mic_is_danger_red_not_amber_while_hex_records_on_a_phone") {
        return;
    }
    let mut rt = page_at((390.0, 900.0));
    assert!(
        rt.find("rail-recording").is_none(),
        "no rail until it records"
    );
    rt.update(|s, _| {
        s.hex.status = Some(Ok(hex_engine_recording()));
    });
    rt.frame(0.2);
    let rail = element(&rt, "rail-recording");
    // Amber is a warning; a live status is danger. The test environment draws the dark theme.
    let p = theme::palette(true);
    let colours: Vec<_> = rt
        .elements()
        .into_iter()
        .filter(|e| e.path.starts_with(&format!("{}/", rail.path)))
        .filter_map(|e| e.text_color)
        .collect();
    assert!(
        colours.contains(&p.danger),
        "the mic is not danger red: {colours:?}"
    );
    assert!(!colours.contains(&p.warn), "the mic is amber");
}

/// Every `meeting-` element lies inside `width`.
fn assert_inside(rt: &Runtime<State>, width: f32) {
    for e in rt.elements() {
        let key = e.key.as_deref().unwrap_or_default();
        if key.starts_with("meeting-") || key.starts_with("more-meeting-") {
            assert!(
                e.rect[0] >= 0.0 && e.rect[0] + e.rect[2] <= width,
                "{key} {:?} runs past the window",
                e.rect
            );
        }
    }
}

#[test]
fn meeting_grid_keeps_its_strip_and_column_below_its_width_with_the_tasks_stacked() {
    if in_parent("meeting_grid_keeps_its_strip_and_column_below_its_width_with_the_tasks_stacked") {
        return;
    }
    // 1024 wide leaves 775 beside the sidebar: less than the 1024 grid plus its 16, but room for
    // the 44 strip and the 640 column with 24 either side (732).
    let mut rt = tasks_page((1024.0, 800.0), true);
    let r = |rt: &Runtime<State>, key: &str| rect_of(rt, key);
    let (grid, strip, main) = (
        r(&rt, "meeting-grid"),
        r(&rt, "meeting-grid-strip"),
        r(&rt, "meeting-main"),
    );
    assert_eq!((grid[2], strip[2], main[2]), (684.0, 44.0, 640.0));
    assert_eq!(
        strip[0] + strip[2],
        main[0],
        "the strip sits left of the notes"
    );
    // Centred in the page beside the sidebar.
    let (left, right) = (
        grid[0] - nav::width(&rt.state),
        1024.0 - (grid[0] + grid[2]),
    );
    assert_eq!(left, right, "centred in the page");
    // The header, the notes and the tasks all start on the notes edge, and the menu ends on the
    // notes' right edge, not past it.
    let header = r(&rt, "meeting-header");
    assert_eq!((header[0], header[2]), (main[0], 640.0), "header");
    let menu = r(&rt, &format!("more-meeting-{MEETING}-button"));
    assert_eq!(
        menu[0] + menu[2],
        main[0] + main[2],
        "the menu is at the notes' right edge"
    );
    assert_eq!(r(&rt, "sec-context")[0], main[0], "notes");
    assert!(rt.find("meeting-tasks-column").is_none());
    let (last, tasks) = (r(&rt, "meeting-point-pt-0-1"), r(&rt, "meeting-actions"));
    assert_eq!(
        tasks[1] - (last[1] + last[3]),
        28.0,
        "tasks below the notes"
    );
    // The stacked rows hang 8 either side of the column, inside the scroll area.
    let task = r(&rt, "meeting-task-0");
    assert_eq!(
        (task[0], task[2]),
        (main[0] - 8.0, 656.0),
        "task row {task:?}"
    );
    // The Ask bar and the transcript sheet start on the notes edge too.
    assert_eq!(r(&rt, "meeting-bottom-bar")[0], main[0], "Ask bar");
    rt.update(|s, _| s.transcript_open = true);
    rt.frame(0.2);
    rt.settle_all();
    let sheet = r(&rt, "meeting-transcript-surface");
    assert_eq!((sheet[0], sheet[2]), (main[0], 640.0), "transcript sheet");
    assert_inside(&rt, 1024.0);
}

#[test]
fn meeting_page_below_the_strip_width_fills_the_page_and_never_overflows() {
    if in_parent("meeting_page_below_the_strip_width_fills_the_page_and_never_overflows") {
        return;
    }
    // 900 wide leaves 651 beside the sidebar: under the 732 the strip and column need.
    let mut rt = tasks_page((900.0, 800.0), true);
    assert!(rt.find("meeting-grid").is_none());
    let notes = rect_of(&rt, "sec-context")[0];
    let left = nav::width(&rt.state);
    assert_eq!(notes, left + 24.0, "24 gutter");
    assert_eq!(
        rect_of(&rt, "meeting-title")[0],
        notes,
        "title on the notes edge"
    );
    rt.update(|s, _| s.transcript_open = true);
    rt.frame(0.2);
    rt.settle_all();
    assert_eq!(
        rect_of(&rt, "meeting-transcript-surface")[0],
        notes,
        "transcript sheet on the notes edge"
    );
    assert_inside(&rt, 900.0);
}

/// Writes a summary with this `made_by` record into `folder`.
fn stamped_summary(folder: &std::path::Path, made_by: Option<serde_json::Value>) {
    let path = folder.join("english-summary.json");
    let mut summary: serde_json::Value = match std::fs::read(&path) {
        Ok(raw) => serde_json::from_slice(&raw).unwrap(),
        Err(_) => serde_json::json!({"title": "Beta launch plan", "context": [CONTEXT]}),
    };
    if let Some(by) = made_by {
        summary["made_by"] = by;
    }
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(path, summary.to_string()).unwrap();
}

/// Whether the saved meeting's Notes view says the transcript was English-only.
fn shows_english_only_note(made_by: Option<serde_json::Value>, stored: impl FnOnce()) -> bool {
    let mut rt = page_with(DEFAULT_SIZE, || {
        stamped_summary(&core::meetings_dir().join(MEETING), made_by);
        stored();
    });
    if rt.find("meeting-status").is_some() {
        tap(&mut rt, "meeting-status");
    }
    rt.find("meeting-english-only").is_some()
}

fn tap(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.2);
}

fn page_with_language_fallback() -> Runtime<State> {
    page_with((390.0, 800.0), || {
        let dir = core::state_dir().join("local/notes-jobs");
        std::fs::create_dir_all(&dir).unwrap();
        let job = serde_json::json!({
            "state": "done", "fallback": false, "tasks_failed": true,
            "transcript_revision": "a".repeat(64),
            "routing_fallbacks": [{"reason": {"cpu_language_models_missing": {"ready": 2}}}],
        });
        std::fs::write(dir.join(format!("{MEETING}.json")), job.to_string()).unwrap();
    })
}

#[test]
fn meeting_status_keeps_language_facts_and_the_settings_link_inside_the_popover() {
    if in_parent("meeting_status_keeps_language_facts_and_the_settings_link_inside_the_popover") {
        return;
    }
    let mut rt = page_with_language_fallback();
    let check = |rt: &Runtime<State>, open| {
        let elements = rt.elements();
        let texts: Vec<_> = elements.iter().filter_map(|e| e.text.as_deref()).collect();
        assert_eq!(
            texts.contains(&"English-only transcript. Hindi or Telugu parts may be missing."),
            open
        );
        assert_eq!(texts.contains(&"Hindi and Telugu models are not installed yet; download them once in Settings, Models (about 567 MB). 2 of 4 ready. Notes use the captured Hex transcript."), open);
    };
    check(&rt, false);
    tap(&mut rt, "meeting-status");
    check(&rt, true);
    tap(&mut rt, "meeting-status-settings");
    assert!(rt.state.hex_open);
    assert!(matches!(rt.state.hex.section, hex::Section::MeetingModels));
}

#[test]
fn meeting_language_status_has_no_task_sorting_title() {
    if in_parent("meeting_language_status_has_no_task_sorting_title") {
        return;
    }
    let rt = page_with_language_fallback();
    let elements = rt.elements();
    let texts: Vec<_> = elements.iter().filter_map(|e| e.text.as_deref()).collect();
    assert!(!texts.contains(&"Tasks not added to a project"));
    assert!(texts.contains(&"Not added to projects yet."));
}

/// A finished-notes job record for the invented meeting: "done", with these two flags.
fn write_done_job(fallback: bool, tasks_failed: bool) {
    let dir = core::state_dir().join("local/notes-jobs");
    std::fs::create_dir_all(&dir).unwrap();
    let job = serde_json::json!({
        "state": "done", "fallback": fallback, "tasks_failed": tasks_failed,
        "transcript_revision": "a".repeat(64), "routing_fallbacks": [],
    });
    std::fs::write(dir.join(format!("{MEETING}.json")), job.to_string()).unwrap();
}

#[test]
fn a_tabbed_meeting_names_the_tasks_notice_with_a_heading() {
    if in_parent("a_tabbed_meeting_names_the_tasks_notice_with_a_heading") {
        return;
    }
    let mut rt = page_with(DEFAULT_SIZE, || write_done_job(false, true));
    rt.update(|s, _| s.meeting_ui.tabs = true);
    rt.settle_all();
    rt.frame(0.0);
    // The heading's text is in a child of the keyed element.
    let root = format!("{}/", element(&rt, "sec-todo").path);
    let elements = rt.elements();
    assert!(
        elements
            .iter()
            .any(|e| e.path.starts_with(&root) && e.text.as_deref() == Some("Tasks"))
    );
    let texts: Vec<_> = elements.iter().filter_map(|e| e.text.as_deref()).collect();
    assert!(texts.contains(&"Not added to projects yet."));
}

#[test]
fn a_summary_stamped_english_only_shows_the_note() {
    if in_parent("a_summary_stamped_english_only_shows_the_note") {
        return;
    }
    let by = serde_json::json!({"helper": "claude", "language_passes": "english_only"});
    assert!(shows_english_only_note(Some(by), || {}));
}

#[test]
fn a_summary_stamped_routed_shows_nothing() {
    if in_parent("a_summary_stamped_routed_shows_nothing") {
        return;
    }
    let by = serde_json::json!({"helper": "claude", "language_passes": "routed"});
    assert!(!shows_english_only_note(Some(by), || {}));
}

#[test]
fn a_summary_with_no_language_record_shows_the_note() {
    if in_parent("a_summary_with_no_language_record_shows_the_note") {
        return;
    }
    // An older summary: it has a `made_by` record, but no word.
    let by = serde_json::json!({"helper": "claude", "model": "claude-sonnet-5-5"});
    assert!(shows_english_only_note(Some(by), || {}));
}

#[test]
fn the_notes_job_record_cannot_hide_the_note() {
    if in_parent("the_notes_job_record_cannot_hide_the_note") {
        return;
    }
    // The job says it pinned a routed revision with no fallback; the summary says English only.
    let by = serde_json::json!({"language_passes": "english_only"});
    assert!(shows_english_only_note(Some(by), || {
        let dir = core::state_dir().join("local/notes-jobs");
        std::fs::create_dir_all(&dir).unwrap();
        let job = serde_json::json!({
            "state": "done", "fallback": false, "tasks_failed": false, "attempts": 1,
            "transcript_revision": "a".repeat(64), "routing_fallbacks": [],
        });
        std::fs::write(dir.join(format!("{MEETING}.json")), job.to_string()).unwrap();
    }));
}

#[test]
fn a_notes_banner_under_the_last_row_keeps_clear_of_it() {
    if in_parent("a_notes_banner_under_the_last_row_keeps_clear_of_it") {
        return;
    }
    let rt = page_with(DEFAULT_SIZE, || write_done_job(true, false));
    let (row, banner) = (
        rect_of(&rt, "meeting-point-pt-0-0"),
        rect_of(&rt, "notes-state-banner"),
    );
    assert_eq!(banner[1] - (row[1] + row[3]), 16.0, "gap to the last row");
}

#[test]
fn a_joined_meeting_shows_its_own_summarys_word() {
    if in_parent("a_joined_meeting_shows_its_own_summarys_word") {
        return;
    }
    // The first recording's own folder says routed; the joined meeting's notes say English only.
    let routed = serde_json::json!({"language_passes": "routed"});
    let english = serde_json::json!({"language_passes": "english_only"});
    assert!(shows_english_only_note(Some(routed), || {
        core::meeting_parts::Meeting::begin(&core::state_dir(), MEETING, 1_800_000_000_000)
            .unwrap();
        let joined = core::meeting_parts::inputs(&core::state_dir()).join(MEETING);
        std::fs::create_dir_all(&joined).unwrap();
        std::fs::write(joined.join("manifest.json"), r#"{"status":"complete"}"#).unwrap();
        stamped_summary(&joined, Some(english));
    }));
}

#[test]
fn deletion_cancels_delayed_ask_text_and_answers_in_all_scopes() {
    if in_parent("deletion_cancels_delayed_ask_text_and_answers_in_all_scopes") {
        return;
    }
    use sidenote::core::{
        meeting_delete::{
            DeleteError, Eligibility, MoveEvent, Phase, Storage, WriteStage, read_visibility,
            suppressed_ids,
        },
        memory::MemoryStore,
    };

    let mut rt = page_with(DEFAULT_SIZE, || {
        let root = core::state_dir();
        let config = serde_json::json!({"version":1,"features":{"people":true,"memory":true}});
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("config.json"), config.to_string()).unwrap();
        let mut prefs = core::UiPrefs::default();
        core::people::initialize(&mut prefs, &[]);
        prefs.save(&root).unwrap();
        let manifest =
            serde_json::json!({"status":"complete","created_at_ms":prefs.people_since_ms});
        let folder = core::meetings_dir().join(MEETING);
        std::fs::write(folder.join("manifest.json"), manifest.to_string()).unwrap();
        core::companies::Companies::save_company(
            &root,
            core::companies::Company {
                id: "fixture".into(),
                name: "Synthetic company".into(),
                ..Default::default()
            },
        )
        .unwrap();
        core::companies::Companies::link(&root, "Synthetic Person", Some("fixture".into()), None)
            .unwrap();
    });
    let state = &mut rt.state;
    let store = Storage::new(&core::meetings_dir(), &state.store.root).unwrap();
    state.meetings[0].summary.people = vec!["Synthetic Person".into()];
    let memory = MemoryStore::new(state.store.root.clone());
    let preview = memory.preview_migration().unwrap();
    memory
        .accept_migration(&preview.source_fingerprint)
        .unwrap();
    const MEMORY: &str = "Synthetic saved claim";
    let mut proposal: core::memory::MemoryProposal = serde_json::from_value(serde_json::json!({
        "op":"add", "text":MEMORY, "rationale":"Synthetic source",
        "sources":[{"type":"meeting", "reference":format!("[[{MEETING}@00:01]]")}]
    }))
    .unwrap();
    proposal.id = core::memory::proposal_id(&proposal);
    memory
        .stage_proposals(std::slice::from_ref(&proposal))
        .unwrap();
    memory.accept_proposal(&proposal.id).unwrap();
    state.memory.records = memory.load().unwrap().records;
    for scope in 0..3 {
        state.memory_open = scope == 1;
        state.selected = (scope == 0).then(|| MEETING.to_owned());
        state.companies.open = scope == 2;
        state.companies.selected = (scope == 2).then(|| "fixture".to_owned());
        let id = state.ask_key();
        let context = state.all_context(&Default::default()).text;
        assert!(context.contains(CONTEXT), "scope {scope}");
        if state.memory_open {
            assert!(context.contains(MEMORY));
        }
        for first in [
            AskProgress::Text {
                text: "Late text".into(),
            },
            AskProgress::Finished,
        ] {
            let mut run = AskRun::stand_in("fixture", &[], 0);
            run.live = true;
            run.visibility = *read_visibility();
            let cancel = run.cancel.clone();
            let (tx, rx) = mpsc::channel();
            tx.send(Ok("Late answer".into())).unwrap();
            run.rx = rx;
            state.asks.insert(id.clone(), run);

            let fail_after_publish = matches!(&first, AskProgress::Finished);
            let prior = core::mutation_generation();
            let result = store.delete(
                MEETING,
                "Synthetic".into(),
                1,
                |_| Ok(Eligibility::Ready),
                |event| {
                    if fail_after_publish && event == MoveEvent::Suppression(WriteStage::Published)
                    {
                        Err(std::io::Error::other("post-publication failure"))
                    } else {
                        Ok(())
                    }
                },
            );
            let receipt = match result {
                Ok(id) if !fail_after_publish => id,
                Err(DeleteError::Partial {
                    transaction,
                    source,
                }) if fail_after_publish => {
                    assert_eq!(source.to_string(), "post-publication failure");
                    transaction
                }
                _ => panic!("unexpected deletion result"),
            };
            assert_ne!(core::mutation_generation(), prior);
            let progress = |event| Event::Progress {
                id: id.clone(),
                generation: 0,
                event,
            };
            state.apply_event(progress(first));
            assert!(cancel.stopped());
            assert!(!state.asks.contains_key(&id));
            state.apply_event(progress(AskProgress::Finished));
            assert!(state.store.load(&id).unwrap().messages.is_empty());
            let suppressed = suppressed_ids(&state.store.root).unwrap();
            assert!(suppressed.contains(MEETING));
            let context = state.all_context(&suppressed).text;
            assert!(!context.contains(CONTEXT));
            assert!(!context.contains(MEMORY));
            assert_eq!(memory.load().unwrap().records.len(), 1);
            assert_eq!(store.recover(&receipt).unwrap(), Phase::Deleted);
            store.restore(&receipt).unwrap();
            if state.memory_open {
                let visible = suppressed_ids(&state.store.root).unwrap();
                assert!(state.all_context(&visible).text.contains(MEMORY));
            }
        }
    }
}

#[cfg(windows)]
#[test]
fn suppression_failure_during_ui_apply_retries_unchanged_catalog() {
    if in_parent("suppression_failure_during_ui_apply_retries_unchanged_catalog") {
        return;
    }
    use std::{os::windows::fs::OpenOptionsExt, sync::atomic::Ordering};

    let mut rt = page();
    let state = &mut rt.state;
    let path = state.store.root.join("local/deleted-meeting-ids.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{}").unwrap();
    state.config.features.memory = false;
    state.config.features.daily_check = false;
    let options = state.refresh_options();
    let snapshot = || {
        Event::Refreshed(refresh::Result {
            generation: core::mutation_generation(),
            options,
            data: Some(Box::new(refresh::Data::read(options))),
        })
    };
    let (handle, dirty) = refresh::observing_handle();
    state.scan_tx = Some(handle);
    let ready = snapshot();
    let before = refresh::metadata();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    state.apply_event(ready);
    assert!(state.meetings.is_empty());
    assert!(dirty.load(Ordering::Acquire));
    drop(held);
    assert!(before == refresh::metadata());
    state.apply_event(snapshot());
    assert_eq!(state.meetings[0].id, MEETING);
}

#[test]
fn live_transcript_stays_above_the_dock_at_short_heights() {
    if in_parent("live_transcript_stays_above_the_dock_at_short_heights") {
        return;
    }
    for width in [1334.0, 390.0] {
        for height in [600.0, 720.0, 900.0] {
            let mut rt = page_at((width, height));
            for phase in ["transcript", "paused"] {
                rt.update(|s, _| {
                    s.selected = None;
                    super::live::parts_scene(s, phase);
                    scene_agents(s, ALL_AGENTS);
                    s.saved.messages.clear();
                    s.live.as_mut().unwrap().panel_open = false;
                });
                rt.settle_all();
                rt.frame(0.0);
                let closed = rect_of(&rt, "ask-pill");
                rt.update(|s, _| s.live.as_mut().unwrap().panel_open = true);
                for answer in [false, true] {
                    if answer {
                        rt.update(|s, _| {
                            s.saved.messages.push(Message {
                                role: "assistant".into(),
                                text: "An answer.\n\n".repeat(40),
                                ..Default::default()
                            })
                        });
                    }
                    rt.settle_all();
                    rt.frame(0.0);
                    let panel = rect_of(&rt, "live-panel");
                    let dock = rect_of(&rt, "ask-controls");
                    let pill = rect_of(&rt, "ask-pill");
                    assert!(
                        panel[1] + panel[3] <= dock[1] + 0.5,
                        "{width}×{height} {phase}, answer {answer}"
                    );
                    assert_eq!((panel[0], panel[2]), (pill[0], pill[2]));
                    assert_eq!(closed, pill);
                }
            }
        }
    }
}
