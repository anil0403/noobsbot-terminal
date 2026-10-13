/// The system clipboard (NSPasteboard), plus a stand-in for the X11 primary
/// selection that macOS lacks: the last selection, for middle-click paste.
pub struct Clipboard(Option<arboard::Clipboard>, String);

impl Clipboard {
    pub fn new() -> Self {
        let inner = arboard::Clipboard::new().map_err(|e| log::warn!("clipboard unavailable: {e}")).ok();
        Self(inner, String::new())
    }

    pub fn set(&mut self, primary: bool, text: String) {
        if primary {
            self.1 = text;
        } else if let Some(cb) = self.0.as_mut()
            && let Err(e) = cb.set_text(text)
        {
            log::warn!("clipboard set failed: {e}");
        }
    }

    /// Clipboard and selection both: Cmd+V and middle-click get it.
    pub fn set_both(&mut self, text: String) {
        self.set(true, text.clone());
        self.set(false, text);
    }

    pub fn get(&mut self, primary: bool) -> Option<String> {
        if primary {
            return Some(self.1.clone()).filter(|s| !s.is_empty());
        }
        self.0.as_mut()?.get_text().ok()
    }
}
