fn wired(source: &str) -> bool {
    let Some(builder) = source.split("velopack::VelopackApp::build()").nth(1) else {
        return false;
    };
    let Some((builder, _)) = builder.split_once(".run();") else {
        return false;
    };
    let compact: String = builder.chars().filter(|c| !c.is_whitespace()).collect();
    [
        r#".on_after_install_fast_callback(|_|{ifletErr(error)=sidenote::updates::after_install(){sidenote::listener::log_process(&format!("after-installhookfailedreason={error}"));eprintln!("{error}");std::process::exit(1);}})"#,
        r#".on_after_update_fast_callback(|_|{ifletErr(error)=sidenote::updates::after_update_fast(){sidenote::listener::log_process(&format!("after-updatehookfailedreason={error}"));eprintln!("{error}");std::process::exit(1);}})"#,
        ".on_first_run(|_|{let_=sidenote::updates::after_update();})",
        ".on_restarted(|_|sidenote::updates::restarted())",
    ].iter().all(|route| compact.contains(route))
}

#[test]
fn installed_callbacks_limit_handoff_to_setup_and_propagate_failure() {
    let source = include_str!("main.rs");
    assert!(wired(source));
    for (route, replacement) in [
        ("on_after_install_fast_callback", "after_install"),
        ("on_first_run", "after_update"),
        ("on_after_update_fast_callback", "after_update_fast"),
        ("on_restarted", "restarted"),
    ] {
        let (before, callback) = source.split_once(route).unwrap();
        let changed = callback.replacen(replacement, "wrong_hook", 1);
        assert!(!wired(&format!("{before}{route}{changed}")), "{route}");
        if route.ends_with("_fast_callback") {
            let changed = callback.replacen("std::process::exit(1);", "return;", 1);
            assert!(!wired(&format!("{before}{route}{changed}")), "{route}");
            let changed = callback.replacen("sidenote::listener::log_process", "skip_logging", 1);
            assert!(!wired(&format!("{before}{route}{changed}")), "{route}");
        }
    }
}
