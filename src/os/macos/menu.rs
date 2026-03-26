use std::ffi::CStr;
use std::os::raw::c_char;
use std::process::Command;

use anyhow::{Context, Result};
use cocoa::appkit::NSApp;
use cocoa::base::{id, nil};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

use crate::{config::DevicePrefs, devices::DeviceInventory};

use super::objc_util::nsstring;
use super::service::{install_launch_agent, is_launch_agent_installed, uninstall_launch_agent};
use super::state::{CONFIG_PATH, CONFIG_PREFS, INPUT_SUBMENU, LOGIN_ITEM, OUTPUT_SUBMENU};

pub(crate) unsafe fn install_status_item(inventory: &DeviceInventory, prefs: &DevicePrefs) {
    let status_bar: id = msg_send![class!(NSStatusBar), systemStatusBar];
    let status_item: id = msg_send![status_bar, statusItemWithLength: -1.0_f64];
    let button: id = msg_send![status_item, button];
    let img: id = msg_send![class!(NSImage),
        imageWithSystemSymbolName: nsstring("mic.fill")
        accessibilityDescription: cocoa::base::nil
    ];
    let _: () = msg_send![img, setTemplate: cocoa::base::YES];
    let _: () = msg_send![button, setImage: img];

    let menu: id = msg_send![class!(NSMenu), alloc];
    let menu: id = msg_send![menu, initWithTitle: nsstring("Jabberwok")];

    let handler_class = register_status_item_handler_class();
    let target: id = msg_send![handler_class, new];

    let input_submenu = build_device_submenu(
        "Input Device",
        &inventory.input_names,
        prefs.input.as_deref(),
        target,
        sel!(selectInputDevice:),
    );
    let input_item: id = msg_send![class!(NSMenuItem), alloc];
    let input_item: id = msg_send![
        input_item,
        initWithTitle: nsstring("Input Device")
        action: nil
        keyEquivalent: nsstring("")
    ];
    let _: () = msg_send![input_item, setSubmenu: input_submenu];
    let _: () = msg_send![menu, addItem: input_item];

    let output_submenu = build_device_submenu(
        "Output Device",
        &inventory.output_names,
        prefs.output.as_deref(),
        target,
        sel!(selectOutputDevice:),
    );
    let output_item: id = msg_send![class!(NSMenuItem), alloc];
    let output_item: id = msg_send![
        output_item,
        initWithTitle: nsstring("Output Device")
        action: nil
        keyEquivalent: nsstring("")
    ];
    let _: () = msg_send![output_item, setSubmenu: output_submenu];
    let _: () = msg_send![menu, addItem: output_item];

    let separator: id = msg_send![class!(NSMenuItem), separatorItem];
    let _: () = msg_send![menu, addItem: separator];

    let login_item: id = msg_send![class!(NSMenuItem), alloc];
    let login_item: id = msg_send![
        login_item,
        initWithTitle: nsstring("Launch at Login")
        action: sel!(toggleLaunchAtLogin:)
        keyEquivalent: nsstring("")
    ];
    let _: () = msg_send![login_item, setTarget: target];
    let is_installed = is_launch_agent_installed();
    let _: () = msg_send![login_item, setState: if is_installed { 1_i32 } else { 0_i32 }];
    let _: () = msg_send![menu, addItem: login_item];
    LOGIN_ITEM.with(|slot| *slot.borrow_mut() = login_item);

    let separator: id = msg_send![class!(NSMenuItem), separatorItem];
    let _: () = msg_send![menu, addItem: separator];

    let reset_item: id = msg_send![class!(NSMenuItem), alloc];
    let reset_item: id = msg_send![
        reset_item,
        initWithTitle: nsstring("Reset Jabberwok...")
        action: sel!(resetApp:)
        keyEquivalent: nsstring("")
    ];
    let _: () = msg_send![reset_item, setTarget: target];
    let _: () = msg_send![menu, addItem: reset_item];

    let separator: id = msg_send![class!(NSMenuItem), separatorItem];
    let _: () = msg_send![menu, addItem: separator];

    let quit_item: id = msg_send![class!(NSMenuItem), alloc];
    let quit_item: id = msg_send![
        quit_item,
        initWithTitle: nsstring("Quit")
        action: sel!(quit:)
        keyEquivalent: nsstring("q")
    ];
    let _: () = msg_send![quit_item, setTarget: target];
    let _: () = msg_send![menu, addItem: quit_item];
    let _: () = msg_send![status_item, setMenu: menu];

    INPUT_SUBMENU.with(|slot| *slot.borrow_mut() = input_submenu);
    OUTPUT_SUBMENU.with(|slot| *slot.borrow_mut() = output_submenu);
}

unsafe fn build_device_submenu(
    title: &str,
    devices: &[String],
    selected: Option<&str>,
    target: id,
    action: Sel,
) -> id {
    let menu: id = msg_send![class!(NSMenu), alloc];
    let menu: id = msg_send![menu, initWithTitle: nsstring(title)];

    for name in devices {
        let item: id = msg_send![class!(NSMenuItem), alloc];
        let item: id = msg_send![
            item,
            initWithTitle: nsstring(name)
            action: action
            keyEquivalent: nsstring("")
        ];
        let _: () = msg_send![item, setTarget: target];
        let _: () = msg_send![item, setRepresentedObject: nsstring(name)];
        let check = selected.is_some_and(|current| current == name.as_str());
        let _: () = msg_send![item, setState: if check { 1_i32 } else { 0_i32 }];
        let _: () = msg_send![menu, addItem: item];
    }

    let separator: id = msg_send![class!(NSMenuItem), separatorItem];
    let _: () = msg_send![menu, addItem: separator];
    let default: id = msg_send![class!(NSMenuItem), alloc];
    let default: id = msg_send![
        default,
        initWithTitle: nsstring("Use OS Default")
        action: action
        keyEquivalent: nsstring("")
    ];
    let _: () = msg_send![default, setTarget: target];
    let _: () = msg_send![default, setRepresentedObject: nil];
    let _: () = msg_send![default, setState: if selected.is_none() { 1_i32 } else { 0_i32 }];
    let _: () = msg_send![menu, addItem: default];

    menu
}

fn menu_item_represented_name(item: id) -> Option<String> {
    unsafe {
        let represented_object: id = msg_send![item, representedObject];
        represented_object_name(represented_object)
    }
}

unsafe fn refresh_menu_selection(submenu: id, selected: Option<&str>) {
    let count: usize = msg_send![submenu, numberOfItems];
    for index in 0..count {
        let item: id = msg_send![submenu, itemAtIndex: index];
        let item_name = menu_item_represented_name(item);
        let matches = match selected {
            Some(name) => item_name.as_deref() == Some(name),
            None => item_name.is_none(),
        };
        let _: () = msg_send![item, setState: if matches { 1_i32 } else { 0_i32 }];
    }
}

fn register_status_item_handler_class() -> &'static Class {
    static CLASS: std::sync::OnceLock<&'static Class> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| unsafe {
        let superclass = class!(NSObject);
        let mut decl = ClassDecl::new("JabberwokStatusItemHandler", superclass)
            .expect("JabberwokStatusItemHandler already registered");

        decl.add_method(sel!(quit:), quit_app as extern "C" fn(&Object, Sel, id));
        decl.add_method(
            sel!(selectInputDevice:),
            select_input_device as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(selectOutputDevice:),
            select_output_device as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(resetApp:),
            reset_app as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(toggleLaunchAtLogin:),
            toggle_launch_at_login as extern "C" fn(&Object, Sel, id),
        );

        decl.register()
    })
}

extern "C" fn select_input_device(_this: &Object, _cmd: Sel, sender: id) {
    handle_device_selection(sender, true);
}

extern "C" fn select_output_device(_this: &Object, _cmd: Sel, sender: id) {
    handle_device_selection(sender, false);
}

fn handle_device_selection(sender: id, is_input: bool) {
    let selected = selected_device_name(sender);
    let config_path = CONFIG_PATH.with(|path| path.borrow().clone());
    if config_path.as_os_str().is_empty() {
        tracing::warn!("ignoring device selection: config path is empty");
        return;
    }
    let prefs = CONFIG_PREFS.with(|slot| slot.borrow().clone());
    let selected_for_menu = selected.clone();
    if let Err(e) = crate::config::update_device_preference(
        &config_path,
        prefs.as_ref(),
        is_input,
        selected.clone(),
        "system_menu",
    ) {
        tracing::error!(error = %e, "failed to persist device preference");
        return;
    }

    if is_input {
        let selected = selected.as_deref();
        INPUT_SUBMENU.with(|menu| {
            let menu = *menu.borrow();
            if menu != nil {
                unsafe { refresh_menu_selection(menu, selected) };
            }
        });
    } else {
        let selected = selected_for_menu.as_deref();
        OUTPUT_SUBMENU.with(|menu| {
            let menu = *menu.borrow();
            if menu != nil {
                unsafe { refresh_menu_selection(menu, selected) };
            }
        });
    }
}

fn selected_device_name(sender: id) -> Option<String> {
    unsafe {
        let represented_object: id = msg_send![sender, representedObject];
        represented_object_name(represented_object)
    }
}

fn represented_object_name(represented_object: id) -> Option<String> {
    if represented_object == nil {
        return None;
    }

    unsafe {
        let ptr: *const c_char = msg_send![represented_object, UTF8String];
        if ptr.is_null() {
            return None;
        }
        Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
    }
}

extern "C" fn quit_app(_this: &Object, _cmd: Sel, _sender: id) {
    unsafe {
        let app = NSApp();
        let _: () = msg_send![app, terminate: nil];
    }
}

extern "C" fn reset_app(_this: &Object, _cmd: Sel, _sender: id) {
    if let Err(error) = confirm_reset_and_restart() {
        tracing::error!(%error, "failed to reset local data from status menu");
    }
}

extern "C" fn toggle_launch_at_login(_this: &Object, _cmd: Sel, _sender: id) {
    let result = if is_launch_agent_installed() {
        uninstall_launch_agent()
    } else {
        install_launch_agent()
    };

    if let Err(e) = result {
        tracing::error!(error = %e, "failed to toggle launch-at-login");
        return;
    }

    let now_installed = is_launch_agent_installed();
    LOGIN_ITEM.with(|slot| {
        let item = *slot.borrow();
        if item != nil {
            unsafe {
                let _: () = msg_send![item, setState: if now_installed { 1_i32 } else { 0_i32 }];
            }
        }
    });
}

pub(crate) fn confirm_reset_and_restart() -> Result<()> {
    if !show_reset_confirmation_dialog() {
        return Ok(());
    }

    spawn_reset_helper()?;

    unsafe {
        let app = NSApp();
        let _: () = msg_send![app, terminate: nil];
    }

    Ok(())
}

fn show_reset_confirmation_dialog() -> bool {
    unsafe {
        let alert: id = msg_send![class!(NSAlert), alloc];
        let alert: id = msg_send![alert, init];
        let _: () = msg_send![alert, setMessageText: nsstring("Reset Jabberwok?")];
        let info = "This will remove downloaded models, local config, and logs, reset macOS privacy permissions, then reopen Jabberwok in setup.";
        let _: () = msg_send![alert, setInformativeText: nsstring(info)];
        let _: () = msg_send![alert, addButtonWithTitle: nsstring("Reset and Restart")];
        let _: () = msg_send![alert, addButtonWithTitle: nsstring("Cancel")];
        let response: i64 = msg_send![alert, runModal];
        response == 1000
    }
}

fn spawn_reset_helper() -> Result<()> {
    let exe = std::env::current_exe().context("failed to locate current executable")?;
    let config = crate::config::config_file()?;
    Command::new(exe)
        .arg("reset")
        .arg("--config")
        .arg(config)
        .arg("--wait-for-pid")
        .arg(std::process::id().to_string())
        .arg("--relaunch")
        .spawn()
        .context("failed to launch reset helper")?;
    Ok(())
}
