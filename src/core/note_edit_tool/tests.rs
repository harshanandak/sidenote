use super::*;

struct Fixture {
    temp: tempfile::TempDir,
    folder: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("100-200");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(
            folder.join("english-summary.json"),
            json!({"title": "Café Café", "topics": [{"title":"Timing", "points":[{"text":"Friday"}]}]}).to_string(),
        ).unwrap();
        recording_guard::keep_in(temp.path(), "100-200").unwrap();
        Self { temp, folder }
    }

    fn turn(&self) -> Turn {
        Turn::new(
            self.temp.path(),
            &self.folder,
            "100-200",
            "Remove the repeat".into(),
            vec![Selection {
                field: Field::Title,
                shown: "Café Café".into(),
                range: 6..11,
            }],
        )
        .unwrap()
    }
}

#[test]
fn quote_prompt_contains_only_the_selected_line_and_its_exact_slice() {
    let fixture = Fixture::new();
    let turn = fixture.turn();
    let mut text = String::new();
    turn.append_prompt(&mut text);
    let (_, quoted) = text.split_once("<quoted>\n").unwrap();
    let (quoted, _) = quoted.split_once("\n</quoted>").unwrap();
    let block: serde_json::Value = serde_json::from_str(quoted).unwrap();
    assert_eq!(block["line_id"], "title");
    assert_eq!(block["quote"], "Café");
    assert_eq!(block["line"], "Café Café");
    assert!(!text.contains("Friday"));
}

#[test]
fn quote_rejects_stale_text_and_a_partial_unicode_character() {
    let fixture = Fixture::new();
    for (shown, range) in [("Stale title", 6..11), ("Café Café", 10..11)] {
        assert!(
            Turn::new(
                fixture.temp.path(),
                &fixture.folder,
                "100-200",
                "Fix it".into(),
                vec![Selection {
                    field: Field::Title,
                    shown: shown.into(),
                    range
                }],
            )
            .is_err()
        );
    }
}
