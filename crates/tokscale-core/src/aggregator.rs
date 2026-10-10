//! Parallel aggregation of session data
//!
//! Uses rayon for parallel map-reduce operations.

use crate::sessions::UnifiedMessage;
use crate::{
    ClientContribution, CostProvenance, CostProvenanceKind, DailyContribution, DailyTotals,
    DataSummary, EstimateSource, GraphMeta, GraphResult, SessionContribution, TokenBreakdown,
    YearSummary,
};
use rayon::prelude::*;
use std::collections::HashMap;

/// Aggregate messages into daily contributions
pub fn aggregate_by_date(messages: Vec<UnifiedMessage>) -> Vec<DailyContribution> {
    if messages.is_empty() {
        return Vec::new();
    }

    // Estimate unique days (typically 1-365) - use message count / 10 as heuristic
    let estimated_days = (messages.len() / 10).clamp(30, 400);

    // Parallel aggregation using fold/reduce pattern
    let daily_map: HashMap<String, DayAccumulator> = messages
        .into_par_iter()
        .fold(
            || HashMap::with_capacity(estimated_days),
            |mut acc: HashMap<String, DayAccumulator>, msg| {
                let entry = acc.entry(msg.date.clone()).or_default();
                entry.add_message(&msg);
                acc
            },
        )
        .reduce(
            || HashMap::with_capacity(estimated_days),
            |mut a, b| {
                for (date, acc) in b {
                    a.entry(date).or_default().merge(acc);
                }
                a
            },
        );

    DailyFold { days: daily_map }.finish()
}

/// Incremental form of [`aggregate_by_date`].
///
/// [`aggregate_by_date`] takes a `Vec<UnifiedMessage>`, which means the caller
/// has already paid to materialize every message. Callers that receive
/// messages one at a time can fold each one in and drop it immediately, so
/// peak memory is the day map (a few hundred entries) rather than the corpus.
///
/// [`aggregate_by_date`] delegates its own tail here, so both paths produce
/// byte-identical contributions.
#[derive(Default)]
pub struct DailyFold {
    days: HashMap<String, DayAccumulator>,
}

impl DailyFold {
    pub fn add(&mut self, message: &UnifiedMessage) {
        self.days
            .entry(message.date.clone())
            .or_default()
            .add_message(message);
    }

    pub fn finish(self) -> Vec<DailyContribution> {
        // Convert to sorted vector with pre-allocated capacity
        let mut contributions: Vec<DailyContribution> = Vec::with_capacity(self.days.len());
        contributions.extend(
            self.days
                .into_iter()
                .map(|(date, acc)| acc.into_contribution(date)),
        );

        // Sort by date
        contributions.sort_by(|a, b| a.date.cmp(&b.date));

        // Calculate intensities based on max cost
        calculate_intensities(&mut contributions);

        contributions
    }
}

/// Aggregate messages into per-session contributions, keyed on `session_id`.
///
/// Each returned [`SessionContribution`] sums all token buckets and cost for a
/// single session and exposes the same client/model breakdown shape as
/// [`aggregate_by_date`].  Sessions are sorted by `last_seen` descending so the
/// most recently active sessions appear first.
pub fn aggregate_by_session(messages: Vec<UnifiedMessage>) -> Vec<SessionContribution> {
    if messages.is_empty() {
        return Vec::new();
    }

    let session_map: HashMap<String, SessionAccumulator> = messages
        .into_par_iter()
        .fold(
            HashMap::new,
            |mut acc: HashMap<String, SessionAccumulator>, msg| {
                let entry = acc.entry(msg.session_id.clone()).or_default();
                entry.add_message(&msg);
                acc
            },
        )
        .reduce(HashMap::new, |mut a, b| {
            for (id, acc) in b {
                a.entry(id).or_default().merge(acc);
            }
            a
        });

    let mut contributions: Vec<SessionContribution> = session_map
        .into_iter()
        .map(|(session_id, acc)| acc.into_contribution(session_id))
        .collect();

    // Most recently active first; stable sort by session_id when ties.
    contributions.sort_by(|a, b| {
        b.last_seen
            .cmp(&a.last_seen)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });

    contributions
}

/// Calculate summary statistics
pub fn calculate_summary(contributions: &[DailyContribution]) -> DataSummary {
    // Daily totals already saturate at i64::MAX (clamped extreme inputs), so
    // summing several such days must saturate too rather than overflow.
    let total_tokens: i64 = contributions
        .iter()
        .map(|c| c.totals.tokens)
        .fold(0i64, i64::saturating_add);
    // @keep: the trailing `+ 0.0` looks redundant and is not.
    // `Sum for f64` folds from `-0.0`, the additive identity that preserves the
    // sign of every addend, so an empty set sums to `-0.0` and `{:.2}` renders
    // it as "-0.00". Adding `+0.0` normalizes that sign without changing any
    // other value, matching the report aggregators.
    //
    // Only an empty set, or one whose addends are exclusively `-0.0`, reaches
    // the fold's identity unchanged. A single `+0.0` contribution already
    // produced `+0.0` before this fix, since `-0.0 + 0.0 == +0.0`.
    let total_cost: f64 = contributions.iter().map(|c| c.totals.cost).sum::<f64>() + 0.0;
    let active_days = contributions
        .iter()
        .filter(|c| c.totals.tokens > 0 || c.totals.cost > 0.0 || c.totals.messages > 0)
        .count() as i32;
    let max_cost = contributions
        .iter()
        .map(|c| c.totals.cost)
        .fold(0.0, f64::max);

    let mut clients_set = std::collections::HashSet::with_capacity(5);
    let mut models_set = std::collections::HashSet::with_capacity(20);
    let mut provenance = CostProvenanceAccumulator::default();

    for c in contributions {
        if c.totals.tokens > 0
            || c.totals.cost > 0.0
            || c.totals.messages > 0
            || c.totals
                .cost_provenance
                .as_ref()
                .is_some_and(|p| p.kind != CostProvenanceKind::Unknown)
        {
            if let Some(prov) = &c.totals.cost_provenance {
                provenance.add_provenance(prov);
            }
        }
        for s in &c.clients {
            clients_set.insert(s.client.clone());
            models_set.insert(s.model_id.clone());
        }
    }

    DataSummary {
        total_tokens,
        total_cost,
        total_days: contributions.len() as i32,
        active_days,
        average_per_day: if active_days > 0 {
            total_cost / active_days as f64
        } else {
            0.0
        },
        max_cost_in_single_day: max_cost,
        clients: {
            let mut v: Vec<_> = clients_set.into_iter().collect();
            v.sort();
            v
        },
        models: {
            let mut v: Vec<_> = models_set.into_iter().collect();
            v.sort();
            v
        },
        cost_provenance: Some(provenance.finish()),
    }
}

/// Calculate year summaries
pub fn calculate_years(contributions: &[DailyContribution]) -> Vec<YearSummary> {
    let mut years_map: HashMap<String, YearAccumulator> = HashMap::with_capacity(5);

    for c in contributions {
        // Guard against short/invalid date strings
        if c.date.len() < 4 {
            eprintln!(
                "Warning: Skipping contribution with invalid date '{}' ({} tokens, ${:.4} cost)",
                c.date, c.totals.tokens, c.totals.cost
            );
            continue;
        }
        let year = &c.date[0..4];
        let entry = years_map.entry(year.to_string()).or_default();
        entry.tokens = entry.tokens.saturating_add(c.totals.tokens);
        entry.cost += c.totals.cost;

        if c.totals.tokens > 0
            || c.totals.cost > 0.0
            || c.totals.messages > 0
            || c.totals
                .cost_provenance
                .as_ref()
                .is_some_and(|p| p.kind != CostProvenanceKind::Unknown)
        {
            if let Some(prov) = &c.totals.cost_provenance {
                entry.provenance.add_provenance(prov);
            }
        }

        if entry.start.is_empty() || c.date < entry.start {
            entry.start = c.date.clone();
        }
        if entry.end.is_empty() || c.date > entry.end {
            entry.end = c.date.clone();
        }
    }

    let mut years: Vec<YearSummary> = Vec::with_capacity(years_map.len());
    years.extend(years_map.into_iter().map(|(year, acc)| YearSummary {
        year,
        total_tokens: acc.tokens,
        total_cost: acc.cost,
        range_start: acc.start,
        range_end: acc.end,
        cost_provenance: Some(acc.provenance.finish()),
    }));

    years.sort_by(|a, b| a.year.cmp(&b.year));
    years
}

/// Generate complete graph result
pub fn generate_graph_result(
    contributions: Vec<DailyContribution>,
    processing_time_ms: u32,
) -> GraphResult {
    let summary = calculate_summary(&contributions);
    let years = calculate_years(&contributions);

    let date_range_start = contributions
        .first()
        .map(|c| c.date.clone())
        .unwrap_or_default();
    let date_range_end = contributions
        .last()
        .map(|c| c.date.clone())
        .unwrap_or_default();

    GraphResult {
        meta: GraphMeta {
            generated_at: chrono::Utc::now().to_rfc3339(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            date_range_start,
            date_range_end,
            processing_time_ms,
        },
        summary,
        years,
        contributions,
        time_metrics: None,
        unpriced_submission_usage: Vec::new(),
        incomplete_cost_dates: std::collections::BTreeSet::new(),
    }
}

// =============================================================================
// Internal helpers
// =============================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CostProvenanceAccumulator {
    has_unknown: bool,
    has_provider_reported: bool,
    has_estimated: bool,
    has_catalog: bool,
    has_custom: bool,
    has_estimate_unknown: bool,
}

impl CostProvenanceAccumulator {
    pub fn is_empty(&self) -> bool {
        !self.has_unknown && !self.has_provider_reported && !self.has_estimated
    }

    pub fn add_message(&mut self, msg: &UnifiedMessage) {
        let has_usage = msg.tokens.total() > 0;
        let has_cost = msg.cost > 0.0;
        let is_explicit_source = msg.cost_source != crate::sessions::CostSource::Unknown;

        if !has_usage && !has_cost && !is_explicit_source {
            return;
        }

        match msg.cost_source {
            crate::sessions::CostSource::ProviderReported => {
                self.has_provider_reported = true;
            }
            crate::sessions::CostSource::Estimated => {
                self.has_estimated = true;
                match msg.estimate_source {
                    Some(EstimateSource::Catalog) => self.has_catalog = true,
                    Some(EstimateSource::Custom) => self.has_custom = true,
                    Some(EstimateSource::Mixed) => {
                        self.has_catalog = true;
                        self.has_custom = true;
                    }
                    Some(EstimateSource::Unknown) | None => {
                        self.has_estimate_unknown = true;
                    }
                }
            }
            crate::sessions::CostSource::Unknown => {
                self.has_unknown = true;
            }
        }
    }

    pub fn add_provenance(&mut self, prov: &CostProvenance) {
        match prov.kind {
            CostProvenanceKind::Unknown => {
                self.has_unknown = true;
            }
            CostProvenanceKind::ProviderReported => {
                self.has_provider_reported = true;
            }
            CostProvenanceKind::Estimated => {
                self.has_estimated = true;
                match prov.estimate_source {
                    Some(EstimateSource::Catalog) => self.has_catalog = true,
                    Some(EstimateSource::Custom) => self.has_custom = true,
                    Some(EstimateSource::Mixed) => {
                        self.has_catalog = true;
                        self.has_custom = true;
                    }
                    Some(EstimateSource::Unknown) | None => {
                        self.has_estimate_unknown = true;
                    }
                }
            }
            CostProvenanceKind::Mixed => {
                if let Some(src) = prov.estimate_source {
                    self.has_estimated = true;
                    match src {
                        EstimateSource::Catalog => self.has_catalog = true,
                        EstimateSource::Custom => self.has_custom = true,
                        EstimateSource::Mixed => {
                            self.has_catalog = true;
                            self.has_custom = true;
                        }
                        EstimateSource::Unknown => self.has_estimate_unknown = true,
                    }
                    self.has_unknown = true;
                } else {
                    self.has_unknown = true;
                    self.has_provider_reported = true;
                }
            }
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.has_unknown |= other.has_unknown;
        self.has_provider_reported |= other.has_provider_reported;
        self.has_estimated |= other.has_estimated;
        self.has_catalog |= other.has_catalog;
        self.has_custom |= other.has_custom;
        self.has_estimate_unknown |= other.has_estimate_unknown;
    }

    pub fn finish(&self) -> CostProvenance {
        let kind_count = (self.has_unknown as u8)
            + (self.has_provider_reported as u8)
            + (self.has_estimated as u8);

        let kind = match kind_count {
            0 => CostProvenanceKind::Unknown,
            1 => {
                if self.has_provider_reported {
                    CostProvenanceKind::ProviderReported
                } else if self.has_estimated {
                    CostProvenanceKind::Estimated
                } else {
                    CostProvenanceKind::Unknown
                }
            }
            _ => CostProvenanceKind::Mixed,
        };

        let estimate_source = if self.has_estimated {
            let est_count = (self.has_catalog as u8)
                + (self.has_custom as u8)
                + (self.has_estimate_unknown as u8);
            match est_count {
                0 => Some(EstimateSource::Unknown),
                1 => {
                    if self.has_catalog {
                        Some(EstimateSource::Catalog)
                    } else if self.has_custom {
                        Some(EstimateSource::Custom)
                    } else {
                        Some(EstimateSource::Unknown)
                    }
                }
                _ => Some(EstimateSource::Mixed),
            }
        } else {
            None
        };

        CostProvenance {
            kind,
            estimate_source,
        }
    }
}

struct DayAccumulator {
    totals: DailyTotals,
    token_breakdown: TokenBreakdown,
    clients: HashMap<String, ClientContribution>,
    provenance: CostProvenanceAccumulator,
    client_provenance: HashMap<String, CostProvenanceAccumulator>,
}

impl Default for DayAccumulator {
    fn default() -> Self {
        Self {
            totals: DailyTotals::default(),
            token_breakdown: TokenBreakdown::default(),
            clients: HashMap::with_capacity(8),
            provenance: CostProvenanceAccumulator::default(),
            client_provenance: HashMap::with_capacity(8),
        }
    }
}

impl DayAccumulator {
    fn add_message(&mut self, msg: &UnifiedMessage) {
        let total_tokens = msg.tokens.total();

        self.totals.tokens = self.totals.tokens.saturating_add(total_tokens);
        self.totals.cost += msg.cost;
        self.totals.messages = self
            .totals
            .messages
            .saturating_add(msg.message_count.max(0));

        self.token_breakdown += &msg.tokens;
        self.provenance.add_message(msg);

        // Update client contribution
        // Canonical (alias-free) id: this contribution is serialized into the
        // submit/upload/export payload, so a machine-local `modelAliases` config
        // must not rewrite the model identity that leaves the machine.
        let key = format!(
            "{}:{}",
            msg.client,
            crate::canonical_model_id(&msg.model_id)
        );
        self.client_provenance
            .entry(key.clone())
            .or_default()
            .add_message(msg);
        let client_entry = self
            .clients
            .entry(key)
            .or_insert_with(|| ClientContribution {
                client: msg.client.clone(),
                model_id: crate::canonical_model_id(&msg.model_id),
                provider_id: msg.provider_id.clone(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                messages: 0,
                cost_provenance: None,
            });

        // Merge provider_id if different provider contributes to same client+model
        if !client_entry
            .provider_id
            .split(", ")
            .any(|p| p == msg.provider_id)
        {
            client_entry.provider_id = format!("{}, {}", client_entry.provider_id, msg.provider_id);
        }

        client_entry.tokens += &msg.tokens;
        client_entry.cost += msg.cost;
        client_entry.messages = client_entry
            .messages
            .saturating_add(msg.message_count.max(0));

        // Normalize provider order for deterministic output
        let mut providers: Vec<&str> = client_entry.provider_id.split(", ").collect();
        providers.sort_unstable();
        providers.dedup();
        client_entry.provider_id = providers.join(", ");
    }

    fn merge(&mut self, other: DayAccumulator) {
        self.totals.tokens = self.totals.tokens.saturating_add(other.totals.tokens);
        self.totals.cost += other.totals.cost;
        self.totals.messages = self.totals.messages.saturating_add(other.totals.messages);

        self.token_breakdown += &other.token_breakdown;
        self.provenance.merge(other.provenance);

        for (key, prov) in other.client_provenance {
            self.client_provenance.entry(key).or_default().merge(prov);
        }

        for (key, client_contrib) in other.clients {
            let entry = self
                .clients
                .entry(key)
                .or_insert_with(|| ClientContribution {
                    client: client_contrib.client.clone(),
                    model_id: client_contrib.model_id.clone(),
                    provider_id: client_contrib.provider_id.clone(),
                    tokens: TokenBreakdown::default(),
                    cost: 0.0,
                    messages: 0,
                    cost_provenance: None,
                });

            // Merge provider_ids from parallel reduction
            for provider in client_contrib.provider_id.split(", ") {
                if !entry.provider_id.split(", ").any(|p| p == provider) {
                    entry.provider_id = format!("{}, {}", entry.provider_id, provider);
                }
            }

            entry.tokens += &client_contrib.tokens;
            entry.cost += client_contrib.cost;
            entry.messages = entry.messages.saturating_add(client_contrib.messages);
        }

        // Normalize provider order for deterministic output
        for entry in self.clients.values_mut() {
            let mut providers: Vec<&str> = entry.provider_id.split(", ").collect();
            providers.sort_unstable();
            providers.dedup();
            entry.provider_id = providers.join(", ");
        }
    }

    fn into_contribution(self, date: String) -> DailyContribution {
        let token_breakdown = TokenBreakdown {
            input: self.token_breakdown.input.max(0),
            output: self.token_breakdown.output.max(0),
            cache_read: self.token_breakdown.cache_read.max(0),
            cache_write: self.token_breakdown.cache_write.max(0),
            cache_write_1h: self.token_breakdown.cache_write_1h.max(0),
            reasoning: self.token_breakdown.reasoning.max(0),
        };

        let mut client_provenance = self.client_provenance;
        let clients: Vec<ClientContribution> = self
            .clients
            .into_iter()
            .map(|(key, mut s)| {
                s.tokens.input = s.tokens.input.max(0);
                s.tokens.output = s.tokens.output.max(0);
                s.tokens.cache_read = s.tokens.cache_read.max(0);
                s.tokens.cache_write = s.tokens.cache_write.max(0);
                s.tokens.cache_write_1h = s.tokens.cache_write_1h.max(0);
                s.tokens.reasoning = s.tokens.reasoning.max(0);
                s.cost = s.cost.max(0.0);
                s.cost_provenance = Some(
                    client_provenance
                        .remove(&key)
                        .map(|p| p.finish())
                        .unwrap_or_default(),
                );
                s
            })
            .collect();

        DailyContribution {
            date,
            totals: DailyTotals {
                tokens: self.totals.tokens.max(0),
                cost: self.totals.cost.max(0.0),
                messages: self.totals.messages.max(0),
                cost_provenance: Some(self.provenance.finish()),
            },
            intensity: 0,
            token_breakdown,
            clients,
            active_time_ms: None,
        }
    }
}

struct SessionAccumulator {
    totals: DailyTotals,
    token_breakdown: TokenBreakdown,
    clients: HashMap<String, ClientContribution>,
    provenance: CostProvenanceAccumulator,
    client_provenance: HashMap<String, CostProvenanceAccumulator>,
    /// Tracks the most-active (client, provider, model) for the session, used
    /// as the canonical top-level fields on `SessionContribution`.
    top_client: String,
    top_provider: String,
    top_model: String,
    top_cost: f64,
    first_seen: i64,
    last_seen: i64,
}

impl Default for SessionAccumulator {
    fn default() -> Self {
        Self {
            totals: DailyTotals::default(),
            token_breakdown: TokenBreakdown::default(),
            clients: HashMap::with_capacity(2),
            provenance: CostProvenanceAccumulator::default(),
            client_provenance: HashMap::with_capacity(2),
            top_client: String::new(),
            top_provider: String::new(),
            top_model: String::new(),
            top_cost: f64::NEG_INFINITY,
            first_seen: i64::MAX,
            last_seen: i64::MIN,
        }
    }
}

impl SessionAccumulator {
    fn add_message(&mut self, msg: &UnifiedMessage) {
        let total_tokens = msg.tokens.total();

        self.totals.tokens = self.totals.tokens.saturating_add(total_tokens);
        self.totals.cost += msg.cost;
        self.totals.messages = self
            .totals
            .messages
            .saturating_add(msg.message_count.max(0));

        self.token_breakdown += &msg.tokens;
        self.provenance.add_message(msg);

        // Track tightest (client, provider, model) by cost contribution.
        // Canonical (alias-free) id — this feeds the submitted/exported payload,
        // so machine-local aliases must not rewrite it (see `add_message`).
        let normalized_model = crate::canonical_model_id(&msg.model_id);
        let key = format!("{}:{}:{}", msg.client, msg.provider_id, normalized_model);
        self.client_provenance
            .entry(key.clone())
            .or_default()
            .add_message(msg);
        let client_entry = self
            .clients
            .entry(key)
            .or_insert_with(|| ClientContribution {
                client: msg.client.clone(),
                model_id: normalized_model.clone(),
                provider_id: msg.provider_id.clone(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                messages: 0,
                cost_provenance: None,
            });
        client_entry.tokens += &msg.tokens;
        client_entry.cost += msg.cost;
        client_entry.messages = client_entry
            .messages
            .saturating_add(msg.message_count.max(0));

        if client_entry.cost > self.top_cost {
            self.top_cost = client_entry.cost;
            self.top_client = client_entry.client.clone();
            self.top_provider = client_entry.provider_id.clone();
            self.top_model = client_entry.model_id.clone();
        }

        // Timestamps in UnifiedMessage are stored in milliseconds in most
        // parsers; normalize to seconds for the contribution wire format.
        let secs = if msg.timestamp.abs() > 1_000_000_000_000 {
            msg.timestamp / 1000
        } else {
            msg.timestamp
        };
        if secs < self.first_seen {
            self.first_seen = secs;
        }
        if secs > self.last_seen {
            self.last_seen = secs;
        }
    }

    fn merge(&mut self, other: SessionAccumulator) {
        self.totals.tokens = self.totals.tokens.saturating_add(other.totals.tokens);
        self.totals.cost += other.totals.cost;
        self.totals.messages = self.totals.messages.saturating_add(other.totals.messages);

        self.token_breakdown += &other.token_breakdown;
        self.provenance.merge(other.provenance);

        for (key, prov) in other.client_provenance {
            self.client_provenance.entry(key).or_default().merge(prov);
        }

        for (key, contrib) in other.clients {
            let entry = self
                .clients
                .entry(key)
                .or_insert_with(|| ClientContribution {
                    client: contrib.client.clone(),
                    model_id: contrib.model_id.clone(),
                    provider_id: contrib.provider_id.clone(),
                    tokens: TokenBreakdown::default(),
                    cost: 0.0,
                    messages: 0,
                    cost_provenance: None,
                });
            entry.tokens += &contrib.tokens;
            entry.cost += contrib.cost;
            entry.messages = entry.messages.saturating_add(contrib.messages);

            if entry.cost > self.top_cost {
                self.top_cost = entry.cost;
                self.top_client = entry.client.clone();
                self.top_provider = entry.provider_id.clone();
                self.top_model = entry.model_id.clone();
            }
        }

        if other.first_seen < self.first_seen {
            self.first_seen = other.first_seen;
        }
        if other.last_seen > self.last_seen {
            self.last_seen = other.last_seen;
        }
    }

    fn into_contribution(self, session_id: String) -> SessionContribution {
        let token_breakdown = TokenBreakdown {
            input: self.token_breakdown.input.max(0),
            output: self.token_breakdown.output.max(0),
            cache_read: self.token_breakdown.cache_read.max(0),
            cache_write: self.token_breakdown.cache_write.max(0),
            cache_write_1h: self.token_breakdown.cache_write_1h.max(0),
            reasoning: self.token_breakdown.reasoning.max(0),
        };

        let mut client_provenance = self.client_provenance;
        let mut clients: Vec<ClientContribution> = self
            .clients
            .into_iter()
            .map(|(key, mut c)| {
                c.tokens.input = c.tokens.input.max(0);
                c.tokens.output = c.tokens.output.max(0);
                c.tokens.cache_read = c.tokens.cache_read.max(0);
                c.tokens.cache_write = c.tokens.cache_write.max(0);
                c.tokens.cache_write_1h = c.tokens.cache_write_1h.max(0);
                c.tokens.reasoning = c.tokens.reasoning.max(0);
                c.cost = c.cost.max(0.0);
                c.cost_provenance = Some(
                    client_provenance
                        .remove(&key)
                        .map(|p| p.finish())
                        .unwrap_or_default(),
                );
                c
            })
            .collect();
        clients.sort_by(|a, b| {
            b.cost
                .partial_cmp(&a.cost)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.client.cmp(&b.client))
                .then_with(|| a.model_id.cmp(&b.model_id))
        });

        let first_seen = if self.first_seen == i64::MAX {
            0
        } else {
            self.first_seen
        };
        let last_seen = if self.last_seen == i64::MIN {
            0
        } else {
            self.last_seen
        };

        SessionContribution {
            session_id,
            client: self.top_client,
            provider: self.top_provider,
            model: self.top_model,
            totals: DailyTotals {
                tokens: self.totals.tokens.max(0),
                cost: self.totals.cost.max(0.0),
                messages: self.totals.messages.max(0),
                cost_provenance: Some(self.provenance.finish()),
            },
            token_breakdown,
            clients,
            first_seen,
            last_seen,
        }
    }
}

#[derive(Default)]
struct YearAccumulator {
    tokens: i64,
    cost: f64,
    start: String,
    end: String,
    provenance: CostProvenanceAccumulator,
}

/// Cost-relative intensity buckets (0-4): each day's intensity is a function
/// of its cost relative to the maximum cost across all `contributions`.
pub fn calculate_intensities(contributions: &mut [DailyContribution]) {
    let max_cost = contributions
        .iter()
        .map(|c| c.totals.cost)
        .fold(0.0, f64::max);

    if max_cost == 0.0 {
        return;
    }

    for c in contributions.iter_mut() {
        let ratio = c.totals.cost / max_cost;
        c.intensity = if ratio >= 0.75 {
            4
        } else if ratio >= 0.5 {
            3
        } else if ratio >= 0.25 {
            2
        } else if ratio > 0.0 {
            1
        } else {
            0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    // Helper function to create mock UnifiedMessage
    fn mock_unified_message(
        date: &str,
        tokens: i64,
        cost: f64,
        model: &str,
        client: &str,
    ) -> UnifiedMessage {
        // Parse date string to timestamp
        let datetime = format!("{}T00:00:00Z", date)
            .parse::<DateTime<Utc>>()
            .unwrap();
        let timestamp = datetime.timestamp_millis();

        UnifiedMessage {
            client: client.to_string(),
            model_id: model.to_string(),
            provider_id: "test-provider".to_string(),
            session_id: "test-session".to_string(),
            workspace_key: None,
            workspace_label: None,
            timestamp,
            date: date.to_string(),
            tokens: TokenBreakdown {
                input: tokens / 2,
                output: tokens / 2,
                cache_read: 0,
                cache_write: 0,
                cache_write_1h: 0,
                reasoning: 0,
            },
            cost,
            cost_source: Default::default(),
            service_tier: None,
            duration_ms: None,
            message_count: 1,
            agent: None,
            dedup_key: None,
            session_title: None,
            parent_session_id: None,
            is_turn_start: false,
            model_attribution_conflicted: false,
            estimate_source: None,
        }
    }

    #[test]
    fn test_aggregate_by_date_empty() {
        let messages = Vec::new();
        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_aggregate_by_date_single_message() {
        let messages = vec![mock_unified_message(
            "2024-01-01",
            1000,
            0.05,
            "claude-3-5-sonnet",
            "opencode",
        )];

        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date, "2024-01-01");
        assert_eq!(result[0].totals.tokens, 1000);
        assert_eq!(result[0].totals.cost, 0.05);
        assert_eq!(result[0].totals.messages, 1);
    }

    #[test]
    fn test_aggregate_by_date_multiple_dates() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-02", 2000, 0.10, "gpt-4", "claude"),
            mock_unified_message("2024-01-03", 1500, 0.08, "claude-3-5-sonnet", "opencode"),
        ];

        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 3);

        // Verify sorted by date
        assert_eq!(result[0].date, "2024-01-01");
        assert_eq!(result[1].date, "2024-01-02");
        assert_eq!(result[2].date, "2024-01-03");

        // Verify totals
        assert_eq!(result[0].totals.tokens, 1000);
        assert_eq!(result[1].totals.tokens, 2000);
        assert_eq!(result[2].totals.tokens, 1500);
    }

    #[test]
    fn test_aggregate_by_date_same_date_aggregation() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-01", 2000, 0.10, "gpt-4", "claude"),
            mock_unified_message("2024-01-01", 1500, 0.08, "claude-3-5-sonnet", "opencode"),
        ];

        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date, "2024-01-01");
        assert_eq!(result[0].totals.tokens, 4500);
        assert!((result[0].totals.cost - 0.23).abs() < 0.0001);
        assert_eq!(result[0].totals.messages, 3);
    }

    #[test]
    fn test_aggregate_by_date_token_breakdown() {
        let mut msg =
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode");
        msg.tokens = TokenBreakdown {
            input: 600,
            output: 300,
            cache_read: 50,
            cache_write: 40,
            cache_write_1h: 0,
            reasoning: 10,
        };

        let result = aggregate_by_date(vec![msg]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].token_breakdown.input, 600);
        assert_eq!(result[0].token_breakdown.output, 300);
        assert_eq!(result[0].token_breakdown.cache_read, 50);
        assert_eq!(result[0].token_breakdown.cache_write, 40);
        assert_eq!(result[0].token_breakdown.reasoning, 10);
    }

    #[test]
    fn test_calculate_summary_empty() {
        let contributions = Vec::new();
        let summary = calculate_summary(&contributions);

        assert_eq!(summary.total_tokens, 0);
        assert_eq!(summary.total_cost, 0.0);
        assert_eq!(summary.total_days, 0);
        assert_eq!(summary.active_days, 0);
        assert_eq!(summary.average_per_day, 0.0);
        assert_eq!(summary.max_cost_in_single_day, 0.0);
        // `-0.0 == 0.0` under IEEE, so the assertion above cannot catch a
        // negative zero. The CLI formats this straight through, and "$-0.00"
        // is what the user sees when every row was excluded from a submission.
        assert!(
            !summary.total_cost.is_sign_negative(),
            "an empty summary must not carry a negative zero cost"
        );
    }

    #[test]
    fn empty_summary_cost_does_not_format_as_negative_zero() {
        let summary = calculate_summary(&[]);
        assert_eq!(format!("${:.2}", summary.total_cost), "$0.00");
    }

    /// Pins the boundary the `+ 0.0` comment describes. Only the empty fold
    /// (and an all-`-0.0` one) ever reached the `-0.0` identity: a single
    /// `+0.0` addend already normalized it, because `-0.0 + 0.0 == +0.0`.
    /// Without this, "all-zero" reads as if every zero-cost day was affected.
    #[test]
    fn a_positive_zero_contribution_was_never_the_negative_zero_case() {
        let messages = vec![mock_unified_message(
            "2024-01-01",
            0,
            0.0,
            "claude-sonnet-4-20250514",
            "claude",
        )];
        let contributions = aggregate_by_date(messages);
        let summary = calculate_summary(&contributions);

        assert!(!summary.total_cost.is_sign_negative());
        assert_eq!(format!("${:.2}", summary.total_cost), "$0.00");
    }

    #[test]
    fn test_calculate_summary_single_day() {
        let messages = vec![mock_unified_message(
            "2024-01-01",
            1000,
            0.05,
            "claude-3-5-sonnet",
            "opencode",
        )];
        let contributions = aggregate_by_date(messages);
        let summary = calculate_summary(&contributions);

        assert_eq!(summary.total_tokens, 1000);
        assert_eq!(summary.total_cost, 0.05);
        assert_eq!(summary.total_days, 1);
        assert_eq!(summary.active_days, 1);
        assert_eq!(summary.average_per_day, 0.05);
        assert_eq!(summary.max_cost_in_single_day, 0.05);
    }

    #[test]
    fn test_calculate_summary_multiple_days() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-02", 2000, 0.10, "gpt-4", "claude"),
            mock_unified_message("2024-01-03", 1500, 0.08, "claude-3-5-sonnet", "opencode"),
        ];
        let contributions = aggregate_by_date(messages);
        let summary = calculate_summary(&contributions);

        assert_eq!(summary.total_tokens, 4500);
        assert!((summary.total_cost - 0.23).abs() < 0.0001);
        assert_eq!(summary.total_days, 3);
        assert_eq!(summary.active_days, 3);
        assert!((summary.average_per_day - 0.23 / 3.0).abs() < 0.0001);
        assert!((summary.max_cost_in_single_day - 0.10).abs() < 0.0001);
    }

    #[test]
    fn test_calculate_summary_with_zero_token_days() {
        let contributions = vec![
            DailyContribution {
                date: "2024-01-01".to_string(),
                totals: DailyTotals {
                    tokens: 1000,
                    cost: 0.05,
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
            DailyContribution {
                date: "2024-01-02".to_string(),
                totals: DailyTotals {
                    tokens: 0,
                    cost: 0.0,
                    messages: 0,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
        ];

        let summary = calculate_summary(&contributions);
        assert_eq!(summary.total_days, 2);
        assert_eq!(summary.active_days, 1);
        assert!((summary.average_per_day - 0.05).abs() < 0.0001);
    }

    #[test]
    fn test_calculate_summary_counts_cost_only_days_as_active() {
        let contributions = vec![
            DailyContribution {
                date: "2024-01-01".to_string(),
                totals: DailyTotals {
                    tokens: 1000,
                    cost: 0.05,
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
            DailyContribution {
                date: "2024-01-02".to_string(),
                totals: DailyTotals {
                    tokens: 0,
                    cost: 1.25,
                    messages: 0,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
            DailyContribution {
                date: "2024-01-03".to_string(),
                totals: DailyTotals {
                    tokens: 0,
                    cost: 0.0,
                    messages: 0,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
        ];

        let summary = calculate_summary(&contributions);
        assert_eq!(summary.total_days, 3);
        assert_eq!(summary.active_days, 2);
        assert!((summary.average_per_day - 0.65).abs() < 0.0001);
    }

    #[test]
    fn test_extreme_day_totals_saturate_in_summary_and_years() {
        // Daily totals clamp extreme inputs to i64::MAX; summing several such
        // days must saturate rather than overflow (debug panic / release wrap).
        let saturated_day = |date: &str| DailyContribution {
            date: date.to_string(),
            totals: DailyTotals {
                tokens: i64::MAX,
                cost: 1.0,
                messages: 1,
                cost_provenance: None,
            },
            intensity: 0,
            token_breakdown: TokenBreakdown::default(),
            clients: Vec::new(),
            active_time_ms: None,
        };
        let contributions = vec![saturated_day("2024-01-01"), saturated_day("2024-01-02")];

        let summary = calculate_summary(&contributions);
        assert_eq!(summary.total_tokens, i64::MAX);

        let years = calculate_years(&contributions);
        assert_eq!(years.len(), 1);
        assert_eq!(years[0].total_tokens, i64::MAX);
    }

    #[test]
    fn test_calculate_years_empty() {
        let contributions = Vec::new();
        let years = calculate_years(&contributions);
        assert_eq!(years.len(), 0);
    }

    #[test]
    fn test_calculate_years_single_year() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-06-15", 2000, 0.10, "gpt-4", "claude"),
            mock_unified_message("2024-12-31", 1500, 0.08, "claude-3-5-sonnet", "opencode"),
        ];
        let contributions = aggregate_by_date(messages);
        let years = calculate_years(&contributions);

        assert_eq!(years.len(), 1);
        assert_eq!(years[0].year, "2024");
        assert_eq!(years[0].total_tokens, 4500);
        assert!((years[0].total_cost - 0.23).abs() < 0.0001);
        assert_eq!(years[0].range_start, "2024-01-01");
        assert_eq!(years[0].range_end, "2024-12-31");
    }

    #[test]
    fn test_calculate_years_multiple_years() {
        let messages = vec![
            mock_unified_message("2023-12-31", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-01", 2000, 0.10, "gpt-4", "claude"),
            mock_unified_message("2024-06-15", 1500, 0.08, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2025-01-01", 3000, 0.15, "gpt-4", "claude"),
        ];
        let contributions = aggregate_by_date(messages);
        let years = calculate_years(&contributions);

        assert_eq!(years.len(), 3);

        // Verify sorted by year
        assert_eq!(years[0].year, "2023");
        assert_eq!(years[1].year, "2024");
        assert_eq!(years[2].year, "2025");

        // Verify 2024 aggregation
        assert_eq!(years[1].total_tokens, 3500);
        assert!((years[1].total_cost - 0.18).abs() < 0.0001);
        assert_eq!(years[1].range_start, "2024-01-01");
        assert_eq!(years[1].range_end, "2024-06-15");
    }

    #[test]
    fn test_calculate_years_year_boundary() {
        let messages = vec![
            mock_unified_message("2024-12-31", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2025-01-01", 2000, 0.10, "gpt-4", "claude"),
        ];
        let contributions = aggregate_by_date(messages);
        let years = calculate_years(&contributions);

        assert_eq!(years.len(), 2);
        assert_eq!(years[0].year, "2024");
        assert_eq!(years[0].total_tokens, 1000);
        assert_eq!(years[1].year, "2025");
        assert_eq!(years[1].total_tokens, 2000);
    }

    #[test]
    fn test_calculate_years_invalid_date() {
        let contributions = vec![DailyContribution {
            date: "abc".to_string(), // Invalid date (less than 4 chars)
            totals: DailyTotals {
                tokens: 1000,
                cost: 0.05,
                messages: 1,
                cost_provenance: None,
            },
            intensity: 0,
            token_breakdown: TokenBreakdown::default(),
            clients: Vec::new(),
            active_time_ms: None,
        }];

        let years = calculate_years(&contributions);
        assert_eq!(years.len(), 0); // Should skip invalid dates
    }

    #[test]
    fn test_generate_graph_result_empty() {
        let contributions = Vec::new();
        let result = generate_graph_result(contributions, 100);

        assert_eq!(result.contributions.len(), 0);
        assert_eq!(result.summary.total_tokens, 0);
        assert_eq!(result.years.len(), 0);
        assert_eq!(result.meta.processing_time_ms, 100);
        assert_eq!(result.meta.date_range_start, "");
        assert_eq!(result.meta.date_range_end, "");
    }

    #[test]
    fn test_generate_graph_result_with_data() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-02", 2000, 0.10, "gpt-4", "claude"),
        ];
        let contributions = aggregate_by_date(messages);
        let result = generate_graph_result(contributions, 150);

        assert_eq!(result.contributions.len(), 2);
        assert_eq!(result.summary.total_tokens, 3000);
        assert_eq!(result.years.len(), 1);
        assert_eq!(result.meta.processing_time_ms, 150);
        assert_eq!(result.meta.date_range_start, "2024-01-01");
        assert_eq!(result.meta.date_range_end, "2024-01-02");
        assert_eq!(result.meta.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_calculate_intensities_empty() {
        let mut contributions = Vec::new();
        calculate_intensities(&mut contributions);
        assert_eq!(contributions.len(), 0);
    }

    #[test]
    fn test_calculate_intensities_zero_cost() {
        let mut contributions = vec![
            DailyContribution {
                date: "2024-01-01".to_string(),
                totals: DailyTotals {
                    tokens: 1000,
                    cost: 0.0,
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
            DailyContribution {
                date: "2024-01-02".to_string(),
                totals: DailyTotals {
                    tokens: 2000,
                    cost: 0.0,
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
        ];

        calculate_intensities(&mut contributions);
        assert_eq!(contributions[0].intensity, 0);
        assert_eq!(contributions[1].intensity, 0);
    }

    #[test]
    fn test_calculate_intensities_levels() {
        let mut contributions = vec![
            DailyContribution {
                date: "2024-01-01".to_string(),
                totals: DailyTotals {
                    tokens: 1000,
                    cost: 1.0, // 100% of max
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
            DailyContribution {
                date: "2024-01-02".to_string(),
                totals: DailyTotals {
                    tokens: 800,
                    cost: 0.8, // 80% of max (>= 0.75)
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
            DailyContribution {
                date: "2024-01-03".to_string(),
                totals: DailyTotals {
                    tokens: 600,
                    cost: 0.6, // 60% of max (>= 0.5)
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
            DailyContribution {
                date: "2024-01-04".to_string(),
                totals: DailyTotals {
                    tokens: 300,
                    cost: 0.3, // 30% of max (>= 0.25)
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
            DailyContribution {
                date: "2024-01-05".to_string(),
                totals: DailyTotals {
                    tokens: 100,
                    cost: 0.1, // 10% of max (> 0.0)
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
        ];

        calculate_intensities(&mut contributions);

        assert_eq!(contributions[0].intensity, 4); // 100%
        assert_eq!(contributions[1].intensity, 4); // 80%
        assert_eq!(contributions[2].intensity, 3); // 60%
        assert_eq!(contributions[3].intensity, 2); // 30%
        assert_eq!(contributions[4].intensity, 1); // 10%
    }

    #[test]
    fn test_calculate_intensities_boundary_values() {
        let mut contributions = vec![
            DailyContribution {
                date: "2024-01-01".to_string(),
                totals: DailyTotals {
                    tokens: 1000,
                    cost: 1.0,
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
            DailyContribution {
                date: "2024-01-02".to_string(),
                totals: DailyTotals {
                    tokens: 750,
                    cost: 0.75, // Exactly 0.75 (should be level 4)
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
            DailyContribution {
                date: "2024-01-03".to_string(),
                totals: DailyTotals {
                    tokens: 500,
                    cost: 0.5, // Exactly 0.5 (should be level 3)
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
            DailyContribution {
                date: "2024-01-04".to_string(),
                totals: DailyTotals {
                    tokens: 250,
                    cost: 0.25, // Exactly 0.25 (should be level 2)
                    messages: 1,
                    cost_provenance: None,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
                active_time_ms: None,
            },
        ];

        calculate_intensities(&mut contributions);

        assert_eq!(contributions[0].intensity, 4);
        assert_eq!(contributions[1].intensity, 4); // >= 0.75
        assert_eq!(contributions[2].intensity, 3); // >= 0.5
        assert_eq!(contributions[3].intensity, 2); // >= 0.25
    }

    #[test]
    fn test_aggregate_by_date_preserves_sources() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-01", 2000, 0.10, "gpt-4", "claude"),
        ];

        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].clients.len(), 2);

        // Verify both clients are present
        let client_names: Vec<&str> = result[0]
            .clients
            .iter()
            .map(|s| s.client.as_str())
            .collect();
        assert!(client_names.contains(&"opencode"));
        assert!(client_names.contains(&"claude"));
    }

    #[test]
    fn test_aggregate_by_date_large_dataset() {
        // Test with 100 messages across 10 days
        let mut messages = Vec::new();
        for day in 1..=10 {
            for _msg in 0..10 {
                let date = format!("2024-01-{:02}", day);
                messages.push(mock_unified_message(
                    &date,
                    1000,
                    0.05,
                    "claude-3-5-sonnet",
                    "opencode",
                ));
            }
        }

        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 10);

        // Each day should have 10 messages aggregated
        for contribution in &result {
            assert_eq!(contribution.totals.messages, 10);
            assert_eq!(contribution.totals.tokens, 10000);
            assert!((contribution.totals.cost - 0.5).abs() < 0.0001);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn session_message(
        session_id: &str,
        client: &str,
        provider: &str,
        model: &str,
        date: &str,
        timestamp_ms: i64,
        tokens: TokenBreakdown,
        cost: f64,
    ) -> UnifiedMessage {
        UnifiedMessage {
            client: client.to_string(),
            model_id: model.to_string(),
            provider_id: provider.to_string(),
            session_id: session_id.to_string(),
            workspace_key: None,
            workspace_label: None,
            timestamp: timestamp_ms,
            date: date.to_string(),
            tokens,
            cost,
            cost_source: Default::default(),
            service_tier: None,
            message_count: 1,
            agent: None,
            dedup_key: None,
            session_title: None,
            parent_session_id: None,
            is_turn_start: false,
            model_attribution_conflicted: false,
            duration_ms: None,
            estimate_source: None,
        }
    }

    #[test]
    fn test_aggregate_by_session_empty() {
        assert!(aggregate_by_session(Vec::new()).is_empty());
    }

    #[test]
    fn test_aggregate_by_session_groups_three_sessions() {
        let t = TokenBreakdown {
            input: 100,
            output: 50,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: 0,
            reasoning: 0,
        };
        // 10 rows across 3 sessions.
        let messages = vec![
            session_message(
                "s-a",
                "codex",
                "openai",
                "gpt-5",
                "2026-05-10",
                1_700_000_001_000,
                t.clone(),
                0.01,
            ),
            session_message(
                "s-a",
                "codex",
                "openai",
                "gpt-5",
                "2026-05-10",
                1_700_000_002_000,
                t.clone(),
                0.01,
            ),
            session_message(
                "s-a",
                "codex",
                "openai",
                "gpt-5",
                "2026-05-10",
                1_700_000_003_000,
                t.clone(),
                0.01,
            ),
            session_message(
                "s-a",
                "codex",
                "openai",
                "gpt-5",
                "2026-05-10",
                1_700_000_004_000,
                t.clone(),
                0.01,
            ),
            session_message(
                "s-b",
                "amp",
                "anthropic",
                "claude-haiku-4-5",
                "2026-05-10",
                1_700_000_005_000,
                t.clone(),
                0.02,
            ),
            session_message(
                "s-b",
                "amp",
                "anthropic",
                "claude-haiku-4-5",
                "2026-05-10",
                1_700_000_006_000,
                t.clone(),
                0.02,
            ),
            session_message(
                "s-b",
                "amp",
                "anthropic",
                "claude-haiku-4-5",
                "2026-05-10",
                1_700_000_007_000,
                t.clone(),
                0.02,
            ),
            session_message(
                "s-c",
                "claude",
                "anthropic",
                "claude-sonnet-4-5",
                "2026-05-11",
                1_700_000_100_000,
                t.clone(),
                0.05,
            ),
            session_message(
                "s-c",
                "claude",
                "anthropic",
                "claude-sonnet-4-5",
                "2026-05-11",
                1_700_000_101_000,
                t.clone(),
                0.05,
            ),
            session_message(
                "s-c",
                "claude",
                "anthropic",
                "claude-sonnet-4-5",
                "2026-05-11",
                1_700_000_102_000,
                t.clone(),
                0.05,
            ),
        ];

        let result = aggregate_by_session(messages);
        assert_eq!(result.len(), 3, "expected 3 sessions");

        // Most-recent-first ordering: s-c last_seen=1_700_000_102 wins.
        assert_eq!(result[0].session_id, "s-c");
        assert_eq!(result[1].session_id, "s-b");
        assert_eq!(result[2].session_id, "s-a");

        let s_a = result.iter().find(|s| s.session_id == "s-a").unwrap();
        assert_eq!(s_a.totals.messages, 4);
        assert_eq!(s_a.totals.tokens, 4 * 150); // (100 input + 50 output) * 4
        assert!((s_a.totals.cost - 0.04).abs() < 1e-9);
        assert_eq!(s_a.token_breakdown.input, 400);
        assert_eq!(s_a.token_breakdown.output, 200);
        assert_eq!(s_a.client, "codex");
        assert_eq!(s_a.provider, "openai");
        assert_eq!(s_a.model, "gpt-5");
        // Timestamps converted to seconds.
        assert_eq!(s_a.first_seen, 1_700_000_001);
        assert_eq!(s_a.last_seen, 1_700_000_004);

        let s_b = result.iter().find(|s| s.session_id == "s-b").unwrap();
        assert_eq!(s_b.totals.messages, 3);
        assert!((s_b.totals.cost - 0.06).abs() < 1e-9);

        let s_c = result.iter().find(|s| s.session_id == "s-c").unwrap();
        assert_eq!(s_c.totals.messages, 3);
        assert!((s_c.totals.cost - 0.15).abs() < 1e-9);
    }

    #[test]
    fn test_aggregate_by_session_picks_top_client_by_cost() {
        // Same session_id but two different clients — top-level fields should
        // reflect the client with the larger cost share.
        let small = TokenBreakdown {
            input: 10,
            output: 10,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: 0,
            reasoning: 0,
        };
        let big = TokenBreakdown {
            input: 1000,
            output: 500,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: 0,
            reasoning: 0,
        };
        let messages = vec![
            session_message(
                "shared",
                "amp",
                "anthropic",
                "claude-haiku-4-5",
                "2026-05-10",
                1_700_000_001_000,
                small,
                0.001,
            ),
            session_message(
                "shared",
                "codex",
                "openai",
                "gpt-5",
                "2026-05-10",
                1_700_000_002_000,
                big,
                0.50,
            ),
        ];

        let result = aggregate_by_session(messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].client, "codex");
        assert_eq!(result[0].provider, "openai");
        assert_eq!(result[0].model, "gpt-5");
        // Per-client breakdown should preserve both clients.
        assert_eq!(result[0].clients.len(), 2);
        assert_eq!(result[0].clients[0].client, "codex");
        assert!((result[0].totals.cost - 0.501).abs() < 1e-9);
    }

    #[test]
    fn test_session_contribution_serde_round_trip() {
        let contrib = SessionContribution {
            session_id: "019e1e27-af49-7cd1-89b7-7bad1c3f3be2".to_string(),
            client: "codex".to_string(),
            provider: "openai".to_string(),
            model: "gpt-5".to_string(),
            totals: DailyTotals {
                tokens: 25298,
                cost: 0.0123,
                messages: 12,
                cost_provenance: Some(CostProvenance::unknown()),
            },
            token_breakdown: TokenBreakdown {
                input: 25_251,
                output: 47,
                cache_read: 1_920,
                cache_write: 0,
                cache_write_1h: 0,
                reasoning: 40,
            },
            clients: vec![ClientContribution {
                client: "codex".to_string(),
                model_id: "gpt-5".to_string(),
                provider_id: "openai".to_string(),
                tokens: TokenBreakdown {
                    input: 25_251,
                    output: 47,
                    cache_read: 1_920,
                    cache_write: 0,
                    cache_write_1h: 0,
                    reasoning: 40,
                },
                cost: 0.0123,
                messages: 12,
                cost_provenance: Some(CostProvenance::unknown()),
            }],
            first_seen: 1_715_551_577,
            last_seen: 1_715_551_612,
        };

        let json = serde_json::to_string(&contrib).expect("serialize");
        let parsed: SessionContribution = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, contrib);
        // Spot-check key field is present in JSON.
        assert!(json.contains("\"session_id\":\"019e1e27"));
    }

    #[test]
    fn test_cost_provenance_serde_round_trip_and_backward_compatibility() {
        // Modern serialization round trip
        let prov = CostProvenance::estimated(EstimateSource::Catalog);
        let json = serde_json::to_string(&prov).unwrap();
        assert_eq!(json, r#"{"kind":"estimated","estimateSource":"catalog"}"#);
        let parsed: CostProvenance = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, prov);

        let prov_reported = CostProvenance::provider_reported();
        let json_reported = serde_json::to_string(&prov_reported).unwrap();
        assert_eq!(json_reported, r#"{"kind":"providerReported"}"#);
        let parsed_reported: CostProvenance = serde_json::from_str(&json_reported).unwrap();
        assert_eq!(parsed_reported, prov_reported);

        // Backward compatibility: missing costProvenance in DailyTotals deserializes as unknown
        let legacy_json = r#"{"tokens":100,"cost":0.5,"messages":2}"#;
        let parsed_totals: DailyTotals = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(parsed_totals.cost_provenance(), CostProvenance::unknown());

        // Backward compatibility: missing costProvenance in ClientContribution
        let legacy_client = r#"{"client":"test","model_id":"m","provider_id":"p","tokens":{"input":10,"output":20,"cache_read":0,"cache_write":0,"cache_write_1h":0,"reasoning":0},"cost":0.1,"messages":1}"#;
        let parsed_client: ClientContribution = serde_json::from_str(legacy_client).unwrap();
        assert_eq!(parsed_client.cost_provenance(), CostProvenance::unknown());
    }

    #[test]
    fn test_cost_provenance_single_source_aggregates() {
        fn make_msg_with_source(
            cost_source: crate::sessions::CostSource,
            estimate_source: Option<EstimateSource>,
            cost: f64,
        ) -> UnifiedMessage {
            let mut msg = mock_unified_message("2024-01-01", 100, cost, "model-a", "client-a");
            msg.cost_source = cost_source;
            msg.estimate_source = estimate_source;
            msg
        }

        // Provider reported
        let msgs = vec![make_msg_with_source(
            crate::sessions::CostSource::ProviderReported,
            None,
            1.0,
        )];
        let daily = aggregate_by_date(msgs);
        assert_eq!(
            daily[0].totals.cost_provenance(),
            CostProvenance::provider_reported()
        );
        assert_eq!(
            daily[0].clients[0].cost_provenance(),
            CostProvenance::provider_reported()
        );

        // Estimated (Catalog)
        let msgs = vec![make_msg_with_source(
            crate::sessions::CostSource::Estimated,
            Some(EstimateSource::Catalog),
            1.0,
        )];
        let daily = aggregate_by_date(msgs);
        assert_eq!(
            daily[0].totals.cost_provenance(),
            CostProvenance::estimated(EstimateSource::Catalog)
        );
        assert_eq!(
            daily[0].clients[0].cost_provenance(),
            CostProvenance::estimated(EstimateSource::Catalog)
        );

        // Estimated (Custom)
        let msgs = vec![make_msg_with_source(
            crate::sessions::CostSource::Estimated,
            Some(EstimateSource::Custom),
            1.0,
        )];
        let daily = aggregate_by_date(msgs);
        assert_eq!(
            daily[0].totals.cost_provenance(),
            CostProvenance::estimated(EstimateSource::Custom)
        );
        assert_eq!(
            daily[0].clients[0].cost_provenance(),
            CostProvenance::estimated(EstimateSource::Custom)
        );

        // Estimated (Unknown)
        let msgs = vec![make_msg_with_source(
            crate::sessions::CostSource::Estimated,
            None,
            1.0,
        )];
        let daily = aggregate_by_date(msgs);
        assert_eq!(
            daily[0].totals.cost_provenance(),
            CostProvenance::estimated(EstimateSource::Unknown)
        );

        // Pure unknown (tokens > 0)
        let msgs = vec![make_msg_with_source(
            crate::sessions::CostSource::Unknown,
            None,
            0.0,
        )];
        let daily = aggregate_by_date(msgs);
        assert_eq!(daily[0].totals.cost_provenance(), CostProvenance::unknown());
    }

    #[test]
    fn test_cost_provenance_mixed_aggregates() {
        fn make_msg(
            model: &str,
            cost_source: crate::sessions::CostSource,
            estimate_source: Option<EstimateSource>,
        ) -> UnifiedMessage {
            let mut msg = mock_unified_message("2024-01-01", 100, 0.5, model, "client-a");
            msg.cost_source = cost_source;
            msg.estimate_source = estimate_source;
            msg
        }

        // Provider reported + Estimated
        let msgs = vec![
            make_msg(
                "model-1",
                crate::sessions::CostSource::ProviderReported,
                None,
            ),
            make_msg(
                "model-2",
                crate::sessions::CostSource::Estimated,
                Some(EstimateSource::Catalog),
            ),
        ];
        let daily = aggregate_by_date(msgs);
        assert_eq!(
            daily[0].totals.cost_provenance().kind,
            CostProvenanceKind::Mixed
        );

        // Catalog + Custom estimates = Estimated (Mixed)
        let msgs = vec![
            make_msg(
                "model-1",
                crate::sessions::CostSource::Estimated,
                Some(EstimateSource::Catalog),
            ),
            make_msg(
                "model-2",
                crate::sessions::CostSource::Estimated,
                Some(EstimateSource::Custom),
            ),
        ];
        let daily = aggregate_by_date(msgs);
        assert_eq!(
            daily[0].totals.cost_provenance(),
            CostProvenance::estimated(EstimateSource::Mixed)
        );

        // Provider reported + Unknown
        let msgs = vec![
            make_msg(
                "model-1",
                crate::sessions::CostSource::ProviderReported,
                None,
            ),
            make_msg("model-2", crate::sessions::CostSource::Unknown, None),
        ];
        let daily = aggregate_by_date(msgs);
        assert_eq!(
            daily[0].totals.cost_provenance().kind,
            CostProvenanceKind::Mixed
        );
    }

    #[test]
    fn test_cost_provenance_zero_cost_preservation() {
        // Provider-reported $0 with tokens participates as ProviderReported
        let mut msg1 = mock_unified_message("2024-01-01", 100, 0.0, "model-free", "client-a");
        msg1.cost_source = crate::sessions::CostSource::ProviderReported;
        let daily = aggregate_by_date(vec![msg1]);
        assert_eq!(
            daily[0].totals.cost_provenance(),
            CostProvenance::provider_reported()
        );

        // Explicit custom $0 rate participates as Estimated(Custom)
        let mut msg2 =
            mock_unified_message("2024-01-01", 100, 0.0, "model-custom-free", "client-a");
        msg2.cost_source = crate::sessions::CostSource::Estimated;
        msg2.estimate_source = Some(EstimateSource::Custom);
        let daily = aggregate_by_date(vec![msg2]);
        assert_eq!(
            daily[0].totals.cost_provenance(),
            CostProvenance::estimated(EstimateSource::Custom)
        );

        // Zero-token, zero-cost, unknown cost source does not participate
        let mut msg_empty = mock_unified_message("2024-01-01", 0, 0.0, "model-none", "client-a");
        msg_empty.cost_source = crate::sessions::CostSource::Unknown;
        let mut msg_prov = mock_unified_message("2024-01-01", 100, 0.5, "model-real", "client-a");
        msg_prov.cost_source = crate::sessions::CostSource::ProviderReported;
        let daily = aggregate_by_date(vec![msg_empty, msg_prov]);
        assert_eq!(
            daily[0].totals.cost_provenance(),
            CostProvenance::provider_reported()
        );
    }

    #[test]
    fn test_cost_provenance_order_independent_merge() {
        let mut acc1 = CostProvenanceAccumulator::default();
        let mut acc2 = CostProvenanceAccumulator::default();

        let mut msg_a = mock_unified_message("2024-01-01", 100, 1.0, "m1", "c1");
        msg_a.cost_source = crate::sessions::CostSource::ProviderReported;
        acc1.add_message(&msg_a);

        let mut msg_b = mock_unified_message("2024-01-01", 100, 1.0, "m2", "c2");
        msg_b.cost_source = crate::sessions::CostSource::Estimated;
        msg_b.estimate_source = Some(EstimateSource::Catalog);
        acc2.add_message(&msg_b);

        let mut merge_1_then_2 = acc1;
        merge_1_then_2.merge(acc2);

        let mut merge_2_then_1 = acc2;
        merge_2_then_1.merge(acc1);

        assert_eq!(merge_1_then_2.finish(), merge_2_then_1.finish());
    }

    #[test]
    fn test_cost_provenance_propagates_to_summary_and_years() {
        let mut msg1 = mock_unified_message("2024-01-01", 100, 1.0, "model-1", "client-1");
        msg1.cost_source = crate::sessions::CostSource::Estimated;
        msg1.estimate_source = Some(EstimateSource::Catalog);

        let mut msg2 = mock_unified_message("2024-02-01", 200, 2.0, "model-2", "client-2");
        msg2.cost_source = crate::sessions::CostSource::Estimated;
        msg2.estimate_source = Some(EstimateSource::Catalog);

        let contributions = aggregate_by_date(vec![msg1, msg2]);
        let summary = calculate_summary(&contributions);
        assert_eq!(
            summary.cost_provenance(),
            CostProvenance::estimated(EstimateSource::Catalog)
        );

        let years = calculate_years(&contributions);
        assert_eq!(years.len(), 1);
        assert_eq!(
            years[0].cost_provenance(),
            CostProvenance::estimated(EstimateSource::Catalog)
        );
    }
}
