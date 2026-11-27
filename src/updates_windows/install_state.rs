use super::{INSTALL_LISTENER_ERROR, InstallGuard, PIPELINE_INSTALL_ERROR, VERSION};
use std::{fs, io, path::Path};

const RECEIPT: &str = "install-hook.complete";
const PIPELINE: &str = "pipeline-install.pending";
const RESUME: &str = "update-listener.resume";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PipelineCompletion {
    Complete,
    NotConfigured,
}

// Display and feed status can sample the legacy files without install admission.
pub(super) fn pipeline_pending(directory: &Path) -> bool {
    directory.join(PIPELINE).exists()
}

pub(super) fn listener_pending(directory: &Path) -> bool {
    directory.join(RESUME).exists()
}

pub(super) fn listener_completed_version(directory: &Path) -> Option<String> {
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
        let current = listener_completed_version(self.directory).as_deref() == Some(VERSION)
            && !pipeline_pending(self.directory);
        if receipt.exists() {
            fs::remove_file(&receipt).map_err(|_| "Could not retire install hook completion.")?;
        }
        fs::write(self.directory.join(PIPELINE), VERSION).map_err(|_| PIPELINE_INSTALL_ERROR)?;
        self.mark_listener_pending()
            .map_err(|_| INSTALL_LISTENER_ERROR)?;
        Ok(current)
    }

    pub(super) fn finish_reconciliation(&self) -> Result<(), String> {
        fs::write(self.directory.join(RECEIPT), VERSION)
            .map_err(|_| "Could not save install hook completion.")?;
        if self.listener_pending() {
            self.clear_listener_pending()
                .map_err(|_| "Could not clear listener restart state.")?;
        }
        Ok(())
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
