//! People is a projection of the meeting roster, with device-local side corrections.
use super::{
    State, cards,
    hex::settings::{self, Page},
    icons, nav, roles, tasks, teams,
    theme::Palette,
    widgets::{hairline, scroll},
};
use crate::ui::widgets::{ButtonDisabled, menu_row, menu_separator, menu_surface, menu_trigger};
use rune::{Align, El, Side as Edge, TextEdit, col, row};
use rune_kit::{Kit, SegmentedControl, TextInput};
use sidenote::core::{
    self,
    people::{Person, Side},
};
use std::path::Path;

pub struct TeamEditor {
    pub original: Option<core::Team>,
    pub advanced: bool,
    pub remove_confirm: bool,
    pub member_search: TextEdit,
}
pub struct PeopleUi {
    pub affiliations: core::people::Affiliations,
    pub tab: usize,
    pub search: TextEdit,
    pub prefs: core::UiPrefs,
    pub team_editor: Option<TeamEditor>,
    pub notice: Option<String>,
}
impl PeopleUi {
    pub fn load(
        root: &Path,
        meetings: &[core::Meeting],
        prefs: &mut core::UiPrefs,
        config: &core::Config,
    ) -> Self {
        let notice = if config.features.people
            && core::people::initialize(prefs, meetings)
            && prefs.save(root).is_err()
        {
            prefs.people_existing_meetings = None;
            Some("Could not remember the existing meetings. People will stay hidden until this can be saved.".into())
        } else {
            None
        };
        Self {
            affiliations: core::people::Affiliations::identity(config),
            tab: prefs
                .settings_tabs
                .get("People")
                .copied()
                .unwrap_or(0)
                .min(2),
            search: TextEdit::default(),
            prefs: prefs.clone(),
            team_editor: None,
            notice,
        }
    }
}

impl State {
    pub fn open_people(&mut self) {
        if !self.config.features.people {
            return;
        }
        self.go_home();
        self.view = tasks::View::People;
        self.nav_done();
    }

    pub fn open_people_tab(&mut self, tab: usize) {
        if !self.config.features.people {
            return;
        }
        if self.view != tasks::View::People {
            self.open_people();
        }
        self.people.tab = tab.min(Page::People.tabs().len() - 1);
        if rune::tooling::headless_run().is_none() || super::isolated() {
            let mut prefs = core::UiPrefs::load(&self.store.root);
            prefs.settings_tabs.insert("People".into(), self.people.tab);
            let _ = prefs.save(&self.store.root);
        }
        super::scene_scroll(self, "people-title");
    }

    fn set_person_side(&mut self, person: &str, side: Option<Side>) {
        if !self.config.features.people {
            return;
        }
        let mut prefs = self.people.prefs.clone();
        match prefs.save_person_side(&self.store.root, person, &self.tasks.teams, side) {
            Ok(()) => {
                self.people.prefs.people_sides = prefs.people_sides;
                self.people.prefs.relay_people_base = prefs.relay_people_base;
                self.people.notice = None;
            }
            Err(_) => {
                self.people.notice =
                    Some("Could not save this person's side. Your previous choice is kept.".into())
            }
        }
    }
}
fn side_control(person: &Person, k: &Kit, p: Palette) -> El {
    let mut selected_kit = k.clone();
    selected_kit.c.bg_base = p.sel;
    selected_kit.c.border_strong = p.ink;
    let name = person.name.clone();
    SegmentedControl::new(
        &["Our team", "External"],
        person.side.map(|s| usize::from(s == Side::External)),
    )
    .label(format!("Side for {}", person.name))
    .width(Some(168.0))
    .on_pick::<State>(move |s, index, _| {
        s.set_person_side(
            &name,
            if index == 0 {
                Some(Side::OurTeam)
            } else {
                Some(Side::External)
            },
        )
    })
    .build(&selected_kit)
    .key(format!("person-side-{}", core::people::key(&person.name)))
}
fn avatar(name: &str, p: Palette) -> El {
    let initials = name
        .split_whitespace()
        .take(2)
        .filter_map(|n| n.chars().next())
        .flat_map(char::to_uppercase)
        .collect::<String>();
    col()
        .size(34.0, 34.0)
        .center()
        .radius(8.0)
        .fill(p.chip)
        .child(roles::NAME.line(initials, p.soft))
}
fn person_menu(s: &State, person: &Person, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let id = core::people::key(&person.name);
    let key = format!("person-menu-{id}");
    let toggle = key.clone();
    let open = s.more_menu.as_deref() == Some(key.as_str());
    let trigger_key = format!("person-more-{id}");
    let trigger = super::widgets::pressable(icons::draw(&icons::MORE, 16.0, p.soft).padding(8.0))
        .key(&trigger_key)
        .label(format!("More actions for {}", person.name))
        .disabled(s.edit_busy)
        .radius(6.0)
        .hover(p.chip);
    let trigger = menu_trigger(trigger, open, p, move |s| {
        s.more_menu = if s.more_menu.as_deref() == Some(&toggle) {
            None
        } else {
            Some(toggle.clone())
        };
    });
    if !open {
        return trigger;
    }
    let email = s
        .companies
        .data
        .catalog
        .people
        .get(&id)
        .and_then(|l| l.email.clone());
    let name = person.name.clone();
    let edit_name = name.clone();
    let item = |name| menu_row(name, p, None, None, None);
    let mut menu = col()
        .child(
            item(if email.is_some() {
                "Edit email"
            } else {
                "Add email"
            })
            .on_press(move |s: &mut State| {
                s.more_menu = None;
                s.edit_person_company(&name);
            }),
        )
        .child(item("Edit person").on_press(move |s: &mut State| {
            s.more_menu = None;
            s.edit_person_company(&edit_name);
        }));
    if let Some(email) = email {
        menu = menu.child(item("Copy email").on_press_tx(move |s: &mut State, tx| {
            s.more_menu = None;
            tx.copy(email.clone());
        }));
    }
    if person.manual_side {
        let name = person.name.clone();
        let danger = Palette { ink: p.danger, ..p };
        menu = menu.child(menu_separator(p)).child(
            menu_row("Reset to automatic", danger, None, None, None).on_press(
                move |s: &mut State| {
                    s.more_menu = None;
                    s.set_person_side(&name, None);
                },
            ),
        );
    }
    menu_surface(
        k,
        p,
        layer,
        menu,
        (&key, &trigger_key, Edge::Bottom, Align::End, 240.0),
        |s| s.more_menu = None,
    );
    trigger
}

pub fn heading(compact: bool, affiliation: Option<bool>, p: Palette) -> El {
    if compact {
        return col();
    }
    cards::row_shell()
        .pt(10.0)
        .pb(0.0)
        .mb(8.0)
        .child(roles::META.line("Person", p.faint).grow(1.0).min_w(190.0))
        .children(affiliation.map(|external| {
            roles::META
                .line(if external { "Company" } else { "Team" }, p.faint)
                .w(100.0)
        }))
        .child(roles::META.line("Meetings", p.faint).w(52.0))
        .child(roles::META.line("Last seen", p.faint).w(65.0))
        .child(roles::META.line("Open", p.faint).w(40.0))
        .child(roles::META.line("Relationship", p.faint).w(168.0))
        .child(col().w(32.0))
}

pub fn person_row(
    s: &State,
    person: &Person,
    k: &Kit,
    compact: bool,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    let id = core::people::key(&person.name);
    let company = s.companies.data.company_for(&person.name);
    let email = s
        .companies
        .data
        .catalog
        .people
        .get(&id)
        .and_then(|l| l.email.as_deref());
    let mut name = col().grow(1.0).min_w(0.0).gap(3.0).child(
        roles::NAME
            .wrap(&person.name, p.ink)
            .max_lines(2)
            .help(&person.name),
    );
    if person.is_self {
        name = name.child(roles::META.line("You", p.soft));
    }
    let company_name = company.map(|c| c.name.as_str()).or_else(|| {
        (!s.companies
            .data
            .catalog
            .people
            .get(&id)
            .is_some_and(|link| link.company_id.is_none()))
        .then_some(person.company.as_deref())
        .flatten()
    });
    let company_name =
        company_name.filter(|_| !(person.manual_side && person.side == Some(Side::External)));
    if let Some(company_name) = company_name {
        name = name.child(roles::META.line(company_name, p.soft));
    }
    if !person.aliases.is_empty() {
        name = name.child(roles::META.line(format!("also {}", person.aliases.join(", ")), p.soft));
    }
    if let Some(email) = email {
        name = name.child(
            roles::META
                .line(email, p.soft)
                .key(format!("person-email-value-{id}")),
        );
    }
    if person.suggested {
        let person_name = person.name.clone();
        name = name.child(
            row()
                .gap(8.0)
                .items_center()
                .child(
                    roles::META
                        .line("Suggested: Our team", p.soft)
                        .key(format!("person-suggestion-{id}")),
                )
                .child(
                    super::widgets::button(
                        "Confirm",
                        &format!("person-confirm-{id}"),
                        super::widgets::ButtonRole::Ghost,
                        p,
                    )
                    .h(28.0)
                    .button_disabled(s.edit_busy)
                    .on_press(move |s: &mut State| {
                        s.set_person_side(&person_name, Some(Side::OurTeam))
                    }),
                ),
        );
    } else if person.side.is_none() {
        name = name.child(roles::META.line("Relationship not set", p.faint));
    }
    let identity = row()
        .min_w(190.0)
        .grow(1.0)
        .gap(12.0)
        .items_center()
        .child(avatar(&person.name, p))
        .child(name);
    let affiliation = if s.companies.open {
        None
    } else if s.people.tab == 1 {
        let name = person.name.clone();
        Some(
            super::widgets::button(
                company_name.unwrap_or("No company"),
                &format!("person-company-cell-{id}"),
                super::widgets::ButtonRole::Ghost,
                p,
            )
            .h(28.0)
            .label(format!("Choose company for {}", person.name))
            .button_disabled(s.edit_busy)
            .on_press(move |s: &mut State| s.edit_person_company(&name)),
        )
    } else {
        Some(roles::SMALL.line(&person.team, p.soft))
    };
    let date = person
        .last_seen
        .map(|d| d.format("%-d %b").to_string())
        .unwrap_or_else(|| "Unknown".into());
    let menu = person_menu(s, person, k, p, layer);
    let mut line = cards::row_shell().key(format!("person-{id}"));
    if compact {
        line = line
            .flex_wrap()
            .child(identity)
            .child(menu)
            .child(side_control(person, k, p));
        if let Some(cell) = affiliation {
            line = line.child(cell.w_full().pl(46.0));
        }
        line = line.child(
            roles::META
                .wrap(
                    format!(
                        "{} meetings \u{b7} {} \u{b7} {} open",
                        person.meetings, date, person.open
                    ),
                    p.soft,
                )
                .w_full()
                .pl(46.0),
        );
    } else {
        line = line
            .child(identity)
            .children(affiliation.map(|cell| cell.w(100.0)))
            .child(
                roles::META
                    .line(person.meetings.to_string(), p.soft)
                    .w(52.0),
            )
            .child(roles::META.line(date, p.soft).w(65.0))
            .child(roles::META.line(person.open.to_string(), p.soft).w(40.0))
            .child(side_control(person, k, p))
            .child(menu.w(32.0));
    }
    let mut out = col().w_full().child(line);
    if s.companies.person.as_deref() == Some(&person.name) {
        out = out.child(super::companies::person_editor(s, k, p));
    }
    out
}
pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let tab = s.people.tab;
    let doc = col()
        .w_full()
        .max_w(940.0)
        .px(if narrow { 16.0 } else { 40.0 })
        .pt(22.0)
        .pb(32.0)
        .child(
            rune::text("People")
                .font(super::theme::display(32.0))
                .color(p.ink)
                .wrap()
                .key("people-title"),
        )
        .child(settings::tab_bar(s, p, false, Page::People, tab))
        .child(content(s, k, p, layer));
    col()
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .child(scroll(
            col().w_full().items_center().child(doc),
            "people-scroll",
        ))
        .child(super::ask::overlay(s, k, p, narrow, layer))
}

pub(super) fn content(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    section(s, k, p, s.people.tab, layer)
}

fn section(s: &State, k: &Kit, p: Palette, tab: usize, layer: &mut Vec<El>) -> El {
    if tab == 2 {
        return col().key("people-import").w_full().mt(18.0).child(cards::panel(p).child(cards::row_shell()
            .child(cards::icon_tile(&icons::FILE_TEXT,p))
            .child(cards::copy(roles::NAME.wrap("People import is not available yet",p.ink),
                "People come from new meeting summaries. You can add and edit teams on Our team.",p))));
    }
    let compact = s.app_w.get() - nav::width(s) < 900.0;
    let query = s.people.search.text.to_lowercase();
    let people = core::people::roster(
        &s.meetings,
        &s.tasks.teams,
        &s.tasks.all,
        &s.people.prefs,
        &s.people.affiliations,
    );
    let filtered: Vec<_> = people
        .iter()
        .filter(|person| {
            let on_team = person.side == Some(Side::OurTeam);
            let company = s
                .companies
                .data
                .company_for(&person.name)
                .filter(|_| !(person.manual_side && person.side == Some(Side::External)))
                .map(|c| format!("{} {}", c.name, c.aliases.join(" ")))
                .unwrap_or_default();
            let searchable = format!(
                "{} {} {} {} {}",
                person.name,
                person.aliases.join(" "),
                person.team,
                person.company.as_deref().unwrap_or(""),
                company
            );
            on_team == (tab == 0)
                && (query.is_empty() || searchable.to_lowercase().contains(&query))
        })
        .collect();
    let mut out = col()
        .key("people-roster")
        .w_full()
        .gap(12.0)
        .mt(18.0)
        .child(
            col().key("people-import").child(
                roles::META
                    .wrap(
                        "People from older meetings appear when you meet them again.",
                        p.soft,
                    )
                    .key("people-history-note"),
            ),
        )
        .child(
            TextInput::new(&s.people.search)
                .leading_icon(&icons::SEARCH)
                .leading_icon_size(16.0)
                .plain()
                .placeholder("Find a person")
                .on_edit(|s: &mut State| &mut s.people.search)
                .on_key(|s: &mut State, e, tx| {
                    if e.key == rune::Key::Escape {
                        s.people.search = TextEdit::default();
                        tx.blur();
                        true
                    } else {
                        false
                    }
                })
                .height(36.0)
                .build(k)
                .key("people-search")
                .border(1.0, p.line)
                .radius(8.0)
                .fill(p.card)
                .w_full(),
        );
    if let Some(message) = s
        .people
        .notice
        .as_ref()
        .or(s.people.affiliations.notice.as_ref())
    {
        out = out.child(roles::SMALL.wrap(message, p.warn).role("alert"));
    }
    let mut section_heading = row().w_full().items_center().gap(12.0).mt(6.0).child(
        roles::GROUP
            .wrap(
                if tab == 0 {
                    "Our team"
                } else {
                    "External and unassigned"
                },
                p.soft,
            )
            .grow(1.0),
    );
    if tab == 1 {
        section_heading = section_heading.child(
            super::widgets::link("All companies", p)
                .key("people-companies")
                .on_press(|s: &mut State| s.open_company(None)),
        );
    }
    out = out.child(section_heading);
    if let Some(message) = &s.companies.notice {
        out = out.child(roles::SMALL.wrap(message, p.warn).role("alert"));
    }
    if filtered.is_empty() {
        let text = if !query.is_empty() {
            "No people match this search."
        } else if tab == 0 {
            "People show up here after your first new meetings."
        } else {
            "No external people from new meetings yet."
        };
        out = out.child(cards::panel(p).child(roles::SMALL.wrap(text, p.soft).padding(14.0)));
    } else {
        let mut groups = std::collections::BTreeMap::<Option<String>, Vec<&Person>>::new();
        for person in &filtered {
            let company =
                if tab == 1 && !(person.manual_side && person.side == Some(Side::External)) {
                    s.companies
                        .data
                        .company_for(&person.name)
                        .map(|c| c.id.clone())
                } else {
                    None
                };
            groups.entry(company).or_default().push(person);
        }
        for (company, group) in groups {
            if tab == 1 {
                if let Some(c) = company
                    .as_ref()
                    .and_then(|id| s.companies.data.catalog.records.get(id))
                {
                    let id = c.id.clone();
                    out = out.child(
                        super::widgets::pressable(
                            row()
                                .w_full()
                                .items_center()
                                .gap(8.0)
                                .py(8.0)
                                .child(icons::draw(&icons::BUILDING_2, 18.0, p.soft))
                                .child(
                                    roles::NAME
                                        .wrap(
                                            format!("{} \u{b7} {} people", c.name, group.len()),
                                            p.ink,
                                        )
                                        .max_lines(2),
                                )
                                .child(icons::draw(&icons::CHEVRON_RIGHT, 16.0, p.ink)),
                        )
                        .key(format!("company-header-{}", c.id))
                        .label(format!("Open company {}", c.name))
                        .on_press(move |s: &mut State| s.open_company(Some(id.clone()))),
                    );
                } else {
                    out = out.child(roles::NAME.wrap("No company assigned", p.soft));
                }
            }
            let mut panel = cards::panel(p)
                .key(format!(
                    "people-card-{}",
                    company.as_deref().unwrap_or("none")
                ))
                .children((!compact).then(|| heading(false, Some(tab == 1), p)))
                .children((!compact).then(|| hairline(p)));
            for (i, person) in group.iter().enumerate() {
                if i > 0 {
                    panel = panel.child(hairline(p));
                }
                panel = panel.child(person_row(s, person, k, compact, p, layer));
            }
            out = out.child(panel);
        }
    }

    if tab == 0 {
        out = out.child(teams::section(s, k, p));
    }
    out
}
