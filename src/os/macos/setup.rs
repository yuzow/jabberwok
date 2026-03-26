use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{
    Arc, LazyLock, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub struct SetupOutcome {
    pub launch_at_login: bool,
}

use anyhow::{Context, Result};
use cocoa::appkit::{
    NSApp, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
    NSWindowStyleMask,
};
use cocoa::base::{NO, YES, id, nil};
use cocoa::foundation::{NSAutoreleasePool, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

use super::menu::confirm_reset_and_restart;
use super::objc_util::{
    make_button, make_text, nsstring, permission_row, permission_status_row, set_permission_state,
};
use super::permissions::{
    has_accessibility_permission, has_microphone_permission, open_accessibility_system_settings,
};
use crate::setup::{SetupProgress, run_setup_worker};

static SETUP_RETRY_REQUESTED: LazyLock<Mutex<Option<Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(None));
static SETUP_CONTINUE_REQUESTED: LazyLock<Mutex<Option<Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(None));
static SETUP_LAUNCH_AT_LOGIN: LazyLock<Mutex<Option<Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(None));

struct SetupWindowControls {
    window: id,
    phase: id,
    progress: id,
    model_status: id,
    mic_status: id,
    mic_grant: id,
    acc_status: id,
    acc_grant: id,
    download_button: id,
    launch_button: id,
}

struct SetupActionFlagGuard;

pub fn run_setup_window(config_path: PathBuf) -> Result<SetupOutcome> {
    use std::thread;
    use std::time::Duration;

    let lock_path = crate::config::config_file()?
        .parent()
        .map(|parent| parent.join(".setup.lock"))
        .unwrap_or_else(|| PathBuf::from("/tmp/jabberwok.setup.lock"));
    let lock = setup_single_instance_lock(&lock_path)?;
    let retry_requested = Arc::new(AtomicBool::new(false));
    let continue_requested = Arc::new(AtomicBool::new(false));
    let cancel_requested = Arc::new(AtomicBool::new(false));
    let launch_at_login = Arc::new(AtomicBool::new(true)); // pre-checked
    let _setup_action_flag_guard =
        SetupActionFlagGuard::install(&retry_requested, &continue_requested, &launch_at_login);

    let (tx, rx) = mpsc::channel::<SetupProgress>();
    let worker_path = config_path.clone();
    let worker_retry = Arc::clone(&retry_requested);
    let worker_cancel = Arc::clone(&cancel_requested);
    let _ = thread::spawn(move || {
        run_setup_worker(worker_path, tx, worker_retry, worker_cancel);
    });

    unsafe {
        let _pool = NSAutoreleasePool::new(nil);
        let app = NSApp();
        app.setActivationPolicy_(
            NSApplicationActivationPolicy::NSApplicationActivationPolicyRegular,
        );

        let action_class = setup_action_class();
        let target: id = msg_send![action_class, new];
        let controls = build_setup_window(target);
        let window = controls.window;
        app.activateIgnoringOtherApps_(YES);

        let mut state = SetupProgress {
            percent: 0.0,
            phase: "Checking setup...".to_string(),
            microphone_permission_granted: false,
            accessibility_permission_granted: false,
            ready: false,
            download_failed: false,
            needs_download: false,
            model_installed: false,
            model_name: None,
        };

        loop {
            let visible: bool = msg_send![window, isVisible];
            if !visible {
                cancel_requested.store(true, Ordering::Relaxed);
                return Err(anyhow::anyhow!("setup canceled before completion"));

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
            }

            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(event) => {
                    apply_setup_event(&controls, &event);

                    if event.ready {
                        let _: () = msg_send![controls.phase, setStringValue: nsstring("Ready. Launch Jabberwok when you're ready.")];
                    }

                    state = event;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !state.ready {
                        let microphone_permission_granted = has_microphone_permission();
                        let accessibility_permission_granted = has_accessibility_permission();

                        if microphone_permission_granted != state.microphone_permission_granted {
                            state.microphone_permission_granted = microphone_permission_granted;
                            set_permission_state(
                                controls.mic_status,
                                state.microphone_permission_granted,
                                "microphone",
                            );
                            let mic_label = if state.microphone_permission_granted {
                                "Granted"
                            } else {
                                "Grant"
                            };
                            let _: () =
                                msg_send![controls.mic_grant, setTitle: nsstring(mic_label)];
                            let _: () = msg_send![
                                controls.mic_grant,
                                setEnabled: if state.microphone_permission_granted { NO } else { YES }
                            ];
                        }

                        if accessibility_permission_granted
                            != state.accessibility_permission_granted
                        {
                            state.accessibility_permission_granted =
                                accessibility_permission_granted;
                            set_permission_state(
                                controls.acc_status,
                                state.accessibility_permission_granted,
                                "accessibility",
                            );
                            let _: () = msg_send![
                                controls.acc_grant,
                                setEnabled: if state.accessibility_permission_granted { NO } else { YES }
                            ];
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if !state.ready {
                        return Err(anyhow::anyhow!("setup ended unexpectedly"));
                    }
                }
            }

            if state.ready && continue_requested.swap(false, Ordering::Relaxed) {
                let _: () =
                    msg_send![controls.phase, setStringValue: nsstring("Launching daemon...")];
                break;
            }
        }

        let _: () = msg_send![controls.window, close];
        drop(lock);
    }

    Ok(SetupOutcome {
        launch_at_login: launch_at_login.load(Ordering::Relaxed),
    })
}

impl SetupActionFlagGuard {
    fn install(
        retry: &Arc<AtomicBool>,
        continue_flag: &Arc<AtomicBool>,
        launch_at_login: &Arc<AtomicBool>,
    ) -> Self {
        store_setup_action_flag(&SETUP_RETRY_REQUESTED, Arc::clone(retry));
        store_setup_action_flag(&SETUP_CONTINUE_REQUESTED, Arc::clone(continue_flag));
        store_setup_action_flag(&SETUP_LAUNCH_AT_LOGIN, Arc::clone(launch_at_login));
        Self
    }
}

impl Drop for SetupActionFlagGuard {
    fn drop(&mut self) {
        clear_setup_action_flag(&SETUP_RETRY_REQUESTED);
        clear_setup_action_flag(&SETUP_CONTINUE_REQUESTED);
        clear_setup_action_flag(&SETUP_LAUNCH_AT_LOGIN);
    }
}

fn store_setup_action_flag(slot: &LazyLock<Mutex<Option<Arc<AtomicBool>>>>, flag: Arc<AtomicBool>) {
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(flag);
    } else {
        tracing::warn!("setup action flag lock is poisoned; UI actions may be stale");
    }
}

fn clear_setup_action_flag(slot: &LazyLock<Mutex<Option<Arc<AtomicBool>>>>) {
    if let Ok(mut guard) = slot.lock() {
        *guard = None;
    }
}

fn load_setup_action_flag(
    slot: &LazyLock<Mutex<Option<Arc<AtomicBool>>>>,
) -> Option<Arc<AtomicBool>> {
    slot.lock().ok().and_then(|guard| guard.clone())
}

unsafe fn build_setup_window(action_target: id) -> SetupWindowControls {
    // Window is 40px taller than the original 290 to accommodate the
    // "Launch at Login" checkbox inserted between the permission rows and the
    // Launch button.  All existing rows are shifted up by the same 40px.
    let frame = NSRect::new(NSPoint::new(260.0, 260.0), NSSize::new(520.0, 330.0));
    let window: id = msg_send![
        NSWindow::alloc(nil),
        initWithContentRect: frame
        styleMask: NSWindowStyleMask::NSTitledWindowMask | NSWindowStyleMask::NSClosableWindowMask
        backing: NSBackingStoreType::NSBackingStoreBuffered
        defer: NO
    ];
    let _: () = msg_send![window, setTitle: nsstring("Jabberwok Setup")];
    let _: () = msg_send![window, center];
    let _: () = msg_send![window, makeKeyAndOrderFront: nil];

    let content: id = msg_send![window, contentView];

    // Title — bold, large, centered across the full width
    let title = make_text(
        content,
        NSRect::new(NSPoint::new(0.0, 284.0), NSSize::new(520.0, 32.0)),
        "Jabberwok Setup",
    );
    let title_font: id = msg_send![class!(NSFont), boldSystemFontOfSize: 22.0_f64];
    let _: () = msg_send![title, setFont: title_font];
    let _: () = msg_send![title, setAlignment: 1_i32]; // NSTextAlignmentCenter

    // Phase — centered
    let phase = make_text(
        content,
        NSRect::new(NSPoint::new(0.0, 252.0), NSSize::new(520.0, 24.0)),
        "Checking setup...",
    );
    let _: () = msg_send![phase, setAlignment: 1_i32]; // NSTextAlignmentCenter

    let progress: id = msg_send![class!(NSProgressIndicator), alloc];
    let progress: id = msg_send![
        progress,
        initWithFrame: NSRect::new(NSPoint::new(20.0, 224.0), NSSize::new(480.0, 18.0))
    ];
    let _: () = msg_send![progress, setIndeterminate: NO];
    let _: () = msg_send![progress, setMinValue: 0.0];
    let _: () = msg_send![progress, setMaxValue: 100.0];
    let _: () = msg_send![progress, setDoubleValue: 0.0];
    let _: () = msg_send![content, addSubview: progress];

    // Model row — download button widened so "Downloaded" isn't clipped
    let (model_label, model_status) = permission_status_row(content, "Model", 172.0);
    let download_button = make_button(
        content,
        NSRect::new(NSPoint::new(410.0, 172.0 + 16.0), NSSize::new(100.0, 24.0)),
        "Download",
        false,
    );

    // Microphone row — initialise button to correct granted state immediately
    let (mic_label, mic_status, mic_grant) = permission_row(content, "Microphone", 136.0);
    let mic_granted = has_microphone_permission();
    let _: () =
        msg_send![mic_grant, setTitle: nsstring(if mic_granted { "Granted" } else { "Grant" })];
    let _: () = msg_send![mic_grant, setEnabled: if mic_granted { NO } else { YES }];

    // Accessibility row — initialise button to correct granted state immediately
    let (acc_label, acc_status, acc_grant) = permission_row(content, "Accessibility", 100.0);
    let acc_granted = has_accessibility_permission();
    let _: () =
        msg_send![acc_grant, setTitle: nsstring(if acc_granted { "Granted" } else { "Grant" })];
    let _: () = msg_send![acc_grant, setEnabled: if acc_granted { NO } else { YES }];

    // Launch at Login checkbox — pre-checked
    let login_checkbox: id = msg_send![class!(NSButton), alloc];
    let login_checkbox: id = msg_send![
        login_checkbox,
        initWithFrame: NSRect::new(NSPoint::new(60.0, 68.0), NSSize::new(400.0, 22.0))
    ];
    let _: () = msg_send![login_checkbox, setButtonType: 3_i32]; // NSSwitchButton
    let _: () = msg_send![login_checkbox, setTitle: nsstring("Launch at Login — start automatically when you log in")];
    let _: () = msg_send![login_checkbox, setState: 1_i32]; // NSControlStateValueOn
    let _: () = msg_send![login_checkbox, setTarget: action_target];
    let _: () = msg_send![login_checkbox, setAction: sel!(toggleLaunchAtLogin:)];
    let _: () = msg_send![content, addSubview: login_checkbox];

    // Bottom — Launch on its own centered line
    let launch_button = make_button(
        content,
        NSRect::new(NSPoint::new(170.0, 14.0), NSSize::new(180.0, 42.0)),
        "Launch!",
        false,
    );
    // RegularSquare bezel fills the full frame height; 16pt font looks proportional
    let _: () = msg_send![launch_button, setBezelStyle: 2_i32];
    let launch_font: id = msg_send![class!(NSFont), systemFontOfSize: 16.0_f64];
    let _: () = msg_send![launch_button, setFont: launch_font];

    // Top-right corner — small gear pull-down that hides the Reset debug action
    let debug_popup: id = msg_send![class!(NSPopUpButton), alloc];
    let debug_popup: id = msg_send![
        debug_popup,
        initWithFrame: NSRect::new(NSPoint::new(480.0, 296.0), NSSize::new(26.0, 22.0))
        pullsDown: YES
    ];
    let debug_menu: id = msg_send![debug_popup, menu];
    // First item is displayed as the pull-down button face
    let gear_item: id = msg_send![class!(NSMenuItem), new];
    let _: () = msg_send![gear_item, setTitle: nsstring("⚙")];
    let _: () = msg_send![debug_menu, addItem: gear_item];
    // Reset action hidden inside the menu
    let reset_item: id = msg_send![class!(NSMenuItem), alloc];
    let reset_item: id = msg_send![reset_item,
        initWithTitle: nsstring("Reset Local Data")
        action: sel!(resetSetupData:)
        keyEquivalent: nsstring("")];
    let _: () = msg_send![reset_item, setTarget: action_target];
    let _: () = msg_send![debug_menu, addItem: reset_item];
    let _: () = msg_send![content, addSubview: debug_popup];

    let _: () = msg_send![acc_grant, setTarget: action_target];
    let _: () = msg_send![acc_grant, setAction: sel!(grantAccessibility:)];
    let _: () = msg_send![download_button, setTarget: action_target];
    let _: () = msg_send![download_button, setAction: sel!(retryDownload:)];
    let _: () = msg_send![launch_button, setTarget: action_target];
    let _: () = msg_send![launch_button, setAction: sel!(continueSetup:)];

    let _: () = msg_send![title, setEditable: NO];
    let _: () = msg_send![model_label, setEditable: NO];
    let _: () = msg_send![model_status, setEditable: NO];
    let _: () = msg_send![mic_label, setEditable: NO];
    let _: () = msg_send![mic_status, setEditable: NO];
    let _: () = msg_send![acc_label, setEditable: NO];
    let _: () = msg_send![acc_status, setEditable: NO];

    SetupWindowControls {
        window,
        phase,
        progress,
        model_status,
        mic_status,
        mic_grant,
        acc_status,
        acc_grant,
        download_button,
        launch_button,
    }
}

fn apply_setup_event(controls: &SetupWindowControls, event: &SetupProgress) {
    unsafe {
        let _: () = msg_send![controls.progress, setDoubleValue: event.percent];
        let _: () = msg_send![controls.phase, setStringValue: nsstring(&event.phase)];

        let name = event.model_name.as_deref().unwrap_or("model");
        let model_text;
        let model_ok;
        if event.model_installed {
            model_text = format!("[OK] Installed: {}", name);
            model_ok = true;
        } else if event.download_failed {
            model_text = "[ERR] Download Failed".to_string();
            model_ok = false;
        } else if event.needs_download {
            model_text = "[MISSING] Model Not Installed".to_string();
            model_ok = false;
        } else {
            model_text = format!("[ ~ ] Downloading {}...", name);
            model_ok = false;
        };
        let _: () = msg_send![controls.model_status, setStringValue: nsstring(&model_text)];
        let green: id = msg_send![class!(NSColor), systemGreenColor];
        let red: id = msg_send![class!(NSColor), systemRedColor];
        let gray: id = msg_send![class!(NSColor), systemGrayColor];
        let model_color = if model_ok {
            green
        } else if event.needs_download || event.download_failed {
            red
        } else {
            gray
        };
        let _: () = msg_send![controls.model_status, setTextColor: model_color];

        set_permission_state(
            controls.mic_status,
            event.microphone_permission_granted,
            "microphone",
        );
        let mic_label = if event.microphone_permission_granted {
            "Granted"
        } else {
            "Grant"
        };
        let _: () = msg_send![controls.mic_grant, setTitle: nsstring(mic_label)];
        let _: () = msg_send![
            controls.mic_grant,
            setEnabled: if event.microphone_permission_granted { NO } else { YES }
        ];

        set_permission_state(
            controls.acc_status,
            event.accessibility_permission_granted,
            "accessibility",
        );
        let acc_label = if event.accessibility_permission_granted {
            "Granted"
        } else {
            "Grant"
        };
        let _: () = msg_send![controls.acc_grant, setTitle: nsstring(acc_label)];
        let _: () = msg_send![
            controls.acc_grant,
            setEnabled: if event.accessibility_permission_granted { NO } else { YES }
        ];

        let download_button_enabled = event.needs_download || event.download_failed;
        let _: () = msg_send![
            controls.download_button,
            setEnabled: if download_button_enabled { YES } else { NO }
        ];
        let download_label = if event.model_installed {
            "Downloaded"
        } else if event.download_failed {
            "Retry"
        } else {
            "Download"
        };
        let _: () = msg_send![controls.download_button, setTitle: nsstring(download_label)];
        let _: () = msg_send![
            controls.launch_button,
            setEnabled: if event.ready { YES } else { NO }
        ];
    }
}

fn setup_action_class() -> &'static Class {
    static CLASS: std::sync::OnceLock<&'static Class> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| unsafe {
        let superclass = class!(NSObject);
        let mut decl = ClassDecl::new("JabberwokSetupActionClass", superclass)
            .expect("JabberwokSetupActionClass already registered");
        decl.add_method(
            sel!(grantAccessibility:),
            grant_accessibility as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(retryDownload:),
            retry_download as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(continueSetup:),
            continue_setup as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(resetSetupData:),
            reset_setup_data as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(toggleLaunchAtLogin:),
            toggle_launch_at_login as extern "C" fn(&Object, Sel, id),
        );
        decl.register()
    })
}

extern "C" fn grant_accessibility(_this: &Object, _cmd: Sel, _sender: id) {
    open_accessibility_system_settings();
}

extern "C" fn retry_download(_this: &Object, _cmd: Sel, _sender: id) {
    if let Some(flag) = load_setup_action_flag(&SETUP_RETRY_REQUESTED) {
        flag.store(true, Ordering::Relaxed);
    }
}

extern "C" fn continue_setup(_this: &Object, _cmd: Sel, _sender: id) {
    if let Some(flag) = load_setup_action_flag(&SETUP_CONTINUE_REQUESTED) {
        flag.store(true, Ordering::Relaxed);
    }
}

extern "C" fn reset_setup_data(_this: &Object, _cmd: Sel, _sender: id) {
    if let Err(error) = confirm_reset_and_restart() {
        tracing::error!(%error, "failed to reset local data from setup window");
    }
}

extern "C" fn toggle_launch_at_login(_this: &Object, _cmd: Sel, sender: id) {
    let state: i32 = unsafe { msg_send![sender, state] };
    if let Some(flag) = load_setup_action_flag(&SETUP_LAUNCH_AT_LOGIN) {
        flag.store(state == 1, Ordering::Relaxed);
    }
}

fn setup_single_instance_lock(path: &PathBuf) -> Result<SetupSingleInstanceLock> {
    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .context("failed to open setup lock file")?;

    // Use flock so the OS releases the lock automatically if the process crashes.
    // create_new leaves a stale file that blocks future launches.
    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if ret != 0 {
        return Err(anyhow::anyhow!("another setup window is already running"));
    }

    Ok(SetupSingleInstanceLock {
        path: path.to_path_buf(),
        file: Some(file),
    })
}

struct SetupSingleInstanceLock {
    path: PathBuf,
    file: Option<std::fs::File>,
}

impl Drop for SetupSingleInstanceLock {
    fn drop(&mut self) {
        let _ = self.file.take();
        let _ = std::fs::remove_file(&self.path);
    }
}
