//! Device-local write-ahead history. Immutable intent and outcome files form one append-only log.
use super::{
    store_io::{self, invalid},
    sync_state,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    TaskProperties,
    ProjectAssignment,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Summary,
    Projects,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Guard {
    store: String,
    prefixes: Vec<Vec<String>>,
    versions: Vec<(Vec<String>, String)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Target {
    content: Option<Value>,
    file: FileKind,
    task_id: Option<String>,
    index: Option<usize>,
    identity: Option<String>,
    guards: Vec<Guard>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub time: String,
    pub kind: Kind,
    pub item_id: String,
    pub label: String,
    pub source_meeting_id: String,
    pub source_offset: Option<String>,
    pub reason: String,
    /// The existing extractors supply no calibrated score.
    pub confidence: Option<f64>,
    pub before: Value,
    pub after: Value,
    pub undo_of: Option<String>,
    target: Target,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Mutation {
    meeting: String,
    file: FileKind,
    before: Option<Value>,
    after: Value,
    create_only: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Intent {
    version: u32,
    sequence: u64,
    id: String,
    device: String,
    entries: Vec<Entry>,
    mutation: Mutation,
    checksum: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Outcome {
    id: String,
    applied: bool,
}
#[derive(Clone, Debug)]
pub struct Row {
    pub entry: Entry,
    pub undo_target: String,
    pub undone: bool,
}
#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
    pub meetings: PathBuf,
}
impl Default for Store {
    fn default() -> Self {
        Self::new(super::state_dir(), super::meetings_dir())
    }
}
pub(super) fn valid_id(id: &str) -> bool {
    id.split_once('-').is_some_and(|(a, b)| {
        !a.is_empty() && !b.is_empty() && a.bytes().chain(b.bytes()).all(|b| b.is_ascii_digit())
    })
}
fn valid_log_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit())
}
fn props(task: &Value) -> Value {
    json!({"owner":task.get("owner").cloned().unwrap_or(Value::Null),"due":task.get("due").cloned().unwrap_or(Value::Null),"due_default":task.get("due_default").and_then(Value::as_bool).unwrap_or(false),"status":task.get("status").and_then(Value::as_str).filter(|s|!s.is_empty()).unwrap_or("todo"),"waiting_on":task.get("waiting_on").cloned().unwrap_or(Value::Null),"team":task.get("team").cloned().unwrap_or(Value::Null),"subtasks":task.get("subtasks").cloned().unwrap_or_else(||json!([]))})
}
fn content(task: &Value) -> Value {
    let mut value = task.clone();
    if let Some(object) = value.as_object_mut() {
        for key in [
            "owner",
            "due",
            "due_default",
            "status",
            "waiting_on",
            "team",
            "subtasks",
        ] {
            object.remove(key);
        }
    }
    value
}
fn identity(task: &Value) -> String {
    task.get("task")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| task.get("title").and_then(Value::as_str))
        .unwrap_or("")
        .into()
}
pub(super) fn summary_lock(root: &Path) -> io::Result<fs::File> {
    store_io::lock(&root.join("local/settle-log.lock"))
}

impl Store {
    pub fn new(root: PathBuf, meetings: PathBuf) -> Self {
        Self { root, meetings }
    }
    fn dir(&self) -> PathBuf {
        self.root.join("memory/settle-log")
    }
    fn lock(&self) -> io::Result<fs::File> {
        summary_lock(&self.root)
    }
    pub(super) fn recycle_claim(&self, ids: &BTreeSet<String>) -> io::Result<Option<fs::File>> {
        let lock = match store_io::try_lock(&self.root.join("local/settle-log.lock")) {
            Ok(lock) => lock,
            Err(e) if super::meeting_delete::busy(&e) => return Ok(None),
            Err(e) => return Err(e),
        };
        for intent in self.intents()? {
            if ids.contains(&intent.mutation.meeting) && self.outcome(&intent)?.is_none() {
                return Ok(None);
            }
        }
        Ok(Some(lock))
    }

    fn source(&self, meeting: &str, kind: &FileKind) -> io::Result<PathBuf> {
        if !valid_id(meeting) {
            return Err(invalid("Invalid source meeting"));
        }
        let folder = self.meetings.join(meeting);
        if super::meeting_delete::suppressed_ids(&self.root)?.contains(meeting)
            || !super::file_reads::directory(&folder)
            || !folder.join("manifest.json").is_file()
        {
            return Err(invalid("This meeting was deleted. Nothing was changed."));
        }
        Ok(folder.join(match kind {
            FileKind::Summary => "english-summary.json",
            FileKind::Projects => "tasks.json",
        }))
    }
    fn read(&self, path: &Path) -> io::Result<Option<Value>> {
        match store_io::read_bounded(path, super::file_reads::SUMMARY_MAX) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(
                bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes),
            )?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn versions(
        &self,
        store: &str,
        prefixes: &[Vec<String>],
    ) -> io::Result<Vec<(Vec<String>, String)>> {
        Ok(sync_state::settle_versions(&self.root, store)?
            .into_iter()
            .filter(|(key, _)| prefixes.iter().any(|prefix| key.starts_with(prefix)))
            .collect())
    }
    fn project_source(&self, meeting: &str) -> io::Result<Option<Value>> {
        Ok(self
            .read(&self.source(meeting, &FileKind::Summary)?)?
            .map(|summary| {
                json!(
                    summary
                        .get("todo")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(content)
                        .collect::<Vec<_>>()
                )
            }))
    }
    fn guards(&self, meeting: &str, task: Option<(usize, &Value)>) -> io::Result<Vec<Guard>> {
        let mut specs = vec![(
            "projects.json".to_string(),
            vec![vec!["meeting_projects".into(), meeting.into()]],
        )];
        specs[0].1.push(vec![format!(
            "assignment:{}",
            super::areas::meeting_key(meeting)
        )]);
        if let Some((index, task)) = task {
            specs[0].1.push(vec![format!(
                "assignment:{}",
                json!(["task", meeting, index, identity(task)])
            )]);
            let id = task.get("id").and_then(Value::as_str).unwrap_or("");
            specs.push((
                "task-overrides.json".into(),
                vec![
                    vec!["tasks".into(), meeting.into(), index.to_string()],
                    vec!["tasks".into(), meeting.into(), format!("id:{id}")],
                ],
            ));
            specs.push((
                format!("{meeting}.json"),
                vec![
                    vec!["checked".into(), identity(task)],
                    vec!["checked".into(), super::task_tick_key(id)],
                    vec!["checked".into(), format!("open:{id}")],
                ],
            ));
        } else {
            if let Some(summary) = self.read(&self.source(meeting, &FileKind::Summary)?)? {
                for (index, task) in summary
                    .get("todo")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    specs[0].1.push(vec![format!(
                        "assignment:{}",
                        json!(["task", meeting, index, identity(task)])
                    )]);
                }
            }
            specs.push((
                "task-overrides.json".into(),
                vec![vec!["tasks".into(), meeting.into()]],
            ));
        }
        specs
            .into_iter()
            .map(|(store, prefixes)| {
                Ok(Guard {
                    versions: self.versions(&store, &prefixes)?,
                    store,
                    prefixes,
                })
            })
            .collect()
    }
    fn guards_match(&self, entry: &Entry) -> io::Result<bool> {
        if entry.kind == Kind::ProjectAssignment
            && self.project_source(&entry.source_meeting_id)? != entry.target.content
        {
            return Ok(false);
        }
        for guard in &entry.target.guards {
            if self.versions(&guard.store, &guard.prefixes)? != guard.versions {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn intents(&self) -> io::Result<Vec<Intent>> {
        let entries = match fs::read_dir(self.dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let device = super::DeviceSettings::load_in(&self.root)?.id;
        let mut out = vec![];
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".intent.json") else {
                continue;
            };
            if !valid_log_id(id) {
                return Err(invalid("Settle history has an invalid entry."));
            }
            let bytes = store_io::read_bounded(&entry.path(), 64 * 1024 * 1024)?;
            let mut intent: Intent = serde_json::from_slice(&bytes)?;
            let checksum = std::mem::take(&mut intent.checksum);
            if intent.version != 1
                || intent.entries.is_empty()
                || intent.id != id
                || checksum
                    != blake3::hash(&serde_json::to_vec(&intent)?)
                        .to_hex()
                        .as_str()
            {
                return Err(invalid(
                    "Settle history could not be verified. Nothing was changed.",
                ));
            }
            intent.checksum = checksum;
            if intent.device == device {
                out.push(intent);
            }
        }
        out.sort_by_key(|intent| intent.sequence);
        Ok(out)
    }
    fn outcome(&self, intent: &Intent) -> io::Result<Option<Outcome>> {
        let path = self.dir().join(format!("{}.done.json", intent.id));
        match store_io::read_bounded(&path, 4096) {
            Ok(bytes) => {
                let outcome: Outcome = serde_json::from_slice(&bytes)?;
                if outcome.id != intent.id {
                    return Err(invalid("Invalid settle outcome"));
                }
                Ok(Some(outcome))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn finish(&self, intent: &Intent, applied: bool) -> io::Result<()> {
        let value = Outcome {
            id: intent.id.clone(),
            applied,
        };
        store_io::publish(
            &self.dir().join(format!("{}.done.json", intent.id)),
            &serde_json::to_vec(&value)?,
        )
    }
    fn commit(&self, mutation: Mutation, entries: Vec<Entry>) -> io::Result<()> {
        if entries.is_empty() {
            let path = self.source(&mutation.meeting, &mutation.file)?;
            return if mutation.create_only {
                store_io::publish(&path, &serde_json::to_vec_pretty(&mutation.after)?)
            } else {
                store_io::write(&path, &mutation.after)
            };
        }
        let sequence = self
            .intents()?
            .last()
            .map_or(1, |intent| intent.sequence + 1);
        let mut intent = Intent {
            version: 1,
            sequence,
            id: super::device::random_id()?,
            device: super::DeviceSettings::load_in(&self.root)?.id,
            entries,
            mutation,
            checksum: String::new(),
        };
        intent.checksum = blake3::hash(&serde_json::to_vec(&intent)?)
            .to_hex()
            .to_string();
        store_io::publish(
            &self.dir().join(format!("{}.intent.json", intent.id)),
            &serde_json::to_vec(&intent)?,
        )?;
        self.land(&intent)
    }
    fn land(&self, intent: &Intent) -> io::Result<()> {
        let stores: BTreeSet<_> = intent
            .entries
            .iter()
            .flat_map(|e| e.target.guards.iter().map(|g| g.store.clone()))
            .collect();
        let _guards = sync_state::settle_locks(&self.root, &stores)?;
        let path = match self.source(&intent.mutation.meeting, &intent.mutation.file) {
            Ok(path) => path,
            Err(error) => {
                self.finish(intent, false)?;
                return Err(error);
            }
        };
        let current = self.read(&path)?;
        if current.as_ref() == Some(&intent.mutation.after) {
            return self.finish(intent, true);
        }
        if current != intent.mutation.before {
            self.finish(intent, false)?;
            return Err(invalid("This item changed. Nothing was overwritten."));
        }
        for entry in &intent.entries {
            if !self.guards_match(entry)? {
                self.finish(intent, false)?;
                return Err(invalid(
                    "You changed this item since Sidenote settled it. Nothing was changed.",
                ));
            }
        }
        if intent.mutation.create_only {
            store_io::publish(&path, &serde_json::to_vec_pretty(&intent.mutation.after)?)?;
        } else {
            store_io::write(&path, &intent.mutation.after)?;
        }
        sync_state::settle_fence();
        self.finish(intent, true)
    }
    fn recover_locked(&self) -> io::Result<usize> {
        let mut recovered = 0;
        for intent in self.intents()? {
            if self.outcome(&intent)?.is_none() {
                match self.land(&intent) {
                    Ok(()) => recovered += 1,
                    Err(e) if self.outcome(&intent)?.is_some() => {
                        let _ = e;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(recovered)
    }
    pub fn recover(&self) -> io::Result<usize> {
        if !self.dir().exists() {
            return Ok(0);
        }
        let _lock = self.lock()?;
        self.recover_locked()
    }
    pub fn publish_summary(
        &self,
        meeting: &str,
        summary: &Value,
        create_only: bool,
    ) -> io::Result<()> {
        let _lock = self.lock()?;
        super::DeviceSettings::load_in(&self.root)?;
        self.recover_locked()?;
        let path = self.source(meeting, &FileKind::Summary)?;
        // A script may publish an opaque summary while extraction is running.
        if create_only && fs::symlink_metadata(&path).is_ok() {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        let before = self.read(&path)?;
        if create_only && before.is_some() {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        let mut entries = vec![];
        for (index, task) in summary
            .get("todo")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let id = task.get("id").and_then(Value::as_str).unwrap_or("");
            let previous = before
                .as_ref()
                .and_then(|value| value.get("todo"))
                .and_then(Value::as_array)
                .and_then(|items| {
                    items.iter().find(|old| {
                        if id.is_empty() {
                            identity(old) == identity(task)
                        } else {
                            old.get("id").and_then(Value::as_str) == Some(id)
                        }
                    })
                });
            let old = previous.map(props).unwrap_or_else(|| props(&Value::Null));
            let after = props(task);
            if old == after {
                continue;
            }
            let guessed = task
                .get("due_default")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            entries.push(Entry {
                id: super::device::random_id()?,
                time: Utc::now().to_rfc3339(),
                kind: Kind::TaskProperties,
                item_id: format!(
                    "{meeting}/{}",
                    if id.is_empty() {
                        index.to_string()
                    } else {
                        id.into()
                    }
                ),
                label: task
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| {
                        task.get("task")
                            .and_then(Value::as_str)
                            .unwrap_or("Meeting task")
                    })
                    .chars()
                    .take(160)
                    .collect(),
                source_meeting_id: meeting.into(),
                source_offset: task
                    .get("at")
                    .and_then(Value::as_array)
                    .and_then(|at| at.first())
                    .and_then(Value::as_str)
                    .map(str::to_string),
                reason: if guessed {
                    "The meeting summary supplied task details, including a guessed date."
                } else {
                    "The meeting summary supplied these task details."
                }
                .into(),
                confidence: None,
                before: old,
                after,
                undo_of: None,
                target: Target {
                    content: Some(content(task)),
                    file: FileKind::Summary,
                    task_id: (!id.is_empty()).then(|| id.into()),
                    index: Some(index),
                    identity: Some(identity(task)),
                    guards: self.guards(meeting, Some((index, task)))?,
                },
            });
        }
        self.commit(
            Mutation {
                meeting: meeting.into(),
                file: FileKind::Summary,
                before,
                after: summary.clone(),
                create_only,
            },
            entries,
        )
    }
    pub fn publish_projects(&self, meeting: &str, label: &str, projects: &Value) -> io::Result<()> {
        let _lock = self.lock()?;
        super::DeviceSettings::load_in(&self.root)?;
        self.recover_locked()?;
        let path = self.source(meeting, &FileKind::Projects)?;
        let before = self.read(&path)?;
        let old = before
            .clone()
            .unwrap_or_else(|| json!({"meeting_project":null,"tasks":[]}));
        if old == *projects {
            if before.is_none() {
                store_io::write(&path, projects)?;
            }
            return Ok(());
        }
        let entry = Entry {
            id: super::device::random_id()?,
            time: Utc::now().to_rfc3339(),
            kind: Kind::ProjectAssignment,
            item_id: format!("{meeting}/projects"),
            label: format!("Projects for {label}").chars().take(160).collect(),
            source_meeting_id: meeting.into(),
            source_offset: None,
            reason: "The meeting summary suggested projects and which tasks are ours.".into(),
            confidence: None,
            before: old,
            after: projects.clone(),
            undo_of: None,
            target: Target {
                content: self.project_source(meeting)?,
                file: FileKind::Projects,
                task_id: None,
                index: None,
                identity: None,
                guards: self.guards(meeting, None)?,
            },
        };
        self.commit(
            Mutation {
                meeting: meeting.into(),
                file: FileKind::Projects,
                before,
                after: projects.clone(),
                create_only: false,
            },
            vec![entry],
        )
    }
    fn entries_locked(&self) -> io::Result<Vec<Entry>> {
        let mut out = vec![];
        for intent in self.intents()? {
            if self.outcome(&intent)?.is_some_and(|o| o.applied) {
                out.extend(intent.entries);
            }
        }
        Ok(out)
    }
    fn active(entries: &[Entry]) -> BTreeMap<String, bool> {
        let mut active: BTreeMap<_, _> = entries.iter().map(|e| (e.id.clone(), true)).collect();
        for e in entries.iter().rev() {
            if active[&e.id]
                && let Some(parent) = &e.undo_of
            {
                active.insert(parent.clone(), false);
            }
        }
        active
    }
    pub fn history(&self) -> io::Result<Vec<Row>> {
        if !self.dir().exists() {
            return Ok(vec![]);
        }
        let _lock = self.lock()?;
        let entries = self.entries_locked()?;
        let active = Self::active(&entries);
        let mut rows: Vec<_> = entries
            .iter()
            .filter(|e| e.undo_of.is_none())
            .map(|entry| {
                let undone = !active[&entry.id];
                let undo_target = if undone {
                    entries
                        .iter()
                        .rev()
                        .find(|e| e.undo_of.as_deref() == Some(entry.id.as_str()) && active[&e.id])
                        .map(|e| e.id.clone())
                        .unwrap_or_else(|| entry.id.clone())
                } else {
                    entry.id.clone()
                };
                Row {
                    entry: entry.clone(),
                    undo_target,
                    undone,
                }
            })
            .collect();
        rows.reverse();
        Ok(rows)
    }
    pub fn meeting_count(&self, meeting: &str) -> io::Result<usize> {
        Ok(self
            .history()?
            .iter()
            .filter(|row| row.entry.source_meeting_id == meeting && !row.undone)
            .count())
    }
    pub fn undo(&self, id: &str) -> io::Result<Entry> {
        let _lock = self.lock()?;
        self.recover_locked()?;
        let entries = self.entries_locked()?;
        let active = Self::active(&entries);
        let entry = entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| invalid("This settled change could not be found."))?;
        if !active[&entry.id] {
            return Err(invalid("This change has already been undone."));
        }
        let path = self.source(&entry.source_meeting_id, &entry.target.file)?;
        let current = self
            .read(&path)?
            .ok_or_else(|| invalid("This item was deleted. Nothing was changed."))?;
        if !self.guards_match(entry)? {
            return Err(invalid(
                "You changed this item since Sidenote settled it. Nothing was changed.",
            ));
        }
        let mut after = current.clone();
        match entry.kind {
            Kind::TaskProperties => {
                let tasks = after
                    .get_mut("todo")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| invalid("This task was deleted. Nothing was changed."))?;
                let task = if let Some(id) = &entry.target.task_id {
                    tasks
                        .iter_mut()
                        .find(|task| task.get("id").and_then(Value::as_str) == Some(id))
                } else {
                    entry.target.index.and_then(|i| tasks.get_mut(i))
                }
                .ok_or_else(|| invalid("This task was deleted. Nothing was changed."))?;
                if identity(task) != entry.target.identity.clone().unwrap_or_default()
                    || entry
                        .target
                        .content
                        .as_ref()
                        .is_some_and(|before| *before != content(task))
                    || props(task) != entry.after
                {
                    return Err(invalid(
                        "This task changed after Sidenote settled it. Nothing was changed.",
                    ));
                }
                for (key, value) in entry
                    .before
                    .as_object()
                    .ok_or_else(|| invalid("Invalid saved task properties"))?
                {
                    task[key] = value.clone();
                }
            }
            Kind::ProjectAssignment => {
                if current != entry.after {
                    return Err(invalid(
                        "These project suggestions changed. Nothing was changed.",
                    ));
                }
                after = entry.before.clone();
            }
        }
        let mut undo = entry.clone();
        undo.id = super::device::random_id()?;
        undo.time = Utc::now().to_rfc3339();
        undo.undo_of = Some(entry.id.clone());
        undo.reason = "The previous value was restored at your request.".into();
        undo.before = entry.after.clone();
        undo.after = entry.before.clone();
        self.commit(
            Mutation {
                meeting: entry.source_meeting_id.clone(),
                file: entry.target.file.clone(),
                before: Some(current),
                after,
                create_only: false,
            },
            vec![undo.clone()],
        )?;
        Ok(undo)
    }
    pub fn recent(&self) -> io::Result<Vec<Row>> {
        Ok(self
            .history()?
            .into_iter()
            .filter(|row| {
                DateTime::parse_from_rfc3339(&row.entry.time).is_ok_and(|at| {
                    Utc::now().signed_duration_since(at) >= chrono::Duration::zero()
                        && Utc::now().signed_duration_since(at) <= chrono::Duration::days(7)
                })
            })
            .collect())
    }
}
#[cfg(test)]
mod tests;
