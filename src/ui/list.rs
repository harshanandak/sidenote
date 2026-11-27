//! The home page: every summarised meeting in one calm column, grouped by day, with the
//! sidebar's project and device filters narrowing it.
use super::{
    ListItem, State, ask,
    nav::Filter,
    project, roles,
    theme::{Palette, display},
    widgets::{
        highlighted, icon_button, link, menu_row, menu_surface, menu_trigger, notice_bar,
        pressable, row_button, scroll,
    },
};
use rune::{Align, El, Key, Side, TextEdit, col, el, row};
use rune_kit::{Kit, TextInput};
use sidenote::core::{self, Meeting, SearchHit, SearchSource};

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Who was there, else what kind of meeting it was, then what came out of it.
pub(super) fn people_line(m: &Meeting) -> String {
    let who = match m.summary.people.as_slice() {
        [] => m.summary.kind.clone(),
        [a] => a.clone(),
        [a, b] => format!("{a}, {b}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    };
    let decided = m.points().filter(|p| p.kind == "decided").count();
    let mut parts = vec![who];
    if let Some(device) = &m.device {
        parts.push(format!("from {device}"));
    }
    if decided > 0 {
        parts.push(plural(decided, "decision", "decisions"));
    }
    if !m.summary.todo.is_empty() {
        parts.push(plural(m.summary.todo.len(), "task", "tasks"));
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" \u{b7} ")
}

fn source_name(source: SearchSource) -> &'static str {
    match source {
        SearchSource::Title => "Title",
        SearchSource::Summary => "Summary",
        SearchSource::Notes => "Typed notes",
        SearchSource::Transcript => "Transcript",
    }
}

pub(super) fn snippet(hit: &SearchHit, p: Palette) -> El {
    col()
        .mt(4.0)
        .gap(1.0)
        .child(roles::CAPTION.wrap(source_name(hit.source), p.faint))
        .child(highlighted(
            &hit.snippet.text,
            &hit.snippet.matches,
            roles::SMALL,
            p.soft,
            p.hit,
        ))
}

fn search_key(s: &mut State, e: &rune::KeyEvent, tx: &mut rune::Tx) -> bool {
    match e.key {
        Key::Up => {
            s.keyboard = true;
            s.move_cursor(-1, tx)
        }
        Key::Down => {
            s.keyboard = true;
            s.move_cursor(1, tx)
        }
        Key::Enter => s.open_cursor(tx),
        Key::Escape => {
            s.search = TextEdit::new("");
            s.on_search_changed();
            tx.blur();
        }
        _ => return false,
    }
    true
}

fn search_box(s: &State, k: &Kit, height: f32, placeholder: &str, p: Palette) -> El {
    TextInput::new(&s.search)
        .plain()
        .placeholder(placeholder)
        .on_edit(|s: &mut State| &mut s.search)
        .on_change(|s: &mut State, _| s.on_search_changed())
        .on_key(search_key)
        .height(height)
        .build(k)
        .key("search")
        .border(1.0, if s.search.focused { p.accent } else { p.line })
        .radius(999.0)
        .fill(p.card)
        .w_full()
        .min_w(0.0)
}

fn empty_text(s: &State) -> &'static str {
    if s.meetings.is_empty() {
        "Choose New meeting to start a recording. It will appear here when it is ready."
    } else if s.filter != Filter::All
        && s.meetings_view == MeetingsView::All
        && s.search.text.trim().is_empty()
    {
        "No meetings here yet."
    } else if s.meetings_view != MeetingsView::All && s.search.text.trim().is_empty() {
        "No meetings match this view. Try the All view."
    } else {
        "No meetings match. Try a different search or clear the search field."
    }
}

// ---- home --------------------------------------------------------------------------------

/// The meetings list's own view filter. It is orthogonal to the sidebar's project and
/// device filters, which narrow the same rows through `matches_filter`.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum MeetingsView {
    #[default]
    All,
    WithTasks,
    WithDecisions,
}

impl State {
    pub(super) fn matches_meetings_view(&self, meeting: &Meeting) -> bool {
        match self.meetings_view {
            MeetingsView::All => true,
            MeetingsView::WithTasks => !meeting.summary.todo.is_empty(),
            MeetingsView::WithDecisions => meeting.points().any(|p| p.kind == "decided"),
        }
    }

    pub(super) fn set_meetings_view(&mut self, view: MeetingsView) {
        if self.meetings_view != view {
            self.meetings_view = view;
            self.cursor = 0;
            self.refresh_list();
        }
    }
}

fn home_row(
    s: &State,
    k: &Kit,
    item: &ListItem,
    position: usize,
    narrow: bool,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    let m = &s.meetings[item.index];
    let at = m
        .started
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default();
    let notes_job = super::notes_status::job(s, m);
    let problem = notes_job.as_ref().is_some_and(|j| {
        super::notes_status::is_failure(&j.state)
            || matches!(
                j.state,
                core::notes::JobState::NoHelper
                    | core::notes::JobState::Done {
                        tasks_failed: true,
                        ..
                    }
            )
    });
    let mut text_col = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(
            roles::TITLE
                .wrap(&m.title, p.ink)
                .wrap_anywhere()
                .max_lines(2),
        )
        .child(
            roles::META.wrap(
                notes_job
                    .as_ref()
                    .and_then(super::notes_status::list_label)
                    .map(str::to_string)
                    .unwrap_or_else(|| people_line(m)),
                if problem { p.ink } else { p.faint },
            ),
        );
    if let Some(hit) = &item.hit {
        text_col = text_col.child(snippet(hit, p));
    }
    let id = m.id.clone();
    let main = pressable(row().gap(14.0).items_center().child(text_col))
        .key(format!("meeting-{}", m.id))
        .label(&m.title)
        .grow(1.0)
        .min_w(0.0)
        .on_press_tx(move |s: &mut State, tx| s.select(&id, tx));
    let mut r = row()
        .w_full()
        .gap(12.0)
        .px(10.0)
        .py(9.0)
        .radius(10.0)
        .hover(p.chip);
    r = if narrow {
        r.items_start()
    } else {
        r.items_center()
    };
    r = r.child(main);
    if problem && let Some(job) = &notes_job {
        let setup = matches!(
            job.state,
            core::notes::JobState::NoHelper | core::notes::JobState::HelperMissing
        );
        let id = m.id.clone();
        let request = if matches!(
            job.state,
            core::notes::JobState::Done {
                tasks_failed: true,
                ..
            }
        ) {
            core::notes::Request::SortAgain
        } else if matches!(job.state, core::notes::JobState::TooLong { .. }) {
            core::notes::Request::FirstPart
        } else {
            job.retry_request()
        };
        let needs_page = matches!(job.state, core::notes::JobState::ContextTooLong { .. });
        r = r.child(
            super::area::button(
                if setup {
                    if matches!(job.state, core::notes::JobState::NoHelper)
                        && s.ai_notes
                            .found
                            .contains(&(core::notes::Helper::Claude, true))
                    {
                        "Turn on Meeting summary"
                    } else {
                        "Choose an agent"
                    }
                } else if matches!(job.state, core::notes::JobState::TooLong { .. }) {
                    "Use first 4 hours"
                } else {
                    "Try again"
                },
                &format!("notes-list-action-{id}"),
                p,
            )
            .on_press_tx(move |s: &mut State, tx| {
                if setup {
                    s.open_ai_notes();
                } else if needs_page {
                    s.select(&id, tx);
                } else {
                    s.request_notes(id.clone(), request);
                }
            }),
        );
    }
    let more = super::more::menu(s, k, m, "row", p, layer);
    let time = roles::meta(at, p);
    r = if narrow {
        r.child(col().items_end().child(more).child(time))
    } else {
        r.child(row().w(44.0).justify_end().child(time)).child(more)
    };
    let keyboard = (s.keyboard || !s.search.text.trim().is_empty()) && s.cursor == position;
    if keyboard {
        r = r.fill(p.chip);
    }
    let mut out = col().w_full().child(r);
    if let Some(form) = project::form(s, k, &m.id, p) {
        out = out.child(form.mx(10.0).mb(6.0));
    }
    out
}

/// The home page: saved and pending meetings, newest day first, with the Ask bar floating over
/// it. Ask here covers all of them.
pub fn home(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    if s.filter == Filter::Short {
        return short_recordings(s, p, narrow);
    }
    let side = if narrow { 16.0 } else { 40.0 };
    // Bare count `24`, or `shown of total` when filtered or searched (old: `24 meetings`).
    let total = s.meetings.len();
    let shown = s.row_count();
    let filtered = s.filter != Filter::All
        || s.meetings_view != MeetingsView::All
        || !s.search.text.trim().is_empty();
    let count = if filtered {
        format!("{shown} of {total}")
    } else {
        format!("{total}")
    };
    let mut doc = col()
        .key("home")
        .w_full()
        .max_w(860.0)
        .px(side)
        .pt(if narrow { 8.0 } else { 20.0 })
        .pb(32.0)
        .child(
            row()
                .key("meetings-title-row")
                .w_full()
                .items_center()
                .gap(8.0)
                .mb(10.0)
                .child(text_title("Meetings", if narrow { 28.0 } else { 34.0 }, p))
                .child(roles::meta(count, p))
                .child(el().grow(1.0))
                .child(title_more(s, k, p, layer)),
        )
        .children((s.filter != Filter::All).then(|| {
            link("Show all meetings", p)
                .key("filter-clear")
                .self_start()
                .mb(10.0)
                .on_press(|s: &mut State| s.go_meetings())
        }))
        .children(
            (s.filter == Filter::All && s.search.text.trim().is_empty())
                .then(|| super::ai_notes::setup_card(s, p))
                .flatten(),
        )
        .children(super::today::strip(s, p))
        .children(super::project_merge::panel(s, k, p, layer))
        .children(super::area::project_tools(s, k, p, layer))
        .child(toolbar(s, k, p, layer));
    let mut position = 0;
    for group in &s.groups {
        doc = doc.child(
            roles::GROUP
                .wrap(&group.label, p.soft)
                .mt(24.0)
                .mb(4.0)
                .px(10.0),
        );
        for item in &group.items {
            doc = doc.child(home_row(s, k, item, position, narrow, p, layer));
            position += 1;
        }
    }
    if position == 0 {
        doc = doc.child(roles::SMALL.wrap(empty_text(s), p.soft).mt(24.0).px(10.0));
    }
    // "meeting-pane" is the key the Ask thread measures to size itself.
    let mut pane = col()
        .key("meeting-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .fill(p.bg);
    if let Some(note) = &s.notice {
        pane = pane.child(notice_bar(note, p));
    }
    pane.child(scroll(
        col().w_full().items_center().child(doc.pb(16.0)),
        "list-scroll",
    ))
    .child(ask::overlay(s, k, p, narrow, layer))
}

/// The key of the open title menu, so only one menu is ever open.
const TITLE_MENU: &str = "meetings-title-menu";

/// The title row's More button. Its menu holds `Deleted meetings`, which opens the same
/// deleted list as the old quiet button did; there is no meetings-root folder action, so
/// no "Open meeting folder" row.
/// One row of the title menu: a named, focusable menu item that closes the menu.
fn title_menu_row(label: &str, key: &str, p: Palette, press: impl Fn(&mut State) + 'static) -> El {
    menu_row(label, p, None, None, None)
        .key(key)
        .on_press(press)
}

fn title_more(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let open = s.more_menu.as_deref() == Some(TITLE_MENU);
    let trigger = menu_trigger(
        icon_button(
            &super::icons::MORE,
            "More actions",
            "meetings-title-more",
            super::widgets::ButtonRole::Ghost,
            p,
        ),
        open,
        p,
        |s| {
            let same = s.more_menu.as_deref() == Some(TITLE_MENU);
            s.more_menu = (!same).then(|| TITLE_MENU.to_string());
        },
    );
    if open {
        let mut menu = col();
        // The delete gate stays, as with the old button; hidden without the capability.
        if s.delete_available() {
            menu = menu.child(title_menu_row(
                "Deleted meetings",
                "meetings-open-deleted",
                p,
                |s| {
                    s.more_menu = None;
                    s.open_deleted();
                },
            ));
        }
        // The removed tab survives as a row so the Keep action stays reachable.
        menu = menu.child(title_menu_row(
            "Short recordings",
            "meetings-open-short",
            p,
            |s| {
                s.more_menu = None;
                s.set_filter(Filter::Short);
            },
        ));
        menu_surface(
            k,
            p,
            layer,
            menu,
            (
                TITLE_MENU,
                "meetings-title-more",
                Side::Bottom,
                Align::End,
                240.0,
            ),
            |s| s.more_menu = None,
        );
    }
    trigger
}

/// The key of the open Filter popover, so only one menu is ever open.
const FILTER_MENU: &str = "meetings-filter";

/// One toolbar row under the title: the list-local view tabs, the persistent search
/// field, and one Filter button whose popover lists only existing project filters.
fn toolbar(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    row()
        .key("meetings-toolbar")
        .w_full()
        .gap(8.0)
        .items_center()
        .flex_wrap()
        .child(view_tab(
            s,
            "All",
            "meetings-view-all",
            MeetingsView::All,
            p,
        ))
        .child(view_tab(
            s,
            "With tasks",
            "meetings-view-with-tasks",
            MeetingsView::WithTasks,
            p,
        ))
        .child(view_tab(
            s,
            "With decisions",
            "meetings-view-with-decisions",
            MeetingsView::WithDecisions,
            p,
        ))
        .child(el().grow(1.0))
        .child(
            el().w(200.0)
                .child(search_box(s, k, 28.0, "Search meetings", p)),
        )
        .child(filter_button(s, k, p, layer))
}

fn view_tab(s: &State, label: &str, key: &str, view: MeetingsView, p: Palette) -> El {
    super::widgets::quiet_button(label, key, p)
        .fill(if s.meetings_view == view {
            p.sel
        } else {
            rune::Color::TRANSPARENT
        })
        .on_press_tx(move |s: &mut State, tx| {
            s.set_meetings_view(view);
            // A clicked tab gives focus back to the page, so the arrows and Enter drive the list.
            if !tx.by_keys() {
                tx.blur();
            }
        })
}

fn filter_button(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let open = s.more_menu.as_deref() == Some(FILTER_MENU);
    let mut button = row_button(
        "Filter",
        "meetings-filter",
        super::widgets::ButtonRole::Secondary,
        p,
    );
    if s.filter != Filter::All {
        button = button.fill(p.sel);
    }
    let trigger = menu_trigger(button, open, p, |s| {
        let same = s.more_menu.as_deref() == Some(FILTER_MENU);
        s.more_menu = (!same).then(|| FILTER_MENU.to_string());
    });
    if open {
        let on = s.config.features.projects;
        let mut names: Vec<&String> = s.nav.project_counts.keys().filter(|_| on).collect();
        names.sort();
        let mut menu = col().child(
            menu_row("All meetings", p, None, None, None)
                .key("meetings-filter-all")
                .on_press(|s: &mut State| {
                    s.more_menu = None;
                    s.set_filter(Filter::All);
                }),
        );
        for name in names {
            let name = name.as_str().to_owned();
            menu = menu.child(
                menu_row(name.clone(), p, None, None, None)
                    .key(format!("meetings-filter-project-{name}"))
                    .on_press(move |s: &mut State| {
                        s.more_menu = None;
                        s.set_filter(Filter::Project(name.clone()));
                    }),
            );
        }
        menu_surface(
            k,
            p,
            layer,
            menu,
            (
                FILTER_MENU,
                "meetings-filter",
                Side::Bottom,
                Align::End,
                240.0,
            ),
            |s| s.more_menu = None,
        );
    }
    trigger
}

pub fn text_title(s: &str, size: f32, p: Palette) -> El {
    rune::text(s.to_string())
        .font(display(size))
        .color(p.ink)
        .wrap()
}

fn short_recordings(s: &State, p: Palette, narrow: bool) -> El {
    let mut doc = col()
        .w_full()
        .max_w(860.0)
        .px(if narrow { 16.0 } else { 40.0 })
        .py(20.0)
        .gap(16.0)
        .child(text_title(
            "Short recordings",
            if narrow { 28.0 } else { 34.0 },
            p,
        ))
        .child(roles::SMALL.wrap(core::recording_guard::EXPLANATION, p.soft))
        .child(
            link("Show all meetings", p)
                .key("filter-clear")
                .self_start()
                .on_press(|s: &mut State| s.go_meetings()),
        );
    if s.short_recordings.is_empty() {
        doc = doc.child(roles::SMALL.wrap("No short recordings.", p.soft));
    }
    for recording in &s.short_recordings {
        let id = recording.id.clone();
        doc = doc.child(row().w_full().items_center().gap(12.0).py(12.0)
            .child(col().grow(1.0).min_w(0.0).gap(4.0)
                .child(roles::TITLE.wrap(&recording.date, p.ink))
                .child(roles::META.wrap(&recording.length, p.soft)))
            .child(super::area::button("Keep", &format!("keep-recording-{id}"), p)
                .on_press(move |s: &mut State| {
                    match core::recording_guard::keep(&id) {
                        Ok(()) => { s.short_recordings.retain(|r| r.id != id); s.notice = Some("Recording kept. Normal processing is queued.".into()); }
                        Err(_) => { s.notice = Some("Could not queue this recording. Try again after the next refresh.".into()); }
                    }
                    if let Some(handle) = &s.scan_tx { handle.invalidate(); }
                })));
    }
    let mut pane = col()
        .key("meeting-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .fill(p.bg);
    if let Some(note) = &s.notice {
        pane = pane.child(notice_bar(note, p));
    }
    pane.child(scroll(
        col().w_full().items_center().child(doc),
        "list-scroll",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Meeting {
        Meeting::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tools/ui-fixtures/screens/meetings/1793955600000-1"),
        )
        .unwrap()
    }

    #[test]
    fn meeting_row_uses_the_singular_for_one_task() {
        let mut meeting = fixture();
        assert!(!meeting.summary.todo.is_empty());
        meeting.summary.todo.truncate(1);
        assert!(people_line(&meeting).ends_with("1 task"));
    }

    #[test]
    fn home_names_the_failed_step_beside_retry() {
        crate::test_paths::isolate();
        let mut state = State::new();
        state.go_meetings(); // Startup moved from Meetings to Home; exercise the Meetings list here.
        let meeting = fixture();
        let action_key = format!("notes-list-action-{}", meeting.id);
        let root = tempfile::tempdir().unwrap();
        let jobs = root.path().join("local/notes-jobs");
        std::fs::create_dir_all(&jobs).unwrap();
        let record = jobs.join(format!("{}.json", meeting.id));
        std::fs::write(&record, r#"{"state":"backend_error"}"#).unwrap();
        state.meetings.push(meeting);
        state.ai_notes.jobs = super::super::notes_status::read(root.path(), &state.meetings);
        state.refresh_list();
        let mut rt = rune::Runtime::new(
            state,
            super::super::view,
            (390.0, 900.0),
            rune::Env::default(),
        );
        rt.frame(0.0);
        let elements = rt.elements();
        assert!(
            elements
                .iter()
                .any(|e| e.text.as_deref() == Some("Summary failed"))
        );
        assert!(elements.iter().any(|e| {
            e.key.as_deref() == Some(action_key.as_str()) && e.label.as_deref() == Some("Try again")
        }));
    }

    fn synthetic_meeting(id: &str, title: &str) -> Meeting {
        Meeting {
            folder: std::path::PathBuf::from(format!("synthetic/{id}")),
            device: None,
            id: id.into(),
            title: title.into(),
            diary_title: String::new(),
            summary: core::Summary {
                title: title.into(),
                ..Default::default()
            },
            summary_raw: String::new(),
            draft_raw: String::new(),
            transcript_raw: String::new(),
            routed: None,
            routed_revision: String::new(),
            summary_language: None,
            notes: String::new(),
            lines: Vec::new(),
            started: None,
            minutes: None,
            export_path: None,
            search: title.to_lowercase(),
        }
    }

    fn solo_meeting() -> Vec<Meeting> {
        vec![synthetic_meeting(
            "1793955600000-1",
            "Invented blueberry standup",
        )]
    }

    fn trio_meetings() -> Vec<Meeting> {
        vec![
            synthetic_meeting("1793955600000-1", "Invented blueberry standup"),
            synthetic_meeting("1793955600000-2", "Invented clover review"),
            synthetic_meeting("1793955600000-3", "Invented juniper retro"),
        ]
    }

    fn home_state(meetings: Vec<Meeting>) -> State {
        let mut state = State::new();
        // Without Hex (as on Windows CI) startup opens Hex setup over the list.
        state.hex_open = false;
        state.go_meetings(); // Startup moved from Meetings to Home; exercise the Meetings list here.
        state.meetings = meetings;
        state.refresh_list();
        state
    }

    fn render_home(state: State, width: f32) -> rune::Runtime<State> {
        let mut rt = rune::Runtime::new(
            state,
            super::super::view,
            (width, 900.0),
            rune::Env::default(),
        );
        rt.frame(0.0);
        rt
    }

    /// Renders the home list and returns its elements. Asserts one title row (new):
    /// ≈33 high, not a stacked title-plus-count ≈54 (old).
    fn title_snapshot(meetings: Vec<Meeting>, search: &str) -> Vec<rune::ElementInfo> {
        let mut state = home_state(meetings);
        state.search = rune::TextEdit::new(search);
        state.refresh_list();
        let elements = render_home(state, 1334.0).elements();
        let row = elements
            .iter()
            .find(|e| e.key.as_deref() == Some("meetings-title-row"))
            .expect("title row");
        assert!(row.rect[3] < 45.0);
        elements
    }

    /// The elements under the title row, for same-row assertions.
    fn under(elements: &[rune::ElementInfo]) -> Vec<&rune::ElementInfo> {
        let prefix = elements
            .iter()
            .find(|e| e.key.as_deref() == Some("meetings-title-row"))
            .map(|row| format!("{}/", row.path))
            .expect("title row");
        elements
            .iter()
            .filter(|e| e.path.starts_with(&prefix))
            .collect()
    }

    fn tap(rt: &mut rune::Runtime<State>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.settle_all();
        rt.frame(0.0);
    }

    fn key(rt: &mut rune::Runtime<State>, key: Key) {
        rt.key(rune::KeyEvent {
            key,
            mods: rune::Mods::default(),
        });
        rt.frame(0.0);
    }

    #[test]
    fn title_row_shows_the_inline_count_and_no_short_tab() {
        let elements = title_snapshot(trio_meetings(), "");
        let under = under(&elements);
        assert!(under.iter().any(|e| e.text.as_deref() == Some("Meetings")));
        assert!(under.iter().any(|e| e.text.as_deref() == Some("3")));
        assert!(
            elements
                .iter()
                .all(|e| e.text.as_deref() != Some("Short recordings"))
        );
        assert!(
            elements
                .iter()
                .all(|e| e.key.as_deref() != Some("filter-short-recordings"))
        );
    }

    #[test]
    fn title_count_shows_shown_of_total_when_searched() {
        let elements = title_snapshot(trio_meetings(), "blueberry");
        let under = under(&elements);
        assert!(under.iter().any(|e| e.text.as_deref() == Some("1 of 3")));
    }

    #[test]
    fn title_menu_opens_the_deleted_list() {
        let mut state = home_state(solo_meeting());
        // The row keeps the capability gate (old: at the button).
        state.enable_delete_for_tests();
        let mut rt = render_home(state, 1334.0);
        tap(&mut rt, "meetings-title-more");
        assert!(
            rt.find("meetings-open-deleted").is_some(),
            "pressing the title More button exposes Deleted meetings"
        );
        tap(&mut rt, "meetings-open-deleted");
        assert!(
            rt.find("deleted-close").is_some(),
            "selecting it opens the same deleted list the old quiet button did"
        );
    }

    #[test]
    fn title_menu_keeps_a_short_recordings_entry() {
        let mut rt = render_home(home_state(solo_meeting()), 1334.0);
        tap(&mut rt, "meetings-title-more");
        assert!(
            rt.find("meetings-open-short").is_some(),
            "the removed tab survives as a menu row, not a tab"
        );
        tap(&mut rt, "meetings-open-short");
        assert!(
            rt.state.filter == Filter::Short,
            "selecting it opens the archived short-recordings list"
        );
        assert!(
            rt.find("meetings-title-row").is_none(),
            "the short-recordings list replaces the home list"
        );
    }

    /// Without the delete capability the gated row has nothing to show.
    #[cfg(target_os = "linux")]
    #[test]
    fn delete_stays_gated_without_the_fixture_flag() {
        assert!(!State::new().delete_available());
    }

    #[test]
    fn rows_keep_their_more_menu_and_lose_the_project_chip() {
        let mut state = home_state(solo_meeting());
        state.config.features.projects = true;
        state.areas.meeting_projects.insert(
            "1793955600000-1".into(),
            super::super::project::MeetingProject {
                name: Some("Invented Atlas".into()),
                confirmed: true,
            },
        );
        let elements = render_home(state, 1334.0).elements();
        // The row More menu stays (old and new key).
        assert!(
            elements
                .iter()
                .any(|e| e.key.as_deref() == Some("more-row-1793955600000-1-button"))
        );
        // No passive row chip (old `Invented Atlas` pill); the page keeps `reading_chip`.
        assert!(elements.iter().all(|e| {
            !e.text
                .as_deref()
                .unwrap_or_default()
                .contains("Invented Atlas")
        }));
        assert!(
            elements
                .iter()
                .all(|e| e.text.as_deref() != Some("Add to project"))
        );
    }

    #[test]
    fn toolbar_holds_view_tabs_search_and_filter() {
        let state = home_state(vec![synthetic_meeting(
            "1793955600000-1",
            "Invented blueberry standup",
        )]);
        let elements = render_home(state, 1334.0).elements();
        let bar = elements
            .iter()
            .find(|e| e.key.as_deref() == Some("meetings-toolbar"))
            .expect("toolbar row");
        let prefix = format!("{}/", bar.path);
        // The search field is persistent (new): it shows while empty and unfocused.
        for key in [
            "meetings-view-all",
            "meetings-view-with-tasks",
            "meetings-view-with-decisions",
            "search",
            "meetings-filter",
        ] {
            assert!(
                elements
                    .iter()
                    .filter(|e| e.path.starts_with(&prefix))
                    .any(|e| e.key.as_deref() == Some(key)),
                "{key} is in the toolbar"
            );
        }
    }

    #[test]
    fn toolbar_click_leaves_the_list_keys_live() {
        let mut state = home_state(vec![
            synthetic_meeting("1793955600000-1", "Invented blueberry standup"),
            synthetic_meeting("1793955600000-2", "Invented clover review"),
        ]);
        for meeting in &mut state.meetings {
            meeting.summary.todo.push(core::Todo {
                task: "Invented follow-up".into(),
                ..Default::default()
            });
        }
        let mut rt = render_home(state, 1334.0);
        tap(&mut rt, "meetings-view-with-tasks");
        key(&mut rt, Key::Down);
        assert_eq!(rt.state.cursor, 1);
        key(&mut rt, Key::Enter);
        assert_eq!(rt.state.selected.as_deref(), Some("1793955600000-2"));
    }

    #[test]
    fn toolbar_filter_keeps_enter_for_its_menu() {
        let mut rt = render_home(home_state(trio_meetings()), 1334.0);
        rt.update(|_, tx| tx.focus("meetings-filter", true));
        key(&mut rt, Key::Enter);
        assert_eq!(rt.state.more_menu.as_deref(), Some(FILTER_MENU));
        assert!(rt.state.selected.is_none());
    }

    #[test]
    fn with_empty_search_down_shows_the_cursor() {
        let mut rt = render_home(home_state(trio_meetings()), 1334.0);
        tap(&mut rt, "search");
        assert!(rt.state.search.focused);
        key(&mut rt, Key::Down);
        assert_eq!(rt.state.cursor, 1);
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        let row = elements
            .iter()
            .find(|e| e.key.as_deref() == Some("meeting-1793955600000-2"))
            .expect("second row");
        let parent = row.path.rsplit_once('/').unwrap().0;
        assert!(
            elements.iter().find(|e| e.path == parent).unwrap().fill != rune::Color::TRANSPARENT
        );
    }

    #[test]
    fn with_empty_view_offers_all_and_go_meetings_resets_the_view() {
        let mut rt = render_home(home_state(solo_meeting()), 1334.0);
        tap(&mut rt, "meetings-view-with-decisions");
        assert!(rt.elements().iter().any(|e| {
            e.text.as_deref() == Some("No meetings match this view. Try the All view.")
        }));
        rt.update(|s, _| s.go_meetings());
        assert!(rt.state.meetings_view == MeetingsView::All);
    }

    #[test]
    fn with_tasks_hides_meetings_without_todo() {
        let mut state = home_state(vec![
            synthetic_meeting("1793955600000-1", "Invented blueberry standup"),
            synthetic_meeting("1793955600000-2", "Invented clover review"),
        ]);
        state.meetings[1].summary.todo.push(core::Todo {
            task: "Invented follow-up".into(),
            ..Default::default()
        });
        let mut rt = render_home(state, 1334.0);
        tap(&mut rt, "meetings-view-with-tasks");
        let elements = rt.elements();
        assert!(
            elements
                .iter()
                .any(|e| e.key.as_deref() == Some("meeting-1793955600000-2"))
        );
        assert!(
            elements
                .iter()
                .all(|e| e.key.as_deref() != Some("meeting-1793955600000-1"))
        );
    }

    #[test]
    fn with_decisions_counts_only_decided_points() {
        let mut state = home_state(vec![
            synthetic_meeting("1793955600000-1", "Invented blueberry standup"),
            synthetic_meeting("1793955600000-2", "Invented clover review"),
        ]);
        state.meetings[0].summary.topics.push(core::Topic {
            title: "Invented topic".into(),
            points: vec![core::Point {
                kind: "discussed".into(),
                text: "Invented chatter".into(),
                ..Default::default()
            }],
        });
        state.meetings[1].summary.topics.push(core::Topic {
            title: "Invented topic".into(),
            points: vec![core::Point {
                kind: "decided".into(),
                text: "Invented decision".into(),
                ..Default::default()
            }],
        });
        let mut rt = render_home(state, 1334.0);
        tap(&mut rt, "meetings-view-with-decisions");
        let elements = rt.elements();
        assert!(
            elements
                .iter()
                .any(|e| e.key.as_deref() == Some("meeting-1793955600000-2"))
        );
        assert!(
            elements
                .iter()
                .all(|e| e.key.as_deref() != Some("meeting-1793955600000-1"))
        );
    }

    #[test]
    fn filter_popover_lists_existing_projects() {
        let mut state = home_state(vec![
            synthetic_meeting("1793955600000-1", "Invented blueberry standup"),
            synthetic_meeting("1793955600000-2", "Invented clover review"),
        ]);
        state.config.features.projects = true;
        state.nav.project_counts.insert("Invented Atlas".into(), 1);
        state
            .nav
            .meeting_project
            .insert("1793955600000-1".into(), "Invented Atlas".into());
        let mut rt = render_home(state, 1334.0);
        tap(&mut rt, "meetings-filter");
        rt.settle_all();
        rt.frame(0.0);
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("Invented Atlas")),
            "the Filter popover lists the existing project filter"
        );
        tap(&mut rt, "meetings-filter-project-Invented Atlas");
        assert!(
            rt.state.filter == Filter::Project("Invented Atlas".into()),
            "selecting it narrows the list to that project"
        );
        assert!(rt.find("meeting-1793955600000-1").is_some());
        assert!(rt.find("meeting-1793955600000-2").is_none());
        // With Projects off, set_filter rejects project filters, so none are offered.
        rt.update(|s, _| s.config.features.projects = false);
        tap(&mut rt, "meetings-filter");
        rt.settle_all();
        rt.frame(0.0);
        assert!(rt.find("meetings-filter-project-Invented Atlas").is_none());
    }

    #[test]
    fn home_last_row_scrolls_clear_of_ask() {
        crate::test_paths::isolate();
        let detect = crate::ui::hex::SKIP_INSTALL_DETECT.replace(true);
        let mut state = State::new();
        state.go_meetings(); // Startup moved from Meetings to Home; exercise the Meetings list here.
        crate::ui::hex::SKIP_INSTALL_DETECT.set(detect);
        crate::ui::scene_agents(&mut state, &["codex"]);
        let meeting = fixture();
        state.meetings = (1..=40)
            .map(|suffix| {
                let mut meeting = meeting.clone();
                meeting.id = format!("1793955600000-{suffix}");
                meeting
            })
            .collect();
        state.refresh_list();
        let last = state.groups.last().unwrap().items.last().unwrap().index;
        let last_key = format!("meeting-{}", state.meetings[last].id);
        let mut rt = rune::Runtime::new(
            state,
            super::super::view,
            (390.0, 900.0),
            rune::Env::default(),
        );
        rt.frame(0.0);
        rt.frame(0.016);
        rt.update(|_, tx| tx.scroll_to_y("list-scroll", 10_000.0));
        rt.frame(0.016);
        let elements = rt.elements();
        let rect = |key: &str| {
            elements
                .iter()
                .find(|e| e.key.as_deref() == Some(key))
                .unwrap()
                .rect
        };
        let [_, list_top, _, list_height] = rect("list-scroll");
        let [_, ask_top, _, _] = rect("ask-overlay");
        let [_, last_top, _, last_height] = rect(&last_key);
        assert!(list_top + list_height <= ask_top + 1.0);
        assert!(last_top >= list_top && last_top + last_height <= list_top + list_height);
    }
}
