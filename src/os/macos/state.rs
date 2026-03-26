use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32},
};

use cocoa::base::{id, nil};

use crate::config::DevicePrefs;

use super::N_BANDS;

thread_local! {
    pub(super) static RECORDING: std::cell::RefCell<Option<Arc<AtomicBool>>> = const { std::cell::RefCell::new(None) };
    pub(super) static BAND_LEVELS: std::cell::RefCell<Option<Arc<Vec<AtomicU32>>>> = const { std::cell::RefCell::new(None) };
    pub(super) static SMOOTH_BARS: std::cell::RefCell<[f64; N_BANDS]> = const { std::cell::RefCell::new([0.0; N_BANDS]) };
    pub(super) static CONFIG_PATH: std::cell::RefCell<PathBuf> = const { std::cell::RefCell::new(PathBuf::new()) };
    pub(super) static CONFIG_PREFS: std::cell::RefCell<Option<Arc<Mutex<DevicePrefs>>>> =
        const { std::cell::RefCell::new(None) };
    pub(super) static INPUT_SUBMENU: std::cell::RefCell<id> = const { std::cell::RefCell::new(nil) };
    pub(super) static OUTPUT_SUBMENU: std::cell::RefCell<id> = const { std::cell::RefCell::new(nil) };
    pub(super) static LOGIN_ITEM: std::cell::RefCell<id> = const { std::cell::RefCell::new(nil) };
}
