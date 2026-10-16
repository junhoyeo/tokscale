//! Active symbols for the interactive terminal UI.
//!
//! Unicode mode restores Tokscale's original crafted chrome. ASCII mode uses
//! only characters whose width is invariant under narrow and wide East Asian
//! ambiguous-width policies.

use ratatui::symbols::border::Set as BorderSet;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation};

use crate::tui::glyph_probe::AmbientWidth;
use crate::tui::settings::GlyphMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChartFill {
    Foreground,
    Background,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Glyphs {
    pub border: BorderSet,
    pub scrollbar_up: &'static str,
    pub scrollbar_down: &'static str,
    pub range_down: &'static str,
    pub scrollbar_track: &'static str,
    pub scrollbar_thumb: &'static str,
    pub tab_divider: &'static str,
    pub axis_vertical: char,
    pub axis_horizontal: char,
    pub ellipsis: &'static str,
    pub missing: &'static str,
    pub infinity: &'static str,
    pub sort_up: &'static str,
    pub sort_down: &'static str,
    pub vertical_navigation: &'static str,
    pub horizontal_navigation: &'static str,
    pub separator: &'static str,
    pub compact_separator: &'static str,
    pub spaced_separator: &'static str,
    pub scanner_active: &'static str,
    pub scanner_inactive: &'static str,
    pub selected_radio: &'static str,
    pub selected_checkbox: &'static str,
    pub legend_dot: &'static str,
    pub chart_blocks: &'static [char; 9],
    pub chart_fill: ChartFill,
    pub ratio_full: &'static str,
    pub ratio_trace: &'static str,
    pub ratio_empty: &'static str,
    pub bar_full: &'static str,
    pub bar_empty: &'static str,
    pub heat_cell: &'static str,
    pub heat_selected: &'static str,
    pub heat_empty: &'static str,
}

const UNICODE_BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const ASCII_BLOCKS: [char; 9] = [' '; 9];

impl Glyphs {
    pub const fn resolve(mode: GlyphMode, ambient_width: AmbientWidth) -> Self {
        match mode {
            GlyphMode::Unicode => Self::unicode(),
            GlyphMode::Ascii => Self::ascii_safe(),
            GlyphMode::Auto => match ambient_width {
                AmbientWidth::Wide => Self::ascii_safe(),
                AmbientWidth::Narrow => Self::unicode(),
                // Most terminals render ambiguous characters narrow. A missing
                // DSR reply must not make the common case lose crafted UI.
                AmbientWidth::Unknown => Self::unicode(),
            },
        }
    }

    pub const fn unicode() -> Self {
        Self {
            border: ratatui::symbols::border::PLAIN,
            scrollbar_up: "▲",
            scrollbar_down: "▼",
            range_down: "↓",
            scrollbar_track: "║",
            scrollbar_thumb: "█",
            tab_divider: " │ ",
            axis_vertical: '│',
            axis_horizontal: '─',
            ellipsis: "…",
            missing: "—",
            infinity: "∞",
            sort_up: " ▲",
            sort_down: " ▼",
            vertical_navigation: "↑↓",
            horizontal_navigation: "←→",
            separator: "•",
            compact_separator: "·",
            spaced_separator: " • ",
            scanner_active: "■",
            scanner_inactive: "⬝",
            selected_radio: "(●)",
            selected_checkbox: "[●]",
            legend_dot: "●",
            chart_blocks: &UNICODE_BLOCKS,
            chart_fill: ChartFill::Foreground,
            ratio_full: "█",
            ratio_trace: "▏",
            ratio_empty: "·",
            bar_full: "█",
            bar_empty: "░",
            heat_cell: "██",
            heat_selected: "▓▓",
            heat_empty: "· ",
        }
    }

    pub const fn ascii_safe() -> Self {
        Self {
            border: BorderSet {
                vertical_left: "|",
                vertical_right: "|",
                horizontal_top: "-",
                horizontal_bottom: "-",
                top_left: "+",
                top_right: "+",
                bottom_left: "+",
                bottom_right: "+",
            },
            scrollbar_up: "▴",
            scrollbar_down: "▾",
            range_down: "▾",
            scrollbar_track: "|",
            scrollbar_thumb: "▮",
            tab_divider: " | ",
            axis_vertical: '|',
            axis_horizontal: '-',
            ellipsis: "⋯",
            missing: "-",
            infinity: "inf",
            sort_up: " ▴",
            sort_down: " ▾",
            vertical_navigation: "▴▾",
            horizontal_navigation: "◂▸",
            separator: "∙",
            compact_separator: "∙",
            spaced_separator: " ∙ ",
            scanner_active: "∙",
            scanner_inactive: "◦",
            selected_radio: "(*)",
            selected_checkbox: "[*]",
            legend_dot: "o",
            chart_blocks: &ASCII_BLOCKS,
            chart_fill: ChartFill::Background,
            ratio_full: " ",
            ratio_trace: " ",
            ratio_empty: ".",
            bar_full: "#",
            bar_empty: ".",
            heat_cell: "  ",
            heat_selected: "[]",
            heat_empty: ". ",
        }
    }

    pub fn scrollbar(self) -> Scrollbar<'static> {
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some(self.scrollbar_up))
            .end_symbol(Some(self.scrollbar_down))
            .track_symbol(Some(self.scrollbar_track))
            .thumb_symbol(self.scrollbar_thumb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn ascii_safe_symbols_are_width_invariant() {
        let glyphs = Glyphs::ascii_safe();
        for symbol in [
            glyphs.scrollbar_up,
            glyphs.scrollbar_down,
            glyphs.range_down,
            glyphs.scrollbar_track,
            glyphs.scrollbar_thumb,
            glyphs.tab_divider,
            glyphs.ellipsis,
            glyphs.missing,
            glyphs.infinity,
            glyphs.sort_up,
            glyphs.sort_down,
            glyphs.vertical_navigation,
            glyphs.horizontal_navigation,
            glyphs.separator,
            glyphs.compact_separator,
            glyphs.spaced_separator,
            glyphs.scanner_active,
            glyphs.scanner_inactive,
            glyphs.selected_radio,
            glyphs.selected_checkbox,
            glyphs.legend_dot,
            glyphs.ratio_full,
            glyphs.ratio_trace,
            glyphs.ratio_empty,
            glyphs.bar_full,
            glyphs.bar_empty,
            glyphs.heat_cell,
            glyphs.heat_selected,
            glyphs.heat_empty,
        ] {
            assert_eq!(
                UnicodeWidthStr::width(symbol),
                UnicodeWidthStr::width_cjk(symbol),
                "{symbol:?}"
            );
        }
        for symbol in glyphs.chart_blocks {
            let symbol = symbol.to_string();
            assert_eq!(
                UnicodeWidthStr::width(symbol.as_str()),
                UnicodeWidthStr::width_cjk(symbol.as_str())
            );
        }
        for symbol in [
            glyphs.border.vertical_left,
            glyphs.border.vertical_right,
            glyphs.border.horizontal_top,
            glyphs.border.horizontal_bottom,
            glyphs.border.top_left,
            glyphs.border.top_right,
            glyphs.border.bottom_left,
            glyphs.border.bottom_right,
        ] {
            assert_eq!(
                UnicodeWidthStr::width(symbol),
                UnicodeWidthStr::width_cjk(symbol)
            );
        }
    }

    #[test]
    fn unicode_restores_original_crafted_symbols() {
        let g = Glyphs::unicode();
        assert_eq!(g.border.top_left, "┌");
        assert_eq!(g.tab_divider, " │ ");
        assert_eq!(g.range_down, "↓");
        assert_eq!((g.sort_up, g.sort_down), (" ▲", " ▼"));
        assert_eq!(
            (g.vertical_navigation, g.horizontal_navigation),
            ("↑↓", "←→")
        );
        assert_eq!((g.scanner_active, g.scanner_inactive), ("■", "⬝"));
        assert_eq!((g.ellipsis, g.missing, g.infinity), ("…", "—", "∞"));
        assert_eq!(g.legend_dot, "●");
        assert_eq!(g.chart_blocks, &UNICODE_BLOCKS);
    }

    #[test]
    fn mode_resolution_respects_forced_values_and_auto_fallback() {
        assert_eq!(
            Glyphs::resolve(GlyphMode::Auto, AmbientWidth::Narrow)
                .border
                .top_left,
            "┌"
        );
        assert_eq!(
            Glyphs::resolve(GlyphMode::Auto, AmbientWidth::Wide)
                .border
                .top_left,
            "+"
        );
        assert_eq!(
            Glyphs::resolve(GlyphMode::Auto, AmbientWidth::Unknown)
                .border
                .top_left,
            "┌"
        );
        assert_eq!(
            Glyphs::resolve(GlyphMode::Unicode, AmbientWidth::Wide)
                .border
                .top_left,
            "┌"
        );
        assert_eq!(
            Glyphs::resolve(GlyphMode::Ascii, AmbientWidth::Narrow)
                .border
                .top_left,
            "+"
        );
    }

    #[test]
    fn tui_renderers_do_not_hardcode_owned_chrome() {
        fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    rust_files(&path, out);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    out.push(path);
                }
            }
        }

        let mut files = Vec::new();
        rust_files(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui"),
            &mut files,
        );
        let forbidden = [
            "\"●\"",
            "\"(●)\"",
            "\"[●]\"",
            "\"▲\"",
            "\"▼\"",
            "\"▴\"",
            "\"▾\"",
            "\"↑↓\"",
            "\"←→\"",
            "\"◂▸\"",
            "\"∙\"",
            "\" • \"",
            "\"■\"",
            "\"⬝\"",
            "\"◦\"",
            "\"█\"",
            "\"░\"",
            "\"⋯\"",
            "\"…\"",
            "'─'",
            "\"─\"",
            "\"│\"",
            "\"── \"",
            "\"—\"",
            "\"∞\"",
            "\\u{2014}",
            "\\u{221e}",
        ];

        for path in files {
            // The probe glyph is intentionally ambiguous; it is measurement,
            // not rendered chrome.
            if path
                .file_name()
                .is_some_and(|name| name == "glyph_probe.rs")
            {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let production = source.split("#[cfg(test)]").next().unwrap_or(&source);
            for (line_number, line) in production.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for pattern in forbidden {
                    assert!(
                        !line.contains(pattern),
                        "{}:{} hardcodes owned TUI chrome {pattern}: {line}",
                        path.display(),
                        line_number + 1,
                    );
                }
            }
        }
    }
}
