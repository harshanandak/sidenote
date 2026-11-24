//! Immutable, device-owned changes. Readers resolve LWW registers without rewriting any input.
use super::{
    DeviceSettings, device,
    store_io::{self, invalid},
};
use chrono::{DateTime, NaiveDate};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

static LOCAL_MUTATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn mutation_generation() -> u64 {
    LOCAL_MUTATIONS.load(std::sync::atomic::Ordering::Acquire)
}
// A successful no-op may adopt peer fields in the UI without publishing a transaction.
fn fence_local_mutation() -> u64 {
    LOCAL_MUTATIONS.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1
}

type Key = Vec<String>;
type Items = BTreeMap<Key, Value>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    key: Key,
    at: u64,
    // None is a durable tombstone, never an absent change.
    value: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    device: String,
    id: String,
    store: String,
    changes: Vec<Change>,
    sources: BTreeSet<String>,
    checksum: String,
}
impl Journal {
    fn bytes(&self) -> io::Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(invalid(
                "This change exceeds the 4 MB journal limit. Save smaller groups.",
            ));
        }
        Ok(bytes)
    }
    fn checksum(&self) -> io::Result<String> {
        Ok(hash(&serde_json::to_vec(&(
            self.version,
            &self.device,
            &self.id,
            &self.store,
            &self.changes,
            &self.sources,
        ))?))
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Register {
    at: u64,
    device: String,
    id: String,
    value: Option<Value>,
}
impl Register {
    fn version(&self) -> (u64, &str, &str) {
        (self.at, &self.device, &self.id)
    }
}
#[derive(Default)]
struct View {
    items: BTreeMap<Key, Register>,
    summary_history: BTreeMap<Key, BTreeMap<String, Value>>,
    sources: BTreeSet<String>,
    warnings: Vec<String>,
    own_bad: bool,
}
impl View {
    fn merge(&mut self, journal: &Journal) {
        self.sources.extend(
            journal
                .sources
                .iter()
                .map(|s| format!("{}:{s}", journal.device)),
        );
        for change in &journal.changes {
            // A losing text register can still hold versions absent from the winner.
            if journal.store == super::summary_edits::FILE
                && let [kind, _] = change.key.as_slice()
                && kind == "edits"
                && let Some(value) = &change.value
                && super::summary_edits::Edit::deserialize(value).is_ok()
                && let Some(history) = value["history"].as_array()
            {
                let snapshots = self.summary_history.entry(change.key.clone()).or_default();
                for snapshot in history {
                    snapshots
                        .entry(super::summary_edits::snapshot_id(snapshot))
                        .or_insert_with(|| snapshot.clone());
                }
            }
            let incoming = Register {
                at: change.at,
                device: journal.device.clone(),
                id: journal.id.clone(),
                value: change.value.clone(),
            };
            if self
                .items
                .get(&change.key)
                .is_none_or(|old| incoming.version() > old.version())
            {
                self.items.insert(change.key.clone(), incoming);
            }
        }
    }
    fn values(&self) -> Items {
        self.items
            .iter()
            .filter_map(|(key, register)| {
                let mut value = register.value.clone()?;
                if let Some(snapshots) = self.summary_history.get(key)
                    && let Some(history) = value.get_mut("history").and_then(Value::as_array_mut)
                {
                    let present: BTreeSet<_> = history
                        .iter()
                        .map(super::summary_edits::snapshot_id)
                        .collect();
                    history.extend(
                        snapshots
                            .iter()
                            .filter(|(id, _)| !present.contains(*id))
                            .map(|(_, snapshot)| snapshot.clone()),
                    );
                }
                Some((key.clone(), value))
            })
            .collect()
    }
    fn next_time(&self) -> io::Result<u64> {
        let now = chrono::Utc::now()
            .timestamp_nanos_opt()
            .ok_or_else(|| invalid("Invalid state clock"))?;
        let now = u64::try_from(now).map_err(|_| invalid("Invalid state clock"))? | (1 << 63);
        let last = self.items.values().map(|r| r.at).max().unwrap_or(0);
        Ok(now.max(
            last.checked_add(1)
                .ok_or_else(|| invalid("State clock exhausted"))?,
        ))
    }
}
fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
fn absent(error: io::Error) -> io::Result<Option<std::sync::Arc<[u8]>>> {
    if error.kind() == io::ErrorKind::NotFound {
        Ok(None)
    } else {
        Err(error)
    }
}
fn read_optional(path: &Path) -> io::Result<Option<std::sync::Arc<[u8]>>> {
    store_io::read_bounded(path, super::file_reads::SUMMARY_MAX)
        .map(|bytes| Some(std::sync::Arc::from(bytes)))
        .or_else(absent)
}
fn parse(bytes: &[u8]) -> io::Result<Value> {
    Ok(serde_json::from_slice(
        bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes),
    )?)
}
fn meeting_store(store: &str) -> bool {
    let Some(stem) = store.strip_suffix(".json") else {
        return false;
    };
    stem == "all"
        || super::persistence::is_live_ask_key(stem)
        || super::persistence::is_company_ask_key(stem)
        || stem.split_once('-').is_some_and(|(a, b)| {
            !a.is_empty() && !b.is_empty() && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
        })
}
fn report_store(store: &str) -> bool {
    store
        .strip_prefix("checks/")
        .and_then(|s| s.strip_suffix(".json"))
        .is_some_and(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok_and(|d| d.to_string() == s))
}
fn supported(store: &str) -> bool {
    meeting_store(store)
        || report_store(store)
        || matches!(
            store,
            "projects.json"
                | "companies.json"
                | "task-overrides.json"
                | super::summary_edits::FILE
                | "tasks-settings.json"
                | "agent-instructions.json"
                | "memory/records.json"
                | "memory/legacy.json"
                | "checks/triage.json"
                | "checks/state.json"
        )
}
fn read_journal(path: &Path, owner: &str, store: Option<&str>) -> io::Result<Journal> {
    let bytes = store_io::read_bounded(path, super::file_reads::JOURNAL_MAX)?;
    let j: Journal = serde_json::from_slice(&bytes)?;
    if j.version != 2
        || j.device != owner
        || !device::valid_id(owner)
        || !device::valid_id(&j.id)
        || !supported(&j.store)
        || store.is_some_and(|s| s != j.store)
        || j.checksum != j.checksum()?
    {
        return Err(invalid("Invalid device journal, schema or checksum"));
    }
    for change in &j.changes {
        if change.key.is_empty() || change.key.len() > 3 {
            return Err(invalid("Invalid state item key"));
        }
    }
    Ok(j)
}
fn entries(path: &Path) -> io::Result<Vec<std::sync::Arc<fs::DirEntry>>> {
    store_io::reject_reparse(path)?;
    super::file_reads::entries(path).map_err(|e| {
        if matches!(e.raw_os_error(), Some(32 | 33)) {
            e
        } else {
            invalid("Could not read the saved data")
        }
    })
}
fn read_view(root: &Path, store: &str, own: &str) -> io::Result<View> {
    let mut view = View::default();
    let mut files = Vec::new();
    let mut total = 0u64;
    for path in legacy_sources(root, store)? {
        if let Ok(meta) = super::file_reads::checked_metadata(&path) {
            total = total
                .checked_add(meta.len())
                .filter(|n| *n <= super::file_reads::STORE_MAX)
                .ok_or_else(|| invalid("Store byte limit exceeded"))?;
        }
    }
    for owner in entries(&root.join("state"))? {
        let name = owner.file_name().to_string_lossy().into_owned();
        if !device::valid_id(&name) || !super::file_reads::directory(&owner.path()) {
            continue;
        }
        for entry in entries(&owner.path().join(hash(store.as_bytes())))? {
            if entry.path().extension().is_none_or(|e| e != "json") {
                continue;
            }
            if let Ok(meta) = super::file_reads::checked_metadata(&entry.path()) {
                total = total
                    .checked_add(meta.len())
                    .filter(|n| *n <= super::file_reads::STORE_MAX)
                    .ok_or_else(|| invalid("Store byte limit exceeded"))?;
            }
            files.push((entry.path(), name.clone()));
        }
    }
    for (path, name) in files {
        match read_journal(&path, &name, Some(store)) {
            Ok(j) => view.merge(&j),
            Err(e) => {
                super::file_reads::error(&path, &e);
                view.own_bad |= name == own;
                view.warnings.push(format!("Unreadable or incomplete device state for {store}; original files were retained"));
            }
        }
    }
    Ok(view)
}

struct Session {
    root: PathBuf,
    store: String,
    own: String,
    _lock: Option<fs::File>,
    _relay_mode_lock: Option<fs::File>,
    preview: Option<std::sync::Arc<super::relay_preview::Preview>>,
    read_only: bool,
    view: View,
}
impl Session {
    fn open(root: &Path, store: &str) -> io::Result<Self> {
        Self::open_mode(root, store, false)
    }
    fn open_mode(root: &Path, store: &str, read_only: bool) -> io::Result<Self> {
        Self::open_mode_preview(root, store, read_only, true)
    }
    fn open_mode_preview(
        root: &Path,
        store: &str,
        read_only: bool,
        use_preview: bool,
    ) -> io::Result<Self> {
        if !supported(store) {
            return Err(invalid("Unsupported shared store"));
        }
        let own = if read_only {
            DeviceSettings::read_in(root)?
        } else {
            DeviceSettings::load_in(root)?
        }
        .id;
        // This lock is machine-local. No peer lock is ever opened or synchronized.
        let lock = if read_only {
            None
        } else {
            Some(store_io::lock(
                &root
                    .join("local/locks")
                    .join(format!("{}.lock", hash(store.as_bytes()))),
            )?)
        };
        let view = read_view(root, store, &own)?;
        let relay_mode_lock = if matches!(super::CHANNEL, super::Channel::Dev)
            && !read_only
            && super::relay_preview::shared::domain(store).is_some()
            && root.join("local/relay/settings.json").try_exists()?
        {
            Some(store_io::lock(&root.join("local/relay/mode.lock"))?)
        } else {
            None
        };
        let preview = if use_preview && super::relay_preview::shared::domain(store).is_some() {
            super::relay_preview::get(root)?
        } else {
            None
        };
        let mut session = Self {
            root: root.to_owned(),
            store: store.into(),
            own,
            _lock: lock,
            _relay_mode_lock: relay_mode_lock,
            preview,
            read_only,
            view,
        };
        if session.view.own_bad && (!read_only || super::file_reads::refreshing()) {
            return Err(invalid(
                "Own journal is unreadable; refusing to edit or import",
            ));
        }
        if !super::file_reads::refreshing() || !use_preview {
            session.import_legacy()?;
        }
        Ok(session)
    }
    fn publish(&mut self, changes: Vec<Change>, sources: BTreeSet<String>) -> io::Result<()> {
        if self.read_only {
            return Err(invalid("Cannot publish a read-only state view"));
        }
        if changes.is_empty() && sources.is_empty() {
            return Ok(());
        }
        let mut j = Journal {
            version: 2,
            device: self.own.clone(),
            id: device::random_id()?,
            store: self.store.clone(),
            changes,
            sources,
            checksum: String::new(),
        };
        j.checksum = j.checksum()?;
        let path = self
            .root
            .join("state")
            .join(&self.own)
            .join(hash(self.store.as_bytes()))
            .join(format!("{}.json", j.id));
        let bytes = j.bytes()?;
        store_io::publish(&path, &bytes)?;
        self.view.merge(&j);
        fence_local_mutation();
        Ok(())
    }
    fn import_legacy(&mut self) -> io::Result<()> {
        for path in legacy_sources(&self.root, &self.store)? {
            let bytes = match read_optional(&path) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => continue,
                Err(_) => {
                    self.view
                        .warnings
                        .push("Could not read a legacy state file; it was retained".into());
                    continue;
                }
            };
            let relative = path
                .strip_prefix(&self.root)
                .map_err(|_| invalid("Invalid migration source"))?;
            let source = hash(&serde_json::to_vec(&(
                relative.to_string_lossy().replace('\\', "/"),
                &bytes,
            ))?);
            if self
                .view
                .sources
                .contains(&format!("{}:{source}", self.own))
            {
                continue;
            }
            let values = (|| {
                let value = if self.store == "memory/legacy.json" {
                    let text = std::str::from_utf8(&bytes)
                        .map_err(|_| invalid("Legacy memory is not UTF-8"))?;
                    json!({"documents": { hash(text.as_bytes()): text }})
                } else {
                    let value = parse(&bytes)?;
                    validate_legacy(&self.store, &value)?;
                    value
                };
                legacy_items(
                    &self.store,
                    value,
                    &format!("{}:{}", self.own, relative.to_string_lossy()),
                )
            })();
            let values = match values {
                Ok(values) => values,
                Err(_) => {
                    self.view
                        .warnings
                        .push("Could not read a legacy state file; it was retained".into());
                    continue;
                }
            };
            // Legacy snapshots seed the same registers in both read and migration modes.
            let changes = values
                .into_iter()
                .map(|(key, value)| Change {
                    key,
                    at: legacy_time(&value),
                    value: Some(value),
                })
                .collect();
            let mut journal = Journal {
                version: 2,
                device: self.own.clone(),
                id: source[..32].into(),
                store: self.store.clone(),
                checksum: String::new(),
                sources: BTreeSet::from([source.clone()]),
                changes,
            };
            journal.checksum = journal.checksum()?;
            // Check the exact publication size before backup; generated IDs have this same length.
            if journal.bytes().is_err() {
                self.view.warnings.push("Could not import a legacy state file within the journal limit; it was retained".into());
                continue;
            }
            if self.read_only || self.preview.is_some() {
                // Apply exactly the same seed registers in memory without migration or backups.
                journal.sources.clear();
                self.view.merge(&journal);
                continue;
            }
            let backup = self
                .root
                .join("backups/sync-migration")
                .join(&self.own)
                .join(&source)
                .join(relative);
            match read_optional(&backup)? {
                Some(old) if old != bytes => {
                    return Err(invalid("Migration backup differs; refusing replacement"));
                }
                Some(_) => {}
                None => store_io::publish(&backup, &bytes)?,
            }
            super::area_ops::proof_boundary(&self.root, "legacy-backup")?;
            let verified = store_io::read_bounded(&backup, 16 * 1024 * 1024)?;
            if hash(&verified) != hash(&bytes) {
                return Err(invalid("Migration backup verification failed"));
            }
            self.publish(journal.changes, journal.sources)?;
        }
        Ok(())
    }
    fn value<T: Serialize + DeserializeOwned + Default>(&self) -> io::Result<Value> {
        self.value_report::<T>().map(|(value, _)| value)
    }
    fn value_report<T: Serialize + DeserializeOwned + Default>(
        &self,
    ) -> io::Result<(Value, usize)> {
        let mut skipped = 0;
        let default = serde_json::to_value(T::default())?;
        let mut valid = Items::new();
        for (key, value) in self.view.values() {
            let single = inflate(
                &self.store,
                BTreeMap::from([(key.clone(), value.clone())]),
                default.clone(),
            );
            if single
                .as_ref()
                .is_ok_and(|v| serde_json::from_value::<T>(v.clone()).is_ok())
            {
                valid.insert(key, value);
            } else {
                skipped += 1;
            }
        }
        let mut value = inflate(&self.store, valid, default)?;
        if let Some(preview) = &self.preview {
            let shared = preview.shared_value(
                &self.store,
                super::relay_preview::shared::selected(&self.store, value.clone()),
            )?;
            if let (Some(target), Some(fields)) = (value.as_object_mut(), shared.as_object()) {
                target.extend(fields.clone());
            }
            if self.store == "task-overrides.json" {
                value["manual"] = serde_json::to_value(preview.tasks()?)?;
            }
        }
        Ok((value, skipped))
    }

    fn diff(&mut self, before: Items, after: Items) -> io::Result<()> {
        // Refresh the clock, without copying unrelated incoming values into this device's edits.
        let latest = read_view(&self.root, &self.store, &self.own)?;
        if latest.own_bad {
            return Err(invalid("Own journal changed or is unreadable"));
        }
        let at = latest.next_time()?;
        self.publish_diff(before, after, at)
    }
    fn publish_diff(&mut self, mut before: Items, mut after: Items, at: u64) -> io::Result<()> {
        let keys: BTreeSet<_> = before.keys().chain(after.keys()).cloned().collect();
        let edits: Vec<_> = keys
            .iter()
            .filter(|k| before.get(*k) != after.get(*k))
            .map(|k| (k.clone(), before.get(k).cloned(), after.get(k).cloned()))
            .collect();
        if super::relay_preview::shared::domain(&self.store).is_some()
            && super::relay_preview::Settings::load(&self.root)?.enabled != self.preview.is_some()
        {
            return Err(invalid("Relay preview mode changed. Reload before saving."));
        }
        if self.store == "task-overrides.json"
            && let Some(preview) = &self.preview
        {
            super::area_version::guard_changes(&self.root, &self.store, &edits)?;
            preview.edit_overrides(
                &inflate(
                    &self.store,
                    before,
                    serde_json::to_value(super::Overrides::default())?,
                )?,
                &inflate(
                    &self.store,
                    after,
                    serde_json::to_value(super::Overrides::default())?,
                )?,
            )?;
            fence_local_mutation();
            return Ok(());
        }
        super::area_version::guard_changes(&self.root, &self.store, &edits)?;
        if let Some(preview) = &self.preview {
            let selected = |key: &Key| {
                self.store != "task-overrides.json" || key.first().is_none_or(|k| k != "manual")
            };
            let keys: BTreeSet<_> = before
                .keys()
                .chain(after.keys())
                .filter(|k| selected(k))
                .cloned()
                .collect();
            let mut relay_before = Items::new();
            let mut relay_after = Items::new();
            for key in keys {
                if self.store != "task-overrides.json"
                    && super::relay_preview::shared::domain(&self.store)
                        .is_some_and(|d| d.starts_with("meeting:"))
                    && key.first().is_none_or(|k| k != "checked")
                {
                    continue;
                }
                if let Some(value) = before.remove(&key) {
                    relay_before.insert(key.clone(), value);
                }
                if let Some(value) = after.remove(&key) {
                    relay_after.insert(key, value);
                }
            }
            if relay_before != relay_after {
                preview.shared_edit(
                    &self.store,
                    &inflate(
                        &self.store,
                        relay_before,
                        super::relay_preview::shared::defaults(&self.store)?,
                    )?,
                    &inflate(
                        &self.store,
                        relay_after,
                        super::relay_preview::shared::defaults(&self.store)?,
                    )?,
                )?;
                fence_local_mutation();
            }
        }
        let keys: BTreeSet<_> = before.keys().chain(after.keys()).cloned().collect();
        let changes = keys
            .into_iter()
            .filter(|k| before.get(k) != after.get(k))
            .map(|key| Change {
                at,
                value: after.get(&key).cloned(),
                key,
            })
            .collect();
        self.publish(changes, BTreeSet::new())
    }
}
pub(super) fn load<T: Serialize + DeserializeOwned + Default>(
    root: &Path,
    store: &str,
) -> io::Result<T> {
    load_report(root, store).map(|(value, _)| value)
}
pub(super) fn load_report<T: Serialize + DeserializeOwned + Default>(
    root: &Path,
    store: &str,
) -> io::Result<(T, bool)> {
    let session = Session::open_mode(root, store, true)?;
    let (value, skipped) = session.value_report::<T>()?;
    Ok((
        serde_json::from_value(value)?,
        skipped != 0 || !session.view.warnings.is_empty() || session.view.own_bad,
    ))
}
pub(super) fn prepare(root: &Path, store: &str) -> io::Result<()> {
    Session::open(root, store).map(|_| ())
}
pub(super) fn folder_shared(root: &Path, store: &str) -> io::Result<Value> {
    fn read<T: Serialize + DeserializeOwned + Default>(
        root: &Path,
        store: &str,
    ) -> io::Result<Value> {
        let s = Session::open_mode_preview(root, store, true, false)?;
        let (v, skipped) = s.value_report::<T>()?;
        if s.view.own_bad || !s.view.warnings.is_empty() || skipped > 0 {
            return Err(invalid(
                "Repair unreadable shared journals before enabling relay sync",
            ));
        }
        serde_json::from_value::<T>(v.clone())?;
        Ok(super::relay_preview::shared::selected(store, v))
    }
    match store {
        "projects.json" => read::<super::Projects>(root, store),
        "task-overrides.json" => read::<super::Overrides>(root, store),
        "memory/records.json" => read::<super::memory::MemoryFile>(root, store),
        _ => read::<super::Saved>(root, store),
    }
}
pub(super) fn shared_files(root: &Path) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for store in stores(root)?
        .into_iter()
        .filter(|s| super::relay_preview::shared::domain(s).is_some())
    {
        for path in legacy_sources(root, &store)? {
            let bytes = store_io::read_bounded(&path, super::file_reads::STORE_MAX)?;
            files.insert(
                path.strip_prefix(root)
                    .map_err(|_| invalid("backup path"))?
                    .to_string_lossy()
                    .replace('\\', "/"),
                bytes,
            );
        }
        for owner in entries(&root.join("state"))? {
            let name = owner.file_name().to_string_lossy().into_owned();
            if !device::valid_id(&name) {
                continue;
            }
            for entry in entries(&owner.path().join(hash(store.as_bytes())))? {
                if entry.path().extension().is_none_or(|e| e != "json") {
                    continue;
                }
                let journal = read_journal(&entry.path(), &name, Some(&store))?;
                if store != "task-overrides.json"
                    && super::relay_preview::shared::domain(&store)
                        .is_some_and(|d| d.starts_with("meeting:"))
                    && journal
                        .changes
                        .iter()
                        .all(|c| c.key.first().is_none_or(|k| k != "checked"))
                {
                    continue;
                }
                files.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .map_err(|_| invalid("backup path"))?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    store_io::read_bounded(&entry.path(), super::file_reads::JOURNAL_MAX)?,
                );
            }
        }
    }
    if root.join("teams.json").try_exists()? {
        files.insert(
            "teams.json".into(),
            store_io::read_bounded(&root.join("teams.json"), super::file_reads::JOURNAL_MAX)?,
        );
    }
    if files.values().map(Vec::len).sum::<usize>() as u64 > super::file_reads::STORE_MAX {
        return Err(invalid("Shared backup exceeds store limit"));
    }
    Ok(files)
}
pub(super) fn folder_manual(root: &Path) -> io::Result<(Vec<super::ManualTask>, Vec<String>)> {
    let session = Session::open_mode_preview(root, "task-overrides.json", true, false)?;
    let (value, skipped) = session.value_report::<super::Overrides>()?;
    if session.view.own_bad || !session.view.warnings.is_empty() || skipped != 0 {
        return Err(invalid(
            "Repair unreadable task journals before enabling relay sync",
        ));
    }
    let deleted = session
        .view
        .items
        .iter()
        .filter_map(|(key, value)| match key.as_slice() {
            [kind, id] if kind == "manual" && value.value.is_none() => Some(id.clone()),
            _ => None,
        })
        .collect();
    Ok((
        serde_json::from_value::<super::Overrides>(value)?.manual,
        deleted,
    ))
}

pub(super) fn manual_journal_files(root: &Path) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let store = "task-overrides.json";
    let mut files = BTreeMap::new();
    let mut paths = Vec::new();
    for owner in entries(&root.join("state"))? {
        let name = owner.file_name().to_string_lossy().into_owned();
        if !device::valid_id(&name) || !super::file_reads::directory(&owner.path()) {
            continue;
        }
        for entry in entries(&owner.path().join(hash(store.as_bytes())))? {
            if entry.path().extension().is_some_and(|ext| ext == "json") {
                let journal = read_journal(&entry.path(), &name, Some(store))?;
                if journal
                    .changes
                    .iter()
                    .any(|c| c.key.first().is_some_and(|k| k == "manual"))
                {
                    paths.push(entry.path());
                }
            }
        }
    }
    for path in legacy_sources(root, store)? {
        if let Some(bytes) = read_optional(&path)?
            && parse(&bytes)?.get("manual").is_some()
        {
            paths.push(path);
        }
    }
    let mut total = 0;
    for path in paths {
        let bytes = store_io::read_bounded(&path, super::file_reads::JOURNAL_MAX)?;
        total += bytes.len();
        if total as u64 > super::file_reads::STORE_MAX {
            return Err(invalid("Task backup exceeds the store limit"));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| invalid("Invalid task backup path"))?
            .to_string_lossy()
            .replace('\\', "/");
        files.insert(relative, bytes);
    }
    Ok(files)
}
pub(super) fn writer(root: &Path, store: &str, key: Vec<String>) -> io::Result<Option<String>> {
    let session = Session::open_mode(root, store, true)?;
    if let Some(preview) = &session.preview {
        return preview.shared_writer(store, key.into_iter().map(|k| format!("o:{k}")).collect());
    }
    Ok(session.view.items.get(&key).map(|r| r.device.clone()))
}
pub(super) fn diagnostics<T: Serialize + DeserializeOwned + Default>(
    root: &Path,
    store: &str,
) -> io::Result<usize> {
    let session = Session::open_mode(root, store, true)?;
    let (_, skipped) = session.value_report::<T>()?;
    Ok(skipped + session.view.warnings.len())
}
/// When each present item of a store was last written, in nanoseconds since 1970, read the same
/// way as `load`. Values imported from a legacy file carry no time and read as 0.
pub(super) fn stamps(root: &Path, store: &str) -> io::Result<BTreeMap<Key, u64>> {
    let session = Session::open_mode(root, store, true)?;
    let mut out: BTreeMap<_, _> = session
        .view
        .items
        .iter()
        .filter(|(_, r)| r.value.is_some())
        .map(|(k, r)| (k.clone(), r.at & !(1 << 63)))
        .collect();
    if let Some(preview) = &session.preview
        && super::relay_preview::shared::domain(store).is_some_and(|d| d.starts_with("meeting:"))
    {
        let prior = out.clone();
        out.retain(|k, _| k.first().is_none_or(|k| k != "checked"));
        for (key, at) in preview.check_stamps(store)? {
            out.insert(
                key.clone(),
                at.unwrap_or_else(|| prior.get(&key).copied().unwrap_or(0)),
            );
        }
    }
    Ok(out)
}
/// Includes tombstones so deleting and recreating the same value still invalidates Undo.
pub(super) fn settle_fence() {
    fence_local_mutation();
}

pub(super) fn settle_versions(root: &Path, store: &str) -> io::Result<BTreeMap<Key, String>> {
    let session = Session::open_mode(root, store, true)?;
    session
        .view
        .items
        .iter()
        .map(|(key, value)| Ok((key.clone(), serde_json::to_string(&value.version())?)))
        .collect()
}

pub(super) fn settle_locks(root: &Path, stores: &BTreeSet<String>) -> io::Result<Vec<fs::File>> {
    stores
        .iter()
        .map(|store| {
            store_io::lock(
                &root
                    .join("local/locks")
                    .join(format!("{}.lock", hash(store.as_bytes()))),
            )
        })
        .collect()
}

pub(super) fn update<T: Serialize + DeserializeOwned + Default>(
    root: &Path,
    store: &str,
    change: impl FnOnce(&mut T),
) -> io::Result<T> {
    let mut session = Session::open(root, store)?;
    let before_value = session.value::<T>()?;
    let mut value: T = serde_json::from_value(before_value)?;
    let before = flatten(store, serde_json::to_value(&value)?)?;
    change(&mut value);
    session.diff(before, flatten(store, serde_json::to_value(&value)?)?)?;
    // Include independent edits that arrived during the local operation.
    session.view = read_view(root, store, &session.own)?;
    let value = serde_json::from_value(session.value::<T>()?)?;
    fence_local_mutation();
    Ok(value)
}
pub(super) fn save_changes<T: Serialize>(
    root: &Path,
    store: &str,
    before: &T,
    after: &T,
) -> io::Result<()> {
    let mut session = Session::open(root, store)?;
    session.diff(
        flatten(store, serde_json::to_value(before)?)?,
        flatten(store, serde_json::to_value(after)?)?,
    )?;
    fence_local_mutation();
    Ok(())
}
/// Run a guarded edit under the existing store lock, then publish one immutable journal.
pub(super) fn update_checked<T: Serialize + DeserializeOwned + Default>(
    root: &Path,
    store: &str,
    change: impl FnOnce(&mut T) -> io::Result<()>,
) -> io::Result<T> {
    update_checked_validated(root, store, change, |_| Ok(()))
}
pub(super) fn update_checked_validated<T: Serialize + DeserializeOwned + Default>(
    root: &Path,
    store: &str,
    change: impl FnOnce(&mut T) -> io::Result<()>,
    validate: impl FnOnce(&T) -> io::Result<()>,
) -> io::Result<T> {
    let mut session = Session::open(root, store)?;
    let before = session.value::<T>()?;
    let registers = session.view.items.clone();
    let mut value: T = serde_json::from_value(before.clone())?;
    change(&mut value)?;
    session.view = read_view(root, store, &session.own)?;
    session.import_legacy()?;
    if session.view.own_bad || session.view.items != registers {
        return Err(invalid("Project state changed; review a fresh preview"));
    }
    super::project_merge::proof_pause(root, "publication")?;
    let latest = read_view(root, store, &session.own)?;
    if latest.own_bad || latest.items != registers {
        return Err(invalid("Project state changed; review a fresh preview"));
    }
    // Compare versions and choose the timestamp from this exact view. Never refresh past it.
    let at = latest.next_time()?;
    validate(&serde_json::from_value(before.clone())?)?;
    let final_view = read_view(root, store, &session.own)?;
    if final_view.own_bad || final_view.items != registers {
        return Err(invalid("Project state changed; review a fresh preview"));
    }
    session.publish_diff(
        flatten(store, before)?,
        flatten(store, serde_json::to_value(&value)?)?,
        at,
    )?;
    fence_local_mutation();
    Ok(value)
}
pub struct EditView<T> {
    pub value: T,
    pub generation: u64,
}
/// Save-time merge only. A busy incoming store returns a retryable error after three attempts.
pub(super) fn update_merged<T: Serialize + DeserializeOwned + Default>(
    root: &Path,
    store: &str,
    change: impl FnMut(&mut T) -> io::Result<()>,
) -> io::Result<EditView<T>> {
    update_merged_validated(root, store, change, || Ok(()))
}
pub(super) fn update_merged_validated<T: Serialize + DeserializeOwned + Default>(
    root: &Path,
    store: &str,
    mut change: impl FnMut(&mut T) -> io::Result<()>,
    validate: impl Fn() -> io::Result<()>,
) -> io::Result<EditView<T>> {
    for _ in 0..3 {
        let mut s = Session::open(root, store)?;
        let before = s.value::<T>()?;
        let mut value: T = serde_json::from_value(before.clone())?;
        change(&mut value)?;
        let latest = read_view(root, store, &s.own)?;
        if latest.own_bad {
            return Err(invalid("Own journal is unreadable"));
        }
        if latest.items != s.view.items || latest.summary_history != s.view.summary_history {
            continue;
        }
        validate()?;
        s.publish_diff(
            flatten(store, before)?,
            flatten(store, serde_json::to_value(&value)?)?,
            latest.next_time()?,
        )?;
        let generation = fence_local_mutation();
        return Ok(EditView { value, generation });
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "Changes are still arriving. Your changes are kept. Try Save again.",
    ))
}
pub(super) fn new_id(root: &Path) -> io::Result<String> {
    Ok(format!(
        "{}-{}",
        DeviceSettings::load_in(root)?.id,
        device::random_id()?
    ))
}
fn legacy_sources(root: &Path, store: &str) -> io::Result<Vec<PathBuf>> {
    let main = if store == "memory/legacy.json" {
        root.join("memory/company.md")
    } else {
        root.join(store)
    };
    let parent = main
        .parent()
        .ok_or_else(|| invalid("Missing legacy parent"))?;
    let prefix = format!(
        "{}.sync-conflict-",
        main.file_stem().unwrap_or_default().to_string_lossy()
    );
    let mut paths = Vec::new();
    if main.try_exists()? {
        paths.push(main.clone());
    }
    for entry in entries(parent)? {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(&prefix)
            && entry.path().extension() == main.extension()
            && super::file_reads::checked_metadata(&entry.path()).is_ok()
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}
fn validate_legacy(store: &str, value: &Value) -> io::Result<()> {
    if !value.is_object() {
        return Err(invalid("Legacy state must be an object"));
    }
    match store {
        "companies.json" => {
            serde_json::from_value::<super::companies::Companies>(value.clone())?;
        }
        "projects.json" => {
            serde_json::from_value::<super::Projects>(value.clone())?;
        }
        "task-overrides.json" => {
            serde_json::from_value::<super::Overrides>(value.clone())?;
        }
        super::summary_edits::FILE => {
            serde_json::from_value::<super::summary_edits::Records>(value.clone())?;
        }
        "tasks-settings.json" => {
            serde_json::from_value::<super::TasksSettings>(value.clone())?;
        }
        "agent-instructions.json" => {
            super::agent_instructions::validate_store(value)?;
        }
        "memory/records.json" => {
            let v: super::memory::MemoryFile = serde_json::from_value(value.clone())?;
            if v.schema_version != 1 {
                return Err(invalid("Unsupported memory schema"));
            }
        }
        "checks/triage.json" => {
            serde_json::from_value::<super::check_state::TriageFile>(value.clone())?;
        }
        "checks/state.json" => {}
        _ if report_store(store) => {
            serde_json::from_value::<super::daily_check::DailyCheck>(value.clone())?;
        }
        _ => {
            serde_json::from_value::<super::Saved>(value.clone())?;
        }
    }
    Ok(())
}
fn legacy_time(value: &Value) -> u64 {
    ["updated_at", "at", "decided_at", "accepted_at"]
        .iter()
        .filter_map(|k| {
            value
                .get(k)?
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .and_then(|d| d.timestamp_nanos_opt())
                .and_then(|n| u64::try_from(n).ok())
        })
        .max()
        .unwrap_or(0)
}
fn stable_id(value: &Value, counts: &mut BTreeMap<String, usize>) -> io::Result<String> {
    let digest = hash(&serde_json::to_vec(value)?);
    let n = counts.entry(digest.clone()).or_default();
    let id = format!("legacy-{digest}-{n}");
    *n += 1;
    Ok(id)
}
// Missing legacy identities are namespaced by installation and source, conservatively
// retaining independent identical tasks/questions rather than guessing they were one item.
fn legacy_items(store: &str, mut value: Value, namespace: &str) -> io::Result<Items> {
    for field in ["manual", "messages"] {
        if let Some(items) = value.get_mut(field).and_then(Value::as_array_mut) {
            let mut counts = BTreeMap::new();
            for (index, item) in items.iter_mut().enumerate() {
                if item
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                {
                    continue;
                }
                let id = format!(
                    "{}-{}",
                    hash(namespace.as_bytes()),
                    stable_id(item, &mut counts)?
                );
                let object = item
                    .as_object_mut()
                    .ok_or_else(|| invalid("Invalid legacy identity"))?;
                object.insert("id".into(), json!(id));
                if field == "messages" {
                    object.insert("legacy_order".into(), json!(index));
                }
            }
        }
    }
    flatten(store, value)
}
fn flatten(store: &str, value: Value) -> io::Result<Items> {
    if value.is_null() {
        return Ok(Items::new());
    }
    if report_store(store) {
        return Ok(BTreeMap::from([(vec!["report".into()], value)]));
    }
    let object = value
        .as_object()
        .ok_or_else(|| invalid("State must be an object"))?;
    let mut out = Items::new();
    for (field, value) in object {
        match (store, field.as_str()) {
            (_, "checked" | "projects") if value.is_array() => {
                for item in value.as_array().unwrap() {
                    let text = item
                        .as_str()
                        .ok_or_else(|| invalid("Invalid state set item"))?;
                    out.insert(vec![field.clone(), text.into()], item.clone());
                }
            }
            (_, "messages" | "manual" | "records" | "proposals" | "events") if value.is_array() => {
                let mut counts = BTreeMap::new();
                for (index, item) in value.as_array().unwrap().iter().enumerate() {
                    let mut item = item.clone();
                    let id = item
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .map(Ok)
                        .unwrap_or_else(|| stable_id(&item, &mut counts))?;
                    if matches!(field.as_str(), "messages" | "manual") {
                        let obj = item
                            .as_object_mut()
                            .ok_or_else(|| invalid("Invalid identified state item"))?;
                        if obj
                            .get("id")
                            .and_then(Value::as_str)
                            .is_none_or(str::is_empty)
                        {
                            obj.insert("id".into(), json!(id));
                            if field == "messages" {
                                obj.insert("legacy_order".into(), json!(index));
                            }
                        }
                    }
                    if out.insert(vec![field.clone(), id], item).is_some() {
                        return Err(invalid(
                            "Duplicate state identity; refusing to drop either item",
                        ));
                    }
                }
            }
            ("task-overrides.json", "tasks") => {
                for (meeting, tasks) in value
                    .as_object()
                    .ok_or_else(|| invalid("Invalid overrides"))?
                {
                    for (index, item) in tasks
                        .as_object()
                        .ok_or_else(|| invalid("Invalid meeting overrides"))?
                    {
                        out.insert(
                            vec![field.clone(), meeting.clone(), index.clone()],
                            item.clone(),
                        );
                    }
                }
            }
            ("projects.json", "aliases" | "merges" | "meeting_projects")
            | (super::summary_edits::FILE, "edits")
            | ("checks/triage.json", "findings")
            | ("memory/legacy.json", "documents") => {
                for (key, item) in value
                    .as_object()
                    .ok_or_else(|| invalid("Invalid state map"))?
                {
                    out.insert(vec![field.clone(), key.clone()], item.clone());
                }
            }
            ("checks/state.json", _) => {
                for (key, item) in value
                    .as_object()
                    .ok_or_else(|| invalid("Invalid legacy triage map"))?
                {
                    out.insert(vec![field.clone(), key.clone()], item.clone());
                }
            }
            _ => {
                out.insert(vec![field.clone()], value.clone());
            }
        }
    }
    Ok(out)
}
fn inflate(store: &str, items: Items, mut default: Value) -> io::Result<Value> {
    if report_store(store) {
        return Ok(items
            .get(&vec!["report".into()])
            .cloned()
            .unwrap_or(Value::Null));
    }
    if default.is_null() {
        default = json!({});
    }
    let root = default
        .as_object_mut()
        .ok_or_else(|| invalid("Invalid default state"))?;
    for (key, value) in items {
        match key.as_slice() {
            [field] => {
                root.insert(field.clone(), value);
            }
            [field, _id]
                if matches!(
                    field.as_str(),
                    "checked"
                        | "projects"
                        | "messages"
                        | "manual"
                        | "records"
                        | "proposals"
                        | "events"
                ) =>
            {
                root.entry(field.clone())
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .ok_or_else(|| invalid("Invalid state collection"))?
                    .push(value);
            }
            [field, id] => {
                root.entry(field.clone())
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or_else(|| invalid("Invalid state map"))?
                    .insert(id.clone(), value);
            }
            [field, meeting, index] if field == "tasks" => {
                root.entry(field.clone())
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or_else(|| invalid("Invalid task map"))?
                    .entry(meeting.clone())
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or_else(|| invalid("Invalid meeting task map"))?
                    .insert(index.clone(), value);
            }
            _ => return Err(invalid("Invalid state item shape")),
        }
    }
    if let Some(messages) = root.get_mut("messages").and_then(Value::as_array_mut) {
        messages.sort_by_key(|m| {
            (
                legacy_time(m),
                m.get("legacy_order").and_then(Value::as_u64).unwrap_or(0),
                m.get("id").and_then(Value::as_str).unwrap_or("").to_owned(),
            )
        });
    }
    Ok(default)
}
/// List logical stores, including read-only legacy input and immutable device journals.
pub(super) fn stores(root: &Path) -> io::Result<Vec<String>> {
    let mut stores = BTreeSet::new();
    for folder in [root.to_owned(), root.join("memory"), root.join("checks")] {
        for entry in entries(&folder)? {
            if !super::file_reads::checked_metadata(&entry.path()).is_ok() {
                continue;
            }
            let path = entry.path();
            let mut name = path
                .strip_prefix(root)
                .map_err(|_| invalid("Invalid legacy path"))?
                .to_string_lossy()
                .replace('\\', "/");
            if let Some((prefix, suffix)) = name.split_once(".sync-conflict-") {
                name = format!(
                    "{prefix}.{}",
                    suffix.rsplit_once('.').map_or("", |(_, ext)| ext)
                );
            }
            if name == "memory/company.md" {
                stores.insert("memory/legacy.json".into());
            } else if supported(&name) {
                stores.insert(name);
            }
        }
    }
    for owner in entries(&root.join("state"))? {
        let name = owner.file_name().to_string_lossy().into_owned();
        if !device::valid_id(&name) || !super::file_reads::directory(&owner.path()) {
            continue;
        }
        for folder in entries(&owner.path())? {
            if !super::file_reads::directory(&folder.path()) {
                continue;
            }
            for file in entries(&folder.path())? {
                if file.path().extension().is_some_and(|e| e == "json")
                    && let Ok(j) = read_journal(&file.path(), &name, None)
                {
                    stores.insert(j.store);
                }
            }
        }
    }
    Ok(stores.into_iter().collect())
}
#[cfg(any(target_os = "linux", debug_assertions, feature = "relay-qualification"))]
pub(super) fn ingestion_revision(root: &Path) -> io::Result<String> {
    let mut journals = BTreeSet::new();
    for owner in entries(&root.join("state"))? {
        let name = owner.file_name().to_string_lossy().into_owned();
        if !device::valid_id(&name) || !super::file_reads::directory(&owner.path()) {
            continue;
        }
        for folder in entries(&owner.path())? {
            if !super::file_reads::directory(&folder.path()) {
                continue;
            }
            for file in entries(&folder.path())? {
                if file.path().extension().is_some_and(|e| e == "json")
                    && let Ok(journal) = read_journal(&file.path(), &name, None)
                {
                    journals.insert((journal.device, journal.id, journal.store, journal.checksum));
                }
            }
        }
    }
    Ok(hash(&serde_json::to_vec(&journals)?))
}
pub(super) fn reconcile(root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let inspect = || -> io::Result<Vec<String>> {
        let mut warnings = Vec::new();
        for owner in entries(&root.join("state"))? {
            let name = owner.file_name().to_string_lossy().into_owned();
            if !device::valid_id(&name) || !super::file_reads::directory(&owner.path()) {
                continue;
            }
            for folder in entries(&owner.path())? {
                if !super::file_reads::directory(&folder.path()) {
                    continue;
                }
                for file in entries(&folder.path())? {
                    if file.path().extension().is_some_and(|e| e == "json")
                        && read_journal(&file.path(), &name, None).is_err()
                    {
                        warnings.push(
                            "Unreadable or incomplete device journal; file retained for retry"
                                .into(),
                        );
                    }
                }
            }
        }
        Ok(warnings)
    };
    match inspect() {
        Ok(warnings) => errors.extend(warnings),
        Err(_) => errors.push("Could not inspect all device journals".into()),
    }
    let stores = match stores(root) {
        Ok(s) => s,
        Err(_) => return vec!["Could not inspect device state".into()],
    };
    for store in stores {
        match Session::open_mode(root, &store, super::paths::read_only()) {
            Ok(s) => errors.extend(s.view.warnings),
            Err(_) => errors.push(format!(
                "Could not read or migrate {store}; original files were retained"
            )),
        }
    }
    errors.sort();
    errors.dedup();
    errors
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registers_converge_in_either_order_and_tombstones_win() {
        let mut a = Journal {
            version: 2,
            device: "a".repeat(32),
            id: "1".repeat(32),
            store: "all.json".into(),
            sources: BTreeSet::new(),
            checksum: String::new(),
            changes: vec![Change {
                key: vec!["checked".into(), "task".into()],
                at: 10,
                value: Some(json!("task")),
            }],
        };
        let mut b = a.clone();
        b.device = "b".repeat(32);
        b.changes[0].value = None;
        let mut first = View::default();
        first.merge(&a);
        first.merge(&b);
        let mut second = View::default();
        second.merge(&b);
        second.merge(&a);
        assert_eq!(first.values(), second.values());
        assert!(first.values().is_empty());
        a.changes[0].at = 9;
        first.merge(&a);
        assert!(first.values().is_empty());
        b.checksum = b.checksum().unwrap();
        assert_eq!(b.checksum, b.checksum().unwrap());
    }
    #[test]
    fn distinct_legacy_manual_tasks_do_not_share_an_identity() {
        let items = flatten(
            "task-overrides.json",
            json!({"manual": [
            {"n":1,"task":"first"}, {"n":1,"task":"second"}, {"n":1,"task":"first"}]}),
        )
        .unwrap();
        assert_eq!(items.len(), 3);
    }
}
