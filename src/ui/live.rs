//! The meeting page records ordered Hex parts. Pause stops a part without starting notes.
//! Only Generate notes or Update notes snapshots the parts and asks for notes. The live
//! reader and personal-note writer remain independent of the notes engine.
use super::{
    State,
    ask::{self, raised, rise},
    hex,
    list::text_title,
    roles,
    theme::Palette,
    widgets::{dot, hairline, label, link, quiet_button, scroll},
};
use chrono::{DateTime, Local, TimeZone};
use rune::{El, ScrollSpec, TextEdit, col, el, row};
use rune_kit::Kit;
use sidenote::core::live::{self as feed_io, LiveLine, Read};
use sidenote::core::{Meeting as CatalogMeeting, meeting_delete};
use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

type Work = Box<dyn FnOnce() + Send>;
pub(super) const NO_ID_COPY: &str = "Recording saved. Its summary will appear in Meetings.";
pub(super) struct NoteWriter {
    tx: mpsc::Sender<Work>,
    stopped: Arc<AtomicBool>,
}

struct WriterExit {
    stopped: Arc<AtomicBool>,
    wake: Option<super::Waker>,
}

impl Drop for WriterExit {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if thread::panicking()
            && let Some(wake) = &self.wake
        {
            wake();
        }
    }
}

impl NoteWriter {
    fn new(wake: Option<super::Waker>) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Work>();
        let stopped = Arc::new(AtomicBool::new(false));
        let exit = WriterExit {
            stopped: stopped.clone(),
            wake,
        };
        thread::Builder::new()
            .name("live-notes".into())
            .spawn(move || {
                let _exit = exit;
                for work in rx {
                    work();
                }
            })?;
        Ok(Self { tx, stopped })
    }

    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    fn drain(&self, wait: Duration) -> bool {
        let (done, result) = mpsc::channel();
        self.tx
            .send(Box::new(move || {
                let _ = done.send(());
            }))
            .is_ok()
            && result.recv_timeout(wait).is_ok()
    }
}
type SaveResult = (String, Result<Option<PathBuf>, String>);

/// How long a start may take before the page gives up waiting for Hex to say it is recording.
const START_WAIT: Duration = Duration::from_secs(25);
/// After this long without a new meeting, the page says the notes are slow.
const MAKING_SLOW: Duration = Duration::from_secs(10 * 60);
/// How often the new meeting's folder is checked for its quick title after Stop.
const TITLE_EVERY: Duration = Duration::from_secs(2);
/// How often the live transcript file is read while the page is visible.
const FEED_EVERY: Duration = Duration::from_millis(2500);
/// Notes are saved this long after the last key.
const SAVE_AFTER: Duration = Duration::from_secs(2);
// Give an in-flight write one autosave interval and a second for disk latency.
const CLOSE_WAIT: Duration = Duration::from_secs(3);
const CLOSE_RECOVERY_WAIT: Duration = Duration::from_secs(1);
static NEXT_CLOSE_RECOVERY: AtomicU64 = AtomicU64::new(0);
const WRITER_STOPPED: &str = "Notes saving stopped. Copy your notes or try saving again.";
/// With no live file after this long, the panel says so instead of only "Listening".
const NO_FEED_AFTER: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Hex was asked to record and has not said so yet.
    Starting,
    Recording,
    Paused,
    /// Stopped; waiting for the pipeline to write the meeting.
    Making,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Transcript {
    Finishing,
    Interrupted,
    Ready,
}

impl Transcript {
    fn heading(self, slow: bool) -> &'static str {
        match self {
            Self::Finishing => "You can leave this page",
            Self::Interrupted => "Transcript waiting for Hex",
            Self::Ready if slow => "The summary is taking a while",
            Self::Ready => "Writing the summary",
        }
    }

    fn detail(self) -> Option<&'static str> {
        match self {
            Self::Finishing => None,
            Self::Interrupted => {
                Some("Hex was restarted. It will finish this transcript when it starts again.")
            }
            Self::Ready => {
                Some("The summary opens here when it is ready. You can leave this page.")
            }
        }
    }
}

fn read_transcript_state(folder: &Path) -> Option<Transcript> {
    let raw = sidenote::core::file_reads::read(&folder.join("manifest.json"), 1024 * 1024).ok()?;
    let manifest: sidenote::core::Manifest = serde_json::from_slice(&raw).ok()?;
    match manifest.status.as_deref() {
        Some("recording" | "starting" | "transcribing") => Some(Transcript::Finishing),
        Some("interrupted") if folder.join("final.pending").is_file() => {
            Some(Transcript::Interrupted)
        }
        Some("complete") => Some(Transcript::Ready),
        _ => None,
    }
}

fn feed_summary_state(
    feed: &mut Feed,
    folder: &Path,
    root: &Path,
    off: bool,
    job: Option<&sidenote::core::notes::JobState>,
) -> bool {
    use sidenote::core::{notes, recording_guard};
    let transcript = read_transcript_state(folder);
    let skipped = (transcript == Some(Transcript::Ready))
        .then(|| folder.file_name()?.to_str())
        .flatten()
        .and_then(|id| notes::summary_skip(recording_guard::receipt(root, id), off, job));
    let changed = feed.transcript != transcript || feed.summary_skip != skipped;
    feed.transcript = transcript;
    feed.summary_skip = skipped;
    changed
}

fn making_transcript(s: &State) -> Transcript {
    s.live
        .as_ref()
        .and_then(|live| {
            live.feed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .transcript
        })
        .unwrap_or_else(|| {
            if s.hex
                .status
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .is_some_and(|st| st.meeting_state == "transcribing")
            {
                Transcript::Finishing
            } else {
                Transcript::Ready
            }
        })
}

pub(super) fn summary_skip(s: &State) -> Option<sidenote::core::notes::SummarySkip> {
    s.live
        .as_ref()?
        .feed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .summary_skip
}

pub(super) fn scene_making(s: &mut State, status: &str) {
    hex::scene(s, false);
    let mut live = Live::scene(Phase::Making, Some("Weekly planning"));
    live.feed.lock().unwrap_or_else(|e| e.into_inner()).hint = Some("1800000000000-1".into());
    live.feed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .transcript = Some(match status {
        "transcribing" => Transcript::Finishing,
        "interrupted" => Transcript::Interrupted,
        _ => Transcript::Ready,
    });
    if status == "short" {
        live.feed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .summary_skip = sidenote::core::notes::summary_skip(
            sidenote::core::recording_guard::Eligibility::Short,
            false,
            None,
        );
    }
    if status == "slow" {
        live.stopped = Some(Instant::now() - Duration::from_secs(49 * 60));
    }
    s.ai_notes.settings = Some(sidenote::core::notes::NotesSettings {
        engine: "scripts".into(),
        ..Default::default()
    });
    if let Some(meeting) = s.meetings.iter_mut().find(|m| m.device.is_none()) {
        meeting.summary_raw.clear();
        live.folder = Some(meeting.folder.clone());
    }
    s.live = Some(live);
    s.live_open = true;
    s.live_notes = TextEdit::new("Check the launch date with the team.");
}

/// What the reader thread knows about the live transcript. The page reads it; only the thread
/// (and the page's recording hints) writes it.
#[derive(Default)]
pub struct Feed {
    /// The id Hex reports for the running recording, when it reports one.
    hint: Option<String>,
    /// The start inferred from Hex's status, never the page's local startup time.
    start_ms: Option<i64>,
    /// The meeting folder, once found.
    folder: Option<PathBuf>,
    pub lines: Vec<LiveLine>,
    /// A plain sentence when the file could not be read.
    pub error: Option<String>,
    /// Words seen so far, and when that number last grew. The voice bars follow this.
    words: usize,
    last_words: Option<Instant>,
    stamp: Option<feed_io::Stamp>,
    /// Counts every change, so the page knows to draw again.
    version: u64,
    summary_skip: Option<sidenote::core::notes::SummarySkip>,
    notes_problem: Option<&'static str>,
    transcript: Option<Transcript>,
}

/// Where the notes stand, for the quiet line beside the heading.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Saving {
    /// Nothing typed, or nothing new since the last save.
    Quiet,
    Pending,
    Saved,
    Failed(String),
}

static NEXT_RECORDING_ID: AtomicU64 = AtomicU64::new(1);

pub struct Live {
    pub(super) session: Option<sidenote::core::meeting_parts::Meeting>,
    // Set only by Hex's accepted Stop reply for this live session.
    pub(super) generate_after_pause: bool,
    pause_save_pending: bool,
    notes_request: std::sync::Weak<()>,
    recovering_part: bool,
    parts_checked: Option<Instant>,
    // Session-local recording identity; never included in timing records.
    pub(super) recording_id: u64,
    pub(super) notes_response_time: u64,
    pub phase: Phase,
    /// The meetings that existed when this one began, so the new one can be told apart.
    known: HashSet<String>,
    source_lost: bool,
    source_seen: bool,
    began: Instant,
    stopped: Option<Instant>,
    /// When the meeting began, for the heading. `exact` is false until Hex gives the real start.
    wall_start: DateTime<Local>,
    exact: bool,
    /// The quick title, once its file exists. The title thread fills it in after Stop.
    quick_title: Arc<Mutex<Option<String>>>,
    /// Ends the scanning threads when the page lets go of this meeting.
    cancel: Arc<AtomicBool>,
    /// The live transcript, filled by the reader thread. `watching` tells that thread whether
    /// anyone can see the result: it reads only while the page is shown.
    pub feed: Arc<Mutex<Feed>>,
    watching: Arc<AtomicBool>,
    shown_version: u64,
    /// The transcript panel above the bar.
    pub panel_open: bool,
    /// The meeting folder the notes are saved into, once Hex has made it.
    folder: Option<PathBuf>,
    notes_loaded: bool,
    /// The editor's text at the last look, and the text on disk. They differ while typing.
    seen: String,
    saved: String,
    save_unconfirmed: bool,
    recovered: Option<String>,
    edited_at: Option<Instant>,
    pub saving: Saving,
    timer_armed: Arc<AtomicBool>,
    /// A render scene: nothing is read, and the bars hold still.
    scene: bool,
    pub close_failed: bool,
    start_failed: bool,
    saves: Arc<Mutex<Vec<SaveResult>>>,
    queued: VecDeque<String>,
}

impl Drop for Live {
    fn drop(&mut self) {
        super::response_time::superseded(std::mem::take(&mut self.notes_response_time));
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Live {
    pub(super) fn pause_feed(&self) {
        let mut feed = self.feed.lock().unwrap_or_else(|e| e.into_inner());
        self.watching.store(false, Ordering::Relaxed);
        feed.version += 1;
    }

    #[cfg(all(test, windows))]
    pub(super) fn age_past_start_timeout(&mut self) {
        self.began = Instant::now() - START_WAIT - Duration::from_secs(1);
    }

    #[cfg(all(test, windows))]
    pub(super) fn age_past_notes_autosave(&mut self) {
        self.edited_at = Some(Instant::now() - SAVE_AFTER);
    }

    pub(super) fn take_notes_timing(&mut self, recording: u64) -> u64 {
        if self.recording_id == recording {
            std::mem::take(&mut self.notes_response_time)
        } else {
            0
        }
    }

    fn can_close(&self, text: &str) -> bool {
        self.queued.is_empty()
            && ((!self.save_unconfirmed && text == self.saved)
                || self.recovered.as_deref() == Some(text))
    }

    fn recover_on_close(
        &mut self,
        root: &Path,
        text: &str,
        writer_stopped: bool,
        write_recovery: impl FnOnce(&str, &Path) -> bool + Send + 'static,
    ) -> bool {
        // An older save can still overwrite or remove its own recovery file.
        let path = root.join("unsaved-notes").join(format!(
            "{}-close-{}-{}.md",
            self.wall_start.timestamp_millis(),
            std::process::id(),
            NEXT_CLOSE_RECOVERY.fetch_add(1, Ordering::Relaxed),
        ));
        let recovered = if writer_stopped {
            // A crashed writer needs a durable copy without another scheduling deadline.
            write_recovery(text, &path)
        } else {
            let (done, result) = mpsc::channel();
            let draft = text.to_owned();
            thread::Builder::new()
                .name("live-notes-recovery".into())
                .spawn(move || {
                    let _ = done.send(write_recovery(&draft, &path));
                })
                .is_ok()
                && matches!(result.recv_timeout(CLOSE_RECOVERY_WAIT), Ok(true))
        };
        if recovered {
            self.recovered = Some(text.to_owned());
        }
        recovered
    }

    fn needs_save(&self, text: &str) -> bool {
        self.save_unconfirmed
            || text != self.saved
            || self.queued.back().is_some_and(|queued| queued != text)
    }

    fn new(phase: Phase, known: HashSet<String>) -> Self {
        Self {
            recording_id: NEXT_RECORDING_ID.fetch_add(1, Ordering::Relaxed),
            notes_response_time: 0,
            phase,
            known,
            source_lost: false,
            source_seen: false,
            began: Instant::now(),
            session: None,
            generate_after_pause: false,
            pause_save_pending: false,
            notes_request: Default::default(),
            recovering_part: false,
            parts_checked: None,
            stopped: (phase == Phase::Making).then(Instant::now),
            wall_start: Local::now(),
            exact: false,
            quick_title: Arc::new(Mutex::new(None)),
            cancel: Arc::new(AtomicBool::new(false)),
            feed: Arc::new(Mutex::new(Feed::default())),
            watching: Arc::new(AtomicBool::new(false)),
            shown_version: 0,
            panel_open: false,
            folder: None,
            notes_loaded: false,
            seen: String::new(),
            saved: String::new(),
            save_unconfirmed: false,
            recovered: None,
            edited_at: None,
            saving: Saving::Quiet,
            timer_armed: Arc::new(AtomicBool::new(false)),
            scene: false,
            close_failed: false,
            start_failed: false,
            saves: Arc::new(Mutex::new(Vec::new())),
            queued: VecDeque::new(),
        }
    }

    /// A stand-in for a render scene: no thread, and nothing waits on Hex. The start time is
    /// fixed so a render does not change with the clock, and `title` is the quick title if the
    /// scene has one.
    pub fn scene(phase: Phase, title: Option<&str>) -> Self {
        let mut live = Self::new(phase, HashSet::new());
        live.wall_start = Local
            .with_ymd_and_hms(2026, 11, 8, 14, 30, 0)
            .single()
            .unwrap_or(live.wall_start);
        live.exact = true;
        live.scene = true;
        *live.quick_title.lock().unwrap_or_else(|e| e.into_inner()) = title.map(str::to_string);
        live
    }

    fn meeting_id(&self) -> Option<String> {
        if let Some(folder) = &self.folder {
            return Some(folder.file_name()?.to_string_lossy().into_owned());
        }
        let feed = self.feed.lock().unwrap_or_else(|e| e.into_inner());
        feed.hint.clone().or_else(|| {
            let folder = feed.folder.as_deref()?;
            Some(folder.file_name()?.to_string_lossy().into_owned())
        })
    }
    fn matches_meeting(&self, meeting: &sidenote::core::Meeting) -> bool {
        meeting.device.is_none() && self.meeting_id().is_some_and(|id| meeting.id == id)
    }

    pub fn scene_pending(mut self, id: &str) -> Self {
        self.folder = Some(PathBuf::from(id));
        self
    }

    /// A scene with the transcript panel and lines shown. `lines` are `(seconds, you, text)`.
    pub fn scene_feed(mut self, lines: &[(u64, bool, &str)], panel_open: bool) -> Self {
        {
            let mut feed = self.feed.lock().unwrap_or_else(|e| e.into_inner());
            feed.lines = lines
                .iter()
                .map(|(seconds, you, text)| LiveLine {
                    seconds: *seconds,
                    you: *you,
                    text: (*text).to_string(),
                })
                .collect();
            feed.words = feed.lines.iter().map(|l| l.text.split(' ').count()).sum();
            feed.last_words = (!lines.is_empty()).then(Instant::now);
        }
        self.panel_open = panel_open;
        self
    }

    /// "Meeting at 14:30": the heading until the meeting has a real title.
    fn heading(&self) -> String {
        format!("Meeting at {}", self.wall_start.format("%H:%M"))
    }

    pub(super) fn quick_title(&self) -> Option<String> {
        self.quick_title
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl State {
    #[cfg(test)]
    pub(super) fn capture_live_work(&mut self) -> mpsc::Receiver<Work> {
        let (tx, rx) = mpsc::channel();
        self.live_writer = Some(NoteWriter {
            tx,
            stopped: Arc::new(AtomicBool::new(false)),
        });
        rx
    }

    pub(super) fn live_work(&mut self, work: impl FnOnce() + Send + 'static) -> bool {
        if self.live_writer.is_none() {
            self.live_writer = NoteWriter::new(self.waker.clone()).ok();
        }
        self.live_writer
            .as_ref()
            .is_some_and(|writer| writer.tx.send(Box::new(work)).is_ok())
    }

    fn live_saved(&mut self) -> bool {
        let stopped = self.live_writer.as_ref().is_some_and(NoteWriter::stopped);
        if stopped {
            self.live_writer = None;
        }
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        let results = std::mem::take(&mut *live.saves.lock().unwrap_or_else(|e| e.into_inner()));
        let mut changed = !results.is_empty();
        for (text, result) in results {
            let _ = live.queued.pop_front();
            match result {
                Ok(Some(folder)) => {
                    live.recovered = None;
                    live.folder = Some(folder);
                    live.saved = text;
                    live.save_unconfirmed = false;
                    if self.live_notes.text == live.saved {
                        live.edited_at = None;
                        live.saving = Saving::Saved;
                    }
                }
                Ok(None) => {
                    live.recovered = Some(text);
                    live.edited_at = Some(Instant::now());
                    live.saving = Saving::Failed(
                        "Notes are in a recovery copy. Trying the meeting folder again.".into(),
                    );
                }
                Err(error) => {
                    live.edited_at = Some(Instant::now());
                    live.saving = Saving::Failed(error);
                }
            }
        }
        if stopped {
            live.save_unconfirmed |= !live.queued.is_empty();
            live.queued.clear();
            live.edited_at = Some(Instant::now());
            live.saving = Saving::Failed(WRITER_STOPPED.into());
            changed = true;
        }
        if live.close_failed && live.can_close(&self.live_notes.text) {
            live.close_failed = false;
            changed = true;
        }
        changed
    }

    pub fn live_close(&mut self) -> bool {
        self.live_close_after(CLOSE_WAIT)
    }

    fn live_close_after(&mut self, wait: Duration) -> bool {
        self.live_close_after_with(wait, |text, path| {
            feed_io::preserve_notes(None, text, path).is_ok()
        })
    }

    fn live_close_after_with(
        &mut self,
        wait: Duration,
        write_recovery: impl FnOnce(&str, &Path) -> bool + Send + 'static,
    ) -> bool {
        if self
            .live_writer
            .as_ref()
            .is_none_or(|writer| !writer.stopped())
        {
            self.live_flush();
        }
        let drained = self
            .live_writer
            .as_ref()
            .is_none_or(|writer| writer.drain(wait));
        let had_writer = self.live_writer.is_some();
        self.live_saved();
        let writer_stopped = had_writer && self.live_writer.is_none();
        if drained {
            self.live_writer = None;
        }
        let safe = self.live.as_mut().is_none_or(|live| {
            let text = &self.live_notes.text;
            let safe = live.can_close(text)
                || live.recover_on_close(&self.store.root, text, writer_stopped, write_recovery);
            live.close_failed = !safe;
            if !safe {
                live.saving = Saving::Failed("Could not save the latest notes.".into());
            }
            safe
        });
        if safe {
            // Pending-part cleanup takes the same write lock a stalled save may hold.
            if self.live_writer.is_none() {
                self.parts_drop_unstarted(true);
            }
        } else {
            self.go_home();
            self.live_open = true;
        }
        safe
    }

    pub(super) fn live_install_blocker(&self) -> Option<&'static str> {
        let live = self.live.as_ref()?;
        if !live.start_failed && (live.phase == Phase::Starting || self.pending_recording_start()) {
            Some("Waiting for Hex to start recording.")
        } else if !live.can_close(&self.live_notes.text) {
            Some("Save your meeting notes before installing.")
        } else {
            None
        }
    }

    pub(super) fn live_delete_refusal(
        &self,
        meeting: &sidenote::core::Meeting,
    ) -> Option<sidenote::core::meeting_delete::Eligibility> {
        use sidenote::core::meeting_delete::Eligibility;
        if meeting.device.is_some() {
            return Some(Eligibility::Peer);
        }
        if let Some(live) = &self.live {
            if live.phase == Phase::Starting {
                return Some(Eligibility::Recording);
            }
            let matched = live.matches_meeting(meeting) && !live.known.contains(&meeting.id)
                || live.session.as_ref().is_some_and(|session| {
                    session.id == meeting.id
                        || session.parts.iter().any(|part| part.id == meeting.id)
                });
            if matched {
                if live.needs_save(&self.live_notes.text)
                    || !live.queued.is_empty()
                    || matches!(live.saving, Saving::Pending | Saving::Failed(_))
                    || live.close_failed
                    || live.recovered.is_some()
                {
                    return Some(Eligibility::UnsavedNotes);
                }
                return Some(match live.phase {
                    Phase::Starting | Phase::Recording => Eligibility::Recording,
                    Phase::Paused => Eligibility::Paused,
                    Phase::Making => Eligibility::Processing,
                });
            }
        }
        matches!(
            read_transcript_state(&meeting.folder),
            Some(Transcript::Finishing | Transcript::Interrupted)
        )
        .then_some(Eligibility::Processing)
    }

    pub fn on_live(&self) -> bool {
        self.live_open
            && self.live.is_some()
            && self.view.is_base()
            && self.selected.is_none()
            && !self.check_open
            && !self.memory_open
            && !self.hex_open
    }

    fn known_ids(&self) -> HashSet<String> {
        self.meetings
            .iter()
            .filter(|m| m.has_summary())
            .map(|m| m.id.clone())
            .collect()
    }

    /// Starts a live page with empty notes and, while a recording runs, the reader thread.
    fn live_begin(&mut self, phase: Phase, known: HashSet<String>) -> bool {
        if !self.live_close() {
            return false;
        }
        let live = Live::new(phase, known);
        self.live_notes = TextEdit::new("");
        if let Some(waker) = self.waker.clone() {
            let (feed, cancel, watching) = (
                live.feed.clone(),
                live.cancel.clone(),
                live.watching.clone(),
            );
            let _ = thread::Builder::new()
                .name("live-feed".into())
                .spawn(move || feed_loop(&feed, &cancel, &watching, &waker, feed_io::read_rolling));
        }
        self.live = Some(live);
        true
    }

    /// The New meeting button: start Hex recording and show the live page.
    pub fn new_meeting(&mut self) {
        if !self.updates.applying && self.hex.connection.begin_action() {
            self.new_meeting_continue();
        }
    }
    pub(super) fn new_meeting_continue(&mut self) {
        if self.pending_recording_start() {
            self.hex.connection.finish_action();
            self.open_live();
            return;
        }
        if hex::start_blocked(self).is_some() {
            self.hex.connection.finish_action();
            return;
        }
        if self.hex_status_unknown() {
            self.hex_recheck_start();
            return;
        }
        if !self.live_begin(Phase::Starting, self.known_ids()) {
            self.hex.error = Some("Save these notes before starting another meeting.");
            self.hex.connection.finish_action();
            return;
        }
        self.go_home();
        self.live_open = true;
        match sidenote::core::meeting_parts::Meeting::pending(
            &sidenote::core::state_dir(),
            chrono::Utc::now().timestamp_millis(),
        ) {
            Ok(m) => {
                if let Some(live) = &mut self.live {
                    live.folder = Some(
                        sidenote::core::meeting_parts::inputs(&sidenote::core::state_dir())
                            .join(&m.id),
                    );
                    live.session = Some(m);
                }
            }
            Err(_) => {
                self.hex.error = Some("Could not save meeting before recording.");
                self.hex.connection.finish_action();
                return;
            }
        }
        self.hex_record_start();
        if !self.hex_record_busy() {
            self.hex.connection.finish_action();
        }
        self.nav_done();
    }

    /// Back to the live page: from the top-right pill, or the sidebar's status line.
    pub fn open_live(&mut self) {
        if self.live.is_none() {
            if !self.hex_recording() {
                return;
            }
            self.live_begin(Phase::Recording, self.known_ids());
            self.live_sync_start();
        }
        self.go_home();
        self.live_open = true;
        self.nav_done();
    }

    /// Takes the heading's start time from the running recording, until Hex gives the real one.
    fn live_sync_start(&mut self) {
        let start = hex::recording_start(self);
        if let (Some(live), Some((at, exact))) = (self.live.as_mut(), start)
            && !live.exact
            && live.phase != Phase::Making
        {
            live.wall_start = at;
            live.exact = exact;
        }
    }

    /// The sidebar's Hex line: the live page while recording, else the Hex settings.
    pub fn hex_line_press(&mut self) {
        if self.hex_recording() {
            self.open_live();
        } else {
            self.open_hex();
            self.nav_done();
        }
    }

    /// Leaves the live page for the meetings list, keeping a recording going.
    pub fn leave_live(&mut self) {
        if self.live_close() {
            self.go_meetings();
        }
    }

    /// Ends a wait that has no source identity without claiming any meeting.
    pub fn finish_no_id_wait(&mut self) {
        if self.live_close() {
            self.live = None;
            self.live_open = false;
            self.go_meetings();
        }
    }
    /// Called after each Hex sample and each scan: moves the page along its three phases.
    pub fn live_sync(&mut self) {
        if self.hex_stop_pending() {
            return;
        }
        if matches!(self.live.as_ref(), Some(live) if live.close_failed || live.source_lost) {
            return;
        }
        self.parts_sync();
        if self.live.is_none() && !self.hex.parts_restore_checked {
            return;
        }
        let recording = self.hex_recording();
        let confirmed = self.hex_confirmed_recording();
        let running = self.hex_running();
        let busy = self.hex_record_busy();
        // A recording begun in Hex itself gets the same page and the same notes afterwards.
        if self.live.is_none() {
            if recording {
                self.live_begin(Phase::Recording, self.known_ids());
                self.live_sync_start();
            }
            return;
        }
        self.live_sync_start();
        if !recording
            && !busy
            && self.hex_stop_confirmed()
            && self.live.as_ref().is_some_and(|l| l.generate_after_pause)
        {
            self.retry_paused_notes();
        }
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.close_failed {
            return;
        }
        let pending_expired = running
            && !busy
            && matches!(&self.hex.status, Some(Ok(status)) if hex::stop::terminal(status))
            && live
                .session
                .as_ref()
                .and_then(|m| m.pending_start_ms)
                .is_some_and(|started| {
                    chrono::Utc::now().timestamp_millis() - started > START_WAIT.as_millis() as i64
                });
        if pending_expired {
            self.parts_start_failed();
            return;
        }
        match live.phase {
            Phase::Starting if confirmed => live.phase = Phase::Recording,
            Phase::Starting
                if running
                    && !busy
                    && !recording
                    && !live.start_failed
                    && live.began.elapsed() > START_WAIT =>
            {
                self.parts_start_failed();
            }
            Phase::Recording if running && !recording && !busy => self.live_stopped(),
            // A cached Stop target cannot confirm that another recording began.
            Phase::Making if matches!(&self.hex.status, Some(Ok(status)) if status.recording.is_some()) =>
            {
                // Another recording began while the notes were still being made.
                let known = self.known_ids();
                if self.live_begin(Phase::Recording, known) {
                    self.live_sync_start();
                }
            }
            _ => {}
        }
    }

    /// Recording ended, by Stop here or in Hex: wait for the notes.
    pub fn live_stopped(&mut self) {
        if self.live.as_ref().is_some_and(|l| l.session.is_some()) {
            self.parts_paused();
            return;
        }
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.phase == Phase::Making {
            return;
        }
        self.live_flush();
        let Some(live) = self.live.as_mut() else {
            return;
        };
        live.phase = Phase::Making;
        live.stopped = Some(Instant::now());
        live.panel_open = false;
        live.watching.store(false, Ordering::Relaxed);
        let title_cancel = live.cancel.clone();
        let quick = live.quick_title.clone();
        // Folder ids start with their creation time; allow for a clock a little behind the start.
        let since_ms = live.wall_start.timestamp_millis().saturating_sub(120_000);
        let known = live.known.clone();
        let waker = self.waker.clone();
        self.request_refresh();
        // The quick title lands a few seconds after Stop, well before the notes, so look often.
        let _ = thread::Builder::new()
            .name("live-title".into())
            .spawn(move || {
                let root = sidenote::core::meetings_dir();
                for _ in 0..(MAKING_SLOW.as_secs() / TITLE_EVERY.as_secs()) {
                    if title_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Some(title) =
                        sidenote::core::pending_quick_title(&root, since_ms, &known)
                    {
                        *quick.lock().unwrap_or_else(|e| e.into_inner()) = Some(title);
                        if let Some(w) = &waker {
                            w();
                        }
                        break;
                    }
                    thread::sleep(TITLE_EVERY);
                }
            });
        // The normal refresh keeps calling live_scanned, even when notes take hours.
    }

    pub(super) fn cancel_missing_summary(
        &mut self,
        known: bool,
        incoming: &[CatalogMeeting],
        failed: &[PathBuf],
    ) {
        if let Some(id) = self.live.as_ref().and_then(Live::meeting_id) {
            let in_catalog = |m: &CatalogMeeting| m.device.is_none() && m.id == id;
            // A failed read never publishes presence, so the previous catalog counts even when
            // this read failed and is about to replace it with an empty one.
            if (self.meetings.iter().any(in_catalog) || (known && incoming.iter().any(in_catalog)))
                && let Some(live) = self.live.as_mut()
            {
                live.source_seen = true;
            }
        }
        let missing = self.live.as_ref().is_some_and(|live| {
            known
                && live.phase == Phase::Making
                && (live.source_lost
                    || live.meeting_id().is_some_and(|id| {
                        let in_catalog = |m: &CatalogMeeting| m.device.is_none() && m.id == id;
                        let deleted = meeting_delete::suppressed_ids(&self.store.root);
                        let suppressed = deleted.is_ok_and(|ids| ids.contains(&id));
                        // A source can live under several roots (Hex folders, meeting parts),
                        // so only a refresh that read everything proves it is gone.
                        (live.source_seen || self.meetings.iter().any(in_catalog) || suppressed)
                            && !incoming.iter().any(in_catalog)
                            && failed.is_empty()
                    }))
        });
        if !missing {
            return;
        }
        self.live.as_mut().unwrap().source_lost = true;
        if self.live_close() {
            self.live = None;
            self.live_open = false;
        }
    }
    /// Accept a result only when Hex supplied this recording's source identity.
    pub fn live_scanned(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.phase != Phase::Making || live.close_failed || live.source_lost {
            return;
        }
        let Some(id) = live.meeting_id() else {
            return;
        };
        let found = self
            .meetings
            .iter()
            .find(|m| {
                m.device.is_none()
                    && id == m.id
                    && m.has_summary()
                    && if let Some(session) = &live.session {
                        session
                            .notes_ready(&sidenote::core::state_dir())
                            .unwrap_or(false)
                    } else {
                        !live.known.contains(&m.id)
                    }
            })
            .map(|m| (m.id.clone(), m.folder.clone()));
        if let Some((id, folder)) = found {
            let timing_matched = live.folder.as_ref() == Some(&folder);
            if let Some(m) = self.live.as_mut().and_then(|l| l.session.as_mut()) {
                m.notes_parts = m.requested_parts;
                let _ = m.save(&sidenote::core::state_dir());
            }
            if !timing_matched && let Some(live) = &mut self.live {
                super::response_time::unmatched(std::mem::take(&mut live.notes_response_time));
            }
            let was_open = self.on_live();
            if let Some(live) = &mut self.live {
                live.folder = Some(folder.clone());
            }
            if !self.live_close() {
                return;
            }
            if let Ok(fresh) = sidenote::core::Meeting::load(&folder)
                && let Some(meeting) = self.meetings.iter_mut().find(|m| m.id == id)
            {
                *meeting = fresh;
                self.refresh_list();
            }
            let timing = self
                .live
                .as_mut()
                .map_or(0, |live| std::mem::take(&mut live.notes_response_time));
            self.live = None;
            self.live_open = false;
            // Opening a meeting needs the UI's scroll handle, so `poll` finishes the job.
            if timing_matched {
                super::response_time::ready(timing);
            }
            if was_open {
                self.live_ready = Some(id);
            }
        }
    }
}

// ---- the live transcript and the notes -------------------------------------------------------

/// Reads while the page is shown or a stopped meeting still needs a summary outcome.
fn feed_loop(
    feed: &Mutex<Feed>,
    cancel: &AtomicBool,
    watching: &AtomicBool,
    waker: &rune::Waker,
    mut read: impl FnMut(&Path, Option<feed_io::Stamp>) -> std::io::Result<Read>,
) {
    let root = sidenote::core::meetings_dir();
    let mut wait = Duration::from_millis(600);
    while !cancel.load(Ordering::Relaxed) {
        thread::sleep(wait);
        wait = FEED_EVERY;
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !watching.load(Ordering::Relaxed) {
            continue;
        }
        let generation;
        let (mut folder, hint, start_ms, last) = {
            let f = feed.lock().unwrap_or_else(|e| e.into_inner());
            if !watching.load(Ordering::Relaxed) {
                continue;
            }
            generation = f.version;
            (f.folder.clone(), f.hint.clone(), f.start_ms, f.stamp)
        };
        if folder.is_none() {
            folder = feed_io::find_folder(&root, hint.as_deref(), start_ms);
        }
        let Some(folder) = folder else {
            continue;
        };
        let read = read(&folder, last);
        let words = match &read {
            Ok(Read::Lines(lines, _)) => lines.iter().map(|l| l.text.split(' ').count()).sum(),
            _ => 0,
        };
        let notes_problem = feed_io::notes_problem(&folder);
        let job = folder
            .file_name()
            .and_then(|id| id.to_str())
            .and_then(sidenote::core::notes::status);
        let mut summary = Feed::default();
        feed_summary_state(
            &mut summary,
            &folder,
            &sidenote::core::state_dir(),
            sidenote::core::notes::scripts_policy() == "off",
            job.as_ref(),
        );
        let mut f = feed.lock().unwrap_or_else(|e| e.into_inner());
        if !watching.load(Ordering::Relaxed) || f.version != generation {
            continue;
        }
        let mut changed = f.folder.as_ref() != Some(&folder)
            || f.notes_problem != notes_problem
            || f.transcript != summary.transcript
            || f.summary_skip != summary.summary_skip;
        f.transcript = summary.transcript;
        f.summary_skip = summary.summary_skip;
        f.notes_problem = notes_problem;
        f.folder = Some(folder);
        match read {
            Ok(Read::Waiting | Read::Same) => {
                if f.error.take().is_some() {
                    changed = true;
                }
            }
            Ok(Read::Lines(lines, stamp)) => {
                if words > f.words {
                    f.last_words = Some(Instant::now());
                }
                f.words = words;
                f.lines = lines;
                f.stamp = Some(stamp);
                f.error = None;
                changed = true;
            }
            Err(e) => {
                let message = format!("Could not read the live transcript ({})", e.kind());
                if f.error.as_deref() != Some(message.as_str()) {
                    f.error = Some(message);
                    changed = true;
                }
            }
        }
        if changed {
            f.version += 1;
            drop(f);
            waker();
        }
    }
}

impl State {
    #[cfg(test)]
    pub(super) fn stop_during_feed_read(&mut self) {
        let dir = tempfile::tempdir().unwrap();
        let rolling = dir.path().join("transcript.rolling.ndjson");
        std::fs::write(rolling, r#"{"text":"Synthetic line"}"#).unwrap();
        let live = self.live.as_ref().unwrap();
        let (feed, watching) = (live.feed.clone(), live.watching.clone());
        feed.lock().unwrap().folder = Some(dir.path().to_owned());
        watching.store(true, Ordering::Relaxed);
        let cancel = AtomicBool::new(false);
        let waker: rune::Waker = Arc::new(|| {});
        feed_loop(&feed, &cancel, &watching, &waker, |folder, last| {
            let result = feed_io::read_rolling(folder, last);
            assert!(matches!(&result, Ok(Read::Lines(lines, _)) if lines.len() == 1));
            self.hex_record_stop(hex::recording_id(self));
            cancel.store(true, Ordering::Relaxed);
            result
        });
        assert!(feed.lock().unwrap().lines.is_empty());
    }

    /// Called on every poll. Hands the reader thread its hint and whether to read, adopts the
    /// meeting folder, and saves the notes 2 s after the last key. True when the page changed.
    pub fn live_tick(&mut self) -> bool {
        if self.live.as_ref().is_some_and(|l| {
            !l.scene
                && !(l.close_failed || l.source_lost)
                && l.session.is_some()
                && l.parts_checked.is_none_or(|at| at.elapsed() >= FEED_EVERY)
        }) {
            self.parts_sync();
        }
        let saved = self.live_saved();
        let stopping = self.hex_stop_pending();
        let visible = self.on_live() && hex::window_shown();
        let hint = hex::recording_id(self);
        let start_ms = hex::recording_start(self).map(|(at, _)| at.timestamp_millis());
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        if live.scene {
            return false;
        }
        let mut changed = saved;
        live.watching.store(
            !stopping && live.session.is_none() && (visible || live.phase == Phase::Making),
            Ordering::Relaxed,
        );
        {
            let mut feed = live.feed.lock().unwrap_or_else(|e| e.into_inner());
            if feed.hint.is_none() {
                feed.hint = hint;
            }
            if feed.start_ms.is_none() {
                feed.start_ms = start_ms;
            }
            if live.folder.is_none() {
                live.folder = feed.folder.clone();
            }
            if feed.version != live.shown_version {
                live.shown_version = feed.version;
                changed = true;
            }
        }
        // A recording that was already running (the app was restarted) has notes on disk.
        if let (Some(folder), false, false) = (live.folder.clone(), live.notes_loaded, stopping) {
            live.notes_loaded = true;
            let existing = feed_io::read_notes(&folder);
            if self.live_notes.text.is_empty() && !existing.is_empty() {
                self.live_notes = TextEdit::new(&existing);
                live.seen = existing.clone();
                live.saved = existing;
                live.saving = Saving::Saved;
                changed = true;
            }
        }
        let edited = self.live_notes.text != live.seen;
        if edited {
            live.seen = self.live_notes.text.clone();
            live.edited_at = Some(Instant::now());
            if !matches!(live.saving, Saving::Failed(_)) {
                live.saving = Saving::Pending;
            }
            changed = true;
        }
        if live.needs_save(&live.seen) {
            let due = live.edited_at.is_none_or(|t| t.elapsed() >= SAVE_AFTER);
            if due {
                changed |= self.live_save();
            } else {
                self.live_arm_save();
            }
        }
        ((saved || edited) && self.parts_drop_unstarted(false)) || changed
    }

    /// Wakes the page once, about 2 s from now, so a save happens with no key pressed.
    fn live_arm_save(&self) {
        let (Some(live), Some(waker)) = (self.live.as_ref(), self.waker.clone()) else {
            return;
        };
        if live.timer_armed.swap(true, Ordering::Relaxed) {
            return;
        }
        let armed = live.timer_armed.clone();
        let _ = thread::Builder::new()
            .name("live-save-timer".into())
            .spawn(move || {
                thread::sleep(SAVE_AFTER + Duration::from_millis(150));
                armed.store(false, Ordering::Relaxed);
                waker();
            });
    }

    /// Writes the notes now. True when the page's state changed.
    fn live_save(&mut self) -> bool {
        if self.updates.applying {
            return false;
        }
        let text = self.live_notes.text.clone();
        let hint = hex::recording_id(self);
        let start_ms = hex::recording_start(self).map(|(at, _)| at.timestamp_millis());
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        if live.queued.back() == Some(&text) {
            return false;
        }
        let folder = live.folder.clone();
        let recovery = self
            .store
            .root
            .join("unsaved-notes")
            .join(format!("{}.md", live.wall_start.timestamp_millis()));
        let results = live.saves.clone();
        live.queued.push_back(text.clone());
        live.saving = Saving::Pending;
        let wake = self.waker.clone();
        let queued = self.live_work(move || {
            let folder = folder.or_else(|| {
                feed_io::find_folder(&sidenote::core::meetings_dir(), hint.as_deref(), start_ms)
            });
            let result = feed_io::preserve_notes(folder, &text, &recovery);
            results
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((text, result));
            if let Some(wake) = wake {
                wake();
            }
        });
        if !queued {
            // A disconnected writer has already published every result it will deliver.
            self.live_writer = None;
            self.live_saved();
            let live = self.live.as_mut().unwrap();
            live.save_unconfirmed = true;
            live.queued.clear();
            live.saving = Saving::Failed(WRITER_STOPPED.into());
        }
        true
    }

    /// Saves any unsaved notes at once: before Stop, and when the recording ends in Hex.
    pub fn live_flush(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.scene {
            return;
        }
        live.seen = self.live_notes.text.clone();
        if live.needs_save(&live.seen) {
            self.live_save();
        }
    }

    fn live_toggle_panel(&mut self) {
        if let Some(live) = self.live.as_mut() {
            live.panel_open = !live.panel_open;
        }
    }
}

// ---- the page ---------------------------------------------------------------------------------

/// "1:05" or "1:02:05" for a line in the live transcript.
fn stamp(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// The live transcript above the bar, newest line at the bottom. A scroll area that follows the
/// end: it keeps following new lines, stops when the reader scrolls up, and follows again once
/// they are back at the bottom.
#[derive(Clone, Copy, Debug)]
pub(super) struct PanelLimits {
    pub space: f32,
    pub thread: f32,
    pub transcript: f32,
}
pub(super) fn gutter(s: &State) -> f32 {
    if s.narrow.get() { 16.0 } else { 40.0 }
}

fn document_height(pane: f32, measured: f32) -> f32 {
    measured.min(pane * 0.42)
}

fn panel_limits(
    pane: f32,
    document: f32,
    has_ask: bool,
    has_transcript: bool,
    notice: bool,
) -> PanelLimits {
    let space = pane - document_height(pane, document);
    // Pill, prompt row, gaps and bottom inset. Both panels spend only the space left over.
    let available = (space - 140.0 - if notice { 64.0 } else { 0.0 }).max(0.0);
    let gaps = if has_ask && has_transcript {
        20.0
    } else {
        10.0
    };
    let panels = (available - gaps).max(0.0);
    let thread = if has_ask {
        if has_transcript {
            (panels * 0.54).min(300.0)
        } else {
            panels.min(300.0)
        }
    } else {
        0.0
    };
    let transcript = if has_transcript {
        (panels - thread).min(384.0)
    } else {
        0.0
    };
    PanelLimits {
        space,
        thread,
        transcript,
    }
}
pub(super) fn limits(s: &State) -> PanelLimits {
    let id = s.ask_key();
    panel_limits(
        s.pane_h.get(),
        s.live_document_h.get(),
        !s.saved.messages.is_empty() || s.asks.contains_key(&id) || s.ask_errors.contains_key(&id),
        s.live.as_ref().is_some_and(|live| live.panel_open),
        s.ask_settings.fallback_notice.is_some(),
    )
}

fn transcript_panel(s: &State, p: Palette, cap: f32) -> El {
    let Some(live) = s.live.as_ref() else {
        return el();
    };
    let feed = live.feed.lock().unwrap_or_else(|e| e.into_inner());
    let header = row()
        .w_full()
        .justify_between()
        .items_center()
        .pl(16.0)
        .pr(8.0)
        .py(8.0)
        .child(
            roles::CONTROL
                .medium()
                .line("Transcript", p.ink)
                .key("live-panel-title"),
        )
        .child(
            quiet_button("Hide", "live-panel-hide", p)
                .label("Hide transcript")
                .on_press_tx(|s: &mut State, tx| {
                    s.live_toggle_panel();
                    tx.focus("meeting-transcript", true);
                }),
        );
    let note = |text: &str| {
        roles::SMALL
            .wrap(text, p.soft)
            .key("live-panel-note")
            .role("status")
            .px(16.0)
            .py(18.0)
    };
    let body = if let Some(error) = &feed.error {
        col().w_full().child(note(&format!(
            "{error}. Your notes are still saved, and the full transcript appears after you stop."
        )))
    } else if feed.lines.is_empty() {
        let mut c = col().w_full().child(note(if live.phase == Phase::Paused {
            "No transcript yet."
        } else {
            "Listening\u{2026} words appear a few seconds after they are said"
        }));
        if live.began.elapsed() > NO_FEED_AFTER && !live.scene {
            c = c.child(
                roles::META
                    .wrap(
                        "Nothing has come from Hex yet. The full transcript still appears after you stop.",
                        p.faint,
                    )
                    .px(16.0)
                .pb(16.0),
            );
        }
        c
    } else {
        let time_width = if feed.lines.iter().any(|line| line.seconds >= 3600) {
            62.0
        } else {
            44.0
        };
        let mut rows = col().w_full().px(16.0).py(6.0);
        for (i, line) in feed.lines.iter().enumerate() {
            if line.text == "Recording resumed" {
                rows = rows.child(
                    row().w_full().justify_center().py(8.0).child(
                        roles::META
                            .line("Recording resumed", p.faint)
                            .key(format!("live-gap-{i}")),
                    ),
                );
                continue;
            }
            let time = stamp(line.seconds);
            let elapsed = format!("+{}", time.strip_prefix('0').unwrap_or(&time));
            rows = rows.child(
                row()
                    .key(format!("live-line-{i}"))
                    .w_full()
                    .gap(10.0)
                    .items_start()
                    .py(3.0)
                    .child(roles::META.line(elapsed, p.soft).w(time_width))
                    .child(
                        roles::META
                            .bold()
                            .wrap(
                                if line.you { "You" } else { "Others" },
                                if line.you { p.ink } else { p.soft },
                            )
                            .w(40.0),
                    )
                    .child(roles::SMALL.wrap(&line.text, p.ink).grow(1.0).min_w(0.0)),
            );
        }
        let max_h = (cap - 44.0).max(0.0);
        col().w_full().min_h(0.0).shrink().child(
            col()
                .key("live-tx-scroll")
                .w_full()
                .max_h(max_h)
                .min_h(0.0)
                .shrink()
                .scroll(ScrollSpec { stick_bottom: true })
                .child(rows),
        )
    };
    raised(
        col()
            .key("live-panel")
            .w_full()
            .radius(16.0)
            .max_h(cap)
            .min_h(0.0)
            .shrink()
            .clip_rounded(16.0)
            .transition(rise(10.0))
            .child(header)
            .child(hairline(p))
            .child(body.min_h(0.0).shrink()),
        p,
    )
}

/// The bottom bar: the recording pill next to the Ask pill, with the transcript above.
fn bar(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let cap = limits(s).transcript;
    let panel = s
        .live
        .as_ref()
        .filter(|l| l.panel_open && cap >= 44.0)
        .map(|_| transcript_panel(s, p, cap));
    ask::overlay_with(s, k, p, narrow, layer, parts_control(s, p), panel)
}

/// The notes editor: plain text, several lines, under the "Meeting at HH:MM" heading.
fn notes_view(s: &State, k: &Kit, p: Palette, narrow: bool, starting: bool) -> El {
    let saving = s.live.as_ref().map(|l| l.saving.clone());
    let close_failed = s.live.as_ref().is_some_and(|live| live.close_failed);
    let status = match &saving {
        Some(Saving::Pending) if !close_failed => Some(("Saving\u{2026}", p.faint)),
        Some(Saving::Saved) if !close_failed => Some(("Saved", p.faint)),
        _ => None,
    };
    let mut head = row()
        .w_full()
        .gap(12.0)
        .items_end()
        .justify_between()
        .child(text_title(&heading(s), if narrow { 28.0 } else { 36.0 }, p).key("live-title"));
    if let Some((words, ink)) = status {
        head = head.child(
            roles::META
                .wrap(words, ink)
                .key("live-saved")
                .role("status")
                .pb(6.0),
        );
    }
    let hint = if starting {
        "Starting the recording. You can start writing now."
    } else {
        "Your notes save as you type. You can generate notes after pausing."
    };
    let field = rune::text_input(&s.live_notes, roles::READ.style())
        .key("live-notes-input")
        .w_full()
        .min_h(2.0 * roles::READ.lh)
        .color(p.ink)
        .placeholder("\u{2009}Write notes\u{2026}", p.faint)
        .caret(p.ink, k.selection())
        .label("Your notes");
    let field = if s.updates.applying {
        field.read_only(true)
    } else {
        field
            .on_edit(|s: &mut State| &mut s.live_notes)
            // Return starts a new line here; it would otherwise submit.
            .on_submit(|s: &mut State, _| s.live_notes.insert("\n"))
    };
    let mut c = col()
        .w_full()
        .items_start()
        .gap(if narrow { 10.0 } else { 12.0 })
        .child(head)
        .children(parts_status(s, p));
    if close_failed || matches!(&saving, Some(Saving::Failed(_))) {
        let recovered = s
            .live
            .as_ref()
            .is_some_and(|live| live.recovered.as_deref() == Some(s.live_notes.text.as_str()));
        let warning = match saving {
            _ if close_failed => {
                "Sidenote could not save your latest notes. Copy them before closing.".into()
            }
            Some(Saving::Failed(error)) if error == WRITER_STOPPED => error,
            _ => save_warning(s.hex_recording() && !s.hex_status_unknown(), recovered),
        };
        c = c.child(
            col()
                .key("live-save-warning")
                .w_full()
                .padding(16.0)
                .radius(10.0)
                .border(1.0, p.warn)
                .fill(p.warnbg)
                .gap(12.0)
                .role("alert")
                .child(
                    row()
                        .gap(8.0)
                        .items_center()
                        .child(super::icons::draw(&super::icons::WARNING, 16.0, p.warn))
                        .child(roles::NAME.wrap("Your notes need saving", p.warn)),
                )
                .child(roles::SMALL.wrap(warning, p.ink))
                .child(
                    row()
                        .flex_wrap()
                        .gaps(8.0, 8.0)
                        .child(
                            super::widgets::button(
                                "Copy notes",
                                "live-copy-notes",
                                super::widgets::ButtonRole::Primary,
                                p,
                            )
                            .on_press_tx(|s: &mut State, tx| {
                                tx.copy(s.live_notes.text.clone());
                            }),
                        )
                        .child(
                            super::area::button("Try saving again", "live-save-retry", p)
                                .key("live-close-retry")
                                .on_press(|s: &mut State| {
                                    s.live_flush();
                                    s.parts_drop_unstarted(false);
                                }),
                        )
                        .children(close_failed.then(|| {
                            super::area::button("Close anyway", "live-close-anyway", p)
                                .on_press_tx(|_: &mut State, tx| tx.quit())
                        })),
                ),
        );
    } else if !s.live.as_ref().is_some_and(|l| l.phase == Phase::Paused) {
        c = c.child(roles::META.wrap(hint, p.faint));
    }
    c = c.child(
        col()
            .key("live-notes")
            .w_full()
            .max_h((s.pane_h.get() * 0.35).clamp(96.0, 280.0))
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .mt(if narrow { 4.0 } else { 10.0 })
            .child(rune::auto_grow(field, 2, 400)),
    );
    c
}

fn save_warning(recording: bool, recovered: bool) -> String {
    let saved = if recovered {
        "Sidenote saved a backup copy, but could not save your notes in the meeting folder."
    } else {
        "Sidenote could not save your notes, even to a backup copy."
    };
    let status = if recording {
        " The meeting is still recording."
    } else {
        ""
    };
    format!(
        "{saved}{status} Copy your notes now, then try saving again. Do not close Sidenote yet."
    )
}

/// The page heading: "Meeting at HH:MM" from the start time.
fn heading(s: &State) -> String {
    s.live
        .as_ref()
        .map_or_else(|| "Meeting".into(), Live::heading)
}

fn making_view(s: &State, p: Palette, narrow: bool) -> El {
    if s.live.as_ref().is_some_and(|live| live.session.is_some()) {
        let failure = s
            .live
            .as_ref()
            .and_then(|live| live.session.as_ref())
            .and_then(|session| {
                let job = s.ai_notes.jobs.get(&session.id)?;
                if matches!(
                    job.state,
                    sidenote::core::notes::JobState::Queued
                        | sidenote::core::notes::JobState::Running
                        | sidenote::core::notes::JobState::Done { .. }
                ) {
                    return None;
                }
                s.meetings
                    .iter()
                    .find(|m| m.id == session.id)
                    .and_then(|m| crate::ui::notes_status::banner(s, m, p))
            });
        return col()
            .w_full()
            .gap(16.0)
            .child(text_title(&heading(s), if narrow { 28.0 } else { 36.0 }, p))
            .child(failure.unwrap_or_else(|| {
                roles::READ
                    .wrap("Writing notes", p.soft)
                    .key("meeting-writing-notes")
                    .role("status")
            }))
            .children((!s.live_notes.text.is_empty()).then(|| your_notes(&s.live_notes.text, p)));
    }

    if s.live
        .as_ref()
        .is_some_and(|live| live.session.is_none() && live.meeting_id().is_none())
    {
        return col()
            .key("live-no-id")
            .w_full()
            .items_start()
            .gap(16.0)
            .child(text_title(&heading(s), if narrow { 28.0 } else { 36.0 }, p))
            .child(roles::SECTION.wrap(NO_ID_COPY, p.ink).role("status"))
            .children((!s.live_notes.text.is_empty()).then(|| your_notes(&s.live_notes.text, p)))
            .child(
                link("Go to Meetings", p)
                    .on_press(|s: &mut State| s.finish_no_id_wait())
                    .key("live-go-meetings"),
            );
    }
    if let Some(skipped) = summary_skip(s) {
        return col()
            .key("live-summary-skipped")
            .w_full()
            .gap(16.0)
            .child(text_title(&heading(s), if narrow { 28.0 } else { 36.0 }, p))
            .child(roles::SECTION.wrap(skipped.heading(), p.ink).role("status"))
            .child(roles::SMALL.wrap(skipped.detail(), p.soft))
            .children(
                (!s.live_notes.text.trim().is_empty())
                    .then(|| your_notes(s.live_notes.text.trim(), p)),
            )
            .child(link("Back to meetings", p).on_press(|s: &mut State| s.leave_live()));
    }
    let transcript = making_transcript(s);
    let finishing = transcript != Transcript::Ready;
    let scripts = s
        .ai_notes
        .settings
        .as_ref()
        .is_some_and(|n| n.engine == "scripts");
    if !finishing
        && !scripts
        && let Some(live) = &s.live
    {
        let id = live
            .folder
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|v| v.to_str());
        let meeting = s.meetings.iter().find(|m| live.matches_meeting(m));
        let job = id
            .and_then(|id| {
                s.ai_notes
                    .jobs
                    .get(id)
                    .cloned()
                    .map(|j| (id.to_string(), j))
            })
            .or_else(|| {
                meeting.and_then(|m| super::notes_status::job(s, m).map(|j| (m.id.clone(), j)))
            });
        if let Some((id, job)) = job
            && let Some(banner) = super::notes_status::banner_for(s, &id, &job, p)
        {
            return col()
                .key("live-notes-state")
                .w_full()
                .gap(16.0)
                .child(text_title(&heading(s), if narrow { 28.0 } else { 36.0 }, p))
                .child(banner)
                .children(
                    (!s.live_notes.text.trim().is_empty())
                        .then(|| your_notes(s.live_notes.text.trim(), p)),
                )
                .child(link("Open meeting and transcript", p).on_press_tx(
                    move |s: &mut State, tx| {
                        s.leave_live();
                        if !s.on_live() {
                            s.select(&id, tx);
                            s.toggle_transcript();
                        }
                    },
                ))
                .child(link("Back to meetings", p).on_press(|s: &mut State| s.leave_live()));
        }
    }
    let slow = s
        .live
        .as_ref()
        .and_then(|l| l.stopped)
        .is_some_and(|t| t.elapsed() > MAKING_SLOW);
    let problem = s.live.as_ref().and_then(|live| {
        if !finishing && scripts {
            live.feed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .notes_problem
        } else {
            None
        }
    });
    if let Some(problem) = problem {
        return col().key("live-notes-problem").w_full().gap(16.0)
            .child(text_title(&heading(s), if narrow { 28.0 } else { 36.0 }, p))
            .child(roles::READ.wrap(problem, p.ink).role("status"))
            .child(roles::SMALL.wrap("Your recording is saved in Hex. Your own notes appear in saved meetings once they are saved.", p.soft))
            .children((!s.live_notes.text.trim().is_empty()).then(|| your_notes(s.live_notes.text.trim(), p)))
            .child(link("Back to meetings", p).on_press(|s: &mut State| s.leave_live()));
    }
    // The quick title as soon as it exists; until then the time heading, with a small note.
    let quick = s.live.as_ref().and_then(Live::quick_title);
    let step = |done: bool, text_: &str| {
        row()
            .gap(10.0)
            .items_center()
            .child(if done {
                label("\u{2713}", 14.0, 700.0, p.ok).w(14.0)
            } else {
                dot(8.0, p.ink).mx(3.0)
            })
            .child(if done {
                roles::READ.wrap(text_, p.soft)
            } else {
                roles::READ.bold().wrap(text_, p.ink)
            })
    };
    let bar = row()
        .w(260.0)
        .h(4.0)
        .radius(2.0)
        .fill(p.chip)
        .child(el().w(96.0).h(4.0).radius(2.0).fill(p.ink));
    let mut c = col()
        .key("live-making")
        .w_full()
        .items_start()
        .gap(14.0)
        .child(
            text_title(
                quick.as_deref().unwrap_or(&heading(s)),
                if narrow { 28.0 } else { 36.0 },
                p,
            )
            .key("live-name"),
        )
        .children((quick.is_none() && !finishing).then(|| {
            roles::META
                .wrap("Naming\u{2026}", p.faint)
                .key("live-naming")
                .role("status")
        }))
        .child(
            roles::SECTION
                .wrap(transcript.heading(slow), p.ink)
                .mt(if narrow { 6.0 } else { 18.0 })
                .role("status"),
        )
        .child(bar)
        .child(
            col()
                .gap(6.0)
                .mt(4.0)
                .child(step(
                    true,
                    if finishing {
                        "Recording stopped"
                    } else {
                        "Transcript ready"
                    },
                ))
                .child(step(
                    false,
                    match transcript {
                        Transcript::Finishing => "Finishing the transcript",
                        Transcript::Interrupted => "Waiting for Hex",
                        Transcript::Ready => "Writing the summary",
                    },
                )),
        )
        .children(
            transcript
                .detail()
                .map(|detail| roles::SMALL.wrap(detail, p.soft).max_w(460.0).mt(6.0)),
        );
    if !s.live_notes.text.trim().is_empty() {
        c = c.child(
            your_notes(s.live_notes.text.trim(), p)
                .key("live-your-notes")
                .mt(10.0),
        );
    }
    c = c.child(
        link("Back to meetings", p)
            .key("live-back")
            .mt(6.0)
            .on_press(|s: &mut State| s.leave_live()),
    );
    c
}

/// The user's own notes as paragraphs in a quiet card, with a small caption. Shared with the
/// meeting page.
pub fn your_notes(notes: &str, p: Palette) -> El {
    let mut body = col().w_full().gap(6.0);
    for line in notes.lines() {
        body = body.child(if line.trim().is_empty() {
            el().h(6.0)
        } else {
            roles::READ.wrap(line, p.ink)
        });
    }
    col()
        .w_full()
        .gap(10.0)
        .px(22.0)
        .py(16.0)
        .radius(16.0)
        .fill(p.card)
        .border(1.0, p.line)
        .child(roles::SECTION.wrap("Your notes", p.ink))
        .child(body)
}

fn stopping_view(s: &State, p: Palette, narrow: bool, timed_out: bool) -> El {
    let status = if timed_out {
        hex::stop::UNCONFIRMED
    } else {
        "Finishing the transcript"
    };
    col()
        .key("live-stop-progress")
        .w_full()
        .gap(16.0)
        .child(text_title(&heading(s), if narrow { 28.0 } else { 36.0 }, p))
        .child(roles::SECTION.wrap(status, p.ink).role("status"))
        .child(roles::SMALL.wrap("You can leave this page", p.soft))
        .children(timed_out.then(|| {
            quiet_button("Check again", "live-stop-recheck", p)
                .self_start()
                .px(0.0)
                .on_press(|s: &mut State| s.hex_recheck_stop())
        }))
        .children((!s.live_notes.text.trim().is_empty()).then(|| your_notes(&s.live_notes.text, p)))
        .child(link("Back to meetings", p).on_press(|s: &mut State| s.leave_live()))
}

pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let phase = s.live.as_ref().map_or(Phase::Recording, |l| l.phase);
    let stopping = s.hex_stop_pending();
    let body = if s.live.as_ref().is_some_and(|l| l.close_failed) {
        notes_view(s, k, p, narrow, phase == Phase::Starting)
    } else if stopping {
        stopping_view(s, p, narrow, s.hex_stop_unconfirmed())
    } else {
        match phase {
            Phase::Making => making_view(s, p, narrow),
            Phase::Paused => notes_view(s, k, p, narrow, false),
            Phase::Starting => notes_view(s, k, p, narrow, true),
            Phase::Recording => notes_view(s, k, p, narrow, false),
        }
    };
    let lost = !stopping && phase == Phase::Recording && !s.hex_running();
    let unknown = RecordingStatus::for_phase(s, Some(phase)) == RecordingStatus::Unknown;
    let mut doc = col()
        .key("live")
        .w_full()
        .max_w(super::meeting::COLUMN + 2.0 * gutter(s))
        .px(gutter(s))
        .pt(if narrow { 20.0 } else { 40.0 })
        .pb(if phase == Phase::Making { 56.0 } else { 12.0 })
        .items_start()
        .gap(16.0);
    if lost {
        doc = doc.child(
            roles::SMALL
                .medium()
                .wrap(
                    if unknown { "Recording status unknown. Sidenote is checking Hex. Pause will check the recording before stopping it." } else { "Hex is not answering. The recording may still be running in Hex." },
                    p.warn,
                )
                .role("alert"),
        );
    }
    if phase == Phase::Making
        && let Some(live) = &s.live
        && let Saving::Failed(error) = &live.saving
    {
        doc = doc.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    doc = doc.child(body);
    let mut pane = col()
        .key("live-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .fill(p.bg)
        .child(
            scroll(col().w_full().items_center().child(doc), "live-scroll").min_h(
                if phase == Phase::Making || stopping {
                    0.0
                } else {
                    document_height(s.pane_h.get(), s.live_document_h.get())
                },
            ),
        );
    if !stopping && (phase != Phase::Making || s.live.as_ref().is_some_and(|l| l.session.is_some()))
    {
        pane = pane.child(bar(s, k, p, narrow, layer));
    }
    pane
}

#[cfg(test)]
pub(super) fn scene_wait_for_summary(s: &mut State, id: Option<&str>, start_ms: i64) {
    let mut live = Live::scene(Phase::Making, None);
    live.known = s.known_ids();
    live.wall_start = DateTime::from_timestamp_millis(start_ms)
        .unwrap()
        .with_timezone(&Local);
    {
        let mut feed = live.feed.lock().unwrap();
        feed.hint = id.map(str::to_owned);
        feed.transcript = Some(Transcript::Ready);
    }
    s.live = Some(live);
    s.live_open = true;
}

#[cfg(test)]
mod writer_tests {
    #[test]
    fn feed_skip_requires_ready_receipt_and_clears_on_resume() {
        use sidenote::core::notes::{JobState, SummarySkip};
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("state");
        let folder = temp.path().join("1800000000000-1");
        fs::create_dir(&folder).unwrap();
        let manifest = |status| {
            let value = serde_json::json!({"status":status,"duration_ms":16700});
            fs::write(folder.join("manifest.json"), value.to_string()).unwrap();
        };
        fs::write(folder.join("transcript.ndjson"), "").unwrap();
        manifest("complete");
        let mut feed = Feed::default();
        feed_summary_state(&mut feed, &folder, &root, false, None);
        assert_eq!(feed.summary_skip, None);
        assert!(!root.exists(), "UI read created guard state or a lock");
        for job in [
            JobState::Queued,
            JobState::Running,
            JobState::BackendError {
                next_try_ms: Some(i64::MAX),
                exit: None,
            },
        ] {
            feed_summary_state(&mut feed, &folder, &root, true, Some(&job));
            assert_eq!(feed.summary_skip, None);
        }
        let receipt = root.join("local/recording-guard/1800000000000-1.json");
        fs::create_dir_all(receipt.parent().unwrap()).unwrap();
        fs::write(&receipt, r#"{"short":true,"kept":false}"#).unwrap();
        fs::write(folder.join("final.pending"), "").unwrap();
        for status in ["transcribing", "interrupted", "recording", "starting"] {
            manifest("complete");
            feed_summary_state(&mut feed, &folder, &root, false, None);
            assert_eq!(feed.summary_skip, Some(SummarySkip::Short));
            manifest(status);
            feed_summary_state(&mut feed, &folder, &root, true, Some(&JobState::Empty));
            assert_eq!(feed.summary_skip, None);
        }
        manifest("complete");
        fs::write(&receipt, r#"{"short":true,"kept":true}"#).unwrap();
        feed_summary_state(&mut feed, &folder, &root, false, None);
        assert_eq!(feed.summary_skip, None);
        assert!(!receipt.with_extension("lock").exists());
    }

    use super::*;

    #[test]
    fn replacing_recording_discards_stop_and_rejects_old_reply() {
        let mut first = Live::scene(Phase::Making, None);
        first.notes_response_time = 17;
        let first_id = first.recording_id;
        let mut current = Some(first);
        drop(current.replace(Live::scene(Phase::Recording, None)));
        let second = current.as_mut().unwrap();
        assert_ne!(second.recording_id, first_id);
        assert_eq!(second.notes_response_time, 0);
        second.notes_response_time = 29;
        assert_eq!(second.take_notes_timing(first_id), 0);
        assert_eq!(second.notes_response_time, 29);
        assert_eq!(second.take_notes_timing(second.recording_id), 29);
    }

    #[test]
    fn panel_budget_preserves_the_document_and_shrinks_transcript_first() {
        for (pane, document) in [(748.0, 340.0), (748.0, 410.0)] {
            let idle = panel_limits(pane, document, false, true, false);
            let answer = panel_limits(pane, document, true, true, false);
            assert!(answer.transcript < idle.transcript);
            assert!(answer.transcript + answer.thread + 160.0 <= answer.space + 0.1);
            assert!(answer.space + document_height(pane, document) <= pane + 0.1);
        }
    }
    #[test]
    fn finishing_page_announces_transcript_progress_once() {
        fn view(s: &State, _: &mut rune::Cx<State>) -> El {
            making_view(s, super::super::theme::palette(false), true)
        }

        let mut state = State::new();
        scene_making(&mut state, "transcribing");
        let mut runtime = rune::Runtime::new(state, view, (360.0, 900.0), rune::Env::default());
        runtime.frame(0.0);
        let elements = runtime.elements();
        let text: Vec<_> = elements.iter().filter_map(|e| e.text.as_deref()).collect();
        let transcript_lines: Vec<_> = text
            .iter()
            .copied()
            .filter(|line| line.contains("transcript"))
            .collect();
        assert_eq!(transcript_lines, ["Finishing the transcript"]);
        assert!(text.contains(&"You can leave this page"));
        assert!(text.contains(&"Recording stopped"));
        assert!(text.contains(&"Back to meetings"));
    }

    #[test]
    fn restart_and_late_summary_preserve_live_notes() {
        use sidenote::{
            core,
            hex_engine::{EngineState, HexError},
        };
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;
        use std::{fs, process::Command};

        // Give State and all of its workers private paths without changing other tests' env.
        if std::env::var_os("SIDENOTE_LIVE_TIMELINE_CHILD").is_none() {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("artifacts")
                .join(format!("live-timeline-{}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "ui::live::writer_tests::restart_and_late_summary_preserve_live_notes",
                    "--nocapture",
                ])
                .env("SIDENOTE_LIVE_TIMELINE_CHILD", "1")
                .env("APPDATA", root.join("appdata"))
                .env("LOCALAPPDATA", root.join("localappdata"))
                .env("USERPROFILE", root.join("home"))
                .env("SIDENOTE_DATA_DIR", root.join("state"))
                .env("SIDENOTE_STATE_DIR", root.join("state"))
                .env("SIDENOTE_HEX_DIR", root.join("meetings"))
                .env("SIDENOTE_PEERS_DIR", root.join("peers"));
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let status = command.status().unwrap();
            assert!(status.success(), "isolated live timeline failed");
            return;
        }
        let root = core::meetings_dir();
        let folder = root.join("1794303362394-45464");
        fs::create_dir_all(&folder).unwrap();
        let manifest = |status: &str| {
            fs::write(
                folder.join("manifest.json"),
                serde_json::to_vec(&serde_json::json!({
                    "status": status, "created_at_ms": 1794303362394_i64
                }))
                .unwrap(),
            )
            .unwrap();
        };
        manifest("recording");
        let mut state = State::new();
        let mut live = Live::scene(Phase::Recording, None);
        live.wall_start = DateTime::from_timestamp_millis(1794303362394 + 1000)
            .unwrap()
            .with_timezone(&Local);
        live.feed.lock().unwrap().hint = Some("1794303362394-45464".into());
        live.folder = Some(folder.clone());
        state.live = Some(live);
        state.live_open = true;
        state.live_notes = TextEdit::new("Typed notes must survive");
        let status = |name: &str| EngineState {
            labs: Default::default(),
            running: true,
            version: "test".into(),
            protocol: 4,
            recording: None,
            meeting_state: name.into(),
            dictating: Some(false),
            capture_state: "idle".into(),
            model: serde_json::from_str("{}").unwrap(),
        };
        manifest("transcribing");
        state.hex.status = Some(Ok(status("transcribing")));
        state.live_sync();
        assert_eq!(state.live.as_ref().unwrap().phase, Phase::Making);
        assert_eq!(making_transcript(&state), Transcript::Finishing);
        assert_eq!(read_transcript_state(&folder), Some(Transcript::Finishing));
        manifest("interrupted");
        fs::write(folder.join("final.pending"), "").unwrap();
        state.hex.status = Some(Err(HexError::HexNotRunning));
        state.live_sync();
        assert_eq!(state.live.as_ref().unwrap().phase, Phase::Making);
        let transcript = read_transcript_state(&folder);
        assert_eq!(transcript, Some(Transcript::Interrupted));
        state.live.as_ref().unwrap().feed.lock().unwrap().transcript = transcript;
        state.hex.status = Some(Ok(status("idle")));
        state.live_sync();
        assert_eq!(state.live.as_ref().unwrap().phase, Phase::Making);
        assert_eq!(making_transcript(&state), Transcript::Interrupted);
        assert_eq!(
            making_transcript(&state).detail(),
            Some("Hex was restarted. It will finish this transcript when it starts again.")
        );
        manifest("complete");
        fs::remove_file(folder.join("final.pending")).unwrap();
        let transcript = read_transcript_state(&folder);
        assert_eq!(transcript, Some(Transcript::Ready));
        state.live.as_ref().unwrap().feed.lock().unwrap().transcript = transcript;
        assert_eq!(
            making_transcript(&state).heading(false),
            "Writing the summary"
        );
        let refresh = |state: &mut State| {
            let options = state.refresh_options();
            state.apply_event(super::super::Event::Refreshed(
                super::super::refresh::Result {
                    generation: core::mutation_generation(),
                    options,
                    data: Some(Box::new(super::super::refresh::Data::read(options))),
                },
            ));
        };
        refresh(&mut state);
        assert!(state.live.is_some());
        state.live.as_mut().unwrap().stopped = Some(Instant::now() - Duration::from_secs(49 * 60));
        state.live.as_mut().unwrap().scene = false;
        state.live.as_mut().unwrap().folder = None;
        assert_eq!(
            making_transcript(&state).heading(true),
            "The summary is taking a while"
        );
        fs::write(
            folder.join("english-summary.json"),
            r#"{"title":"Finished meeting"}"#,
        )
        .unwrap();
        refresh(&mut state);
        assert!(state.live.is_none());
        assert_eq!(state.live_ready.as_deref(), Some("1794303362394-45464"));
        assert_eq!(feed_io::read_notes(&folder), "Typed notes must survive");
        assert_eq!(state.meetings[0].notes, "Typed notes must survive");

        state.live = Some(Live::new(Phase::Making, HashSet::new()));
        state.live.as_mut().unwrap().folder = Some(folder.clone());
        state.live_open = true;
        state.live_notes = TextEdit::new("Notes from the last Stop click");
        state.hex_record_stop(None);
        assert!(
            !state.on_live(),
            "Stop after Hex stopped should release the page"
        );
        assert!(state.live_close());
        assert_eq!(
            feed_io::read_notes(&folder),
            "Notes from the last Stop click"
        );

        // Back leaves the page; completion later must not pull the owner away from the list.
        state.live = Some(Live::new(Phase::Making, HashSet::new()));
        state.live.as_mut().unwrap().folder = Some(folder.clone());
        state.live_open = true;
        state.live_notes = TextEdit::new("Notes saved when leaving");
        state.live_ready = None;
        state.leave_live();
        assert!(!state.on_live());
        assert_eq!(feed_io::read_notes(&folder), "Notes saved when leaving");
        state.live_scanned();
        assert!(state.live.is_none());
        assert!(state.live_ready.is_none());
    }

    #[test]
    fn no_id_never_matches_without_hex_id_or_manifest_start() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("artifacts/live-match-test")
            .join("1794303362394-45464");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("english-summary.json"), "{}").unwrap();
        let mut meeting = sidenote::core::Meeting::load(&root).unwrap();
        let mut live = Live::scene(Phase::Making, None);
        live.wall_start = DateTime::from_timestamp_millis(1794303362394 + 120_000)
            .unwrap()
            .with_timezone(&Local);
        assert!(!live.matches_meeting(&meeting));
        live.wall_start += chrono::Duration::milliseconds(1);
        assert!(!live.matches_meeting(&meeting));
        live.wall_start -= chrono::Duration::milliseconds(1);
        meeting.device = Some("peer".into());
        assert!(!live.matches_meeting(&meeting));
    }

    #[test]
    fn save_warning_tells_the_truth_about_recording_and_backups() {
        for recording in [false, true] {
            for recovered in [false, true] {
                let warning = save_warning(recording, recovered);
                assert_eq!(warning.contains("still recording"), recording);
                assert_eq!(warning.contains("saved a backup copy"), recovered);
                assert!(warning.contains("Copy your notes now"));
                assert!(warning.contains("Do not close Sidenote yet"));
            }
        }
    }

    #[test]
    fn close_requires_the_latest_text_to_be_durable_and_the_queue_drained() {
        let mut live = Live::scene(Phase::Starting, None);
        assert!(live.can_close(""));
        live.saving = Saving::Failed("disk full".into());
        assert!(!live.can_close("draft"));
        live.saved = "older notes".into();
        assert!(!live.can_close(""));
        live.recovered = Some("draft".into());
        assert!(live.can_close("draft"));
        assert!(!live.can_close("newer draft"));
        live.saved = "newer draft".into();
        live.queued.push_back("old draft".into());
        assert!(!live.can_close("newer draft"));
        live.queued.clear();
        assert!(live.can_close("newer draft"));
    }

    #[test]
    fn a_previous_pending_meeting_cannot_replace_the_current_live_meeting() {
        let root = std::env::temp_dir().join(format!(
            "notes-live-match-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("english-summary.json"), "{}").unwrap();
        let mut meeting = sidenote::core::Meeting::load(&root).unwrap();
        let mut live = Live::scene(Phase::Making, None);
        meeting.started = Some(live.wall_start - chrono::Duration::minutes(20));
        assert!(!live.matches_meeting(&meeting));
        meeting.started = Some(live.wall_start);
        assert!(!live.matches_meeting(&meeting));
        live.folder = Some(PathBuf::from("1234-1"));
        assert!(!live.matches_meeting(&meeting));
        meeting.id = "1234-1".into();
        meeting.started = None;
        assert!(live.matches_meeting(&meeting));
        meeting.device = Some("peer".into());
        assert!(!live.matches_meeting(&meeting));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reverting_text_still_flushes_after_an_older_queued_save() {
        let mut live = Live::scene(Phase::Recording, None);
        live.saved = "original".into();
        live.queued.push_back("edited".into());
        assert!(live.needs_save("original"));
        live.queued.push_back("original".into());
        assert!(!live.needs_save("original"));
    }

    #[test]
    fn close_drains_queued_saves_before_stop() {
        let writer = NoteWriter::new(None).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        for message in ["first save", "failed save", "last save", "stop"] {
            let tx = tx.clone();
            writer
                .tx
                .send(Box::new(move || {
                    tx.send(message).unwrap();
                }))
                .unwrap();
        }
        assert!(writer.drain(Duration::from_secs(1)));
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            ["first save", "failed save", "last save", "stop"]
        );
    }

    fn close_fixture() -> (tempfile::TempDir, State) {
        let root = tempfile::tempdir().unwrap();
        let mut state = State::new();
        state.store.root = root.path().to_owned();
        let mut live = Live::new(Phase::Recording, HashSet::new());
        live.folder = Some(root.path().to_owned());
        live.notes_loaded = true;
        state.live = Some(live);
        state.live_notes = TextEdit::new("The latest draft");
        (root, state)
    }

    #[test]
    fn stuck_save_closes_with_an_independent_copy_of_the_latest_text() {
        let (root, mut state) = close_fixture();
        let (release, blocked) = mpsc::channel();
        state.live_work(move || {
            let _ = blocked.recv_timeout(Duration::from_secs(2));
        });
        state.live_notes = TextEdit::new("An earlier draft");
        state.live_flush();
        state.live_notes = TextEdit::new("The latest draft");
        assert!(state.live_close_after(Duration::ZERO));
        let copy = std::fs::read_dir(root.path().join("unsaved-notes"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "The latest draft");
        release.send(()).unwrap();
        assert!(
            state
                .live_writer
                .take()
                .unwrap()
                .drain(Duration::from_secs(1))
        );
        assert_eq!(std::fs::read_to_string(copy).unwrap(), "The latest draft");
    }

    #[test]
    fn panicked_worker_wakes_the_page_and_keeps_a_close_copy() {
        let (root, mut state) = close_fixture();
        let (wake, woken) = mpsc::channel();
        state.waker = Some(Arc::new(move || {
            let _ = wake.send(());
        }));
        state.live_work(|| panic!("simulated writer panic"));
        woken.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(state.live_writer.as_ref().unwrap().stopped());
        assert!(state.live_close_after(Duration::ZERO));
        assert!(matches!(
            &state.live.as_ref().unwrap().saving,
            Saving::Failed(message) if message == WRITER_STOPPED
        ));
        assert_eq!(
            state.live.as_ref().unwrap().recovered.as_deref(),
            Some("The latest draft")
        );
        assert_eq!(
            std::fs::read_dir(root.path().join("unsaved-notes"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn close_after_writer_panic_does_not_wait_for_a_recovery_thread() {
        let (root, mut state) = close_fixture();
        let (wake, woken) = mpsc::channel();
        state.waker = Some(Arc::new(move || {
            let _ = wake.send(());
        }));
        state.live_work(|| panic!("simulated writer panic"));
        woken.recv_timeout(Duration::from_secs(1)).unwrap();

        let closing_thread = thread::current().id();
        let (_release, blocked) = mpsc::channel::<()>();
        let closed = state.live_close_after_with(Duration::ZERO, move |text, path| {
            // Hold an asynchronous recovery past its deadline. Dropping the sender frees it.
            if thread::current().id() != closing_thread && blocked.recv().is_err() {
                return false;
            }
            feed_io::preserve_notes(None, text, path).is_ok()
        });
        assert!(closed);
        let copies = std::fs::read_dir(root.path().join("unsaved-notes")).unwrap();
        let paths: Vec<_> = copies.map(|entry| entry.unwrap().path()).collect();
        assert_eq!(paths.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&paths[0]).unwrap(),
            "The latest draft"
        );
    }

    #[test]
    fn panic_marks_the_next_edit_failed_until_it_is_retried() {
        let (root, mut state) = close_fixture();
        let (wake, woken) = mpsc::channel();
        state.waker = Some(Arc::new(move || {
            let _ = wake.send(());
        }));
        state.live_work(|| panic!("simulated writer panic"));
        woken.recv_timeout(Duration::from_secs(1)).unwrap();
        state.waker = None;
        state.live_notes = TextEdit::new("A new edit");
        assert!(state.live_tick());
        assert!(state.live_writer.is_none());
        assert!(matches!(
            &state.live.as_ref().unwrap().saving,
            Saving::Failed(message) if message == WRITER_STOPPED
        ));
        assert!(state.live_close_after(Duration::from_secs(1)));
        assert_eq!(feed_io::read_notes(root.path()), "A new edit");
    }

    #[test]
    fn closed_channel_uses_recovery_or_keeps_the_notes_on_screen() {
        for blocked in [false, true] {
            let (root, mut state) = close_fixture();
            if blocked {
                std::fs::write(root.path().join("unsaved-notes"), "blocked").unwrap();
            }
            let (tx, rx) = mpsc::channel();
            drop(rx);
            state.live_writer = Some(NoteWriter {
                tx,
                stopped: Arc::new(AtomicBool::new(false)),
            });
            assert_eq!(state.live_close_after(Duration::ZERO), !blocked);
            assert_eq!(state.on_live(), blocked);
            let live = state.live.as_ref().unwrap();
            assert_eq!(live.close_failed, blocked);
            assert_eq!(
                live.recovered.as_deref(),
                (!blocked).then_some("The latest draft")
            );
            assert_eq!(state.live_notes.text, "The latest draft");
            assert!(state.live_writer.is_none());
        }
    }

    #[test]
    fn slow_work_within_the_close_limit_saves_to_the_meeting_folder() {
        let (root, mut state) = close_fixture();
        state.live_work(|| thread::sleep(Duration::from_millis(20)));
        assert!(state.live_close_after(Duration::from_secs(1)));
        assert_eq!(feed_io::read_notes(root.path()), "The latest draft");
        assert!(!root.path().join("unsaved-notes").exists());
    }

    #[test]
    fn missing_source_survives_a_failed_catalog_read() {
        fn catalog_entry(dir: &std::path::Path, id: &str) -> CatalogMeeting {
            let folder = dir.join(id);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("english-summary.json"), "{}").unwrap();
            CatalogMeeting::load(&folder).unwrap()
        }
        fn waiting_state(store_root: &std::path::Path, id: &str) -> State {
            let mut state = State::new();
            state.store.root = store_root.to_owned();
            let mut live = Live::new(Phase::Making, HashSet::new());
            live.folder = Some(PathBuf::from(id));
            state.live = Some(live);
            state.live_open = true;
            state.live_notes = TextEdit::new("");
            state
        }
        let id = "1794303362394-45464";
        let store_root = tempfile::tempdir().unwrap();
        let catalog_dir = tempfile::tempdir().unwrap();
        let entry = catalog_entry(catalog_dir.path(), id);

        // (a) A known refresh that lists the meeting keeps the wait.
        let mut state = waiting_state(store_root.path(), id);
        state.meetings = vec![entry.clone()];
        state.cancel_missing_summary(true, std::slice::from_ref(&entry), &[]);
        assert!(
            state.live.is_some(),
            "a known refresh listing the meeting must not cancel the wait"
        );
        assert!(!state.live.as_ref().unwrap().source_lost);
        assert!(state.live.as_ref().unwrap().source_seen);

        // (b) A failed refresh with an empty catalog does not cancel.
        state.meetings = Vec::new();
        state.cancel_missing_summary(false, &[], &[]);
        assert!(
            state.live.is_some(),
            "a failed catalog read must not cancel the wait"
        );
        assert!(!state.live.as_ref().unwrap().source_lost);

        // (c) The next known refresh without the meeting cancels the wait.
        state.cancel_missing_summary(true, &[], &[]);
        assert!(
            state.live.is_none(),
            "a known refresh missing a previously seen source must cancel the wait"
        );

        // A source that was never seen does not cancel.
        let mut unseen = waiting_state(store_root.path(), id);
        unseen.meetings = Vec::new();
        unseen.cancel_missing_summary(true, &[], &[]);
        assert!(
            unseen.live.is_some(),
            "a source that was never seen must not cancel the wait"
        );
        assert!(!unseen.live.as_ref().unwrap().source_lost);

        // Listed while recording, then the first refresh after Stop fails: still cancels later.
        let mut early = waiting_state(store_root.path(), id);
        early.live.as_mut().unwrap().phase = Phase::Recording;
        early.meetings = vec![entry.clone()];
        early.cancel_missing_summary(true, std::slice::from_ref(&entry), &[]);
        early.live.as_mut().unwrap().phase = Phase::Making;
        early.cancel_missing_summary(false, &[], &[]);
        assert!(
            early.live.is_some(),
            "a failed catalog read must not cancel the wait"
        );
        early.meetings = Vec::new();
        early.cancel_missing_summary(true, &[], &[]);
        assert!(
            early.live.is_none(),
            "a source listed before Stop and lost during a failed read must cancel the wait"
        );

        // A good read that failed to open any folder does not cancel a seen source.
        let mut unread = waiting_state(store_root.path(), id);
        unread.live.as_mut().unwrap().source_seen = true;
        let folder = sidenote::core::meetings_dir().join(id);
        unread.cancel_missing_summary(true, &[], std::slice::from_ref(&folder));
        assert!(
            unread.live.is_some(),
            "a source folder the refresh could not read must not cancel the wait"
        );
        unread.cancel_missing_summary(true, &[], &[PathBuf::from("local/meeting-parts")]);
        assert!(
            unread.live.is_some(),
            "an unreadable meeting-parts folder must not cancel the wait"
        );
    }
}

#[path = "live/parts.rs"]
mod parts;
#[path = "live/parts_scene.rs"]
mod parts_scene;
pub(super) use parts_scene::scene_over as parts_scene_over;
#[path = "live/recording_control.rs"]
mod recording_control;
pub(super) use parts_scene::scene as parts_scene;
pub(super) use recording_control::{
    RecordingStatus, parts_control, recording_note, status as parts_status,
};
