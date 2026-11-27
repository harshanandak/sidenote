use super::*;
use PipelineCompletion::Complete;

fn hook(
    root: &Path,
    listener: impl FnOnce(Instant) -> Result<(), String>,
    make_report: impl Fn(&Status) -> serde_json::Value,
) -> Result<bool, String> {
    reconcile_installed(
        root,
        Instant::now(),
        ReconciliationBudget::InstallOrRecovery,
        |_, _| Ok(()),
        |_| Ok(Complete),
        listener,
        make_report,
    )
}

fn assert_startup_busy(path: &Path) {
    let error = start_installed(
        path,
        Instant::now() - Duration::from_secs(2),
        |_, _| panic!("startup reached reconciliation preflight"),
        |_| panic!("startup reached listener preparation"),
        || panic!("startup reached pipeline install"),
        || panic!("startup inspected a pending update"),
        |_| panic!("startup prepared an update report"),
    )
    .unwrap_err();
    assert_eq!(error, InstallAdmission::Busy.message());
}

#[test]
fn startup_busy_does_not_run_preflight_or_persist() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();
    let _guard = InstallGuard::acquire(path).unwrap();
    let files = [
        "install-hook.complete",
        "pipeline-install.pending",
        "update-listener.resume",
        "update-listener.stop",
        "updates.json",
        "update-report.json",
    ];
    for name in files {
        fs::write(path.join(name), name).unwrap();
    }
    assert_startup_busy(path);
    for name in files {
        assert_eq!(fs::read(path.join(name)).unwrap(), name.as_bytes());
    }
    assert_eq!(fs::read_dir(path).unwrap().count(), files.len() + 1);
}

#[test]
fn startup_cannot_install_pipeline_during_hook_record_transition() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();
    let resume = path.join("update-listener.resume");
    let pending = path.join("pipeline-install.pending");
    fs::create_dir(&resume).unwrap();
    let error = hook(
        path,
        |_| panic!("resume write must fail first"),
        |status| {
            // Expose the pending-only state while the hook still owns the guard.
            fs::remove_dir(&resume).unwrap();
            assert_eq!(fs::read(&pending).unwrap(), VERSION.as_bytes());
            let error = start_installed(
                path,
                Instant::now() - Duration::from_secs(2),
                |_, _| panic!("no listener recovery marker"),
                |_| Ok(()),
                || panic!("startup ran the hook's pending pipeline"),
                || false,
                |_| panic!("startup prepared an update report"),
            )
            .unwrap_err();
            assert_eq!(error, InstallAdmission::Busy.message());
            assert_eq!(fs::read(&pending).unwrap(), VERSION.as_bytes());
            assert!(!resume.exists());
            serde_json::json!({"status": status})
        },
    )
    .unwrap_err();
    assert_eq!(error, INSTALL_LISTENER_ERROR);

    let installed = std::cell::Cell::new(false);
    assert!(
        !start_installed(
            path,
            Instant::now(),
            |_, _| panic!("no listener recovery marker"),
            |_| Ok(()),
            || {
                installed.set(true);
                Ok(Complete)
            },
            || false,
            |_| panic!("no pipeline error to report"),
        )
        .unwrap()
    );
    assert!(installed.get());
    assert!(!pending.exists());
}

#[test]
fn reconcile_persist_finishes_before_next_owner_enters() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();
    let publish = |status: &Status| serde_json::json!({"status": status});
    let failure = hook(
        path,
        |_| Err(INSTALL_LISTENER_ERROR.into()),
        |status| {
            assert_eq!(
                read_saved_status(path).error.as_deref(),
                Some(INSTALL_LISTENER_ERROR)
            );
            assert_eq!(
                hook(
                    path,
                    |_| panic!("second owner entered"),
                    |_| panic!("second owner saved")
                )
                .unwrap_err(),
                InstallAdmission::Busy.message(),
            );
            assert!(!path.join("update-report.json").exists());
            publish(status)
        },
    );
    assert_eq!(failure.unwrap_err(), INSTALL_LISTENER_ERROR);
    assert!(hook(path, |_| Ok(()), publish).unwrap());
    assert!(read_saved_status(path).error.is_none());
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("update-report.json")).unwrap()).unwrap();
    assert_eq!(saved["status"]["phase"], "Idle");
    assert!(saved["status"]["error"].is_null());
}

#[test]
fn hook_lock_only_sharing_violation_is_busy() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();
    let guard = InstallGuard::acquire(path).unwrap();
    assert!(matches!(
        InstallGuard::acquire(path),
        Err(InstallAdmission::Busy)
    ));
    drop(guard);
    let not_directory = path.join("install-hook.lock");
    assert!(matches!(
        InstallGuard::acquire(&not_directory),
        Err(InstallAdmission::Io(_))
    ));
    for code in [5, 33] {
        let admission = InstallAdmission::from(io::Error::from_raw_os_error(code));
        assert!(matches!(admission, InstallAdmission::Io(_)));
    }
}

#[test]
fn window_and_cli_apply_wait_for_install_admission_before_handoff() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();
    let _guard = InstallGuard::acquire(path).unwrap();
    for from_window in [true, false] {
        let error = apply_during_hook(path, from_window).unwrap_err();
        assert_eq!(error, InstallAdmission::Busy.message());
        assert!(!path.join("update-listener.resume").exists());
    }
}

fn apply_during_hook(path: &Path, from_window: bool) -> Result<(), String> {
    apply_installed(
        path,
        from_window,
        || panic!("apply inspected the listener while the hook owned recovery"),
        |_| panic!("rejected apply wrote a report"),
        |_| panic!("apply reached handoff while the hook owned recovery"),
    )
}

#[test]
fn pipeline_failure_listener_success_allows_newer_update() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();
    let report = |status: &Status| serde_json::json!(status);
    let error = reconcile_installed(
        path,
        Instant::now(),
        ReconciliationBudget::InstallOrRecovery,
        |_, _| Ok(()),
        |_| Err(PIPELINE_INSTALL_ERROR.into()),
        |_| Ok(()),
        report,
    )
    .unwrap_err();
    assert_eq!(error, PIPELINE_INSTALL_ERROR);
    assert!(install_state::pipeline_pending(path));
    assert!(!path.join("install-hook.complete").exists());
    assert!(!install_state::listener_pending(path));
    apply_installed(
        path,
        false,
        || false,
        report,
        |guard| {
            assert!(!guard.listener_pending());
            assert!(install_state::pipeline_pending(path));
            Ok(())
        },
    )
    .unwrap();

    assert!(
        !start_installed(
            path,
            Instant::now(),
            |_, _| panic!("listener already recovered"),
            |_| Ok(()),
            || Ok(Complete),
            || false,
            report,
        )
        .unwrap()
    );
    assert!(!install_state::pipeline_pending(path));
    assert_eq!(
        install_state::completion_receipt_version(path).as_deref(),
        Some(VERSION)
    );
}

#[test]
fn listener_recovery_failure_blocks_apply_before_handoff() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();
    let report = |status: &Status| serde_json::json!(status);
    assert_eq!(
        hook(path, |_| Err(INSTALL_LISTENER_ERROR.into()), report).unwrap_err(),
        INSTALL_LISTENER_ERROR
    );
    assert!(install_state::listener_pending(path));
    assert!(!install_state::pipeline_pending(path));
    assert!(!path.join("update-listener.stop").exists());

    for from_window in [false, true] {
        assert_eq!(
            apply_installed(
                path,
                from_window,
                || false,
                |_| panic!("rejected apply wrote a report"),
                |_| panic!("apply reached handoff with unfinished listener recovery"),
            )
            .unwrap_err(),
            "Reopen Sidenote to finish restarting its meeting listener, then retry the update."
        );
    }
}

#[test]
fn window_apply_rollback_retires_resume_with_the_same_guard() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();
    let pending = path.join("pipeline-install.pending");
    let resume = path.join("update-listener.resume");
    let stop = path.join("update-listener.stop");
    fs::write(&pending, VERSION).unwrap();

    let result = apply_installed(
        path,
        true,
        || false,
        |status| serde_json::json!(status),
        |guard| {
            guard.mark_listener_pending().unwrap();
            fs::write(&stop, "synthetic stop request").unwrap();
            restart_listener(guard, || {
                assert!(matches!(
                    InstallGuard::acquire(path),
                    Err(InstallAdmission::Busy)
                ));
                assert_eq!(fs::read(&resume).unwrap(), b"resume");
                assert!(!stop.exists());
                Ok(())
            });
            Err("synthetic handoff failure".into())
        },
    );

    assert_eq!(result.as_ref().unwrap_err(), "synthetic handoff failure");
    assert_eq!(read_saved_status(path).error, result.err());
    assert!(!resume.exists());
    assert_eq!(fs::read(&pending).unwrap(), VERSION.as_bytes());
    assert!(InstallGuard::acquire(path).is_ok());
}

#[test]
fn startup_apply_busy_preserves_the_hooks_successful_status_and_report() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();
    let snapshot =
        || ["updates.json", "update-report.json"].map(|name| fs::read(path.join(name)).unwrap());
    let startup = start_installed(
        path,
        Instant::now(),
        |_, _| panic!("no recovery pending"),
        |_| Ok(()),
        || panic!("no pipeline pending"),
        || true,
        |_| panic!("startup wrote a report before apply"),
    )
    .unwrap();
    let mut hook_files = None;
    let result = apply_pending(startup, || {
        let mut rejected = Ok(());
        hook(
            path,
            |_| {
                rejected = apply_during_hook(path, true);
                Ok(())
            },
            |status| serde_json::json!({"hook": status}),
        )
        .unwrap();
        hook_files = Some(snapshot());
        rejected
    });
    assert_eq!(result.unwrap_err(), InstallAdmission::Busy.message());
    assert_eq!(snapshot(), hook_files.unwrap());
}

#[test]
fn corrupt_or_newer_record_never_admits_apply() {
    use serde_json::{Value, json};

    let record = |version: &str, listener: Value, pipeline: Value| {
        json!({
            "schema_version": 1, "app_version": version,
            "listener_work": listener, "pipeline_work": pipeline,
        })
        .to_string()
    };
    let complete = |version: &str| record(version, json!("Complete"), json!("Complete"));
    let current = complete(VERSION);
    let duplicate = current.replace(
        r#""pipeline_work":"Complete""#,
        r#""pipeline_work":"Pending","pipeline_work":"Complete""#,
    );
    let case_duplicate = duplicate.replacen("pipeline_work", "Pipeline_Work", 1);
    let mut cases = vec![
        ("corrupt".into(), Some("{".into()), false, false),
        ("unreadable".into(), Some(current.clone()), true, false),
        ("newer".into(), Some(complete("999.0.0")), false, false),
        ("duplicate".into(), Some(duplicate), false, false),
        (
            "case-only duplicate".into(),
            Some(case_duplicate),
            false,
            false,
        ),
        ("current".into(), Some(current), false, true),
        ("older".into(), Some(complete("0.0.1")), false, true),
        ("absent".into(), None, false, true),
        (
            "not configured".into(),
            Some(record(VERSION, json!("Complete"), json!("NotConfigured"))),
            false,
            true,
        ),
        (
            "object work fields".into(),
            Some(record(
                VERSION,
                json!({"Complete": null}),
                json!({"NotConfigured": null}),
            )),
            false,
            false,
        ),
    ];
    for (shape, listener, pipeline) in [
        (
            "object",
            json!({"Complete": null}),
            json!({"NotConfigured": null}),
        ),
        ("array", json!(["Complete"]), json!(["NotConfigured"])),
        ("number", json!(1), json!(1)),
        ("null", Value::Null, Value::Null),
    ] {
        cases.push((
            format!("listener {shape}"),
            Some(record(VERSION, listener, json!("Complete"))),
            false,
            false,
        ));
        cases.push((
            format!("pipeline {shape}"),
            Some(record(VERSION, json!("Complete"), pipeline)),
            false,
            false,
        ));
    }

    let mut mismatches = Vec::new();
    for (name, bytes, unreadable, should_admit) in cases {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("install-state.json");
        if let Some(bytes) = &bytes {
            fs::write(&path, bytes).unwrap();
        }
        let side_files = [
            "updates.json",
            "update-report.json",
            "install-hook.complete",
            "pipeline-install.pending",
            "update-listener.resume",
        ];
        if name == "corrupt" {
            for file in side_files {
                fs::write(root.path().join(file), file).unwrap();
            }
        }
        let snapshot = || side_files.map(|file| fs::read(root.path().join(file)).ok());
        let before = snapshot();
        let lock = unreadable.then(|| exclusive_at(&path).unwrap());
        let reached_handoff = std::cell::Cell::new(false);
        let result = apply_installed(
            root.path(),
            true,
            || false,
            |_| serde_json::Value::Null,
            |_| {
                reached_handoff.set(true);
                Ok(())
            },
        );
        if result.is_ok() != should_admit || reached_handoff.get() != should_admit {
            mismatches.push(name.clone());
        }
        if name == "corrupt" {
            assert!(result.unwrap_err().contains("install-state.json"));
        }
        drop(lock);
        match bytes {
            Some(bytes) => assert_eq!(fs::read(&path).unwrap(), bytes.as_bytes(), "{name}"),
            None => assert!(!path.try_exists().unwrap(), "{name}"),
        }
        assert_eq!(snapshot(), before, "side files changed for {name}");
    }
    assert!(
        mismatches.is_empty(),
        "wrong apply admission: {mismatches:?}"
    );
}
