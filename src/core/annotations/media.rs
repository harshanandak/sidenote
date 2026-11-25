//! Only approved derivatives go to a runner; Hex bundles remain read-only.
use super::{Capture, IMAGE_MAX_BYTES};
use crate::core::{file_reads, store_io};
use image::{ImageFormat, Rgba, RgbaImage, imageops};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

pub const LONG_EDGE: u32 = 1568;
pub const CROP_PADDING: f32 = 0.15;
pub type BoxNorm = [f32; 4];
/// Shared by every synthetic scene and live proof, in source-image coordinates.
pub const SYNTHETIC_MASK: BoxNorm = [0.31, 0.06, 0.45, 0.08];
/// Input points are local to the displayed image, never the containing page.
pub fn box_from_display(start: [f32; 2], end: [f32; 2], size: [f32; 2]) -> Option<BoxNorm> {
    if !size.iter().all(|v| v.is_finite() && *v > 0.0) {
        return None;
    }
    let x0 = start[0].min(end[0]).clamp(0.0, size[0]);
    let y0 = start[1].min(end[1]).clamp(0.0, size[1]);
    let x1 = start[0].max(end[0]).clamp(0.0, size[0]);
    let y1 = start[1].max(end[1]).clamp(0.0, size[1]);
    normalized_box([
        x0 / size[0],
        y0 / size[1],
        (x1 - x0) / size[0],
        (y1 - y0) / size[1],
    ])
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewFrame {
    pub index: u64,
    pub included: bool,
    pub masks: Vec<BoxNorm>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedFrame {
    pub index: u64,
    pub original: String,
    pub marked: String,
    pub crops: BTreeMap<usize, String>,
    pub metadata: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Approval {
    pub capture_id: String,
    pub source_fingerprint: String,
    pub approved_at_ms: i64,
    pub frames: Vec<PreparedFrame>,
    pub review: Vec<ReviewFrame>,
    pub file_hashes: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovedCapture {
    pub folder: PathBuf,
    pub approval: Approval,
}
fn error(s: impl std::fmt::Display) -> io::Error {
    io::Error::other(s.to_string())
}
pub fn normalized_box(b: BoxNorm) -> Option<BoxNorm> {
    if !b.iter().all(|v| v.is_finite()) {
        return None;
    }
    let x = b[0].clamp(0.0, 1.0);
    let y = b[1].clamp(0.0, 1.0);
    let w = b[2].clamp(0.0, 1.0 - x);
    let h = b[3].clamp(0.0, 1.0 - y);
    (w > 0.0 && h > 0.0).then_some([x, y, w, h])
}
pub fn crop_rect(b: BoxNorm, width: u32, height: u32) -> Option<[u32; 4]> {
    let [x, y, w, h] = normalized_box(b)?;
    let left = ((x - w * CROP_PADDING).max(0.0) * width as f32).floor() as u32;
    let top = ((y - h * CROP_PADDING).max(0.0) * height as f32).floor() as u32;
    let right = (((x + w + w * CROP_PADDING).min(1.0) * width as f32).ceil() as u32).min(width);
    let bottom = (((y + h + h * CROP_PADDING).min(1.0) * height as f32).ceil() as u32).min(height);
    (right > left && bottom > top).then_some([left, top, right - left, bottom - top])
}
pub fn bake(image: &RgbaImage, masks: &[BoxNorm]) -> RgbaImage {
    let mut out = image.clone();
    for b in masks.iter().filter_map(|b| normalized_box(*b)) {
        let x = (b[0] * out.width() as f32).floor() as u32;
        let y = (b[1] * out.height() as f32).floor() as u32;
        let right = (((b[0] + b[2]) * out.width() as f32).ceil() as u32).min(out.width());
        let bottom = (((b[1] + b[3]) * out.height() as f32).ceil() as u32).min(out.height());
        for yy in y..bottom {
            for xx in x..right {
                out.put_pixel(xx, yy, Rgba([0, 0, 0, 255]));
            }
        }
    }
    out
}
pub fn downscale(image: &RgbaImage) -> RgbaImage {
    downscale_to(image, LONG_EDGE)
}
pub fn downscale_to(image: &RgbaImage, limit: u32) -> RgbaImage {
    let edge = image.width().max(image.height());
    if edge <= limit {
        return image.clone();
    }
    let ratio = limit.max(1) as f64 / edge as f64;
    imageops::resize(
        image,
        (image.width() as f64 * ratio).round().max(1.0) as u32,
        (image.height() as f64 * ratio).round().max(1.0) as u32,
        imageops::FilterType::Triangle,
    )
}
pub fn marked(image: &RgbaImage, marks: &[(usize, BoxNorm)], masks: &[BoxNorm]) -> RgbaImage {
    let mut out = image.clone();
    const DIGITS: [[u8; 7]; 10] = [
        [14, 17, 19, 21, 25, 17, 14],
        [4, 12, 4, 4, 4, 4, 14],
        [14, 17, 1, 2, 4, 8, 31],
        [30, 1, 1, 14, 1, 1, 30],
        [2, 6, 10, 18, 31, 2, 2],
        [31, 16, 16, 30, 1, 1, 30],
        [14, 16, 16, 30, 17, 17, 14],
        [31, 1, 2, 4, 8, 8, 8],
        [14, 17, 17, 14, 17, 17, 14],
        [14, 17, 17, 15, 1, 1, 14],
    ];
    for (number, b) in marks {
        let Some([x, y, w, h]) = normalized_box(*b) else {
            continue;
        };
        let left = (x * out.width() as f32) as u32;
        let top = (y * out.height() as f32) as u32;
        let right = (((x + w) * out.width() as f32).ceil() as u32).min(out.width());
        let bottom = (((y + h) * out.height() as f32).ceil() as u32).min(out.height());
        let ink = Rgba([30, 74, 116, 255]);
        for yy in top..bottom {
            for xx in left..right {
                if xx < left + 2 || xx + 2 >= right || yy < top + 2 || yy + 2 >= bottom {
                    out.put_pixel(xx, yy, ink);
                }
            }
        }
        let digits = number.to_string();
        let badge_w = (digits.len() as u32 * 12 + 6).max(22);
        for yy in top..(top + 22).min(out.height()) {
            for xx in left..(left + badge_w).min(out.width()) {
                out.put_pixel(xx, yy, ink);
            }
        }
        for (n, digit) in digits.bytes().enumerate() {
            for (yy, bits) in DIGITS[(digit - b'0') as usize].iter().enumerate() {
                for xx in 0..5 {
                    if bits & (1 << (4 - xx)) != 0 {
                        for dy in 0..2 {
                            for dx in 0..2 {
                                let px = left + 4 + n as u32 * 12 + xx * 2 + dx;
                                let py = top + 4 + yy as u32 * 2 + dy;
                                if px < out.width() && py < out.height() {
                                    out.put_pixel(px, py, Rgba([255, 255, 255, 255]));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    bake(&out, masks)
}
pub fn capture_key(c: &Capture) -> String {
    let folder = c.legacy_folder.as_ref().unwrap_or(&c.folder);
    blake3::hash(folder.to_string_lossy().as_bytes())
        .to_hex()
        .to_string()
}
pub fn note_folder(root: &Path, c: &Capture) -> PathBuf {
    root.join("annotations").join(capture_key(c))
}
pub fn source_fingerprint(c: &Capture) -> io::Result<String> {
    let (bytes, current) = super::read_manifest(&c.folder)?;
    let mut current = current;
    current
        .keyframes
        .sort_by(|a, b| a.t.total_cmp(&b.t).then(a.index.cmp(&b.index)));
    if serde_json::to_value(&current)? != serde_json::to_value(&c.data)? {
        return Err(error("Capture metadata changed. Refresh and review again."));
    }
    let mut hash = blake3::Hasher::new();
    hash.update(&bytes);
    for frame in &c.data.keyframes {
        hash.update(&file_reads::read(&c.frame_path(frame)?, IMAGE_MAX_BYTES)?);
    }
    Ok(hash.finalize().to_hex().to_string())
}
pub fn note_fingerprint_matches(c: &Capture, expected: &str) -> io::Result<bool> {
    if source_fingerprint(c)? == expected {
        return Ok(true);
    }
    let Some(folder) = &c.legacy_folder else {
        return Ok(false);
    };
    let legacy = super::read_capture(folder)?;
    Ok(source_fingerprint(&legacy)? == expected)
}
fn save_png(
    folder: &Path,
    name: &str,
    image: &RgbaImage,
    hashes: &mut BTreeMap<String, String>,
) -> io::Result<()> {
    let mut png = io::Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).map_err(error)?;
    let bytes = png.into_inner();
    store_io::atomic_write(&folder.join(name), &bytes)?;
    hashes.insert(name.into(), blake3::hash(&bytes).to_hex().to_string());
    Ok(())
}
/// Bake locally before review. This snapshot is not permission to send.
pub fn prepare(c: &Capture, review: &[ReviewFrame], root: &Path) -> io::Result<ApprovedCapture> {
    prepare_at_edge(c, review, root, LONG_EDGE)
}
pub(super) fn prepare_at_edge(
    c: &Capture,
    review: &[ReviewFrame],
    root: &Path,
    edge: u32,
) -> io::Result<ApprovedCapture> {
    let indices: std::collections::BTreeSet<_> = c.data.keyframes.iter().map(|f| f.index).collect();
    if indices.len() != c.data.keyframes.len() {
        return Err(error(
            "This capture has duplicate frame indices. Refresh it before review.",
        ));
    }
    if !review.iter().any(|f| f.included) {
        return Err(error("Choose at least one frame."));
    }
    if review
        .iter()
        .any(|r| !c.data.keyframes.iter().any(|f| f.index == r.index))
    {
        return Err(error("A reviewed frame no longer exists."));
    }
    let source = source_fingerprint(c)?;
    let folder = note_folder(root, c).join("approved").join(format!(
        "{}-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
        std::process::id()
    ));
    store_io::reject_reparse(&folder)?;
    fs::create_dir_all(&folder)?;
    let mut approval = Approval {
        capture_id: c.data.id.clone(),
        source_fingerprint: source,
        approved_at_ms: 0,
        frames: Vec::new(),
        review: review.to_owned(),
        file_hashes: BTreeMap::new(),
    };
    for frame in &c.data.keyframes {
        let Some(r) = review.iter().find(|r| r.index == frame.index && r.included) else {
            continue;
        };
        if r.masks.iter().any(|b| normalized_box(*b).is_none()) {
            return Err(error("A redaction box is invalid."));
        }
        let bytes = file_reads::read(&c.frame_path(frame)?, IMAGE_MAX_BYTES)?;
        let original = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
            .map_err(error)?
            .to_rgba8();
        let redacted = bake(&downscale_to(&bake(&original, &r.masks), edge), &r.masks);
        let marks: Vec<_> = c
            .data
            .marks
            .iter()
            .enumerate()
            .filter(|(_, m)| c.frame_for_mark(m).is_some_and(|f| f.index == frame.index))
            .collect();
        let boxes: Vec<_> = marks
            .iter()
            .filter_map(|(n, m)| m.box_norm.map(|b| (n + 1, b)))
            .collect();
        let original_name = format!("frame-{}-original.png", frame.index);
        let marked_name = format!("frame-{}-marked.png", frame.index);
        save_png(
            &folder,
            &original_name,
            &redacted,
            &mut approval.file_hashes,
        )?;
        save_png(
            &folder,
            &marked_name,
            &marked(&redacted, &boxes, &r.masks),
            &mut approval.file_hashes,
        )?;
        let mut crops = BTreeMap::new();
        for (number, b) in boxes {
            if let Some([x, y, w, h]) = crop_rect(b, redacted.width(), redacted.height()) {
                let name = format!("frame-{}-mark-{number}.png", frame.index);
                save_png(
                    &folder,
                    &name,
                    &imageops::crop_imm(&redacted, x, y, w, h).to_image(),
                    &mut approval.file_hashes,
                )?;
                crops.insert(number, name);
            }
        }
        let metadata = serde_json::json!({"capture_id":c.data.id,"app":c.app(),"window_title":c.title(),"keyframe_index":frame.index,"keyframe_reason":frame.reason,"keyframe_time":frame.t,"figure_id":format!("frame-{}",frame.index),"marks":marks.iter().map(|(n,m)|serde_json::json!({"mark_number":n+1,"mark_id":format!("{}:mark-{}",c.data.id,n+1),"boxNorm":m.box_norm.map(|b|b.map(|v| (v as f64*1_000_000.0).round()/1_000_000.0)),"description":m.description,"t0":m.t0,"t1":m.t1,"words":c.words_for_mark(m)})).collect::<Vec<_>>()});
        approval.frames.push(PreparedFrame {
            index: frame.index,
            original: original_name,
            marked: marked_name,
            crops,
            metadata,
        });
    }
    if source_fingerprint(c)? != approval.source_fingerprint {
        return Err(error("Capture changed during approval. Review it again."));
    }
    store_io::atomic_write(
        &folder.join("approval.json"),
        &serde_json::to_vec_pretty(&approval)?,
    )?;
    Ok(ApprovedCapture { folder, approval })
}
/// Seal the exact preview bytes; do not re-render or resize after approval.
pub fn approve_preview(
    c: &Capture,
    mut preview: ApprovedCapture,
    review: &[ReviewFrame],
) -> io::Result<ApprovedCapture> {
    preview.verify_preview()?;
    if preview.approval.review != review
        || source_fingerprint(c)? != preview.approval.source_fingerprint
    {
        return Err(error("Review changed. Review the baked images again."));
    }
    preview.approval.approved_at_ms = chrono::Utc::now().timestamp_millis();
    store_io::atomic_write(
        &preview.folder.join("approval.json"),
        &serde_json::to_vec_pretty(&preview.approval)?,
    )?;
    preview.verify()?;
    Ok(preview)
}
/// The synthetic proof uses the same preparation and sealing path as the UI.
pub fn approve(c: &Capture, review: &[ReviewFrame], root: &Path) -> io::Result<ApprovedCapture> {
    approve_preview(c, prepare(c, review, root)?, review)
}
impl ApprovedCapture {
    pub fn verify(&self) -> io::Result<()> {
        if self.approval.approved_at_ms <= 0 {
            return Err(error("These images have not been approved."));
        }
        self.verify_preview()
    }
    pub fn verify_preview(&self) -> io::Result<()> {
        let saved: Approval = serde_json::from_slice(&file_reads::read(
            &self.folder.join("approval.json"),
            16 * 1024 * 1024,
        )?)?;
        if serde_json::to_value(&saved)? != serde_json::to_value(&self.approval)? {
            return Err(error("Approval changed. Review and approve again."));
        }
        if self.approval.frames.is_empty() {
            return Err(error("Approval contains no frames."));
        }
        for frame in &self.approval.frames {
            for name in std::iter::once(&frame.original)
                .chain(std::iter::once(&frame.marked))
                .chain(frame.crops.values())
            {
                if !self.approval.file_hashes.contains_key(name) {
                    return Err(error("An image is not included in the approval hashes."));
                }
            }
        }
        for (name, expected) in &self.approval.file_hashes {
            if Path::new(name).components().count() != 1 || !name.ends_with(".png") {
                return Err(error("Invalid approved image path."));
            }
            let bytes = file_reads::read(&self.folder.join(name), IMAGE_MAX_BYTES)?;
            if blake3::hash(&bytes).to_hex().as_str() != expected {
                return Err(error("Approved images changed. Review and approve again."));
            }
        }
        Ok(())
    }
    #[cfg(windows)]
    pub fn pin_images(&self, images: &[PathBuf]) -> io::Result<Vec<fs::File>> {
        use std::io::Read;
        use std::os::windows::fs::OpenOptionsExt;
        self.verify()?;
        images
            .iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| error("Invalid image"))?;
                if path.parent() != Some(self.folder.as_path()) {
                    return Err(error("Unapproved image folder"));
                }
                let expected = self
                    .approval
                    .file_hashes
                    .get(name)
                    .ok_or_else(|| error("Unapproved image"))?;
                let mut file = fs::OpenOptions::new().read(true).share_mode(1).open(path)?;
                let mut bytes = Vec::new();
                (&mut file)
                    .take(IMAGE_MAX_BYTES + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() as u64 > IMAGE_MAX_BYTES
                    || blake3::hash(&bytes).to_hex().as_str() != expected
                {
                    return Err(error("The sent image differs from the baked review copy."));
                }
                Ok(file)
            })
            .collect()
    }
    pub fn images_for(&self, frame: &PreparedFrame) -> Vec<PathBuf> {
        std::iter::once(&frame.original)
            .chain(std::iter::once(&frame.marked))
            .chain(frame.crops.values())
            .map(|n| self.folder.join(n))
            .collect()
    }
}

pub fn publish_latest(root: &Path, a: &ApprovedCapture) -> io::Result<()> {
    let parent = a
        .folder
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| error("Invalid annotation folder"))?;
    if !parent.starts_with(root.join("annotations")) {
        return Err(error("Invalid annotation store"));
    }
    let leaf = a
        .folder
        .file_name()
        .ok_or_else(|| error("Invalid approval folder"))?
        .to_string_lossy();
    store_io::atomic_write(
        &parent.join("latest.json"),
        &serde_json::to_vec(&serde_json::json!({"approved_folder":leaf}))?,
    )
}

/// Windows share-mode handles deny replacement/writes while the model reads paths.
/// Unix advisory locks do not provide that guarantee to an unrelated writer.
pub fn immutable_image_pinning_supported() -> bool {
    cfg!(windows)
}

#[cfg(not(windows))]
impl ApprovedCapture {
    pub fn pin_images(&self, _images: &[PathBuf]) -> io::Result<Vec<fs::File>> {
        Err(crate::platform::unavailable())
    }
}

#[cfg(all(test, not(windows)))]
mod linux_capability_tests {
    #[test]
    fn immutable_pinning_is_unavailable_and_never_sends_mutable_images() {
        assert!(!super::immutable_image_pinning_supported());
        let root = tempfile::tempdir().unwrap();
        let capture = crate::core::annotations::read_capture(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/annotations/synthetic.scrawl"),
        )
        .unwrap();
        let review = vec![super::ReviewFrame {
            index: 1,
            included: true,
            masks: vec![],
        }];
        let approved = super::approve(&capture, &review, root.path()).unwrap();
        let error = approved
            .pin_images(&approved.images_for(&approved.approval.frames[0]))
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("UNAVAILABLE_ON_PLATFORM"));
    }
}
