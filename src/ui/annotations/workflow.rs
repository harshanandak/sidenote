use super::*;
use crate::ui::widgets::ButtonDisabled;
use serde_json::{Value, json};
use sidenote::platform::CommandExt;
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[cfg(windows)]
pub(super) fn hex_program() -> Result<PathBuf, String> {
    let status = HexEngine::new()
        .status()
        .map_err(|_| "Hex is not answering. Open Hex to record an annotation.")?;
    if status.version.is_empty()
        || !status
            .version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
    {
        return Err("Hex did not report a supported version.".into());
    }
    let exe = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
        .join("hex/versions")
        .join(status.version)
        .join("hex.exe");
    exe.is_file()
        .then_some(exe)
        .ok_or_else(|| "The Hex Annotate command is unavailable. Update Hex and try again.".into())
}
#[cfg(not(windows))]
pub(super) fn hex_program() -> Result<PathBuf, String> {
    Err("Hex annotation recording is unavailable on Linux.".into())
}
pub(in crate::ui) fn new_button(s: &State, p: Palette) -> El {
    let enabled = s.annotations.can_record
        && s.hex.status.as_ref().is_some_and(Result::is_ok)
        && !s.annotations.record_busy;
    let b = crate::ui::widgets::button(
        "New annotation",
        "new-annotation",
        crate::ui::widgets::ButtonRole::Primary,
        p,
    );
    if enabled {
        b.help("Record your screen while you draw and talk in Hex")
            .on_press(|s: &mut State| s.start_annotation())
    } else {
        b.dim(0.5).help("Hex is not answering or Annotate is unavailable. Open Hex and refresh Annotations.").disabled(true)
    }
}
fn run_hex_start() -> Result<(), String> {
    let mut child = Command::new(hex_program()?)
        .args(["annotate", "start"])
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start Hex Annotate.")?;
    let job = match sidenote::platform::ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Could not contain the Hex command.".into());
        }
    };
    let start = Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                break if s.success() {
                    Ok(())
                } else {
                    Err("Hex could not start the annotation. Check Hex and try again.".into())
                };
            }
            Err(_) => break Err("Could not check the Hex command.".into()),
            Ok(None) => {}
        }
        if start.elapsed() > Duration::from_secs(12) {
            break Err("Hex did not answer in time. Open Hex and try again.".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    drop(job);
    let _ = child.kill();
    let _ = child.wait();
    result
}
pub(super) fn save_latest(root: &Path, a: &ApprovedCapture) -> std::io::Result<()> {
    media::publish_latest(root, a)
}
pub(super) fn load_note(
    root: &Path,
    c: &Capture,
) -> std::io::Result<Option<(ApprovedCapture, Value)>> {
    let folder = media::note_folder(root, c);
    let bytes = match file_reads::read(&folder.join("latest.json"), 4096) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let latest: Value = serde_json::from_slice(&bytes)?;
    let leaf = latest["approved_folder"]
        .as_str()
        .ok_or_else(|| std::io::Error::other("Invalid saved annotation path"))?;
    if !leaf.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err(std::io::Error::other("Invalid saved annotation path"));
    }
    let folder = folder.join("approved").join(leaf);
    let approval = serde_json::from_slice(&file_reads::read(
        &folder.join("approval.json"),
        16 * 1024 * 1024,
    )?)?;
    let approved = ApprovedCapture { folder, approval };
    approved.verify()?;
    if media::source_fingerprint(c)? != approved.approval.source_fingerprint {
        return Ok(None);
    }
    let Some(note) = note::load(&approved.folder)? else {
        return Ok(None);
    };
    note::validate(&note, &approved).map_err(std::io::Error::other)?;
    Ok(Some((approved, note)))
}
impl State {
    pub fn cancel_annotation_note(&mut self) {
        if let Some(t) = self.annotations.ticket.take() {
            t.cancel.stop();
        }
        self.annotations.epoch.invalidate();
        if self.annotations.busy {
            self.annotations.status = "Stopped. Drafts stay on this PC. Retry when ready.".into();
        }
        self.annotations.busy = false;
    }
    pub(super) fn annotation_job_current(&self, id: u64) -> bool {
        self.annotations
            .ticket
            .as_ref()
            .is_some_and(|t| t.id == id && t.current())
    }
    pub(super) fn annotation_options(&self) -> Result<generation::Options, String> {
        if !self.config.features.annotations {
            return Err("Annotations and pictures is off.".into());
        }
        if let Some(options) = &self.annotations.options {
            generation::available(options)?;
            return Ok(options.clone());
        }
        let s = self
            .ai_notes
            .settings
            .as_ref()
            .ok_or("Choose an agent and model in Models and agents.")?;
        let helper = s
            .helper
            .ok_or("Choose Claude Code or Codex in Models and agents.")?;
        let options = generation::Options {
            helper,
            model: if s.model.trim().is_empty() {
                helper.default_model().into()
            } else {
                s.model.clone()
            },
        };
        generation::available(&options)?;
        Ok(options)
    }
    pub fn load_annotation_options(&mut self) {
        match generation::load_options(&self.store.root){Ok(options)=>{self.annotations.options=options;self.annotations.model=TextEdit::new(self.annotations.options.as_ref().map(|o|o.model.as_str()).unwrap_or("claude-sonnet-5-5"));},Err(_)=>self.annotations.status="Could not read annotation agent settings. Choose the agent again in Models and agents.".into()}
    }
    fn choose_annotation_agent(&mut self, helper: sidenote::core::notes::Helper) {
        let model = if self
            .annotations
            .options
            .as_ref()
            .is_some_and(|o| o.helper == helper)
        {
            self.annotations.model.text.trim().to_owned()
        } else {
            helper.default_model().to_owned()
        };
        self.annotations.other_model = false;
        self.annotations.model_open = false;
        let options = generation::Options { helper, model };
        match generation::save_options(&self.store.root, &options) {
            Ok(()) => {
                self.annotations.model = TextEdit::new(&options.model);
                self.annotations.options = Some(options);
                self.annotations.status = "Annotation agent saved.".into();
            }
            Err(_) => {
                self.annotations.status = "Could not save the annotation agent. Try again.".into()
            }
        }
    }
    fn save_annotation_model(&mut self) {
        let Some(current) = self.annotations.options.as_ref() else {
            self.annotations.status = "Choose an annotation agent before saving a model.".into();
            return;
        };
        let options = generation::Options {
            helper: current.helper,
            model: self.annotations.model.text.trim().to_owned(),
        };
        match generation::save_options(&self.store.root, &options) {
            Ok(()) => {
                self.annotations.options = Some(options);
                self.annotations.other_model = false;
                self.annotations.status = "Annotation model saved.".into();
            }
            Err(e) => self.annotations.status = e.to_string(),
        }
    }
    fn start_annotation(&mut self) {
        if !self.config.features.annotations
            || rune::tooling::headless_run().is_some()
            || self.annotations.record_busy
        {
            return;
        }
        self.annotations.record_busy = true;
        let sender = self.events_tx.clone();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let _ = sender.send(Event::Annotations(Msg::Recording(run_hex_start())));
            if let Some(w) = wake {
                w();
            }
        });
    }
    pub(super) fn review_annotation(&mut self) {
        self.cancel_annotation_note();
        let Some(c) = self
            .annotations
            .captures
            .iter()
            .find(|c| Some(&c.folder) == self.annotations.selected.as_ref())
        else {
            return;
        };
        if self.annotations.source_fingerprint.is_none() {
            self.annotations.errors.push(
                "Frame images are still loading or changed. Refresh this capture before review."
                    .into(),
            );
            return;
        }
        self.annotations.review_source = self.annotations.source_fingerprint.clone();
        self.annotations.review = Some(
            c.data
                .keyframes
                .iter()
                .map(|f| ReviewFrame {
                    index: f.index,
                    included: true,
                    masks: Vec::new(),
                })
                .collect(),
        );
        self.annotations.status.clear();
        self.annotations.editing.clear();
        self.prepare_annotation_preview();
    }
    pub(super) fn prepare_annotation_preview(&mut self) {
        self.annotations.preview = None;
        self.annotations.preview_images.clear();
        let Some(review) = &self.annotations.review else {
            return;
        };
        if !review.iter().any(|f| f.included) {
            return;
        }
        let Some(c) = self
            .annotations
            .captures
            .iter()
            .find(|c| Some(&c.folder) == self.annotations.selected.as_ref())
        else {
            return;
        };
        let result = (|| {
            let preview = media::prepare(c, review, &self.store.root)?;
            let cache = ImageCache::new(self.waker.clone().unwrap_or_else(|| Arc::new(|| {})));
            for frame in &preview.approval.frames {
                let bytes = file_reads::read(
                    &preview.folder.join(&frame.marked),
                    annotations::IMAGE_MAX_BYTES,
                )?;
                if blake3::hash(&bytes).to_hex().as_str()
                    != preview.approval.file_hashes[&frame.marked]
                {
                    return Err(std::io::Error::other("Review image changed."));
                }
                let image = cache.load(bytes);
                image
                    .wait()
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                self.annotations.preview_images.insert(frame.index, image);
            }
            preview.verify_preview()?;
            Ok::<_, std::io::Error>(preview)
        })();
        match result {
            Ok(preview) => self.annotations.preview = Some(preview),
            Err(e) => self.annotations.status = e.to_string(),
        }
    }
    fn send_annotation(&mut self, retry: bool) {
        if rune::tooling::headless_run().is_some() {
            return;
        }
        let options = match self.annotation_options() {
            Ok(o) => o,
            Err(e) => {
                self.annotations.status = e;
                return;
            }
        };
        let Some(capture) = self
            .annotations
            .captures
            .iter()
            .find(|c| Some(&c.folder) == self.annotations.selected.as_ref())
            .cloned()
        else {
            return;
        };
        let review = self
            .annotations
            .review
            .clone()
            .or_else(|| self.annotations.last_review.clone());
        let previous = if retry {
            self.annotations.approved.clone()
        } else {
            None
        };
        let preview = self.annotations.preview.clone();
        if previous.is_none()
            && preview
                .as_ref()
                .is_none_or(|p| Some(&p.approval.review) != review.as_ref())
        {
            self.annotations.status = "Wait for the baked review images, then send again.".into();
            return;
        }
        if previous.is_none()
            && review
                .as_ref()
                .is_none_or(|frames| !frames.iter().any(|f| f.included))
        {
            self.annotations.status = "Choose at least one frame.".into();
            return;
        }
        let expected = self.annotations.review_source.clone();
        self.annotations.last_review = review.clone();
        let preparing = previous.is_none();
        self.cancel_annotation_note();
        let ticket = self.annotations.epoch.begin();
        let id = ticket.id;
        self.annotations.ticket = Some(ticket.clone());
        self.annotations.busy = true;
        self.annotations.note_open = true;
        self.annotations.review = None;
        self.annotations.note = None;
        self.annotations.drafts.clear();
        self.annotations.stream.clear();
        self.annotations.status = "Preparing the approved images...".into();
        let sender = self.events_tx.clone();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let result = (|| {
                ticket.check()?;
                if preparing
                    && media::source_fingerprint(&capture)
                        .map_err(|e| e.to_string())?
                        .as_str()
                        != expected.as_deref().unwrap_or("")
                {
                    return Err(
                        "Capture changed since review. Open Write note and review it again.".into(),
                    );
                }
                let approved = if let Some(a) = previous {
                    a
                } else {
                    media::approve_preview(
                        &capture,
                        preview.ok_or("Review the baked images before sending.")?,
                        review
                            .as_deref()
                            .ok_or("Review the frames before sending.")?,
                    )
                    .map_err(|e| e.to_string())?
                };
                ticket.check()?;
                let _ = sender.send(Event::Annotations(Msg::Approved {
                    id,
                    approved: approved.clone(),
                }));
                if let Some(w) = &wake {
                    w();
                }
                generation::run(&capture, &approved, &options, &ticket, |progress| {
                    let _ = sender.send(Event::Annotations(Msg::Progress { id, progress }));
                    if let Some(w) = &wake {
                        w();
                    }
                })?;
                Ok(())
            })();
            let _ = sender.send(Event::Annotations(Msg::Finished { id, result }));
            if let Some(w) = wake {
                w();
            }
        });
    }
    pub(super) fn load_annotation_note_images(&mut self) {
        let Some(a) = &self.annotations.approved else {
            return;
        };
        let cache = ImageCache::new(self.waker.clone().unwrap_or_else(|| Arc::new(|| {})));
        for f in &a.approval.frames {
            if let Ok(bytes) =
                file_reads::read(&a.folder.join(&f.marked), annotations::IMAGE_MAX_BYTES)
            {
                let image = cache.load(bytes);
                if rune::tooling::headless_run().is_some() {
                    let _ = image.wait();
                }
                self.annotations.note_images.insert(f.index, image);
            }
        }
    }
    fn edit_annotation_note(&mut self) {
        let Some(note) = &self.annotations.note else {
            return;
        };
        let mut fields = BTreeMap::new();
        editable_fields(note, "", &mut fields);
        self.annotations.editing = fields;
    }
    fn save_annotation_edits(&mut self) {
        let Some(mut note) = self.annotations.note.clone() else {
            return;
        };
        let Some(approved) = &self.annotations.approved else {
            return;
        };
        for (path, text) in &self.annotations.editing {
            if let Some(value) = note.pointer_mut(path) {
                *value = json!(text.text);
            }
        }
        match note::validate(&note, approved).and_then(|()| {
            note::save(&approved.folder, &note)
                .and_then(|()| save_latest(&self.store.root, approved))
                .map_err(|_| "Could not save these edits. Your text is still here.".into())
        }) {
            Ok(()) => {
                self.annotations.note = Some(note);
                self.annotations.editing.clear();
                self.annotations.status = "Your edits are saved on this PC.".into();
            }
            Err(e) => self.annotations.status = e,
        }
    }
}
impl Drop for Annotations {
    fn drop(&mut self) {
        if let Some(t) = &self.ticket {
            t.cancel.stop();
        }
        self.epoch.invalidate();
    }
}
pub(super) fn editable_fields(v: &Value, path: &str, out: &mut BTreeMap<String, TextEdit>) {
    const TEXT: &[&str] = &[
        "title",
        "summary",
        "heading",
        "caption",
        "label",
        "explanation",
        "uncertainty",
        "action",
        "owner",
        "due_date",
    ];
    match v {
        Value::Object(obj) => {
            for (key, value) in obj {
                let next = format!("{path}/{key}");
                if TEXT.contains(&key.as_str()) && value.is_string() {
                    out.insert(next, TextEdit::new(value.as_str().unwrap()));
                } else if value.is_array() || value.is_object() {
                    editable_fields(value, &next, out);
                }
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let next = format!("{path}/{i}");
                if item.is_string() && (path.ends_with("/paragraphs") || path == "/open_questions")
                {
                    out.insert(next, TextEdit::new(item.as_str().unwrap()));
                } else {
                    editable_fields(item, &next, out);
                }
            }
        }
        _ => {}
    }
}
pub(super) fn review_page(s: &State, c: &Capture, width: f32, p: Palette, mut page: El) -> El {
    let frames = s.annotations.review.as_ref().unwrap();
    let selected = frames.iter().filter(|f| f.included).count();
    let marks: usize = frames
        .iter()
        .filter(|f| f.included)
        .map(|f| {
            c.data
                .marks
                .iter()
                .filter(|m| c.frame_for_mark(m).is_some_and(|kf| kf.index == f.index))
                .count()
        })
        .sum();
    let included: Vec<_> = c
        .data
        .marks
        .iter()
        .filter(|m| {
            c.frame_for_mark(m)
                .is_some_and(|f| frames.iter().any(|r| r.index == f.index && r.included))
        })
        .collect();
    let words: usize = included.iter().map(|m| c.words_for_mark(m).len()).sum();
    let crops = included
        .iter()
        .filter(|m| m.box_norm.and_then(media::normalized_box).is_some())
        .count();
    page=page.child(roles::SECTION.wrap("Review before sending",p.ink)).child(roles::SMALL.wrap("Drag on a frame to black out an area. Untick frames to leave them out.",p.soft))
        .child(roles::META.wrap(format!("Sends: {selected} original PNGs, {selected} marked PNGs, {crops} crops; {marks} marks, {words} timed words; app and window title."),p.ink).key("annotation-send-disclosure"));
    if let Ok(options) = s.annotation_options() {
        page=page.child(roles::META.wrap(format!("Via {} using {}. Crops include 15% padding. PNGs are at most 1568 px on the long edge.",options.helper.name(),options.model),p.soft));
    }
    if s.annotations.notebooks.pending.is_some() {
        page=page.child(roles::SMALL.wrap("This capture needs a note for the notebook. Review its images, then send for the capture note.",p.soft).key("notebook-capture-review-context"));
    }
    let mut controls = row().gap(12.0).items_center();
    let images_ready = s
        .annotations
        .preview
        .as_ref()
        .is_some_and(|preview| preview.approval.review == *frames)
        && frames.iter().filter(|f| f.included).all(|f| {
            s.annotations
                .preview_images
                .get(&f.index)
                .is_some_and(|i| i.size().is_some())
        });
    let allowed = frames.iter().any(|f| f.included)
        && images_ready
        && s.annotations.dragging.is_none()
        && s.annotation_options().is_ok();
    let send = crate::ui::widgets::button(
        "Send for note",
        "annotation-send",
        crate::ui::widgets::ButtonRole::Primary,
        p,
    );
    controls = controls
        .child(if allowed {
            send.on_press(|s: &mut State| s.send_annotation(false))
        } else {
            send.disabled(true)
                .help(if s.annotations.dragging.is_some() {
                    "Finish drawing the black-out box before sending.".into()
                } else if !images_ready {
                    "Wait for the selected frame images to load, or untick unavailable frames."
                        .into()
                } else {
                    s.annotation_options()
                        .err()
                        .unwrap_or_else(|| "Choose at least one frame.".into())
                })
        })
        .child(
            crate::ui::widgets::action("Cancel", p)
                .key("annotation-review-cancel")
                .on_press(|s: &mut State| {
                    s.annotations.review = None;
                    if let Some(id) = s.annotations.notebooks.pending.take() {
                        s.open_notebook(&id);
                    }
                }),
        )
        .child(
            link("Models and agents", p)
                .key("annotation-agent-settings")
                .on_press(|s: &mut State| {
                    s.open_hex();
                    s.hex.section = hex::Section::Connect;
                    s.load_annotation_options();
                }),
        );
    page = page.child(controls);
    if let Err(error) = s.annotation_options() {
        page = page.child(roles::SMALL.wrap(error, p.soft).role("status"));
    }
    if !s.annotations.status.is_empty() {
        page = page.child(
            roles::SMALL
                .wrap(&s.annotations.status, p.warn)
                .role("status"),
        );
    }
    for (i, frame) in c.data.keyframes.iter().enumerate() {
        let Some(review) = frames.iter().find(|r| r.index == frame.index) else {
            continue;
        };
        let index = frame.index;
        let included = review.included;
        let toggle = rune_kit::Checkbox::new(included)
            .label(format!(
                "Frame {}: {}",
                i + 1,
                frame.reason.as_deref().unwrap_or("Captured frame")
            ))
            .on_toggle(move |s: &mut State| {
                if let Some(frames) = &mut s.annotations.review
                    && let Some(frame) = frames.iter_mut().find(|f| f.index == index)
                {
                    frame.included = !frame.included;
                }
                s.prepare_annotation_preview();
            })
            .build(&theme::kit(&p))
            .key(format!("review-include-{index}"));
        page = page.child(
            row()
                .w_full()
                .items_center()
                .gap(12.0)
                .child(toggle)
                .child(roles::META.wrap(
                    format!(
                        "{} black-out {}",
                        review.masks.len(),
                        if review.masks.len() == 1 {
                            "box"
                        } else {
                            "boxes"
                        }
                    ),
                    p.soft,
                ))
                .child(
                    crate::ui::widgets::action("Undo box", p)
                        .key(format!("review-undo-{index}"))
                        .button_disabled(review.masks.is_empty())
                        .on_press(move |s: &mut State| {
                            if let Some(frames) = &mut s.annotations.review
                                && let Some(frame) = frames.iter_mut().find(|f| f.index == index)
                            {
                                frame.masks.pop();
                            }
                            s.prepare_annotation_preview();
                        }),
                ),
        );
        if !included {
            continue;
        }
        let image = s.annotations.preview_images.get(&index);
        let height = image
            .and_then(Image::size)
            .map_or(width * 0.5625, |[w, h]| width * h as f32 / w as f32);
        let mut canvas = el()
            .key(format!("review-image-{index}"))
            .w(width)
            .h(height)
            .role("img")
            .label("Baked redacted frame with numbered marks");
        if let Some(image) = image {
            canvas = canvas.pixels(image.pixels());
        }
        if let Some((dragging, b)) = s.annotations.dragging
            && dragging == index
        {
            canvas = canvas.child(
                el().absolute(b[0] * width, b[1] * height)
                    .w(b[2] * width)
                    .h(b[3] * height)
                    .fill(rune::Color::BLACK),
            );
        }
        canvas = canvas.on_drag(move |s: &mut State, d: &mut rune::Drag, _| {
            let Some(b) = media::box_from_display([d.start_x, d.start_y], [d.x, d.y], [d.w, d.h])
            else {
                s.annotations.dragging = None;
                return;
            };
            if d.phase == rune::Phase::Up {
                if !d.cancelled()
                    && b[2] * d.w >= 3.0
                    && b[3] * d.h >= 3.0
                    && let Some(frames) = &mut s.annotations.review
                    && let Some(frame) = frames.iter_mut().find(|f| f.index == index)
                {
                    frame.masks.push(b);
                }
                s.annotations.dragging = None;
                s.prepare_annotation_preview();
            } else {
                s.annotations.dragging = Some((index, b));
            }
        });
        page = page.child(canvas);
        for (n, m) in c.data.marks.iter().enumerate() {
            if c.frame_for_mark(m).is_some_and(|f| f.index == index) {
                page = page.child(callout(c, n + 1, m, p));
            }
        }
    }
    page
}
fn note_image(s: &State, index: u64, width: f32, p: Palette) -> El {
    if let Some(image) = s.annotations.note_images.get(&index)
        && let Some([w, h]) = image.size()
    {
        return el()
            .key(format!("note-image-{index}"))
            .w(width)
            .h(width * h as f32 / w as f32)
            .pixels(image.pixels())
            .role("img")
            .label("Approved redacted frame with numbered marks");
    }
    roles::SMALL.wrap("Loading approved frame...", p.soft)
}
pub(super) fn note_page(s: &State, _c: &Capture, width: f32, p: Palette, mut page: El) -> El {
    let mut actions = row().gap(12.0).items_center().child(
        link("Back to capture", p)
            .key("annotation-note-back")
            .on_press(|s: &mut State| {
                s.cancel_annotation_note();
                s.annotations.note_open = false;
            }),
    );
    if s.annotations.busy {
        actions = actions.child(
            super::super::area::button("Cancel", "annotation-note-cancel", p)
                .on_press(|s: &mut State| s.cancel_annotation_note()),
        );
    } else if s.annotations.note.is_none()
        && (s.annotations.approved.is_some() || s.annotations.last_review.is_some())
    {
        actions = actions.child(
            super::super::area::button("Retry", "annotation-note-retry", p)
                .help(
                    "Resend the same approved frames and words using the selected agent and model",
                )
                .on_press(|s: &mut State| s.send_annotation(true)),
        );
    } else if s.annotations.note.is_some() {
        actions = actions.child(
            super::super::area::button(
                if s.annotations.editing.is_empty() {
                    "Edit note"
                } else {
                    "Save changes"
                },
                "annotation-note-edit",
                p,
            )
            .on_press(|s: &mut State| {
                if s.annotations.editing.is_empty() {
                    s.edit_annotation_note();
                } else {
                    s.save_annotation_edits();
                }
            }),
        );
    }
    if s.annotations.note.is_some() && !s.annotations.busy && s.annotations.editing.is_empty() {
        actions = actions.child(
            super::super::area::button("Share", "share-capture", p)
                .on_press(|s: &mut State| s.share_annotation(false)),
        );
    }
    page = page.child(actions).child(
        roles::META
            .wrap(&s.annotations.status, p.soft)
            .key("annotation-note-status")
            .role("status"),
    );
    if !s.annotations.editing.is_empty() {
        let kit = theme::kit(&p);
        let mut fields: Vec<_> = s.annotations.editing.iter().collect();
        fields.sort_by_key(|(path, _)| {
            if path.as_str() == "/title" {
                0
            } else if path.as_str() == "/summary" {
                1
            } else if path.starts_with("/sections/") {
                2
            } else if path.starts_with("/figures/") {
                3
            } else if path.starts_with("/open_questions/") {
                4
            } else {
                5
            }
        });
        for (path, input) in fields {
            let key = path.clone();
            page = page
                .child(roles::GROUP.wrap(
                    edit_label(path, s.annotations.note.as_ref().unwrap()),
                    p.soft,
                ))
                .child(
                    TextArea::new(input)
                        .rows(3)
                        .width(None)
                        .label(edit_label(path, s.annotations.note.as_ref().unwrap()))
                        .on_edit(move |s: &mut State| s.annotations.editing.get_mut(&key).unwrap())
                        .build(&kit)
                        .key(format!("note-edit-{path}"))
                        .w_full(),
                );
        }
        return page;
    }
    let Some(note) = &s.annotations.note else {
        page = page.child(
            roles::SECTION
                .wrap("Draft", p.ink)
                .key("annotation-draft-label"),
        );
        let preview = generation::stream_preview(&s.annotations.stream);
        if !preview.is_empty() {
            page = page.child(
                roles::READ
                    .wrap(preview, p.ink)
                    .key("annotation-note-stream")
                    .selectable(true),
            );
        }
        if let Some(a) = &s.annotations.approved
            && (generation::uses_frame_drafts(a.approval.frames.len())
                || !s.annotations.drafts.is_empty())
        {
            for f in &a.approval.frames {
                if let Some(d) = s.annotations.drafts.get(&f.index) {
                    page = page.child(hex::settings::card(
                        &format!("Frame {} draft", f.index),
                        col()
                            .gap(10.0)
                            .padding(14.0)
                            .child(roles::READ.wrap(d["summary"].as_str().unwrap_or(""), p.ink))
                            .children(d["callouts"].as_array().unwrap_or(&Vec::new()).iter().map(
                                |c| {
                                    roles::SMALL.wrap(
                                        format!(
                                            "{}. {}",
                                            c["mark_number"],
                                            c["explanation"].as_str().unwrap_or("")
                                        ),
                                        p.soft,
                                    )
                                },
                            )),
                        p,
                    ));
                } else {
                    page = page.child(roles::SMALL.wrap(
                        format!("Frame {}: waiting for its draft...", f.index),
                        p.soft,
                    ));
                }
            }
        }
        if let Some(a) = &s.annotations.approved
            && !generation::uses_frame_drafts(a.approval.frames.len())
            && s.annotations.drafts.is_empty()
        {
            for frame in &a.approval.frames {
                page = page.child(note_image(s, frame.index, width, p));
            }
        }
        return page;
    };
    page = page
        .child(
            roles::SECTION
                .wrap(note["title"].as_str().unwrap_or("Annotation note"), p.ink)
                .key("annotation-note-title"),
        )
        .child(
            roles::READ
                .wrap(note["summary"].as_str().unwrap_or(""), p.ink)
                .key("annotation-note-summary")
                .selectable(true),
        );
    for section in note["sections"].as_array().unwrap() {
        page = page.child(
            roles::SECTION
                .wrap(section["heading"].as_str().unwrap(), p.ink)
                .mt(16.0),
        );
        for id in section["figure_refs"].as_array().unwrap() {
            let Some(figure) = note["figures"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| &f["figure_id"] == id)
            else {
                continue;
            };
            let index = figure["keyframe_index"].as_u64().unwrap();
            let side = width >= 860.0;
            let image_width = if side { width - 320.0 } else { width };
            let mut callouts = col().w(if side { 300.0 } else { width }).gap(10.0);
            for callout in figure["callouts"].as_array().unwrap() {
                let n = callout["mark_number"].as_u64().unwrap() as usize;
                let mut text = col()
                    .gap(8.0)
                    .padding(12.0)
                    .radius(8.0)
                    .border(1.0, p.line)
                    .fill(p.card)
                    .child(
                        row()
                            .gap(8.0)
                            .items_center()
                            .child(number(n, p))
                            .child(roles::NAME.wrap(callout["label"].as_str().unwrap(), p.ink)),
                    )
                    .child(
                        roles::SMALL
                            .wrap(callout["explanation"].as_str().unwrap(), p.ink)
                            .selectable(true),
                    );
                if let Some(unclear) = callout["uncertainty"].as_str() {
                    text = text.child(roles::SMALL.wrap(unclear, p.soft));
                }
                callouts = callouts.child(text.key(format!("note-callout-{n}")));
            }
            let image = note_image(s, index, image_width, p);
            page = page
                .child(if side {
                    row().w_full().gap(20.0).child(image).child(callouts)
                } else {
                    col().w_full().gap(12.0).child(image).child(callouts)
                })
                .child(roles::META.wrap(figure["caption"].as_str().unwrap(), p.soft));
        }
        for paragraph in section["paragraphs"].as_array().unwrap() {
            page = page.child(
                roles::READ
                    .wrap(paragraph.as_str().unwrap(), p.ink)
                    .selectable(true),
            );
        }
    }
    page = page.child(roles::SECTION.wrap("Open questions", p.ink).mt(16.0));
    for q in note["open_questions"].as_array().unwrap() {
        page = page.child(roles::READ.wrap(q.as_str().unwrap(), p.ink));
    }
    if note["open_questions"].as_array().unwrap().is_empty() {
        page = page.child(roles::SMALL.wrap("No open questions.", p.soft));
    }
    page = page.child(roles::SECTION.wrap("To-dos", p.ink).mt(16.0));
    for todo in note["todos"].as_array().unwrap() {
        page = page.child(roles::READ.wrap(todo["action"].as_str().unwrap(), p.ink));
    }
    if note["todos"].as_array().unwrap().is_empty() {
        page = page.child(roles::SMALL.wrap("No explicit to-dos.", p.soft));
    }
    page
}
fn edit_label(path: &str, note: &Value) -> String {
    let p: Vec<_> = path.trim_start_matches('/').split('/').collect();
    let number = |s: &str| s.parse::<usize>().unwrap_or(0) + 1;
    match p.as_slice() {
        ["title"] => "Title".into(),
        ["summary"] => "Summary".into(),
        ["open_questions", n] => format!("Open question {}", number(n)),
        ["sections", n, "heading"] => format!("Section {} heading", number(n)),
        ["sections", n, "paragraphs", paragraph] => {
            format!("Section {}, paragraph {}", number(n), number(paragraph))
        }
        ["figures", n, "caption"] => format!("Frame {} caption", number(n)),
        ["figures", n, "callouts", c, field] => {
            let mark = note
                .pointer(&format!("/figures/{n}/callouts/{c}/mark_number"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            format!("Callout {mark}: {field}")
        }
        ["todos", n, field] => format!("To-do {}: {}", number(n), field.replace('_', " ")),
        _ => "Note text".into(),
    }
}

pub(in crate::ui) fn agents_section(
    s: &State,
    k: &rune_kit::Kit,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    use super::super::{
        ask_settings::model_menu,
        hex::settings::{row_field_kit, setting_row, short_select_width},
        widgets::row_action,
    };
    use sidenote::core::{ask_options, ask_session, notes::Helper};
    let k = &row_field_kit(k, p);
    let selected = s.annotations.options.as_ref().map(|o| o.helper);
    let helper = selected.unwrap_or(Helper::Claude);
    let runner = s
        .runners
        .iter()
        .find(|r| ask_options::agent_name(r) == helper.name());
    let models = runner.map(ask_session::models).unwrap_or_default();
    let current = s.annotations.model.text.as_str();
    let other = s.annotations.other_model || !models.iter().any(|m| m.id == current);
    let mut choices = row().gap(10.0).flex_wrap();
    for helper in [Helper::Claude, Helper::Codex] {
        choices = choices.child(
            row_action(
                helper.name(),
                &format!("annotation-agent-{}", helper.name()),
                p,
            )
            .fill(if selected == Some(helper) {
                p.sel
            } else {
                p.card
            })
            .on_press(move |s: &mut State| s.choose_annotation_agent(helper)),
        );
    }
    let select = model_menu(
        "annotation-model-menu",
        "Annotation model",
        &models,
        Some(current),
        other,
    )
    .width(short_select_width(s, 220.0))
    .disabled(runner.is_none())
    .open(s.annotations.model_open)
    .on_toggle(|s: &mut State| s.annotations.model_open = !s.annotations.model_open)
    .on_close(|s: &mut State| s.annotations.model_open = false)
    .on_choose(|s: &mut State, value: &str, _| {
        s.annotations.model_open = false;
        if value == "Other\u{2026}" {
            s.annotations.other_model = true;
            return;
        }
        let helper = s
            .annotations
            .options
            .as_ref()
            .map_or(Helper::Claude, |o| o.helper);
        if let Some(runner) = s
            .runners
            .iter()
            .find(|r| ask_options::agent_name(r) == helper.name())
            && let Some(model) = ask_session::models(runner)
                .into_iter()
                .find(|m| m.name == value)
        {
            s.annotations.model = TextEdit::new(&model.id);
            s.save_annotation_model();
        }
    })
    .build(k, layer);
    let mut model = col().gap(8.0).child(select);
    if other {
        model = model.child(
            row()
                .gap(8.0)
                .items_center()
                .flex_wrap()
                .child(
                    rune_kit::TextInput::new(&s.annotations.model)
                        .label("Other annotation model")
                        .placeholder("Model name")
                        .height(30.0)
                        .on_edit(|s: &mut State| &mut s.annotations.model)
                        .on_submit(|s: &mut State, _| s.save_annotation_model())
                        .build(k)
                        .key("annotation-model")
                        .w(205.0),
                )
                .child(
                    row_action("Save", "annotation-save-model", p)
                        .on_press(|s: &mut State| s.save_annotation_model()),
                ),
        );
    }
    let mut body = col().w_full()
        .child(setting_row(s, "annotation-agent-row", "Agent", "Images and words are sent only after Send for note. Image input for OpenCode is not supported in this version.", choices, true, p))
        .child(setting_row(s, "annotation-model-row", "Model", "Choose an offered model, or choose Other to enter a model name.", model, true, p));
    if !s.annotations.status.is_empty() {
        body = body.child(
            roles::META
                .wrap(&s.annotations.status, p.soft)
                .role("status"),
        );
    }
    hex::settings::card("Annotation notes", body, p)
}

#[test]
fn annotation_model_save_keeps_the_agent_after_reenabling() {
    use sidenote::core::{Feature, notes::Helper, state_dir};

    const ISOLATED: &str = "SIDENOTE_ANNOTATION_OPTIONS_TEST";
    if std::env::var_os(ISOLATED).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ui::annotations::workflow::annotation_model_save_keeps_the_agent_after_reenabling",
                "--nocapture",
            ])
            .env(ISOLATED, "1")
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    crate::test_paths::isolate();
    let root = state_dir();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("config.json"),
        r#"{"version":1,"features":{"annotations":true},"notes":{}}"#,
    )
    .unwrap();
    let mut state = State::new();
    state.choose_annotation_agent(Helper::Codex);
    state.annotations.model = TextEdit::new("synthetic-original-model");
    state.save_annotation_model();
    state.toggle_feature(Feature::Annotations);
    drop(state);

    let mut restarted = State::new();
    assert!(!restarted.config.features.annotations);
    assert!(restarted.annotations.options.is_none());
    restarted.annotations.model = TextEdit::new("synthetic-unloaded-model");
    restarted.save_annotation_model();
    let saved = generation::load_options(&root).unwrap().unwrap();
    assert_eq!(saved.helper, Helper::Codex);
    assert_eq!(saved.model, "synthetic-original-model");

    restarted.toggle_feature(Feature::Annotations);
    assert!(restarted.config.features.annotations);
    assert_eq!(restarted.annotations.model.text, "synthetic-original-model");
    restarted.annotations.model = TextEdit::new("synthetic-replacement-model");
    restarted.save_annotation_model();
    let saved = generation::load_options(&root).unwrap().unwrap();
    assert_eq!(saved.helper, Helper::Codex);
    assert_eq!(saved.model, "synthetic-replacement-model");
}
