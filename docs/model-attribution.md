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

1. A response-reported served model for the same request.
2. A confirmed gateway selection, admissible only when the adapter's own contract allows it and no response-reported value contradicts it.
3. A stable request/response identifier held by both the client record and the gateway record. This proves the two describe one call; it does not by itself name a model.
4. A native session id plus a verified token signature, or the requested selector plus a verified token signature and timing.

A requested or configured model is a fallback for display, never proof of the model behind a selector.

## Correlation invariants

- Every gateway record attributes at most one client row.
- Every client row consumes at most one record from each gateway representation of a call.
- Multiple matching candidates do not resolve by sorting, adapter order, or recency.
- Failed retries and handed-on attempts are not the request's outcome; only the completed successful attempt is evidence.
- Token signatures are normalized per source before comparison: Tokscale buckets are additive, gateway counters may already include cache traffic and reasoning, and a source that does not split reasoning compares on totals alone. No bucket is counted twice.
- A row that aggregates more than one call (`message_count != 1`) is never re-identified by a single gateway record.
- Incomplete streaming responses and estimated usage are not authoritative matches.

## Conflict policy

Stronger evidence can replace weaker evidence, and independent sources that agree reinforce one attribution. Contradictory served models across adapters stay unresolved: the row keeps its source-reported identity and is marked `modelAttributionConflicted`, which keeps it out of estimated pricing.

Agreement is checked on the model identity only. Two gateways may spell the same upstream provider differently, so the provider spelling is not a disagreement; the adopter nearest the provider supplies it.

## Application and pricing

Attribution is an overlay, not an accounting event. It changes `model_id`, `provider_id` and `service_tier` on an existing row and then re-derives its estimate. Token buckets, timestamps, dedup keys, message counts and session/parent linkage are never touched, so re-attribution cannot create, merge or split counted requests.

A provider-reported amount, including zero, survives attribution untouched. Every other row is repriced from scratch: a stale estimate computed for a router label is discarded rather than carried onto a model it never priced. When no pricing evidence exists the row is explicitly unpriced, which is distinct from an explicit zero-rate rule.

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

`allow_selected_fallback` is the one per-adapter policy that is not a field: OpenCodex requires a response-reported model, so a row whose `servedModel` is empty stays unclaimed. Magpie accepts `model` as a confirmed selection because its ledger only records a call after the chosen provider answered.

Adding a gateway means decoding its records into the shared evidence shape and registering the adapter; correlation, precedence, application, and diagnostics are shared.

## Local diagnostics

`tokscale pricing explain --json --no-spinner` reports, per row, the attribution outcome (`resolved`, `unresolved`, `conflict`), the requested model and provider, the adapter and evidence strength that established the served model, and the served identity. The attribution block is local-only: it is not added to cached messages or to submission payloads.

## Reserved and unimplemented surfaces

What the code does not claim, deliberately:

- `upstream` (an aggregator naming the provider behind itself, e.g. OpenRouter naming DeepInfra) is ignored. It names a party the ledger observed, not a model, and Tokscale has no provider identity to attach it to.
- Ledger fields that would only describe a call (effort, endpoint, stop reason, error type, credits, key fingerprints, account identity) are not read. Attribution needs the model, the provider, the tier the response reported, and the correlation evidence; the rest would be local telemetry this feature does not expose.
- Identifier matching accepts both the response id and the vendor request id, because either is a stable string a client parser can repeat in its dedup key. It proves the two records describe one call; it never names a model on its own.
- Cost splitting across models inside one aggregate is not implemented. A row with `message_count != 1` is never attributed.

## Limits

- A portable Magpie install keeps its ledger beside the executable, outside the home directory a scan reads; those rows stay unattributed rather than guessed.
- Current gateway configuration is not evidence of historical routing.
- Rows aggregated from several calls with only combined counters stay unresolved; no distribution is invented.
- A gateway whose ledger predates a field (for example, a row without token totals) can still match on a stable identifier, but never on tokens.
