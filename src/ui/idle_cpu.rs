//! No native window or GPU: the production view, poll and background status workers.
use super::*;
use rune::{Env, Runtime, text::TextSystem};
use sidenote::hex_engine::sample_status;
#[cfg(windows)]
mod measurement;

#[cfg(all(windows, feature = "idle-cpu-proof"))]
pub(super) fn run_proof() -> bool {
    if std::env::args().any(|a| a == "--measure") {
        measurement::idle_cpu_real_thirty_seconds();
        return true;
    }
    let checks: [(&str, fn()); 4] = [
        ("delayed empty poll", idle_cpu_delayed_empty_poll_sleeps),
        (
            "Hex state and meeting progress",
            idle_cpu_hex_state_changes_reach_the_view,
        ),
        ("simulated 30 s idle", idle_cpu_simulated_thirty_seconds),
        (
            "unchanged refresh",
            idle_cpu_unchanged_refresh_does_not_rebuild,
        ),
    ];
    let mut failed = 0;
    for (name, run) in checks {
        let passed = std::panic::catch_unwind(run).is_ok();
        println!("{name}: {}", if passed { "PASS" } else { "FAIL" });
        failed += usize::from(!passed);
    }
    println!("idle CPU proof: {} passed; {failed} failed", 4 - failed);
    failed == 0
}

// Each fixture owns its paths and fake Hex discovery for its whole worker lifetime.
#[cfg(test)]
fn isolated_test() -> bool {
    const FLAG: &str = "SIDENOTE_IDLE_UI_TEST";
    let thread = std::thread::current();
    let name = thread.name().unwrap();
    if std::env::var(FLAG).ok().as_deref() == Some(name) {
        return false;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture"])
        .env(FLAG, name);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "isolated idle UI test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
    true
}

fn runtime() -> Runtime<State> {
    #[cfg(test)]
    crate::test_paths::isolate();
    #[cfg(not(test))]
    for path in [core::state_dir(), core::meetings_dir(), core::peers_dir()] {
        assert!(core::is_proof_path(&path, "idle-cpu-proof"));
    }
    let config = core::state_dir().join("config.json");
    if !config.exists() {
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            br#"{"version":1,"features":{},"notes":{"engine":"app","helper":null}}"#,
        )
        .unwrap();
    }
    for index in 0..300 {
        let folder = core::meetings_dir().join(format!("1800000000000-{index}"));
        std::fs::create_dir_all(&folder).unwrap();
        #[cfg(feature = "idle-cpu-proof")]
        if std::env::args().any(|a| a == "--retained-done") {
            std::fs::write(
                folder.join("manifest.json"),
                r#"{"status":"complete","duration_ms":30000}"#,
            )
            .unwrap();
            std::fs::write(
                folder.join("quick-title.json"),
                r#"{"title":"Synthetic completion"}"#,
            )
            .unwrap();
            std::fs::write(
                folder.join("transcript.ndjson"),
                serde_json::json!({"start_ms":0,"text":"Synthetic ".repeat(25)}).to_string(),
            )
            .unwrap();
            continue;
        }
        std::fs::write(
            folder.join("english-summary.json"),
            format!(
                r#"{{"title":"Synthetic meeting {index}","context":["Synthetic idle fixture."]}}"#
            ),
        )
        .unwrap();
    }
    #[cfg(feature = "idle-cpu-proof")]
    if std::env::args().any(|a| a == "--retained-done") {
        sidenote::core::notes::idle_fixture::seed().unwrap();
    }
    let mut rt = Runtime::with_text(
        State::new(),
        view,
        DEFAULT_SIZE,
        Env::default(),
        TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
    );
    rt.poll = Some(poll);
    rt
}

#[cfg_attr(test, test)]
fn idle_cpu_simulated_thirty_seconds() {
    #[cfg(test)]
    if isolated_test() {
        return;
    }
    let mut rt = runtime();
    let engine = sidenote::hex_engine::HexEngine::new();
    rt.settle_all();
    let mut rebuilds = 0;
    let mut repaint_requests = 0;
    let mut wakeups = 0;
    for _ in 0..20 {
        let status = sample_status(|| engine.status());
        rt.state
            .events_tx
            .send(Event::Hex(hex::Msg::ObservedStatus(status)))
            .unwrap();
        wakeups += 1;
        let before = rt.stats.commits;
        repaint_requests += usize::from(rt.frame(1.5));
        rebuilds += usize::from(rt.stats.commits != before);
    }
    println!(
        "simulated 30 s: animation repaint requests={repaint_requests}, UI rebuilds={rebuilds}, loop wakeups={wakeups}, commits={}",
        rt.stats.commits
    );
    assert!(rebuilds <= 2, "idle UI keeps rebuilding");
    assert!(
        repaint_requests <= 2,
        "idle UI keeps requesting animation frames"
    );
    assert!(wakeups <= 30, "idle UI keeps waking");
}

#[cfg_attr(test, test)]
fn idle_cpu_unchanged_refresh_does_not_rebuild() {
    #[cfg(test)]
    if isolated_test() {
        return;
    }
    let (mut rt, receive) = refreshed_runtime();
    let before = rt.stats.commits;
    let handle = rt.state.scan_tx.as_ref().unwrap();
    // A real scan request, with no filesystem mutation.
    handle.request(rt.state.refresh_options());
    receive.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(handle.unchanged_snapshot_ready(), "fixture was not idle");
    rt.frame(1.0);
    assert_eq!(
        rt.stats.commits, before,
        "unchanged production scan rebuilt the UI"
    );
}

fn refreshed_runtime() -> (Runtime<State>, Receiver<()>) {
    let mut rt = runtime();
    let (send, receive) = mpsc::channel();
    refresh::first_frame();
    rt.state.scan_tx = Some(refresh::start(
        std::sync::Arc::new(move || {
            let _ = send.send(());
        }),
        rt.state.refresh_options(),
    ));
    receive.recv_timeout(Duration::from_secs(30)).unwrap();
    rt.settle_all();
    rt.frame(0.0);
    for _ in 0..3 {
        rt.frame(0.1);
    }
    // Applying initial data can change refresh options or create local state. Complete those
    // real scans before measuring an unchanged one; never fabricate a refresh result.
    for scans in 1..=4 {
        let handle = rt.state.scan_tx.as_ref().unwrap();
        handle.request(rt.state.refresh_options());
        receive.recv_timeout(Duration::from_secs(30)).unwrap();
        let unchanged = handle.unchanged_snapshot_ready();
        rt.frame(0.1);
        if unchanged {
            println!("refresh initialization scans={scans}; unchanged snapshot verified");
            return (rt, receive);
        }
    }
    panic!("refresh fixture did not become idle");
}

#[cfg_attr(test, test)]
fn idle_cpu_delayed_empty_poll_sleeps() {
    #[cfg(test)]
    if isolated_test() {
        return;
    }
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (mut rt, _receive) = refreshed_runtime();
    let wakes = Arc::new(AtomicUsize::new(0));
    let count = wakes.clone();
    rt.state.waker = Some(Arc::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
    }));
    let (acquired, ready) = mpsc::sync_channel(1);
    let (release, held) = mpsc::channel();
    let worker = rt
        .state
        .scan_tx
        .as_ref()
        .unwrap()
        .pause_take(acquired, held);
    ready.recv_timeout(Duration::from_secs(30)).unwrap();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(10));
        release.send(()).unwrap();
    });
    rt.frame(0.1);
    worker.join().unwrap();
    releaser.join().unwrap();
    assert_eq!(
        wakes.load(Ordering::Relaxed),
        0,
        "an empty poll rescheduled itself after its budget expired"
    );
}

#[cfg_attr(test, test)]
fn idle_cpu_hex_state_changes_reach_the_view() {
    #[cfg(test)]
    if isolated_test() {
        return;
    }
    use std::{
        io::{BufRead, Read, Write},
        net::{TcpListener, TcpStream},
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };
    let mut rt = runtime();
    let engine = sidenote::hex_engine::HexEngine::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let dictating = Arc::new(AtomicBool::new(false));
    let meeting_seconds = Arc::new(AtomicUsize::new(usize::MAX));
    let stop = Arc::new(AtomicBool::new(false));
    let (state, stopping) = (dictating.clone(), stop.clone());
    let elapsed = meeting_seconds.clone();
    let worker = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            if stopping.load(Ordering::Relaxed) {
                break;
            }
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut reader = std::io::BufReader::new(&stream);
            let mut length = 0;
            loop {
                let mut line = String::new();
                assert_ne!(
                    reader.read_line(&mut line).unwrap(),
                    0,
                    "truncated synthetic HTTP request"
                );
                if line == "\r\n" {
                    break;
                }
                if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse::<usize>().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            let active = state.load(Ordering::Relaxed);
            let seconds = elapsed.load(Ordering::Relaxed);
            let meeting = if seconds == usize::MAX {
                serde_json::json!({"state":"idle"})
            } else {
                serde_json::json!({"state":"recording","id":"1800000000000-0","elapsed_seconds":seconds})
            };
            let body = serde_json::json!({"version":4,"status":"status","app_version":"synthetic",
                "meeting":meeting, "capture":{"state":if active {"recording"} else {"idle"},"recording":"dictation"},
                "model":{},"capabilities":{}}).to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let discovery = core::meetings_dir()
        .parent()
        .unwrap()
        .join("local-api.json");
    std::fs::write(&discovery, serde_json::json!({"port":address.port(),"token":format!("{:x}", chrono::Utc::now().timestamp_nanos_opt().unwrap())}).to_string()).unwrap();
    for active in [false, false, true, true, false] {
        dictating.store(active, Ordering::Relaxed);
        let earlier_sent = Instant::now();
        let status = sample_status(|| engine.status());
        assert!(status.1.is_ok());
        let before = rt.stats.commits;
        let changed = rt
            .state
            .hex
            .status
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|s| s.dictating)
            != Some(active);
        if changed && active {
            // A poll applies at most 8 events, so a backlog of earlier samples (a hidden window
            // shown again) leaves this one for a later frame; the waker repeats the poll.
            let earlier = rt.state.hex.status.clone().unwrap();
            for _ in 0..8 {
                rt.state
                    .events_tx
                    .send(Event::Hex(hex::Msg::ObservedStatus((
                        earlier_sent,
                        earlier.clone(),
                    ))))
                    .unwrap();
            }
        }
        rt.state
            .events_tx
            .send(Event::Hex(hex::Msg::ObservedStatus(status)))
            .unwrap();
        rt.frame(1.5);
        // Each frame applies at least one queued sample, so the queue (at most 9 here) drains
        // in a few frames. The assertion below is unchanged.
        for _ in 0..16 {
            let seen = rt
                .state
                .hex
                .status
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .and_then(|s| s.dictating);
            if seen == Some(active) {
                break;
            }
            rt.frame(0.0);
        }
        assert_eq!(
            rt.state
                .hex
                .status
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .dictating,
            Some(active)
        );
        if changed {
            assert!(
                rt.stats.commits > before,
                "Hex transition did not rebuild the view"
            );
        }
    }
    for seconds in [0, 2] {
        meeting_seconds.store(seconds, Ordering::Relaxed);
        let status = sample_status(|| engine.status());
        let before = rt.stats.commits;
        rt.state
            .events_tx
            .send(Event::Hex(hex::Msg::ObservedStatus(status)))
            .unwrap();
        rt.frame(1.5);
        assert!(rt.state.hex_recording() && rt.state.live.is_some());
        assert_eq!(
            rt.state
                .hex
                .status
                .as_ref()
                .unwrap()
                .as_ref()
                .ok()
                .unwrap()
                .recording
                .as_ref()
                .unwrap()
                .elapsed_seconds,
            Some(seconds as u64)
        );
        assert!(
            rt.stats.commits > before,
            "meeting progress did not rebuild the view"
        );
    }
    std::fs::remove_file(discovery).unwrap();
    stop.store(true, Ordering::Relaxed);
    let _ = TcpStream::connect(address);
    worker.join().unwrap();
}
