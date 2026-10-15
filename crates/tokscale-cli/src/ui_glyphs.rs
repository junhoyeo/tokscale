//! Width-stable glyphs used by app-owned TUI chrome.
//!
//! Every symbol here is one cell under both `unicode-width` ambients and is
//! present directly in Menlo, SF Mono, JetBrains Mono, and DejaVu Sans Mono.
//! Keep localized or user-provided text outside this policy: it applies to the
//! layout glyphs Tokscale controls.

pub(crate) const AMBIENT_STABLE_SEPARATOR: &str = "∙";
pub(crate) const AMBIENT_STABLE_SPACED_SEPARATOR: &str = " ∙ ";
pub(crate) const AMBIENT_STABLE_VERTICAL_NAVIGATION: &str = "▴▾";
pub(crate) const AMBIENT_STABLE_HORIZONTAL_NAVIGATION: &str = "◂▸";
pub(crate) const AMBIENT_STABLE_SCROLL_UP: &str = "▴";
pub(crate) const AMBIENT_STABLE_SCROLL_DOWN: &str = "▾";

// ASCII keeps selected controls in the active monospace font. The visually
// closer U+25AA BLACK SMALL SQUARE is missing from SF Mono.
pub(crate) const AMBIENT_STABLE_SELECTED_RADIO: &str = "(*)";
pub(crate) const AMBIENT_STABLE_SELECTED_CHECKBOX: &str = "[*]";

// This filled/open pair is covered by all four reference monospace families.
// The former U+25A0/U+2B1D scanner pair was ambiguous/fallback-prone.
pub(crate) const AMBIENT_STABLE_SCANNER_ACTIVE: &str = "∙";
pub(crate) const AMBIENT_STABLE_SCANNER_INACTIVE: &str = "◦";

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::*;

    #[test]
    fn app_owned_chrome_glyphs_are_width_stable() {
        for glyph in [
            AMBIENT_STABLE_SEPARATOR,
            AMBIENT_STABLE_SPACED_SEPARATOR,
            AMBIENT_STABLE_VERTICAL_NAVIGATION,
            AMBIENT_STABLE_HORIZONTAL_NAVIGATION,
            AMBIENT_STABLE_SCROLL_UP,
            AMBIENT_STABLE_SCROLL_DOWN,
            AMBIENT_STABLE_SELECTED_RADIO,
            AMBIENT_STABLE_SELECTED_CHECKBOX,
            AMBIENT_STABLE_SCANNER_ACTIVE,
            AMBIENT_STABLE_SCANNER_INACTIVE,
        ] {
            let width = UnicodeWidthStr::width(glyph);
            assert_eq!(
                width,
                UnicodeWidthStr::width_cjk(glyph),
                "chrome glyph must not change width in a CJK locale: {glyph:?}"
            );
            assert_eq!(
                width,
                glyph.chars().count(),
                "every chrome scalar must occupy exactly one cell: {glyph:?}"
            );
        }
    }
}
