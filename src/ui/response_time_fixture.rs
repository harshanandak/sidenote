//! Synthetic scale fixture; reachable only through the isolated headless scene.
use super::{State, core};

pub fn scene(s: &mut State) {
    if !super::isolated() || rune::tooling::headless_run().is_none() {
        return;
    }
    s.go_meetings();
    s.meetings = (0..2000)
        .map(|index| {
            let title = format!("Synthetic meeting {index}");
            core::Meeting {
                folder: s.store.root.join(format!("response-time-fixture/{index}")),
                device: None,
                id: format!("synthetic-{index}"),
                title: title.clone(),
                diary_title: title.clone(),
                summary: core::Summary {
                    title,
                    context: vec!["Synthetic latency fixture.".into()],
                    ..Default::default()
                },
                summary_raw: "Synthetic latency fixture.".into(),
                draft_raw: String::new(),
                transcript_raw: String::new(),
                routed: None,
                routed_revision: String::new(),
                summary_language: None,
                notes: String::new(),
                lines: Vec::new(),
                started: None,
                minutes: Some(60),
                export_path: None,
                search: format!("synthetic meeting {index}"),
            }
        })
        .collect();
    s.meetings[0].lines = (0..10000)
        .map(|index| core::Line {
            at: format!("{:02}:{:02}", index / 60, index % 60),
            seconds: Some(index),
            who: "Synthetic speaker".into(),
            language: "English".into(),
            text: format!("Synthetic transcript line {index}."),
        })
        .collect();
    s.meetings[0].transcript_raw = "Synthetic transcript fixture.".into();
    s.search = rune::TextEdit::default();
    s.filter = super::nav::Filter::All;
    s.rebuild_nav();
    s.refresh_list();
}
