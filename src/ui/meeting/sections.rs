//! Navigation for the headings that the reading view renders.
use super::*;
use crate::ui::widgets::{menu_row, menu_surface, menu_trigger};
use rune::{Align, Side};
use rune_motion::{Beat, Curve, Easing, Sequence};

const CHIP: &str = "meeting-sections";
const GLOW: &[Beat] = &[Beat::new(0.0), Beat::new(0.66).opacity(0.0)];

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Section {
    key: String,
    label: String,
}

#[derive(Default)]
pub(super) struct Visit {
    scope: Option<(String, Tab, bool)>,
    selected: Option<Section>,
    pulse: u64,
    open: bool,
}

impl Visit {
    pub(super) fn sync(&mut self, m: &Meeting, tab: Tab, strip: bool) {
        if !self.matches(m, tab) {
            *self = Self {
                scope: Some((m.id.clone(), tab, m.has_summary())),
                ..Self::default()
            };
        }
        self.open &= !strip;
    }

    fn matches(&self, m: &Meeting, tab: Tab) -> bool {
        self.scope.as_ref().is_some_and(|(id, view, published)| {
            *id == m.id && *view == tab && *published == m.has_summary()
        })
    }

    pub(super) fn retain(&mut self, sections: &[Section]) {
        self.open &= sections.len() >= 3;
        self.selected = self.selected.take().filter(|item| sections.contains(item));
    }

    pub(super) fn heading(
        &self,
        sections: &mut Vec<Section>,
        label: &str,
        key: String,
        first: bool,
        p: Palette,
    ) -> El {
        let section = Section {
            key,
            label: label.into(),
        };
        let glow = self.selected.as_ref() == Some(&section);
        let heading = roles::COMPACT_SECTION
            .wrap(label, p.ink)
            .key(section.key.clone())
            .selectable(true);
        if !label.trim().is_empty() {
            sections.push(section);
        }
        col()
            .mt(if first { 0.0 } else { 20.0 })
            .mb(6.0)
            .children(glow.then(|| {
                el().key(format!("section-glow-{}", self.pulse))
                    .pin(Some(-4.0), Some(-2.0), Some(0.0), Some(-2.0))
                    .radius(4.0)
                    .fill(p.ink.alpha(0.06))
                    .sequence(Sequence::new(GLOW))
                    .a11y_hidden()
            }))
            .child(heading)
    }

    pub(super) fn marks(&self, sections: &[Section], p: Palette) -> El {
        let buttons = sections.iter().cloned().map(|section| {
            let active = self.selected.as_ref() == Some(&section);
            pressable(row().w_full().h(24.0).pl(10.0).items_center())
                .key(format!("section-mark-{}", section.key))
                .label(&section.label)
                .focus_selected(active)
                .focus_ring(1.0, -3.0, p.soft)
                .on_press_tx(move |s, tx| jump(s, tx, &section))
                .child(
                    el().w(if active { 24.0 } else { 13.0 })
                        .h(2.0)
                        .radius(1.0)
                        .fill(if active { p.ink.alpha(0.8) } else { p.mark })
                        .group_opacity(0.0, 1.0)
                        .a11y_hidden(),
                )
        });
        let marks = col()
            .min_h(0.0)
            .shrink()
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .role("list")
            .label("Sections")
            .focus_group()
            .children(buttons);
        let fade = Curve::tween(0.16, Easing::EASE_OUT);
        col()
            .h_full()
            .min_h(0.0)
            .justify_center()
            .hover_group()
            .hover_motion(fade, fade)
            .child(marks)
    }

    pub(super) fn chip(
        &self,
        sections: &[Section],
        k: &Kit,
        p: Palette,
        layer: &mut Vec<El>,
    ) -> El {
        let trigger = menu_trigger(
            pressable(chip("Sections", p)).key(CHIP).label("Sections"),
            self.open,
            p,
            |s| s.meeting_ui.sections.get_mut().open = true,
        )
        .hover(p.chip)
        .focus_ring(2.0, 1.0, p.accent);
        if self.open {
            let rows = col().children(sections.iter().cloned().map(|section| {
                menu_row(&section.label, p, None, None, None)
                    .key(format!("section-item-{}", section.key))
                    .on_press_tx(move |s: &mut State, tx| {
                        s.meeting_ui.sections.get_mut().open = false;
                        jump(s, tx, &section);
                    })
            }));
            menu_surface(
                k,
                p,
                layer,
                rows,
                (CHIP, CHIP, Side::Bottom, Align::Start, 244.0),
                |s| s.meeting_ui.sections.get_mut().open = false,
            );
        }
        trigger
    }
}

fn jump(s: &mut State, tx: &mut Tx, section: &Section) {
    let mut visit = s.meeting_ui.sections.borrow_mut();
    if !s.current().is_some_and(|m| visit.matches(m, s.meeting_tab)) {
        return;
    }
    visit.selected = Some(section.clone());
    visit.pulse += 1;
    tx.scroll_key_to(&section.key, 12.0);
}
