use arboard::{GetExtLinux, LinuxClipboardKind, SetExtLinux};

/// System clipboard plus the X11/Wayland primary selection (middle-click paste).
pub struct Clipboard(Option<arboard::Clipboard>);

fn kind(primary: bool) -> LinuxClipboardKind {
    if primary { LinuxClipboardKind::Primary } else { LinuxClipboardKind::Clipboard }
}

impl Clipboard {
    pub fn new() -> Self {
        let inner = arboard::Clipboard::new().map_err(|e| log::warn!("clipboard unavailable: {e}")).ok();
        Self(inner)
    }

    pub fn set(&mut self, primary: bool, text: String) {
        if let Some(cb) = self.0.as_mut() {
            if let Err(e) = cb.set().clipboard(kind(primary)).text(text) {
                log::warn!("clipboard set failed: {e}");
            }
        }
    }

    pub fn get(&mut self, primary: bool) -> Option<String> {
        self.0.as_mut()?.get().clipboard(kind(primary)).text().ok()
    }
}
