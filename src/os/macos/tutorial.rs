use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use anyhow::Result;
use cocoa::appkit::{
    NSApp, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
    NSWindowStyleMask,
};
use cocoa::base::{NO, YES, id, nil};
use cocoa::foundation::{NSAutoreleasePool, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

use super::objc_util::{make_button, make_text, nsstring};

const KEY_IMAGE_DATA: &[u8] =
    include_bytes!("../../../xtask/assets/macos/right_command_key.png");

const ACTION_NONE: u8 = 0;
const ACTION_NEXT: u8 = 1;
const ACTION_BACK: u8 = 2;
const ACTION_DONE: u8 = 3;

static TUTORIAL_ACTION: LazyLock<AtomicU8> = LazyLock::new(|| AtomicU8::new(ACTION_NONE));

pub fn run_tutorial_window(config_path: PathBuf) -> Result<()> {
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);
        let app = NSApp();
        app.setActivationPolicy_(
            NSApplicationActivationPolicy::NSApplicationActivationPolicyRegular,
        );

        let action_class = tutorial_action_class();
        let target: id = msg_send![action_class, new];

        let ctrl = build_tutorial_window(target);

        app.activateIgnoringOtherApps_(YES);
        let _: () = msg_send![ctrl.window, makeFirstResponder: ctrl.screen1_text];

        let mut on_screen2 = false;
        let mut on_screen3 = false;

        loop {
            let visible: bool = msg_send![ctrl.window, isVisible];
            if !visible {
                break;
            }

            let event: id = msg_send![
                app,
                nextEventMatchingMask: u64::MAX
                untilDate: nil
                inMode: NSDefaultRunLoopMode
                dequeue: YES
            ];
            if event != nil {
                let _: () = msg_send![app, sendEvent: event];
            } else {
                std::thread::sleep(Duration::from_millis(16));
            }

            // Enable/disable Next based on screen1 text field content.
            let s1_val: id = msg_send![ctrl.screen1_text, stringValue];
            let s1_len: usize = msg_send![s1_val, length];
            let _: () = msg_send![ctrl.next_btn, setEnabled: (s1_len > 0)];

            // Enable/disable Next on screen2 when all three boxes have content.
            let b1_val: id = msg_send![ctrl.box1, stringValue];
            let b1_len: usize = msg_send![b1_val, length];
            let b2_val: id = msg_send![ctrl.box2, stringValue];
            let b2_len: usize = msg_send![b2_val, length];
            let b3_val: id = msg_send![ctrl.box3, stringValue];
            let b3_len: usize = msg_send![b3_val, length];
            let _: () = msg_send![ctrl.next_btn2, setEnabled: (b1_len > 0 && b2_len > 0 && b3_len > 0)];

            let action = TUTORIAL_ACTION.swap(ACTION_NONE, Ordering::Relaxed);
            match action {
                ACTION_NEXT if !on_screen2 && !on_screen3 => {
                    let _: () = msg_send![ctrl.screen1, setHidden: YES];
                    let _: () = msg_send![ctrl.screen2, setHidden: NO];
                    on_screen2 = true;
                }
                ACTION_NEXT if on_screen2 => {
                    let _: () = msg_send![ctrl.screen2, setHidden: YES];
                    let _: () = msg_send![ctrl.screen3, setHidden: NO];
                    on_screen2 = false;
                    on_screen3 = true;
                }
                ACTION_BACK if on_screen2 => {
                    let _: () = msg_send![ctrl.screen2, setHidden: YES];
                    let _: () = msg_send![ctrl.screen1, setHidden: NO];
                    let _: () = msg_send![ctrl.window, makeFirstResponder: ctrl.screen1_text];
                    on_screen2 = false;
                }
                ACTION_BACK if on_screen3 => {
                    let _: () = msg_send![ctrl.screen3, setHidden: YES];
                    let _: () = msg_send![ctrl.screen2, setHidden: NO];
                    on_screen3 = false;
                    on_screen2 = true;
                }
                ACTION_DONE => {
                    break;
                }
                _ => {}
            }
        }

        let _: () = msg_send![ctrl.window, close];
    }

    // Mark tutorial as seen — best effort, failure is non-fatal.
    if let Ok(mut cfg) = crate::config::JabberwokConfig::load(&config_path) {
        cfg.tutorial.has_seen_tutorial = true;
        if let Err(e) = cfg.save(&config_path) {
            tracing::warn!(error = %e, "failed to save has_seen_tutorial; tutorial may repeat on next launch");
        }
    }

    Ok(())
}

struct TutorialControls {
    window: id,
    screen1: id,
    screen2: id,
    screen3: id,
    screen1_text: id,
    next_btn: id,   // screen 1 → screen 2
    box1: id,
    box2: id,
    box3: id,
    next_btn2: id,  // screen 2 → screen 3 (enabled when all boxes have content)
}

unsafe fn build_tutorial_window(target: id) -> TutorialControls {
    let frame = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(640.0, 520.0));
    let window: id = msg_send![
        NSWindow::alloc(nil),
        initWithContentRect: frame
        styleMask: NSWindowStyleMask::NSTitledWindowMask | NSWindowStyleMask::NSClosableWindowMask
        backing: NSBackingStoreType::NSBackingStoreBuffered
        defer: NO
    ];
    let _: () = msg_send![window, setTitle: nsstring("Jabberwok Tutorial")];
    let _: () = msg_send![window, center];
    let _: () = msg_send![window, makeKeyAndOrderFront: nil];

    let content: id = msg_send![window, contentView];

    // Screen 1 container — visible on open
    let screen1: id = msg_send![class!(NSView), alloc];
    let screen1: id = msg_send![
        screen1,
        initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(640.0, 520.0))
    ];
    let _: () = msg_send![content, addSubview: screen1];
    let (screen1_text, next_btn) = build_screen1(screen1, target);

    // Screen 2 container — hidden until Next is clicked from screen 1
    let screen2: id = msg_send![class!(NSView), alloc];
    let screen2: id = msg_send![
        screen2,
        initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(640.0, 520.0))
    ];
    let _: () = msg_send![screen2, setHidden: YES];
    let _: () = msg_send![content, addSubview: screen2];
    let (box1, box2, box3, next_btn2) = build_screen2(screen2, target);

    // Screen 3 container — hidden until Next is clicked from screen 2
    let screen3: id = msg_send![class!(NSView), alloc];
    let screen3: id = msg_send![
        screen3,
        initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(640.0, 520.0))
    ];
    let _: () = msg_send![screen3, setHidden: YES];
    let _: () = msg_send![content, addSubview: screen3];
    build_screen3(screen3, target);

    TutorialControls { window, screen1, screen2, screen3, screen1_text, next_btn, box1, box2, box3, next_btn2 }
}

// ---------------------------------------------------------------------------
// Screen 1 — basic usage demo
// ---------------------------------------------------------------------------
//
// Layout (y coordinates are from the bottom of the 640×520 content view):
//
//   y=464  Title "Welcome to Jabberwok"          h=44
//   y=400  Instruction text (2 lines)            h=56
//   y=150  Key image                             h=235  w=580  x=30
//   y= 65  Editable text box                     h=70   w=600  x=20
//   y= 14  Buttons (Skip left, Next right)       h=36

unsafe fn build_screen1(parent: id, target: id) -> (id, id) {
    // Title
    let title = make_text(
        parent,
        NSRect::new(NSPoint::new(0.0, 464.0), NSSize::new(640.0, 44.0)),
        "Welcome to Jabberwok",
    );
    let title_font: id = msg_send![class!(NSFont), boldSystemFontOfSize: 24.0_f64];
    let _: () = msg_send![title, setFont: title_font];
    let _: () = msg_send![title, setAlignment: 1_i32]; // NSTextAlignmentCenter

    // Instructions
    let instr = make_multiline_label(
        parent,
        NSRect::new(NSPoint::new(20.0, 400.0), NSSize::new(600.0, 56.0)),
        "Hold the Right \u{2318} Command key, say \u{201c}Testing 1, 2, 3\u{201d}, then release.\nYour words will appear wherever your cursor is.",
        16.0,
    );
    let _: () = msg_send![instr, setAlignment: 1_i32]; // centered

    // Key image — large, centred, preserves aspect ratio (580/235 ≈ 2.47)
    make_key_image_view(
        parent,
        NSRect::new(NSPoint::new(30.0, 150.0), NSSize::new(580.0, 235.0)),
    );

    // Small editable text field — just enough room for a short phrase
    let text_field = make_multiline_text_field(
        parent,
        NSRect::new(NSPoint::new(20.0, 65.0), NSSize::new(600.0, 70.0)),
        "Your transcribed text will appear here\u{2026}",
        16.0,
    );

    // Skip Tutorial (left)
    let skip = make_button(
        parent,
        NSRect::new(NSPoint::new(20.0, 14.0), NSSize::new(160.0, 36.0)),
        "Skip Tutorial",
        true,
    );
    let _: () = msg_send![skip, setTarget: target];
    let _: () = msg_send![skip, setAction: sel!(tutorialDone:)];

    // Next → (right) — disabled until the text box has content
    let next = make_button(
        parent,
        NSRect::new(NSPoint::new(460.0, 14.0), NSSize::new(160.0, 36.0)),
        "Next \u{2192}",
        false,
    );
    let _: () = msg_send![next, setTarget: target];
    let _: () = msg_send![next, setAction: sel!(tutorialNext:)];

    (text_field, next)
}

// ---------------------------------------------------------------------------
// Screen 2 — multi-target practice (2×2 grid)
// ---------------------------------------------------------------------------
//
// Layout (y from bottom):
//
//   y=468  Title "Try it three ways"             h=40
//   Top row:    y=258, h=200
//     x=20..305   Instructions panel (top-left)
//     x=325..620  Text box 1 (top-right)
//   Bottom row: y=58, h=190
//     x=20..305   Text box 2 (bottom-left)
//     x=325..620  Text box 3 (bottom-right)
//   y= 10  Buttons (Back, Skip, Done)            h=36

unsafe fn build_screen2(parent: id, target: id) -> (id, id, id, id) {
    // Title
    let title = make_text(
        parent,
        NSRect::new(NSPoint::new(0.0, 468.0), NSSize::new(640.0, 40.0)),
        "Try it three ways",
    );
    let title_font: id = msg_send![class!(NSFont), boldSystemFontOfSize: 20.0_f64];
    let _: () = msg_send![title, setFont: title_font];
    let _: () = msg_send![title, setAlignment: 1_i32];

    // Top-left: dark instruction panel (NSBoxCustom, white text)
    // Panel occupies (20, 258, 285, 200) in parent coords.
    // Content inside is positioned in the panel's local coordinate space.
    let panel: id = msg_send![class!(NSBox), alloc];
    let panel: id = msg_send![
        panel,
        initWithFrame: NSRect::new(NSPoint::new(20.0, 258.0), NSSize::new(285.0, 200.0))
    ];
    let _: () = msg_send![panel, setBoxType: 4_i32];     // NSBoxCustom
    let _: () = msg_send![panel, setTitlePosition: 0_i32]; // NSNoTitle
    let _: () = msg_send![panel, setBorderWidth: 0.0_f64];
    let dark: id = msg_send![class!(NSColor), colorWithWhite: 0.14_f64 alpha: 1.0_f64];
    let _: () = msg_send![panel, setFillColor: dark];
    let _: () = msg_send![parent, addSubview: panel];
    let cv: id = msg_send![panel, contentView]; // local origin = bottom-left of panel

    // Instruction text — lower portion of panel, white text
    let instr = make_multiline_label(
        cv,
        NSRect::new(NSPoint::new(10.0, 8.0), NSSize::new(265.0, 178.0)),
        "Click one of the text boxes, then hold Right \u{2318} and say a phrase.\n\nTry all three to see how Jabberwok follows your cursor.",
        17.0,
    );
    let white: id = msg_send![class!(NSColor), whiteColor];
    let _: () = msg_send![instr, setTextColor: white];

    // Top-right: label + text box 1
    make_say_label(
        parent,
        NSRect::new(NSPoint::new(325.0, 422.0), NSSize::new(295.0, 36.0)),
        "How now brown cow",
    );
    let box1 = make_multiline_text_field(
        parent,
        NSRect::new(NSPoint::new(325.0, 258.0), NSSize::new(295.0, 160.0)),
        "Transcribed text will appear here\u{2026}",
        14.0,
    );

    // Bottom-left: label + text box 2
    make_say_label(
        parent,
        NSRect::new(NSPoint::new(20.0, 208.0), NSSize::new(295.0, 50.0)),
        "Sally sells seashells by the sea shore",
    );
    let box2 = make_multiline_text_field(
        parent,
        NSRect::new(NSPoint::new(20.0, 58.0), NSSize::new(295.0, 146.0)),
        "Transcribed text will appear here\u{2026}",
        14.0,
    );

    // Bottom-right: label + text box 3
    make_say_label(
        parent,
        NSRect::new(NSPoint::new(325.0, 208.0), NSSize::new(295.0, 50.0)),
        "The quick brown fox jumped over the lazy dog",
    );
    let box3 = make_multiline_text_field(
        parent,
        NSRect::new(NSPoint::new(325.0, 58.0), NSSize::new(295.0, 146.0)),
        "Transcribed text will appear here\u{2026}",
        14.0,
    );

    // ← Back (left)
    let back = make_button(
        parent,
        NSRect::new(NSPoint::new(20.0, 10.0), NSSize::new(120.0, 36.0)),
        "\u{2190} Back",
        true,
    );
    let _: () = msg_send![back, setTarget: target];
    let _: () = msg_send![back, setAction: sel!(tutorialBack:)];

    // Skip Tutorial (centre-left)
    let skip = make_button(
        parent,
        NSRect::new(NSPoint::new(160.0, 10.0), NSSize::new(150.0, 36.0)),
        "Skip Tutorial",
        true,
    );
    let _: () = msg_send![skip, setTarget: target];
    let _: () = msg_send![skip, setAction: sel!(tutorialDone:)];

    // Next → (right) — disabled until all three text boxes have content
    let next = make_button(
        parent,
        NSRect::new(NSPoint::new(460.0, 10.0), NSSize::new(160.0, 36.0)),
        "Next \u{2192}",
        false,
    );
    let _: () = msg_send![next, setTarget: target];
    let _: () = msg_send![next, setAction: sel!(tutorialNext:)];

    (box1, box2, box3, next)
}

// ---------------------------------------------------------------------------
// Screen 3 — wrap-up / congratulations
// ---------------------------------------------------------------------------
//
// Layout (y from bottom of the 640×520 content view), ~42px gaps throughout:
//
//   y=400  Title "Good job — now you're talking!"  h=60  bold 24pt  centered
//   y=260  Body text                               h=100 16pt  centered
//   y=182  Tip 1                                   h=36  16pt  secondary
//   y= 92  Tip 2                                   h=48  16pt  secondary
//   y= 14  Buttons: ← Back (left), Done (right)   h=36

unsafe fn build_screen3(parent: id, target: id) {
    // Title
    let title = make_text(
        parent,
        NSRect::new(NSPoint::new(0.0, 400.0), NSSize::new(640.0, 60.0)),
        "Good job \u{2014} now you\u{2019}re talking!",
    );
    let title_font: id = msg_send![class!(NSFont), boldSystemFontOfSize: 24.0_f64];
    let _: () = msg_send![title, setFont: title_font];
    let _: () = msg_send![title, setAlignment: 1_i32]; // NSTextAlignmentCenter

    // Body
    let body = make_multiline_label(
        parent,
        NSRect::new(NSPoint::new(40.0, 260.0), NSSize::new(560.0, 100.0)),
        "Use Jabberwok anywhere you\u{2019}d normally type \u{2014} click into any window, hold Right \u{2318}, say what you want, and release.",
        16.0,
    );
    let _: () = msg_send![body, setAlignment: 1_i32]; // centered

    // Tip 1
    let tip1 = make_multiline_label(
        parent,
        NSRect::new(NSPoint::new(60.0, 182.0), NSSize::new(520.0, 36.0)),
        "\u{2022}  Speak naturally \u{2014} pauses are fine.",
        16.0,
    );
    let color: id = msg_send![class!(NSColor), secondaryLabelColor];
    let _: () = msg_send![tip1, setTextColor: color];

    // Tip 2
    let tip2 = make_multiline_label(
        parent,
        NSRect::new(NSPoint::new(60.0, 92.0), NSSize::new(520.0, 48.0)),
        "\u{2022}  Works in any focused text field: messages, notes, browsers, editors, and more.",
        16.0,
    );
    let color: id = msg_send![class!(NSColor), secondaryLabelColor];
    let _: () = msg_send![tip2, setTextColor: color];

    // ← Back (left)
    let back = make_button(
        parent,
        NSRect::new(NSPoint::new(20.0, 14.0), NSSize::new(120.0, 36.0)),
        "\u{2190} Back",
        true,
    );
    let _: () = msg_send![back, setTarget: target];
    let _: () = msg_send![back, setAction: sel!(tutorialBack:)];

    // Done (right) — always enabled
    let done = make_button(
        parent,
        NSRect::new(NSPoint::new(460.0, 14.0), NSSize::new(160.0, 36.0)),
        "Done",
        false,
    );
    let _: () = msg_send![done, setTarget: target];
    let _: () = msg_send![done, setAction: sel!(tutorialDone:)];
    let _: () = msg_send![done, setEnabled: YES];
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Small "Say: "phrase"" label rendered above a practice text box.
unsafe fn make_say_label(parent: id, frame: NSRect, phrase: &str) {
    let text = format!("Say: \"{phrase}\"");
    let label = make_multiline_label(parent, frame, &text, 17.0);
    let color: id = msg_send![class!(NSColor), secondaryLabelColor];
    let _: () = msg_send![label, setTextColor: color];
}

/// NSImageView loaded from the embedded PNG bytes, scaled to fit the frame.
unsafe fn make_key_image_view(parent: id, frame: NSRect) -> id {
    let data: id = msg_send![
        class!(NSData),
        dataWithBytes: KEY_IMAGE_DATA.as_ptr()
        length: KEY_IMAGE_DATA.len()
    ];
    let image: id = msg_send![class!(NSImage), alloc];
    let image: id = msg_send![image, initWithData: data];
    let iv: id = msg_send![class!(NSImageView), alloc];
    let iv: id = msg_send![iv, initWithFrame: frame];
    let _: () = msg_send![iv, setImage: image];
    // NSImageScaleProportionallyUpOrDown = 3
    let _: () = msg_send![iv, setImageScaling: 3_usize];
    let _: () = msg_send![parent, addSubview: iv];
    iv
}

/// Non-editable, no-bezel label with word-wrap enabled. Use for multi-line
/// instruction text where `make_text` from objc_util would clip the content.
unsafe fn make_multiline_label(parent: id, frame: NSRect, text: &str, font_size: f64) -> id {
    let field: id = msg_send![class!(NSTextField), alloc];
    let field: id = msg_send![field, initWithFrame: frame];
    let _: () = msg_send![field, setEditable: NO];
    let _: () = msg_send![field, setBezeled: NO];
    let _: () = msg_send![field, setDrawsBackground: NO];
    let _: () = msg_send![field, setStringValue: nsstring(text)];
    let cell: id = msg_send![field, cell];
    let _: () = msg_send![cell, setWraps: YES];
    let _: () = msg_send![cell, setScrollable: NO];
    let font: id = msg_send![class!(NSFont), systemFontOfSize: font_size];
    let _: () = msg_send![field, setFont: font];
    let _: () = msg_send![parent, addSubview: field];
    field
}

/// Editable, bordered, multi-line text field with placeholder text. Used for
/// the practice boxes that receive transcribed text from the running daemon.
unsafe fn make_multiline_text_field(
    parent: id,
    frame: NSRect,
    placeholder: &str,
    font_size: f64,
) -> id {
    let field: id = msg_send![class!(NSTextField), alloc];
    let field: id = msg_send![field, initWithFrame: frame];
    let _: () = msg_send![field, setEditable: YES];
    let _: () = msg_send![field, setBezeled: YES];
    let _: () = msg_send![field, setDrawsBackground: YES];
    let cell: id = msg_send![field, cell];
    let _: () = msg_send![cell, setWraps: YES];
    let _: () = msg_send![cell, setScrollable: NO];
    let _: () = msg_send![field, setPlaceholderString: nsstring(placeholder)];
    let font: id = msg_send![class!(NSFont), systemFontOfSize: font_size];
    let _: () = msg_send![field, setFont: font];
    let _: () = msg_send![parent, addSubview: field];
    field
}

// ---------------------------------------------------------------------------
// ObjC action class
// ---------------------------------------------------------------------------

fn tutorial_action_class() -> &'static Class {
    static CLASS: std::sync::OnceLock<&'static Class> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| unsafe {
        let superclass = class!(NSObject);
        let mut decl = ClassDecl::new("JabberwokTutorialActionClass", superclass)
            .expect("JabberwokTutorialActionClass already registered");
        decl.add_method(
            sel!(tutorialNext:),
            tutorial_next as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(tutorialBack:),
            tutorial_back as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(tutorialDone:),
            tutorial_done as extern "C" fn(&Object, Sel, id),
        );
        decl.register()
    })
}

extern "C" fn tutorial_next(_this: &Object, _cmd: Sel, _sender: id) {
    TUTORIAL_ACTION.store(ACTION_NEXT, Ordering::Relaxed);
}

extern "C" fn tutorial_back(_this: &Object, _cmd: Sel, _sender: id) {
    TUTORIAL_ACTION.store(ACTION_BACK, Ordering::Relaxed);
}

extern "C" fn tutorial_done(_this: &Object, _cmd: Sel, _sender: id) {
    TUTORIAL_ACTION.store(ACTION_DONE, Ordering::Relaxed);
}
