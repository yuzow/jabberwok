use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::models::ModelsConfig;

fn default_logging_level() -> String {
    "warn".to_string()
}

fn default_logging_targets() -> Vec<String> {
    vec!["jabberwok=info".to_string()]
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct DevicePrefs {
    pub input: Option<String>,
    pub output: Option<String>,
}

impl DevicePrefs {
    pub const DEFAULT: DevicePrefs = DevicePrefs {
        input: None,
        output: None,
    };
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct DevicesConfig {
    /// Keys are hostnames; the special key "default" is the fallback.
    #[serde(flatten)]
    pub hosts: HashMap<String, DevicePrefs>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LoggingConfig {
    #[serde(default = "default_logging_level")]
    pub level: String,
    #[serde(default = "default_logging_targets")]
    pub targets: Vec<String>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_logging_level(),
            targets: default_logging_targets(),
        }
    }
}

impl LoggingConfig {
    pub fn filter_spec(&self) -> String {
        let mut directives = Vec::with_capacity(1 + self.targets.len());
        directives.push(self.level.trim().to_string());
        directives.extend(
            self.targets
                .iter()
                .map(|target| target.trim())
                .filter(|target| !target.is_empty())
                .map(ToOwned::to_owned),
        );
        directives.join(",")
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct TutorialConfig {
    #[serde(default)]
    pub has_seen_tutorial: bool,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct JabberwokConfig {
    #[serde(default)]
    pub devices: DevicesConfig,
    #[serde(default)]
    pub models: ModelsConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub tutorial: TutorialConfig,
}

impl JabberwokConfig {
    pub fn load(config_path: &Path) -> Result<Self> {
        tracing::info!(config_path = %config_path.display(), "loading config");
        if !config_path.exists() {
            tracing::debug!("config not found; using defaults");
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let config: Self = toml::from_str(&data)
            .with_context(|| format!("failed to parse {}", config_path.display()))?;
        tracing::debug!(
            model_count = config.models.models.len(),
            default = ?config.models.default,
            device_host_count = config.devices.hosts.len(),
            logging_filter = config.logging.filter_spec(),
            "app config loaded"
        );
        Ok(config)
    }

    pub fn save(&self, config_path: &Path) -> Result<()> {
        tracing::debug!(config_path = %config_path.display(), "saving app config");
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let data = toml::to_string_pretty(self).context("failed to serialize config")?;
        std::fs::write(config_path, data)
            .with_context(|| format!("failed to write {}", config_path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::LoggingConfig;

    #[test]
    fn logging_config_defaults_to_jabberwok_info_and_global_warn() {
        let config = LoggingConfig::default();
        assert_eq!(config.filter_spec(), "warn,jabberwok=info");
    }

    #[test]
    fn logging_filter_spec_ignores_blank_targets() {
        let config = LoggingConfig {
            level: "info".to_string(),
            targets: vec!["jabberwok=debug".to_string(), "   ".to_string()],
        };
        assert_eq!(config.filter_spec(), "info,jabberwok=debug");
    }
}
