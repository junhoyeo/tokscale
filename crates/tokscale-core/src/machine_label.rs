//! Machine labels for session logs mirrored from other machines.
//!
//! A user can copy another machine's session folders into a local directory
//! and add that directory through `scanner.extraScanPaths`. Without a label,
//! every row then looks like it came from this machine. The
//! `scanner.machineLabels` setting maps such a folder to a machine name; every
//! session file under that folder carries the name on its messages, and the
//! `machine,model` grouping splits usage by it. Files under no labeled folder
//! carry `scanner.localMachineLabel` (default [`DEFAULT_LOCAL_MACHINE_LABEL`]).
//!
//! Matching is a path-prefix match on whole components. Both the folder and
//! the file are compared as written (with `~` expanded) and, when they exist,
//! in their canonical form, so a symlinked or differently spelled path still
//! matches. Separators are normalized, and the comparison ignores case on
//! Windows. When several folders contain a file, the longest one wins.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::sessions::UnifiedMessage;

/// Label for messages whose source file is under no `machineLabels` folder,
/// when `localMachineLabel` is not set.
pub const DEFAULT_LOCAL_MACHINE_LABEL: &str = "local";

/// Resolves the machine label of a session file from the configured folders.
#[derive(Debug, Clone)]
pub struct MachineLabeler {
    /// `(normalized folder, label)`, longest folder first so the first match
    /// is the longest matching prefix.
    rules: Vec<(String, String)>,
    local: String,
    /// Whether `local` came from `localMachineLabel` rather than the default.
    local_configured: bool,
    case_insensitive: bool,
}

impl Default for MachineLabeler {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            local: DEFAULT_LOCAL_MACHINE_LABEL.to_string(),
            local_configured: false,
            case_insensitive: cfg!(windows),
        }
    }
}

impl MachineLabeler {
    /// Build a labeler from `machineLabels` and `localMachineLabel`.
    ///
    /// `home_dir` expands a leading `~` in a folder. Entries with an empty
    /// folder or label are ignored, and an empty local label falls back to
    /// [`DEFAULT_LOCAL_MACHINE_LABEL`].
    pub fn new(
        machine_labels: &BTreeMap<String, String>,
        local_machine_label: Option<&str>,
        home_dir: Option<&str>,
    ) -> Self {
        Self::with_case_sensitivity(machine_labels, local_machine_label, home_dir, cfg!(windows))
    }

    /// Build a labeler from the scanner settings.
    pub fn from_scanner_settings(
        settings: &crate::scanner::ScannerSettings,
        home_dir: Option<&str>,
    ) -> Self {
        Self::new(
            &settings.machine_labels,
            settings.local_machine_label.as_deref(),
            home_dir,
        )
    }

    fn with_case_sensitivity(
        machine_labels: &BTreeMap<String, String>,
        local_machine_label: Option<&str>,
        home_dir: Option<&str>,
        case_insensitive: bool,
    ) -> Self {
        let configured_local = local_machine_label
            .map(str::trim)
            .filter(|label| !label.is_empty());
        let local_configured = configured_local.is_some();
        let local = configured_local
            .unwrap_or(DEFAULT_LOCAL_MACHINE_LABEL)
            .to_string();

        let mut rules: Vec<(String, String)> = Vec::new();
        for (folder, label) in machine_labels {
            let folder = folder.trim();
            let label = label.trim();
            if folder.is_empty() || label.is_empty() {
                continue;
            }
            let expanded = expand_home(folder, home_dir);
            let mut forms = vec![normalize_path_text(&expanded, case_insensitive)];
            if let Ok(canonical) = std::fs::canonicalize(&expanded) {
                forms.push(normalize_path_text(
                    &canonical.to_string_lossy(),
                    case_insensitive,
                ));
            }
            for form in forms {
                if !form.is_empty() && !rules.iter().any(|(existing, _)| *existing == form) {
                    rules.push((form, label.to_string()));
                }
            }
        }
        // Longest folder first; ties keep the settings order (BTreeMap order),
        // which keeps the result deterministic.
        rules.sort_by_key(|(folder, _)| std::cmp::Reverse(folder.len()));

        Self {
            rules,
            local,
            local_configured,
            case_insensitive,
        }
    }

    /// The label for files under no configured folder.
    pub fn local_label(&self) -> &str {
        &self.local
    }

    /// The local label when `localMachineLabel` sets one, else `None`.
    pub fn configured_local_label(&self) -> Option<&str> {
        self.local_configured.then_some(self.local.as_str())
    }

    /// Whether any folder is configured. Without one, every file is local and
    /// no path needs to be inspected.
    pub fn has_rules(&self) -> bool {
        !self.rules.is_empty()
    }

    /// The label of a configured folder containing `path`, longest folder
    /// first, or `None` when no configured folder contains it.
    pub fn configured_label_for(&self, path: &Path) -> Option<&str> {
        if self.rules.is_empty() {
            return None;
        }
        let mut candidates = vec![normalize_path_text(
            &path.to_string_lossy(),
            self.case_insensitive,
        )];
        if let Ok(canonical) = std::fs::canonicalize(path) {
            let canonical =
                normalize_path_text(&canonical.to_string_lossy(), self.case_insensitive);
            if canonical != candidates[0] {
                candidates.push(canonical);
            }
        }
        self.rules
            .iter()
            .find(|(folder, _)| {
                candidates
                    .iter()
                    .any(|candidate| is_path_prefix(folder, candidate))
            })
            .map(|(_, label)| label.as_str())
    }

    /// The machine label of `path`: a configured folder's label, else the local
    /// label.
    pub fn label_for(&self, path: &Path) -> &str {
        self.configured_label_for(path).unwrap_or(&self.local)
    }

    /// Label every message read from `path` that a configured folder contains.
    ///
    /// Messages from other files are left unlabeled here; the parse tail gives
    /// them the configured local label, and an unlabeled message reads as the
    /// default local label.
    pub(crate) fn label_messages(&self, path: &Path, messages: &mut [UnifiedMessage]) {
        if let Some(label) = self.configured_label_for(path) {
            for message in messages {
                message.machine_label = Some(label.to_string());
            }
        }
    }
}

fn expand_home(value: &str, home_dir: Option<&str>) -> String {
    let Some(home) = home_dir else {
        return value.to_string();
    };
    if value == "~" {
        return home.to_string();
    }
    match value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        Some(relative) => format!("{}/{}", home.trim_end_matches(['/', '\\']), relative),
        None => value.to_string(),
    }
}

/// Normalize a path's text for prefix comparison: `/` separators, no
/// Windows verbatim prefix, no `.` segments, no repeated or trailing
/// separators, and lowercase when the platform ignores case.
fn normalize_path_text(raw: &str, case_insensitive: bool) -> String {
    let mut text = raw.replace('\\', "/");
    if let Some(rest) = text.strip_prefix("//?/UNC/") {
        text = format!("//{rest}");
    } else if let Some(rest) = text.strip_prefix("//?/") {
        text = rest.to_string();
    }
    let unc = text.starts_with("//");
    let rooted = text.starts_with('/');
    let segments: Vec<&str> = text
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    let mut normalized = String::with_capacity(text.len());
    if unc {
        normalized.push_str("//");
    } else if rooted {
        normalized.push('/');
    }
    normalized.push_str(&segments.join("/"));
    if case_insensitive {
        normalized = normalized.to_lowercase();
    }
    normalized
}

/// Whether `folder` is `path` or one of its ancestors, on whole components.
fn is_path_prefix(folder: &str, path: &str) -> bool {
    if folder == path {
        return true;
    }
    if folder.ends_with('/') {
        return path.starts_with(folder);
    }
    path.strip_prefix(folder)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// Lossy deserializer for `scanner.machineLabels`: a value that is not an
/// object reads as an empty map, and entries whose label is not a string are
/// dropped, so one bad value never discards the rest of the settings file.
pub fn deserialize_machine_labels_lossy<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Object(object) => object
            .into_iter()
            .filter_map(|(folder, label)| match label {
                serde_json::Value::String(label) => Some((folder, label)),
                _ => None,
            })
            .collect(),
        _ => BTreeMap::new(),
    })
}

/// Lossy deserializer for `scanner.localMachineLabel`: anything but a string
/// reads as unset.
pub fn deserialize_local_machine_label_lossy<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::String(label) => Some(label),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn labels(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(folder, label)| (folder.to_string(), label.to_string()))
            .collect()
    }

    #[test]
    fn files_outside_every_folder_get_the_default_local_label() {
        let labeler = MachineLabeler::with_case_sensitivity(
            &labels(&[("/mirror/steven", "steven")]),
            None,
            None,
            false,
        );
        assert_eq!(labeler.local_label(), DEFAULT_LOCAL_MACHINE_LABEL);
        assert_eq!(
            labeler.label_for(Path::new("/home/me/.codex/sessions/a.jsonl")),
            "local"
        );
        assert_eq!(
            labeler.configured_label_for(Path::new("/home/me/.codex/sessions/a.jsonl")),
            None
        );
    }

    #[test]
    fn a_configured_local_label_replaces_the_default() {
        let labeler =
            MachineLabeler::with_case_sensitivity(&BTreeMap::new(), Some(" blade "), None, false);
        assert_eq!(labeler.label_for(Path::new("/any/file.jsonl")), "blade");
        assert_eq!(labeler.configured_local_label(), Some("blade"));
        assert!(!labeler.has_rules());

        let blank =
            MachineLabeler::with_case_sensitivity(&BTreeMap::new(), Some("  "), None, false);
        assert_eq!(blank.local_label(), "local");
        assert_eq!(blank.configured_local_label(), None);
    }

    #[test]
    fn a_file_under_a_folder_gets_its_label() {
        let labeler = MachineLabeler::with_case_sensitivity(
            &labels(&[("/mirror/steven/", "steven")]),
            None,
            None,
            false,
        );
        assert_eq!(
            labeler.label_for(Path::new("/mirror/steven/.codex/sessions/a.jsonl")),
            "steven"
        );
        assert_eq!(labeler.label_for(Path::new("/mirror/steven")), "steven");
    }

    #[test]
    fn a_prefix_matches_whole_components_only() {
        let labeler = MachineLabeler::with_case_sensitivity(
            &labels(&[("/mirror/steven", "steven")]),
            None,
            None,
            false,
        );
        assert_eq!(
            labeler.label_for(Path::new("/mirror/stevenson/a.jsonl")),
            "local"
        );
    }

    #[test]
    fn the_longest_matching_folder_wins() {
        let labeler = MachineLabeler::with_case_sensitivity(
            &labels(&[
                ("/mirror", "mirrors"),
                ("/mirror/steven", "steven"),
                ("/mirror/steven/work", "steven-work"),
            ]),
            None,
            None,
            false,
        );
        assert_eq!(
            labeler.label_for(Path::new("/mirror/steven/work/a.jsonl")),
            "steven-work"
        );
        assert_eq!(
            labeler.label_for(Path::new("/mirror/steven/home/a.jsonl")),
            "steven"
        );
        assert_eq!(
            labeler.label_for(Path::new("/mirror/kev/a.jsonl")),
            "mirrors"
        );
    }

    #[test]
    fn windows_style_paths_match_regardless_of_separator_and_case() {
        let labeler = MachineLabeler::with_case_sensitivity(
            &labels(&[(r"D:\Mirrors\Steven", "steven")]),
            None,
            None,
            true,
        );
        assert_eq!(
            labeler.label_for(Path::new(r"d:\mirrors\steven\.codex\sessions\a.jsonl")),
            "steven"
        );
        assert_eq!(
            labeler.label_for(Path::new("D:/MIRRORS/STEVEN/.claude/projects/x.jsonl")),
            "steven"
        );
        assert_eq!(
            labeler.label_for(Path::new(r"\\?\D:\Mirrors\Steven\a.jsonl")),
            "steven"
        );
        assert_eq!(
            labeler.label_for(Path::new(r"D:\Mirrors\StevenOld\a.jsonl")),
            "local"
        );
    }

    #[test]
    fn case_is_significant_where_the_platform_says_so() {
        let labeler = MachineLabeler::with_case_sensitivity(
            &labels(&[("/Mirror/Steven", "steven")]),
            None,
            None,
            false,
        );
        assert_eq!(
            labeler.label_for(Path::new("/mirror/steven/a.jsonl")),
            "local"
        );
    }

    #[test]
    fn a_tilde_folder_expands_against_home() {
        let labeler = MachineLabeler::with_case_sensitivity(
            &labels(&[("~/mirrors/steven", "steven")]),
            None,
            Some("/home/me"),
            false,
        );
        assert_eq!(
            labeler.label_for(Path::new("/home/me/mirrors/steven/a.jsonl")),
            "steven"
        );
    }

    #[test]
    fn empty_folders_and_labels_are_ignored() {
        let labeler = MachineLabeler::with_case_sensitivity(
            &labels(&[("", "nobody"), ("/mirror/steven", " ")]),
            None,
            None,
            false,
        );
        assert!(!labeler.has_rules());
    }

    #[test]
    fn a_differently_spelled_folder_still_matches() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(real.join("sessions")).unwrap();
        let file = real.join("sessions").join("a.jsonl");
        std::fs::write(&file, "").unwrap();
        // A folder spelled with a `.` segment and a trailing separator still
        // names the same directory.
        let spelled: PathBuf = dir.path().join(".").join("real");
        let folder = format!("{}/", spelled.to_string_lossy());
        let labeler = MachineLabeler::new(&labels(&[(&folder, "steven")]), None, None);
        assert_eq!(labeler.label_for(&file), "steven");
    }

    #[test]
    fn lossy_deserializers_keep_the_valid_parts() {
        #[derive(Deserialize)]
        struct Probe {
            #[serde(default, deserialize_with = "deserialize_machine_labels_lossy")]
            labels: BTreeMap<String, String>,
            #[serde(default, deserialize_with = "deserialize_local_machine_label_lossy")]
            local: Option<String>,
        }
        let probe: Probe =
            serde_json::from_str(r#"{"labels": {"/a": "a", "/b": 5}, "local": ["x"]}"#).unwrap();
        assert_eq!(probe.labels, labels(&[("/a", "a")]));
        assert_eq!(probe.local, None);

        let probe: Probe = serde_json::from_str(r#"{"labels": "oops", "local": "blade"}"#).unwrap();
        assert!(probe.labels.is_empty());
        assert_eq!(probe.local.as_deref(), Some("blade"));
    }
}
