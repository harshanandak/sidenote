use super::{
    State, ask, cards, list, roles,
    theme::{PAGE_TITLE, Palette},
    widgets::{notice_bar, scroll},
};
use chrono::{Local, NaiveDate, Timelike};
use rune::{El, col, row};
use rune_kit::Kit;
use sidenote::core::{Meeting, TaskItem, TaskStatus, Teams, people};
use std::collections::HashSet;

fn greeting(hour: u32, owner: &str) -> String {
    let hello = match hour {
        0..12 => "Good morning",
        12..18 => "Good afternoon",
        _ => "Good evening",
    };
    match owner.split_whitespace().next() {
        Some(first) => format!("{hello}, {first}"),
        None => hello.into(),
    }
}

fn recent_meetings(meetings: &[Meeting], today: NaiveDate) -> impl Iterator<Item = &Meeting> {
    let yesterday = today.pred_opt().unwrap_or(today);
    meetings
        .iter()
        .filter(move |m| m.started.is_some_and(|at| at.date_naive() >= yesterday))
}

fn needs_you(task: &TaskItem, owner: &str, teams: &Teams) -> bool {
    let owned = |name| !owner.is_empty() && people::key(&teams.owner_name(name)) == owner;
    !task.done && (task.mine == Some(true) || task.owner.as_deref().is_some_and(owned))
}

// The profile name the sidebar shows; engine_settings.owner_name is only a legacy import snapshot.
fn your_name(s: &State) -> &str {
    s.ai_notes
        .settings
        .as_ref()
        .map_or("", |n| n.your_name.trim())
}

fn task_counts(s: &State, today: NaiveDate) -> (usize, usize) {
    let teams = &s.tasks.teams;
    let owner = people::key(&teams.owner_name(your_name(s)));
    s.tasks
        .all
        .iter()
        .filter(|task| needs_you(task, &owner, teams))
        .fold((0, 0), |(open, late), task| {
            (open + 1, late + usize::from(task.is_overdue(today)))
        })
}

fn meetings_label(n: usize) -> String {
    format!("{n} meetings since yesterday")
}

fn tile(key: &str, title: String, subline: Option<String>, p: Palette) -> El {
    cards::panel(p).key(key).grow(1.0).child(
        col()
            .padding(16.0)
            .gap(4.0)
            .child(roles::SECTION.wrap(title, p.ink))
            .children(subline.map(|line| roles::META.wrap(line, p.soft))),
    )
}

fn brief_group<'a>(
    label: &str,
    empty: &str,
    rows: impl Iterator<Item = &'a str>,
    p: Palette,
) -> El {
    let mut seen = HashSet::new();
    let rows: Vec<_> = rows
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty() && seen.insert(text.clone()))
        .map(|text| roles::READ.wrap(text, p.ink))
        .collect();
    col()
        .gap(4.0)
        .child(roles::GROUP.wrap(label, p.soft))
        .children(rows.is_empty().then(|| roles::SMALL.wrap(empty, p.soft)))
        .children(rows)
}

fn daily_brief(meetings: &[&Meeting], summary: Option<&str>, p: Palette) -> El {
    let points = |kind: &'static str| {
        meetings
            .iter()
            .flat_map(|meeting| meeting.points())
            .filter(move |point| point.kind == kind)
            .map(|point| point.text.as_str())
    };
    let questions = meetings
        .iter()
        .flat_map(|meeting| &meeting.summary.to_settle)
        .map(|item| item.question.as_str());
    cards::panel(p).key("home-daily-brief").child(
        col()
            .padding(16.0)
            .gap(12.0)
            .child(roles::SECTION.wrap(summary.unwrap_or("What changed since yesterday"), p.ink))
            .child(brief_group("Decided", "No decisions", points("decided"), p))
            .child(brief_group(
                "Still open",
                "No open questions",
                points("open").chain(questions),
                p,
            ))
            .child(roles::META.wrap(format!("From {}", meetings_label(meetings.len())), p.soft)),
    )
}

fn task_sections<'a>(
    tasks: &'a [TaskItem],
    owner: &str,
    teams: &Teams,
    today: NaiveDate,
) -> (Vec<&'a TaskItem>, Vec<&'a TaskItem>) {
    let owner = people::key(&teams.owner_name(owner));
    let mut needs = Vec::new();
    let mut waiting = Vec::new();
    for task in tasks.iter().filter(|task| !task.done) {
        let mine = needs_you(task, &owner, teams);
        if task.status == TaskStatus::Waiting
            || task.waiting_on.is_some()
            || task.waiting_label.is_some()
            || (!mine && task.owner.is_some())
        {
            waiting.push(task);
        } else if mine {
            needs.push(task);
        }
    }
    needs.sort_by_key(|task| {
        if task.is_overdue(today) {
            0
        } else if task.due_date == Some(today) {
            1
        } else {
            2
        }
    });
    (needs, waiting)
}

fn task_card(key: &str, title: &str, tasks: &[&TaskItem], today: NaiveDate, p: Palette) -> El {
    let rows = tasks.iter().map(|task| {
        let due = task.due_date.map(|date| {
            let label = if task.is_overdue(today) {
                "Late"
            } else {
                "Due"
            };
            format!("{label} {}", date.format("%-d %b"))
        });
        let waiting = task.waiting_label.as_deref().or_else(|| {
            task.waiting_on
                .as_ref()
                .and_then(|value| value.text.as_deref())
        });
        let meta = [
            task.owner.as_deref(),
            due.as_deref().or(task.due.as_deref()),
            waiting,
            Some(task.meeting_title.as_str()),
        ]
        .into_iter()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
        col()
            .gap(2.0)
            .child(roles::TITLE.wrap(&task.title, p.ink))
            .child(roles::META.wrap(meta, p.soft))
    });
    cards::panel(p).key(key).child(
        col()
            .padding(16.0)
            .gap(12.0)
            .child(roles::SECTION.wrap(title, p.ink))
            .children(
                tasks
                    .is_empty()
                    .then(|| roles::SMALL.wrap("No tasks", p.soft)),
            )
            .children(rows),
    )
}

fn document(s: &State, p: Palette, narrow: bool, today: NaiveDate, hour: u32) -> El {
    let owner = your_name(s);
    let (open, late) = task_counts(s, today);
    let recent: Vec<_> = recent_meetings(&s.meetings, today).collect();
    let summary = s.check.day_summaries.get(&today).or_else(|| {
        today
            .pred_opt()
            .and_then(|day| s.check.day_summaries.get(&day))
    });
    let (needs, waiting) = task_sections(&s.tasks.all, owner, &s.tasks.teams, today);
    let tiles = if narrow { col() } else { row() };
    let hello = list::text_title(&greeting(hour, owner), PAGE_TITLE, p).key("home-greeting");
    let date = roles::META.wrap(today.format("%A %-d %B").to_string(), p.soft);
    // Task tiles and cards follow the Tasks feature switch, like the sidebar row.
    let on = s.config.features.tasks;
    let tasks = on.then(|| {
        tile(
            "home-tasks-tile",
            format!("{open} tasks need you"),
            Some(format!("{late} late")),
            p,
        )
    });
    let meetings = tile("home-meetings-tile", meetings_label(recent.len()), None, p);
    col()
        .w_full()
        .max_w(860.0)
        .px(if narrow { 16.0 } else { 40.0 })
        .py(24.0)
        .gap(16.0)
        .child(col().gap(4.0).child(hello).child(date))
        .child(tiles.gap(12.0).children(tasks).child(meetings))
        .child(daily_brief(&recent, summary.map(String::as_str), p))
        .children(on.then(|| task_card("home-needs-you", "Needs you", &needs, today, p)))
        .children(on.then(|| task_card("home-waiting", "Waiting on others", &waiting, today, p)))
}

pub(super) fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let now = Local::now();
    let doc = document(s, p, narrow, now.date_naive(), now.hour());
    let body = col().w_full().items_center().child(doc);
    col()
        .key("meeting-pane")
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .children(s.notice.as_deref().map(|note| notice_bar(note, p)))
        .child(scroll(body, "home-scroll"))
        .child(ask::overlay(s, k, p, narrow, layer))
}

#[cfg(test)]
mod tests {
    use super::super::{nav::Page, theme};
    use super::*;

    fn profile(name: &str) -> sidenote::core::notes::NotesSettings {
        sidenote::core::notes::NotesSettings {
            your_name: name.into(),
            ..Default::default()
        }
    }

    const TODAY: NaiveDate = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();

    fn meeting(root: &std::path::Path, date: NaiveDate, summary: &str) -> Meeting {
        let dir = root.join(date.to_string());
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("english-summary.json"), summary).unwrap();
        let mut meeting = Meeting::load(&dir).unwrap();
        let noon = date.and_hms_opt(12, 0, 0).unwrap();
        meeting.started = noon.and_local_timezone(Local).single();
        assert!(meeting.started.is_some());
        meeting
    }

    #[test]
    fn tiles_count_recent_meetings_and_open_tasks_that_need_you() {
        let root = tempfile::tempdir().unwrap();
        let summary = r#"{"title":"Workshop","todo":[
            {"task":"Draft notes","due":"2026-11-24"},
            {"task":"Send agenda","owner":"CASEY MORGAN","due":"2026-11-25"},
            {"task":"File notes","owner":"Casey Morgan","due":"2026-11-24"},
            {"task":"Print agendas","owner":"Lee","due":"2026-11-24"},
            {"task":"Reserve room","owner":"Casey Morgan"}
        ]}"#;
        let meetings =
            [0, 1, 2].map(|days| meeting(root.path(), TODAY - chrono::Days::new(days), summary));
        assert_eq!(
            meetings_label(recent_meetings(&meetings, TODAY).count()),
            "2 meetings since yesterday"
        );
        let mut tasks = sidenote::core::build_tasks(
            &meetings[..1],
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        tasks[0].mine = Some(true);
        tasks[1].mine = Some(false);
        tasks[2].done = true;
        let mut s = State::new();
        s.ai_notes.settings = Some(profile("Casey Morgan"));
        s.tasks.all = tasks;
        assert!(s.tasks.items.is_empty());
        assert_eq!(task_counts(&s, TODAY), (3, 1));
    }

    #[test]
    fn helpers_opened_from_home_offer_summary_setup() {
        let mut s = State::new();
        s.ai_notes.scene = true;
        s.ai_notes.settings = Some(Default::default());
        s.open_ai_notes();
        assert!(super::super::ai_notes::setup_card(&s, theme::palette(false)).is_some());
    }

    #[test]
    fn brief_and_tasks_use_recent_meetings_and_order_work_that_needs_you() {
        use rune::{Env, Runtime, text::TextSystem};

        let root = tempfile::tempdir().unwrap();
        let today = TODAY;
        let mut state = State::new();
        state.ai_notes.settings = Some(profile("Casey Morgan"));
        state.meetings = vec![
            meeting(
                root.path(),
                today,
                r#"{"title":"Workshop","topics":[{"points":[
                {"type":"decided","text":"Ship the sample."},
                {"type":"open","text":"Who reviews the sample?"}
            ]}],"to_settle":[{"question":"Who reviews  the sample?"}]}"#,
            ),
            meeting(
                root.path(),
                today - chrono::Days::new(1),
                r#"{"title":"Planning","topics":[{"points":[
                {"type":"decided","text":"Ship the sample."}
            ]}]}"#,
            ),
            meeting(
                root.path(),
                today - chrono::Days::new(2),
                r#"{"title":"Archive","topics":[{"points":[
                {"type":"decided","text":"Use the old sample."}
            ]}]}"#,
            ),
        ];
        state.meetings[0].summary.todo = serde_json::from_str(
            r#"[
            {"task":"Future sample task","owner":"Casey Morgan"},
            {"task":"External sample task","owner":"Lee"},
            {"task":"Today sample task","owner":"Casey Morgan"},
            {"task":"Late sample task","owner":"Casey Morgan"},
            {"task":"Waiting sample task","owner":"Casey Morgan","waiting_on":{"text":"Sign-off"}}
        ]"#,
        )
        .unwrap();
        state.tasks.all = sidenote::core::build_tasks(
            &state.meetings,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        state.tasks.all[2].due_date = Some(today);
        state.tasks.all[3].due_date = today.pred_opt();

        let mut rt = Runtime::with_text(
            state,
            |s, _| document(s, theme::palette(false), false, TODAY, 9),
            (1200.0, 1200.0),
            Env::default(),
            TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
        );
        rt.frame(0.0);
        let elements = rt.elements();
        let text: Vec<_> = elements
            .iter()
            .filter_map(|el| el.text.as_deref())
            .collect();
        for row in [
            "4 tasks need you",
            "Decided",
            "Ship the sample.",
            "Still open",
            "Who reviews the sample?",
            "What changed since yesterday",
            "From 2 meetings since yesterday",
        ] {
            assert_eq!(
                text.iter().filter(|&&value| value == row).count(),
                1,
                "{row}"
            );
        }
        for excluded in ["Changed", "Conflict", "Use the old sample."] {
            assert!(!text.contains(&excluded), "{excluded}");
        }
        let position = |value| text.iter().position(|&text| text == value).unwrap();
        let order = [
            "Needs you",
            "Late sample task",
            "Today sample task",
            "Future sample task",
            "Waiting on others",
            "External sample task",
            "Waiting sample task",
        ];
        assert!(
            order
                .windows(2)
                .all(|pair| position(pair[0]) < position(pair[1]))
        );

        rt.update(|state, _| {
            state
                .check
                .day_summaries
                .insert(today.pred_opt().unwrap(), "Saved sample brief".into());
        });
        rt.frame(0.0);
        assert!(
            rt.elements()
                .iter()
                .any(|el| el.text.as_deref() == Some("Saved sample brief"))
        );
    }

    #[test]
    fn startup_home_and_meeting_breadcrumb_route() {
        use super::super::{nav::Filter, tasks::View, view};
        use rune::{Env, Runtime, text::TextSystem};
        crate::test_paths::isolate();
        let mut s = State::new();
        // Without Hex (as on Windows CI) startup opens Hex setup over the Home default.
        s.hex_open = false;
        assert!(s.page() == Page::Home);
        s.sidebar_open.set(true);
        let mut rt = Runtime::with_text(
            s,
            view,
            (1200.0, 900.0),
            Env::new(false, 1.0),
            TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
        );
        rt.update(|s, _| s.notice = Some("Recover synthetic settled changes".into()));
        rt.settle_all();
        rt.frame(0.0);
        let home = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("nav-home"))
            .unwrap();
        assert_eq!(
            home.fill,
            theme::sidebar_palette(false, theme::palette(false)).sel
        );
        assert!(rt.find("notice-dismiss").is_some());
        for key in ["home-greeting", "home-tasks-tile", "home-meetings-tile"] {
            assert!(rt.find(key).is_some(), "missing {key}");
        }
        rt.update(|s, _| s.config.features.tasks = false);
        rt.settle_all();
        rt.frame(0.0);
        for key in ["home-tasks-tile", "home-needs-you", "home-waiting"] {
            assert!(rt.find(key).is_none(), "{key} shown with Tasks off");
        }
        assert!(rt.find("home-meetings-tile").is_some());
        rt.update(|s, _| s.config.features.tasks = true);

        let root = tempfile::tempdir().unwrap();
        let sample = meeting(root.path(), TODAY, r#"{"title":"Workshop"}"#);
        rt.update(|s, tx| {
            s.go_meetings();
            assert!(s.page() == Page::Meetings);
            let id = sample.id.clone();
            s.meetings.push(sample);
            for origin in [View::Home, View::Meetings] {
                s.view = origin;
                s.open_citation(&id, "sample-answer".into(), tx);
                s.go_back(tx);
                assert_eq!(s.view, origin);
                assert_eq!(s.pending_thread.as_ref().unwrap().key, "sample-answer");
            }
            s.filter = Filter::Peers;
            s.select(&id, tx);
        });
        rt.settle_all();
        rt.frame(0.0);
        let (x, y) = rt.find("meeting-breadcrumb-parent").unwrap();
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.2);
        assert!(rt.state.filter == Filter::Peers);
        assert!(rt.state.page() == Page::Meetings);
        assert!(rt.state.selected.is_none());
    }
}
