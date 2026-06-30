//! Session interval derivation and time-based metrics.

use crate::sessions::UnifiedMessage;
use crate::TokenBreakdown;
use std::collections::HashMap;

/// Default idle gap threshold: 3 minutes (ms).
pub const DEFAULT_IDLE_GAP_MS: i64 = 3 * 60 * 1000;

/// A derived session interval representing one continuous usage session.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionInterval {
    pub client: String,
    pub session_id: String,
    /// First message timestamp (Unix ms)
    pub start_ts: i64,
    /// Last message timestamp (Unix ms)
    pub end_ts: i64,
    /// Wall-clock duration: end_ts - start_ts
    pub wall_duration_ms: i64,
    /// Active duration excluding idle gaps beyond the threshold
    pub active_duration_ms: i64,
    pub message_count: i32,
    pub tokens: TokenBreakdown,
    pub cost: f64,
}

/// Time-based usage metrics computed from session intervals.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TimeMetrics {
    /// Total active usage time across all sessions (ms)
    pub total_active_time_ms: i64,
    /// Total wall-clock usage time across all sessions (ms)
    pub total_wall_time_ms: i64,
    /// Longest single session active duration (ms)
    pub longest_continuous_ms: i64,
    /// Peak number of sessions overlapping at the same time
    pub max_concurrent_sessions: u32,
    /// Total number of derived session intervals
    pub session_count: u32,
}

#[derive(Debug)]
struct SessionMessageSpan<'a> {
    msg: &'a UnifiedMessage,
    start_ts: i64,
    end_ts: i64,
}

#[derive(Debug)]
struct SessionBlock {
    start_ts: i64,
    end_ts: i64,
    tokens: TokenBreakdown,
    cost: f64,
    message_count: i32,
}

impl SessionBlock {
    fn new(span: &SessionMessageSpan<'_>) -> Self {
        let mut block = Self {
            start_ts: span.start_ts,
            end_ts: span.end_ts,
            tokens: TokenBreakdown::default(),
            cost: 0.0,
            message_count: 0,
        };
        block.add(span);
        block
    }

    fn add(&mut self, span: &SessionMessageSpan<'_>) {
        self.end_ts = self.end_ts.max(span.end_ts);
        self.tokens.input += span.msg.tokens.input;
        self.tokens.output += span.msg.tokens.output;
        self.tokens.cache_read += span.msg.tokens.cache_read;
        self.tokens.cache_write += span.msg.tokens.cache_write;
        self.tokens.reasoning += span.msg.tokens.reasoning;
        self.cost += span.msg.cost;
        self.message_count += span.msg.message_count.max(1);
    }
}

/// Derive session intervals from unified messages.
///
/// Groups messages by `(client, session_id)`, sorts each group by timestamp,
/// and computes per-session start/end/duration/active-time.
///
/// `idle_gap_ms` controls how much silence between messages is still counted
/// as "active". Time gaps exceeding this threshold are excluded from
/// `active_duration_ms` by splitting a source session into separate active
/// intervals.
pub fn sessionize(messages: &[UnifiedMessage], idle_gap_ms: i64) -> Vec<SessionInterval> {
    if messages.is_empty() {
        return Vec::new();
    }

    // Group by (client, session_id)
    let mut groups: HashMap<(&str, &str), Vec<&UnifiedMessage>> = HashMap::new();
    for msg in messages {
        if msg.timestamp <= 0 {
            continue;
        }
        groups
            .entry((&msg.client, &msg.session_id))
            .or_default()
            .push(msg);
    }

    let mut intervals: Vec<SessionInterval> = Vec::with_capacity(groups.len());

    for ((client, session_id), mut msgs) in groups {
        msgs.sort_unstable_by_key(|m| m.timestamp);

        let spans: Vec<SessionMessageSpan<'_>> = msgs
            .into_iter()
            .map(|msg| {
                let duration_ms = msg
                    .duration_ms
                    .filter(|duration| *duration > 0)
                    .unwrap_or(0);
                SessionMessageSpan {
                    msg,
                    start_ts: msg.timestamp,
                    end_ts: msg.timestamp.saturating_add(duration_ms),
                }
            })
            .collect();

        let mut blocks: Vec<SessionBlock> = Vec::new();
        for span in spans {
            match blocks.last_mut() {
                Some(block) if span.start_ts <= block.end_ts.saturating_add(idle_gap_ms) => {
                    block.add(&span);
                }
                _ => blocks.push(SessionBlock::new(&span)),
            }
        }

        for block in blocks {
            let wall_duration_ms = block.end_ts.saturating_sub(block.start_ts);

            intervals.push(SessionInterval {
                client: client.to_string(),
                session_id: session_id.to_string(),
                start_ts: block.start_ts,
                end_ts: block.end_ts,
                wall_duration_ms,
                active_duration_ms: wall_duration_ms,
                message_count: block.message_count,
                tokens: block.tokens,
                cost: block.cost,
            });
        }
    }

    // Sort by start time for downstream consumers
    intervals.sort_unstable_by_key(|s| s.start_ts);
    intervals
}

/// Compute time-based metrics from session intervals.
///
/// - `total_active_time_ms`: sum of all `active_duration_ms`
/// - `total_wall_time_ms`: sum of all `wall_duration_ms`
/// - `longest_continuous_ms`: longest merged activity window across ALL sessions
///   (using the idle gap threshold to merge overlapping/adjacent activity)
/// - `max_concurrent_sessions`: peak overlap of session wall-clock intervals
pub fn compute_time_metrics(intervals: &[SessionInterval], _idle_gap_ms: i64) -> TimeMetrics {
    if intervals.is_empty() {
        return TimeMetrics {
            total_active_time_ms: 0,
            total_wall_time_ms: 0,
            longest_continuous_ms: 0,
            max_concurrent_sessions: 0,
            session_count: 0,
        };
    }

    let total_active_time_ms: i64 = intervals.iter().map(|s| s.active_duration_ms).sum();
    let total_wall_time_ms: i64 = intervals.iter().map(|s| s.wall_duration_ms).sum();
    let session_count = intervals.len() as u32;

    // --- Longest continuous usage ---
    // Collect all session [start, end] as activity windows, merge overlapping
    // ones (with idle_gap_ms tolerance), find the longest merged span.
    // Use active_duration_ms instead of wall-clock span to exclude idle gaps
    // within sessions from inflating the metric.
    let longest_continuous_ms = {
        let mut windows: Vec<(i64, i64)> = intervals
            .iter()
            .filter(|s| s.start_ts > 0 && s.active_duration_ms > 0)
            .map(|s| (s.start_ts, s.start_ts + s.active_duration_ms))
            .collect();
        windows.sort_unstable_by_key(|w| w.0);

        let mut longest: i64 = 0;
        if let Some(&first) = windows.first() {
            let mut merged_start = first.0;
            let mut merged_end = first.1;

            for &(start, end) in &windows[1..] {
                if start <= merged_end + _idle_gap_ms {
                    // Overlapping or within idle gap tolerance — extend
                    merged_end = merged_end.max(end);
                } else {
                    // Gap too large — finalize previous window
                    longest = longest.max(merged_end - merged_start);
                    merged_start = start;
                    merged_end = end;
                }
            }
            longest = longest.max(merged_end - merged_start);
        }
        longest
    };

    // --- Max concurrent sessions ---
    let max_concurrent_sessions = compute_max_concurrent(intervals);

    TimeMetrics {
        total_active_time_ms,
        total_wall_time_ms,
        longest_continuous_ms,
        max_concurrent_sessions,
        session_count,
    }
}

/// Sweep-line algorithm to find peak concurrent sessions.
fn compute_max_concurrent(intervals: &[SessionInterval]) -> u32 {
    if intervals.is_empty() {
        return 0;
    }

    let mut events: Vec<(i64, i32)> = Vec::with_capacity(intervals.len() * 2);
    for s in intervals {
        if s.start_ts <= 0 {
            continue;
        }
        events.push((s.start_ts, 1));
        // For zero-duration sessions (start == end), push end as start+1
        // so the +1 event is processed before the -1 event at the same logical point
        let end = if s.end_ts <= s.start_ts {
            s.start_ts + 1
        } else {
            s.end_ts
        };
        events.push((end, -1));
    }

    // Sort by time; ties broken by start (+1) before end (-1) so concurrent
    // sessions at the same timestamp are counted together
    events.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    let mut max_concurrent: u32 = 0;
    let mut current: i32 = 0;

    for (_, delta) in events {
        current += delta;
        if current > max_concurrent as i32 {
            max_concurrent = current as u32;
        }
    }

    max_concurrent
}

/// Compute per-day active time (ms) from session intervals.
///
/// For each interval, distributes its `active_duration_ms` proportionally
/// across local-time days for the active timezone. Single-day sessions get their full active
/// time assigned to that day.
pub fn compute_daily_active_time(
    intervals: &[SessionInterval],
) -> std::collections::HashMap<String, i64> {
    compute_daily_active_time_with_timezone(intervals, &chrono::Local)
}

fn compute_daily_active_time_with_timezone<Tz>(
    intervals: &[SessionInterval],
    timezone: &Tz,
) -> std::collections::HashMap<String, i64>
where
    Tz: chrono::TimeZone,
{
    use std::collections::HashMap;

    let mut daily: HashMap<String, i64> = HashMap::new();

    for interval in intervals {
        if interval.active_duration_ms <= 0 {
            continue;
        }

        let start_date = match local_date(interval.start_ts, timezone) {
            Some(date) => date,
            None => continue,
        };
        let end_date = match local_date(interval.end_ts, timezone) {
            Some(date) => date,
            None => continue,
        };

        let wall = interval.wall_duration_ms.max(1);
        let mut day = start_date;

        loop {
            let day_key = day.format("%Y-%m-%d").to_string();
            let Some(day_start) = local_day_start(day, timezone) else {
                // DST gap: skip the rest of this interval when local midnight
                // is not representable for the active timezone.
                break;
            };

            let Some(next_day) = day.succ_opt() else {
                break;
            };
            let Some(next_day_start) = local_day_start(next_day, timezone) else {
                // DST gap: skip the rest of this interval when the next local
                // midnight is not representable for the active timezone.
                break;
            };

            let overlap_start = interval.start_ts.max(day_start);
            let overlap_end = interval.end_ts.min(next_day_start);
            let overlap = (overlap_end - overlap_start).max(0);
            let proportion = overlap as f64 / wall as f64;
            let active_for_day = (interval.active_duration_ms as f64 * proportion) as i64;

            if active_for_day > 0 {
                *daily.entry(day_key).or_default() += active_for_day;
            }

            if day == end_date {
                break;
            }

            if let Some(next) = day.succ_opt() {
                day = next;
            } else {
                break;
            }
        }
    }

    daily
}

fn local_date<Tz>(timestamp_ms: i64, timezone: &Tz) -> Option<chrono::NaiveDate>
where
    Tz: chrono::TimeZone,
{
    timezone
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|datetime| datetime.date_naive())
}

fn local_day_start<Tz>(date: chrono::NaiveDate, timezone: &Tz) -> Option<i64>
where
    Tz: chrono::TimeZone,
{
    let midnight = date.and_time(chrono::NaiveTime::MIN);
    match timezone.from_local_datetime(&midnight) {
        chrono::LocalResult::Single(datetime) => Some(datetime.timestamp_millis()),
        chrono::LocalResult::Ambiguous(earliest, _) => Some(earliest.timestamp_millis()),
        chrono::LocalResult::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};

    fn make_msg(client: &str, session_id: &str, timestamp: i64) -> UnifiedMessage {
        UnifiedMessage {
            client: client.to_string(),
            model_id: "test-model".to_string(),
            provider_id: "test-provider".to_string(),
            session_id: session_id.to_string(),
            workspace_key: None,
            workspace_label: None,
            timestamp,
            date: "2024-01-01".to_string(),
            tokens: TokenBreakdown {
                input: 100,
                output: 50,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost: 0.01,
            cost_source: Default::default(),
            message_count: 1,
            agent: None,
            dedup_key: None,
            is_turn_start: false,
            duration_ms: None,
        }
    }

    fn make_timed_msg(
        client: &str,
        session_id: &str,
        timestamp: i64,
        duration_ms: i64,
    ) -> UnifiedMessage {
        let mut message = make_msg(client, session_id, timestamp);
        message.duration_ms = Some(duration_ms);
        message
    }

    #[test]
    fn test_sessionize_empty() {
        let result = sessionize(&[], DEFAULT_IDLE_GAP_MS);
        assert!(result.is_empty());
    }

    #[test]
    fn test_sessionize_single_message() {
        let msgs = vec![make_msg("opencode", "ses1", 1000000)];
        let result = sessionize(&msgs, DEFAULT_IDLE_GAP_MS);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].wall_duration_ms, 0);
        assert_eq!(result[0].active_duration_ms, 0);
        assert_eq!(result[0].message_count, 1);
    }

    #[test]
    fn test_sessionize_counts_single_timed_message_duration() {
        let msgs = vec![make_timed_msg("opencode", "ses1", 1_000_000, 45_000)];
        let result = sessionize(&msgs, DEFAULT_IDLE_GAP_MS);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_ts, 1_000_000);
        assert_eq!(result[0].end_ts, 1_045_000);
        assert_eq!(result[0].wall_duration_ms, 45_000);
        assert_eq!(result[0].active_duration_ms, 45_000);
    }

    #[test]
    fn test_sessionize_splits_timed_messages_across_idle_gap() {
        let msgs = vec![
            make_timed_msg("opencode", "ses1", 1_000_000, 60_000),
            make_timed_msg("opencode", "ses1", 1_000_000 + 10 * 60_000, 30_000),
        ];

        let result = sessionize(&msgs, DEFAULT_IDLE_GAP_MS);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].start_ts, 1_000_000);
        assert_eq!(result[0].end_ts, 1_060_000);
        assert_eq!(result[0].active_duration_ms, 60_000);
        assert_eq!(result[1].start_ts, 1_600_000);
        assert_eq!(result[1].end_ts, 1_630_000);
        assert_eq!(result[1].active_duration_ms, 30_000);
    }

    #[test]
    fn test_sessionize_continuous_session() {
        // 5 messages, each 1 minute apart (within 3-min threshold)
        let msgs: Vec<UnifiedMessage> = (0..5)
            .map(|i| make_msg("opencode", "ses1", 1000000 + i * 60_000))
            .collect();

        let result = sessionize(&msgs, DEFAULT_IDLE_GAP_MS);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].wall_duration_ms, 4 * 60_000);
        assert_eq!(result[0].active_duration_ms, 4 * 60_000); // all gaps <= 3min
        assert_eq!(result[0].message_count, 5);
    }

    #[test]
    fn test_sessionize_with_idle_gap() {
        // 3 messages: first two 1 min apart, then 5 min gap (exceeds 3-min threshold)
        let msgs = vec![
            make_msg("opencode", "ses1", 1000000),
            make_msg("opencode", "ses1", 1000000 + 60_000),
            make_msg("opencode", "ses1", 1000000 + 60_000 + 5 * 60_000),
        ];

        let result = sessionize(&msgs, DEFAULT_IDLE_GAP_MS);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].active_duration_ms, 60_000);
        assert_eq!(result[0].wall_duration_ms, 60_000);
        assert_eq!(result[0].message_count, 2);
        assert_eq!(result[1].active_duration_ms, 0);
        assert_eq!(result[1].wall_duration_ms, 0);
        assert_eq!(result[1].message_count, 1);
    }

    #[test]
    fn test_sessionize_multiple_sessions() {
        let msgs = vec![
            make_msg("opencode", "ses1", 1000000),
            make_msg("opencode", "ses1", 1060000),
            make_msg("claude", "ses2", 1000000),
            make_msg("claude", "ses2", 1120000),
        ];

        let result = sessionize(&msgs, DEFAULT_IDLE_GAP_MS);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_sessionize_skips_zero_timestamp() {
        let msgs = vec![
            make_msg("opencode", "ses1", 0),
            make_msg("opencode", "ses1", 1000000),
        ];

        let result = sessionize(&msgs, DEFAULT_IDLE_GAP_MS);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].message_count, 1); // only the non-zero one
    }

    #[test]
    fn test_compute_time_metrics_empty() {
        let metrics = compute_time_metrics(&[], DEFAULT_IDLE_GAP_MS);
        assert_eq!(metrics.total_active_time_ms, 0);
        assert_eq!(metrics.longest_continuous_ms, 0);
        assert_eq!(metrics.max_concurrent_sessions, 0);
        assert_eq!(metrics.session_count, 0);
    }

    #[test]
    fn test_max_concurrent_sessions() {
        // Two overlapping sessions
        let intervals = vec![
            SessionInterval {
                client: "opencode".to_string(),
                session_id: "ses1".to_string(),
                start_ts: 1000,
                end_ts: 5000,
                wall_duration_ms: 4000,
                active_duration_ms: 4000,
                message_count: 3,
                tokens: TokenBreakdown::default(),
                cost: 0.0,
            },
            SessionInterval {
                client: "claude".to_string(),
                session_id: "ses2".to_string(),
                start_ts: 3000,
                end_ts: 7000,
                wall_duration_ms: 4000,
                active_duration_ms: 4000,
                message_count: 3,
                tokens: TokenBreakdown::default(),
                cost: 0.0,
            },
        ];

        let metrics = compute_time_metrics(&intervals, DEFAULT_IDLE_GAP_MS);
        assert_eq!(metrics.max_concurrent_sessions, 2);
    }

    #[test]
    fn test_max_concurrent_non_overlapping() {
        // Two non-overlapping sessions
        let intervals = vec![
            SessionInterval {
                client: "opencode".to_string(),
                session_id: "ses1".to_string(),
                start_ts: 1000,
                end_ts: 3000,
                wall_duration_ms: 2000,
                active_duration_ms: 2000,
                message_count: 2,
                tokens: TokenBreakdown::default(),
                cost: 0.0,
            },
            SessionInterval {
                client: "claude".to_string(),
                session_id: "ses2".to_string(),
                start_ts: 5000,
                end_ts: 7000,
                wall_duration_ms: 2000,
                active_duration_ms: 2000,
                message_count: 2,
                tokens: TokenBreakdown::default(),
                cost: 0.0,
            },
        ];

        let metrics = compute_time_metrics(&intervals, DEFAULT_IDLE_GAP_MS);
        assert_eq!(metrics.max_concurrent_sessions, 1);
    }

    #[test]
    fn test_longest_continuous_is_max_session_active_duration() {
        let intervals = vec![
            SessionInterval {
                client: "opencode".to_string(),
                session_id: "ses1".to_string(),
                start_ts: 1000,
                end_ts: 5000,
                wall_duration_ms: 4000,
                active_duration_ms: 3000,
                message_count: 3,
                tokens: TokenBreakdown::default(),
                cost: 0.0,
            },
            SessionInterval {
                client: "claude".to_string(),
                session_id: "ses2".to_string(),
                start_ts: 3000,
                end_ts: 8000,
                wall_duration_ms: 5000,
                active_duration_ms: 5000,
                message_count: 3,
                tokens: TokenBreakdown::default(),
                cost: 0.0,
            },
        ];

        let metrics = compute_time_metrics(&intervals, DEFAULT_IDLE_GAP_MS);
        assert_eq!(metrics.longest_continuous_ms, 7000);
    }

    #[test]
    fn test_longest_continuous_picks_max_active() {
        let intervals = vec![
            SessionInterval {
                client: "opencode".to_string(),
                session_id: "ses1".to_string(),
                start_ts: 1,
                end_ts: 60_000,
                wall_duration_ms: 60_000,
                active_duration_ms: 60_000,
                message_count: 3,
                tokens: TokenBreakdown::default(),
                cost: 0.0,
            },
            SessionInterval {
                client: "opencode".to_string(),
                session_id: "ses2".to_string(),
                start_ts: 60_000 + 2 * 60_000,
                end_ts: 60_000 + 2 * 60_000 + 120_000,
                wall_duration_ms: 120_000,
                active_duration_ms: 120_000,
                message_count: 3,
                tokens: TokenBreakdown::default(),
                cost: 0.0,
            },
        ];

        let metrics = compute_time_metrics(&intervals, DEFAULT_IDLE_GAP_MS);
        assert_eq!(metrics.longest_continuous_ms, 299_999);
    }

    #[test]
    fn test_longest_continuous_single_session() {
        let intervals = vec![
            SessionInterval {
                client: "opencode".to_string(),
                session_id: "ses1".to_string(),
                start_ts: 1000,
                end_ts: 61_000,
                wall_duration_ms: 60_000,
                active_duration_ms: 60_000,
                message_count: 3,
                tokens: TokenBreakdown::default(),
                cost: 0.0,
            },
            SessionInterval {
                client: "opencode".to_string(),
                session_id: "ses2".to_string(),
                start_ts: 61_000 + 10 * 60_000,
                end_ts: 61_000 + 10 * 60_000 + 120_000,
                wall_duration_ms: 120_000,
                active_duration_ms: 120_000,
                message_count: 5,
                tokens: TokenBreakdown::default(),
                cost: 0.0,
            },
        ];

        let metrics = compute_time_metrics(&intervals, DEFAULT_IDLE_GAP_MS);
        assert_eq!(metrics.longest_continuous_ms, 120_000);
    }

    #[test]
    fn test_compute_daily_active_time_matches_local_day_boundaries_for_fixed_offset() {
        let interval = SessionInterval {
            client: "trae".to_string(),
            session_id: "session-local-boundary".to_string(),
            start_ts: FixedOffset::east_opt(9 * 3600)
                .unwrap()
                .with_ymd_and_hms(2026, 1, 1, 23, 30, 0)
                .single()
                .unwrap()
                .timestamp_millis(),
            end_ts: FixedOffset::east_opt(9 * 3600)
                .unwrap()
                .with_ymd_and_hms(2026, 1, 2, 0, 30, 0)
                .single()
                .unwrap()
                .timestamp_millis(),
            wall_duration_ms: 3_600_000,
            active_duration_ms: 3_600_000,
            message_count: 2,
            tokens: TokenBreakdown::default(),
            cost: 0.0,
        };

        let daily = compute_daily_active_time_with_timezone(
            &[interval],
            &FixedOffset::east_opt(9 * 3600).unwrap(),
        );

        assert_eq!(daily.get("2026-01-01"), Some(&1_800_000));
        assert_eq!(daily.get("2026-01-02"), Some(&1_800_000));
        assert_eq!(daily.len(), 2);
    }

    #[test]
    fn test_daily_active_time_keeps_idle_separated_timed_blocks_on_their_days() {
        let timezone = FixedOffset::east_opt(0).unwrap();
        let first_start = timezone
            .with_ymd_and_hms(2026, 1, 1, 23, 58, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let second_start = timezone
            .with_ymd_and_hms(2026, 1, 2, 0, 10, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let msgs = vec![
            make_timed_msg("opencode", "ses1", first_start, 120_000),
            make_timed_msg("opencode", "ses1", second_start, 60_000),
        ];

        let intervals = sessionize(&msgs, DEFAULT_IDLE_GAP_MS);
        let daily = compute_daily_active_time_with_timezone(&intervals, &timezone);

        assert_eq!(intervals.len(), 2);
        assert_eq!(daily.get("2026-01-01"), Some(&120_000));
        assert_eq!(daily.get("2026-01-02"), Some(&60_000));
        assert_eq!(daily.len(), 2);
    }
}
