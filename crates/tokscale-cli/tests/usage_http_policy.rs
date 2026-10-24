//! Usage adapters must build their HTTP client through
//! `commands::usage::helpers::http_client()`, which bounds every request.
//!
//! `tokscale_core::http::client()` sets no timeout, so a usage adapter that
//! calls it directly can hang a usage refresh on a stalled server. That call
//! compiles and passes every test that talks to a responsive fixture, so the
//! rule is checked against the source here.

use std::path::{Path, PathBuf};

const FORBIDDEN_CALL: &str = "http::client()";

fn usage_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/usage")
}

/// The part of a source file that ships: everything before its
/// `#[cfg(test)] mod tests` block. Test-only files return `None`.
fn non_test_source(file_name: &str, source: &str) -> Option<String> {
    if file_name == "test_server.rs" {
        return None;
    }
    let source = source.replace("\r\n", "\n");
    let end = source
        .find("#[cfg(test)]\nmod tests {")
        .unwrap_or(source.len());
    Some(source[..end].to_string())
}

#[test]
fn usage_adapters_do_not_call_the_unbounded_shared_client() {
    let mut offenders = Vec::new();
    let mut scanned = 0;
    for entry in std::fs::read_dir(usage_dir()).expect("usage directory must be readable") {
        let path = entry.expect("directory entry must be readable").path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let Some(shipped) = non_test_source(&file_name, &source) else {
            continue;
        };
        scanned += 1;
        for (index, line) in shipped.lines().enumerate() {
            if line.contains(FORBIDDEN_CALL) {
                offenders.push(format!("{file_name}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        scanned > 10,
        "expected to scan the usage adapters, scanned {scanned}"
    );
    assert!(
        offenders.is_empty(),
        "usage adapters must use helpers::http_client() instead of \
         tokscale_core::http::client():\n{}",
        offenders.join("\n")
    );
}

/// The scan must see a call in shipped code and ignore one in the test module;
/// otherwise the test above passes for the wrong reason.
#[test]
fn the_scan_flags_shipped_calls_and_skips_test_modules() {
    let source = "fn fetch() {\n    let c = tokscale_core::http::client();\n}\n\
                  #[cfg(test)]\nmod tests {\n    fn t() { tokscale_core::http::client(); }\n}\n";
    let shipped = non_test_source("amp.rs", source).unwrap();
    assert_eq!(shipped.matches(FORBIDDEN_CALL).count(), 1);

    let test_only = "#[cfg(test)]\nmod tests {\n    fn t() { tokscale_core::http::client(); }\n}\n";
    assert!(!non_test_source("amp.rs", test_only)
        .unwrap()
        .contains(FORBIDDEN_CALL));
    assert!(non_test_source("test_server.rs", source).is_none());
}
