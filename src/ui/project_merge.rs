use super::{State, nav::Filter, roles, theme::Palette};
use crate::ui::widgets::ButtonDisabled;
use rune::{El, col, row};
use rune_kit::{Kit, Menu, MenuItem, Placement};
use sidenote::core::project_merge::MergePreview;

#[derive(Default)]
pub struct MergeUi {
    pub from: Option<String>,
    pub preview: Option<MergePreview>,
    pub error: Option<String>,
    pub menu: bool,
    pub picker: bool,
    pub pending: Option<Pending>,
    pub generation: u64,
    pub expanded: [bool; 3],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    Preview,
    Merge,
    Undo,
}
impl Pending {
    fn label(self) -> &'static str {
        match self {
            Self::Preview => "Preparing preview...",
            Self::Merge => "Merging...",
            Self::Undo => "Undoing...",
        }
    }
}
enum Job {
    Preview(String, String),
    Merge(MergePreview),
    Undo(String, String),
}
pub enum Completion {
    Preview(MergePreview),
    Changed(Box<Changed>),
}
pub struct Changed {
    projects: sidenote::core::Projects,
    selected: String,
    memory: Option<sidenote::core::memory::MemoryFile>,
}
impl Job {
    fn run(self, store: &sidenote::core::TaskStore) -> std::io::Result<Completion> {
        sidenote::core::project_merge::proof_pause(&store.root, "worker")?;
        let (projects, selected) = match self {
            Self::Preview(from, into) => {
                return store.preview_merge(&from, &into).map(Completion::Preview);
            }
            Self::Merge(preview) => (store.merge_projects(&preview)?, preview.into),
            Self::Undo(from, id) => (store.undo_project_merge(&from, &id)?, from),
        };
        let memory = sidenote::core::memory::MemoryStore::new(store.root.clone())
            .load()
            .ok();
        Ok(Completion::Changed(Box::new(Changed {
            projects,
            selected,
            memory,
        })))
    }
}
impl State {
    pub fn start_project_merge(&mut self, from: &str) {
        if !self.config.features.projects || self.project_merge.pending.is_some() {
            return;
        }
        self.tasks.rename = None;
        self.clear_project_merge();
        self.project_merge.from = Some(from.into());
        self.go_meetings();
        self.filter = Filter::Project(from.into());
        self.refresh_list();
    }
    fn clear_project_merge(&mut self) {
        let generation = self.project_merge.generation;
        self.project_merge = MergeUi {
            generation,
            ..Default::default()
        };
    }
    pub fn preview_project_merge(&mut self, into: &str) {
        let Some(from) = self.project_merge.from.clone() else {
            return;
        };
        self.launch_project_merge(Job::Preview(from, into.into()), Pending::Preview);
    }
    fn confirm_project_merge(&mut self) {
        let Some(preview) = self.project_merge.preview.clone() else {
            return;
        };
        self.launch_project_merge(Job::Merge(preview), Pending::Merge);
    }
    fn undo_project_merge(&mut self, from: &str, id: &str) {
        self.launch_project_merge(Job::Undo(from.into(), id.into()), Pending::Undo);
    }
    fn launch_project_merge(&mut self, job: Job, pending: Pending) {
        if !self.config.features.projects || self.project_merge.pending.is_some() {
            return;
        }
        self.project_merge.pending = Some(pending);
        self.project_merge.error = None;
        self.project_merge.picker = false;
        self.project_merge.menu = false;
        self.project_merge.generation += 1;
        let generation = self.project_merge.generation;
        let (store, events, waker) = (
            self.tasks.store.clone(),
            self.events_tx.clone(),
            self.waker.clone(),
        );
        if let Err(error) = std::thread::Builder::new()
            .name("project-merge".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| job.run(&store))
                    .map_err(|_| {
                        "Could not finish the project operation. Reload before trying again.".into()
                    })
                    .and_then(|result| result.map_err(|e| e.to_string()));
                let _ = events.send(super::Event::ProjectMerge {
                    generation,
                    worker: std::thread::current().id(),
                    result,
                });
                if let Some(waker) = waker {
                    waker();
                }
            })
        {
            self.project_merge.pending = None;
            self.project_merge.error = Some(error.to_string());
        }
    }
    pub fn project_merge_finished(&mut self, generation: u64, result: Result<Completion, String>) {
        if generation != self.project_merge.generation {
            return;
        }
        let pending = self.project_merge.pending.take();
        match result {
            Ok(Completion::Preview(preview)) => {
                self.project_merge.preview = Some(preview);
                self.project_merge.expanded = [false; 3];
            }
            Ok(Completion::Changed(change)) => {
                let stay =
                    self.config.features.projects && matches!(self.filter, Filter::Project(_));
                self.tasks.projects = change.projects;
                self.clear_project_merge();
                self.rebuild_tasks();
                if stay {
                    self.filter = Filter::Project(change.selected);
                    self.refresh_list();
                }
                if let Some(file) = change.memory {
                    self.memory.records = file.records;
                    self.memory.events = file.events;
                    self.memory.proposals = file
                        .proposals
                        .into_iter()
                        .filter(|p| p.status == sidenote::core::memory::ProposalStatus::Pending)
                        .collect();
                    self.memory.projects = self.tasks.projects.clone();
                    self.memory.project = self
                        .memory
                        .project
                        .as_deref()
                        .map(|p| self.tasks.projects.resolve(p));
                } else {
                    self.memory.notice = Some(
                        "Could not refresh memory. Reload it to see the current project names."
                            .into(),
                    );
                }
            }
            Err(error) => {
                self.project_merge.error = Some(error);
                if pending == Some(Pending::Merge) {
                    self.project_merge.preview = None;
                }
            }
        }
    }
}
fn button(title: impl Into<String>, key: &str, p: Palette) -> El {
    let title = title.into();
    super::widgets::button(
        title.clone(),
        key,
        super::widgets::button_role(&title, false),
        p,
    )
}
pub fn panel(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> Option<El> {
    if !s.config.features.projects {
        return None;
    }
    let Filter::Project(name) = &s.filter else {
        return None;
    };
    let busy = s.project_merge.pending.is_some();
    let start = name.clone();
    let menu_start = name.clone();
    let open = s.project_merge.menu && !busy;
    let trigger = super::widgets::menu_trigger(
        button("Project menu", "project-actions", p).button_disabled(busy),
        open,
        p,
        |s| {
            if s.project_merge.pending.is_none() {
                s.project_merge.menu = !s.project_merge.menu;
            }
        },
    );
    if open {
        let items = col().child(
            super::widgets::menu_row("Merge into...", p, None, None, None)
                .on_press(move |s: &mut State| s.start_project_merge(&menu_start)),
        );
        super::widgets::menu_surface(
            k,
            p,
            layer,
            items,
            (
                "project-actions",
                "project-actions",
                rune::Side::Bottom,
                rune::Align::Start,
                240.0,
            ),
            |s| s.project_merge.menu = false,
        );
    }
    let mut content = col().w_full().gap(10.0).mb(16.0).child(
        row()
            .gap(8.0)
            .child(
                button("Merge into...", "project-merge", p)
                    .button_disabled(busy)
                    .on_press(move |s: &mut State| s.start_project_merge(&start)),
            )
            .child(trigger),
    );
    for (from, merge) in &s.tasks.projects.merges {
        if !merge.active || s.tasks.projects.resolve(from) != *name {
            continue;
        }
        let changed = s.tasks.projects.aliases.get(from) != Some(&merge.into);
        let local = s.areas.merge_undo_available.contains(&merge.id);
        let (from, id) = (from.clone(), merge.id.clone());
        let undoing = s.project_merge.pending == Some(Pending::Undo);
        let mut card = col()
            .w_full()
            .gap(6.0)
            .px(12.0)
            .py(12.0)
            .radius(8.0)
            .fill(p.okbg)
            .child(roles::NAME.wrap(
                if changed {
                    format!("Alias changed for {from}")
                } else {
                    format!("Merged {from} into {name}")
                },
                p.ink,
            ))
            .child(roles::SMALL.wrap(
                if changed {
                    "This project was renamed after the merge. Undo will not replace that choice."
                } else {
                    "The old name is an alias. Notes and source files are preserved."
                },
                p.soft,
            ))
            .child(roles::META.wrap(
                if local {
                    "Backup saved on this device"
                } else {
                    "Undo is available on the device that wrote this merge"
                },
                p.faint,
            ));
        if local {
            card = card.child(
                button(
                    if undoing { "Undoing..." } else { "Undo merge" },
                    "project-undo",
                    p,
                )
                .self_start()
                .button_disabled(busy)
                .on_press(move |s: &mut State| s.undo_project_merge(&from, &id)),
            );
        }
        content = content.child(card);
    }
    if let Some(from) = &s.project_merge.from {
        let mut card = col()
            .w_full()
            .gap(12.0)
            .px(14.0)
            .py(14.0)
            .radius(8.0)
            .border(1.0, p.line)
            .fill(p.card)
            .child(roles::TITLE.wrap(format!("Merge {from}"), p.ink));
        let choice = s
            .project_merge
            .preview
            .as_ref()
            .map(|v| v.into.as_str())
            .unwrap_or("Choose a project");
        let mut picker = Menu::new("merge-survivor")
            .trigger(
                button(choice, "merge-survivor", p)
                    .button_disabled(busy)
                    .on_press(|s: &mut State| {
                        if s.project_merge.pending.is_none() {
                            s.project_merge.picker = !s.project_merge.picker;
                        }
                    }),
            )
            .open(s.project_merge.picker && !busy)
            .placement(Placement::BottomStart);
        if s.project_merge.picker && !busy {
            picker = picker.on_close(|s: &mut State| s.project_merge.picker = false);
            for into in &s.tasks.names {
                if into == from {
                    continue;
                }
                let into = into.clone();
                picker = picker.item(
                    MenuItem::new(&into)
                        .checked(choice == into)
                        .on_select(move |s: &mut State| s.preview_project_merge(&into)),
                );
            }
        }
        card = card.child(
            row()
                .items_center()
                .gap(8.0)
                .child(roles::CONTROL.wrap("Merge into:", p.soft))
                .child(picker.build(k, layer)),
        );
        if let Some(pending) = s.project_merge.pending {
            card = card.child(roles::SMALL.wrap(pending.label(), p.soft).role("status"));
        }
        if let Some(preview) = &s.project_merge.preview {
            card = card.child(roles::SMALL.wrap("The old name becomes an alias. A dated, readable backup is saved before applying. Undo remains available after restart.", p.soft));
            for (index, (label, items)) in [
                ("Meetings", &preview.meetings),
                ("Tasks", &preview.tasks),
                ("Memory", &preview.memory),
            ]
            .into_iter()
            .enumerate()
            {
                let expanded = s.project_merge.expanded[index];
                card = card.child(
                    row()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .child(roles::GROUP.wrap(format!("{label} {}", items.len()), p.soft))
                        .child(
                            button(
                                if expanded { "Hide items" } else { "Show items" },
                                &format!("merge-items-{index}"),
                                p,
                            )
                            .button_disabled(busy)
                            .on_press(move |s: &mut State| {
                                if s.project_merge.pending.is_none() {
                                    s.project_merge.expanded[index] =
                                        !s.project_merge.expanded[index];
                                }
                            }),
                        ),
                );
                if expanded {
                    for item in items {
                        card = card.child(roles::SMALL.wrap(item, p.ink));
                    }
                }
            }
            let merging = s.project_merge.pending == Some(Pending::Merge);
            card = card.child(
                super::tasks::button(
                    if merging {
                        "Merging..."
                    } else {
                        "Confirm merge"
                    },
                    "project-merge-confirm",
                    true,
                    p,
                )
                .disabled(busy)
                .self_start()
                .on_press(|s: &mut State| s.confirm_project_merge()),
            );
        } else if !busy {
            card = card.child(roles::SMALL.wrap(
                "Choose the project to keep, then review the counts and affected items.",
                p.soft,
            ));
        }
        content = content.child(
            card.child(
                super::tasks::button("Cancel", "project-merge-cancel", false, p)
                    .disabled(busy)
                    .self_start()
                    .on_press(|s: &mut State| {
                        if s.project_merge.pending.is_none() {
                            s.clear_project_merge();
                        }
                    }),
            ),
        );
    }
    if let Some(error) = &s.project_merge.error {
        content = content.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    Some(content)
}

pub fn scene(s: &mut State, action: &str) {
    s.go_meetings();
    let from = "Northwind Field Guide";
    let into = "Northwind Atlas";
    s.tasks.projects = s.tasks.store.add_project(from).unwrap();
    s.tasks.projects = s.tasks.store.add_project(into).unwrap();
    let id = s.meetings[0].id.clone();
    s.set_meeting_project(&id, from);
    s.filter = Filter::Project(from.into());
    s.refresh_list();
    if action == "worker-proof" {
        proof::run(s);
        return;
    }
    if action != "page" {
        s.start_project_merge(from);
    }
    if matches!(
        action,
        "preview" | "preview-expanded" | "done" | "merging" | "undoing"
    ) {
        s.preview_project_merge(into);
        wait_for_merge(s);
        assert!(
            s.project_merge.preview.is_some(),
            "merge preview: {:?}",
            s.project_merge.error
        );
    }
    if action == "preview-expanded" {
        s.project_merge.expanded = [true; 3];
    }
    if action == "picker" {
        s.project_merge.picker = true;
    }
    if action == "done" || action == "undoing" {
        s.confirm_project_merge();
        wait_for_merge(s);
        assert!(s.project_merge.error.is_none(), "merge failed");
        s.tasks.projects = s.tasks.store.load_projects().unwrap();
        assert_eq!(s.tasks.projects.resolve(from), into);
    }
    s.project_merge.pending = match action {
        "preparing" => Some(Pending::Preview),
        "merging" => Some(Pending::Merge),
        "undoing" => Some(Pending::Undo),
        _ => None,
    };
}

#[path = "project_merge_proof.rs"]
mod proof;

fn wait_for_merge(s: &mut State) {
    while s.project_merge.pending.is_some() {
        let event = s
            .events_rx
            .recv_timeout(std::time::Duration::from_secs(15))
            .expect("merge worker finished");
        if let super::Event::ProjectMerge { worker, .. } = &event {
            assert_ne!(*worker, std::thread::current().id());
        }
        s.apply_event(event);
    }
}
