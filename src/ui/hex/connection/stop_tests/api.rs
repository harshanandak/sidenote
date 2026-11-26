use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
};

pub(super) struct Data {
    pub id: Option<&'static str>,
    pub omit_id: bool,
    pub meeting_state: Option<&'static str>,
    pub status_timeout: bool,
    pub stop_changed: bool,
    pub stop_busy: bool,
    pub stop_state: Option<&'static str>,
    pub commands: Vec<String>,
    pub delay: Duration,
    pub stops: usize,
    stop_in_flight: bool,
    stop_gate: Option<(Sender<()>, Receiver<()>)>,
    pub starts: usize,
    pub start_failure: bool,
    pub start_records: bool,
    pub start_release: Option<std::sync::mpsc::Receiver<()>>,
}
pub(super) struct Fixture {
    pub data: Arc<Mutex<Data>>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Fixture {
    pub fn start() -> (Self, State, Instant) {
        Self::with_recording(Some("100-1"))
    }
    pub fn start_idle() -> (Self, State, Instant) {
        Self::with_recording(None)
    }
    fn with_recording(id: Option<&'static str>) -> (Self, State, Instant) {
        // Discovery and worker paths are process-wide, so each API test needs its own process.
        let current = thread::current();
        assert_eq!(
            std::env::var("SIDENOTE_HEX_API_TEST").ok().as_deref(),
            current
                .name()
                .and_then(|name| { name.strip_prefix("ui::hex::connection::stop_tests::") }),
            "use isolated_api_test before starting a loopback Hex API"
        );
        let mut state = State::new();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let data = Arc::new(Mutex::new(Data {
            id,
            omit_id: false,
            meeting_state: None,
            status_timeout: false,
            stop_changed: true,
            stop_busy: false,
            stop_state: None,
            commands: Vec::new(),
            delay: Duration::ZERO,
            stops: 0,
            stop_in_flight: false,
            stop_gate: None,
            starts: 0,
            start_failure: false,
            start_records: true,
            start_release: None,
        }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let shared = data.clone();
        let stopping = shutdown.clone();
        let worker = thread::spawn(move || {
            let mut clients = Vec::new();
            while !stopping.load(Ordering::Relaxed) {
                let Ok((stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let shared = shared.clone();
                clients.push(thread::spawn(move || {
                // Only the accept loop is nonblocking; a client must wait for its HTTP request.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some((key, value)) = line.split_once(':')
                        && key.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                if length == 0 { return; }
                assert!(length < 4096);
                let mut body = vec![0; length];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                let request: Value = serde_json::from_slice(&body).unwrap();
                shared.lock().unwrap().commands.push(request["command"].as_str().unwrap().into());
                let reply = match request["command"].as_str().unwrap() {
                    "status" => {
                        if shared.lock().unwrap().status_timeout {
                            let _ = reader.get_mut().write_all(b"HTTP/1.1 504 Gateway Timeout\r\nContent-Length: 0\r\n\r\n");
                            return;
                        }
                        let delay = shared.lock().unwrap().delay;
                        thread::sleep(delay);
                        let data = shared.lock().unwrap();
                        let mut reply = serde_json::json!({"version":4,"status":"status","app_version":"test",
                            "model":{},"capture":{"state":"idle"},"capabilities":{"meetings":true},
                            "features":{"meetings":true},
                            "meeting":{"state":data.meeting_state.unwrap_or(if data.id.is_some() {"recording"} else {"idle"}),"id":data.id,"elapsed_seconds":1}});
                        if data.omit_id { reply["meeting"].as_object_mut().unwrap().remove("id"); }
                        reply
                    }
                    "meeting-record" => {
                        let release = {
                            let mut data = shared.lock().unwrap();
                            data.starts += 1;
                            data.start_release.take()
                        };
                        if let Some(release) = release {
                            release.recv_timeout(Duration::from_secs(5)).unwrap();
                        }
                        let mut data = shared.lock().unwrap();
                        if data.start_failure {
                            serde_json::json!({"version":4,"status":"error","code":"failed"})
                        } else {
                            if data.start_records { data.id = Some("300-3"); }
                            serde_json::json!({"version":4,"status":"done","changed":true})
                        }
                    }
                    "meeting-stop" => {
                        let (gate, reply) = {
                            let mut data = shared.lock().unwrap();
                            data.stops += 1;
                            assert!(!data.stop_in_flight, "overlapping Hex Stop commands");
                            data.stop_in_flight = true;
                            let reply = if data.stop_busy {
                                serde_json::json!({"version":4,"status":"error","code":"busy"})
                            } else {
                                data.id = None;
                                data.meeting_state = data.stop_state;
                                serde_json::json!({"version":4,"status":"done","changed":data.stop_changed})
                            };
                            (data.stop_gate.take(), reply)
                        };
                        if let Some((received, release)) = gate {
                            let _ = received.send(());
                            let _ = release.recv_timeout(Duration::from_secs(5));
                        }
                        shared.lock().unwrap().stop_in_flight = false;
                        reply
                    }
                    other => panic!("unexpected fake API command: {other}"),
                }.to_string();
                let _ = write!(
                    reader.get_mut(),
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.len(),
                    reply
                );
                }));
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        let dir = sidenote::core::meetings_dir()
            .parent()
            .unwrap()
            .to_path_buf();
        assert!(dir.starts_with(env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&dir).unwrap();
        // Synthetic discovery only, inside the test's isolated meetings parent.
        std::fs::write(
            dir.join("local-api.json"),
            serde_json::json!({"port":port,"token":"fixture"}).to_string(),
        )
        .unwrap();
        state.hex_apply(Msg::Caps(
            "test".into(),
            Ok(Capabilities::from([
                (Feature::RecordStart, Support::Supported),
                (Feature::RecordStop, Support::Supported),
            ])),
        ));
        let now = Instant::now() + Duration::from_secs(3600);
        let good = state.hex.engine.status_for_command().unwrap();
        state.hex_status_at(Ok(good), now);
        state.hex_status_at(Err(HexError::Timeout), now + Duration::from_secs(2));
        (
            Self {
                data,
                shutdown,
                worker: Some(worker),
            },
            state,
            now,
        )
    }
    pub fn hold_stop(&self) -> (Receiver<()>, Sender<()>) {
        let (received, arrived) = mpsc::channel();
        let (resume, release) = mpsc::channel();
        self.data.lock().unwrap().stop_gate = Some((received, release));
        (arrived, resume)
    }

    pub fn assert_stops(&self, expected: usize) {
        let data = self.data.lock().unwrap();
        let commands = data
            .commands
            .iter()
            .filter(|command| command.as_str() == "meeting-stop")
            .count();
        assert_eq!(commands, expected, "Hex commands: {:?}", data.commands);
        assert_eq!(data.stops, expected);
    }

    pub fn stop(state: &mut State) {
        state.hex_record_stop(recording_id(state));
        Self::finish(state);
    }
    pub fn finish(state: &mut State) {
        while state.hex.record_busy {
            let event = state
                .events_rx
                .recv_timeout(Duration::from_secs(15))
                .expect("Stop completion");
            if let Event::Hex(msg) = event {
                state.hex_apply(msg);
            }
        }
    }
    pub fn stop_status(state: &mut State) {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let event = state
                .events_rx
                .recv_timeout(until.saturating_duration_since(Instant::now()))
                .expect("Stop status");
            if let Event::Hex(msg @ Msg::StopStatus(..)) = event {
                state.hex_apply(msg);
                return;
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
