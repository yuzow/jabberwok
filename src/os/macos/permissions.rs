use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait};
use objc::{class, msg_send, sel, sel_impl};

use super::objc_util::nsstring;

const ACCESSIBILITY_PERMISSION_MESSAGE: &str = "Accessibility permission required.\n\
     Open System Settings -> Privacy & Security -> Accessibility\n\
     and enable jabberwok, then try again.";

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

pub fn accessibility_help() -> &'static str {
    ACCESSIBILITY_PERMISSION_MESSAGE
}

pub fn has_accessibility_permission() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub fn ensure_accessibility_permission_for_daemon_startup() -> Result<()> {
    if has_accessibility_permission() {
        return Ok(());
    }
    bail!(ACCESSIBILITY_PERMISSION_MESSAGE);
}

pub fn has_microphone_permission() -> bool {
    let host = cpal::default_host();
    if let Some(device) = host.default_input_device() {
        device.default_input_config().is_ok()
    } else {
        false
    }
}

pub fn request_microphone_permission() {
    if let Err(error) = crate::audio::request_microphone_access() {
        tracing::warn!(%error, "failed to trigger microphone access request");
    }
}

pub fn clear_permission_state(kind: &str) -> Result<()> {
    let bundle_id = "computer.handy.jabberwok";
    if kind == "Accessibility" || kind == "All" {
        let status = std::process::Command::new("tccutil")
            .args(["reset", "Accessibility", bundle_id])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .context("failed to reset Accessibility via tccutil")?;
        tracing::info!(status = %status, "tccutil Accessibility reset");
    }
    if kind == "Microphone" || kind == "All" {
        let status = std::process::Command::new("tccutil")
            .args(["reset", "Microphone", bundle_id])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .context("failed to reset Microphone via tccutil")?;
        tracing::info!(status = %status, "tccutil Microphone reset");
    }
    Ok(())
}

pub fn open_accessibility_system_settings() {
    let urls = [
        "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility",
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
    ];
    unsafe {
        let workspace: cocoa::base::id = msg_send![class!(NSWorkspace), sharedWorkspace];
        for url in urls {
            let ns_string = nsstring(url);
            let ns_url: cocoa::base::id = msg_send![class!(NSURL), URLWithString: ns_string];
            let opened: bool = msg_send![workspace, openURL: ns_url];
            if opened {
                break;
            }
        }
    }
}
