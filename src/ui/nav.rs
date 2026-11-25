//! The left sidebar and the top-right actions.
//!
//! The sidebar holds places and project filters, with one profile button at its foot.
//! It folds to a recording rail during a call and hides fully otherwise.
//! The top right holds one primary "New meeting" button, which becomes "Recording 12:34 | Stop"
//! while Hex records, except on the meeting page that already shows Pause.
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
pub const WIDE: f32 = 240.0;
pub const RAIL: f32 = 56.0;
/// A window this wide or narrower starts with the sidebar folded.
pub const COLLAPSE_AT: f32 = 820.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
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
        } else {
            Page::Meetings
        }
    }

    pub fn rebuild_nav(&mut self) {
        let mut cache = NavCache {
            show_all: self.nav.show_all,
            profile_menu: self.nav.profile_menu,
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
        self.go_home();
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
        self.set_filter(Filter::All);
    }

    /// Open: the full sidebar beside the page, or in a narrow window the drawer over it.
    pub fn sidebar_is_open(&self) -> bool {
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
        let open = !self.sidebar_is_open();
        self.set_sidebar(open);
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

// ---- icons ------------------------------------------------------------------------------------

fn icon_meetings(c: Color) -> El {
    icons::draw(&icons::MEETINGS, icons::ROW, c)
}
fn icon_tasks(c: Color) -> El {
    icons::draw(&icons::TASKS, icons::ROW, c)
}
fn icon_memory(c: Color) -> El {
    icons::draw(&icons::MEMORY, icons::ROW, c)
}
fn icon_check(c: Color, _: Color) -> El {
    icons::draw(&icons::DIARY, icons::ROW, c)
}
fn icon_device(c: Color) -> El {
    icons::draw(&icons::DEVICE, icons::ROW, c)
}
// ---- the sidebar ------------------------------------------------------------------------------

const MAIN_H: f32 = 32.0;
const PAD: f32 = 8.0;
const RAIL_ITEM_W: f32 = 36.0;
const SIGNAL_D: f32 = 6.0;
const SIGNAL_INSET: f32 = 10.0;

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

/// One of the places in the open sidebar: an icon and a name on a 32-point row.
fn main_row(place: Place, icon: El, p: Palette) -> El {
    let mut r = row()
        .w_full()
        .h(MAIN_H)
        .gap(10.0)
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
        .radius(7.0)
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
        .radius(7.0)
        .fill(bg)
        .hover(hover)
        .on_press(place.press)
}

fn toggle(s: &State, p: Palette) -> El {
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
    .on_press(|s: &mut State| s.toggle_sidebar())
}

/// A place or a tool: what it is called, where it goes and the one signal it may carry.
struct Place {
    key: &'static str,
    name: &'static str,
    help: String,
    active: bool,
    icon: fn(Color, Color) -> El,
    flag: Option<Color>,
    press: fn(&mut State),
}

impl Place {
    fn icon(&self, p: Palette) -> El {
        (self.icon)(if self.active { p.ink } else { p.soft }, p.side)
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
        key: "nav-meetings",
        name: "Meetings",
        help: "Meetings".into(),
        active: page == Page::Meetings && s.filter == Filter::All,
        icon: |c, _| icon_meetings(c),
        flag: None,
        press: |s| s.go_meetings(),
    }];
    if features.tasks {
        places.push(Place {
            key: "view-tasks",
            name: "Tasks",
            help: format!(
                "Tasks, {open_tasks} open, {} mine",
                s.tasks.counts.mine_open
            ),
            active: page == Page::Tasks,
            icon: |c, _| icon_tasks(c),
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
        icon: |c, _| icons::draw(&icons::PEOPLE, icons::ROW, c),
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
            icon: |c, _| icon_memory(c),
            flag: (proposals > 0).then_some(p.warn),
            press: |s| {
                s.open_memory();
                s.nav_done();
            },
        });
    }
    // The Diary is read every day like the other places, so it sits with them, not in the
    // footer with the tools.
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
            icon: icon_check,
            flag: (high > 0).then_some(p.warn),
            press: |s| {
                s.open_check();
                s.nav_done();
            },
        });
    }
    places.extend(features.annotations.then(|| Place {
        key: "nav-annotations",
        name: "Annotations",
        help: "Annotations".into(),
        active: page == Page::Annotations,
        icon: |c, _| icons::draw(&icons::ANNOTATIONS, icons::ROW, c),
        flag: None,
        press: |s| s.open_annotations(),
    }));
    let peers = (features.other_devices && s.nav.peers > 0).then(|| Place {
        key: "nav-peers",
        name: "Other devices",
        help: format!(
            "Show the {} meetings recorded on another device",
            s.nav.peers
        ),
        active: page == Page::Meetings && s.filter == Filter::Peers,
        icon: |c, _| icon_device(c),
        flag: None,
        press: |s| s.set_filter(Filter::Peers),
    });
    (places, peers)
}

/// The sidebar. `wide` is the open sidebar; otherwise the rail of icons.
pub fn sidebar(s: &State, p: Palette, wide: bool, layer: &mut Vec<El>) -> El {
    if !wide && !s.hex_recording() {
        return el().w(0.0).h_full();
    }
    let (places, peers) = places(s, p);

    let header = row().w_full().h(48.0).gap(10.0).items_center();
    let header = if wide {
        header
            .child(logo())
            .pl(PAD)
            .child(roles::COMPACT_SECTION.line("Sidenote", p.ink).grow(1.0))
            .child(toggle(s, p))
    } else {
        col()
            .w_full()
            .h(48.0)
            .gap(2.0)
            .center()
            .child(logo())
            .child(toggle(s, p))
    };

    let search = pressable(
        row()
            .w(if wide { WIDE - 2.0 * PAD } else { RAIL_ITEM_W })
            .h(30.0)
            .gap(10.0)
            .items_center()
            .px(if wide { PAD } else { 10.0 })
            .child(icons::draw(&icons::SEARCH, icons::ROW, p.faint))
            .children(wide.then(|| roles::NAV.line("Search", p.faint).grow(1.0)))
            .children(wide.then(|| {
                roles::META
                    .line("Ctrl K", p.faint)
                    .px(6.0)
                    .radius(5.0)
                    .border(1.0, p.line)
            })),
    )
    .key("nav-search")
    .label("Search (/ or Ctrl+K)")
    .help("Search (/ or Ctrl+K)")
    .radius(8.0)
    .fill(p.chip)
    .hover(p.chip)
    .on_press(|s: &mut State| s.toggle_finder());
    let mut top = col().w_full().gap(1.0).items_center();
    for place in places {
        let icon = place.icon(p);
        top = top.child(if wide {
            main_row(place, icon, p)
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
    let mut footer = col().w_full().pb(8.0).child(profile(s, p, wide, layer));
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
        .px(PAD)
        .child(header)
        .child(search.mb(8.0))
        .child(top)
        .children(devices)
        .child(middle)
        .child(footer);
    if !wide {
        pane = pane.items_center();
    }
    pane
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
    shown_live(s).is_some_and(|live| {
        (s.on_live() && live.phase == super::live::Phase::Recording)
            || live
                .session
                .as_ref()
                .is_some_and(|session| session.phase == core::meeting_parts::Phase::Recording)
    })
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

/// "New meeting", or while Hex records, the recording actions outside the active meeting.
pub fn actions(s: &State, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    if s.annotations.open {
        return super::annotations::new_button(s, p);
    }
    if recording_status(s) == super::live::RecordingStatus::Paused {
        return recording_pill(s, p, narrow);
    }
    if s.hex_recording() {
        if meeting_shows_pause(s) {
            return if s.hex_status_unknown() {
                new_meeting_button(s, p, narrow, layer)
            } else {
                el()
            };
        }
        let recording = recording_pill(s, p, narrow);
        return if s.hex_status_unknown() {
            row()
                .gap(8.0)
                .items_center()
                .child(recording)
                .child(new_meeting_button(s, p, narrow, layer))
        } else {
            recording
        };
    }
    new_meeting_button(s, p, narrow, layer)
}

pub(super) fn new_meeting_button(s: &State, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let new = if narrow { "+ New" } else { "+ New meeting" };
    use super::widgets::ButtonRole;
    let button = |label, role| super::widgets::button(label, "new-meeting", role, p);
    if let Some(error) = super::updates::listener_problem(s) {
        return super::updates::listener_popover(
            s,
            button(new, ButtonRole::Primary),
            error,
            p,
            layer,
        );
    }
    let blocked = hex::start_blocked(s);
    let progress = match blocked {
        Some(hex::CHECKING) => Some(hex::CHECKING),
        Some(hex::WAIT_FOR_STOP) => Some("Finishing recording\u{2026}"),
        _ => None,
    };
    if let Some(label) = progress
        && let Some(reason) = blocked
    {
        return button(label, ButtonRole::Secondary)
            .label(format!("New meeting ({reason})"))
            .help(reason)
            .disabled(true);
    }
    let b = button(new, ButtonRole::Primary);
    let action = match blocked {
        None => b
            .label("New meeting")
            .help("Start recording a meeting")
            .on_press(|s: &mut State| s.new_meeting()),
        Some(reason) => b
            .label(format!("New meeting ({reason})"))
            .help(reason)
            .opacity(0.5)
            .disabled(true),
    };
    if cfg!(target_os = "linux") {
        return action;
    }
    col()
        .items_end()
        .gap(if narrow { 6.0 } else { 2.0 })
        .child(action)
        .children(blocked.map(|reason| {
            roles::META
                .wrap(reason, p.soft)
                .max_w(if narrow { 300.0 } else { 440.0 })
                .role("status")
        }))
}

pub(super) fn recording_words(s: &State, narrow: bool) -> String {
    use super::live::RecordingStatus;
    let status = recording_status(s);
    if status == RecordingStatus::Paused {
        return status.label().into();
    }
    if status == RecordingStatus::Unknown {
        return if narrow {
            "Status unknown"
        } else {
            status.label()
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
        (Some(c), false) => format!("{} {c}", status.label()),
        (None, _) => status.label().into(),
    }
}

fn recording_pill(s: &State, p: Palette, narrow: bool) -> El {
    let words = recording_words(s, narrow);
    let color = match recording_status(s) {
        super::live::RecordingStatus::Paused => None,
        super::live::RecordingStatus::Recording => Some(p.danger),
        super::live::RecordingStatus::Unknown => Some(p.warn),
    };
    let open = super::widgets::button(
        "",
        "recording-open",
        super::widgets::ButtonRole::Secondary,
        p,
    )
    .gap(0.0)
    .child(
        row()
            .gap(8.0)
            .items_center()
            .children(color.map(|color| dot(8.0, color).key("recording-live-dot")))
            .child(roles::CONTROL.wrap(words.clone(), p.soft)),
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
        return open;
    }
    let expected_id = hex::recording_id(s);
    let why = hex::stop_blocked(s);
    let stop = super::widgets::button("Stop", "hex-stop", super::widgets::ButtonRole::Secondary, p);
    let stop = match why {
        None => stop
            .label("Stop recording")
            .help("Stop this recording part. Generate notes from the meeting page.")
            .on_press(move |s: &mut State| s.hex_record_stop(expected_id.clone())),
        Some(reason) => stop
            .label(format!("Stop ({reason})"))
            .help(reason)
            .disabled(true)
            .dim(0.5),
    };
    row()
        .key("recording-pill")
        .gap(8.0)
        .items_center()
        .child(open)
        .child(stop)
}

fn logo() -> El {
    const SIZE: f32 = 18.0;
    let mut mark = el().size(SIZE, SIZE);
    static PART_0: rune::Icon = rune::Icon {
        name: "sidenote-logo-0",
        view: 20.0,
        body: r#"<rect x="0" y="0" width="20" height="20" rx="4.5" fill="currentColor"/>"#,
    };
    mark = mark.child(
        rune::icon(&PART_0, SIZE)
            .color(rune::color::hex("#bcb9b0").expect("logo colour"))
            .absolute(0.0, 0.0),
    );
    static PART_1: rune::Icon = rune::Icon {
        name: "sidenote-logo-1",
        view: 20.0,
        body: r#"<rect x="1" y="1" width="18" height="18" rx="3.5" fill="currentColor"/>"#,
    };
    mark = mark.child(
        rune::icon(&PART_1, SIZE)
            .color(rune::color::hex("#1e1e1c").expect("logo colour"))
            .absolute(0.0, 0.0),
    );
    static PART_2: rune::Icon = rune::Icon {
        name: "sidenote-logo-2",
        view: 20.0,
        body: r#"<path d="M5 6 H7 V7 H5 Z M4 7 H8 V8 H4 Z M3 8 H5 V9 H3 Z M7 8 H9 V9 H7 Z M3 9 H5 V10 H3 Z M7 9 H9 V10 H7 Z M3 10 H5 V11 H3 Z M8 10 H10 V11 H8 Z M3 11 H5 V12 H3 Z M8 11 H10 V12 H8 Z M12 11 H15 V12 H12 Z M3 12 H5 V13 H3 Z M9 12 H15 V13 H9 Z M10 13 H13 V14 H10 Z M11 14 H13 V15 H11 Z" fill="currentColor"/>"#,
    };
    mark = mark.child(
        rune::icon(&PART_2, SIZE)
            .color(rune::color::hex("#fcfbf8").expect("logo colour"))
            .absolute(0.0, 0.0),
    );
    static PART_3: rune::Icon = rune::Icon {
        name: "sidenote-logo-3",
        view: 20.0,
        body: r#"<path d="M15 11 H17 V12 H15 Z M15 12 H17 V13 H15 Z" fill="currentColor"/>"#,
    };
    mark = mark.child(
        rune::icon(&PART_3, SIZE)
            .color(rune::color::hex("#e3aa55").expect("logo colour"))
            .absolute(0.0, 0.0),
    );
    mark
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

fn profile(s: &State, p: Palette, wide: bool, layer: &mut Vec<El>) -> El {
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
    let avatar = |size, dot_size| {
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
        .child(avatar(24.0, SIGNAL_D));
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
            .radius(7.0)
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
                    .child(avatar(24.0, SIGNAL_D))
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

pub fn meetings_home(s: &State) -> bool {
    s.page() == Page::Meetings
        && s.selected.is_none()
        && !s.on_live()
        && !matches!(s.filter, Filter::Area(_) | Filter::Areas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn narrow_new_meeting_shows_hex_check_inside_the_button() {
        use crate::ui::theme;
        use rune::{Env, Runtime, text::TextSystem};
        crate::test_paths::isolate();
        let mut state = State::new();
        crate::ui::updates::settings_fixture(&mut state);
        state.listener_probed(
            Some(sidenote::updates::ListenerStatus {
                ready: true,
                preparing: false,
                error: None,
            }),
            None,
        );
        let mut rt = Runtime::with_text(
            state,
            |s: &State, _: &mut rune::Cx<State>| {
                let mut layer = Vec::new();
                let action = new_meeting_button(s, theme::palette(true), true, &mut layer);
                col().child(action).children(layer)
            },
            (360.0, 200.0),
            Env::default(),
            TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
        );
        rt.frame(0.0);
        let elements = rt.elements();
        let button = elements
            .iter()
            .find(|e| e.key.as_deref() == Some("new-meeting"))
            .unwrap();
        let checking = elements
            .iter()
            .find(|e| e.text.as_deref() == Some("Checking for Hex"))
            .expect("short checking message");
        let inside = |r: [f32; 4]| {
            r[0] >= button.rect[0] - 0.5
                && r[1] >= button.rect[1] - 0.5
                && r[0] + r[2] <= button.rect[0] + button.rect[2] + 0.5
                && r[1] + r[3] <= button.rect[1] + button.rect[3] + 0.5
        };
        assert!(inside(checking.rect), "progress shows inside the button");
        assert_eq!(
            elements
                .iter()
                .filter(|e| e.text.as_deref() == Some("Checking for Hex"))
                .count(),
            1,
            "no separate status line below the button"
        );
        assert_eq!(
            button.label.as_deref(),
            Some("New meeting (Checking for Hex)")
        );
        assert_eq!(button.help.as_deref(), Some("Checking for Hex"));
        assert!(!button.focusable, "the button is disabled while checking");
        assert!(button.rect[0] + button.rect[2] <= 360.0 + 0.5);
    }

    #[test]
    fn profile_menu_accepts_enter_arrows_and_escape() {
        use rune::Key::{Down, Enter, Escape, Tab, Up};
        use rune::{Env, Runtime};
        crate::test_paths::isolate();
        let view = |s: &State, _: &mut rune::Cx<State>| {
            let mut layer = Vec::new();
            let trigger = profile(s, super::super::theme::palette(true), true, &mut layer);
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

        let (places, _) = places(&s, super::super::theme::palette(true));
        let names: Vec<_> = places.iter().map(|place| place.name).collect();
        let expected = "Meetings, Tasks, People, Memory, Diary, Annotations";
        assert_eq!(
            (MAIN_H, WIDE, names.join(", ").as_str()),
            (32.0, 240.0, expected)
        );
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
            ["Meetings", "Tasks"]
        );
    }
}
