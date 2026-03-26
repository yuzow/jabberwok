use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32},
};

use anyhow::{Result, bail};
use rdev::Key;

use crate::os::RuntimeCapabilities;
use crate::{config::DevicePrefs, devices::DeviceInventory};

pub const N_BANDS: usize = 7;

const NOT_IMPLEMENTED: &str =
    "Windows runtime support is not implemented yet for overlay, accessibility, or text injection.";

pub fn default_push_to_talk_key() -> Key {
    Key::MetaRight
}

pub fn runtime_capabilities() -> RuntimeCapabilities {
    RuntimeCapabilities {
        overlay: false,
        global_hotkey: true,
        text_injection: false,
        accessibility_permissions: false,
        sound_playback: true,
    }
}

pub fn default_push_to_talk_key_label() -> &'static str {
    "Right Win"
}

pub fn accessibility_help() -> &'static str {
    NOT_IMPLEMENTED
}

pub fn has_accessibility_permission() -> bool {
    false
}

pub fn has_microphone_permission() -> bool {
    false
}

pub fn request_microphone_permission() {}

pub fn open_accessibility_system_settings() {}

pub fn open_microphone_system_settings() {}

pub fn clear_permission_state(_kind: &str) -> Result<()> {
    bail!(NOT_IMPLEMENTED)
}

pub fn ensure_accessibility_permission_for_daemon_startup() -> Result<()> {
    bail!(NOT_IMPLEMENTED)
}

pub fn inject_text(_text: &str) -> Result<()> {
    bail!(NOT_IMPLEMENTED)
}

pub fn run_overlay(
    _recording: Arc<AtomicBool>,
    _band_levels: Arc<Vec<AtomicU32>>,
    _inventory: DeviceInventory,
    _prefs: Arc<Mutex<DevicePrefs>>,
    _config_path: PathBuf,
) {
    panic!("{NOT_IMPLEMENTED}");
}

pub fn run_setup_window(_config_path: PathBuf) -> Result<()> {
    bail!(NOT_IMPLEMENTED)
}

pub fn listen_for_ptt_events(tx: Sender<bool>) -> Result<()> {
    super::rdev_ptt::listen_for_ptt_events(tx, default_push_to_talk_key())
}
