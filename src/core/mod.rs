//! Meeting data and background work, with no Rune or other UI dependencies.
pub mod agent_consent;
pub mod agent_instructions;
pub mod annotations;
pub mod area_ops;
pub mod area_version;
pub mod areas;
pub mod ask;
pub mod ask_all;
pub mod ask_options;
pub mod ask_session;
mod channel;
pub use channel::{CHANNEL, Channel};
pub mod asr;
pub mod check_state;
pub mod config;
pub mod engine_settings;
pub mod export;
pub(crate) mod failure;
pub mod growing_wav;
pub(crate) mod idle_pass;
pub mod lid;
pub mod meeting_delete;
pub(crate) mod meeting_evidence;
pub mod meeting_job;
pub mod meeting_parts;
pub mod meeting_router;
pub mod meeting_watch;
pub mod provider;
pub mod recording_guard;
pub mod recording_source;
pub mod relay_preview;
#[cfg(windows)]
pub mod speech_host;
pub use config::{Capability, Config, Feature, Features};
pub mod check_v2;
pub mod daily_check;
pub mod diary;
pub mod memory;
pub mod note_edit_tool;
pub mod notes;
mod paths;
#[cfg(any(
    test,
    target_os = "linux",
    debug_assertions,
    feature = "relay-qualification"
))]
pub(crate) use paths::ReadOnlyScope;
pub mod project_merge;
pub mod prompt_library;
pub mod settle;
#[cfg(target_os = "linux")]
pub mod speech_jobs;
#[cfg(target_os = "linux")]
pub mod speech_sessions;
pub use paths::{configure_paths, is_proof_path};
pub mod agent_tools;
mod device;
pub mod file_reads;
pub mod live_edit;
pub mod mcp_connect;
pub use sync_state::{EditView, mutation_generation};
mod memory_migration;
pub(crate) mod store_io;
#[cfg(target_os = "linux")]
pub use store_io::UiLockBudget;
pub mod summary_edits;
mod sync_state;
pub use daily_check::{apply_action, company_memory, latest_check, list_checks, load_check};
pub use device::DeviceSettings;
mod grouping;
pub mod live;
mod meetings;
mod persistence;
mod runner_process;
pub mod runners;
mod search;
mod tasks;
mod ui_prefs;
pub use ask::{ask, ask_with_progress};
pub use grouping::*;
pub use meetings::*;
pub use persistence::*;
pub use runner_process::run_runner;
pub use runners::{AskProgress, Runner, last_used_runner, list_runners};
pub use search::*;
pub use tasks::*;
pub use ui_prefs::{TasksLayout, UiPrefs};

pub mod model_catalog;
mod model_catalog_data;

pub fn validation_message(error: &std::io::Error) -> Option<&str> {
    store_io::message(error)
}

pub mod companies;
pub mod people;
pub mod tasks_board;
pub mod tasks_v2;
pub use tasks_board::*;
pub use tasks_v2::*;

pub mod task_titles;

/// Reconcile transported journals without requiring a window or a Hex installation.
pub fn reconcile_service() -> bool {
    sync_state::reconcile(&state_dir()).is_empty()
}
#[cfg(any(target_os = "linux", debug_assertions, feature = "relay-qualification"))]
pub(crate) fn service_journal_revision(root: &std::path::Path) -> std::io::Result<String> {
    sync_state::ingestion_revision(root)
}

#[cfg(any(target_os = "linux", debug_assertions, feature = "relay-qualification"))]
pub(crate) fn scan_ingestion_source(
    path: &std::path::Path,
    pending: bool,
) -> (Vec<Meeting>, Vec<String>) {
    meetings::scan_in_mode(path, pending)
}
