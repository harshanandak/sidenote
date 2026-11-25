use std::borrow::Cow;

const START: &str = "<tool";
const OPEN: &str = r#"<tool name="propose_note_edit">"#;
const CLOSE: &str = "</tool>";

pub(super) fn split(raw: &str, streaming: bool) -> (Cow<'_, str>, Vec<Option<&str>>) {
    let Some((prefix, mut rest)) = opener(raw) else {
        return (Cow::Borrowed(visible_tail(raw, streaming)), Vec::new());
    };
    let mut visible = prefix.to_owned();
    let mut calls = Vec::new();
    loop {
        let payload = rest.strip_prefix(&OPEN[START.len()..]);
        let block = payload.unwrap_or(rest);
        let Some((end, nested)) = close(block) else {
            calls.push(None);
            return (visible.into(), calls);
        };
        calls.push(payload.filter(|_| !nested).map(|json| json[..end].trim()));
        rest = &block[end + CLOSE.len()..];
        let Some((text, after)) = opener(rest) else {
            visible.push_str(visible_tail(rest, streaming));
            return (visible.into(), calls);
        };
        visible.push_str(text);
        rest = after;
    }
}

fn opener(text: &str) -> Option<(&str, &str)> {
    text.match_indices(START).find_map(|(start, _)| {
        let end = start + START.len();
        tool_boundary(&text[end..]).then(|| (&text[..start], &text[end..]))
    })
}

fn tool_boundary(tail: &str) -> bool {
    tail.starts_with(|ch: char| matches!(ch, '>' | '/') || ch.is_ascii_whitespace())
        || tail.is_empty()
}

fn visible_tail(text: &str, streaming: bool) -> &str {
    if streaming {
        for len in (1..START.len()).rev() {
            if let Some(prefix) = text.strip_suffix(&START[..len]) {
                return prefix;
            }
        }
    }
    text
}

fn close(block: &str) -> Option<(usize, bool)> {
    let mut depth = 1;
    let mut nested = false;
    let mut quoted = false;
    let mut escaped = false;
    for (i, byte) in block.bytes().enumerate() {
        if escaped {
            escaped = false;
        } else if quoted && byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            quoted = !quoted;
        } else if !quoted && byte == b'<' {
            let tail = &block[i..];
            if tail.starts_with(CLOSE) {
                depth -= 1;
                if depth == 0 {
                    return Some((i, nested));
                }
            } else if tail.strip_prefix(START).is_some_and(tool_boundary) {
                depth += 1;
                nested = true;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::note_edit_tool::visible_progress;

    #[test]
    fn streams_hide_partial_and_complete_tags() {
        for prefix in [
            "<",
            "<t",
            "<to",
            "<too",
            "<tool",
            OPEN,
            "<tool name=\"propose_note_edit\">{",
        ] {
            assert_eq!(split(&format!("Visible {prefix}"), true).0, "Visible ");
        }
        assert_eq!(split("x < y", true).0, "x < y");
        let raw = format!(r#"Before {OPEN}{{"text":"x \" </tool> y"}}{CLOSE} after"#);
        let (visible, calls) = split(&raw, false);
        assert_eq!(visible, "Before  after");
        assert_eq!(calls, [Some(r#"{"text":"x \" </tool> y"}"#)]);
    }

    #[test]
    fn nested_blocks_stay_hidden_at_every_byte_split() {
        let before = "Before ? ";
        let after = " Apr?s";
        let block = format!(r#"{OPEN}{{"x":{OPEN}{{}}{CLOSE},"replacement":"cach?"}}"#);
        for (outer_close, shown_after) in [("", ""), (CLOSE, after)] {
            let raw = format!("{before}{block}{outer_close}{after}");
            let hidden_end = raw.len() - shown_after.len();
            for byte in 0..=raw.len() {
                let end = raw.floor_char_boundary(byte);
                let expected = format!(
                    "{}{}",
                    &raw[..end.min(before.len())],
                    &raw[end.min(hidden_end)..end]
                );
                assert_eq!(visible_progress(&raw[..end]), expected, "byte {byte}");
            }
            let (visible, calls) = split(&raw, false);
            assert_eq!(visible, format!("{before}{shown_after}"));
            assert_eq!(calls, [None]);
        }
    }

    #[test]
    fn lookalike_tags_reappear_at_every_byte_split() {
        let before = "Use ";
        let raw = "Use <toolbox> for caf?. Keep the notes.";
        for byte in 0..=raw.len() {
            let end = raw.floor_char_boundary(byte);
            let expected = if (before.len() + 1..=before.len() + START.len()).contains(&end) {
                before
            } else {
                &raw[..end]
            };
            assert_eq!(visible_progress(&raw[..end]), expected, "byte {byte}");
        }
        assert_eq!(split(raw, false), (Cow::Borrowed(raw), Vec::new()));
    }

    #[test]
    fn malformed_wrapper_is_hidden_and_counts_as_failed() {
        let (visible, calls) = split("Answer <tool invalid>payload</tool><tool only", false);
        assert_eq!(visible, "Answer ");
        assert_eq!(calls, [None, None]);
    }
}
