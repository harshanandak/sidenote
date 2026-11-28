//! Saved edits overlay generated notes.
use super::{
    file_reads::SUMMARY_MAX, meeting_router::inputs::Snapshot, settle, store_io, sync_state,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
};

pub(super) const FILE: &str = "summary-edits.json";
pub(super) const STALE: &str = "This note line changed. Your changes are kept.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Title,
    Context,
    Heading(usize),
    Point(usize, usize),
    Verify(usize),
}

impl Field {
    pub fn id(self) -> String {
        match self {
            Self::Title => "title".into(),
            Self::Context => "context".into(),
            Self::Heading(t) => format!("heading-{t}"),
            Self::Point(t, p) => format!("point-{t}-{p}"),
            Self::Verify(i) => format!("verify-{i}"),
        }
    }

    fn pointer(self) -> String {
        match self {
            Self::Title => "/title".into(),
            Self::Context => "/context".into(),
            Self::Heading(t) => format!("/topics/{t}/title"),
            Self::Point(t, p) => format!("/topics/{t}/points/{p}/text"),
            Self::Verify(i) => format!("/verify/{i}"),
        }
    }

    pub(super) fn read(self, summary: &Value) -> Option<String> {
        let value = summary.pointer(&self.pointer())?;
        if self == Self::Context {
            let parts = value.as_array()?;
            if parts.is_empty() {
                return None;
            }
            return Some(
                parts
                    .iter()
                    .map(Value::as_str)
                    .collect::<Option<Vec<_>>>()?
                    .join(" "),
            );
        }
        value.as_str().map(str::to_owned)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Author {
    You(You),
    Helper { helper: String, model: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum You {
    You,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SnapshotEdit {
    pub text: String,
    pub by: Author,
    pub request: Option<String>,
    pub at: chrono::DateTime<chrono::Utc>,
}

pub(super) fn snapshot_id(value: &Value) -> String {
    let hash = blake3::hash(value.to_string().as_bytes());
    hash.to_hex().to_string()
}

impl SnapshotEdit {
    pub fn id(&self) -> String {
        snapshot_id(&json!(self))
    }

    pub fn by_you(text: &str) -> Self {
        Self {
            text: text.into(),
            by: Author::You(You::You),
            request: None,
            at: chrono::Utc::now(),
        }
    }

    fn original(text: &str, summary: &Value) -> Self {
        let provenance = &summary["made_by"];
        Self {
            text: text.into(),
            by: Author::Helper {
                helper: provenance["helper"]
                    .as_str()
                    .unwrap_or("Generated notes")
                    .into(),
                model: provenance["model"].as_str().unwrap_or_default().into(),
            },
            request: None,
            at: provenance["time"]
                .as_str()
                .and_then(|time| time.parse().ok())
                .unwrap_or_else(chrono::Utc::now),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Edit {
    pub meeting: String,
    pub field: Field,
    pub base: String,
    pub text: String,
    pub history: Vec<SnapshotEdit>,
}

#[derive(Default, Deserialize, Serialize)]
pub(super) struct Records {
    edits: BTreeMap<String, Edit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Line {
    pub base: String,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryVersion {
    AiOriginal,
    UserEdited,
}

#[derive(Clone, Debug)]
pub struct Effective {
    pub value: Value,
    pub version: SummaryVersion,
    pub detached: Vec<Edit>,
    pub warning: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Save {
    pub field: Field,
    pub expected: Line,
    pub text: String,
}

pub struct Store<'a> {
    root: &'a Path,
    folder: &'a Path,
    meeting: &'a str,
}

impl<'a> Store<'a> {
    pub fn new(root: &'a Path, folder: &'a Path, meeting: &'a str) -> Self {
        Self {
            root,
            folder,
            meeting,
        }
    }

    pub fn generated(&self) -> io::Result<Value> {
        let bytes = Snapshot::read_file(&self.folder.join("english-summary.json"), SUMMARY_MAX)?;
        Ok(serde_json::from_slice(
            bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes),
        )?)
    }

    fn edits_report(&self) -> io::Result<(Vec<Edit>, bool)> {
        let (records, warning) = sync_state::load_report::<Records>(self.root, FILE)?;
        Ok((
            records
                .edits
                .into_values()
                .filter(|edit| edit.meeting == self.meeting)
                .collect(),
            warning,
        ))
    }

    pub fn edits(&self) -> io::Result<Vec<Edit>> {
        self.edits_report().map(|(edits, _)| edits)
    }

    pub(super) fn corrections(&self, summary: &Value) -> io::Result<Vec<Edit>> {
        Ok(self
            .edits()?
            .into_iter()
            .filter(|edit| {
                edit.text != edit.base && edit.field.read(summary).as_ref() == Some(&edit.base)
            })
            .collect())
    }

    fn overlay_edits(summary: &mut Value, edits: Vec<Edit>) -> (bool, Vec<Edit>) {
        let mut applied = false;
        let mut aside = Vec::new();
        for edit in edits {
            if edit.field.read(summary).as_ref() != Some(&edit.base) {
                aside.push(edit);
            } else if edit.text != edit.base
                && let Some(value) = summary.pointer_mut(&edit.field.pointer())
            {
                *value = if edit.field == Field::Context {
                    json!([edit.text])
                } else {
                    json!(edit.text)
                };
                applied = true;
            }
        }
        (applied, aside)
    }

    pub fn effective(&self, generated: &Value) -> io::Result<Effective> {
        let (edits, warning) = self.edits_report()?;
        let mut value = generated.clone();
        let (applied, detached) = Self::overlay_edits(&mut value, edits);
        Ok(Effective {
            value,
            version: if applied {
                SummaryVersion::UserEdited
            } else {
                SummaryVersion::AiOriginal
            },
            detached,
            warning,
        })
    }

    pub fn overlay(&self, summary: &mut Value) -> io::Result<Vec<Edit>> {
        let (edits, _) = self.edits_report()?;
        Ok(Self::overlay_edits(summary, edits).1)
    }

    pub fn current(&self, field: Field) -> io::Result<Line> {
        Self::line(
            &self.generated()?,
            field,
            self.edits()?.iter().find(|edit| edit.field == field),
        )
    }

    fn line(summary: &Value, field: Field, edit: Option<&Edit>) -> io::Result<Line> {
        let base = field.read(summary).ok_or_else(stale)?;
        let text = edit
            .filter(|edit| edit.base == base)
            .map_or_else(|| base.clone(), |edit| edit.text.clone());
        Ok(Line { base, text })
    }

    /// Keep the caller's draft on a stale result so it can be compared with current().
    pub fn save(&self, field: Field, expected: &Line, text: &str) -> io::Result<()> {
        self.record(field, expected, |_| Ok(SnapshotEdit::by_you(text)))
    }

    pub fn save_many(&self, saves: &[Save]) -> io::Result<()> {
        self.update(|records, summary| {
            let mut keys = BTreeSet::new();
            for save in saves {
                if !keys.insert(save.field.id()) {
                    return Err(stale());
                }
                let key = Self::key(self.meeting, save.field);
                if Self::line(summary, save.field, records.edits.get(&key))? != save.expected {
                    return Err(stale());
                }
            }
            for save in saves {
                if save.text == save.expected.text {
                    continue;
                }
                Self::write(
                    records,
                    self.meeting,
                    save.field,
                    &save.expected,
                    SnapshotEdit::by_you(&save.text),
                    summary,
                );
            }
            Ok(())
        })
    }

    pub fn restore(&self, field: Field, expected: &Line, version: &str) -> io::Result<()> {
        self.record(field, expected, |edit| {
            let version = edit
                .and_then(|edit| {
                    edit.history
                        .iter()
                        .find(|snapshot| snapshot.id() == version)
                })
                .ok_or_else(stale)?;
            Ok(SnapshotEdit::by_you(&version.text))
        })
    }

    pub(super) fn record(
        &self,
        field: Field,
        expected: &Line,
        snapshot: impl Fn(Option<&Edit>) -> io::Result<SnapshotEdit>,
    ) -> io::Result<()> {
        self.update(|records, summary| {
            let key = Self::key(self.meeting, field);
            let old = records.edits.get(&key);
            if &Self::line(summary, field, old)? != expected {
                return Err(stale());
            }
            let next = snapshot(old)?;
            if next.text == expected.text {
                return Ok(());
            }
            Self::write(records, self.meeting, field, expected, next, summary);
            Ok(())
        })
    }

    fn key(meeting: &str, field: Field) -> String {
        format!("{meeting}/{}", field.id())
    }

    fn write(
        records: &mut Records,
        meeting: &str,
        field: Field,
        expected: &Line,
        next: SnapshotEdit,
        summary: &Value,
    ) {
        let edit = records
            .edits
            .entry(Self::key(meeting, field))
            .or_insert_with(|| Edit {
                meeting: meeting.into(),
                field,
                base: expected.base.clone(),
                text: expected.base.clone(),
                history: vec![SnapshotEdit::original(&expected.base, summary)],
            });
        if edit.base != expected.base {
            edit.history
                .push(SnapshotEdit::original(&expected.base, summary));
            edit.base.clone_from(&expected.base);
        }
        edit.text.clone_from(&next.text);
        edit.history.push(next);
    }

    fn update(
        &self,
        mut change: impl FnMut(&mut Records, &Value) -> io::Result<()>,
    ) -> io::Result<()> {
        if !settle::valid_id(self.meeting)
            || self.folder.file_name().and_then(|name| name.to_str()) != Some(self.meeting)
        {
            return Err(stale());
        }
        let Some(_recycle) = super::meeting_delete::claim_write(self.root, self.meeting)? else {
            return Err(stale());
        };
        let _lock = settle::summary_lock(self.root)?;
        let revision = std::cell::RefCell::new(String::new());
        sync_state::update_merged_validated(
            self.root,
            FILE,
            |records: &mut Records| {
                let summary = self.generated()?;
                *revision.borrow_mut() = snapshot_id(&summary);
                change(records, &summary)
            },
            || {
                if snapshot_id(&self.generated()?) != revision.borrow().as_str() {
                    return Err(stale());
                }
                Ok(())
            },
        )?;
        Ok(())
    }
}

fn stale() -> io::Error {
    store_io::invalid(STALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let (root, folder) = (dir.path().join("state"), dir.path().join("100-200"));
        std::fs::create_dir(&folder).unwrap();
        (dir, root, folder)
    }

    fn copy_journal(from: &Path, to: &Path) {
        let device = super::super::DeviceSettings::load_in(from).unwrap();
        let key = blake3::hash(FILE.as_bytes()).to_string();
        let path = Path::new("state").join(device.id).join(key);
        std::fs::create_dir_all(to.join(&path)).unwrap();
        for entry in std::fs::read_dir(from.join(&path)).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), to.join(&path).join(entry.file_name())).unwrap();
        }
    }

    fn write_summary(folder: &Path, value: Value) -> Vec<u8> {
        let bytes = value.to_string().into_bytes();
        std::fs::write(folder.join("english-summary.json"), &bytes).unwrap();
        bytes
    }

    fn draft(field: Field, expected: Line, text: &str) -> Save {
        Save {
            field,
            expected,
            text: text.into(),
        }
    }

    #[test]
    fn saved_edit_overlays_generated_text_after_reload() {
        let (_dir, root, folder) = fixture();
        let original = br#"{"title":"Original title","topics":[]}"#;
        let path = folder.join("english-summary.json");
        std::fs::write(&path, original).unwrap();
        let store = Store::new(&root, &folder, "100-200");
        let before = store.current(Field::Title).unwrap();
        store.save(Field::Title, &before, "My title").unwrap();
        let reloaded = Store::new(&root, &folder, "100-200");
        let edited = reloaded.current(Field::Title).unwrap();
        assert_eq!(edited.text, "My title");
        assert!(store.save(Field::Title, &before, "Stale draft").is_err());
        let id = reloaded.edits().unwrap()[0].history[0].id();
        reloaded.restore(Field::Title, &edited, &id).unwrap();
        let edits = reloaded.edits().unwrap();
        assert_eq!(edits.len(), 1);
        let history = &edits[0].history;
        let texts: Vec<_> = history.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["Original title", "My title", "Original title"]);
        let displayed = reloaded.current(Field::Title).unwrap();
        assert_eq!(displayed.text, "Original title");

        let peer = root.with_file_name("peer");
        copy_journal(&root, &peer);
        sync_state::update_merged(&peer, FILE, |records: &mut Records| {
            records.edits.values_mut().next().unwrap().history[1].text = "Peer title".into();
            Ok(())
        })
        .unwrap();
        copy_journal(&peer, &root);
        assert_eq!(reloaded.current(Field::Title).unwrap(), displayed);
        let displaced = reloaded.edits().unwrap();
        assert_eq!(displaced[0].history[1].text, "Peer title");
        let selected = &history[1];
        let result = reloaded.restore(Field::Title, &displayed, &selected.id());
        let expected = if result.is_ok() {
            &selected.text
        } else {
            &displayed.text
        };
        assert_eq!(&reloaded.current(Field::Title).unwrap().text, expected);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn concurrent_device_edits_keep_both_versions_for_restore() {
        let (dir, _, folder) = fixture();
        std::fs::write(
            folder.join("english-summary.json"),
            br#"{"title":"Original title","made_by":{"time":"2026-11-22T00:00:00Z"}}"#,
        )
        .unwrap();
        let a_root = dir.path().join("device-a");
        let b_root = dir.path().join("device-b");
        let a = Store::new(&a_root, &folder, "100-200");
        let b = Store::new(&b_root, &folder, "100-200");
        let a_before = a.current(Field::Title).unwrap();
        let b_before = b.current(Field::Title).unwrap();
        a.save(Field::Title, &a_before, "Device A title").unwrap();
        b.save(Field::Title, &b_before, "Device B title").unwrap();
        let a_local = a.current(Field::Title).unwrap();
        let b_local = b.current(Field::Title).unwrap();

        copy_journal(&a_root, &b_root);
        copy_journal(&b_root, &a_root);
        let winner = a.current(Field::Title).unwrap();
        assert_eq!(winner, b.current(Field::Title).unwrap());
        let (loser, stale_draft) = if winner == a_local {
            (&b, &b_local)
        } else {
            assert_eq!(winner, b_local);
            (&a, &a_local)
        };
        assert!(
            loser
                .save(Field::Title, stale_draft, "Stale draft")
                .is_err()
        );
        for store in [a, b] {
            let edits = store.edits().unwrap();
            let history = &edits[0].history;
            assert_eq!(history[0].text, "Original title");
            for text in ["Device A title", "Device B title"] {
                let version = history.iter().find(|snapshot| snapshot.text == text);
                assert!(version.is_some(), "missing saved version {text}");
                store
                    .restore(
                        Field::Title,
                        &store.current(Field::Title).unwrap(),
                        &version.unwrap().id(),
                    )
                    .unwrap();
                assert_eq!(store.current(Field::Title).unwrap().text, text);
            }
        }
    }

    #[test]
    fn restore_keeps_the_selected_version_when_delayed_history_moves_its_index() {
        let (dir, _, folder) = fixture();
        std::fs::write(
            folder.join("english-summary.json"),
            br#"{"title":"Original title","made_by":{"time":"2026-11-22T00:00:00Z"}}"#,
        )
        .unwrap();
        let roots = ["one", "two", "three"].map(|name| dir.path().join(name));
        let stores = roots
            .each_ref()
            .map(|root| Store::new(root, &folder, "100-200"));
        let observer = dir.path().join("observer");
        for (i, store) in stores.iter().enumerate() {
            let before = store.current(Field::Title).unwrap();
            store.save(Field::Title, &before, &i.to_string()).unwrap();
            copy_journal(&roots[i], &observer);
        }
        let winner = Store::new(&observer, &folder, "100-200")
            .current(Field::Title)
            .unwrap();
        let winning_index = winner.text.parse::<usize>().unwrap();
        let store = &stores[winning_index];
        let mut losing = (0..stores.len())
            .filter(|&i| i != winning_index)
            .map(|i| (stores[i].edits().unwrap()[0].history[1].id(), i))
            .collect::<Vec<_>>();
        losing.sort();
        let (selected, latest) = &losing[1];
        assert!(store.restore(Field::Title, &winner, selected).is_err());
        copy_journal(&roots[*latest], &roots[winning_index]);
        let displayed = store.edits().unwrap();
        assert_eq!(displayed[0].history[2].id(), *selected);
        copy_journal(&roots[losing[0].1], &roots[winning_index]);
        assert_eq!(store.edits().unwrap()[0].history[2].id(), losing[0].0);
        store.restore(Field::Title, &winner, selected).unwrap();
        assert_eq!(
            store.current(Field::Title).unwrap().text,
            latest.to_string()
        );
    }

    #[test]
    fn stale_save_returns_the_edit_to_the_caller() {
        let (_dir, root, folder) = fixture();
        let path = folder.join("english-summary.json");
        std::fs::write(&path, br#"{"title":"Original title"}"#).unwrap();
        let store = Store::new(&root, &folder, "100-200");
        let before = store.current(Field::Title).unwrap();
        std::fs::write(&path, br#"{"title":"New title"}"#).unwrap();
        let draft = "My title";
        assert!(store.save(Field::Title, &before, draft).is_err());
        assert_eq!(draft, "My title");
        assert_eq!(store.current(Field::Title).unwrap().text, "New title");
    }

    #[test]
    fn changed_generated_line_keeps_the_edit_aside() {
        let (_dir, root, folder) = fixture();
        let path = folder.join("english-summary.json");
        std::fs::write(&path, br#"{"title":"Original title"}"#).unwrap();
        let store = Store::new(&root, &folder, "100-200");
        let before = store.current(Field::Title).unwrap();
        store.save(Field::Title, &before, "My title").unwrap();
        std::fs::write(&path, br#"{"title":"Regenerated title"}"#).unwrap();
        let mut generated = store.generated().unwrap();
        let kept = store.overlay(&mut generated).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(generated["title"], "Regenerated title");
    }

    #[test]
    fn effective_view_loads_missing_and_legacy_stores() {
        let (_dir, root, folder) = fixture();
        let generated = json!({
            "title": "Original title",
            "context": [],
            "topics": [],
            "custom": {"keep": true}
        });
        write_summary(&folder, generated.clone());
        let store = Store::new(&root, &folder, "100-200");
        let missing = store.effective(&generated).unwrap();
        assert_eq!(missing.value, generated);
        assert_eq!(missing.version, SummaryVersion::AiOriginal);
        assert!(missing.detached.is_empty());
        assert!(!missing.warning);

        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(FILE),
            br#"{"edits":{"100-200/title":{"meeting":"100-200","field":"title","base":"Original title","text":"Legacy title","history":[{"text":"Legacy title","by":"you","request":null,"at":"2026-11-22T00:00:00Z"}]}}}"#,
        )
        .unwrap();
        let legacy = store.effective(&generated).unwrap();
        assert_eq!(legacy.value["title"], "Legacy title");
        assert_eq!(legacy.value["custom"], json!({"keep": true}));
        assert_eq!(legacy.version, SummaryVersion::UserEdited);
        assert!(!legacy.warning);
    }

    #[test]
    fn context_edit_joins_generated_parts() {
        let (_dir, root, folder) = fixture();
        let generated = json!({"context": ["First part.", "Second part."], "topics": []});
        write_summary(&folder, generated.clone());
        let store = Store::new(&root, &folder, "100-200");
        let before = store.current(Field::Context).unwrap();
        assert_eq!(before.base, "First part. Second part.");
        store
            .save_many(&[draft(Field::Context, before, "Joined context")])
            .unwrap();
        let effective = store.effective(&generated).unwrap();
        assert_eq!(effective.value["context"], json!(["Joined context"]));
        assert_eq!(effective.version, SummaryVersion::UserEdited);
    }

    #[test]
    fn stale_batch_save_changes_nothing_and_preserves_generated_bytes() {
        let (_dir, root, folder) = fixture();
        let original = json!({"title": "Original title", "context": ["Original context"]});
        write_summary(&folder, original);
        let store = Store::new(&root, &folder, "100-200");
        let title = store.current(Field::Title).unwrap();
        let context = store.current(Field::Context).unwrap();
        let changed = json!({"title": "Original title", "context": ["New context"]});
        let path = folder.join("english-summary.json");
        let changed_bytes = write_summary(&folder, changed);
        let saves = [
            draft(Field::Title, title, "Batch title"),
            draft(Field::Context, context, "Batch context"),
        ];
        assert!(store.save_many(&saves).is_err());
        assert!(store.edits().unwrap().is_empty());
        assert_eq!(store.current(Field::Title).unwrap().text, "Original title");
        assert_eq!(std::fs::read(&path).unwrap(), changed_bytes);
    }

    #[test]
    fn equal_to_base_edit_is_ai_original() {
        let (_dir, root, folder) = fixture();
        let generated = json!({"title": "Original title"});
        write_summary(&folder, generated.clone());
        let store = Store::new(&root, &folder, "100-200");
        let before = store.current(Field::Title).unwrap();
        store.save(Field::Title, &before, "My title").unwrap();
        let edited = store.current(Field::Title).unwrap();
        store.save(Field::Title, &edited, "Original title").unwrap();
        let effective = store.effective(&generated).unwrap();
        assert_eq!(effective.version, SummaryVersion::AiOriginal);
        assert_eq!(effective.value["title"], "Original title");
    }

    #[test]
    fn detached_edit_is_reported_without_marking_summary_edited() {
        let (_dir, root, folder) = fixture();
        write_summary(&folder, json!({"title": "Original title"}));
        let store = Store::new(&root, &folder, "100-200");
        let before = store.current(Field::Title).unwrap();
        store.save(Field::Title, &before, "My title").unwrap();
        let regenerated = json!({"title": "Regenerated title"});
        write_summary(&folder, regenerated.clone());
        let effective = store.effective(&regenerated).unwrap();
        assert_eq!(effective.detached.len(), 1);
        assert_eq!(effective.detached[0].text, "My title");
        assert_eq!(effective.value["title"], "Regenerated title");
        assert_eq!(effective.version, SummaryVersion::AiOriginal);
        assert!(!effective.warning);
    }

    #[test]
    fn save_many_rejects_duplicate_fields() {
        let (_dir, root, folder) = fixture();
        write_summary(&folder, json!({"title": "Original title"}));
        let store = Store::new(&root, &folder, "100-200");
        let before = store.current(Field::Title).unwrap();
        let save = draft(Field::Title, before, "Batch title");
        assert!(store.save_many(&[save.clone(), save]).is_err());
        assert!(store.edits().unwrap().is_empty());
    }

    #[test]
    fn corrupt_journal_is_reported_as_a_warning() {
        let (_dir, root, folder) = fixture();
        let generated = json!({"title": "Original title"});
        write_summary(&folder, generated.clone());
        let store = Store::new(&root, &folder, "100-200");
        let before = store.current(Field::Title).unwrap();
        store.save(Field::Title, &before, "My title").unwrap();
        let device = super::super::DeviceSettings::load_in(&root).unwrap().id;
        let key = blake3::hash(FILE.as_bytes()).to_string();
        let journal_dir = root.join("state").join(device).join(key);
        let journal = std::fs::read_dir(&journal_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::write(journal, b"corrupt").unwrap();
        let effective = store.effective(&generated).unwrap();
        assert!(effective.warning);
    }
}
