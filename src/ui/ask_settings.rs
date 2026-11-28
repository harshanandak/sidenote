use super::{
    State,
    hex::settings::{card, row_field_kit, setting_row, short_select_width},
    roles,
    theme::Palette,
};
use rune::{El, TextEdit, col, row};
use rune_kit::{Kit, Menu, MenuItem, Placement, Select, SelectAppearance, TextInput};
use sidenote::core::{
    self, UiPrefs,
    ask_options::{self, Thinking},
};
use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap},
};

pub const OPENCODE_THINKING: &str = "OpenCode has no separate effort flag. Choose a thinking variant with provider/model#variant in Models and agents.";

pub struct AskSettings {
    pub connection: super::cloud_ask::Connection,
    pub meeting: Thinking,
    pub meeting_agent: String,
    pub meeting_model: Option<String>,
    pub meeting_override: Option<(String, Option<String>)>,
    /// The helper and model picked for the next question outside a live meeting. Asking uses
    /// it once, then the choice returns to Default.
    pub pick: Option<(String, Option<String>)>,
    pub meeting_level: Option<Thinking>,
    pub meeting_epoch: u64,
    pub fallback_notice: Option<String>,
    pub live_all: bool,
    pub scope_open: bool,
    pub reduced_motion: Cell<bool>,
    pub details_open: Option<String>,
    pub retry_questions: HashMap<String, core::Message>,
    pub saved_all: bool,
    pub loaded_scope: String,
    pub live_chat: String,
    pub chat: Thinking,
    pub models: BTreeMap<String, String>,
    pub model_input: TextEdit,
    pub thinking_open: bool,
    pub select_open: Option<String>,
    pub last_end_ms: Option<i64>,
    pub last_live: Option<ask_options::LiveSource>,
    pub recording: bool,
    pub recording_source: Option<ask_options::LiveSource>,
    pub warm_signature: String,
    pub last_input: String,
    pub catalog_version: u64,
    pub other_model: bool,
    pub overrides: HashMap<(String, bool), Thinking>,
}
impl AskSettings {
    pub fn new(prefs: &UiPrefs) -> Self {
        let recent = ask_options::recent_meeting(
            &core::meetings_dir(),
            chrono::Local::now().timestamp_millis(),
        );
        Self {
            connection: super::cloud_ask::Connection::new(),
            meeting: if prefs.ask_meeting_thinking == Thinking::High {
                Thinking::Medium
            } else {
                prefs.ask_meeting_thinking
            },
            meeting_agent: prefs.ask_meeting_agent.clone(),
            meeting_model: prefs.ask_meeting_model.clone(),
            meeting_override: None,
            pick: None,
            meeting_level: None,
            meeting_epoch: 0,
            fallback_notice: None,
            live_all: false,
            scope_open: false,
            reduced_motion: Cell::default(),
            details_open: None,
            retry_questions: HashMap::new(),
            saved_all: false,
            loaded_scope: String::new(),
            live_chat: core::live_ask_key(&chrono::Local::now().timestamp_millis().to_string()),
            chat: prefs.ask_chat_thinking,
            models: prefs.ask_models.clone(),
            model_input: TextEdit::default(),
            thinking_open: false,
            select_open: None,
            last_end_ms: recent.as_ref().map(|(end, _)| *end),
            last_live: recent.map(|(_, source)| source),
            recording: false,
            recording_source: None,
            warm_signature: String::new(),
            last_input: String::new(),
            catalog_version: 0,
            other_model: false,
            overrides: HashMap::new(),
        }
    }
}
impl State {
    fn tracked_ask_id(&self) -> Option<&str> {
        self.live
            .as_ref()
            .filter(|live| {
                self.on_live()
                    && (live.phase == super::live::Phase::Paused
                        || (live.phase == super::live::Phase::Recording
                            && !self.ask_live_meeting()))
            })?
            .session
            .as_ref()
            .map(|session| session.id.as_str())
    }
    pub fn live_scope_available(&self) -> bool {
        self.on_live() && (self.ask_live_meeting() || self.tracked_ask_id().is_some())
    }
    pub fn ask_current_live(&self) -> bool {
        self.on_live()
            && self.ask_live_meeting()
            && self.tracked_ask_id().is_none()
            && !self.ask_settings.live_all
    }
    pub fn ask_meeting_id(&self) -> Option<&str> {
        if self.selected.is_some() {
            self.selected
                .as_deref()
                .filter(|_| !self.ask_settings.saved_all)
        } else {
            self.tracked_ask_id()
                .filter(|_| !self.ask_settings.live_all)
        }
    }
    pub fn set_ask_scope(&mut self, all: bool) {
        if self.selected.is_some() {
            self.ask_settings.saved_all = all;
        } else if self.live_scope_available() {
            self.ask_settings.live_all = all;
        }
        self.ask_settings.scope_open = false;
        self.ask_settings.warm_signature.clear();
        self.load_saved();
    }
    pub fn ask_live_meeting(&self) -> bool {
        self.hex_recording() || self.ask_settings.recording
    }
    pub fn ask_runner_name(&self) -> Option<&str> {
        if self.ask_settings.connection.settings.enabled {
            return Some(core::provider::NAME);
        }
        if self.ask_live_meeting() {
            Some(
                self.ask_settings
                    .meeting_override
                    .as_ref()
                    .map_or(self.ask_settings.meeting_agent.as_str(), |(name, _)| {
                        name.as_str()
                    }),
            )
        } else {
            self.ask_settings
                .pick
                .as_ref()
                .map(|(name, _)| name.as_str())
                .or(self.runner.as_deref())
        }
    }
    pub fn normal_ask_model(&self) -> Option<String> {
        let name = self.runner.as_ref()?;
        self.ask_settings
            .models
            .get(name)
            .cloned()
            .or_else(|| self.runners.iter().find(|r| &r.name == name)?.model.clone())
    }
    pub fn choose_meeting_model(&mut self, name: String, model: Option<String>) {
        self.ask_settings.fallback_notice = None;
        self.ask_settings.meeting_epoch += 1;
        self.ask_settings.meeting_override = Some((name, model));
        self.runner_open = false;
        self.ask_settings.warm_signature.clear();
        core::ask_session::release();
    }
    pub fn meeting_agent_transition(&mut self) {
        self.ask_settings.live_all = false;
        self.ask_settings.scope_open = false;
        let seed = super::hex::recording_start(self)
            .map(|(at, _)| at.timestamp_millis())
            .unwrap_or_else(|| chrono::Local::now().timestamp_millis());
        self.ask_settings.live_chat = core::live_ask_key(&seed.to_string());
        self.ask_settings.fallback_notice = None;
        self.ask_settings.meeting_epoch += 1;
        self.ask_settings.meeting_override = None;
        self.ask_settings.meeting_level = None;
        self.ask_settings.warm_signature.clear();
        self.runner_open = false;
        self.ask_settings.thinking_open = false;
    }

    pub fn warm_ask(&mut self) {
        if self.ask_settings.connection.settings.enabled {
            return;
        }
        if self.waker.is_none() {
            return;
        }
        let Some(runner) = self
            .runners
            .iter()
            .find(|r| Some(r.name.as_str()) == self.ask_runner_name())
            .cloned()
        else {
            return;
        };
        if self.ask_live_meeting()
            && (ask_options::agent_name(&runner) != "Codex"
                || !self.runner_found.contains(&runner.name))
        {
            return;
        }
        let model = self.ask_model();
        let thinking = self.ask_thinking();
        let chat = self.ask_key();
        let signature = format!("{}:{model:?}:{thinking:?}:{chat}", runner.name);
        if self.ask_settings.warm_signature == signature {
            return;
        }
        self.ask_settings.warm_signature = signature;
        let options = core::ask_options::Options {
            model,
            thinking,
            ..Default::default()
        };
        if self.ask_live_meeting() {
            let _ = core::ask::prewarm_meeting(runner, &options, chat);
        } else {
            core::ask::prewarm(runner, &options, chat);
        }
    }

    pub fn discover_ask_models(&mut self) {
        self.ask_settings.warm_signature.clear();
        self.warm_ask();
        if !self.ask_settings.connection.settings.enabled
            && self.waker.is_some()
            && let Some(runner) = self
                .runners
                .iter()
                .find(|r| Some(&r.name) == self.runner.as_ref())
        {
            core::ask_session::discover(runner.clone());
        }
    }

    pub fn ask_in_meeting(&self) -> bool {
        ask_options::in_meeting(
            self.hex_recording() || self.ask_settings.recording,
            self.ask_settings.last_end_ms,
            chrono::Local::now().timestamp_millis(),
        )
    }
    pub fn ask_thinking(&self) -> Thinking {
        if self.ask_live_meeting() {
            return self
                .ask_settings
                .meeting_level
                .unwrap_or(self.ask_settings.meeting);
        }
        let meeting = self.ask_in_meeting();
        self.ask_settings
            .overrides
            .get(&(self.ask_key(), meeting))
            .copied()
            .unwrap_or(if meeting {
                self.ask_settings.meeting
            } else {
                self.ask_settings.chat
            })
    }
    pub fn set_ask_thinking(&mut self, thinking: Thinking) {
        if self.ask_live_meeting() {
            self.ask_settings.meeting_epoch += 1;
            self.ask_settings.meeting_level = Some(thinking);
            self.ask_settings.thinking_open = false;
            self.ask_settings.warm_signature.clear();
            core::ask_session::reset(&self.ask_key());
            return;
        }
        let meeting = self.ask_in_meeting();
        self.ask_settings
            .overrides
            .insert((self.ask_key(), meeting), thinking);
        self.ask_settings.thinking_open = false;
        if !meeting {
            self.ask_settings.chat = thinking;
            self.save_ask_preferences();
        }
    }
    pub fn save_ask_preferences(&mut self) {
        let mut prefs = UiPrefs::load(&self.store.root);
        prefs.ask_meeting_thinking = self.ask_settings.meeting;
        prefs.ask_meeting_agent = self.ask_settings.meeting_agent.clone();
        prefs.ask_meeting_model = self.ask_settings.meeting_model.clone();
        prefs.ask_chat_thinking = self.ask_settings.chat;
        prefs.ask_models = self.ask_settings.models.clone();
        if prefs.save(&self.store.root).is_err() {
            self.notice = Some(
                "Could not save your Ask preferences. Check the app data folder and try again."
                    .into(),
            );
        }
    }
    pub fn choose_ask_runner(&mut self, name: String) {
        if self.ask_live_meeting() {
            let model = self
                .ask_settings
                .models
                .get(&name)
                .cloned()
                .or_else(|| self.runners.iter().find(|r| r.name == name)?.model.clone());
            self.choose_meeting_model(name, model);
            return;
        }
        self.choose_normal_ask_runner(name);
    }
    pub fn choose_normal_ask_runner(&mut self, name: String) {
        self.runner = Some(name);
        self.runner_open = false;
        if let Some(name) = &self.runner
            && core::runners::remember_runner(name).is_err()
        {
            self.notice = Some("Could not remember your agent choice.".into());
        }
        self.refresh_ask_model_input();
    }
    pub fn refresh_ask_model_input(&mut self) {
        let model = self.normal_ask_model();
        self.ask_settings.other_model = self
            .runners
            .iter()
            .find(|r| Some(&r.name) == self.runner.as_ref())
            .is_some_and(|r| {
                model.as_ref().is_some_and(|value| {
                    !core::ask_session::models(r).iter().any(|m| &m.id == value)
                })
            });
        self.ask_settings.model_input = TextEdit::new(model.as_deref().unwrap_or(""));
    }
    pub fn ask_model(&self) -> Option<String> {
        if self.ask_live_meeting() {
            return self
                .ask_settings
                .meeting_override
                .as_ref()
                .map(|(_, model)| model.clone())
                .unwrap_or_else(|| self.ask_settings.meeting_model.clone())
                .or_else(|| {
                    self.runners
                        .iter()
                        .find(|r| Some(r.name.as_str()) == self.ask_runner_name())?
                        .model
                        .clone()
                });
        }
        if let Some((name, model)) = &self.ask_settings.pick {
            return model
                .clone()
                .or_else(|| self.ask_settings.models.get(name).cloned())
                .or_else(|| self.runners.iter().find(|r| &r.name == name)?.model.clone());
        }
        self.normal_ask_model()
    }
    /// A choice from the model picker: in a live meeting it stays for the meeting, elsewhere
    /// it applies to the next question only.
    pub fn choose_ask_pick(&mut self, name: String, model: Option<String>) {
        if self.ask_live_meeting() {
            self.choose_meeting_model(name, model);
        } else {
            self.ask_settings.pick = Some((name, model));
            self.runner_open = false;
        }
    }
    /// Back to what Settings say.
    pub fn choose_ask_default(&mut self) {
        self.runner_open = false;
        if self.ask_live_meeting() {
            self.ask_settings.fallback_notice = None;
            self.ask_settings.meeting_epoch += 1;
            self.ask_settings.meeting_override = None;
            self.ask_settings.warm_signature.clear();
            core::ask_session::release();
        } else {
            self.ask_settings.pick = None;
        }
    }
    pub fn save_ask_model(&mut self) {
        let Some(name) = self.runner.clone() else {
            return;
        };
        let model = self.ask_settings.model_input.text.trim().to_string();
        if model.is_empty() {
            self.ask_settings.models.remove(&name);
        } else {
            self.ask_settings.models.insert(name, model);
        }
        self.save_ask_preferences();
        self.refresh_ask_model_input();
    }
}

fn thinking(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, meeting: bool) -> El {
    if !meeting
        && s.runners
            .iter()
            .find(|r| Some(&r.name) == s.runner.as_ref())
            .is_some_and(|r| !ask_options::supports_thinking(r))
    {
        return roles::META
            .wrap("Managed by OpenCode", p.faint)
            .help(OPENCODE_THINKING);
    }
    let key = if meeting {
        "ask-meeting-thinking"
    } else {
        "ask-chat-thinking"
    };
    let value = if meeting {
        s.ask_settings.meeting
    } else {
        s.ask_settings.chat
    };
    let levels = if meeting {
        &Thinking::ALL[..2]
    } else {
        &Thinking::ALL[..]
    };
    let options: Vec<_> = levels.iter().map(|v| v.label().to_string()).collect();
    Select::new(key, &options)
        .label(if meeting {
            "Thinking during meetings"
        } else {
            "Thinking in chats"
        })
        .current(Some(value.label()))
        .appearance(SelectAppearance::Field)
        .width(short_select_width(s, 180.0))
        .open(s.ask_settings.select_open.as_deref() == Some(key))
        .on_toggle(move |s: &mut State| {
            s.ask_settings.select_open = if s.ask_settings.select_open.as_deref() == Some(key) {
                None
            } else {
                Some(key.into())
            };
        })
        .on_close(|s: &mut State| s.ask_settings.select_open = None)
        .on_choose(move |s: &mut State, value: &str, _| {
            if let Some(value) = Thinking::ALL.into_iter().find(|v| v.label() == value) {
                if meeting {
                    s.ask_settings.meeting = value;
                } else {
                    s.ask_settings.chat = value;
                }
                s.ask_settings
                    .overrides
                    .retain(|(_, during_meeting), _| *during_meeting != meeting);
                s.save_ask_preferences();
            }
            s.ask_settings.select_open = None;
        })
        .build(k, layer)
}

pub(super) fn model_menu(
    key: &str,
    label: &str,
    models: &[core::ask_session::Model],
    current: Option<&str>,
    other: bool,
) -> Select {
    let mut options: Vec<String> = models.iter().map(|m| m.name.clone()).collect();
    options.push("Other\u{2026}".into());
    let selected = if other {
        Some("Other\u{2026}")
    } else {
        models
            .iter()
            .find(|m| Some(m.id.as_str()) == current)
            .map(|m| m.name.as_str())
    };
    Select::new(key, &options)
        .label(label)
        .current(selected)
        .placeholder("Choose a model")
        .appearance(SelectAppearance::Field)
        .width(220.0)
}

fn model_choice(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let runner = s
        .runners
        .iter()
        .find(|r| Some(&r.name) == s.runner.as_ref());
    let models = runner.map(core::ask_session::models).unwrap_or_default();
    let current = s.normal_ask_model();
    let other = s.ask_settings.other_model
        || current
            .as_ref()
            .is_some_and(|id| !models.iter().any(|m| &m.id == id));
    let select = model_menu(
        "ask-settings-model",
        "Ask model",
        &models,
        current.as_deref(),
        other,
    )
    .width(short_select_width(s, 220.0))
    .disabled(runner.is_none())
    .open(s.ask_settings.select_open.as_deref() == Some("ask-settings-model"))
    .on_toggle(|s: &mut State| {
        let opening = s.ask_settings.select_open.as_deref() != Some("ask-settings-model");
        s.ask_settings.select_open = opening.then(|| "ask-settings-model".into());
        if opening {
            s.discover_ask_models();
        }
    })
    .on_close(|s: &mut State| s.ask_settings.select_open = None)
    .on_choose(|s: &mut State, value: &str, _| {
        s.ask_settings.select_open = None;
        if value == "Other\u{2026}" {
            s.ask_settings.other_model = true;
            return;
        }
        if let Some(runner) = s
            .runners
            .iter()
            .find(|r| Some(&r.name) == s.runner.as_ref())
            && let Some(model) = core::ask_session::models(runner)
                .into_iter()
                .find(|m| m.name == value)
        {
            s.ask_settings.model_input = TextEdit::new(&model.id);
            s.save_ask_model();
        }
    })
    .build(k, layer);
    let mut control = col().gap(8.0).child(select);
    if other {
        control = control.child(
            row()
                .gap(8.0)
                .items_center()
                .flex_wrap()
                .child(
                    TextInput::new(&s.ask_settings.model_input)
                        .label("Other model")
                        .placeholder("Model name")
                        .height(30.0)
                        .on_edit(|s: &mut State| &mut s.ask_settings.model_input)
                        .on_submit(|s: &mut State, _| s.save_ask_model())
                        .build(k)
                        .key("ask-settings-model-input")
                        .w(205.0),
                )
                .child(
                    super::widgets::row_button(
                        "Save",
                        "ask-settings-model-save",
                        super::widgets::ButtonRole::Secondary,
                        p,
                    )
                    .on_press(|s: &mut State| s.save_ask_model()),
                ),
        );
    }
    control
}

pub(super) fn meeting_choices(s: &State) -> Vec<(String, String, Option<String>)> {
    let mut choices = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for runner in s
        .runners
        .iter()
        .filter(|r| s.runner_found.contains(&r.name))
    {
        let agent = ask_options::agent_name(runner);
        if !seen.insert(agent) {
            continue;
        }
        choices.push((format!("{agent} \u{b7} Default"), runner.name.clone(), None));
        for model in core::ask_session::models(runner) {
            choices.push((
                format!("{agent} \u{b7} {}", model.name),
                runner.name.clone(),
                Some(model.id),
            ));
        }
        if let Some(model) = s.ask_settings.models.get(&runner.name)
            && !choices
                .iter()
                .any(|(_, name, id)| name == &runner.name && id.as_ref() == Some(model))
        {
            choices.push((
                format!("{agent} \u{b7} {model}"),
                runner.name.clone(),
                Some(model.clone()),
            ));
        }
    }
    choices
}
/// What Settings resolve Ask to: the helper and model a question uses with no pick made.
pub(super) fn default_choice(s: &State) -> (Option<String>, Option<String>) {
    if s.ask_live_meeting() {
        let name = s.ask_settings.meeting_agent.clone();
        let model = s
            .ask_settings
            .meeting_model
            .clone()
            .or_else(|| s.runners.iter().find(|r| r.name == name)?.model.clone());
        (Some(name), model)
    } else {
        (s.runner.clone(), s.normal_ask_model())
    }
}

/// One group per installed helper: its name, the runner that runs it, and its models as
/// (menu name, id). Built from the runner list and the model catalogue, never hard-coded; a
/// helper with no listed model gets one "Provider default" row.
pub(super) type PickerGroup = (String, String, Vec<(String, Option<String>)>);
pub(super) fn picker_groups(s: &State) -> Vec<PickerGroup> {
    let mut groups: Vec<PickerGroup> = Vec::new();
    for runner in s
        .runners
        .iter()
        .filter(|r| s.runner_found.contains(&r.name))
    {
        let agent = ask_options::agent_name(runner);
        if groups.iter().any(|g| g.0 == agent) {
            continue;
        }
        let mut models: Vec<_> = core::ask_session::models(runner)
            .into_iter()
            .map(|m| (m.name, Some(m.id)))
            .collect();
        if models.is_empty() {
            models.push(("Provider default".into(), None));
        }
        groups.push((agent.to_string(), runner.name.clone(), models));
    }
    groups
}
fn meeting_agent(s: &State, k: &Kit, layer: &mut Vec<El>) -> El {
    let choices = meeting_choices(s);
    let options: Vec<_> = choices.iter().map(|(label, _, _)| label.clone()).collect();
    let selected = choices.iter().find(|(_, name, model)| {
        name == &s.ask_settings.meeting_agent && model == &s.ask_settings.meeting_model
    });
    let current = selected
        .map(|(label, _, _)| label.as_str())
        .unwrap_or(&s.ask_settings.meeting_agent);
    let open = s.ask_settings.select_open.as_deref() == Some("ask-meeting-agent");
    let trigger = Select::new("ask-meeting-agent", &options)
        .label("Agent during meetings")
        .current(Some(current))
        .appearance(SelectAppearance::Field)
        .width(short_select_width(s, 220.0))
        .disabled(options.is_empty())
        .open(open)
        .on_toggle(|s: &mut State| {
            s.ask_settings.select_open = (s.ask_settings.select_open.as_deref()
                != Some("ask-meeting-agent"))
            .then(|| "ask-meeting-agent".into());
        })
        .trigger(k);
    let mut menu = Menu::new("ask-meeting-agent-menu")
        .trigger(trigger)
        .open(open)
        .placement(Placement::BottomEnd)
        .min_width(320.0)
        .on_close(|s: &mut State| s.ask_settings.select_open = None);
    for (label, name, model) in choices {
        let checked = name == s.ask_settings.meeting_agent && model == s.ask_settings.meeting_model;
        menu = menu.item(
            MenuItem::new(label)
                .checked(checked)
                .on_select(move |s: &mut State| {
                    s.ask_settings.meeting_agent = name.clone();
                    s.ask_settings.meeting_model = model.clone();
                    s.save_ask_preferences();
                    s.ask_settings.select_open = None;
                }),
        );
    }
    menu.build(k, layer)
}

pub fn section(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let k = &row_field_kit(k, p);
    let options: Vec<String> = s
        .runners
        .iter()
        .filter(|r| s.runner_found.contains(&r.name))
        .map(ask_options::agent_name)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(str::to_string)
        .collect();
    let current = s
        .runners
        .iter()
        .find(|r| Some(&r.name) == s.runner.as_ref())
        .map(ask_options::agent_name)
        .map(str::to_string);
    let agent =
        Select::new("ask-agent", &options)
            .label("Ask agent")
            .current(current.as_deref())
            .placeholder("No agent found")
            .disabled(options.is_empty())
            .appearance(SelectAppearance::Field)
            .width(short_select_width(s, 180.0))
            .open(s.ask_settings.select_open.as_deref() == Some("ask-agent"))
            .on_toggle(|s: &mut State| {
                s.ask_settings.select_open =
                    if s.ask_settings.select_open.as_deref() == Some("ask-agent") {
                        None
                    } else {
                        Some("ask-agent".into())
                    };
            })
            .on_close(|s: &mut State| s.ask_settings.select_open = None)
            .on_choose(|s: &mut State, value: &str, _| {
                if let Some(r) = s.runners.iter().find(|r| {
                    ask_options::agent_name(r) == value && s.runner_found.contains(&r.name)
                }) {
                    s.choose_normal_ask_runner(r.name.clone());
                }
                s.ask_settings.select_open = None;
            })
            .build(k, layer);
    let model = model_choice(s, k, p, layer);
    let body = col()
        .w_full()
        .child(setting_row(
            s,
            "ask-agent-row",
            "Agent",
            "Answers Ask using an agent installed on this PC.",
            agent,
            true,
            p,
        ))
        .child(setting_row(
            s,
            "ask-model-row",
            "Model",
            "Choose an offered model, or choose Other to enter a model name.",
            model,
            true,
            p,
        ))
        .child(setting_row(
            s,
            "ask-meeting-agent-row",
            "Agent during meetings",
            "Used for questions while a meeting is live. Low thinking keeps answers fast.",
            meeting_agent(s, k, layer),
            true,
            p,
        ))
        .child(setting_row(
            s,
            "ask-meeting-thinking-row",
            "Thinking during meetings",
            "Low keeps meeting answers fast. You can override it in Ask.",
            thinking(s, k, p, layer, true),
            true,
            p,
        ))
        .child(setting_row(
            s,
            "ask-chat-thinking-row",
            "Thinking in chats",
            "The last level you choose outside meetings is remembered.",
            thinking(s, k, p, layer, false),
            true,
            p,
        ));
    card("Ask", body, p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chat_overrides_and_models_persist_without_changing_meeting_defaults_or_summary_settings() {
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;
        use std::{fs, process::Command};
        if std::env::var_os("SIDENOTE_ASK_PREFS_CHILD").is_none() {
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join(format!("ask-prefs-{}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "ui::ask_settings::tests::chat_overrides_and_models_persist_without_changing_meeting_defaults_or_summary_settings", "--nocapture"])
                .env("SIDENOTE_ASK_PREFS_CHILD", "1")
                .env("APPDATA", root.join("appdata"))
                .env("LOCALAPPDATA", root.join("localappdata"))
                .env("USERPROFILE", root.join("home"))
                .env("SIDENOTE_DATA_DIR", root.join("state"))
                .env("SIDENOTE_STATE_DIR", root.join("state"))
                .env("SIDENOTE_HEX_DIR", root.join("meetings"))
                .env("SIDENOTE_PEERS_DIR", root.join("peers"))
                ;
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let status = command.status().unwrap();
            assert!(status.success());
            return;
        }
        let mut state = State::new();
        let root = state.store.root.clone();
        let settings = core::notes::NotesSettings::default();
        core::notes::save_settings(&core::notes::settings().unwrap(), &settings).unwrap();
        let summary_before = serde_json::to_value(&settings).unwrap();
        assert_eq!(state.ask_thinking(), Thinking::Medium);
        state.set_ask_thinking(Thinking::High);
        assert_eq!(UiPrefs::load(&root).ask_chat_thinking, Thinking::High);
        state.selected = Some("1234-1".into());
        assert_eq!(state.ask_thinking(), Thinking::High);
        state.ask_settings.last_end_ms = Some(chrono::Local::now().timestamp_millis());
        assert_eq!(state.ask_thinking(), Thinking::Low);
        state.set_ask_thinking(Thinking::Medium);
        assert_eq!(state.ask_thinking(), Thinking::Medium);
        state.selected = Some("1234-2".into());
        assert_eq!(state.ask_thinking(), Thinking::Low);
        state.ask_settings.last_end_ms = Some(chrono::Local::now().timestamp_millis() - 600001);
        assert_eq!(state.ask_thinking(), Thinking::High);
        state.choose_ask_runner("Codex".into());
        state.ask_settings.model_input = TextEdit::new("chosen-model");
        state.save_ask_model();
        assert_eq!(state.ask_model().as_deref(), Some("chosen-model"));
        let prefs = UiPrefs::load(&root);
        assert_eq!(prefs.ask_meeting_thinking, Thinking::Low);
        assert_eq!(prefs.ask_chat_thinking, Thinking::High);
        assert_eq!(
            prefs.ask_models.get("Codex").map(String::as_str),
            Some("chosen-model")
        );
        assert_eq!(core::last_used_runner().unwrap().as_deref(), Some("Codex"));
        assert_eq!(
            serde_json::to_value(core::notes::settings().unwrap()).unwrap(),
            summary_before
        );
        state.choose_normal_ask_runner("Claude Sonnet".into());
        state.ask_settings.recording = true;
        state.meeting_agent_transition();
        assert_eq!(state.ask_runner_name(), Some("Codex"));
        assert_eq!(state.ask_thinking(), Thinking::Low);
        assert_ne!(state.ask_model().as_deref(), Some("chosen-model"));
        state.question = TextEdit::new("Keep my typed question");
        state.choose_meeting_model("OpenCode".into(), Some("test-model".into()));
        state.set_ask_thinking(Thinking::Medium);
        assert_eq!(state.ask_runner_name(), Some("OpenCode"));
        assert_eq!(state.ask_model().as_deref(), Some("test-model"));
        assert_eq!(state.question.text, "Keep my typed question");
        let saved = UiPrefs::load(&root);
        assert_eq!(saved.ask_meeting_agent, "Codex");
        assert_eq!(saved.ask_meeting_thinking, Thinking::Low);
        assert_eq!(saved.ask_chat_thinking, Thinking::High);
        assert_eq!(
            core::last_used_runner().unwrap().as_deref(),
            Some("Claude Sonnet")
        );
        state.ask_settings.recording = false;
        state.meeting_agent_transition();
        assert_eq!(state.ask_runner_name(), Some("Claude Sonnet"));
        state.ask_settings.recording = true;
        state.meeting_agent_transition();
        let id = state.ask_key();
        state
            .asks
            .insert(id.clone(), super::super::AskRun::stand_in("Codex", &[], 0));
        for event in [
            core::AskProgress::StartingCodex,
            core::AskProgress::Text {
                text: String::new(),
            },
        ] {
            state.apply_event(super::super::Event::Progress {
                id: id.clone(),
                generation: 0,
                event,
            });
            assert!(state.asks[&id].starting_codex);
            assert_eq!(state.ask_runner_name(), Some("Codex"));
            assert!(state.ask_settings.fallback_notice.is_none());
        }
        state.apply_event(super::super::Event::Progress {
            id: id.clone(),
            generation: 0,
            event: core::AskProgress::Started {
                runner: "Codex".into(),
            },
        });
        assert!(!state.asks[&id].starting_codex);
        assert!(state.asks[&id].turn_started);
        assert!(state.asks[&id].started.elapsed().as_secs() < 1);
        state.apply_event(super::super::Event::Progress {
            id: id.clone(),
            generation: 0,
            event: core::AskProgress::Text {
                text: "First answer text".into(),
            },
        });
        assert!(!state.asks[&id].starting_codex);
        assert_eq!(state.asks[&id].runner, "Codex");
        state.asks.remove(&id);
        state.ask_settings.recording = false;
        state.meeting_agent_transition();
        assert_eq!(state.ask_thinking(), Thinking::High);
        state.ask_settings.recording = true;
        assert_eq!(state.ask_runner_name(), Some("Codex"));
        assert_eq!(state.ask_thinking(), Thinking::Low);

        let id = state.ask_key();
        let (_, message_id) = state
            .store
            .append_message_identified(
                &id,
                core::Message {
                    role: "user".into(),
                    text: "Synthetic retained question".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let mut run = super::super::AskRun::stand_in("Codex", &[], 0);
        run.submitted_message_id = Some(message_id);
        state.question = TextEdit::default();
        state.finish_ask(&id, run, Some("Both agents unavailable".into()));
        assert_eq!(state.question.text, "Synthetic retained question");
        assert!(state.store.load(&id).unwrap().messages.is_empty());

        let (_, message_id) = state
            .store
            .append_message_identified(
                &id,
                core::Message {
                    role: "user".into(),
                    text: "Keep this unanswered question too".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let mut run = super::super::AskRun::stand_in("Codex", &[], 0);
        run.submitted_message_id = Some(message_id);
        run.meeting_request = true;
        state.question = TextEdit::new("A newer draft");
        state.finish_ask(&id, run, Some("Both agents unavailable".into()));
        assert_eq!(state.question.text, "A newer draft");
        assert_eq!(
            state.store.load(&id).unwrap().messages.last().unwrap().text,
            "Keep this unanswered question too"
        );

        let (_, message_id) = state
            .store
            .append_message_identified(
                &id,
                core::Message {
                    role: "user".into(),
                    text: "Synthetic question at meeting end".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Ok("Synthetic completed answer".into())).unwrap();
        let mut run = super::super::AskRun::stand_in("Codex", &[], 0);
        run.submitted_message_id = Some(message_id);
        run.rx = rx;
        state.ask_settings.recording = false;
        state.meeting_agent_transition();
        state.finish_ask(&id, run, None);
        assert_eq!(
            state.store.load(&id).unwrap().messages.last().unwrap().text,
            "Synthetic completed answer"
        );
        assert_eq!(state.ask_runner_name(), Some("Claude Sonnet"));

        state.selected = None;
        state.ask_settings.recording = true;
        state.meeting_agent_transition();
        state.live = Some(crate::ui::live::Live::scene(
            crate::ui::live::Phase::Recording,
            None,
        ));
        state.live_open = true;
        assert!(state.ask_current_live());
        let live_key = state.ask_key();
        assert_ne!(live_key, core::ALL_KEY);
        state
            .store
            .append_message(
                &live_key,
                core::Message {
                    role: "user".into(),
                    text: "Only this meeting".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        state.question = TextEdit::new("Keep the draft while changing scope");
        state.set_ask_scope(true);
        assert_eq!(state.ask_key(), core::ALL_KEY);
        assert!(!state.ask_current_live());
        state.set_ask_scope(false);
        assert_eq!(state.ask_key(), live_key);
        assert_eq!(
            state.saved.messages.last().unwrap().text,
            "Only this meeting"
        );
        assert_eq!(state.question.text, "Keep the draft while changing scope");
        state.ask_settings.recording = false;
        assert_eq!(state.ask_key(), core::ALL_KEY);
    }
}
