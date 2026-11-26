use regex::Regex;
use std::{io::Read, ops::Range, sync::LazyLock};

use crate::core::meetings::milliseconds;

// Leave room for instructions and output in the backend's context.
pub(super) const WINDOW_BYTES: usize = super::prompt::TRANSCRIPT_CAP / 8;
const SPAN_BYTES: usize = 4096;

#[derive(Clone, Copy)]
struct Span<'a> {
    offset: usize,
    raw: &'a str,
    head: &'a str,
}

impl Span<'_> {
    fn end(self) -> usize {
        self.offset + self.raw.len()
    }

    fn bytes(self) -> usize {
        self.head.len() + self.raw.len() + usize::from(!self.raw.ends_with('\n'))
    }

    fn append(self, text: &mut String) {
        text.push_str(self.head);
        text.push_str(self.raw);
        if !self.raw.ends_with('\n') {
            text.push('\n');
        }
    }
}

fn continuation_head(line: &str) -> &str {
    static LABEL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^ \S+ \([^()\r\n]+\): ").unwrap());
    let head = &line[..line.floor_char_boundary(SPAN_BYTES / 2)];
    if let Some((stamp, rest)) = head.strip_prefix('[').and_then(|s| s.split_once(']'))
        && milliseconds(stamp).is_some()
    {
        let stamp_end = stamp.len() + 2;
        return LABEL.find(rest).map_or_else(
            || &head[..stamp_end + usize::from(rest.starts_with(' '))],
            |label| &head[..stamp_end + label.end()],
        );
    }
    ""
}

fn spans(transcript: &str) -> impl Iterator<Item = Span<'_>> {
    transcript
        .split_inclusive('\n')
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        })
        .flat_map(|(mut offset, mut line)| {
            let head = if line.len() >= SPAN_BYTES {
                continuation_head(line)
            } else {
                ""
            };
            let mut prefix = "";
            std::iter::from_fn(move || {
                if line.is_empty() {
                    return None;
                }
                let room = SPAN_BYTES - prefix.len();
                let end = if line.len() + usize::from(!line.ends_with('\n')) <= room {
                    line.len()
                } else {
                    line.floor_char_boundary(room - 1)
                };
                let (raw, rest) = line.split_at(end);
                let span = Span {
                    offset,
                    raw,
                    head: prefix,
                };
                offset += end;
                line = rest;
                prefix = head;
                Some(span)
            })
        })
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Window {
    pub primary: Range<usize>,
    pub overlap: Range<usize>,
    pub text: String,
}

/// Ranges refer to the original UTF-8 bytes. The text can repeat a row heading
/// and add a line break when a single row needs to be split.
pub(super) fn windows(transcript: &str) -> impl Iterator<Item = Window> + '_ {
    let mut spans = spans(transcript).peekable();
    let mut previous: Option<Span<'_>> = None;
    std::iter::from_fn(move || {
        let first = spans.next()?;
        let mut text = String::new();
        let overlap = if let Some(span) = previous {
            span.append(&mut text);
            span.offset..span.end()
        } else {
            0..0
        };
        first.append(&mut text);
        let mut last = first;
        while spans
            .peek()
            .is_some_and(|span| text.len() + span.bytes() <= WINDOW_BYTES)
        {
            last = spans.next().unwrap();
            last.append(&mut text);
        }
        previous = Some(last);
        Some(Window {
            primary: first.offset..last.end(),
            overlap,
            text,
        })
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Failure<E> {
    Empty,
    UnreadInput { window: usize },
    Backend { window: usize, error: E },
    BadOutput { window: usize, reason: &'static str },
}

/// Return results only after every window has been read in full and validated.
pub(super) fn process<T, E>(
    transcript: &str,
    mut backend: impl FnMut(&mut dyn Read) -> Result<String, E>,
    mut validate: impl FnMut(&str, &str) -> Result<T, &'static str>,
) -> Result<Vec<T>, Failure<E>> {
    if transcript.trim().is_empty() {
        return Err(Failure::Empty);
    }
    windows(transcript)
        .enumerate()
        .map(|(window, input)| {
            let mut unread = input.text.as_bytes();
            let raw = backend(&mut unread).map_err(|error| Failure::Backend { window, error })?;
            if !unread.is_empty() {
                return Err(Failure::UnreadInput { window });
            }
            validate(&raw, &input.text).map_err(|reason| Failure::BadOutput { window, reason })
        })
        .collect()
}
