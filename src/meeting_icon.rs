//! A lazy, native listener overlay. No Rune application or GPU device is initialized here.
mod native;
mod nub;
use crate::core::meeting_job::Cause;
use crate::core::{self, file_reads, store_io};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[cfg(windows)]
pub const WINDOW_NUB_ARG: &str = "--open-from-meeting-icon=";
#[cfg(windows)]
pub fn open_window_message() -> u32 {
    use windows_sys::{Win32::UI::WindowsAndMessaging::RegisterWindowMessageW, w};
    unsafe { RegisterWindowMessageW(w!("Sidenote.OpenFromMeetingIcon.v1")) }
}

#[cfg(any(windows, test))]
pub fn can_restore(placed: [i32; 4], current: Option<[i32; 4]>) -> bool {
    current == Some(placed)
}

#[cfg(any(windows, test))]
pub fn beside(work: [i32; 4], icon: [i32; 4], size: [i32; 2], gap: i32) -> [i32; 4] {
    let [left, top, right, bottom] = work;
    let [x, y, icon_width, _] = icon;
    let width = size[0].min(right - left);
    let height = size[1].min(bottom - top);
    let right_of_icon = x + icon_width + gap;
    let x = if right - right_of_icon >= width || right - right_of_icon >= x - gap - left {
        right_of_icon
    } else {
        x - gap - width
    };
    let x = x.clamp(left, right - width);
    let y = y.clamp(top, bottom - height);
    [x, y, width, height]
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct Snapshot {
    id: Option<String>,
    recording: bool,
    speaking: bool,
    elapsed: u64,
    seen: bool,
    now_ms: Option<i64>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct Pending {
    elapsed: u64,
    #[serde(default)]
    stopped_at_ms: Option<i64>,
    #[serde(default)]
    skipped_at_ms: Option<i64>,
    #[serde(default)]
    notice_at_ms: Option<i64>,
    #[serde(skip)]
    notice_since: Option<Instant>,
    #[serde(skip)]
    stopped_since: Option<Instant>,
    #[serde(skip)]
    skipped_since: Option<Instant>,
}

// Ten minutes allows queued notes/retries without pinning GUI resources when notes never arrive.
const NO_DRAFT_BOUND_MS: i64 = 10 * 60 * 1000;
const SKIPPED_NOTICE_MS: i64 = 3000;
const SUMMARY_NOTICE_MS: i64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Notice {
    Writing,
    Later,
    Failed(Cause),
    Skipped(core::notes::SummarySkip),
    Unavailable,
}
impl Notice {
    fn text(self) -> String {
        match self {
            Self::Writing => "Writing summary...",
            Self::Later => "Summary will be written later",
            Self::Failed(cause) => cause.label(),
            Self::Skipped(reason) => return reason.notice(),
            Self::Unavailable => "Summary unavailable. Open Sidenote.",
        }
        .into()
    }
}
fn summary_skip(id: &str) -> Option<core::notes::SummarySkip> {
    core::notes::summary_skip(
        core::recording_guard::receipt(&core::state_dir(), id),
        core::notes::scripts_policy() == "off",
        core::notes::status(id).as_ref(),
    )
}
fn notice(id: &str) -> Notice {
    use core::notes::JobState;
    if let Some(skipped) = summary_skip(id) {
        return Notice::Skipped(skipped);
    }
    let policy = core::notes::scripts_policy();
    match policy {
        "app" | "scripts" | "off" => {}
        _ => return Notice::Unavailable,
    }
    match core::notes::status(id) {
        Some(JobState::Queued | JobState::Running) => return Notice::Writing,
        Some(JobState::BackendError {
            next_try_ms: Some(_),
            ..
        }) => return Notice::Later,
        _ => {}
    }
    if let Some(cause) = core::meeting_job::script_failure(&core::meetings_dir().join(id))
        .or_else(|| core::notes::status(id).and_then(|s| s.failure_cause()))
    {
        return Notice::Failed(cause);
    }
    let deferred = store_io::read::<String>(
        &core::state_dir()
            .join("local/draft-outcomes")
            .join(format!("{id}.json")),
    )
    .is_ok_and(|s| s == "deferred");
    if deferred
        || matches!(
            core::notes::status(id),
            Some(JobState::NoHelper | JobState::NeedsClick)
        )
    {
        Notice::Later
    } else if matches!(core::notes::status(id), Some(JobState::Queued))
        || (matches!(core::notes::status(id), Some(JobState::Running))
            && claim_busy(&core::state_dir().join("local/notes-jobs/worker.lock")))
        || claim_busy(
            &core::meetings_dir()
                .join(id)
                .join(&core::meeting_job::contract().lock_file),
        )
        || claim_busy(
            &core::state_dir()
                .join("local/draft-summaries")
                .join(format!("{id}.lock")),
        )
    {
        Notice::Writing
    } else {
        Notice::Unavailable
    }
}
fn claim_busy(path: &std::path::Path) -> bool {
    path.is_file()
        && store_io::lock(path).is_err_and(|e| {
            matches!(e.raw_os_error(), Some(32 | 33)) || e.kind() == std::io::ErrorKind::WouldBlock
        })
}
fn dismissed_path(id: &str) -> PathBuf {
    core::state_dir()
        .join("local/floating-icon-dismissed")
        .join(format!("{id}.json"))
}
fn dismiss_notice(id: &str) {
    if valid_id(id) {
        let _ = store_io::write(&dismissed_path(id), &chrono::Utc::now().timestamp_millis());
    }
}
fn notice_fresh(at: i64, now: i64, dismissed: Option<i64>) -> bool {
    now.saturating_sub(at) < SUMMARY_NOTICE_MS && dismissed.is_none_or(|d| d < at)
}

fn retain_notice(
    p: &mut Pending,
    state: Notice,
    now: i64,
    expired: bool,
    dismissed: Option<i64>,
) -> bool {
    if state == Notice::Writing && !expired {
        p.notice_at_ms = None;
        p.notice_since = None;
        return true;
    }
    let at = *p.notice_at_ms.get_or_insert(now);
    let since = *p.notice_since.get_or_insert_with(Instant::now);
    notice_fresh(at, now, dismissed)
        && since.elapsed() < Duration::from_millis(SUMMARY_NOTICE_MS as u64)
}

/// Render the native indicator without creating windows or starting a listener.
#[doc(hidden)]
pub fn render_synthetic() -> std::io::Result<()> {
    if !core::is_proof_path(&core::state_dir(), "meeting-indicator-proof") {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    std::fs::create_dir_all(core::state_dir().join("local"))?;
    let selected = notice("1800000000000-1");
    let mut pending = Pending {
        elapsed: 120,
        stopped_at_ms: Some(1000),
        ..Pending::default()
    };
    let first = retain_notice(&mut pending, selected, 1000, false, None);
    let after_30s = retain_notice(&mut pending, selected, 31_000, false, None);
    store_io::write(
        &core::state_dir().join("local/notice-selection.json"),
        &serde_json::json!({"notice":format!("{selected:?}"),"initial":first,"after_30s":after_30s}),
    )?;
    native::render_synthetic()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

fn pending_path() -> PathBuf {
    core::state_dir().join("local/floating-meetings.json")
}

fn seen_path(id: &str) -> PathBuf {
    core::state_dir()
        .join("local/floating-icon-seen")
        .join(format!("{id}.json"))
}

/// Called only when a local meeting's actual draft/final content is displayed.
pub fn draft_seen(id: &str, raw: &str) {
    if valid_id(id) && !raw.is_empty() && !was_seen(id, raw) {
        let digest = blake3::hash(raw.as_bytes()).to_hex().to_string();
        if store_io::write(&seen_path(id), &digest).is_err() {
            crate::listener::log("could not acknowledge the floating meeting draft");
        }
    }
}

fn summary(id: &str) -> Option<String> {
    let final_path = core::meetings_dir().join(id).join("english-summary.json");
    let path = if final_path.is_file() {
        final_path
    } else if matches!(notice(id), Notice::Failed(_)) {
        return None;
    } else {
        core::state_dir()
            .join("local/draft-summaries")
            .join(format!("{id}.json"))
    };
    file_reads::text(&path, file_reads::SUMMARY_MAX)
        .ok()
        .filter(|s| {
            serde_json::from_str::<core::Summary>(s).is_ok_and(|s| !s.title.trim().is_empty())
        })
}

fn was_seen(id: &str, raw: &str) -> bool {
    store_io::read::<String>(&seen_path(id))
        .is_ok_and(|s| s == blake3::hash(raw.as_bytes()).to_hex().as_str())
}

fn card_text(id: &str, recording: bool, raw: Option<&str>) -> String {
    if let Some(raw) = raw
        && let Ok(s) = serde_json::from_str::<core::Summary>(raw)
    {
        let mut lines = vec![s.title];
        lines.extend(s.context);
        for topic in s.topics {
            lines.push(topic.title);
            lines.extend(topic.points.into_iter().map(|p| format!("  {}", p.text)));
        }
        lines.extend(s.todo.into_iter().map(|t| format!("To do: {}", t.text())));
        lines.extend(s.to_settle.into_iter().map(|q| q.question));
        return lines.join("\r\n\r\n");
    }
    let folder = core::meetings_dir().join(id);
    let text = match core::live::read_rolling(&folder, None) {
        Ok(core::live::Read::Lines(lines, _)) => lines
            .iter()
            .rev()
            .take(12)
            .rev()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\r\n\r\n"),
        _ => String::new(),
    };
    format!(
        "{}\r\n\r\n{text}",
        if recording {
            "Meeting is recording. Live words may be a few seconds behind."
        } else {
            "Recording saved. Waiting for meeting notes."
        }
    )
}

pub(crate) fn start(isolated: bool) {
    let replay = isolated && std::env::args().any(|a| a == "--floating-icon-replay");
    if let Err(e) = std::thread::Builder::new()
        .name("meeting-icon".into())
        .spawn(move || run(isolated, replay))
    {
        crate::listener::log(&format!("floating icon could not start ({})", e.kind()));
    }
}

fn frame_for(
    current: &Snapshot,
    id: &str,
    pending: &Pending,
    speech_until: Instant,
    transcript_revision: u64,
) -> native::Frame {
    let recording = current.recording && current.id.as_deref() == Some(id);
    let raw = summary(id);
    native::Frame {
        id: id.to_owned(),
        recording,
        pill: recording && (current.speaking || Instant::now() < speech_until),
        elapsed: pending.elapsed,
        notice: (!recording && raw.is_none()).then(|| {
            let state = notice(id);
            if state == Notice::Writing && pending.notice_at_ms.is_some() {
                Notice::Later
            } else {
                state
            }
        }),
        raw,
        transcript_revision,
        skipped: pending.skipped_at_ms.is_some(),
    }
}

fn run(isolated: bool, replay: bool) {
    let engine = crate::hex_engine::HexEngine::new();
    let root = core::state_dir();
    let mut pending: BTreeMap<String, Pending> =
        store_io::read(&pending_path()).unwrap_or_default();
    pending.retain(|id, _| valid_id(id));
    let mut host: Option<native::Host> = None;
    let mut current = Snapshot::default();
    let mut stamp = None;
    let mut last_words = 0;
    let mut speech_until = Instant::now();
    let mut next_status = Instant::now();
    let mut transcript_revision = 0u64;
    let mut last_diagnostics = None;
    while !crate::updates::listener_stopping() {
        if replay {
            let status = native::diagnostics();
            if last_diagnostics != Some(status) {
                let _ = store_io::write(
                    &core::state_dir().join("local/floating-icon-workers.json"),
                    &status,
                );
                last_diagnostics = Some(status);
            }
        }
        if replay && core::state_dir().join("local/floating-icon-exit").is_file() {
            // Only the explicitly isolated replay process accepts this bounded shutdown.
            crate::listener::log("exit reason=stop source=floating-icon-replay");
            std::process::exit(0);
        }
        if Instant::now() >= next_status {
            next_status = Instant::now() + Duration::from_millis(if replay { 100 } else { 1500 });
            let update = if replay {
                file_reads::text(
                    &core::state_dir().join("local/floating-icon-replay.json"),
                    4096,
                )
                .ok()
                .and_then(|s| serde_json::from_str::<Snapshot>(&s).ok())
            } else if isolated {
                Some(Snapshot::default())
            } else {
                engine.status().ok().and_then(|status| {
                    if !matches!(
                        status.meeting_state.as_str(),
                        "recording" | "idle" | "stopped" | "complete" | "finished"
                    ) {
                        return None;
                    }
                    let id = core::meeting_watch::recording_id(&status, &core::meetings_dir());
                    if status.meeting_state == "recording" && id.is_none() {
                        return None;
                    }
                    Some(Snapshot {
                        recording: id.is_some(),
                        id,
                        elapsed: status
                            .recording
                            .as_ref()
                            .and_then(|r| r.elapsed_seconds)
                            .unwrap_or(0),
                        speaking: false,
                        ..Snapshot::default()
                    })
                })
            };
            if let Some(mut update) = update {
                update.id = update.id.filter(|id| valid_id(id));
                update.recording &= update.id.is_some();
                if update.id != current.id {
                    stamp = None;
                    last_words = 0;
                    speech_until = Instant::now();
                    transcript_revision = transcript_revision.wrapping_add(1);
                }
                current = update;
            }
            let now = if replay {
                current
                    .now_ms
                    .unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
            } else {
                chrono::Utc::now().timestamp_millis()
            };
            let mut dirty = false;
            if replay
                && current.seen
                && let Some(id) = &current.id
                && let Some(raw) = summary(id)
            {
                draft_seen(id, &raw);
            }
            if current.recording
                && let Some(id) = &current.id
            {
                let fresh = !pending.contains_key(id);
                dirty |= fresh || pending.get(id).is_some_and(|p| p.stopped_at_ms.is_some());
                pending.insert(
                    id.clone(),
                    Pending {
                        elapsed: current.elapsed,
                        stopped_at_ms: None,
                        notice_at_ms: None,
                        notice_since: None,
                        skipped_at_ms: None,
                        stopped_since: None,
                        skipped_since: None,
                    },
                );
                if let Ok(core::live::Read::Lines(lines, new_stamp)) =
                    core::live::read_rolling(&core::meetings_dir().join(id), stamp)
                {
                    let had_baseline = stamp.is_some();
                    stamp = Some(new_stamp);
                    transcript_revision = transcript_revision.wrapping_add(1);
                    let words = lines
                        .iter()
                        .map(|l| l.text.split_whitespace().count())
                        .sum();
                    // Initial history is not evidence of speech happening now.
                    if !replay && had_baseline && words > last_words {
                        speech_until = Instant::now() + Duration::from_secs(2);
                    }
                    last_words = words;
                }
            }
            let before = pending.len();
            pending.retain(|id, p| {
                if current.recording && current.id.as_ref() == Some(id) {
                    return true;
                }
                if p.stopped_at_ms.is_none_or(|t| t > now) {
                    p.stopped_at_ms = Some(now);
                    dirty = true;
                }
                let stopped_since = *p.stopped_since.get_or_insert_with(Instant::now);
                if let Some(raw) = summary(id) {
                    return !was_seen(id, &raw);
                }
                if summary_skip(id).is_some_and(|s| s != core::notes::SummarySkip::Off) {
                    if p.skipped_at_ms.is_none_or(|t| t > now) {
                        p.skipped_at_ms = Some(now);
                        dirty = true;
                    }
                    let skipped_since = *p.skipped_since.get_or_insert_with(Instant::now);
                    return now.saturating_sub(p.skipped_at_ms.unwrap_or(now)) < SKIPPED_NOTICE_MS
                        && skipped_since.elapsed()
                            < Duration::from_millis(SKIPPED_NOTICE_MS as u64);
                }
                if p.skipped_at_ms.take().is_some() {
                    dirty = true;
                }
                p.skipped_since = None;
                let state = notice(id);
                let expired = now.saturating_sub(p.stopped_at_ms.unwrap_or(now))
                    >= NO_DRAFT_BOUND_MS
                    || stopped_since.elapsed() >= Duration::from_millis(NO_DRAFT_BOUND_MS as u64);
                let previous = p.notice_at_ms;
                let keep = retain_notice(
                    p,
                    state,
                    now,
                    expired,
                    store_io::read::<i64>(&dismissed_path(id)).ok(),
                );
                dirty |= previous != p.notice_at_ms;
                keep
            });
            if (dirty || pending.len() != before)
                && store_io::write(&pending_path(), &pending).is_err()
            {
                crate::listener::log("could not save floating meeting acknowledgement");
            }
            let item = current
                .id
                .as_ref()
                .filter(|id| current.recording && pending.contains_key(*id))
                .or_else(|| pending.last_key_value().map(|(id, _)| id));
            if let Some(id) = item {
                if host.is_none() && !native::disabled() {
                    host = native::Host::new(replay, &root)
                        .map_err(|_| crate::listener::log("floating icon window unavailable"))
                        .ok();
                }
                if let Some(window) = &host
                    && !window.update(frame_for(
                        &current,
                        id,
                        &pending[id],
                        speech_until,
                        transcript_revision,
                    ))
                {
                    host = None;
                }
            } else {
                host = None;
            }
        }
        native::maintenance();
        std::thread::sleep(Duration::from_millis(if replay { 100 } else { 250 }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nub_frame_does_not_read_meeting_parts_after_stop() {
        use std::io::Write;

        let dir = core::state_dir().join("local/meeting-parts");
        std::fs::create_dir_all(&dir).unwrap();
        let _records = ["9088645-1", "9088645-2"].map(|id| {
            let mut record = tempfile::Builder::new()
                .prefix(id)
                .suffix(".json")
                .rand_bytes(0)
                .tempfile_in(&dir)
                .unwrap();
            write!(record, r#"{{"id":"{id}","phase":"Paused"}}"#).unwrap();
            record
        });
        let id = "9088645-3";
        let recording = Snapshot {
            id: Some(id.to_owned()),
            recording: true,
            elapsed: 125,
            ..Snapshot::default()
        };
        let mut pending = Pending {
            elapsed: recording.elapsed,
            ..Pending::default()
        };
        let stopped = Snapshot::default();
        let mut counts = Vec::new();
        for snapshot in [&recording, &stopped, &stopped, &stopped] {
            pending.stopped_at_ms = (!snapshot.recording).then_some(1000);
            let scope = file_reads::Scope::begin();
            let frame = frame_for(snapshot, id, &pending, Instant::now(), 1);
            assert_eq!(frame.id, id);
            assert_eq!(frame.recording, snapshot.recording);
            assert_eq!(frame.notice.is_none(), snapshot.recording);
            assert_eq!(frame.elapsed, 125);
            let stats = scope.stats();
            counts.push((stats.reads, stats.entries));
        }
        assert_eq!(counts, [(0, 0); 4], "reads and directory entries per frame");
    }

    #[test]
    fn meeting_window_chooses_a_side_and_fits_the_work_area() {
        let work = [0, 0, 1334, 860];
        let small = [-350, 20, 0, 420];
        for (work, icon, expected) in [
            (work, [10, 330, 46, 109], [64, 180, 400, 680]),
            (work, [1278, 330, 46, 109], [870, 180, 400, 680]),
            (small, [-56, 340, 46, 109], [-350, 20, 350, 400]),
        ] {
            assert_eq!(beside(work, icon, [400, 680], 8), expected);
        }
    }

    #[test]
    fn meeting_window_restores_only_while_it_has_the_placed_geometry() {
        let placed = [870, 180, 400, 680];
        assert!(can_restore(placed, Some(placed)));
        assert!(!can_restore(placed, Some([871, 180, 400, 680])));
        assert!(!can_restore(placed, Some([870, 180, 401, 680])));
    }

    #[test]
    fn terminal_notice_is_bounded_and_click_dismisses() {
        assert!(notice_fresh(1000, 30_999, None));
        assert!(!notice_fresh(1000, 31_000, None));
        assert!(!notice_fresh(1000, 1001, Some(1001)));
        assert!(notice_fresh(1000, 1001, Some(999)));
        assert_eq!(Notice::Later.text(), "Summary will be written later");
        assert_eq!(Notice::Failed(Cause::Auth).text(), "Agent sign-in needed");
        assert_eq!(
            Notice::Failed(Cause::Limit).text(),
            "Agent usage limit reached"
        );
        assert_eq!(
            Notice::Failed(Cause::TimedOut).text(),
            "Meeting processing took too long"
        );
    }
}
