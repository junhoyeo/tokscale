//! OpenCode-configured model display names.
//!
//! OpenCode lets providers assign a human-readable `name` to a model key in
//! its user-level configuration. These names are presentation metadata:
//! they are applied only while grouping local reports and never replace the
//! model id used for pricing, caching, or exports.

use crate::{canonical_model_id, provider_identity};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Default)]
pub struct OpenCodeModelNameResolver {
    names: HashMap<(String, String), String>,
}

impl OpenCodeModelNameResolver {
    fn from_json(contents: &str) -> Self {
        let Some(config) = parse_jsonc(contents) else {
            return Self::default();
        };

        let Some(providers) = config
            .get("provider")
            .and_then(serde_json::Value::as_object)
        else {
            return Self::default();
        };

        let mut names = HashMap::new();
        for (provider, provider_config) in providers {
            let Some(models) = provider_config
                .get("models")
                .and_then(serde_json::Value::as_object)
            else {
                continue;
            };

            let provider = provider_identity::canonical_provider(provider)
                .unwrap_or_else(|| provider.to_string());
            for (model_id, model_config) in models {
                let Some(name) = model_config
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                else {
                    continue;
                };

                let name = name.to_string();
                names
                    .entry((provider.clone(), canonical_model_id(model_id)))
                    .or_insert(name);
            }
        }

        Self { names }
    }

    fn extend_from_json(&mut self, contents: &str) {
        self.names.extend(Self::from_json(contents).names);
    }

    pub fn display_name(&self, provider_id: &str, model_id: &str) -> Option<&str> {
        let provider = provider_identity::canonical_provider(provider_id)
            .unwrap_or_else(|| provider_id.to_string());
        self.names
            .get(&(provider, canonical_model_id(model_id)))
            .map(String::as_str)
    }
}

fn parse_jsonc(contents: &str) -> Option<serde_json::Value> {
    serde_json::from_str(contents)
        .ok()
        .or_else(|| serde_json::from_str(&strip_jsonc_syntax(contents)).ok())
}
fn strip_jsonc_syntax(contents: &str) -> String {
    let chars: Vec<char> = contents.chars().collect();
    let mut without_comments = String::with_capacity(contents.len());
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;

    while index < chars.len() {
        let ch = chars[index];
        if in_string {
            without_comments.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
            without_comments.push(ch);
            index += 1;
        } else if ch == '/' && chars.get(index + 1) == Some(&'/') {
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
        } else if ch == '/' && chars.get(index + 1) == Some(&'*') {
            without_comments.push(' ');
            index += 2;
            while index + 1 < chars.len() && !(chars[index] == '*' && chars[index + 1] == '/') {
                index += 1;
            }
            index = (index + 2).min(chars.len());
        } else {
            without_comments.push(ch);
            index += 1;
        }
    }

    let chars: Vec<char> = without_comments.chars().collect();
    let mut out = String::with_capacity(without_comments.len());
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;

    while index < chars.len() {
        let ch = chars[index];
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
            out.push(ch);
        } else if ch == ',' {
            let next = chars[index + 1..].iter().find(|c| !c.is_ascii_whitespace());
            if !matches!(next, Some(&'}') | Some(&']')) {
                out.push(ch);
            }
        } else {
            out.push(ch);
        }
        index += 1;
    }

    out
}

fn global_config_dir(home_dir: &Path, xdg_config_home: Option<&Path>) -> std::path::PathBuf {
    if let Some(config_home) = xdg_config_home {
        return config_home.join("opencode");
    }
    home_dir.join(".config/opencode")
}

#[derive(Default)]
struct EnvironmentConfig {
    xdg_config_home: Option<std::path::PathBuf>,
    custom_config: Option<std::path::PathBuf>,
    custom_config_dir: Option<std::path::PathBuf>,
    inline_config: Option<String>,
}

impl EnvironmentConfig {
    fn from_env() -> Self {
        let path = |name| {
            std::env::var_os(name)
                .filter(|path| !path.is_empty())
                .map(std::path::PathBuf::from)
        };
        Self {
            xdg_config_home: path("XDG_CONFIG_HOME"),
            custom_config: path("OPENCODE_CONFIG"),
            custom_config_dir: path("OPENCODE_CONFIG_DIR"),
            inline_config: std::env::var("OPENCODE_CONFIG_CONTENT").ok(),
        }
    }
}

fn load_with_environment(
    home_dir: &Path,
    environment: Option<&EnvironmentConfig>,
) -> OpenCodeModelNameResolver {
    let mut resolver = OpenCodeModelNameResolver::default();

    let config_dir = global_config_dir(
        home_dir,
        environment.and_then(|config| config.xdg_config_home.as_deref()),
    );
    for filename in ["opencode.json", "opencode.jsonc"] {
        if let Ok(contents) = std::fs::read_to_string(config_dir.join(filename)) {
            resolver.extend_from_json(&contents);
        }
    }

    // Mirror OpenCode's user-level source order. Project, remote, and managed
    // configs are intentionally excluded because reports span many workspaces.
    if let Some(path) = environment.and_then(|config| config.custom_config.as_deref()) {
        if let Ok(contents) = std::fs::read_to_string(path) {
            resolver.extend_from_json(&contents);
        }
    }

    let legacy_dir = home_dir.join(".opencode");
    for filename in ["opencode.json", "opencode.jsonc"] {
        if let Ok(contents) = std::fs::read_to_string(legacy_dir.join(filename)) {
            resolver.extend_from_json(&contents);
        }
    }

    if let Some(environment) = environment {
        if let Some(config_dir) = environment.custom_config_dir.as_deref() {
            for filename in ["opencode.json", "opencode.jsonc"] {
                if let Ok(contents) = std::fs::read_to_string(config_dir.join(filename)) {
                    resolver.extend_from_json(&contents);
                }
            }
        }
        if let Some(contents) = environment.inline_config.as_deref() {
            resolver.extend_from_json(contents);
        }
    }

    resolver
}

/// Load configured model names from OpenCode's user-level configuration.
///
/// Normal runs honor OpenCode's XDG config root, custom config path and
/// directory, inline config, and legacy `~/.opencode` directory. An explicit
/// `--home` only reads paths under that home so reports stay hermetic. Invalid
/// or unreadable config files are ignored so usage reporting remains available.
pub fn load_for_home(home_dir: Option<&Path>) -> OpenCodeModelNameResolver {
    let use_env_roots = home_dir.is_none();
    let Some(home_dir) = home_dir.map(Path::to_path_buf).or_else(dirs::home_dir) else {
        return OpenCodeModelNameResolver::default();
    };

    let environment = use_env_roots.then(EnvironmentConfig::from_env);
    load_with_environment(&home_dir, environment.as_ref())
}

static GLOBAL: OnceLock<OpenCodeModelNameResolver> = OnceLock::new();
static EMPTY: OnceLock<OpenCodeModelNameResolver> = OnceLock::new();

/// Install the process-wide OpenCode name resolver. The first call wins, like
/// the existing model-alias resolver.
pub fn set_global(resolver: OpenCodeModelNameResolver) {
    let _ = GLOBAL.set(resolver);
}

pub(crate) fn global() -> &'static OpenCodeModelNameResolver {
    GLOBAL
        .get()
        .unwrap_or_else(|| EMPTY.get_or_init(OpenCodeModelNameResolver::default))
}

#[cfg(test)]
mod tests {
    use super::{
        global_config_dir, load_for_home, load_with_environment, EnvironmentConfig,
        OpenCodeModelNameResolver,
    };

    fn config_with_name(name: &str) -> String {
        format!(
            r#"{{"provider":{{"fireworks-ai":{{"models":{{"accounts/fireworks/models/glm-5p2":{{"name":"{name}"}}}}}}}}}}"#
        )
    }

    #[test]
    fn uses_configured_name_for_matching_provider_and_model() {
        let resolver = OpenCodeModelNameResolver::from_json(
            r#"{
                "provider": {
                    "fireworks-ai": {
                        "models": {
                            "accounts/fireworks/models/glm-5p2": { "name": "GLM 5.2" }
                        }
                    }
                }
            }"#,
        );

        assert_eq!(
            resolver.display_name("fireworks_ai", "accounts/fireworks/models/glm-5p2"),
            Some("GLM 5.2")
        );
        assert_eq!(
            resolver.display_name("fireworks_ai", "accounts/fireworks/models/glm-5p1"),
            None
        );
    }

    #[test]
    fn skips_missing_or_blank_names_without_rejecting_other_models() {
        let resolver = OpenCodeModelNameResolver::from_json(
            r#"{
                "provider": {
                    "fireworks-ai": {
                        "models": {
                            "missing": {},
                            "blank": { "name": "  " },
                            "named": { "name": "DeepSeek V4 Flash" }
                        }
                    }
                }
            }"#,
        );

        assert_eq!(resolver.display_name("fireworks-ai", "missing"), None);
        assert_eq!(resolver.display_name("fireworks-ai", "blank"), None);
        assert_eq!(
            resolver.display_name("fireworks-ai", "named"),
            Some("DeepSeek V4 Flash")
        );
    }

    #[test]
    fn reads_jsonc_global_config_with_comments_and_trailing_commas() {
        let home = tempfile::TempDir::new().unwrap();
        let config_dir = home.path().join(".config/opencode");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("opencode.jsonc"),
            r#"{
                // Model labels are for display only.
                "provider": {
                    "fireworks-ai": {
                        "models": {
                            "accounts/fireworks/models/glm-5p2": {
                                "name": "GLM 5.2",
                            },
                        },
                    },
                },
            }"#,
        )
        .unwrap();

        let resolver = load_for_home(Some(home.path()));

        assert_eq!(
            resolver.display_name("fireworks", "accounts/fireworks/models/glm-5p2"),
            Some("GLM 5.2")
        );
    }

    #[test]
    fn explicit_home_ignores_environment_config_sources() {
        let home = tempfile::TempDir::new().unwrap();
        let home_config_dir = home.path().join(".config/opencode");
        std::fs::create_dir_all(&home_config_dir).unwrap();
        std::fs::write(
            home_config_dir.join("opencode.json"),
            r#"{"provider":{"fireworks-ai":{"models":{"accounts/fireworks/models/glm-5p2":{"name":"Home Config"}}}}}"#,
        )
        .unwrap();

        let legacy_config_dir = home.path().join(".opencode");
        std::fs::create_dir_all(&legacy_config_dir).unwrap();
        std::fs::write(
            legacy_config_dir.join("opencode.json"),
            r#"{"provider":{"fireworks-ai":{"models":{"accounts/fireworks/models/glm-5p2":{"name":"Home Legacy"}}}}}"#,
        )
        .unwrap();

        let resolver = load_with_environment(home.path(), None);

        assert_eq!(
            resolver.display_name("fireworks", "accounts/fireworks/models/glm-5p2"),
            Some("Home Legacy")
        );
    }

    #[test]
    fn normal_runs_apply_xdg_and_environment_sources_in_opencode_order() {
        let home = tempfile::TempDir::new().unwrap();
        let xdg = tempfile::TempDir::new().unwrap();
        let external = tempfile::TempDir::new().unwrap();
        let xdg_config_dir = xdg.path().join("opencode");
        let legacy_config_dir = home.path().join(".opencode");
        let config_dir = external.path().join("config-dir");
        let custom_config = external.path().join("custom.json");
        std::fs::create_dir_all(&xdg_config_dir).unwrap();
        std::fs::create_dir_all(&legacy_config_dir).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            xdg_config_dir.join("opencode.json"),
            config_with_name("XDG"),
        )
        .unwrap();
        std::fs::write(&custom_config, config_with_name("Custom")).unwrap();
        std::fs::write(
            legacy_config_dir.join("opencode.json"),
            config_with_name("Legacy"),
        )
        .unwrap();
        std::fs::write(
            config_dir.join("opencode.json"),
            config_with_name("Config Dir"),
        )
        .unwrap();

        let environment = EnvironmentConfig {
            xdg_config_home: Some(xdg.path().to_path_buf()),
            custom_config: Some(custom_config),
            custom_config_dir: Some(config_dir),
            inline_config: Some(config_with_name("Inline")),
        };

        assert_eq!(
            global_config_dir(home.path(), environment.xdg_config_home.as_deref()),
            xdg_config_dir
        );
        let resolver = load_with_environment(home.path(), Some(&environment));

        assert_eq!(
            resolver.display_name("fireworks", "accounts/fireworks/models/glm-5p2"),
            Some("Inline")
        );
    }

    #[test]
    fn trailing_comma_with_comment_then_close_brace_is_valid_jsonc() {
        let resolver = OpenCodeModelNameResolver::from_json(
            r#"{
                "provider": {
                    "fireworks-ai": {
                        "models": {
                            "accounts/fireworks/models/glm-5p2": {
                                "name": "GLM 5.2", // model label comment
                            },
                        },
                    },
                },
            }"#,
        );

        assert_eq!(
            resolver.display_name("fireworks", "accounts/fireworks/models/glm-5p2"),
            Some("GLM 5.2")
        );
    }

    #[test]
    fn colliding_canonical_ids_resolve_to_single_name() {
        let resolver = OpenCodeModelNameResolver::from_json(
            r#"{
                "provider": {
                    "fireworks": {
                        "models": {
                            "glm-5p2-20250701": { "name": "Dated" },
                            "glm-5p2": { "name": "Short" }
                        }
                    }
                }
            }"#,
        );

        // Both keys canonicalize to "glm-5p2" (date suffix stripped),
        // so both lookups return the same name.
        let dated = resolver.display_name("fireworks", "glm-5p2-20250701");
        let short = resolver.display_name("fireworks", "glm-5p2");
        assert!(dated.is_some());
        assert_eq!(dated, short);
    }
}
