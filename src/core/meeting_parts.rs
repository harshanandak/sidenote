//! Sidenote owns the meeting; Hex owns each recording part.
use super::{file_reads, store_io};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Recording,
    #[default]
    Paused,
    Ended,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Part {
    pub id: String,
    pub started_ms: i64,
    pub duration_ms: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Meeting {
    pub id: String,
    pub parts: Vec<Part>,
    pub phase: Phase,
    pub notes_parts: usize,
    pub pending_start_ms: Option<i64>,
    pub requested_parts: usize,
}
fn valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
fn path(root: &Path, id: &str) -> io::Result<PathBuf> {
    if !valid(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    Ok(root.join("local/meeting-parts").join(format!("{id}.json")))
}
pub fn read(root: &Path, id: &str) -> io::Result<Option<Meeting>> {
    let path = path(root, id)?;
    match file_reads::read(&path, file_reads::SUMMARY_MAX) {
        Ok(raw) => {
            let m: Meeting = serde_json::from_slice(&raw)?;
            if m.id != id || m.parts.iter().any(|p| !valid(&p.id)) {
                return Err(store_io::invalid("Invalid meeting parts"));
            }
            Ok(Some(m))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
pub fn list(root: &Path) -> io::Result<Vec<Meeting>> {
    let directory = root.join("local/meeting-parts");
    let entries = match file_reads::dir_iter(&directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut result = Vec::new();
    for entry in entries.take(file_reads::ENTRIES_MAX) {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e == "json") {
            let id = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if let Some(m) = read(root, id)? {
                result.push(m);
            }
        }
    }
    Ok(result)
}
pub fn owner(root: &Path, part: &str) -> io::Result<Option<Meeting>> {
    Ok(list(root)?
        .into_iter()
        .find(|m| m.parts.iter().any(|p| p.id == part)))
}
pub enum Draft {
    Empty,
    Primary,
    Recovery,
}

/// Returns true when the pending record was removed or finished.
pub fn discard_pending(root: &Path, id: &str, draft: Draft) -> io::Result<bool> {
    // A save to this folder must either finish before the notes check or retry via recovery.
    let _write = super::live::NOTES_WRITE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let Some(mut m) = read(root, id)? else {
        return Ok(false);
    };
    if !m.parts.is_empty() {
        return Ok(false);
    }
    let _claim = m.claim(root)?;
    let folder = inputs(root).join(id);
    match (
        &draft,
        std::fs::metadata(folder.join(super::live::NOTES_FILE)),
    ) {
        (_, Ok(meta)) if meta.is_file() => {
            if m.pending_start_ms.take().is_some() || m.phase != Phase::Ended {
                m.phase = Phase::Ended;
                m.save_claimed(root)?;
            }
            return Ok(true);
        }
        (_, Err(error)) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        (Draft::Primary, _) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "The primary notes file is missing",
            ));
        }
        // Remove the record first so a blocked inputs directory cannot restore this draft.
        (Draft::Recovery, _) => std::fs::remove_file(path(root, id)?)?,
        _ => {}
    }
    if let Err(error) = std::fs::remove_dir_all(folder)
        && error.kind() != io::ErrorKind::NotFound
    {
        return Err(error);
    }
    if !matches!(draft, Draft::Recovery) {
        std::fs::remove_file(path(root, id)?)?;
    }
    Ok(true)
}
pub fn inputs(root: &Path) -> PathBuf {
    root.join("local/meeting-inputs")
}
pub fn source(root: &Path, hex: &Path, id: &str) -> PathBuf {
    if read(root, id).ok().flatten().is_some() {
        inputs(root)
    } else {
        hex.to_owned()
    }
}
impl Meeting {
    fn claim(&self, root: &Path) -> io::Result<Vec<super::meeting_delete::MeetingClaim>> {
        let ids: std::collections::BTreeSet<_> = std::iter::once(self.id.as_str())
            .chain(self.parts.iter().map(|part| part.id.as_str()))
            .collect();
        ids.into_iter()
            .map(|id| {
                super::meeting_delete::claim_write(root, id)?
                    .ok_or_else(|| io::ErrorKind::NotFound.into())
            })
            .collect()
    }

    pub fn write_view(&self, root: &Path) -> io::Result<()> {
        let _claim = self.claim(root)?;
        if self.requested_parts > 0 {
            return Ok(());
        }
        let folder = inputs(root).join(&self.id);
        store_io::write(
            &folder.join("manifest.json"),
            &serde_json::json!({"schema_version":1,
            "status":"complete","created_at_ms":self.parts.first().map(|p|p.started_ms).or(self.pending_start_ms),"duration_ms":self.elapsed_ms()}),
        )
    }
    pub fn save(&self, root: &Path) -> io::Result<()> {
        let _claim = self.claim(root)?;
        self.save_claimed(root)
    }

    fn save_claimed(&self, root: &Path) -> io::Result<()> {
        let path = path(root, &self.id)?;
        let _lock = store_io::lock(&path.with_extension("lock"))?;
        let mut next = self.clone();
        if let Some(saved) = read(root, &self.id)? {
            let mut parts = saved.parts;
            for part in &next.parts {
                if let Some(old) = parts.iter_mut().find(|p| p.id == part.id) {
                    *old = part.clone();
                } else {
                    parts.push(part.clone());
                }
            }
            next.parts = parts;
            next.notes_parts = next.notes_parts.max(saved.notes_parts);
        }
        store_io::write(&path, &next)
    }
    pub fn pending(root: &Path, started_ms: i64) -> io::Result<Self> {
        let id = format!("{started_ms}-{}", std::process::id());
        let m = Self {
            id,
            pending_start_ms: Some(started_ms),
            ..Default::default()
        };
        m.save(root)?;
        m.write_view(root)?;
        Ok(m)
    }
    pub fn begin(root: &Path, id: &str, started_ms: i64) -> io::Result<Self> {
        if let Some(m) = read(root, id)? {
            return Ok(m);
        }
        let m = Self {
            id: id.into(),
            parts: vec![Part {
                id: id.into(),
                started_ms,
                duration_ms: 0,
            }],
            phase: Phase::Recording,
            notes_parts: 0,
            pending_start_ms: None,
            requested_parts: 0,
        };
        m.save(root)?;
        m.write_view(root)?;
        Ok(m)
    }
    pub fn resume(&mut self, root: &Path, id: &str, started_ms: i64) -> io::Result<()> {
        path(root, id)?;
        if !self.parts.iter().any(|p| p.id == id) {
            self.parts.push(Part {
                id: id.into(),
                started_ms,
                duration_ms: 0,
            });
        }
        self.phase = Phase::Recording;
        self.pending_start_ms = None;
        self.save(root)
    }
    pub fn pause(&mut self, root: &Path) -> io::Result<()> {
        self.phase = Phase::Paused;
        self.save(root)?;
        self.write_view(root)
    }
    pub fn notes_ready(&self, root: &Path) -> io::Result<bool> {
        let folder = inputs(root).join(&self.id);
        if !folder.join("english-summary.json").is_file() {
            return Ok(false);
        }
        if matches!(
            super::Config::load_in(root)?
                .notes
                .unwrap_or_default()
                .engine(),
            super::notes::Engine::Scripts(_)
        ) {
            let summary =
                super::file_reads::checked_metadata(&folder.join("english-summary.json"))?
                    .modified()?;
            let manifest =
                super::file_reads::checked_metadata(&folder.join("manifest.json"))?.modified()?;
            let transcript =
                super::file_reads::checked_metadata(&folder.join("transcript.ndjson"))?
                    .modified()?;
            return Ok(summary >= manifest && summary >= transcript);
        }
        super::notes::queue::input_finished(root, &folder, &self.id)
    }
    pub fn stale(&self) -> bool {
        self.notes_parts > 0 && self.parts.len() > self.notes_parts
    }
    pub fn elapsed_ms(&self) -> u64 {
        self.parts
            .iter()
            .fold(0u64, |total, p| total.saturating_add(p.duration_ms))
    }
    pub fn refresh(&mut self, hex: &Path) -> io::Result<()> {
        for part in &mut self.parts {
            let raw = file_reads::read(&hex.join(&part.id).join("manifest.json"), 1024 * 1024)?;
            let m: serde_json::Value = serde_json::from_slice(&raw)?;
            if let Some(duration) = m["duration_ms"]
                .as_f64()
                .filter(|d| d.is_finite() && *d >= 0.0)
            {
                let duration = duration.min(u64::MAX as f64) as u64;
                if m["status"] == "complete" || duration > part.duration_ms {
                    part.duration_ms = duration;
                }
            }
        }
        Ok(())
    }
    /// Recovery never sends a recorder command or starts notes.
    pub fn restore(mut self, root: &Path, hex: &Path) -> io::Result<Self> {
        if self.parts.is_empty()
            && self.pending_start_ms.is_none()
            && inputs(root)
                .join(&self.id)
                .join(super::live::NOTES_FILE)
                .is_file()
        {
            return Ok(self);
        }
        if let Some(since) = self.pending_start_ms {
            let mut found = Vec::new();
            for entry in file_reads::dir_iter(hex)?.take(file_reads::ENTRIES_MAX) {
                let entry = entry?;
                if !file_reads::directory(&entry.path()) {
                    continue;
                }
                let id = entry.file_name().to_string_lossy().into_owned();
                if !valid(&id) || self.parts.iter().any(|p| p.id == id) {
                    continue;
                }
                let raw = file_reads::read(&entry.path().join("manifest.json"), 1024 * 1024)?;
                let m: serde_json::Value = serde_json::from_slice(&raw)?;
                if m["created_at_ms"]
                    .as_i64()
                    .is_some_and(|at| at >= since && at <= since.saturating_add(25_000))
                {
                    found.push((id, m["created_at_ms"].as_i64().unwrap()));
                }
            }
            if found.len() == 1 {
                self.resume(root, &found[0].0, found[0].1)?;
            } else if found.len() > 1 {
                return Err(store_io::invalid(
                    "Could not identify the resumed recording",
                ));
            }
        }
        self.refresh(hex)?;
        self.pending_start_ms = None;
        if self.phase == Phase::Recording {
            self.phase = Phase::Paused;
        }
        self.save(root)?;
        self.write_view(root)?;
        Ok(self)
    }
}
/// Physical parts never enter automatic notes, title, tasks, routing or Memory work.
pub fn held(root: &Path, id: &str) -> io::Result<bool> {
    Ok(owner(root, id)?.is_some())
}

#[path = "meeting_parts_input.rs"]
mod input;

#[cfg(test)]
#[path = "meeting_parts_tests.rs"]
mod tests;

/// A finished notification can beat the UI's first status reply after Start.
pub fn held_in(root: &Path, hex: &Path, id: &str) -> io::Result<bool> {
    path(root, id)?;
    if held(root, id)? {
        return Ok(true);
    }
    for mut m in list(root)? {
        let Some(since) = m.pending_start_ms else {
            continue;
        };
        let raw = file_reads::read(&hex.join(id).join("manifest.json"), 1024 * 1024)?;
        let v: serde_json::Value = serde_json::from_slice(&raw)?;
        if let Some(at) = v["created_at_ms"]
            .as_i64()
            .filter(|at| *at >= since && *at <= since.saturating_add(25_000))
        {
            m.resume(root, id, at)?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Preserve the old notes while the script writes the replacement for a joined input.
pub fn prepare_script_update(root: &Path, id: &str) -> io::Result<()> {
    let _claim = super::meeting_delete::claim_write(root, id)?.ok_or(io::ErrorKind::NotFound)?;
    let folder = inputs(root).join(id);
    let summary = folder.join("english-summary.json");
    if summary.is_file() {
        store_io::atomic_write(
            &folder.join("previous-summary.json"),
            &file_reads::read(&summary, file_reads::SUMMARY_MAX)?,
        )?;
        std::fs::remove_file(summary)?;
    }
    Ok(())
}
