//! AI notes controls. Disk access and helper calls return through one worker mailbox.
use super::{Event, State, area::button, roles, theme::Palette};
use super::{
    cards,
    hex::settings,
    widgets::{ButtonRole, row_action},
};
use rune::{El, TextEdit, col, row};
use rune_kit::{Kit, Switch, TextInput};
use sidenote::core::{
    agent_instructions::{self as instructions, Agent},
    notes::{self, Helper, NotesSettings, Request},
    prompt_library as library,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    thread,
};

pub const HELPERS: [Helper; 3] = [Helper::Claude, Helper::Codex, Helper::OpenCode];
pub const INSTALL_URL: &str = "https://code.claude.com/docs/en/setup";
const SETUP_STEPS: &str =
    "1. Install Claude Code. 2. Open a terminal, type claude, and sign in. 3. Press Check again.";
pub const LOAD_ERROR: &str = "Sidenote could not load Meeting summary settings. Press Reload agent status. If it fails again, restart Sidenote.";
pub const SAVE_ERROR: &str = "Sidenote could not save your instructions. Your text is still here. Try Save instructions again. If it fails twice, copy the text and restart Sidenote.";
const SWITCH_CONFIRM: &str = "Built-in summaries use only Hex's own transcript. The extra Telugu and Hindi passes and the translation step from your scripts do not run yet. If your scripts also run from somewhere else, such as a scheduled task, turn that off, or both will write meeting summaries.";

pub fn helper_fallback_message(helper: Helper) -> String {
    format!(
        "{} is coming in a later version. Choose Claude Code below to turn on Meeting summary.",
        helper.name()
    )
}

fn summary_agent(settings: &NotesSettings) -> Option<Helper> {
    settings
        .effective_helper()
        .filter(|_| settings.engine == "app" && settings.enabled_at_ms.is_some())
}

fn uses_summary_script(settings: &NotesSettings) -> bool {
    settings.engine == "scripts" && settings.script.is_some()
}

fn request_problem(settings: Option<&NotesSettings>) -> Option<String> {
    if let Some(settings) = settings {
        if let Some(helper) = settings.helper
            && settings.helper_reason().is_some()
        {
            return Some(helper_fallback_message(helper));
        }
        if summary_agent(settings).is_some() {
            return None;
        }
    }
    Some(
        "Meeting summaries are off. Choose an agent in Meeting summary settings, then try again."
            .into(),
    )
}

/// What a save or reset was asked to do when it found another window's newer copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retry {
    Save,
    Reset,
}
/// Another window changed this prompt after this editor started from its copy. Nothing was
/// written. You keep your text, take the other copy, or compare the two first.
pub struct Conflict {
    /// What is stored now: the other window's copy, or `None` when it is back at the default.
    pub theirs: Option<instructions::Entry>,
    pub retry: Retry,
    /// Showing the two copies side by side, as a diff from theirs to yours.
    pub both: bool,
    pub compare: Option<super::prompt_library::DiffCache>,
}
impl Conflict {
    pub(super) fn new(theirs: Option<instructions::Entry>, retry: Retry) -> Self {
        Self {
            theirs,
            retry,
            both: false,
            compare: None,
        }
    }
}
pub struct Editor {
    pub text: TextEdit,
    pub saved: String,
    pub edited_at: Option<String>,
    pub reset_confirm: bool,
    /// The saved text this editor started from, or `None` for the default. A save is checked
    /// against it, so a window with an old copy cannot overwrite a newer one.
    pub base: Option<String>,
    pub conflict: Option<Conflict>,
}
impl Editor {
    fn new(agent: Agent) -> Self {
        let text = instructions::default_base(agent).to_string();
        Self {
            text: TextEdit::new(&text),
            saved: text,
            edited_at: None,
            reset_confirm: false,
            base: None,
            conflict: None,
        }
    }
    /// The store was read again. A clean editor takes the new copy. An editor with unsaved text
    /// keeps its text and its starting point, and flags a conflict when the store moved on.
    fn loaded(&mut self, agent: Agent, entry: Option<instructions::Entry>, reset: bool) {
        self.apply(agent, entry, reset, false);
    }
    /// This window's own save or reset finished, so what is stored is the new starting point.
    fn stored(&mut self, agent: Agent, entry: Option<instructions::Entry>, reset: bool) {
        self.apply(agent, entry, reset, true);
    }
    fn apply(&mut self, agent: Agent, entry: Option<instructions::Entry>, reset: bool, own: bool) {
        let text = entry
            .as_ref()
            .map(|e| e.text.clone())
            .unwrap_or_else(|| instructions::default_base(agent).into());
        let disk = entry.as_ref().map(|e| e.text.clone());
        let fresh = reset || !self.dirty();
        if fresh {
            self.text = TextEdit::new(&text);
        }
        if fresh || own {
            self.base = disk;
            self.conflict = None;
        } else if self.base != disk {
            let retry = self.conflict.as_ref().map_or(Retry::Save, |c| c.retry);
            self.conflict = Some(Conflict::new(entry.clone(), retry));
        }
        self.saved = text;
        self.edited_at = entry.map(|e| e.edited_at);
        self.reset_confirm = false;
    }
    pub(super) fn dirty(&self) -> bool {
        self.text.text != self.saved
    }
}
pub struct AiNotes {
    pub settings: Option<NotesSettings>,
    pub found: Vec<(Helper, bool)>,
    pub models: [TextEdit; 3],
    pub name: TextEdit,
    /// One editor per prompt with editable instructions, in `Agent::ALL` order.
    pub editors: [Editor; Agent::ALL.len()],
    pub library: super::prompt_library::Library,
    pub pending: Option<&'static str>,
    pub editor_target: Option<usize>,
    pub testing: Option<usize>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub test_results: BTreeMap<usize, String>,
    pub jobs: BTreeMap<String, super::notes_status::Job>,
    pub dismissed: BTreeSet<String>,
    pub details: BTreeSet<String>,
    pub privacy_open: bool,
    pub model_open: bool,
    pub other_model: bool,
    pub(super) helper_save: bool,
    pub retry_after_save: Option<(String, Request)>,
    pub ask_setup_open: bool,
    pub fallback: Option<Helper>,
    pub switch_confirm: Option<Helper>,
    pub generation: u64,
    pub scene: bool,
}
impl AiNotes {
    fn prepare_codex_recovery(&mut self, id: String) -> Option<NotesSettings> {
        let job = self.jobs.get(&id)?;
        if !super::notes_status::can_use_codex(job, &self.found) {
            return None;
        }
        let mut settings = self.settings.clone()?;
        settings.engine = "app".into();
        settings.helper = Some(Helper::Codex);
        settings.model = self.models[1].text.trim().into();
        self.retry_after_save = Some((id, job.retry_request()));
        Some(settings)
    }
    fn saved_recovery_request(&mut self) -> Option<(String, Request)> {
        self.retry_after_save
            .take()
            .map(|(id, _)| (id, Request::UseCodex))
    }
}
impl Default for AiNotes {
    fn default() -> Self {
        Self {
            settings: None,
            found: Vec::new(),
            models: HELPERS.map(|h| TextEdit::new(h.default_model())),
            name: TextEdit::default(),
            editors: Agent::ALL.map(Editor::new),
            library: Default::default(),
            pending: None,
            editor_target: None,
            testing: None,
            error: None,
            notice: None,
            test_results: BTreeMap::new(),
            jobs: BTreeMap::new(),
            dismissed: BTreeSet::new(),
            details: BTreeSet::new(),
            privacy_open: false,
            model_open: false,
            other_model: false,
            helper_save: false,
            retry_after_save: None,
            ask_setup_open: false,
            fallback: None,
            switch_confirm: None,
            generation: 0,
            scene: false,
        }
    }
}
pub struct Snapshot {
    settings: NotesSettings,
    found: Vec<(Helper, bool)>,
    store: instructions::Store,
}
const INSTRUCTIONS_LOAD_ERROR: &str = "Sidenote could not read your instructions. Your saved text was kept. Press Reload agent status. If it fails again, restart Sidenote.";
const SAVING_HELPER: &str = "Saving Meeting summary...";

pub(super) fn instructions_warning(store: &instructions::Store) -> Option<String> {
    store
        .using_defaults_after_error
        .then(|| INSTRUCTIONS_LOAD_ERROR.to_owned())
}
impl Snapshot {
    fn read(root: &Path) -> Result<Self, String> {
        Ok(Self {
            settings: notes::settings().map_err(|_| LOAD_ERROR.to_string())?,
            found: notes::detect(),
            store: instructions::load(root).map_err(|_| INSTRUCTIONS_LOAD_ERROR.to_string())?,
        })
    }
}
pub enum Completion {
    Loaded(Box<Snapshot>),
    Settings(Box<sidenote::core::Config>),
    Instructions(usize, Option<instructions::Entry>, bool),
    /// A save or reset found another window's newer copy and wrote nothing.
    Conflict(usize, Option<instructions::Entry>, Retry),
    Tested(usize, Result<std::time::Duration, String>),
    Requested(String),
}
enum Work {
    Load,
    Settings {
        baseline: Box<NotesSettings>,
        edited: NotesSettings,
    },
    /// The prompt, your text, and the saved text this editor started from.
    Save(usize, String, Option<String>),
    Reset(usize, Option<String>),
    Test(usize, String),
    Request(String, Request),
}
fn agent(index: usize) -> Agent {
    Agent::ALL[index]
}
fn run(root: &Path, work: Work) -> Result<Completion, String> {
    match work {
        Work::Load => Snapshot::read(root).map(|s| Completion::Loaded(Box::new(s))),
        Work::Settings { baseline, edited } => {
            notes::save_settings(&baseline, &edited).map_err(|_| {
                "Could not save Meeting summary settings. Your previous choice was kept."
                    .to_string()
            })?;
            sidenote::core::Config::load_in(root)
                .map(|config| Completion::Settings(Box::new(config)))
                .map_err(|_| {
                    "Settings were written but could not be read back. Reload to check.".into()
                })
        }
        Work::Save(index, text, baseline) => {
            let entry = library::for_agent(agent(index));
            let saved = library::save(root, entry, &text, baseline.as_deref())
                .map_err(|_| SAVE_ERROR.to_string())?;
            if let library::Saved::Conflict(theirs) = saved {
                return Ok(Completion::Conflict(index, theirs, Retry::Save));
            }
            let store = instructions::load(root).map_err(|_| {
                "Instructions were written but could not be read back. Reload to check.".to_string()
            })?;
            // Text equal to the default removes your copy, so the editor shows the default.
            Ok(Completion::Instructions(
                index,
                store.get(agent(index)).cloned(),
                saved == library::Saved::Default,
            ))
        }
        Work::Reset(index, baseline) => {
            let reset = library::reset(root, library::for_agent(agent(index)), baseline.as_deref())
                .map_err(|_| "Sidenote could not reset your instructions. Your text is still here. Press Reset to default again.".to_string())?;
            if let library::Saved::Conflict(theirs) = reset {
                return Ok(Completion::Conflict(index, theirs, Retry::Reset));
            }
            let store = instructions::load(root).map_err(|_| {
                "Instructions were reset but could not be read back. Reload to check.".to_string()
            })?;
            Ok(Completion::Instructions(
                index,
                store.get(agent(index)).cloned(),
                true,
            ))
        }
        Work::Test(index, _) if !HELPERS[index].offered() => {
            Err(helper_fallback_message(HELPERS[index]))
        }
        Work::Test(index, model) => Ok(Completion::Tested(
            index,
            notes::test_helper(HELPERS[index], &model),
        )),
        Work::Request(id, mode) => {
            notes::request(&id, mode).map_err(|_| "Could not queue a meeting summary. Choose an installed agent in Meeting summary settings, then try again.".to_string())?;
            Ok(Completion::Requested(id))
        }
    }
}
impl State {
    fn notes_save_settings(&mut self, edited: NotesSettings, label: &'static str) {
        if let Some(baseline) = self.ai_notes.settings.clone() {
            self.notes_work(
                Work::Settings {
                    baseline: Box::new(baseline),
                    edited,
                },
                label,
            );
        }
    }
    fn notes_work(&mut self, work: Work, label: &'static str) {
        if self.ai_notes.pending.is_some() || self.ai_notes.scene {
            return;
        }
        self.ai_notes.pending = Some(label);
        self.ai_notes.helper_save = label == SAVING_HELPER;
        self.ai_notes.editor_target = match &work {
            Work::Save(i, ..) | Work::Reset(i, _) => Some(*i),
            _ => None,
        };
        self.ai_notes.testing = match &work {
            Work::Test(i, _) => Some(*i),
            _ => None,
        };
        self.ai_notes.error = None;
        self.ai_notes.notice = None;
        self.ai_notes.generation += 1;
        let generation = self.ai_notes.generation;
        let (root, events, waker) = (
            self.store.root.clone(),
            self.events_tx.clone(),
            self.waker.clone(),
        );
        if thread::Builder::new()
            .name("ai-notes-operation".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| run(&root, work)).unwrap_or_else(|_| {
                    Err("Meeting summaries hit a problem. Reload before trying again.".into())
                });
                let _ = events.send(Event::AiNotes {
                    generation,
                    worker: thread::current().id(),
                    result,
                });
                if let Some(waker) = waker {
                    waker();
                }
            })
            .is_err()
        {
            self.ai_notes.pending = None;
            self.ai_notes.retry_after_save = None;
            self.ai_notes.error =
                Some("Could not start the Meeting summary worker. Try again.".into());
        }
    }
    pub fn refresh_ai_notes(&mut self) {
        if !self.ai_notes.scene {
            self.reload_runners();
            self.agents_rechecked = !self.has_agent();
        }
        self.notes_work(Work::Load, "Loading Meeting summary...");
    }
    #[cfg(windows)]
    fn open_notes_install_page(&mut self) {
        if self.ai_notes.scene {
            return;
        }
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let url: Vec<u16> = INSTALL_URL.encode_utf16().chain(Some(0)).collect();
        // A null verb uses the registered default URL handler.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            self.ai_notes.error = Some(format!(
                "Could not open your browser. Open {INSTALL_URL} to install Claude Code."
            ));
        }
    }
    #[cfg(not(windows))]
    fn open_notes_install_page(&mut self) {
        self.ai_notes.error = Some(format!("Open {INSTALL_URL} to install Claude Code."));
    }
    pub fn open_ai_notes(&mut self) {
        if self.ai_notes.scene {
            self.go_home();
            self.hex_open = true;
        } else {
            self.open_hex();
        }
        self.hex.section = super::hex::Section::Connect;
        self.hex.filter = TextEdit::default();
        self.nav_done();
        super::scene_scroll(self, "ai-notes-settings");
        if self.ai_notes.settings.is_none() {
            self.refresh_ai_notes();
        }
    }
    pub fn notes_finished(&mut self, generation: u64, result: Result<Completion, String>) {
        if generation != self.ai_notes.generation {
            return;
        }
        self.ai_notes.pending = None;
        match result {
            Err(error) => {
                self.ai_notes.retry_after_save = None;
                self.ai_notes.error = Some(error);
            }
            Ok(Completion::Loaded(snapshot)) => {
                let s = &mut self.ai_notes;
                s.error = instructions_warning(&snapshot.store);
                s.fallback = snapshot
                    .settings
                    .helper
                    .filter(|_| snapshot.settings.helper_reason().is_some());
                for (index, h) in HELPERS.iter().enumerate() {
                    s.models[index] = TextEdit::new(
                        if snapshot.settings.helper == Some(*h)
                            && !snapshot.settings.model.trim().is_empty()
                        {
                            &snapshot.settings.model
                        } else {
                            h.default_model()
                        },
                    );
                }
                s.name = TextEdit::new(&snapshot.settings.your_name);
                s.settings = Some(snapshot.settings);
                s.found = snapshot.found;
                for index in 0..Agent::ALL.len() {
                    self.notes_editor_loaded(
                        index,
                        snapshot.store.get(agent(index)).cloned(),
                        false,
                        false,
                    );
                }
            }
            Ok(Completion::Settings(config)) => {
                let keep_draft = self.ai_notes.helper_save
                    && self.ai_notes.other_model
                    && !config.notes.as_ref().is_some_and(|saved| {
                        HELPERS.iter().enumerate().any(|(index, &helper)| {
                            saved.helper == Some(helper)
                                && self.ai_notes.models[index].text.trim() == saved.model
                        })
                    });
                if self.ai_notes.helper_save && !keep_draft {
                    self.ai_notes.other_model = false;
                }
                self.ai_notes.settings = config.notes.clone();
                self.config.refresh_notes_from(&config);
                self.ai_notes.fallback = None;
                self.hex.shell.saved = (!keep_draft)
                    .then(|| std::time::Instant::now() + std::time::Duration::from_secs(2));
                self.ai_notes.notice =
                    (!keep_draft).then(|| "Meeting summary settings saved.".into());
                if let Some((id, mode)) = self.ai_notes.saved_recovery_request() {
                    self.request_notes(id, mode);
                }
            }
            Ok(Completion::Instructions(index, entry, reset)) => {
                self.notes_editor_loaded(index, entry, reset, true);
                self.hex.shell.saved =
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
                self.ai_notes.notice = Some("Instructions saved.".into());
            }
            Ok(Completion::Conflict(index, theirs, retry)) => {
                // Your text stays in the editor and nothing was written.
                self.ai_notes.editors[index].conflict = Some(Conflict::new(theirs, retry));
                self.ai_notes.editors[index].reset_confirm = false;
                super::prompt_library::refresh(self, index);
            }
            Ok(Completion::Tested(index, result)) => {
                let text = match result {
                    Ok(duration) => format!(
                        "Worked in {} s. Only fixed sample text was sent.",
                        duration.as_secs().max(1)
                    ),
                    Err(error) => test_error(&error),
                };
                self.ai_notes.test_results.insert(index, text);
            }
            Ok(Completion::Requested(id)) => {
                self.ai_notes
                    .jobs
                    .insert(id.clone(), super::notes_status::Job::queued());
                self.ai_notes.dismissed.remove(&id);
                self.request_refresh();
                if let Some(handle) = &self.scan_tx {
                    handle.invalidate();
                }
            }
        }
    }
    fn notes_editor_loaded(
        &mut self,
        index: usize,
        entry: Option<instructions::Entry>,
        reset: bool,
        own: bool,
    ) {
        if index == 0 {
            self.meeting_ui.custom = entry.is_some();
        }
        let editor = &mut self.ai_notes.editors[index];
        if own {
            editor.stored(agent(index), entry, reset);
        } else {
            editor.loaded(agent(index), entry, reset);
        }
        super::prompt_library::refresh(self, index);
    }
    pub(super) fn notes_reset_editor(&mut self, index: usize) {
        let baseline = self.ai_notes.editors[index].base.clone();
        self.notes_work(Work::Reset(index, baseline), "Resetting instructions...");
    }
    pub(super) fn notes_save_editor(&mut self, index: usize) {
        let editor = &self.ai_notes.editors[index];
        let (text, baseline) = (editor.text.text.clone(), editor.base.clone());
        if editor.conflict.is_some() || !valid_instructions(&text) {
            return;
        }
        self.notes_work(Work::Save(index, text, baseline), "Saving instructions...");
    }
    /// Conflict choice "keep mine": run the same save or reset again, now measured against the
    /// other window's copy. If that copy changes once more, it is a new conflict.
    pub(super) fn notes_keep_mine(&mut self, index: usize) {
        if self.ai_notes.pending.is_some() {
            return;
        }
        let Some(conflict) = self.ai_notes.editors[index].conflict.take() else {
            return;
        };
        let editor = &mut self.ai_notes.editors[index];
        editor.base = conflict.theirs.as_ref().map(|e| e.text.clone());
        match conflict.retry {
            Retry::Save => self.notes_save_editor(index),
            Retry::Reset => self.notes_reset_editor(index),
        }
        if self.ai_notes.pending.is_none() {
            // The worker did not start: keep the choice on screen.
            self.ai_notes.editors[index].conflict = Some(conflict);
        }
    }
    /// Conflict choice "use theirs": your unsaved text is replaced by the stored copy.
    pub(super) fn notes_use_theirs(&mut self, index: usize) {
        let Some(conflict) = self.ai_notes.editors[index].conflict.take() else {
            return;
        };
        self.notes_editor_loaded(index, conflict.theirs, true, true);
    }
    pub fn choose_notes_helper(&mut self, index: usize) {
        let Some(mut settings) = self.ai_notes.settings.clone() else {
            return;
        };
        if !HELPERS[index].offered() || !self.ai_notes.found.contains(&(HELPERS[index], true)) {
            return;
        }
        if settings.engine == "scripts" && self.ai_notes.switch_confirm != Some(HELPERS[index]) {
            self.ai_notes.switch_confirm = Some(HELPERS[index]);
            return;
        }
        self.ai_notes.switch_confirm = None;
        settings.engine = "app".into();
        settings.helper = Some(HELPERS[index]);
        settings.model = self.ai_notes.models[index].text.trim().into();
        settings.your_name = self.ai_notes.name.text.trim().into();
        self.notes_save_settings(settings, SAVING_HELPER);
    }
    pub fn use_codex_for_summary(&mut self, id: String) {
        if self.ai_notes.pending.is_some() || self.ai_notes.scene {
            return;
        }
        if let Some(settings) = self.ai_notes.prepare_codex_recovery(id) {
            self.notes_save_settings(settings, "Switching summaries to Codex...");
        }
    }
    pub fn request_notes(&mut self, id: String, mode: Request) {
        if matches!(mode, Request::MakeNotes | Request::TryAgain)
            && self.selected.as_ref() == Some(&id)
            && sidenote::core::meeting_parts::read(&sidenote::core::state_dir(), &id)
                .ok()
                .flatten()
                .is_some_and(|session| session.phase != sidenote::core::meeting_parts::Phase::Ended)
        {
            self.meeting_generate();
            return;
        }
        if let Some(problem) = self.notes_request_problem() {
            self.ai_notes.error = Some(problem);
            return;
        }
        self.notes_work(
            Work::Request(id, mode),
            "Adding the meeting summary to the queue...",
        );
    }
    pub fn notes_request_problem(&self) -> Option<String> {
        if let Some(helper) = self.ai_notes.fallback {
            return Some(helper_fallback_message(helper));
        }
        request_problem(
            self.ai_notes
                .settings
                .as_ref()
                .or(self.config.notes.as_ref()),
        )
    }
}
pub(super) fn test_error(message: &str) -> String {
    if message == notes::LINUX_HELPER_UNAVAILABLE {
        return message.into();
    }
    if message.to_ascii_lowercase().contains("json") {
        "Claude Code replied, but Sidenote could not read the sample. Press Send a sample again. If it fails again, choose the recommended model.".into()
    } else {
        "The sample failed. Claude Code did not answer. Open a terminal, type claude, sign in, then press Send a sample.".into()
    }
}
fn needs_sign_in(found: bool, sample: Option<&str>) -> bool {
    found
        && !sample.is_some_and(|result| {
            result.starts_with("Worked in ") || result.starts_with("Claude Code replied")
        })
}
pub fn valid_instructions(text: &str) -> bool {
    text.chars().count() <= instructions::MAX_CHARS && !text.contains('\0')
}
fn field(edit: &TextEdit, label: &str) -> TextInput {
    TextInput::new(edit).label(label).placeholder(label)
}
pub(super) fn count_text(count: usize) -> String {
    if count < 1000 {
        count.to_string()
    } else {
        format!("{},{:03}", count / 1000, count % 1000)
    }
}
fn card(p: Palette) -> El {
    super::hex::settings::panel(p).padding(15.0).gap(10.0)
}
pub(super) fn edit_time(time: Option<&str>) -> String {
    time.and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("Edited %-d %b, %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Default".into())
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Identity,
    Summaries,
    Helpers,
    Advanced,
    Scripts,
    Privacy,
}
/// `extras` are lines inside the same card, under the picker: a status, a save in progress or
/// a failed save.
pub fn summary_model(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, extras: Vec<El>) -> El {
    let Some(helper) = s
        .ai_notes
        .settings
        .as_ref()
        .and_then(NotesSettings::effective_helper)
    else {
        return rune::el();
    };
    let index = HELPERS.iter().position(|h| *h == helper).unwrap();
    let models = helper.models();
    let current = &s.ai_notes.models[index].text;
    let other = s.ai_notes.other_model;
    let custom = !models.iter().any(|m| &m.id == current);
    let menu = super::ask_settings::model_menu(
        "notes-model-menu",
        "Summary model",
        &models,
        Some(current),
        other || custom,
    )
    .width(settings::short_select_width(s, 220.0))
    .disabled(s.ai_notes.pending.is_some())
    .open(s.ai_notes.model_open)
    .on_toggle(|s: &mut State| s.ai_notes.model_open = !s.ai_notes.model_open)
    .on_close(|s: &mut State| s.ai_notes.model_open = false)
    .on_choose(move |s: &mut State, value: &str, _| {
        s.ai_notes.model_open = false;
        if value == "Other\u{2026}" {
            s.ai_notes.other_model = true;
        } else if let Some(model) = helper.models().into_iter().find(|m| m.name == value) {
            s.ai_notes.models[index] = TextEdit::new(&model.id);
            s.choose_notes_helper(index);
        }
    })
    .build(&settings::row_field_kit(k, p), layer);
    let mut control = col().gap(8.0).child(menu);
    if other {
        control = control.child(
            row()
                .flex_wrap()
                .gap(8.0)
                .child(
                    field(&s.ai_notes.models[index], "Other summary model")
                        .height(30.0)
                        .on_edit(move |s: &mut State| &mut s.ai_notes.models[index])
                        .build(k)
                        .key("notes-model-other")
                        .radius(7.0)
                        .w(205.0),
                )
                .child(
                    row_action("Save model", "notes-save-model", p)
                        .disabled(s.ai_notes.pending.is_some())
                        .on_press(move |s: &mut State| s.choose_notes_helper(index)),
                ),
        );
        if s.ai_notes.helper_save
            && let Some(error) = &s.ai_notes.error
        {
            control = control.child(roles::SMALL.wrap(error, p.warn).role("status"));
        }
    } else if custom {
        control = control.child(roles::META.wrap(current, p.soft));
    }
    let row = super::hex::settings::setting_row(
        s,
        "notes-model-0",
        "Summary model",
        "Choose an offered model, or choose Other to enter a model name.",
        control,
        true,
        p,
    );
    let mut card = super::hex::settings::panel(p)
        .key("notes-model-card")
        .child(row);
    for extra in extras {
        card = card.child(extra);
    }
    card
}

pub fn section(s: &State, k: &Kit, p: Palette, part: Part) -> El {
    let a = &s.ai_notes;
    let mut c = col()
        .key("ai-notes-settings")
        .fill(super::hex::settings::group_highlight(
            s,
            "ai-notes-settings",
            p,
        ))
        .w_full()
        .gap(if part == Part::Helpers { 10.0 } else { 16.0 })
        .mt(12.0)
        .children((part != Part::Helpers).then(|| {
            roles::GROUP.wrap(
                match part {
                    Part::Privacy => "Sidenote · Summary requests",
                    Part::Identity => "You",
                    Part::Helpers => "Models and agents",
                    Part::Advanced => "Agent diagnostics",
                    Part::Summaries => "Meeting summary",
                    Part::Scripts => "Summary engine",
                },
                p.soft,
            )
        }));
    if let Some(pending) = a.pending {
        c = c.child(roles::SMALL.wrap(pending, p.soft).role("status"));
    }
    let inline_model_error = part == Part::Helpers
        && a.other_model
        && a.helper_save
        && a.settings
            .as_ref()
            .and_then(NotesSettings::effective_helper)
            .is_some();
    if let Some(error) = &a.error
        && a.editor_target.is_none()
        && !inline_model_error
    {
        c = c.child(super::widgets::banner(
            "Settings need attention",
            error,
            super::widgets::BannerKind::Problem,
            p,
        ));
    }
    if let Some(notice) = &a.notice {
        c = c.child(roles::SMALL.wrap(notice, p.ok).role("status"));
    }
    if part == Part::Advanced || a.settings.is_none() {
        c = c.child(
            button("Reload agent status", "ai-notes-reload", p)
                .self_start()
                .on_press(|s: &mut State| s.refresh_ai_notes()),
        );
    }
    let Some(settings) = &a.settings else {
        return c;
    };
    if let Some(helper) = a.fallback.or(settings
        .helper
        .filter(|_| settings.helper_reason().is_some()))
    {
        c = c.child(super::widgets::banner(
            "Saved choice not available",
            helper_fallback_message(helper),
            super::widgets::BannerKind::Info,
            p,
        ));
    }
    let agent = summary_agent(settings);
    let scripts = uses_summary_script(settings);
    let summaries_on = agent.is_some() || scripts;
    let automatic = summaries_on && settings.auto;
    let status = match (agent, scripts) {
        (Some(h), _) => format!(
            "Meeting summaries are on. Summaries use {} ({}).",
            h.name(),
            if settings.model.is_empty() || settings.model == h.default_model() {
                "default model"
            } else {
                "custom model"
            }
        ),
        (_, true) => "Your own scripts write summaries on this PC.".into(),
        _ => "Meeting summaries are off. New meetings get a transcript only.".into(),
    };
    if part == Part::Advanced {
        c = c.child(
            roles::SMALL
                .medium()
                .wrap(status, p.ink)
                .self_start()
                .px(10.0)
                .py(6.0)
                .radius(6.0)
                .fill(p.chip),
        );
    }
    if let Some((id, job)) = a
        .jobs
        .iter()
        .filter(|(_, j)| super::notes_status::is_failure(&j.state))
        .max_by_key(|(_, j)| j.updated_at)
    {
        c = c.child(
            super::widgets::banner(
                "Last summary problem",
                super::notes_status::message(job, settings.helper),
                super::widgets::BannerKind::Problem,
                p,
            )
            .children(super::notes_status::use_codex_button(
                s,
                id,
                job,
                "settings-use-codex",
                p,
            )),
        );
    }

    if part == Part::Summaries {
        let summary = col()
            .w_full()
            .child(super::hex::settings::setting_row(
                s,
                "notes-options",
                "Write a meeting summary when a meeting ends",
                if !summaries_on {
                    "Choose an AI agent to write meeting summaries."
                } else if scripts {
                    "Your configured script writes it when the meeting ends."
                } else {
                    "Your selected agent writes it as soon as the meeting ends."
                },
                Switch::new(automatic)
                    .disabled(!summaries_on)
                    .label("Write a meeting summary when a meeting ends")
                    .hide_label()
                    .on_toggle(|s: &mut State| {
                        if let Some(mut settings) = s.ai_notes.settings.clone() {
                            settings.auto = !settings.auto;
                            s.notes_save_settings(
                                settings,
                                "Saving automatic meeting summaries...",
                            );
                        }
                    })
                    .build(k)
                    .key("notes-auto"),
                true,
                p,
            ))
            .child(super::hex::settings::setting_row(
                s,
                "summary-template",
                "Summary template",
                "Edit the instructions used to write meeting summaries.",
                row_action("Edit", "summary-template-edit", p)
                    .label("Edit Meeting summary instructions")
                    .on_press(|s: &mut State| {
                        s.hex.section = super::hex::Section::PromptLibrary;
                        super::scene_scroll(s, "agent-instructions");
                    }),
                false,
                p,
            ));
        let actions = row()
            .w_full()
            .flex_wrap()
            .gap(8.0)
            .padding(14.0)
            .child(
                super::widgets::button(
                    "What is sent",
                    "summary-privacy-link",
                    ButtonRole::Secondary,
                    p,
                )
                .on_press(|s: &mut State| {
                    s.hex.section = super::hex::Section::Privacy;
                    super::scene_scroll(s, "notes-privacy");
                }),
            )
            .children(summaries_on.then(|| {
                super::widgets::button(
                    "Turn off Meeting summary",
                    "notes-off",
                    ButtonRole::Secondary,
                    p,
                )
                .on_press(|s: &mut State| {
                    if let Some(mut settings) = s.ai_notes.settings.clone() {
                        settings.engine = "off".into();
                        s.notes_save_settings(settings, "Turning off Meeting summary...");
                    }
                })
            }));
        c = c.child(super::hex::settings::panel(p).child(summary).child(actions));
    }
    if part == Part::Privacy {
        let sent = match (agent, scripts, automatic) {
            (Some(_), _, true) => {
                "When a meeting ends or you request a summary, Sidenote sends the transcript and your typed notes to your selected agent. Audio stays on this PC."
            }
            (Some(_), _, false) => {
                "When you request a summary, Sidenote sends the transcript and your typed notes to your selected agent. Audio stays on this PC."
            }
            (_, true, _) => "Your configured script controls what is sent for meeting summaries.",
            _ => "Meeting summaries are off. Sidenote sends no meeting text for summaries.",
        };
        let mut privacy = super::hex::settings::setting_row(
            s,
            "notes-privacy",
            "What is sent",
            sent,
            row_action(
                if a.privacy_open {
                    "Hide details"
                } else {
                    "Details"
                },
                "notes-privacy-details",
                p,
            )
            .on_press(|s: &mut State| s.ai_notes.privacy_open = !s.ai_notes.privacy_open),
            false,
            p,
        );
        if a.privacy_open {
            privacy = col().w_full().child(privacy)
            .child(roles::SMALL.wrap("Nothing is sent until you turn on Meeting summary. After that, Sidenote sends a meeting when it ends, if automatic meeting summaries are on. Write summary and Try again also send that meeting. Send a sample sends only sample text.", p.soft))
            .child(roles::SMALL.wrap("Each request includes the meeting title, date, time, length, glossary, teams and members, projects, your name if set, and instructions. It does not include other meetings or audio.", p.soft))
            .child(roles::SMALL.wrap("Claude Code has no tools and no MCP access for these requests. It still loads your CLAUDE.md and hooks. Sidenote's logs do not contain meeting text or agent replies. Your file sync tool may copy saved meeting summaries and transcripts between your devices.", p.soft))
            .child(roles::SMALL.wrap("Claude Code and Codex can write summaries. Codex uses an ephemeral, read-only session. OpenCode is available for Ask only.", p.soft));
        }
        c = c.child(super::hex::settings::panel(p).child(privacy));
    }
    if matches!(part, Part::Helpers | Part::Advanced) {
        if part == Part::Helpers {
            c = c.child(
                row()
                    .w_full()
                    .flex_wrap()
                    .gap(10.0)
                    .child(
                        TextInput::new(&s.hex.shell.helper_query)
                            .placeholder("Search agents")
                            .label("Search agents")
                            .height(34.0)
                            .on_edit(|s: &mut State| &mut s.hex.shell.helper_query)
                            .build(k)
                            .key("helper-search")
                            .grow(1.0),
                    )
                    .child(
                        button(
                            if s.hex.shell.recommended {
                                "Recommended \u{2713}"
                            } else {
                                "Recommended"
                            },
                            "helper-recommended",
                            p,
                        )
                        .on_press(|s: &mut State| {
                            s.hex.shell.recommended = !s.hex.shell.recommended
                        }),
                    ),
            );
        }

        let mut helper_list = col().w_full();
        for (index, h) in HELPERS.iter().copied().enumerate() {
            let found = a.found.contains(&(h, true));
            let offered = h.offered();
            let selected = agent == Some(h);
            let mut r = card(p)
                .key(format!("notes-helper-{index}"))
                .child(roles::NAME.wrap(
                    format!(
                        "{}{}",
                        h.name(),
                        if index == 0 { " (recommended)" } else { "" }
                    ),
                    p.ink,
                ));
            if part == Part::Helpers {
                let query = s.hex.shell.helper_query.text.to_lowercase();
                if !h.name().to_lowercase().contains(query.trim())
                    || (s.hex.shell.recommended && index != 0)
                {
                    continue;
                }
                let help = if h == Helper::OpenCode {
                    "Answers Ask."
                } else {
                    "Writes your meeting summaries and answers Ask."
                };
                let title = row()
                    .items_center()
                    .flex_wrap()
                    .gap(8.0)
                    .child(cards::settings_title(h.name(), p))
                    .children((index == 0).then(|| roles::META.line("Recommended", p.ink)));
                let info = cards::settings_copy_with_title(title)
                    .child(roles::META.wrap(help, p.soft))
                    .child(roles::META.wrap(
                        if found {
                            "Detected on this PC. Sign-in has not been checked."
                        } else {
                            "Not on this PC."
                        },
                        p.soft,
                    ));
                let action = if !offered {
                    roles::CONTROL
                        .line("Ask only", p.soft)
                        .key(format!("notes-ask-only-{index}"))
                } else if selected {
                    roles::CONTROL
                        .line("In use", p.ok)
                        .key(format!("notes-use-{index}"))
                } else if !found {
                    row_action("Set up", "notes-install-page", p)
                        .on_press(|s: &mut State| s.open_notes_install_page())
                } else {
                    row_action(
                        format!("Use {}", h.name()),
                        &format!("notes-use-{index}"),
                        p,
                    )
                    .on_press(move |s: &mut State| s.choose_notes_helper(index))
                };
                r = cards::settings_row(true)
                    .key(format!("notes-helper-{index}"))
                    .fill(p.card)
                    .child(info)
                    .child(action);
                helper_list = helper_list.child(r);
                if a.switch_confirm == Some(h) {
                    c = c.child(
                        card(p)
                            .child(roles::SMALL.wrap(SWITCH_CONFIRM, p.warn))
                            .child(
                                row()
                                    .flex_wrap()
                                    .gap(8.0)
                                    .child(
                                        button(
                                            "Switch to built-in meeting summaries",
                                            "notes-confirm-switch",
                                            p,
                                        )
                                        .on_press(
                                            move |s: &mut State| s.choose_notes_helper(index),
                                        ),
                                    )
                                    .child(
                                        button("Keep scripts", "notes-cancel-switch", p).on_press(
                                            |s: &mut State| s.ai_notes.switch_confirm = None,
                                        ),
                                    ),
                            ),
                    );
                }
                continue;
            }
            if !offered {
                r = r.child(roles::SMALL.wrap("Coming in a later version", p.soft));
                c = c.child(r);
                continue;
            }
            r = r.child(roles::META.wrap(
                format!(
                    "{}{}",
                    if found { "Detected" } else { "Not detected" },
                    if selected { ". In use" } else { "" }
                ),
                if selected { p.ok } else { p.soft },
            ));
            r = r.child(roles::SMALL.wrap(
                if found {
                    "Uses your selected agent account. Sign in through its terminal command."
                } else {
                    SETUP_STEPS
                },
                p.soft,
            ));
            if !found {
                r = r.child(
                    row()
                        .flex_wrap()
                        .gaps(8.0, 8.0)
                        .child(
                            button("Open install page", "notes-install-page", p)
                                .on_press(|s: &mut State| s.open_notes_install_page()),
                        )
                        .child(
                            button("Check again", "notes-helper-check", p)
                                .on_press(|s: &mut State| s.refresh_ai_notes()),
                        ),
                );
                c = c.child(r);
                continue;
            }
            if needs_sign_in(found, a.test_results.get(&index).map(String::as_str)) {
                r = r.child(
                    button(
                        format!(
                            "Copy sign-in command: {}",
                            if h == Helper::Codex {
                                "codex"
                            } else {
                                "claude"
                            }
                        ),
                        "notes-copy-command",
                        p,
                    )
                    .self_start()
                    .on_press_tx(move |_: &mut State, tx| {
                        tx.copy(if h == Helper::Codex {
                            "codex"
                        } else {
                            "claude"
                        })
                    }),
                );
            }
            let mut actions = row().flex_wrap().gaps(8.0, 8.0);
            let test = button("Send a sample", &format!("notes-test-{index}"), p);
            actions = actions.child(test.disabled(a.pending.is_some()).on_press(
                move |s: &mut State| {
                    let model = s.ai_notes.models[index].text.clone();
                    s.notes_work(Work::Test(index, model), "Testing fixed sample text...");
                },
            ));
            r = r.child(actions);
            if a.testing == Some(index) && a.pending.is_some() {
                r = r.child(
                    roles::SMALL
                        .wrap("Testing fixed sample text...", p.soft)
                        .role("status"),
                );
            }
            if let Some(result) = a.test_results.get(&index) {
                r = r.child(if result.starts_with("Worked in ") {
                    roles::SMALL.wrap(result, p.ok).role("status")
                } else {
                    super::widgets::banner(
                        "Sample did not finish",
                        result,
                        super::widgets::BannerKind::Problem,
                        p,
                    )
                });
            }
            if a.switch_confirm == Some(h) {
                r = r.child(roles::SMALL.wrap(SWITCH_CONFIRM, p.warn)).child(
                    row()
                        .flex_wrap()
                        .gaps(8.0, 8.0)
                        .child(
                            button(
                                "Switch to built-in meeting summaries",
                                "notes-confirm-switch",
                                p,
                            )
                            .on_press(move |s: &mut State| s.choose_notes_helper(index)),
                        )
                        .child(
                            button("Keep scripts", "notes-cancel-switch", p)
                                .on_press(|s: &mut State| s.ai_notes.switch_confirm = None),
                        ),
                );
            }
            c = c.child(r);
        }
        if part == Part::Helpers {
            c = c.child(super::hex::settings::panel(p).child(helper_list));
        }
    }
    if part == Part::Scripts && settings.script.is_some() {
        c = c.child(
            card(p)
                .key("notes-scripts-card").fill(super::hex::settings::highlight(s,"notes-scripts-card",p))
                .child(roles::TITLE.wrap("Your own scripts (advanced)", p.ink))
                .child(roles::SMALL.wrap(
                    "Use your configured script. Built-in meeting summaries will stop starting new jobs.",
                    p.soft,
                ))
                .child(
                    button("Use scripts", "notes-scripts", p)
                        .self_start()
                        .on_press(|s: &mut State| {
                            if let Some(mut settings) = s.ai_notes.settings.clone() {
                                settings.engine = "scripts".into();
                                s.notes_save_settings(settings, "Switching to scripts...");
                            }
                        }),
                ),
        );
    }
    if part == Part::Summaries && !summaries_on {
        c = c.child(
            card(p)
                .child(roles::TITLE.wrap("Meeting summaries are off", p.ink))
                .child(roles::SMALL.wrap("Keep new meetings as transcripts only.", p.soft))
                .child(
                    button("Selected", "notes-off", p)
                        .color(p.ok)
                        .fill(p.sel)
                        .self_start()
                        .focusable(false),
                ),
        );
    }
    if part == Part::Identity {
        let control = row()
            .w(300.0)
            .max_w(300.0)
            .gap(8.0)
            .items_center()
            .child(
                field(&a.name, "Your name")
                    .on_edit(|s: &mut State| &mut s.ai_notes.name)
                    .build(k)
                    .key("notes-name")
                    .grow(1.0)
                    .min_w(80.0),
            )
            .child(
                button("Save name", "notes-save-name", p).on_press(|s: &mut State| {
                    if let Some(mut settings) = s.ai_notes.settings.clone() {
                        settings.your_name = s.ai_notes.name.text.trim().into();
                        s.notes_save_settings(settings, "Saving your name...");
                    }
                }),
            );
        c = c.child(super::hex::settings::panel(p).child(super::hex::settings::setting_row(s,"notes-identity","Your name","Used to mark tasks that are yours and included when a meeting is sent to your agent.",control,true,p)));
    }
    c
}
pub fn setup_visible(s: &State) -> bool {
    s.view == super::tasks::View::Meetings
        && s.selected.is_none()
        && (!s.on_hex() || s.hex.section == super::hex::Section::Connect)
        && !s.on_memory()
        && !s.on_check()
        && !s.on_live()
        && (s.on_hex() || s.filter == super::nav::Filter::All)
        && (s.on_hex() || s.search.text.trim().is_empty())
        && s.ai_notes.settings.as_ref().is_some_and(|settings| {
            summary_agent(settings).is_none() && !uses_summary_script(settings)
        })
}
pub fn setup_card(s: &State, p: Palette) -> Option<El> {
    if !setup_visible(s) {
        return None;
    }
    let a = &s.ai_notes;
    let helper = HELPERS
        .into_iter()
        .find(|h| h.offered() && a.found.contains(&(*h, true)));
    let setup = if helper.is_some() {
        roles::SMALL.wrap(
            "Choose which installed AI tool writes meeting summaries.",
            p.soft,
        )
    } else {
        col()
            .gap(4.0)
            .child(roles::SMALL.wrap("1. Install Claude Code.", p.soft))
            .child(roles::SMALL.wrap("2. In a terminal, run claude and sign in.", p.soft))
            .child(roles::SMALL.wrap("3. Press Check again.", p.soft))
    };
    let card = card(p).key("today-ai-notes").mb(16.0)
        .child(roles::TITLE.wrap(if helper.is_some() { "Summaries for your meetings" } else { "Meeting summaries need an agent" }, p.ink))
        .child(setup)
        .child(roles::SMALL.wrap("After you turn on Meeting summary, meeting text is sent through your selected agent account.", p.soft));
    let mut actions = row().flex_wrap().gaps(8.0, 8.0);
    if helper.is_none() {
        actions = actions
            .child(
                button("Open install page", "today-notes-install", p)
                    .on_press(|s: &mut State| s.open_notes_install_page()),
            )
            .child(
                button("Check again", "today-notes-check", p)
                    .on_press(|s: &mut State| s.refresh_ai_notes()),
            );
    }
    Some(
        card.child(
            actions
                .child(
                    button(
                        if helper.is_some() {
                            "Turn on Meeting summary"
                        } else {
                            "Show setup steps"
                        },
                        "today-notes-setup",
                        p,
                    )
                    .on_press(|s: &mut State| s.open_ai_notes()),
                )
                .child(button("What is sent", "today-notes-privacy", p).on_press(
                    |s: &mut State| {
                        s.open_hex();
                        s.hex.section = super::hex::Section::Privacy;
                        s.ai_notes.privacy_open = true;
                        super::scene_scroll(s, "notes-privacy");
                    },
                )),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_switch_and_privacy_report_the_same_saved_setting() {
        use rune::accesskit::{Role, Toggled};

        crate::test_paths::isolate();
        let mut rt = rune::Runtime::new(
            State::new(),
            |s, cx| {
                let p = super::super::theme::palette(cx.system_dark());
                let k = super::super::theme::kit(&p);
                col()
                    .child(section(s, &k, p, Part::Summaries))
                    .child(section(s, &k, p, Part::Privacy))
            },
            (800.0, 900.0),
            rune::Env::default(),
        );
        for (engine, consent, auto, checked, privacy) in [
            (
                "off",
                true,
                true,
                Toggled::False,
                "Meeting summaries are off.",
            ),
            (
                "app",
                false,
                true,
                Toggled::False,
                "Meeting summaries are off.",
            ),
            (
                "app",
                true,
                false,
                Toggled::False,
                "When you request a summary,",
            ),
            (
                "app",
                true,
                true,
                Toggled::True,
                "When a meeting ends or you request a summary,",
            ),
        ] {
            rt.update(|s, _| {
                s.ai_notes.settings = Some(NotesSettings {
                    engine: engine.into(),
                    helper: Some(Helper::Claude),
                    enabled_at_ms: consent.then_some(1),
                    auto,
                    ..NotesSettings::default()
                });
            });
            rt.frame(0.0);
            let tree = rt.accessibility_tree();
            let switch = tree
                .nodes
                .iter()
                .map(|(_, node)| node)
                .find(|node| {
                    node.role() == Role::Switch
                        && node.label() == Some("Write a meeting summary when a meeting ends")
                })
                .expect("automatic summaries switch");
            assert_eq!(switch.toggled(), Some(checked));
            assert_eq!(switch.is_disabled(), engine == "off" || !consent);

            let elements = rt.elements();
            assert!(elements.iter().any(|element| {
                element
                    .text
                    .as_deref()
                    .is_some_and(|text| text.starts_with(privacy))
            }));
            assert!(
                !elements
                    .iter()
                    .any(|element| element.text.as_deref() == Some("Off"))
            );
        }
    }

    #[test]
    fn recovery_click_then_save_completion_dispatches_exactly_one_codex_retry() {
        let id = "123-1".to_string();
        let mut a = AiNotes {
            settings: Some(NotesSettings {
                helper: Some(Helper::Claude),
                enabled_at_ms: Some(1),
                ..NotesSettings::default()
            }),
            found: vec![(Helper::Codex, true)],
            ..AiNotes::default()
        };
        a.jobs.insert(
            id.clone(),
            super::super::notes_status::Job {
                state: notes::JobState::HelperLimit { exit: Some(1) },
                helper: Some(Helper::Claude),
                updated_at: 0,
                request: Some(Request::FirstPart),
                transcript_revision: None,
                routing_fallbacks: Vec::new(),
                translation: None,
            },
        );
        assert!(a.saved_recovery_request().is_none()); // A limit alone cannot schedule recovery.
        let saved = a.prepare_codex_recovery(id.clone()).unwrap();
        assert_eq!(saved.helper, Some(Helper::Codex));
        assert_eq!(a.retry_after_save, Some((id.clone(), Request::FirstPart)));
        a.settings = Some(saved); // The actual save completion installs the re-read config first.
        assert!(request_problem(a.settings.as_ref()).is_none());
        let mut enqueues = Vec::new();
        enqueues.extend(a.saved_recovery_request());
        enqueues.extend(a.saved_recovery_request());
        assert_eq!(enqueues, [(id, Request::UseCodex)]);
    }
    // Both tests write to the same isolated config.
    static MODEL_SAVES: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn other_model_editor(draft: &str) -> (rune::Runtime<State>, usize) {
        crate::test_paths::isolate();
        let mut s = State::new();
        let index = HELPERS.iter().position(|h| *h == Helper::Claude).unwrap();
        s.ai_notes.found = vec![(Helper::Claude, true)];
        let settings = NotesSettings {
            engine: "app".into(),
            helper: Some(Helper::Claude),
            model: "synthetic-saved".into(),
            ..NotesSettings::default()
        };
        notes::save_settings(s.config.notes.as_ref().unwrap(), &settings).unwrap();
        s.ai_notes.settings = Some(notes::settings().unwrap());
        s.ai_notes.models[index] = TextEdit::new(draft);
        s.ai_notes.other_model = true;
        let mut rt = rune::Runtime::new(
            s,
            |s, cx| {
                let p = super::super::theme::palette(cx.system_dark());
                let k = super::super::theme::kit(&p);
                summary_model(s, &k, p, &mut Vec::new(), Vec::new())
            },
            (1334.0, 900.0),
            rune::Env::default(),
        );
        rt.frame(0.0);
        (rt, index)
    }

    fn finish_model_save(s: &mut State) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while s.ai_notes.pending.is_some() {
            let event = s
                .events_rx
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("summary settings save completed");
            s.apply_event(event);
        }
    }

    #[test]
    fn a_failed_model_save_keeps_what_was_typed_and_shows_the_error() {
        let _guard = MODEL_SAVES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let invalid = "synthetic-\tmodel";
        let failure = "Could not save Meeting summary settings. Your previous choice was kept.";
        let (mut rt, index) = other_model_editor(invalid);
        let save = |s: &mut State| {
            s.choose_notes_helper(index);
            assert!(s.ai_notes.other_model);
            finish_model_save(s);
        };
        rt.update(|s, _| save(s));
        rt.frame(0.0);
        let a = &rt.state.ai_notes;
        assert_eq!(a.models[index].text, invalid);
        assert!(a.other_model && a.pending.is_none());
        assert_eq!(a.error.as_deref(), Some(failure));
        assert!(rt.find("notes-model-other").is_some());
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some(failure))
        );
        rt.update(|s, _| {
            s.ai_notes.models[index] = TextEdit::new("synthetic-typed");
            save(s);
        });
        rt.frame(0.0);
        let a = &rt.state.ai_notes;
        assert!(!a.other_model && a.pending.is_none());
        assert!(a.error.is_none());
        assert_eq!(a.settings.as_ref().unwrap().model, "synthetic-typed");
        assert!(rt.find("notes-model-other").is_none());
    }

    #[test]
    fn a_model_save_keeps_edits_typed_before_completion() {
        let _guard = MODEL_SAVES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut rt, index) = other_model_editor("");
        let click = |rt: &mut rune::Runtime<State>, key, t| {
            let (x, y) = rt.find(key).unwrap();
            rt.pointer_move(x, y);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(t);
        };
        let type_text = |rt: &mut rune::Runtime<State>, text: &str, t| {
            rt.key(rune::KeyEvent {
                key: rune::Key::Text(text.into()),
                mods: rune::Mods::default(),
            });
            rt.frame(t);
        };
        click(&mut rt, "notes-model-other", 0.1);
        type_text(&mut rt, "synthetic-a", 0.2);
        click(&mut rt, "notes-save-model", 0.3);
        assert_eq!(rt.state.ai_notes.pending, Some(SAVING_HELPER));
        click(&mut rt, "notes-model-other", 0.4);
        rt.key(rune::KeyEvent {
            key: rune::Key::End,
            mods: rune::Mods::default(),
        });
        type_text(&mut rt, "-newer", 0.5);
        assert_eq!(rt.state.ai_notes.models[index].text, "synthetic-a-newer");

        rt.update(|s, _| finish_model_save(s));
        rt.frame(0.6);
        let a = &rt.state.ai_notes;
        assert!(a.other_model && a.pending.is_none());
        assert_eq!(a.models[index].text, "synthetic-a-newer");
        assert_eq!(a.settings.as_ref().unwrap().model, "synthetic-a");
        assert!(a.error.is_none() && a.notice.is_none());
        assert!(rt.state.hex.shell.saved.is_none());
        assert!(rt.find("notes-model-other").is_some());
    }
    #[test]
    fn sign_in_is_only_offered_for_a_detected_helper_without_a_known_reply() {
        assert!(!needs_sign_in(false, None));
        assert!(needs_sign_in(true, None));
        assert!(needs_sign_in(true, Some(&test_error("sign-in failed"))));
        assert!(!needs_sign_in(
            true,
            Some("Worked in 8 s. Only fixed sample text was sent.")
        ));
        assert!(!needs_sign_in(true, Some(&test_error("invalid JSON"))));
    }
    #[test]
    fn instruction_fallback_reaches_existing_banner_without_blocking_settings() {
        assert!(instructions_warning(&instructions::Store::default()).is_none());
        let mut store = instructions::Store::default();
        store.using_defaults_after_error = true;
        assert_eq!(
            instructions_warning(&store).as_deref(),
            Some(INSTRUCTIONS_LOAD_ERROR)
        );
    }
    #[test]
    fn summary_and_rebuild_requests_require_an_offered_helper_and_consent() {
        assert!(request_problem(None).is_some());
        let mut settings = NotesSettings::default();
        assert!(request_problem(Some(&settings)).is_some());
        settings.helper = Some(Helper::Claude);
        assert!(request_problem(Some(&settings)).is_some());
        settings.enabled_at_ms = Some(1);
        assert!(request_problem(Some(&settings)).is_none());
        settings.helper = Some(Helper::Codex);
        assert!(request_problem(Some(&settings)).is_none());
        for engine in ["off", "scripts"] {
            settings.engine = engine.into();
            assert!(request_problem(Some(&settings)).is_some());
        }
        settings.engine = "app".into();
        {
            let helper = Helper::OpenCode;
            settings.helper = Some(helper);
            assert!(
                request_problem(Some(&settings))
                    .unwrap()
                    .contains(helper.name())
            );
        }
        settings.helper = None;
        assert!(request_problem(Some(&settings)).is_some());
    }
    #[test]
    fn only_opencode_sample_calls_are_refused() {
        assert!(Helper::Claude.offered());
        {
            let (index, helper) = (2, Helper::OpenCode);
            assert!(!helper.offered());
            let error = run(
                Path::new("unused-policy-test"),
                Work::Test(index, String::new()),
            )
            .err()
            .unwrap();
            assert!(error.contains(helper.name()));
            assert!(error.contains("coming in a later version"));
            assert!(error.contains("Choose Claude Code"));
        }
    }
    #[test]
    fn unreadable_test_answers_use_plain_words() {
        assert!(!test_error("The agent test returned invalid JSON").contains("JSON"));
        assert_eq!(
            test_error("The agent was not found on this PC"),
            "The sample failed. Claude Code did not answer. Open a terminal, type claude, sign in, then press Send a sample."
        );
    }

    #[test]
    fn instruction_limit_counts_unicode_and_refuses_nul() {
        assert!(valid_instructions(&"\u{e9}".repeat(16_000)));
        assert!(!valid_instructions(&"\u{e9}".repeat(16_001)));
        assert!(!valid_instructions("keep this\0"));
        assert_eq!(count_text(16_000), "16,000");
    }
    #[test]
    fn a_refresh_keeps_the_unsaved_draft_and_reset_replaces_it() {
        let mut editor = Editor::new(Agent::Notes);
        editor.text = TextEdit::new("Unsaved local text");
        editor.loaded(
            Agent::Notes,
            Some(instructions::Entry {
                text: "Synced text".into(),
                based_on: 1,
                edited_at: "2026-11-10T04:00:00Z".into(),
            }),
            false,
        );
        assert_eq!(editor.text.text, "Unsaved local text");
        assert_eq!(editor.saved, "Synced text");
        assert!(editor.dirty());
        editor.loaded(Agent::Notes, None, true);
        assert_eq!(editor.text.text, instructions::default_base(Agent::Notes));
        assert!(!editor.dirty());
        assert!(editor.edited_at.is_none());
    }
    #[test]
    fn successful_save_marks_only_the_saved_draft_clean() {
        let mut editor = Editor::new(Agent::Tasks);
        editor.text = TextEdit::new("First draft");
        let entry = instructions::Entry {
            text: "First draft".into(),
            based_on: 1,
            edited_at: "2026-11-10T04:00:00Z".into(),
        };
        editor.stored(Agent::Tasks, Some(entry.clone()), false);
        assert!(!editor.dirty());
        assert_eq!(editor.base.as_deref(), Some("First draft"));
        editor.text = TextEdit::new("Newer draft typed while saving");
        editor.stored(Agent::Tasks, Some(entry), false);
        assert_eq!(editor.text.text, "Newer draft typed while saving");
        assert!(editor.dirty());
        // Our own save never reads as another window's change.
        assert!(editor.conflict.is_none());
    }
    #[test]
    fn a_refresh_that_finds_a_newer_copy_under_unsaved_text_flags_a_conflict() {
        let mut editor = Editor::new(Agent::Ask);
        editor.text = TextEdit::new("My unsaved wording");
        let theirs = instructions::Entry {
            text: "Their saved wording".into(),
            based_on: 1,
            edited_at: "2026-11-10T04:00:00Z".into(),
        };
        editor.loaded(Agent::Ask, Some(theirs.clone()), false);
        let conflict = editor.conflict.as_ref().expect("a conflict");
        assert_eq!(conflict.theirs.as_ref(), Some(&theirs));
        assert_eq!(conflict.retry, Retry::Save);
        // Your text and your starting point are untouched, so Save stays measured against it.
        assert_eq!(editor.text.text, "My unsaved wording");
        assert_eq!(editor.base, None);
        // A clean editor just takes the new copy, with no conflict.
        let mut clean = Editor::new(Agent::Ask);
        clean.loaded(Agent::Ask, Some(theirs), false);
        assert!(clean.conflict.is_none());
        assert_eq!(clean.base.as_deref(), Some("Their saved wording"));
    }
    #[test]
    fn instruction_worker_saves_and_resets_each_agent_without_a_helper() {
        let root = std::env::temp_dir().join(format!(
            "notes-ui-worker-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let ui_thread = thread::current().id();
        let worker_root = root.clone();
        thread::spawn(move || {
            assert_ne!(ui_thread, thread::current().id());
            for index in 0..Agent::ALL.len() {
                let result = run(
                    &worker_root,
                    Work::Save(index, format!("Custom agent {index}"), None),
                )
                .unwrap();
                assert!(
                    matches!(result, Completion::Instructions(i, Some(_), false) if i == index)
                );
            }
            let store = instructions::load(&worker_root).unwrap();
            for agent in Agent::ALL {
                assert!(store.get(agent).is_some());
            }
            assert!(matches!(
                run(&worker_root, Work::Reset(0, Some("Custom agent 0".into()))).unwrap(),
                Completion::Instructions(0, None, true)
            ));
            let store = instructions::load(&worker_root).unwrap();
            assert!(store.notes.is_none() && store.tasks.is_some() && store.ask.is_some());
            // Saving the default text is a reset: your copy goes and the editor shows the default.
            let default = instructions::default_base(Agent::Tasks).to_owned();
            assert!(matches!(
                run(
                    &worker_root,
                    Work::Save(1, default, Some("Custom agent 1".into()))
                )
                .unwrap(),
                Completion::Instructions(1, None, true)
            ));
            assert!(run(&worker_root, Work::Save(1, "x".repeat(16_001), None)).is_err());
            // A window that still starts from the old copy gets a conflict and writes nothing.
            let stale = run(
                &worker_root,
                Work::Save(2, "Stale window text".into(), Some("Older copy".into())),
            )
            .unwrap();
            assert!(matches!(
                stale,
                Completion::Conflict(2, Some(ref theirs), Retry::Save)
                    if theirs.text == "Custom agent 2"
            ));
            let stale_reset = run(&worker_root, Work::Reset(2, Some("Older copy".into()))).unwrap();
            assert!(matches!(
                stale_reset,
                Completion::Conflict(2, Some(_), Retry::Reset)
            ));
            let store = instructions::load(&worker_root).unwrap();
            assert_eq!(store.get(Agent::Title).unwrap().text, "Custom agent 2");
        })
        .join()
        .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
