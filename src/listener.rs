use crate::core::failure::{AtStage, Stage};
use serde::Deserialize;
use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::Command,
    time::Duration,
};

#[cfg(any(windows, test))]
pub(crate) mod completion;
pub mod heartbeat;
#[cfg(windows)]
mod notes_environment;
pub mod status;
mod timings;
#[derive(Deserialize)]
struct Discovery {
    port: u16,
    token: String,
}
enum Problem {
    Http(u16),
    Io(io::ErrorKind),
    Value,
}
impl From<io::Error> for Problem {
    fn from(e: io::Error) -> Self {
        Self::Io(e.kind())
    }
}
impl Problem {
    fn message(&self) -> String {
        match self {
            Self::Http(404) => "Labs > Meeting finished event is off".into(),
            Self::Http(409) => "another app is already listening".into(),
            Self::Http(code) => format!("HTTP {code}"),
            Self::Value => "Hex not reachable (JSONDecodeError)".into(),
            Self::Io(kind) => format!(
                "Hex not reachable ({})",
                match kind {
                    io::ErrorKind::NotFound => "FileNotFoundError",
                    io::ErrorKind::PermissionDenied => "PermissionError",
                    io::ErrorKind::ConnectionRefused => "URLError",
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "TimeoutError",
                    _ => "OSError",
                }
            ),
        }
    }
}
/// Append a content-free diagnostic to the existing app log.
pub fn log_process(message: &str) {
    let role = if std::env::args_os().any(|arg| arg == "--listen") {
        "listener"
    } else {
        "ui"
    };
    log(&format!("{message} role={role} pid={}", std::process::id()));
}

/// Append a content-free diagnostic to the existing app log.
pub fn log(message: &str) {
    #[cfg(target_os = "linux")]
    if crate::service::role() != crate::service::DeviceRole::Full || crate::service::active() {
        let _ = crate::operations::record(
            &crate::core::state_dir(),
            crate::operations::EventKind::WorkerChanged,
        );
        return;
    }
    if cfg!(not(windows)) && crate::service::active() {
        log_at(&crate::core::log_path(), "worker status=changed");
        return;
    }
    log_at(&crate::core::log_path(), message);
}
/// Record only non-sensitive diagnostics for a failed Areas worker.
pub fn log_area_io_error(refresh: bool, error: &io::Error) {
    let stage = if refresh { "refresh" } else { "operation" };
    log(&format!(
        "areas {stage} io_error kind={:?} raw_os_error={:?}",
        error.kind(),
        error.raw_os_error()
    ));
}
/// Existing catch-up and queue drains, owned by the service instead of a window.
pub(crate) fn service_tick() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if crate::service::role() == crate::service::DeviceRole::Server {
        return native_service_tick();
    }
    catch_up()?;
    crate::core::meeting_router::jobs::start_worker()?;
    crate::core::notes::queue::start_worker()
}
/// Linux consumes complete folders synchronized into the configured meetings root.
/// Peer inventories remain read-only; their files are never edited in place.
#[cfg(target_os = "linux")]
fn native_service_tick() -> io::Result<()> {
    let meetings = crate::core::meetings_dir();
    fs::create_dir_all(&meetings)?;
    let mut failure = None;
    if crate::core::meeting_router::jobs::configured() {
        if let Err(error) = catch_up_routing_automatic(&meetings) {
            failure = Some(error);
        }
        if let Err(error) = crate::core::meeting_router::jobs::start_worker() {
            failure.get_or_insert(error);
        }
    }
    // A routing error must not starve notes for other meetings.
    match crate::core::notes::settings()?.engine() {
        crate::core::notes::Engine::App => {
            if let Err(error) = crate::core::notes::queue::catch_up() {
                failure.get_or_insert(error);
            }
        }
        crate::core::notes::Engine::Scripts(_) => {
            failure.get_or_insert_with(crate::platform::unavailable);
        }
        crate::core::notes::Engine::Off => {}
    }
    failure.map_or(Ok(()), Err)
}
pub(crate) fn log_at(path: &std::path::Path, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let line = format!(
            "{} listener: {message}\n",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")
        );
        let _ = file.write_all(line.as_bytes());
    }
}
pub(crate) fn pipeline(meeting_id: Option<&str>) -> io::Result<()> {
    let root = crate::core::state_dir();
    let hex = crate::core::meetings_dir();
    let source = meeting_id
        .map(|id| crate::core::meeting_parts::source(&root, &hex, id))
        .unwrap_or(hex);
    pipeline_in(&root, &source, meeting_id)
}
pub fn generate_meeting(id: &str) -> io::Result<()> {
    if crate::core::notes::settings()?.engine() == crate::core::notes::Engine::App {
        if crate::core::meeting_router::jobs::notes_enabled(&crate::core::state_dir())?
            && let Some(meeting) = crate::core::meeting_parts::read(&crate::core::state_dir(), id)?
        {
            for part in meeting.parts.iter().take(meeting.requested_parts) {
                crate::core::meeting_router::jobs::enqueue_automatic(&part.id)
                    .map_err(io::Error::other)?;
            }
            crate::core::meeting_router::jobs::start_worker().map_err(io::Error::other)?;
        }
        crate::core::notes::request(id, crate::core::notes::Request::TryAgain)
    } else if crate::core::notes::settings()?.engine() == crate::core::notes::Engine::Off {
        Err(io::Error::other("Notes are off"))
    } else {
        crate::core::meeting_parts::prepare_script_update(&crate::core::state_dir(), id)?;
        pipeline(Some(id))
    }
}
fn pipeline_in(
    state: &std::path::Path,
    meetings: &std::path::Path,
    meeting_id: Option<&str>,
) -> io::Result<()> {
    pipeline_with(state, meetings, meeting_id, false)
}
fn pipeline_with(
    state: &std::path::Path,
    meetings: &std::path::Path,
    meeting_id: Option<&str>,
    automatic: bool,
) -> io::Result<()> {
    let activity = crate::updates::activity().at(Stage::CatchUpActivity)?;
    if let Some(id) = meeting_id
        && !crate::core::meeting_parts::held_in(state, meetings, id)?
        && crate::core::meeting_router::jobs::configured()
    {
        let queued = if automatic {
            crate::core::meeting_router::jobs::enqueue_automatic(id)
        } else {
            crate::core::meeting_router::jobs::enqueue(id)
        };
        if queued.is_err() {
            log("could not queue meeting routing; existing pipeline continues");
        } else if crate::core::meeting_router::jobs::start_worker().is_err() {
            log("routing remains queued for the next drain");
        }
    }
    let Some(settings) = crate::core::store_io::defer_busy(crate::core::notes::settings())
        .at(Stage::CatchUpSettings)?
    else {
        return Ok(());
    };
    let script = match settings.engine() {
        crate::core::notes::Engine::App => {
            if let Some(id) = meeting_id {
                crate::core::notes::queue::enqueue(id).at(Stage::CatchUpEnqueue)?;
                crate::core::notes::queue::start_worker()?;
            } else {
                crate::core::notes::queue::catch_up()?;
            }
            return Ok(());
        }
        crate::core::notes::Engine::Scripts(script) => {
            if let Some(id) = meeting_id {
                crate::core::notes::draft::start(
                    state.to_owned(),
                    meetings.to_owned(),
                    id.to_owned(),
                    settings,
                );
            }
            script
        }
        crate::core::notes::Engine::Off => {
            if let Some(id) = meeting_id {
                log(&format!("notes engine=off meeting={id}"));
            }
            return Ok(());
        }
    };
    if !script.is_file() {
        log(crate::core::live::NOTES_UNAVAILABLE);
        return Ok(());
    }
    let mut folders = script_folders(state, meetings, meeting_id).at(Stage::CatchUpMeetings)?;
    let script_lock = match crate::core::store_io::lock(&state.join("local/notes-script.lock")) {
        Ok(lock) => lock,
        Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => return Ok(()),
        Err(e) => return Err(Stage::CatchUpScriptLock.error(e)),
    };
    let (mut command, filtered) =
        script_command(&script, &folders, log).at(Stage::CatchUpScriptCommand)?;
    if filtered && folders.is_empty() {
        return Ok(());
    }
    if !filtered {
        folders = legacy_script_folders(meetings, meeting_id).at(Stage::CatchUpMeetings)?;
    }
    command
        .env("SIDENOTE_DATA_DIR", state)
        .env("SIDENOTE_NOTES_QUERY_EXE", std::env::current_exe()?)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let child = command.spawn().at(Stage::CatchUpScriptCommand);
    std::thread::spawn(move || {
        let _script_lock = script_lock;
        let _activity = activity;
        let result = script_result(child.and_then(|mut child| {
            if let Some(stdout) = child.stdout.take() {
                timings::capture(stdout, &folders, |message| log(&message));
            }
            child.wait().at(Stage::CatchUpScriptCommand)
        }));
        let succeeded = result.is_ok();
        for folder in &folders {
            if let Some(id) = folder.file_name().and_then(|n| n.to_str()) {
                let _ = crate::core::notes::draft::read(&crate::core::state_dir(), folder, id);
            }
        }
        script_markers(&folders, succeeded);
        if let Err(error) = result {
            log(&format!("catch-up: {error}"));
        }
    });
    Ok(())
}
fn script_result(result: io::Result<std::process::ExitStatus>) -> io::Result<()> {
    let status = result.at(Stage::CatchUpScriptCommand)?;
    if status.success() {
        Ok(())
    } else {
        Err(Stage::CatchUpScriptCommand.error(io::Error::other("Script exited unsuccessfully")))
    }
}

fn script_folders(
    state: &std::path::Path,
    meetings: &std::path::Path,
    meeting_id: Option<&str>,
) -> io::Result<Vec<PathBuf>> {
    Ok(fs::read_dir(meetings)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|folder| {
            folder
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|id| {
                    meeting_id.is_none_or(|requested| requested == id)
                        && (meetings == crate::core::meeting_parts::inputs(state)
                            || !crate::core::meeting_parts::held_in(state, meetings, id)
                                .unwrap_or(true))
                        && crate::core::recording_guard::eligibility(state, meetings, id)
                            .is_ok_and(|e| e == crate::core::recording_guard::Eligibility::Eligible)
                })
                && !folder.join("english-summary.json").is_file()
        })
        .collect())
}
pub(crate) fn catch_up_routing_automatic(meetings: &std::path::Path) -> io::Result<()> {
    catch_up_routing(meetings, |id| {
        crate::core::meeting_router::jobs::enqueue_automatic(id).map_err(io::Error::other)
    })
}
fn catch_up_routing(
    meetings: &std::path::Path,
    mut enqueue: impl FnMut(&str) -> io::Result<()>,
) -> io::Result<()> {
    for entry in fs::read_dir(meetings).at(Stage::CatchUpMeetings)? {
        let entry = entry.at(Stage::CatchUpMeetings)?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !id.is_empty()
            && id.bytes().any(|b| b.is_ascii_digit())
            && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
            && !crate::core::meeting_parts::held(&crate::core::state_dir(), &id)?
        {
            enqueue(&id).at(Stage::CatchUpRouting)?;
        }
    }
    Ok(())
}
#[cfg(windows)]
const LEGACY_SCRIPT_NOTICE: &str =
    "notes script does not accept MeetingFolders; short-recording filter not applied to the script";
fn legacy_script_folders(
    meetings: &std::path::Path,
    meeting_id: Option<&str>,
) -> io::Result<Vec<PathBuf>> {
    Ok(fs::read_dir(meetings)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|folder| {
            !folder.join("english-summary.json").is_file()
                && (meetings == crate::core::meeting_parts::inputs(&crate::core::state_dir())
                    || !crate::core::meeting_parts::held(
                        &crate::core::state_dir(),
                        folder.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                    )
                    .unwrap_or(true))
                && (meeting_id.is_some_and(|id| folder.file_name().is_some_and(|n| n == id))
                    || fs::read(folder.join("manifest.json"))
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                        .is_some_and(|m| m["status"] == "complete"))
        })
        .collect())
}
fn script_markers(folders: &[PathBuf], succeeded: bool) {
    for folder in folders {
        if !succeeded {
            let _ = fs::write(folder.join("notes-failed"), b"failed");
        } else {
            let _ = fs::remove_file(folder.join("notes-failed"));
        }
    }
}
#[cfg(windows)]
fn script_command(
    script: &std::path::Path,
    folders: &[PathBuf],
    report: impl FnOnce(&str),
) -> io::Result<(Command, bool)> {
    script_command_with_environment(script, folders, report, |command| {
        notes_environment::apply(
            command,
            notes_environment::read_user,
            |name| std::env::var_os(name),
            log,
        )
    })
}
#[cfg(windows)]
fn script_command_with_environment(
    script: &std::path::Path,
    folders: &[PathBuf],
    report: impl FnOnce(&str),
    refresh: impl FnOnce(&mut Command) -> io::Result<()>,
) -> io::Result<(Command, bool)> {
    let filtered = crate::platform::windows_powershell()
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-NonInteractive", "-Command",
            "$ErrorActionPreference='Stop'; if ((Get-Command -Name $env:SIDENOTE_NOTES_SCRIPT).Parameters.ContainsKey('MeetingFolders')) { exit 0 }; exit 1"])
        .env("SIDENOTE_NOTES_SCRIPT", script)
        .output().is_ok_and(|output| output.status.success());
    let mut command = crate::platform::windows_powershell();
    if filtered {
        command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-NonInteractive", "-Command",
            "$ErrorActionPreference='Stop'; $folders=@(ConvertFrom-Json $env:SIDENOTE_MEETING_FOLDERS); & $env:SIDENOTE_NOTES_SCRIPT -MeetingFolders $folders; if (!$?) { exit 1 }"])
            .env("SIDENOTE_NOTES_SCRIPT", script)
            .env("SIDENOTE_MEETING_FOLDERS", serde_json::to_string(folders)?);
    } else {
        report(LEGACY_SCRIPT_NOTICE);
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script);
    }
    refresh(&mut command)?;
    Ok((command, filtered))
}
fn catch_up() -> io::Result<()> {
    let _activity = crate::updates::activity().at(Stage::CatchUpActivity)?;
    if crate::core::meeting_router::jobs::configured() {
        catch_up_routing_automatic(&crate::core::meetings_dir()).at(Stage::CatchUpRouting)?;
        crate::core::meeting_router::jobs::start_worker().at(Stage::CatchUpRouting)?;
    }
    pipeline(None)
}
fn continue_after_pipeline(result: io::Result<()>, report: impl FnOnce(&str)) {
    if let Err(error) = result {
        report(&format!("catch-up: {}", Stage::CatchUpEnqueue.error(error)));
    }
}
// A streaming HTTP body, with no TLS or proxy needed for the loopback API.
struct Body {
    stream: BufReader<TcpStream>,
    chunked: bool,
    remaining: usize,
    ended: bool,
}
impl Read for Body {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.ended {
            return Ok(0);
        }
        if !self.chunked {
            return self.stream.read(out);
        }
        if self.remaining == 0 {
            let mut line = String::new();
            self.stream.read_line(&mut line)?;
            self.remaining = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| io::ErrorKind::InvalidData)?;
            if self.remaining == 0 {
                self.ended = true;
                return Ok(0);
            }
        }
        let len = out.len().min(self.remaining);
        let n = self.stream.read(&mut out[..len])?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.remaining -= n;
        if self.remaining == 0 {
            let mut end = [0; 2];
            self.stream.read_exact(&mut end)?;
            if end != *b"\r\n" {
                return Err(io::ErrorKind::InvalidData.into());
            }
        }
        Ok(n)
    }
}
fn listen(finished: &std::sync::mpsc::Sender<String>) -> Result<(), Problem> {
    let path = crate::core::meetings_dir()
        .parent()
        .ok_or(Problem::Value)?
        .join("local-api.json");
    let config: Discovery = serde_json::from_slice(&fs::read(path)?).map_err(|_| Problem::Value)?;
    // Reject header injection without ever including the credential in an error.
    if config.token.contains(['\r', '\n']) {
        return Err(Problem::Value);
    }
    let addr: SocketAddr = ([127, 0, 0, 1], config.port).into();
    let mut socket = TcpStream::connect_timeout(
        &addr,
        Duration::from_secs(if crate::service::active() { 2 } else { 60 }),
    )?;
    let (watch, stopped) = std::sync::mpsc::channel::<()>();
    let _watch = watch;
    if let Ok(control) = socket.try_clone() {
        std::thread::spawn(move || {
            while !crate::updates::listener_stopping() {
                if matches!(
                    stopped.recv_timeout(Duration::from_millis(250)),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                ) {
                    return;
                }
            }
            let _ = control.shutdown(std::net::Shutdown::Both);
        });
    }
    socket.set_read_timeout(Some(Duration::from_secs(60)))?;
    socket.set_write_timeout(Some(Duration::from_secs(60)))?;
    write!(
        socket,
        "GET /meetings/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n",
        config.port, config.token
    )?;
    let mut stream = BufReader::new(socket);
    let mut header = String::new();
    stream.read_line(&mut header)?;
    let status = header
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or(Problem::Value)?;
    if !(200..300).contains(&status) {
        return Err(Problem::Http(status));
    }
    let mut chunked = false;
    loop {
        header.clear();
        let n = stream.read_line(&mut header)?;
        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if header
            .to_ascii_lowercase()
            .starts_with("transfer-encoding:")
            && header.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }
    log("connected");
    crate::hex_engine::meeting_event();
    continue_after_pipeline(catch_up(), log);
    let mut body = BufReader::new(Body {
        stream,
        chunked,
        remaining: 0,
        ended: false,
    });
    let mut raw = Vec::new();
    loop {
        raw.clear();
        if body.read_until(b'\n', &mut raw)? == 0 {
            break;
        }
        let mut event_span = crate::core::idle_pass::Span::begin("sse_handling");
        event_span.bytes(raw.len());
        let decoded = String::from_utf8_lossy(&raw);
        let Some(data) = decoded.trim().strip_prefix("data:") else {
            continue;
        };
        let event: serde_json::Value = serde_json::from_str(data).map_err(|_| Problem::Value)?;
        crate::hex_engine::meeting_event();
        if event.get("type").and_then(|v| v.as_str()) == Some("meeting.finished") {
            let id = event.get("id").and_then(|v| v.as_str()).unwrap_or("None");
            // Only meeting ids belong in the shared log, never server-provided error bodies.
            if id.chars().all(|c| c.is_ascii_digit() || c == '-') {
                log(&format!("meeting finished {id}"));
            } else {
                log("meeting finished None");
            }
            let valid = !id.is_empty()
                && id.bytes().any(|c| c.is_ascii_digit())
                && id.bytes().all(|c| c.is_ascii_digit() || c == b'-');
            if valid {
                let _ = finished.send(id.to_owned());
            }
            #[cfg(not(windows))]
            continue_after_pipeline(
                pipeline_with(
                    &crate::core::state_dir(),
                    &crate::core::meetings_dir(),
                    valid.then_some(id),
                    true,
                ),
                log,
            );
            #[cfg(windows)]
            if !valid {
                continue_after_pipeline(pipeline(None), log);
            }
        }
    }
    Ok(())
}

// Exit independently of catch-up and worker joins, within the updater's 8 s wait.
pub(crate) fn watch_update_stop() {
    let mut recorder = heartbeat::Recorder::start();
    std::thread::spawn(move || {
        loop {
            if let Some(source) = crate::updates::listener_stop_source() {
                recorder.stop();
                log(&format!("exit reason=stop source=watcher/{source}"));
                std::process::exit(0);
            }
            recorder.tick();
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}

pub(crate) fn acquire_lock() -> io::Result<Option<fs::File>> {
    retry_lock(crate::updates::listener_lock, std::thread::sleep, 30)
}
fn retry_lock<T>(
    mut acquire: impl FnMut() -> io::Result<T>,
    mut wait: impl FnMut(Duration),
    retries: usize,
) -> io::Result<T> {
    for attempt in 0..=retries {
        match acquire() {
            Err(error)
                if cfg!(windows) && error.raw_os_error() == Some(32) && attempt < retries =>
            {
                wait(Duration::from_millis(100));
            }
            result => return result,
        }
    }
    unreachable!()
}
pub(crate) fn log_lock_exit(error: &io::Error) {
    // The repeating task can race a manual start; an existing listener is expected.
    if !(cfg!(windows) && error.raw_os_error() == Some(32)) {
        log(&format!("exit reason=lock-error kind={:?}", error.kind()));
    }
}
pub fn run() -> ! {
    let _instance = match acquire_lock() {
        Ok(lock) => lock,
        Err(error) => {
            log_lock_exit(&error);
            std::process::exit(0);
        }
    };
    if let Some(source) = crate::updates::listener_stop_source() {
        log(&format!("exit reason=stop source=startup/{source}"));
        std::process::exit(0);
    }
    watch_update_stop();
    let isolated = std::env::args().any(|a| a == "--listener-isolated")
        && std::env::args().any(|a| a == "--data-dir");
    #[cfg(all(windows, feature = "desktop"))]
    if crate::service::ui_allowed() {
        crate::meeting_icon::start(isolated);
    }
    if isolated && !std::env::args().any(|a| a == "--floating-icon-replay") {
        loop {
            if let Some(source) = crate::updates::listener_stop_source() {
                log(&format!("exit reason=stop source=isolated/{source}"));
                std::process::exit(0);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    status::start();
    // The watcher scans completed meetings even when the event API is unavailable.
    let finished = crate::core::meeting_watch::start(log);
    if let Err(error) = catch_up() {
        log(&format!(
            "startup catch-up: {}",
            Stage::CatchUpRouting.error(error)
        ));
    }
    std::thread::spawn(|| {
        loop {
            let pass = crate::core::idle_pass::Span::begin("listener_pass");
            let _ = crate::core::meeting_router::jobs::start_worker();
            if let Err(error) = crate::core::notes::queue::start_worker() {
                log(&format!(
                    "notes worker: {}",
                    Stage::NotesWorkerSpawn.error(error)
                ));
            }
            if crate::core::meeting_router::jobs::configured() {
                let _ = catch_up_routing_automatic(&crate::core::meetings_dir());
            }
            if let Err(error) = pipeline(None) {
                log(&format!(
                    "catch-up: {}",
                    Stage::CatchUpMeetings.error(error)
                ));
            }
            drop(pass);
            std::thread::sleep(Duration::from_secs(30));
        }
    });
    let mut last = String::new();
    loop {
        let problem = match listen(&finished) {
            Ok(()) => "stream ended".into(),
            Err(e) => e.message(),
        };
        if problem != last {
            log(&format!("{problem}; retrying every 30 s"));
            last = problem;
        }
        std::thread::sleep(Duration::from_secs(30));
    }
}

#[cfg(not(windows))]
fn script_command(
    _script: &std::path::Path,
    _folders: &[PathBuf],
    _report: impl FnOnce(&str),
) -> io::Result<(Command, bool)> {
    Err(crate::platform::unavailable())
}

pub(crate) struct Background {
    handles: Vec<std::thread::JoinHandle<()>>,
}
impl Drop for Background {
    fn drop(&mut self) {
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}
pub(crate) fn service_background() -> Background {
    let status = status::start();
    let (finished, watcher) = crate::core::meeting_watch::start_managed(log);
    let events = std::thread::spawn(move || {
        let mut last = String::new();
        while !crate::service::stopping() {
            let problem = match listen(&finished) {
                Ok(()) => "stream ended".into(),
                Err(error) => error.message(),
            };
            if problem != last {
                log(&format!("{problem}; retrying every 30 s"));
                last = problem;
            }
            crate::service::pause(Duration::from_secs(30));
        }
    });
    Background {
        handles: vec![events, watcher, status],
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn discovery_does_not_open_inputs_before_the_queue_captures_them() {
        use std::os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::{ffi::OsStrExt, fs::symlink},
        };
        for ancestor in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let meetings = root.path().join("meetings");
            let folder = meetings.join("123-4");
            fs::create_dir_all(&folder).unwrap();
            fs::write(
                folder.join("manifest.json"),
                br#"{"status":"complete","duration_ms":30000}"#,
            )
            .unwrap();
            fs::write(
                folder.join("transcript.ndjson"),
                serde_json::json!({"text":vec!["synthetic";25].join(" ")}).to_string(),
            )
            .unwrap();
            let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
            assert!(fd >= 0);
            let watch = unsafe { OwnedFd::from_raw_fd(fd) };
            let name = std::ffi::CString::new(folder.as_os_str().as_bytes()).unwrap();
            assert!(unsafe { libc::inotify_add_watch(fd, name.as_ptr(), libc::IN_OPEN) } >= 0);
            catch_up_routing(&meetings, |id| {
                let mut events = [0u8; 4096];
                assert_eq!(
                    unsafe {
                        libc::read(watch.as_raw_fd(), events.as_mut_ptr().cast(), events.len())
                    },
                    -1,
                    "discovery opened unpinned inputs before queue capture"
                );
                assert_eq!(io::Error::last_os_error().kind(), io::ErrorKind::WouldBlock);
                if ancestor {
                    fs::rename(&meetings, root.path().join("saved")).unwrap();
                    symlink(root.path().join("saved"), &meetings).unwrap();
                } else {
                    fs::rename(folder.join("transcript.ndjson"), folder.join("saved")).unwrap();
                    symlink(folder.join("saved"), folder.join("transcript.ndjson")).unwrap();
                }
                assert!(
                    crate::core::meeting_router::inputs::Snapshot::capture(&meetings.join(id))
                        .is_err()
                );
                Ok(())
            })
            .unwrap();
        }
    }
    #[cfg(windows)]
    fn script_command(
        script: &std::path::Path,
        folders: &[PathBuf],
        report: impl FnOnce(&str),
    ) -> io::Result<(Command, bool)> {
        super::script_command_with_environment(script, folders, report, |command| {
            notes_environment::apply(
                command,
                |_| Ok(Some("synthetic environment value".into())),
                |_| None,
                |_| panic!("nothing missing"),
            )
        })
    }
    #[cfg(windows)]
    #[test]
    fn notes_script_runs_without_the_inherited_module_path() {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("notes.ps1");
        fs::write(&script, "param()\n").unwrap();
        let (command, _) = script_command(&script, &[], |_| {}).unwrap();
        assert!(command.get_envs().any(|(key, value)| {
            key == std::ffi::OsStr::new("PSModulePath") && value.is_none()
        }));
    }
    #[cfg(windows)]
    #[test]
    fn listener_retries_contention_and_acquires_after_the_probe_releases() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("listener.lock");
        let open = || {
            OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .share_mode(0)
                .open(&path)
        };
        let mut probe = Some(open().unwrap());
        let mut waits = 0;
        let listener = retry_lock(
            open,
            |_| {
                waits += 1;
                drop(probe.take());
            },
            30,
        )
        .unwrap();
        assert_eq!(waits, 1);
        assert_eq!(open().unwrap_err().raw_os_error(), Some(32));
        drop(listener);
        assert!(open().is_ok());
    }
    #[cfg(windows)]
    #[test]
    fn lock_retry_is_bounded_and_does_not_retry_other_errors() {
        for code in [32, 5, 112] {
            let mut waits = 0;
            let error = retry_lock::<()>(
                || Err(io::Error::from_raw_os_error(code)),
                |_| waits += 1,
                30,
            )
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(code));
            assert_eq!(waits, if code == 32 { 30 } else { 0 });
        }
    }
    #[test]
    fn script_execution_failures_keep_only_stage_and_error_kind() {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::TimedOut] {
            let error =
                script_result(Err(io::Error::new(kind, "synthetic private detail"))).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("CATCH_UP_SCRIPT_COMMAND ({kind:?})")
            );
        }
        let error = script_result(Ok(std::process::ExitStatus::from_raw(1))).unwrap_err();
        assert_eq!(error.to_string(), "CATCH_UP_SCRIPT_COMMAND (Other)");
        assert!(script_result(Ok(std::process::ExitStatus::from_raw(0))).is_ok());
    }

    #[test]
    fn discovery_defers_to_the_pinned_queue_gate_and_scripts_share_it() {
        let root = std::env::temp_dir().join(format!(
            "rec-guard-listener-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let meetings = root.join("meetings");
        for (id, duration) in [("1793946600000-1", 29999), ("1793946600000-2", 30000)] {
            let folder = meetings.join(id);
            fs::create_dir_all(&folder).unwrap();
            fs::write(
                folder.join("manifest.json"),
                serde_json::json!({"status":"complete","duration_ms":duration}).to_string(),
            )
            .unwrap();
            fs::write(
                folder.join("transcript.ndjson"),
                serde_json::json!({"text":vec!["synthetic";25].join(" ")}).to_string(),
            )
            .unwrap();
        }
        let mut queued = Vec::new();
        catch_up_routing(&meetings, |id| {
            if crate::core::recording_guard::eligibility(&root, &meetings, id)?
                == crate::core::recording_guard::Eligibility::Eligible
            {
                queued.push(id.to_owned());
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(queued, ["1793946600000-2"]);
        let folders = script_folders(&root, &meetings, None).unwrap();
        assert_eq!(folders, [meetings.join("1793946600000-2")]);
        assert!(
            script_folders(&root, &meetings, Some("1793946600000-1"))
                .unwrap()
                .is_empty()
        );
    }
    fn script_fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rec-guard-script-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    #[cfg(windows)]
    fn legacy_no_param_script_runs_without_arguments_or_folder_environment_and_logs_once() {
        let root = script_fixture();
        let script = root.join("notes.ps1");
        let output = root.join("legacy.json");
        let logfile = root.join("listener.log");
        fs::write(&script, "@{arguments=@($args); parameters=@($PSBoundParameters.Keys); folders=$env:SIDENOTE_MEETING_FOLDERS} | ConvertTo-Json | Set-Content -LiteralPath $env:REC_GUARD_OUTPUT").unwrap();
        let (mut command, filtered) =
            script_command(&script, &[root.join("1793946600000-2")], |message| {
                log_at(&logfile, message)
            })
            .unwrap();
        assert!(!filtered);
        // Eight synthetic settings plus the removed PSModulePath.
        assert_eq!(command.get_envs().count(), 9);
        assert!(command.get_envs().all(|(key, value)| {
            if key == std::ffi::OsStr::new("PSModulePath") {
                value.is_none()
            } else {
                value == Some(std::ffi::OsStr::new("synthetic environment value"))
            }
        }));
        assert_eq!(
            command
                .get_args()
                .map(|v| v.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                "-NoProfile".to_owned(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                script.to_string_lossy().into_owned()
            ]
        );
        let result = command.env("REC_GUARD_OUTPUT", &output).output().unwrap();
        assert!(result.status.success());
        let actual: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(actual["arguments"], serde_json::json!([]));
        assert_eq!(actual["parameters"], serde_json::json!([]));
        assert_eq!(
            actual["folders"].as_str(),
            std::env::var("SIDENOTE_MEETING_FOLDERS").ok().as_deref()
        );
        let log = fs::read_to_string(logfile).unwrap();
        assert_eq!(log.lines().count(), 1);
        assert!(log.contains(LEGACY_SCRIPT_NOTICE));
    }
    #[test]
    #[cfg(windows)]
    fn param_script_receives_only_eligible_folders() {
        let root = script_fixture();
        let meetings = root.join("meetings");
        let script = root.join("notes.ps1");
        let output = root.join("folders.json");
        for (id, duration) in [("1793946600000-1", 29999), ("1793946600000-2", 30000)] {
            let folder = meetings.join(id);
            fs::create_dir_all(&folder).unwrap();
            fs::write(
                folder.join("manifest.json"),
                serde_json::json!({"status":"complete","duration_ms":duration}).to_string(),
            )
            .unwrap();
            fs::write(
                folder.join("transcript.ndjson"),
                serde_json::json!({"text":vec!["synthetic";25].join(" ")}).to_string(),
            )
            .unwrap();
        }
        fs::write(&script, "param([string[]]$MeetingFolders)\nConvertTo-Json -InputObject @($MeetingFolders) | Set-Content -LiteralPath $env:REC_GUARD_OUTPUT").unwrap();
        let folders = script_folders(&root, &meetings, None).unwrap();
        let (mut command, filtered) = script_command(&script, &folders, |_| {
            panic!("parameter script must not log a legacy notice")
        })
        .unwrap();
        assert!(filtered);
        assert!(
            command
                .env("REC_GUARD_OUTPUT", &output)
                .output()
                .unwrap()
                .status
                .success()
        );
        let actual: Vec<PathBuf> = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(actual, [meetings.join("1793946600000-2")]);
        assert_eq!(legacy_script_folders(&meetings, None).unwrap().len(), 2);
    }
    #[test]
    #[cfg(windows)]
    fn param_script_keeps_mixed_separator_paths_by_meeting_id() {
        let root = script_fixture();
        let meetings = root.join("hex/meetings");
        let id = "1793946600000-2";
        for name in [id, "1793946600000-3"] {
            fs::create_dir_all(meetings.join(name)).unwrap();
        }
        let mixed = PathBuf::from(format!(r"{}\hex/meetings\{id}", root.display()));
        assert!(mixed.to_string_lossy().contains(r"hex/meetings\"));
        let script = root.join("leaf-filter.ps1");
        let output = root.join("kept.json");
        fs::write(&script, r#"param([string[]]$MeetingFolders)
$allowed = if ($PSBoundParameters.ContainsKey('MeetingFolders')) { @($MeetingFolders | ForEach-Object { Split-Path -Leaf $_ }) }
if ((Get-ExecutionPolicy -Scope Process) -ne 'Bypass') { throw 'Expected the dev16 execution policy' }
$kept = @(foreach ($dir in Get-ChildItem -LiteralPath $env:REC_GUARD_MEETINGS -Directory) {
    if ($null -ne $allowed -and $dir.Name -notin $allowed) { continue }
    $dir.Name
})
ConvertTo-Json -InputObject $kept | Set-Content -LiteralPath $env:REC_GUARD_OUTPUT
"#).unwrap();
        let (mut command, filtered) = script_command(&script, &[mixed], |_| {
            panic!("parameter script must not use the legacy fallback")
        })
        .unwrap();
        assert!(filtered);
        let result = command
            .env("REC_GUARD_MEETINGS", &meetings)
            .env("REC_GUARD_OUTPUT", &output)
            .output()
            .unwrap();
        assert!(result.status.success());
        let kept: Vec<String> = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(kept, [id]);
    }
    #[test]
    fn script_failure_markers_match_dev16() {
        let root = script_fixture();
        let folders = [root.clone()];
        script_markers(&folders, false);
        assert_eq!(fs::read(root.join("notes-failed")).unwrap(), b"failed");
        script_markers(&folders, true);
        assert!(!root.join("notes-failed").exists());
    }
    #[test]
    fn enqueue_failure_is_reported_without_ending_event_handling() {
        let mut messages = Vec::new();
        for result in [Err(io::Error::other("private failure")), Ok(())] {
            continue_after_pipeline(result, |message| messages.push(message.to_owned()));
        }
        assert_eq!(messages, ["catch-up: CATCH_UP_ENQUEUE (Other)"]);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_script_tests {
    #[test]
    fn legacy_powershell_is_explicitly_unavailable() {
        let result =
            super::script_command(std::path::Path::new("/synthetic/notes.ps1"), &[], |_| {});
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Unsupported);
    }
}
