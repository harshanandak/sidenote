//! Automatic settlements are distinct from the owner's memory decisions.
use super::*;
use crate::ui::{cards, icons};
use sidenote::core::{self, settle};
mod change;

fn description(s: &State, entry: &settle::Entry, reverse: bool) -> change::Change {
    let (before, after) = if reverse {
        (&entry.after, &entry.before)
    } else {
        (&entry.before, &entry.after)
    };
    change::describe(entry.kind, before, after, |id| {
        s.meetings
            .iter()
            .find(|m| m.id == entry.source_meeting_id)?
            .summary
            .todo
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.text().to_string())
    })
}
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
fn change_icon(kind: change::Icon) -> &'static rune::Icon {
    match kind {
        change::Icon::Owner => &icons::USER,
        change::Icon::Date => &icons::DIARY,
        change::Icon::Project => &icons::FOLDER,
        change::Icon::Team => &icons::PEOPLE,
        change::Icon::Waiting => &icons::HOURGLASS,
        change::Icon::Ours => &icons::DONE,
        change::Icon::Tasks => &icons::TASKS,
    }
}

impl State {
    pub fn settle_undo(&mut self, id: String, redo: bool) {
        if self.memory.settle_busy {
            return;
        }
        self.memory.settle_busy = true;
        self.memory.errors.remove("settle");
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let result = settle::Store::default().undo(&id).map_err(|e| {
                core::validation_message(&e)
                    .unwrap_or("Could not restore this change. Try again.")
                    .to_string()
            });
            let _ = events.send(crate::ui::Event::Settled { redo, result });
            if let Some(waker) = waker {
                waker();
            }
        });
    }
    pub fn settle_finished(&mut self, redo: bool, result: Result<settle::Entry, String>) {
        self.memory.settle_busy = false;
        match result {
            Ok(entry) => {
                let change = description(self, &entry, !redo);
                self.memory.settle_pending_toast = Some((
                    format!(
                        "{}: {}",
                        if redo { "Redid" } else { "Undid" },
                        change.text.split(" \u{b7} ").next().unwrap_or(&change.text)
                    ),
                    entry.id,
                    !redo,
                ));
                self.refresh_memory(false);
                self.request_refresh();
            }
            Err(error) => {
                self.memory.errors.insert("settle".into(), error);
            }
        }
    }
}
fn rows(s: &State, history: Vec<&settle::Row>, p: Palette) -> El {
    let mut card = cards::panel(p).key("settle-card");
    for (i, row) in history.into_iter().enumerate() {
        if i > 0 {
            card = card.child(hairline(p));
        }
        let entry = &row.entry;
        let title = s
            .meetings
            .iter()
            .find(|m| m.id == entry.source_meeting_id)
            .map(|m| m.title.clone())
            .unwrap_or_else(|| "Source meeting".into());
        let target = match &entry.source_offset {
            Some(at) => format!("{}@{at}", entry.source_meeting_id),
            None => entry.source_meeting_id.clone(),
        };
        let origin = format!("settled-{}", entry.id);
        let from = origin.clone();
        let when = DateTime::parse_from_rfc3339(&entry.time)
            .map(|at| at.with_timezone(&Local).format("%-d %b, %H:%M").to_string())
            .unwrap_or_default();
        let mut metadata = vec![title.clone()];
        if let Some(at) = &entry.source_offset {
            metadata.push(at.clone());
        }
        if !when.is_empty() {
            metadata.push(when);
        }
        let source = metadata.join(" \u{b7} ");
        let change = description(s, entry, false);
        let label = if entry.kind == settle::Kind::ProjectAssignment {
            &title
        } else {
            &entry.label
        };
        let action = row.undo_target.clone();
        let redo = row.undone;
        card = card.child(
            cards::row_shell()
                .key(origin)
                .items_start()
                .child(
                    cards::icon_tile(change_icon(change.icon), p).key(format!("settle-icon-{i}")),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(roles::NAME.wrap(label, p.ink).max_lines(2))
                        .child(
                            roles::SMALL
                                .wrap(sentence(&change.text), p.soft)
                                .max_lines(1)
                                .key(format!("settle-change-{i}"))
                                .help(&entry.reason),
                        )
                        .child(
                            pressable(roles::META.line(source, p.faint))
                                .key(format!("settle-source-{i}"))
                                .self_start()
                                .on_press_tx(move |s: &mut State, tx| {
                                    s.open_check_citation(&target, from.clone(), tx)
                                }),
                        ),
                )
                .child(
                    button(
                        if redo { "Redo" } else { "Undo" },
                        format!("settle-undo-{i}"),
                        false,
                        p,
                    )
                    .on_press(move |s: &mut State| s.settle_undo(action.clone(), redo)),
                ),
        );
    }
    card
}
fn error(s: &State, p: Palette) -> Option<El> {
    s.memory
        .errors
        .get("settle")
        .map(|e| roles::SMALL.wrap(e, p.warn).role("alert"))
}
pub fn section(s: &State, p: Palette) -> Option<El> {
    let recent: Vec<_> = s
        .memory
        .settled
        .iter()
        .filter(|row| {
            DateTime::parse_from_rfc3339(&row.entry.time).is_ok_and(|at| {
                let age = Local::now().signed_duration_since(at);
                age >= chrono::Duration::zero() && age <= chrono::Duration::days(7)
            })
        })
        .collect();
    if recent.is_empty() {
        return None;
    }
    Some(
        col()
            .w_full()
            .gap(10.0)
            .mt(18.0)
            .key("recently-settled")
            .child(
                row()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(roles::GROUP.wrap("Recently settled by Sidenote", p.soft))
                    .child(button("See all", "settle-see-all", false, p).on_press(
                        |s: &mut State| {
                            s.memory.settle_all = true;
                            s.memory.settle_meeting = None;
                        },
                    )),
            )
            .children(error(s, p))
            .child(rows(s, recent.into_iter().take(5).collect(), p))
            .child(roles::META.wrap("The last 7 days.", p.faint)),
    )
}
pub fn all(s: &State, p: Palette, narrow: bool) -> El {
    let history = s
        .memory
        .settled
        .iter()
        .filter(|row| {
            s.memory
                .settle_meeting
                .as_ref()
                .is_none_or(|id| &row.entry.source_meeting_id == id)
        })
        .collect();
    scroll(
        col()
            .w_full()
            .max_w(940.0)
            .px(if narrow { 16.0 } else { 40.0 })
            .pt(22.0)
            .pb(40.0)
            .gap(14.0)
            .child(
                row()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(text_title("Settled by Sidenote", 30.0, p))
                    .child(button("Back to Memory", "settle-back", false, p).on_press(
                        |s: &mut State| {
                            s.memory.settle_all = false;
                            s.memory.settle_meeting = None;
                        },
                    )),
            )
            .children(error(s, p))
            .child(rows(s, history, p)),
        "settle-all-scroll",
    )
}
pub fn meeting_footer(s: &State, m: &core::Meeting, p: Palette) -> El {
    let count = meeting_count(s, m);
    if count == 0 {
        return el();
    }
    meeting_link(m, count, p).mt(18.0)
}

pub(in crate::ui) fn meeting_count(s: &State, m: &core::Meeting) -> usize {
    if !s.config.features.memory {
        return 0;
    }
    s.memory
        .settled
        .iter()
        .filter(|row| row.entry.source_meeting_id == m.id && !row.undone)
        .count()
}

pub(in crate::ui) fn meeting_link(m: &core::Meeting, count: usize, p: Palette) -> El {
    let id = m.id.clone();
    crate::ui::widgets::link(
        format!(
            "{count} {} added to Memory \u{203a}",
            if count == 1 { "thing" } else { "things" }
        ),
        p,
    )
    .key("meeting-settle-memory")
    .self_start()
    .on_press(move |s: &mut State| {
        s.open_memory();
        s.memory.settle_all = true;
        s.memory.settle_meeting = Some(id.clone());
    })
}

/// Uses the same write-ahead entry points as the notes worker, only with invented fixture data.
pub fn scene(s: &mut State) {
    assert!(crate::ui::isolated(), "Settle scenes require isolated data");
    crate::ui::hex::scene(s, false);
    crate::ui::sync_scene::settle(s);
    let meeting = s
        .meetings
        .iter()
        .find(|m| m.id == "1800000000000-1")
        .expect("Settle fixture meeting");
    let id = meeting.id.clone();
    let title = meeting.title.clone();
    let folder = meeting.folder.clone();
    let store = settle::Store::new(s.store.root.clone(), folder.parent().unwrap().to_owned());
    if store.history().unwrap().is_empty() {
        let mut summary: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.join("english-summary.json")).unwrap())
                .unwrap();
        let tasks = summary["todo"].as_array_mut().unwrap();
        for (i, task) in tasks.iter_mut().enumerate() {
            task["owner"] = serde_json::json!(if i % 2 == 0 {
                "Alex Chen"
            } else {
                "Morgan Lee"
            });
            task["due"] = serde_json::json!("2026-11-13");
            task["due_default"] = serde_json::json!(i % 2 == 0);
        }
        let tags: Vec<_> = (0..tasks.len())
            .map(|i| serde_json::json!({"i":i,"project":"Northwind Atlas","mine":true}))
            .collect();
        store.publish_summary(&id, &summary, false).unwrap();
        store
            .publish_projects(
                &id,
                &title,
                &serde_json::json!({"meeting_project":"Northwind Atlas","tasks":tags}),
            )
            .unwrap();
    }
    s.open_memory();
    crate::ui::sync_scene::settle(s);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn change_kinds_use_the_requested_lucide_icons() {
        for (kind, name) in [
            (change::Icon::Owner, "lucide-user-round"),
            (change::Icon::Date, "lucide-calendar"),
            (change::Icon::Project, "lucide-folder"),
            (change::Icon::Team, "lucide-users-round"),
            (change::Icon::Waiting, "solar-hourglass"),
            (change::Icon::Ours, "solar-check-circle"),
        ] {
            assert_eq!(change_icon(kind).name, name);
        }
        assert_eq!(icons::STROKE_WIDTH, 2.0);
    }
}
