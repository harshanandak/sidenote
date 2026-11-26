//! Small shared pieces: text, chips, links, headings, bullets and the drawn icons.
use super::{
    State, icons, roles,
    theme::{BODY, BODY_LH, Palette},
};
use rune::{
    Align, Color, Cursor, El, Icon, Key, Link, LinkPaint, ScrollSpec, Side, Span, TextStyle,
    Transition, col, el, row, text,
};
use rune_kit::Kit;
use sidenote::core::{self, Point};

/// A glyph or icon character (a tick, a chevron, a close cross) at its own size. Words use the
/// roles in `roles.rs` instead.
pub fn label(s: impl Into<String>, size: f32, weight: f32, ink: Color) -> El {
    super::icons::styled(
        s,
        TextStyle::new(size, weight).lh((size * 1.55).round()),
        ink,
    )
}

pub fn spaced_caps(s: &str, p: Palette) -> El {
    roles::CAPTION.wrap(s.to_uppercase(), p.faint)
}

/// A section heading in the reading column, with an optional count pill after it.
pub fn heading(s: &str, count: Option<usize>, key: &str, p: Palette) -> El {
    roles::section(s, count, p).key(key.to_string())
}

/// A quiet outlined chip for facts about a meeting.
pub fn chip(s: impl Into<String>, p: Palette) -> El {
    roles::META
        .wrap(s, p.soft)
        .px(10.0)
        .py(2.0)
        .radius(999.0)
        .border(1.0, p.line)
}

pub fn pressable(e: El) -> El {
    e.role("button").focusable(true).cursor(Cursor::Pointer)
}

pub fn menu_trigger(
    button: El,
    open: bool,
    p: Palette,
    toggle: impl Fn(&mut State) + Clone + 'static,
) -> El {
    button
        .expanded(open)
        .focus_ring(1.0, 0.0, p.line)
        .on_press(toggle.clone())
        .on_key(move |s, event, tx| {
            if event.key != Key::Enter || event.mods != rune::Mods::default() {
                return false;
            }
            if !tx.key_repeat() {
                toggle(s);
            }
            true
        })
}

pub fn menu_surface(
    k: &Kit,
    p: Palette,
    layer: &mut Vec<El>,
    rows: El,
    (key, trigger, side, align, width): (&str, &str, Side, Align, f32),
    close: fn(&mut State),
) {
    let rows = rune_kit::popover::fit_scroll(rows.padding(4.0));
    let scroll = rune_kit::scroll_bar(k, &format!("{key}-scroll-area"), "scroll", rows)
        .w_full()
        .shrink()
        .min_h(0.0);
    let origin = rune_motion::Vec2::new(
        if align == Align::End { 1.0 } else { 0.0 },
        if side == Side::Top { 1.0 } else { 0.0 },
    );
    let frame = col()
        .key(format!("{key}-menu"))
        .w(width.clamp(220.0, 300.0))
        .min_h(0.0)
        .role("menu")
        .label(if key == "profile-menu" {
            "Profile menu"
        } else {
            "Actions"
        })
        .focus_group()
        .radius(10.0)
        .fill(p.raised)
        .border(1.0, p.line)
        .child(scroll)
        .anchored_fit(trigger, side, align, 6.0)
        .transition(Transition::scale_anchor(0.96, origin).springs(rune_motion::tokens::pair::SNAP))
        .token_motion(k.motion.overlay)
        .focus_return(trigger)
        .modal(true, close);
    layer.push(rune_kit::scrim(format!("{key}-scrim"), close));
    layer.push(
        k.shadow_layers(&k.elevation.floating)
            .fold(frame, El::box_shadow),
    );
}

pub fn menu_separator(p: Palette) -> El {
    el().h(1.0).my(4.0).mx(-4.0).fill(p.line).a11y_hidden()
}

pub fn menu_row(
    name: impl Into<String>,
    p: Palette,
    detail: Option<&str>,
    icon: Option<&'static Icon>,
    shortcut: Option<&str>,
) -> El {
    let name = name.into();
    let mut text = col()
        .grow(1.0)
        .shrink()
        .gap(1.0)
        .a11y_hidden()
        .child(roles::NAV.line(name.clone(), p.ink).key("label"));
    let mut entry = row()
        .role("menuitem")
        .label(name)
        .radius(6.0)
        .px(8.0)
        .gap(10.0)
        .items_center()
        .focusable(true)
        .no_focus_ring()
        .focus_highlight(p.chip)
        .children(icon.map(|glyph| icons::draw(glyph, 16.0, p.soft).a11y_hidden()));
    if let Some(detail) = detail {
        let description = roles::META.wrap(detail, p.faint).min_w(0.0);
        text = text.child(description.key("description"));
        entry = entry.description(detail).min_h(40.0).py(5.0);
    } else {
        entry = entry.h(30.0);
    }
    entry = entry.child(text);
    if let Some(shortcut) = shortcut {
        entry = entry
            .shortcut_hint(shortcut)
            .child(roles::META.line(shortcut, p.faint).a11y_hidden());
    }
    entry
}

#[derive(Clone, Copy)]
pub enum HelpHint {
    Questions,
    Tasks,
    Model,
}

impl HelpHint {
    pub fn build(self, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
        let (key, topic, explanation) = match self {
            Self::Questions => (
                "help-hint-questions",
                "open questions",
                "Questions still waiting for an answer.",
            ),
            Self::Tasks => (
                "help-hint-tasks",
                "tasks",
                "Follow-up work from this meeting.",
            ),
            Self::Model => (
                "help-hint-model",
                "the Ask model",
                "Choose the model for your next question.",
            ),
        };
        let surface = roles::META
            .line(explanation, p.faint)
            .key(format!("{key}-tip"))
            .px(8.0)
            .py(4.0)
            .radius(8.0)
            .fill(p.raised)
            .border(1.0, p.line)
            .inert()
            .group_opacity(0.0, 1.0);
        layer.push(rune_kit::popover::anchored(
            k,
            row().hover_group_shared(key).child(surface),
            key,
            rune_kit::Placement::Top,
            4.0,
        ));
        pressable(row().items_center().justify_center().child(icons::draw(
            &icons::INFO,
            13.0,
            p.faint,
        )))
        .key(key)
        .label(format!("About {topic}"))
        .description(explanation)
        .size(24.0, 24.0)
        .min_w(24.0)
        .radius(8.0)
        .hover_group_shared(key)
        .on_press_tx(move |_: &mut State, tx| tx.focus(key, true))
    }
}

/// Disabled push controls retain their role but never take pointer or keyboard input.
pub trait ButtonDisabled {
    fn button_disabled(self, disabled: bool) -> Self;
}
impl ButtonDisabled for El {
    fn button_disabled(self, disabled: bool) -> Self {
        if disabled {
            self.disabled(true).dim(0.5).inert()
        } else {
            self
        }
    }
}

/// A text link with a quiet hover.
pub fn link(s: impl Into<String>, p: Palette) -> El {
    pressable(roles::CONTROL.icon_label(s, p.ink).px(4.0).radius(5.0)).hover(p.chip)
}

/// Drill-down navigation has one parent and one current item.
pub fn breadcrumb(
    parent: &str,
    current: &str,
    key: &str,
    p: Palette,
    back: impl Fn(&mut State) + 'static,
) -> El {
    let style = TextStyle::new(13.0, 400.0).lh(20.0);
    row()
        .key(key)
        .role("navigation")
        .label("Breadcrumb")
        .w_full()
        .min_w(0.0)
        .items_center()
        .gap(7.0)
        .child(
            text(parent)
                .font(style)
                .color(p.soft)
                .truncate()
                .min_w(0.0)
                .max_w(160.0)
                .key(format!("{key}-parent"))
                .link(
                    Link::new(0..parent.len(), "parent")
                        .hover(LinkPaint::new().color(p.soft).underline(p.soft)),
                )
                .on_link(move |s: &mut State, _, _| back(s)),
        )
        .child(super::icons::draw(
            &super::icons::CHEVRON_RIGHT,
            14.0,
            p.soft,
        ))
        .child(
            text(current)
                .font(TextStyle::new(13.0, 500.0).lh(20.0))
                .color(p.ink)
                .truncate()
                .min_w(0.0)
                .grow(1.0)
                .key(format!("{key}-current")),
        )
}

#[derive(Clone, Copy)]
pub enum ButtonRole {
    Primary,
    Secondary,
    Ghost,
    Destructive,
}

pub fn button_role(name: &str, primary: bool) -> ButtonRole {
    if name.starts_with("Remove") || name.starts_with("Delete") || name.starts_with("Forget") {
        ButtonRole::Destructive
    } else if primary {
        ButtonRole::Primary
    } else {
        ButtonRole::Ghost
    }
}

/// All push buttons share geometry and state paint. Navigation uses `link`.
pub fn button(name: impl Into<String>, key: &str, role: ButtonRole, p: Palette) -> El {
    button_state(name.into(), key, role, rune_kit::State::Rest, p)
}

pub fn row_button(name: impl Into<String>, key: &str, role: ButtonRole, p: Palette) -> El {
    button(name, key, role, p).h(28.0).px(10.0).radius(7.0)
}

pub fn row_action(name: impl Into<String>, key: &str, p: Palette) -> El {
    let name = name.into();
    let role = button_role(&name, false);
    row_button(name, key, role, p)
}

fn button_state(
    name: String,
    key: &str,
    role: ButtonRole,
    state: rune_kit::State,
    p: Palette,
) -> El {
    let accessible_name = name.clone();
    let ink = match role {
        ButtonRole::Primary => p.button_ink,
        ButtonRole::Destructive => p.danger,
        _ => p.soft,
    };
    let fill = match role {
        ButtonRole::Primary => p.button_fill,
        ButtonRole::Secondary => p.card,
        _ => Color::TRANSPARENT,
    };
    let (hover, pressed) = if matches!(role, ButtonRole::Primary) {
        (p.button_primary_hover, p.button_primary_pressed)
    } else {
        (p.button_hover, p.button_pressed)
    };
    let mut b = pressable(
        row()
            .h(32.0)
            .px(12.0)
            .gap(6.0)
            .center()
            .radius(8.0)
            .fill(fill)
            .child(super::icons::styled(name, roles::CONTROL.bold().style(), ink).max_lines(1)),
    )
    .label(accessible_name)
    .hover(hover)
    .press(pressed)
    .focus_ring(2.0, 2.0, p.accent);
    if !key.is_empty() {
        b = b.key(key);
    }
    match role {
        ButtonRole::Secondary => b = b.border(1.0, p.button_border).shadow(1.0, 0.0, p.shadow),
        ButtonRole::Destructive => b = b.border(1.0, p.danger_border),
        _ => {}
    }
    let overlay = match state {
        rune_kit::State::Hover => Some(hover),
        rune_kit::State::Pressed => Some(pressed),
        _ => None,
    };
    if let Some(overlay) = overlay {
        b = b.child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(overlay)
                .radius(8.0),
        );
    }
    match state {
        rune_kit::State::Disabled => b.disabled(true).inert().dim(0.5),
        rune_kit::State::Focus => b.outline(2.0, 2.0, p.accent),
        _ => b,
    }
}

pub fn icon_button(
    icon: &'static rune::Icon,
    label: &str,
    key: &str,
    role: ButtonRole,
    p: Palette,
) -> El {
    let ink = match role {
        ButtonRole::Primary => p.button_ink,
        ButtonRole::Destructive => p.danger,
        _ => p.soft,
    };
    button("", key, role, p)
        .w(32.0)
        .px(8.0)
        .gap(0.0)
        .label(label)
        .help(label)
        .child(super::icons::draw(icon, 16.0, ink))
}

pub fn action(name: impl Into<String>, p: Palette) -> El {
    let name = name.into();
    let role = button_role(&name, false);
    button(name, "", role, p).h(28.0)
}

pub fn quiet_button(s: &str, key: &str, p: Palette) -> El {
    button(s, key, ButtonRole::Ghost, p).h(28.0)
}

pub fn button_sheet(p: Palette) -> El {
    use rune_kit::State::*;
    let mut sheet = col()
        .w_full()
        .padding(16.0)
        .gap(12.0)
        .child(roles::TITLE.wrap("Quiet ink buttons", p.ink));
    for (name, role) in [
        ("Primary", ButtonRole::Primary),
        ("Secondary", ButtonRole::Secondary),
        ("Ghost", ButtonRole::Ghost),
        ("Remove", ButtonRole::Destructive),
    ] {
        let mut group = col().gap(8.0).child(roles::GROUP.wrap(name, p.ink));
        let mut buttons = row().flex_wrap().gap(8.0);
        for (label, state) in [
            ("Rest", Rest),
            ("Hover", Hover),
            ("Pressed", Pressed),
            ("Focus", Focus),
            ("Disabled", Disabled),
        ] {
            buttons = buttons.child(button_state(
                label.into(),
                &format!("sheet-{name}-{label}"),
                role,
                state,
                p,
            ));
        }
        group = group.child(buttons).child(
            button("Compact 28 px", &format!("compact-{name}"), role, p)
                .h(28.0)
                .self_start(),
        );
        sheet = sheet.child(group.child(row().gap(8.0).items_center().child(icon_button(
            &super::icons::ARROW_RIGHT,
            "Continue",
            &format!("icon-{name}"),
            role,
            p,
        ))));
    }
    scroll(sheet, "button-sheet-scroll")
}

pub fn hairline(p: Palette) -> El {
    el().w_full().h(1.0).fill(p.line)
}

#[derive(Clone, Copy)]
pub enum BannerKind {
    Info,
    Working,
    Problem,
}

/// Shared page-column notices. Actions and optional details are added by the caller.
pub fn banner(title: &str, body: impl Into<String>, kind: BannerKind, p: Palette) -> El {
    let problem = matches!(kind, BannerKind::Problem);
    let ink = if problem { p.warn } else { p.ink };
    let mark = match kind {
        BannerKind::Problem => super::icons::draw(&super::icons::WARNING, 16.0, ink),
        BannerKind::Info => roles::NAME.wrap("i", p.soft),
        BannerKind::Working => rune_kit::Spinner::new()
            .size(16.0)
            .color(p.ink)
            .build(&super::theme::kit(&p)),
    };
    col()
        .w_full()
        .min_w(0.0)
        .padding(16.0)
        .gap(12.0)
        .radius(10.0)
        .fill(if problem { p.warnbg } else { p.card })
        .border(1.0, if problem { p.warn } else { p.line })
        .role(if problem { "alert" } else { "status" })
        .child(
            row()
                .gap(10.0)
                .items_center()
                .child(mark)
                .child(roles::NAME.wrap(title, ink).grow(1.0).min_w(0.0)),
        )
        .child(roles::SMALL.wrap(body, p.ink))
}

pub fn notice_action(name: &str, p: Palette) -> El {
    pressable(
        roles::CONTROL
            .bold()
            .wrap(name, p.ink)
            .spans([Span::new(0, name.len()).underline(p.ink)]),
    )
    .focus_ring(2.0, 2.0, p.accent)
}

/// A retry that is running stays focusable and dimmed, but takes no second press, so keyboard
/// focus is not lost when it starts.
pub fn retry_action(action: El, running: bool, on_press: impl Fn(&mut State) + 'static) -> El {
    if running {
        action.dim(0.5)
    } else {
        action.on_press(on_press)
    }
}

pub fn inline_notice(message: &str, action: El, p: Palette) -> El {
    row()
        .gap(6.0)
        .items_center()
        .flex_wrap()
        .child(icons::draw(&icons::WARNING, 14.0, p.warn))
        .child(roles::CONTROL.wrap(message, p.soft).role("status"))
        .child(action)
}

pub fn alert_strip(message: impl Into<String>, action: El, p: Palette) -> El {
    row()
        .w_full()
        .min_h(40.0)
        .px(16.0)
        .py(4.0)
        .gap(8.0)
        .items_center()
        .fill(p.warnbg)
        .child(icons::draw(&icons::WARNING, 16.0, p.warn))
        .child(
            roles::CONTROL
                .wrap(message, p.ink)
                .min_w(0.0)
                .grow(1.0)
                .role("alert"),
        )
        .child(action)
}

pub fn scroll(content: El, key: &str) -> El {
    col()
        .key(key.to_string())
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(content)
}

/// The bullet column of a point. Colour carries the kind, the shape stays small.
fn bullet(kind: &str, number: Option<usize>, p: Palette) -> El {
    let glyph = |g: &str, ink: Color, weight: f32| {
        text(g.to_string())
            .font(TextStyle::new(14.0, weight).lh(BODY_LH))
            .color(ink)
    };
    let mark = match (number, kind) {
        (Some(n), _) => glyph(&format!("{n}."), p.ok, 600.0),
        (None, "decided") => super::icons::draw(&super::icons::FLAG, 16.0, p.ok),
        (None, "concern") => super::icons::draw(&super::icons::WARNING, 16.0, p.warn),
        (None, "open") => super::icons::draw(&super::icons::HELP, 16.0, p.open),
        (None, "response") => el()
            .size(6.0, 6.0)
            .radius(3.0)
            .border(1.3, p.faint)
            .mt(11.0),
        _ => el().size(5.0, 5.0).radius(2.5).fill(p.mark).mt(11.0),
    };
    col().w(22.0).items_center().child(mark)
}

/// Recording positions keep a minutes field even before the first minute.
pub fn recording_time(at: &str) -> String {
    core::seconds(at).map_or_else(
        || at.to_string(),
        |seconds| format!("{}:{:02}", seconds / 60, seconds % 60),
    )
}

/// The timestamp that opens the transcript at its line.
pub fn stamp(at: &[String], origin: &str, p: Palette) -> Option<El> {
    let first = at.first()?.clone();
    let origin = origin.to_string();
    let shown = format!("\u{b7} {} in recording", recording_time(&first));
    Some(
        pressable(text(shown.clone()).font(roles::TIME.style()).color(p.faint))
            .key(format!("stamp-{origin}"))
            .label(format!("Open the transcript at {}", recording_time(&first)))
            .help("Time in the recording. Open the transcript.")
            .px(5.0)
            .py(1.0)
            .radius(4.0)
            .hover(p.chip)
            .on_press_tx(move |s: &mut State, tx| s.jump(&first, origin.clone(), tx)),
    )
}

/// A point: bullet, text with the speaker, and a timestamp link. Decisions are bold, open
/// questions are softer, and responses sit one level in, under the point they answer.
pub fn point_row(pt: &Point, key: String, number: Option<usize>, from: bool, p: Palette) -> El {
    // Decided points are the key items of a topic, in bold. The numbered Decisions list is
    // all decisions, so it uses a lighter weight to stay readable.
    let decided = pt.kind == "decided";
    let weight = match (decided, number) {
        (true, None) => 600.0,
        (_, Some(_)) => 500.0,
        _ => 400.0,
    };
    let ink = if pt.kind == "open" { p.soft } else { p.ink };
    let mut content = pt.text.clone();
    let mut spans = Vec::new();
    if let Some(who) = &pt.who {
        content.push_str("  ");
        let start = content.len();
        // Non-breaking spaces keep the speaker on one line.
        content.push_str(&format!("\u{b7}\u{a0}{}", who.replace(' ', "\u{a0}")));
        spans.push(
            Span::new(start, content.len())
                .weight(400.0)
                .size(roles::META.size)
                .color(p.faint),
        );
    }
    let mut body = row()
        .flex_wrap()
        .items_center()
        .gaps(4.0, 0.0)
        .grow(1.0)
        .min_w(0.0)
        .child(
            text(content)
                .font(TextStyle::new(BODY, weight).lh(BODY_LH))
                .color(ink)
                .wrap()
                .spans(spans)
                .min_w(0.0)
                .shrink(),
        );
    if let Some(at) = pt.at.first() {
        let at = at.clone();
        let origin = key.clone();
        let time = recording_time(&at);
        let caption = format!("at\u{a0}{time}");
        let link = Link::new(0..caption.len(), "source")
            .label(format!("Open the transcript at {time}"))
            .help("Read this in the transcript")
            .hover(LinkPaint::new().color(p.ink).bg(p.chip));
        body = body.child(
            roles::TIME
                .wrap(caption, p.faint)
                .key(format!("{key}-time"))
                .min_w(0.0)
                .shrink()
                .link(link)
                .on_link(move |s: &mut State, _, tx| s.jump(&at, origin.clone(), tx)),
        );
    }
    let mut r = row()
        .key(key)
        .gap(8.0)
        .py(3.0)
        .pr(6.0)
        .radius(8.0)
        .child(bullet(&pt.kind, number, p))
        .child(body);
    if pt.kind == "response" {
        r = r.pl(26.0);
    }
    if from {
        r = r.fill(p.hit);
    }
    r
}

/// Highlighted text from byte ranges into `text`.
pub fn highlighted(
    text_: &str,
    ranges: &[std::ops::Range<usize>],
    role: roles::Role,
    ink: Color,
    mark_bg: Color,
) -> El {
    let spans: Vec<Span> = ranges
        .iter()
        .map(|r| Span::new(r.start, r.end).weight(600.0).bg(mark_bg, 1.5))
        .collect();
    role.wrap(text_, ink).spans(spans)
}

fn notice_row(note: &str, p: Palette, icon: Option<&'static rune::Icon>) -> El {
    row()
        .px(24.0)
        .py(8.0)
        .gap(12.0)
        .items_center()
        .fill(p.warnbg)
        .children(icon.map(|icon| super::icons::draw(icon, 16.0, p.warn)))
        .child(roles::CONTROL.wrap(note, p.warn).grow(1.0))
}
/// A persistent status strip using the same geometry as dismissible notices.
pub fn status_notice(note: &str, icon: &'static rune::Icon, p: Palette) -> El {
    notice_row(note, p, Some(icon)).role("alert")
}
/// A warning strip above a page, with a Dismiss link.
pub fn notice_bar(note: &str, p: Palette) -> El {
    notice_row(note, p, None).child(
        action("Dismiss", p)
            .key("notice-dismiss")
            .on_press(|s: &mut State| s.notice = None),
    )
}

/// A small rounded label on a tint: a status, a kind of change.
pub fn badge(s: impl Into<String>, ink: Color, bg: Option<Color>) -> El {
    let b = text(s)
        .font(roles::META.bold().style())
        .color(ink)
        .px(8.0)
        .py(1.0)
        .radius(999.0);
    match bg {
        Some(f) => b.fill(f),
        None => b,
    }
}

/// A filled dot, as the mark of a severity or a project.
pub fn dot(size: f32, ink: Color) -> El {
    el().size(size, size).radius(size / 2.0).fill(ink)
}

/// Where a list page and its right-hand panel sit: the list's width with the panel open, the
/// panel's width, and whether the panel covers the list because there is no room for both.
pub fn split_geometry(s: &State, narrow: bool, panel_open: bool) -> (f32, f32, bool) {
    let pane = s.app_w.get() - super::nav::width(s);
    let overlay = narrow || pane < 780.0;
    let panel = (pane * 0.38).clamp(340.0, 420.0);
    let list = if panel_open && !overlay {
        pane - panel - 1.0
    } else {
        pane
    };
    (list, panel, overlay)
}

/// The panel's top strip: a caption, the way back on a narrow window, and the close button.
pub fn panel_top(title: &str, back: &str, overlay: bool, close: fn(&mut State), p: Palette) -> El {
    let shut = icon_button(
        &super::icons::X,
        "Close the panel (Escape)",
        "panel-close",
        ButtonRole::Ghost,
        p,
    )
    .on_press_tx(move |s: &mut State, tx| {
        close(s);
        tx.blur();
    });
    let mut top = row().w_full().h(48.0).px(12.0).gap(8.0).items_center();
    if overlay {
        top = top.child(
            pressable(roles::CONTROL.icon_label(format!("\u{2039}  {back}"), p.soft))
                .key("panel-back")
                .label(format!("Back to {back} (Escape)"))
                .px(10.0)
                .py(4.0)
                .radius(8.0)
                .hover(p.chip)
                .on_press_tx(move |s: &mut State, tx| {
                    close(s);
                    tx.blur();
                }),
        );
    }
    top.child(
        spaced_caps(if overlay { "" } else { title }, p)
            .ml(if overlay { 0.0 } else { 8.0 })
            .grow(1.0),
    )
    .child(shut)
}

/// A panel's frame: top strip, scrolling body, hairline, footer.
pub fn panel_frame(key: &str, top: El, body: El, foot: El, p: Palette) -> El {
    col()
        .key(key.to_string())
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .fill(p.card)
        .child(top)
        .child(scroll(body, &format!("{key}-scroll")))
        .child(hairline(p))
        .child(foot)
}

/// The list column, a hairline and the panel, side by side; or the panel alone when it covers
/// the list.
pub fn list_and_panel(list: El, panel: Option<El>, panel_w: f32, overlay: bool, p: Palette) -> El {
    let body = row().w_full().grow(1.0).min_h(0.0).min_w(0.0);
    match panel {
        None => body.child(list),
        Some(panel) if overlay => body.child(panel.grow(1.0)),
        Some(panel) => body
            .child(list)
            .child(el().w(1.0).h_full().fill(p.line))
            .child(panel.w(panel_w)),
    }
}

/// A key cap, as in "Ctrl K": a small outlined label for a shortcut.
pub fn keycap(s: impl Into<String>, p: Palette) -> El {
    roles::CAPTION
        .wrap(s, p.faint)
        .px(6.0)
        .py(1.0)
        .radius(5.0)
        .border(1.0, p.line)
}

/// A short confirmation, pinned above the bottom controls. It slides in and fades; Rune
/// shortens that to a fade when the system asks for reduced motion.
pub fn toast(text_: &str, p: Palette) -> El {
    toast_with_action(text_, None, p)
}
pub fn toast_with_action(text_: &str, action: Option<El>, p: Palette) -> El {
    col()
        .key("toast")
        .pin(Some(0.0), None, Some(0.0), Some(96.0))
        .px(12.0)
        .items_center()
        .child(
            row()
                .gap(8.0)
                .items_center()
                .px(16.0)
                .py(8.0)
                .radius(999.0)
                .fill(p.card)
                .border(1.0, p.line)
                .shadow(4.0, 16.0, p.shadow)
                .transition(super::ask::rise(8.0))
                .child(label("\u{2713}", 13.0, 700.0, p.ok))
                .child(roles::CONTROL.wrap(text_, p.ink))
                .children(action),
        )
}

#[cfg(test)]
mod tests {
    use super::recording_time;

    #[test]
    fn disabled_buttons_reject_pointer_and_keyboard_even_with_a_handler() {
        use super::{ButtonDisabled, ButtonRole, button};
        use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime};
        #[derive(Default)]
        struct Clicks {
            disabled: bool,
            count: usize,
        }
        fn view(s: &Clicks, _: &mut Cx<Clicks>) -> El {
            button(
                "Example",
                "example",
                ButtonRole::Secondary,
                crate::ui::theme::palette(false),
            )
            .button_disabled(s.disabled)
            .on_press(|s: &mut Clicks| s.count += 1)
        }
        let mut runtime = Runtime::new(
            Clicks {
                disabled: true,
                count: 0,
            },
            view,
            (180.0, 80.0),
            Env::default(),
        );
        runtime.frame(0.0);
        runtime.pointer_move(20.0, 16.0);
        runtime.pointer_down(20.0, 16.0);
        runtime.pointer_up(20.0, 16.0);
        runtime.set_full_keyboard_access(true);
        for key in [Key::Tab, Key::Enter] {
            runtime.key(KeyEvent {
                key,
                mods: Mods::default(),
            });
            runtime.frame(1.0 / 60.0);
        }
        assert_eq!(runtime.state.count, 0);
        runtime.update(|s, _| s.disabled = false);
        runtime.frame(1.0 / 60.0);
        let bounds = runtime
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("example"))
            .unwrap()
            .rect;
        let (x, y) = (bounds[0] + bounds[2] / 2.0, bounds[1] + bounds[3] / 2.0);
        runtime.pointer_move(x, y);
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(1.0 / 60.0);
        assert_eq!(runtime.state.count, 1);
    }

    #[test]
    fn recording_times_keep_minutes_when_the_source_omits_hours() {
        for (source, shown) in [
            ("00:00:24", "0:24"),
            ("00:24", "0:24"),
            ("00:04:10", "4:10"),
            ("04:10", "4:10"),
            ("01:04:10", "64:10"),
            ("01:02.520", "1:02"),
        ] {
            assert_eq!(recording_time(source), shown);
        }
    }
}
