/*
For licensing see accompanying LICENSE file.
Copyright (C) 2026 Apple Inc. All Rights Reserved.
*/

// swift-tools-version: 6.2
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
  name: "foundation-models-c-bindings",
  platforms: [.macOS(.v26), .iOS(.v26), .visionOS(.v26)],
  products: [
    .library(name: "FoundationModels", type: .dynamic, targets: ["FoundationModelsCBindings"]),
    .library(name: "FoundationModelsStatic", type: .static, targets: ["FoundationModelsCBindings"]),
    .executable(
      name: "fm-c-example",
      targets: ["fm-c-example"]
    )
  ],
  targets: [
    // A placeholder target that exposes the declarations from the bindings header to the bindings library itself.
    .target(
      name: "FoundationModelsCDeclarations"
    ),
    // The main target.
    .target(
      name: "FoundationModelsCBindings",
      dependencies: ["FoundationModelsCDeclarations"],
      publicHeadersPath: "include",
      cSettings: [
        .headerSearchPath("Sources/FoundationModelsCBindings/include")
      ],
    ),
    .executableTarget(
      name: "fm-c-example",
      dependencies: [
        .byName(name: "FoundationModelsCBindings")
      ],
      cSettings: [
        .headerSearchPath("../FoundationModelsCBindings/include")
      ]
    ),
    .testTarget(
      name: "FoundationModelsCBindingsTests",
      dependencies: ["FoundationModelsCBindings"]
    )
  ],
  cLanguageStandard: .c99
)
