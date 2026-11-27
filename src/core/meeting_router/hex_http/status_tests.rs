use super::super::{
    hex::{SpeechChild, collect_child},
    hex_bridge::{HexCommand, HexFailure},
};
use super::*;
use std::{
    io, os::windows::process::ExitStatusExt, process::ExitStatus, sync::atomic::AtomicUsize,
};

struct StatusChild {
    release: Instant,
    exited: bool,
    kills: Arc<AtomicUsize>,
}
impl SpeechChild for StatusChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Ok((self.exited || Instant::now() >= self.release).then(|| ExitStatus::from_raw(0)))
    }
    fn kill(&mut self) -> io::Result<()> {
        self.kills.fetch_add(1, Ordering::Relaxed);
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
}
struct StatusCommand {
    started: mpsc::Sender<()>,
    timed_out: mpsc::Sender<bool>,
    exited: bool,
    kills: Arc<AtomicUsize>,
}
impl HexCommand for StatusCommand {
    fn call(&mut self, args: &[&str], timeout: Duration) -> Result<String, HexFailure> {
        assert_eq!(args, ["status", "--json"]);
        let (stdout, stdout_writer) = io::pipe().unwrap();
        let (stderr, stderr_writer) = io::pipe().unwrap();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(6));
            drop((stdout_writer, stderr_writer));
        });
        let mut child = StatusChild {
            release: Instant::now() + Duration::from_secs(6),
            exited: self.exited,
            kills: Arc::clone(&self.kills),
        };
        self.started.send(()).unwrap();
        let result = collect_child(&mut child, stdout, stderr, timeout, || {});
        let typed = result
            .as_ref()
            .err()
            .and_then(|e| e.downcast_ref::<HexFailure>())
            == Some(&HexFailure::Timeout);
        let _ = self.timed_out.send(typed);
        result.map(|_| String::new()).map_err(|e| {
            e.downcast_ref::<HexFailure>()
                .cloned()
                .unwrap_or(HexFailure::Failed)
        })
    }
}
fn watcher_deadline(exited: bool) {
    let job = Arc::new(ProcessJob::new().unwrap());
    let mut command = Command::new("C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe");
    command.env("SystemRoot", "C:/Windows");
    // Exit normally if the watcher fails to kill the fixture, so the wait has a fallback.
    command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30; exit 0"]);
    let helper = job.spawn(&command).unwrap();
    let (started, rx) = mpsc::channel();
    let (timed_out, result) = mpsc::channel();
    let kills = Arc::new(AtomicUsize::new(0));
    let watch = OwnerWatch::start_command(
        StatusCommand {
            started,
            timed_out,
            exited,
            kills: Arc::clone(&kills),
        },
        &job,
    );
    rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(helper.try_wait().unwrap().is_none());
    let start = Instant::now();
    let active = watch.finish();
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "owner watcher blocked for {elapsed:?}"
    );
    assert!(active);
    // A job kill starts termination asynchronously. The process handle signals completion.
    assert_eq!(
        helper.wait().unwrap().code(),
        Some(1),
        "owner watcher must terminate the private helper"
    );
    assert!(
        result.recv_timeout(Duration::from_millis(100)).unwrap(),
        "adapter must return typed timeout"
    );
    assert_eq!(kills.load(Ordering::Relaxed) > 0, !exited);
}
#[test]
fn status_child_ignoring_termination_cannot_block_owner_helper_cleanup() {
    watcher_deadline(false);
}
#[test]
fn status_child_open_pipes_cannot_block_owner_helper_cleanup() {
    watcher_deadline(true);
}
