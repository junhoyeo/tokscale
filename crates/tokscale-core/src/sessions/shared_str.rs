//! Interned message strings.
//!
//! A parsed corpus is hundreds of thousands of [`crate::sessions::UnifiedMessage`]
//! values that repeat the same client, model, provider, session, workspace, and
//! date. Owning a fresh `String` for each copy is what made a single client
//! lane resident at about a gigabyte (#1209). `SharedStr` keeps one allocation
//! per distinct value and hands out refcounted handles.
//!
//! The serde encoding is a bare string, the same bytes as `String`, so existing
//! source-cache shards stay readable. Do not use this for values that are
//! unique per message (dedup keys): the pool retains every distinct value for
//! the process lifetime, which is the point for repeated fields and a leak for
//! unique ones.

use std::borrow::Borrow;
use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::{Arc, LazyLock, Mutex};

const SHARDS: usize = 16;

static POOL: LazyLock<[Mutex<HashSet<Arc<str>>>; SHARDS]> =
    LazyLock::new(|| std::array::from_fn(|_| Mutex::new(HashSet::new())));

thread_local! {
    static LOCAL: std::cell::RefCell<HashSet<Arc<str>>> =
        std::cell::RefCell::new(HashSet::new());
}

fn shard_index(value: &str) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    (hasher.finish() as usize) % SHARDS
}

fn global_intern(value: &str) -> Arc<str> {
    let shard = &POOL[shard_index(value)];
    let mut pool = shard
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = pool.get(value) {
        return Arc::clone(existing);
    }
    let interned: Arc<str> = Arc::from(value);
    pool.insert(Arc::clone(&interned));
    interned
}

fn intern(value: &str) -> Arc<str> {
    if let Some(hit) = LOCAL.with(|local| local.borrow().get(value).cloned()) {
        return hit;
    }
    let interned = global_intern(value);
    LOCAL.with(|local| {
        local.borrow_mut().insert(Arc::clone(&interned));
    });
    interned
}

/// A refcounted string shared with every other `SharedStr` of the same contents.
#[derive(Eq)]
pub struct SharedStr(Arc<str>);

impl SharedStr {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when `other` is the same pooled allocation, not merely equal text.
    pub fn shares_allocation_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Default for SharedStr {
    fn default() -> Self {
        Self::from("")
    }
}

impl Clone for SharedStr {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl Deref for SharedStr {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for SharedStr {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for SharedStr {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for SharedStr {
    fn from(value: &str) -> Self {
        Self(intern(value))
    }
}

impl From<String> for SharedStr {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<&String> for SharedStr {
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<SharedStr> for String {
    fn from(value: SharedStr) -> Self {
        value.as_str().to_owned()
    }
}

impl fmt::Display for SharedStr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Debug for SharedStr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), formatter)
    }
}

impl PartialEq for SharedStr {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<str> for SharedStr {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for SharedStr {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for SharedStr {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<SharedStr> for str {
    fn eq(&self, other: &SharedStr) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<SharedStr> for String {
    fn eq(&self, other: &SharedStr) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<SharedStr> for &str {
    fn eq(&self, other: &SharedStr) -> bool {
        *self == other.as_str()
    }
}

impl Hash for SharedStr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl PartialOrd for SharedStr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SharedStr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl serde::Serialize for SharedStr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for SharedStr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = SharedStr;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E>(self, value: &str) -> Result<SharedStr, E>
            where
                E: serde::de::Error,
            {
                Ok(SharedStr::from(value))
            }

            fn visit_string<E>(self, value: String) -> Result<SharedStr, E>
            where
                E: serde::de::Error,
            {
                Ok(SharedStr::from(value))
            }
        }

        deserializer.deserialize_string(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_text_shares_one_allocation() {
        let first = SharedStr::from("claude-opus");
        let second = SharedStr::from(String::from("claude-opus"));
        assert!(first.shares_allocation_with(&second));
        assert_eq!(first, second);
        assert_eq!(first, "claude-opus");
    }

    #[test]
    fn bincode_matches_string() {
        let shared = SharedStr::from("workspace/tokscale");
        let owned = "workspace/tokscale".to_string();
        let shared_bytes = bincode::serialize(&shared).unwrap();
        let owned_bytes = bincode::serialize(&owned).unwrap();
        assert_eq!(shared_bytes, owned_bytes);
        let decoded: SharedStr = bincode::deserialize(&owned_bytes).unwrap();
        assert!(decoded.shares_allocation_with(&shared));
    }
}
