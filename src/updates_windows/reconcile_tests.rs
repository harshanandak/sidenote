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
    fs::write(&receipt, "previous version").unwrap();
    start_fixture(dir, reconcile, || Ok(()));
    assert_eq!(fs::read_to_string(&receipt).unwrap(), VERSION);
    fs::remove_file(receipt).unwrap();
    let _lock = open_hook_lock(dir).unwrap().unwrap();
    assert!(open_hook_lock(dir).unwrap().is_none());
    let defer = |reconcile: fn() -> Result<ReconcileOutcome, String>| {
        let outcome = start_fixture(dir, reconcile, || panic!("busy startup worked"));
        assert_eq!(outcome, ReconcileOutcome::Busy);
    };
    let pending = dir.join("pipeline-install.pending");
    fs::write(&pending, VERSION).unwrap();
    defer(|| panic!("busy startup reconciled"));
    assert_eq!(fs::read_to_string(&pending).unwrap(), VERSION);
    let saved = dir.join("updates.json");
    let bytes = br#"{"phase":"Error","error":"prior task failure"}"#;
    let resume = dir.join("update-listener.resume");
    fs::write(&saved, bytes).unwrap();
    fs::write(&resume, b"resume").unwrap();
    defer(|| Err(ListenerTaskError::TimedOut.message()));
    assert_eq!(fs::read(saved).unwrap(), bytes);
    assert_eq!(fs::read(resume).unwrap(), b"resume");
}

#[test]
fn cli_admission_rejects_pending_reconciliation_and_ends_the_check() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let saved = dir.join("updates.json");
    let bytes = br#"{"phase":"Error","error":"prior task failure"}"#;
    fs::write(&saved, bytes).unwrap();
    let lock = open_hook_lock(dir).unwrap().unwrap();
    let error = update_operation_lock(dir, true).unwrap_err();
    let mut published = Status::default();
    publish_check_failure(Err(error), |status| published = status);
    assert_eq!(published.phase, Phase::Error);
    assert_eq!(published.error.as_deref(), Some(UPDATE_RECONCILING));
    drop(lock);
    fs::write(dir.join("update-listener.resume"), b"resume").unwrap();
    assert_eq!(
        update_operation_lock(dir, true).unwrap_err(),
        UPDATE_RECONCILING
    );
    assert_eq!(fs::read(saved).unwrap(), bytes);
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
    fs::write(dir.join("update-listener.resume"), b"resume").unwrap();
    start_fixture(dir, || Err(PIPELINE_INSTALL_ERROR.into()), || Ok(()));
    assert!(dir.join("pipeline-install.pending").exists());
    assert!(update_operation_lock(dir, true).is_ok());
}
#[test]
fn successful_window_recovery_clears_its_saved_task_error() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path();
    let resume = directory.join("update-listener.resume");
    fs::write(&resume, b"resume").unwrap();
    let persist =
        |status: &Status| fs::write(directory.join("updates.json"), serde_json::to_vec(status)?);
    let start = |result: Result<bool, String>| {
        window_start_at(
            directory,
            (Instant::now, |_| panic!("the hook lock was held")),
            || save_reconcile_status(result, directory, persist),
            || Ok(()),
            || Ok(()),
            persist,
            || Ok(()),
        )
    };
    let failure = ListenerTaskError::TimedOut.message();
    assert_eq!(start(Err(failure.clone())).unwrap_err(), failure);
    assert_eq!(read_status(directory).phase, Phase::Error);
    start(Ok(true)).unwrap();
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
