use super::*;
use std::cell::Cell;
fn start_fixture(
    dir: &Path,
    reconcile: impl FnOnce() -> Result<ReconcileOutcome, String>,
    work: impl Fn() -> Result<(), String> + Copy,
) -> ReconcileOutcome {
    let clock = Cell::new(Instant::now());
    window_start_at(
        dir,
        (|| clock.get(), |delay| clock.set(clock.get() + delay)),
        reconcile,
        work,
        work,
        |_| work().map_err(io::Error::other),
        work,
    )
    .unwrap()
}
fn run_busy_window(dir: &Path, reconcile: impl FnOnce() -> Result<ReconcileOutcome, String>) {
    let _lock = open_hook_lock(dir).unwrap().unwrap();
    assert!(open_hook_lock(dir).unwrap().is_none());
    let unexpected = || panic!("startup did work while the hook held the lock");
    assert_eq!(
        start_fixture(dir, reconcile, unexpected),
        ReconcileOutcome::Busy
    );
}

#[test]
fn startup_recovers_a_clean_profile_and_waits_for_a_held_hook() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let lock = dir.join("install-hook.lock");
    fs::create_dir(&lock).unwrap();
    assert_ne!(open_hook_lock(dir).unwrap_err().raw_os_error(), Some(32));
    fs::remove_dir(lock).unwrap();
    let receipt = dir.join("install-hook.complete");
    let reconcile = || {
        fs::write(&receipt, VERSION).unwrap();
        Ok(ReconcileOutcome::Complete { changed: true })
    };
    start_fixture(dir, reconcile, || Ok(()));
    assert_eq!(fs::read_to_string(&receipt).unwrap(), VERSION);
    fs::remove_file(receipt).unwrap();
    let pending = dir.join("pipeline-install.pending");
    fs::write(&pending, VERSION).unwrap();
    run_busy_window(dir, || panic!("busy startup reconciled"));
    assert_eq!(fs::read_to_string(&pending).unwrap(), VERSION);
    let saved = dir.join("updates.json");
    let bytes = br#"{"phase":"Error","error":"prior task failure"}"#;
    let resume = dir.join("update-listener.resume");
    fs::write(&saved, bytes).unwrap();
    fs::write(&resume, b"resume").unwrap();
    run_busy_window(dir, || Err(ListenerTaskError::TimedOut.message()));
    assert_eq!(fs::read(saved).unwrap(), bytes);
    assert_eq!(fs::read(resume).unwrap(), b"resume");
}

fn assert_cli_recovery_gate(command: &str) {
    let root = tempfile::tempdir().unwrap();
    let invoke =
        || cli_update_at(Some(root.path()), || panic!("pending {command} ran")).unwrap_err();
    let saved = root.path().join("updates.json");
    let bytes = br#"{"phase":"Error","error":"prior task failure"}"#;
    fs::write(&saved, bytes).unwrap();
    let lock = open_hook_lock(root.path()).unwrap().unwrap();
    let error = invoke();
    assert_eq!(error, UPDATE_RECONCILING);
    if command == "--update-check" {
        let mut published = Status::default();
        publish_check_failure(Err(error), |status| published = status);
        assert_eq!(published.phase, Phase::Error);
        assert_eq!(published.error.as_deref(), Some(UPDATE_RECONCILING));
    }
    drop(lock);
    fs::write(root.path().join("update-listener.resume"), b"resume").unwrap();
    assert_eq!(invoke(), UPDATE_RECONCILING);
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

#[test]
fn missing_pipeline_settings_keep_retry_marker_and_allow_updates() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    // Configured scripts without private settings exit 1.
    let pipeline = || pipeline_script_result(Some(1));
    let result = reconcile_install(dir, || Ok(()), pipeline, || Ok(()));
    assert_eq!(result.unwrap_err(), PIPELINE_INSTALL_ERROR);
    assert!(dir.join("pipeline-install.pending").exists());
    assert!(!dir.join("update-listener.resume").exists());
    assert!(update_operation_lock(dir, true).is_ok());
}
#[test]
fn successful_window_recovery_clears_its_saved_lock_error() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path();
    let lock_path = directory.join("install-hook.lock");
    let status_file = directory.join("updates.json");
    fs::create_dir(&lock_path).unwrap();
    let resume = directory.join("update-listener.resume");
    fs::write(&resume, b"resume").unwrap();
    let start = || {
        window_start_at(
            directory,
            (Instant::now, |_| panic!("a lock I/O failure was retried")),
            || {
                fs::remove_file(&resume).unwrap();
                Ok(ReconcileOutcome::Complete { changed: true })
            },
            || Ok(()),
            || Ok(()),
            |status| fs::write(&status_file, serde_json::to_vec(status)?),
            || Ok(()),
        )
    };
    assert!(start().unwrap_err().contains("install hook lock"));
    assert_eq!(read_status(directory).phase, Phase::Error);
    fs::remove_dir(lock_path).unwrap();
    start().unwrap();
    let status = read_status(directory);
    assert_eq!(status.phase, Phase::Idle);
    assert!(status.error.is_none());
}
#[test]
fn deferred_startup_retries_after_cli_check_releases_hook_lock() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path();
    fs::write(directory.join("install-hook.complete"), VERSION).unwrap();
    let mut holder = Some(update_operation_lock(directory, true).unwrap());
    let deferred = start_fixture(directory, || panic!("reconciled"), || panic!("started"));
    assert_eq!(deferred, ReconcileOutcome::Busy);
    let now = Instant::now();
    let assert_held = || {
        assert!(open_hook_lock(directory).unwrap().is_none());
        Ok(())
    };
    let wait = |_| drop(holder.take());
    let start = window_start_at(
        directory,
        (|| now, wait),
        || panic!("reconciled without a resume marker"),
        assert_held,
        assert_held,
        |_| panic!("idle startup wrote status"),
        || {
            assert!(open_hook_lock(directory).unwrap().is_some());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(start, ReconcileOutcome::Complete { changed: false });
}
