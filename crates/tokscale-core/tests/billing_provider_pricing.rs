use serde_json::{json, Value};
use std::{collections::HashMap, path::Path};
use tokscale_core::{
    pricing::{custom::CustomPricing, PricingContext, PricingService},
    sessions::{BillingIdentity, BillingIdentityOrigin, UnifiedMessage},
    TokenBreakdown,
};

fn service(value: Value) -> PricingService {
    let custom = CustomPricing::load_from_str(&value.to_string(), Path::new("fixture.json"));
    assert!(
        custom.diagnostics().is_empty(),
        "{:?}",
        custom.diagnostics()
    );
    PricingService::new_with_custom(custom, HashMap::new(), HashMap::new())
}
fn rule(provider: &str, model: &str, rate: f64) -> Value {
    json!({"provider_id":provider,"model_id":model,"pricing":{"input_cost_per_million_tokens":rate,"output_cost_per_million_tokens":rate}})
}
fn usage() -> TokenBreakdown {
    TokenBreakdown {
        input: 1_000_000,
        ..Default::default()
    }
}

#[test]
fn billing_identity_is_opaque_and_unknown_never_scoped() {
    let svc = service(
        json!({"provider_overrides":[rule("vertex","atlas",1.),rule("anthropic","atlas",7.),rule("openrouter/google","atlas",3.),rule("123","atlas",4.),rule("gateway-a","atlas",5.),rule("gateway_a","atlas",6.)]}),
    );
    for (provider, cost) in [
        (Some(" VERTEX "), 1.),
        (Some("anthropic"), 7.),
        (Some("openrouter/google"), 3.),
        (Some("google"), 0.),
        (Some("123"), 4.),
        (Some("gateway-a"), 5.),
        (Some("gateway_a"), 6.),
        (None, 0.),
        (Some("unknown"), 0.),
        (Some(" "), 0.),
    ] {
        let context = PricingContext {
            raw_model: "atlas",
            model: "atlas",
            billing_provider: provider,
            upstream_hint: Some("anthropic"),
            service_tier: None,
        };
        let estimate = svc.estimate_context(&context, &usage());
        assert_eq!(estimate.cost, cost, "{provider:?}");
        assert_eq!(estimate.coverage, cost > 0.);
        assert_eq!(
            svc.lookup_context(&context, Some("custom")).is_some(),
            cost > 0.
        );
    }
}

#[test]
fn four_level_precedence_is_order_independent_and_whole_row() {
    let raw = "hf:zai-org/GLM-4.7";
    for iteration in 0..128 {
        let mut rules = vec![
            rule("gateway", "glm-4.7", 3.),
            rule("gateway", raw, 1.),
            rule("other", raw, 99.),
        ];
        rules.rotate_left(iteration % 3);
        if iteration % 2 == 0 {
            rules.reverse();
        }
        let mut value = json!({"models":{raw:{"input_cost_per_million_tokens":2.},"glm-4.7":{"input_cost_per_million_tokens":4.}},"provider_overrides":rules});
        for expected in 1..=4 {
            let svc = service(value.clone());
            let context = PricingContext::explicit(raw, Some("gateway"));
            let selected = svc.lookup_context(&context, Some("custom")).unwrap();
            assert_eq!(
                svc.estimate_context(&context, &usage()).cost,
                expected as f64
            );
            assert_eq!(selected.evidence.normalized, expected >= 3);
            assert_eq!(
                selected.matched_provider_id.as_deref(),
                if expected % 2 == 1 {
                    Some("gateway")
                } else {
                    None
                }
            );
            assert!(svc.covers_usage_context(&context, &usage()));
            match expected {
                1 => value["provider_overrides"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|r| r["provider_id"] != "gateway" || r["model_id"] != raw),
                2 => {
                    value["models"].as_object_mut().unwrap().remove(raw);
                }
                3 => value["provider_overrides"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|r| r["provider_id"] != "gateway"),
                _ => {}
            }
        }
    }
    let svc = service(
        json!({"models":{"atlas":{"input_cost_per_token":1.,"cache_read_input_token_cost":1.}},"provider_overrides":[rule("gateway","atlas",2.)]}),
    );
    let used = TokenBreakdown {
        input: 1,
        cache_read: 1,
        ..Default::default()
    };
    let context = PricingContext::explicit("atlas", Some("gateway"));
    let estimate = svc.estimate_context(&context, &used);
    assert_eq!(estimate.cost, 0.000002);
    assert!(!estimate.coverage);
    assert!(estimate
        .selected
        .unwrap()
        .pricing
        .cache_read_input_token_cost
        .is_none());
}

#[test]
fn duplicate_groups_are_disabled_even_if_one_member_is_invalid() {
    for reverse in [false, true] {
        let mut rules = vec![
            rule(" Vertex ", "ATLAS", 1.),
            json!({"provider_id":"vertex","model_id":"atlas","pricing":{"input_cost_per_token":-1.}}),
            rule("other", "atlas", 7.),
        ];
        if reverse {
            rules.reverse();
        }
        let custom = CustomPricing::load_from_str(
            &json!({"provider_overrides":rules}).to_string(),
            Path::new("fixture.json"),
        );
        assert!(custom
            .lookup_with_provider_key("atlas", Some("vertex"))
            .is_none());
        assert!(custom
            .lookup_with_provider_key("atlas", Some("other"))
            .is_some());
        assert!(custom
            .diagnostics()
            .iter()
            .any(|d| d.code == "duplicate_selector"));
    }
}

#[test]
fn invalid_new_selectors_and_rates_are_visible_without_poisoning_good_rows() {
    let custom=CustomPricing::load_from_str(&json!({"provider_overrides":[rule("unknown","atlas",1.),rule(" ","atlas",1.),json!({"provider_id":"p","model_id":"atlas","pricing":{"input_cost_per_tokne":1.}}),json!({"provider_id":"q","model_id":"atlas","pricing":{"input_cost_per_token":null}}),rule("valid","atlas",0.)]}).to_string(),Path::new("fixture.json"));
    assert_eq!(custom.diagnostics().len(), 4);
    assert_eq!(custom.len(), 1);
    assert_eq!(
        custom
            .lookup_with_provider_key("atlas", Some("valid"))
            .unwrap()
            .pricing
            .input_cost_per_token,
        Some(0.)
    );
}

#[test]
fn gateway_billing_does_not_change_fast_tier_or_borrow_another_scope() {
    let svc = service(
        json!({"provider_overrides":[rule("private-gateway","gpt-5.4-fast",2.),rule("openai","gpt-5.4-fast",99.)]}),
    );
    let mut context = PricingContext {
        raw_model: "gpt-5.4-fast",
        model: "gpt-5.4-fast",
        billing_provider: Some("private-gateway"),
        upstream_hint: Some("openai"),
        service_tier: None,
    };
    assert_eq!(svc.estimate_context(&context, &usage()).cost, 2.);
    context.service_tier = Some("priority");
    let estimate = svc.estimate_context(&context, &usage());
    assert_eq!(estimate.cost, 4.);
    assert_eq!(estimate.effective_multiplier, 2.);
    assert_eq!(
        estimate.selected.unwrap().matched_provider_id.as_deref(),
        Some("private-gateway")
    );
}

#[test]
fn parser_opencode_preserves_distinct_billing_before_attribution() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(
        json!({"provider_overrides":[rule("vertex","claude-sonnet-4",1.),rule("anthropic","claude-sonnet-4",7.)]}),
    );
    let mut total = 0.;
    for (id, provider, expected) in [
        ("v", Some("vertex"), 1.),
        ("a", Some("anthropic"), 7.),
        ("u", None, 0.),
    ] {
        let path = dir.path().join(format!("{id}.json"));
        std::fs::write(&path,json!({"id":id,"sessionID":"s","role":"assistant","modelID":"claude-sonnet-4","providerID":provider,"time":{"created":1780000000000u64},"tokens":{"input":1000000,"output":0,"cache":{"read":0,"write":0}}}).to_string()).unwrap();
        let message = tokscale_core::sessions::opencode::parse_opencode_file(&path).unwrap();
        assert_eq!(message.billing_identity.id(), provider);
        assert_eq!(
            svc.estimate_context(&message.pricing_context(), &message.tokens)
                .cost,
            expected
        );
        total += expected;
    }
    assert_eq!(total, 8.);
}

#[test]
fn constructor_does_not_promote_attribution_to_billing() {
    let message = UnifiedMessage::new(
        "claude",
        "claude-sonnet-4",
        "anthropic",
        "s",
        1,
        usage(),
        0.,
    );
    assert_eq!(message.billing_identity, BillingIdentity::Unknown);
    assert_eq!(
        BillingIdentity::from_explicit(Some(" Vertex "), BillingIdentityOrigin::ExplicitRecord)
            .id(),
        Some("vertex")
    );
}

#[test]
fn parser_alias_compatibility_only_falls_back_to_old_global_rules() {
    let svc = service(
        json!({"models":{"glm-4.7":{"input_cost_per_million_tokens":3.}},"provider_overrides":[rule("gateway","glm-4.7",99.)]}),
    );
    let context = PricingContext {
        raw_model: "big-pickle",
        model: "glm-4.7",
        billing_provider: Some("gateway"),
        upstream_hint: Some("zai"),
        service_tier: None,
    };
    let selected = svc.lookup_context(&context, Some("custom")).unwrap();
    assert_eq!(selected.matched_provider_id, None);
    assert!(selected.evidence.alias_applied);
    assert!(selected.evidence.normalized);
    assert_eq!(svc.estimate_context(&context, &usage()).cost, 3.);
}

#[test]
fn scoped_free_and_authoritative_zero_have_distinct_success_states() {
    let svc = service(
        json!({"provider_overrides":[rule("private","atlas",0.),rule("zed.dev","atlas",10.)]}),
    );
    let mut message = UnifiedMessage::new("test", "atlas", "openai", "s", 1, usage(), 7.25)
        .with_billing(Some("private"), BillingIdentityOrigin::ExplicitRecord);
    let explain = svc.explain_message(&message);
    assert_eq!(explain.final_estimate, 0.);
    assert!(explain.coverage);
    assert_eq!(
        explain.estimate_status,
        tokscale_core::sessions::EstimateStatus::CurrentEstimate
    );
    message.cost = 0.;
    message.mark_provider_reported_cost();
    message.billing_identity = BillingIdentity::Unknown;
    let explain = svc.explain_message(&message);
    assert_eq!(explain.final_estimate, 0.);
    assert!(explain.coverage);
    assert_eq!(
        explain.estimate_status,
        tokscale_core::sessions::EstimateStatus::Authoritative
    );
    assert_eq!(explain.price_source.as_deref(), Some("ProviderReported"));
    assert_eq!(explain.scope, "reported");
    assert_eq!(explain.effective_multiplier, 1.0);
    assert!(explain.matched_model_id.is_none());
    assert!(explain.matched_provider_id.is_none());
    let zed = UnifiedMessage::new("zed", "atlas", "zed.dev", "s", 1, usage(), 0.)
        .with_billing(Some("zed.dev"), BillingIdentityOrigin::KnownHostedChannel);
    let explain = svc.explain_message(&zed);
    assert_eq!(explain.effective_multiplier, 1.1);
    assert_eq!(explain.final_estimate, 11.);
}

#[test]
fn zed_hosted_multiplier_uses_attribution_not_custom_billing_selector() {
    let svc = service(
        json!({"models":{"atlas":{"input_cost_per_million_tokens":2.}},"provider_overrides":[rule("private","atlas",3.),rule("zed.dev","atlas",10.)]}),
    );
    for (attribution, billing, expected_rate, multiplier) in [
        ("zed.dev", Some("private"), 3., 1.1),
        ("zed.dev", None, 2., 1.1),
        ("openai", Some("zed.dev"), 10., 1.),
    ] {
        let message = UnifiedMessage::new("zed", "atlas", attribution, "s", 1, usage(), 0.)
            .with_billing(billing, BillingIdentityOrigin::ExplicitRecord);
        let explain = svc.explain_message(&message);
        assert_eq!(explain.effective_multiplier, multiplier);
        assert!((explain.final_estimate - expected_rate * multiplier).abs() < 1e-10);
        assert_eq!(svc.estimate_message(&message).cost, explain.final_estimate);
    }
}

#[test]
fn parser_sqlite_dedup_does_not_fold_different_billing_channels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("opencode.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, data TEXT NOT NULL);",
    )
    .unwrap();
    for (id, provider) in [("v", "vertex"), ("a", "anthropic")] {
        let data = json!({"id":id,"role":"assistant","modelID":"claude-sonnet-4","providerID":provider,"time":{"created":1780000000000u64},"tokens":{"input":1000000,"output":0,"cache":{"read":0,"write":0}}});
        db.execute(
            "INSERT INTO message VALUES (?1,'s',?2)",
            rusqlite::params![id, data.to_string()],
        )
        .unwrap();
    }
    let messages = tokscale_core::sessions::opencode::parse_opencode_sqlite(&path);
    assert_eq!(messages.len(), 2);
    let svc = service(
        json!({"provider_overrides":[rule("vertex","claude-sonnet-4",1.),rule("anthropic","claude-sonnet-4",7.)]}),
    );
    assert_eq!(
        messages
            .iter()
            .map(|m| svc.estimate_message(m).cost)
            .sum::<f64>(),
        8.
    );
}

#[test]
fn codex_session_billing_switches_are_time_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout.jsonl");
    let rows = [
        json!({"type":"session_meta","payload":{"id":"s","model_provider":"gateway-a"}}),
        json!({"type":"turn_context","payload":{"model":"gpt-5","turn_id":"t1"}}),
        json!({"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}}}),
        json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"model_provider_id":"gateway-b"}}}),
        json!({"type":"turn_context","payload":{"model":"gpt-5","turn_id":"t2"}}),
        json!({"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":20,"output_tokens":3,"total_tokens":23}}}}),
    ];
    std::fs::write(
        &path,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let messages = tokscale_core::sessions::codex::parse_codex_file(&path);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].billing_identity.id(), Some("gateway-a"));
    assert_eq!(messages[1].billing_identity.id(), Some("gateway-b"));
}

#[test]
fn checked_in_schema_and_examples_track_every_rate_dimension() {
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/custom-pricing.schema.json")).unwrap();
    let example = CustomPricing::load_from_str(
        include_str!("../../../examples/custom-pricing.json"),
        Path::new("example.json"),
    );
    assert!(example.diagnostics().is_empty());
    assert_eq!(example.len(), 3);
    let props = schema["$defs"]["pricing"]["properties"]
        .as_object()
        .unwrap();
    for field in props
        .keys()
        .filter(|key| !matches!(key.as_str(), "notes" | "source"))
    {
        let mut pricing = json!({"input_cost_per_million_tokens":1.});
        if field == "input_cost_per_token" {
            pricing = json!({});
        }
        pricing[field] = json!(1.);
        let value = json!({"provider_overrides":[{"provider_id":"fixture","model_id":"atlas","pricing":pricing}]});
        let custom =
            CustomPricing::load_from_str(&value.to_string(), Path::new("schema-case.json"));
        assert!(
            custom.diagnostics().is_empty(),
            "{field}: {:?}",
            custom.diagnostics()
        );
    }
}

#[test]
fn claude_conflicting_record_channels_remain_unknown_independent_of_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    for providers in [
        ["vertex", "anthropic", "vertex"],
        ["anthropic", "vertex", "anthropic"],
    ] {
        let lines=providers.iter().map(|provider|json!({"type":"assistant","requestId":"r","timestamp":"2026-10-01T10:00:00Z","message":{"id":"m","model":"claude-sonnet-4","provider_id":provider,"usage":{"input_tokens":100,"output_tokens":20}}}).to_string()).collect::<Vec<_>>().join("\n");
        std::fs::write(&path, lines).unwrap();
        let messages = tokscale_core::sessions::claudecode::parse_claude_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].billing_identity, BillingIdentity::Unknown);
    }
}

#[test]
fn protocol_and_model_namespace_are_not_billing_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let roo = dir.path().join("ui_messages.json");
    let payload = json!({"tokensIn":10,"tokensOut":1,"apiProtocol":"anthropic"}).to_string();
    std::fs::write(&roo,json!([{"type":"say","say":"api_req_started","ts":"2026-10-01T10:00:00Z","text":payload,"modelInfo":{"modelId":"claude-sonnet-4"}}]).to_string()).unwrap();
    let messages = tokscale_core::sessions::roocode::parse_roocode_file(&roo);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].billing_identity, BillingIdentity::Unknown);
    let mux = dir.path().join("session-usage.json");
    std::fs::write(&mux,json!({"version":1,"byModel":{"anthropic:claude-sonnet-4":{"input":{"tokens":10,"cost_usd":0.},"output":{"tokens":1,"cost_usd":0.}}},"lastRequest":{"model":"anthropic:claude-sonnet-4","timestamp":1780000000000u64}}).to_string()).unwrap();
    let messages = tokscale_core::sessions::mux::parse_mux_file(&mux);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].billing_identity, BillingIdentity::Unknown);
    let reasonix = dir.path().join("stats.jsonl");
    for model in [
        "anthropic/claude-sonnet-4",
        "claude-sonnet-4",
        "plugin/relay/claude-sonnet-4",
    ] {
        std::fs::write(&reasonix,json!({"ts":"2026-10-01T10:00:00Z","model":model,"prompt":10,"completion":1,"total":11}).to_string()).unwrap();
        let messages = tokscale_core::sessions::reasonix::parse_reasonix_file(&reasonix);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].billing_identity, BillingIdentity::Unknown);
    }
}

#[test]
fn synthetic_report_tail_keeps_raw_model_for_scoped_pricing() {
    let dir = tempfile::tempdir().unwrap();
    let messages_dir = dir.path().join(".local/share/opencode/storage/message/s");
    std::fs::create_dir_all(&messages_dir).unwrap();
    let raw = "hf:zai-org/GLM-4.7";
    std::fs::write(messages_dir.join("m.json"),json!({"id":"m","sessionID":"s","role":"assistant","modelID":raw,"providerID":"private-gateway","time":{"created":1780000000000u64},"tokens":{"input":1000000,"output":0,"cache":{"read":0,"write":0}}}).to_string()).unwrap();
    let svc = service(
        json!({"models":{raw:{"input_cost_per_million_tokens":2.}},"provider_overrides":[rule("private-gateway",raw,1.),rule("private-gateway","glm-4.7",3.)]}),
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    let messages = rt
        .block_on(
            tokscale_core::parse_local_unified_messages_with_pricing_uncached(
                tokscale_core::LocalParseOptions {
                    home_dir: Some(dir.path().to_string_lossy().into()),
                    clients: Some(vec!["opencode".into(), "synthetic".into()]),
                    ..Default::default()
                },
                Some(&svc),
            ),
        )
        .unwrap();
    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message.pricing_model_id.as_deref(), Some(raw));
    assert_eq!(message.model_id, "glm-4.7");
    assert_eq!(message.billing_identity.id(), Some("private-gateway"));
    assert_eq!(message.cost, 1.);
    let explained = svc.explain_message(message);
    assert_eq!(explained.final_estimate, 1.);
    assert!(!explained.normalized);
    assert_eq!(
        explained.matched_model_id.as_deref(),
        Some("hf:zai-org/glm-4.7")
    );
}

#[test]
fn claude_record_channel_outweighs_later_variant_metadata() {
    let home = tempfile::tempdir().unwrap();
    let variant = home.path().join(".cc-mirror/fixture");
    let config = variant.join("config");
    let path = config.join("projects/p/session.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        variant.join("variant.json"),
        json!({"name":"fixture","provider":"session-b","configDir":config}).to_string(),
    )
    .unwrap();
    let first = json!({"type":"assistant","requestId":"r","message":{"id":"m","model":"claude-sonnet-4","providerId":"record-a","usage":{"input_tokens":10,"output_tokens":1}}});
    let later = json!({"type":"assistant","requestId":"r","message":{"id":"m","model":"claude-sonnet-4","usage":{"input_tokens":10,"output_tokens":2}}});
    std::fs::write(&path, format!("{first}\n{later}\n")).unwrap();
    let messages =
        tokscale_core::sessions::claudecode::parse_claude_file_with_home(&path, Some(home.path()));
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].billing_identity.id(), Some("record-a"));
}

#[test]
fn roo_kilo_and_cline_pricing_preserves_reported_cost_and_estimates_net_input() {
    use tokscale_core::sessions::{
        cline::parse_cline_file, kilocode::parse_kilocode_file, roocode::parse_roocode_file,
    };
    let svc = service(json!({"provider_overrides":[{
        "provider_id":"gateway-a", "model_id":"atlas",
        "pricing":{"input_cost_per_million_tokens":10.,"output_cost_per_million_tokens":0.,
            "cache_read_input_token_cost":0.,"cache_creation_input_token_cost":0.}
    }]}));
    for (parse, expected_input) in [
        (parse_roocode_file as fn(&Path) -> Vec<UnifiedMessage>, 300),
        (parse_kilocode_file as fn(&Path) -> Vec<UnifiedMessage>, 300),
        (parse_cline_file as fn(&Path) -> Vec<UnifiedMessage>, 1000),
    ] {
        for cost in [0., 0.25] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("ui_messages.json");
            std::fs::write(
                &path,
                json!([{
                    "type":"say", "say":"api_req_started", "ts":1789022955199_i64,
                    "modelInfo":{"modelId":"atlas","providerId":"gateway-a"},
                    "text":json!({"cost":cost,"tokensIn":1000,"tokensOut":10,
                        "cacheReads":600,"cacheWrites":100,"apiProtocol":"openai"}).to_string()
                }])
                .to_string(),
            )
            .unwrap();
            let messages = parse(&path);
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].tokens.input, expected_input);
            assert_eq!(messages[0].billing_identity.id(), Some("gateway-a"));
            let explained = svc.explain_message(&messages[0]);
            let expected_cost = if cost > 0. {
                cost
            } else {
                expected_input as f64 * 10. / 1_000_000.
            };
            assert!((explained.final_estimate - expected_cost).abs() < 1e-12);
            assert_eq!(explained.scope == "reported", cost > 0.);
        }
    }
}
