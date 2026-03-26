use cocoa::base::id;
use cocoa::foundation::{NSPoint, NSRect, NSSize};
use objc::{class, msg_send, sel, sel_impl};

pub unsafe fn nsstring(text: &str) -> id {
    let string: id = msg_send![class!(NSString), alloc];
    msg_send![string, initWithBytes: text.as_ptr() length: text.len() encoding: 4_usize]
}

pub fn make_text(parent: id, frame: NSRect, value: &str) -> id {
    unsafe {
        let text: id = msg_send![class!(NSTextField), alloc];
        let text: id = msg_send![text, initWithFrame: frame];
        let _: () = msg_send![text, setEditable: cocoa::base::NO];
        let _: () = msg_send![text, setBezeled: cocoa::base::NO];
        let _: () = msg_send![text, setDrawsBackground: cocoa::base::NO];
        let _: () = msg_send![text, setStringValue: nsstring(value)];
        let _: () = msg_send![parent, addSubview: text];
        text
    }
}

pub fn make_button(parent: id, frame: NSRect, title: &str, enabled: bool) -> id {
    unsafe {
        let button: id = msg_send![class!(NSButton), alloc];
        let button: id = msg_send![button, initWithFrame: frame];
        let _: () = msg_send![button, setTitle: nsstring(title)];
        let _: () = msg_send![button, setBezelStyle: 1_i32];
        let _: () = msg_send![
            button,
            setEnabled: if enabled { cocoa::base::YES } else { cocoa::base::NO }
        ];
        let _: () = msg_send![parent, addSubview: button];
        button
    }
}

pub fn permission_status_row(content: id, name: &str, y: f64) -> (id, id) {
    let label = make_text(
        content,
        NSRect::new(NSPoint::new(20.0, y + 18.0), NSSize::new(180.0, 22.0)),
        name,
    );
    let status = make_text(
        content,
        NSRect::new(NSPoint::new(210.0, y + 18.0), NSSize::new(300.0, 22.0)),
        "[ ] Permission Not Granted",
    );
    (label, status)
}

pub fn permission_row(content: id, name: &str, y: f64) -> (id, id, id) {
    let (label, status) = permission_status_row(content, name, y);
    let button = make_button(
        content,
        NSRect::new(NSPoint::new(450.0, y + 16.0), NSSize::new(60.0, 24.0)),
        "Grant",
        true,
    );
    (label, status, button)
}

pub fn set_permission_state(label: id, granted: bool, _name: &str) {
    unsafe {
        let text = if granted {
            "[OK] Permission Granted"
        } else {
            "[ ] Permission Not Granted"
        };
        let _: () = msg_send![label, setStringValue: nsstring(text)];
        let green: id = msg_send![class!(NSColor), systemGreenColor];
        let red: id = msg_send![class!(NSColor), systemRedColor];
        let _: () = msg_send![label, setTextColor: if granted { green } else { red }];
    }
}
