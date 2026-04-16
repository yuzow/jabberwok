use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, anyhow};

use crate::config::JabberwokConfig;
use crate::os;
use crate::state::{LocalStatePaths, StartupState};

#[derive(Debug, Clone)]
pub struct Readiness {
    pub parseable_config: bool,
    pub default_model_name: Option<String>,
    pub default_model_path: Option<PathBuf>,
    pub model_installed: bool,
    pub model_dir_writable: bool,
    pub model_dir_enough_space: bool,
    pub model_dir_free_bytes: Option<u64>,
    pub microphone_permission_granted: bool,
    pub accessibility_permission_granted: bool,
    pub can_start_daemon: bool,
    pub startup_state: StartupState,
    pub repair_reason: Option<String>,
}

impl Readiness {
    pub fn log_for_startup(&self, config_path: &Path) {
        tracing::info!(
            config_path = %config_path.display(),
            parseable_config = self.parseable_config,
            default_model_name = self.default_model_name.as_deref().unwrap_or("<none>"),
            default_model_path = self
                .default_model_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<none>".to_string()),
            model_installed = self.model_installed,
            model_dir_writable = self.model_dir_writable,
            model_dir_enough_space = self.model_dir_enough_space,
            model_dir_free_bytes = self.model_dir_free_bytes,
            microphone_permission_granted = self.microphone_permission_granted,
            accessibility_permission_granted = self.accessibility_permission_granted,
            can_start_daemon = self.can_start_daemon,
            startup_state = ?self.startup_state,
            repair_reason = self.repair_reason.as_deref().unwrap_or("<none>"),
            "startup readiness evaluated"
        );
    }
}

pub fn models_dir_for_config(config_path: &Path) -> PathBuf {
    LocalStatePaths::from_config_path(config_path)
        .map(|paths| paths.models_dir)
        .unwrap_or_else(|_| {
            let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
            parent
                .parent()
                .map_or_else(|| parent.join("models"), |p| p.join("models"))
        })
}

pub fn evaluate_readiness(config_path: &Path) -> Result<Readiness> {
    let cfg = match JabberwokConfig::load(config_path) {
        Ok(cfg) => cfg,
        Err(_) => {
            return Ok(Readiness {
                parseable_config: false,
                default_model_name: None,
                default_model_path: None,
                model_installed: false,
                model_dir_writable: false,
                model_dir_enough_space: false,
                model_dir_free_bytes: None,
                microphone_permission_granted: false,
                accessibility_permission_granted: false,
                can_start_daemon: false,
                startup_state: StartupState::NeedsRepair,
                repair_reason: Some(format!(
                    "Existing config at {} could not be parsed. Fix it or reset local data to continue.",
                    config_path.display()
                )),
            });
        }
    };

    let model_dir = models_dir_for_config(config_path);
    tracing::info!(model_dir = %model_dir.display(), "resolved model directory");
    let model_dir_writable = is_directory_writable(&model_dir);
    let (model_dir_enough_space, model_dir_free_bytes) = sufficient_disk_space(&model_dir);

    let models = crate::config::ModelConfig::from(cfg.models);
    let default_model_name = models.default.clone();
    let default_model_path = default_model_name
        .as_deref()
        .and_then(|name| models.get(name).and_then(|m| m.path.clone()));
    let model_installed = default_model_path.as_ref().is_some_and(|p| p.exists());

    let microphone_permission_granted = os::has_microphone_permission();
    let accessibility_permission_granted = os::has_accessibility_permission();

    let can_start_daemon = models.default.is_some()
        && model_installed
        && model_dir_writable
        && model_dir_enough_space
        && microphone_permission_granted
        && accessibility_permission_granted;

    let (startup_state, repair_reason) = if can_start_daemon {
        (StartupState::Ready, None)
    } else if !model_dir_writable {
        (
            StartupState::NeedsRepair,
            Some(format!(
                "The models directory at {} is not writable.",
                model_dir.display()
            )),
        )
    } else if !model_dir_enough_space {
        (
            StartupState::NeedsRepair,
            Some(
                "There is not enough free disk space to install or repair the default model."
                    .to_string(),
            ),
        )
    } else if models.default.is_none() {
        (
            StartupState::NeedsSetup,
            Some("Choose and install a default model to finish setup.".to_string()),
        )
    } else if !model_installed {
        let detail = match (default_model_name.as_deref(), default_model_path.as_ref()) {
            (Some(name), Some(path)) => format!(
                "Your existing Jabberwok data was found, but the default model `{name}` is missing at {}. Re-download it to continue.",
                path.display()
            ),
            (Some(name), None) => format!(
                "Your existing Jabberwok data was found, but the default model `{name}` has no install path. Re-download it to continue."
            ),
            _ => "Your existing Jabberwok data was found, but the default model is missing. Re-download it to continue.".to_string(),
        };
        (StartupState::NeedsRepair, Some(detail))
    } else {
        (
            StartupState::NeedsSetup,
            Some("Finish the remaining permission steps to start Jabberwok.".to_string()),
        )
    };

    Ok(Readiness {
        parseable_config: true,
        default_model_name,
        default_model_path,
        model_installed,
        model_dir_writable,
        model_dir_enough_space,
        model_dir_free_bytes,
        microphone_permission_granted,
        accessibility_permission_granted,
        can_start_daemon,
        startup_state,
        repair_reason,
    })
}

pub fn status_text(granted: bool) -> &'static str {
    if granted { "Granted" } else { "Not granted" }
}

pub struct DoctorReport {
    pub readiness: Readiness,
    pub text: String,
}

#[derive(Clone)]
pub struct SetupProgress {
    pub percent: f64,
    pub phase: String,
    pub microphone_permission_granted: bool,
    pub accessibility_permission_granted: bool,
    pub ready: bool,
    pub download_failed: bool,
    pub needs_download: bool,
    pub model_installed: bool,
    pub model_name: Option<String>,
}

pub fn doctor_report(config_path: &Path) -> Result<DoctorReport> {
    let ready = evaluate_readiness(config_path)?;

    let mut text = String::new();
    text.push_str("Jabberwok doctor\n\n");

    if !ready.parseable_config {
        text.push_str("Config:         REPAIR  could not parse jabberwok.toml\n");
        text.push_str("Model:          UNKNOWN\n");
        text.push_str("Storage:        UNKNOWN\n");
        text.push_str("Microphone:     UNKNOWN\n");
        text.push_str("Accessibility:  UNKNOWN\n");
        text.push_str("Startup state:  NEEDS REPAIR\n");
        text.push_str("Daemon ready:   NO\n\n");
        text.push_str("Hints:\n");
        text.push_str("- Fix `jabberwok.toml` and run `jabberwok doctor` again.\n");
        text.push_str("- Or use `jabberwok reset` to return to first-run setup.\n");
        return Ok(DoctorReport {
            readiness: ready,
            text,
        });
    }

    text.push_str("Config:         OK\n");

    if let (Some(name), Some(path)) = (
        ready.default_model_name.as_deref(),
        ready.default_model_path.as_ref(),
    ) {
        if ready.model_installed {
            text.push_str(&format!(
                "Model:          OK      {name} at {}\n",
                path.display()
            ));
        } else {
            text.push_str(&format!(
                "Model:          MISSING {name} expected at {}\n",
                path.display()
            ));
        }
    } else if let Some(name) = ready.default_model_name.as_deref() {
        text.push_str(&format!(
            "Model:          MISSING {name} has no install path\n"
        ));
    } else {
        text.push_str("Model:          MISSING default model not configured\n");
    }

    let free_space = ready
        .model_dir_free_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "unknown".to_string());
    let storage_status = if ready.model_dir_writable && ready.model_dir_enough_space {
        "OK"
    } else {
        "MISSING"
    };
    let storage_detail = match (ready.model_dir_writable, ready.model_dir_enough_space) {
        (true, true) => format!("writable, {free_space} free"),
        (false, true) => format!("model directory is not writable, {free_space} free"),
        (true, false) => format!("not enough free space, {free_space} available"),
        (false, false) => format!("model directory is not writable, {free_space} available"),
    };
    text.push_str(&format!(
        "Storage:        {storage_status} {storage_detail}\n"
    ));

    text.push_str(&format!(
        "Microphone:     {} {}\n",
        if ready.microphone_permission_granted {
            "OK"
        } else {
            "MISSING"
        },
        status_text(ready.microphone_permission_granted)
    ));
    text.push_str(&format!(
        "Accessibility:  {} {}\n",
        if ready.accessibility_permission_granted {
            "OK"
        } else {
            "MISSING"
        },
        status_text(ready.accessibility_permission_granted)
    ));
    text.push_str(&format!(
        "Startup state:  {}\n",
        match ready.startup_state {
            StartupState::Ready => "READY",
            StartupState::NeedsSetup => "NEEDS SETUP",
            StartupState::NeedsRepair => "NEEDS REPAIR",
        }
    ));
    if let Some(reason) = ready.repair_reason.as_deref() {
        text.push_str(&format!("Detail:         {reason}\n"));
    }
    text.push_str(&format!(
        "Daemon ready:   {}\n\n",
        if ready.can_start_daemon { "YES" } else { "NO" }
    ));

    if !ready.can_start_daemon {
        text.push_str("Hints:\n");
        if !ready.model_installed
            && let Some(name) = ready.default_model_name.as_deref()
        {
            text.push_str(&format!(
                "- Download the default model: jabberwok download-model {name}\n"
            ));
        }
        if !ready.model_dir_writable {
            text.push_str("- Fix write access to the app support `models/` directory.\n");
        }
        if !ready.model_dir_enough_space {
            text.push_str("- Free disk space before downloading the model.\n");
        }
        if !ready.microphone_permission_granted {
            text.push_str("- Grant microphone permission: jabberwok permissions microphone\n");
        }
        if !ready.accessibility_permission_granted {
            text.push_str(
                "- Grant accessibility permission: jabberwok permissions accessibility\n",
            );
        }
        if matches!(ready.startup_state, StartupState::NeedsRepair) {
            text.push_str("- Use `jabberwok reset` for a clean local reset.\n");
        }
    }

    Ok(DoctorReport {
        readiness: ready,
        text,
    })
}

pub fn run_setup_worker(
    config_path: PathBuf,
    tx: Sender<SetupProgress>,
    retry_requested: Arc<AtomicBool>,
    cancel_requested: Arc<AtomicBool>,
) {
    use crate::models::download_model_with_progress_and_phase;
    use std::thread::sleep;
    use std::time::Duration;

    let _ = tx.send(SetupProgress {
        percent: 0.0,
        phase: "Checking setup...".to_string(),
        microphone_permission_granted: false,
        accessibility_permission_granted: false,
        ready: false,
        download_failed: false,
        needs_download: false,
        model_installed: false,
        model_name: None,
    });

    loop {
        if cancel_requested.load(Ordering::Relaxed) {
            return;
        }

        let state = match evaluate_readiness(&config_path) {
            Ok(v) => v,
            Err(_) => {
                sleep(Duration::from_millis(500));
                continue;
            }
        };

        let _ = tx.send(SetupProgress {
            percent: 10.0,
            phase: state
                .repair_reason
                .clone()
                .unwrap_or_else(|| "Checking permissions...".to_string()),
            microphone_permission_granted: state.microphone_permission_granted,
            accessibility_permission_granted: state.accessibility_permission_granted,
            ready: state.can_start_daemon,
            download_failed: false,
            needs_download: false,
            model_installed: state.model_installed,
            model_name: state.default_model_name.clone(),
        });

        if let Some(name) = state.default_model_name.clone()
            && !state.model_installed
        {
            let _ = tx.send(SetupProgress {
                percent: 10.0,
                phase: "Click Download to install the model.".to_string(),
                microphone_permission_granted: state.microphone_permission_granted,
                accessibility_permission_granted: state.accessibility_permission_granted,
                ready: false,
                download_failed: false,
                needs_download: true,
                model_installed: false,
                model_name: Some(name.clone()),
            });
            loop {
                if cancel_requested.load(Ordering::Relaxed) {
                    return;
                }
                if retry_requested.swap(false, Ordering::Relaxed) {
                    break;
                }
                sleep(Duration::from_millis(200));
            }

            let _ = tx.send(SetupProgress {
                percent: 20.0,
                phase: format!("Downloading {}...", name),
                microphone_permission_granted: state.microphone_permission_granted,
                accessibility_permission_granted: state.accessibility_permission_granted,
                ready: false,
                download_failed: false,
                needs_download: false,
                model_installed: false,
                model_name: Some(name.clone()),
            });

            let dir = models_dir_for_config(&config_path);
            let progress_tx = tx.clone();
            let phase_tx = tx.clone();
            let name_for_progress = name.clone();
            let name_for_phase = name.clone();
            match download_model_with_progress_and_phase(
                &config_path,
                &dir,
                &name,
                move |downloaded, total| {
                    let pct = if total == 0 {
                        0.0
                    } else {
                        (downloaded as f64 / total as f64) * 60.0 + 20.0
                    };
                    let _ = progress_tx.send(SetupProgress {
                        percent: pct,
                        phase: format!("Downloading {}...", name_for_progress),
                        microphone_permission_granted: state.microphone_permission_granted,
                        accessibility_permission_granted: state.accessibility_permission_granted,
                        ready: false,
                        download_failed: false,
                        needs_download: false,
                        model_installed: false,
                        model_name: Some(name_for_progress.clone()),
                    });
                },
                move |phase| {
                    let (percent, display_phase) = match phase {
                        "Downloading model..." => {
                            (20.0, format!("Downloading {}...", name_for_phase))
                        }
                        "Extracting model..." => {
                            (85.0, format!("Extracting {}...", name_for_phase))
                        }
                        "Finishing setup..." => (95.0, "Finishing setup...".to_string()),
                        _ => (20.0, format!("Downloading {}...", name_for_phase)),
                    };
                    let _ = phase_tx.send(SetupProgress {
                        percent,
                        phase: display_phase,
                        microphone_permission_granted: state.microphone_permission_granted,
                        accessibility_permission_granted: state.accessibility_permission_granted,
                        ready: false,
                        download_failed: false,
                        needs_download: false,
                        model_installed: false,
                        model_name: Some(name_for_phase.clone()),
                    });
                },
            ) {
                Ok(_) => {}
                Err(err) => {
                    let _ = tx.send(SetupProgress {
                        percent: 0.0,
                        phase: format!("Download failed: {err}. Click Retry."),
                        microphone_permission_granted: state.microphone_permission_granted,
                        accessibility_permission_granted: state.accessibility_permission_granted,
                        ready: false,
                        download_failed: true,
                        needs_download: false,
                        model_installed: false,
                        model_name: Some(name.clone()),
                    });
                    loop {
                        if cancel_requested.load(Ordering::Relaxed) {
                            return;
                        }
                        if retry_requested.swap(false, Ordering::Relaxed) {
                            break;
                        }
                        sleep(Duration::from_millis(200));
                    }
                    continue;
                }
            }
        }

        if state.can_start_daemon {
            let _ = tx.send(SetupProgress {
                percent: 100.0,
                phase: "Ready. Launch Jabberwok when you're ready.".to_string(),
                microphone_permission_granted: true,
                accessibility_permission_granted: true,
                ready: true,
                download_failed: false,
                needs_download: false,
                model_installed: true,
                model_name: state.default_model_name.clone(),
            });
            return;
        }

        sleep(Duration::from_millis(500));
    }
}

pub fn doctor_text(config_path: &Path) -> Result<String> {
    Ok(doctor_report(config_path)?.text)
}

pub fn permission_already_granted(name: &str) -> Result<bool> {
    match name {
        "microphone" => Ok(os::has_microphone_permission()),
        "accessibility" => Ok(os::has_accessibility_permission()),
        "all" => Ok(os::has_microphone_permission() && os::has_accessibility_permission()),
        _ => Err(anyhow!("unknown permission target: {name}")),
    }
}

pub fn request_permission(name: &str) -> Result<()> {
    match name {
        "microphone" => {
            os::request_microphone_permission();
            Ok(())
        }
        "accessibility" => {
            os::open_accessibility_system_settings();
            Ok(())
        }
        "all" => {
            os::request_microphone_permission();
            os::open_accessibility_system_settings();
            Ok(())
        }
        _ => Err(anyhow!("unknown permission target: {name}")),
    }
}

pub fn permission_request_instructions(name: &str) -> Result<String> {
    let current_exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    let launch_agent_exe = crate::os::launch_agent_executable_path()
        .ok()
        .map(|p| p.display().to_string());

    match name {
        "microphone" => Ok(format!(
            "Requested microphone permission flow.\nCurrent executable: {current_exe}"
        )),
        "accessibility" => {
            let mut lines = vec![
                "Opened Accessibility settings.".to_string(),
                "macOS does not allow Jabberwok to add or grant Accessibility permission automatically.".to_string(),
                format!("Current executable: {current_exe}"),
            ];
            if let Some(path) = launch_agent_exe.as_deref()
                && path != current_exe
            {
                lines.push(format!("LaunchAgent executable: {path}"));
            }
            lines.push(
                "Use the + button in System Settings if needed, add the executable you actually run, and enable it."
                    .to_string(),
            );
            Ok(lines.join("\n"))
        }
        "all" => {
            let mut lines = vec![
                "Requested microphone permission flow and opened Accessibility settings."
                    .to_string(),
                "macOS does not allow Jabberwok to add or grant Accessibility permission automatically.".to_string(),
                format!("Current executable: {current_exe}"),
            ];
            if let Some(path) = launch_agent_exe.as_deref()
                && path != current_exe
            {
                lines.push(format!("LaunchAgent executable: {path}"));
            }
            lines.push(
                "Use the + button in System Settings if needed, add the executable you actually run, and enable it."
                    .to_string(),
            );
            Ok(lines.join("\n"))
        }
        _ => Err(anyhow!("unknown permission target: {name}")),
    }
}

pub fn reset_local_data(paths: &LocalStatePaths) -> Result<Vec<PathBuf>> {
    let removed = crate::state::reset_local_data(paths)?;
    if let Err(err) = os::clear_permission_state("All") {
        tracing::warn!(%err, "failed to clear permission state during reset");
    }
    Ok(removed)
}

pub fn describe_reset_result(paths: &LocalStatePaths, removed: &[PathBuf]) -> String {
    let mut lines = vec!["Reset Jabberwok local data.".to_string()];

    if removed.is_empty() {
        lines.push("No local config, models, or logs were present.".to_string());
    } else {
        lines.push("Removed:".to_string());
        for path in removed {
            lines.push(format!("- {}", path.display()));
        }
    }

    lines.push(format!(
        "Next launch will bootstrap fresh defaults from {}.",
        paths.config_file.display()
    ));
    lines.join("\n")
}

pub fn relaunch_after_reset() -> Result<()> {
    let exe = std::env::current_exe()?;
    Command::new(exe).arg("setup").spawn()?;
    Ok(())
}

#[cfg(not(debug_assertions))]
pub fn remove_permissions(_name: &str) -> Result<String> {
    Err(anyhow!(
        "--remove is debug-only; rebuild with debug to use it"
    ))
}

#[cfg(debug_assertions)]
pub fn remove_permissions(name: &str) -> Result<String> {
    let kind = match name {
        "microphone" => "Microphone",
        "accessibility" => "Accessibility",
        "all" => "All",
        _ => return Err(anyhow!("unknown permission target: {name}")),
    };
    os::clear_permission_state(kind)?;
    let running_bundled_app = crate::config::is_bundled_app();
    let followup = if running_bundled_app {
        format!(
            "This reset targets the bundled Jabberwok app identity. If the permission still appears granted, open System Settings > Privacy & Security > {kind} and toggle Jabberwok off, then re-run `jabberwok doctor`."
        )
    } else {
        format!(
            "This reset targets the bundled Jabberwok app identity, but `cargo run` usually inherits {kind} permission from your terminal app.\nOpen System Settings > Privacy & Security > {kind} and toggle Terminal or WezTerm off, then re-run `jabberwok doctor`."
        )
    };
    Ok(format!(
        "Cleared {name} state. This is a debug-only local reset and may still require a reboot.\n{followup}"
    ))
}

fn is_directory_writable(path: &Path) -> bool {
    if !path.exists() && std::fs::create_dir_all(path).is_err() {
        return false;
    }
    let marker = path.join(".jabberwok-write-check");
    match std::fs::write(&marker, b"ok") {
        Ok(_) => {
            let _ = std::fs::remove_file(marker);
            true
        }
        Err(_) => false,
    }
}

#[cfg(target_os = "macos")]
fn sufficient_disk_space(path: &Path) -> (bool, Option<u64>) {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    let c_path = match CString::new(path.to_string_lossy().as_ref()) {
        Ok(v) => v,
        Err(_) => return (false, None),
    };
    if unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return (false, None);
    }
    let stats = unsafe { stats.assume_init() };
    let avail = (stats.f_bavail as u64).saturating_mul(stats.f_bsize);
    (avail >= 1024 * 1024 * 1024, Some(avail))
}

#[cfg(not(target_os = "macos"))]
fn sufficient_disk_space(_path: &Path) -> (bool, Option<u64>) {
    (true, None)
}

fn format_bytes(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    }
}
