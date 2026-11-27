use super::*;

fn hook(
    root: &Path,
    listener: impl FnOnce(Instant) -> Result<(), String>,
    make_report: impl FnOnce(&Status) -> serde_json::Value,
) -> Result<bool, String> {
    reconcile_installed(
        root,
        Instant::now(),
        ReconciliationBudget::InstallOrRecovery,
        |_, _| Ok(()),
        |guard, _| complete_pipeline_install(guard, || Ok(()), |status| serde_json::json!(status)),
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
                Ok(())
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
