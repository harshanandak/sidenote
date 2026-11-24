//! The home page: every summarised meeting in one calm column, grouped by day, with the
//! sidebar's project and device filters narrowing it.
use super::{
    ListItem, State, ask,
    nav::Filter,
    project, roles,
    theme::{Palette, display},
    widgets::{highlighted, link, notice_bar, pressable, scroll},
};
use rune::{El, Key, TextEdit, col, row};
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
        parts.push(format!("{} tasks", m.summary.todo.len()));
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
        Key::Up => s.move_cursor(-1, tx),
        Key::Down => s.move_cursor(1, tx),
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
        .radius(10.0)
        .fill(p.card)
        .w_full()
        .min_w(0.0)
}

fn empty_text(s: &State) -> &'static str {
    if s.meetings.is_empty() {
        "Your completed meetings and transcripts appear here."
    } else if s.filter != Filter::All && s.search.text.trim().is_empty() {
        "No meetings here yet."
    } else {
        "No meeting mentions that."
    }
}

// ---- home --------------------------------------------------------------------------------

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
            roles::META.line(
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
    if !narrow && let Some(chip) = project::chip(s, k, &m.id, p, layer) {
        r = r.child(chip);
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
    if narrow && let Some(chip) = project::chip(s, k, &m.id, p, layer) {
        out = out.child(row().w_full().pl(56.0).mb(6.0).child(chip.shrink()));
    }
    if let Some(form) = project::form(s, k, &m.id, p) {
        out = out.child(form.ml(56.0).mb(6.0));
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
    let (title, count) = match &s.filter {
        Filter::All => (
            "Meetings".to_string(),
            plural(s.meetings.len(), "meeting", "meetings"),
        ),
        f => {
            let n = s.meetings.iter().filter(|m| s.matches_filter(m)).count();
            let name = match f {
                Filter::Project(name) => name.clone(),
                _ => "From other devices".into(),
            };
            (name, plural(n, "meeting", "meetings"))
        }
    };
    let mut doc = col()
        .key("home")
        .w_full()
        .max_w(860.0)
        .px(side)
        .pt(if narrow { 8.0 } else { 20.0 })
        .pb(32.0)
        .child(
            col()
                .w_full()
                .gap(4.0)
                .mb(10.0)
                .child(
                    text_title(&title, if narrow { 28.0 } else { 34.0 }, p)
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(roles::meta(count, p)),
        )
        .child(meeting_views(s, p))
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
        .children(
            (s.search.focused || !s.search.text.is_empty())
                .then(|| search_box(s, k, 34.0, "Filter meetings and transcripts", p)),
        );
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
    pane.child(
        scroll(col().w_full().items_center().child(doc), "list-scroll").mb(s.overlay_h.get()),
    )
    .child(ask::overlay(s, k, p, narrow, layer))
}

fn meeting_views(s: &State, p: Palette) -> El {
    let short = super::widgets::quiet_button("Short recordings", "filter-short-recordings", p)
        .fill(if s.filter == Filter::Short {
            p.sel
        } else {
            rune::Color::TRANSPARENT
        })
        .on_press(|s: &mut State| s.set_filter(Filter::Short));
    row().gap(4.0).child(short).children(
        (super::nav::meetings_home(s) && s.delete_available())
            .then(|| super::meeting_delete::list_button(p)),
    )
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
        .child(meeting_views(s, p))
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
    fn home_names_the_failed_step_beside_retry() {
        crate::test_paths::isolate();
        let mut state = State::new();
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

    #[test]
    fn home_last_row_scrolls_clear_of_ask() {
        crate::test_paths::isolate();
        let mut state = State::new();
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
