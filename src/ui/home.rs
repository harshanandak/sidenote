use super::{
    State, ask, cards, list, roles,
    theme::{PAGE_TITLE, Palette},
    widgets::scroll,
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
    !task.done
        && (task.mine == Some(true)
            || (!owner.is_empty()
                && task
                    .owner
                    .as_deref()
                    .is_some_and(|name| people::key(&teams.owner_name(name)) == owner)))
}

fn task_counts(tasks: &[TaskItem], owner: &str, teams: &Teams, today: NaiveDate) -> (usize, usize) {
    let owner = people::key(&teams.owner_name(owner));
    tasks
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
    let (open, late) = task_counts(&s.tasks.items, owner, &s.tasks.teams, today);
    let meetings = meetings_label(recent_meetings(&s.meetings, today).count());
    let tiles = if narrow { col() } else { row() };
    let doc = col()
        .w_full()
        .max_w(860.0)
        .px(if narrow { 16.0 } else { 40.0 })
        .py(24.0)
        .gap(16.0)
        .child(
            col()
                .gap(4.0)
                .child(
                    list::text_title(&greeting(now.hour(), owner), PAGE_TITLE, p)
                        .key("home-greeting"),
                )
                .child(roles::META.wrap(today.format("%A %-d %B").to_string(), p.soft)),
        )
        .child(
            tiles
                .gap(12.0)
                .child(tile(
                    "home-tasks-tile",
                    format!("{open} tasks need you"),
                    Some(format!("{late} late")),
                    p,
                ))
                .child(tile("home-meetings-tile", meetings, None, p)),
        );
    col()
        .key("meeting-pane")
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .child(scroll(
            col().w_full().items_center().child(doc),
            "home-scroll",
        ))
        .child(ask::overlay(s, k, p, narrow, layer))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let today = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        let summary = r#"{"title":"Workshop","todo":[
            {"task":"Draft notes","due":"2026-11-24"},
            {"task":"Send agenda","owner":"CASEY MORGAN","due":"2026-11-25"},
            {"task":"File notes","owner":"Casey Morgan","due":"2026-11-24"},
            {"task":"Print agendas","owner":"Lee","due":"2026-11-24"},
            {"task":"Reserve room","owner":"Casey Morgan"}
        ]}"#;
        let meetings =
            [0, 1, 2].map(|days| meeting(root.path(), today - chrono::Days::new(days), summary));
        assert_eq!(
            meetings_label(recent_meetings(&meetings, today).count()),
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
        assert_eq!(
            task_counts(&tasks, "Casey Morgan", &Teams::default(), today),
            (3, 1)
        );
    }

    #[test]
    fn startup_home_and_meeting_breadcrumb_route() {
        use super::super::{nav::Page, theme, view};
        use rune::{Env, Runtime, text::TextSystem};

        let mut s = State::new();
        assert!(s.page() == Page::Home);
        s.sidebar_open.set(true);
        let mut rt = Runtime::with_text(
            s,
            view,
            (1200.0, 900.0),
            Env::new(false, 1.0),
            TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
        );
        rt.settle_all();
        rt.frame(0.0);
        let home = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("nav-home"))
            .unwrap();
        assert_eq!(home.fill, theme::palette(false).sel);
        for key in ["home-greeting", "home-tasks-tile", "home-meetings-tile"] {
            assert!(rt.find(key).is_some(), "missing {key}");
        }

        let root = tempfile::tempdir().unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        let sample = meeting(root.path(), today, r#"{"title":"Workshop"}"#);
        rt.update(|s, tx| {
            s.go_meetings();
            assert!(s.page() == Page::Meetings);
            let id = sample.id.clone();
            s.meetings.push(sample);
            s.select(&id, tx);
        });
        rt.frame(0.0);
        let (x, y) = rt.find("meeting-breadcrumb-parent").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        assert!(rt.state.page() == Page::Meetings);
        assert!(rt.state.selected.is_none());
    }
}
