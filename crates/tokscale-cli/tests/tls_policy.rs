//! Guards on which TLS backend each HTTP client in this workspace ends up on.
//!
//! reqwest picks its backend from Cargo features, not from code: `native-tls`
//! implies `default-tls`, and `impl Default for TlsBackend` returns the
//! *native-tls* backend whenever `default-tls` is enabled and `http3` is not.
//! Cargo also unifies features across the dependency graph. So a one-word edit
//! to a shared manifest silently changes the TLS stack of every client in the
//! repo, and nothing about it fails to compile.
//!
//! That is exactly what happened while fixing #1250 (cursor.com blocks rustls'
//! ClientHello): `native-tls` was added to the workspace `reqwest` entry, which
//! moved all ~28 clients onto native-tls while the change claimed to touch only
//! Cursor — and made the `.use_native_tls()` call it added a literal no-op,
//! since it assigns the value the default already had. A second iteration
//! gated vendored OpenSSL on `target_env = "musl"`, which left linux-gnu
//! looking for a system libssl that the cargo-zigbuild cross environment does
//! not have, and turned both linux-gnu release builds red.
//!
//! These tests read the manifests and the source, because the properties they
//! protect live there rather than in any value a normal unit test can observe.
//! The complementary runtime check is in `tokscale_core::http`, which asserts
//! the shared builder really does select rustls.

use std::path::{Path, PathBuf};

/// Cursor's TLS opt-in is excluded on exactly this cfg. Android is the one
/// target that must keep pure rustls: openssl-src configures Android with
/// `no-stdio`, which stubs `BIO_new_file`/`BIO_s_file` and so disables
/// OpenSSL's CA-file loader entirely. A native-tls Android build would run
/// with an empty trust store and reject every certificate, silently.
const NATIVE_TLS_TARGET: &str = r#"cfg(not(target_os = "android"))"#;

/// Vendoring must key off the OS, not the libc. `target_os = "linux"` is true
/// for both -gnu and -musl and false for Android (which reports
/// `target_os = "android"`), so this covers every Linux release artifact and
/// no others.
const VENDORED_TARGET: &str = r#"cfg(target_os = "linux")"#;

const MEMBER_MANIFESTS: [&str; 2] = [
    "crates/tokscale-cli/Cargo.toml",
    "crates/tokscale-core/Cargo.toml",
];

/// The only two places allowed to construct a reqwest client directly. Every
/// other call site must go through `tokscale_core::http`, which pins rustls.
/// `clippy.toml` enforces the same rule at lint time; this test also fails a
/// plain `cargo test`, and unlike clippy it does not depend on the lint being
/// run with the right flags.
const CLIENT_CONSTRUCTION_ALLOWLIST: [&str; 3] = [
    "crates/tokscale-core/src/http.rs",
    "crates/tokscale-cli/src/cursor.rs",
    // This file spells the forbidden constructors out as string literals in
    // order to search for them, so it matches its own scan.
    "crates/tokscale-cli/tests/tls_policy.rs",
];

/// Every directory the client-construction scan walks.
///
/// `crates/*/tests` is in here because CI runs clippy without `--all-targets`
/// (`.github/workflows/test_coverage.yml`), so integration tests are linted by
/// nothing. A raw client planted in a test would otherwise be caught by neither
/// this scan nor `clippy::disallowed_methods`.
const SCANNED_DIRS: [&str; 4] = [
    "crates/tokscale-cli/src",
    "crates/tokscale-core/src",
    "crates/tokscale-cli/tests",
    "crates/tokscale-core/tests",
];

/// The five constructors that yield a client on reqwest's *default* backend.
///
/// `Client::new`, `Client::builder` and `ClientBuilder::new` are the obvious
/// three -- and the scan only ever looked for the first two, because
/// `"ClientBuilder::new("` does not contain `"Client::new("` as a substring.
/// `Default` is the pair that both lanes missed: reqwest implements it for
/// both `Client` (`async_impl/client.rs`) and `ClientBuilder`, and both
/// impls delegate to
/// `new()`, so they select the same native-tls backend by exactly the same
/// mechanism.
///
/// This scan covers all five. `clippy.toml` covers only the first three --
/// see [`LINTABLE_CONSTRUCTORS`].
const DEFAULT_BACKEND_CONSTRUCTORS: [&str; 5] = [
    "Client::new(",
    "Client::builder(",
    "ClientBuilder::new(",
    "Client::default(",
    "ClientBuilder::default(",
];

/// The subset of [`DEFAULT_BACKEND_CONSTRUCTORS`] that `clippy.toml` can
/// actually enforce.
///
/// `disallowed-methods` resolves a path to a function, and only these three
/// are inherent associated functions on reqwest's types. `default` is a
/// *trait-impl* item, so `reqwest::Client::default` does not resolve: clippy
/// reports `does not refer to a reachable function` and skips the entry, which
/// means listing it buys a warning on every build and no enforcement at all.
/// Writing it qualified as `<reqwest::Client as Default>::default` is worse --
/// clippy 0.1.98 accepts that form without resolving it, so it warns about
/// nothing and still never fires.
///
/// Hence the asymmetry this file has to encode: the two `Default` doors are
/// closed by the scan below and by nothing else.
const LINTABLE_CONSTRUCTORS: [&str; 3] =
    ["Client::new(", "Client::builder(", "ClientBuilder::new("];

/// True when `needle` occurs in `line` as a whole path segment rather than as
/// the tail of a longer identifier.
///
/// Without this, the bare substring `Client::new(` also matches
/// `FooClient::new(`, and an unrelated type would trip the scan with a
/// confusing TLS-policy failure.
fn contains_standalone(line: &str, needle: &str) -> bool {
    let mut rest = line;
    while let Some(at) = rest.find(needle) {
        let preceding = rest[..at].chars().next_back();
        if !preceding.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            return true;
        }
        rest = &rest[at + needle.len()..];
    }
    false
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root must exist two levels above the CLI crate")
}

/// Read a workspace file with line endings normalised to `\n`.
///
/// GitHub's `windows-latest` runner ships Git for Windows with
/// `core.autocrlf=true`, and this repo carries no `.gitattributes`, so the
/// checkout on the Windows test leg has CRLF endings. Every assertion below
/// that spans a line break would then fail on Windows only — a red hard-gated
/// job that says nothing about the property being guarded.
fn read(relative: &str) -> String {
    let path = workspace_root().join(relative);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    normalize_newlines(raw)
}

fn normalize_newlines(source: String) -> String {
    source.replace("\r\n", "\n")
}

/// Without the normalisation in `read`, every assertion here that spans a line
/// break passes on Linux and macOS and fails on Windows — on a job with no
/// `continue-on-error`, for a reason that has nothing to do with TLS.
#[test]
fn source_is_read_with_line_endings_normalised() {
    let windows_checkout = "#[cfg(not(target_os = \"android\"))]\r\n    let builder = \
                            builder.use_native_tls();"
        .to_string();
    let expected = "#[cfg(not(target_os = \"android\"))]\n    let builder = \
                    builder.use_native_tls();";

    assert!(
        !windows_checkout.contains(expected),
        "this test is pointless unless a CRLF checkout really does defeat the \
         substring match it guards"
    );
    assert!(
        normalize_newlines(windows_checkout).contains(expected),
        "reads must normalise CRLF so the Windows leg checks the same property \
         as the Linux one"
    );
}

fn manifest(relative: &str) -> toml::Value {
    read(relative)
        .parse::<toml::Value>()
        .unwrap_or_else(|error| panic!("{relative} is not valid TOML: {error}"))
}

/// Every `features = [..]` entry for `reqwest` under a `[target.'..']` table,
/// paired with the cfg it is gated on.
fn per_target_reqwest_features(relative: &str) -> Vec<(String, Vec<String>)> {
    let Some(targets) = manifest(relative).get("target").cloned() else {
        return Vec::new();
    };
    let table = targets
        .as_table()
        .unwrap_or_else(|| panic!("{relative}: [target] must be a table"))
        .clone();

    table
        .into_iter()
        .filter_map(|(cfg, entry)| {
            let features = entry
                .get("dependencies")?
                .get("reqwest")?
                .get("features")?
                .as_array()?
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect();
            Some((cfg, features))
        })
        .collect()
}

fn workspace_reqwest_features() -> Vec<String> {
    manifest("Cargo.toml")["workspace"]["dependencies"]["reqwest"]["features"]
        .as_array()
        .expect("the workspace reqwest entry must list features")
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

/// The regression that shipped in the first #1250 attempt. `native-tls` on the
/// shared entry is unified into every crate and every target, so it is not an
/// opt-in for one client — it is a workspace-wide backend swap.
#[test]
fn the_workspace_reqwest_entry_stays_rustls_only() {
    let features = workspace_reqwest_features();

    for forbidden in [
        "native-tls",
        "native-tls-vendored",
        "native-tls-alpn",
        "default-tls",
    ] {
        assert!(
            !features.iter().any(|feature| feature == forbidden),
            "the workspace `reqwest` entry must not enable `{forbidden}`: it implies \
             `default-tls`, which makes native-tls the default backend for every client \
             on every target. Opt in per target in the member crates instead. Got: {features:?}"
        );
    }

    assert!(
        features
            .iter()
            .any(|feature| feature == "rustls-tls-native-roots"),
        "rustls must stay available workspace-wide, and with OS roots rather than the \
         baked-in webpki set. Got: {features:?}"
    );
}

/// Both member crates need their own blocks: `use_native_tls()` only exists
/// when the feature is on for the crate being compiled, so relying on feature
/// unification from the binary would break a standalone `tokscale-core` build.
#[test]
fn both_member_crates_gate_native_tls_on_the_same_non_android_cfg() {
    for relative in MEMBER_MANIFESTS {
        let features = per_target_reqwest_features(relative);
        let gated: Vec<_> = features
            .iter()
            .filter(|(_, features)| features.iter().any(|feature| feature == "native-tls"))
            .collect();

        assert_eq!(
            gated.len(),
            1,
            "{relative} must enable reqwest's `native-tls` under exactly one target cfg, \
             found {gated:?}"
        );
        assert_eq!(
            gated[0].0, NATIVE_TLS_TARGET,
            "{relative} must gate `native-tls` on {NATIVE_TLS_TARGET}"
        );
    }
}

/// The second regression: gating vendored OpenSSL on `target_env = "musl"`
/// left x86_64/aarch64-unknown-linux-gnu hunting for a system libssl that the
/// cargo-zigbuild cross environment cannot provide, and both release builds
/// failed in openssl-sys' `build/main.rs` before this test existed.
#[test]
fn vendored_openssl_covers_every_linux_target_and_only_linux() {
    for relative in MEMBER_MANIFESTS {
        let features = per_target_reqwest_features(relative);
        let gated: Vec<_> = features
            .iter()
            .filter(|(_, features)| {
                features
                    .iter()
                    .any(|feature| feature == "native-tls-vendored")
            })
            .collect();

        assert_eq!(
            gated.len(),
            1,
            "{relative} must vendor OpenSSL under exactly one target cfg, found {gated:?}"
        );
        assert_eq!(
            gated[0].0, VENDORED_TARGET,
            "{relative} must vendor OpenSSL for all of Linux, not a single libc: -gnu and \
             -musl both cross-compile under cargo-zigbuild with no system OpenSSL available"
        );
    }
}

/// The manifest cfg and the code cfg have to agree. If the manifest excludes
/// Android but the code does not, Android fails to compile; if the code
/// excludes Android but the manifest does not, Android silently regains
/// no-stdio OpenSSL and an empty trust store.
#[test]
fn the_cursor_call_site_repeats_the_manifest_cfg_verbatim() {
    let predicate = NATIVE_TLS_TARGET
        .strip_prefix("cfg(")
        .and_then(|rest| rest.strip_suffix(')'))
        .expect("NATIVE_TLS_TARGET is a cfg(..) expression");

    let cursor = read("crates/tokscale-cli/src/cursor.rs");
    let expected = format!("#[cfg({predicate})]\n    let builder = builder.use_native_tls();");

    assert!(
        cursor.contains(&expected),
        "cursor.rs must gate `.use_native_tls()` on the same cfg its manifest uses. \
         Expected to find:\n{expected}"
    );
    let call_sites: Vec<_> = cursor
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//") && trimmed.contains(".use_native_tls()")
        })
        .collect();
    assert_eq!(
        call_sites.len(),
        1,
        "only one call site may switch TLS backends, found: {call_sites:?}"
    );
}

/// `clippy.toml` and the scan above enforce the same rule by two different
/// mechanisms, and each covers a gap the other has: clippy resolves real paths
/// (so it is not fooled by a rename) but CI runs it without `--all-targets`,
/// while the scan is a plain text search that runs under `cargo test` on both
/// CI platforms.
///
/// They agree on the three constructors clippy can resolve, and this test
/// pins that agreement in both directions -- including the direction that is
/// easy to get wrong. `clippy.toml` used to list all five, and the two
/// `Default` entries were dead: clippy printed `does not refer to a reachable
/// function` twice per build and skipped them, so the parity this test
/// asserted was textual only. Re-adding either one restores that, which is
/// why the second half asserts their *absence*.
#[test]
fn the_lint_config_and_the_source_scan_forbid_the_same_constructors() {
    let clippy = read("clippy.toml");

    for needle in LINTABLE_CONSTRUCTORS {
        assert!(
            DEFAULT_BACKEND_CONSTRUCTORS.contains(&needle),
            "`{needle}` is listed as lintable but the source scan does not \
             look for it; the lint config must never be the only lane"
        );
        let method = needle
            .strip_suffix('(')
            .expect("every scanned constructor ends in an open paren");
        let path = format!("reqwest::{method}");
        assert!(
            clippy.contains(&format!("path = \"{path}\"")),
            "clippy.toml must also disallow `{path}`; the source scan already \
             rejects it, and a rule only one of the two enforces is a rule that \
             silently lapses whenever the other is skipped"
        );
    }

    let disallowed = clippy.matches("path = \"reqwest::").count();
    assert_eq!(
        disallowed,
        LINTABLE_CONSTRUCTORS.len(),
        "clippy.toml disallows {disallowed} reqwest constructors but only {} \
         resolve to a function clippy can match; an entry clippy skips is \
         enforcement the config only appears to have",
        LINTABLE_CONSTRUCTORS.len()
    );

    for needle in DEFAULT_BACKEND_CONSTRUCTORS {
        if LINTABLE_CONSTRUCTORS.contains(&needle) {
            continue;
        }
        let method = needle
            .strip_suffix('(')
            .expect("every scanned constructor ends in an open paren");
        let path = format!("reqwest::{method}");
        assert!(
            !clippy.contains(&format!("path = \"{path}\"")),
            "clippy.toml lists `{path}`, but `default` is a trait-impl item \
             that `disallowed-methods` cannot resolve: clippy warns `does not \
             refer to a reachable function` and skips it. The scan above is \
             what closes that door -- leave it there"
        );
    }
}

/// The scan matches raw text, so it has to distinguish `Client::new(` from the
/// tail of some unrelated `FooClient::new(`. Nothing in the tree trips this
/// today; the point is that adding such a type later must not produce a
/// baffling TLS-policy failure.
#[test]
fn the_scan_matches_whole_segments_rather_than_identifier_tails() {
    assert!(contains_standalone(
        "let c = reqwest::Client::new();",
        "Client::new("
    ));
    assert!(contains_standalone("Client::new()", "Client::new("));
    assert!(!contains_standalone(
        "let c = FooClient::new();",
        "Client::new("
    ));
    assert!(!contains_standalone(
        "let c = my_client::new();",
        "Client::new("
    ));
    // A tail match early in the line must not hide a real one later.
    assert!(contains_standalone(
        "FooClient::new(); reqwest::Client::new();",
        "Client::new("
    ));
}

/// Once `default-tls` is in the graph, an unqualified `reqwest::Client::new()`
/// is a native-tls client. Anything that is not Cursor must therefore say
/// rustls out loud.
#[test]
fn no_client_is_constructed_outside_the_allowlist() {
    let root = workspace_root();
    let mut offenders = Vec::new();

    for crate_dir in SCANNED_DIRS {
        let dir = root.join(crate_dir);
        if !dir.exists() {
            continue;
        }
        let mut stack = vec![dir];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("source directory must be readable") {
                let path = entry.expect("directory entry must be readable").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "rs") {
                    continue;
                }
                let relative = path
                    .strip_prefix(&root)
                    .expect("scanned paths live under the workspace root")
                    .to_string_lossy()
                    .replace('\\', "/");
                if CLIENT_CONSTRUCTION_ALLOWLIST.contains(&relative.as_str()) {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("source must be readable");
                for (number, line) in source.lines().enumerate() {
                    if DEFAULT_BACKEND_CONSTRUCTORS
                        .iter()
                        .any(|needle| contains_standalone(line, needle))
                    {
                        offenders.push(format!("{relative}:{}: {}", number + 1, line.trim()));
                    }
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "construct HTTP clients through `tokscale_core::http` so the TLS backend is \
         explicit (#1250). Offending lines:\n{}",
        offenders.join("\n")
    );
}
