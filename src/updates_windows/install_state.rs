use super::install_record::{Diagnostic, FILE, ListenerWork, PipelineWork, Record, read};
use super::{InstallGuard, PIPELINE_INSTALL_ERROR, VERSION, write_json};
use std::{fs, path::Path};

const RECEIPT: &str = "install-hook.complete";
const PIPELINE: &str = "pipeline-install.pending";
const RESUME: &str = "update-listener.resume";
const STORAGE: &str =
    "Could not save install-state.json. Check the update directory permissions and retry.";
pub(super) const CONTENTION: &str = "Another install hook is reconciling Sidenote. Try again.";
const ROLLBACK: &str =
    "The update finished, but the listener could not restart. Check its Sidenote task.";
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PipelineCompletion {
    Complete,
    NotConfigured,
}

impl Record {
    fn migrate(directory: &Path) -> Self {
        let current = fs::read_to_string(directory.join(RECEIPT)).ok().as_deref() == Some(VERSION);
        let listener_pending = !current || directory.join(RESUME).try_exists().unwrap_or(true);
        let pipeline_pending = !current || directory.join(PIPELINE).try_exists().unwrap_or(true);
        let saved = fs::read(directory.join("updates.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .unwrap_or_default();
        let listener_diagnostic = match saved["error"].as_str() {
            Some(CONTENTION | PIPELINE_INSTALL_ERROR) | None => None,
            Some(super::INSTALL_LISTENER_ERROR | ROLLBACK) => Some(Diagnostic::ListenerRecovery),
            Some(_) if saved["reconcile_error"] == true => Some(Diagnostic::ListenerRecovery),
            Some(_) => None,
        };
        Self {
            schema_version: 1,
            app_version: VERSION.into(),
            listener_work: if listener_pending {
                ListenerWork::Pending
            } else {
                ListenerWork::Complete
            },
            pipeline_work: if pipeline_pending {
                PipelineWork::Pending
            } else {
                PipelineWork::Complete
            },
            listener_diagnostic,
            pipeline_diagnostic: (saved["error"].as_str() == Some(PIPELINE_INSTALL_ERROR))
                .then_some(Diagnostic::PipelineInstall),
        }
    }

    fn pipeline_pending(&self) -> bool {
        self.pipeline_work == PipelineWork::Pending
    }

    fn complete(&self) -> bool {
        self.app_version == VERSION
            && self.listener_work == ListenerWork::Complete
            && !self.pipeline_pending()
    }
}

fn snapshot(directory: &Path) -> Option<Record> {
    read(directory).ok().flatten()
}

pub(super) fn pipeline_pending(directory: &Path) -> bool {
    snapshot(directory).is_some_and(|state| state.pipeline_pending())
}

pub(super) fn listener_pending(directory: &Path) -> bool {
    snapshot(directory).is_some_and(|state| state.listener_work == ListenerWork::Pending)
}

#[cfg(test)]
pub(super) fn completion_receipt_version(directory: &Path) -> Option<String> {
    snapshot(directory)
        .filter(Record::complete)
        .map(|state| state.app_version)
}

#[cfg(test)]
pub(super) fn pipeline_version(directory: &Path) -> Option<String> {
    snapshot(directory)
        .filter(Record::pipeline_pending)
        .map(|state| state.app_version)
}

impl InstallGuard<'_> {
    pub(super) fn state(&self) -> Result<Record, String> {
        let state = match read(self.directory)? {
            Some(state) => state,
            None => {
                let state = Record::migrate(self.directory);
                self.persist_state(&state)?;
                state
            }
        };
        // A sharing violation during cleanup is retried by the next owner.
        for name in [RECEIPT, PIPELINE, RESUME] {
            let _ = fs::remove_file(self.directory.join(name));
        }
        Ok(state)
    }

    fn persist_state(&self, state: &Record) -> Result<(), String> {
        write_json(&self.directory.join(FILE), state).map_err(|_| STORAGE.into())
    }

    pub(super) fn listener_pending(&self) -> Result<bool, String> {
        Ok(read(self.directory)?.map_or_else(
            || self.directory.join(RESUME).exists(),
            |state| state.listener_work == ListenerWork::Pending,
        ))
    }

    pub(super) fn mark_listener_pending(&self) -> Result<(), String> {
        let mut state = self.state()?;
        state.listener_work = ListenerWork::Pending;
        self.persist_state(&state)
    }

    pub(super) fn finish_listener_recovery(&self, recovered: bool) -> Result<(), String> {
        let mut state = self.state()?;
        if recovered {
            state.listener_work = ListenerWork::Complete;
        }
        state.listener_diagnostic = (!recovered).then_some(Diagnostic::ListenerRecovery);
        self.persist_state(&state)
    }

    pub(super) fn begin_reconciliation(&self, force: bool) -> Result<bool, String> {
        let mut state = self.state()?;
        let current = state.complete();
        if force || state.app_version != VERSION {
            state.app_version = VERSION.into();
            state.listener_work = ListenerWork::Pending;
            state.pipeline_work = PipelineWork::Pending;
            self.persist_state(&state)?;
        }
        Ok(current)
    }
}

pub(super) fn install_pending_pipeline(
    guard: &InstallGuard<'_>,
    install: impl FnOnce() -> Result<PipelineCompletion, String>,
    clear_status: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    let mut state = guard.state()?;
    if !state.pipeline_pending() {
        return Ok(false);
    }
    if state.app_version != VERSION {
        return Err(PIPELINE_INSTALL_ERROR.into());
    }
    match install() {
        Ok(completion) => {
            clear_status()?;
            state.pipeline_work = match completion {
                PipelineCompletion::Complete => PipelineWork::Complete,
                PipelineCompletion::NotConfigured => PipelineWork::NotConfigured,
            };
            state.pipeline_diagnostic = None;
            guard.persist_state(&state)?;
            Ok(true)
        }
        Err(error) => {
            state.pipeline_diagnostic = Some(Diagnostic::PipelineInstall);
            guard.persist_state(&state)?;
            Err(error)
        }
    }
}
