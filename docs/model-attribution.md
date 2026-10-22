# Evidence-based model attribution

A client records what it asked a model to be, a gateway records what it contacted, and the upstream response records what actually served the request. Those are three different identities. Tokscale keeps them separate and only replaces the first with the third when a source actually observed it.

## Identities

| Identity | Meaning | Example |
|---|---|---|
| Requested model | What the client asked for | `group/auto` |
| Selected model | What the gateway chose to contact | `provider-a/model-x` |
| Served model | What the completed response reported serving | `model-x-202610` |
| Attribution provider | Provider of the served model | `provider-a` |
| Billing identity | Independently established billing channel | `subscription-a` |

The public model identity is the served model when it is established. A router label is never presented as a physical model. Billing identity is never derived from the attribution provider: a served model does not name the channel that was billed.

## Evidence precedence

Two things are ordered here, and they are independent.

What a source proves about the model:

1. A native response-reported model. A parser that read the provider's own response field (`responseModel`, `responseModelId`, Antigravity `#19`, OpenCodex `servedModel`, Magpie `served`) records `model_evidence = ResponseReported` on the row. A gateway observation may confirm that identity, but it may never replace it, and a gateway that names a different model contradicts it and fails closed.
2. A confirmed gateway selection, admissible only when the adapter's own contract allows it and no response-reported value exists.
3. A requested or configured model is a fallback for display, never proof of the model behind a selector.

How a gateway record is correlated with a client row (strongest rule only; a weaker rule never widens a stronger one):

1. A stable request/response identifier held by both sides. This proves the two describe one call; it does not by itself name a model.
2. A native session id plus a verified token signature.
3. The requested selector plus a verified token signature and timing.

Nothing in a ledger proves that one adapter is upstream of another, so no adapter name carries a fixed gateway chain position. Registration order is used only to keep two readings of one ledger from counting as two independent observations; it never chooses a winner.

## Correlation invariants

- Every gateway record attributes at most one client row.
- Every client row consumes at most one record from each gateway representation of a call.
- Multiple matching candidates do not resolve by sorting, adapter order, or recency.
- Failed retries and handed-on attempts are not the request's outcome; only the completed successful attempt is evidence.
- Token signatures are normalized per source before comparison: Tokscale buckets are additive, gateway counters may already include cache traffic and reasoning, and a source that does not split reasoning compares on totals alone. No bucket is counted twice.
- A row that aggregates more than one call (`message_count != 1`) is never re-identified by a single gateway record.
- Incomplete streaming responses and estimated usage are not authoritative matches.

## Candidate competition

Every parked row is correlated against every loaded ledger before any row changes.

- A record claimed by more than one row is removed from *every* row's feasible set. It is not awarded to the first row, and its removal never promotes a sibling candidate: a row that started with two feasible records from one adapter is ambiguous and stays unresolved even if only one survived the contest.
- A row whose only candidate was contested has no observation at all.
- One adapter must contribute at most one record; two records from the same ledger cannot agree with each other.
- Nothing here depends on lane order or adapter registration order.

## Conflict policy

Stronger evidence can replace weaker evidence, and independent sources that agree reinforce one attribution. Any disagreement about the model, the provider, or an authoritative service tier across adapters is a conflict: the row keeps its source-reported identity and is marked `modelAttributionConflicted`, which keeps it out of estimated pricing and drops any estimate that priced the identity now in doubt.

Provider spelling is part of the identity because there is no verifiable chain relation between adapters. Two gateways may in reality name the same upstream differently, but choosing between them without evidence would be a guess, so the disagreement fails closed. The winning observation supplies the provider and model, and its strength is never combined with another observation's identity.

The service tier is a separate authoritative claim about the same call. Every observation that reports one must agree: a source that stayed silent does not contradict a source that reported a tier, so an agreed tier is kept even when only one adapter reported it, but two different reported tiers describe two different tariffs for one request. That is a conflict as well: the row keeps its source-reported identity and stays out of estimated pricing instead of picking a rate by adapter name or registration order.

A parser-reported conflict (for example contradicting child attribution evidence inside one transcript) is also final: gateway evidence never overrides it.

## Application and pricing

Attribution is an overlay, not an accounting event. It changes `model_id`, `provider_id` and `service_tier` on an existing row and then re-derives its estimate. Token buckets, timestamps, dedup keys, message counts and session/parent linkage are never touched, so re-attribution cannot create, merge or split counted requests.

A provider-reported amount, including zero, survives attribution untouched. Every other row is repriced from scratch: a stale estimate computed for a router label is discarded rather than carried onto a model it never priced. The captured pricing origin follows the same rule — it is re-recorded for the new identity, and it is dropped entirely when the identity changed but no new estimate could be produced, so a row is never labeled with a price source that describes the model it used to be. When the resolved identity replaces the model, the row's existing service tier is kept only if the new evidence reported one: a tier recorded for the requested identity is not evidence for the served one, so an unconfirmed old tier is cleared instead of applying its multiplier to a model it never priced. When no pricing evidence exists the row is explicitly unpriced, which is distinct from an explicit zero-rate rule.

Because attribution runs inside the parse flush, the report graph, JSON output, submission pricing and `tokscale pricing explain` all read the same already-corrected row. Cold and warm cache scans produce the same identity and price; the evidence tier itself is cached, so a reparse is only needed when a parser starts reading a field it did not read before.

## Supported adapters

| Adapter | Ledger | Selector | Selected | Served | Provider | Tier | Stable ids | Sessions | Tokens | Timing |
|---|---|---|---|---|---|---|---|---|---|---|
| OpenCodex | `~/.opencodex/usage.jsonl` (`OPENCODEX_HOME` override) | `requestedModel`, required | not read | `servedModel`, only from the single eligible attempt | eligible attempt's `provider` | `tierOutcome` when `responseTierAuthoritative` | none | none | the eligible attempt's reported usage | row start + row duration |
| Magpie | `~/.config/magpie/usage.jsonl` (XDG-aware) | `req` | `model` | `served`, unless it echoes the selector | `provider` | `tier_served` | `response_id`, `rid` | `session`, `native_session` | `in`, `out`, `cache_read`, `cache_write`, `reasoning` | row `t` + `ms` |

Neither adapter reads a field it cannot justify. Every row must also be a completed 2xx call:

| Adapter | Rejected rows |
|---|---|
| OpenCodex | not `combo`; not `responses`; no requested selector; no start; no duration; an attempt that is unreported, estimated, locally answered, stream aborted, or missing ordinal/provider/model/adapter/status/send count |
| Magpie | locally refused; `handed_on` (a retried failure, not the outcome); `source: reported` (a client backfill the gateway never served); a `computer` id (synced from another machine); no provider; no parseable time |

`allow_selected_fallback` is the one per-adapter policy that is not a field: OpenCodex requires a response-reported model, so a row whose `servedModel` is empty stays unclaimed. Magpie accepts `model` as a confirmed selection because its ledger only records a call after the chosen provider answered — but only when that value is a physical model: an echo of the requested selector, a routing label (`auto`, `agent_review`) or a `<router>/auto` spelling stays unclaimed rather than being promoted to a confirmed identity.

Adding a gateway means decoding its records into the shared evidence shape and registering the adapter; correlation, precedence, application, and diagnostics are shared.

## Local diagnostics

`tokscale pricing explain --json --no-spinner` reports, per row, the attribution outcome (`resolved`, `unresolved`, `conflict`), the requested model and provider, the adapter and evidence strength that established the served model, and the served identity. The attribution block is local-only: it is not added to cached messages or to submission payloads.

The decision table is owned by one scan and returned beside the rows it describes. There is no process-global table, so two concurrent scans cannot clear or read each other's evidence, and nothing persists after the scan.

## Reserved and unimplemented surfaces

What the code does not claim, deliberately:

- `upstream` (an aggregator naming the provider behind itself, e.g. OpenRouter naming DeepInfra) is ignored. It names a party the ledger observed, not a model, and Tokscale has no provider identity to attach it to.
- Ledger fields that would only describe a call (effort, endpoint, stop reason, error type, credits, key fingerprints, account identity) are not read. Attribution needs the model, the provider, the tier the response reported, and the correlation evidence; the rest would be local telemetry this feature does not expose.
- Identifier matching accepts both the response id and the vendor request id, because either is a stable string a client parser can repeat in its dedup key. It proves the two records describe one call; it never names a model on its own.
- Cost splitting across models inside one aggregate is not implemented. A row with `message_count != 1` is never attributed.

## Bounds

- A ledger is truncated to its newest records past the cap. Trimming is batched rather than performed after every appended row, so the cost stays linear in the number of lines and memory stays bounded by the cap plus one batch.
- The parked-message set is capped. Reaching the cap attributes nothing at all for that scan instead of attributing an arbitrary prefix; that is a resource limit, not evidence.
- The decision table is bounded; past its cap, newer decisions are simply not recorded. Diagnostics are best-effort.

## Limits

- A portable Magpie install keeps its ledger beside the executable, outside the home directory a scan reads; those rows stay unattributed rather than guessed.
- Current gateway configuration is not evidence of historical routing.
- Rows aggregated from several calls with only combined counters stay unresolved; no distribution is invented.
- A gateway whose ledger predates a field (for example, a row without token totals) can still match on a stable identifier, but never on tokens.
