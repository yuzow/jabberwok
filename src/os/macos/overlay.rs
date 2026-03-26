use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

use cocoa::appkit::{
    NSApp, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
    NSWindowStyleMask,
};
use cocoa::base::{NO, YES, id, nil};
use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

use crate::{config::DevicePrefs, devices::DeviceInventory};

use super::N_BANDS;
use super::menu::install_status_item;
use super::state::{BAND_LEVELS, CONFIG_PATH, CONFIG_PREFS, RECORDING, SMOOTH_BARS};

pub fn run_overlay(
    recording: Arc<AtomicBool>,
    band_levels: Arc<Vec<AtomicU32>>,
    inventory: DeviceInventory,
    prefs: Arc<Mutex<DevicePrefs>>,
    config_path: PathBuf,
) {
    RECORDING.with(|r| *r.borrow_mut() = Some(recording));
    BAND_LEVELS.with(|b| *b.borrow_mut() = Some(band_levels));
    CONFIG_PATH.with(|path| *path.borrow_mut() = config_path);
    CONFIG_PREFS.with(|stored| *stored.borrow_mut() = Some(Arc::clone(&prefs)));

    unsafe {
        let _pool = NSAutoreleasePool::new(nil);

        let app = NSApp();
        app.setActivationPolicy_(
            NSApplicationActivationPolicy::NSApplicationActivationPolicyAccessory,
        );

        let snapshot = prefs
            .lock()
            .map(|guard| (*guard).clone())
            .unwrap_or_default();
        install_status_item(&inventory, &snapshot);

        let window = create_overlay_window();
        let view: id = msg_send![window, contentView];
        let _timer: id = msg_send![
            class!(NSTimer),
            scheduledTimerWithTimeInterval: (1.0_f64 / 30.0)
            target: view
            selector: sel!(timerFired:)
            userInfo: nil
            repeats: YES
        ];

        app.run();
    }
}

unsafe fn create_overlay_window() -> id {
    let screen: id = msg_send![class!(NSScreen), mainScreen];
    let screen_frame: NSRect = msg_send![screen, frame];

    let win_w = 132.0_f64;
    let win_h = 44.0_f64;
    let win_x = (screen_frame.size.width - win_w) / 2.0;
    let win_y = 100.0_f64;

    let frame = NSRect::new(NSPoint::new(win_x, win_y), NSSize::new(win_w, win_h));

    let window: id = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
        frame,
        NSWindowStyleMask::NSBorderlessWindowMask,
        NSBackingStoreType::NSBackingStoreBuffered,
        NO,
    );

    let _: () = msg_send![window, setLevel: 25_i64];
    let _: () = msg_send![window, setOpaque: NO];
    let _: () = msg_send![window, setHasShadow: YES];
    let _: () = msg_send![window, setIgnoresMouseEvents: YES];
    let _: () = msg_send![window, setCollectionBehavior: 209_u64];

    let clear: id = msg_send![class!(NSColor), clearColor];
    let _: () = msg_send![window, setBackgroundColor: clear];

    let view_class = register_view_class();
    let view: id = msg_send![view_class, alloc];
    let view: id = msg_send![view, initWithFrame: frame];
    let _: () = msg_send![window, setContentView: view];

    window
}

fn register_view_class() -> &'static Class {
    static CLASS: std::sync::OnceLock<&'static Class> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| unsafe {
        let superclass = class!(NSView);
        let mut decl = ClassDecl::new("JabberwokOverlayView", superclass)
            .expect("JabberwokOverlayView already registered");

        decl.add_method(
            sel!(drawRect:),
            draw_rect as extern "C" fn(&Object, Sel, NSRect),
        );
        decl.add_method(
            sel!(timerFired:),
            timer_fired as extern "C" fn(&Object, Sel, id),
        );

        decl.register()
    })
}

extern "C" fn timer_fired(this: &Object, _cmd: Sel, _timer: id) {
    let is_recording = RECORDING.with(|r| {
        r.borrow()
            .as_ref()
            .is_some_and(|a| a.load(Ordering::Relaxed))
    });

    unsafe {
        let window: id = msg_send![this, window];
        if window != nil {
            if is_recording {
                let _: () = msg_send![window, orderFrontRegardless];
            } else {
                SMOOTH_BARS.with(|sb| *sb.borrow_mut() = [0.0; N_BANDS]);
                let _: () = msg_send![window, orderOut: nil];
            }
        }
        let _: () = msg_send![this, setNeedsDisplay: YES];
    }
}

extern "C" fn draw_rect(this: &Object, _cmd: Sel, _rect: NSRect) {
    let fracs: [f64; N_BANDS] = SMOOTH_BARS.with(|sb| {
        let mut bars = sb.borrow_mut();
        BAND_LEVELS.with(|bl| {
            if let Some(ref bands) = *bl.borrow() {
                for (i, atom) in bands.iter().enumerate() {
                    let raw = f32::from_bits(atom.load(Ordering::Relaxed)).clamp(0.0, 1.0) as f64;
                    if raw == 0.0 {
                        bars[i] = 0.0;
                    } else {
                        let prev = bars[i];
                        let alpha = if raw > prev { 0.65 } else { 0.12 };
                        bars[i] = (prev + alpha * (raw - prev)).max(0.0);
                    }
                }
            }
        });
        *bars
    });

    unsafe {
        let bounds: NSRect = msg_send![this, bounds];

        let bg: id = msg_send![class!(NSColor),
            colorWithRed: 0.07_f64 green: 0.07_f64 blue: 0.10_f64 alpha: 0.90_f64];
        let _: () = msg_send![bg, setFill];
        let bg_path: id = msg_send![class!(NSBezierPath),
            bezierPathWithRoundedRect: bounds xRadius: 8.0_f64 yRadius: 8.0_f64];
        let _: () = msg_send![bg_path, fill];

        let bar_w = 6.0_f64;
        let gap = 3.0_f64;
        let total_w = N_BANDS as f64 * bar_w + (N_BANDS - 1) as f64 * gap;
        let start_x = (bounds.size.width - total_w) / 2.0;
        let min_h = 2.0_f64;
        let max_h = bounds.size.height - 8.0;

        for (i, &frac) in fracs.iter().enumerate().take(N_BANDS) {
            let frac = frac.clamp(0.0, 1.0);
            let h = min_h + frac * (max_h - min_h);
            let x = start_x + i as f64 * (bar_w + gap);
            let y = (bounds.size.height - h) / 2.0;

            let brightness = 0.35 + frac * 0.65;
            let alpha = 0.4 + frac * 0.6;
            let bar_color: id = msg_send![class!(NSColor),
                colorWithRed: brightness green: brightness blue: brightness alpha: alpha];
            let _: () = msg_send![bar_color, setFill];

            let bar_rect = NSRect::new(NSPoint::new(x, y), NSSize::new(bar_w, h));
            let bar_path: id = msg_send![class!(NSBezierPath),
                bezierPathWithRoundedRect: bar_rect xRadius: 3.0_f64 yRadius: 3.0_f64];
            let _: () = msg_send![bar_path, fill];
        }
    }
}
