//! The Ask pill keeps its place while prompts, scope, models and answers open above it.
//!
//! The panel and thread use Rune's shared motion tokens and follow the system's reduced motion setting.
use super::{
    AskRun, State, icons,
    meeting::COLUMN,
    roles,
    theme::Palette,
    widgets::{ButtonDisabled, hairline, pressable, quiet_button, spaced_caps},
};
use regex::{Captures, Regex};
use rune::tok::motion::{SNAP, pair};
use rune::{El, Key, KeyEvent, Spring, SpringPair, Transition, Tx, col, row};
use rune_kit::{Kit, Markdown, MarkdownStyle, Menu, MenuItem, Placement, Spinner, TextInput};
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

/// "Codex \u{b7} GPT-6.1-Sol" under an answer: the helper and the model it reported. A helper
/// that reported none reads "Codex (model not reported)", never the model that was asked for.
/// Messages saved before this was recorded have no runner and show nothing.
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
        (None, _) => format!("{agent} (model not reported){fallback}"),
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
    tx.focus("ask-input", tx.focus_visible());
}

fn submit(s: &mut State, question: &str, tx: &mut Tx) {
    s.send(question);
    keep_question_focus(s, tx);
}

pub(super) fn focus_open(s: &State, tx: &mut Tx) -> bool {
    let open = s.question.focused
        || tx.focused().is_some_and(|key| {
            matches!(
                key,
                "ask-model" | "ask-endpoint-settings" | "ask-options" | "ask-send" | "ask-stop"
            ) || key.starts_with("ask-options-")
        });
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
            if s.ask_current_live() {
                roles::META.medium().wrap(&title, p.soft)
            } else {
                spaced_caps(&title, p)
            }
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
                .key("ask-streaming-answer")
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
        messages = messages.child(
            roles::META
                .medium()
                .wrap(format!("Could not answer. {e}"), p.warn)
                .role("alert")
                .px(8.0)
                .pb(14.0),
        );
    }
    // The thread floats over the meeting, so it takes at most about half the pane.
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
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(messages);
    let panel = panel.max_w(COLUMN).self_center();
    Some(with_note(panel.child(hairline(p)).child(list), p))
}

fn prompts(s: &State) -> (&'static str, &'static [&'static str]) {
    if s.companies.open {
        (
            "Where do things stand?",
            &["What are they waiting on from us?", "Prep my next call"][..],
        )
    } else if s.ask_current_live() {
        ("What did I miss", SUGGESTIONS)
    } else if s.ask_meeting_id().is_some() {
        ("What did we decide?", SUGGESTIONS)
    } else {
        ("List my tasks", ask_all::SUGGESTIONS)
    }
}

fn ask_options(s: &State, k: &Kit, p: Palette, chip: bool, layer: &mut Vec<El>) -> El {
    let (prompt, more) = prompts(s);
    let trigger = pressable(
        row()
            .center()
            .child(icons::draw(&icons::MORE, 16.0, p.soft)),
    )
    .key("ask-options")
    .label("Ask options")
    .size(32.0, 44.0)
    .radius(16.0)
    .hover(p.button_hover)
    .focus_ring(1.0, 1.0, p.faint)
    .preserve_focus_on_press(true)
    .on_press(|s: &mut State| {
        s.ask_settings.scope_open = !s.ask_settings.scope_open;
        s.runner_open = false;
    });
    let mut menu = Menu::new("ask-options")
        .trigger(trigger)
        .open(s.ask_settings.scope_open)
        .placement(Placement::TopEnd)
        .min_width(220.0)
        .on_close(|s: &mut State| s.ask_settings.scope_open = false);
    if chip {
        for prompt in std::iter::once(prompt).chain(more.iter().copied().filter(|p| *p != prompt)) {
            menu = menu.item(
                MenuItem::new(prompt).on_select_tx(move |s: &mut State, tx| {
                    s.ask_settings.scope_open = false;
                    submit(s, prompt, tx);
                }),
            );
        }
    }
    menu = menu.item(MenuItem::new("All prompts").on_select(|s: &mut State| {
        s.ask_settings.scope_open = false;
        s.open_hex();
        s.hex.section = super::hex::Section::PromptLibrary;
        super::scene_scroll(s, "agent-instructions");
    }));
    if !s.companies.open {
        let this = s.ask_current_live() || s.ask_meeting_id().is_some();
        menu = menu
            .group_label("Ask scope")
            .item(
                MenuItem::new("This meeting")
                    .checked(this)
                    .disabled(s.selected.is_none() && !s.live_scope_available())
                    .on_select(|s: &mut State| s.set_ask_scope(false)),
            )
            .item(
                MenuItem::new("All meetings")
                    .checked(!this)
                    .on_select(|s: &mut State| s.set_ask_scope(true)),
            );
    }
    if !s.ask_settings.connection.settings.enabled && s.runner_error.is_none() {
        menu = menu.item(
            MenuItem::new("Model")
                .description(picker_label(s))
                .on_select(|s: &mut State| {
                    s.ask_settings.scope_open = false;
                    s.runner_open = true;
                    s.reload_runners();
                }),
        );
    }
    menu.build(k, layer).min_w(0.0).shrink()
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

/// The model picker: one button showing the helper and model, and one popover list. Default
/// first (it follows Settings and shows what it resolves to), then each installed helper's
/// models in a group, then a Thinking switch. It is on the meeting page, the live screen and
/// before the first question of a new chat.
fn runner_choice(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let shown = picker_label(s);

    let hint = super::widgets::HelpHint::Model.build(k, p, layer);
    let trigger = super::widgets::pressable(row().items_center())
        .key("ask-model")
        .min_h(28.0)
        .px(8.0)
        .radius(14.0)
        .gap(4.0)
        .hover(p.button_hover)
        .press(p.button_pressed)
        .children((!narrow).then(|| roles::CONTROL.line(&shown, p.soft)))
        .child(rune::icon(k.glyph.chevron_down, 14.0).color(p.faint))
        .role("combobox")
        .label(format!("Model: {shown}"))
        .description("Choose the model for your next question.")
        .preserve_focus_on_press(true)
        .focus_ring(1.0, 1.0, p.faint)
        .min_w(0.0)
        .max_w(if narrow { 160.0 } else { 200.0 })
        .hover_group_shared("help-hint-model")
        .child(
            hint.pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .dim(0.0)
                .inert(),
        )
        .on_press(|s: &mut State| {
            s.runner_open = !s.runner_open;
            s.ask_settings.thinking_open = false;
            if s.runner_open {
                s.reload_runners();
            }
        });
    let overridden = if s.ask_live_meeting() {
        s.ask_settings.meeting_override.is_some()
    } else {
        s.ask_settings.pick.is_some()
    };
    let (default_runner, default_model) = super::ask_settings::default_choice(s);
    let mut menu = Menu::new("runner")
        .trigger(trigger)
        .open(s.runner_open)
        .placement(Placement::TopEnd)
        .min_width(280.0)
        .on_close(|s: &mut State| s.runner_open = false)
        .item(
            MenuItem::new("Default")
                .description(choice_label(
                    s,
                    default_runner.as_deref(),
                    default_model.as_deref(),
                ))
                .checked(!overridden)
                .on_select(|s: &mut State| s.choose_ask_default()),
        );
    for (agent, runner, models) in super::ask_settings::picker_groups(s) {
        menu = menu.group_label(agent);
        for (label, id) in models {
            let chosen =
                overridden && s.ask_runner_name() == Some(runner.as_str()) && s.ask_model() == id;
            let runner = runner.clone();
            menu = menu.item(
                MenuItem::new(label)
                    .checked(chosen)
                    .on_select(move |s: &mut State| s.choose_ask_pick(runner.clone(), id.clone())),
            );
        }
    }
    let thinks = s
        .runners
        .iter()
        .find(|r| Some(r.name.as_str()) == s.ask_runner_name())
        .is_some_and(core::ask_options::supports_thinking);
    if thinks && !s.ask_settings.connection.settings.enabled {
        menu = menu.group_label("Thinking");
        let value = s.ask_thinking();
        for level in core::ask_options::Thinking::ALL {
            menu = menu.item(
                MenuItem::new(level.label())
                    .checked(level == value)
                    .on_select(move |s: &mut State| {
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
        menu = menu.item(
            MenuItem::new(agent)
                .description("Not installed")
                .disabled(true),
        );
    }
    menu.build(k, layer).min_w(0.0).shrink()
}

const ACTION: f32 = 28.0;

/// The round button at the pill's right end: a send arrow once there is text, a Stop square
/// while a question runs.
fn action(s: &State, p: Palette, pending: bool, empty: bool) -> El {
    let slot = col().key("ask-action").size(ACTION, ACTION).shrink();
    let replace = Transition::blur_replace().springs(pair::SNAP);
    if pending {
        let run = s.asks.get(&s.ask_key());
        let stopping = run.is_some_and(|r| r.cancel.stopped());
        let who = run.map_or("the answer", |r| short_runner(&r.runner));
        slot.child(
            super::widgets::icon_button(
                &super::icons::STOP,
                "Stop",
                "ask-stop",
                super::widgets::ButtonRole::Primary,
                p,
            )
            .help(format!("Stop {who}"))
            .size(ACTION, ACTION)
            .radius(14.0)
            .focus_ring(1.0, 1.0, p.faint)
            .preserve_focus_on_press(true)
            .transition(replace)
            .opacity_motion(SNAP)
            .dim(if stopping { 0.5 } else { 1.0 })
            .on_press_tx(|s: &mut State, tx| {
                s.stop_ask();
                keep_question_focus(s, tx);
            }),
        )
    } else {
        slot.child(
            super::widgets::icon_button(
                &super::icons::ARROW_UP,
                "Ask (Enter)",
                "ask-send",
                super::widgets::ButtonRole::Primary,
                p,
            )
            .size(ACTION, ACTION)
            .radius(14.0)
            .focus_ring(1.0, 1.0, p.faint)
            .preserve_focus_on_press(true)
            .transition(replace)
            .button_disabled(empty)
            .on_press_tx(|s: &mut State, tx| {
                let q = s.question.text.clone();
                submit(s, &q, tx);
            }),
        )
    }
}

/// What the pill says when no agent CLI is on this machine.
fn no_agent(s: &State, p: Palette) -> El {
    let mut note = col().min_w(0.0).gap(2.0).child(roles::CONTROL.wrap(
        "Ask works with Claude Code, Codex or OpenCode. Meeting summaries need Claude Code.",
        p.ink,
    ));
    if s.agents_rechecked {
        note = note.child(roles::META.wrap(
            "Still not found. If you just installed one, press Check again, or restart Sidenote if it still fails.",
            p.faint,
        ));
    }
    let (content, note) = if s.app_w.get() < 600.0 {
        (col().items_start(), note.w_full())
    } else {
        (row().items_center(), note.grow(1.0))
    };
    content
        .key("ask-pill")
        .w_full()
        .gap(10.0)
        .px(20.0)
        .py(10.0)
        .radius(22.0)
        .child(note)
        .child(
            crate::ui::widgets::action("Check again", p)
                .key("ask-recheck")
                .on_press(|s: &mut State| {
                    s.refresh_ai_notes();
                }),
        )
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
    let id = s.ask_key();
    let pending = s.asks.contains_key(&id);
    let compact = s.app_w.get() - super::nav::width(s) < 600.0;
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
                    } else if s.cite_back.is_some() {
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
    let thread = thread(s, k, &id, p);
    if let Some(panel) = thread {
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
    if s.ask_settings.connection.settings.enabled
        || s.ask_settings.connection.labels.contains_key(&id)
    {
        let label = s
            .ask_settings
            .connection
            .labels
            .get(&id)
            .cloned()
            .unwrap_or_else(|| s.ask_settings.connection.settings.label());
        let selected = s.ask_settings.connection.settings.label();
        let mut disclosure = col().w_full().padding(8.0).radius(12.0).fill(p.card);
        let label =
            if !pending && (!s.ask_settings.connection.settings.enabled || label != selected) {
                format!("Last attempt: {label}")
            } else {
                label
            };
        disclosure = disclosure.child(
            roles::META
                .wrap(label, p.soft)
                .key("ask-endpoint-label")
                .role("status")
                .w_full(),
        );
        if !pending
            && s.ask_settings.connection.settings.enabled
            && s.ask_settings
                .connection
                .labels
                .get(&id)
                .is_some_and(|label| label != &selected)
        {
            disclosure = disclosure.child(
                roles::META
                    .wrap(format!("Next Ask: {selected}"), p.soft)
                    .key("ask-next-endpoint-label")
                    .w_full(),
            );
        }
        let recovery = super::cloud_ask::recovery(s, p);
        let recovering = recovery.is_some();
        disclosure = disclosure.children(recovery);
        if let Some(metrics) = s.ask_settings.connection.metrics.get(&id) {
            let label = if recovering {
                super::cloud_ask::known_metrics_label(metrics)
            } else {
                metrics.label()
            };
            disclosure = disclosure.child(
                roles::META
                    .wrap(label, p.faint)
                    .key("ask-endpoint-timing")
                    .w_full(),
            );
        }
        stack = stack.child(disclosure.max_w(COLUMN).self_center());
    }
    if !s.has_agent() && !pending && s.runner_error.is_none() {
        let pill = if super::ai_notes::setup_visible(s) && !s.ai_notes.ask_setup_open {
            raised(
                super::widgets::action("Ask anything", p)
                    .key("ask-open")
                    .px(20.0)
                    .py(10.0)
                    .on_press(|s: &mut State| s.ai_notes.ask_setup_open = true),
                p,
            )
        } else {
            raised(no_agent(s, p), p)
        };
        let (stack, lead) = if compact {
            (stack.children(lead.map(El::self_start)), None)
        } else {
            (stack, lead)
        };
        let dock = col().key("ask-controls").w_full().child(pill);
        return place(s, stack.children(lead).child(dock));
    }
    let empty = s.question.text.trim().is_empty();
    let focused = s.ask_open || s.runner_open;
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
            "Ask anything".into()
        })
        .label(if s.companies.open {
            "Ask about this company"
        } else if s.ask_meeting_id().is_some() || s.ask_current_live() {
            "Ask about this meeting"
        } else {
            "Ask about your meetings"
        })
        .height(44.0)
        .text_style(rune::TextStyle::new(13.5, 400.0).lh(20.0))
        .on_edit(|s: &mut State| &mut s.question)
        .on_submit(|s: &mut State, tx| {
            let q = s.question.text.clone();
            submit(s, &q, tx);
        })
        .on_key(|s: &mut State, e: &KeyEvent, tx: &mut Tx| {
            if e.key == Key::Tab
                && e.mods == rune::Mods::default()
                && !s.ask_settings.connection.settings.enabled
                && s.runner_error.is_none()
            {
                tx.focus_when_shown("ask-model", true);
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
            } else {
                tx.blur();
            }
            true
        })
        .build(k)
        .key("ask-input")
        .grow(1.0)
        .min_w(0.0);
    let model = (focused || pending).then(|| {
        if s.ask_settings.connection.settings.enabled {
            let caption = if compact { "Setup" } else { "Connection" };
            quiet_button(caption, "ask-endpoint-settings", p)
                .label("Connection")
                .button_disabled(pending)
                .help(if pending {
                    "Available after this answer finishes"
                } else {
                    "Open Ask connection settings"
                })
                .on_press(|s: &mut State| {
                    if !s.asks.contains_key(&s.ask_key()) {
                        s.open_connection_settings();
                    }
                })
        } else {
            match &s.runner_error {
                Some(e) => roles::META
                    .medium()
                    .wrap(format!("Runners are not set up. {e}"), p.warn)
                    .px(8.0)
                    .max_w(260.0),
                None => runner_choice(s, k, p, compact, layer),
            }
        }
    });
    let show_prompt = empty && !pending;
    let dock = col()
        .key("ask-controls")
        .w_full()
        .gap(4.0)
        .children(show_prompt.then(|| {
            let prompt = prompts(s).0;
            quiet_button(prompt, "ask-prompt", p)
                .label(format!("Ask options, {prompt}"))
                .h(44.0)
                .radius(22.0)
                .border(1.0, p.line)
                .fill(p.card)
                .self_start()
                .on_press_tx(move |s: &mut State, tx| submit(s, prompt, tx))
        }));
    let (stack, lead) = if s.on_live() && s.app_w.get() < 380.0 {
        (stack.children(lead.map(El::self_start)), None)
    } else {
        (stack, lead)
    };
    let pill = row()
        .key("ask-pill")
        .w_full()
        .max_w(COLUMN)
        .h(70.0)
        .min_h(70.0)
        .gap(if compact { 2.0 } else { 4.0 })
        .items_center()
        .pl(if compact { 8.0 } else { 22.0 })
        .pr(if compact { 6.0 } else { 10.0 })
        .radius(35.0)
        .fill(p.card)
        .focus_within_flat()
        .on_press_tx(|_: &mut State, tx| tx.focus("ask-input", false))
        .children(lead)
        .child(input)
        .children(model)
        .child(ask_options(s, k, p, show_prompt, layer))
        .child(action(s, p, pending, empty));
    place(s, stack.child(dock.child(pill)))
}

fn with_note(stack: El, p: Palette) -> El {
    stack.child(
        roles::META
            .wrap("Answers come from your meetings and can be wrong.", p.faint)
            .key("ask-note")
            .self_center()
            .align(rune::Align::Center),
    )
}

fn place(s: &State, stack: El) -> El {
    if s.on_live() || (s.selected.is_some() && !s.meeting_ui.tabs && !s.companies.open) {
        col()
            .key("ask-overlay")
            .w_full()
            .items_center()
            .pb(8.0)
            .child(stack)
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
        let metrics = by_key("ask-endpoint-timing").unwrap();
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
