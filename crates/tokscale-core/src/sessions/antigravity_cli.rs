use super::UnifiedMessage;
use crate::TokenBreakdown;
use std::path::Path;

#[allow(dead_code)]
struct AntigravityMetadata {
    model_id: String,
    display_name: Option<String>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    cached_tokens: Option<i64>,
    thoughts_tokens: Option<i64>,
    tool_tokens: Option<i64>,
}

fn read_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut val = 0;
    let mut shift = 0;
    while *pos < data.len() {
        let b = data[*pos];
        *pos += 1;
        val |= ((b & 0x7F) as u64) << shift;
        if (b & 0x80) == 0 {
            return Some(val);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

fn parse_antigravity_protobuf(data: &[u8]) -> Option<AntigravityMetadata> {
    let mut pos = 0;
    let mut model_id = String::new();
    let mut display_name = None;
    let mut prompt_tokens = None;
    let mut completion_tokens = None;
    let mut cached_tokens = None;
    let mut thoughts_tokens = None;
    let mut tool_tokens = None;

    while pos < data.len() {
        let Some(key) = read_varint(data, &mut pos) else {
            break;
        };
        let wire_type = key & 0x7;
        let field_num = key >> 3;

        match wire_type {
            0 => {
                let _val = read_varint(data, &mut pos);
            }
            1 => {
                if pos + 8 > data.len() {
                    break;
                }
                pos += 8;
            }
            2 => {
                let Some(len_u64) = read_varint(data, &mut pos) else {
                    break;
                };
                let len = len_u64 as usize;
                if pos + len > data.len() {
                    break;
                }
                let field_bytes = &data[pos..pos + len];
                pos += len;

                if field_num == 1 {
                    let mut sub_pos = 0;
                    while sub_pos < field_bytes.len() {
                        let Some(sub_key) = read_varint(field_bytes, &mut sub_pos) else {
                            break;
                        };
                        let sub_wire = sub_key & 0x7;
                        let sub_field = sub_key >> 3;
                        match sub_wire {
                            2 => {
                                let Some(sub_len) = read_varint(field_bytes, &mut sub_pos) else {
                                    break;
                                };
                                let sub_len = sub_len as usize;
                                if sub_pos + sub_len > field_bytes.len() {
                                    break;
                                }
                                let sub_bytes = &field_bytes[sub_pos..sub_pos + sub_len];
                                sub_pos += sub_len;

                                if sub_field == 19 {
                                    if let Ok(s) = std::str::from_utf8(sub_bytes) {
                                        model_id = s.to_string();
                                    }
                                } else if sub_field == 21 {
                                    if let Ok(s) = std::str::from_utf8(sub_bytes) {
                                        display_name = Some(s.to_string());
                                    }
                                } else if sub_field == 17 {
                                    let mut usage_pos = 0;
                                    while usage_pos < sub_bytes.len() {
                                        let Some(u_key) = read_varint(sub_bytes, &mut usage_pos)
                                        else {
                                            break;
                                        };
                                        let u_wire = u_key & 0x7;
                                        let u_field = u_key >> 3;
                                        if u_wire == 2 && u_field == 2 {
                                            let Some(u_len) =
                                                read_varint(sub_bytes, &mut usage_pos)
                                            else {
                                                break;
                                            };
                                            let u_len = u_len as usize;
                                            if usage_pos + u_len > sub_bytes.len() {
                                                break;
                                            }
                                            let token_bytes =
                                                &sub_bytes[usage_pos..usage_pos + u_len];
                                            usage_pos += u_len;

                                            let mut token_pos = 0;
                                            while token_pos < token_bytes.len() {
                                                let Some(t_key) =
                                                    read_varint(token_bytes, &mut token_pos)
                                                else {
                                                    break;
                                                };
                                                let t_wire = t_key & 0x7;
                                                let t_field = t_key >> 3;
                                                if t_wire == 0 {
                                                    if let Some(val) =
                                                        read_varint(token_bytes, &mut token_pos)
                                                    {
                                                        let val_i64 =
                                                            i64::try_from(val).unwrap_or(i64::MAX);
                                                        match t_field {
                                                            // Field 1 matches static cache metadata, whereas Field 5 matches cumulative
                                                            // context cache reads. Field 5 (if present) takes precedence via sequential override.
                                                            1 => cached_tokens = Some(val_i64),
                                                            2 => prompt_tokens = Some(val_i64),
                                                            3 => completion_tokens = Some(val_i64),
                                                            5 => cached_tokens = Some(val_i64),
                                                            6 => tool_tokens = Some(val_i64),
                                                            9 => thoughts_tokens = Some(val_i64),
                                                            _ => {}
                                                        }
                                                    }
                                                } else if t_wire == 1 {
                                                    if token_pos + 8 > token_bytes.len() {
                                                        break;
                                                    }
                                                    token_pos += 8;
                                                } else if t_wire == 2 {
                                                    let Some(t_len) =
                                                        read_varint(token_bytes, &mut token_pos)
                                                    else {
                                                        break;
                                                    };
                                                    if token_pos + t_len as usize
                                                        > token_bytes.len()
                                                    {
                                                        break;
                                                    }
                                                    token_pos += t_len as usize;
                                                } else if t_wire == 5 {
                                                    if token_pos + 4 > token_bytes.len() {
                                                        break;
                                                    }
                                                    token_pos += 4;
                                                } else {
                                                    break;
                                                }
                                            }
                                        } else if u_wire == 0 {
                                            let _ = read_varint(sub_bytes, &mut usage_pos);
                                        } else if u_wire == 1 {
                                            if usage_pos + 8 > sub_bytes.len() {
                                                break;
                                            }
                                            usage_pos += 8;
                                        } else if u_wire == 2 {
                                            let Some(u_len) =
                                                read_varint(sub_bytes, &mut usage_pos)
                                            else {
                                                break;
                                            };
                                            if usage_pos + u_len as usize > sub_bytes.len() {
                                                break;
                                            }
                                            usage_pos += u_len as usize;
                                        } else if u_wire == 5 {
                                            if usage_pos + 4 > sub_bytes.len() {
                                                break;
                                            }
                                            usage_pos += 4;
                                        } else {
                                            break;
                                        }
                                    }
                                }
                            }
                            0 => {
                                let _val = read_varint(field_bytes, &mut sub_pos);
                            }
                            1 => {
                                if sub_pos + 8 > field_bytes.len() {
                                    break;
                                }
                                sub_pos += 8;
                            }
                            5 => {
                                if sub_pos + 4 > field_bytes.len() {
                                    break;
                                }
                                sub_pos += 4;
                            }
                            _ => break,
                        }
                    }
                }
            }
            5 => {
                if pos + 4 > data.len() {
                    break;
                }
                pos += 4;
            }
            _ => break,
        }
    }

    if model_id.is_empty() {
        return None;
    }

    Some(AntigravityMetadata {
        model_id,
        display_name,
        prompt_tokens,
        completion_tokens,
        cached_tokens,
        thoughts_tokens,
        tool_tokens,
    })
}

fn resolve_antigravity_model(model_id: &str, display_name: Option<&str>) -> String {
    if let Some(dn) = display_name {
        let dn_lower = dn.to_lowercase();
        if dn_lower.contains("gemini 3.5 flash") {
            if dn_lower.contains("high") {
                return "gemini-3.5-flash-high".to_string();
            } else if dn_lower.contains("medium") {
                return "gemini-3.5-flash-medium".to_string();
            } else if dn_lower.contains("low") {
                return "gemini-3.5-flash-low".to_string();
            }
        }
    }
    crate::pricing::aliases::resolve_alias(model_id)
        .unwrap_or(model_id)
        .to_string()
}

pub fn parse_antigravity_cli_file(db_path: &Path) -> Vec<UnifiedMessage> {
    use rusqlite::{Connection, OpenFlags};
    let fallback_timestamp = super::utils::file_modified_timestamp_ms(db_path);

    let conn = match Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => conn,
        Err(_) => return Vec::new(),
    };

    let mut stmt = match conn.prepare("SELECT idx, data FROM gen_metadata ORDER BY idx ASC") {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };

    let mut rows = match stmt.query([]) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };

    let session_id = db_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mut messages = Vec::new();

    while let Ok(Some(row)) = rows.next() {
        let idx: i32 = match row.get(0) {
            Ok(val) => val,
            Err(_) => continue,
        };
        let data: Vec<u8> = match row.get(1) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };

        if let Some(meta) = parse_antigravity_protobuf(&data) {
            let completion = meta.completion_tokens.unwrap_or(0);
            let thoughts = meta.thoughts_tokens.unwrap_or(0);
            let breakdown = TokenBreakdown {
                input: meta.prompt_tokens.unwrap_or(0),
                output: completion.saturating_sub(thoughts),
                cache_read: meta.cached_tokens.unwrap_or(0),
                reasoning: thoughts,
                ..Default::default()
            };

            let resolved_model =
                resolve_antigravity_model(&meta.model_id, meta.display_name.as_deref());

            let mut message = UnifiedMessage::new(
                "antigravity-cli",
                resolved_model,
                "google",
                session_id.clone(),
                fallback_timestamp + (idx as i64 * 1000),
                breakdown,
                0.0,
            );
            message.message_count = 1;
            message.dedup_key = Some(format!("antigravity-cli:{}:{}", session_id, idx));
            messages.push(message);
        }
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_antigravity_cli_sqlite() {
        use rusqlite::Connection;
        let file = tempfile::Builder::new().suffix(".db").tempfile().unwrap();

        {
            let conn = Connection::open(file.path()).unwrap();
            conn.execute(
                "CREATE TABLE gen_metadata (idx INTEGER PRIMARY KEY, data BLOB, size INTEGER DEFAULT 0)",
                [],
            ).unwrap();

            // Protobuf blob manual encoding:
            // - tag 1 (Length-delimited, root envelope) -> 1 << 3 | 2 = 10 (0x0A), len = inner.len()
            // Inner envelope:
            //   - tag 19 (Length-delimited, model_id) -> 19 << 3 | 2 = 154 (0x9A, 0x01), len = 16, val = gemini-3-flash-a
            //   - tag 17 (Length-delimited, usage) -> 17 << 3 | 2 = 138 (0x8A, 0x01), len = sub_17.len()
            //     Inner usage (sub_17):
            //       - tag 2 (Length-delimited, token_bytes) -> 2 << 3 | 2 = 18 (0x12), len = token_bytes.len()
            //         Inner token_bytes:
            //           - tag 1 (Varint, cached) = 15 -> 1 << 3 | 0 = 8 (0x08), val = 15 (0x0F)
            //           - tag 2 (Varint, prompt) = 100 -> 2 << 3 | 0 = 16 (0x10), val = 100 (0x64)
            //           - tag 3 (Varint, completion) = 50 -> 3 << 3 | 0 = 24 (0x18), val = 50 (0x32)
            //           - tag 5 (Varint, cached) = 20 -> 5 << 3 | 0 = 40 (0x28), val = 20 (0x14)
            //           - tag 6 (Varint, tool) = 5 -> 6 << 3 | 0 = 48 (0x30), val = 5 (0x05)
            //           - tag 9 (Varint, thoughts) = 10 -> 9 << 3 | 0 = 72 (0x48), val = 10 (0x0A)

            let token_bytes = vec![0x08, 15, 0x10, 100, 0x18, 50, 0x28, 20, 0x30, 5, 0x48, 10];

            let mut sub_17 = vec![0x12, token_bytes.len() as u8];
            sub_17.extend(token_bytes);

            let mut inner = vec![0x9A, 0x01, 16];
            inner.extend_from_slice(b"gemini-3-flash-a");

            inner.extend_from_slice(&[0x8A, 0x01, sub_17.len() as u8]);
            inner.extend(sub_17);

            // Add tag 21 (display_name) -> 21 << 3 | 2 = 170 (0xAA, 0x01), len = 23, val = "Gemini 3.5 Flash (High)"
            let display_name_bytes = b"Gemini 3.5 Flash (High)";
            inner.extend_from_slice(&[0xAA, 0x01, display_name_bytes.len() as u8]);
            inner.extend_from_slice(display_name_bytes);

            let mut data = vec![0x0A, inner.len() as u8];
            data.extend(inner);

            conn.execute(
                "INSERT INTO gen_metadata (idx, data, size) VALUES (?1, ?2, ?3)",
                rusqlite::params![0, data.clone(), data.len() as i32],
            )
            .unwrap();

            conn.execute(
                "INSERT INTO gen_metadata (idx, data, size) VALUES (?1, ?2, ?3)",
                rusqlite::params![1, data, data.len() as i32],
            )
            .unwrap();
        }

        let messages = parse_antigravity_cli_file(file.path());
        assert_eq!(messages.len(), 2);

        let msg0 = &messages[0];
        assert_eq!(msg0.client, "antigravity-cli");
        assert_eq!(msg0.model_id, "gemini-3.5-flash-high");
        assert_eq!(msg0.tokens.input, 100);
        assert_eq!(msg0.tokens.output, 40);
        assert_eq!(msg0.tokens.cache_read, 20);
        assert_eq!(msg0.tokens.reasoning, 10);

        let msg1 = &messages[1];
        // Timestamps must be monotonic and distinct because of the idx-based offset
        assert!(
            msg0.timestamp < msg1.timestamp,
            "Timestamps must be monotonic and distinct: {} vs {}",
            msg0.timestamp,
            msg1.timestamp
        );
    }
}
