//! The Dock icon's right-click menu: "New Window" above the system's items,
//! like Terminal.app.

use objc2::runtime::{AnyObject, Imp, Sel};
use objc2::{class, ffi, msg_send, sel};

/// Adds the menu. Call once the event loop runs: winit sets the app delegate
/// when the loop is built, and the menu's methods are added to that delegate.
pub fn install() {
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
        let new: extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) = new_window;
        ffi::class_addMethod(cls, sel!(applicationDockMenu:), std::mem::transmute::<_, Imp>(menu), c"@@:@".as_ptr());
        ffi::class_addMethod(cls, sel!(noobsbotNewWindow:), std::mem::transmute::<_, Imp>(new), c"v@:@".as_ptr());
        // AppKit reads which methods the delegate has when it is set, so set it again.
        let _: () = msg_send![app, setDelegate: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![app, setDelegate: delegate];
    }
}

/// `applicationDockMenu:`, asked each time the Dock menu opens.
extern "C-unwind" fn dock_menu(this: *mut AnyObject, _: Sel, _app: *mut AnyObject) -> *mut AnyObject {
    // SAFETY: AppKit calls this on the main thread; the menu is returned autoreleased.
    unsafe {
        let menu: *mut AnyObject = msg_send![class!(NSMenu), new];
        let title: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: c"New Window".as_ptr()];
        let key: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: c"".as_ptr()];
        let item: *mut AnyObject = msg_send![menu, addItemWithTitle: title, action: sel!(noobsbotNewWindow:), keyEquivalent: key];
        let _: () = msg_send![item, setTarget: this];
        msg_send![menu, autorelease]
    }
}

/// "New Window": another noobsbot, as Cmd+N opens.
extern "C-unwind" fn new_window(_: *mut AnyObject, _: Sel, _sender: *mut AnyObject) {
    if let Err(e) = crate::restart::launch(&[], None) {
        log::warn!("can't open a window: {e}");
    }
}
