use super::{
    Helper, JobState, NotesSettings,
    helper::{Failure, Run},
    prompt, queue, transcript,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const ROW_STEP_MS: u64 = 5_000;
const ROW_SPEECH_MS: u64 = 2_000;
const UNICODE_PROBE: &str = "café उत्पादन తెలుగు 🌒";
const ID: &str = "1793946600000-148";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Density {
    Sparse,
    Conversation,
    Dense,
}
impl Density {
    fn label(self) -> &'static str {
        ["sparse", "conversation", "dense"][self as usize]
    }
    fn words_per_row(self) -> usize {
        [5, 10, 32][self as usize]
    }
}

struct Fact {
    text: String,
    at_ms: u64,
}
struct Facts {
    start: Fact,
    middle: Fact,
    tail: Fact,
}
impl Facts {
    fn each(&self) -> [&Fact; 3] {
        [&self.start, &self.middle, &self.tail]
    }
}
struct Part {
    start: u64,
    end: u64,
    pause: u64,
}
struct Fixture {
    density: Density,
    duration_ms: u64,
    ndjson: String,
    parts: Vec<Part>,
    facts: Facts,
}
impl Fixture {
    fn rendered(&self) -> String {
        transcript::render(&self.ndjson)
    }
}

fn stamp(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}
fn fact(hours: u8, density: Density, name: &str, at_ms: u64) -> Fact {
    Fact {
        text: format!(
            "LM{hours}H-{}-{name} synthetic fact at {}: {UNICODE_PROBE} release token {name}-{hours}",
            density.label(),
            stamp(at_ms)
        ),
        at_ms,
    }
}
fn filler(row: usize, words: usize) -> String {
    const TERMS: [&str; 8] = [
        "synthetic",
        "agenda",
        "café",
        "उत्पादन",
        "తెలుగు",
        "decision",
        "window",
        "marker",
    ];
    let body = (0..words)
        .map(|word| TERMS[(row + word) % TERMS.len()])
        .collect::<Vec<_>>()
        .join(" ");
    let probe = if row % 19 == 0 {
        format!(" {UNICODE_PROBE}")
    } else {
        String::new()
    };
    format!("row-{row} {body}{probe}")
}
fn generate(hours: u8, density: Density) -> Fixture {
    let duration_ms = u64::from(hours) * HOUR_MS;
    let pause1 = duration_ms / 30;
    let pause2 = duration_ms / 60;
    let active = duration_ms - pause1 - pause2;
    let first = active * 5 / 12;
    let second = active / 3;
    let third = active - first - second;
    let mut parts = Vec::new();
    let mut at = 0;
    for (length, pause) in [(first, pause1), (second, pause2), (third, 0)] {
        parts.push(Part {
            start: at,
            end: at + length,
            pause,
        });
        at += length + pause;
    }
    let facts = Facts {
        start: fact(hours, density, "START", 30_000),
        middle: fact(hours, density, "MIDDLE", duration_ms / 2),
        tail: fact(hours, density, "TAIL", duration_ms - 2 * MINUTE_MS),
    };
    let mut ndjson = String::new();
    let mut row = 0;
    for part in &parts {
        let mut start = part.start;
        while start + ROW_SPEECH_MS <= part.end {
            let text = if start == facts.start.at_ms {
                facts.start.text.clone()
            } else if start == facts.middle.at_ms {
                facts.middle.text.clone()
            } else if start == facts.tail.at_ms {
                facts.tail.text.clone()
            } else {
                filler(row, density.words_per_row())
            };
            let microphone = row % 2 == 0;
            ndjson.push_str(
                &serde_json::json!({
                    "start_ms": start,
                    "end_ms": start + ROW_SPEECH_MS,
                    "source": if microphone { "microphone" } else { "system" },
                    "speaker": if microphone { "You" } else { "Them" },
                    "text": text,
                })
                .to_string(),
            );
            ndjson.push('\n');
            row += 1;
            start += ROW_STEP_MS;
        }
    }
    Fixture {
        density,
        duration_ms,
        ndjson,
        parts,
        facts,
    }
}
fn line_ms(line: &str) -> u64 {
    let stamp = line
        .strip_prefix('[')
        .and_then(|line| line.split_once(']'))
        .map(|(stamp, _)| stamp)
        .unwrap();
    let mut pieces = stamp
        .split(|c| c == ':' || c == '.')
        .map(|piece| piece.parse::<u64>().unwrap());
    pieces.next().unwrap() * 3_600_000
        + pieces.next().unwrap() * 60_000
        + pieces.next().unwrap() * 1000
        + pieces.next().unwrap()
}

#[test]
fn generated_fixtures_preserve_facts_unicode_density_and_joined_part_shape() {
    for hours in [1, 2, 3] {
        let cases = [
            generate(hours, Density::Sparse),
            generate(hours, Density::Conversation),
            generate(hours, Density::Dense),
        ];
        assert!(cases[0].ndjson.len() < cases[1].ndjson.len());
        assert!(cases[1].ndjson.len() < cases[2].ndjson.len());
        for fixture in cases {
            assert_eq!(fixture.ndjson, generate(hours, fixture.density).ndjson);
            assert_eq!(fixture.duration_ms, u64::from(hours) * HOUR_MS);
            assert_eq!(fixture.parts.len(), 3);
            assert_eq!(fixture.parts[2].end, fixture.duration_ms);
            assert!(
                fixture.parts[..2]
                    .iter()
                    .all(|part| part.pause >= MINUTE_MS)
            );
            assert!(
                fixture
                    .parts
                    .windows(2)
                    .all(|pair| pair[1].start == pair[0].end + pair[0].pause)
            );
            let rendered = fixture.rendered();
            let times = rendered.lines().map(line_ms).collect::<Vec<_>>();
            assert!(!times.is_empty());
            assert!(times.windows(2).all(|pair| pair[0] <= pair[1]));
            assert!(times.iter().all(|at| *at < fixture.duration_ms));
            for pair in fixture.parts.windows(2) {
                assert!(
                    times
                        .iter()
                        .all(|at| *at < pair[0].end || *at >= pair[1].start),
                    "a row landed in the pause after {}ms",
                    pair[0].end
                );
            }
            assert!(rendered.contains(UNICODE_PROBE));
            for fact in fixture.facts.each() {
                assert!(
                    fixture.parts.iter().any(|part| {
                        fact.at_ms >= part.start && fact.at_ms + ROW_SPEECH_MS <= part.end
                    }),
                    "{}ms is not inside a recording part",
                    fact.at_ms
                );
                assert_eq!(rendered.matches(&fact.text).count(), 1);
                assert!(rendered.contains(&format!("[{}]", stamp(fact.at_ms))));
            }
            assert!(fixture.facts.tail.at_ms >= fixture.duration_ms - 5 * MINUTE_MS);
            let chars = rendered.chars().count();
            match fixture.density {
                Density::Sparse => assert!(chars < prompt::TRANSCRIPT_CAP),
                Density::Dense if fixture.duration_ms >= 2 * HOUR_MS => {
                    assert!(chars > prompt::TRANSCRIPT_CAP)
                }
                _ => {}
            }
        }
    }
}

fn settings() -> NotesSettings {
    NotesSettings {
        helper: Some(Helper::Claude),
        enabled_at_ms: Some(1),
        ..NotesSettings::default()
    }
}
fn write_fixture(root: &Path, fixture: &Fixture) -> PathBuf {
    let folder = root.join("meetings").join(ID);
    fs::create_dir_all(&folder).unwrap();
    let created = 1_793_946_600_000_i64;
    fs::write(
        folder.join("manifest.json"),
        serde_json::json!({
            "schema_version": 1,
            "status": "complete",
            "title": "Synthetic long meeting",
            "created_at_ms": created,
            "duration_ms": fixture.duration_ms,
            "ended_at_ms": created + fixture.duration_ms as i64,
        })
        .to_string(),
    )
    .unwrap();
    fs::write(folder.join("transcript.ndjson"), &fixture.ndjson).unwrap();
    fs::write(
        folder.join("quick-title.json"),
        r#"{"title":"Synthetic long meeting"}"#,
    )
    .unwrap();
    folder
}

struct TailHelper {
    tail_fact: String,
    tail_stamp: String,
    prompts: Vec<String>,
}
impl TailHelper {
    fn summary(&self, saw_tail: bool) -> String {
        let text = if saw_tail {
            self.tail_fact.as_str()
        } else {
            "tail fact absent from helper input"
        };
        serde_json::json!({
            "title": "Synthetic long meeting",
            "context": [text],
            "topics": [],
            "todo": [{
                "ref": "t1",
                "title": "Record the tail fact",
                "description": text,
                "owner": "You",
                "at": [self.tail_stamp.as_str()]
            }],
            "to_settle": []
        })
        .to_string()
    }
}
impl Run for TailHelper {
    fn installed(&self, _: Helper) -> bool {
        true
    }
    fn run(&mut self, _: Helper, _: &str, prompt: String, _: Duration) -> Result<String, Failure> {
        let first = self.prompts.is_empty();
        let saw_tail = first && prompt.contains(&self.tail_fact);
        self.prompts.push(prompt);
        Ok(if first {
            self.summary(saw_tail)
        } else {
            r#"{"meeting_project":null,"tasks":[{"i":0,"project":"Delivery","new":true}]}"#.into()
        })
    }
}

#[test]
#[ignore = "main returns TooLong before the notes helper; enable with the bounded-input fix"]
fn dense_long_meeting_tail_fact_reaches_notes_and_summary() {
    let root = tempfile::TempDir::new().unwrap();
    let fixture = generate(3, Density::Dense);
    let folder = write_fixture(root.path(), &fixture);
    let settings = settings();
    let meetings = root.path().join("meetings");
    queue::enqueue_in(root.path(), &meetings, ID, None, &settings).unwrap();
    let mut helper = TailHelper {
        tail_fact: fixture.facts.tail.text.clone(),
        tail_stamp: stamp(fixture.facts.tail.at_ms),
        prompts: Vec::new(),
    };
    queue::drain(root.path(), &meetings, &mut helper, || Ok(settings.clone())).unwrap();
    let state = queue::read(root.path(), ID).unwrap().unwrap().state;
    let tail_in_prompt = helper
        .prompts
        .first()
        .is_some_and(|prompt| prompt.contains(&fixture.facts.tail.text));
    let summary = fs::read_to_string(folder.join("english-summary.json")).unwrap_or_default();
    let tail_in_summary = summary.contains(&fixture.facts.tail.text);
    assert!(
        matches!(&state, JobState::Done { .. }) && tail_in_prompt && tail_in_summary,
        "long-meeting tail regression: state={state:?}, helper_calls={}, tail_in_prompt={tail_in_prompt}, tail_in_summary={tail_in_summary}",
        helper.prompts.len()
    );
}
