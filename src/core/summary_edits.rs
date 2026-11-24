//! Saved edits overlay generated notes.
use super::{
    file_reads::SUMMARY_MAX, meeting_router::inputs::Snapshot, settle, store_io, sync_state,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io, path::Path};

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

impl SnapshotEdit {
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

    pub fn edits(&self) -> io::Result<Vec<Edit>> {
        Ok(sync_state::load::<Records>(self.root, FILE)?
            .edits
            .into_values()
            .filter(|edit| edit.meeting == self.meeting)
            .collect())
    }

    pub fn overlay(&self, summary: &mut Value) -> io::Result<Vec<Edit>> {
        let mut aside = Vec::new();
        for edit in self.edits()? {
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
            }
        }
        Ok(aside)
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

    pub fn restore(&self, field: Field, expected: &Line, version: usize) -> io::Result<()> {
        self.record(field, expected, |edit| {
            let version = edit
                .and_then(|edit| edit.history.get(version))
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
        if !settle::valid_id(self.meeting)
            || self.folder.file_name().and_then(|name| name.to_str()) != Some(self.meeting)
        {
            return Err(stale());
        }
        let Some(_recycle) = super::meeting_delete::claim_write(self.root, self.meeting)? else {
            return Err(stale());
        };
        let _lock = settle::summary_lock(self.root)?;
        let key = format!("{}/{}", self.meeting, field.id());
        sync_state::update_merged_validated(
            self.root,
            FILE,
            |records: &mut Records| {
                let old = records.edits.get(&key);
                let summary = self.generated()?;
                if &Self::line(&summary, field, old)? != expected {
                    return Err(stale());
                }
                let next = snapshot(old)?;
                if next.text == expected.text {
                    return Ok(());
                }
                let edit = records.edits.entry(key.clone()).or_insert_with(|| Edit {
                    meeting: self.meeting.into(),
                    field,
                    base: expected.base.clone(),
                    text: expected.base.clone(),
                    history: vec![SnapshotEdit::original(&expected.base, &summary)],
                });
                if edit.base != expected.base {
                    edit.history
                        .push(SnapshotEdit::original(&expected.base, &summary));
                    edit.base.clone_from(&expected.base);
                }
                edit.text.clone_from(&next.text);
                edit.history.push(next);
                Ok(())
            },
            || {
                if field.read(&self.generated()?).as_ref() != Some(&expected.base) {
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

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("100-200")).unwrap();
        dir
    }

    #[test]
    fn saved_edit_overlays_generated_text_after_reload() {
        let dir = fixture();
        let root = dir.path().join("state");
        let folder = dir.path().join("100-200");
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
        reloaded.restore(Field::Title, &edited, 0).unwrap();
        let edits = reloaded.edits().unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0]
                .history
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["Original title", "My title", "Original title"]
        );
        assert_eq!(
            reloaded.current(Field::Title).unwrap().text,
            "Original title"
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn stale_save_returns_the_edit_to_the_caller() {
        let dir = fixture();
        let root = dir.path().join("state");
        let folder = dir.path().join("100-200");
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
        let dir = fixture();
        let root = dir.path().join("state");
        let folder = dir.path().join("100-200");
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
}
