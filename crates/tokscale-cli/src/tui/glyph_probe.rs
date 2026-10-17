//! Detect how the active terminal renders East Asian Ambiguous characters.
//!
//! The probe runs only after raw mode and the alternate screen are active. It
//! writes one ambiguous-width scalar at a known location, asks the terminal for
//! its cursor position (DSR), and erases the probe before the first frame.

use std::io::{self, IsTerminal, Write};
#[cfg(any(unix, test))]
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
#[cfg(any(unix, test))]
use crossterm::event::{KeyCode, KeyModifiers};

#[cfg(any(unix, test))]
const PROBE_TIMEOUT: Duration = Duration::from_millis(225);
const PROBE_PREFIX: &[u8] = b"\x1b[1;1H\x1b[2K";
const PROBE_GLYPH: &[u8] = "·".as_bytes();
#[cfg(any(unix, test))]
const DSR_QUERY: &[u8] = b"\x1b[6n";
const PROBE_CLEANUP: &[u8] = b"\x1b[1;1H\x1b[2K\x1b[1;1H";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum AmbientWidth {
    Narrow,
    Wide,
    #[default]
    Unknown,
}

#[derive(Debug, Default)]
pub(crate) struct ProbeResult {
    pub width: AmbientWidth,
    pub pending_keys: Vec<KeyEvent>,
}

/// Probe the real terminal attached to stdin/stdout.
///
/// Any unsupported terminal, I/O error, or missing/invalid DSR reply degrades
/// to `Unknown`. The deadline is deliberately short so a terminal multiplexer
/// which does not pass DSR replies through cannot delay startup noticeably.
pub(crate) fn probe_ambient_width(output: &mut impl Write) -> ProbeResult {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return ProbeResult::default();
    }

    probe_tty(output).unwrap_or_default()
}

#[cfg(unix)]
fn probe_tty(output: &mut impl Write) -> io::Result<ProbeResult> {
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;

    // Use the controlling terminal rather than assuming fd 0 is suitable for
    // direct reads. The IsTerminal checks above still ensure redirected TUI
    // launches do not emit a probe at all.
    let tty = OpenOptions::new().read(true).open("/dev/tty")?;
    let nonblocking = NonblockingGuard::new(tty.as_raw_fd())?;
    // Never steal input that was already waiting before the probe began. Auto
    // falls back to Unknown and the normal crossterm reader receives it intact.
    if input_is_ready(nonblocking.fd)? {
        return Ok(ProbeResult::default());
    }

    let query_result = output
        .write_all(PROBE_PREFIX)
        .and_then(|()| output.write_all(PROBE_GLYPH))
        .and_then(|()| output.write_all(DSR_QUERY))
        .and_then(|()| output.flush());

    let read = if query_result.is_ok() {
        read_dsr_with_timeout(PROBE_TIMEOUT, |remaining| {
            poll_and_read(nonblocking.fd, remaining)
        })
    } else {
        DsrRead::default()
    };

    // Always remove the probe, even when the terminal ignored DSR or returned
    // an unexpected position. A full TUI frame has not been drawn yet.
    let cleanup_result = output
        .write_all(PROBE_CLEANUP)
        .and_then(|()| output.flush());
    query_result?;
    cleanup_result?;

    let mut pending = read.pending;
    if read.position.is_none() {
        strip_partial_dsr(&mut pending);
    }
    Ok(ProbeResult {
        width: classify_position(read.position),
        pending_keys: decode_pending_keys(&pending),
    })
}

#[cfg(unix)]
struct NonblockingGuard {
    fd: std::os::fd::RawFd,
    original_flags: libc::c_int,
}

#[cfg(unix)]
impl NonblockingGuard {
    fn new(fd: std::os::fd::RawFd) -> io::Result<Self> {
        // SAFETY: `fd` is owned by the live File in `probe_tty`. fcntl neither
        // takes ownership nor retains the pointer/value passed to it.
        let original_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if original_flags == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: same live descriptor; flags are the existing valid flags plus
        // O_NONBLOCK and are restored when this guard drops.
        if unsafe { libc::fcntl(fd, libc::F_SETFL, original_flags | libc::O_NONBLOCK) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd, original_flags })
    }
}

#[cfg(unix)]
impl Drop for NonblockingGuard {
    fn drop(&mut self) {
        // SAFETY: the File outlives this guard because it is declared first in
        // `probe_tty`. Restoration is best effort on an already-failing path.
        let _ = unsafe { libc::fcntl(self.fd, libc::F_SETFL, self.original_flags) };
    }
}

#[cfg(unix)]
fn input_is_ready(fd: std::os::fd::RawFd) -> io::Result<bool> {
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `descriptor` is a valid one-element pollfd array and poll does
    // not retain its pointer.
    let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result > 0)
    }
}

#[cfg(unix)]
fn poll_and_read(fd: std::os::fd::RawFd, remaining: Duration) -> io::Result<Option<Vec<u8>>> {
    let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };

    // SAFETY: `descriptor` is a valid one-element pollfd array for the
    // duration of this call. `poll` does not retain its pointer.
    let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
    if ready > 0 {
        let mut bytes = [0_u8; 64];
        // SAFETY: `bytes` is writable for its full length and `fd` is a
        // live nonblocking controlling-terminal descriptor.
        let count = unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), bytes.len()) };
        if count > 0 {
            return Ok(Some(bytes[..count as usize].to_vec()));
        }
        if count == 0 {
            return Ok(None);
        }
        return Err(io::Error::last_os_error());
    }
    if ready == 0 {
        return Ok(None);
    }
    // Do not restart with the original timeout after EINTR: doing so could
    // exceed the probe's absolute deadline under repeated signals.
    Err(io::Error::last_os_error())
}

#[cfg(windows)]
fn probe_tty(output: &mut impl Write) -> io::Result<ProbeResult> {
    let query_result = output
        .write_all(PROBE_PREFIX)
        .and_then(|()| output.write_all(PROBE_GLYPH))
        .and_then(|()| output.flush());

    // Windows consoles expose the cursor position synchronously through the
    // console API, so crossterm does not need an input-side DSR round trip.
    let position = if query_result.is_ok() {
        crossterm::cursor::position()
            .ok()
            .map(|(column, row)| (row.saturating_add(1), column.saturating_add(1)))
    } else {
        None
    };

    let cleanup_result = output
        .write_all(PROBE_CLEANUP)
        .and_then(|()| output.flush());
    query_result?;
    cleanup_result?;
    Ok(ProbeResult {
        width: classify_position(position),
        pending_keys: Vec::new(),
    })
}

#[cfg(not(any(unix, windows)))]
fn probe_tty(_output: &mut impl Write) -> io::Result<ProbeResult> {
    Ok(ProbeResult::default())
}

fn classify_position(position: Option<(u16, u16)>) -> AmbientWidth {
    match position {
        Some((1, 2)) => AmbientWidth::Narrow,
        Some((1, 3)) => AmbientWidth::Wide,
        _ => AmbientWidth::Unknown,
    }
}

#[cfg(any(unix, test))]
#[derive(Debug, Default, PartialEq, Eq)]
struct DsrRead {
    position: Option<(u16, u16)>,
    pending: Vec<u8>,
}

#[cfg(any(unix, test))]
fn read_dsr_with_timeout<F>(timeout: Duration, mut next_chunk: F) -> DsrRead
where
    F: FnMut(Duration) -> io::Result<Option<Vec<u8>>>,
{
    let Some(deadline) = Instant::now().checked_add(timeout) else {
        return DsrRead::default();
    };
    let mut parser = DsrParser::default();
    let mut position = None;
    let mut scan_from = 0;

    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return DsrRead {
                position,
                pending: parser.pending,
            };
        };
        if remaining.is_zero() {
            return DsrRead {
                position,
                pending: parser.pending,
            };
        }

        match next_chunk(remaining) {
            Ok(Some(bytes)) => {
                parser.pending.extend_from_slice(&bytes);
                if position.is_none() {
                    if let Some((found, response_range)) = find_dsr_from(&parser.pending, scan_from)
                    {
                        parser.pending.drain(response_range);
                        position = Some(found);
                    } else {
                        // A response can begin at the final ESC of this chunk.
                        // Rescan only that boundary byte with the next chunk.
                        scan_from = parser.pending.len().saturating_sub(32);
                    }
                }
                // A key can be split across the same fd reads as the DSR. Once
                // the response is gone, keep reading within the original
                // deadline until every retained key sequence is complete.
                if position.is_some() && pending_input_is_complete(&parser.pending) {
                    return DsrRead {
                        position,
                        pending: parser.pending,
                    };
                }
            }
            Ok(None) | Err(_) => {
                return DsrRead {
                    position,
                    pending: parser.pending,
                };
            }
        }
    }
}

#[cfg(any(unix, test))]
#[derive(Default)]
struct DsrParser {
    pending: Vec<u8>,
}

#[cfg(test)]
impl DsrParser {
    fn push(&mut self, bytes: &[u8]) -> Option<((u16, u16), std::ops::Range<usize>)> {
        self.pending.extend_from_slice(bytes);
        find_dsr(&self.pending)
    }
}

#[cfg(test)]
fn parse_dsr(bytes: &[u8]) -> Option<(u16, u16)> {
    find_dsr(bytes).map(|(position, _)| position)
}

#[cfg(test)]
fn find_dsr(bytes: &[u8]) -> Option<((u16, u16), std::ops::Range<usize>)> {
    find_dsr_from(bytes, 0)
}

#[cfg(any(unix, test))]
fn find_dsr_from(bytes: &[u8], scan_from: usize) -> Option<((u16, u16), std::ops::Range<usize>)> {
    for start in scan_from..bytes.len().saturating_sub(1) {
        if bytes.get(start..start + 2) != Some(b"\x1b[") {
            continue;
        }

        let mut cursor = start + 2;
        let row_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == row_start || bytes.get(cursor) != Some(&b';') {
            continue;
        }
        let Some(row) = parse_u16_ascii(&bytes[row_start..cursor]) else {
            continue;
        };

        cursor += 1;
        let column_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == column_start || bytes.get(cursor) != Some(&b'R') {
            continue;
        }
        let Some(column) = parse_u16_ascii(&bytes[column_start..cursor]) else {
            continue;
        };
        return Some(((row, column), start..cursor + 1));
    }
    None
}

#[cfg(any(unix, test))]
fn parse_u16_ascii(bytes: &[u8]) -> Option<u16> {
    bytes.iter().try_fold(0_u16, |value, byte| {
        value
            .checked_mul(10)?
            .checked_add(u16::from(byte.checked_sub(b'0')?))
    })
}

#[cfg(any(unix, test))]
fn strip_partial_dsr(bytes: &mut Vec<u8>) {
    let Some(start) = bytes.windows(2).rposition(|window| window == b"\x1b[") else {
        return;
    };
    let tail = &bytes[start + 2..];
    let semicolons = tail.iter().filter(|byte| **byte == b';').count();
    if semicolons <= 1
        && tail
            .iter()
            .all(|byte| byte.is_ascii_digit() || *byte == b';')
    {
        bytes.truncate(start);
    }
}

#[cfg(any(unix, test))]
fn pending_input_is_complete(bytes: &[u8]) -> bool {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"\x1b[200~") {
            let Some(end) = find_subslice(&bytes[index + 6..], b"\x1b[201~") else {
                return false;
            };
            index += 6 + end + 6;
            continue;
        }
        if bytes[index] == b'\x1b' {
            let Some((_, consumed)) = decode_escape_key(&bytes[index..]) else {
                return false;
            };
            index += consumed;
            continue;
        }
        if bytes[index].is_ascii() {
            index += 1;
            continue;
        }
        match std::str::from_utf8(&bytes[index..]) {
            Ok(_) => return true,
            Err(error) if error.valid_up_to() > 0 => index += error.valid_up_to(),
            Err(error) if error.error_len().is_none() => return false,
            Err(error) => index += error.error_len().unwrap_or(1),
        }
    }
    true
}

#[cfg(any(unix, test))]
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Convert non-DSR bytes read during the probe back into the key events the
/// event loop would otherwise have received. The probe is short, but startup
/// typing must not disappear merely because a cursor report shared its read.
#[cfg(any(unix, test))]
fn decode_pending_keys(bytes: &[u8]) -> Vec<KeyEvent> {
    let mut keys = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        // Bracketed paste is ignored by the live EventHandler. Drop the whole
        // wrapped payload here too, rather than turning pasted hotkeys into
        // commands during startup.
        if bytes[index..].starts_with(b"\x1b[200~") {
            let Some(end) = find_subslice(&bytes[index + 6..], b"\x1b[201~") else {
                break;
            };
            index += 6 + end + 6;
            continue;
        }
        if bytes[index] == b'\x1b' {
            if let Some((key, consumed)) = decode_escape_key(&bytes[index..]) {
                if let Some(key) = key {
                    keys.push(key);
                }
                index += consumed;
                continue;
            }
            keys.push(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            index += 1;
            continue;
        }

        let byte = bytes[index];
        let event = match byte {
            b'\r' | b'\n' => Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            b'\t' => Some(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            0x7f | 0x08 => Some(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            1..=26 => Some(KeyEvent::new(
                KeyCode::Char(char::from(b'a' + byte - 1)),
                KeyModifiers::CONTROL,
            )),
            0x20..=0x7e => Some(KeyEvent::new(
                KeyCode::Char(char::from(byte)),
                KeyModifiers::NONE,
            )),
            _ => None,
        };
        if let Some(event) = event {
            keys.push(event);
            index += 1;
            continue;
        }

        let tail = &bytes[index..];
        match std::str::from_utf8(tail) {
            Ok(text) => {
                for ch in text.chars() {
                    keys.push(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
                }
                break;
            }
            Err(error) if error.valid_up_to() > 0 => {
                let valid = &tail[..error.valid_up_to()];
                let text = std::str::from_utf8(valid)
                    .expect("valid_up_to always identifies a valid UTF-8 prefix");
                for ch in text.chars() {
                    keys.push(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
                }
                index += error.valid_up_to();
            }
            Err(error) => {
                let skip = error.error_len().unwrap_or(1).min(tail.len());
                index += skip;
            }
        }
    }
    keys
}

/// Decode one escape-prefixed key. `None` as the event means a terminal control
/// wrapper (currently bracketed-paste start/end) that should be removed while
/// preserving the text between its two markers.
#[cfg(any(unix, test))]
fn decode_csi_key(bytes: &[u8]) -> Option<KeyEvent> {
    let length = csi_sequence_len(bytes)?;
    let final_byte = bytes[length - 1];
    let params = std::str::from_utf8(&bytes[..length - 1]).ok()?;
    let numbers: Vec<u16> = if params.is_empty() {
        Vec::new()
    } else {
        params
            .split(';')
            .map(|part| part.parse::<u16>().ok())
            .collect::<Option<_>>()?
    };
    let modifiers = csi_modifiers(numbers.get(1).copied());
    let code = match final_byte {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'Z' => KeyCode::BackTab,
        b'~' => match numbers.first().copied()? {
            2 => KeyCode::Insert,
            3 => KeyCode::Delete,
            5 => KeyCode::PageUp,
            6 => KeyCode::PageDown,
            15 => KeyCode::F(5),
            17 => KeyCode::F(6),
            18 => KeyCode::F(7),
            19 => KeyCode::F(8),
            20 => KeyCode::F(9),
            21 => KeyCode::F(10),
            23 => KeyCode::F(11),
            24 => KeyCode::F(12),
            _ => return None,
        },
        _ => return None,
    };
    Some(KeyEvent::new(code, modifiers))
}

#[cfg(any(unix, test))]
fn csi_modifiers(value: Option<u16>) -> KeyModifiers {
    match value.unwrap_or(1) {
        2 => KeyModifiers::SHIFT,
        3 => KeyModifiers::ALT,
        4 => KeyModifiers::SHIFT | KeyModifiers::ALT,
        5 => KeyModifiers::CONTROL,
        6 => KeyModifiers::SHIFT | KeyModifiers::CONTROL,
        7 => KeyModifiers::ALT | KeyModifiers::CONTROL,
        8 => KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL,
        _ => KeyModifiers::NONE,
    }
}

#[cfg(any(unix, test))]
fn csi_sequence_len(bytes: &[u8]) -> Option<usize> {
    bytes
        .iter()
        .position(|byte| (0x40..=0x7e).contains(byte))
        .map(|index| index + 1)
}

#[cfg(any(unix, test))]
fn decode_escape_key(bytes: &[u8]) -> Option<(Option<KeyEvent>, usize)> {
    let simple = |code, consumed| Some((Some(KeyEvent::new(code, KeyModifiers::NONE)), consumed));
    match bytes {
        [0x1b, b'[', b'A', ..] | [0x1b, b'O', b'A', ..] => simple(KeyCode::Up, 3),
        [0x1b, b'[', b'B', ..] | [0x1b, b'O', b'B', ..] => simple(KeyCode::Down, 3),
        [0x1b, b'[', b'C', ..] | [0x1b, b'O', b'C', ..] => simple(KeyCode::Right, 3),
        [0x1b, b'[', b'D', ..] | [0x1b, b'O', b'D', ..] => simple(KeyCode::Left, 3),
        [0x1b, b'[', b'H', ..] | [0x1b, b'O', b'H', ..] => simple(KeyCode::Home, 3),
        [0x1b, b'[', b'F', ..] | [0x1b, b'O', b'F', ..] => simple(KeyCode::End, 3),
        [0x1b, b'[', b'2', b'~', ..] => simple(KeyCode::Insert, 4),
        [0x1b, b'[', b'3', b'~', ..] => simple(KeyCode::Delete, 4),
        [0x1b, b'[', b'5', b'~', ..] => simple(KeyCode::PageUp, 4),
        [0x1b, b'[', b'6', b'~', ..] => simple(KeyCode::PageDown, 4),
        [0x1b, b'[', rest @ ..] => {
            csi_sequence_len(rest).map(|length| (decode_csi_key(&rest[..length]), length + 2))
        }
        [0x1b, b'O', b'P', ..] => simple(KeyCode::F(1), 3),
        [0x1b, b'O', b'Q', ..] => simple(KeyCode::F(2), 3),
        [0x1b, b'O', b'R', ..] => simple(KeyCode::F(3), 3),
        [0x1b, b'O', b'S', ..] => simple(KeyCode::F(4), 3),
        [0x1b, b'O', _, ..] => Some((None, 3)),
        [0x1b, byte, ..] if byte.is_ascii_graphic() => Some((
            Some(KeyEvent::new(
                KeyCode::Char(char::from(*byte)),
                KeyModifiers::ALT,
            )),
            2,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cursor_position_report_amid_noise() {
        assert_eq!(parse_dsr(b"key\x1b[12;34Rtail"), Some((12, 34)));
    }

    #[test]
    fn parses_fragmented_cursor_position_report() {
        let mut parser = DsrParser::default();
        assert_eq!(parser.push(b"\x1b[1;"), None);
        assert_eq!(parser.push(b"3R"), Some(((1, 3), 0..6)));
    }

    #[test]
    fn rejects_malformed_or_overflowing_cursor_reports() {
        assert_eq!(parse_dsr(b"\x1b[;2R"), None);
        assert_eq!(parse_dsr(b"\x1b[1;xR"), None);
        assert_eq!(parse_dsr(b"\x1b[999999;2R"), None);
        assert_eq!(parse_dsr(b"\x1b[999999;2Rnoise\x1b[1;2R"), Some((1, 2)));
    }

    #[test]
    fn preserves_keys_surrounding_the_cursor_report() {
        let mut chunks = vec![b"q\x1b[1;".to_vec(), b"2R\x1b[B".to_vec()].into_iter();
        let read = read_dsr_with_timeout(Duration::from_millis(20), |_| Ok(chunks.next()));
        assert_eq!(read.position, Some((1, 2)));
        let keys = decode_pending_keys(&read.pending);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].code, KeyCode::Char('q'));
        assert_eq!(keys[1].code, KeyCode::Down);
    }

    #[test]
    fn preserves_control_and_alt_but_ignores_bracketed_paste_like_live_input() {
        let keys = decode_pending_keys(b"\x03\x1bx\x1b[200~hq\xc3\xa9\x1b[201~");
        assert_eq!(
            keys,
            [
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
            ]
        );
    }

    #[test]
    fn completes_utf8_and_arrow_sequences_split_around_the_cursor_report() {
        let mut utf8_chunks = vec![b"\xc3\x1b[1;2R".to_vec(), b"\xa9".to_vec()].into_iter();
        let utf8 = read_dsr_with_timeout(Duration::from_millis(20), |_| Ok(utf8_chunks.next()));
        assert_eq!(
            decode_pending_keys(&utf8.pending)[0].code,
            KeyCode::Char('é')
        );

        let mut arrow_chunks = vec![b"\x1b[\x1b[1;2R".to_vec(), b"A".to_vec()].into_iter();
        let arrow = read_dsr_with_timeout(Duration::from_millis(20), |_| Ok(arrow_chunks.next()));
        assert_eq!(decode_pending_keys(&arrow.pending)[0].code, KeyCode::Up);
    }

    #[test]
    fn strips_only_a_trailing_partial_cursor_report() {
        let mut bytes = b"q\x1b[12;".to_vec();
        strip_partial_dsr(&mut bytes);
        assert_eq!(bytes, b"q");

        let mut arrow = b"\x1b[1;5A".to_vec();
        strip_partial_dsr(&mut arrow);
        assert_eq!(arrow, b"\x1b[1;5A");
    }

    #[test]
    fn preserves_shift_tab_function_and_modified_arrow_keys() {
        let keys = decode_pending_keys(b"\x1b[Z\x1bOP\x1b[1;5A");
        assert_eq!(keys[0].code, KeyCode::BackTab);
        assert_eq!(keys[1].code, KeyCode::F(1));
        assert_eq!(keys[2].code, KeyCode::Up);
        assert_eq!(keys[2].modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn recognizes_narrow_and_wide_probe_positions() {
        assert_eq!(classify_position(Some((1, 2))), AmbientWidth::Narrow);
        assert_eq!(classify_position(Some((1, 3))), AmbientWidth::Wide);
        assert_eq!(classify_position(Some((2, 3))), AmbientWidth::Unknown);
    }

    #[test]
    fn timeout_is_forwarded_as_a_strict_bound() {
        let timeout = Duration::from_millis(225);
        let mut observed = None;
        let result = read_dsr_with_timeout(timeout, |remaining| {
            observed = Some(remaining);
            Ok(None)
        });

        assert_eq!(result, DsrRead::default());
        assert!(observed.is_some_and(|remaining| remaining <= timeout));
    }

    #[test]
    fn incomplete_replies_stop_at_the_absolute_timeout() {
        let started = Instant::now();
        let mut polls = 0;
        let result = read_dsr_with_timeout(Duration::from_millis(2), |_| {
            polls += 1;
            Ok(Some(Vec::new()))
        });

        assert_eq!(result, DsrRead::default());
        assert!(polls > 0);
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn zero_timeout_does_not_attempt_a_read() {
        let mut called = false;
        let result = read_dsr_with_timeout(Duration::ZERO, |_| {
            called = true;
            Ok(None)
        });

        assert_eq!(result, DsrRead::default());
        assert!(!called);
    }
}
