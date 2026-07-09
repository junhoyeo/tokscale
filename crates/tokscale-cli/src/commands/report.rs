use anyhow::Result;
use chrono::{Local, TimeZone};
use colored::Colorize;
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use unicode_normalization::UnicodeNormalization;

use super::apple_fm;
use std::path::PathBuf;
use std::time::Duration;
use tokscale_core::content_extractor::SessionContent;
use tokscale_core::content_extractor::{extract_session_content, metadata_only_content};
use tokscale_core::pricing::PricingService;
use tokscale_core::wiki::{WikiDb, WikiEntry};
use tokscale_core::{parse_local_clients, LocalParseOptions, ParsedMessage, TokenBreakdown};

pub struct ReportOptions {
    pub json: bool,
    pub since: Option<String>,
    pub until: Option<String>,
    pub workspace: Option<String>,
    pub client: Option<String>,
    pub no_summarize: bool,
    pub summarizer: String,
    pub rebuild: bool,
    pub home_dir: Option<String>,
    pub scanner_settings: tokscale_core::scanner::ScannerSettings,
    pub today: bool,
    pub week: bool,
    pub month: bool,
    pub full: bool,
}

pub fn run_report(opts: ReportOptions) -> Result<()> {
    let wiki_path = WikiDb::default_path();
    let db =
        WikiDb::open(&wiki_path).map_err(|e| anyhow::anyhow!("Failed to open wiki DB: {}", e))?;

    populate_wiki_from_sessions(&db, &opts)?;

    let (since_ts, until_ts) = parse_date_range(&opts.since, &opts.until);

    if opts.rebuild {
        let count = db
            .reset_summaries_in_range(since_ts, until_ts)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        eprintln!("  Reset {} session summaries", count.to_string().cyan());
    }

    let unsummarized = if opts.no_summarize {
        Vec::new()
    } else {
        db.get_unsummarized_session_ids_in_range(since_ts, until_ts)
            .map_err(|e| anyhow::anyhow!("{}", e))?
    };

    if !unsummarized.is_empty() {
        let session_paths = build_session_path_index(&opts);
        run_summarizer(&db, &unsummarized, &opts.summarizer, &session_paths)?;
    }

    let entries = db
        .query_entries(
            since_ts,
            until_ts,
            opts.workspace.as_deref(),
            opts.client.as_deref(),
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let needs_grouping = entries
        .iter()
        .any(|e| e.title.is_some() && e.task_group.is_none());
    if needs_grouping && !opts.no_summarize {
        run_task_grouping(&db, &entries, &opts.summarizer)?;
        let entries = db
            .query_entries(
                since_ts,
                until_ts,
                opts.workspace.as_deref(),
                opts.client.as_deref(),
            )
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        if opts.json {
            let json = serde_json::to_string_pretty(&entries)?;
            println!("{}", json);
        } else {
            let is_multi_day = opts.week || opts.month || (opts.since.is_some() && !opts.today);
            print_report_table(&entries, &db, is_multi_day, opts.full)?;
        }
    } else if opts.json {
        let json = serde_json::to_string_pretty(&entries)?;
        println!("{}", json);
    } else {
        let is_multi_day = opts.week || opts.month || (opts.since.is_some() && !opts.today);
        print_report_table(&entries, &db, is_multi_day, opts.full)?;
    }

    Ok(())
}

fn populate_wiki_from_sessions(db: &WikiDb, opts: &ReportOptions) -> Result<()> {
    let existing = db
        .get_existing_session_ids()
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let parsed = parse_local_clients(LocalParseOptions {
        home_dir: opts.home_dir.clone(),
        use_env_roots: opts.home_dir.is_none(),
        clients: None,
        since: None,
        until: None,
        year: None,
        scanner_settings: opts.scanner_settings.clone(),
    })
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    let pricing = load_pricing_service();

    let mut session_map: HashMap<String, SessionAgg> = HashMap::new();

    for msg in &parsed.messages {
        let agg = session_map
            .entry(msg.session_id.clone())
            .or_insert_with(|| SessionAgg {
                client: msg.client.clone(),
                workspace: msg.workspace_key.clone(),
                workspace_label: msg.workspace_label.clone(),
                created_at: msg.timestamp,
                last_active: msg.timestamp,
                total_input: 0,
                total_output: 0,
                total_cache_read: 0,
                total_cost: 0.0,
                models: HashMap::new(),
                message_count: 0,
            });

        agg.last_active = agg.last_active.max(msg.timestamp);
        agg.created_at = agg.created_at.min(msg.timestamp);
        // saturating: per-message token fields from a corrupt source can be
        // clamped to i64::MAX (see tokscale-core), so plain `+=` can overflow.
        agg.total_input = agg.total_input.saturating_add(msg.input);
        agg.total_output = agg.total_output.saturating_add(msg.output);
        agg.total_cache_read = agg.total_cache_read.saturating_add(msg.cache_read);
        agg.total_cost += compute_msg_cost(msg, pricing.as_deref());
        // NOTE: the wiki `report` view intentionally groups on the raw model_id
        // and does not apply `modelAliases` folding (nor the grouping
        // normalization every other report uses). Wiki entries are persisted
        // append-only — previously-recorded sessions are not rewritten (see the
        // `existing.contains` skip below) — so folding here would leave a mix of
        // raw and canonical names across sessions recorded before vs after
        // aliases were configured. To fold in a future change, wrap the key with
        // `tokscale_core::normalize_model_for_grouping(&msg.model_id)` here and in
        // the by-model/daily/session/JSON surfaces.
        *agg.models.entry(msg.model_id.clone()).or_insert(0) += 1;
        agg.message_count += msg.message_count;
    }

    let mut new_count = 0;
    for (session_id, agg) in &session_map {
        if existing.contains(session_id) {
            continue;
        }

        let models_used: Vec<String> = agg.models.keys().cloned().collect();
        let duration_minutes = (agg.last_active - agg.created_at) / 60;

        let entry = WikiEntry {
            session_id: session_id.clone(),
            client: agg.client.clone(),
            workspace: agg.workspace.clone(),
            workspace_label: agg.workspace_label.clone(),
            created_at: agg.created_at,
            last_active: agg.last_active,
            title: None,
            task_category: None,
            description: None,
            complexity: None,
            task_group: None,
            total_input_tokens: agg.total_input,
            total_output_tokens: agg.total_output,
            total_cache_read: agg.total_cache_read,
            total_cost: agg.total_cost,
            models_used,
            message_count: agg.message_count,
            duration_minutes,
            summarized_at: None,
            fm_version: None,
        };

        db.upsert_entry(&entry)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        new_count += 1;
    }

    if new_count > 0 {
        eprintln!(
            "  {} new sessions added to wiki",
            new_count.to_string().cyan()
        );
    }

    Ok(())
}

fn run_summarizer(
    db: &WikiDb,
    session_ids: &[String],
    backend: &str,
    session_paths: &SessionPathIndex,
) -> Result<()> {
    let mut payloads: Vec<serde_json::Value> = Vec::new();
    for sid in session_ids {
        if let Ok(Some(entry)) = db.get_entry(sid) {
            let content = extract_content_for_session(&entry, session_paths);
            payloads.push(serde_json::json!({
                "session_id": entry.session_id,
                "client": entry.client,
                "workspace": entry.workspace.unwrap_or_default(),
                "first_user_message": content.first_user_message,
                "models_used": entry.models_used,
                "total_tokens": entry.total_input_tokens.saturating_add(entry.total_output_tokens),
                "duration_minutes": entry.duration_minutes,
                "message_count": entry.message_count,
            }));
        }
    }

    if payloads.is_empty() {
        return Ok(());
    }

    eprintln!(
        "  Summarizing {} sessions with {}...",
        payloads.len().to_string().cyan(),
        backend.cyan()
    );

    // apple-fm runs each session as a self-contained on-device generation, so a
    // single giant chunk would suppress the per-batch progress indicator below
    // (it's gated on `batch_size < payloads.len()`). Use a modest batch size so
    // the "\r  Batch i/total" line fires and 100+ sequential on-device calls
    // show visible progress instead of hanging silent until the very end.
    // Re-fetching the model + rebuilding the schema per small batch is cheap;
    // the generation dominates.
    let batch_size = match backend {
        "apple-fm" => 8,
        _ => 20,
    };

    let mut total_summarized = 0;
    // Count how many summaries actually came from Apple FM vs the heuristic
    // fallback, so a silent total-fallback (e.g. FM unavailable, or every
    // generation erroring) is visible rather than reported as plain "N
    // summarized". Only meaningful for the apple-fm backend; CLI backends leave
    // fm_version null by design.
    let mut fm_generated = 0;
    for (batch_idx, chunk) in payloads.chunks(batch_size).enumerate() {
        if batch_size < payloads.len() {
            eprint!(
                "\r  Batch {}/{} ({} done)...",
                batch_idx + 1,
                payloads.len().div_ceil(batch_size),
                total_summarized
            );
        }

        let results = match backend {
            "apple-fm" => run_apple_fm_summarizer(chunk)?,
            "claude" | "codex" | "gemini" | "kiro" => run_cli_summarizer(backend, chunk)?,
            other => {
                return Err(anyhow::anyhow!(
                    "Unknown summarizer backend: '{}'. Valid options: apple-fm, claude, codex, gemini, kiro",
                    other
                ));
            }
        };

        for result in &results {
            let session_id = result["session_id"].as_str().unwrap_or_default();
            let title = result["title"].as_str().unwrap_or("Untitled");
            let category = result["task_category"].as_str().unwrap_or("other");
            let description = result["description"].as_str().unwrap_or("");
            let complexity = result["complexity"].as_str().unwrap_or("moderate");
            let fm_version = result["fm_version"].as_str();
            if fm_version == Some("apple-fm-on-device") {
                fm_generated += 1;
            }

            db.update_summary(
                session_id,
                title,
                category,
                description,
                complexity,
                fm_version,
            )
            .map_err(|e| anyhow::anyhow!("Failed to save summary for {}: {}", session_id, e))?;
        }

        total_summarized += results.len();
    }

    if backend == "apple-fm" {
        let heuristic = total_summarized.saturating_sub(fm_generated);
        eprintln!(
            "\n  {} {} sessions summarized ({} via Apple FM, {} heuristic)",
            "✓".green(),
            total_summarized,
            fm_generated,
            heuristic
        );
    } else {
        eprintln!(
            "\n  {} {} sessions summarized",
            "✓".green(),
            total_summarized
        );
    }

    Ok(())
}

const GROUPING_SYSTEM_PROMPT: &str = r#"You are a task grouping assistant. Given a list of coding session titles, group them into high-level project tasks (2-5 words each).

Rules:
- Group related sessions under a single short label (e.g. "Kiro Auth", "Tokscale Report", "System Config")
- Each group should represent a coherent project or feature area
- Sessions that don't fit any group get their own group name
- Aim for 3-8 groups total. Fewer is better.

Respond ONLY with a JSON array where each element has: session_id, task_group"#;

fn run_task_grouping(db: &WikiDb, entries: &[WikiEntry], backend: &str) -> Result<()> {
    let summarized: Vec<&WikiEntry> = entries
        .iter()
        .filter(|e| e.title.is_some() && e.task_group.is_none())
        .collect();

    if summarized.is_empty() {
        return Ok(());
    }

    // Non-CLI backends (apple-fm and any future on-device backend) have no LLM
    // grouping path. Rather than skip — which leaves every task_group null and
    // makes the report collapse sessions by EXACT title — cluster titles
    // deterministically in Rust. This merges near-duplicate titles ("Enhance API
    // Security" / "Enhance API security with JWT auth middleware") into a single
    // labeled group while keeping unrelated titles apart.
    if !matches!(backend, "claude" | "codex" | "gemini" | "kiro") {
        let assignments = cluster_titles(&summarized);
        let group_count = assignments
            .iter()
            .map(|(_, label)| label.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len();
        for (session_id, label) in &assignments {
            db.update_task_group(session_id, label).map_err(|e| {
                anyhow::anyhow!("Failed to save task_group for {}: {}", session_id, e)
            })?;
        }
        eprintln!(
            "  {} grouped {} sessions into {} tasks",
            "✓".green(),
            summarized.len(),
            group_count
        );
        return Ok(());
    }

    eprint!(
        "  Grouping {} sessions into tasks...",
        summarized.len().to_string().cyan()
    );

    let mut parts = Vec::new();
    parts.push("Group these coding sessions by project/feature:\n".to_string());
    for (i, entry) in summarized.iter().enumerate() {
        parts.push(format!(
            "  {} (id: {}): {} [{}]",
            i + 1,
            entry.session_id,
            entry.title.as_deref().unwrap_or("?"),
            entry.workspace.as_deref().unwrap_or("?"),
        ));
    }
    parts.push("\nRespond with a JSON array.".to_string());
    let prompt = parts.join("\n");

    let cmd = match backend {
        "claude" => {
            let mut c = Command::new("claude");
            c.args(["-p", "--output-format", "text"])
                .arg(format!("System: {}\n\n{}", GROUPING_SYSTEM_PROMPT, prompt));
            c
        }
        "codex" => {
            let mut c = Command::new("codex");
            c.args(["exec"])
                .arg(format!("{}\n\n{}", GROUPING_SYSTEM_PROMPT, prompt));
            c
        }
        "gemini" => {
            let mut c = Command::new("gemini");
            c.args(["-p"])
                .arg(format!("{}\n\n{}", GROUPING_SYSTEM_PROMPT, prompt));
            c
        }
        "kiro" => {
            let mut c = Command::new("kiro-cli");
            c.args(["chat", "--no-interactive"])
                .arg(format!("{}\n\n{}", GROUPING_SYSTEM_PROMPT, prompt));
            c
        }
        // Non-CLI backends were already handled by the title-clustering path
        // above (which early-returns), so only the four CLI backends reach here.
        other => unreachable!("non-CLI backend '{}' must be handled by clustering", other),
    };

    // A timed-out (or otherwise un-spawnable) backend must degrade gracefully:
    // skip grouping and continue the report rather than aborting it.
    let output = match run_command_with_timeout(cmd, BACKEND_TIMEOUT, None) {
        Ok(output) => output,
        Err(e) => {
            eprintln!("\n  {} grouping failed: {}", "⚠".yellow(), e);
            return Ok(());
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("\n  {} grouping failed: {}", "⚠".yellow(), stderr.trim());
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_str = extract_json_array(&stdout);

    match serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
        Ok(results) => {
            for result in &results {
                let session_id = result["session_id"].as_str().unwrap_or_default();
                let task_group = result["task_group"].as_str().unwrap_or_default();
                if !session_id.is_empty() && !task_group.is_empty() {
                    db.update_task_group(session_id, task_group).map_err(|e| {
                        anyhow::anyhow!("Failed to save task_group for {}: {}", session_id, e)
                    })?;
                }
            }
            eprintln!(" {}", "✓".green());
        }
        Err(e) => {
            eprintln!(
                "\n  {} Failed to parse grouping response: {}",
                "⚠".yellow(),
                e
            );
        }
    }

    Ok(())
}

/// Generic verbs and stopwords stripped from titles before clustering. These
/// carry no signal about *which* project/feature a session touched (every other
/// session "adds" or "fixes" something), so keeping them would make unrelated
/// titles look similar.
const CLUSTER_STOPWORDS: &[&str] = &[
    "add",
    "fix",
    "fixes",
    "fixed",
    "update",
    "updates",
    "refactor",
    "improve",
    "implement",
    "enhance",
    "create",
    "remove",
    "the",
    "a",
    "an",
    "to",
    "for",
    "with",
    "and",
    "of",
    "in",
    "on",
    "via",
];

/// Reduce a title to its set of SIGNIFICANT tokens: lowercase, strip
/// punctuation/ellipsis, collapse whitespace, drop generic verbs/stopwords.
/// The returned tokens are deduplicated and sorted so two titles with the same
/// significant words (in any order) produce equal sets.
fn is_combining_mark(c: char) -> bool {
    matches!(
        c as u32,
        0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x20d0..=0x20ff | 0xfe20..=0xfe2f
    )
}

fn significant_tokens(title: &str) -> Vec<String> {
    let mut tokens: Vec<String> = title
        // Normalize to NFC FIRST so canonically-equivalent inputs (precomposed
        // "é" vs base "e" + combining acute) collapse to the same code points
        // before lowercasing and combining-mark stripping. Without this, an NFC
        // title kept the precomposed letter while an NFD one had its combining
        // mark stripped, tokenizing identical titles differently and splitting
        // them across clusters.
        .nfc()
        .collect::<String>()
        .to_lowercase()
        .chars()
        // Lowercase before filtering: Unicode lowercase can expand a character
        // into a base letter plus combining mark (e.g. "İ" -> "i" + dot).
        // Dropping combining marks here keeps case-only variants token-equal.
        // Other punctuation/ellipsis maps to spaces, stripping trailing "…" or
        // "..." that the on-device model often emits.
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c)
            } else if is_combining_mark(c) {
                None
            } else {
                Some(' ')
            }
        })
        .collect::<String>()
        .split_whitespace()
        .map(|t| t.to_string())
        .filter(|t| !CLUSTER_STOPWORDS.contains(&t.as_str()))
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}

/// Normalize a title for exact-equality grouping of token-empty titles:
/// full Unicode lowercase with whitespace collapsed. Used only to keep
/// identical all-stopword titles together while keeping distinct ones apart.
fn normalized_title(title: &str) -> String {
    title
        // Match significant_tokens: normalize to NFC before lowercasing so
        // canonically-equivalent all-stopword titles share an exact key.
        .nfc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Similarity threshold for treating two titles as the same task. Applied to
/// the overlap coefficient (shared / size-of-smaller-set), which — unlike a raw
/// shared-token count — scales with how much of the smaller title is covered.
const CLUSTER_SIMILARITY_THRESHOLD: f64 = 0.6;

/// Two token sets are considered the same task when they overlap strongly,
/// measured by the OVERLAP COEFFICIENT: `shared / min(|a|, |b|)`.
///
/// This deliberately replaces the old "share ≥ 2 tokens" rule, which fired on
/// any two long-but-unrelated titles that happened to share two common words
/// (e.g. "add" + "service") and — combined with union-growing cluster
/// signatures — let a single cluster transitively swallow everything.
///
/// The overlap coefficient stays high (1.0) when a short title is fully
/// contained in a longer one ("Enhance API Security" ⊂ "Enhance API security
/// with JWT auth middleware"), so genuine near-duplicates still merge, while two
/// large sets that share only a couple of incidental tokens score low and stay
/// apart.
fn tokens_overlap(a: &[String], b: &[String]) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let shared = a.iter().filter(|t| b.contains(t)).count();
    let smaller = a.len().min(b.len());
    // A single-token set scores a perfect 1.0 against any longer title that
    // merely contains that token, so a generic summary like "API" / "Fix API"
    // would cluster with every unrelated "Add API auth", "Update API billing",
    // etc. Require at least TWO shared tokens whenever the smaller set has only
    // one token, so singletons need real signal before merging.
    if smaller <= 1 {
        return shared >= 2;
    }
    (shared as f64 / smaller as f64) >= CLUSTER_SIMILARITY_THRESHOLD
}

/// Deterministically cluster summarized entries by title similarity and return
/// `(session_id, group_label)` for every entry. Greedy O(n²) clustering — n is
/// small (one report's worth of sessions). Each cluster is labeled with its most
/// frequent original title, tie-broken by shortest, so the label is a real
/// human-readable title rather than a synthetic key.
fn cluster_titles(entries: &[&WikiEntry]) -> Vec<(String, String)> {
    struct Cluster {
        // Token sets of EVERY member, kept separately rather than unioned into a
        // single signature. A candidate joins if it overlaps ANY member, which
        // preserves transitive grouping of genuine near-duplicates WITHOUT the
        // union ballooning into a catch-all that absorbs unrelated titles.
        member_tokens: Vec<Vec<String>>,
        // Exact normalized title shared by a token-empty cluster (all-stopword
        // titles). `None` for tokened clusters.
        empty_key: Option<String>,
        members: Vec<usize>,
    }

    // Precompute significant tokens + normalized title once per entry.
    let prepared: Vec<(usize, Vec<String>, String)> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let raw = e.title.as_deref().unwrap_or("");
            (i, significant_tokens(raw), normalized_title(raw))
        })
        .collect();

    let mut clusters: Vec<Cluster> = Vec::new();
    for (idx, tokens, norm) in &prepared {
        // Find the first existing cluster this entry overlaps strongly with.
        let mut placed = false;
        for cluster in clusters.iter_mut() {
            let matches = if tokens.is_empty() {
                // Token-empty titles (all stopwords) carry no signal to cluster
                // on, so they only merge with an identical normalized title.
                // This stops every generic/stopword-only title from collapsing
                // into one arbitrary blob.
                cluster.empty_key.as_deref() == Some(norm.as_str())
            } else {
                // Tokened entries never join an empty cluster; they match if they
                // overlap ANY existing member of the cluster.
                cluster.empty_key.is_none()
                    && cluster
                        .member_tokens
                        .iter()
                        .any(|m| tokens_overlap(tokens, m))
            };
            if matches {
                cluster.members.push(*idx);
                cluster.member_tokens.push(tokens.clone());
                placed = true;
                break;
            }
        }
        if !placed {
            clusters.push(Cluster {
                member_tokens: vec![tokens.clone()],
                empty_key: if tokens.is_empty() {
                    Some(norm.clone())
                } else {
                    None
                },
                members: vec![*idx],
            });
        }
    }

    // Consolidation pass: the single greedy pass above is order-dependent — an
    // entry compared before its eventual neighbor was seen can land in its own
    // cluster even though it overlaps a member of another cluster. Repeatedly
    // merge any two clusters that have a pair of overlapping members (or, for
    // token-empty clusters, an identical normalized title) until a fixpoint,
    // making the final grouping independent of input order. n is small, so the
    // O(n²)-per-round loop is cheap. Crucially, merging only happens on a real
    // member-to-member overlap, so it cannot chain unrelated clusters together.
    loop {
        let mut merged_any = false;
        'outer: for i in 0..clusters.len() {
            for j in (i + 1)..clusters.len() {
                let overlap = match (&clusters[i].empty_key, &clusters[j].empty_key) {
                    // Token-empty clusters merge only with an identical title.
                    (Some(ki), Some(kj)) => ki == kj,
                    // A token-empty cluster never merges with a tokened one.
                    (Some(_), None) | (None, Some(_)) => false,
                    // Tokened clusters merge if any member pair overlaps.
                    (None, None) => clusters[i].member_tokens.iter().any(|mi| {
                        clusters[j]
                            .member_tokens
                            .iter()
                            .any(|mj| tokens_overlap(mi, mj))
                    }),
                };
                if overlap {
                    let other = clusters.remove(j);
                    clusters[i].members.extend(other.members);
                    clusters[i].member_tokens.extend(other.member_tokens);
                    merged_any = true;
                    break 'outer;
                }
            }
        }
        if !merged_any {
            break;
        }
    }

    let mut assignments = Vec::new();
    for cluster in &clusters {
        let label = cluster_label(entries, &cluster.members);
        for &idx in &cluster.members {
            assignments.push((entries[idx].session_id.clone(), label.clone()));
        }
    }
    assignments
}

/// Pick a human-readable label for a cluster: the most frequent original title,
/// tie-broken by shortest (char count), then lexicographically for full
/// determinism.
fn cluster_label(entries: &[&WikiEntry], members: &[usize]) -> String {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for &idx in members {
        let title = entries[idx]
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or("(unsummarized)");
        *counts.entry(title).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by(|(at, ac), (bt, bc)| {
            ac.cmp(bc)
                // Higher frequency wins; on a tie prefer the SHORTER title, then
                // lexicographically smaller, so the label is stable run-to-run.
                .then_with(|| bt.chars().count().cmp(&at.chars().count()))
                .then_with(|| bt.cmp(at))
        })
        .map(|(title, _)| title.to_string())
        .unwrap_or_else(|| "(unsummarized)".to_string())
}

fn run_apple_fm_summarizer(payloads: &[serde_json::Value]) -> Result<Vec<serde_json::Value>> {
    // Build typed inputs from the JSON payloads.
    let inputs: Vec<apple_fm::SessionInput> = payloads
        .iter()
        .map(|p| apple_fm::SessionInput {
            session_id: p["session_id"].as_str().unwrap_or_default().to_string(),
            client: p["client"].as_str().unwrap_or_default().to_string(),
            workspace: p["workspace"].as_str().unwrap_or_default().to_string(),
            first_user_message: p["first_user_message"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
            models_used: p["models_used"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            total_tokens: p["total_tokens"].as_i64().unwrap_or(0),
            duration_minutes: p["duration_minutes"].as_i64().unwrap_or(0),
            message_count: p["message_count"].as_i64().unwrap_or(0),
        })
        .collect();

    // `summarize` returns `Some` only when Apple Intelligence is available and
    // the feature is enabled on macOS. In every other case (unavailable,
    // feature-off, or non-macOS) it returns `None` and we apply the Rust
    // heuristic to all sessions. apple-fm therefore stays the default backend
    // and degrades gracefully cross-platform — it never errors out the report.
    let summaries: Vec<apple_fm::SessionSummary> = match apple_fm::summarize(&inputs) {
        Some(v) => v,
        None => inputs.iter().map(apple_fm::heuristic_classify).collect(),
    };

    // Provenance is carried PER summary (`s.fm_version`): even when Apple FM is
    // available, an individual generation that fails/times out is backfilled with
    // the heuristic and must not be recorded as `apple-fm-on-device`.
    let results = summaries
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "session_id": s.session_id,
                "title": s.title,
                "task_category": s.task_category,
                "description": s.description,
                "complexity": s.complexity,
                "fm_version": s.fm_version,
            })
        })
        .collect();

    Ok(results)
}

const SUMMARIZER_SYSTEM_PROMPT: &str = r#"You are a coding session classifier. Given metadata about an AI coding session, produce a structured summary.

Rules:
- title: 3-8 word description of what was done (imperative mood, e.g. "Add JWT auth middleware")
- task_category: exactly one of: feature, bugfix, refactor, research, debug, review, docs, config, other
- description: 1-2 sentences explaining what happened in the session
- complexity: exactly one of: trivial, moderate, complex

Respond ONLY with a JSON array where each element has: session_id, title, task_category, description, complexity."#;

fn build_cli_prompt(payloads: &[serde_json::Value]) -> String {
    let mut parts = Vec::new();
    parts.push("Classify these coding sessions:\n".to_string());
    for (i, p) in payloads.iter().enumerate() {
        parts.push(format!(
            "Session {} (id: {}):\n  Workspace: {}\n  Client: {}\n  Models: {}\n  Tokens: {}\n  Duration: {} min\n  Messages: {}\n  First message: {}\n",
            i + 1,
            p["session_id"].as_str().unwrap_or("?"),
            p["workspace"].as_str().unwrap_or("?"),
            p["client"].as_str().unwrap_or("?"),
            p["models_used"],
            p["total_tokens"],
            p["duration_minutes"],
            p["message_count"],
            p["first_user_message"].as_str().unwrap_or("(none)").chars().take(200).collect::<String>(),
        ));
    }
    parts.push("Respond with a JSON array.".to_string());
    parts.join("\n")
}

fn run_cli_summarizer(
    backend: &str,
    payloads: &[serde_json::Value],
) -> Result<Vec<serde_json::Value>> {
    let prompt = build_cli_prompt(payloads);

    let cmd = match backend {
        "claude" => {
            let mut c = Command::new("claude");
            c.args(["-p", "--output-format", "text"]).arg(format!(
                "System: {}\n\n{}",
                SUMMARIZER_SYSTEM_PROMPT, prompt
            ));
            c
        }
        "codex" => {
            let mut c = Command::new("codex");
            c.args(["exec"])
                .arg(format!("{}\n\n{}", SUMMARIZER_SYSTEM_PROMPT, prompt));
            c
        }
        "gemini" => {
            let mut c = Command::new("gemini");
            c.args(["-p"])
                .arg(format!("{}\n\n{}", SUMMARIZER_SYSTEM_PROMPT, prompt));
            c
        }
        "kiro" => {
            let mut c = Command::new("kiro-cli");
            c.args(["chat", "--no-interactive"])
                .arg(format!("{}\n\n{}", SUMMARIZER_SYSTEM_PROMPT, prompt));
            c
        }
        _ => return Ok(Vec::new()),
    };

    // A timed-out (or un-spawnable) backend must degrade gracefully: log it and
    // return no summaries so the caller continues, matching the non-zero-exit
    // path below.
    let output = match run_command_with_timeout(cmd, BACKEND_TIMEOUT, None) {
        Ok(output) => output,
        Err(e) => {
            eprintln!("  {} {} summarizer failed: {}", "⚠".yellow(), backend, e);
            return Ok(Vec::new());
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!(
            "  {} {} summarizer failed: {}",
            "⚠".yellow(),
            backend,
            stderr.trim()
        );
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_str = extract_json_array(&stdout);

    match serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
        Ok(results) => Ok(results),
        Err(e) => {
            eprintln!(
                "  {} Failed to parse {} response: {}",
                "⚠".yellow(),
                backend,
                e
            );
            Ok(Vec::new())
        }
    }
}

/// Upper bound on how long any LLM summarizer subprocess may run before we
/// kill it. Deliberately generous (5 min) so legitimate batched LLM calls
/// never trip it, but it bounds a true hang (auth prompt, network stall) so
/// `tokscale report` can never block forever.
const BACKEND_TIMEOUT: Duration = Duration::from_secs(300);

/// Spawn `cmd`, optionally write `stdin_bytes` to its stdin, and wait up to
/// `timeout` for it to finish.
///
/// Mirrors the pure-std spawn + reader-thread + `try_wait()` deadline + kill
/// approach used by `run_capture_command` in `main.rs` (no extra dependency).
/// Both stdout and stderr are drained on dedicated threads so a chatty backend
/// cannot deadlock on a full pipe buffer while we poll for exit.
///
/// On timeout the child is killed and an `io::Error` of kind `TimedOut` is
/// returned, which callers treat as a recoverable "skip this backend" signal.
fn run_command_with_timeout(
    mut cmd: Command,
    timeout: Duration,
    stdin_bytes: Option<&[u8]>,
) -> std::io::Result<Output> {
    use std::io::Read;
    use std::thread;
    use std::time::Instant;

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin_bytes.is_some() {
        cmd.stdin(Stdio::piped());
    }

    let mut child = cmd.spawn()?;

    // Write to stdin (if requested) before draining output, then drop the
    // handle so the child sees EOF.
    if let Some(bytes) = stdin_bytes {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(bytes)?;
        }
    }

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("failed to capture subprocess stdout"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("failed to capture subprocess stderr"))?;

    let stdout_handle = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf)?;
        Ok(buf)
    });
    let stderr_handle = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        stderr.read_to_end(&mut buf)?;
        Ok(buf)
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "summarizer backend timed out",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    };

    let stdout = stdout_handle
        .join()
        .map_err(|_| std::io::Error::other("subprocess stdout reader thread panicked"))??;
    let stderr = stderr_handle
        .join()
        .map_err(|_| std::io::Error::other("subprocess stderr reader thread panicked"))??;

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn extract_json_array(text: &str) -> &str {
    if let Some(start) = text.find('[') {
        if let Some(end) = text.rfind(']') {
            return &text[start..=end];
        }
    }
    text
}

fn print_report_table(
    entries: &[WikiEntry],
    _db: &WikiDb,
    is_multi_day: bool,
    full: bool,
) -> Result<()> {
    if entries.is_empty() {
        println!("No sessions found for the given filters.");
        return Ok(());
    }

    let total_cost: f64 = entries.iter().map(|e| e.total_cost).sum();
    let total_tokens: i64 = entries
        .iter()
        .map(|e| e.total_input_tokens.saturating_add(e.total_output_tokens))
        .fold(0i64, i64::saturating_add);
    let total_sessions = entries.len();
    let summarized = entries.iter().filter(|e| e.title.is_some()).count();

    println!();
    println!(
        "  {} sessions | {} summarized | ${:.2} total | {} tokens",
        total_sessions.to_string().cyan(),
        summarized.to_string().green(),
        total_cost,
        format_tokens(total_tokens).yellow(),
    );
    println!();

    let mut by_model: HashMap<&str, (f64, i64, usize)> = HashMap::new();
    for entry in entries {
        if entry.models_used.is_empty() {
            continue;
        }
        for model in &entry.models_used {
            let agg = by_model.entry(model.as_str()).or_insert((0.0, 0, 0));
            agg.0 += entry.total_cost / entry.models_used.len() as f64;
            agg.1 = agg.1.saturating_add(
                entry
                    .total_input_tokens
                    .saturating_add(entry.total_output_tokens)
                    / entry.models_used.len() as i64,
            );
            agg.2 += 1;
        }
    }

    let mut models: Vec<_> = by_model.iter().collect();
    models.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));

    println!(
        "  {:<30} {:>8} {:>12} {:>8}",
        "Model", "Sessions", "Tokens", "Cost"
    );
    println!("  {}", "─".repeat(62));
    for (model, (cost, tokens, count)) in &models {
        println!(
            "  {:<30} {:>8} {:>12} {:>8}",
            model,
            count,
            format_tokens(*tokens),
            format!("${:.2}", cost),
        );
    }
    println!("  {}", "─".repeat(62));
    println!(
        "  {:<30} {:>8} {:>12} {:>8}",
        "TOTAL",
        total_sessions,
        format_tokens(total_tokens),
        format!("${:.2}", total_cost),
    );
    println!();

    let mut by_group: HashMap<&str, (f64, i64, usize, Vec<&str>)> = HashMap::new();
    for entry in entries {
        let group = entry
            .task_group
            .as_deref()
            .unwrap_or(entry.title.as_deref().unwrap_or("(unsummarized)"));
        let title = entry.title.as_deref().unwrap_or("(unsummarized)");
        let agg = by_group.entry(group).or_insert((0.0, 0, 0, Vec::new()));
        agg.0 += entry.total_cost;
        agg.1 = agg.1.saturating_add(
            entry
                .total_input_tokens
                .saturating_add(entry.total_output_tokens),
        );
        agg.2 += 1;
        if !agg.3.contains(&title) {
            agg.3.push(title);
        }
    }

    let mut groups: Vec<_> = by_group.iter().collect();
    groups.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));

    println!(
        "  {:<40} {:>5} {:>10} {:>8}",
        "Task Group", "Sess", "Tokens", "Cost"
    );
    println!("  {}", "─".repeat(67));
    for (group, (cost, tokens, count, titles)) in groups.iter().take(15) {
        let display_group: String = if group.chars().count() > 40 {
            format!("{}…", group.chars().take(39).collect::<String>())
        } else {
            group.to_string()
        };
        println!(
            "  {:<40} {:>5} {:>10} {:>8}",
            display_group.bold(),
            count,
            format_tokens(*tokens),
            format!("${:.2}", cost),
        );
        if *count > 1 {
            for t in titles.iter().take(3) {
                let display_t: String = if t.chars().count() > 38 {
                    t.chars().take(38).collect::<String>()
                } else {
                    t.to_string()
                };
                println!("    {}", display_t.dimmed());
            }
            if titles.len() > 3 {
                println!("    … +{} more", titles.len() - 3);
            }
        }
    }
    if groups.len() > 15 {
        let rest_count: usize = groups.iter().skip(15).map(|(_, v)| v.2).sum();
        let rest_cost: f64 = groups.iter().skip(15).map(|(_, v)| v.0).sum();
        let rest_tokens: i64 = groups.iter().skip(15).map(|(_, v)| v.1).sum();
        println!(
            "  {:<40} {:>5} {:>10} {:>8}",
            format!("… +{} more", groups.len() - 15),
            rest_count,
            format_tokens(rest_tokens),
            format!("${:.2}", rest_cost),
        );
    }
    println!("  {}", "─".repeat(67));
    println!();

    if is_multi_day {
        print_daily_breakdown(entries, full);
    } else {
        print_session_list(entries, full);
    }

    Ok(())
}

fn print_daily_breakdown(entries: &[WikiEntry], full: bool) {
    use std::collections::BTreeMap;

    let mut by_date: BTreeMap<String, (f64, i64, usize, Vec<&WikiEntry>)> = BTreeMap::new();
    for entry in entries {
        let date_key = Local
            .timestamp_millis_opt(entry.created_at)
            .single()
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let agg = by_date.entry(date_key).or_insert((0.0, 0, 0, Vec::new()));
        agg.0 += entry.total_cost;
        agg.1 = agg.1.saturating_add(
            entry
                .total_input_tokens
                .saturating_add(entry.total_output_tokens),
        );
        agg.2 += 1;
        agg.3.push(entry);
    }

    let mut dates: Vec<_> = by_date.iter().collect();
    dates.sort_by(|a, b| b.0.cmp(a.0));

    println!("  Daily breakdown:");
    println!("  {}", "─".repeat(72));
    for (date, (cost, tokens, count, sessions)) in &dates {
        println!(
            "  {} {:>3} sessions  {:>10} tokens  {:>8}",
            date.cyan(),
            count,
            format_tokens(*tokens),
            format!("${:.2}", cost),
        );
        let daily_limit = if full { sessions.len() } else { 5 };
        for s in sessions.iter().take(daily_limit) {
            let title = s.title.as_deref().unwrap_or("(pending)");
            let model = s.models_used.first().map(|m| m.as_str()).unwrap_or("-");
            let display_title: String = if title.chars().count() > 40 {
                title.chars().take(40).collect::<String>()
            } else {
                title.to_string()
            };
            println!(
                "    {:>6} {:<18} {}",
                format!("${:.2}", s.total_cost),
                model.dimmed(),
                display_title,
            );
        }
        if sessions.len() > 5 {
            println!("    … +{} more sessions", sessions.len() - 5);
        }
    }
    println!();
}

fn print_session_list(entries: &[WikiEntry], full: bool) {
    let list_limit = if full { entries.len() } else { 10 };
    let recent: Vec<&WikiEntry> = entries.iter().take(list_limit).collect();
    if !recent.is_empty() {
        println!("  Sessions:");
        println!("  {}", "─".repeat(80));
        for entry in recent {
            let date = Local
                .timestamp_millis_opt(entry.created_at)
                .single()
                .map(|dt| dt.format("%H:%M").to_string())
                .unwrap_or_else(|| "??:??".to_string());

            let title = entry.title.as_deref().unwrap_or("(pending summarization)");
            let model = entry.models_used.first().map(|s| s.as_str()).unwrap_or("-");
            let cost = format!("${:.2}", entry.total_cost);

            println!(
                "  {} {:>6} {:<20} {}",
                date.dimmed(),
                cost,
                model.dimmed(),
                title,
            );
        }
        if !full && entries.len() > 10 {
            println!("    … +{} more sessions", entries.len() - 10);
        }
        println!();
    }
}

/// Maps every locally-discovered session to the on-disk file(s) its content can
/// be extracted from, so the summarizer payload carries the real first user
/// message instead of metadata only.
///
/// File-keyed clients (claude, codex, gemini) live as one transcript file per
/// session, indexed here by `(client, session_id)`. The client is part of the
/// key so cross-client `session_id` collisions can't feed one client's file to
/// another client's extractor. OpenCode sessions live as rows inside a shared
/// SQLite database, so every opencode database is kept as a candidate and the
/// extractor selects the matching session internally.
#[derive(Default)]
struct SessionPathIndex {
    by_client_session: HashMap<(String, String), Vec<PathBuf>>,
    opencode_dbs: Vec<PathBuf>,
}

impl SessionPathIndex {
    /// Candidate file(s) to feed the dispatcher for `(client, session_id)`.
    fn candidates_for(&self, client: &str, session_id: &str) -> Vec<PathBuf> {
        if client == "opencode" {
            return self.opencode_dbs.clone();
        }
        self.by_client_session
            .get(&(client.to_string(), session_id.to_string()))
            .cloned()
            .unwrap_or_default()
    }
}

/// Scan local client data once and index every session file by
/// `(client, session_id)` plus the OpenCode databases, so per-session content
/// extraction never has to re-walk the filesystem. Scanning is best-effort: any
/// client the summarizer can't extract simply yields no candidates and falls
/// back to metadata-only.
///
/// The `session_id` used as the key must match how the wiki populates its
/// entries. For most clients that is the file stem, but Gemini transcripts derive
/// the id from the in-file `sessionId`/`session_id` field, so they are keyed by
/// the parsed id (with the stem kept as a fallback alias).
fn build_session_path_index(opts: &ReportOptions) -> SessionPathIndex {
    let home_dir = opts
        .home_dir
        .clone()
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_default();
    let use_env_roots = opts.home_dir.is_none();

    let scan = tokscale_core::scanner::scan_all_clients_with_scanner_settings(
        &home_dir,
        &[],
        use_env_roots,
        &opts.scanner_settings,
    );

    let mut by_client_session: HashMap<(String, String), Vec<PathBuf>> = HashMap::new();
    for (client, path) in scan.all_files() {
        let client_str = client.as_str().to_string();
        let mut session_ids: Vec<String> = Vec::new();

        if client == tokscale_core::ClientId::Gemini {
            // Gemini's wiki session_id comes from inside the file, not the stem.
            if let Some(id) = tokscale_core::sessions::gemini::gemini_session_id_for_file(&path) {
                session_ids.push(id);
            }
        }
        // Always keep the file stem as a key/alias so stem-keyed clients work and
        // Gemini lookups still resolve if the wiki used the stem.
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            session_ids.push(stem.to_string());
        }

        session_ids.sort();
        session_ids.dedup();
        for session_id in session_ids {
            by_client_session
                .entry((client_str.clone(), session_id))
                .or_default()
                .push(path.clone());
        }
    }

    SessionPathIndex {
        by_client_session,
        opencode_dbs: scan.opencode_dbs.clone(),
    }
}

/// Resolve a session's real content by dispatching to the correct per-client
/// extractor over its on-disk file(s). Falls back to metadata-only when the
/// client is unsupported or no candidate file yields a first user message.
fn extract_content_for_session(
    entry: &WikiEntry,
    session_paths: &SessionPathIndex,
) -> SessionContent {
    let candidates = session_paths.candidates_for(&entry.client, &entry.session_id);
    if candidates.is_empty() {
        return metadata_only_content(&entry.session_id, &entry.client);
    }
    extract_session_content(&entry.client, &entry.session_id, &candidates)
}

fn parse_date_range(since: &Option<String>, until: &Option<String>) -> (Option<i64>, Option<i64>) {
    // The `since`/`until` strings are local-calendar dates (e.g. produced by
    // `build_date_filter`, which derives them from `chrono::Local::now()`), and
    // session dates are bucketed in local time (see
    // `sessions::timestamp_to_date`). Interpret the day boundaries in local time
    // so filtering lines up with grouping and avoids off-by-a-day mismatches.
    let since_ts = since
        .as_ref()
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .and_then(local_start_of_day_millis);
    let until_ts = until
        .as_ref()
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .and_then(|d| d.succ_opt())
        .and_then(|next| local_start_of_day_millis(next).map(|ms| ms - 1));
    (since_ts, until_ts)
}

/// Returns the Unix-millisecond timestamp for the start of `date` in the local
/// timezone.
///
/// This is normally midnight (00:00:00), but in zones that spring forward at
/// local midnight (e.g. `America/Nuuk` on `2024-03-31`) that wall-clock time
/// does not exist. Rather than dropping the boundary (which would silently make
/// date filtering unbounded), we walk forward to the first representable instant
/// after the gap so the day boundary is preserved.
fn local_start_of_day_millis(date: chrono::NaiveDate) -> Option<i64> {
    start_of_day_millis_with(date, |wall| Local.from_local_datetime(wall))
}

/// Core of [`local_start_of_day_millis`], parameterized over the timezone
/// resolver so the DST-gap handling can be exercised deterministically in tests.
///
/// Starts at midnight and, when that wall-clock time is skipped (a spring-forward
/// gap), walks forward in 1-minute steps to the first representable instant. The
/// probe window covers a full day so even unusual offsets resolve rather than
/// silently dropping the boundary.
fn start_of_day_millis_with<F>(date: chrono::NaiveDate, resolve: F) -> Option<i64>
where
    F: Fn(&chrono::NaiveDateTime) -> chrono::LocalResult<chrono::DateTime<Local>>,
{
    let mut wall = date.and_hms_opt(0, 0, 0)?;
    for _ in 0..=(24 * 60) {
        match resolve(&wall) {
            chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
                return Some(dt.timestamp_millis());
            }
            chrono::LocalResult::None => {
                wall += chrono::Duration::minutes(1);
            }
        }
    }
    None
}

/// Loads the canonical pricing dataset for cost attribution, preferring a fresh
/// fetch but falling back to any cached dataset so reports still work offline.
/// Returns `None` only when no pricing data is available at all.
fn load_pricing_service() -> Option<std::sync::Arc<PricingService>> {
    let fresh = tokio::runtime::Runtime::new()
        .ok()
        .and_then(|rt| rt.block_on(async { PricingService::get_or_init().await.ok() }));
    fresh.or_else(|| PricingService::load_cached_any_age().map(std::sync::Arc::new))
}

/// Computes a message's cost using the canonical [`PricingService`], honoring
/// per-model rates and every billed token type (input/output/cache read/cache
/// write/reasoning). Returns 0.0 when no pricing dataset is available.
fn compute_msg_cost(msg: &ParsedMessage, pricing: Option<&PricingService>) -> f64 {
    let Some(pricing) = pricing else {
        return 0.0;
    };
    pricing.calculate_cost_with_provider(
        &msg.model_id,
        Some(&msg.provider_id),
        &TokenBreakdown {
            input: msg.input,
            output: msg.output,
            cache_read: msg.cache_read,
            cache_write: msg.cache_write,
            reasoning: msg.reasoning,
        },
    )
}

fn format_tokens(tokens: i64) -> String {
    if tokens >= 1_000_000_000 {
        format!("{:.1}B", tokens as f64 / 1_000_000_000.0)
    } else if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.0}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

struct SessionAgg {
    client: String,
    workspace: Option<String>,
    workspace_label: Option<String>,
    created_at: i64,
    last_active: i64,
    total_input: i64,
    total_output: i64,
    total_cache_read: i64,
    total_cost: f64,
    models: HashMap<String, i32>,
    message_count: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokscale_core::pricing::{ModelPricing, PricingService};

    fn test_pricing_service() -> PricingService {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-haiku-4".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.000004),
                output_cost_per_token: Some(0.000006),
                cache_read_input_token_cost: Some(0.000001),
                ..Default::default()
            },
        );
        PricingService::new(litellm, HashMap::new())
    }

    fn parsed_message(model_id: &str) -> ParsedMessage {
        ParsedMessage {
            client: "claude".to_string(),
            model_id: model_id.to_string(),
            provider_id: "anthropic".to_string(),
            session_id: "s1".to_string(),
            workspace_key: None,
            workspace_label: None,
            timestamp: 0,
            date: "2026-01-01".to_string(),
            input: 1_000,
            output: 500,
            cache_read: 2_000,
            cache_write: 0,
            reasoning: 0,
            duration_ms: None,
            message_count: 1,
            agent: None,
        }
    }

    #[test]
    fn compute_msg_cost_matches_canonical_pricing_service() {
        let pricing = test_pricing_service();
        let msg = parsed_message("claude-haiku-4");

        let report_cost = compute_msg_cost(&msg, Some(&pricing));
        let canonical = pricing.calculate_cost_with_provider(
            &msg.model_id,
            Some(&msg.provider_id),
            &TokenBreakdown {
                input: msg.input,
                output: msg.output,
                cache_read: msg.cache_read,
                cache_write: msg.cache_write,
                reasoning: msg.reasoning,
            },
        );

        // The report must price exactly what PricingService yields — no
        // hardcoded flat rates, no fuzzy matching.
        assert_eq!(report_cost, canonical);
        assert!(
            canonical > 0.0,
            "expected a positive cost for a known model"
        );
    }

    #[test]
    fn parse_date_range_buckets_in_local_time() {
        use chrono::{Local, TimeZone};

        // Pick an arbitrary calendar day. The exact day is irrelevant; what
        // matters is that `parse_date_range` interprets the boundaries in the
        // *local* timezone, matching how `sessions::timestamp_to_date` buckets
        // each message (and how `build_date_filter` derives these strings from
        // `chrono::Local::now()`).
        let day = "2026-03-08";
        let (since, until) = parse_date_range(&Some(day.into()), &Some(day.into()));

        let expected_since = Local
            .with_ymd_and_hms(2026, 3, 8, 0, 0, 0)
            .single()
            .map(|dt| dt.timestamp_millis())
            .expect("local midnight exists for this fixed date");
        // The window is inclusive of the whole local day: [00:00:00.000,
        // next-day 00:00:00.000 - 1ms].
        let expected_until = Local
            .with_ymd_and_hms(2026, 3, 9, 0, 0, 0)
            .single()
            .map(|dt| dt.timestamp_millis() - 1)
            .expect("local midnight exists for this fixed date");

        assert_eq!(since, Some(expected_since));
        assert_eq!(until, Some(expected_until));

        // Regression guard: the previous implementation interpreted the day as
        // UTC. Any machine running in a non-UTC zone would then see boundaries
        // shifted by the offset. Confirm the local boundary differs from the
        // UTC one whenever the local offset is non-zero, so this test actually
        // exercises the fix on offset machines (and stays correct on UTC ones).
        let utc_since = chrono::NaiveDate::from_ymd_opt(2026, 3, 8)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        let local_offset_secs = Local
            .offset_from_utc_datetime(
                &chrono::NaiveDate::from_ymd_opt(2026, 3, 8)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
            )
            .local_minus_utc();
        if local_offset_secs != 0 {
            assert_ne!(
                since,
                Some(utc_since),
                "local-time bucketing must differ from UTC on offset machines"
            );
        } else {
            assert_eq!(since, Some(utc_since));
        }
    }

    #[test]
    fn start_of_day_preserves_boundary_across_dst_gap() {
        use chrono::{Local, LocalResult, TimeZone};

        // Simulate a zone that springs forward at local midnight (like
        // `America/Nuuk` on 2024-03-31, where 00:00–00:59 do not exist). The
        // resolver maps any wall-clock time before 01:00 to `None` (the gap) and
        // resolves 01:00+ as a real instant. The first valid instant after the
        // gap must be returned instead of dropping the boundary.
        let date = chrono::NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();
        let resolve = |wall: &chrono::NaiveDateTime| -> LocalResult<chrono::DateTime<Local>> {
            if wall.time() < chrono::NaiveTime::from_hms_opt(1, 0, 0).unwrap() {
                LocalResult::None
            } else {
                // Resolve against the machine's local zone for an arbitrary but
                // representable instant; the value just needs to be `Single`.
                Local.from_local_datetime(wall)
            }
        };

        let result = start_of_day_millis_with(date, resolve);
        let expected = Local
            .from_local_datetime(&date.and_hms_opt(1, 0, 0).unwrap())
            .single()
            .map(|dt| dt.timestamp_millis());

        // Boundary must be preserved (not `None`) and equal to the first valid
        // post-gap instant (01:00 local).
        assert!(
            result.is_some(),
            "DST-gap midnight must not drop the date boundary (would make filtering unbounded)"
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn start_of_day_uses_midnight_when_representable() {
        use chrono::{Local, TimeZone};

        // Sanity: when midnight exists, it is used unchanged (no forward walk).
        let date = chrono::NaiveDate::from_ymd_opt(2026, 6, 22).unwrap();
        let result = start_of_day_millis_with(date, |wall| Local.from_local_datetime(wall));
        let expected = Local
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .map(|dt| dt.timestamp_millis());
        assert_eq!(result, expected);
    }

    #[test]
    fn compute_msg_cost_without_pricing_is_zero() {
        let msg = parsed_message("claude-haiku-4");
        assert_eq!(compute_msg_cost(&msg, None), 0.0);
    }

    fn titled_entry(session_id: &str, title: &str) -> WikiEntry {
        WikiEntry {
            session_id: session_id.to_string(),
            client: "apple-fm".to_string(),
            workspace: None,
            workspace_label: None,
            created_at: 0,
            last_active: 0,
            title: Some(title.to_string()),
            task_category: None,
            description: None,
            complexity: None,
            task_group: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read: 0,
            total_cost: 0.0,
            models_used: Vec::new(),
            message_count: 0,
            duration_minutes: 0,
            summarized_at: None,
            fm_version: None,
        }
    }

    #[test]
    fn significant_tokens_normalizes_and_strips_stopwords() {
        // Lowercase, punctuation/ellipsis stripped, generic verbs dropped,
        // result sorted + deduped.
        assert_eq!(
            significant_tokens("Add JWT auth middleware…"),
            vec!["auth", "jwt", "middleware"]
        );
        assert_eq!(
            significant_tokens("Enhance API Security"),
            vec!["api", "security"]
        );
        // Trailing "..." and mixed case collapse to the same key.
        assert_eq!(
            significant_tokens("Fix the API Security..."),
            vec!["api", "security"]
        );
    }

    #[test]
    fn cluster_titles_merges_near_duplicates() {
        let entries = [
            titled_entry("a", "Enhance API Security"),
            titled_entry("b", "Enhance API security with JWT auth middleware"),
            titled_entry("c", "Add JWT auth middleware"),
        ];
        let refs: Vec<&WikiEntry> = entries.iter().collect();
        let assignments = cluster_titles(&refs);

        let label_of = |sid: &str| {
            assignments
                .iter()
                .find(|(s, _)| s == sid)
                .map(|(_, l)| l.clone())
                .unwrap()
        };

        // a & b share "api" + "security" → same cluster.
        assert_eq!(label_of("a"), label_of("b"));
        // b & c share "auth" + "jwt" + "middleware" → all three collapse via b.
        assert_eq!(label_of("b"), label_of("c"));

        let distinct: std::collections::HashSet<_> =
            assignments.iter().map(|(_, l)| l.clone()).collect();
        assert_eq!(distinct.len(), 1, "all three should merge into one task");
    }

    #[test]
    fn cluster_titles_keeps_unrelated_apart() {
        let entries = [
            titled_entry("a", "Add JWT auth middleware"),
            titled_entry("b", "Update database migration scripts"),
            titled_entry("c", "Refactor pricing service cache"),
        ];
        let refs: Vec<&WikiEntry> = entries.iter().collect();
        let assignments = cluster_titles(&refs);

        let distinct: std::collections::HashSet<_> =
            assignments.iter().map(|(_, l)| l.clone()).collect();
        assert_eq!(
            distinct.len(),
            3,
            "unrelated titles must stay in separate groups"
        );
    }

    #[test]
    fn cluster_label_prefers_most_frequent_then_shortest() {
        // Two identical long titles + one shorter variant: frequency wins, so the
        // repeated long title is the label even though a shorter one exists.
        let entries = [
            titled_entry("a", "Add JWT auth middleware"),
            titled_entry("b", "Add JWT auth middleware"),
            titled_entry("c", "JWT auth"),
        ];
        let refs: Vec<&WikiEntry> = entries.iter().collect();
        let assignments = cluster_titles(&refs);
        let label = &assignments[0].1;
        assert_eq!(label, "Add JWT auth middleware");
    }

    #[test]
    fn tokens_overlap_uses_ratio_not_absolute_count() {
        // Two long, unrelated titles that incidentally share TWO tokens
        // ("add" is a stopword, so the shared pair here is "service" + "api").
        // Under the old `shared >= 2` rule these merged; the overlap coefficient
        // (2 / 5 = 0.4 < 0.6) correctly keeps them apart.
        let a = significant_tokens("Add pricing service api cache layer");
        let b = significant_tokens("Add billing service api webhook handler");
        let shared: Vec<_> = a.iter().filter(|t| b.contains(t)).collect();
        assert_eq!(
            shared.len(),
            2,
            "fixture must share exactly two tokens to exercise the old rule"
        );
        assert!(
            !tokens_overlap(&a, &b),
            "two shared tokens out of five must NOT merge under the ratio rule"
        );

        // A short title fully contained in a long one still merges (coeff 1.0).
        let short = significant_tokens("pricing service");
        assert!(tokens_overlap(&short, &a));
    }

    #[test]
    fn tokens_overlap_singleton_does_not_overcluster() {
        // A title reducing to a SINGLE significant token must not merge with
        // every unrelated longer title that happens to contain that token —
        // the overlap coefficient alone would score 1.0 here.
        let single = significant_tokens("Fix API"); // -> ["api"]
        assert_eq!(single, vec!["api".to_string()]);
        let auth = significant_tokens("Add API auth"); // -> ["api", "auth"]
        let billing = significant_tokens("Update API billing"); // -> ["api", "billing"]
        assert!(
            !tokens_overlap(&single, &auth),
            "single shared token must not merge a singleton title"
        );
        assert!(
            !tokens_overlap(&single, &billing),
            "single shared token must not merge a singleton title"
        );
        // Two unrelated singletons that share their one token must also stay apart.
        let other_single = significant_tokens("API"); // -> ["api"]
        assert!(!tokens_overlap(&single, &other_single));
        // Genuinely related multi-token titles still cluster.
        let related_long = significant_tokens("Add API security with JWT auth middleware");
        let related_short = significant_tokens("Enhance API auth"); // -> ["api", "auth"]
        assert!(
            tokens_overlap(&related_short, &related_long),
            "two shared tokens out of two must still merge"
        );
    }

    #[test]
    fn cluster_titles_does_not_overcluster_singletons() {
        // "Fix API" (singleton "api") must NOT swallow unrelated API titles.
        let entries = [
            titled_entry("a", "Fix API"),
            titled_entry("b", "Add API auth"),
            titled_entry("c", "Update API billing"),
        ];
        let refs: Vec<&WikiEntry> = entries.iter().collect();
        let assignments = cluster_titles(&refs);
        let distinct: std::collections::HashSet<_> =
            assignments.iter().map(|(_, l)| l.clone()).collect();
        assert_eq!(
            distinct.len(),
            3,
            "a singleton-token title must not cluster with unrelated longer titles"
        );
    }

    #[test]
    fn significant_tokens_normalizes_nfc_nfd_equivalents() {
        // Precomposed "café" (NFC, U+00E9) and decomposed "café" (NFD,
        // "e" + U+0301 combining acute) are canonically equivalent and must
        // tokenize identically once normalized to NFC before stripping marks.
        let nfc = "Caf\u{00e9} module"; // café
        let nfd = "Cafe\u{0301} module"; // cafe + combining acute
        assert_ne!(nfc, nfd, "fixture must use distinct byte sequences");
        assert_eq!(significant_tokens(nfc), significant_tokens(nfd));
        assert!(significant_tokens(nfc).contains(&"café".to_string()));
    }

    #[test]
    fn cluster_titles_merges_nfc_nfd_equivalents() {
        let entries = [
            titled_entry("a", "Refactor Caf\u{00e9} Strat\u{00e9}gie"), // NFC
            titled_entry("b", "Refactor Cafe\u{0301} Strate\u{0301}gie"), // NFD
        ];
        let refs: Vec<&WikiEntry> = entries.iter().collect();
        let assignments = cluster_titles(&refs);
        let distinct: std::collections::HashSet<_> =
            assignments.iter().map(|(_, l)| l.clone()).collect();
        assert_eq!(
            distinct.len(),
            1,
            "canonically-equivalent titles must merge regardless of NFC/NFD form"
        );
    }

    #[test]
    fn cluster_titles_does_not_transitively_absorb_unrelated() {
        // Chain of titles where each adjacent pair shares two incidental tokens
        // but the ends are unrelated. The old union-growing signature plus the
        // `shared >= 2` rule made all of these collapse into one blob. With the
        // ratio rule and member-based matching they stay apart.
        let entries = [
            titled_entry("a", "Add pricing service api cache"),
            titled_entry("b", "Add billing service api webhook"),
            titled_entry("c", "Add billing report export csv"),
        ];
        let refs: Vec<&WikiEntry> = entries.iter().collect();
        let assignments = cluster_titles(&refs);
        let distinct: std::collections::HashSet<_> =
            assignments.iter().map(|(_, l)| l.clone()).collect();
        assert_eq!(
            distinct.len(),
            3,
            "incidental two-token overlaps must not chain unrelated titles into one cluster"
        );
    }

    #[test]
    fn cluster_titles_separates_distinct_stopword_only_titles() {
        // Titles made entirely of stopwords/generic verbs reduce to no
        // significant tokens. The old code lumped ALL of them into one arbitrary
        // group; now each distinct normalized title is its own singleton while
        // identical ones still collapse.
        let entries = [
            titled_entry("a", "Fix and update"),
            titled_entry("b", "Refactor and improve"),
            titled_entry("c", "Fix and update"),
        ];
        // Sanity: these really are token-empty so we exercise the empty path.
        assert!(significant_tokens("Fix and update").is_empty());
        assert!(significant_tokens("Refactor and improve").is_empty());

        let refs: Vec<&WikiEntry> = entries.iter().collect();
        let assignments = cluster_titles(&refs);
        let label_of = |sid: &str| {
            assignments
                .iter()
                .find(|(s, _)| s == sid)
                .map(|(_, l)| l.clone())
                .unwrap()
        };
        // Identical stopword-only titles merge; distinct ones do not.
        assert_eq!(label_of("a"), label_of("c"));
        assert_ne!(label_of("a"), label_of("b"));
        let distinct: std::collections::HashSet<_> =
            assignments.iter().map(|(_, l)| l.clone()).collect();
        assert_eq!(distinct.len(), 2);
    }

    #[test]
    fn significant_tokens_folds_non_ascii_case() {
        // Full Unicode lowercasing: "Café" and "café" must produce the same
        // token. to_ascii_lowercase left the accented capital untouched, so the
        // two titles would have clustered apart.
        assert_eq!(
            significant_tokens("Café Münchën Stratégie"),
            significant_tokens("café münchën stratégie")
        );
        assert!(significant_tokens("Café").contains(&"café".to_string()));
        assert_eq!(
            significant_tokens("İstanbul API"),
            significant_tokens("i\u{307}stanbul api")
        );
        assert!(significant_tokens("İstanbul").contains(&"istanbul".to_string()));
    }

    #[test]
    fn cluster_titles_merges_non_ascii_case_variants() {
        let entries = [
            titled_entry("a", "Refactor Café Stratégie module"),
            titled_entry("b", "Refactor café stratégie module"),
        ];
        let refs: Vec<&WikiEntry> = entries.iter().collect();
        let assignments = cluster_titles(&refs);
        let distinct: std::collections::HashSet<_> =
            assignments.iter().map(|(_, l)| l.clone()).collect();
        assert_eq!(distinct.len(), 1, "case-only differences must merge");
    }

    #[test]
    fn cluster_titles_groups_exact_duplicates() {
        // The degenerate case from the report: many identical titles must
        // collapse into exactly one group.
        let entries: Vec<WikiEntry> = (0..51)
            .map(|i| titled_entry(&format!("s{i}"), "Add JWT auth middleware"))
            .collect();
        let refs: Vec<&WikiEntry> = entries.iter().collect();
        let assignments = cluster_titles(&refs);
        let distinct: std::collections::HashSet<_> =
            assignments.iter().map(|(_, l)| l.clone()).collect();
        assert_eq!(distinct.len(), 1);
        assert_eq!(assignments.len(), 51);
    }

    fn entry_for(session_id: &str, client: &str) -> WikiEntry {
        let mut e = titled_entry(session_id, "ignored");
        e.client = client.to_string();
        e.title = None;
        e
    }

    #[test]
    fn extract_content_for_session_reads_real_claude_first_message() {
        // A claudecode transcript on disk, keyed by file stem == session_id.
        let dir = tempfile::tempdir().unwrap();
        let session_id = "sess-claude-1";
        let path = dir.path().join(format!("{session_id}.jsonl"));
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Fix the login bug"}]}}
"#,
        )
        .unwrap();

        let mut by_client_session: HashMap<(String, String), Vec<PathBuf>> = HashMap::new();
        by_client_session.insert(("claude".to_string(), session_id.to_string()), vec![path]);
        let index = SessionPathIndex {
            by_client_session,
            opencode_dbs: Vec::new(),
        };

        let entry = entry_for(session_id, "claude");
        let content = extract_content_for_session(&entry, &index);

        // The dispatcher must have reached the real claudecode extractor and
        // surfaced the actual first user message — not metadata-only.
        assert_eq!(
            content.first_user_message.as_deref(),
            Some("Fix the login bug")
        );
        assert_eq!(content.client, "claude");
    }

    #[test]
    fn extract_content_for_session_unknown_client_falls_back_to_metadata_only() {
        // An unsupported client has no dedicated extractor: must degrade to
        // metadata-only (None) without error or panic, even if a stray file
        // happens to share the session id.
        let dir = tempfile::tempdir().unwrap();
        let session_id = "sess-unknown-1";
        let path = dir.path().join(format!("{session_id}.jsonl"));
        std::fs::write(&path, "garbage\n").unwrap();

        let mut by_client_session: HashMap<(String, String), Vec<PathBuf>> = HashMap::new();
        by_client_session.insert(
            (
                "some-unsupported-client".to_string(),
                session_id.to_string(),
            ),
            vec![path],
        );
        let index = SessionPathIndex {
            by_client_session,
            opencode_dbs: Vec::new(),
        };

        let entry = entry_for(session_id, "some-unsupported-client");
        let content = extract_content_for_session(&entry, &index);

        assert!(content.first_user_message.is_none());
        assert_eq!(content.client, "some-unsupported-client");
    }

    #[test]
    fn extract_content_for_session_missing_file_falls_back_to_metadata_only() {
        // Supported client but no candidate file on disk: never panic, return
        // metadata-only.
        let index = SessionPathIndex::default();
        let entry = entry_for("does-not-exist", "claude");
        let content = extract_content_for_session(&entry, &index);
        assert!(content.first_user_message.is_none());
        assert_eq!(content.client, "claude");
    }

    #[test]
    fn session_path_index_isolates_clients_with_same_session_id() {
        // Two different clients share a session_id. Keying by (client, id) must
        // route each lookup to that client's own file, never the other's.
        let dir = tempfile::tempdir().unwrap();
        let claude_path = dir.path().join("claude.jsonl");
        let codex_path = dir.path().join("codex.jsonl");
        std::fs::write(&claude_path, "claude-bytes").unwrap();
        std::fs::write(&codex_path, "codex-bytes").unwrap();

        let mut by_client_session: HashMap<(String, String), Vec<PathBuf>> = HashMap::new();
        by_client_session.insert(
            ("claude".to_string(), "shared".to_string()),
            vec![claude_path.clone()],
        );
        by_client_session.insert(
            ("codex".to_string(), "shared".to_string()),
            vec![codex_path.clone()],
        );
        let index = SessionPathIndex {
            by_client_session,
            opencode_dbs: Vec::new(),
        };

        assert_eq!(index.candidates_for("claude", "shared"), vec![claude_path]);
        assert_eq!(index.candidates_for("codex", "shared"), vec![codex_path]);
        // Lookup for a client without a matching key returns nothing.
        assert!(index.candidates_for("gemini", "shared").is_empty());
    }

    #[test]
    fn build_session_path_index_keys_gemini_by_inner_session_id() {
        // A Gemini chat recording's wiki session_id is the in-file `sessionId`,
        // not the filename stem. The index must key by that inner id so the
        // summarizer lookup resolves and surfaces the real first prompt.
        let home = tempfile::tempdir().unwrap();
        let chats = home
            .path()
            .join(".gemini")
            .join("tmp")
            .join("projhash")
            .join("chats");
        std::fs::create_dir_all(&chats).unwrap();
        let inner_id = "b8d9ab56-e7da-4dca-abc1-eb61158bed4f";
        let file = chats.join("session-2026-06-08T19-53-b8d9ab56.json");
        std::fs::write(
            &file,
            format!(
                r#"{{"sessionId":"{inner_id}","messages":[{{"type":"user","content":"Hello Gemini"}}]}}"#
            ),
        )
        .unwrap();

        let opts = ReportOptions {
            json: false,
            since: None,
            until: None,
            workspace: None,
            client: None,
            no_summarize: false,
            summarizer: String::new(),
            rebuild: false,
            home_dir: Some(home.path().to_string_lossy().into_owned()),
            scanner_settings: Default::default(),
            today: false,
            week: false,
            month: false,
            full: false,
        };
        let index = build_session_path_index(&opts);

        // Lookup by the wiki session_id (inner id) must find the file.
        let candidates = index.candidates_for("gemini", inner_id);
        assert!(
            candidates.iter().any(|p| p == &file),
            "expected gemini index keyed by inner sessionId, candidates={candidates:?}"
        );

        let entry = entry_for(inner_id, "gemini");
        let content = extract_content_for_session(&entry, &index);
        assert_eq!(content.first_user_message.as_deref(), Some("Hello Gemini"));
    }
}
