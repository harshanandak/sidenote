//! Capture review, explicit approval and locally saved annotation notes.
mod filmstrip;
pub(super) mod notebooks;
mod workflow;
use super::{
    Event, State, hex, nav, roles,
    theme::{self, Palette},
    widgets::{link, pressable, scroll},
};
use rune::{Color, El, Image, ImageCache, Key, Span, TextEdit, col, el, row};
use rune_kit::{Menu, MenuItem, Placement, TextArea};
use sidenote::{
    core::{
        annotations::{
            self, Capture, Keyframe, Mark, Scan, Word,
            generation::{self, Epoch, Progress, Ticket},
            media::{self, ApprovedCapture, ReviewFrame},
            note,
        },
        file_reads,
    },
    hex_engine::HexEngine,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
pub(super) use workflow::agents_section;
pub(super) use workflow::new_button;
use workflow::*;

#[derive(Default)]
pub struct Annotations {
    pub(super) response_time: u64,
    pub open: bool,
    notebooks: notebooks::Ui,
    captures: Vec<Capture>,
    thumbnails: BTreeMap<PathBuf, Option<Image>>,
    errors: Vec<String>,
    selected: Option<PathBuf>,
    transcript: bool,
    filmstrip: bool,
    filmstrip_step: usize,
    menu_open: bool,
    loading: bool,
    images: Vec<Result<Image, String>>,
    shortcut: Option<String>,
    disabled: bool,
    can_record: bool,
    record_busy: bool,
    review: Option<Vec<ReviewFrame>>,
    dragging: Option<(u64, media::BoxNorm)>,
    preview: Option<ApprovedCapture>,
    preview_images: BTreeMap<u64, Image>,
    approved: Option<ApprovedCapture>,
    drafts: BTreeMap<u64, serde_json::Value>,
    note: Option<serde_json::Value>,
    stream: String,
    busy: bool,
    status: String,
    epoch: Epoch,
    ticket: Option<Ticket>,
    editing: BTreeMap<String, TextEdit>,
    note_images: BTreeMap<u64, Image>,
    note_open: bool,
    source_fingerprint: Option<String>,
    review_source: Option<String>,
    last_review: Option<Vec<ReviewFrame>>,
    options: Option<generation::Options>,
    model: TextEdit,
    model_open: bool,
    other_model: bool,
}
impl Annotations {
    pub(super) fn with_view(mut self, view: usize) -> Self {
        self.filmstrip = view == 1;
        self
    }

    pub(super) fn working(&self) -> bool {
        self.busy
    }
}
pub enum Msg {
    Thumbnails(BTreeMap<PathBuf, Option<Image>>),
    NotebookDelta {
        id: u64,
        text: String,
    },
    NotebookReady {
        id: u64,
        book_id: String,
        revision: u64,
        result: Result<sidenote::core::annotations::notebook::Document, String>,
    },
    Scanned {
        scan: Scan,
        shortcut: Option<String>,
        disabled: bool,
        can_record: bool,
    },
    Images {
        folder: PathBuf,
        images: Vec<Result<Image, String>>,
        fingerprint: Option<String>,
    },
    Recording(Result<(), String>),
    Progress {
        id: u64,
        progress: Progress,
    },
    Approved {
        id: u64,
        approved: ApprovedCapture,
    },
    Finished {
        id: u64,
        result: Result<(), String>,
    },
    Loaded {
        folder: PathBuf,
        approved: ApprovedCapture,
        note: serde_json::Value,
    },
}

fn images(capture: &Capture, wake: rune::Waker, wait: bool) -> Vec<Result<Image, String>> {
    let cache = ImageCache::new(wake);
    capture
        .data
        .keyframes
        .iter()
        .map(|frame| {
            let path = capture.frame_path(frame).map_err(|e| e.to_string())?;
            let bytes = file_reads::read(&path, annotations::IMAGE_MAX_BYTES)
                .map_err(|_| "Could not read this frame image.".to_owned())?;
            let image = cache.load(bytes);
            if wait {
                image
                    .wait()
                    .map_err(|_| "Could not decode this frame image.".to_owned())?;
            }
            Ok(image)
        })
        .collect()
}
fn thumbnails(
    root: &std::path::Path,
    captures: &[Capture],
    wake: rune::Waker,
    wait: bool,
) -> BTreeMap<PathBuf, Option<Image>> {
    let cache = ImageCache::new(wake);
    captures
        .iter()
        .filter_map(|c| {
            let (approved, _) = annotations::notebook::load_capture_note(root, c).ok()??;
            let image = approved.approval.frames.first().and_then(|f| {
                let bytes = file_reads::read(
                    &approved.folder.join(&f.marked),
                    annotations::IMAGE_MAX_BYTES,
                )
                .ok()?;
                if blake3::hash(&bytes).to_hex().as_str()
                    != approved.approval.file_hashes[&f.marked]
                {
                    return None;
                }
                let image = cache.load(bytes);
                if wait {
                    image.wait().ok()?;
                }
                Some(image)
            });
            Some((c.folder.clone(), image))
        })
        .collect()
}
fn thumbnail(s: &State, folder: &std::path::Path, key: String, p: Palette) -> El {
    let mut view = el()
        .w(72.0)
        .h(44.0)
        .radius(4.0)
        .fill(p.chip)
        .key(key)
        .role("img")
        .label("Approved frame thumbnail");
    if let Some(Some(image)) = s.annotations.thumbnails.get(folder) {
        view = view.pixels(image.pixels());
    } else {
        view = view.child(
            super::icons::draw(&super::icons::ANNOTATIONS, 18.0, p.faint).absolute(27.0, 13.0),
        );
    }
    view
}
impl State {
    fn refresh_annotation_thumbnails(&mut self) {
        if !self.config.features.annotations {
            return;
        }
        let captures = self.annotations.captures.clone();
        let root = self.store.root.clone();
        let sender = self.events_tx.clone();
        let wake = self.waker.clone().unwrap_or_else(|| Arc::new(|| {}));
        std::thread::spawn(move || {
            let result = thumbnails(&root, &captures, wake.clone(), false);
            let _ = sender.send(Event::Annotations(Msg::Thumbnails(result)));
            wake();
        });
    }
}
fn hex_hints(settings: &[sidenote::hex_engine::Setting]) -> (Option<String>, bool) {
    let shortcut = settings.iter().filter(|s| !s.secret).find_map(|s| {
        let key = s.key.to_lowercase();
        (key.contains("annotate") && (key.contains("shortcut") || key.contains("hotkey")))
            .then(|| {
                s.value
                    .as_str()
                    .filter(|v| !v.trim().is_empty())
                    .map(str::to_owned)
            })
            .flatten()
    });
    let disabled = settings
        .iter()
        .any(|s| s.key == "labs.annotate" && s.value == false);
    (shortcut, disabled)
}
impl State {
    pub fn open_annotations(&mut self) {
        if !self.config.features.annotations {
            return;
        }
        let timing = super::Operation::new(super::TimedAction::Annotations);
        self.go_home();
        self.annotations.response_time = timing.defer();
        self.annotations.open = true;
        self.annotations.notebooks.selected = None;
        self.annotations.notebooks.shared = None;
        self.annotations.notebooks.pending = None;
        self.load_notebooks();
        self.annotations.selected = None;
        self.annotations.images.clear();
        self.load_annotation_options();
        self.refresh_annotations();
        self.nav_done();
        if !self.annotations.loading {
            super::response_time::ready(std::mem::take(&mut self.annotations.response_time));
        }
    }
    pub fn refresh_annotations(&mut self) {
        if !self.config.features.annotations
            || self.annotations.loading
            || self.annotations.busy
            || self.annotations.review.is_some()
            || rune::tooling::headless_run().is_some()
        {
            return;
        }
        self.annotations.loading = true;
        let sender = self.events_tx.clone();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let roots = annotations::annotation_roots();
            let scan = if roots.is_empty() {
                Scan {
                    captures: Vec::new(),
                    errors: vec!["Could not find the Hex captures folder.".into()],
                }
            } else {
                annotations::scan_roots(&roots)
            };
            let engine = HexEngine::new();
            let settings = engine.settings_list().unwrap_or_default();
            let (shortcut, disabled) = hex_hints(&settings);
            let can_record = hex_program().is_ok();
            let _ = sender.send(Event::Annotations(Msg::Scanned {
                scan,
                shortcut,
                disabled,
                can_record,
            }));
            if let Some(wake) = wake {
                wake();
            }
        });
    }
    fn select_annotation(&mut self, index: usize) {
        let Some(capture) = self.annotations.captures.get(index).cloned() else {
            return;
        };
        self.cancel_annotation_note();
        self.annotations.review = None;
        self.annotations.approved = None;
        self.annotations.note = None;
        self.annotations.drafts.clear();
        self.annotations.note_open = false;
        self.annotations.note_images.clear();
        self.annotations.last_review = None;
        self.annotations.source_fingerprint = None;
        self.annotations.review_source = None;
        self.annotations.editing.clear();
        self.annotations.selected = Some(capture.folder.clone());
        self.annotations.transcript = false;
        self.annotations.filmstrip_step = 0;
        self.annotations.menu_open = false;
        self.annotations.images.clear();
        let sender = self.events_tx.clone();
        let wake = self.waker.clone().unwrap_or_else(|| Arc::new(|| {}));
        let root = self.store.root.clone();
        std::thread::spawn(move || {
            let fingerprint = media::source_fingerprint(&capture).ok();
            let images = images(&capture, wake.clone(), false);
            let fingerprint = fingerprint
                .filter(|f| media::source_fingerprint(&capture).ok().as_ref() == Some(f));
            let _ = sender.send(Event::Annotations(Msg::Images {
                folder: capture.folder.clone(),
                images,
                fingerprint,
            }));
            if let Ok(Some((approved, note))) = load_note(&root, &capture) {
                let _ = sender.send(Event::Annotations(Msg::Loaded {
                    folder: capture.folder,
                    approved,
                    note,
                }));
            }
            wake();
        });
    }
    pub fn apply_annotations(&mut self, msg: Msg) {
        match msg {
            Msg::NotebookDelta { id, text } if self.annotation_job_current(id) => {
                self.annotations.stream = text
            }
            Msg::NotebookReady {
                id,
                book_id,
                revision,
                result,
            } if self.annotation_job_current(id) => {
                self.apply_notebook_ready(book_id, revision, result)
            }
            Msg::NotebookDelta { .. } | Msg::NotebookReady { .. } => {}
            Msg::Thumbnails(images) => self.annotations.thumbnails = images,
            Msg::Scanned {
                scan,
                shortcut,
                disabled,
                can_record,
            } => {
                let timing = std::mem::take(&mut self.annotations.response_time);
                if !self.annotations.open {
                    super::response_time::superseded(timing);
                } else if !scan.errors.is_empty() {
                    super::response_time::failed(timing);
                } else {
                    super::response_time::ready(timing);
                }
                self.annotations.captures = scan.captures;
                self.annotations.errors = scan.errors;
                self.annotations.shortcut = shortcut;
                self.annotations.disabled = disabled;
                self.annotations.can_record = can_record;
                self.annotations.loading = false;
                self.refresh_annotation_thumbnails();
                if let Some(folder) = self.annotations.selected.clone()
                    && !self.annotations.captures.iter().any(|c| c.folder == folder)
                {
                    self.annotations.selected = None;
                    self.annotations.images.clear();
                }
            }
            Msg::Images {
                folder,
                images,
                fingerprint,
            } if self.annotations.selected.as_ref() == Some(&folder) => {
                self.annotations.images = images;
                self.annotations.source_fingerprint = fingerprint;
                if self.annotations.notebooks.pending.is_some()
                    && self.annotations.review.is_none()
                    && self.annotations.source_fingerprint.is_some()
                {
                    self.review_annotation();
                }
            }
            Msg::Images { .. } => {}
            Msg::Recording(result) => {
                self.annotations.record_busy = false;
                self.annotations.status = match result {
                    Ok(()) => "Recording in Hex. Stop it in Hex, then refresh Annotations.".into(),
                    Err(e) => e,
                };
            }
            Msg::Approved { id, approved } if self.annotation_job_current(id) => {
                self.annotations.status =
                    if generation::uses_frame_drafts(approved.approval.frames.len()) {
                        "Writing frame drafts...".into()
                    } else {
                        "Writing the annotation note...".into()
                    };
                self.annotations.approved = Some(approved);
                self.load_annotation_note_images();
            }
            Msg::Progress { id, progress } if self.annotation_job_current(id) => match progress {
                Progress::Frame { index, draft, .. } => {
                    self.annotations.drafts.insert(index, draft);
                    let ready = self.annotations.drafts.len();
                    let total = self
                        .annotations
                        .approved
                        .as_ref()
                        .map_or(ready, |a| a.approval.frames.len());
                    self.annotations.status = if ready < total {
                        format!("{ready} of {total} frame drafts ready.")
                    } else {
                        format!(
                            "{ready} of {total} frame drafts ready. Writing the annotation note..."
                        )
                    };
                }
                Progress::NoteDelta(text) => self.annotations.stream = text,
                Progress::Final { note, .. } => {
                    self.annotations.note = Some(note);
                    self.annotations.busy = false;
                    self.annotations.status = "Annotation note saved on this PC.".into();
                    if let Some(a) = &self.annotations.approved
                        && save_latest(&self.store.root, a).is_err()
                    {
                        self.annotations.status="The note is saved, but Sidenote could not save its capture link. Save your edits to try again.".into();
                    }
                    self.annotations.note_open = true;
                    self.refresh_annotation_thumbnails();
                    self.load_annotation_note_images();
                    if self.annotations.notebooks.pending.is_some() {
                        self.continue_notebook();
                    }
                }
            },
            Msg::Finished { id, result } if self.annotation_job_current(id) => {
                self.annotations.busy = false;
                if let Err(error) = result {
                    self.annotations.status = error;
                }
            }
            Msg::Loaded {
                folder,
                approved,
                note,
            } if self.annotations.selected.as_ref() == Some(&folder) && !self.annotations.busy => {
                self.annotations.approved = Some(approved);
                self.annotations.note = Some(note);
                self.load_annotation_note_images();
            }
            Msg::Approved { .. }
            | Msg::Progress { .. }
            | Msg::Finished { .. }
            | Msg::Loaded { .. } => {}
        }
    }
}

pub fn on_key(s: &mut State, e: &rune::KeyEvent, tx: &mut rune::Tx) -> bool {
    if e.key == Key::Escape {
        if s.annotations.review.take().is_some() {
            if let Some(id) = s.annotations.notebooks.pending.take() {
                s.open_notebook(&id);
            }
            return true;
        }
        if s.annotations.note_open {
            s.cancel_annotation_note();
            s.annotations.note_open = false;
            return true;
        }
        s.cancel_annotation_note();
        if s.annotations.selected.take().is_none() {
            s.go_home();
        }
        s.annotations.images.clear();
        return true;
    }
    filmstrip::on_key(s, e, tx)
}
fn date(c: &Capture) -> String {
    chrono::DateTime::from_timestamp_millis(c.data.created_at_ms)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d %b %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Date unavailable".into())
}
fn time(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
fn facts(c: &Capture, p: Palette) -> El {
    roles::meta_line(
        &[
            date(c),
            time(c.data.duration_ms as f64 / 1000.0),
            format!("{} words", c.word_count()),
            format!("{} marks", c.data.marks.len()),
        ],
        p,
    )
}
fn number(n: usize, p: Palette) -> El {
    col()
        .w(24.0)
        .h(24.0)
        .center()
        .radius(12.0)
        .fill(p.ink)
        .child(roles::CAPTION.line(n.to_string(), p.card))
}
fn words_text(words: &[&Word]) -> String {
    words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}
fn callout(c: &Capture, n: usize, mark: &Mark, p: Palette) -> El {
    let words = c.words_for_mark(mark);
    let description = mark
        .description
        .as_deref()
        .filter(|d| !d.is_empty())
        .unwrap_or("Mark without a description");
    let mut content = col().w_full().min_w(0.0).gap(7.0).child(
        row()
            .gap(8.0)
            .items_center()
            .child(number(n, p))
            .child(roles::NAME.wrap(description, p.ink).grow(1.0).min_w(0.0)),
    );
    if let (Some(first), Some(last)) = (words.first(), words.last()) {
        content = content
            .child(roles::META.wrap(
                format!("Words nearby, {} to {}", time(first.t0), time(last.t1)),
                p.faint,
            ))
            .child(
                roles::SMALL
                    .wrap(words_text(&words), p.ink)
                    .selectable(true),
            );
    } else {
        content = content.child(roles::SMALL.wrap("No timed words near this mark.", p.soft));
    }
    content
        .key(format!("annotation-callout-{n}"))
        .padding(12.0)
        .radius(8.0)
        .border(1.0, p.line)
        .fill(p.card)
}
fn step_excerpt<'a>(c: &Capture, mark: &'a Mark) -> Option<(&'a str, String)> {
    let heading = mark.description.as_deref()?.trim();
    if heading.is_empty() {
        return None;
    }
    let words = c.words_for_mark(mark);
    let ends_sentence = |word: &Word| {
        word.text
            .trim()
            .trim_end_matches(['"', '\'', ')', '”', '’'])
            .ends_with(['.', '?', '!', '。', '？', '！'])
    };
    let timed = &c.data.transcript.words;
    let first = *words.first()?;
    let start = timed.iter().position(|word| std::ptr::eq(word, first))?;
    let before = &timed[..start];
    let sentence_start = before.iter().rposition(ends_sentence).map_or(0, |i| i + 1);
    let excerpt = before[sentence_start..]
        .iter()
        .chain(words)
        .scan(0, |sentences, word| {
            if *sentences == 2 {
                return None;
            }
            *sentences += usize::from(ends_sentence(word));
            Some(word.text.trim())
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!excerpt.is_empty()).then_some((heading, excerpt))
}
fn mark_badge(n: usize) -> El {
    col()
        .size(22.0, 22.0)
        .center()
        .radius(11.0)
        .fill(Color::BLACK)
        .border(1.5, Color::WHITE)
        .child(roles::CAPTION.line(n.to_string(), Color::WHITE))
        .label(format!("Mark {n}"))
}

fn frame_image(
    s: &State,
    c: &Capture,
    index: usize,
    frame: &Keyframe,
    width: f32,
    p: Palette,
) -> El {
    let image = s.annotations.images.get(index);
    let image = match image {
        Some(Ok(image)) if image.error().is_none() => image,
        Some(Err(error)) => {
            return roles::SMALL
                .wrap(error, p.warn)
                .padding(20.0)
                .w(width)
                .role("status");
        }
        Some(Ok(_)) => {
            return roles::SMALL
                .wrap("Could not decode this frame image.", p.warn)
                .padding(20.0)
                .w(width)
                .role("status");
        }
        None => {
            return roles::SMALL
                .wrap("Loading frame image...", p.soft)
                .padding(20.0)
                .w(width)
                .role("status");
        }
    };
    let Some([w, h]) = image.size() else {
        return roles::SMALL
            .wrap("Loading frame image...", p.soft)
            .padding(20.0)
            .w(width);
    };
    let height = width * h as f32 / w as f32;
    let mut canvas = el()
        .key(format!("annotation-image-{}", frame.index))
        .w(width)
        .h(height)
        .pixels(image.pixels())
        .clip_rounded(8.0)
        .role("img")
        .label(format!(
            "{}: {}",
            c.app(),
            frame.reason.as_deref().unwrap_or("Capture frame")
        ));
    for (i, mark) in c.data.marks.iter().enumerate() {
        if !c
            .frame_for_mark(mark)
            .is_some_and(|f| f.index == frame.index)
        {
            continue;
        }
        if let Some([x, y, w, h]) = mark.box_norm.filter(|b| b.iter().all(|n| n.is_finite())) {
            let x = x.clamp(0.0, 1.0);
            let y = y.clamp(0.0, 1.0);
            let bw = w.clamp(0.0, 1.0 - x) * width;
            let bh = h.clamp(0.0, 1.0 - y) * height;
            canvas = canvas.child(
                el().key(format!("annotation-box-{}", i + 1))
                    .absolute(x * width, y * height)
                    .w(bw)
                    .h(bh)
                    .border(1.5, Color::WHITE)
                    .radius(4.0)
                    .child(
                        el().absolute(1.5, 1.5)
                            .w((bw - 3.0).max(0.0))
                            .h((bh - 3.0).max(0.0))
                            .border(1.5, Color::BLACK)
                            .radius(2.5),
                    ),
            );
            canvas = canvas.child(
                mark_badge(i + 1)
                    .key(format!("annotation-badge-{}", i + 1))
                    .absolute(
                        (x * width - 11.0).clamp(0.0, (width - 22.0).max(0.0)),
                        (y * height - 11.0).clamp(0.0, (height - 22.0).max(0.0)),
                    ),
            );
        }
    }
    canvas
}
fn capture_menu(s: &State, c: &Capture, p: Palette, layer: &mut Vec<El>) -> El {
    let trigger = super::widgets::icon_button(
        &super::icons::MORE,
        "Capture actions",
        "annotation-actions-button",
        super::widgets::ButtonRole::Ghost,
        p,
    )
    .size(28.0, 28.0)
    .px(6.0)
    .on_press(|s: &mut State| s.annotations.menu_open = !s.annotations.menu_open);
    let folder = c.folder.clone();
    let mut menu = Menu::new("annotation-actions")
        .trigger(trigger)
        .open(s.annotations.menu_open)
        .placement(Placement::BottomEnd)
        .on_close(|s: &mut State| s.annotations.menu_open = false)
        .item(MenuItem::new("Write note").on_select(|s: &mut State| {
            s.annotations.menu_open = false;
            s.review_annotation();
        }));
    if s.annotations.note.is_some() {
        menu = menu.item(
            MenuItem::new("Open annotation note").on_select(|s: &mut State| {
                s.annotations.menu_open = false;
                s.annotations.note_open = true;
            }),
        );
    }
    menu.item(MenuItem::new("Add to notebook").on_select(|s: &mut State| {
        s.annotations.menu_open = false;
        s.choose_notebook_for_capture();
    }))
    .item(MenuItem::new("Files").on_select(move |s: &mut State| {
        s.annotations.menu_open = false;
        if rune::tooling::headless_run().is_none()
            && sidenote::platform::open_folder(&folder).is_err()
        {
            s.annotations
                .errors
                .push("Could not open the capture folder.".into());
        }
    }))
    .item(
        MenuItem::new(if s.annotations.transcript {
            "Back to steps"
        } else {
            "Transcript"
        })
        .on_select_tx(|s: &mut State, tx| {
            s.annotations.menu_open = false;
            s.annotations.transcript = !s.annotations.transcript;
            tx.scroll_to_y("annotations-pane", 0.0);
        }),
    )
    .build(&theme::kit(&p), layer)
}

const TRANSCRIPT_CHUNK: usize = 28;

fn capture_page(s: &State, c: &Capture, width: f32, p: Palette, layer: &mut Vec<El>) -> El {
    let mut page = col().w_full().gap(14.0).child(
        crate::ui::widgets::action("Back to annotations", p)
            .key("annotations-back")
            .self_start()
            .on_press(|s: &mut State| {
                s.cancel_annotation_note();
                s.annotations.selected = None;
                s.annotations.menu_open = false;
                s.annotations.images.clear();
                s.annotations.review = None;
                s.annotations.note_open = false;
            }),
    );
    if s.annotations.busy || s.annotations.note_open {
        return note_page(s, c, width, p, page);
    }
    if s.annotations.review.is_some() {
        return review_page(
            s,
            c,
            width,
            p,
            page.child(roles::GROUP.wrap(c.app(), p.soft))
                .child(roles::SECTION.wrap(c.title(), p.ink).max_lines(2))
                .child(facts(c, p)),
        );
    }
    let steps = c.data.keyframes.len();
    page = page.child(
        col()
            .w_full()
            .gap(4.0)
            .child(
                roles::SECTION
                    .wrap(c.title(), p.ink)
                    .font(theme::display(26.0))
                    .max_lines(2),
            )
            .child(
                row()
                    .w_full()
                    .items_center()
                    .gap(8.0)
                    .child(
                        roles::meta_line(
                            &[
                                c.app().to_owned(),
                                date(c),
                                time(c.data.duration_ms as f64 / 1000.0),
                                format!("{steps} {}", if steps == 1 { "step" } else { "steps" }),
                            ],
                            p,
                        )
                        .grow(1.0)
                        .min_w(0.0),
                    )
                    .child(filmstrip::view_switch(s, p))
                    .child(capture_menu(s, c, p, layer)),
            ),
    );
    if s.annotations.transcript {
        let transcript = &c.data.transcript;
        let mut text = col()
            .w_full()
            .gap(16.0)
            .key("annotation-transcript")
            .role("region")
            .label("Capture transcript");
        if !transcript.words.is_empty() {
            for (i, chunk) in transcript.words.chunks(TRANSCRIPT_CHUNK).enumerate() {
                text = text.child(
                    col()
                        .w_full()
                        .gap(4.0)
                        .key(format!("annotation-transcript-part-{i}"))
                        .child(roles::META.wrap(time(chunk[0].t0), p.faint))
                        .child(
                            roles::READ
                                .wrap(
                                    chunk
                                        .iter()
                                        .map(|w| w.text.as_str())
                                        .collect::<Vec<_>>()
                                        .join(" "),
                                    p.ink,
                                )
                                .key(format!("annotation-transcript-words-{i}"))
                                .selectable(true),
                        ),
                );
            }
        } else if !transcript.text.trim().is_empty() {
            text = text.child(roles::READ.wrap(&transcript.text, p.ink).selectable(true));
        } else if !transcript.segments.is_empty() {
            for segment in &transcript.segments {
                text = text.child(roles::READ.wrap(&segment.text, p.ink).selectable(true));
            }
        } else {
            text = text.child(roles::SMALL.wrap("This capture has no transcript yet.", p.soft));
        }
        return page.child(hex::settings::panel(p).child(text.padding(18.0)));
    }
    if s.annotations.filmstrip && steps > 0 {
        page = page.child(filmstrip::view(s, c, width, p));
    } else {
        page = blocks(s, c, width, p, page);
    }
    if steps == 0 {
        page = page.child(roles::SMALL.wrap("This capture has no keyframes yet.", p.soft));
    }
    let missing: Vec<_> = c
        .data
        .marks
        .iter()
        .enumerate()
        .filter(|(_, m)| c.frame_for_mark(m).is_none())
        .collect();
    if !missing.is_empty() {
        page = page.child(roles::GROUP.wrap("Marks without a matching frame", p.soft));
        for (i, mark) in missing {
            page = page.child(callout(c, i + 1, mark, p));
        }
    }
    page
}
fn blocks(s: &State, c: &Capture, width: f32, p: Palette, mut page: El) -> El {
    let side_by_side = width >= 640.0;
    let image_width = width.min(360.0);
    for (index, frame) in c.data.keyframes.iter().enumerate() {
        let details = step_details(c, index, frame, p, false);
        let visual = frame_image(s, c, index, frame, image_width, p);
        let body = if side_by_side {
            row().items_start().gap(24.0).child(visual).child(details)
        } else {
            col().gap(12.0).child(visual).child(details)
        };
        page = page.child(
            col()
                .w_full()
                .gap(16.0)
                .key(format!("annotation-frame-{}", frame.index))
                .child(el().w_full().h(1.0).fill(p.line))
                .child(body.w_full()),
        );
    }
    page
}

fn step_details(c: &Capture, index: usize, frame: &Keyframe, p: Palette, filmstrip: bool) -> El {
    let step = index + 1;
    let step_label = if filmstrip {
        format!(
            "Step {step} of {}{}",
            c.data.keyframes.len(),
            roles::DOT.trim_end()
        )
    } else {
        format!("Step {step}{}", roles::DOT.trim_end())
    };
    let timestamp = time(frame.t);
    let transcript_part = c
        .data
        .transcript
        .words
        .iter()
        .position(|word| word.t1 >= frame.t)
        .unwrap_or_else(|| c.data.transcript.words.len().saturating_sub(1))
        / TRANSCRIPT_CHUNK;
    let mut details = col().grow(1.0).min_w(0.0).gap(8.0).child(
        row()
            .items_center()
            .gap(4.0)
            .child(roles::META.line(step_label, p.faint))
            .child(
                pressable(
                    roles::TIME
                        .line(&timestamp, p.soft)
                        .decorations(vec![Span::new(0, timestamp.len()).underline(p.soft)]),
                )
                .key(format!("annotation-step-time-{step}"))
                .role("link")
                .label(format!("Transcript at {timestamp}"))
                .on_press_tx(move |s: &mut State, tx| {
                    s.annotations.transcript = true;
                    tx.scroll_to_y("annotations-pane", 0.0);
                    tx.scroll_key_to(
                        format!("annotation-transcript-part-{transcript_part}"),
                        12.0,
                    );
                }),
            ),
    );
    for (i, mark) in c.data.marks.iter().enumerate() {
        if c.frame_for_mark(mark)
            .is_some_and(|f| f.index == frame.index)
            && let Some((heading, excerpt)) = step_excerpt(c, mark).or_else(|| {
                let heading = mark.description.as_deref()?.trim();
                (filmstrip && !heading.is_empty()).then(|| (heading, String::new()))
            })
        {
            details = details.child(
                col()
                    .w_full()
                    .gap(4.0)
                    .key(format!("annotation-callout-{}", i + 1))
                    .child(
                        row().items_start().gap(8.0).child(mark_badge(i + 1)).child(
                            roles::TITLE
                                .wrap(heading, p.ink)
                                .grow(1.0)
                                .min_w(0.0)
                                .role("heading"),
                        ),
                    )
                    .children(
                        (!excerpt.is_empty())
                            .then(|| roles::READ.wrap(excerpt, p.body).selectable(true)),
                    ),
            );
        }
    }
    details
}

pub fn pane(s: &State, p: Palette, narrow: bool) -> El {
    let width = (s.app_w.get() - nav::width(s) - 1.0 - if narrow { 32.0 } else { 64.0 })
        .clamp(240.0, 1040.0);
    let mut layer = Vec::new();
    let mut content = if s.annotations.notebooks.shared.is_some() {
        notebooks::confirmation(s, p)
    } else if s.annotations.notebooks.picker.is_some() {
        notebooks::picker(s, p)
    } else if s.annotations.notebooks.selected.is_some() {
        notebooks::page(s, width, p)
    } else if let Some(c) = s
        .annotations
        .captures
        .iter()
        .find(|c| Some(&c.folder) == s.annotations.selected.as_ref())
    {
        capture_page(s, c, width, p, &mut layer)
    } else {
        let mut list = col()
            .w_full()
            .gap(10.0)
            .child(
                row()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        roles::SECTION
                            .wrap("Annotations", p.ink)
                            .font(theme::display(28.0)),
                    )
                    .child(
                        crate::ui::widgets::action(
                            if s.annotations.loading {
                                "Refreshing..."
                            } else {
                                "Refresh"
                            },
                            p,
                        )
                        .key("annotations-refresh")
                        .on_press(|s: &mut State| s.refresh_annotations()),
                    ),
            )
            .child(roles::SMALL.wrap("Screen captures and the words around each mark.", p.soft))
            .child(roles::META.wrap("Local on this PC", p.faint))
            .child(notebooks::list_section(s, p));
        if s.annotations.captures.is_empty() {
            let title = if s.annotations.loading {
                "Looking for captures..."
            } else if s.annotations.disabled {
                "Hex Annotate is turned off"
            } else {
                "No annotations yet"
            };
            let hint = if let Some(shortcut) = &s.annotations.shortcut {
                format!("In Hex, turn on Annotate and press {shortcut} to draw and talk.")
            } else {
                "In Hex, turn on Annotate, then record your screen while you draw and talk.".into()
            };
            list = list.child(hex::settings::card(
                title,
                col()
                    .gap(12.0)
                    .padding(22.0)
                    .child(super::icons::draw(&super::icons::ANNOTATIONS, 28.0, p.soft))
                    .child(roles::SMALL.wrap(hint, p.soft)),
                p,
            ));
        } else {
            let mut rows = col().w_full();
            for (index, c) in s.annotations.captures.iter().enumerate() {
                rows = rows.child(
                    pressable(
                        row()
                            .w_full()
                            .gap(12.0)
                            .px(16.0)
                            .py(12.0)
                            .items_center()
                            .child(thumbnail(
                                s,
                                &c.folder,
                                format!("annotation-thumbnail-{index}"),
                                p,
                            ))
                            .child(
                                col()
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .gap(4.0)
                                    .child(roles::META.wrap(c.app(), p.soft))
                                    .child(roles::TITLE.wrap(c.title(), p.ink).max_lines(2))
                                    .child(facts(c, p)),
                            ),
                    )
                    .key(format!("annotation-row-{index}"))
                    .label(format!("{}: {}", c.app(), c.title()))
                    .radius(9.0)
                    .hover(p.chip)
                    .on_press(move |s: &mut State| s.select_annotation(index)),
                );
            }
            list = list.child(hex::settings::card("Captures, newest first", rows, p));
        }
        list
    };
    for error in &s.annotations.errors {
        content = content.child(roles::SMALL.wrap(error, p.warn).role("status"));
    }
    if !s.annotations.note_open
        && s.annotations.review.is_none()
        && !s.annotations.status.is_empty()
    {
        content = content.child(
            roles::SMALL
                .wrap(&s.annotations.status, p.soft)
                .role("status"),
        );
    }
    let viewport = scroll(
        col()
            .w_full()
            .items_center()
            .py(20.0)
            .child(content.w(width).pb(40.0)),
        "annotations-scroll",
    )
    .key("annotations-pane");
    rune_kit::root(viewport, layer)
        .key("annotation-menu-root")
        .grow(1.0)
        .min_h(0.0)
}

pub fn scene(s: &mut State, name: &str) {
    hex::scene(s, false);
    s.go_home();
    s.annotations.open = true;
    s.annotations.loading = false;
    s.annotations.errors.clear();
    s.annotations.can_record = true;
    s.ai_notes.settings = Some(sidenote::core::notes::NotesSettings {
        helper: Some(sidenote::core::notes::Helper::Claude),
        model: "claude-sonnet-5-5".into(),
        ..Default::default()
    });
    s.annotations.captures = if name == "annotations-empty" {
        Vec::new()
    } else {
        annotations::scan(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/annotations"),
        )
        .captures
    };
    super::scene_sidebar(s, true);
    if (name.starts_with("annotation-capture")
        || name == "annotation-transcript"
        || matches!(
            name,
            "annotation-review" | "annotation-drafting" | "annotation-note"
        ))
        && let Some(c) = s.annotations.captures.first().cloned()
    {
        s.annotations.selected = Some(c.folder.clone());
        s.annotations.images = images(&c, Arc::new(|| {}), true);
        s.annotations.source_fingerprint = media::source_fingerprint(&c).ok();
        s.annotations.transcript = name == "annotation-transcript";
        if name == "annotation-review" {
            s.review_annotation();
            if let Some(frames) = &mut s.annotations.review {
                frames[0].masks.push(media::SYNTHETIC_MASK);
            }
        }
        if name == "annotation-review" {
            s.prepare_annotation_preview();
        }
        if matches!(name, "annotation-drafting" | "annotation-note") {
            let review: Vec<_> = c
                .data
                .keyframes
                .iter()
                .enumerate()
                .map(|(i, f)| ReviewFrame {
                    index: f.index,
                    included: true,
                    masks: if i == 0 {
                        vec![media::SYNTHETIC_MASK]
                    } else {
                        Vec::new()
                    },
                })
                .collect();
            let approved =
                media::approve(&c, &review, &s.store.root).expect("synthetic annotation approval");
            let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tools/ui-fixtures/annotation-note.json");
            let value = std::fs::read(fixture)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .filter(|n| note::validate(n, &approved).is_ok())
                .unwrap_or_else(|| note::synthetic_note(&approved));
            s.annotations.approved = Some(approved);
            s.annotations.note_open = true;
            s.load_annotation_note_images();
            if name == "annotation-note" {
                s.annotations.note = Some(value);
                s.annotations.status = "Annotation note saved on this PC.".into();
            } else {
                s.annotations.busy = true;
                s.annotations.ticket = Some(s.annotations.epoch.begin());
                s.annotations.status = "Writing the annotation note...".into();
                let summary = value["summary"].as_str().unwrap_or("");
                let preview: String = summary.chars().take(135).collect();
                s.annotations.stream = format!(
                    "{{\"title\":{},\"summary\":{}",
                    value["title"],
                    serde_json::to_string(&preview)
                        .unwrap()
                        .trim_end_matches('"')
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_text_needs_a_heading_and_words_and_keeps_at_most_two_sentences() {
        let mut capture = Capture {
            folder: PathBuf::new(),
            data: serde_json::from_value(serde_json::json!({
                "transcript": {"words": [
                    {"t0": 0.0, "t1": 0.5, "text": "First."},
                    {"t0": 0.5, "t1": 1.0, "text": "Second!"},
                    {"t0": 1.0, "t1": 1.5, "text": "Third?"}
                ]}
            }))
            .unwrap(),
            legacy_folder: None,
        };
        let mut mark = Mark {
            description: Some("  Sample mark  ".into()),
            t1: 1.0,
            ..Default::default()
        };
        assert_eq!(
            step_excerpt(&capture, &mark),
            Some(("Sample mark", "First. Second!".into()))
        );
        mark.description = Some("  ".into());
        assert_eq!(step_excerpt(&capture, &mark), None);
        mark.description = Some("Sample mark".into());
        capture.data.transcript.words.clear();
        assert_eq!(step_excerpt(&capture, &mark), None);
    }

    #[test]
    fn step_excerpt_recovers_the_start_of_a_clipped_sentence() {
        let capture = Capture {
            folder: PathBuf::new(),
            data: serde_json::from_value(serde_json::json!({
                "transcript": {"words": [
                    {"t0": 0.0, "t1": 1.0, "text": "Earlier."},
                    {"t0": 5.0, "t1": 6.0, "text": "Bring"},
                    {"t0": 7.0, "t1": 8.0, "text": "cards.”"},
                    {"t0": 9.0, "t1": 10.0, "text": "Thanks!"}
                ]}
            }))
            .unwrap(),
            legacy_folder: None,
        };
        let mark = Mark {
            description: Some("Materials".into()),
            t0: 10.0,
            t1: 10.0,
            ..Default::default()
        };
        assert_eq!(
            step_excerpt(&capture, &mark),
            Some(("Materials", "Bring cards.” Thanks!".into()))
        );
    }

    #[test]
    fn hints_use_only_reported_non_secret_annotation_settings() {
        let settings =
            serde_json::from_value::<Vec<sidenote::hex_engine::Setting>>(serde_json::json!([
                {"key":"labs.annotate","value":false,"kind":"boolean","writable":true},
                {"key":"shortcuts.annotate","value":"Ctrl+Alt+A","kind":"string","writable":true},
                {"key":"dictation.shortcut","value":"Ctrl+D","kind":"string","writable":true}
            ]))
            .unwrap();
        assert_eq!(hex_hints(&settings), (Some("Ctrl+Alt+A".into()), true));
        assert_eq!(hex_hints(&[]), (None, false));
        let mut secret = settings[1].clone();
        secret.secret = true;
        assert_eq!(hex_hints(&[secret]), (None, false));
    }
}
