//! The left sidebar and the top-right actions.
//!
//! The sidebar holds places and project filters, with one profile button at its foot.
//! It folds to a recording rail during a call and hides fully otherwise.
//! Lucide entries live in icons.rs; the app mark keeps its original colours.
use super::{
    State, hex, icons, roles,
    tasks::View,
    theme::Palette,
    widgets::{dot, menu_row, menu_separator, menu_surface, menu_trigger, pressable},
};
use chrono::{DateTime, Local};
use rune::{Color, El, col, el, row};
use sidenote::core::{self, UiPrefs};
use std::collections::HashMap;

/// The open sidebar, and the rail it folds to, in points.
pub const WIDE: f32 = 204.0;
pub const RAIL: f32 = 56.0;
/// A window this wide or narrower starts with the sidebar folded.
pub const COLLAPSE_AT: f32 = 820.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Companies,
    Meetings,
    Annotations,
    Tasks,
    People,
    Memory,
    Check,
    Hex,
}

/// What the home list shows.
#[derive(Clone, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Short,
    /// Meetings whose to-dos mostly sit in this project.
    Project(String),
    Area(String),
    Areas,
    /// Meetings that came from another device.
    Peers,
}

/// Looked up once per rebuild, not per frame: each meeting's project, how many meetings and
/// the newest meeting in each project, and the meetings from other devices.
#[derive(Default)]
pub struct NavCache {
    pub meeting_project: HashMap<String, String>,
    pub project_counts: HashMap<String, usize>,
    /// When each project last met, to list the most recent ones first.
    pub project_latest: HashMap<String, DateTime<Local>>,
    pub peers: usize,
    /// "Show all" under Projects. Kept across rebuilds for the session, never saved.
    pub show_all: bool,
    pub profile_menu: bool,
    pub new_menu: bool,
}

impl State {
    pub fn page(&self) -> Page {
        if self.companies.open {
            Page::Companies
        } else if self.annotations.open {
            Page::Annotations
        } else if self.view == View::Tasks {
            Page::Tasks
        } else if self.view == View::People {
            Page::People
        } else if self.on_memory() {
            Page::Memory
        } else if self.on_check() {
            Page::Check
        } else if self.on_hex() {
            Page::Hex
        } else if self.view == View::Home && self.selected.is_none() && !self.on_live() {
            Page::Home
        } else {
            Page::Meetings
        }
    }

    pub fn rebuild_nav(&mut self) {
        let mut cache = NavCache {
            show_all: self.nav.show_all,
            profile_menu: self.nav.profile_menu,
            new_menu: self.nav.new_menu,
            ..NavCache::default()
        };
        for m in &self.meetings {
            if m.device.is_some() {
                cache.peers += 1;
            }
            let project = self
                .areas
                .meeting_projects
                .get(&m.id)
                .and_then(|v| v.name.clone());
            if let Some(name) = project {
                *cache.project_counts.entry(name.clone()).or_default() += 1;
                if let Some(started) = m.started {
                    let newest = cache.project_latest.entry(name.clone()).or_insert(started);
                    *newest = (*newest).max(started);
                }
                cache.meeting_project.insert(m.id.clone(), name);
            }
        }
        self.nav = cache;
        // A filter on a project that has gone away would show an empty list for nothing.
        if let Filter::Project(name) = &self.filter
            && !self.tasks.names.contains(name)
        {
            self.filter = Filter::All;
        }
        if self.filter == Filter::Peers && self.nav.peers == 0 {
            self.filter = Filter::All;
        }
    }

    pub fn matches_filter(&self, meeting: &core::Meeting) -> bool {
        match &self.filter {
            Filter::All => true,
            Filter::Short => false,
            Filter::Project(name) => self.nav.meeting_project.get(&meeting.id) == Some(name),
            Filter::Area(id) => self
                .areas
                .activity
                .meetings
                .get(&meeting.id)
                .is_some_and(|t| t.area_id == *id),
            Filter::Areas => true,
            Filter::Peers => meeting.device.is_some(),
        }
    }

    pub fn set_filter(&mut self, filter: Filter) {
        if matches!(filter, Filter::Project(_) | Filter::Area(_) | Filter::Areas)
            && !self.config.features.projects
            || filter == Filter::Peers && !self.config.features.other_devices
        {
            return;
        }
        self.reset_navigation();
        if matches!(filter, Filter::Area(_)) {
            self.areas.show_all = false;
        }
        self.filter = filter;
        self.cursor = 0;
        self.refresh_list();
        self.nav_done();
    }

    /// Meetings, with no filter.
    pub fn go_meetings(&mut self) {
        self.meetings_view = super::list::MeetingsView::All;
        self.set_filter(Filter::All);
    }

    /// Open: the full sidebar beside the page, or in a narrow window the drawer over it.
    pub fn sidebar_is_open(&self) -> bool {
        if self.on_hex() {
            return hex::nav_open(self);
        }
        if self.narrow.get() {
            self.drawer_open.get()
        } else {
            self.sidebar_open.get()
        }
    }

    /// Opens or folds the sidebar. A choice made in a wide window is remembered for next time;
    /// the drawer of a narrow window is not a preference.
    pub fn set_sidebar(&mut self, open: bool) {
        if self.narrow.get() {
            self.drawer_open.set(open);
            return;
        }
        self.sidebar_open.set(open);
        if rune::tooling::headless_run().is_none() {
            let mut prefs = UiPrefs::load(&self.store.root);
            prefs.sidebar_collapsed = Some(!open);
            // A preference is not worth an error: failing to save only loses the memory of it.
            let _ = prefs.save(&self.store.root);
        }
    }

    pub fn toggle_sidebar(&mut self) {
        if self.on_hex() {
            return hex::toggle_nav(self);
        }
        let open = !self.sidebar_is_open();
        self.set_sidebar(open);
    }

    /// Open beside the page, where it carries the toggle in its own top row. A drawer, a rail
    /// and a closed sidebar leave the toggle to the page header. On Hex Settings it is the
    /// settings list.
    pub fn sidebar_docked(&self) -> bool {
        if self.on_hex() {
            hex::nav_docked(self)
        } else {
            !self.narrow.get() && self.sidebar_open.get()
        }
    }

    /// After going somewhere: a narrow window's drawer closes over the page it opened.
    pub fn nav_done(&mut self) {
        self.drawer_open.set(false);
    }
}

/// The sidebar's width including its divider, for pages that size themselves from the window.
pub fn width(s: &State) -> f32 {
    if s.narrow.get() || !s.sidebar_open.get() {
        if s.hex_recording() { RAIL + 1.0 } else { 0.0 }
    } else {
        WIDE + 1.0
    }
}

// ---- the sidebar ------------------------------------------------------------------------------

const MAIN_H: f32 = 30.0;
const PAD: f32 = 8.0;
const RAIL_ITEM_W: f32 = 36.0;
const SIGNAL_D: f32 = 6.0;
const SIGNAL_INSET: f32 = 10.0;

const IDENTITY_NAME: roles::Role = roles::Role {
    size: 12.5,
    weight: 600.0,
    lh: 18.0,
    tracking: 0.0,
    features: "",
};

/// Signals for memory proposals to review and high findings in the Diary.
fn signal(c: Color) -> El {
    dot(SIGNAL_D, c)
}

/// The fill and the hover fill. A selected row keeps its fill under the pointer.
fn fills(active: bool, p: Palette) -> (Color, Color) {
    if active {
        (p.sel, p.sel)
    } else {
        (p.side, p.chip)
    }
}

/// A place's name: 400 in `ink` when active, 400 in `soft` otherwise.
fn place_name(name: &str, active: bool, p: Palette) -> El {
    if active {
        roles::NAV.line(name, p.ink)
    } else {
        roles::NAV.line(name, p.soft)
    }
}

/// One of the places in the open sidebar: an icon and a name.
fn main_row(place: Place, icon: El, p: Palette) -> El {
    let mut r = row()
        .w_full()
        .h(MAIN_H)
        .gap(9.0)
        .items_center()
        .px(PAD)
        .child(icon)
        .child(place_name(place.name, place.active, p).grow(1.0));
    if let Some(c) = place.flag {
        r = r.child(signal(c).mr(SIGNAL_INSET - PAD));
    }
    let (bg, hover) = fills(place.active, p);
    pressable(r)
        .key(place.key)
        .label(place.help)
        .radius(8.0)
        .fill(bg)
        .hover(hover)
        .on_press(place.press)
}

/// A place or a tool in the rail: the icon alone, with the place's dot on its corner.
fn rail_item(place: Place, icon: El, p: Palette) -> El {
    let mut b = col().size(RAIL_ITEM_W, MAIN_H).center().child(icon);
    if let Some(c) = place.flag {
        b = b.child(
            signal(c)
                .absolute(RAIL_ITEM_W - SIGNAL_INSET - SIGNAL_D, 5.0)
                .border(1.5, p.side),
        );
    }
    let (bg, hover) = fills(place.active, p);
    pressable(b)
        .key(place.key)
        .label(place.help.clone())
        .help(place.help)
        .radius(8.0)
        .fill(bg)
        .hover(hover)
        .on_press(place.press)
}

/// Opens or folds the sidebar. A toggle that had focus unmounts as it flips, so focus moves to
/// the one shown next.
pub(super) fn press_toggle(s: &mut State, tx: &mut rune::Tx) {
    let on_toggle = matches!(tx.focused(), Some("sidebar-toggle" | "nav-collapse"));
    s.toggle_sidebar();
    if on_toggle {
        let key = if s.sidebar_docked() {
            "nav-collapse"
        } else {
            "sidebar-toggle"
        };
        tx.focus_when_shown(key, tx.focus_visible());
    }
}

pub(super) fn toggle(s: &State, p: Palette) -> El {
    let open = s.sidebar_is_open();
    let name = if open {
        "Hide sidebar (Ctrl+\\)"
    } else {
        "Show sidebar (Ctrl+\\)"
    };
    super::widgets::icon_button(
        if open {
            &icons::PANEL_CLOSE
        } else {
            &icons::PANEL_OPEN
        },
        name,
        "sidebar-toggle",
        super::widgets::ButtonRole::Ghost,
        p,
    )
    .size(28.0, 28.0)
    .px(6.0)
    .radius(7.0)
    .on_press_tx(press_toggle)
}

/// A place or a tool: what it is called, where it goes and the one signal it may carry.
struct Place {
    key: &'static str,
    name: &'static str,
    help: String,
    active: bool,
    icon: &'static rune::Icon,
    flag: Option<Color>,
    press: fn(&mut State),
}

impl Place {
    fn icon(&self, p: Palette) -> El {
        let ink = if self.active { p.ink } else { p.soft };
        icons::draw(self.icon, icons::ROW, ink)
    }
}

/// The main places and the filter for meetings from other devices, when there are any.
fn places(s: &State, p: Palette) -> (Vec<Place>, Option<Place>) {
    let page = s.page();
    let features = &s.config.features;
    let open_tasks = s.tasks.counts.open;
    let proposals = s.memory.proposals.len();
    let high = s.check.latest.high;
    let mut places = vec![Place {
        key: "nav-home",
        name: "Home",
        help: "Home".into(),
        active: page == Page::Home,
        icon: icons::HOME,
        flag: None,
        press: |s| {
            s.go_home();
            s.nav_done();
        },
    }];
    places.push(Place {
        key: "nav-meetings",
        name: "Meetings",
        help: "Meetings".into(),
        active: page == Page::Meetings && s.filter == Filter::All,
        icon: &icons::MEETINGS,
        flag: None,
        press: State::go_meetings,
    });
    if features.tasks {
        places.push(Place {
            key: "view-tasks",
            name: "Tasks",
            help: format!(
                "Tasks, {open_tasks} open, {} mine",
                s.tasks.counts.mine_open
            ),
            active: page == Page::Tasks,
            icon: &icons::TASKS,
            flag: None,
            press: |s| {
                s.set_view(View::Tasks);
                s.nav_done();
            },
        });
    }
    places.extend(features.people.then(|| Place {
        key: "nav-people",
        name: "People",
        help: "People".into(),
        active: page == Page::People,
        icon: &icons::PEOPLE,
        flag: None,
        press: State::open_people,
    }));
    if features.memory {
        places.push(Place {
            key: "view-memory",
            name: "Memory",
            help: if proposals > 0 {
                format!("Memory, {proposals} proposals to review")
            } else {
                "Memory".into()
            },
            active: page == Page::Memory,
            icon: &icons::MEMORY,
            flag: (proposals > 0).then_some(p.warn),
            press: |s| {
                s.open_memory();
                s.nav_done();
            },
        });
    }
    let secondary_start = places.len();
    if features.daily_check && features.memory {
        places.push(Place {
            key: "view-check",
            name: "Diary",
            help: if high > 0 {
                format!("Diary, {high} open high-severity findings")
            } else {
                "Diary".into()
            },
            active: page == Page::Check,
            icon: &icons::DIARY,
            flag: (high > 0).then_some(p.warn),
            press: |s| {
                s.open_check();
                s.nav_done();
            },
        });
    }
    let annotations = features.annotations.then(|| Place {
        key: "nav-annotations",
        name: "Annotations",
        help: "Annotations".into(),
        active: page == Page::Annotations,
        icon: &icons::ANNOTATIONS,
        flag: None,
        press: |s| s.open_annotations(),
    });
    if let Some(annotations) = annotations {
        places.insert(secondary_start, annotations);
    }
    let peers = (features.other_devices && s.nav.peers > 0).then(|| Place {
        key: "nav-peers",
        name: "Other devices",
        help: format!(
            "Show the {} meetings recorded on another device",
            s.nav.peers
        ),
        active: page == Page::Meetings && s.filter == Filter::Peers,
        icon: &icons::DEVICE,
        flag: None,
        press: |s| s.set_filter(Filter::Peers),
    });
    (places, peers)
}

pub(super) fn search(p: Palette) -> El {
    super::widgets::icon_button(
        &icons::SEARCH,
        "Search (/ or Ctrl+K)",
        "nav-search",
        super::widgets::ButtonRole::Ghost,
        p,
    )
    .size(28.0, 28.0)
    .px(6.0)
    .on_press(|s: &mut State| s.toggle_finder())
}

/// The sidebar. `wide` is the open sidebar; otherwise the rail of icons.
pub fn sidebar(s: &State, p: Palette, page: Palette, wide: bool, layer: &mut Vec<El>) -> El {
    if !wide && !s.hex_recording() {
        return el().w(0.0).h_full();
    }
    let (places, peers) = places(s, p);

    let mut top = col().w_full().gap(1.0).items_center();
    let mut secondary = false;
    for place in places {
        let icon = place.icon(p);
        let starts_group = matches!(place.key, "nav-annotations" | "view-check") && !secondary;
        secondary |= starts_group;
        top = top.child(if wide {
            main_row(place, icon, p).mt(if starts_group { 12.0 } else { 0.0 })
        } else {
            rail_item(place, icon, p)
        });
    }

    let middle = col().grow(1.0);

    let devices = peers.filter(|_| wide).map(|place| {
        let icon = place.icon(p);
        let heading = roles::META.line(place.name, p.faint);
        col()
            .w_full()
            .mt(12.0)
            .child(row().h(28.0).px(PAD).items_center().child(heading))
            .child(main_row(place, icon, p))
    });
    let mut footer = col()
        .w_full()
        .pb(8.0)
        .child(profile(s, p, page, wide, layer));
    if !wide && s.hex_recording() {
        footer = footer.items_center().child(
            pressable(col().size(RAIL_ITEM_W, MAIN_H).center().child(icons::draw(
                &icons::MIC,
                icons::ROW,
                p.danger,
            )))
            .key("rail-recording")
            .label("Recording. Open live meeting")
            .help("Recording")
            .on_press(|s: &mut State| s.hex_line_press()),
        );
    }

    let mut pane = col()
        .key("nav")
        .w(if wide { WIDE } else { RAIL })
        .h_full()
        .min_h(0.0)
        .fill(p.side)
        .px(PAD);
    if wide {
        let controls = !s.narrow.get();
        pane = pane
            .child(workspace_row(p, controls).w(WIDE).mx(-PAD))
            .child(new_menu(s, p, page, layer));
    } else {
        pane = pane.pt(10.0);
    }
    pane = pane
        .child(top)
        .children(devices)
        .child(middle)
        .child(footer);
    if !wide {
        pane = pane.items_center();
    }
    pane
}

pub(crate) fn workspace_row(p: Palette, controls: bool) -> El {
    let identity = row()
        .grow(1.0)
        .min_w(0.0)
        .h(32.0)
        .gap(8.0)
        .px(6.0)
        .radius(9.0)
        .items_center()
        .child(logo().key("nav-logo").a11y_hidden())
        .child(IDENTITY_NAME.line("Sidenote", p.ink).key("nav-name"))
        .child(
            icons::draw(&icons::CHEVRON_DOWN, 12.0, p.faint)
                .key("nav-caret")
                .a11y_hidden(),
        );
    let mut out = row()
        .key("nav-workspace")
        .w_full()
        .h(48.0)
        .px(10.0)
        .gap(2.0)
        .items_center()
        .child(identity);
    out = if controls { out.window_drag() } else { out };
    if controls {
        out = out.child(collapse(p)).child(search(p));
    }
    out
}

pub(crate) fn compact_mark() -> El {
    logo().key("nav-logo").label("Sidenote").help("Sidenote")
}

/// The sidebar's collapse button. It lives in the workspace row and folds the sidebar.
pub(super) fn collapse(p: Palette) -> El {
    super::widgets::icon_button(
        &icons::PANEL_CLOSE,
        "Hide sidebar (Ctrl+\\)",
        "nav-collapse",
        super::widgets::ButtonRole::Ghost,
        p,
    )
    .size(28.0, 28.0)
    .px(6.0)
    .radius(8.0)
    .on_press_tx(press_toggle)
}

fn new_menu(s: &State, p: Palette, page: Palette, layer: &mut Vec<El>) -> El {
    let chip = row()
        .w_full()
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .items_center()
        .radius(9.0)
        .fill(p.chip)
        .child(icons::draw(&icons::PLUS, 16.0, p.ink))
        .child(roles::NAV.bold().line("New", p.ink).grow(1.0))
        .child(icons::draw(&icons::CHEVRON_DOWN, 13.0, p.soft).a11y_hidden());
    let trigger = menu_trigger(
        pressable(chip).key("nav-new-menu").label("New"),
        s.nav.new_menu,
        p,
        |s| s.nav.new_menu = !s.nav.new_menu,
    );
    if s.nav.new_menu {
        let mut recovery = Vec::new();
        let menu = col().child(record_meeting_row(s, page, &mut recovery));
        let anchor = (
            "nav-new-menu",
            "nav-new-menu",
            rune::Side::Bottom,
            rune::Align::Start,
            220.0,
        );
        menu_surface(&super::theme::kit(&page), page, layer, menu, anchor, |s| {
            s.nav.new_menu = false;
            s.updates.listener_open = false;
        });
        layer.extend(recovery);
    }
    col().w_full().pt(2.0).pb(8.0).child(trigger)
}

/// The sidebar scene with every project shown.
pub fn scene_all(s: &mut State) {
    s.nav.show_all = true;
}

// ---- the top right ----------------------------------------------------------------------------

fn shown_live(s: &State) -> Option<&super::live::Live> {
    let live = s.live.as_ref()?;
    let selected = s.page() == Page::Meetings
        && !s.meeting_ui.tabs
        && live
            .session
            .as_ref()
            .is_some_and(|session| s.selected.as_ref() == Some(&session.id))
        && s.current().is_some();
    (s.on_live() || selected).then_some(live)
}

fn meeting_shows_pause(s: &State) -> bool {
    let Some(live) = shown_live(s) else {
        return false;
    };
    let recording = live
        .session
        .as_ref()
        .filter(|session| session.phase == core::meeting_parts::Phase::Recording);
    (s.on_live() && live.phase == super::live::Phase::Recording) || recording.is_some()
}

fn recording_status(s: &State) -> super::live::RecordingStatus {
    let phase = shown_live(s)
        .filter(|live| {
            live.session
                .as_ref()
                .is_none_or(|session| session.phase != core::meeting_parts::Phase::Ended)
        })
        .map(|live| live.phase);
    super::live::RecordingStatus::for_phase(s, phase)
}

pub(super) fn actions(s: &State, p: Palette, narrow: bool) -> (Option<El>, Option<El>) {
    if s.annotations.open {
        return (None, Some(super::annotations::new_button(s, p)));
    }
    if recording_status(s) == super::live::RecordingStatus::Paused {
        return (Some(recording_pill(s, p, narrow)), None);
    }
    let recording = s.hex_recording();
    let pill = (recording && (s.on_live() || !meeting_shows_pause(s)))
        .then(|| recording_pill(s, p, narrow));
    (pill, None)
}

pub(super) fn record_meeting_row(s: &State, p: Palette, layer: &mut Vec<El>) -> El {
    use super::live::Phase;
    const NAME: &str = "Record a meeting";
    let phase = s
        .live
        .as_ref()
        .filter(|live| {
            live.session
                .as_ref()
                .is_none_or(|session| session.phase != core::meeting_parts::Phase::Ended)
        })
        .map(|live| live.phase);
    let blocked = if cfg!(windows) && phase == Some(Phase::Paused) && !s.hex_stop_pending() {
        Some("Finish the paused meeting first")
    } else if let Some(error) = super::updates::listener_problem(s) {
        let row = menu_row(NAME, p, None, Some(&icons::MIC), None).key("new-meeting");
        return super::updates::listener_popover(s, row, error, p, layer);
    } else {
        hex::start_blocked(s).or(match phase {
            Some(Phase::Starting | Phase::Recording) => Some("Finish the current meeting first"),
            _ => None,
        })
    };
    let detail = match blocked {
        Some(hex::WAIT_FOR_STOP) => Some("Finishing recording\u{2026}"),
        _ => blocked.filter(|_| !cfg!(target_os = "linux")),
    };
    let row = menu_row(NAME, p, detail, Some(&icons::MIC), None).key("new-meeting");
    match blocked {
        None => row
            .help("Start recording a meeting")
            .on_press(|s: &mut State| {
                s.nav.new_menu = false;
                s.new_meeting()
            }),
        Some(reason) => row
            .label(format!("Record a meeting ({reason})"))
            .help(reason)
            .opacity(if matches!(reason, hex::CHECKING | hex::WAIT_FOR_STOP) {
                1.0
            } else {
                0.5
            })
            .disabled(true),
    }
}

pub(super) fn recording_words(s: &State, narrow: bool) -> String {
    let status = recording_status(s);
    if status == super::live::RecordingStatus::Paused {
        return status.label().into();
    }
    if status == super::live::RecordingStatus::Unknown {
        return if narrow {
            "Status unknown"
        } else {
            "Recording status unknown"
        }
        .into();
    }
    let clock = s
        .live
        .as_ref()
        .filter(|live| live.phase == super::live::Phase::Recording)
        .and_then(|live| live.session.as_ref())
        .map(|m| {
            let seconds = m.elapsed_ms() / 1000;
            format!("{}:{:02}", seconds / 60, seconds % 60)
        })
        .or_else(|| hex::recording_clock(s));
    match (&clock, narrow) {
        (Some(c), true) => c.clone(),
        (Some(c), false) => format!("Recording {c}"),
        (None, _) => "Recording".into(),
    }
}

pub(super) fn live_recording_dot(size: f32, p: Palette) -> El {
    dot(size, p.danger).looping(rune_motion::LoopSpec::opacity(1.8, 0.0, 0.55, 1.0))
}

fn owns_live_recording(s: &State, id: &str) -> bool {
    s.live
        .as_ref()
        .and_then(|live| live.session.as_ref())
        .filter(|session| session.phase == core::meeting_parts::Phase::Recording)
        .and_then(|session| session.parts.last())
        .is_some_and(|part| part.id == id)
}

fn recording_pill(s: &State, p: Palette, narrow: bool) -> El {
    let words = recording_words(s, narrow);
    use super::live::RecordingStatus;
    let status = recording_status(s);
    // The pill never reaches the caption buttons: its title takes what the header leaves.
    let (lead, trail, _) = s.chrome.get();
    let reserve = if !s.sidebar_docked() && !s.sidebar_is_open() {
        150.0
    } else {
        130.0
    };
    let room = (s.app_w.get() - 13.0_f32.max(lead) - 8.0_f32.max(trail) - reserve).max(0.0);
    let title = s
        .hex
        .status
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .and_then(|state| state.recording.as_ref())
        .and_then(|r| r.title.as_deref());
    let display = match (status, title.filter(|t| !t.trim().is_empty())) {
        (RecordingStatus::Recording, _) if s.on_live() => words.clone(),
        (RecordingStatus::Recording, Some(title)) if narrow => title.to_owned(),
        (RecordingStatus::Recording, Some(title)) => format!("{title} \u{b7} {words}"),
        (RecordingStatus::Unknown, _) if narrow => "Unknown".into(),
        _ => words.clone(),
    };
    let dot = match status {
        RecordingStatus::Paused => None,
        RecordingStatus::Recording => Some(live_recording_dot(8.0, p).key("recording-live-dot")),
        RecordingStatus::Unknown => Some(dot(8.0, p.warn).key("recording-live-dot")),
    };
    let open = super::widgets::button("", "recording-open", super::widgets::ButtonRole::Ghost, p)
        .h(26.0)
        .px(8.0)
        .gap(0.0)
        .child(
            row().gap(6.0).items_center().children(dot).child(
                roles::CONTROL
                    .line(display, p.soft)
                    .truncate()
                    .max_w((if narrow { 70.0_f32 } else { 230.0 }).min(room)),
            ),
        )
        .label(format!("{words}. Open the live meeting"))
        .help("Open the live meeting")
        .on_press(|s: &mut State| s.open_live());
    if s.on_live()
        || s.live
            .as_ref()
            .and_then(|l| l.session.as_ref())
            .is_some_and(|m| s.selected.as_ref() == Some(&m.id))
    {
        return open.fill(p.card).radius(13.0);
    }
    let expected_id = hex::recording_id(s);
    let pause = expected_id
        .as_deref()
        .is_some_and(|id| owns_live_recording(s, id));
    let (glyph, name, key, label) = if pause {
        (
            &icons::CIRCLE_PAUSE,
            "Pause",
            "hex-pause",
            "Pause recording",
        )
    } else {
        (&icons::STOP, "Stop", "hex-stop", "Stop recording")
    };
    let action =
        super::widgets::icon_button(glyph, label, key, super::widgets::ButtonRole::Ghost, p)
            .size(24.0, 24.0)
            .px(4.0);
    let action = match hex::stop_blocked(s) {
        Some(reason) => action
            .label(format!("{name} ({reason})"))
            .help(reason)
            .disabled(true)
            .dim(0.5),
        None if pause => action.on_press(move |s: &mut State| {
            if expected_id
                .as_deref()
                .is_some_and(|id| owns_live_recording(s, id))
                && hex::recording_id(s) == expected_id
            {
                s.meeting_pause();
            }
        }),
        None => action.on_press(move |s: &mut State| s.hex_record_stop(expected_id.clone())),
    };
    row()
        .key("recording-pill")
        .radius(14.0)
        .fill(p.card)
        .border(1.0, p.line)
        .items_center()
        .child(open)
        .child(action)
}

pub(super) fn logo() -> El {
    const SIZE: f32 = 18.0;
    // The crisp 18 px mark: a rounded square, a white curve and an amber tip.
    static PARTS: [(rune::Icon, &str); 3] = [
        (
            rune::Icon {
                name: "sidenote-logo-square",
                view: 18.0,
                body: r#"<rect width="18" height="18" rx="5" fill="currentColor"/>"#,
            },
            "#1e1e1c",
        ),
        (
            rune::Icon {
                name: "sidenote-logo-curve",
                view: 18.0,
                body: r#"<path d="M3 10 C3 7.4 4 4 6 4 C8 4 8 13 9.6 13 C11 13 11.4 10 13 10 H14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#,
            },
            "#fcfbf8",
        ),
        (
            rune::Icon {
                name: "sidenote-logo-tip",
                view: 18.0,
                body: r#"<path d="M14 10 H16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>"#,
            },
            "#e3aa55",
        ),
    ];
    el().size(SIZE, SIZE)
        .children(PARTS.iter().map(|(part, color)| {
            rune::icon(part, SIZE)
                .color(rune::color::hex(color).expect("logo colour"))
                .absolute(0.0, 0.0)
        }))
}

fn profile_avatar(name: &str, p: Palette, size: f32) -> El {
    let initial = name
        .chars()
        .next()
        .unwrap_or('Y')
        .to_uppercase()
        .to_string();
    col()
        .size(size, size)
        .radius(size / 2.0)
        .fill(p.chip)
        .center()
        .child(roles::NAME.line(initial, p.ink))
}

fn profile(s: &State, p: Palette, page: Palette, wide: bool, layer: &mut Vec<El>) -> El {
    let name = s
        .ai_notes
        .settings
        .as_ref()
        .map(|settings| settings.your_name.trim())
        .filter(|name| !name.is_empty())
        .unwrap_or("You");
    let (_, status) = hex::short_status(s, p);
    let helper_issue = s
        .ai_notes
        .settings
        .as_ref()
        .is_some_and(|settings| settings.engine == "app" && settings.helper.is_some())
        && (s.ai_notes.error.is_some()
            || s.ai_notes.test_results.values().any(|result| {
                !result.starts_with("Worked in ") && !result.starts_with("Claude Code replied")
            }));
    let status = if helper_issue && matches!(status.as_str(), "Ready" | "Checking") {
        "Meeting summary needs attention".into()
    } else {
        status
    };
    let update_ready = super::updates::footer(s, p, wide).is_some();
    let attention = update_ready
        || !matches!(
            status.as_str(),
            "Ready" | "Recording" | "Checking" | "Dictating"
        ) && !s.hex_recording();
    let avatar = |p: Palette, size, dot_size| {
        let mut avatar = profile_avatar(name, p, size);
        if attention {
            avatar = avatar.child(dot(dot_size, p.warn).absolute(size - dot_size, 0.0));
        }
        avatar
    };
    let mut trigger = row()
        .w_full()
        .h(40.0)
        .px(PAD)
        .gap(10.0)
        .items_center()
        .child(avatar(p, 24.0, SIGNAL_D));
    if wide {
        trigger = trigger
            .child(roles::NAV.line(name, p.ink).grow(1.0))
            .child(icons::draw(
                &icons::CHEVRONS_UP_DOWN,
                icons::INLINE,
                p.faint,
            ));
    }
    let trigger = menu_trigger(
        pressable(trigger)
            .key("profile-button")
            .label("Profile and settings")
            .radius(8.0)
            .hover(p.chip),
        s.nav.profile_menu,
        p,
        |s| s.nav.profile_menu = !s.nav.profile_menu,
    );
    let sync = s.last_updated.map(|at| {
        let minutes = at.elapsed().as_secs() / 60;
        if minutes == 0 {
            "Synced just now".to_string()
        } else {
            format!("Synced {minutes} min ago")
        }
    });
    let device = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This PC".into());
    if s.nav.profile_menu {
        let p = page;
        let device_line = format!(
            "This PC: {device}{}",
            sync.as_ref()
                .map(|status| format!(". {status}"))
                .unwrap_or_default()
        );
        let mut menu = col()
            .child(
                row()
                    .h(40.0)
                    .px(8.0)
                    .gap(10.0)
                    .items_center()
                    .child(avatar(p, 24.0, SIGNAL_D))
                    .child(
                        col()
                            .grow(1.0)
                            .shrink()
                            .child(roles::NAV.line(name, p.ink).key("profile-menu-name"))
                            .child(
                                roles::META
                                    .line(device_line, p.faint)
                                    .key("profile-menu-device"),
                            ),
                    ),
            )
            .child(menu_separator(p));
        if update_ready {
            menu = menu.child(profile_item(
                "Update ready",
                "profile-update",
                &super::icons::UPDATES,
                p,
                |s| {
                    s.nav.profile_menu = false;
                    super::updates::settings_scene(s);
                },
            ));
        }
        if attention && !update_ready {
            menu = menu.child(profile_item(
                &format!("{status}. Fix"),
                "profile-fix",
                &super::icons::WARNING,
                p,
                move |s| {
                    s.nav.profile_menu = false;
                    if helper_issue {
                        s.open_hex();
                        s.hex.section = hex::Section::Connect;
                    } else {
                        s.open_hex();
                    }
                },
            ));
        }
        let menu = menu
            .child(profile_item(
                "Prompt library",
                "profile-menu-item2",
                &super::icons::PROMPTS,
                p,
                |s| {
                    s.nav.profile_menu = false;
                    s.open_hex();
                    s.hex.section = hex::Section::PromptLibrary;
                    super::scene_scroll(s, "agent-instructions");
                },
            ))
            .child(profile_item(
                "Help",
                "profile-menu-item3",
                &super::icons::HELP,
                p,
                |s| {
                    s.nav.profile_menu = false;
                    s.open_hex();
                    s.hex.section = super::hex::Section::Shortcuts;
                },
            ))
            .child(menu_separator(p))
            .child(
                menu_row("Settings", p, None, Some(&icons::SETTINGS), Some("Ctrl+,"))
                    .key("profile-menu-item4")
                    .help("Settings")
                    .on_press(|s: &mut State| {
                        s.nav.profile_menu = false;
                        s.open_hex();
                    }),
            );
        menu_surface(
            &super::theme::kit(&p),
            p,
            layer,
            menu,
            (
                "profile-menu",
                "profile-button",
                rune::Side::Top,
                rune::Align::Start,
                WIDE - 2.0 * PAD,
            ),
            |s| s.nav.profile_menu = false,
        );
    }
    trigger
}

fn profile_item(
    name: &str,
    key: &str,
    icon: &'static rune::Icon,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    menu_row(name, p, None, Some(icon), None)
        .key(key)
        .help(name)
        .on_press(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime, text::TextSystem};

    fn new_meeting_runtime(width: f32, listener_preparing: bool) -> rune::Runtime<State> {
        use crate::ui::theme;
        use rune::{Env, Runtime, text::TextSystem};
        crate::test_paths::isolate();
        let mut state = State::new();
        state.go_home();
        state.config.features.annotations = false;
        crate::ui::updates::settings_fixture(&mut state);
        state.listener_probed(
            Some(sidenote::updates::ListenerStatus {
                ready: !listener_preparing,
                preparing: listener_preparing,
                error: None,
            }),
            None,
        );
        let mut rt = Runtime::with_text(
            state,
            crate::ui::view,
            (width, 900.0),
            Env::default(),
            TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
        );
        rt.settle_all();
        rt.frame(0.0);
        rt
    }

    fn open_new_menu(width: f32, preparing: bool) -> rune::Runtime<State> {
        let mut rt = new_meeting_runtime(width, preparing);
        rt.state.drawer_open.set(true);
        rt.state.nav.new_menu = true;
        rt.settle_all();
        rt.frame(0.0);
        assert!(rt.find("new-meeting-status").is_none());
        rt
    }

    #[test]
    fn blocked_record_meeting_row_shows_reason_in_sidebar_menu() {
        let reason = if cfg!(target_os = "linux") {
            "Recording is not available on Linux yet."
        } else {
            "Checking the meeting listener"
        };
        let label = format!("Record a meeting ({reason})");
        for width in [390.0, 1334.0] {
            let elements = open_new_menu(width, true).elements();
            let find = |key| {
                elements
                    .iter()
                    .find(|e| e.key.as_deref() == Some(key))
                    .unwrap_or_else(|| panic!("missing {key} at width {width}"))
            };
            // No bar spans the window: the sidebar's top row (or the drawer's header above it)
            // is 48 px, and the menu opens below it.
            assert!(elements.iter().all(|e| e.key.as_deref() != Some("top-bar")));
            let header = find("page-header");
            let button = find("new-meeting");
            assert!(!button.path.starts_with(&header.path));
            assert!(button.rect[1] >= crate::ui::TITLEBAR_H);
            if width < 600.0 {
                // A drawer opens below the live header, which keeps the toggle.
                let top = |key| find(key).rect[1];
                assert!(top("nav-scrim") >= crate::ui::TITLEBAR_H);
                assert!(top("nav") >= crate::ui::TITLEBAR_H && top("sidebar-toggle") < 48.0);
            }
            assert_eq!(button.label.as_deref(), Some(label.as_str()));
            assert!(!button.focusable);
            let status = elements.iter().find(|e| e.text.as_deref() == Some(reason));
            if cfg!(target_os = "linux") {
                assert!(status.is_none());
            } else {
                let status = status.expect("visible start-blocked reason");
                assert!(status.path.starts_with(&button.path));
                assert!(status.rect[1] >= crate::ui::TITLEBAR_H);
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn listener_check_status_keeps_navigation_available() {
        let mut rt = new_meeting_runtime(1334.0, true);
        rt.state.sidebar_open.set(true);
        rt.set_full_keyboard_access(true);
        settle(&mut rt);
        for key in ["nav-home", "nav-meetings", "nav-search", "nav-new-menu"] {
            assert!(el(&rt, key).focusable, "{key}");
        }
        assert!(rt.find("home-greeting").is_some());
        assert_eq!(
            el(&rt, "listener-startup-status").text.as_deref(),
            Some("Checking the meeting listener…")
        );
        let palette = super::super::theme::palette(false);
        assert_eq!(
            hex::short_status(&rt.state, palette),
            (palette.warn, "Recording off".into())
        );

        tap(&mut rt, "nav-new-menu");
        let record = el(&rt, "new-meeting");
        assert_eq!(
            record.label.as_deref(),
            Some("Record a meeting (Checking the meeting listener)")
        );
        assert!(!record.focusable);
        tap(&mut rt, "new-meeting");
        assert!(!rt.state.hex_record_busy());
        assert!(!rt.state.on_live());
        assert!(rt.find("listener-startup-status").is_some());

        rt.update(|s, _| s.nav.new_menu = false);
        settle(&mut rt);
        tap(&mut rt, "nav-search");
        assert!(rt.state.finder.is_some());
        rt.update(|s, _| {
            s.apply_event(super::super::Event::ListenerStartupStatus(
                sidenote::updates::ListenerStatus {
                    ready: true,
                    ..Default::default()
                },
            ));
        });
        settle(&mut rt);
        assert!(rt.state.finder.is_some(), "startup kept the open finder");
        assert!(rt.find("home-greeting").is_some());
        assert!(rt.find("listener-startup-status").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn blocked_record_meeting_row_shows_reason_in_sidebar_menu_narrow_listener() {
        let mut rt = open_new_menu(360.0, true);
        let elements = rt.elements();
        let button = elements
            .iter()
            .find(|e| e.key.as_deref() == Some("new-meeting"))
            .unwrap();
        let checking = elements
            .iter()
            .find(|e| e.text.as_deref() == Some("Checking the meeting listener"))
            .expect("short checking message");
        let label = elements
            .iter()
            .find(|e| e.text.as_deref() == Some("Record a meeting"))
            .unwrap();
        assert!(checking.rect[1] >= label.rect[1] + label.rect[3]);
        assert_eq!(checking.rect[3], roles::META.lh);
        assert!(checking.path.starts_with(&button.path));
        assert!(button.rect[0] >= 0.0 && button.rect[0] + button.rect[2] <= 360.0);
        rt.update(|s, _| s.listener_probed(Some(Default::default()), None));
        rt.settle_all();
        rt.frame(0.0);
        assert!(rt.elements().iter().any(|e| {
            e.text.as_deref() == Some("Recording is unavailable. Restart Sidenote to retry.")
                && e.rect[3] > roles::META.lh
        }));
    }

    #[cfg(windows)]
    #[test]
    fn blocked_record_meeting_row_shows_reason_in_sidebar_menu_narrow_hex() {
        let mut rt = open_new_menu(360.0, false);
        let elements = rt.elements();
        let button = elements
            .iter()
            .find(|e| e.key.as_deref() == Some("new-meeting"))
            .unwrap();
        let mut labels = elements
            .iter()
            .filter(|e| e.text.as_deref() == Some("Checking for Hex"));
        let [x, y, w, h] = labels.next().expect("short checking message").rect;
        let [bx, by, bw, bh] = button.rect;
        assert!(
            x >= bx - 0.5 && y >= by - 0.5 && x + w <= bx + bw + 0.5 && y + h <= by + bh + 0.5,
            "progress shows inside the button"
        );
        assert!(
            labels.next().is_none(),
            "no separate status line below the button"
        );
        assert_eq!(
            button.label.as_deref(),
            Some("Record a meeting (Checking for Hex)")
        );
        assert_eq!(button.help.as_deref(), Some("Checking for Hex"));
        assert!(!button.focusable, "the button is disabled while checking");
        assert!(button.rect[0] >= 0.0 && button.rect[0] + button.rect[2] <= 360.0);
        rt.update(|s, _| hex::stop::scene(s, false));
        rt.frame(0.0);
        assert!(rt.elements().iter().any(|e| {
            e.text.as_deref() == Some("Finishing recording\u{2026}")
                && e.path.starts_with(&button.path)
        }));
    }

    #[test]
    fn new_menu_trigger_opens_the_record_row() {
        let mut rt = new_meeting_runtime(1334.0, false);
        rt.update(|s, _| s.config.features.annotations = true);
        tap(&mut rt, "nav-new-menu");
        assert!(rt.find("new-meeting").is_some());
        assert!(rt.find("nav-new-annotation").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn new_menu_listener_retry_receives_pointer_and_escape_dismisses_it() {
        let mut rt = new_meeting_runtime(1334.0, false);
        rt.update(|s, _| s.listener_finished(Err("Synthetic listener error".into())));
        tap(&mut rt, "nav-new-menu");
        tap(&mut rt, "new-meeting");
        let (x, y) = rt.find("listener-retry").expect("Try again");
        rt.pointer_move(x, y);
        assert_eq!(rt.cursor(), rune::Cursor::Pointer);
        rt.key(rune::KeyEvent {
            key: rune::Key::Escape,
            mods: rune::Mods::default(),
        });
        rt.frame(0.2);
        assert!(rt.find("new-meeting").is_none());
        tap(&mut rt, "nav-new-menu");
        assert!(rt.find("new-meeting").is_some());
        assert!(rt.find("listener-error-popover-content").is_none());
    }

    fn tap(rt: &mut rune::Runtime<State>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.settle_all();
        rt.frame(0.0);
    }

    fn el(rt: &rune::Runtime<State>, key: &str) -> rune::ElementInfo {
        let mut list = rt.elements().into_iter();
        list.find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("missing {key}"))
    }

    fn count(rt: &rune::Runtime<State>, key: &str) -> usize {
        let list = rt.elements();
        list.iter()
            .filter(|e| e.key.as_deref() == Some(key))
            .count()
    }

    fn settle(rt: &mut rune::Runtime<State>) {
        rt.settle_all();
        rt.frame(0.0);
    }

    /// The window's own buttons take `trailing` px at the right edge and `height` px at the top.
    fn with_insets(rt: &mut rune::Runtime<State>, trailing: f32, height: f32) {
        let mut insets = rune::ChromeInsets::default();
        insets.trailing = trailing;
        insets.height = height;
        rt.set_chrome_insets(insets);
        settle(rt);
    }

    #[test]
    fn shell_divider_uses_the_page_line_outside_settings() {
        let mut rt = new_meeting_runtime(1334.0, false);
        rt.set_system_dark(false);
        settle(&mut rt);
        let p = crate::ui::theme::palette(false);
        let divider = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("sidebar-divider"))
            .expect("sidebar divider");
        assert_eq!(divider.fill, p.line);
    }

    #[test]
    fn sidebar_top_row_geometry() {
        // The open sidebar is 204 wide, nav rows are 30 high, and the first top row is 48 high
        // with the collapse and search buttons inside the sidebar, which starts at the top.
        let rt = new_meeting_runtime(1334.0, false);
        let (nav, header) = (el(&rt, "nav"), el(&rt, "page-header"));
        assert_eq!((nav.rect[1], nav.rect[2]), (0.0, 204.0));
        assert_eq!(width(&rt.state), 204.0 + 1.0);
        assert_eq!(el(&rt, "nav-meetings").rect[3], 30.0);
        assert_eq!(el(&rt, "nav-workspace").rect[3], 48.0);
        for key in ["nav-search", "nav-collapse"] {
            let path = el(&rt, key).path;
            assert!(path.starts_with(&nav.path) && !path.starts_with(&header.path));
        }
        let counts = ["sidebar-toggle", "top-bar", "nav-search"].map(|k| count(&rt, k));
        assert_eq!(counts, [0, 0, 1]);
    }

    /// A page, with the key of an element that sits in or under its header.
    fn chrome_page(name: &str) -> (rune::Runtime<State>, &'static str) {
        let mut rt = new_meeting_runtime(1334.0, false);
        let title = match name {
            "home" => "home-greeting",
            "tasks" => "tasks-header",
            "hex" => "settings-title",
            _ => "meetings-title-row",
        };
        rt.update(|s, _| match name {
            "home" => s.go_home(),
            "tasks" => s.set_view(View::Tasks),
            "hex" => s.open_hex(),
            _ => s.go_meetings(),
        });
        settle(&mut rt);
        (rt, title)
    }

    #[test]
    fn an_idle_page_has_no_blank_band_above_its_title() {
        // Sidebar open and nothing to show: the header row holds no control, so it takes only
        // the height of the window's caption buttons (none without them), never 48 px.
        for caption in [0.0, crate::ui::TITLEBAR_H] {
            for name in ["home", "tasks", "meetings"] {
                let (mut rt, title) = chrome_page(name);
                with_insets(&mut rt, 0.0, caption);
                let header = el(&rt, "page-header");
                let prefix = format!("{}/", header.path);
                let list = rt.elements();
                let controls = list.iter().filter(|e| e.path.starts_with(&prefix));
                assert_eq!(controls.filter(|e| e.focusable).count(), 0, "{name}");
                assert_eq!((header.rect[1], header.rect[3]), (0.0, caption), "{name}");
                assert_eq!(count(&rt, "top-bar"), 0, "{name}");
                assert!(el(&rt, title).rect[1] >= caption, "{name}");
            }
        }
    }

    #[test]
    fn header_row_keeps_window_drag_and_caption_inset() {
        const CAPTION: f32 = 138.0;
        for name in ["home", "tasks", "meetings", "hex"] {
            let (mut rt, _) = chrome_page(name);
            with_insets(&mut rt, CAPTION, crate::ui::TITLEBAR_H);
            let header = el(&rt, "page-header");
            let prefix = format!("{}/", header.path);
            for e in rt.elements().iter().filter(|e| e.path.starts_with(&prefix)) {
                let right = e.rect[0] + e.rect[2];
                assert!(right <= 1334.0 - CAPTION + 0.5, "{name}: {:?}", e.key);
            }
            // A press in the header's empty middle moves the window.
            let (x, y) = (header.rect[0] + header.rect[2] / 2.0, 24.0);
            rt.pointer_move(x, y);
            rt.pointer_down(x, y);
            let action = rt.take_titlebar_action();
            assert_eq!(action, Some(rune::TitlebarAction::Drag), "{name}");
            rt.pointer_up(x, y);
        }
    }

    #[test]
    fn docked_sidebar_headers_drag_from_empty_space() {
        for (name, sidebar) in [("home", "nav"), ("hex", "settings-sidebar")] {
            let (mut rt, _) = chrome_page(name);
            let [x, y, w, _] = el(&rt, sidebar).rect;
            let (x, y) = (x + w / 2.0, y + 24.0);
            rt.pointer_down(x, y);
            assert_eq!(
                rt.take_titlebar_action(),
                Some(rune::TitlebarAction::Drag),
                "{name}"
            );
            rt.pointer_up(x, y);
            tap(&mut rt, "nav-search");
            assert_eq!(rt.take_titlebar_action(), None, "{name}");
            assert!(rt.state.finder.is_some(), "{name}");
        }
    }

    #[test]
    fn collapsing_docked_sidebar_preserves_content_path() {
        let (mut rt, _) = chrome_page("hex");
        let path = el(&rt, "hex-scroll").path;
        tap(&mut rt, "nav-collapse");
        assert_eq!(el(&rt, "hex-scroll").path, path);
        tap(&mut rt, "sidebar-toggle");
        assert_eq!(el(&rt, "hex-scroll").path, path);
    }

    /// A toggle saves the sidebar preference, which every other test's `State::new` reads from
    /// the shared folder. A test that toggles keeps its own folder.
    fn own_prefs(rt: &mut rune::Runtime<State>) {
        let name = format!("sidenote-nav-{:?}", std::thread::current().id());
        rt.state.store.root = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&rt.state.store.root).unwrap();
    }

    fn ctrl_backslash(rt: &mut rune::Runtime<State>) {
        rt.update(|s, tx| {
            let mods = rune::Mods {
                ctrl: true,
                ..Default::default()
            };
            let key = rune::Key::Command('\\');
            assert!(crate::ui::on_key(s, &rune::KeyEvent { key, mods }, tx));
        });
        settle(rt);
    }

    #[test]
    fn the_toggles_and_ctrl_backslash_open_and_close_the_sidebar() {
        // Open: the toggle is in the sidebar. Closed: one toggle and one search at the top left.
        let mut rt = new_meeting_runtime(1334.0, false);
        own_prefs(&mut rt);
        // A hidden sidebar leaves no divider strip at the window's edge.
        let keys = [
            "nav-collapse",
            "sidebar-toggle",
            "top-bar",
            "nav-search",
            "sidebar-divider",
        ];
        let shown = |rt: &rune::Runtime<State>| keys.map(|k| count(rt, k));
        assert_eq!(shown(&rt), [1, 0, 0, 1, 1]);
        tap(&mut rt, "nav-collapse");
        assert_eq!(shown(&rt), [0, 1, 0, 1, 0]);
        assert!(el(&rt, "sidebar-toggle").rect[0] < 40.0);
        tap(&mut rt, "sidebar-toggle");
        assert_eq!(shown(&rt), [1, 0, 0, 1, 1]);
        for open in [false, true] {
            ctrl_backslash(&mut rt);
            assert_eq!(rt.state.sidebar_is_open(), open);
        }
    }

    #[test]
    fn hex_settings_toggle_and_ctrl_backslash_control_the_settings_list() {
        let (mut rt, _) = chrome_page("hex");
        let width = |rt: &rune::Runtime<State>| el(rt, "settings-sidebar").rect[2];
        assert!(width(&rt) > 0.0);
        tap(&mut rt, "nav-collapse");
        assert_eq!((width(&rt), count(&rt, "sidebar-divider")), (0.0, 0));
        assert!(el(&rt, "sidebar-toggle").rect[0] < 40.0);
        for shown in [true, false] {
            ctrl_backslash(&mut rt);
            assert_eq!(width(&rt) > 0.0, shown);
            assert_eq!(count(&rt, "sidebar-divider"), usize::from(shown));
        }
        tap(&mut rt, "sidebar-toggle");
        assert!(width(&rt) > 0.0);
    }

    #[test]
    fn focus_follows_the_toggle_that_unmounts() {
        let mut rt = new_meeting_runtime(1334.0, false);
        own_prefs(&mut rt);
        rt.update(|_, tx| tx.focus("nav-collapse", true));
        rt.frame(0.0);
        rt.update(press_toggle);
        settle(&mut rt);
        rt.update(|_, tx| assert_eq!(tx.focused(), Some("sidebar-toggle")));
    }

    #[test]
    fn the_sidebar_top_is_the_same_on_every_page() {
        let mut rt = new_meeting_runtime(1334.0, false);
        rt.update(|s, _| {
            s.config.features.people = true;
            s.config.features.memory = true;
        });
        type Open = fn(&mut State);
        let pages: [(&str, Open); 6] = [
            ("home", |s| s.go_home()),
            ("tasks", |s| s.set_view(View::Tasks)),
            ("people", |s| s.open_people()),
            ("companies", |s| s.companies.open = true),
            ("memory", |s| s.open_memory()),
            ("hex", |s| s.open_hex()),
        ];
        for (name, open) in pages {
            rt.update(|s, _| {
                s.companies.open = false;
                open(s);
            });
            settle(&mut rt);
            let list = rt.elements();
            let side = ["nav", "settings-sidebar", "page-header"];
            let tops = list
                .iter()
                .filter(|e| side.contains(&e.key.as_deref().unwrap_or_default()));
            assert!(tops.map(|e| e.rect[1]).all(|y| y == 0.0), "{name}");
            assert_eq!(count(&rt, "top-bar"), 0, "{name}");
        }
    }

    #[test]
    fn hex_page_logo_uses_the_new_mark() {
        // S1a replaced the layered 20-unit mark with the crisp 18 px mark: a #1e1e1c square,
        // a white curve and an #e3aa55 tip, and no #bcb9b0 frame.
        let rt = chrome_page("hex").0;
        let logo = el(&rt, "nav-logo");
        assert_eq!(&logo.rect[2..], &[18.0, 18.0]);
        let workspace = el(&rt, "nav-workspace");
        let sidebar = el(&rt, "settings-sidebar");
        let header = el(&rt, "page-header");
        assert!(logo.path.starts_with(&format!("{}/", workspace.path)));
        assert!(workspace.path.starts_with(&format!("{}/", sidebar.path)));
        assert!(!logo.path.starts_with(&format!("{}/", header.path)));
        assert_eq!(workspace.rect[3], 48.0);
        let elements = rt.elements();
        let prefix = format!("{}/", logo.path);
        let tints: Vec<_> = elements
            .iter()
            .filter(|e| e.path.starts_with(&prefix))
            .filter_map(|e| e.text_color)
            .collect();
        assert_eq!(tints.len(), 3, "square, curve and tip");
        for want in ["#1e1e1c", "#fcfbf8", "#e3aa55"] {
            let color = rune::color::hex(want).expect("logo colour");
            assert!(tints.contains(&color), "missing {want}");
        }
        assert!(
            !tints.contains(&rune::color::hex("#bcb9b0").expect("old frame")),
            "the old frame is gone"
        );
    }

    const INVENTED_MEETING_ID: &str = "1800000000000-9";
    fn invented_meeting() -> sidenote::core::Meeting {
        let title = "Invented identity standup";
        sidenote::core::Meeting {
            folder: std::path::PathBuf::from(format!("synthetic/{INVENTED_MEETING_ID}")),
            device: None,
            id: INVENTED_MEETING_ID.into(),
            title: title.into(),
            diary_title: String::new(),
            summary: sidenote::core::Summary {
                title: title.into(),
                ..Default::default()
            },
            summary_raw: String::new(),
            draft_raw: String::new(),
            transcript_raw: String::new(),
            routed: None,
            routed_revision: String::new(),
            summary_language: None,
            notes: String::new(),
            lines: Vec::new(),
            started: None,
            minutes: None,
            export_path: None,
            search: title.to_lowercase(),
        }
    }
    fn identity_rt(width: f32, height: f32, dark: bool) -> rune::Runtime<State> {
        crate::test_paths::isolate();
        let mut state = State::new();
        crate::ui::updates::settings_fixture(&mut state);
        let probed = sidenote::updates::ListenerStatus {
            ready: true,
            ..Default::default()
        };
        state.listener_probed(Some(probed), None);
        let mut rt = Runtime::with_text(
            state,
            crate::ui::view,
            (width, height),
            Env::default(),
            TextSystem::new(crate::ui::theme::fonts(), (crate::ui::theme::SANS, "Inter")),
        );
        rt.set_system_dark(dark);
        settle(&mut rt);
        rt
    }
    fn key_backslash(rt: &mut rune::Runtime<State>) {
        rt.key(rune::KeyEvent {
            key: rune::Key::Command('\\'),
            mods: rune::Mods {
                ctrl: true,
                ..Default::default()
            },
        });
        settle(rt);
    }
    fn assert_full_identity(rt: &rune::Runtime<State>, width: f32) {
        let mut ts = TextSystem::new(crate::ui::theme::fonts(), (crate::ui::theme::SANS, "Inter"));
        let want = ts.measure("Sidenote", IDENTITY_NAME.style(), 1.0).0;
        assert_eq!(count(rt, "nav-workspace"), 1);
        let workspace = el(rt, "nav-workspace");
        assert_eq!(workspace.rect[3], 48.0);
        assert_eq!(count(rt, "nav-logo"), 1);
        let logo = el(rt, "nav-logo");
        assert_eq!(&logo.rect[2..], &[18.0, 18.0]);
        assert!(logo.path.starts_with(&format!("{}/", workspace.path)));
        assert_eq!(count(rt, "nav-caret"), 1);
        let caret = el(rt, "nav-caret");
        assert_eq!(&caret.rect[2..], &[12.0, 12.0]);
        assert!(caret.path.starts_with(&format!("{}/", workspace.path)));
        assert_eq!(count(rt, "nav-name"), 1);
        let name = el(rt, "nav-name");
        assert_eq!(name.text.as_deref(), Some("Sidenote"));
        assert!(name.path.starts_with(&format!("{}/", workspace.path)));
        assert!(name.rect[2] + 0.5 >= want);
        assert!((caret.rect[0] - (name.rect[0] + name.rect[2]) - 8.0).abs() <= 0.5);
        let right = name.rect[0] + name.rect[2];
        assert!(name.rect[0] >= workspace.rect[0] - 0.5);
        assert!(right <= workspace.rect[0] + workspace.rect[2] + 0.5);
        let header = el(rt, "page-header");
        assert!(!logo.path.starts_with(&format!("{}/", header.path)));
        let [ax, ay, aw, ah] = workspace.rect;
        let [bx, by, bw, bh] = header.rect;
        assert!(
            ax + aw <= bx + 0.5
                || bx + bw <= ax + 0.5
                || ay + ah <= by + 0.5
                || by + bh <= ay + 0.5
        );
        if width >= 600.0 {
            let collapse = el(rt, "nav-collapse");
            let search = el(rt, "nav-search");
            assert!(right <= caret.rect[0] + 0.5 && right <= collapse.rect[0] + 0.5);
            assert!(right <= search.rect[0] + 0.5);
        }
    }
    fn assert_compact_identity(rt: &rune::Runtime<State>) {
        assert_eq!(count(rt, "nav-workspace"), 0);
        assert_eq!(count(rt, "nav-logo"), 1);
        let logo = el(rt, "nav-logo");
        assert_eq!(&logo.rect[2..], &[18.0, 18.0]);
        let header = el(rt, "page-header");
        assert!(logo.path.starts_with(&format!("{}/", header.path)));
        assert_eq!(logo.label.as_deref(), Some("Sidenote"));
        assert_eq!(count(rt, "nav-caret"), 0);
    }
    fn ensure_open(rt: &mut rune::Runtime<State>) {
        if !rt.state.sidebar_is_open() {
            tap(rt, "sidebar-toggle");
        }
        assert!(rt.state.sidebar_is_open());
    }
    fn visit(rt: &mut rune::Runtime<State>, width: f32, key: &str, page: Page) {
        ensure_open(rt);
        tap(rt, key);
        assert!(rt.state.page() == page);
        ensure_open(rt);
        assert_full_identity(rt, width);
    }
    #[test]
    fn workspace_identity_is_in_the_sidebar_on_every_page() {
        for (width, height) in [(1334.0, 900.0), (390.0, 844.0)] {
            for dark in [false, true] {
                let mut rt = identity_rt(width, height, dark);
                rt.update(|s, _| {
                    s.config.features.tasks = true;
                    s.config.features.people = true;
                    s.config.features.memory = true;
                    s.config.features.daily_check = true;
                    s.config.features.annotations = true;
                    s.meetings.push(invented_meeting());
                    s.refresh_list();
                    s.rebuild_nav();
                });
                visit(&mut rt, width, "nav-home", Page::Home);
                visit(&mut rt, width, "nav-meetings", Page::Meetings);
                assert!(rt.state.selected.is_none());
                if width < 600.0 && rt.state.sidebar_is_open() {
                    tap(&mut rt, "sidebar-toggle");
                }
                tap(&mut rt, &format!("meeting-{INVENTED_MEETING_ID}"));
                assert_eq!(rt.state.selected.as_deref(), Some(INVENTED_MEETING_ID));
                assert!(rt.state.page() == Page::Meetings);
                ensure_open(&mut rt);
                assert_full_identity(&rt, width);
                visit(&mut rt, width, "view-tasks", Page::Tasks);
                visit(&mut rt, width, "nav-people", Page::People);
                visit(&mut rt, width, "view-memory", Page::Memory);
                visit(&mut rt, width, "nav-annotations", Page::Annotations);
                visit(&mut rt, width, "view-check", Page::Check);
                ensure_open(&mut rt);
                tap(&mut rt, "profile-button");
                tap(&mut rt, "profile-menu-item4");
                assert!(rt.state.page() == Page::Hex);
                assert_full_identity(&rt, width);
                assert_eq!(count(&rt, "nav-collapse"), usize::from(width >= 600.0));
                assert_eq!(count(&rt, "sidebar-toggle"), usize::from(width < 600.0));
                assert_eq!(count(&rt, "nav-search"), 1);
            }
        }
    }
    #[test]
    fn workspace_identity_compacts_and_restores_through_toggle() {
        for (width, height) in [(1334.0, 900.0), (390.0, 844.0)] {
            for dark in [false, true] {
                let mut rt = identity_rt(width, height, dark);
                rt.on_key = Some(crate::ui::on_key);
                own_prefs(&mut rt);
                ensure_open(&mut rt);
                assert_full_identity(&rt, width);
                key_backslash(&mut rt);
                assert!(!rt.state.sidebar_is_open());
                assert_compact_identity(&rt);
                rt.update(|_, tx| assert_eq!(tx.focused(), Some("sidebar-toggle")));
                assert!(el(&rt, "sidebar-toggle").focus_visible);
                key_backslash(&mut rt);
                assert!(rt.state.sidebar_is_open());
                assert_full_identity(&rt, width);
                if width >= 600.0 {
                    rt.update(|_, tx| assert_eq!(tx.focused(), Some("nav-collapse")));
                    assert!(el(&rt, "nav-collapse").focus_visible);
                } else {
                    rt.update(|_, tx| assert_eq!(tx.focused(), Some("sidebar-toggle")));
                    assert!(el(&rt, "sidebar-toggle").focus_visible);
                }
                rt.update(|s, _| {
                    s.hex.status = Some(Ok(sidenote::hex_engine::EngineState {
                        running: true,
                        version: "test".into(),
                        protocol: 1,
                        recording: Some(sidenote::hex_engine::Recording {
                            id: None,
                            title: None,
                            since: None,
                            elapsed_seconds: Some(60),
                        }),
                        meeting_state: "recording".into(),
                        dictating: Some(false),
                        capture_state: "idle".into(),
                        model: sidenote::hex_engine::ModelState {
                            id: None,
                            name: None,
                            language: None,
                            installed: None,
                            preparing: None,
                            downloaded_bytes: None,
                        },
                        labs: Box::default(),
                    }));
                });
                ensure_open(&mut rt);
                assert_full_identity(&rt, width);
                if width >= 600.0 {
                    tap(&mut rt, "nav-collapse");
                } else {
                    tap(&mut rt, "sidebar-toggle");
                }
                assert_compact_identity(&rt);
                rt.update(|_, tx| assert_eq!(tx.focused(), Some("sidebar-toggle")));
                assert_eq!(count(&rt, "rail-recording"), 1);
                assert_eq!(el(&rt, "nav").rect[2], RAIL);
                tap(&mut rt, "sidebar-toggle");
                assert_full_identity(&rt, width);
                if width >= 600.0 {
                    rt.update(|_, tx| assert_eq!(tx.focused(), Some("nav-collapse")));
                } else {
                    rt.update(|_, tx| assert_eq!(tx.focused(), Some("sidebar-toggle")));
                }
                assert_eq!(count(&rt, "rail-recording"), 0);
            }
        }
    }
    #[test]
    fn workspace_hover_does_not_take_editor_focus() {
        fn type_text(rt: &mut rune::Runtime<State>, text: &str) {
            rt.key(rune::KeyEvent {
                key: rune::Key::Text(text.into()),
                mods: rune::Mods::default(),
            });
            rt.frame(0.0);
        }
        fn hover(rt: &mut rune::Runtime<State>, key: &str) {
            let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
            rt.pointer_move(x, y);
            rt.frame(0.0);
        }
        for (width, height) in [(1334.0, 900.0), (390.0, 844.0)] {
            for dark in [false, true] {
                let mut rt = identity_rt(width, height, dark);
                tap(&mut rt, "ask-input");
                type_text(&mut rt, "invented ask probe 7Q");
                assert_eq!(rt.state.question.text, "invented ask probe 7Q");
                assert!(rt.state.question.focused);
                ensure_open(&mut rt);
                hover(&mut rt, "nav-logo");
                hover(&mut rt, "nav-caret");
                hover(&mut rt, "nav-name");
                assert!(rt.state.question.focused);
                assert_eq!(rt.state.question.text, "invented ask probe 7Q");
                type_text(&mut rt, "-again");
                assert_eq!(rt.state.question.text, "invented ask probe 7Q-again");
                rt.update(|s, _| s.open_hex());
                settle(&mut rt);
                ensure_open(&mut rt);
                tap(&mut rt, "hex-filter");
                type_text(&mut rt, "invented settings probe 7Q");
                assert!(
                    rt.state
                        .hex
                        .filter
                        .text
                        .contains("invented settings probe 7Q")
                );
                assert!(rt.state.hex.filter.focused);
                hover(&mut rt, "nav-logo");
                hover(&mut rt, "nav-caret");
                hover(&mut rt, "nav-name");
                assert!(rt.state.hex.filter.focused);
                assert!(
                    rt.state
                        .hex
                        .filter
                        .text
                        .contains("invented settings probe 7Q")
                );
                type_text(&mut rt, "-again");
                assert!(
                    rt.state
                        .hex
                        .filter
                        .text
                        .contains("invented settings probe 7Q-again")
                );
            }
        }
    }

    #[test]
    fn profile_menu_accepts_enter_arrows_and_escape() {
        use rune::Key::{Down, Enter, Escape, Tab, Up};
        use rune::{Env, Runtime};
        crate::test_paths::isolate();
        let view = |s: &State, _: &mut rune::Cx<State>| {
            let mut layer = Vec::new();
            let page = super::super::theme::palette(true);
            let sidebar = super::super::theme::sidebar_palette(true, page);
            let trigger = profile(s, sidebar, page, true, &mut layer);
            col().child(trigger).children(layer)
        };
        let mut rt = Runtime::new(State::new(), view, (400.0, 600.0), Env::new(true, 1.0));
        rt.frame(0.0);
        let keys = [Tab, Enter, Down, Down, Up, Escape];
        let [trigger, opened, first, next, back, closed] = keys.map(|key| {
            rt.key(key.into());
            rt.frame(0.016);
            rt.elements().into_iter().find(|e| e.focused).unwrap()
        });
        assert_eq!(trigger.key.as_deref(), Some("profile-button"));
        assert_eq!(opened.role, "menuitem");
        assert_ne!(first.key, next.key);
        assert_eq!(first.key, back.key);
        assert_eq!(closed.key, trigger.key);
        assert!(!rt.state.nav.profile_menu);
    }

    #[test]
    fn home_sidebar_has_the_approved_dimensions_and_place_order() {
        crate::test_paths::isolate();
        let mut s = State::new();
        s.config.features.tasks = true;
        s.config.features.people = true;
        s.config.features.annotations = true;
        s.config.features.memory = true;
        s.config.features.daily_check = true;

        s.config.features.other_devices = true;
        s.nav.peers = 1;
        let (places, peers) = places(&s, super::super::theme::palette(true));
        let names: Vec<_> = places.iter().map(|place| place.name).collect();
        let expected = "Home, Meetings, Tasks, People, Memory, Annotations, Diary";
        // S1 sizes: nav rows 30 high and the open sidebar 204 wide (old 34.0 and 230.0).
        assert_eq!(
            (MAIN_H, WIDE, names.join(", ").as_str()),
            (30.0, 204.0, expected)
        );

        let glyphs = [
            "solar-home-smile",
            "solar-document",
            "solar-check-circle",
            "solar-users-group-rounded",
            "solar-pin",
            "solar-pen",
            "solar-calendar-minimalistic",
        ];
        for (place, glyph) in places.iter().zip(glyphs) {
            icons::assert_solar_weights(place.icon, glyph);
        }
        icons::assert_solar_weights(peers.unwrap().icon, "solar-monitor-smartphone");
    }

    #[test]
    fn home_sidebar_hides_disabled_experiments() {
        let mut state = State::new();
        state.config.features.tasks = true;
        state.config.features.people = false;
        state.config.features.memory = false;
        state.config.features.daily_check = true;
        state.config.features.annotations = false;
        let (places, _) = places(&state, super::super::theme::palette(true));
        assert_eq!(
            places.iter().map(|place| place.name).collect::<Vec<_>>(),
            ["Home", "Meetings", "Tasks"]
        );
    }
}
