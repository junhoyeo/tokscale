use std::sync::atomic::{AtomicBool, Ordering};

// Some diagnostics (e.g. cache save failures) fall back to a direct
// eprintln! when no tracing subscriber is guaranteed to be installed, so
// non-TUI commands still surface them. The TUI owns raw mode and the
// crossterm alternate screen for its whole lifetime, and a stray stdio
// write there corrupts the rendered display instead of being visible as a
// normal log line. This flag lets that diagnostic code suppress the raw
// stdio fallback while the TUI holds the terminal.
static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn set_tui_active(active: bool) {
    TUI_ACTIVE.store(active, Ordering::Relaxed);
}

pub fn is_tui_active() -> bool {
    TUI_ACTIVE.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn round_trips() {
        let previous = is_tui_active();
        set_tui_active(true);
        assert!(is_tui_active());
        set_tui_active(false);
        assert!(!is_tui_active());
        set_tui_active(previous);
    }
}
