use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const APP_NAME: &str = "Jabberwok";
const INFO_PLIST: &str = include_str!("../assets/macos/Info.plist");
const LAUNCH_AGENT_PLIST: &str = include_str!("../assets/macos/LaunchAgent.plist");
const APP_ICON_PATH: &str = "xtask/assets/macos/app-icon.icns";
const APP_BUNDLE: &str = "Jabberwok.app";
const BUNDLE_IDENTIFIER: &str = "computer.handy.jabberwok";

pub fn package() -> Result<()> {
    stage_app_bundle()?;
    package_dmg()?;
    Ok(())
}

pub fn stage_app_bundle() -> Result<PathBuf> {
    let stage_dir = macos_stage_dir();
    reset_app_stage_dir(&stage_dir)?;

    let app_dir = staged_app_path();
    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");
    let defaults_dir = resources_dir.join("defaults");

    fs::create_dir_all(&macos_dir)
        .with_context(|| format!("failed to create {}", macos_dir.display()))?;
    fs::create_dir_all(&defaults_dir)
        .with_context(|| format!("failed to create {}", defaults_dir.display()))?;

    copy_file(
        &Path::new("target").join("release").join("jabberwok"),
        &macos_dir.join(APP_NAME),
    )?;
    mark_executable(&macos_dir.join(APP_NAME))?;
    copy_file(
        Path::new(APP_ICON_PATH),
        &resources_dir.join("app-icon.icns"),
    )?;

    fs::write(contents_dir.join("Info.plist"), INFO_PLIST)
        .with_context(|| format!("failed to write Info.plist to {}", contents_dir.display()))?;

    copy_tree_filtered(Path::new("config"), &defaults_dir.join("config"))?;
    sign_app_bundle(&app_dir)?;

    println!("staged macOS app bundle in {}", app_dir.display());
    Ok(app_dir)
}

pub fn package_dmg() -> Result<PathBuf> {
    let app_dir = ensure_staged_app_bundle()?;
    let dmg_path = staged_dmg_path();

    create_dmg(&app_dir, &dmg_path)?;
    println!("created macOS distributable at {}", dmg_path.display());
    Ok(dmg_path)
}

pub fn install_service() -> Result<()> {
    let staged_app = ensure_staged_app_bundle()?;
    let installed_app = installed_app_path()?;
    let installed_executable = installed_app.join("Contents").join("MacOS").join(APP_NAME);
    let launch_agent_path = launch_agent_path()?;

    let logs_dir = home_dir()?.join("Library").join("Logs").join("jabberwok");
    fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;

    install_app_bundle(&staged_app, &installed_app)?;
    reset_accessibility_permission()?;
    install_launch_agent(&launch_agent_path, &installed_executable)?;
    bootstrap_launch_agent(&launch_agent_path)?;

    println!("installed macOS app bundle at {}", installed_app.display());
    println!(
        "registered LaunchAgent {} at {}",
        BUNDLE_IDENTIFIER,
        launch_agent_path.display()
    );
    println!("Accessibility permission was reset — grant it when prompted on first launch.");
    Ok(())
}

pub fn uninstall_service() -> Result<()> {
    let launch_agent_path = launch_agent_path()?;
    let installed_app = installed_app_path()?;
    let app_support_dir = home_dir()?
        .join("Library")
        .join("Application Support")
        .join("jabberwok");
    let logs_dir = home_dir()?.join("Library").join("Logs").join("jabberwok");

    bootout_launch_agent()?;

    reset_app_permissions()?;
    remove_path(&launch_agent_path)?;
    remove_path(&installed_app)?;
    remove_path(&app_support_dir)?;
    remove_path(&logs_dir)?;

    println!("removed macOS LaunchAgent {}", BUNDLE_IDENTIFIER);
    println!(
        "removed installed app bundle at {}",
        installed_app.display()
    );
    println!(
        "removed app support dir at {} (including installed config)",
        app_support_dir.display()
    );
    println!("removed logs dir at {}", logs_dir.display());
    println!(
        "reset macOS TCC entries for {} (Accessibility, Microphone)",
        BUNDLE_IDENTIFIER
    );
    Ok(())
}

fn remove_path(path: &std::path::Path) -> Result<()> {
    if path.is_file() {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    } else if path.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

fn stage_dir(platform: &str) -> PathBuf {
    Path::new("target").join("xtask").join(platform)
}

fn macos_stage_dir() -> PathBuf {
    stage_dir("macos")
}

fn staged_app_path() -> PathBuf {
    macos_stage_dir().join(APP_BUNDLE)
}

fn staged_dmg_path() -> PathBuf {
    macos_stage_dir().join("Jabberwok.dmg")
}

fn reset_app_stage_dir(stage_dir: &Path) -> Result<()> {
    if stage_dir.exists() {
        let app_dir = stage_dir.join(APP_BUNDLE);
        let dmg_path = stage_dir.join("Jabberwok.dmg");
        let tmp_dmg_stem = stage_dir.join("Jabberwok-tmp");
        let tmp_dmg = stage_dir.join("Jabberwok-tmp.dmg");

        remove_path(&app_dir)?;
        remove_path(&dmg_path)?;
        remove_path(&tmp_dmg)?;
        remove_path(&tmp_dmg_stem)?;
    }

    fs::create_dir_all(stage_dir)
        .with_context(|| format!("failed to create {}", stage_dir.display()))?;
    Ok(())
}

fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(src, dst)
        .with_context(|| format!("failed to copy {} to {}", src.display(), dst.display()))?;
    Ok(())
}

fn copy_tree_filtered(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }

    if src.is_file() {
        copy_file(src, dst)?;
        return Ok(());
    }

    fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;

    for entry in fs::read_dir(src).with_context(|| format!("failed to read {}", src.display()))? {
        let entry = entry.with_context(|| format!("failed to read entry in {}", src.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        if should_skip_packaged_file(&name) {
            continue;
        }
        copy_tree_filtered(&path, &dst.join(name))?;
    }

    Ok(())
}

fn should_skip_packaged_file(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with("._")
}

fn mark_executable(path: &Path) -> Result<()> {
    let mut perms = fs::metadata(path)
        .with_context(|| format!("failed to read metadata for {}", path.display()))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)
        .with_context(|| format!("failed to set executable bit on {}", path.display()))?;
    Ok(())
}

fn sign_app_bundle(app_dir: &Path) -> Result<()> {
    // Re-sign the assembled bundle so Gatekeeper evaluates the final app
    // contents instead of the copied release binary's embedded signature data.
    let status = Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-", "--timestamp=none"])
        .arg(app_dir)
        .status()
        .with_context(|| format!("failed to run codesign for {}", app_dir.display()))?;
    if !status.success() {
        bail!("codesign failed for {}", app_dir.display());
    }

    let status = Command::new("codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=2"])
        .arg(app_dir)
        .status()
        .with_context(|| format!("failed to verify code signature for {}", app_dir.display()))?;
    if !status.success() {
        bail!("codesign verification failed for {}", app_dir.display());
    }

    Ok(())
}

fn create_dmg(app_dir: &Path, dmg_path: &Path) -> Result<()> {
    let stage_dir = app_dir
        .parent()
        .context("app bundle has no parent directory")?;
    let tmp_dmg_stem = stage_dir.join("Jabberwok-tmp");
    let tmp_dmg = stage_dir.join("Jabberwok-tmp.dmg");
    let mount_point = Path::new("/tmp/jabberwok-dmg-mount");

    // 1. Create temporary writable DMG (hdiutil appends .dmg to the stem)
    let status = Command::new("hdiutil")
        .args(["create", "-size", "150m", "-volname", "Jabberwok"])
        .arg(&tmp_dmg_stem)
        .status()
        .context("failed to run hdiutil create")?;
    if !status.success() {
        bail!("hdiutil create failed");
    }

    // 2–5. Attach, populate, run AppleScript, detach; clean up tmp DMG on error
    let result = attach_populate_detach(app_dir, &tmp_dmg, mount_point);
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp_dmg);
        return Err(e);
    }

    // 6. Convert to compressed read-only UDZO
    let status = Command::new("hdiutil")
        .args(["convert"])
        .arg(&tmp_dmg)
        .args(["-format", "UDZO", "-imagekey", "zlib-level=9", "-o"])
        .arg(dmg_path)
        .status()
        .context("failed to run hdiutil convert")?;
    if !status.success() {
        let _ = fs::remove_file(&tmp_dmg);
        bail!("hdiutil convert failed");
    }

    // 7. Remove temporary writable DMG
    fs::remove_file(&tmp_dmg).with_context(|| format!("failed to remove {}", tmp_dmg.display()))?;

    Ok(())
}

fn attach_populate_detach(app_dir: &Path, tmp_dmg: &Path, mount_point: &Path) -> Result<()> {
    // 2. Attach
    let status = Command::new("hdiutil")
        .args([
            "attach",
            "-readwrite",
            "-noverify",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(mount_point)
        .arg(tmp_dmg)
        .status()
        .context("failed to run hdiutil attach")?;
    if !status.success() {
        bail!("hdiutil attach failed");
    }

    // 3–4. Populate and configure layout; always attempt detach afterwards
    let populate_result = populate_dmg(app_dir, mount_point);

    // 5. Detach regardless of whether populate succeeded
    let detach_ok = Command::new("hdiutil")
        .args(["detach"])
        .arg(mount_point)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    populate_result?;

    if !detach_ok {
        bail!("hdiutil detach failed");
    }

    Ok(())
}

fn populate_dmg(app_dir: &Path, mount_point: &Path) -> Result<()> {
    // Copy app bundle into the volume
    let status = Command::new("ditto")
        .arg(app_dir)
        .arg(mount_point.join(APP_BUNDLE))
        .status()
        .context("failed to run ditto")?;
    if !status.success() {
        bail!("ditto failed while copying app bundle to DMG");
    }

    // Create /Applications symlink
    let status = Command::new("ln")
        .args(["-s", "/Applications"])
        .arg(mount_point.join("Applications"))
        .status()
        .context("failed to create Applications symlink")?;
    if !status.success() {
        bail!("ln -s failed while creating Applications symlink");
    }

    Ok(())
}

fn ensure_staged_app_bundle() -> Result<PathBuf> {
    let app_dir = staged_app_path();
    if app_dir.is_dir() {
        Ok(app_dir)
    } else {
        crate::package::build_release_binary()?;
        let staged_app = stage_app_bundle()?;
        if staged_app.is_dir() {
            Ok(staged_app)
        } else {
            bail!("expected staged app bundle at {}", app_dir.display())
        }
    }
}

fn install_app_bundle(staged_app: &Path, installed_app: &Path) -> Result<()> {
    let applications_dir = installed_app
        .parent()
        .context("installed app path has no parent directory")?;
    fs::create_dir_all(applications_dir)
        .with_context(|| format!("failed to create {}", applications_dir.display()))?;

    if installed_app.exists() {
        fs::remove_dir_all(installed_app)
            .with_context(|| format!("failed to remove {}", installed_app.display()))?;
    }

    let status = Command::new("ditto")
        .arg(staged_app)
        .arg(installed_app)
        .status()
        .with_context(|| {
            format!(
                "failed to copy app bundle from {} to {}",
                staged_app.display(),
                installed_app.display()
            )
        })?;

    if status.success() {
        Ok(())
    } else {
        bail!(
            "ditto failed while copying {} to {}",
            staged_app.display(),
            installed_app.display()
        )
    }
}

fn install_launch_agent(launch_agent_path: &Path, installed_executable: &Path) -> Result<()> {
    let launch_agents_dir = launch_agent_path
        .parent()
        .context("launch agent path has no parent directory")?;
    fs::create_dir_all(launch_agents_dir)
        .with_context(|| format!("failed to create {}", launch_agents_dir.display()))?;

    let logs_dir = home_dir()?.join("Library").join("Logs").join("jabberwok");
    let plist = LAUNCH_AGENT_PLIST
        .replace(
            "__APP_EXECUTABLE__",
            &installed_executable.display().to_string(),
        )
        .replace("__LOGS_DIR__", &logs_dir.display().to_string());

    fs::write(launch_agent_path, plist)
        .with_context(|| format!("failed to write {}", launch_agent_path.display()))?;
    Ok(())
}

fn reset_accessibility_permission() -> Result<()> {
    // Clear any stale TCC entry (e.g. from a dev binary at a different path)
    // so the newly installed app can request a clean grant on first launch.
    // tccutil exits non-zero if no entry exists — that's fine.
    reset_tcc_service("Accessibility")
}

fn reset_app_permissions() -> Result<()> {
    reset_tcc_service("Accessibility")?;
    reset_tcc_service("Microphone")?;
    Ok(())
}

fn reset_tcc_service(service: &str) -> Result<()> {
    let output = Command::new("tccutil")
        .args(["reset", service, BUNDLE_IDENTIFIER])
        .output()
        .context("failed to run tccutil")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "tccutil reset {service} failed for {BUNDLE_IDENTIFIER}: {}",
            stderr.trim()
        );
    }
    Ok(())
}

fn bootstrap_launch_agent(launch_agent_path: &Path) -> Result<()> {
    bootout_launch_agent()?;
    wait_for_service_gone()?;
    run_launchctl([
        "bootstrap",
        &gui_domain()?,
        &launch_agent_path.display().to_string(),
    ])?;
    run_launchctl(["enable", &launch_agent_service_target()?])?;
    Ok(())
}

fn bootout_launch_agent() -> Result<()> {
    let service_target = launch_agent_service_target()?;
    let output = Command::new("launchctl")
        .args(["bootout", &service_target])
        .output()
        .with_context(|| format!("failed to run launchctl bootout for {service_target}"))?;

    if output.status.success() {
        return Ok(());
    }

    let file_target = launch_agent_path()?.display().to_string();
    let _output = Command::new("launchctl")
        .args(["bootout", &gui_domain()?, &file_target])
        .output()
        .with_context(|| format!("failed to run launchctl bootout for {file_target}"))?;

    Ok(())
}

/// Poll until the service is no longer listed in launchd, or bail after 5s.
fn wait_for_service_gone() -> Result<()> {
    for _ in 0..10 {
        let still_listed = Command::new("launchctl")
            .args(["list", BUNDLE_IDENTIFIER])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !still_listed {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    bail!("service {BUNDLE_IDENTIFIER} did not unregister within 5s after bootout")
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

fn installed_app_path() -> Result<PathBuf> {
    Ok(home_dir()?.join("Applications").join(APP_BUNDLE))
}

fn launch_agent_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{BUNDLE_IDENTIFIER}.plist")))
}
fn home_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home))
}

fn gui_domain() -> Result<String> {
    Ok(format!("gui/{}", current_uid()?))
}

fn launch_agent_service_target() -> Result<String> {
    Ok(format!("{}/{}", gui_domain()?, BUNDLE_IDENTIFIER))
}

fn current_uid() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("failed to determine current uid with id -u")?;

    if !output.status.success() {
        bail!("id -u exited with {}", output.status);
    }

    let uid = String::from_utf8(output.stdout).context("id -u returned non-UTF-8 output")?;
    Ok(uid.trim().to_string())
}
