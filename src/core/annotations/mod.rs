//! Read-only Hex Annotate bundles. Media is never opened by the reader.
pub mod generation;
pub mod media;
pub mod note;
pub mod notebook;
pub mod share;
use super::file_reads;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

/// Legacy scrawl word and mark times are seconds; Hex v1 times are converted from milliseconds.
pub const WORD_WINDOW_SECONDS: f64 = 3.0;
pub const WORD_END_LIMIT_SECONDS: f64 = 8.0;
pub const BUNDLE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MANIFEST_MAX_BYTES: u64 = 16 * 1024 * 1024;
const HEX_MANIFEST_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const IMAGE_MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Scrawl {
    pub version: serde_json::Value,
    pub id: String,
    pub created_at_ms: i64,
    pub duration_ms: u64,
    #[serde(deserialize_with = "null_default")]
    pub target: Target,
    #[serde(deserialize_with = "null_default")]
    pub transcript: Transcript,
    #[serde(deserialize_with = "null_default")]
    pub marks: Vec<Mark>,
    #[serde(deserialize_with = "null_default")]
    pub keyframes: Vec<Keyframe>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Target {
    pub mode: Option<String>,
    pub app: Option<String>,
    pub window_title: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Transcript {
    pub source: Option<String>,
    pub model: Option<String>,
    pub text: String,
    #[serde(deserialize_with = "null_default")]
    pub segments: Vec<Word>,
    #[serde(deserialize_with = "null_default")]
    pub words: Vec<Word>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Word {
    pub t0: f64,
    pub t1: f64,
    pub text: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Mark {
    pub id: serde_json::Value,
    pub t0: f64,
    pub t1: f64,
    pub box_norm: Option<[f32; 4]>,
    pub shape: Option<String>,
    pub description: Option<String>,
    pub frame: Option<FrameRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum FrameRef {
    Index(u64),
    File(String),
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Keyframe {
    pub index: u64,
    pub t: f64,
    pub reason: Option<String>,
    pub file: Option<String>,
    pub event: serde_json::Value,
}
#[derive(Clone, Debug)]
pub struct Capture {
    pub folder: PathBuf,
    pub data: Scrawl,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HexPackage {
    format_version: u64,
    #[serde(default)]
    annotation: serde_json::Value,
}
fn null_default<'de, D: serde::Deserializer<'de>, T: Deserialize<'de> + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
impl Capture {
    pub fn app(&self) -> &str {
        self.data
            .target
            .app
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Screen capture")
    }
    pub fn title(&self) -> &str {
        self.data
            .target
            .window_title
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Untitled capture")
    }
    pub fn word_count(&self) -> usize {
        if self.data.transcript.words.is_empty() {
            self.data.transcript.text.split_whitespace().count()
        } else {
            self.data.transcript.words.len()
        }
    }
    pub fn frame_for_mark(&self, mark: &Mark) -> Option<&Keyframe> {
        match mark.frame.as_ref()? {
            FrameRef::Index(index) => self.data.keyframes.iter().find(|f| f.index == *index),
            FrameRef::File(file) => self.data.keyframes.iter().find(|f| {
                f.file.as_ref().is_some_and(|name| {
                    name.replace('\\', "/") == file.replace('\\', "/")
                        || Path::new(&name.replace('\\', "/")).file_name()
                            == Path::new(&file.replace('\\', "/")).file_name()
                })
            }),
        }
    }
    pub fn words_for_mark(&self, mark: &Mark) -> Vec<&Word> {
        let start = (mark.t0 - WORD_WINDOW_SECONDS).max(0.0);
        let initial_end = mark.t1 + WORD_WINDOW_SECONDS;
        let limit = mark.t1 + WORD_END_LIMIT_SECONDS;
        let sentence_end = self
            .data
            .transcript
            .words
            .iter()
            .filter(|w| {
                w.t1 >= initial_end
                    && w.t1 <= limit
                    && w.text
                        .trim_end_matches(['\"', '\'', ')'])
                        .ends_with(['.', '?', '!'])
            })
            .map(|w| w.t1);
        let segment_end = self
            .data
            .transcript
            .segments
            .iter()
            .filter(|s| s.t1 >= initial_end && s.t1 <= limit)
            .map(|s| s.t1);
        let end = sentence_end
            .chain(segment_end)
            .min_by(f64::total_cmp)
            .unwrap_or(limit);
        self.data
            .transcript
            .words
            .iter()
            .filter(|w| w.t1 >= start && w.t0 <= end && w.t1 <= end && w.t1 >= w.t0)
            .collect()
    }
    /// Only PNGs below the frame directories may reach the image decoder.
    pub fn frame_path(&self, frame: &Keyframe) -> io::Result<PathBuf> {
        let file = frame
            .file
            .as_deref()
            .ok_or_else(|| invalid("Frame has no image"))?
            .replace('\\', "/");
        let relative = Path::new(&file);
        let parts: Vec<_> = relative.components().collect();
        if parts.len() < 2
            || file.contains(':')
            || !parts.iter().all(|p| matches!(p, Component::Normal(_)))
            || !matches!(parts[0], Component::Normal(s) if s == "frames" || s == "frames-full" || s == "frames-clean")
            || !relative
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("png"))
        {
            return Err(invalid(
                "Frame image must be a PNG inside the bundle's frames folder",
            ));
        }
        let path = self.folder.join(relative);
        if file_reads::checked_metadata(&path)?.len() > IMAGE_MAX_BYTES {
            return Err(invalid("Frame image exceeds 32 MB"));
        }
        Ok(path)
    }
}
#[derive(Default)]
pub struct Scan {
    pub captures: Vec<Capture>,
    pub errors: Vec<String>,
}
pub fn captures_dir() -> Option<PathBuf> {
    // Overrides also keep headless renders away from the owner's captures.
    crate::env_var("SIDENOTE_CAPTURES_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            #[cfg(windows)]
            {
                std::env::var_os("LOCALAPPDATA")
                    .map(|root| PathBuf::from(root).join("scrawl/captures"))
            }
            #[cfg(not(windows))]
            {
                Some(super::paths::data().map_or_else(
                    || crate::platform::data_home().join("scrawl/captures"),
                    |root| root.join("captures"),
                ))
            }
        })
}
pub fn annotation_roots() -> Vec<PathBuf> {
    if let Some(folder) = crate::env_var("SIDENOTE_CAPTURES_DIR") {
        return vec![PathBuf::from(folder)];
    }
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(|root| {
                let root = PathBuf::from(root);
                vec![root.join("hex/annotations"), root.join("scrawl/captures")]
            })
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        super::paths::data().map_or_else(
            || {
                let root = crate::platform::data_home();
                vec![root.join("hex/annotations"), root.join("scrawl/captures")]
            },
            |root| vec![root.join("hex/annotations"), root.join("captures")],
        )
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn bundle_size(folder: &Path) -> io::Result<u64> {
    if !file_reads::directory(folder) {
        return Err(invalid("Bundle is not a regular folder"));
    }
    let mut pending = vec![(folder.to_owned(), 0)];
    let mut size = 0u64;
    let mut entries = 0usize;
    while let Some((folder, depth)) = pending.pop() {
        if depth > 32 {
            return Err(invalid("Bundle is nested too deeply"));
        }
        for entry in fs::read_dir(folder)? {
            let entry = entry?;
            entries += 1;
            if entries > 100_000 {
                return Err(invalid("Bundle has too many files"));
            }
            let meta = fs::symlink_metadata(entry.path())?;
            if crate::platform::linked(&meta) {
                return Err(invalid("Bundle contains a linked file or folder"));
            }
            if meta.is_dir() {
                pending.push((entry.path(), depth + 1));
            } else if meta.is_file() {
                size = size.saturating_add(meta.len());
                if size > BUNDLE_MAX_BYTES {
                    return Err(invalid("Bundle exceeds 2 GB and was skipped"));
                }
            }
        }
    }
    Ok(size)
}
fn is_hex_package(folder: &Path) -> bool {
    folder
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("hexannotation"))
}
fn hex_times(mut data: Scrawl) -> Scrawl {
    for word in data
        .transcript
        .words
        .iter_mut()
        .chain(data.transcript.segments.iter_mut())
    {
        word.t0 /= 1000.0;
        word.t1 /= 1000.0;
    }
    for mark in &mut data.marks {
        mark.t0 /= 1000.0;
        mark.t1 /= 1000.0;
    }
    for frame in &mut data.keyframes {
        frame.t /= 1000.0;
    }
    data
}
fn read_manifest(folder: &Path) -> io::Result<(std::sync::Arc<[u8]>, Scrawl)> {
    if is_hex_package(folder) {
        let bytes = file_reads::read(&folder.join("annotation.json"), HEX_MANIFEST_MAX_BYTES)?;
        let package: HexPackage = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("Could not parse annotation.json"))?;
        if package.format_version > 1 {
            return Err(invalid(&format!(
                "annotation.json formatVersion {} is a future format",
                package.format_version
            )));
        }
        if package.format_version != 1 {
            return Err(invalid("annotation.json formatVersion must be 1"));
        }
        let data: Scrawl = serde_json::from_value(package.annotation)
            .map_err(|_| invalid("Could not parse annotation.json"))?;
        return Ok((bytes, hex_times(data)));
    }
    let bytes = file_reads::read(&folder.join("scrawl.json"), MANIFEST_MAX_BYTES)?;
    let data: Scrawl =
        serde_json::from_slice(&bytes).map_err(|_| invalid("Could not parse scrawl.json"))?;
    Ok((bytes, data))
}
pub fn read_capture(folder: &Path) -> io::Result<Capture> {
    bundle_size(folder)?;
    let (_, mut data) = read_manifest(folder)?;
    // Sort the frames, never the marks: mark numbers are their stable capture order.
    data.keyframes
        .sort_by(|a, b| a.t.total_cmp(&b.t).then(a.index.cmp(&b.index)));
    Ok(Capture {
        folder: folder.to_owned(),
        data,
    })
}
pub fn scan(folder: &Path) -> Scan {
    let mut scan = Scan::default();
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return scan,
        Err(_) => {
            scan.errors
                .push("Could not read the Hex captures folder.".into());
            return scan;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            scan.errors.push("Could not read a capture folder.".into());
            continue;
        };
        let path = entry.path();
        let hex = is_hex_package(&path);
        if !hex
            && !path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("scrawl"))
        {
            continue;
        }
        match read_capture(&path) {
            Ok(capture) => scan.captures.push(capture),
            Err(e) if hex && e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => scan
                .errors
                .push(format!("{}: {e}", entry.file_name().to_string_lossy())),
        }
    }
    scan.captures = dedupe(std::mem::take(&mut scan.captures));
    newest_first(&mut scan.captures);
    scan
}
fn preferred(candidate: &Capture, existing: &Capture) -> bool {
    let candidate_hex = is_hex_package(&candidate.folder);
    let existing_hex = is_hex_package(&existing.folder);
    (candidate_hex && !existing_hex)
        || (candidate_hex == existing_hex && candidate.folder < existing.folder)
}
fn dedupe(captures: Vec<Capture>) -> Vec<Capture> {
    let mut ids = std::collections::HashMap::new();
    let mut out = Vec::new();
    for capture in captures {
        let id = capture.data.id.clone();
        if id.is_empty() {
            out.push(capture);
            continue;
        }
        if let Some(&index) = ids.get(&id) {
            if preferred(&capture, &out[index]) {
                out[index] = capture;
            }
        } else {
            ids.insert(id, out.len());
            out.push(capture);
        }
    }
    out
}
fn newest_first(captures: &mut [Capture]) {
    captures.sort_by(|a, b| {
        b.data
            .created_at_ms
            .cmp(&a.data.created_at_ms)
            .then(a.folder.cmp(&b.folder))
    });
}
pub fn scan_roots(roots: &[PathBuf]) -> Scan {
    let mut merged = Scan::default();
    for root in roots {
        let mut scan = scan(root);
        merged.captures.append(&mut scan.captures);
        merged.errors.append(&mut scan.errors);
    }
    merged.captures = dedupe(std::mem::take(&mut merged.captures));
    newest_first(&mut merged.captures);
    merged
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod notebook_tests;
