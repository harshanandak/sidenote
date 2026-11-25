use super::*;

#[cfg_attr(test, test)]
#[cfg(windows)]
#[cfg_attr(
    test,
    ignore = "fixed 30 s release processor sample; no native windows"
)]
pub(super) fn idle_cpu_real_thirty_seconds() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetPriorityClass};
    println!(
        "release headless Runtime; 300 synthetic meetings; no native window/GPU; priority_class={}",
        unsafe { GetPriorityClass(GetCurrentProcess()) }
    );
    let mut rt = runtime();
    let (send, receive) = mpsc::channel();
    refresh::first_frame();
    let wake: Waker = Arc::new(move || {
        let _ = send.send(());
    });
    rt.state.waker = Some(wake.clone());
    rt.state.refresh_ai_notes();
    rt.state.scan_tx = Some(refresh::start(wake.clone(), rt.state.refresh_options()));
    // Exercise real status calls without joining the installed app's shared Windows event.
    let (engine, events) = (
        sidenote::hex_engine::HexEngine::new(),
        rt.state.events_tx.clone(),
    );
    let stop = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let (stopping, count) = (stop.clone(), calls.clone());
    let status = std::thread::spawn(move || {
        while !stopping.load(Ordering::Relaxed) {
            let result = sample_status(|| engine.status());
            count.fetch_add(1, Ordering::Relaxed);
            if events
                .send(Event::Hex(hex::Msg::ObservedStatus(result)))
                .is_err()
            {
                break;
            }
            wake();
            std::thread::sleep(Duration::from_millis(1500));
        }
    });
    rt.settle_all();
    // Exclude initialization and font loading from the idle sample.
    rt.frame(0.0);
    let settling = Instant::now();
    while settling.elapsed() < Duration::from_secs(5) {
        let _ = receive.recv_timeout(Duration::from_millis(100));
        rt.frame(0.1);
    }
    println!("IDLE_CPU_READY");
    let before = processor_times();
    let calls_before = calls.load(Ordering::Relaxed);
    let start = Instant::now();
    let mut previous = start;
    let mut clock = rt.clock;
    let mut wakes = 0;
    let mut rebuilds = 0;
    let mut repaint_requests = 0;
    while start.elapsed() < Duration::from_secs(30) {
        let now = Instant::now();
        let dt = now.duration_since(previous).as_secs_f64();
        previous = now;
        clock += dt;
        let commits = rt.stats.commits;
        let animating = rt.frame(dt);
        wakes += 1;
        rebuilds += usize::from(rt.stats.commits != commits);
        repaint_requests += usize::from(animating);
        let wait = if animating {
            Duration::from_secs_f64(1.0 / 60.0)
        } else {
            Duration::from_secs_f64((rt.wake_deadline().unwrap_or(clock + 30.0) - clock).max(0.001))
        }
        .min(Duration::from_secs(30).saturating_sub(start.elapsed()));
        let _ = receive.recv_timeout(wait);
        while receive.try_recv().is_ok() {}
    }
    let after = processor_times();
    let seconds = start.elapsed().as_secs_f64();
    let cpu = (after.0 - before.0) as f64 / 10_000_000.0;
    let load = 100.0 * (1.0 - (after.2 - before.2) as f64 / (after.1 - before.1) as f64);
    println!(
        "30 s deadline: seconds={seconds:.3}, cpu_s={cpu:.6}, one_core_percent={:.3}, system_load_percent={load:.3}, animation repaint requests={repaint_requests}, UI rebuilds={rebuilds}, loop wakeups={wakes}, commits={}, status_calls={}",
        cpu / seconds * 100.0,
        rt.stats.commits,
        calls.load(Ordering::Relaxed) - calls_before
    );
    stop.store(true, Ordering::Relaxed);
    status.join().unwrap();
    assert!(cpu < 0.3, "idle UI exceeded 1% of one core over 30 s");
}

#[cfg(windows)]
fn processor_times() -> (u64, u64, u64) {
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::*};
    fn value(t: FILETIME) -> u64 {
        u64::from(t.dwHighDateTime) << 32 | u64::from(t.dwLowDateTime)
    }
    unsafe {
        let (
            mut created,
            mut exit,
            mut kernel,
            mut user,
            mut idle,
            mut system_kernel,
            mut system_user,
        ) = std::mem::zeroed();
        assert_ne!(
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exit,
                &mut kernel,
                &mut user
            ),
            0
        );
        assert_ne!(
            GetSystemTimes(&mut idle, &mut system_kernel, &mut system_user),
            0
        );
        (
            value(kernel) + value(user),
            value(system_kernel) + value(system_user),
            value(idle),
        )
    }
}
