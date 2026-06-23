//! Build script for tokscale-cli.
//!
//! When (and only when) the optional `apple-fm` feature is enabled AND the
//! target OS is macOS, this builds the vendored `foundation-models-c` SwiftPM
//! package and links the resulting `libFoundationModels.dylib`.
//!
//! When the feature is off, or the target is not macOS, this build script is a
//! complete no-op so that cross-platform / default builds are unaffected.

use std::path::Path;
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

    // Build the SwiftPM package in release mode.
    let status = Command::new("swift")
        .args([
            "build",
            "-c",
            "release",
            "--product",
            "FoundationModelsStatic",
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

    // Copy the STATIC archive into OUT_DIR and link it statically, so the final
    // tokscale binary is self-contained — no `libFoundationModels.dylib` to ship
    // alongside it. The archive's only remaining dependencies are Apple's system
    // FoundationModels framework and the OS Swift runtime, both always present on
    // macOS 26 (verified with `otool -L`: no non-system dylib references).
    let lib_name = "libFoundationModelsStatic.a";
    let built_lib = pkg_dir.join(".build/release").join(lib_name);
    if !built_lib.exists() {
        panic!(
            "apple-fm: swift build succeeded but {} was not found",
            built_lib.display()
        );
    }
    let dest_lib = Path::new(&out_dir).join(lib_name);
    std::fs::copy(&built_lib, &dest_lib).unwrap_or_else(|e| {
        panic!(
            "apple-fm: failed to copy {} -> {}: {e}",
            built_lib.display(),
            dest_lib.display()
        )
    });

    // Statically link the bindings archive, plus the system FoundationModels
    // framework and the OS Swift runtime search path. The archive also carries
    // autolink hints, but these are made explicit for a deterministic link.
    println!("cargo:rustc-link-search=native={out_dir}");
    println!("cargo:rustc-link-lib=static=FoundationModelsStatic");
    println!("cargo:rustc-link-lib=framework=FoundationModels");
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    // The Swift runtime dylibs (e.g. libswift_Concurrency.dylib) are referenced
    // via `@rpath`. They live in /usr/lib/swift, which is part of every macOS 26
    // install's dyld shared cache, so baking this system rpath keeps the binary
    // self-contained (it needs only OS-provided libraries at runtime).
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
