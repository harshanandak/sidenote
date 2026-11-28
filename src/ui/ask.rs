//! The Ask pill keeps its place while prompts, scope, models and answers open above it.
//!
//! The panel and thread use Rune's shared motion tokens and follow the system's reduced motion setting.
use super::{
    AskRun, State, icons,
    meeting::COLUMN,
    roles,
    theme::Palette,
    widgets::{
        ButtonDisabled, ButtonRole, button, hairline, menu_row, menu_row_base, menu_separator,
        pressable, quiet_button,
    },
};
use regex::{Captures, Regex};
use rune::tok::motion::{SNAP, pair};
use rune::{Align, El, Key, KeyEvent, Shadow, Side, Spring, SpringPair, Transition, Tx, col, row};
use rune_kit::{Kit, Markdown, MarkdownStyle, Spinner, TextInput};
use sidenote::core::{self, Meeting, Message, ask::SUGGESTIONS, ask_all};
use std::sync::LazyLock;

static STAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[((?:\d{1,2}:)?\d{1,2}:\d{2})\]").expect("stamp pattern"));

/// Turns `[mm:ss]` and `[hh:mm:ss]` into Markdown links the thread can press.
fn linkify(answer: &str) -> String {
    STAMP
        .replace_all(answer, |c: &Captures| {
            format!("[{}](stamp:{})", core::short(&c[1]), &c[1])
        })
        .into_owned()
}

/// `[[<meeting id>@mm:ss]]` and `[[<meeting id>]]`, as the home Ask is told to cite.
static CITE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[\[(\d+-\d+)(?:@((?:\d{1,2}:)?\d{1,2}:\d{2}))?\]\]").expect("citation pattern")
});

/// A citation's time as shown. `mm:ss` stays as written; only a leading `00:` hour goes.
pub(super) fn cite_time(stamp: &str) -> &str {
    if stamp.matches(':').count() == 2 {
        core::short(stamp)
    } else {
        stamp
    }
}

/// The longest a meeting's title gets inside a citation link.
const CITE_TITLE: usize = 26;

/// A title short enough for a link, with the characters Markdown would act on escaped.
pub(super) fn short_title(title: &str) -> String {
    let title = title.trim();
    let cut = if title.chars().count() > CITE_TITLE {
        let head: String = title.chars().take(CITE_TITLE).collect();
        let head = head.rsplit_once(' ').map_or(head.as_str(), |(h, _)| h);
        format!(
            "{}\u{2026}",
            head.trim_end_matches([',', ';', ':', '-', ' '])
        )
    } else {
        title.to_string()
    };
    let mut out = String::new();
    for c in cut.chars() {
        if "\\*_`[]<>".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Turns the home Ask's citations into Markdown links the thread can press: the short meeting
/// title, then the time when there is one. An id that is not a known meeting stays plain text.
pub(super) fn link_citations(answer: &str, meetings: &[Meeting]) -> String {
    CITE.replace_all(answer, |c: &Captures| {
        let id = &c[1];
        let Some(m) = meetings.iter().find(|m| m.id == id) else {
            return c[0].to_string();
        };
        let title = short_title(&m.title);
        match c.get(2) {
            Some(at) => format!(
                "[{title} \u{b7} {}](cite:{id}@{})",
                cite_time(at.as_str()),
                at.as_str()
            ),
            None => format!("[{title}](cite:{id})"),
        }
    })
    .into_owned()
}

fn message(s: &State, k: &Kit, i: usize, m: &Message, p: Palette) -> El {
    let key = s.msg_key(i);
    let from = s.back.as_deref() == Some(key.as_str());
    let user = m.role == "user";
    let body = if user {
        roles::TITLE.wrap(&m.text, p.ink)
    } else {
        let origin = key.clone();
        let all = s.ask_meeting_id().is_none() && !s.ask_current_live();
        Markdown::new(if all {
            link_citations(&m.text, &s.meetings)
        } else {
            linkify(&m.text)
        })
        .style(
            MarkdownStyle::from_kit(k)
                .text(p.ink)
                .muted(p.soft)
                .link(p.ink)
                .body_font(roles::READ.style()),
        )
        .on_link_tx(move |s: &mut State, url, tx| {
            if let Some(stamp) = url.strip_prefix("stamp:") {
                s.jump(stamp, origin.clone(), tx);
            } else if let Some(target) = url.strip_prefix("cite:") {
                s.open_citation(target, origin.clone(), tx);
            }
        })
        .build(k)
    };
    // A question and its answer read as one block; the hairline goes above each question.
    col()
        .key(key)
        .w_full()
        .pt(if user { 14.0 } else { 4.0 })
        .pb(if user { 2.0 } else { 14.0 })
        .px(8.0)
        .radius(8.0)
        .fill(if from { p.hit } else { p.card })
        .transition(if user {
            slide(8.0, pair::GLIDE)
        } else {
            Transition::opacity()
        })
        .child(body)
        .children((!user && m.stopped).then(|| roles::META.wrap("Stopped", p.faint).mt(4.0)))
        .children((!user).then(|| answer_label(s, m)).flatten().map(|label| {
            roles::META
                .wrap(label, p.faint)
                .key(format!("answer-model-{i}"))
                .mt(4.0)
        }))
}

/// Show the provider and the reported model when available. Older messages can lack both.
pub(super) fn answer_label(s: &State, m: &Message) -> Option<String> {
    let name = m.runner.as_deref()?;
    let runner = s.runners.iter().find(|r| r.name == name);
    let agent = runner.map_or_else(
        || short_runner(name).to_string(),
        |r| core::ask_options::agent_name(r).to_string(),
    );
    let fallback = if m.fallback { " \u{b7} fallback" } else { "" };
    Some(match (&m.model, runner) {
        (Some(id), Some(r)) => format!("{agent} \u{b7} {}{fallback}", model_label(r, Some(id))),
        (Some(id), None) => format!("{agent} \u{b7} {id}{fallback}"),
        (None, _) => format!("{agent}{fallback}"),
    })
}

/// One ease for the whole bar: 180 ms in, 140 ms out, no bounce.
pub(super) const EASE_IN: Spring = Spring::new(0.18, 0.0);
const EASE_OUT: Spring = Spring::new(0.14, 0.0);

/// Fades in while sliding up `rise` points, and out the same way.
pub(super) fn rise(rise: f32) -> Transition {
    slide(rise, SpringPair::new(EASE_IN, EASE_OUT))
}

fn slide(rise: f32, springs: SpringPair) -> Transition {
    Transition::offset(0.0, rise)
        .combined(Transition::opacity())
        .springs(springs)
}

fn keep_question_focus(s: &mut State, tx: &mut Tx) {
    s.question.focused = true;
    tx.focus_when_shown("ask-input", tx.focus_visible());
}

fn submit(s: &mut State, question: &str, tx: &mut Tx) {
    s.send(question);
    keep_question_focus(s, tx);
}

#[rustfmt::skip]
fn type_in_question(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    let typing = matches!(&e.key, Key::Text(t) if t != " ") && !((e.mods.cmd || e.mods.ctrl) && (cfg!(target_os = "macos") || !e.mods.alt)); // Space presses the focused row.
    if typing { s.ask_settings.scope_open = false; s.question.key(e, None); keep_question_focus(s, tx); } typing }

pub(super) fn focus_open(s: &mut State, tx: &mut Tx) -> bool {
    let open = s.question.focused
        || tx.focused().is_some_and(|key| {
            // The meeting page also uses this key for a tab outside the pill.
            let live_transcript = key == "meeting-transcript" && s.on_live();
            matches!(
                key,
                "ask-model" | "ask-endpoint-settings" | "ask-options" | "ask-send" | "ask-stop"
            ) || matches!(key, "meeting-pause" | "meeting-resume" | "meeting-generate")
                || live_transcript
                || key.starts_with("ask-options-")
                || key.starts_with("ask-quick-")
                || key.starts_with("ask-model-picker-")
        });
    let anchored = open && s.question.text.trim().is_empty() && !s.asks.contains_key(&s.ask_key());
    if s.ask_settings.scope_open && !anchored {
        s.ask_settings.scope_open = false;
        keep_question_focus(s, tx);
    }
    if open != s.ask_open {
        // Focus is reported by a later poll, after the input event that caused it.
        tx.animated();
        tx.motion(SNAP);
    }
    open
}

/// "Thinking, 12 s", then "Thinking, 1 min 5 s".
fn elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("Thinking, {secs} s")
    } else {
        format!("Thinking, {} min {} s", secs / 60, secs % 60)
    }
}

/// What a running Ask shows: a quiet line with the seconds so far and the step it is on.
fn status(run: &AskRun) -> String {
    if run.cancel.stopped() {
        "Stopping...".to_string()
    } else if run.starting_codex {
        "Starting Codex\u{2026}".into()
    } else if !run.turn_started {
        run.steps
            .last()
            .cloned()
            .unwrap_or_else(|| "Reading the meeting...".into())
    } else {
        elapsed(run.started.elapsed().as_secs())
    }
}
fn progress(run: &AskRun, k: &Kit, p: Palette) -> El {
    let head = status(run);
    let step = if run.starting_codex || !run.turn_started {
        String::new()
    } else {
        run.steps
            .last()
            .map(|s| format!("{}...", s.trim_end_matches('.')))
            .unwrap_or_default()
    };
    row()
        .key("ask-progress")
        .transition(rise(4.0))
        .w_full()
        .gap(8.0)
        .items_center()
        .px(8.0)
        .pt(4.0)
        .pb(14.0)
        .child(Spinner::new().size(14.0).color(p.faint).build(k))
        .child(roles::CONTROL.wrap(head, p.soft))
        .child(
            roles::META
                .wrap(step, p.faint)
                .key("ask-step")
                .role("status")
                .min_w(0.0),
        )
}

pub(super) fn raised(e: El, p: Palette) -> El {
    e.fill(p.card)
        .border(1.0, p.line)
        .shadow(8.0, 28.0, p.shadow)
}

fn thread(s: &State, k: &Kit, id: &str, p: Palette) -> Option<El> {
    let run = s.asks.get(id);
    let error = s.ask_errors.get(id);
    if s.saved.messages.is_empty() && run.is_none() && error.is_none() {
        return None;
    }
    let hidden = s.thread_hidden && run.is_none();
    if !hidden {
        super::response_time::visible_operation(
            run.map_or(s.ask_frame_response_time, |r| r.response_time),
        );
    }
    let mut actions = row().gap(2.0).items_center().child(
        quiet_button(if hidden { "Show" } else { "Hide" }, "ask-toggle", p)
            .on_press(|s: &mut State| s.thread_hidden = !s.thread_hidden),
    );
    if run.is_none() {
        actions = actions.child(
            quiet_button("Clear", "ask-clear", p).on_press(|s: &mut State| s.clear_thread()),
        );
    }
    let asked = s.saved.messages.iter().filter(|m| m.role == "user").count();
    let about = if s.ask_meeting_id().is_some() || s.ask_current_live() {
        "this meeting"
    } else {
        "all your meetings"
    };
    let title = if s.ask_current_live() {
        "Asked about this meeting".into()
    } else if asked > 0 {
        format!(
            "Asked about {about} \u{b7} {asked} question{}",
            if asked == 1 { "" } else { "s" }
        )
    } else {
        format!("Ask about {about}")
    };
    let header = row()
        .w_full()
        .justify_between()
        .items_center()
        .pl(16.0)
        .pr(8.0)
        .py(8.0)
        .gap(8.0)
        // The caption takes what the actions leave and wraps; Hide and Clear keep their own room.
        .child(
            roles::META
                .medium()
                .wrap(&title, p.soft)
                .key("ask-caption")
                .grow(1.0)
                .min_w(0.0),
        )
        .child(actions);
    let panel = raised(
        col()
            .key("ask-thread")
            .w_full()
            .radius(22.0)
            .transition(slide(12.0, pair::GLIDE))
            .child(header),
        p,
    );
    let panel = if s.on_live() {
        panel
            .max_h(super::live::limits(s).thread)
            .min_h(0.0)
            .shrink()
            .clip_rounded(22.0)
    } else {
        panel
    };
    if hidden {
        return Some(panel);
    }
    let mut messages = col()
        .w_full()
        .px(10.0)
        .py(2.0)
        .growth(rune::GrowthSpec::default());
    for (i, m) in s.saved.messages.iter().enumerate() {
        if i > 0 && m.role == "user" {
            messages = messages.child(hairline(p));
        }
        messages = messages.child(message(s, k, i, m, p));
    }
    if let Some(run) = run {
        if run.text.is_empty() {
            messages = messages.child(progress(run, k, p));
        } else {
            messages = messages.child(
                message(
                    s,
                    k,
                    s.saved.messages.len(),
                    &Message {
                        role: "assistant".into(),
                        text: run.text.clone(),
                        runner: Some(run.runner.clone()),
                        model: run.reported_model.clone(),
                        requested_model: run.requested_model.clone(),
                        fallback: run.fell_back,
                        ..Default::default()
                    },
                    p,
                )
                .role("status"),
            );
        }
    }
    let unanswered = run.is_none()
        && error.is_none()
        && s.saved.messages.last().is_some_and(|m| m.role == "user");
    if unanswered {
        messages = messages.child(
            roles::META
                .wrap(
                    "No answer was saved for this question. Ask it again to retry.",
                    p.faint,
                )
                .px(8.0)
                .pb(14.0),
        );
    }
    if let Some(e) = error {
        messages = messages.child(error_controls(s, id, e, p));
    }
    messages = messages.children(endpoint_details(s, id, p));
    if s.saved.messages.iter().any(|m| m.role == "assistant")
        || run.is_some_and(|r| !r.text.is_empty())
    {
        messages = messages.child(
            roles::META
                .wrap("Answers come from your meetings and can be wrong.", p.faint)
                .key("ask-note")
                .px(8.0)
                .py(6.0),
        );
    }
    // Leave room for the document and any open transcript above the dock.
    let max_h = if s.on_live() {
        (super::live::limits(s).thread - 44.0).max(0.0)
    } else if s.transcript_open && !s.meeting_ui.tabs {
        (s.pane_h.get() * 0.22).clamp(96.0, 220.0)
    } else {
        (s.pane_h.get() * 0.5).clamp(180.0, 460.0)
    };
    let list = col()
        .key("thread-scroll")
        .w_full()
        .max_h(max_h)
        .min_h(0.0)
        .shrink()
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(messages);
    Some(
        panel
            .max_w(COLUMN)
            .self_center()
            .child(hairline(p))
            .child(list),
    )
}

fn error_controls(s: &State, id: &str, error: &str, p: Palette) -> El {
    let mut row = row().w_full().flex_wrap().gaps(8.0, 6.0);
    let connection = &s.ask_settings.connection;
    if !connection.labels.contains_key(id) {
        if let Some(question) = s.ask_settings.retry_questions.get(id) {
            let question = question.clone();
            row = row.child(
                button("Try again", "ask-retry", ButtonRole::Secondary, p)
                    .button_disabled(
                        !s.question.text.is_empty() && s.question.text != question.text,
                    )
                    .on_press_tx(move |s: &mut State, tx| {
                        s.send_question(&question.text, question.id.as_deref());
                        keep_question_focus(s, tx);
                    }),
            );
        }
        if s.has_agent() {
            row = row.child(
                button(
                    "Choose a model",
                    "ask-error-model",
                    ButtonRole::Secondary,
                    p,
                )
                .on_press(|s: &mut State| {
                    if s.ask_settings.connection.settings.enabled {
                        s.open_connection_settings();
                    } else {
                        s.runner_open = true;
                        s.ask_settings.scope_open = false;
                        s.reload_runners();
                    }
                }),
            );
        }
    }
    col()
        .w_full()
        .px(8.0)
        .pb(8.0)
        .gap(6.0)
        .child(
            roles::META
                .medium()
                .wrap(format!("Could not answer. {error}"), p.warn)
                .role("alert"),
        )
        .child(row)
}

fn endpoint_details(s: &State, id: &str, p: Palette) -> Option<El> {
    let connection = &s.ask_settings.connection;
    let label = connection.labels.get(id)?;
    let recovery = super::cloud_ask::recovery(s, p);
    let recovering = recovery.is_some();
    let open = s.ask_settings.details_open.as_deref() == Some(id);
    let scope = id.to_string();
    let mut details = col().w_full().px(8.0).pb(8.0).children(recovery).child(
        button("Details", "ask-endpoint-details", ButtonRole::Secondary, p)
            .self_start()
            .expanded(open)
            .on_press(move |s: &mut State| {
                s.ask_settings.details_open = (!open).then(|| scope.clone())
            }),
    );
    if open {
        let selected = connection.settings.label();
        let previous =
            !s.asks.contains_key(id) && (!connection.settings.enabled || label != &selected);
        let label = if previous {
            format!("Last attempt: {label}")
        } else {
            label.clone()
        };
        details = details
            .child(roles::META.wrap(label, p.soft).key("ask-endpoint-label"))
            .children((previous && connection.settings.enabled).then(|| {
                roles::META
                    .wrap(format!("Next Ask: {selected}"), p.soft)
                    .key("ask-next-endpoint-label")
            }))
            .children(connection.metrics.get(id).map(|metrics| {
                let label = if recovering {
                    super::cloud_ask::known_metrics_label(metrics)
                } else {
                    metrics.label()
                };
                roles::META.wrap(label, p.faint).key("ask-endpoint-timing")
            }));
    }
    Some(details)
}

fn prompts(s: &State) -> (&'static str, &'static [&'static str]) {
    if s.companies.open {
        (
            "Where do things stand?",
            &["What are they waiting on from us?", "Prep my next call"][..],
        )
    } else if s.on_live()
        && (s.ask_current_live() || s.ask_meeting_id().is_some())
        && s.live
            .as_ref()
            .is_some_and(|live| live.phase == super::live::Phase::Recording)
    {
        ("What did I miss", SUGGESTIONS)
    } else if s.ask_meeting_id().is_some() {
        ("What did we decide?", SUGGESTIONS)
    } else {
        use super::nav::{Filter, Page};

        let prompt = match s.page() {
            Page::Home => "What's on my plate this week?",
            Page::Tasks => "What is late this week?",
            Page::People => "Who am I waiting on?",
            Page::Memory => "What is in memory?",
            Page::Check => "Draft my daily update",
            Page::Annotations => "Find my notes about a project",
            Page::Hex => "Turn on live captions",
            Page::Meetings
                if s.config.features.projects
                    && matches!(s.filter, Filter::Area(_) | Filter::Areas) =>
            {
                "Which projects went quiet?"
            }
            _ => "List my tasks",
        };
        (prompt, ask_all::SUGGESTIONS)
    }
}

const OPTIONS_KEY: &str = "ask-options";
const PICKER_KEY: &str = "ask-model-picker";
const PICKER_DEFAULT_KEY: &str = "ask-model-picker-default";

#[derive(Default)]
struct AskMenu {
    rows: Vec<El>,
}

impl AskMenu {
    fn item(&mut self, row: El) {
        self.rows.push(row);
    }

    fn section(&mut self, label: impl Into<String>, p: Palette) {
        self.rows
            .push(roles::META.line(label, p.faint).px(8.0).pt(6.0).pb(3.0));
    }
}

fn ask_menu_row(name: impl Into<String>, p: Palette, detail: Option<&str>) -> El {
    ask_menu_focus(menu_row(name, p, detail, None, None), p)
}

fn ask_menu_focus(row: El, p: Palette) -> El {
    row.focus_fill(p.sel).focus_ring(2.0, -2.0, p.accent)
}

fn ask_menu_checked(row: El, p: Palette, checked: bool) -> El {
    let item = row
        .role("menuitemcheckbox")
        .checked(checked)
        .focus_selected(checked);
    if !checked {
        return item;
    }
    item.fill(p.sel)
        .child(icons::draw(&icons::CHECK, 14.0, p.soft).a11y_hidden())
}

fn ask_menu_disabled(row: El) -> El {
    row.disabled(true).focusable(false).layer_opacity(0.5)
}

fn mount_ask_menu(
    s: &State,
    k: &Kit,
    p: Palette,
    layer: &mut Vec<El>,
    menu: AskMenu,
    (key, label, anchor, gutter): (&'static str, &'static str, &'static str, f32),
    close: fn(&mut State),
) {
    let motion = if s.ask_settings.reduced_motion.get() {
        Transition::none()
    } else {
        Transition::opacity().springs(pair::SNAP)
    };
    let width = 240.0_f32.min((s.app_w.get() - 16.0).max(0.0));
    let rows = col()
        .padding(4.0)
        .items_stretch()
        .children(menu.rows)
        .instant_changes();
    let scroll = rune_kit::popover::fit_scroll(rows);
    let surface = col()
        .key(format!("{key}-menu"))
        .w(width)
        .min_h(0.0)
        .role("menu")
        .label(label.to_owned())
        .focus_group()
        .radius(10.0)
        .fill(p.raised)
        .child(
            rune_kit::scroll_bar(k, &format!("{key}-scroll-area"), "scroll", scroll)
                .w_full()
                .shrink()
                .min_h(0.0),
        )
        .shadow(8.0, 24.0, p.shadow)
        .box_shadow(Shadow {
            spread: 1.0,
            color: p.line,
            ..Shadow::default()
        })
        .anchored_fit(anchor, Side::Top, Align::End, gutter)
        .transition(motion)
        .token_motion(k.motion.overlay)
        .focus_return(if key == PICKER_KEY && model_chip_visible(s) {
            "ask-model"
        } else {
            OPTIONS_KEY
        })
        .on_key(move |s, e, tx| key == OPTIONS_KEY && type_in_question(s, e, tx))
        .modal(true, close);
    layer.push(rune_kit::scrim(format!("{key}-scrim"), close));
    layer.push(surface);
}

fn mount_ask_menus(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, shown: usize) {
    if s.runner_open && !s.ask_settings.connection.settings.enabled {
        mount_ask_menu(
            s,
            k,
            p,
            layer,
            runner_choice(s, p),
            (PICKER_KEY, "Ask model", "ask-pill", 8.0),
            |s| {
                s.runner_open = false;
            },
        );
    } else if s.ask_settings.scope_open && shown > 0 {
        mount_ask_menu(
            s,
            k,
            p,
            layer,
            scope_choices(s, p, shown),
            (OPTIONS_KEY, "Ask options", OPTIONS_KEY, 6.0),
            |s| {
                s.ask_settings.scope_open = false;
            },
        );
    }
}

fn prompt_list(s: &State) -> Vec<&'static str> {
    let (prompt, more) = prompts(s);
    std::iter::once(prompt)
        .chain(more.iter().copied().filter(|p| *p != prompt))
        .collect()
}
fn quick_chip_count(s: &State, reading: bool) -> usize {
    3 - usize::from(reading || s.app_w.get() - super::nav::width(s) < 652.0)
}
fn shown_quick_chips(s: &State, pending: bool, reading: bool) -> usize {
    let empty = s.question.text.trim().is_empty();
    if !card_open(s) || !empty || pending {
        return 0;
    }
    quick_chip_count(s, reading).min(prompt_list(s).len())
}
fn scope_choices(s: &State, p: Palette, shown: usize) -> AskMenu {
    let mut menu = AskMenu::default();
    menu.section("Prompts", p);
    for prompt in prompt_list(s).into_iter().skip(shown) {
        menu.item(
            ask_menu_row(prompt, p, None)
                .key(format!("{OPTIONS_KEY}-prompt-{prompt}"))
                .on_press_tx(move |s: &mut State, tx| {
                    s.ask_settings.scope_open = false;
                    submit(s, prompt, tx);
                }),
        );
    }
    menu.item(
        ask_menu_row("All prompts", p, None)
            .key("ask-options-all-prompts")
            .on_press(|s: &mut State| {
                s.ask_settings.scope_open = false;
                s.open_hex();
                s.hex.section = super::hex::Section::PromptLibrary;
                super::scene_scroll(s, "agent-instructions");
            }),
    );
    if !s.companies.open {
        let this = s.ask_current_live() || s.ask_meeting_id().is_some();
        let this_meeting = ask_menu_checked(ask_menu_row("This meeting", p, None), p, this)
            .key("ask-options-this-meeting");
        let this_meeting = if s.selected.is_none() && !s.live_scope_available() {
            ask_menu_disabled(this_meeting)
        } else {
            this_meeting.on_press(|s: &mut State| s.set_ask_scope(false))
        };
        menu.rows.push(menu_separator(p));
        menu.section("Ask scope", p);
        menu.item(this_meeting);
        menu.item(
            ask_menu_checked(ask_menu_row("All meetings", p, None), p, !this)
                .key("ask-options-all-meetings")
                .on_press(|s: &mut State| s.set_ask_scope(true)),
        );
    }
    menu
}

fn open_model_picker(s: &mut State, tx: &mut Tx) {
    s.ask_settings.scope_open = false;
    s.runner_open = true;
    s.reload_runners();
    tx.focus_when_shown(PICKER_DEFAULT_KEY, tx.by_keys());
}

fn model_chip_visible(s: &State) -> bool {
    card_open(s) && (s.ask_settings.connection.settings.enabled || s.runner_error.is_none())
}

fn model_chip(
    s: &State,
    k: &Kit,
    p: Palette,
    pending: bool,
    width: f32,
    layer: &mut Vec<El>,
) -> El {
    let connection = s.ask_settings.connection.settings.enabled;
    let hint = super::widgets::HelpHint::Model.build(k, p, layer);
    let label = roles::MEETING_CONTROL.medium().line(short_model(s), p.soft);
    pressable(
        row()
            .items_center()
            .gap(2.0)
            .child(label.shrink())
            .child(icons::draw(&icons::CHEVRON_DOWN, 12.0, p.soft)),
    )
    .key("ask-model")
    .label(format!("Model: {}", picker_label(s)))
    .h(26.0)
    .max_w(width)
    .shrink()
    .pl(if width < 30.0 { 0.0 } else { 10.0 })
    .pr(if width < 30.0 { 0.0 } else { 6.0 })
    .radius(13.0)
    .hover(p.button_hover)
    .focus_ring(1.0, 1.0, p.faint)
    .preserve_focus_on_press(true)
    .hover_group_shared("help-hint-model")
    .child(
        hint.pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .inert()
            .dim(0.0),
    )
    .on_press_tx(move |s: &mut State, tx| {
        if connection {
            s.open_connection_settings();
        } else {
            open_model_picker(s, tx);
        }
    })
    .button_disabled(connection && pending)
}

fn short_model(s: &State) -> String {
    let connection = &s.ask_settings.connection.settings;
    if connection.enabled {
        let model = connection.model.trim();
        return if model.is_empty() {
            "Set up a model"
        } else {
            model
        }
        .into();
    }
    let name = s.ask_runner_name().unwrap_or("Choose a model");
    s.runners.iter().find(|r| r.name == name).map_or_else(
        || short_runner(name).to_owned(),
        |r| model_label(r, s.ask_model().as_deref()),
    )
}

/// "Claude Sonnet (answer only)" reads as "Sonnet": the picker stays small. What the
/// suffix said moves to a tooltip.
fn short_runner(name: &str) -> &str {
    let name = name.strip_prefix("Claude ").unwrap_or(name);
    name.strip_suffix(" (answer only)").unwrap_or(name)
}

/// A model's menu name; a helper that lists none shows its own name.
fn model_label(runner: &core::Runner, model: Option<&str>) -> String {
    model
        .map(|id| {
            core::ask_session::models(runner)
                .into_iter()
                .find(|m| m.id == id)
                .map_or_else(|| id.to_string(), |m| m.name)
        })
        .unwrap_or_else(|| short_runner(&runner.name).to_string())
}

/// What the picker's button says: the helper and model the next question uses. While an answer
/// runs and the choice has changed since it started, it says "Next answer: ...", so the
/// answer on screen is never mistaken for the new choice.
pub(super) fn picker_label(s: &State) -> String {
    if s.ask_settings.connection.settings.enabled {
        return short_model(s);
    }
    let current = choice_label(s, s.ask_runner_name(), s.ask_model().as_deref());
    match s.asks.get(&s.ask_key()) {
        Some(run)
            if Some(run.runner.as_str()) != s.ask_runner_name() || run.model != s.ask_model() =>
        {
            format!("Next answer: {current}")
        }
        _ => current,
    }
}

/// "Codex \u{b7} GPT-6.1-Sol", or "Codex \u{b7} provider default" when no model is set.
fn choice_label(s: &State, name: Option<&str>, model: Option<&str>) -> String {
    let Some(name) = name else {
        return "Choose a model".into();
    };
    let Some(runner) = s.runners.iter().find(|r| r.name == name) else {
        return name.to_string();
    };
    let agent = core::ask_options::agent_name(runner);
    match model {
        Some(id) => format!("{agent} \u{b7} {}", model_label(runner, Some(id))),
        None => format!("{agent} \u{b7} provider default"),
    }
}

fn picker_choice(name: &str, p: Palette, detail: Option<&str>, checked: bool) -> El {
    // The pointer can still be over a row when the options menu becomes the picker.
    let row = menu_row_base(name, p, detail, None, None)
        .hover(p.chip)
        .focus_within_flat();
    ask_menu_checked(ask_menu_focus(row, p), p, checked)
}

pub(super) fn picker_model_row(label: &str, p: Palette, chosen: bool) -> El {
    picker_choice(label, p, None, chosen).help(label.to_owned())
}

fn runner_choice(s: &State, p: Palette) -> AskMenu {
    let overridden = if s.ask_live_meeting() {
        s.ask_settings.meeting_override.is_some()
    } else {
        s.ask_settings.pick.is_some()
    };
    let (default_runner, default_model) = super::ask_settings::default_choice(s);
    let mut menu = AskMenu::default();
    let default = choice_label(s, default_runner.as_deref(), default_model.as_deref());
    menu.item(
        picker_choice("Default", p, Some(&default), !overridden)
            .key(PICKER_DEFAULT_KEY)
            .on_press(|s: &mut State| s.choose_ask_default()),
    );
    for (agent, runner, models) in super::ask_settings::picker_groups(s) {
        menu.section(agent, p);
        for (label, id) in models {
            let chosen =
                overridden && s.ask_runner_name() == Some(runner.as_str()) && s.ask_model() == id;
            let runner = runner.clone();
            menu.item(
                picker_model_row(&label, p, chosen)
                    .key(format!("{PICKER_KEY}-model-{runner:?}-{id:?}"))
                    .on_press(move |s: &mut State| s.choose_ask_pick(runner.clone(), id.clone())),
            );
        }
    }
    let thinks = s
        .runners
        .iter()
        .find(|r| Some(r.name.as_str()) == s.ask_runner_name())
        .is_some_and(core::ask_options::supports_thinking);
    if thinks && !s.ask_settings.connection.settings.enabled {
        menu.rows.push(menu_separator(p));
        menu.section("Thinking effort", p);
        let value = s.ask_thinking();
        for level in core::ask_options::Thinking::ALL {
            menu.item(
                picker_choice(level.label(), p, None, level == value)
                    .key(format!("{PICKER_KEY}-thinking-{}", level.label()))
                    .on_press(move |s: &mut State| {
                        s.set_ask_thinking(level);
                        s.runner_open = false;
                    }),
            );
        }
    }
    // Helpers that are not installed stay listed, marked, so a missing one is not a mystery.
    let mut missing: Vec<&str> = s
        .runners
        .iter()
        .filter(|r| !s.runner_found.contains(&r.name))
        .map(core::ask_options::agent_name)
        .filter(|agent| {
            !s.runners.iter().any(|r| {
                s.runner_found.contains(&r.name) && core::ask_options::agent_name(r) == *agent
            })
        })
        .collect();
    missing.sort_unstable();
    missing.dedup();
    for agent in missing {
        let unavailable = ask_menu_row(agent, p, Some("Not installed"))
            .key(format!("{PICKER_KEY}-unavailable-{agent}"));
        menu.item(ask_menu_disabled(unavailable));
    }
    menu
}

pub(super) const DOCK_BUTTON: f32 = 32.0;
const PILL_HEIGHT: f32 = 48.0;
const PROMPT_HEIGHT: f32 = 28.0;
const PROMPT_GAP: f32 = 4.0;

const CARD_RADIUS: f32 = 18.0;
const FIELD_RADIUS: f32 = 14.0;
const CARD_PAD: (f32, f32, f32) = (10.0, 8.0, 8.0);
const LABEL_ROW: f32 = 22.0;
const QUICK_ROW: f32 = 34.0;

fn scope_text(s: &State) -> &'static str {
    if s.companies.open {
        "This company"
    } else if s.ask_current_live() || s.ask_meeting_id().is_some() {
        "This meeting"
    } else {
        "All meetings"
    }
}

fn card_open(s: &State) -> bool {
    s.ask_open || !s.question.text.trim().is_empty()
}
#[rustfmt::skip]
fn quick_chips(s: &State, p: Palette, compact: bool, reading: bool) -> El {
    let chip = |e: El| pressable(e).h(24.0).px(10.0).radius(12.0).hover(p.button_hover).preserve_focus_on_press(true);
    let more = chip(row().items_center().gap(2.0).child(roles::MEETING_CONTROL.line("More", p.soft)).child(icons::draw(&icons::CHEVRON_DOWN, 12.0, p.soft)))
        .key(OPTIONS_KEY).label("Ask options").description("Choose the question scope or a suggested prompt.");
    let more = super::widgets::menu_trigger(more, s.ask_settings.scope_open, p, |s| { s.ask_settings.scope_open = !s.ask_settings.scope_open; }).focus_ring(1.0, 1.0, p.faint);
    let chips = prompt_list(s).into_iter().take(quick_chip_count(s, reading)).enumerate().map(|(index, prompt)| {
        chip(row().items_center().child(roles::MEETING_CONTROL.line(prompt, p.ink)))
            .key(format!("ask-quick-{index}")).label(format!("Ask: {prompt}"))
            .min_w(0.0).fill(p.chip).shrink().on_press_tx(move |s: &mut State, tx| submit(s, prompt, tx))
    });
    row().w_full().h(QUICK_ROW).px(6.0).pb(8.0).items_center().gap(6.0).children(chips).child(more).child(rune::el().grow(1.0)).children((!compact).then(|| roles::MEETING_META.line(scope_text(s), p.faint).key("ask-scope")))
}
/// The round button at the pill's right end: a send arrow once there is text, a Stop square
/// while a question runs.
fn action(s: &State, p: Palette, pending: bool, empty: bool) -> El {
    let slot = col()
        .key("ask-action")
        .size(DOCK_BUTTON, DOCK_BUTTON)
        .shrink();
    let replace = Transition::blur_replace().springs(pair::SNAP);
    let quiet = Palette {
        button_fill: p.chip,
        button_ink: p.ink,
        button_primary_hover: p.button_hover,
        button_primary_pressed: p.button_pressed,
        ..p
    };
    if pending {
        let run = s.asks.get(&s.ask_key());
        let stopping = run.is_some_and(|r| r.cancel.stopped());
        let who = run.map_or("the answer", |r| short_runner(&r.runner));
        return slot.child(
            super::widgets::icon_button(
                &super::icons::STOP,
                "Stop",
                "ask-stop",
                super::widgets::ButtonRole::Primary,
                quiet,
            )
            .help(format!("Stop {who}"))
            .size(DOCK_BUTTON, DOCK_BUTTON)
            .radius(DOCK_BUTTON / 2.0)
            .focus_ring(1.0, 1.0, p.faint)
            .preserve_focus_on_press(true)
            .transition(replace)
            .opacity_motion(SNAP)
            .dim(if stopping { 0.5 } else { 1.0 })
            .on_press_tx(|s: &mut State, tx| {
                s.stop_ask();
                keep_question_focus(s, tx);
            }),
        );
    }
    let send = super::widgets::icon_button(
        &super::icons::ARROW_UP,
        "Ask (Enter)",
        "ask-send",
        super::widgets::ButtonRole::Primary,
        if empty {
            Palette {
                button_ink: p.faint,
                ..quiet
            }
        } else {
            p
        },
    )
    .size(DOCK_BUTTON, DOCK_BUTTON)
    .radius(DOCK_BUTTON / 2.0)
    .focus_ring(1.0, 1.0, p.faint)
    .preserve_focus_on_press(true)
    .transition(replace)
    // Keep the fill and arrow in sync with availability on the first frame.
    .motion(Spring::NONE)
    .disabled(empty)
    .on_press_tx(|s: &mut State, tx| {
        let q = s.question.text.clone();
        submit(s, &q, tx);
    });
    slot.child(if empty { send.inert() } else { send })
}

/// Pinned to the bottom of the meeting pane or the home page, over what is behind it.
pub fn overlay(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    overlay_with(s, k, p, narrow, layer, None, None)
}

/// The bar with room for more: `lead` sits beside the pill, and
/// `above` floats over the thread. The live page puts its recording pill and transcript here.
pub fn overlay_with(
    s: &State,
    k: &Kit,
    p: Palette,
    _narrow: bool,
    layer: &mut Vec<El>,
    lead: Option<El>,
    above: Option<El>,
) -> El {
    overlay_at(s, k, p, layer, lead, above, None)
}

pub(super) struct ReadingPlacement {
    pub center_offset: f32,
    pub yield_thread: bool,
}

pub(super) fn reading_overlay(
    s: &State,
    k: &Kit,
    p: Palette,
    layer: &mut Vec<El>,
    lead: Option<El>,
    placement: ReadingPlacement,
) -> El {
    overlay_at(s, k, p, layer, lead, None, Some(placement))
}

fn overlay_at(
    s: &State,
    k: &Kit,
    p: Palette,
    layer: &mut Vec<El>,
    lead: Option<El>,
    above: Option<El>,
    reading: Option<ReadingPlacement>,
) -> El {
    let id = s.ask_key();
    let pending = s.asks.contains_key(&id);
    let pane_width = s.app_w.get() - super::nav::width(s);
    let compact = pane_width < 600.0;
    let gutter = if s.companies.open {
        0.0
    } else if s.selected.is_some() && !s.meeting_ui.tabs {
        super::meeting::gutter(s)
    } else if s.on_live() {
        super::live::gutter(s)
    } else {
        16.0
    };
    let mut stack = col()
        .w_full()
        .max_w(COLUMN + 2.0 * gutter)
        .px(gutter)
        .gap(6.0);
    if s.back.is_some() && !(s.selected.is_some() && !s.meeting_ui.tabs && s.transcript_open) {
        stack = stack.child(
            row().w_full().justify_end().child(
                super::widgets::button(
                    if s.check_back {
                        "\u{2039} Back to the check"
                    } else if !s.citation_returns.is_empty() {
                        "\u{2039} Back to your answer"
                    } else {
                        "\u{2191} Back to the summary point"
                    },
                    "back-to-summary",
                    super::widgets::ButtonRole::Primary,
                    p,
                )
                .on_press_tx(|s: &mut State, tx| s.go_back(tx)),
            ),
        );
    }
    if let Some(panel) = above {
        stack = stack.child(panel);
    }
    if let Some(mut panel) = thread(s, k, &id, p) {
        if s.selected.is_some() && !s.meeting_ui.tabs && !s.on_live() {
            let mut slot = col().key("ask-thread-slot").w_full();
            if reading
                .as_ref()
                .is_some_and(|placement| placement.yield_thread)
            {
                // Keep the scroll node mounted so closing the transcript restores its offset.
                slot = slot.h(0.0);
                panel = panel
                    .pin(Some(0.0), Some(0.0), Some(0.0), None)
                    .opacity(0.0)
                    .inert();
            }
            panel = slot.child(panel);
        }
        stack = stack.child(panel);
    }
    if let Some(notice) = &s.ask_settings.fallback_notice {
        stack = stack.child(
            roles::META
                .wrap(notice, p.soft)
                .key("ask-fallback-notice")
                .role("status")
                .w_full()
                .px(14.0)
                .py(8.0)
                .radius(10.0)
                .fill(p.card)
                .border(1.0, p.line),
        );
    }
    let no_agent = !s.has_agent() && !pending && s.runner_error.is_none();
    let open = !no_agent && card_open(s);
    let empty = s.question.text.trim().is_empty();
    let reading_dock = reading.is_some();
    let compact_lead = open && compact && lead.is_some();
    let (left, right, gap) = if compact_lead {
        (8.0, 6.0, 1.0)
    } else if open {
        (14.0, 8.0, 2.0)
    } else if compact {
        (8.0, 6.0, 2.0)
    } else {
        (22.0, 10.0, 4.0)
    };
    let (editor_min, model_max) = if compact_lead {
        // The paused live dock adds Generate to the two recording controls.
        let paused_live = s.on_live()
            && s.live
                .as_ref()
                .is_some_and(|l| l.phase == super::live::Phase::Paused);
        let lead_width = if paused_live { 120.0 } else { 80.0 };
        let field_overhead = left + right + 4.0 * gap + 2.0 * DOCK_BUTTON;
        let room = pane_width - 2.0 * (gutter + CARD_PAD.1) - lead_width - field_overhead;
        let editor = (room - 30.0).clamp(40.0, 90.0);
        (editor, (room - editor).clamp(12.0, 140.0))
    } else {
        (0.0, 140.0)
    };
    let field = row()
        .key("ask-field")
        .min_h(PILL_HEIGHT)
        .items_center()
        .gap(gap)
        .pl(left)
        .pr(right);
    let radius = if open { CARD_RADIUS } else { PILL_HEIGHT / 2.0 };
    let mut top_slot = col().key("ask-top-slot").w_full();
    if open && empty && !pending {
        top_slot = top_slot.child(quick_chips(s, p, compact, reading_dock));
    } else if open && !compact {
        top_slot = top_slot.child(
            row()
                .w_full()
                .h(LABEL_ROW)
                .px(6.0)
                .pb(6.0)
                .justify_end()
                .child(
                    roles::MEETING_META
                        .line(scope_text(s), p.faint)
                        .key("ask-scope"),
                ),
        );
    }
    let pill = col()
        .key("ask-pill")
        .w_full()
        .max_w(COLUMN)
        .radius(radius)
        .instant_changes()
        .motion(Spring::NONE)
        .child(top_slot);
    let dock = col().key("ask-controls").w_full().gap(PROMPT_GAP);
    let (dock, bottom_inset) = if let Some(placement) = reading {
        // Center alignment divides the difference between the two side margins in half.
        let bias = 2.0 * placement.center_offset;
        (
            dock.max_w(520.0)
                .self_center()
                .ml(bias.max(0.0))
                .mr((-bias).max(0.0)),
            16.0,
        )
    } else {
        (dock, 8.0)
    };
    mount_ask_menus(s, k, p, layer, shown_quick_chips(s, pending, reading_dock));
    if no_agent {
        let field = field.children(lead);
        let field = if super::ai_notes::setup_visible(s) && !s.ai_notes.ask_setup_open {
            field.child(
                super::widgets::action("Ask anything", p)
                    .key("ask-open")
                    .h(DOCK_BUTTON)
                    .on_press(|s: &mut State| s.ai_notes.ask_setup_open = true),
            )
        } else {
            let status = if s.agents_rechecked {
                "Still no AI tool"
            } else {
                "No AI tool found"
            };
            field.child(roles::CONTROL.line(status, p.ink).grow(1.0))
                .child(
                    super::widgets::action("Check again", p)
                        .key("ask-recheck")
                        .h(DOCK_BUTTON)
                        .description("Ask works with Claude Code, Codex or OpenCode. Meeting summaries need Claude Code.")
                        .help("If you just installed an AI tool, check again or restart Sidenote.")
                        .on_press(|s: &mut State| s.refresh_ai_notes()),
                )
        };
        let pill = raised(pill.child(field), p);
        return place(s, stack.child(dock.child(pill)), bottom_inset);
    }
    let input = TextInput::new(&s.question)
        .plain()
        .placeholder(if s.companies.open {
            s.companies
                .selected
                .as_ref()
                .and_then(|id| s.companies.data.catalog.records.get(id))
                .map(|c| format!("Ask about {}", c.name))
                .unwrap_or_else(|| "Ask about this company".into())
        } else {
            "\u{2009}Ask anything".into()
        })
        .label(if s.companies.open {
            "Ask about this company"
        } else if s.ask_meeting_id().is_some() || s.ask_current_live() {
            "Ask about this meeting"
        } else {
            "Ask about your meetings"
        })
        .height(DOCK_BUTTON)
        .text_style(rune::TextStyle::new(13.5, 400.0).lh(20.0))
        .on_edit(|s: &mut State| &mut s.question)
        .on_submit(|s: &mut State, tx| {
            let q = s.question.text.clone();
            submit(s, &q, tx);
        })
        .on_key(|s: &mut State, e: &KeyEvent, tx: &mut Tx| {
            if e.key == Key::Tab && e.mods == rune::Mods::default() {
                if model_chip_visible(s)
                    && !(s.ask_settings.connection.settings.enabled
                        && s.asks.contains_key(&s.ask_key()))
                {
                    tx.focus("ask-model", true);
                } else if !s.question.text.trim().is_empty() && !s.asks.contains_key(&s.ask_key()) {
                    tx.focus("ask-send", true);
                } else {
                    return false;
                }
                return true;
            }
            if e.key != Key::Escape {
                return false;
            }
            if s.ask_settings.scope_open {
                s.ask_settings.scope_open = false;
            } else if s.runner_open {
                s.runner_open = false;
                s.ask_settings.thinking_open = false;
            } else if s.on_live()
                && let Some(live) = s.live.as_mut().filter(|live| live.panel_open)
            {
                live.panel_open = false;
            } else if s.selected.is_some() && !s.meeting_ui.tabs && s.transcript_open {
                s.close_transcript_sheet();
                tx.focus("meeting-transcript", true);
            } else {
                tx.blur();
            }
            true
        })
        .build(k)
        .key("ask-input")
        .grow(1.0)
        .min_w(editor_min);
    let stack = stack.children(s.runner_error.as_ref().map(|error| {
        roles::META
            .wrap(format!("Could not load Ask models. {error}"), p.warn)
            .role("alert")
    }));
    let show_prompt = !card_open(s) && empty && !pending;
    let dock = dock.children(show_prompt.then(|| {
        let prompt = prompts(s).0;
        quiet_button(prompt, "ask-prompt", p)
            .label(format!("Ask: {prompt}"))
            .h(PROMPT_HEIGHT)
            .radius(PROMPT_HEIGHT / 2.0)
            .border(1.0, p.line)
            .fill(p.card)
            .self_start()
            .on_press_tx(move |s: &mut State, tx| submit(s, prompt, tx))
    }));
    let error = s.ask_errors.contains_key(&id);
    let (fill, y, blur) = if open {
        (p.raised, 12.0, 32.0)
    } else {
        (p.card, 4.0, 18.0)
    };
    let pill = pill
        .fill(fill)
        .hover(if open { fill } else { p.chip })
        .shadow(y, blur, p.shadow);
    let (pill, field) = if open {
        let (top, sides, bottom) = CARD_PAD;
        let card = pill.pt(top).px(sides).pb(bottom).box_shadow(Shadow {
            spread: 4.0,
            color: p.faint.alpha(if error { 0.0 } else { 0.16 }),
            ..Shadow::default()
        });
        (card, field.radius(FIELD_RADIUS).fill(p.bg))
    } else {
        (pill, field)
    };
    let field = field
        .children(lead)
        .child(input)
        .children(model_chip_visible(s).then(|| model_chip(s, k, p, pending, model_max, layer)))
        .child(action(s, p, pending, empty));
    let pill = pill
        .border(1.0, if error { p.warn } else { p.line })
        .on_press_tx(|_: &mut State, tx| tx.focus("ask-input", false))
        .child(field);
    place(s, stack.child(dock.child(pill)), bottom_inset)
}

fn place(s: &State, stack: El, bottom_inset: f32) -> El {
    if s.on_live() {
        let space = super::live::limits(s).space;
        col()
            .key("ask-overlay")
            .w_full()
            .max_h(space)
            .min_h(0.0)
            .shrink()
            .items_center()
            .pb(8.0)
            .child(
                stack
                    .key("live-ask-scroll")
                    .max_h((space - 8.0).max(0.0))
                    .min_h(0.0)
                    .shrink()
                    .scroll(rune::ScrollSpec {
                        stick_bottom: false,
                    }),
            )
    } else if !s.companies.open && (s.selected.is_none() || !s.meeting_ui.tabs) {
        col()
            .key("ask-overlay")
            .w_full()
            .min_h(0.0)
            .shrink()
            .items_center()
            .pb(bottom_inset)
            .child(stack.min_h(0.0).shrink())
    } else if s.companies.open {
        col().key("company-ask").w_full().min_w(0.0).child(stack)
    } else {
        col()
            .key("ask-overlay")
            .pin(Some(0.0), None, Some(0.0), Some(0.0))
            .items_center()
            .pb(8.0)
            .child(stack)
    }
}

#[cfg(test)]
mod meeting_agent_tests {
    use super::*;
    #[test]
    fn local_endpoint_failure_shows_recovery_before_the_known_metrics() {
        use sidenote::core::provider::{Metrics, Settings};

        crate::test_paths::isolate();
        let mut state = State::new();
        state.app_w.set(360.0);
        let id = state.ask_key();
        state
            .ask_errors
            .insert(id.clone(), "Endpoint failed".into());
        let connection = &mut state.ask_settings.connection;
        connection.settings = Settings {
            enabled: true,
            ..Settings::local()
        };
        connection
            .labels
            .insert(id.clone(), connection.settings.label());
        connection.metrics.insert(
            id,
            Metrics {
                total_ms: 750,
                completion_tokens: Some(0),
                ..Metrics::default()
            },
        );
        let mut runtime = rune::Runtime::new(
            state,
            |s: &State, _| {
                let p = super::super::theme::palette(true);
                overlay(s, &super::super::theme::kit(&p), p, true, &mut Vec::new())
            },
            (360.0, 900.0),
            rune::Env::default(),
        );
        runtime.frame(0.0);
        let elements = runtime.elements();
        let by_key = |key| elements.iter().find(|e| e.key.as_deref() == Some(key));
        let retry = by_key("ask-connection-recovery-0").unwrap();
        assert_eq!(retry.label.as_deref(), Some("Try again with Local"));
        assert!(by_key("ask-connection-recovery-1").is_none());
        let settings = by_key("ask-connection-recovery-2").unwrap();
        let details = by_key("ask-endpoint-details").unwrap();
        assert!(by_key("ask-endpoint-timing").is_none());
        let [x, y, w, h] = details.rect;
        runtime.pointer_down(x + w / 2.0, y + h / 2.0);
        runtime.pointer_up(x + w / 2.0, y + h / 2.0);
        runtime.frame(0.0);
        let opened = runtime.elements();
        let metrics = opened
            .iter()
            .find(|e| e.key.as_deref() == Some("ask-endpoint-timing"))
            .unwrap();
        assert!(retry.rect[1] + retry.rect[3] <= metrics.rect[1]);
        assert!(settings.rect[1] + settings.rect[3] <= metrics.rect[1]);
        assert_eq!(
            metrics.text.as_deref(),
            Some("Total 0.75s \u{b7} 0 output tokens")
        );
    }

    #[test]
    fn catalog_names_replace_ids_and_unknown_ids_remain_unchanged() {
        let runner =
            serde_json::from_str::<Vec<core::Runner>>(include_str!("../../default-runners.json"))
                .unwrap()
                .into_iter()
                .find(|r| r.name == "Codex")
                .unwrap();
        assert_eq!(model_label(&runner, Some("gpt-6.1-terra")), "GPT-6.1 Terra");
        assert_eq!(
            model_label(&runner, Some("unlisted-custom-model")),
            "unlisted-custom-model"
        );
    }
    #[test]
    fn warming_and_started_turn_have_one_distinct_status() {
        let mut run = AskRun::stand_in("Codex", &[], 14);
        run.starting_codex = true;
        run.turn_started = false;
        assert_eq!(status(&run), "Starting Codex\u{2026}");
        run.starting_codex = false;
        run.turn_started = true;
        assert!(status(&run).starts_with("Thinking,"));
    }
}
