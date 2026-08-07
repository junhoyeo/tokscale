use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

// Some diagnostics (e.g. cache save failures) fall back to a direct
// eprintln! when no tracing subscriber is guaranteed to be installed, so
// non-TUI commands still surface them. The TUI owns raw mode and the
// crossterm alternate screen for its whole lifetime, and a stray stdio
// write there corrupts the rendered display instead of being visible as a
// normal log line. Diagnostics routed through this module are held until the
// TUI releases the terminal, then written once the normal screen is restored.
static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);
static DEFERRED_STDERR: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn deferred_stderr() -> &'static Mutex<Vec<String>> {
    DEFERRED_STDERR.get_or_init(|| Mutex::new(Vec::new()))
}

fn transition_tui_active(active: bool) -> Vec<String> {
    // Coordinate the state transition with routing a diagnostic. Without the
    // shared lock, a writer could observe active, lose a race with the flush,
    // and enqueue a message after the queue had already been drained.
    let mut deferred = deferred_stderr()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    TUI_ACTIVE.store(active, Ordering::Relaxed);

    if active {
        Vec::new()
    } else {
        std::mem::take(&mut *deferred)
    }
}

pub fn set_tui_active(active: bool) {
    for message in transition_tui_active(active) {
        eprintln!("{message}");
    }
}

pub fn is_tui_active() -> bool {
    TUI_ACTIVE.load(Ordering::Relaxed)
}

fn route_stderr(message: String) -> Option<String> {
    let mut deferred = deferred_stderr()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if is_tui_active() {
        deferred.push(message);
        None
    } else {
        Some(message)
    }
}

pub(crate) fn emit_or_defer_stderr(message: String) {
    if let Some(message) = route_stderr(message) {
        eprintln!("{message}");
    }
}

#[cfg(test)]
pub(crate) fn take_deferred_stderr_for_test() -> Vec<String> {
    let mut deferred = deferred_stderr()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *deferred)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn defers_stderr_until_the_tui_is_inactive() {
        let previous = is_tui_active();

        assert!(
            transition_tui_active(false).is_empty(),
            "the test must not inherit deferred diagnostics"
        );
        assert!(transition_tui_active(true).is_empty());
        assert!(is_tui_active());

        let marker = "deferred TUI diagnostic".to_string();
        assert!(route_stderr(marker.clone()).is_none());

        assert_eq!(transition_tui_active(false), vec![marker]);
        assert!(!is_tui_active());
        assert!(transition_tui_active(false).is_empty());
        assert_eq!(
            route_stderr("immediate diagnostic".to_string()),
            Some("immediate diagnostic".to_string())
        );

        let _ = transition_tui_active(previous);
    }
}
