use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VariantFile {
    pub(crate) name: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) provider_id: Option<String>,
    pub(crate) config_dir: Option<String>,
}

pub(crate) fn read_variant_file(variant_path: &Path) -> Option<VariantFile> {
    let contents = std::fs::read_to_string(variant_path).ok()?;
    serde_json::from_str(&contents).ok()
}

pub(crate) fn variant_file_path(variant_dir: &Path) -> PathBuf {
    variant_dir.join("variant.json")
}

pub(crate) fn discover_claude_project_roots(home_dir: &Path) -> Vec<PathBuf> {
    let root = home_dir.join(".cc-mirror");
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut roots: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let variant_path = variant_file_path(&entry.path());
            let config_dir = config_dir_from_variant_file(&variant_path, home_dir)?;
            let projects_dir = config_dir.join("projects");
            projects_dir.is_dir().then_some(projects_dir)
        })
        .collect();
    roots.sort_unstable();
    roots
}

pub(crate) fn variant_file_for_session_path(
    path: &Path,
    home_dir: Option<&Path>,
) -> Option<PathBuf> {
    variant_dir_from_session_path(path, home_dir).map(|variant_dir| variant_file_path(&variant_dir))
}

pub(crate) fn variant_dir_from_session_path(
    path: &Path,
    home_dir: Option<&Path>,
) -> Option<PathBuf> {
    default_layout_variant_dir_from_session_path(path)
        .or_else(|| configured_variant_dir_from_session_path(path, home_dir?))
}

fn default_layout_variant_dir_from_session_path(path: &Path) -> Option<PathBuf> {
    for ancestor in path.ancestors() {
        if ancestor.file_name().and_then(|name| name.to_str()) != Some("config") {
            continue;
        }
        if !path.starts_with(ancestor.join("projects")) {
            continue;
        }
        let variant_dir = ancestor.parent()?;
        if variant_dir
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            == Some(".cc-mirror")
        {
            return Some(variant_dir.to_path_buf());
        }
    }

    None
}

fn configured_variant_dir_from_session_path(path: &Path, home_dir: &Path) -> Option<PathBuf> {
    let root = home_dir.join(".cc-mirror");
    let entries = std::fs::read_dir(root).ok()?;
    let normal_claude_projects = home_dir.join(".claude").join("projects");
    let mut candidates: Vec<(usize, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let variant_dir = entry.path();
            if !variant_dir.is_dir() {
                return None;
            }
            let variant_path = variant_file_path(&variant_dir);
            let config_dir = config_dir_from_variant_file(&variant_path, home_dir)?;
            let projects_dir = config_dir.join("projects");

            if projects_dir == normal_claude_projects || !path.starts_with(&projects_dir) {
                return None;
            }

            Some((projects_dir.components().count(), variant_dir))
        })
        .collect();

    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    candidates
        .into_iter()
        .map(|(_, variant_dir)| variant_dir)
        .next()
}

fn config_dir_from_variant_file(variant_path: &Path, home_dir: &Path) -> Option<PathBuf> {
    let variant_dir = variant_path.parent()?;
    let metadata = read_variant_file(variant_path)?;
    metadata
        .config_dir
        .as_deref()
        .and_then(|raw| expand_config_dir(raw, home_dir, variant_dir))
        .or_else(|| Some(variant_dir.join("config")))
}

fn expand_config_dir(raw: &str, home_dir: &Path, variant_dir: &Path) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("~/") {
        return Some(home_dir.join(rest));
    }

    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(variant_dir.join(path))
    }
}
