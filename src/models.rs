use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use reqwest::blocking as reqwest_blocking;
use sha2::{Digest, Sha256};
use tempfile::tempdir_in;

use crate::config::{ModelConfig, ModelEntry, catalog_entry, is_tar_gz};

pub fn download_model_with_progress_and_phase<F, P>(
    config_path: &Path,
    models_dir: &Path,
    name: &str,
    progress: F,
    phase: P,
) -> anyhow::Result<PathBuf>
where
    F: Fn(u64, u64) + Send + Sync,
    P: Fn(&'static str) + Send + Sync,
{
    download_model_impl(config_path, models_dir, name, progress, phase)
}

pub fn download_model_with_cli_progress(
    config_path: &Path,
    models_dir: &Path,
    name: &str,
) -> anyhow::Result<PathBuf> {
    let progress = Arc::new(Mutex::new(CliDownloadProgress::new(name, models_dir)));
    let progress_for_bytes = Arc::clone(&progress);
    let progress_for_phase = Arc::clone(&progress);

    download_model_with_progress_and_phase(
        config_path,
        models_dir,
        name,
        move |downloaded, total| {
            if let Ok(mut reporter) = progress_for_bytes.lock() {
                reporter.on_progress(downloaded, total);
            }
        },
        move |phase| {
            if let Ok(mut reporter) = progress_for_phase.lock() {
                reporter.on_phase(phase);
            }
        },
    )
}

fn download_model_impl<F, P>(
    config_path: &Path,
    models_dir: &Path,
    name: &str,
    progress: F,
    phase: P,
) -> anyhow::Result<PathBuf>
where
    F: Fn(u64, u64) + Send + Sync,
    P: Fn(&'static str) + Send + Sync,
{
    if name.is_empty() {
        anyhow::bail!("model name must not be empty");
    }

    let entry = catalog_entry(config_path, name)?;

    std::fs::create_dir_all(models_dir)
        .with_context(|| format!("failed to create {}", models_dir.display()))?;

    tracing::info!(url = entry.url, name, "downloading model");
    phase("Downloading model...");
    progress(0, 0);

    let mut response =
        reqwest_blocking::get(&entry.url).with_context(|| format!("GET {} failed", entry.url))?;

    if !response.status().is_success() {
        anyhow::bail!("server returned {}", response.status());
    }

    let total = response.content_length().unwrap_or(0);
    let progress_log_step = if total > 0 { total.div_ceil(7) } else { 0 };
    let token = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = models_dir.join(format!(".{name}.{token}.download.tmp"));

    let result: anyhow::Result<PathBuf> = (|| {
        let mut tmp_file = std::fs::File::create(&tmp_path)
            .with_context(|| format!("failed to create {}", tmp_path.display()))?;

        let mut downloaded: u64 = 0;
        let mut next_progress_log = progress_log_step;
        let mut buf = vec![0u8; 64 * 1024];
        let mut hasher = Sha256::new();
        loop {
            let n = response
                .read(&mut buf)
                .context("error reading response body")?;
            if n == 0 {
                break;
            }
            tmp_file
                .write_all(&buf[..n])
                .context("error writing temp file")?;
            hasher.update(&buf[..n]);
            downloaded += n as u64;
            progress(downloaded, total);

            if total > 0 && downloaded >= next_progress_log {
                tracing::debug!(
                    downloaded_bytes = downloaded,
                    total_bytes = total,
                    downloaded_mib = format_args!("{:.1}", bytes_to_mib(downloaded)),
                    total_mib = format_args!("{:.1}", bytes_to_mib(total)),
                    percent = format_args!("{:.1}", downloaded as f64 * 100.0 / total as f64),
                    "model download progress"
                );
                next_progress_log = next_progress_log.saturating_add(progress_log_step);
            }
        }

        let digest = hasher.finalize();
        let actual = digest.iter().map(|b| format!("{:02x}", b)).collect::<String>();
        if let Some(expected) = entry.sha256.as_deref()
            && expected.trim() != actual
        {
            anyhow::bail!("sha256 mismatch for {name}; expected {expected}, got {actual}");
        }

        if is_tar_gz(&entry.url) {
            phase("Extracting model...");
            Ok(extract_archive_atomic(&tmp_path, models_dir, name)?)
        } else {
            let file_dest = models_dir.join(name);
            if file_dest.exists() {
                std::fs::remove_file(&file_dest).with_context(|| {
                    format!("failed to replace existing {}", file_dest.display())
                })?;
            }
            std::fs::rename(&tmp_path, &file_dest)
                .with_context(|| format!("failed to move download to {}", file_dest.display()))?;
            Ok(file_dest)
        }
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
        progress(0, 0);
    }

    let dest = result?;

    tracing::info!(dest = %dest.display(), "model ready");

    phase("Finishing setup...");
    let mut config = ModelConfig::load(config_path)?;
    if let Some(m) = config.get_mut(name) {
        m.path = Some(dest.clone());
    } else {
        config.models.push(ModelEntry {
            name: name.to_string(),
            url: entry.url.clone(),
            sha256: entry.sha256.clone(),
            path: Some(dest.clone()),
        });
    }
    config.default = Some(name.to_string());
    config.save(config_path)?;

    Ok(dest)
}

fn bytes_to_mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

const CLI_DOWNLOAD_PROGRESS_STEP_BYTES: u64 = 256 * 1024 * 1024;

struct CliDownloadProgress {
    model: String,
    models_dir: PathBuf,
    last_phase: Option<&'static str>,
    next_report_at: u64,
    last_reported_bytes: u64,
}

impl CliDownloadProgress {
    fn new(model: &str, models_dir: &Path) -> Self {
        Self {
            model: model.to_string(),
            models_dir: models_dir.to_path_buf(),
            last_phase: None,
            next_report_at: 0,
            last_reported_bytes: 0,
        }
    }

    fn on_phase(&mut self, phase: &'static str) {
        if self.last_phase == Some(phase) {
            return;
        }

        match phase {
            "Downloading model..." => {
                println!(
                    "Downloading model `{}` into {}",
                    self.model,
                    self.models_dir.display()
                );
            }
            "Extracting model..." => println!("Extracting model archive..."),
            "Finishing setup..." => println!("Saving model configuration..."),
            other => println!("{other}"),
        }

        self.last_phase = Some(phase);
    }

    fn on_progress(&mut self, downloaded: u64, total: u64) {
        if total == 0 {
            if downloaded == 0 {
                self.next_report_at = CLI_DOWNLOAD_PROGRESS_STEP_BYTES;
                self.last_reported_bytes = 0;
                return;
            }
            if downloaded >= self.next_report_at {
                println!("Download progress: {} downloaded", human_bytes(downloaded));
                self.last_reported_bytes = downloaded;
                self.next_report_at = downloaded.saturating_add(CLI_DOWNLOAD_PROGRESS_STEP_BYTES);
            }
            return;
        }

        if downloaded == 0 {
            let step = download_progress_step(total);
            self.next_report_at = step;
            self.last_reported_bytes = 0;
            println!("Download size: {}", human_bytes(total));
            return;
        }

        if downloaded >= self.next_report_at || downloaded == total {
            if downloaded != self.last_reported_bytes {
                let pct = downloaded as f64 * 100.0 / total as f64;
                println!(
                    "Download progress: {} / {} ({pct:.0}%)",
                    human_bytes(downloaded),
                    human_bytes(total),
                );
                self.last_reported_bytes = downloaded;
            }

            let step = download_progress_step(total);
            while self.next_report_at <= downloaded {
                self.next_report_at = self.next_report_at.saturating_add(step);
            }
        }
    }
}

fn download_progress_step(total: u64) -> u64 {
    if total == 0 {
        CLI_DOWNLOAD_PROGRESS_STEP_BYTES
    } else {
        total.div_ceil(5).max(CLI_DOWNLOAD_PROGRESS_STEP_BYTES)
    }
}

fn human_bytes(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    }
}

fn extract_archive_atomic(
    downloaded: &Path,
    models_dir: &Path,
    name: &str,
) -> anyhow::Result<PathBuf> {
    tracing::info!(archive = %downloaded.display(), model = name, "extracting model archive atomically");

    let temp_dir = tempdir_in(models_dir).with_context(|| {
        format!(
            "failed to allocate temp extraction dir in {}",
            models_dir.display()
        )
    })?;
    let extract_root = temp_dir.path().join(format!("{name}.extract"));
    std::fs::create_dir_all(&extract_root)
        .with_context(|| format!("failed to create {}", extract_root.display()))?;

    let tmp_file = std::fs::File::open(downloaded)
        .with_context(|| format!("failed to open {}", downloaded.display()))?;
    let gz = flate2::read::GzDecoder::new(tmp_file);
    let mut archive = tar::Archive::new(gz);

    for entry in archive
        .entries()
        .context("failed to read archive entries")?
    {
        let mut entry = entry.context("failed to read archive entry")?;
        let entry_path = entry.path().context("archive entry has no path")?;
        let stripped: PathBuf = entry_path.components().skip(1).collect();
        if stripped.as_os_str().is_empty() {
            continue;
        }
        entry
            .unpack(extract_root.join(&stripped))
            .with_context(|| format!("failed to extract {}", stripped.display()))?;
    }

    let final_dir = models_dir.join(name);
    if final_dir.exists() {
        if final_dir.is_dir() {
            std::fs::remove_dir_all(&final_dir)
                .with_context(|| format!("failed to remove existing {}", final_dir.display()))?;
        } else {
            std::fs::remove_file(&final_dir)
                .with_context(|| format!("failed to remove existing {}", final_dir.display()))?;
        }
    }

    std::fs::rename(&extract_root, &final_dir).with_context(|| {
        format!(
            "failed to finalize extracted model at {}",
            final_dir.display()
        )
    })?;
    drop(temp_dir);
    std::fs::remove_file(downloaded).ok();

    Ok(final_dir)
}

#[cfg(test)]
mod tests {
    use crate::config::{ModelConfig, ModelEntry, catalog_entry, is_tar_gz, name_from_url};

    use super::{
        CLI_DOWNLOAD_PROGRESS_STEP_BYTES, download_model_with_progress_and_phase,
        download_progress_step,
    };

    #[test]
    fn name_from_plain_url() {
        assert_eq!(
            name_from_url("https://example.com/ggml-base.en.bin").unwrap(),
            "ggml-base.en.bin"
        );
    }

    #[test]
    fn name_from_tar_gz_url_strips_suffix() {
        assert_eq!(
            name_from_url("https://blob.handy.computer/parakeet-v3-int8.tar.gz").unwrap(),
            "parakeet-v3-int8"
        );
    }

    #[test]
    fn name_from_tgz_url_strips_suffix() {
        assert_eq!(
            name_from_url("https://example.com/model.tgz").unwrap(),
            "model"
        );
    }

    #[test]
    fn name_from_url_strips_query_string() {
        assert_eq!(
            name_from_url("https://example.com/model.bin?token=abc").unwrap(),
            "model.bin"
        );
    }

    #[test]
    fn is_tar_gz_detects_tar_gz() {
        assert!(is_tar_gz("https://example.com/model.tar.gz"));
        assert!(is_tar_gz("https://example.com/model.tgz"));
        assert!(!is_tar_gz("https://example.com/model.bin"));
        assert!(
            is_tar_gz("https://example.com/model.tar.gz?foo=bar"),
            "query string does not affect detection"
        );
    }

    #[test]
    fn model_config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("jabberwok.toml");

        let mut config = ModelConfig::default();
        config.models.push(ModelEntry {
            name: "parakeet-v3".to_string(),
            url: "https://blob.handy.computer/parakeet-v3-int8.tar.gz".to_string(),
            sha256: Some("abc".to_string()),
            path: Some(std::path::PathBuf::from("models/parakeet-v3")),
        });
        config.default = Some("parakeet-v3".to_string());
        config.save(&config_path).unwrap();

        let loaded = ModelConfig::load(&config_path).unwrap();
        assert_eq!(loaded.default.as_deref(), Some("parakeet-v3"));
        assert_eq!(loaded.models.len(), 1);
        assert_eq!(loaded.models[0].name, "parakeet-v3");
        assert_eq!(
            loaded.models[0].path.as_deref(),
            Some(std::path::Path::new("models/parakeet-v3"))
        );
        assert_eq!(loaded.models[0].sha256.as_deref(), Some("abc"));
    }

    #[test]
    fn model_config_load_missing_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let config = ModelConfig::load(&dir.path().join("jabberwok.toml")).unwrap();
        assert!(config.default.is_none());
        assert!(config.models.is_empty());
    }

    #[test]
    fn default_model_path_returns_none_when_empty() {
        let config = ModelConfig::default();
        assert!(config.default_model_path().is_none());
    }

    #[test]
    fn default_model_path_returns_none_when_not_installed() {
        let mut config = ModelConfig::default();
        config.models.push(ModelEntry {
            name: "parakeet-v3".to_string(),
            url: "https://example.com/model.tar.gz".to_string(),
            sha256: Some("abc".to_string()),
            path: None,
        });
        config.default = Some("parakeet-v3".to_string());
        assert!(config.default_model_path().is_none());
    }

    #[test]
    fn default_model_path_returns_some_when_installed() {
        let mut config = ModelConfig::default();
        let path = std::path::PathBuf::from("/models/test");
        config.models.push(ModelEntry {
            name: "test".to_string(),
            url: "https://example.com/model.bin".to_string(),
            sha256: Some("abc".to_string()),
            path: Some(path.clone()),
        });
        config.default = Some("test".to_string());
        assert_eq!(config.default_model_path(), Some(path.as_path()));
    }

    #[test]
    fn get_finds_model_by_name() {
        let mut config = ModelConfig::default();
        config.models.push(ModelEntry {
            name: "alpha".to_string(),
            url: "https://example.com/a.bin".to_string(),
            sha256: None,
            path: None,
        });
        assert!(config.get("alpha").is_some());
        assert!(config.get("beta").is_none());
    }

    #[test]
    fn model_config_save_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("nested").join("dir").join("jabberwok.toml");
        ModelConfig::default().save(&config_path).unwrap();
        assert!(config_path.exists());
    }

    #[test]
    fn model_config_load_invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("jabberwok.toml");
        std::fs::write(&config_path, "[[[ not valid toml").unwrap();
        assert!(ModelConfig::load(&config_path).is_err());
    }

    #[test]
    fn catalog_entry_found() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("jabberwok.toml");
        let mut config = ModelConfig::default();
        config.models.push(ModelEntry {
            name: "test-model".to_string(),
            url: "https://example.com/model.bin".to_string(),
            sha256: Some("abc".to_string()),
            path: None,
        });
        config.save(&config_path).unwrap();
        let entry = catalog_entry(&config_path, "test-model").unwrap();
        assert_eq!(entry.name, "test-model");
        assert_eq!(entry.url, "https://example.com/model.bin");
    }

    #[test]
    fn catalog_entry_not_found_lists_available() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("jabberwok.toml");
        let mut config = ModelConfig::default();
        config.models.push(ModelEntry {
            name: "model-a".to_string(),
            url: "https://example.com/a.bin".to_string(),
            sha256: Some("abc".to_string()),
            path: None,
        });
        config.save(&config_path).unwrap();
        let err = catalog_entry(&config_path, "nonexistent").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown model"), "message was: {msg}");
        assert!(msg.contains("model-a"), "should list available: {msg}");
    }

    #[test]
    fn download_model_unknown_name_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("jabberwok.toml");
        ModelConfig::default().save(&config_path).unwrap();
        let err = download_model_with_progress_and_phase(
            &config_path,
            dir.path(),
            "no-such-model",
            |_, _| {},
            |_| {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown model"));
    }

    #[test]
    fn download_progress_step_targets_about_five_updates_for_large_downloads() {
        let step = download_progress_step(2 * 1024 * 1024 * 1024);
        assert_eq!(step, (2 * 1024 * 1024 * 1024_u64).div_ceil(5));
    }

    #[test]
    fn download_progress_step_uses_hundreds_of_megabytes_for_smaller_downloads() {
        let step = download_progress_step(600 * 1024 * 1024);
        assert_eq!(step, CLI_DOWNLOAD_PROGRESS_STEP_BYTES);
    }
}
