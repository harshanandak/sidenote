use super::*;
use std::cell::Cell;

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
    let started = Instant::now();
    let clock = Cell::new(started);
    let outcome = window_start_at(
        directory,
        (|| clock.get(), |wait| clock.set(clock.get() + wait)),
        reconcile,
        || panic!("busy startup checked the listener"),
        || panic!("busy startup retried the pipeline"),
        |_| panic!("busy startup persisted status"),
        || panic!("busy startup applied an update"),
    );
    assert_eq!(outcome.unwrap(), ReconcileOutcome::Busy);
    assert_eq!(clock.get() - started, Duration::from_secs(2));
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
fn app_start_retries_held_lock_before_preparing_listener_and_pipeline() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path();
    let mut holder = open_hook_lock(directory).unwrap();
    let clock = Cell::new(Instant::now());
    let assert_held = || {
        assert!(open_hook_lock(directory).unwrap().is_none());
        Ok(())
    };
    let start = window_start_at(
        directory,
        (
            || clock.get(),
            |wait| {
                clock.set(clock.get() + wait);
                drop(holder.take());
            },
        ),
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
    assert!(holder.is_none());
    assert_eq!(start, ReconcileOutcome::Complete { changed: false });
}
