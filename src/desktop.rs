//! Shared desktop launch policy; screens and native window hooks are supplied separately.
use crate::platform;

pub mod startup;

pub fn run(process_started: std::time::Instant, open_screens: impl FnOnce()) {
    if crate::service::role() != crate::service::DeviceRole::Full {
        eprintln!("UNAVAILABLE_ON_PLATFORM");
        return;
    }
    let headless = headless_args(std::env::args().skip(1));
    let offscreen_bench = crate::env_var("SIDENOTE_BENCH_OFFSCREEN").is_some();
    let _instance = if headless || offscreen_bench {
        None
    } else {
        match platform::single_instance() {
            Ok(Some(instance)) => Some(instance),
            Ok(None) => return,
            Err(_) => {
                #[cfg(windows)]
                platform::show_error(
                    "Sidenote could not check whether it is already open. Reopen Sidenote to try again. If this continues, restart Windows.",
                );
                #[cfg(not(windows))]
                platform::show_error("Sidenote could not acquire its desktop instance lock.");
                return;
            }
        }
    };
    #[cfg(windows)]
    let _update_window = if headless || offscreen_bench {
        None
    } else {
        match crate::updates::window_lock() {
            Ok(guard) => Some(guard),
            Err(crate::updates::WindowLockError::AlreadyOpen) => return,
            Err(crate::updates::WindowLockError::Io(error)) => {
                let message = match error.kind() {
                    std::io::ErrorKind::PermissionDenied => {
                        "Sidenote cannot access its update folder. Check your Windows account's folder permissions, then reopen Sidenote."
                    }
                    _ => {
                        "Sidenote could not open its update lock. Check that your local drive is available and has free space, then reopen Sidenote."
                    }
                };
                platform::show_error(message);
                return;
            }
        }
    };
    if !headless && !offscreen_bench {
        startup::start(process_started);
    }
    if !headless && (!offscreen_bench || crate::env_var("SIDENOTE_BENCH_NO_WARM").is_none()) {
        rune::warm_gpu(Default::default());
    }
    open_screens();
    // Stop Ask and Memory helpers before exiting so their processes and scratch folders go with them.
    crate::core::ask::cancel_all(std::time::Duration::from_secs(3));
    std::process::exit(0);
}

fn headless_args(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--snap"
                | "--tree"
                | "--script"
                | "--frames"
                | "--video"
                | "--strip"
                | "--list-scenes"
                | "--help"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::headless_args;

    #[test]
    fn render_commands_bypass_interactive_locks() {
        for arg in [
            "--snap",
            "--tree",
            "--script",
            "--frames",
            "--video",
            "--strip",
            "--list-scenes",
            "--help",
        ] {
            assert!(headless_args([arg.to_owned()]));
        }
        assert!(!headless_args(["--response-times".to_owned()]));
        assert!(!headless_args([]));
    }
}
