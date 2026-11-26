//! Colours, fonts and the type scale. Named base colours feed meaning tokens, which the
//! component palette maps to the fields used by today's renderers. The caller supplies the
//! light/dark setting. The default palette is Olive, built on a shared base.
//! docs/design/typography.md lists the text contrast ratios.
use rune::{Color, Family, TextStyle};
use rune_kit::{Kit, PaletteSeed};

#[derive(Clone, Copy)]
pub struct Palette {
    /// The reading page.
    pub bg: Color,
    /// Surfaces: the Ask pill, its thread and inputs.
    pub card: Color,
    /// Floating details and help hints on the meeting page.
    pub raised: Color,
    /// The quiet inset fill for the current region.
    pub side: Color,
    pub ink: Color,
    pub body: Color,
    pub soft: Color,
    /// The dimmest text: facts, placeholders and small notes. Still 4.5:1 on its fills.
    pub faint: Color,
    pub line: Color,
    pub mark: Color,
    /// Focus ring. The primary button uses its own fill.
    pub accent: Color,
    /// Preset link colour. Classic uses the current page ink.
    pub link: Option<Color>,
    pub ok: Color,
    pub okbg: Color,
    pub warn: Color,
    pub warnbg: Color,
    pub open: Color,
    /// Hover fills, count pills and quiet buttons.
    pub chip: Color,
    /// The selected row in a list.
    pub sel: Color,
    pub hit: Color,
    pub shadow: Color,
    /// Dims the page under a modal, dark enough that page headings do not compete with it.
    pub scrim: Color,
    /// The primary button and its contrasting ink.
    pub button_fill: Color,
    pub button_ink: Color,
    /// Classic uses the positive status green for an enabled switch in both modes.
    pub switch_on: Color,
    /// The enabled knob uses a colour that contrasts with the track.
    pub switch_knob: Color,
    pub button_border: Color,
    pub danger: Color,
    pub danger_border: Color,
    pub button_hover: Color,
    pub button_pressed: Color,
    pub button_primary_hover: Color,
    pub button_primary_pressed: Color,
}

fn hex(value: &str) -> Color {
    rune::color::hex(value).expect("valid theme colour")
}

#[derive(Clone, Copy)]
struct TextTokens {
    primary: Color,
    body: Color,
    secondary: Color,
    tertiary: Color,
}

#[derive(Clone, Copy)]
struct SurfaceTokens {
    fill: Color,
    text: TextTokens,
    quiet_fill: Color,
    selected_fill: Color,
    border_subtle: Color,
    focus_ring: Color,
    link: Option<Color>,
    open_status: Color,
    warning: Color,
    danger: Color,
}

struct MeaningTokens {
    page: SurfaceTokens,
    sidebar: SurfaceTokens,
    page_inset: Color,
    surface: Color,
    floating_surface: Color,
    border_strong: Color,
    mark: Color,
    accent_solid: Color,
    accent_on: Color,
    accent_hover_overlay: Color,
    accent_pressed_overlay: Color,
    positive: Color,
    positive_fill: Color,
    switch_track: Color,
    switch_knob: Color,
    warning_fill: Color,
    danger_outline: Color,
    quiet_hover_overlay: Color,
    quiet_pressed_overlay: Color,
    search_highlight: Color,
    shadow: Color,
    scrim: Color,
}

mod classic {
    use super::{MeaningTokens, SurfaceTokens, TextTokens, hex};

    const GREY_0: &str = "#ffffff";
    const GREY_10: &str = "#fcfbf8";
    const GREY_50: &str = "#f3f2ed";
    const GREY_75: &str = "#efede7";
    const GREY_100: &str = "#eceae4";
    const GREY_125: &str = "#e9e7e1";
    const GREY_150: &str = "#e6e3db";
    const GREY_200: &str = "#dcd9d2";
    const GREY_225: &str = "#d6d3cc";
    const GREY_300: &str = "#bcb9b0";
    const GREY_350: &str = "#b0ada4";
    const GREY_400: &str = "#95928a";
    const GREY_500: &str = "#77746d";
    const GREY_550: &str = "#6b6861";
    const GREY_600: &str = "#5d5b55";
    const GREY_700: &str = "#3e3d39";
    const GREY_800: &str = "#302f2c";
    const GREY_810: &str = "#2f2f2c";
    const GREY_825: &str = "#2a2a28";
    const GREY_830: &str = "#2a2a27";
    const GREY_850: &str = "#242422";
    const GREY_900: &str = "#1e1e1c";
    const GREY_925: &str = "#1a1a18";
    const GREY_950: &str = "#141413";
    const SLATE_900: &str = "#1f2328";

    const CORAL_400: &str = "#ee8566";
    const CORAL_700: &str = "#b8452a";
    const GREEN_50: &str = "#e4f3eb";
    const GREEN_400: &str = "#69c995";
    const GREEN_700: &str = "#1d7a4c";
    const GREEN_900: &str = "#1b2c22";
    const GREEN_950: &str = "#12251a";
    const AMBER_50: &str = "#fcf0dc";
    const AMBER_100: &str = "#fff3c4";
    const AMBER_400: &str = "#e3aa55";
    const AMBER_700: &str = "#9a5800";
    const AMBER_900: &str = "#363019";
    const AMBER_950: &str = "#33281a";
    const RED_100: &str = "#efc5c5";
    const RED_300: &str = "#f29b99";
    const RED_700: &str = "#a92b30";
    const RED_800: &str = "#704345";
    const VIOLET_400: &str = "#b49ee3";
    const VIOLET_700: &str = "#6a4ea3";

    const BLACK_08: &str = "#00000008";
    const BLACK_14: &str = "#00000014";
    const BLACK_29: &str = "#00000029";
    const BLACK_66: &str = "#00000066";
    const BLACK_B3: &str = "#000000b3";
    const WHITE_0F: &str = "#ffffff0f";
    const WHITE_14: &str = "#ffffff14";
    const WHITE_1F: &str = "#ffffff1f";
    const WHITE_24: &str = "#ffffff24";
    const GREY_900_1F: &str = "#1e1e1c1f";
    const GREY_900_73: &str = "#1e1e1c73";

    pub(super) fn meaning(dark: bool) -> MeaningTokens {
        let pick = |light, night| hex(if dark { night } else { light });
        let accent = pick(CORAL_700, CORAL_400);
        let positive = pick(GREEN_700, GREEN_400);
        let page = SurfaceTokens {
            fill: pick(GREY_10, GREY_925),
            text: TextTokens {
                primary: pick(GREY_900, GREY_125),
                body: pick(GREY_700, GREY_225),
                secondary: pick(GREY_600, GREY_300),
                tertiary: pick(GREY_550, GREY_350),
            },
            quiet_fill: pick(GREY_75, GREY_830),
            selected_fill: pick(GREY_100, GREY_810),
            border_subtle: pick(GREY_150, GREY_800),
            focus_ring: accent,
            link: None,
            open_status: pick(VIOLET_700, VIOLET_400),
            warning: pick(AMBER_700, AMBER_400),
            danger: pick(RED_700, RED_300),
        };
        let inset = pick(GREY_50, GREY_950);
        MeaningTokens {
            page,
            sidebar: SurfaceTokens {
                fill: inset,
                ..page
            },
            page_inset: inset,
            surface: pick(GREY_0, GREY_850),
            floating_surface: pick(GREY_0, GREY_825),
            border_strong: pick(GREY_200, GREY_800),
            mark: pick(GREY_400, GREY_500),
            accent_solid: accent,
            accent_on: pick(GREY_0, SLATE_900),
            accent_hover_overlay: pick(BLACK_14, WHITE_14),
            accent_pressed_overlay: pick(BLACK_29, WHITE_24),
            positive,
            positive_fill: pick(GREEN_50, GREEN_900),
            switch_track: positive,
            switch_knob: pick(GREY_0, GREEN_950),
            warning_fill: pick(AMBER_50, AMBER_950),
            danger_outline: pick(RED_100, RED_800),
            quiet_hover_overlay: pick(BLACK_08, WHITE_0F),
            quiet_pressed_overlay: pick(BLACK_14, WHITE_1F),
            search_highlight: pick(AMBER_100, AMBER_900),
            shadow: pick(GREY_900_1F, BLACK_66),
            scrim: pick(GREY_900_73, BLACK_B3),
        }
    }
}

type PresetConstructor = fn(bool) -> MeaningTokens;

// `palette` uses the first constructor for existing renderers.
const PRESETS: [(&str, PresetConstructor); 3] =
    [("Olive", olive), ("Lime", lime), ("Rose Clay", rose_clay)];

fn olive(dark: bool) -> MeaningTokens {
    let pick = |light, night| hex(if dark { night } else { light });
    let base = classic::meaning(dark);
    let accent = pick("#55682a", "#a9c25a");
    let ink_on_accent = pick("#ffffff", "#141a08");
    let page = SurfaceTokens {
        fill: pick("#f5f0e6", "#161a12"),
        text: TextTokens {
            primary: pick("#1f2118", "#eef0e6"),
            body: pick("#3b3d31", "#d3d7c6"),
            secondary: pick("#4f5143", "#b3b8a2"),
            tertiary: pick("#5f6152", "#a7a890"),
        },
        quiet_fill: pick("#ece4d3", "#232a1d"),
        selected_fill: pick("#e9e1cf", "#2c3424"),
        border_subtle: pick("#e2d9c6", "#2f3628"),
        focus_ring: accent,
        link: Some(pick("#55682a", "#b8cf6c")),
        warning: pick("#8f5200", "#e8b25e"),
        danger: pick("#b42318", "#f4a19a"),
        ..base.page
    };
    let sidebar = if dark {
        SurfaceTokens {
            fill: hex("#11140d"),
            ..page
        }
    } else {
        // The sidebar has a dark fill even when the page is light.
        let link = hex("#b8cf6c");
        SurfaceTokens {
            fill: hex("#2f3a28"),
            text: TextTokens {
                primary: hex("#f3f1e8"),
                body: hex("#e6e8da"),
                secondary: hex("#d9dccb"),
                tertiary: hex("#c3c8b0"),
            },
            quiet_fill: hex("#3b4732"),
            selected_fill: hex("#45533b"),
            border_subtle: hex("#3d4a35"),
            focus_ring: link,
            link: Some(link),
            open_status: hex("#b49ee3"),
            warning: hex("#e8b25e"),
            danger: hex("#f4a19a"),
        }
    };
    MeaningTokens {
        page,
        sidebar,
        page_inset: page.quiet_fill,
        surface: pick("#fffdf8", "#1e2319"),
        floating_surface: pick("#fffdf8", "#252b1f"),
        border_strong: pick("#d8ceb9", "#2f3628"),
        mark: pick("#8d8b79", "#6d7360"),
        accent_solid: accent,
        accent_on: ink_on_accent,
        positive: pick("#2e7a1f", "#8fd37a"),
        switch_track: accent,
        switch_knob: ink_on_accent,
        warning_fill: pick("#f8ead2", "#2e2414"),
        shadow: pick("#2f3a281f", "#00000066"),
        scrim: pick("#1f211873", "#000000b3"),
        ..base
    }
}

fn lime(dark: bool) -> MeaningTokens {
    let pick = |light, night| hex(if dark { night } else { light });
    let base = classic::meaning(dark);
    let accent = hex("#c8f53a");
    let link = pick("#3f6212", "#c8f53a");
    let page = SurfaceTokens {
        fill: pick("#fbfcf7", "#0e0f0d"),
        text: TextTokens {
            primary: pick("#14160f", "#f1f3ec"),
            body: pick("#33372a", "#cdd1c4"),
            secondary: pick("#4a4e40", "#adb2a3"),
            tertiary: pick("#5c604f", "#999f8f"),
        },
        quiet_fill: pick("#eef1e4", "#1f221c"),
        selected_fill: pick("#e6eadb", "#23261f"),
        border_subtle: pick("#e1e5d6", "#2a2d26"),
        focus_ring: link,
        link: Some(link),
        warning: pick("#8f5200", "#f2b75b"),
        danger: pick("#b42318", "#ff9a95"),
        ..base.page
    };
    MeaningTokens {
        page,
        sidebar: SurfaceTokens {
            fill: pick("#f1f3ea", "#0a0b09"),
            ..page
        },
        page_inset: page.quiet_fill,
        surface: pick("#ffffff", "#171915"),
        floating_surface: pick("#ffffff", "#1d1f1a"),
        border_strong: pick("#d6dbc8", "#2a2d26"),
        mark: pick("#9da38f", "#62675a"),
        accent_solid: accent,
        accent_on: hex("#11140a"),
        positive: link,
        switch_track: pick("#14160f", "#c8f53a"),
        switch_knob: pick("#ffffff", "#11140a"),
        warning_fill: pick("#f8ead2", "#2e2414"),
        shadow: pick("#14160f1f", "#00000066"),
        scrim: pick("#14160f73", "#000000b3"),
        ..base
    }
}

fn rose_clay(dark: bool) -> MeaningTokens {
    let pick = |light, night| hex(if dark { night } else { light });
    let base = classic::meaning(dark);
    let accent = pick("#a3244f", "#f07aa0");
    let ink_on_accent = pick("#ffffff", "#1f0a12");
    let page = SurfaceTokens {
        fill: pick("#fbf4f2", "#1a1214"),
        text: TextTokens {
            primary: pick("#2a1719", "#f6ecee"),
            body: pick("#4a3235", "#dccdd0"),
            secondary: pick("#5d4447", "#bfaeb2"),
            tertiary: pick("#6c5356", "#ab9a9e"),
        },
        quiet_fill: pick("#f3e3df", "#2a1f22"),
        selected_fill: pick("#eedad5", "#33262a"),
        border_subtle: pick("#ecd8d3", "#3a2b2e"),
        focus_ring: accent,
        link: Some(pick("#a3244f", "#f48fb0")),
        warning: pick("#8f5200", "#e8b25e"),
        danger: pick("#b42318", "#ff9a95"),
        ..base.page
    };
    MeaningTokens {
        page,
        sidebar: SurfaceTokens {
            fill: pick("#f4e6e2", "#140d0f"),
            ..page
        },
        page_inset: page.quiet_fill,
        surface: pick("#fffbfa", "#231a1c"),
        floating_surface: pick("#fffbfa", "#2b2023"),
        border_strong: pick("#e5cfc9", "#3a2b2e"),
        mark: pick("#a8908f", "#76666a"),
        accent_solid: accent,
        accent_on: ink_on_accent,
        positive: pick("#17773f", "#7fd6a4"),
        switch_track: accent,
        switch_knob: ink_on_accent,
        warning_fill: pick("#f8ead2", "#2e2414"),
        shadow: pick("#2a17191f", "#00000066"),
        scrim: pick("#2a171973", "#000000b3"),
        ..base
    }
}

fn default_tokens(dark: bool) -> MeaningTokens {
    let [(_, default), ..] = PRESETS;
    default(dark)
}

pub fn palette(dark: bool) -> Palette {
    component_palette(&default_tokens(dark))
}

pub fn sidebar_palette(dark: bool, page: Palette) -> Palette {
    let sidebar = default_tokens(dark).sidebar;
    Palette {
        side: sidebar.fill,
        ink: sidebar.text.primary,
        body: sidebar.text.body,
        soft: sidebar.text.secondary,
        faint: sidebar.text.tertiary,
        line: sidebar.border_subtle,
        warn: sidebar.warning,
        danger: sidebar.danger,
        chip: sidebar.quiet_fill,
        sel: sidebar.selected_fill,
        ..page
    }
}

#[cfg(test)]
pub(super) fn classic_palette(dark: bool) -> Palette {
    component_palette(&classic::meaning(dark))
}

fn component_palette(tokens: &MeaningTokens) -> Palette {
    let page = tokens.page;
    Palette {
        bg: page.fill,
        card: tokens.surface,
        raised: tokens.floating_surface,
        side: tokens.page_inset,
        ink: page.text.primary,
        body: page.text.body,
        soft: page.text.secondary,
        faint: page.text.tertiary,
        line: page.border_subtle,
        mark: tokens.mark,
        accent: page.focus_ring,
        link: page.link,
        ok: tokens.positive,
        okbg: tokens.positive_fill,
        warn: page.warning,
        warnbg: tokens.warning_fill,
        open: page.open_status,
        chip: page.quiet_fill,
        sel: page.selected_fill,
        hit: tokens.search_highlight,
        shadow: tokens.shadow,
        scrim: tokens.scrim,
        button_fill: tokens.accent_solid,
        button_ink: tokens.accent_on,
        switch_on: tokens.switch_track,
        switch_knob: tokens.switch_knob,
        button_border: tokens.border_strong,
        danger: page.danger,
        danger_border: tokens.danger_outline,
        button_hover: tokens.quiet_hover_overlay,
        button_pressed: tokens.quiet_pressed_overlay,
        button_primary_hover: tokens.accent_hover_overlay,
        button_primary_pressed: tokens.accent_pressed_overlay,
    }
}

/// Kit controls (text fields, the runner dropdown, Markdown) drawn from the same palette.
pub fn kit(p: &Palette) -> Kit {
    let mut kit = Kit::from_palette(PaletteSeed {
        bg: p.card,
        fg: p.ink,
        accent: p.accent,
    });
    kit.glyph = rune_kit::Glyphs {
        check: &super::icons::CHECK,
        check_small: &super::icons::CHECK,
        chevron_down: &super::icons::CHEVRON_DOWN,
        chevron_up: &super::icons::CHEVRON_UP,
        caret_right: &super::icons::CHEVRON_RIGHT,
        triangle_down: &super::icons::CHEVRON_DOWN,
        triangle_right: &super::icons::CHEVRON_RIGHT,
        close: &super::icons::X,
        clear: &super::icons::X,
        copy: &super::icons::COPY,
        search: &super::icons::SEARCH,
        lock: &super::icons::LOCK,
        eye: &super::icons::EYE,
        eye_off: &super::icons::EYE_OFF,
        circle_x: &super::icons::CIRCLE_X,
        dash: &super::icons::DASH,
    };
    kit.c.bg_accent = p.switch_on;
    kit.c.knob_on = p.switch_knob;
    kit.c.text_accent = p.link.unwrap_or(p.ink);
    kit.c.knob = Color::WHITE;
    kit.c.text_faint = p.faint;
    kit.ty.ui = super::roles::CONTROL.style();
    kit.ty.small = super::roles::META.style();
    kit
}

/// Reading text: summary points, to-dos and answers (14.5 px, line height 1.6).
pub const BODY: f32 = 14.5;
pub const BODY_LH: f32 = 23.0;

/// The largest a page title gets. Every page title goes through `display`, so one number sets
/// them all.
pub const PAGE_TITLE: f32 = 26.0;

/// Titles use bundled Fraunces on every platform, so Windows and Linux draw the same face.
const DISPLAY: Family = Family::Named("Fraunces");

pub fn display(size: f32) -> TextStyle {
    let size = size.min(PAGE_TITLE);
    TextStyle::new(size, 400.0)
        .family(DISPLAY)
        .lh((size * 1.25).round())
}

/// The sans list. Figtree draws Latin, with Inter behind it. Windows ships Nirmala UI, which
/// draws Telugu and Devanagari, so text in those scripts, such as the Telugu examples in the
/// translation prompt, shows letters and not empty boxes. Linux bundles Noto Sans Devanagari
/// and Noto Sans Telugu for the same job. Rune has no Telugu fallback of its own.
#[cfg(windows)]
pub const SANS: &str = "Figtree, Inter, Nirmala UI";
#[cfg(not(windows))]
pub const SANS: &str = "Figtree, Inter, Noto Sans Devanagari, Noto Sans Telugu";

/// The bundled faces every platform loads: Figtree for text, Fraunces for titles and Inter
/// as the mono slot's name and a Latin fallback.
fn bundled() -> Vec<&'static [u8]> {
    vec![
        include_bytes!("../../assets/fonts/Figtree-Variable.ttf"),
        include_bytes!("../../assets/fonts/Fraunces-Variable.ttf"),
        include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
        include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
        include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    ]
}

/// Linux uses only the repo's licensed fonts, with Noto for Devanagari and Telugu.
#[cfg(not(windows))]
pub fn fonts() -> &'static [&'static [u8]] {
    let mut list = bundled();
    list.extend([
        include_bytes!("../../assets/fonts/NotoSansDevanagari-Variable.ttf").as_slice(),
        include_bytes!("../../assets/fonts/NotoSansTelugu-Variable.ttf").as_slice(),
    ]);
    Box::leak(list.into_boxed_slice())
}

/// The bundled faces, plus Nirmala UI from Windows for the scripts they lack.
#[cfg(windows)]
pub fn fonts() -> &'static [&'static [u8]] {
    let mut list = bundled();
    let dir = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("Fonts");
    if let Ok(bytes) = std::fs::read(dir.join("nirmala.ttc")) {
        // Read once at startup and kept for the life of the process, as Rune requires.
        list.push(Box::leak(bytes.into_boxed_slice()));
    }
    Box::leak(list.into_boxed_slice())
}

#[cfg(test)]
mod token_tests {
    use super::*;

    #[test]
    fn classic_base_keeps_its_light_and_dark_colours() {
        for dark in [false, true] {
            let p = component_palette(&classic::meaning(dark));
            for (name, actual, light, night) in [
                ("bg", p.bg, "#fcfbf8", "#1a1a18"),
                ("card", p.card, "#ffffff", "#242422"),
                ("raised", p.raised, "#ffffff", "#2a2a28"),
                ("side", p.side, "#f3f2ed", "#141413"),
                ("ink", p.ink, "#1e1e1c", "#e9e7e1"),
                ("body", p.body, "#3e3d39", "#d6d3cc"),
                ("soft", p.soft, "#5d5b55", "#bcb9b0"),
                ("faint", p.faint, "#6b6861", "#b0ada4"),
                ("line", p.line, "#e6e3db", "#302f2c"),
                ("mark", p.mark, "#95928a", "#77746d"),
                ("accent", p.accent, "#b8452a", "#ee8566"),
                ("ok", p.ok, "#1d7a4c", "#69c995"),
                ("okbg", p.okbg, "#e4f3eb", "#1b2c22"),
                ("warn", p.warn, "#9a5800", "#e3aa55"),
                ("warnbg", p.warnbg, "#fcf0dc", "#33281a"),
                ("open", p.open, "#6a4ea3", "#b49ee3"),
                ("chip", p.chip, "#efede7", "#2a2a27"),
                ("sel", p.sel, "#eceae4", "#2f2f2c"),
                ("hit", p.hit, "#fff3c4", "#363019"),
                ("shadow", p.shadow, "#1e1e1c1f", "#00000066"),
                ("scrim", p.scrim, "#1e1e1c73", "#000000b3"),
                ("button_fill", p.button_fill, "#b8452a", "#ee8566"),
                ("button_ink", p.button_ink, "#ffffff", "#1f2328"),
                ("switch_on", p.switch_on, "#1d7a4c", "#69c995"),
                ("switch_knob", p.switch_knob, "#ffffff", "#12251a"),
                ("button_border", p.button_border, "#dcd9d2", "#302f2c"),
                ("danger", p.danger, "#a92b30", "#f29b99"),
                ("danger_border", p.danger_border, "#efc5c5", "#704345"),
                ("button_hover", p.button_hover, "#00000008", "#ffffff0f"),
                ("button_pressed", p.button_pressed, "#00000014", "#ffffff1f"),
                (
                    "primary_hover",
                    p.button_primary_hover,
                    "#00000014",
                    "#ffffff14",
                ),
                (
                    "primary_pressed",
                    p.button_primary_pressed,
                    "#00000029",
                    "#ffffff24",
                ),
            ] {
                assert_eq!(
                    actual,
                    hex(if dark { night } else { light }),
                    "{name}, dark={dark}"
                );
            }
        }
    }

    #[test]
    fn default_palette_is_olive() {
        assert_eq!(
            PRESETS.map(|(name, _)| name),
            ["Olive", "Lime", "Rose Clay"]
        );
        for dark in [false, true] {
            let p = palette(dark);
            let olive = component_palette(&olive(dark));
            assert_eq!(p.link, olive.link, "link, dark={dark}");
            for (name, actual, expected) in [
                ("bg", p.bg, olive.bg),
                ("card", p.card, olive.card),
                ("raised", p.raised, olive.raised),
                ("side", p.side, olive.side),
                ("ink", p.ink, olive.ink),
                ("body", p.body, olive.body),
                ("soft", p.soft, olive.soft),
                ("faint", p.faint, olive.faint),
                ("line", p.line, olive.line),
                ("mark", p.mark, olive.mark),
                ("accent", p.accent, olive.accent),
                ("ok", p.ok, olive.ok),
                ("okbg", p.okbg, olive.okbg),
                ("warn", p.warn, olive.warn),
                ("warnbg", p.warnbg, olive.warnbg),
                ("open", p.open, olive.open),
                ("chip", p.chip, olive.chip),
                ("sel", p.sel, olive.sel),
                ("hit", p.hit, olive.hit),
                ("shadow", p.shadow, olive.shadow),
                ("scrim", p.scrim, olive.scrim),
                ("button_fill", p.button_fill, olive.button_fill),
                ("button_ink", p.button_ink, olive.button_ink),
                ("switch_on", p.switch_on, olive.switch_on),
                ("switch_knob", p.switch_knob, olive.switch_knob),
                ("button_border", p.button_border, olive.button_border),
                ("danger", p.danger, olive.danger),
                ("danger_border", p.danger_border, olive.danger_border),
                ("button_hover", p.button_hover, olive.button_hover),
                ("button_pressed", p.button_pressed, olive.button_pressed),
                (
                    "button_primary_hover",
                    p.button_primary_hover,
                    olive.button_primary_hover,
                ),
                (
                    "button_primary_pressed",
                    p.button_primary_pressed,
                    olive.button_primary_pressed,
                ),
            ] {
                assert_eq!(actual, expected, "{name}, dark={dark}");
            }
        }
    }

    #[test]
    fn component_palette_maps_meanings_independently() {
        let mut meaning = classic::meaning(false);
        meaning.page.focus_ring = hex("#ff0000");
        meaning.accent_solid = hex("#0000ff");
        meaning.sidebar.text.primary = hex("#ffffff");
        let p = component_palette(&meaning);
        assert_eq!(p.accent, meaning.page.focus_ring);
        assert_eq!(p.button_fill, meaning.accent_solid);
        assert_eq!(p.switch_on, meaning.positive);
        assert_eq!(p.ink, meaning.page.text.primary);
        assert_eq!(p.side, meaning.sidebar.fill);
        assert_eq!(p.raised, meaning.floating_surface);
    }

    #[test]
    fn olive_light_sidebar_popovers_keep_the_page_palette() {
        let meaning = olive(false);
        let p = component_palette(&meaning);
        assert_eq!(meaning.sidebar.text.primary, hex("#f3f1e8"));
        assert_eq!(meaning.sidebar.quiet_fill, hex("#3b4732"));
        assert_eq!(meaning.sidebar.selected_fill, hex("#45533b"));
        assert_eq!(meaning.sidebar.border_subtle, hex("#3d4a35"));
        assert_eq!(p.raised, hex("#fffdf8"));
        assert_eq!(p.ink, hex("#1f2118"));
    }

    #[test]
    fn olive_page_card_sidebar_and_accent_tokens_are_exact() {
        for (dark, page, card, sidebar, accent) in [
            (false, "#f5f0e6", "#fffdf8", "#2f3a28", "#55682a"),
            (true, "#161a12", "#1e2319", "#11140d", "#a9c25a"),
        ] {
            let p = component_palette(&olive(dark));
            assert_eq!(p.bg, hex(page), "page, dark={dark}");
            assert_eq!(p.card, hex(card), "card, dark={dark}");
            assert_eq!(
                olive(dark).sidebar.fill,
                hex(sidebar),
                "sidebar, dark={dark}"
            );
            assert_eq!(p.accent, hex(accent), "accent, dark={dark}");
        }
    }
}

#[cfg(test)]
mod quiet_ink_switch_tests {
    use super::*;

    #[test]
    fn the_sans_list_leads_with_figtree_and_names_the_indic_fallback() {
        assert!(SANS.starts_with("Figtree"));
        assert_eq!(SANS.contains("Nirmala UI"), cfg!(windows));
    }

    #[test]
    fn off_switches_keep_a_light_knob_in_both_themes() {
        for dark in [false, true] {
            let k = kit(&palette(dark));
            assert_eq!(k.c.knob, Color::WHITE);
            assert_ne!(k.c.knob, k.c.layer_04);
        }
    }

    #[test]
    fn classic_on_switches_use_the_positive_status_color_in_both_themes() {
        for dark in [false, true] {
            let p = component_palette(&classic::meaning(dark));
            let k = kit(&p);
            assert_eq!(k.c.bg_accent, p.ok);
            assert_eq!(k.c.bg_accent, p.switch_on);
            assert_eq!(k.c.knob_on, p.switch_knob);
            assert_ne!(k.c.bg_accent, p.accent);
            assert_ne!(k.c.bg_accent, p.button_fill);
            assert_eq!(k.c.text_accent, p.ink);
        }
    }

    #[test]
    fn olive_on_switches_use_the_accent_in_both_themes() {
        for dark in [false, true] {
            let p = palette(dark);
            let k = kit(&p);
            assert_eq!(k.c.bg_accent, p.accent);
            assert_eq!(k.c.bg_accent, p.switch_on);
            assert_eq!(k.c.knob_on, p.switch_knob);
            assert_ne!(k.c.bg_accent, p.ok);
            assert_eq!(k.c.text_accent, p.link.unwrap());
        }
    }
}

#[cfg(test)]
mod type_and_contrast_tests {
    use super::*;
    use parley::PositionedLayoutItem;
    use rune::text::TextSystem;

    const HINDI: &str = "\u{92f}\u{939} \u{90f}\u{915} \u{92a}\u{930}\u{940}\u{915}\u{94d}\u{937}\u{923} \u{935}\u{93e}\u{915}\u{94d}\u{92f} \u{939}\u{948}";
    const TELUGU: &str = "\u{c07}\u{c26}\u{c3f} \u{c12}\u{c15} \u{c2a}\u{c30}\u{c40}\u{c15}\u{c4d}\u{c37} \u{c35}\u{c3e}\u{c15}\u{c4d}\u{c2f}\u{c02}";

    fn glyph_ids(ts: &mut TextSystem, text: &str, style: TextStyle) -> Vec<u32> {
        let layout = ts.layout_rich(text, &[], style, 1.0, None);
        let mut ids = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    ids.extend(run.glyphs().map(|g| g.id));
                }
            }
        }
        ids
    }

    fn system() -> TextSystem {
        TextSystem::new(fonts(), (SANS, "Inter"))
    }

    #[test]
    fn time_role_gives_recording_times_equal_width() {
        let mut ts = system();
        let style = crate::ui::roles::TIME.style();
        let narrow = ts.layout_rich("11:11", &[], style, 1.0, None).width();
        let wide = ts.layout_rich("22:22", &[], style, 1.0, None).width();
        assert!((narrow - wide).abs() < 0.01, "{narrow} vs {wide}");
    }

    #[test]
    fn meeting_renderers_use_no_mono_family() {
        for (name, source) in [
            ("roles", include_str!("roles.rs")),
            ("meeting", include_str!("meeting.rs")),
            ("reading", include_str!("meeting/reading.rs")),
            ("widgets", include_str!("widgets.rs")),
            ("tasks", include_str!("tasks.rs")),
            ("todo", include_str!("todo.rs")),
        ] {
            let source: String = source.split_whitespace().collect();
            for mono in [".mono(", "Family::Mono", "Family::SystemMono", "mono:true"] {
                assert!(!source.contains(mono), "{name} requests {mono}");
            }
        }
    }

    #[test]
    fn titles_draw_in_fraunces_and_text_in_figtree_not_in_inter() {
        let mut ts = system();
        let word = "Hamburgefonstiv 1234";
        let text = glyph_ids(&mut ts, word, TextStyle::new(14.0, 400.0));
        let inter = glyph_ids(
            &mut ts,
            word,
            TextStyle::new(14.0, 400.0).family(Family::Named("Inter")),
        );
        let title = glyph_ids(&mut ts, word, display(26.0));
        assert_ne!(text, inter, "text still falls through to Inter");
        assert_ne!(title, text, "titles still draw in the text face");
        assert!(title.iter().chain(&text).all(|id| *id != 0));
    }

    #[test]
    fn a_page_title_never_passes_the_shared_size() {
        assert_eq!(display(40.0).size, PAGE_TITLE);
        assert_eq!(display(16.0).size, 16.0);
    }

    #[test]
    fn figtree_semibold_is_a_real_weight_of_the_variable_face() {
        let mut ts = system();
        let (regular, _) = ts.measure("Meeting notes", TextStyle::new(14.0, 400.0), 1.0);
        let (bold, _) = ts.measure("Meeting notes", TextStyle::new(14.0, 600.0), 1.0);
        assert!(
            bold > regular,
            "600 measured {bold}, 400 measured {regular}"
        );
    }

    /// Hindi and Telugu transcript lines must draw letters. Linux has no system fallback
    /// here, so the bundled Noto faces carry them; Windows reads Nirmala UI.
    #[test]
    fn a_hindi_and_a_telugu_line_draw_letters_not_boxes() {
        let mut ts = system();
        for line in [HINDI, TELUGU] {
            let ids = glyph_ids(&mut ts, line, TextStyle::new(14.0, 400.0));
            assert!(!ids.is_empty());
            assert!(
                ids.iter().all(|id| *id != 0),
                "missing glyphs in {line}: {ids:?}"
            );
        }
    }

    /// The detector sees a missing glyph: a code point no font has draws the box (id 0).
    /// Windows falls back to the system's own Indic faces, so the lines above cannot be
    /// made to fail here; this proves the check is not blind.
    #[test]
    fn the_box_detector_sees_a_missing_glyph() {
        let mut ts = system();
        let ids = glyph_ids(&mut ts, "\u{10fffd}", TextStyle::new(14.0, 400.0));
        assert!(ids.contains(&0), "{ids:?}");
    }

    /// Off Windows nothing but the bundled list carries Indic text, so it must name both.
    #[test]
    fn the_non_windows_font_list_carries_both_indic_faces() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(fonts().len(), bundled().len() + 2);
        assert!(SANS.contains("Noto Sans Devanagari") && SANS.contains("Noto Sans Telugu"));
    }

    fn ratio(a: Color, b: Color) -> f32 {
        let (hi, lo) = (
            a.luminance().max(b.luminance()),
            a.luminance().min(b.luminance()),
        );
        (hi + 0.05) / (lo + 0.05)
    }

    /// `top` painted over `base`.
    fn over(base: Color, top: Color) -> Color {
        let a = top.a;
        Color {
            r: base.r * (1.0 - a) + top.r * a,
            g: base.g * (1.0 - a) + top.g * a,
            b: base.b * (1.0 - a) + top.b * a,
            a: 1.0,
        }
    }

    /// Every text token against every fill it is drawn on, in both modes: at least 4.5:1.
    #[test]
    fn text_tokens_keep_four_and_a_half_to_one() {
        let mut failures = Vec::new();
        for ((preset, constructor), dark) in PRESETS
            .into_iter()
            .flat_map(|preset| [false, true].map(|dark| (preset, dark)))
        {
            let tokens = constructor(dark);
            let p = component_palette(&tokens);
            let k = kit(&p);
            // Sidebar popovers use the page surface or raised fill and the page text.
            let plain = [p.bg, p.card, p.raised, p.chip, p.sel, p.hit];
            let side = tokens.sidebar;
            let sidebar_fills = [side.fill, side.quiet_fill, side.selected_fill];
            let rows: Vec<(&str, Color, Vec<Color>)> = vec![
                ("ink", p.ink, plain.to_vec()),
                ("body", p.body, plain.to_vec()),
                ("soft", p.soft, plain.to_vec()),
                ("faint", p.faint, plain.to_vec()),
                ("sidebar ink", side.text.primary, sidebar_fills.to_vec()),
                ("sidebar body", side.text.body, sidebar_fills.to_vec()),
                ("sidebar soft", side.text.secondary, sidebar_fills.to_vec()),
                ("sidebar faint", side.text.tertiary, sidebar_fills.to_vec()),
                ("accent", p.accent, vec![p.bg, p.card, p.raised]),
                ("sidebar accent", side.focus_ring, vec![side.fill]),
                ("link", k.c.text_accent, vec![p.bg, p.card, p.raised]),
                (
                    "sidebar link",
                    side.link.unwrap_or(side.text.primary),
                    sidebar_fills.to_vec(),
                ),
                ("open", p.open, vec![p.bg, p.card, p.raised, p.chip]),
                ("sidebar open", side.open_status, vec![side.fill]),
                ("danger", p.danger, vec![p.bg, p.card, p.raised]),
                ("sidebar danger", side.danger, vec![side.fill]),
                ("ok", p.ok, vec![p.bg, p.card, p.okbg]),
                (
                    "warn",
                    p.warn,
                    vec![p.bg, p.card, p.raised, p.chip, p.warnbg],
                ),
                (
                    "sidebar warn",
                    side.warning,
                    vec![side.fill, side.quiet_fill],
                ),
                ("button ink", p.button_ink, vec![p.button_fill]),
                (
                    "button ink pressed",
                    p.button_ink,
                    vec![over(p.button_fill, p.button_primary_pressed)],
                ),
                (
                    "button ink hover",
                    p.button_ink,
                    vec![over(p.button_fill, p.button_primary_hover)],
                ),
                ("switch knob", p.switch_knob, vec![p.switch_on]),
                ("kit base", k.c.text_base, vec![p.card]),
                ("kit muted", k.c.text_muted, vec![p.card]),
                ("kit faint", k.c.text_faint, vec![p.card]),
                ("kit weak", k.c.text_weak, vec![p.card]),
                ("kit strong", k.c.text_strong, vec![p.card]),
            ];
            for (name, ink, fills) in rows {
                for (i, fill) in fills.into_iter().enumerate() {
                    let r = ratio(ink, fill);
                    if r < 4.5 {
                        failures.push(format!(
                            "{preset} {} {name} on fill {i}: {r:.2}",
                            if dark { "dark" } else { "light" }
                        ));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "below 4.5:1: {failures:#?}");
    }

    #[test]
    fn marks_keep_three_to_one_on_the_page() {
        for dark in [false, true] {
            let p = palette(dark);
            for (name, colour) in [("faint", p.faint), ("mark", p.mark)] {
                let contrast = ratio(colour, p.bg);
                assert!(contrast >= 3.0, "{name}, dark={dark}: {contrast:.2}:1");
            }
        }
    }

    /// The colour rules: the accent is coral, the focus ring clears 3:1 on every surface, the
    /// primary button is the accent, and selection is a grey with no hue.
    #[test]
    fn classic_accent_is_coral_the_primary_button_and_the_ring_and_selection_is_grey() {
        for dark in [false, true] {
            let p = component_palette(&classic::meaning(dark));
            assert_eq!(p.button_fill, p.accent);
            assert!(p.accent.r > p.accent.b * 1.8 && p.accent.r > p.accent.g * 1.4);
            for fill in [p.bg, p.card, p.side, p.sel] {
                assert!(ratio(p.accent, fill) >= 3.0, "ring on a surface");
            }
            // The old selection was blue-tinted (blue above red); the greys here are warm.
            assert!(p.sel.b <= p.sel.r, "selection is blue-tinted");
            assert_ne!(p.sel, p.accent);
        }
    }

    #[test]
    fn olive_accent_is_the_primary_button_and_the_ring_and_selection_is_warm() {
        for (dark, accent, selection) in
            [(false, "#55682a", "#e9e1cf"), (true, "#a9c25a", "#2c3424")]
        {
            let p = palette(dark);
            assert_eq!(p.accent, hex(accent));
            assert_eq!(p.button_fill, p.accent);
            for fill in [p.bg, p.card, p.side, p.sel] {
                assert!(ratio(p.accent, fill) >= 3.0, "ring on a surface");
            }
            assert_eq!(p.sel, hex(selection));
            assert!(p.sel.b <= p.sel.r, "selection is blue-tinted");
            assert_ne!(p.sel, p.accent);
        }
    }

    #[test]
    fn olive_sidebar_labels_keep_four_and_a_half_to_one() {
        for dark in [false, true] {
            let page = palette(dark);
            let p = sidebar_palette(dark, page);
            let tokens = olive(dark);
            assert_eq!(page.side, tokens.page.quiet_fill);
            assert_eq!(p.side, tokens.sidebar.fill);
            assert_eq!(p.chip, tokens.sidebar.quiet_fill);
            assert_eq!(p.sel, tokens.sidebar.selected_fill);
            assert_eq!(p.line, tokens.sidebar.border_subtle);
            for (ink, expected) in [
                (p.ink, tokens.sidebar.text.primary),
                (p.body, tokens.sidebar.text.body),
                (p.soft, tokens.sidebar.text.secondary),
                (p.faint, tokens.sidebar.text.tertiary),
            ] {
                assert_eq!(ink, expected);
                for fill in [p.side, p.chip, p.sel] {
                    let contrast = ratio(ink, fill);
                    assert!(contrast >= 4.5, "dark={dark}: {contrast:.2}:1");
                }
            }
        }
    }

    /// The accent is the main action and the focus ring only. Every use of `p.accent` outside
    /// this file sits on a focus ring (its line or the two before it say "focus"); links,
    /// progress, selection and decoration draw in greys.
    #[test]
    fn the_accent_is_used_only_for_focus_rings() {
        fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, hits);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && path.file_name().is_some_and(|n| n != "theme.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    let lines: Vec<&str> = text.lines().collect();
                    for (i, line) in lines.iter().enumerate() {
                        let near = &lines[i.saturating_sub(2)..=i];
                        if line.contains("p.accent")
                            && !near.iter().any(|l| l.to_lowercase().contains("focus"))
                        {
                            hits.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                        }
                    }
                }
            }
        }
        let mut hits = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui"),
            &mut hits,
        );
        assert!(hits.is_empty(), "accent outside a focus ring: {hits:#?}");
    }

    /// Small text on a raised surface in dark mode is at least #a9a69f, and the old
    /// #77746d is not a text token any more.
    #[test]
    fn dark_small_text_on_raised_surfaces_is_at_least_a9a69f() {
        let p = palette(true);
        let floor = rune::color::hex("#a9a69f").unwrap().luminance();
        let old = rune::color::hex("#77746d").unwrap().luminance();
        for ink in [p.ink, p.body, p.soft, p.faint] {
            assert!(ink.luminance() >= floor);
            assert_ne!(ink.luminance(), old);
        }
    }
}
