# Vendored: foundation-models-c

This directory vendors the `foundation-models-c` Swift package from Apple's
[python-apple-fm-sdk](https://github.com/apple/python-apple-fm-sdk) (Apache-2.0,
Copyright © 2026 Apple Inc.). See `LICENSE.md`. Per-file Apple copyright headers
are preserved unmodified.

It is built (only when the `apple-fm` Cargo feature is enabled on macOS) into a
`libFoundationModels` dynamic library exposing a C ABI over Apple's Swift-only
FoundationModels framework, which `tokscale` calls via Rust FFI.
