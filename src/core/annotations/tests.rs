use super::*;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
use std::{
    fs::File,
    sync::atomic::{AtomicUsize, Ordering},
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/annotations/synthetic.scrawl")
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let folder = std::env::temp_dir().join(format!(
            "sidenote-annotations-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&folder).unwrap();
        Self(folder)
    }
    fn bundle(&self, name: &str, json: &str) -> PathBuf {
        let folder = self.0.join(format!("{name}.scrawl"));
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("scrawl.json"), json).unwrap();
        folder
    }
    fn hex_bundle(&self, name: &str, json: &str) -> PathBuf {
        let folder = self.0.join(format!("{name}.hexannotation"));
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("annotation.json"), json).unwrap();
        folder
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn parses_fixture_and_preserves_mark_numbers_while_ordering_frames() {
    let capture = read_capture(&fixture()).unwrap();
    assert_eq!(capture.data.marks.len(), 3);
    assert_eq!(capture.word_count(), 12);
    assert_eq!(capture.app(), "Northwind Board");
    assert_eq!(
        capture
            .data
            .keyframes
            .iter()
            .map(|f| f.index)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    for (i, mark) in capture.data.marks.iter().enumerate() {
        assert_eq!(
            capture.frame_for_mark(mark).unwrap().index,
            if i < 2 { 1 } else { 2 }
        );
    }
    assert!(
        capture
            .frame_path(&capture.data.keyframes[0])
            .unwrap()
            .ends_with("frames/frame-01.png")
    );
}

#[test]
fn tolerates_missing_optional_and_unknown_fields() {
    let temp = Temp::new();
    let folder = temp.bundle(
        "minimal",
        r#"{"id":"minimal","newField":42,"marks":[{}],"keyframes":[{"index":7,"unknown":true}]}"#,
    );
    let c = read_capture(&folder).unwrap();
    assert_eq!(c.app(), "Screen capture");
    assert_eq!(c.title(), "Untitled capture");
    assert_eq!(c.word_count(), 0);
    assert!(c.words_for_mark(&c.data.marks[0]).is_empty());
    assert!(c.frame_for_mark(&c.data.marks[0]).is_none());
    assert!(c.frame_path(&c.data.keyframes[0]).is_err());
    let nulls: Scrawl = serde_json::from_str(r#"{"target":{"app":null,"windowTitle":null},"marks":[{"frame":null,"boxNorm":null,"description":null}]}"#).unwrap();
    assert!(nulls.marks[0].frame.is_none());
    let pending: Scrawl =
        serde_json::from_str(r#"{"target":null,"transcript":null,"marks":null,"keyframes":null}"#)
            .unwrap();
    assert!(pending.marks.is_empty());
    assert!(pending.transcript.words.is_empty());
}

#[test]
fn word_window_includes_overlapping_words_at_both_three_second_boundaries() {
    assert_eq!(WORD_WINDOW_SECONDS, 3.0);
    let mut c = read_capture(&fixture()).unwrap();
    let mark = Mark {
        t0: 6.0,
        t1: 7.0,
        ..Default::default()
    };
    c.data.transcript.words = [
        (2.0, 2.99, "before"),
        (2.9, 3.0, "left"),
        (6.0, 6.5, "during"),
        (9.9, 10.0, "right."),
        (10.01, 11.0, "after"),
    ]
    .into_iter()
    .map(|(t0, t1, text)| Word {
        t0,
        t1,
        text: text.into(),
    })
    .collect();
    assert_eq!(
        c.words_for_mark(&mark)
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>(),
        ["left", "during", "right."]
    );
    let early = Mark {
        t0: 0.2,
        t1: 0.3,
        ..Default::default()
    };
    assert_eq!(c.words_for_mark(&early).len(), 3);
}

#[test]
fn nearby_words_finish_a_sentence_or_segment_without_exceeding_eight_seconds() {
    let mut c = read_capture(&fixture()).unwrap();
    let words = c.words_for_mark(&c.data.marks[1]);
    assert_eq!(words.last().unwrap().text, "Friday.");
    c.data.transcript.words = vec![
        Word {
            t0: 2.0,
            t1: 3.0,
            text: "Start".into(),
        },
        Word {
            t0: 8.0,
            t1: 8.5,
            text: "middle".into(),
        },
        Word {
            t0: 12.0,
            t1: 12.5,
            text: "finish".into(),
        },
        Word {
            t0: 15.5,
            t1: 16.0,
            text: "too late.".into(),
        },
    ];
    c.data.transcript.segments = vec![Word {
        t0: 2.0,
        t1: 12.5,
        text: "Start middle finish".into(),
    }];
    let mark = Mark {
        t0: 5.0,
        t1: 6.0,
        ..Default::default()
    };
    assert_eq!(c.words_for_mark(&mark).last().unwrap().text, "finish");
    c.data.transcript.segments.clear();
    assert_eq!(c.words_for_mark(&mark).last().unwrap().text, "finish");
    assert!(
        c.words_for_mark(&mark)
            .iter()
            .all(|w| w.t1 <= mark.t1 + WORD_END_LIMIT_SECONDS)
    );
}

#[test]
fn redaction_crop_and_downscale_preserve_original_and_opaque_pixels() {
    use super::media::*;
    let original = image::RgbaImage::from_pixel(3200, 1600, image::Rgba([210, 40, 60, 77]));
    let masks = [[0.25, 0.25, 0.5, 0.5]];
    let sent = bake(&downscale(&bake(&original, &masks)), &masks);
    assert_eq!((sent.width(), sent.height()), (1568, 784));
    assert_eq!(sent.get_pixel(784, 392).0, [0, 0, 0, 255]);
    for y in 196..588 {
        for x in 392..1176 {
            assert_eq!(sent.get_pixel(x, y).0, [0, 0, 0, 255]);
        }
    }
    assert!(original.pixels().all(|p| p.0 == [210, 40, 60, 77]));
    assert_eq!(original.get_pixel(1600, 800).0, [210, 40, 60, 77]);
    assert_eq!(
        crop_rect([0.2, 0.2, 0.4, 0.4], 1000, 500),
        Some([140, 70, 520, 260])
    );
    assert_eq!(
        crop_rect([0.0, 0.0, 1.0, 1.0], 100, 50),
        Some([0, 0, 100, 50])
    );
    assert!(crop_rect([f32::NAN, 0.0, 0.5, 0.5], 100, 50).is_none());
    assert_eq!(
        downscale(&image::RgbaImage::new(20, 10)).dimensions(),
        (20, 10)
    );
}

#[test]
fn approval_bakes_every_derivative_validates_schema_and_detects_changed_images() {
    use super::{media::*, note};
    let temp = Temp::new();
    let c = read_capture(&fixture()).unwrap();
    let original = fs::read(c.frame_path(&c.data.keyframes[0]).unwrap()).unwrap();
    let reviewed = vec![
        ReviewFrame {
            index: 1,
            included: true,
            masks: vec![[0.0, 0.0, 1.0, 1.0]],
        },
        ReviewFrame {
            index: 2,
            included: false,
            masks: Vec::new(),
        },
    ];
    let approved = approve(&c, &reviewed, &temp.0).unwrap();
    approved.verify().unwrap();
    assert_eq!(approved.approval.frames.len(), 1);
    for file in approved.approval.file_hashes.keys() {
        let image = image::open(approved.folder.join(file)).unwrap().to_rgba8();
        assert!(image.pixels().all(|p| p.0 == [0, 0, 0, 255]));
    }
    assert_eq!(
        fs::read(c.frame_path(&c.data.keyframes[0]).unwrap()).unwrap(),
        original
    );
    let frame = &approved.approval.frames[0];
    let prompt = note::frame_prompt(frame);
    assert!(prompt.contains("mark_number"));
    assert!(prompt.contains("Workshop navigation"));
    assert!(prompt.contains("Never invent names or numbers"));
    let note = note::synthetic_note(&approved);
    note::validate(&note, &approved).unwrap();
    assert!(note::notebook_prompt(&approved, &[]).contains("keyframe_reason"));
    let mut bad = note.clone();
    bad["figures"][0]["callouts"][0]["mark_number"] = serde_json::json!(999);
    assert!(note::validate(&bad, &approved).is_err());
    let mut bad = note.clone();
    bad["figures"][0]["callouts"][0]["mark_number"] = serde_json::json!(1.0);
    assert!(note::validate(&bad, &approved).is_err());
    let mut bad = note.clone();
    bad["unexpected"] = serde_json::json!(true);
    assert!(note::validate(&bad, &approved).is_err());
    let mut bad = note.clone();
    bad["figures"][0]["callouts"][0]["evidence_refs"] = serde_json::json!(["unknown"]);
    assert!(note::validate(&bad, &approved).is_err());
    fs::write(approved.folder.join(&frame.original), "changed").unwrap();
    assert!(approved.verify().is_err());
}

#[test]
fn stale_job_and_cancel_cannot_publish_over_new_job() {
    let epochs = super::generation::Epoch::default();
    let old = epochs.begin();
    assert!(old.current());
    let current = epochs.begin();
    assert!(!old.current());
    assert!(old.check().is_err());
    assert!(current.current());
    current.cancel.stop();
    assert!(current.check().is_err());
    let fresh = epochs.begin();
    epochs.invalidate();
    assert!(!fresh.current());
}

#[test]
fn real_claude_synthetic_note_matches_the_current_schema_and_evidence() {
    let temp = Temp::new();
    let capture = read_capture(&fixture()).unwrap();
    let review: Vec<_> = capture
        .data
        .keyframes
        .iter()
        .enumerate()
        .map(|(i, f)| super::media::ReviewFrame {
            index: f.index,
            included: true,
            masks: if i == 0 {
                vec![super::media::SYNTHETIC_MASK]
            } else {
                Vec::new()
            },
        })
        .collect();
    let approved = super::media::approve(&capture, &review, &temp.0).unwrap();
    let value = serde_json::from_str(include_str!(
        "../../../tools/ui-fixtures/annotation-note.json"
    ))
    .unwrap();
    super::note::validate(&value, &approved).unwrap();
}

#[test]
fn image_transports_use_chosen_model_and_actual_png_bytes() {
    use super::{generation::*, media::*};
    use crate::core::notes::Helper;
    let path = fixture().join("frames/frame-01.png");
    let input = claude_message("Only these approved pixels", std::slice::from_ref(&path)).unwrap();
    let input: serde_json::Value = serde_json::from_str(&input).unwrap();
    assert_eq!(
        input["message"]["content"][2]["source"]["media_type"],
        "image/png"
    );
    use base64::Engine;
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(
                input["message"]["content"][2]["source"]["data"]
                    .as_str()
                    .unwrap()
            )
            .unwrap(),
        fs::read(&path).unwrap()
    );
    let options = Options {
        helper: Helper::Claude,
        model: "chosen-model".into(),
    };
    let args = command_args(&options, "low", std::slice::from_ref(&path), false);
    assert!(args.windows(2).any(|a| a == ["--model", "chosen-model"]));
    assert!(args.windows(2).any(|a| a == ["--tools", ""]));
    assert!(args.windows(2).any(|a| a == ["--effort", "low"]));
    let args = command_args(
        &Options {
            helper: Helper::Codex,
            model: "chosen-codex-model".into(),
        },
        "medium",
        &[path],
        true,
    );
    assert!(args.contains(&"--image".into()));
    assert!(args.contains(&"--output-schema".into()));
    assert_eq!(
        super::generation::stream_preview(r#"{"title":"Workshop","summary":"Paper ca"#),
        "Workshop\n\nPaper ca"
    );
    assert_eq!(LONG_EDGE, 1568);
    let mut schema: serde_json::Value = serde_json::from_str(super::note::SCHEMA).unwrap();
    schema.as_object_mut().unwrap().remove("$schema");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&super::note::cli_schema()).unwrap(),
        schema
    );
}

#[test]
fn frame_references_match_indices_or_files_without_guessing_nearest_time() {
    let c = read_capture(&fixture()).unwrap();
    for reference in [
        FrameRef::Index(2),
        FrameRef::File("frames\\frame-02.png".into()),
        FrameRef::File("frame-02.png".into()),
    ] {
        assert_eq!(
            c.frame_for_mark(&Mark {
                frame: Some(reference),
                ..Default::default()
            })
            .unwrap()
            .index,
            2
        );
    }
    assert!(
        c.frame_for_mark(&Mark {
            frame: Some(FrameRef::Index(999)),
            ..Default::default()
        })
        .is_none()
    );
}

#[test]
fn only_local_frame_pngs_can_reach_the_image_decoder() {
    let c = read_capture(&fixture()).unwrap();
    for file in [
        "../outside.png",
        "frames/../../outside.png",
        "C:/outside.png",
        "recording.mp4",
        "audio.wav",
        "frames/../scrawl.json",
        "https://example.invalid/a.png",
        "frames/frame-01.png:stream.png",
    ] {
        assert!(
            c.frame_path(&Keyframe {
                file: Some(file.into()),
                ..Default::default()
            })
            .is_err(),
            "{file}"
        );
    }
}

#[test]
fn oversized_bundle_is_skipped_before_manifest_or_video_is_read() {
    let temp = Temp::new();
    let folder = temp.bundle("large", "invalid JSON proves the size check runs first");
    let video = folder.join("recording.mp4");
    let mut f = File::create(&video).unwrap();
    // Sparse allocation makes this a metadata-only 2 GB test, not a 2 GB fixture.
    #[cfg(windows)]
    use std::os::windows::io::AsRawHandle;
    #[cfg(windows)]
    unsafe {
        let mut returned = 0;
        assert_ne!(
            windows_sys::Win32::System::IO::DeviceIoControl(
                f.as_raw_handle(),
                0x000900c4,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut()
            ),
            0
        );
    }
    f.set_len(BUNDLE_MAX_BYTES + 1).unwrap();
    use std::io::{Seek, Write};
    f.rewind().unwrap();
    f.write_all(b"private media").unwrap();
    drop(f);
    let _locked = deny_media_read(&video);
    let error = read_capture(&folder).unwrap_err();
    assert!(error.to_string().contains("exceeds 2 GB"));
    let scan = scan(&temp.0);
    assert!(scan.captures.is_empty());
    assert_eq!(scan.errors.len(), 1);
}

#[test]
fn scan_is_newest_first_and_does_not_open_audio_or_video() {
    let temp = Temp::new();
    temp.bundle("older", r#"{"createdAtMs":1}"#);
    let newest = temp.bundle("newest", r#"{"createdAtMs":2}"#);
    temp.bundle("bad", "not json");
    fs::write(newest.join("audio.wav"), "must not read").unwrap();
    fs::write(newest.join("recording.mp4"), "must not read").unwrap();
    let before = fs::read(newest.join("scrawl.json")).unwrap();
    let _audio = deny_media_read(&newest.join("audio.wav"));
    let _video = deny_media_read(&newest.join("recording.mp4"));
    let scan = scan(&temp.0);
    assert_eq!(scan.captures.len(), 2);
    assert_eq!(scan.captures[0].data.created_at_ms, 2);
    assert_eq!(scan.errors.len(), 1);
    assert_eq!(fs::read(newest.join("scrawl.json")).unwrap(), before);
    assert!(scan.captures[0].data.transcript.words.is_empty());
    assert!(super::scan(&temp.0.join("missing")).captures.is_empty());
}

#[test]
fn hexannotation_package_reads_marks_frames_and_millisecond_times() {
    let temp = Temp::new();
    let folder = temp.hex_bundle(
        "chart-note",
        r#"{
          "formatVersion": 1,
          "annotation": {
            "version": 1,
            "id": "chart-note",
            "createdAtMs": 1777000000123,
            "durationMs": 3200,
            "target": {"mode":"window","app":"Example","windowTitle":"Chart"},
            "transcript": {
              "source":"hex",
              "text":"Move this",
              "segments":[{"t0":400,"t1":1050,"text":"Move this"}],
              "words":[{"t0":400,"t1":650,"text":"Move"},{"t0":700,"t1":1050,"text":"this"}]
            },
            "strokes": [{"t0":600,"t1":790,"pts":[1000,2000,0]}],
            "marks": [{
              "id":"m1",
              "strokes":[0],
              "t0":600,
              "t1":790,
              "boxNorm":[0.1,0.2,0.5,0.2],
              "shape":"line",
              "description":"underline",
              "frame":"frames/frame-01.png"
            }],
            "events": [],
            "keyframes": [{
              "index":1,
              "t":790,
              "reason":"mark",
              "file":"frames/frame-01.png",
              "event":"underline",
              "markId":"m1"
            }],
            "contactSheet":"contact-sheet.png",
            "video":"recording.mp4",
            "videoOffsetMs":120,
            "destination":{"session":"example-session"}
          }
        }"#,
    );
    fs::create_dir(folder.join("frames")).unwrap();
    fs::write(
        folder.join("frames/frame-01.png"),
        include_bytes!("../../../tests/fixtures/annotations/synthetic.scrawl/frames/frame-01.png"),
    )
    .unwrap();
    let capture = read_capture(&folder).unwrap();
    assert_eq!(capture.data.id, "chart-note");
    assert_eq!(capture.data.marks.len(), 1);
    assert_eq!(
        capture.data.marks[0].description.as_deref(),
        Some("underline")
    );
    assert!((capture.data.marks[0].t0 - 0.6).abs() < 0.000001);
    assert!((capture.data.transcript.words[1].t1 - 1.05).abs() < 0.000001);
    assert!((capture.data.keyframes[0].t - 0.79).abs() < 0.000001);
    assert_eq!(
        capture
            .frame_for_mark(&capture.data.marks[0])
            .unwrap()
            .index,
        1
    );
    assert!(
        capture
            .frame_path(&capture.data.keyframes[0])
            .unwrap()
            .ends_with("frames/frame-01.png")
    );
    let mut clean = capture.data.keyframes[0].clone();
    clean.file = Some("frames-clean/frame-01.png".into());
    fs::create_dir(folder.join("frames-clean")).unwrap();
    fs::write(
        folder.join("frames-clean/frame-01.png"),
        include_bytes!("../../../tests/fixtures/annotations/synthetic.scrawl/frames/frame-01.png"),
    )
    .unwrap();
    assert!(
        capture
            .frame_path(&clean)
            .unwrap()
            .ends_with("frames-clean/frame-01.png")
    );
    let scan = scan(&temp.0);
    assert_eq!(scan.captures.len(), 1);
    assert!(scan.errors.is_empty());
}

#[test]
fn hexannotation_without_manifest_is_skipped() {
    let temp = Temp::new();
    fs::create_dir(temp.0.join("partial.hexannotation")).unwrap();
    let scan = scan(&temp.0);
    assert!(scan.captures.is_empty());
    assert!(scan.errors.is_empty());
}

#[test]
fn hexannotation_future_format_is_skipped_with_a_clear_error() {
    let temp = Temp::new();
    temp.hex_bundle("future", r#"{"formatVersion":2}"#);
    let scan = scan(&temp.0);
    assert!(scan.captures.is_empty());
    assert_eq!(scan.errors.len(), 1);
    assert!(scan.errors[0].contains("formatVersion 2"));
    assert!(scan.errors[0].contains("future format"));
}

#[test]
fn hexannotation_duplicate_id_keeps_the_legacy_capture() {
    let legacy_root = Temp::new();
    let hex_root = Temp::new();
    let legacy = legacy_root.bundle("legacy", r#"{"id":"duplicate","createdAtMs":2}"#);
    hex_root.hex_bundle(
        "hex",
        r#"{"formatVersion":1,"annotation":{"id":"duplicate","createdAtMs":1}}"#,
    );
    let scan = scan_roots(&[hex_root.0.clone(), legacy_root.0.clone()]);
    assert_eq!(scan.captures.len(), 1);
    assert_eq!(scan.captures[0].folder, legacy);
    assert_eq!(scan.captures[0].data.created_at_ms, 2);
    assert!(scan.errors.is_empty());
}

#[test]
fn hexannotation_duplicate_keeps_legacy_saved_note_readable() {
    use super::{media, note, notebook};
    let store = Temp::new();
    let hex_root = Temp::new();
    let legacy = fixture();
    let root = legacy.parent().unwrap().to_owned();
    let initial = scan_roots(&[root.clone()]);
    let capture = initial.captures.into_iter().next().unwrap();
    let review = [media::ReviewFrame {
        index: 1,
        included: true,
        masks: Vec::new(),
    }];
    let approved = media::approve(&capture, &review, &store.0).unwrap();
    let saved = note::synthetic_note(&approved);
    note::save(&approved.folder, &saved).unwrap();
    media::publish_latest(&store.0, &approved).unwrap();
    hex_root.hex_bundle(
        "hex",
        r#"{"formatVersion":1,"annotation":{"id":"synthetic","createdAtMs":1}}"#,
    );
    let scan = scan_roots(&[hex_root.0.clone(), root]);
    assert_eq!(scan.captures.len(), 1);
    assert_eq!(scan.captures[0].folder, legacy);
    let (approved, loaded) = notebook::load_capture_note(&store.0, &scan.captures[0])
        .unwrap()
        .unwrap();
    note::validate(&loaded, &approved).unwrap();
    assert_eq!(loaded, saved);
}

#[test]
fn display_drag_at_two_sizes_bakes_exactly_the_same_source_pixels() {
    use super::media::*;
    let source = image::RgbaImage::from_pixel(800, 400, image::Rgba([90, 110, 130, 77]));
    for [w, h] in [[1000.0, 500.0], [400.0, 200.0]] {
        let b = box_from_display([w * 0.25, h * 0.25], [w * 0.75, h * 0.75], [w, h]).unwrap();
        assert_eq!(b, [0.25, 0.25, 0.5, 0.5]);
        let sent = bake(&source, &[b]);
        for (x, y, p) in sent.enumerate_pixels() {
            assert_eq!(
                *p,
                if (200..600).contains(&x) && (100..300).contains(&y) {
                    image::Rgba([0, 0, 0, 255])
                } else {
                    *source.get_pixel(x, y)
                }
            );
        }
        assert!(source.pixels().all(|p| p.0 == [90, 110, 130, 77]));
    }
}
#[test]
#[cfg_attr(
    not(windows),
    ignore = "requires immutable image pinning: Windows share-mode write exclusion"
)]
fn approval_seals_reviewed_bytes_and_send_pins_the_same_files() {
    use super::media::*;
    let temp = Temp::new();
    let c = read_capture(&fixture()).unwrap();
    let review = vec![ReviewFrame {
        index: 1,
        included: true,
        masks: vec![SYNTHETIC_MASK],
    }];
    let preview = prepare(&c, &review, &temp.0).unwrap();
    assert!(preview.verify().is_err());
    let before: Vec<_> = preview
        .approval
        .file_hashes
        .keys()
        .map(|n| (n.clone(), fs::read(preview.folder.join(n)).unwrap()))
        .collect();
    let approved = approve_preview(&c, preview, &review).unwrap();
    let pinned = approved
        .pin_images(&approved.images_for(&approved.approval.frames[0]))
        .unwrap();
    for (name, bytes) in &before {
        assert_eq!(fs::read(approved.folder.join(name)).unwrap(), *bytes);
        assert!(fs::write(approved.folder.join(name), b"changed").is_err());
    }
    drop(pinned);
    fs::write(approved.folder.join(&before[0].0), b"changed").unwrap();
    assert!(
        approved
            .pin_images(&approved.images_for(&approved.approval.frames[0]))
            .is_err()
    );
}

#[test]
fn prompts_request_plain_explanations_and_one_place_for_uncertainty() {
    let temp = Temp::new();
    let c = read_capture(&fixture()).unwrap();
    let approved = super::media::approve(
        &c,
        &[super::media::ReviewFrame {
            index: 1,
            included: true,
            masks: Vec::new(),
        }],
        &temp.0,
    )
    .unwrap();
    for prompt in [
        super::note::frame_prompt(&approved.approval.frames[0]),
        super::note::notebook_prompt(&approved, &[]),
    ] {
        assert!(prompt.contains("2 to 4"));
        assert!(prompt.contains("1 to 2"));
        assert!(prompt.contains("uncertainty field to null"));
        assert!(prompt.contains("once in open_questions"));
        assert!(prompt.contains("Never invent names or numbers"));
    }
    assert_eq!(
        super::media::downscale_to(&image::RgbaImage::new(3200, 1600), 1024).dimensions(),
        (1024, 512)
    );
    assert!(!super::generation::uses_frame_drafts(3));
    assert!(super::generation::uses_frame_drafts(4));
}

fn deny_media_read(path: &Path) -> Option<File> {
    #[cfg(windows)]
    {
        Some(
            fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(path)
                .unwrap(),
        )
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o0)).unwrap();
        assert!(File::open(path).is_err(), "fixture must forbid media reads");
        None
    }
}
