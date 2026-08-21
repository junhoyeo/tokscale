//! Build script for tokscale-cli.
//!
//! When (and only when) the optional `apple-fm` feature is enabled AND the
//! target OS is macOS, this builds the vendored `foundation-models-c` SwiftPM
//! package as a DYNAMIC `libFoundationModels.dylib` and stages it next to the
//! final binary.
//!
//! The dylib is deliberately NOT linked into `tokscale`. Apple's
//! `FoundationModels.framework` only exists on macOS 26+, and the Swift runtime
//! the dylib pulls in (e.g. `libswiftSynchronization`, macOS 15+) does too;
//! hard-linking any of them would make the *whole* CLI fail to `dyld`-load on
//! older macOS — a crash-on-launch for every command, not a feature fallback.
//! Worse, `import FoundationModels` autolinks the framework as a NON-weak load
//! command, so a `-weak_framework` flag can't reliably flip it.
//!
//! Instead the binary links nothing FM/Swift (verifiable: `otool -L tokscale`
//! shows no FoundationModels and no libswift*), and the `apple-fm` code path
//! `dlopen`s this dylib lazily at runtime — only on macOS 26+, where all its
//! dependencies are present. On older macOS the `dlopen` simply fails and the
//! caller degrades to the cross-platform Rust heuristic. This keeps a SINGLE
//! arm64 binary safe to ship to every Apple Silicon Mac via npm.
//!
//! When the feature is off, or the target is not macOS, this build script is a
//! complete no-op so that cross-platform / default builds are unaffected.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Re-run only when the feature flag toggles. (Cheap; keeps the no-op path no-op.)
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_APPLE_FM");

    let feature_enabled = std::env::var("CARGO_FEATURE_APPLE_FM").is_ok();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    // No-op unless the feature is enabled and we're building for macOS.
    if !feature_enabled || target_os != "macos" {
        return;
    }

    build_apple_fm();
}

fn build_apple_fm() {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set by cargo");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set by cargo");

    let pkg_dir = Path::new(&manifest_dir).join("vendor/foundation-models-c");
    if !pkg_dir.join("Package.swift").exists() {
        panic!(
            "apple-fm feature is enabled but the vendored SwiftPM package was not found at {}. \
             Expected Package.swift there.",
            pkg_dir.display()
        );
    }

    // Re-run if any vendored Swift source, the manifest, the header, or this
    // build script changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        pkg_dir.join("Package.swift").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        pkg_dir.join("Sources").display()
    );

    // Build the DYNAMIC `FoundationModels` product (`libFoundationModels.dylib`)
    // in release mode. We do not build/link the static archive: the dylib is
    // loaded at runtime via `dlopen`, so nothing FM/Swift ends up in the
    // tokscale binary's load commands.
    let status = Command::new("swift")
        .args([
            "build",
            "-c",
            "release",
            "--product",
            "FoundationModels",
            "--package-path",
        ])
        .arg(&pkg_dir)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "apple-fm feature is enabled but `swift build` could not be spawned: {e}. \
                 Is the Swift toolchain installed and on PATH?"
            )
        });

    if !status.success() {
        panic!(
            "apple-fm feature is enabled but `swift build -c release` failed in {} \
             (exit status: {status}). Fix the Swift build or disable the apple-fm feature.",
            pkg_dir.display()
        );
    }

    let lib_name = "libFoundationModels.dylib";
    let built_lib = pkg_dir.join(".build/release").join(lib_name);
    if !built_lib.exists() {
        panic!(
            "apple-fm: swift build succeeded but {} was not found",
            built_lib.display()
        );
    }

    // 1) Copy into OUT_DIR and bake its absolute path into the binary as a
    //    fallback. This is what `cargo test` / `cargo run` from arbitrary CWDs
    //    resolve to (the test harness binary lives in target/<profile>/deps, so
    //    a sibling-of-exe copy alone would not be found there).
    let out_lib = Path::new(&out_dir).join(lib_name);
    copy(&built_lib, &out_lib);
    println!("cargo:rustc-env=TOKSCALE_FM_DYLIB={}", out_lib.display());

    // 2) Stage a copy NEXT TO the final binary, so the primary runtime lookup
    //    (`current_exe()`'s directory) succeeds for both `cargo run` and the
    //    shipped npm package, where the dylib travels alongside `tokscale`.
    //
    //    OUT_DIR is `.../target/<triple?>/<profile>/build/<crate>-<hash>/out`;
    //    ascending three parents lands on the profile dir that holds the binary.
    if let Some(profile_dir) = profile_dir_from_out(&out_dir) {
        let staged = profile_dir.join(lib_name);
        copy(&built_lib, &staged);
        // CI's release step copies this sibling dylib into the npm package's
        // bin/ next to tokscale; surface its path for that step / debugging.
        println!("cargo:warning=apple-fm: staged {}", staged.display());
    }
}

/// `.../<profile>/build/<crate>-<hash>/out` -> `.../<profile>`.
fn profile_dir_from_out(out_dir: &str) -> Option<PathBuf> {
    Path::new(out_dir)
        .parent() // <crate>-<hash>
        .and_then(Path::parent) // build
        .and_then(Path::parent) // <profile>
        .map(Path::to_path_buf)
}

fn copy(from: &Path, to: &Path) {
    std::fs::copy(from, to).unwrap_or_else(|e| {
        panic!(
            "apple-fm: failed to copy {} -> {}: {e}",
            from.display(),
            to.display()
        )
    });
}
