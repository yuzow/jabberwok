use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const BUNDLE_IDENTIFIER: &str = "computer.handy.jabberwok";

const LAUNCH_AGENT_PLIST_TEMPLATE: &str =
    include_str!("../../../xtask/assets/macos/LaunchAgent.plist");

pub fn is_launch_agent_installed() -> bool {
    launch_agent_path().map(|p| p.exists()).unwrap_or(false)
}

pub fn install_launch_agent() -> Result<()> {
    let exe = launch_agent_executable()?;
    let logs_dir = crate::config::logs_dir()?;

    let plist = LAUNCH_AGENT_PLIST_TEMPLATE
        .replace("__APP_EXECUTABLE__", &exe.display().to_string())
        .replace("__LOGS_DIR__", &logs_dir.display().to_string());

    let agent_path = launch_agent_path()?;
    if let Some(parent) = agent_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    // Bootout any existing registration before writing the new plist.
    bootout();

    std::fs::write(&agent_path, &plist)
        .with_context(|| format!("failed to write {}", agent_path.display()))?;

    run_launchctl([
        "bootstrap",
        &gui_domain()?,
        &agent_path.display().to_string(),
    ])?;
    run_launchctl(["enable", &service_target()?])?;

    Ok(())
}

pub fn uninstall_launch_agent() -> Result<()> {
    bootout();

    let path = launch_agent_path()?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
    }

    Ok(())
}

/// Attempts to unregister the service from launchd; ignores errors since it
/// may not be registered yet.
fn bootout() {
    if let Ok(target) = service_target() {
        let _ = Command::new("launchctl")
            .args(["bootout", &target])
            .output();
    }
    if let (Ok(domain), Ok(path)) = (gui_domain(), launch_agent_path()) {
        let _ = Command::new("launchctl")
            .args(["bootout", &domain, &path.display().to_string()])
            .output();
    }
}

fn run_launchctl<const N: usize>(args: [&str; N]) -> Result<()> {
    let status = Command::new("launchctl")
        .args(args)
        .status()
        .with_context(|| format!("failed to run launchctl {}", args.join(" ")))?;
    if status.success() {
        Ok(())
    } else {
        bail!("launchctl {} failed", args.join(" "))
    }
}

fn launch_agent_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{BUNDLE_IDENTIFIER}.plist")))
}

fn gui_domain() -> Result<String> {
    Ok(format!("gui/{}", current_uid()))
}

fn service_target() -> Result<String> {
    Ok(format!("{}/{BUNDLE_IDENTIFIER}", gui_domain()?))
}

fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

fn launch_agent_executable() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("failed to locate current executable")?;
    Ok(homebrew_opt_executable(&exe).unwrap_or(exe))
}

fn homebrew_opt_executable(exe: &Path) -> Option<PathBuf> {
    let bin_dir = exe.parent()?;
    let version_dir = bin_dir.parent()?;
    let package_dir = version_dir.parent()?;
    let cellar_dir = package_dir.parent()?;

    if package_dir.file_name()? != "jabberwok" || cellar_dir.file_name()? != "Cellar" {
        return None;
    }

    let prefix = cellar_dir.parent()?;
    let opt_exe = prefix
        .join("opt")
        .join("jabberwok")
        .join("bin")
        .join("jabberwok");
    opt_exe.exists().then_some(opt_exe)
}
