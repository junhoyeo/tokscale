//! Bounded streaming input for DSH's plain and concatenated-zstd JSONL stores.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
const BUFFER_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub file_bytes: u64,
    pub decoded_bytes: u64,
    pub record_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 1024 * 1024 * 1024,
            decoded_bytes: 1024 * 1024 * 1024,
            record_bytes: 64 * 1024 * 1024,
        }
    }
}

pub(super) struct TranscriptLines<R> {
    reader: R,
    limits: Limits,
    decoded: u64,
    line: Vec<u8>,
    first: bool,
    finished: bool,
}

pub(super) fn open(
    path: &Path,
    limits: Limits,
) -> Result<TranscriptLines<Box<dyn BufRead>>, String> {
    let file = File::open(path).map_err(|e| format!("could not open transcript: {e}"))?;
    let length = file
        .metadata()
        .map_err(|e| format!("could not stat transcript: {e}"))?
        .len();
    if length > limits.file_bytes {
        return Err(format!(
            "transcript exceeds the {} byte file limit",
            limits.file_bytes
        ));
    }
    // Snapshot the physical length. A producer appending frames cannot extend
    // this scan forever; the cache loader independently verifies the fingerprint.
    let mut input = BufReader::with_capacity(BUFFER_BYTES, file.take(length));
    let compressed = input
        .fill_buf()
        .map_err(|e| e.to_string())?
        .starts_with(&ZSTD_MAGIC);
    let reader: Box<dyn BufRead> = if compressed {
        let mut decoder = zstd::stream::read::Decoder::with_buffer(input)
            .map_err(|e| format!("could not initialize zstd decoder: {e}"))?;
        // Bound the decoder's own allocation as well as our record buffer.
        decoder.window_log_max(26).map_err(|e| e.to_string())?;
        Box::new(BufReader::with_capacity(BUFFER_BYTES, decoder))
    } else {
        Box::new(input)
    };
    Ok(TranscriptLines::new(reader, limits))
}

impl<R: BufRead> TranscriptLines<R> {
    pub(super) fn new(reader: R, limits: Limits) -> Self {
        Self {
            reader,
            limits,
            decoded: 0,
            line: Vec::new(),
            first: true,
            finished: false,
        }
    }

    fn read_record(&mut self) -> Result<Option<String>, String> {
        self.line.clear();
        loop {
            let available = self
                .reader
                .fill_buf()
                .map_err(|e| format!("incomplete transcript: {e}"))?;
            if available.is_empty() {
                self.finished = true;
                break;
            }
            let count = available
                .iter()
                .position(|b| *b == b'\n')
                .map_or(available.len(), |i| i + 1);
            let newline = available[count - 1] == b'\n';
            if count as u64 > self.limits.decoded_bytes.saturating_sub(self.decoded) {
                return Err(format!(
                    "transcript exceeds the {} byte decoded limit",
                    self.limits.decoded_bytes
                ));
            }
            if count > self.limits.record_bytes.saturating_sub(self.line.len()) {
                // Refuse the rest of the source: losing a session header, fork
                // boundary, retry or routing event makes later rows ambiguous.
                return Err(format!(
                    "transcript record exceeds the {} byte record limit",
                    self.limits.record_bytes
                ));
            }
            self.line.extend_from_slice(&available[..count]);
            self.reader.consume(count);
            self.decoded += count as u64;
            if newline {
                break;
            }
        }
        if self.line.is_empty() {
            return Ok(None);
        }
        if self.line.last() == Some(&b'\n') {
            self.line.pop();
            if self.line.last() == Some(&b'\r') {
                self.line.pop();
            }
        }
        let start = if std::mem::take(&mut self.first) && self.line.starts_with(b"\xef\xbb\xbf") {
            3
        } else {
            0
        };
        Ok(Some(
            String::from_utf8_lossy(&self.line[start..]).into_owned(),
        ))
    }
}

impl<R: BufRead> Iterator for TranscriptLines<R> {
    type Item = Result<String, String>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        match self.read_record() {
            Ok(Some(line)) => Some(Ok(line)),
            Ok(None) => None,
            Err(error) => {
                self.finished = true;
                Some(Err(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(decoded_bytes: u64, record_bytes: usize) -> Limits {
        Limits {
            file_bytes: 4096,
            decoded_bytes,
            record_bytes,
        }
    }

    #[test]
    fn exact_limits_accept_crlf_bom_utf8_and_final_record() {
        let bytes = "\u{feff}α\r\nlast".as_bytes();
        let input = BufReader::with_capacity(1, bytes);
        let lines = TranscriptLines::new(input, limits(bytes.len() as u64, 7))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(lines, ["α", "last"]);
    }

    #[test]
    fn decoded_limit_bounds_many_small_records() {
        let mut lines =
            TranscriptLines::new(BufReader::with_capacity(2, &b"a\nb\nc\n"[..]), limits(4, 2));
        assert_eq!(lines.next().unwrap().unwrap(), "a");
        assert_eq!(lines.next().unwrap().unwrap(), "b");
        assert!(lines.next().unwrap().unwrap_err().contains("decoded limit"));
        assert!(lines.next().is_none());
        assert_eq!(lines.decoded, 4);
    }

    #[test]
    fn record_limit_stops_before_allocating_the_record_or_reading_the_suffix() {
        let bytes = vec![b'x'; 1024 * 1024];
        let mut lines = TranscriptLines::new(
            BufReader::with_capacity(8, bytes.as_slice()),
            limits(2_000_000, 24),
        );
        assert!(lines.next().unwrap().unwrap_err().contains("record limit"));
        assert!(lines.line.len() <= 24);
        assert_eq!(lines.decoded, 24);
        assert!(lines.next().is_none());
    }

    #[test]
    fn compressed_expansion_is_bounded_while_the_decoder_produces_records() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let frame = zstd::encode_all(vec![b'x'; 1024 * 1024].as_slice(), 1).unwrap();
        std::fs::write(file.path(), frame).unwrap();
        let mut lines = open(file.path(), limits(512 * 1024, 4096)).unwrap();
        assert!(lines.next().unwrap().unwrap_err().contains("record limit"));
        assert!(lines.line.len() <= 4096);
        assert!(lines.next().is_none());
    }
}
