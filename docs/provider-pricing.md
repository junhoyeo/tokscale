# Billing-channel custom pricing

`custom-pricing.json` keeps its original `models` object for global rules. The optional top-level `provider_overrides` array contains `{ "provider_id": "vertex", "model_id": "claude-sonnet-4", "pricing": { "input_cost_per_million_tokens": 2, "output_cost_per_million_tokens": 10 } }` entries. See the [complete example](../examples/custom-pricing.json) and [repository schema](../schemas/custom-pricing.schema.json). The schema is a repository file; no hosted schema URL is promised.

## Identity and selection contract

Billing identity is a log-declared channel label, separate from the attribution provider used for grouping and upstream searches. Explicit record/session fields and verified hosted channels can establish it. A model's vendor does not. Missing, blank and `unknown` identities cannot match scoped rules. This distinguishes labels in logs, not verified merchants, accounts, API keys or endpoints. Two endpoints recorded under the same label cannot be separated.

Provider matching trims outer whitespace and lowercases ASCII characters, comparing the entire result. `vertex` is distinct from `anthropic`; `gateway-a` from `gateway_a`; `openrouter/google` from `google`. Numeric IDs are valid. No provider alias registry is used. Model keys retain the existing case-insensitive matching.

Custom precedence is fixed:

1. Exact raw model in the matching billing scope
2. Exact raw model in global `models`
3. Synthetic-normalized model in the matching billing scope
4. Synthetic-normalized model in global `models`

Only existing synthetic normalization is used in those four steps. As a backward-compatibility exception after all four miss, a model already normalized by its parser may reach an old global rule; this is marked `aliasApplied` and never broadens scoped rules. Otherwise the existing upstream provider-aware resolver runs. Unknown billing identity may use global or upstream estimates. A selected custom rule is a whole row: missing cache rates cannot come from a global rule or another billing scope.

## Rates, zero and current estimates

All custom prices are USD base rates, per token or per million tokens. Each dimension accepts one unit form, not both; values must be finite and nonnegative. New scoped rate objects reject null and unknown fields, while `notes` and `source` metadata are allowed and never fetched. Global legacy metadata stays compatible, but unknown global fields whose names look like rates (`cost`, `price`, `pricing`, or `rate`) are rejected so a misspelled rate cannot silently produce a partial estimate. All existing token buckets and threshold dimensions remain available.

Zero is an explicit valid rate, distinct from no matching rule. Existing complete-free pricing policy remains unchanged. A provider-reported amount, including zero, always wins over estimates. Estimated history is recalculated using the current process's configuration and current loaded price tables; restarting after edits applies new rates. Paid → free → paid changes replace old estimates. Removed/missing prices cannot leave stale money in current totals. Partial estimates are marked incomplete; stale/unpriced amounts contribute zero and are not submission-safe. Coverage means supported token/rate evidence, not a verified bill.

Custom base rates keep existing Zed hosted markup (×1.1) and OpenAI fast/priority policy (×2 when applicable), applied once from the selected row. A literal fast-model custom rule follows the existing literal-rate policy; service-tier evidence still applies. Local explanations report the effective multiplier and final estimate. Subscription charges, historical effective dates, tax, discounts and actual account bills are outside this feature.

## Commands and local diagnostics

```sh
tokscale pricing claude-sonnet-4 --billing-provider vertex --source custom --json --no-spinner
tokscale pricing list-overrides --json --no-spinner
tokscale pricing validate-overrides --strict --json --no-spinner
tokscale pricing explain --json --no-spinner
```

`--provider` still selects a pricing source (`custom`, `litellm`, `openrouter`, `models.dev`); it is not a billing channel. `--source` is the clearer equivalent. A lookup without `--billing-provider` cannot choose scoped rules, even if only one exists. Forced upstream sources bypass custom. Custom-only queries, validation and listing are offline.

`pricing explain` is a local per-message report using cached catalogs and local overrides without network fetches. It reports attribution provider, billing identity/evidence, raw/display model, price source/scope, matched model, normalization/alias evidence, base-rate basis, effective multiplier, final estimate, coverage and estimate status. Use `--billing-provider` to filter it. Incomplete normal report totals also produce a stderr warning; JSON stdout stays machine-readable. Private channel labels and explanation fields are not added to public submission payloads. Do not publish this local diagnostic report without reviewing it.

## Errors, history and downgrade behavior

Normal loading is best effort: invalid scoped entries are diagnosed and skipped. Every member of a duplicated normalized `(provider_id, model_id)` group is disabled, independent of array order. Global legacy duplicate keys retain last-valid-wins warnings. Valid rows remain usable, and skipped rows can fall back to global/upstream prices. `validate-overrides --strict` returns nonzero on any configuration error; ordinary warnings alone do not fail it. Diagnostics go to stderr and are also available in validation JSON. Configuration is a process snapshot, not hot-reloaded.

Cache wire migration preserves supported legacy messages, token splits, authoritative amounts (including zero), parent metadata and MiCode row provenance. Missing legacy billing identity stays Unknown. Existing sources are reparsed once to acquire explicit identity; cached Claude compacted history is retained and live duplicate metadata wins as one model/billing pair unless an authoritative amount requires the original authoritative pair. Equal-priority estimated copies with conflicting billing channels are conservatively Unknown. Within the current source roots, missing-source Claude cache entries survive only when they hold retained compacted history, including pre-provenance legacy entries whose retained subset cannot be identified. Ordinary deleted, uncompacted Claude sessions are pruned, as are missing-source cache entries for other clients, including legacy entries still awaiting billing-identity backfill. Failed or unstable identity backfills keep the original messages and coupled metadata and remain eligible for a later retry. Failed cache decoding/migration never replaces the original shard with an empty one. Pricing edits alone do not trigger source rescans.

Old configuration files need no rewrite. Older binaries ignore `provider_overrides`, so downgrading disables scoped prices and can change estimates. Do not rely on scoped pricing with an older version.

## Source-format limits

For Codex response-scoped usage, Tokscale can use a uniquely matched local Magpie response ID and agreeing token counters to identify the served model. Failed streams, remote or reported rows, duplicate IDs, router labels and conflicting OpenCodex claims cannot establish an identity. The gateway ledger does not add token usage, and its provider is not billing-channel evidence. Ordinary Codex token-count rows without a response ID are unchanged.

Provider-shaped model strings are not automatically billing evidence. The Roo task-log parser uses `modelInfo.providerId` only: [Roo's protocol classifier](https://github.com/RooCodeInc/Roo-Code/blob/b867ec9145750d0ae1ff7f02d35406e9bf2a0b16/packages/types/src/provider-settings.ts#L530-L547) collapses several channels into an API protocol family. Mux's persisted `byModel` prefix remains Unknown because [gateway normalization](https://github.com/coder/xum/blob/9484694582925d4e1e3687cfacdb18c3ef99908f/src/common/utils/providers/modelEntries.test.ts#L174-L224) removes the actual routing label. Reasonix and FX model-prefix-only records likewise remain Unknown; this avoids guessing from vendor or plugin namespaces. All can still use legacy global rules and upstream estimates. An explicit channel field can be supported later when the source format supplies one.

Jcode snapshot-wide `provider_key` is mutable when the user changes routes, so it is attribution only for historical snapshot messages. A journal append batch's own provider metadata can establish billing for newly observed message IDs in that batch. Missing or unknown metadata never inherits the previous batch's billing channel. Later corrections keep the first-observed billing identity, including Unknown for snapshot history and records that originally had no usable token counts.
