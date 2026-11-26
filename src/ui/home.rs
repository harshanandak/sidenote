use super::{
    State, ask, cards, list, roles,
    theme::{PAGE_TITLE, Palette},
    widgets::{notice_bar, scroll},
};
use chrono::{Local, NaiveDate, Timelike};
use rune::{El, col, row};
use rune_kit::Kit;
use sidenote::core::{Meeting, TaskItem, Teams, people};

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

fn task_counts(s: &State, today: NaiveDate) -> (usize, usize) {
    let teams = &s.tasks.teams;
    let owner = people::key(&teams.owner_name(&s.config.engine_settings.owner_name));
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

pub(super) fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let now = Local::now();
    let today = now.date_naive();
    let owner = &s.config.engine_settings.owner_name;
    let (open, late) = task_counts(s, today);
    let meetings = meetings_label(recent_meetings(&s.meetings, today).count());
    let tiles = if narrow { col() } else { row() };
    let hello = list::text_title(&greeting(now.hour(), owner), PAGE_TITLE, p).key("home-greeting");
    let date = roles::META.wrap(today.format("%A %-d %B").to_string(), p.soft);
    let tasks = tile(
        "home-tasks-tile",
        format!("{open} tasks need you"),
        Some(format!("{late} late")),
        p,
    );
    let meetings = tile("home-meetings-tile", meetings, None, p);
    let doc = col()
        .w_full()
        .max_w(860.0)
        .px(if narrow { 16.0 } else { 40.0 })
        .py(24.0)
        .gap(16.0)
        .child(col().gap(4.0).child(hello).child(date))
        .child(tiles.gap(12.0).child(tasks).child(meetings));
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
        s.config.engine_settings.owner_name = "Casey Morgan".into();
        s.tasks.all = tasks;
        assert!(s.tasks.items.is_empty());
        assert_eq!(task_counts(&s, TODAY), (3, 1));
    }

    #[test]
    fn startup_home_and_meeting_breadcrumb_route() {
        use super::super::{nav::Filter, tasks::View, view};
        use rune::{Env, Runtime, text::TextSystem};

        let s = State::new();
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
        rt.frame(0.0);
        let (x, y) = rt.find("meeting-breadcrumb-parent").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.2);
        assert!(rt.state.filter == Filter::Peers);
        assert!(rt.state.page() == Page::Meetings);
        assert!(rt.state.selected.is_none());
    }
}
