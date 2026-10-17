# Cost provenance API

The original graph structs and functions retain their Rust field layouts and signatures. Existing `DailyTotals { tokens, cost, messages }` literals and callers of `generate_graph`, `aggregate_by_date`, `calculate_summary`, and `DailyFold` continue to compile. `CostSource` retains its original layout. Parser-facing `UnifiedMessage` now has the additional public fields `billing_identity`, `pricing_model_id`, and `estimate_status`. Existing exhaustive struct literals must be updated to include these fields; callers should use its constructors instead.

Opt into provenance with `GraphResultV2`, the related `V2` aggregate types, and the corresponding `_v2` graph and aggregation functions. The CLI uses these APIs, so its JSON includes `costProvenance` beside the existing cost fields. `From` conversions between original and V2 aggregates preserve numeric values and metadata; converting to the original types discards provenance, and converting legacy data to V2 treats its source as unknown.

```rust
use tokscale_core::{generate_graph_v2, GraphResult, ReportOptions};

async fn report() -> Result<(), String> {
    let graph = generate_graph_v2(ReportOptions::default()).await?;
    let provenance = graph.summary.cost_provenance();
    let original: GraphResult = graph.into();
    Ok(())
}
```

For messages with externally known pricing origins, use `DailyFoldV2::add_with_estimate_source` or the parallel `aggregate_by_date_v2_with_estimate_sources` API, which accepts `(UnifiedMessage, Option<EstimateSource>)` pairs. `add` and `aggregate_by_date_v2` preserve `CostSource`, but estimated messages without separate pricing metadata have `estimateSource: "unknown"`. Graph generation carries the source from the original pricing application, including service-tier and hosted-provider multipliers, without repeating the cost calculation during aggregation. Retained estimates and imported exports without captured pricing metadata keep an unknown pricing origin.

Every aggregation level applies the same participation rule: token-bearing rows, positive-cost rows, and rows with explicit reported or estimated provenance participate. Empty unknown activity does not affect provenance, even when it spans separate days or years. An absent provenance field on a participating row counts as unknown; combining it with known provenance produces mixed. Provider-reported zero and covered estimated zero retain their explicit source.

The existing cost fields and `costIsComplete` keep their semantics. Provenance describes how included costs were formed and does not establish cost completeness.
