use rune::{Align, Color, Cx, El, Icon, Span, Tx, col, el, row};
use rune_motion::tokens::{SETTLE, pair};
use rune_motion::{Spring, Transition};
use sidenote::core::Urgency;
use std::{rc::Rc, time::Duration};

use super::{State, icons, roles, theme::Palette, widgets};

pub(super) type Action = Rc<dyn Fn(&mut State)>;
type RecordRetry = Rc<dyn Fn(&mut State, &mut Tx)>;

#[derive(Clone, Copy, Default)]
pub(super) enum Cursor {
    #[default]
    Rest,
    Hover,
    Keyboard,
    Selected,
}
#[derive(Clone, Copy)]
pub(super) enum Density {
    Rail,
    List,
}
impl Density {
    fn wide(self, width: f32) -> bool {
        matches!(self, Self::List) && width >= 720.0
    }

    pub(super) fn separator(self, width: f32, p: Palette) -> El {
        let wide = self.wide(width);
        let (left, right) = if wide { (38.0, 12.0) } else { (34.0, 8.0) };
        el().h(1.0).ml(left).mr(right).fill(p.line)
    }
}
#[derive(Clone, Copy)]
pub(super) enum Status<'a> {
    Open,
    Done,
    Waiting(&'a str),
    Review,
}
#[derive(Default)]
pub(super) enum Save<'a> {
    #[default]
    Idle,
    Pending(Duration),
    Failed(&'a str, Action),
    RecordFailed(&'a str, RecordRetry),
    Retrying,
}
#[derive(Default)]
pub(super) struct Row<'a> {
    pub title: &'a str,
    pub cursor: Cursor,
    pub interactive: bool,
    pub save: Save<'a>,
}
pub(super) struct Task<'a> {
    pub row: Row<'a>,
    pub description: Option<&'a str>,
    pub project: Option<&'a str>,
    pub owner: Option<&'a str>,
    pub due: Option<(&'a str, Urgency, bool)>,
    pub status: Status<'a>,
    pub toggle: Option<Action>,
}

fn shell(r: &Row<'_>, wide: bool, p: Palette) -> El {
    let (x, y) = if wide { (12.0, 10.0) } else { (8.0, 8.0) };
    let e = row().w_full().items_start().px(x).py(y);
    let e = e.gap(10.0).radius(8.0);
    if !r.interactive {
        return e;
    }
    let e = widgets::pressable(e).focus_ring(1.0, 0.0, p.line);
    let e = e.motion(Spring::NONE);
    if matches!(r.cursor, Cursor::Selected) {
        let bar = el().size(2.0, 18.0).radius(1.0).fill(p.ink);
        let bar = bar.absolute(0.0, 10.0);
        return e.fill(p.sel).child(bar);
    }
    let e = e.hover(p.chip).focus_fill(p.chip);
    match r.cursor {
        Cursor::Keyboard => e.fill(p.chip).outline(1.0, 0.0, p.line),
        Cursor::Hover => e.fill(p.chip),
        _ => e,
    }
}

fn title(label: &str, role: roles::Role, color: Color, cap: usize) -> El {
    let e = role.wrap(label, color).wrap_anywhere().max_lines(cap);
    e.key("title").help_when_truncated(label)
}

fn glyph(icon: &'static Icon, ink: Color) -> El {
    icons::draw(icon, icons::ROW, ink).trace_name(icon.name)
}

fn tick(task: &Task<'_>, mark: El, narrow: bool, p: Palette) -> El {
    let slot = el().size(16.0, 19.0);
    let Some(toggle) = task.toggle.clone() else {
        return slot.child(mark.mt(1.5));
    };
    let group = el().size(16.0, 16.0).hover_group().motion(Spring::NONE);
    let mark = if matches!(task.status, Status::Open) {
        let preview = glyph(&icons::DONE, p.faint).absolute(0.0, 0.0);
        let preview = preview.group_opacity(0.0, 1.0);
        group.child(mark.group_opacity(1.0, 0.0)).child(preview)
    } else {
        group.child(mark)
    };
    let done = matches!(task.status, Status::Done);
    let verb = if done { "open" } else { "done" };
    let side = if narrow { 32.0 } else { 24.0 };
    let target = el().size(side, side).items_center().justify_center();
    let target = target.role("checkbox").checked(done).focusable(false);
    let target = target.label(format!("Mark {} {verb}", task.row.title));
    let target = target.on_press(move |s: &mut State| toggle(s)).child(mark);
    slot.child(target.absolute((16.0 - side) / 2.0, (19.0 - side) / 2.0))
}

type Meta<'a> = (&'a str, Color, f32);
fn meta<'a>(p: Palette, parts: impl IntoIterator<Item = Meta<'a>>) -> El {
    let (mut text, mut spans) = (String::new(), Vec::new());
    for (label, ink, weight) in parts {
        if !text.is_empty() {
            text.push_str(" · ");
        }
        let start = text.len();
        text.push_str(label);
        spans.push(Span::new(start, text.len()).color(ink).weight(weight));
    }
    let line = roles::META.wrap(text, p.faint).spans(spans);
    line.wrap_anywhere().max_lines(2)
}

fn date_style(p: Palette, done: bool, urgency: Urgency) -> (Color, f32) {
    match (done, urgency) {
        (true, _) | (_, Urgency::Calm) => (p.faint, 400.0),
        (_, Urgency::Late) => (p.warn, 600.0),
        (_, Urgency::Today) => (p.ink, 600.0),
        (_, Urgency::Soon) => (p.soft, 400.0),
    }
}

fn save_line(save: Save<'_>, p: Palette) -> Option<El> {
    let retry_action = |error| {
        widgets::notice_action("Try again", p)
            .focus_ring(1.0, 0.0, p.line)
            .help(error)
    };
    let line = match save {
        Save::Pending(elapsed) if elapsed < Duration::from_secs(1) => return None,
        Save::Pending(_) | Save::Retrying => {
            let icon = icons::draw(&icons::LOADER_CIRCLE, icons::INLINE, p.faint);
            let label = roles::CONTROL.wrap("Saving", p.faint).role("status");
            row().items_center().gap(6.0).child(icon).child(label)
        }
        Save::Failed(error, retry) => {
            let action = widgets::retry_action(retry_action(error), false, move |s| retry(s));
            widgets::inline_notice("Not saved.", action, p)
        }
        Save::RecordFailed(error, retry) => {
            let action = retry_action(error)
                .key("retry")
                .focusable(false)
                .on_press_tx(move |s, tx| retry(s, tx));
            widgets::inline_notice("Not saved.", action, p)
        }
        Save::Idle => return None,
    };
    Some(line.mt(4.0).key("save").transition(Transition::opacity()))
}

pub(super) fn task_row(t: Task<'_>, density: Density, width: f32, reduced: bool, p: Palette) -> El {
    let wide = density.wide(width);
    let done = matches!(t.status, Status::Done);
    let (mark, ink, status): (&Icon, _, _) = match t.status {
        Status::Open => (&icons::CIRCLE, p.soft, None),
        Status::Done => (&icons::DONE, p.faint, None),
        Status::Waiting(label) => (&icons::WAITING, p.soft, Some(label)),
        Status::Review => (&icons::CIRCLE_DOT, p.soft, Some("In review")),
    };
    let narrow = width < 600.0;
    let mut icon = glyph(mark, ink).key(mark.name);
    if !reduced {
        icon = icon.transition(Transition::blur_replace().springs(pair::SNAP));
    }
    let mut shell = shell(&t.row, wide, p).child(tick(&t, icon, narrow, p));
    if let Some(toggle) = t.toggle {
        shell = shell.on_key(move |s: &mut State, event, _| {
            if matches!(&event.key, rune::Key::Text(key) if key == " ") {
                toggle(s);
                return true;
            }
            false
        });
    }
    let color = if done { p.faint } else { p.ink };
    let role = if done { roles::SMALL } else { roles::NAME };
    let mut name = title(t.row.title, role, color, if narrow { 3 } else { 2 });
    if done {
        name = name.strike(0, t.row.title.len(), None);
    }
    let fade = Spring::new(0.18, 0.0);
    let spring = if reduced { fade } else { SETTLE };
    let body = copy().child(name.motion(spring));
    let owner_ink = t.owner.filter(|_| !done).map_or(p.faint, |_| p.soft);
    let owner = (t.owner.unwrap_or("No owner"), owner_ink, 400.0);
    let dates = t.due.into_iter().flat_map(|(label, urgency, guessed)| {
        let (ink, weight) = date_style(p, done, urgency);
        let hint = guessed.then_some(("guessed", p.faint, 400.0));
        [(label, ink, weight)].into_iter().chain(hint)
    });
    let status = status.map(|s| (s, p.soft, 400.0));
    let save = save_line(t.row.save, p);
    if !wide {
        let details = meta(p, [owner].into_iter().chain(dates).chain(status));
        return shell.child(body.child(details).children(save));
    }
    let grey = if done { p.faint } else { p.soft };
    let desc = t.description.map(|s| roles::META.line(s, grey));
    let status = status.map(|s| meta(p, [s]));
    let body = body.children(desc).children(status).children(save);
    let project = t.project.map(|label| {
        let dot = el().size(7.0, 7.0).radius(3.5).fill(grey);
        let project = roles::META.line(label, p.faint);
        row().gap(6.0).items_center().child(dot).child(project)
    });
    let owner = meta(p, [owner]).w(120.0).align(Align::End);
    let due = meta(p, dates).w(96.0).align(Align::End);
    let project = row().w(140.0).children(project);
    shell.child(body).child(project).child(owner).child(due)
}

fn copy() -> El {
    col().grow(1.0).min_w(0.0).gap(2.0).motion(SETTLE)
}

pub(super) fn question_row(q: Row<'_>, details: &str, settled: bool, width: f32, p: Palette) -> El {
    let kind = if settled {
        Record::Settled
    } else {
        Record::Question
    };
    record_row(q, details, kind, width, p)
}

pub(super) enum Record {
    Claim,
    Question,
    Settled,
}

pub(super) fn record_row(r: Row<'_>, details: &str, kind: Record, width: f32, p: Palette) -> El {
    let (mark, ink, color, role, lines): (&Icon, _, _, _, _) = match kind {
        Record::Claim => (&icons::CIRCLE, p.soft, p.ink, roles::NAME, 2),
        Record::Question => (&icons::CIRCLE_DASHED, p.soft, p.ink, roles::SMALL, 3),
        Record::Settled => (&icons::DONE, p.faint, p.faint, roles::SMALL, 3),
    };
    let name = title(r.title, role, color, lines + usize::from(width < 600.0));
    let body = copy().child(name);
    let details = (!details.is_empty()).then(|| meta(p, [(details, p.faint, 400.0)]));
    let e = shell(&r, false, p).child(glyph(mark, ink).mt(1.5));
    e.child(body.children(details).children(save_line(r.save, p)))
}

pub(super) fn done_line(count: usize, open: bool, toggle: Action, p: Palette) -> El {
    let (glyph, label): (&Icon, String) = match open {
        true => (&icons::CHEVRON_DOWN, "Hide done".into()),
        false => (&icons::CHEVRON_RIGHT, format!("{count} done")),
    };
    let mark = icons::draw(glyph, icons::INLINE, p.faint);
    let line = row().h(28.0).px(8.0).gap(6.0).items_center().child(mark);
    let line = widgets::pressable(line.child(roles::META.line(label, p.faint)));
    let line = line.expanded(open).focus_ring(1.0, 0.0, p.line);
    line.on_press(move |s: &mut State| toggle(s))
}

const LONG: &str = "Confirm the delivery schedule and write up the remaining questions before the next review with the release team, then send the updated plan to everyone who will join";

fn sheet(width: f32, reduced: bool, p: Palette) -> El {
    use {Cursor::*, Save::*, Status::*};
    let action: Action = Rc::new(|_| {});
    let sample = |label, item| {
        let label = roles::META.line(label, p.faint).ml(8.0);
        col().child(label).child(item)
    };
    let label = "Send the revised timeline";
    let soon = ("Thu 27 Nov", Urgency::Soon, false);
    let late = ("2 days late", Urgency::Late, false);
    let retry = Failed("The write could not finish.", action.clone());
    let wait = Pending(Duration::from_secs(2));
    let mut rail = col().w(width.min(372.0) - 32.0).gap(10.0);
    let task = |key, title, cursor, status, save, due, density| {
        let r = Row {
            title,
            cursor,
            save,
            interactive: true,
        };
        let t = Task {
            row: r,
            owner: (title != LONG).then_some("You"),
            due: Some(due),
            status,
            toggle: Some(action.clone()),
            description: Some("Update the plan with the team's feedback."),
            project: Some("Release plan"),
        };
        task_row(t, density, width, reduced, p).key(key)
    };
    let today = ("Today", Urgency::Today, false);
    for (key, cursor, status, save, due) in [
        ("Open", Rest, Open, Idle, today),
        ("Hover", Hover, Open, Idle, today),
        ("Focus", Keyboard, Open, Idle, today),
        ("Selected", Selected, Open, Idle, today),
        ("Done", Rest, Done, Idle, late),
        ("Waiting", Rest, Waiting("Waiting on Jo"), Idle, late),
        ("In review", Rest, Review, Idle, today),
        ("Failed", Rest, Open, retry, today),
        ("Saving", Rest, Done, wait, today),
    ] {
        let item = task(key, label, cursor, status, save, due, Density::Rail);
        rail = rail.child(sample(key, item));
    }
    let done = task("Expanded", label, Rest, Done, Idle, late, Density::Rail);
    let opened = col()
        .child(done_line(2, true, action.clone(), p))
        .child(done);
    rail = rail.child(sample(
        "Done tasks, folded",
        done_line(2, false, action.clone(), p),
    ));
    rail = rail.child(sample("Done tasks, opened", opened));
    let due = ("Fri 28 Nov", Urgency::Calm, true);
    let guess = task("Guessed", LONG, Rest, Open, Idle, due, Density::Rail);
    rail = rail.child(sample("Guessed date, no owner", guess));
    for (text, details, settled) in [
        ("Open question", "Waiting on Jo", false),
        ("Settled question", "Release plan · 2 sources", true),
    ] {
        let q = Row {
            title: "Will the updated schedule fit?",
            ..Row::default()
        };
        let item = question_row(q, details, settled, width, p).key(text);
        rail = rail.child(sample(text, item));
    }
    let list = task("List", label, Rest, Open, Idle, soon, Density::List);
    let list = col().w(width.min(732.0) - 32.0).child(list);
    let list = list.child(Density::List.separator(width, p));
    let page = if width < 720.0 { col() } else { row() };
    page.gap(24.0).child(rail).child(list)
}

pub(super) fn scene(s: &State, cx: &mut Cx<State>, p: Palette) -> El {
    let width = cx.layout_with("rows-sheet", |r| r[2] as i32);
    let width = width.map_or(s.app_w.get(), |w| w as f32);
    let page = col().w_full().h_full().padding(16.0).fill(p.bg);
    page.key("rows-sheet")
        .child(sheet(width, cx.is_reduced_motion(), p))
}

#[test]
fn row_states_draw_glyphs_without_pills_and_borders() {
    let p = super::theme::palette(false);
    let view = |p: &Palette, _: &mut Cx<Palette>| sheet(1334.0, true, *p);
    let mut rt = rune::Runtime::new(p, view, (1334.0, 1250.0), rune::Env::default());
    rt.frame(0.0);
    let nodes = rt.elements();
    let by_key = |key| {
        let item = nodes.iter().find(|n| n.key.as_deref() == Some(key));
        item.unwrap()
    };
    let clear = Color::TRANSPARENT;
    for (key, fill, icon) in [
        ("Open", clear, "circle"),
        ("Hover", p.chip, "circle"),
        ("Focus", p.chip, "circle"),
        ("Selected", p.sel, "circle"),
        ("Done", clear, "circle-check"),
        ("Waiting", clear, "circle-pause"),
        ("In review", clear, "circle-dot"),
        ("Failed", clear, "circle"),
        ("Saving", clear, "circle-check"),
        ("Open question", clear, "circle-dashed"),
        ("Settled question", clear, "circle-check"),
    ] {
        let item = by_key(key);
        assert_eq!(item.fill, fill, "{key}");
        let icon = match icon {
            "circle-check" => "check-circle",
            "circle-pause" => "pause-circle",
            "circle-dashed" => "question-circle",
            _ => icon,
        };
        let icon = format!("solar-{icon}");
        let glyph = nodes.iter().any(|n| {
            n.path.starts_with(&item.path)
                && n.trace_name == Some(icon.as_str())
                && n.opacity > 0.99
        });
        assert!(glyph, "{key}");
    }
    let long = nodes.iter().find(|n| n.help.as_deref() == Some(LONG));
    assert!((long.unwrap().rect[3] - 38.0).abs() < 1.0);
    for key in ["Open", "Done", "Waiting", "In review"] {
        let [x, y, w, h] = by_key(key).rect;
        for i in &rt.instances {
            let [left, top, _, _] = i.rect;
            if left >= x && left < x + w && top >= y && top < y + h {
                assert_eq!(i.border[3], 0.0, "{key}");
                assert_eq!(i.radii, [0.0; 4], "{key}");
                if key == "Done" && i.fill[3] > 0.0 {
                    assert_eq!(i.fill, p.faint.premul(1.0));
                }
            }
        }
    }
}
