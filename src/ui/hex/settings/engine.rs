//! Meeting engine tab: edits the reviewed `engine_settings` model and shows its read-only status.
//!
//! Drafts are plain text. Only a field the person changed is applied, so the reviewed save path
//! validates only that field and a stored value that is already wrong never blocks another edit.
//! Status lines say where a value came from and whether it is valid. They never print a value
//! the status did not come from, so nothing a script import rejected can appear here.
use super::{Page, group_highlight, navigate, panel, row_field_kit};
use crate::ui::{
    State, ai_notes,
    cards::{settings_copy, settings_row},
    roles,
    theme::Palette,
    widgets::{ButtonDisabled, action, row_action},
};
use rune::{El, TextEdit, col, row};
use rune_kit::{Kit, Switch, TextArea, TextInput};
use sidenote::core::{
    Config,
    engine_settings::{EngineSettings, FieldStatus, Prompt, Source, Validation},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const NOT_USED: &str =
    "You can save these choices for later. The built-in engine does not use them yet.";
const NOT_SAVED: &str = "Nothing was saved. Fix the values marked not valid.";
const SAVE_FAILED: &str = "Could not save. Reload Settings and try again.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Field {
    English,
    Hindi,
    Telugu,
    Threads,
    Deadline,
    RequestDeadline,
    Translation,
    Title,
    Tasks,
    Comparison,
    NotesPrompt,
    TranslationPrompt,
}
impl Field {
    pub(super) const TRANSCRIPTION: [Field; 6] = [
        Self::English,
        Self::Hindi,
        Self::Telugu,
        Self::Threads,
        Self::Deadline,
        Self::RequestDeadline,
    ];
    /// Models for the steps after transcription. The notes model has its own existing home.
    const STEP_MODELS: [Field; 4] = [
        Self::Translation,
        Self::Title,
        Self::Tasks,
        Self::Comparison,
    ];
    const PROMPTS: [Field; 2] = [Self::NotesPrompt, Self::TranslationPrompt];
    /// Every field with a draft, in the order of `index`.
    const ALL: [Field; 12] = [
        Self::English,
        Self::Hindi,
        Self::Telugu,
        Self::Threads,
        Self::Deadline,
        Self::RequestDeadline,
        Self::Translation,
        Self::Title,
        Self::Tasks,
        Self::Comparison,
        Self::NotesPrompt,
        Self::TranslationPrompt,
    ];
    const COUNT: usize = 12;
    fn visible(self, config: &Config) -> bool {
        match self {
            Self::Hindi | Self::Telugu => config.features.indic_engine,
            Self::Translation | Self::TranslationPrompt => config.features.translation,
            _ => true,
        }
    }
    fn index(self) -> usize {
        self as usize
    }
    fn language(self) -> Option<&'static str> {
        match self {
            Self::English => Some("en"),
            Self::Hindi => Some("hi"),
            Self::Telugu => Some("te"),
            _ => None,
        }
    }
    /// The row key, also the scroll target for search.
    fn key(self) -> &'static str {
        match self {
            Self::English => "engine-model-en",
            Self::Hindi => "engine-model-hi",
            Self::Telugu => "engine-model-te",
            Self::Threads => "engine-worker-threads",
            Self::Deadline => "engine-transcription-deadline",
            Self::RequestDeadline => "engine-request-deadline",
            Self::Translation => "engine-translation-model",
            Self::Title => "engine-title-model",
            Self::Tasks => "engine-tasks-model",
            Self::Comparison => "engine-comparison-model",
            Self::NotesPrompt => "engine-prompt-notes",
            Self::TranslationPrompt => "engine-prompt-translation",
        }
    }
    /// The name `Config::engine_field_status` uses.
    fn status_key(self) -> &'static str {
        match self {
            Self::English => "models.en",
            Self::Hindi => "models.hi",
            Self::Telugu => "models.te",
            Self::Threads => "worker_threads",
            Self::Deadline => "stage_deadline_seconds",
            Self::RequestDeadline => "stages.asr_request_deadline_seconds",
            Self::Translation => "stages.translation_model",
            Self::Title => "stages.title_model",
            Self::Tasks => "stages.tasks_model",
            Self::Comparison => "stages.comparison_model",
            Self::NotesPrompt => "notes_prompt",
            Self::TranslationPrompt => "translation_prompt",
        }
    }
    /// The name `Config::validate_engine_settings` reports; one name covers every language.
    fn error_name(self) -> &'static str {
        match self {
            Self::English | Self::Hindi | Self::Telugu => "models",
            _ => self.status_key(),
        }
    }
    /// Label, help and the hint shown in an empty box.
    fn text(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::English => (
                "English model",
                "The speech model used for English parts of a meeting.",
                "Default",
            ),
            Self::Hindi => (
                "Hindi model",
                "The speech model used for Hindi parts of a meeting.",
                "Default",
            ),
            Self::Telugu => (
                "Telugu model",
                "The speech model used for Telugu parts of a meeting.",
                "Default",
            ),
            Self::Threads => (
                "Parallel transcription jobs",
                "How many parts of a recording can be transcribed at once. The default is 4.",
                "4",
            ),
            Self::Deadline => (
                "Time limit for a full transcript (seconds)",
                "The longest a whole transcription may take. Leave empty to work it out from the audio length.",
                "Automatic",
            ),
            Self::RequestDeadline => (
                "Time limit for one transcription request (seconds)",
                "How long the engine waits for one request to finish. The default is 180 seconds.",
                "180",
            ),
            Self::Translation => (
                "Translation model",
                "The model that translates speech in other languages into English.",
                "Model name",
            ),
            Self::Title => (
                "Title model",
                "The model that gives a meeting its short title.",
                "Model name",
            ),
            Self::Tasks => (
                "Task model",
                "The model that picks out tasks and follow-ups.",
                "Model name",
            ),
            Self::Comparison => (
                "Comparison model",
                "The model that compares your notes with an outside note taker. That step is off by default.",
                "Model name",
            ),
            Self::NotesPrompt => (
                "Notes prompt",
                "The instructions the notes writer follows to write a meeting summary.",
                "",
            ),
            Self::TranslationPrompt => (
                "Translation prompt",
                "The instructions the translation step follows to turn speech into English.",
                "",
            ),
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::English | Self::Hindi | Self::Telugu => {
                "Not valid. Use letters, numbers and - _ / . : only."
            }
            Self::Translation | Self::Title | Self::Tasks | Self::Comparison => {
                "Not valid. Enter a model name with letters, numbers and - _ / . : only."
            }
            Self::NotesPrompt | Self::TranslationPrompt => {
                "Not valid. A prompt cannot be empty or use control characters."
            }
            Self::Threads => "Not valid. Use a whole number from 1 to 256.",
            Self::Deadline | Self::RequestDeadline => {
                "Not valid. Use a whole number of seconds from 1 to 3600."
            }
        }
    }
}

/// Only the notes writer and model are read by the built-in engine today. Every other control
/// here is stored for the engine that will read it, and says so.
fn in_use(status_key: &str) -> bool {
    matches!(status_key, "notes.helper" | "notes.model")
}
/// A language the person already routed in their language settings keeps that one home.
fn routed(config: &Config, field: Field) -> bool {
    let routes = config.pipeline.after_stop_languages.as_ref();
    field.language().is_some_and(|language| {
        let route = routes.and_then(|routes| routes.routes.get(language));
        route.is_some_and(|route| !route.model.trim().is_empty())
    })
}
fn read(config: &Config, field: Field) -> String {
    let settings = &config.engine_settings;
    match field {
        Field::English | Field::Hindi | Field::Telugu => config
            .engine_model(field.language().unwrap_or_default())
            .unwrap_or_default()
            .to_owned(),
        Field::Threads => settings.worker_threads.to_string(),
        Field::Deadline => settings
            .stage_deadline_seconds
            .filter(|v| *v > 0)
            .map_or(String::new(), |v| v.to_string()),
        Field::RequestDeadline => settings.stages.asr_request_deadline_seconds.to_string(),
        Field::Translation => settings.stages.translation_model.clone(),
        Field::Title => settings.stages.title_model.clone(),
        Field::Tasks => settings.stages.tasks_model.clone(),
        Field::Comparison => settings.stages.comparison_model.clone(),
        Field::NotesPrompt => settings.prompt(Prompt::Notes).trim().to_owned(),
        Field::TranslationPrompt => settings.prompt(Prompt::Translation).trim().to_owned(),
    }
}
/// Text equal to the shipped prompt is stored as no override, so it stays "default".
fn custom_prompt(prompt: Prompt, text: &str) -> Option<String> {
    let default = EngineSettings::default();
    (text != default.prompt(prompt).trim()).then(|| text.to_owned())
}
fn customised(config: &Config, field: Field) -> bool {
    let settings = &config.engine_settings;
    match field {
        Field::NotesPrompt => settings.notes_prompt.is_some(),
        Field::TranslationPrompt => settings.translation_prompt.is_some(),
        _ => false,
    }
}
fn reset_prompt(config: &mut Config, field: Field) {
    let settings = &mut config.engine_settings;
    match field {
        Field::NotesPrompt => settings.reset_prompt(Prompt::Notes),
        Field::TranslationPrompt => settings.reset_prompt(Prompt::Translation),
        _ => {}
    }
}
fn write(config: &mut Config, field: Field, text: &str) -> Result<(), ()> {
    let number = || text.parse::<u32>().map_err(|_| ());
    let settings = &mut config.engine_settings;
    match field {
        Field::English | Field::Hindi | Field::Telugu => {
            let language = field.language().unwrap_or_default().to_owned();
            if text.is_empty() {
                settings.models.remove(&language);
            } else {
                settings.models.insert(language, text.to_owned());
            }
        }
        Field::Threads => settings.worker_threads = number()?,
        Field::Deadline => {
            settings.stage_deadline_seconds = if text.is_empty() {
                None
            } else {
                Some(number()?)
            }
        }
        Field::RequestDeadline => settings.stages.asr_request_deadline_seconds = number()?,
        Field::Translation => settings.stages.translation_model = text.to_owned(),
        Field::Title => settings.stages.title_model = text.to_owned(),
        Field::Tasks => settings.stages.tasks_model = text.to_owned(),
        Field::Comparison => settings.stages.comparison_model = text.to_owned(),
        Field::NotesPrompt => settings.notes_prompt = custom_prompt(Prompt::Notes, text),
        Field::TranslationPrompt => {
            settings.translation_prompt = custom_prompt(Prompt::Translation, text);
        }
    }
    Ok(())
}
/// Would this one edit be valid? Other fields, even ones already wrong, do not count.
fn probe(base: &Config, field: Field, text: &str) -> bool {
    let mut config = base.clone();
    if let Some(language) = field.language() {
        // One error name covers every language, so judge only this one.
        let models = &mut config.engine_settings.models;
        models.retain(|name, _| name == language);
    }
    write(&mut config, field, text).is_ok()
        && !config
            .validate_engine_settings()
            .contains(&field.error_name())
}
/// The base with every changed, valid draft applied. `Ok(None)` means nothing changed.
fn apply(base: &Config, edits: &[(Field, &str)]) -> Result<Option<Config>, Vec<Field>> {
    let mut edited = base.clone();
    let (mut bad, mut changed) = (Vec::new(), false);
    for &(field, text) in edits {
        let text = text.trim();
        if routed(base, field) || text == read(base, field) {
            continue;
        }
        if probe(base, field, text) && write(&mut edited, field, text).is_ok() {
            changed = true;
        } else {
            bad.push(field);
        }
    }
    if bad.is_empty() {
        Ok(changed.then_some(edited))
    } else {
        Err(bad)
    }
}
/// Source and validity in plain words, and whether the line needs attention. The notes model
/// passes the text checks, but nothing here has run it, so it says the name is not checked.
fn status_text(key: &str, status: &FieldStatus) -> (String, bool) {
    let source = match status.source {
        Source::Default => "Using the default",
        Source::Saved => "Saved on this PC",
        Source::LegacyEnvironment => "Imported from your scripts",
        Source::ExistingRoute => "Set in your language settings",
    };
    let hint = match status.validation {
        Validation::Empty => "",
        Validation::Valid if key == "notes.model" => "Model has not been tested on this PC",
        Validation::Valid => "",
        Validation::Invalid => "Check this setting",
        Validation::Unavailable => "Not available on this PC",
        Validation::ModelQualificationRequired => "Model has not been tested on this PC",
    };
    let warn = matches!(
        status.validation,
        Validation::Invalid | Validation::Unavailable
    );
    (
        if hint.is_empty() {
            source.into()
        } else {
            format!("{source} \u{b7} {hint}")
        },
        warn,
    )
}

#[derive(Default)]
pub(super) struct Engine {
    pub(super) unused_open: bool,
    base: Option<Config>,
    drafts: Vec<TextEdit>,
    bad: Vec<Field>,
    error: Option<&'static str>,
    /// The prompt whose Reset to default was pressed once and waits for a second press.
    confirm_reset: Option<Field>,
}
impl Engine {
    fn dirty(&self, fields: &[Field]) -> bool {
        self.base.as_ref().is_some_and(|base| {
            fields.iter().any(|field| {
                let draft = self.drafts.get(field.index());
                let changed = draft.is_some_and(|d| d.text.trim() != read(base, *field));
                changed && !routed(base, *field)
            })
        })
    }
}

/// Start the tab from what is stored now.
pub(super) fn load(s: &mut State) {
    let config = s.config.clone();
    s.hex.shell.engine = Engine {
        drafts: Field::ALL
            .iter()
            .map(|field| TextEdit::new(read(&config, *field)))
            .collect(),
        base: Some(config),
        ..Default::default()
    };
}
/// Come back to the tab, or press the tab you are on. Text the person typed stays, and what they
/// did not touch follows what is stored now, so a visit never loses or invents an edit.
pub(super) fn enter(s: &mut State) {
    if s.hex.shell.engine.base.is_none() || s.hex.shell.engine.drafts.len() < Field::COUNT {
        load(s);
        return;
    }
    let fresh = s.config.clone();
    let engine = &mut s.hex.shell.engine;
    rebase(engine, fresh);
    engine.confirm_reset = None;
    let kept: Vec<Field> = engine
        .bad
        .iter()
        .copied()
        .filter(|field| engine.dirty(&[*field]))
        .collect();
    // The hints follow the drafts still held. A failed disk save is not a hint: it stays until a
    // later save succeeds.
    if engine.error != Some(SAVE_FAILED) {
        engine.error = (!kept.is_empty()).then_some(NOT_SAVED);
    }
    engine.bad = kept;
}
fn draft_mut(s: &mut State, index: usize) -> &mut TextEdit {
    if s.hex.shell.engine.drafts.len() < Field::COUNT {
        load(s);
    }
    &mut s.hex.shell.engine.drafts[index]
}
/// Fields the person changed keep their text. Every untouched field takes what is stored in
/// `fresh`, so it can never turn into an edit that overwrites a newer setting.
fn rebase(engine: &mut Engine, fresh: Config) {
    if let Some(old) = &engine.base {
        for field in Field::ALL {
            if let Some(draft) = engine.drafts.get_mut(field.index())
                && draft.text.trim() == read(old, field)
            {
                *draft = TextEdit::new(read(&fresh, field));
            }
        }
    }
    engine.base = Some(fresh);
}
/// A save went through: its fields take the stored text, other cards keep what was typed, and
/// everything untouched follows the saved config, which may hold a newer setting from elsewhere.
fn adopt(engine: &mut Engine, config: Config, fields: &[Field]) {
    rebase(engine, config.clone());
    for field in fields {
        if let Some(draft) = engine.drafts.get_mut(field.index()) {
            *draft = TextEdit::new(read(&config, *field));
        }
    }
}
/// Reset to default asks twice, because it throws away the person's own text.
fn reset(s: &mut State, field: Field) {
    if !field.visible(&s.config) {
        return;
    }
    if s.hex.shell.engine.confirm_reset != Some(field) {
        s.hex.shell.engine.confirm_reset = Some(field);
        return;
    }
    s.hex.shell.engine.confirm_reset = None;
    s.hex.shell.engine.error = None;
    let mut config = s.hex.shell.engine.base.clone().unwrap_or(s.config.clone());
    reset_prompt(&mut config, field);
    match config.save(&s.store.root) {
        Ok(()) => {
            s.config = config.clone();
            adopt(&mut s.hex.shell.engine, config, &[field]);
            s.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
        }
        Err(_) => s.hex.shell.engine.error = Some(SAVE_FAILED),
    }
}
fn save(s: &mut State, fields: &[Field]) {
    let Some(base) = s.hex.shell.engine.base.clone() else {
        return;
    };
    let drafts: Vec<_> = fields
        .iter()
        .filter_map(|field| {
            if !field.visible(&s.config) {
                return None;
            }
            let draft = s.hex.shell.engine.drafts.get(field.index())?;
            Some((*field, draft.text.clone()))
        })
        .collect();
    let edits: Vec<_> = drafts.iter().map(|(f, t)| (*f, t.as_str())).collect();
    let outcome = apply(&base, &edits);
    s.hex.shell.engine.bad.clear();
    s.hex.shell.engine.error = None;
    s.hex.shell.engine.confirm_reset = None;
    match outcome {
        Err(bad) => {
            s.hex.shell.engine.bad = bad;
            s.hex.shell.engine.error = Some(NOT_SAVED);
        }
        Ok(None) => {}
        Ok(Some(mut config)) => match config.save(&s.store.root) {
            Ok(()) => {
                s.config = config.clone();
                adopt(&mut s.hex.shell.engine, config, fields);
                s.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
            }
            Err(_) => {
                s.hex.shell.engine.error = Some(SAVE_FAILED);
                if let Ok(fresh) = Config::load_in(&s.store.root) {
                    rebase(&mut s.hex.shell.engine, fresh);
                }
            }
        },
    }
}

/// Name, help, status and the not-used line, shared by every kind of row.
fn copy_column(
    p: Palette,
    key: &str,
    text: (&str, &str),
    status_key: &str,
    statuses: &BTreeMap<String, FieldStatus>,
) -> El {
    let mut copy = settings_copy(text.0, p).child(roles::META.wrap(text.1, p.soft));
    if let Some(status) = statuses.get(status_key)
        && in_use(status_key)
    {
        let (line, warn) = status_text(status_key, status);
        let color = if warn { p.warn } else { p.soft };
        copy = copy.child(roles::META.wrap(line, color).key(format!("{key}-status")));
    }
    copy
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Tasks,
    Title,
    Comparison,
}
impl Step {
    const ALL: [Step; 3] = [Self::Tasks, Self::Title, Self::Comparison];
    fn key(self) -> &'static str {
        match self {
            Self::Tasks => "engine-step-tasks",
            Self::Title => "engine-step-title",
            Self::Comparison => "engine-step-comparison",
        }
    }
    /// The name `Config::engine_field_status` uses.
    fn status_key(self) -> &'static str {
        match self {
            Self::Tasks => "tasks",
            Self::Title => "quick_title",
            Self::Comparison => "external_comparison",
        }
    }
    fn text(self) -> (&'static str, &'static str) {
        match self {
            Self::Tasks => ("Tasks", "Pick out tasks and follow-ups after each meeting."),
            Self::Title => ("Meeting title", "Give each meeting a short title."),
            Self::Comparison => (
                "Comparison",
                "Compare your notes with an outside note taker. This is off by default.",
            ),
        }
    }
    fn get(self, config: &Config) -> bool {
        let steps = &config.engine_settings.steps;
        match self {
            Self::Tasks => steps.tasks,
            Self::Title => steps.quick_title,
            Self::Comparison => steps.external_comparison,
        }
    }
    fn set(self, config: &mut Config, on: bool) {
        let steps = &mut config.engine_settings.steps;
        match self {
            Self::Tasks => steps.tasks = on,
            Self::Title => steps.quick_title = on,
            Self::Comparison => steps.external_comparison = on,
        }
    }
}
/// Turning a step on or off saves at once, through the same reviewed path, and keeps any text
/// the person has typed in the other cards.
fn toggle(s: &mut State, step: Step) {
    let mut config = s.hex.shell.engine.base.clone().unwrap_or(s.config.clone());
    let on = !step.get(&config);
    step.set(&mut config, on);
    s.hex.shell.engine.error = None;
    match config.save(&s.store.root) {
        Ok(()) => {
            s.config = config.clone();
            rebase(&mut s.hex.shell.engine, config);
            s.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
        }
        Err(_) => s.hex.shell.engine.error = Some(SAVE_FAILED),
    }
}
fn step_row(
    s: &State,
    k: &Kit,
    p: Palette,
    step: Step,
    statuses: &BTreeMap<String, FieldStatus>,
) -> El {
    let (name, help) = step.text();
    let key = step.key();
    let copy = copy_column(p, key, (name, help), step.status_key(), statuses);
    let on = step.get(&s.config);
    let switch = Switch::new(on)
        .label(name)
        .hide_label()
        .on_toggle(move |s: &mut State| toggle(s, step))
        .build(k)
        .key(format!("{key}-switch"));
    settings_row(true)
        .key(key)
        .fill(group_highlight(s, key, p))
        .child(copy)
        .child(switch)
}
fn steps_card(s: &State, k: &Kit, p: Palette, statuses: &BTreeMap<String, FieldStatus>) -> El {
    let mut rows = col().w_full();
    for step in Step::ALL.iter() {
        rows = rows.child(step_row(s, k, p, *step, statuses));
    }
    if let Some(error) = s.hex.shell.engine.error {
        let alert = roles::SMALL.wrap(error, p.warn).role("alert");
        rows = rows.child(col().w_full().padding(14.0).child(alert));
    }
    col()
        .w_full()
        .gap(8.0)
        .mt(18.0)
        .key("engine-steps")
        .child(roles::GROUP.wrap("Optional steps", p.soft))
        .child(panel(p).child(rows))
}
/// The one-time import of script settings is not wired to anything yet.
fn import_row(s: &State, p: Palette) -> El {
    let key = "engine-import";
    let name = "Import from scripts";
    let help =
        "Bring in the settings your old scripts use, one time. This comes in a later version.";
    let copy = settings_copy(name, p).child(roles::META.wrap(help, p.soft));
    let button = row_action("Import", "engine-import-button", p).button_disabled(true);
    let line = settings_row(true)
        .key(key)
        .fill(group_highlight(s, key, p))
        .child(copy)
        .child(button);
    col()
        .w_full()
        .gap(8.0)
        .mt(18.0)
        .child(roles::GROUP.wrap("Script settings", p.soft))
        .child(panel(p).child(line))
}
fn prompt_row(
    s: &State,
    k: &Kit,
    p: Palette,
    field: Field,
    statuses: &BTreeMap<String, FieldStatus>,
) -> El {
    let (name, help, _) = field.text();
    let key = field.key();
    let engine = &s.hex.shell.engine;
    let mut copy = copy_column(p, key, (name, help), field.status_key(), statuses);
    if engine.bad.contains(&field) {
        let hint = roles::META.medium().wrap(field.hint(), p.warn);
        copy = copy.child(hint.role("alert"));
    }
    let index = field.index();
    let fallback = TextEdit::default();
    let area = TextArea::new(engine.drafts.get(index).unwrap_or(&fallback))
        .label(name)
        .auto_grow(4, 10)
        .on_edit(move |s: &mut State| draft_mut(s, index))
        .build(k)
        .key(format!("{key}-input"))
        .w_full()
        .min_w(0.0);
    let config = engine.base.as_ref().unwrap_or(&s.config);
    let confirming = engine.confirm_reset == Some(field);
    let reset_label = if confirming {
        "Press again to reset"
    } else {
        "Reset to default"
    };
    let buttons = row()
        .gap(8.0)
        .flex_wrap()
        .child(
            action("Save prompt", p)
                .key(format!("{key}-save"))
                .button_disabled(!engine.dirty(&[field]))
                .on_press(move |s: &mut State| save(s, &[field])),
        )
        .child(
            action(reset_label, p)
                .key(format!("{key}-reset"))
                .button_disabled(!customised(config, field))
                .on_press(move |s: &mut State| reset(s, field)),
        );
    let top = settings_row(true)
        .key(key)
        .fill(group_highlight(s, key, p))
        .child(copy);
    let edit = col().w_full().gap(10.0).px(14.0).pb(14.0);
    col()
        .w_full()
        .child(top)
        .child(edit.child(area).child(buttons))
}
fn prompts_card(s: &State, k: &Kit, p: Palette, statuses: &BTreeMap<String, FieldStatus>) -> El {
    let mut rows = col().w_full();
    for field in Field::PROMPTS
        .iter()
        .filter(|field| field.visible(&s.config))
    {
        rows = rows.child(prompt_row(s, k, p, *field, statuses));
    }
    let engine = &s.hex.shell.engine;
    if let Some(error) = engine.error.filter(|_| {
        engine.bad.iter().any(|f| Field::PROMPTS.contains(f)) || error_is_save_failure(engine)
    }) {
        let alert = roles::SMALL.wrap(error, p.warn).role("alert");
        rows = rows.child(col().w_full().padding(14.0).child(alert));
    }
    col()
        .w_full()
        .gap(8.0)
        .mt(18.0)
        .key("engine-prompts")
        .child(roles::GROUP.wrap("Prompts", p.soft))
        .child(panel(p).child(rows))
}
fn error_is_save_failure(engine: &Engine) -> bool {
    engine.error == Some(SAVE_FAILED)
}
fn field_row(
    s: &State,
    k: &Kit,
    p: Palette,
    field: Field,
    statuses: &BTreeMap<String, FieldStatus>,
) -> El {
    let (name, help, placeholder) = field.text();
    let key = field.key();
    let engine = &s.hex.shell.engine;
    let mut copy = copy_column(p, key, (name, help), field.status_key(), statuses);
    if engine.bad.contains(&field) {
        let hint = roles::META.medium().wrap(field.hint(), p.warn);
        copy = copy.child(hint.role("alert"));
    }
    let index = field.index();
    let fallback = TextEdit::default();
    let config = engine.base.as_ref().unwrap_or(&s.config);
    let input = if routed(config, field) {
        let value = roles::CONTROL.wrap(read(config, field), p.ink);
        value.key(format!("{key}-value")).w(220.0).px(9.0)
    } else {
        TextInput::new(engine.drafts.get(index).unwrap_or(&fallback))
            .height(30.0)
            .label(name)
            .placeholder(placeholder)
            .on_edit(move |s: &mut State| draft_mut(s, index))
            .build(&row_field_kit(k, p))
            .key(format!("{key}-input"))
            .w(220.0)
    };
    settings_row(true)
        .key(key)
        .fill(group_highlight(s, key, p))
        .child(copy)
        .child(input)
}
fn group(
    s: &State,
    k: &Kit,
    p: Palette,
    title: &str,
    key: &str,
    fields: &'static [Field],
    statuses: &BTreeMap<String, FieldStatus>,
) -> El {
    let mut rows = col().w_full();
    for field in fields.iter().filter(|field| field.visible(&s.config)) {
        rows = rows.child(field_row(s, k, p, *field, statuses));
    }
    let engine = &s.hex.shell.engine;
    let mut footer = col().w_full().gap(8.0).padding(14.0);
    if let Some(error) = engine.error {
        footer = footer.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    footer = footer.child(
        action(format!("Save {}", title.to_lowercase()), p)
            .key(format!("{key}-save"))
            .self_start()
            .button_disabled(!engine.dirty(fields))
            .on_press(move |s: &mut State| save(s, fields)),
    );
    col()
        .w_full()
        .gap(8.0)
        .mt(18.0)
        .key(key)
        .child(roles::GROUP.wrap(title, p.soft))
        .child(panel(p).child(rows.child(footer)))
}

/// The notes writer is the existing dev.28 choice. This row shows it and links to where it is
/// changed, so there is one setting and one save path.
fn notes_writer(s: &State, p: Palette, statuses: &BTreeMap<String, FieldStatus>) -> El {
    let settings = s.ai_notes.settings.as_ref();
    let writer = settings.and_then(|notes| notes.effective_helper());
    let help =
        "The agent that writes meeting summaries. It is the same choice as in Models and agents.";
    let mut copy = settings_copy("Notes writer", p).child(roles::META.wrap(help, p.soft));
    if let Some(status) = statuses.get("notes.helper") {
        let (line, warn) = status_text("notes.helper", status);
        let color = if warn { p.warn } else { p.soft };
        let line = roles::META.wrap(line, color);
        copy = copy.child(line.key("engine-notes-writer-status"));
    }
    let name = writer.map_or("None chosen", |helper| helper.name());
    let control = row()
        .gap(10.0)
        .items_center()
        .flex_wrap()
        .child(
            roles::CONTROL
                .wrap(name, p.ink)
                .key("engine-notes-writer-value"),
        )
        .child(
            row_action("Change writer", "engine-notes-writer-change", p)
                .on_press(|s: &mut State| navigate(s, Page::Helpers, 0)),
        );
    let key = "engine-notes-writer";
    let line = settings_row(true)
        .key(key)
        .fill(group_highlight(s, key, p))
        .child(copy)
        .child(control);
    panel(p).child(line)
}
fn notes(
    s: &State,
    k: &Kit,
    p: Palette,
    layer: &mut Vec<El>,
    st: &BTreeMap<String, FieldStatus>,
) -> El {
    let extras = notes_extras(s, p, st);
    col()
        .w_full()
        .gap(8.0)
        .mt(18.0)
        .key("engine-notes")
        .child(roles::GROUP.wrap("Notes", p.soft))
        .child(notes_writer(s, p, st))
        .child(ai_notes::summary_model(s, k, p, layer, extras))
}
/// What the Summary model card says besides its picker: a save in progress and a failed save.
/// The picker is the existing one, so its save reports through the existing notes state.
fn notes_notices(a: &ai_notes::AiNotes) -> Vec<(&'static str, &'static str, String)> {
    let mut out = Vec::new();
    if let Some(pending) = a.pending {
        out.push(("engine-notes-pending", "status", pending.to_owned()));
    }
    if let Some(error) = &a.error
        && a.editor_target.is_none()
    {
        out.push(("engine-notes-error", "alert", error.clone()));
    }
    out
}
fn notes_extras(s: &State, p: Palette, st: &BTreeMap<String, FieldStatus>) -> Vec<El> {
    let mut lines = col().w_full().gap(4.0).px(14.0).pb(14.0);
    let mut any = false;
    if let Some(status) = st.get("notes.model") {
        let (line, warn) = status_text("notes.model", status);
        let line = roles::META.wrap(line, if warn { p.warn } else { p.soft });
        lines = lines.child(line.key("engine-notes-model-status"));
        any = true;
    }
    for (key, role, text) in notes_notices(&s.ai_notes) {
        let color = if role == "alert" { p.warn } else { p.soft };
        lines = lines.child(roles::SMALL.wrap(text, color).role(role).key(key));
        any = true;
    }
    any.then_some(lines).into_iter().collect()
}

pub(super) fn body(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let statuses = s.config.engine_field_status();
    let open = s.hex.shell.engine.unused_open;
    let mut deferred = col().key("engine-unused-section").w_full().mt(18.0).child(
        crate::ui::cards::fold_control("Advanced engine settings", "engine-unused", open, p)
            .on_press(|s: &mut State| {
                s.hex.shell.engine.unused_open = !s.hex.shell.engine.unused_open;
            }),
    );
    if open {
        deferred = deferred
            .child(roles::SMALL.wrap(NOT_USED, p.soft).mt(6.0))
            .child(group(
                s,
                k,
                p,
                "Transcription",
                "engine-transcription",
                &Field::TRANSCRIPTION,
                &statuses,
            ))
            .child(group(
                s,
                k,
                p,
                "Step models",
                "engine-step-models",
                &Field::STEP_MODELS,
                &statuses,
            ))
            .child(prompts_card(s, k, p, &statuses))
            .child(steps_card(s, k, p, &statuses))
            .child(import_row(s, p));
    }
    col()
        .w_full()
        .key("meeting-engine")
        .child(
            roles::SMALL
                .wrap(
                    "Choose the agent and model that write your meeting notes.",
                    p.soft,
                )
                .mt(12.0),
        )
        .child(notes(s, k, p, layer, &statuses))
        .child(deferred)
}

/// Render fixtures, used only by the isolated scenes: a saved Telugu model and a Hindi model
/// that comes from the language settings. Everything else stays at its default.
pub(super) fn scene_config(config: &mut Config) {
    use sidenote::core::meeting_router::{LanguageRoutes, ProviderRoute};
    config.features.indic_engine = true;
    config.features.translation = true;
    let models = &mut config.engine_settings.models;
    models.insert("te".into(), "synthetic-te-model".into());
    config.engine_settings.stages.title_model = "synthetic-title-model".into();
    config.engine_settings.steps.tasks = false;
    // A Telugu and a Hindi word, so the render shows both scripts drawing as letters.
    let prompt = "Telugu \u{c24}\u{c46}\u{c32}\u{c41}\u{c17}\u{c41}, Hindi \u{939}\u{93f}\u{928}\u{94d}\u{926}\u{940}. Synthetic notes instructions for the render.";
    config.engine_settings.notes_prompt = Some(prompt.into());
    let route = ProviderRoute {
        provider: "local".into(),
        model: "synthetic-hi-route-model".into(),
    };
    config.pipeline.after_stop_languages = Some(LanguageRoutes {
        primary: "hi".into(),
        supporting: vec!["en".into()],
        routes: [("hi".into(), route)].into(),
    });
}
/// A Summary model save that failed, as the existing notes state reports it. Render scene only.
pub(super) fn scene_notes_error(s: &mut State) {
    let message = "Could not save Meeting summary settings. Your previous choice was kept.";
    s.ai_notes.error = Some(message.into());
}
/// Type values that cannot be saved and press Save, as the person would. Render scenes only.
pub(super) fn scene_invalid(s: &mut State) {
    let edits = [
        (Field::English, "bad model name"),
        (Field::Threads, "0"),
        (Field::Deadline, "9999"),
    ];
    for (field, text) in edits {
        *draft_mut(s, field.index()) = TextEdit::new(text);
    }
    save(s, &Field::TRANSCRIPTION);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_write_round_trip_and_blank_deadline_means_automatic() {
        let mut config = Config::default();
        assert_eq!(read(&config, Field::Threads), "4");
        assert_eq!(read(&config, Field::Deadline), "");
        assert_eq!(read(&config, Field::RequestDeadline), "180");
        assert!(write(&mut config, Field::Deadline, "900").is_ok());
        assert_eq!(config.engine_settings.stage_deadline_seconds, Some(900));
        assert!(write(&mut config, Field::Deadline, "").is_ok());
        assert_eq!(config.engine_settings.stage_deadline_seconds, None);
        assert!(write(&mut config, Field::Threads, "four").is_err());
    }
    #[test]
    fn only_changed_fields_are_applied_and_only_they_are_checked() {
        let mut base = Config::default();
        // A stored value that is already wrong must not block an unrelated edit.
        base.engine_settings.stages.asr_request_deadline_seconds = 0;
        assert!(apply(&base, &[(Field::Threads, "4")]).unwrap().is_none());
        let edited = apply(&base, &[(Field::Threads, " 8 "), (Field::Deadline, "60")])
            .unwrap()
            .unwrap();
        assert_eq!(edited.engine_settings.worker_threads, 8);
        assert_eq!(edited.engine_settings.stage_deadline_seconds, Some(60));
        assert_eq!(read(&edited, Field::RequestDeadline), "0");
        let edits = [
            (Field::Threads, "0"),
            (Field::Deadline, "3601"),
            (Field::RequestDeadline, "90"),
        ];
        let bad = apply(&base, &edits).err().unwrap();
        assert_eq!(bad, [Field::Threads, Field::Deadline]);
    }
    #[test]
    fn models_edit_per_language_and_a_wrong_sibling_does_not_block_them() {
        let mut base = Config::default();
        base.engine_settings
            .models
            .insert("hi".into(), "not a model".into());
        let edits = [(Field::English, " synthetic-en "), (Field::Telugu, "")];
        let edited = apply(&base, &edits).unwrap().unwrap();
        assert_eq!(read(&edited, Field::English), "synthetic-en");
        assert_eq!(read(&edited, Field::Hindi), "not a model");
        let bad = apply(&base, &[(Field::Telugu, "bad model")]).err().unwrap();
        assert_eq!(bad, [Field::Telugu]);
        let mut config = edited;
        assert!(write(&mut config, Field::English, "").is_ok());
        assert!(!config.engine_settings.models.contains_key("en"));
    }
    #[test]
    fn a_language_route_stays_the_only_home_of_its_model() {
        let mut config = Config::default();
        scene_config(&mut config);
        assert!(routed(&config, Field::Hindi) && !routed(&config, Field::Telugu));
        assert_eq!(read(&config, Field::Hindi), "synthetic-hi-route-model");
        let edits = [(Field::Hindi, "other-model")];
        assert!(apply(&config, &edits).unwrap().is_none());
        let statuses = config.engine_field_status();
        assert_eq!(statuses["models.hi"].source, Source::ExistingRoute);
        let te = statuses["models.te"].validation;
        assert_eq!(te, Validation::ModelQualificationRequired);
    }
    #[test]
    fn step_models_read_defaults_edit_alone_and_reject_empty_or_odd_names() {
        let base = Config::default();
        assert_eq!(read(&base, Field::Title), "claude-haiku-5-5");
        assert_eq!(read(&base, Field::Comparison), "sonnet");
        let edits = [
            (Field::Tasks, " synthetic-tasks-model "),
            (Field::Title, "claude-haiku-5-5"),
        ];
        let edited = apply(&base, &edits).unwrap().unwrap();
        assert_eq!(
            edited.engine_settings.stages.tasks_model,
            "synthetic-tasks-model"
        );
        assert_eq!(
            read(&edited, Field::Translation),
            read(&base, Field::Translation)
        );
        let edits = [
            (Field::Translation, ""),
            (Field::Comparison, "bad model"),
            (Field::Title, "ok-model"),
        ];
        let bad = apply(&base, &edits).err().unwrap();
        assert_eq!(bad, [Field::Translation, Field::Comparison]);
    }
    #[test]
    fn every_step_model_says_its_name_is_unchecked_and_not_used() {
        let status = Config::default().engine_field_status();
        for field in Field::STEP_MODELS {
            assert!(!in_use(field.status_key()), "{field:?}");
            let validation = status[field.status_key()].validation;
            assert_eq!(validation, Validation::ModelQualificationRequired);
        }
        assert!(in_use("notes.helper") && in_use("notes.model"));
        assert_eq!(Field::ALL.len(), Field::COUNT);
        assert!(Field::ALL.iter().enumerate().all(|(i, f)| f.index() == i));
    }
    #[test]
    fn steps_start_as_the_model_says_and_comparison_is_off_by_default() {
        let mut config = Config::default();
        let on: Vec<_> = Step::ALL.iter().map(|step| step.get(&config)).collect();
        assert_eq!(on, [true, true, false]);
        Step::Comparison.set(&mut config, true);
        Step::Tasks.set(&mut config, false);
        assert!(config.engine_settings.steps.external_comparison);
        assert!(!config.engine_settings.steps.tasks && config.engine_settings.steps.quick_title);
        let status = config.engine_field_status();
        for step in Step::ALL {
            assert!(status.contains_key(step.status_key()), "{step:?}");
            assert!(!in_use(step.status_key()));
        }
        assert_eq!(status["external_comparison"].source, Source::Saved);
        assert_eq!(status["quick_title"].source, Source::Default);
    }
    #[test]
    fn a_step_toggle_goes_through_the_reviewed_save_and_keeps_other_edits() {
        crate::test_paths::isolate();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        std::fs::write(&path, r#"{"version":1,"features":{},"notes":{}}"#).unwrap();
        let mut config = Config::load_in(root.path()).unwrap();
        Step::Comparison.set(&mut config, true);
        config.save(root.path()).unwrap();
        let mut stale = Config::load_in(root.path()).unwrap();
        let mut other = Config::load_in(root.path()).unwrap();
        other.engine_settings.worker_threads = 8;
        other.save(root.path()).unwrap();
        Step::Tasks.set(&mut stale, false);
        stale.save(root.path()).unwrap();
        let loaded = Config::load_in(root.path()).unwrap();
        assert!(Step::Comparison.get(&loaded) && !Step::Tasks.get(&loaded));
        assert_eq!(loaded.engine_settings.worker_threads, 8);
    }
    #[test]
    fn prompts_show_the_shipped_text_and_text_equal_to_it_is_not_stored() {
        let base = Config::default();
        let shipped = read(&base, Field::NotesPrompt);
        assert!(!shipped.is_empty() && !customised(&base, Field::NotesPrompt));
        assert!(
            apply(&base, &[(Field::NotesPrompt, &shipped)])
                .unwrap()
                .is_none()
        );
        let custom = apply(&base, &[(Field::NotesPrompt, "Synthetic instructions")]);
        let custom = custom.unwrap().unwrap();
        assert!(customised(&custom, Field::NotesPrompt));
        assert!(!customised(&custom, Field::TranslationPrompt));
        let back = apply(&custom, &[(Field::NotesPrompt, &shipped)])
            .unwrap()
            .unwrap();
        assert!(back.engine_settings.notes_prompt.is_none());
        let bad = apply(&base, &[(Field::TranslationPrompt, " ")])
            .err()
            .unwrap();
        assert_eq!(bad, [Field::TranslationPrompt]);
    }
    #[test]
    fn reset_to_default_goes_through_the_reviewed_save_and_leaves_the_other_prompt() {
        crate::test_paths::isolate();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        std::fs::write(&path, r#"{"version":1,"features":{},"notes":{}}"#).unwrap();
        let mut config = Config::load_in(root.path()).unwrap();
        config.engine_settings.notes_prompt = Some("Synthetic notes".into());
        config.engine_settings.translation_prompt = Some("Synthetic translation".into());
        config.save(root.path()).unwrap();
        let mut config = Config::load_in(root.path()).unwrap();
        reset_prompt(&mut config, Field::NotesPrompt);
        config.save(root.path()).unwrap();
        let loaded = Config::load_in(root.path()).unwrap();
        assert!(!customised(&loaded, Field::NotesPrompt));
        assert!(customised(&loaded, Field::TranslationPrompt));
        let status = loaded.engine_field_status();
        assert_eq!(status["notes_prompt"].source, Source::Default);
        assert_eq!(status["translation_prompt"].source, Source::Saved);
    }
    #[test]
    fn a_save_refreshes_only_its_own_drafts() {
        let base = Config::default();
        let drafts = Field::ALL.map(|f| TextEdit::new(read(&base, f)));
        let mut engine = Engine {
            drafts: drafts.to_vec(),
            base: Some(base.clone()),
            ..Default::default()
        };
        engine.drafts[Field::Threads.index()] = TextEdit::new("7");
        engine.drafts[Field::Title.index()] = TextEdit::new("typed title model");
        let mut saved = base;
        saved.engine_settings.stages.title_model = "stored-title-model".into();
        adopt(&mut engine, saved, &[Field::Title]);
        assert_eq!(
            engine.drafts[Field::Title.index()].text,
            "stored-title-model"
        );
        assert_eq!(engine.drafts[Field::Threads.index()].text, "7");
        assert!(engine.dirty(&[Field::Threads]) && !engine.dirty(&[Field::Title]));
    }
    const EMPTY: &str = r#"{"version":1,"features":{},"notes":{}}"#;
    /// A State whose settings folder is a fresh temp dir, loaded as the tab would be.
    fn state_with(raw: &str) -> (State, tempfile::TempDir) {
        crate::test_paths::isolate();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("config.json"), raw).unwrap();
        let mut s = State::new();
        s.store.root = root.path().to_path_buf();
        s.config = Config::load_in(root.path()).unwrap();
        load(&mut s);
        (s, root)
    }
    fn type_into(s: &mut State, field: Field, text: &str) {
        *draft_mut(s, field.index()) = TextEdit::new(text);
    }
    fn text(s: &State, field: Field) -> String {
        s.hex.shell.engine.drafts[field.index()].text.clone()
    }
    #[test]
    fn visiting_the_tab_again_or_pressing_it_keeps_typed_text_and_refreshes_the_rest() {
        let (mut s, _root) = state_with(EMPTY);
        type_into(&mut s, Field::Threads, "8");
        type_into(&mut s, Field::NotesPrompt, "Synthetic instructions");
        s.config.engine_settings.stages.title_model = "stored-elsewhere".into();
        for _ in 0..2 {
            // The second pass is the active tab pressed again.
            super::super::navigate(&mut s, Page::Meetings, 3);
            assert_eq!(text(&s, Field::Threads), "8");
            assert_eq!(text(&s, Field::NotesPrompt), "Synthetic instructions");
            assert_eq!(text(&s, Field::Title), "stored-elsewhere");
            assert!(
                s.hex
                    .shell
                    .engine
                    .dirty(&[Field::Threads, Field::NotesPrompt])
            );
            assert!(!s.hex.shell.engine.dirty(&[Field::Title]));
        }
    }
    #[test]
    fn leaving_a_rejected_edit_keeps_it_and_its_hint() {
        let (mut s, _root) = state_with(EMPTY);
        type_into(&mut s, Field::Threads, "0");
        save(&mut s, &Field::TRANSCRIPTION);
        assert_eq!(s.hex.shell.engine.bad, [Field::Threads]);
        super::super::navigate(&mut s, Page::Meetings, 3);
        assert_eq!(text(&s, Field::Threads), "0");
        assert_eq!(s.hex.shell.engine.bad, [Field::Threads]);
        assert_eq!(s.hex.shell.engine.error, Some(NOT_SAVED));
        type_into(&mut s, Field::Threads, "4");
        super::super::navigate(&mut s, Page::Meetings, 3);
        assert!(s.hex.shell.engine.bad.is_empty() && s.hex.shell.engine.error.is_none());
    }
    #[test]
    fn pressing_the_tab_again_keeps_a_failed_disk_save_message() {
        let (mut s, _root) = state_with(EMPTY);
        type_into(&mut s, Field::Threads, "8");
        s.hex.shell.engine.error = Some(SAVE_FAILED);
        assert!(s.hex.shell.engine.bad.is_empty());
        super::super::navigate(&mut s, Page::Meetings, 3);
        assert_eq!(s.hex.shell.engine.error, Some(SAVE_FAILED));
        assert_eq!(text(&s, Field::Threads), "8");
    }
    #[test]
    fn a_wrong_sibling_model_does_not_stop_the_edit_from_saving() {
        let raw = r#"{"version":1,"features":{},"notes":{},"engine_settings":{"models":{"hi":"not a model"}}}"#;
        let (mut s, root) = state_with(raw);
        type_into(&mut s, Field::English, "synthetic-en");
        save(&mut s, &Field::TRANSCRIPTION);
        assert!(s.hex.shell.engine.bad.is_empty() && s.hex.shell.engine.error.is_none());
        let loaded = Config::load_in(root.path()).unwrap();
        assert_eq!(read(&loaded, Field::English), "synthetic-en");
        assert_eq!(read(&loaded, Field::Hindi), "not a model");
        assert!(!s.hex.shell.engine.dirty(&[Field::English]));
    }
    #[test]
    fn a_save_never_turns_an_untouched_stale_field_into_an_overwrite() {
        // Each kind of save in this tab is followed by a second, unrelated save.
        for kind in ["toggle", "step model", "prompt reset"] {
            let (mut s, root) =
                state_with(r#"{"version":1,"features":{"translation":true},"notes":{}}"#);
            let mut other = Config::load_in(root.path()).unwrap();
            other.engine_settings.worker_threads = 8;
            other.engine_settings.notes_prompt = Some("Synthetic other window".into());
            other.save(root.path()).unwrap();
            match kind {
                "toggle" => toggle(&mut s, Step::Tasks),
                "step model" => {
                    type_into(&mut s, Field::Title, "synthetic-title");
                    save(&mut s, &Field::STEP_MODELS);
                }
                _ => {
                    reset(&mut s, Field::TranslationPrompt);
                    reset(&mut s, Field::TranslationPrompt);
                }
            }
            assert!(s.hex.shell.engine.error.is_none(), "{kind}");
            assert_eq!(text(&s, Field::Threads), "8", "{kind}");
            type_into(&mut s, Field::Deadline, "60");
            save(&mut s, &Field::TRANSCRIPTION);
            let loaded = Config::load_in(root.path()).unwrap();
            assert_eq!(loaded.engine_settings.stage_deadline_seconds, Some(60));
            assert_eq!(loaded.engine_settings.worker_threads, 8, "{kind}");
            let prompt = loaded.engine_settings.notes_prompt.as_deref();
            assert_eq!(prompt, Some("Synthetic other window"), "{kind}");
        }
    }
    #[test]
    fn status_words_cover_every_state_without_a_value() {
        let sources = [
            Source::Default,
            Source::Saved,
            Source::LegacyEnvironment,
            Source::ExistingRoute,
        ];
        let validations = [
            Validation::Empty,
            Validation::Valid,
            Validation::Invalid,
            Validation::Unavailable,
            Validation::ModelQualificationRequired,
        ];
        for source in sources {
            for validation in validations {
                let status = FieldStatus {
                    source,
                    validation,
                    conflict: false,
                    invalid_import: false,
                };
                let (line, warn) = status_text("worker_threads", &status);
                assert!(!line.contains("Default value") && !line.contains("valid"));
                let attention = matches!(validation, Validation::Invalid | Validation::Unavailable);
                assert_eq!(warn, attention);
            }
        }
        assert!(in_use("notes.model") && !in_use("worker_threads"));
    }
    #[test]
    fn the_notes_model_is_never_called_valid_because_nothing_ran_it() {
        let status = FieldStatus {
            source: Source::Saved,
            validation: Validation::Valid,
            conflict: false,
            invalid_import: false,
        };
        let (model, _) = status_text("notes.model", &status);
        assert_eq!(
            model,
            "Saved on this PC \u{b7} Model has not been tested on this PC"
        );
        let (threads, _) = status_text("worker_threads", &status);
        assert_eq!(threads, "Saved on this PC");
    }
    #[test]
    fn a_pending_or_failed_summary_save_is_shown_on_this_tab() {
        let mut notes = ai_notes::AiNotes::default();
        assert!(notes_notices(&notes).is_empty());
        notes.pending = Some("Saving Meeting summary...");
        notes.error = Some("Could not save Meeting summary settings.".into());
        let shown = notes_notices(&notes);
        let roles: Vec<_> = shown.iter().map(|(key, role, _)| (*key, *role)).collect();
        assert_eq!(
            roles,
            [
                ("engine-notes-pending", "status"),
                ("engine-notes-error", "alert")
            ]
        );
        assert!(shown[1].2.contains("Could not save"));
        // An error that belongs to a prompt editor elsewhere is not this card's.
        notes.editor_target = Some(0);
        assert_eq!(notes_notices(&notes).len(), 1);
    }
    #[test]
    fn a_failed_save_keeps_edited_drafts_and_refreshes_the_rest() {
        let (mut old, mut fresh) = (Config::default(), Config::default());
        old.engine_settings.worker_threads = 5;
        fresh.engine_settings.worker_threads = 6;
        fresh.engine_settings.stages.asr_request_deadline_seconds = 200;
        let drafts = Field::ALL.map(|f| TextEdit::new(read(&old, f)));
        let mut engine = Engine {
            drafts: drafts.to_vec(),
            base: Some(old),
            ..Default::default()
        };
        engine.drafts[Field::Deadline.index()] = TextEdit::new("120");
        engine.drafts[Field::Threads.index()] = TextEdit::new("7");
        rebase(&mut engine, fresh);
        assert_eq!(engine.drafts[Field::Deadline.index()].text, "120");
        assert_eq!(engine.drafts[Field::Threads.index()].text, "7");
        assert_eq!(engine.drafts[Field::RequestDeadline.index()].text, "200");
    }
    #[test]
    fn the_reviewed_save_path_keeps_the_edit_and_rejects_a_bad_one_untouched() {
        crate::test_paths::isolate();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        std::fs::write(&path, r#"{"version":1,"features":{},"notes":{}}"#).unwrap();
        let base = Config::load_in(root.path()).unwrap();
        let mut edited = apply(&base, &[(Field::Threads, "8")]).unwrap().unwrap();
        edited.save(root.path()).unwrap();
        let loaded = Config::load_in(root.path()).unwrap();
        assert_eq!(read(&loaded, Field::Threads), "8");
        let status = loaded.engine_field_status();
        assert_eq!(status["worker_threads"].source, Source::Saved);
        let prior = serde_json::to_vec(&loaded).unwrap();
        assert!(apply(&loaded, &[(Field::Threads, "0")]).is_err());
        let after = Config::load_in(root.path()).unwrap();
        assert_eq!(serde_json::to_vec(&after).unwrap(), prior);
    }
}
