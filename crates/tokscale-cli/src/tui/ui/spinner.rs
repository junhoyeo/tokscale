use ratatui::prelude::*;

use crate::tui::themes::Theme;
use crate::ui_glyphs::Glyphs;

const WIDTH: usize = 8;
const HOLD_START: usize = 30;
const HOLD_END: usize = 9;
const TRAIL_LENGTH: usize = 4;

const TRAIL_COLORS: &[Color] = &[
    Color::Rgb(0, 255, 255), // #00FFFF - cyan
    Color::Rgb(0, 215, 215), // #00D7D7
    Color::Rgb(0, 175, 175), // #00AFAF
    Color::Rgb(0, 135, 135), // #008787
];
const INACTIVE_COLOR: Color = Color::Rgb(68, 68, 68);

pub struct ScannerState {
    pub position: usize,
    pub forward: bool,
}

pub fn get_scanner_state(frame: usize) -> ScannerState {
    let forward_frames = WIDTH;
    let backward_frames = WIDTH - 1;
    let total_cycle = forward_frames + HOLD_END + backward_frames + HOLD_START;
    let normalized = frame % total_cycle;

    if normalized < forward_frames {
        ScannerState {
            position: normalized,
            forward: true,
        }
    } else if normalized < forward_frames + HOLD_END {
        ScannerState {
            position: WIDTH - 1,
            forward: true,
        }
    } else if normalized < forward_frames + HOLD_END + backward_frames {
        ScannerState {
            position: WIDTH - 2 - (normalized - forward_frames - HOLD_END),
            forward: false,
        }
    } else {
        ScannerState {
            position: 0,
            forward: false,
        }
    }
}

pub fn get_scanner_spans(frame: usize, theme: &Theme, glyphs: Glyphs) -> Vec<Span<'static>> {
    let state = get_scanner_state(frame);
    let mut spans = Vec::with_capacity(WIDTH);

    for i in 0..WIDTH {
        let distance = if state.forward {
            if state.position >= i {
                state.position - i
            } else {
                usize::MAX
            }
        } else if i >= state.position {
            i - state.position
        } else {
            usize::MAX
        };

        let (symbol, color) = if distance < TRAIL_LENGTH {
            (glyphs.scanner_active, TRAIL_COLORS[distance])
        } else {
            (glyphs.scanner_inactive, INACTIVE_COLOR)
        };

        spans.push(Span::styled(
            symbol,
            Style::default().fg(theme.color(color)),
        ));
    }

    spans
}

pub fn get_phase_message(phase: &str, lang: crate::tui::i18n::TuiLanguage) -> &'static str {
    use crate::tui::i18n::{tr, MessageKey};
    match phase {
        "idle" => tr(lang, MessageKey::PhaseInitializing),
        "parsing-sources" => tr(lang, MessageKey::PhaseScanningSessions),
        "loading-pricing" => tr(lang, MessageKey::PhaseLoadingPricing),
        "finalizing-report" => tr(lang, MessageKey::PhaseFinalizingReport),
        "complete" => tr(lang, MessageKey::PhaseComplete),
        _ => tr(lang, MessageKey::PhaseLoadingData),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, widgets::Paragraph, Terminal};
    use unicode_width::UnicodeWidthStr;

    use super::*;
    use crate::tui::themes::{Theme, ThemeName};

    #[test]
    fn scanner_widget_has_the_same_width_in_cjk_locales() {
        let theme = Theme::from_name_for_current_terminal(ThemeName::Blue);
        let backend = TestBackend::new(WIDTH as u16, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new(Line::from(get_scanner_spans(
                        3,
                        &theme,
                        Glyphs::ascii_safe(),
                    ))),
                    frame.area(),
                );
            })
            .unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert_eq!(
            UnicodeWidthStr::width(rendered.as_str()),
            UnicodeWidthStr::width_cjk(rendered.as_str()),
            "scanner width changed in a CJK locale: {rendered:?}"
        );
    }
}
