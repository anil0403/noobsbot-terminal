//! Shell integration: start bash and zsh with our scripts so they report
//! prompts, commands and the directory. Fish does this on its own.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use alacritty_terminal::tty::Shell;

const FILES: [(&str, &str); 3] = [
    ("noobsbot.bash", include_str!("../shell/noobsbot.bash")),
    ("zsh/.zshenv", include_str!("../shell/zsh/.zshenv")),
    ("zsh/noobsbot.zsh", include_str!("../shell/zsh/noobsbot.zsh")),
];

/// The scripts, written out once per run so they match this build.
fn dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
        let dir = base.join("noobsbot/shell");
        for (name, text) in FILES {
            let path = dir.join(name);
            let written = std::fs::create_dir_all(path.parent()?).and_then(|()| std::fs::write(&path, text));
            if let Err(e) = written {
                log::warn!("shell integration off, can't write {}: {e}", path.display());
                return None;
            }
        }
        Some(dir)
    })
    .as_ref()
}

/// How to start `shell` (empty: `$SHELL`) with integration, and env to add.
pub fn login(shell: &str) -> (Option<Shell>, HashMap<String, String>) {
    let mut env = HashMap::new();
    let shell = match shell {
        "" => match std::env::var("SHELL") {
            Ok(s) => s,
            Err(_) => return (None, env), // alacritty asks passwd
        },
        s => s.to_owned(),
    };
    let mut args = Vec::new();
    match (shell.rsplit('/').next(), dir()) {
        (Some("bash"), Some(dir)) => {
            args = vec!["--rcfile".into(), dir.join("noobsbot.bash").to_string_lossy().into_owned()];
        }
        (Some("zsh"), Some(dir)) => {
            if let Ok(old) = std::env::var("ZDOTDIR") {
                env.insert("NOOBSBOT_ZDOTDIR".into(), old);
            }
            env.insert("ZDOTDIR".into(), dir.join("zsh").to_string_lossy().into_owned());
        }
        _ => {}
    }
    (Some(Shell::new(shell, args)), env)
}
