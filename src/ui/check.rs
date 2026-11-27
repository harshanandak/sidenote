//! The daily check. Diary aggregates current findings across report dates. The full report
//! of any day opens from there, in the style of an inbox: a header with coverage, a three-line brief, a triage list with a right-hand panel,
//! what changed since the last check, story alignment per project, and a seven-day history.
//! Reports are written by the engine only; this page reads them, records the owner's triage
//! decisions and applies a proposed task action when the owner presses Apply. Reports from
//! before schema 2 still show, without the new parts.
use super::{
    State,
    ask::{cite_time, link_citations},
    diary,
    list::text_title,
    meeting::PAGE_COLUMN,
    roles,
    tasks::{button, cap, chevron, pill, prop_row, value_button},
    theme::Palette,
    widgets::{
        badge, dot, hairline, heading, label, link, list_and_panel, notice_bar, panel_frame,
        panel_top, pressable, quiet_button, scroll, spaced_caps, split_geometry,
    },
};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate};
use rune::{El, Key, KeyEvent, Span, TextEdit, Tx, col, el, row, text};
use rune_kit::{Kit, Markdown, MarkdownStyle, Menu, MenuItem, Placement, TextInput};
use sidenote::core::{
    self,
    check_state::{self, Mark, Status, TriageStatus, TriageStore},
    check_v2::{AlignmentStatus, FindingChange},
    daily_check::{ActionKind, CheckAction, DailyCheck, Finding, FindingKind, Severity},
};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    time::SystemTime,
};

/// Open findings of the newest report, for the top bar and the Home card.
#[derive(Clone, Copy, Default)]
pub struct Counts {
    pub date: Option<NaiveDate>,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    /// Open findings the engine marked new. Zero for a report without change tracking.
    pub new: usize,
}

/// What the owner has done with one finding, as the page needs it.
#[derive(Clone)]
pub struct Handled {
    pub status: TriageStatus,
    pub at: Option<String>,
    pub until: Option<NaiveDate>,
}

/// One day in the seven-day strip. `counts` is None for a day with no report.
#[derive(Clone)]
pub struct StripDay {
    pub date: NaiveDate,
    /// Findings in the report, and how many the engine called new when it can say.
    pub counts: Option<(usize, Option<usize>)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Open,
    Snoozed,
    Dismissed,
    Applied,
}

impl Filter {
    fn status(self) -> TriageStatus {
        match self {
            Self::Open => TriageStatus::Open,
            Self::Snoozed => TriageStatus::Snoozed,
            Self::Dismissed => TriageStatus::Dismissed,
            Self::Applied => TriageStatus::Applied,
        }
    }
}

/// Which list under "Changed since" is unfolded.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    Resolved,
    Unmatched,
}

pub struct CheckState {
    pub dates: Vec<NaiveDate>,
    /// The report on screen. None until a report exists.
    pub date: Option<NaiveDate>,
    pub report: Option<DailyCheck>,
    /// When the on-screen report file was saved.
    pub saved_at: Option<DateTime<Local>>,
    /// Triage state by finding key. Findings missing here are open.
    pub handled: HashMap<String, Handled>,
    pub latest: Counts,
    pub today: NaiveDate,
    pub picker_open: bool,
    pub details_open: bool,
    /// A KB path or command just copied, so its button can say so.
    pub copied: Option<String>,
    /// Why one finding could not be applied or changed, by finding key.
    pub errors: HashMap<String, String>,
    pub notice: Option<String>,
    pub filter: Filter,
    pub change: Option<FindingChange>,
    pub project: Option<String>,
    pub project_menu: bool,
    /// The row the keyboard is on. With `panel` set, the finding the panel shows.
    pub cursor: Option<String>,
    pub keyboard: bool,
    pub panel: bool,
    pub snooze_menu: bool,
    pub snooze_pick: bool,
    pub snooze_date: TextEdit,
    pub snooze_error: Option<String>,
    pub story_open: BTreeSet<String>,
    pub story_text_open: bool,
    pub gone: Option<Gone>,
    /// Titles of the findings in the baseline report, by the id the comparison uses.
    pub baseline_titles: HashMap<String, String>,
    pub strip: Vec<StripDay>,
    /// The full report of `date` is open; otherwise the Diary.
    pub report_open: bool,
    /// How many Diary days are listed.
    pub shown: usize,
    pub diary_selected: Option<NaiveDate>,
    pub diary_cursor: Option<NaiveDate>,
    pub diary_calendar_epoch: bool,
    pub diary_month: Option<NaiveDate>,
    pub diary_week_open: bool,
    pub diary_return: Option<String>,
    pub diary_review_open: bool,
    /// The day each ticked meeting task was ticked, by meeting id and task index.
    pub done_on: HashMap<(String, usize), NaiveDate>,
    /// Findings and new findings of the recent reports, for their days in the Diary.
    pub totals: HashMap<NaiveDate, (usize, Option<usize>)>,
    /// The check's paragraph about each day, by the day it describes, from the recent reports.
    pub day_summaries: HashMap<NaiveDate, String>,
    pub diary_reports: Vec<core::diary::Report>,
    pub diary_read_error: bool,
    /// The Diary sections the owner opened, by `diary::part_key`. All start folded.
    pub open_parts: BTreeSet<String>,
}

pub(super) struct Refresh {
    pub dates: Vec<NaiveDate>,
    pub date: Option<NaiveDate>,
    pub report: Option<DailyCheck>,
    pub saved_at: Option<DateTime<Local>>,
    pub handled: HashMap<String, Handled>,
    pub latest: Counts,
    pub today: NaiveDate,
    pub notice: Option<String>,
    pub baseline_titles: HashMap<String, String>,
    pub strip: Vec<StripDay>,
    pub done_on: HashMap<(String, usize), NaiveDate>,
    pub totals: HashMap<NaiveDate, (usize, Option<usize>)>,
    pub day_summaries: HashMap<NaiveDate, String>,
    pub diary_reports: Vec<core::diary::Report>,
    pub diary_read_error: bool,
}
impl Default for CheckState {
    fn default() -> Self {
        Self {
            dates: Vec::new(),
            date: None,
            report: None,
            saved_at: None,
            handled: HashMap::new(),
            latest: Counts::default(),
            today: Local::now().date_naive(),
            picker_open: false,
            details_open: false,
            copied: None,
            errors: HashMap::new(),
            notice: None,
            filter: Filter::Open,
            change: None,
            project: None,
            project_menu: false,
            cursor: None,
            keyboard: false,
            panel: false,
            snooze_menu: false,
            snooze_pick: false,
            snooze_date: TextEdit::default(),
            snooze_error: None,
            story_open: BTreeSet::new(),
            story_text_open: false,
            gone: None,
            baseline_titles: HashMap::new(),
            strip: Vec::new(),
            report_open: false,
            shown: diary::PAGE,
            diary_selected: None,
            diary_cursor: None,
            diary_calendar_epoch: false,
            diary_month: None,
            diary_week_open: false,
            diary_return: None,
            diary_review_open: false,
            done_on: HashMap::new(),
            totals: HashMap::new(),
            day_summaries: HashMap::new(),
            diary_reports: Vec::new(),
            diary_read_error: false,
            open_parts: BTreeSet::new(),
        }
    }
}

/// The key the page uses for finding `i`: its durable id, or its place in an old report.
fn key_of(report: &DailyCheck, i: usize) -> String {
    let f = &report.content.findings[i];
    if report.schema_version == 2 && !f.id.is_empty() {
        f.id.clone()
    } else {
        format!("v1:{i}")
    }
}

fn is_v2(r: &DailyCheck) -> bool {
    r.schema_version == 2
}

/// Handled state of every finding of `report`, from durable triage or the old marks file.
fn load_handled(report: &DailyCheck, today: NaiveDate) -> Result<HashMap<String, Handled>, ()> {
    let mut out = HashMap::new();
    if is_v2(report) {
        let store = TriageStore::default();
        let file = store.load().map_err(|_| ())?;
        for (i, f) in report.content.findings.iter().enumerate() {
            let status = file.status_for(f, today);
            if status == TriageStatus::Open {
                continue;
            }
            let entry = file.findings.get(&f.id);
            out.insert(
                key_of(report, i),
                Handled {
                    status,
                    at: entry.map(|e| e.updated_at.clone()),
                    until: entry.and_then(|e| e.snooze_until),
                },
            );
        }
    } else {
        let marks = check_state::load(report.date).map_err(|_| ())?;
        for (i, f) in report.content.findings.iter().enumerate() {
            if let Some(m) = marks.get(&check_state::finding_key(i, &f.title)) {
                out.insert(
                    key_of(report, i),
                    Handled {
                        status: match m.status {
                            Status::Applied => TriageStatus::Applied,
                            Status::Dismissed => TriageStatus::Dismissed,
                        },
                        at: Some(m.at.clone()),
                        until: None,
                    },
                );
            }
        }
    }
    Ok(out)
}

fn open_counts(report: &DailyCheck, handled: &HashMap<String, Handled>) -> Counts {
    let mut c = Counts {
        date: Some(report.date),
        ..Counts::default()
    };
    for (i, f) in report.content.findings.iter().enumerate() {
        if handled.contains_key(&key_of(report, i)) {
            continue;
        }
        match f.severity {
            Severity::High => c.high += 1,
            Severity::Medium => c.medium += 1,
            Severity::Low => c.low += 1,
        }
        if is_v2(report) && f.change == FindingChange::New {
            c.new += 1;
        }
    }
    c
}

fn modified(path: std::path::PathBuf) -> Option<DateTime<Local>> {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(|t: SystemTime| t.into())
}

impl CheckState {
    /// Reads the report list, the report on screen (the newest when none is chosen), the
    /// triage state, the newest report's open counts and the seven-day strip.
    pub fn refresh(&mut self) {
        self.notice = None;
        self.today = Local::now().date_naive();
        self.dates = match core::list_checks() {
            Ok(d) => d,
            Err(_) => {
                self.notice = Some("Could not read the daily check reports.".into());
                Vec::new()
            }
        };
        let latest = self.dates.first().copied();
        if self.date.is_none_or(|d| !self.dates.contains(&d)) {
            self.date = latest;
        }
        self.latest = Counts::default();
        self.report = None;
        self.handled = HashMap::new();
        self.baseline_titles = HashMap::new();
        if let Some(date) = self.date {
            match core::load_check(date) {
                Ok(r) => {
                    self.handled = load_handled(&r, self.today).unwrap_or_else(|()| {
                        self.notice = Some("Could not read which findings you handled.".into());
                        HashMap::new()
                    });
                    self.saved_at =
                        modified(core::state_dir().join(format!("local/checks/{date}.json")));
                    self.load_baseline(&r);
                    self.report = Some(r);
                }
                Err(_) => {
                    self.notice = Some(format!(
                        "Could not read the report for {}. Its file may be damaged.",
                        date.format("%-d %b %Y")
                    ));
                }
            }
        }
        if let Some(latest) = latest {
            if Some(latest) == self.date {
                if let Some(r) = &self.report {
                    self.latest = open_counts(r, &self.handled);
                }
            } else if let Ok(r) = core::load_check(latest) {
                let handled = load_handled(&r, self.today).unwrap_or_default();
                self.latest = open_counts(&r, &handled);
            }
        }
        self.build_strip();
        // A finding that is gone from this report cannot stay open in the panel.
        if let (Some(key), Some(r)) = (&self.cursor, &self.report)
            && !(0..r.content.findings.len()).any(|i| &key_of(r, i) == key)
        {
            self.cursor = None;
            self.panel = false;
        }
        if self.report.is_none() {
            self.panel = false;
        }
    }

    pub(super) fn read_refresh(date: Option<NaiveDate>) -> Refresh {
        let mut fresh = Self {
            date,
            ..Self::default()
        };
        fresh.refresh();
        Refresh {
            dates: fresh.dates,
            date: fresh.date,
            report: fresh.report,
            saved_at: fresh.saved_at,
            handled: fresh.handled,
            latest: fresh.latest,
            today: fresh.today,
            notice: fresh.notice,
            baseline_titles: fresh.baseline_titles,
            strip: fresh.strip,
            done_on: fresh.done_on,
            totals: fresh.totals,
            day_summaries: fresh.day_summaries,
            diary_reports: fresh.diary_reports,
            diary_read_error: fresh.diary_read_error,
        }
    }
    pub(super) fn apply_refresh(&mut self, fresh: Refresh) {
        self.dates = fresh.dates;
        self.date = fresh.date;
        self.report = fresh.report;
        self.saved_at = fresh.saved_at;
        self.handled = fresh.handled;
        self.latest = fresh.latest;
        self.today = fresh.today;
        self.notice = fresh.notice;
        self.baseline_titles = fresh.baseline_titles;
        self.strip = fresh.strip;
        self.done_on = fresh.done_on;
        self.totals = fresh.totals;
        self.day_summaries = fresh.day_summaries;
        self.diary_reports = fresh.diary_reports;
        self.diary_read_error = fresh.diary_read_error;
    }
    /// Titles for the ids in the comparison, from the baseline report.
    fn load_baseline(&mut self, r: &DailyCheck) {
        let Some(date) = r.content.comparison.baseline_date else {
            return;
        };
        let Ok(base) = core::load_check(date) else {
            return;
        };
        for (i, f) in base.content.findings.iter().enumerate() {
            self.baseline_titles.insert(f.id.clone(), f.title.clone());
            self.baseline_titles
                .insert(format!("legacy:{date}:{i}"), f.title.clone());
        }
    }

    fn build_strip(&mut self) {
        // Scan every date so unresolved older findings do not disappear at a cutoff.
        self.totals = HashMap::new();
        self.day_summaries = HashMap::new();
        self.diary_reports.clear();
        self.diary_read_error = self.notice.is_some();
        for &d in &self.dates {
            let Ok(r) = core::load_check(d) else {
                self.diary_read_error = true;
                continue;
            };
            let handled = match load_handled(&r, self.today) {
                Ok(h) => h,
                Err(()) => {
                    self.diary_read_error = true;
                    continue;
                }
            };
            let open = (0..r.content.findings.len())
                .filter(|&i| !handled.contains_key(&key_of(&r, i)))
                .collect();
            self.diary_reports.push(core::diary::Report {
                report: r.clone(),
                open,
            });
            let new = is_v2(&r).then_some(r.content.comparison.new.len());
            self.totals.insert(d, (r.content.findings.len(), new));
            if let Some(day) = r.content.day_summary_date
                && !r.content.day_summary.trim().is_empty()
            {
                // Dates are newest first, so a later report's paragraph wins.
                self.day_summaries
                    .entry(day)
                    .or_insert_with(|| r.content.day_summary.trim().to_string());
            }
        }
        let newest = self.dates.first().copied().unwrap_or(self.today);
        let anchor = newest.max(self.today);
        self.strip = (0..7)
            .rev()
            .map(|back| {
                let date = anchor - Duration::days(back);
                let counts = self.totals.get(&date).copied();
                StripDay { date, counts }
            })
            .collect();
    }

    pub fn typing(&self) -> bool {
        self.snooze_date.focused
    }

    fn status_at(&self, report: &DailyCheck, i: usize) -> TriageStatus {
        self.handled
            .get(&key_of(report, i))
            .map_or(TriageStatus::Open, |h| h.status)
    }

    fn count_status(&self, status: TriageStatus) -> usize {
        self.report.as_ref().map_or(0, |r| {
            (0..r.content.findings.len())
                .filter(|&i| self.status_at(r, i) == status)
                .count()
        })
    }
}

// ---- state changes ---------------------------------------------------------------------------

impl State {
    pub fn on_check(&self) -> bool {
        self.check_open && !self.memory_open && self.view.is_base() && self.selected.is_none()
    }

    pub fn open_check(&mut self) {
        if !self.config.features.daily_check || !self.config.features.memory {
            return;
        }
        self.go_home();
        self.check_open = true;
        self.check.picker_open = false;
        self.check.date = None;
        self.check.panel = false;
        self.check.report_open = false;
        self.check.open_parts.clear();
        self.check.diary_return = None;
        self.check.diary_review_open = false;
        self.check.diary_week_open = false;
        self.check.shown = diary::PAGE;
        self.check.filter = Filter::Open;
        self.check.refresh();
        // Open findings of high priority should not hide behind a fold: open "Needs your
        // attention" for the latest check's day. Folding it again lasts until the next visit.
        let urgent = self.check.report.as_ref().and_then(|r| {
            (0..r.content.findings.len())
                .any(|i| {
                    self.check.status_at(r, i) == TriageStatus::Open
                        && matches!(r.content.findings[i].severity, Severity::High)
                })
                .then_some(r.date)
        });
        if let Some(date) = urgent {
            self.check
                .open_parts
                .insert(diary::part_key(date, diary::Part::Attention));
        }
        self.refresh_diary();
        if self.config.features.memory {
            self.refresh_memory(true);
        }
    }

    /// The full report of one day, from its entry in the Diary.
    pub fn open_report(&mut self, date: NaiveDate) {
        self.pick_check(date);
        self.check.report_open = true;
    }

    /// Back from a full report to the Diary, which shows the newest report's findings.
    pub fn close_report(&mut self) {
        self.check.report_open = false;
        self.check.picker_open = false;
        self.check.filter = Filter::Open;
        self.check.project = None;
        self.check.change = None;
        self.check.date = None;
        self.check.panel = false;
        self.check.cursor = None;
        self.check.refresh();
    }

    pub fn pick_check(&mut self, date: NaiveDate) {
        self.check.date = Some(date);
        self.check.picker_open = false;
        self.check.errors.clear();
        self.check.panel = false;
        self.check.cursor = None;
        self.check.refresh();
    }

    /// Opens the meeting a citation names, at its line when it has a time. Back returns here.
    pub fn open_check_citation(&mut self, target: &str, origin: String, tx: &mut Tx) {
        let (id, stamp) = match target.split_once('@') {
            Some((id, stamp)) => (id, Some(stamp)),
            None => (target, None),
        };
        if !self.meetings.iter().any(|m| m.id == id) {
            self.check.notice = Some("That meeting is not in the list any more.".into());
            return;
        }
        let from_memory = self.memory_open;
        self.select(id, tx);
        if let Some(stamp) = stamp {
            self.jump(stamp, origin.clone(), tx);
        }
        self.back = Some(origin);
        self.check_back = !from_memory;
        self.memory_back = from_memory;
    }

    fn finding_by_key(&self, key: &str) -> Option<(usize, &Finding)> {
        let r = self.check.report.as_ref()?;
        (0..r.content.findings.len())
            .find(|&i| key_of(r, i) == key)
            .map(|i| (i, &r.content.findings[i]))
    }

    pub fn open_finding(&mut self, key: &str) {
        self.check.cursor = Some(key.to_string());
        self.check.panel = true;
        self.check.snooze_menu = false;
        self.check.snooze_pick = false;
        self.check.snooze_error = None;
        self.check.snooze_date = TextEdit::default();
    }

    pub fn close_finding(&mut self) {
        self.check.panel = false;
        self.check.snooze_menu = false;
        self.check.snooze_pick = false;
        self.check.snooze_date = TextEdit::default();
    }

    fn after_triage(&mut self) {
        let Some(r) = &self.check.report else {
            return;
        };
        self.check.handled = load_handled(r, self.check.today).unwrap_or_else(|()| {
            self.check.notice = Some("Could not read which findings you handled.".into());
            HashMap::new()
        });
        if self.check.dates.first() == Some(&r.date) {
            self.check.latest = open_counts(r, &self.check.handled);
        }
    }

    /// Records the owner's decision on one finding, in the durable store for new reports and
    /// in the old per-day marks for old ones. `until` is for Snoozed.
    pub fn set_triage(&mut self, key: &str, status: TriageStatus, until: Option<NaiveDate>) {
        self.check.errors.remove(key);
        let Some((index, finding)) = self.finding_by_key(key) else {
            return;
        };
        let Some(report) = &self.check.report else {
            return;
        };
        let result = if is_v2(report) {
            TriageStore::default()
                .set_finding(finding, status, until, None)
                .map(|_| ())
        } else {
            let mark = match status {
                TriageStatus::Dismissed => Some(Status::Dismissed),
                TriageStatus::Applied => Some(Status::Applied),
                _ => None,
            }
            .map(|status| Mark {
                status,
                at: Local::now().to_rfc3339(),
            });
            check_state::set(
                report.date,
                &check_state::finding_key(index, &finding.title),
                mark,
            )
            .map(|_| ())
        };
        match result {
            Ok(()) => self.after_triage(),
            Err(_) => {
                self.check.errors.insert(
                    key.to_string(),
                    "Could not save that. Check that the app data folder is writable.".into(),
                );
            }
        }
    }

    pub fn dismiss_finding(&mut self, key: &str) {
        self.set_triage(key, TriageStatus::Dismissed, None);
    }

    pub fn undo_finding(&mut self, key: &str) {
        self.set_triage(key, TriageStatus::Open, None);
    }

    pub fn snooze_finding(&mut self, key: &str, until: NaiveDate) {
        self.set_triage(key, TriageStatus::Snoozed, Some(until));
        if !self.check.errors.contains_key(key) {
            self.check.snooze_menu = false;
            self.check.snooze_pick = false;
            self.check.snooze_date = TextEdit::default();
            self.check.snooze_error = None;
        }
    }

    /// Enter in the snooze date box.
    pub fn snooze_commit(&mut self) {
        let Some(key) = self.check.cursor.clone() else {
            return;
        };
        let typed = self.check.snooze_date.text.trim().to_string();
        match core::parse_due(&typed, self.check.today) {
            Some(d) if d > self.check.today => self.snooze_finding(&key, d),
            Some(_) => {
                self.check.snooze_error = Some("Pick a day after today.".into());
            }
            None => {
                self.check.snooze_error =
                    Some("Try a date like 12 Nov, 2026-11-12 or friday.".into());
            }
        }
    }

    /// Runs the finding's proposed action through core, then marks it applied.
    pub fn apply_finding(&mut self, key: &str) {
        let Some((_, f)) = self.finding_by_key(key) else {
            return;
        };
        let Some(action) = f.action.clone() else {
            return;
        };
        if self.check.handled.contains_key(key) {
            return;
        }
        self.check.errors.remove(key);
        if let Err(e) = core::apply_action(&action) {
            self.check.errors.insert(
                key.to_string(),
                super::edit_worker::SaveError::from(&e).message().into(),
            );
            return;
        }
        self.set_triage(key, TriageStatus::Applied, None);
        if self.check.errors.contains_key(key) {
            self.check.errors.insert(
                key.to_string(),
                "The task change was made, but the mark was not saved. Reload this page before pressing Apply again.".into(),
            );
        }
        // The Tasks view reads these files; bring it in line with what was just written.
        if let Ok(o) = self.tasks.store.load_overrides() {
            self.tasks.overrides = o;
        }
        self.tasks.checked.remove(&action.meeting_id);
        self.rebuild_tasks();
    }
}

// ---- words -----------------------------------------------------------------------------------

fn kind_name(kind: &FindingKind) -> &'static str {
    match kind {
        FindingKind::ProjectMismatch => "Project mismatch",
        FindingKind::ConflictingDecisions => "Conflicting decisions",
        FindingKind::DecisionNotInKb => "Decision missing from the KB",
        FindingKind::KbContradictsMeeting => "KB disagrees with a meeting",
        FindingKind::TaskWithoutOwner => "Task without an owner",
        FindingKind::OverdueOrStale => "Overdue or stale",
        FindingKind::DuplicateTask => "Duplicate task",
        FindingKind::MissingFollowup => "Missing follow-up",
        FindingKind::StoryGap => "Gap in the story",
        FindingKind::Other => "Other",
    }
}

fn kind_short(kind: &FindingKind) -> &'static str {
    match kind {
        FindingKind::DecisionNotInKb => "Not in the KB",
        FindingKind::KbContradictsMeeting => "KB disagrees",
        FindingKind::TaskWithoutOwner => "No owner",
        FindingKind::ConflictingDecisions => "Conflict",
        other => kind_name(other),
    }
}

fn severity_name(s: &Severity) -> &'static str {
    match s {
        Severity::High => "High priority",
        Severity::Medium => "Medium",
        Severity::Low => "Low",
    }
}

fn tone(sev: &Severity, p: Palette) -> rune::Color {
    match sev {
        Severity::High => p.warn,
        Severity::Medium => p.open,
        Severity::Low => p.faint,
    }
}

fn meeting_title<'a>(meetings: &'a [core::Meeting], id: &str) -> &'a str {
    meetings
        .iter()
        .find(|m| m.id == id)
        .map_or("an unknown meeting", |m| m.title.as_str())
}

/// What Apply will change, in plain words.
fn describe(a: &CheckAction) -> String {
    let task = &a.expected_task;
    let value = a.value.as_str().unwrap_or_default();
    match a.kind {
        ActionKind::SetProject => format!("Move task '{task}' to project {value}"),
        ActionKind::SetOwner => format!("Set the owner of task '{task}' to {value}"),
        ActionKind::SetDue if value.is_empty() => format!("Clear the due date of task '{task}'"),
        ActionKind::SetDue => format!("Set the due date of task '{task}' to {}", long_date(value)),
        ActionKind::MarkDone if a.value.as_bool() == Some(true) => {
            format!("Mark task '{task}' as done")
        }
        ActionKind::MarkDone => format!("Mark task '{task}' as not done"),
        ActionKind::Note => format!("Add a note to the meeting history: {value}"),
    }
}

fn long_date(value: &str) -> String {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_or_else(|_| value.to_string(), |d| d.format("%-d %b %Y").to_string())
}

/// Core has no way to reverse an action, so say how to change it by hand.
fn no_undo(a: &CheckAction) -> &'static str {
    match a.kind {
        ActionKind::Note => "This cannot be undone. The note stays in the meeting's saved history.",
        ActionKind::MarkDone => {
            "This cannot be undone from here. Change the tick in the meeting or on the Tasks page."
        }
        _ => "This cannot be undone from here. Change the task again on the Tasks page.",
    }
}

/// The task's value now, the value after Apply, and a warning when Apply would refuse.
fn before_after(s: &State, a: &CheckAction) -> (Option<String>, String, Option<&'static str>) {
    let task = s
        .tasks
        .items
        .iter()
        .find(|t| t.meeting_id == a.meeting_id && t.index == a.todo_index);
    let value = a.value.as_str().unwrap_or_default();
    let after = match a.kind {
        ActionKind::SetDue if value.is_empty() => "No date".to_string(),
        ActionKind::SetDue => long_date(value),
        ActionKind::MarkDone if a.value.as_bool() == Some(true) => "Done".into(),
        ActionKind::MarkDone => "Open".into(),
        _ => value.to_string(),
    };
    let Some(t) = task else {
        return (
            None,
            after,
            Some("That task is not in your list any more. Apply will refuse."),
        );
    };
    let before = match a.kind {
        ActionKind::SetOwner => Some(match (t.mine, t.owner.as_deref()) {
            (Some(true), _) => "Me".to_string(),
            (_, Some(o)) => o.to_string(),
            _ => "No owner".to_string(),
        }),
        ActionKind::SetDue => Some(t.due_date.map_or_else(
            || "No date".to_string(),
            |d| d.format("%-d %b %Y").to_string(),
        )),
        ActionKind::SetProject => Some(t.project.clone()),
        ActionKind::MarkDone => Some(if t.done { "Done" } else { "Open" }.to_string()),
        ActionKind::Note => None,
    };
    let warn = (t.task != a.expected_task)
        .then_some("The task text changed after this check. Apply will refuse.");
    (before, after, warn)
}

fn duration(seconds: f64) -> String {
    let s = seconds.round() as u64;
    if s < 60 {
        format!("{s} s")
    } else {
        format!("{} min {} s", s / 60, s % 60)
    }
}

fn when(at: &str) -> String {
    DateTime::parse_from_rfc3339(at).map_or_else(
        |_| at.to_string(),
        |t| t.with_timezone(&Local).format("%-d %b, %H:%M").to_string(),
    )
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ---- which findings show ---------------------------------------------------------------------

fn sev_rank(s: &Severity) -> u8 {
    match s {
        Severity::High => 0,
        Severity::Medium => 1,
        Severity::Low => 2,
    }
}

/// The change badge of a finding. None for a report without change tracking.
fn change_of(r: &DailyCheck, f: &Finding) -> Option<FindingChange> {
    is_v2(r).then_some(f.change)
}

/// The due date of the task a finding is about, when it is about one.
fn finding_due(s: &State, f: &Finding) -> Option<NaiveDate> {
    let mut targets: Vec<(String, usize)> = Vec::new();
    if let Some(a) = &f.action {
        targets.push((a.meeting_id.clone(), a.todo_index));
    }
    for r in &f.subject_refs {
        let mut parts = r.split(':');
        if let (Some("task"), Some(id), Some(n)) = (parts.next(), parts.next(), parts.next())
            && let Ok(n) = n.parse::<usize>()
        {
            targets.push((id.to_string(), n));
        }
    }
    targets
        .iter()
        .filter_map(|(id, n)| {
            s.tasks
                .items
                .iter()
                .find(|t| &t.meeting_id == id && t.index == *n && !t.done)
        })
        .filter_map(|t| t.due_date)
        .min()
}

/// Indices of the findings on screen: severity first, then new and changed before still open,
/// then the soonest due date.
fn rows(s: &State) -> Vec<usize> {
    let Some(r) = &s.check.report else {
        return Vec::new();
    };
    let c = &s.check;
    let mut v: Vec<usize> = (0..r.content.findings.len())
        .filter(|&i| {
            let f = &r.content.findings[i];
            // The Diary lists what is open, lows only while their fold is open; the full
            // report has the filters.
            if !c.report_open {
                let lows = c
                    .open_parts
                    .contains(&diary::part_key(r.date, diary::Part::Low));
                return c.status_at(r, i) == TriageStatus::Open
                    && (lows || !matches!(f.severity, Severity::Low));
            }
            c.status_at(r, i) == c.filter.status()
                && c.project
                    .as_ref()
                    .is_none_or(|p| f.project.as_deref() == Some(p.as_str()))
                && c.change.is_none_or(|ch| change_of(r, f) == Some(ch))
        })
        .collect();
    v.sort_by_key(|&i| {
        let f = &r.content.findings[i];
        let change = match change_of(r, f) {
            Some(FindingChange::New) => 0,
            Some(FindingChange::Changed) => 1,
            _ => 2,
        };
        (
            sev_rank(&f.severity),
            change,
            finding_due(s, f).map_or(i64::MAX, |d| i64::from(d.num_days_from_ce())),
            i,
        )
    });
    v
}

fn projects(r: &DailyCheck) -> Vec<String> {
    let set: BTreeSet<String> = r
        .content
        .findings
        .iter()
        .filter_map(|f| f.project.clone())
        .collect();
    set.into_iter().collect()
}

// ---- prose with citations --------------------------------------------------------------------

pub fn prose(s: &State, k: &Kit, p: Palette, md: &str, origin: &str, size: f32) -> El {
    let origin = origin.to_string();
    Markdown::new(link_citations(md, &s.meetings))
        .style(
            MarkdownStyle::from_kit(k)
                .text(p.ink)
                .muted(p.soft)
                .link(p.ink)
                .body_font(if size >= roles::READ.size {
                    roles::READ.style()
                } else {
                    roles::SMALL.style()
                }),
        )
        .on_link_tx(move |s: &mut State, url, tx| {
            if let Some(target) = url.strip_prefix("cite:") {
                s.open_check_citation(target, origin.clone(), tx);
            }
        })
        .build(k)
}

fn bullet(p: Palette) -> El {
    col()
        .w(22.0)
        .items_center()
        .child(el().size(5.0, 5.0).radius(2.5).fill(p.faint).mt(11.0))
}

// ---- header ----------------------------------------------------------------------------------

fn picker(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let c = &s.check;
    let Some(current) = c.date else {
        return el();
    };
    if c.dates.len() < 2 {
        return roles::META.wrap("Only one check so far", p.faint);
    }
    let chevron = if c.picker_open {
        k.glyph.chevron_up
    } else {
        k.glyph.chevron_down
    };
    let trigger = pressable(
        row()
            .gap(4.0)
            .items_center()
            .child(roles::CONTROL.wrap("Earlier days", p.soft))
            .child(rune::icon(chevron, 16.0).color(p.faint)),
    )
    .key("check-picker")
    .role("combobox")
    .label(format!("Choose a day, now {}", current.format("%-d %b %Y")))
    .px(12.0)
    .py(5.0)
    .radius(999.0)
    .border(1.0, p.line)
    .hover(p.chip)
    .on_press(|s: &mut State| s.check.picker_open = !s.check.picker_open);
    let mut menu = Menu::new("check-days")
        .trigger(trigger)
        .open(c.picker_open)
        .placement(Placement::BottomEnd)
        .min_width(220.0)
        .on_close(|s: &mut State| s.check.picker_open = false);
    for d in c.dates.iter().take(60) {
        let d = *d;
        menu = menu.item(
            MenuItem::new(d.format("%a %-d %b %Y").to_string())
                .checked(Some(d) == c.date)
                .on_select(move |s: &mut State| s.pick_check(d)),
        );
    }
    menu.build(k, layer)
}

fn kb_when(at: &str) -> String {
    DateTime::parse_from_rfc3339(at).map_or_else(
        |_| at.to_string(),
        |t| t.with_timezone(&Local).format("%-d %b, %H:%M").to_string(),
    )
}

fn fact(name: &str, value: String, p: Palette) -> El {
    let content = format!("{name}  {value}");
    text(content)
        .font(roles::META.style())
        .color(p.soft)
        .wrap()
        .spans(vec![Span::new(0, name.len()).color(p.faint).weight(500.0)])
}

fn header(s: &State, k: &Kit, r: &DailyCheck, narrow: bool, p: Palette, layer: &mut Vec<El>) -> El {
    let today = s.check.today;
    let day = r.date.format("%A %-d %B %Y").to_string();
    let day = if r.date == today {
        format!("Today, {day}")
    } else {
        day
    };
    let mut head = col().w_full().gap(8.0).child(
        link("\u{2039} Diary", p)
            .key("check-diary")
            .label("Back to the Diary")
            .on_press(|s: &mut State| s.close_report()),
    );
    head = head.child(
        row()
            .w_full()
            .items_end()
            .justify_between()
            .gap(12.0)
            .flex_wrap()
            .child(text_title(
                "Daily check",
                if narrow { 26.0 } else { 30.0 },
                p,
            ))
            .child(picker(s, k, p, layer).pb(4.0)),
    );
    head = head.child(roles::TITLE.wrap(day, p.ink).key("check-date"));
    let cov = &r.content.coverage;
    let (meetings, tasks) = if is_v2(r) {
        (cov.meeting_count, cov.task_count)
    } else {
        (r.meeting_count, r.task_count)
    };
    let revision = if is_v2(r) && !cov.kb_revision.is_empty() {
        cov.kb_revision.clone()
    } else {
        r.kb.revision.clone()
    };
    let captured = if is_v2(r) && !cov.kb_captured_at.is_empty() {
        cov.kb_captured_at.clone()
    } else {
        r.kb.captured_at.clone()
    };
    let line = format!(
        "{}, {}. KB revision {}, captured {}.",
        plural(meetings, "meeting", "meetings"),
        plural(tasks, "task", "tasks"),
        revision.chars().take(7).collect::<String>(),
        kb_when(&captured)
    );
    let details = super::widgets::action(
        if s.check.details_open {
            "Hide details"
        } else {
            "Details"
        },
        p,
    )
    .key("check-details")
    .label("Show how and when this check ran")
    .on_press(|s: &mut State| s.check.details_open = !s.check.details_open);
    head = head.child(
        row()
            .w_full()
            .flex_wrap()
            .items_center()
            .gaps(2.0, 8.0)
            .child(roles::META.wrap(line, p.soft))
            .child(details),
    );
    if s.check.details_open {
        let ran = match s.check.saved_at {
            Some(t) => format!(
                "{}, took {}",
                t.format("%H:%M"),
                duration(r.elapsed_seconds)
            ),
            None => format!("took {}", duration(r.elapsed_seconds)),
        };
        head = head.child(
            col()
                .w_full()
                .px(12.0)
                .py(8.0)
                .radius(10.0)
                .fill(p.side)
                .child(fact("Ran", ran, p))
                .child(fact("Model", r.runner.clone(), p))
                .child(fact("Report", format!("schema {}", r.schema_version), p)),
        );
    }
    if r.kb.stale {
        head = head.child(
            roles::SMALL.medium().wrap("The KB snapshot is stale. The refresh failed, so this check used the last saved copy. Its date is not today's verification.", p.warn)
            .px(12.0)
            .py(8.0)
            .radius(10.0)
            .fill(p.warnbg),
        );
    }
    let mut seen = BTreeSet::new();
    for w in cov.warnings.iter().chain(&r.warnings) {
        if w.starts_with("KB refresh failed") || !seen.insert(w.clone()) {
            continue;
        }
        head = head.child(roles::SMALL.medium().wrap(w, p.warn));
    }
    head
}

// ---- brief -----------------------------------------------------------------------------------

/// The finding each brief line points at. Brief lines carry no ids, so a line that cites a
/// source goes to the one open finding citing it; otherwise the contract's order decides: what
/// is new or changed, the biggest open issue, the next due item.
fn brief_targets(s: &State, r: &DailyCheck) -> [Option<String>; 3] {
    let open: Vec<usize> = (0..r.content.findings.len())
        .filter(|&i| s.check.status_at(r, i) == TriageStatus::Open)
        .collect();
    let key = |i: usize| Some(key_of(r, i));
    let by_rank = |pick: &dyn Fn(usize) -> bool| {
        open.iter()
            .copied()
            .filter(|&i| pick(i))
            .min_by_key(|&i| (sev_rank(&r.content.findings[i].severity), i))
    };
    let fresh = by_rank(&|i| {
        matches!(
            change_of(r, &r.content.findings[i]),
            Some(FindingChange::New | FindingChange::Changed)
        )
    });
    let biggest = by_rank(&|_| true);
    let due = open
        .iter()
        .copied()
        .filter_map(|i| finding_due(s, &r.content.findings[i]).map(|d| (d, i)))
        .min();
    let positional = [fresh, biggest, due.map(|(_, i)| i)];
    // A citation in the line points at its finding more reliably than the order does, when
    // exactly one open finding cites it.
    let cited = |line: &str| -> Option<usize> {
        let mut rest = line;
        while let Some(start) = rest.find("[[") {
            let end = rest[start..].find("]]")? + start + 2;
            let token = &rest[start..end];
            let hits: Vec<usize> = open
                .iter()
                .copied()
                .filter(|&i| r.content.findings[i].evidence.iter().any(|e| e == token))
                .collect();
            if let [only] = hits[..] {
                return Some(only);
            }
            rest = &rest[end..];
        }
        None
    };
    let pick = |n: usize| {
        r.content
            .brief
            .get(n)
            .and_then(|l| cited(l))
            .or(positional[n])
            .and_then(key)
    };
    [pick(0), pick(1), pick(2)]
}

fn brief(s: &State, k: &Kit, r: &DailyCheck, p: Palette) -> El {
    let lines: Vec<String> = if is_v2(r) {
        r.content.brief.clone()
    } else {
        r.content
            .summary
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect()
    };
    let targets = if is_v2(r) {
        brief_targets(s, r)
    } else {
        [None, None, None]
    };
    let mut c = col().key("check-summary").w_full().gap(2.0).mt(18.0);
    for (n, line) in lines.iter().enumerate() {
        let mut line_row = row().w_full().gap(8.0).py(2.0).child(bullet(p)).child(
            prose(s, k, p, line.trim(), "check-summary", 15.5)
                .grow(1.0)
                .min_w(0.0),
        );
        if let Some(Some(key)) = targets.get(n) {
            let key = key.clone();
            line_row = line_row.child(
                pressable(roles::CONTROL.icon_label("Open \u{203a}", p.ink))
                    .key(format!("brief-open-{n}"))
                    .label(format!("Open the finding for line {}", n + 1))
                    .px(6.0)
                    .py(3.0)
                    .mt(2.0)
                    .radius(6.0)
                    .hover(p.chip)
                    .on_press(move |s: &mut State| s.open_finding(&key)),
            );
        }
        c = c.child(line_row);
    }
    c
}

// ---- the triage list -------------------------------------------------------------------------

fn change_badge(change: FindingChange, p: Palette) -> El {
    match change {
        FindingChange::New => badge("New", p.ink, Some(p.chip)),
        FindingChange::Changed => badge("Changed", p.open, Some(p.chip)),
        FindingChange::StillOpen => badge("Still open", p.faint, None),
    }
}

fn due_chip(due: NaiveDate, today: NaiveDate, p: Palette) -> El {
    let (ink, bg, weight) = if due < today {
        (p.warn, Some(p.warnbg), 600.0)
    } else if due == today {
        (p.ink, Some(p.chip), 600.0)
    } else {
        (p.soft, None, 500.0)
    };
    let chip = text(format!("Due {}", due.format("%-d %b")))
        .font(
            roles::Role {
                weight,
                ..roles::META
            }
            .style(),
        )
        .color(ink)
        .px(8.0)
        .py(1.0)
        .radius(999.0);
    match bg {
        Some(f) => chip.fill(f),
        None => chip,
    }
}

#[derive(Clone, Copy)]
struct Lay {
    /// Two lines per row: the title, then kind, project and due date.
    compact: bool,
    project: bool,
}

fn finding_row(s: &State, r: &DailyCheck, i: usize, lay: Lay, p: Palette) -> El {
    let c = &s.check;
    let f = &r.content.findings[i];
    let key = key_of(r, i);
    let status = c.status_at(r, i);
    let done = status != TriageStatus::Open;
    let selected = c.panel && c.cursor.as_deref() == Some(key.as_str());
    let at_cursor = c.keyboard && c.cursor.as_deref() == Some(key.as_str()) && !selected;
    let due = finding_due(s, f);
    let ink = if done { p.faint } else { p.ink };
    let title = |s: &str| {
        if done {
            roles::SMALL.line(s, ink).shrink()
        } else {
            roles::NAME.line(s, ink).shrink()
        }
    };
    let mark = dot(8.0, if done { p.line } else { tone(&f.severity, p) });
    let project_name = f.project.clone().unwrap_or_default();
    let open_key = key.clone();
    let mut line = if lay.compact {
        let mut meta = row()
            .gap(8.0)
            .items_center()
            .min_w(0.0)
            .child(roles::META.line(kind_short(&f.kind), p.faint));
        if !project_name.is_empty() {
            meta = meta
                .child(roles::META.wrap("\u{b7}", p.faint))
                .child(roles::META.line(&project_name, p.faint));
        }
        if let Some(d) = due {
            meta = meta.child(due_chip(d, c.today, p));
        }
        let mut side = col().gap(2.0).items_end().justify_center();
        if let Some(ch) = change_of(r, f).filter(|_| !done) {
            side = side.child(change_badge(ch, p));
        }
        pressable(
            row()
                .w_full()
                .gap(10.0)
                .items_center()
                .child(mark)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(title(&f.title))
                        .child(meta),
                )
                .child(side),
        )
        .h(54.0)
    } else {
        let mut cells = row()
            .w_full()
            .gap(10.0)
            .items_center()
            .child(mark)
            .child(title(&f.title).grow(1.0))
            .child(
                row()
                    .w(150.0)
                    .items_center()
                    .child(roles::META.line(kind_short(&f.kind), p.faint)),
            );
        if lay.project {
            cells = cells.child(
                row()
                    .w(150.0)
                    .items_center()
                    .gap(6.0)
                    .child(if project_name.is_empty() {
                        el()
                    } else {
                        dot(7.0, p.soft)
                    })
                    .child(roles::META.line(&project_name, p.faint)),
            );
        }
        cells = cells
            .child(row().w(84.0).justify_end().items_center().child(
                match change_of(r, f).filter(|_| !done) {
                    Some(ch) => change_badge(ch, p),
                    None => el(),
                },
            ))
            .child(row().w(86.0).justify_end().items_center().child(match due {
                Some(d) => due_chip(d, c.today, p),
                None => el(),
            }));
        pressable(cells).h(38.0)
    };
    line = line
        .key(format!("frow-{i}"))
        .label(format!("Open the finding: {}", f.title))
        .w_full()
        .px(8.0)
        .radius(6.0)
        .hover(p.chip)
        .on_press(move |s: &mut State| s.open_finding(&open_key));
    if selected {
        line = line
            .fill(p.sel)
            .child(el().w(2.0).h(18.0).radius(1.0).fill(p.ink).pin(
                Some(0.0),
                Some(10.0),
                None,
                None,
            ));
    } else if at_cursor {
        line = line.fill(p.chip).outline(1.0, 0.0, p.line);
    }
    line
}

fn group_head(name: &str, n: usize, ink: rune::Color, p: Palette) -> El {
    row()
        .w_full()
        .h(32.0)
        .px(8.0)
        .gap(8.0)
        .items_center()
        .child(dot(8.0, ink))
        .child(roles::GROUP.wrap(name, p.ink))
        .child(roles::META.medium().wrap(n.to_string(), p.faint))
}

fn filter_pills(s: &State, r: &DailyCheck, p: Palette) -> El {
    let c = &s.check;
    let mut bar = row().w_full().flex_wrap().items_center().gaps(6.0, 6.0);
    for (f, name, key) in [
        (Filter::Open, "Open", "filter-open"),
        (Filter::Snoozed, "Snoozed", "filter-snoozed"),
        (Filter::Dismissed, "Dismissed", "filter-dismissed"),
        (Filter::Applied, "Applied", "filter-applied"),
    ] {
        let n = c.count_status(f.status());
        if f == Filter::Applied && n == 0 && c.filter != Filter::Applied {
            continue;
        }
        bar = bar.child(
            pill(&format!("{name} {n}"), key, c.filter == f, p)
                .label(format!("Show {name} findings"))
                .on_press(move |s: &mut State| {
                    s.check.filter = f;
                    s.check.cursor = None;
                    s.check.panel = false;
                }),
        );
    }
    if let Some(ch) = c.change {
        let name = match ch {
            FindingChange::New => "New only",
            FindingChange::Changed => "Changed only",
            FindingChange::StillOpen => "Still open only",
        };
        bar = bar.child(
            pill(&format!("{name}  \u{d7}"), "filter-change", true, p)
                .label("Clear the change filter")
                .on_press(|s: &mut State| s.check.change = None),
        );
    }
    let names = projects(r);
    if names.len() > 1 {
        bar = bar.child(el().grow(1.0));
    }
    bar
}

fn project_menu(s: &State, k: &Kit, r: &DailyCheck, p: Palette, layer: &mut Vec<El>) -> Option<El> {
    let names = projects(r);
    if names.len() < 2 {
        return None;
    }
    let c = &s.check;
    let shown = c.project.clone().unwrap_or_else(|| "All projects".into());
    let trigger = pressable(
        row()
            .gap(6.0)
            .items_center()
            .child(roles::META.wrap("Project", p.faint))
            .child(roles::GROUP.wrap(shown.clone(), p.ink))
            .child(label("\u{25be}", 10.0, 400.0, p.faint)),
    )
    .key("check-project")
    .label(format!("Project filter: {shown}"))
    .px(10.0)
    .py(5.0)
    .radius(8.0)
    .border(1.0, p.line)
    .fill(p.card)
    .hover(p.chip)
    .on_press(|s: &mut State| s.check.project_menu = !s.check.project_menu);
    let mut menu = Menu::new("check-project-menu")
        .trigger(trigger)
        .open(c.project_menu)
        .placement(Placement::BottomEnd)
        .min_width(200.0);
    if c.project_menu {
        menu = menu.on_close(|s: &mut State| s.check.project_menu = false);
        menu = menu.item(
            MenuItem::new("All projects")
                .checked(c.project.is_none())
                .on_select(|s: &mut State| {
                    s.check.project = None;
                    s.check.project_menu = false;
                }),
        );
        for n in names {
            let on = c.project.as_deref() == Some(n.as_str());
            menu = menu.item(MenuItem::new(n.clone()).checked(on).on_select(
                move |s: &mut State| {
                    s.check.project = Some(n.clone());
                    s.check.project_menu = false;
                },
            ));
        }
    }
    Some(menu.build(k, layer))
}

fn attention(s: &State, k: &Kit, r: &DailyCheck, lay: Lay, p: Palette, layer: &mut Vec<El>) -> El {
    let c = &s.check;
    let mut out = col().key("check-attention").w_full();
    let open_n = c.count_status(TriageStatus::Open);
    out = out.child(heading("Needs attention", Some(open_n), "check-attention-head", p).mt(30.0));
    let mut bar = filter_pills(s, r, p);
    if let Some(menu) = project_menu(s, k, r, p, layer) {
        bar = bar.child(menu);
    }
    out = out.child(bar.mb(8.0));
    if r.content.findings.is_empty() {
        return out.child(roles::SMALL.wrap("This check found nothing to flag.", p.soft));
    }
    let shown = rows(s);
    if shown.is_empty() {
        let text_ = match c.filter {
            Filter::Open => "Nothing open here. Everything shown by the filters is handled.",
            Filter::Snoozed => "Nothing is snoozed.",
            Filter::Dismissed => "Nothing is dismissed.",
            Filter::Applied => "Nothing has been applied.",
        };
        return out.child(roles::SMALL.wrap(text_, p.soft).py(10.0));
    }
    for (sev, ink) in [
        (Severity::High, p.warn),
        (Severity::Medium, p.open),
        (Severity::Low, p.faint),
    ] {
        let of: Vec<usize> = shown
            .iter()
            .copied()
            .filter(|&i| sev_rank(&r.content.findings[i].severity) == sev_rank(&sev))
            .collect();
        if of.is_empty() {
            continue;
        }
        let mut group = col()
            .w_full()
            .mt(roles::GROUP_TOP)
            .gap(1.0)
            .child(group_head(severity_name(&sev), of.len(), ink, p));
        for i in of {
            group = group.child(finding_row(s, r, i, lay, p));
        }
        out = out.child(group);
    }
    out
}

// ---- in the Diary ----------------------------------------------------------------------------

/// "Full check" for one day, as a quiet link.
fn report_link(date: NaiveDate, words: &str, p: Palette) -> El {
    link(format!("{words} \u{203a}"), p)
        .key(format!("diary-report-{date}"))
        .label(format!("Open the full check of {}", date.format("%-d %b")))
        .on_press(move |s: &mut State| s.open_report(date))
}

/// An older day's check in one quiet line, with a link to its report.
pub(super) fn day_line(s: &State, date: NaiveDate, p: Palette) -> El {
    let said = match s.check.totals.get(&date) {
        Some((0, _)) => "The daily check found nothing to flag.".to_string(),
        Some((n, Some(new))) if *new > 0 => format!(
            "The daily check flagged {}, {new} of them new.",
            plural(*n, "thing", "things")
        ),
        Some((n, _)) => format!("The daily check flagged {}.", plural(*n, "thing", "things")),
        None => "The daily check ran.".to_string(),
    };
    row()
        .w_full()
        .flex_wrap()
        .items_center()
        .gaps(2.0, 8.0)
        .child(roles::SMALL.wrap(said, p.soft))
        .child(report_link(date, "Open", p))
}

// ---- changed since ---------------------------------------------------------------------------

fn changed_section(s: &State, r: &DailyCheck, p: Palette) -> El {
    let c = &s.check;
    let cmp = &r.content.comparison;
    let mut out = col().key("check-changed").w_full();
    let Some(base) = cmp.baseline_date.filter(|_| is_v2(r)) else {
        let note = if is_v2(r) {
            "This is the first check on record, so there is nothing to compare it with."
        } else {
            "This report is from before change tracking, so it cannot say what changed."
        };
        return out
            .child(heading(
                "Changed since the last check",
                None,
                "check-changed-head",
                p,
            ))
            .child(roles::SMALL.wrap(note, p.soft));
    };
    out = out.child(heading(
        &format!("Changed since {}", base.format("%a %-d %b")),
        None,
        "check-changed-head",
        p,
    ));
    let chips = row()
        .w_full()
        .flex_wrap()
        .items_center()
        .gaps(6.0, 6.0)
        .child(
            pill(
                &format!("New {}", cmp.new.len()),
                "changed-new",
                c.change == Some(FindingChange::New),
                p,
            )
            .label("Show only the new findings")
            .on_press(|s: &mut State| toggle_change(s, FindingChange::New)),
        )
        .child(
            pill(
                &format!("Changed {}", cmp.changed.len()),
                "changed-changed",
                c.change == Some(FindingChange::Changed),
                p,
            )
            .label("Show only the changed findings")
            .on_press(|s: &mut State| toggle_change(s, FindingChange::Changed)),
        )
        .child(
            pill(
                &format!("Carried over {}", cmp.carried.len()),
                "changed-carried",
                c.change == Some(FindingChange::StillOpen),
                p,
            )
            .label("Show only the findings carried over")
            .on_press(|s: &mut State| toggle_change(s, FindingChange::StillOpen)),
        )
        .child(
            pill(
                &format!("Resolved {}", cmp.resolved.len()),
                "changed-resolved",
                c.gone == Some(Gone::Resolved),
                p,
            )
            .label("List the resolved findings")
            .on_press(|s: &mut State| toggle_gone(s, Gone::Resolved)),
        )
        .child(
            pill(
                &format!("Unmatched {}", cmp.unmatched.len()),
                "changed-unmatched",
                c.gone == Some(Gone::Unmatched),
                p,
            )
            .label("List the findings that could not be matched")
            .on_press(|s: &mut State| toggle_gone(s, Gone::Unmatched)),
        );
    out = out.child(chips);
    if !cmp.complete {
        out = out.child(
            roles::SMALL.medium().wrap("The comparison is incomplete. This check or the one before it had warnings, so a finding that is missing is not necessarily resolved.", p.warn)
            .mt(8.0),
        );
    }
    if let Some(g) = c.gone {
        let (ids, note) = match g {
            Gone::Resolved => (&cmp.resolved, "New evidence shows these were settled."),
            Gone::Unmatched => (
                &cmp.unmatched,
                "Not in today's check, and nothing shows they were settled. They are not counted as resolved.",
            ),
        };
        let mut list = col()
            .w_full()
            .gap(4.0)
            .mt(8.0)
            .px(12.0)
            .py(10.0)
            .radius(10.0)
            .fill(p.side);
        list = list.child(roles::META.wrap(note, p.soft));
        if ids.is_empty() {
            list = list.child(roles::META.wrap("None.", p.faint));
        }
        for id in ids {
            let title = c
                .baseline_titles
                .get(id)
                .cloned()
                .unwrap_or_else(|| "A finding from an older report".into());
            let why = r
                .content
                .resolutions
                .iter()
                .find(|x| &x.finding_id == id)
                .map(|x| x.explanation.clone());
            let mut item = col()
                .w_full()
                .gap(1.0)
                .child(roles::SMALL.bold().wrap(title, p.ink));
            if let Some(w) = why.filter(|_| g == Gone::Resolved) {
                item = item.child(roles::META.wrap(w, p.soft));
            }
            list = list.child(item);
        }
        out = out.child(list);
    }
    out
}

fn toggle_change(s: &mut State, ch: FindingChange) {
    s.check.change = if s.check.change == Some(ch) {
        None
    } else {
        Some(ch)
    };
    s.check.filter = Filter::Open;
    s.check.cursor = None;
    s.check.panel = false;
}

fn toggle_gone(s: &mut State, g: Gone) {
    s.check.gone = if s.check.gone == Some(g) {
        None
    } else {
        Some(g)
    };
}

// ---- story alignment -------------------------------------------------------------------------

fn status_pill(status: AlignmentStatus, p: Palette) -> El {
    match status {
        AlignmentStatus::Aligned => badge("Aligned", p.ok, Some(p.okbg)),
        AlignmentStatus::Tension => badge("Tension", p.warn, Some(p.warnbg)),
        AlignmentStatus::Unknown => badge("Unknown", p.soft, Some(p.chip)),
        AlignmentStatus::NeedsDecision => badge("Needs decision", p.open, Some(p.chip)),
    }
}

fn claim_column(
    s: &State,
    title: &str,
    claim: &core::check_v2::Claim,
    key: &str,
    p: Palette,
) -> El {
    let mut c = col()
        .w(0.0)
        .grow(1.0)
        .min_w(0.0)
        .gap(6.0)
        .px(12.0)
        .py(10.0)
        .radius(10.0)
        .fill(p.card)
        .border(1.0, p.line)
        .child(spaced_caps(title, p));
    if claim.text.trim().is_empty() {
        c = c.child(roles::SMALL.wrap("Nothing on record.", p.faint));
    } else {
        c = c.child(roles::SMALL.wrap(&claim.text, p.ink).selectable(true));
    }
    if claim.sources.is_empty() {
        c = c.child(roles::META.wrap("No source cited.", p.faint));
    }
    c.children(source_rows(
        s,
        claim.sources.iter().map(String::as_str),
        &format!("{key}-src"),
        p,
    ))
}

fn story_section(s: &State, k: &Kit, r: &DailyCheck, narrow: bool, p: Palette) -> El {
    let c = &s.check;
    let mut out = col().key("check-story").w_full();
    if !is_v2(r) {
        out = out.child(heading("Story by project", None, "check-story-head", p));
        if r.content.story.trim().is_empty() {
            return out.child(roles::SMALL.wrap("This report has no story.", p.soft));
        }
        out = out.child(
            super::widgets::action(
                format!(
                    "{}  {} the story",
                    if c.story_text_open {
                        "\u{25be}"
                    } else {
                        "\u{25b8}"
                    },
                    if c.story_text_open { "Hide" } else { "Show" }
                ),
                p,
            )
            .key("story-toggle")
            .self_start()
            .py(4.0)
            .on_press(|s: &mut State| s.check.story_text_open = !s.check.story_text_open),
        );
        if c.story_text_open {
            out = out.child(prose(s, k, p, &r.content.story, "check-story", 15.0).mt(8.0));
        }
        return out;
    }
    let all = &r.content.story_alignment;
    out = out.child(heading(
        "Story alignment",
        Some(all.len()),
        "check-story-head",
        p,
    ));
    if all.is_empty() {
        return out.child(roles::SMALL.wrap("This check made no story comparison.", p.soft));
    }
    let mut list = col().w_full().gap(1.0);
    for (n, a) in all.iter().enumerate() {
        let open = c.story_open.contains(&a.project);
        let project = a.project.clone();
        let head = pressable(
            row()
                .w_full()
                .h(38.0)
                .px(8.0)
                .gap(8.0)
                .items_center()
                .child(chevron(open, p))
                .child(dot(7.0, p.soft))
                .child(roles::SMALL.bold().line(&a.project, p.ink).grow(1.0))
                .child(status_pill(a.status, p)),
        )
        .key(format!("story-row-{n}"))
        .label(format!(
            "{} {}",
            if open { "Fold" } else { "Unfold" },
            a.project
        ))
        .w_full()
        .radius(6.0)
        .hover(p.chip)
        .on_press(move |s: &mut State| {
            if !s.check.story_open.remove(&project) {
                s.check.story_open.insert(project.clone());
            }
        });
        let mut item = col().w_full().gap(1.0).child(head);
        if open {
            let kb = claim_column(
                s,
                "The knowledge base says",
                &a.kb_claim,
                &format!("sk-{n}"),
                p,
            );
            let meet = claim_column(
                s,
                "The meetings say",
                &a.meeting_claim,
                &format!("sm-{n}"),
                p,
            );
            let pair = if narrow {
                col().w_full().gap(8.0).child(kb).child(meet)
            } else {
                row().w_full().gap(10.0).child(kb).child(meet)
            };
            let mut body = col()
                .w_full()
                .gap(8.0)
                .pl(if narrow { 4.0 } else { 30.0 })
                .pt(4.0)
                .pb(10.0)
                .child(pair);
            if !a.question.trim().is_empty() {
                body = body.child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .child(label("?", 14.0, 700.0, p.open))
                        .child(roles::CONTROL.wrap(&a.question, p.ink).grow(1.0).min_w(0.0)),
                );
            }
            item = item.child(body);
        }
        list = list.child(item);
    }
    out.child(list)
}

// ---- history ---------------------------------------------------------------------------------

fn history_section(s: &State, narrow: bool, p: Palette) -> El {
    let c = &s.check;
    let mut out = col().key("check-history").w_full();
    out = out.child(heading("Last 7 days", None, "check-history-head", p));
    let mut strip = row().w_full().gap(if narrow { 4.0 } else { 8.0 });
    for d in &c.strip {
        let on = Some(d.date) == c.date;
        let day = d.date;
        let name = format!("{} {}", d.date.format("%a"), d.date.day());
        let mut cell = col()
            .w(0.0)
            .grow(1.0)
            .min_w(0.0)
            .gap(2.0)
            .px(if narrow { 4.0 } else { 8.0 })
            .py(8.0)
            .radius(10.0)
            .items_center()
            .child(roles::META.medium().wrap(name.clone(), p.faint));
        match d.counts {
            Some((total, new)) => {
                cell = cell
                    .child(
                        rune::text(total.to_string())
                            .color(p.ink)
                            .font(roles::SECTION.style()),
                    )
                    .child(roles::META.wrap(
                        match new {
                            Some(n) => format!("{n} new"),
                            None => "findings".to_string(),
                        },
                        p.soft,
                    ));
            }
            None => {
                cell = cell
                    .child(
                        label("\u{2013}", 20.0, 400.0, p.faint)
                            .font(roles::SECTION.regular().style()),
                    )
                    .child(roles::meta("No check", p));
            }
        }
        let cell = if d.counts.is_some() {
            pressable(cell)
                .key(format!("strip-{day}"))
                .label(format!("Open the check of {name}"))
                .fill(if on { p.card } else { p.side })
                .border(1.0, if on { p.ink } else { p.line })
                .hover(p.chip)
                .on_press(move |s: &mut State| s.pick_check(day))
        } else {
            cell.border(1.0, p.line)
        };
        strip = strip.child(cell);
    }
    out.child(strip).child(
        roles::META
            .wrap(
                "Each box is one day. A box that says No check is a day when no check ran.",
                p.faint,
            )
            .mt(8.0),
    )
}

// ---- sources ---------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum SourceGroup<'a> {
    Meeting {
        id: &'a str,
        stamps: Vec<&'a str>,
        untimed: bool,
    },
    Other(&'a str),
}

fn group_sources<'a>(sources: impl IntoIterator<Item = &'a str>) -> Vec<SourceGroup<'a>> {
    let mut groups = Vec::new();
    for source in sources {
        let Some(citation) = source.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) else {
            groups.push(SourceGroup::Other(source));
            continue;
        };
        let (id, stamp) = citation
            .split_once('@')
            .map_or((citation, None), |(id, stamp)| (id, Some(stamp)));
        if let Some(SourceGroup::Meeting {
            stamps, untimed, ..
        }) = groups
            .iter_mut()
            .find(|g| matches!(g, SourceGroup::Meeting { id: seen, .. } if *seen == id))
        {
            *untimed |= stamp.is_none();
            if let Some(stamp) = stamp
                && !stamps.contains(&stamp)
            {
                stamps.push(stamp);
            }
        } else {
            groups.push(SourceGroup::Meeting {
                id,
                stamps: stamp.into_iter().collect(),
                untimed: stamp.is_none(),
            });
        }
    }
    groups
}

#[cfg(test)]
#[test]
fn meeting_sources_group_in_order_and_keep_distinct_times() {
    assert_eq!(
        group_sources([
            "[[1800000000001-1@00:05]]",
            "docs/demo.md",
            "[[1800000000001-1@00:15]]",
            "[[1800000000001-1@00:05]]",
            "[[1800000000002-1]]",
        ]),
        [
            SourceGroup::Meeting {
                id: "1800000000001-1",
                stamps: vec!["00:05", "00:15"],
                untimed: false
            },
            SourceGroup::Other("docs/demo.md"),
            SourceGroup::Meeting {
                id: "1800000000002-1",
                stamps: vec![],
                untimed: true
            },
        ],
    );
}

pub fn source_rows<'a>(
    s: &State,
    sources: impl IntoIterator<Item = &'a str>,
    key: &str,
    p: Palette,
) -> Vec<El> {
    group_sources(sources)
        .into_iter()
        .enumerate()
        .map(|(i, group)| {
            let key = format!("{key}-{i}");
            match group {
                SourceGroup::Other(value) => copy_row(s, value, key, p),
                SourceGroup::Meeting {
                    id,
                    stamps,
                    untimed,
                } if stamps.is_empty() || (stamps.len() == 1 && !untimed) => {
                    let target = stamps
                        .first()
                        .map_or_else(|| id.to_owned(), |time| format!("{id}@{time}"));
                    source_row(s, &format!("[[{target}]]"), key, p)
                }
                SourceGroup::Meeting { id, stamps, .. } => {
                    let mut group = row()
                        .w_full()
                        .gap(4.0)
                        .min_w(0.0)
                        .items_center()
                        .flex_wrap()
                        .key(&key)
                        .child(source_row(
                            s,
                            &format!("[[{id}]]"),
                            format!("{key}-meeting"),
                            p,
                        ));
                    if let Some(meeting) = s.meetings.iter().find(|m| m.id == id) {
                        for (n, stamp) in stamps.into_iter().enumerate() {
                            let target = format!("{id}@{stamp}");
                            let time_key = format!("{key}-time-{n}");
                            group = group.child(
                                quiet_button(cite_time(stamp), &time_key, p)
                                    .label(format!(
                                        "Open {} at {}",
                                        meeting.title,
                                        cite_time(stamp)
                                    ))
                                    .on_press_tx(move |s: &mut State, tx| {
                                        s.open_check_citation(&target, time_key.clone(), tx)
                                    }),
                            );
                        }
                    }
                    group
                }
            }
        })
        .collect()
}

/// A meeting citation as a link that opens the meeting at that line, or a KB path with a copy
/// button.
pub fn source_row(s: &State, e: &str, key: String, p: Palette) -> El {
    if let Some(cite) = e.strip_prefix("[[").and_then(|c| c.strip_suffix("]]")) {
        let (id, stamp) = cite
            .split_once('@')
            .map_or((cite, None), |(i, t)| (i, Some(t)));
        let (title, at) = match (s.meetings.iter().find(|m| m.id == id), stamp) {
            (Some(m), Some(t)) => (m.title.clone(), Some(cite_time(t))),
            (Some(m), None) => (m.title.clone(), None),
            (None, _) => (format!("Meeting {id} is not in the list"), None),
        };
        let shown = match &at {
            Some(t) => format!("{title} \u{b7} {t}"),
            None => title.clone(),
        };
        let known = s.meetings.iter().any(|m| m.id == id);
        let target = cite.to_string();
        let origin = key.clone();
        let ink = if known { p.ink } else { p.faint };
        let mut content = row().gap(6.0).items_center().child(
            rune::text(title)
                .font(roles::CONTROL.style())
                .color(ink)
                .truncate()
                .min_w(0.0)
                .shrink(),
        );
        if let Some(t) = at {
            content = content.child(
                rune::text(format!("\u{b7} {t}"))
                    .font(roles::CONTROL.style())
                    .color(ink),
            );
        }
        if !known {
            return content;
        }
        return pressable(content)
            .key(key)
            .label(format!("Open the meeting: {shown}"))
            .help("Open the meeting at that line")
            .w_full()
            .max_w(PAGE_COLUMN)
            .min_w(0.0)
            .px(4.0)
            .py(1.0)
            .radius(5.0)
            .hover(p.chip)
            .on_press_tx(move |s: &mut State, tx| {
                s.open_check_citation(&target, origin.clone(), tx)
            });
    }
    copy_row(s, e, key, p)
}

/// A KB path or command as plain text, with a copy action.
pub fn copy_row(s: &State, value: &str, key: String, p: Palette) -> El {
    let copied = s.check.copied.as_deref() == Some(value);
    let v = value.to_string();
    row()
        .w_full()
        .gap(8.0)
        .items_center()
        .child(
            rune::text(value.to_string())
                .font(roles::META.style().mono())
                .color(p.soft)
                .wrap()
                .selectable(true)
                .min_w(0.0)
                .shrink(),
        )
        .child(
            quiet_button(if copied { "Copied" } else { "Copy" }, &key, p)
                .label(format!("Copy {value}"))
                .on_press_tx(move |s: &mut State, tx| {
                    tx.copy(v.clone());
                    s.check.copied = Some(v.clone());
                }),
        )
}

// ---- the panel -------------------------------------------------------------------------------

fn snooze_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        s.check.snooze_pick = false;
        s.check.snooze_date = TextEdit::default();
        s.check.snooze_error = None;
        tx.blur();
        return true;
    }
    false
}

fn snooze_menu(s: &State, k: &Kit, key: &str, p: Palette, layer: &mut Vec<El>) -> El {
    let c = &s.check;
    let today = c.today;
    let trigger = value_button(
        row()
            .gap(8.0)
            .items_center()
            .child(roles::CONTROL.wrap("Snooze", p.ink))
            .child(label("\u{25be}", 10.0, 400.0, p.faint)),
        "snooze",
        "Snooze this finding (S)".to_string(),
        p,
    )
    .on_press(|s: &mut State| s.check.snooze_menu = !s.check.snooze_menu);
    let mut menu = Menu::new("snooze-menu")
        .trigger(trigger)
        .open(c.snooze_menu)
        .placement(Placement::TopStart)
        .min_width(190.0);
    if c.snooze_menu {
        menu = menu.on_close(|s: &mut State| s.check.snooze_menu = false);
        let tomorrow = today.succ_opt().unwrap_or(today);
        let week = core::next_week(today);
        let (k1, k2) = (key.to_string(), key.to_string());
        menu = menu
            .item(
                MenuItem::new(format!("Tomorrow, {}", tomorrow.format("%a %-d %b")))
                    .on_select(move |s: &mut State| s.snooze_finding(&k1, tomorrow)),
            )
            .item(
                MenuItem::new(format!("Next week, {}", week.format("%a %-d %b")))
                    .on_select(move |s: &mut State| s.snooze_finding(&k2, week)),
            )
            .item(MenuItem::new("Pick a date...").on_select(|s: &mut State| {
                s.check.snooze_menu = false;
                s.check.snooze_pick = true;
                s.check.snooze_error = None;
                s.check.snooze_date = TextEdit::new("");
                s.check.snooze_date.focused = true;
            }));
    }
    menu.build(k, layer)
}

fn snooze_pick_row(s: &State, k: &Kit, p: Palette) -> El {
    let c = &s.check;
    let input = TextInput::new(&c.snooze_date)
        .plain()
        .placeholder("Back on: friday, 12 Nov")
        .on_edit(|s: &mut State| &mut s.check.snooze_date)
        .on_submit(|s: &mut State, _| s.snooze_commit())
        .on_key(snooze_key)
        .height(34.0)
        .build(k)
        .key("snooze-input")
        .border(
            1.0,
            if c.snooze_date.focused {
                p.accent
            } else {
                p.line
            },
        )
        .radius(8.0)
        .fill(p.card)
        .grow(1.0)
        .min_w(0.0);
    let mut out = col().w_full().gap(6.0).child(
        row()
            .w_full()
            .gap(8.0)
            .items_center()
            .child(input)
            .child(
                button("Snooze", "snooze-go", true, p).on_press(|s: &mut State| s.snooze_commit()),
            )
            .child(
                button("Cancel", "snooze-cancel", false, p).on_press_tx(|s: &mut State, tx| {
                    s.check.snooze_pick = false;
                    s.check.snooze_date = TextEdit::default();
                    tx.blur();
                }),
            ),
    );
    if let Some(e) = &c.snooze_error {
        out = out.child(roles::SMALL.medium().wrap(e, p.warn).role("alert"));
    }
    out
}

fn status_line(h: &Handled, v2: bool, p: Palette) -> (String, rune::Color, rune::Color) {
    let at = h.at.as_deref().map(when).unwrap_or_default();
    match h.status {
        TriageStatus::Dismissed => (
            if v2 {
                format!(
                    "Dismissed {at}. That is your judgment, not a fix. It comes back if new evidence appears."
                )
            } else {
                format!("Dismissed {at}.")
            },
            p.soft,
            p.chip,
        ),
        TriageStatus::Snoozed => (
            format!(
                "Snoozed until {}. It comes back sooner if new evidence appears.",
                h.until.map_or_else(
                    || "a later day".into(),
                    |d| d.format("%a %-d %b").to_string()
                )
            ),
            p.soft,
            p.chip,
        ),
        TriageStatus::Applied => (format!("Applied {at}."), p.ok, p.okbg),
        TriageStatus::Open => (String::new(), p.soft, p.chip),
    }
}

fn finding_panel(
    s: &State,
    k: &Kit,
    r: &DailyCheck,
    i: usize,
    overlay: bool,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    let c = &s.check;
    let f = &r.content.findings[i];
    let key = key_of(r, i);
    let v2 = is_v2(r);
    let handled = c.handled.get(&key);
    let status = handled.map_or(TriageStatus::Open, |h| h.status);

    let mut badges = row().flex_wrap().gaps(6.0, 6.0).items_center().child(badge(
        severity_name(&f.severity),
        tone(&f.severity, p),
        Some(match f.severity {
            Severity::High => p.warnbg,
            _ => p.chip,
        }),
    ));
    badges = badges.child(badge(kind_name(&f.kind), p.soft, Some(p.chip)));
    if let Some(ch) = change_of(r, f).filter(|_| status == TriageStatus::Open) {
        badges = badges.child(change_badge(ch, p));
    }
    let mut props = col().w_full().gap(6.0);
    if let Some(n) = &f.project {
        props = props.child(prop_row(
            "Project",
            row()
                .gap(8.0)
                .items_center()
                .min_h(24.0)
                .child(dot(7.0, p.soft))
                .child(roles::CONTROL.wrap(n, p.ink)),
            p,
        ));
    }
    if v2 && let (Some(a), Some(b)) = (f.first_seen, f.last_seen) {
        let seen = if a == b {
            format!("{}", a.format("%-d %b"))
        } else {
            format!("{} to {}", a.format("%-d %b"), b.format("%-d %b"))
        };
        props = props.child(prop_row("Seen", roles::SMALL.wrap(seen, p.ink).mt(2.0), p));
    }
    if let Some(d) = finding_due(s, f) {
        props = props.child(prop_row("Due", due_chip(d, c.today, p), p));
    }
    props = props.child(prop_row(
        "Sources",
        roles::SMALL
            .wrap(plural(f.evidence.len(), "citation", "citations"), p.ink)
            .mt(2.0),
        p,
    ));

    let mut body = col()
        .w_full()
        .gap(16.0)
        .px(20.0)
        .pt(4.0)
        .pb(24.0)
        .child(roles::SECTION.wrap(&f.title, p.ink).w_full())
        .child(badges)
        .child(props)
        .child(hairline(p))
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(spaced_caps("What the check found", p))
                .child(prose(s, k, p, &f.explanation, &format!("frow-{i}"), 14.5)),
        );
    let mut ev = col()
        .w_full()
        .gap(1.0)
        .child(spaced_caps("Citations", p).mb(2.0));
    if f.evidence.is_empty() {
        ev = ev.child(roles::META.wrap("No citation.", p.faint));
    }
    ev = ev.children(source_rows(
        s,
        f.evidence.iter().map(String::as_str),
        &format!("ev-{i}"),
        p,
    ));
    ev = ev.child(
        roles::META
            .wrap(
                "A citation shows where this came from. It does not prove the finding.",
                p.faint,
            )
            .mt(4.0),
    );
    body = body.child(ev);

    if let Some(a) = f
        .action
        .as_ref()
        .filter(|_| status != TriageStatus::Dismissed)
    {
        let applied = status == TriageStatus::Applied;
        let (before, after, warn) = before_after(s, a);
        let mut box_ = col()
            .w_full()
            .gap(6.0)
            .px(12.0)
            .py(10.0)
            .radius(10.0)
            .fill(if applied { p.okbg } else { p.side })
            .child(spaced_caps(
                if applied {
                    "Applied"
                } else {
                    "Proposed change"
                },
                p,
            ))
            .child(
                roles::SMALL
                    .bold()
                    .wrap(describe(a), p.ink)
                    .selectable(true),
            );
        let pair = |name: &str, value: String, ink: rune::Color, strike: bool| {
            let mut v = roles::CONTROL.wrap(value.clone(), ink);
            if strike {
                v = v.strike(0, value.len(), None);
            }
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(roles::META.medium().wrap(name, p.faint).w(48.0))
                .child(v.grow(1.0).min_w(0.0))
        };
        if let Some(b) = before {
            box_ = box_.child(pair("Now", b, p.soft, !applied));
        }
        box_ = box_.child(pair(
            "After",
            after,
            if applied { p.ok } else { p.ink },
            false,
        ));
        box_ = box_.child(roles::META.wrap(
            format!(
                "In the meeting: {}",
                meeting_title(&s.meetings, &a.meeting_id)
            ),
            p.soft,
        ));
        if let Some(w) = warn.filter(|_| !applied) {
            box_ = box_.child(roles::SMALL.medium().wrap(w, p.warn));
        }
        box_ = box_.child(roles::META.wrap(no_undo(a), if applied { p.soft } else { p.warn }));
        body = body.child(box_);
    }
    if let Some(h) = handled.filter(|h| h.status != TriageStatus::Applied || f.action.is_none()) {
        let (line, ink, bg) = status_line(h, v2, p);
        body = body.child(
            roles::CONTROL
                .wrap(line, ink)
                .px(12.0)
                .py(8.0)
                .radius(10.0)
                .fill(bg),
        );
    }
    if let Some(err) = c.errors.get(&key) {
        body = body.child(roles::SMALL.medium().wrap(err, p.warn).role("alert"));
    }

    // The footer: what the owner can do with it.
    let mut foot = col().w_full().px(16.0).py(12.0).gap(8.0);
    if c.snooze_pick && status == TriageStatus::Open {
        foot = foot.child(snooze_pick_row(s, k, p));
    }
    let mut buttons = row().w_full().gap(8.0).items_center().flex_wrap();
    match status {
        TriageStatus::Open => {
            if f.action.is_some() {
                let kk = key.clone();
                buttons = buttons.child(
                    button("Apply", "apply", true, p)
                        .label("Apply the proposed change (A)")
                        .on_press(move |s: &mut State| s.apply_finding(&kk)),
                );
            }
            let kk = key.clone();
            buttons = buttons.child(
                button("Dismiss", "dismiss", false, p)
                    .label("Dismiss this finding (D)")
                    .on_press(move |s: &mut State| s.dismiss_finding(&kk)),
            );
            if v2 {
                buttons = buttons.child(snooze_menu(s, k, &key, p, layer));
            }
        }
        TriageStatus::Dismissed | TriageStatus::Snoozed => {
            let kk = key.clone();
            buttons = buttons.child(
                button("Undo", "undo", false, p)
                    .label("Bring this finding back (U)")
                    .on_press(move |s: &mut State| s.undo_finding(&kk)),
            );
        }
        TriageStatus::Applied => {}
    }
    foot = foot.child(buttons);
    let back = if c.report_open {
        "Daily check"
    } else {
        "Diary"
    };
    let top = panel_top("Finding", back, overlay, |s| s.close_finding(), p);
    let hint = if overlay {
        el()
    } else {
        row()
            .w_full()
            .flex_wrap()
            .gaps(4.0, 14.0)
            .child(hint_keys(&["A"], "apply", p))
            .child(hint_keys(&["D"], "dismiss", p))
            .child(hint_keys(&["S"], "snooze", p))
            .child(hint_keys(&["J", "K"], "next", p))
            .child(hint_keys(&["Esc"], "close", p))
    };
    panel_frame("check-panel", top, body, foot.child(hint), p)
}

fn hint_keys(keys: &[&str], what: &str, p: Palette) -> El {
    let mut r = row().gap(4.0).items_center();
    for k in keys {
        r = r.child(cap(k, p));
    }
    r.child(roles::META.wrap(what, p.faint).ml(2.0))
}

// ---- the page --------------------------------------------------------------------------------

fn list_column(
    s: &State,
    k: &Kit,
    p: Palette,
    narrow: bool,
    list_w: f32,
    layer: &mut Vec<El>,
) -> El {
    if !s.check.report_open {
        return diary::feed(s, k, p, narrow, layer);
    }
    let side = if narrow {
        16.0
    } else if s.check.panel {
        24.0
    } else {
        40.0
    };
    let mut doc = col()
        .key("check-top")
        .w_full()
        .max_w(920.0)
        .px(side)
        .pt(if narrow { 8.0 } else { 18.0 })
        .pb(56.0);
    if let Some(r) = &s.check.report {
        let lay = Lay {
            compact: narrow || list_w < 700.0,
            project: list_w >= 800.0,
        };
        let slim = narrow || list_w < 700.0;
        doc = doc
            .child(header(s, k, r, narrow, p, layer))
            .child(brief(s, k, r, p))
            .child(attention(s, k, r, lay, p, layer))
            .child(changed_section(s, r, p))
            .child(story_section(s, k, r, slim, p))
            .child(history_section(s, narrow, p));
    }
    col().grow(1.0).min_w(0.0).min_h(0.0).child(scroll(
        col().w_full().items_center().child(doc),
        "check-scroll",
    ))
}

pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let c = &s.check;
    let open_row = c.report.as_ref().filter(|_| c.panel).and_then(|r| {
        let key = c.cursor.as_deref()?;
        (0..r.content.findings.len())
            .find(|&i| key_of(r, i) == key)
            .map(|i| (r, i))
    });
    let (list_w, panel_w, overlay) = split_geometry(s, narrow, open_row.is_some());
    let mut pane = col()
        .key("check-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .fill(p.bg);
    if s.check.report_open {
        pane = pane.child(diary::return_link(s, p));
    }
    if let Some(note) = c.notice.as_ref().or(s.notice.as_ref()) {
        pane = pane.child(notice_bar(note, p));
    }
    let panel = open_row.map(|(r, i)| finding_panel(s, k, r, i, overlay, p, layer));
    let list = list_column(s, k, p, narrow, list_w, layer);
    pane.child(list_and_panel(list, panel, panel_w, overlay, p))
        .child(super::ask::overlay(s, k, p, narrow, layer))
}

// ---- the keyboard ----------------------------------------------------------------------------

fn move_cursor(s: &mut State, delta: isize, tx: &mut Tx) {
    // In the Diary the keys work inside "Needs your attention", so they open it.
    if !s.check.report_open
        && let Some(date) = s.check.report.as_ref().map(|r| r.date)
    {
        s.check
            .open_parts
            .insert(diary::part_key(date, diary::Part::Attention));
    }
    let shown = rows(s);
    let Some(r) = &s.check.report else {
        return;
    };
    if shown.is_empty() {
        return;
    }
    let keys: Vec<String> = shown.iter().map(|&i| key_of(r, i)).collect();
    let at = s
        .check
        .cursor
        .as_ref()
        .and_then(|c| keys.iter().position(|k| k == c));
    let next = match at {
        // The first press shows the cursor where it already is.
        Some(i) if s.check.keyboard => {
            (i as isize + delta).clamp(0, keys.len() as isize - 1) as usize
        }
        Some(i) => i,
        None if delta < 0 => keys.len() - 1,
        None => 0,
    };
    s.check.keyboard = true;
    let key = keys[next].clone();
    tx.scroll_into_view(format!("frow-{}", shown[next]));
    if s.check.panel {
        s.open_finding(&key);
    } else {
        s.check.cursor = Some(key);
    }
}

/// The Check page's keys. Letters only count while no text box has the keyboard.
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if !s.check.report_open {
        return false;
    }
    let Key::Text(t) = &e.key else {
        return match e.key {
            Key::Down => {
                move_cursor(s, 1, tx);
                true
            }
            Key::Up => {
                move_cursor(s, -1, tx);
                true
            }
            Key::Enter => match s.check.cursor.clone() {
                Some(key) if s.check.keyboard => {
                    s.open_finding(&key);
                    true
                }
                _ => false,
            },
            Key::Escape => {
                if s.check.snooze_menu {
                    s.check.snooze_menu = false;
                } else if s.check.panel {
                    s.close_finding();
                } else if s.check.picker_open || s.check.project_menu {
                    s.check.picker_open = false;
                    s.check.project_menu = false;
                } else if s.check.report_open {
                    s.close_report();
                } else {
                    s.go_home();
                }
                true
            }
            _ => false,
        };
    };
    let key = s.check.cursor.clone();
    match (t.as_str(), key) {
        ("j", _) => move_cursor(s, 1, tx),
        ("k", _) => move_cursor(s, -1, tx),
        ("a", Some(key)) if s.check.keyboard || s.check.panel => s.apply_finding(&key),
        ("d", Some(key)) if s.check.keyboard || s.check.panel => s.dismiss_finding(&key),
        ("u", Some(key)) if s.check.keyboard || s.check.panel => s.undo_finding(&key),
        ("s", Some(key)) if s.check.keyboard || s.check.panel => {
            let v2 = s.check.report.as_ref().is_some_and(is_v2);
            if v2 {
                s.open_finding(&key);
                s.check.snooze_menu = true;
            }
        }
        _ => return false,
    }
    true
}

// ---- the top bar pill and the Home card ------------------------------------------------------

/// The newest daily report in one line for Home's Today strip, and whether it has a high
/// finding open: "Daily check today: 2 high, 1 medium. 1 new." None when Daily check is off or
/// no report has been read yet.
pub fn last_line(s: &State) -> Option<(String, bool)> {
    if !s
        .config
        .features
        .enabled(sidenote::core::Feature::DailyCheck)
    {
        return None;
    }
    let c = s.check.latest;
    let date = c.date?;
    let when = if date == Local::now().date_naive() {
        "today".to_string()
    } else {
        date.format("%a %-d %b").to_string()
    };
    let mut parts = Vec::new();
    if c.high > 0 {
        parts.push(format!("{} high", c.high));
    }
    if c.medium > 0 {
        parts.push(format!("{} medium", c.medium));
    }
    if c.low > 0 {
        parts.push(format!("{} low", c.low));
    }
    let mut said = if parts.is_empty() {
        format!("Daily check {when}: nothing open.")
    } else {
        format!("Daily check {when}: {}.", parts.join(", "))
    };
    if c.new > 0 {
        said.push_str(&format!(" {} new.", c.new));
    }
    Some((said, c.high > 0))
}
