use anyhow::Result;
use chrono::{DateTime, Duration, Local, TimeZone, Utc};

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s.to_string(),
    }
}

pub fn read_keychain(service: &str) -> Result<String> {
    if cfg!(not(target_os = "macos")) {
        anyhow::bail!("Keychain lookup is only available on macOS");
    }
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", service, "-w"])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("Keychain lookup failed for service '{service}'");
    }
    Ok(String::from_utf8(out.stdout)?.trim_end().to_string())
}

pub fn format_reset_time(resets_at: &str) -> String {
    format_reset_time_at(resets_at, Utc::now(), &Local)
}

fn format_reset_time_at<Tz>(resets_at: &str, now_utc: DateTime<Utc>, display_tz: &Tz) -> String
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let dt_utc = match DateTime::parse_from_rfc3339(resets_at) {
        Ok(d) => d.with_timezone(&Utc),
        Err(_) => return resets_at.into(),
    };
    let diff = dt_utc - now_utc;
    if diff <= Duration::zero() {
        return "resets now".into();
    }
    let total_mins = diff.num_minutes();
    if total_mins < 60 {
        format!("resets in {total_mins}m")
    } else if total_mins < 24 * 60 {
        let h = diff.num_hours();
        let m = (diff - Duration::hours(h)).num_minutes();
        if m > 0 {
            format!("resets in {h}h {m}m")
        } else {
            format!("resets in {h}h")
        }
    } else if diff.num_days() < 7 {
        let dt_local = dt_utc.with_timezone(display_tz);
        format!("resets {}", dt_local.format("%a %-I:%M%P"))
    } else {
        let dt_local = dt_utc.with_timezone(display_tz);
        format!("resets {}", dt_local.format("%b %-d %-I:%M%P"))
    }
}

pub fn render_ascii_bar(remaining_percent: f64, width: usize) -> String {
    let filled = (remaining_percent.clamp(0.0, 100.0) / 100.0 * width as f64).round() as usize;
    format!("[{}{}]", "=".repeat(filled), "-".repeat(width - filled))
}

pub fn atomic_write_secret(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path has no parent directory",
        )
    })?;
    std::fs::create_dir_all(dir)?;
    let temp_path = path.with_extension(format!("{}.tmp", std::process::id()));
    {
        #[cfg(unix)]
        let mut opts = {
            use std::os::unix::fs::OpenOptionsExt;
            let mut o = std::fs::OpenOptions::new();
            o.mode(0o600);
            o
        };
        #[cfg(not(unix))]
        let mut opts = std::fs::OpenOptions::new();
        let mut f = match opts.write(true).create_new(true).open(&temp_path) {
            Ok(f) => f,
            Err(e) => {
                let _ = std::fs::remove_file(&temp_path);
                return Err(e);
            }
        };
        if let Err(e) = std::io::Write::write_all(&mut f, data) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(e);
        }
    }
    if let Err(e) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};

    #[test]
    fn reset_time_uses_display_timezone_not_utc() {
        let now = Utc.with_ymd_and_hms(2026, 6, 19, 0, 0, 0).unwrap();
        let pacific_daylight = FixedOffset::west_opt(7 * 60 * 60).unwrap();

        let shown = format_reset_time_at("2026-06-25T11:45:00Z", now, &pacific_daylight);

        assert_eq!(shown, "resets Thu 4:45am");
    }

    #[test]
    fn reset_time_keeps_minutes_for_local_absolute_times() {
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let pacific_daylight = FixedOffset::west_opt(7 * 60 * 60).unwrap();

        let shown = format_reset_time_at("2026-06-10T18:59:00Z", now, &pacific_daylight);

        assert_eq!(shown, "resets Jun 10 11:59am");
    }

    #[test]
    fn reset_time_rolls_back_to_previous_local_day() {
        let now = Utc.with_ymd_and_hms(2026, 6, 19, 0, 0, 0).unwrap();
        let pacific_daylight = FixedOffset::west_opt(7 * 60 * 60).unwrap();

        // 2026-06-25 03:00 UTC -> 2026-06-24 20:00 PDT: the local day (Wed)
        // is the day before the UTC day (Thu), so the weekday must come from
        // the converted time, not the UTC timestamp.
        let shown = format_reset_time_at("2026-06-25T03:00:00Z", now, &pacific_daylight);

        assert_eq!(shown, "resets Wed 8:00pm");
    }
}
