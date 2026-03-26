use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::schema::JabberwokConfig;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelEntry {
    pub name: String,
    pub url: String,
    pub sha256: Option<String>,
    /// Set after the model is downloaded; absent for catalog-only entries.
    pub path: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct ModelsConfig {
    pub default: Option<String>,
    #[serde(rename = "model", default)]
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ModelConfig {
    pub default: Option<String>,
    #[serde(rename = "model", default)]
    pub models: Vec<ModelEntry>,
}

impl From<ModelsConfig> for ModelConfig {
    fn from(config: ModelsConfig) -> Self {
        Self {
            default: config.default,
            models: config.models,
        }
    }
}

impl From<ModelConfig> for ModelsConfig {
    fn from(config: ModelConfig) -> Self {
        Self {
            default: config.default,
            models: config.models,
        }
    }
}

impl ModelConfig {
    pub fn load(config_path: &Path) -> Result<Self> {
        let config = JabberwokConfig::load(config_path)?;
        Ok(config.models.into())
    }

    pub fn save(&self, config_path: &Path) -> Result<()> {
        let mut config = JabberwokConfig::load(config_path)?;
        config.models = ModelsConfig {
            default: self.default.clone(),
            models: self.models.clone(),
        };
        config.save(config_path)
    }

    pub fn get(&self, name: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.name == name)
    }

    pub(crate) fn get_mut(&mut self, name: &str) -> Option<&mut ModelEntry> {
        self.models.iter_mut().find(|m| m.name == name)
    }

    /// Return the path of the default model, if one is installed.
    pub fn default_model_path(&self) -> Option<&Path> {
        let name = self.default.as_deref()?;
        self.get(name)?.path.as_deref()
    }
}

pub fn catalog_entry(config_path: &Path, name: &str) -> anyhow::Result<ModelEntry> {
    let config = ModelConfig::load(config_path)?;
    config.get(name).cloned().ok_or_else(|| {
        let available: Vec<_> = config.models.iter().map(|m| m.name.as_str()).collect();
        anyhow::anyhow!(
            "unknown model {:?}; available: {}",
            name,
            available.join(", ")
        )
    })
}

pub fn is_tar_gz(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or(url);
    path.ends_with(".tar.gz") || path.ends_with(".tgz")
}

#[allow(dead_code)]
pub fn name_from_url(url: &str) -> Result<String> {
    let path = url.split('?').next().unwrap_or(url);
    let file = path
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .with_context(|| "could not derive a name from the URL; use --name to provide one")?;
    let name = file
        .strip_suffix(".tar.gz")
        .or_else(|| file.strip_suffix(".tgz"))
        .unwrap_or(file);
    Ok(name.to_string())
}
