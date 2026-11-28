//! The board layout of the Tasks page: four columns over the same tasks, one card each. Every
//! way of moving a card (the Move menu, the keyboard) ends in `State::move_card`, which saves
//! through the same status edit the task panel uses. The page and the panel are `tasks.rs`;
//! this file only lays the cards out. It is a child of that module, so it reads its helpers.
use super::{
    Panel, RowId, State, TickState, View, avatar, button, cards, chevron, date_chip, empty_note,
    owner_name, pressable, project_dot, roles, shown_title, status_dot, task_tick_feedback,
    tick_key,
};
use crate::ui::{edit_worker::Job, theme::Palette};
use rune::{El, Key, KeyEvent, Tx, col, row};
use rune_kit::{Kit, Menu, MenuItem, Placement, SegmentedControl};
use sidenote::core::{self, BoardColumn, BoardView, MoveIntent, TaskItem};
use std::{
    borrow::Cow,
    collections::HashSet,
    hash::Hash,
    rc::Rc,
    time::{Duration, Instant},
};

fn row_of(t: &TaskItem) -> RowId {
    (t.meeting_id.clone(), t.index)
}

/// Narrowest a column can be before the board falls back to one column at a time.
const COLUMN_MIN: f32 = 220.0;
const GAP: f32 = 12.0;
/// Padding of a column and of a card, which the drag arithmetic adds up.
const COLUMN_PAD: f32 = 8.0;
const CARD_PAD: f32 = 11.0;
/// The list width the four columns need, side padding included.
pub const WIDE_MIN: f32 = 4.0 * COLUMN_MIN + 3.0 * GAP + 48.0;
/// Cards drawn per column before "Show more". The page scrolls as one, so this bounds the work.
const PAGE: usize = 25;
const DONE_PAGE: usize = 10;
const LIVE_FOR: Duration = Duration::from_secs(4);

pub fn first_page(c: BoardColumn) -> usize {
    if c == BoardColumn::Done {
        DONE_PAGE
    } else {
        PAGE
    }
}

impl State {
    /// The task as it will be once every save still waiting has run: the card over the merge of
    /// every move made on it since the last idle save.
    pub(super) fn effective(&self, t: &TaskItem) -> TaskItem {
        let mut eff = t.clone();
        if let Some(pending) = self.tasks.board_pending.get(&self.tasks.pending_key(t)) {
            let intent = pending.intent;
            if let Some(done) = intent.done {
                eff.done = done;
            }
            if let Some(status) = intent.status {
                eff.status = status;
            }
        }
        eff
    }

    /// The card as the board draws it. Only a card with a move saving is copied.
    pub(super) fn shown_card<'a>(&self, t: &'a TaskItem) -> Cow<'a, TaskItem> {
        if !self.tasks.board_pending.is_empty()
            && self
                .tasks
                .board_pending
                .contains_key(&self.tasks.pending_key(t))
        {
            Cow::Owned(self.effective(t))
        } else {
            Cow::Borrowed(t)
        }
    }

    /// The folder a task's status edit reads its source summary from: the meeting's, or the
    /// data folder for a hand-made task.
    fn edit_source(&self, t: &TaskItem) -> Option<std::path::PathBuf> {
        if t.manual {
            return Some(self.tasks.store.root.clone());
        }
        self.meetings
            .iter()
            .find(|m| m.id == t.meeting_id && m.device.is_none())
            .map(|m| m.folder.clone())
    }

    /// Moves one card to `target`.
    pub(super) fn move_card(&mut self, id: &str, index: usize, target: BoardColumn) {
        self.apply_intent(id, index, MoveIntent::to(target), false);
    }

    /// Saves what a move asks for. The card is drawn where it is going at once; its intent
    /// merges with any move still waiting on the same card, and the save writes the final
    /// status and tick as values, so a queue of quick moves ends where the last one said and
    /// never forgets the status from before a trip through Done.
    fn apply_intent(&mut self, id: &str, index: usize, intent: MoveIntent, checkbox: bool) {
        self.tasks.board_menu = None;
        let Some(t) = self.tasks.item(id, index).cloned() else {
            return;
        };
        let eff = self.effective(&t);
        let (from, to) = (BoardColumn::of(&eff), intent.column(&eff));
        self.tasks.cursor = Some(row_of(&t));
        let title = shown_title(self, &t).to_string();
        if to == from {
            self.say(format!(
                "\u{201c}{title}\u{201d} is already in {}",
                to.label()
            ));
            return;
        }
        if let Some(on) = intent.done.filter(|_| checkbox) {
            self.publish_checkbox(Box::new(t), on, TickState::Pending);
        } else {
            let source = match self.edit_source(&t) {
                Some(source) => source,
                // A tick needs no summary file. A status does.
                None if intent.status.is_none() => std::path::PathBuf::new(),
                None => {
                    self.tasks.notice =
                        Some("This task's meeting is on another device. Move it there.".into());
                    return;
                }
            };
            let tick = self.stage_task_move(&t, intent, None);
            self.publish_move(Job::Move {
                store: self.tasks.store.clone(),
                ticks: self.store.clone(),
                task: Box::new(t),
                source,
                intent,
                tick,
            });
        }
        self.reveal_card(id, index);
        self.say(if intent.status.is_none() && intent.done == Some(false) {
            format!(
                "Marked \u{201c}{title}\u{201d} not done. Now in {}",
                to.label()
            )
        } else {
            format!("Moved \u{201c}{title}\u{201d} to {}", to.label())
        });
    }

    /// The tick, from the card's checkbox, the menu or the X key. It sets the opposite of what
    /// the card will be once earlier saves run, so quick presses alternate and every one is
    /// saved. Out of Done the card goes back to the status it had.
    pub(super) fn toggle_done(&mut self, id: &str, index: usize) {
        let Some(t) = self.tasks.item(id, index).cloned() else {
            return;
        };
        let now = self.effective(&t).done;
        self.apply_intent(id, index, MoveIntent::tick(!now), true);
    }

    /// Queues a reveal against the next board refresh.
    pub(super) fn reveal_card(&mut self, id: &str, index: usize) {
        if self.tasks.layout == core::TasksLayout::Board
            && let Some(task) = self.tasks.item(id, index)
        {
            self.tasks.reveal = Some(tick_key(task));
        }
    }

    fn refresh_reveal(&mut self, tx: &mut Tx) -> bool {
        let Some(key) = self.tasks.reveal.as_ref() else {
            return false;
        };
        let Some(task) = self.tasks.task_by_key(key) else {
            self.tasks.reveal = None;
            return false;
        };
        let Some(i) = self
            .tasks
            .items
            .iter()
            .position(|t| t.meeting_id == task.meeting_id && t.index == task.index)
        else {
            return false;
        };
        if self.view != View::Tasks || self.tasks.layout != core::TasksLayout::Board {
            return false;
        }
        let view = self.tasks.board();
        let Some((c, n)) = BoardColumn::ALL
            .into_iter()
            .find_map(|c| view.column(c).iter().position(|&r| r == i).map(|n| (c, n)))
        else {
            return false;
        };
        if c == BoardColumn::Done && view.done_folded {
            self.tasks.done_open = true;
        }
        let shown = &mut self.tasks.board_shown[c.index()];
        *shown = (*shown).max(first_page(c)).max(n + 1);
        self.tasks.board_tab = c;
        let (id, index) = row_of(&self.tasks.items[i]);
        tx.scroll_into_view(format!("trow-{id}-{index}"));
        self.tasks.cursor = Some((id, index));
        self.tasks.reveal = None;
        true
    }

    /// A short polite line under the toolbar. It goes after a few seconds or the next key.
    pub(super) fn say(&mut self, line: String) {
        self.tasks.board_live = Some((line, Instant::now() + LIVE_FOR));
    }

    /// Shift+Left and Shift+Right: one card one column over.
    fn nudge_card(&mut self, id: &str, index: usize, delta: isize) {
        let Some(from) = self
            .tasks
            .item(id, index)
            .map(|t| BoardColumn::of(&self.effective(t)))
        else {
            return;
        };
        match from.neighbour(delta) {
            Some(to) => self.move_card(id, index, to),
            None => self.say(format!(
                "Nothing {} of {}",
                if delta < 0 { "left" } else { "right" },
                from.label()
            )),
        }
    }

    /// Left and Right: the cursor to the nearest card in the next column that has one.
    fn step_cursor_column(&mut self, delta: isize, tx: &mut Tx) {
        let view = self.tasks.board();
        let at = self.cursor_row().and_then(|(id, index)| {
            let i = self
                .tasks
                .items
                .iter()
                .position(|t| t.meeting_id == id && t.index == index)?;
            BoardColumn::ALL
                .into_iter()
                .find_map(|c| view.column(c).iter().position(|&r| r == i).map(|n| (c, n)))
        });
        let mut column = at.map_or(BoardColumn::Todo, |(c, _)| c);
        let row = at.map_or(0, |(_, n)| n);
        loop {
            let Some(next) = (if at.is_none() {
                Some(column)
            } else {
                column.neighbour(delta)
            }) else {
                return;
            };
            column = next;
            let cards = view.column(column);
            let hidden = column == BoardColumn::Done && view.done_folded && !self.tasks.done_open;
            if !cards.is_empty() && !hidden {
                let i = cards[row.min(cards.len() - 1)];
                let (id, index) = (
                    self.tasks.items[i].meeting_id.clone(),
                    self.tasks.items[i].index,
                );
                self.tasks.board_tab = column;
                self.tasks.keyboard = true;
                if matches!(self.tasks.panel, Some(Panel::Task(_))) && !self.open_task(&id, index) {
                    return;
                }
                self.tasks.cursor = Some((id.clone(), index));
                self.reveal_card(&id, index);
                tx.scroll_into_view(format!("trow-{id}-{index}"));
                return;
            }
            if at.is_none() {
                column = column.neighbour(1).unwrap_or(column);
                if column == BoardColumn::Done {
                    return;
                }
            }
        }
    }
}

/// The card a control's key belongs to. Card controls are keyed `<prefix>-<meeting>-<index>`,
/// where the meeting id has dashes of its own and a subtask key carries more after the index,
/// so each split is checked against the cards that exist.
fn card_of_key(key: &str, exists: impl Fn(&str, usize) -> bool) -> Option<RowId> {
    const PREFIXES: [&str; 9] = [
        "trow-",
        "ttask-",
        "ttick-",
        "tsource-",
        "tgrip-",
        "subtasks-toggle-",
        "subtick-",
        "tmove-menu-",
        "tmove-",
    ];
    let rest = PREFIXES.iter().find_map(|p| key.strip_prefix(p))?;
    rest.match_indices('-').find_map(|(at, _)| {
        let id = &rest[..at];
        let index = rest[at + 1..].split('-').next()?.parse().ok()?;
        exists(id, index).then(|| (id.to_string(), index))
    })
}

/// The card a move key acts on: the one whose control has the keyboard. Focus on anything else
/// (a text box, a toolbar button) leaves the key alone. Only when nothing at all has focus does
/// it fall to the card the j and k keys are showing, which is the one outlined on screen.
fn move_target(
    focused: Option<&str>,
    cursor: Option<RowId>,
    exists: impl Fn(&str, usize) -> bool,
) -> Option<RowId> {
    match focused {
        Some(key) => card_of_key(key, exists),
        None => cursor,
    }
}

/// The board's own keys, tried before the list's. Returns whether the key was used.
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    let focused = tx.focused().map(str::to_string);
    let exists = |s: &State, id: &str, index: usize| s.tasks.item(id, index).is_some();
    s.tasks.card_focus = focused
        .as_deref()
        .and_then(|k| card_of_key(k, |id, i| exists(s, id, i)));
    let target = move_target(
        focused.as_deref(),
        s.cursor_row().filter(|_| s.tasks.keyboard),
        |id, i| exists(s, id, i),
    );
    let shift = e.mods.shift;
    match &e.key {
        Key::Left | Key::Right => {
            let delta = if e.key == Key::Left { -1 } else { 1 };
            if shift {
                let Some((id, index)) = target else {
                    return false;
                };
                s.nudge_card(&id, index, delta);
            } else {
                s.step_cursor_column(delta, tx);
            }
            true
        }
        Key::Text(t) => match t.as_str() {
            "[" | "]" => {
                let Some((id, index)) = target else {
                    return false;
                };
                s.nudge_card(&id, index, if t == "[" { -1 } else { 1 });
                true
            }
            "h" => {
                s.step_cursor_column(-1, tx);
                true
            }
            "l" => {
                s.step_cursor_column(1, tx);
                true
            }
            "m" => match target {
                Some(row) => {
                    s.tasks.board_menu = Some(row);
                    true
                }
                None => false,
            },
            _ => false,
        },
        _ => false,
    }
}

/// Clears the live line when its time is up, and scrolls to a card that was just revealed.
/// Returns whether the page changed.
pub fn live_tick(s: &mut State, tx: &mut Tx) -> bool {
    let changed = s.refresh_reveal(tx);
    let Some((_, until)) = &s.tasks.board_live else {
        return changed;
    };
    let left = until.saturating_duration_since(Instant::now());
    if left.is_zero() {
        s.tasks.board_live = None;
        return true;
    }
    tx.wake_after(left);
    changed
}

fn area_label(s: &State, t: &TaskItem) -> String {
    t.area_id
        .as_ref()
        .and_then(|id| s.areas.catalog.areas.get(id))
        .map(|area| area.name.clone())
        .unwrap_or_else(|| {
            s.areas
                .project_labels
                .get(&t.project)
                .cloned()
                .unwrap_or_else(|| t.project.clone())
        })
}

fn column_icon(c: BoardColumn) -> &'static rune::Icon {
    match c {
        BoardColumn::Todo => &crate::ui::icons::CIRCLE,
        BoardColumn::Waiting => &crate::ui::icons::WAITING,
        BoardColumn::Review => &crate::ui::icons::EYE,
        BoardColumn::Done => &crate::ui::icons::DONE,
    }
}

/// The Move menu: the same choices the task panel's status menu has, for one card. It is the
/// path for a screen reader, for narrow windows and for anyone who does not drag.
fn move_menu(s: &State, k: &Kit, t: &TaskItem, p: Palette, layer: &mut Vec<El>) -> El {
    let (id, index) = (t.meeting_id.clone(), t.index);
    let me: RowId = (id.clone(), index);
    let open = s.tasks.board_menu.as_ref() == Some(&me);
    let here = BoardColumn::of(t);
    let toggle = {
        let me = me.clone();
        move |s: &mut State| {
            let on = s.tasks.board_menu.as_ref() != Some(&me);
            s.tasks.close_menus();
            s.tasks.board_menu = on.then(|| me.clone());
        }
    };
    let trigger = crate::ui::widgets::button(
        "Move",
        &format!("tmove-{id}-{index}"),
        crate::ui::widgets::ButtonRole::Secondary,
        p,
    )
    .label(format!(
        "Move task: {}. Now in {}.",
        shown_title(s, t),
        here.label()
    ))
    .h(28.0)
    .self_start()
    .on_press(toggle);
    let mut menu = Menu::new(format!("tmove-menu-{id}-{index}"))
        .trigger(trigger)
        .open(open)
        .placement(Placement::BottomEnd)
        .min_width(180.0);
    if open {
        menu = menu.on_close(|s: &mut State| s.tasks.board_menu = None);
        for c in BoardColumn::ALL
            .into_iter()
            .filter(|c| c.status().is_some())
        {
            let id = id.clone();
            menu = menu.item(
                MenuItem::new(c.label())
                    .checked(here == c)
                    .on_select(move |s: &mut State| s.move_card(&id, index, c)),
            );
        }
        let id = id.clone();
        menu = menu.separator().item(
            MenuItem::new(if t.done { "Mark not done" } else { "Mark done" })
                .on_select(move |s: &mut State| s.toggle_done(&id, index)),
        );
    }
    menu.build(k, layer)
}

fn card(
    s: &State,
    k: &Kit,
    t: &TaskItem,
    drag: Option<f32>,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    let (id, index) = (t.meeting_id.clone(), t.index);
    let me: RowId = (id.clone(), index);
    let tasks = &s.tasks;
    let on = t.done;
    let selected = matches!(&tasks.panel, Some(Panel::Task(r)) if *r == me);
    let at_cursor = tasks.keyboard && tasks.cursor.as_ref() == Some(&me) && !selected;
    let tick = {
        let id = id.clone();
        pressable(status_dot(on, 22.0, p))
            .role("checkbox")
            .checked(on)
            .label(t.task.clone())
            .key(format!("ttick-{id}-{index}"))
            .radius(11.0)
            .on_press(move |s: &mut State| {
                s.tasks.card_focus = Some((id.clone(), index));
                s.toggle_done(&id, index)
            })
    };
    // The grip is the one place a drag starts, so a click on the card stays a click. Drag is
    // for the wide board only; the Move button and the keys work everywhere.
    let grip = drag.map(|width| {
        let (id, from) = (id.clone(), BoardColumn::of(t));
        let over = move |d: &rune::Drag| {
            core::drop_column(from, COLUMN_PAD + CARD_PAD + d.start_x, d.dx, width, GAP)
        };
        col()
            .key(format!("tgrip-{id}-{index}"))
            .w(18.0)
            .h(26.0)
            .center()
            .cursor(rune::Cursor::Pointer)
            .help("Drag to another column")
            .child(crate::ui::icons::draw(
                &crate::ui::icons::GRIP_VERTICAL,
                16.0,
                p.faint,
            ))
            .on_drag(move |s: &mut State, d, _| match d.phase {
                rune::Phase::Down => s.tasks.board_drag = Some(((id.clone(), index), from)),
                rune::Phase::Move => {
                    s.tasks.board_drag = Some(((id.clone(), index), over(d)));
                }
                rune::Phase::Up => {
                    let to = over(d);
                    s.tasks.board_drag = None;
                    if !d.cancelled() && to != from {
                        s.move_card(&id, index, to);
                    }
                }
            })
    });
    let title = if on {
        let shown = shown_title(s, t);
        roles::SMALL
            .wrap(shown, p.faint)
            .max_lines(3)
            .strike(0, shown.len(), None)
    } else {
        roles::SMALL
            .bold()
            .wrap(shown_title(s, t), p.ink)
            .max_lines(3)
    };
    let open_title = {
        let id = id.clone();
        pressable(
            col().min_w(0.0).gap(2.0).child(title).children(
                (t.status == core::TaskStatus::Waiting && !on)
                    .then(|| t.waiting_label.clone())
                    .flatten()
                    .map(|w| roles::META.wrap(format!("Waiting on: {w}"), p.soft)),
            ),
        )
        .key(format!("ttask-{id}-{index}"))
        .label(format!("Open the task: {}", t.task))
        .grow(1.0)
        .min_w(0.0)
        .on_press(move |s: &mut State| {
            s.tasks.card_focus = Some((id.clone(), index));
            s.open_task(&id, index);
        })
    };
    let owner = match owner_name(t) {
        Some((name, me)) => row()
            .gap(6.0)
            .items_center()
            .min_w(0.0)
            .child(avatar(&name, me, 20.0, p))
            .child(roles::META.line(name, p.soft)),
        None => row()
            .min_w(0.0)
            .child(roles::META.line("No owner", p.faint)),
    };
    let when = date_chip(t, tasks.today, p);
    let source = if t.manual {
        roles::META.line("Added by hand", p.faint)
    } else {
        let (id, stamp) = (id.clone(), t.stamp.clone());
        pressable(roles::META.line(format!("{} \u{203a}", t.meeting_title), p.soft))
            .key(format!("tsource-{id}-{index}"))
            .label(format!("Open meeting: {}", t.meeting_title))
            .min_w(0.0)
            .on_press_tx(move |s: &mut State, tx| match &stamp {
                Some(stamp) => s.open_meeting_at(&id, index, stamp, tx),
                None => s.open_meeting(&id, tx),
            })
    };
    let area = row()
        .gap(5.0)
        .items_center()
        .min_w(0.0)
        .label(format!("Project: {}", t.project))
        .child(project_dot(&t.project, p))
        .child(roles::META.line(area_label(s, t), p.faint));
    let conflicts = core::task_conflict_versions(&s.relay_ui.conflicts, t);
    let toggle = (!t.subtasks.is_empty()).then(|| {
        let expanded = tasks.expanded_subtasks.contains(&me);
        let row_id = me.clone();
        pressable(
            row()
                .gap(4.0)
                .items_center()
                .child(chevron(expanded, p))
                .child(roles::META.line(format!("Subtasks {}", t.progress()), p.soft)),
        )
        .key(format!("subtasks-toggle-{id}-{index}"))
        .label(format!(
            "{} subtasks: {}",
            if expanded { "Hide" } else { "Show" },
            t.title
        ))
        .expanded(expanded)
        .on_press(move |s: &mut State| {
            s.tasks.card_focus = Some(row_id.clone());
            if !s.tasks.expanded_subtasks.remove(&row_id) {
                s.tasks.expanded_subtasks.insert(row_id.clone());
            }
        })
    });
    // On the wide board the Move button shows while the pointer is over the card, while the card
    // has the keyboard, and while its menu is open. Its room is always kept, so nothing jumps.
    // On the narrow board, which is also the touch layout, it is always there.
    let reveals = drag.is_none()
        || tasks.hovered.as_ref() == Some(&me)
        || tasks.card_focus.as_ref() == Some(&me)
        || tasks.board_menu.as_ref() == Some(&me)
        || selected
        || at_cursor;
    let move_control = move_menu(s, k, t, p, layer);
    let move_control = if reveals {
        move_control
    } else {
        move_control.opacity(0.0)
    };
    let mut body = col()
        .w_full()
        .min_w(0.0)
        .gap(6.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .children(grip)
                .child(tick)
                .child(open_title),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_center()
                .justify_between()
                .child(owner.grow(1.0))
                .children(when),
        )
        .child(source)
        .child(area)
        .children(task_tick_feedback(s, t, "board-tick", p));
    if let Some(count) = conflicts {
        body = body.child(
            roles::SMALL
                .wrap(format!("{count} versions"), p.warn)
                .role("status"),
        );
    }
    body = body.child(
        row()
            .w_full()
            .items_center()
            .justify_between()
            .gap(8.0)
            .child(row().grow(1.0).min_w(0.0).children(toggle))
            .child(move_control),
    );
    for sub in t
        .subtasks
        .iter()
        .filter(|_| tasks.expanded_subtasks.contains(&me))
    {
        let (id, sub_id, done) = (id.clone(), sub.id.clone(), sub.done);
        body = body.child(
            pressable(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_center()
                    .child(status_dot(done, 18.0, p))
                    .child(
                        roles::META
                            .wrap(&sub.title, if done { p.faint } else { p.soft })
                            .max_lines(2)
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            )
            .key(format!("subtick-{id}-{index}-{sub_id}"))
            .label(format!("Subtask: {}", sub.title))
            .role("checkbox")
            .checked(done)
            .radius(5.0)
            .hover(p.chip)
            .on_press(move |s: &mut State| {
                let stable = s
                    .tasks
                    .item(&id, index)
                    .map(|t| t.stable_id.clone())
                    .unwrap_or_default();
                s.edit_task_v2(
                    &id,
                    index,
                    core::TaskV2Edit::SubtaskTick {
                        key: core::subtask_tick_key(&stable, &sub_id),
                        done: !done,
                    },
                )
            }),
        );
    }
    let dragged = tasks.board_drag.as_ref().is_some_and(|(r, _)| *r == me);
    let mut shell = cards::panel(p)
        .key(format!("trow-{id}-{index}"))
        // When the card changes column it is rebuilt elsewhere. Focus follows it by key.
        .focus_return(format!("ttask-{id}-{index}"))
        .padding(CARD_PAD)
        .on_hover_change({
            let id = id.clone();
            move |s: &mut State, inside, _| {
                let me = Some((id.clone(), index));
                if inside {
                    s.tasks.hovered = me;
                } else if s.tasks.hovered == me {
                    s.tasks.hovered = None;
                }
            }
        })
        .child(body);
    if dragged {
        shell = shell.dim(0.5);
    }
    if selected {
        shell = shell.fill(p.sel).border(1.0, p.ink);
    } else if at_cursor {
        shell = shell.outline(1.0, 0.0, p.accent); // focus ring
    }
    shell
}

fn empty_column(drag: bool, p: Palette) -> El {
    col()
        .w_full()
        .px(10.0)
        .py(14.0)
        .radius(10.0)
        .child(roles::META.wrap(
            if drag {
                "Nothing here. Drag a card here, or use Move on a card."
            } else {
                "Nothing here. Use Move on a card to bring it here."
            },
            p.faint,
        ))
}

/// The cards of one column, a page at a time.
fn column_cards(
    s: &State,
    k: &Kit,
    view: &BoardView,
    c: BoardColumn,
    drag: Option<f32>,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    let rows = view.column(c);
    let tasks = &s.tasks;
    let mut out = col().w_full().min_w(0.0).gap(8.0);
    if c == BoardColumn::Done && view.done_folded && !tasks.done_open {
        if !rows.is_empty() {
            out = out.child(
                button(
                    format!("Show {} done", rows.len()),
                    "tasks-board-done-show",
                    false,
                    p,
                )
                .label(format!("Show {} done tasks", rows.len()))
                .on_press(|s: &mut State| s.tasks.done_open = true),
            );
        }
        return if rows.is_empty() {
            out.child(empty_column(drag.is_some(), p))
        } else {
            out
        };
    }
    if rows.is_empty() {
        return out.child(empty_column(drag.is_some(), p));
    }
    let shown = tasks.board_shown[c.index()].max(first_page(c));
    for &i in rows.iter().take(shown) {
        let t = s.shown_card(&tasks.items[i]);
        out = out.child(card(s, k, &t, drag, p, layer));
    }
    if rows.len() > shown {
        let more = (rows.len() - shown).min(PAGE);
        out = out.child(
            button(
                format!("Show {more} more"),
                format!("tasks-board-more-{}", c.index()),
                false,
                p,
            )
            .label(format!("Show {more} more in {}", c.label()))
            .on_press(move |s: &mut State| {
                let at = &mut s.tasks.board_shown[c.index()];
                *at = (*at).max(first_page(c)) + PAGE;
            }),
        );
    }
    out
}

fn heading(view: &BoardView, c: BoardColumn, s: &State, p: Palette) -> El {
    let mut r = row()
        .w_full()
        .gap(8.0)
        .items_center()
        .child(crate::ui::icons::draw(column_icon(c), 16.0, p.soft))
        .child(roles::GROUP.line(c.label(), p.ink).grow(1.0))
        .child(roles::META.line(view.column(c).len().to_string(), p.soft));
    if c == BoardColumn::Done && view.done_folded && s.tasks.done_open {
        r = r.child(
            pressable(roles::META.line("Hide", p.soft))
                .key("tasks-board-done-hide")
                .label("Hide the done tasks")
                .on_press(|s: &mut State| s.tasks.done_open = false),
        );
    }
    r.label(format!("{}, {} tasks", c.label(), view.column(c).len()))
        .key(format!("tasks-board-heading-{}", c.index()))
}

/// The whole board for one list width: four columns when they fit, otherwise one at a time
/// under four tabs.
pub fn board(s: &State, k: &Kit, p: Palette, list_w: f32, side: f32, layer: &mut Vec<El>) -> El {
    let view = s.tasks.board();
    if view.columns.iter().all(Vec::is_empty) {
        return empty_note(s, p);
    }
    if list_w >= WIDE_MIN {
        let avail = list_w - 2.0 * side - 3.0 * GAP;
        let width = (avail / 4.0).max(COLUMN_MIN);
        let mut out = row()
            .key("tasks-board")
            .gap(GAP)
            .items_start()
            .role("group")
            .label("Tasks board");
        let over = s.tasks.board_drag.as_ref().map(|(_, c)| *c);
        for c in BoardColumn::ALL {
            out = out.child(
                col()
                    .key(format!("tasks-board-column-{}", c.index()))
                    .role("group")
                    .label(c.label())
                    .w(width)
                    .gap(8.0)
                    .padding(COLUMN_PAD)
                    .radius(12.0)
                    .fill(if over == Some(c) {
                        p.sel
                    } else {
                        p.chip.alpha(0.5)
                    })
                    .border(
                        1.0,
                        if over == Some(c) {
                            p.ink
                        } else {
                            p.chip.alpha(0.0)
                        },
                    )
                    .child(heading(&view, c, s, p))
                    .child(column_cards(s, k, &view, c, Some(width), p, layer)),
            );
        }
        return out;
    }
    let tab = s.tasks.board_tab;
    let labels: Vec<String> = BoardColumn::ALL
        .iter()
        .map(|c| format!("{} {}", c.short_label(), view.column(*c).len()))
        .collect();
    let names: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut selected = k.clone();
    selected.c.bg_base = p.sel;
    selected.c.border_strong = p.ink;
    col()
        .key("tasks-board")
        .w_full()
        .gap(10.0)
        .child(
            SegmentedControl::new(&names, Some(tab.index()))
                .label("Board column")
                .width(Some((list_w - 2.0 * side).max(240.0)))
                .on_pick::<State>(|s, index, _| {
                    if let Some(c) = BoardColumn::ALL.get(index) {
                        s.tasks.board_tab = *c;
                    }
                })
                .build(&selected)
                .key("tasks-board-tabs"),
        )
        .child(heading(&view, tab, s, p))
        .child(column_cards(s, k, &view, tab, None, p, layer))
}

/// The polite line under the toolbar.
pub fn live_line(s: &State, p: Palette) -> Option<El> {
    let (line, _) = s.tasks.board_live.as_ref()?;
    Some(
        roles::SMALL
            .wrap(line, p.soft)
            .role("status")
            .key("tasks-board-live")
            .mb(8.0),
    )
}

/// The List / Board switch in the toolbar.
pub fn layout_toggle(s: &State, k: &Kit, p: Palette) -> El {
    let mut selected = k.clone();
    selected.c.bg_base = p.sel;
    selected.c.border_strong = p.ink;
    selected.c.text_muted = p.soft;
    SegmentedControl::new(
        &["List", "Board"],
        Some(usize::from(s.tasks.layout == core::TasksLayout::Board)),
    )
    .label("Tasks layout")
    .width(Some(132.0))
    .on_pick::<State>(|s, index, _| {
        s.set_layout(if index == 0 {
            core::TasksLayout::List
        } else {
            core::TasksLayout::Board
        })
    })
    .build(&selected)
    .key("tasks-layout")
}

/// Which of the three filter menus is open.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FilterMenu {
    Person,
    Area,
    Meeting,
}

/// A choice in one of the menus: the value it sets, and what it is called.
struct Choice<T> {
    value: Option<T>,
    name: String,
}

/// The first item for each key, in list order, in one pass. Items with no key are skipped.
fn first_of_each<'a, T, K: Hash + Eq>(
    items: &'a [T],
    key: impl Fn(&'a T) -> Option<K>,
) -> Vec<&'a T> {
    let mut seen = HashSet::new();
    items
        .iter()
        .filter(|t| key(*t).is_some_and(|k| seen.insert(k)))
        .collect()
}

fn people(s: &State) -> Vec<Choice<core::PersonFilter>> {
    let items = &s.tasks.items;
    let mut names: Vec<&str> = first_of_each(items, |t| t.owner.as_deref())
        .into_iter()
        .filter_map(|t| t.owner.as_deref())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    let mut out: Vec<_> = names
        .into_iter()
        .map(|n| Choice {
            name: n.to_string(),
            value: Some(core::PersonFilter::Owner(n.to_string())),
        })
        .collect();
    if items.iter().any(|t| t.owner.is_none()) {
        out.push(Choice {
            name: "No owner".into(),
            value: Some(core::PersonFilter::NoOwner),
        });
    }
    out
}

fn areas(s: &State) -> Vec<Choice<String>> {
    let mut out: Vec<_> = first_of_each(&s.tasks.items, |t| t.area_id.as_deref())
        .into_iter()
        .filter_map(|t| t.area_id.as_ref())
        .map(|id| Choice {
            name: s
                .areas
                .catalog
                .areas
                .get(id)
                .map_or_else(|| id.clone(), |a| a.name.clone()),
            value: Some(id.clone()),
        })
        .collect();
    out.sort_by_key(|c| c.name.to_lowercase());
    out
}

/// The newest meetings that have tasks, then the hand-made ones. One pass over the tasks and a
/// set of the meetings seen, so 2,000 meetings cost 2,000 lookups, not 2 million comparisons.
fn meetings(s: &State) -> Vec<Choice<String>> {
    let items = &s.tasks.items;
    let mut seen = first_of_each(items, |t| (!t.manual).then_some(t.meeting_id.as_str()));
    seen.sort_by_key(|t| t.meeting_rank);
    let mut out: Vec<_> = seen
        .into_iter()
        .take(12)
        .map(|t| Choice {
            name: match t.meeting_date {
                Some(d) => format!("{} ({})", t.meeting_title, d.format("%-d %b")),
                None => t.meeting_title.clone(),
            },
            value: Some(t.meeting_id.clone()),
        })
        .collect();
    if items.iter().any(|t| t.manual) {
        out.push(Choice {
            name: "Added by hand".into(),
            value: Some(core::MANUAL.to_string()),
        });
    }
    out
}

/// What the three filter menus offer, worked out once per change of the task list or the area
/// names and shared by every frame after.
pub struct Options {
    people: Vec<Choice<core::PersonFilter>>,
    areas: Vec<Choice<String>>,
    meetings: Vec<Choice<String>>,
}

pub struct OptionsCache {
    items_rev: u64,
    catalog: String,
    options: Rc<Options>,
}

fn options(s: &State) -> Rc<Options> {
    if let Some(c) = &*s.tasks.options_cache.borrow()
        && c.items_rev == s.tasks.items_rev
        && c.catalog == s.areas.catalog_revision
    {
        return c.options.clone();
    }
    let options = Rc::new(Options {
        people: people(s),
        areas: areas(s),
        meetings: meetings(s),
    });
    *s.tasks.options_cache.borrow_mut() = Some(OptionsCache {
        items_rev: s.tasks.items_rev,
        catalog: s.areas.catalog_revision.clone(),
        options: options.clone(),
    });
    options
}

/// What one filter menu needs: its title, its "any" line, and how it sets the choice.
struct Filter<T: 'static> {
    which: FilterMenu,
    title: &'static str,
    any: &'static str,
    set: fn(&mut State, Option<T>),
}

/// One filter menu: "Any", then a choice per value present in the tasks.
fn filter_menu<T: Clone + PartialEq + 'static>(
    s: &State,
    k: &Kit,
    spec: Filter<T>,
    current: Option<&T>,
    choices: &[Choice<T>],
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    let Filter {
        which,
        title,
        any,
        set,
    } = spec;
    let key = format!("tasks-filter-{title}").to_lowercase();
    let open = s.tasks.filter_menu == Some(which);
    let chosen = current.and_then(|c| {
        choices
            .iter()
            .find(|x| x.value.as_ref() == Some(c))
            .map(|x| x.name.clone())
    });
    let name = match &chosen {
        Some(n) => format!("{title}: {n}"),
        None => title.to_string(),
    };
    let mut menu = Menu::new(format!("{key}-menu"))
        .trigger(
            super::menu_trigger(&name, &key, p).on_press(move |s: &mut State| {
                let now = s.tasks.filter_menu != Some(which);
                s.tasks.close_menus();
                s.tasks.filter_menu = now.then_some(which);
            }),
        )
        .open(open)
        .placement(Placement::BottomStart)
        .min_width(200.0);
    if open {
        menu = menu.on_close(|s: &mut State| s.tasks.filter_menu = None);
        menu = menu.item(MenuItem::new(any).checked(current.is_none()).on_select(
            move |s: &mut State| {
                s.tasks.filter_menu = None;
                set(s, None);
            },
        ));
        for choice in choices {
            let value = choice.value.clone();
            menu = menu.item(
                MenuItem::new(choice.name.clone())
                    .checked(value.as_ref() == current)
                    .on_select(move |s: &mut State| {
                        s.tasks.filter_menu = None;
                        set(s, value.clone());
                    }),
            );
        }
    }
    menu.build(k, layer)
}

/// Person, area and meeting, for the list and the board alike. These three are for this visit
/// only, so the page never opens filtered by something forgotten.
pub fn filter_row(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let t = &s.tasks;
    let any = t.person.is_some() || t.area.is_some() || t.meeting.is_some();
    let options = options(s);
    row()
        .key("tasks-filter-row")
        .w_full()
        .flex_wrap()
        .gaps(6.0, 8.0)
        .items_center()
        .mb(10.0)
        .child(filter_menu(
            s,
            k,
            Filter {
                which: FilterMenu::Person,
                title: "Person",
                any: "Anyone",
                set: |s, v| s.tasks.person = v,
            },
            t.person.as_ref(),
            &options.people,
            p,
            layer,
        ))
        .child(filter_menu(
            s,
            k,
            Filter {
                which: FilterMenu::Area,
                title: "Area",
                any: "Any area",
                set: |s, v| s.tasks.area = v,
            },
            t.area.as_ref(),
            &options.areas,
            p,
            layer,
        ))
        .child(filter_menu(
            s,
            k,
            Filter {
                which: FilterMenu::Meeting,
                title: "Meeting",
                any: "Any meeting",
                set: |s, v| s.tasks.meeting = v,
            },
            t.meeting.as_ref(),
            &options.meetings,
            p,
            layer,
        ))
        .children(any.then(|| {
            button("Clear", "tasks-filters-clear", false, p)
                .label("Clear person, area and meeting")
                .on_press(|s: &mut State| s.tasks.clear_scope())
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_follows_a_task_to_its_new_due_date_page() {
        let mut s = State::new();
        s.view = View::Tasks;
        s.tasks.layout = core::TasksLayout::Board;
        s.tasks.done = core::DoneFilter::All;
        s.tasks.who = core::Who::Everyone;
        s.tasks.keyboard = true;
        s.tasks.items = (0..=PAGE)
            .map(|index| TaskItem {
                meeting_id: "synthetic".into(),
                meeting_title: "Test meeting".into(),
                meeting_date: None,
                meeting_rank: 0,
                index,
                task: format!("Task {index}"),
                stable_id: format!("task-{index}"),
                title: format!("Task {index}"),
                description: String::new(),
                status: Default::default(),
                waiting_on: None,
                waiting_label: None,
                subtasks: Vec::new(),
                team: String::new(),
                owner: None,
                due: None,
                due_date: Some(s.tasks.today),
                due_default: false,
                stamp: None,
                project: core::UNSORTED.into(),
                area_id: None,
                mine: None,
                done: false,
                edited: false,
                manual: false,
            })
            .collect();
        s.tasks.items_changed();
        s.reveal_card("synthetic", 0);

        let target = &mut s.tasks.items[0];
        target.due_date = s.tasks.today.succ_opt();
        target.index = PAGE + 1;
        s.tasks.items_changed();
        s.tasks.all = s.tasks.items.clone();
        let expected = (String::from("synthetic"), PAGE + 1);
        let key = format!("trow-{}-{}", expected.0, expected.1);

        let mut rt = rune::Runtime::new(
            s,
            super::super::super::view,
            (1334.0, 900.0),
            rune::Env::default(),
        );
        rt.update(|s, tx| assert!(live_tick(s, tx)));
        rt.settle_all();
        rt.frame(0.0);
        assert_eq!(
            rt.state.tasks.board_shown[BoardColumn::Todo.index()],
            PAGE + 1
        );
        assert_eq!(rt.state.tasks.cursor, Some(expected));
        assert!(rt.state.tasks.reveal.is_none());
        assert!(rt.find(&key).is_some());
    }

    /// The cards of one page: two meetings (one with dashes in its id) and a hand-made task.
    fn exists(id: &str, index: usize) -> bool {
        matches!(
            (id, index),
            ("20261112-0900", 2)
                | ("20261112-0900", 12)
                | ("20261111-1500", 0)
                | ("manual/ab-cd", 0)
        )
    }

    #[test]
    fn every_control_of_a_card_names_that_card() {
        let card = |key: &str| card_of_key(key, exists);
        let here = Some(("20261112-0900".to_string(), 2));
        for key in [
            "trow-20261112-0900-2",
            "ttask-20261112-0900-2",
            "ttick-20261112-0900-2",
            "tsource-20261112-0900-2",
            "tgrip-20261112-0900-2",
            "subtasks-toggle-20261112-0900-2",
            "tmove-20261112-0900-2",
            "tmove-menu-20261112-0900-2",
            // A subtask's id can carry dashes of its own.
            "subtick-20261112-0900-2-sub-a1-b2",
        ] {
            assert_eq!(card(key), here, "{key}");
        }
        // Index 12 is not index 2, and a hand-made task's id has a slash and a dash.
        assert_eq!(
            card("ttick-20261112-0900-12"),
            Some(("20261112-0900".to_string(), 12))
        );
        assert_eq!(
            card("ttask-manual/ab-cd-0"),
            Some(("manual/ab-cd".to_string(), 0))
        );
        // Not a card control, or a card that is gone.
        assert_eq!(card("tasks-filter-person"), None);
        assert_eq!(card("tasks-board-more-1"), None);
        assert_eq!(card("ttick-20261112-0900-7"), None);
    }

    #[test]
    fn a_move_key_acts_on_the_focused_card_not_the_cursor() {
        let cursor = Some(("20261111-1500".to_string(), 0));
        // Focus is on one card, the outlined cursor on another: the focused card moves.
        assert_eq!(
            move_target(Some("ttask-20261112-0900-2"), cursor.clone(), exists),
            Some(("20261112-0900".to_string(), 2))
        );
        // Focus on the Move button of a card, the same.
        assert_eq!(
            move_target(Some("tmove-20261112-0900-12"), cursor.clone(), exists),
            Some(("20261112-0900".to_string(), 12))
        );
        // Focus on something that is not a card never falls back to the cursor.
        assert_eq!(
            move_target(Some("tasks-filter-person"), cursor.clone(), exists),
            None
        );
        assert_eq!(move_target(Some("#3"), cursor.clone(), exists), None);
        // With nothing focused, the cursor the j and k keys show is the target.
        assert_eq!(move_target(None, cursor.clone(), exists), cursor);
        assert_eq!(move_target(None, None, exists), None);
    }

    #[test]
    fn the_first_item_of_each_key_is_kept_in_order_in_one_pass() {
        let items = [("a", 1), ("b", 2), ("a", 3), ("", 4), ("c", 5), ("b", 6)];
        let first = first_of_each(&items, |t| (!t.0.is_empty()).then_some(t.0));
        assert_eq!(first.iter().map(|t| t.1).collect::<Vec<_>>(), vec![1, 2, 5]);
        // 2,000 meetings with 5 tasks each.
        let many: Vec<(String, usize)> = (0..10_000).map(|i| (format!("m{}", i / 5), i)).collect();
        let first = first_of_each(&many, |t| Some(t.0.as_str()));
        assert_eq!(first.len(), 2_000);
        assert_eq!(first[1999].1, 9_995);
    }
}
