// ── Sakana (Fugu) subscription-usage provider ──
//
// IMPORTANT — read before touching this file:
//
//   Sakana (Fugu) exposes NO public usage/quota API. This was investigated and
//   confirmed: there is no documented REST endpoint and no OAuth-scoped usage
//   route comparable to Claude's `/api/oauth/usage` or Z.ai's quota endpoint.
//   The ONLY source of subscription-usage data is the authenticated billing
//   console at https://console.sakana.ai/billing.
//
//   That console is a Next.js app, but the rendered usage values ARE present in
//   the served HTML of a plain authenticated GET (verified against the real
//   page). So this provider fetches that HTML with the user's session cookie and
//   scrapes the values out of it.
//
//   Consequences you MUST keep in mind:
//     * This is a best-effort, COOKIE-AUTH, LAYOUT-COUPLED scraper. If Sakana
//       restructures the billing page, the parser silently degrades (fields
//       become None) or fails to find the markers. It is not a stable contract.
//     * The session cookie EXPIRES. When it does, the GET returns a login page
//       (or 401/403). We detect that and tell the user to refresh the cookie —
//       we do NOT panic and do NOT emit a bogus parse.
//     * The numbers reported here are rolling QUOTA windows (5-hour / weekly
//       "% used"), NOT dollar spend. Sakana subscription billing is a flat
//       monthly fee ($NN/mo), so there is no per-request spend to report; the
//       monthly price and next-renewal date are surfaced as plan metadata only.
//
//   Parsing is done with plain `str` scanning (no `regex` dependency in this
//   crate) but follows the exact validated token formats documented inline.

use anyhow::Result;

use super::{UsageMetric, UsageOutput};

const BILLING_URL: &str = "https://console.sakana.ai/billing";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

// ── Credential discovery ──

/// Returns the session cookie string if one is available, else None.
///
/// Source order:
///   1. env var `SAKANA_SESSION_COOKIE`
///   2. file `~/.config/tokscale/sakana-session` (raw cookie string, trimmed)
///
/// Empty / whitespace-only values are treated as absent.
fn session_cookie() -> Option<String> {
    if let Ok(val) = std::env::var("SAKANA_SESSION_COOKIE") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let path = home.join(".config").join("tokscale").join("sakana-session");
    if let Ok(content) = std::fs::read_to_string(&path) {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

pub fn has_credentials() -> bool {
    session_cookie().is_some()
}

// ── Parsed shape ──

#[derive(Debug, Default, PartialEq)]
struct ParsedBilling {
    plan: Option<String>,
    monthly_price: Option<u32>,
    next_renewal: Option<String>,
    windows: Vec<ParsedWindow>,
}

#[derive(Debug, PartialEq)]
struct ParsedWindow {
    label: String,
    used_percent: f64,
    resets_at: Option<String>,
}

// ── Login-page detection ──

/// Heuristic: does this HTML look like a logged-out / login page rather than the
/// authenticated billing console?
///
/// The reliable positive signal is the presence of the billing data markers
/// ("Billing" plus a "$/mo" price or a "% used" quota). Their ABSENCE means we
/// did not get the billing page (login redirect, 401 body, or error page). We key
/// off marker absence rather than sign-in strings on purpose: a valid logged-in
/// page can legitimately contain "/login" / "Sign in" references (auth nav,
/// callbacks), which would otherwise false-flag a working session as expired.
fn looks_logged_out(html: &str) -> bool {
    let has_billing_markers =
        html.contains("Billing") && (html.contains("/mo") || html.contains("% used"));
    !has_billing_markers
}

// ── Parsing helpers (plain str, no regex) ──

/// Find the monthly price for the pattern `\$(\d+)\s*/\s*mo`, e.g. `$20 / mo`,
/// `$20/mo`. Returns (price, byte index of the `$` that matched).
fn find_monthly_price(html: &str) -> Option<(u32, usize)> {
    let bytes = html.as_bytes();
    let mut search_from = 0usize;
    while let Some(rel) = html[search_from..].find('$') {
        let dollar_idx = search_from + rel;
        let mut i = dollar_idx + 1;
        // digits
        let digits_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == digits_start {
            search_from = dollar_idx + 1;
            continue;
        }
        let digits = &html[digits_start..i];
        // optional whitespace
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // slash
        if i < bytes.len() && bytes[i] == b'/' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if html[i..].starts_with("mo") {
                if let Ok(price) = digits.parse::<u32>() {
                    return Some((price, dollar_idx));
                }
            }
        }
        search_from = dollar_idx + 1;
    }
    None
}

/// Determine the active plan tier. The active tier is the (Standard|Pro|Max)
/// token NEAREST-PRECEDING the `$NN/mo` price match — the upgrade buttons also
/// contain "Pro"/"Max", but the active tier is the one rendered with the price.
/// If we cannot find one before the price, fall back to the first
/// (Standard|Pro|Max) occurrence after the word "Billing".
fn find_plan(html: &str, price_idx: Option<usize>) -> Option<String> {
    const TIERS: [&str; 3] = ["Standard", "Pro", "Max"];

    // Nearest-preceding the price.
    if let Some(idx) = price_idx {
        let prefix = &html[..idx];
        let mut best: Option<(usize, &str)> = None;
        for tier in TIERS {
            if let Some(pos) = prefix.rfind(tier) {
                match best {
                    Some((bp, _)) if bp >= pos => {}
                    _ => best = Some((pos, tier)),
                }
            }
        }
        if let Some((_, tier)) = best {
            return Some(tier.to_string());
        }
    }

    // Fallback: first tier after "Billing".
    let start = html
        .find("Billing")
        .map(|i| i + "Billing".len())
        .unwrap_or(0);
    let region = &html[start..];
    let mut best: Option<(usize, &str)> = None;
    for tier in TIERS {
        if let Some(pos) = region.find(tier) {
            match best {
                Some((bp, _)) if bp <= pos => {}
                _ => best = Some((pos, tier)),
            }
        }
    }
    best.map(|(_, tier)| tier.to_string())
}

/// Collect all `(\d{1,3})% used` percentages in document order.
fn find_used_percents(html: &str) -> Vec<f64> {
    const NEEDLE: &str = "% used";
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = html[search_from..].find(NEEDLE) {
        let pct_sign = search_from + rel; // index of '%'
                                          // Walk backwards over up to 3 digits immediately preceding '%'.
        let mut start = pct_sign;
        let mut count = 0;
        while start > 0 && count < 3 && bytes[start - 1].is_ascii_digit() {
            start -= 1;
            count += 1;
        }
        if count > 0 {
            if let Ok(v) = html[start..pct_sign].parse::<f64>() {
                out.push(v.clamp(0.0, 100.0));
            }
        }
        search_from = pct_sign + NEEDLE.len();
    }
    out
}

/// Collect window label positions matching `>(5-hour|Weekly)<`, sorted by
/// document order. Returns (byte index of the label, label).
fn find_window_label_positions(html: &str) -> Vec<(usize, String)> {
    const LABELS: [&str; 2] = ["5-hour", "Weekly"];
    let mut found: Vec<(usize, String)> = Vec::new();
    for label in LABELS {
        let needle = format!(">{label}<");
        let mut search_from = 0usize;
        while let Some(rel) = html[search_from..].find(&needle) {
            let idx = search_from + rel;
            found.push((idx, label.to_string()));
            search_from = idx + needle.len();
        }
    }
    found.sort_by_key(|(idx, _)| *idx);
    found
}

/// Build one window per on-page label (`5-hour` / `Weekly`), binding the
/// percentage and reset time that STRUCTURALLY belong to that label — the first
/// of each within the label's section (from the label up to the next label) —
/// rather than collecting every `% used` in the document.
///
/// This matters because the served HTML embeds each usage value MORE THAN ONCE
/// (the rendered card markup AND serialized RSC data), so a global
/// "collect-all-percents, pair-by-index" approach invents phantom windows and
/// mis-pairs percentages. The window labels appear only in the rendered cards,
/// so anchoring on them is the reliable structural key.
fn parse_windows(html: &str) -> Vec<ParsedWindow> {
    let labels = find_window_label_positions(html);
    if !labels.is_empty() {
        let mut windows = Vec::with_capacity(labels.len());
        for (k, (pos, label)) in labels.iter().enumerate() {
            let end = labels.get(k + 1).map(|(p, _)| *p).unwrap_or(html.len());
            let segment = &html[*pos..end];
            if let Some(&pct) = find_used_percents(segment).first() {
                windows.push(ParsedWindow {
                    label: label.clone(),
                    used_percent: pct,
                    resets_at: find_reset_times(segment).into_iter().next(),
                });
            }
        }
        return windows;
    }

    // Degraded fallback: no labels found at all. Emit at most the known number
    // of windows from the leading percentages, in document order, so a label
    // markup change still surfaces *something* without inventing phantoms.
    const FALLBACK_LABELS: [&str; 2] = ["5-hour", "Weekly"];
    find_used_percents(html)
        .into_iter()
        .take(FALLBACK_LABELS.len())
        .enumerate()
        .map(|(i, pct)| ParsedWindow {
            label: FALLBACK_LABELS[i].to_string(),
            used_percent: pct,
            resets_at: None,
        })
        .collect()
}

/// Collect reset times matching
/// `Resets on\s+([A-Z][a-z]+ \d{1,2}, \d{4} at \d{1,2}:\d{2} [AP]M)` in order.
fn find_reset_times(html: &str) -> Vec<String> {
    const PREFIX: &str = "Resets on";
    let mut out = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = html[search_from..].find(PREFIX) {
        let idx = search_from + rel;
        let after = &html[idx + PREFIX.len()..];
        let trimmed = after.trim_start();
        if let Some(reset) = parse_reset_value(trimmed) {
            out.push(reset);
        }
        search_from = idx + PREFIX.len();
    }
    out
}

/// Parse `Month D, YYYY at H:MM AM/PM` from the start of `s`.
fn parse_reset_value(s: &str) -> Option<String> {
    // Month
    let (month, rest) = take_capitalized_word(s)?;
    let rest = rest.strip_prefix(' ')?;
    let (_day, rest) = take_digits(rest, 1, 2)?;
    let rest = rest.strip_prefix(", ")?;
    let (_year, rest) = take_digits(rest, 4, 4)?;
    let rest = rest.strip_prefix(" at ")?;
    let (_hour, rest) = take_digits(rest, 1, 2)?;
    let rest = rest.strip_prefix(':')?;
    let (_min, rest) = take_digits(rest, 2, 2)?;
    let rest = rest.strip_prefix(' ')?;
    let meridiem = if rest.starts_with("AM") {
        "AM"
    } else if rest.starts_with("PM") {
        "PM"
    } else {
        return None;
    };
    // Reconstruct the exact matched substring.
    let consumed = s.len() - rest.len() + meridiem.len();
    let _ = month;
    Some(s[..consumed].to_string())
}

/// Next renewal: `Next renewal:?\s*([A-Z][a-z]+ \d{1,2}, \d{4})`.
fn find_next_renewal(html: &str) -> Option<String> {
    const PREFIX: &str = "Next renewal";
    let idx = html.find(PREFIX)?;
    let mut after = &html[idx + PREFIX.len()..];
    after = after.strip_prefix(':').unwrap_or(after);
    let after = after.trim_start();
    parse_date_value(after)
}

/// Parse `Month D, YYYY` from the start of `s`.
fn parse_date_value(s: &str) -> Option<String> {
    let (_month, rest) = take_capitalized_word(s)?;
    let rest = rest.strip_prefix(' ')?;
    let (_day, rest) = take_digits(rest, 1, 2)?;
    let rest = rest.strip_prefix(", ")?;
    let (_year, rest) = take_digits(rest, 4, 4)?;
    let consumed = s.len() - rest.len();
    Some(s[..consumed].to_string())
}

/// Take a leading `[A-Z][a-z]+` word; return (word, remainder).
fn take_capitalized_word(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_uppercase() {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && bytes[i].is_ascii_lowercase() {
        i += 1;
    }
    if i < 2 {
        return None;
    }
    Some((&s[..i], &s[i..]))
}

/// Take between `min` and `max` leading ASCII digits; return (digits, remainder).
fn take_digits(s: &str, min: usize, max: usize) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && i < max && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < min {
        return None;
    }
    Some((&s[..i], &s[i..]))
}

// ── Top-level parser ──

/// Parse the served billing HTML into a structured shape. Returns Err only when
/// the page looks logged-out / the session is invalid. Missing individual fields
/// degrade to None / empty rather than failing the whole parse.
fn parse_billing(html: &str) -> Result<ParsedBilling> {
    if looks_logged_out(html) {
        anyhow::bail!("NEEDS_AUTH");
    }

    let price = find_monthly_price(html);
    let monthly_price = price.map(|(p, _)| p);
    let price_idx = price.map(|(_, i)| i);
    let plan = find_plan(html, price_idx);

    Ok(ParsedBilling {
        plan,
        monthly_price,
        next_renewal: find_next_renewal(html),
        windows: parse_windows(html),
    })
}

// ── Output assembly ──

fn build_output(parsed: ParsedBilling) -> UsageOutput {
    let metrics = parsed
        .windows
        .into_iter()
        .map(|w| UsageMetric {
            label: w.label,
            used_percent: w.used_percent,
            remaining_percent: 100.0 - w.used_percent,
            remaining_label: None,
            resets_at: w.resets_at,
        })
        .collect();

    // Surface monthly price + next renewal as plan metadata (the struct has no
    // dedicated billing fields, and these are flat-fee subscription details, not
    // quota windows).
    let plan = match (parsed.plan, parsed.monthly_price, parsed.next_renewal) {
        (Some(tier), Some(price), Some(renew)) => {
            Some(format!("{tier} (${price}/mo, renews {renew})"))
        }
        (Some(tier), Some(price), None) => Some(format!("{tier} (${price}/mo)")),
        (Some(tier), None, Some(renew)) => Some(format!("{tier} (renews {renew})")),
        (Some(tier), None, None) => Some(tier),
        (None, Some(price), _) => Some(format!("${price}/mo")),
        (None, None, _) => None,
    };

    UsageOutput {
        provider: "Sakana".into(),
        account: None,
        plan,
        email: None,
        metrics,
        reset_credits: None,
        credit_status: None,
        spend_control: None,
    }
}

async fn fetch_billing_html(client: &reqwest::Client, cookie: &str) -> Result<String> {
    let resp = client
        .get(BILLING_URL)
        .header("Cookie", cookie)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "text/html")
        .send()
        .await?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("NEEDS_AUTH");
    }
    if !status.is_success() {
        anyhow::bail!("Sakana billing request failed (HTTP {status})");
    }
    Ok(resp.text().await?)
}

pub fn fetch() -> Result<UsageOutput> {
    let cookie = session_cookie().ok_or_else(|| {
        anyhow::anyhow!(
            "No Sakana session cookie. Set SAKANA_SESSION_COOKIE or write \
             ~/.config/tokscale/sakana-session."
        )
    })?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?;

        let html = fetch_billing_html(&client, &cookie).await.map_err(|e| {
            if e.to_string().contains("NEEDS_AUTH") {
                anyhow::anyhow!(
                    "Sakana session expired or invalid. Refresh SAKANA_SESSION_COOKIE \
                     (re-copy the __Secure-authjs.session-token cookie from \
                     console.sakana.ai)."
                )
            } else {
                e
            }
        })?;

        let parsed = parse_billing(&html).map_err(|e| {
            if e.to_string().contains("NEEDS_AUTH") {
                anyhow::anyhow!(
                    "Sakana session expired or invalid (login page returned). Refresh \
                     SAKANA_SESSION_COOKIE from console.sakana.ai."
                )
            } else {
                e
            }
        })?;

        Ok(build_output(parsed))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A representative valid billing fixture using the exact validated token
    // formats. Synthetic — contains NO real cookies/tokens.
    const VALID_STANDARD: &str = r#"
<html><body>
  <nav>Billing</nav>
  <div class="plan-card">
    <span>Standard</span>
    <span>$20 / mo</span>
    <button>Upgrade to Pro</button>
    <button>Upgrade to Max</button>
  </div>
  <div class="window">
    <span>5-hour</span>
    <span>55% used</span>
    <span>Resets on June 22, 2026 at 9:58 AM</span>
  </div>
  <div class="window">
    <span>Weekly</span>
    <span>19% used</span>
    <span>Resets on June 28, 2026 at 12:00 PM</span>
  </div>
  <div>Next renewal: July 22, 2026</div>
</body></html>
"#;

    #[test]
    fn parses_valid_standard_billing() {
        let parsed = parse_billing(VALID_STANDARD).expect("should parse");
        assert_eq!(parsed.plan.as_deref(), Some("Standard"));
        assert_eq!(parsed.monthly_price, Some(20));
        assert_eq!(parsed.next_renewal.as_deref(), Some("July 22, 2026"));
        assert_eq!(parsed.windows.len(), 2);

        assert_eq!(parsed.windows[0].label, "5-hour");
        assert_eq!(parsed.windows[0].used_percent, 55.0);
        assert_eq!(
            parsed.windows[0].resets_at.as_deref(),
            Some("June 22, 2026 at 9:58 AM")
        );

        assert_eq!(parsed.windows[1].label, "Weekly");
        assert_eq!(parsed.windows[1].used_percent, 19.0);
        assert_eq!(
            parsed.windows[1].resets_at.as_deref(),
            Some("June 28, 2026 at 12:00 PM")
        );
    }

    #[test]
    fn build_output_shapes_metrics_and_plan() {
        let parsed = parse_billing(VALID_STANDARD).unwrap();
        let out = build_output(parsed);
        assert_eq!(out.provider, "Sakana");
        assert_eq!(
            out.plan.as_deref(),
            Some("Standard ($20/mo, renews July 22, 2026)")
        );
        assert_eq!(out.metrics.len(), 2);
        assert_eq!(out.metrics[0].label, "5-hour");
        assert_eq!(out.metrics[0].used_percent, 55.0);
        assert_eq!(out.metrics[0].remaining_percent, 45.0);
        assert_eq!(out.metrics[1].label, "Weekly");
        assert_eq!(out.metrics[1].used_percent, 19.0);
    }

    // Pro tier, where the "Upgrade to Max" button also contains a tier word.
    // The active tier ("Pro") is the one rendered nearest-preceding the price.
    const VALID_PRO: &str = r#"
<html><body>
  <nav>Billing</nav>
  <div class="plan-card">
    <span>Pro</span>
    <span>$100 / mo</span>
    <button>Upgrade to Max</button>
  </div>
  <div class="window">
    <span>5-hour</span>
    <span>8% used</span>
    <span>Resets on June 22, 2026 at 3:15 PM</span>
  </div>
  <div class="window">
    <span>Weekly</span>
    <span>72% used</span>
    <span>Resets on June 29, 2026 at 1:00 AM</span>
  </div>
  <div>Next renewal: July 22, 2026</div>
</body></html>
"#;

    #[test]
    fn disambiguates_active_pro_tier_from_upgrade_button() {
        let parsed = parse_billing(VALID_PRO).expect("should parse");
        // "Max" appears in the upgrade button AFTER the price; active tier is
        // "Pro", which is nearest-preceding the $100/mo price.
        assert_eq!(parsed.plan.as_deref(), Some("Pro"));
        assert_eq!(parsed.monthly_price, Some(100));
        assert_eq!(parsed.windows.len(), 2);
        assert_eq!(parsed.windows[0].used_percent, 8.0);
        assert_eq!(parsed.windows[1].used_percent, 72.0);
    }

    // Logged-out / login page: billing markers absent, sign-in present.
    const LOGIN_PAGE: &str = r#"
<html><body>
  <h1>Sign in to Sakana</h1>
  <a href="/login">Continue with Google</a>
</body></html>
"#;

    #[test]
    fn login_page_returns_needs_auth_error() {
        let err = parse_billing(LOGIN_PAGE).expect_err("login page must error");
        assert!(
            err.to_string().contains("NEEDS_AUTH"),
            "expected NEEDS_AUTH, got: {err}"
        );
    }

    // Missing renewal + reset; percentages must still parse.
    const MISSING_META: &str = r#"
<html><body>
  <nav>Billing</nav>
  <div class="plan-card">
    <span>Standard</span>
    <span>$20 / mo</span>
  </div>
  <div class="window">
    <span>5-hour</span>
    <span>33% used</span>
  </div>
  <div class="window">
    <span>Weekly</span>
    <span>5% used</span>
  </div>
</body></html>
"#;

    #[test]
    fn graceful_degradation_when_meta_missing() {
        let parsed = parse_billing(MISSING_META).expect("should still parse percentages");
        assert_eq!(parsed.plan.as_deref(), Some("Standard"));
        assert_eq!(parsed.monthly_price, Some(20));
        assert_eq!(parsed.next_renewal, None);
        assert_eq!(parsed.windows.len(), 2);
        assert_eq!(parsed.windows[0].label, "5-hour");
        assert_eq!(parsed.windows[0].used_percent, 33.0);
        assert_eq!(parsed.windows[0].resets_at, None);
        assert_eq!(parsed.windows[1].label, "Weekly");
        assert_eq!(parsed.windows[1].used_percent, 5.0);
        assert_eq!(parsed.windows[1].resets_at, None);
    }

    // Mirrors the REAL served HTML: the usage values are ALSO embedded in
    // serialized RSC data (extra "% used" tokens, some preceding the rendered
    // cards, with NO window label). The parser must anchor on the labels and
    // emit exactly 2 correctly-paired windows — not invent "Window 3/4" or
    // mis-pair the weekly value. Regression test for the bug caught by a live run.
    const DUPLICATED_RSC: &str = r#"
<html><body>
  <script>self.__next_f.push([1,"...stuff 19% used more 55% used trailing..."])</script>
  <nav>Billing</nav>
  <div class="plan-card"><span>Standard</span><span>$20 / mo</span></div>
  <div class="window"><span>5-hour</span><span>55% used</span><span>Resets on June 22, 2026 at 9:58 AM</span></div>
  <div class="window"><span>Weekly</span><span>19% used</span><span>Resets on June 29, 2026 at 12:00 AM</span></div>
  <script>self.__next_f.push([1,"...echo 55% used and 19% used again..."])</script>
</body></html>
"#;

    #[test]
    fn ignores_duplicated_rsc_percentages() {
        let parsed = parse_billing(DUPLICATED_RSC).expect("should parse");
        assert_eq!(
            parsed.windows.len(),
            2,
            "must bind to the 2 labels, not collect every % used"
        );
        assert_eq!(parsed.windows[0].label, "5-hour");
        assert_eq!(parsed.windows[0].used_percent, 55.0);
        assert_eq!(
            parsed.windows[0].resets_at.as_deref(),
            Some("June 22, 2026 at 9:58 AM")
        );
        assert_eq!(parsed.windows[1].label, "Weekly");
        assert_eq!(parsed.windows[1].used_percent, 19.0);
        assert_eq!(
            parsed.windows[1].resets_at.as_deref(),
            Some("June 29, 2026 at 12:00 AM")
        );
    }

    #[test]
    fn percents_parse_in_document_order() {
        let pcts = find_used_percents("a 55% used b 19% used c 100% used");
        assert_eq!(pcts, vec![55.0, 19.0, 100.0]);
    }

    #[test]
    fn monthly_price_tolerates_spacing_variants() {
        assert_eq!(find_monthly_price("$20/mo").map(|(p, _)| p), Some(20));
        assert_eq!(find_monthly_price("$20 / mo").map(|(p, _)| p), Some(20));
        assert_eq!(find_monthly_price("$200  /  mo").map(|(p, _)| p), Some(200));
        assert_eq!(find_monthly_price("no price here"), None);
    }
}
