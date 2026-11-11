# Licensing of the Tokscale web app

The web app in this directory (`packages/frontend`) is licensed under the
**GNU Affero General Public License, version 3 only** (`AGPL-3.0-only`).
The full text is in [`LICENSE`](./LICENSE).

## What this covers

- All code in `packages/frontend`, including the submission API and the other server routes.
- It does not cover the Tokscale CLI and core library in `crates/`. They stay under the [MIT License](../../LICENSE).
- Data that Tokscale compiled in [`src/content`](./src/content) has its own license, CC BY 4.0. Third-party data there keeps the terms of its source. See [`src/content/LICENSE.md`](./src/content/LICENSE.md).
- Separately developed programs, such as a model router, billing, or team features, are intended to keep their own licenses. Whether an integration forms a combined or derivative work depends on how it uses this app, not only its folder or deployment boundary. This document does not guarantee AGPL isolation for any integration; review its actual architecture and obligations.

## Earlier MIT releases

Until the commit that added this file, this directory was published under the MIT License. Versions released before that commit stay available under MIT, and the MIT copyright and permission notice in [`../../LICENSE`](../../LICENSE) is kept for the code that came from them.

## Notices in build artifacts

Run `bun run build` from `packages/frontend` to build the app and package the notices. Distribute the whole `.next/standalone` directory, including its `notices/` directory. The root Dockerfile carries the same bundle at `/app/notices`.

The bundle includes this scope document, the full frontend AGPL license, the data notice, the models.dev MIT notice and its retrieval provenance, the inherited root MIT notice, and the linked trademark and contributor documents. Their repository-relative layout keeps local links intact. Start with `notices/README.md` in the standalone output or `/app/notices/README.md` in the image.

Running `next build` directly does not run the notice packager; use the package build command for a distributable standalone artifact. The notice bundle is not a source-code offer or a substitute for any source-distribution obligations. Keep dependency and asset notices with their respective files too.

## If you run a modified version

The AGPL lets you use, change, and run this web app, including for commercial purposes. If people interact with your modified version over a network, you must offer them its source code under the AGPL (section 13).

## Trademarks

The AGPL gives no rights to the Tokscale name or logo. See [`../../TRADEMARKS.md`](../../TRADEMARKS.md).

## Contributing

Contributions to this directory need a one-time signature of the [Contributor License Agreement](../../CLA.md).
