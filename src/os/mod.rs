#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod rdev_ptt;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use self::linux as imp;
#[cfg(target_os = "macos")]
use self::macos as imp;
#[cfg(target_os = "windows")]
use self::windows as imp;

#[derive(Clone, Copy, Debug)]
pub struct RuntimeCapabilities {
    pub overlay: bool,
    pub global_hotkey: bool,
    pub text_injection: bool,
    pub accessibility_permissions: bool,
    pub sound_playback: bool,
}

impl RuntimeCapabilities {
    fn missing_daemon_features(self) -> Vec<&'static str> {
        let mut missing = Vec::new();

        if !self.overlay {
            missing.push("overlay");
        }
        if !self.global_hotkey {
            missing.push("global hotkey");
        }
        if !self.text_injection {
            missing.push("text injection");
        }
        if !self.accessibility_permissions {
            missing.push("accessibility permission handling");
        }
        if !self.sound_playback {
            missing.push("start/stop sounds");
        }

        missing
    }
}

pub use imp::{
    N_BANDS, default_push_to_talk_key_label, ensure_accessibility_permission_for_daemon_startup,
    has_accessibility_permission, has_microphone_permission, inject_text,
    open_accessibility_system_settings, request_microphone_permission, run_overlay,
};


pub fn runtime_capabilities() -> RuntimeCapabilities {
    imp::runtime_capabilities()
}

pub fn accessibility_help() -> &'static str {
    imp::accessibility_help()
}

pub fn ensure_accessibility_permission() -> anyhow::Result<()> {
    if has_accessibility_permission() {
        Ok(())
    } else {
        anyhow::bail!(accessibility_help())
    }
}

pub fn ensure_accessibility_permission_for_daemon() -> anyhow::Result<()> {
    ensure_accessibility_permission_for_daemon_startup()
}

pub fn clear_permission_state(kind: &str) -> anyhow::Result<()> {
    imp::clear_permission_state(kind)
}

pub fn setup_window(config_path: std::path::PathBuf) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let outcome = imp::run_setup_window(config_path)?;
        if outcome.launch_at_login {
            if let Err(e) = imp::install_launch_agent() {
                tracing::warn!(error = %e, "failed to install launch agent; continuing without it");
            }
        }
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    imp::run_setup_window(config_path)
}

pub fn tutorial_window(config_path: std::path::PathBuf) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    return imp::run_tutorial_window(config_path);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = config_path;
        Ok(())
    }
}

pub fn listen_for_ptt_events(tx: std::sync::mpsc::Sender<bool>) -> anyhow::Result<()> {
    imp::listen_for_ptt_events(tx)
}

pub fn play_start_sound(output_device: Option<cpal::Device>) {
    crate::audio::play_start_sound(output_device);
}

pub fn play_stop_sound(output_device: Option<cpal::Device>) {
    crate::audio::play_stop_sound(output_device);
}

pub fn ensure_daemon_runtime_support() -> anyhow::Result<()> {
    let missing = runtime_capabilities().missing_daemon_features();
    if missing.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "{} daemon runtime is not implemented yet. Missing features: {}.",
        std::env::consts::OS,
        missing.join(", "),
    );
}
