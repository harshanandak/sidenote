//! Solar Linear and Lucide 1.61.0 icons. See assets/licenses for attribution.
use rune::{Color, El, Icon};
use std::sync::LazyLock;

/// Width in the original 24 px view box, shared by every Lucide icon.
pub const STROKE_WIDTH: f32 = 2.0;

/// Icon size for row and button controls.
pub const ROW: f32 = 16.0;
/// Icon size alongside compact text.
pub const INLINE: f32 = 14.0;

const _: () = {
    assert!(ROW == 16.0);
    assert!(INLINE == 14.0);
};

macro_rules! lucide {
    ($entry:ident, $name:literal, $body:literal) => {
        pub static $entry: LazyLock<Icon> = LazyLock::new(|| {
            static BODY: LazyLock<String> =
                LazyLock::new(|| $body.replace("STROKE_WIDTH", &STROKE_WIDTH.to_string()));
            Icon {
                name: $name,
                view: 24.0,
                body: BODY.as_str(),
            }
        });
    };
}

pub fn draw(icon: &'static Icon, size: f32, ink: Color) -> El {
    rune::icon(weighted(icon, IconWeight::default()), size).color(ink)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum IconWeight {
    Light,
    #[default]
    Regular,
    Bold,
}

impl From<f32> for IconWeight {
    fn from(width: f32) -> Self {
        match width {
            1.5 => Self::Light,
            2.1 => Self::Bold,
            _ => Self::Regular,
        }
    }
}

macro_rules! solar_icon {
    ($entry:ident, $name:literal, $($shape:literal),+ $(,)?) => {
        pub(super) static $entry: [rune::Icon; 3] = [
            solar_icon!(@variant $name, "1.5", $($shape),+),
            solar_icon!(@variant $name, "1.8", $($shape),+),
            solar_icon!(@variant $name, "2.1", $($shape),+),
        ];
    };
    (@variant $name:literal, $width:literal, $($shape:literal),+) => {
        rune::Icon {
            name: concat!("solar-", $name),
            view: 24.0,
            body: concat!(
                r#"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-width=""#,
                $width, r#"">"#, $($shape,)+ "</g>"
            ),
        }
    };
}

mod solar;

solar_icon!(
    MODELS_AND_AGENTS,
    "four-point-star",
    r#"<path d="M12 2C13.4 8.2 15.8 10.6 22 12C15.8 13.4 13.4 15.8 12 22C10.6 15.8 8.2 13.4 2 12C8.2 10.6 10.6 8.2 12 2Z" stroke-linejoin="round"/>"#,
);

// Solar microphone-3 with the two interior strokes removed.
solar_icon!(
    MICROPHONE,
    "microphone-3",
    r#"<path d="M12 16C9.23858 16 7 13.7614 7 11V8C7 5.23858 9.23858 3 12 3C14.7614 3 17 5.23858 17 8V11C17 13.7614 14.7614 16 12 16Z"/>"#,
    r#"<path d="M20 10V11C20 15.4183 16.4183 19 12 19C7.58172 19 4 15.4183 4 11V10"/>"#,
    r#"<path d="M12 19V22"/>"#,
);

const REGULAR: usize = IconWeight::Regular as usize;
pub(super) const HOME: &Icon = &solar::HOME[REGULAR];
pub(super) const GENERAL: &Icon = &solar::GENERAL[REGULAR];
pub(super) const AGENTS: &Icon = &MODELS_AND_AGENTS[REGULAR];
pub(super) const CONNECTED_APPS: &Icon = &solar::CONNECTED_APPS[REGULAR];
pub(super) const APPEARANCE: &Icon = &solar::APPEARANCE[REGULAR];
pub(super) const PRIVACY: &Icon = &solar::PRIVACY[REGULAR];
pub(super) const SETTINGS_UPDATES: &Icon = &solar::UPDATES[REGULAR];
pub(super) const DIAGNOSTICS: &Icon = &solar::DIAGNOSTICS[REGULAR];

/// Static addresses let Rune reuse the raster atlas entry for each weight and size.
pub(super) fn weighted(icon: &'static Icon, weight: IconWeight) -> &'static Icon {
    let variants = match icon.name {
        "lucide-clock" => &solar::CLOCK,
        "lucide-folder" => &solar::FOLDER,
        name if name == HOME.name => &solar::HOME,
        "lucide-notepad-text" => &solar::MEETINGS,
        "lucide-list-checks" => &solar::TASKS,
        "lucide-users-round" => &solar::PEOPLE,
        "lucide-book-open" => &solar::MEMORY,
        "lucide-calendar" => &solar::DIARY,
        "lucide-pen-line" => &solar::ANNOTATIONS,
        "lucide-settings" => &solar::SETTINGS,
        "lucide-monitor-smartphone" => &solar::DEVICE,
        "lucide-mic" => &MICROPHONE,
        "solar-user" => &solar::GENERAL,
        "lucide-scroll-text" => &solar::PROMPTS,
        "lucide-flask-conical" => &solar::EXPERIMENTAL,
        "solar-four-point-star" => &MODELS_AND_AGENTS,
        "solar-link" => &solar::CONNECTED_APPS,
        "solar-sun" => &solar::APPEARANCE,
        "solar-lock-keyhole-minimalistic" => &solar::PRIVACY,
        "solar-download-minimalistic" => &solar::UPDATES,
        "solar-pulse" => &solar::DIAGNOSTICS,
        _ => return icon,
    };
    &variants[weight as usize]
}

// Kit selects paint into a 16-point icon box. Scale the glyph around its centre.
macro_rules! select_chevron {
    ($entry:ident, $base:ident, $name:literal) => {
        pub static $entry: LazyLock<Icon> = LazyLock::new(|| {
            static BODY: LazyLock<String> = LazyLock::new(|| {
                let scale = INLINE / ROW;
                let offset = $base.view * (1.0 - scale) / 2.0;
                format!(
                    "<g transform=\"translate({offset} {offset}) scale({scale})\">{}</g>",
                    $base.body
                )
            });
            Icon {
                name: $name,
                view: $base.view,
                body: BODY.as_str(),
            }
        });
    };
}

select_chevron!(SELECT_DOWN, CHEVRON_DOWN, "settings-chevron-down");
select_chevron!(SELECT_UP, CHEVRON_UP, "settings-chevron-up");

// Embedded entries
lucide!(
    MEETINGS,
    "lucide-notepad-text",
    r#"<path d="M8 2v4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 2v4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M16 2v4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect width="16" height="18" x="4" y="4" rx="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 10h6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 14h8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 18h5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    ANNOTATIONS,
    "lucide-pen-line",
    r#"<path d="M13 21h8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    GRIP_VERTICAL,
    "lucide-grip-vertical",
    r#"<circle cx="9" cy="12" r="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="9" cy="5" r="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="9" cy="19" r="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="15" cy="12" r="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="15" cy="5" r="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="15" cy="19" r="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    TASKS,
    "lucide-list-checks",
    r#"<path d="M13 5h8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M13 12h8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M13 19h8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m3 17 2 2 4-4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m3 7 2 2 4-4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    MEMORY,
    "lucide-book-open",
    r#"<path d="M12 5v16" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M20.001 19A2 2 0 0022 17V5a2 2 0 00-1.999-2L16 3.002A5 5 0 0012 5a5 5 0 00-4-2H4a2 2 0 00-2 2v12a2 2 0 001.999 2H8a5 5 0 014 2 5 5 0 014-2z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    DIARY,
    "lucide-calendar",
    r#"<path d="M8 2v3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M16 2v3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect x="3" y="3" width="18" height="18" rx="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M3 9h18" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    SETTINGS,
    "lucide-settings",
    r#"<path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="12" cy="12" r="3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    REFRESH_CW,
    "lucide-refresh-cw",
    r#"<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M21 3v5h-5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 16H3v5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    DEVICE,
    "lucide-monitor-smartphone",
    r#"<path d="M18 8V6a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M10 19v-3.96 3.15" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M7 19h5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect width="6" height="10" x="16" y="12" rx="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    PLUS,
    "lucide-plus",
    r#"<path d="M5 12h14" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 5v14" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    STOP,
    "lucide-square",
    r#"<rect width="18" height="18" x="3" y="3" rx="2" fill="currentColor" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CHECK,
    "lucide-check",
    r#"<path d="M20 6 9 17l-5-5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CHEVRON_DOWN,
    "lucide-chevron-down",
    r#"<path d="m6 9 6 6 6-6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CHEVRON_RIGHT,
    "lucide-chevron-right",
    r#"<path d="m9 18 6-6-6-6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CHEVRON_LEFT,
    "lucide-chevron-left",
    r#"<path d="m15 18-6-6 6-6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CHEVRONS_UP_DOWN,
    "lucide-chevrons-up-down",
    r#"<path d="m7 15 5 5 5-5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m7 9 5-5 5 5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    ARROW_RIGHT,
    "lucide-arrow-right",
    r#"<path d="M5 12h14" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m12 5 7 7-7 7" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    ARROW_UP,
    "lucide-arrow-up",
    r#"<path d="m5 12 7-7 7 7" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 19V5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    X,
    "lucide-x",
    r#"<path d="M18 6 6 18" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m6 6 12 12" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    MORE,
    "lucide-ellipsis",
    r#"<circle cx="12" cy="12" r="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="19" cy="12" r="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="5" cy="12" r="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    WARNING,
    "lucide-triangle-alert",
    r#"<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 9v4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 17h.01" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    ERROR,
    "lucide-circle-alert",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><line x1="12" x2="12" y1="8" y2="12" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><line x1="12" x2="12.01" y1="16" y2="16" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    HELP,
    "lucide-circle-question-mark",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 17h.01" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    INFO,
    "lucide-info",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 16v-4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 8h.01" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    SEARCH,
    "lucide-search",
    r#"<path d="m21 21-4.34-4.34" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="11" cy="11" r="8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    PANEL_CLOSE,
    "lucide-panel-left",
    r#"<rect width="18" height="18" x="3" y="3" rx="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M9 3v18" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    PANEL_OPEN,
    "lucide-panel-left",
    r#"<rect width="18" height="18" x="3" y="3" rx="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M9 3v18" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    PROMPTS,
    "lucide-scroll-text",
    r#"<path d="M15 12h-5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M15 8h-5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M19 17V5a2 2 0 0 0-2-2H4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 21h12a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1H11a1 1 0 0 0-1 1v1a2 2 0 1 1-4 0V5a2 2 0 1 0-4 0v2a1 1 0 0 0 1 1h3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    MIC,
    "lucide-mic",
    r#"<path d="M12 19v3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M19 10v2a7 7 0 0 1-14 0v-2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect x="9" y="2" width="6" height="13" rx="3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CIRCLE,
    "lucide-circle",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    WAITING,
    "lucide-circle-pause",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><line x1="10" x2="10" y1="15" y2="9" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><line x1="14" x2="14" y1="15" y2="9" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CIRCLE_DOT,
    "lucide-circle-dot",
    r#"<circle cx="12" cy="12" r="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CIRCLE_DASHED,
    "lucide-circle-dashed",
    r#"<path d="M10.1 2.182a10 10 0 0 1 3.8 0" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M13.9 21.818a10 10 0 0 1-3.8 0" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M17.609 3.721a10 10 0 0 1 2.69 2.7" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M2.182 13.9a10 10 0 0 1 0-3.8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M20.279 17.609a10 10 0 0 1-2.7 2.69" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M21.818 10.1a10 10 0 0 1 0 3.8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M3.721 6.391a10 10 0 0 1 2.7-2.69" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M6.391 20.279a10 10 0 0 1-2.69-2.7" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    LOADER_CIRCLE,
    "lucide-loader-circle",
    r#"<path d="M21 12a9 9 0 1 1-6.219-8.56" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    BOT,
    "lucide-bot",
    r#"<path d="M12 8V4H8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect width="16" height="12" x="4" y="8" rx="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M2 14h2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M20 14h2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M15 13v2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M9 13v2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    MESSAGE_SQUARE,
    "lucide-message-square",
    r#"<path d="M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    PALETTE,
    "lucide-palette",
    r#"<path d="M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="13.5" cy="6.5" r=".5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="17.5" cy="10.5" r=".5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="6.5" cy="12.5" r=".5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="8.5" cy="7.5" r=".5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CHEVRON_UP,
    "lucide-chevron-up",
    r#"<path d="m18 15-6-6-6 6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    COPY,
    "lucide-copy",
    r#"<rect width="14" height="14" x="8" y="8" rx="2" ry="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    LOCK,
    "lucide-lock",
    r#"<rect width="18" height="11" x="3" y="11" rx="2" ry="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M7 11V7a5 5 0 0 1 10 0v4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    EYE,
    "lucide-eye",
    r#"<path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="12" cy="12" r="3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    EYE_OFF,
    "lucide-eye-off",
    r#"<path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M14.084 14.158a3 3 0 0 1-4.242-4.242" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m2 2 20 20" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CIRCLE_X,
    "lucide-circle-x",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m15 9-6 6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m9 9 6 6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    DASH,
    "lucide-minus",
    r#"<path d="M5 12h14" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    DONE,
    "lucide-circle-check",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m16 9-5.5 5.5L8 12" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    SUMMARY,
    "lucide-sparkles",
    r#"<path d="M11.017 2.814a1 1 0 0 1 1.966 0l1.051 5.558a2 2 0 0 0 1.594 1.594l5.558 1.051a1 1 0 0 1 0 1.966l-5.558 1.051a2 2 0 0 0-1.594 1.594l-1.051 5.558a1 1 0 0 1-1.966 0l-1.051-5.558a2 2 0 0 0-1.594-1.594l-5.558-1.051a1 1 0 0 1 0-1.966l5.558-1.051a2 2 0 0 0 1.594-1.594z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M20 2v4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M22 4h-4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="4" cy="20" r="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    UPDATES,
    "lucide-download",
    r#"<path d="M12 15V3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m7 10 5 5 5-5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CONNECTED,
    "lucide-plug",
    r#"<path d="M12 22v-5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M15 8V2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M17 8a1 1 0 0 1 1 1v4a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V9a1 1 0 0 1 1-1z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M9 8V2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    PEOPLE,
    "lucide-users-round",
    r#"<path d="M18 21a8 8 0 0 0-16 0" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="10" cy="8" r="5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M22 20c0-3.37-2-6.5-4-8a5 5 0 0 0-.45-8.3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    FLAG,
    "lucide-flag",
    r#"<path d="M4 22V4a1 1 0 0 1 .4-.8A6 6 0 0 1 8 2c3 0 5 2 7.333 2q2 0 3.067-.8A1 1 0 0 1 20 4v10a1 1 0 0 1-.4.8A6 6 0 0 1 16 16c-3 0-5-2-8-2a6 6 0 0 0-4 1.528" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    SPELLING,
    "lucide-spell-check",
    r#"<path d="m20 15-5.5 5.5L12 18" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m4 16 6-12 5.115 10.23" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M6 12h8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    PHONE_CALL,
    "lucide-phone-call",
    r#"<path d="M13 2a9 9 0 0 1 9 9" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M13 6a5 5 0 0 1 5 5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M13.832 16.568a1 1 0 0 0 1.213-.303l.355-.465A2 2 0 0 1 17 15h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2A18 18 0 0 1 2 4a2 2 0 0 1 2-2h3a2 2 0 0 1 2 2v3a2 2 0 0 1-.8 1.6l-.468.351a1 1 0 0 0-.292 1.233 14 14 0 0 0 6.392 6.384" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    LANGUAGES,
    "lucide-languages",
    r#"<path d="m5 8 6 6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m4 14 6-6 2-3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M2 5h12" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M7 2h1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m22 22-5-10-5 10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M14 18h6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    BRAIN,
    "lucide-brain",
    r#"<path d="M12 18V5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M15 13a4.17 4.17 0 01-3-4 4.17 4.17 0 01-3 4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M17.707 6.294A3 3 0 1012 5a3 3 0 10-5.707 1.294" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M17.996 5.125a4 4 0 012.526 5.77" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M18 18a4 4 0 002-7.465" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M19.967 17.482a4 4 0 11-7.967.517 4 4 0 11-7.967-.517" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M6 18a4 4 0 01-2-7.465" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M6.004 5.125a4 4 0 00-2.526 5.77" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    AUDIO_LINES,
    "lucide-audio-lines",
    r#"<path d="M2 10v3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M6 6v11" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M10 3v18" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M14 8v7" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M18 5v13" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M22 10v3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    TRASH,
    "lucide-trash-2",
    r#"<path d="M10 11v6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M14 11v6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M3 6h18" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    VOLUME_X,
    "lucide-volume-x",
    r#"<path d="M11 4.702a.7.7 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.7.7 0 0 0 11 19.298z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m16.5 14.5 5-5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m16.5 9.5 5 5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    FOLDER,
    "lucide-folder",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    USER,
    "lucide-user-round",
    r#"<circle cx="12" cy="8" r="5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M20 21a8 8 0 0 0-16 0" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    FILE_TEXT,
    "lucide-file-text",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M14 2v5a1 1 0 0 0 1 1h5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M10 9H8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M16 13H8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M16 17H8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    KEYBOARD,
    "lucide-keyboard",
    r#"<path d="M10 8h.01" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 12h.01" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M14 8h.01" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M16 12h.01" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M18 8h.01" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M6 8h.01" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M7 16h10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 12h.01" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect width="20" height="16" x="2" y="4" rx="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    ERASER,
    "lucide-eraser",
    r#"<path d="M21 21H8a2 2 0 0 1-1.42-.587l-3.994-3.999a2 2 0 0 1 0-2.828l10-10a2 2 0 0 1 2.829 0l5.999 6a2 2 0 0 1 0 2.828L12.834 21" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m5.082 11.09 8.828 8.828" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CLIPBOARD_PASTE,
    "lucide-clipboard-paste",
    r#"<path d="M11 14h10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M16 4h2a2 2 0 0 1 2 2v1.344" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m17 18 4-4-4-4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 1.793-1.113" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect x="8" y="2" width="8" height="4" rx="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    TIMER,
    "lucide-timer",
    r#"<line x1="10" x2="14" y1="2" y2="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><line x1="12" x2="15" y1="14" y2="11" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="12" cy="14" r="8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CLOCK,
    "lucide-clock",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 6v6l4 2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CLOUD_OFF,
    "lucide-cloud-off",
    r#"<path d="M10.94 5.274A7 7 0 0 1 15.71 10h1.79a4.5 4.5 0 0 1 4.222 6.057" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M18.796 18.81A4.5 4.5 0 0 1 17.5 19H9A7 7 0 0 1 5.79 5.78" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m2 2 20 20" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CLIPBOARD,
    "lucide-clipboard",
    r#"<rect width="8" height="4" x="8" y="2" rx="1" ry="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    SPEECH,
    "lucide-speech",
    r#"<path d="M8.8 20v-4.1l1.9.2a2.3 2.3 0 0 0 2.164-2.1V8.3A5.37 5.37 0 0 0 2 8.25c0 2.8.656 3.054 1 4.55a5.77 5.77 0 0 1 .029 2.758L2 20" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M19.8 17.8a7.5 7.5 0 0 0 .003-10.603" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M17 15a3.5 3.5 0 0 0-.025-4.975" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    POWER,
    "lucide-power",
    r#"<path d="M12 2v10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M18.4 6.6a9 9 0 1 1-12.77.04" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    VOLUME_2,
    "lucide-volume-2",
    r#"<path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M16 9a5 5 0 0 1 0 6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M19.364 18.364a9 9 0 0 0 0-12.728" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    FLASK,
    "lucide-flask-conical",
    r#"<path d="M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M6.453 15h11.094" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M8.5 2h7" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CAPTIONS,
    "lucide-captions",
    r#"<rect width="18" height="14" x="3" y="5" rx="2" ry="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M7 15h4M15 15h2M7 11h2M13 11h4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CPU,
    "lucide-cpu",
    r#"<path d="M12 20v2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 2v2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M17 20v2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M17 2v2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M2 12h2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M2 17h2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M2 7h2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M20 12h2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M20 17h2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M20 7h2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M7 20v2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M7 2v2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect x="4" y="4" width="16" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect x="8" y="8" width="8" height="8" rx="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    KEY_ROUND,
    "lucide-key-round",
    r#"<path d="M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="16.5" cy="7.5" r=".5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    PACKAGE,
    "lucide-package",
    r#"<path d="M11 21.73a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 22V12" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M3.29 7 12 12 20.71 7" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m7.5 4.27 9 5.15" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    FILE_CLOCK,
    "lucide-file-clock",
    r#"<path d="M16 22h2a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v2.85" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M14 2v5a1 1 0 0 0 1 1h5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 14v2.2l1.6 1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="8" cy="16" r="6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    GLOBE,
    "lucide-globe",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M2 12h20" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    AUDIO_WAVEFORM,
    "lucide-audio-waveform",
    r#"<path d="M2 13a2 2 0 0 0 2-2V7a2 2 0 0 1 4 0v13a2 2 0 0 0 4 0V4a2 2 0 0 1 4 0v13a2 2 0 0 0 4 0v-4a2 2 0 0 1 2-2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CALENDAR_CHECK,
    "lucide-calendar-check",
    r#"<path d="M8 2v3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M16 2v3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect x="3" y="3" width="18" height="18" rx="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M3 9h18" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m9 15 2 2 4-4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    LIST_TODO,
    "lucide-list-todo",
    r#"<path d="M13 5h8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M13 12h8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M13 19h8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m3 17 2 2 4-4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect x="3" y="4" width="6" height="6" rx="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    FOLDERS,
    "lucide-folders",
    r#"<path d="M20 5a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h2.5a1.5 1.5 0 0 1 1.2.6l.6.8a1.5 1.5 0 0 0 1.2.6z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M3 8.268a2 2 0 0 0-1 1.738V19a2 2 0 0 0 2 2h11a2 2 0 0 0 1.732-1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    RADIO_TOWER,
    "lucide-radio-tower",
    r#"<path d="M4.9 16.1C1 12.2 1 5.8 4.9 1.9" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M7.8 4.7a6.14 6.14 0 0 0-.8 7.5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><circle cx="12" cy="9" r="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M16.2 4.8c2 2 2.26 5.11.8 7.47" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M19.1 1.9a9.96 9.96 0 0 1 0 14.1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M9.5 18h5" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m8 22 4-11 4 11" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    MONITOR,
    "lucide-monitor",
    r#"<rect width="20" height="14" x="2" y="3" rx="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><line x1="8" x2="16" y1="21" y2="21" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><line x1="12" x2="12" y1="17" y2="21" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    SMARTPHONE,
    "lucide-smartphone",
    r#"<rect width="14" height="20" x="5" y="2" rx="2" ry="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 18h.01" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    TERMINAL,
    "lucide-terminal",
    r#"<path d="M12 19h8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m4 17 6-6-6-6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CODE_XML,
    "lucide-code-xml",
    r#"<path d="m18 16 4-4-4-4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m6 8-4 4 4 4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m14.5 4-5 16" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    SQUARE_TERMINAL,
    "lucide-square-terminal",
    r#"<path d="m7 11 2-2-2-2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M11 13h4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect width="18" height="18" x="3" y="3" rx="2" ry="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    APP_WINDOW,
    "lucide-app-window",
    r#"<rect x="2" y="4" width="20" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M10 4v4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M2 8h20" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M6 4v4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    MOUSE_POINTER,
    "lucide-mouse-pointer-2",
    r#"<path d="M4.037 4.688a.495.495 0 0 1 .651-.651l16 6.5a.5.5 0 0 1-.063.947l-6.124 1.58a2 2 0 0 0-1.438 1.435l-1.579 6.126a.5.5 0 0 1-.947.063z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    NETWORK,
    "lucide-network",
    r#"<rect x="16" y="16" width="6" height="6" rx="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect x="2" y="16" width="6" height="6" rx="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect x="9" y="2" width="6" height="6" rx="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M5 16v-3a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v3" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 12V8" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    SHIELD_CHECK,
    "lucide-shield-check",
    r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="m9 12 2 2 4-4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    CIRCLE_STOP,
    "lucide-circle-stop",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><rect x="9" y="9" width="6" height="6" rx="1" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    HOURGLASS,
    "lucide-hourglass",
    r#"<path d="M5 22h14" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M5 2h14" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);

#[cfg(test)]
const GENERATED_LUCIDE_ICONS: &[&LazyLock<Icon>] = &[
    &MEETINGS,
    &ANNOTATIONS,
    &GRIP_VERTICAL,
    &TASKS,
    &MEMORY,
    &DIARY,
    &SETTINGS,
    &REFRESH_CW,
    &DEVICE,
    &PLUS,
    &STOP,
    &CHECK,
    &CHEVRON_DOWN,
    &CHEVRON_RIGHT,
    &CHEVRON_LEFT,
    &CHEVRONS_UP_DOWN,
    &ARROW_RIGHT,
    &ARROW_UP,
    &X,
    &MORE,
    &WARNING,
    &ERROR,
    &HELP,
    &INFO,
    &SEARCH,
    &PANEL_CLOSE,
    &PANEL_OPEN,
    &PROMPTS,
    &MIC,
    &CIRCLE,
    &WAITING,
    &CIRCLE_DOT,
    &CIRCLE_DASHED,
    &LOADER_CIRCLE,
    &BOT,
    &MESSAGE_SQUARE,
    &PALETTE,
    &CHEVRON_UP,
    &COPY,
    &LOCK,
    &EYE,
    &EYE_OFF,
    &CIRCLE_X,
    &DASH,
    &DONE,
    &SUMMARY,
    &UPDATES,
    &CONNECTED,
    &PEOPLE,
    &FLAG,
    &SPELLING,
    &PHONE_CALL,
    &LANGUAGES,
    &BRAIN,
    &AUDIO_LINES,
    &TRASH,
    &VOLUME_X,
    &FOLDER,
    &USER,
    &FILE_TEXT,
    &KEYBOARD,
    &ERASER,
    &CLIPBOARD_PASTE,
    &TIMER,
    &CLOCK,
    &CLOUD_OFF,
    &CLIPBOARD,
    &SPEECH,
    &POWER,
    &VOLUME_2,
    &FLASK,
    &CAPTIONS,
    &CPU,
    &KEY_ROUND,
    &PACKAGE,
    &FILE_CLOCK,
    &GLOBE,
    &AUDIO_WAVEFORM,
    &CALENDAR_CHECK,
    &LIST_TODO,
    &FOLDERS,
    &RADIO_TOWER,
    &MONITOR,
    &SMARTPHONE,
    &TERMINAL,
    &CODE_XML,
    &SQUARE_TERMINAL,
    &APP_WINDOW,
    &MOUSE_POINTER,
    &NETWORK,
    &SHIELD_CHECK,
    &CIRCLE_STOP,
    &HOURGLASS,
];

/// Existing controls share this adapter so text glyphs cannot drift from the SVG set.
pub fn styled(value: impl Into<String>, style: rune::TextStyle, ink: Color) -> El {
    let value = value.into();
    let glyph = |ch| -> Option<&'static Icon> {
        Some(match ch {
            '+' => &PLUS,
            '\u{2713}' => &CHECK,
            '\u{d7}' => &X,
            '\u{2039}' => &CHEVRON_LEFT,
            '\u{203a}' => &CHEVRON_RIGHT,
            '\u{25be}' | '\u{25bc}' => &CHEVRON_DOWN,
            '\u{25b8}' | '\u{25b6}' => &CHEVRON_RIGHT,
            '\u{2191}' => &ARROW_UP,
            _ => return None,
        })
    };
    if let Some(icon) = match value.as_str() {
        "\u{2026}" => Some(&*MORE),
        "!" => Some(&*ERROR),
        "?" => Some(&*HELP),
        "i" => Some(&*INFO),
        _ => None,
    } {
        return draw(icon, INLINE, ink);
    }
    if !value.chars().any(|ch| glyph(ch).is_some()) {
        return rune::text(value).font(style).color(ink).wrap();
    }
    let mut line = rune::row().gap(4.0).items_center().min_w(0.0);
    let mut word = String::new();
    for ch in value.chars() {
        if let Some(icon) = glyph(ch) {
            if !word.trim().is_empty() {
                line = line.child(
                    rune::text(word.trim().to_string())
                        .font(style)
                        .color(ink)
                        .wrap(),
                );
            }
            word.clear();
            line = line.child(draw(icon, INLINE, ink));
        } else {
            word.push(ch);
        }
    }
    if !word.trim().is_empty() {
        line = line.child(
            rune::text(word.trim().to_string())
                .font(style)
                .color(ink)
                .wrap()
                .min_w(0.0),
        );
    }
    line
}

lucide!(
    BUILDING_2,
    "lucide-building-2",
    r#"<path d="M6 22V4a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v18Z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M6 12H4a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h2M18 8h2a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2h-2M10 6h4M10 10h4M10 14h4M10 18h4" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);

lucide!(
    CIRCLE_PAUSE,
    "lucide-circle-pause",
    r#"<circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/><path d="M10 9v6M14 9v6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round" stroke-linejoin="round"/>"#
);
lucide!(
    HEX_MARK,
    "hex-ownership",
    r#"<path d="m12 2 9 5v10l-9 5-9-5V7z" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linejoin="round"/>"#
);
lucide!(
    SLIDERS,
    "lucide-sliders-horizontal",
    r#"<path d="M21 4h-7M10 4H3m18 8H10m-4 0H3m18 8h-3m-4 0H3M14 1v6M6 9v6m12 2v6" fill="none" stroke="currentColor" stroke-width="STROKE_WIDTH" stroke-linecap="round"/>"#
);

pub static PLAY: Icon = Icon {
    name: "sidenote-play",
    view: 24.0,
    body: r#"<path d="M8 4v16l12-8Z" fill="currentColor"/>"#,
};

#[cfg(test)]
pub(super) fn assert_solar_weights(source: &'static Icon, name: &str) {
    for (weight, stroke) in [(1.5, "1.5"), (1.8, "1.8"), (2.1, "2.1")] {
        let icon = weighted(source, weight.into());
        assert_eq!(icon.name, name);
        assert!(icon.body.contains(&format!("stroke-width=\"{stroke}\"")));
        let coverage = rune::text::icon_coverage(icon, 16);
        assert!(coverage.into_iter().any(|ink| ink > 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solar_weights_reuse_three_distinct_icons_and_default_to_regular() {
        for (source, name) in [(&*CLOCK, "solar-clock-circle"), (&*FOLDER, "solar-folder")] {
            let light = weighted(source, IconWeight::Light);
            let regular = weighted(source, IconWeight::Regular);
            let bold = weighted(source, IconWeight::Bold);
            assert!(std::ptr::eq(
                weighted(source, IconWeight::default()),
                regular
            ));
            assert!(!std::ptr::eq(light, regular));
            assert!(!std::ptr::eq(regular, bold));
            assert!(!std::ptr::eq(light, bold));

            let mut coverage = 0;
            for (width, icon) in [(1.5, light), (1.8, regular), (2.1, bold)] {
                assert_eq!(icon.name, name);
                assert!(icon.body.contains(&format!("stroke-width=\"{width}\"")));
                let ink: u32 = rune::text::icon_coverage(icon, 24)
                    .into_iter()
                    .map(u32::from)
                    .sum();
                assert!(ink > coverage);
                coverage = ink;
                assert!(std::ptr::eq(
                    weighted(source, IconWeight::from(width)),
                    icon
                ));
            }
            for invalid in [0.0, -1.0, 1.7, f32::INFINITY, f32::NAN] {
                assert!(std::ptr::eq(
                    weighted(source, IconWeight::from(invalid)),
                    regular
                ));
            }
        }
        assert!(std::ptr::eq(
            weighted(&GRIP_VERTICAL, IconWeight::Bold),
            &*GRIP_VERTICAL
        ));
    }

    #[test]
    fn solar_home_settings_and_custom_glyphs_have_all_three_weights() {
        for (source, name) in [
            (HOME, "solar-home-smile"),
            (&*SETTINGS, "solar-settings"),
            (AGENTS, "solar-four-point-star"),
            (&*MIC, "solar-microphone-3"),
        ] {
            assert_solar_weights(source, name);
        }
    }

    #[test]
    fn shell_lucide_icons_use_the_shared_stroke_and_requested_glyphs() {
        let manual = [&BUILDING_2, &CIRCLE_PAUSE, &SLIDERS];
        for icon in GENERATED_LUCIDE_ICONS.iter().copied().chain(manual) {
            let mut widths = icon.body.split("stroke-width=\"").skip(1).peekable();
            assert!(
                widths.peek().is_some() && widths.all(|width| width.starts_with("2\"")),
                "{} must use stroke-width=\"2\"",
                icon.name
            );
        }

        for (icon, name) in [
            (&*MEETINGS, "lucide-notepad-text"),
            (&*ANNOTATIONS, "lucide-pen-line"),
            (&*MEMORY, "lucide-book-open"),
            (&*PEOPLE, "lucide-users-round"),
            (&*USER, "lucide-user-round"),
            (&*DEVICE, "lucide-monitor-smartphone"),
            (&*PROMPTS, "lucide-scroll-text"),
            (&*UPDATES, "lucide-download"),
            (&*PANEL_CLOSE, "lucide-panel-left"),
            (&*PANEL_OPEN, "lucide-panel-left"),
            (&*STOP, "lucide-square"),
        ] {
            assert_eq!(icon.name, name);
        }
        assert!(STOP.body.contains("fill=\"currentColor\""));
    }
}
