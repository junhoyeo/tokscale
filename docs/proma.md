# Proma Agent usage

Use `tokscale --no-spinner --client proma` or select **Proma** in the TUI. Proma is included in default local scans and the submission client registry. This integration reads local Agent usage; it does not contact Proma or inspect channel credentials.

## Sources

All platforms use the home directory, including `%USERPROFILE%` on Windows:

- `~/.proma/sdk-config/sessions/**/*.jsonl`: native Pi sessions.
- `~/.proma/sdk-config/projects/**/*.jsonl`: legacy Claude SDK sessions.
- `~/.proma/agent-sessions/*.jsonl`: the SDK-shaped display journal, including standalone history.
- The same paths under `~/.proma-dev/` for development builds.

Additional exported roots can be configured with `scanner.extraScanPaths.proma`; preserve the `agent-sessions` or `sdk-config/sessions` / `sdk-config/projects` directory structure. `agent-sessions.json` is optional linking metadata. Paths stored inside that index are never opened; only independently discovered usage files are considered. Conversation chat exports, workspace files, configuration and credentials are excluded.

## Accounting

Pi records supply disjoint input, output, cache-read and cache-write buckets. Reasoning is already part of output and is not added again. Recorded finite nonnegative USD `usage.cost.total`, including zero, is authoritative. Missing cost remains eligible for Tokscale pricing. Native billed operations such as compaction are retained even when the UI journal has no corresponding assistant message.

Claude SDK assistant rows supply `message.usage` counters. Session/turn `result.usage` aggregates and user rows are excluded. Display timestamps use Proma's persisted `_createdAt`; legacy transcript timestamps are also supported. File modification time is only a fallback. Unknown models retain their tokens and remain subject to the ordinary unpriced-usage rules.

A native Pi session is authoritative over its display mirror when they share the SDK session ID. The optional index links forked display journals to native sessions through `sdkSessionId`, discovered `piSessionFile`, and `piEntryBindings`. An exact binding missing from the native file retains its display record, covering a partially persisted tail. Claude response IDs and Pi response/entry identities remove repeated stream snapshots and copied fork history. Equal token counts or text are never used as identity. Legacy Claude records copied into a Pi session are retained.

Raw per-file records are cached independently. Reconciliation reads current linking metadata on every scan, including warm scans. Adding or removing a native file, or changing just the index, therefore updates source precedence without a manual cache reset. If a native file disappears, remaining display usage becomes the fallback; its missing provider cost cannot be reconstructed from the mirror.

## Limits

This covers persisted Agent usage, not ordinary Chat mode, remote billing, or usage absent from every local store. If both the native session relationship and its index have been lost, separate anonymous records cannot safely be identified as copies. Such records are retained rather than merged by content. A native session without per-entry bindings is authoritative as a whole over its matching Pi display records; missing native tails cannot be recovered selectively without that linkage. Malformed records are skipped, and malformed or oversized indexes are ignored. The index read is limited to 16 MiB.

## Upstream contracts

Implementation verified against [Proma commit 86d9dfb](https://github.com/proma-ai/Proma/tree/86d9dfb511db4c1e3ffded3b6c3e312996f50cbf):

- [Configuration paths](https://github.com/proma-ai/Proma/blob/86d9dfb511db4c1e3ffded3b6c3e312996f50cbf/apps/electron/src/main/lib/config-paths.ts).
- [Session index, persistence and fork copies](https://github.com/proma-ai/Proma/blob/86d9dfb511db4c1e3ffded3b6c3e312996f50cbf/apps/electron/src/main/lib/agent-session-manager.ts).
- [Persisted display timestamps and final-message filtering](https://github.com/proma-ai/Proma/blob/86d9dfb511db4c1e3ffded3b6c3e312996f50cbf/apps/electron/src/main/lib/agent-orchestrator.ts).
- [Pi display conversion](https://github.com/proma-ai/Proma/blob/86d9dfb511db4c1e3ffded3b6c3e312996f50cbf/apps/electron/src/main/lib/adapters/pi-message-adapter.ts) and [exact entry bindings](https://github.com/proma-ai/Proma/blob/86d9dfb511db4c1e3ffded3b6c3e312996f50cbf/apps/electron/src/main/lib/adapters/pi-agent-adapter.ts).

The logo URL references the unmodified [upstream application icon](https://github.com/proma-ai/Proma/blob/86d9dfb511db4c1e3ffded3b6c3e312996f50cbf/apps/electron/resources/icon.png). Related issue: [#1370](https://github.com/junhoyeo/tokscale/issues/1370); its unrelated client requests are outside this integration.
