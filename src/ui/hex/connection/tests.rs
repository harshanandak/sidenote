use super::*;
use crate::ui::{
    State,
    hex::{self, Capabilities, Feature, Msg, Support},
    theme,
};
use sidenote::hex_engine::sample_status;
use std::collections::VecDeque;

struct FakeControl {
    replies: VecDeque<Result<EngineState, HexError>>,
    now: Instant,
}
impl FakeControl {
    fn poll(&mut self, state: &mut State) {
        // Use a fake clock for freshness and expiry checks.
        state.hex_status_at(self.replies.pop_front().unwrap(), self.now);
        self.now += Duration::from_secs(2);
    }
}
fn setup(
    errors: impl IntoIterator<Item = Result<EngineState, HexError>>,
    recording: bool,
) -> (State, FakeControl) {
    let mut state = State::new();
    state.hex.caps_version = Some("test".into());
    state.hex.caps = Some(Capabilities::from([
        (Feature::RecordStart, Support::Supported),
        (Feature::RecordStop, Support::Supported),
    ]));
    let good = EngineState {
        running: true,
        version: "test".into(),
        protocol: 4,
        recording: recording.then_some(Recording {
            id: Some("100-1".into()),
            title: None,
            since: None,
            elapsed_seconds: Some(1),
        }),
        meeting_state: "recording".into(),
        dictating: Some(false),
        capture_state: "idle".into(),
        model: hex::ModelState {
            id: None,
            name: None,
            language: None,
            installed: None,
            preparing: None,
            downloaded_bytes: None,
        },
        labs: Default::default(),
    };
    let replies = std::iter::once(Ok(good)).chain(errors).collect();
    (
        state,
        FakeControl {
            replies,
            // Keep real scheduling delays out of the fake clock.
            now: Instant::now() + Duration::from_secs(3600),
        },
    )
}

#[test]
fn timeout_and_transport_keep_recording_until_recovery() {
    for failure in [
        HexError::Timeout,
        HexError::Failed {
            code: None,
            stderr_summary: "synthetic",
        },
    ] {
        let (mut state, mut client) = setup([Err(failure)], true);
        client.poll(&mut state);
        let good = state.hex.status.as_ref().unwrap().as_ref().unwrap().clone();
        assert!(state.hex_running());
        assert!(state.hex_recording());
        client.poll(&mut state);
        assert!(state.hex_running());
        assert!(state.hex_recording());
        assert_eq!(hex::stop_blocked(&state), None);
        assert_eq!(
            super::super::recorder_start_block_reason(&state),
            Some(SLOW)
        );
        assert!(
            hex::indicator(&state, theme::palette(false))
                .1
                .starts_with(SLOW)
        );
        client.replies.push_back(Ok(good));
        client.poll(&mut state);
        assert!(state.hex_running());
        assert!(state.hex_recording());
        assert!(
            !hex::indicator(&state, theme::palette(false))
                .1
                .contains(SLOW)
        );
        let mut idle = state.hex.status.as_ref().unwrap().as_ref().unwrap().clone();
        idle.recording = None;
        client.replies.push_back(Ok(idle));
        client.poll(&mut state);
        assert!(
            !state.hex_recording(),
            "a fresh idle answer clears the recording"
        );
    }
}

#[test]
fn not_running_clears_immediately_and_initial_timeout_never_says_open_hex() {
    let (mut state, mut client) = setup([Err(HexError::HexNotRunning)], true);
    client.poll(&mut state);
    client.poll(&mut state);
    assert!(!state.hex_running());
    assert!(!state.hex_recording());
    // Exercise the ordinary installed case independently of this laptop's installation.
    state.hex.installation = sidenote::hex_install::Installation::Incomplete;
    assert_eq!(
        super::super::recorder_start_block_reason(&state),
        Some("Open Hex, then try New meeting again.")
    );
    state.hex_apply(Msg::ObservedStatus(sample_status(|| {
        Err(HexError::Timeout)
    })));
    assert_eq!(
        super::super::recorder_start_block_reason(&state),
        Some(SLOW)
    );
    assert_eq!(hex::indicator(&state, theme::palette(false)).1, SLOW);
}

#[test]
fn stale_bound_expires_availability_but_keeps_stop() {
    let (mut state, mut client) = setup(
        [
            Err(HexError::Timeout),
            Err(HexError::Timeout),
            Err(HexError::HexNotRunning),
        ],
        true,
    );
    client.poll(&mut state);
    client.now += STALE_LIMIT - Duration::from_secs(2);
    client.poll(&mut state);
    assert!(state.hex_running(), "last good is valid at the bound");
    client.poll(&mut state);
    assert!(
        !state.hex_running(),
        "expired status must not claim availability"
    );
    assert_eq!(super::super::recorder_start_block_reason(&state), None);
    assert!(
        hex::indicator(&state, theme::palette(false))
            .1
            .contains("status unknown")
    );
    assert_eq!(hex::stop_blocked(&state), None);
    client.poll(&mut state);
    assert!(!state.hex_recording());
    assert!(hex::stop_blocked(&state).is_some());
}

#[test]
#[cfg(windows)]
fn unknown_status_does_not_offer_to_open_an_installed_hex() {
    fn setup_view(state: &State, _: &mut rune::Cx<State>) -> rune::El {
        hex::setup::card(state, theme::palette(false)).unwrap_or_else(rune::el)
    }

    struct RestoreInstallDetect(bool);
    impl Drop for RestoreInstallDetect {
        fn drop(&mut self) {
            hex::SKIP_INSTALL_DETECT.set(self.0);
        }
    }
    let _restore = RestoreInstallDetect(hex::SKIP_INSTALL_DETECT.replace(true));
    let (mut state, mut client) = setup([], false);
    state.hex.installation = sidenote::hex_install::Installation::Installed {
        version: "0.0.0-dev-1085".into(),
        exe: "fixture/hex.exe".into(),
    };
    client.replies[0].as_mut().unwrap().version = "0.0.0-dev-1083".into();
    state.hex.caps_version = Some("0.0.0-dev-1083".into());
    client.poll(&mut state);
    assert!(state.hex_running(), "an update waiting does not stop Hex");
    assert!(hex::setup::card(&state, theme::palette(false)).is_none());
    let good = state.hex.status.as_ref().unwrap().as_ref().unwrap().clone();
    state.hex_status_at(Ok(good), Instant::now() - Duration::from_secs(11));
    assert!(state.hex_status_unknown());
    assert!(hex::setup::card(&state, theme::palette(false)).is_none());
    let mut runtime = rune::Runtime::new(state, setup_view, (640.0, 480.0), rune::Env::default());
    runtime.state.hex.install = sidenote::hex_install::State::Complete;
    for error in [
        HexError::Timeout,
        HexError::Failed {
            code: None,
            stderr_summary: "synthetic",
        },
    ] {
        runtime
            .state
            .hex_apply(Msg::ObservedStatus(sample_status(|| Err(error))));
        runtime.invalidate();
        runtime.frame(0.0);
        assert!(
            runtime
                .elements()
                .iter()
                .all(|element| element.key.as_deref() != Some("hex-setup-open"))
        );
    }
    runtime.state.hex.install = sidenote::hex_install::State::Idle;
    runtime
        .state
        .hex_apply(Msg::ObservedStatus(sample_status(|| {
            Err(HexError::HexNotRunning)
        })));
    runtime.invalidate();
    runtime.frame(0.0);
    assert!(
        runtime
            .elements()
            .iter()
            .any(|element| element.key.as_deref() == Some("hex-setup-open"))
    );
}

#[test]
fn recorder_ready_duplicate_entry_keeps_the_pending_action_sequence() {
    crate::test_paths::isolate();
    let mut connection = Connection::default();
    assert!(connection.begin_action());
    let pending = connection.action;
    assert!(!connection.begin_action());
    assert_eq!(connection.action, pending);
    connection.finish_action();
    assert!(connection.begin_action());
    assert_ne!(connection.action, pending);
    connection.finish_action();
}
