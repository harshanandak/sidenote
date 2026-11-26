use super::{prompt, transcript};

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const ROW_STEP_MS: u64 = 5_000;
const ROW_SPEECH_MS: u64 = 2_000;
const UNICODE_PROBE: &str = "café उत्पादन తెలుగు 🌒";

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

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedRow {
    start_ms: u64,
    end_ms: u64,
    source: String,
    speaker: String,
    text: String,
}
fn rows(ndjson: &str) -> Vec<GeneratedRow> {
    ndjson
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
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
    let probe = if row.is_multiple_of(19) {
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
            let rows = rows(&fixture.ndjson);
            let starts = fixture
                .parts
                .iter()
                .flat_map(|part| {
                    (part.start..part.end)
                        .step_by(ROW_STEP_MS as usize)
                        .take_while(move |start| *start + ROW_SPEECH_MS <= part.end)
                })
                .collect::<Vec<_>>();
            assert_eq!(rows.len(), starts.len());
            assert!(
                rows.iter()
                    .zip(&starts)
                    .all(|(row, start)| row.start_ms == *start)
            );
            let rendered = fixture.rendered();
            let lines = rendered.lines().collect::<Vec<_>>();
            assert_eq!(lines.len(), rows.len());
            for (index, (row, line)) in rows.iter().zip(&lines).enumerate() {
                assert!(row.start_ms < fixture.duration_ms);
                assert_eq!(row.end_ms, row.start_ms + ROW_SPEECH_MS);
                assert!(row.end_ms <= fixture.duration_ms);
                assert!(
                    fixture
                        .parts
                        .iter()
                        .any(|part| { row.start_ms >= part.start && row.end_ms <= part.end })
                );
                let microphone = index % 2 == 0;
                assert_eq!(row.source, if microphone { "microphone" } else { "system" });
                assert_eq!(row.speaker, if microphone { "You" } else { "Them" });
                assert_eq!(
                    *line,
                    format!(
                        "[{}] {} (English): {}",
                        stamp(row.start_ms),
                        row.speaker,
                        row.text
                    )
                );
            }
            assert!(rows.iter().any(|row| row.text.contains(UNICODE_PROBE)));
            for fact in fixture.facts.each() {
                assert!(
                    fixture.parts.iter().any(|part| {
                        fact.at_ms >= part.start && fact.at_ms + ROW_SPEECH_MS <= part.end
                    }),
                    "{}ms is not inside a recording part",
                    fact.at_ms
                );
                let matching = rows
                    .iter()
                    .filter(|row| row.text == fact.text)
                    .collect::<Vec<_>>();
                assert_eq!(matching.len(), 1);
                assert_eq!(matching[0].start_ms, fact.at_ms);
                assert_eq!(matching[0].end_ms, fact.at_ms + ROW_SPEECH_MS);
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
