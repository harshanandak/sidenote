#![cfg_attr(all(windows, feature = "desktop"), windows_subsystem = "windows")]
#[cfg(windows)]
mod hex_probe;
#[cfg(all(windows, test))]
mod install_hooks_tests;
#[cfg(all(test, feature = "desktop"))]
#[path = "../tests/common/mod.rs"]
mod test_paths;
#[cfg(all(test, feature = "desktop"))]
#[test]
fn default_test_paths_are_isolated_from_owner_appdata() {
    test_paths::isolate();
}
#[cfg(feature = "desktop")]
mod ui;
fn unavailable_in_stable(args: &[std::ffi::OsString]) -> Option<&str> {
    const DEV_FLAGS: &[&str] = &[
        "--server",
        "--speech-worker",
        "--speech-port",
        "--sync-control",
        "--mcp",
        "--mcp-access",
        "--mcp-port",
        "--ops",
        "--cpu-models",
        "--relay-preview-headless",
    ];
    if args
        .windows(2)
        .any(|pair| pair[0] == "--device-role" && (pair[1] == "server" || pair[1] == "hub"))
    {
        return Some("Server or hub mode");
    }
    args.iter()
        .filter_map(|arg| arg.to_str())
        .find(|arg| DEV_FLAGS.contains(arg) || (!cfg!(windows) && *arg == "--listen"))
}
fn main() {
    #[cfg(feature = "desktop")]
    let process_started = std::time::Instant::now();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if matches!(sidenote::core::CHANNEL, sidenote::core::Channel::Stable)
        && let Some(arg) = unavailable_in_stable(&args)
    {
        eprintln!("{arg} is unavailable in Sidenote Stable.");
        std::process::exit(2);
    }
    #[cfg(target_os = "linux")]
    sidenote::platform::supervisor_entry();
    #[cfg(all(windows, feature = "idle-cpu-proof"))]
    if args.iter().any(|a| a == "--idle-cpu-seed-notes") {
        let result = sidenote::core::configure_paths(&args)
            .and_then(|_| sidenote::core::notes::idle_fixture::seed().map_err(|e| e.to_string()));
        std::process::exit(i32::from(result.is_err()));
    }
    #[cfg(all(windows, feature = "idle-cpu-proof"))]
    if args.iter().any(|a| a == "--idle-cpu-proof") {
        std::process::exit(i32::from(!ui::idle_cpu_proof()));
    }
    #[cfg(any(windows, target_os = "linux"))]
    if args.iter().any(|a| a == "--routing-native") {
        std::process::exit(i32::from(
            sidenote::core::meeting_router::production::native_worker().is_err(),
        ));
    }
    if args.iter().any(|a| a == "--language-detect") {
        std::process::exit(sidenote::core::lid::run_cli(&args));
    }
    if args.iter().any(|a| a == "--notes-engine") {
        let policy = if sidenote::core::configure_paths(&args).is_ok() {
            sidenote::core::notes::scripts_policy()
        } else {
            "settings_invalid"
        };
        println!("{policy}");
        return;
    }
    #[cfg(all(windows, feature = "desktop"))]
    if args.iter().any(|a| a == "--render-meeting-indicator") {
        if sidenote::core::configure_paths(&args).is_err()
            || sidenote::meeting_icon::render_synthetic().is_err()
        {
            std::process::exit(2);
        }
        return;
    }
    #[cfg(windows)]
    {
        velopack::VelopackApp::build()
            .set_auto_apply_on_startup(false)
            .on_after_install_fast_callback(|_| {
                if let Err(error) = sidenote::updates::after_install() {
                    sidenote::listener::log_process(&format!(
                        "after-install hook failed reason={error}"
                    ));
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            })
            .on_first_run(|_| {
                let _ = sidenote::updates::after_update();
            })
            .on_after_update_fast_callback(|_| {
                if let Err(error) = sidenote::updates::after_update_fast() {
                    sidenote::listener::log_process(&format!(
                        "after-update hook failed reason={error}"
                    ));
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            })
            .on_restarted(|_| sidenote::updates::restarted())
            .run();
    }
    #[cfg(feature = "desktop")]
    if args.iter().any(|arg| arg == "--response-times") {
        sidenote::response_time::startup(process_started);
    }
    if args.iter().any(|a| a == "--listener-isolated")
        && let Some(pair) = args
            .windows(2)
            .find(|pair| pair[0] == "--listener-localappdata")
    {
        if !std::path::Path::new(&pair[1]).is_absolute() {
            std::process::exit(2);
        }
        // The isolated task configures its own environment before any app worker starts.
        unsafe {
            std::env::set_var("LOCALAPPDATA", &pair[1]);
        }
    }
    if let Err(error) = sidenote::core::configure_paths(&args) {
        eprintln!("{error}");
        std::process::exit(2);
    }
    if let Err(code) = sidenote::service::configure(&args) {
        eprintln!("{code}");
        std::process::exit(2);
    }
    #[cfg(all(windows, feature = "desktop", feature = "asr-runtime"))]
    if let Some(index) = args
        .iter()
        .position(|arg| arg == "--model-provisioning-check")
    {
        if !args.windows(2).any(|pair| pair[0] == "--data-dir") {
            std::process::exit(2);
        }
        let mode = args
            .get(index + 1)
            .and_then(|arg| arg.to_str())
            .unwrap_or("");
        let result = ui::models::proof::run(mode);
        if let Err(error) = &result {
            eprintln!("{error}");
        }
        std::process::exit(i32::from(result.is_err()));
    }
    if args.iter().any(|a| a == "--sync-control") {
        #[cfg(target_os = "linux")]
        std::process::exit(sidenote::sync_owner::cli());
        #[cfg(not(target_os = "linux"))]
        {
            eprintln!("SYNC_CONTROL_UNAVAILABLE_ON_PLATFORM");
            std::process::exit(2);
        }
    }
    #[cfg(target_os = "linux")]
    if args.iter().any(|a| a == "--mcp-access") {
        std::process::exit(sidenote::mcp::access::cli(&args));
    }
    #[cfg(target_os = "linux")]
    if args.iter().any(|a| a == "--ops") {
        std::process::exit(sidenote::operations::cli(&args));
    }
    #[cfg(target_os = "linux")]
    if args.iter().any(|a| a == "--cpu-models") {
        std::process::exit(sidenote::server_models::cli(&args));
    }
    if !args.iter().any(|a| a == "--mcp") && sidenote::service::requested(&args) {
        std::process::exit(sidenote::service::run());
    }
    if args.iter().any(|a| a == "--mcp") {
        std::process::exit(sidenote::mcp::run());
    }
    if let Some(variant) = args.windows(2).find(|a| a[0] == "--notebook-benchmark") {
        let Some(output) = args
            .windows(2)
            .find(|a| a[0] == "--annotation-proof-output")
        else {
            std::process::exit(2)
        };
        if let Err(e) = sidenote::core::annotations::notebook::benchmark(
            std::path::Path::new(&output[1]),
            &variant[1].to_string_lossy(),
        ) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(output) = args.windows(2).find(|a| a[0] == "--notebook-live-proof") {
        if let Err(e) =
            sidenote::core::annotations::notebook::live_proof(std::path::Path::new(&output[1]))
        {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(variant) = args.windows(2).find(|a| a[0] == "--annotation-benchmark") {
        let Some(output) = args
            .windows(2)
            .find(|a| a[0] == "--annotation-proof-output")
        else {
            std::process::exit(2);
        };
        if let Err(e) = sidenote::core::annotations::generation::benchmark(
            &variant[1].to_string_lossy(),
            std::path::Path::new(&output[1]),
        ) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(model) = args.windows(2).find(|a| a[0] == "--annotation-live-proof") {
        let output = args
            .windows(2)
            .find(|a| a[0] == "--annotation-proof-output")
            .map(|a| std::path::PathBuf::from(&a[1]));
        let Some(output) = output else {
            eprintln!("Provide --annotation-proof-output for the synthetic proof.");
            std::process::exit(2);
        };
        if let Err(error) = sidenote::core::annotations::generation::live_proof(
            &model[1].to_string_lossy(),
            &output,
        ) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|a| a == "--relay-preview-headless") {
        #[cfg(any(debug_assertions, feature = "relay-qualification"))]
        std::process::exit(i32::from(
            sidenote::core::relay_preview::headless(&args).is_err(),
        ));
        #[cfg(not(any(debug_assertions, feature = "relay-qualification")))]
        std::process::exit(2);
    }
    if args.iter().any(|a| a == "--version") {
        println!("Sidenote {}", sidenote::updates::VERSION);
        return;
    }
    if args.iter().any(|a| {
        matches!(
            a.to_str(),
            Some(
                "--update-check"
                    | "--update-apply"
                    | "--update-status"
                    | "--update-window-start"
                    | "--update-probe"
            )
        )
    }) {
        std::process::exit(sidenote::updates::cli(&args));
    }
    let helper = args.iter().any(|a| {
        matches!(
            a.to_str(),
            Some(
                "--transcribe-file"
                    | "--route-one"
                    | "--route-pending"
                    | "--notes-pending"
                    | "--daily-check"
            )
        )
    });
    let _activity = if helper {
        match sidenote::updates::activity() {
            Ok(guard) => Some(guard),
            Err(_) => std::process::exit(2),
        }
    } else {
        None
    };
    if args.iter().any(|a| a == "--transcribe-file") {
        std::process::exit(sidenote::core::meeting_router::run_cli(&args));
    }
    if args.iter().any(|a| a == "--route-one") {
        let result = sidenote::core::meeting_router::jobs::run_one();
        if let Err(error) = &result
            && let Some(path) = args
                .windows(2)
                .find(|a| a[0] == "--router-request")
                .map(|a| std::path::PathBuf::from(&a[1]))
            && let Some(parent) = path.parent()
        {
            let _ = std::fs::write(
                parent.join("failure.json"),
                serde_json::json!({"error": error.to_string()}).to_string(),
            );
        }
        std::process::exit(i32::from(result.is_err()));
    }
    if let Some(id) = args.windows(2).find(|a| a[0] == "--enqueue-meeting") {
        let result = sidenote::core::meeting_router::jobs::enqueue(&id[1].to_string_lossy());
        std::process::exit(i32::from(result.is_err()));
    }
    if args.iter().any(|a| a == "--route-pending") {
        let result = sidenote::core::meeting_router::jobs::run_pending();
        std::process::exit(i32::from(result.is_err()));
    }
    if args.iter().any(|a| a == "--notes-pending") {
        let result = sidenote::core::notes::queue::run_pending();
        std::process::exit(i32::from(result.is_err()));
    }
    if std::env::args().any(|a| a == "--hex-engine-status") {
        std::process::exit(sidenote::hex_engine::run_status());
    }
    #[cfg(windows)]
    if std::env::args().any(|a| a == "--probe-hex") {
        std::process::exit(hex_probe::run());
    }
    if std::env::args().any(|a| a == "--daily-check") {
        std::process::exit(sidenote::core::daily_check::run_cli());
    }
    #[cfg(feature = "desktop")]
    sidenote::desktop::run(ui::run);
    #[cfg(not(feature = "desktop"))]
    {
        eprintln!("UNAVAILABLE_ON_PLATFORM");
        std::process::exit(2);
    }
}
