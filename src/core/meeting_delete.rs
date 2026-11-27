//! Explicit ownership for local meeting recycling.
pub use super::store_io::WriteStage;
use super::{file_reads, meeting_parts::Part, store_io};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub relative: PathBuf,
    identity: [u64; 2],
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HexInput {
    pub relative: PathBuf,
    pub bytes: u64,
    pub hash: String,
    pub directory: bool,
}

pub struct Inventory {
    pub ids: BTreeSet<String>,
    pub members: Vec<Member>,
    pub hex_inputs: Vec<HexInput>,
}

#[derive(Deserialize)]
struct Session {
    id: String,
    parts: Vec<Part>,
}

struct Selection {
    ids: BTreeSet<String>,
    physical: BTreeSet<String>,
    multipart: bool,
}

pub struct Storage {
    meetings: PathBuf,
    state: PathBuf,
}

fn invalid() -> io::Error {
    store_io::invalid("Unsafe or ambiguous meeting storage")
}

fn valid_id(id: &str) -> bool {
    id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

fn metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    store_io::reject_reparse(path)?;
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() || m.is_dir() => Ok(Some(m)),
        Ok(_) => Err(invalid()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn identity(_path: &Path, m: &fs::Metadata) -> io::Result<[u64; 2]> {
    use std::os::unix::fs::MetadataExt;
    if m.is_file() && m.nlink() != 1 {
        return Err(invalid());
    }
    Ok([m.dev(), m.ino()])
}

#[cfg(windows)]
fn windows_file_info(
    file: &fs::File,
) -> io::Result<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

#[cfg(windows)]
fn identity(path: &Path, m: &fs::Metadata) -> io::Result<[u64; 2]> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    let file = fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let info = windows_file_info(&file)?;
    if info.dwFileAttributes & 0x400 != 0 || (m.is_file() && info.nNumberOfLinks != 1) {
        return Err(invalid());
    }
    Ok([
        u64::from(info.dwVolumeSerialNumber),
        u64::from(info.nFileIndexHigh) << 32 | u64::from(info.nFileIndexLow),
    ])
}

fn open_hex_file(path: &Path) -> io::Result<fs::File> {
    let file = super::meeting_router::inputs::Snapshot::open_file(path)?;
    // Check the handle used for the fingerprint, since the path can change after discovery.
    #[cfg(windows)]
    if windows_file_info(&file)?.nNumberOfLinks != 1 {
        return Err(invalid());
    }
    Ok(file)
}

#[cfg(target_os = "linux")]
fn volume(path: &Path, _: [u64; 2]) -> io::Result<u64> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| invalid())?;
    let mut info: libc::statx = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::statx(
            libc::AT_FDCWD,
            path.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_MNT_ID,
            &mut info,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if info.stx_mask & libc::STATX_MNT_ID == 0 {
        return Err(invalid());
    }
    Ok(info.stx_mnt_id)
}

#[cfg(not(target_os = "linux"))]
fn volume(_: &Path, identity: [u64; 2]) -> io::Result<u64> {
    Ok(identity[0])
}

fn same_volume(expected: u64, actual: u64) -> io::Result<()> {
    if actual != expected {
        return Err(io::ErrorKind::CrossesDevices.into());
    }
    Ok(())
}

fn inspect(path: &Path, target_volume: u64) -> io::Result<Option<[u64; 2]>> {
    let Some(m) = metadata(path)? else {
        return Ok(None);
    };
    let key = identity(path, &m)?;
    let mut pending = vec![(path.to_owned(), m, key)];
    while let Some((path, m, key)) = pending.pop() {
        same_volume(target_volume, volume(&path, key)?)?;
        if m.is_dir() {
            for entry in fs::read_dir(path)? {
                let path = entry?.path();
                let m = metadata(&path)?.ok_or_else(invalid)?;
                let key = identity(&path, &m)?;
                pending.push((path, m, key));
            }
        }
    }
    Ok(Some(key))
}

fn read<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    Ok(serde_json::from_slice(&store_io::read_bounded(
        path,
        file_reads::SUMMARY_MAX,
    )?)?)
}

impl Storage {
    pub fn new(meetings: &Path, state: &Path) -> io::Result<Self> {
        for path in [meetings, state] {
            if path.components().any(|c| c == Component::ParentDir)
                || !metadata(path)?.is_some_and(|m| m.is_dir())
            {
                return Err(invalid());
            }
        }
        let meetings = fs::canonicalize(meetings)?;
        let state = fs::canonicalize(state)?;
        if meetings.starts_with(&state) || state.starts_with(&meetings) {
            return Err(invalid());
        }
        Ok(Self { meetings, state })
    }

    fn directory_volume(&self, path: &Path) -> io::Result<u64> {
        let m = metadata(path)?
            .filter(fs::Metadata::is_dir)
            .ok_or_else(invalid)?;
        volume(path, identity(path, &m)?)
    }

    fn volume(&self) -> io::Result<u64> {
        self.directory_volume(&self.state)
    }

    fn state_path(&self, relative: &Path) -> io::Result<PathBuf> {
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(invalid());
        }
        Ok(self.state.join(relative))
    }

    fn selection(&self, id: &str) -> io::Result<Selection> {
        if !valid_id(id) {
            return Err(invalid());
        }
        let records = self.state.join("local/meeting-parts");
        let mut sessions = Vec::new();
        if metadata(&records)?.is_some() {
            for entry in fs::read_dir(&records)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "json") {
                    continue;
                }
                let session: Session = read(&path)?;
                if !valid_id(&session.id)
                    || path.file_stem().and_then(|s| s.to_str()) != Some(session.id.as_str())
                    || session.parts.iter().any(|p| !valid_id(&p.id))
                {
                    return Err(invalid());
                }
                sessions.push(session);
            }
        }
        let mut ids = BTreeSet::from([id.to_owned()]);
        let mut physical = BTreeSet::new();
        let aggregate = PathBuf::from("local/meeting-inputs").join(id);
        let session = sessions.iter().find(|s| s.id == id);
        if let Some(session) = session {
            for part in &session.parts {
                if !physical.insert(part.id.clone()) {
                    return Err(invalid());
                }
                ids.insert(part.id.clone());
            }
            if !physical.contains(id) && metadata(&self.meetings.join(id))?.is_some() {
                return Err(invalid());
            }
        } else {
            if metadata(&self.state.join(aggregate))?.is_some() {
                return Err(invalid());
            }
            physical.insert(id.to_owned());
        }
        if sessions
            .iter()
            .filter(|s| s.id != id)
            .any(|s| ids.contains(&s.id) || s.parts.iter().any(|p| ids.contains(&p.id)))
        {
            return Err(invalid());
        }
        Ok(Selection {
            ids,
            physical,
            multipart: session.is_some(),
        })
    }

    pub fn inventory(&self, id: &str) -> io::Result<Inventory> {
        self.inventory_selected(id, self.selection(id)?)
    }

    fn inventory_selected(&self, id: &str, selection: Selection) -> io::Result<Inventory> {
        let Selection {
            ids,
            physical,
            multipart,
        } = selection;
        let mut inventory = Inventory {
            ids,
            members: Vec::new(),
            hex_inputs: Vec::new(),
        };
        if multipart {
            let aggregate = PathBuf::from("local/meeting-inputs").join(id);
            let _: serde_json::Map<String, serde_json::Value> =
                read(&self.state.join(&aggregate).join("manifest.json"))?;
            self.member(aggregate, true, &mut inventory.members)?;
            self.member(
                PathBuf::from("local/meeting-parts").join(format!("{id}.json")),
                true,
                &mut inventory.members,
            )?;
        }
        for part in physical {
            self.physical(&part, &mut inventory)?;
        }
        for id in &inventory.ids {
            self.state_members(id, &mut inventory.members)?;
        }
        Ok(inventory)
    }

    fn physical(&self, id: &str, inventory: &mut Inventory) -> io::Result<()> {
        let root = self.meetings.join(id);
        let _: serde_json::Map<String, serde_json::Value> = read(&root.join("manifest.json"))?;
        self.fingerprint_hex(Path::new(id), &mut inventory.hex_inputs)?;
        Ok(())
    }

    fn fingerprint_hex(
        &self,
        relative: &Path,
        entries: &mut Vec<HexInput>,
    ) -> io::Result<(u64, String)> {
        let path = self.meetings.join(relative);
        let m = metadata(&path)?.ok_or_else(invalid)?;
        let mut hash = blake3::Hasher::new();
        let mut bytes = 0_u64;
        if m.is_dir() {
            identity(&path, &m)?;
            let mut children = fs::read_dir(&path)?.collect::<io::Result<Vec<_>>>()?;
            children.sort_by_key(fs::DirEntry::file_name);
            for child in children {
                let name = child.file_name();
                let (size, digest) = self.fingerprint_hex(&relative.join(&name), entries)?;
                bytes = bytes.checked_add(size).ok_or_else(invalid)?;
                hash.update(&serde_json::to_vec(&(name, size, digest))?);
            }
        } else {
            let mut file = open_hex_file(&path)?;
            bytes = io::copy(&mut file, &mut hash)?;
            if bytes != m.len() {
                return Err(invalid());
            }
        }
        let hash = hash.finalize().to_hex().to_string();
        entries.push(HexInput {
            relative: relative.to_owned(),
            bytes,
            hash: hash.clone(),
            directory: m.is_dir(),
        });
        Ok((bytes, hash))
    }

    fn state_members(&self, id: &str, members: &mut Vec<Member>) -> io::Result<()> {
        for directory in [
            "transcripts",
            "router-work",
            "evidence",
            "joined-transcripts",
        ] {
            let relative = PathBuf::from("local").join(directory).join(id);
            self.member(relative, false, members)?;
        }
        for (directory, ext) in [
            ("router-queue", "json"),
            ("router-queue", "lock"),
            ("router-queue", "completed"),
            ("notes-jobs", "json"),
            ("notes-jobs", "lock"),
            ("draft-summaries", "json"),
            ("draft-outcomes", "json"),
            ("recording-guard", "json"),
            ("probe", "json"),
            ("auto-stop", "json"),
        ] {
            let relative = PathBuf::from("local")
                .join(directory)
                .join(format!("{id}.{ext}"));
            self.member(relative, false, members)?;
        }
        Ok(())
    }

    fn member(
        &self,
        relative: PathBuf,
        required: bool,
        members: &mut Vec<Member>,
    ) -> io::Result<()> {
        let path = self.state_path(&relative)?;
        let volume = self.volume()?;
        if let Some(identity) = inspect(&path, volume)? {
            if metadata(&path)?.is_some_and(|m| m.is_file() != relative.extension().is_some()) {
                return Err(invalid());
            }
            same_volume(
                volume,
                self.directory_volume(path.parent().ok_or_else(invalid)?)?,
            )?;
            members.push(Member { relative, identity });
        } else if required {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Eligibility {
    #[default]
    Ready,
    Recording,
    Paused,
    Processing,
    UnsavedNotes,
    Peer,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum Phase {
    Deleting,
    Deleted,
    Restoring,
    Restored,
    Purging,
    Purged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveEvent {
    SyncState,
    CreatedRecycleRoot,
    SyncLocal,
    CreatedTransaction,
    SyncRecycleRoot,
    Journal(Phase, WriteStage),
    Suppression(WriteStage),
    Receipt,
    Suppressed,
    Moved(usize),
    Recorded(usize),
    SyncMove(usize, bool),
    Finished(Phase),
    Unhide(WriteStage),
    Unhidden,
    BeforeRemove(usize),
    Removed(usize),
    SyncPurge,
}

#[derive(Debug)]
pub enum DeleteError {
    Refused(Eligibility),
    Failed(io::Error),
    Partial {
        transaction: String,
        source: io::Error,
    },
}

impl From<io::Error> for DeleteError {
    fn from(source: io::Error) -> Self {
        Self::Failed(source)
    }
}

#[derive(Deserialize, Serialize)]
pub struct Receipt {
    pub title: String,
    pub ids: BTreeSet<String>,
    pub deleted_ms: i64,
    pub phase: Phase,
    transaction: String,
    members: Vec<Member>,
    pub hex_inputs: Vec<HexInput>,
    moved: Vec<bool>,
}

type Suppression = BTreeMap<String, String>;

static VISIBILITY: std::sync::RwLock<u64> = std::sync::RwLock::new(0);

/// Keep the read guard through accepting an Ask event so a deletion cannot interleave
/// with publishing its answer.
pub fn read_visibility() -> std::sync::RwLockReadGuard<'static, u64> {
    VISIBILITY
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn suppression_path(state: &Path) -> PathBuf {
    state.join("local/deleted-meeting-ids.json")
}

fn read_suppression(state: &Path) -> io::Result<Suppression> {
    let path = suppression_path(state);
    match super::meeting_router::inputs::Snapshot::read_file(&path, file_reads::SUMMARY_MAX) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Suppression::new()),
        Err(e) => Err(e),
    }
}

pub fn suppressed_ids(state: &Path) -> io::Result<BTreeSet<String>> {
    let ids = read_suppression(state).inspect_err(|error| {
        file_reads::error(&suppression_path(state), error);
    })?;
    Ok(ids.into_keys().collect())
}

pub struct MeetingClaim {
    _file: fs::File,
}

#[cfg(unix)]
impl Drop for MeetingClaim {
    fn drop(&mut self) {
        // A forked child can still hold a copy of the descriptor before exec.
        let _ = self._file.unlock();
    }
}

/// A writer must keep the returned claim until its last publication.
pub(crate) fn claim_write(state: &Path, id: &str) -> io::Result<Option<MeetingClaim>> {
    let claim = MeetingClaim {
        _file: store_io::shared_lock(&claim_path(state, id)?)?,
    };
    Ok((!read_suppression(state)?.contains_key(id)).then_some(claim))
}

pub(super) fn busy(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock || matches!(error.raw_os_error(), Some(32 | 33))
}

pub(super) fn lock_busy(path: &Path) -> io::Result<bool> {
    if !path.try_exists()? {
        return Ok(false);
    }
    match store_io::try_lock(path) {
        Ok(_) => Ok(false),
        Err(e) if busy(&e) => Ok(true),
        Err(e) => Err(e),
    }
}

pub fn writer_claim(state: &Path, id: &str) -> io::Result<MeetingClaim> {
    Ok(MeetingClaim {
        _file: store_io::try_lock(&claim_path(state, id)?)?,
    })
}

fn claim_path(state: &Path, id: &str) -> io::Result<PathBuf> {
    if !valid_id(id) {
        return Err(invalid());
    }
    Ok(state.join("local/deleted-meeting-claims").join(id))
}

impl Storage {
    fn transaction(&self, id: &str) -> io::Result<PathBuf> {
        if !super::device::valid_id(id) {
            return Err(invalid());
        }
        Ok(self.state.join("local/deleted-meetings").join(id))
    }

    pub fn receipt(&self, id: &str) -> io::Result<Receipt> {
        let path = self.transaction(id)?.join("receipt.json");
        let m = metadata(&path)?
            .filter(fs::Metadata::is_file)
            .ok_or_else(invalid)?;
        identity(&path, &m)?;
        let (seal, data): (String, Receipt) = read(&path)?;
        if seal != blake3::hash(&serde_json::to_vec(&data)?).to_hex().as_str()
            || data.transaction != id
        {
            return Err(invalid());
        }
        Ok(data)
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    let _ = path;
    Ok(())
}

fn move_member(
    from: &Path,
    to: &Path,
    member: &Member,
    pending: bool,
    index: usize,
    checkpoint: &mut impl FnMut(MoveEvent) -> io::Result<()>,
) -> io::Result<()> {
    #[cfg(windows)]
    if pending {
        store_io::reject_reparse(from)?;
        store_io::reject_reparse(to)?;
        if !expected_member(from, member)? {
            return Err(invalid());
        }
        crate::platform::publish_file(from, to)?;
        checkpoint(MoveEvent::Moved(index))?;
    }
    #[cfg(target_os = "linux")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let from_parent = store_io::checked_parent_existing(from)?;
        let to_parent = store_io::checked_parent_existing(to)?;
        let from_path = from_parent.path.join(from.file_name().ok_or_else(invalid)?);
        let to_path = to_parent.path.join(to.file_name().ok_or_else(invalid)?);
        if pending && !expected_member(&from_path, member)? {
            return Err(invalid());
        }
        let from = CString::new(from_path.as_os_str().as_bytes()).map_err(|_| invalid())?;
        let to = CString::new(to_path.as_os_str().as_bytes()).map_err(|_| invalid())?;
        // A local recreation after preflight must never be overwritten.
        if pending
            && unsafe {
                libc::renameat2(
                    libc::AT_FDCWD,
                    from.as_ptr(),
                    libc::AT_FDCWD,
                    to.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if pending {
            checkpoint(MoveEvent::Moved(index))?;
        }
        checkpoint(MoveEvent::SyncMove(index, true))?;
        sync_directory(&from_parent.path)?;
        checkpoint(MoveEvent::SyncMove(index, false))?;
        sync_directory(&to_parent.path)?;
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    return Err(io::ErrorKind::Unsupported.into());
    if !expected_member(to, member)? {
        return Err(invalid());
    }
    Ok(())
}

fn expected_member(path: &Path, member: &Member) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(m) if !crate::platform::linked(&m) => Ok(identity(path, &m)? == member.identity),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

impl Storage {
    fn lock(&self) -> io::Result<fs::File> {
        store_io::lock(&self.state.join("local/deleted-meetings.lock"))
    }

    fn journal_bytes(receipt: &Receipt) -> io::Result<Vec<u8>> {
        let seal = blake3::hash(&serde_json::to_vec(receipt)?)
            .to_hex()
            .to_string();
        Self::bounded(&(seal, receipt))
    }

    fn bounded(value: &impl Serialize) -> io::Result<Vec<u8>> {
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() as u64 > file_reads::SUMMARY_MAX {
            return Err(store_io::invalid("Meeting recycle metadata is too large"));
        }
        Ok(bytes)
    }

    fn journal(
        &self,
        receipt: &Receipt,
        checkpoint: &mut impl FnMut(MoveEvent) -> io::Result<()>,
    ) -> io::Result<()> {
        let path = self.transaction(&receipt.transaction)?.join("receipt.json");
        store_io::atomic_write_checked(&path, &Self::journal_bytes(receipt)?, |stage| {
            checkpoint(MoveEvent::Journal(receipt.phase, stage))
        })
    }

    fn claims(&self, ids: &BTreeSet<String>) -> io::Result<Vec<MeetingClaim>> {
        ids.iter().map(|id| writer_claim(&self.state, id)).collect()
    }

    pub fn delete(
        &self,
        id: &str,
        title: String,
        now_ms: i64,
        eligibility: impl FnOnce(&BTreeSet<String>) -> io::Result<Eligibility>,
        mut moved: impl FnMut(MoveEvent) -> io::Result<()>,
    ) -> Result<String, DeleteError> {
        let _lock = self.lock()?;
        let initial = self.selection(id)?;
        let _claims = match self.claims(&initial.ids) {
            Err(e) if busy(&e) => return Err(DeleteError::Refused(Eligibility::Processing)),
            result => result?,
        };
        let selected = self.selection(id)?;
        if selected.ids != initial.ids || chrono::DateTime::from_timestamp_millis(now_ms).is_none()
        {
            return Err(invalid().into());
        }
        let eligibility = eligibility(&selected.ids)?;
        if eligibility != Eligibility::Ready {
            return Err(DeleteError::Refused(eligibility));
        }
        let Inventory {
            ids,
            members,
            hex_inputs,
        } = self.inventory_selected(id, selected)?;
        let suppression = read_suppression(&self.state)?;
        if ids.iter().any(|id| suppression.contains_key(id)) {
            return Err(invalid().into());
        }
        let transaction = super::device::random_id()?;
        let mut receipt = Receipt {
            title,
            ids,
            deleted_ms: now_ms,
            phase: Phase::Deleting,
            transaction: transaction.clone(),
            moved: vec![false; members.len()],
            members,
            hex_inputs,
        };
        // Moved flags shrink on delete. Reserve room for the longer restore phase.
        if Self::journal_bytes(&receipt)?.len() as u64 + 16 > file_reads::SUMMARY_MAX {
            return Err(invalid().into());
        }
        self.suppression_bytes(&receipt)?;
        let dir = self.transaction(&transaction)?;
        let parent = dir.parent().ok_or_else(invalid)?;
        store_io::reject_reparse(parent)?;
        moved(MoveEvent::SyncState)?;
        sync_directory(&self.state)?;
        fs::create_dir_all(parent)?;
        moved(MoveEvent::CreatedRecycleRoot)?;
        same_volume(self.volume()?, self.directory_volume(parent)?)?;
        moved(MoveEvent::SyncLocal)?;
        sync_directory(parent.parent().ok_or_else(invalid)?)?;
        fs::create_dir(&dir)?;
        moved(MoveEvent::CreatedTransaction)?;
        moved(MoveEvent::SyncRecycleRoot)?;
        sync_directory(parent)?;
        self.journal(&receipt, &mut moved)
            .and_then(|()| moved(MoveEvent::Receipt))
            .and_then(|()| self.resume(&mut receipt, &mut moved))
            .map_err(|source| DeleteError::Partial {
                transaction: transaction.clone(),
                source,
            })?;
        Ok(transaction)
    }

    fn suppression_bytes(&self, receipt: &Receipt) -> io::Result<Vec<u8>> {
        let mut ids = read_suppression(&self.state)?;
        for id in &receipt.ids {
            if ids
                .get(id)
                .is_some_and(|owner| owner != &receipt.transaction)
            {
                return Err(invalid());
            }
            ids.insert(id.clone(), receipt.transaction.clone());
        }
        Self::bounded(&ids)
    }

    fn suppress(
        &self,
        receipt: &Receipt,
        checkpoint: &mut impl FnMut(MoveEvent) -> io::Result<()>,
    ) -> io::Result<()> {
        let bytes = self.suppression_bytes(receipt)?;
        let mut visibility = VISIBILITY
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Replacement can expose the tombstone before a later flush reports an error.
        if receipt.phase == Phase::Deleting {
            *visibility = visibility.wrapping_add(1);
            super::sync_state::settle_fence();
        }
        store_io::atomic_write_checked(&suppression_path(&self.state), &bytes, |stage| {
            checkpoint(MoveEvent::Suppression(stage))
        })
    }

    fn recycled(dir: &Path, index: usize) -> PathBuf {
        dir.join(index.to_string())
    }

    fn resume(
        &self,
        receipt: &mut Receipt,
        moved: &mut impl FnMut(MoveEvent) -> io::Result<()>,
    ) -> io::Result<()> {
        if receipt.phase == Phase::Restored {
            return self.unsuppress(receipt, moved);
        }
        let deleting = receipt.phase == Phase::Deleting;
        let starting_restore = receipt.phase == Phase::Deleted;
        let dir = self.transaction(&receipt.transaction)?;
        let volume = self.volume()?;
        let mut work = Vec::new();
        for (index, member) in receipt.members.iter().enumerate() {
            if !deleting && !receipt.moved[index] {
                continue;
            }
            let original = self.state_path(&member.relative)?;
            let recycled = Self::recycled(&dir, index);
            let (from, to) = if deleting {
                (original, recycled)
            } else {
                (recycled, original)
            };
            if starting_restore && metadata(&to)?.is_some() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            let source = expected_member(&from, member)?;
            let target = inspect(&to, volume)?;
            let pending = match (source, target, receipt.moved[index] == deleting) {
                (true, None, false) => true,
                (false, Some(key), _) if key == member.identity => false,
                _ => return Err(invalid()),
            };
            for path in [&from, &to] {
                let parent = path.parent().ok_or_else(invalid)?;
                same_volume(volume, self.directory_volume(parent)?)?;
            }
            work.push((index, from, to, pending));
        }

        if starting_restore {
            receipt.phase = Phase::Restoring;
            self.journal(receipt, moved)?;
            moved(MoveEvent::Receipt)?;
        }
        self.suppress(receipt, moved)?;
        moved(MoveEvent::Suppressed)?;
        for (index, from, to, pending) in work {
            if receipt.moved[index] != deleting {
                let member = &receipt.members[index];
                move_member(&from, &to, member, pending, index, moved)?;
                receipt.moved[index] = deleting;
                self.journal(receipt, moved)?;
                moved(MoveEvent::Recorded(index))?;
            }
        }
        receipt.phase = if deleting {
            Phase::Deleted
        } else {
            Phase::Restored
        };
        self.journal(receipt, moved)?;
        moved(MoveEvent::Finished(receipt.phase))?;
        if !deleting {
            self.unsuppress(receipt, moved)?;
        }
        Ok(())
    }

    pub fn recover(&self, id: &str) -> io::Result<Phase> {
        let _lock = self.lock()?;
        let mut receipt = self.receipt(id)?;
        let _claims = self.claims(&receipt.ids)?;
        if matches!(
            receipt.phase,
            Phase::Deleting | Phase::Restoring | Phase::Restored
        ) {
            self.resume(&mut receipt, &mut |_| Ok(()))?;
        }
        Ok(receipt.phase)
    }
    pub fn restore(&self, id: &str) -> io::Result<()> {
        self.restore_checked(id, |_| Ok(()))
    }

    pub fn restore_checked(
        &self,
        id: &str,
        mut checkpoint: impl FnMut(MoveEvent) -> io::Result<()>,
    ) -> io::Result<()> {
        let _lock = self.lock()?;
        let mut receipt = self.receipt(id)?;
        let _claims = self.claims(&receipt.ids)?;
        match receipt.phase {
            Phase::Deleted | Phase::Restoring | Phase::Restored => {
                self.resume(&mut receipt, &mut checkpoint)
            }
            _ => Err(invalid()),
        }
    }

    fn unsuppress(
        &self,
        receipt: &Receipt,
        checkpoint: &mut impl FnMut(MoveEvent) -> io::Result<()>,
    ) -> io::Result<()> {
        let path = suppression_path(&self.state);
        let mut ids = read_suppression(&self.state)?;
        let prior = ids.len();
        ids.retain(|_, owner| owner != &receipt.transaction);
        if ids.len() != prior {
            let mut visibility = VISIBILITY
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            store_io::atomic_write_checked(&path, &Self::bounded(&ids)?, |stage| {
                checkpoint(MoveEvent::Unhide(stage))
            })?;
            *visibility = visibility.wrapping_add(1);
            super::sync_state::settle_fence();
        }
        checkpoint(MoveEvent::Unhidden)
    }

    pub fn transactions(&self) -> io::Result<Vec<String>> {
        let path = self.state.join("local/deleted-meetings");
        let mut ids = Vec::new();
        if metadata(&path)?.is_some() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                if let Some(id) = entry
                    .file_name()
                    .to_str()
                    .filter(|id| super::device::valid_id(id))
                    && metadata(&entry.path().join("receipt.json"))?.is_some()
                {
                    ids.push(id.to_owned());
                }
            }
        }
        ids.sort_unstable();
        Ok(ids)
    }
    pub fn purge(&self, id: &str, now_ms: i64) -> io::Result<bool> {
        self.purge_checked(id, now_ms, |_| Ok(()))
    }

    pub fn purge_checked(
        &self,
        id: &str,
        now_ms: i64,
        mut checkpoint: impl FnMut(MoveEvent) -> io::Result<()>,
    ) -> io::Result<bool> {
        const RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
        let _lock = self.lock()?;
        let mut receipt = self.receipt(id)?;
        if !matches!(receipt.phase, Phase::Deleted | Phase::Purging)
            || chrono::DateTime::from_timestamp_millis(now_ms).is_none()
            || !now_ms
                .checked_sub(receipt.deleted_ms)
                .is_some_and(|ms| ms >= RETENTION_MS)
        {
            return Ok(false);
        }
        let dir = self.transaction(id)?;
        let volume = self.volume()?;
        let mut existing = Vec::new();
        for (index, member) in receipt.members.iter().enumerate() {
            if !receipt.moved[index] {
                continue;
            }
            let path = Self::recycled(&dir, index);
            match inspect(&path, volume)? {
                Some(key) if key == member.identity => existing.push((index, path)),
                None if receipt.phase == Phase::Purging => {}
                _ => return Err(invalid()),
            }
        }
        self.suppress(&receipt, &mut checkpoint)?;
        receipt.phase = Phase::Purging;
        self.journal(&receipt, &mut checkpoint)?;
        checkpoint(MoveEvent::Receipt)?;
        for (index, path) in existing {
            checkpoint(MoveEvent::BeforeRemove(index))?;
            if metadata(&path)?.is_some_and(|m| m.is_dir()) {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
            checkpoint(MoveEvent::Removed(index))?;
        }
        checkpoint(MoveEvent::SyncPurge)?;
        sync_directory(&dir)?;
        receipt.phase = Phase::Purged;
        self.journal(&receipt, &mut checkpoint)?;
        checkpoint(MoveEvent::Finished(receipt.phase))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn delete_can_proceed_after_a_writer_releases_a_duplicated_claim() {
        use super::{DeleteError, Eligibility, Storage, claim_write, fs};

        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let meetings = root.path().join("meetings");
        fs::create_dir(&state).unwrap();
        fs::create_dir_all(meetings.join("100-1")).unwrap();
        fs::write(meetings.join("100-1/manifest.json"), b"{}").unwrap();
        let storage = Storage::new(&meetings, &state).unwrap();
        let delete = || {
            storage.delete(
                "100-1",
                String::new(),
                0,
                |_| Ok(Eligibility::Ready),
                |_| Ok(()),
            )
        };

        let claim = claim_write(&state, "100-1").unwrap().unwrap();
        // Like fork, cloning the descriptor keeps the same open file description alive.
        let inherited = claim._file.try_clone().unwrap();
        assert!(matches!(
            delete(),
            Err(DeleteError::Refused(Eligibility::Processing))
        ));
        drop(claim);
        delete().unwrap();
        drop(inherited);
    }

    #[test]
    fn different_volumes_refuse_before_a_move() {
        assert_eq!(
            super::same_volume(1, 2).unwrap_err().kind(),
            std::io::ErrorKind::CrossesDevices
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn delete_hex_opener_refuses_hard_links() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("original");
        let alias = root.path().join("alias");
        super::fs::write(&source, b"synthetic").unwrap();
        assert!(super::open_hex_file(&source).is_ok());
        super::fs::hard_link(&source, &alias).unwrap();
        assert_eq!(
            super::open_hex_file(&alias).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }
}
