//! The Dock icon's right-click menu, above the system's items, like Terminal.app:
//! recent folders, New Window, New Tab, Open Config File, Check for Updates.

use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use objc2::runtime::{AnyObject, Imp, Sel};
use objc2::{class, ffi, msg_send, sel};
use winit::event_loop::EventLoopProxy;

use crate::term::UserEvent;

/// For the items the running window handles (New Tab, Check for Updates).
static PROXY: Mutex<Option<EventLoopProxy<UserEvent>>> = Mutex::new(None);

/// One menu action: what the delegate is sent when the item is picked.
type Action = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);

/// Adds the menu. Call once the event loop runs: winit sets the app delegate
/// when the loop is built, and the menu's methods are added to that delegate.
pub fn install(proxy: EventLoopProxy<UserEvent>) {
    *PROXY.lock().unwrap() = Some(proxy);
    // SAFETY: Objective-C runtime calls on the main thread; each added method
    // matches its type string ("@@:@": returns an object, takes one).
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let delegate: *mut AnyObject = msg_send![app, delegate];
        if delegate.is_null() {
            return;
        }
        let cls = ffi::object_getClass(delegate).cast_mut();
        let menu: extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> *mut AnyObject = dock_menu;
        ffi::class_addMethod(cls, sel!(applicationDockMenu:), std::mem::transmute::<_, Imp>(menu), c"@@:@".as_ptr());
        let actions: [(Sel, Action); 5] = [
            (sel!(noobsbotOpenFolder:), open_folder),
            (sel!(noobsbotNewWindow:), new_window),
            (sel!(noobsbotNewTab:), new_tab),
            (sel!(noobsbotOpenConfig:), open_config),
            (sel!(noobsbotUpdate:), update),
        ];
        for (name, imp) in actions {
            ffi::class_addMethod(cls, name, std::mem::transmute::<Action, Imp>(imp), c"v@:@".as_ptr());
        }
        // AppKit reads which methods the delegate has when it is set, so set it again.
        let _: () = msg_send![app, setDelegate: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![app, setDelegate: delegate];
    }
}

/// An autoreleased NSString.
pub unsafe fn ns_string(text: &str) -> *mut AnyObject {
    let text = CString::new(text).unwrap_or_default();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

/// Adds an item that sends `action` to `target`.
pub unsafe fn add_item(menu: *mut AnyObject, title: &str, action: Sel, target: *mut AnyObject) -> *mut AnyObject {
    unsafe {
        let item: *mut AnyObject = msg_send![menu, addItemWithTitle: ns_string(title), action: action, keyEquivalent: ns_string("")];
        let _: () = msg_send![item, setTarget: target];
        item
    }
}

/// "~/projects/x" for a folder under home.
fn short(dir: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| dir.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => dir.display().to_string(),
    }
}

/// `applicationDockMenu:`, asked each time the Dock menu opens.
extern "C-unwind" fn dock_menu(this: *mut AnyObject, _: Sel, _app: *mut AnyObject) -> *mut AnyObject {
    // SAFETY: AppKit calls this on the main thread; the menu is returned autoreleased.
    unsafe {
        let menu: *mut AnyObject = msg_send![class!(NSMenu), new];
        let folders = crate::recent::list();
        for dir in &folders {
            let item = add_item(menu, &short(dir), sel!(noobsbotOpenFolder:), this);
            let _: () = msg_send![item, setRepresentedObject: ns_string(&dir.to_string_lossy())];
        }
        if !folders.is_empty() {
            let line: *mut AnyObject = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![menu, addItem: line];
        }
        add_item(menu, "New Window", sel!(noobsbotNewWindow:), this);
        add_item(menu, "New Tab", sel!(noobsbotNewTab:), this);
        let line: *mut AnyObject = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![menu, addItem: line];
        add_item(menu, "Open Config File", sel!(noobsbotOpenConfig:), this);
        add_item(menu, "Check for Updates", sel!(noobsbotUpdate:), this);
        msg_send![menu, autorelease]
    }
}

/// A recent folder: a new window there.
extern "C-unwind" fn open_folder(_: *mut AnyObject, _: Sel, sender: *mut AnyObject) {
    // SAFETY: `sender` is the menu item; its represented object is the NSString set above.
    let dir = unsafe {
        let path: *mut AnyObject = msg_send![sender, representedObject];
        let text: *const std::ffi::c_char = if path.is_null() { std::ptr::null() } else { msg_send![path, UTF8String] };
        (!text.is_null()).then(|| PathBuf::from(CStr::from_ptr(text).to_string_lossy().into_owned()))
    };
    if let Err(e) = crate::restart::new_window(dir.as_deref()) {
        log::warn!("can't open a window: {e}");
    }
}

extern "C-unwind" fn new_window(_: *mut AnyObject, _: Sel, _sender: *mut AnyObject) {
    if let Err(e) = crate::restart::new_window(None) {
        log::warn!("can't open a window: {e}");
    }
}

extern "C-unwind" fn new_tab(_: *mut AnyObject, _: Sel, _sender: *mut AnyObject) {
    send(UserEvent::NewTab);
}

extern "C-unwind" fn open_config(_: *mut AnyObject, _: Sel, _sender: *mut AnyObject) {
    if let Err(e) = crate::config::open_in_editor() {
        log::warn!("can't open the config: {e}");
    }
}

/// "Check for Updates": asks GitHub, then says so in a popup. Only a newer
/// release offers to update, in a new tab running `noobsbot --update`.
extern "C-unwind" fn update(_: *mut AnyObject, _: Sel, _sender: *mut AnyObject) {
    std::thread::spawn(|| {
        let ours = env!("CARGO_PKG_VERSION");
        let latest = crate::welcome::latest_release();
        let offer = latest.as_deref().is_some_and(|t| crate::welcome::newer(t, ours));
        let (text, buttons) = match &latest {
            None => ("Noobsbot Terminal can't reach GitHub to check for updates. Try again later.".to_owned(), r#"{"OK"}"#),
            Some(_) if !offer => (format!("Noobsbot Terminal {ours} is the latest version."), r#"{"OK"}"#),
            Some(tag) => (format!("Noobsbot Terminal {tag} is out. You have {ours}.\n\nUpdate now?"), r#"{"Later", "Update"}"#),
        };
        // The text goes in as an argument, not into the script: the tag comes from GitHub.
        let script = format!(
            r#"display dialog (item 1 of argv) buttons {buttons} default button {} with title "Noobsbot Terminal""#,
            if offer { "\"Update\"" } else { "1" }
        );
        let Ok(out) = std::process::Command::new("/usr/bin/osascript").args(["-e", "on run argv", "-e", &script, "-e", "end run", &text]).output() else {
            return;
        };
        if offer && String::from_utf8_lossy(&out.stdout).contains("button returned:Update") {
            send(UserEvent::Update);
        }
    });
}

pub fn send(event: UserEvent) {
    if let Some(proxy) = PROXY.lock().unwrap().as_ref() {
        let _ = proxy.send_event(event);
    }
}
