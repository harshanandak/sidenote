//! Ordered local notebooks. Images remain immutable approved annotation assets.
use super::{
    Capture,
    media::{self, ApprovedCapture},
    note,
};
use crate::core::{file_reads, store_io};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reference {
    pub key: String,
    pub folder: PathBuf,
    pub title: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub key: String,
    pub approved: ApprovedCapture,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub value: Value,
    pub assets: Vec<Asset>,
}
/// Reading projection: one approved image per content hash, with notebook-wide badges.
/// Source notes and approval files keep their original metadata and bytes.
pub fn reading_value(doc: &Document) -> Value {
    let mut value = doc.value.clone();
    let mut seen = std::collections::BTreeMap::<String, (String, Value)>::new();
    let mut next = 1_u64;
    for (section, asset) in value["sections"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(&doc.assets)
    {
        let heading = section["heading"].as_str().unwrap().to_owned();
        for figure in section["figures"].as_array_mut().unwrap() {
            let frame = asset
                .approved
                .approval
                .frames
                .iter()
                .find(|f| Some(f.index) == figure["keyframe_index"].as_u64())
                .unwrap();
            let hash = &asset.approved.approval.file_hashes[&frame.marked];
            if let Some((first, original)) = seen.get(hash) {
                let previous = original["callouts"].as_array().unwrap();
                let numbers = previous
                    .iter()
                    .map(|c| c["mark_number"].as_u64().unwrap())
                    .collect::<Vec<_>>();
                let marks = numbers
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut extra = Vec::new();
                if figure["caption"] != original["caption"] {
                    extra.push(figure["caption"].clone());
                }
                for callout in figure["callouts"].as_array_mut().unwrap() {
                    if let Some(prior) = previous
                        .iter()
                        .find(|c| c["original_mark_number"] == callout["mark_number"])
                    {
                        callout["original_mark_number"] = callout["mark_number"].clone();
                        callout["mark_number"] = prior["mark_number"].clone();
                        if callout["label"] != prior["label"]
                            || callout["explanation"] != prior["explanation"]
                        {
                            extra.push(json!(format!(
                                "Mark {}: {}. {}",
                                callout["mark_number"],
                                callout["label"].as_str().unwrap(),
                                callout["explanation"].as_str().unwrap()
                            )));
                        }
                    }
                }
                figure["reuse_explanations"] = json!(extra);
                figure["same_screen"] = json!(if numbers.is_empty() {
                    format!("Same screen as {first}.")
                } else {
                    format!(
                        "Same screen as {first}, {} {marks}.",
                        if numbers.len() == 1 { "mark" } else { "marks" }
                    )
                });
                continue;
            }
            for callout in figure["callouts"].as_array_mut().unwrap() {
                callout["original_mark_number"] = callout["mark_number"].clone();
                callout["mark_number"] = json!(next);
                next += 1;
            }
            seen.insert(hash.clone(), (heading.clone(), figure.clone()));
        }
    }
    value
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notebook {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub captures: Vec<Reference>,
    pub revision: u64,
    pub note_revision: u64,
    pub note: Option<Document>,
}
#[derive(Clone)]
pub struct Input {
    pub key: String,
    pub approved: ApprovedCapture,
    pub note: Value,
}
fn err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() < 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
fn path(root: &Path, id: &str) -> io::Result<PathBuf> {
    if !valid_id(id) {
        return Err(err("Invalid notebook name in storage."));
    }
    Ok(root
        .join("annotations/notebooks")
        .join(format!("{id}.json")))
}
impl Notebook {
    pub fn new(title: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            id: format!(
                "{}-{}-{nonce:020}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
                std::process::id()
            ),
            title: title.into(),
            purpose: String::new(),
            captures: Vec::new(),
            revision: 0,
            note_revision: 0,
            note: None,
        }
    }
    pub fn add(&mut self, c: &Capture) -> bool {
        let key = media::capture_key(c);
        if self.captures.iter().any(|r| r.key == key) {
            return false;
        }
        self.captures.push(Reference {
            key,
            folder: c.folder.clone(),
            title: c.title().into(),
        });
        self.revision += 1;
        true
    }
    pub fn reorder(&mut self, from: usize, to: usize) -> bool {
        if from >= self.captures.len() || to >= self.captures.len() || from == to {
            return false;
        }
        let current = self.fresh();
        let item = self.captures.remove(from);
        self.captures.insert(to, item);
        self.revision += 1;
        if current
            && let Some(doc) = &mut self.note
            && let Some(sections) = doc.value["sections"].as_array_mut()
            && sections.len() == self.captures.len()
            && doc.assets.len() == sections.len()
        {
            let section = sections.remove(from);
            sections.insert(to, section);
            let asset = doc.assets.remove(from);
            doc.assets.insert(to, asset);
            self.note_revision = self.revision;
        }
        true
    }
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.captures.len() {
            return false;
        }
        self.captures.remove(index);
        self.revision += 1;
        true
    }
    pub fn fresh(&self) -> bool {
        self.note.is_some() && self.revision == self.note_revision
    }
}
pub fn save(root: &Path, book: &Notebook) -> io::Result<()> {
    if book.title.trim().is_empty()
        || book.title.len() > 1000
        || book.purpose.len() > 2000
        || book.captures.len() > 100
    {
        return Err(err("Use a title and at most 100 captures."));
    }
    let mut seen = BTreeSet::new();
    if book.captures.iter().any(|r| !seen.insert(&r.key)) {
        return Err(err("A capture is listed twice."));
    }
    let p = path(root, &book.id)?;
    store_io::reject_reparse(&p)?;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    store_io::atomic_write(&p, &serde_json::to_vec_pretty(book)?)
}
pub fn list(root: &Path) -> io::Result<Vec<Notebook>> {
    let folder = root.join("annotations/notebooks");
    store_io::reject_reparse(&folder)?;
    let entries = match fs::read_dir(folder) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut books = Vec::new();
    for entry in entries.take(1000) {
        let entry = entry?;
        if entry.path().extension().is_some_and(|e| e == "json") {
            let b: Notebook =
                serde_json::from_slice(&file_reads::read(&entry.path(), 64 * 1024 * 1024)?)?;
            if path(root, &b.id)? != entry.path() {
                return Err(err("Notebook identity changed."));
            }
            books.push(b);
        }
    }
    books.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(books)
}
pub fn load_capture_note(root: &Path, c: &Capture) -> io::Result<Option<(ApprovedCapture, Value)>> {
    let folder = media::note_folder(root, c);
    let bytes = match file_reads::read(&folder.join("latest.json"), 4096) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let latest: Value = serde_json::from_slice(&bytes)?;
    let leaf = latest["approved_folder"]
        .as_str()
        .ok_or_else(|| err("Invalid approval link"))?;
    if leaf.is_empty() || !leaf.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err(err("Invalid approval link"));
    }
    let folder = folder.join("approved").join(leaf);
    let approved = ApprovedCapture {
        approval: serde_json::from_slice(&file_reads::read(
            &folder.join("approval.json"),
            16 * 1024 * 1024,
        )?)?,
        folder,
    };
    approved.verify()?;
    if media::source_fingerprint(c)? != approved.approval.source_fingerprint {
        return Ok(None);
    }
    let Some(value) = note::load(&approved.folder)? else {
        return Ok(None);
    };
    note::validate(&value, &approved).map_err(err)?;
    Ok(Some((approved, value)))
}
pub fn text_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["title","summary","sections"],"properties":{"title":{"type":"string"},"summary":{"type":"string"},"sections":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["capture_key","heading","paragraphs"],"properties":{"capture_key":{"type":"string"},"heading":{"type":"string"},"paragraphs":{"type":"array","items":{"type":"string"}}}}}}})
}
pub fn prompt(book: &Notebook, inputs: &[Input]) -> String {
    let mut notes = Vec::<Value>::new();
    let mut known = std::collections::BTreeMap::new();
    let mut captures = Vec::new();
    for (reference, input) in book.captures.iter().zip(inputs) {
        let mut points = Vec::new();
        let mut seen = BTreeSet::new();
        for section in input.note["sections"].as_array().unwrap() {
            if let Some(heading) = section["heading"].as_str()
                && seen.insert(heading.to_owned())
            {
                points.push(json!(heading));
            }
            for p in section["paragraphs"].as_array().unwrap() {
                if seen.insert(p.as_str().unwrap().to_owned()) {
                    points.push(p.clone());
                }
            }
        }
        for figure in input.note["figures"].as_array().unwrap() {
            if let Some(caption) = figure["caption"].as_str()
                && seen.insert(caption.to_owned())
            {
                points.push(json!(caption));
            }
            for c in figure["callouts"].as_array().unwrap() {
                let point = format!(
                    "{}: {}",
                    c["label"].as_str().unwrap(),
                    c["explanation"].as_str().unwrap()
                );
                if seen.insert(point.clone()) {
                    points.push(json!(point));
                }
            }
        }
        let prose = json!({"title":input.note["title"],"summary":input.note["summary"],"points":points,"open_questions":merged_questions(&input.note),"todos":input.note["todos"].as_array().unwrap().iter().map(|t|json!({"action":t["action"],"owner":t["owner"],"due_date":t["due_date"]})).collect::<Vec<_>>()});
        let fingerprint = prose.to_string();
        let index = if let Some(index) = known.get(&fingerprint) {
            *index
        } else {
            let index = notes.len();
            known.insert(fingerprint, index);
            notes.push(prose);
            index
        };
        captures.push(
            json!({"capture_key":input.key,"capture_title":reference.title,"note_reference":index}),
        );
    }
    format!(
        "{}\nNotebook title: {}\nPurpose: {}\nOrdered captures and existing note prose (note_reference indexes notes):\n{}\nRequired JSON schema:\n{}",
        include_str!("prompts/notebook.md"),
        book.title,
        book.purpose,
        json!({"captures":captures,"notes":notes}),
        text_schema()
    )
}
fn normalized(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(['?', '!', '.'])
        .to_lowercase()
}

/// Keep legacy inline uncertainty at the end as well as explicit open questions.
pub(super) fn merged_questions(value: &Value) -> Vec<Value> {
    fn collect(value: &Value, output: &mut Vec<String>) {
        match value {
            Value::Object(fields) => {
                for (key, v) in fields {
                    if key == "uncertainty" {
                        if let Some(s) = v.as_str() {
                            output.push(s.into());
                        }
                    } else {
                        collect(v, output);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect(item, output);
                }
            }
            _ => {}
        }
    }
    let mut candidates = value["open_questions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    collect(value, &mut candidates);
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|s| !s.trim().is_empty() && seen.insert(normalized(s)))
        .map(Value::String)
        .collect()
}
pub fn assemble(book: &Notebook, inputs: &[Input], text: Value) -> Result<Document, String> {
    if !jsonschema::validator_for(&text_schema())
        .map_err(|e| e.to_string())?
        .is_valid(&text)
    {
        return Err("The notebook note has an invalid structure. Retry it.".into());
    }
    let sections = text["sections"].as_array().unwrap();
    if sections.len() != book.captures.len() || inputs.len() != book.captures.len() {
        return Err("Notebook captures changed. Write the note again.".into());
    }
    let mut figures = Vec::new();
    let mut assets = Vec::new();
    let mut questions = Vec::new();
    let mut todos = Vec::new();
    let mut qseen = BTreeSet::new();
    let mut tseen = BTreeSet::new();
    for ((reference, input), section) in book.captures.iter().zip(inputs).zip(sections) {
        if reference.key != input.key || section["capture_key"] != input.key {
            return Err("The notebook note changed capture order.".into());
        }
        input.approved.verify().map_err(|e| e.to_string())?;
        note::validate(&input.note, &input.approved)?;
        let mut section = section.clone();
        section["figures"] = json!(
            input.note["sections"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|s| s["figure_refs"].as_array().unwrap())
                .map(|id| input.note["figures"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|f| &f["figure_id"] == id)
                    .unwrap()
                    .clone())
                .collect::<Vec<_>>()
        );
        for figure in section["figures"].as_array_mut().unwrap() {
            for callout in figure["callouts"].as_array_mut().unwrap() {
                callout["uncertainty"] = Value::Null;
            }
        }
        figures.push(section);
        assets.push(Asset {
            key: input.key.clone(),
            approved: input.approved.clone(),
        });
        for q in merged_questions(&input.note) {
            let s = q.as_str().unwrap();
            if qseen.insert(normalized(s)) {
                questions.push(q.clone());
            }
        }
        for t in input.note["todos"].as_array().unwrap() {
            let k = format!(
                "{}|{}|{}",
                normalized(t["action"].as_str().unwrap()),
                t["owner"],
                t["due_date"]
            );
            if tseen.insert(k) {
                todos.push(t.clone());
            }
        }
    }
    let doc = Document {
        value: json!({"title":text["title"],"summary":text["summary"],"sections":figures,"open_questions":questions,"todos":todos}),
        assets,
    };
    validate(&doc)?;
    Ok(doc)
}
pub fn validate(doc: &Document) -> Result<(), String> {
    let mut schema = text_schema();
    schema["required"] = json!(["title", "summary", "sections", "open_questions", "todos"]);
    let source: Value = serde_json::from_str(note::SCHEMA).map_err(|e| e.to_string())?;
    schema["properties"]["open_questions"] = source["properties"]["open_questions"].clone();
    schema["properties"]["todos"] = source["properties"]["todos"].clone();
    schema["properties"]["sections"]["items"]["required"] =
        json!(["capture_key", "heading", "paragraphs", "figures"]);
    schema["properties"]["sections"]["items"]["properties"]["figures"] =
        source["properties"]["figures"].clone();
    if !jsonschema::validator_for(&schema)
        .map_err(|e| e.to_string())?
        .is_valid(&doc.value)
    {
        return Err("The notebook note is invalid.".into());
    }
    let sections = doc.value["sections"].as_array().unwrap();
    if sections.len() != doc.assets.len() {
        return Err("The notebook note lost a capture.".into());
    }
    let mut keys = BTreeSet::new();
    for (section, asset) in sections.iter().zip(&doc.assets) {
        if section["capture_key"] != asset.key || !keys.insert(&asset.key) {
            return Err("The notebook note changed capture order.".into());
        }
        asset.approved.verify().map_err(|e| e.to_string())?;
        let figs = &section["figures"];
        let cap = json!({"title":section["heading"],"summary":"","sections":[{"heading":"","paragraphs":[],"figure_refs":figs.as_array().unwrap().iter().map(|f|f["figure_id"].clone()).collect::<Vec<_>>()}],"figures":figs,"open_questions":[],"todos":[]});
        note::validate(&cap, &asset.approved)?;
    }
    Ok(())
}

/// Invented captures only, shared by unit tests, offscreen scenes and the explicit live proof.
pub fn synthetic(
    root: &Path,
    count: usize,
) -> Result<(Notebook, Vec<Input>, Vec<Capture>), String> {
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/annotations/synthetic.scrawl");
    let source = super::read_capture(&base).map_err(|e| e.to_string())?;
    let mut book = Notebook::new("Paper prototypes and Friday review");
    book.purpose = "Explain the workshop setup and the next review.".into();
    let mut inputs = Vec::new();
    let mut captures = Vec::new();
    for index in 0..count {
        let folder = root
            .join("synthetic")
            .join(format!("capture-{}.scrawl", index + 1));
        fs::create_dir_all(folder.join("frames")).map_err(|e| e.to_string())?;
        let mut data = source.data.clone();
        data.id = format!("notebook-synthetic-{}", index + 1);
        data.target.window_title = Some(
            [
                "Workshop setup",
                "Materials for the prototype",
                "Friday review",
            ][index % 3]
                .into(),
        );
        for frame in &data.keyframes {
            let name = frame.file.as_deref().ok_or("Missing synthetic frame")?;
            let destination = folder.join(name);
            if let Some(p) = destination.parent() {
                fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            fs::copy(
                source.frame_path(frame).map_err(|e| e.to_string())?,
                destination,
            )
            .map_err(|e| e.to_string())?;
        }
        store_io::atomic_write(
            &folder.join("scrawl.json"),
            &serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let c = super::read_capture(&folder).map_err(|e| e.to_string())?;
        let review = c
            .data
            .keyframes
            .iter()
            .enumerate()
            .map(|(i, f)| media::ReviewFrame {
                index: f.index,
                included: true,
                masks: if i == 0 {
                    vec![media::SYNTHETIC_MASK]
                } else {
                    Vec::new()
                },
            })
            .collect::<Vec<_>>();
        let approved = media::approve(&c, &review, root).map_err(|e| e.to_string())?;
        let mut value: Value = serde_json::from_str(include_str!(
            "../../../tools/ui-fixtures/annotation-note.json"
        ))
        .map_err(|e| e.to_string())?;
        for figure in value["figures"].as_array_mut().unwrap() {
            let frame = approved
                .approval
                .frames
                .iter()
                .find(|f| Some(f.index) == figure["keyframe_index"].as_u64())
                .unwrap();
            for key in [
                "capture_id",
                "app",
                "window_title",
                "keyframe_index",
                "keyframe_reason",
            ] {
                figure[key] = frame.metadata[key].clone();
            }
            for callout in figure["callouts"].as_array_mut().unwrap() {
                let mark = frame.metadata["marks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["mark_number"] == callout["mark_number"])
                    .unwrap();
                for key in ["mark_id", "boxNorm", "description"] {
                    callout[key] = mark[key].clone();
                }
                callout["evidence_refs"] = json!([mark["mark_id"]]);
            }
        }
        note::validate(&value, &approved)?;
        note::save(&approved.folder, &value).map_err(|e| e.to_string())?;
        media::publish_latest(root, &approved).map_err(|e| e.to_string())?;
        book.add(&c);
        inputs.push(Input {
            key: media::capture_key(&c),
            approved,
            note: value,
        });
        captures.push(c);
    }
    Ok((book, inputs, captures))
}
pub fn synthetic_document(book: &Notebook, inputs: &[Input]) -> Result<Document, String> {
    assemble(
        book,
        inputs,
        json!({"title":book.title,"summary":"Use paper cards to prepare the workshop prototypes. Review the work on Friday morning and bring two versions.","sections":book.captures.iter().zip(inputs).map(|(r,i)|json!({"capture_key":r.key,"heading":r.title,"paragraphs":[i.note["summary"]]})).collect::<Vec<_>>()}),
    )
}
pub fn live_proof(root: &Path) -> Result<(), String> {
    benchmark(root, "sonnet-medium")
}
#[doc(hidden)]
pub fn benchmark(root: &Path, variant: &str) -> Result<(), String> {
    let (helper, model, effort) = match variant {
        "sonnet-medium" => (
            crate::core::notes::Helper::Claude,
            "claude-sonnet-5-5",
            "medium",
        ),
        "codex-low" => (crate::core::notes::Helper::Codex, "gpt-5.6-luna", "low"),
        "codex-medium" => (crate::core::notes::Helper::Codex, "gpt-5.6-luna", "medium"),
        "haiku" => (crate::core::notes::Helper::Claude, "haiku", "low"),
        _ => return Err("Unknown notebook benchmark variant.".into()),
    };
    let (mut book, inputs, _) = synthetic(root, 3)?;
    save(root, &book).map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    let first = std::sync::Mutex::new(None::<f64>);
    let first_header = std::sync::Mutex::new(None::<f64>);
    let preview_events = std::sync::Mutex::new(Vec::<Value>::new());
    let ticket = super::generation::Epoch::default().begin();
    let options = super::generation::Options {
        helper,
        model: model.into(),
    };
    let doc = super::generation::run_notebook_at_effort(
        &book,
        &inputs,
        &options,
        &root.join("job"),
        &ticket,
        effort,
        |text| {
            let elapsed = started.elapsed().as_secs_f64();
            let preview = super::generation::notebook_stream_preview(&text);
            if !preview.trim().is_empty() {
                let mut first = first.lock().unwrap();
                if first.is_none() {
                    *first = Some(elapsed);
                }
            }
            if !super::generation::stream_preview(&text).trim().is_empty() {
                let mut first = first_header.lock().unwrap();
                if first.is_none() {
                    *first = Some(elapsed);
                }
            }
            preview_events
                .lock()
                .unwrap()
                .push(json!({"seconds":elapsed,"visible_text":preview}));
        },
    )?;
    let total = started.elapsed().as_secs_f64();
    book.note = Some(doc.clone());
    book.note_revision = book.revision;
    save(root, &book).map_err(|e| e.to_string())?;
    let html = super::share::notebook(&doc)?;
    super::share::save(&root.join("notebook.html"), &html)?;
    store_io::atomic_write(
        &root.join("notebook-note.json"),
        &serde_json::to_vec_pretty(&doc.value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    store_io::atomic_write(&root.join("timing.json"),&serde_json::to_vec_pretty(&json!({"variant":variant,"effort":effort,"model":options.model,"captures":3,"first_text_seconds":*first.lock().unwrap(),"first_title_summary_seconds":*first_header.lock().unwrap(),"complete_seconds":total,"valid":true,"images_sent":0,"existing_notes_only":true})).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    store_io::atomic_write(
        &root.join("stream-proof.json"),
        &serde_json::to_vec_pretty(&*preview_events.lock().unwrap()).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!(
        "Notebook proof: first text {:?} s, complete {total:.2} s",
        *first.lock().unwrap()
    );
    Ok(())
}
