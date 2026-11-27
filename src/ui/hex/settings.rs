//! Settings layout and destinations. Hex keys are routed, never renamed or migrated.
use super::*;
use crate::ui::{ai_notes, cards, icons, nav, tasks, widgets::ButtonDisabled};
use sidenote::core::{
    CHANNEL, Channel, Feature as AppFeature, TaskGrouping, TasksLayout, UiPrefs, Who,
};
use std::collections::BTreeMap;

mod engine;
#[cfg(windows)]
mod host;

const DRAWER_AT: f32 = 600.0;
const LIST_WIDE: f32 = 240.0;
const LIST_ROW_H: f32 = 32.0;
const TAB_H: f32 = 28.0;
const PAGE_COLUMN: f32 = 640.0;

#[derive(Default)]
pub struct Settings {
    pub navigation_open: bool,
    /// The settings list is folded away in a window wide enough to show it.
    pub nav_hidden: bool,
    pub labs: sidenote::hex_labs::Snapshot,
    pub labs_loading: bool,
    labs_show_all: bool,
    labs_version: Option<String>,
    lab_confirm: Option<String>,
    pub lab_error: Option<String>,
    jump_active: usize,
    jump_requested: Option<usize>,
    jump_origin: f32,
    jump_positions: BTreeMap<usize, f32>,
    jump_offset: std::rc::Rc<std::cell::Cell<f32>>,
    jump_max: std::rc::Rc<std::cell::Cell<f32>>,
    pub last_tabs: BTreeMap<String, usize>,
    back: Option<ReturnTo>,
    pub advanced: HashSet<&'static str>,
    pub highlighted: Option<(String, Instant)>,
    pub highlight_clock: Option<(Instant, f64)>,
    pub saved: Option<Instant>,
    pub helper_query: TextEdit,
    pub model_query: TextEdit,
    pub language_query: TextEdit,
    pub recommended: bool,
    pub model_recommended: bool,
    pub relay: sidenote::core::relay_preview::Settings,
    pub relay_address: TextEdit,
    pub relay_error: Option<String>,
    pub device_name: TextEdit,
    pub device_name_error: Option<String>,
    kb_fields: [TextEdit; 5],
    kb_base: Option<sidenote::core::Config>,
    kb_error: Option<String>,
    kb_open: bool,
    engine: engine::Engine,
    #[cfg(windows)]
    speech_host: host::Form,
}

struct ReturnTo {
    page: nav::Page,
    selected: Option<String>,
    filter: nav::Filter,
    live: bool,
    transcript: bool,
    meeting_tab: crate::ui::meeting::Tab,
}
pub(super) fn remember_back(s: &mut State) {
    s.hex.shell.back = Some(ReturnTo {
        page: s.page(),
        selected: s.selected.clone(),
        filter: s.filter.clone(),
        live: s.live_open,
        transcript: s.transcript_open,
        meeting_tab: s.meeting_tab,
    });
}

/// Reopens Settings after an Ask citation detour, keeping the settings back button's
/// address: `open_hex` remembers the current page, but the detour already reset to the list.
pub(crate) fn reopen(s: &mut State) {
    let back = s.hex.shell.back.take();
    s.open_hex();
    if let Some(back) = back {
        s.hex.shell.back = Some(back);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Meetings,
    Prompts,
    Experimental,
    Tasks,
    /// Search destination in the main sidebar.
    People,
    Hex,
    Helpers,
    ConnectedApps,
    Appearance,
    Sync,
    Privacy,
    Updates,
    Diagnostics,
}
impl Page {
    const ALL: [Self; 13] = [
        Self::General,
        Self::Meetings,
        Self::Prompts,
        Self::Experimental,
        Self::Tasks,
        Self::Helpers,
        Self::ConnectedApps,
        Self::Hex,
        Self::Appearance,
        Self::Sync,
        Self::Privacy,
        Self::Updates,
        Self::Diagnostics,
    ];
    fn group(self) -> &'static str {
        match self {
            Self::General | Self::Meetings | Self::Prompts | Self::Experimental | Self::People => {
                "You and meetings"
            }
            Self::Tasks => "Your work",
            Self::Helpers | Self::ConnectedApps => "Ask and agents",
            Self::Hex | Self::Appearance | Self::Sync => "This PC",
            Self::Privacy | Self::Updates | Self::Diagnostics => "Data and maintenance",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Meetings => "Meetings",
            Self::Prompts => "Prompt library",
            Self::Experimental => Section::Experimental.name(),
            Self::Tasks => "Tasks",
            Self::People => "People",
            Self::Hex => "Hex and dictation",
            Self::Helpers => "Models and agents",
            Self::ConnectedApps => "Connected apps",
            Self::Appearance => "Appearance and startup",
            Self::Sync => "Sync",
            Self::Privacy => "Privacy and data",
            Self::Updates => "Updates",
            Self::Diagnostics => "Diagnostics",
        }
    }
    fn visible(self, channel: Channel) -> bool {
        channel == Channel::Dev || !matches!(self, Self::Sync | Self::ConnectedApps)
    }
    fn shown(self, s: &State) -> bool {
        match self {
            Self::Tasks => s.config.features.tasks,
            _ => true,
        }
    }
    pub(in crate::ui) fn tabs(self) -> &'static [&'static str] {
        match self {
            Self::People => &["Our team", "External"],
            Self::Meetings => &["Recording", "Summaries", "Daily check", "Engine"],
            Self::Helpers => &["Agents", "Speech model"],
            _ => &[],
        }
    }
    fn section(self, tab: usize) -> Section {
        match self {
            Self::General => Section::Identity,
            Self::Meetings => match tab {
                1 => Section::AiNotes,
                2 => Section::Features,
                3 => Section::MeetingEngine,
                _ => Section::Meetings,
            },
            Self::Prompts => Section::PromptLibrary,
            Self::Experimental => Section::Experimental,
            Self::Tasks => Section::Tasks,
            Self::People => Section::Teams,
            Self::Hex => Section::Dictation,
            Self::Helpers => match tab {
                1 => Section::MeetingModels,
                2 => Section::HelpersAdvanced,
                _ => Section::Connect,
            },
            Self::ConnectedApps => Section::ConnectedApps,
            Self::Appearance => Section::General,
            Self::Sync => Section::Sync,
            Self::Privacy => Section::Privacy,
            Self::Updates => Section::Updates,
            Self::Diagnostics => Section::Diagnostics,
        }
    }
    fn icon(self) -> &'static rune::Icon {
        match self {
            Self::General => icons::GENERAL,
            Self::Meetings => &icons::MEETINGS,
            Self::Prompts => &icons::PROMPTS,
            Self::Experimental => &icons::FLASK,
            Self::Tasks => &icons::TASKS,
            Self::People => &icons::PEOPLE,
            Self::Hex => &icons::MIC,
            Self::Helpers => icons::AGENTS,
            Self::ConnectedApps => icons::CONNECTED_APPS,
            Self::Appearance => icons::APPEARANCE,
            Self::Sync => &icons::DEVICE,
            Self::Privacy => icons::PRIVACY,
            Self::Updates => icons::SETTINGS_UPDATES,
            Self::Diagnostics => icons::DIAGNOSTICS,
        }
    }
}

fn current(section: Section) -> (Page, usize) {
    match section {
        Section::Identity => (Page::General, 0),
        Section::Meetings | Section::Vocabulary => (Page::Meetings, 0),
        Section::AiNotes => (Page::Meetings, 1),
        Section::Features => (Page::Meetings, 2),
        Section::Experimental => (Page::Experimental, 0),
        Section::Tasks => (Page::Tasks, 0),
        Section::MeetingEngine => (Page::Meetings, 3),
        Section::PromptLibrary => (Page::Prompts, 0),
        Section::Teams => (Page::People, 0),
        Section::Model => (Page::Hex, 0),
        Section::MeetingModels => (Page::Helpers, 1),
        Section::Connect => (Page::Helpers, 0),
        Section::ConnectedApps => (Page::ConnectedApps, 0),
        Section::HelpersAdvanced => (Page::Diagnostics, 0),
        Section::Privacy => (Page::Privacy, 0),
        Section::Diagnostics => (Page::Diagnostics, 0),
        Section::General => (Page::Appearance, 0),
        Section::Sync => (Page::Sync, 0),
        Section::Updates => (Page::Updates, 0),
        Section::Advanced => (Page::Diagnostics, 0),
        Section::Dictation | Section::Shortcuts => (Page::Hex, 0),
    }
}

fn active(s: &State) -> (Page, usize) {
    let (page, tab) = current(s.hex.section);
    if page.visible(CHANNEL) && page.shown(s) {
        (page, tab)
    } else {
        (Page::General, 0)
    }
}

pub(in crate::ui) fn section_visible(s: &State, section: Section) -> bool {
    let page = current(section).0;
    page.visible(CHANNEL) && page.shown(s)
}

fn tab_visible(s: &State, page: Page, tab: usize) -> bool {
    (page, tab) != (Page::Meetings, 2) || s.config.features.memory
}

pub(crate) fn navigate(s: &mut State, page: Page, tab: usize) {
    if !page.visible(CHANNEL) {
        return;
    }
    if page == Page::People {
        if s.config.features.people {
            s.hex.filter = TextEdit::default();
            s.open_people_tab(tab);
        }
        return;
    }
    let tab = if tab_visible(s, page, tab) { tab } else { 0 };
    if page == Page::ConnectedApps && !super::super::isolated() {
        s.connect_agents(false, None);
    }
    let changed =
        !s.on_hex() || s.hex.section != page.section(tab.min(page.tabs().len().saturating_sub(1)));
    let timing = if changed {
        crate::ui::Operation::new(crate::ui::TimedAction::Settings)
    } else {
        crate::ui::Operation::default()
    };
    if page == Page::Meetings && tab == 2 && s.hex.shell.kb_base.is_none() {
        load_kb(s);
    }
    if page == Page::Meetings && tab == 3 {
        engine::enter(s);
    }
    if page == Page::Meetings && !super::super::isolated() {
        s.hex_check_settings();
    }
    s.hex.shell.navigation_open = false;
    s.hex.shell.lab_confirm = None;
    s.hex.shell.jump_active = 0;
    s.hex.shell.jump_requested = None;
    s.hex.shell.jump_origin = 0.0;
    s.hex.shell.jump_positions.clear();
    s.hex.shell.jump_offset.set(0.0);
    s.hex.shell.jump_max.set(0.0);
    let tab = tab.min(page.tabs().len().saturating_sub(1));
    s.hex.section = page.section(tab);
    s.hex.select_open = None;
    s.hex.filter = TextEdit::default();
    s.hex.shell.last_tabs.insert(page.name().into(), tab);
    if rune::tooling::headless_run().is_none() {
        let mut prefs = UiPrefs::load(&s.store.root);
        prefs.settings_tabs = s.hex.shell.last_tabs.clone();
        let _ = prefs.save(&s.store.root);
    }
    super::super::scene_scroll(s, "hex-top");
    timing.ready();
}

fn back(s: &mut State) {
    let dest = s.hex.shell.back.take();
    s.go_home();
    if let Some(dest) = dest {
        if dest.page != nav::Page::Home {
            s.view = tasks::View::Meetings;
        }
        s.filter = dest.filter;
        s.selected = dest.selected;
        s.live_open = dest.live;
        s.transcript_open = dest.transcript;
        s.meeting_tab = dest.meeting_tab;
        match dest.page {
            nav::Page::Tasks => s.view = tasks::View::Tasks,
            nav::Page::Memory if s.config.features.memory => s.memory_open = true,
            nav::Page::Check if s.config.features.daily_check && s.config.features.memory => {
                s.check_open = true;
            }
            nav::Page::Annotations if s.config.features.annotations => s.annotations.open = true,
            nav::Page::People if s.config.features.people => s.view = tasks::View::People,
            _ => {}
        }
        s.refresh_list();
    }
}

pub fn tick(s: &mut State, _tx: &mut Tx) -> bool {
    let mut changed = false;
    #[cfg(windows)]
    {
        let speech = s.on_hex() && active(s) == (Page::Meetings, 3) && !super::super::isolated();
        if speech {
            _tx.wake_after(Duration::from_secs(1));
        }
        changed |= host::tick(s, speech);
    }
    if s.on_hex()
        && !super::super::isolated()
        && !s.hex.settings_status_pending
        && (s.hex.status.is_none() || s.hex.connection.expired())
        && s.hex
            .settings_status_at
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(10))
    {
        s.hex_check_settings();
        changed = true;
    }
    if s.on_hex() && s.hex.section == Section::Features && s.hex.shell.kb_base.is_none() {
        load_kb(s);
        changed = true;
    }
    if rune::tooling::headless_run().is_none()
        && let Some(version) = s.hex.state().map(|state| state.version.clone())
        && s.hex.shell.labs_version.as_ref() != Some(&version)
        && !s.hex.shell.labs_loading
    {
        s.hex.shell.labs_version = Some(version);
        refresh_labs(s, None);
        changed = true;
    }
    if s.on_hex() && !jump_groups(active(s).0, active(s).1).is_empty() {
        let offset = s.hex.shell.jump_offset.get();
        let origin = s.hex.shell.jump_origin;
        let max = s.hex.shell.jump_max.get();
        let at_bottom = max > 0.0 && offset + 1.0 >= max;
        // A clamped scroll can leave several headings visible. Keep the clicked section selected.
        let requested = s
            .hex
            .shell
            .jump_requested
            .filter(|_| s.pending_scroll.is_some() || max <= 0.0 || at_bottom);
        s.hex.shell.jump_requested = requested;
        let selected = if let Some(requested) = requested {
            requested
        } else if at_bottom {
            s.hex
                .shell
                .jump_positions
                .keys()
                .next_back()
                .copied()
                .unwrap_or(0)
        } else {
            s.hex
                .shell
                .jump_positions
                .iter()
                .filter(|(_, y)| **y - origin <= offset + 24.0)
                .map(|(i, _)| *i)
                .next_back()
                .unwrap_or(0)
        };
        if selected != s.hex.shell.jump_active {
            s.hex.shell.jump_active = selected;
            changed = true;
        }
    }
    if s.on_hex() {
        let (page, tab) = active(s);
        if s.hex.shell.last_tabs.get(page.name()) != Some(&tab) {
            s.hex.shell.last_tabs.insert(page.name().into(), tab);
            if rune::tooling::headless_run().is_none() {
                let mut prefs = UiPrefs::load(&s.store.root);
                prefs.settings_tabs = s.hex.shell.last_tabs.clone();
                let _ = prefs.save(&s.store.root);
            }
        }
    }
    if s.hex
        .shell
        .highlighted
        .as_ref()
        .is_some_and(|(_, until)| highlight_now(s) >= *until)
    {
        s.hex.shell.highlighted = None;
        changed = true;
    }
    if s.hex
        .shell
        .saved
        .is_some_and(|until| Instant::now() >= until)
    {
        s.hex.shell.saved = None;
        s.ai_notes.notice = None;
        changed = true;
    }
    changed
}
pub(crate) fn highlight_now(s: &State) -> Instant {
    // Headless deadlines follow Rune's virtual clock; live input keeps its wall-clock timing.
    s.hex
        .shell
        .highlight_clock
        .map_or_else(Instant::now, |(start, clock)| {
            start + Duration::from_secs_f64(clock)
        })
}
pub(crate) fn highlight(s: &State, key: &str, p: Palette) -> Color {
    if s.hex
        .shell
        .highlighted
        .as_ref()
        .is_some_and(|(id, until)| id == key && highlight_now(s) < *until)
    {
        p.hit
    } else {
        p.card
    }
}
pub(crate) fn group_highlight(s: &State, key: &str, p: Palette) -> Color {
    if s.hex
        .shell
        .highlighted
        .as_ref()
        .is_some_and(|(id, until)| id == key && highlight_now(s) < *until)
    {
        p.hit
    } else {
        p.bg.alpha(0.0)
    }
}
pub(crate) use crate::ui::cards::panel;

pub(super) fn compact_rows(s: &State) -> bool {
    !matches!(active(s).0, Page::General | Page::Appearance)
}

pub(crate) fn row_field_kit(k: &Kit, p: Palette) -> Kit {
    let mut kit = k.clone();
    kit.radius.md = 7.0;
    kit.ty.ui_compact = roles::CONTROL.style();
    kit.c.text_base = p.ink;
    kit.c.icon_muted = p.faint;
    kit.glyph.chevron_down = &icons::SELECT_DOWN;
    kit.glyph.chevron_up = &icons::SELECT_UP;
    kit
}

pub(crate) fn short_select_width(s: &State, wide: f32) -> f32 {
    let width = s.app_w.get();
    if width < DRAWER_AT {
        wide.min((width - 240.0).clamp(120.0, 152.0))
    } else {
        wide
    }
}

pub(crate) fn row_control(s: &State, control: El) -> El {
    if s.app_w.get() < DRAWER_AT {
        control.max_w((s.app_w.get() - 64.0).max(0.0))
    } else {
        control
    }
}

fn ownership_text(page: Page) -> &'static str {
    if page == Page::Hex {
        "These settings live in Hex and also change Hex."
    } else {
        "Settings labeled Hex also change Hex."
    }
}
fn ownership(page: Page, p: Palette) -> El {
    row()
        .w_full()
        .items_center()
        .gap(6.0)
        .mt(10.0)
        .children((page == Page::Hex).then(|| icons::draw(&icons::HEX_MARK, 14.0, p.soft)))
        .child(roles::SMALL.wrap(ownership_text(page), p.soft))
}
pub(crate) fn row_icon(key: &str) -> &'static rune::Icon {
    match key.trim_start_matches("hs-") {
        "relay-preview" => &icons::REFRESH_CW,
        "relay-address" => &icons::NETWORK,
        "relay-last-contact" => &icons::CLOCK,
        key if key.starts_with("relay-paired-device-") => &icons::DEVICE,
        "ask-agent-row" | "ask-meeting-agent-row" => &icons::BOT,
        "ask-model-row" => &icons::CPU,
        "ask-meeting-thinking-row" => &icons::TIMER,
        "ask-chat-thinking-row" => &icons::MESSAGE_SQUARE,
        key if key.starts_with("hex-call-app-") => {
            if key.contains("Teams") {
                &icons::PEOPLE
            } else if key.contains("Meet") {
                &icons::GLOBE
            } else {
                &icons::PHONE_CALL
            }
        }
        "meetings.auto_start" => &icons::PHONE_CALL,
        "meetings.record_my_microphone" => &icons::MIC,
        "languages-row" | "meeting-language-mode" => &icons::LANGUAGES,
        "vocabulary-row" => &icons::SPELLING,
        "feature-Memory" => &icons::BRAIN,
        "keep-audio" => &icons::AUDIO_LINES,
        "meetings.retention" => &icons::TRASH,
        "meetings.silence_threshold" => &icons::VOLUME_X,
        "meetings.output_folder" | "sync-folder" => &icons::FOLDER,
        "notes-identity" => &icons::USER,
        "summary-template" => &icons::FILE_TEXT,
        "notes-options" => &icons::SUMMARY,
        "notes-privacy" => &icons::SHIELD_CHECK,
        "dictation.shortcut" => &icons::KEYBOARD,
        "shortcuts.toggle_meeting" => &icons::CIRCLE_STOP,
        "dictation.remove_filler_words" => &icons::ERASER,
        "shortcuts.paste_last" => &icons::CLIPBOARD_PASTE,
        "dictation.max_seconds" => &icons::TIMER,
        "dictation.paste_method" => &icons::CLIPBOARD,
        "dictation.language" => &icons::SPEECH,
        "general.theme" => &icons::PALETTE,
        "general.launch_at_login" => &icons::POWER,
        "general.sound_effects" => &icons::VOLUME_2,
        "labs.meeting_finished_event" => &icons::FLASK,
        "labs.live_captions" => &icons::CAPTIONS,
        "labs.gpu_overlay" => &icons::CPU,
        "cloud.api_key" => &icons::KEY_ROUND,
        "about.build" => &icons::PACKAGE,
        "diagnostics.log_level" => &icons::FILE_CLOCK,
        "network.proxy" => &icons::GLOBE,
        "hex-status" => &icons::AUDIO_WAVEFORM,
        "feature-Daily check" => &icons::CALENDAR_CHECK,
        "feature-Tasks" => &icons::LIST_TODO,
        "feature-Projects" => &icons::FOLDERS,
        "feature-People" => &icons::PEOPLE,
        "feature-Annotations and pictures" => &icons::ANNOTATIONS,
        "feature-Hindi and Telugu engine" => &icons::CPU,
        "feature-Translation" => &icons::LANGUAGES,
        "feature-Other devices" => &icons::RADIO_TOWER,
        "this-pc-name" | "meeting-icon-visibility" => &icons::MONITOR,
        "connect-Claude Code" => &icons::CONNECTED,
        "connect-Claude Desktop" => &icons::MONITOR,
        "connect-Codex" => &icons::APP_WINDOW,
        "connect-Cursor" => &icons::MOUSE_POINTER,
        "connect-OpenCode" => &icons::NETWORK,
        "notes-helper-0" => &icons::TERMINAL,
        "notes-helper-1" => &icons::CODE_XML,
        "notes-helper-2" => &icons::SQUARE_TERMINAL,
        key if key.starts_with("sync-device-") => &icons::SMARTPHONE,
        "update-version" => &icons::REFRESH_CW,
        "meetings.providers" | "labs.meetings" | "meetings.enabled" => &icons::PHONE_CALL,
        "meetings.summaries" => &icons::FILE_TEXT,
        "meetings.opencode_model" => &icons::CPU,
        "meetings.show_recording_indicator" | "meetings.recording_indicator_corner" => {
            &icons::MONITOR
        }
        _ => &icons::SLIDERS,
    }
}
pub(crate) fn setting_row(
    s: &State,
    key: &str,
    name: &str,
    help: &str,
    control: El,
    _local: bool,
    p: Palette,
) -> El {
    if compact_rows(s) {
        let copy = cards::settings_copy(name, p)
            .children((!help.is_empty()).then(|| roles::META.wrap(help, p.soft)));
        return cards::settings_row(!help.is_empty())
            .key(key)
            .fill(group_highlight(s, key, p))
            .child(copy)
            .child(row_control(s, control));
    }
    let title = roles::NAME.wrap(name, p.ink);
    crate::ui::cards::row_shell()
        .key(key)
        .flex_wrap()
        .px(16.0)
        .py(11.0)
        .fill(group_highlight(s, key, p))
        .child(crate::ui::cards::copy(title, help, p))
        .child(control)
}
pub(crate) fn card(title: &str, body: El, p: Palette) -> El {
    titled_card(title, body, p, roles::GROUP, 18.0)
}
fn settings_card(title: &str, body: El, p: Palette) -> El {
    titled_card(title, body, p, roles::SMALL.bold(), roles::SECTION_TOP)
}
fn titled_card(title: &str, body: El, p: Palette, role: roles::Role, top: f32) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .mt(top)
        .child(if title.starts_with("Hex") {
            row()
                .items_center()
                .gap(6.0)
                .child(icons::draw(&icons::HEX_MARK, 14.0, p.soft))
                .child(role.wrap(title, p.soft))
        } else {
            role.wrap(title, p.soft)
        })
        .child(panel(p).child(body.w_full()))
}
fn advanced(s: &State, body: El, p: Palette) -> El {
    let page = active(s).0.name();
    let open = s.hex.shell.advanced.contains(page) || s.hex.section == Section::Advanced;
    let mut out = col().w_full().mt(18.0).child(
        crate::ui::cards::fold_control("Advanced", "settings-advanced", open, p).on_press(
            move |s: &mut State| {
                if !s.hex.shell.advanced.remove(page) {
                    s.hex.shell.advanced.insert(page);
                }
                if s.hex.section == Section::Advanced {
                    s.hex.section = Section::Updates;
                }
            },
        ),
    );
    if open {
        out = out.child(body);
    }
    out
}

#[derive(Clone)]
struct Destination {
    key: String,
    name: String,
    words: String,
    page: Page,
    tab: usize,
    advanced: bool,
}
fn entry(
    key: &str,
    name: &str,
    words: &str,
    page: Page,
    tab: usize,
    advanced: bool,
) -> Destination {
    Destination {
        key: key.into(),
        name: name.into(),
        words: words.into(),
        page,
        tab,
        advanced,
    }
}
fn location(st: &Setting) -> (Page, usize, bool) {
    let key = st.key.as_str();
    match key {
        "meetings.audio_retention"
        | "labs.expire_meeting_audio"
        | "meetings.retention"
        | "meetings.output_folder" => (Page::Privacy, 0, false),
        "meetings.silence_threshold" | "labs.meeting_finished_event" => (Page::Meetings, 0, true),
        "labs.live_captions"
        | "labs.meetings"
        | "meetings.enabled"
        | "meetings.show_recording_indicator"
        | "meetings.recording_indicator_corner"
        | "meetings.providers" => (Page::Meetings, 0, false),
        "meetings.summaries" | "meetings.opencode_model" => (Page::Meetings, 1, false),
        "about.build" | "diagnostics.log_level" | "labs.gpu_overlay" => {
            (Page::Diagnostics, 0, false)
        }
        "cloud.api_key" | "network.proxy" => (Page::Hex, 0, true),
        "sound_effects" | "sound_effect_volume" | "sound_theme" | "ui_language" => {
            (Page::Appearance, 0, false)
        }
        "history_retention"
        | "screenshots.folder"
        | "capture.folder"
        | "capture.media_days"
        | "capture.bundle_days" => (Page::Privacy, 0, false),
        "transcription.model" | "transcription.language" | "vocabulary_boost" => {
            (Page::Hex, 0, false)
        }
        _ if st.key.contains("hotkey") || st.key.contains("shortcut") => (Page::Hex, 0, false),
        _ if !st.key.contains('.') => (Page::Hex, 0, true),
        _ => match place(st) {
            Place::In(Section::Meetings) => (Page::Meetings, 0, false),
            Place::In(Section::General) => (Page::Appearance, 0, false),
            Place::In(Section::Dictation) => (
                Page::Hex,
                0,
                matches!(key, "dictation.max_seconds" | "dictation.paste_method"),
            ),
            Place::In(Section::Shortcuts) => (Page::Hex, 0, false),
            _ => (Page::Diagnostics, 0, false),
        },
    }
}
pub(super) fn setting_name(st: &Setting) -> String {
    match st.key.as_str() {
        "labs.meetings" | "meetings.enabled" => "Enable meeting recording".into(),
        "meetings.providers" => "Call apps".into(),
        "meetings.summaries" => "Hex summaries".into(),
        "meetings.opencode_model" => "OpenCode summary model".into(),
        "meetings.show_recording_indicator" => "Show recording indicator".into(),
        "meetings.recording_indicator_corner" => "Indicator position".into(),
        "dictation.max_seconds" => "Maximum dictation length (seconds)".into(),
        "labs.gpu_overlay" => "GPU recording overlay".into(),
        "cloud.api_key" => "Cloud credential".into(),
        "about.build" => "Hex build".into(),
        "network.proxy" => "Download proxy".into(),
        "diagnostics.log_level" => "Log level".into(),
        "labs.meeting_finished_event" => "Recording-finished notification".into(),
        "meetings.auto_start" => "Start when a call starts".into(),
        "meetings.record_my_microphone" => "Record my microphone".into(),
        "shortcuts.toggle_meeting" => "Start or stop a meeting".into(),
        "shortcuts.paste_last" => "Paste last shortcut".into(),
        "dictation.shortcut" => "Dictate".into(),
        "dictation.language" => "Dictation language".into(),
        "general.launch_at_login" => "Open Hex when you sign in to Windows".into(),
        _ => human(st, place(st)),
    }
}
pub(super) fn setting_help(st: &Setting) -> String {
    match st.key.as_str() {
        "meetings.auto_start" => "Hex starts recording when a supported call starts.".into(),
        "cloud.api_key" => {
            "Optional Hex cloud credential. This value cannot be edited here.".into()
        }
        "meetings.summaries" => {
            "Separate from Sidenote summaries. Turning this on may create a second summary.".into()
        }
        "meetings.opencode_model" => "Used when Hex writes a summary with OpenCode.".into(),
        "meetings.providers" => "Choose what Hex does when a call starts in each app.".into(),
        "labs.gpu_overlay" => "Experimental GPU rendering for the recording overlay.".into(),
        "labs.meetings" | "meetings.enabled" => "Hex records calls for Sidenote.".into(),
        _ if st.about.contains(&st.key)
            || st.about.contains('{')
            || st.about.to_ascii_lowercase().contains("json")
            || st
                .about
                .split_whitespace()
                .any(|word| word.contains('_') || word.trim_end_matches('.').contains('.')) =>
        {
            "Managed by Hex. Changes also apply in Hex.".into()
        }
        _ => one_line(&st.about),
    }
}
fn app_index() -> Vec<Destination> {
    vec![
        entry(
            "labs-stop-forgotten",
            "Stop forgotten recordings",
            "Labs forgotten idle auto stop minutes",
            Page::Diagnostics,
            0,
            false,
        ),
        entry(
            "labs-measure-files",
            "Measure recording files (diagnostics)",
            "Labs probe WAV growing files",
            Page::Diagnostics,
            0,
            false,
        ),
        entry(
            "labs-process-windows",
            "Process meetings in 5-minute windows (test)",
            "Labs evidence audio windows timing backlog",
            Page::Diagnostics,
            0,
            false,
        ),
        entry(
            "notes-identity",
            "Your name",
            "identity profile",
            Page::General,
            0,
            false,
        ),
        entry(
            "notes-options",
            "Write a meeting summary when a meeting ends",
            "AI notes Notes automatic summary",
            Page::Meetings,
            1,
            false,
        ),
        entry(
            "summary-template",
            "Summary template",
            "AI notes Notes instructions prompt",
            Page::Meetings,
            1,
            false,
        ),
        entry(
            "notes-scripts-card",
            "Summaries from your own script",
            "AI notes scripts engine",
            Page::Meetings,
            1,
            true,
        ),
        entry(
            "knowledge-base",
            "Knowledge base",
            "daily check repository revision source folders company roster",
            Page::Meetings,
            2,
            false,
        ),
        entry(
            "feature-Daily check",
            "Daily check",
            "tasks",
            Page::Meetings,
            2,
            false,
        ),
        entry(
            "engine-notes-writer",
            "Notes writer",
            "meeting engine notes summary agent helper writer",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-translation-model",
            "Translation model",
            "meeting engine translate english model",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-title-model",
            "Title model",
            "meeting engine meeting title model",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-tasks-model",
            "Task model",
            "meeting engine tasks follow-ups model",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-comparison-model",
            "Comparison model",
            "meeting engine compare outside notes model",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-prompt-notes",
            "Notes prompt",
            "meeting engine prompt instructions notes summary reset default",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-prompt-translation",
            "Translation prompt",
            "meeting engine prompt instructions translation reset default",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-step-tasks",
            "Tasks step",
            "meeting engine optional step tasks follow-ups on off",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-step-title",
            "Meeting title step",
            "meeting engine optional step title on off",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-step-comparison",
            "Comparison step",
            "meeting engine optional step compare outside notes on off",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-import",
            "Import from scripts",
            "meeting engine import migrate script settings",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-model-en",
            "English model",
            "meeting engine transcription speech model english",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-model-hi",
            "Hindi model",
            "meeting engine transcription speech model hindi",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-model-te",
            "Telugu model",
            "meeting engine transcription speech model telugu",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-worker-threads",
            "Parallel transcription jobs",
            "meeting engine transcription worker threads parallel",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-transcription-deadline",
            "Time limit for a full transcript",
            "meeting engine transcription deadline timeout seconds",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "engine-request-deadline",
            "Time limit for one transcription request",
            "meeting engine transcription request deadline timeout seconds",
            Page::Meetings,
            3,
            false,
        ),
        entry(
            "feature-Tasks",
            "Tasks",
            "features",
            Page::General,
            0,
            false,
        ),
        entry(
            "work-tasks-layout",
            "Tasks open as",
            "tasks list board layout view",
            Page::Tasks,
            0,
            false,
        ),
        entry(
            "work-tasks-grouping",
            "Group tasks by",
            "group by status owner due project meeting team",
            Page::Tasks,
            0,
            false,
        ),
        entry(
            "work-tasks-who",
            "Show",
            "mine everyone unassigned filter",
            Page::Tasks,
            0,
            false,
        ),
        entry(
            "agent-instructions",
            "Meeting summary instructions",
            "AI notes Notes prompt library instructions tasks",
            Page::Prompts,
            0,
            false,
        ),
        entry(
            "settings-teams",
            "Teams",
            "Teams people members aliases",
            Page::People,
            0,
            false,
        ),
        entry(
            "people-roster",
            "Our team",
            "People person roster names",
            Page::People,
            0,
            false,
        ),
        entry(
            "people-roster",
            "External",
            "People external company",
            Page::People,
            1,
            false,
        ),
        entry(
            "people-import",
            "How people are added",
            "People import contacts",
            Page::People,
            0,
            false,
        ),
        entry(
            "hex-vocabulary",
            "Names and terms",
            "Vocabulary spelling",
            Page::Meetings,
            0,
            false,
        ),
        entry(
            "spoken-languages",
            "Languages you speak",
            "languages meeting languages",
            Page::Meetings,
            0,
            false,
        ),
        entry(
            "ai-notes-settings",
            "Models and agents",
            "Claude Codex OpenCode AI helpers meeting summary thinking effort",
            Page::Helpers,
            0,
            false,
        ),
        entry(
            "ask-agent-row",
            "Ask agent",
            "Claude Code Codex OpenCode models and agents",
            Page::Helpers,
            0,
            false,
        ),
        entry(
            "ask-model-row",
            "Ask model",
            "models and agents model",
            Page::Helpers,
            0,
            false,
        ),
        entry(
            "ask-meeting-agent-row",
            "Agent during meetings",
            "Codex Claude OpenCode live recording model low medium",
            Page::Helpers,
            0,
            false,
        ),
        entry(
            "ask-meeting-thinking-row",
            "Thinking during meetings",
            "effort low medium high recording",
            Page::Helpers,
            0,
            false,
        ),
        entry(
            "ask-chat-thinking-row",
            "Thinking in chats",
            "effort low medium high",
            Page::Helpers,
            0,
            false,
        ),
        entry(
            "annotation-agent-row",
            "Annotation agent",
            "Annotation notes Claude Code Codex",
            Page::Helpers,
            0,
            false,
        ),
        entry(
            "annotation-model-row",
            "Annotation model",
            "Annotation notes model",
            Page::Helpers,
            0,
            false,
        ),
        entry(
            "mcp-connect",
            "Connected apps",
            "MCP connect Claude Codex Cursor OpenCode Desktop",
            Page::ConnectedApps,
            0,
            false,
        ),
        entry(
            "speech-models",
            "Speech model",
            "Model languages download",
            Page::Helpers,
            1,
            false,
        ),
        entry(
            "notes-model-0",
            "Summary model",
            "Meeting summary model",
            Page::Helpers,
            0,
            false,
        ),
        entry(
            "notes-privacy",
            "What is sent",
            "privacy transcript audio requests",
            Page::Privacy,
            0,
            false,
        ),
        entry(
            "update-source",
            "Update source",
            "update folder diagnostics",
            Page::Diagnostics,
            0,
            false,
        ),
        entry(
            "response-times",
            "Response times",
            "latency timing performance",
            Page::Diagnostics,
            0,
            false,
        ),
        entry(
            "helper-advanced",
            "Agent diagnostics",
            "helper helpers meeting summary model sample sign in safety rules",
            Page::Diagnostics,
            0,
            false,
        ),
        entry(
            "meeting-language-mode",
            "Language handling",
            "routing pipeline",
            Page::Helpers,
            1,
            true,
        ),
        entry(
            "speech-model-anuvani-lid-int8",
            "Anuvani language identification",
            "language identification detection",
            Page::Helpers,
            1,
            true,
        ),
        entry(
            "legacy-hex-models",
            "Hex dictation model",
            "model",
            Page::Hex,
            0,
            false,
        ),
        entry(
            "feature-Other devices",
            "Other devices",
            "sync",
            Page::Sync,
            0,
            false,
        ),
        entry(
            "sync-folder",
            "Sync folder",
            "data directory",
            Page::Sync,
            0,
            true,
        ),
        entry(
            "updates-section",
            "Check for updates",
            "version what changed",
            Page::Updates,
            0,
            false,
        ),
    ]
}
const MEETING_ICON_CAPTURE: &str = "meeting-icon-capture";
const MEETING_ICON_CAPTURE_LABEL: &str = "Show the meeting icon in screenshots and recordings";

fn meeting_icon_capture_destination() -> Destination {
    entry(
        MEETING_ICON_CAPTURE,
        MEETING_ICON_CAPTURE_LABEL,
        "floating indicator capture",
        Page::General,
        0,
        false,
    )
}

fn index(s: &State) -> Vec<Destination> {
    index_for_channel(s, CHANNEL)
}

fn index_for_channel(s: &State, channel: Channel) -> Vec<Destination> {
    let mut entries = app_index();
    entries.extend(AppFeature::EXPERIMENTAL.map(|feature| {
        entry(
            &format!("feature-{}", feature.name()),
            feature.name(),
            feature.description(),
            Page::Experimental,
            0,
            false,
        )
    }));
    if cfg!(windows) {
        entries.push(meeting_icon_capture_destination());
    }
    entries.push(entry(
        "relay-preview",
        "Relay sync (preview)",
        "Labs experimental tasks",
        Page::Sync,
        0,
        true,
    ));
    if s.hex.shell.relay.enabled {
        entries.push(entry(
            "relay-address",
            "Relay address",
            "sync endpoint server",
            Page::Sync,
            0,
            true,
        ));
    }
    entries.retain(|dest| {
        let required = match dest.key.as_str() {
            "annotation-agent-row" | "annotation-model-row" => Some(AppFeature::Annotations),
            "engine-model-hi"
            | "engine-model-te"
            | "spoken-languages"
            | "meeting-language-mode"
            | "speech-model-anuvani-lid-int8"
            | "labs-process-windows" => Some(AppFeature::IndicEngine),
            "engine-translation-model" | "engine-prompt-translation" => {
                Some(AppFeature::Translation)
            }
            _ => None,
        };
        (dest.page != Page::People || s.config.features.people)
            && tab_visible(s, dest.page, dest.tab)
            && required.is_none_or(|feature| s.config.features.enabled(feature))
            && (dest.key != "speech-model-anuvani-lid-int8"
                || s.models.has_local_files("anuvani-lid-int8"))
            && (dest.key != "notes-scripts-card"
                || s.ai_notes
                    .settings
                    .as_ref()
                    .is_some_and(|settings| settings.script.is_some()))
    });
    entries.extend(
        s.hex
            .settings
            .iter()
            .filter(|st| !readable::windows_only_hidden(st))
            .map(|st| {
                let (page, tab, advanced) = location(st);
                let key = if matches!(
                    st.key.as_str(),
                    "meetings.audio_retention" | "labs.expire_meeting_audio"
                ) {
                    "keep-audio".into()
                } else {
                    format!("hs-{}", st.key)
                };
                entry(
                    &key,
                    &if key == "keep-audio" {
                        "Keep audio".into()
                    } else {
                        setting_name(st)
                    },
                    &format!("{} {}", st.key, st.about),
                    page,
                    tab,
                    advanced,
                )
            }),
    );
    let mut seen = HashSet::new();
    entries.retain(|dest| {
        dest.page.visible(channel)
            && dest.page.shown(s)
            && seen.insert((dest.key.clone(), dest.page.name(), dest.tab))
    });
    for alias in legacy_aliases() {
        if let Some(dest) = entries.iter_mut().find(|d| d.key == alias.target) {
            dest.words.push(' ');
            dest.words.push_str(&alias.words);
        }
    }
    entries
}
struct LegacyAlias {
    target: String,
    words: String,
}
fn legacy_aliases() -> &'static [LegacyAlias] {
    static ALIASES: std::sync::LazyLock<Vec<LegacyAlias>> = std::sync::LazyLock::new(|| {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tools/ui-fixtures/settings-search-map.json"
        ))
        .expect("Settings search map");
        rows.as_array()
            .expect("Settings search rows")
            .iter()
            .map(|row| LegacyAlias {
                target: row["target"].as_str().expect("Settings target").into(),
                words: row["search_aliases"]
                    .as_array()
                    .expect("Settings aliases")
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            })
            .collect()
    });
    &ALIASES
}
fn matches(item: &Destination, query: &str) -> bool {
    let hay = format!("{} {} {}", item.name, item.words, item.page.name()).to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| hay.contains(word))
}
fn results(s: &State) -> Vec<Destination> {
    let words = s.hex.filter.text.to_lowercase();
    index(s)
        .into_iter()
        .filter(|item| matches(item, &words))
        .collect()
}
fn jump(s: &mut State, dest: Destination) {
    navigate(s, dest.page, dest.tab);
    if dest.advanced {
        s.hex.shell.advanced.insert(dest.page.name());
    }
    if matches!(
        dest.key.as_str(),
        "hs-meetings.summaries" | "hs-meetings.opencode_model"
    ) {
        s.hex.open_groups.insert("hex-summaries");
    }
    if matches!(dest.key.as_str(), "ai-notes-settings" | "notes-model-0") {
        s.ai_notes.model_open = true;
    }
    if dest.key == "settings-teams" {
        let team = s.tasks.teams.teams.first().cloned();
        super::super::teams::open(s, team);
        if let Some(editor) = &mut s.people.team_editor {
            editor.advanced = true;
        }
    }
    if dest.key == "spoken-languages" {
        s.hex.open_groups.insert("languages");
    }
    if dest.key == "knowledge-base" {
        s.hex.shell.kb_open = true;
    }
    if dest.key == "hex-vocabulary" {
        s.hex.open_groups.insert("vocabulary-edit");
    }
    s.hex.shell.highlighted = Some((dest.key.clone(), highlight_now(s) + Duration::from_secs(2)));
    super::super::scene_scroll(s, &dest.key);
}

pub fn sidebar(s: &State, k: &Kit, p: Palette) -> El {
    if !nav_docked(s) {
        return col().key("settings-sidebar").w(0.0).h_full();
    }
    let width = if s.app_w.get() < 700.0 {
        200.0
    } else {
        LIST_WIDE
    };
    settings_list(s, k, p, width, true)
}

fn drawer_open(s: &State) -> bool {
    s.app_w.get() < DRAWER_AT && (s.hex.shell.navigation_open || s.hex.filter.focused)
}

/// The settings list sits beside the page and carries the sidebar toggle in its own top row.
pub fn nav_docked(s: &State) -> bool {
    s.app_w.get() >= DRAWER_AT && !s.hex.shell.nav_hidden
}

pub fn nav_open(s: &State) -> bool {
    nav_docked(s) || drawer_open(s)
}

/// Ctrl+\ and the top-left toggle: fold the list, or open and close the narrow drawer.
pub fn toggle_nav(s: &mut State) {
    if s.app_w.get() >= DRAWER_AT {
        s.hex.shell.nav_hidden = !s.hex.shell.nav_hidden;
    } else if drawer_open(s) {
        close_drawer(s);
    } else {
        s.hex.shell.navigation_open = true;
    }
}

fn close_drawer(s: &mut State) {
    s.hex.shell.navigation_open = false;
    s.hex.filter.focused = false;
}

fn list_row(glyph: &'static rune::Icon, name: &str, active: bool, p: Palette) -> El {
    let ink = if active { p.ink } else { p.soft };
    pressable(
        row()
            .w_full()
            .h(LIST_ROW_H)
            .items_center()
            .gap(10.0)
            .px(8.0)
            .child(icons::draw(glyph, icons::ROW, ink))
            .child(roles::NAV.line(name, ink).grow(1.0).help(name)),
    )
    .radius(7.0)
    .fill(if active { p.sel } else { p.side })
    .hover(p.chip)
    .focus_fill(p.chip)
    .focus_ring(1.0, 0.0, p.line)
}

fn settings_back(p: Palette) -> El {
    list_row(&icons::CHEVRON_LEFT, "Back", false, p)
        .label("Back (Esc)")
        .on_press(back)
}

fn settings_list(s: &State, k: &Kit, p: Palette, width: f32, docked: bool) -> El {
    let mut list = col()
        .w_full()
        .gap(1.0)
        .px(8.0)
        .pt(12.0)
        .pb(8.0)
        .child(settings_back(p).key("settings-back"))
        .child(
            TextInput::new(&s.hex.filter)
                .leading_icon(&icons::SEARCH)
                .leading_icon_size(icons::ROW)
                .trailing(
                    roles::META
                        .line("/", p.faint)
                        .px(6.0)
                        .radius(5.0)
                        .border(1.0, p.line),
                )
                .placeholder("Search settings")
                .label("Search settings")
                .height(30.0)
                .plain()
                .on_edit(|s: &mut State| &mut s.hex.filter)
                .on_submit(|s: &mut State, _| {
                    if let Some(dest) = results(s).into_iter().next() {
                        jump(s, dest);
                    }
                })
                .build(k)
                .key("hex-filter")
                .w_full()
                .radius(8.0)
                .fill(p.chip)
                .outline(if s.hex.filter.focused { 1.0 } else { 0.0 }, -1.0, p.faint)
                .mt(4.0)
                .mb(8.0),
        );
    if !s.hex.filter.text.trim().is_empty() {
        let matches = results(s);
        if matches.is_empty() {
            list = list.child(roles::SMALL.wrap(
                format!(
                    "No setting matches \"{}\". Try a shorter word.",
                    s.hex.filter.text
                ),
                p.soft,
            ));
        }
        for dest in matches {
            let path = path(dest.page, dest.tab);
            list = list.child(
                pressable(
                    col()
                        .w_full()
                        .gap(4.0)
                        .padding(8.0)
                        .child(roles::NAME.wrap(&dest.name, p.ink))
                        .child(roles::META.wrap(path, p.soft)),
                )
                .key(format!("find-setting-{}", dest.key))
                .radius(6.0)
                .hover(p.chip)
                .focus_ring(1.0, 0.0, p.line)
                .on_press(move |s: &mut State| jump(s, dest.clone())),
            );
        }
    } else {
        let (active, _) = active(s);
        let mut previous_group: Option<&'static str> = None;
        for page in Page::ALL
            .into_iter()
            .filter(|page| page.visible(CHANNEL) && page.shown(s))
        {
            let group = page.group();
            if previous_group != Some(group) {
                let slug = group.to_ascii_lowercase().replace(' ', "-");
                list = list.child(
                    row()
                        .h(28.0)
                        .px(8.0)
                        .items_center()
                        .mt(12.0)
                        .child(roles::META.line(group, p.faint))
                        .key(format!("settings-heading-{slug}")),
                );
                previous_group = Some(group);
            }
            let button = list_row(page.icon(), page.name(), active == page, p)
                .key(format!("settings-page-{}", page.name()))
                .label(page.name())
                .help(page.name())
                .on_press(move |s: &mut State| {
                    let tab = s
                        .hex
                        .shell
                        .last_tabs
                        .get(page.name())
                        .or_else(|| {
                            (page == Page::Helpers)
                                .then(|| s.hex.shell.last_tabs.get("AI helpers"))
                                .flatten()
                        })
                        .copied()
                        .unwrap_or(0);
                    navigate(s, page, tab);
                });
            list = list.child(button);
        }
    }
    col()
        .key("settings-sidebar")
        .w(width)
        .h_full()
        .min_h(0.0)
        .fill(p.side)
        .children(docked.then(|| {
            row()
                .h(48.0)
                .pl(14.0)
                .gap(2.0)
                .items_center()
                .child(nav::collapse(p))
                .child(nav::search(p))
        }))
        .child(scroll(list, "settings-list-scroll"))
}
fn path(page: Page, tab: usize) -> String {
    page.tabs().get(tab).map_or_else(
        || page.name().into(),
        |name| format!("{} / {name}", page.name()),
    )
}
pub(in crate::ui) fn tab_bar(
    s: &State,
    p: Palette,
    jumps: bool,
    page: Page,
    selected: usize,
) -> El {
    let active = if jumps {
        s.hex.shell.jump_active
    } else {
        selected
    };
    let names = if jumps {
        jump_groups(page, selected)
    } else {
        page.tabs()
    };
    let people = page == Page::People;
    let mut tabs = row()
        .key(if jumps {
            "settings-jump-tabs"
        } else {
            "settings-tabs"
        })
        .role("tablist")
        .label(format!("{} sections", page.name()))
        .w_full()
        .gap(if people { 20.0 } else { 4.0 })
        .mt(if people { 0.0 } else { 16.0 })
        .flex_wrap()
        .focus_group();
    for (tab, name) in names.iter().enumerate() {
        if !jumps && !tab_visible(s, page, tab) {
            continue;
        }
        let selected = tab == active;
        let ink = if selected { p.ink } else { p.soft };
        tabs = tabs.child(
            pressable(if people {
                col()
                    .gap(12.0)
                    .pt(14.0)
                    .child(if selected {
                        roles::CONTROL.bold().wrap(*name, ink)
                    } else {
                        roles::CONTROL.wrap(*name, ink)
                    })
                    .child(
                        rune::el()
                            .w_full()
                            .h(2.0)
                            .fill(if selected { p.ink } else { p.bg }),
                    )
            } else {
                row()
                    .h(TAB_H)
                    .px(10.0)
                    .items_center()
                    .radius(7.0)
                    .fill(if selected { p.sel } else { Color::TRANSPARENT })
                    .hover(p.chip)
                    .focus_ring(1.0, 0.0, p.line)
                    .child(roles::CONTROL.line(*name, ink))
            })
            .key(format!(
                "settings-{}-{tab}",
                if jumps { "jump" } else { "tab" }
            ))
            .role("tab")
            .focus_selected(selected)
            .label(*name)
            .on_press(move |s: &mut State| {
                if jumps {
                    s.hex.shell.jump_requested = Some(tab);
                    s.hex.shell.jump_active = tab;
                    if page == Page::Hex && (4..=5).contains(&tab) {
                        s.hex.open_groups.insert(if tab == 4 {
                            "hex-compatibility"
                        } else {
                            "hex-connection"
                        });
                    }
                    super::super::scene_scroll(s, &format!("settings-group-{tab}"));
                } else {
                    navigate(s, page, tab);
                }
            }),
        );
    }
    tabs
}
fn hex_rows(
    s: &State,
    k: &Kit,
    p: Palette,
    layer: &mut Vec<El>,
    page: Page,
    tab: usize,
    folded: bool,
) -> El {
    let mut items: Vec<_> = s
        .hex
        .settings
        .iter()
        .filter(|st| !readable::windows_only_hidden(st) && location(st) == (page, tab, folded))
        .filter(|st| {
            !matches!(
                st.key.as_str(),
                "meetings.summaries" | "meetings.opencode_model" | "meetings.providers"
            )
        })
        .collect();
    items.sort_by_key(|st| match st.key.as_str() {
        "meetings.enabled" | "labs.meetings" => 0,
        "meetings.auto_start" => 1,
        "meetings.record_my_microphone" => 2,
        "meetings.show_recording_indicator" => 3,
        "meetings.recording_indicator_corner" => 4,
        "labs.live_captions" => 5,
        _ => 6,
    });
    let compact = compact_rows(s);
    let has_row = items.iter().any(|st| {
        matches!(
            st.key.as_str(),
            "meetings.audio_retention" | "labs.expire_meeting_audio"
        )
    });
    let mut body = col().w_full();
    if has_row {
        body = body
            .child(
                keep_audio_row(s, k, p, layer)
                    .key("keep-audio")
                    .fill(highlight(s, "keep-audio", p)),
            )
            .children((!compact).then(|| hairline(p)));
    }
    for st in items {
        if matches!(
            st.key.as_str(),
            "meetings.audio_retention" | "labs.expire_meeting_audio"
        ) {
            continue;
        }
        body = body
            .child(super::setting_row(s, k, st, place(st), p, layer))
            .children((!compact).then(|| hairline(p)));
    }
    body
}
fn feature(s: &State, k: &Kit, feature: AppFeature, name: &str, help: &str, p: Palette) -> El {
    if feature == AppFeature::Translation
        && s.config
            .engine_settings
            .stages
            .translation_enabled_at_ms
            .is_none()
    {
        return setting_row(
            s,
            "feature-Translation",
            name,
            "Requires a qualified translation provider.",
            roles::META.line("Unavailable", p.soft),
            true,
            p,
        );
    }
    setting_row(
        s,
        &format!("feature-{}", feature.name()),
        name,
        help,
        Switch::new(s.config.features.enabled(feature))
            .label(name)
            .hide_label()
            .on_toggle(move |s: &mut State| s.toggle_feature(feature))
            .build(k),
        !matches!(feature, AppFeature::OtherDevices),
        p,
    )
}
fn save_labs(s: &mut State, edit: impl FnOnce(&mut sidenote::core::meeting_watch::Labs)) {
    let result = sidenote::core::Config::load_in(&s.store.root).and_then(|mut config| {
        edit(&mut config.labs);
        config.save(&s.store.root)?;
        Ok(config)
    });
    match result {
        Ok(config) => {
            s.config = config;
            s.feature_error = None;
            s.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
        }
        Err(_) => s.feature_error = Some("Could not save Labs settings. Try again.".into()),
    }
}
fn labs(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let minutes = s.config.labs.silence_minutes.minutes();
    let options = [10, 20, 30, 60].map(|n| format!("{n} minutes"));
    let current = format!("{minutes} minutes");
    let duration = Select::new("labs-silence-minutes", &options)
        .label("Stop after no speech for")
        .current(Some(current.as_str()))
        .appearance(SelectAppearance::Field)
        .width(short_select_width(s, 180.0))
        .open(s.hex.select_open.as_deref() == Some("labs-silence-minutes"))
        .on_toggle(|s: &mut State| {
            s.hex.select_open = if s.hex.select_open.is_some() {
                None
            } else {
                Some("labs-silence-minutes".into())
            };
        })
        .on_close(|s: &mut State| s.hex.select_open = None)
        .on_choose(|s: &mut State, value: &str, _: &mut Tx| {
            use sidenote::core::meeting_watch::SilenceMinutes;
            let minutes = match value {
                "10 minutes" => SilenceMinutes::Ten,
                "30 minutes" => SilenceMinutes::Thirty,
                "60 minutes" => SilenceMinutes::Sixty,
                _ => SilenceMinutes::Twenty,
            };
            save_labs(s, |labs| labs.silence_minutes = minutes);
            s.hex.select_open = None;
        })
        .build(&row_field_kit(k, p), layer);
    settings_card("Labs", col().w_full()
        .children(s.config.features.indic_engine.then(|| setting_row(s, "labs-language-detection", "Language detection (experimental)",
            "Enable the standalone prepared-audio language stage. Writes advisory guesses only; transcription stays unchanged. Confidence thresholds are uncalibrated.",
            Switch::new(s.config.labs.language_detection)
                .label("Language detection (experimental)").hide_label()
                .on_toggle(|s: &mut State| save_labs(s, |labs| labs.language_detection = !labs.language_detection)).build(k).key("labs-language-detection-toggle"), true, p)))
        .child(setting_row(s, "labs-stop-forgotten", "Stop forgotten recordings",
            &format!("If a recording hears no speech for {minutes} minutes, Sidenote stops it so it doesn't run for hours."),
            Switch::new(s.config.labs.stop_forgotten_recordings)
                .label("Stop forgotten recordings").hide_label()
                .on_toggle(|s: &mut State| save_labs(s, |labs| labs.stop_forgotten_recordings = !labs.stop_forgotten_recordings)).build(k).key("labs-stop-forgotten-toggle"), true, p))
        .child(setting_row(s, "labs-silence-duration", "Stop after no speech for", "Choose 10, 20, 30 or 60 minutes.", duration, false, p))
        .child(setting_row(s, "labs-measure-files", "Measure recording files (diagnostics)",
            "Save file sizes, WAV header details and transcript line counts during recordings. No audio or text is saved.",
            Switch::new(s.config.labs.measure_recording_files)
                .label("Measure recording files (diagnostics)").hide_label()
                .on_toggle(|s: &mut State| save_labs(s, |labs| labs.measure_recording_files = !labs.measure_recording_files)).build(k).key("labs-measure-files-toggle"), true, p))
        .children(s.config.features.indic_engine.then(|| setting_row(s, "labs-process-windows", "Process meetings in 5-minute windows (test)",
            "Test calls only. Runs Telugu and Hindi models while recording and saves audio windows, results and timings in Sidenote. May slow live transcription. Your meeting notes stay as they are.",
            Switch::new(s.config.labs.process_meeting_windows)
                .label("Process meetings in 5-minute windows (test)").hide_label()
                .on_toggle(|s: &mut State| save_labs(s, |labs| labs.process_meeting_windows = !labs.process_meeting_windows)).build(k).key("labs-process-windows-toggle"), true, p))), p)
}
fn save_device_name(s: &mut State) {
    s.hex.shell.device_name_error = None;
    if s.relay_ui.pairing_busy() {
        s.hex.shell.device_name_error =
            Some("Finish or cancel pairing before renaming this device.".into());
        return;
    }
    let Some(name) = sidenote::core::DeviceSettings::valid_name(&s.hex.shell.device_name.text)
    else {
        s.hex.shell.device_name_error =
            Some("Use 1 to 80 characters, with no control characters.".into());
        return;
    };
    match sidenote::core::relay_preview::pairing::rename_device(&s.store.root, &name) {
        Ok(()) => {
            s.hex.shell.device_name = TextEdit::new(&name);
            s.relay_refresh();
            s.request_refresh();
            let tail = if s.hex.shell.relay.enabled {
                " Paired devices show it after their next sync."
            } else {
                ""
            };
            s.toast = Some((format!("Name saved.{tail}"), Instant::now()));
        }
        Err(_) => {
            s.hex.shell.device_name_error = Some("Could not save the name. Try again.".into())
        }
    }
}
/// The date is when the newest of the device's meetings was recorded, not when it arrived.
fn folder_device_line(count: usize, latest: Option<chrono::DateTime<chrono::Local>>) -> String {
    let latest = latest
        .map(|t| format!(" Latest meeting recorded {}.", t.format("%-d %b %Y")))
        .unwrap_or_default();
    format!("{count} meetings received.{latest}")
}
fn devices(s: &State, k: &Kit, p: Palette) -> El {
    let help = if s.relay_ui.pairing_busy() {
        "Finish or cancel pairing first."
    } else {
        "Other devices show this name. Press Enter to save."
    };
    let mut out = col().w_full().child(setting_row(
        s,
        "this-pc-name",
        "This device's name",
        help,
        TextInput::new(&s.hex.shell.device_name)
            .height(30.0)
            .label("This device's name")
            .on_edit(|s: &mut State| &mut s.hex.shell.device_name)
            .on_submit(|s: &mut State, _| save_device_name(s))
            .build(&row_field_kit(k, p))
            .key("device-name-input")
            .w(220.0),
        false,
        p,
    ));
    if let Some(error) = &s.hex.shell.device_name_error {
        out = out.child(
            roles::SMALL
                .wrap(error, p.warn)
                .padding(14.0)
                .role("alert")
                .key("device-name-error"),
        );
    }
    let mut peers = BTreeMap::<&str, (usize, Option<chrono::DateTime<chrono::Local>>)>::new();
    for meeting in &s.meetings {
        if let Some(device) = meeting.device.as_deref() {
            let entry = peers.entry(device).or_default();
            entry.0 += 1;
            entry.1 = entry.1.max(meeting.started);
        }
    }
    for (device, (count, last)) in peers {
        out = out.child(setting_row(
            s,
            &format!("sync-device-{device}"),
            device,
            &folder_device_line(count, last),
            roles::CONTROL.line("Received", p.soft),
            false,
            p,
        ));
    }
    out.child(
        roles::SMALL
            .wrap(
                "Folder sync lists the meetings each device recorded. It does not show whether a device is online.",
                p.soft,
            )
            .padding(14.0)
            .key("sync-devices-no-online"),
    )
}
fn relay_labs(s: &State, k: &Kit, p: Palette) -> El {
    setting_row(
        s,
        "relay-preview",
        "Relay sync (preview)",
        "Tasks, teams, People labels, projects, Memory and your name use the relay. A backup stays on this PC.\nA lost or stolen device can't be removed in this preview. If that happens, turn off relay sync on all devices.",
        Switch::new(s.hex.shell.relay.enabled)
            .label("Relay sync (preview)")
            .hide_label()
            .on_toggle(|s: &mut State| {
                let mut settings = s.hex.shell.relay.clone();
                settings.enabled = !settings.enabled;
                if !settings.enabled {
                    s.relay_cancel();
                }
                match settings.save(&s.store.root) {
                    Ok(()) => {
                        s.hex.shell.relay =
                            sidenote::core::relay_preview::Settings::load(&s.store.root)
                                .unwrap_or(settings);
                        s.relay_refresh();
                        s.request_refresh();
                        s.hex.shell.relay_error = None;
                        s.toast = Some((
                            "Relay preview changed. Your folder data is kept.".into(),
                            Instant::now(),
                        ));
                    }
                    Err(error) => {
                        s.feature_error = Some(
                            sidenote::core::validation_message(&error)
                                .unwrap_or("Could not save relay preview settings.")
                                .into(),
                        );
                    }
                }
            })
            .build(k)
            .key("relay-preview-toggle"),
        true,
        p,
    )
}
fn relay_address(s: &State, k: &Kit, p: Palette) -> El {
    if !s.hex.shell.relay.enabled {
        return col();
    }
    let mut out = col().w_full().child(setting_row(
        s,
        "relay-address",
        "Relay address",
        "Your relay's address, like wss://relay.example.com. Press Enter to save.",
        TextInput::new(&s.hex.shell.relay_address)
            .height(30.0)
            .label("Relay address")
            .placeholder("wss://relay.example.com")
            .on_edit(|s: &mut State| &mut s.hex.shell.relay_address)
            .on_submit(|s: &mut State, _| {
                let mut settings = s.hex.shell.relay.clone();
                settings.address = s.hex.shell.relay_address.text.trim().into();
                match settings.save(&s.store.root) {
                    Ok(()) => {
                        s.relay_cancel();
                        s.relay_refresh();
                        let connected = sidenote::core::relay_preview::get(&s.store.root)
                            .and_then(|preview| preview.map(|p| p.online(true)).unwrap_or(Ok(())));
                        s.hex.shell.relay_error = connected.err().map(|_| {
                            "Address saved. Restart Sidenote if sync does not reconnect.".into()
                        });
                    }
                    Err(_) => {
                        s.hex.shell.relay_error =
                            Some("Use a WSS origin on port 443 or ws://127.0.0.1:port.".into())
                    }
                }
            })
            .build(&row_field_kit(k, p))
            .key("relay-address-input")
            .w(300.0),
        false,
        p,
    ));
    let operational = &s.relay_ui.operational;
    let contact = operational
        .last_contact
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%-d %b %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Not yet contacted".into());
    out = out.child(setting_row(
        s,
        "relay-last-contact",
        "Last relay check",
        "Last time this PC checked that the relay answered. It says nothing about other devices.",
        roles::SMALL.wrap(contact, p.soft),
        false,
        p,
    ));
    if !operational.message.is_empty() {
        out = out.child(
            roles::SMALL
                .wrap(
                    &operational.message,
                    if operational.read_only {
                        p.warn
                    } else {
                        p.soft
                    },
                )
                .padding(14.0)
                .key("relay-contact-status")
                .role("status"),
        );
    }
    if operational.read_only {
        out = out.child(
            roles::SMALL
                .wrap(
                    "Relay changes are read-only. Folder sync keeps working.",
                    p.soft,
                )
                .padding(14.0)
                .key("relay-read-only-note"),
        );
    }
    if let Some(error) = &s.hex.shell.relay_error {
        out = out.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    out
}
fn vocabulary(s: &State, k: &Kit, p: Palette) -> El {
    let mut out = col()
        .key("hex-vocabulary")
        .w_full()
        .fill(highlight(s, "hex-vocabulary", p))
        .child(setting_row(
            s,
            "vocabulary-row",
            "Names and terms",
            "Hex and Sidenote spell these right in transcripts and summaries.",
            crate::ui::widgets::action("Edit terms", p)
                .px(10.0)
                .radius(7.0)
                .key("vocabulary-edit")
                .on_press(|s: &mut State| {
                    if !s.hex.open_groups.remove("vocabulary-edit") {
                        s.hex.open_groups.insert("vocabulary-edit");
                    }
                }),
            true,
            p,
        ));
    if s.hex.open_groups.contains("vocabulary-edit") {
        out = out.child(vocabulary_section(s, k, p).padding(14.0));
    }
    out
}
fn sync_page(s: &State, k: &Kit, p: Palette) -> El {
    let devices = col().w_full().child(settings_card(
        "Devices",
        devices(s, k, p).child(feature(
            s,
            k,
            AppFeature::OtherDevices,
            "Other devices",
            &match s.nav.peers {
                0 => "Show meetings recorded on your other devices.".into(),
                1 => "1 meeting from another device.".into(),
                n => format!("{n} meetings from other devices."),
            },
            p,
        )),
        p,
    ));
    if s.hex.shell.relay.enabled {
        col()
            .w_full()
            .child(settings_card("Labs", relay_labs(s, k, p), p).key("relay-labs-card"))
            .child(super::super::relay::section(s, k, p))
            .child(devices)
    } else {
        col()
            .w_full()
            .child(devices)
            .child(super::super::relay::peers_card(s, p))
            .child(settings_card("Labs", relay_labs(s, k, p), p).key("relay-labs-card"))
    }
}
fn jump_groups(page: Page, _tab: usize) -> &'static [&'static str] {
    if page == Page::Hex { &HEX_GROUPS } else { &[] }
}
const HEX_GROUPS: [&str; 7] = [
    "Engine",
    "Shortcuts",
    "Dictation",
    "Speech model",
    "Compatibility",
    "Connection",
    "Hex Labs",
];
fn hex_group(index: usize, body: El) -> El {
    body.key(format!("settings-group-{index}"))
        .on_layout(move |s: &mut State, rect| {
            s.hex.shell.jump_positions.insert(index, rect[1]);
        })
}
fn hex_group_rows(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, group: usize) -> El {
    let mut body = col().w_full();
    for st in &s.hex.settings {
        if readable::windows_only_hidden(st) || location(st).0 != Page::Hex {
            continue;
        }
        let index = if matches!(st.key.as_str(), "network.proxy" | "cloud.api_key") {
            5
        } else if location(st).2 {
            4
        } else if matches!(place(st), Place::In(Section::Shortcuts)) {
            1
        } else {
            2
        };
        if index == group {
            body = body.child(super::setting_row(s, k, st, place(st), p, layer));
        }
    }
    body
}
fn refresh_labs(s: &mut State, enable: Option<String>) {
    if s.hex.shell.labs_loading || rune::tooling::headless_run().is_some() {
        return;
    }
    s.hex.shell.labs_loading = true;
    s.hex.shell.lab_error = None;
    s.hex.shell.labs = Default::default();
    s.hex_spawn(move |engine| {
        let result = enable.as_deref().map_or(Ok(()), |id| engine.enable_lab(id));
        let snapshot = engine.labs();
        Msg::Labs(snapshot, result)
    });
}
fn labs_group(s: &State, p: Palette) -> El {
    use sidenote::hex_labs::{Group, Snapshot};
    let unknown = Snapshot::default();
    let snapshot = if s.hex.running() {
        &s.hex.shell.labs
    } else {
        &unknown
    };
    let mut out = col().key("hex-labs").w_full().mt(18.0).child(
        panel(p).child(setting_row(
            s,
            "hex-labs-refresh",
            "Hex Labs",
            "Hex Labs that change what Sidenote sees.",
            crate::ui::widgets::action(
                if s.hex.shell.labs_loading {
                    "Reading..."
                } else {
                    "Refresh"
                },
                p,
            )
            .key("hex-labs-refresh-button")
            .button_disabled(s.hex.shell.labs_loading || !s.hex.running())
            .on_press(|s: &mut State| refresh_labs(s, None)),
            false,
            p,
        )),
    );
    if let Some(error) = &s.hex.shell.lab_error {
        out = out.child(roles::SMALL.wrap(error, p.warn).role("alert").padding(14.0));
    }
    let relevant = snapshot.relevant();
    for group in Group::ALL {
        let members: Vec<_> = relevant
            .iter()
            .filter(|(id, _, _)| snapshot.group(id, s.hex.shell.labs_show_all) == Some(group))
            .collect();
        if members.is_empty() {
            continue;
        }
        let mut rows = col().w_full();
        for (id, _, _) in members.iter() {
            rows = rows.child(lab_row(s, snapshot, id, p));
        }
        out = out.child(
            settings_card(group.name(), rows, p).key(format!("hex-labs-group-{}", group.key())),
        );
    }
    if relevant
        .iter()
        .any(|(id, _, _)| snapshot.group(id, false).is_none() && snapshot.group(id, true).is_some())
    {
        out = out.child(
            crate::ui::cards::fold_control(
                "Show all",
                "hex-labs-show-all",
                s.hex.shell.labs_show_all,
                p,
            )
            .mt(12.0)
            .on_press(|s: &mut State| s.hex.shell.labs_show_all = !s.hex.shell.labs_show_all),
        );
    }
    out
}
fn lab_row(s: &State, snapshot: &sidenote::hex_labs::Snapshot, id: &str, p: Palette) -> El {
    use sidenote::hex_labs::{UNKNOWN_EFFECT, find};
    let lab = find(id);
    let name = snapshot.name(id);
    let effect = lab.map_or(UNKNOWN_EFFECT, |lab| lab.effect);
    let state = snapshot.state(id);
    let mut controls = row().items_center().gap(8.0).child(
        roles::META
            .wrap(
                match state {
                    Some(true) => "On",
                    Some(false) => "Off",
                    None => "Unknown",
                },
                p.soft,
            )
            .key(format!("hex-lab-state-{id}")),
    );
    if snapshot.can_enable(id) {
        let id = id.to_owned();
        controls = controls.child(
            crate::ui::widgets::row_button(
                "Turn on",
                &format!("hex-lab-enable-{id}"),
                crate::ui::widgets::ButtonRole::Primary,
                p,
            )
            .label(format!("Turn on {name}"))
            .button_disabled(s.hex.shell.labs_loading)
            .on_press(move |s: &mut State| s.hex.shell.lab_confirm = Some(id.clone())),
        );
    }
    let mut out = col().w_full().child(setting_row(
        s,
        &format!("hex-lab-{id}"),
        &name,
        effect,
        controls,
        false,
        p,
    ));
    if s.hex.shell.lab_confirm.as_deref() == Some(id) && lab.is_some() {
        let id = id.to_owned();
        out = out.child(
            col()
                .key("hex-lab-confirm-dialog")
                .role("dialog")
                .label(format!("Turn on {name} in Hex?"))
                .w_full()
                .padding(14.0)
                .gap(8.0)
                .child(roles::NAME.wrap(format!("Turn on {name} in Hex?"), p.ink))
                .child(roles::SMALL.wrap(effect, p.soft))
                .child(roles::SMALL.wrap("You can turn it off in Hex \u{203a} Labs.", p.soft))
                .child(
                    row()
                        .flex_wrap()
                        .gap(8.0)
                        .child(
                            crate::ui::widgets::action("Cancel", p)
                                .key("hex-lab-cancel")
                                .on_press(|s: &mut State| s.hex.shell.lab_confirm = None),
                        )
                        .child(
                            crate::ui::widgets::button(
                                "Turn on",
                                "",
                                crate::ui::widgets::ButtonRole::Primary,
                                p,
                            )
                            .h(28.0)
                            .key("hex-lab-confirm")
                            .on_press(move |s: &mut State| {
                                if s.hex.shell.lab_confirm.take().as_ref() == Some(&id) {
                                    refresh_labs(s, Some(id.clone()));
                                }
                            }),
                        ),
                ),
        );
    }
    out
}
fn hex_fold(title: &str, key: &str, open: bool, p: Palette) -> El {
    crate::ui::cards::fold_control(title, key, open, p).child(icons::draw(
        &icons::HEX_MARK,
        14.0,
        p.soft,
    ))
}
fn hex_summaries(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let settings: Vec<_> = s
        .hex
        .settings
        .iter()
        .filter(|st| {
            matches!(
                st.key.as_str(),
                "meetings.summaries" | "meetings.opencode_model"
            )
        })
        .collect();
    if settings.is_empty() {
        return rune::el();
    }
    let open = s.hex.open_groups.contains("hex-summaries");
    let mut out = col().w_full().mt(18.0).child(
        hex_fold("Hex · Summaries", "hex-summaries", open, p).on_press(|s: &mut State| {
            if !s.hex.open_groups.remove("hex-summaries") {
                s.hex.open_groups.insert("hex-summaries");
            }
        }),
    );
    if open {
        let mut rows = col().w_full();
        for st in settings.into_iter() {
            rows = rows.child(super::setting_row(s, k, st, place(st), p, layer));
        }
        out = out.child(settings_card("Hex · Separate summary engine", rows, p));
    }
    out
}
fn load_kb(s: &mut State) {
    let config = s.config.clone();
    let kb = &config.knowledge_base;
    s.hex.shell.kb_fields = [
        TextEdit::new(&kb.repo_path),
        TextEdit::new(&kb.revision),
        TextEdit::new(kb.source_roots.join("\n")),
        TextEdit::new(&kb.company_name),
        TextEdit::new(&kb.people_roster_source),
    ];
    s.hex.shell.kb_base = Some(config);
}
fn edited_kb(s: &State) -> sidenote::core::config::KnowledgeBase {
    kb_edits(
        &s.hex
            .shell
            .kb_base
            .as_ref()
            .unwrap_or(&s.config)
            .knowledge_base,
        &s.hex.shell.kb_fields,
    )
}
fn kb_edits(
    base: &sidenote::core::config::KnowledgeBase,
    fields: &[TextEdit; 5],
) -> sidenote::core::config::KnowledgeBase {
    let mut kb = base.clone();
    if fields[0].text != base.repo_path {
        kb.repo_path = fields[0].text.clone();
    }
    if fields[1].text != base.revision {
        kb.revision = fields[1].text.clone();
    }
    if fields[2].text != base.source_roots.join("\n") {
        kb.source_roots = fields[2]
            .text
            .split('\n')
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
    }
    if fields[3].text != base.company_name {
        kb.company_name = fields[3].text.clone();
    }
    if fields[4].text != base.people_roster_source {
        kb.people_roster_source = fields[4].text.clone();
    }
    kb
}
fn save_kb(s: &mut State) {
    let kb = edited_kb(s);
    if !kb.is_empty()
        && let Err(reason) = kb.validate()
    {
        s.hex.shell.kb_error = Some(reason.into());
        return;
    }
    let mut config = s.hex.shell.kb_base.as_ref().unwrap_or(&s.config).clone();
    config.knowledge_base = kb;
    match config.save(&s.store.root) {
        Ok(()) => {
            s.config = config;
            load_kb(s);
            s.hex.shell.kb_error = None;
            s.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
        }
        Err(error) => {
            s.hex.shell.kb_error = Some(kb_save_error(&error));
            if let Ok(fresh) = sidenote::core::Config::load_in(&s.store.root) {
                rebase_kb(s, fresh);
            }
        }
    }
}
/// After a failed save, refresh the card from disk: fields the user changed keep
/// their draft text, untouched fields take the current value on disk.
fn rebase_kb(s: &mut State, fresh: sidenote::core::Config) {
    let was = &s
        .hex
        .shell
        .kb_base
        .as_ref()
        .unwrap_or(&s.config)
        .knowledge_base;
    let now = &fresh.knowledge_base;
    let pairs = [
        (was.repo_path.clone(), now.repo_path.clone()),
        (was.revision.clone(), now.revision.clone()),
        (was.source_roots.join("\n"), now.source_roots.join("\n")),
        (was.company_name.clone(), now.company_name.clone()),
        (
            was.people_roster_source.clone(),
            now.people_roster_source.clone(),
        ),
    ];
    for (field, (old, new)) in s.hex.shell.kb_fields.iter_mut().zip(pairs) {
        if field.text == old {
            *field = TextEdit::new(new);
        }
    }
    s.hex.shell.kb_base = Some(fresh);
}
fn kb_save_error(error: &std::io::Error) -> String {
    error
        .get_ref()
        .and_then(|e| e.downcast_ref::<sidenote::core::config::KnowledgeBaseSaveError>())
        .map(ToString::to_string)
        .unwrap_or_else(|| "Could not save knowledge base. Reload Settings and try again.".into())
}
fn knowledge_base(s: &State, k: &Kit, p: Palette) -> El {
    let kb = edited_kb(s);
    let field_kit = row_field_kit(k, p);
    let mut body = col()
        .w_full()
        .padding(16.0)
        .gap(12.0)
        .child(roles::SMALL.wrap(
            "Daily check reads these sources from your repository. Use a Markdown file for the people roster. These settings stay on this device.",
            p.soft,
        ));
    for (index, label) in [
        "Repository folder",
        "Revision",
        "Source folders (one per line)",
        "Company name",
        "People roster file",
    ]
    .into_iter()
    .enumerate()
    {
        let control = if index == 2 {
            rune_kit::TextArea::new(&s.hex.shell.kb_fields[index])
                .label(label)
                .auto_grow(2, 5)
                .on_edit(move |s: &mut State| &mut s.hex.shell.kb_fields[index])
                .build(&field_kit)
        } else {
            TextInput::new(&s.hex.shell.kb_fields[index])
                .height(30.0)
                .label(label)
                .placeholder(if index == 1 { "origin/main" } else { "" })
                .on_edit(move |s: &mut State| &mut s.hex.shell.kb_fields[index])
                .build(&field_kit)
        };
        body = body.child(
            col()
                .w_full()
                .gap(5.0)
                .child(roles::CONTROL.wrap(label, p.ink))
                .child(
                    control
                        .key(format!("kb-field-{index}"))
                        .w_full()
                        .min_w(0.0)
                        .border(1.0, field_kit.c.border_strong),
                ),
        );
    }
    let reason = if kb.is_empty() {
        "Not configured. Add a repository, revision and source folders to use daily check."
    } else {
        kb.validate()
            .err()
            .unwrap_or("Ready to save. Source paths are relative to the repository.")
    };
    body = body.child(roles::SMALL.wrap(reason, p.soft).key("kb-validation"));
    if let Some(error) = &s.hex.shell.kb_error {
        body = body.child(roles::SMALL.wrap(error, p.soft));
    }
    body = body.child(
        crate::ui::widgets::button(
            "Save knowledge base",
            "kb-save",
            crate::ui::widgets::ButtonRole::Primary,
            p,
        )
        .self_start()
        .button_disabled(!kb.is_empty() && kb.validate().is_err())
        .on_press(save_kb),
    );
    settings_card("Knowledge base", body, p).key("knowledge-base")
}

const MEETING_ICON_VISIBILITY_ERROR: &str = "Could not save whether to show the meeting icon.";

fn meeting_icon_visibility(s: &State, k: &Kit, p: Palette) -> El {
    let label = "Show the meeting icon";
    let help = match s.feature_error.as_deref() {
        Some(MEETING_ICON_VISIBILITY_ERROR) => MEETING_ICON_VISIBILITY_ERROR,
        _ => "Show a small icon at the screen edge while a meeting records.",
    };
    setting_row(
        s,
        "meeting-icon-visibility",
        label,
        help,
        Switch::new(sidenote::core::Config::meeting_icon_settings(&s.store.root).0)
            .label(label)
            .hide_label()
            .on_toggle(|s: &mut State| {
                match sidenote::core::Config::toggle_meeting_icon(&s.store.root) {
                    Ok(config) => {
                        s.config = config;
                        s.feature_error = None;
                    }
                    Err(_) => s.feature_error = Some(MEETING_ICON_VISIBILITY_ERROR.into()),
                }
            })
            .build(k)
            .key("meeting-icon-visibility-toggle"),
        true,
        p,
    )
}

fn meeting_icon_capture(s: &State, k: &Kit, p: Palette) -> El {
    setting_row(
        s,
        MEETING_ICON_CAPTURE,
        MEETING_ICON_CAPTURE_LABEL,
        "The meeting icon and its card are hidden from screenshots by default to keep meetings private.",
        Switch::new(sidenote::core::Config::meeting_icon_capture_allowed(
            &s.store.root,
        ))
        .label(MEETING_ICON_CAPTURE_LABEL)
        .hide_label()
        .on_toggle(|s: &mut State| {
            match sidenote::core::Config::toggle_meeting_icon_capture(&s.store.root) {
                Ok(config) => {
                    s.config = config;
                    s.feature_error = None;
                    s.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
                }
                Err(_) => {
                    s.feature_error =
                        Some("Could not save the meeting icon setting. Try again.".into())
                }
            }
        })
        .build(k)
        .key("meeting-icon-capture-toggle"),
        true,
        p,
    )
}

const TASK_GROUPINGS: [TaskGrouping; 6] = [
    TaskGrouping::Due,
    TaskGrouping::Project,
    TaskGrouping::Meeting,
    TaskGrouping::Team,
    TaskGrouping::Status,
    TaskGrouping::Owner,
];

fn work_select(
    s: &State,
    key: &'static str,
    label: &'static str,
    options: &[&str],
    current: Option<&str>,
) -> Select {
    Select::new(key, options)
        .label(label)
        .current(current)
        .appearance(SelectAppearance::Field)
        .width(short_select_width(s, 180.0))
        .open(s.hex.select_open.as_deref() == Some(key))
        .on_toggle(move |s: &mut State| {
            s.hex.select_open = if s.hex.select_open.is_some() {
                None
            } else {
                Some(key.into())
            };
        })
        .on_close(|s: &mut State| s.hex.select_open = None)
}

fn work_tasks(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let options = ["List", "Board"];
    let current = match s.tasks.layout {
        TasksLayout::List => "List",
        TasksLayout::Board => "Board",
    };
    let select = Select::new("work-tasks-layout-select", &options)
        .label("Tasks open as")
        .current(Some(current))
        .appearance(SelectAppearance::Field)
        .width(short_select_width(s, 180.0))
        .open(s.hex.select_open.as_deref() == Some("work-tasks-layout-select"))
        .on_toggle(|s: &mut State| {
            s.hex.select_open = if s.hex.select_open.is_some() {
                None
            } else {
                Some("work-tasks-layout-select".into())
            };
        })
        .on_close(|s: &mut State| s.hex.select_open = None)
        .on_choose(|s: &mut State, value: &str, _| {
            let layout = if value == "Board" {
                TasksLayout::Board
            } else {
                TasksLayout::List
            };
            if s.save_layout(layout).is_ok() {
                s.feature_error = None;
                s.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
            } else {
                s.hex.shell.saved = None;
                s.feature_error = Some("Could not save Tasks settings. Try again.".into());
            }
            s.hex.select_open = None;
        })
        .build(&row_field_kit(k, p), layer);

    let grouping_options: Vec<_> = TASK_GROUPINGS
        .into_iter()
        .filter(|by| *by != TaskGrouping::Project || s.config.features.projects)
        .map(tasks::group_label)
        .collect();
    let grouping = if s.tasks.grouping_loaded {
        Ok(s.tasks.grouping)
    } else {
        s.tasks.store.load_grouping()
    };
    let grouping_failed = grouping.is_err();
    // Tasks groups by due date while Projects is off, whatever was saved.
    let grouping_current = grouping
        .ok()
        .map(|by| match by {
            TaskGrouping::Project if !s.config.features.projects => TaskGrouping::Due,
            by => by,
        })
        .map(tasks::group_label);
    let grouping_select = work_select(
        s,
        "work-tasks-grouping-select",
        "Group tasks by",
        &grouping_options,
        grouping_current,
    )
    .placeholder("Unavailable")
    // A queued Group menu save would land after this direct write and undo it.
    .disabled(grouping_failed || s.edit_busy)
    .on_choose(|s: &mut State, value: &str, _| {
        s.hex.select_open = None;
        if s.edit_busy {
            return;
        }
        let Some(by) = TASK_GROUPINGS
            .into_iter()
            .find(|by| tasks::group_label(*by) == value)
        else {
            return;
        };
        if s.save_grouping(by).is_ok() {
            s.feature_error = None;
            s.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
        } else {
            s.hex.shell.saved = None;
            s.feature_error = Some("Could not save Tasks settings. Try again.".into());
        }
        s.hex.select_open = None;
    })
    .build(&row_field_kit(k, p), layer);

    let who_current = if s.tasks.who == Who::Mine && s.tasks.include_unassigned {
        "Mine + unassigned"
    } else if s.tasks.who == Who::Mine {
        "Mine"
    } else {
        "Everyone"
    };
    let who_select = work_select(
        s,
        "work-tasks-who-select",
        "Show",
        &["Everyone", "Mine", "Mine + unassigned"],
        Some(who_current),
    )
    .on_choose(|s: &mut State, value: &str, _| {
        let (who, include_unassigned) = match value {
            "Mine" => (Who::Mine, false),
            "Mine + unassigned" => (Who::Mine, true),
            _ => (Who::Everyone, false),
        };
        s.tasks.who = who;
        s.tasks.include_unassigned = include_unassigned;
        if s.save_task_audience().is_ok() {
            s.feature_error = None;
            s.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
        } else {
            s.hex.shell.saved = None;
            s.feature_error = Some("Could not save Tasks settings. Try again.".into());
        }
        s.hex.select_open = None;
    })
    .build(&row_field_kit(k, p), layer);

    let grouping_help = if grouping_failed {
        "Could not read this setting."
    } else {
        "Choose how tasks are grouped."
    };
    let rows = [
        (
            "work-tasks-layout",
            "Tasks open as",
            "Choose whether Tasks opens as a list or a board.",
            select,
        ),
        (
            "work-tasks-grouping",
            "Group tasks by",
            grouping_help,
            grouping_select,
        ),
        (
            "work-tasks-who",
            "Show",
            "Choose whose tasks are shown.",
            who_select,
        ),
    ];
    settings_card(
        "Tasks",
        col().w_full().children(
            rows.into_iter().map(|(key, name, help, control)| {
                setting_row(s, key, name, help, control, false, p)
            }),
        ),
        p,
    )
}

fn privacy(s: &State, k: &Kit, p: Palette) -> El {
    ai_notes::section(s, k, p, ai_notes::Part::Privacy)
}
fn body(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, page: Page, tab: usize) -> El {
    match (page, tab) {
        (Page::Experimental, _) => col().w_full().mt(18.0).child(
            panel(p).children(AppFeature::EXPERIMENTAL.into_iter().map(|test| {
                feature(s, k, test, test.name(), test.description(), p)
            })),
        ),
        (Page::Tasks, _) => work_tasks(s, k, p, layer),
        (Page::General, _) => col()
            .w_full()
            .child(ai_notes::section(s, k, p, ai_notes::Part::Identity))
            .child(card(
                "Meeting icon",
                col()
                    .child(meeting_icon_visibility(s, k, p))
                    .children(cfg!(windows).then(|| meeting_icon_capture(s, k, p))),
                p,
            ))
            .child(card(
                "Sidebar features",
                feature(s, k, AppFeature::Tasks, "Tasks", "Show Tasks in the sidebar.", p),
                p,
            )),
        (Page::Meetings, 0) => col()
            .w_full()
            .child(hex_group(
                0,
                settings_card(
                    "Hex · Meeting recording",
                    hex_rows(s, k, p, layer, page, tab, false),
                    p,
                ),
            ))
            .children(s.hex.setting("meetings.providers").map(|st| {
                hex_group(
                    1,
                    settings_card(
                        "Hex · Call apps",
                        readable::providers(s, k, st, p, layer),
                        p,
                    ),
                )
            }))
            .children(s.config.features.indic_engine.then(|| hex_group(
                2,
                panel(p)
                    .mt(roles::SECTION_TOP)
                    .child(super::super::models::language_picker(s, k, p)),
            )))
            .child(hex_group(
                3,
                settings_card("Hex · Names and terms", vocabulary(s, k, p), p),
            ))
            .child(hex_group(
                4,
                advanced(
                    s,
                    settings_card(
                        "Hex · Recording compatibility",
                        hex_rows(s, k, p, layer, page, tab, true),
                        p,
                    ),
                    p,
                ),
            )),
        (Page::Meetings, 1) => col()
            .w_full()
            .child(ai_notes::section(s, k, p, ai_notes::Part::Summaries))
            .child(advanced(
                s,
                ai_notes::section(s, k, p, ai_notes::Part::Scripts),
                p,
            ))
            .child(hex_summaries(s, k, p, layer)),
        (Page::Meetings, 3) => {
            let body = engine::body(s, k, p, layer);
            #[cfg(windows)]
            let body = body.child(host::card(s, k, p));
            body
        }
        (Page::Meetings, _) => {
            let enabled = s.config.features.enabled(AppFeature::DailyCheck);
            let open = s.hex.shell.kb_open;
            col()
                .w_full()
                .mt(roles::SECTION_TOP)
                .child(panel(p).child(feature(
                    s,
                    k,
                    AppFeature::DailyCheck,
                    "Daily check",
                    "Sidenote checks your meeting knowledge for gaps and conflicts each day.",
                    p,
                )))
                .children((!enabled).then(|| {
                    cards::fold_control("Configure knowledge base", "knowledge-base-fold", open, p)
                        .mt(12.0)
                        .on_press(|s: &mut State| s.hex.shell.kb_open = !s.hex.shell.kb_open)
                }))
                .children((enabled || open).then(|| knowledge_base(s, k, p)))
        }
        (Page::Prompts, _) => super::super::prompt_library::view(s, p),
        (Page::People, _) => super::super::people::content(s, k, p, layer),
        (Page::Helpers, 0) => col()
            .w_full()
            .children(ai_notes::setup_card(s, p))
            .child(super::super::ask_settings::section(s, k, p, layer))
            .child(super::super::cloud_ask::section(s, k, p, layer))
            .child(roles::SMALL.bold().wrap("Meeting summary agent", p.soft).mt(roles::SECTION_TOP))
            .child(ai_notes::section(s, k, p, ai_notes::Part::Helpers))
            .child(ai_notes::summary_model(s, k, p, layer, Vec::new()))
            .children(s.config.features.annotations.then(|| {
                super::super::annotations::agents_section(s, k, p, layer)
            })),
        (Page::Helpers, _) => col()
            .w_full()
            .child(
                super::super::models::view(s, p, rune::el())
                    .key("speech-models")
                    .fill(highlight(s, "speech-models", p)),
            )
            .children(s.config.features.indic_engine.then(|| {
                crate::ui::widgets::action("Choose meeting languages", p)
                    .key("speech-language-link")
                    .on_press(|s: &mut State| navigate(s, Page::Meetings, 0))
            }))
            .children(s.config.features.indic_engine.then(|| advanced(
                s,
                col()
                    .w_full()
                    .child(settings_card(
                        "Language handling",
                        meeting_languages::section(s, k, p, layer),
                        p,
                    ))
                    .child(super::super::models::advanced(s, p)),
                p,
            ))),
        (Page::Hex, _) => {
            let mut out = col()
                .w_full()
                .child(hex_group(
                    0,
                    settings_card("Recording engine", summary_line(s, p), p),
                ))
                .child(hex_group(
                    1,
                    settings_card("Shortcuts", hex_group_rows(s, k, p, layer, 1), p),
                ))
                .child(hex_group(
                    2,
                    settings_card("Dictation", hex_group_rows(s, k, p, layer, 2), p),
                ))
                .child(hex_group(
                    3,
                    settings_card(
                        "Speech model",
                        model_section(s, p).padding(14.0).key("legacy-hex-models"),
                        p,
                    ),
                ));
            for index in [4, 5] {
                let title = if index == 4 {
                    "Dictation compatibility"
                } else {
                    "Connection"
                };
                let open = s.hex.open_groups.contains(if index == 4 {
                    "hex-compatibility"
                } else {
                    "hex-connection"
                }) || s.hex.shell.advanced.contains(page.name());
                let key = if index == 4 {
                    "hex-compatibility"
                } else {
                    "hex-connection"
                };
                let mut group = col().w_full().mt(18.0).child(
                    crate::ui::cards::fold_control(title, key, open, p).on_press(
                        move |s: &mut State| {
                            if !s.hex.open_groups.remove(key) {
                                s.hex.open_groups.insert(key);
                            }
                        },
                    ),
                );
                if open {
                    group = group.child(panel(p).child(hex_group_rows(s, k, p, layer, index)));
                }
                out = out.child(hex_group(index, group));
            }
            out.child(hex_group(6, labs_group(s, p)))
        }
        (Page::Appearance, _) => card(
            "Hex · Preferences",
            hex_rows(s, k, p, layer, page, tab, false),
            p,
        ),
        (Page::Sync, _) => col()
            .w_full()
            .child(sync_page(s, k, p))
            .child(named_folder(s, k, p)),
        // The page title already says "Connected apps" and the page has only this section.
        (Page::ConnectedApps, _) => col().w_full().min_w(0.0).mt(18.0).child(panel(p).child(super::super::mcp_connect::section(s, p).key("mcp-connect").fill(highlight(s, "mcp-connect", p)).w_full())),
        (Page::Privacy, _) => col()
            .w_full()
            .child(privacy(s, k, p))
            .children(Page::ConnectedApps.visible(CHANNEL).then(|| {
                col()
                    .w_full()
                    .min_w(0.0)
                    .mt(roles::GROUP_TOP)
                    .gap(roles::SECTION_GAP)
                    .child(
                        roles::SMALL
                            .wrap("Connected apps can read meetings, typed notes, tasks and memory, and send them to their AI provider.", p.soft)
                            .w_full()
                            .min_w(0.0),
                    )
                    .child(
                        row().w_full().min_w(0.0).child(
                            crate::ui::widgets::link("Manage connected apps", p)
                                .key("privacy-connected-apps")
                                .label("Manage connected apps")
                                .shrink()
                                .min_h(32.0)
                                .px(0.0)
                                .py(7.0)
                                .focus_ring(2.0, 2.0, p.accent)
                                .on_press(|s: &mut State| navigate(s, Page::ConnectedApps, 0)),
                        ),
                    )
            }))
            .child(settings_card(
                "Hex · Retention and files",
                hex_rows(s, k, p, layer, page, 0, false),
                p,
            )),
        (Page::Updates, _) => super::super::updates::section(s, p),
        (Page::Diagnostics, _) => col()
            .w_full()
            .key("helper-advanced")
            .child(crate::ui::response_times::section(p))
            .child(labs(s, k, p, layer))
            .child(ai_notes::section(s, k, p, ai_notes::Part::Advanced))
            .child(settings_card(
                "Update source",
                super::super::updates::source(s, p)
                    .key("update-source")
                    .padding(14.0),
                p,
            ))
            .child(settings_card(
                "Hex · Diagnostics",
                hex_rows(s, k, p, layer, page, 0, false),
                p,
            )),
    }
}
fn named_folder(s: &State, k: &Kit, p: Palette) -> El {
    let open = s.hex.shell.advanced.contains(Page::Sync.name());
    let mut out = col().w_full().mt(18.0).child(
        crate::ui::cards::fold_control(
            if s.hex.shell.relay.enabled {
                "Advanced"
            } else {
                "Folder location"
            },
            "settings-advanced",
            open,
            p,
        )
        .on_press(|s: &mut State| {
            if !s.hex.shell.advanced.remove(Page::Sync.name()) {
                s.hex.shell.advanced.insert(Page::Sync.name());
            }
        }),
    );
    if open {
        out = out.child(settings_card(
            "Sync folder",
            roles::SMALL
                .wrap(s.store.root.display().to_string(), p.ink)
                .key("sync-folder")
                .padding(14.0),
            p,
        ));
        if s.hex.shell.relay.enabled {
            out = out.child(settings_card("Relay address", relay_address(s, k, p), p));
        }
    }
    out
}
pub fn pane(s: &State, k: &Kit, p: Palette, _narrow: bool, layer: &mut Vec<El>) -> El {
    let (page, tab) = active(s);
    let narrow = s.app_w.get() < DRAWER_AT;
    let gutter = if narrow { 16.0 } else { 40.0 };
    let pane_width = PAGE_COLUMN + 2.0 * gutter;
    let drawer = drawer_open(s);
    let hex_unavailable = page == Page::Hex && !s.hex.running();
    let page_note = s.hex.page_note().map(|reason| {
        if reason == super::connection::UNKNOWN {
            "Could not confirm Hex recording status. Checking again automatically."
        } else {
            reason
        }
    });
    let mut doc = col()
        .key("hex-top")
        .on_layout(|s: &mut State, rect| s.hex.shell.jump_origin = rect[1])
        .w_full()
        .max_w(pane_width)
        .px(gutter)
        .pb(40.0);
    let setup = (page == Page::Hex)
        .then(|| super::setup::card(s, p))
        .flatten();
    let has_setup = setup.is_some();
    if let Some(setup) = setup {
        doc = doc.child(setup);
    }
    if page == Page::Hex
        && let Some(status) = s.hex.state()
        && let sidenote::hex_install::Installation::Installed { version, .. } = &s.hex.installation
        && version != &status.version
    {
        doc = doc.child(
            roles::SMALL
                .wrap(
                    format!("Hex will update to {version} when it restarts."),
                    p.soft,
                )
                .key("hex-pending-update"),
        );
    }
    if s.hex.settings_status_pending && !hex_unavailable {
        doc = doc.child(
            roles::SMALL
                .wrap("Checking Hex...", p.soft)
                .role("status")
                .key("settings-hex-check"),
        );
    } else if let Some(note_) = page_note
        .filter(|_| {
            !hex_unavailable && matches!(page, Page::Hex | Page::Appearance | Page::Meetings)
        })
        .filter(|_| {
            !has_setup || !matches!(s.hex.status, None | Some(Err(HexError::HexNotRunning)))
        })
    {
        doc = doc.child(note(note_, p));
    }
    if let Some(problem) = s.hex.settings_note {
        doc = doc.child(note(problem, p));
    }
    if let Some(problem) = &s.feature_error
        && (page != Page::General || problem != MEETING_ICON_VISIBILITY_ERROR)
    {
        doc = doc.child(roles::SMALL.wrap(problem, p.warn).role("alert"));
    }
    // The section controls above always need their sections, so the body shows in every state.
    // Its controls stay disabled and the header says why while Hex is unconfirmed.
    doc = doc.child(body(s, k, p, layer, page, tab));
    if drawer {
        layer.push(rune_kit::scrim("settings-scrim", close_drawer).fill(p.scrim));
        layer.push(
            settings_list(s, k, p, LIST_WIDE, false)
                .key("settings-drawer")
                .role("dialog")
                .label("Settings pages")
                .pin(Some(0.0), Some(0.0), None, Some(0.0))
                .focus_return("settings-navigation")
                .modal(true, close_drawer),
        );
    }
    col()
        .key("hex-pane")
        .w_full()
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .h_full()
        .children(
            (page == Page::Sync)
                .then(|| super::super::relay::status_banner(s, p))
                .flatten(),
        )
        .child(
            col().w_full().items_center().child(
                col()
                    .w_full()
                    .max_w(pane_width)
                    .px(gutter)
                    .children(narrow.then(|| {
                        row()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .mt(12.0)
                            .child(
                                settings_back(p)
                                    .key("settings-header-back")
                                    .w(76.0)
                                    .fill(Color::TRANSPARENT),
                            )
                            .child(
                                pressable(row().size(TAB_H, TAB_H).center().child(icons::draw(
                                    &icons::PANEL_OPEN,
                                    icons::ROW,
                                    p.soft,
                                )))
                                .key("settings-navigation")
                                .label("Settings pages")
                                .radius(7.0)
                                .hover(p.chip)
                                .focus_ring(1.0, 0.0, p.line)
                                .expanded(drawer)
                                .on_press(|s: &mut State| s.hex.shell.navigation_open = true),
                            )
                    }))
                    .children(
                        s.hex
                            .shell
                            .saved
                            .map(|_| roles::SMALL.line("Saved", p.ok).role("status")),
                    )
                    .child(
                        rune::text(page.name())
                            .font(if narrow {
                                crate::ui::theme::display(24.0)
                            } else {
                                crate::ui::theme::display(crate::ui::theme::PAGE_TITLE).lh(32.0)
                            })
                            .color(p.ink)
                            .wrap()
                            .key("settings-title")
                            .mt(roles::SECTION_TOP),
                    )
                    .children(
                        (page != Page::Prompts)
                            .then(|| roles::META.wrap("Applies to this PC", p.faint).mt(4.0)),
                    )
                    .children(
                        matches!(
                            page,
                            Page::Hex | Page::Appearance | Page::Privacy | Page::Diagnostics
                        )
                        .then(|| ownership(page, p)),
                    )
                    .children((page == Page::Meetings && tab <= 1).then(|| ownership(page, p)))
                    .children((!page.tabs().is_empty()).then(|| tab_bar(s, p, false, page, tab)))
                    .children(
                        (!jump_groups(page, tab).is_empty())
                            .then(|| tab_bar(s, p, true, page, tab)),
                    )
                    .children(
                        (hex_unavailable
                            && (!has_setup
                                || !matches!(
                                    s.hex.status,
                                    None | Some(Err(HexError::HexNotRunning))
                                )))
                        .then(|| {
                            let reason = if s.hex.settings_status_pending {
                                "Checking Hex."
                            } else {
                                page_note.unwrap_or("Checking Hex.")
                            };
                            roles::SMALL
                                .wrap(reason, p.soft)
                                .role("status")
                                .key("hex-unavailable-reason")
                                .mt(8.0)
                        }),
                    )
                    .pb(4.0),
            ),
        )
        .child(if page == Page::Prompts {
            // The prompt library is a list and an editor that fill the window. They scroll on
            // their own, so the page around them does not.
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .items_center()
                .child(doc.grow(1.0).min_h(0.0).pb(16.0))
        } else {
            scroll(col().w_full().items_center().child(doc), "hex-scroll").on_scroll_offset({
                let offset = s.hex.shell.jump_offset.clone();
                let max = s.hex.shell.jump_max.clone();
                move |pos| {
                    offset.set(pos.offset[1]);
                    max.set(pos.max[1]);
                }
            })
        })
}
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape && s.hex.shell.lab_confirm.take().is_some() {
        tx.blur();
        return true;
    }
    if e.key == Key::Escape && drawer_open(s) {
        close_drawer(s);
        tx.blur();
        return true;
    }
    if active(s).0 == Page::Prompts && super::super::prompt_library::on_key(s, e, tx) {
        return true;
    }
    if e.key == Key::Escape {
        if s.hex.select_open.take().is_some() {
        } else if !s.hex.filter.text.is_empty() {
            s.hex.filter = TextEdit::default();
            tx.blur();
        } else {
            back(s);
            tx.blur();
        }
        return true;
    }
    if e.key == Key::Tab && e.mods.ctrl {
        let (page, tab) = active(s);
        if !page.tabs().is_empty() {
            let delta = if e.mods.shift { -1 } else { 1 };
            let step = |tab| (tab as isize + delta).rem_euclid(page.tabs().len() as isize) as usize;
            let mut next = step(tab);
            if !tab_visible(s, page, next) {
                next = step(next);
            }
            navigate(s, page, next);
            tx.blur();
            return true;
        }
    }
    false
}

pub fn scene_status(s: &mut State, pending: bool) {
    scene(s, "settings-engine");
    s.hex.shell.engine.unused_open = false;
    s.hex_status_at(
        Err(HexError::Timeout),
        Instant::now() - Duration::from_secs(20),
    );
    s.hex.settings_status_pending = pending;
    s.hex.settings_status_at = Some(Instant::now());
}

/// Synthetic scenes run only with the render harness's isolated state.
pub fn scene(s: &mut State, name: &str) {
    assert!(super::super::isolated());
    if name.starts_with("settings-kb-") {
        super::super::notes_scene::prepare(s);
        s.config.features.memory = true;
        s.hex_open = true;
        s.config.knowledge_base = sidenote::core::config::KnowledgeBase::default();
        if name != "settings-kb-empty" {
            s.config.knowledge_base.repo_path = "E:/example/kb".into();
            s.config.knowledge_base.revision = if name == "settings-kb-invalid" {
                "main"
            } else {
                "origin/main"
            }
            .into();
            s.config.knowledge_base.source_roots = vec!["docs".into(), "handbook".into()];
            s.config.knowledge_base.company_name = "Example company".into();
            s.config.knowledge_base.people_roster_source = "people/roster.md".into();
        }
        load_kb(s);
        navigate(s, Page::Meetings, 2);
        s.hex.shell.kb_open = true;
        if name == "settings-kb-merge-error" {
            s.hex.shell.kb_error = Some(
                "Could not save knowledge base after merging other saved changes. Enter the repository folder. Reload Settings and try again."
                    .into(),
            );
        }
        return;
    }
    super::super::notes_scene::settings(s, "scripts");
    if name != "settings-experimental" {
        s.config.features.memory = true;
    }
    super::super::models::scene(s, "empty");
    super::super::scene_agents(s, super::super::ALL_AGENTS);
    s.refresh_ask_model_input();
    s.ai_notes.name = TextEdit::new("Ravi Menon");
    if let Some(settings) = &mut s.ai_notes.settings {
        settings.your_name = "Ravi Menon".into();
        settings.engine = "app".into();
    }
    let (page, tab) = match name.trim_end_matches("-expanded") {
        "settings-general" => (Page::General, 0),
        "settings-experimental" => (Page::Experimental, 0),
        "settings-work-tasks" => (Page::Tasks, 0),
        "settings-recording" => (Page::Meetings, 0),
        "settings-summaries" => (Page::Meetings, 1),
        "settings-tasks" => (Page::Meetings, 2),
        "settings-engine" | "settings-engine-invalid" | "settings-engine-notes-error" => {
            (Page::Meetings, 3)
        }
        "settings-prompts"
        | "settings-prompts-edited"
        | "settings-prompts-diff"
        | "settings-prompts-sort"
        | "settings-prompts-redo"
        | "settings-prompts-conflict"
        | "settings-prompts-readonly" => (Page::Prompts, 0),
        "settings-people" => (Page::People, 0),
        "people-our-team" | "people-team-editor" | "people-search" => (Page::People, 0),
        "people-external" => (Page::People, 1),
        "people-import" => (Page::People, 2),
        "settings-hex"
        | "settings-hex-setup"
        | "settings-hex-installing"
        | "settings-hex-install-failed"
        | "settings-hex-installed"
        | "settings-hex-stale"
        | "settings-hex-update"
        | "settings-hex-meetings-off"
        | "settings-hex-meetings-enable" => (Page::Hex, 0),
        "settings-helpers" | "settings-helpers-expanded" => (Page::Helpers, 0),
        "settings-mcp-added" | "settings-mcp-mixed" | "settings-mcp-none" => {
            (Page::ConnectedApps, 0)
        }
        "settings-speech" => (Page::Helpers, 1),
        "settings-helper-advanced" => (Page::Diagnostics, 0),
        "settings-privacy" => (Page::Privacy, 0),
        "settings-diagnostics" => (Page::Diagnostics, 0),
        "settings-appearance" => (Page::Appearance, 0),
        "settings-sync" => (Page::Sync, 0),
        "settings-updates" => (Page::Updates, 0),
        "settings-search" => (Page::Meetings, 0),
        _ => panic!("Unknown Settings scene"),
    };
    if page == Page::Updates {
        super::super::updates::settings_fixture(s);
    }
    if name.starts_with("settings-engine") {
        engine::scene_config(&mut s.config);
        s.config.notes = s.ai_notes.settings.clone();
    }
    navigate(s, page, tab);
    if name.starts_with("settings-mcp-") {
        super::super::mcp_connect::scene(s, name);
    }
    if name.starts_with("settings-engine") {
        s.hex.shell.engine.unused_open = true;
    }
    if name == "settings-engine-invalid" {
        engine::scene_invalid(s);
    }
    if name == "settings-engine-notes-error" {
        engine::scene_notes_error(s);
    }
    if page == Page::Prompts {
        super::super::prompt_library::scene(s, name);
    }
    match name {
        "settings-hex-meetings-off" | "settings-hex-meetings-enable" => {
            let supported = name == "settings-hex-meetings-enable";
            let state = s.hex.status.as_mut().unwrap().as_mut().unwrap();
            state.version = "3.4.0".into();
            *state.labs = sidenote::hex_labs::Snapshot::status(&serde_json::json!({
                "features": {"meetings": false}, "labs": []
            }));
            s.hex.installation = sidenote::hex_install::Installation::Installed {
                version: "3.4.0".into(),
                exe: "fixture/hex.exe".into(),
            };
            if supported {
                s.hex.install = sidenote::hex_install::State::Complete;
            }
            let support = if supported {
                Support::Supported
            } else {
                Support::Unsupported
            };
            for feature in [Feature::SettingsGet, Feature::SettingsSet] {
                s.hex.caps.as_mut().unwrap().insert(feature, support);
            }
            s.hex
                .settings
                .retain(|setting| setting.key != "meetings.enabled");
            s.hex.settings.push(serde_json::from_value(serde_json::json!({
                "key": "meetings.enabled", "value": false, "kind": "bool", "writable": true, "available": true
            })).unwrap());
        }
        // Hex answered once, then went quiet past the freshness limit: unknown, not stopped.
        "settings-hex-stale" | "settings-hex-update" => {
            let mut good = s.hex.status.as_ref().unwrap().as_ref().unwrap().clone();
            s.hex.installation = sidenote::hex_install::Installation::Installed {
                version: "0.0.0-dev-1085".into(),
                exe: "fixture/hex.exe".into(),
            };
            if name == "settings-hex-update" {
                good.version = "0.0.0-dev-1083".into();
                s.hex.caps_version = Some(good.version.clone());
                s.hex.status = Some(Ok(good));
            } else {
                s.hex_status_at(Ok(good), Instant::now() - Duration::from_secs(11));
            }
        }
        "settings-hex-setup"
        | "settings-hex-installing"
        | "settings-hex-install-failed"
        | "settings-hex-installed" => {
            use sidenote::hex_install::{Installation, State};
            s.hex.status = Some(Err(HexError::HexNotRunning));
            s.hex.installation = Installation::Missing;
            s.hex.install = match name {
                "settings-hex-installing" => State::Downloading {
                    bytes: 12_000_000,
                    total: 48_000_000,
                },
                "settings-hex-install-failed" => {
                    State::Failed("Could not download Hex. Check your connection and try again.")
                }
                "settings-hex-installed" => {
                    s.hex.installation = Installation::Installed {
                        version: "3.4.0".into(),
                        exe: "fixture/hex.exe".into(),
                    };
                    State::Complete
                }
                _ => State::Idle,
            };
        }
        _ => {}
    }
    if name == "people-team-editor"
        && let Some(team) = s.tasks.teams.teams.first().cloned()
    {
        super::super::teams::open(s, Some(team));
    }
    if name == "people-search" {
        s.people.search = TextEdit::new("Alex");
    }
    if name.ends_with("-expanded") {
        s.hex.shell.advanced.insert(page.name());
        s.ai_notes.model_open = true;
        s.connection.show_details = true;
        s.connection.show_all = true;
        s.hex.open_groups.insert("hex-summaries");
        if page == Page::Meetings && tab == 0 {
            s.hex.open_groups.insert("languages");
            s.hex.open_groups.insert("vocabulary-edit");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_check_keeps_configuration_reachable_while_off() {
        crate::test_paths::isolate();
        let mut state = State::new();
        state.config.features.memory = true;
        state.config.features.daily_check = false;
        state.hex_open = true;
        state.hex.settings_status_pending = true;
        navigate(&mut state, Page::Meetings, 2);
        let destination = index(&state)
            .into_iter()
            .find(|item| item.key == "knowledge-base")
            .unwrap();
        let mut rt = rune::Runtime::new(
            state,
            crate::ui::view,
            (1280.0, 900.0),
            rune::Env::default(),
        );
        rt.frame(0.0);
        assert!(rt.find("knowledge-base-fold").is_some());
        assert!(rt.find("kb-field-0").is_none());
        rt.update(|s, _| jump(s, destination));
        rt.frame(0.016);
        assert!(rt.find("kb-field-0").is_some());
    }

    #[cfg(windows)]
    #[test]
    fn missing_hex_shows_setup_without_asking_to_start_hex() {
        crate::test_paths::isolate();
        let mut state = State::new();
        state.hex_open = true;
        state.hex.status = Some(Err(HexError::HexNotRunning));
        state.hex.installation = sidenote::hex_install::Installation::Missing;
        navigate(&mut state, Page::Hex, 0);
        let mut rt = rune::Runtime::new(
            state,
            crate::ui::view,
            (1280.0, 900.0),
            rune::Env::default(),
        );
        rt.frame(0.0);
        assert!(rt.find("hex-install").is_some());
        assert!(rt.find("hex-unavailable-reason").is_none());
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("Not installed"))
        );
    }

    #[test]
    fn settings_list_rows_and_tabs_use_the_compact_heights() {
        crate::test_paths::isolate();
        let mut state = State::new();
        state.open_hex();
        navigate(&mut state, Page::Meetings, 0);
        let mut rt = rune::Runtime::with_text(
            state,
            crate::ui::view,
            (1280.0, 900.0),
            rune::Env::default(),
            rune::text::TextSystem::new(
                crate::ui::theme::fonts(),
                (crate::ui::theme::SANS, "Inter"),
            ),
        );
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        let rect = |key| {
            elements
                .iter()
                .find(|element| element.key.as_deref() == Some(key))
                .unwrap_or_else(|| panic!("missing {key}"))
                .rect
        };
        for (key, height) in [
            ("settings-back", 32.0),
            ("settings-page-Meetings", 32.0),
            ("settings-tab-0", 28.0),
        ] {
            assert_eq!(rect(key)[3], height, "{key}");
        }
        assert_eq!(rect("hex-filter")[2], rect("settings-sidebar")[2] - 16.0);
    }

    #[test]
    fn people_is_a_sidebar_page_and_search_and_back_return_to_it() {
        crate::test_paths::isolate();
        let mut state = State::new();
        state.go_meetings();
        state.config.features.people = true;
        state.sidebar_open.set(true);
        let mut rt = rune::Runtime::with_text(
            state,
            crate::ui::view,
            (1334.0, 900.0),
            rune::Env::default(),
            rune::text::TextSystem::new(
                crate::ui::theme::fonts(),
                (crate::ui::theme::SANS, "Inter"),
            ),
        );
        rt.settle_all();
        rt.frame(0.0);
        let click = |rt: &mut rune::Runtime<State>, key: &str| {
            let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
            rt.pointer_move(x, y);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.2);
        };

        assert!(
            rt.find("nav-meetings").is_some(),
            "home sidebar was not rendered"
        );
        click(&mut rt, "nav-people");
        assert!(rt.find("people-roster").is_some());
        let elements = rt.elements();
        for name in ["Our team", "External"] {
            assert!(
                elements
                    .iter()
                    .any(|e| e.role == "tab" && e.label.as_deref() == Some(name))
            );
        }

        rt.update(|s, _| s.open_people_tab(0));
        for (shift, expected) in [(false, 1), (false, 0), (true, 1), (true, 0)] {
            rt.update(|s, tx| {
                let event = KeyEvent {
                    key: Key::Tab,
                    mods: rune::Mods {
                        ctrl: true,
                        shift,
                        ..Default::default()
                    },
                };
                assert!(
                    crate::ui::on_key(s, &event, tx),
                    "People ignored Ctrl+Tab with shift={shift}"
                );
                assert_eq!(s.people.tab, expected, "Ctrl+Tab with shift={shift}");
            });
        }

        rt.update(|s, _| s.open_hex());
        rt.frame(0.2);
        assert!(rt.find("settings-page-People").is_none());
        click(&mut rt, "settings-back");
        assert!(rt.find("people-roster").is_some());

        rt.update(|s, _| {
            s.go_meetings();
            s.open_finder();
            s.finder_query = TextEdit::new("people");
            s.finder_changed();
        });
        rt.frame(0.2);
        click(&mut rt, "finder-row-0");
        assert!(rt.find("people-roster").is_some());
        assert!(!rt.state.on_hex());

        rt.update(|s, _| {
            s.open_hex();
            s.hex.filter = TextEdit::new("External");
        });
        rt.frame(0.2);
        click(&mut rt, "find-setting-people-roster");
        assert_eq!(rt.state.people.tab, 1);
        assert!(rt.find("people-companies").is_some());
        click(&mut rt, "people-companies");
        rt.update(|s, tx| {
            assert!(crate::ui::on_key(
                s,
                &KeyEvent {
                    key: Key::Escape,
                    mods: rune::Mods::default()
                },
                tx
            ));
        });
        rt.frame(0.2);
        assert!(rt.find("people-companies").is_some());
        assert!(!rt.state.on_hex());
    }

    fn stored_device_name(root: &std::path::Path) -> String {
        let file = std::fs::read(root.join("device.json")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&file).unwrap();
        value["name"].as_str().unwrap().to_owned()
    }
    #[test]
    fn a_folder_device_shows_the_recording_date_not_a_receipt_date() {
        use chrono::TimeZone;
        let day = chrono::Local
            .with_ymd_and_hms(2026, 11, 1, 9, 30, 0)
            .unwrap();
        assert_eq!(
            folder_device_line(3, Some(day)),
            "3 meetings received. Latest meeting recorded 1 Nov 2026."
        );
        assert_eq!(folder_device_line(1, None), "1 meetings received.");
    }
    const BAD_NAME: &str = "Use 1 to 80 characters, with no control characters.";

    #[test]
    fn saving_the_device_name_stores_it_and_rejects_bad_input() {
        let root = tempfile::tempdir().unwrap();
        let mut s = State::new();
        s.store.root = root.path().to_path_buf();
        s.hex.shell.device_name = TextEdit::new("  Studio PC ");
        save_device_name(&mut s);
        assert_eq!(s.hex.shell.device_name_error, None);
        assert_eq!(stored_device_name(root.path()), "Studio PC");
        assert_eq!(s.hex.shell.device_name.text, "Studio PC");
        assert!(
            s.toast
                .as_ref()
                .is_some_and(|(t, _)| t.starts_with("Name saved."))
        );
        for bad in ["   ".to_owned(), "x".repeat(81), "tab\there".to_owned()] {
            s.hex.shell.device_name = TextEdit::new(&bad);
            save_device_name(&mut s);
            assert_eq!(s.hex.shell.device_name_error.as_deref(), Some(BAD_NAME));
            assert_eq!(stored_device_name(root.path()), "Studio PC");
        }
        s.hex.shell.device_name = TextEdit::new("Travel laptop");
        save_device_name(&mut s);
        assert_eq!(
            s.hex.shell.device_name_error, None,
            "a good name clears the error"
        );
        assert_eq!(stored_device_name(root.path()), "Travel laptop");
    }

    #[cfg(windows)]
    #[test]
    #[cfg(feature = "dev")]
    fn saving_the_device_name_is_blocked_while_pairing_is_busy() {
        let root = tempfile::tempdir().unwrap();
        let mut s = State::new();
        s.store.root = root.path().to_path_buf();
        s.hex.shell.device_name = TextEdit::new("Studio PC");
        save_device_name(&mut s);
        assert_eq!(stored_device_name(root.path()), "Studio PC");
        sidenote::core::relay_preview::Settings {
            enabled: true,
            address: "ws://127.0.0.1:9".into(),
            ..Default::default()
        }
        .save(root.path())
        .unwrap();
        s.relay_pair(false);
        assert!(s.relay_ui.pairing_busy(), "a real pairing job is running");
        s.hex.shell.device_name = TextEdit::new("Travel laptop");
        save_device_name(&mut s);
        assert_eq!(
            s.hex.shell.device_name_error.as_deref(),
            Some("Finish or cancel pairing before renaming this device.")
        );
        assert_eq!(stored_device_name(root.path()), "Studio PC");
        s.relay_cancel();
        save_device_name(&mut s);
        assert_eq!(s.hex.shell.device_name_error, None);
        assert_eq!(stored_device_name(root.path()), "Travel laptop");
        sidenote::core::relay_preview::disconnect(root.path()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[cfg(feature = "dev")]
    fn linux_relay_settings_refuse_enable_and_allow_local_device_rename() {
        use sidenote::core::relay_preview::Settings;

        fn view(s: &State, _: &mut rune::Cx<State>) -> El {
            let p = crate::ui::theme::palette(false);
            relay_labs(s, &crate::ui::theme::kit(&p), p)
        }

        let root = tempfile::tempdir().unwrap();
        Settings {
            address: "ws://127.0.0.1:9".into(),
            ..Default::default()
        }
        .save(root.path())
        .unwrap();
        let settings = root.path().join("local/relay/settings.json");
        let original = std::fs::read(&settings).unwrap();
        let mut state = State::new();
        state.store.root = root.path().to_path_buf();
        state.relay_refresh();
        state.hex.shell.device_name = TextEdit::new("Studio PC");
        save_device_name(&mut state);
        assert_eq!(stored_device_name(root.path()), "Studio PC");

        let mut runtime = rune::Runtime::new(state, view, (600.0, 300.0), rune::Env::default());
        runtime.frame(0.0);
        let [left, top, width, height] = runtime
            .elements()
            .into_iter()
            .find(|el| el.key.as_deref() == Some("relay-preview-toggle"))
            .unwrap()
            .rect;
        let (x, y) = (left + width / 2.0, top + height / 2.0);
        runtime.pointer_move(x, y);
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(1.0 / 60.0);

        let state = &mut runtime.state;
        assert_eq!(
            state.feature_error.as_deref(),
            Some("Could not save relay preview settings.")
        );
        assert!(!state.hex.shell.relay.enabled);
        assert_eq!(
            state.hex.shell.relay.unavailable_reason.as_deref(),
            Some(sidenote_sync::RELAY_UNAVAILABLE)
        );
        assert!(!state.relay_ui.pairing_busy());
        assert_eq!(std::fs::read(&settings).unwrap(), original);
        assert_eq!(stored_device_name(root.path()), "Studio PC");

        state.hex.shell.device_name = TextEdit::new("Travel laptop");
        save_device_name(state);
        assert_eq!(state.hex.shell.device_name_error, None);
        assert_eq!(stored_device_name(root.path()), "Travel laptop");
        assert_eq!(std::fs::read(settings).unwrap(), original);
    }

    #[test]
    fn merged_kb_validation_reason_reaches_the_inline_error_without_raw_io_details() {
        crate::test_paths::isolate();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        std::fs::write(&path, r#"{"version":1,"features":{},"notes":{}}"#).unwrap();
        let mut draft = sidenote::core::Config::load_in(root.path()).unwrap();
        draft.knowledge_base.company_name = "Synthetic company".into();
        let error = draft.save(root.path()).unwrap_err();
        let message = kb_save_error(&error);
        assert!(message.contains("merging other saved changes"));
        assert!(message.contains("Enter the repository folder."));
        assert!(message.contains("Reload Settings"));
        assert!(!message.contains("Synthetic company"));
        assert_eq!(
            kb_save_error(&std::io::Error::other("Synthetic private path")),
            "Could not save knowledge base. Reload Settings and try again."
        );
    }

    #[test]
    fn kb_edit_preserves_untouched_values_and_meaningful_path_whitespace() {
        let mut base = sidenote::core::config::KnowledgeBase::default();
        base.repo_path = " E:/example/kb ".into();
        base.revision = "origin/main".into();
        base.source_roots = vec![" handbook".into(), "docs ".into()];
        base.company_name = " Example company ".into();
        base.people_roster_source = " people/roster.md".into();
        let mut fields = [
            TextEdit::new(&base.repo_path),
            TextEdit::new(&base.revision),
            TextEdit::new(base.source_roots.join("\n")),
            TextEdit::new(&base.company_name),
            TextEdit::new(&base.people_roster_source),
        ];
        assert_eq!(
            serde_json::to_value(kb_edits(&base, &fields)).unwrap(),
            serde_json::to_value(&base).unwrap()
        );
        fields[3] = TextEdit::new("Example revision");
        let edited = kb_edits(&base, &fields);
        assert_eq!(edited.repo_path, base.repo_path);
        assert_eq!(edited.revision, base.revision);
        assert_eq!(edited.source_roots, base.source_roots);
        assert_eq!(edited.people_roster_source, base.people_roster_source);
        assert_eq!(edited.company_name, "Example revision");
        fields[2] = TextEdit::new(" handbook\ndocs \n another folder ");
        assert_eq!(
            kb_edits(&base, &fields).source_roots,
            [" handbook", "docs ", " another folder "]
        );
    }

    #[test]
    fn final_review_uses_one_meetings_tab_row_and_scopes_hex_ownership() {
        assert!(jump_groups(Page::Meetings, 0).is_empty());
        assert_eq!(
            Page::Meetings.tabs(),
            ["Recording", "Summaries", "Daily check", "Engine"]
        );
        assert_eq!(current(Page::Meetings.section(3)), (Page::Meetings, 3));
        assert_eq!(jump_groups(Page::Hex, 0).len(), 7);
        assert_eq!(
            ownership_text(Page::Hex),
            "These settings live in Hex and also change Hex."
        );
        assert_eq!(
            ownership_text(Page::Meetings),
            "Settings labeled Hex also change Hex."
        );
        let fixture: Vec<Setting> = serde_json::from_str(include_str!(
            "../../../tools/ui-fixtures/settings-recording.json"
        ))
        .unwrap();
        for st in fixture.iter().filter(|st| {
            matches!(
                st.key.as_str(),
                "meetings.summaries" | "meetings.opencode_model"
            )
        }) {
            assert_eq!(location(st), (Page::Meetings, 1, false));
        }
    }
    #[test]
    fn approved_search_map_keeps_all_destinations() {
        let map: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tools/ui-fixtures/settings-search-map.json"
        ))
        .unwrap();
        let rows = map.as_array().unwrap();
        assert_eq!(rows.len(), 50);
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../tools/ui-fixtures/hex.json")).unwrap();
        let mut state = State::new();
        state.hex.settings = fixture.settings;
        for feature in AppFeature::EXPERIMENTAL {
            if !state.config.features.enabled(feature) {
                state.config.features.toggle(feature);
            }
        }
        // The legacy map includes Dev destinations and the Windows capture setting.
        let mut entries = index_for_channel(&state, Channel::Dev);
        if !cfg!(windows) {
            entries.push(meeting_icon_capture_destination());
        }
        for row in rows {
            let key = row["key"].as_str().unwrap();
            let alias = legacy_aliases()
                .iter()
                .find(|a| a.words.split_whitespace().any(|w| w == key))
                .unwrap_or_else(|| panic!("Missing alias {key}"));
            let dest = entries
                .iter()
                .find(|d| d.key == alias.target)
                .unwrap_or_else(|| panic!("Missing destination {key}"));
            assert!(
                row["home"].as_str().unwrap().starts_with(dest.page.name()),
                "Wrong page for {key}"
            );
            assert!(
                dest.tab < dest.page.tabs().len().max(1),
                "Invalid tab for {key}"
            );
        }
    }
    #[cfg(windows)]
    #[test]
    fn meeting_icon_capture_aliases_jump_to_general_settings() {
        let mut s = State::new();
        for query in [
            "screenshot",
            "screenshots",
            "screen recording",
            "recording",
            "screen share",
            "sharing",
            "nub",
            "meeting icon",
            "floating icon",
            "hide icon",
            "show icon",
        ] {
            navigate(&mut s, Page::Meetings, 0);
            s.hex.filter = TextEdit::new(query);
            let destination = results(&s)
                .into_iter()
                .find(|entry| entry.key == MEETING_ICON_CAPTURE)
                .unwrap_or_else(|| panic!("Missing meeting icon result for {query}"));
            jump(&mut s, destination);
            assert_eq!(active(&s), (Page::General, 0), "{query}");
            assert_eq!(
                s.hex.shell.highlighted.as_ref().unwrap().0,
                MEETING_ICON_CAPTURE
            );
            assert_eq!(s.pending_scroll.as_ref().unwrap().key, MEETING_ICON_CAPTURE);
        }
    }

    #[test]
    fn settings_navigation_resolves_to_solar_at_every_weight() {
        let expected = [
            (Page::General, "solar-user"),
            (Page::Meetings, "solar-document"),
            (Page::Prompts, "solar-notebook-minimalistic"),
            (Page::Experimental, "solar-flask"),
            (Page::Tasks, "solar-check-circle"),
            (Page::Helpers, "solar-four-point-star"),
            (Page::ConnectedApps, "solar-link"),
            (Page::Hex, "solar-microphone-3"),
            (Page::Appearance, "solar-sun"),
            (Page::Sync, "solar-monitor-smartphone"),
            (Page::Privacy, "solar-lock-keyhole-minimalistic"),
            (Page::Updates, "solar-download-minimalistic"),
            (Page::Diagnostics, "solar-pulse"),
        ];
        for (page, glyph) in expected {
            icons::assert_solar_weights(page.icon(), glyph);
        }
        icons::assert_solar_weights(Page::People.icon(), "solar-users-group-rounded");
        let mic = row_icon("meetings.record_my_microphone");
        icons::assert_solar_weights(mic, "solar-microphone-3");
    }

    #[test]
    fn groups_headings_follow_spec_order() {
        crate::test_paths::isolate();
        let mut state = State::new();
        state.open_hex();
        let mut rt = rune::Runtime::with_text(
            state,
            crate::ui::view,
            (1334.0, 900.0),
            rune::Env::default(),
            rune::text::TextSystem::new(
                crate::ui::theme::fonts(),
                (crate::ui::theme::SANS, "Inter"),
            ),
        );
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        let mut ordered: Vec<(&str, f32)> = elements
            .iter()
            .filter_map(|element| {
                let key = element.key.as_deref()?;
                if key.starts_with("settings-heading-") || key.starts_with("settings-page-") {
                    Some((key, element.rect[1]))
                } else {
                    None
                }
            })
            .collect();
        ordered.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let actual: Vec<&str> = ordered.iter().map(|(key, _)| *key).collect();
        // Spec section 2 order.
        let groups: [(&str, &[Page]); 5] = [
            (
                "settings-heading-you-and-meetings",
                &[
                    Page::General,
                    Page::Meetings,
                    Page::Prompts,
                    Page::Experimental,
                ],
            ),
            ("settings-heading-your-work", &[Page::Tasks]),
            (
                "settings-heading-ask-and-agents",
                &[Page::Helpers, Page::ConnectedApps],
            ),
            (
                "settings-heading-this-pc",
                &[Page::Hex, Page::Appearance, Page::Sync],
            ),
            (
                "settings-heading-data-and-maintenance",
                &[Page::Privacy, Page::Updates, Page::Diagnostics],
            ),
        ];
        let mut expected: Vec<String> = Vec::new();
        for (heading, pages) in groups {
            let visible: Vec<Page> = pages
                .iter()
                .copied()
                .filter(|page| page.visible(CHANNEL))
                .collect();
            if visible.is_empty() {
                continue;
            }
            expected.push(heading.to_owned());
            for page in visible {
                expected.push(format!("settings-page-{}", page.name()));
            }
        }
        assert_eq!(actual, expected);
        for (heading, label) in [
            ("settings-heading-you-and-meetings", "You and meetings"),
            ("settings-heading-ask-and-agents", "Ask and agents"),
            ("settings-heading-this-pc", "This PC"),
            (
                "settings-heading-data-and-maintenance",
                "Data and maintenance",
            ),
        ] {
            if expected.iter().any(|key| key == heading) {
                // The keyed row holds the label as a child text element.
                let row = elements
                    .iter()
                    .find(|element| element.key.as_deref() == Some(heading))
                    .unwrap_or_else(|| panic!("missing {heading}"));
                let inside = format!("{}/", row.path);
                assert!(
                    elements
                        .iter()
                        .any(|element| element.path.starts_with(&inside)
                            && element.text.as_deref() == Some(label)),
                    "{heading} shows {label:?}"
                );
            }
        }
    }

    #[test]
    fn groups_headings_fit_in_the_drawer_at_phone_width() {
        crate::test_paths::isolate();
        let mut state = State::new();
        state.open_hex();
        let mut rt = rune::Runtime::with_text(
            state,
            crate::ui::view,
            (390.0, 844.0),
            rune::Env::default(),
            rune::text::TextSystem::new(
                crate::ui::theme::fonts(),
                (crate::ui::theme::SANS, "Inter"),
            ),
        );
        rt.settle_all();
        rt.frame(0.0);
        let (x, y) = rt
            .find("settings-navigation")
            .unwrap_or_else(|| panic!("missing settings-navigation"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.2);
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        let rect = |key: &str| {
            elements
                .iter()
                .find(|element| element.key.as_deref() == Some(key))
                .unwrap_or_else(|| panic!("missing {key}"))
                .rect
        };
        let drawer = rect("settings-drawer");
        assert!(drawer[3] > 0.0);
        let headings: Vec<[f32; 4]> = elements
            .iter()
            .filter(|element| {
                element
                    .key
                    .as_deref()
                    .is_some_and(|key| key.starts_with("settings-heading-"))
            })
            .map(|element| element.rect)
            .collect();
        assert!(!headings.is_empty());
        let rows: Vec<[f32; 4]> = elements
            .iter()
            .filter(|element| {
                element
                    .key
                    .as_deref()
                    .is_some_and(|key| key.starts_with("settings-page-"))
            })
            .map(|element| element.rect)
            .collect();
        assert!(!rows.is_empty());
        for heading in &headings {
            assert!(
                heading[0] >= drawer[0] - 1.0
                    && heading[1] >= drawer[1] - 1.0
                    && heading[0] + heading[2] <= drawer[0] + drawer[2] + 1.0
                    && heading[1] + heading[3] <= drawer[1] + drawer[3] + 1.0,
                "heading {heading:?} outside drawer {drawer:?}"
            );
            for row in &rows {
                let overlap = heading[0] < row[0] + row[2]
                    && row[0] < heading[0] + heading[2]
                    && heading[1] < row[1] + row[3]
                    && row[1] < heading[1] + heading[3];
                assert!(!overlap, "heading {heading:?} overlaps row {row:?}");
            }
        }
    }

    #[test]
    fn groups_no_heading_without_a_visible_page() {
        // Runs in both the Stable and Dev test builds, so each channel's page set is checked.
        crate::test_paths::isolate();
        let mut state = State::new();
        state.open_hex();
        let mut rt = rune::Runtime::with_text(
            state,
            crate::ui::view,
            (1334.0, 900.0),
            rune::Env::default(),
            rune::text::TextSystem::new(
                crate::ui::theme::fonts(),
                (crate::ui::theme::SANS, "Inter"),
            ),
        );
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        let headings: Vec<&str> = elements
            .iter()
            .filter_map(|element| element.key.as_deref())
            .filter(|key| key.starts_with("settings-heading-"))
            .collect();
        let distinct: HashSet<&str> = Page::ALL
            .into_iter()
            .filter(|page| page.visible(CHANNEL))
            .map(|page| page.group())
            .collect();
        assert_eq!(headings.len(), distinct.len());
        for heading in &headings {
            let slug = heading
                .strip_prefix("settings-heading-")
                .unwrap_or_else(|| panic!("bad heading key {heading}"));
            assert!(
                Page::ALL
                    .into_iter()
                    .filter(|page| page.visible(CHANNEL))
                    .any(|page| page.group().to_ascii_lowercase().replace(' ', "-") == *slug),
                "{heading} has no visible page"
            );
        }
    }

    #[test]
    fn groups_search_hides_headings() {
        crate::test_paths::isolate();
        let mut state = State::new();
        state.open_hex();
        state.hex.filter = TextEdit::new("sound");
        let mut rt = rune::Runtime::with_text(
            state,
            crate::ui::view,
            (1334.0, 900.0),
            rune::Env::default(),
            rune::text::TextSystem::new(
                crate::ui::theme::fonts(),
                (crate::ui::theme::SANS, "Inter"),
            ),
        );
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        assert!(
            elements
                .iter()
                .filter_map(|element| element.key.as_deref())
                .all(|key| !key.starts_with("settings-heading-")
                    && !key.starts_with("settings-page-")),
            "search results replaced the grouped list"
        );
        assert!(
            elements.iter().any(|element| element
                .key
                .as_deref()
                .is_some_and(|key| key.starts_with("find-setting-"))),
            "expected search results for \"sound\""
        );
    }

    fn settings_runtime(state: State, size: (f32, f32)) -> rune::Runtime<State> {
        let mut rt = rune::Runtime::with_text(
            state,
            crate::ui::view,
            size,
            rune::Env::default(),
            rune::text::TextSystem::new(
                crate::ui::theme::fonts(),
                (crate::ui::theme::SANS, "Inter"),
            ),
        );
        rt.settle_all();
        rt.frame(0.0);
        rt
    }

    fn work_tasks_state(root: Option<&std::path::Path>) -> State {
        crate::test_paths::isolate();
        let mut state = State::new();
        if let Some(root) = root {
            state.store.root = root.to_path_buf();
            state.tasks.store = sidenote::core::TaskStore::new(root.to_path_buf());
            state.tasks.layout = TasksLayout::List;
        }
        state.config.features.tasks = true;
        state.open_hex();
        navigate(&mut state, Page::Tasks, 0);
        state
    }

    fn tap_key(rt: &mut rune::Runtime<State>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.2);
    }

    fn element_rect(elements: &[rune::ElementInfo], key: &str) -> [f32; 4] {
        elements
            .iter()
            .find(|element| element.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("missing {key}"))
            .rect
    }

    fn row_shows_text(elements: &[rune::ElementInfo], key: &str, text: &str) -> bool {
        let Some(row) = elements
            .iter()
            .find(|element| element.key.as_deref() == Some(key))
        else {
            return false;
        };
        let inside = format!("{}/", row.path);
        elements.iter().any(|element| {
            element.path.starts_with(&inside) && element.text.as_deref() == Some(text)
        })
    }

    fn choose_select(rt: &mut rune::Runtime<State>, trigger: &str, option: usize) {
        tap_key(rt, trigger);
        rt.settle_all();
        rt.frame(0.0);
        let key = format!("option{option}");
        tap_key(rt, &key);
        rt.settle_all();
        rt.frame(0.0);
    }

    fn choose_tasks_layout(rt: &mut rune::Runtime<State>, option: usize) {
        choose_select(rt, "work-tasks-layout-select-trigger", option);
    }

    fn select_options(rt: &mut rune::Runtime<State>, trigger: &str) -> Vec<String> {
        tap_key(rt, trigger);
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        elements
            .iter()
            .filter(|element| {
                element
                    .key
                    .as_deref()
                    .is_some_and(|key| key.starts_with("option"))
            })
            .filter_map(|element| element.label.clone())
            .collect()
    }

    #[test]
    fn work_tasks_page_is_listed_under_your_work() {
        for size in [(1334.0, 900.0), (390.0, 844.0)] {
            let state = work_tasks_state(None);
            let mut rt = settings_runtime(state, size);
            if size.0 < DRAWER_AT {
                tap_key(&mut rt, "settings-navigation");
                rt.settle_all();
                rt.frame(0.0);
            }
            let elements = rt.elements();
            let heading = element_rect(&elements, "settings-heading-your-work");
            let page = element_rect(&elements, "settings-page-Tasks");
            let next = element_rect(&elements, "settings-heading-ask-and-agents");
            assert!(heading[1] < page[1], "{size:?}: heading above page");
            assert!(page[1] < next[1], "{size:?}: page before Ask and agents");
            assert!(
                row_shows_text(&elements, "settings-heading-your-work", "Your work"),
                "{size:?}: heading label"
            );
        }
    }

    #[test]
    fn work_tasks_page_hides_when_tasks_are_off() {
        crate::test_paths::isolate();
        let mut state = State::new();
        state.config.features.tasks = false;
        state.open_hex();
        state.hex.section = Section::Tasks;
        let mut rt = settings_runtime(state, (1334.0, 900.0));
        assert_eq!(active(&rt.state), (Page::General, 0));
        let elements = rt.elements();
        assert!(
            elements
                .iter()
                .all(|element| element.key.as_deref() != Some("settings-page-Tasks"))
        );
        assert!(
            elements
                .iter()
                .all(|element| element.key.as_deref() != Some("settings-heading-your-work"))
        );
        rt.update(|s, _| s.hex.filter = TextEdit::new("board"));
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        assert!(
            elements
                .iter()
                .all(|element| element.key.as_deref() != Some("find-setting-work-tasks-layout"))
        );

        rt.update(|s, _| {
            s.open_finder();
            s.finder_query = TextEdit::new("tasks");
            s.finder_changed();
        });
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        assert!(
            elements
                .iter()
                .filter(|element| {
                    element
                        .key
                        .as_deref()
                        .is_some_and(|key| key.starts_with("finder-row-"))
                })
                .all(|element| element.label.as_deref() != Some("Settings · Tasks")),
            "the finder offered Settings · Tasks with Tasks off"
        );
    }

    #[test]
    fn work_tasks_layout_row_writes_the_same_pref_as_the_toolbar() {
        for size in [(1334.0, 900.0), (390.0, 844.0)] {
            let root = tempfile::tempdir().unwrap();
            let state = work_tasks_state(Some(root.path()));
            let mut rt = settings_runtime(state, size);
            choose_tasks_layout(&mut rt, 1);
            assert_eq!(UiPrefs::load(root.path()).tasks_layout, TasksLayout::Board);
            assert_eq!(rt.state.tasks.layout, TasksLayout::Board);
            assert_eq!(rt.state.feature_error, None, "{size:?}");
            choose_tasks_layout(&mut rt, 0);
            assert_eq!(UiPrefs::load(root.path()).tasks_layout, TasksLayout::List);
            assert_eq!(rt.state.tasks.layout, TasksLayout::List);
            assert_eq!(rt.state.feature_error, None, "{size:?}");
        }
    }

    #[test]
    fn work_tasks_layout_save_failure_is_shown() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("local/ui.json")).unwrap();
        let mut state = work_tasks_state(Some(root.path()));
        // An earlier save's confirmation must not stay beside the error.
        state.hex.shell.saved = Some(Instant::now() + Duration::from_secs(60));
        let mut rt = settings_runtime(state, (1334.0, 900.0));
        choose_tasks_layout(&mut rt, 1);
        assert_eq!(
            rt.state.feature_error.as_deref(),
            Some("Could not save Tasks settings. Try again.")
        );
        assert!(rt.state.hex.shell.saved.is_none());
        // Going back to List must not pass because the unreadable file loads as List.
        choose_tasks_layout(&mut rt, 0);
        assert_eq!(
            rt.state.feature_error.as_deref(),
            Some("Could not save Tasks settings. Try again.")
        );
        assert!(rt.state.hex.shell.saved.is_none());
        let elements = rt.elements();
        assert!(
            elements.iter().any(|element| element.text.as_deref()
                == Some("Could not save Tasks settings. Try again.")),
            "the save failure is rendered"
        );
        assert!(
            elements
                .iter()
                .all(|element| element.text.as_deref() != Some("Saved")),
            "a failed save must not say Saved"
        );
    }

    #[test]
    fn work_tasks_grouping_row_lists_six_values_and_hides_project_without_projects() {
        for (projects, expected) in [
            (true, "Due date,Project,Meeting,Team,Status,Owner"),
            (false, "Due date,Meeting,Team,Status,Owner"),
        ] {
            let mut state = work_tasks_state(None);
            state.config.features.projects = projects;
            let mut rt = settings_runtime(state, (1334.0, 900.0));
            let options = select_options(&mut rt, "work-tasks-grouping-select-trigger");
            assert_eq!(options.join(","), expected);
        }
    }

    #[test]
    fn work_tasks_grouping_row_updates_tasks_and_saves() {
        let root = tempfile::tempdir().unwrap();
        let mut state = work_tasks_state(Some(root.path()));
        state.config.features.projects = true;
        let mut rt = settings_runtime(state, (1334.0, 900.0));
        choose_select(&mut rt, "work-tasks-grouping-select-trigger", 5);
        assert_eq!(rt.state.tasks.grouping, TaskGrouping::Owner);
        assert!(rt.state.tasks.grouping_loaded);
        assert_eq!(
            rt.state.tasks.store.load_grouping().ok(),
            Some(TaskGrouping::Owner)
        );
        assert_eq!(rt.state.feature_error, None);
    }

    #[test]
    fn work_tasks_grouping_row_shows_the_saved_value_before_tasks_ever_opened() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("local")).unwrap();
        let grouping = serde_json::to_string(&TaskGrouping::Owner).unwrap();
        std::fs::write(root.path().join("local/task-grouping.json"), grouping).unwrap();
        let state = work_tasks_state(Some(root.path()));
        assert!(!state.tasks.grouping_loaded);
        let mut rt = settings_runtime(state, (1334.0, 900.0));
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        assert!(row_shows_text(&elements, "work-tasks-grouping", "Owner"));
    }

    #[test]
    fn work_tasks_who_row_maps_three_choices_to_the_two_prefs() {
        for (option, mine, unassigned, label) in [
            (0, false, false, "Filters"),
            (1, true, false, "Filters: Mine"),
            (2, true, true, "Filters: Mine + unassigned"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut state = work_tasks_state(Some(root.path()));
            // Show must leave the saved Open filter alone, even when the page shows All.
            state.tasks.done = sidenote::core::DoneFilter::All;
            let mut rt = settings_runtime(state, (1334.0, 900.0));
            choose_select(&mut rt, "work-tasks-who-select-trigger", option);
            let prefs = UiPrefs::load(root.path());
            assert_eq!(
                (prefs.tasks_mine, prefs.tasks_include_unassigned),
                (mine, unassigned)
            );
            assert_eq!(prefs.tasks_done, sidenote::core::DoneFilter::Open);
            rt.update(|s, _| s.set_view(tasks::View::Tasks));
            rt.settle_all();
            rt.frame(0.0);
            let elements = rt.elements();
            let trigger = elements
                .iter()
                .find(|element| element.key.as_deref() == Some("task-filters"))
                .unwrap();
            assert_eq!(trigger.label.as_deref(), Some(label));
        }
    }

    #[test]
    fn work_tasks_rows_fit_at_phone_width() {
        let mut rt = settings_runtime(work_tasks_state(None), (390.0, 844.0));
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        let scroll = element_rect(&elements, "hex-scroll");
        for key in ["work-tasks-layout", "work-tasks-grouping", "work-tasks-who"] {
            let row = element_rect(&elements, key);
            assert!(
                row[0] >= scroll[0] - 0.5 && row[0] + row[2] <= scroll[0] + scroll[2] + 0.5,
                "{key} overflows horizontally"
            );
        }
        let max = short_select_width(&rt.state, 180.0);
        for key in [
            "work-tasks-layout-select-trigger",
            "work-tasks-grouping-select-trigger",
            "work-tasks-who-select-trigger",
        ] {
            assert!(
                element_rect(&elements, key)[2] <= max + 0.5,
                "{key} is too wide"
            );
        }
    }

    #[test]
    fn work_tasks_grouping_save_failure_is_shown() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("local/task-grouping.json")).unwrap();
        let mut state = work_tasks_state(Some(root.path()));
        state.tasks.grouping_loaded = true;
        state.hex.shell.saved = Some(Instant::now() + Duration::from_secs(60));
        let mut rt = settings_runtime(state, (1334.0, 900.0));
        choose_select(&mut rt, "work-tasks-grouping-select-trigger", 4);
        assert_eq!(
            rt.state.feature_error.as_deref(),
            Some("Could not save Tasks settings. Try again.")
        );
        assert!(rt.state.hex.shell.saved.is_none());
    }

    #[test]
    fn work_tasks_grouping_waits_for_a_queued_save() {
        let root = tempfile::tempdir().unwrap();
        let mut state = work_tasks_state(Some(root.path()));
        state.tasks.grouping_loaded = true;
        state.edit_busy = true;
        let mut rt = settings_runtime(state, (1334.0, 900.0));
        tap_key(&mut rt, "work-tasks-grouping-select-trigger");
        rt.settle_all();
        rt.frame(0.0);
        assert_eq!(rt.state.hex.select_open, None);
        assert_eq!(rt.state.tasks.grouping, TaskGrouping::Status);
        assert!(!root.path().join("local/task-grouping.json").exists());
    }

    #[test]
    fn work_tasks_grouping_shows_due_date_for_a_saved_project_without_projects() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("local")).unwrap();
        let grouping = serde_json::to_string(&TaskGrouping::Project).unwrap();
        std::fs::write(root.path().join("local/task-grouping.json"), grouping).unwrap();
        let mut state = work_tasks_state(Some(root.path()));
        state.config.features.projects = false;
        let rt = settings_runtime(state, (1334.0, 900.0));
        let elements = rt.elements();
        assert!(row_shows_text(&elements, "work-tasks-grouping", "Due date"));
    }

    #[test]
    fn work_tasks_who_save_failure_is_shown() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("local/ui.json")).unwrap();
        let mut state = work_tasks_state(Some(root.path()));
        state.hex.shell.saved = Some(Instant::now() + Duration::from_secs(60));
        let mut rt = settings_runtime(state, (1334.0, 900.0));
        choose_select(&mut rt, "work-tasks-who-select-trigger", 1);
        assert_eq!(
            rt.state.feature_error.as_deref(),
            Some("Could not save Tasks settings. Try again.")
        );
        assert!(rt.state.hex.shell.saved.is_none());
    }

    #[test]
    fn work_tasks_search_finds_the_row() {
        let mut rt = settings_runtime(work_tasks_state(None), (1334.0, 900.0));
        rt.update(|s, _| {
            navigate(s, Page::Meetings, 0);
            s.hex.filter = TextEdit::new("board");
        });
        rt.settle_all();
        rt.frame(0.0);
        // Click the rendered result, so a broken handler or scroll fails here.
        tap_key(&mut rt, "find-setting-work-tasks-layout");
        rt.settle_all();
        rt.frame(0.0);
        assert_eq!(active(&rt.state), (Page::Tasks, 0));
        assert_eq!(
            rt.state.hex.shell.highlighted.as_ref().unwrap().0,
            "work-tasks-layout"
        );
        let elements = rt.elements();
        let [_, top, _, height] = element_rect(&elements, "hex-scroll");
        let [_, y, _, h] = element_rect(&elements, "work-tasks-layout");
        assert!(y >= top && y + h <= top + height, "the row is on screen");
    }

    #[test]
    fn work_tasks_page_leaves_the_ask_dock_clear() {
        let state = work_tasks_state(None);
        let mut rt = settings_runtime(state, (390.0, 844.0));
        rt.update(|s, _| crate::ui::scene_agents(s, crate::ui::ALL_AGENTS));
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        let [_, scroll_y, _, scroll_h] = element_rect(&elements, "hex-scroll");
        let [dock_x, dock_y, dock_w, _] = element_rect(&elements, "ask-overlay");
        let [pill_x, pill_y, pill_w, pill_h] = element_rect(&elements, "ask-pill");
        let [_, input_y, _, input_h] = element_rect(&elements, "ask-input");
        assert!(scroll_y + scroll_h <= dock_y + 1.0);
        assert_eq!(pill_h, 48.0);
        assert!((pill_x + pill_w / 2.0 - dock_x - dock_w / 2.0).abs() <= 1.0);
        assert!((input_y + input_h / 2.0 - pill_y - pill_h / 2.0).abs() <= 1.0);
    }

    #[test]
    fn ask_rows_have_distinct_requested_lucide_icons_at_the_shared_stroke() {
        let rows = [
            ("ask-agent-row", "lucide-bot"),
            ("ask-model-row", "lucide-cpu"),
            ("ask-meeting-thinking-row", "lucide-timer"),
            ("ask-chat-thinking-row", "lucide-message-square"),
        ];
        let mut seen = HashSet::new();
        seen.insert(Page::Helpers.icon().name);
        assert_eq!(icons::STROKE_WIDTH, 2.0);
        for (key, name) in rows {
            let icon = row_icon(key);
            assert_eq!(icon.name, name);
            assert!(seen.insert(icon.name));
            assert!(icon.body.contains("stroke-width=\"2\""));
        }
    }
    #[test]
    fn every_old_hex_setting_has_a_reachable_destination() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../tools/ui-fixtures/hex.json")).unwrap();
        for setting in fixture.settings {
            let (page, tab, _) = location(&setting);
            assert!(Page::ALL.contains(&page), "{}", setting.key);
            assert!(tab < page.tabs().len().max(1), "{}", setting.key);
            assert_eq!(current(page.section(tab)), (page, tab));
        }
    }
    #[test]
    fn connected_apps_search_and_navigation() {
        let entries = app_index();
        let destination = entries.iter().find(|d| d.key == "mcp-connect").unwrap();
        for query in ["MCP", "connect", "Claude", "Codex", "Cursor", "OpenCode"] {
            assert!(matches(destination, query), "{query}");
        }
        assert_eq!(destination.page, Page::ConnectedApps);
        assert_eq!(
            current(Page::ConnectedApps.section(0)),
            (Page::ConnectedApps, 0)
        );
        let pages: Vec<_> = Page::ALL.into_iter().collect();
        let index = pages.iter().position(|p| *p == Page::Helpers).unwrap();
        assert_eq!(pages[index + 1], Page::ConnectedApps);
    }

    #[test]
    fn privacy_connected_apps_disclosure_fits_and_opens_the_page_on_dev() {
        crate::test_paths::isolate();
        let mut state = State::new();
        state.open_hex();
        // Keep the navigation test from discovering apps on this machine.
        state.connection.busy = true;
        navigate(&mut state, Page::Privacy, 0);
        let mut rt = rune::Runtime::with_text(
            state,
            crate::ui::view,
            (390.0, 900.0),
            rune::Env::default(),
            rune::text::TextSystem::new(
                crate::ui::theme::fonts(),
                (crate::ui::theme::SANS, "Inter"),
            ),
        );
        rt.settle_all();
        rt.frame(0.0);
        let elements = rt.elements();
        let disclosure = elements.iter().find(|element| {
            element
                .text
                .as_deref()
                .is_some_and(|text| text.starts_with("Connected apps can read meetings"))
        });
        let action = elements
            .iter()
            .find(|element| element.key.as_deref() == Some("privacy-connected-apps"));
        if CHANNEL == Channel::Stable {
            assert!(disclosure.is_none() && action.is_none());
            return;
        }
        let details = disclosure.unwrap().rect;
        let action = action.unwrap().rect;
        let summary = elements
            .iter()
            .find(|element| element.key.as_deref() == Some("ai-notes-settings"))
            .unwrap()
            .rect;
        assert!(details[1] > summary[1] + summary[3]);
        assert_eq!(action[0], details[0]);
        assert!(action[1] > details[1] + details[3]);
        assert!(action[0] + action[2] <= details[0] + details[2]);

        let x = action[0] + action[2] / 2.0;
        let y = action[1] + action[3] / 2.0;
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.2);
        assert_eq!(active(&rt.state), (Page::ConnectedApps, 0));
        assert!(rt.find("mcp-connect").is_some());
    }

    #[test]
    fn stable_hides_sync_and_external_app_setup() {
        let state = State::new();
        let destinations = index(&state);
        for page in [Page::Sync, Page::ConnectedApps] {
            assert!(!page.visible(Channel::Stable));
            assert!(page.visible(Channel::Dev));
            assert_eq!(
                destinations
                    .iter()
                    .any(|destination| destination.page == page),
                CHANNEL == Channel::Dev
            );
        }
        assert!(Page::Helpers.visible(Channel::Stable));
    }
    #[test]
    fn legacy_search_words_find_the_new_destinations() {
        let entries = app_index();
        for (query, key, page, tab) in [
            ("AI notes", "notes-options", Page::Meetings, 1),
            ("AI helpers", "ai-notes-settings", Page::Helpers, 0),
            ("Vocabulary", "hex-vocabulary", Page::Meetings, 0),
            ("Teams", "settings-teams", Page::People, 0),
        ] {
            let result = entries.iter().find(|dest| dest.key == key).unwrap();
            assert!(matches(result, query));
            assert_eq!((result.page, result.tab), (page, tab));
        }
    }
    #[test]
    fn recording_rows_have_distinct_icons() {
        let keys = [
            "meetings.auto_start",
            "meetings.record_my_microphone",
            "languages-row",
            "vocabulary-row",
            "keep-audio",
            "meetings.retention",
            "meetings.silence_threshold",
            "meetings.output_folder",
        ];
        let mut names = HashSet::new();
        for key in keys {
            let icon = row_icon(key);
            assert!(names.insert(icon.name), "Repeated icon for {key}");
            assert_ne!(icon.name, icons::MEETINGS.name);
        }
        assert_eq!(row_icon("meetings.auto_start").name, "lucide-phone-call");
        assert_eq!(row_icon("notes-identity").name, "lucide-user-round");
        assert_eq!(row_icon("summary-template").name, "lucide-file-text");
    }
    #[test]
    fn old_deep_links_and_tab_limits_are_preserved() {
        assert_eq!(current(Section::AiNotes), (Page::Meetings, 1));
        assert_eq!(current(Section::Teams), (Page::People, 0));
        assert_eq!(current(Section::Model), (Page::Hex, 0));
        for page in Page::ALL {
            assert!(page.tabs().len() <= 4);
        }
    }

    #[test]
    fn experimental_settings_search_finds_all_switches_with_experiments_off() {
        let mut state = State::new();
        for feature in AppFeature::EXPERIMENTAL {
            if state.config.features.enabled(feature) {
                state.config.features.toggle(feature);
            }
        }
        navigate(&mut state, Page::Experimental, 0);
        assert_eq!(active(&state), (Page::Experimental, 0));
        let entries = index(&state);
        let switches: Vec<_> = entries
            .iter()
            .filter(|entry| entry.page == Page::Experimental)
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(switches, AppFeature::EXPERIMENTAL.map(AppFeature::name));
        assert!(entries.iter().all(|entry| entry.page != Page::People));
        for key in [
            "feature-Daily check",
            "knowledge-base",
            "annotation-agent-row",
            "annotation-model-row",
            "engine-model-hi",
            "engine-model-te",
            "spoken-languages",
            "meeting-language-mode",
            "labs-process-windows",
            "engine-translation-model",
            "engine-prompt-translation",
        ] {
            assert!(entries.iter().all(|entry| entry.key != key), "{key}");
        }
        assert!(entries.iter().any(|entry| entry.key == "engine-model-en"));
    }
}
