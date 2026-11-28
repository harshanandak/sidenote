#[cfg(windows)]
#[path = "updates_windows.rs"]
mod implementation;
#[cfg(not(windows))]
#[path = "updates_unix.rs"]
mod implementation;
#[cfg(windows)]
pub(crate) use implementation::verify_sha256;
pub use implementation::*;

pub const CONFIRMATION_TARGET: &str = "update-confirmation.json";
