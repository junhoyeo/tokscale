//! Process-wide opt-out from networking, installed before CLI workers start.
//!
//! The policy is monotonic: a report or background task cannot turn networking
//! back on. This does not sandbox third-party processes; callers must also avoid
//! launching network-capable helpers when offline.

use std::sync::atomic::{AtomicBool, Ordering};

static OFFLINE: AtomicBool = AtomicBool::new(false);

/// Disable networking for the rest of this process.
pub fn go_offline() {
    OFFLINE.store(true, Ordering::Release);
}

pub fn is_offline() -> bool {
    OFFLINE.load(Ordering::Acquire)
}

pub fn require_online(action: &str) -> Result<(), String> {
    if is_offline() {
        Err(format!(
            "{action} requires network access; omit --offline to run it"
        ))
    } else {
        Ok(())
    }
}
