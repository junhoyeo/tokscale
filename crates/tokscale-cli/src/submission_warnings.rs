use std::collections::{HashMap, HashSet};

struct DailyDecrease<'a> {
    date: &'a str,
    client: &'a str,
    stored: &'a str,
    incoming: &'a str,
}

fn daily_decrease(warning: &str) -> Option<DailyDecrease<'_>> {
    let (date, rest) = warning.strip_prefix("Day ")?.split_once(": Preserved ")?;
    let (client, rest) = rest.split_once(" because this same-device resubmit would reduce ")?;
    let (stored, incoming) = rest.strip_suffix('.')?.split_once(" tokens to ")?;
    let tokens = |value: &str| {
        !value.is_empty()
            && value.bytes().any(|byte| byte.is_ascii_digit())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b',')
    };
    if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()
        || client.is_empty()
        || !client
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || !tokens(stored)
        || !tokens(incoming)
    {
        return None;
    }
    Some(DailyDecrease {
        date,
        client,
        stored,
        incoming,
    })
}

/// Compact only the known daily-decrease warning; preserve every value and
/// leave other server messages unchanged.
pub(crate) fn compact(warnings: &[String]) -> Vec<String> {
    const MIN_GROUP: usize = 4;
    const DETAILS_PER_LINE: usize = 2;

    let mut groups: HashMap<&str, Vec<DailyDecrease<'_>>> = HashMap::new();
    for warning in warnings {
        if let Some(decrease) = daily_decrease(warning) {
            groups.entry(decrease.client).or_default().push(decrease);
        }
    }

    let mut lines = Vec::new();
    let mut emitted = HashSet::new();
    for warning in warnings {
        if let Some(decrease) = daily_decrease(warning) {
            let group = &groups[decrease.client];
            if group.len() >= MIN_GROUP {
                if emitted.insert(decrease.client) {
                    lines.push(format!(
                        "- Preserved {}: {} daily token decreases on this device (stored -> incoming):",
                        decrease.client,
                        group.len()
                    ));
                    for chunk in group.chunks(DETAILS_PER_LINE) {
                        let details = chunk
                            .iter()
                            .map(|item| {
                                format!("{}: {} -> {}", item.date, item.stored, item.incoming)
                            })
                            .collect::<Vec<_>>()
                            .join(" | ");
                        lines.push(format!("  {details}"));
                    }
                }
                continue;
            }
        }
        lines.push(format!("- {warning}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_unrecognized_warnings_are_unchanged() {
        let warnings = vec![
            "Day 2026-05-01: Preserved hermes because this same-device resubmit would reduce 1,000 tokens to 900.".to_string(),
            "Day 2026-05-02: Preserved hermes because this same-device resubmit would reduce 1,000 tokens to 900.".to_string(),
            "Day 2026-05-03: Preserved hermes because this same-device resubmit would reduce 1,000 tokens to 900.".to_string(),
            "Day 2026-05-02: Preserved hermes because this same-device resubmit would reduce unknown tokens to 1.".to_string(),
            "Day 2026-02-30: Preserved hermes because this same-device resubmit would reduce 100 tokens to 1.".to_string(),
            "Day 2026-05-02: Preserved hermes because this same-device resubmit would reduce 100 tokens to 1. Extra detail.".to_string(),
            "Ignored Amp changes.".to_string(),
        ];
        assert_eq!(
            compact(&warnings),
            warnings
                .iter()
                .map(|warning| format!("- {warning}"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn groups_each_client_without_losing_dates_or_values() {
        let warnings: Vec<String> = ["hermes", "grok"]
            .into_iter()
            .flat_map(|client| {
                (1..=4).map(move |day| format!("Day 2026-05-{day:02}: Preserved {client} because this same-device resubmit would reduce 1,{day}00 tokens to {day}."))
            })
            .collect();
        let lines = compact(&warnings);
        assert_eq!(lines.len(), 6);
        for client in ["hermes", "grok"] {
            assert!(lines
                .iter()
                .any(|line| line.contains(&format!("Preserved {client}: 4 daily"))));
        }
        for day in 1..=4 {
            let detail = format!("2026-05-{day:02}: 1,{day}00 -> {day}");
            assert_eq!(
                lines.iter().filter(|line| line.contains(&detail)).count(),
                2
            );
        }
    }
}
