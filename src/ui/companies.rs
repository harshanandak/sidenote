//! Companies reuse the Settings rows and the existing scoped Ask composer.
use super::{
    State, ask, cards, icons, people, roles,
    theme::Palette,
    widgets::{hairline, scroll},
};
use crate::ui::widgets::ButtonDisabled;
use rune::{El, TextEdit, col, row};
use rune_kit::{Kit, SegmentedControl, TextInput};
use sidenote::core::{
    self,
    companies::{Companies, Company},
};
use std::path::Path;

pub enum Edit {
    Save(Company),
    Link {
        person: String,
        company: Option<String>,
        email: Option<String>,
    },
    Merge {
        from: String,
        into: String,
    },
    Undo,
}
impl Edit {
    pub fn run(self, root: &Path) -> std::io::Result<Companies> {
        match self {
            Self::Save(c) => Companies::save_company(root, c),
            Self::Link {
                person,
                company,
                email,
            } => Companies::link(root, &person, company, email),
            Self::Merge { from, into } => Companies::merge(root, &from, &into),
            Self::Undo => Companies::undo(root),
        }
    }
}
#[derive(Default)]
pub struct CompanyUi {
    pub expanded_decisions: std::collections::BTreeSet<String>,
    pub open: bool,
    pub selected: Option<String>,
    pub tab: usize,
    pub data: Companies,
    pub notice: Option<String>,
    pub editing: bool,
    pub name: TextEdit,
    pub domain: TextEdit,
    pub notes: TextEdit,
    pub aliases: TextEdit,
    pub merging: bool,
    pub merge_into: Option<String>,
    pub person: Option<String>,
    pub person_company: Option<String>,
    pub email: TextEdit,
}
impl CompanyUi {
    pub fn load(root: &Path) -> Self {
        match Companies::migrate(root) {
            Ok(data) => Self { data, ..Default::default() },
            Err(_) => Self { notice: Some("Could not read companies. Your saved files are kept. Reopen Sidenote to retry.".into()), ..Default::default() },
        }
    }
}
impl State {
    pub fn open_company(&mut self, id: Option<String>) {
        if !self.config.features.people {
            return;
        }
        self.go_home();
        match Companies::load(&self.store.root) {
            Ok(data) => self.companies.data = data,
            Err(_) => {
                self.companies.notice =
                    Some("Could not refresh companies. Your saved files are kept.".into())
            }
        }
        self.companies.open = true;
        // A company removed since it was opened falls back to the list, not its old Ask thread.
        self.companies.selected =
            id.filter(|id| self.companies.data.catalog.records.contains_key(id));
        self.companies.tab = 0;
        self.companies.expanded_decisions.clear();
        self.companies.editing = false;
        self.companies.merging = false;
        self.question = TextEdit::default();
        self.load_saved();
        self.nav_done();
    }
    pub fn edit_company(&mut self) {
        let c = self
            .companies
            .selected
            .as_ref()
            .and_then(|id| self.companies.data.catalog.records.get(id))
            .cloned()
            .unwrap_or_default();
        self.companies.name = TextEdit::new(c.name);
        self.companies.domain = TextEdit::new(c.domain.unwrap_or_default());
        self.companies.notes = TextEdit::new(c.notes.unwrap_or_default());
        self.companies.aliases = TextEdit::new(c.aliases.join(", "));
        self.companies.editing = true;
        self.companies.merging = false;
    }
    pub fn company_edit(&mut self, edit: Edit) {
        if !self.config.features.people {
            return;
        }
        self.publish_edit(super::edit_worker::Job::Company {
            root: self.store.root.clone(),
            edit,
        });
    }
    pub fn edit_person_company(&mut self, person: &str) {
        let link = self
            .companies
            .data
            .catalog
            .people
            .get(&core::people::key(person));
        self.companies.person = Some(person.into());
        self.companies.person_company = link.and_then(|l| l.company_id.clone());
        self.companies.email =
            TextEdit::new(link.and_then(|l| l.email.clone()).unwrap_or_default());
    }
}
fn button(label: &str, key: &str, p: Palette) -> El {
    super::tasks::button(label, key, false, p)
}
fn editor(s: &State, k: &Kit, p: Palette) -> El {
    cards::panel(p).key("company-editor").child(
        col()
            .padding(14.0)
            .gap(8.0)
            .w_full()
            .child(roles::NAME.line("Edit company", p.ink))
            .child(
                TextInput::new(&s.companies.name)
                    .label("Company name")
                    .placeholder("Company name")
                    .on_edit(|s: &mut State| &mut s.companies.name)
                    .disabled(s.edit_busy)
                    .build(k)
                    .key("company-name")
                    .w_full(),
            )
            .child(
                TextInput::new(&s.companies.domain)
                    .label("Domain (optional)")
                    .placeholder("Domain (optional)")
                    .on_edit(|s: &mut State| &mut s.companies.domain)
                    .disabled(s.edit_busy)
                    .build(k)
                    .key("company-domain")
                    .w_full(),
            )
            .child(
                TextInput::new(&s.companies.notes)
                    .label("Notes (optional)")
                    .placeholder("Notes (optional)")
                    .on_edit(|s: &mut State| &mut s.companies.notes)
                    .disabled(s.edit_busy)
                    .build(k)
                    .key("company-notes")
                    .w_full(),
            )
            .child(roles::META.wrap("Advanced: other names, separated by commas", p.soft))
            .child(
                TextInput::new(&s.companies.aliases)
                    .label("Company aliases")
                    .placeholder("Other names")
                    .on_edit(|s: &mut State| &mut s.companies.aliases)
                    .disabled(s.edit_busy)
                    .build(k)
                    .key("company-aliases")
                    .w_full(),
            )
            .child(
                row()
                    .gap(8.0)
                    .child(
                        button("Save", "company-save", p)
                            .button_disabled(s.edit_busy)
                            .on_press(|s: &mut State| {
                                s.company_edit(Edit::Save(Company {
                                    id: s.companies.selected.clone().unwrap_or_default(),
                                    name: s.companies.name.text.clone(),
                                    domain: Some(s.companies.domain.text.trim().into()),
                                    notes: Some(s.companies.notes.text.clone()),
                                    aliases: s
                                        .companies
                                        .aliases
                                        .text
                                        .split(',')
                                        .map(str::trim)
                                        .filter(|a| !a.is_empty())
                                        .map(str::to_string)
                                        .collect(),
                                }));
                            }),
                    )
                    .child(
                        button("Cancel", "company-cancel", p)
                            .button_disabled(s.edit_busy)
                            .on_press(|s: &mut State| s.companies.editing = false),
                    ),
            ),
    )
}
pub fn person_editor(s: &State, k: &Kit, p: Palette) -> El {
    let mut choices = row().w_full().flex_wrap().gap(6.0).child(
        button("No company", "person-company-none", p)
            .button_disabled(s.edit_busy)
            .on_press(|s: &mut State| s.companies.person_company = None),
    );
    for c in s.companies.data.catalog.records.values() {
        let id = c.id.clone();
        choices = choices.child(
            button(&c.name, &format!("person-company-{}", c.id), p)
                .role("radio")
                .checked(s.companies.person_company.as_ref() == Some(&c.id))
                .button_disabled(s.edit_busy)
                .on_press(move |s: &mut State| s.companies.person_company = Some(id.clone())),
        );
    }
    col()
        .key("person-company-editor")
        .child(super::widgets::breadcrumb(
            "People",
            s.companies.person.as_deref().unwrap_or("Person"),
            "person-breadcrumb",
            p,
            |s| s.companies.person = None,
        ))
        .w_full()
        .padding(14.0)
        .gap(8.0)
        .child(roles::NAME.wrap(
            format!(
                "Company for {}",
                s.companies.person.as_deref().unwrap_or_default()
            ),
            p.ink,
        ))
        .child(choices)
        .child(
            TextInput::new(&s.companies.email)
                .label("Email (optional, typed by you)")
                .placeholder("Email (optional)")
                .on_edit(|s: &mut State| &mut s.companies.email)
                .disabled(s.edit_busy)
                .build(k)
                .key("person-email")
                .w_full(),
        )
        .child(roles::META.wrap(
            "Sidenote only copies email addresses. It never sends mail.",
            p.soft,
        ))
        .child(
            row()
                .gap(8.0)
                .child(
                    button("Save person", "person-company-save", p)
                        .button_disabled(s.edit_busy)
                        .on_press(|s: &mut State| {
                            if let Some(person) = s.companies.person.clone() {
                                s.company_edit(Edit::Link {
                                    person,
                                    company: s.companies.person_company.clone(),
                                    email: Some(s.companies.email.text.trim().into()),
                                });
                            }
                        }),
                )
                .child(
                    button("Cancel", "person-company-cancel", p)
                        .button_disabled(s.edit_busy)
                        .on_press(|s: &mut State| s.companies.person = None),
                ),
        )
}
fn merge_dialog(s: &State, p: Palette) -> El {
    let mut out=col().key("company-merge-dialog").role("dialog").label("Merge companies").w_full().padding(14.0).gap(10.0)
        .child(roles::SECTION.wrap("Merge companies",p.ink))
        .child(roles::SMALL.wrap("Choose the company to keep. People move with it. Names, domains and notes are kept. You can undo this merge.",p.soft));
    for c in s
        .companies
        .data
        .catalog
        .records
        .values()
        .filter(|c| Some(&c.id) != s.companies.selected.as_ref())
    {
        let id = c.id.clone();
        out = out.child(
            button(&c.name, &format!("company-merge-target-{}", c.id), p)
                .role("radio")
                .checked(s.companies.merge_into.as_ref() == Some(&c.id))
                .button_disabled(s.edit_busy)
                .on_press(move |s: &mut State| s.companies.merge_into = Some(id.clone())),
        );
    }
    if s.companies.data.catalog.records.len() < 2 {
        out = out.child(roles::SMALL.wrap("Add another company before merging.", p.soft));
    }
    out.child(
        row()
            .flex_wrap()
            .gap(8.0)
            .child(
                button("Merge into selected company", "company-merge-confirm", p)
                    .button_disabled(s.edit_busy || s.companies.merge_into.is_none())
                    .on_press(|s: &mut State| {
                        if let (Some(from), Some(into)) =
                            (s.companies.selected.clone(), s.companies.merge_into.clone())
                        {
                            s.company_edit(Edit::Merge { from, into });
                        }
                    }),
            )
            .child(
                button("Cancel", "company-merge-cancel", p)
                    .button_disabled(s.edit_busy)
                    .on_press(|s: &mut State| s.companies.merging = false),
            ),
    )
}
fn meeting_row(
    m: &core::Meeting,
    title: &str,
    icon: &'static rune::Icon,
    key: String,
    p: Palette,
) -> El {
    let id = m.id.clone();
    super::widgets::pressable(cards::row_shell().child(cards::icon_tile(icon, p)).child(
        cards::copy(
            roles::NAME.wrap(title, p.ink).max_lines(2),
            &format!(
                    "{} · {}",
                    m.title,
                    m.started
                        .map(|d| d.format("%-d %b %Y").to_string())
                        .unwrap_or_else(|| "Date unknown".into())
                ),
            p,
        ),
    ))
    .key(key)
    .label(format!("{}: {}", title, m.title))
    .on_press_tx(move |s: &mut State, tx| s.select(&id, tx))
}
fn decision_row(s: &State, group: &core::companies::DecisionGroup<'_>, i: usize, p: Palette) -> El {
    let first = group.citations[0];
    let expanded = s.companies.expanded_decisions.contains(&group.key);
    let key = group.key.clone();
    let date = first
        .started
        .map(|d| d.format("%-d %b").to_string())
        .unwrap_or_else(|| "Date unknown".into());
    let earlier = group.citations.len() - 1;
    let extra = match earlier {
        0 => String::new(),
        1 => " \u{b7} also in 1 earlier meeting".into(),
        n => format!(" \u{b7} also in {n} earlier meetings"),
    };
    let source = first.id.clone();
    let copy = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(4.0)
        .child(
            super::widgets::pressable(
                row()
                    .w_full()
                    .items_center()
                    .gap(8.0)
                    .child(
                        roles::NAME
                            .wrap(group.text, p.ink)
                            .max_lines(2)
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(icons::draw(
                        if expanded {
                            &icons::CHEVRON_DOWN
                        } else {
                            &icons::CHEVRON_RIGHT
                        },
                        16.0,
                        p.soft,
                    )),
            )
            .key(format!("company-decision-{i}"))
            .label(format!("Meeting citations for {}", group.text))
            .expanded(expanded)
            .on_press(move |s: &mut State| {
                if !s.companies.expanded_decisions.remove(&key) {
                    s.companies.expanded_decisions.insert(key.clone());
                }
            }),
        )
        .child(
            super::widgets::pressable(
                roles::META.wrap(format!("{} \u{b7} {date}{extra}", first.title), p.soft),
            )
            .key(format!("company-decision-source-{i}"))
            .label(format!("Open {}", first.title))
            .on_press_tx(move |s: &mut State, tx| s.select(&source, tx)),
        );
    let mut out = col().w_full().child(
        cards::row_shell()
            .child(cards::icon_tile(&icons::FLAG, p))
            .child(copy),
    );
    if expanded {
        for (j, meeting) in group.citations.iter().enumerate() {
            out = out.child(meeting_row(
                meeting,
                &meeting.title,
                &icons::MEETINGS,
                format!("company-decision-citation-{i}-{j}"),
                p,
            ));
        }
    }
    out
}
fn task_group(
    title: String,
    empty: String,
    tasks: &[&core::TaskItem],
    icon: &'static rune::Icon,
    p: Palette,
) -> El {
    let mut out = col()
        .w_full()
        .gap(8.0)
        .child(roles::GROUP.wrap(title, p.ink));
    if tasks.is_empty() {
        return out.child(roles::SMALL.wrap(empty, p.soft));
    }
    let mut panel = cards::panel(p);
    for (i, t) in tasks.iter().enumerate() {
        if i > 0 {
            panel = panel.child(hairline(p));
        }
        let id = t.meeting_id.clone();
        panel = panel.child(
            super::widgets::pressable(cards::row_shell().child(cards::icon_tile(icon, p)).child(
                cards::copy(
                    roles::NAME.wrap(&t.title, p.ink).max_lines(2),
                    &format!(
                        "{} · {}",
                        t.owner.as_deref().unwrap_or("No owner"),
                        t.meeting_title
                    ),
                    p,
                ),
            ))
            .key(format!("company-task-{}-{}", t.meeting_id, t.index))
            .on_press_tx(move |s: &mut State, tx| s.select(&id, tx)),
        );
    }
    out = out.child(panel);
    out
}
pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let mut doc = col()
        .w_full()
        .min_w(0.0)
        .max_w(984.0)
        .padding(if narrow { 16.0 } else { 24.0 })
        .gap(12.0);
    if let Some(message) = &s.companies.notice {
        doc = doc.child(roles::SMALL.wrap(message, p.warn).role("alert"));
    }
    if let Some(undo) = &s.companies.data.undo {
        doc = doc.child(
            row()
                .flex_wrap()
                .gap(8.0)
                .child(roles::META.wrap(format!("{} saved", undo.label), p.soft))
                .child(
                    crate::ui::widgets::action("Undo", p)
                        .key("company-undo")
                        .button_disabled(s.edit_busy)
                        .on_press(|s: &mut State| s.company_edit(Edit::Undo)),
                ),
        );
    }
    let company = s
        .companies
        .selected
        .as_ref()
        .and_then(|id| s.companies.data.catalog.records.get(id));
    let Some(c) = company else {
        doc = doc.child(roles::SECTION.wrap("Companies", p.ink)).child(
            button("Add company", "company-add", p)
                .button_disabled(s.edit_busy)
                .on_press(|s: &mut State| {
                    s.companies.selected = None;
                    s.edit_company();
                }),
        );
        if s.companies.editing {
            doc = doc.child(editor(s, k, p));
        }
        if s.companies.data.catalog.records.is_empty() {
            doc = doc.child(roles::SMALL.wrap(
                "No companies yet. Add a company, then choose it for a person in People.",
                p.soft,
            ));
        } else {
            let mut panel = cards::panel(p);
            for (i, c) in s.companies.data.catalog.records.values().enumerate() {
                if i > 0 {
                    panel = panel.child(hairline(p));
                }
                let id = c.id.clone();
                panel = panel.child(
                    super::widgets::pressable(
                        cards::row_shell()
                            .child(cards::icon_tile(&icons::BUILDING_2, p))
                            .child(cards::copy(
                                roles::NAME.wrap(&c.name, p.ink).max_lines(2),
                                c.domain.as_deref().unwrap_or("Company"),
                                p,
                            )),
                    )
                    .key(format!("company-{}", c.id))
                    .on_press(move |s: &mut State| s.open_company(Some(id.clone()))),
                );
            }
            doc = doc.child(panel);
        }
        return scroll(col().w_full().items_center().child(doc), "companies-scroll");
    };
    doc = doc.child(super::widgets::breadcrumb(
        "People",
        &c.name,
        "company-breadcrumb",
        p,
        |s| s.open_people_tab(1),
    ));
    let data = core::companies::query(
        &c.id,
        &s.companies.data,
        &s.meetings,
        &s.tasks.teams,
        &s.tasks.all,
        &s.people.prefs,
        &s.people.affiliations,
    );
    doc = doc
        .child(
            row()
                .w_full()
                .items_center()
                .gap(10.0)
                .child(
                    super::list::text_title(&c.name, 32.0, p)
                        .key("company-title")
                        .max_lines(2)
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    button("More", "company-more", p)
                        .button_disabled(s.edit_busy)
                        .on_press(|s: &mut State| s.edit_company()),
                ),
        )
        .child(roles::META.wrap(
            format!(
                    "{} people \u{b7} {} meetings{} \u{b7} Last seen {} \u{b7} {} open",
                    data.people.len(),
                    data.meetings.len(),
                    c.domain
                        .as_ref()
                        .map(|d| format!(" · {d}"))
                        .unwrap_or_default(),
                    data.meetings
                        .first()
                        .and_then(|m| m.started)
                        .map(|d| d.format("%-d %b").to_string())
                        .unwrap_or_else(|| "Not yet".into()),
                    data.open.len()
                ),
            p.soft,
        ));
    if s.companies.editing {
        doc = doc.child(editor(s, k, p)).child(
            button("Merge companies", "company-merge-open", p)
                .button_disabled(s.edit_busy)
                .on_press(|s: &mut State| {
                    s.companies.editing = false;
                    s.companies.merging = true;
                    s.companies.merge_into = None;
                }),
        );
    }
    if s.companies.merging {
        doc = doc.child(cards::panel(p).child(merge_dialog(s, p)));
    }
    doc = doc.child(ask::overlay(s, k, p, narrow, layer));
    let mut shortcuts = row().key("company-shortcuts").w_full().flex_wrap().gap(6.0);
    for (i, q) in [
        "Where do things stand?",
        "What are they waiting on from us?",
        "Prep my next call",
    ]
    .iter()
    .enumerate()
    {
        let q = q.to_string();
        shortcuts = shortcuts.child(button(&q, &format!("company-ask-{i}"), p).on_press(
            move |s: &mut State| {
                s.question = TextEdit::new(&q);
                s.question.focused = true;
                s.ask_open = true;
            },
        ));
    }
    doc = doc.child(shortcuts).child(
        SegmentedControl::new(
            &["Overview", "People", "Meetings", "Decisions"],
            Some(s.companies.tab),
        )
        .label("Company tabs")
        .width(Some(if narrow {
            (s.app_w.get() - super::nav::width(s) - 33.0).max(240.0)
        } else {
            460.0
        }))
        .on_pick::<State>(|s, i, tx| {
            s.companies.tab = i;
            tx.scroll_to_y("companies-scroll", 0.0);
        })
        .build(k)
        .key("company-tabs"),
    );
    match s.companies.tab {
        1 => {
            if data.people.is_empty() {
                doc = doc.child(roles::SMALL.wrap(
                    "No people linked to this company from new meetings yet.",
                    p.soft,
                ));
            } else {
                let mut panel = cards::panel(p);
                for (i, person) in data.people.iter().enumerate() {
                    if i > 0 {
                        panel = panel.child(hairline(p));
                    }
                    panel = panel.child(people::person_row(s, person, k, narrow, p, layer));
                }
                doc = doc.child(people::heading(narrow, None, p)).child(panel);
            }
        }
        2 => {
            if data.meetings.is_empty() {
                doc = doc.child(roles::SMALL.wrap("No meetings with this company yet.", p.soft));
            } else {
                let mut panel = cards::panel(p);
                for (i, m) in data.meetings.iter().enumerate() {
                    if i > 0 {
                        panel = panel.child(hairline(p));
                    }
                    panel = panel.child(meeting_row(
                        m,
                        &m.title,
                        &icons::MEETINGS,
                        format!("company-meeting-{}", m.id),
                        p,
                    ));
                }
                doc = doc.child(panel);
            }
        }
        3 => {
            let groups = core::companies::group_decisions(&data.decisions);
            if groups.is_empty() {
                doc = doc.child(roles::SMALL.wrap(
                    "No decisions in this company's Meeting summaries yet.",
                    p.soft,
                ));
            } else {
                let mut panel = cards::panel(p);
                for (i, group) in groups.iter().enumerate() {
                    if i > 0 {
                        panel = panel.child(hairline(p));
                    }
                    panel = panel.child(decision_row(s, group, i, p));
                }
                doc = doc.child(panel);
            }
        }
        _ => {
            doc = doc.child(roles::SECTION.wrap("Who they are", p.ink)).child(
                roles::SMALL.wrap(
                    c.notes
                        .as_deref()
                        .unwrap_or("No notes about this company yet."),
                    p.soft,
                ),
            );
            if let Some(m) = data.meetings.first() {
                doc = doc.child(cards::panel(p).child(meeting_row(
                    m,
                    "Last meeting",
                    &icons::MEETINGS,
                    "company-last-meeting".into(),
                    p,
                )));
            } else {
                doc = doc.child(roles::SMALL.wrap("No meetings with this company yet.", p.soft));
            }
            let known_external = data
                .people
                .iter()
                .all(|person| person.side == Some(core::people::Side::External));
            let theirs = task_group(
                if known_external {
                    format!("Waiting on {}", c.name)
                } else {
                    format!("Open items involving {}", c.name)
                },
                format!("Nothing waiting on {}.", c.name),
                &data.open,
                if known_external {
                    &icons::CIRCLE_PAUSE
                } else {
                    &icons::CIRCLE
                },
                p,
            );
            let ours = task_group(
                "Our tasks".into(),
                format!("No open tasks for {}.", c.name),
                &data.ours,
                &icons::CIRCLE,
                p,
            );
            doc = doc.child(if narrow {
                col().w_full().gap(18.0).child(theirs).child(ours)
            } else {
                row()
                    .w_full()
                    .gap(20.0)
                    .items_start()
                    .child(theirs.grow(1.0).min_w(0.0))
                    .child(ours.grow(1.0).min_w(0.0))
            });
        }
    }
    col()
        .key("meeting-pane")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(scroll(
            col().w_full().items_center().child(doc.pb(24.0)),
            "companies-scroll",
        ))
}

pub fn scene(s: &mut State, tab: usize, empty: bool, merge: bool) {
    s.go_home();
    super::hex::scene(s, false);
    super::scene_agents(s, super::ALL_AGENTS);
    s.refresh_ask_model_input();
    let company = Company {
        id: "fixture-northwind".into(),
        name: "Northwind".into(),
        domain: Some("northwind.example".into()),
        notes: Some("Prototype workshop partner. Keep the next sample small.".into()),
        ..Default::default()
    };
    s.companies
        .data
        .catalog
        .records
        .insert(company.id.clone(), company.clone());
    s.companies.data.catalog.records.insert(
        "fixture-lumen".into(),
        Company {
            id: "fixture-lumen".into(),
            name: "Lumen".into(),
            ..Default::default()
        },
    );
    if !empty {
        for name in ["Dana Ortiz", "Leo Park"] {
            s.companies.data.catalog.people.insert(
                core::people::key(name),
                core::companies::PersonLink {
                    company_id: Some(company.id.clone()),
                    email: None,
                },
            );
        }
    }
    s.companies.open = true;
    s.companies.selected = Some(company.id);
    s.companies.tab = tab;
    s.companies.merging = merge;
    s.sidebar_open.set(s.app_w.get() > 820.0);
}
