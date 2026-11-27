//! Public and local Velopack feeds. Checks and downloads never apply an update.
use crate::core::Channel;
pub use crate::update_state::{CheckRequest, ListenerStatus, Phase, Status, WindowStart};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};
use velopack::{
    UpdateCheck, UpdateManager, UpdateOptions, VelopackAsset,
    locator::VelopackLocatorConfig,
    sources::{FileSource, UpdateSource},
};

// Velopack's HttpSource constructs an unbounded default agent internally.
#[derive(Clone)]
struct HttpSource {
    base: String,
    agent: ureq::Agent,
    feed_deadline: Duration,
    download_deadline: Duration,
}
impl HttpSource {
    fn new(base: &str) -> Self {
        Self {
            base: format!("{}/", base.trim_end_matches('/')),
            agent: ureq::Agent::config_builder()
                .timeout_connect(Some(Duration::from_secs(10)))
                .timeout_recv_response(Some(Duration::from_secs(20)))
                .timeout_recv_body(Some(Duration::from_secs(300)))
                .build()
                .into(),
            feed_deadline: Duration::from_secs(30),
            download_deadline: Duration::from_secs(300),
        }
    }
    fn url(&self, name: &str) -> Result<url::Url, velopack::Error> {
        Ok(url::Url::parse(&self.base)?.join(name)?)
    }
    fn feed_url(&self, channel: &str) -> Result<url::Url, velopack::Error> {
        self.url(&format!("releases.{channel}.json"))
    }
    fn get(
        &self,
        url: &url::Url,
        deadline: Duration,
    ) -> Result<ureq::http::Response<ureq::Body>, velopack::Error> {
        self.agent
            .get(url.as_str())
            .config()
            .timeout_global(Some(deadline))
            .build()
            .call()
            .map_err(|error| {
                velopack::Error::Generic(format!("Update request failed: {error}. Try again."))
            })
    }
}
impl UpdateSource for HttpSource {
    fn get_release_feed(
        &self,
        channel: &str,
        app: &velopack::bundle::Manifest,
        id: &str,
    ) -> Result<velopack::VelopackAssetFeed, velopack::Error> {
        let mut url = self.feed_url(channel)?;
        url.query_pairs_mut()
            .append_pair("localVersion", &app.version.to_string())
            .append_pair("id", &app.id)
            .append_pair("stagingId", id);
        let json = self
            .get(&url, self.feed_deadline)?
            .body_mut()
            .read_to_string()
            .map_err(|error| {
                velopack::Error::Generic(format!("Update feed failed: {error}. Try again."))
            })?;
        let feed: velopack::VelopackAssetFeed = serde_json::from_str(&json)?;
        if !feed.Assets.iter().all(valid_asset) {
            return Err(velopack::Error::Generic("Invalid public feed asset".into()));
        }
        Ok(feed)
    }
    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        destination: &str,
        progress: Option<mpsc::Sender<i16>>,
    ) -> Result<(), velopack::Error> {
        if !valid_asset(asset) {
            return Err(velopack::Error::Generic("Invalid public feed asset".into()));
        }
        let mut response = self.get(&self.url(&asset.FileName)?, self.download_deadline)?;
        let mut reader = response.body_mut().as_reader();
        let mut output = File::create(destination)?;
        let mut bytes = [0u8; 128 * 1024];
        let mut copied = 0u64;
        loop {
            let count = reader.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            output.write_all(&bytes[..count])?;
            copied += count as u64;
            if let Some(tx) = &progress {
                let _ = tx.send((copied.saturating_mul(100) / asset.Size.max(1)).min(100) as i16);
            }
        }
        output.sync_all()?;
        Ok(())
    }
    fn clone_boxed(&self) -> Box<dyn UpdateSource> {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
struct LocalSource(PathBuf);
fn valid_asset(asset: &VelopackAsset) -> bool {
    asset.PackageId == "sidenote"
        && !asset.FileName.is_empty()
        && asset
            .FileName
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        && asset.FileName.ends_with(".nupkg")
        && asset.SHA256.len() == 64
        && asset.SHA256.bytes().all(|c| c.is_ascii_hexdigit())
}
impl UpdateSource for LocalSource {
    fn get_release_feed(
        &self,
        channel: &str,
        app: &velopack::bundle::Manifest,
        id: &str,
    ) -> Result<velopack::VelopackAssetFeed, velopack::Error> {
        let feed = FileSource::new(&self.0).get_release_feed(channel, app, id)?;
        if !feed.Assets.iter().all(valid_asset) {
            return Err(velopack::Error::Generic("Invalid local feed asset".into()));
        }
        Ok(feed)
    }
    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        destination: &str,
        progress: Option<mpsc::Sender<i16>>,
    ) -> Result<(), velopack::Error> {
        if !valid_asset(asset) {
            return Err(velopack::Error::Generic("Invalid local feed asset".into()));
        }
        let mut source = File::open(self.0.join(&asset.FileName))?;
        let mut output = File::create(destination)?;
        let mut bytes = [0u8; 128 * 1024];
        let mut copied = 0u64;
        loop {
            let count = source.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            output.write_all(&bytes[..count])?;
            copied += count as u64;
            if let Some(tx) = &progress {
                let _ = tx.send((copied.saturating_mul(100) / asset.Size.max(1)).min(100) as i16);
            }
        }
        output.sync_all()?;
        Ok(())
    }
    fn clone_boxed(&self) -> Box<dyn UpdateSource> {
        Box::new(self.clone())
    }
}
fn cache_path(asset: &VelopackAsset) -> Result<PathBuf, String> {
    if !valid_asset(asset) {
        return Err("The update package is invalid.".into());
    }
    let locator = velopack::locator::auto_locate_app_manifest(
        velopack::locator::LocationContext::FromCurrentExe,
    )
    .map_err(|_| "Could not locate installed update packages.")?;
    Ok(locator.get_packages_dir().join(&asset.FileName))
}
fn verify(asset: &VelopackAsset) -> Result<(), String> {
    if !valid_asset(asset) {
        return Err("The update package is invalid.".into());
    }
    let mut file = File::open(cache_path(asset)?).map_err(|_| "Download the update again.")?;
    if file
        .metadata()
        .map_err(|_| "Could not read the update.")?
        .len()
        != asset.Size
    {
        return Err("The update size does not match. Try again.".into());
    }
    if !verify_sha256(&mut file, &asset.SHA256).map_err(|_| "Could not read the update.")? {
        return Err("The update checksum does not match. Try again.".into());
    }
    Ok(())
}

pub(crate) fn verify_sha256(reader: &mut impl Read, expected: &str) -> io::Result<bool> {
    if expected.len() != 64 || !expected.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Ok(false);
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .eq_ignore_ascii_case(expected))
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
pub const PUBLIC_FEED: &str = "https://github.com/harshanandak/sidenote/releases/latest/download/";

fn public_feed(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => PUBLIC_FEED,
        Channel::Dev => "https://github.com/harshanandak/sidenote/releases/download/dev/",
        Channel::Latest => {
            "https://github.com/harshanandak/sidenote/releases/download/latest-main/"
        }
    }
}

fn channel_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "stable",
        Channel::Dev => "dev",
        Channel::Latest => "latest",
    }
}

fn selected_channel(root: &Path, fallback: Channel) -> Channel {
    match fs::read(root.join("channel")).as_deref() {
        Ok(b"stable") => Channel::Stable,
        Ok(b"dev") => Channel::Dev,
        Ok(b"latest") => Channel::Latest,
        _ => fallback,
    }
}

pub fn channel() -> Channel {
    selected_channel(&directory(), Channel::DEFAULT_UPDATE)
}

fn save_channel(root: &Path, choice: Channel) -> io::Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(root)?;
    temp.write_all(channel_name(choice).as_bytes())?;
    match fs::remove_file(root.join("update-ready.json")) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    temp.persist(root.join("channel"))?;
    Ok(())
}

pub fn set_channel(choice: Channel) -> Result<(), String> {
    let _check =
        exclusive("update-check.lock").map_err(|_| "Wait for the update check to finish.")?;
    save_channel(&directory(), choice).map_err(|_| "Could not save the update channel.".into())
}

pub const FOCUS_CHECK_AFTER: Duration = Duration::from_secs(30 * 60);

fn focus_check_due(last_attempt: Instant, now: Instant) -> bool {
    now.duration_since(last_attempt) > FOCUS_CHECK_AFTER
}
/// Dropping the sender stops the six-hour worker after its current check/download.
pub fn start(publish: impl Fn(Status) + Send + 'static) -> Option<mpsc::Sender<CheckRequest>> {
    if !installed() {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = write_report(&load());
        let mut last_attempt = Instant::now();
        let run = || publish_check_failure(check(&publish), &publish);
        run();
        loop {
            match rx.recv_timeout(CHECK_EVERY) {
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(CheckRequest::Focus) if !focus_check_due(last_attempt, Instant::now()) => {
                    continue;
                }
                _ => {}
            }
            last_attempt = Instant::now();
            run();
        }
    });
    Some(tx)
}
fn publish_check_failure(result: Result<Status, String>, publish: impl FnOnce(Status)) {
    let Err(error) = result else { return };
    publish(Status {
        phase: Phase::Error,
        error: Some(error),
        ..Status::default()
    });
}

pub fn installed() -> bool {
    static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *INSTALLED.get_or_init(|| {
        std::env::var_os("LOCALAPPDATA").is_some()
            && velopack::locator::auto_locate_app_manifest(
                velopack::locator::LocationContext::FromCurrentExe,
            )
            .is_ok()
    })
}
pub fn directory() -> PathBuf {
    #[cfg(feature = "recorder-ready-proof")]
    if let Some(root) = recorder_ready_root() {
        return root.join("updates");
    }
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Sidenote/updates")
}
#[cfg(feature = "recorder-ready-proof")]
fn recorder_ready_root() -> Option<PathBuf> {
    let root = PathBuf::from(crate::env_var("SIDENOTE_RECORDER_READY_TEST")?);
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts")
        .canonicalize()
        .unwrap();
    let root = root.canonicalize().unwrap();
    assert!(root.starts_with(fixture));
    assert!(
        std::env::current_exe()
            .unwrap()
            .canonicalize()
            .unwrap()
            .starts_with(&root)
    );
    assert!(
        crate::core::state_dir()
            .canonicalize()
            .unwrap()
            .starts_with(&root)
    );
    Some(root)
}
fn local(name: &str) -> PathBuf {
    directory().join(name)
}
pub fn load() -> Status {
    if !installed() {
        return Status {
            phase: Phase::Manual,
            ..Default::default()
        };
    }
    read_status(&directory())
}
fn read_status(directory: &Path) -> Status {
    let mut status = pipeline_status(
        read_saved_status(directory),
        &directory.join("pipeline-install.pending"),
    );
    if status.error.is_none()
        && directory.join("update-listener.resume").exists()
        && !directory.join("pipeline-install.pending").exists()
    {
        status.phase = Phase::Error;
        status.error = Some(INSTALL_LISTENER_ERROR.into());
    }
    status
}
fn read_saved_status(directory: &Path) -> Status {
    fs::read(directory.join("updates.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Status>(&b).ok())
        .filter(|status| match status.channel.as_deref() {
            Some(name) => {
                name == channel_name(selected_channel(directory, Channel::DEFAULT_UPDATE))
            }
            None => status.phase != Phase::Ready,
        })
        .unwrap_or_default()
}
fn pipeline_status(mut status: Status, marker: &Path) -> Status {
    if marker.exists() && (!status.reconcile_error || status.error.is_none()) {
        if matches!(status.phase, Phase::Idle | Phase::Manual) {
            status.phase = Phase::Error;
        }
        status.error = Some(PIPELINE_INSTALL_ERROR.into());
    }
    status
}
fn save(status: &Status) -> io::Result<()> {
    if !installed() {
        return Ok(());
    }
    let status = pipeline_status(status.clone(), &local("pipeline-install.pending"));
    save_status(&status)
}
fn save_status(status: &Status) -> io::Result<()> {
    if !installed() || status.error.as_deref() == Some(LEGACY_HOOK_CONTENTION) {
        return Ok(());
    }
    let path = local("updates.json");
    fs::create_dir_all(path.parent().unwrap())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(status)?)?;
    crate::platform::replace_file(&temp, &path)?;
    write_report(status).map(|_| ())
}
pub fn feed() -> Option<PathBuf> {
    crate::env_var("SIDENOTE_UPDATE_FEED").map(PathBuf::from)
}
pub fn source_label() -> String {
    feed().map_or_else(
        || public_feed(channel()).into(),
        |p| p.display().to_string(),
    )
}
// Messages are saved to updates.json and the report, so they never copy feed content.
fn feed_error(folder: Option<&Path>, channel: Channel, error: &velopack::Error) -> String {
    let Some(folder) = folder else {
        return "Could not read the update feed. Check your connection and try again.".into();
    };
    let index = format!("releases.{}.json", channel_name(channel));
    let folder = folder.display();
    match error {
        velopack::Error::Io(e) if e.kind() == io::ErrorKind::NotFound => {
            format!("Could not read {index} in {folder}. The file is missing.")
        }
        velopack::Error::Io(e) => format!("Could not read {index} in {folder}: {}.", e.kind()),
        velopack::Error::Json(e) => format!(
            "{index} in {folder} is not a valid update feed (line {}, column {}).",
            e.line(),
            e.column()
        ),
        velopack::Error::Generic(m) if m == "Invalid local feed asset" => {
            format!("{index} in {folder} lists a package that is not allowed.")
        }
        _ => format!("Could not read the update feed in {folder}."),
    }
}
fn manager(channel: Channel) -> Result<UpdateManager, String> {
    let source: Box<dyn UpdateSource> = if let Some(feed) = feed() {
        if !feed.is_absolute() || feed.to_string_lossy().contains("://") {
            return Err("The update feed must be an absolute folder or file-share path.".into());
        }
        Box::new(LocalSource(feed))
    } else {
        Box::new(HttpSource::new(public_feed(channel)))
    };
    channel_manager(source, channel, None)
        .map_err(|_| "Updates need a Sidenote Setup install. Manual builds cannot update.".into())
}

fn channel_manager(
    source: Box<dyn UpdateSource>,
    channel: Channel,
    locator: Option<VelopackLocatorConfig>,
) -> Result<UpdateManager, velopack::Error> {
    let options = UpdateOptions {
        ExplicitChannel: Some(channel_name(channel).into()),
        AllowVersionDowngrade: false,
        ..Default::default()
    };
    UpdateManager::new_boxed(source, Some(options), locator)
}

fn pending_asset(mgr: &UpdateManager, channel: Channel) -> Option<VelopackAsset> {
    let (name, asset): (String, VelopackAsset) =
        serde_json::from_slice(&fs::read(local("update-ready.json")).ok()?).ok()?;
    if name != channel_name(channel) {
        return None;
    }
    let path = cache_path(&asset).ok()?;
    let manifest = velopack::bundle::load_bundle_from_file(path)
        .ok()?
        .read_manifest()
        .ok()?;
    (manifest.version > mgr.get_current_version()
        && manifest.version.to_string() == asset.Version
        && manifest.id == asset.PackageId
        && manifest.channel == name)
        .then_some(asset)
}
pub fn check(mut publish: impl FnMut(Status)) -> Result<Status, String> {
    if !installed() {
        return Ok(load());
    }
    let mut publish = |status| publish(pipeline_status(status, &local("pipeline-install.pending")));
    drop(update_operation_lock(&directory(), true)?);
    let _check = exclusive("update-check.lock").map_err(|_| "Another update check is running.")?;
    let channel = channel();
    let mut status = Status {
        channel: Some(channel_name(channel).into()),
        phase: Phase::Checking,
        checked: Some(chrono::Local::now().to_rfc3339()),
        ..Default::default()
    };
    publish(status.clone());
    let mut commit = None;
    let result = (|| {
        let mgr = manager(channel)?;
        let update = mgr
            .check_for_updates()
            .map_err(|error| feed_error(feed().as_deref(), channel, &error))?;
        match update {
            UpdateCheck::UpdateAvailable(mut update) => {
                // Reconstructed delta ZIPs are not byte-identical to the feed's full SHA256.
                update.BaseRelease = None;
                update.DeltasToTarget.clear();
                let cached = cache_path(&update.TargetFullRelease)?;
                if cached.exists() && verify(&update.TargetFullRelease).is_err() {
                    fs::rename(&cached, cached.with_extension("rejected"))
                        .map_err(|_| "Could not set aside an invalid cached update.")?;
                }
                status.version = update.TargetFullRelease.Version.clone();
                status.notes = update
                    .TargetFullRelease
                    .NotesMarkdown
                    .chars()
                    .take(16000)
                    .collect();
                status.phase = Phase::Available;
                publish(status.clone());
                status.phase = Phase::Downloading;
                publish(status.clone());
                let (tx, rx) = mpsc::channel();
                std::thread::scope(|scope| {
                    let worker = scope.spawn(|| mgr.download_updates(&update, Some(tx)));
                    for progress in rx {
                        status.progress = progress.clamp(0, 100) as u8;
                        publish(status.clone());
                    }
                    worker
                        .join()
                        .map_err(|_| "The update download stopped.")?
                        .map_err(|_| "Could not download or verify the update. Try again.")
                })?;
                verify(&update.TargetFullRelease)?;
                if commit
                    .insert(update_operation_lock(&directory(), true))
                    .is_err()
                {
                    return Ok(());
                }
                fs::write(
                    local("update-ready.json"),
                    serde_json::to_vec(&(channel_name(channel), &update.TargetFullRelease))
                        .map_err(|_| "Could not save the update.")?,
                )
                .map_err(|_| "Could not save the update.")?;
                status.phase = Phase::Ready;
                status.progress = 100;
            }
            UpdateCheck::NoUpdateAvailable => status.phase = Phase::Idle,
            _ => return Err("This update feed does not match the installed app.".into()),
        }
        Ok(())
    })();
    let _hook = commit.unwrap_or_else(|| update_operation_lock(&directory(), true))?;
    if let Err(error) = result {
        status.phase = Phase::Error;
        status.error = Some(error);
    }
    status = pipeline_status(status, &local("pipeline-install.pending"));
    publish(status.clone());
    save(&status).map_err(|_| "Could not save update status.")?;
    Ok(status)
}
pub fn exclusive(name: &str) -> io::Result<File> {
    fs::create_dir_all(local(""))?;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .share_mode(0)
        .open(local(name))
}
/// Shared readers represent work. Apply needs an exclusive writer of the same file.
pub fn activity() -> io::Result<Option<File>> {
    if !installed() {
        return Ok(None);
    }
    if listener_stopping() {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "An update is being installed",
        ));
    }
    fs::create_dir_all(local(""))?;
    let path = local("update-activity.lock");
    if !path.exists() {
        let _ = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path);
    }
    OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map(Some)
}
fn recording_idle() -> Result<(), String> {
    match crate::hex_engine::HexEngine::new().status() {
        Ok(s) if s.recording.is_none() && s.dictating == Some(false) && s.meeting_state == "idle" => Ok(()),
        Ok(_) => Err("Finish recording before installing the update.".into()),
        Err(_) => match crate::platform::hex_running() {
            Ok(false) => Ok(()),
            _ => Err("Hex is running but its recording status cannot be read. Treat it as recording and try again later.".into()),
        },
    }
}
fn task() -> Result<String, String> {
    let name = crate::env_var("SIDENOTE_LISTENER_TASK")
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Sidenote meeting listener".into());
    if !name.starts_with("Sidenote ") || name.contains(['\'', '"', '\r', '\n', '\\', '/']) {
        return Err("The listener task must use a Sidenote task name.".into());
    }
    Ok(name)
}
fn task_command(run: bool) -> Result<(), String> {
    match checked_listener_task(run, false) {
        Ok(()) => Ok(()),
        Err(ListenerTaskError::TimedOut) if run => {
            task_validation_result(listener_probe(listener_lock))
        }
        Err(error) => Err(error.message()),
    }
}
#[derive(Debug)]
enum ListenerTaskError {
    TimedOut,
    Failed(String),
    OtherUser,
    Elevated,
    Disabled,
    UnmanagedAction,
    PermissionDenied,
}
impl ListenerTaskError {
    fn message(self) -> String {
        self.description().into()
    }

    fn description(&self) -> &str {
        match self {
            Self::TimedOut => "The meeting listener setup did not finish. Try again.",
            Self::Failed(message) => message,
            Self::OtherUser => "The meeting listener task belongs to another Windows account.",
            Self::Elevated => {
                "The meeting listener task is configured to run with elevated privileges."
            }
            Self::Disabled => "The meeting listener task is disabled.",
            Self::UnmanagedAction => {
                "The meeting listener task does not target this Sidenote installation with the expected arguments."
            }
            Self::PermissionDenied => {
                "Sidenote could not verify the meeting listener task's Windows identity."
            }
        }
    }
}
#[derive(Default)]
struct TaskValidation {
    retry_after: Option<Instant>,
    failure: Option<String>,
}
impl TaskValidation {
    fn complete(&mut self, result: &Result<(), ListenerTaskError>, now: Instant) {
        match result {
            Ok(()) => {
                self.retry_after = None;
                self.failure = None;
            }
            Err(error) => {
                self.retry_after = Some(now + Duration::from_secs(30));
                // A timeout cannot disprove a definite failure from an earlier check.
                if !matches!(error, ListenerTaskError::TimedOut) {
                    self.failure = Some(error.description().into());
                }
            }
        }
    }
    fn resolve(&self, running: Result<(), String>) -> Result<(), String> {
        running?;
        self.failure.clone().map_or(Ok(()), Err)
    }
}
static TASK_VALIDATION: std::sync::Mutex<TaskValidation> = std::sync::Mutex::new(TaskValidation {
    retry_after: None,
    failure: None,
});
fn checked_listener_task(run: bool, register: bool) -> Result<(), ListenerTaskError> {
    checked_listener_task_until(run, register, Instant::now() + TASK_COMMAND_TIMEOUT)
}
fn checked_listener_task_until(
    run: bool,
    register: bool,
    deadline: Instant,
) -> Result<(), ListenerTaskError> {
    let result = listener_task(ListenerTaskMode::Recover { run, register }, deadline);
    TASK_VALIDATION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .complete(&result, Instant::now());
    result
}
fn task_validation_result(running: Result<(), String>) -> Result<(), String> {
    TASK_VALIDATION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .resolve(running)
}
const LISTENER_TASK_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$sid = $identity.User.Value
$adminsSid = 'S-1-5-32-544'
function Open-ListenerTask([switch]$AllowMissing) {
    $service = New-Object -ComObject 'Schedule.Service'
    $service.Connect()
    $folder = $service.GetFolder('\')
    try { $folder.GetTask($env:SIDENOTE_TASK_NAME) }
    catch {
        # An exact lookup distinguishes a missing task from an inaccessible task.
        if ($AllowMissing -and $_.Exception.GetBaseException().HResult -eq -2147024894) { return }
        throw
    }
}
function Read-ListenerTask([switch]$AllowMissing) {
    $registered = Open-ListenerTask -AllowMissing:$AllowMissing
    if ($null -eq $registered) { return }
    $t = Get-ScheduledTask -TaskPath '\' -TaskName $env:SIDENOTE_TASK_NAME
    $a = @($t.Actions)
    $principal = $t.Principal.UserId
    if ($principal -match '^S-\d-') { $principal = ([Security.Principal.SecurityIdentifier]$principal).Value }
    else { $principal = ([Security.Principal.NTAccount]$principal).Translate([Security.Principal.SecurityIdentifier]).Value }
    if ($principal -ne $sid -or $t.Principal.LogonType -ne 'Interactive') { throw 'other_user' }
    if ($t.Principal.RunLevel -ne 'Limited') { throw 'elevated' }
    if (!$t.Settings.Enabled) { throw 'disabled' }
    if ($a.Count -ne 1 -or $a[0].Execute -ne $env:SIDENOTE_TASK_EXE -or $a[0].Arguments -cne $env:SIDENOTE_TASK_ARGS) { throw 'unmanaged_action' }
    $owner = [Security.AccessControl.RawSecurityDescriptor]::new($registered.GetSecurityDescriptor(1)).Owner
    if (!$owner) { throw 'permission_denied' }
    # UAC can mark group membership deny-only for a filtered administrator.
    $legacyOwner = $owner.Value -eq $adminsSid -and ($identity.UserClaims | Where-Object {
        $_.Value -eq $adminsSid -and $_.Type -in @([Security.Claims.ClaimTypes]::GroupSid, [Security.Claims.ClaimTypes]::DenyOnlySid) -and
            $_.Properties.ContainsKey([Security.Claims.ClaimTypes]::WindowsSubAuthority)
    })
    if ($owner.Value -ne $sid -and !$legacyOwner) { throw 'other_user' }
    return $t
}
if ($env:SIDENOTE_TASK_REGISTER -eq 'inspect') {
    try {
        $null = Read-ListenerTask -AllowMissing
        exit 0
    } catch {
        switch ($_.Exception.Message) {
            'other_user' { exit 31 }
            'elevated' { exit 32 }
            'disabled' { exit 33 }
            'unmanaged_action' { exit 34 }
            default { exit 35 }
        }
    }
}
function Test-RestartSchedule($t) {
    # Windows saves logon users as account names and indefinite repetition with StopAtDurationEnd true.
    $logon = @($t.Triggers | Where-Object {
        if ($_.CimClass.CimClassName -ne 'MSFT_TaskLogonTrigger' -or !$_.Enabled) { return $false }
        $user = $_.UserId
        if ($user -ne $sid) {
            try { $user = ([Security.Principal.NTAccount]$user).Translate([Security.Principal.SecurityIdentifier]).Value }
            catch { return $false }
        }
        return $user -eq $sid
    })
    $retry = @($t.Triggers | Where-Object { $_.CimClass.CimClassName -eq 'MSFT_TaskTimeTrigger' -and $_.Enabled -and $_.Repetition.Interval -eq 'PT5M' -and !$_.Repetition.Duration })
    return ($logon.Count -eq 1 -and $retry.Count -eq 1 -and @($t.Triggers).Count -eq 2 -and [string]$t.Settings.MultipleInstances -in @('IgnoreNew', '2') -and $t.Settings.ExecutionTimeLimit -eq 'PT0S')
}
$register = $env:SIDENOTE_TASK_REGISTER -eq 'yes'
$t = Read-ListenerTask -AllowMissing:$register
if (!$t -and $register) {
    $action = New-ScheduledTaskAction -Execute $env:SIDENOTE_TASK_EXE -Argument $env:SIDENOTE_TASK_ARGS
    $logon = New-ScheduledTaskTrigger -AtLogOn -User $sid
    $retry = New-ScheduledTaskTrigger -Once -At (Get-Date) -RepetitionInterval (New-TimeSpan -Minutes 5)
    $principal = New-ScheduledTaskPrincipal -UserId $sid -LogonType Interactive -RunLevel Limited
    $settings = New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskPath '\' -TaskName $env:SIDENOTE_TASK_NAME -Action $action -Trigger @($logon, $retry) -Principal $principal -Settings $settings | Out-Null
    $registered = Open-ListenerTask
    $owner = [Security.AccessControl.RawSecurityDescriptor]::new($registered.GetSecurityDescriptor(1)).Owner
    # Elevated registration can assign the caller's default group owner.
    if ($owner -and $owner.Value -eq $adminsSid) {
        $null = Read-ListenerTask
        $descriptor = [Security.AccessControl.RawSecurityDescriptor]::new($registered.GetSecurityDescriptor(7))
        if ($descriptor.Owner.Value -ne $owner.Value) { throw 'other_user' }
        $descriptor.Owner = $identity.User
        $sddl = $descriptor.GetSddlForm([Security.AccessControl.AccessControlSections]'Owner,Group,Access')
        $registered.SetSecurityDescriptor($sddl, 16) # TASK_DONT_ADD_PRINCIPAL_ACE
    }
    $t = Read-ListenerTask
}
if (!(Test-RestartSchedule $t)) {
    if ($env:SIDENOTE_TASK_REGISTER -ne 'yes') { throw 'Listener task needs its restart schedule repaired' }
    # Identity and action were checked above; never replace an unrelated task or stop a running one.
    $logon = New-ScheduledTaskTrigger -AtLogOn -User $sid
    $retry = New-ScheduledTaskTrigger -Once -At (Get-Date) -RepetitionInterval (New-TimeSpan -Minutes 5)
    $settings = $t.Settings
    $settings.MultipleInstances = 2 # TASK_INSTANCES_IGNORE_NEW; CIM may expose the raw UInt32.
    $settings.ExecutionTimeLimit = 'PT0S'
    Set-ScheduledTask -TaskPath '\' -TaskName $env:SIDENOTE_TASK_NAME -Trigger @($logon, $retry) -Settings $settings | Out-Null
    $t = Read-ListenerTask
    if (!(Test-RestartSchedule $t)) { throw 'Listener task restart schedule was not saved' }
}
if ($env:SIDENOTE_TASK_RUN -eq 'yes') {
    Start-ScheduledTask -InputObject $t
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $previous = $null
    while ($true) {
        $running = @(Get-CimInstance Win32_Process -Filter "Name = 'sidenote.exe'" | Where-Object {
            $_.ExecutablePath -ieq $env:SIDENOTE_TASK_EXE -and $_.CreationDate -and
            $_.CommandLine -match '(?i)(?:^|\s)"?--listen"?(?=\s|$)'
        })
        $current = if ($running.Count -eq 1) { "$($running[0].ProcessId):$($running[0].CreationDate.Ticks)" } else { $null }
        if ($current -and $current -eq $previous) { break }
        if ($watch.Elapsed.TotalSeconds -ge 7) { throw 'Listener process did not become ready' }
        $previous = $current
        # IgnoreNew can have ignored the first start while the old process was exiting.
        if (-not $running.Count) { Start-ScheduledTask -InputObject $t }
        Start-Sleep -Milliseconds 250
    }
}
"#;
const TASK_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
#[derive(Clone, Copy)]
enum ListenerTaskMode {
    Preflight,
    Recover { run: bool, register: bool },
}

fn listener_task(mode: ListenerTaskMode, deadline: Instant) -> Result<(), ListenerTaskError> {
    let name = task().map_err(ListenerTaskError::Failed)?;
    let exe = std::env::current_exe()
        .map_err(|_| ListenerTaskError::Failed("Could not locate Sidenote.".into()))?;
    let expected_args = if name.starts_with("Sidenote updates proof ") {
        let data = crate::env_var("SIDENOTE_DATA_DIR").ok_or_else(|| {
            ListenerTaskError::Failed("Proof task needs an isolated data folder.".into())
        })?;
        format!(
            "--listen --listener-isolated --data-dir \"{}\" --listener-localappdata \"{}\"",
            data.to_string_lossy(),
            std::env::var_os("LOCALAPPDATA")
                .ok_or_else(|| ListenerTaskError::Failed(
                    "Proof task needs isolated LocalAppData.".into()
                ))?
                .to_string_lossy()
        )
    } else {
        "--listen".into()
    };

    #[cfg(feature = "recorder-ready-proof")]
    let script = recorder_ready_root().map_or_else(
        || LISTENER_TASK_SCRIPT.to_owned(),
        |root| {
            format!(
                "{}\n{LISTENER_TASK_SCRIPT}",
                fs::read_to_string(root.join("task-fixture.ps1")).unwrap()
            )
        },
    );
    #[cfg(not(feature = "recorder-ready-proof"))]
    let script = LISTENER_TASK_SCRIPT.to_owned();
    let deadline = match mode {
        ListenerTaskMode::Preflight => deadline,
        ListenerTaskMode::Recover { .. } => deadline.min(Instant::now() + TASK_COMMAND_TIMEOUT),
    };
    run_listener_task(
        &mut crate::platform::windows_powershell(),
        &script,
        mode,
        &name,
        &exe,
        &expected_args,
        deadline,
    )
}

fn run_listener_task(
    command: &mut Command,
    script: &str,
    mode: ListenerTaskMode,
    name: &str,
    exe: &Path,
    expected_args: &str,
    deadline: Instant,
) -> Result<(), ListenerTaskError> {
    if Instant::now() >= deadline {
        return Err(ListenerTaskError::TimedOut);
    }
    let (run, register) = match mode {
        ListenerTaskMode::Preflight => ("no", "inspect"),
        ListenerTaskMode::Recover { run, register } => (
            if run { "yes" } else { "no" },
            if register { "yes" } else { "no" },
        ),
    };
    let mut child = command
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("SIDENOTE_TASK_NAME", name)
        .env("SIDENOTE_TASK_EXE", exe)
        .env("SIDENOTE_TASK_ARGS", expected_args)
        .env("SIDENOTE_TASK_RUN", run)
        .env("SIDENOTE_TASK_REGISTER", register)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| ListenerTaskError::Failed("Could not check the listener task.".into()))?;
    let status = wait_for_child(&mut child, deadline).map_err(|error| {
        if error.kind() == io::ErrorKind::TimedOut {
            ListenerTaskError::TimedOut
        } else {
            ListenerTaskError::Failed("Could not check the listener task.".into())
        }
    })?;
    if status.success() {
        return Ok(());
    }
    Err(if matches!(mode, ListenerTaskMode::Preflight) {
        match status.code() {
            Some(31) => ListenerTaskError::OtherUser,
            Some(32) => ListenerTaskError::Elevated,
            Some(33) => ListenerTaskError::Disabled,
            Some(34) => ListenerTaskError::UnmanagedAction,
            _ => ListenerTaskError::PermissionDenied,
        }
    } else {
        ListenerTaskError::Failed("Sidenote could not prepare its meeting listener. Try again. If this continues, reinstall Sidenote for your Windows account.".into())
    })
}

fn wait_for_child(
    child: &mut std::process::Child,
    deadline: Instant,
) -> io::Result<std::process::ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(
                    Duration::from_millis(100)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(result
                    .err()
                    .unwrap_or_else(|| io::ErrorKind::TimedOut.into()));
            }
        }
    }
}

impl ListenerStatus {
    fn complete(&mut self, result: &Result<(), String>) {
        self.preparing = false;
        self.ready = result.is_ok();
        self.error = result.as_ref().err().cloned();
    }
}
static LISTENER: std::sync::Mutex<ListenerStatus> = std::sync::Mutex::new(ListenerStatus {
    ready: false,
    preparing: true,
    error: None,
});
fn finish_listener(result: &Result<(), String>) -> ListenerStatus {
    let (changed, status) = {
        let mut status = LISTENER.lock().unwrap_or_else(|e| e.into_inner());
        let changed = status.preparing
            || status.ready != result.is_ok()
            || status.error != result.as_ref().err().cloned();
        status.complete(result);
        (changed, status.clone())
    };
    if changed {
        let reason = match result.as_ref().map_err(String::as_str) {
            Ok(()) => "ready",
            Err(
                "The meeting listener is not running. Try again."
                | "The meeting listener did not start. Try again, or reinstall it if this continues.",
            ) => "lock_missing",
            Err("Could not check whether the listener is running.") => "lock_check_failed",
            Err(_) => "task_validation_failed",
        };
        crate::listener::log_process(&format!("listener readiness reason={reason}"));
    }
    status
}
// Preparation, recovery probes and apply must not compete for the listener lock.
static LISTENER_CHECK: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub fn listener_status() -> ListenerStatus {
    if !installed() {
        return ListenerStatus {
            ready: true,
            preparing: false,
            error: None,
        };
    }
    LISTENER.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
pub fn listener_ready() -> bool {
    listener_status().ready
}
#[derive(Serialize, Deserialize)]
struct StopMarker {
    pid: u32,
    created: i64,
}
fn write_stop(pid: u32) -> io::Result<()> {
    let temp = local("update-listener.stop.tmp");
    fs::write(
        &temp,
        serde_json::to_vec(&StopMarker {
            pid,
            created: chrono::Utc::now().timestamp(),
        })?,
    )?;
    crate::platform::replace_file(&temp, &local("update-listener.stop"))
}
pub fn listener_stopping() -> bool {
    listener_stop_source().is_some()
}
pub(crate) fn listener_stop_source() -> Option<&'static str> {
    if crate::service::stopping() {
        return Some("service-stop");
    }
    if !installed() {
        return None;
    }
    let path = local("update-listener.stop");
    let Ok(bytes) = fs::read(&path) else {
        return None;
    };
    let current = serde_json::from_slice::<StopMarker>(&bytes)
        .ok()
        .is_some_and(|marker| {
            let age = chrono::Utc::now().timestamp() - marker.created;
            age < 600 && crate::platform::process_running(marker.pid)
        });
    if !current {
        // Do not remove a fresh request that replaced the stale marker while it was read.
        if fs::read(&path).ok().as_deref() == Some(bytes.as_slice()) {
            let _ = fs::remove_file(path);
        }
    }
    current.then_some("update-stop-marker")
}
#[derive(Debug)]
pub enum WindowLockError {
    AlreadyOpen,
    Io(io::Error),
}
impl From<io::Error> for WindowLockError {
    fn from(error: io::Error) -> Self {
        if error.raw_os_error() == Some(32) {
            Self::AlreadyOpen
        } else {
            Self::Io(error)
        }
    }
}
pub fn window_lock() -> Result<Option<File>, WindowLockError> {
    if !installed() {
        return Ok(None);
    }
    exclusive("update-window.lock")
        .map(Some)
        .map_err(Into::into)
}

pub fn listener_lock() -> io::Result<Option<File>> {
    if !installed() {
        return Ok(None);
    }
    exclusive("update-listener.lock").map(Some)
}
pub fn ensure_listener() -> Result<(), String> {
    let _check = LISTENER_CHECK.lock().unwrap_or_else(|e| e.into_inner());
    let result = prepare_listener();
    finish_listener(&result);
    result
}
/// The window's worker probes the lock and retries incomplete task validation.
pub fn probe_listener() -> Option<ListenerStatus> {
    if !installed() {
        return None;
    }
    let _check = LISTENER_CHECK.try_lock().ok()?;
    if listener_stopping() {
        return None;
    }
    let running = listener_probe(listener_lock);
    let recovering = LISTENER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .error
        .is_some();
    let retry_due = TASK_VALIDATION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retry_after
        .map_or(recovering && running.is_ok(), |at| Instant::now() >= at);
    if retry_due {
        // A deadline may have interrupted registration; revalidate and finish that migration.
        let _ = checked_listener_task(false, true);
    }
    let result = task_validation_result(if retry_due {
        listener_probe(listener_lock)
    } else {
        running
    });
    if listener_stopping() {
        return None;
    }
    Some(finish_listener(&result))
}
fn listener_probe(mut probe: impl FnMut() -> io::Result<Option<File>>) -> Result<(), String> {
    match probe() {
        Ok(lock) => {
            drop(lock);
            Err("The meeting listener is not running. Try again.".into())
        }
        Err(error) if error.raw_os_error() == Some(32) => Ok(()),
        Err(_) => Err("Could not check whether the listener is running.".into()),
    }
}
fn wait_for_listener(
    deadline: Instant,
    mut probe: impl FnMut() -> io::Result<Option<File>>,
    mut wait: impl FnMut(Duration),
) -> Result<(), String> {
    loop {
        match probe() {
            Err(error) if error.raw_os_error() == Some(32) => return Ok(()),
            Err(_) => return Err("Could not check whether the listener is running.".into()),
            Ok(lock) => drop(lock),
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(
                "The meeting listener did not start. Try again, or reinstall it if this continues."
                    .into(),
            );
        }
        wait(Duration::from_millis(100).min(remaining));
    }
}
fn prepare_listener() -> Result<(), String> {
    if !installed() {
        return Ok(());
    }
    if listener_stopping() {
        return Err("Sidenote is updating its meeting listener. Wait for the update to finish, then reopen Sidenote.".into());
    }
    {
        let mut status = LISTENER.lock().unwrap_or_else(|e| e.into_inner());
        status.ready = false;
        status.preparing = true;
        status.error = None;
    }
    crate::listener::log_process("listener readiness reason=preparing");
    match checked_listener_task(false, true) {
        Ok(()) => {}
        Err(ListenerTaskError::TimedOut) => {
            return task_validation_result(listener_probe(listener_lock));
        }
        Err(error) => return Err(error.message()),
    }
    match listener_lock() {
        Ok(lock) => {
            drop(lock);
            task_command(true)?;
            wait_for_listener(
                Instant::now() + Duration::from_secs(8),
                listener_lock,
                std::thread::sleep,
            )?;
        }
        Err(error) if error.raw_os_error() == Some(32) => {}
        Err(_) => return Err("Could not check whether the listener is running.".into()),
    }
    Ok(())
}
/// Runs on the window's worker, including recovery after a failed asynchronous apply.
pub fn window_start(publish: impl Fn(Phase)) -> Result<WindowStart, String> {
    if !installed() {
        return Ok(WindowStart::default());
    }
    let channel = channel();
    let pending = manager(channel)
        .ok()
        .and_then(|mgr| pending_asset(&mgr, channel));
    let directory = directory();
    let outcome = window_start_at(
        &directory,
        (Instant::now, std::thread::sleep),
        || reconcile_on_device(&directory, Instant::now(), ReconciliationBudget::App),
        ensure_listener,
        complete_pipeline_install,
        save_status,
        || {
            if !listener_stopping() && pending.is_some() {
                apply(true, Vec::new(), publish)?;
            }
            Ok(())
        },
    )?;
    Ok(WindowStart {
        pending_status: (outcome == ReconcileOutcome::Busy).then(|| Status {
            phase: Phase::Preparing,
            ..read_saved_status(&directory)
        }),
    })
}
fn window_start_at(
    directory: &Path,
    clock: (impl Fn() -> Instant, impl FnMut(Duration)),
    reconcile: impl FnOnce() -> Result<ReconcileOutcome, String>,
    ensure: impl FnOnce() -> Result<(), String>,
    pipeline: impl FnOnce() -> Result<(), String>,
    persist: impl Fn(&Status) -> io::Result<()>,
    resume_apply: impl FnOnce() -> Result<(), String>,
) -> Result<ReconcileOutcome, String> {
    let Some(lock) = retry_hook_lock(directory, clock.0, clock.1)? else {
        return Ok(ReconcileOutcome::Busy);
    };
    let receipt = fs::read(directory.join("install-hook.complete")).ok();
    let needs_reconcile = directory.join("update-listener.resume").exists()
        || receipt.as_deref() != Some(VERSION.as_bytes());
    if needs_reconcile
        && let Err(error) = reconcile()
        && error != PIPELINE_INSTALL_ERROR
    {
        return Err(error);
    }
    ensure()?;
    retire_listener_resume(directory)?;
    let pipeline = if needs_reconcile { Ok(()) } else { pipeline() };
    if !needs_reconcile && pipeline.is_ok() && read_saved_status(directory).reconcile_error {
        save_reconcile_status(Ok(false), directory, &persist)?;
    }
    finish_window_start(
        pipeline,
        |error| {
            let _ = persist(&Status {
                phase: Phase::Error,
                version: VERSION.into(),
                error: Some(error),
                reconcile_error: true,
                ..read_status(directory)
            });
        },
        || {
            drop(lock);
            resume_apply()
        },
    )
    .map(|()| ReconcileOutcome::Complete { changed: false })
}
fn finish_window_start(
    pipeline: Result<(), String>,
    report: impl FnOnce(String),
    resume: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    // Keep the pending marker for retry without gating app updates or listener readiness.
    if let Err(error) = pipeline {
        report(error);
    }
    resume()
}
const PIPELINE_INSTALL_ERROR: &str = "The update could not install the meeting scripts. Check private pipeline settings, close processing jobs and reopen Sidenote to retry; the previous generation and original script backups are retained.";
const PIPELINE_NOT_CONFIGURED_CODE: i32 = 3;

fn pipeline_script_result(code: Option<i32>) -> Result<(), String> {
    match code {
        Some(PIPELINE_NOT_CONFIGURED_CODE) => Ok(()),
        _ => Err(PIPELINE_INSTALL_ERROR.into()),
    }
}

fn complete_pipeline_install() -> Result<(), String> {
    complete_pipeline_install_until(None)
}
fn complete_pipeline_install_until(deadline: Option<Instant>) -> Result<(), String> {
    install_pending_pipeline(
        &local("pipeline-install.pending"),
        || install_pipeline_scripts(deadline),
        || {
            let mut status = read_saved_status(&directory());
            if status.error.as_deref() == Some(PIPELINE_INSTALL_ERROR) {
                if status.phase == Phase::Error {
                    status.phase = Phase::Idle;
                }
                status.error = None;
                status.reconcile_error = false;
                // Persist success before retiring the marker that makes cleanup retryable.
                save_status(&status).map_err(|_| PIPELINE_INSTALL_ERROR)?;
            }
            Ok(())
        },
    )
    .map(|_| ())
}
fn install_pending_pipeline(
    marker: &Path,
    install: impl FnOnce() -> Result<(), String>,
    clear_status: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    if !marker.exists() {
        return Ok(false);
    }
    if fs::read(marker).map_err(|_| PIPELINE_INSTALL_ERROR)? != VERSION.as_bytes() {
        return Err(PIPELINE_INSTALL_ERROR.into());
    }
    install()?;
    clear_status()?;
    fs::remove_file(marker).map_err(|_| PIPELINE_INSTALL_ERROR)?;
    Ok(true)
}

fn install_pipeline_scripts(deadline: Option<Instant>) -> Result<(), String> {
    // Only an installed app update or its pending retry reaches this path.
    let files = std::collections::BTreeMap::from([
        (
            "hex-meeting-compare-delayed.ps1",
            include_str!("../scripts/meeting-pipeline/hex-meeting-compare-delayed.ps1"),
        ),
        (
            "hex-meeting-compare-granola.py",
            include_str!("../scripts/meeting-pipeline/hex-meeting-compare-granola.py"),
        ),
        (
            "hex-meeting-english.ps1",
            include_str!("../scripts/meeting-pipeline/hex-meeting-english.ps1"),
        ),
        (
            "hex-meeting-notes.ps1",
            include_str!("../scripts/meeting-pipeline/hex-meeting-notes.ps1"),
        ),
        (
            "hex-meeting-publish.py",
            include_str!("../scripts/meeting-pipeline/hex-meeting-publish.py"),
        ),
        (
            "hex-meeting-quick-title.ps1",
            include_str!("../scripts/meeting-pipeline/hex-meeting-quick-title.ps1"),
        ),
        (
            "hex-meeting-render.py",
            include_str!("../scripts/meeting-pipeline/hex-meeting-render.py"),
        ),
        (
            "hex-meeting-tasks.py",
            include_str!("../scripts/meeting-pipeline/hex-meeting-tasks.py"),
        ),
        (
            "hex-meeting-watch.ps1",
            include_str!("../scripts/meeting-pipeline/hex-meeting-watch.ps1"),
        ),
        (
            "hex-pipeline-cli.ps1",
            include_str!("../scripts/meeting-pipeline/hex-pipeline-cli.ps1"),
        ),
        (
            "job-contract.json",
            include_str!("../scripts/meeting-pipeline/job-contract.json"),
        ),
        (
            "hex-pipeline-run.py",
            include_str!("../scripts/meeting-pipeline/hex-pipeline-run.py"),
        ),
        (
            "meeting-audio.py",
            include_str!("../scripts/meeting-pipeline/meeting-audio.py"),
        ),
        (
            "meeting-notes-prompt.md",
            include_str!("../scripts/meeting-pipeline/meeting-notes-prompt.md"),
        ),
        (
            "meeting-translate-prompt.md",
            include_str!("../scripts/meeting-pipeline/meeting-translate-prompt.md"),
        ),
    ]);
    let sha256 = |content: &str| {
        Sha256::digest(content.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let hashes: std::collections::BTreeMap<_, _> = files
        .iter()
        .map(|(name, content)| (*name, sha256(content)))
        .collect();
    let entry = include_str!("pipeline_entry.ps1");
    let payload = serde_json::json!({
        "version": VERSION, "files": files, "hashes": hashes,
        "entry": entry, "entry_hash": sha256(entry),
    });
    let error = PIPELINE_INSTALL_ERROR;
    // A child that stalls before reading stdin must not block the hook's deadline.
    let mut input = tempfile::tempfile().map_err(|_| error)?;
    serde_json::to_writer(&mut input, &payload).map_err(|_| error)?;
    std::io::Seek::rewind(&mut input).map_err(|_| error)?;
    if deadline.is_some_and(|limit| Instant::now() >= limit) {
        return Err(error.into());
    }
    let mut child = crate::platform::windows_powershell()
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            include_str!("pipeline_install.ps1"),
        ])
        .stdin(input)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| error)?;
    let status = match deadline {
        Some(limit) => wait_for_child(&mut child, limit),
        None => child.wait(),
    }
    .map_err(|_| error)?;
    if !status.success() {
        return pipeline_script_result(status.code());
    }
    Ok(())
}

const INSTALL_LISTENER_ERROR: &str = "Sidenote could not restart its meeting listener after installation. Reopen Sidenote to retry, or reinstall it if this continues.";
// Velopack 0.0.1298 allows 30 s for Setup (commands/install.rs:190)
// and 15 s for the update hook (commands/apply_windows_impl.rs:155).
const INSTALL_HOOK_WORK_LIMIT: Duration = Duration::from_secs(25);
const UPDATE_HOOK_WORK_LIMIT: Duration = Duration::from_secs(11);
const INSTALL_HOOK_RECOVERY_RESERVE: Duration = Duration::from_secs(4);

enum ReconciliationBudget {
    InstallOrRecovery,
    FastUpdate,
    App,
}

#[derive(Debug, PartialEq, Eq)]
enum ReconcileOutcome {
    Complete { changed: bool },
    Busy,
}

pub fn after_install() -> Result<(), String> {
    run_install_hook(ReconciliationBudget::InstallOrRecovery)
}

pub fn after_update() -> Result<(), String> {
    run_install_hook(ReconciliationBudget::InstallOrRecovery)
}

pub fn after_update_fast() -> Result<(), String> {
    run_install_hook(ReconciliationBudget::FastUpdate)
}

fn run_install_hook(budget: ReconciliationBudget) -> Result<(), String> {
    let started = Instant::now();
    if !installed() {
        return Ok(());
    }
    match reconcile_on_device(&directory(), started, budget)? {
        ReconcileOutcome::Busy => Err(UPDATE_RECONCILING.into()),
        ReconcileOutcome::Complete { .. } => Ok(()),
    }
}
fn reconcile_on_device(
    directory: &Path,
    started: Instant,
    budget: ReconciliationBudget,
) -> Result<ReconcileOutcome, String> {
    reconcile_installed(
        directory,
        started,
        budget,
        listener_task,
        |deadline| complete_pipeline_install_until(Some(deadline)),
        recover_listener_until,
        save_status,
    )
}
fn save_reconcile_status(
    result: Result<bool, String>,
    directory: &Path,
    persist: impl FnOnce(&Status) -> io::Result<()>,
) -> Result<ReconcileOutcome, String> {
    let status = match &result {
        Err(error) => Status {
            phase: Phase::Error,
            version: VERSION.into(),
            error: Some(error.clone()),
            reconcile_error: true,
            ..read_saved_status(directory)
        },
        Ok(_) => after_update_status(
            read_saved_status(directory),
            &directory.join("pipeline-install.pending"),
        ),
    };
    let saved = persist(&status);
    let changed = result?;
    saved.map_err(|_| "Could not save installation status.")?;
    Ok(ReconcileOutcome::Complete { changed })
}

fn reconcile_installed(
    directory: &Path,
    started: Instant,
    budget: ReconciliationBudget,
    task: impl Fn(ListenerTaskMode, Instant) -> Result<(), ListenerTaskError>,
    pipeline: impl FnOnce(Instant) -> Result<(), String>,
    listener: impl FnOnce(Instant) -> Result<(), String>,
    persist: impl FnOnce(&Status) -> io::Result<()>,
) -> Result<ReconcileOutcome, String> {
    let deadline = started
        + match budget {
            ReconciliationBudget::InstallOrRecovery | ReconciliationBudget::App => {
                INSTALL_HOOK_WORK_LIMIT
            }
            ReconciliationBudget::FastUpdate => UPDATE_HOOK_WORK_LIMIT,
        };
    let pipeline_deadline = deadline - INSTALL_HOOK_RECOVERY_RESERVE;
    let preflight =
        || task(ListenerTaskMode::Preflight, pipeline_deadline).map_err(ListenerTaskError::message);
    let _lock;
    if !matches!(budget, ReconciliationBudget::App) {
        preflight()?;
        let Some(acquired) = acquire_hook_lock(directory)? else {
            return Ok(ReconcileOutcome::Busy);
        };
        _lock = acquired;
    }
    let pipeline = || pipeline(pipeline_deadline);
    let listener = || listener(deadline);
    let result = reconcile_install(directory, preflight, pipeline, listener);
    save_reconcile_status(result, directory, persist)
}

fn recover_listener() -> Result<(), String> {
    recover_listener_until(Instant::now() + TASK_COMMAND_TIMEOUT + Duration::from_secs(8))
}
fn recover_listener_until(deadline: Instant) -> Result<(), String> {
    fs::remove_file(local("update-listener.stop"))
        .or_else(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(error)
            }
        })
        .map_err(|_| "Could not clear the listener stop marker.")?;
    checked_listener_task_until(true, true, deadline).map_err(ListenerTaskError::message)?;
    wait_for_listener(
        deadline.min(Instant::now() + Duration::from_secs(8)),
        listener_lock,
        std::thread::sleep,
    )
}
fn open_hook_lock(directory: &Path) -> io::Result<Option<File>> {
    fs::create_dir_all(directory)?;
    match OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .share_mode(0)
        .open(directory.join("install-hook.lock"))
    {
        Ok(lock) => Ok(Some(lock)),
        Err(error) if error.raw_os_error() == Some(32) => Ok(None),
        Err(error) => Err(error),
    }
}
fn acquire_hook_lock(directory: &Path) -> Result<Option<File>, String> {
    open_hook_lock(directory)
        .map_err(|error| format!("Could not open the Sidenote install hook lock: {error}"))
}
fn retry_hook_lock(
    directory: &Path,
    now: impl Fn() -> Instant,
    mut sleep: impl FnMut(Duration),
) -> Result<Option<File>, String> {
    let deadline = now() + Duration::from_secs(2);
    let mut lock = acquire_hook_lock(directory)?;
    while lock.is_none() && now() < deadline {
        sleep(
            deadline
                .saturating_duration_since(now())
                .min(Duration::from_millis(100)),
        );
        if now() >= deadline {
            break;
        }
        lock = acquire_hook_lock(directory)?;
    }
    Ok(lock)
}
const UPDATE_RECONCILING: &str = "Sidenote is finishing the previous update. Try again shortly.";
fn update_operation_lock(directory: &Path, check_resume: bool) -> Result<File, String> {
    let lock = acquire_hook_lock(directory)?.ok_or(UPDATE_RECONCILING)?;
    if check_resume && directory.join("update-listener.resume").exists() {
        return Err(UPDATE_RECONCILING.into());
    }
    Ok(lock)
}

fn reconcile_install(
    directory: &Path,
    preflight: impl Fn() -> Result<(), String>,
    pipeline: impl FnOnce() -> Result<(), String>,
    listener: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    preflight()?;
    let receipt = directory.join("install-hook.complete");
    let pending = directory.join("pipeline-install.pending");
    let current =
        fs::read(&receipt).ok().as_deref() == Some(VERSION.as_bytes()) && !pending.exists();
    if receipt.exists() {
        fs::remove_file(&receipt).map_err(|_| "Could not retire install hook completion.")?;
    }
    fs::write(&pending, VERSION).map_err(|_| PIPELINE_INSTALL_ERROR)?;
    fs::write(directory.join("update-listener.resume"), b"resume")
        .map_err(|_| INSTALL_LISTENER_ERROR)?;
    // The generation installer verifies hashes and deduplicates publication, even on reinstall.
    let pipeline_result = pipeline();
    let listener_result = listener();
    if pipeline_result.is_ok() && listener_result.is_ok() {
        fs::write(receipt, VERSION).map_err(|_| "Could not save install hook completion.")?;
    }
    if listener_result.is_ok() {
        retire_listener_resume(directory)?;
    }
    pipeline_result.and(listener_result)?;
    Ok(!current)
}
fn retire_listener_resume(directory: &Path) -> Result<(), String> {
    let resume = directory.join("update-listener.resume");
    if resume.exists() {
        fs::remove_file(resume).map_err(|_| "Could not clear listener restart state.")?;
    }
    Ok(())
}
const LEGACY_HOOK_CONTENTION: &str = "Another install hook is reconciling Sidenote. Try again.";
fn after_update_status(status: Status, marker: &Path) -> Status {
    let recovered = status.reconcile_error
        || matches!(
            status.error.as_deref(),
            None | Some(LEGACY_HOOK_CONTENTION | INSTALL_LISTENER_ERROR | PIPELINE_INSTALL_ERROR)
        );
    let status = if status.version == VERSION && !marker.exists() && recovered {
        Status {
            checked: status.checked,
            ..Default::default()
        }
    } else {
        status
    };
    pipeline_status(status, marker)
}
pub fn restarted() {
    let directory = directory();
    if installed()
        && let Ok(Some(_lock)) = acquire_hook_lock(&directory)
    {
        let _ = reconcile_on_device(&directory, Instant::now(), ReconciliationBudget::App);
    }
}
fn restart_listener() {
    if local("update-listener.resume").exists() {
        let _ = fs::remove_file(local("update-listener.stop"));
        if recover_listener().is_ok() {
            let _ = fs::remove_file(local("update-listener.resume"));
        } else {
            let _ = save(&Status { phase: Phase::Error, error: Some("The update finished, but the listener could not restart. Check its Sidenote task.".into()), ..load() });
        }
    }
}
pub fn apply(
    from_window: bool,
    mut restart_args: Vec<String>,
    mut publish: impl FnMut(Phase),
) -> Result<(), String> {
    let _check = LISTENER_CHECK.lock().unwrap_or_else(|e| e.into_inner());
    if !installed() {
        return Ok(());
    }
    let _hook = update_operation_lock(&directory(), !from_window)?;
    if listener_stopping() {
        return Err("An update is already being installed.".into());
    }
    publish(Phase::Preparing);
    let args: Vec<_> = std::env::args().skip(1).collect();
    for pair in args.windows(2).filter(|pair| {
        matches!(
            pair[0].as_str(),
            "--data-dir" | "--meetings-dir" | "--peers-dir"
        )
    }) {
        restart_args.extend(pair.iter().cloned());
    }
    let _check =
        exclusive("update-check.lock").map_err(|_| "Wait for the update download to finish.")?;
    let _window = if from_window {
        None
    } else {
        Some(
            exclusive("update-window.lock")
                .map_err(|_| "Close the Sidenote window before using the update command.")?,
        )
    };
    let _gate = exclusive("update-activity.lock")
        .map_err(|_| "Wait for Ask, routing and notes jobs to finish before installing.")?;
    let channel = channel();
    let mgr = manager(channel)?;
    let asset = pending_asset(&mgr, channel).ok_or("Check for updates before installing.")?;
    verify(&asset)?;
    recording_idle()?;
    task_command(false)?;
    fs::write(local("update-listener.resume"), b"resume")
        .map_err(|_| "Could not save listener restart state.")?;
    publish(Phase::StoppingListener);
    write_stop(std::process::id()).map_err(|_| "Could not stop the listener.")?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let stopped = loop {
        if let Ok(lock) = listener_lock() {
            break Ok(lock);
        }
        if Instant::now() >= deadline {
            break Err("The listener is still busy. Try again later.");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let result = stopped.and_then(|_listener| {
        recording_idle().map_err(|_| "Recording started. Try again when it is finished.")?;
        mgr.wait_exit_then_apply_updates(&asset, true, true, restart_args)
            .map_err(|_| "Could not start the update. Try again.")?;
        // Update.exe now waits for this process to exit, and nothing here can cancel it.
        // Never roll back past this point: a restarted listener could be mid-job when the
        // updater replaces files. If the handoff below fails, the stop marker keeps this
        // process's PID, which is dead after exit, so the next start restarts the listener.
        if let Some(pid) = crate::platform::updater_child().ok().flatten() {
            let _ = write_stop(pid);
        }
        Ok(())
    });
    if let Err(error) = result {
        restart_listener();
        return Err(error.into());
    }
    std::process::exit(0)
}
fn report(status: &Status) -> serde_json::Value {
    serde_json::json!({"current_version":VERSION,"reported_at":chrono::Utc::now().to_rfc3339(),
        "data_dir":crate::core::state_dir(),"peers_dir":crate::core::peers_dir(),
        "feed":source_label(),"listener_task":task().ok(),"update_dir":directory(),
        "installed":installed(),"status":status})
}
fn write_report(status: &Status) -> io::Result<serde_json::Value> {
    let report = report(status);
    if installed() {
        let path = local("update-report.json");
        fs::create_dir_all(directory())?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temp, serde_json::to_vec_pretty(&report)?)?;
        crate::platform::replace_file(&temp, &path)?;
    }
    Ok(report)
}
pub fn cli(args: &[std::ffi::OsString]) -> i32 {
    let probe = args.iter().any(|a| a == "--update-probe");
    let mut persist = probe;
    let result = if args.iter().any(|a| a == "--update-window-start") {
        window_start(|_| {}).map(|start| start.pending_status.unwrap_or_else(load))
    } else if probe {
        recording_idle().map(|_| load())
    } else if args.iter().any(|a| a == "--update-apply") {
        apply(false, vec!["--update-status".into()], |phase| {
            persist = true;
            let _ = save(&Status { phase, ..load() });
        })
        .map(|_| load())
    } else if args.iter().any(|a| a == "--update-check") {
        check(|_| {})
    } else {
        Ok(load())
    };
    let status = match result {
        Ok(s) => s,
        Err(error) => Status {
            phase: Phase::Error,
            error: Some(error),
            ..load()
        },
    };
    if persist
        && installed()
        && let Ok(_lock) = update_operation_lock(&directory(), true)
    {
        let _ = save(&status);
    }
    let report = write_report(&status).unwrap_or_else(|_| report(&status));
    println!("{report}");
    i32::from(status.phase == Phase::Error)
}

#[cfg(test)]
mod window_lock_tests {
    use super::*;

    fn open_lock(path: &std::path::Path) -> io::Result<Option<File>> {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path)
            .map(Some)
    }
    #[test]
    fn window_probe_releases_its_handle_before_waiting() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("listener.lock");
        let held = std::cell::RefCell::new(None);
        let mut waits = 0;
        wait_for_listener(
            Instant::now() + Duration::from_secs(30),
            || open_lock(&path),
            |_| {
                *held.borrow_mut() = open_lock(&path).unwrap();
                waits += 1;
            },
        )
        .unwrap();
        assert_eq!(waits, 1);
    }
    #[test]
    fn readiness_recovers_when_the_listener_holds_the_lock_and_detects_death() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("listener.lock");
        let mut status = ListenerStatus::default();
        status.complete(&listener_probe(|| open_lock(&path)));
        assert!(!status.ready);
        assert!(status.error.is_some());
        // A free-lock probe must also leave no handle behind.
        let listener = open_lock(&path).unwrap();
        status.complete(&listener_probe(|| open_lock(&path)));
        assert!(status.ready);
        assert!(status.error.is_none());
        drop(listener);
        status.complete(&listener_probe(|| open_lock(&path)));
        assert!(!status.ready);
        assert!(status.error.is_some());
    }
    #[test]
    fn only_a_sharing_violation_means_an_existing_window() {
        assert!(matches!(
            WindowLockError::from(io::Error::from_raw_os_error(32)),
            WindowLockError::AlreadyOpen
        ));
        for code in [2, 3, 5, 19, 112] {
            assert!(
                matches!(WindowLockError::from(io::Error::from_raw_os_error(code)), WindowLockError::Io(error) if error.raw_os_error() == Some(code))
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_switch_selects_its_feed_and_only_offers_newer_versions() {
        use Channel::{Dev, Latest, Stable};

        let stable = "0.1.0";
        let dev = "0.1.1-dev.10";
        let latest = "0.1.1-dev.10.main.7";
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        for (name, versions) in [
            ("stable", [stable, "0.0.1-dev.47"]),
            ("dev", ["0.1.1-dev.9", dev]),
            ("latest", ["0.1.1-dev.10.main.6", latest]),
        ] {
            let assets = versions.map(|version| VelopackAsset {
                PackageId: "sidenote".into(),
                FileName: format!("sidenote-{version}-full.nupkg"),
                Version: version.into(),
                Type: "Full".into(),
                SHA256: "a".repeat(64),
                ..Default::default()
            });
            fs::write(
                path.join(format!("releases.{name}.json")),
                serde_json::to_vec(&serde_json::json!({"Assets": assets})).unwrap(),
            )
            .unwrap();
        }
        let locator = VelopackLocatorConfig {
            ManifestPath: path.join("app.nuspec"),
            UpdateExePath: path.join("Update.exe"),
            PackagesDir: path.into(),
            ..Default::default()
        };
        fs::write(&locator.UpdateExePath, "").unwrap();
        for (installed, default, choice, expected) in [
            ("0.0.1-dev.47", Stable, None, Some(stable)),
            ("0.1.1-dev.9", Dev, None, Some(dev)),
            (dev, Latest, None, Some(latest)),
            (stable, Stable, Some(Dev), Some(dev)),
            (dev, Dev, Some(Stable), None),
            (stable, Dev, Some(Stable), None),
            ("0.0.1-dev.47", Dev, Some(Stable), Some(stable)),
            (dev, Dev, Some(Latest), Some(latest)),
            (latest, Latest, Some(Dev), None),
        ] {
            fs::write(
                &locator.ManifestPath,
                format!("<package><metadata><id>sidenote</id><version>{installed}</version><mainExe>sidenote.exe</mainExe><channel>{}</channel></metadata></package>", channel_name(default)),
            )
            .unwrap();
            if let Some(choice) = choice {
                fs::write(path.join("update-ready.json"), "previous download").unwrap();
                save_channel(path, choice).unwrap();
                assert!(!path.join("update-ready.json").exists());
            }
            let selected = selected_channel(path, default);
            assert_eq!(selected, choice.unwrap_or(default));
            let manager = channel_manager(
                Box::new(LocalSource(path.into())),
                selected,
                Some(locator.clone()),
            )
            .unwrap();
            match (manager.check_for_updates().unwrap(), expected) {
                (UpdateCheck::UpdateAvailable(update), Some(version)) => {
                    assert_eq!(update.TargetFullRelease.Version, version);
                }
                (UpdateCheck::NoUpdateAvailable, None) => {}
                _ => panic!("unexpected update for {installed} on {selected:?}"),
            }
        }
    }

    #[test]
    fn public_update_feeds_use_sidenote_release_assets() {
        for (channel, expected) in [
            (Channel::Stable, "latest/download/releases.stable.json"),
            (Channel::Dev, "download/dev/releases.dev.json"),
            (Channel::Latest, "download/latest-main/releases.latest.json"),
        ] {
            let source = HttpSource::new(public_feed(channel));
            let url = source.feed_url(channel_name(channel)).unwrap();
            let expected = format!("https://github.com/harshanandak/sidenote/releases/{expected}");
            assert_eq!(url.as_str(), expected);
        }
    }

    #[test]
    fn latest_channel_persists_and_unknown_values_fall_back() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        let channel_file = path.join("channel");

        fs::write(&channel_file, b"unknown").unwrap();
        assert_eq!(selected_channel(path, Channel::Latest), Channel::Latest);
        assert_eq!(selected_channel(path, Channel::Dev), Channel::Dev);

        save_channel(path, Channel::Latest).unwrap();
        assert_eq!(channel_name(Channel::Latest), "latest");
        assert_eq!(fs::read_to_string(channel_file).unwrap(), "latest");
        assert_eq!(selected_channel(path, Channel::Stable), Channel::Latest);
        save_channel(path, Channel::Stable).unwrap();
        assert_eq!(selected_channel(path, Channel::Latest), Channel::Stable);
    }

    #[test]
    fn same_version_hooks_revalidate_scripts_and_listener() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("install-hook.complete"), "old version").unwrap();
        fs::write(root.path().join("update-listener.resume"), "resume").unwrap();
        let script = root.path().join("synthetic-script.ps1");
        fs::write(&script, "old script").unwrap();
        let calls = std::cell::RefCell::new(Vec::new());
        for expected in [true, false, false] {
            assert_eq!(
                reconcile_install(
                    root.path(),
                    || Ok(()),
                    || {
                        calls.borrow_mut().push("pipeline");
                        install_pending_pipeline(
                            &root.path().join("pipeline-install.pending"),
                            || {
                                fs::write(&script, "new script").unwrap();
                                Ok(())
                            },
                            || Ok(()),
                        )
                        .map(|_| ())
                    },
                    || {
                        assert_eq!(fs::read_to_string(&script).unwrap(), "new script");
                        calls.borrow_mut().push("listener");
                        Ok(())
                    }
                )
                .unwrap(),
                expected
            );
            assert!(!root.path().join("update-listener.resume").exists());
        }
        assert_eq!(
            *calls.borrow(),
            [
                "pipeline", "listener", "pipeline", "listener", "pipeline", "listener"
            ]
        );
    }

    #[test]
    fn failed_install_hook_remains_retryable() {
        for failed_step in ["pipeline", "listener"] {
            let root = tempfile::tempdir().unwrap();
            let started = std::cell::Cell::new(false);
            let pipeline = || {
                install_pending_pipeline(
                    &root.path().join("pipeline-install.pending"),
                    || Ok(()),
                    || Ok(()),
                )
                .map(|_| ())
            };
            assert!(
                reconcile_install(
                    root.path(),
                    || Ok(()),
                    || {
                        if failed_step == "pipeline" {
                            Err(PIPELINE_INSTALL_ERROR.into())
                        } else {
                            pipeline()
                        }
                    },
                    || {
                        started.set(true);
                        Err("listener failed".into())
                    }
                )
                .is_err()
            );
            assert!(
                started.get(),
                "pipeline failure must not leave the listener stopped"
            );
            assert!(!root.path().join("install-hook.complete").exists());
            assert!(root.path().join("update-listener.resume").exists());
            assert_eq!(
                root.path().join("pipeline-install.pending").exists(),
                failed_step == "pipeline"
            );
            assert!(reconcile_install(root.path(), || Ok(()), pipeline, || Ok(())).unwrap());
            assert!(!root.path().join("pipeline-install.pending").exists());
        }
    }

    #[test]
    fn unconfigured_pipeline_install_retires_marker_without_error() {
        assert!(pipeline_script_result(Some(PIPELINE_NOT_CONFIGURED_CODE)).is_ok());
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("pipeline-install.pending");
        fs::write(&marker, VERSION).unwrap();
        assert!(
            install_pending_pipeline(
                &marker,
                || pipeline_script_result(Some(PIPELINE_NOT_CONFIGURED_CODE)),
                || Ok(()),
            )
            .unwrap()
        );
        assert!(!marker.exists());
        assert!(read_status(root.path()).error.is_none());
    }

    #[test]
    fn other_pipeline_exit_keeps_marker_retryable() {
        assert_eq!(
            pipeline_script_result(Some(1)).unwrap_err(),
            PIPELINE_INSTALL_ERROR
        );
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("pipeline-install.pending");
        fs::write(&marker, VERSION).unwrap();
        assert_eq!(
            install_pending_pipeline(
                &marker,
                || pipeline_script_result(Some(1)),
                || panic!("failed install cleared status"),
            )
            .unwrap_err(),
            PIPELINE_INSTALL_ERROR
        );
        assert!(marker.exists());
        assert_eq!(
            read_status(root.path()).error.as_deref(),
            Some(PIPELINE_INSTALL_ERROR)
        );
        assert!(install_pending_pipeline(&marker, || Ok(()), || Ok(())).unwrap());
        assert!(!marker.exists());
    }

    #[test]
    fn pending_pipeline_overrides_a_completed_hook_receipt() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("install-hook.complete"), VERSION).unwrap();
        fs::write(
            root.path().join("pipeline-install.pending"),
            "older version",
        )
        .unwrap();
        assert!(
            reconcile_install(
                root.path(),
                || Ok(()),
                || {
                    install_pending_pipeline(
                        &root.path().join("pipeline-install.pending"),
                        || Ok(()),
                        || Ok(()),
                    )
                    .map(|_| ())
                },
                || Err("listener failed".into())
            )
            .is_err()
        );
        assert!(!root.path().join("install-hook.complete").exists());
        assert!(
            reconcile_install(
                root.path(),
                || Ok(()),
                || {
                    install_pending_pipeline(
                        &root.path().join("pipeline-install.pending"),
                        || Ok(()),
                        || Ok(()),
                    )
                    .map(|_| ())
                },
                || Ok(())
            )
            .unwrap()
        );
    }

    #[test]
    fn same_version_listener_failure_is_visible_without_a_saved_status_and_can_retry() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("install-hook.complete"), VERSION).unwrap();
        // Simulate a failed status save: the existing resume marker must still expose failure.
        fs::create_dir(root.path().join("updates.json")).unwrap();
        let pipeline = || {
            install_pending_pipeline(
                &root.path().join("pipeline-install.pending"),
                || Ok(()),
                || Ok(()),
            )
            .map(|_| ())
        };
        assert!(
            reconcile_install(
                root.path(),
                || Ok(()),
                pipeline,
                || Err(INSTALL_LISTENER_ERROR.into())
            )
            .is_err()
        );
        assert!(!root.path().join("install-hook.complete").exists());
        assert_eq!(
            read_status(root.path()).error.as_deref(),
            Some(INSTALL_LISTENER_ERROR)
        );
        assert!(reconcile_install(root.path(), || Ok(()), pipeline, || Ok(())).unwrap());
        assert!(!root.path().join("update-listener.resume").exists());
        assert!(read_status(root.path()).error.is_none());
    }

    #[test]
    fn overlapping_hook_does_no_work_and_can_retry() {
        let root = tempfile::tempdir().unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .share_mode(0)
            .open(root.path().join("install-hook.lock"))
            .unwrap();
        let files = [
            "install-hook.complete",
            "pipeline-install.pending",
            "update-listener.resume",
            "update-listener.stop",
            "updates.json",
        ];
        for name in files {
            fs::write(root.path().join(name), name).unwrap();
        }
        assert_eq!(
            reconcile_installed(
                root.path(),
                Instant::now(),
                ReconciliationBudget::InstallOrRecovery,
                |_, _| Ok(()),
                |_| panic!("overlapping pipeline"),
                |_| panic!("overlapping listener"),
                |_| panic!("busy hook saved status"),
            )
            .unwrap(),
            ReconcileOutcome::Busy
        );
        for name in files {
            assert_eq!(fs::read_to_string(root.path().join(name)).unwrap(), name);
        }
        drop(lock);
        assert!(
            reconcile_install(
                root.path(),
                || Ok(()),
                || {
                    fs::remove_file(root.path().join("pipeline-install.pending")).unwrap();
                    Ok(())
                },
                || Ok(())
            )
            .unwrap()
        );
    }

    #[test]
    fn pending_pipeline_survives_after_update_reset_and_restart() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("pipeline-install.pending");
        let status_path = root.path().join("updates.json");
        fs::write(&marker, VERSION).unwrap();
        assert!(
            install_pending_pipeline(
                &marker,
                || Err(PIPELINE_INSTALL_ERROR.into()),
                || panic!("failed install cleared status")
            )
            .is_err()
        );
        assert_eq!(fs::read(&marker).unwrap(), VERSION.as_bytes());
        for phase in [
            Phase::Checking,
            Phase::Downloading,
            Phase::Ready,
            Phase::Preparing,
        ] {
            let status = pipeline_status(
                Status {
                    phase: phase.clone(),
                    ..Default::default()
                },
                &marker,
            );
            assert_eq!(status.phase, phase);
            assert_eq!(status.error.as_deref(), Some(PIPELINE_INSTALL_ERROR));
        }
        let status = Status {
            version: VERSION.into(),
            checked: Some("synthetic check".into()),
            ..Default::default()
        };
        fs::write(&status_path, serde_json::to_vec(&status).unwrap()).unwrap();
        for _ in 0..2 {
            let status = after_update_status(read_status(root.path()), &marker);
            assert_eq!(status.phase, Phase::Error);
            assert_eq!(status.error.as_deref(), Some(PIPELINE_INSTALL_ERROR));
            assert_eq!(status.version, VERSION);
            fs::write(&status_path, serde_json::to_vec(&status).unwrap()).unwrap();
            let restarted = read_status(root.path());
            assert_eq!(restarted.phase, Phase::Error);
            assert_eq!(restarted.error.as_deref(), Some(PIPELINE_INSTALL_ERROR));
            assert!(marker.exists());
        }
        // A failed install, including a marker from an older version, remains pending.
        fs::write(&marker, "older version").unwrap();
        assert!(
            install_pending_pipeline(
                &marker,
                || panic!("wrong version installed"),
                || panic!("wrong version cleared status")
            )
            .is_err()
        );
        let reset = pipeline_status(Status::default(), &marker);
        assert_eq!(reset.phase, Phase::Error);
        assert_eq!(reset.error.as_deref(), Some(PIPELINE_INSTALL_ERROR));
        // Only successful completion removes the marker, enabling the normal reset.
        fs::write(&marker, VERSION).unwrap();
        assert!(install_pending_pipeline(&marker, || Ok(()), || Ok(())).unwrap());
        assert!(!marker.exists());
        let cleared = after_update_status(read_status(root.path()), &marker);
        assert_eq!(cleared.phase, Phase::Idle);
        assert!(cleared.error.is_none());
        assert_eq!(cleared.checked.as_deref(), Some("synthetic check"));
    }

    #[test]
    fn pipeline_retry_survives_failure_between_status_save_and_marker_removal() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("pipeline-install.pending");
        let path = root.path().join("updates.json");
        fs::write(&marker, VERSION).unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&Status {
                phase: Phase::Error,
                error: Some(PIPELINE_INSTALL_ERROR.into()),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
        let installs = std::cell::Cell::new(0);
        let run = |interrupt| {
            install_pending_pipeline(
                &marker,
                || {
                    installs.set(installs.get() + 1);
                    Ok(())
                },
                || {
                    fs::write(&path, serde_json::to_vec(&Status::default()).unwrap()).unwrap();
                    if interrupt {
                        Err("injected failure after status save".into())
                    } else {
                        Ok(())
                    }
                },
            )
        };
        assert_eq!(run(true).unwrap_err(), "injected failure after status save");
        assert!(
            marker.exists(),
            "retry marker was removed before cleanup finished"
        );
        let saved = read_saved_status(root.path());
        assert!(saved.error.is_none());
        assert_eq!(
            read_status(root.path()).error.as_deref(),
            Some(PIPELINE_INSTALL_ERROR)
        );
        assert!(run(false).unwrap());
        assert_eq!(installs.get(), 2);
        assert!(!marker.exists());
        assert!(read_status(root.path()).error.is_none());
    }

    #[test]
    fn pipeline_failure_reports_and_still_resumes_app_update() {
        let reported = std::cell::RefCell::new(None);
        let applied = std::cell::Cell::new(false);
        let result = finish_window_start(
            Err(PIPELINE_INSTALL_ERROR.into()),
            |error| *reported.borrow_mut() = Some(error),
            || {
                applied.set(true);
                Err("app update failure".into())
            },
        );
        assert!(applied.get());
        assert_eq!(reported.borrow().as_deref(), Some(PIPELINE_INSTALL_ERROR));
        assert_eq!(result.unwrap_err(), "app update failure");
    }

    #[test]
    fn successful_pipeline_retry_resumes_without_reporting_failure() {
        assert!(finish_window_start(Ok(()), |_| panic!("unexpected failure"), || Ok(())).is_ok());
    }

    // The stalled peer exits when the operation returns; no detached request or worker remains.
    fn with_stalled_server(headers: bool, test: impl FnOnce(HttpSource)) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut source = HttpSource::new(&format!("http://{}", listener.local_addr().unwrap()));
        source.agent = ureq::Agent::config_builder().proxy(None).build().into();
        source.feed_deadline = Duration::from_secs(1);
        source.download_deadline = Duration::from_secs(1);
        let (tx, rx) = mpsc::channel();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(8);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if Instant::now() >= until
                            || rx.try_recv() != Err(mpsc::TryRecvError::Empty)
                        {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("Local accept failed: {error}"),
                }
            };
            if headers {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10000\r\n\r\n{")
                    .unwrap();
            }
            let _ = rx.recv_timeout(Duration::from_secs(5));
        });
        let started = Instant::now();
        test(source);
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = tx.send(());
        server.join().unwrap();
    }

    #[test]
    fn public_feed_expires_while_waiting_for_headers_or_body() {
        for headers in [false, true] {
            with_stalled_server(headers, |source| {
                let error = source
                    .get_release_feed("win", &velopack::bundle::Manifest::default(), "test")
                    .unwrap_err();
                assert!(error.to_string().contains("Try again"), "{error}");
            });
        }
    }

    #[test]
    fn public_download_expires_and_releases_the_destination() {
        with_stalled_server(true, |source| {
            let root = std::env::temp_dir().join(format!("update-timeout-{}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            let destination = root.join("sidenote.nupkg");
            let asset = VelopackAsset {
                PackageId: "sidenote".into(),
                FileName: "sidenote.nupkg".into(),
                SHA256: "a".repeat(64),
                Size: 10000,
                ..Default::default()
            };
            let error = source
                .download_release_entry(&asset, destination.to_str().unwrap(), None)
                .unwrap_err();
            assert!(
                destination.exists(),
                "Request failed before download started: {error}"
            );
            assert!(
                error.to_string().to_lowercase().contains("timeout"),
                "{error}"
            );
            // Windows refuses this rename if a timed-out writer still owns the file.
            fs::rename(&destination, root.join("retry.nupkg")).unwrap();
            fs::remove_dir_all(root).unwrap();
        });
    }

    // Shadow the scheduler and process commands so tests cannot reach a real task or listener.
    const MOCK_LISTENER_TASK: &str = r#"
function Get-MockListenerIdentity {
    $groupOwner = $env:SIDENOTE_TEST_MISMATCH -eq 'default-owner'
    $user = if ($groupOwner) { 'S-1-5-18' } else { 'S-1-5-19' }
    $claimType = if ($env:SIDENOTE_TEST_MISMATCH -eq 'legacy-admin-owner') { [Security.Claims.ClaimTypes]::DenyOnlySid } else { [Security.Claims.ClaimTypes]::GroupSid }
    [PSCustomObject]@{
        User = [Security.Principal.SecurityIdentifier]$user
        UserClaims = @(if ($groupOwner -or $env:SIDENOTE_TEST_MISMATCH -in @('legacy-admin-owner', 'foreign-owner')) {
            $claim = [Security.Claims.Claim]::new($claimType, 'S-1-5-32-544')
            $claim.Properties.Add([Security.Claims.ClaimTypes]::WindowsSubAuthority, '5')
            $claim
        })
    }
}
function New-Object {
    param([string]$ComObject)
    if ($ComObject -ne 'Schedule.Service') { throw 'Unexpected COM request' }
    $task = [PSCustomObject]@{}
    $task | Add-Member ScriptMethod GetSecurityDescriptor {
        param($Flags)
        if ($Flags -notin @(1, 7) -or $env:SIDENOTE_TEST_MISMATCH -eq 'unreadable-owner') { throw 'Synthetic owner read error' }
        if ($env:SIDENOTE_TEST_MISMATCH -eq 'foreign-owner') { return 'O:S-1-5-21-101-102-103-1001' }
        if ($env:SIDENOTE_TEST_MISMATCH -in @('legacy-admin-owner', 'legacy-nonadmin-owner')) { return 'O:BA' }
        if ($env:SIDENOTE_TEST_MISMATCH -eq 'default-owner' -and !$script:userOwned) { return 'O:BA' }
        'O:' + (Get-MockListenerIdentity).User.Value
    }
    $task | Add-Member ScriptMethod SetSecurityDescriptor {
        param($Descriptor, $Flags)
        $owner = [Security.AccessControl.RawSecurityDescriptor]::new($Descriptor).Owner.Value
        if ($env:SIDENOTE_TEST_MISMATCH -ne 'default-owner' -or !$script:registered -or $Flags -ne 16 -or
            $owner -ne (Get-MockListenerIdentity).User.Value) { throw 'Unexpected owner change' }
        Write-TaskOperation 'set-owner'
        $script:userOwned = $true
    }
    $service = [PSCustomObject]@{ Task = $task }
    $service | Add-Member ScriptMethod Connect {}
    $service | Add-Member ScriptMethod GetFolder { $this }
    $service | Add-Member ScriptMethod GetTask {
        switch ($env:SIDENOTE_TEST_MISMATCH) {
            { $_ -in @('absent', 'default-owner') -and !$script:registered } { throw [Runtime.InteropServices.COMException]::new('Synthetic missing task', -2147024894) }
            'inaccessible' { throw [Runtime.InteropServices.COMException]::new('Synthetic read refusal', -2147024891) }
        }
        $this.Task
    }
    $service
}
function Get-ScheduledTask {
    if ($env:SIDENOTE_TEST_MISMATCH -in @('absent', 'default-owner') -and !$script:registered) { return }
    $action = [PSCustomObject]@{ Execute = $env:SIDENOTE_TASK_EXE; Arguments = $env:SIDENOTE_TASK_ARGS }
    $actions = @($action)
    $principal = [PSCustomObject]@{ UserId = (Get-MockListenerIdentity).User.Value; LogonType = 'Interactive'; RunLevel = 'Limited' }
    $settings = [PSCustomObject]@{ Enabled = $true; MultipleInstances = 'IgnoreNew'; ExecutionTimeLimit = 'PT0S' }
    $triggers = @(New-ScheduledTaskTrigger -AtLogOn -User $principal.UserId)
    if ($script:migrated -or $env:SIDENOTE_TEST_MISMATCH -notin @('legacy', 'unsaved', 'validate-only', 'foreign-owner')) { $triggers += New-ScheduledTaskTrigger }
    switch ($env:SIDENOTE_TEST_MISMATCH) {
        'exe' { $action.Execute = 'other.exe' }
        'args' { $action.Arguments = '--other' }
        'extra-args' { $action.Arguments = '--listen --synthetic' }
        'multiple-actions' { $actions += $action }
        'foreign-principal' { $principal.UserId = 'S-1-5-18' }
        'unreadable-principal' { $principal.UserId = 'S-1-invalid' }
        'cross-root' { $action.Execute = 'C:\prior\current\sidenote.exe' }
        'privilege' { $principal.RunLevel = 'Highest' }
        'disabled' { $settings.Enabled = $false }
    }
    [PSCustomObject]@{ TaskName = $env:SIDENOTE_TASK_NAME; Actions = $actions; Principal = $principal; Settings = $settings; Triggers = $triggers }
}
function New-ScheduledTaskTrigger {
    param([switch]$AtLogOn, [string]$User, [switch]$Once, $At, $RepetitionInterval)
    $kind = if ($AtLogOn) { 'MSFT_TaskLogonTrigger' } else { 'MSFT_TaskTimeTrigger' }
    if ($AtLogOn) { $User = ([Security.Principal.SecurityIdentifier]$User).Translate([Security.Principal.NTAccount]).Value }
    [PSCustomObject]@{ CimClass = [PSCustomObject]@{ CimClassName = $kind }; Enabled = $true; UserId = $User; Repetition = [PSCustomObject]@{ Interval = 'PT5M'; Duration = ''; StopAtDurationEnd = $true } }
}
function Write-TaskOperation($Operation) {
    if ($env:SIDENOTE_TEST_OPERATIONS) {
        [IO.File]::AppendAllText($env:SIDENOTE_TEST_OPERATIONS, "$Operation`n")
    }
}
function New-ScheduledTaskAction {}
function New-ScheduledTaskPrincipal {}
function New-ScheduledTaskSettingsSet {}
function Register-ScheduledTask {
    Write-TaskOperation 'register'
    if ($env:SIDENOTE_TEST_MISMATCH -notin @('absent', 'default-owner')) { throw 'Unexpected registration' }
    $script:registered = $true
}
function Set-ScheduledTask {
    param($TaskPath, $TaskName, $Trigger, $Settings)
    Write-TaskOperation 'repair'
    if ($env:SIDENOTE_TEST_MISMATCH -notin @('legacy', 'unsaved')) { throw 'Unexpected migration' }
    if (@($Trigger).Count -ne 2 -or $Settings.MultipleInstances -ne 2) { throw 'Invalid migration' }
    $script:migrated = $env:SIDENOTE_TEST_MISMATCH -ne 'unsaved'
}
function Start-ScheduledTask {
    Write-TaskOperation 'start'
    $script:started = $true
}
function New-ListenerTestStopwatch {
    $script:elapsedMs = 0
    $clock = [PSCustomObject]@{}
    Add-Member -InputObject $clock -MemberType ScriptProperty -Name Elapsed -Value {
        [TimeSpan]::FromMilliseconds($script:elapsedMs)
    }
    $clock
}
function Start-Sleep {
    param([int]$Milliseconds)
    $script:elapsedMs += $Milliseconds
}
function Get-CimInstance {
    $script:samples++
    if ($env:SIDENOTE_TEST_MISMATCH -eq 'no-process' -or
        ($env:SIDENOTE_TEST_MISMATCH -eq 'exits' -and $script:samples -gt 1) -or
        ($env:SIDENOTE_TEST_MISMATCH -eq 'restarting' -and $script:samples -lt 3)) { return }
    [PSCustomObject]@{ ExecutablePath = $env:SIDENOTE_TASK_EXE; CommandLine = '--listen'; ProcessId = 123; CreationDate = [datetime]'2026-01-01' }
}
$script:samples = 0
$script:started = $false
$script:migrated = $false
"#;

    fn mock_task_script() -> String {
        LISTENER_TASK_SCRIPT
            .replace(
                "[Diagnostics.Stopwatch]::StartNew()",
                "(New-ListenerTestStopwatch)",
            )
            .replace(
                "[Security.Principal.WindowsIdentity]::GetCurrent()",
                "(Get-MockListenerIdentity)",
            )
    }

    fn mock_install_task(
        root: &Path,
        scenario: &str,
        mode: ListenerTaskMode,
    ) -> Result<(), ListenerTaskError> {
        run_listener_task(
            crate::platform::windows_powershell()
                .env("SIDENOTE_TEST_MISMATCH", scenario)
                .env("SIDENOTE_TEST_OPERATIONS", root.join("task-operations")),
            &format!("{MOCK_LISTENER_TASK}\n{}", mock_task_script()),
            mode,
            "Sidenote mock listener",
            &root.join("current/sidenote.exe"),
            "--listen",
            // The deadline test uses a simulated clock to isolate it from PowerShell startup.
            Instant::now() + Duration::from_secs(60),
        )
    }

    #[test]
    fn install_preflight_refuses_unsafe_task_without_side_effects() {
        for (scenario, error) in [
            ("foreign-principal", ListenerTaskError::OtherUser),
            ("foreign-owner", ListenerTaskError::OtherUser),
            ("legacy-nonadmin-owner", ListenerTaskError::OtherUser),
            ("privilege", ListenerTaskError::Elevated),
            ("disabled", ListenerTaskError::Disabled),
            ("extra-args", ListenerTaskError::UnmanagedAction),
            ("multiple-actions", ListenerTaskError::UnmanagedAction),
            ("unreadable-principal", ListenerTaskError::PermissionDenied),
            ("unreadable-owner", ListenerTaskError::PermissionDenied),
            ("inaccessible", ListenerTaskError::PermissionDenied),
            ("exe", ListenerTaskError::UnmanagedAction),
            ("cross-root", ListenerTaskError::UnmanagedAction),
            ("changed-under-lock", ListenerTaskError::OtherUser),
        ] {
            let root = tempfile::tempdir().unwrap();
            let files = [
                "pipeline-install.pending",
                "update-listener.stop",
                "update-listener.resume",
                "install-hook.complete",
            ];
            for file in files {
                fs::write(root.path().join(file), file).unwrap();
            }
            let lock = root.path().join("install-hook.lock");
            let result = reconcile_installed(
                root.path(),
                Instant::now(),
                ReconciliationBudget::InstallOrRecovery,
                |mode, _| {
                    let task = if scenario == "changed-under-lock" {
                        match OpenOptions::new().write(true).open(&lock) {
                            Err(error) if error.raw_os_error() == Some(32) => "foreign-owner",
                            Err(error) if error.kind() == io::ErrorKind::NotFound => "none",
                            other => {
                                panic!("expected an absent or exclusively held lock: {other:?}")
                            }
                        }
                    } else {
                        scenario
                    };
                    mock_install_task(root.path(), task, mode)
                },
                |_| panic!("unexpected pipeline call for {scenario}"),
                |_| panic!("unexpected listener recovery for {scenario}"),
                |status| {
                    assert_eq!(scenario, "changed-under-lock");
                    assert!(open_hook_lock(root.path()).unwrap().is_none());
                    fs::write(
                        root.path().join("updates.json"),
                        serde_json::to_vec(status)?,
                    )
                },
            );
            assert_eq!(result.unwrap_err(), error.message(), "{scenario}");
            for file in files {
                assert_eq!(fs::read_to_string(root.path().join(file)).unwrap(), file);
            }
            assert_eq!(lock.exists(), scenario == "changed-under-lock");
            assert_eq!(root.path().join("updates.json").exists(), lock.exists());
            assert!(!root.path().join("task-operations").exists(), "{scenario}");
        }
    }

    #[test]
    fn install_preflight_allows_absent_and_same_install_task() {
        for (scenario, operations) in [
            ("absent", "register\nstart\n"),
            ("default-owner", "register\nset-owner\nstart\n"),
            ("none", "start\n"),
            ("legacy-admin-owner", "start\n"),
            ("legacy", "repair\nstart\n"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let pending = root.path().join("pipeline-install.pending");
            let log = root.path().join("task-operations");
            let probes = std::cell::Cell::new(0);
            assert_eq!(
                reconcile_installed(
                    root.path(),
                    Instant::now(),
                    ReconciliationBudget::InstallOrRecovery,
                    |mode, _| {
                        probes.set(probes.get() + 1);
                        mock_install_task(root.path(), scenario, mode)
                    },
                    |_| {
                        assert_eq!(probes.get(), 2);
                        assert!(!log.exists(), "preflight touched the task: {scenario}");
                        assert_eq!(fs::read_to_string(&pending).unwrap(), VERSION);
                        fs::remove_file(&pending).unwrap();
                        Ok(())
                    },
                    |_| mock_install_task(
                        root.path(),
                        scenario,
                        ListenerTaskMode::Recover {
                            run: true,
                            register: true
                        },
                    )
                    .map_err(ListenerTaskError::message),
                    |_| Ok(()),
                )
                .unwrap_or_else(|error| panic!("{scenario}: {error}")),
                ReconcileOutcome::Complete { changed: true },
                "{scenario}"
            );
            assert_eq!(fs::read_to_string(log).unwrap(), operations, "{scenario}");
            assert!(
                root.path().join("install-hook.complete").exists(),
                "{scenario}"
            );
        }
    }

    #[test]
    fn recovery_refuses_foreign_task_owner_before_repair_or_start() {
        let root = tempfile::tempdir().unwrap();
        for scenario in ["foreign-owner", "legacy-nonadmin-owner"] {
            let refused = mock_install_task(
                root.path(),
                scenario,
                ListenerTaskMode::Recover {
                    run: true,
                    register: true,
                },
            );
            assert!(refused.is_err(), "{scenario}");
            assert!(!root.path().join("task-operations").exists(), "{scenario}");
        }
    }

    #[test]
    fn reconciliation_shares_one_deadline_and_reserves_recovery_time() {
        assert_reconciliation_budget(ReconciliationBudget::InstallOrRecovery, 25);
    }

    #[test]
    fn update_reconciliation_fits_velopack_hook_deadline() {
        assert_reconciliation_budget(ReconciliationBudget::FastUpdate, 11);
    }

    fn assert_reconciliation_budget(budget: ReconciliationBudget, work_seconds: u64) {
        let root = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let clock = std::cell::Cell::new(started);
        let spend = |maximum: Duration, deadline: Instant| {
            let now = clock.get();
            clock.set(now + maximum.min(deadline.saturating_duration_since(now)));
        };
        let result = reconcile_installed(
            root.path(),
            started,
            budget,
            |_, deadline| {
                spend(TASK_COMMAND_TIMEOUT, deadline);
                Ok(())
            },
            |deadline| {
                spend(Duration::from_secs(7), deadline);
                Err(PIPELINE_INSTALL_ERROR.into())
            },
            |deadline| {
                assert!(deadline - clock.get() >= Duration::from_secs(4));
                spend(TASK_COMMAND_TIMEOUT + Duration::from_secs(8), deadline);
                Err(INSTALL_LISTENER_ERROR.into())
            },
            |_| Ok(()),
        );
        assert_eq!(result.unwrap_err(), PIPELINE_INSTALL_ERROR);
        assert_eq!(clock.get() - started, Duration::from_secs(work_seconds));
        assert!(!root.path().join("install-hook.complete").exists());
    }

    #[test]
    fn mismatched_listener_task_is_never_started() {
        use std::os::windows::process::CommandExt;
        let suffix = r#"
} catch { if ($script:started) { exit 2 }; exit 1 }
if ($script:started) { exit 0 }; exit 3
"#;
        let real_clock = "[Diagnostics.Stopwatch]::StartNew()";
        assert_eq!(LISTENER_TASK_SCRIPT.matches(real_clock).count(), 1);
        let script = mock_task_script();
        for mismatch in [
            "none",
            "exe",
            "args",
            "privilege",
            "disabled",
            "legacy",
            "unsaved",
            "validate-only",
            "no-process",
            "exits",
            "restarting",
        ] {
            let status = Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &format!("{MOCK_LISTENER_TASK}\ntry {{\n{script}\n{suffix}"),
                ])
                .env("SIDENOTE_TASK_NAME", "Sidenote mock listener")
                .env("SIDENOTE_TASK_EXE", "sidenote-test.exe")
                .env("SIDENOTE_TASK_ARGS", "--listen")
                .env("SIDENOTE_TASK_RUN", "yes")
                .env(
                    "SIDENOTE_TASK_REGISTER",
                    if matches!(mismatch, "legacy" | "unsaved") {
                        "yes"
                    } else {
                        "no"
                    },
                )
                .env("SIDENOTE_TEST_MISMATCH", mismatch)
                .creation_flags(0x08000000)
                .status()
                .unwrap();
            assert_eq!(
                status.code(),
                Some(if matches!(mismatch, "none" | "legacy" | "restarting") {
                    0
                } else if matches!(mismatch, "no-process" | "exits") {
                    2
                } else {
                    1
                }),
                "{mismatch}"
            );
        }
    }

    #[test]
    fn task_timeout_uses_the_lock_and_retries_without_hiding_definite_failure() {
        let now = Instant::now();
        let mut validation = TaskValidation::default();
        validation.complete(&Err(ListenerTaskError::TimedOut), now);
        assert_eq!(validation.retry_after, Some(now + Duration::from_secs(30)));
        assert!(validation.resolve(Ok(())).is_ok());
        assert!(validation.resolve(Err("listener absent".into())).is_err());
        validation.complete(
            &Err(ListenerTaskError::Failed("mismatched task".into())),
            now,
        );
        assert_eq!(validation.resolve(Ok(())).unwrap_err(), "mismatched task");
        validation.complete(&Err(ListenerTaskError::TimedOut), now);
        assert_eq!(validation.resolve(Ok(())).unwrap_err(), "mismatched task");
        validation.complete(&Ok(()), now);
        assert!(validation.resolve(Ok(())).is_ok());
        assert!(validation.retry_after.is_none());
        assert!(validation.resolve(Err("listener absent".into())).is_err());
    }

    #[test]
    fn listener_failure_is_independent_and_retry_clears_it_only_on_success() {
        let mut listener = ListenerStatus::default();
        listener.complete(&Err(
            "Listener task does not target this user and install".into()
        ));
        assert!(!listener.ready);
        assert!(listener.error.is_some());
        listener.complete(&Err("Still mismatched".into()));
        assert!(!listener.ready);
        assert_eq!(listener.error.as_deref(), Some("Still mismatched"));
        listener.complete(&Ok(()));
        assert!(listener.ready);
        assert!(listener.error.is_none());
    }

    #[test]
    fn focus_checks_wait_more_than_thirty_minutes_since_the_attempt() {
        let attempt = Instant::now();
        assert!(!focus_check_due(attempt, attempt));
        assert!(!focus_check_due(attempt, attempt + FOCUS_CHECK_AFTER));
        assert!(focus_check_due(
            attempt,
            attempt + FOCUS_CHECK_AFTER + Duration::from_nanos(1)
        ));
        // A failed check also becomes the latest attempt.
        let retry = attempt + FOCUS_CHECK_AFTER + Duration::from_secs(1);
        assert!(!focus_check_due(retry, retry + Duration::from_secs(60)));
    }

    #[test]
    fn rejects_untrusted_package_paths_before_cache_lookup() {
        for name in [
            "../sidenote.nupkg",
            "other/sidenote.nupkg",
            "https://example.com/a.nupkg",
        ] {
            let asset = VelopackAsset {
                PackageId: "sidenote".into(),
                FileName: name.into(),
                SHA256: "a".repeat(64),
                ..Default::default()
            };
            assert!(!valid_asset(&asset));
            assert_eq!(
                cache_path(&asset).unwrap_err(),
                "The update package is invalid."
            );
        }
    }

    #[test]
    fn feed_error_names_the_missing_channel_index() {
        let root = tempfile::tempdir().unwrap();
        // FileSource reads the index with read_to_string, so a missing index is Io(NotFound).
        let error = velopack::Error::Io(
            fs::read_to_string(root.path().join("releases.dev.json")).unwrap_err(),
        );
        let message = feed_error(Some(root.path()), Channel::Dev, &error);
        assert!(message.contains("releases.dev.json"), "{message}");
        assert!(message.contains("missing"), "{message}");
        assert!(
            message.contains(&root.path().display().to_string()),
            "{message}"
        );
    }

    #[test]
    fn feed_error_keeps_access_denied_for_an_existing_index() {
        let root = tempfile::tempdir().unwrap();
        let error = velopack::Error::Io(io::Error::from(io::ErrorKind::PermissionDenied));
        let message = feed_error(Some(root.path()), Channel::Dev, &error);
        assert!(message.contains("releases.dev.json"), "{message}");
        assert!(message.contains("permission denied"), "{message}");
        assert!(!message.contains("missing"), "{message}");
    }

    #[test]
    fn feed_error_never_copies_feed_content() {
        let root = tempfile::tempdir().unwrap();
        let bad = r#"{"Assets":[{"Size":"private-feed-marker"}]}"#;
        let error = velopack::Error::Json(
            serde_json::from_str::<velopack::VelopackAssetFeed>(bad).unwrap_err(),
        );
        let message = feed_error(Some(root.path()), Channel::Dev, &error);
        assert!(!message.contains("private-feed-marker"), "{message}");
        assert!(message.contains("releases.dev.json"), "{message}");
        assert!(message.contains("line 1"), "{message}");
    }

    #[test]
    fn feed_error_keeps_the_connection_message_for_the_public_feed() {
        let error = velopack::Error::Generic("synthetic".into());
        assert_eq!(
            feed_error(None, Channel::Dev, &error),
            "Could not read the update feed. Check your connection and try again."
        );
    }
}

#[cfg(test)]
#[path = "updates_windows/reconcile_tests.rs"]
mod reconcile_tests;
