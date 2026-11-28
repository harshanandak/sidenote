//! Text roles: the few sizes, weights and greys the app reads in. See docs/design/typography.md.
//! The scale follows the v3 approval: section heading 15 semibold, note text 14.5 on 1.6,
//! tasks and chips 12.5, 28 between sections; page titles are capped in `theme::display`.
//! Pages use these instead of raw sizes, so every page keeps the same hierarchy.
use super::theme::{BODY, BODY_LH, Palette};
use rune::{Color, El, TextStyle, row, text};

/// A size, weight, line height and letter spacing. The colour comes from the caller's palette.
#[derive(Clone, Copy)]
pub struct Role {
    pub size: f32,
    pub weight: f32,
    pub lh: f32,
    pub tracking: f32,
    pub features: &'static str,
}

impl Role {
    const fn new(size: f32, weight: f32, lh: f32) -> Self {
        Self {
            size,
            weight,
            lh,
            tracking: 0.0,
            features: "",
        }
    }

    const fn tabular(self, tracking: f32) -> Self {
        Self {
            tracking,
            features: "'tnum' 1",
            ..self
        }
    }

    /// The same size in the title weight, for a name inside a dense row or a lead word.
    pub const fn bold(self) -> Role {
        Role {
            weight: 600.0,
            ..self
        }
    }

    /// The same size at 500, for controls and warnings that need a little weight.
    pub const fn medium(self) -> Role {
        Role {
            weight: 500.0,
            ..self
        }
    }

    /// The same size at 400.
    pub const fn regular(self) -> Role {
        Role {
            weight: 400.0,
            ..self
        }
    }

    pub fn style(self) -> TextStyle {
        TextStyle::new(self.size, self.weight)
            .lh(self.lh)
            .tracking(self.tracking)
            .features(self.features)
    }

    /// Wrapping text in this role.
    pub fn wrap(self, s: impl Into<String>, ink: Color) -> El {
        text(s).font(self.style()).color(ink).wrap()
    }

    /// A control caption containing a disclosure, back arrow or status mark.
    pub fn icon_label(self, s: impl Into<String>, ink: Color) -> El {
        super::icons::styled(s, self.style(), ink)
    }

    /// One line that ends in an ellipsis when it does not fit.
    pub fn line(self, s: impl Into<String>, ink: Color) -> El {
        text(s).font(self.style()).color(ink).truncate().min_w(0.0)
    }
}

/// Page sections: Decisions, To do, Needs attention, a settings group.
pub const SECTION: Role = Role::new(15.0, 600.0, 22.0);
/// Date headings and summaries in narrow reading views.
pub const COMPACT_SECTION: Role = Role::new(14.0, 600.0, 20.0);
pub const COMPACT_READ: Role = Role::new(13.0, 400.0, 21.0);
/// Item titles: a meeting in a list, a topic, a model card, a panel's subject.
pub const TITLE: Role = Role::new(14.5, 600.0, 22.0);
/// Reading text: summary points, to-dos, answers and live notes.
pub const READ: Role = Role::new(BODY, 400.0, BODY_LH);
/// Dense rows and descriptions: tasks, findings, transcript lines, form text.
pub const SMALL: Role = Role::new(12.5, 400.0, 19.0);
/// The name in a dense row: a setting, a finding, a memory claim, a sidebar place.
pub const NAME: Role = SMALL.bold();
/// Places in the sidebar and Settings list, and menu labels.
pub const NAV: Role = Role::new(13.5, 400.0, 20.0);
/// Tabs, buttons, links and menu items.
pub const CONTROL: Role = Role::new(12.5, 400.0, 18.0).tabular(0.05);
/// Dates, durations and other facts shown alongside a control.
pub const FACT: Role = Role::new(12.5, 400.0, 18.0).tabular(0.05);
/// Fine print, sources and help text.
pub const META: Role = Role::new(12.0, 400.0, 17.0).tabular(0.1);
pub const MEETING_META: Role = Role::new(11.5, 400.0, 16.0).tabular(0.1);
pub const MEETING_CONTROL: Role = Role::new(12.0, 400.0, 17.0).tabular(0.05);
pub const MEETING_READ: Role = Role::new(13.5, 400.0, 22.0);
pub const MEETING_REPLY: Role = Role::new(13.0, 400.0, 22.0);
pub const MEETING_PROJECT: Role = Role::new(12.5, 500.0, 20.0);
/// Recording positions.
pub const TIME: Role = META;
/// Counts accompanying a label.
pub const COUNT: Role = FACT;
/// Group labels inside a section (Today, This week, Medium) and field labels.
pub const GROUP: Role = Role::new(13.0, 600.0, 20.0);
/// Panel captions in capitals, and the number in a count pill.
pub const CAPTION: Role = Role::new(12.0, 600.0, 16.0);

/// Space above a section heading. Sections are grouped by space first, lines second.
pub const SECTION_TOP: f32 = 28.0;
/// Space between a section heading and its first item.
pub const SECTION_GAP: f32 = 12.0;
/// Space above a group label inside a section.
pub const GROUP_TOP: f32 = 20.0;
/// Space between a title and its meta line.
pub const META_GAP: f32 = 2.0;

/// The " · " that joins facts on one meta line.
pub const DOT: &str = "  \u{b7}  ";

/// Meta text in the scanning grey.
pub fn meta(s: impl Into<String>, p: Palette) -> El {
    META.wrap(s, p.faint)
}

/// Several facts on one quiet line, joined by " · ". Empty parts are left out. Each fact keeps
/// its words together, so a narrow window breaks the line only between facts.
pub fn meta_line<S: AsRef<str>>(parts: &[S], p: Palette) -> El {
    let joined = parts
        .iter()
        .map(AsRef::as_ref)
        .filter(|s| !s.is_empty())
        .map(|s| s.replace(' ', "\u{a0}"))
        .collect::<Vec<_>>()
        .join(DOT);
    META.wrap(joined, p.faint)
}

/// A section heading: the role, the shared spacing and an optional count pill.
pub fn section(s: impl Into<String>, count_: Option<usize>, p: Palette) -> El {
    let mut r = row()
        .items_center()
        .gap(8.0)
        .mt(SECTION_TOP)
        .mb(SECTION_GAP)
        .child(SECTION.wrap(s, p.ink));
    if let Some(n) = count_ {
        r = r.child(count(n, p));
    }
    r
}

/// A number after a heading or tab, in a small pill.
pub fn count(n: usize, p: Palette) -> El {
    text(n.to_string())
        .font(COUNT.style())
        .color(p.faint)
        .px(7.0)
        .radius(999.0)
        .fill(p.chip)
}
