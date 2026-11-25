use super::*;

fn next_recovery_event(state: &mut State, until: Instant) -> Result<Event, RecvTimeoutError> {
    loop {
        let event = state
            .events_rx
            .recv_timeout(until.saturating_duration_since(Instant::now()))?;
        match event {
            // State::new launches these refreshes before the test starts the watcher.
            Event::Area { refresh: true, .. } | Event::MemoryRefreshed { .. } => {
                eprintln!("recovery fixture handled {}", event_description(&event));
                state.apply_event(event);
            }
            _ => return Ok(event),
        }
    }
}

fn event_description(event: &Event) -> String {
    match event {
        Event::Area { refresh, .. } => format!("Area(refresh={refresh})"),
        Event::MemoryRefreshed { .. } => "MemoryRefreshed".into(),
        Event::Hex(Msg::RecoveryStatus((_, result))) => format!("Hex::RecoveryStatus({result:?})"),
        Event::Hex(Msg::ObservedStatus((_, result))) => format!("Hex::ObservedStatus({result:?})"),
        Event::Hex(Msg::WatcherStopped) => "Hex::WatcherStopped".into(),
        Event::Hex(Msg::Caps(..)) => "Hex::Caps".into(),
        Event::Hex(Msg::Install(..)) => "Hex::Install".into(),
        Event::Hex(msg) => format!("Hex message {:?}", std::mem::discriminant(msg)),
        other => format!("Event {:?}", std::mem::discriminant(other)),
    }
}

fn expire_status(state: &mut State) {
    let (_, status) = command_status_sample(state);
    state.apply_event(Event::Hex(Msg::ObservedStatus((
        Instant::now() - Duration::from_secs(11),
        status,
    ))));
    assert!(state.hex_status_unknown());
}

#[test]
fn recovery_retries_expired_status_through_tick() {
    if isolated_api_test("recovery::recovery_retries_expired_status_through_tick") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    state.waker = Some(std::sync::Arc::new(|| {}));
    let (commands, _watcher) = mpsc::channel();
    state.hex.visibility = Some(commands);
    state.hex_visibility(true);
    let mut tx = Tx::default();
    state.hex_connection_tick(&mut tx);
    assert!(!state.hex.connection.recovering, "the watcher is healthy");

    expire_status(&mut state);
    state.hex_visibility(false);
    state.hex_connection_tick(&mut tx);
    assert!(!state.hex.connection.recovering, "the window is hidden");

    state.hex_visibility(true);
    api.data.lock().unwrap().status_timeout = true;
    state.hex_connection_tick(&mut tx);
    state.hex_connection_tick(&mut tx);
    let event = next_recovery_event(&mut state, Instant::now() + Duration::from_secs(2)).unwrap();
    assert!(
        matches!(
            &event,
            Event::Hex(Msg::RecoveryStatus((_, Err(HexError::Timeout))))
        ),
        "expected a recovery timeout, got {}",
        event_description(&event)
    );
    match next_recovery_event(&mut state, Instant::now() + Duration::from_millis(250)) {
        Err(RecvTimeoutError::Timeout) => {}
        Err(error) => panic!("checking for a duplicate recovery reply: {error}"),
        Ok(event) => panic!("unexpected extra event: {}", event_description(&event)),
    }
    state.apply_event(event);
    assert!(state.hex_status_unknown());
    assert_eq!(state.hex.connection.recovery_delay, Duration::from_secs(1));
    state.hex_connection_tick(&mut tx);
    assert!(!state.hex.connection.recovering, "backoff still applies");

    // Move the retry deadline forward without waiting on a wall-clock timer.
    state.hex.connection.recovery_at = Some(Instant::now());
    {
        let mut data = api.data.lock().unwrap();
        data.status_timeout = false;
        data.id = Some("200-2");
    }
    state.hex_connection_tick(&mut tx);
    let event = next_recovery_event(&mut state, Instant::now() + Duration::from_secs(2)).unwrap();
    assert!(
        matches!(&event, Event::Hex(Msg::RecoveryStatus((_, Ok(_))))),
        "expected a successful recovery reply, got {}",
        event_description(&event)
    );
    state.apply_event(event);
    state.hex_connection_tick(&mut tx);
    assert_eq!(recording_id(&state).as_deref(), Some("200-2"));
    assert!(!state.hex_status_unknown());
    assert_eq!(state.hex.connection.recovery_delay, Duration::ZERO);
}

#[test]
fn recovery_restarts_stopped_watcher_and_rejects_an_older_reply() {
    if isolated_api_test("recovery::recovery_restarts_stopped_watcher_and_rejects_an_older_reply") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    state.waker = Some(std::sync::Arc::new(|| {}));
    let (commands, _watcher) = mpsc::channel();
    state.hex.visibility = Some(commands);
    state.hex_visibility(true);
    state.apply_event(Event::Hex(Msg::WatcherStopped));
    assert!(state.hex.visibility.is_none());

    // A dead watcher needs replacing even while its last sample is fresh.
    assert!(!state.hex_status_unknown());
    state.hex_connection_tick(&mut Tx::default());
    assert!(state.hex.visibility.is_some());
    let mut recovery = None;
    let mut observed = false;
    let until = Instant::now() + Duration::from_secs(2);
    while recovery.is_none() || !observed {
        let event = next_recovery_event(&mut state, until).unwrap();
        match event {
            Event::Hex(Msg::RecoveryStatus((_, Ok(_)))) => recovery = Some(event),
            Event::Hex(Msg::ObservedStatus((_, Ok(_)))) => {
                state.apply_event(event);
                observed = true;
            }
            other => panic!(
                "expected a successful sample from the restarted watcher or recovery, got {}",
                event_description(&other)
            ),
        }
    }
    api.data.lock().unwrap().id = None;
    state.apply_event(Event::Hex(Msg::ObservedStatus(command_status_sample(
        &state,
    ))));
    state.apply_event(recovery.unwrap());
    assert!(!state.hex_recording());
    assert!(!state.ask_live_meeting());
    assert!(!state.hex_status_unknown());
}
