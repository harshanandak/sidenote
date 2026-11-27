//! The Memory page: what the company memory holds, one claim per row, with the proposals the
//! daily check wants approved and, until the owner accepts it, the migration of the old text.
//! Every change here is the owner's own action; the daily check only proposes.
use super::{
    State,
    check::{copy_row, prose, source_rows},
    list::text_title,
    roles, rows,
    tasks::{button, prop_row},
    theme::Palette,
    widgets::{
        ButtonDisabled, badge, dot, hairline, list_and_panel, notice_bar, panel_frame, panel_top,
        pressable, scroll, spaced_caps, split_geometry,
    },
};
use chrono::{DateTime, Local, NaiveDate};
use rune::{El, Key, KeyEvent, TextEdit, Tx, col, el, row};
use rune_kit::{Kit, Menu, MenuItem, Placement, TextArea, TextInput};
use sidenote::core::memory::{
    AcceptChoice, Basis, HistoryRow, MemoryEvent, MemoryRecord, MemorySource, MemoryStore,
    MigrationPreview, PendingProposal, ProposalOp, RecordStatus, RecordType, RevisionRequest,
};
use std::{collections::HashMap, io, rc::Rc};
mod landing;
pub(super) mod settled;

/// A claim not confirmed for this many days is suggested for review. Information only: age
/// never changes a claim.
const REVIEW_DAYS: i64 = 90;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Review,
    All,
    Kind(RecordType),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Active,
    NeedsReview,
    Retired,
}

/// Before migration: the old text, or the claims that would be imported.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Before {
    Text,
    Candidates,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    Retire,
    Forget,
}

#[derive(Clone)]
enum RecordWrite {
    Confirm,
    Retire,
    Forget,
    Undo(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FilterMenu {
    Type,
    State,
    Project,
}

pub struct MemState {
    pub meeting: Option<String>,
    pub settled: Vec<sidenote::core::settle::Row>,
    pub settle_all: bool,
    pub settle_meeting: Option<String>,
    pub settle_busy: bool,
    pub settle_toast_action: Option<(String, bool)>,
    pub settle_pending_toast: Option<(String, String, bool)>,
    pub browse: bool,
    pub filter_menu: Option<FilterMenu>,
    filters_loaded: bool,
    collapsed: Vec<RecordType>,
    pub comment_for: Option<String>,
    pub comment: TextEdit,
    rewrites: HashMap<String, Rewrite>,
    earlier_wording: Option<String>,
    pub result: Option<String>,
    pub projects: sidenote::core::Projects,
    pub records: Vec<MemoryRecord>,
    pub proposals: Vec<PendingProposal>,
    pub events: Vec<MemoryEvent>,
    history: Vec<HistoryRow>,
    comments_not_used: Vec<(String, String)>,
    pub migrated: bool,
    pub accepted_at: Option<String>,
    pub preview: Option<MigrationPreview>,
    pub preview_running: bool,
    pub legacy: String,
    pub tab: Tab,
    pub scope: Scope,
    pub before: Before,
    pub search: TextEdit,
    pub project: Option<String>,
    pub project_menu: bool,
    pub cursor: Option<String>,
    pub keyboard: bool,
    pub panel: bool,
    pub editing: bool,
    pub edit: TextEdit,
    pub edit_base: Option<MemoryRecord>,
    edit_sent: Option<String>,
    pub editor_generation: u64,
    pub ask: Option<Ask>,
    /// Why an action failed, by record, proposal or `migration`.
    pub errors: HashMap<String, String>,
    record_retries: HashMap<String, RecordWrite>,
    pub notice: Option<String>,
    pub refresh_pending: bool,
    refresh_generation: u64,
    refresh_again: Option<bool>,
    pub(super) refresh_error: Option<String>,
    prepare_after_refresh: bool,
    pub today: NaiveDate,
}

struct Rewrite {
    cancel: sidenote::core::ask::Cancel,
    next: Option<RevisionRequest>,
}

impl Default for MemState {
    fn default() -> Self {
        Self {
            meeting: None,
            settled: vec![],
            settle_all: false,
            settle_meeting: None,
            settle_busy: false,
            settle_toast_action: None,
            settle_pending_toast: None,
            browse: false,
            filter_menu: None,
            filters_loaded: false,
            collapsed: vec![],
            comment_for: None,
            comment: TextEdit::default(),
            rewrites: HashMap::new(),
            earlier_wording: None,
            result: None,
            projects: sidenote::core::Projects::default(),
            records: Vec::new(),
            proposals: Vec::new(),
            events: Vec::new(),
            history: Vec::new(),
            comments_not_used: Vec::new(),
            migrated: false,
            accepted_at: None,
            preview: None,
            preview_running: false,
            legacy: String::new(),
            tab: Tab::All,
            scope: Scope::Active,
            before: Before::Text,
            search: TextEdit::default(),
            project: None,
            project_menu: false,
            cursor: None,
            keyboard: false,
            panel: false,
            editing: false,
            edit: TextEdit::default(),
            edit_base: None,
            edit_sent: None,
            editor_generation: 0,
            ask: None,
            errors: HashMap::new(),
            record_retries: HashMap::new(),
            notice: None,
            refresh_pending: false,
            refresh_generation: 0,
            refresh_again: None,
            refresh_error: None,
            prepare_after_refresh: false,
            today: Local::now().date_naive(),
        }
    }
}

fn parse_day(at: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Local).date_naive())
}

fn needs_review(r: &MemoryRecord, today: NaiveDate) -> bool {
    match r.status {
        RecordStatus::Proposed => true,
        RecordStatus::Active => r
            .last_confirmed
            .as_deref()
            .and_then(parse_day)
            .is_none_or(|d| (today - d).num_days() > REVIEW_DAYS),
        _ => false,
    }
}

/// A claim's text for a one-line row: no citation markup, no emphasis marks.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        out.push_str(&rest[..start]);
        match rest[start..].find("]]") {
            Some(end) => rest = &rest[start + end + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    let out = out.replace(['*', '`'], "");
    let first = out
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let mut s = String::new();
    for w in first.split_whitespace() {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(w);
    }
    s
}

fn kind_label(t: RecordType) -> &'static str {
    match t {
        RecordType::Person => "People",
        RecordType::Project => "Projects",
        RecordType::Decision => "Decisions",
        RecordType::Commitment => "Commitments",
        RecordType::Question => "Open questions",
        RecordType::Story => "Story",
    }
}

fn kind_one(t: RecordType) -> &'static str {
    match t {
        RecordType::Person => "Person",
        RecordType::Project => "Project",
        RecordType::Decision => "Decision",
        RecordType::Commitment => "Commitment",
        RecordType::Question => "Open question",
        RecordType::Story => "Story",
    }
}

const KINDS: [RecordType; 6] = [
    RecordType::Person,
    RecordType::Project,
    RecordType::Decision,
    RecordType::Commitment,
    RecordType::Question,
    RecordType::Story,
];

fn basis_name(b: Basis) -> &'static str {
    match b {
        Basis::OwnerConfirmed => "You confirmed it",
        Basis::SourceSupported => "Supported by a source",
        Basis::Inferred => "Inferred, not stated",
        Basis::Unknown => "Not known",
    }
}

fn status_name(s: RecordStatus) -> &'static str {
    match s {
        RecordStatus::Proposed => "Proposed",
        RecordStatus::Active => "Active",
        RecordStatus::Superseded => "Replaced",
        RecordStatus::Retired => "Retired",
        RecordStatus::Forgotten => "Forgotten",
    }
}

fn op_name(op: ProposalOp) -> &'static str {
    match op {
        ProposalOp::Add => "Add",
        ProposalOp::Confirm => "Confirm",
        ProposalOp::Supersede => "Replace",
        ProposalOp::Retire => "Retire",
    }
}

fn day_text(at: &str) -> String {
    parse_day(at).map_or_else(|| at.to_string(), |d| d.format("%-d %b %Y").to_string())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn confirmed_text(r: &MemoryRecord) -> String {
    match r.last_confirmed.as_deref().and_then(parse_day) {
        Some(d) => format!("Confirmed {}", d.format("%-d %b")),
        None => "Not confirmed".to_string(),
    }
}

// ---- state ---------------------------------------------------------------------------------

#[derive(Default)]
pub(super) struct MemoryView {
    pub(super) file: sidenote::core::memory::MemoryFile,
    history: Vec<HistoryRow>,
    comments_not_used: Vec<(String, String)>,
}

impl MemoryView {
    pub(super) fn read(store: &MemoryStore) -> io::Result<Self> {
        let file = store.load()?;
        Ok(Self {
            history: store.history_rows(&file),
            comments_not_used: store.comments_not_used(&file),
            file: store.visible_file(file),
        })
    }
}

pub struct Snapshot {
    settled: Vec<sidenote::core::settle::Row>,
    mutation_generation: u64,
    projects: sidenote::core::Projects,
    file: MemoryView,
    preview: Option<MigrationPreview>,
    legacy: String,
}

impl Snapshot {
    fn read(store: &MemoryStore, full: bool) -> io::Result<Self> {
        if full {
            store.list_pending_proposals()?;
        }
        // Reconciliation can publish its own mutations before the snapshot read begins.
        let mutation_generation = sidenote::core::mutation_generation();
        let projects = sidenote::core::TaskStore::new(store.root.clone()).load_projects()?;
        let file = MemoryView::read(store)?;
        let (preview, legacy) = if file.file.migration.is_some() {
            (None, String::new())
        } else {
            (Some(store.preview_migration()?), store.company_memory()?)
        };
        let settled =
            sidenote::core::settle::Store::new(store.root.clone(), sidenote::core::meetings_dir())
                .history()?;
        Ok(Self {
            settled,
            mutation_generation,
            projects,
            file,
            preview,
            legacy,
        })
    }
}

impl MemState {
    fn begin_refresh(&mut self, full: bool) -> Option<u64> {
        self.notice = None;
        if self.refresh_pending {
            self.refresh_again = Some(self.refresh_again.unwrap_or(false) || full);
            return None;
        }
        self.refresh_generation += 1;
        self.refresh_pending = true;
        self.refresh_error = None;
        Some(self.refresh_generation)
    }

    fn finish_refresh(
        &mut self,
        generation: u64,
        mutation_generation: u64,
        result: Result<Snapshot, String>,
    ) -> Option<bool> {
        if generation != self.refresh_generation {
            return None;
        }
        self.refresh_pending = false;
        let again = self.refresh_again.take();
        match result {
            Ok(snapshot) => {
                if snapshot.mutation_generation != mutation_generation || again.is_some() {
                    return Some(again.unwrap_or(false));
                }
                self.apply_snapshot(snapshot);
            }
            Err(error) => self.refresh_error = Some(error),
        }
        again
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot) {
        self.today = Local::now().date_naive();
        self.settled = snapshot.settled;
        self.projects = snapshot.projects;
        self.project = self.project.as_deref().map(|n| self.projects.resolve(n));
        self.apply_file(snapshot.file);
        self.preview = snapshot.preview;
        self.legacy = snapshot.legacy;
        if !self.migrated {
            self.tab = Tab::All;
        }
        if !self.editing && self.cursor.as_ref().is_some_and(|c| !self.has_target(c)) {
            self.cursor = None;
            self.panel = false;
            self.editing = false;
        }
    }

    pub(super) fn apply_file(&mut self, view: MemoryView) -> bool {
        let changed =
            self.history != view.history || self.comments_not_used != view.comments_not_used;
        self.history = view.history;
        self.comments_not_used = view.comments_not_used;
        let file = view.file;
        self.migrated = file.migration.is_some();
        self.accepted_at = file.migration.as_ref().map(|m| m.accepted_at.clone());
        self.records = file.records;
        self.events = file.events;
        self.proposals = file
            .proposals
            .into_iter()
            .filter(|p| p.status == sidenote::core::memory::ProposalStatus::Pending)
            .collect();
        changed
    }
    pub fn typing(&self) -> bool {
        self.search.focused || self.edit.focused || self.comment.focused
    }

    fn has_target(&self, id: &str) -> bool {
        self.records.iter().any(|r| r.id == id)
            || self.proposals.iter().any(|p| p.proposal.id == id)
            || self
                .preview
                .as_ref()
                .is_some_and(|v| v.candidates.iter().any(|c| c.record.id == id))
    }

    fn record(&self, id: &str) -> Option<&MemoryRecord> {
        self.records.iter().find(|r| r.id == id).or_else(|| {
            self.edit_base
                .as_ref()
                .filter(|r| self.editing && r.id == id)
        })
    }

    fn shown_record(&self, r: &MemoryRecord) -> bool {
        let text = r.text.as_deref().unwrap_or_default();
        r.status != RecordStatus::Forgotten
            && !text.is_empty()
            && match self.tab {
                Tab::Kind(t) => r.record_type == t,
                _ => true,
            }
            && match self.scope {
                Scope::Active => r.status == RecordStatus::Active,
                Scope::NeedsReview => needs_review(r, self.today),
                Scope::Retired => {
                    matches!(r.status, RecordStatus::Retired | RecordStatus::Superseded)
                }
            }
            && self
                .project
                .as_ref()
                .is_none_or(|p| r.project.as_deref() == Some(p.as_str()))
            && self.matches_query(r)
            && self.matches_meeting(&r.sources)
    }
    fn matches_meeting(&self, sources: &[MemorySource]) -> bool {
        self.meeting.as_ref().is_none_or(|id| sources.iter().any(|s| matches!(s,MemorySource::Meeting{reference} if reference == &format!("[[{id}]]") || reference.starts_with(&format!("[[{id}@")))))
    }
    fn matches_query(&self, r: &MemoryRecord) -> bool {
        let q = self.search.text.trim().to_lowercase();
        q.is_empty()
            || r.text
                .as_deref()
                .is_some_and(|text| text.to_lowercase().contains(&q))
            || r.project
                .as_deref()
                .is_some_and(|p| self.projects.project_matches_query(p, &q))
            || r.people.iter().any(|p| p.to_lowercase().contains(&q))
    }

    /// The ids of the rows on screen, in order.
    pub fn shown_ids(&self) -> Vec<String> {
        if !self.migrated {
            return match (self.before, &self.preview) {
                (Before::Candidates, Some(v)) => {
                    v.candidates.iter().map(|c| c.record.id.clone()).collect()
                }
                _ => Vec::new(),
            };
        }
        if self.tab == Tab::Review {
            return self
                .proposals
                .iter()
                .filter(|p| self.matches_meeting(&p.proposal.sources))
                .map(|p| p.proposal.id.clone())
                .collect();
        }
        self.sorted_records()
            .iter()
            .filter(|r| !self.collapsed.contains(&r.record_type))
            .map(|r| r.id.clone())
            .collect()
    }

    /// Rows grouped by kind of claim, each group in the order of the file.
    fn sorted_records(&self) -> Vec<&MemoryRecord> {
        let mut v: Vec<&MemoryRecord> = self
            .records
            .iter()
            .filter(|r| self.shown_record(r))
            .collect();
        v.sort_by_key(|r| KINDS.iter().position(|k| *k == r.record_type));
        v
    }

    fn projects(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .records
            .iter()
            .filter(|r| r.status != RecordStatus::Forgotten)
            .filter_map(|r| r.project.clone())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    fn count_kind(&self, t: Option<RecordType>) -> usize {
        self.records
            .iter()
            .filter(|r| {
                r.status == RecordStatus::Active
                    && t.is_none_or(|t| r.record_type == t)
                    && self.matches_meeting(&r.sources)
            })
            .count()
    }
}

impl State {
    /// One reconcile-and-load worker, with at most one coalesced follow-up request.
    pub fn refresh_memory(&mut self, full: bool) {
        let Some(generation) = self.memory.begin_refresh(full) else {
            return;
        };
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        let result = std::thread::Builder::new()
            .name("memory-refresh".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| {
                    Snapshot::read(&MemoryStore::default(), full)
                })
                .map_err(|_| "The Memory update stopped. Reopen Memory or Diary to try again.".into())
                .and_then(|r| {
                    r.map_err(|_| {
                        "Could not update Memory. Previous data is kept. Reopen Memory or Diary to try again."
                            .into()
                    })
                });
                let _ = events.send(super::Event::MemoryRefreshed {
                    generation,
                    worker: std::thread::current().id(),
                    result: result.map(Box::new),
                });
                if let Some(waker) = waker {
                    waker();
                }
            });
        if result.is_err() {
            self.memory.refresh_pending = false;
            self.memory.refresh_error = Some(
                "Could not start the Memory update. Reopen Memory or Diary to try again.".into(),
            );
        }
    }

    pub fn memory_refreshed(&mut self, generation: u64, result: Result<Snapshot, String>) {
        if generation != self.memory.refresh_generation {
            return;
        }
        if !self.config.features.memory {
            self.memory.refresh_pending = false;
            self.memory.refresh_again = None;
            self.memory.prepare_after_refresh = false;
            return;
        }
        if let Some(full) =
            self.memory
                .finish_refresh(generation, sidenote::core::mutation_generation(), result)
        {
            self.refresh_memory(full);
        } else if self.memory.refresh_error.is_none()
            && std::mem::take(&mut self.memory.prepare_after_refresh)
            && self.on_memory()
        {
            self.migration_prepare(false);
        }
    }

    pub fn on_memory(&self) -> bool {
        self.memory_open && self.view.is_base() && self.selected.is_none()
    }

    pub fn open_memory(&mut self) {
        if !self.config.features.memory {
            return;
        }
        self.go_home();
        self.memory_open = true;
        if !self.memory.filters_loaded {
            let prefs = sidenote::core::UiPrefs::load(&self.store.root);
            self.memory.tab = if prefs.memory_pending {
                Tab::Review
            } else {
                prefs.memory_kind.map_or(Tab::All, Tab::Kind)
            };
            self.memory.scope = match prefs.memory_scope.as_str() {
                "review" => Scope::NeedsReview,
                "retired" => Scope::Retired,
                _ => Scope::Active,
            };
            self.memory.project = prefs.memory_project;
            self.memory.meeting = prefs.memory_meeting;
            self.memory.filters_loaded = true;
        }
        self.memory.browse = false;
        self.memory.settle_all = false;
        self.memory.settle_meeting = None;
        self.memory.panel = self.memory.editing;
        self.memory.prepare_after_refresh = true;
        self.refresh_memory(true);
    }

    fn save_memory_filters(&mut self) {
        let mut prefs = sidenote::core::UiPrefs::load(&self.store.root);
        prefs.memory_kind = match self.memory.tab {
            Tab::Kind(kind) => Some(kind),
            _ => None,
        };
        prefs.memory_pending = self.memory.tab == Tab::Review;
        prefs.memory_scope = match self.memory.scope {
            Scope::Active => "active",
            Scope::NeedsReview => "review",
            Scope::Retired => "retired",
        }
        .into();
        prefs.memory_project = self.memory.project.clone();
        prefs.memory_meeting = self.memory.meeting.clone();
        if prefs.save(&self.store.root).is_err() {
            self.memory.notice = Some("Could not remember filters on this PC.".into());
        }
    }

    pub fn open_record(&mut self, id: &str) {
        if self.memory.editing {
            self.memory.notice =
                Some("Your draft is kept. Save or cancel it before opening another claim.".into());
            return;
        }
        self.memory.editor_generation += 1;
        self.memory.cursor = Some(id.to_string());
        self.memory.panel = true;
        self.memory.editing = false;
        self.memory.ask = None;
        self.memory.edit = TextEdit::default();
    }

    pub fn close_record(&mut self) {
        if self.memory.editing {
            self.memory.notice =
                Some("Your draft is kept. Save or cancel it before closing.".into());
            return;
        }
        self.memory.editor_generation += 1;
        self.memory.panel = false;
        self.memory.editing = false;
        self.memory.ask = None;
        self.memory.edit = TextEdit::default();
    }

    fn memory_done(&mut self, key: &str, result: std::io::Result<()>) -> bool {
        match result {
            Ok(()) => {
                self.memory.errors.remove(key);
                self.refresh_memory(false);
                true
            }
            Err(e) => {
                self.memory.errors.insert(
                    key.to_string(),
                    sidenote::core::validation_message(&e)
                        .unwrap_or_else(|| super::edit_worker::SaveError::from(&e).message())
                        .into(),
                );
                false
            }
        }
    }

    pub fn memory_confirm(&mut self, id: &str) {
        let r = MemoryStore::default().confirm_record(id);
        self.record_saved(id, RecordWrite::Confirm, r);
    }

    pub fn memory_retire(&mut self, id: &str) {
        // Runtime fixtures can invoke this action without a native window turn.
        #[cfg(target_os = "linux")]
        let _budget = sidenote::core::UiLockBudget::begin();
        let r = MemoryStore::default().retire_record(id);
        if self.memory.cursor.as_deref() == Some(id) {
            self.memory.ask = None;
        }
        self.record_saved(id, RecordWrite::Retire, r);
    }

    pub fn memory_forget(&mut self, id: &str) {
        let r = MemoryStore::default().forget_record(id);
        let is_open = self.memory.panel && self.memory.cursor.as_deref() == Some(id);
        if is_open {
            self.memory.ask = None;
        }
        if self.record_saved(id, RecordWrite::Forget, r) && is_open {
            self.close_record();
        }
    }

    fn record_saved(&mut self, id: &str, write: RecordWrite, result: io::Result<()>) -> bool {
        if self.memory_done(id, result) {
            self.memory.record_retries.remove(id);
            true
        } else {
            self.memory.record_retries.insert(id.into(), write);
            false
        }
    }

    fn retry_record(&mut self, id: &str, tx: &mut Tx) {
        if !self.memory.errors.contains_key(id) {
            return;
        }
        match self.memory.record_retries.get(id).cloned() {
            Some(RecordWrite::Confirm) => self.memory_confirm(id),
            Some(RecordWrite::Retire) => self.memory_retire(id),
            Some(RecordWrite::Forget) => self.memory_forget(id),
            Some(RecordWrite::Undo(at)) => self.memory_undo(id, &at, tx),
            None if self.memory.editing && self.memory.cursor.as_deref() == Some(id) => {
                let text = self
                    .memory
                    .edit_sent
                    .clone()
                    .unwrap_or_else(|| self.memory.edit.text.clone());
                self.save_memory_text(text);
            }
            None => {}
        }
    }

    pub fn cancel_memory_edit(&mut self) {
        if let Some(id) = &self.memory.cursor {
            self.memory.errors.remove(id);
            self.memory.record_retries.remove(id);
        }
        self.memory.notice = None;
        self.memory.editor_generation += 1;
        self.memory.editing = false;
        self.memory.edit = TextEdit::default();
        self.memory.edit_base = None;
        self.memory.edit_sent = None;
    }
    pub fn memory_edit_start(&mut self, id: &str) {
        let Some(text) = self.memory.record(id).and_then(|r| r.text.clone()) else {
            return;
        };
        self.memory.editor_generation += 1;
        self.memory.edit_base = self.memory.record(id).cloned();
        self.memory.edit_sent = None;
        self.memory.edit = TextEdit::new(&text);
        self.memory.edit.focused = true;
        self.memory.editing = true;
        self.memory.ask = None;
    }

    pub fn memory_edit_save(&mut self) {
        self.save_memory_text(self.memory.edit.text.clone());
    }

    fn save_memory_text(&mut self, text: String) {
        let Some(id) = self.memory.cursor.clone() else {
            return;
        };
        if self.edit_busy {
            return;
        }
        self.memory.record_retries.remove(&id);
        if text.trim().is_empty() {
            self.memory.edit_sent = None;
            self.memory
                .errors
                .insert(id, "A claim needs some text.".into());
            return;
        }
        let Some(base) = self.memory.edit_base.clone() else {
            return;
        };
        debug_assert_eq!(self.memory.cursor.as_deref(), Some(base.id.as_str()));
        if self.memory.cursor.as_deref() != Some(base.id.as_str()) {
            return;
        }
        self.memory.edit_sent = Some(text.clone());
        self.publish_edit(super::edit_worker::Job::Memory {
            store: MemoryStore::default(),
            base: Box::new(base),
            text,
            session: self.memory.editor_generation,
        });
    }

    pub fn proposal_accept(&mut self, shown: &PendingProposal, choice: AcceptChoice) {
        let id = &shown.proposal.id;
        self.cancel_rewrite(id);
        let result = MemoryStore::default().accept_shown(shown, choice);
        if !self.memory_done(id, result) {
            self.refresh_memory(false);
        }
    }

    pub fn proposal_reject(&mut self, id: &str) {
        self.cancel_rewrite(id);
        let r = MemoryStore::default().reject_proposal(id);
        if !self.memory_done(id, r) {
            self.refresh_memory(false);
        }
    }
    pub fn memory_undo(&mut self, record: &str, at: &str, tx: &mut Tx) {
        if let Some(id) = self
            .memory
            .events
            .iter()
            .find(|e| e.record_id == record && e.at == at)
            .and_then(|e| e.proposal_id.clone())
        {
            self.cancel_rewrite(&id);
        }
        let result = MemoryStore::default().undo_event(record, at);
        if self.record_saved(record, RecordWrite::Undo(at.into()), result) {
            self.memory.result = None;
            self.show_toast("Change undone. The previous memory is restored.", tx);
        }
    }
    pub fn memory_comment_save(&mut self, id: &str) {
        self.cancel_rewrite(id);
        match MemoryStore::default().comment_proposal(id, &self.memory.comment.text) {
            Ok(request) => {
                self.memory_done(id, Ok(()));
                self.memory.comment_for = None;
                if let Some(request) = request {
                    self.start_rewrite(id, request);
                }
            }
            Err(error) => {
                self.memory_done(id, Err(error));
            }
        }
    }

    fn cancel_rewrite(&mut self, id: &str) {
        if let Some(worker) = self.memory.rewrites.get_mut(id) {
            worker.cancel.stop();
            worker.next = None;
        }
    }

    pub(super) fn stop_memory_rewrites(&mut self) {
        for worker in self.memory.rewrites.values_mut() {
            worker.cancel.stop();
            worker.next = None;
        }
    }

    fn start_rewrite(&mut self, id: &str, request: RevisionRequest) {
        if !self.config.features.memory {
            return;
        }
        if let Some(worker) = self.memory.rewrites.get_mut(id) {
            worker.next = Some(request);
            return;
        }
        let cancel = sidenote::core::ask::Cancel::new();
        self.memory.rewrites.insert(
            id.into(),
            Rewrite {
                cancel: cancel.clone(),
                next: None,
            },
        );
        let id = id.to_owned();
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        MemoryStore::default().spawn_rewrite(request, cancel, move |result| {
            let _ = events.send(super::Event::MemoryRewritten { id, result });
            if let Some(waker) = waker {
                waker();
            }
        });
    }

    pub(super) fn memory_rewritten(&mut self, id: &str, result: Result<bool, &'static str>) {
        let Some(worker) = self.memory.rewrites.remove(id) else {
            return;
        };
        if let Some(next) = worker.next {
            self.start_rewrite(id, next);
        } else if !worker.cancel.stopped()
            && let Err(error) = result
        {
            self.memory.errors.insert(id.into(), error.into());
        }
        self.refresh_memory(false);
    }

    pub fn migration_accept(&mut self) {
        if !self.config.features.memory || self.memory.preview_running {
            return;
        }
        let Some(fingerprint) = self
            .memory
            .preview
            .as_ref()
            .map(|p| p.source_fingerprint.clone())
        else {
            return;
        };
        let r = MemoryStore::default()
            .accept_migration(&fingerprint)
            .map(|_| ());
        if self.memory_done("migration", r) {
            self.memory.tab = Tab::All;
            self.close_record();
        }
    }

    pub fn migration_prepare(&mut self, force: bool) {
        if !self.config.features.memory
            || self.memory.migrated
            || self.memory.preview_running
            || (!force && self.memory.preview.as_ref().is_some_and(|p| p.prepared))
        {
            return;
        }
        self.memory.preview_running = true;
        self.memory.errors.remove("migration");
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let result = MemoryStore::default()
                .prepare_migration(force)
                .map(|_| ())
                .map_err(|e| super::edit_worker::SaveError::from(&e).message().into());
            let _ = events.send(super::Event::MigrationPrepared(result));
            if let Some(waker) = waker {
                waker();
            }
        });
    }
}

// ---- rows ----------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Lay {
    compact: bool,
    project: bool,
}

fn status_badge(s: RecordStatus, p: Palette) -> El {
    match s {
        RecordStatus::Active => badge("Active", p.ok, Some(p.okbg)),
        RecordStatus::Proposed => badge("Proposed", p.open, Some(p.chip)),
        RecordStatus::Superseded => badge("Replaced", p.faint, Some(p.chip)),
        RecordStatus::Retired => badge("Retired", p.faint, Some(p.chip)),
        RecordStatus::Forgotten => badge("Forgotten", p.faint, Some(p.chip)),
    }
}

fn row_shell(
    content: El,
    key: String,
    name: String,
    selected: bool,
    at_cursor: bool,
    id: String,
    p: Palette,
) -> El {
    let mut line = pressable(content)
        .key(key)
        .label(name)
        .w_full()
        .px(8.0)
        .radius(6.0)
        .hover(p.chip)
        .on_press(move |s: &mut State| s.open_record(&id));
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

fn record_row(m: &MemState, r: &MemoryRecord, width: f32, edit_busy: bool, p: Palette) -> El {
    let at_cursor = m.cursor.as_deref() == Some(r.id.as_str());
    let cursor = if m.panel && at_cursor {
        rows::Cursor::Selected
    } else if m.keyboard && at_cursor {
        rows::Cursor::Keyboard
    } else {
        rows::Cursor::Rest
    };
    let kind = if matches!(r.status, RecordStatus::Retired | RecordStatus::Superseded) {
        rows::Record::Settled
    } else if r.record_type == RecordType::Question {
        rows::Record::Question
    } else {
        rows::Record::Claim
    };
    let claim = plain(r.text.as_deref().unwrap_or_default());
    let mut details = Vec::new();
    details.extend(r.project.iter().cloned());
    details.push(confirmed_text(r));
    if r.sources.len() >= 2 {
        details.push(plural(r.sources.len(), "source", "sources"));
    }
    let id = r.id.clone();
    let save = if let Some(error) = m.errors.get(&id) {
        if m.editing && at_cursor && edit_busy && m.edit_sent.is_some() {
            rows::Save::Retrying
        } else {
            let retry_id = id.clone();
            rows::Save::RecordFailed(error, Rc::new(move |s, tx| s.retry_record(&retry_id, tx)))
        }
    } else {
        rows::Save::Idle
    };
    let data = rows::Row {
        title: &claim,
        cursor,
        interactive: true,
        save,
    };
    rows::record_row(data, &details.join(" · "), kind, width, p)
        .key(format!("mrow-{id}"))
        .label(format!("Open the claim: {claim}"))
        .on_press(move |s: &mut State| s.open_record(&id))
}

fn proposal_row(s: &State, pp: &PendingProposal, lay: Lay, p: Palette) -> El {
    let m = &s.memory;
    let pr = &pp.proposal;
    let selected = m.panel && m.cursor.as_deref() == Some(pr.id.as_str());
    let at_cursor = m.keyboard && m.cursor.as_deref() == Some(pr.id.as_str()) && !selected;
    let claim = plain(&pr.text);
    let project = pr.project.clone().unwrap_or_default();
    let sources = plural(pr.sources.len(), "source", "sources");
    let content = if lay.compact {
        row()
            .w_full()
            .h(54.0)
            .gap(10.0)
            .items_center()
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(roles::SMALL.medium().line(&claim, p.ink))
                    .child(roles::META.line(
                        format!(
                            "{} \u{b7} {sources}",
                            if project.is_empty() {
                                "No project"
                            } else {
                                &project
                            }
                        ),
                        p.faint,
                    )),
            )
            .child(badge(op_name(pr.op), p.ink, Some(p.chip)))
    } else {
        let mut cells = row()
            .w_full()
            .h(38.0)
            .gap(10.0)
            .items_center()
            .child(
                row()
                    .w(72.0)
                    .items_center()
                    .child(badge(op_name(pr.op), p.ink, Some(p.chip))),
            )
            .child(roles::SMALL.medium().line(&claim, p.ink).grow(1.0));
        if lay.project {
            cells = cells.child(
                row()
                    .w(150.0)
                    .items_center()
                    .gap(6.0)
                    .child(if project.is_empty() {
                        el()
                    } else {
                        dot(7.0, p.soft)
                    })
                    .child(roles::META.line(&project, p.faint)),
            );
        }
        cells
            .child(
                row()
                    .w(76.0)
                    .justify_end()
                    .items_center()
                    .child(roles::META.wrap(sources, p.faint)),
            )
            .child(
                row().w(64.0).justify_end().items_center().child(
                    roles::META.wrap(
                        parse_day(&pp.created_at)
                            .map_or(String::new(), |d| d.format("%-d %b").to_string()),
                        p.faint,
                    ),
                ),
            )
    };
    row_shell(
        content,
        format!("mprop-{}", pr.id),
        format!("Open the proposal: {claim}"),
        selected,
        at_cursor,
        pr.id.clone(),
        p,
    )
}

fn candidate_row(
    s: &State,
    c: &sidenote::core::memory::MigrationCandidate,
    lay: Lay,
    p: Palette,
) -> El {
    let m = &s.memory;
    let id = &c.record.id;
    let selected = m.panel && m.cursor.as_deref() == Some(id.as_str());
    let at_cursor = m.keyboard && m.cursor.as_deref() == Some(id.as_str()) && !selected;
    let claim = plain(c.record.text.as_deref().unwrap_or_default());
    let mut tags = row().gap(6.0).items_center();
    if c.ambiguous {
        tags = tags.child(badge("May hold more than one claim", p.open, Some(p.chip)));
    }
    if c.uncited {
        tags = tags.child(badge("No citation", p.warn, Some(p.warnbg)));
    }
    let line_no = roles::META.wrap(format!("Line {}", c.line), p.faint);
    let content = if lay.compact {
        row().w_full().h(54.0).gap(10.0).items_center().child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(roles::SMALL.medium().line(&claim, p.ink))
                .child(row().gap(8.0).items_center().child(line_no).child(tags)),
        )
    } else {
        row()
            .w_full()
            .h(38.0)
            .gap(10.0)
            .items_center()
            .child(roles::SMALL.medium().line(&claim, p.ink).grow(1.0))
            .child(row().items_center().child(tags))
            .child(row().w(56.0).justify_end().child(line_no))
    };
    row_shell(
        content,
        format!("mcand-{id}"),
        format!("Open the candidate: {claim}"),
        selected,
        at_cursor,
        id.clone(),
        p,
    )
}

fn group_head(name: &str, n: usize, p: Palette) -> El {
    row()
        .w_full()
        .h(32.0)
        .px(8.0)
        .gap(8.0)
        .items_center()
        .child(roles::GROUP.wrap(name, p.ink))
        .child(roles::META.medium().wrap(n.to_string(), p.faint))
}

// ---- the top of the page -------------------------------------------------------------------

fn migration_banner(s: &State, narrow: bool, p: Palette) -> El {
    let m = &s.memory;
    let Some(v) = &m.preview else {
        return el();
    };
    let mut counts = row()
        .flex_wrap()
        .gaps(6.0, 6.0)
        .child(badge(
            format!("{} to import", v.imported),
            p.ink,
            Some(p.chip),
        ))
        .child(badge(
            format!("{} with no citation", v.uncited),
            if v.uncited > 0 { p.warn } else { p.soft },
            Some(if v.uncited > 0 { p.warnbg } else { p.chip }),
        ))
        .child(badge(
            format!("{} may hold more than one claim", v.ambiguous),
            if v.ambiguous > 0 { p.open } else { p.soft },
            Some(p.chip),
        ));
    for kind in KINDS {
        let n = v
            .candidates
            .iter()
            .filter(|c| c.record.record_type == kind)
            .count();
        counts = counts.child(badge(
            format!("{} {n}", kind_label(kind)),
            p.soft,
            Some(p.chip),
        ));
    }
    let skim = button(
        if m.before == Before::Candidates {
            "Show the old text"
        } else {
            "Skim the claims"
        },
        "mig-skim",
        false,
        p,
    )
    .label("Switch between the old text and the claims that would be imported")
    .on_press(|s: &mut State| {
        s.memory.before = if s.memory.before == Before::Candidates {
            Before::Text
        } else {
            Before::Candidates
        };
        s.close_record();
    });
    let accept = button(
        if m.preview_running {
            "Preparing claims..."
        } else {
            "Accept migration"
        },
        "mig-accept",
        true,
        p,
    )
    .label("Accept the migration")
    .on_press(|s: &mut State| s.migration_accept());
    let mut card = col()
        .key("mem-banner")
        .w_full()
        .gap(10.0)
        .px(if narrow { 12.0 } else { 16.0 })
        .py(14.0)
        .radius(12.0)
        .fill(p.warnbg)
        .child(roles::TITLE.wrap("Memory has not moved to the new format yet", p.ink))
        .child(roles::META.wrap("The new format keeps one claim per row, with its sources, so you can confirm, edit or retire each one. Nothing changes until you accept. Your current text stays exactly as it is in company.legacy.md.", p.soft))
        .child(counts)
        .child(
            row()
                .flex_wrap()
                .gaps(8.0, 8.0)
                .items_center()
                .child(accept)
                .child(skim)
                .child(button("Run preview again", "mig-rerun", false, p)
                    .on_press(|s: &mut State| s.migration_prepare(true))),
        );
    if m.preview_running {
        card = card.child(roles::META.wrap(
            "Splitting and typing the claims in the background. You can keep using Meetings.",
            p.soft,
        ));
    }
    for (title, issues) in [
        ("Unplaced text", &v.unplaced),
        ("Kept the original paragraph", &v.fallbacks),
    ] {
        for issue in issues {
            card = card.child(roles::META.wrap(
                format!(
                    "{title}, line {}: {}\n{}",
                    issue.line, issue.reason, issue.text
                ),
                p.warn,
            ));
        }
    }
    if let Some(e) = m.errors.get("migration") {
        card = card.child(roles::SMALL.medium().wrap(e, p.warn).role("alert"));
    }
    card
}

fn search_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        s.memory.search = TextEdit::default();
        tx.blur();
        return true;
    }
    false
}

fn empty_note(head: &str, body: &str, p: Palette) -> El {
    col()
        .w_full()
        .gap(6.0)
        .pt(36.0)
        .items_center()
        .child(roles::SECTION.wrap(head, p.ink))
        .child(roles::SMALL.wrap(body, p.soft))
}

fn record_list(s: &State, width: f32, p: Palette) -> El {
    let m = &s.memory;
    let recs = m.sorted_records();
    if recs.is_empty() {
        let (head, body) = if m.records.is_empty() {
            (
                "No memory yet",
                "Claims appear here after the daily check proposes them and you approve.",
            )
        } else if m.scope == Scope::NeedsReview {
            (
                "Nothing to review",
                "Every active claim was confirmed in the last 90 days.",
            )
        } else {
            (
                "Nothing matches",
                "Try another tab, another filter or a different search.",
            )
        };
        return empty_note(head, body, p);
    }
    let mut out = col().w_full();
    for kind in KINDS {
        let of: Vec<&&MemoryRecord> = recs.iter().filter(|r| r.record_type == kind).collect();
        if of.is_empty() {
            continue;
        }
        let mut g = col().w_full().mt(roles::GROUP_TOP).gap(8.0).child(
            super::widgets::button(
                format!(
                    "{} {} {}",
                    if m.collapsed.contains(&kind) {
                        "\u{203a}"
                    } else {
                        "\u{25be}"
                    },
                    kind_label(kind),
                    of.len()
                ),
                &format!("memory-collapse-{kind:?}"),
                super::widgets::ButtonRole::Ghost,
                p,
            )
            .h(28.0)
            .self_start()
            .expanded(!m.collapsed.contains(&kind))
            .on_press(move |s: &mut State| {
                if s.memory.collapsed.contains(&kind) {
                    s.memory.collapsed.retain(|value| *value != kind);
                } else {
                    s.memory.collapsed.push(kind);
                }
            }),
        );
        if m.collapsed.contains(&kind) {
            out = out.child(g);
            continue;
        }
        let mut rows = col().w_full().key(format!("memory-group-{kind:?}"));
        for (index, r) in of.into_iter().enumerate() {
            if index > 0 {
                rows = rows.child(rows::Density::Rail.separator(width, p));
            }
            rows = rows.child(record_row(m, r, width, s.edit_busy, p));
        }
        g = g.child(rows);
        out = out.child(g);
    }
    out
}

fn proposal_list(s: &State, lay: Lay, p: Palette) -> El {
    let m = &s.memory;
    let proposals: Vec<_> = m
        .proposals
        .iter()
        .filter(|p| m.matches_meeting(&p.proposal.sources))
        .collect();
    if proposals.is_empty() {
        return empty_note(
            "No proposals waiting",
            "When the daily check suggests a change to memory, it waits here for you to accept or reject it.",
            p,
        );
    }
    let mut g = col().w_full().mt(10.0).gap(1.0);
    if !m.migrated {
        g = g.child(
            roles::SMALL.medium().wrap("Accept the migration first. Proposals cannot be accepted until memory is in the new format.", p.warn)
            .mb(6.0),
        );
    }
    for pp in proposals {
        g = g.child(proposal_row(s, pp, lay, p));
    }
    g.child(
        roles::META
            .wrap(
                "Nothing changes until you accept. Rejecting leaves memory as it is.",
                p.faint,
            )
            .mt(10.0),
    )
}

fn candidate_list(s: &State, lay: Lay, p: Palette) -> El {
    let Some(v) = &s.memory.preview else {
        return empty_note("Nothing to import", "There is no legacy memory to move.", p);
    };
    let mut out = col().w_full();
    let mut groups: Vec<(String, Vec<&sidenote::core::memory::MigrationCandidate>)> = Vec::new();
    for c in &v.candidates {
        let name = c.record.project.clone().unwrap_or_else(|| "Company".into());
        match groups.iter_mut().find(|(n, _)| *n == name) {
            Some((_, v)) => v.push(c),
            None => groups.push((name, vec![c])),
        }
    }
    for (name, of) in groups {
        let mut g = col()
            .w_full()
            .mt(roles::GROUP_TOP)
            .gap(1.0)
            .child(group_head(&name, of.len(), p));
        for c in of {
            g = g.child(candidate_row(s, c, lay, p));
        }
        out = out.child(g);
    }
    out
}

fn list_column(
    s: &State,
    k: &Kit,
    p: Palette,
    narrow: bool,
    list_w: f32,
    layer: &mut Vec<El>,
) -> El {
    let m = &s.memory;
    if m.settle_all {
        return settled::all(s, p, narrow);
    }
    if !m.browse {
        return landing::column(s, k, p, narrow);
    }
    let side = if narrow {
        16.0
    } else if m.panel {
        24.0
    } else {
        40.0
    };
    let lay = Lay {
        compact: narrow || list_w < 700.0,
        project: list_w >= 800.0,
    };
    let mut doc = col()
        .key("memory-top")
        .w_full()
        .max_w(920.0)
        .px(side)
        .pt(if narrow { 8.0 } else { 18.0 })
        .pb(56.0)
        .child(text_title("Memory", if narrow { 26.0 } else { 30.0 }, p).mb(4.0))
        .mb(14.0);
    if let Some(id) = &m.meeting {
        let title = s
            .meetings
            .iter()
            .find(|meeting| &meeting.id == id)
            .map_or(id.as_str(), |meeting| meeting.title.as_str());
        doc = doc.child(roles::META.wrap(format!("From {title}"), p.soft).mb(10.0));
    }
    if !m.migrated {
        doc = doc.child(migration_banner(s, narrow, p).mb(14.0));
        match m.before {
            Before::Text => {
                doc = doc.child(legacy_text(s, k, p));
            }
            Before::Candidates => {
                doc = doc.child(candidate_list(s, lay, p));
            }
        }
    } else {
        doc = doc.child(landing::toolbar(s, k, p, layer).mb(6.0));
        doc = doc.child(if m.tab == Tab::Review {
            proposal_list(s, lay, p)
        } else {
            record_list(s, list_w.min(920.0) - 2.0 * side, p)
        });
        let forgotten = m
            .records
            .iter()
            .filter(|r| r.status == RecordStatus::Forgotten)
            .count();
        if forgotten > 0 && m.tab != Tab::Review {
            doc = doc.child(
                roles::META.wrap(format!(
                        "{} forgotten. Their text is gone from memory, and the same sources will not recreate them.",
                        plural(forgotten, "claim is", "claims are")
                    ), p.faint)
                .mt(16.0),
            );
        }
    }
    col().grow(1.0).min_w(0.0).min_h(0.0).child(scroll(
        col().w_full().items_center().child(doc),
        "memory-scroll",
    ))
}

fn legacy_text(s: &State, k: &Kit, p: Palette) -> El {
    let m = &s.memory;
    let mut c = col()
        .key("mem-legacy")
        .w_full()
        .gap(6.0)
        .child(spaced_caps("Current memory text", p));
    if m.legacy.trim().is_empty() {
        return c.child(roles::SMALL.wrap(
            "There is no company memory yet. New memory needs owner approval.",
            p.soft,
        ));
    }
    c = c.child(prose(s, k, p, &m.legacy, "mem-legacy", 15.0));
    c
}

// ---- the panel -----------------------------------------------------------------------------

fn source_block(s: &State, sources: &[MemorySource], key: &str, p: Palette) -> El {
    let mut c = col()
        .w_full()
        .gap(2.0)
        .child(spaced_caps("Sources", p).mb(2.0));
    if sources.is_empty() {
        c = c.child(roles::META.wrap("No source recorded.", p.faint));
    }
    c = c.children(source_rows(
        s,
        sources.iter().filter_map(|src| match src {
            MemorySource::Meeting { reference } => Some(reference.as_str()),
            MemorySource::Kb { .. } => None,
        }),
        &format!("{key}-src"),
        p,
    ));
    for (n, src) in sources.iter().enumerate() {
        match src {
            MemorySource::Meeting { .. } => {}
            MemorySource::Kb { path, revision } => {
                c = c.child(
                    col()
                        .w_full()
                        .gap(0.0)
                        .child(copy_row(s, path, format!("{key}-kb-{n}"), p))
                        .child(roles::META.wrap(
                            match revision {
                                Some(r) => {
                                    format!("KB revision {}", r.chars().take(7).collect::<String>())
                                }
                                None => "KB revision not recorded".to_string(),
                            },
                            p.faint,
                        )),
                );
            }
        }
    }
    c.child(
        roles::META
            .wrap(
                "A source shows where a claim came from. It does not prove the claim.",
                p.faint,
            )
            .mt(4.0),
    )
}

fn event_name(op: &str) -> String {
    match op {
        "accept_migration" => "Imported from the old memory text".into(),
        "accept_proposal" => "Accepted a proposal from the daily check".into(),
        "reject_proposal" => "Rejected a proposal".into(),
        "edit" => "Edited by you".into(),
        "confirm" => "Confirmed by you".into(),
        "retire" => "Retired by you".into(),
        "forget" => "Forgotten".into(),
        other => {
            let mut t = other.replace('_', " ");
            if let Some(f) = t.get(..1) {
                t = format!("{}{}", f.to_uppercase(), &t[1..]);
            }
            t
        }
    }
}

fn history_block(s: &State, r: &MemoryRecord, p: Palette) -> El {
    let m = &s.memory;
    let mut c = col()
        .w_full()
        .gap(3.0)
        .child(spaced_caps("History", p).mb(2.0));
    let mut lines: Vec<(String, String)> = m
        .history
        .iter()
        .filter(|e| {
            e.record_id == r.id || (e.record_id == "migration" && r.id.starts_with("legacy-"))
        })
        .map(|e| {
            let mut label = event_name(&e.operation);
            if let Some(text) = e.decision.as_ref().and_then(|d| d.text.as_deref()) {
                label.push('\n');
                label.push_str(text);
            }
            if let Some(comment_label) = e.comment_label {
                label.push('\n');
                label.push_str(comment_label);
            }
            (day_text(&e.at), label)
        })
        .collect();
    if let Some(prev) = r.supersedes.as_ref().and_then(|id| m.record(id)) {
        lines.push((
            day_text(&prev.updated_at),
            format!(
                "Replaces: {}",
                plain(prev.text.as_deref().unwrap_or("an earlier claim"))
            ),
        ));
    }
    if let Some(next) = m
        .records
        .iter()
        .find(|x| x.supersedes.as_deref() == Some(r.id.as_str()))
    {
        lines.push((
            day_text(&next.updated_at),
            format!(
                "Replaced by: {}",
                plain(next.text.as_deref().unwrap_or_default())
            ),
        ));
    }
    if lines.is_empty() {
        c = c.child(roles::META.wrap("No changes recorded since it was added.", p.faint));
    }
    for (when, what) in lines.into_iter().rev() {
        c = c.child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(roles::META.medium().wrap(when, p.faint).w(78.0))
                .child(roles::META.wrap(what, p.soft).grow(1.0).min_w(0.0)),
        );
    }
    c
}

fn edit_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        s.close_record();
        tx.blur();
        return true;
    }
    false
}

fn record_panel(s: &State, k: &Kit, r: &MemoryRecord, overlay: bool, p: Palette) -> El {
    let m = &s.memory;
    let id = r.id.clone();
    let text_ = r.text.clone().unwrap_or_default();
    let review = needs_review(r, m.today);
    let badges = row()
        .flex_wrap()
        .gaps(6.0, 6.0)
        .items_center()
        .child(badge(kind_one(r.record_type), p.soft, Some(p.chip)))
        .child(status_badge(r.status, p));
    let mut body = col()
        .w_full()
        .gap(16.0)
        .px(20.0)
        .pt(4.0)
        .pb(24.0)
        .child(badges);
    if m.editing {
        body = body.child(
            TextArea::new(&m.edit)
                .label("Claim text")
                .auto_grow(5, 14)
                .on_edit(|s: &mut State| &mut s.memory.edit)
                .on_key(edit_key)
                .build(k)
                .key("mem-edit")
                .min_w(0.0),
        );
    } else {
        body = body.child(prose(s, k, p, &text_, &format!("mrow-{}", r.id), 15.0));
    }
    let mut props = col().w_full().gap(6.0);
    if let Some(pr) = &r.project {
        props = props.child(prop_row(
            "Project",
            row()
                .gap(8.0)
                .items_center()
                .min_h(24.0)
                .child(dot(7.0, p.soft))
                .child(roles::CONTROL.wrap(pr, p.ink)),
            p,
        ));
    }
    if !r.people.is_empty() {
        props = props.child(prop_row(
            "People",
            roles::SMALL.wrap(r.people.join(", "), p.ink).mt(2.0),
            p,
        ));
    }
    props = props
        .child(prop_row(
            "Basis",
            roles::SMALL.wrap(basis_name(r.basis), p.ink).mt(2.0),
            p,
        ))
        .child(prop_row(
            "First seen",
            roles::META
                .wrap(
                    r.first_seen
                        .as_deref()
                        .map_or_else(|| "Not recorded".to_string(), day_text),
                    if r.first_seen.is_some() {
                        p.ink
                    } else {
                        p.faint
                    },
                )
                .mt(2.0),
            p,
        ))
        .child(prop_row(
            "Confirmed",
            if review { roles::NAME } else { roles::SMALL }
                .wrap(
                    r.last_confirmed
                        .as_deref()
                        .map_or_else(|| "Not confirmed yet".to_string(), day_text),
                    if review { p.open } else { p.ink },
                )
                .mt(2.0),
            p,
        ));
    body = body.child(props);
    if review && r.status == RecordStatus::Active {
        body = body.child(roles::META.wrap("Suggested for review: it has not been confirmed in the last 90 days. Age does not make a claim wrong.", p.soft));
    }
    body = body
        .child(hairline(p))
        .child(source_block(s, &r.sources, &format!("mp-{}", r.id), p))
        .child(hairline(p))
        .child(history_block(s, r, p));
    if let Some(e) = m.errors.get(&r.id) {
        let retry_id = id.clone();
        body = body
            .child(roles::SMALL.medium().wrap(e, p.warn).role("alert"))
            .child(
                super::widgets::notice_action("Try again", p)
                    .key("mem-save-retry")
                    .self_start()
                    .on_press_tx(move |s: &mut State, tx| s.retry_record(&retry_id, tx)),
            );
    }

    let mut foot = col().w_full().px(16.0).py(12.0).gap(8.0);
    match m.ask {
        Some(Ask::Forget) => {
            let kk = id.clone();
            foot = foot
                .child(roles::SMALL.bold().wrap("Forget this claim?", p.ink))
                .child(roles::META.wrap("Its text is removed from memory, and the same sources will not bring it back. Old daily check reports, the legacy backup and your meetings may still contain it.", p.soft))
                .child(
                    row()
                        .gap(8.0)
                        .items_center()
                        .child(
                            pressable(roles::CONTROL.wrap("Forget it", p.card))
                                .key("mem-forget-go")
                                .px(14.0)
                                .py(6.0)
                                .radius(8.0)
                                .fill(p.warn)
                                .on_press(move |s: &mut State| s.memory_forget(&kk)),
                        )
                        .child(button("Cancel", "mem-ask-cancel", false, p).on_press(|s: &mut State| s.memory.ask = None)),
                );
        }
        Some(Ask::Retire) => {
            let kk = id.clone();
            foot = foot
                .child(roles::SMALL.bold().wrap("Retire this claim?", p.ink))
                .child(roles::META.wrap("It leaves the active list and the memory the daily check reads. Its text stays here as history, under Retired.", p.soft))
                .child(
                    row()
                        .gap(8.0)
                        .items_center()
                        .child(button("Retire", "mem-retire-go", true, p).on_press(move |s: &mut State| s.memory_retire(&kk)))
                        .child(button("Cancel", "mem-ask-cancel", false, p).on_press(|s: &mut State| s.memory.ask = None)),
                );
        }
        None if m.editing => {
            foot = foot.child(
                row()
                    .gap(8.0)
                    .items_center()
                    .child(
                        button("Save", "mem-edit-save", true, p)
                            .on_press(|s: &mut State| s.memory_edit_save()),
                    )
                    .child(button("Cancel", "mem-edit-cancel", false, p).on_press_tx(
                        |s: &mut State, tx| {
                            s.cancel_memory_edit();
                            tx.blur();
                        },
                    ))
                    .child(
                        roles::META
                            .wrap(
                                "Saving counts as your confirmation of the new wording.",
                                p.faint,
                            )
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
        }
        None => {
            let mut buttons = row().w_full().gap(8.0).items_center().flex_wrap();
            if r.status == RecordStatus::Active {
                let (a, b) = (id.clone(), id.clone());
                buttons = buttons
                    .child(
                        button("Confirm", "mem-confirm", true, p)
                            .label("Confirm that this is still true")
                            .on_press(move |s: &mut State| s.memory_confirm(&a)),
                    )
                    .child(
                        button("Edit", "mem-edit", false, p)
                            .on_press(move |s: &mut State| s.memory_edit_start(&b)),
                    )
                    .child(
                        button("Retire", "mem-retire", false, p)
                            .on_press(|s: &mut State| s.memory.ask = Some(Ask::Retire)),
                    );
            } else {
                buttons = buttons.child(roles::META.wrap(
                    format!("{}. Its text stays here as history.", status_name(r.status)),
                    p.soft,
                ));
            }
            buttons = buttons.child(el().grow(1.0)).child(
                pressable(roles::SMALL.medium().wrap("Forget", p.warn))
                    .key("mem-forget")
                    .label("Forget this claim")
                    .px(10.0)
                    .py(6.0)
                    .radius(8.0)
                    .hover(p.warnbg)
                    .on_press(|s: &mut State| s.memory.ask = Some(Ask::Forget)),
            );
            foot = foot.child(buttons);
        }
    }
    panel_frame(
        "memory-panel",
        panel_top("Claim", "Memory", overlay, |s| s.close_record(), p),
        body,
        foot,
        p,
    )
}

fn before_after_block(before: Option<(&str, String)>, after: (&str, String), p: Palette) -> El {
    let mut c = col().w_full().gap(8.0);
    if let Some((name, text_)) = before {
        c = c.child(
            col()
                .w_full()
                .gap(4.0)
                .px(12.0)
                .py(10.0)
                .radius(10.0)
                .fill(p.side)
                .child(spaced_caps(name, p))
                .child(roles::SMALL.wrap(text_, p.soft).selectable(true)),
        );
    }
    c.child(
        col()
            .w_full()
            .gap(4.0)
            .px(12.0)
            .py(10.0)
            .radius(10.0)
            .fill(p.okbg)
            .child(spaced_caps(after.0, p))
            .child(roles::CONTROL.wrap(after.1, p.ink).selectable(true)),
    )
}

fn unapplied_comment(pp: &PendingProposal) -> bool {
    pp.can_rewrite() && pp.comment.is_some() && pp.revision_text().is_none()
}

fn original_button(m: &MemState, pp: &PendingProposal, key: String, p: Palette) -> Option<El> {
    (pp.can_rewrite() && pp.comment.is_some()).then(|| {
        let shown = pp.clone();
        button("Accept original wording", key, false, p)
            .button_disabled(!m.migrated)
            .on_press(move |s: &mut State| s.proposal_accept(&shown, AcceptChoice::Original))
    })
}

fn proposal_comment(s: &State, pp: &PendingProposal, p: Palette) -> Option<El> {
    let comment = pp.comment.as_deref()?;
    let id = &pp.proposal.id;
    let mut copy = col().w_full().gap(4.0);
    if let Some(text) = pp.revision_text() {
        let open = s.memory.earlier_wording.as_deref() == Some(id);
        let id = id.clone();
        copy = copy
            .child(roles::NAME.wrap("New wording ready", p.ink))
            .child(roles::SMALL.wrap(text, p.ink).selectable(true))
            .child(roles::SMALL.wrap(format!("Your comment: {comment}"), p.soft))
            .child(
                row().child(
                    button(
                        if open {
                            "Hide earlier wording"
                        } else {
                            "Show earlier wording"
                        },
                        format!("memory-earlier-{id}"),
                        false,
                        p,
                    )
                    .on_press(move |s: &mut State| {
                        s.memory.earlier_wording = (!open).then(|| id.clone());
                    }),
                ),
            )
            .children(open.then(|| roles::SMALL.wrap(&pp.proposal.text, p.soft)));
    } else if pp.can_rewrite() {
        let working = s
            .memory
            .rewrites
            .get(id)
            .is_some_and(|w| !w.cancel.stopped() || w.next.is_some());
        let stop_id = id.clone();
        let retry_id = id.clone();
        let request = pp.revision_request();
        copy = copy
            .child(roles::SMALL.wrap(format!("Your comment: {comment}"), p.soft))
            .child(roles::NAME.wrap(
                if working {
                    "Rewriting"
                } else {
                    "Rewrite failed"
                },
                p.ink,
            ))
            .child(roles::SMALL.wrap(
                "Wait for new wording or accept the original wording.",
                p.soft,
            ))
            .child(if working {
                button("Stop", format!("memory-stop-{id}"), false, p)
                    .on_press(move |s: &mut State| s.cancel_rewrite(&stop_id))
            } else {
                button("Try again", format!("memory-retry-{id}"), false, p).on_press(
                    move |s: &mut State| {
                        if let Some(request) = request.clone() {
                            s.start_rewrite(&retry_id, request);
                        }
                    },
                )
            });
    } else {
        copy = copy.child(roles::SMALL.wrap(format!("Your note: {comment}"), p.soft));
    }
    Some(copy)
}

fn proposal_panel(s: &State, pp: &PendingProposal, overlay: bool, p: Palette) -> El {
    let m = &s.memory;
    let pr = &pp.proposal;
    let target = pr.target_id.as_deref().and_then(|id| m.record(id));
    let target_text = target.and_then(|t| t.text.clone()).unwrap_or_default();
    let title = match pr.op {
        ProposalOp::Add => "Add a new claim",
        ProposalOp::Confirm => "Confirm an existing claim",
        ProposalOp::Supersede => "Replace a claim",
        ProposalOp::Retire => "Retire a claim",
    };
    let diff = match (pr.op, pp.revision_text()) {
        (ProposalOp::Add | ProposalOp::Supersede, Some(_)) => el(),
        (ProposalOp::Add, _) => before_after_block(None, ("New claim", pr.text.clone()), p),
        (ProposalOp::Confirm, _) => before_after_block(
            Some(("Claim now", target_text.clone())),
            (
                "After",
                format!(
                    "Same claim, confirmed {}, with the new source below.",
                    m.today.format("%-d %b %Y")
                ),
            ),
            p,
        ),
        (ProposalOp::Supersede, _) => before_after_block(
            Some(("Claim now", target_text.clone())),
            ("Replaced by", pr.text.clone()),
            p,
        ),
        (ProposalOp::Retire, _) => before_after_block(
            Some(("Claim now", target_text.clone())),
            ("After", "Retired. The text stays in history.".to_string()),
            p,
        ),
    };
    let mut body = col()
        .w_full()
        .gap(16.0)
        .px(20.0)
        .pt(4.0)
        .pb(24.0)
        .child(roles::SECTION.wrap(title, p.ink).w_full())
        .child(
            row()
                .flex_wrap()
                .gaps(6.0, 6.0)
                .child(badge(op_name(pr.op), p.ink, Some(p.chip)))
                .child(badge(kind_one(pr.record_type), p.soft, Some(p.chip)))
                .child(match &pr.project {
                    Some(n) => badge(n.clone(), p.soft, Some(p.chip)),
                    None => el(),
                }),
        )
        .child(diff)
        .children(proposal_comment(s, pp, p))
        .child(
            col()
                .w_full()
                .gap(4.0)
                .child(spaced_caps("Why the check suggests it", p))
                .child(roles::SMALL.wrap(&pr.rationale, p.ink)),
        );
    let stale = target.is_some_and(|t| !pr.matches_target_version(t));
    if pr.op != ProposalOp::Add && target.is_none() {
        body = body.child(roles::SMALL.medium().wrap(
            "The claim this refers to is gone or no longer active. Accepting will refuse.",
            p.warn,
        ));
    } else if stale {
        body = body.child(roles::SMALL.medium().wrap("This claim changed after the check suggested the update. Accepting will refuse. Reject it and let the next check look again.", p.warn));
    }
    body = body
        .child(hairline(p))
        .child(source_block(s, &pr.sources, &format!("pp-{}", pr.id), p));
    if let Some(e) = m.errors.get(&pr.id) {
        body = body.child(roles::SMALL.medium().wrap(e, p.warn).role("alert"));
    }
    let id2 = pr.id.clone();
    let mut foot = col().w_full().px(16.0).py(12.0).gap(8.0);
    if !m.migrated {
        foot = foot.child(
            roles::META
                .medium()
                .wrap("Accept the migration first, then accept this.", p.warn),
        );
    }
    let shown = pp.clone();
    let accept = if m.migrated && !unapplied_comment(pp) {
        button("Accept", "prop-accept", true, p)
            .on_press(move |s: &mut State| s.proposal_accept(&shown, AcceptChoice::Shown))
    } else {
        button("Accept", "prop-accept", false, p).dim(0.5)
    };
    foot = foot.child(
        row()
            .flex_wrap()
            .gap(8.0)
            .items_center()
            .child(accept.label("Accept this change to memory"))
            .children(original_button(m, pp, "prop-accept-original".into(), p))
            .child(
                button("Reject", "prop-reject", false, p)
                    .label("Reject this change")
                    .on_press(move |s: &mut State| s.proposal_reject(&id2)),
            )
            .child(
                roles::META
                    .wrap("Nothing changes until you accept.", p.faint)
                    .grow(1.0)
                    .min_w(0.0),
            ),
    );
    panel_frame(
        "memory-panel",
        panel_top("Proposal", "Memory", overlay, |s| s.close_record(), p),
        body,
        foot,
        p,
    )
}

fn candidate_panel(
    s: &State,
    k: &Kit,
    c: &sidenote::core::memory::MigrationCandidate,
    overlay: bool,
    p: Palette,
) -> El {
    let r = &c.record;
    let mut body = col()
        .w_full()
        .gap(16.0)
        .px(20.0)
        .pt(4.0)
        .pb(24.0)
        .child(
            row()
                .flex_wrap()
                .gaps(6.0, 6.0)
                .child(badge("Candidate", p.ink, Some(p.chip)))
                .child(badge(kind_one(r.record_type), p.soft, Some(p.chip)))
                .child(if c.ambiguous {
                    badge("May hold more than one claim", p.open, Some(p.chip))
                } else {
                    el()
                })
                .child(if c.uncited {
                    badge("No citation", p.warn, Some(p.warnbg))
                } else {
                    el()
                }),
        )
        .child(prose(
            s,
            k,
            p,
            r.text.as_deref().unwrap_or_default(),
            &format!("mcand-{}", r.id),
            15.0,
        ));
    let mut props = col().w_full().gap(6.0);
    if let Some(n) = &r.project {
        props = props.child(prop_row(
            "Group",
            row()
                .gap(8.0)
                .items_center()
                .min_h(24.0)
                .child(dot(7.0, p.soft))
                .child(roles::CONTROL.wrap(n, p.ink)),
            p,
        ));
    }
    props = props.child(prop_row(
        "Old text",
        roles::SMALL
            .wrap(format!("Line {} of company.md", c.line), p.ink)
            .mt(2.0),
        p,
    ));
    body = body
        .child(props)
        .child(hairline(p))
        .child(source_block(s, &r.sources, &format!("mc-{}", r.id), p))
        .child(roles::META.wrap("Imported dates stay unknown unless the old text states them. Approving the migration does not say each claim is freshly confirmed.", p.faint));
    let foot = row().w_full().px(16.0).py(12.0).child(roles::META.wrap(
        "This is a candidate. Nothing is saved until you accept the migration.",
        p.soft,
    ));
    panel_frame(
        "memory-panel",
        panel_top("Candidate", "Memory", overlay, |s| s.close_record(), p),
        body,
        foot,
        p,
    )
}

fn panel(s: &State, k: &Kit, overlay: bool, p: Palette) -> Option<El> {
    let m = &s.memory;
    if !m.panel {
        return None;
    }
    let id = m.cursor.as_deref()?;
    if !m.migrated {
        let c = m
            .preview
            .as_ref()?
            .candidates
            .iter()
            .find(|c| c.record.id == id)?;
        return Some(candidate_panel(s, k, c, overlay, p));
    }
    if m.tab == Tab::Review {
        let pp = m.proposals.iter().find(|x| x.proposal.id == id)?;
        return Some(proposal_panel(s, pp, overlay, p));
    }
    let r = m.record(id)?;
    Some(record_panel(s, k, r, overlay, p))
}

pub(super) fn refresh_status(m: &MemState, p: Palette) -> El {
    if m.refresh_pending {
        roles::META.wrap("Updating...", p.faint).role("status")
    } else if let Some(error) = &m.refresh_error {
        roles::SMALL.wrap(error, p.warn).role("alert")
    } else {
        el()
    }
}

pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let m = &s.memory;
    let probe = split_geometry(s, narrow, true);
    let shown = panel(s, k, probe.2, p);
    let (list_w, panel_w, overlay) = split_geometry(s, narrow, shown.is_some());
    let mut pane = col()
        .key("memory-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .fill(p.bg)
        .child(super::diary::return_link(s, p))
        .child(refresh_status(m, p).px(16.0));
    if let Some(note) = m.notice.as_ref().or(s.notice.as_ref()) {
        pane = pane.child(notice_bar(note, p));
    }
    if let Some(result) = &m.result {
        pane = pane.child(
            row()
                .w_full()
                .px(24.0)
                .py(8.0)
                .gap(12.0)
                .items_center()
                .fill(p.chip)
                .key("memory-result")
                .role("status")
                .child(roles::CONTROL.wrap(result, p.soft).grow(1.0))
                .child(
                    button("Dismiss", "memory-result-dismiss", false, p)
                        .on_press(|s: &mut State| s.memory.result = None),
                ),
        );
    }
    pane.child(list_and_panel(
        list_column(s, k, p, narrow, list_w, layer),
        shown,
        panel_w,
        overlay,
        p,
    ))
    .child(super::ask::overlay(s, k, p, narrow, layer))
}

// ---- the keyboard --------------------------------------------------------------------------

fn move_cursor(s: &mut State, delta: isize, tx: &mut Tx) {
    let ids = s.memory.shown_ids();
    if ids.is_empty() {
        return;
    }
    let at = s
        .memory
        .cursor
        .as_ref()
        .and_then(|c| ids.iter().position(|i| i == c));
    let next = match at {
        Some(i) if s.memory.keyboard => {
            (i as isize + delta).clamp(0, ids.len() as isize - 1) as usize
        }
        Some(i) => i,
        None if delta < 0 => ids.len() - 1,
        None => 0,
    };
    s.memory.keyboard = true;
    let id = ids[next].clone();
    let prefix = if !s.memory.migrated {
        "mcand"
    } else if s.memory.tab == Tab::Review {
        "mprop"
    } else {
        "mrow"
    };
    tx.scroll_into_view(format!("{prefix}-{id}"));
    if s.memory.panel {
        s.open_record(&id);
    } else {
        s.memory.cursor = Some(id);
    }
}

/// The Memory page's keys. Letters only count while no text box has the keyboard.
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    match &e.key {
        Key::Down => move_cursor(s, 1, tx),
        Key::Up => move_cursor(s, -1, tx),
        Key::Enter => match s.memory.cursor.clone() {
            Some(id) if s.memory.keyboard => s.open_record(&id),
            _ => return false,
        },
        Key::Escape => {
            if s.memory.ask.is_some() {
                s.memory.ask = None;
            } else if s.memory.panel {
                s.close_record();
            } else if s.memory.comment_for.is_some() {
                s.memory.comment_for = None;
            } else if s.memory.filter_menu.is_some() {
                s.memory.filter_menu = None;
            } else if s.memory.project_menu {
                s.memory.project_menu = false;
            } else if !s.memory.search.text.is_empty() {
                s.memory.search = TextEdit::default();
            } else if s.memory.settle_all {
                s.memory.settle_all = false;
                s.memory.settle_meeting = None;
            } else if s.memory.browse {
                s.memory.browse = false;
                s.memory.meeting = None;
            } else {
                s.go_home();
            }
        }
        Key::Text(t) if t == "j" => move_cursor(s, 1, tx),
        Key::Text(t) if t == "k" => move_cursor(s, -1, tx),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod refresh_tests {
    use super::*;

    fn synthetic_record(id: &str, kind: RecordType, project: &str) -> MemoryRecord {
        serde_json::from_value(serde_json::json!({
            "id": id, "type": kind, "text": "Synthetic workshop reference", "project": project,
            "people": [], "sources": [], "first_seen": null, "last_confirmed": null,
            "status": "active", "basis": "source_supported", "supersedes": null,
            "updated_at": "2026-11-01T00:00:00Z", "source_fingerprint": "synthetic"
        }))
        .unwrap()
    }

    #[test]
    fn record_rows_render_question_settled_and_failed_states_with_one_tab_stop_each() {
        let mut question = synthetic_record("question", RecordType::Question, "Workshop");
        question.sources = ["[[demo]]", "[[sample]]"]
            .map(|reference| MemorySource::Meeting {
                reference: reference.into(),
            })
            .into();
        let mut retired = synthetic_record("retired", RecordType::Decision, "Workshop");
        retired.status = RecordStatus::Retired;
        let mut replaced = synthetic_record("replaced", RecordType::Question, "Workshop");
        replaced.status = RecordStatus::Superseded;
        let memory = MemState {
            cursor: Some(question.id.clone()),
            keyboard: true,
            errors: HashMap::from([(question.id.clone(), "The write failed.".into())]),
            records: vec![question, retired, replaced],
            ..Default::default()
        };
        let p = super::super::theme::palette(false);
        let view = |m: &MemState, _: &mut rune::Cx<MemState>| {
            let p = super::super::theme::palette(false);
            col()
                .w_full()
                .children(m.records.iter().map(|r| record_row(m, r, 390.0, false, p)))
        };
        let mut rt = rune::Runtime::new(memory, view, (390.0, 350.0), rune::Env::default());
        rt.frame(0.0);
        let nodes = rt.elements();
        for (key, glyph, fill, ink) in [
            ("mrow-question", "solar-question-circle", p.chip, p.ink),
            (
                "mrow-retired",
                "solar-check-circle",
                rune::Color::TRANSPARENT,
                p.faint,
            ),
            (
                "mrow-replaced",
                "solar-check-circle",
                rune::Color::TRANSPARENT,
                p.faint,
            ),
        ] {
            let row = nodes
                .iter()
                .find(|n| n.key.as_deref() == Some(key))
                .unwrap();
            let parts: Vec<_> = nodes
                .iter()
                .filter(|n| n.path.starts_with(&row.path))
                .collect();
            assert!(row.focusable);
            assert_eq!(row.fill, fill);
            assert_eq!(parts.iter().filter(|n| n.focusable).count(), 1);
            assert!(parts.iter().any(|n| n.trace_name == Some(glyph)));
            assert!(
                parts
                    .iter()
                    .any(|n| n.key.as_deref() == Some("title") && n.text_color == Some(ink))
            );
        }
        assert!(
            nodes
                .iter()
                .any(|n| n.text.as_deref() == Some("Workshop · Not confirmed · 2 sources"))
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.text.as_deref() == Some("Not saved."))
        );
        assert!(
            nodes.iter().any(|n| n.key.as_deref() == Some("retry")
                && n.help.as_deref() == Some("The write failed."))
        );
    }

    #[test]
    fn combined_filters_and_collapsed_groups_keep_keyboard_rows_in_sync() {
        let mut memory = MemState {
            migrated: true,
            tab: Tab::Kind(RecordType::Person),
            project: Some("Synthetic workshop".into()),
            records: vec![
                synthetic_record("person", RecordType::Person, "Synthetic workshop"),
                synthetic_record(
                    "other-project",
                    RecordType::Person,
                    "Other synthetic project",
                ),
                synthetic_record("decision", RecordType::Decision, "Synthetic workshop"),
            ],
            ..Default::default()
        };
        assert_eq!(memory.shown_ids(), ["person"]);
        memory.collapsed.push(RecordType::Person);
        assert!(memory.shown_ids().is_empty());
        assert_eq!(memory.sorted_records().len(), 1);
        memory.collapsed.clear();
        memory.tab = Tab::All;
        memory.project = None;
        assert_eq!(memory.shown_ids().len(), 3);
    }

    #[test]
    fn meeting_filter_matches_whole_references_and_timestamps_only() {
        let m = MemState {
            meeting: Some("123-1".into()),
            ..Default::default()
        };
        for reference in ["[[123-1]]", "[[123-1@02:10]]"] {
            assert!(m.matches_meeting(&[MemorySource::Meeting {
                reference: reference.into()
            }]));
        }
        for reference in ["[[123-10]]", "[[123-10@02:10]]", "[[other]]"] {
            assert!(!m.matches_meeting(&[MemorySource::Meeting {
                reference: reference.into()
            }]));
        }
        assert!(!m.matches_meeting(&[]));
        assert!(MemState::default().matches_meeting(&[]));
    }

    fn snapshot(mutation_generation: u64, legacy: &str) -> Snapshot {
        Snapshot {
            settled: vec![],
            mutation_generation,
            projects: Default::default(),
            file: Default::default(),
            preview: None,
            legacy: legacy.into(),
        }
    }

    #[test]
    fn coalesces_requests_and_keeps_full_reconciliation_queued() {
        let mut m = MemState {
            legacy: "previous".into(),
            notice: Some("Reload memory to see current assignments.".into()),
            ..Default::default()
        };
        assert_eq!(m.begin_refresh(false), Some(1));
        assert!(m.notice.is_none());
        for full in [false, true, false, true, false] {
            assert_eq!(m.begin_refresh(full), None);
        }
        assert_eq!(m.refresh_generation, 1);
        assert_eq!(m.refresh_again, Some(true));
        assert_eq!(m.legacy, "previous");
        assert_eq!(m.finish_refresh(1, 7, Ok(snapshot(7, "old"))), Some(true));
        assert_eq!(m.legacy, "previous");
        assert!(!m.refresh_pending);
        assert_eq!(m.begin_refresh(true), Some(2));
        assert_eq!(m.finish_refresh(2, 7, Ok(snapshot(7, "latest"))), None);
        assert_eq!(m.legacy, "latest");
        assert!(!m.refresh_pending);
        assert_eq!(m.refresh_again, None);
    }

    #[test]
    fn rejects_old_request_and_mutation_results_without_losing_a_draft() {
        let mut m = MemState {
            legacy: "previous".into(),
            editing: true,
            panel: true,
            cursor: Some("claim".into()),
            edit: TextEdit::new("my draft"),
            ..Default::default()
        };
        assert_eq!(m.begin_refresh(true), Some(1));
        assert_eq!(
            m.finish_refresh(1, 8, Ok(snapshot(7, "stale"))),
            Some(false)
        );
        assert_eq!(m.legacy, "previous");
        assert_eq!(m.begin_refresh(false), Some(2));
        assert_eq!(m.finish_refresh(1, 8, Err("old error".into())), None);
        assert!(m.refresh_pending);
        assert!(m.refresh_error.is_none());
        assert_eq!(m.finish_refresh(2, 8, Ok(snapshot(8, "latest"))), None);
        assert_eq!(m.legacy, "latest");
        assert!(m.editing && m.panel);
        assert_eq!(m.cursor.as_deref(), Some("claim"));
        assert_eq!(m.edit.text, "my draft");
    }

    #[test]
    fn failure_keeps_the_snapshot_and_allows_retry() {
        let mut m = MemState {
            legacy: "previous".into(),
            ..Default::default()
        };
        assert_eq!(m.begin_refresh(true), Some(1));
        assert_eq!(
            m.finish_refresh(1, 7, Err("Could not update Memory.".into())),
            None
        );
        assert!(!m.refresh_pending);
        assert_eq!(m.legacy, "previous");
        assert_eq!(m.refresh_error.as_deref(), Some("Could not update Memory."));
        assert_eq!(m.begin_refresh(false), Some(2));
        assert!(m.refresh_error.is_none());
        assert_eq!(m.begin_refresh(true), None);
        assert_eq!(m.finish_refresh(2, 7, Err("Stopped".into())), Some(true));
        assert_eq!(m.legacy, "previous");
    }
}
