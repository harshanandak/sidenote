//! Hex stays a separate, unmodified process. No method starts or restarts it.
use crate::platform::CommandExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fmt, fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime},
};

mod audio_retention;
pub use audio_retention::AudioRetention;

const PROTOCOL: u64 = 4;
const LIMIT: usize = 1024 * 1024;
const STATUS_TIMEOUT: Duration = Duration::from_millis(750);
const CALL_TIMEOUT: Duration = Duration::from_secs(12);
#[cfg(windows)]
const POLL: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HexError {
    HexNotRunning,
    Unsupported,
    Timeout,
    Failed {
        code: Option<String>,
        stderr_summary: &'static str,
    },
}
impl fmt::Display for HexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for HexError {}
pub type Result<T> = std::result::Result<T, HexError>;
fn failed(summary: &'static str) -> HexError {
    HexError::Failed {
        code: None,
        stderr_summary: summary,
    }
}
fn io_error(e: std::io::Error) -> HexError {
    match e.kind() {
        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound => {
            HexError::HexNotRunning
        }
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => HexError::Timeout,
        _ => failed("local transport failed; details suppressed"),
    }
}
#[derive(Deserialize)]
struct Discovery {
    port: u16,
    token: String,
}
fn discovery() -> Result<Discovery> {
    let d: Discovery =
        serde_json::from_slice(&fs::read(hex_dir()?.join("local-api.json")).map_err(io_error)?)
            .map_err(|_| failed("invalid discovery; credential suppressed"))?;
    if d.port == 0 || d.token.is_empty() || !d.token.bytes().all(|c| c.is_ascii_graphic()) {
        return Err(failed("invalid discovery; credential suppressed"));
    }
    Ok(d)
}
/// Privacy-filtered status for the headless listener. No model or title is retained.
pub(crate) fn listener_status() -> Result<Value> {
    let v = control_response(json!({"command":"status"}), STATUS_TIMEOUT)?;
    if v["status"] != "status" {
        return Err(failed("unexpected status reply"));
    }
    Ok(json!({
        "app_version": safe_label(&v["app_version"]), "protocol": v["version"].as_u64(),
        "meeting_state": known_state(&v["meeting"]["state"]),
        "capture_state": known_state(&v["capture"]["state"]),
        "capture_kind": v["capture"]["recording"].as_str().filter(|s| matches!(*s,"dictation"|"service")),
        "can_process": v["version"].as_u64()==Some(PROTOCOL) && v["capture"]["state"]=="idle" && v["model"]["preparing"].is_null(),
        "elapsed": v["meeting"]["elapsed_seconds"].as_u64(),
        "id": v["meeting"]["id"].as_str().filter(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit() || c == b'-')),
        "has_elapsed": !v["meeting"]["elapsed_seconds"].is_null(),
        "has_id": !v["meeting"]["id"].is_null()
    }))
}
fn known_state(value: &Value) -> &str {
    match value.as_str() {
        Some(
            s @ ("starting" | "recording" | "paused" | "stopping" | "finishing" | "idle"
            | "stopped" | "complete" | "finished" | "transcribing"),
        ) => s,
        _ => "unknown",
    }
}
fn safe_label(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| {
        s.len() <= 80
            && !s.is_empty()
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    })
}

fn hex_dir() -> Result<PathBuf> {
    crate::core::meetings_dir()
        .parent()
        .map(PathBuf::from)
        .ok_or(HexError::HexNotRunning)
}
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or(HexError::Timeout)
}
// Each read uses the remaining total budget, so a trickling peer cannot extend the call forever.
fn line(reader: &mut BufReader<TcpStream>, deadline: Instant) -> Result<String> {
    let mut bytes = Vec::new();
    loop {
        reader
            .get_ref()
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(io_error)?;
        let buffer = reader.fill_buf().map_err(io_error)?;
        if buffer.is_empty() {
            return Err(failed("truncated HTTP framing"));
        }
        let end = buffer.iter().position(|c| *c == b'\n');
        let len = end.map_or(buffer.len(), |i| i + 1);
        if bytes.len() + len > 8192 {
            return Err(failed("HTTP line too large"));
        }
        bytes.extend_from_slice(&buffer[..len]);
        reader.consume(len);
        if end.is_some() {
            break;
        }
    }
    String::from_utf8(bytes).map_err(|_| failed("invalid HTTP framing"))
}
fn read_bytes(
    reader: &mut BufReader<TcpStream>,
    mut len: usize,
    deadline: Instant,
    out: &mut Vec<u8>,
) -> Result<()> {
    if len > LIMIT.saturating_sub(out.len()) {
        return Err(failed("control reply too large"));
    }
    let mut buffer = [0; 8192];
    while len != 0 {
        reader
            .get_ref()
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(io_error)?;
        let n = reader
            .read(&mut buffer[..len.min(8192)])
            .map_err(io_error)?;
        if n == 0 {
            return Err(failed("truncated control reply"));
        }
        out.extend_from_slice(&buffer[..n]);
        len -= n;
    }
    Ok(())
}
fn control(request: Value, timeout: Duration) -> Result<Value> {
    let response = control_response(request, timeout)?;
    if response["version"].as_u64() != Some(PROTOCOL) {
        return Err(HexError::Unsupported);
    }
    Ok(response)
}
fn control_response(request: Value, timeout: Duration) -> Result<Value> {
    let deadline = Instant::now() + timeout;
    let d = discovery()?;
    control_response_at(request, &d, deadline)
}
fn control_response_at(mut request: Value, d: &Discovery, deadline: Instant) -> Result<Value> {
    request["version"] = json!(PROTOCOL);
    let body = request.to_string();
    if body.len() > 64 * 1024 {
        return Err(failed("control request too large"));
    }
    let addr: SocketAddr = ([127, 0, 0, 1], d.port).into();
    let mut socket = TcpStream::connect_timeout(&addr, remaining(deadline)?).map_err(io_error)?;
    let request = format!(
        "POST /control HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        d.port,
        d.token,
        body.len(),
        body
    );
    let mut pending = request.as_bytes();
    while !pending.is_empty() {
        socket
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(io_error)?;
        let n = socket.write(pending).map_err(io_error)?;
        if n == 0 {
            return Err(failed("control request truncated"));
        }
        pending = &pending[n..];
    }
    let mut reader = BufReader::new(socket);
    let first = line(&mut reader, deadline)?;
    let status: u16 = first
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| failed("invalid HTTP status"))?;
    match status {
        200 => (),
        404 => return Err(HexError::Unsupported),
        504 => return Err(HexError::Timeout),
        _ => return Err(failed("control HTTP error; body suppressed")),
    }
    let mut length = None;
    let mut chunked = false;
    let mut header_size = 0;
    loop {
        let h = line(&mut reader, deadline)?;
        header_size += h.len();
        if header_size > 32768 {
            return Err(failed("HTTP headers too large"));
        }
        if h == "\r\n" {
            break;
        }
        if let Some((key, value)) = h.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| failed("invalid content length"))?,
                );
            }
            if key.eq_ignore_ascii_case("transfer-encoding") {
                if !value.trim().eq_ignore_ascii_case("chunked") {
                    return Err(failed("unsupported HTTP encoding"));
                }
                chunked = true;
            }
        }
    }
    let mut bytes = Vec::new();
    if chunked {
        loop {
            let h = line(&mut reader, deadline)?;
            let size = usize::from_str_radix(h.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| failed("invalid chunk"))?;
            if size == 0 {
                break;
            }
            read_bytes(&mut reader, size, deadline, &mut bytes)?;
            if line(&mut reader, deadline)? != "\r\n" {
                return Err(failed("invalid chunk ending"));
            }
        }
    } else if let Some(n) = length {
        read_bytes(&mut reader, n, deadline, &mut bytes)?;
    } else {
        loop {
            reader
                .get_ref()
                .set_read_timeout(Some(remaining(deadline)?))
                .map_err(io_error)?;
            let mut buffer = [0; 8192];
            let n = reader.read(&mut buffer).map_err(io_error)?;
            if n == 0 {
                break;
            }
            if bytes.len() + n > LIMIT {
                return Err(failed("control reply too large"));
            }
            bytes.extend_from_slice(&buffer[..n]);
        }
    }
    let response: Value = serde_json::from_slice(&bytes)
        .map_err(|_| failed("invalid control JSON; body suppressed"))?;
    if response["status"] == "error" {
        let code = response["code"].as_str().unwrap_or("");
        return Err(match code {
            "not-running" => HexError::HexNotRunning,
            "unsupported" | "unknown-command" | "version-mismatch" => HexError::Unsupported,
            "control-timeout" => HexError::Timeout,
            _ => HexError::Failed {
                code: match code {
                    "unknown-setting" | "read-only" | "invalid-value" | "shortcut-in-use"
                    | "feature-off" | "not-ready" | "busy" | "not-recording" | "failed"
                    | "internal" | "unavailable" => Some(code.into()),
                    _ => None,
                },
                stderr_summary: "Hex refused the command; message and values suppressed",
            },
        });
    }
    Ok(response)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Supported,
    Unsupported,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Feature {
    Status,
    RecordStart,
    RecordStop,
    DictationStop,
    DictationToggle,
    PasteLast,
    SettingsList,
    SettingsGet,
    SettingsSet,
    VocabularyList,
    VocabularyAdd,
    VocabularyRemove,
    ModelsList,
    ModelStatus,
    ModelUse,
    ModelInstall,
    WindowShow,
}
pub type Capabilities = BTreeMap<Feature, Support>;
const FEATURES: [Feature; 17] = [
    Feature::Status,
    Feature::RecordStart,
    Feature::RecordStop,
    Feature::DictationStop,
    Feature::DictationToggle,
    Feature::PasteLast,
    Feature::SettingsList,
    Feature::SettingsGet,
    Feature::SettingsSet,
    Feature::VocabularyList,
    Feature::VocabularyAdd,
    Feature::VocabularyRemove,
    Feature::ModelsList,
    Feature::ModelStatus,
    Feature::ModelUse,
    Feature::ModelInstall,
    Feature::WindowShow,
];

#[derive(Clone, Deserialize, PartialEq)]
pub struct ModelState {
    pub id: Option<String>,
    pub name: Option<String>,
    pub language: Option<String>,
    pub installed: Option<bool>,
    pub preparing: Option<String>,
    pub downloaded_bytes: Option<u64>,
}
#[derive(Clone, PartialEq)]
pub struct Recording {
    pub id: Option<String>,
    pub title: Option<String>,
    pub since: Option<SystemTime>,
    pub elapsed_seconds: Option<u64>,
}
#[derive(Clone, PartialEq)]
pub struct EngineState {
    pub running: bool,
    pub version: String,
    pub protocol: u64,
    pub recording: Option<Recording>,
    pub meeting_state: String,
    pub dictating: Option<bool>,
    pub capture_state: String,
    pub model: ModelState,
    pub labs: Box<crate::hex_labs::Snapshot>,
}
#[derive(Clone, Deserialize)]
pub struct Setting {
    pub key: String,
    #[serde(
        default,
        rename = "name",
        alias = "label",
        alias = "title",
        alias = "display_name"
    )]
    pub display_name: Option<String>,
    pub value: Value,
    pub kind: String,
    pub writable: bool,
    #[serde(default)]
    pub choices: Vec<Value>,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub available: Option<bool>,
    #[serde(default)]
    pub secret: bool,
}
#[derive(Clone, Deserialize)]
pub struct VocabularyEntry {
    pub term: String,
    #[serde(default)]
    pub sounds_like: Vec<String>,
}
#[derive(Clone, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub languages: Vec<String>,
    pub size: String,
    pub installed: bool,
    pub selected: bool,
    #[serde(default)]
    pub streams: bool,
}
// Values returned to UI callers are never included in debug output.
macro_rules! private_debug {
    ($($t:ty),*) => { $(impl fmt::Debug for $t {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(concat!(stringify!($t), " { values redacted }"))
        }
    })* };
}
private_debug!(
    EngineState,
    Recording,
    ModelState,
    Setting,
    VocabularyEntry,
    ModelInfo
);

#[derive(Default)]
struct Cache {
    version: String,
    cli: Option<PathBuf>,
    capabilities: Option<Capabilities>,
    reported: BTreeMap<String, bool>,
}
#[derive(Clone, Default)]
pub struct HexEngine {
    cache: Arc<Mutex<Cache>>,
}
pub type RecordStopResult = (Option<StatusSample>, Result<()>);
pub type StatusSample = (Instant, Result<EngineState>);

pub fn sample_status(read: impl FnOnce() -> Result<EngineState>) -> StatusSample {
    (Instant::now(), read())
}

impl HexEngine {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn status(&self) -> Result<EngineState> {
        self.status_with_budget(STATUS_TIMEOUT)
    }
    /// An explicit user action may confirm state using the existing command budget.
    pub fn status_for_command(&self) -> Result<EngineState> {
        self.status_with_budget(CALL_TIMEOUT)
    }
    fn status_with_budget(&self, budget: Duration) -> Result<EngineState> {
        let v = control(json!({"command":"status"}), budget)?;
        self.parse_status(v)
    }
    fn parse_status(&self, v: Value) -> Result<EngineState> {
        if v["status"] != "status" {
            return Err(failed("unexpected status reply"));
        }
        let version = v["app_version"]
            .as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
            })
            .ok_or_else(|| failed("invalid app version"))?
            .to_owned();
        {
            let mut cache = self
                .cache
                .lock()
                .map_err(|_| failed("engine cache unavailable"))?;
            if cache.version != version {
                cache.cli = Some(hex_dir()?.join("versions").join(&version).join("hex.com"));
                cache.version = version.clone();
                cache.capabilities = None;
            }
            let reported = v["capabilities"]
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_bool().map(|b| (k.clone(), b)))
                        .collect()
                })
                .unwrap_or_default();
            if cache.reported != reported {
                cache.capabilities = None;
            }
            cache.reported = reported;
        }
        let meeting = &v["meeting"];
        let elapsed = meeting["elapsed_seconds"].as_u64();
        let recording = match meeting["state"].as_str() {
            Some("recording" | "starting") => Some(Recording {
                id: meeting["id"].as_str().map(str::to_owned),
                title: meeting["title"].as_str().map(str::to_owned),
                since: elapsed.and_then(|s| SystemTime::now().checked_sub(Duration::from_secs(s))),
                elapsed_seconds: elapsed,
            }),
            _ => None,
        };
        let capture_state = v["capture"]["state"]
            .as_str()
            .unwrap_or("unknown")
            .to_owned();
        let dictating = match (capture_state.as_str(), v["capture"]["recording"].as_str()) {
            ("recording", Some("dictation")) => Some(true),
            ("idle" | "transcribing", _) | ("recording", Some("service")) => Some(false),
            _ => None,
        };
        Ok(EngineState {
            labs: Box::new(crate::hex_labs::Snapshot::status(&v)),
            running: true,
            version,
            protocol: PROTOCOL,
            recording,
            meeting_state: meeting["state"].as_str().unwrap_or("unknown").into(),
            dictating,
            capture_state,
            model: serde_json::from_value(v["model"].clone())
                .map_err(|_| failed("invalid model state"))?,
        })
    }
    pub fn labs(&self) -> Result<crate::hex_labs::Snapshot> {
        let settings = lab_settings("labs");
        if let Ok(mut settings) = settings {
            for key in [
                "paste_with_shift",
                "share_dictation_shortcut",
                "keyboard_model_switching.enabled",
                "meetings.enabled",
                "capture.enabled",
            ] {
                if let Ok(extra) = lab_settings(key) {
                    settings.extend(extra);
                }
            }
            let snapshot = crate::hex_labs::Snapshot::settings(&settings);
            if !snapshot.states.is_empty() {
                return Ok(snapshot);
            }
        }
        let cli = newest_cli()?;
        let text = hidden_help(&cli, &["labs", "list"], Instant::now() + CALL_TIMEOUT)?;
        let mut snapshot = crate::hex_labs::Snapshot::list(&text);
        if let Ok(state) = self.status() {
            for (id, value) in state.labs.states {
                snapshot.states.entry(id).or_insert(value);
            }
        }
        if snapshot.states.is_empty() {
            return Err(failed("Hex Labs state unavailable"));
        }
        Ok(snapshot)
    }
    /// Called only by the settings confirmation; never enable an unreviewed or unknown state.
    pub fn enable_lab(&self, id: &str) -> Result<()> {
        if !self.labs()?.can_enable(id) {
            return Err(failed("lab is not eligible to enable"));
        }
        // Refuse an offline write: the CLI otherwise edits saved settings even when Hex is closed.
        self.status()?;
        hidden_help(
            &newest_cli()?,
            &["labs", "on", id],
            Instant::now() + CALL_TIMEOUT,
        )?;
        if self.labs()?.state(id) != Some(true) {
            return Err(failed(
                "Hex did not confirm that the lab is on; refresh its state before trying again",
            ));
        }
        Ok(())
    }
    pub fn model_status(&self) -> Result<ModelState> {
        Ok(self.status()?.model)
    }

    /// Only the setup button calls this. Never use `labs on meetings` on Windows.
    pub fn enable_meetings(&self) -> Result<(Instant, EngineState)> {
        let sent = Instant::now();
        let state = self.status()?;
        if state.labs.state("meetings") == Some(true) {
            return Ok((sent, state));
        }
        if state.labs.state("meetings") != Some(false)
            || self.capabilities()?.get(&Feature::SettingsSet) != Some(&Support::Supported)
            || !crate::hex_labs::meetings_setting_can_enable(
                &self.settings_get("meetings.enabled")?,
            )
        {
            return Err(HexError::Unsupported);
        }
        self.settings_set("meetings.enabled", "true")?;
        let sent = Instant::now();
        let state = self.status()?;
        if state.labs.state("meetings") != Some(true) {
            return Err(failed(
                "Hex did not confirm that Meetings is on. Check again.",
            ));
        }
        Ok((sent, state))
    }
    pub(crate) fn stop_forgotten(&self, id: &str) -> Result<()> {
        // Recheck immediately before the CLI call, including after a slow file read.
        let status = self.status()?;
        if crate::core::meeting_watch::recording_id(&status, &crate::core::meetings_dir())
            .as_deref()
            != Some(id)
        {
            return Err(failed("recording changed before auto-stop"));
        }
        let cli = self
            .cache
            .lock()
            .map_err(|_| failed("engine cache unavailable"))?
            .cli
            .clone()
            .ok_or(HexError::Unsupported)?;
        hidden_help(
            &cli,
            &["meetings", "stop", "--json"],
            Instant::now() + CALL_TIMEOUT,
        )?;
        Ok(())
    }
    pub fn capabilities(&self) -> Result<Capabilities> {
        self.status()?;
        let (version, cli, reported, cached) = {
            let cache = self
                .cache
                .lock()
                .map_err(|_| failed("engine cache unavailable"))?;
            (
                cache.version.clone(),
                cache.cli.clone(),
                cache.reported.clone(),
                cache.capabilities.clone(),
            )
        };
        if let Some(caps) = cached {
            return Ok(caps);
        }
        let cli = cli.ok_or(HexError::Unsupported)?;
        let deadline = Instant::now() + CALL_TIMEOUT;
        let mut helps = BTreeMap::new();
        for group in [
            "",
            "meetings",
            "dictation",
            "settings",
            "vocabulary",
            "model",
            "window",
        ] {
            let args: Vec<&str> = if group.is_empty() {
                vec!["--help"]
            } else {
                vec![group, "--help"]
            };
            let help = hidden_help(&cli, &args, deadline);
            helps.insert(group, help.ok());
        }
        let mut caps = Capabilities::new();
        for (feature, group, word, capability) in [
            (Feature::RecordStart, "meetings", "record", Some("meetings")),
            (Feature::RecordStop, "meetings", "stop", Some("meetings")),
            (
                Feature::DictationStop,
                "dictation",
                "stop",
                Some("listener_control"),
            ),
            (
                Feature::DictationToggle,
                "dictation",
                "toggle",
                Some("listener_control"),
            ),
            (
                Feature::PasteLast,
                "",
                "paste-last",
                Some("listener_control"),
            ),
            (Feature::SettingsList, "settings", "list", None),
            (Feature::SettingsGet, "settings", "get", None),
            (Feature::SettingsSet, "settings", "set", None),
            (Feature::VocabularyList, "vocabulary", "list", None),
            (Feature::VocabularyAdd, "vocabulary", "add", None),
            (Feature::VocabularyRemove, "vocabulary", "remove", None),
            (Feature::ModelsList, "model", "list", None),
            (Feature::ModelUse, "model", "use", None),
            (Feature::ModelInstall, "model", "install", None),
            (Feature::WindowShow, "window", "show", None),
        ] {
            let support = if capability.is_some_and(|c| reported.get(c) == Some(&false)) {
                Support::Unsupported
            } else {
                match helps[group].as_ref() {
                    Some(h)
                        if h.split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                            .any(|w| w == word) =>
                    {
                        Support::Supported
                    }
                    Some(_) => Support::Unsupported,
                    None => Support::Unknown,
                }
            };
            caps.insert(feature, support);
        }
        caps.insert(Feature::Status, Support::Supported);
        caps.insert(Feature::ModelStatus, Support::Supported);
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| failed("engine cache unavailable"))?;
        if cache.version == version && caps.values().all(|s| *s != Support::Unknown) {
            cache.capabilities = Some(caps.clone());
        }
        Ok(caps)
    }
    fn call(&self, feature: Feature, request: Value, expected: &str) -> Result<Value> {
        if feature == Feature::WindowShow && !crate::service::ui_allowed() {
            return Err(failed(crate::platform::UNAVAILABLE));
        }
        if self.capabilities()?.get(&feature) != Some(&Support::Supported) {
            return Err(HexError::Unsupported);
        }
        let v = control(request, CALL_TIMEOUT)?;
        if v["status"] != expected {
            return Err(failed("unexpected control reply; values suppressed"));
        }
        Ok(v)
    }
    fn change(&self, feature: Feature, request: Value) -> Result<()> {
        self.call(feature, request, "done").map(|_| ())
    }
    pub fn record_start(&self, title: Option<&str>, cached: &Capabilities) -> Result<()> {
        let mut v = json!({"command":"meeting-record"});
        if let Some(title) = title {
            v["title"] = json!(title);
        }
        if cached.get(&Feature::RecordStart) != Some(&Support::Supported) {
            return Err(HexError::Unsupported);
        }
        let reply = control(v, CALL_TIMEOUT)?;
        if reply["status"] != "done" {
            return Err(failed("unexpected start reply; values suppressed"));
        }
        Ok(())
    }
    pub fn record_stop(&self, expected_id: &str, cached: &Capabilities) -> RecordStopResult {
        let mut observed = None;
        let result = self.record_stop_checked(expected_id, cached, &mut observed);
        (observed, result)
    }

    fn record_stop_checked(
        &self,
        expected_id: &str,
        cached: &Capabilities,
        observed: &mut Option<StatusSample>,
    ) -> Result<()> {
        let refused = |code: &str| HexError::Failed {
            code: Some(code.into()),
            stderr_summary: "Stop refused; values suppressed",
        };
        if cached.get(&Feature::RecordStop) != Some(&Support::Supported) {
            return Err(HexError::Unsupported);
        }
        let deadline = Instant::now() + CALL_TIMEOUT;
        // Pin both calls to one server lease; never rediscover a replacement between them.
        let endpoint = discovery().map_err(|_| refused("stop-unconfirmed"))?;
        let request = |command| {
            let reply = control_response_at(json!({"command":command}), &endpoint, deadline)?;
            if reply["version"].as_u64() != Some(PROTOCOL) {
                return Err(HexError::Unsupported);
            }
            Ok(reply)
        };
        let sent = Instant::now();
        let current = request("status").map_err(|_| refused("stop-unconfirmed"))?;
        if current["status"] != "status" {
            return Err(refused("stop-unconfirmed"));
        }
        if current["meeting"]["state"] == "idle" {
            return Err(refused("not-recording"));
        }
        if !matches!(
            current["meeting"]["state"].as_str(),
            Some("recording" | "starting")
        ) || (!current["meeting"]["id"].is_null()
            && current["meeting"]["id"].as_str() != Some(expected_id))
        {
            return Err(refused("recording-changed"));
        }
        if current["capabilities"]["meetings"] == false {
            return Err(HexError::Unsupported);
        }
        *observed = self
            .parse_status(current)
            .ok()
            .map(|status| (sent, Ok(status)));
        // Hex has no atomic ID-scoped stop. This is the caller-requested status-check fallback.
        let reply = request("meeting-stop")?;
        if reply["status"] != "done" {
            return Err(refused("stop-unconfirmed"));
        }
        if reply["changed"] == false {
            return Err(refused("not-recording"));
        }
        Ok(())
    }
    pub fn dictation_stop(&self) -> Result<()> {
        self.change(
            Feature::DictationStop,
            json!({"command":"dictation","action":"stop"}),
        )
    }
    pub fn dictation_toggle(&self) -> Result<()> {
        self.change(
            Feature::DictationToggle,
            json!({"command":"dictation","action":"toggle"}),
        )
    }
    pub fn paste_last(&self) -> Result<()> {
        self.change(Feature::PasteLast, json!({"command":"paste-last"}))
    }
    pub fn settings_list(&self) -> Result<Vec<Setting>> {
        let v = self.call(
            Feature::SettingsList,
            json!({"command":"settings-get"}),
            "settings",
        )?;
        decode(v, "settings")
    }
    pub fn settings_get(&self, key: &str) -> Result<Vec<Setting>> {
        let v = self.call(
            Feature::SettingsGet,
            json!({"command":"settings-get","key":key}),
            "settings",
        )?;
        decode(v, "settings")
    }
    pub fn settings_set(&self, key: &str, value: &str) -> Result<()> {
        self.change(
            Feature::SettingsSet,
            json!({"command":"settings-set","key":key,"value":value}),
        )
    }
    pub fn vocabulary_list(&self) -> Result<Vec<VocabularyEntry>> {
        let v = self.call(
            Feature::VocabularyList,
            json!({"command":"vocabulary-list"}),
            "vocabulary",
        )?;
        decode(v, "entries")
    }
    pub fn vocabulary_add(&self, term: &str, sounds_like: &[&str]) -> Result<()> {
        self.change(
            Feature::VocabularyAdd,
            json!({"command":"vocabulary-add","term":term,"sounds_like":sounds_like}),
        )
    }
    pub fn vocabulary_remove(&self, term: &str) -> Result<()> {
        self.change(
            Feature::VocabularyRemove,
            json!({"command":"vocabulary-remove","term":term}),
        )
    }
    pub fn models_list(&self) -> Result<Vec<ModelInfo>> {
        let v = self.call(
            Feature::ModelsList,
            json!({"command":"model-list"}),
            "models",
        )?;
        decode(v, "models")
    }
    pub fn model_use(&self, model: &str) -> Result<()> {
        self.change(
            Feature::ModelUse,
            json!({"command":"model-use","model":model}),
        )
    }
    pub fn model_install(&self, model: &str) -> Result<()> {
        self.change(
            Feature::ModelInstall,
            json!({"command":"model-install","model":model}),
        )
    }
    pub fn open_models(&self) -> Result<()> {
        self.change(
            Feature::WindowShow,
            json!({"command":"window-show","pane":"models"}),
        )
    }
    pub fn watch(&self) -> Result<StatusWatcher> {
        StatusWatcher::start(self.clone())
    }
}
fn decode<T: serde::de::DeserializeOwned>(v: Value, field: &str) -> Result<T> {
    serde_json::from_value(v[field].clone())
        .map_err(|_| failed("invalid control data; values suppressed"))
}
fn lab_settings(key: &str) -> Result<Vec<Setting>> {
    if let Ok(v) = control(json!({"command":"settings-get", "key":key}), STATUS_TIMEOUT)
        && let Ok(settings) = decode(v, "settings")
    {
        return Ok(settings);
    }
    let text = hidden_help(
        &newest_cli()?,
        &["settings", "get", key, "--json"],
        Instant::now() + CALL_TIMEOUT,
    )?;
    let value = serde_json::from_str(&text).map_err(|_| failed("invalid Labs settings JSON"))?;
    decode(value, "settings")
}
#[cfg(windows)]
fn newest_cli() -> Result<PathBuf> {
    fs::read_dir(hex_dir()?.join("versions"))
        .map_err(io_error)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path().join("hex.com");
            if !path.is_file() {
                return None;
            }
            Some((entry.metadata().ok()?.modified().ok()?, path))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
        .ok_or(HexError::HexNotRunning)
}
fn hidden_help(path: &PathBuf, args: &[&str], deadline: Instant) -> Result<String> {
    crate::platform::require_process_support().map_err(|_| failed(crate::platform::UNAVAILABLE))?;
    remaining(deadline)?;
    if hex_dir()?.join("gpu-draw.pending").exists() {
        return Err(failed("Hex maintenance pending; CLI not launched"));
    }
    let mut child = crate::platform::worker_command(path)
        .map_err(io_error)?
        .args(args)
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(io_error)?;
    let job = match crate::platform::ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failed("cannot contain hidden help process"));
        }
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| failed("missing help output"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| failed("missing help errors"))?;
    let out = thread::spawn(move || {
        let mut b = Vec::new();
        let r = stdout.take((LIMIT + 1) as u64).read_to_end(&mut b);
        (r, b)
    });
    let err = thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stderr.take((LIMIT + 1) as u64).read_to_end(&mut b);
    });
    let result = loop {
        match child.try_wait() {
            Ok(Some(s)) if s.success() => break Ok(()),
            Ok(Some(_)) => break Err(failed("CLI help failed; stderr suppressed")),
            Err(_) => break Err(failed("CLI help wait failed")),
            Ok(None) => (),
        }
        if remaining(deadline).is_err() {
            break Err(HexError::Timeout);
        }
        thread::sleep(Duration::from_millis(5));
    };
    drop(job);
    let _ = child.kill();
    let _ = child.wait();
    let _ = err.join();
    let (read, b) = out.join().map_err(|_| failed("help reader failed"))?;
    result?;
    read.map_err(io_error)?;
    if b.len() > LIMIT {
        return Err(failed("CLI help too large"));
    }
    String::from_utf8(b).map_err(|_| failed("invalid help encoding"))
}

#[cfg(windows)]
mod watcher {
    use super::*;
    use std::{sync::mpsc, thread::JoinHandle};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0},
        System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects},
    };
    struct Event(HANDLE);
    // Windows event handles support waiting/signaling from any thread; this object owns the handle.
    unsafe impl Send for Event {}
    unsafe impl Sync for Event {}
    impl Event {
        fn new(name: Option<&str>) -> Result<Self> {
            let name: Option<Vec<u16>> = name.map(|s| s.encode_utf16().chain(Some(0)).collect());
            let handle = unsafe {
                CreateEventW(
                    std::ptr::null(),
                    0,
                    0,
                    name.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                )
            };
            if handle.is_null() {
                Err(failed("event handle unavailable"))
            } else {
                Ok(Self(handle))
            }
        }
        fn raw(&self) -> HANDLE {
            self.0
        }
        fn signal(&self) {
            unsafe {
                SetEvent(self.0);
            }
        }
    }
    impl Drop for Event {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    const EVENT_NAME: &str = "Local\\HexMeetingsEngineChanged";
    /// Called by the existing SSE listener. No second /meetings/events subscription is opened.
    pub(crate) fn meeting_event() {
        if let Ok(event) = Event::new(Some(EVENT_NAME)) {
            event.signal();
        }
    }
    enum WatchCommand {
        Visible(bool),
        Stop,
    }
    pub struct StatusWatcher {
        pub updates: mpsc::Receiver<StatusSample>,
        commands: mpsc::Sender<WatchCommand>,
        wake: Arc<Event>,
        thread: Option<JoinHandle<()>>,
    }
    impl StatusWatcher {
        pub(super) fn start(engine: HexEngine) -> Result<Self> {
            let wake = Arc::new(Event::new(None)?);
            let shared = Event::new(Some(EVENT_NAME))?;
            let (commands, input) = mpsc::channel();
            let (output, updates) = mpsc::sync_channel(1);
            let worker_wake = wake.clone();
            let thread = thread::Builder::new()
                .name("hex-status".into())
                .spawn(move || {
                    let mut visible = false;
                    loop {
                        let handles = [worker_wake.raw(), shared.raw()];
                        let result = unsafe {
                            WaitForMultipleObjects(
                                if visible { 2 } else { 1 },
                                handles.as_ptr(),
                                0,
                                if visible {
                                    POLL.as_millis() as u32
                                } else {
                                    INFINITE
                                },
                            )
                        };
                        if result == WAIT_FAILED {
                            let _ = output.try_send(sample_status(|| {
                                Err(failed("status watcher wait failed"))
                            }));
                            return;
                        }
                        let mut refresh = result != WAIT_OBJECT_0;
                        for command in input.try_iter() {
                            match command {
                                WatchCommand::Stop => return,
                                WatchCommand::Visible(v) => {
                                    refresh |= v && !visible;
                                    visible = v;
                                }
                            }
                        }
                        if visible
                            && refresh
                            && let Err(mpsc::TrySendError::Disconnected(_)) =
                                output.try_send(sample_status(|| engine.status()))
                        {
                            return;
                        }
                    }
                })
                .map_err(|_| failed("cannot start status watcher"))?;
            Ok(Self {
                updates,
                commands,
                wake,
                thread: Some(thread),
            })
        }
        pub fn set_visible(&self, visible: bool) {
            let _ = self.commands.send(WatchCommand::Visible(visible));
            self.wake.signal();
        }
    }
    impl Drop for StatusWatcher {
        fn drop(&mut self) {
            let _ = self.commands.send(WatchCommand::Stop);
            self.wake.signal();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}
#[cfg(windows)]
pub use watcher::StatusWatcher;
#[cfg(windows)]
pub(crate) use watcher::meeting_event;
/// Read-only diagnostics. Counts and state only; never serializes a complete control response.
pub fn run_status() -> i32 {
    let engine = HexEngine::new();
    let result = (|| -> Result<()> {
        let mut timings = Vec::new();
        let mut state = None;
        for _ in 0..10 {
            let start = Instant::now();
            state = Some(engine.status()?);
            timings.push(start.elapsed().as_micros());
        }
        let s = state.ok_or_else(|| failed("no status"))?;
        println!(
            "Hex running={} version={} protocol={} meeting={} dictating={:?} model={} recording_details=redacted",
            s.running,
            s.version,
            s.protocol,
            s.meeting_state,
            s.dictating,
            s.model.id.as_deref().unwrap_or("unknown")
        );
        println!(
            "status_us min={} max={} mean={}",
            timings.iter().min().unwrap_or(&0),
            timings.iter().max().unwrap_or(&0),
            timings.iter().sum::<u128>() / timings.len() as u128
        );
        let caps = engine.capabilities()?;
        for f in FEATURES {
            println!("{f:?}: {:?}", caps.get(&f).unwrap_or(&Support::Unknown));
        }
        println!(
            "settings_count={} settings_meetings_count={} vocabulary_count={} models_count={} model_status_installed={:?}; values suppressed",
            engine.settings_list()?.len(),
            engine.settings_get("meetings")?.len(),
            engine.vocabulary_list()?.len(),
            engine.models_list()?.len(),
            engine.model_status()?.installed
        );
        if std::env::args().any(|a| a == "--watch-visible" || a == "--watch-hidden") {
            let watcher = engine.watch()?;
            let visible = std::env::args().any(|a| a == "--watch-visible");
            watcher.set_visible(visible);
            println!("watcher visible={visible}; sampling 20 seconds");
            let start = Instant::now();
            let mut updates = 0;
            while start.elapsed() < Duration::from_secs(20) {
                if let Ok(update) = watcher.updates.recv_timeout(Duration::from_millis(500)) {
                    update.1?;
                    updates += 1;
                }
            }
            println!("watcher updates={updates}");
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn meeting_event() {}
#[cfg(not(windows))]
pub struct StatusWatcher {
    pub updates: std::sync::mpsc::Receiver<StatusSample>,
}
#[cfg(not(windows))]
impl StatusWatcher {
    pub fn set_visible(&self, _visible: bool) {}
    fn start(_engine: HexEngine) -> Result<Self> {
        Err(failed(crate::platform::UNAVAILABLE))
    }
}

#[cfg(not(windows))]
fn newest_cli() -> Result<PathBuf> {
    Err(failed(crate::platform::UNAVAILABLE))
}
