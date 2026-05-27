use std::path::{Path, PathBuf};

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticPath {
    pub label: &'static str,
    pub path: String,
    pub exists: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDiagnostic {
    pub code: &'static str,
    pub severity: &'static str,
    pub message: &'static str,
    pub help: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<DiagnosticPath>,
}

const CLAUDE_DESKTOP_MESSAGE: &str =
    "Claude Desktop app data was detected, but Tokscale counts Claude Code JSONL transcripts only.";

const CLAUDE_DESKTOP_HELP: &str = "Claude Desktop chat storage and Claude data exports do not expose a documented per-message token ledger. Use `tokscale usage` for Claude subscription quota bars; organization/API billing requires Anthropic Admin Usage/Cost API outside local scanning.";

pub fn diagnostics_for_clients_row(home_dir: &Path) -> Vec<ClientDiagnostic> {
    claude_diagnostics(home_dir, true)
}

pub fn diagnostics_for_empty_explicit_report(
    home_dir: &Path,
    clients: &Option<Vec<String>>,
    claude_message_count: i32,
) -> Vec<ClientDiagnostic> {
    if !explicitly_requests_claude(clients) || claude_message_count > 0 {
        return Vec::new();
    }

    claude_diagnostics(home_dir, false)
}

fn explicitly_requests_claude(clients: &Option<Vec<String>>) -> bool {
    clients
        .as_ref()
        .is_some_and(|ids| ids.iter().any(|id| id == "claude"))
}

fn claude_diagnostics(home_dir: &Path, include_info: bool) -> Vec<ClientDiagnostic> {
    let mut diagnostics = Vec::new();

    let desktop_paths: Vec<PathBuf> = claude_desktop_storage_paths(home_dir)
        .into_iter()
        .filter(|path| path.exists())
        .collect();

    if !desktop_paths.is_empty() {
        diagnostics.push(ClientDiagnostic {
            code: "claude_desktop_not_scanned",
            severity: "warning",
            message: CLAUDE_DESKTOP_MESSAGE,
            help: CLAUDE_DESKTOP_HELP,
            paths: diagnostic_paths(home_dir, desktop_paths),
        });
    }

    let stats_cache = home_dir.join(".claude").join("stats-cache.json");
    if include_info && stats_cache.exists() {
        diagnostics.push(ClientDiagnostic {
            code: "claude_stats_cache_not_imported",
            severity: "info",
            message: "Claude Code stats-cache.json was detected, but Tokscale does not import aggregate cache totals as session usage.",
            help: "stats-cache.json contains aggregate /usage data without stable per-message/session attribution, so importing it would risk double counting or fabricated model/session totals.",
            paths: vec![DiagnosticPath {
                label: "statsCache",
                path: stats_cache.to_string_lossy().to_string(),
                exists: true,
            }],
        });
    }

    diagnostics
}

fn claude_desktop_storage_paths(home_dir: &Path) -> Vec<PathBuf> {
    vec![
        home_dir
            .join("Library")
            .join("Application Support")
            .join("Claude"),
        home_dir.join("AppData").join("Roaming").join("Claude"),
        home_dir.join(".config").join("Claude"),
    ]
}

fn diagnostic_paths(home_dir: &Path, desktop_paths: Vec<PathBuf>) -> Vec<DiagnosticPath> {
    let mut paths: Vec<DiagnosticPath> = desktop_paths
        .into_iter()
        .map(|path| DiagnosticPath {
            label: "desktopStorage",
            path: path.to_string_lossy().to_string(),
            exists: true,
        })
        .collect();

    for (label, path) in [
        (
            "claudeCodeProjects",
            home_dir.join(".claude").join("projects"),
        ),
        (
            "claudeCodeTranscripts",
            home_dir.join(".claude").join("transcripts"),
        ),
    ] {
        let exists = path.exists();
        paths.push(DiagnosticPath {
            label,
            path: path.to_string_lossy().to_string(),
            exists,
        });
    }

    paths
}
