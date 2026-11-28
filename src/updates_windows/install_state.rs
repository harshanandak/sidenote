use super::install_record::{Diagnostic, ListenerWork, PipelineWork, Record, read};
use super::{INSTALL_LISTENER_ERROR, InstallGuard, PIPELINE_INSTALL_ERROR, VERSION};
use std::{fs, io, path::Path};

const RECEIPT: &str = "install-hook.complete";
const PIPELINE: &str = "pipeline-install.pending";
const RESUME: &str = "update-listener.resume";
pub(super) const CONTENTION: &str = "Another install hook is reconciling Sidenote. Try again.";
pub(super) const ROLLBACK: &str =
    "The update finished, but the listener could not restart. Check its Sidenote task.";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PipelineCompletion {
    Complete,
    NotConfigured,
}

impl Record {
    fn from_legacy(directory: &Path) -> Self {
        let saved = saved_status(directory);
        let listener_diagnostic = saved["error"]
            .as_str()
            .filter(|error| listener_error(error, &saved))
            .map(|_| Diagnostic::ListenerRecovery);
        Self {
            schema_version: 1,
            app_version: VERSION.into(),
            listener_work: if listener_pending(directory) {
                ListenerWork::Pending
            } else {
                ListenerWork::Complete
            },
            pipeline_work: if pipeline_pending(directory) {
                PipelineWork::Pending
            } else {
                PipelineWork::Complete
            },
            listener_diagnostic,
            pipeline_diagnostic: (saved["error"].as_str() == Some(PIPELINE_INSTALL_ERROR))
                .then_some(Diagnostic::PipelineInstall),
        }
    }

    pub(super) fn needs_listener_recovery(&self) -> bool {
        self.app_version != VERSION || self.listener_work == ListenerWork::Pending
    }

    pub(super) fn pipeline_pending(&self) -> bool {
        self.pipeline_work == PipelineWork::Pending
    }
}

fn saved_status(directory: &Path) -> serde_json::Value {
    fs::read(directory.join("updates.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn listener_error(error: &str, saved: &serde_json::Value) -> bool {
    match error {
        CONTENTION | PIPELINE_INSTALL_ERROR => false,
        INSTALL_LISTENER_ERROR | ROLLBACK => true,
        _ => saved["reconcile_error"] == true && saved["error"].as_str() == Some(error),
    }
}

pub(super) fn owns_error(directory: &Path, error: &str) -> bool {
    matches!(error, CONTENTION | PIPELINE_INSTALL_ERROR)
        || listener_error(error, &saved_status(directory))
}

pub(super) fn status_record(directory: &Path) -> Result<(Record, bool), String> {
    let record = read(directory)?;
    let canonical = record.is_some();
    Ok((
        record.unwrap_or_else(|| Record::from_legacy(directory)),
        canonical,
    ))
}

// Display and feed status can sample the legacy files without install admission.
pub(super) fn pipeline_pending(directory: &Path) -> bool {
    directory.join(PIPELINE).exists()
}

pub(super) fn listener_pending(directory: &Path) -> bool {
    directory.join(RESUME).exists()
}

pub(super) fn completion_receipt_version(directory: &Path) -> Option<String> {
    fs::read_to_string(directory.join(RECEIPT)).ok()
}

#[cfg(test)]
pub(super) fn pipeline_version(directory: &Path) -> Option<String> {
    fs::read_to_string(directory.join(PIPELINE)).ok()
}

impl InstallGuard<'_> {
    pub(super) fn listener_pending(&self) -> bool {
        listener_pending(self.directory)
    }

    pub(super) fn mark_listener_pending(&self) -> io::Result<()> {
        fs::write(self.directory.join(RESUME), b"resume")
    }

    pub(super) fn clear_listener_pending(&self) -> io::Result<()> {
        fs::remove_file(self.directory.join(RESUME))
    }

    pub(super) fn begin_reconciliation(&self) -> Result<bool, String> {
        let receipt = self.directory.join(RECEIPT);
        let current = completion_receipt_version(self.directory).as_deref() == Some(VERSION)
            && !pipeline_pending(self.directory);
        if receipt.exists() {
            fs::remove_file(&receipt).map_err(|_| "Could not retire install hook completion.")?;
        }
        fs::write(self.directory.join(PIPELINE), VERSION).map_err(|_| PIPELINE_INSTALL_ERROR)?;
        self.mark_listener_pending()
            .map_err(|_| INSTALL_LISTENER_ERROR)?;
        Ok(current)
    }

    pub(super) fn publish_completion_receipt(&self) -> Result<(), String> {
        fs::write(self.directory.join(RECEIPT), VERSION)
            .map_err(|_| "Could not save install hook completion.".into())
    }
}

pub(super) fn install_pending_pipeline(
    guard: &InstallGuard<'_>,
    install: impl FnOnce() -> Result<PipelineCompletion, String>,
    clear_status: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    let marker = guard.directory.join(PIPELINE);
    if !marker.exists() {
        return Ok(false);
    }
    if fs::read(&marker).map_err(|_| PIPELINE_INSTALL_ERROR)? != VERSION.as_bytes() {
        return Err(PIPELINE_INSTALL_ERROR.into());
    }
    install()?;
    clear_status()?;
    fs::remove_file(marker).map_err(|_| PIPELINE_INSTALL_ERROR)?;
    Ok(true)
}
