use super::{Event, State, nav, roles, theme::Palette, widgets};
use chrono::{DateTime, Local, TimeDelta, Utc};
use rune::{El, ScrollSpec, Tx, col, row};
use sidenote::core::{
    self, Meeting,
    meeting_delete::{DeleteError, Eligibility, Phase, Receipt, Storage},
};
use std::{cmp::Reverse, io, path::Path, time::Duration};

const RETENTION: TimeDelta = TimeDelta::days(30);
const UNDO_FOR: Duration = Duration::from_secs(8);
const RECYCLE_DIR: &str = "local/deleted-meetings";
const RESTORING: &str = "Restoring\u{2026}";
const RESTORE_UNFINISHED: &str = "This restore will finish when you reopen Sidenote.";
const RESTORE_REFUSED: &str = "Could not restore this meeting. It may have expired or be in use.";
const INCOMPLETE_LIST: &str = "Some deleted meetings could not be read.";

enum Dialog {
    Confirm {
        id: String,
        title: String,
        trigger: String,
    },
    Deleted {
        rows: Vec<(String, Receipt)>,
        incomplete: bool,
    },
}

type Position = (nav::Page, bool, nav::Filter);

pub(crate) enum RecycleResult {
    Deleted(String),
    Restored(Option<Box<Meeting>>),
    Failed {
        refresh: bool,
        message: &'static str,
    },
}

struct Pending {
    label: &'static str,
    position: Option<Position>,
    result: Option<RecycleResult>,
}

fn worker_failed() -> RecycleResult {
    RecycleResult::Failed {
        refresh: true,
        message: "Sidenote could not finish. Reopen Sidenote to recover any unfinished changes.",
    }
}

#[derive(Default)]
pub(super) struct DeleteUi {
    dialog: Option<Dialog>,
    error: Option<&'static str>,
    pub(super) undo: Option<String>,
    generation: u64,
    pending: Option<Pending>,
    #[cfg(test)]
    enabled_in_fixture: bool,
}

fn storage(root: &Path) -> io::Result<Storage> {
    Storage::new(&core::meetings_dir(), root)
}

pub(super) fn maintain(root: &Path) -> io::Result<()> {
    if !root.join(RECYCLE_DIR).try_exists()? {
        return Ok(());
    }
    let store = storage(root)?;
    let now = Utc::now().timestamp_millis();
    let hidden = core::meeting_delete::suppressed_ids(root)?;
    let mut result = Ok(());
    for id in store.transactions()? {
        let maintained = (|| {
            let receipt = store.receipt(&id)?;
            if matches!(receipt.phase, Phase::Deleting | Phase::Restoring)
                || receipt.phase == Phase::Restored && !receipt.ids.is_disjoint(&hidden)
            {
                store.recover(&id)?;
            }
            if matches!(
                receipt.phase,
                Phase::Deleted | Phase::Purging | Phase::Deleting
            ) {
                store.purge(&id, now)?;
            }
            Ok(())
        })();
        result = result.and(maintained);
    }
    result
}

fn remaining_days(receipt: &Receipt) -> u64 {
    let remaining = RETENTION.num_milliseconds()
        - Utc::now()
            .timestamp_millis()
            .saturating_sub(receipt.deleted_ms)
            .max(0);
    (remaining.max(0) as u64).div_ceil(TimeDelta::days(1).num_milliseconds() as u64)
}

fn restore_error(root: &Path, id: &str) -> &'static str {
    let unfinished = storage(root)
        .and_then(|store| store.receipt(id))
        .is_ok_and(|receipt| {
            receipt.phase == Phase::Restoring
                || receipt.phase == Phase::Restored
                    && core::meeting_delete::suppressed_ids(root)
                        .map_or(true, |hidden| !receipt.ids.is_disjoint(&hidden))
        });
    if unfinished {
        RESTORE_UNFINISHED
    } else {
        RESTORE_REFUSED
    }
}

fn reason(error: DeleteError) -> &'static str {
    match error {
        DeleteError::Refused(Eligibility::Recording) => {
            "Check that Hex is connected and this meeting has stopped recording, then try again."
        }
        DeleteError::Refused(Eligibility::Paused) => "End this paused meeting before deleting it.",
        DeleteError::Refused(Eligibility::Processing) => {
            "This meeting is still processing. Try again when it finishes."
        }
        DeleteError::Refused(Eligibility::UnsavedNotes) => {
            "Save your notes before deleting this meeting."
        }
        DeleteError::Refused(Eligibility::Peer) => "Delete this meeting on the recording device.",
        DeleteError::Partial { .. } => {
            "Sidenote could not finish deleting this meeting. Reopen Sidenote to recover it."
        }
        _ => "Sidenote could not delete this meeting. Try again after refreshing.",
    }
}

impl State {
    pub(super) fn delete_available(&self) -> bool {
        #[cfg(test)]
        if self.delete_ui.enabled_in_fixture {
            return true;
        }
        self.config.features.enabled(core::Feature::Delete)
    }

    /// Lets tests outside this module see the gated Deleted meetings entry.
    #[cfg(test)]
    pub(super) fn enable_delete_for_tests(&mut self) {
        self.delete_ui.enabled_in_fixture = true;
    }

    fn close_delete_dialog(&mut self) {
        if self.delete_ui.pending.is_none() {
            self.leave_delete_view();
        }
    }

    fn ask_to_delete(&mut self, id: &str, trigger: &str) {
        self.more_menu = None;
        if self.delete_ui.pending.is_some() {
            return;
        }
        self.delete_ui.error = None;
        if let Some(m) = self.meetings.iter().find(|m| m.id == id) {
            self.delete_ui.dialog = Some(Dialog::Confirm {
                id: id.into(),
                title: m.title.clone(),
                trigger: trigger.into(),
            });
        }
    }

    fn delete_refusal(&self, meeting: &Meeting) -> Option<&'static str> {
        self.live_delete_refusal(meeting)
            .map(|eligibility| reason(DeleteError::Refused(eligibility)))
            .or(self
                .edit_busy
                .then_some("Wait for your changes to save, then try again."))
    }

    fn confirm_delete(&mut self) {
        if self.delete_ui.pending.is_some() {
            return;
        }
        let Some(Dialog::Confirm { id, title, .. }) = &self.delete_ui.dialog else {
            return;
        };
        let Some(meeting) = self
            .meetings
            .iter()
            .find(|m| m.id == *id && m.title == *title)
        else {
            self.delete_ui.error = Some("This meeting changed. Close this dialog and try again.");
            return;
        };
        if let Some(refusal) = self.delete_refusal(meeting) {
            self.delete_ui.error = Some(refusal);
            return;
        }
        let meeting = meeting.clone();
        let root = self.store.root.clone();
        self.start_recycle("Deleting\u{2026}", move || {
            match std::fs::create_dir_all(root)
                .map_err(DeleteError::from)
                .and_then(|()| core::recycle_meeting(&meeting, false))
            {
                Ok(id) => RecycleResult::Deleted(id),
                Err(error) => RecycleResult::Failed {
                    refresh: matches!(error, DeleteError::Partial { .. }),
                    message: reason(error),
                },
            }
        });
    }

    fn recycle_position(&self) -> Position {
        (self.page(), self.on_live(), self.filter.clone())
    }

    pub(super) fn leave_delete_view(&mut self) {
        self.delete_ui.dialog = None;
        self.delete_ui.error = None;
        if let Some(pending) = &mut self.delete_ui.pending {
            pending.position = None;
        }
    }

    fn start_recycle(
        &mut self,
        label: &'static str,
        work: impl FnOnce() -> RecycleResult + Send + std::panic::UnwindSafe + 'static,
    ) {
        self.delete_ui.generation = self.delete_ui.generation.wrapping_add(1);
        let generation = self.delete_ui.generation;
        self.delete_ui.error = None;
        self.delete_ui.pending = Some(Pending {
            label,
            position: Some(self.recycle_position()),
            result: None,
        });
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        if std::thread::Builder::new()
            .name("meeting-recycle".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(work).unwrap_or_else(|_| worker_failed());
                let _ = events.send(Event::MeetingRecycle { generation, result });
                if let Some(waker) = waker {
                    waker();
                }
            })
            .is_err()
        {
            self.recycle_finished(generation, worker_failed());
        }
    }

    pub(super) fn recycle_finished(&mut self, generation: u64, result: RecycleResult) {
        if generation == self.delete_ui.generation
            && let Some(pending) = &mut self.delete_ui.pending
            && pending.result.is_none()
        {
            pending.result = Some(result);
        }
    }

    pub(super) fn apply_recycle_result(&mut self, tx: &mut Tx) -> bool {
        let Some(pending) = self
            .delete_ui
            .pending
            .as_mut()
            .filter(|p| p.result.is_some())
        else {
            return false;
        };
        let result = pending.result.take().unwrap();
        let position = pending.position.take();
        self.delete_ui.pending = None;
        let current = position.as_ref() == Some(&self.recycle_position());
        if !current {
            self.leave_delete_view();
        }
        match result {
            RecycleResult::Deleted(id) => {
                self.leave_delete_view();
                self.remove_deleted_from_catalog(current);
                if current {
                    self.show_toast_for("Meeting deleted", UNDO_FOR, tx);
                    self.delete_ui.undo = Some(id);
                }
            }
            RecycleResult::Restored(meeting) => {
                self.leave_delete_view();
                let restored = meeting.map(|meeting| {
                    let id = meeting.id.clone();
                    if !self.meetings.iter().any(|m| m.id == id) {
                        self.meetings.push(*meeting);
                        self.meetings.sort_by_key(|m| Reverse(m.started));
                    }
                    id
                });
                self.update_after_recycle();
                if current {
                    if let Some(id) = restored {
                        self.select(&id, tx);
                    }
                    self.show_toast("Meeting restored", tx);
                }
            }
            RecycleResult::Failed { refresh, message } => {
                if refresh {
                    self.remove_deleted_from_catalog(current);
                }
                if current {
                    if self.delete_ui.dialog.is_none() {
                        self.open_deleted();
                    }
                    self.delete_ui.error = Some(message);
                }
            }
        }
        true
    }

    fn remove_deleted_from_catalog(&mut self, current: bool) {
        let hidden = core::meeting_delete::suppressed_ids(&self.store.root);
        // An unreadable suppression catalog fails closed, matching refresh admission.
        self.meetings
            .retain(|m| hidden.as_ref().is_ok_and(|ids| !ids.contains(&m.id)));
        let keep = |t: &core::TaskItem| {
            t.manual
                || hidden
                    .as_ref()
                    .is_ok_and(|ids| !ids.contains(&t.meeting_id))
        };
        self.tasks.all.retain(keep);
        self.tasks.items.retain(keep);
        self.tasks.items_changed();
        if self.selected.is_some() && self.current().is_none() {
            if current {
                self.go_home();
            } else {
                self.selected = None;
                self.saved = Default::default();
            }
        }
        self.update_after_recycle();
    }

    fn update_after_recycle(&mut self) {
        self.cancel_stale_asks(*core::meeting_delete::read_visibility());
        self.rebuild_nav();
        self.refresh_list();
        self.rebuild_tasks();
        if self.finder.is_some() {
            self.finder_changed();
        }
        self.request_refresh();
    }

    pub(super) fn open_deleted(&mut self) {
        if self.delete_ui.pending.is_some() {
            return;
        }
        let root = &self.store.root;
        let result = (|| -> io::Result<_> {
            if !root.join(RECYCLE_DIR).try_exists()? {
                return Ok((Vec::new(), false));
            }
            let store = storage(root)?;
            let mut rows = Vec::new();
            let mut incomplete = false;
            for id in store.transactions()? {
                match store.receipt(&id) {
                    Ok(receipt) if receipt.phase == Phase::Deleted => rows.push((id, receipt)),
                    Ok(_) => (),
                    Err(_) => incomplete = true,
                }
            }
            rows.sort_by_key(|(_, receipt)| Reverse(receipt.deleted_ms));
            Ok((rows, incomplete))
        })();
        self.delete_ui.error = result
            .as_ref()
            .err()
            .map(|_| "Could not read deleted meetings.");
        let (rows, incomplete) = result.unwrap_or_default();
        self.delete_ui.dialog = Some(Dialog::Deleted { rows, incomplete });
    }

    fn restore_deleted(&mut self, id: &str) {
        if self.delete_ui.pending.is_some() {
            return;
        }
        let id = id.to_owned();
        let root = self.store.root.clone();
        self.start_recycle(RESTORING, move || {
            let result = (|| {
                let store = storage(&root)?;
                let receipt = store.receipt(&id)?;
                if remaining_days(&receipt) == 0 {
                    return Err(io::Error::other("Restore period ended"));
                }
                store.restore(&id)?;
                let (meetings, _) =
                    core::scan_catalog(&root, &core::meetings_dir(), &core::peers_dir());
                Ok(meetings
                    .into_iter()
                    .find(|m| receipt.ids.contains(&m.id))
                    .map(Box::new))
            })();
            match result {
                Ok(meeting) => RecycleResult::Restored(meeting),
                Err(_) => RecycleResult::Failed {
                    refresh: false,
                    message: restore_error(&root, &id),
                },
            }
        });
    }
}

pub(super) fn menu_row(m: &Meeting, trigger: &str, p: Palette) -> El {
    let (id, trigger) = (m.id.clone(), trigger.to_owned());
    let p = Palette { ink: p.danger, ..p };
    widgets::menu_row("Delete meeting", p, None, None, None)
        .key("delete-meeting")
        .on_press(move |s: &mut State| s.ask_to_delete(&id, &trigger))
}

pub(super) fn undo_button(s: &State, id: &str, p: Palette) -> El {
    let id = id.to_owned();
    let pending = s.delete_ui.pending.as_ref().map(|p| p.label);
    widgets::action(
        if pending == Some(RESTORING) {
            RESTORING
        } else {
            "Undo"
        },
        p,
    )
    .key("delete-undo")
    .disabled(pending.is_some())
    .on_press(move |s: &mut State| s.restore_deleted(&id))
}

pub(super) fn overlay(s: &State, p: Palette, layer: &mut Vec<El>) {
    let Some(dialog) = &s.delete_ui.dialog else {
        return;
    };
    let close = State::close_delete_dialog;
    let pending = s.delete_ui.pending.as_ref().map(|p| p.label);
    let content = col().gap(12.0);
    let (label, trigger, content) = match dialog {
        Dialog::Confirm { title, trigger, .. } => (
            "Delete meeting",
            trigger.as_str(),
            content
                .child(roles::SECTION.wrap(format!("Delete {title}?"), p.ink))
                .child(roles::READ.wrap("This meeting will leave Meetings, search, tasks and Ask on this device. You can restore it for 30 days from Deleted meetings. Folder sync may affect copies on other devices.", p.body))
                .child(roles::SMALL.wrap("Hex recordings, exports, earlier Ask answers, daily-check history and accepted memory are kept.", p.soft))
                .child(row().gap(8.0).justify_end()
                    .child(widgets::quiet_button("Cancel", "delete-cancel", p)
                        .disabled(pending.is_some()).on_press(close))
                    .child(widgets::row_button(pending.unwrap_or("Delete meeting"), "delete-confirm", widgets::ButtonRole::Destructive, p)
                        .disabled(pending.is_some()).on_press(State::confirm_delete))),
        ),
        Dialog::Deleted { rows, incomplete } => (
            "Deleted meetings",
            // The title More button outlives the menu, so focus returns there on close
            // (old: the removed quiet button's key).
            "meetings-title-more",
            content
                .child(row().gap(8.0).items_center()
                    .child(roles::SECTION.wrap("Deleted meetings", p.ink).grow(1.0))
                    .child(widgets::quiet_button("Close", "deleted-close", p)
                        .disabled(pending.is_some()).on_press(close)))
                .children(pending.map(|label| roles::SMALL.wrap(label, p.soft).role("status")))
                .children((rows.is_empty() && !incomplete && s.delete_ui.error.is_none()).then(|| roles::SMALL.wrap("No deleted meetings.", p.soft)))
                .child(col().gap(8.0).max_h(420.0).scroll(ScrollSpec { stick_bottom: false })
                    .children(rows.iter().map(|(id, receipt)| {
                        let date = DateTime::from_timestamp_millis(receipt.deleted_ms)
                            .map(|date| date.with_timezone(&Local).format("%-d %b %Y").to_string())
                            .unwrap_or_default();
                        let days = remaining_days(receipt);
                        let id = id.clone();
                        row().gap(12.0).items_center()
                            .child(col().grow(1.0).min_w(0.0)
                                .child(roles::TITLE.wrap(&receipt.title, p.ink))
                                .child(roles::META.wrap(format!("Deleted {date} | {days} {} left", if days == 1 { "day" } else { "days" }), p.soft)))
                            .child(widgets::quiet_button("Restore", &format!("restore-{id}"), p)
                                .disabled(days == 0 || pending.is_some())
                                .on_press(move |s: &mut State| s.restore_deleted(&id)))
                    })))
                .children(incomplete.then(|| roles::SMALL.wrap(INCOMPLETE_LIST, p.warn).role("status"))),
        ),
    };
    layer.push(rune_kit::scrim("meeting-delete-scrim", close).fill(p.scrim));
    layer.push(
        col()
            .key("meeting-delete-overlay")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .px(16.0)
            .items_center()
            .justify_center()
            .child(
                content
                    .key("meeting-delete-dialog")
                    .role("dialog")
                    .label(label)
                    .focus_group()
                    .focus_return(trigger)
                    .modal(true, close)
                    .w_full()
                    .max_w(480.0)
                    .padding(16.0)
                    .radius(12.0)
                    .fill(p.raised)
                    .border(1.0, p.line)
                    .shadow(12.0, 40.0, p.shadow)
                    .children(s.delete_ui.error.map(|error| {
                        roles::SMALL
                            .wrap(error, p.danger)
                            .key("meeting-delete-error")
                            .role("alert")
                    })),
            ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Event, live, refresh, theme, view};
    use rune::{Env, Runtime, text::TextSystem};
    use serde_json::json;
    use std::{
        fs,
        io::{BufRead, BufReader, Read, Write},
        net::{Ipv4Addr, TcpListener},
        process::Command,
        thread::{self, JoinHandle},
        time::Instant,
    };

    const TITLE: &str = "Synthetic workshop";
    const FLAG: &str = "SIDENOTE_DELETE_UI_CASE";

    fn in_parent(name: &str) -> bool {
        if std::env::var(FLAG).ok().as_deref() == Some(name) {
            return false;
        }
        let out = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("ui::meeting_delete::tests::{name}"),
                "--nocapture",
            ])
            .env(FLAG, name)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{name}:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        true
    }

    fn write(path: &Path, value: serde_json::Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value.to_string()).unwrap();
    }

    fn fixture() -> (String, Runtime<State>) {
        crate::test_paths::isolate();
        fs::create_dir_all(core::state_dir()).unwrap();
        let now = Utc::now().timestamp_millis();
        let id = write_meeting(now, TITLE);
        (id, runtime(State::new()))
    }

    fn write_meeting(now: i64, title: &str) -> String {
        let id = format!("{now}-1");
        let folder = core::meetings_dir().join(&id);
        write(
            &folder.join("manifest.json"),
            json!({
                "id": id, "status": "complete", "created_at_ms": now, "duration_ms": 600_000
            }),
        );
        write(
            &folder.join("english-summary.json"),
            json!({
                "title": title, "todo": [{"task": "Publish the agenda", "owner": "You"}]
            }),
        );
        fs::write(folder.join("notes.md"), "A synthetic note").unwrap();
        id
    }

    fn runtime(mut state: State) -> Runtime<State> {
        state.go_home();
        state.delete_ui.enabled_in_fixture = true;
        wait_tasks(&mut state);
        let mut rt = Runtime::with_text(
            state,
            view,
            (1334.0, 900.0),
            Env::default(),
            TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
        );
        rt.settle_all();
        rt.frame(0.0);
        rt
    }

    fn wait_tasks(state: &mut State) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while state.areas.refresh_pending {
            let event = state
                .events_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            state.apply_event(event);
        }
    }

    fn next_recycle_event(state: &mut State) -> Event {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let event = state
                .events_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("recycle worker finished");
            if matches!(event, Event::MeetingRecycle { .. }) {
                return event;
            }
            state.apply_event(event);
        }
    }

    fn poll_recycle_event(rt: &mut Runtime<State>, event: Event) {
        assert!(rt.state.events_tx.send(event).is_ok());
        rt.poll = Some(crate::ui::poll);
        rt.settle_all();
        rt.frame(0.0);
        rt.poll = None;
    }

    fn apply_recycle_event(rt: &mut Runtime<State>, event: Event) {
        poll_recycle_event(rt, event);
        assert!(
            rt.state.delete_ui.pending.is_none(),
            "the app poll must apply a completed recycle operation"
        );
    }

    fn wait_recycle(rt: &mut Runtime<State>) {
        if rt.state.delete_ui.pending.is_some() {
            let event = next_recycle_event(&mut rt.state);
            apply_recycle_event(rt, event);
        }
    }

    fn tap(rt: &mut Runtime<State>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.settle_all();
        rt.frame(0.0);
    }

    /// Opens the deleted list through the title More menu (old: the removed quiet
    /// Deleted meetings button). The fixture enables deletion, so the gated row shows.
    fn open_deleted_from_title(rt: &mut Runtime<State>) {
        tap(rt, "meetings-title-more");
        tap(rt, "meetings-open-deleted");
    }

    fn confirm_from(rt: &mut Runtime<State>, id: &str, scope: &str) {
        tap(rt, &format!("more-{scope}-{id}-button"));
        tap(rt, "delete-meeting");
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("Delete Synthetic workshop?"))
        );
    }

    // One status reply per confirm, on an OS-assigned port under this test process's roots.
    fn hex_status(status: &str, id: &str) -> JoinHandle<()> {
        hex_status_with(status, id, || {})
    }

    fn hex_status_with(
        status: &str,
        id: &str,
        before_reply: impl FnOnce() + Send + 'static,
    ) -> JoinHandle<()> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        write(
            &core::meetings_dir()
                .parent()
                .unwrap()
                .join("local-api.json"),
            json!({
                "port": listener.local_addr().unwrap().port(), "token": "fixture"
            }),
        );
        let body =
            json!({"status": "status", "version": 4, "meeting": {"state": status, "id": id}})
                .to_string();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(750)))
                .unwrap();
            let mut request = BufReader::new(&mut stream);
            let mut len = 0;
            loop {
                let mut line = String::new();
                assert_ne!(request.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.strip_prefix("Content-Length: ") {
                    len = value.trim().parse().unwrap();
                }
            }
            let mut body_bytes = vec![0; len];
            request.read_exact(&mut body_bytes).unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&body_bytes).unwrap()["command"],
                "status"
            );
            before_reply();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        })
    }

    fn hex_hash(id: &str) -> blake3::Hash {
        let inventory = storage(&core::state_dir()).unwrap().inventory(id).unwrap();
        blake3::hash(&serde_json::to_vec(&inventory.hex_inputs).unwrap())
    }

    #[test]
    fn both_menus_cancel_without_writes_and_delete_undo_rebuilds_the_catalog() {
        if in_parent("both_menus_cancel_without_writes_and_delete_undo_rebuilds_the_catalog") {
            return;
        }
        let (id, mut rt) = fixture();
        let hash = hex_hash(&id);
        rt.update(|s, tx| s.select(&id, tx));
        rt.settle_all();
        rt.frame(0.0);
        confirm_from(&mut rt, &id, "meeting");
        tap(&mut rt, "delete-cancel");
        assert_eq!(rt.state.current().unwrap().id, id);
        rt.update(|s, _| {
            s.go_home();
            s.search = rune::TextEdit::new(TITLE);
            s.refresh_list();
        });
        rt.settle_all();
        rt.frame(0.0);
        confirm_from(&mut rt, &id, "row");
        tap(&mut rt, "delete-cancel");
        assert!(
            storage(&rt.state.store.root)
                .unwrap()
                .transactions()
                .unwrap()
                .is_empty()
        );
        assert!(!rt.state.store.root.join("local/deleted-meetings").exists());
        assert_eq!(hex_hash(&id), hash);
        assert_eq!(rt.state.row_count(), 1);
        assert!(rt.state.tasks.all.iter().any(|t| t.meeting_id == id));

        confirm_from(&mut rt, &id, "row");
        let hex = hex_status("idle", &id);
        tap(&mut rt, "delete-confirm");
        wait_recycle(&mut rt);
        assert!(rt.find("delete-undo").is_some());
        hex.join().unwrap();
        assert!(rt.state.meetings.is_empty());
        assert!(core::search(&rt.state.meetings, TITLE).is_empty());
        assert_eq!(rt.state.row_count(), 0);
        assert!(rt.state.tasks.all.iter().all(|t| t.meeting_id != id));
        assert_eq!(hex_hash(&id), hash);

        tap(&mut rt, "delete-undo");

        wait_recycle(&mut rt);
        assert_eq!(rt.state.current().unwrap().id, id);
        wait_tasks(&mut rt.state);
        assert!(rt.state.tasks.all.iter().any(|t| t.meeting_id == id));
        assert_eq!(hex_hash(&id), hash);
    }

    #[test]
    fn reload_shows_the_deleted_meeting_and_the_restore_button_reopens_it() {
        if in_parent("reload_shows_the_deleted_meeting_and_the_restore_button_reopens_it") {
            return;
        }
        let (id, mut rt) = fixture();
        let hash = hex_hash(&id);
        confirm_from(&mut rt, &id, "row");
        let hex = hex_status("idle", &id);
        tap(&mut rt, "delete-confirm");
        wait_recycle(&mut rt);
        let transaction = rt.state.delete_ui.undo.clone().unwrap();
        hex.join().unwrap();
        drop(rt);

        let mut rt = runtime(State::new());
        assert!(rt.state.meetings.is_empty());
        open_deleted_from_title(&mut rt);
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some(TITLE))
        );
        tap(&mut rt, &format!("restore-{transaction}"));
        wait_recycle(&mut rt);
        assert_eq!(rt.state.current().unwrap().id, id);
        assert_eq!(hex_hash(&id), hash);
    }

    #[test]
    fn confirm_rechecks_busy_meetings_and_preserves_the_notes_editor() {
        if in_parent("confirm_rechecks_busy_meetings_and_preserves_the_notes_editor") {
            return;
        }
        let (id, mut rt) = fixture();
        confirm_from(&mut rt, &id, "row");
        for (phase, saving, text, expected) in [
            (
                live::Phase::Starting,
                live::Saving::Quiet,
                "Draft",
                Eligibility::Recording,
            ),
            (
                live::Phase::Recording,
                live::Saving::Quiet,
                "",
                Eligibility::Recording,
            ),
            (
                live::Phase::Paused,
                live::Saving::Quiet,
                "",
                Eligibility::Paused,
            ),
            (
                live::Phase::Making,
                live::Saving::Quiet,
                "",
                Eligibility::Processing,
            ),
            (
                live::Phase::Making,
                live::Saving::Quiet,
                "Draft",
                Eligibility::UnsavedNotes,
            ),
            (
                live::Phase::Making,
                live::Saving::Pending,
                "",
                Eligibility::UnsavedNotes,
            ),
            (
                live::Phase::Making,
                live::Saving::Failed("Test".into()),
                "Draft",
                Eligibility::UnsavedNotes,
            ),
        ] {
            let mut live = live::Live::scene(phase, None).scene_pending(&id);
            live.saving = saving;
            rt.state.live = Some(live);
            rt.state.live_notes = rune::TextEdit::new(text);
            tap(&mut rt, "delete-confirm");
            wait_recycle(&mut rt);
            assert_eq!(
                rt.state.delete_ui.error,
                Some(reason(DeleteError::Refused(expected)))
            );
            assert_eq!(rt.state.live_notes.text, text);
            assert!(rt.state.live.is_some());
        }
        rt.state.live = None;
        for status in ["recording", "paused", "finishing"] {
            let hex = hex_status(status, &id);
            tap(&mut rt, "delete-confirm");
            wait_recycle(&mut rt);
            assert!(rt.find("meeting-delete-error").is_some());
            hex.join().unwrap();
        }
        let queued = rt
            .state
            .store
            .root
            .join(format!("local/router-queue/{id}.json"));
        write(&queued, json!({}));
        let hex = hex_status("idle", &id);
        tap(&mut rt, "delete-confirm");
        wait_recycle(&mut rt);
        assert_eq!(
            rt.state.delete_ui.error,
            Some(reason(DeleteError::Refused(Eligibility::Processing)))
        );
        hex.join().unwrap();
        fs::remove_file(queued).unwrap();
        let notes_job = rt
            .state
            .store
            .root
            .join(format!("local/notes-jobs/{id}.json"));
        write(
            &notes_job,
            json!({
                "state": "running", "attempts": 0, "model": "", "instructions": "",
                "updated_at": Utc::now().timestamp_millis(), "message": ""
            }),
        );
        let hex = hex_status("idle", &id);
        tap(&mut rt, "delete-confirm");
        wait_recycle(&mut rt);
        assert_eq!(
            rt.state.delete_ui.error,
            Some(reason(DeleteError::Refused(Eligibility::Processing)))
        );
        hex.join().unwrap();
        fs::remove_file(notes_job).unwrap();
        rt.state.meetings[0].device = Some("Synthetic peer".into());
        tap(&mut rt, "delete-confirm");
        wait_recycle(&mut rt);
        assert_eq!(
            rt.state.delete_ui.error,
            Some(reason(DeleteError::Refused(Eligibility::Peer)))
        );
        assert_eq!(rt.state.live_notes.text, "Draft");
        assert!(
            storage(&rt.state.store.root)
                .unwrap()
                .transactions()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn pending_confirm_runs_once_and_undo_targets_the_latest_delete() {
        if in_parent("pending_confirm_runs_once_and_undo_targets_the_latest_delete") {
            return;
        }
        let (first, rt) = fixture();
        let second = write_meeting(Utc::now().timestamp_millis() - 1000, "Second workshop");
        drop(rt);
        let mut rt = runtime(State::new());
        rt.update(|s, tx| s.select(&first, tx));
        rt.settle_all();
        rt.frame(0.0);
        confirm_from(&mut rt, &first, "meeting");

        let (release, held) = std::sync::mpsc::channel();
        let hex = hex_status_with("idle", &first, move || {
            held.recv_timeout(Duration::from_millis(500)).unwrap();
        });
        rt.update(|s, _| s.confirm_delete());
        let generation = rt.state.delete_ui.generation;
        assert!(rt.state.delete_ui.pending.is_some());
        rt.update(|s, _| s.confirm_delete());
        assert_eq!(rt.state.delete_ui.generation, generation);
        assert!(
            storage(&rt.state.store.root)
                .unwrap()
                .transactions()
                .unwrap()
                .is_empty()
        );
        release.send(()).unwrap();
        let completed = next_recycle_event(&mut rt.state);
        hex.join().unwrap();
        let Event::MeetingRecycle {
            generation: first_generation,
            result: RecycleResult::Deleted(first_transaction),
        } = &completed
        else {
            panic!("first worker did not report a deletion");
        };
        assert_eq!(*first_generation, generation);
        let first_transaction = first_transaction.clone();

        let options = rt.state.refresh_options();
        rt.state.apply_event(Event::Refreshed(refresh::Result {
            generation: core::mutation_generation(),
            options,
            data: Some(Box::new(refresh::Data::read(options))),
        }));
        assert!(
            rt.state.selected.is_none(),
            "refresh dropped the deleted selection"
        );
        apply_recycle_event(&mut rt, completed);

        assert!(rt.find("delete-undo").is_some());
        assert_eq!(
            rt.state.delete_ui.undo.as_deref(),
            Some(first_transaction.as_str())
        );

        tap(&mut rt, &format!("more-row-{second}-button"));
        tap(&mut rt, "delete-meeting");
        let (release, held) = std::sync::mpsc::channel();
        let hex = hex_status_with("idle", &second, move || {
            held.recv_timeout(Duration::from_millis(500)).unwrap();
        });
        rt.update(|s, _| s.confirm_delete());
        let newer_generation = rt.state.delete_ui.generation;
        assert_ne!(newer_generation, generation);

        poll_recycle_event(
            &mut rt,
            Event::MeetingRecycle {
                generation,
                result: RecycleResult::Deleted(first_transaction.clone()),
            },
        );
        let pending = rt
            .state
            .delete_ui
            .pending
            .as_ref()
            .expect("the old result must leave the newer operation pending");
        assert!(pending.result.is_none());
        assert_eq!(rt.state.delete_ui.generation, newer_generation);
        assert_eq!(
            rt.state.delete_ui.undo.as_deref(),
            Some(first_transaction.as_str())
        );
        assert!(rt.state.meetings.iter().any(|meeting| meeting.id == second));

        release.send(()).unwrap();
        let completed = next_recycle_event(&mut rt.state);
        hex.join().unwrap();
        let Event::MeetingRecycle {
            generation: second_generation,
            result: RecycleResult::Deleted(second_transaction),
        } = &completed
        else {
            panic!("second worker did not report a deletion");
        };
        assert_eq!(*second_generation, newer_generation);
        let second_transaction = second_transaction.clone();
        apply_recycle_event(&mut rt, completed);
        assert!(rt.state.meetings.is_empty());
        assert_eq!(
            storage(&rt.state.store.root)
                .unwrap()
                .transactions()
                .unwrap()
                .len(),
            2
        );
        assert_ne!(second_transaction, first_transaction);
        assert_eq!(
            rt.state.delete_ui.undo.as_deref(),
            Some(second_transaction.as_str())
        );
        tap(&mut rt, "delete-undo");
        wait_recycle(&mut rt);
        assert_eq!(rt.state.current().unwrap().id, second);
        assert!(
            core::meeting_delete::suppressed_ids(&rt.state.store.root)
                .unwrap()
                .contains(&first)
        );

        let store = storage(&rt.state.store.root).unwrap();
        assert_eq!(
            store.receipt(&first_transaction).unwrap().phase,
            Phase::Deleted
        );
        assert_eq!(
            store.receipt(&second_transaction).unwrap().phase,
            Phase::Restored
        );
        tap(&mut rt, "meeting-breadcrumb-parent");
        assert!(rt.state.selected.is_none());
        open_deleted_from_title(&mut rt);
        let Some(Dialog::Deleted { rows, .. }) = &rt.state.delete_ui.dialog else {
            panic!("the title menu must open the deleted list");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, first_transaction);
        tap(&mut rt, &format!("restore-{first_transaction}"));
        wait_recycle(&mut rt);
        assert_eq!(rt.state.current().unwrap().id, first);
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("Meeting restored"))
        );
        assert_eq!(rt.state.meetings.len(), 2);
    }

    #[test]
    fn late_delete_result_updates_catalog_without_taking_over_navigation() {
        if in_parent("late_delete_result_updates_catalog_without_taking_over_navigation") {
            return;
        }
        let (id, rt) = fixture();
        let other = write_meeting(Utc::now().timestamp_millis() - 1000, "Other workshop");
        drop(rt);
        let mut rt = runtime(State::new());
        confirm_from(&mut rt, &id, "row");
        let hex = hex_status("idle", &id);
        rt.update(|s, _| s.confirm_delete());
        assert!(rt.state.delete_ui.pending.is_some());
        rt.update(|s, tx| s.select(&other, tx));
        wait_recycle(&mut rt);
        hex.join().unwrap();
        assert_eq!(rt.state.current().unwrap().id, other);
        assert!(rt.state.delete_ui.dialog.is_none());
        assert!(rt.state.delete_ui.undo.is_none());
        assert_eq!(rt.state.meetings.len(), 1);
        assert!(
            core::meeting_delete::suppressed_ids(&rt.state.store.root)
                .unwrap()
                .contains(&id)
        );
    }

    #[test]
    fn a_late_refresh_is_ignored_and_an_unrelated_live_draft_survives() {
        if in_parent("a_late_refresh_is_ignored_and_an_unrelated_live_draft_survives") {
            return;
        }
        let (id, mut rt) = fixture();
        confirm_from(&mut rt, &id, "row");
        let options = rt.state.refresh_options();
        let generation = core::mutation_generation();
        let mut data = refresh::Data::read(options);
        data.errors.push("Stale refresh was applied".into());
        rt.state.live =
            Some(live::Live::scene(live::Phase::Recording, None).scene_pending("123-2"));
        rt.state.live_open = true;
        rt.state.live_notes = rune::TextEdit::new("Unrelated live notes");
        let hex = hex_status("recording", "123-2");
        tap(&mut rt, "delete-confirm");
        wait_recycle(&mut rt);
        assert!(rt.find("delete-undo").is_some());
        hex.join().unwrap();
        rt.state.apply_event(Event::Refreshed(refresh::Result {
            generation,
            options,
            data: Some(Box::new(data)),
        }));
        assert!(rt.state.meetings.is_empty());
        assert!(
            !rt.state
                .scan_errors
                .iter()
                .any(|e| e == "Stale refresh was applied")
        );
        assert!(rt.state.on_live());
        assert_eq!(rt.state.live_notes.text, "Unrelated live notes");
    }

    fn seed_deleted(store: &Storage, id: &str, title: &str) -> String {
        store
            .delete(
                id,
                title.into(),
                Utc::now().timestamp_millis(),
                |_| Ok(Eligibility::Ready),
                |_| Ok(()),
            )
            .unwrap()
    }

    fn has_text(rt: &Runtime<State>, text: &str) -> bool {
        rt.elements()
            .iter()
            .any(|e| e.text.as_deref() == Some(text))
    }

    #[test]
    fn open_finder_tracks_delete_and_restore_completions() {
        if in_parent("open_finder_tracks_delete_and_restore_completions") {
            return;
        }
        let (id, mut rt) = fixture();
        confirm_from(&mut rt, &id, "row");
        let hex = hex_status("idle", &id);
        tap(&mut rt, "delete-confirm");
        rt.update(|s, _| s.open_finder());
        rt.settle_all();
        rt.frame(0.0);
        assert!(has_text(&rt, "Recent meetings"));
        wait_recycle(&mut rt);
        hex.join().unwrap();
        assert!(rt.state.finder.is_some());
        assert!(!has_text(&rt, "Recent meetings"));

        let transaction = rt.state.delete_ui.undo.clone().unwrap();
        rt.update(|s, _| s.restore_deleted(&transaction));
        wait_recycle(&mut rt);
        assert!(rt.state.finder.is_some());
        assert!(has_text(&rt, "Recent meetings"));
    }

    #[test]
    fn unreadable_receipt_keeps_readable_deleted_rows() {
        if in_parent("unreadable_receipt_keeps_readable_deleted_rows") {
            return;
        }
        let (id, mut rt) = fixture();
        let now = Utc::now().timestamp_millis();
        let second = write_meeting(now - 1000, "Second workshop");
        let third = write_meeting(now - 2000, "Third workshop");
        let root = &rt.state.store.root;
        let store = storage(root).unwrap();
        let first = seed_deleted(&store, &id, TITLE);
        let second = seed_deleted(&store, &second, "Second workshop");
        let bad = seed_deleted(&store, &third, "Third workshop");
        fs::write(root.join(RECYCLE_DIR).join(&bad).join("receipt.json"), "{").unwrap();

        open_deleted_from_title(&mut rt);
        assert!(has_text(&rt, INCOMPLETE_LIST));
        assert!(rt.find(&format!("restore-{first}")).is_some());
        assert!(rt.find(&format!("restore-{second}")).is_some());
        assert!(rt.find(&format!("restore-{bad}")).is_none());
    }

    #[test]
    fn restore_error_explains_when_reopening_finishes_the_restore() {
        use sidenote::core::meeting_delete::{MoveEvent, writer_claim};

        if in_parent("restore_error_explains_when_reopening_finishes_the_restore") {
            return;
        }
        let (id, mut rt) = fixture();
        let root = rt.state.store.root.clone();
        let store = storage(&root).unwrap();
        let transaction = seed_deleted(&store, &id, TITLE);
        open_deleted_from_title(&mut rt);
        let restore = format!("restore-{transaction}");
        for (phase, message, interruption) in [
            (Phase::Deleted, RESTORE_REFUSED, None),
            (
                Phase::Restoring,
                RESTORE_UNFINISHED,
                Some(MoveEvent::Receipt),
            ),
            (
                Phase::Restored,
                RESTORE_UNFINISHED,
                Some(MoveEvent::Finished(Phase::Restored)),
            ),
        ] {
            if let Some(interruption) = interruption {
                assert!(
                    store
                        .restore_checked(&transaction, |event| {
                            if event == interruption {
                                return Err(io::Error::other("Interrupted restore"));
                            }
                            Ok(())
                        })
                        .is_err()
                );
            }
            assert_eq!(store.receipt(&transaction).unwrap().phase, phase);
            let claim = writer_claim(&root, &id).unwrap();
            tap(&mut rt, &restore);
            wait_recycle(&mut rt);
            assert!(has_text(&rt, message));
            drop(claim);
        }
    }

    #[test]
    fn startup_recovery_warning_survives_a_clean_refresh_until_dismissed() {
        if in_parent("startup_recovery_warning_survives_a_clean_refresh_until_dismissed") {
            return;
        }
        let (_, rt) = fixture();
        let bad = rt
            .state
            .store
            .root
            .join(RECYCLE_DIR)
            .join("0".repeat(32))
            .join("receipt.json");
        write(&bad, json!({}));
        drop(rt);
        let mut rt = runtime(State::new_with_delete_maintenance(true));
        let warning = "Some deleted meetings could not be recovered or cleared.";
        assert!(has_text(&rt, warning));

        let options = rt.state.refresh_options();
        let data = refresh::Data::read(options);
        assert!(data.errors.is_empty());
        poll_recycle_event(
            &mut rt,
            Event::Refreshed(refresh::Result {
                generation: core::mutation_generation(),
                options,
                data: Some(Box::new(data)),
            }),
        );
        assert!(has_text(&rt, warning));
        tap(&mut rt, "delete-recovery-dismiss");
        assert!(!has_text(&rt, warning));
    }

    #[test]
    fn pending_settle_toast_waits_for_delete_undo_to_expire() {
        if in_parent("pending_settle_toast_waits_for_delete_undo_to_expire") {
            return;
        }
        let (id, mut rt) = fixture();
        confirm_from(&mut rt, &id, "row");
        let hex = hex_status("idle", &id);
        tap(&mut rt, "delete-confirm");
        rt.state.memory.settle_pending_toast =
            Some(("Memory settled".into(), "fixture".into(), false));
        wait_recycle(&mut rt);
        hex.join().unwrap();
        assert!(rt.find("delete-undo").is_some());
        assert!(rt.state.memory.settle_pending_toast.is_some());

        rt.state.toast.as_mut().unwrap().1 = Instant::now();
        rt.poll = Some(crate::ui::poll);
        rt.frame(0.0);
        assert!(rt.state.memory.settle_pending_toast.is_none());
        assert!(has_text(&rt, "Memory settled"));
        assert!(rt.find("settle-toast-action").is_some());
    }
}
