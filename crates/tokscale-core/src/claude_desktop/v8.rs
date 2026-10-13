// Adapted from blob-decoder 0.2.2, Apache-2.0 (see LICENSE-v8).
// Source: https://crates.io/crates/blob-decoder/0.2.2 (src/v8_value.rs)
// Local changes: padding, raw framing, varint/container validation, and resource
// budgets for BigInt conversion, object registration and reference expansion.
//! V8 / Blink structured-clone **value deserializer**.
//!
//! When Chromium persists a JavaScript value — an IndexedDB record, a
//! `postMessage` payload, a Service Worker cache entry — it serializes it with
//! V8's `ValueSerializer` (the *structured clone* wire format), optionally
//! wrapped in Blink's `SerializedScriptValue` envelope. This module decodes those
//! bytes back into a structured [`V8Value`], so an opaque IndexedDB blob unwraps
//! the way `blob-decoder` already unwraps bplist / gzip / protobuf.
//!
//! # Wire format
//!
//! A V8 stream opens with a version header (`0xFF` + an LEB128 version) and is
//! then a sequence of one-byte **serialization tags**, each introducing a typed
//! value; lengths are unsigned LEB128 varints and `kInt32` is zig-zag encoded.
//! Blink prepends its own `0xFF <blink-version>` and an optional `0xFE` trailer
//! (an 8-byte BE offset + 4-byte BE size) before the nested V8 payload.
//!
//! Authoritative references (decode logic below is a clean-room Rust
//! reimplementation, **not** a port of the C++):
//! - V8 `src/objects/value-serializer.cc` — the `SerializationTag` enum, varint
//!   framing, the `0xFF` version header.
//! - Blink `.../serialization/serialization_tag.h` — the `0xFF`/`0xFE` envelope.
//!
//! The canonical tag→name tables live in the fleet knowledge crate
//! `forensicnomicon-core::v8_serialization` (module present in the source tree,
//! not yet on crates.io as of forensicnomicon-core 1.4.0). The individual tag
//! *byte* constants a decoder must match on are mirrored here from that source;
//! migrate to the published constants once that module ships to the registry.
//!
//! # Safety
//!
//! All input is attacker-controllable. The invariant is: **never panic, never
//! read out of bounds, never trust a length field, never OOM.** Every read is
//! bounds-checked (returns [`V8Error`], never indexes blindly), recursion is
//! depth-capped, and total materialized nodes are budget-capped so a crafted blob
//! (deep nesting, huge sparse-array length, reference amplification) fails loud
//! instead of exhausting memory or the stack.

/// A decoded V8 / Blink structured-clone value.
///
/// Numeric JS values split by their wire encoding: [`V8Value::Int`] for the
/// zig-zag `kInt32` / `kUint32` tags, [`V8Value::Double`] for `kDouble` (which V8
/// also uses for any integer outside `i32` range). `BigInt` is rendered to a
/// decimal string. Boxed primitives (`new Number(7)`, `new String('x')`,
/// `new Boolean(true)`) keep their wrapper identity so the reading is faithful.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum V8Value {
    /// `kUndefined` (`_`).
    Undefined,
    /// `kNull` (`0`).
    Null,
    /// `kTheHole` (`-`) — an absent element in a sparse/holey array.
    Hole,
    /// `kTrue` / `kFalse`.
    Bool(bool),
    /// `kInt32` (zig-zag) or `kUint32`.
    Int(i64),
    /// `kDouble` — an IEEE-754 double (also used for integers outside `i32`).
    Double(f64),
    /// `kBigInt`, rendered to its decimal string (e.g. `"-42"`).
    BigInt(String),
    /// `kUtf8String` / `kOneByteString` (Latin-1) / `kTwoByteString` (UTF-16LE).
    String(String),
    /// `kDate` — milliseconds since the Unix epoch.
    Date(f64),
    /// `kRegExp` — the source pattern plus V8's raw flag bitset.
    RegExp {
        /// The regexp source (without delimiters).
        source: String,
        /// V8's raw flag bits (`global=1, ignoreCase=2, multiline=4, …`).
        flags: u32,
    },
    /// `kBeginDenseJSArray` / `kBeginSparseJSArray` — holes are [`V8Value::Hole`].
    Array(Vec<V8Value>),
    /// `kBeginJSObject` — properties in serialized (insertion) order.
    Object(Vec<(String, V8Value)>),
    /// `kBeginJSMap` — key/value pairs in insertion order.
    Map(Vec<(V8Value, V8Value)>),
    /// `kBeginJSSet` — members in insertion order.
    Set(Vec<V8Value>),
    /// `kArrayBuffer` — the raw bytes.
    ArrayBuffer(Vec<u8>),
    /// `kNumberObject` — `new Number(x)`.
    NumberObject(f64),
    /// `kStringObject` — `new String(x)`.
    StringObject(String),
    /// `kTrueObject` / `kFalseObject` — `new Boolean(x)`.
    BooleanObject(bool),
    /// `kBigIntObject` — `Object(x)` of a BigInt, decimal string.
    BigIntObject(String),
}

/// A decode failure. Every arm names *what* failed and *where* (byte offset), and
/// surfaces the offending value (tag byte / id) so an analyst can identify it —
/// an "unknown" is never reported without the bytes that were actually there.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum V8Error {
    /// Fewer bytes remained than the value required.
    #[error("truncated at offset {offset}: needed {needed} more byte(s), {available} available")]
    Truncated {
        offset: usize,
        needed: usize,
        available: usize,
    },
    /// The stream did not open with the `0xFF` version tag.
    #[error("missing V8 version header at offset {offset}: expected 0xFF, found 0x{found:02x}")]
    BadVersion { offset: usize, found: u8 },
    /// A tag byte is not a value-introducing V8 serialization tag we decode.
    #[error("unsupported V8 serialization tag 0x{tag:02x} ({tag:?} as char) at offset {offset}")]
    UnsupportedTag { offset: usize, tag: u8 },
    /// A `kHostObject` (`\\`) embedder value (Blink DOM type or a node typed-array
    /// delegate). Surfaced with the following Blink tag byte, not fabricated.
    #[error(
        "unsupported host/embedder object at offset {offset} (kHostObject, Blink tag 0x{blink_tag:02x})"
    )]
    HostObject { offset: usize, blink_tag: u8 },
    /// A `kObjectReference` pointed at an id that was never assigned.
    #[error("dangling object reference to id {id} at offset {offset}")]
    BadReference { offset: usize, id: u64 },
    /// A varint ran past 10 bytes / would overflow `u64`.
    #[error("malformed varint at offset {offset}")]
    BadVarint { offset: usize },
    /// A `kTwoByteString` had an odd byte length (not whole UTF-16 units).
    #[error("odd-length UTF-16 string ({len} bytes) at offset {offset}")]
    OddUtf16 { offset: usize, len: usize },
    /// An object/map key was neither a string nor an integer.
    #[error("unsupported property key type at offset {offset}")]
    BadKey { offset: usize },
    /// Recursion depth exceeded [`V8Limits::max_depth`] (DoS guard).
    #[error("recursion depth cap ({cap}) exceeded at offset {offset}")]
    DepthCap { offset: usize, cap: usize },
    /// Total materialized nodes exceeded [`V8Limits::max_nodes`] (DoS guard).
    #[error("value/node cap ({cap}) exceeded")]
    NodeCap { cap: usize },
    /// Total materialized string/buffer bytes, including copies, exceeded the cap.
    #[error("payload byte cap ({cap}) exceeded")]
    ByteCap { cap: usize },
    #[error("invalid container metadata at offset {offset}")]
    BadContainer { offset: usize },
    /// A declared length exceeded the node cap (e.g. a huge sparse-array length or
    /// bigint digit count) — rejected before allocating.
    #[error("declared length {len} exceeds cap {cap} at offset {offset}")]
    LengthCap { offset: usize, len: u64, cap: usize },
}

/// Resource bounds for the deserializer — the guard against stack overflow,
/// unbounded allocation, and reference-amplification on untrusted input.
#[derive(Debug, Clone, Copy)]
pub struct V8Limits {
    /// Maximum nesting depth of arrays/objects/maps/sets.
    pub max_depth: usize,
    /// Maximum total materialized values, including registered and referenced copies.
    pub max_nodes: usize,
    /// Maximum materialized string/buffer bytes, including keys and copies.
    pub max_bytes: usize,
}

impl Default for V8Limits {
    fn default() -> Self {
        Self {
            max_depth: 256,
            max_nodes: 4_000_000,
            max_bytes: 128 * 1024 * 1024,
        }
    }
}

// ---------------------------------------------------------------------------
// Serialization tag bytes.
//
// Mirrored from forensicnomicon-core::v8_serialization (the canonical tag table);
// only the value-introducing tags this decoder implements are named here.
// ---------------------------------------------------------------------------

// More than sufficient for usage counters (2048 bits), while bounding the
// quadratic base-256 to decimal conversion independently of the node budget.
const MAX_BIGINT_BYTES: usize = 256;

const TAG_VERSION: u8 = 0xFF;
const TAG_THE_HOLE: u8 = b'-';
const TAG_UNDEFINED: u8 = b'_';
const TAG_NULL: u8 = b'0';
const TAG_TRUE: u8 = b'T';
const TAG_FALSE: u8 = b'F';
const TAG_INT32: u8 = b'I';
const TAG_UINT32: u8 = b'U';
const TAG_DOUBLE: u8 = b'N';
const TAG_BIGINT: u8 = b'Z';
const TAG_UTF8_STRING: u8 = b'S';
const TAG_ONE_BYTE_STRING: u8 = b'"';
const TAG_TWO_BYTE_STRING: u8 = b'c';
const TAG_OBJECT_REFERENCE: u8 = b'^';
const TAG_BEGIN_OBJECT: u8 = b'o';
const TAG_END_OBJECT: u8 = b'{';
const TAG_BEGIN_SPARSE_ARRAY: u8 = b'a';
const TAG_END_SPARSE_ARRAY: u8 = b'@';
const TAG_BEGIN_DENSE_ARRAY: u8 = b'A';
const TAG_END_DENSE_ARRAY: u8 = b'$';
const TAG_DATE: u8 = b'D';
const TAG_TRUE_OBJECT: u8 = b'y';
const TAG_FALSE_OBJECT: u8 = b'x';
const TAG_NUMBER_OBJECT: u8 = b'n';
const TAG_BIGINT_OBJECT: u8 = b'z';
const TAG_STRING_OBJECT: u8 = b's';
const TAG_REGEXP: u8 = b'R';
const TAG_BEGIN_MAP: u8 = b';';
const TAG_END_MAP: u8 = b':';
const TAG_BEGIN_SET: u8 = b'\'';
const TAG_END_SET: u8 = b',';
const TAG_ARRAY_BUFFER: u8 = b'B';
const TAG_HOST_OBJECT: u8 = b'\\';

/// Blink `kTrailerOffsetTag` — introduces an 8-byte BE offset + 4-byte BE size.
const BLINK_TRAILER_OFFSET: u8 = 0xFE;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Deserialize a raw V8 `ValueSerializer` stream (`0xFF <version> <value>`) with
/// default [`V8Limits`].
///
/// # Errors
/// Returns [`V8Error`] on a truncated, malformed, or unsupported (host-object)
/// stream — never panics.
pub fn deserialize(bytes: &[u8]) -> Result<V8Value, V8Error> {
    deserialize_with_limits(bytes, V8Limits::default())
}

/// Deserialize a raw V8 stream with explicit [`V8Limits`].
///
/// # Errors
/// See [`deserialize`].
pub fn deserialize_with_limits(bytes: &[u8], limits: V8Limits) -> Result<V8Value, V8Error> {
    let mut r = Reader::new(bytes, limits);
    r.read_version_header()?;
    r.read_value(0)
}

/// Deserialize a Blink `SerializedScriptValue` (the on-disk IndexedDB form): the
/// Blink `0xFF <version>` envelope, an optional `0xFE` trailer, then the nested
/// V8 payload. Falls back to a raw V8 read when no Blink envelope is present.
///
/// # Errors
/// See [`deserialize`].
pub fn deserialize_blink(bytes: &[u8]) -> Result<V8Value, V8Error> {
    deserialize_blink_with_limits(bytes, V8Limits::default())
}

/// Deserialize a Blink `SerializedScriptValue` with explicit [`V8Limits`].
///
/// # Errors
/// See [`deserialize`].
pub fn deserialize_blink_with_limits(bytes: &[u8], limits: V8Limits) -> Result<V8Value, V8Error> {
    let mut r = Reader::new(bytes, limits);
    // Blink envelope: 0xFF <blink-version>. If absent, treat the whole buffer as a
    // raw V8 stream (some extraction paths hand back the inner payload directly).
    if r.peek() == Some(TAG_VERSION) {
        r.pos += 1;
        let _blink_version = r.read_varint()?;
        // Consume envelope framing tags (currently only the trailer offset) until
        // the nested V8 version header (`0xFF`) begins.
        while r.peek() == Some(BLINK_TRAILER_OFFSET) {
            r.pos += 1;
            // 8-byte BE offset + 4-byte BE size.
            r.take(12)?;
        }
        if r.peek() != Some(TAG_VERSION) {
            // A raw V8 stream has only one version header.
            r.pos = 0;
        }
    }
    r.read_version_header()?;
    r.read_value(0)
}

/// True when `tag` opens a value we recognise — used by the identifier to decide
/// whether a `0xFF`-led blob is plausibly V8 before reporting a failed decode.
#[must_use]
pub fn is_value_tag(tag: u8) -> bool {
    matches!(
        tag,
        TAG_THE_HOLE
            | TAG_UNDEFINED
            | TAG_NULL
            | TAG_TRUE
            | TAG_FALSE
            | TAG_INT32
            | TAG_UINT32
            | TAG_DOUBLE
            | TAG_BIGINT
            | TAG_UTF8_STRING
            | TAG_ONE_BYTE_STRING
            | TAG_TWO_BYTE_STRING
            | TAG_OBJECT_REFERENCE
            | TAG_BEGIN_OBJECT
            | TAG_BEGIN_SPARSE_ARRAY
            | TAG_BEGIN_DENSE_ARRAY
            | TAG_DATE
            | TAG_TRUE_OBJECT
            | TAG_FALSE_OBJECT
            | TAG_NUMBER_OBJECT
            | TAG_BIGINT_OBJECT
            | TAG_STRING_OBJECT
            | TAG_REGEXP
            | TAG_BEGIN_MAP
            | TAG_BEGIN_SET
            | TAG_ARRAY_BUFFER
            | TAG_HOST_OBJECT
    )
}

impl V8Value {
    /// A short, bounded, human summary of this value's shape — for the
    /// identifier's candidate summary line.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Undefined => "undefined".to_owned(),
            Self::Null => "null".to_owned(),
            Self::Hole => "hole".to_owned(),
            Self::Bool(b) => format!("boolean {b}"),
            Self::Int(i) => format!("integer {i}"),
            Self::Double(d) => format!("number {d}"),
            Self::BigInt(s) => format!("bigint {s}n"),
            Self::String(s) => format!("string {:?}", ellipsize(s)),
            Self::Date(ms) => format!("date ({ms} ms)"),
            Self::RegExp { source, flags } => {
                format!("regexp /{}/ (flags {flags})", ellipsize(source))
            }
            Self::Array(v) => format!("array ({} element{})", v.len(), plural(v.len())),
            Self::Object(kv) => format!("object ({} key{})", kv.len(), plural(kv.len())),
            Self::Map(kv) => format!(
                "map ({} entr{})",
                kv.len(),
                if kv.len() == 1 { "y" } else { "ies" }
            ),
            Self::Set(v) => format!("set ({} member{})", v.len(), plural(v.len())),
            Self::ArrayBuffer(b) => format!("arraybuffer ({} byte{})", b.len(), plural(b.len())),
            Self::NumberObject(d) => format!("Number({d})"),
            Self::StringObject(s) => format!("String({:?})", ellipsize(s)),
            Self::BooleanObject(b) => format!("Boolean({b})"),
            Self::BigIntObject(s) => format!("BigInt({s})"),
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn ellipsize(s: &str) -> String {
    const MAX: usize = 32;
    if s.chars().count() <= MAX {
        s.to_owned()
    } else {
        let head: String = s.chars().take(MAX).collect();
        format!("{head}…")
    }
}

// ---------------------------------------------------------------------------
// Reader — bounds-checked, panic-free cursor
// ---------------------------------------------------------------------------

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    limits: V8Limits,
    /// Objects assigned an id, in V8 assignment order. `None` = reserved but not
    /// yet filled (an in-progress object; a reference to it before completion is a
    /// cyclic structure we decline rather than loop on).
    id_map: Vec<Option<V8Value>>,
    /// Remaining budgets, including all registered and referenced copies.
    budget: usize,
    byte_budget: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], limits: V8Limits) -> Self {
        let budget = limits.max_nodes;
        Self {
            data,
            pos: 0,
            limits,
            id_map: Vec::new(),
            budget,
            byte_budget: limits.max_bytes,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn read_u8(&mut self) -> Result<u8, V8Error> {
        let b = self.data.get(self.pos).copied().ok_or(V8Error::Truncated {
            offset: self.pos,
            needed: 1,
            available: 0,
        })?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], V8Error> {
        let end = self.pos.checked_add(n).ok_or(V8Error::Truncated {
            offset: self.pos,
            needed: n,
            available: self.data.len().saturating_sub(self.pos),
        })?;
        let slice = self.data.get(self.pos..end).ok_or(V8Error::Truncated {
            offset: self.pos,
            needed: n,
            available: self.data.len().saturating_sub(self.pos),
        })?;
        self.pos = end;
        Ok(slice)
    }

    /// Unsigned LEB128 varint, capped at 10 bytes (64 bits).
    fn read_varint(&mut self) -> Result<u64, V8Error> {
        let start = self.pos;
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            if shift >= 64 {
                return Err(V8Error::BadVarint { offset: start });
            }
            let byte = self.read_u8()?;
            if shift == 63 && byte > 1 {
                return Err(V8Error::BadVarint { offset: start });
            }
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    /// Zig-zag decoded signed varint (`kInt32`).
    fn read_zigzag(&mut self) -> Result<i64, V8Error> {
        let u = self.read_varint()?;
        // (u >> 1) ^ -(u & 1)
        Ok(((u >> 1) as i64) ^ -((u & 1) as i64))
    }

    fn read_f64_le(&mut self) -> Result<f64, V8Error> {
        let b = self.take(8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(b);
        Ok(f64::from_le_bytes(arr))
    }

    fn read_version_header(&mut self) -> Result<(), V8Error> {
        while self.peek() == Some(0) {
            self.read_u8()?;
        }
        let offset = self.pos;
        let tag = self.read_u8()?;
        if tag != TAG_VERSION {
            return Err(V8Error::BadVersion { offset, found: tag });
        }
        // Format version follows as a varint; we accept any (forward-compatible).
        let _version = self.read_varint()?;
        Ok(())
    }

    /// Charge one node against the budget (call once per materialized value).
    fn charge(&mut self, n: usize) -> Result<(), V8Error> {
        if n > self.budget {
            return Err(V8Error::NodeCap {
                cap: self.limits.max_nodes,
            });
        }
        self.budget -= n;
        Ok(())
    }

    /// Reserve the next object id (before reading contents, matching V8, so
    /// forward references in acyclic data resolve).
    fn reserve_id(&mut self) -> usize {
        let id = self.id_map.len();
        self.id_map.push(None);
        id
    }

    fn charge_bytes(&mut self, bytes: usize) -> Result<(), V8Error> {
        if bytes > self.byte_budget {
            return Err(V8Error::ByteCap {
                cap: self.limits.max_bytes,
            });
        }
        self.byte_budget -= bytes;
        Ok(())
    }

    fn fill_id(&mut self, id: usize, value: &V8Value) -> Result<(), V8Error> {
        let (nodes, bytes, _) = materialized_size(value);
        self.charge(nodes)?;
        self.charge_bytes(bytes)?;
        self.id_map[id] = Some(value.clone());
        Ok(())
    }

    fn read_value(&mut self, depth: usize) -> Result<V8Value, V8Error> {
        if depth > self.limits.max_depth {
            return Err(V8Error::DepthCap {
                offset: self.pos,
                cap: self.limits.max_depth,
            });
        }
        self.charge(1)?;
        while self.peek() == Some(0) {
            self.read_u8()?;
        }
        let offset = self.pos;
        let tag = self.read_u8()?;
        match tag {
            TAG_UNDEFINED => Ok(V8Value::Undefined),
            TAG_NULL => Ok(V8Value::Null),
            TAG_THE_HOLE => Ok(V8Value::Hole),
            TAG_TRUE => Ok(V8Value::Bool(true)),
            TAG_FALSE => Ok(V8Value::Bool(false)),
            TAG_INT32 => Ok(V8Value::Int(self.read_zigzag()?)),
            TAG_UINT32 => Ok(V8Value::Int(self.read_varint()? as i64)),
            TAG_DOUBLE => Ok(V8Value::Double(self.read_f64_le()?)),
            TAG_BIGINT => Ok(V8Value::BigInt(self.read_bigint()?)),
            TAG_UTF8_STRING => Ok(V8Value::String(self.read_utf8_string()?)),
            TAG_ONE_BYTE_STRING => Ok(V8Value::String(self.read_one_byte_string()?)),
            TAG_TWO_BYTE_STRING => Ok(V8Value::String(self.read_two_byte_string()?)),
            TAG_DATE => {
                let id = self.reserve_id();
                let v = V8Value::Date(self.read_f64_le()?);
                self.fill_id(id, &v)?;
                Ok(v)
            }
            TAG_BEGIN_OBJECT => self.read_js_object(depth),
            TAG_BEGIN_DENSE_ARRAY => self.read_dense_array(depth),
            TAG_BEGIN_SPARSE_ARRAY => self.read_sparse_array(depth),
            TAG_BEGIN_MAP => self.read_map(depth),
            TAG_BEGIN_SET => self.read_set(depth),
            TAG_ARRAY_BUFFER => self.read_array_buffer(),
            TAG_REGEXP => self.read_regexp(depth),
            TAG_NUMBER_OBJECT => {
                let id = self.reserve_id();
                let v = V8Value::NumberObject(self.read_f64_le()?);
                self.fill_id(id, &v)?;
                Ok(v)
            }
            TAG_TRUE_OBJECT => {
                let id = self.reserve_id();
                let v = V8Value::BooleanObject(true);
                self.fill_id(id, &v)?;
                Ok(v)
            }
            TAG_FALSE_OBJECT => {
                let id = self.reserve_id();
                let v = V8Value::BooleanObject(false);
                self.fill_id(id, &v)?;
                Ok(v)
            }
            TAG_STRING_OBJECT => {
                let id = self.reserve_id();
                let inner = self.read_value(depth + 1)?;
                let V8Value::String(s) = inner else {
                    return Err(V8Error::UnsupportedTag { offset, tag });
                };
                let v = V8Value::StringObject(s);
                self.fill_id(id, &v)?;
                Ok(v)
            }
            TAG_BIGINT_OBJECT => {
                let id = self.reserve_id();
                let v = V8Value::BigIntObject(self.read_bigint()?);
                self.fill_id(id, &v)?;
                Ok(v)
            }
            TAG_OBJECT_REFERENCE => self.read_reference(depth),
            TAG_HOST_OBJECT => {
                let blink_tag = self.read_u8().unwrap_or(0);
                Err(V8Error::HostObject { offset, blink_tag })
            }
            _ => Err(V8Error::UnsupportedTag { offset, tag }),
        }
    }

    fn read_utf8_string(&mut self) -> Result<String, V8Error> {
        let raw = self.read_varint()?;
        let len = self.checked_len(raw)?;
        let bytes = self.take(len)?;
        self.charge_bytes(len.saturating_mul(3))?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    fn read_one_byte_string(&mut self) -> Result<String, V8Error> {
        let raw = self.read_varint()?;
        let len = self.checked_len(raw)?;
        let bytes = self.take(len)?;
        self.charge_bytes(len.saturating_mul(2))?;
        // One-byte strings are Latin-1 (ISO-8859-1): each byte is a code point.
        Ok(bytes.iter().map(|&b| b as char).collect())
    }

    fn read_two_byte_string(&mut self) -> Result<String, V8Error> {
        let offset = self.pos;
        let raw = self.read_varint()?;
        let len = self.checked_len(raw)?;
        if len % 2 != 0 {
            return Err(V8Error::OddUtf16 { offset, len });
        }
        let bytes = self.take(len)?;
        self.charge_bytes(len.saturating_mul(3) / 2)?;
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(String::from_utf16_lossy(&units))
    }

    fn read_bigint(&mut self) -> Result<String, V8Error> {
        let offset = self.pos;
        let bitfield = self.read_varint()?;
        let negative = bitfield & 1 == 1;
        let raw_len = bitfield >> 1;
        if raw_len > MAX_BIGINT_BYTES as u64 {
            return Err(V8Error::LengthCap {
                offset,
                len: raw_len,
                cap: MAX_BIGINT_BYTES,
            });
        }
        let byte_len = self.checked_len(raw_len)?;
        self.charge_bytes(byte_len.saturating_mul(3).saturating_add(1))?;
        let digits = self.take(byte_len)?;
        let magnitude = le_bytes_to_decimal(digits);
        if magnitude == "0" {
            // A zero bigint is never negative.
            return Ok(magnitude);
        }
        if negative {
            Ok(format!("-{magnitude}"))
        } else {
            Ok(magnitude)
        }
    }

    fn read_array_buffer(&mut self) -> Result<V8Value, V8Error> {
        let id = self.reserve_id();
        let raw = self.read_varint()?;
        let len = self.checked_len(raw)?;
        self.charge_bytes(len)?;
        let bytes = self.take(len)?.to_vec();
        let v = V8Value::ArrayBuffer(bytes);
        self.fill_id(id, &v)?;
        Ok(v)
    }

    fn read_regexp(&mut self, depth: usize) -> Result<V8Value, V8Error> {
        let offset = self.pos;
        let id = self.reserve_id();
        let V8Value::String(source) = self.read_value(depth + 1)? else {
            return Err(V8Error::BadKey { offset });
        };
        let flags = u32::try_from(self.read_varint()?).unwrap_or(u32::MAX);
        let v = V8Value::RegExp { source, flags };
        self.fill_id(id, &v)?;
        Ok(v)
    }

    fn read_js_object(&mut self, depth: usize) -> Result<V8Value, V8Error> {
        let id = self.reserve_id();
        let mut props = Vec::new();
        loop {
            if self.peek() == Some(TAG_END_OBJECT) {
                self.pos += 1;
                break;
            }
            let key = self.read_property_key(depth + 1)?;
            let value = self.read_value(depth + 1)?;
            props.push((key, value));
        }
        if self.read_varint()? != props.len() as u64 {
            return Err(V8Error::BadContainer { offset: self.pos });
        }
        let v = V8Value::Object(props);
        self.fill_id(id, &v)?;
        Ok(v)
    }

    fn read_dense_array(&mut self, depth: usize) -> Result<V8Value, V8Error> {
        let raw = self.read_varint()?;
        let length = self.checked_len(raw)?;
        let id = self.reserve_id();
        let mut elems = Vec::new();
        for _ in 0..length {
            elems.push(self.read_value(depth + 1)?);
        }
        let mut count = 0;
        while self.peek() != Some(TAG_END_DENSE_ARRAY) {
            self.read_property_key(depth + 1)?;
            self.read_value(depth + 1)?;
            count += 1;
        }
        self.read_u8()?;
        if self.read_varint()? != count || self.read_varint()? != raw {
            return Err(V8Error::BadContainer { offset: self.pos });
        }
        let v = V8Value::Array(elems);
        self.fill_id(id, &v)?;
        Ok(v)
    }

    fn read_sparse_array(&mut self, depth: usize) -> Result<V8Value, V8Error> {
        let raw = self.read_varint()?;
        let length = self.checked_len(raw)?;
        let id = self.reserve_id();
        // Charge the materialized (hole-filled) length up front so a huge declared
        // length that has no backing bytes still fails loud.
        self.charge(length)?;
        let mut elems = vec![V8Value::Hole; length];
        loop {
            if self.peek() == Some(TAG_END_SPARSE_ARRAY) {
                self.pos += 1;
                break;
            }
            let key = self.read_value(depth + 1)?;
            let value = self.read_value(depth + 1)?;
            // Integer keys are array indices; named keys are extra properties we do
            // not attach to the positional array (kept as holes).
            if let V8Value::Int(i) = key {
                if let Ok(idx) = usize::try_from(i) {
                    if idx < elems.len() {
                        elems[idx] = value;
                    }
                }
            }
        }
        let _num_props = self.read_varint()?;
        let _length_again = self.read_varint()?;
        let v = V8Value::Array(elems);
        self.fill_id(id, &v)?;
        Ok(v)
    }

    fn read_map(&mut self, depth: usize) -> Result<V8Value, V8Error> {
        let id = self.reserve_id();
        let mut entries = Vec::new();
        loop {
            if self.peek() == Some(TAG_END_MAP) {
                self.pos += 1;
                break;
            }
            let key = self.read_value(depth + 1)?;
            let value = self.read_value(depth + 1)?;
            entries.push((key, value));
        }
        // Trailing count (= 2 × entries).
        let _count = self.read_varint()?;
        let v = V8Value::Map(entries);
        self.fill_id(id, &v)?;
        Ok(v)
    }

    fn read_set(&mut self, depth: usize) -> Result<V8Value, V8Error> {
        let id = self.reserve_id();
        let mut members = Vec::new();
        loop {
            if self.peek() == Some(TAG_END_SET) {
                self.pos += 1;
                break;
            }
            members.push(self.read_value(depth + 1)?);
        }
        let _count = self.read_varint()?;
        let v = V8Value::Set(members);
        self.fill_id(id, &v)?;
        Ok(v)
    }

    /// A property key is a serialized value; V8 emits strings and integer indices.
    fn read_property_key(&mut self, depth: usize) -> Result<String, V8Error> {
        let offset = self.pos;
        let key = match self.read_value(depth)? {
            V8Value::String(s) => return Ok(s), // Already charged by the string reader.
            V8Value::Int(i) => i.to_string(),
            V8Value::Double(d) => format!("{d}"),
            _ => return Err(V8Error::BadKey { offset }),
        };
        self.charge_bytes(key.len())?;
        Ok(key)
    }

    fn read_reference(&mut self, depth: usize) -> Result<V8Value, V8Error> {
        let offset = self.pos;
        let id = self.read_varint()?;
        let idx = usize::try_from(id).map_err(|_| V8Error::BadReference { offset, id })?;
        let value = self
            .id_map
            .get(idx)
            .and_then(Option::as_ref)
            .ok_or(V8Error::BadReference { offset, id })?;
        let (nodes, bytes, height) = materialized_size(value);
        if depth.saturating_add(height) > self.limits.max_depth {
            return Err(V8Error::DepthCap {
                offset,
                cap: self.limits.max_depth,
            });
        }
        // Check all budgets BEFORE cloning: both explicit references and the
        // ID registry hold owned copies, including large strings and buffers.
        self.charge(nodes)?;
        self.charge_bytes(bytes)?;
        Ok(self.id_map[idx].as_ref().unwrap().clone())
    }

    /// Reject a declared length that exceeds the node cap before allocating.
    fn checked_len(&self, len: u64) -> Result<usize, V8Error> {
        let cap = self.limits.max_nodes as u64;
        if len > cap {
            return Err(V8Error::LengthCap {
                offset: self.pos,
                len,
                cap: self.limits.max_nodes,
            });
        }
        // The guard above already rejected len > cap (max_nodes, 4_000_000), so
        // len <= cap == max_nodes as u64. max_nodes is a usize, so any value <= it
        // fits usize on every supported target and this map_err closure is a
        // defense-in-depth backstop no input can reach.
        usize::try_from(len).map_err(|_| V8Error::LengthCap {
            offset: self.pos,           // cov:unreachable: len <= cap makes try_from infallible
            len,                        // cov:unreachable: len <= cap makes try_from infallible
            cap: self.limits.max_nodes, // cov:unreachable: len <= cap makes try_from infallible
        })
    }
}

/// Nodes, owned payload bytes and subtree height of a materialized value.
fn materialized_size(value: &V8Value) -> (usize, usize, usize) {
    let mut nodes: usize = 1;
    let mut bytes: usize = 0;
    let mut height: usize = 0;
    let mut add_child = |child: &V8Value| {
        let (n, b, h) = materialized_size(child);
        nodes = nodes.saturating_add(n);
        bytes = bytes.saturating_add(b);
        height = height.max(h.saturating_add(1));
    };
    match value {
        V8Value::Array(items) | V8Value::Set(items) => items.iter().for_each(add_child),
        V8Value::Object(items) => {
            for (_, child) in items {
                add_child(child);
            }
            for (key, _) in items {
                bytes = bytes.saturating_add(key.len());
            }
        }
        V8Value::Map(items) => {
            for (key, child) in items {
                add_child(key);
                add_child(child);
            }
        }
        V8Value::String(s)
        | V8Value::BigInt(s)
        | V8Value::StringObject(s)
        | V8Value::BigIntObject(s)
        | V8Value::RegExp { source: s, .. } => bytes = s.len(),
        V8Value::ArrayBuffer(buffer) => bytes = buffer.len(),
        _ => {}
    }
    (nodes, bytes, height)
}

/// Render a little-endian byte magnitude to a decimal string (schoolbook
/// base-256 → base-10). Returns `"0"` for an empty / all-zero magnitude.
fn le_bytes_to_decimal(bytes: &[u8]) -> String {
    let mut decimal: Vec<u8> = vec![0]; // little-endian decimal digits
    for &byte in bytes.iter().rev() {
        let mut carry = u32::from(byte);
        for d in &mut decimal {
            let v = u32::from(*d) * 256 + carry;
            *d = (v % 10) as u8;
            carry = v / 10;
        }
        while carry > 0 {
            decimal.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    // Strip leading zeros (most-significant end), then render most-significant first.
    //
    // The schoolbook loop above never leaves a zero in the most-significant slot.
    // Induction: `decimal` starts `[0]` (len 1). A digit is appended only by
    // `while carry > 0`, whose final push is `carry` itself with 0 < carry < 10, so
    // a pushed top digit is non-zero. Otherwise the top digit becomes
    // `v = d_top * 256 + carry_in` with v < 10, which is 0 only when d_top == 0 and
    // carry_in == 0 — and d_top == 0 implies len == 1 by the same induction, which
    // this loop's own `len > 1` guard excludes. Verified by exhaustive replay of all
    // magnitudes up to 3 bytes (16,843,009 cases) plus 200k random magnitudes up to
    // 12 bytes: the loop never fires. Kept as a defensive normalizer so a future
    // change to the carry loop cannot emit a leading-zero decimal.
    while decimal.len() > 1 && *decimal.last().unwrap_or(&0) == 0 {
        decimal.pop(); // cov:unreachable: the carry loop never leaves a zero on top
    }
    decimal.iter().rev().map(|d| (b'0' + d) as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut n: usize, bytes: &mut Vec<u8>) {
        while n >= 128 {
            bytes.push((n as u8 & 127) | 128);
            n >>= 7;
        }
        bytes.push(n as u8);
    }

    #[test]
    fn bigint_conversion_has_a_separate_small_byte_limit() {
        for tag in [TAG_BIGINT, TAG_BIGINT_OBJECT] {
            for negative in [false, true] {
                let mut bytes = vec![TAG_VERSION, 15, tag];
                varint((MAX_BIGINT_BYTES << 1) | usize::from(negative), &mut bytes);
                bytes.extend([255; MAX_BIGINT_BYTES]);
                let value = deserialize(&bytes).unwrap();
                let decimal = match value {
                    V8Value::BigInt(s) | V8Value::BigIntObject(s) => s,
                    _ => panic!("expected bigint"),
                };
                assert_eq!(decimal.starts_with('-'), negative);
                assert!(decimal.len() > 600);

                // Reject the declared length even before looking for payload bytes.
                let mut oversized = vec![TAG_VERSION, 15, tag];
                varint(
                    ((MAX_BIGINT_BYTES + 1) << 1) | usize::from(negative),
                    &mut oversized,
                );
                assert!(matches!(
                    deserialize(&oversized),
                    Err(V8Error::LengthCap {
                        cap: MAX_BIGINT_BYTES,
                        ..
                    })
                ));
            }
        }
        assert_eq!(
            deserialize(&[255, 15, b'Z', 0]).unwrap(),
            V8Value::BigInt("0".into())
        );
        assert_eq!(
            deserialize(&[255, 15, b'Z', 3, 42]).unwrap(),
            V8Value::BigInt("-42".into())
        );
        assert_eq!(
            deserialize(&[255, 15, b'Z', 3, 0]).unwrap(),
            V8Value::BigInt("0".into())
        );
    }

    #[test]
    fn nested_object_registration_is_charged_against_node_budget() {
        // Three wire values, but the inner and outer arrays retain copies of
        // their complete subtrees even though there are no reference tags.
        let bytes = [255, 15, b'A', 1, b'A', 1, b'T', b'$', 0, 1, b'$', 0, 1];
        let limits = V8Limits {
            max_nodes: 7,
            ..V8Limits::default()
        };
        assert_eq!(
            deserialize_with_limits(&bytes, limits),
            Err(V8Error::NodeCap { cap: 7 })
        );
        assert!(deserialize_with_limits(
            &bytes,
            V8Limits {
                max_nodes: 8,
                ..limits
            }
        )
        .is_ok());
    }

    #[test]
    fn payload_copies_cannot_bypass_node_limits() {
        // One large buffer has only one node. Each stored/referenced copy also
        // consumes payload budget, including the final outer array registration.
        let mut bytes = vec![255, 15, b'A', 2, b'B', 64];
        bytes.extend([1; 64]);
        bytes.extend([b'^', 1, b'$', 0, 2]);
        let limits = V8Limits {
            max_bytes: 191,
            ..V8Limits::default()
        };
        assert_eq!(
            deserialize_with_limits(&bytes, limits),
            Err(V8Error::ByteCap { cap: 191 })
        );
        assert!(deserialize_with_limits(
            &bytes,
            V8Limits {
                max_bytes: 320,
                ..limits
            }
        )
        .is_ok());

        // Nested containers also duplicate property names and string payloads.
        let bytes = [
            255, 15, b'A', 1, b'o', b'S', 1, b'k', b'S', 1, b'v', b'{', 1, b'$', 0, 1,
        ];
        assert!(matches!(
            deserialize_with_limits(
                &bytes,
                V8Limits {
                    max_bytes: 9,
                    ..limits
                }
            ),
            Err(V8Error::ByteCap { .. })
        ));
        assert!(deserialize_with_limits(
            &bytes,
            V8Limits {
                max_bytes: 10,
                ..limits
            }
        )
        .is_ok());
    }

    #[test]
    fn references_cannot_expand_past_the_depth_limit() {
        // Both inner arrays are shallow on the wire, but the second references
        // the first, making the materialized tree one level deeper.
        let bytes = [
            255, 15, b'A', 2, b'A', 1, b'T', b'$', 0, 1, b'A', 1, b'^', 1, b'$', 0, 1, b'$', 0, 2,
        ];
        let limits = V8Limits {
            max_depth: 2,
            ..V8Limits::default()
        };
        assert!(matches!(
            deserialize_with_limits(&bytes, limits),
            Err(V8Error::DepthCap { .. })
        ));
        assert!(deserialize_with_limits(
            &bytes,
            V8Limits {
                max_depth: 3,
                ..limits
            }
        )
        .is_ok());
    }
    #[test]
    fn unicode_alignment_and_object_references() {
        let bytes = [
            0xff, 15, b'A', 2, b'o', b'"', 1, b'x', 0, b'c', 2, 0x16, 0x04, b'{', 1, b'^', 1, b'$',
            0, 2,
        ];
        let value = deserialize_blink(&bytes).unwrap();
        let expected = V8Value::Object(vec![("x".into(), V8Value::String("Ж".into()))]);
        assert_eq!(value, V8Value::Array(vec![expected.clone(), expected]));
    }
    #[test]
    fn invalid_lengths_depth_and_unsupported_tags_fail() {
        assert!(deserialize(&[0xff, 15, b'A', 0, b'$', 0, 1]).is_err());
        assert!(deserialize(&[0xff, 15, 0x7f]).is_err());
        assert!(deserialize_with_limits(
            &[0xff, 15, b'A', 1, b'o', b'{', 0, b'$', 0, 1],
            V8Limits {
                max_depth: 0,
                max_nodes: 100,
                ..V8Limits::default()
            }
        )
        .is_err());
        assert!(
            deserialize(&[0xff, 15, b'U', 255, 255, 255, 255, 255, 255, 255, 255, 255, 2]).is_err()
        );
    }
}
