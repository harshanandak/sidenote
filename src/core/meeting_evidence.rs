//! Measurement only. Never publishes to Hex or the notes pipeline.
use super::{
    growing_wav::{GrowingWav, RATE},
    meeting_watch::shared_read,
    store_io,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const TRACKS: [&str; 2] = ["microphone", "system"];
const MODELS: [(&str, &str); 2] = [
    ("custom-9ac644a9878e7415", "te"),
    ("nemotron_3_5_asr", "hi"),
];
const WINDOW: u64 = 300 * RATE;
const SEARCH: u64 = 2 * RATE;

#[derive(Clone, Serialize, Deserialize)]
struct Job {
    track: String,
    index: u64,
    start: u64,
    end: u64,
    audio_start: u64,
    queued_ms: i64,
}
fn folders(path: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_type().ok().filter(|t| t.is_dir()).map(|_| e.path()))
        .collect();
    paths.sort();
    paths
}
fn jobs(folder: &Path) -> io::Result<Vec<(PathBuf, Job)>> {
    let mut found = Vec::new();
    for dir in folders(folder) {
        let path = dir.join("job.json");
        if path.exists() {
            found.push((dir, serde_json::from_slice(&fs::read(path)?)?));
        }
    }
    Ok(found)
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
// Folder names carry Hex's creation clock, so association never needs manifest content.
fn discover(root: &Path, status: &Value) -> Option<PathBuf> {
    if let Some(id) = status["id"].as_str().filter(|s| valid_id(s)) {
        let path = root.join(id);
        return super::file_reads::directory(&path).then_some(path);
    }
    let start = chrono::Utc::now().timestamp_millis().checked_sub(
        i64::try_from(status["elapsed"].as_u64()?)
            .ok()?
            .checked_mul(1000)?,
    )?;
    let mut found = folders(root).into_iter().filter(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .filter(|s| valid_id(s))
            .and_then(|n| n.split('-').next()?.parse::<i64>().ok())
            .is_some_and(|t| t.abs_diff(start) <= 90_000)
    });
    let first = found.next()?;
    found.next().is_none().then_some(first)
}
fn evidence_claim(meeting: &Path) -> io::Result<Option<super::meeting_delete::MeetingClaim>> {
    let id = meeting
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(io::ErrorKind::InvalidInput)?;
    let state = meeting
        .ancestors()
        .nth(3)
        .ok_or(io::ErrorKind::InvalidInput)?;
    super::meeting_delete::claim_write(state, id)
}

struct Active {
    source: PathBuf,
    last_lengths: [u64; 2],
    stable: u8,
}
pub(crate) struct Collector {
    root: PathBuf,
    active: Option<Active>,
    worker: Option<std::thread::JoinHandle<()>>,
    #[cfg(windows)]
    uploader: Option<std::thread::JoinHandle<()>>,
}
impl Collector {
    pub(crate) fn new(state: &Path) -> Self {
        let root = state.join("local/evidence");
        let active = folders(&root).into_iter().find_map(|folder| {
            let id = folder.file_name()?.to_str()?;
            (valid_id(id)
                && folder.join("session.json").exists()
                && !folder.join("collection.json").exists())
            .then(|| Active {
                source: super::meetings_dir().join(id),
                last_lengths: [u64::MAX; 2],
                stable: 0,
            })
        });
        Self {
            root,
            active,
            worker: None,
            #[cfg(windows)]
            uploader: None,
        }
    }
    pub(crate) fn tick(&mut self, status: &Value, enabled: bool) -> io::Result<()> {
        #[cfg(windows)]
        let upload = self.upload_tick();
        #[cfg(not(windows))]
        let upload = false;
        if !enabled && !upload {
            return Ok(());
        }
        if enabled && self.worker.is_none() {
            self.worker = Some(start_worker(self.root.clone()));
        }
        self.capture(status, &super::meetings_dir(), upload)
    }

    fn capture(&mut self, status: &Value, meetings: &Path, upload: bool) -> io::Result<()> {
        let recording = matches!(
            status["meeting_state"].as_str(),
            Some("starting" | "recording" | "paused" | "stopping" | "finishing")
        );
        let source = recording.then(|| discover(meetings, status)).flatten();
        if let Some(source) = source {
            if let Some(previous) = self.active.as_ref().filter(|a| a.source != source) {
                self.collect_with_limit(&previous.source, true, upload)?;
                self.active = None;
            }
            self.active.get_or_insert(Active {
                source,
                last_lengths: [u64::MAX; 2],
                stable: 0,
            });
        }
        let Some(active) = self.active.as_mut() else {
            return Ok(());
        };
        let source = active.source.clone();
        let dest = self
            .root
            .join(source.file_name().ok_or(io::ErrorKind::InvalidInput)?);
        let Some(_claim) = evidence_claim(&dest)? else {
            self.active = None;
            return Ok(());
        };
        fs::create_dir_all(&dest)?;
        if !dest.join("session.json").exists() {
            store_io::write(
                &dest.join("session.json"),
                &json!({"started_ms":chrono::Utc::now().timestamp_millis()}),
            )?;
        }
        if recording {
            sample_lag(&source, &dest, status["elapsed"].as_u64())?;
            return self.collect_claimed(&source, false, upload);
        }
        if !matches!(
            status["meeting_state"].as_str(),
            Some("idle" | "stopped" | "complete" | "finished" | "transcribing")
        ) {
            return Ok(());
        }
        let lengths =
            TRACKS.map(|t| fs::metadata(source.join(format!("{t}.wav"))).map_or(0, |m| m.len()));
        active.stable = if active.last_lengths == lengths {
            active.stable.saturating_add(1)
        } else {
            0
        };
        active.last_lengths = lengths;
        // Status can turn idle before the writer finalizes. Wait for two stable polls.
        if active.stable >= 2 {
            self.collect_claimed(&source, true, upload)?;
            self.active = None;
        }
        Ok(())
    }
    #[cfg(test)]
    fn collect(&self, source: &Path, stopped: bool) -> io::Result<()> {
        self.collect_with_limit(source, stopped, false)
    }

    fn collect_with_limit(&self, source: &Path, stopped: bool, uploading: bool) -> io::Result<()> {
        let dest = self
            .root
            .join(source.file_name().ok_or(io::ErrorKind::InvalidInput)?);
        let Some(_claim) = evidence_claim(&dest)? else {
            return Ok(());
        };
        self.collect_claimed(source, stopped, uploading)
    }

    fn collect_claimed(&self, source: &Path, stopped: bool, uploading: bool) -> io::Result<()> {
        let dest = self
            .root
            .join(source.file_name().ok_or(io::ErrorKind::InvalidInput)?);
        fs::create_dir_all(&dest)?;
        let previous = jobs(&dest)?;
        let mut missing = Vec::new();
        for track in TRACKS {
            let path = source.join(format!("{track}.wav"));
            let mut reader = match GrowingWav::open(&path) {
                Ok(r) => r,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    missing.push(track);
                    continue;
                }
                Err(e)
                    if !stopped
                        && matches!(
                            e.kind(),
                            io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData
                        ) =>
                {
                    continue;
                }
                Err(e) => return Err(e),
            };
            let available = reader.len()?;
            let mut cursor = previous
                .iter()
                .filter(|(_, j)| j.track == track)
                .map(|(_, j)| j.end)
                .max()
                .unwrap_or(0);
            let mut index = previous
                .iter()
                .filter(|(_, j)| j.track == track)
                .map(|(_, j)| j.index + 1)
                .max()
                .unwrap_or(0);
            if available < cursor {
                return Err(io::ErrorKind::InvalidData.into());
            }
            while available > cursor && (stopped || available - cursor >= WINDOW + SEARCH) {
                let cap = if uploading { WINDOW } else { WINDOW + SEARCH };
                let audio_start = if cursor == 0 {
                    0
                } else {
                    cursor.saturating_sub(2 * RATE)
                };
                let limit = (cursor + WINDOW + SEARCH).min(available);
                let audio = reader.read(audio_start, limit)?;
                let end = if available - cursor >= WINDOW + SEARCH
                    || (uploading && available - cursor > WINDOW)
                {
                    audio_start
                        + low_energy_cut(
                            &audio,
                            (cursor + WINDOW - SEARCH - audio_start) as usize,
                            (cursor + cap - audio_start) as usize,
                        ) as u64
                } else {
                    available
                };
                let job = Job {
                    track: track.into(),
                    index,
                    start: cursor,
                    end,
                    audio_start,
                    queued_ms: chrono::Utc::now().timestamp_millis(),
                };
                let dir = dest.join(format!("{track}-{index:06}"));
                fs::create_dir_all(&dir)?;
                let mut writer = hound::WavWriter::create(
                    dir.join("audio.tmp"),
                    hound::WavSpec {
                        channels: 1,
                        sample_rate: RATE as u32,
                        bits_per_sample: 16,
                        sample_format: hound::SampleFormat::Int,
                    },
                )
                .map_err(io::Error::other)?;
                for &s in &audio[..(end - audio_start) as usize] {
                    writer
                        .write_sample((s * 32767.0).round() as i16)
                        .map_err(io::Error::other)?;
                }
                writer.finalize().map_err(io::Error::other)?;
                fs::rename(dir.join("audio.tmp"), dir.join("audio.wav"))?;
                // The descriptor is published last, so workers never see a partial WAV.
                store_io::write(&dir.join("job.json"), &job)?;
                cursor = end;
                index += 1;
            }
        }
        if stopped {
            store_io::write(
                &dest.join("collection.json"),
                &json!({"stopped_ms":chrono::Utc::now().timestamp_millis(),"missing_tracks":missing}),
            )?;
        }
        Ok(())
    }
}
#[cfg(windows)]
impl Collector {
    fn upload_tick(&mut self) -> bool {
        use super::speech_host::{self, Host, WorkMode};
        let Ok(settings) = speech_host::settings_path() else {
            return false;
        };
        let host = Host::load(&settings).ok().flatten();
        if !WorkMode::load(&settings).is_ok_and(|(mode, _)| {
            mode == WorkMode::ThisDevice || host.as_ref().is_some_and(Host::upload_enabled)
        }) {
            return false;
        }
        if self
            .uploader
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return true;
        }
        if let Some(worker) = self.uploader.take() {
            let _ = worker.join();
        }
        let evidence = self.root.clone();
        let data = evidence
            .parent()
            .and_then(Path::parent)
            .expect("collector data root")
            .to_owned();
        self.uploader = std::thread::Builder::new()
            .name("speech-host-upload".into())
            .spawn(move || {
                let _ = upload_windows(
                    &evidence,
                    &data,
                    &settings,
                    host,
                    &super::meetings_dir(),
                    || chrono::Utc::now().timestamp_millis(),
                    local_span_runner(),
                );
            })
            .ok();
        true
    }
}

#[cfg(windows)]
fn upload_windows(
    evidence: &Path,
    data: &Path,
    settings: &Path,
    host: Option<super::speech_host::Host>,
    source: &Path,
    now: impl Fn() -> i64,
    mut local: Option<
        impl FnMut(
            &Path,
            &super::speech_host::journal::Piece,
            (u64, u64),
            &super::Config,
            Instant,
        ) -> super::asr::Result<Vec<Value>>,
    >,
) -> io::Result<()> {
    use super::{
        meeting_router::ProviderRoute,
        speech_host::{Progress, WorkMode, journal::Piece, upload::Upload},
    };
    let mut config = super::Config::load_in(data)?;
    let mut routes = config
        .pipeline
        .after_stop_languages
        .clone()
        .map_or_else(|| super::model_catalog::load_language_routes(data), Ok)?;
    let languages: Vec<_> = std::iter::once(&routes.primary)
        .chain(&routes.supporting)
        .cloned()
        .collect();
    if (config.pipeline.language_mode != "auto"
        && WorkMode::load(settings)?.0 != WorkMode::ThisDevice)
        || languages
            .iter()
            .any(|l| !matches!(l.as_str(), "en" | "hi" | "te"))
    {
        return Ok(());
    }
    if config.pipeline.after_stop_languages.is_none() {
        for language in &languages {
            let model = match language.as_str() {
                "en" => "parakeet_unified_en",
                "hi" => "indicconformer-hi-ctc",
                _ => "indicconformer-te-ctc",
            };
            routes
                .routes
                .entry(language.clone())
                .or_insert_with(|| ProviderRoute {
                    model: config.engine_model(language).unwrap_or(model).into(),
                    provider: if language == "en" { "hex" } else { "local" }.into(),
                });
        }
    }
    config.pipeline.after_stop_languages = Some(routes);
    config.pipeline.meeting_languages.clone_from(&languages);
    if config.pipeline.model_state_dir.as_os_str().is_empty() {
        config.pipeline.model_state_dir = data.to_owned();
    }
    if config.pipeline.runtime_dll.as_os_str().is_empty() {
        config.pipeline.runtime_dll =
            std::env::current_exe()?.with_file_name(crate::platform::asr_library_name());
    }
    let Some(mut upload) =
        Upload::new(host, settings, data.join("local/speech-uploads"), languages)
    else {
        return Ok(());
    };
    let mut failure = None;
    let mut latest_progress = None;
    for folder in folders(evidence) {
        let Some(meeting) = folder
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|id| valid_id(id))
        else {
            continue;
        };
        let Some(_claim) = evidence_claim(&folder)? else {
            continue;
        };
        let pending: Vec<_> = jobs(&folder)?
            .into_iter()
            .filter(|(_, j)| {
                j.queued_ms > upload.since()
                    && j.end.checked_sub(j.start).is_some_and(|n| n <= WINDOW)
            })
            .collect();
        if pending.is_empty() {
            continue;
        }
        let manifest = source.join(meeting).join("manifest.json");
        let Ok(bytes) = super::meeting_router::inputs::Snapshot::read_file(&manifest, 1 << 20)
        else {
            continue;
        };
        let manifest: Value = serde_json::from_slice(&bytes)?;
        if manifest["created_at_ms"]
            .as_i64()
            .is_none_or(|ms| ms <= upload.since())
        {
            continue;
        }
        let total = pending.len();
        let directory = match upload.directory(meeting) {
            Ok(directory) => directory,
            Err(error) => {
                failure.get_or_insert(error);
                continue;
            }
        };
        let mut progress = Progress {
            total,
            ..Progress::default()
        };
        let mut journals = Vec::with_capacity(total);
        for (dir, job) in pending {
            let piece = Piece {
                track: job.track,
                index: u32::try_from(job.index).map_err(io::Error::other)?,
                origin_sample: 0,
                start_sample: job.start,
                end_sample: job.end,
                context_start_sample: job.audio_start,
                sha256: String::new(),
            };
            let wav = dir.join("audio.wav");
            let seconds = (job.end - job.start) as f64 / RATE as f64;
            let allowance = (config.asr_budget(seconds) * 1000.0).ceil() as u64;
            let age = now().saturating_sub(job.queued_ms).max(0) as u64;
            let tick = Instant::now();
            let cutoff = |ms: u64| tick + Duration::from_millis(ms.saturating_sub(age));
            match upload.piece(
                meeting,
                &directory,
                piece,
                &wav,
                (cutoff(allowance / 2), cutoff(allowance)),
                local.as_mut().map(|run| {
                    let (wav, config, progress) = (&wav, &config, &progress);
                    move |journal: &super::speech_host::journal::Journal, span, deadline| {
                        progress.save(settings, Some(journal));
                        run(wav, journal.piece(), span, config, deadline).ok()
                    }
                }),
            ) {
                Ok(Some(journal)) => {
                    progress.sent += usize::from(journal.acknowledged());
                    progress.host += usize::from(journal.done() && !journal.host_failed());
                    progress.local += usize::from(journal.finished_here());
                    progress.gaps += journal.missing().len();
                    journals.push(journal);
                }
                Ok(None) => {}
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        latest_progress = Some(progress);
        if folder.join("collection.json").exists()
            && manifest["status"] == "complete"
            && journals.len() == total
            && journals.iter().all(|journal| journal.done())
        {
            let origins: std::collections::BTreeMap<_, _> = journals
                .iter()
                .filter_map(|journal| {
                    let track = &journal.piece().track;
                    let us = manifest[track]["first_arrival_us"].as_u64()?;
                    Some((
                        track.clone(),
                        i64::try_from((us / 1000).checked_mul(16)?).ok()?,
                    ))
                })
                .collect();
            if !journals
                .iter()
                .all(|journal| origins.contains_key(&journal.piece().track))
            {
                continue;
            }
            let transcript = source.join(meeting).join("transcript.ndjson");
            if let Ok(bytes) =
                super::meeting_router::inputs::Snapshot::read_file(&transcript, 16 << 20)
            {
                upload.stage(&directory, &journals, &origins, &bytes)?;
            }
        }
    }
    if let Some(progress) = latest_progress {
        progress.save(settings, None);
    }
    failure.map_or(Ok(()), Err)
}

#[cfg(windows)]
type LocalSpan = fn(
    &Path,
    &super::speech_host::journal::Piece,
    (u64, u64),
    &super::Config,
    Instant,
) -> super::asr::Result<Vec<Value>>;

#[cfg(windows)]
fn local_span_runner() -> Option<LocalSpan> {
    cfg!(feature = "asr-runtime").then_some(run_local_span)
}

#[cfg(windows)]
fn run_local_span(
    wav: &Path,
    piece: &super::speech_host::journal::Piece,
    (start, end): (u64, u64),
    config: &super::Config,
    deadline: Instant,
) -> super::asr::Result<Vec<Value>> {
    use super::{
        meeting_router::{batches, inputs::Snapshot, jobs::BatchOutput, production},
        speech_host::upload::{MAX_WAV, digest},
    };
    batches::remaining(Some(deadline))?;
    let temporary = tempfile::tempdir()?;
    let input = temporary.path().join("input");
    let work = temporary.path().join("job");
    fs::create_dir(&input)?;
    let cut = input.join(format!("{}.wav", piece.track));
    let bytes = Snapshot::read_file(wav, MAX_WAV)?;
    if digest(&bytes) != piece.sha256 {
        return Err("Collector WAV changed".into());
    }
    let mut reader = hound::WavReader::new(io::Cursor::new(bytes))?;
    let offset = start
        .checked_sub(piece.context_start_sample)
        .ok_or("Invalid local audio start")?;
    if end <= start || offset.saturating_add(end - start) > u64::from(reader.duration()) {
        return Err("Local span exceeds collector WAV".into());
    }
    reader.seek(u32::try_from(offset)?)?;
    let mut writer = hound::WavWriter::create(&cut, reader.spec())?;
    for sample in reader.samples::<i16>().take(usize::try_from(end - start)?) {
        writer.write_sample(sample?)?;
    }
    writer.finalize()?;
    store_io::write(&input.join("manifest.json"), &json!({"status":"complete"}))?;
    store_io::atomic_write(&input.join("transcript.ndjson"), b"")?;
    let id = digest(&serde_json::to_vec(&(&piece.sha256, start, end))?);
    production::run(
        &input,
        &[(piece.track.clone(), cut)],
        &[],
        config,
        &work,
        (&id, 0),
        Some(deadline),
    )?;
    let output: BatchOutput =
        serde_json::from_slice(&Snapshot::read_file(&work.join("output.json"), 16 << 20)?)?;
    if !output.fallbacks.is_empty() {
        return Err("Unrouted speech on this device".into());
    }
    output
        .plan
        .batches
        .values()
        .flatten()
        .map(|(key, range)| {
            let mut value =
                batches::read_result(&work, &output.plan, key)?.ok_or("Missing local reading")?;
            value["piece"] = json!(id);
            value["range"] = json!(key);
            value["track"] = json!(piece.track);
            value["route"] = json!(range.range.language);
            value["language"] = json!(range.range.language);
            value["model_sha256"] = json!(range.model_sha256);
            value["start_sample"] = json!(start + range.range.context.0);
            value["end_sample"] = json!(start + range.range.context.1);
            Ok(value)
        })
        .collect()
}

fn low_energy_cut(audio: &[f32], left: usize, right: usize) -> usize {
    let candidates = (left..=right.min(audio.len())).step_by(320);
    let mut best = (left, f64::INFINITY, usize::MAX);
    let middle = (left + right) / 2;
    for i in candidates {
        let frame = &audio[i.saturating_sub(640)..(i + 640).min(audio.len())];
        if frame.is_empty() {
            continue;
        }
        let energy = frame.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / frame.len() as f64;
        let distance = i.abs_diff(middle);
        if energy < best.1 || (energy == best.1 && distance < best.2) {
            best = (i, energy, distance);
        }
    }
    best.0
}
fn append(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")
}
fn sample_lag(source: &Path, dest: &Path, elapsed: Option<u64>) -> io::Result<()> {
    let mut file = match shared_read(&source.join("transcript.rolling.ndjson")) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return append(
                &dest.join("lag.ndjson"),
                &json!({"timestamp_ms":chrono::Utc::now().timestamp_millis(),"elapsed_seconds":elapsed,"covered_ms":null,"lag_ms":null}),
            );
        }
        Err(e) => return Err(e),
    };
    // Read exactly the header, without buffering ahead into transcript text.
    let covered = read_coverage(&mut file)?;
    append(
        &dest.join("lag.ndjson"),
        &json!({"timestamp_ms":chrono::Utc::now().timestamp_millis(),"elapsed_seconds":elapsed,"covered_ms":covered,"lag_ms":elapsed.zip(covered).map(|(e,c)|e.saturating_mul(1000).saturating_sub(c))}),
    )
}

fn read_coverage(file: &mut impl Read) -> io::Result<Option<u64>> {
    let mut header = Vec::new();
    for _ in 0..1024 {
        let mut byte = [0];
        if file.read(&mut byte)? == 0 || byte[0] == b'\n' {
            break;
        }
        header.push(byte[0]);
    }
    #[derive(Deserialize)]
    struct Header {
        covered_ms: Option<u64>,
    }
    Ok(serde_json::from_slice::<Header>(&header)
        .ok()
        .and_then(|h| h.covered_ms))
}

#[derive(Serialize, Deserialize)]
struct Timing {
    track: String,
    index: u64,
    audio_seconds: f64,
    queue_wait_seconds: f64,
    model_seconds: [f64; 2],
    backlog_windows: usize,
    error: Option<String>,
}
fn pending(root: &Path) -> io::Result<Vec<(PathBuf, Job)>> {
    let mut all = Vec::new();
    for meeting in folders(root) {
        all.extend(
            jobs(&meeting)?
                .into_iter()
                .filter(|(p, _)| !p.join("timing.json").exists()),
        );
    }
    all.sort_by_key(|(_, j)| j.queued_ms);
    Ok(all)
}
fn start_worker(root: PathBuf) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // Caller priority does not control Hex inference priority. The Lab measures that contention.
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Threading::SetThreadPriority(
                windows_sys::Win32::System::Threading::GetCurrentThread(),
                windows_sys::Win32::System::Threading::THREAD_PRIORITY_BELOW_NORMAL,
            );
        }
        loop {
            if crate::updates::listener_stopping() {
                return;
            }
            if super::Config::load()
                .is_ok_and(|c| c.features.indic_engine && c.labs.process_meeting_windows)
            {
                if crate::hex_engine::listener_status().is_ok_and(|s| s["can_process"] == true)
                    && drain(&root, post_audio).is_err()
                {
                    crate::listener::log("window measurement worker failed; pending work is saved");
                }
                for meeting in folders(&root) {
                    let _ = summary(&meeting);
                }
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    })
}
fn drain(
    root: &Path,
    transcribe: impl FnMut(&str, &str, &[u8]) -> io::Result<Value>,
) -> io::Result<()> {
    drain_pending(root, pending(root)?, transcribe)
}

fn drain_pending(
    root: &Path,
    pending: Vec<(PathBuf, Job)>,
    mut transcribe: impl FnMut(&str, &str, &[u8]) -> io::Result<Value>,
) -> io::Result<()> {
    let backlog = pending.len();
    let mut claims = std::collections::BTreeMap::new();
    let mut batch = Vec::new();
    for (dir, job) in pending {
        let meeting = dir
            .ancestors()
            .find(|p| p.parent() == Some(root))
            .ok_or(io::ErrorKind::InvalidInput)?;
        if let std::collections::btree_map::Entry::Vacant(slot) = claims.entry(meeting.to_owned()) {
            slot.insert(evidence_claim(meeting)?);
        }
        if claims[meeting].is_some() && dir.is_dir() {
            batch.push((dir, job));
            if batch.len() == 4 {
                break;
            }
        }
    }
    let mut times = Vec::new();
    for (dir, j) in &batch {
        let path = dir.join("timing-start.json");
        let timing = if path.exists() {
            serde_json::from_slice::<Timing>(&fs::read(path)?)?
        } else {
            let timing = Timing {
                track: j.track.clone(),
                index: j.index,
                audio_seconds: (j.end - j.audio_start) as f64 / RATE as f64,
                queue_wait_seconds: (chrono::Utc::now().timestamp_millis() - j.queued_ms).max(0)
                    as f64
                    / 1000.0,
                model_seconds: [0.0; 2],
                backlog_windows: backlog,
                error: None,
            };
            store_io::write(&path, &timing)?;
            timing
        };
        times.push(timing);
    }
    // Group the ready batch by model, including both tracks, to reduce model swaps.
    for (model_index, (model, language)) in MODELS.iter().enumerate() {
        for ((dir, _), timing) in batch.iter().zip(&mut times) {
            let result_path = dir.join(format!("{language}.json"));
            if result_path.exists() {
                #[derive(Deserialize)]
                struct Receipt {
                    seconds: f64,
                }
                timing.model_seconds[model_index] =
                    serde_json::from_slice::<Receipt>(&fs::read(result_path)?)?.seconds;
                continue;
            }
            let audio = fs::read(dir.join("audio.wav"))?;
            match transcribe_window(dir, model, language, &audio, &mut transcribe) {
                Ok((response, seconds)) => {
                    timing.model_seconds[model_index] = seconds;
                    store_io::write(
                        &result_path,
                        &json!({"model":model,"language":language,"seconds":timing.model_seconds[model_index],"response":response}),
                    )?;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(_) => {
                    // Bodies and transport errors may contain content or credentials.
                    store_io::write(
                        &dir.join("error.json"),
                        &json!({"timestamp_ms":chrono::Utc::now().timestamp_millis(),"error":"local_transcription_failed","model":model}),
                    )?;
                    return Err(io::ErrorKind::ConnectionAborted.into());
                }
            }
        }
    }
    for ((dir, _), timing) in batch.iter().zip(times) {
        store_io::write(&dir.join("timing.json"), &timing)?;
    }
    Ok(())
}
// Match the owner's bounded requests, while keeping five minutes as the queue unit.
fn transcribe_window(
    dir: &Path,
    model: &str,
    language: &str,
    bytes: &[u8],
    transcribe: &mut impl FnMut(&str, &str, &[u8]) -> io::Result<Value>,
) -> io::Result<(Value, f64)> {
    let mut reader =
        hound::WavReader::new(std::io::Cursor::new(bytes)).map_err(io::Error::other)?;
    let audio: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.map(|v| f32::from(v) / 32768.0))
        .collect::<Result<_, _>>()
        .map_err(io::Error::other)?;
    let receipts = dir.join(language);
    fs::create_dir_all(&receipts)?;
    let mut start = 0;
    let mut segments = Vec::new();
    let mut seconds = 0.0;
    while start < audio.len() {
        let end = if audio.len() - start > (24.4 * RATE as f64) as usize {
            low_energy_cut(
                &audio,
                start + (19.4 * RATE as f64) as usize,
                (start + (24.4 * RATE as f64) as usize).min(audio.len() - RATE as usize),
            )
        } else {
            audio.len()
        };
        let receipt = receipts.join(format!("chunk-{:04}.json", segments.len()));
        let result: Value = if receipt.exists() {
            serde_json::from_slice(&fs::read(&receipt)?)?
        } else {
            let mut wav = std::io::Cursor::new(Vec::new());
            {
                let mut writer = hound::WavWriter::new(
                    &mut wav,
                    hound::WavSpec {
                        channels: 1,
                        sample_rate: RATE as u32,
                        bits_per_sample: 16,
                        sample_format: hound::SampleFormat::Int,
                    },
                )
                .map_err(io::Error::other)?;
                for &sample in &audio[start..end] {
                    writer
                        .write_sample((sample * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
                        .map_err(io::Error::other)?;
                }
                writer.finalize().map_err(io::Error::other)?;
            }
            let began = Instant::now();
            let response = transcribe(model, language, &wav.into_inner())?;
            let result = json!({"start_seconds":start as f64/RATE as f64,"end_seconds":end as f64/RATE as f64,"seconds":began.elapsed().as_secs_f64(),"response":response});
            store_io::write(&receipt, &result)?;
            result
        };
        seconds += result["seconds"]
            .as_f64()
            .ok_or(io::ErrorKind::InvalidData)?;
        segments.push(result);
        start = end;
    }
    Ok((json!({"segments":segments}), seconds))
}

fn post_audio(model: &str, language: &str, audio: &[u8]) -> io::Result<Value> {
    if crate::updates::listener_stopping()
        || !super::Config::load()
            .is_ok_and(|c| c.features.indic_engine && c.labs.process_meeting_windows)
        || !crate::hex_engine::listener_status().is_ok_and(|s| s["can_process"] == true)
    {
        return Err(io::ErrorKind::WouldBlock.into());
    }
    #[derive(Deserialize)]
    struct Discovery {
        port: u16,
        token: String,
    }
    let path = super::meetings_dir()
        .parent()
        .ok_or(io::ErrorKind::NotFound)?
        .join("local-api.json");
    let discovery: Discovery = serde_json::from_slice(&fs::read(path)?)?;
    if discovery.port == 0
        || discovery.token.is_empty()
        || !discovery.token.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let agent = ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(180)))
        .build()
        .new_agent();
    let mut response = agent
        .post(format!(
            "http://127.0.0.1:{}/transcriptions?model={model}&language={language}",
            discovery.port
        ))
        .header("Content-Type", "audio/wav")
        .header("Authorization", format!("Bearer {}", discovery.token))
        .send(audio)
        .map_err(|_| io::Error::from(io::ErrorKind::ConnectionAborted))?;
    let body = response
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_vec()
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    let value: Value =
        serde_json::from_slice(&body).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    if !value["transcript"].is_string() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(value)
}
fn summary(meeting: &Path) -> io::Result<()> {
    let Some(_claim) = evidence_claim(meeting)? else {
        return Ok(());
    };
    if !meeting.join("collection.json").exists() || meeting.join("summary.json").exists() {
        return Ok(());
    }
    let jobs = jobs(meeting)?;
    if jobs.iter().any(|(p, _)| !p.join("timing.json").exists()) {
        return Ok(());
    }
    let mut timings = Vec::new();
    for (p, _) in jobs {
        timings.push(serde_json::from_slice::<Timing>(&fs::read(
            p.join("timing.json"),
        )?)?);
    }
    let total = timings.iter().map(|t| t.audio_seconds).sum::<f64>();
    let models = [0, 1].map(|i| timings.iter().map(|t| t.model_seconds[i]).sum::<f64>());
    store_io::write(
        &meeting.join("summary.json"),
        &json!({"finished_ms":chrono::Utc::now().timestamp_millis(),"window_count":timings.len(),"audio_seconds_including_overlap":total,"model_seconds":models,"windows":timings,"collection":serde_json::from_slice::<Value>(&fs::read(meeting.join("collection.json"))?)?,"lag_samples":"lag.ndjson"}),
    )
}

impl Drop for Collector {
    fn drop(&mut self) {
        if crate::service::stopping()
            && let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
    }
}

#[cfg(all(test, windows))]
mod speech_host_tests;

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn audio(path: &Path, seconds: u64) {
        let mut w = hound::WavWriter::create(
            path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..seconds * 8000 {
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();
    }
    #[test]
    fn windows_tail_overlap_restart_grouping_and_summary_are_measurement_only() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("hex/100-1");
        fs::create_dir_all(&source).unwrap();
        for track in TRACKS {
            audio(&source.join(format!("{track}.wav")), 610);
        }
        let collector = Collector::new(temp.path());
        collector.collect(&source, false).unwrap();
        let dest = collector.root.join("100-1");
        let ready = jobs(&dest).unwrap();
        assert_eq!(ready.len(), 4);
        for track in TRACKS {
            let mut ranges: Vec<_> = ready
                .iter()
                .filter(|(_, j)| j.track == track)
                .map(|(_, j)| (j.start, j.end, j.audio_start))
                .collect();
            ranges.sort();
            assert_eq!(
                ranges,
                vec![(0, 300 * RATE, 0), (300 * RATE, 600 * RATE, 298 * RATE)]
            );
        }
        let restarted = Collector::new(temp.path());
        restarted.collect(&source, true).unwrap();
        assert_eq!(jobs(&dest).unwrap().len(), 6);
        let mut calls = Vec::new();
        drain(&collector.root, |model, _, bytes| {
            let clip = hound::WavReader::new(std::io::Cursor::new(bytes)).unwrap();
            assert!(clip.duration() <= 390400);
            calls.push(model.to_owned());
            Ok(json!({"transcript":"synthetic"}))
        })
        .unwrap();
        let split = calls.iter().position(|m| m == MODELS[1].0).unwrap();
        assert!(split > 4);
        assert!(calls[..split].iter().all(|m| m == MODELS[0].0));
        assert!(calls[split..].iter().all(|m| m == MODELS[1].0));
        summary(&dest).unwrap();
        assert!(!dest.join("summary.json").exists());
        drain(&collector.root, |_, _, _| {
            Ok(json!({"transcript":"synthetic"}))
        })
        .unwrap();
        summary(&dest).unwrap();
        let result: Value =
            serde_json::from_slice(&fs::read(dest.join("summary.json")).unwrap()).unwrap();
        assert_eq!(result["window_count"], 6);
        assert_eq!(fs::read_dir(&source).unwrap().count(), 2);
    }
    #[test]
    fn error_keeps_queue_pending_and_header_only_lag_is_content_free() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("100-1");
        fs::create_dir_all(&source).unwrap();
        audio(&source.join("system.wav"), 1);
        fs::write(
            source.join("transcript.rolling.ndjson"),
            b"{\"covered_ms\":123}\nnot-json-sensitive-content",
        )
        .unwrap();
        let collector = Collector::new(temp.path());
        collector.collect(&source, true).unwrap();
        let dest = collector.root.join("100-1");
        sample_lag(&source, &dest, Some(5)).unwrap();
        let lag = fs::read_to_string(dest.join("lag.ndjson")).unwrap();
        assert!(!lag.contains("sensitive"));
        assert!(lag.contains("4877"));
        assert!(
            drain(&collector.root, |_, _, _| Err(
                io::ErrorKind::TimedOut.into()
            ))
            .is_err()
        );
        assert_eq!(pending(&collector.root).unwrap().len(), 1);
        summary(&dest).unwrap();
        assert!(!dest.join("summary.json").exists());
        drain(&collector.root, |_, _, _| {
            Ok(json!({"transcript":"synthetic"}))
        })
        .unwrap();
        summary(&dest).unwrap();
        assert!(dest.join("summary.json").exists());
    }
    #[test]
    fn recycled_evidence_cannot_be_published_from_a_stale_batch() {
        use crate::core::meeting_delete::{Eligibility, Storage};
        let temp = tempfile::tempdir().unwrap();
        let meetings = temp.path().join("meetings");
        let source = meetings.join("100-1");
        let state = temp.path().join("state");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir(&state).unwrap();
        fs::write(source.join("manifest.json"), b"{}").unwrap();
        audio(&source.join("system.wav"), 1);
        let collector = Collector::new(&state);
        collector.collect(&source, true).unwrap();
        let queued = pending(&collector.root).unwrap();
        let staged = queued[0].0.clone();
        let before = fs::read(staged.join("audio.wav")).unwrap();
        let store = Storage::new(&meetings, &state).unwrap();
        let id = store
            .delete(
                "100-1",
                "Synthetic".into(),
                1,
                |_| Ok(Eligibility::Ready),
                |_| Ok(()),
            )
            .unwrap();
        drain_pending(&collector.root, queued, |_, _, _| {
            panic!("stale evidence was submitted")
        })
        .unwrap();
        collector.collect(&source, true).unwrap();
        assert!(!collector.root.join("100-1").exists());
        store.restore(&id).unwrap();
        assert_eq!(fs::read(staged.join("audio.wav")).unwrap(), before);
    }

    #[test]
    fn source_switch_retries_final_tail_after_a_busy_claim() {
        let temp = tempfile::tempdir().unwrap();
        let meetings = temp.path().join("meetings");
        let first = meetings.join("100-1");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir(meetings.join("100-2")).unwrap();
        audio(&first.join("system.wav"), 1);
        let mut collector = Collector::new(temp.path());
        let running = |id| json!({"meeting_state":"recording","id":id});
        collector
            .capture(&running("100-1"), &meetings, false)
            .unwrap();

        let held = crate::core::meeting_delete::writer_claim(temp.path(), "100-1").unwrap();
        let dest = collector.root.join("100-1");
        assert!(
            collector
                .capture(&running("100-2"), &meetings, false)
                .is_err()
        );
        assert!(!dest.join("collection.json").exists());
        drop(held);

        collector
            .capture(&running("100-2"), &meetings, false)
            .unwrap();
        assert!(dest.join("collection.json").is_file());
        let tail = jobs(&dest).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!((tail[0].1.start, tail[0].1.end), (0, RATE));
        assert_eq!(
            collector.active.as_ref().unwrap().source,
            meetings.join("100-2")
        );
    }

    #[test]
    fn coverage_reader_stops_at_header_newline() {
        let bytes = b"{\"covered_ms\":456}\nprivate-text-must-not-be-read";
        let mut cursor = std::io::Cursor::new(bytes);
        assert_eq!(read_coverage(&mut cursor).unwrap(), Some(456));
        assert_eq!(
            cursor.position(),
            bytes.iter().position(|b| *b == b'\n').unwrap() as u64 + 1
        );
    }
    #[test]
    fn boundary_chooses_low_energy_and_equal_energy_nearest_target() {
        let mut a = vec![1.0; 16000 * 6];
        a[16000 * 2..16000 * 3].fill(0.0);
        assert!((16000 * 2..16000 * 3).contains(&low_energy_cut(&a, 16000, 16000 * 5)));
        assert_eq!(
            low_energy_cut(&vec![0.0; 16000 * 6], 16000, 16000 * 5),
            16000 * 3
        );
    }
}
