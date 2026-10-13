//! Select live SSTables from CURRENT/MANIFEST. Scanning every old .ldb can
//! resurrect deleted conversations after compaction discards a tombstone.
use super::indexeddb::{read_bounded, varint};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub(super) fn log_records(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut records = Vec::new();
    let mut fragment: Option<Vec<u8>> = None;
    for block in bytes.chunks(32768) {
        let mut offset = 0;
        while offset + 7 <= block.len() {
            let header = &block[offset..offset + 7];
            if header.iter().all(|b| *b == 0) {
                if block[offset..].iter().any(|b| *b != 0) {
                    return None;
                }
                break;
            }
            let size = usize::from(u16::from_le_bytes([header[4], header[5]]));
            let kind = header[6];
            let data = block.get(offset + 7..offset + 7 + size)?;
            let crc = crc32c::crc32c_append(crc32c::crc32c(&[kind]), data)
                .rotate_left(17)
                .wrapping_add(0xa282_ead8);
            if crc != u32::from_le_bytes(header[..4].try_into().ok()?) {
                return None;
            }
            match kind {
                1 if fragment.is_none() => records.push(data.to_vec()),
                2 if fragment.is_none() => fragment = Some(data.to_vec()),
                3 => fragment.as_mut()?.extend_from_slice(data),
                4 => {
                    let mut value = fragment.take()?;
                    value.extend_from_slice(data);
                    records.push(value);
                }
                _ => return None,
            }
            offset += 7 + size;
        }
        if offset < block.len()
            && block.len() - offset < 7
            && block[offset..].iter().any(|b| *b != 0)
        {
            return None;
        }
    }
    if fragment.is_some() {
        return None;
    }
    Some(records)
}

fn skip_string(data: &[u8], pos: &mut usize) -> Option<()> {
    let size = usize::try_from(varint(data, pos)?).ok()?;
    *pos = pos.checked_add(size)?;
    (*pos <= data.len()).then_some(())
}

fn live_files(edits: &[Vec<u8>]) -> Option<(BTreeSet<u64>, u64, u64)> {
    let mut tables = BTreeSet::new();
    let mut log = None;
    let mut prev_log = 0;
    for edit in edits {
        let mut pos = 0;
        while pos < edit.len() {
            match varint(edit, &mut pos)? {
                1 => skip_string(edit, &mut pos)?,
                2 => log = Some(varint(edit, &mut pos)?),
                9 => prev_log = varint(edit, &mut pos)?,
                3 | 4 => {
                    varint(edit, &mut pos)?;
                }
                5 => {
                    varint(edit, &mut pos)?;
                    skip_string(edit, &mut pos)?;
                }
                6 => {
                    varint(edit, &mut pos)?;
                    tables.remove(&varint(edit, &mut pos)?);
                }
                7 => {
                    varint(edit, &mut pos)?;
                    tables.insert(varint(edit, &mut pos)?);
                    varint(edit, &mut pos)?;
                    skip_string(edit, &mut pos)?;
                    skip_string(edit, &mut pos)?;
                }
                _ => return None,
            }
        }
    }
    // Without an explicit current log number, obsolete WALs cannot be
    // distinguished from live ones. Never fall back to scanning all logs.
    Some((tables, log?, prev_log))
}

pub(super) fn current(dir: &Path) -> Result<String, String> {
    let name =
        String::from_utf8(read_bounded(&dir.join("CURRENT"))?).map_err(|_| "invalid CURRENT")?;
    let name = name.trim();
    if !name
        .strip_prefix("MANIFEST-")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
    {
        return Err("invalid manifest name".into());
    }
    Ok(name.to_string())
}

pub(super) fn active_paths(dir: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let data = read_bounded(&dir.join(name))?;
    let edits = log_records(&data).ok_or("incomplete or corrupt cache manifest")?;
    let (tables, log, prev) = live_files(&edits).ok_or("unsupported cache manifest")?;
    let mut paths = Vec::new();
    for table in tables {
        let path = dir.join(format!("{table:06}.ldb"));
        paths.push(if path.exists() {
            path
        } else {
            dir.join(format!("{table:06}.sst"))
        });
    }
    for entry in std::fs::read_dir(dir).map_err(|_| "cache directory unavailable")? {
        let path = entry.map_err(|_| "cache directory changed")?.path();
        if path.extension().is_some_and(|e| e == "log")
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
                .is_some_and(|n| n >= log || n == prev)
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
pub(super) fn encode_log(payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() < 32761);
    let crc = crc32c::crc32c_append(crc32c::crc32c(&[1]), payload)
        .rotate_left(17)
        .wrapping_add(0xa282_ead8);
    let mut bytes = crc.to_le_bytes().to_vec();
    bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    bytes.push(1);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_log_number_never_selects_obsolete_wals() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("000001.log"), []).unwrap();
        for payload in [vec![], vec![3, 10, 4, 20], vec![9, 1]] {
            std::fs::write(dir.path().join("MANIFEST-000002"), encode_log(&payload)).unwrap();
            assert!(active_paths(dir.path(), "MANIFEST-000002").is_err());
        }
        assert_eq!(live_files(&[vec![2, 0]]), Some((BTreeSet::new(), 0, 0)));
    }

    #[test]
    fn current_and_previous_log_numbers_exclude_obsolete_wals() {
        let dir = tempfile::tempdir().unwrap();
        for number in 1..=4 {
            std::fs::write(dir.path().join(format!("{number:06}.log")), []).unwrap();
        }
        std::fs::write(
            dir.path().join("MANIFEST-000005"),
            encode_log(&[2, 3, 9, 2]),
        )
        .unwrap();
        assert_eq!(
            active_paths(dir.path(), "MANIFEST-000005").unwrap(),
            [2, 3, 4].map(|n| dir.path().join(format!("{n:06}.log")))
        );
    }
    #[test]
    fn respects_manifest_deletions_and_rejects_corruption() {
        let edits = vec![
            vec![2, 9, 7, 0, 1, 20, 0, 0, 7, 0, 2, 20, 0, 0],
            vec![6, 0, 1],
        ];
        assert_eq!(live_files(&edits), Some((BTreeSet::from([2]), 9, 0)));
        let mut log = encode_log(&edits[0]);
        assert_eq!(log_records(&log), Some(vec![edits[0].clone()]));
        log[7] ^= 1;
        assert!(log_records(&log).is_none());
    }
}
