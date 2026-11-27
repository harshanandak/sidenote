use super::*;

const FIXTURE: &str = r#"
function New-HandoffFixture {
    $script:action = [PSCustomObject]@{ Type=0; Path='C:\fixture\current\sidenote.exe'; Arguments='--listen'; WorkingDirectory='C:\fixture\current' }
    $script:settings = [PSCustomObject]@{ Enabled=$true }
    $actions = [PSCustomObject]@{ Count=1 }
    $actions | Add-Member ScriptMethod Item { $script:action }
    $definition = [PSCustomObject]@{ Actions=$actions; Settings=$script:settings }
    $definition | Add-Member ScriptProperty XmlText { "<task enabled='$($this.Settings.Enabled)' exe='$($script:action.Path)' directory='$($script:action.WorkingDirectory)'><principal/><triggers/><settings/></task>" } {}
    $registered = [PSCustomObject]@{ Definition=$definition; State=4 }
    $registered | Add-Member ScriptProperty Enabled { $script:settings.Enabled } { $script:settings.Enabled = $args[0] }
    $registered | Add-Member ScriptMethod GetSecurityDescriptor { 'O:' + [Security.Principal.WindowsIdentity]::GetCurrent().User.Value }
    $service = [PSCustomObject]@{ Task=$registered }
    $service | Add-Member ScriptMethod Connect {}
    $service | Add-Member ScriptMethod GetFolder { $this }
    $service | Add-Member ScriptMethod GetTask { $this.Task }
    $service | Add-Member ScriptMethod NewTask { $this.Task.Definition }
    $service | Add-Member ScriptMethod RegisterTask { throw 'Unexpected registration' }
    $service
}
function Read-HandoffFixtureTask {
    $action = [PSCustomObject]@{ Execute=$script:action.Path; Arguments=$script:action.Arguments; WorkingDirectory=$script:action.WorkingDirectory }
    $principal = [PSCustomObject]@{ UserId=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value; LogonType='Interactive'; RunLevel='Limited' }
    [PSCustomObject]@{ Actions=@($action); Principal=$principal; Settings=$script:settings }
}
"#;

fn task_request(path: &Path, request: serde_json::Value) -> Result<(), ListenerTaskError> {
    fs::write(path, request.to_string()).unwrap();
    run_listener_task(
        windows_powershell().env("SIDENOTE_HANDOFF_REQUEST", path),
        &format!(
            "{}\n{FIXTURE}\n{SCRIPT}\n{LISTENER_TASK_SCRIPT}",
            super::super::tests::MOCK_LISTENER_TASK
        ),
        ListenerTaskMode::Handoff,
        "Sidenote mock listener",
        Path::new(r"C:\target\current\sidenote.exe"),
        "--listen",
    )
}

fn journal(root: &Path) -> Journal {
    let request = root.join("request.json");
    task_request(&request, serde_json::json!({})).unwrap();
    Journal {
        schema: 1,
        task: "Sidenote mock listener".into(),
        target: r"C:\target\current\sidenote.exe".into(),
        version: VERSION.into(),
        plan: serde_json::from_slice(&fs::read(request).unwrap()).unwrap(),
        phase: Phase::Disable,
    }
}

#[test]
fn snapshot_and_journal_preserve_the_reverse_path() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = journal(root.path());
    let prior = &journal.plan.prior;
    assert_eq!(prior.exe, Path::new(r"C:\fixture\current\sidenote.exe"));
    assert_eq!(prior.args, "--listen");
    assert_eq!(prior.working_dir, r"C:\fixture\current");
    assert!(prior.running && prior.enabled);
    assert!(
        prior
            .xml
            .ends_with("<principal/><triggers/><settings/></task>")
    );
    assert!(journal.publish(&root.path().join("missing")).is_err());
    let first = journal.publish(root.path()).unwrap();
    let second = journal.publish(root.path()).unwrap();
    assert_ne!(first, second);
    journal.persist_phase(&first, Phase::Switch).unwrap();
    let read = |path| serde_json::from_slice::<Journal>(&fs::read(path).unwrap()).unwrap();
    assert_eq!(read(&first).phase, Phase::Switch);
    assert_eq!(read(&second).phase, Phase::Disable);
    assert_eq!(read(&first).plan, journal.plan);
}

#[test]
fn recovery_matches_the_recorded_target_phase_and_security() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = journal(root.path());
    assert!(journal.belongs_to(&journal.task, &journal.target));
    assert!(!journal.belongs_to(&journal.task, &journal.plan.prior.exe));
    let mut current: Snapshot =
        serde_json::from_value(serde_json::to_value(&journal.plan.prior).unwrap()).unwrap();
    for (phase, permitted) in [
        (
            Phase::Disable,
            &[Definition::Prior, Definition::Disabled][..],
        ),
        (
            Phase::Switch,
            &[Definition::Disabled, Definition::Switched][..],
        ),
        (
            Phase::Enable,
            &[Definition::Switched, Definition::Enabled][..],
        ),
        (Phase::Ready, &[Definition::Enabled][..]),
        (
            Phase::Restore(Definition::Switched),
            &[Definition::Switched, Definition::Prior][..],
        ),
        (Phase::Restored, &[][..]),
        (Phase::Complete, &[][..]),
    ] {
        journal.phase = phase;
        for definition in [
            Definition::Prior,
            Definition::Disabled,
            Definition::Switched,
            Definition::Enabled,
        ] {
            current.xml = journal.plan.xml(definition).into();
            assert_eq!(
                journal.restore_from(&current).ok(),
                permitted.contains(&definition).then_some(definition)
            );
        }
    }
    journal.phase = Phase::Disable;
    current.xml.clone_from(&journal.plan.prior.xml);
    current.security.push('!');
    assert!(matches!(
        journal.restore_from(&current),
        Err(ListenerTaskError::TaskChanged)
    ));
    assert!(matches!(
        task_request(
            &root.path().join("request.json"),
            serde_json::json!({
                "expected": "external change", "security": journal.plan.prior.security
            })
        ),
        Err(ListenerTaskError::TaskChanged)
    ));
}
