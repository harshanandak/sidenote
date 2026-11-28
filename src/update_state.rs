//! Update state shared by the screens and platform adapters.
use serde::{Deserialize, Serialize};

pub enum CheckRequest {
    Manual,
    Focus,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Manual,
    Preparing,
    StoppingListener,
    Restarting,
    Checking,
    Available,
    Downloading,
    Ready,
    Error,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub channel: Option<String>,
    pub phase: Phase,
    pub version: String,
    pub progress: u8,
    pub notes: String,
    pub checked: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ListenerStatus {
    pub ready: bool,
    pub preparing: bool,
    pub error: Option<String>,
}
