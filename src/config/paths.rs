use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn prepare_process_args<I, S>(raw_args: I) -> Result<Vec<String>>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = raw_args.into_iter().map(Into::into).collect();

    let exe = std::env::current_exe().context("failed to locate current executable")?;
    tracing::info!(exe = %exe.display(), "executable");

    let cwd = std::env::current_dir().unwrap_or_default();
    tracing::info!(cwd = %cwd.display(), "working directory");

    if let Some(resources_dir) = bundled_resources_dir()? {
        tracing::info!(resources_dir = %resources_dir.display(), "running inside macOS app bundle");
        let app_support_dir = macos_app_support_dir()?;
        tracing::info!(app_support_dir = %app_support_dir.display(), "app support directory");
        bootstrap_packaged_app_defaults(&resources_dir, &app_support_dir)?;
        std::env::set_current_dir(&app_support_dir)
            .with_context(|| format!("failed to switch to {}", app_support_dir.display()))?;
        tracing::info!(cwd = %app_support_dir.display(), "working directory set to app support");
    } else {
        #[cfg(target_os = "macos")]
        {
            let app_support_dir = macos_app_support_dir()?;
            tracing::info!(app_support_dir = %app_support_dir.display(), "app support directory");
            bootstrap_unbundled_macos_defaults(&cwd, &app_support_dir)?;
        }
        tracing::info!(cwd = %cwd.display(), "not in app bundle; working directory unchanged");
    }

    Ok(args)
}

fn bootstrap_packaged_app_defaults(resources_dir: &Path, app_support_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(app_support_dir)
        .with_context(|| format!("failed to create {}", app_support_dir.display()))?;

    let defaults_dir = resources_dir.join("defaults");
    if !defaults_dir.is_dir() {
        tracing::debug!(defaults_dir = %defaults_dir.display(), "no defaults directory; skipping bootstrap");
        return Ok(());
    }

    tracing::debug!(defaults_dir = %defaults_dir.display(), "bootstrapping app defaults");
    copy_missing_tree(
        &defaults_dir.join("config"),
        &app_support_dir.join("config"),
    )?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn bootstrap_unbundled_macos_defaults(cwd: &Path, app_support_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(app_support_dir)
        .with_context(|| format!("failed to create {}", app_support_dir.display()))?;

    tracing::debug!(cwd = %cwd.display(), app_support_dir = %app_support_dir.display(), "bootstrapping unbundled macOS defaults");
    copy_missing_tree(&cwd.join("config"), &app_support_dir.join("config"))?;

    Ok(())
}

fn copy_missing_tree(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        tracing::debug!(src = %src.display(), "source does not exist; skipping");
        return Ok(());
    }

    if src.is_file() {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        if !dst.exists() {
            tracing::debug!(src = %src.display(), dst = %dst.display(), "copying default file");
            std::fs::copy(src, dst).with_context(|| {
                format!("failed to copy {} to {}", src.display(), dst.display())
            })?;
        } else {
            tracing::debug!(dst = %dst.display(), "file already exists; skipping");
        }
        return Ok(());
    }

    std::fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;

    for entry in
        std::fs::read_dir(src).with_context(|| format!("failed to read {}", src.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", src.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        if is_ignored_packaged_file(&name) {
            continue;
        }
        copy_missing_tree(&path, &dst.join(name))?;
    }

    Ok(())
}

fn is_ignored_packaged_file(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with("._")
}

pub fn bundled_resources_dir() -> Result<Option<PathBuf>> {
    let exe = std::env::current_exe().context("failed to locate current executable")?;
    let Some(macos_dir) = exe.parent() else {
        return Ok(None);
    };
    let Some(contents_dir) = macos_dir.parent() else {
        return Ok(None);
    };
    let Some(app_dir) = contents_dir.parent() else {
        return Ok(None);
    };

    if macos_dir.file_name() != Some(OsStr::new("MacOS")) {
        return Ok(None);
    }
    if contents_dir.file_name() != Some(OsStr::new("Contents")) {
        return Ok(None);
    }
    if app_dir.extension() != Some(OsStr::new("app")) {
        return Ok(None);
    }

    Ok(Some(contents_dir.join("Resources")))
}

pub fn is_bundled_app() -> bool {
    bundled_resources_dir().is_ok_and(|d| d.is_some())
}

fn macos_app_support_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("jabberwok"))
}

pub fn config_file() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let dir = macos_app_support_dir()?;
        Ok(dir.join("config").join("jabberwok.toml"))
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var_os("APPDATA").context("APPDATA is not set")?;
        Ok(PathBuf::from(appdata)
            .join("jabberwok")
            .join("config")
            .join("jabberwok.toml"))
    }

    #[cfg(target_os = "linux")]
    {
        let root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| {
                    let home = PathBuf::from(home);
                    home.join(".config")
                })
            })
            .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
        Ok(root.join("jabberwok").join("config").join("jabberwok.toml"))
    }
}

pub fn logs_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Logs")
            .join("jabberwok"))
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var_os("APPDATA").context("APPDATA is not set")?;
        Ok(PathBuf::from(appdata).join("jabberwok").join("logs"))
    }

    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").unwrap_or_default();
                PathBuf::from(home).join(".local").join("state")
            });
        Ok(base.join("jabberwok").join("logs"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignored_packaged_files_skip_appledouble_entries() {
        assert!(is_ignored_packaged_file(OsStr::new("._config.json")));
        assert!(!is_ignored_packaged_file(OsStr::new("config.json")));
    }

    #[test]
    fn logs_dir_ends_with_jabberwok() {
        let dir = logs_dir().expect("logs_dir should succeed");
        assert!(
            dir.components().any(|c| c.as_os_str() == "jabberwok"),
            "logs_dir should contain jabberwok component, got: {}",
            dir.display()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn logs_dir_is_under_library_logs_on_macos() {
        let dir = logs_dir().expect("logs_dir should succeed");
        let s = dir.to_string_lossy();
        assert!(
            s.contains("Library/Logs"),
            "macOS logs_dir should be under Library/Logs, got: {s}"
        );
    }

    #[test]
    fn bundled_resources_dir_returns_none_outside_app_bundle() {
        let result = bundled_resources_dir().expect("should not error");
        assert!(
            result.is_none(),
            "expected None outside an app bundle, got: {result:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_app_support_dir_contains_expected_components() {
        let dir = macos_app_support_dir().expect("should succeed with HOME set");
        let s = dir.to_string_lossy();
        assert!(
            s.contains("Application Support"),
            "should be under Application Support, got: {s}"
        );
        assert!(
            s.contains("jabberwok"),
            "should end with jabberwok, got: {s}"
        );
    }

    #[test]
    fn copy_missing_tree_is_noop_when_source_missing() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("nonexistent");
        let dst = dir.path().join("dst");
        copy_missing_tree(&src, &dst).expect("should succeed even when src is absent");
        assert!(!dst.exists(), "dst should not be created");
    }

    #[test]
    fn copy_missing_tree_copies_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("sub").join("dst.txt");
        std::fs::write(&src, b"hello").unwrap();

        copy_missing_tree(&src, &dst).unwrap();

        assert_eq!(std::fs::read(&dst).unwrap(), b"hello");
    }

    #[test]
    fn copy_missing_tree_does_not_overwrite_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("dst.txt");
        std::fs::write(&src, b"new content").unwrap();
        std::fs::write(&dst, b"original").unwrap();

        copy_missing_tree(&src, &dst).unwrap();

        assert_eq!(
            std::fs::read(&dst).unwrap(),
            b"original",
            "existing destination should not be overwritten"
        );
    }
}
