use super::*;

const WAIT: Duration = Duration::from_secs(30);
const FINISHING: &str = "Recording stopped. Hex is still transcribing.";
pub(in crate::ui) const UNCONFIRMED: &str = "The recording may still be finishing in Hex.";

pub(super) struct Completion {
    recording: u64,
    pub(super) deadline: Option<Instant>,
}

pub(in crate::ui) fn scene(s: &mut State, timed_out: bool) {
    super::super::live::scene_making(s, "transcribing");
    s.hex.stop = Some(Completion {
        recording: 0,
        deadline: (!timed_out).then(|| Instant::now() + WAIT),
    });
}

pub(in crate::ui) fn terminal(state: &EngineState) -> bool {
    state.recording.is_none()
        && matches!(
            state.meeting_state.as_str(),
            "idle" | "stopped" | "complete" | "finished"
        )
}

impl State {
    pub(in crate::ui) fn hex_stop_pending(&self) -> bool {
        self.hex.stop.is_some()
    }

    pub(in crate::ui) fn hex_stop_unconfirmed(&self) -> bool {
        self.hex
            .stop
            .as_ref()
            .is_some_and(|stop| stop.deadline.is_none())
    }

    pub(super) fn hex_begin_stop(&mut self, recording: u64, now: Instant) {
        if let Some(live) = &self.live {
            live.pause_feed();
        }
        self.hex.stop = Some(Completion {
            recording,
            deadline: Some(now + WAIT),
        });
    }

    pub(super) fn hex_stop_failed(
        &mut self,
        recording: u64,
        error: &HexError,
        checked: bool,
        applied: bool,
    ) {
        if let Some(live) = &mut self.live {
            super::super::response_time::failed(live.take_notes_timing(recording));
            live.generate_after_pause = false;
        }
        let refused = matches!(error, HexError::Unsupported)
            || matches!(error, HexError::Failed { code: Some(code), .. }
                if matches!(code.as_str(), "recording-changed" | "not-recording" | "busy"));
        let status = self.hex.status.as_ref().and_then(|r| r.as_ref().ok());
        if status.is_some_and(terminal) {
            self.hex_complete_stop(recording);
            self.hex.error = Some("The recording already ended in Hex.");
        } else if matches!(status, Some(s) if s.meeting_state == "transcribing") {
            self.hex.error = Some(FINISHING);
            self.hex_check_stop();
        } else if refused || !checked {
            self.hex.stop = None;
            self.live_sync();
            if checked && !applied {
                self.hex.error = None;
            }
        } else {
            self.hex_check_stop();
        }
    }

    pub(super) fn hex_check_stop(&mut self) {
        if let Some(deadline) = self.hex.stop.as_ref().and_then(|stop| stop.deadline) {
            // The watcher can deliver a sample taken before Stop replied.
            self.hex_spawn(move |engine| {
                Msg::StopStatus(deadline, sample_status(|| engine.status()))
            });
        }
    }

    pub(in crate::ui) fn hex_recheck_stop(&mut self) {
        if self.hex.record_busy || !self.hex_stop_unconfirmed() {
            return;
        }
        self.hex.stop.as_mut().unwrap().deadline = Some(Instant::now() + WAIT);
        self.hex_check_stop();
    }

    pub(super) fn hex_stop_status(&mut self, deadline: Instant, sample: StatusSample) {
        if self.hex.stop.as_ref().and_then(|stop| stop.deadline) != Some(deadline) {
            return;
        }
        if !self.hex_accept_status(&sample) {
            return;
        }
        let (sent, Ok(status)) = sample else { return };
        if terminal(&status) {
            let stop = self.hex.stop.as_mut().unwrap();
            stop.deadline = None;
            let recording = stop.recording;
            self.hex_status_at(Ok(status), sent);
            self.hex.error = None;
            self.hex_complete_stop(recording);
        } else {
            let transcribing = status.meeting_state == "transcribing";
            self.hex_status_at(Ok(status), sent);
            if transcribing {
                self.hex.error = Some(FINISHING);
            }
        }
    }

    fn hex_complete_stop(&mut self, recording: u64) {
        self.hex.stop = None;
        if self.live.as_ref().map(|l| l.recording_id) == Some(recording) {
            self.live_stopped();
            self.live_sync();
        } else {
            self.request_refresh();
        }
    }

    pub(super) fn hex_stop_tick(&mut self, now: Instant, tx: &mut Tx) -> bool {
        let Some(stop) = &mut self.hex.stop else {
            return false;
        };
        if let Some(deadline) = stop.deadline.filter(|&deadline| now < deadline) {
            tx.wake_after(deadline - now);
            return false;
        }
        stop.deadline.take().is_some()
    }
}
