//! Session parsers for different AI coding assistant formats
//!
//! Each client has its own parser that converts to a unified message format.

pub mod amp;
pub mod antigravity;
pub mod antigravity_cli;
pub mod augment;
pub mod cherrystudio;
pub mod claudecode;
pub mod cline;
pub mod codebuddy;
pub mod codebuff;
pub mod codex;
pub mod commandcode;
pub mod copilot;
pub mod copilot_desktop;
pub mod copilot_vscode;
pub mod crush;
pub mod cursor;
pub mod devin;
pub mod droid;
pub mod dsh;
pub mod freebuff;
pub mod fx;
pub mod gemini;
pub mod gjc;
pub mod goose;
pub mod grok;
pub mod hermes;
pub mod jcode;
pub mod junie;
pub mod kilo;
pub mod kilocode;
pub mod kimchi;
pub mod kimi;
pub mod kiro;
pub mod mcode;
pub mod micode;
pub mod mux;
pub mod openclaw;
pub mod opencode;
pub mod opencode_schema;
pub mod opencodereview;
pub mod pi;
pub mod prime_agent;
pub mod qwen;
pub mod reasonix;
pub mod roocode;
pub mod senpi;
pub mod synthetic;
pub(crate) mod tencent_buddy;
pub mod trae;
pub(crate) mod utils;
pub mod warp;
pub mod workbuddy;
pub mod zcode;
pub mod zed;

use std::io::Read;
use std::path::{Path, PathBuf, MAIN_SEPARATOR_STR};

use crate::TokenBreakdown;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CostSource {
    #[default]
    Unknown,
    ProviderReported,
    Estimated,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UnifiedMessage {
    pub client: String,
    pub model_id: String,
    pub provider_id: String,
    pub session_id: String,
    pub workspace_key: Option<String>,
    pub workspace_label: Option<String>,
    pub timestamp: i64,
    pub date: String,
    pub tokens: TokenBreakdown,
    pub cost: f64,
    #[serde(default)]
    pub cost_source: CostSource,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default = "default_message_count")]
    pub message_count: i32,
    pub agent: Option<String>,
    pub dedup_key: Option<String>,
    /// Human-readable session title/name when the source client stores one
    /// (e.g. OpenCode's `session.title` column). `None` for clients that
    /// don't record a title; the Sessions tab falls back to showing just
    /// the session ID in that case.
    #[serde(default)]
    pub session_title: Option<String>,
    /// True if this message is the first assistant response after a user turn.
    /// Used to count user interaction turns (as opposed to API message count).
    #[serde(default)]
    pub is_turn_start: bool,
    /// True when the parser observed conflicting authoritative model evidence.
    /// Such rows must remain unpriced rather than accepting fallback attribution.
    #[serde(default)]
    pub model_attribution_conflicted: bool,
}

const fn default_message_count() -> i32 {
    1
}

pub fn normalize_agent_name(agent: &str) -> String {
    let cleaned = strip_zero_width_chars(agent);
    let trimmed = cleaned.trim();
    let stripped = strip_agent_prefix(trimmed);
    let canonical = canonicalize_agent_name(stripped);
    let agent_lower = canonical.to_lowercase();

    if agent_lower.contains("plan") {
        if agent_lower.contains("omo") || agent_lower.contains("sisyphus") {
            return "Planner-Sisyphus".to_string();
        }
        return titlecase_agent(&canonical);
    }

    if agent_lower == "omo" || agent_lower == "sisyphus" {
        return "Sisyphus".to_string();
    }

    if agent_lower == "orchestrator-sisyphus" {
        return "Atlas".to_string();
    }

    titlecase_agent(&canonical)
}

pub fn normalize_opencode_agent_name(agent: &str) -> String {
    let cleaned = strip_zero_width_chars(agent);
    let trimmed = cleaned.trim();
    let stripped = strip_agent_prefix(trimmed);
    let canonical = canonicalize_agent_name(stripped);
    let agent_lower = canonical.to_lowercase();

    if let Some(normalized) = normalize_oh_my_opencode_agent_name(&agent_lower) {
        return normalized;
    }

    normalize_agent_name(&canonical)
}

pub fn normalize_copilot_agent_name(agent: &str) -> String {
    // Hardcoded brand name for the default native agent
    if agent.eq_ignore_ascii_case("github.copilot.default") {
        return "GitHub Copilot".to_string();
    }

    // Native github.copilot.* agents: strip prefix, titlecase remainder
    const GITHUB_COPILOT_PREFIX: &str = "github.copilot.";
    if agent
        .get(..GITHUB_COPILOT_PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(GITHUB_COPILOT_PREFIX))
    {
        let remainder = &agent[GITHUB_COPILOT_PREFIX.len()..];
        let hyphenated = remainder.replace('.', "-");
        return titlecase_agent(&hyphenated);
    }

    // Plugin:team:slug format — titlecase each colon-separated part, join with ": "
    const PLUGIN_PREFIX: &str = "Plugin:";
    if agent
        .get(..PLUGIN_PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(PLUGIN_PREFIX))
    {
        let rest = &agent[PLUGIN_PREFIX.len()..];
        let parts: Vec<&str> = rest.splitn(2, ':').collect();
        if parts.len() == 2 {
            let team = titlecase_agent(parts[0]);
            let slug = titlecase_agent(parts[1]);
            return format!("{}: {}", team, slug);
        }
        return titlecase_agent(rest);
    }

    normalize_agent_name(agent)
}

fn normalize_oh_my_opencode_agent_name(agent_lower: &str) -> Option<String> {
    let normalized = match agent_lower {
        // Parenthesized format and dash format
        "sisyphus (ultraworker)"
        | "sisyphus - ultraworker"
        | "sisyphus ultraworker"
        | "sisyphus" => "Sisyphus",
        "hephaestus (deep agent)"
        | "hephaestus - deep agent"
        | "hephaestus deep agent"
        | "hephaestus" => "Hephaestus",
        "prometheus (plan builder)"
        | "prometheus - plan builder"
        | "prometheus plan builder"
        | "prometheus (planner)"
        | "prometheus" => "Prometheus",
        "atlas (plan executor)" | "atlas - plan executor" | "atlas plan executor" | "atlas" => {
            "Atlas"
        }
        "metis (plan consultant)"
        | "metis - plan consultant"
        | "metis plan consultant"
        | "metis" => "Metis",
        "momus (plan critic)"
        | "momus - plan critic"
        | "momus plan critic"
        | "momus (plan reviewer)"
        | "momus" => "Momus",
        "orchestrator-sisyphus" => "Atlas",
        "sisyphus-junior" => "Sisyphus-Junior",
        "planner-sisyphus" => "Planner-Sisyphus",
        _ => return None,
    };

    Some(normalized.to_string())
}

/// Strip zero-width Unicode characters that oh-my-openagent uses as
/// invisible sort-order prefixes (U+200B ZERO WIDTH SPACE, U+200C ZERO
/// WIDTH NON-JOINER, U+200D ZERO WIDTH JOINER, U+FEFF BOM/ZWNBSP).
fn strip_zero_width_chars(s: &str) -> String {
    if !s.contains(['\u{200B}', '\u{200C}', '\u{200D}', '\u{FEFF}']) {
        return s.to_string();
    }
    s.chars()
        .filter(|c| !matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}'))
        .collect()
}

fn strip_agent_prefix(name: &str) -> &str {
    for prefix in &["astrape:", "oh-my-claudecode:", "oh-my-codex:"] {
        if name
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        {
            return &name[prefix.len()..];
        }
    }
    name
}

fn canonicalize_agent_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn titlecase_word(word: &str) -> String {
    match word.to_lowercase().as_str() {
        "ui" => "UI".to_string(),
        "ux" => "UX".to_string(),
        "api" => "API".to_string(),
        _ => {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + &chars.collect::<String>()
                }
            }
        }
    }
}

fn titlecase_agent(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    name.split('-')
        .flat_map(|part| part.split_whitespace())
        .map(titlecase_word)
        .collect::<Vec<_>>()
        .join(" ")
}

impl UnifiedMessage {
    pub fn new(
        client: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
    ) -> Self {
        Self::new_full(
            client,
            model_id,
            provider_id,
            session_id,
            timestamp,
            tokens,
            cost,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_agent(
        client: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
        agent: Option<String>,
    ) -> Self {
        Self::new_full(
            client,
            model_id,
            provider_id,
            session_id,
            timestamp,
            tokens,
            cost,
            agent,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_dedup(
        client: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
        dedup_key: Option<String>,
    ) -> Self {
        Self::new_full(
            client,
            model_id,
            provider_id,
            session_id,
            timestamp,
            tokens,
            cost,
            None,
            dedup_key,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_full(
        client: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
        agent: Option<String>,
        dedup_key: Option<String>,
    ) -> Self {
        let date = timestamp_to_date(timestamp);
        Self {
            client: client.into(),
            model_id: model_id.into(),
            provider_id: provider_id.into(),
            session_id: session_id.into(),
            workspace_key: None,
            workspace_label: None,
            timestamp,
            date,
            tokens,
            cost,
            cost_source: CostSource::Unknown,
            duration_ms: None,
            message_count: default_message_count(),
            agent,
            dedup_key,
            session_title: None,
            is_turn_start: false,
            model_attribution_conflicted: false,
        }
    }

    pub fn set_workspace(
        &mut self,
        workspace_key: Option<String>,
        workspace_label: Option<String>,
    ) {
        self.workspace_key = workspace_key;
        self.workspace_label = workspace_label;
    }

    pub(crate) fn refresh_derived_fields(&mut self) {
        self.date = timestamp_to_date(self.timestamp);
    }

    /// Re-derive the day bucket under an explicitly chosen timezone.
    ///
    /// `UnifiedMessage::new` is a constructor called from 92 sites across 42
    /// parser files, so the zone cannot be threaded into it without touching
    /// every one. It does not need to be: `date` is a derived field, already
    /// recomputed from `timestamp` after construction. This lets the one
    /// post-parse pass that holds the user's settings re-key every message at
    /// once, which is the only place the pinned zone is actually known.
    pub(crate) fn rebucket_date(&mut self, timezone: &crate::bucket_tz::BucketTimezone) {
        // A non-positive timestamp is the parsers' "no usable time" sentinel,
        // not an instant before 1970. Re-keying it would move garbage between
        // two equally wrong days, and it is also what bounds the window the
        // auto-pin agreement check has to cover: leaving these alone is what
        // makes `AGREEMENT_WINDOW_START_MS` a real lower bound rather than a
        // convenient one.
        if self.timestamp <= 0 {
            return;
        }

        let key = timezone.day_key(self.timestamp);
        // An unrepresentable instant yields an empty key. Keeping the previous
        // date is wrong by at most the offset between two zones; replacing it
        // with `""` would collapse the message into a bucket that is not a day
        // at all, and that bucket would then be submitted.
        if !key.is_empty() {
            self.date = key;
        }
    }

    pub(crate) fn set_timestamp(&mut self, timestamp: i64) {
        self.timestamp = timestamp;
        self.refresh_derived_fields();
    }

    pub fn mark_provider_reported_cost(&mut self) {
        self.cost_source = CostSource::ProviderReported;
    }

    pub(crate) fn mark_estimated_cost(&mut self) {
        self.cost_source = CostSource::Estimated;
    }

    pub(crate) fn has_authoritative_cost(&self) -> bool {
        self.cost_source == CostSource::ProviderReported
    }
}

pub fn normalize_workspace_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let preserve_unc_prefix = trimmed.starts_with("\\\\") || trimmed.starts_with("//");
    let mut normalized = trimmed.replace('\\', "/");

    if preserve_unc_prefix {
        let body = normalized.trim_start_matches('/');
        let mut collapsed = body.to_string();
        while collapsed.contains("//") {
            collapsed = collapsed.replace("//", "/");
        }
        normalized = format!("//{}", collapsed);
    } else {
        while normalized.contains("//") {
            normalized = normalized.replace("//", "/");
        }
    }

    // A root is not a trailing separator to strip. `/` survives on length alone
    // and `//share` on the UNC minimum, but `C:/` did not: stripping left `C:`,
    // which is Windows' DRIVE-RELATIVE form (`C:work` means "work under the
    // current directory of drive C"), so a real drive root classified as
    // relative and was refused a filesystem read it was entitled to.
    let minimum_len = if preserve_unc_prefix {
        2
    } else if windows_drive_root(&normalized, ':', '/').is_some() {
        3
    } else {
        1
    };
    if normalized.len() > minimum_len {
        normalized = normalized.trim_end_matches('/').to_string();
    }

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

/// Whether `key` names an absolute location rather than a relative fragment.
///
/// Both roots a workspace key can be anchored at: `/` (POSIX, and the `//` of a
/// UNC share) or a Windows drive. Normalized first so `C:\\work\\api` answers the
/// same as `C:/work/api`. Callers use this before touching the filesystem —
/// a relative key resolves against the process working directory, which is not
/// a place any workspace lives.
fn is_absolute_workspace_key(key: &str) -> bool {
    normalize_workspace_key(key)
        .is_some_and(|key| key.starts_with('/') || windows_drive_root(&key, ':', '/').is_some())
}

/// Split a Windows drive anchor off the front of `key`, returning the drive
/// letter and the remainder starting at `separator`.
///
/// Shared with the slug decoder, which sees the same anchor with both the colon
/// and the separator encoded as `-` (`C:\\Users\\me` becomes `C--Users-me`). One
/// test for what counts as a drive means a path and the slug built from it can
/// never disagree about whether they are absolute.
fn windows_drive_root(key: &str, colon: char, separator: char) -> Option<(char, &str)> {
    let drive = key.chars().next()?;
    if !drive.is_ascii_alphabetic() {
        return None;
    }
    let remaining = key[1..].strip_prefix(colon)?;
    remaining
        .starts_with(separator)
        .then_some((drive, remaining))
}

pub fn workspace_label_from_key(key: &str) -> Option<String> {
    key.rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
}

/// Marker between a repository and the worktree checked out inside it, e.g.
/// `claude-witness ⑃ lens-backfill-findings`. Worktrees are the common case for
/// agent CLIs that isolate each task, and a repo-only label would render a dozen
/// identical rows.
pub const WORKTREE_SEPARATOR: &str = " ⑃ ";

/// Path segments that mean "everything below me is a worktree, not the repo".
const WORKTREE_MARKERS: [&str; 2] = [".claude/worktrees/", ".git/worktrees/"];

/// The same markers as they appear in a dash-encoded Claude Code slug. A deleted
/// worktree cannot be resolved against the filesystem, but the marker survives
/// verbatim in the slug, so the repo prefix is still recoverable from the string.
const ENCODED_WORKTREE_MARKERS: [&str; 2] = ["--claude-worktrees-", "--git-worktrees-"];

/// Split a dash-encoded slug at its worktree marker into (repo slug, worktree
/// name). Lets rollup and labeling keep working for worktrees whose directories
/// have since been deleted — otherwise those rows keep the raw slug forever.
fn split_encoded_worktree(key: &str) -> Option<(String, String)> {
    let (index, marker_len) = first_encoded_worktree_marker(key)?;
    let repo = &key[..index];
    // Nested worktrees name the row after the INNERMOST one, the same way the
    // path form resolves `.../worktrees/outer/.claude/worktrees/inner`.
    let mut worktree = &key[index + marker_len..];
    while let Some((inner, inner_len)) = first_encoded_worktree_marker(worktree) {
        worktree = &worktree[inner + inner_len..];
    }
    Some((repo.to_string(), worktree.to_string()))
}

/// Earliest encoded worktree marker in `key`, as `(offset, marker length)`.
///
/// Smallest offset, not first marker in the array: a nested slug carries both
/// kinds, and the repository ends at whichever one appears first in the string.
/// Occurrences that would leave an empty repo or an empty worktree name are not
/// splits at all.
fn first_encoded_worktree_marker(key: &str) -> Option<(usize, usize)> {
    ENCODED_WORKTREE_MARKERS
        .iter()
        .filter_map(|marker| key.find(marker).map(|index| (index, marker.len())))
        .filter(|(index, marker_len)| *index > 0 && index + marker_len < key.len())
        .min()
}

/// The repository root a workspace key belongs to, with any worktree suffix
/// removed. Returns `None` when the key is not inside a worktree, so callers can
/// tell "already a repo root" from "rolled up to one".
///
/// Only path-shaped keys are handled: clients that store an opaque id (Warp's
/// workspace UUID) have nothing to roll up and are returned untouched.
///
/// Nested worktrees resolve to the outermost repository, because the first
/// marker in the path is the one the repo owns.
pub fn workspace_repo_root(key: &str) -> Option<String> {
    let key = normalize_workspace_key(key)?;
    // Smallest offset, not first marker in the array: a path can contain both
    // kinds (`/a/.git/worktrees/x/.claude/worktrees/y`), and iterating the array
    // would answer with whichever marker happens to be listed first rather than
    // with the outermost one. That named `/a/.git/worktrees/x` as the repo, so
    // `--merge-worktrees` gave one repository two rows.
    let index = WORKTREE_MARKERS
        .iter()
        .filter_map(|marker| find_segment_marker(&key, marker))
        .filter(|index| !key[..*index].trim_end_matches('/').is_empty())
        .min()?;
    Some(key[..index].trim_end_matches('/').to_string())
}

/// Locate `marker` where it starts a path segment.
///
/// Substring matching is wrong here: a plain directory named `my.git` makes
/// `/notes/my.git/worktrees/draft` contain `.git/worktrees/` even though nothing
/// in it is a repository, and stripping there would roll the row up under a
/// `/notes/my` that does not exist.
fn find_segment_marker(key: &str, marker: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(offset) = key[from..].find(marker) {
        let index = from + offset;
        if index == 0 || key.as_bytes()[index - 1] == b'/' {
            return Some(index);
        }
        from = index + 1;
    }
    None
}

/// The repo root of a worktree checked out *beside* its repository.
///
/// `git worktree add ../feature-x` leaves nothing in the worktree's own path to
/// key on — the only link is a `.git` FILE holding
/// `gitdir: /path/to/repo/.git/worktrees/feature-x`. A normal checkout has a
/// `.git` DIRECTORY there, so the read simply fails and this returns `None`;
/// a submodule points at `.git/modules/...`, which carries no worktree marker
/// and is likewise rejected.
pub fn workspace_git_worktree_root(path: &str) -> Option<String> {
    // Only an absolute key names a directory on its own. Claude Code's slug is
    // relative, so `join` resolved it against the process working directory and
    // read `$CWD/<slug>/.git` — a file the user never named, planted by whoever
    // owns the directory the binary was started in, yet trusted to rename the
    // row and, under `--merge-worktrees`, to re-key it.
    //
    // Both spellings have to be absolute, because the test and the read see
    // different strings: `is_absolute_workspace_key` normalizes separators, so a
    // Windows-shaped key passes it on any host, while the read below hands the
    // RAW key to `Path`. On POSIX `Path::new(r"C:\Users\me\repo")` is one
    // relative filename, so that key resolved against `$CWD` again — the same
    // hole the slug closed, entered through a different door.
    if !is_absolute_workspace_key(path) || !Path::new(path).is_absolute() {
        return None;
    }
    let contents = read_git_pointer_file(&Path::new(path).join(".git"))?;
    let pointer = contents
        .lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:"))?
        .trim();
    // Git may record the pointer relative to the worktree directory.
    let joined = if Path::new(pointer).is_absolute() || pointer.starts_with('/') {
        normalize_workspace_key(pointer)?
    } else {
        normalize_workspace_key(&Path::new(path).join(pointer).to_string_lossy())?
    };
    repo_root_from_gitdir(&lexically_normalize(&joined)?)
}

/// Largest `.git` pointer file worth reading.
///
/// Git writes a single short `gitdir: <path>` line. Anything past this is not a
/// pointer file, and reading it whole would let an unrelated directory dictate
/// how much memory a report allocates.
const GIT_POINTER_MAX_BYTES: u64 = 64 * 1024;

/// Read a `.git` pointer file, refusing anything that is not an ordinary file of
/// plausible size.
///
/// This runs once per distinct workspace key on every report and every TUI
/// refresh, against directories the user never named — the workspace key comes
/// from whatever a client recorded. A bare `read_to_string` there is a liability:
/// `open` on a FIFO blocks until someone writes to the other end, so a `.git`
/// FIFO wedges the whole report forever, and a 20MB regular file is read in full
/// on every refresh. `symlink_metadata` first so a `.git` symlink is resolved
/// deliberately rather than followed blind, and the resolved target is checked
/// again: only a regular file within the cap is ever opened.
fn read_git_pointer_file(path: &Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    let metadata = if metadata.is_symlink() {
        std::fs::metadata(path).ok()?
    } else {
        metadata
    };
    if !metadata.is_file() || metadata.len() > GIT_POINTER_MAX_BYTES {
        return None;
    }

    // Bounded read rather than `read_to_string`: `stat` and `open` are two
    // syscalls, and the file can grow between them.
    let mut contents = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(GIT_POINTER_MAX_BYTES)
        .read_to_string(&mut contents)
        .ok()?;
    Some(contents)
}

/// Resolve `.` and `..` segments without touching the filesystem.
///
/// A relative pointer joins into `/work/feature-x/../api/.git/worktrees/x`,
/// which names the right directory but is not the string the repository's own
/// row is keyed by. Rollup compares identities as strings, so leaving the `..`
/// in would keep the worktree and its repo in separate rows — the exact thing
/// reading the pointer is meant to fix. Lexical rather than `canonicalize` so a
/// symlinked path keeps the spelling its own row uses.
fn lexically_normalize(key: &str) -> Option<String> {
    let prefix_len = if key.starts_with("//") {
        2
    } else if key.starts_with('/') {
        1
    } else {
        0
    };
    let (prefix, rest) = key.split_at(prefix_len);
    let mut segments: Vec<&str> = Vec::new();
    for segment in rest.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                // Above an absolute root there is nothing to pop, and POSIX
                // defines `/..` as `/`; a relative key has to keep the segment.
                if segments.last().is_some_and(|last| *last != "..") {
                    segments.pop();
                } else if prefix.is_empty() {
                    segments.push("..");
                }
            }
            other => segments.push(other),
        }
    }
    normalize_workspace_key(&format!("{prefix}{}", segments.join("/")))
}

/// The repository a `gitdir:` pointer belongs to.
///
/// Git always writes `<git dir>/worktrees/<name>`, so the segment before
/// `worktrees` is the repository's git directory: `<repo>/.git` for an ordinary
/// checkout, and the repository itself (`/srv/repo.git`) when it is bare. Bare
/// layouts have to be handled here rather than by `workspace_repo_root`, which
/// deliberately refuses to read `.git/worktrees/` out of a `repo.git` segment —
/// from a bare path alone there is nothing to distinguish a repository from a
/// directory that merely ends in `.git`. Arriving through a `.git` pointer file
/// is what makes it certain.
fn repo_root_from_gitdir(gitdir: &str) -> Option<String> {
    let (git_dir, worktree) = gitdir.rsplit_once("/worktrees/")?;
    if worktree.is_empty() {
        return None;
    }
    let root = git_dir.strip_suffix("/.git").unwrap_or(git_dir);
    (!root.is_empty()).then(|| root.to_string())
}

/// The repository root for `path` whether its worktree lives inside the repo
/// (a path prefix) or beside it (a `gitdir:` pointer file).
pub fn workspace_repo_root_resolved(path: &str) -> Option<String> {
    workspace_repo_root(path).or_else(|| workspace_git_worktree_root(path))
}

/// The real filesystem path a workspace key names: the key itself when it is
/// already a path, or the directory a Claude Code slug was built from.
///
/// `None` for keys that are not paths at all (Warp's workspace UUID) and for
/// slugs whose directory is gone — which is exactly when there is no parent
/// segment available to disambiguate a colliding label with.
pub fn workspace_path_for_key(key: &str) -> Option<String> {
    workspace_path_for_decoded_key(key, decode_claude_project_slug(key).as_deref())
}

/// [`workspace_path_for_key`] with the slug decode already done.
///
/// Decoding walks the filesystem, and a caller that needs the label, the path
/// and the repo root for one key would otherwise pay for that walk three times.
pub fn workspace_path_for_decoded_key(key: &str, decoded: Option<&str>) -> Option<String> {
    if let Some(decoded) = decoded {
        return Some(decoded.to_string());
    }
    let normalized = normalize_workspace_key(key)?;
    normalized.contains('/').then_some(normalized)
}

/// Human-readable label for a workspace key: `repo` or `repo ⑃ worktree`.
///
/// The key is whatever the originating client wrote to disk, so this also has to
/// cope with Claude Code's dash-mangled directory slug
/// (`-Users-zetian-devpro-ing-claude-witness`), which carries no `/` to split on
/// and therefore used to render as the entire path — the exact prefix every row
/// shares, so truncation dropped the only distinguishing part.
pub fn workspace_display_label(key: &str) -> Option<String> {
    workspace_display_label_for_decoded_key(key, decode_claude_project_slug(key).as_deref())
}

/// [`workspace_display_label`] with the slug decode already done. See
/// [`workspace_path_for_decoded_key`] for why the decode is hoisted out.
pub fn workspace_display_label_for_decoded_key(key: &str, decoded: Option<&str>) -> Option<String> {
    // Normalize before splitting: a client that recorded a raw Windows path
    // (`C:\a\repo`) carries no `/` to split on, so without this the label
    // would be the whole path — the same unreadable row this function exists to
    // prevent, on the one platform the slug decoder cannot help with either.
    let path = decoded
        .map(str::to_string)
        .or_else(|| normalize_workspace_key(key))
        .unwrap_or_else(|| key.to_string());

    if let Some(root) = workspace_repo_root_resolved(&path) {
        let repo = workspace_label_from_key(&root)?;
        return match workspace_label_from_key(&path) {
            Some(worktree) => Some(format!("{repo}{WORKTREE_SEPARATOR}{worktree}")),
            None => Some(repo),
        };
    }

    // Undecodable slug (the directory was deleted): the marker still tells us
    // where the repo ends, so name it from the string rather than giving up and
    // showing the whole mangled path.
    if let Some((repo_slug, worktree)) = split_encoded_worktree(&path) {
        let repo = decode_claude_project_slug(&repo_slug)
            .and_then(|decoded| workspace_label_from_key(&decoded))
            .or_else(|| last_slug_segment(&repo_slug))?;
        return Some(format!("{repo}{WORKTREE_SEPARATOR}{worktree}"));
    }

    workspace_label_from_key(&path)
}

/// Repo identity for a dash-encoded worktree slug whose directory no longer
/// exists, so rollup can still merge it into its repository. Prefers the repo's
/// real path when THAT still resolves, falling back to the repo slug itself —
/// which keeps deleted worktrees of one repo together even then.
pub fn workspace_repo_root_from_slug(key: &str) -> Option<String> {
    let (repo_slug, _) = split_encoded_worktree(key)?;
    Some(decode_claude_project_slug(&repo_slug).unwrap_or(repo_slug))
}

/// Best-effort trailing name of a dash-encoded slug whose directory is gone. The
/// original `/` boundaries are unrecoverable, so this returns the last dash
/// segment — a hint, not an exact path.
fn last_slug_segment(slug: &str) -> Option<String> {
    slug.rsplit('-')
        .find(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
}

/// Claude Code names each project directory after the absolute path it was
/// launched from, replacing every non-alphanumeric byte with `-`. That map is
/// lossy — `/`, `.`, `+` and `-` all collapse to `-` — so it cannot be inverted
/// by string surgery alone. Instead this walks the filesystem, re-applying the
/// same map to real directory names to find which one the slug came from, which
/// makes the recovered path exact rather than a guess.
///
/// Returns `None` for keys that are already real paths, or when no directory on
/// disk matches (a project whose folder has since been deleted or renamed).
pub fn decode_claude_project_slug(key: &str) -> Option<String> {
    // A real path (already usable) keeps its separators; `normalize_workspace_key`
    // rewrites Windows backslashes to `/`, so one check covers both platforms.
    if key.contains('/') {
        return None;
    }

    let (root, remaining) = slug_root_and_remainder(key)?;
    let mut budget = SLUG_DECODE_STEP_BUDGET;
    resolve_slug_under(&root, remaining, &mut budget)
}

/// Filesystem probes one slug decode may spend before it gives up.
///
/// `resolve_slug_under` backtracks, so its search tree grows with the number of
/// dashes a slug can split on: an adversarial 69-character key laid out over a
/// few symlinked directories took 7.3s for a single label, and the labeler asked
/// for it once per method. A real slug resolves in roughly one probe per path
/// segment, so this is orders of magnitude above any honest decode while still
/// making the worst case finite. Exceeding it costs a prettier label, never
/// correctness: the caller falls back to naming the row from the slug string.
const SLUG_DECODE_STEP_BUDGET: u32 = 4_096;

/// Charge `cost` probes against `budget`, reporting whether it could be paid.
fn spend_slug_budget(budget: &mut u32, cost: u32) -> bool {
    match budget.checked_sub(cost) {
        Some(remaining) => {
            *budget = remaining;
            true
        }
        None => {
            *budget = 0;
            false
        }
    }
}

/// Split a slug into the filesystem root it was anchored at and the rest.
///
/// A POSIX slug begins at `/`, so it opens with the separator-turned-dash. A
/// Windows slug encodes the drive instead (`C:\Users\me` becomes `C--Users-me`:
/// one dash for the colon, one for the separator), so the root has to be
/// reconstructed from the drive letter rather than assumed to be `/`.
fn slug_root_and_remainder(key: &str) -> Option<(PathBuf, &str)> {
    if key.starts_with('-') {
        // Pass the whole key through: `slug_matches_prefix` expects every segment
        // to arrive separator-first, including the first one.
        return Some((PathBuf::from(MAIN_SEPARATOR_STR), key));
    }

    // Both the colon and the separator arrive encoded as `-`; the remainder
    // keeps its leading dash so `slug_matches_prefix` still sees every segment
    // separator-first.
    let (drive, remaining) = windows_drive_root(key, '-', '-')?;
    Some((
        PathBuf::from(format!("{drive}:{MAIN_SEPARATOR_STR}")),
        remaining,
    ))
}

/// Walk `remaining` against the real directories under `dir`.
///
/// A dash in the slug is ambiguous — it may be a `/` boundary, or part of a
/// directory name that genuinely contains `-`, `.` or `+` — so a single greedy
/// pass mis-resolves paths like `claude-witness` (one directory, not two). This
/// consumes one real directory at a time and backtracks when a branch dead-ends,
/// which makes the result exact wherever the directory still exists on disk.
fn resolve_slug_under(dir: &Path, remaining: &str, budget: &mut u32) -> Option<String> {
    if remaining.is_empty() {
        // Hand back a normalized key, not a native path. Every consumer compares
        // against forward-slash markers (`workspace_repo_root` looks for
        // `.claude/worktrees/`), and on Windows `Path::join` produces `\`, so
        // returning the native spelling would silently defeat worktree rollup and
        // make a decoded key unequal to the same directory recorded by a client
        // that stores a real path.
        return normalize_workspace_key(&dir.to_string_lossy());
    }

    // One probe for the directory listing this node is about to make.
    if !spend_slug_budget(budget, 1) {
        return None;
    }

    let matched: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        // Name filter first: it is pure string work, and it rejects nearly every
        // entry in a large directory. The `is_dir` check below costs a `stat` per
        // survivor, so ordering it second keeps the syscalls proportional to
        // matches rather than to directory size.
        .filter(|name| slug_matches_prefix(remaining, name))
        .collect();
    // The `stat` per survivor is the other unbounded cost here, so charge for it
    // before paying it.
    if !spend_slug_budget(budget, matched.len() as u32) {
        return None;
    }

    // Longest candidate first: prefer `IngTian.github.io` over a shorter
    // `IngTian` that happens to also exist.
    let mut candidates: Vec<String> = matched
        .into_iter()
        // `Path::is_dir` follows symlinks where `DirEntry::file_type` would not.
        // Symlinked directories are load-bearing here: macOS reaches temp dirs
        // through `/var -> /private/var`, and users symlink project roots.
        .filter(|name| dir.join(name).is_dir())
        .collect();
    // Ties are real: `a.b` and `a-b` encode identically and nothing on disk
    // distinguishes them, so order deterministically instead of trusting
    // readdir order.
    candidates.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    for name in candidates {
        let consumed = slugify_path_segment(&name).len() + 1;
        if let Some(resolved) = resolve_slug_under(&dir.join(&name), &remaining[consumed..], budget)
        {
            return Some(resolved);
        }
    }

    None
}

/// Whether `remaining` starts with `-` + the encoded form of `name`, ending on a
/// segment boundary so a directory cannot match half of a longer name.
fn slug_matches_prefix(remaining: &str, name: &str) -> bool {
    let encoded = slugify_path_segment(name);
    let Some(rest) = remaining.strip_prefix('-') else {
        return false;
    };
    let Some(tail) = rest.strip_prefix(encoded.as_str()) else {
        return false;
    };
    tail.is_empty() || tail.starts_with('-')
}

/// Claude Code's forward map: every non-alphanumeric byte becomes `-`.
fn slugify_path_segment(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// A temp directory whose path is spelled the way `read_dir` reports it.
///
/// The slug decoder matches against real directory entries, so the fixture path
/// has to agree with what the OS enumerates. On Windows `%TEMP%` is often an 8.3
/// short name (`RUNNER~1`) while `read_dir` yields the long name
/// (`runneradmin`), and `canonicalize` returns a `\\?\` verbatim prefix that is
/// not a walkable root — so strip that and let the walk start at the drive.
/// Without this the slug describes a path no directory listing contains and the
/// decode correctly finds nothing.
///
/// Lives outside `mod tests` so the decoder tests here and the aggregation tests
/// in `crate::lib` share one copy: two copies means a verbatim-prefix fix lands
/// in one of them and the other keeps failing on Windows only.
#[cfg(test)]
pub(crate) fn canonical_tempdir() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(temp.path()).unwrap();
    let spelled = canonical.to_string_lossy().to_string();
    let stripped = spelled
        .strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or(canonical);
    (temp, stripped)
}

/// Convert Unix milliseconds to a local YYYY-MM-DD date string.
fn timestamp_to_date(timestamp_ms: i64) -> String {
    timestamp_to_date_with_timezone(timestamp_ms, &chrono::Local)
}

fn timestamp_to_date_with_timezone<Tz>(timestamp_ms: i64, timezone: &Tz) -> String
where
    Tz: chrono::TimeZone,
    Tz::Offset: std::fmt::Display,
{
    crate::bucket_tz::format_day_key(timestamp_ms, timezone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn workspace_repo_root_strips_worktree_suffixes() {
        assert_eq!(
            workspace_repo_root("/Users/z/devpro/witness/.claude/worktrees/lens-backfill")
                .as_deref(),
            Some("/Users/z/devpro/witness")
        );
        assert_eq!(
            workspace_repo_root("/Users/z/devpro/witness/.git/worktrees/wt-1").as_deref(),
            Some("/Users/z/devpro/witness")
        );
        // Plain repo checkouts have nothing to roll up.
        assert_eq!(workspace_repo_root("/Users/z/devpro/witness"), None);
        // Opaque, non-path keys (Warp's workspace UUID) must not be mangled.
        assert_eq!(
            workspace_repo_root("9f2c1a04-1e4b-4c3f-a0d1-77b2e5c9aa10"),
            None
        );
    }

    #[test]
    fn workspace_display_label_names_repo_and_worktree() {
        assert_eq!(
            workspace_display_label("/Users/z/devpro/witness/.claude/worktrees/lens-backfill")
                .as_deref(),
            Some("witness ⑃ lens-backfill")
        );
        assert_eq!(
            workspace_display_label("/Users/z/devpro/witness").as_deref(),
            Some("witness")
        );
    }

    /// A directory that merely contains `.git` in its NAME is not a worktree.
    /// Substring matching folded `/notes/my.git/worktrees/draft` into a `/notes/my`
    /// that does not exist, which under `--merge-worktrees` invented a row.
    #[test]
    fn worktree_markers_only_match_whole_path_segments() {
        assert_eq!(workspace_repo_root("/notes/my.git/worktrees/draft"), None);
        assert_eq!(
            workspace_repo_root("/notes/my.claude/worktrees/draft"),
            None
        );
        assert_eq!(
            workspace_display_label("/notes/my.git/worktrees/draft").as_deref(),
            Some("draft")
        );
        // The real layout still resolves.
        assert_eq!(
            workspace_repo_root("/notes/my/.git/worktrees/draft").as_deref(),
            Some("/notes/my")
        );
    }

    /// A worktree checked out inside another worktree belongs to the repository
    /// at the top, not to the intermediate worktree — the first marker in the
    /// path is the one the repo owns.
    #[test]
    fn nested_worktrees_roll_up_to_the_outermost_repo() {
        let nested = "/Users/z/devpro/witness/.claude/worktrees/outer/.claude/worktrees/inner";
        assert_eq!(
            workspace_repo_root(nested).as_deref(),
            Some("/Users/z/devpro/witness")
        );
        assert_eq!(
            workspace_display_label(nested).as_deref(),
            Some("witness ⑃ inner")
        );
    }

    /// Nested worktrees of DIFFERENT kinds: the repo ends at the marker that
    /// appears first in the path, not at whichever marker is listed first in
    /// `WORKTREE_MARKERS`. Iterating the array answered `/a/.git/worktrees/x` for
    /// the first case below, so `--merge-worktrees` gave one repository two rows.
    #[test]
    fn nested_worktrees_of_mixed_kinds_roll_up_to_the_outermost_repo() {
        assert_eq!(
            workspace_repo_root("/a/.git/worktrees/x/.claude/worktrees/y").as_deref(),
            Some("/a")
        );
        assert_eq!(
            workspace_repo_root("/a/.claude/worktrees/x/.git/worktrees/y").as_deref(),
            Some("/a")
        );
        assert_eq!(
            workspace_display_label("/a/.git/worktrees/x/.claude/worktrees/y").as_deref(),
            Some("a ⑃ y")
        );
        // Same rule for the dash-encoded form, whose directory is gone. The root
        // segment is spelled so that no machine has such a directory: these
        // assertions are about the string fallback, and a slug that happens to
        // resolve on the runner (Windows CI really does have a `C:\a`) would be
        // testing the decoder instead.
        let encoded = "-tokscale-probe-repo--git-worktrees-x--claude-worktrees-y";
        assert_eq!(
            workspace_display_label(encoded).as_deref(),
            Some("repo ⑃ y")
        );
        assert_eq!(
            workspace_repo_root_from_slug(encoded).as_deref(),
            Some("-tokscale-probe-repo")
        );
    }

    /// The `.git` pointer read runs once per distinct workspace key on every
    /// report and every TUI refresh, against directories nobody vetted. A FIFO
    /// there blocks `open` until something writes to the other end, which wedged
    /// the whole report; a huge regular file was read into memory in full.
    #[test]
    fn git_pointer_reads_refuse_non_files_and_oversized_files() {
        let (_temp, root) = canonical_tempdir();

        let oversized = root.join("oversized");
        std::fs::create_dir_all(&oversized).unwrap();
        let mut body = String::from(
            "gitdir: /repo/.git/worktrees/x
",
        );
        body.push_str(&"x".repeat(GIT_POINTER_MAX_BYTES as usize + 1));
        std::fs::write(oversized.join(".git"), body).unwrap();
        assert_eq!(
            workspace_git_worktree_root(&oversized.to_string_lossy()),
            None,
            "a .git file past the cap must not be read"
        );

        // A directory named `.git` is an ordinary checkout, not a pointer.
        let checkout = root.join("checkout");
        std::fs::create_dir_all(checkout.join(".git")).unwrap();
        assert_eq!(
            workspace_git_worktree_root(&checkout.to_string_lossy()),
            None
        );

        // A pointer just under the cap still resolves, so the guard is not a
        // blanket refusal.
        let ok = root.join("ok");
        std::fs::create_dir_all(&ok).unwrap();
        std::fs::write(
            ok.join(".git"),
            "gitdir: /srv/api/.git/worktrees/x
",
        )
        .unwrap();
        assert_eq!(
            workspace_git_worktree_root(&ok.to_string_lossy()).as_deref(),
            Some("/srv/api")
        );
    }

    /// The hang the size cap alone would not catch: `read_to_string` on a FIFO
    /// never returns. The test asserts the call comes back at all — under the old
    /// code it blocks until the harness is killed.
    #[cfg(unix)]
    #[test]
    fn git_pointer_reads_do_not_block_on_a_fifo() {
        use std::ffi::CString;

        let (_temp, root) = canonical_tempdir();
        let fifo_dir = root.join("fifo");
        std::fs::create_dir_all(&fifo_dir).unwrap();
        let fifo = fifo_dir.join(".git");
        let path = CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: `path` is a NUL-terminated path inside a fresh temp directory.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o644) }, 0);

        assert_eq!(
            workspace_git_worktree_root(&fifo_dir.to_string_lossy()),
            None
        );
        // And through the public entry point the report actually calls.
        assert_eq!(
            workspace_repo_root_resolved(&fifo_dir.to_string_lossy()),
            None
        );
    }

    /// A `.git` symlink is resolved deliberately: an ordinary file behind it is
    /// still a valid pointer, but a FIFO behind it must be refused just like a
    /// bare FIFO.
    #[cfg(unix)]
    #[test]
    fn git_pointer_symlinks_are_resolved_then_rechecked() {
        use std::ffi::CString;

        let (_temp, root) = canonical_tempdir();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("pointer"),
            "gitdir: /srv/api/.git/worktrees/x
",
        )
        .unwrap();

        let linked = root.join("linked");
        std::fs::create_dir_all(&linked).unwrap();
        std::os::unix::fs::symlink(root.join("pointer"), linked.join(".git")).unwrap();
        assert_eq!(
            workspace_git_worktree_root(&linked.to_string_lossy()).as_deref(),
            Some("/srv/api")
        );

        let fifo = root.join("target-fifo");
        let path = CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: `path` is a NUL-terminated path inside a fresh temp directory.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o644) }, 0);
        let linked_fifo = root.join("linked-fifo");
        std::fs::create_dir_all(&linked_fifo).unwrap();
        std::os::unix::fs::symlink(&fifo, linked_fifo.join(".git")).unwrap();
        assert_eq!(
            workspace_git_worktree_root(&linked_fifo.to_string_lossy()),
            None
        );
    }

    /// The slug decoder backtracks, so a dash-dense key has a search tree that
    /// grows exponentially in the number of dashes. Without a budget one 69-char
    /// key spent 7.3s in `resolve_slug_under`. The budget makes the worst case
    /// finite; an honest slug never comes close to it.
    #[test]
    fn slug_decoding_is_bounded_and_still_resolves_real_paths() {
        let (_temp, root) = canonical_tempdir();

        // A directory tree where every level offers several encode-identical
        // names, which is what makes the search branch.
        let mut dir = root.clone();
        for _ in 0..8 {
            for name in ["a-b", "a.b", "a+b"] {
                std::fs::create_dir_all(dir.join(name)).unwrap();
            }
            dir = dir.join("a-b");
        }
        let slug = format!(
            "{}{}",
            slugify_path_segment(&root.to_string_lossy()),
            "-a-b".repeat(9)
        );

        let started = std::time::Instant::now();
        let decoded = decode_claude_project_slug(&slug);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "bounded decode took {:?}",
            started.elapsed()
        );
        // Whatever it answers, it must not be a lie: either the real directory or
        // nothing at all.
        if let Some(decoded) = decoded {
            assert!(Path::new(&decoded).is_dir(), "decoded to {decoded}");
        }

        // An exhausted budget refuses rather than returning a wrong path.
        let mut spent = 0u32;
        assert_eq!(resolve_slug_under(&root, "-a-b", &mut spent), None);

        // And an ordinary slug still decodes with the budget in place.
        let plain = root.join("devpro/claude-witness");
        std::fs::create_dir_all(&plain).unwrap();
        let plain_slug = slugify_path_segment(&plain.to_string_lossy());
        assert_eq!(
            decode_claude_project_slug(&plain_slug),
            normalize_workspace_key(&plain.to_string_lossy())
        );
    }

    /// Clients that never normalized their keys hand us native Windows paths.
    /// Splitting on `/` alone made the entire path the label, which is the
    /// unreadable row this labeling exists to prevent.
    #[test]
    fn workspace_display_label_handles_windows_style_paths() {
        assert_eq!(
            workspace_display_label(r"C:\Users\me\devpro\app").as_deref(),
            Some("app")
        );
        assert_eq!(
            workspace_display_label(r"C:\Users\me\devpro\app\.claude\worktrees\feature-x")
                .as_deref(),
            Some("app ⑃ feature-x")
        );
        assert_eq!(
            workspace_display_label(r"\\server\share\team\app").as_deref(),
            Some("app")
        );
    }

    /// `git worktree add ../feature-x` leaves no marker in the worktree's own
    /// path: only a `.git` FILE pointing back at the repository. Without reading
    /// it, `--merge-worktrees` silently left those rows unmerged.
    #[test]
    fn sibling_worktrees_resolve_their_repo_through_the_gitdir_pointer() {
        let (_temp, root) = canonical_tempdir();
        let repo = root.join("devpro/api");
        let worktree = root.join("devpro/api-feature-x");
        std::fs::create_dir_all(repo.join(".git/worktrees/feature-x")).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!(
                "gitdir: {}\n",
                repo.join(".git/worktrees/feature-x").to_string_lossy()
            ),
        )
        .unwrap();

        let key = normalize_workspace_key(&worktree.to_string_lossy()).unwrap();
        let expected = normalize_workspace_key(&repo.to_string_lossy()).unwrap();

        assert_eq!(workspace_repo_root(&key), None, "no marker in its own path");
        assert_eq!(
            workspace_repo_root_resolved(&key).as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            workspace_display_label(&key).as_deref(),
            Some("api ⑃ api-feature-x")
        );
    }

    /// Git writes the pointer relative to the worktree whenever the two live on
    /// the same volume, which is the common case for `git worktree add ../x`.
    /// Joining it leaves a `..` in the middle of the path, and a repo identity
    /// spelled `/work/feature-x/../api` never string-matches the repository's
    /// own row, so rollup silently did nothing.
    #[test]
    fn relative_gitdir_pointers_resolve_to_the_repos_own_key() {
        let (_temp, root) = canonical_tempdir();
        let repo = root.join("devpro/api");
        let worktree = root.join("devpro/api-feature-x");
        std::fs::create_dir_all(repo.join(".git/worktrees/feature-x")).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(
            worktree.join(".git"),
            "gitdir: ../api/.git/worktrees/feature-x\n",
        )
        .unwrap();

        let key = normalize_workspace_key(&worktree.to_string_lossy()).unwrap();
        let expected = normalize_workspace_key(&repo.to_string_lossy()).unwrap();
        assert_eq!(
            workspace_repo_root_resolved(&key).as_deref(),
            Some(expected.as_str()),
            "the rolled-up identity must be spelled like the repo's own key"
        );
    }

    /// A worktree of a BARE repository points at `/srv/api.git/worktrees/x`,
    /// which carries no standalone `.git` segment for the marker search to find.
    /// The pointer file is what proves `api.git` is a repository rather than a
    /// directory that happens to end in `.git`.
    #[test]
    fn bare_repository_worktrees_resolve_to_the_bare_repo() {
        let (_temp, root) = canonical_tempdir();
        let bare = root.join("srv/api.git");
        let worktree = root.join("srv/feature-x");
        std::fs::create_dir_all(bare.join("worktrees/feature-x")).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!(
                "gitdir: {}\n",
                bare.join("worktrees/feature-x").to_string_lossy()
            ),
        )
        .unwrap();

        let key = normalize_workspace_key(&worktree.to_string_lossy()).unwrap();
        let expected = normalize_workspace_key(&bare.to_string_lossy()).unwrap();
        assert_eq!(
            workspace_repo_root_resolved(&key).as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            workspace_display_label(&key).as_deref(),
            Some("api.git ⑃ feature-x")
        );
        // A path that merely ENDS in `.git`, with no pointer file vouching for
        // it, is still not a repository.
        assert_eq!(workspace_repo_root("/notes/my.git/worktrees/draft"), None);
    }

    /// A submodule's `.git` file points at `.git/modules/...`, which is not a
    /// worktree — rolling it up would merge a submodule into its superproject.
    /// An ordinary checkout has a `.git` DIRECTORY and must be left alone too.
    #[test]
    fn gitdir_pointers_that_are_not_worktrees_are_ignored() {
        let (_temp, root) = canonical_tempdir();
        let submodule = root.join("super/vendor/lib");
        std::fs::create_dir_all(&submodule).unwrap();
        std::fs::create_dir_all(root.join("super/.git/modules/vendor/lib")).unwrap();
        std::fs::write(
            submodule.join(".git"),
            "gitdir: ../../.git/modules/vendor/lib\n",
        )
        .unwrap();

        let key = normalize_workspace_key(&submodule.to_string_lossy()).unwrap();
        assert_eq!(workspace_repo_root_resolved(&key), None);
        assert_eq!(workspace_display_label(&key).as_deref(), Some("lib"));

        let plain = root.join("super");
        std::fs::create_dir_all(plain.join(".git")).unwrap();
        let plain_key = normalize_workspace_key(&plain.to_string_lossy()).unwrap();
        assert_eq!(workspace_repo_root_resolved(&plain_key), None);
    }

    /// A Claude Code slug is a RELATIVE key: on its own it names no directory,
    /// so joining `.git` onto it resolved against the process working directory
    /// and read `$CWD/<slug>/.git`. Any repository can carry such a file, and
    /// the pointer it holds renamed the row and — under `--merge-worktrees` —
    /// re-keyed it, purely because of where the binary happened to be run.
    #[test]
    #[serial_test::serial]
    fn relative_keys_never_read_a_pointer_file_from_the_working_directory() {
        let (_temp, root) = canonical_tempdir();
        let slug = "-Users-junhoyeo-mlx-motif";
        std::fs::create_dir_all(root.join(slug)).unwrap();
        std::fs::write(
            root.join(slug).join(".git"),
            "gitdir: /srv/evilrepo/.git/worktrees/pwned\n",
        )
        .unwrap();

        // Restore before asserting so a failure cannot strand the whole test
        // binary in the temp directory.
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();
        let pointer_root = workspace_git_worktree_root(slug);
        let resolved = workspace_repo_root_resolved(slug);
        let label = workspace_display_label(slug);
        std::env::set_current_dir(&previous).unwrap();

        assert_eq!(pointer_root, None, "a relative key names no directory");
        assert_eq!(resolved, None, "--merge-worktrees must not re-key the row");
        assert_eq!(label.as_deref(), Some(slug));
    }

    /// The same hole through the other door: the absoluteness test normalizes
    /// separators, so `C:\Users\me\repo` passed it on macOS and Linux, but the
    /// pointer read hands the RAW key to `Path`, where that string is a single
    /// relative filename. A directory of that name in `$CWD` got its `.git`
    /// read and re-keyed the row, exactly as the slug used to.
    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn windows_shaped_keys_never_read_a_pointer_file_from_the_working_directory() {
        let (_temp, root) = canonical_tempdir();
        let key = "C:\\Users\\me\\repo";
        std::fs::create_dir_all(root.join(key)).unwrap();
        std::fs::write(
            root.join(key).join(".git"),
            "gitdir: /srv/evilrepo/.git/worktrees/pwned\n",
        )
        .unwrap();

        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();
        let pointer_root = workspace_git_worktree_root(key);
        let resolved = workspace_repo_root_resolved(key);
        std::env::set_current_dir(&previous).unwrap();

        assert_eq!(
            pointer_root, None,
            "a Windows-shaped key names no directory on this host"
        );
        assert_eq!(resolved, None, "--merge-worktrees must not re-key the row");
    }

    /// `C:\` is a root, not a path with a trailing separator. Trimming it left
    /// `C:` — Windows' drive-RELATIVE form — so the top of a drive classified as
    /// relative while POSIX `/` stayed absolute.
    #[test]
    fn windows_drive_roots_stay_absolute() {
        assert_eq!(normalize_workspace_key("C:\\").as_deref(), Some("C:/"));
        assert_eq!(normalize_workspace_key("C:/").as_deref(), Some("C:/"));
        assert_eq!(normalize_workspace_key("C:///").as_deref(), Some("C:/"));
        assert!(is_absolute_workspace_key("C:\\"));
        assert!(is_absolute_workspace_key("C:/"));
        assert!(is_absolute_workspace_key("/"));

        // A bare drive is not a root: `C:work` is relative to whatever directory
        // that drive is currently sitting in.
        assert!(!is_absolute_workspace_key("C:"));
        assert!(!is_absolute_workspace_key("C:work"));
        assert!(!is_absolute_workspace_key("repo/x"));

        // Deeper keys still lose their trailing separator.
        assert_eq!(
            normalize_workspace_key("C:\\work\\api\\").as_deref(),
            Some("C:/work/api")
        );
    }

    #[test]
    fn workspace_path_for_key_separates_paths_from_opaque_ids() {
        assert_eq!(
            workspace_path_for_key("/Users/z/devpro/witness").as_deref(),
            Some("/Users/z/devpro/witness")
        );
        assert_eq!(
            workspace_path_for_key(r"C:\work\api").as_deref(),
            Some("C:/work/api")
        );
        // Opaque client ids and slugs whose directory is gone carry no parent
        // segment, which is what makes the label fall back to the key itself.
        assert_eq!(
            workspace_path_for_key("9f2c1a04-1e4b-4c3f-a0d1-77b2e5c9aa10"),
            None
        );
        assert_eq!(
            workspace_path_for_key("-nonexistent-tokscale-probe-dir-xyz"),
            None
        );
    }

    #[test]
    fn decode_claude_project_slug_recovers_names_containing_dashes() {
        let (_temp, root) = canonical_tempdir();
        // A real name with a literal '-' is the case a greedy split gets wrong:
        // "claude-witness" is ONE directory, not "claude" then "witness".
        let real = root.join("devpro/ing/claude-witness");
        std::fs::create_dir_all(&real).unwrap();

        let decoded = decode_claude_project_slug(&slug_for(&real))
            .expect("slug should resolve to the directory it was built from");

        assert_eq!(
            std::fs::canonicalize(decoded).unwrap(),
            std::fs::canonicalize(&real).unwrap()
        );
    }

    /// Encode a real path the way Claude Code names its project directory, going
    /// through `normalize_workspace_key` first so Windows backslashes become `/`
    /// exactly as they do in production before the slug is formed.
    fn slug_for(path: &Path) -> String {
        let normalized = normalize_workspace_key(&path.to_string_lossy()).unwrap();
        super::slugify_path_segment(&normalized)
    }

    #[test]
    fn decode_claude_project_slug_recovers_dots_and_worktrees() {
        let (_temp, root) = canonical_tempdir();
        // '.' also encodes to '-', so "IngTian.github.io" and the ".claude"
        // worktree marker both have to come back exactly.
        let real = root.join("ing/IngTian.github.io/.claude/worktrees/scroll-reveal");
        std::fs::create_dir_all(&real).unwrap();

        let decoded = decode_claude_project_slug(&slug_for(&real))
            .expect("slug should resolve to the directory it was built from");

        assert_eq!(
            std::fs::canonicalize(decoded).unwrap(),
            std::fs::canonicalize(&real).unwrap()
        );
    }

    #[test]
    fn decode_claude_project_slug_handles_non_ascii_directory_names() {
        // `slugify_path_segment` maps each non-alphanumeric CHAR to one '-', so a
        // multi-byte name encodes SHORTER than its byte length ("café" is 5 bytes
        // and encodes to "caf-" at 4; "日本語" is 9 and encodes to "---" at 3).
        // `resolve_slug_under` then advances by the ENCODED length and slices
        // `&remaining[consumed..]`, which is a byte index — so this pins that the
        // walk stays aligned and never slices mid-character. What keeps it safe is
        // `slug_matches_prefix`: it only admits a candidate whose encoded form is
        // an ASCII prefix of `remaining`, so `consumed` always lands on a char
        // boundary. Without that guard this input would panic rather than degrade.
        let (_temp, root) = canonical_tempdir();
        let real = root.join("café/日本語/my-app");
        std::fs::create_dir_all(&real).unwrap();

        let decoded = decode_claude_project_slug(&slug_for(&real))
            .expect("non-ASCII directory names must still resolve");

        assert_eq!(
            std::fs::canonicalize(decoded).unwrap(),
            std::fs::canonicalize(&real).unwrap()
        );
    }

    #[test]
    fn decoded_slugs_use_normalized_separators() {
        // The decoder builds its result with `Path::join`, which emits `\` on
        // Windows. Every consumer compares against forward-slash markers, so a
        // native-spelled key silently defeats worktree rollup there and never
        // compares equal to the same directory recorded by a client that stores a
        // real path. Asserting on the returned string keeps that platform-specific
        // failure visible from any host.
        let (_temp, root) = canonical_tempdir();
        let real = root.join("repo-a/.claude/worktrees/feature-x");
        std::fs::create_dir_all(&real).unwrap();

        let decoded = decode_claude_project_slug(&slug_for(&real)).expect("slug should resolve");

        assert!(
            !decoded.contains('\\'),
            "decoded key must not carry native separators: {decoded}"
        );
        // And the worktree marker must therefore be findable.
        assert!(
            workspace_repo_root(&decoded).is_some(),
            "worktree marker must be visible in {decoded}"
        );
    }

    #[test]
    fn slug_root_and_remainder_anchors_posix_and_windows_slugs() {
        // POSIX: the leading dash IS the root separator, and stays in the
        // remainder so the first segment arrives separator-first.
        let (root, remaining) = super::slug_root_and_remainder("-Users-me-app").unwrap();
        assert_eq!(root, PathBuf::from(MAIN_SEPARATOR_STR));
        assert_eq!(remaining, "-Users-me-app");

        // Windows: `C:\Users\me\app` encodes as `C--Users-me-app` -- one dash for
        // the colon, one for the separator -- so the drive has to be rebuilt
        // rather than assumed to be `/`.
        let (root, remaining) = super::slug_root_and_remainder("C--Users-me-app").unwrap();
        assert_eq!(root, PathBuf::from(format!("C:{MAIN_SEPARATOR_STR}")));
        assert_eq!(remaining, "-Users-me-app");

        // Neither shape: nothing to anchor against.
        assert_eq!(super::slug_root_and_remainder("Users-me-app"), None);
        assert_eq!(super::slug_root_and_remainder("1--Users-me"), None);
        assert_eq!(super::slug_root_and_remainder(""), None);
    }

    #[test]
    fn decode_claude_project_slug_ignores_real_paths_and_unknown_dirs() {
        // Already a path: nothing to decode.
        assert_eq!(decode_claude_project_slug("/Users/z/devpro/witness"), None);
        // Slug whose directory does not exist on disk.
        assert_eq!(
            decode_claude_project_slug("-nonexistent-tokscale-probe-dir-xyz"),
            None
        );
    }

    #[test]
    fn deleted_worktree_slugs_still_name_and_group_by_their_repo() {
        // A worktree deleted from disk cannot be resolved, but its slug still
        // carries the marker, so it must not fall back to the raw mangled key.
        let slug = "-Users-zed-devpro-ing-claude-witness--claude-worktrees-store-c1-dissolve";

        assert_eq!(
            workspace_display_label(slug).as_deref(),
            Some("witness ⑃ store-c1-dissolve")
        );
        // And every deleted worktree of that repo shares one rollup identity.
        assert_eq!(
            workspace_repo_root_from_slug(slug).as_deref(),
            Some("-Users-zed-devpro-ing-claude-witness")
        );
        assert_eq!(
            workspace_repo_root_from_slug(
                "-Users-zed-devpro-ing-claude-witness--claude-worktrees-proc-port-43"
            )
            .as_deref(),
            Some("-Users-zed-devpro-ing-claude-witness")
        );
    }

    #[test]
    fn workspace_display_label_falls_back_to_raw_key_when_undecodable() {
        // A deleted project directory cannot be resolved, so the label stays the
        // raw slug rather than becoming empty.
        let slug = "-nonexistent-tokscale-probe-dir-xyz";
        assert_eq!(workspace_display_label(slug).as_deref(), Some(slug));
    }

    #[test]
    fn warp_cache_parser_preserves_requests_and_spend_without_tokens() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            r#"{
  "version": 1,
  "syncedAt": "2026-05-29T12:00:00Z",
  "usage": {
    "requestsUsed": 42,
    "requestLimit": 100,
    "spendCents": 1234,
    "nextRefreshTime": "2026-06-01T00:00:00Z"
  },
  "workspaces": [
    {
      "id": "workspace-1",
      "name": "Personal",
      "requestsUsed": 12,
      "spendCents": 345
    }
  ]
}"#,
        )
        .unwrap();

        let messages = crate::sessions::warp::parse_warp_file(file.path());
        assert_eq!(messages.len(), 1);

        let workspace = messages
            .iter()
            .find(|message| message.session_id == "warp-aggregate-workspace-1")
            .unwrap();
        assert_eq!(workspace.client, "warp");
        assert_eq!(workspace.model_id, "aggregate-requests");
        assert_eq!(workspace.provider_id, "warp");
        assert_eq!(workspace.workspace_label.as_deref(), Some("Personal"));
        assert_eq!(workspace.message_count, 12);
        assert_eq!(workspace.tokens, TokenBreakdown::default());
        assert!((workspace.cost - 3.45).abs() < 1e-9);

        std::fs::write(
            file.path(),
            r#"{
  "version": 1,
  "syncedAt": "2026-05-29T12:00:00Z",
  "usage": {
    "requestsUsed": 42,
    "requestLimit": 100,
    "spendCents": 1234,
    "nextRefreshTime": "2026-06-01T00:00:00Z"
  },
  "workspaces": []
}"#,
        )
        .unwrap();

        let messages = crate::sessions::warp::parse_warp_file(file.path());
        assert_eq!(messages.len(), 1);
        let account = &messages[0];
        assert_eq!(account.session_id, "warp-aggregate-account");
        assert_eq!(account.message_count, 42);
        assert_eq!(account.tokens, TokenBreakdown::default());
        assert!((account.cost - 12.34).abs() < 1e-9);
    }

    #[test]
    fn test_timestamp_to_date_with_positive_offset() {
        let kst = FixedOffset::east_opt(9 * 60 * 60).unwrap();
        let ts = 1772512200000_i64; // 2026-03-03T04:30:00Z
        let date = timestamp_to_date_with_timezone(ts, &kst);
        assert_eq!(date, "2026-03-03");
    }

    #[test]
    fn test_timestamp_to_date_with_negative_offset() {
        let pst = FixedOffset::west_opt(8 * 60 * 60).unwrap();
        let ts = 1772512200000_i64; // 2026-03-03T04:30:00Z
        let date = timestamp_to_date_with_timezone(ts, &pst);
        assert_eq!(date, "2026-03-02");
    }

    #[test]
    fn test_timestamp_to_date_invalid_timestamp() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let date = timestamp_to_date_with_timezone(i64::MAX, &utc);
        assert_eq!(date, "");
    }

    #[test]
    fn test_unified_message_creation() {
        let tokens = TokenBreakdown {
            input: 100,
            output: 50,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        };

        let msg = UnifiedMessage::new(
            "opencode",
            "claude-3-5-sonnet",
            "anthropic",
            "test-session-id",
            1733011200000,
            tokens,
            0.05,
        );

        assert_eq!(msg.client, "opencode");
        assert_eq!(msg.model_id, "claude-3-5-sonnet");
        assert_eq!(msg.session_id, "test-session-id");
        assert_eq!(msg.date, timestamp_to_date(1733011200000));
        assert_eq!(msg.cost, 0.05);
        assert_eq!(msg.agent, None);
        assert_eq!(msg.workspace_key, None);
        assert_eq!(msg.workspace_label, None);
    }

    #[test]
    fn test_normalize_workspace_key_normalizes_slashes_and_trailing_separator() {
        assert_eq!(
            normalize_workspace_key(r"C:\Users\alice\repo\"),
            Some("C:/Users/alice/repo".to_string())
        );
        assert_eq!(
            normalize_workspace_key("/Users/alice//repo/"),
            Some("/Users/alice/repo".to_string())
        );
    }

    #[test]
    fn test_normalize_workspace_key_preserves_unc_prefix() {
        assert_eq!(
            normalize_workspace_key(r"\\server\share\repo\"),
            Some("//server/share/repo".to_string())
        );
        assert_eq!(
            normalize_workspace_key("//server//share///repo/"),
            Some("//server/share/repo".to_string())
        );
    }

    #[test]
    fn test_workspace_label_from_key_uses_last_path_segment() {
        assert_eq!(
            workspace_label_from_key("/Users/alice/my-repo"),
            Some("my-repo".to_string())
        );
        assert_eq!(
            workspace_label_from_key("encoded-project-key"),
            Some("encoded-project-key".to_string())
        );
    }

    #[test]
    fn test_normalize_agent_name() {
        assert_eq!(normalize_agent_name("OmO"), "Sisyphus");
        assert_eq!(normalize_agent_name("Sisyphus"), "Sisyphus");
        assert_eq!(normalize_agent_name("omo"), "Sisyphus");
        assert_eq!(normalize_agent_name("sisyphus"), "Sisyphus");
        assert_eq!(
            normalize_agent_name("Sisyphus (Ultraworker)"),
            "Sisyphus (Ultraworker)"
        );

        assert_eq!(
            normalize_opencode_agent_name("Sisyphus (Ultraworker)"),
            "Sisyphus"
        );
        assert_eq!(normalize_opencode_agent_name("hephaestus"), "Hephaestus");
        assert_eq!(normalize_opencode_agent_name("prometheus"), "Prometheus");
        assert_eq!(normalize_opencode_agent_name("atlas"), "Atlas");
        assert_eq!(normalize_opencode_agent_name("metis"), "Metis");
        assert_eq!(normalize_opencode_agent_name("momus"), "Momus");
        assert_eq!(
            normalize_opencode_agent_name("sisyphus-junior"),
            "Sisyphus-Junior"
        );
        assert_eq!(
            normalize_opencode_agent_name("planner-sisyphus"),
            "Planner-Sisyphus"
        );

        assert_eq!(
            normalize_opencode_agent_name("Hephaestus (Deep Agent)"),
            "Hephaestus"
        );
        assert_eq!(
            normalize_opencode_agent_name("Prometheus (Plan Builder)"),
            "Prometheus"
        );
        assert_eq!(
            normalize_opencode_agent_name("Prometheus (Planner)"),
            "Prometheus"
        );
        assert_eq!(
            normalize_opencode_agent_name("Atlas (Plan Executor)"),
            "Atlas"
        );
        assert_eq!(
            normalize_opencode_agent_name("Metis (Plan Consultant)"),
            "Metis"
        );
        assert_eq!(
            normalize_opencode_agent_name("Momus (Plan Critic)"),
            "Momus"
        );
        assert_eq!(
            normalize_opencode_agent_name("Momus (Plan Reviewer)"),
            "Momus"
        );

        assert_eq!(normalize_agent_name("OmO-Plan"), "Planner-Sisyphus");
        assert_eq!(normalize_agent_name("Planner-Sisyphus"), "Planner-Sisyphus");
        assert_eq!(normalize_agent_name("omo-plan"), "Planner-Sisyphus");

        assert_eq!(normalize_agent_name("orchestrator-sisyphus"), "Atlas");
        assert_eq!(
            normalize_opencode_agent_name("orchestrator-sisyphus"),
            "Atlas"
        );
        assert_eq!(normalize_agent_name("explore"), "Explore");
        assert_eq!(normalize_agent_name("CustomAgent"), "CustomAgent");

        assert_eq!(normalize_agent_name("executor"), "Executor");
        assert_eq!(
            normalize_agent_name("task-orchestrator"),
            "Task Orchestrator"
        );
        assert_eq!(normalize_agent_name("git-committer"), "Git Committer");
        assert_eq!(
            normalize_agent_name("frontend-ui-ux-engineer"),
            "Frontend UI UX Engineer"
        );
        assert_eq!(
            normalize_agent_name("astrape:executor-high"),
            "Executor High"
        );
        assert_eq!(
            normalize_agent_name("oh-my-claudecode:code-reviewer"),
            "Code Reviewer"
        );
    }

    #[test]
    fn test_normalize_copilot_agent_name() {
        assert_eq!(
            normalize_copilot_agent_name("github.copilot.default"),
            "GitHub Copilot"
        );
        assert_eq!(
            normalize_copilot_agent_name("GITHUB.COPILOT.DEFAULT"),
            "GitHub Copilot"
        );
        assert_eq!(normalize_copilot_agent_name("github.copilot.chat"), "Chat");
        assert_eq!(
            normalize_copilot_agent_name("Plugin:software-engineering-team:se-ux-ui-designer"),
            "Software Engineering Team: Se UX UI Designer"
        );
        assert_eq!(
            normalize_copilot_agent_name("plugin:my-team:my-agent"),
            "My Team: My Agent"
        );
        assert_eq!(
            normalize_copilot_agent_name("Plugin:code-review-team:api-reviewer"),
            "Code Review Team: API Reviewer"
        );
        assert_eq!(
            normalize_copilot_agent_name("some-custom-agent"),
            "Some Custom Agent"
        );
        assert_eq!(normalize_agent_name("oh-my-codex:librarian"), "Librarian");
        assert_eq!(normalize_agent_name("astrape:executor"), "Executor");
        assert_eq!(normalize_agent_name("plan-reviewer"), "Plan Reviewer");
        assert_eq!(normalize_agent_name("astrape:planner"), "Planner");

        assert_eq!(
            normalize_opencode_agent_name("astrape:sisyphus"),
            "Sisyphus"
        );
        assert_eq!(
            normalize_opencode_agent_name("oh-my-claudecode:executor"),
            "Executor"
        );

        // New dash format (oh-my-openagent current)
        assert_eq!(
            normalize_opencode_agent_name("Sisyphus - Ultraworker"),
            "Sisyphus"
        );
        assert_eq!(
            normalize_opencode_agent_name("Hephaestus - Deep Agent"),
            "Hephaestus"
        );
        assert_eq!(
            normalize_opencode_agent_name("Prometheus - Plan Builder"),
            "Prometheus"
        );
        assert_eq!(
            normalize_opencode_agent_name("Atlas - Plan Executor"),
            "Atlas"
        );
        assert_eq!(
            normalize_opencode_agent_name("Metis - Plan Consultant"),
            "Metis"
        );
        assert_eq!(
            normalize_opencode_agent_name("Momus - Plan Critic"),
            "Momus"
        );

        // ZWSP-prefixed names (oh-my-openagent sort-order prefixes)
        assert_eq!(
            normalize_opencode_agent_name("\u{200B}Sisyphus - Ultraworker"),
            "Sisyphus"
        );
        assert_eq!(
            normalize_opencode_agent_name("\u{200B}\u{200B}\u{200B}Prometheus - Plan Builder"),
            "Prometheus"
        );
        assert_eq!(
            normalize_opencode_agent_name("\u{200B}\u{200B}\u{200B}\u{200B}Atlas - Plan Executor"),
            "Atlas"
        );
        assert_eq!(
            normalize_opencode_agent_name("\u{FEFF}Momus - Plan Critic"),
            "Momus"
        );
        assert_eq!(
            normalize_opencode_agent_name("\u{200B}sisyphus-junior"),
            "Sisyphus-Junior"
        );
        assert_eq!(
            normalize_opencode_agent_name("\u{200B}sisyphus"),
            "Sisyphus"
        );
        assert_eq!(
            normalize_opencode_agent_name("\u{200B}  Sisyphus   -   Ultraworker  "),
            "Sisyphus"
        );
        assert_eq!(
            normalize_opencode_agent_name("\u{200B}\u{200B}\u{200B}   Prometheus    Plan Builder"),
            "Prometheus"
        );
    }

    #[test]
    fn test_strip_zero_width_chars() {
        assert_eq!(strip_zero_width_chars("hello"), "hello");
        assert_eq!(strip_zero_width_chars("\u{200B}hello"), "hello");
        assert_eq!(
            strip_zero_width_chars("\u{200B}\u{200B}\u{200B}hello"),
            "hello"
        );
        assert_eq!(strip_zero_width_chars("\u{FEFF}hello"), "hello");
        assert_eq!(strip_zero_width_chars("\u{200C}hello\u{200D}"), "hello");
        assert_eq!(strip_zero_width_chars(""), "");
        assert_eq!(
            strip_zero_width_chars("no special chars"),
            "no special chars"
        );
    }
}
