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
    // Opened from Finder or the Dock: in /, with no locale. Start at home with a
    // UTF-8 locale, as Terminal.app does.
    if std::env::current_dir().is_ok_and(|d| d == std::path::Path::new("/"))
        && let Some(home) = std::env::var_os("HOME")
    {
        let _ = std::env::set_current_dir(home);
    }
    if ["LC_ALL", "LC_CTYPE", "LANG"].iter().all(|v| std::env::var_os(v).is_none()) {
        // SAFETY: called at startup, before any other thread exists.
        unsafe { std::env::set_var("LANG", locale()) };
    }
    let installed = || {
        let quiet = || std::process::Stdio::null();
        Command::new("infocmp").arg("noobsbot").stdout(quiet()).stderr(quiet()).status().is_ok_and(|s| s.success())
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

/// The user's region as a UTF-8 locale ("en_GB.UTF-8"), if the system has it.
fn locale() -> String {
    let out = Command::new("/usr/bin/defaults").args(["read", "-g", "AppleLocale"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let name = format!("{}.UTF-8", out.unwrap_or_default().trim().split('@').next().unwrap_or(""));
    if name.len() > 6 && std::path::Path::new("/usr/share/locale").join(&name).is_dir() { name } else { "en_US.UTF-8".into() }
}

/// `name` on $PATH, or where Homebrew and MacPorts put it: an app opened from
/// Finder gets only the system's PATH.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let extra = ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from);
    std::env::split_paths(&path).chain(extra).map(|dir| dir.join(name)).find(|p| p.is_file())
}

/// Your login shell: $SHELL, else the user database's, else zsh (macOS's default).
pub fn user_shell() -> String {
    if let Ok(s) = std::env::var("SHELL")
        && !s.is_empty()
    {
        return s;
    }
    // SAFETY: getpwuid's result is read right away, on the main thread.
    let from_passwd = unsafe {
        let pw = libc::getpwuid(libc::getuid());
        (!pw.is_null() && !(*pw).pw_shell.is_null()).then(|| std::ffi::CStr::from_ptr((*pw).pw_shell).to_string_lossy().into_owned())
    };
    from_passwd.filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into())
}

/// The shells we integrate with that are installed, as paths found on $PATH.
pub fn installed() -> Vec<String> {
    ["zsh", "bash", "fish"].iter().filter_map(|name| on_path(name)).map(|p| p.to_string_lossy().into_owned()).collect()
}

/// How to start `shell` (empty: your login shell) with integration, and env to add.
/// `pretty`: the scripts draw noobsbot's prompt instead of the shell's.
/// Login shells, as Terminal.app and iTerm start them: that's where macOS
/// (path_helper) and Homebrew set up PATH.
pub fn login(shell: &str, pretty: bool) -> (Option<Shell>, HashMap<String, String>) {
    let mut env = HashMap::new();
    if pretty {
        env.insert("NOOBSBOT_PROMPT".into(), "pretty".into());
    }
    let shell = match shell {
        "" => user_shell(),
        s => s.to_owned(),
    };
    let mut args = vec!["-l".to_owned()];
    match (shell.rsplit('/').next(), dir()) {
        // --rcfile only works for a non-login bash; the script does the login part.
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
                "-l".into(),
                "--init-command".into(),
                format!("source '{}'", dir.join("noobsbot.fish").display().to_string().replace('\\', "\\\\").replace('\'', "\\'")),
            ];
        }
        _ => {}
    }
    (Some(Shell::new(shell, args)), env)
}
