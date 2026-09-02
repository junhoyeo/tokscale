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
const CLIENT_CONSTRUCTION_ALLOWLIST: [&str; 2] = [
    "crates/tokscale-core/src/http.rs",
    "crates/tokscale-cli/src/cursor.rs",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root must exist two levels above the CLI crate")
}

fn read(relative: &str) -> String {
    let path = workspace_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
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

/// Once `default-tls` is in the graph, an unqualified `reqwest::Client::new()`
/// is a native-tls client. Anything that is not Cursor must therefore say
/// rustls out loud.
#[test]
fn no_client_is_constructed_outside_the_allowlist() {
    let root = workspace_root();
    let mut offenders = Vec::new();

    for crate_dir in ["crates/tokscale-cli/src", "crates/tokscale-core/src"] {
        let mut stack = vec![root.join(crate_dir)];
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
                    if line.contains("Client::new(") || line.contains("Client::builder(") {
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
