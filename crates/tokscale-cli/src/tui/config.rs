use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use ratatui::style::Color;
use serde::Deserialize;

static CONFIG: OnceLock<TokscaleConfig> = OnceLock::new();

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TokscaleConfig {
    #[serde(default)]
    pub colors: ColorsConfig,
    #[serde(default)]
    pub display_names: DisplayNamesConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ColorsConfig {
    #[serde(default)]
    pub providers: HashMap<String, String>,
    #[serde(default, alias = "sources")]
    pub clients: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DisplayNamesConfig {
    #[serde(default)]
    pub providers: HashMap<String, String>,
    #[serde(default, alias = "sources")]
    pub clients: HashMap<String, String>,
}

impl TokscaleConfig {
    fn load_from_path(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|content| toml::from_str(&content).ok())
            .unwrap_or_default()
    }

    pub fn load() -> &'static TokscaleConfig {
        CONFIG.get_or_init(|| {
            // Keep unit tests deterministic at the irreversible OnceLock
            // boundary. Parser and widget-injection tests exercise disk/config
            // behavior through explicit values without reading ambient files.
            #[cfg(test)]
            {
                Self::default()
            }
            #[cfg(not(test))]
            {
                crate::paths::home_dir()
                    .map(|home| home.join(".tokscale"))
                    .as_deref()
                    .map(Self::load_from_path)
                    .unwrap_or_default()
            }
        })
    }

    pub fn get_provider_color(&self, provider: &str) -> Option<Color> {
        self.colors
            .providers
            .get(&provider.to_lowercase())
            .and_then(|hex| parse_hex_color(hex))
    }

    pub fn get_client_color(&self, client: &str) -> Option<Color> {
        self.colors
            .clients
            .get(&client.to_lowercase())
            .and_then(|hex| parse_hex_color(hex))
    }

    pub fn get_provider_display_name(&self, provider: &str) -> Option<&str> {
        self.display_names
            .providers
            .get(&provider.to_lowercase())
            .map(|s| s.as_str())
    }

    pub fn get_client_display_name(&self, client: &str) -> Option<&str> {
        self.display_names
            .clients
            .get(&client.to_lowercase())
            .map(|s| s.as_str())
    }
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    let hex = hex.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_from_path_parses_custom_colors_and_display_names() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("tokscale.toml");
        fs::write(
            &path,
            r##"[colors.providers]
anthropic = "#123456"

[display_names.clients]
claude = "My Claude"
"##,
        )
        .unwrap();

        let config = TokscaleConfig::load_from_path(&path);
        assert_eq!(
            config.get_provider_color("Anthropic"),
            Some(Color::Rgb(18, 52, 86))
        );
        assert_eq!(config.get_client_display_name("CLAUDE"), Some("My Claude"));
    }

    #[test]
    fn load_from_path_uses_defaults_for_missing_file() {
        let temp = TempDir::new().unwrap();
        let config = TokscaleConfig::load_from_path(&temp.path().join("missing.toml"));
        assert!(config.colors.providers.is_empty());
        assert!(config.display_names.clients.is_empty());
    }
}
