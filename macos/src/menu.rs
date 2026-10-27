//! The menu bar, like Terminal.app's: the app menu, Shell, Edit, View, Window
//! and Help. Each item runs a palette action and shows its keys (the config's
//! `[keys]` too).
//!
//! The keys are only shown: the menus answer "no key equivalent", so every key
//! still reaches the window first, where the palette, search and link hints
//! get theirs. Only the app menu's own keys (Cmd+Q, Cmd+H, Cmd+,) work through it.

use objc2::runtime::{AnyObject, Bool, Imp, Sel};
use objc2::{class, ffi, msg_send, sel};
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::dock::{add_item, ns_string};
use crate::input::{self, Action};
use crate::pane::Dir;
use crate::term::UserEvent;

const HELP_URL: &str = "https://github.com/noobsbot-team/noobsbot-terminal/tree/main/macos#readme";

/// One row: an action with its title, a line, or a standard AppKit item.
enum Row {
    Do(&'static str, Action),
    Line,
    /// Title, selector, key ("" for none) and modifier mask.
    Std(&'static str, Sel, &'static str, usize),
}

const CMD: usize = 1 << 20;
const OPTION: usize = 1 << 19;
const CTRL: usize = 1 << 18;
const SHIFT: usize = 1 << 17;

/// Builds the menu bar, or builds it again with the current keys.
pub fn install() {
    use Row::*;
    let shell = [
        Do("New Window", Action::NewWindow),
        Do("New Tab", Action::NewTab),
        Line,
        Do("Split Right", Action::SplitRight),
        Do("Split Down", Action::SplitDown),
        Do("Split Left", Action::SplitLeft),
        Do("Split Up", Action::SplitUp),
        Line,
        Do("Broadcast Input to All Panes", Action::Broadcast),
        Line,
        Do("Close Pane", Action::ClosePane),
        Do("Restart Noobsbot Terminal", Action::Restart),
    ];
    let edit = [
        Do("Copy", Action::Copy),
        Do("Paste", Action::Paste),
        Do("Copy Last Command's Output", Action::CopyOutput),
        Do("Select All", Action::SelectAll),
        Line,
        Do("Clear Screen", Action::Clear),
        Line,
        Do("Find…", Action::Search),
        Do("Link Hints", Action::LinkHints),
        Do("Vi Mode", Action::ViMode),
    ];
    let view = [
        Do("Bigger", Action::ZoomIn),
        Do("Smaller", Action::ZoomOut),
        Do("Default Size", Action::ZoomReset),
        Line,
        Do("Zoom Pane", Action::ToggleZoom),
        Do("CRT Effect", Action::ToggleCrt),
        Line,
        Do("Previous Prompt", Action::PrevPrompt),
        Do("Next Prompt", Action::NextPrompt),
        Do("Scroll to Top", Action::ScrollTop),
        Do("Scroll to Bottom", Action::ScrollBottom),
        Line,
        Do("Command Palette…", Action::CommandPalette),
    ];
    let window = [
        Do("Minimize", Action::Minimize),
        Std("Zoom", sel!(performZoom:), "", 0),
        Line,
        Do("Show Previous Tab", Action::PrevTab),
        Do("Show Next Tab", Action::NextTab),
        Line,
        Do("Focus Pane Left", Action::Focus(Dir::Left)),
        Do("Focus Pane Right", Action::Focus(Dir::Right)),
        Do("Focus Pane Up", Action::Focus(Dir::Up)),
        Do("Focus Pane Down", Action::Focus(Dir::Down)),
        Line,
        Std("Bring All to Front", sel!(arrangeInFront:), "", 0),
    ];
    let help = [
        Std("Noobsbot Terminal Help", sel!(noobsbotHelp:), "", 0),
        Do("Keyboard Shortcuts", Action::CommandPalette),
    ];

    // SAFETY: AppKit calls on the main thread; the methods added match their type strings.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let delegate: *mut AnyObject = msg_send![app, delegate];
        if delegate.is_null() {
            return;
        }
        let cls = ffi::object_getClass(delegate).cast_mut();
        let picked: extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) = picked;
        let help_page: extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) = help_page;
        let quit: extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) = quit;
        let no_keys: extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject, *mut AnyObject) -> Bool = no_keys;
        // Already there after the first time: class_addMethod then leaves it.
        ffi::class_addMethod(cls, sel!(noobsbotMenu:), std::mem::transmute::<_, Imp>(picked), c"v@:@".as_ptr());
        ffi::class_addMethod(cls, sel!(noobsbotHelp:), std::mem::transmute::<_, Imp>(help_page), c"v@:@".as_ptr());
        ffi::class_addMethod(cls, sel!(noobsbotQuit:), std::mem::transmute::<_, Imp>(quit), c"v@:@".as_ptr());
        ffi::class_addMethod(cls, sel!(menuHasKeyEquivalent:forEvent:target:action:), std::mem::transmute::<_, Imp>(no_keys), c"B@:@@^@^:".as_ptr());

        let bar: *mut AnyObject = msg_send![class!(NSMenu), new];
        let app_menu = app_menu(delegate);
        add_menu(bar, "Noobsbot Terminal", app_menu);
        for (title, rows) in [("Shell", &shell[..]), ("Edit", &edit[..]), ("View", &view[..]), ("Window", &window[..]), ("Help", &help[..])] {
            let menu = submenu(title, rows, delegate);
            let _: () = msg_send![menu, setDelegate: delegate]; // keys reach the window (above)
            add_menu(bar, title, menu);
            match title {
                "Window" => {
                    let _: () = msg_send![app, setWindowsMenu: menu]; // macOS lists the windows here
                }
                "Help" => {
                    let _: () = msg_send![app, setHelpMenu: menu];
                }
                _ => {}
            }
        }
        let _: () = msg_send![app, setMainMenu: bar];
        let _: () = msg_send![bar, release];
    }
}

/// The app menu: About, Settings, Check for Updates, Services, Hide, Quit.
/// Its keys work through the menu, as in every Mac app.
unsafe fn app_menu(delegate: *mut AnyObject) -> *mut AnyObject {
    unsafe {
        let menu: *mut AnyObject = msg_send![class!(NSMenu), new];
        add_std(menu, "About Noobsbot Terminal", sel!(orderFrontStandardAboutPanel:), "", 0, std::ptr::null_mut());
        add_line(menu);
        add_action(menu, "Settings…", Action::OpenConfig, delegate);
        add_item(menu, "Check for Updates…", sel!(noobsbotUpdate:), delegate);
        add_line(menu);
        let services: *mut AnyObject = msg_send![class!(NSMenu), new];
        let item = add_item(menu, "Services", sel!(noobsbotNothing:), std::ptr::null_mut());
        let _: () = msg_send![item, setAction: std::ptr::null::<u8>()];
        let _: () = msg_send![item, setSubmenu: services];
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setServicesMenu: services];
        let _: () = msg_send![services, release];
        add_line(menu);
        add_std(menu, "Hide Noobsbot Terminal", sel!(hide:), "h", CMD, std::ptr::null_mut());
        add_std(menu, "Hide Others", sel!(hideOtherApplications:), "h", CMD | OPTION, std::ptr::null_mut());
        add_std(menu, "Show All", sel!(unhideAllApplications:), "", 0, std::ptr::null_mut());
        add_line(menu);
        // Ours, not terminate:, so it can ask first while programs still run.
        add_std(menu, "Quit Noobsbot Terminal", sel!(noobsbotQuit:), "q", CMD, delegate);
        menu
    }
}

unsafe fn submenu(title: &str, rows: &[Row], delegate: *mut AnyObject) -> *mut AnyObject {
    unsafe {
        let menu: *mut AnyObject = msg_send![class!(NSMenu), alloc];
        let menu: *mut AnyObject = msg_send![menu, initWithTitle: ns_string(title)];
        for row in rows {
            match row {
                Row::Do(title, action) => {
                    add_action(menu, title, *action, delegate);
                }
                Row::Line => add_line(menu),
                Row::Std(title, sel, key, mask) => {
                    // Our own selectors go to the delegate; AppKit's find their target.
                    let target = if sel.name().to_bytes().starts_with(b"noobsbot") { delegate } else { std::ptr::null_mut() };
                    add_std(menu, title, *sel, key, *mask, target);
                }
            }
        }
        menu
    }
}

/// Puts `menu` in the menu bar under `title`.
unsafe fn add_menu(bar: *mut AnyObject, title: &str, menu: *mut AnyObject) {
    unsafe {
        let item = add_item(bar, title, sel!(noobsbotNothing:), std::ptr::null_mut());
        let _: () = msg_send![item, setAction: std::ptr::null::<u8>()];
        let _: () = msg_send![menu, setTitle: ns_string(title)];
        let _: () = msg_send![item, setSubmenu: menu];
        let _: () = msg_send![menu, release];
    }
}

unsafe fn add_line(menu: *mut AnyObject) {
    unsafe {
        let line: *mut AnyObject = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![menu, addItem: line];
    }
}

unsafe fn add_std(menu: *mut AnyObject, title: &str, action: Sel, key: &str, mask: usize, target: *mut AnyObject) -> *mut AnyObject {
    unsafe {
        let item = add_item(menu, title, action, target);
        let _: () = msg_send![item, setKeyEquivalent: ns_string(key)];
        let _: () = msg_send![item, setKeyEquivalentModifierMask: mask];
        item
    }
}

/// An item that runs `action`; its tag is the action's place in the palette list.
unsafe fn add_action(menu: *mut AnyObject, title: &str, action: Action, delegate: *mut AnyObject) -> *mut AnyObject {
    let Some(tag) = input::actions().iter().position(|e| e.action == action) else { return std::ptr::null_mut() };
    let (key, mask) = action_keys(action).unwrap_or_default();
    unsafe {
        let item = add_std(menu, title, sel!(noobsbotMenu:), &key, mask, delegate);
        let _: () = msg_send![item, setTag: tag as isize];
        item
    }
}

/// The first of the action's keys a menu can show, as AppKit's key and modifier mask.
fn action_keys(action: Action) -> Option<(String, usize)> {
    let entry = input::actions().iter().find(|e| e.action == action)?;
    input::keys(entry).split(" or ").find_map(|spec| {
        let (mods, key) = input::parse(spec)?;
        Some((key_equivalent(&key)?, mask(mods)))
    })
}

fn mask(mods: ModifiersState) -> usize {
    [(ModifiersState::SUPER, CMD), (ModifiersState::ALT, OPTION), (ModifiersState::CONTROL, CTRL), (ModifiersState::SHIFT, SHIFT)]
        .into_iter()
        .filter(|(m, _)| mods.contains(*m))
        .fold(0, |all, (_, bit)| all | bit)
}

/// The key as AppKit writes it: a lowercase character, or a function-key code.
fn key_equivalent(key: &Key) -> Option<String> {
    let named = match key {
        Key::Character(c) => return Some(c.to_lowercase()),
        Key::Named(n) => n,
        _ => return None,
    };
    let code = match named {
        NamedKey::ArrowUp => '\u{F700}',
        NamedKey::ArrowDown => '\u{F701}',
        NamedKey::ArrowLeft => '\u{F702}',
        NamedKey::ArrowRight => '\u{F703}',
        NamedKey::Home => '\u{F729}',
        NamedKey::End => '\u{F72B}',
        NamedKey::PageUp => '\u{F72C}',
        NamedKey::PageDown => '\u{F72D}',
        NamedKey::Enter => '\r',
        NamedKey::Space => ' ',
        NamedKey::Tab => '\t',
        _ => return None,
    };
    Some(code.to_string())
}

/// A menu item was picked: run its action in the window.
extern "C-unwind" fn picked(_: *mut AnyObject, _: Sel, sender: *mut AnyObject) {
    // SAFETY: `sender` is the menu item that was picked.
    let tag: isize = unsafe { msg_send![sender, tag] };
    if let Some(entry) = input::actions().get(tag as usize) {
        crate::dock::send(UserEvent::Menu(entry.action));
    }
}

/// "Quit Noobsbot Terminal" (Cmd+Q): the window closes as from its close button.
extern "C-unwind" fn quit(_: *mut AnyObject, _: Sel, _sender: *mut AnyObject) {
    crate::dock::send(UserEvent::Quit);
}

extern "C-unwind" fn help_page(_: *mut AnyObject, _: Sel, _sender: *mut AnyObject) {
    if let Err(e) = std::process::Command::new("/usr/bin/open").arg(HELP_URL).status() {
        log::warn!("can't open the help page: {e}");
    }
}

/// `menuHasKeyEquivalent:forEvent:target:action:`: no, so keys go to the window.
extern "C-unwind" fn no_keys(_: *mut AnyObject, _: Sel, _menu: *mut AnyObject, _event: *mut AnyObject, _target: *mut AnyObject, _action: *mut AnyObject) -> Bool {
    Bool::NO
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_keys() {
        assert_eq!(action_keys(Action::SplitDown), Some(("d".into(), CMD | SHIFT)));
        assert_eq!(action_keys(Action::Clear), Some(("k".into(), CMD)));
        assert_eq!(action_keys(Action::PrevPrompt), Some(("\u{F700}".into(), CMD)));
        assert_eq!(action_keys(Action::Focus(Dir::Left)), Some(("\u{F702}".into(), CMD | OPTION)));
        assert_eq!(action_keys(Action::ToggleCrt), None); // palette only
    }
}
