pub use crate::update_state::{CheckRequest, ListenerStatus, Phase, Status};
use std::{fs::File, io};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub fn activity() -> io::Result<Option<File>> {
    Ok(None)
}
pub fn listener_lock() -> io::Result<Option<File>> {
    crate::core::store_io::lock(&crate::core::state_dir().join("local/service.lock")).map(Some)
}
pub fn listener_stopping() -> bool {
    listener_stop_source().is_some()
}
pub(crate) fn listener_stop_source() -> Option<&'static str> {
    crate::service::stopping().then_some("service-stop")
}
pub fn cli(_args: &[std::ffi::OsString]) -> i32 {
    eprintln!("{}", crate::platform::UNAVAILABLE);
    2
}

// Desktop launch has no Linux updater or tray/listener installer yet.
pub const FOCUS_CHECK_AFTER: std::time::Duration = std::time::Duration::from_secs(30 * 60);
pub fn installed() -> bool {
    false
}
pub fn channel() -> crate::core::Channel {
    crate::core::CHANNEL
}
pub fn set_channel(_: crate::core::Channel) -> Result<(), String> {
    Err("Updates are unavailable on Linux.".into())
}
pub fn load() -> Status {
    Status {
        phase: Phase::Manual,
        ..Status::default()
    }
}
pub fn listener_status() -> ListenerStatus {
    ListenerStatus {
        ready: false,
        preparing: false,
        error: Some("The desktop meeting listener is unavailable on Linux.".into()),
    }
}
pub fn listener_ready() -> bool {
    false
}
pub fn probe_listener() -> Option<ListenerStatus> {
    Some(listener_status())
}
pub fn ensure_listener() -> Result<(), String> {
    Err("The desktop meeting listener is unavailable on Linux.".into())
}
pub fn start(_: impl Fn(Status) + Send + 'static) -> Option<std::sync::mpsc::Sender<CheckRequest>> {
    None
}
pub fn window_start(_: impl Fn(Phase), _: impl Fn(ListenerStatus)) -> Result<(), String> {
    Err("Updates are unavailable on Linux.".into())
}
pub fn apply(_: bool, _: Vec<String>, _: impl FnMut(Phase)) -> Result<(), String> {
    Err("Updates are unavailable on Linux.".into())
}
pub fn source_label() -> String {
    "Updates are unavailable on Linux.".into()
}
pub fn feed() -> Option<std::path::PathBuf> {
    None
}
