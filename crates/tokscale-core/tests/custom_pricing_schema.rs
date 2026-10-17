use serde_json::{json, Value};
use std::{path::Path, sync::LazyLock};
use tokscale_core::pricing::custom::CustomPricing;

static SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../schemas/custom-pricing.schema.json")).unwrap()
});
static VALIDATOR: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    jsonschema::draft202012::new(&SCHEMA).expect("custom pricing schema must compile")
});

fn assert_validation(value: Value, expected_valid: bool, expected_rules: usize) {
    let runtime = CustomPricing::load_from_str(&value.to_string(), Path::new("fixture.json"));
    assert_eq!(
        runtime.diagnostics().is_empty(),
        expected_valid,
        "runtime validation for {value}: {:?}",
        runtime.diagnostics()
    );
    assert_eq!(
        runtime.len(),
        expected_rules,
        "runtime usable rules for {value}"
    );
    assert_eq!(
        VALIDATOR.is_valid(&value),
        expected_valid,
        "schema/runtime validation differs for {value}: {:?}",
        VALIDATOR
            .iter_errors(&value)
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
    );
}

fn global(pricing: Value) -> Value {
    json!({"models": {"fixture-model": pricing}})
}

fn scoped(pricing: Value) -> Value {
    json!({"provider_overrides": [{
        "provider_id": "private-gateway",
        "model_id": "fixture-model",
        "pricing": pricing,
    }]})
}

#[test]
fn schema_and_documented_example_are_valid() {
    assert!(jsonschema::draft202012::meta::is_valid(&SCHEMA));
    let example =
        serde_json::from_str(include_str!("../../../examples/custom-pricing.json")).unwrap();
    assert_validation(example, true, 3);
}

#[test]
fn schema_rejects_global_null_only_pricing() {
    for field in [
        "input_cost_per_token",
        "input_cost_per_million_tokens",
        "output_cost_per_token",
        "output_cost_per_million_tokens",
    ] {
        assert_validation(global(json!({field: null})), false, 0);
        // A cache or tier rate cannot make a null base rate usable.
        assert_validation(
            global(json!({field: null, "cache_read_input_token_cost": 0.0})),
            false,
            0,
        );
    }
    assert_validation(
        global(json!({"input_cost_per_token": null, "output_cost_per_token": null})),
        false,
        0,
    );
}

#[test]
fn schema_preserves_global_nullable_unit_compatibility() {
    for (per_token, per_million) in [
        ("input_cost_per_token", "input_cost_per_million_tokens"),
        ("output_cost_per_token", "output_cost_per_million_tokens"),
        (
            "input_cost_per_token_above_200k_tokens",
            "input_cost_per_million_tokens_above_200k_tokens",
        ),
        (
            "cache_read_input_token_cost",
            "cache_read_input_token_cost_per_million_tokens",
        ),
        (
            "cache_creation_input_token_cost_above_1hr_above_200k_tokens",
            "cache_creation_input_token_cost_per_million_tokens_above_1hr_above_200k_tokens",
        ),
    ] {
        for (left, right) in [(json!(null), json!(0.0)), (json!(1.0), json!(null))] {
            let mut pricing = json!({"input_cost_per_token": 0.0, "output_cost_per_token": 0.0});
            pricing[per_token] = left;
            pricing[per_million] = right;
            assert_validation(global(pricing), true, 1);
        }
        let mut pricing = json!({"input_cost_per_token": 0.0, "output_cost_per_token": 0.0});
        pricing[per_token] = json!(0.0);
        pricing[per_million] = json!(0.0);
        assert_validation(global(pricing), false, 0);
    }
}

#[test]
fn schema_matches_runtime_rate_validation() {
    for pricing in [
        json!({"input_cost_per_token": 0.0}),
        json!({"output_cost_per_million_tokens": 0.0}),
        json!({"input_cost_per_token": 1.0, "notes": "base rate", "source": "invoice"}),
    ] {
        assert_validation(global(pricing.clone()), true, 1);
        assert_validation(scoped(pricing), true, 1);
    }
    for pricing in [
        json!({}),
        json!({"input_cost_per_token": -1.0}),
        json!({"input_cost_per_token": "1"}),
        json!({"cache_read_input_token_cost": 1.0}),
        json!({"input_cost_per_token": 1.0, "input_cost_per_million_tokens": 1.0}),
    ] {
        assert_validation(global(pricing.clone()), false, 0);
        assert_validation(scoped(pricing), false, 0);
    }
    // Legacy rows retain nullable/unknown metadata; scoped rules are strict.
    for pricing in [
        json!({"input_cost_per_token": 0.0, "output_cost_per_token": null}),
        json!({"input_cost_per_token": 0.0, "cache_read_input_token_cost": null}),
        json!({"input_cost_per_token": 0.0, "old_metadata": "retained"}),
        json!({"input_cost_per_token": 0.0, "max_tokens": 128_000}),
    ] {
        assert_validation(global(pricing.clone()), true, 1);
        assert_validation(scoped(pricing), false, 0);
    }

    // A misspelled rate cannot be treated as harmless legacy metadata: doing so
    // would select the partial custom row and add partial money to local totals.
    for field in [
        "output_cost_per_million_toknes",
        "custom_price",
        "discount_rate",
    ] {
        assert_validation(
            global(json!({"input_cost_per_million_tokens": 1.0, (field): 10.0})),
            false,
            0,
        );
    }
}

#[test]
fn schema_matches_runtime_scoped_selector_validation() {
    for provider in ["unknown", " UnKnOwN ", "", " \t\n "] {
        let mut value = scoped(json!({"input_cost_per_token": 0.0}));
        value["provider_overrides"][0]["provider_id"] = json!(provider);
        assert_validation(value, false, 0);
    }
    for provider in [
        " VERTEX ",
        "openrouter/google",
        "123",
        "gateway_a",
        "vertex\n",
        "\nvertex",
        "a\nb",
    ] {
        let mut value = scoped(json!({"input_cost_per_token": 0.0}));
        value["provider_overrides"][0]["provider_id"] = json!(provider);
        assert_validation(value, true, 1);
    }
    let mut value = scoped(json!({"input_cost_per_token": 0.0}));
    value["provider_overrides"][0]["model_id"] = json!(" \t ");
    assert_validation(value, false, 0);
}
