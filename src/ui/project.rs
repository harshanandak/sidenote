//! A meeting's project, shown as a chip on its home row and under its title. The pipeline's
//! guess (from `tasks.json`) shows as a suggestion until the owner accepts or changes it.
//! Both write the same per-task overrides the Tasks view writes, in one file write.
use super::{State, roles, theme::Palette, widgets::pressable};
use rune::{El, Key, KeyEvent, TextEdit, Tx, col, row};
use rune_kit::{Kit, Menu, MenuItem, Placement, TextInput};
use sidenote::core::{self, TaskOverride, UNSORTED};

/// What the chip shows.
#[derive(Clone)]
pub struct MeetingProject {
    /// None when every to-do is unsorted.
    pub name: Option<String>,
    /// Every to-do in that project was placed there by the owner, not only by the pipeline.
    pub confirmed: bool,
}
pub fn cache(
    projects: &core::Projects,
    pipeline: &core::Pipeline,
    overrides: &core::Overrides,
    tasks: &[core::TaskItem],
    meetings: &[core::Meeting],
) -> std::collections::BTreeMap<String, MeetingProject> {
    let mut grouped: std::collections::HashMap<&str, Vec<&core::TaskItem>> = Default::default();
    for task in tasks.iter().filter(|t| !t.manual) {
        grouped.entry(&task.meeting_id).or_default().push(task);
    }
    let mut out = std::collections::BTreeMap::new();
    for meeting in meetings {
        let id = &meeting.id;
        let value = if let Some(name) = projects.meeting_projects.get(id) {
            MeetingProject {
                name: (name != UNSORTED).then(|| projects.resolve(name)),
                confirmed: true,
            }
        } else {
            let items = grouped.get(id.as_str());
            let mut candidates: Vec<_> =
                items.into_iter().flatten().map(|t| (**t).clone()).collect();
            let file = pipeline.get(id);
            let mut confirmed = std::collections::BTreeMap::new();
            for task in &mut candidates {
                let edit = overrides
                    .get(id, task.index)
                    .filter(|o| o.task.as_deref().is_none_or(|text| text == task.task));
                let overridden =
                    edit.is_some_and(|o| o.assignment.is_some() || o.project.is_some());
                *confirmed.entry(task.project.clone()).or_insert(true) &= overridden;
            }
            let name = core::dominant_project(candidates.iter()).or_else(|| {
                file.and_then(|f| f.meeting_project.as_deref())
                    .map(|n| projects.resolve(n))
                    .filter(|n| !n.is_empty() && n != UNSORTED)
            });
            MeetingProject {
                confirmed: name
                    .as_ref()
                    .is_some_and(|n| confirmed.get(n) == Some(&true)),
                name,
            }
        };
        out.insert(id.clone(), value);
    }
    out
}

impl State {
    fn suggested_task_project(&self, id: &str, index: usize) -> Option<String> {
        let file = self.tasks.pipeline.get(id)?;
        file.tasks
            .iter()
            .find(|t| t.i == index)
            .map(|t| &t.project)
            .or(file.meeting_project.as_ref())
            .map(|name| self.tasks.projects.resolve(name))
            .filter(|name| !name.is_empty() && name != UNSORTED)
    }

    /// The dominant suggestion or user choice. A text-bound override wins over the model.
    pub fn meeting_project(&self, id: &str) -> Option<MeetingProject> {
        if !self.config.features.projects {
            return None;
        }
        self.areas.meeting_projects.get(id).cloned()
    }

    /// Moves the to-dos that sit in the meeting's project (or every to-do, when none is
    /// set) to `name`. To-dos the owner filed elsewhere one by one stay where they are.
    /// Accepting a suggestion is the same call with the suggested name.
    pub fn set_meeting_project(&mut self, id: &str, name: &str) {
        if self.areas.pending.is_some() {
            self.tasks.notice =
                Some("Wait for the area change to finish before editing projects.".into());
            return;
        }
        if !self.config.features.projects {
            return;
        }
        self.project_menu = None;
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let known = self.cached_project(name);
        if name != UNSORTED && known.is_none() {
            self.start_project_form(id);
            self.project_input = TextEdit::new(name);
            return;
        }
        let name = known.unwrap_or_else(|| name.into());
        let from = self
            .meeting_project(id)
            .and_then(|m| m.name)
            .unwrap_or_else(|| UNSORTED.to_string());
        let targets: Vec<(usize, String)> = self
            .tasks
            .all
            .iter()
            .filter(|t| {
                t.meeting_id == id
                    && (t.project == from
                        || (t.project == UNSORTED
                            && self
                                .tasks
                                .overrides
                                .get(id, t.index)
                                .filter(|o| o.task.as_deref().is_none_or(|text| text == t.task))
                                .and_then(|o| o.project.as_ref())
                                .is_none()
                            && self.suggested_task_project(id, t.index).as_deref()
                                == Some(from.as_str())))
            })
            .map(|t| (t.index, t.task.clone()))
            .collect();
        let result = self.tasks.store.update_overrides(|o| {
            let per_meeting = o.tasks.entry(id.to_string()).or_default();
            for (index, task) in &targets {
                let entry = per_meeting.entry(index.to_string()).or_default();
                // An override made against other text belongs to a different to-do.
                if entry.task.as_deref().is_some_and(|t| t != task) {
                    *entry = TaskOverride::default();
                }
                entry.project = Some(name.clone());
                entry.assignment = Some(core::areas::ProjectChoice {
                    project: name.clone(),
                    revision: format!(
                        "meeting-choice-{}",
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
                    ),
                });
                entry.task = Some(task.clone());
            }
        });
        match result {
            Ok(o) => {
                self.tasks.overrides = o;
                match self.tasks.store.assign_meeting(id, &name) {
                    Ok(p) => self.tasks.projects = p,
                    Err(e) => self.tasks.notice = Some(e.to_string()),
                }
                self.rebuild_tasks();
            }
            Err(_) => self.tasks.notice = Some(super::tasks::SAVE_FAILED.into()),
        }
    }

    fn create_meeting_project(&mut self, id: &str, name: &str) {
        match self.tasks.store.add_project(name) {
            Ok(p) => {
                self.tasks.names = core::project_names(&p, &self.tasks.all);
                self.tasks.projects = p;
                self.project_form = None;
                self.set_meeting_project(id, name);
            }
            Err(e) => self.tasks.notice = Some(e.to_string()),
        }
    }

    pub fn start_project_form(&mut self, id: &str) {
        if !self.config.features.projects {
            return;
        }
        self.project_menu = None;
        let mut e = TextEdit::new("");
        e.focused = true;
        self.project_input = e;
        self.project_form = Some(id.to_string());
    }

    fn commit_project_form(&mut self) {
        if let Some(id) = self.project_form.take() {
            let name = self.project_input.text.clone();
            self.set_meeting_project(&id, &name);
        }
    }
}

/// The project chip, or None for a meeting without to-dos.
pub fn chip(s: &State, k: &Kit, id: &str, p: Palette, layer: &mut Vec<El>) -> Option<El> {
    chip_with(s, k, id, p, layer, false)
}
pub fn reading_chip(s: &State, k: &Kit, id: &str, p: Palette, layer: &mut Vec<El>) -> Option<El> {
    chip_with(s, k, id, p, layer, true)
}
fn chip_with(
    s: &State,
    k: &Kit,
    id: &str,
    p: Palette,
    layer: &mut Vec<El>,
    quiet: bool,
) -> Option<El> {
    if !s.config.features.projects {
        return None;
    }
    let mut mp = s.meeting_project(id)?;
    let target = s.areas.activity.meetings.get(id);
    if target.is_some_and(|t| t.project_id.is_none()) {
        mp.confirmed = true;
    }
    let display_name = target.map(|t| s.areas.catalog.label(t)).or_else(|| {
        mp.name.as_deref().map(|n| {
            s.areas
                .project_labels
                .get(n)
                .cloned()
                .unwrap_or_else(|| n.into())
        })
    });
    let open = s.project_menu.as_deref() == Some(id);
    let chevron = rune::icon(
        if open {
            k.glyph.chevron_up
        } else {
            k.glyph.chevron_down
        },
        16.0,
    )
    .color(p.faint);
    let new_project = !mp.confirmed
        && mp.name.as_ref().is_some_and(|name| {
            !s.tasks
                .names
                .iter()
                .any(|known| known.eq_ignore_ascii_case(name))
        });
    let guessed = mp.name.is_some() && !mp.confirmed;
    let label = match if quiet { &display_name } else { &mp.name } {
        Some(n) if guessed => format!("Suggested project: {n}. Change it"),
        Some(n) => format!("Project: {n}. Change it"),
        None => "Add this meeting to a project".into(),
    };
    let name_ink = if mp.confirmed { p.soft } else { p.ink };
    let name_style = if quiet {
        roles::FACT.bold()
    } else {
        roles::META.medium()
    };
    let trigger_row = row()
        .gap(if quiet { 6.0 } else { 4.0 })
        .items_center()
        .child(
            name_style
                .line(
                    display_name.unwrap_or_else(|| "Add to project".into()),
                    if mp.name.is_some() { name_ink } else { p.faint },
                )
                .max_w(190.0),
        )
        .children((s.selected.is_some() || s.on_live()).then_some(chevron));
    let menu_id = id.to_string();
    let trigger = pressable(trigger_row)
        .label(label)
        .min_w(0.0)
        .pl(10.0)
        .pr(6.0)
        .py(2.0)
        .radius(999.0)
        .hover(p.chip)
        .on_press(move |s: &mut State| {
            let same = s.project_menu.as_deref() == Some(menu_id.as_str());
            s.project_menu = (!same).then(|| menu_id.clone());
        });
    let trigger = if quiet {
        trigger
            .key(format!("meeting-project-{id}"))
            .h(28.0)
            .px(8.0)
            .py(0.0)
            .radius(8.0)
            .focus_ring(2.0, 1.0, p.accent)
    } else {
        trigger
    };
    let mut menu = Menu::new(format!("proj-{id}"))
        .trigger(trigger)
        .open(open)
        .placement(Placement::BottomStart)
        .min_width(240.0);
    if open {
        menu = menu
            .on_close(|s: &mut State| s.project_menu = None)
            .group_label("Choose an existing project");
        if quiet && let Some(name) = mp.name.as_ref().filter(|_| !mp.confirmed) {
            let (id, name) = (id.to_string(), name.clone());
            menu = menu.item(MenuItem::new("Use suggested project").on_select(
                move |s: &mut State| {
                    if new_project {
                        s.create_meeting_project(&id, &name);
                    } else {
                        s.set_meeting_project(&id, &name);
                    }
                },
            ));
        }
        for n in &s.tasks.names {
            let (id, n_owned) = (id.to_string(), n.clone());
            menu = menu.item(
                MenuItem::new(n.clone())
                    .checked(mp.name.as_deref() == Some(n.as_str()))
                    .on_select(move |s: &mut State| s.set_meeting_project(&id, &n_owned)),
            );
        }
        let new_id = id.to_string();
        menu = menu.item(
            MenuItem::new("New project...")
                .on_select(move |s: &mut State| s.start_project_form(&new_id)),
        );
        menu = menu.separator().group_label("Area-only work");
        for area in s.areas.catalog.areas.values() {
            let (area_id, key) = (area.id.clone(), core::areas::meeting_key(id));
            menu = menu.item(MenuItem::new(&area.name).on_select(move |s: &mut State| {
                s.project_menu = None;
                s.assign_area(key.clone(), area_id.clone());
            }));
        }
        if mp.name.is_some() {
            let id = id.to_string();
            menu = menu.separator().item(
                MenuItem::new("No project")
                    .on_select(move |s: &mut State| s.set_meeting_project(&id, UNSORTED)),
            );
        }
    }
    let menu = menu.build(k, layer).min_w(0.0);
    let mut wrap = row()
        .items_center()
        .key(format!("project-{id}"))
        .min_w(0.0)
        .max_w(420.0)
        .radius(if quiet { 8.0 } else { 12.0 })
        .child(menu.shrink());
    if guessed {
        wrap = wrap
            .border(1.0, if quiet { p.mark } else { p.line })
            .border_dash(4.0, 3.0);
    } else if !quiet {
        wrap = wrap.border(1.0, p.line);
    }
    if guessed && !quiet {
        let dismiss_id = id.to_string();
        let id = id.to_string();
        let name = mp.name.clone().unwrap_or_default();
        let accept = pressable(
            roles::META
                .bold()
                .wrap(if new_project { "Create" } else { "Add" }, p.ink),
        )
        .key(format!("project-accept-{id}"))
        .label(format!(
            "{} the project {name}",
            if new_project { "Create" } else { "Accept" }
        ))
        .help(if new_project {
            "Create this project"
        } else {
            "Accept this project"
        })
        .px(8.0)
        .radius(999.0)
        .hover(p.chip)
        .on_press(move |s: &mut State| {
            if new_project {
                s.create_meeting_project(&id, &name);
            } else {
                s.set_meeting_project(&id, &name);
            }
        });
        let dismiss = pressable(roles::meta("Dismiss", p))
            .key(format!("project-dismiss-{dismiss_id}"))
            .label("Dismiss this project suggestion")
            .px(8.0)
            .radius(999.0)
            .hover(p.chip)
            .on_press(move |s: &mut State| s.set_meeting_project(&dismiss_id, UNSORTED));
        wrap = wrap.flex_wrap().child(accept).child(dismiss);
    }
    Some(wrap)
}

fn form_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        s.project_form = None;
        tx.blur();
        return true;
    }
    false
}

/// The small form "New project..." opens, under the chip's row.
pub fn form(s: &State, k: &Kit, id: &str, p: Palette) -> Option<El> {
    if !s.config.features.projects || s.project_form.as_deref() != Some(id) {
        return None;
    }
    let input = TextInput::new(&s.project_input)
        .plain()
        .placeholder("New project name")
        .on_edit(|s: &mut State| &mut s.project_input)
        .on_submit(|s: &mut State, _| s.commit_project_form())
        .on_key(form_key)
        .height(34.0)
        .build(k)
        .key("project-input")
        .border(1.0, p.accent) // focus ring
        .radius(8.0)
        .fill(p.card)
        .grow(1.0)
        .min_w(0.0);
    let area_alias = s
        .areas
        .alias_names
        .contains(&core::project_merge::normalized_name(&s.project_input.text));
    let matches = if area_alias {
        Vec::new()
    } else {
        s.tasks.projects.matches(&s.project_input.text)
    };
    let typed = s.project_input.text.trim().to_string();
    let mut choices = col()
        .w_full()
        .gap(6.0)
        .mt(8.0)
        .child(roles::GROUP.wrap("Use an existing project", p.soft));
    if area_alias {
        choices=choices.child(roles::SMALL.wrap("This name belongs to an area. Choose Area-only work in the meeting picker, or choose a project below.",p.soft));
    }
    for name in if matches.is_empty() {
        &s.tasks.names
    } else {
        &matches
    } {
        let (id, name) = (id.to_string(), name.clone());
        choices = choices.child(super::widgets::action(name.clone(), p).on_press(
            move |s: &mut State| {
                s.project_form = None;
                s.set_meeting_project(&id, &name);
            },
        ));
    }
    if !typed.is_empty() && matches.is_empty() && !area_alias {
        let id = id.to_string();
        choices = choices.child(
            super::widgets::button(
                format!("Create {typed}"),
                "project-create",
                super::widgets::ButtonRole::Secondary,
                p,
            )
            .on_press(move |s: &mut State| s.create_meeting_project(&id, &typed)),
        );
    }
    Some(
        col()
            .w_full()
            .max_w(460.0)
            .child(
                row()
                    .w_full()
                    .max_w(460.0)
                    .gap(8.0)
                    .items_center()
                    .mt(8.0)
                    .child(input)
                    .children((matches.len() == 1).then(|| {
                        super::tasks::button("Use project", "project-save", true, p)
                            .on_press(|s: &mut State| s.commit_project_form())
                    }))
                    .child(
                        super::tasks::button("Cancel", "project-cancel", false, p)
                            .on_press(|s: &mut State| s.project_form = None),
                    ),
            )
            .child(choices),
    )
}

/// The same synthetic home row before and after the real Create / Dismiss actions.
pub fn scene(s: &mut State, action: &str) {
    s.go_meetings();
    let Some(m) = s.meetings.iter_mut().find(|m| !m.summary.todo.is_empty()) else {
        return;
    };
    let id = m.id.clone();
    m.title = "Website launch planning".into();
    let tags = m
        .summary
        .todo
        .iter()
        .enumerate()
        .map(|(i, _)| core::TaskTag {
            i,
            project: "Website launch".into(),
            new: true,
            mine: None,
        })
        .collect();
    s.tasks.pipeline.insert(
        id.clone(),
        core::TasksFile {
            meeting_project: Some("Website launch".into()),
            tasks: tags,
        },
    );
    s.rebuild_tasks();
    if action == "existing" {
        s.tasks.projects = s
            .tasks
            .store
            .add_project("Website launch")
            .expect("render existing project saved");
        s.rebuild_tasks();
    }
    match action {
        "create" => s.create_meeting_project(&id, "Website launch"),
        "dismiss" => s.set_meeting_project(&id, UNSORTED),
        _ => (),
    }
    // Reload persisted state so the after renders also exercise the storage read path.
    s.tasks.projects = s
        .tasks
        .store
        .load_projects()
        .expect("render projects readable");
    s.tasks.overrides = s
        .tasks
        .store
        .load_overrides()
        .expect("render overrides readable");
    s.rebuild_tasks();
    super::area_scene::wait(s);
    let project = s.meeting_project(&id).expect("render meeting has tasks");
    match action {
        "create" => {
            assert!(project.confirmed && project.name.as_deref() == Some("Website launch"));
            assert!(s.tasks.names.iter().any(|n| n == "Website launch"));
        }
        "dismiss" => {
            assert!(project.name.is_none());
            assert!(!s.tasks.names.iter().any(|n| n == "Website launch"));
        }
        "existing" => {
            assert!(!project.confirmed && project.name.as_deref() == Some("Website launch"));
        }
        _ => {
            assert!(!project.confirmed && project.name.as_deref() == Some("Website launch"));
            assert!(!s.tasks.names.iter().any(|n| n == "Website launch"));
        }
    }
    if action == "meeting" {
        s.selected = Some(id);
        s.load_saved();
    }
}
