use std::collections::HashMap;

use ratatui::style::Color;

use super::data::ModelUsage;
use super::ui::widgets::{get_provider_from_model, get_provider_shade};

/// Builds a `model_name -> Color` map where each provider's models are
/// cost-ranked; rank 0 (highest cost) gets the base provider color and
/// later ranks get progressively lighter shades.
///
/// Aggregates cost per (provider, model) so the same model appearing in
/// multiple group-by buckets (e.g. `GroupBy::WorkspaceModel`) doesn't
/// inflate the rank count. Ties on cost are resolved by model name so
/// shade assignment stays deterministic across refreshes.
pub fn build_model_shade_map(models: &[ModelUsage]) -> HashMap<String, Color> {
    let mut by_provider: HashMap<&str, HashMap<&str, f64>> = HashMap::new();
    for m in models {
        let provider = get_provider_from_model(&m.model);
        let cost = if m.cost.is_finite() { m.cost } else { 0.0 };
        *by_provider
            .entry(provider)
            .or_default()
            .entry(m.model.as_str())
            .or_insert(0.0) += cost;
    }

    let mut map = HashMap::new();
    for (provider, models_map) in by_provider {
        let mut ranked: Vec<(&str, f64)> = models_map.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        for (rank, (name, _)) in ranked.iter().enumerate() {
            map.insert(name.to_string(), get_provider_shade(provider, rank));
        }
    }
    map
}
