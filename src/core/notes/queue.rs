use super::{
    Engine, Helper, JobState, Language, NotesSettings, Request,
    helper::{self, Failure, Run},
    language, prompt, schema, transcript, translation,
};
use crate::core::failure::{AtStage, Stage};
use crate::core::meeting_job::{self, Stage as JobStage};
use crate::core::{self, agent_instructions::Agent, file_reads, store_io};
use crate::platform::CommandExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Job {
    #[serde(default)]
    pub stage: JobStage,
    #[serde(flatten)]
    pub state: JobState,
    pub attempts: u32,
    pub next_try_at: Option<i64>,
    pub helper: Option<Helper>,
    pub model: String,
    pub instructions: String,
    pub updated_at: i64,
    pub message: String,
    pub request: Option<Request>,
    #[serde(default)]
    pub sorting_pending: bool,
    #[serde(default)]
    pub transcript_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routing_fallbacks: Vec<core::meeting_router::fallback::RoutingFallback>,
    #[serde(default)]
    pub input_revision: Option<String>,
    #[serde(default)]
    pub replace_summary: bool,
    #[serde(default)]
    pub translation: Option<translation::Disposition>,
}
impl Job {
    fn new(request: Option<Request>, settings: &NotesSettings) -> Self {
        Self {
            stage: JobStage::Pipeline,
            state: JobState::Queued,
            attempts: 0,
            next_try_at: None,
            helper: settings.effective_helper(),
            model: settings
                .effective_helper()
                .map(|h| {
                    if settings.model.trim().is_empty() {
                        h.default_model().into()
                    } else {
                        settings.model.clone()
                    }
                })
                .unwrap_or_default(),
            instructions: "default".into(),
            updated_at: now(),
            message: "queued".into(),
            request,
            sorting_pending: false,
            transcript_revision: None,
            routing_fallbacks: Vec::new(),
            input_revision: None,
            replace_summary: false,
            translation: None,
        }
    }
    fn ready(&self, time: i64, root: &Path) -> bool {
        (matches!(self.state, JobState::Queued | JobState::Running)
            && (self.message != "waiting for transcript revision"
                || !core::meeting_router::jobs::notes_enabled(root).unwrap_or(false)
                || self.next_try_at.is_none_or(|at| at <= time)))
            || matches!(self.state, JobState::BackendError {next_try_ms: Some(at), ..} if at <= time)
    }
}
fn job_eligible(root: &Path, meetings: &Path, id: &str, job: &Job) -> io::Result<bool> {
    let source = core::meeting_parts::source(root, meetings, id);
    let meetings = source.as_path();
    if let Some(m) = core::meeting_parts::read(root, id)? {
        let revision = core::recording_source::revision::input(&meetings.join(id))?;
        if m.requested_parts == 0
            || job.request.is_none()
            || job.input_revision.as_ref() != Some(&revision)
        {
            return Ok(false);
        }
    }
    let eligibility = core::recording_guard::eligibility(root, meetings, id)?;
    // Saved chosen revisions and explicit task redo survive missing Hex originals.
    Ok(eligibility == core::recording_guard::Eligibility::Eligible
        || ((job.request == Some(Request::RedoTasks)
            || (job.transcript_revision.is_some()
                && core::meeting_router::jobs::notes_enabled(root)?))
            && eligibility == core::recording_guard::Eligibility::Unknown
            && complete(&meetings.join(id))))
}
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn jobs(root: &Path) -> PathBuf {
    root.join("local/notes-jobs")
}
fn job_path(root: &Path, id: &str) -> PathBuf {
    jobs(root).join(format!("{id}.json"))
}
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
pub(super) fn read(root: &Path, id: &str) -> io::Result<Option<Job>> {
    if !valid_id(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    match store_io::read_bounded(&job_path(root, id), file_reads::JOURNAL_MAX) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::from),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

type JobStamp = (u64, SystemTime, Option<SystemTime>);
#[derive(Default)]
struct ScheduleCache {
    root: PathBuf,
    jobs: BTreeMap<String, (JobStamp, Arc<Job>)>,
}
fn job_stamp(path: &Path) -> io::Result<JobStamp> {
    let meta = fs::symlink_metadata(path)?;
    if !file_reads::regular(&meta) {
        return Err(store_io::invalid("Unsafe notes job"));
    }
    Ok((meta.len(), meta.modified()?, meta.created().ok()))
}
// Only scheduling uses this cache; claiming work still rereads the locked job.
fn scheduled_job(root: &Path, id: &str) -> io::Result<Option<Arc<Job>>> {
    static CACHE: OnceLock<Mutex<ScheduleCache>> = OnceLock::new();
    let path = job_path(root, id);
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| {
            let mut cache = poisoned.into_inner();
            cache.jobs.clear();
            cache
        });
    if cache.root != root {
        cache.root = root.to_owned();
        cache.jobs.clear();
    }
    let before = match job_stamp(&path) {
        Ok(stamp) => stamp,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            cache.jobs.remove(id);
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    if let Some((stamp, job)) = cache.jobs.get(id)
        && *stamp == before
    {
        return Ok(Some(job.clone()));
    }
    cache.jobs.remove(id);
    let job = read(root, id)?.map(Arc::new);
    if let Some(job) = &job
        && job_stamp(&path).is_ok_and(|after| after == before)
    {
        cache.jobs.insert(id.to_owned(), (before, job.clone()));
    }
    Ok(job)
}
#[cfg(any(windows, test))]
pub(crate) fn accepted(root: &Path, meetings: &Path, id: &str) -> io::Result<bool> {
    let revision = core::recording_source::revision::input(&meetings.join(id))?;
    Ok(read(root, id)?.is_some_and(|job| job.input_revision.as_ref() == Some(&revision)))
}
fn write(root: &Path, id: &str, job: &mut Job, state: JobState, message: &str) -> io::Result<()> {
    job.state = state;
    job.updated_at = now();
    job.message = message.into();
    job.next_try_at = match job.state {
        JobState::BackendError { next_try_ms, .. } => next_try_ms,
        _ => None,
    };
    store_io::write(&job_path(root, id), job)?;
    let _ = log(root, id, message);
    Ok(())
}
fn log(root: &Path, id: &str, message: &str) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if crate::service::role() != crate::service::DeviceRole::Full || crate::service::active() {
        return crate::operations::record(root, crate::operations::EventKind::NotesChanged);
    }
    fs::create_dir_all(root)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("sidenote.log"))?;
    writeln!(file, "{} notes: {id} {message}", now())
}
fn stage(root: &Path, id: &str, job: &mut Job, stage: JobStage) -> io::Result<()> {
    job.stage = stage;
    store_io::write(&job_path(root, id), job)
}
pub(super) fn manifest(folder: &Path) -> io::Result<serde_json::Value> {
    let bytes = core::meeting_router::inputs::Snapshot::read_file(
        &folder.join("manifest.json"),
        file_reads::JOURNAL_MAX,
    )?;
    serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
        .map_err(io::Error::from)
}
fn complete(folder: &Path) -> bool {
    manifest(folder).is_ok_and(|m| m["status"] == "complete")
}

pub(super) fn regenerate_in(
    root: &Path,
    meetings: &Path,
    id: &str,
    settings: &NotesSettings,
) -> io::Result<()> {
    enqueue_request(root, meetings, id, Some(Request::TryAgain), settings, true)
}

pub(crate) fn enqueue_in(
    root: &Path,
    meetings: &Path,
    id: &str,
    mode: Option<Request>,
    settings: &NotesSettings,
) -> io::Result<()> {
    enqueue_request(root, meetings, id, mode, settings, false)
}

fn enqueue_request(
    root: &Path,
    meetings: &Path,
    id: &str,
    mode: Option<Request>,
    settings: &NotesSettings,
    confirmed_regeneration: bool,
) -> io::Result<()> {
    let _update_activity = crate::updates::activity()?;
    let Some(_recycle) = core::meeting_delete::claim_write(root, id)? else {
        return Ok(());
    };
    if core::meeting_parts::held_in(root, meetings, id)?
        && meetings != core::meeting_parts::inputs(root)
    {
        return Ok(());
    }
    if !valid_id(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    if settings.engine() != Engine::App {
        if mode.is_some() {
            return Err(store_io::invalid(
                "Turn on built-in Meeting summary in Settings first",
            ));
        }
        return Ok(());
    }
    if mode.is_none()
        && core::recording_guard::eligibility(root, meetings, id)?
            != core::recording_guard::Eligibility::Eligible
    {
        return Ok(());
    }
    let source = core::meeting_parts::source(root, meetings, id);
    let meetings = source.as_path();
    let revision = core::recording_source::revision::input(&meetings.join(id))?;
    let unchanged = |job: &Job| {
        job.input_revision.as_ref() == Some(&revision)
            && (!matches!(job.state, JobState::Done { .. })
                || meetings.join(id).join("english-summary.json").is_file())
    };
    // A running job holds the job lock for its whole run. An existing job that needs no
    // change answers without taking that lock, so enqueue never waits on or fails at it.
    if read(root, id)?.is_some_and(|job| {
        (mode.is_none() && unchanged(&job))
            || (mode.is_some() && matches!(job.state, JobState::Running | JobState::Queued))
    }) {
        return Ok(());
    }
    let _lock = store_io::lock(&jobs(root).join(format!("{id}.lock")))?;
    let previous = read(root, id)?;
    if previous.as_ref().is_some_and(|job| {
        (mode.is_none() && unchanged(job))
            || (mode.is_some() && matches!(job.state, JobState::Running | JobState::Queued))
    }) {
        return Ok(());
    }
    let recovery = mode == Some(Request::UseCodex);
    let mode = if recovery {
        let job = previous
            .as_ref()
            .filter(|job| {
                matches!(job.state, JobState::HelperLimit { .. })
                    && job.helper == Some(Helper::Claude)
            })
            .ok_or_else(|| store_io::invalid("This meeting does not need Codex recovery"))?;
        if settings.effective_helper() != Some(Helper::Codex) || settings.enabled_at_ms.is_none() {
            return Err(store_io::invalid(
                "Choose Codex for summaries before retrying",
            ));
        }
        Some(job.request.unwrap_or(Request::TryAgain))
    } else {
        mode
    };
    let folder = meetings.join(id);
    store_io::reject_reparse(&folder)?;
    if !complete(&folder) {
        return Err(store_io::invalid("Meeting is not complete"));
    }
    let sorting = matches!(mode, Some(Request::SortAgain | Request::RedoTasks));
    let changed = (mode.is_none() || core::meeting_parts::read(root, id)?.is_some())
        && (previous
            .as_ref()
            .is_some_and(|job| job.input_revision.as_ref() != Some(&revision))
            || (previous.is_none() && core::recording_source::input_changed(root, id, &revision)));
    let replace_summary = confirmed_regeneration
        || (core::meeting_parts::read(root, id)?.is_some() && mode.is_some())
        || changed
        || previous
            .as_ref()
            .is_some_and(|job| job.replace_summary && !matches!(job.state, JobState::Done { .. }));
    if meeting_job::preserve_summary(&folder, sorting || replace_summary) {
        let mut preserved = Job::new(None, settings);
        preserved.input_revision = Some(revision);
        preserved.state = JobState::Done {
            fallback: false,
            tasks_failed: !folder.join("tasks.json").is_file(),
        };
        store_io::write(&job_path(root, id), &preserved)?;
        return Ok(());
    }
    if sorting && !folder.join("english-summary.json").is_file() {
        return Err(store_io::invalid("Make notes before sorting tasks"));
    }
    let mut next = Job::new(mode, settings);
    next.input_revision = Some(revision);
    next.replace_summary = replace_summary;
    if !changed {
        next.translation = previous.as_ref().and_then(|job| job.translation.clone());
    }
    if !changed && core::meeting_router::jobs::notes_enabled(root)? {
        next.transcript_revision = previous
            .as_ref()
            .and_then(|j| j.transcript_revision.clone());
        next.routing_fallbacks = previous
            .as_ref()
            .map(|j| j.routing_fallbacks.clone())
            .unwrap_or_default();
    }
    if !recovery && let Some(previous) = previous.filter(|job| job.helper.is_some()) {
        next.helper = previous.helper;
        next.model = previous.model;
    }
    store_io::write(&job_path(root, id), &next)
}

fn failure(root: &Path, id: &str, job: &mut Job, error: Failure) -> io::Result<()> {
    let (state, message) = match error {
        Failure::Busy => return Err(io::ErrorKind::WouldBlock.into()),
        Failure::LinuxUnavailable => (
            JobState::HelperUnavailableOnLinux,
            "HELPER_UNAVAILABLE_LINUX",
        ),
        Failure::Consent => (JobState::NoHelper, "notes permission changed"),
        Failure::Missing => (JobState::HelperMissing, "agent missing"),
        Failure::Limit { exit } => (JobState::HelperLimit { exit }, "HELPER_LIMIT"),
        Failure::Auth { exit } => (JobState::HelperAuth { exit }, "HELPER_AUTH"),
        Failure::TimedOut => (JobState::TimedOut, "agent timed out"),
        Failure::BadOutput => (JobState::BadOutput, "invalid agent output"),
        Failure::Backend { exit } => {
            let delay = match job.attempts {
                1 => Some(15 * 60_000),
                2 => Some(60 * 60_000),
                _ => None,
            };
            (
                JobState::BackendError {
                    next_try_ms: delay.map(|d| now() + d),
                    exit,
                },
                "agent backend error",
            )
        }
    };
    write(root, id, job, state, message)
}
fn text(path: &Path, cap: u64) -> io::Result<String> {
    match core::meeting_router::inputs::Snapshot::read_file(path, cap) {
        Ok(bytes) => {
            String::from_utf8(bytes).map_err(|_| store_io::invalid("Invalid text encoding"))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}
pub(super) fn teams(root: &Path) -> io::Result<Vec<schema::Team>> {
    let path = root.join("teams.json");
    match store_io::read_bounded(&path, file_reads::JOURNAL_MAX) {
        Ok(bytes) => {
            let v: serde_json::Value = serde_json::from_slice(&bytes)?;
            serde_json::from_value(v.get("teams").cloned().unwrap_or(v)).map_err(io::Error::from)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}
fn projects(root: &Path) -> io::Result<Vec<String>> {
    Ok(core::TaskStore::new(root.to_owned())
        .load_projects()?
        .projects)
}
fn consent(settings: &NotesSettings, manual: bool, end: Option<i64>) -> Option<JobState> {
    if settings.engine() != Engine::App
        || settings.effective_helper().is_none()
        || settings.enabled_at_ms.is_none()
    {
        return Some(JobState::NoHelper);
    }
    if !manual
        && (!settings.auto
            || end
                .zip(settings.enabled_at_ms)
                .is_none_or(|(end, enabled)| end <= enabled))
    {
        return Some(JobState::NeedsClick);
    }
    None
}

fn process(
    root: &Path,
    meetings: &Path,
    id: &str,
    job: &mut Job,
    settings: &NotesSettings,
    run: &mut impl Run,
) -> io::Result<()> {
    if !core::meeting_router::jobs::notes_enabled(root)? {
        job.transcript_revision = None;
        job.routing_fallbacks.clear();
    }
    let source = core::meeting_parts::source(root, meetings, id);
    let meetings = source.as_path();
    let folder = meetings.join(id);
    store_io::reject_reparse(&folder)?;
    // Both engines hold this OS lock until publication finishes; a crash releases it immediately.
    let _claim = match meeting_job::claim(&folder) {
        Ok(claim) => claim,
        Err(e)
            if matches!(e.raw_os_error(), Some(32 | 33))
                || e.kind() == io::ErrorKind::WouldBlock =>
        {
            let _ = log(root, id, "notes deferred engine busy");
            return Err(io::ErrorKind::WouldBlock.into());
        }
        Err(e) => return Err(e),
    };
    let _ = log(root, id, &format!("notes engine=app meeting={id}"));
    let m = manifest(&folder)?;
    if m["status"] != "complete" {
        return write(
            root,
            id,
            job,
            JobState::NeedsClick,
            "meeting is not complete",
        );
    }
    let sort_only =
        job.sorting_pending || matches!(job.request, Some(Request::SortAgain | Request::RedoTasks));
    job.input_revision = core::recording_source::revision::input(&folder).ok();
    if meeting_job::preserve_summary(&folder, sort_only || job.replace_summary) {
        return write(
            root,
            id,
            job,
            JobState::Done {
                fallback: false,
                tasks_failed: !folder.join("tasks.json").exists(),
            },
            "existing notes preserved",
        );
    }
    let started = m["created_at_ms"]
        .as_i64()
        .or_else(|| id.split('-').next()?.parse().ok());
    let duration = m["duration_ms"]
        .as_f64()
        .filter(|d| d.is_finite() && *d >= 0.0);
    let end = m["ended_at_ms"].as_i64().or_else(|| {
        started
            .zip(duration)
            .map(|(s, d)| s.saturating_add(d as i64))
    });
    if let Some(state) = consent(
        settings,
        job.request.is_some() || core::recording_guard::is_kept(root, id),
        end,
    ) {
        return write(
            root,
            id,
            job,
            state,
            "notes need setup or an explicit click",
        );
    }
    // Global changes can revoke consent, but never select a different agent for this job.
    let Some(helper) = job.helper.filter(|h| h.offered()) else {
        return failure(root, id, job, Failure::Consent);
    };
    if settings.effective_helper() != Some(helper) {
        return failure(root, id, job, Failure::Consent);
    }
    let mut pinned = settings.clone();
    pinned.helper = Some(helper);
    pinned.model = job.model.clone();
    let settings = &pinned;
    stage(root, id, job, JobStage::Prepare)?;
    let routed = transcript::chosen_sized(
        root,
        id,
        &folder,
        job.transcript_revision.as_deref(),
        end.unwrap_or(job.updated_at),
        &now,
        duration.or_else(|| end.zip(started).map(|(e, s)| e.saturating_sub(s) as f64)),
    )?;
    if let Some(selection) = &routed {
        job.transcript_revision = Some(selection.revision.clone());
        job.routing_fallbacks = selection.reasons.clone();
    }
    if !sort_only {
        let previous = job.translation.as_ref().filter(|state| {
            state.snapshot.as_ref().is_some_and(|snapshot| {
                routed
                    .as_ref()
                    .is_some_and(|selection| selection.revision == snapshot.revision)
            })
        });
        job.translation = Some(translation::prepare(
            root,
            routed.as_ref(),
            previous,
            end.unwrap_or(job.updated_at),
        )?);
    }
    store_io::write(&job_path(root, id), job)?;
    let mut english = None;
    if !sort_only && let Some(state) = job.translation.as_mut() {
        english = translation::invoke(routed.as_ref(), state, run);
        store_io::write(&job_path(root, id), job)?;
    }
    // Status only: how the passes went for the transcript this summary is written from.
    let passes = routed
        .as_ref()
        .map_or(Some(Language::EnglishOnly), |selection| selection.language);
    let passes_revision = routed.as_ref().map(|selection| selection.revision.clone());
    let mut transcript = if let Some(selection) = routed {
        // Translation is all or nothing: a failure leaves the original text in place.
        english.map_or_else(|| selection.render(), |e| selection.render_english(&e))
    } else if job.request == Some(Request::RedoTasks) {
        let existing = text(&folder.join("english-transcript.md"), file_reads::STORE_MAX)?;
        if existing.trim().is_empty() {
            transcript::read(&folder)?
        } else {
            existing
        }
    } else if job.replace_summary {
        let fresh = transcript::read(&folder)?;
        store_io::atomic_write(&folder.join("english-transcript.md"), fresh.as_bytes())?;
        fresh
    } else {
        transcript::write(&folder)?
    };
    if transcript.is_empty() {
        return write(root, id, job, JobState::Empty, "empty transcript");
    }
    let minutes = duration
        .map(|d| (d / 60_000.0).ceil() as u64)
        .unwrap_or_else(|| {
            core::parse_transcript(&transcript)
                .iter()
                .filter_map(|l| l.seconds)
                .max()
                .unwrap_or(0)
                / 60
        });
    if transcript.chars().count() > prompt::TRANSCRIPT_CAP {
        if job.request == Some(Request::FirstPart) {
            transcript = transcript
                .lines()
                .filter(|line| {
                    core::parse_transcript(line)
                        .first()
                        .and_then(|l| l.seconds)
                        .is_some_and(|s| s < 4 * 60 * 60)
                })
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .take(prompt::TRANSCRIPT_CAP)
                .collect();
        } else {
            return write(
                root,
                id,
                job,
                JobState::TooLong { minutes },
                "transcript exceeds size limit",
            );
        }
    }
    let typed = text(&folder.join("notes.md"), file_reads::JOURNAL_MAX)?;
    let glossary = run.glossary();
    if !prompt::bounded_inputs(&typed, &glossary) {
        return write(
            root,
            id,
            job,
            JobState::ContextTooLong {
                typed_notes: typed.chars().count() > prompt::TYPED_CAP,
                glossary: glossary.iter().map(|s| s.chars().count()).sum::<usize>()
                    > prompt::GLOSSARY_CAP,
            },
            "meeting context exceeds size limit",
        );
    }
    let helper = settings
        .effective_helper()
        .ok_or_else(|| store_io::invalid("No agent selected"))?;
    if !run.installed(helper) {
        return failure(root, id, job, Failure::Missing);
    }
    let model = if settings.model.trim().is_empty() {
        helper.default_model()
    } else {
        &settings.model
    };
    job.model = model.into();
    job.attempts = job.attempts.saturating_add(1);
    write(root, id, job, JobState::Running, "running")?;
    let teams = teams(root)?;
    let projects = projects(root)?;
    let date = started
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|d| d.with_timezone(&chrono::Local).date_naive())
        .ok_or_else(|| store_io::invalid("Missing meeting date"))?;
    if job.request == Some(Request::RedoTasks) {
        // The automatic Tasks switch does not apply to an explicit redo.
        return redo_tasks(
            job,
            run,
            Redo {
                root,
                folder: &folder,
                id,
                settings,
                transcript: &transcript,
                typed: &typed,
                date,
                teams: &teams,
                projects: &projects,
            },
        );
    }
    let edits = core::summary_edits::Store::new(root, &folder, id);
    let (summary, fallback) = if sort_only {
        let mut summary = edits.generated()?;
        let replacement = job.sorting_pending && job.replace_summary;
        if !replacement {
            edits.overlay(&mut summary)?;
        }
        (
            existing_summary(summary)?,
            replacement && job.instructions == "default-fallback",
        )
    } else {
        if !folder.join("quick-title.json").exists() {
            stage(root, id, job, JobStage::Title)?;
            let title_model = if helper == Helper::Claude {
                "claude-haiku-5-5"
            } else {
                model
            };
            match run
                .run(
                    helper,
                    title_model,
                    prompt::quick(root, &transcript),
                    helper::TITLE_TIMEOUT,
                )
                .ok()
                .and_then(|s| prompt::clean_quick(&s))
            {
                Some(title) => {
                    let _ = store_io::publish(
                        &folder.join("quick-title.json"),
                        &serde_json::to_vec(&serde_json::json!({"title":title}))?,
                    );
                }
                None => {
                    let _ = log(root, id, "quick title failed");
                }
            }
        }
        let title = core::read_quick_title(&folder)
            .or_else(|| m["title"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "not provided".into());
        let mut material = prompt::material(
            prompt::Details {
                title: &title,
                started_ms: started.unwrap_or(0),
                minutes,
                your_name: &settings.your_name,
            },
            &typed,
            &transcript,
            &teams,
            &glossary,
        );
        if job.replace_summary {
            match edits.generated() {
                Ok(previous) => {
                    prompt::append_corrections(&mut material, &edits.corrections(&previous)?)?
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        let call = prompt::Call {
            source: Some((&folder, id)),
            base_override: crate::core::UiPrefs::load(root)
                .meeting_summary_templates
                .get(id)
                .filter(|choice| choice.as_str() == "standard")
                .map(|_| {
                    (
                        crate::core::agent_instructions::default_base(Agent::Notes).into(),
                        false,
                    )
                }),
            root,
            helper,
            model,
            timeout: helper::NOTES_TIMEOUT,
        };
        stage(root, id, job, JobStage::Notes)?;
        let answer = match call.ladder(run, Agent::Notes, &material, |raw| {
            schema::parse(raw, id, date, &transcript, &teams)
        }) {
            Ok(answer) => answer,
            Err(error) => return failure(root, id, job, error),
        };
        job.instructions = if answer.fallback {
            "default-fallback"
        } else if answer.custom {
            "custom"
        } else {
            "default"
        }
        .into();
        let mut summary = answer.value;
        let mut made_by = prompt::provenance(helper, model, answer.custom, answer.fallback);
        if let Some(passes) = passes {
            language::stamp(&mut made_by, passes, passes_revision.as_deref());
        }
        summary.made_by = Some(made_by);
        let bytes = serde_json::to_vec_pretty(&summary)?;
        // Publication also preserves a summary written before this engine acquired the lock.
        let settle = core::settle::Store::new(
            root.to_owned(),
            folder
                .parent()
                .ok_or_else(|| store_io::invalid("Missing meetings folder"))?
                .to_owned(),
        );
        stage(root, id, job, JobStage::Publish)?;
        match settle.publish_summary(id, &serde_json::from_slice(&bytes)?, !job.replace_summary) {
            Ok(()) => job.sorting_pending = job.replace_summary,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return write(
                    root,
                    id,
                    job,
                    JobState::Done {
                        fallback: false,
                        tasks_failed: true,
                    },
                    "existing notes preserved",
                );
            }
            Err(e) => return Err(e),
        }
        (summary, answer.fallback)
    };
    let tasks_enabled = match core::Config::load_in(root) {
        Ok(config) => config.engine_settings.steps.tasks,
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Err(e),
        Err(_) => {
            return write(
                root,
                id,
                job,
                JobState::Done {
                    fallback,
                    tasks_failed: true,
                },
                "task settings could not be read",
            );
        }
    };
    if !tasks_enabled {
        return write(
            root,
            id,
            job,
            JobState::Done {
                fallback,
                tasks_failed: false,
            },
            "done",
        );
    }
    // Preserve the exact summary on a sorting retry; only tasks.json is replaced.
    let material = prompt::task_material(&summary, &projects)?;
    let call = prompt::Call {
        base_override: None,
        source: Some((&folder, id)),
        root,
        helper,
        model,
        timeout: helper::TASKS_TIMEOUT,
    };
    stage(root, id, job, JobStage::Tasks)?;
    let answer = call.ladder(run, Agent::Tasks, &material, |raw| {
        schema::sorting(raw, &summary, &projects, &settings.your_name)
    });
    let tasks_failed = match answer {
        Err(Failure::Busy) => return Err(io::ErrorKind::WouldBlock.into()),
        Ok(answer) => {
            if answer.fallback {
                let _ = log(root, id, "tasks used default fallback");
            }
            core::settle::Store::new(root.to_owned(), folder.parent().unwrap().to_owned())
                .publish_projects(id, &summary.title, &serde_json::to_value(&answer.value)?)
                .is_err()
        }
        Err(_) => {
            let _ = log(root, id, "task sorting failed");
            true
        }
    };
    write(
        root,
        id,
        job,
        JobState::Done {
            fallback,
            tasks_failed,
        },
        if tasks_failed {
            "done; task sorting failed"
        } else {
            "done"
        },
    )
}

/// All I/O is rooted explicitly, so tests never need the real app directories.
#[cfg(test)]
pub(super) fn drain(
    root: &Path,
    meetings: &Path,
    run: &mut impl Run,
    load: impl Fn() -> io::Result<NotesSettings>,
) -> io::Result<()> {
    let _worker = match store_io::lock(&jobs(root).join("worker.lock")) {
        Ok(lock) => lock,
        Err(e)
            if (cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)))
                || (cfg!(unix) && e.kind() == io::ErrorKind::WouldBlock) =>
        {
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    drain_locked(root, meetings, run, load)
}
pub(super) fn drain_locked(
    root: &Path,
    meetings: &Path,
    run: &mut impl Run,
    load: impl Fn() -> io::Result<NotesSettings>,
) -> io::Result<()> {
    loop {
        if crate::service::stopping() {
            return Ok(());
        }
        let Some(settings) = store_io::defer_busy(load())? else {
            return Ok(());
        };
        if settings.engine() != Engine::App {
            return Ok(());
        }
        let mut ready = Vec::new();
        let mut missing = Vec::new();
        let suppressed = core::meeting_delete::suppressed_ids(root)?;
        store_io::reject_reparse(&jobs(root))?;
        for entry in fs::read_dir(jobs(root))? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|s| s != "json") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !valid_id(id) || suppressed.contains(id) {
                continue;
            }
            match scheduled_job(root, id) {
                Ok(Some(job)) if job.ready(now(), root) => {
                    if job_eligible(root, meetings, id, &job)? {
                        ready.push(id.to_owned());
                    }
                }
                Ok(Some(job)) if job.state == JobState::HelperMissing => {
                    missing.push((id.to_owned(), job.as_ref().clone()))
                }
                Err(_) => {
                    let Some(_recycle) = core::meeting_delete::claim_write(root, id)? else {
                        continue;
                    };
                    let target = path.with_extension(format!(
                        "json.{}.quarantined",
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
                    ));
                    let message = if fs::rename(&path, target).is_ok() {
                        "unreadable job quarantined"
                    } else {
                        "unreadable job skipped; quarantine could not be written"
                    };
                    let _ = log(root, id, message);
                }
                _ => {}
            }
        }
        if !missing.is_empty()
            && !ready.is_empty()
            && settings
                .effective_helper()
                .is_some_and(|h| run.installed(h))
        {
            for (id, mut job) in missing {
                let Some(_recycle) = core::meeting_delete::claim_write(root, &id)? else {
                    continue;
                };
                write(
                    root,
                    &id,
                    &mut job,
                    JobState::Queued,
                    "rechecking agent for new meeting",
                )?;
                ready.push(id);
            }
        }
        ready.sort_by_key(|id| {
            (
                id.split('-')
                    .next()
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(0),
                id.clone(),
            )
        });
        let Some(id) = ready.into_iter().next() else {
            return Ok(());
        };
        let Some(_recycle) = core::meeting_delete::claim_write(root, &id)? else {
            continue;
        };
        let _job_lock = store_io::lock(&jobs(root).join(format!("{id}.lock")))?;
        let Some(mut job) = read(root, &id)? else {
            continue;
        };
        if !job_eligible(root, meetings, &id, &job)? {
            continue;
        }
        if !job.ready(now(), root) {
            continue;
        }
        let mut before = job.clone();
        let result = process(root, meetings, &id, &mut job, &settings, run);
        if matches!(&result, Err(e) if e.kind() == io::ErrorKind::WouldBlock) {
            let routing_pending = result
                .as_ref()
                .is_err_and(|e| store_io::message(e) == Some("routing_pending"));
            before.transcript_revision = job.transcript_revision;
            before.routing_fallbacks = job.routing_fallbacks;
            before.translation = job.translation;
            if routing_pending {
                before.next_try_at = Some(now().saturating_add(1000));
                before.message = "waiting for transcript revision".into();
            }
            before.sorting_pending |= job.sorting_pending
                || (!before.replace_summary
                    && before.request != Some(Request::RedoTasks)
                    && meetings.join(&id).join("english-summary.json").is_file());
            if before.replace_summary && job.sorting_pending {
                before.instructions = job.instructions;
            }
            store_io::write(&job_path(root, &id), &before)?;
            if routing_pending {
                continue;
            }
            return Ok(());
        }
        if result.is_err() {
            // Filesystem and parse errors are intentionally reduced to a fixed message.
            write(
                root,
                &id,
                &mut job,
                JobState::BackendError {
                    next_try_ms: None,
                    exit: None,
                },
                "local notes data could not be read or written",
            )?;
        }
    }
}

pub fn enqueue(id: &str) -> io::Result<()> {
    let Some(settings) = store_io::defer_busy(super::settings())? else {
        return Ok(());
    };
    enqueue_in(
        &core::state_dir(),
        &core::meeting_parts::source(&core::state_dir(), &core::meetings_dir(), id),
        id,
        None,
        &settings,
    )
}
pub fn catch_up() -> io::Result<()> {
    let _stage = core::idle_pass::Span::begin("notes_catch_up");
    let Some(settings) = store_io::defer_busy(super::settings()).at(Stage::NotesSettingsRead)?
    else {
        return Ok(());
    };
    if settings.engine() != Engine::App {
        return Ok(());
    }
    #[cfg(windows)]
    let delivered =
        core::recording_source::delivered(&core::state_dir().join("local/recording-source.json"))
            .unwrap_or_default();
    #[cfg(windows)]
    store_io::reject_reparse(&core::meetings_dir())?;
    #[cfg(windows)]
    let root_meta = fs::metadata(core::meetings_dir())?;
    let mut failure = None;
    for entry in fs::read_dir(core::meetings_dir()).at(Stage::CatchUpMeetings)? {
        let entry = entry.at(Stage::CatchUpMeetings)?;
        let id = entry.file_name().to_string_lossy().into_owned();
        #[cfg(windows)]
        let directory_meta = entry.metadata().ok();
        #[cfg(windows)]
        if valid_id(&id)
            && directory_meta.as_ref().is_some_and(|meta| {
                core::recording_source::revision::input_with_metadata(
                    &entry.path(),
                    &root_meta,
                    meta,
                )
                .is_ok_and(|revision| delivered.get(&id) == Some(&revision))
            })
            && ((directory_meta
                .as_ref()
                .is_some_and(|meta| meta.is_dir() && !crate::platform::linked(meta))
                && meeting_job::preserve_summary(&entry.path(), false))
                || core::recording_guard::receipt(&core::state_dir(), &id)
                    == core::recording_guard::Eligibility::Short)
        {
            continue;
        }
        if valid_id(&id)
            && let Err(error) = enqueue_in(
                &core::state_dir(),
                &core::meetings_dir(),
                &id,
                None,
                &settings,
            )
            .at(Stage::CatchUpEnqueue)
        {
            failure.get_or_insert(error);
        }
    }
    start_worker()?;
    failure.map_or(Ok(()), Err)
}
pub fn start_worker() -> io::Result<()> {
    let _stage = core::idle_pass::Span::begin("notes_queue");
    let Some(settings) = store_io::defer_busy(super::settings()).at(Stage::NotesSettingsRead)?
    else {
        return Ok(());
    };
    if settings.engine() != Engine::App {
        return Ok(());
    }
    let root = core::state_dir();
    let meetings = core::meetings_dir();
    start_worker_in(&root, &meetings, now(), launch_worker)
}

pub(super) fn start_worker_in(
    root: &Path,
    meetings: &Path,
    time: i64,
    launch: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    store_io::reject_reparse(&jobs(root))?;
    let suppressed = core::meeting_delete::suppressed_ids(root)?;
    if !fs::read_dir(jobs(root)).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            let path = e.path();
            if path.extension().is_none_or(|s| s != "json") {
                return false;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                return false;
            };
            if !valid_id(id) || suppressed.contains(id) {
                return false;
            }
            match scheduled_job(root, id) {
                Ok(Some(job)) => {
                    job.ready(time, root) && job_eligible(root, meetings, id, &job).unwrap_or(false)
                }
                Err(_) => true,
                Ok(None) => false,
            }
        })
    }) {
        return Ok(());
    }
    launch()
}

fn launch_worker() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if crate::service::active() {
        return crate::service::workers::start("notes", || {
            let _ = run_pending();
        });
    }
    let child = crate::platform::worker_command(std::env::current_exe().at(Stage::NotesExePath)?)?
        .arg("--notes-pending")
        .arg("--data-dir")
        .arg(core::state_dir())
        .arg("--meetings-dir")
        .arg(core::meetings_dir())
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .at(Stage::NotesWorkerSpawn)?;
    crate::service::own_child(child)
}
pub fn worker_running(root: &Path) -> bool {
    core::meeting_delete::lock_busy(&jobs(root).join("worker.lock")).unwrap_or(false)
}

pub fn run_pending() -> io::Result<()> {
    let root = core::state_dir();
    let meetings = core::meetings_dir();
    let _worker = match store_io::lock(&jobs(&root).join("worker.lock")) {
        Ok(lock) => lock,
        Err(e)
            if (cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)))
                || (cfg!(unix) && e.kind() == io::ErrorKind::WouldBlock) =>
        {
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let _activity = match crate::updates::activity() {
        Ok(activity) => activity,
        Err(error) if core::meeting_delete::busy(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    core::settle::Store::default().recover()?;
    drain_locked(
        &root,
        &meetings,
        &mut helper::Cli {
            root: Some(root.clone()),
        },
        super::settings,
    )
}

fn existing_summary(mut value: serde_json::Value) -> io::Result<schema::Summary> {
    for todo in value["todo"].as_array_mut().into_iter().flatten() {
        if todo.get("title").is_none() {
            todo["title"] = todo["task"].clone();
        }
    }
    // Older notes omitted empty sections.
    for key in ["context", "topics", "todo", "to_settle"] {
        if value.get(key).is_none() {
            value[key] = serde_json::json!([]);
        }
    }
    serde_json::from_value(value).map_err(io::Error::from)
}
struct Redo<'a> {
    root: &'a Path,
    folder: &'a Path,
    id: &'a str,
    settings: &'a NotesSettings,
    transcript: &'a str,
    typed: &'a str,
    date: chrono::NaiveDate,
    teams: &'a [schema::Team],
    projects: &'a [String],
}
fn redo_tasks(job: &mut Job, run: &mut impl Run, redo: Redo<'_>) -> io::Result<()> {
    let Redo {
        root,
        folder,
        id,
        settings,
        transcript,
        typed,
        date,
        teams,
        projects,
    } = redo;
    let path = folder.join("english-summary.json");
    let raw = text(&path, file_reads::SUMMARY_MAX)?;
    let mut source: serde_json::Value = serde_json::from_str(&raw)?;
    let old: core::Summary = serde_json::from_value(source.clone())?;
    let mut view = source.clone();
    core::summary_edits::Store::new(root, folder, id).overlay(&mut view)?;
    let material = format!(
        "Extract the tasks from this meeting. Only redo the tasks.\nTeams: {}\nTranscript:\n{}\nExisting notes:\n{}\nUser notes:\n{}",
        serde_json::to_string(teams)?,
        transcript,
        view,
        typed
    );
    let helper = settings
        .effective_helper()
        .ok_or_else(|| store_io::invalid("No agent selected"))?;
    let model = if settings.model.trim().is_empty() {
        helper.default_model()
    } else {
        &settings.model
    };
    let call = prompt::Call {
        base_override: None,
        root,
        helper,
        model,
        timeout: helper::TASKS_TIMEOUT,
        source: Some((folder, id)),
    };
    stage(root, id, job, JobStage::Tasks)?;
    let answer = match call.ladder_output(run,Agent::Tasks,&material,|raw| {
        let tasks: serde_json::Value = schema::object(raw)?;
        let todo = tasks.get("todo").filter(|v| v.is_array()).ok_or("missing tasks list")?;
        let value = serde_json::json!({"title":"Tasks", "context":[], "topics":[], "to_settle":[], "todo":todo});
        schema::parse(&value.to_string(),id,date,transcript,teams)
    },Some(include_str!("tasks_redo_tail.txt"))) {
        Ok(answer) => answer,
        Err(e) => return failure(root,id,job,e),
    };
    source["todo"] = serde_json::to_value(&answer.value.todo)?;
    // Do not overwrite notes that changed while the helper was running.
    if text(&path, file_reads::SUMMARY_MAX)? != raw {
        return Err(store_io::invalid("Notes changed while tasks were running"));
    }
    let new: core::Summary = serde_json::from_value(source.clone())?;
    core::TaskStore::new(root.to_owned()).preserve_redo(id, &old.todo, &new.todo)?;
    core::settle::Store::new(root.to_owned(), folder.parent().unwrap().to_owned())
        .publish_summary(id, &source, false)?;
    let sorted = call.ladder(
        run,
        Agent::Tasks,
        &prompt::task_material(&answer.value, projects)?,
        |raw| schema::sorting(raw, &answer.value, projects, &settings.your_name),
    );
    let tasks_failed = match sorted {
        Err(Failure::Busy) => return Err(io::ErrorKind::WouldBlock.into()),
        Ok(sorted) => {
            core::settle::Store::new(root.to_owned(), folder.parent().unwrap().to_owned())
                .publish_projects(
                    id,
                    &answer.value.title,
                    &serde_json::to_value(&sorted.value)?,
                )
                .is_err()
        }
        Err(_) => true,
    };
    write(
        root,
        id,
        job,
        JobState::Done {
            fallback: answer.fallback,
            tasks_failed,
        },
        "tasks redone",
    )
}

/// Publication must belong to this joined input, rather than the previous notes job.
pub fn input_finished(root: &Path, folder: &Path, id: &str) -> io::Result<bool> {
    let revision = core::recording_source::revision::input(folder)?;
    Ok(read(root, id)?.is_some_and(|job| {
        matches!(job.state, JobState::Done { .. }) && job.input_revision.as_ref() == Some(&revision)
    }))
}

pub(crate) fn recycle_busy(root: &Path, id: &str) -> io::Result<bool> {
    Ok(active(root, id)?
        || core::meeting_delete::lock_busy(&jobs(root).join(format!("{id}.lock")))?)
}

pub fn active(root: &Path, id: &str) -> io::Result<bool> {
    Ok(
        read(root, id)?
            .is_some_and(|job| matches!(job.state, JobState::Queued | JobState::Running)),
    )
}
