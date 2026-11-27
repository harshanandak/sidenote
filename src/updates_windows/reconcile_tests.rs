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
        |guard, _| complete_pipeline_install_at(guard.directory, || Ok(())),
        listener,
        make_report,
    )
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
