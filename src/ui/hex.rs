//! The Hex engine in the Sidenote window: the status the sidebar and the top right show, and a
//! Hex page with the model, the vocabulary and the settings in sections. Starting and stopping a
//! recording is the New meeting flow in `nav` and `live`.
//!
//! No call here runs on the UI thread. Status comes from the engine's `StatusWatcher`, which a
//! forwarder thread owns. Every other call runs on a short thread and comes back as a `Msg`.
use super::{
    Event, State, roles,
    theme::Palette,
    widgets::{badge, hairline, pressable, scroll},
};
mod connection;
mod meeting_languages;
mod readable;
pub(super) mod settings;
mod setup;
#[path = "hex/connection/stop.rs"]
pub(super) mod stop;
use rune::{Color, El, Key, KeyEvent, TextEdit, Tx, Waker, col, row};
use rune_kit::{Kit, Select, SelectAppearance, Switch, TextInput};
use serde_json::Value;
pub use settings::{
    nav_docked, nav_open, sidebar as settings_sidebar, tick as settings_tick, toggle_nav,
};
use sidenote::hex_engine::{
    AudioRetention, Capabilities, EngineState, Feature, HexEngine, HexError, ModelInfo, ModelState,
    Recording, Setting, StatusSample, Support, VocabularyEntry, sample_status,
};
use std::{
    collections::{HashMap, HashSet},
    sync::mpsc::{self, RecvTimeoutError, Sender, TryRecvError},
    thread,
    time::{Duration, Instant, SystemTime},
};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};

/// How long after a failed capability lookup the next status sample may try again.
const CAPS_RETRY: Duration = Duration::from_secs(30);
/// How often the forwarder looks for a visibility change while the window is visible.
const FORWARD_TICK: Duration = Duration::from_millis(500);

// ---- window visibility ----------------------------------------------------------------------

#[cfg(windows)]
static WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Called once with the window's handle, so the indicator can tell shown from minimized.
#[cfg(windows)]
pub fn remember_window(hwnd: isize) {
    use std::sync::atomic::Ordering;
    WINDOW.store(hwnd, Ordering::Relaxed);
    sidenote::listener::status::remember_window(hwnd);
}

/// On screen and not minimized. Before the handle is known the window counts as shown.
#[cfg(windows)]
pub fn window_shown() -> bool {
    use std::sync::atomic::Ordering;
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd == 0 {
        return true;
    }
    let handle = hwnd as *mut std::ffi::c_void;
    // SAFETY: the handle belongs to this process's window; both calls only read its state.
    unsafe { IsWindowVisible(handle) != 0 && IsIconic(handle) == 0 }
}

#[cfg(not(windows))]
pub fn window_shown() -> bool {
    super::window::shown()
}

// ---- messages from worker threads -----------------------------------------------------------

pub enum Action {
    Start,
    Stop(u64, Option<Box<StatusSample>>),
    Set(String),
    Add,
    Remove,
    Model,
}

pub enum Msg {
    NotesRequested(Result<(), HexError>),
    Meetings(Result<(Instant, EngineState), HexError>),
    Install(sidenote::hex_install::State),
    ObservedStatus(StatusSample),
    RecoveryStatus(StatusSample),
    WatcherStopped,
    StopStatus(Instant, StatusSample),
    RecheckStart(StatusSample),
    RecheckResume(StatusSample),
    SettingsStatus(StatusSample),
    Caps(String, Result<Capabilities, HexError>),
    Data {
        settings: Result<Vec<Setting>, HexError>,
        vocabulary: Result<Vec<VocabularyEntry>, HexError>,
        models: Result<Vec<ModelInfo>, HexError>,
    },
    Settings(Result<Vec<Setting>, HexError>),
    AudioSettings(Result<(), HexError>, Result<Vec<Setting>, HexError>),
    Labs(
        Result<sidenote::hex_labs::Snapshot, HexError>,
        Result<(), HexError>,
    ),
    Vocabulary(Result<Vec<VocabularyEntry>, HexError>),
    Done(Action, Result<(), HexError>),
}

fn plain(e: &HexError) -> &'static str {
    match e {
        HexError::HexNotRunning => "Hex is not running.",
        HexError::Unsupported => "This Hex build does not support that.",
        HexError::Timeout => "Hex did not answer in time.",
        HexError::Failed { code, .. } => match code.as_deref() {
            Some("unknown-setting") => "Hex does not know that setting.",
            Some("read-only") => "That setting is read only.",
            Some("invalid-value") => "Hex rejected that value.",
            Some("shortcut-in-use") => "That shortcut is in use.",
            Some("feature-off") => "That feature is off in Hex.",
            Some("not-ready") => "Hex is not ready yet.",
            Some("busy") => "Hex is busy.",
            Some("recording-changed") => {
                "Hex is recording a different meeting. Stop it from Hex if you meant that one."
            }
            Some("stop-unconfirmed") => {
                "Could not confirm this recording. Stop was not confirmed. Stop it from Hex."
            }
            Some("not-recording") => "Hex says no meeting is recording.",
            _ => "Hex could not do that.",
        },
    }
}

// ---- state ----------------------------------------------------------------------------------

pub struct HexState {
    pub(super) parts_restore_checked: bool,
    meetings_setup_checked: bool,
    installation: sidenote::hex_install::Installation,
    install: sidenote::hex_install::State,
    engine: HexEngine,
    /// Tells the forwarder thread when the window is shown or hidden.
    visibility: Option<Sender<bool>>,
    visible: bool,
    /// Latest result, separate from the bounded last-known-good snapshot.
    pub status: Option<Result<EngineState, HexError>>,
    last_status_request: Option<Instant>,
    pub(super) connection: connection::Connection,
    was_running: bool,
    settings_status_pending: bool,
    settings_status_at: Option<Instant>,
    caps: Option<Capabilities>,
    caps_version: Option<String>,
    caps_loading: bool,
    caps_retry_at: Option<Instant>,
    pub settings: Vec<Setting>,
    pub vocabulary: Vec<VocabularyEntry>,
    pub models: Vec<ModelInfo>,
    settings_note: Option<&'static str>,
    vocabulary_note: Option<&'static str>,
    models_note: Option<&'static str>,
    data_dirty: bool,
    data_loading: bool,
    /// The note under the top bar, on every page: why Record or Stop failed.
    pub error: Option<&'static str>,
    row_errors: HashMap<String, String>,
    vocab_error: Option<&'static str>,
    record_busy: bool,
    stop: Option<stop::Completion>,
    notes_stop_recording: Option<u64>,
    vocab_busy: bool,
    model_busy: bool,
    busy_keys: HashSet<String>,
    /// Keys whose draft takes the value Hex reports next, because the write just went through.
    fresh: HashSet<String>,
    drafts: HashMap<String, TextEdit>,
    pub filter: TextEdit,
    pub term: TextEdit,
    pub sounds: TextEdit,
    select_open: Option<String>,
    /// The section the page shows, and which Advanced groups the owner opened.
    pub section: Section,
    pub shell: settings::Settings,
    open_groups: HashSet<&'static str>,
}

#[cfg(test)]
thread_local! { pub(super) static SKIP_INSTALL_DETECT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

impl Default for HexState {
    fn default() -> Self {
        Self {
            parts_restore_checked: false,
            meetings_setup_checked: false,
            #[cfg(not(test))]
            installation: sidenote::hex_install::detect(),
            #[cfg(test)]
            installation: if SKIP_INSTALL_DETECT.get() {
                sidenote::hex_install::Installation::Incomplete
            } else {
                sidenote::hex_install::detect()
            },
            install: sidenote::hex_install::State::default(),
            engine: HexEngine::new(),
            visibility: None,
            visible: false,
            status: None,
            last_status_request: None,
            connection: connection::Connection::default(),
            was_running: false,
            settings_status_pending: false,
            settings_status_at: None,
            caps: None,
            caps_version: None,
            caps_loading: false,
            caps_retry_at: None,
            settings: Vec::new(),
            vocabulary: Vec::new(),
            models: Vec::new(),
            settings_note: None,
            vocabulary_note: None,
            models_note: None,
            data_dirty: true,
            data_loading: false,
            error: None,
            row_errors: HashMap::new(),
            vocab_error: None,
            record_busy: false,
            stop: None,
            notes_stop_recording: None,
            vocab_busy: false,
            model_busy: false,
            busy_keys: HashSet::new(),
            fresh: HashSet::new(),
            drafts: HashMap::new(),
            filter: TextEdit::default(),
            term: TextEdit::default(),
            sounds: TextEdit::default(),
            select_open: None,
            section: Section::Identity,
            shell: settings::Settings::default(),
            open_groups: HashSet::new(),
        }
    }
}

impl HexState {
    pub fn language_picker_open(&self) -> bool {
        self.open_groups.contains("languages")
    }
    pub fn open_language_picker(&mut self) -> bool {
        self.open_groups.insert("languages")
    }
    pub fn close_language_picker(&mut self) {
        self.open_groups.remove("languages");
    }

    pub fn typing(&self) -> bool {
        self.filter.focused
            || self.term.focused
            || self.sounds.focused
            || self.drafts.values().any(|d| d.focused)
    }

    fn state(&self) -> Option<&EngineState> {
        if self.connection.expired() {
            return None;
        }
        self.status
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .or_else(|| self.connection.state())
    }

    fn running(&self) -> bool {
        self.state().is_some()
    }

    /// Hex answered that it is not running. A missing, slow or expired answer is not this.
    fn stopped(&self) -> bool {
        matches!(self.status, Some(Err(HexError::HexNotRunning)))
    }

    // A Stop target is an observation, not a claim that an expired recording is still live.
    fn recording(&self) -> Option<&Recording> {
        self.status
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|state| state.recording.as_ref())
            .or_else(|| self.connection.recording())
    }

    /// Hex is up and this build has the command.
    fn can(&self, f: Feature) -> bool {
        self.running() && self.caps.as_ref().and_then(|c| c.get(&f)) == Some(&Support::Supported)
    }

    /// Hex is up and the build says it lacks the command.
    fn lacks(&self, f: Feature) -> bool {
        self.running() && self.caps.as_ref().and_then(|c| c.get(&f)) == Some(&Support::Unsupported)
    }

    /// Opens every Advanced group, for a render that shows what is inside.
    pub fn open_all_groups(&mut self) {
        self.open_groups.extend(["labs", "rare"]);
    }

    pub fn draft_mut(&mut self, key: &str) -> &mut TextEdit {
        self.drafts.entry(key.to_string()).or_default()
    }

    fn setting(&self, key: &str) -> Option<&Setting> {
        self.settings.iter().find(|s| s.key == key)
    }

    /// Why nothing on the page can be read or changed right now.
    fn page_note(&self) -> Option<&'static str> {
        if self.connection.expired() {
            return Some(connection::UNKNOWN);
        }
        match &self.status {
            None => Some("Checking Hex."),
            Some(Err(HexError::HexNotRunning)) => {
                Some("Hex is not running. Start it to read or change these.")
            }
            Some(Err(HexError::Timeout | HexError::Failed { .. })) => Some(connection::SLOW),
            Some(Err(HexError::Unsupported)) => Some("This version of Hex is not supported."),
            Some(Ok(_)) => None,
        }
    }
}

fn setting_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn editable(st: &Setting) -> bool {
    st.writable
        && st.available != Some(false)
        && !st.secret
        && (matches!(
            st.kind.as_str(),
            "bool" | "choice" | "number" | "integer" | "text" | "path"
        ) || (st.key == "meetings.providers" && st.kind == "json" && st.value.is_array()))
}

fn is_numeric(kind: &str) -> bool {
    matches!(kind, "number" | "integer")
}

/// True when `new` would change the setting. Numbers compare as numbers, so `1` equals `1.0`.
fn changed(st: &Setting, new: &str) -> bool {
    match st.kind.as_str() {
        "bool" => st.value.as_bool().map(|b| b.to_string()).as_deref() != Some(new),
        k if is_numeric(k) => match (st.value.as_f64(), new.parse::<f64>()) {
            (Some(a), Ok(b)) => a != b,
            _ => setting_text(&st.value) != new,
        },
        _ => setting_text(&st.value) != new,
    }
}

fn pretty(key: &str) -> String {
    // Words that read wrong in lower case ("api key") keep their capitals.
    let words = |s: &str| {
        s.split('_')
            .map(|w| match w {
                "api" | "url" | "id" | "ui" | "gpu" | "cpu" => w.to_uppercase(),
                _ => w.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let text = match key.split_once('.') {
        Some((head, rest)) => format!("{}: {}", words(head), words(rest)),
        None => words(key),
    };
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => text,
    }
}

fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn elapsed(r: &Recording) -> Option<u64> {
    r.since
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| d.as_secs())
        .or(r.elapsed_seconds)
}

// ---- actions --------------------------------------------------------------------------------

impl State {
    pub fn on_hex(&self) -> bool {
        self.hex_open && self.view.is_base() && self.selected.is_none()
    }

    pub fn open_hex(&mut self) {
        let timing = if !self.on_hex() {
            super::Operation::new(super::TimedAction::Settings)
        } else {
            super::Operation::default()
        };
        if !self.on_hex() {
            settings::remember_back(self);
            self.hex.shell.last_tabs =
                sidenote::core::UiPrefs::load(&self.store.root).settings_tabs;
        }
        self.go_home();
        self.hex_open = true;
        self.hex.data_dirty = true;
        self.hex.select_open = None;
        timing.ready();
        if !super::isolated() {
            self.hex_check_settings();
        }
        self.hex_refresh_if_due();
        self.refresh_ai_notes();
    }

    /// Runs `work` on its own thread and delivers the result to the UI.
    fn hex_spawn(&self, work: impl FnOnce(HexEngine) -> Msg + Send + 'static) -> bool {
        let engine = self.hex.engine.clone();
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        thread::Builder::new()
            .name("hex-call".into())
            .spawn(move || {
                let _ = events.send(Event::Hex(work(engine)));
                if let Some(w) = &waker {
                    w();
                }
            })
            .is_ok()
    }

    /// Starts the status watcher. Its forwarder thread owns the watcher and reads its updates.
    pub fn hex_start(&mut self, waker: Waker) {
        self.hex.visible = window_shown();
        let watcher = match self.hex.engine.watch() {
            Ok(w) => w,
            Err(e) => {
                if self.hex.status.is_none() {
                    self.hex_observed_event(sample_status(|| Err(e)));
                }
                return;
            }
        };
        let (commands, input) = mpsc::channel::<bool>();
        let events = self.events_tx.clone();
        let started = thread::Builder::new()
            .name("hex-ui-status".into())
            .spawn(move || {
                let mut visible = false;
                loop {
                    // Hidden, the thread waits for a command and does nothing else.
                    let mut want = visible;
                    if visible {
                        match watcher.updates.recv_timeout(FORWARD_TICK) {
                            Ok(update) => {
                                if events
                                    .send(Event::Hex(Msg::ObservedStatus(update)))
                                    .is_err()
                                {
                                    return;
                                }
                                waker();
                            }
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                        loop {
                            match input.try_recv() {
                                Ok(v) => want = v,
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => return,
                            }
                        }
                    } else {
                        match input.recv() {
                            Ok(v) => want = v,
                            Err(_) => return,
                        }
                    }
                    if want != visible {
                        if want {
                            // A sample left over from before the window hid would block the fresh one.
                            while watcher.updates.try_recv().is_ok() {}
                        }
                        watcher.set_visible(want);
                        visible = want;
                    }
                }
                let _ = events.send(Event::Hex(Msg::WatcherStopped));
                waker();
            });
        if started.is_ok() {
            let _ = commands.send(self.hex.visible);
            self.hex.visibility = Some(commands);
        }
    }

    /// Tells the watcher the window is shown or hidden. It makes no Hex calls while hidden.
    pub fn hex_visibility(&mut self, visible: bool) {
        if self.hex.visible == visible {
            return;
        }
        self.hex.visible = visible;
        if let Some(tx) = &self.hex.visibility {
            let _ = tx.send(visible);
        }
    }

    pub fn hex_apply(&mut self, msg: Msg) {
        match msg {
            Msg::Meetings(result) => {
                self.hex.busy_keys.remove("meetings.enabled");
                match result {
                    Ok((sent, state)) => {
                        self.hex.row_errors.remove("meetings.enabled");
                        self.hex.data_dirty = true;
                        self.hex_observed_event((sent, Ok(state)));
                    }
                    Err(_) => {
                        self.hex.row_errors.insert("meetings.enabled".into(),
                            "Could not confirm that Meetings is on. Open Hex > Settings > Labs > Meetings, then check again.".into());
                    }
                }
            }
            Msg::Install(state) => {
                if !state.busy() {
                    self.hex.installation = sidenote::hex_install::detect();
                    self.hex_refresh_if_due();
                }
                self.hex.install = state;
            }
            Msg::ObservedStatus(sample) => {
                self.hex_observed_event(sample);
            }
            Msg::RecoveryStatus(sample) => {
                self.hex.connection.finish_recovery(Instant::now());
                if !matches!(&sample.1, Err(HexError::Timeout | HexError::Failed { .. })) {
                    self.hex_observed_event(sample);
                }
            }
            Msg::WatcherStopped => self.hex.visibility = None,
            Msg::StopStatus(deadline, r) => self.hex_stop_status(deadline, r),
            Msg::SettingsStatus(result) => {
                self.hex.settings_status_pending = false;
                self.hex.settings_status_at = Some(Instant::now());
                self.hex_observed_event(result);
            }
            Msg::NotesRequested(result) => {
                if result.is_err() {
                    self.parts_notes_failed();
                    self.hex.error = Some(
                        "Notes could not start. Check the notes engine and try Generate notes again.",
                    );
                }
            }
            Msg::RecheckResume((sent, result)) => {
                self.hex.record_busy = false;
                match result {
                    Ok(status) => {
                        self.hex_observed_event((sent, Ok(status)));
                        if self.hex_recording() {
                            self.hex.error =
                                Some("Hex is already recording. No new part was started.");
                        } else {
                            self.meeting_resume();
                        }
                    }
                    Err(e) => {
                        self.hex.error = Some(plain(&e));
                        self.hex_observed_event((sent, Err(e)));
                    }
                }
            }
            Msg::RecheckStart((sent, result)) => {
                self.hex.connection.action_stage("recheck_done");
                self.hex.record_busy = false;
                match result {
                    Ok(status) => {
                        self.hex_observed_event((sent, Ok(status)));
                        if self.hex_recording() {
                            self.hex.error =
                                Some("Hex is already recording. No new meeting was started.");
                        } else if let Some(reason) = start_blocked(self) {
                            self.hex.error = Some(reason);
                        } else {
                            self.new_meeting_continue();
                        }
                    }
                    Err(error) => {
                        self.hex_observed_event((sent, Err(error)));
                        self.hex.error =
                            Some("Could not check Hex. New meeting was not started. Try again.");
                    }
                }
                if !self.hex.record_busy {
                    self.hex.connection.finish_action();
                }
            }
            Msg::Caps(version, r) => {
                self.hex.caps_loading = false;
                match r {
                    Ok(caps) => {
                        self.hex.caps = Some(caps);
                        self.hex.caps_version = Some(version);
                    }
                    Err(_) => self.hex.caps_retry_at = Some(Instant::now() + CAPS_RETRY),
                }
            }
            Msg::Data {
                settings,
                vocabulary,
                models,
            } => {
                self.hex.data_loading = false;
                self.hex_set_settings(settings);
                self.hex_set_vocabulary(vocabulary);
                match models {
                    Ok(m) => {
                        self.hex.models = m;
                        self.hex.models_note = None;
                    }
                    Err(e) => self.hex.models_note = Some(plain(&e)),
                }
            }
            Msg::Labs(snapshot, result) => {
                self.hex.shell.labs_loading = false;
                self.hex.shell.lab_error = result.err().map(|e| match &e {
                    HexError::Failed { stderr_summary, .. } => {
                        format!("{} {stderr_summary}", plain(&e))
                    }
                    _ => plain(&e).to_owned(),
                });
                match snapshot {
                    Ok(snapshot) => self.hex.shell.labs = snapshot,
                    Err(e) => {
                        self.hex.shell.labs = Default::default();
                        let previous = self.hex.shell.lab_error.take().unwrap_or_default();
                        self.hex.shell.lab_error =
                            Some(format!("{previous} Labs state unavailable. {}", plain(&e)));
                    }
                }
            }
            Msg::Settings(r) => self.hex_set_settings(r),
            Msg::AudioSettings(result, settings) => {
                self.hex.busy_keys.remove("meetings.audio_retention");
                self.hex_set_settings(settings);
                if result.is_ok() {
                    self.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
                }
                if let Err(e) = result {
                    self.hex
                        .row_errors
                        .insert("meetings.audio_retention".into(), plain(&e).into());
                }
            }
            Msg::Vocabulary(r) => self.hex_set_vocabulary(r),
            Msg::Done(action, r) => self.hex_done(action, r),
        }
    }

    fn hex_accept_status(&mut self, (sent, result): &StatusSample) -> bool {
        if self
            .hex
            .last_status_request
            .is_some_and(|last| *sent < last)
        {
            return false;
        }
        // A transport failure has no observation to supersede a slow, successful reply.
        if !matches!(result, Err(HexError::Timeout | HexError::Failed { .. })) {
            self.hex.last_status_request = Some(*sent);
        }
        true
    }

    pub(super) fn hex_observed_event(&mut self, sample: StatusSample) -> bool {
        if !self.hex_accept_status(&sample) {
            return false;
        }
        let (sent, r) = sample;
        let changed = self.hex.status.as_ref() != Some(&r) || self.live.is_some();
        let before = (
            self.hex.installation.clone(),
            self.hex.caps.clone(),
            self.hex.caps_loading,
            self.hex.data_loading,
            self.ask_settings.recording,
        );
        // Keep capability retries, install detection and visibility handling on every sample.
        self.hex_status_at(r, sent);
        changed
            || self.live.is_some()
            || before
                != (
                    self.hex.installation.clone(),
                    self.hex.caps.clone(),
                    self.hex.caps_loading,
                    self.hex.data_loading,
                    self.ask_settings.recording,
                )
    }

    fn hex_status_at(&mut self, r: Result<EngineState, HexError>, now: Instant) {
        let show_meetings_setup = !self.hex.meetings_setup_checked
            && r.as_ref()
                .is_ok_and(|state| state.labs.state("meetings") == Some(false));
        self.hex.meetings_setup_checked |= r.is_ok();
        let check_installation = !self.hex.install.busy() && !super::isolated();
        #[cfg(test)]
        let check_installation = check_installation && !SKIP_INSTALL_DETECT.get();
        if check_installation {
            self.hex.installation = sidenote::hex_install::detect();
        }
        let running = r.is_ok();
        let model_changed = r
            .as_ref()
            .ok()
            .zip(self.hex.state())
            .is_some_and(|(now, before)| now.model != before.model);
        if running && (!self.hex.was_running || model_changed) {
            self.hex.data_dirty = true;
        }
        self.hex.was_running = running;
        if let Ok(st) = &r
            && self.hex.caps_version.as_deref() != Some(st.version.as_str())
        {
            self.hex.caps = None;
            if !self.hex.caps_loading && self.hex.caps_retry_at.is_none_or(|t| Instant::now() >= t)
            {
                self.hex.caps_loading = true;
                let version = st.version.clone();
                self.hex_spawn(move |e| Msg::Caps(version, e.capabilities()));
            }
        }
        let recording = r.as_ref().ok().map(|state| state.recording.is_some());
        if self.ask_settings.recording && recording == Some(false) {
            if !self.hex_stop_pending() {
                self.request_refresh();
            }
            self.ask_settings.last_end_ms = Some(chrono::Local::now().timestamp_millis());
            self.ask_settings.last_live = self.ask_settings.recording_source.take();
        }
        self.hex.connection.apply(&self.hex.status, &r, now);
        if !self.hex.record_busy && r.as_ref().is_ok_and(stop::terminal) {
            self.hex_check_stop();
        }
        self.hex.status = Some(r);
        if show_meetings_setup && !super::isolated() {
            self.open_hex();
            self.hex.section = Section::Dictation;
        }
        if let Some(recording) = recording {
            if recording {
                self.ask_settings.recording_source =
                    Some(sidenote::core::ask_options::LiveSource {
                        hint: recording_id(self),
                        start_ms: recording_start(self).map(|(at, _)| at.timestamp_millis()),
                    });
            }
            if recording != self.ask_settings.recording {
                self.meeting_agent_transition();
                self.ask_settings.recording = recording;
                self.load_saved();
                if self.waker.is_some() {
                    sidenote::core::ask_session::recording(recording);
                    if recording {
                        self.ask_settings.warm_signature.clear();
                        self.warm_ask();
                    }
                }
            }
        }
        self.live_sync();
        self.hex_refresh_if_due();
        // A minimize can slip past the focus event; this catches it on the next sample.
        if !window_shown() {
            self.hex_visibility(false);
        }
    }

    fn hex_refresh_if_due(&mut self) {
        if self.on_hex() && self.hex.data_dirty && self.hex.running() && !self.hex.data_loading {
            self.hex.data_dirty = false;
            self.hex.data_loading = true;
            self.hex_spawn(|e| Msg::Data {
                settings: e.settings_list(),
                vocabulary: e.vocabulary_list(),
                models: e.models_list(),
            });
        }
    }

    fn hex_set_settings(&mut self, r: Result<Vec<Setting>, HexError>) {
        let list = match r {
            Ok(list) => list,
            Err(e) => {
                self.hex.settings_note = Some(plain(&e));
                return;
            }
        };
        self.hex.settings_note = None;
        // A draft the owner changed and has not saved survives the refresh.
        let kept: HashSet<String> = self
            .hex
            .drafts
            .iter()
            .filter(|(k, d)| {
                !self.hex.fresh.contains(*k)
                    && self
                        .hex
                        .setting(k)
                        .is_some_and(|old| d.text != setting_text(&old.value))
            })
            .map(|(k, _)| k.clone())
            .collect();
        self.hex
            .drafts
            .retain(|k, _| list.iter().any(|s| &s.key == k));
        for st in &list {
            if editable(st)
                && !matches!(st.kind.as_str(), "bool" | "choice")
                && !kept.contains(&st.key)
            {
                self.hex
                    .drafts
                    .insert(st.key.clone(), TextEdit::new(setting_text(&st.value)));
            }
        }
        self.hex.fresh.clear();
        self.hex.settings = list;
    }

    fn hex_set_vocabulary(&mut self, r: Result<Vec<VocabularyEntry>, HexError>) {
        match r {
            Ok(v) => {
                self.hex.vocabulary = v;
                self.hex.vocabulary_note = None;
            }
            Err(e) => self.hex.vocabulary_note = Some(plain(&e)),
        }
    }

    fn hex_done(&mut self, mut action: Action, r: Result<(), HexError>) {
        let (sample, notes_requested) = match &mut action {
            Action::Stop(recording, sample) => (
                sample.take().map(|sample| *sample),
                self.hex.notes_stop_recording.take() == Some(*recording),
            ),
            _ => (None, false),
        };
        let checked = sample.is_some();
        let observed = sample.filter(|sample| self.hex_accept_status(sample));
        let applied = observed.is_some();
        if let Some((sent, status)) = observed {
            self.hex_status_at(status, sent);
        }
        if matches!(action, Action::Start) {
            self.hex.connection.finish_action();
        }
        match action {
            Action::Start | Action::Stop(..) => {
                self.hex.record_busy = false;
                match r {
                    Ok(()) => {
                        self.hex.error = None;
                        if let Action::Stop(recording, _) = action {
                            if notes_requested
                                && let Some(live) = &mut self.live
                                && live.recording_id == recording
                            {
                                live.generate_after_pause = true;
                            }
                            self.hex_check_stop();
                        } else {
                            self.hex_spawn(|e| Msg::ObservedStatus(sample_status(|| e.status())));
                        }
                    }
                    Err(e) => {
                        if matches!(action, Action::Start) && self.hex.recording().is_none() {
                            self.parts_start_failed();
                        }
                        self.hex.error = Some(match (&action, &e) {
                            (
                                Action::Stop(..),
                                HexError::Failed {
                                    code: Some(code), ..
                                },
                            ) if matches!(
                                code.as_str(),
                                "recording-changed" | "stop-unconfirmed" | "not-recording"
                            ) =>
                            {
                                plain(&e)
                            }
                            (Action::Stop(..), _) => "Could not stop recording. Stop it from Hex.",
                            _ => plain(&e),
                        });
                        if let Action::Stop(recording, _) = action {
                            self.hex_stop_failed(recording, &e, checked, applied);
                        }
                    }
                }
            }
            Action::Set(key) => {
                self.hex.busy_keys.remove(&key);
                match r {
                    Ok(()) => {
                        self.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
                        self.hex.fresh.insert(key);
                        self.hex_spawn(|e| Msg::Settings(e.settings_list()));
                    }
                    Err(e) => {
                        self.hex.row_errors.insert(key, plain(&e).into());
                    }
                }
            }
            Action::Model => {
                self.hex.model_busy = false;
                match r {
                    Ok(()) => {
                        self.hex.models_note = None;
                        self.hex.data_dirty = true;
                        self.hex_refresh_if_due();
                    }
                    Err(e) => self.hex.models_note = Some(plain(&e)),
                }
            }
            Action::Add | Action::Remove => {
                self.hex.vocab_busy = false;
                match r {
                    Ok(()) => {
                        self.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
                        self.hex.vocab_error = None;
                        self.hex_spawn(|e| Msg::Vocabulary(e.vocabulary_list()));
                    }
                    Err(e) => self.hex.vocab_error = Some(plain(&e)),
                }
            }
        }
    }

    fn hex_change_model(&mut self, id: &str, action: ModelAction) {
        if rune::tooling::headless_run().is_some()
            || self.hex.model_busy
            || self.hex_recording()
            || self
                .hex
                .state()
                .is_none_or(|st| st.dictating != Some(false) || st.model.preparing.is_some())
        {
            return;
        }
        let Some(model) = self.hex.models.iter().find(|m| m.id == id) else {
            return;
        };
        if model_action(model, self.hex.can(Feature::ModelInstall)) != Some(action) {
            return;
        }
        let feature = match action {
            ModelAction::Use => Feature::ModelUse,
            ModelAction::Install => Feature::ModelInstall,
            ModelAction::OpenHex => Feature::WindowShow,
        };
        if !self.hex.can(feature) {
            return;
        }
        self.hex.model_busy = true;
        self.hex.models_note = None;
        let id = id.to_owned();
        self.hex_spawn(move |e| {
            Msg::Done(
                Action::Model,
                match action {
                    ModelAction::Use => e.model_use(&id),
                    ModelAction::Install => e.model_install(&id),
                    ModelAction::OpenHex => e.open_models(),
                },
            )
        });
    }

    pub(super) fn hex_check_settings(&mut self) {
        if self.hex.settings_status_pending {
            return;
        }
        self.hex.settings_status_pending = true;
        self.hex_spawn(|engine| Msg::SettingsStatus(sample_status(|| engine.status_for_command())));
    }

    pub fn hex_status_unknown(&self) -> bool {
        self.hex.connection.expired()
    }
    pub(super) fn hex_connection_tick(&mut self, tx: &mut Tx) -> bool {
        let now = Instant::now();
        if self.hex.visibility.is_some() && !self.hex_status_unknown() {
            self.hex.connection.reset_recovery_backoff();
        } else if cfg!(windows)
            && self.hex.visible
            && window_shown()
            && !self.hex.record_busy
            && let Some(waker) = self.waker.clone()
            && self.hex.connection.begin_recovery(now, tx)
        {
            if self.hex.visibility.is_none() {
                self.hex_start(waker);
            }
            if !self.hex_spawn(|engine| {
                Msg::RecoveryStatus(sample_status(|| engine.status_for_command()))
            }) {
                tx.wake_after(self.hex.connection.finish_recovery(now));
            }
        }
        self.hex.connection.tick(tx) | self.hex_stop_tick(now, tx)
    }
    pub(super) fn hex_recheck_resume(&mut self) {
        if self.hex.record_busy {
            return;
        }
        self.hex.record_busy = true;
        self.hex.error = None;
        self.hex_spawn(|engine| Msg::RecheckResume(sample_status(|| engine.status_for_command())));
    }
    pub(super) fn hex_recheck_start(&mut self) {
        self.hex.record_busy = true;
        self.hex.error = None;
        self.hex_spawn(|engine| Msg::RecheckStart(sample_status(|| engine.status_for_command())));
    }

    pub fn hex_recording(&self) -> bool {
        self.hex.recording().is_some()
    }

    /// A fresh status confirms that Hex is recording and gives the part Sidenote can own.
    pub(super) fn hex_confirmed_recording(&self) -> bool {
        !self.hex_status_unknown()
            && matches!(&self.hex.status, Some(Ok(state)) if state.meeting_state == "recording"
                && state.recording.as_ref().and_then(|r| r.id.as_deref()).is_some_and(|id| !id.is_empty()))
    }

    pub(super) fn pending_recording_start(&self) -> bool {
        self.live
            .as_ref()
            .and_then(|live| live.session.as_ref())
            .is_some_and(|m| m.pending_start_ms.is_some())
    }

    pub(super) fn hex_stop_confirmed(&self) -> bool {
        !self.hex_status_unknown() && matches!(&self.hex.status, Some(Ok(s)) if stop::terminal(s))
    }

    pub fn hex_running(&self) -> bool {
        self.hex.running()
    }

    pub fn hex_record_busy(&self) -> bool {
        self.hex.record_busy || self.hex_stop_pending()
    }

    pub fn hex_record_start(&mut self) {
        if !super::updates::listener_ready(self)
            || self.hex_record_busy()
            || self.hex.connection.slow(&self.hex.status)
            || !self.hex.can(Feature::RecordStart)
            || self.hex.recording().is_some()
            || self
                .hex
                .state()
                .is_some_and(|state| state.labs.state("meetings") == Some(false))
        {
            return;
        }
        self.hex.record_busy = true;
        self.hex.error = None;
        let cached = self.hex.caps.clone().unwrap_or_default();
        self.hex.connection.action_stage("request_sent");
        self.hex_spawn(move |e| Msg::Done(Action::Start, e.record_start(None, &cached)));
    }

    pub fn hex_record_stop(&mut self, displayed_id: Option<String>) {
        self.hex_submit_stop(displayed_id, false);
    }

    pub(super) fn hex_submit_stop(&mut self, displayed_id: Option<String>, generate_notes: bool) {
        if self.hex_stop_pending() {
            return;
        }
        if displayed_id != self.hex.recording().and_then(|r| r.id.clone()) {
            self.hex.error = Some(
                "The recording changed before Stop was pressed. Stop was not sent. Stop it from Hex.",
            );
            return;
        }
        let pressed = Instant::now();
        // Notes typed on the live page reach the folder before the pipeline can start.
        self.live_flush();
        if !self.hex_recording() {
            if self.hex_running() {
                self.live_sync();
            }
            if self
                .live
                .as_ref()
                .is_some_and(|live| live.phase == super::live::Phase::Making)
            {
                self.leave_live();
            }
            self.hex.error = stop_blocked(self);
            return;
        }
        if let Some(reason) = stop_blocked(self) {
            self.hex.error = Some(reason);
            return;
        }
        let expected_id = displayed_id.unwrap_or_default();
        let cached = self.hex.caps.clone().unwrap_or_default();
        self.hex.record_busy = true;
        self.hex.error = None;
        let engine = self.hex.engine.clone();
        let recording = self.live.as_mut().map_or(0, |live| {
            if live.session.is_none() {
                live.notes_response_time =
                    super::Operation::at(super::TimedAction::Notes, pressed).defer();
            }
            live.recording_id
        });
        let events = self.events_tx.clone();
        let wake = self.waker.clone();
        self.hex_begin_stop(recording, pressed);
        if self.live_work(move || {
            let (status, result) = engine.record_stop(&expected_id, &cached);
            let _ = events.send(Event::Hex(Msg::Done(
                Action::Stop(recording, status.map(Box::new)),
                result,
            )));
            if let Some(wake) = wake {
                wake();
            }
        }) {
            self.hex.notes_stop_recording = generate_notes.then_some(recording);
        } else {
            self.hex.record_busy = false;
            self.hex.stop = None;
            self.hex.error = Some("Could not stop recording. Try Stop again or stop it in Hex.");
            if let Some(live) = &mut self.live {
                super::response_time::failed(live.take_notes_timing(recording));
            }
        }
    }

    /// Sends one setting, and only when the value differs from what Hex reports.
    fn hex_set(&mut self, key: &str, value: String) {
        if !self.hex.can(Feature::SettingsSet) || self.hex.busy_keys.contains(key) {
            return;
        }
        let Some(st) = self.hex.setting(key) else {
            return;
        };
        if !editable(st) || !changed(st, &value) {
            return;
        }
        self.hex.busy_keys.insert(key.to_string());
        self.hex.row_errors.remove(key);
        let key = key.to_string();
        self.hex_spawn(move |e| {
            let r = e.settings_set(&key, &value);
            Msg::Done(Action::Set(key), r)
        });
    }

    fn hex_keep_audio(&mut self, choice: &str) {
        self.hex.select_open = None;
        let value = match choice {
            "Same as meetings" => "none",
            "24 hours" => "day",
            "7 days" => "week",
            "1 month" => "month",
            "Forever" => "forever",
            _ => return,
        };
        let audio = AudioRetention::from_settings(&self.hex.settings);
        if !self.hex.can(Feature::SettingsSet)
            || self.hex.settings_note.is_some()
            || self.hex.busy_keys.contains("meetings.audio_retention")
            || !audio.editable
        {
            return;
        }
        self.hex.busy_keys.insert("meetings.audio_retention".into());
        self.hex.row_errors.remove("meetings.audio_retention");
        self.hex_spawn(move |e| {
            let result = audio.set(&e, value);
            let settings = e.settings_list();
            Msg::AudioSettings(result, settings)
        });
    }

    fn hex_toggle(&mut self, key: &str) {
        let Some(on) = self.hex.setting(key).and_then(|s| s.value.as_bool()) else {
            return;
        };
        self.hex_set(key, (!on).to_string());
    }

    fn hex_choose(&mut self, key: &str, value: &str) {
        self.hex.select_open = None;
        self.hex_set(key, value.to_string());
    }

    fn hex_commit(&mut self, key: &str) {
        let Some(st) = self.hex.setting(key) else {
            return;
        };
        let text = self
            .hex
            .drafts
            .get(key)
            .map(|d| d.text.trim().to_string())
            .unwrap_or_default();
        let valid = match st.kind.as_str() {
            "integer" => text.parse::<i64>().is_ok(),
            "number" => text.parse::<f64>().is_ok_and(f64::is_finite),
            _ => true,
        };
        if !valid {
            self.hex
                .row_errors
                .insert(key.to_string(), "Enter a number.".into());
            return;
        }
        self.hex_set(key, text);
    }

    fn hex_revert(&mut self, key: &str) {
        let text = self
            .hex
            .setting(key)
            .map(|s| setting_text(&s.value))
            .unwrap_or_default();
        self.hex.drafts.insert(key.to_string(), TextEdit::new(text));
        self.hex.row_errors.remove(key);
    }

    fn hex_vocab_add(&mut self) {
        let term = self.hex.term.text.trim().to_string();
        if term.is_empty() || self.hex.vocab_busy || !self.hex.can(Feature::VocabularyAdd) {
            return;
        }
        let sounds: Vec<String> = self
            .hex
            .sounds
            .text
            .split(',')
            .map(|w| w.trim().to_string())
            .filter(|w| !w.is_empty())
            .collect();
        self.hex.vocab_busy = true;
        self.hex.vocab_error = None;
        self.hex_spawn(move |e| {
            let sounds: Vec<&str> = sounds.iter().map(String::as_str).collect();
            let r = e.vocabulary_add(&term, &sounds);
            Msg::Done(Action::Add, r)
        });
        self.hex.term = TextEdit::default();
        self.hex.sounds = TextEdit::default();
    }

    fn hex_vocab_remove(&mut self, term: &str) {
        if self.hex.vocab_busy || !self.hex.can(Feature::VocabularyRemove) {
            return;
        }
        self.hex.vocab_busy = true;
        self.hex.vocab_error = None;
        let term = term.to_string();
        self.hex_spawn(move |e| {
            let r = e.vocabulary_remove(&term);
            Msg::Done(Action::Remove, r)
        });
    }
}

// ---- the status words -----------------------------------------------------------------------

/// The dot and words for the indicator, from the latest sample.
fn indicator(s: &State, p: Palette) -> (Color, String) {
    if s.hex.connection.expired() || s.hex.connection.slow(&s.hex.status) {
        return (p.warn, s.hex.connection.message());
    }
    match &s.hex.status {
        None => (p.faint, "Checking Hex".into()),
        Some(Err(HexError::HexNotRunning))
            if s.hex.installation == sidenote::hex_install::Installation::Missing =>
        {
            (p.warn, "Install Hex".into())
        }
        Some(Err(HexError::HexNotRunning)) => (p.faint, "Hex not running".into()),
        Some(Err(HexError::Timeout)) => (p.warn, "Hex not answering".into()),
        Some(Err(_)) => (p.warn, "Hex unavailable".into()),
        Some(Ok(st)) if st.labs.state("meetings") == Some(false) => (p.warn, "Meetings off".into()),
        Some(Ok(st)) => match (&st.recording, st.dictating) {
            (Some(r), _) => (
                p.warn,
                match elapsed(r) {
                    Some(sec) => format!("Recording {}", clock(sec)),
                    None => "Recording".into(),
                },
            ),
            (None, Some(true)) => (p.open, "Dictating".into()),
            (None, _) => (p.ok, "Ready".into()),
        },
    }
}

/// The dot and short words for the sidebar's Hex line.
pub fn short_status(s: &State, p: Palette) -> (Color, String) {
    if !super::updates::listener_ready(s) {
        return (p.warn, "Recording off".into());
    }
    if s.hex.connection.expired() || s.hex.connection.slow(&s.hex.status) {
        return indicator(s, p);
    }
    match &s.hex.status {
        None => (p.faint, "Checking".into()),
        Some(Err(HexError::HexNotRunning))
            if s.hex.installation == sidenote::hex_install::Installation::Missing =>
        {
            (p.warn, "Install Hex".into())
        }
        Some(Err(HexError::HexNotRunning)) => (p.faint, "Not running".into()),
        Some(Err(HexError::Timeout)) => (p.warn, "Not answering".into()),
        Some(Err(_)) => (p.warn, "Unavailable".into()),
        Some(Ok(_)) => {
            let (ink, words) = indicator(s, p);
            (ink, words)
        }
    }
}

/// The id Hex gives the running recording, which is the name of its meeting folder.
pub fn recording_id(s: &State) -> Option<String> {
    s.hex.recording().and_then(|r| r.id.clone())
}

pub(super) fn part_elapsed_ms(s: &State) -> Option<u64> {
    s.hex
        .recording()?
        .elapsed_seconds
        .map(|s| s.saturating_mul(1000))
}

/// The elapsed time of the running recording, as `12:34`.
pub fn recording_clock(s: &State) -> Option<String> {
    s.hex.recording().and_then(elapsed).map(clock)
}

/// When the running recording began, by the wall clock. The flag is true when Hex gave the start
/// itself, and false when it is worked back from the elapsed seconds (which can drift a little).
pub fn recording_start(s: &State) -> Option<(chrono::DateTime<chrono::Local>, bool)> {
    let r = s.hex.recording()?;
    if let Some(t) = r.since {
        return Some((t.into(), true));
    }
    let secs = i64::try_from(r.elapsed_seconds?).ok()?;
    Some((
        chrono::Local::now() - chrono::Duration::seconds(secs),
        false,
    ))
}

pub(super) const CHECKING: &str = "Checking for Hex";
pub(super) const WAIT_FOR_STOP: &str = "Check that Hex has finished the previous recording.";

/// Why a new recording cannot start right now, in words fit for a tooltip.
pub fn start_blocked(s: &State) -> Option<&'static str> {
    let reason = start_block_reason(s);
    s.hex.connection.log_blocked(reason);
    reason
}

fn start_block_reason(s: &State) -> Option<&'static str> {
    if cfg!(target_os = "linux") {
        Some("Recording is not available on Linux yet.")
    } else {
        super::updates::listener_start_blocked(s).or_else(|| recorder_start_block_reason(s))
    }
}

fn recorder_start_block_reason(s: &State) -> Option<&'static str> {
    let h = &s.hex;
    if s.hex_stop_pending() {
        Some(WAIT_FOR_STOP)
    } else if h.connection.expired() {
        if h.record_busy {
            Some("Checking Hex before New meeting")
        } else {
            None
        }
    } else if h.connection.slow(&h.status) {
        Some(connection::SLOW)
    } else if matches!(h.status, Some(Err(HexError::HexNotRunning)))
        && h.installation == sidenote::hex_install::Installation::Missing
    {
        Some("Install Hex in Settings > Hex and dictation to record and transcribe.")
    } else if matches!(h.status, Some(Err(HexError::HexNotRunning))) {
        Some("Open Hex, then try New meeting again.")
    } else if h.status.is_none() || h.caps_loading {
        Some(CHECKING)
    } else if !h.running() {
        Some("This version of Hex is not supported.")
    } else if h
        .state()
        .is_some_and(|state| state.labs.state("meetings") == Some(false))
    {
        Some("Turn on Meetings in Hex. Open Settings > Hex and dictation.")
    } else if h.recording().is_some() {
        Some("Already recording")
    } else if h.record_busy {
        Some("Working")
    } else if h.lacks(Feature::RecordStart) {
        Some("This Hex build cannot record")
    } else if !h.can(Feature::RecordStart) {
        Some(CHECKING)
    } else {
        None
    }
}

/// Why the recording cannot be stopped right now.
pub fn stop_blocked(s: &State) -> Option<&'static str> {
    let h = &s.hex;
    if s.hex_stop_pending() {
        Some("Stop is already in progress. Check the meeting page.")
    } else if !h.running() && h.recording().is_none() {
        Some(if h.connection.slow(&h.status) {
            "Hex is slow to answer. Stop it from Hex."
        } else {
            "Hex is not running. Stop it from Hex if it is recording."
        })
    } else if h.recording().is_none() {
        Some("Hex says no meeting is recording.")
    } else if h.record_busy {
        Some("Stop is already in progress. Stop it from Hex if needed.")
    } else if h.caps.as_ref().and_then(|c| c.get(&Feature::RecordStop)) != Some(&Support::Supported)
    {
        Some("This Hex build cannot stop. Stop it from Hex.")
    } else {
        None
    }
}

/// Why Record or Stop failed, with a Dismiss link. Not a dialog.
pub fn error_bar(s: &State, p: Palette) -> Option<El> {
    let recheck = s.hex_stop_unconfirmed() && !s.on_live();
    let note = if recheck {
        stop::UNCONFIRMED
    } else {
        s.hex.error?
    };
    Some(
        row()
            .key("hex-error")
            .px(24.0)
            .py(8.0)
            .gap(12.0)
            .items_center()
            .fill(p.warnbg)
            .child(
                roles::SMALL
                    .medium()
                    .wrap(note, p.warn)
                    .grow(1.0)
                    .role("alert"),
            )
            .child(
                crate::ui::widgets::action(if recheck { "Check again" } else { "Dismiss" }, p)
                    .key("hex-error-dismiss")
                    .on_press(move |s: &mut State| {
                        if recheck {
                            s.hex_recheck_stop();
                        } else {
                            s.hex.error = None;
                        }
                    }),
            ),
    )
}

// ---- the page -------------------------------------------------------------------------------

fn note(text: &str, p: Palette) -> El {
    roles::SMALL
        .medium()
        .wrap(text, p.warn)
        .role("alert")
        .mb(8.0)
}

fn summary_line(s: &State, p: Palette) -> El {
    let (_, words) = indicator(s, p);
    let missing =
        s.hex.stopped() && s.hex.installation == sidenote::hex_install::Installation::Missing;
    let words = if missing {
        "Not installed".into()
    } else {
        words
    };
    let status = s
        .hex
        .state()
        .map_or(words.clone(), |st| format!("Hex {}. {words}", st.version));
    settings::setting_row(
        s,
        "hex-status",
        "Hex",
        if s.hex.running() {
            "Hex is running. Check its status here before recording."
        } else if missing {
            "Needed to record and transcribe meetings."
        } else {
            "Check the Hex connection before recording."
        },
        roles::CONTROL.wrap(status, p.soft),
        false,
        p,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelAction {
    Use,
    Install,
    OpenHex,
}
fn model_action(model: &ModelInfo, can_install: bool) -> Option<ModelAction> {
    if !model.installed {
        Some(if can_install {
            ModelAction::Install
        } else {
            ModelAction::OpenHex
        })
    } else if model.selected {
        None
    } else {
        Some(ModelAction::Use)
    }
}
fn model_section(s: &State, p: Palette) -> El {
    let h = &s.hex;
    let mut sec = col().key("hex-models").w_full();
    // The list below marks the selected model, so this line only stands in when it is empty.
    if let Some(st) = h.state().filter(|_| h.models.is_empty()) {
        let m = &st.model;
        let name = m
            .name
            .clone()
            .or_else(|| m.id.clone())
            .unwrap_or_else(|| "Unknown".into());
        let mut r = row()
            .w_full()
            .gap(8.0)
            .items_center()
            .flex_wrap()
            .py(6.0)
            .child(
                roles::SMALL
                    .medium()
                    .wrap(format!("Selected: {name}"), p.ink),
            );
        r = r.child(match m.installed {
            Some(true) => badge("Installed", p.ok, Some(p.okbg)),
            Some(false) => badge("Not installed", p.warn, Some(p.warnbg)),
            None => badge("Install state unknown", p.soft, Some(p.chip)),
        });
        if m.preparing.is_some() {
            r = r.child(badge("Preparing", p.ink, Some(p.chip)));
        }
        sec = sec.child(r);
    }
    if let Some(n) = h.models_note.filter(|_| h.running()) {
        sec = sec.child(note(n, p));
    }
    for m in &h.models {
        let mut r = row()
            .key(format!("hex-model-{}", m.id))
            .w_full()
            .gap(8.0)
            .py(5.0)
            .items_center()
            .flex_wrap()
            .child(
                if m.selected {
                    roles::NAME.wrap(m.name.clone(), p.ink)
                } else {
                    roles::SMALL.wrap(m.name.clone(), p.ink)
                }
                .w(170.0),
            )
            .child(
                roles::META
                    .wrap(format!("{}  {}", m.languages.join(", "), m.size), p.faint)
                    .w(190.0),
            )
            .child(
                if m.installed {
                    badge("Installed", p.ok, None)
                } else {
                    badge("Not installed", p.faint, None)
                }
                .w(100.0),
            );
        if m.selected {
            r = r.child(roles::CONTROL.line("Selected", p.soft));
        }
        if let Some(action) = model_action(m, h.can(Feature::ModelInstall)) {
            let (label, feature, key) = match action {
                ModelAction::Use => ("Use", Feature::ModelUse, "use"),
                ModelAction::Install => ("Install", Feature::ModelInstall, "install"),
                ModelAction::OpenHex => ("Install in Hex", Feature::WindowShow, "open"),
            };
            let id = m.id.clone();
            let idle = !h.model_busy
                && h.recording().is_none()
                && h.state()
                    .is_some_and(|st| st.dictating == Some(false) && st.model.preparing.is_none());
            r = r.child(
                crate::ui::widgets::button(
                    label,
                    &format!("hex-model-{key}-{}", m.id),
                    crate::ui::widgets::ButtonRole::Secondary,
                    p,
                )
                .disabled(!idle || !h.can(feature))
                .on_press(move |s: &mut State| s.hex_change_model(&id, action)),
            );
        }
        sec = sec.child(r);
    }
    sec
}

fn vocabulary_section(s: &State, k: &Kit, p: Palette) -> El {
    let h = &s.hex;
    let can_add = h.can(Feature::VocabularyAdd) && !h.vocab_busy;
    let framed = |e: El, focused: bool| {
        e.border(1.0, if focused { p.accent } else { p.line })
            .radius(8.0)
            .fill(p.card)
    };
    let term = framed(
        TextInput::new(&h.term)
            .plain()
            .placeholder("Word or phrase")
            .label("New vocabulary word")
            .height(36.0)
            .disabled(!can_add)
            .on_edit(|s: &mut State| &mut s.hex.term)
            .on_submit(|s: &mut State, _| s.hex_vocab_add())
            .build(k)
            .key("hex-term"),
        h.term.focused,
    )
    .grow(1.0)
    .min_w(140.0);
    let sounds = framed(
        TextInput::new(&h.sounds)
            .plain()
            .placeholder("Sounds like, optional, comma separated")
            .label("Sounds like")
            .height(36.0)
            .disabled(!can_add)
            .on_edit(|s: &mut State| &mut s.hex.sounds)
            .on_submit(|s: &mut State, _| s.hex_vocab_add())
            .build(k)
            .key("hex-sounds"),
        h.sounds.focused,
    )
    .grow(1.0)
    .min_w(140.0);
    let ready = can_add && !h.term.text.trim().is_empty();
    let mut add = super::widgets::button(
        "Add",
        "hex-vocab-add",
        super::widgets::ButtonRole::Primary,
        p,
    )
    .label("Add the word");
    if ready {
        add = add.on_press(|s: &mut State| s.hex_vocab_add());
    } else {
        add = add.dim(0.5).disabled(true);
    }
    let mut sec = col()
        .key("hex-vocabulary")
        .w_full()
        .child(roles::SMALL.wrap(
            "Sidenote spells these right in transcripts and summaries.",
            p.soft,
        ))
        .child(section_head(
            Section::Vocabulary,
            Some(h.vocabulary.len()),
            p,
        ))
        .child(
            row()
                .w_full()
                .flex_wrap()
                .items_center()
                .gaps(8.0, 8.0)
                .mb(8.0)
                .child(term)
                .child(sounds)
                .child(add),
        );
    if h.lacks(Feature::VocabularyAdd) {
        sec = sec.child(note("This Hex build cannot add words.", p));
    }
    if let Some(n) = h.vocabulary_note.filter(|_| h.running()).or(h.vocab_error) {
        sec = sec.child(note(n, p));
    }
    if h.vocabulary.is_empty() && h.vocabulary_note.is_none() && h.running() {
        sec = sec.child(roles::SMALL.wrap("No words yet.", p.soft));
    }
    let can_remove = h.can(Feature::VocabularyRemove) && !h.vocab_busy;
    for (i, v) in h.vocabulary.iter().enumerate() {
        let mut text = col()
            .grow(1.0)
            .min_w(0.0)
            .child(roles::NAME.line(v.term.clone(), p.ink));
        if !v.sounds_like.is_empty() {
            text = text.child(
                roles::META.line(format!("Sounds like {}", v.sounds_like.join(", ")), p.faint),
            );
        }
        let term = v.term.clone();
        let mut remove = pressable(roles::CONTROL.wrap("Remove", p.soft))
            .key(format!("hex-vocab-rm-{i}"))
            .label(format!("Remove {}", v.term))
            .px(8.0)
            .py(2.0)
            .radius(6.0);
        if can_remove {
            remove = remove
                .hover(p.chip)
                .on_press(move |s: &mut State| s.hex_vocab_remove(&term));
        } else {
            remove = remove.dim(0.5);
        }
        sec = sec
            .child(
                row()
                    .key(format!("hex-vocab-{i}"))
                    .w_full()
                    .py(6.0)
                    .gap(12.0)
                    .items_center()
                    .child(text)
                    .child(remove),
            )
            .child(hairline(p));
    }
    sec
}

// ---- sections ---------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Connect,
    ConnectedApps,
    Teams,
    AiNotes,
    Updates,
    MeetingModels,
    Model,
    Vocabulary,
    General,
    Dictation,
    Meetings,
    Shortcuts,
    Advanced,
    Features,
    Experimental,
    Tasks,
    MeetingEngine,
    Identity,
    PromptLibrary,
    Sync,
    HelpersAdvanced,
    Privacy,
    Diagnostics,
}

/// The tabs, left to right. Features comes first: it is the one that shapes the app itself.
const SECTIONS: [Section; 13] = [
    Section::Identity,
    Section::Meetings,
    Section::PromptLibrary,
    Section::Experimental,
    Section::Tasks,
    Section::Dictation,
    Section::Connect,
    Section::ConnectedApps,
    Section::General,
    Section::Sync,
    Section::Privacy,
    Section::Updates,
    Section::Diagnostics,
];

impl Section {
    /// Every tab, in the order the page shows them.
    pub const ALL: [Section; 13] = SECTIONS;

    pub fn name(self) -> &'static str {
        match self {
            Self::Privacy => "Privacy and data",
            Self::Diagnostics => "Diagnostics",
            Self::Identity => "General",
            Self::PromptLibrary => "Prompt library",
            Self::Sync => "Sync",
            Self::HelpersAdvanced => "Models and agents advanced",
            Self::Connect => "Models and agents",
            Self::ConnectedApps => "Connected apps",
            Self::Teams => "People",
            Self::AiNotes => "Meeting summary",
            Self::Updates => "Updates",
            Self::MeetingModels => "Languages",
            Self::Features => "Features",
            Self::Experimental => "Experimental features",
            Self::Tasks => "Tasks",
            Self::MeetingEngine => "Meeting engine",
            Self::Model => "Model",
            Self::Vocabulary => "Names and terms",
            Self::General => "Appearance and startup",
            Self::Dictation => "Hex and dictation",
            Self::Meetings => "Meetings",
            Self::Shortcuts => "Shortcuts",
            Self::Advanced => "Advanced",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::Privacy => "What is sent, connected apps and retained files.",
            Self::Diagnostics => "Agent recovery and Hex diagnostics.",
            Self::Identity => "Your name and setup.",
            Self::PromptLibrary => "Edit Meeting summary instructions.",
            Self::Sync => "This PC and other devices.",
            Self::HelpersAdvanced => "Agent models and language handling.",
            Self::Connect => "Connect an AI app to your local meeting knowledge.",
            Self::ConnectedApps => "Add Sidenote to your installed AI apps.",
            Self::Teams => "Group tasks by the people who own them.",
            Self::AiNotes => {
                "Choose an agent and the instructions it uses to write meeting summaries."
            }
            Self::Updates => "Keep Sidenote up to date from your update folder.",
            Self::MeetingModels => "Choose spoken languages and download meeting models.",
            Self::Features => "Choose what appears on this device. Changes apply right away.",
            Self::Experimental => "Choose which experimental features are enabled.",
            Self::Tasks => "How the Tasks page opens.",
            Self::MeetingEngine => "Models, steps and prompts for turning meetings into notes.",
            Self::Model => "The speech model Hex uses to write down what it hears.",
            Self::Vocabulary => "Names and terms Hex should spell the way you do.",
            Self::General => "How Hex starts and how it sounds.",
            Self::Dictation => "Speaking into any app.",
            Self::Meetings => "How Hex records and saves meetings.",
            Self::Shortcuts => "Keys that start, stop and paste.",
            Self::Advanced => "Experimental and rarely used. The defaults suit most people.",
        }
    }
}

/// Where a setting is listed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    In(Section),
    /// Under Advanced: experimental features.
    Labs,
    /// Under Advanced: read only, unavailable, hidden, or not one of the everyday kinds.
    Rare,
}

fn place(st: &Setting) -> Place {
    let key = st.key.as_str();
    let head = key.split('.').next().unwrap_or_default();
    if head == "labs" {
        return Place::Labs;
    }
    // Nothing here can be changed, or it is not on offer, so it is not an everyday setting.
    if !st.writable || st.available == Some(false) || st.secret {
        return Place::Rare;
    }
    if st.kind == "shortcut"
        || key.contains("shortcut")
        || key.contains("hotkey")
        || matches!(head, "shortcuts" | "hotkeys")
    {
        return Place::In(Section::Shortcuts);
    }
    match head {
        "dictation" => Place::In(Section::Dictation),
        "meetings" | "meeting" => Place::In(Section::Meetings),
        "general" | "app" | "appearance" | "startup" | "audio" => Place::In(Section::General),
        _ => Place::Rare,
    }
}

/// The name a person would give it: "Launch at login", without the section it sits in.
fn human(st: &Setting, at: Place) -> String {
    if st.key == "meetings.retention" {
        return "Keep whole meetings".into();
    }
    match (at, st.key.split_once('.')) {
        (Place::In(_) | Place::Labs, Some((_, rest))) => pretty(rest),
        _ => pretty(&st.key),
    }
}

/// The first sentence of what Hex says about a setting.
fn one_line(about: &str) -> String {
    match about.split_once(". ") {
        Some((first, _)) => format!("{}.", first.trim_end_matches('.')),
        None => about.to_string(),
    }
}

fn section_head(section: Section, count: Option<usize>, p: Palette) -> El {
    let mut top = row()
        .gap(8.0)
        .items_center()
        .child(roles::SECTION.wrap(section.name(), p.ink));
    if let Some(n) = count {
        top = top.child(roles::count(n, p));
    }
    col()
        .key(format!("hex-head-{}", section.name()))
        .w_full()
        .gap(4.0)
        .mt(28.0)
        .mb(16.0)
        .child(top)
        .child(roles::SMALL.wrap(section.blurb(), p.soft))
}

fn keep_audio_row(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let h = &s.hex;
    let audio = AudioRetention::from_settings(&h.settings);
    let current = audio.current;
    let busy = h.busy_keys.contains("meetings.audio_retention");
    let can = h.can(Feature::SettingsSet) && h.settings_note.is_none() && !busy && audio.editable;
    let mut text = crate::ui::cards::settings_copy("Keep audio", p).child(roles::META.wrap(
        "Sidenote keeps the sound file next to the transcript.",
        p.soft,
    ));
    if let Some(reason) = h.page_note().or(h.settings_note) {
        text = text.child(roles::SMALL.wrap(reason, p.soft));
    } else if !can && !busy {
        text = text.child(roles::SMALL.wrap("This Hex build cannot change audio expiry.", p.soft));
    }
    if busy {
        text = text.child(roles::SMALL.wrap("Saving.", p.soft));
    }
    if let Some(error) = h.row_errors.get("meetings.audio_retention") {
        text = text.child(
            roles::SMALL
                .medium()
                .wrap(error.clone(), p.warn)
                .role("alert"),
        );
    }
    if audio.follows_meetings {
        text = text.child(roles::SMALL.wrap("Audio follows Keep whole meetings in Hex.", p.soft));
    }
    let options: Vec<_> = audio.choices().iter().map(|s| s.to_string()).collect();
    let control = Select::new("keep-audio", &options)
        .label("Keep audio")
        .current(if h.running() && h.settings_note.is_none() {
            current
        } else {
            None
        })
        .placeholder("Not read from Hex")
        .appearance(SelectAppearance::Field)
        .width(settings::short_select_width(s, 220.0))
        .disabled(!can)
        .open(can && h.select_open.as_deref() == Some("keep-audio"))
        .on_toggle(|s: &mut State| {
            s.hex.select_open = if s.hex.select_open.as_deref() == Some("keep-audio") {
                None
            } else {
                Some("keep-audio".into())
            };
        })
        .on_close(|s: &mut State| s.hex.select_open = None)
        .on_choose(|s: &mut State, value: &str, _: &mut Tx| s.hex_keep_audio(value))
        .build(&settings::row_field_kit(k, p), layer);
    crate::ui::cards::settings_row(true)
        .child(text)
        .child(control)
}

fn setting_row(
    s: &State,
    k: &Kit,
    st: &Setting,
    _at: Place,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    if st.key == "meetings.providers" && !st.secret {
        return readable::providers(s, k, st, p, layer);
    }
    let h = &s.hex;
    let key = st.key.clone();
    let name = settings::setting_name(st);
    let can = editable(st) && h.can(Feature::SettingsSet) && !h.busy_keys.contains(&key);
    let compact = settings::compact_rows(s);
    let help_role = if compact { roles::META } else { roles::SMALL };
    let mut text = if compact {
        crate::ui::cards::settings_copy(&name, p)
    } else {
        col().gap(4.0).grow(1.0).min_w(170.0).child(
            row()
                .items_center()
                .flex_wrap()
                .gap(8.0)
                .child(roles::NAME.wrap(name.clone(), p.ink)),
        )
    };
    if !st.about.is_empty() {
        text = text.child(
            help_role
                .wrap(settings::setting_help(st), p.soft)
                .help_when_truncated(settings::setting_help(st)),
        );
    }
    if st.available == Some(false) {
        text = text.child(help_role.wrap("Not available here.", p.soft));
    } else if !st.writable {
        text = text.child(help_role.wrap("Read only.", p.soft));
    }
    if let Some(e) = h.row_errors.get(&key) {
        text = text.child(help_role.medium().wrap(e.clone(), p.warn).role("alert"));
    }
    if h.busy_keys.contains(&key) {
        text = text.child(help_role.wrap("Saving.", p.soft));
    }
    // A folder or file path needs more room than a short value.
    let width = if st.kind == "path" {
        360.0_f32.min((s.app_w.get() - 64.0).max(220.0))
    } else {
        220.0
    };
    let model_edit_key = format!("model-edit-{}", st.key);
    let model_edit = s.hex.select_open.as_deref() == Some(model_edit_key.as_str());
    let control = if st.kind == "json" || st.value.is_array() || st.value.is_object() {
        roles::SMALL
            .wrap("Structured setting. Open Hex to review or edit it.", p.soft)
            .w(width)
    } else if st.key.contains("model") && st.kind == "text" && !model_edit {
        let edit_key = model_edit_key.clone();
        row()
            .w(width)
            .flex_wrap()
            .gap(8.0)
            .items_center()
            .child(roles::CONTROL.wrap(readable::current_label(st), p.ink))
            .children(editable(st).then(|| {
                crate::ui::widgets::action("Edit model", p)
                    .px(if compact { 10.0 } else { 12.0 })
                    .radius(if compact { 7.0 } else { 8.0 })
                    .key(model_edit_key)
                    .on_press(move |s: &mut State| s.hex.select_open = Some(edit_key.clone()))
            }))
    } else if !editable(st) {
        let shown = readable::current_label(st);
        roles::SMALL
            .line(
                if shown.is_empty() {
                    "Not set".into()
                } else {
                    shown.clone()
                },
                p.soft,
            )
            .w(width)
            .help_when_truncated(shown)
    } else {
        match st.kind.as_str() {
            "bool" => {
                let key = key.clone();
                Switch::new(st.value.as_bool().unwrap_or(false))
                    .label(name)
                    .hide_label()
                    .disabled(!can)
                    .on_toggle(move |s: &mut State| s.hex_toggle(&key))
                    .build(k)
                    .key(format!("hs-sw-{}", st.key))
            }
            "choice" => {
                let choices: Vec<_> = st
                    .choices
                    .iter()
                    .filter(|v| readable::supported_choice(v))
                    .cloned()
                    .collect();
                let options: Vec<String> = choices
                    .iter()
                    .map(|v| readable::value_label(st, v))
                    .collect();
                let current_label = readable::current_label(st);
                let current = Some(current_label.as_str());
                let open = can && h.select_open.as_deref() == Some(key.as_str());
                let (toggle_key, choose_key) = (key.clone(), key.clone());
                let labels = options.clone();
                let field_kit = compact.then(|| settings::row_field_kit(k, p));
                Select::new(format!("hs-sel-{key}"), &options)
                    .label(name)
                    .current(current)
                    .placeholder("Not set")
                    .appearance(SelectAppearance::Field)
                    .width(if compact {
                        settings::short_select_width(s, 220.0)
                    } else {
                        220.0
                    })
                    .disabled(!can)
                    .open(open)
                    .on_toggle(move |s: &mut State| {
                        s.hex.select_open =
                            if s.hex.select_open.as_deref() == Some(toggle_key.as_str()) {
                                None
                            } else {
                                Some(toggle_key.clone())
                            };
                    })
                    .on_close(|s: &mut State| s.hex.select_open = None)
                    .on_choose(move |s: &mut State, v: &str, _: &mut Tx| {
                        if let Some(index) = labels.iter().position(|label| label == v) {
                            s.hex_choose(&choose_key, &setting_text(&choices[index]));
                        }
                    })
                    .build(field_kit.as_ref().unwrap_or(k), layer)
            }
            _ => {
                let empty = TextEdit::default();
                let draft = h.drafts.get(&key).unwrap_or(&empty);
                let dirty = changed(st, draft.text.trim());
                let (submit_key, esc_key, save_key) = (key.clone(), key.clone(), key.clone());
                let input = TextInput::new(draft)
                    .plain()
                    .label(name)
                    .height(if compact { 30.0 } else { 32.0 })
                    .disabled(!can)
                    .on_edit(move |s: &mut State| s.hex.draft_mut(&key))
                    .on_submit(move |s: &mut State, _| s.hex_commit(&submit_key))
                    .on_key(move |s: &mut State, e: &KeyEvent, tx: &mut Tx| {
                        if e.key == Key::Escape {
                            s.hex_revert(&esc_key);
                            tx.blur();
                            return true;
                        }
                        false
                    })
                    .build(k)
                    .key(format!("hs-in-{}", st.key))
                    .border(1.0, if draft.focused { p.accent } else { p.line })
                    .radius(if compact { 7.0 } else { 8.0 })
                    .fill(p.card)
                    .grow(1.0)
                    .min_w(120.0);
                let mut r = row().w(width).gap(6.0).items_center().child(input);
                if dirty && can {
                    r = r.child(
                        crate::ui::widgets::action("Save", p)
                            .px(if compact { 10.0 } else { 12.0 })
                            .radius(if compact { 7.0 } else { 8.0 })
                            .key(format!("hs-save-{}", st.key))
                            .on_press(move |s: &mut State| s.hex_commit(&save_key)),
                    );
                }
                r
            }
        }
    };
    let (shell, control) = if compact {
        let has_help = !st.about.is_empty()
            || !st.writable
            || st.available == Some(false)
            || h.row_errors.contains_key(&st.key)
            || h.busy_keys.contains(&st.key);
        (
            crate::ui::cards::settings_row(has_help),
            settings::row_control(s, control),
        )
    } else {
        (
            crate::ui::cards::row_shell().flex_wrap().px(16.0).py(11.0),
            control,
        )
    };
    shell
        .key(format!("hs-{}", st.key))
        .child(text)
        .child(control)
        .fill(settings::highlight(s, &format!("hs-{}", st.key), p))
}

pub fn pane(
    s: &State,
    k: &Kit,
    p: Palette,
    narrow: bool,
    sidebar_k: &Kit,
    sidebar_p: Palette,
    layer: &mut Vec<El>,
) -> El {
    settings::pane(s, k, p, narrow, sidebar_k, sidebar_p, layer)
        .child(super::ask::overlay(s, k, p, narrow, layer))
}

pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    settings::on_key(s, e, tx)
}

// ---- render scenes --------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Fixture {
    version: String,
    model: ModelState,
    models: Vec<ModelInfo>,
    vocabulary: Vec<VocabularyEntry>,
    settings: Vec<Setting>,
}

/// Fills the Hex state from `hex-fixture.json` in the scene's state folder, which
/// `tools/render_ui.ps1` copies from `tools/ui-fixtures/hex.json`. A headless render starts no
/// watcher, and nothing here calls Hex, so the shot is the same every time.
pub fn scene(s: &mut State, recording: bool) {
    let Some(dir) = sidenote::env_var("SIDENOTE_STATE_DIR") else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(std::path::Path::new(&dir).join("hex-fixture.json"))
    else {
        return;
    };
    let Ok(f) = serde_json::from_str::<Fixture>(&text) else {
        return;
    };
    let caps: Capabilities = [
        Feature::RecordStart,
        Feature::RecordStop,
        Feature::SettingsSet,
        Feature::VocabularyAdd,
        Feature::VocabularyRemove,
        Feature::ModelUse,
        Feature::ModelInstall,
        Feature::WindowShow,
    ]
    .into_iter()
    .map(|feature| (feature, Support::Supported))
    .collect();
    s.hex.caps = Some(caps);
    s.hex.caps_version = Some(f.version.clone());
    s.hex.status = Some(Ok(EngineState {
        running: true,
        version: f.version,
        protocol: 1,
        recording: recording.then(|| Recording {
            id: Some("fixture".into()),
            title: None,
            since: None,
            elapsed_seconds: Some(754),
        }),
        meeting_state: if recording { "recording" } else { "idle" }.into(),
        dictating: Some(false),
        capture_state: "ready".into(),
        labs: Box::default(),
        model: f.model,
    }));
    s.hex.was_running = true;
    s.hex.data_dirty = false;
    s.hex.models = f.models;
    let labs: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/hex-labs/settings.json"))
            .expect("synthetic Labs fixture");
    let labs: Vec<Setting> =
        serde_json::from_value(labs["settings"].clone()).expect("synthetic Labs settings");
    s.hex.shell.labs = sidenote::hex_labs::Snapshot::settings(&labs);
    s.hex_set_settings(Ok(f.settings));
    s.hex_set_vocabulary(Ok(f.vocabulary));
}

#[cfg(test)]
mod model_action_tests {
    use super::*;
    #[test]
    fn installed_models_never_offer_download_and_selected_models_are_information() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../tools/ui-fixtures/hex.json")).unwrap();
        assert_eq!(model_action(&fixture.models[0], true), None);
        assert_eq!(
            model_action(&fixture.models[1], true),
            Some(ModelAction::Use)
        );
        assert_eq!(
            model_action(&fixture.models[1], false),
            Some(ModelAction::Use)
        );
        assert_eq!(
            model_action(&fixture.models[2], true),
            Some(ModelAction::Install)
        );
        assert_eq!(
            model_action(&fixture.models[2], false),
            Some(ModelAction::OpenHex)
        );
        let mut missing_selected = fixture.models[2].clone();
        missing_selected.selected = true;
        assert_eq!(
            model_action(&missing_selected, true),
            Some(ModelAction::Install)
        );
    }
}

#[cfg(test)]
mod notes_stop_tests {
    use super::*;
    use crate::ui::live::Phase;
    use sidenote::core::{self, notes};
    use std::{fs, path::Path, process::Command};

    const PART: &str = "100-1";

    fn fixture(name: &str) -> Option<State> {
        if std::env::var("SIDENOTE_NOTES_STOP_TEST").as_deref() != Ok(name) {
            let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-storage");
            fs::create_dir_all(&parent).unwrap();
            let root = tempfile::tempdir_in(parent).unwrap();
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", &format!("ui::hex::notes_stop_tests::{name}")])
                .env("SIDENOTE_NOTES_STOP_TEST", name)
                .env("SIDENOTE_DATA_DIR", root.path().join("state"))
                .env("SIDENOTE_STATE_DIR", root.path().join("state"))
                .env("SIDENOTE_HEX_DIR", root.path().join("meetings"))
                .env("SIDENOTE_PEERS_DIR", root.path().join("peers"));
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            assert!(command.status().unwrap().success());
            return None;
        }
        SKIP_INSTALL_DETECT.set(true);
        let mut state = State::new();
        let part = core::meetings_dir().join(PART);
        fs::create_dir_all(&part).unwrap();
        fs::write(
            part.join("manifest.json"),
            r#"{"status":"recording","duration_ms":1000}"#,
        )
        .unwrap();
        fs::write(
            part.join("transcript.ndjson"),
            r#"{"start_ms":0,"end_ms":1000,"text":"Synthetic part","source":"microphone"}"#,
        )
        .unwrap();
        fs::create_dir_all(core::state_dir()).unwrap();
        fs::write(
            core::state_dir().join("config.json"),
            r#"{"version":1,"features":{"indic_engine":false},"notes":{"engine":"app","auto":false}}"#,
        )
        .unwrap();
        state.hex_apply(Msg::Caps(
            "test".into(),
            Ok(Capabilities::from([(
                Feature::RecordStop,
                Support::Supported,
            )])),
        ));
        state.hex_apply(Msg::ObservedStatus(sample_status(|| Ok(status(true)))));
        state.parts_sync();
        state.parts_paused();
        assert_eq!(state.live.as_ref().unwrap().phase, Phase::Paused);
        assert!(state.hex_recording());
        Some(state)
    }

    fn status(recording: bool) -> EngineState {
        EngineState {
            running: true,
            version: "test".into(),
            protocol: 4,
            recording: recording.then(|| Recording {
                id: Some(PART.into()),
                title: None,
                since: None,
                elapsed_seconds: Some(1),
            }),
            meeting_state: if recording { "recording" } else { "idle" }.into(),
            dictating: Some(false),
            capture_state: "idle".into(),
            model: serde_json::from_str("{}").unwrap(),
            labs: Default::default(),
        }
    }

    fn stop_reply(state: &mut State, result: Result<(), HexError>) {
        let recording = state.live.as_ref().unwrap().recording_id;
        assert!(state.hex_stop_pending());
        state.hex_apply(Msg::Done(
            Action::Stop(
                recording,
                Some(Box::new(sample_status(|| Ok(status(true))))),
            ),
            result,
        ));
    }

    fn finish_stop(state: &mut State) {
        stop_reply(state, Ok(()));
        fs::write(
            core::meetings_dir().join(PART).join("manifest.json"),
            r#"{"status":"complete","duration_ms":1000}"#,
        )
        .unwrap();
        let deadline = state.hex.stop.as_ref().unwrap().deadline.unwrap();
        state.hex_apply(Msg::StopStatus(
            deadline,
            sample_status(|| Ok(status(false))),
        ));
        assert!(!state.hex_stop_pending());
    }

    fn pause_without_notes(state: &mut State) {
        let work = state.capture_live_work();
        state.meeting_pause();
        drop(work.try_recv().unwrap());
        finish_stop(state);
        assert_eq!(state.live.as_ref().unwrap().phase, Phase::Paused);
        assert!(matches!(work.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(notes::status(PART), None);
    }

    #[test]
    fn refused_generate_stop_does_not_start_notes_on_pause() {
        let Some(mut state) = fixture("refused_generate_stop_does_not_start_notes_on_pause") else {
            return;
        };
        let work = state.capture_live_work();
        state.meeting_generate();
        drop(work.try_recv().unwrap());
        assert!(!state.live.as_ref().unwrap().generate_after_pause);
        stop_reply(
            &mut state,
            Err(HexError::Failed {
                code: Some("not-recording".into()),
                stderr_summary: "Synthetic Stop refusal",
            }),
        );
        assert!(!state.hex_stop_pending());
        pause_without_notes(&mut state);
    }

    #[test]
    fn unavailable_generate_stop_does_not_start_notes_on_pause() {
        let Some(mut state) = fixture("unavailable_generate_stop_does_not_start_notes_on_pause")
        else {
            return;
        };
        let caps = state.hex.caps.take();
        state.meeting_generate();
        assert_eq!(
            state.hex.error,
            Some("This Hex build cannot stop. Stop it from Hex.")
        );
        assert!(!state.hex_stop_pending());
        state.hex.caps = caps;
        pause_without_notes(&mut state);
    }

    #[test]
    fn failed_generate_stop_submission_does_not_start_notes_on_pause() {
        let Some(mut state) =
            fixture("failed_generate_stop_submission_does_not_start_notes_on_pause")
        else {
            return;
        };
        drop(state.capture_live_work());
        state.meeting_generate();
        assert_eq!(
            state.hex.error,
            Some("Could not stop recording. Try Stop again or stop it in Hex.")
        );
        assert!(!state.hex_stop_pending());
        pause_without_notes(&mut state);
    }

    #[test]
    fn accepted_generate_stop_starts_one_notes_job() {
        let Some(mut state) = fixture("accepted_generate_stop_starts_one_notes_job") else {
            return;
        };
        let work = state.capture_live_work();
        state.meeting_generate();
        drop(work.try_recv().unwrap());
        assert_eq!(notes::status(PART), None);
        finish_stop(&mut state);
        assert_eq!(state.live.as_ref().unwrap().phase, Phase::Making);
        work.try_recv().unwrap()();
        state.live_sync();
        assert!(matches!(work.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(notes::status(PART), Some(notes::JobState::Queued));
    }

    fn busy_notes_job() -> std::path::PathBuf {
        let dir = core::state_dir().join("local/notes-jobs");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{PART}.json"));
        fs::write(&path, r#"{"state":"running","attempts":1,"updated_at":0,"model":"","instructions":"default","message":""}"#).unwrap();
        assert_eq!(notes::status(PART), Some(notes::JobState::Running));
        path
    }

    #[test]
    fn accepted_generate_retries_save_preparation_and_submission() {
        let Some(mut state) = fixture("accepted_generate_retries_save_preparation_and_submission")
        else {
            return;
        };
        let work = state.capture_live_work();
        state.meeting_generate();
        drop(work.try_recv().unwrap());
        let saved = core::state_dir().join(format!("local/meeting-parts/{PART}.json"));
        let original = fs::read(&saved).unwrap();
        fs::write(&saved, "{").unwrap();
        let busy = busy_notes_job();
        finish_stop(&mut state);
        assert_eq!(state.hex.error, Some("Could not save paused meeting."));
        assert!(matches!(work.try_recv(), Err(TryRecvError::Empty)));
        fs::write(saved, original).unwrap();
        state.live_sync();
        assert_eq!(
            state.hex.error,
            Some("Notes are still being written. Update them when they finish.")
        );
        assert!(matches!(work.try_recv(), Err(TryRecvError::Empty)));
        fs::remove_file(busy).unwrap();
        drop(work);
        state.live_sync();
        assert!(state.live.as_ref().unwrap().generate_after_pause);
        assert_eq!(notes::status(PART), None);
        let work = state.capture_live_work();
        state.live_sync();
        work.try_recv().unwrap()();
        state.live_sync();
        assert!(matches!(work.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(notes::status(PART), Some(notes::JobState::Queued));
    }

    #[test]
    fn resume_cancels_deferred_generate() {
        let Some(mut state) = fixture("resume_cancels_deferred_generate") else {
            return;
        };
        let busy = busy_notes_job();
        let work = state.capture_live_work();
        state.meeting_generate();
        drop(work.try_recv().unwrap());
        finish_stop(&mut state);
        assert!(state.live.as_ref().unwrap().generate_after_pause);
        state.meeting_resume();
        assert!(!state.live.as_ref().unwrap().generate_after_pause);
        fs::remove_file(busy).unwrap();
        state.hex_apply(Msg::ObservedStatus(sample_status(|| Ok(status(true)))));
        pause_without_notes(&mut state);
    }

    #[test]
    fn replacing_live_session_cancels_deferred_generate() {
        let Some(mut state) = fixture("replacing_live_session_cancels_deferred_generate") else {
            return;
        };
        let busy = busy_notes_job();
        let work = state.capture_live_work();
        state.meeting_generate();
        drop(work.try_recv().unwrap());
        finish_stop(&mut state);
        let live = state.live.as_ref().unwrap();
        assert!(live.generate_after_pause);
        let recording = live.recording_id;
        let root = core::state_dir();
        let mut other = core::meeting_parts::Meeting::begin(&root, "200-2", 200).unwrap();
        other.phase = core::meeting_parts::Phase::Ended;
        other.save(&root).unwrap();
        state.live_writer = None;
        state.selected = Some(other.id);
        state.meeting_generate();
        assert_ne!(state.live.as_ref().unwrap().recording_id, recording);
        fs::remove_file(busy).unwrap();
        state.live_sync();
        assert_eq!(notes::status(PART), None);
        assert_eq!(notes::status("200-2"), None);
    }
}
