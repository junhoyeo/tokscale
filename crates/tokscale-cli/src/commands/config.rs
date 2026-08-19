//! `tokscale config` — read and write persistent settings from the CLI.
//!
//! Exposes the settings a user has a concrete reason to address by hand: the
//! timezone a device buckets usage days into, pinned automatically on first
//! run, and the TUI theme, which otherwise can only be reached by cycling
//! through every theme in the app.

use anyhow::{bail, Result};
use colored::Colorize;
use tokscale_core::bucket_tz::BucketTimezone;

use crate::tui::settings::Settings;
use crate::tui::themes::ThemeName;

/// The settings `tokscale config` can address.
///
/// Kept as an explicit list rather than a free-form path into settings.json so
/// a typo is rejected instead of silently writing a key nothing reads.
const KNOWN_KEYS: &[&str] = &["timezone", "theme"];

pub fn run_get(key: &str) -> Result<()> {
    let key = normalize_key(key)?;
    let settings = Settings::load();

    match key {
        "timezone" => match settings.scanner.bucket_timezone.as_deref() {
            Some(zone) => println!("{zone}"),
            None => {
                println!("{}", "(unset)".dimmed());
                eprintln!(
                    "No bucketing timezone is pinned. Day boundaries follow this machine's \
                     current timezone and will move if it changes."
                );
            }
        },
        "theme" => println!("{}", settings.theme_name().as_str()),
        _ => unreachable!("normalize_key only returns keys handled here"),
    }

    Ok(())
}

pub fn run_set(key: &str, value: &str) -> Result<()> {
    let key = normalize_key(key)?;
    let mut settings = load_for_write()?;

    match key {
        "timezone" => {
            let is_auto = value.trim().eq_ignore_ascii_case("auto");
            let resolved = resolve_timezone_value(value)?;
            let previous = settings.scanner.bucket_timezone.clone();
            if is_auto && is_valid_timezone(previous.as_deref()) {
                bail!(
                    "cannot set scanner.bucketTimezone to auto because historical submitted day \
                     rows are monotonic. A server resync/replacement transition is required \
                     before changing this bucket timezone."
                );
            }
            reject_timezone_rekey(previous.as_deref(), &resolved)?;
            settings.scanner.bucket_timezone = Some(resolved.clone());
            settings.save()?;

            println!("{} timezone = {}", "set".green().bold(), resolved.bold());
            match previous {
                Some(previous) if previous == resolved => {
                    println!("(unchanged)");
                }
                Some(_) => println!("(unchanged)"),
                None => {
                    eprintln!(
                        "Day boundaries are now fixed to {resolved} and no longer follow this \
                         machine's timezone."
                    );
                }
            }
        }
        "theme" => {
            let theme: ThemeName = value.trim().to_ascii_lowercase().parse().map_err(|_| {
                anyhow::anyhow!(
                    "unknown theme `{value}` (known themes: {})",
                    theme_names().join(", ")
                )
            })?;
            let previous = settings.theme_name();
            settings.set_theme(theme);
            settings.save()?;

            println!("{} theme = {}", "set".green().bold(), theme.as_str().bold());
            if previous == theme {
                println!("(unchanged)");
            }
            eprintln!(
                "A running TUI writes its own theme back when it exits, so restart it to \
                 pick this up."
            );
        }
        _ => unreachable!("normalize_key only returns keys handled here"),
    }

    Ok(())
}

pub fn run_unset(key: &str) -> Result<()> {
    let key = normalize_key(key)?;
    let mut settings = load_for_write()?;

    match key {
        "timezone" => {
            reject_timezone_unset(settings.scanner.bucket_timezone.as_deref())?;
            let previous = settings.scanner.bucket_timezone.take();
            settings.save()?;

            match previous {
                Some(previous) => println!("{} timezone (was {previous})", "unset".green().bold()),
                None => println!("timezone was already unset"),
            }
            eprintln!("The next tokscale run re-detects this machine's timezone.");
        }
        "theme" => {
            // No "unset" state exists: the TUI always renders some theme, so
            // clearing the key means returning to the default rather than
            // leaving it absent.
            let previous = settings.theme_name();
            settings.set_theme(ThemeName::default_theme());
            settings.save()?;
            println!(
                "{} theme (was {}, now {})",
                "reset".green().bold(),
                previous.as_str(),
                ThemeName::default_theme().as_str()
            );
        }
        _ => unreachable!("normalize_key only returns keys handled here"),
    }

    Ok(())
}

pub fn run_list() -> Result<()> {
    let settings = Settings::load();

    let timezone = settings
        .scanner
        .bucket_timezone
        .clone()
        .unwrap_or_else(|| "(unset)".to_string());
    println!("{:<12} {}", "timezone", timezone);
    println!("{:<12} {}", "theme", settings.theme_name().as_str());

    Ok(())
}

/// Every theme `config set theme` accepts, for the error message.
fn theme_names() -> Vec<&'static str> {
    ThemeName::all().iter().map(|t| t.as_str()).collect()
}

/// Load settings for a command that is going to write them straight back.
///
/// `Settings::load()` answers an unparseable settings.json with
/// `Settings::default()`, so saving after it would replace a file we could not
/// read with defaults we invented — losing scanner paths, aliases, autosubmit
/// config and UI preferences to fix one field. `tokscale config` is a
/// deliberate, interactive command, so it says so and stops instead of guessing
/// which is worse.
fn load_for_write() -> Result<Settings> {
    let (settings, origin) = Settings::load_with_origin();
    if !origin.is_safe_to_overwrite() {
        // Deliberately does not name settings.json: this also fires when the
        // config *directory* cannot be resolved or created, where there is no
        // file to fix or remove and telling someone to delete one is a dead end.
        bail!(
            "could not read this machine's tokscale settings, so writing them would \
             replace every setting with a default. Check that the config directory \
             is readable and writable, and that settings.json in it is valid JSON."
        );
    }
    Ok(settings)
}

fn normalize_key(key: &str) -> Result<&'static str> {
    let candidate = key.trim().to_ascii_lowercase();
    KNOWN_KEYS
        .iter()
        .copied()
        .find(|known| *known == candidate)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unknown config key `{key}` (known keys: {})",
                KNOWN_KEYS.join(", ")
            )
        })
}

/// Resolve a user-supplied timezone value to a canonical IANA name.
///
/// `auto` re-detects from the machine. Anything else must be a name the tz
/// database knows: a raw UTC offset is rejected rather than accepted as a fixed
/// offset, because an offset cannot follow DST and a pinned offset drifts off
/// local midnight twice a year — the failure pinning exists to prevent.
fn resolve_timezone_value(value: &str) -> Result<String> {
    let trimmed = value.trim();

    if trimmed.eq_ignore_ascii_case("auto") {
        return tokscale_core::bucket_tz::detect_local_iana_name().ok_or_else(|| {
            anyhow::anyhow!(
                "could not determine this machine's IANA timezone name — \
                 pass one explicitly, e.g. `tokscale config set timezone Asia/Seoul`"
            )
        });
    }

    match BucketTimezone::from_pinned_name(Some(trimmed)) {
        BucketTimezone::Pinned(tz) => Ok(tz.name().to_string()),
        BucketTimezone::Local => bail!(
            "`{trimmed}` is not a known IANA timezone name (expected something like \
             `Asia/Seoul` or `America/New_York`). Fixed UTC offsets are not accepted: \
             they cannot follow daylight saving time, so a pinned offset would drift \
             off local midnight twice a year."
        ),
    }
}

/// A valid pin defines submitted day keys. The server only merges those keys
/// monotonically, so replacing or removing it would submit the same historical
/// usage under different keys and permanently inflate totals.
fn reject_timezone_rekey(previous: Option<&str>, resolved: &str) -> Result<()> {
    let BucketTimezone::Pinned(previous) = BucketTimezone::from_pinned_name(previous) else {
        return Ok(());
    };

    if previous.name() == resolved {
        return Ok(());
    }

    bail!(
        "cannot change scanner.bucketTimezone from {} to {resolved}: historical submitted day \
         rows are monotonic. A server resync/replacement transition is required before changing \
         this bucket timezone.",
        previous.name()
    )
}

fn reject_timezone_unset(previous: Option<&str>) -> Result<()> {
    let BucketTimezone::Pinned(previous) = BucketTimezone::from_pinned_name(previous) else {
        return Ok(());
    };

    bail!(
        "cannot unset scanner.bucketTimezone ({}) because historical submitted day rows are \
         monotonic. A server resync/replacement transition is required before changing this \
         bucket timezone.",
        previous.name()
    )
}

fn is_valid_timezone(value: Option<&str>) -> bool {
    BucketTimezone::from_pinned_name(value).is_pinned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_keys_are_matched_case_insensitively_and_trimmed() {
        assert_eq!(normalize_key(" TimeZone ").unwrap(), "timezone");
        assert!(normalize_key("timezon").is_err());
        assert!(normalize_key("scanner.bucketTimezone").is_err());
    }

    #[test]
    fn timezone_values_canonicalize_through_the_tz_database() {
        assert_eq!(resolve_timezone_value("Asia/Seoul").unwrap(), "Asia/Seoul");
        assert_eq!(
            resolve_timezone_value("  America/New_York  ").unwrap(),
            "America/New_York"
        );
    }

    #[test]
    fn fixed_offsets_are_rejected_rather_than_pinned() {
        for value in ["+09:00", "UTC+9", "-0500", "9"] {
            let error = resolve_timezone_value(value)
                .expect_err("a fixed offset must not be accepted as a pinned zone");
            assert!(
                error.to_string().contains("not a known IANA timezone name"),
                "unexpected error for {value}: {error}"
            );
        }
    }

    #[test]
    fn utc_is_a_real_zone_and_stays_acceptable() {
        // `UTC` is in the tz database and has no DST, so unlike `+00:00` it is
        // a legitimate pin — useful for servers and CI that genuinely run on it.
        assert_eq!(resolve_timezone_value("UTC").unwrap(), "UTC");
    }

    #[test]
    fn valid_pins_only_allow_the_same_canonical_zone() {
        assert!(reject_timezone_rekey(Some("Asia/Seoul"), "Asia/Seoul").is_ok());
        let error = reject_timezone_rekey(Some("Asia/Seoul"), "UTC").unwrap_err();
        assert!(error
            .to_string()
            .contains("historical submitted day rows are monotonic"));
        assert!(reject_timezone_unset(Some("Asia/Seoul")).is_err());
    }

    #[test]
    fn unpinned_or_invalid_values_can_be_recovered() {
        assert!(reject_timezone_rekey(None, "UTC").is_ok());
        assert!(reject_timezone_rekey(Some("Mars/Olympus_Mons"), "UTC").is_ok());
        assert!(reject_timezone_unset(None).is_ok());
        assert!(reject_timezone_unset(Some("Mars/Olympus_Mons")).is_ok());
    }

    #[test]
    fn valid_pin_is_distinguished_from_an_invalid_hand_edited_value() {
        assert!(is_valid_timezone(Some("Asia/Seoul")));
        assert!(!is_valid_timezone(Some("Mars/Olympus_Mons")));
        assert!(!is_valid_timezone(None));
    }
}
