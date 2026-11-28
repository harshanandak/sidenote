//! Focus and visible-window timer refresh. One worker, no filesystem watcher.
use super::{Waker, check, memory};
use sidenote::core::{self, Meeting, Saved};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant, SystemTime},
};
static FIRST_FRAME: AtomicBool = AtomicBool::new(false);
pub(super) fn first_frame() {
    FIRST_FRAME.store(true, Ordering::Release);
}
const EVERY: Duration = Duration::from_secs(10);
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Options {
    pub check_date: Option<chrono::NaiveDate>,
    pub people: bool,
    pub memory: bool,
    pub check: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            check_date: None,
            people: true,
            memory: true,
            check: true,
        }
    }
}
pub(super) struct Data {
    pub meetings: Vec<Meeting>,
    pub short_recordings: Option<Vec<core::recording_guard::ShortRecording>>,
    pub recent_meeting: Option<(i64, core::ask_options::LiveSource)>,
    pub notes_jobs: std::collections::BTreeMap<String, super::notes_status::Job>,
    pub retained: Vec<PathBuf>,
    pub temporary: bool,
    pub errors: Vec<String>,
    pub pipeline: core::Pipeline,
    pub shared: Option<bool>,
    pub projects: Option<core::Projects>,
    pub companies: Option<core::companies::Companies>,
    pub affiliations: Option<core::people::Affiliations>,
    pub overrides: Option<core::Overrides>,
    pub cutoff: Option<Option<chrono::NaiveDate>>,
    pub saved: HashMap<String, Saved>,
    pub memory: Option<memory::MemoryView>,
    pub profile: Option<core::relay_preview::SharedProfile>,
    pub check: Option<check::Refresh>,
}
pub(crate) struct Result {
    pub(super) generation: u64,
    pub(super) options: Options,
    pub(super) data: Option<Box<Data>>,
}
fn loaded<T>(result: std::io::Result<T>) -> Option<T> {
    result
        .inspect_err(|e| {
            core::file_reads::error(&core::state_dir().join("state"), e);
        })
        .ok()
}
impl Data {
    pub(super) fn read(options: Options) -> Self {
        let scope = core::file_reads::Scope::begin();
        let device = core::DeviceSettings::load();
        let shared = device.as_ref().ok().and_then(|d| {
            core::file_reads::entries(&core::state_dir().join("state"))
                .ok()
                .map(|entries| {
                    entries.iter().any(|e| {
                        let name = e.file_name();
                        let name = name.to_string_lossy();
                        name != d.id
                            && name.len() == 32
                            && name.bytes().all(|c| c.is_ascii_hexdigit())
                            && core::file_reads::directory(&e.path())
                    })
                })
        });
        let device_error = device.is_err();
        let (meetings, mut errors) = core::scan();
        let short_recordings = loaded(core::recording_guard::short_recordings(
            &core::state_dir(),
            &core::meetings_dir(),
        ));
        let shared = shared.map(|seen| seen || meetings.iter().any(|m| m.device.is_some()));
        let notes_jobs = super::notes_status::read(&core::state_dir(), &meetings);
        let pipeline = core::load_pipeline(&meetings);
        let store = core::TaskStore::default();
        let saved_store = core::StateStore::default();
        let affiliations = options
            .people
            .then(|| loaded(core::Config::load()))
            .flatten()
            .map(|config| {
                if FIRST_FRAME.load(Ordering::Acquire) {
                    core::people::Affiliations::load(&config)
                } else {
                    core::people::Affiliations::identity(&config)
                }
            });
        let companies = options
            .people
            .then(|| loaded(core::companies::Companies::load(&core::state_dir())))
            .flatten();
        let company_threads: Vec<_> = companies
            .iter()
            .flat_map(|f| f.catalog.records.keys())
            .map(|id| core::company_ask_key(id))
            .collect();
        let saved = meetings
            .iter()
            .map(|m| m.id.as_str())
            .chain(std::iter::once(core::ALL_KEY))
            .chain(company_threads.iter().map(String::as_str))
            .filter_map(|id| loaded(saved_store.load(id)).map(|s| (id.to_string(), s)))
            .collect();
        let profile = loaded(core::relay_preview::profile(&store.root));
        let projects = loaded(store.load_projects());
        let overrides = loaded(store.load_overrides());
        let cutoff = loaded(store.load_settings()).map(|s| s.cutoff());
        let memory = if options.memory {
            loaded(memory::MemoryView::read(
                &core::memory::MemoryStore::default(),
            ))
        } else {
            None
        };
        let check = options
            .check
            .then(|| check::CheckState::read_refresh(options.check_date))
            .filter(|c| {
                if let Some(notice) = &c.notice {
                    errors.push(notice.clone());
                    false
                } else {
                    true
                }
            });

        let reads = scope.stats();
        if device_error {
            errors.push("Device settings could not be read. Refresh will retry.".into());
        }
        if reads.skipped > 0 {
            errors.push(format!(
                "Skipped {} unreadable or unsafe files. Previous data is kept where a read failed.",
                reads.skipped
            ));
        }
        Self {
            recent_meeting: core::ask_options::recent_meeting(
                &core::meetings_dir(),
                chrono::Utc::now().timestamp_millis(),
            ),
            meetings,
            short_recordings,
            notes_jobs,
            retained: scope.failed(),
            temporary: reads.temporary,
            errors,
            pipeline,
            shared,
            projects,
            companies,
            affiliations,
            overrides,
            cutoff,
            saved,
            memory,
            profile,
            check,
        }
    }
}
pub(super) fn retain_catalog(
    current: &[Meeting],
    incoming: &mut Vec<Meeting>,
    failed: &[PathBuf],
) -> bool {
    retain_catalog_in(&core::state_dir(), current, incoming, failed)
}

fn retain_catalog_in(
    state: &Path,
    current: &[Meeting],
    incoming: &mut Vec<Meeting>,
    failed: &[PathBuf],
) -> bool {
    let Ok(suppressed) = core::meeting_delete::suppressed_ids(state) else {
        incoming.clear();
        return false;
    };
    incoming.retain(|meeting| !suppressed.contains(&meeting.id));
    for old in current {
        if !suppressed.contains(&old.id)
            && failed
                .iter()
                .any(|p| old.folder.starts_with(p) || p.starts_with(&old.folder))
            // One copy per meeting id, as the scan keeps: a readable peer copy replaces a failed one.
            && !incoming
                .iter()
                .any(|m| m.folder == old.folder || m.id == old.id)
        {
            incoming.push(old.clone());
        }
    }
    incoming.sort_by(|a, b| b.started.cmp(&a.started).then(b.id.cmp(&a.id)));
    true
}
#[derive(PartialEq, Eq)]
pub(super) struct Metadata {
    files: Vec<(PathBuf, u64, Option<SystemTime>)>,
    pub skipped: usize,
    temporary: bool,
    relay_revision: Option<String>,
}
fn walk(path: &Path, out: &mut Metadata, depth: usize) {
    if !core::file_reads::directory(path) {
        if path.exists() {
            out.skipped += 1;
        }
        return;
    }
    walk_checked(path, out, depth);
}
// Roots are checked above; every child is checked in its parent's enumeration.
fn walk_checked(path: &Path, out: &mut Metadata, depth: usize) {
    #[cfg(windows)]
    use std::os::windows::fs::MetadataExt;
    if depth > 8 {
        out.skipped += 1;
        return;
    }
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(e) => {
            out.temporary |= matches!(e.raw_os_error(), Some(32 | 33));
            out.skipped += 1;
            return;
        }
    };
    for (n, e) in entries.enumerate() {
        if n >= core::file_reads::ENTRIES_MAX {
            out.skipped += 1;
            break;
        }
        let e = match e {
            Ok(e) => e,
            Err(_) => {
                out.skipped += 1;
                continue;
            }
        };
        let path = e.path();
        let m = match e.metadata() {
            Ok(m) => m,
            Err(_) => {
                out.skipped += 1;
                continue;
            }
        };
        #[cfg(windows)]
        let linked = m.file_attributes() & 0x400 != 0;
        #[cfg(not(windows))]
        let linked = m.file_type().is_symlink();
        if path.as_os_str().len() > 1024 || linked {
            out.skipped += 1;
            continue;
        }
        if m.is_dir() {
            if path.extension().is_some_and(|e| e == "json") {
                out.skipped += 1;
                continue;
            }
            walk_checked(&path, out, depth + 1);
        } else if core::file_reads::regular(&m)
            && path
                .extension()
                .is_some_and(|e| matches!(e.to_str(), Some("json" | "md" | "ndjson")))
        {
            out.files.push((path, m.len(), m.modified().ok()));
        } else if !m.is_file() {
            out.skipped += 1;
        }
    }
}
pub(super) fn metadata() -> Metadata {
    let mut out = Metadata {
        files: Vec::new(),
        skipped: 0,
        temporary: false,
        relay_revision: core::relay_preview::revision(&core::state_dir())
            .ok()
            .flatten(),
    };
    let root = core::state_dir();
    let suppression = root.join("local/deleted-meeting-ids.json");
    if let Ok(m) = core::file_reads::checked_metadata(&suppression) {
        out.files.push((suppression, m.len(), m.modified().ok()));
    }
    for folder in [
        root.join("state"),
        root.join("memory"),
        root.join("checks"),
        root.join("local/notes-jobs"),
        root.join("local/draft-summaries"),
        root.join("local/recording-guard"),
        core::meetings_dir(),
        core::peers_dir(),
    ] {
        walk(&folder, &mut out, 0);
    }
    if let Ok(entries) = core::file_reads::entries(&root) {
        for e in entries {
            if e.path().extension().is_some_and(|e| e == "json") {
                if let Ok(m) = core::file_reads::checked_metadata(&e.path()) {
                    out.files.push((e.path(), m.len(), m.modified().ok()));
                } else {
                    out.skipped += 1;
                }
            }
        }
    }
    out.files.sort();
    out
}
#[derive(Clone)]
pub(super) struct Handle {
    tx: Arc<SyncSender<()>>,
    options: Arc<Mutex<Options>>,
    slot: Arc<Mutex<Option<Result>>>,
    dirty: Arc<AtomicBool>,
}
impl Handle {
    #[cfg(any(test, all(windows, feature = "idle-cpu-proof")))]
    pub(super) fn unchanged_snapshot_ready(&self) -> bool {
        self.slot
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|result| result.data.is_none())
    }

    #[cfg(any(test, all(windows, feature = "idle-cpu-proof")))]
    pub(super) fn pause_take(
        &self,
        acquired: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
    ) -> std::thread::JoinHandle<()> {
        let slot = self.slot.clone();
        std::thread::spawn(move || {
            let _guard = slot.lock().unwrap();
            acquired.send(()).unwrap();
            let _ = release.recv();
        })
    }
    pub fn request(&self, options: Options) {
        let mut current = self.options.lock().unwrap();
        if *current != options {
            *current = options;
            self.dirty.store(true, Ordering::Release);
        }
        drop(current);
        let _ = self.tx.try_send(());
    }
    pub fn invalidate(&self) {
        self.dirty.store(true, Ordering::Release);
    }
    pub fn take(&self) -> Option<Result> {
        self.slot.lock().unwrap().take()
    }
}
fn visible() -> bool {
    super::hex::window_shown()
}
pub(super) fn updated_at(
    previous: Option<Instant>,
    shared: Option<bool>,
    changed: bool,
) -> Option<Instant> {
    if shared == Some(false) {
        None
    } else if changed && shared == Some(true) {
        Some(Instant::now())
    } else {
        previous
    }
}
fn refresh_needed(force: bool, changed: bool, retry: &Retry) -> bool {
    force || changed || retry.ready()
}
#[derive(Default)]
struct Retry {
    attempts: usize,
    due: Option<Instant>,
}
impl Retry {
    fn ready(&self) -> bool {
        self.due.is_some_and(|d| Instant::now() >= d)
    }
    fn complete(&mut self, temporary: bool) {
        if temporary {
            self.failed();
        } else {
            self.clear();
        }
    }
    fn clear(&mut self) {
        *self = Self::default();
    }
    fn failed(&mut self) {
        let delays = [10, 30, 60, 300];
        self.due = Some(Instant::now() + Duration::from_secs(delays[self.attempts.min(3)]));
        self.attempts = self.attempts.saturating_add(1);
    }
}
static PANICS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn read_caught(options: Options) -> (Data, bool) {
    read_caught_with(|| Data::read(options))
}
fn read_caught_with(read: impl FnOnce() -> Data + std::panic::UnwindSafe) -> (Data, bool) {
    match std::panic::catch_unwind(read) {
        Ok(data) => (data, false),
        Err(_) => {
            PANICS.fetch_add(1, Ordering::Relaxed);
            (
                Data {
                    recent_meeting: None,
                    short_recordings: None,
                    meetings: vec![],
                    notes_jobs: Default::default(),
                    retained: vec![core::meetings_dir(), core::peers_dir()],
                    temporary: false,
                    errors: vec!["Refresh hit a problem and will retry".into()],
                    pipeline: HashMap::new(),
                    shared: None,
                    projects: None,
                    companies: None,
                    affiliations: None,
                    overrides: None,
                    cutoff: None,
                    saved: HashMap::new(),
                    memory: None,
                    profile: None,
                    check: None,
                },
                true,
            )
        }
    }
}
pub(super) fn start(waker: Waker, initial: Options) -> Handle {
    let (tx, rx) = mpsc::sync_channel(1);
    let tx = Arc::new(tx);
    let options = Arc::new(Mutex::new(initial));
    let worker_options = options.clone();
    let slot = Arc::new(Mutex::new(None));
    let worker_slot = slot.clone();
    let dirty = Arc::new(AtomicBool::new(true));
    let worker_dirty = dirty.clone();
    std::thread::spawn(move || {
        let mut previous = None;
        let mut retry = Retry::default();
        let mut next = Instant::now();
        loop {
            if let Err(mpsc::RecvTimeoutError::Disconnected) =
                rx.recv_timeout(next.saturating_duration_since(Instant::now()))
            {
                break;
            }
            next = Instant::now() + EVERY;
            if !FIRST_FRAME.load(Ordering::Acquire) || !visible() {
                continue;
            }
            let generation = core::mutation_generation();
            let options = *worker_options.lock().unwrap();
            let changed = metadata();
            let force = worker_dirty.swap(false, Ordering::AcqRel);
            let data = if refresh_needed(force, previous.as_ref() != Some(&changed), &retry) {
                let (data, panicked) = read_caught(options);
                retry.complete(panicked || changed.temporary || data.temporary);
                Some(Box::new(data))
            } else {
                None
            };
            previous = Some(changed);
            let result = Result {
                generation,
                options,
                data,
            };
            let mut slot = worker_slot.lock().unwrap();
            // An unchanged poll must not replace changed data the UI has yet to take.
            let old = if result.data.is_none() && slot.is_some() {
                None
            } else {
                slot.replace(result)
            };
            drop(slot);
            drop(old);
            waker();
        }
    });
    Handle {
        tx,
        options,
        slot,
        dirty,
    }
}

#[cfg(test)]
pub(super) fn observing_handle() -> (Handle, Arc<AtomicBool>) {
    let (tx, _) = mpsc::sync_channel(1);
    let dirty = Arc::new(AtomicBool::new(false));
    (
        Handle {
            tx: Arc::new(tx),
            options: Arc::new(Mutex::new(Options::default())),
            slot: Arc::new(Mutex::new(None)),
            dirty: dirty.clone(),
        },
        dirty,
    )
}

#[cfg(test)]
#[path = "final_refresh_proofs.rs"]
mod final_refresh_proofs;

#[cfg(test)]
mod recycle_tests {
    use super::*;

    #[test]
    fn stale_refresh_cannot_admit_or_retain_deleted_meetings() {
        use sidenote::core::meeting_delete::{Eligibility, Storage};
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let local = temp.path().join("meetings");
        let folder = local.join("100-1");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir(&state).unwrap();
        std::fs::write(folder.join("manifest.json"), br#"{"status":"complete"}"#).unwrap();
        let initial = Meeting::load_pending(&folder).unwrap();
        let generation = core::mutation_generation();
        let storage = Storage::new(&local, &state).unwrap();
        let id = storage
            .delete(
                "100-1",
                "Synthetic".into(),
                1,
                |_| Ok(Eligibility::Ready),
                |_| Ok(()),
            )
            .unwrap();
        assert_ne!(core::mutation_generation(), generation);
        let mut incoming = vec![initial.clone()];
        assert!(retain_catalog_in(
            &state,
            &[initial],
            &mut incoming,
            &[folder]
        ));
        assert!(incoming.is_empty());
        storage.restore(&id).unwrap();
        assert_eq!(
            core::scan_catalog(&state, &local, &temp.path().join("peers"))
                .0
                .len(),
            1
        );
    }

    #[test]
    fn failed_peer_copy_is_not_retained_beside_a_readable_copy() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let copy = |device: &str| {
            let folder = temp.path().join("peers").join(device).join("100-1");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("manifest.json"), br#"{"status":"complete"}"#).unwrap();
            Meeting::load_pending(&folder).unwrap()
        };
        let (failed, readable) = (copy("a"), copy("b"));
        assert_eq!(failed.id, readable.id);
        let mut incoming = vec![readable.clone()];
        assert!(retain_catalog_in(
            &state,
            std::slice::from_ref(&failed),
            &mut incoming,
            std::slice::from_ref(&failed.folder)
        ));
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].folder, readable.folder);
    }
}
