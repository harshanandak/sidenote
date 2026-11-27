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
    assert_ne!(
        open_hook_lock(root.path()).unwrap_err().raw_os_error(),
        Some(32)
    );
}

#[test]
fn app_busy_reconcile_preserves_saved_status_and_markers() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("updates.json");
    let bytes = br#"{"phase":"Error","error":"a definite prior failure"}"#;
    fs::write(&path, bytes).unwrap();
    let resume = root.path().join("update-listener.resume");
    fs::write(&resume, b"resume").unwrap();
    let _lock = open_hook_lock(root.path()).unwrap().unwrap();
    let clock = Cell::new(Instant::now());
    let limit = clock.get() + Duration::from_millis(250);
    let wait = |delay| clock.set(clock.get() + delay);
    let held = retry_hook_lock(limit, || clock.get(), wait, || open_hook_lock(root.path()));
    assert!(held.unwrap().is_none());
    assert_eq!(
        save_reconcile_status(Ok(ReconcileOutcome::Busy), root.path(), |_| panic!(
            "busy wrote status"
        ))
        .unwrap(),
        ReconcileOutcome::Busy
    );
    assert_eq!(clock.get(), limit);
    let pending = pending_window_start(root.path()).pending_status.unwrap();
    assert_eq!(pending.phase, Phase::Preparing);
    assert_eq!(pending.error.as_deref(), Some("a definite prior failure"));
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert_eq!(fs::read(resume).unwrap(), b"resume");
}

#[test]
fn app_reconcile_completes_after_lock_release() {
    let root = tempfile::tempdir().unwrap();
    let mut lock = open_hook_lock(root.path()).unwrap();
    let clock = Cell::new(Instant::now());
    let wait = |delay| {
        clock.set(clock.get() + delay);
        drop(lock.take());
    };
    let acquired = retry_hook_lock(
        clock.get() + Duration::from_secs(25),
        || clock.get(),
        wait,
        || open_hook_lock(root.path()),
    )
    .unwrap();
    let pending = root.path().join("pipeline-install.pending");
    let resume = root.path().join("update-listener.resume");
    fs::write(&resume, b"resume").unwrap();
    let result = reconcile_install(
        root.path(),
        || Ok(()),
        |_| Ok(acquired),
        || fs::remove_file(&pending).map_err(|error| error.to_string()),
        || Ok(()),
    );
    assert_eq!(
        result.unwrap(),
        ReconcileOutcome::Complete { changed: true }
    );
    assert!(!resume.exists());
    assert!(!pending.exists());
}
