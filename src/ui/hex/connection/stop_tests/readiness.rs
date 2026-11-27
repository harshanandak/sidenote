use super::*;
use sidenote::updates;

const TASK_FIXTURE: &str = r#"
function New-Object {
    param([string]$ComObject)
    if ($ComObject -ne 'Schedule.Service') { throw 'Unexpected COM request' }
    $service = [PSCustomObject]@{}
    $service | Add-Member ScriptMethod Connect {}
    $service | Add-Member ScriptMethod GetFolder { $this }
    $service | Add-Member ScriptMethod GetTask { $this }
    $service | Add-Member ScriptMethod GetSecurityDescriptor {
        'O:' + [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    }
    $service
}
function Get-ScheduledTask {
    $root = $env:SIDENOTE_RECORDER_READY_TEST
    [IO.File]::WriteAllText((Join-Path $root 'validation-entered'), '')
    $until = [DateTime]::UtcNow.AddSeconds(5)
    while (!(Test-Path (Join-Path $root 'validation-release'))) {
        if ([DateTime]::UtcNow -gt $until) { throw 'Fixture validation stalled' }
        Start-Sleep -Milliseconds 10
    }
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $execute = if (Test-Path (Join-Path $root 'mismatch')) { 'wrong.exe' } else { $env:SIDENOTE_TASK_EXE }
    [PSCustomObject]@{
        TaskName = $env:SIDENOTE_TASK_NAME
        Actions = @([PSCustomObject]@{ Execute = $execute; Arguments = $env:SIDENOTE_TASK_ARGS })
        Principal = [PSCustomObject]@{ UserId = $sid; LogonType = 'Interactive'; RunLevel = 'Limited' }
        Settings = [PSCustomObject]@{ Enabled = $true; MultipleInstances = 'IgnoreNew'; ExecutionTimeLimit = 'PT0S' }
        Triggers = @(
            [PSCustomObject]@{ CimClass = [PSCustomObject]@{ CimClassName = 'MSFT_TaskLogonTrigger' }; Enabled = $true; UserId = $sid },
            [PSCustomObject]@{ CimClass = [PSCustomObject]@{ CimClassName = 'MSFT_TaskTimeTrigger' }; Enabled = $true; Repetition = [PSCustomObject]@{ Interval = 'PT5M'; Duration = '' } }
        )
    }
}
function Register-ScheduledTask { throw 'Registration forbidden' }
function Set-ScheduledTask { throw 'Migration forbidden' }
function Start-ScheduledTask { throw 'Listener start forbidden' }
"#;

struct ValidationWorker {
    root: std::path::PathBuf,
    worker: Option<thread::JoinHandle<()>>,
}
impl Drop for ValidationWorker {
    fn drop(&mut self) {
        std::fs::write(self.root.join("validation-release"), []).unwrap();
        self.worker.take().unwrap().join().unwrap();
    }
}

pub(super) fn run() {
    crate::test_paths::isolate();
    assert!(updates::installed());
    let root = std::path::PathBuf::from(std::env::var_os("SIDENOTE_RECORDER_READY_TEST").unwrap());
    std::fs::write(root.join("task-fixture.ps1"), TASK_FIXTURE).unwrap();
    let listener = updates::listener_lock().unwrap().unwrap();
    let (api, state, _) = Fixture::start_idle();
    let mut runtime =
        rune::Runtime::new(state, meeting_control, (500.0, 160.0), rune::Env::default());
    runtime.set_full_keyboard_access(true);
    runtime.frame(0.0);
    runtime.update(|state, _| {
        let sample = command_status_sample(state);
        state.hex_apply(Msg::ObservedStatus(sample));
    });
    let mut worker = None;
    runtime.update(|state, _| worker = state.retry_listener());
    let worker = worker.expect("real retry worker");
    let worker = ValidationWorker {
        root: root.clone(),
        worker: Some(worker),
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.join("validation-entered").exists() {
        assert!(Instant::now() < deadline, "validation helper did not enter");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(updates::probe_listener().is_none());
    assert!(!runtime.state.hex.connection.slow(&runtime.state.hex.status));
    assert!(!runtime.state.hex.connection.expired());
    assert!(runtime.state.hex.can(Feature::RecordStart));
    assert!(!runtime.state.hex.caps_loading);
    assert!(!runtime.state.hex_recording());
    assert!(!runtime.state.hex_record_busy());
    runtime.frame(0.0);
    let control = runtime
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("new-meeting"))
        .unwrap();
    runtime.update(|state, _| state.new_meeting());
    assert!(
        !runtime.state.hex_record_busy(),
        "listener preparation must refuse the action before scheduling a recording"
    );
    assert_eq!(api.data.lock().unwrap().starts, 0);
    assert_eq!(
        control.label.as_deref(),
        Some("Record a meeting (Checking the meeting listener)")
    );
    assert!(!control.focusable);
    std::fs::write(root.join("validation-release"), []).unwrap();
    let event = runtime
        .state
        .events_rx
        .recv_timeout(Duration::from_secs(15))
        .unwrap();
    runtime.update(|state, _| state.apply_event(event));
    drop(worker);
    let probe = updates::probe_listener().unwrap();
    assert!(probe.ready);
    runtime.update(|state, _| {
        state.apply_event(Event::ListenerProbed(Some(probe), None));
        let sample = command_status_sample(state);
        state.hex_apply(Msg::ObservedStatus(sample));
    });
    runtime.frame(0.0);
    let control = runtime
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("new-meeting"))
        .unwrap();
    assert_eq!(control.label.as_deref(), Some("Record a meeting"));
    assert!(control.focusable);
    assert_eq!(api.data.lock().unwrap().starts, 0);
    std::fs::write(root.join("mismatch"), []).unwrap();
    let failed = updates::ensure_listener();
    assert!(failed.is_err());
    runtime.update(|state, _| state.apply_event(Event::ListenerFinished(failed)));
    assert_eq!(
        crate::ui::hex::start_blocked(&runtime.state),
        Some("Recording is unavailable. Restart Sidenote to retry.")
    );
    runtime.update(|state, _| state.new_meeting());
    assert_eq!(api.data.lock().unwrap().starts, 0);
    drop(listener);
    assert!(!updates::probe_listener().unwrap().ready);
    let directory = updates::directory();
    let saved = root.join("saved-updates");
    std::fs::rename(&directory, &saved).unwrap();
    std::fs::write(&directory, []).unwrap();
    let failed_check = updates::probe_listener().unwrap();
    std::fs::remove_file(&directory).unwrap();
    std::fs::rename(saved, directory).unwrap();
    assert!(!failed_check.ready);
    let log = std::fs::read_to_string(sidenote::core::log_path()).unwrap();
    for reason in [
        "preparing",
        "ready",
        "task_validation_failed",
        "lock_missing",
        "lock_check_failed",
    ] {
        assert!(log.contains(&format!(
            "listener readiness reason={reason} role=ui pid={}",
            std::process::id()
        )));
    }
}
