use std::collections::{HashMap, HashSet};

enum WarningItem<'a> {
    Decrease {
        date: &'a str,
        client: &'a str,
        stored: &'a str,
        incoming: &'a str,
    },
    Disappeared {
        date: &'a str,
        client: &'a str,
        kept: &'a str,
    },
    ActiveTime {
        date: &'a str,
        stored: &'a str,
        incoming: &'a str,
    },
}

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
enum GroupKey<'a> {
    Decrease(&'a str),
    Disappeared(&'a str),
    ActiveTime,
}

impl<'a> WarningItem<'a> {
    fn group_key(&self) -> GroupKey<'a> {
        match self {
            Self::Decrease { client, .. } => GroupKey::Decrease(client),
            Self::Disappeared { client, .. } => GroupKey::Disappeared(client),
            Self::ActiveTime { .. } => GroupKey::ActiveTime,
        }
    }

    fn format_detail(&self) -> String {
        match self {
            Self::Decrease {
                date,
                stored,
                incoming,
                ..
            } => format!("{date}: {stored} -> {incoming}"),
            Self::Disappeared { date, kept, .. } => format!("{date}: {kept}"),
            Self::ActiveTime {
                date,
                stored,
                incoming,
            } => format!("{date}: {stored} -> {incoming}"),
        }
    }
}

fn is_valid_client(client: &str) -> bool {
    !client.is_empty()
        && client
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn is_valid_numeric(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().any(|byte| byte.is_ascii_digit())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b',')
}

fn parse_daily_decrease(warning: &str) -> Option<WarningItem<'_>> {
    let (date, rest) = warning.strip_prefix("Day ")?.split_once(": Preserved ")?;
    let (client, rest) = rest.split_once(" because this same-device resubmit would reduce ")?;
    let (stored, incoming) = rest.strip_suffix('.')?.split_once(" tokens to ")?;
    if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()
        || !is_valid_client(client)
        || !is_valid_numeric(stored)
        || !is_valid_numeric(incoming)
    {
        return None;
    }
    Some(WarningItem::Decrease {
        date,
        client,
        stored,
        incoming,
    })
}

fn parse_disappeared(warning: &str) -> Option<WarningItem<'_>> {
    let (date, rest) = warning.strip_prefix("Day ")?.split_once(": Preserved ")?;
    let (client, rest) =
        rest.split_once(" because it disappeared from this same-device resubmit; kept ")?;
    let kept = rest.strip_suffix(" tokens.")?;
    if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()
        || !is_valid_client(client)
        || !is_valid_numeric(kept)
    {
        return None;
    }
    Some(WarningItem::Disappeared { date, client, kept })
}

fn parse_active_time(warning: &str) -> Option<WarningItem<'_>> {
    let (date, rest) = warning.strip_prefix("Day ")?.split_once(": Preserved ")?;
    let (stored, rest) =
        rest.split_once("ms active time because this same-device resubmit reported only ")?;
    let incoming = rest.strip_suffix("ms.")?;
    if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()
        || !is_valid_numeric(stored)
        || !is_valid_numeric(incoming)
    {
        return None;
    }
    Some(WarningItem::ActiveTime {
        date,
        stored,
        incoming,
    })
}

fn parse_warning(warning: &str) -> Option<WarningItem<'_>> {
    parse_daily_decrease(warning)
        .or_else(|| parse_disappeared(warning))
        .or_else(|| parse_active_time(warning))
}

fn format_header(key: GroupKey<'_>, count: usize) -> String {
    match key {
        GroupKey::Decrease(client) => format!(
            "- Preserved {client}: {count} daily token decreases on this device (stored -> incoming):"
        ),
        GroupKey::Disappeared(client) => format!(
            "- Preserved {client}: {count} days missing from this device's resubmit (date: kept tokens):"
        ),
        GroupKey::ActiveTime => format!(
            "- Preserved active time: {count} daily decreases on this device (stored -> incoming ms):"
        ),
    }
}

/// Compact known preservation warnings (token decrease, missing clients, and active-time);
/// preserve every value and leave other server messages unchanged.
pub(crate) fn compact(warnings: &[String]) -> Vec<String> {
    const MIN_GROUP: usize = 4;
    const DETAILS_PER_LINE: usize = 2;

    let mut groups: HashMap<GroupKey<'_>, Vec<WarningItem<'_>>> = HashMap::new();
    for warning in warnings {
        if let Some(item) = parse_warning(warning) {
            groups.entry(item.group_key()).or_default().push(item);
        }
    }

    let mut lines = Vec::new();
    let mut emitted = HashSet::new();
    for warning in warnings {
        if let Some(item) = parse_warning(warning) {
            let key = item.group_key();
            let group = &groups[&key];
            if group.len() >= MIN_GROUP {
                if emitted.insert(key) {
                    lines.push(format_header(key, group.len()));
                    for chunk in group.chunks(DETAILS_PER_LINE) {
                        let details = chunk
                            .iter()
                            .map(|entry| entry.format_detail())
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
            "Day 2026-03-27: Preserved opencode because it disappeared from this same-device resubmit; kept 58,503,692 tokens.".to_string(),
            "Day 2026-03-28: Preserved opencode because it disappeared from this same-device resubmit; kept 10,000 tokens.".to_string(),
            "Day 2026-03-29: Preserved opencode because it disappeared from this same-device resubmit; kept 20,000 tokens.".to_string(),
            "Day 2026-03-30: Preserved opencode because it disappeared from this same-device resubmit; kept unknown tokens.".to_string(),
            "Day 2026-02-30: Preserved opencode because it disappeared from this same-device resubmit; kept 50 tokens.".to_string(),
            "Day 2026-10-14: Preserved 116300880ms active time because this same-device resubmit reported only 20206166ms.".to_string(),
            "Day 2026-10-15: Preserved 90000000ms active time because this same-device resubmit reported only 10000000ms.".to_string(),
            "Day 2026-10-16: Preserved 80000000ms active time because this same-device resubmit reported only 5000000ms.".to_string(),
            "Day 2026-10-17: Preserved invalidms active time because this same-device resubmit reported only 10ms.".to_string(),
            "Day 2026-02-30: Preserved 100ms active time because this same-device resubmit reported only 10ms.".to_string(),
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

    #[test]
    fn groups_disappeared_clients_per_client() {
        let warnings: Vec<String> = ["opencode", "codex"]
            .into_iter()
            .flat_map(|client| {
                (1..=4).map(move |day| {
                    format!("Day 2026-03-{day:02}: Preserved {client} because it disappeared from this same-device resubmit; kept {day},503,692 tokens.")
                })
            })
            .collect();
        let lines = compact(&warnings);
        assert_eq!(lines.len(), 6);
        for client in ["opencode", "codex"] {
            assert!(lines.iter().any(|line| line.contains(&format!(
                "Preserved {client}: 4 days missing from this device's resubmit"
            ))));
        }
        for day in 1..=4 {
            let detail = format!("2026-03-{day:02}: {day},503,692");
            assert_eq!(
                lines.iter().filter(|line| line.contains(&detail)).count(),
                2
            );
        }
    }

    #[test]
    fn groups_active_time_preservation_warnings() {
        let warnings: Vec<String> = (1..=4)
            .map(|day| {
                format!("Day 2026-10-{day:02}: Preserved {day}16300880ms active time because this same-device resubmit reported only {day}0206166ms.")
            })
            .collect();
        let lines = compact(&warnings);
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().any(|line| line.contains(
            "Preserved active time: 4 daily decreases on this device (stored -> incoming ms):"
        )));
        for day in 1..=4 {
            let detail = format!("2026-10-{day:02}: {day}16300880 -> {day}0206166");
            assert_eq!(
                lines.iter().filter(|line| line.contains(&detail)).count(),
                1
            );
        }
    }

    #[test]
    fn mixed_preservation_warnings_keep_relative_order_and_unrecognized() {
        let mut warnings = Vec::new();
        // 4 disappeared warnings for codex (compacted)
        for day in 1..=4 {
            warnings.push(format!("Day 2026-03-{day:02}: Preserved codex because it disappeared from this same-device resubmit; kept {day},000 tokens."));
        }
        // 2 decrease warnings for zcode (< 4, uncompacted)
        warnings.push("Day 2026-04-01: Preserved zcode because this same-device resubmit would reduce 500 tokens to 400.".to_string());
        warnings.push("Day 2026-04-02: Preserved zcode because this same-device resubmit would reduce 500 tokens to 400.".to_string());
        // 1 unrecognized warning
        warnings.push(
            "Ignored Amp changes because this snapshot cannot advance the device high-water."
                .to_string(),
        );
        // 4 active-time warnings (compacted)
        for day in 1..=4 {
            warnings.push(format!("Day 2026-05-{day:02}: Preserved {day}000ms active time because this same-device resubmit reported only {day}00ms."));
        }
        // 1 disappeared warning for opencode (< 4, uncompacted)
        warnings.push("Day 2026-06-01: Preserved opencode because it disappeared from this same-device resubmit; kept 999 tokens.".to_string());

        let lines = compact(&warnings);
        // Header for codex disappeared
        assert!(lines[0].contains("Preserved codex: 4 days missing from this device's resubmit"));
        assert!(lines[1].contains("2026-03-01: 1,000 | 2026-03-02: 2,000"));
        assert!(lines[2].contains("2026-03-03: 3,000 | 2026-03-04: 4,000"));
        // Uncompacted zcode decrease warnings
        assert_eq!(
            lines[3],
            "- Day 2026-04-01: Preserved zcode because this same-device resubmit would reduce 500 tokens to 400."
        );
        assert_eq!(
            lines[4],
            "- Day 2026-04-02: Preserved zcode because this same-device resubmit would reduce 500 tokens to 400."
        );
        // Unrecognized warning
        assert_eq!(
            lines[5],
            "- Ignored Amp changes because this snapshot cannot advance the device high-water."
        );
        // Header for active time
        assert!(lines[6].contains("Preserved active time: 4 daily decreases on this device"));
        assert!(lines[7].contains("2026-05-01: 1000 -> 100 | 2026-05-02: 2000 -> 200"));
        assert!(lines[8].contains("2026-05-03: 3000 -> 300 | 2026-05-04: 4000 -> 400"));
        // Uncompacted opencode disappeared warning
        assert_eq!(
            lines[9],
            "- Day 2026-06-01: Preserved opencode because it disappeared from this same-device resubmit; kept 999 tokens."
        );
        assert_eq!(lines.len(), 10);
    }
}
