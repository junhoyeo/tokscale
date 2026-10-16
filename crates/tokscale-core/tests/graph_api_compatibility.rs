//! Compile and wire-contract coverage for callers of the original graph API.
use std::collections::BTreeSet;
use tokscale_core::*;

/// Build the fixture through the public constructor rather than an exhaustive
/// struct literal.
///
/// `original_public_struct_literals_and_function_signatures_still_compile`
/// below is the test that deliberately pins the *aggregate* layouts, because
/// breaking those is the regression this file exists to catch. `UnifiedMessage`
/// is different: it is a parser-facing type that legitimately grows fields as
/// clients are added (`copilot_context` in #1434 is one), and an exhaustive
/// literal here only made every such addition fail this test for a reason
/// unrelated to graph API compatibility.
fn legacy_message() -> UnifiedMessage {
    let mut message = UnifiedMessage::new(
        "test",
        "model",
        "provider",
        "session",
        0,
        TokenBreakdown {
            input: 100,
            ..Default::default()
        },
        1.0,
    );
    // `new` derives the date from the timestamp; pin it so the assertions below
    // do not depend on the local timezone of the machine running the test.
    message.date = "2024-01-01".into();
    message.mark_provider_reported_cost();
    assert_eq!(message.cost_source, CostSource::ProviderReported);
    assert_eq!(message.message_count, 1);
    message
}

#[test]
fn original_public_struct_literals_and_function_signatures_still_compile() {
    let totals = DailyTotals {
        tokens: 100,
        cost: 1.0,
        messages: 1,
    };
    let client = ClientContribution {
        client: "test".into(),
        model_id: "model".into(),
        provider_id: "provider".into(),
        tokens: TokenBreakdown {
            input: 100,
            ..Default::default()
        },
        cost: 1.0,
        messages: 1,
    };
    let day = DailyContribution {
        date: "2024-01-01".into(),
        totals: totals.clone(),
        intensity: 4,
        token_breakdown: client.tokens.clone(),
        clients: vec![client.clone()],
        active_time_ms: None,
    };
    let session = SessionContribution {
        session_id: "session".into(),
        client: "test".into(),
        provider: "provider".into(),
        model: "model".into(),
        totals,
        token_breakdown: client.tokens.clone(),
        clients: vec![client],
        first_seen: 0,
        last_seen: 0,
    };
    let year = YearSummary {
        year: "2024".into(),
        total_tokens: 100,
        total_cost: 1.0,
        range_start: day.date.clone(),
        range_end: day.date.clone(),
    };
    let summary = DataSummary {
        total_tokens: 100,
        total_cost: 1.0,
        total_days: 1,
        active_days: 1,
        average_per_day: 1.0,
        max_cost_in_single_day: 1.0,
        clients: vec!["test".into()],
        models: vec!["model".into()],
    };
    let graph = GraphResult {
        meta: GraphMeta {
            generated_at: String::new(),
            version: String::new(),
            date_range_start: day.date.clone(),
            date_range_end: day.date.clone(),
            processing_time_ms: 0,
        },
        summary,
        years: vec![year],
        contributions: vec![day],
        time_metrics: None,
        unpriced_submission_usage: Vec::new(),
        incomplete_cost_dates: BTreeSet::new(),
    };
    let _: fn(Vec<UnifiedMessage>) -> Vec<DailyContribution> = aggregate_by_date;
    let _: fn(Vec<UnifiedMessage>) -> Vec<SessionContribution> = aggregate_by_session;
    let _: fn(&[DailyContribution]) -> DataSummary = calculate_summary;
    let _: fn(&[DailyContribution]) -> Vec<YearSummary> = calculate_years;
    let _: fn(Vec<DailyContribution>, u32) -> GraphResult = generate_graph_result;
    let _: fn(&mut [DailyContribution]) = calculate_intensities;
    drop(generate_graph(ReportOptions::default()));
    drop(generate_submission_graph(ReportOptions::default()));
    drop(generate_local_graph_report(ReportOptions::default()));
    drop(generate_source_graph_report(ReportOptions::default()));
    let mut fold = DailyFold::default();
    fold.add(&legacy_message());
    assert_eq!(fold.finish()[0].totals.cost, graph.summary.total_cost);
    assert_eq!(aggregate_by_session(vec![legacy_message()])[0], session);
}

fn strip_provenance(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.remove("costProvenance");
            for child in object.values_mut() {
                strip_provenance(child);
            }
        }
        serde_json::Value::Array(array) => {
            for child in array {
                strip_provenance(child);
            }
        }
        _ => {}
    }
}

#[test]
fn v2_graph_conversion_preserves_every_existing_json_field() {
    let graph = generate_graph_result_v2(aggregate_by_date_v2(vec![legacy_message()]), 123);
    let mut modern = serde_json::to_value(&graph).unwrap();
    assert_eq!(
        modern["summary"]["costProvenance"]["kind"],
        "providerReported"
    );
    assert_eq!(
        modern["years"][0]["costProvenance"]["kind"],
        "providerReported"
    );
    assert_eq!(
        modern["contributions"][0]["totals"]["costProvenance"]["kind"],
        "providerReported"
    );
    assert_eq!(
        modern["contributions"][0]["clients"][0]["costProvenance"]["kind"],
        "providerReported"
    );
    strip_provenance(&mut modern);
    let legacy: GraphResult = graph.into();
    assert_eq!(modern, serde_json::to_value(&legacy).unwrap());
    let roundtrip: GraphResultV2 = legacy.into();
    assert_eq!(
        roundtrip.summary.cost_provenance(),
        CostProvenance::unknown()
    );
    assert_eq!(
        roundtrip.contributions[0].totals.cost_provenance(),
        CostProvenance::unknown()
    );
}
