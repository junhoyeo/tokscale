//! Original graph API, retained for downstream struct literals and function signatures.
//! Use the `V2` types and `_v2` functions to retain cost provenance.

use crate::sessionize;
use crate::{GraphMeta, ReportOptions, TokenBreakdown, UnpricedSubmissionUsage};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DailyTotals {
    pub tokens: i64,
    pub cost: f64,
    pub messages: i32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClientContribution {
    pub client: String,
    pub model_id: String,
    pub provider_id: String,
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub messages: i32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DailyContribution {
    pub date: String,
    pub totals: DailyTotals,
    pub intensity: u8,
    pub token_breakdown: TokenBreakdown,
    pub clients: Vec<ClientContribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_time_ms: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SessionContribution {
    pub session_id: String,
    pub client: String,
    pub provider: String,
    pub model: String,
    pub totals: DailyTotals,
    pub token_breakdown: TokenBreakdown,
    pub clients: Vec<ClientContribution>,
    /// Earliest message timestamp (unix seconds) in the session.
    pub first_seen: i64,
    /// Latest message timestamp (unix seconds) in the session.
    pub last_seen: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct YearSummary {
    pub year: String,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub range_start: String,
    pub range_end: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DataSummary {
    pub total_tokens: i64,
    pub total_cost: f64,
    pub total_days: i32,
    pub active_days: i32,
    pub average_per_day: f64,
    pub max_cost_in_single_day: f64,
    pub clients: Vec<String>,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphResult {
    pub meta: GraphMeta,
    pub summary: DataSummary,
    pub years: Vec<YearSummary>,
    pub contributions: Vec<DailyContribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_metrics: Option<sessionize::TimeMetrics>,
    #[serde(skip)]
    pub unpriced_submission_usage: Vec<UnpricedSubmissionUsage>,
    /// Days whose submitted cost is only a floor because at least one
    /// token-bearing message could not be priced authoritatively.
    ///
    /// The CLI serializes these days as `totals.costIsComplete: false`. Kept
    /// outside `DailyTotals` because it is submission-only protocol metadata,
    /// not part of local report aggregation or exported historical reports.
    #[serde(skip)]
    pub incomplete_cost_dates: BTreeSet<String>,
}

impl From<DailyTotals> for crate::DailyTotalsV2 {
    fn from(value: DailyTotals) -> Self {
        Self {
            tokens: value.tokens,
            cost: value.cost,
            messages: value.messages,
            cost_provenance: None,
        }
    }
}

impl From<crate::DailyTotalsV2> for DailyTotals {
    fn from(value: crate::DailyTotalsV2) -> Self {
        Self {
            tokens: value.tokens,
            cost: value.cost,
            messages: value.messages,
        }
    }
}

impl From<ClientContribution> for crate::ClientContributionV2 {
    fn from(value: ClientContribution) -> Self {
        Self {
            client: value.client,
            model_id: value.model_id,
            provider_id: value.provider_id,
            tokens: value.tokens,
            cost: value.cost,
            messages: value.messages,
            cost_provenance: None,
        }
    }
}

impl From<crate::ClientContributionV2> for ClientContribution {
    fn from(value: crate::ClientContributionV2) -> Self {
        Self {
            client: value.client,
            model_id: value.model_id,
            provider_id: value.provider_id,
            tokens: value.tokens,
            cost: value.cost,
            messages: value.messages,
        }
    }
}

impl From<DailyContribution> for crate::DailyContributionV2 {
    fn from(value: DailyContribution) -> Self {
        Self {
            date: value.date,
            totals: value.totals.into(),
            intensity: value.intensity,
            token_breakdown: value.token_breakdown,
            clients: value.clients.into_iter().map(Into::into).collect(),
            active_time_ms: value.active_time_ms,
        }
    }
}

impl From<crate::DailyContributionV2> for DailyContribution {
    fn from(value: crate::DailyContributionV2) -> Self {
        Self {
            date: value.date,
            totals: value.totals.into(),
            intensity: value.intensity,
            token_breakdown: value.token_breakdown,
            clients: value.clients.into_iter().map(Into::into).collect(),
            active_time_ms: value.active_time_ms,
        }
    }
}

impl From<SessionContribution> for crate::SessionContributionV2 {
    fn from(value: SessionContribution) -> Self {
        Self {
            session_id: value.session_id,
            client: value.client,
            provider: value.provider,
            model: value.model,
            totals: value.totals.into(),
            token_breakdown: value.token_breakdown,
            clients: value.clients.into_iter().map(Into::into).collect(),
            first_seen: value.first_seen,
            last_seen: value.last_seen,
        }
    }
}

impl From<crate::SessionContributionV2> for SessionContribution {
    fn from(value: crate::SessionContributionV2) -> Self {
        Self {
            session_id: value.session_id,
            client: value.client,
            provider: value.provider,
            model: value.model,
            totals: value.totals.into(),
            token_breakdown: value.token_breakdown,
            clients: value.clients.into_iter().map(Into::into).collect(),
            first_seen: value.first_seen,
            last_seen: value.last_seen,
        }
    }
}

impl From<YearSummary> for crate::YearSummaryV2 {
    fn from(value: YearSummary) -> Self {
        Self {
            year: value.year,
            total_tokens: value.total_tokens,
            total_cost: value.total_cost,
            range_start: value.range_start,
            range_end: value.range_end,
            cost_provenance: None,
        }
    }
}

impl From<crate::YearSummaryV2> for YearSummary {
    fn from(value: crate::YearSummaryV2) -> Self {
        Self {
            year: value.year,
            total_tokens: value.total_tokens,
            total_cost: value.total_cost,
            range_start: value.range_start,
            range_end: value.range_end,
        }
    }
}

impl From<DataSummary> for crate::DataSummaryV2 {
    fn from(value: DataSummary) -> Self {
        Self {
            total_tokens: value.total_tokens,
            total_cost: value.total_cost,
            total_days: value.total_days,
            active_days: value.active_days,
            average_per_day: value.average_per_day,
            max_cost_in_single_day: value.max_cost_in_single_day,
            clients: value.clients,
            models: value.models,
            cost_provenance: None,
        }
    }
}

impl From<crate::DataSummaryV2> for DataSummary {
    fn from(value: crate::DataSummaryV2) -> Self {
        Self {
            total_tokens: value.total_tokens,
            total_cost: value.total_cost,
            total_days: value.total_days,
            active_days: value.active_days,
            average_per_day: value.average_per_day,
            max_cost_in_single_day: value.max_cost_in_single_day,
            clients: value.clients,
            models: value.models,
        }
    }
}

impl From<GraphResult> for crate::GraphResultV2 {
    fn from(value: GraphResult) -> Self {
        Self {
            meta: value.meta,
            summary: value.summary.into(),
            years: value.years.into_iter().map(Into::into).collect(),
            contributions: value.contributions.into_iter().map(Into::into).collect(),
            time_metrics: value.time_metrics,
            unpriced_submission_usage: value.unpriced_submission_usage,
            incomplete_cost_dates: value.incomplete_cost_dates,
            recount_proofs: std::collections::BTreeMap::new(),
            source_reads: std::collections::BTreeMap::new(),
        }
    }
}

impl From<crate::GraphResultV2> for GraphResult {
    fn from(value: crate::GraphResultV2) -> Self {
        Self {
            meta: value.meta,
            summary: value.summary.into(),
            years: value.years.into_iter().map(Into::into).collect(),
            contributions: value.contributions.into_iter().map(Into::into).collect(),
            time_metrics: value.time_metrics,
            unpriced_submission_usage: value.unpriced_submission_usage,
            incomplete_cost_dates: value.incomplete_cost_dates,
        }
    }
}

pub async fn generate_graph(options: ReportOptions) -> Result<GraphResult, String> {
    crate::generate_graph_v2(options).await.map(Into::into)
}

pub async fn generate_submission_graph(options: ReportOptions) -> Result<GraphResult, String> {
    crate::generate_submission_graph_v2(options)
        .await
        .map(Into::into)
}

pub async fn generate_local_graph_report(options: ReportOptions) -> Result<GraphResult, String> {
    crate::generate_local_graph_report_v2(options)
        .await
        .map(Into::into)
}

pub async fn generate_source_graph_report(options: ReportOptions) -> Result<GraphResult, String> {
    crate::generate_source_graph_report_v2(options)
        .await
        .map(Into::into)
}
