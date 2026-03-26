#![allow(unsafe_op_in_unsafe_fn)]
#![allow(deprecated)]

use crate::os::RuntimeCapabilities;

mod input;
mod menu;
mod objc_util;
mod overlay;
mod permissions;
mod service;
mod setup;
mod state;
mod tutorial;

pub const N_BANDS: usize = 7;

pub fn runtime_capabilities() -> RuntimeCapabilities {
    RuntimeCapabilities {
        overlay: true,
        global_hotkey: true,
        text_injection: true,
        accessibility_permissions: true,
        sound_playback: true,
    }
}

pub use input::{inject_text, listen_for_ptt_events};
pub use overlay::run_overlay;
pub use permissions::{
    accessibility_help, ensure_accessibility_permission_for_daemon_startup,
    has_accessibility_permission, has_microphone_permission, open_accessibility_system_settings,
    request_microphone_permission,
};
pub use service::install_launch_agent;
pub use setup::run_setup_window;
pub(super) use tutorial::run_tutorial_window;

pub fn default_push_to_talk_key_label() -> &'static str {
    "Right Cmd"
}

pub use permissions::clear_permission_state;
