//! Read-only Chromium IndexedDB adapter. Never opens LevelDB through its writer,
//! takes its LOCK, reads auth stores, or saves decoded conversation text.
//!
//! Formats: Chromium indexed_db_leveldb_coding.cc and idb_value_wrapping.cc.
use super::v8::{deserialize_blink, V8Value};
use leveldb_core::Record;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAX_VALUE_BYTES: u64 = 128 * 1024 * 1024;

pub(super) struct StoreValue {
    pub store: String,
    pub value: Value,
}

pub(super) fn varint(data: &[u8], offset: &mut usize) -> Option<u64> {
    let mut value = 0;
    for shift in (0..70).step_by(7) {
        let byte = *data.get(*offset)?;
        *offset += 1;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return Some(value);
        }
    }
    None
}

fn prefix(key: &[u8]) -> Option<(u64, u64, u64, usize)> {
    let first = *key.first()?;
    let lengths = [(first >> 5) + 1, ((first >> 2) & 7) + 1, (first & 3) + 1];
    let mut offset = 1;
    let mut ids = [0; 3];
    for (id, len) in ids.iter_mut().zip(lengths) {
        for shift in 0..usize::from(len) {
            *id |= u64::from(*key.get(offset)?) << (shift * 8);
            offset += 1;
        }
    }
    Some((ids[0], ids[1], ids[2], offset))
}

fn utf16(data: &[u8]) -> Option<String> {
    if !data.len().is_multiple_of(2) {
        return None;
    }
    String::from_utf16(
        &data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .collect::<Vec<_>>(),
    )
    .ok()
}

fn sized_string(data: &[u8], offset: &mut usize) -> Option<String> {
    let len = usize::try_from(varint(data, offset)?)
        .ok()?
        .checked_mul(2)?;
    let end = offset.checked_add(len)?;
    let value = utf16(data.get(*offset..end)?)?;
    *offset = end;
    Some(value)
}

fn latest_records(records: Vec<Record>) -> BTreeMap<Vec<u8>, Record> {
    let mut latest: BTreeMap<Vec<u8>, Record> = BTreeMap::new();
    for record in records {
        if latest
            .get(&record.key)
            .is_none_or(|old| record.seq > old.seq || (record.seq == old.seq && record.deleted))
        {
            latest.insert(record.key.clone(), record);
        }
    }
    // Resolve deletions BEFORE decoding values or external blob references.
    latest.retain(|_, record| !record.deleted);
    latest
}

pub(super) fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| "cache file unavailable".to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_VALUE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cache read failed".to_string())?;
    if bytes.len() as u64 > MAX_VALUE_BYTES {
        return Err("cache record exceeds read limit".into());
    }
    Ok(bytes)
}

fn files(dir: &Path, manifest: &str) -> Result<Vec<(PathBuf, u64, std::time::SystemTime)>, String> {
    let mut files = Vec::new();
    let mut paths = super::manifest::active_paths(dir, manifest)?;
    paths.push(dir.join(manifest));
    for path in paths {
        let meta = path.metadata().map_err(|_| "cache directory changed")?;
        if meta.is_file() {
            files.push((
                path,
                meta.len(),
                meta.modified().map_err(|_| "cache timestamp unavailable")?,
            ));
        }
    }
    files.sort();
    Ok(files)
}

pub(super) fn read(dir: &Path) -> Result<(Vec<StoreValue>, Vec<String>), String> {
    read_with(dir, read_bounded)
}

fn read_with(
    dir: &Path,
    mut read_file: impl FnMut(&Path) -> Result<Vec<u8>, String>,
) -> Result<(Vec<StoreValue>, Vec<String>), String> {
    // Compaction can remove a file or replace CURRENT at any point, including
    // during discovery, parsing and final validation. Retry the whole attempt;
    // never return records from an incomplete or mixed snapshot.
    read_snapshot(dir, &mut read_file).or_else(|_| read_snapshot(dir, &mut read_file))
}

fn read_snapshot(
    dir: &Path,
    read_file: &mut impl FnMut(&Path) -> Result<Vec<u8>, String>,
) -> Result<(Vec<StoreValue>, Vec<String>), String> {
    let manifest = super::manifest::current(dir)?;
    let before = files(dir, &manifest)?;
    let mut records = Vec::new();
    for (path, _, _) in &before {
        if path.file_name().is_some_and(|n| n == manifest.as_str()) {
            continue;
        }
        let bytes = read_file(path)?;
        let parsed = if path.extension().is_some_and(|s| s == "log") {
            super::manifest::log_records(&bytes).ok_or("incomplete or corrupt cache log")?;
            leveldb_core::parse_log_bytes(&bytes, path)
        } else {
            leveldb_core::parse_table_bytes(&bytes, path)
        };
        records.extend(parsed.map_err(|_| "invalid LevelDB cache file")?);
    }
    let records = latest_records(records);
    let result = decode(&records, &dir.with_extension("blob"));
    if manifest != super::manifest::current(dir)? || before != files(dir, &manifest)? {
        return Err("Claude cache changed during reading; retry when idle".into());
    }
    Ok(result)
}

fn decode(records: &BTreeMap<Vec<u8>, Record>, blobs: &Path) -> (Vec<StoreValue>, Vec<String>) {
    let mut databases = Vec::new();
    let mut stores = BTreeMap::new();
    for r in records.values() {
        let Some((db, store, index, offset)) = prefix(&r.key) else {
            continue;
        };
        let tail = &r.key[offset..];
        if (db, store, index) == (0, 0, 0) && tail.first() == Some(&201) {
            let mut pos = 1;
            if sized_string(tail, &mut pos).is_some()
                && sized_string(tail, &mut pos).as_deref() == Some("claude-conversation-store")
                && r.value.len() <= 8
            {
                let id = r
                    .value
                    .iter()
                    .enumerate()
                    .fold(0, |v, (i, b)| v | (u64::from(*b) << (8 * i)));
                databases.push(id);
            }
        }
        if store == 0 && index == 0 && tail.first() == Some(&50) {
            let mut pos = 1;
            if let Some(id) = varint(tail, &mut pos) {
                if tail.get(pos) == Some(&0) {
                    if let Some(name) =
                        utf16(&r.value).filter(|s| matches!(s.as_str(), "trees" | "meta"))
                    {
                        stores.insert((db, id), name);
                    }
                }
            }
        }
    }
    let mut values = Vec::new();
    let mut errors = Vec::new();
    for r in records.values() {
        let Some((db, store, index, offset)) = prefix(&r.key) else {
            continue;
        };
        let Some(name) = stores
            .get(&(db, store))
            .filter(|_| databases.contains(&db) && index == 1)
        else {
            continue;
        };
        let mut pos = 0;
        let decoded = varint(&r.value, &mut pos)
            .and_then(|_| {
                unwrap_value(
                    &r.value[pos..],
                    db,
                    store,
                    &r.key[offset..],
                    records,
                    blobs,
                    0,
                )
            })
            .ok_or_else(|| "unavailable external blob or invalid cache wrapper".to_string())
            .and_then(|data| deserialize_blink(&data).map_err(|error| error.to_string()));
        match decoded {
            Ok(value) => values.push(StoreValue {
                store: name.clone(),
                value: json(value),
            }),
            Err(error) => errors.push(error),
        }
    }
    (values, errors)
}

fn unwrap_value(
    data: &[u8],
    db: u64,
    store: u64,
    key: &[u8],
    records: &BTreeMap<Vec<u8>, Record>,
    blobs: &Path,
    depth: usize,
) -> Option<Vec<u8>> {
    if depth > 3 || data.len() as u64 > MAX_VALUE_BYTES {
        return None;
    }
    if !data.starts_with(&[0xff, 0x11]) {
        return Some(data.to_vec());
    }
    match data.get(2)? {
        2 => {
            let body = data.get(3..)?;
            if snap::raw::decompress_len(body).ok()? as u64 > MAX_VALUE_BYTES {
                return None;
            }
            let bytes = snap::raw::Decoder::new().decompress_vec(body).ok()?;
            unwrap_value(&bytes, db, store, key, records, blobs, depth + 1)
        }
        1 => {
            let mut pos = 3;
            let size = varint(data, &mut pos)?;
            let blob_index = varint(data, &mut pos)?;
            if size > MAX_VALUE_BYTES {
                return None;
            }
            let metadata = records.values().find(|r| {
                prefix(&r.key)
                    .is_some_and(|(d, s, i, o)| (d, s, i) == (db, store, 3) && &r.key[o..] == key)
            })?;
            let mut pos = 0;
            for i in 0..=blob_index {
                let kind = *metadata.value.get(pos)?;
                pos += 1;
                if kind > 1 {
                    return None;
                }
                let number = varint(&metadata.value, &mut pos)?;
                sized_string(&metadata.value, &mut pos)?;
                let _size = varint(&metadata.value, &mut pos)?;
                if kind == 1 {
                    sized_string(&metadata.value, &mut pos)?;
                    varint(&metadata.value, &mut pos)?;
                }
                if i == blob_index {
                    let path = blobs.join(format!("{db:x}/{:02x}/{number:x}", number >> 8));
                    let bytes = read_bounded(&path).ok()?;
                    if bytes.len() as u64 != size {
                        return None;
                    }
                    return unwrap_value(&bytes, db, store, key, records, blobs, depth + 1);
                }
            }
            None
        }
        _ => None,
    }
}

fn json(value: V8Value) -> Value {
    match value {
        V8Value::Bool(v) => v.into(),
        V8Value::Int(v) => v.into(),
        V8Value::Double(v)
            if v.is_finite() && v.fract() == 0.0 && v >= i64::MIN as f64 && v < i64::MAX as f64 =>
        {
            (v as i64).into()
        }
        V8Value::Double(v) | V8Value::Date(v) => serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        V8Value::String(v) => v.into(),
        V8Value::Array(v) => Value::Array(v.into_iter().map(json).collect()),
        V8Value::Object(v) => Value::Object(v.into_iter().map(|(k, v)| (k, json(v))).collect()),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_during_read_retries_the_entire_snapshot() {
        use super::super::manifest::encode_log;
        for failure in ["missing-file", "partial-log", "changed-manifest"] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            std::fs::write(root.join("CURRENT"), "MANIFEST-000001\n").unwrap();
            std::fs::write(root.join("MANIFEST-000001"), encode_log(&[2, 2])).unwrap();
            let empty_batch = encode_log(&[0; 12]);
            std::fs::write(root.join("000002.log"), &empty_batch).unwrap();
            let mut attempts = 0;
            let (values, errors) = read_with(root, |path| {
                attempts += 1;
                if attempts == 1 {
                    std::fs::write(root.join("MANIFEST-000003"), encode_log(&[2, 4])).unwrap();
                    std::fs::write(root.join("000004.log"), &empty_batch).unwrap();
                    std::fs::write(root.join("CURRENT"), "MANIFEST-000003\n").unwrap();
                    match failure {
                        "missing-file" => std::fs::remove_file(path).unwrap(),
                        "partial-log" => return Ok(vec![1]),
                        _ => {}
                    }
                } else {
                    assert!(path.ends_with("000004.log"));
                }
                read_bounded(path)
            })
            .unwrap();
            assert_eq!(attempts, 2, "{failure}");
            assert!(values.is_empty());
            assert!(errors.is_empty());
        }
    }

    #[test]
    fn persistent_read_failure_stops_after_one_retry() {
        use super::super::manifest::encode_log;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CURRENT"), "MANIFEST-000001\n").unwrap();
        std::fs::write(dir.path().join("MANIFEST-000001"), encode_log(&[2, 2])).unwrap();
        std::fs::write(dir.path().join("000002.log"), []).unwrap();
        let mut attempts = 0;
        let result = read_with(dir.path(), |_| {
            attempts += 1;
            Err("cache file unavailable".into())
        });
        assert_eq!(attempts, 2);
        assert_eq!(result.err().as_deref(), Some("cache file unavailable"));
    }

    #[test]
    fn deletion_and_latest_version_win_before_decode() {
        let record = |key, seq, deleted| Record {
            key: vec![key],
            value: vec![],
            seq,
            deleted,
            origin_file: PathBuf::new(),
        };
        let result = latest_records(vec![
            record(1, 2, true),
            record(1, 1, false),
            record(2, 5, false),
            record(2, 3, false),
        ]);
        assert!(!result.contains_key(&vec![1]));
        assert_eq!(result[&vec![2]].seq, 5);
    }

    #[test]
    fn compressed_value_and_malformed_lengths() {
        let data = [0xff, 15, b'o', b'{', 0];
        let mut compressed = vec![0xff, 17, 2];
        compressed.extend(snap::raw::Encoder::new().compress_vec(&data).unwrap());
        assert_eq!(
            unwrap_value(&compressed, 1, 1, &[], &BTreeMap::new(), Path::new(""), 0),
            Some(data.to_vec())
        );
        assert!(varint(&[255; 12], &mut 0).is_none());
        for len in 0..4 {
            assert!(prefix(&[0; 4][..len]).is_none());
        }
    }
    fn put_varint(mut n: u64, bytes: &mut Vec<u8>) {
        while n >= 128 {
            bytes.push((n as u8 & 127) | 128);
            n >>= 7;
        }
        bytes.push(n as u8);
    }

    fn put_string(value: &str, bytes: &mut Vec<u8>) {
        let units: Vec<_> = value.encode_utf16().collect();
        put_varint(units.len() as u64, bytes);
        for unit in units {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
    }

    #[test]
    fn reads_live_external_blob_with_unicode_and_ignores_obsolete_tables() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("https_claude.ai_0.indexeddb.leveldb");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CURRENT"), "MANIFEST-000001\n").unwrap();
        std::fs::write(
            dir.join("MANIFEST-000001"),
            super::super::manifest::encode_log(&[2, 2]),
        )
        .unwrap();
        // A stale, invalid table must not be read because it is not in MANIFEST.
        std::fs::write(dir.join("000099.ldb"), "obsolete").unwrap();
        let mut db_key = vec![0, 0, 0, 0, 201];
        put_string("https_claude.ai", &mut db_key);
        put_string("claude-conversation-store", &mut db_key);
        let store_value: Vec<_> = "trees".encode_utf16().flat_map(u16::to_be_bytes).collect();
        let blob = vec![
            0xff, 21, 0xfe, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 15, b'o', b'"', 1, b'x', 0,
            b'c', 2, 0x16, 0x04, b'{', 1,
        ];
        let mut reference = vec![1, 0xff, 17, 1];
        put_varint(blob.len() as u64, &mut reference);
        reference.push(0);
        let mut blob_info = vec![0];
        put_varint(257, &mut blob_info);
        put_string("", &mut blob_info);
        put_varint(blob.len() as u64, &mut blob_info);
        let blob_dir = dir.with_extension("blob").join("1/01");
        std::fs::create_dir_all(&blob_dir).unwrap();
        std::fs::write(blob_dir.join("101"), blob).unwrap();
        let entries = vec![
            (db_key, vec![1]),
            (vec![0, 1, 0, 0, 50, 1, 0], store_value),
            (vec![0, 1, 1, 1, 1, 0], reference),
            (vec![0, 1, 1, 3, 1, 0], blob_info),
        ];
        let mut batch = 1u64.to_le_bytes().to_vec();
        batch.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (key, value) in entries {
            batch.push(1);
            put_varint(key.len() as u64, &mut batch);
            batch.extend(key);
            put_varint(value.len() as u64, &mut batch);
            batch.extend(value);
        }
        std::fs::write(
            dir.join("000002.log"),
            super::super::manifest::encode_log(&batch),
        )
        .unwrap();
        let (records, errors) = read(&dir).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value, serde_json::json!({"x":"Ж"}));
        std::fs::remove_file(blob_dir.join("101")).unwrap();
        let (records, errors) = read(&dir).unwrap();
        assert!(records.is_empty());
        assert_eq!(errors.len(), 1);
    }
}
