//! Shared Settings and page list geometry.
use super::{icons, roles, theme::Palette};
use rune::{El, Icon, col, row};

pub fn panel(p: Palette) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .padding(1.0)
        .radius(12.0)
        .clip_rounded(12.0)
        .fill(p.card)
        .shadow(0.0, 8.0, p.shadow)
}

pub fn icon_tile(icon: &'static Icon, p: Palette) -> El {
    col()
        .w(34.0)
        .h(34.0)
        .center()
        .radius(8.0)
        .fill(p.chip)
        .child(icons::draw(icon, 18.0, p.soft))
}

pub fn row_shell() -> El {
    row().w_full().items_center().gaps(12.0, 10.0).padding(14.0)
}

const ROW_PADDING: f32 = 16.0;
const ROW_GAP: f32 = 12.0;

pub fn settings_row(has_help: bool) -> El {
    row()
        .w_full()
        .items_center()
        .flex_wrap()
        .gap(ROW_GAP)
        .px(ROW_PADDING)
        .py(11.0)
        // Old minimums were 44 without help and 56 with help; the 11px vertical
        // rhythm and the taller controls lift them to 52 and 64.
        .min_h(if has_help { 64.0 } else { 52.0 })
}

pub fn settings_copy(name: &str, p: Palette) -> El {
    settings_copy_with_title(settings_title(name, p))
}

pub fn settings_title(name: &str, p: Palette) -> El {
    rune::text(name)
        .font(roles::NAV.style().lh(19.0))
        .color(p.ink)
        .truncate()
        .min_w(0.0)
}

pub fn settings_copy_with_title(title: El) -> El {
    col()
        .grow(1.0)
        .min_w(120.0)
        .gap(roles::META_GAP)
        .child(title)
}

pub fn copy(title: El, help: &str, p: Palette) -> El {
    col()
        .grow(1.0)
        .min_w(170.0)
        .gap(4.0)
        .child(title)
        .child(roles::SMALL.wrap(help, p.soft))
}

/// The text column of a row whose lines the caller adds. It may shrink to nothing, so an
/// ellipsis name never pushes the row wider than its card.
pub fn copy_column() -> El {
    col().grow(1.0).min_w(0.0).gap(4.0)
}

pub fn fold_control(title: &str, key: &str, open: bool, p: Palette) -> El {
    super::widgets::pressable(
        row()
            .w_full()
            .items_center()
            .gap(8.0)
            .py(12.0)
            .child(icons::draw(
                if open {
                    &icons::CHEVRON_DOWN
                } else {
                    &icons::CHEVRON_RIGHT
                },
                icons::INLINE,
                p.soft,
            ))
            .child(roles::NAME.line(title, p.ink)),
    )
    .key(key)
    .label(title)
    .role("button")
    .expanded(open)
}

#[cfg(test)]
mod soft_card_tests {
    use super::*;
    use rune::{Cx, text::TextSystem};

    fn text_system() -> TextSystem {
        TextSystem::new(
            super::super::theme::fonts(),
            (super::super::theme::SANS, "Inter"),
        )
    }

    fn fixture(_: &Palette, _: &mut Cx<Palette>) -> El {
        let p = super::super::theme::palette(false);
        col().w_full().child(
            panel(p)
                .key("soft-card")
                .child(
                    settings_row(false)
                        .key("soft-row")
                        .child(settings_copy("Keep audio", p).key("soft-row-text"))
                        .child(rune::el().w(120.0).h(30.0).fill(p.card)),
                )
                .child(
                    settings_row(true)
                        .key("soft-row-help")
                        .child(
                            settings_copy("Stop forgotten recordings", p)
                                .child(roles::META.wrap("Helper line", p.soft))
                                .key("soft-row-help-text"),
                        )
                        .child(rune::el().w(120.0).h(30.0).fill(p.card)),
                ),
        )
    }

    #[test]
    fn soft_card_is_radius_12_with_tile_free_rows() {
        let p = super::super::theme::palette(false);
        let mut rt = rune::Runtime::with_text(
            p,
            fixture,
            (500.0, 400.0),
            rune::Env::default(),
            text_system(),
        );
        rt.frame(0.0);
        let nodes = rt.elements();
        let card = nodes
            .iter()
            .find(|e| e.key.as_deref() == Some("soft-card"))
            .expect("soft card");
        assert_eq!(card.radius, 12.0);
        assert_eq!(card.fill, p.card);
        let src = include_str!("cards.rs");
        let start = src.find("pub fn panel").expect("panel fn");
        let rest = &src[start..];
        let end = rest
            .find("\npub fn ")
            .map(|i| start + i)
            .unwrap_or(src.len());
        let panel_fn = &src[start..end];
        assert!(panel_fn.contains(".clip_rounded(12.0)"), "soft card clips");
        assert!(panel_fn.contains(".shadow("), "soft card has a shadow");
        assert!(!panel_fn.contains(".border("), "soft card has no border");
        // No element is keyed as a divider or tile; the helpers are gone (see below).
        assert!(
            !nodes.iter().any(|e| e
                .key
                .as_deref()
                .is_some_and(|k| k.contains("divider") || k.contains("tile"))),
            "divider or tile key remains"
        );
        for (key, text_key, want_h) in [
            // Old minimums were 44 without help and 56 with help; the 11px vertical
            // rhythm lifts them to 52 and 64.
            ("soft-row", "soft-row-text", 52.0),
            ("soft-row-help", "soft-row-help-text", 64.0),
        ] {
            let row = nodes
                .iter()
                .find(|e| e.key.as_deref() == Some(key))
                .unwrap_or_else(|| panic!("missing {key}"));
            assert_eq!(row.rect[3], want_h, "{key} height");
            let text = nodes
                .iter()
                .find(|e| e.key.as_deref() == Some(text_key))
                .unwrap_or_else(|| panic!("missing {text_key}"));
            // Old text offset with a 28px tile, 14px sides and a 12px gap was 54;
            // without the tile the text starts at the 16px side padding.
            assert!((text.rect[0] - row.rect[0] - 16.0).abs() < 0.5, "{key} x");
        }
    }
}
