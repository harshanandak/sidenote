use super::*;
use crate::ui::Event;
use sidenote::core::{
    self,
    meeting_parts::{self, Draft, Meeting, Phase as SavedPhase},
};
impl State {
    pub(in crate::ui) fn parts_sync(&mut self) {
        if self.hex_stop_pending() || self.live.as_ref().is_some_and(|l| l.scene) {
            return;
        }
        let root = core::state_dir();
        let hex = core::meetings_dir();
        if self.live.is_none() && !self.hex.parts_restore_checked {
            self.hex.parts_restore_checked = true;
            match meeting_parts::list(&root) {
                Ok(all) => {
                    if let Some(m) = all
                        .into_iter()
                        .filter(|m| m.phase != SavedPhase::Ended)
                        .max_by_key(|m| m.parts.first().map(|p| p.started_ms))
                    {
                        match m.restore(&root, &hex) {
                            Ok(m) => {
                                let mut live = Live::new(Phase::Paused, self.known_ids());
                                live.folder = Some(meeting_parts::inputs(&root).join(&m.id));
                                self.live_notes = TextEdit::new(feed_io::read_notes(
                                    live.folder.as_ref().unwrap(),
                                ));
                                if let Some(at) = m
                                    .parts
                                    .first()
                                    .and_then(|p| DateTime::from_timestamp_millis(p.started_ms))
                                {
                                    live.wall_start = at.with_timezone(&Local);
                                    live.exact = true;
                                }
                                live.recovering_part = true;
                                live.saved = self.live_notes.text.clone();
                                live.seen = live.saved.clone();
                                live.notes_loaded = true;
                                live.session = Some(m);
                                self.live = Some(live);
                                self.go_home();
                                self.live_open = true;
                            }
                            Err(_) => {
                                self.hex.parts_restore_checked = false;
                                self.hex.error = Some("Could not restore the paused meeting.");
                                return;
                            }
                        }
                    }
                }
                Err(_) => {
                    self.hex.parts_restore_checked = false;
                    self.hex.error = Some("Could not read meeting parts.");
                    return;
                }
            }
        }
        let hint = hex::recording_id(self);
        let elapsed = hex::part_elapsed_ms(self);
        let active_id = hint.clone();
        let status_known = !self.hex_status_unknown() && matches!(&self.hex.status, Some(Ok(_)));
        let confirmed = self.hex_confirmed_recording();
        let start = hex::recording_start(self).map(|(at, _)| at.timestamp_millis());
        let adopting = self.live.as_ref().is_some_and(|live| {
            live.session.is_none() && matches!(live.phase, Phase::Starting | Phase::Recording)
        }) && hint
            .as_ref()
            .is_some_and(|id| hex.join(id).join("manifest.json").is_file());
        if adopting && !self.live_close() {
            self.hex.error = Some("Save these notes before pausing the meeting.");
            return;
        }
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.scene {
            return;
        }
        if let Some(m) = &mut live.session
            && m.pending_start_ms.is_some()
            && let Ok(Some(saved)) = meeting_parts::read(&root, &m.id)
            && saved.parts.len() > m.parts.len()
        {
            *m = saved;
            if active_id.is_none() {
                let _ = m.pause(&root);
                live.phase = Phase::Paused;
            }
        }
        let start = start.or_else(|| live.session.as_ref().and_then(|m| m.pending_start_ms));
        if let (Some(id), Some(start)) = (hint, start) {
            if live.session.is_none()
                && hex.join(&id).join("manifest.json").is_file()
                && matches!(live.phase, Phase::Starting | Phase::Recording)
            {
                match Meeting::begin(&root, &id, start) {
                    Ok(m) => {
                        live.folder = Some(meeting_parts::inputs(&root).join(&m.id));
                        live.session = Some(m);
                        live.notes_loaded = true;
                        live.saved.clear();
                    }
                    Err(_) => {
                        self.hex.error =
                            Some("Could not save meeting parts. Pause is unavailable.");
                    }
                }
            } else if confirmed
                && let Some(m) = &mut live.session
                && m.pending_start_ms.is_some()
                && !m.parts.iter().any(|p| p.id == id)
                && m.resume(&root, &id, start).is_ok()
            {
                live.phase = Phase::Recording;
            }
        }
        live.parts_checked = Some(Instant::now());
        if let Some(m) = &mut live.session {
            if !m.parts.is_empty() {
                live.start_failed = false;
            }
            if m.requested_parts > m.notes_parts && m.notes_ready(&root).unwrap_or(false) {
                m.notes_parts = m.requested_parts;
                let _ = m.save(&root);
            }
            let _ = m.refresh(&hex);
            if let Some(part) = m.parts.last_mut()
                && active_id.as_deref() == Some(&part.id)
                && let Some(elapsed) = elapsed
            {
                part.duration_ms = part.duration_ms.max(elapsed);
                let _ = m.save(&root);
            }
            let lines = m.live_lines(&hex);
            let mut feed = live.feed.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(saved) = self.meetings.iter_mut().find(|saved| saved.id == m.id) {
                saved.minutes = Some(m.elapsed_ms().div_ceil(60_000));
                if let Ok(text) = m.view_text(&hex) {
                    saved.lines = core::parse_transcript(&text);
                    saved.transcript_raw = text;
                }
            }
            feed.lines = lines;
            feed.version += 1;
        }
        if live.recovering_part && status_known {
            live.recovering_part = false;
            if active_id.is_some()
                && live
                    .session
                    .as_ref()
                    .and_then(|m| m.parts.last())
                    .is_some_and(|part| Some(&part.id) == active_id.as_ref())
            {
                self.hex_record_stop(active_id);
            }
        }
    }
    pub(in crate::ui) fn parts_notes_failed(&mut self) {
        if let Some(live) = &mut self.live {
            live.phase = Phase::Paused;
            if let Some(m) = &mut live.session {
                let _ = m.pause(&core::state_dir());
            }
        }
    }
    pub(super) fn parts_drop_unstarted(&mut self, closing: bool) -> bool {
        if self.hex_stop_pending() {
            return false;
        }
        let Some(live) = &mut self.live else {
            return false;
        };
        let text = self.live_notes.text.as_str();
        if !live.start_failed || !live.can_close(text) || (!closing && live.saved != text) {
            return false;
        }
        let Some(m) = &live.session else {
            return false;
        };
        let draft = if live.saved != text {
            Draft::Recovery
        } else if text.is_empty() {
            Draft::Empty
        } else {
            Draft::Primary
        };
        match meeting_parts::discard_pending(&core::state_dir(), &m.id, draft) {
            Ok(true) => {
                self.live = None;
                self.live_open = false;
                self.request_refresh();
                true
            }
            Ok(false) | Err(_) => {
                live.saved.clear();
                live.edited_at = Some(Instant::now());
                live.saving = Saving::Failed("Could not finish saving this meeting.".into());
                false
            }
        }
    }
    pub(in crate::ui) fn parts_start_failed(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        let Some(m) = &mut live.session else {
            if self.live_close() {
                self.live = None;
                self.live_open = false;
            }
            return;
        };
        if m.parts.is_empty() {
            live.start_failed = true;
            self.live_flush();
            self.parts_drop_unstarted(false);
            return;
        }
        // A failed Resume returns the meeting to Paused with its parts intact.
        live.phase = Phase::Paused;
        m.pending_start_ms = None;
        let _ = m.save(&core::state_dir());
    }
    pub(in crate::ui) fn parts_paused(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        let Some(m) = &mut live.session else {
            return;
        };
        let _ = m.refresh(&core::meetings_dir());
        live.pause_save_pending = m.pause(&core::state_dir()).is_err();
        if live.pause_save_pending {
            self.hex.error = Some("Could not save paused meeting.");
            return;
        }
        live.phase = Phase::Paused;
        live.stopped = Some(Instant::now());
        self.request_refresh();
    }
    pub(super) fn retry_paused_notes(&mut self) {
        if self.live.as_ref().is_some_and(|l| l.pause_save_pending) {
            self.parts_paused();
        }
        if self
            .live
            .as_ref()
            .is_some_and(|l| l.phase == Phase::Paused && !l.pause_save_pending)
        {
            self.generate_live_notes();
        }
    }
    pub(in crate::ui) fn meeting_pause(&mut self) {
        self.pause_meeting(false);
    }
    fn pause_meeting(&mut self, generate_notes: bool) {
        if self.hex_stop_pending() {
            return;
        }
        // Try to persist ownership before Stop can produce a completion event.
        self.parts_sync();
        if !self.hex_recording()
            && self
                .live
                .as_ref()
                .and_then(|l| l.session.as_ref())
                .is_none()
        {
            return;
        }
        self.hex_submit_stop(hex::recording_id(self), generate_notes);
    }
    fn parts_select(&mut self) -> bool {
        if let Some(live) = &self.live {
            if self
                .selected
                .as_ref()
                .is_none_or(|id| live.session.as_ref().is_some_and(|m| &m.id == id))
            {
                return true;
            }
            if self.hex_recording() || !self.live_close() {
                return false;
            }
        }
        let Some(id) = self.selected.clone() else {
            return false;
        };
        let root = core::state_dir();
        let Ok(Some(m)) = meeting_parts::read(&root, &id) else {
            return false;
        };
        let mut live = Live::new(Phase::Paused, self.known_ids());
        if let Some(at) = m
            .parts
            .first()
            .and_then(|p| DateTime::from_timestamp_millis(p.started_ms))
        {
            live.wall_start = at.with_timezone(&Local);
            live.exact = true;
        }
        if let Some(saved) = self.meetings.iter().find(|saved| saved.id == id) {
            *live.quick_title.lock().unwrap_or_else(|e| e.into_inner()) = Some(saved.title.clone());
        }
        live.folder = Some(meeting_parts::inputs(&root).join(&id));
        self.live_notes = TextEdit::new(feed_io::read_notes(live.folder.as_ref().unwrap()));
        live.saved = self.live_notes.text.clone();
        live.seen = live.saved.clone();
        live.notes_loaded = true;
        let mut m = m;
        if m.phase == SavedPhase::Ended && m.notes_ready(&root).unwrap_or(false) {
            m.notes_parts = m.requested_parts;
        }
        live.session = Some(m);
        self.live = Some(live);
        true
    }
    pub(in crate::ui) fn meeting_resume(&mut self) {
        if self.updates.applying || self.hex_stop_pending() {
            return;
        }
        if let Some(live) = &mut self.live {
            live.generate_after_pause = false;
            live.pause_save_pending = false;
        }
        if self.hex_status_unknown() {
            self.hex_recheck_resume();
            return;
        }
        if let Some(reason) = hex::start_blocked(self) {
            self.hex.error = Some(reason);
            return;
        }
        let root = core::state_dir();
        let show_live = self.current().is_some_and(|meeting| !meeting.has_summary());
        if !self.parts_select() {
            return;
        }
        let Some(live) = &mut self.live else {
            return;
        };
        let Some(m) = &mut live.session else {
            return;
        };
        if m.pending_start_ms.is_some() {
            return;
        }
        m.pending_start_ms = Some(chrono::Utc::now().timestamp_millis());
        if m.save(&root).is_err() {
            self.hex.error = Some("Could not save meeting before Resume.");
            return;
        }
        if show_live {
            self.go_home();
            self.live_open = true;
        }
        // Keep the page and its notes; the next status supplies the new Hex part id.
        self.hex_record_start();
    }
    pub(in crate::ui) fn meeting_generate(&mut self) {
        if self.hex_stop_pending() {
            return;
        }
        if !self.parts_select() {
            return;
        }
        if self.hex_recording() {
            self.pause_meeting(true);
            return;
        }
        self.generate_live_notes();
    }

    fn track_notes_request(&mut self, pending: &std::sync::Weak<()>) {
        self.live_notes_requests
            .retain(|request| request.strong_count() > 0);
        self.live_notes_requests.push(pending.clone());
    }

    fn generate_live_notes(&mut self) {
        if self.updates.applying {
            return;
        }
        let Some(live) = &mut self.live else {
            return;
        };
        let Some(m) = &mut live.session else {
            return;
        };
        if live.notes_request.strong_count() > 0 {
            return;
        }
        let root = core::state_dir();
        let mut prepared = m.clone();
        let preparation = if m.phase != SavedPhase::Ended {
            prepared.prepare_notes(&root, &core::meetings_dir())
        } else if live.generate_after_pause {
            // Retry submitting input that this request already prepared.
            Ok(meeting_parts::inputs(&root).join(&m.id))
        } else {
            return;
        };
        match preparation {
            Ok(folder) => {
                *m = prepared;
                live.folder = Some(folder);
                let id = m.id.clone();
                self.live_flush();
                let events = self.events_tx.clone();
                let wake = self.waker.clone();
                let request = Arc::new(());
                let pending = Arc::downgrade(&request);
                if !self.live_work(move || {
                    let result = sidenote::listener::generate_meeting(&id)
                        .map_err(|_| sidenote::hex_engine::HexError::Unsupported);
                    drop(request);
                    let _ = events.send(Event::Hex(hex::Msg::NotesRequested(result)));
                    if let Some(wake) = wake {
                        wake();
                    }
                }) {
                    self.parts_notes_failed();
                    self.hex.error = Some("Could not start notes. Try Generate notes again.");
                    return;
                }
                self.track_notes_request(&pending);
                if let Some(live) = &mut self.live {
                    live.phase = Phase::Making;
                    live.notes_request = pending;
                    live.generate_after_pause = false;
                }
                self.hex.error = None;
                self.request_refresh();
            }
            Err(error) => {
                self.hex.error = Some(if error.kind() == std::io::ErrorKind::WouldBlock {
                    "Notes are still being written. Update them when they finish."
                } else {
                    "Wait for Hex to finish all parts, then Generate notes again."
                })
            }
        }
    }
}

#[cfg(test)]
#[test]
fn switching_meetings_keeps_the_pending_summary_install_blocker() {
    crate::test_paths::isolate();
    let mut s = State::new();
    s.live = Some(Live::scene(Phase::Making, None).scene_pending("100-1"));
    let submission = Arc::new(());
    let pending = Arc::downgrade(&submission);
    s.track_notes_request(&pending);
    s.live.as_mut().unwrap().notes_request = pending;

    let root = core::state_dir();
    let mut other = Meeting::pending(&root, chrono::Utc::now().timestamp_millis()).unwrap();
    other.pending_start_ms = None;
    other.pause(&root).unwrap();
    s.selected = Some(other.id.clone());
    assert!(s.parts_select());
    let live = s.live.as_ref().unwrap();
    assert_eq!(live.session.as_ref().unwrap().id, other.id);
    let reason = Some("Waiting to queue the meeting summary.");
    assert_eq!(s.install_blocker(), reason);
    drop(submission);
    assert_eq!(s.install_blocker(), None);
}
