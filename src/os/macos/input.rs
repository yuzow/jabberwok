use std::os::raw::c_void;
use std::sync::mpsc::Sender;
use std::sync::{LazyLock, Mutex};

use anyhow::{Result, bail};
use cocoa::base::{id, nil};
use cocoa::foundation::NSAutoreleasePool;
static PTT_EVENT_SENDER: LazyLock<Mutex<Option<Sender<bool>>>> = LazyLock::new(|| Mutex::new(None));

type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFRunLoopMode = *const c_void;
type CGEventTapProxy = *mut c_void;
type CGEventRef = *mut c_void;
type CGEventMask = u64;
type CGEventType = u32;
type CGEventField = i32;

const KCG_EVENT_TAP_LOCATION_HID: u32 = 0;
const KCG_HEAD_INSERT_EVENT_TAP: u32 = 0;
const KCG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;
const KCG_EVENT_FLAGS_CHANGED: CGEventType = 12;
const KCG_KEYBOARD_EVENT_KEYCODE: CGEventField = 9;
const KCG_EVENT_FLAG_MASK_COMMAND: u64 = 1 << 20;
const RIGHT_COMMAND_KEYCODE: i64 = 54;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: CGEventMask,
        callback: unsafe extern "C" fn(
            proxy: CGEventTapProxy,
            event_type: CGEventType,
            event: CGEventRef,
            user_info: *mut c_void,
        ) -> CGEventRef,
        user_info: id,
    ) -> CFMachPortRef;
    fn CFMachPortCreateRunLoopSource(
        allocator: id,
        tap: CFMachPortRef,
        order: u64,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFRunLoopMode);
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CFRunLoopRun();
    fn CGEventGetIntegerValueField(event: CGEventRef, field: CGEventField) -> i64;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    static kCFRunLoopCommonModes: CFRunLoopMode;
}

pub fn inject_text(text: &str) -> Result<()> {
    crate::os::ensure_accessibility_permission()?;
    crate::input::inject_text_with_enigo(text)
}

pub fn listen_for_ptt_events(tx: Sender<bool>) -> Result<()> {
    store_ptt_sender(tx);

    unsafe {
        let _pool = NSAutoreleasePool::new(nil);
        let tap = CGEventTapCreate(
            KCG_EVENT_TAP_LOCATION_HID,
            KCG_HEAD_INSERT_EVENT_TAP,
            KCG_EVENT_TAP_OPTION_LISTEN_ONLY,
            1_u64 << KCG_EVENT_FLAGS_CHANGED,
            ptt_event_callback,
            nil,
        );
        if tap.is_null() {
            bail!("failed to create macOS event tap");
        }

        let run_loop_source = CFMachPortCreateRunLoopSource(nil, tap, 0);
        if run_loop_source.is_null() {
            bail!("failed to create macOS event tap run loop source");
        }

        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, run_loop_source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        CFRunLoopRun();
    }

    Ok(())
}

fn store_ptt_sender(tx: Sender<bool>) {
    if let Ok(mut slot) = PTT_EVENT_SENDER.lock() {
        *slot = Some(tx);
    } else {
        tracing::warn!("ptt sender lock is poisoned; replacing listener target was skipped");
    }
}

unsafe extern "C" fn ptt_event_callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    if event_type == KCG_EVENT_FLAGS_CHANGED {
        let keycode = CGEventGetIntegerValueField(event, KCG_KEYBOARD_EVENT_KEYCODE);
        if keycode == RIGHT_COMMAND_KEYCODE
            && let Some(tx) = current_ptt_sender()
        {
            let pressed = (CGEventGetFlags(event) & KCG_EVENT_FLAG_MASK_COMMAND) != 0;
            let _ = tx.send(pressed);
        }
    }

    event
}

fn current_ptt_sender() -> Option<Sender<bool>> {
    PTT_EVENT_SENDER.lock().ok().and_then(|slot| slot.clone())
}
