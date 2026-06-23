#[derive(Debug, Clone)]
pub(crate) enum CodexLoginOutcome {
    Imported(crate::commands::usage::codex::CodexAccountInfo),
    Failed(String),
}
