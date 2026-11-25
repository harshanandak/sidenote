use super::*;
use crate::ui::{
    ask::DOCK_BUTTON,
    widgets::{ButtonDisabled, dot},
};
use sidenote::core::{
    self,
    meeting_parts::{self, Phase as SavedPhase},
};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum RecordingStatus {
    Recording,
    Paused,
    Unknown,
}

impl RecordingStatus {
    pub(in crate::ui) fn for_phase(s: &State, phase: Option<Phase>) -> Self {
        if phase == Some(Phase::Paused) {
            Self::Paused
        } else if s.hex_status_unknown() {
            Self::Unknown
        } else {
            Self::Recording
        }
    }

    pub(in crate::ui) fn label(self) -> &'static str {
        match self {
            Self::Recording => "Recording",
            Self::Paused => "Paused",
            Self::Unknown => "Recording status unknown",
        }
    }
}

fn session(s: &State) -> Option<meeting_parts::Meeting> {
    s.live
        .as_ref()
        .and_then(|l| l.session.as_ref())
        .filter(|m| s.selected.as_ref().is_none_or(|id| id == &m.id))
        .cloned()
        .or_else(|| {
            s.selected
                .as_deref()
                .and_then(|id| meeting_parts::read(&core::state_dir(), id).ok().flatten())
        })
}
fn is_recording(s: &State, m: &Option<meeting_parts::Meeting>) -> bool {
    m.as_ref().is_some_and(|m| m.phase == SavedPhase::Recording)
        || (s.selected.is_none() && s.live.as_ref().is_some_and(|l| l.phase == Phase::Recording))
}
/// Level E: the line over the notes of the meeting that is recording. The notes are written when
/// it pauses, so until then they are the last ones.
pub(in crate::ui) fn recording_note(s: &State, p: Palette) -> Option<El> {
    is_recording(s, &session(s)).then(|| {
        let unknown =
            RecordingStatus::for_phase(s, Some(Phase::Recording)) == RecordingStatus::Unknown;
        let words = if unknown {
            RecordingStatus::Unknown.label()
        } else {
            "Recording. These notes update when you pause."
        };
        row()
            .key("meeting-recording-note")
            .gap(8.0)
            .items_center()
            .mb(16.0)
            .child(dot(8.0, if unknown { p.warn } else { p.danger }))
            .child(roles::SMALL.wrap(words, p.faint))
    })
}
pub(in crate::ui) fn parts_control(s: &State, p: Palette) -> Option<El> {
    use crate::ui::{
        icons,
        widgets::{ButtonRole, icon_button},
    };
    let m = session(s);
    let recording = is_recording(s, &m);
    let live = s.on_live();
    let compact = s.app_w.get() - crate::ui::nav::width(s) < 600.0;
    let mut control = row()
        .key("meeting-recording-control")
        .items_center()
        .gap(2.0);
    let button = |name, key, glyph| {
        if compact {
            icon_button(glyph, name, key, ButtonRole::Ghost, p).size(DOCK_BUTTON, DOCK_BUTTON)
        } else {
            quiet_button(name, key, p).h(DOCK_BUTTON)
        }
    };
    if live {
        let open = s.live.as_ref().is_some_and(|l| l.panel_open);
        let label = if open {
            "Hide transcript"
        } else {
            "Show transcript"
        };
        control = control.child(
            button("Transcript", "meeting-transcript", &icons::AUDIO_LINES)
                .label(label)
                .help(label)
                .on_press(|s: &mut State| s.live_toggle_panel()),
        );
    }
    if !recording && !live && m.as_ref().is_none_or(|m| m.phase == SavedPhase::Ended) {
        return None;
    }
    if recording {
        // Two bars distinguish Pause here from Stop on other pages.
        let bar = |key: &'static str| el().key(key).w(3.0).h(9.0).radius(1.0).fill(p.ink);
        control = control
            .children((live && !s.narrow.get()).then(|| recording_status(s, p)))
            .child(
                pressable(
                    row()
                        .key("meeting-pause-bars")
                        .gap(2.0)
                        .items_center()
                        .justify_center()
                        .child(bar("meeting-pause-bar-0"))
                        .child(bar("meeting-pause-bar-1")),
                )
                .key("meeting-pause")
                .label("Pause recording")
                .size(DOCK_BUTTON, DOCK_BUTTON)
                .radius(DOCK_BUTTON / 2.0)
                .hover(p.button_hover)
                .center()
                .button_disabled(s.hex_record_busy())
                .on_press(|s: &mut State| s.meeting_pause()),
            );
    } else if m.as_ref().is_some_and(|m| m.phase != SavedPhase::Ended) {
        control = control.child(
            button("Resume", "meeting-resume", &icons::MIC)
                .button_disabled(s.hex_record_busy())
                .on_press(|s: &mut State| s.meeting_resume()),
        );
    }
    if let Some(m) = m
        && m.phase != SavedPhase::Ended
        && !m.parts.is_empty()
        && !recording
    {
        control = control.child(
            button(
                if m.stale() {
                    "Update notes"
                } else {
                    "Generate notes"
                },
                "meeting-generate",
                &icons::FILE_TEXT,
            )
            .button_disabled(s.hex_record_busy())
            .on_press(|s: &mut State| s.meeting_generate()),
        );
    }
    if recording
        && (live
            || s.live
                .as_ref()
                .and_then(|l| l.session.as_ref())
                .is_some_and(|m| s.selected.as_deref() == Some(m.id.as_str())))
    {
        let expected = super::super::hex::recording_id(s);
        let reason = super::super::hex::stop_blocked(s);
        let stop = button("Stop", "hex-stop", &icons::STOP);
        control = control.child(
            stop.label(
                reason.map_or_else(|| "Stop recording".into(), |why| format!("Stop ({why})")),
            )
            .help(
                reason.unwrap_or("Stop this recording part. Generate notes from the meeting page."),
            )
            .button_disabled(reason.is_some())
            .on_press(move |s: &mut State| s.hex_record_stop(expected.clone())),
        );
    }
    Some(control)
}

fn recording_status(s: &State, p: Palette) -> El {
    roles::META
        .line(crate::ui::nav::recording_words(s, false), p.ink)
        .key("live-recording-status")
}

pub(in crate::ui) fn status(s: &State, p: Palette) -> Option<El> {
    let session = session(s);
    if s.on_live() && s.narrow.get() && is_recording(s, &session) {
        return Some(recording_status(s, p));
    }
    let m = session?;
    if m.phase == SavedPhase::Ended {
        return None;
    }
    let phase = s
        .live
        .as_ref()
        .filter(|live| {
            live.session
                .as_ref()
                .is_some_and(|session| session.id == m.id)
        })
        .map(|live| live.phase)
        .unwrap_or(if m.phase == SavedPhase::Recording {
            Phase::Recording
        } else {
            Phase::Paused
        });
    if phase == Phase::Recording {
        return None;
    }
    let state = match phase {
        Phase::Starting => "Starting",
        Phase::Recording => "Recording",
        Phase::Paused => RecordingStatus::for_phase(s, Some(phase)).label(),
        Phase::Making => "Writing notes",
    };
    let mut c = col().gap(8.0).child(
        roles::META
            .line(
                format!("{state} \u{00b7} {} recorded", stamp(m.elapsed_ms() / 1000)),
                p.soft,
            )
            .key("meeting-recorded-time"),
    );
    if m.stale() {
        c = c.child(
            roles::SMALL
                .wrap("New audio since these notes", p.soft)
                .key("meeting-notes-stale"),
        );
    }
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{theme, view};
    use rune::{Env, Runtime, text::TextSystem};

    #[test]
    fn live_recording_shows_elapsed_time_beside_pause() {
        crate::test_paths::isolate();
        for (width, narrow) in [(1334.0, false), (390.0, true), (360.0, true)] {
            let previous = crate::ui::hex::SKIP_INSTALL_DETECT.replace(true);
            let mut state = State::new();
            crate::ui::hex::SKIP_INSTALL_DETECT.set(previous);
            crate::ui::scene_agents(&mut state, &["codex"]);
            let mut live = Live::scene(Phase::Recording, None);
            live.session = Some(meeting_parts::Meeting {
                phase: SavedPhase::Recording,
                parts: vec![meeting_parts::Part {
                    id: "1800000000000-1".into(),
                    started_ms: 1_800_000_000_000,
                    duration_ms: 754_000,
                }],
                ..Default::default()
            });
            state.live = Some(live);
            state.live_open = true;
            assert!(state.on_live());
            let mut rt = Runtime::with_text(
                state,
                view,
                (width, 900.0),
                Env::default(),
                TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
            );
            rt.settle_all();
            rt.frame(0.0);
            let elements = rt.elements();
            let keyed = |key| {
                elements
                    .iter()
                    .find(|element| element.key.as_deref() == Some(key))
                    .unwrap_or_else(|| panic!("missing {key} at {width} px"))
            };
            let status = keyed("live-recording-status");
            let [x, y, w, h] = status.rect;
            let pause = keyed("meeting-pause");
            assert_eq!(pause.label.as_deref(), Some("Pause recording"));
            assert!(rt.find("meeting-generate").is_none());
            assert_eq!(status.text.as_deref(), Some("Recording 12:34"));
            assert_eq!(h, roles::META.lh, "one line at {width} px");
            assert!(x >= 0.0 && x + w <= width);
            if narrow {
                assert!(y + h <= pause.rect[1]);
                assert!(
                    keyed("ask-input").rect[2] >= 90.0,
                    "Ask is squeezed at {width} px"
                );
            } else {
                assert!(x + w <= pause.rect[0]);
            }
        }
    }
}
