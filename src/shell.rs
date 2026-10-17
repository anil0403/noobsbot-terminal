//! Shell integration: start bash and zsh with our scripts so they report
//! prompts, commands and the directory. Fish does this on its own; it loads
//! our script for the pretty prompt and the ssh TERM fix.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use alacritty_terminal::tty::Shell;

const FILES: [(&str, &str); 4] = [
    ("noobsbot.fish", include_str!("../shell/noobsbot.fish")),
    ("noobsbot.bash", include_str!("../shell/noobsbot.bash")),
    ("zsh/.zshenv", include_str!("../shell/zsh/.zshenv")),
    ("zsh/noobsbot.zsh", include_str!("../shell/zsh/noobsbot.zsh")),
];

/// The scripts, written out once per run so they match this build.
fn dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
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

/// EDITOR=nano when no editor is set (programs inside and file links use it).
/// TERM=noobsbot when its terminfo is installed, or can be (compiled into
/// ~/.terminfo with `tic`). Otherwise TERM stays as alacritty_terminal set it.
pub fn set_term() {
    // Its file in the usual places first: `infocmp` is a process, 140 ms a run.
    let installed = || {
        let quiet = || std::process::Stdio::null();
        terminfo_file().is_some() || Command::new("infocmp").arg("noobsbot").stdout(quiet()).stderr(quiet()).status().is_ok_and(|s| s.success())
    };
    if !installed() {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let src = std::env::temp_dir().join(format!("noobsbot-{}.terminfo", std::process::id()));
        // A new file only: never write through something another user left at that name.
        let new = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&src);
        if new.and_then(|mut f| f.write_all(include_str!("../extra/noobsbot.terminfo").as_bytes())).is_ok() {
            match Command::new("tic").arg("-x").arg(&src).output() {
                Ok(out) if out.status.success() => log::info!("installed the noobsbot terminfo"),
                Ok(out) => log::warn!("tic: {}", String::from_utf8_lossy(&out.stderr).trim()),
                Err(e) => log::info!("no tic ({e}); TERM stays {:?}", std::env::var("TERM")),
            }
            let _ = std::fs::remove_file(&src);
        }
    }
    // SAFETY: called at startup, before any other thread exists.
    unsafe {
        if installed() {
            std::env::set_var("TERM", "noobsbot");
        }
        std::env::set_var("TERM_PROGRAM", "noobsbot");
        std::env::set_var("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        // git, gh, crontab and our file links use nano unless an editor was chosen.
        if std::env::var_os("VISUAL").is_none() && std::env::var_os("EDITOR").is_none() && on_path("nano").is_some() {
            std::env::set_var("EDITOR", "nano");
        }
    }
}

/// The noobsbot terminfo, compiled, in one of the places ncurses looks first
/// ($TERMINFO, ~/.terminfo, the system database).
fn terminfo_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".terminfo"));
    let dirs = std::env::var_os("TERMINFO")
        .map(PathBuf::from)
        .into_iter()
        .chain(home)
        .chain(["/usr/share/terminfo", "/lib/terminfo", "/usr/lib/terminfo", "/etc/terminfo", "/usr/local/share/terminfo"].map(PathBuf::from));
    terminfo_in(dirs)
}

/// The entry in the first of `dirs` holding it: under a directory named for its first
/// letter, in hex (macOS) or as is (Linux).
fn terminfo_in(dirs: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    dirs.into_iter().flat_map(|d| [d.join("6e/noobsbot"), d.join("n/noobsbot")]).find(|p| p.is_file())
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(name)).find(|p| p.is_file())
}

/// The shells we integrate with that are installed, as paths found on $PATH.
pub fn installed() -> Vec<String> {
    ["zsh", "bash", "fish"].iter().filter_map(|name| on_path(name)).map(|p| p.to_string_lossy().into_owned()).collect()
}

/// How to start `shell` (empty: `$SHELL`) with integration, and env to add.
/// `pretty`: the scripts draw noobsbot's prompt instead of the shell's.
pub fn login(shell: &str, pretty: bool) -> (Option<Shell>, HashMap<String, String>) {
    let mut env = HashMap::new();
    if pretty {
        env.insert("NOOBSBOT_PROMPT".into(), "pretty".into());
    }
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
        // Fish marks prompts itself; we add the pretty prompt and the ssh TERM fix.
        (Some("fish"), Some(dir)) => {
            args = vec![
                "--init-command".into(),
                format!("source '{}'", dir.join("noobsbot.fish").display().to_string().replace('\\', "\\\\").replace('\'', "\\'")),
            ];
        }
        _ => {}
    }
    (Some(Shell::new(shell, args)), env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_compiled_terminfo() {
        let dir = std::env::temp_dir().join(format!("noobsbot-terminfo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(terminfo_in([dir.clone()]), None);
        std::fs::create_dir_all(dir.join("6e")).unwrap();
        std::fs::write(dir.join("6e/noobsbot"), "x").unwrap();
        assert_eq!(terminfo_in([PathBuf::from("/nonexistent"), dir.clone()]), Some(dir.join("6e/noobsbot")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
