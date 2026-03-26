use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStatePaths {
    pub config_file: PathBuf,
    pub config_dir: PathBuf,
    pub app_support_dir: PathBuf,
    pub models_dir: PathBuf,
    pub logs_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupState {
    Ready,
    NeedsSetup,
    NeedsRepair,
}

impl LocalStatePaths {
    pub fn from_config_path(config_path: &Path) -> Result<Self> {
        let config_dir = config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let app_support_dir = config_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| config_dir.clone());
        let models_dir = app_support_dir.join("models");
        let logs_dir = crate::config::logs_dir()?;

        Ok(Self {
            config_file: config_path.to_path_buf(),
            config_dir,
            app_support_dir,
            models_dir,
            logs_dir,
        })
    }

    pub fn resettable_paths(&self) -> [PathBuf; 3] {
        [
            self.config_dir.clone(),
            self.models_dir.clone(),
            self.logs_dir.clone(),
        ]
    }
}

pub fn reset_local_data(paths: &LocalStatePaths) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for path in paths.resettable_paths() {
        if remove_path_if_present(&path)? {
            removed.push(path);
        }
    }
    Ok(removed)
}

fn remove_path_if_present(path: &Path) -> Result<bool> {
    if path.is_file() {
        std::fs::remove_file(path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
        return Ok(true);
    }
    if path.is_dir() {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
        return Ok(true);
    }
    Ok(false)
}

pub fn wait_for_process_exit(pid: u32, timeout: Duration) -> Result<()> {
    let start = std::time::Instant::now();
    while process_exists(pid) {
        if start.elapsed() >= timeout {
            anyhow::bail!("timed out waiting for process {pid} to exit");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    let status = unsafe { libc::kill(pid as i32, 0) };
    status == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn process_exists(pid: u32) -> bool {
    let _ = pid;
    false
}
