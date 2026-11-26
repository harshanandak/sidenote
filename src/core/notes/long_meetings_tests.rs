use super::{
    long_meetings::{self, Failure, WINDOW_BYTES},
    prompt, schema, transcript,
};

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

#[test]
fn every_generated_span_reaches_the_backend_with_its_original_timestamp() {
    use crate::core::meetings::milliseconds;

    for hours in [1, 2, 3] {
        for density in [Density::Sparse, Density::Conversation, Density::Dense] {
            let fixture = generate(hours, density);
            let input = fixture.rendered();
            let windows: Vec<_> = long_meetings::windows(&input).collect();
            assert!(windows.len() > 1, "{hours}h {}", density.label());
            assert_eq!(long_meetings::windows(&input).collect::<Vec<_>>(), windows);

            let mut cursor = 0;
            let mut expected_overlap = 0..0;
            for window in &windows {
                assert_eq!(window.primary.start, cursor);
                assert!(window.primary.end > cursor);
                assert_eq!(window.overlap, expected_overlap);
                let primary = &input[window.primary.clone()];
                let overlap = &input[window.overlap.clone()];
                assert!(window.text.len() <= WINDOW_BYTES);
                assert_eq!(window.text, [overlap, primary].concat());
                assert!(primary.starts_with('[') && primary.ends_with('\n'));
                cursor = window.primary.end;
                let last = primary.lines().last().unwrap();
                expected_overlap = cursor - last.len() - 1..cursor;
            }
            assert_eq!(cursor, input.len());

            let mut index = 0;
            let outputs = long_meetings::process(
                &input,
                |reader| {
                    let mut received = String::new();
                    reader.read_to_string(&mut received).unwrap();
                    assert_eq!(received, windows[index].text);
                    index += 1;
                    let points: Vec<_> = received
                        .lines()
                        .filter_map(|line| {
                            let (at, text) = line.strip_prefix('[')?.split_once(']')?;
                            assert!(milliseconds(at).unwrap() < fixture.duration_ms);
                            let fact = fixture
                                .facts
                                .each()
                                .into_iter()
                                .find(|f| text.contains(&f.text))?;
                            Some(serde_json::json!({"type": "fact", "text": fact.text, "at": [at]}))
                        })
                        .collect();
                    Ok::<_, &'static str>(
                        serde_json::json!({
                            "title": "Synthetic meeting",
                            "context": [],
                            "topics": [{"title": "Facts", "points": points}],
                            "todo": [],
                            "to_settle": []
                        })
                        .to_string(),
                    )
                },
                |raw, chunk| {
                    schema::parse(
                        raw,
                        "1-1",
                        chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                        chunk,
                        &[],
                    )
                },
            )
            .unwrap();
            assert_eq!(index, windows.len());
            assert_eq!(outputs.len(), windows.len());
            for fact in fixture.facts.each() {
                let at = stamp(fact.at_ms);
                let points = outputs
                    .iter()
                    .flat_map(|summary| &summary.topics[0].points)
                    .filter(|point| point.text == fact.text)
                    .collect::<Vec<_>>();
                assert!(!points.is_empty());
                for point in points {
                    assert_eq!(point.at, [at.as_str()]);
                }
            }
        }
    }
}

#[test]
fn oversized_row_continues_on_utf8_boundaries_with_the_same_timestamp() {
    const HEAD: &str = "[00:05:04.000] You (English): ";
    let input = format!(
        "{HEAD}{}{}\n",
        "a".repeat(4094 - HEAD.len()),
        "🌒éक".repeat(WINDOW_BYTES / 3)
    );
    let mut reconstructed = HEAD.to_owned();
    let mut cursor = 0;
    for window in long_meetings::windows(&input) {
        assert_eq!(window.primary.start, cursor);
        assert_eq!(window.overlap.end, cursor);
        assert!(input.is_char_boundary(window.primary.end));
        assert!(input.is_char_boundary(window.overlap.start));
        assert!(window.text.len() <= WINDOW_BYTES);
        let mut lines = window.text.lines();
        if cursor == 0 {
            assert_eq!(lines.clone().next().unwrap().len(), 4094);
        } else {
            let overlap = &input[window.overlap.clone()];
            assert_eq!(lines.next().unwrap().strip_prefix(HEAD).unwrap(), overlap);
        }
        let primary = &input[window.primary.clone()];
        let start = reconstructed.len();
        for line in lines {
            reconstructed.push_str(line.strip_prefix(HEAD).unwrap());
        }
        assert_eq!(
            &reconstructed[if cursor == 0 { 0 } else { start }..],
            primary.trim_end_matches('\n')
        );
        cursor = window.primary.end;
    }
    reconstructed.push('\n');
    assert_eq!(reconstructed, input);
}

#[test]
fn unread_input_malformed_output_and_context_refusal_never_complete() {
    for hours in [1, 2, 3] {
        for density in [Density::Sparse, Density::Conversation, Density::Dense] {
            let input = generate(hours, density).rendered();
            let count = long_meetings::windows(&input).count();
            for (stop, failure) in [
                (0, Failure::UnreadInput { window: 0 }),
                (
                    count / 2,
                    Failure::BadOutput {
                        window: count / 2,
                        reason: "invalid chunk JSON",
                    },
                ),
                (
                    count - 1,
                    Failure::Backend {
                        window: count - 1,
                        error: "context limit",
                    },
                ),
            ] {
                let mut calls = 0;
                let result = long_meetings::process(
                    &input,
                    |reader| {
                        let current = calls;
                        calls += 1;
                        if current == stop && matches!(&failure, Failure::UnreadInput { .. }) {
                            reader.read_exact(&mut [0; 1]).unwrap();
                            return Ok(r#"{"title":"valid"}"#.into());
                        }
                        reader.read_to_string(&mut String::new()).unwrap();
                        if current == stop {
                            return match &failure {
                                Failure::Backend { error, .. } => Err(*error),
                                _ => Ok(r#"{"title":"truncated""#.into()),
                            };
                        }
                        Ok(r#"{"title":"valid"}"#.into())
                    },
                    |raw, _| {
                        serde_json::from_str::<std::collections::BTreeMap<String, String>>(raw)
                            .map_err(|_| "invalid chunk JSON")
                    },
                );
                assert_eq!(result, Err(failure));
                assert_eq!(calls, stop + 1);
            }
        }
    }
}
