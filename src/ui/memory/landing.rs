//! Needs you and Browse use the same pending proposal snapshot.
use super::*;
use crate::ui::widgets::ButtonDisabled;
use crate::ui::{cards, icons};

pub(super) fn count(s: &State, p: Palette) -> El {
    let n = s.memory.proposals.len() + s.memory.comments_not_used.len();
    roles::SMALL
        .wrap(
            if n == 0 {
                "Nothing needs you right now.".into()
            } else if n == 1 {
                "1 thing needs you".into()
            } else {
                format!("{n} things need you")
            },
            p.soft,
        )
        .key("memory-count")
}

fn search(s: &State, k: &Kit, p: Palette) -> El {
    TextInput::new(&s.memory.search)
        .plain()
        .leading_icon(&icons::SEARCH)
        .leading_icon_size(16.0)
        .placeholder("Ask or search what Sidenote remembers")
        .on_edit(|s: &mut State| &mut s.memory.search)
        .on_key(search_key)
        .on_submit(|s: &mut State, _| {
            let q = s.memory.search.text.clone();
            if s.has_agent() {
                // The answer shows under the Ask dock on this page.
                s.send(&q);
            } else {
                s.memory.result = Some(
                    "Choose a helper in Settings to ask about memory. You can still search here."
                        .into(),
                );
            }
        })
        .height(36.0)
        .build(k)
        .key("mem-search")
        .border(1.0, p.line)
        .radius(8.0)
        .fill(p.card)
        .grow(1.0)
        .min_w(0.0)
}

fn filter_chip(
    s: &State,
    k: &Kit,
    p: Palette,
    layer: &mut Vec<El>,
    kind: FilterMenu,
    name: &str,
    key: &str,
) -> El {
    let m = &s.memory;
    let open = m.filter_menu == Some(kind);
    let trigger = crate::ui::widgets::button(
        format!("{name} \u{25be}"),
        key,
        crate::ui::widgets::ButtonRole::Secondary,
        p,
    )
    .expanded(open)
    .on_press(move |s: &mut State| {
        s.memory.filter_menu = if open { None } else { Some(kind) };
    });
    let mut menu = Menu::new(format!("{key}-menu"))
        .trigger(trigger)
        .open(open)
        .placement(Placement::BottomStart)
        .min_width(240.0);
    if open {
        menu = menu.on_close(|s: &mut State| s.memory.filter_menu = None);
        match kind {
            FilterMenu::Type => {
                for (name, tab) in [("All types", Tab::All), ("Pending proposals", Tab::Review)]
                    .into_iter()
                    .chain(KINDS.into_iter().map(|t| (kind_label(t), Tab::Kind(t))))
                {
                    let count = match tab {
                        Tab::Review => m.proposals.len(),
                        Tab::All => m
                            .records
                            .iter()
                            .filter(|r| r.status != RecordStatus::Forgotten)
                            .count(),
                        Tab::Kind(kind) => m
                            .records
                            .iter()
                            .filter(|r| {
                                r.record_type == kind && r.status != RecordStatus::Forgotten
                            })
                            .count(),
                    };
                    menu = menu.item(
                        MenuItem::new(format!("{name} ({count})"))
                            .checked(m.tab == tab)
                            .on_select(move |s: &mut State| {
                                s.close_record();
                                s.memory.tab = tab;
                                s.memory.filter_menu = None;
                                s.save_memory_filters();
                            }),
                    );
                }
            }
            FilterMenu::State => {
                for (name, scope) in [
                    ("Active", Scope::Active),
                    ("Not recently confirmed", Scope::NeedsReview),
                    ("Retired and superseded", Scope::Retired),
                ] {
                    menu = menu.item(MenuItem::new(name).checked(m.scope == scope).on_select(
                        move |s: &mut State| {
                            s.close_record();
                            s.memory.scope = scope;
                            s.memory.filter_menu = None;
                            s.save_memory_filters();
                        },
                    ));
                }
            }
            FilterMenu::Project => {
                menu = menu.item(
                    MenuItem::new("All projects")
                        .checked(m.project.is_none())
                        .on_select(|s: &mut State| {
                            s.close_record();
                            s.memory.project = None;
                            s.memory.filter_menu = None;
                            s.save_memory_filters();
                        }),
                );
                for project in m.projects() {
                    let count = m
                        .records
                        .iter()
                        .filter(|r| {
                            r.project.as_ref() == Some(&project)
                                && r.status != RecordStatus::Forgotten
                        })
                        .count();
                    menu = menu.item(
                        MenuItem::new(format!("{project} ({count})"))
                            .checked(m.project.as_ref() == Some(&project))
                            .on_select(move |s: &mut State| {
                                s.close_record();
                                s.memory.project = Some(project.clone());
                                s.memory.filter_menu = None;
                                s.save_memory_filters();
                            }),
                    );
                }
            }
        }
    }
    menu.build(k, layer)
}

pub(super) fn toolbar(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let m = &s.memory;
    let filters = row()
        .flex_wrap()
        .gap(8.0)
        .items_center()
        .child(filter_chip(
            s,
            k,
            p,
            layer,
            FilterMenu::Type,
            "Type",
            "memory-filter-type",
        ))
        .child(filter_chip(
            s,
            k,
            p,
            layer,
            FilterMenu::State,
            "State",
            "memory-filter-state",
        ))
        .child(filter_chip(
            s,
            k,
            p,
            layer,
            FilterMenu::Project,
            "Project",
            "memory-filter-project",
        ));
    let mut toolbar = col().w_full().gap(8.0).child(
        row()
            .w_full()
            .flex_wrap()
            .gap(8.0)
            .items_center()
            .child(search(s, k, p).min_w(180.0))
            .child(filters),
    );
    let mut active = row().w_full().flex_wrap().gap(8.0).items_center();
    let mut any = false;
    if m.tab != Tab::All {
        any = true;
        let label = match m.tab {
            Tab::Kind(kind) => kind_label(kind),
            _ => "Pending proposals",
        };
        active = active.child(
            crate::ui::widgets::button(
                format!("Type: {label} \u{d7}"),
                "memory-clear-type",
                crate::ui::widgets::ButtonRole::Secondary,
                p,
            )
            .h(28.0)
            .label(format!("Remove type filter: {label}"))
            .on_press(|s: &mut State| {
                s.close_record();
                s.memory.tab = Tab::All;
                s.save_memory_filters();
            }),
        );
    }
    if m.scope != Scope::Active {
        any = true;
        let label = if m.scope == Scope::NeedsReview {
            "Needs review"
        } else {
            "Retired"
        };
        active = active.child(
            crate::ui::widgets::button(
                format!("State: {label} \u{d7}"),
                "memory-clear-state",
                crate::ui::widgets::ButtonRole::Secondary,
                p,
            )
            .h(28.0)
            .label("Remove state filter")
            .on_press(|s: &mut State| {
                s.close_record();
                s.memory.scope = Scope::Active;
                s.save_memory_filters();
            }),
        );
    }
    if let Some(project) = &m.project {
        any = true;
        active = active.child(
            crate::ui::widgets::button(
                format!("Project: {project} \u{d7}"),
                "memory-clear-project",
                crate::ui::widgets::ButtonRole::Secondary,
                p,
            )
            .h(28.0)
            .label(format!("Remove project filter: {project}"))
            .on_press(|s: &mut State| {
                s.close_record();
                s.memory.project = None;
                s.save_memory_filters();
            }),
        );
    }
    if m.meeting.is_some() {
        any = true;
        active = active.child(
            crate::ui::widgets::button(
                "From meeting \u{d7}",
                "memory-clear-meeting",
                crate::ui::widgets::ButtonRole::Secondary,
                p,
            )
            .h(28.0)
            .label("Remove meeting filter")
            .on_press(|s: &mut State| {
                s.close_record();
                s.memory.meeting = None;
                s.save_memory_filters();
            }),
        );
    }
    if any {
        toolbar = toolbar.child(active.child(
            crate::ui::widgets::quiet_button("Clear all", "memory-clear-all", p).on_press(
                |s: &mut State| {
                    s.close_record();
                    s.memory.tab = Tab::All;
                    s.memory.scope = Scope::Active;
                    s.memory.project = None;
                    s.memory.meeting = None;
                    s.memory.filter_menu = None;
                    s.save_memory_filters();
                },
            ),
        ));
    }
    toolbar
}

fn sources(s: &State, values: &[MemorySource], prefix: &str, p: Palette) -> El {
    let mut out = col().w_full().gap(2.0).children(source_rows(
        s,
        values.iter().map(MemorySource::reference),
        &format!("{prefix}-source"),
        p,
    ));
    if values.is_empty() {
        out = out.child(roles::META.wrap("No source recorded", p.faint));
    }
    out
}

fn proposal(s: &State, k: &Kit, pp: &PendingProposal, p: Palette) -> El {
    let pr = &pp.proposal;
    let target = pr.target_id.as_deref().and_then(|id| s.memory.record(id));
    let id = pr.id.clone();
    let shown = pp.clone();
    let reject_id = id.clone();
    let comment_id = id.clone();
    let (title, yes, no) = match pr.op {
        ProposalOp::Add => ("Add this to memory?", "Add to memory", "Leave out"),
        ProposalOp::Supersede => (
            "Which version should Sidenote keep?",
            "Use suggestion",
            "Keep current",
        ),
        ProposalOp::Confirm => ("Confirm this memory?", "Confirm", "Keep as is"),
        ProposalOp::Retire => ("Close this item?", "Close item", "Keep open"),
    };
    let commented = pp.can_rewrite() && pp.comment.is_some();
    let mut body = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(6.0)
        .child(roles::NAME.wrap(title, p.ink))
        .children(
            pp.revision_text()
                .is_none()
                .then(|| roles::SMALL.wrap(&pr.rationale, p.soft)),
        );
    if let Some(target) = target {
        body = body
            .child(roles::SMALL.wrap(
                format!(
                    "Current: {}",
                    plain(target.text.as_deref().unwrap_or_default())
                ),
                p.ink,
            ))
            .child(sources(
                s,
                &target.sources,
                &format!("need-{id}-current"),
                p,
            ));
    }
    body = body
        .children(
            pp.revision_text()
                .is_none()
                .then(|| roles::SMALL.wrap(format!("Suggested: {}", pr.text), p.ink)),
        )
        .child(sources(s, &pr.sources, &format!("need-{id}-suggested"), p))
        .children(proposal_comment(s, pp, p));
    body = body.child(
        row()
            .w_full()
            .flex_wrap()
            .gap(8.0)
            .child(
                button(
                    if commented { "Accept" } else { yes },
                    format!("memory-accept-{id}"),
                    true,
                    p,
                )
                .button_disabled(!s.memory.migrated || unapplied_comment(pp))
                .on_press(move |s: &mut State| s.proposal_accept(&shown, AcceptChoice::Shown)),
            )
            .children(original_button(
                &s.memory,
                pp,
                format!("memory-accept-original-{id}"),
                p,
            ))
            .child(
                button(
                    if commented { "Reject" } else { no },
                    format!("memory-reject-{id}"),
                    false,
                    p,
                )
                .border(1.0, p.line)
                .on_press(move |s: &mut State| s.proposal_reject(&reject_id)),
            ),
    );
    if s.memory.comment_for.as_deref() == Some(&id) {
        let save_id = id.clone();
        body = body
            .child(
                TextInput::new(&s.memory.comment)
                    .placeholder(if pp.can_rewrite() {
                        "Add a correction or context"
                    } else {
                        "Add a note to this decision"
                    })
                    .on_edit(|s: &mut State| &mut s.memory.comment)
                    .on_submit(move |s: &mut State, _| s.memory_comment_save(&save_id))
                    .height(36.0)
                    .build(k)
                    .key("memory-comment-input")
                    .w_full(),
            )
            .child(
                button("Save comment", "memory-comment-save", true, p)
                    .on_press(move |s: &mut State| s.memory_comment_save(&comment_id)),
            );
    } else {
        let initial = pp.comment.clone().unwrap_or_default();
        body = body.child(
            button(
                if pp.comment.is_some() {
                    "Edit comment"
                } else {
                    "Add a comment for Sidenote"
                },
                format!("memory-comment-{id}"),
                false,
                p,
            )
            .on_press(move |s: &mut State| {
                s.memory.comment_for = Some(comment_id.clone());
                s.memory.comment = TextEdit::new(&initial);
            }),
        );
    }
    if let Some(error) = s.memory.errors.get(&id) {
        body = body.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    if !s.memory.migrated {
        body = body.child(roles::META.wrap(
            "Browse all to review the existing memory format before accepting changes.",
            p.soft,
        ));
    }
    cards::panel(p).key(format!("memory-need-{id}")).child(
        cards::row_shell()
            .items_start()
            .child(cards::icon_tile(&icons::HELP, p))
            .child(body),
    )
}

fn recent(s: &State, p: Palette) -> El {
    let mut out = col().w_full().gap(8.0);
    let mut events: Vec<_> = s
        .memory
        .history
        .iter()
        .rev()
        .filter(|e| {
            !e.undone
                && matches!(
                    e.operation.as_str(),
                    "accept_proposal" | "reject_proposal" | "edit" | "confirm" | "retire"
                )
                && DateTime::parse_from_rfc3339(&e.at).is_ok_and(|at| {
                    let age = Local::now().signed_duration_since(at);
                    age >= chrono::Duration::zero() && age <= chrono::Duration::days(7)
                })
        })
        .collect();
    events.sort_by_key(|e| std::cmp::Reverse(DateTime::parse_from_rfc3339(&e.at).ok()));
    if events.is_empty() {
        return out;
    }
    out = out.mt(20.0);
    out = out.child(
        roles::GROUP
            .wrap("Recent changes you approved", p.soft)
            .mt(10.0),
    );
    let mut card = cards::panel(p);
    for (i, e) in events.into_iter().enumerate() {
        if i > 0 {
            card = card.child(hairline(p));
        }
        let title = e.decision.as_ref().and_then(|d| d.text.as_deref());
        let operation = event_name(&e.operation);
        let time = DateTime::parse_from_rfc3339(&e.at)
            .map(|d| d.with_timezone(&Local).format("%-d %b, %H:%M").to_string())
            .unwrap_or_else(|_| e.at.clone());
        let mut copy = col()
            .grow(1.0)
            .min_w(0.0)
            .gap(4.0)
            .child(roles::NAME.wrap(title.unwrap_or(&operation), p.ink))
            .child(roles::META.wrap(
                title.map_or_else(|| time.clone(), |_| format!("{operation} · {time}")),
                p.soft,
            ));
        if let Some(label) = e.comment_label {
            copy = copy.child(roles::META.wrap(label, p.soft));
        }
        if let Some(decision) = &e.decision {
            if let Some(comment) = &decision.comment {
                copy = copy.child(roles::SMALL.wrap(comment, p.soft));
            }
            copy = copy.child(sources(s, &decision.sources, &format!("recent-{i}"), p));
        }
        if let Some(error) = s.memory.errors.get(&e.record_id) {
            copy = copy.child(roles::SMALL.wrap(error, p.warn).role("alert"));
        }
        let mut line = cards::row_shell()
            .flex_wrap()
            .child(cards::icon_tile(&icons::FLAG, p))
            .child(copy);
        if e.can_undo {
            let record = e.record_id.clone();
            let at = e.at.clone();
            line = line.child(
                button("Undo", format!("memory-undo-{i}"), false, p)
                    .on_press_tx(move |s: &mut State, tx| s.memory_undo(&record, &at, tx)),
            );
        }
        card = card.child(line);
    }
    out.child(card)
        .child(roles::META.wrap("The last 7 days.", p.faint))
}

fn legacy_comments(s: &State, p: Palette) -> Option<El> {
    if s.memory.comments_not_used.is_empty() {
        return None;
    }
    let mut list = col()
        .w_full()
        .gap(8.0)
        .key("memory-comments-not-used")
        .child(roles::GROUP.wrap("Your comment was not used", p.soft));
    for (id, comment) in &s.memory.comments_not_used {
        let Some(text) = s.memory.record(id).and_then(|r| r.text.as_deref()) else {
            continue;
        };
        let edit = id.clone();
        let confirm = id.clone();
        list = list.child(
            cards::panel(p).child(
                cards::row_shell()
                    .flex_wrap()
                    .child(cards::copy(roles::NAME.wrap(text, p.ink), comment, p))
                    .child(
                        button("Edit", format!("memory-legacy-edit-{id}"), false, p).on_press(
                            move |s: &mut State| {
                                s.memory.tab = Tab::All;
                                s.open_record(&edit);
                                s.memory_edit_start(&edit);
                            },
                        ),
                    )
                    .child(
                        button("Confirm", format!("memory-legacy-confirm-{id}"), false, p)
                            .on_press(move |s: &mut State| {
                                s.memory_confirm(&confirm);
                            }),
                    ),
            ),
        );
    }
    Some(list)
}

pub(super) fn column(s: &State, k: &Kit, p: Palette, narrow: bool) -> El {
    let m = &s.memory;
    let mut doc = col()
        .key("memory-top")
        .w_full()
        .max_w(940.0)
        .px(if narrow { 16.0 } else { 40.0 })
        .pt(22.0)
        .pb(56.0)
        .gap(12.0)
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(text_title("Memory", if narrow { 26.0 } else { 30.0 }, p))
                .children(
                    (m.count_kind(None) > 0 || !m.proposals.is_empty()).then(|| {
                        button(
                            if m.count_kind(None) == 0 {
                                "Browse all".into()
                            } else {
                                format!("Browse all {}", m.count_kind(None))
                            },
                            "memory-browse",
                            false,
                            p,
                        )
                        .on_press(|s: &mut State| {
                            s.close_record();
                            s.memory.browse = true;
                        })
                    }),
                ),
        )
        .children((!m.records.is_empty() || !m.proposals.is_empty()).then(|| count(s, p)))
        .child(search(s, k, p).w_full().mb(6.0));
    let query = m.search.text.trim().to_lowercase();
    if query.is_empty() && m.records.is_empty() && m.proposals.is_empty() {
        doc = doc.child(cards::panel(p).child(
            col()
                .padding(16.0)
                .gap(6.0)
                .child(roles::NAME.wrap("No saved memory yet", p.ink))
                .child(roles::SMALL.wrap(
                    "Daily check suggests facts from your meetings. Review them here and approve the ones you want Sidenote to remember.",
                    p.soft,
                )),
        ));
    }
    let pending: Vec<_> = m
        .proposals
        .iter()
        .filter(|pp| {
            query.is_empty()
                || format!("{} {}", pp.proposal.text, pp.proposal.rationale)
                    .to_lowercase()
                    .contains(&query)
        })
        .collect();
    if !m.proposals.is_empty() || !query.is_empty() {
        doc = doc.child(roles::GROUP.wrap(
            format!("Needs you · {} of at most 5", pending.len().min(5)),
            p.soft,
        ));
        for pp in pending.into_iter().take(5) {
            doc = doc.child(proposal(s, k, pp, p));
        }
        if m.proposals.len() > 5 {
            doc = doc.child(roles::META.wrap("Browse all to see the remaining proposals.", p.soft));
        }
    }
    if !query.is_empty() {
        doc = doc.child(roles::GROUP.wrap("Search results", p.soft));
        let mut found = 0;
        for record in m
            .records
            .iter()
            .filter(|r| r.status == RecordStatus::Active && m.matches_query(r))
        {
            found += 1;
            let id = record.id.clone();
            doc = doc.child(
                cards::panel(p).child(
                    cards::row_shell()
                        .child(cards::copy(
                            roles::NAME
                                .wrap(plain(record.text.as_deref().unwrap_or_default()), p.ink),
                            &record.project.clone().unwrap_or_default(),
                            p,
                        ))
                        .child(
                            button("Open", format!("memory-search-{}", record.id), false, p)
                                .on_press(move |s: &mut State| s.open_record(&id)),
                        ),
                ),
            );
        }
        if found == 0 {
            doc = doc.child(roles::SMALL.wrap(
                "No saved memory matches this search. Press Enter to ask Sidenote.",
                p.soft,
            ));
        }
    } else {
        doc = doc
            .children(legacy_comments(s, p))
            .children(super::settled::section(s, p))
            .child(recent(s, p));
    }
    col().grow(1.0).min_w(0.0).min_h(0.0).child(scroll(
        col().w_full().items_center().child(doc),
        "memory-scroll",
    ))
}
