use std::{
    io,
    sync::{
        Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

#[derive(Clone, Copy)]
pub struct Stage(&'static str, u8);
pub const PROCESS_START: Stage = Stage("process_start", 1);
pub const WINDOW_SHOWN: Stage = Stage("window_shown", 2);
pub const FIRST_PAINT: Stage = Stage("first_paint", 4);
pub const DATA_LOADED: Stage = Stage("data_loaded", 8);
pub const LISTENER_READY: Stage = Stage("listener_ready", 16);
const STAGE_COUNT: usize = 5;
// Submit under one lock to preserve order across callers. The writer never takes this lock.
static LOG: Mutex<Option<Log>> = Mutex::new(None);

struct Log {
    started: Instant,
    seen: u8,
    lines: SyncSender<String>,
}

pub fn start(started: Instant) {
    let mut log = LOG.lock().unwrap();
    log.get_or_insert_with(|| {
        let (log, lines) = Log::new(started);
        let _ = spawn_writer(lines, crate::listener::log_process);
        log
    })
    .submit(PROCESS_START, started);
}

pub fn observe(stage: Stage, at: Instant) {
    if let Some(log) = LOG.lock().unwrap().as_mut() {
        log.submit(stage, at);
    }
}

impl Log {
    fn new(started: Instant) -> (Self, Receiver<String>) {
        let (lines, receiver) = mpsc::sync_channel(STAGE_COUNT);
        (
            Self {
                started,
                seen: 0,
                lines,
            },
            receiver,
        )
    }

    fn submit(&mut self, Stage(name, bit): Stage, at: Instant) {
        if self.seen & bit == 0 {
            self.seen |= bit;
            let elapsed = at.saturating_duration_since(self.started).as_millis();
            let _ = self
                .lines
                .try_send(format!("startup stage={name} elapsed_ms={elapsed}"));
        }
    }
}

fn spawn_writer(
    lines: Receiver<String>,
    mut append: impl FnMut(&str) + Send + 'static,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("startup-log".into())
        .spawn(move || {
            for line in lines {
                append(&line);
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::Duration};

    #[test]
    fn startup_log_writes_each_stage_once_in_order_with_only_timing_fields() {
        let started = Instant::now();
        let events = [
            (PROCESS_START, "process_start"),
            (LISTENER_READY, "listener_ready"),
            (WINDOW_SHOWN, "window_shown"),
            (FIRST_PAINT, "first_paint"),
            (DATA_LOADED, "data_loaded"),
        ];
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("sidenote.log");
        let (mut log, lines) = Log::new(started);
        let (appended, wait) = mpsc::channel();
        let writer_path = path.clone();
        let writer = spawn_writer(lines, move |line| {
            crate::listener::log_at(&writer_path, line);
            let _ = appended.send(());
        })
        .unwrap();
        let mut expected = Vec::new();
        for (ms, (stage, label)) in events.into_iter().enumerate() {
            log.submit(stage, started + Duration::from_millis(ms as u64));
            log.submit(stage, started);
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            expected.push(format!("startup stage={label} elapsed_ms={ms}"));
            let file = fs::read_to_string(&path).unwrap();
            let stages = file.lines().map(|line| {
                let (timestamp, message) = line.split_once(" listener: ").unwrap();
                chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S").unwrap();
                message
            });
            assert!(stages.eq(expected.iter().map(String::as_str)), "{file}");
        }
        drop(log);
        writer.join().unwrap();
        assert!(wait.try_recv().is_err(), "each stage appends once");
    }
}
