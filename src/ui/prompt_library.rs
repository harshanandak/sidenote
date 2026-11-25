//! Settings > Prompt library: every prompt Sidenote sends to a model, grouped by job. A list on
//! the left, the selected prompt on the right. You edit the base instructions; the parts Sidenote
//! fixes for safety and for the reply format are shown read only and never saved.
use super::{
    State,
    ai_notes::{self, Conflict, Editor, Retry},
    area::button,
    cards, roles,
    theme::Palette,
    widgets::{self, ButtonDisabled, ButtonRole},
};
use rune::{Color, El, Key, KeyEvent, Tx, col, row};
use sidenote::core::{
    agent_instructions::Agent,
    prompt_library::{self as library, Change, Chunk, Entry, Group, Storage},
};
use std::time::{Duration, Instant};

/// A window narrower than this shows the list, then the prompt, as two steps.
pub const TWO_PANE: f32 = 900.0;
const LIST_WIDTH: f32 = 272.0;
const KEY_PREFIX: &str = "prompt-row-";

#[derive(Default)]
pub struct Library {
    /// The prompt on the right, as a place in the catalog. In a wide window the first prompt is
    /// open until you pick another.
    pub selected: Option<usize>,
    /// Showing the changes from the default instead of the editor.
    pub changes: bool,
    /// Which fixed blocks of the open prompt are expanded.
    pub open: [bool; 4],
    /// Counts, token estimates and the comparison with the default for each editable prompt,
    /// in `Agent::ALL` order. Drawing only reads these. They are rebuilt when the text changes.
    pub derived: [Derived; Agent::ALL.len()],
    /// The blocks of Redo tasks in reading order, with your saved Sort tasks instructions
    /// between the fixed ones. Rebuilt when those instructions load or save.
    pub shared: Vec<library::Part>,
}

/// How long typing must pause before the counts are worked out again.
const DEBOUNCE: Duration = Duration::from_millis(180);
/// The most diff rows drawn. A huge edit stays quick; Copy prompt still has all of it.
const MAX_ROWS: usize = 300;

/// A comparison of two texts, kept so drawing never works it out again.
pub struct DiffCache {
    /// The right hand text this comparison was built from.
    source: String,
    chunks: Vec<Chunk>,
    added: usize,
    removed: usize,
    /// Rows left out past `MAX_ROWS`.
    hidden: usize,
}
fn build_diff(left: &str, right: &str) -> DiffCache {
    let lines = library::diff(left, right);
    let (added, removed) = library::changed(&lines);
    let mut chunks = library::with_context(&lines, 2);
    let hidden = chunks.len().saturating_sub(MAX_ROWS);
    chunks.truncate(MAX_ROWS);
    DiffCache {
        source: right.to_owned(),
        chunks,
        added,
        removed,
        hidden,
    }
}

/// What the page shows about one editor's text, worked out once per change.
#[derive(Default)]
pub struct Derived {
    /// The text the numbers below describe.
    source: Option<String>,
    chars: usize,
    tokens: usize,
    whole: usize,
    bad: bool,
    /// When the text was first seen to differ from `source`, while the debounce runs.
    waiting: Option<Instant>,
    diff: Option<DiffCache>,
}
impl Derived {
    fn valid(&self) -> bool {
        !self.bad
    }
    fn fill(&mut self, entry: &Entry, text: &str) {
        self.chars = text.chars().count();
        self.bad = !ai_notes::valid_instructions(text);
        self.tokens = library::tokens(text);
        self.whole = library::tokens(&entry.assembled(text));
        self.source = Some(text.to_owned());
        self.waiting = None;
    }
    fn is_for(&self, text: &str) -> bool {
        self.source.as_deref() == Some(text)
    }
}
/// Whether changed text should be worked out now. The first look, a headless render and a pause
/// long enough all say yes. Typing in between waits.
fn due(first: bool, headless: bool, waited: Duration) -> bool {
    first || headless || waited >= DEBOUNCE
}
/// Works out the numbers for one editor now, and the comparison when that prompt is showing it.
pub(super) fn refresh(s: &mut State, ei: usize) {
    let entry = library::for_agent(Agent::ALL[ei]);
    let showing = current(s)
        .and_then(|i| library::catalog().get(i))
        .and_then(editor_index)
        == Some(ei);
    let a = &mut s.ai_notes;
    let text = &a.editors[ei].text.text;
    let lib = &mut a.library;
    let d = &mut lib.derived[ei];
    d.fill(entry, text);
    if lib.changes && showing {
        d.diff = Some(build_diff(&entry.default_base(), text));
    }
    if entry.agent() == Some(Agent::Tasks) {
        refresh_shared(s);
    }
}
fn refresh_shared(s: &mut State) {
    let Some(entry) = library::find("tasks-redo") else {
        return;
    };
    let Some(from) = entry.shares().and_then(agent_index) else {
        return;
    };
    let base = library::normalize(&s.ai_notes.editors[from].saved);
    s.ai_notes.library.shared = entry.parts_with(&base);
}
/// Runs once a frame. Typing is picked up here: the counts follow a light debounce, so a held
/// key never works out a whole prompt per character. Returns true when it changed something.
pub fn tick(s: &mut State, tx: &mut Tx) -> bool {
    if !s.on_hex() {
        return false;
    }
    let Some(entry) = current(s).and_then(|i| library::catalog().get(i)) else {
        return false;
    };
    let Some(ei) = editor_index(entry) else {
        return false;
    };
    let headless = rune::tooling::headless_run().is_some();
    let a = &mut s.ai_notes;
    let text = &a.editors[ei].text.text;
    let lib = &mut a.library;
    let d = &mut lib.derived[ei];
    let mut changed = false;
    if !d.is_for(text) {
        let now = Instant::now();
        let since = *d.waiting.get_or_insert(now);
        let waited = now.saturating_duration_since(since);
        if due(d.source.is_none(), headless, waited) {
            d.fill(entry, text);
            changed = true;
        } else {
            tx.wake_after(DEBOUNCE - waited);
        }
    }
    if lib.changes && d.diff.as_ref().is_none_or(|x| x.source != *text) {
        d.diff = Some(build_diff(&entry.default_base(), text));
        changed = true;
    }
    changed
}

pub fn single_pane(s: &State) -> bool {
    s.app_w.get() < TWO_PANE
}
fn current(s: &State) -> Option<usize> {
    let selected = s.ai_notes.library.selected;
    if single_pane(s) {
        selected
    } else {
        Some(selected.unwrap_or(0))
    }
}
pub fn select(s: &mut State, index: usize) {
    let entry = &library::catalog()[index];
    let lib = &mut s.ai_notes.library;
    lib.selected = Some(index);
    lib.changes = false;
    // A read-only prompt is nothing but its fixed blocks, so they open by default.
    lib.open = [!entry.editable(); 4];
    if let Some(ei) = editor_index(entry) {
        refresh(s, ei);
    } else if entry.shares().is_some() {
        refresh_shared(s);
    }
}
fn back(s: &mut State) {
    s.ai_notes.library.selected = None;
    s.ai_notes.library.changes = false;
}
/// Moves the selection one prompt in list order, which is the order the groups are listed in.
fn step(s: &mut State, delta: isize) {
    let order: Vec<usize> = Group::ALL
        .into_iter()
        .flat_map(|g| library::in_group(g).map(|(i, _)| i))
        .collect();
    let at = current(s).and_then(|i| order.iter().position(|o| *o == i));
    let next = match at {
        Some(at) => (at as isize + delta).clamp(0, order.len() as isize - 1) as usize,
        None => 0,
    };
    select(s, order[next]);
}

/// Arrow keys move through the list while the list or nothing has focus. In a narrow window
/// Escape goes from the editor back to the list.
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    let in_list = tx.focused().is_none_or(|f| f.starts_with(KEY_PREFIX));
    match e.key {
        Key::Escape if single_pane(s) && s.ai_notes.library.selected.is_some() => {
            if s.ai_notes.library.changes {
                s.ai_notes.library.changes = false;
            } else {
                back(s);
            }
            tx.blur();
            true
        }
        Key::Up | Key::Down if in_list && !e.mods.ctrl && !e.mods.alt && !e.mods.cmd => {
            if single_pane(s) && s.ai_notes.library.selected.is_none() {
                return false;
            }
            step(s, if e.key == Key::Up { -1 } else { 1 });
            true
        }
        _ => false,
    }
}

fn editor_index(entry: &Entry) -> Option<usize> {
    entry
        .agent()
        .and_then(|a| Agent::ALL.iter().position(|x| *x == a))
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Default,
    Edited,
    Unsaved,
    ReadOnly,
}
impl Status {
    fn word(self) -> &'static str {
        match self {
            Status::Default => "Default",
            Status::Edited => "Edited",
            Status::Unsaved => "Unsaved",
            Status::ReadOnly => "Read only",
        }
    }
    fn badge(self, p: Palette) -> El {
        let (ink, fill) = match self {
            Status::Default => (p.soft, p.chip),
            Status::Edited => (p.ink, p.sel),
            Status::Unsaved => (p.warn, p.warnbg),
            Status::ReadOnly => (p.faint, p.chip),
        };
        widgets::badge(self.word(), ink, Some(fill))
    }
}
fn status(s: &State, entry: &Entry) -> Status {
    match editor_index(entry).map(|i| &s.ai_notes.editors[i]) {
        None => Status::ReadOnly,
        Some(e) if e.dirty() => Status::Unsaved,
        Some(e) if e.edited_at.is_some() => Status::Edited,
        Some(_) => Status::Default,
    }
}

pub fn view(s: &State, p: Palette) -> El {
    let a = &s.ai_notes;
    let mut c = col()
        .key("agent-instructions")
        .fill(super::hex::settings::group_highlight(
            s,
            "agent-instructions",
            p,
        ))
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(12.0)
        .pt(4.0)
        .child(roles::SMALL.wrap(
            "Change how your agent writes. Each prompt has instructions you can edit and parts Sidenote keeps fixed for safety and for the reply format. Edited prompts sync to your other devices.",
            p.soft,
        ));
    if let Some(error) = &a.error
        && a.editor_target.is_none()
    {
        c = c.child(widgets::banner(
            "Prompts need attention",
            error,
            widgets::BannerKind::Problem,
            p,
        ));
    }
    if a.settings.is_none() && a.pending.is_none() {
        c = c.child(
            button("Reload agent status", "ai-notes-reload", p)
                .self_start()
                .on_press(|s: &mut State| s.refresh_ai_notes()),
        );
    }
    if let Some(pending) = a.pending
        && a.editor_target.is_none()
    {
        c = c.child(roles::SMALL.wrap(pending, p.soft).role("status"));
    }
    match (single_pane(s), current(s)) {
        (false, Some(index)) => c.child(
            row()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .gap(16.0)
                .child(
                    col()
                        .key("prompt-list-pane")
                        .w(LIST_WIDTH)
                        .h_full()
                        .min_h(0.0)
                        .child(widgets::scroll(list(s, p), "prompt-list-scroll")),
                )
                .child(panel(s, p, index).grow(1.0).min_w(0.0).h_full().min_h(0.0)),
        ),
        (true, Some(index)) => c
            .child(
                widgets::pressable(
                    row()
                        .items_center()
                        .gap(6.0)
                        .py(6.0)
                        .child(roles::CONTROL.icon_label("\u{2039}", p.soft))
                        .child(roles::CONTROL.line("All prompts", p.soft)),
                )
                .key("prompt-back")
                .label("Back to all prompts (Esc)")
                .self_start()
                .on_press(back),
            )
            .child(panel(s, p, index).w_full().grow(1.0).min_h(0.0)),
        (_, None) => c.child(widgets::scroll(list(s, p), "prompt-list-scroll")),
    }
}

fn list(s: &State, p: Palette) -> El {
    let selected = current(s);
    let mut c = col()
        .key("prompt-list")
        .w_full()
        .gap(2.0)
        .pb(24.0)
        .role("list");
    for (n, group) in Group::ALL.into_iter().enumerate() {
        c = c.child(
            roles::GROUP
                .line(group.label(), p.faint)
                .role("heading")
                .px(10.0)
                .mt(if n == 0 { 0.0 } else { 16.0 })
                .mb(4.0),
        );
        for (index, entry) in library::in_group(group) {
            let state = status(s, entry);
            let open = selected == Some(index);
            c = c.child(
                widgets::pressable(
                    col()
                        .w_full()
                        .gap(3.0)
                        .px(10.0)
                        .py(8.0)
                        .child(
                            row()
                                .w_full()
                                .items_center()
                                .gap(8.0)
                                .child(roles::NAME.line(entry.name, p.ink).grow(1.0))
                                .child(state.badge(p)),
                        )
                        .child(roles::META.line(entry.purpose, p.soft)),
                )
                .key(format!("{KEY_PREFIX}{}", entry.id))
                .label(format!(
                    "{}, {}. {}{}",
                    entry.name,
                    state.word(),
                    entry.purpose,
                    if open { " Open." } else { "" }
                ))
                .radius(8.0)
                .fill(if open { p.sel } else { Color::TRANSPARENT })
                .hover(p.chip)
                .focus_ring(2.0, 1.0, p.soft)
                .on_press(move |s: &mut State| select(s, index)),
            );
        }
    }
    c
}

fn storage_badge(storage: Storage, p: Palette) -> El {
    match storage {
        Storage::Synced | Storage::Shared => widgets::badge(storage.label(), p.ok, Some(p.okbg)),
        Storage::ThisDevice | Storage::BuiltIn => {
            widgets::badge(storage.label(), p.soft, Some(p.chip))
        }
    }
}
/// The whole prompt as Sidenote sends it, apart from the meeting material. Built when you press
/// Copy, never while drawing. An editable prompt includes your unsaved text. A prompt sent with
/// another prompt's instructions (Redo tasks) includes that prompt's saved copy.
fn prompt_text(s: &State, index: usize) -> String {
    let entry = &library::catalog()[index];
    let editors = &s.ai_notes.editors;
    match (editor_index(entry), entry.shares().and_then(agent_index)) {
        (Some(ei), _) => entry.assembled(&editors[ei].text.text),
        (None, Some(from)) => entry.assembled(&library::normalize(&editors[from].saved)),
        (None, None) => entry.assembled(""),
    }
}
fn agent_index(agent: Agent) -> Option<usize> {
    Agent::ALL.iter().position(|x| *x == agent)
}
fn copy_button(index: usize, p: Palette) -> El {
    button("Copy prompt", "prompt-copy", p).on_press_tx(move |s: &mut State, tx| {
        let text = prompt_text(s, index);
        tx.copy(text);
        s.show_toast("Copied the whole prompt", tx);
    })
}

fn panel(s: &State, p: Palette, index: usize) -> El {
    let entry = &library::catalog()[index];
    let state = status(s, entry);
    let editor = editor_index(entry);
    let meta = match editor.map(|i| &s.ai_notes.editors[i]) {
        Some(e) => format!(
            "{}. {}",
            ai_notes::edit_time(e.edited_at.as_deref()),
            entry.storage.detail()
        ),
        None => entry.storage.detail().to_owned(),
    };
    let mut top = col()
        .w_full()
        .gap(6.0)
        .px(16.0)
        .pt(14.0)
        .pb(12.0)
        .child(
            row()
                .w_full()
                .items_center()
                .gaps(6.0, 8.0)
                .flex_wrap()
                .child(roles::TITLE.wrap(entry.name, p.ink).role("heading"))
                .child(state.badge(p))
                .child(storage_badge(entry.storage, p)),
        )
        .child(roles::SMALL.wrap(entry.purpose, p.soft))
        .child(roles::META.wrap(meta, p.faint));
    let mut body = col().w_full().gap(14.0).px(16.0).pt(14.0).pb(20.0);
    match editor {
        Some(ei) => {
            let e = &s.ai_notes.editors[ei];
            let d = &s.ai_notes.library.derived[ei];
            top = top.child(toolbar(s, index, ei, e, d, p));
            top = top.child(counts(entry, ei, d, e.dirty(), p));
            body = editable_body(s, entry, ei, e, d, body, p);
        }
        None => {
            top = top.child(
                row()
                    .gaps(8.0, 8.0)
                    .flex_wrap()
                    .child(copy_button(index, p)),
            );
            if let Some(reason) = entry.reason() {
                body = body.child(widgets::banner(
                    if entry.fixed_parts().is_empty() {
                        "Nothing is sent to a model"
                    } else {
                        "Why this prompt is read only"
                    },
                    reason,
                    widgets::BannerKind::Info,
                    p,
                ));
            }
            if let Some(from) = entry.shares().map(library::for_agent) {
                let target = library::catalog()
                    .iter()
                    .position(|e| e.id == from.id)
                    .expect("shared prompt is listed");
                body = body.child(
                    button(format!("Edit {}", from.name), "prompt-open-shared", p)
                        .self_start()
                        .on_press(move |s: &mut State| select(s, target)),
                );
            }
        }
    }
    if entry.id == "tasks-sort" {
        body = body.child(teams_note(s, p));
    }
    let editable = entry.editable();
    if !editable && !entry.fixed_parts().is_empty() {
        body = body.child(fixed_page(s, entry, p));
    }
    cards::panel(p)
        .key(format!("prompt-panel-{}", entry.id))
        .child(top)
        .child(widgets::hairline(p))
        .child(widgets::scroll(
            body,
            &format!("prompt-scroll-{}", entry.id),
        ))
        .children(editable.then(|| widgets::hairline(p)))
        .children(editable.then(|| fixed_footer(s, entry, p)))
}

fn toolbar(s: &State, index: usize, ei: usize, e: &Editor, d: &Derived, p: Palette) -> El {
    let a = &s.ai_notes;
    // A conflict has to be settled first, so neither Save nor Reset can run past it.
    let ready = a.pending.is_none() && a.settings.is_some() && e.conflict.is_none();
    let valid = d.valid();
    let dirty = e.dirty();
    let changes = a.library.changes;
    let save = widgets::button(
        "Save instructions",
        &format!("instructions-save-{ei}"),
        if dirty {
            ButtonRole::Primary
        } else {
            ButtonRole::Ghost
        },
        p,
    );
    let save = if ready && valid && dirty {
        save.on_press(move |s: &mut State| s.notes_save_editor(ei))
    } else {
        save.button_disabled(true)
    };
    let reset = button("Reset to default", &format!("instructions-reset-{ei}"), p);
    let reset = if ready && (e.edited_at.is_some() || dirty) {
        reset.on_press(move |s: &mut State| s.ai_notes.editors[ei].reset_confirm = true)
    } else {
        reset.button_disabled(true)
    };
    let view = button(
        if changes {
            "Hide changes"
        } else {
            "View changes"
        },
        "prompt-changes",
        p,
    )
    .on_press(move |s: &mut State| {
        s.ai_notes.library.changes = !s.ai_notes.library.changes;
        // The comparison is built here, once, so drawing only reads it.
        refresh(s, ei);
    });
    row()
        .w_full()
        .gaps(8.0, 8.0)
        .flex_wrap()
        .child(save)
        .child(reset)
        .child(view)
        .child(copy_button(index, p))
}

fn counts(entry: &Entry, ei: usize, d: &Derived, dirty: bool, p: Palette) -> El {
    let valid = d.valid();
    let line = if d.chars > library::MAX_CHARS {
        let over = d.chars - library::MAX_CHARS;
        format!(
            "{over} character{} over the limit. Remove at least {over} to save.",
            if over == 1 { "" } else { "s" }
        )
    } else if !valid {
        "This text contains a character that cannot be saved. Remove it and try again.".into()
    } else {
        format!(
            "{} of {} characters used.{}",
            ai_notes::count_text(d.chars),
            ai_notes::count_text(library::MAX_CHARS),
            if dirty { " Unsaved changes." } else { "" }
        )
    };
    let mut c = col()
        .w_full()
        .gap(2.0)
        .key(format!("instructions-count-{ei}"))
        .child(
            roles::META
                .wrap(line, if valid { p.soft } else { p.warn })
                .help(format!(
                    "Estimated tokens: {} in your instructions, {} in the whole prompt.",
                    ai_notes::count_text(d.tokens),
                    ai_notes::count_text(d.whole),
                )),
        );
    if entry.id == "meeting-summary" {
        c = c.child(roles::META.wrap("Summaries are written in English.", p.soft));
    }
    c
}

fn editable_body(
    s: &State,
    entry: &Entry,
    ei: usize,
    e: &Editor,
    d: &Derived,
    mut body: El,
    p: Palette,
) -> El {
    let a = &s.ai_notes;
    if a.editor_target == Some(ei) {
        body = body
            .children(
                a.pending
                    .map(|text| roles::SMALL.wrap(text, p.soft).role("status")),
            )
            .children(a.error.as_ref().map(|text| {
                widgets::banner(
                    "Instructions were not saved",
                    text,
                    widgets::BannerKind::Problem,
                    p,
                )
            }))
            .children(
                a.notice
                    .as_ref()
                    .map(|text| roles::SMALL.wrap(text, p.ok).role("status")),
            );
    }
    if let Some(conflict) = &e.conflict {
        body = body.child(conflict_panel(ei, conflict, p));
    }
    if e.reset_confirm {
        body = body
            .child(roles::SMALL.wrap(
                "Replace your instructions with the default? Your saved copy and unsaved changes will be removed.",
                p.warn,
            ))
            .child(
                row()
                    .flex_wrap()
                    .gaps(8.0, 8.0)
                    .child(
                        button("Reset instructions", &format!("instructions-confirm-{ei}"), p)
                            .color(p.warn)
                            .on_press(move |s: &mut State| s.notes_reset_editor(ei)),
                    )
                    .child(
                        button(
                            "Keep my instructions",
                            &format!("instructions-cancel-{ei}"),
                            p,
                        )
                        .fill(p.sel)
                        .on_press(move |s: &mut State| s.ai_notes.editors[ei].reset_confirm = false),
                    ),
            );
    }
    if a.library.changes {
        body.child(changes_view(e, d, p))
    } else {
        body.child(input(s, entry, ei, e, d, p))
    }
}

/// Another window saved this prompt after this one started editing. Nothing was written. The
/// three choices are the only way forward.
fn conflict_panel(ei: usize, c: &Conflict, p: Palette) -> El {
    let when = match &c.theirs {
        Some(theirs) => ai_notes::edit_time(Some(theirs.edited_at.as_str())),
        None => "It was reset to the default".into(),
    };
    let ask = match c.retry {
        Retry::Save => "Keep your text, use the other copy, or look at both first.",
        Retry::Reset => {
            "You asked to reset to the default. Keep that, use the other copy, or look at both first."
        }
    };
    let mut panel = col()
        .key(format!("prompt-conflict-{ei}"))
        .w_full()
        .gap(10.0)
        .child(widgets::banner(
            "Another window changed this prompt",
            format!("{when}, after you started editing. Nothing was overwritten. {ask}"),
            widgets::BannerKind::Problem,
            p,
        ))
        .child(
            row()
                .flex_wrap()
                .gaps(8.0, 8.0)
                .child(
                    widgets::button(
                        "Keep mine",
                        &format!("prompt-conflict-mine-{ei}"),
                        ButtonRole::Primary,
                        p,
                    )
                    .on_press(move |s: &mut State| s.notes_keep_mine(ei)),
                )
                .child(
                    button("Use theirs", &format!("prompt-conflict-theirs-{ei}"), p)
                        .on_press(move |s: &mut State| s.notes_use_theirs(ei)),
                )
                .child(
                    button(
                        if c.both { "Hide both" } else { "View both" },
                        &format!("prompt-conflict-both-{ei}"),
                        p,
                    )
                    .on_press(move |s: &mut State| toggle_both(s, ei)),
                ),
        );
    if c.both
        && let Some(compare) = &c.compare
    {
        panel = panel
            .child(roles::GROUP.wrap("Theirs compared with yours", p.ink))
            .child(
                roles::META
                    .wrap(
                        format!(
                            "Lines only in theirs are marked \u{2212} ({}). Lines only in yours are marked + ({}).",
                            compare.removed, compare.added
                        ),
                        p.soft,
                    )
                    .role("status"),
            )
            .child(diff_rows(compare, p));
    }
    panel
}
/// Builds the side by side comparison once, when you open it.
fn toggle_both(s: &mut State, ei: usize) {
    let entry = library::for_agent(Agent::ALL[ei]);
    let editor = &mut s.ai_notes.editors[ei];
    let mine = match editor.conflict.as_ref().map(|c| c.retry) {
        Some(Retry::Reset) => entry.default_base(),
        _ => editor.text.text.clone(),
    };
    let Some(conflict) = editor.conflict.as_mut() else {
        return;
    };
    conflict.both = !conflict.both;
    if conflict.both && conflict.compare.as_ref().is_none_or(|c| c.source != mine) {
        let theirs = library::base_of(entry, conflict.theirs.as_ref());
        conflict.compare = Some(build_diff(&theirs, &mine));
    }
}

/// The editable area. It grows with its text, so the page scrolls as one instead of the text
/// scrolling inside a small box, and it is at least about as tall as the window.
fn input(s: &State, entry: &Entry, ei: usize, e: &Editor, d: &Derived, p: Palette) -> El {
    let valid = d.valid();
    let editable = s.ai_notes.pending.is_none() && s.ai_notes.settings.is_some();
    let lines = (((s.app_h.get() - 360.0) / roles::SMALL.lh).floor() as usize).clamp(10, 60);
    let field = rune::text_input(&e.text, roles::SMALL.style())
        .key(format!("instructions-input-{ei}"))
        .color(p.ink)
        .wrap_anywhere()
        .caret(p.ink, p.hit)
        .label(format!("{} instructions", entry.name))
        .on_edit(move |s: &mut State| &mut s.ai_notes.editors[ei].text)
        .on_submit(move |s: &mut State, _| s.ai_notes.editors[ei].text.insert("\n"))
        .w_full()
        .min_w(0.0)
        .min_h(lines as f32 * roles::SMALL.lh + 24.0)
        .px(14.0)
        .py(12.0)
        .fill(p.bg)
        .border(1.0, if valid { p.line } else { p.warn })
        .radius(8.0);
    let field = if editable { field } else { field.dim(0.6) };
    col()
        .w_full()
        .gap(6.0)
        .child(roles::GROUP.wrap("Your instructions", p.ink))
        .child(rune::auto_grow(field, lines, 20_000))
}

/// The comparison with the default, read from the cache that `refresh` and `tick` keep.
fn changes_view(e: &Editor, d: &Derived, p: Palette) -> El {
    let mut c = col()
        .key("prompt-diff")
        .w_full()
        .gap(6.0)
        .child(roles::GROUP.wrap("Your instructions compared with the default", p.ink));
    let Some(diff) = d.diff.as_ref().filter(|x| x.source == e.text.text) else {
        return c.child(
            roles::META
                .wrap("Comparing with the default...", p.soft)
                .role("status"),
        );
    };
    let (added, removed) = (diff.added, diff.removed);
    let summary = if added + removed == 0 {
        "No changes. These instructions match the default.".to_owned()
    } else {
        format!(
            "{added} line{} added, {removed} removed.{}",
            if added == 1 { "" } else { "s" },
            if e.dirty() {
                " This includes changes you have not saved."
            } else {
                ""
            }
        )
    };
    c = c.child(roles::META.wrap(summary, p.soft).role("status"));
    if added + removed > 0 {
        c = c.child(diff_rows(diff, p));
    }
    c
}
/// The rows of a cached comparison. At most `MAX_ROWS` are drawn, so a huge edit stays quick.
fn diff_rows(diff: &DiffCache, p: Palette) -> El {
    let mut rows = col()
        .w_full()
        .py(4.0)
        .radius(8.0)
        .clip_rounded(8.0)
        .border(1.0, p.line)
        .fill(p.bg);
    for chunk in &diff.chunks {
        rows = rows.child(match chunk {
            Chunk::Skipped(n) => roles::META
                .wrap(
                    format!(
                        "{n} unchanged line{} not shown",
                        if *n == 1 { "" } else { "s" }
                    ),
                    p.faint,
                )
                .px(12.0)
                .py(4.0),
            Chunk::Line(line) => {
                let (sign, ink, fill) = match line.change {
                    Change::Added => ("+", p.ok, p.okbg),
                    Change::Removed => ("\u{2212}", p.warn, p.warnbg),
                    Change::Same => (" ", p.faint, Color::TRANSPARENT),
                };
                row()
                    .w_full()
                    .items_start()
                    .gap(8.0)
                    .px(12.0)
                    .py(2.0)
                    .fill(fill)
                    .child(roles::CONTROL.line(sign, ink).w(12.0))
                    .child(
                        roles::SMALL
                            .wrap(
                                line.text.clone(),
                                if line.change == Change::Same {
                                    p.soft
                                } else {
                                    p.ink
                                },
                            )
                            .wrap_anywhere()
                            .grow(1.0)
                            .min_w(0.0)
                            .min_h(roles::SMALL.lh),
                    )
            }
        });
    }
    if diff.hidden > 0 {
        rows = rows.child(
            roles::META
                .wrap(
                    format!(
                        "{} more rows not shown. Copy the prompt to read all of it.",
                        diff.hidden
                    ),
                    p.faint,
                )
                .px(12.0)
                .py(4.0),
        );
    }
    rows
}

fn fixed_title(p: Palette) -> El {
    row()
        .items_center()
        .gap(6.0)
        .child(super::icons::draw(&super::icons::LOCK, 14.0, p.soft))
        .child(
            roles::GROUP
                .wrap("Fixed by Sidenote", p.ink)
                .role("heading"),
        )
}
fn fixed_block(text: String, p: Palette) -> El {
    col()
        .w_full()
        .px(14.0)
        .py(12.0)
        .radius(8.0)
        .fill(p.bg)
        .border(1.0, p.line)
        .child(
            roles::SMALL
                .wrap(text, p.soft)
                .wrap_anywhere()
                .selectable(true),
        )
}
fn toggle_fixed(s: &mut State, i: usize) {
    let open = &mut s.ai_notes.library.open[i];
    *open = !*open;
}

/// A read-only prompt is all fixed text, so it fills the page under its reason.
fn fixed_page(s: &State, entry: &Entry, p: Palette) -> El {
    let mut c = col()
        .key("prompt-fixed")
        .w_full()
        .gap(2.0)
        .child(fixed_title(p))
        .child(roles::META.wrap(
            if entry.shares().is_some() {
                "Shown in the order Sidenote sends it, apart from the meeting material it adds. The middle block is the saved copy of your instructions, not unsaved edits."
            } else {
                "Shown exactly as Sidenote sends it, apart from the meeting material it adds."
            },
            p.soft,
        ));
    let shared = &s.ai_notes.library.shared;
    let parts: &[library::Part] = if entry.shares().is_some() && !shared.is_empty() {
        shared
    } else {
        entry.fixed_parts()
    };
    for (i, part) in parts.iter().enumerate().take(4) {
        let open = s.ai_notes.library.open[i];
        c = c.child(
            cards::fold_control(part.label, &format!("prompt-fixed-{i}"), open, p)
                .on_press(move |s: &mut State| toggle_fixed(s, i)),
        );
        if open {
            c = c.child(fixed_block(part.text.clone(), p));
        }
    }
    c
}

/// An editable prompt keeps its fixed parts one line below the editor, collapsed, so the whole
/// prompt is a click away however long your instructions are.
fn fixed_footer(s: &State, entry: &Entry, p: Palette) -> El {
    let mut toggles = row().w_full().items_center().gaps(4.0, 6.0).flex_wrap();
    let mut blocks = col().w_full().gap(8.0);
    let mut any = false;
    for (i, part) in entry.fixed_parts().iter().enumerate().take(4) {
        let open = s.ai_notes.library.open[i];
        toggles = toggles.child(
            widgets::pressable(
                row()
                    .items_center()
                    .gap(4.0)
                    .px(8.0)
                    .py(5.0)
                    .child(roles::CONTROL.line(if open { "\u{25be}" } else { "\u{25b8}" }, p.soft))
                    .child(roles::CONTROL.line(part.label, p.ink)),
            )
            .key(format!("prompt-fixed-{i}"))
            .label(format!("{} (read only)", part.label))
            .expanded(open)
            .radius(6.0)
            .border(1.0, p.line)
            .hover(p.chip)
            .focus_ring(2.0, 1.0, p.soft)
            .on_press(move |s: &mut State| toggle_fixed(s, i)),
        );
        if open {
            any = true;
            blocks = blocks
                .child(roles::GROUP.wrap(part.label, p.soft))
                .child(fixed_block(part.text.clone(), p));
        }
    }
    col()
        .key("prompt-fixed")
        .w_full()
        .gap(8.0)
        .px(16.0)
        .py(10.0)
        .child(fixed_title(p))
        .child(
            roles::META
                .wrap(
                    "Read only. Sidenote adds these around your instructions, so a saved edit can never remove them.",
                    p.soft,
                )
                .w_full(),
        )
        .child(toggles)
        .children(any.then(|| {
            col()
                .key("prompt-fixed-scroll")
                .w_full()
                .shrink()
                .max_h(170.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(blocks)
        }))
}

fn teams_note(s: &State, p: Palette) -> El {
    col()
        .w_full()
        .gap(8.0)
        .child(roles::SMALL.wrap(
            if s.tasks.teams.teams.is_empty() {
                "No teams yet. Tasks will not be given a team."
            } else {
                "Tasks use the team of the person who owns them."
            },
            p.soft,
        ))
        .children(
            (s.config.features.tasks && s.config.features.people).then(|| {
                button("Set up teams", "notes-teams", p)
                    .self_start()
                    .on_press(|s: &mut State| s.open_people_tab(0))
            }),
        )
}

/// Invented states for the render harness. The isolated state holds no saved instructions, so
/// the scenes set what a few edited prompts would look like after loading.
pub fn scene(s: &mut State, name: &str) {
    let edit = |s: &mut State, id: &str, text: String| {
        let entry = library::find(id).expect("scene prompt");
        let e = &mut s.ai_notes.editors[editor_index(entry).expect("editable prompt")];
        e.text = rune::TextEdit::new(&text);
        e.saved = text;
        e.edited_at = Some("2026-11-10T04:00:00Z".into());
    };
    let summary = library::find("meeting-summary").unwrap();
    let mut lines: Vec<String> = summary.default_base().lines().map(str::to_owned).collect();
    lines[0] = "Write as someone who sat in the meeting and explains it to a colleague who did not: short, plain and accurate. Write the notes in English, even when the transcript is in another language.".into();
    lines.retain(|l| !l.starts_with("- Note the intent behind the talk"));
    lines.push("- Keep every number and date exactly as said.".into());
    edit(s, "meeting-summary", lines.join("\n"));
    edit(
        s,
        "ask-meeting",
        "Lead with the answer in one or two sentences. Use British spelling.".into(),
    );
    let index = |id: &str| {
        library::catalog()
            .iter()
            .position(|e| e.id == id)
            .expect("scene prompt")
    };
    for ei in 0..Agent::ALL.len() {
        refresh(s, ei);
    }
    match name {
        "settings-prompts" => {}
        "settings-prompts-edited" => select(s, index("meeting-summary")),
        "settings-prompts-diff" => {
            select(s, index("meeting-summary"));
            s.ai_notes.library.changes = true;
            refresh(s, 0);
        }
        "settings-prompts-redo" => {
            let tasks = library::find("tasks-sort").unwrap();
            let e = &mut s.ai_notes.editors[editor_index(tasks).unwrap()];
            e.text = rune::TextEdit::new("Group by client, never by month.");
            e.saved = e.text.text.clone();
            e.edited_at = Some("2026-11-10T04:00:00Z".into());
            refresh(s, editor_index(tasks).unwrap());
            select(s, index("tasks-redo"));
            s.ai_notes.library.open = [true; 4];
        }
        "settings-prompts-conflict" => {
            let ask = library::find("ask-meeting").unwrap();
            let ei = editor_index(ask).unwrap();
            let e = &mut s.ai_notes.editors[ei];
            e.text = rune::TextEdit::new(
                "Lead with the answer in one sentence. Use British spelling and name the speaker.",
            );
            let theirs = sidenote::core::agent_instructions::Entry {
                text: "Lead with the answer in one or two sentences. Use American spelling.".into(),
                based_on: 1,
                edited_at: "2026-11-11T09:30:00Z".into(),
            };
            let mut conflict = Conflict::new(Some(theirs), Retry::Save);
            conflict.both = true;
            conflict.compare = Some(build_diff(
                &library::base_of(ask, conflict.theirs.as_ref()),
                &e.text.text,
            ));
            e.conflict = Some(conflict);
            refresh(s, ei);
            select(s, index("ask-meeting"));
        }
        "settings-prompts-sort" => {
            select(s, index("tasks-sort"));
            s.ai_notes.library.open[0] = true;
        }
        "settings-prompts-readonly" => select(s, index("daily-check")),
        _ => panic!("Unknown prompt library scene"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_editable_prompt_has_one_editor_and_read_only_prompts_have_none() {
        let mut slots = Vec::new();
        for entry in library::catalog() {
            match editor_index(entry) {
                Some(i) => {
                    assert!(entry.editable(), "{}", entry.id);
                    slots.push(i);
                }
                None => assert!(!entry.editable(), "{}", entry.id),
            }
        }
        slots.sort_unstable();
        assert_eq!(slots, (0..Agent::ALL.len()).collect::<Vec<_>>());
    }

    #[test]
    fn a_read_only_prompt_opens_its_fixed_blocks_and_an_editable_one_keeps_them_closed() {
        let mut state = State::new();
        for (index, entry) in library::catalog().iter().enumerate() {
            select(&mut state, index);
            assert_eq!(state.ai_notes.library.selected, Some(index));
            assert!(!state.ai_notes.library.changes);
            assert_eq!(state.ai_notes.library.open[0], !entry.editable());
        }
    }

    fn set_text(state: &mut State, id: &str, text: &str) -> usize {
        let ei = editor_index(library::find(id).unwrap()).unwrap();
        state.ai_notes.editors[ei].text = rune::TextEdit::new(text);
        ei
    }

    #[test]
    fn refresh_works_out_the_counts_once_and_matches_a_direct_count() {
        let mut state = State::new();
        let ei = set_text(&mut state, "ask-meeting", "Answer in one line.");
        refresh(&mut state, ei);
        let entry = library::find("ask-meeting").unwrap();
        let d = &state.ai_notes.library.derived[ei];
        assert!(d.is_for("Answer in one line."));
        assert_eq!(d.chars, "Answer in one line.".chars().count());
        assert_eq!(d.tokens, library::tokens("Answer in one line."));
        assert_eq!(
            d.whole,
            library::tokens(&entry.assembled("Answer in one line."))
        );
        assert!(d.valid());
        // Text with a character that cannot be saved reads as not valid, from the cache alone.
        set_text(&mut state, "ask-meeting", "bad\0text");
        refresh(&mut state, ei);
        assert!(!state.ai_notes.library.derived[ei].valid());
    }

    #[test]
    fn typing_waits_for_a_pause_but_the_first_look_and_headless_do_not() {
        assert!(due(true, false, Duration::ZERO));
        assert!(due(false, true, Duration::ZERO));
        assert!(!due(false, false, Duration::from_millis(10)));
        assert!(due(false, false, DEBOUNCE));
    }

    #[test]
    fn a_comparison_is_built_once_for_its_text_and_capped() {
        let left = "keep\n".repeat(3);
        let right: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let diff = build_diff(&left, &right);
        assert_eq!(diff.source, right);
        assert!(diff.chunks.len() <= MAX_ROWS);
        assert!(diff.hidden > 0);
        assert!(diff.added >= 1000);
        // A small edit is not capped.
        let small = build_diff("a\nb\n", "a\nc\n");
        assert_eq!((small.added, small.removed, small.hidden), (1, 1, 0));
    }

    #[test]
    fn showing_changes_builds_the_comparison_when_asked_and_tick_state_stays_valid() {
        let mut state = State::new();
        let index = library::catalog()
            .iter()
            .position(|e| e.id == "meeting-title")
            .unwrap();
        select(&mut state, index);
        let ei = set_text(&mut state, "meeting-title", "Name it in four words.");
        state.ai_notes.library.changes = true;
        refresh(&mut state, ei);
        let d = &state.ai_notes.library.derived[ei];
        let diff = d.diff.as_ref().expect("comparison built by refresh");
        assert_eq!(diff.source, "Name it in four words.");
        assert!(diff.added >= 1);
    }

    #[test]
    fn redo_tasks_copy_has_the_saved_tasks_instructions_between_its_fixed_blocks() {
        let mut state = State::new();
        let tasks = library::find("tasks-sort").unwrap();
        let at = editor_index(tasks).unwrap();
        state.ai_notes.editors[at].saved = "Group by client, never by month.".into();
        let index = library::catalog()
            .iter()
            .position(|e| e.id == "tasks-redo")
            .unwrap();
        let copied = prompt_text(&state, index);
        let (head, mid, tail) = (
            copied
                .find(
                    library::normalize(sidenote::core::agent_instructions::core_head(Agent::Tasks))
                        .trim(),
                )
                .unwrap(),
            copied.find("Group by client, never by month.").unwrap(),
            copied
                .find(library::normalize(include_str!("../core/notes/tasks_redo_tail.txt")).trim())
                .unwrap(),
        );
        assert!(head < mid && mid < tail);
        // Unsaved edits are not what the model gets, so they are not copied.
        state.ai_notes.editors[at].text = rune::TextEdit::new("An unsaved draft");
        assert!(!prompt_text(&state, index).contains("An unsaved draft"));
        // Opening the prompt builds the blocks the page shows, with the same middle block.
        select(&mut state, index);
        let shared = &state.ai_notes.library.shared;
        assert_eq!(shared.len(), 3);
        assert_eq!(shared[1].text, "Group by client, never by month.");
    }
}
