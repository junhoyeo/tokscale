use anyhow::Result;
use chrono::{TimeZone, Utc};
use colored::Colorize;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tokscale_core::content_extractor::metadata_only_content;
use tokscale_core::content_extractor::SessionContent;
use tokscale_core::wiki::{WikiDb, WikiEntry};
use tokscale_core::{parse_local_clients, LocalParseOptions, ParsedMessage};

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
}

pub fn run_report(opts: ReportOptions) -> Result<()> {
    let wiki_path = WikiDb::default_path();
    let db = WikiDb::open(&wiki_path)
        .map_err(|e| anyhow::anyhow!("Failed to open wiki DB: {}", e))?;

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
        run_summarizer(&db, &unsummarized, &opts.summarizer)?;
    }

    let entries = db
        .query_entries(
            since_ts,
            until_ts,
            opts.workspace.as_deref(),
            opts.client.as_deref(),
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let needs_grouping = entries.iter().any(|e| e.title.is_some() && e.task_group.is_none());
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
            print_report_table(&entries, &db, is_multi_day)?;
        }
    } else {
        if opts.json {
            let json = serde_json::to_string_pretty(&entries)?;
            println!("{}", json);
        } else {
            let is_multi_day = opts.week || opts.month || (opts.since.is_some() && !opts.today);
            print_report_table(&entries, &db, is_multi_day)?;
        }
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
        agg.total_input += msg.input;
        agg.total_output += msg.output;
        agg.total_cache_read += msg.cache_read;
        agg.total_cost += compute_msg_cost(msg);
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

fn run_summarizer(db: &WikiDb, session_ids: &[String], backend: &str) -> Result<()> {
    let mut payloads: Vec<serde_json::Value> = Vec::new();
    for sid in session_ids {
        if let Ok(Some(entry)) = db.get_entry(sid) {
            let content = extract_content_for_session(&entry);
            payloads.push(serde_json::json!({
                "session_id": entry.session_id,
                "client": entry.client,
                "workspace": entry.workspace.unwrap_or_default(),
                "first_user_message": content.first_user_message,
                "models_used": entry.models_used,
                "total_tokens": entry.total_input_tokens + entry.total_output_tokens,
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

    let batch_size = match backend {
        "apple-fm" => payloads.len(),
        _ => 20,
    };

    let mut total_summarized = 0;
    for (batch_idx, chunk) in payloads.chunks(batch_size).enumerate() {
        if batch_size < payloads.len() {
            eprint!(
                "\r  Batch {}/{} ({} done)...",
                batch_idx + 1,
                (payloads.len() + batch_size - 1) / batch_size,
                total_summarized
            );
        }

        let results = match backend {
            "apple-fm" => run_apple_fm_summarizer(chunk)?,
            "claude" | "codex" | "gemini" | "kiro" => run_cli_summarizer(backend, chunk)?,
            other => {
                eprintln!("  {} Unknown summarizer: {}", "⚠".yellow(), other);
                return Ok(());
            }
        };

        for result in &results {
            let session_id = result["session_id"].as_str().unwrap_or_default();
            let title = result["title"].as_str().unwrap_or("Untitled");
            let category = result["task_category"].as_str().unwrap_or("other");
            let description = result["description"].as_str().unwrap_or("");
            let complexity = result["complexity"].as_str().unwrap_or("moderate");
            let fm_version = result["fm_version"].as_str();

            let _ = db.update_summary(session_id, title, category, description, complexity, fm_version);
        }

        total_summarized += results.len();
    }

    eprintln!(
        "\n  {} {} sessions summarized",
        "✓".green(),
        total_summarized
    );

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

    eprint!("  Grouping {} sessions into tasks...", summarized.len().to_string().cyan());

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

    let output = match backend {
        "claude" => Command::new("claude")
            .args(["-p", "--output-format", "text"])
            .arg(format!("System: {}\n\n{}", GROUPING_SYSTEM_PROMPT, prompt))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?,
        "codex" => Command::new("codex")
            .args(["--quiet", "--approval-mode", "never"])
            .arg(format!("{}\n\n{}", GROUPING_SYSTEM_PROMPT, prompt))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?,
        "gemini" => Command::new("gemini")
            .args(["-p"])
            .arg(format!("{}\n\n{}", GROUPING_SYSTEM_PROMPT, prompt))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?,
        "kiro" => Command::new("kiro")
            .args(["--non-interactive", "--prompt"])
            .arg(format!("{}\n\n{}", GROUPING_SYSTEM_PROMPT, prompt))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?,
        _ => {
            eprintln!(" skipped (unsupported backend)");
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
                    let _ = db.update_task_group(session_id, task_group);
                }
            }
            eprintln!(" {}", "✓".green());
        }
        Err(e) => {
            eprintln!("\n  {} Failed to parse grouping response: {}", "⚠".yellow(), e);
        }
    }

    Ok(())
}

fn run_apple_fm_summarizer(payloads: &[serde_json::Value]) -> Result<Vec<serde_json::Value>> {
    let script_path = find_summarizer_script()?;
    let input_json = serde_json::to_string(payloads)?;

    let mut child = Command::new("python3")
        .arg(&script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input_json.as_bytes())?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("  {} Apple FM summarizer: {}", "⚠".yellow(), stderr.trim());
        return Ok(Vec::new());
    }

    let results: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)?;
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

fn run_cli_summarizer(backend: &str, payloads: &[serde_json::Value]) -> Result<Vec<serde_json::Value>> {
    let prompt = build_cli_prompt(payloads);

    let output = match backend {
        "claude" => Command::new("claude")
            .args(["-p", "--output-format", "text"])
            .arg(format!("System: {}\n\n{}", SUMMARIZER_SYSTEM_PROMPT, prompt))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?,
        "codex" => Command::new("codex")
            .args(["--quiet", "--approval-mode", "never"])
            .arg(format!("{}\n\n{}", SUMMARIZER_SYSTEM_PROMPT, prompt))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?,
        "gemini" => Command::new("gemini")
            .args(["-p"])
            .arg(format!("{}\n\n{}", SUMMARIZER_SYSTEM_PROMPT, prompt))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?,
        "kiro" => Command::new("kiro")
            .args(["--non-interactive", "--prompt"])
            .arg(format!("{}\n\n{}", SUMMARIZER_SYSTEM_PROMPT, prompt))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?,
        _ => return Ok(Vec::new()),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("  {} {} summarizer failed: {}", "⚠".yellow(), backend, stderr.trim());
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_str = extract_json_array(&stdout);

    match serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
        Ok(results) => Ok(results),
        Err(e) => {
            eprintln!("  {} Failed to parse {} response: {}", "⚠".yellow(), backend, e);
            Ok(Vec::new())
        }
    }
}

fn extract_json_array(text: &str) -> &str {
    if let Some(start) = text.find('[') {
        if let Some(end) = text.rfind(']') {
            return &text[start..=end];
        }
    }
    text
}

fn print_report_table(entries: &[WikiEntry], _db: &WikiDb, is_multi_day: bool) -> Result<()> {
    if entries.is_empty() {
        println!("No sessions found for the given filters.");
        return Ok(());
    }

    let total_cost: f64 = entries.iter().map(|e| e.total_cost).sum();
    let total_tokens: i64 = entries
        .iter()
        .map(|e| e.total_input_tokens + e.total_output_tokens)
        .sum();
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
        for model in &entry.models_used {
            let agg = by_model.entry(model.as_str()).or_insert((0.0, 0, 0));
            agg.0 += entry.total_cost / entry.models_used.len() as f64;
            agg.1 += (entry.total_input_tokens + entry.total_output_tokens)
                / entry.models_used.len() as i64;
            agg.2 += 1;
        }
    }

    let mut models: Vec<_> = by_model.iter().collect();
    models.sort_by(|a, b| b.1 .0.partial_cmp(&a.1 .0).unwrap());

    println!("  {:<30} {:>8} {:>12} {:>8}", "Model", "Sessions", "Tokens", "Cost");
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
        let group = entry.task_group.as_deref().unwrap_or(
            entry.title.as_deref().unwrap_or("(unsummarized)")
        );
        let title = entry.title.as_deref().unwrap_or("(unsummarized)");
        let agg = by_group.entry(group).or_insert((0.0, 0, 0, Vec::new()));
        agg.0 += entry.total_cost;
        agg.1 += entry.total_input_tokens + entry.total_output_tokens;
        agg.2 += 1;
        if !agg.3.contains(&title) {
            agg.3.push(title);
        }
    }

    let mut groups: Vec<_> = by_group.iter().collect();
    groups.sort_by(|a, b| b.1 .0.partial_cmp(&a.1 .0).unwrap());

    println!("  {:<40} {:>5} {:>10} {:>8}", "Task Group", "Sess", "Tokens", "Cost");
    println!("  {}", "─".repeat(67));
    for (group, (cost, tokens, count, titles)) in groups.iter().take(15) {
        let display_group: String = if group.len() > 40 {
            format!("{}…", &group[..39])
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
                let display_t: &str = if t.len() > 38 { &t[..38] } else { t };
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
        print_daily_breakdown(entries);
    } else {
        print_session_list(entries);
    }

    Ok(())
}

fn print_daily_breakdown(entries: &[WikiEntry]) {
    use std::collections::BTreeMap;

    let mut by_date: BTreeMap<String, (f64, i64, usize, Vec<&WikiEntry>)> = BTreeMap::new();
    for entry in entries {
        let date_key = Utc
            .timestamp_opt(entry.created_at / 1000, 0)
            .single()
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let agg = by_date
            .entry(date_key)
            .or_insert((0.0, 0, 0, Vec::new()));
        agg.0 += entry.total_cost;
        agg.1 += entry.total_input_tokens + entry.total_output_tokens;
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
        for s in sessions.iter().take(5) {
            let title = s.title.as_deref().unwrap_or("(pending)");
            let model = s.models_used.first().map(|m| m.as_str()).unwrap_or("-");
            let display_title: &str = if title.len() > 40 { &title[..40] } else { title };
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

fn print_session_list(entries: &[WikiEntry]) {
    let recent: Vec<&WikiEntry> = entries.iter().take(10).collect();
    if !recent.is_empty() {
        println!("  Sessions:");
        println!("  {}", "─".repeat(80));
        for entry in recent {
            let date = Utc
                .timestamp_opt(entry.created_at / 1000, 0)
                .single()
                .map(|dt| dt.format("%H:%M").to_string())
                .unwrap_or_else(|| "??:??".to_string());

            let title = entry
                .title
                .as_deref()
                .unwrap_or("(pending summarization)");
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
        if entries.len() > 10 {
            println!("    … +{} more sessions", entries.len() - 10);
        }
        println!();
    }
}

fn extract_content_for_session(entry: &WikiEntry) -> SessionContent {
    metadata_only_content(&entry.session_id, &entry.client)
}

fn find_summarizer_script() -> Result<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    let candidates = [
        exe_dir
            .as_ref()
            .map(|d| d.join("../scripts/wiki-summarizer.py")),
        exe_dir
            .as_ref()
            .map(|d| d.join("../../scripts/wiki-summarizer.py")),
        Some(PathBuf::from("scripts/wiki-summarizer.py")),
        dirs::config_dir().map(|d| d.join("tokscale/wiki-summarizer.py")),
    ];

    for candidate in candidates.iter().flatten() {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }

    Err(anyhow::anyhow!(
        "wiki-summarizer.py not found. Expected at scripts/wiki-summarizer.py or ~/.config/tokscale/wiki-summarizer.py"
    ))
}

fn parse_date_range(since: &Option<String>, until: &Option<String>) -> (Option<i64>, Option<i64>) {
    let since_ts = since.as_ref().and_then(|s| {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis())
    });
    let until_ts = until.as_ref().and_then(|s| {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .map(|d| d.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp_millis())
    });
    (since_ts, until_ts)
}

fn compute_msg_cost(msg: &ParsedMessage) -> f64 {
    let input_cost = msg.input as f64 * 0.000003;
    let output_cost = msg.output as f64 * 0.000015;
    let cache_cost = msg.cache_read as f64 * 0.0000003;
    input_cost + output_cost + cache_cost
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
