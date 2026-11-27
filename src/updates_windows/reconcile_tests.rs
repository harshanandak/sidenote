use super::*;

#[test]
fn hook_lock_sharing_violation_is_busy() {
    let root = tempfile::tempdir().unwrap();
    let _lock = open_hook_lock(root.path()).unwrap().unwrap();
    assert!(open_hook_lock(root.path()).unwrap().is_none());
}

#[test]
fn hook_lock_other_io_error_is_failure() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("install-hook.lock")).unwrap();
    let error = open_hook_lock(root.path()).unwrap_err();
    assert_ne!(error.raw_os_error(), Some(32));
}

fn run_busy_window(directory: &Path, reconcile: impl FnOnce() -> Result<ReconcileOutcome, String>) {
    let _lock = open_hook_lock(directory).unwrap().unwrap();
    let outcome = window_start_at(
        directory,
        acquire_hook_lock,
        reconcile,
        || panic!("busy startup checked the listener"),
        || panic!("busy startup retried the pipeline"),
        |_| panic!("busy startup persisted status"),
        || panic!("busy startup applied an update"),
    );
    assert_eq!(outcome.unwrap(), ReconcileOutcome::Busy);
}

#[test]
fn busy_startup_preserves_saved_failure_before_app_preflight_can_time_out() {
    let root = tempfile::tempdir().unwrap();
    let saved = root.path().join("updates.json");
    let bytes = br#"{"phase":"Error","error":"prior task failure"}"#;
    fs::write(&saved, bytes).unwrap();
    let resume = root.path().join("update-listener.resume");
    fs::write(&resume, b"resume").unwrap();
    run_busy_window(root.path(), || Err(ListenerTaskError::TimedOut.message()));
    assert_eq!(fs::read(saved).unwrap(), bytes);
    assert_eq!(fs::read(resume).unwrap(), b"resume");
}

#[test]
fn busy_startup_cannot_retry_pipeline_before_resume_marker_exists() {
    let root = tempfile::tempdir().unwrap();
    let pending = root.path().join("pipeline-install.pending");
    fs::write(&pending, VERSION).unwrap();
    run_busy_window(root.path(), || panic!("busy startup reconciled"));
    assert_eq!(fs::read_to_string(pending).unwrap(), VERSION);
}

fn assert_cli_recovery_gate(command: &str) {
    let root = tempfile::tempdir().unwrap();
    let args = [command.into()];
    let invoke = || {
        let result = cli_update_at(Some(root.path()), &args, |_| panic!("pending command ran"));
        assert_eq!(result.unwrap().unwrap_err(), UPDATE_RECONCILING);
    };
    let saved = root.path().join("updates.json");
    let bytes = br#"{"phase":"Error","error":"prior task failure"}"#;
    fs::write(&saved, bytes).unwrap();
    let lock = open_hook_lock(root.path()).unwrap().unwrap();
    invoke();
    drop(lock);
    fs::write(root.path().join("update-listener.resume"), b"resume").unwrap();
    invoke();
    assert_eq!(fs::read(saved).unwrap(), bytes);
}

#[test]
fn cli_check_rejects_pending_reconciliation() {
    assert_cli_recovery_gate("--update-check");
}

#[test]
fn cli_apply_rejects_pending_reconciliation() {
    assert_cli_recovery_gate("--update-apply");
}
