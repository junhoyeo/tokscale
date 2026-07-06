use ratatui::prelude::*;

use crate::tui::themes::Theme;

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

pub fn get_scanner_spans(frame: usize, theme: &Theme) -> Vec<Span<'static>> {
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

        let (ch, color) = if distance < TRAIL_LENGTH {
            ('■', TRAIL_COLORS[distance])
        } else {
            ('⬝', INACTIVE_COLOR)
        };

        spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(theme.color(color)),
        ));
    }

    spans
}

pub fn get_phase_message(phase: &str) -> &'static str {
    match phase {
        "idle" => "Initializing...",
        "parsing-sources" => "Scanning session data...",
        "loading-pricing" => "Loading pricing data...",
        "finalizing-report" => "Finalizing report...",
        "complete" => "Complete",
        _ => "Loading data...",
    }
}
