//! Suggestions for what's typed at the prompt, drawn as grey text after the
//! cursor (like fish): a command that worked in this folder before, then one
//! that worked anywhere, then the shell's history, then a file or folder name.
//! Commands typed with a leading space, or that look like they hold a password
//! or token, are neither remembered nor shown.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Commands kept in noobsbot's own history; the file is trimmed to this on load.
const KEEP: usize = 5_000;

#[derive(Default)]
pub struct Suggest {
    /// Commands that succeeded in noobsbot and where they ran, oldest first.
    ran: Vec<(String, PathBuf)>,
    ran_file: Option<PathBuf>,
    /// The shell's own history, reread when the file changes.
    shell: Vec<String>,
    shell_name: String,
    shell_file: Option<PathBuf>,
    shell_mtime: Option<SystemTime>,
    /// The last folder listed: path, mtime, names (folders end in /) sorted.
    listing: Option<(PathBuf, SystemTime, Vec<String>)>,
    /// The last answer, for the same input: typed, folder, suggestion.
    last: Option<(String, Option<PathBuf>, Option<String>)>,
}

impl Suggest {
    pub fn new() -> Self {
        let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or(home().join(".local/share"));
        let mut s = Self::default();
        s.load_ran(data.join("noobsbot/history"));
        s
    }

    /// Read the history of `shell` ("zsh", "bash"; others have none).
    pub fn set_shell(&mut self, shell: &str) {
        if self.shell_name == shell {
            return;
        }
        self.shell_name = shell.to_owned();
        self.shell_file = std::env::var_os("HISTFILE").map(PathBuf::from).or_else(|| match shell {
            "zsh" => Some(std::env::var_os("ZDOTDIR").map_or(home(), PathBuf::from).join(".zsh_history")),
            "bash" => Some(home().join(".bash_history")),
            _ => None,
        });
        (self.shell, self.shell_mtime, self.last) = (Vec::new(), None, None);
    }

    fn load_ran(&mut self, file: PathBuf) {
        let text = fs::read_to_string(&file).unwrap_or_default();
        let _ = fs::set_permissions(&file, fs::Permissions::from_mode(0o600)); // yours alone
        let lines: Vec<&str> = text.lines().collect();
        let skip = lines.len().saturating_sub(KEEP);
        self.ran = lines[skip..].iter().filter_map(|l| l.split_once('\t')).map(|(dir, cmd)| (cmd.to_owned(), dir.into())).collect();
        if skip > KEEP {
            // Twice as long as kept: write back only the kept part.
            let _ = fs::write(&file, lines[skip..].iter().map(|l| format!("{l}\n")).collect::<String>());
        }
        self.ran_file = Some(file);
    }

    /// Remember a command that succeeded in `dir`.
    pub fn ran(&mut self, cmd: &str, dir: &Path) {
        if cmd.starts_with(' ') || secret(cmd) {
            return;
        }
        let cmd = cmd.trim();
        if cmd.is_empty() || cmd.contains(['\n', '\t']) || dir.as_os_str().to_string_lossy().contains('\t') {
            return;
        }
        if self.ran.last().is_some_and(|(c, d)| c == cmd && d == dir) {
            return; // the same again
        }
        self.ran.push((cmd.to_owned(), dir.to_owned()));
        self.last = None;
        let Some(file) = &self.ran_file else { return };
        let _ = fs::create_dir_all(file.parent().unwrap());
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).mode(0o600).open(file) {
            let _ = writeln!(f, "{}\t{cmd}", dir.display());
        }
    }

    /// What to add after `typed`, typed in folder `cwd`.
    pub fn get(&mut self, typed: &str, cwd: Option<&Path>) -> Option<String> {
        if let Some((t, c, answer)) = &self.last
            && t == typed
            && c.as_deref() == cwd
        {
            return answer.clone();
        }
        let answer = self.find(typed, cwd);
        self.last = Some((typed.to_owned(), cwd.map(Path::to_owned), answer.clone()));
        answer
    }

    fn find(&mut self, typed: &str, cwd: Option<&Path>) -> Option<String> {
        if typed.trim().is_empty() {
            return None;
        }
        self.reload_shell();
        let longer = |c: &str| c.len() > typed.len() && c.starts_with(typed) && !secret(c);
        let ran = self.ran.iter().rev();
        ran.clone()
            .find(|(c, d)| Some(d.as_path()) == cwd && longer(c))
            .or_else(|| ran.clone().find(|(c, _)| longer(c)))
            .map(|(c, _)| c)
            .or_else(|| self.shell.iter().rev().find(|c| longer(c)))
            .map(|c| c[typed.len()..].to_owned())
            .or_else(|| self.file(typed, cwd?))
    }

    fn reload_shell(&mut self) {
        let Some(file) = &self.shell_file else { return };
        let mtime = fs::metadata(file).and_then(|m| m.modified()).ok();
        if mtime == self.shell_mtime {
            return;
        }
        self.shell_mtime = mtime;
        self.shell = parse_history(&fs::read(file).unwrap_or_default());
    }

    /// The rest of a file or folder name for the last word, if it's an argument
    /// or a path.
    fn file(&mut self, typed: &str, cwd: &Path) -> Option<String> {
        let word = typed.rsplit(' ').next()?;
        if word.is_empty() || (!typed.contains(' ') && !word.contains('/')) {
            return None;
        }
        let (dir, prefix) = word.rsplit_once('/').map_or(("", word), |(d, p)| (d, p));
        let dir = match dir {
            "" if word.starts_with('/') => PathBuf::from("/"),
            "" => cwd.to_owned(),
            "~" => home(),
            d => match d.strip_prefix("~/") {
                Some(rest) => home().join(rest),
                None => cwd.join(d),
            },
        };
        let mtime = fs::metadata(&dir).and_then(|m| m.modified()).ok()?;
        if !self.listing.as_ref().is_some_and(|(d, m, _)| *d == dir && *m == mtime) {
            let mut names: Vec<String> = fs::read_dir(&dir)
                .ok()?
                .flatten()
                .map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    // The entry's type comes with the listing; only symlinks need a stat.
                    let dir = e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink() && e.path().is_dir());
                    if dir { name + "/" } else { name }
                })
                .collect();
            names.sort();
            self.listing = Some((dir, mtime, names));
        }
        let names = &self.listing.as_ref()?.2;
        let name = names.iter().find(|n| n.len() > prefix.len() && n.starts_with(prefix) && (prefix.starts_with('.') || !n.starts_with('.')))?;
        Some(name[prefix.len()..].replace(' ', "\\ "))
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// Looks like it holds a password or token. ponytail: a word list and token
/// prefixes; it also skips harmless commands that mention "token", which is fine.
fn secret(cmd: &str) -> bool {
    const WORDS: [&str; 9] = ["password", "passwd", "token", "secret", "api_key", "apikey", "api-key", "authorization:", "bearer "];
    const PREFIXES: [&str; 10] = ["ghp_", "gho_", "ghu_", "ghs_", "github_pat_", "glpat-", "xoxb-", "xoxp-", "AKIA", "sk-"];
    let lower = cmd.to_lowercase();
    WORDS.iter().any(|w| lower.contains(w))
        || cmd.split(|c: char| c.is_whitespace() || "=:'\"".contains(c)).any(|w| w.len() >= 16 && PREFIXES.iter().any(|p| w.starts_with(p)))
}

/// The first word of a suggestion, with the spaces before it, up to and
/// including the next space or slash (Ctrl+Right takes this much).
pub fn word(s: &str) -> &str {
    let start = s.len() - s.trim_start().len();
    let end = s[start..].find([' ', '/']).map_or(s.len(), |i| start + i + 1);
    &s[..end]
}

/// Commands from a bash or zsh history file, oldest first.
fn parse_history(bytes: &[u8]) -> Vec<String> {
    // zsh "metafies" some bytes (0x83..=0x9d among them): 0x83, then the byte xor 0x20.
    let mut plain = Vec::with_capacity(bytes.len());
    let mut it = bytes.iter();
    while let Some(&b) = it.next() {
        match b {
            0x83 => plain.extend(it.next().map(|n| n ^ 0x20)),
            b => plain.push(b),
        }
    }
    let text = String::from_utf8_lossy(&plain);
    let mut out = Vec::new();
    let mut multi = false; // inside a multi-line command
    for line in text.lines() {
        let continued = std::mem::replace(&mut multi, line.ends_with('\\'));
        if continued || line.starts_with('#') {
            continue; // the rest of a multi-line command, or a bash timestamp
        }
        // zsh extended history: ": 1700000000:0;command"
        let cmd = match line.strip_prefix(": ") {
            Some(rest) => rest.split_once(';').map_or(line, |(_, c)| c),
            None => line,
        };
        if !multi && !cmd.trim().is_empty() {
            out.push(cmd.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests() {
        let dir = std::env::temp_dir().join(format!("noobsbot-suggest-{}", std::process::id()));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("Cargo.toml"), "").unwrap();
        fs::write(dir.join("my notes.txt"), "").unwrap();
        fs::write(dir.join(".hidden"), "").unwrap();
        let mut s = Suggest { shell: vec!["git status".into(), "git push".into()], ..Default::default() };
        s.ran.push(("git pull".into(), "/elsewhere".into()));
        s.ran.push(("git log".into(), dir.clone()));
        let here = Some(dir.as_path());
        assert_eq!(s.get("git ", here).as_deref(), Some("log")); // ran here
        assert_eq!(s.get("git p", here).as_deref(), Some("ull")); // ran anywhere
        assert_eq!(s.get("git st", here).as_deref(), Some("atus")); // shell history
        assert_eq!(s.get("git status", here), None);
        assert_eq!(s.get("cat Ca", here).as_deref(), Some("rgo.toml"));
        assert_eq!(s.get("cd s", here).as_deref(), Some("rc/"));
        assert_eq!(s.get("less my", here).as_deref(), Some("\\ notes.txt"));
        assert_eq!(s.get("cat .h", here).as_deref(), Some("idden"));
        assert_eq!(s.get("Ca", here), None); // a command name, not a file
        assert_eq!(s.get("ls ", here), None);
        assert_eq!(s.get("  ", here), None);
        fs::remove_dir_all(&dir).unwrap();

        // Secrets and space-prefixed commands stay out.
        s.shell.push("mysql --password=hunter2".into());
        s.shell.push("curl -H 'X: ghp_abcdefghijklmnop1234' x".into());
        assert_eq!(s.get("mysql", here), None);
        assert_eq!(s.get("curl", here), None);
        s.ran(" echo private", &dir);
        s.ran("export API_KEY=1", &dir);
        assert_eq!(s.ran.len(), 2);
        assert!(!secret("git commit -m 'fix the disk-usage bar'"));

        assert_eq!(word("status --short"), "status ");
        assert_eq!(word(" --short"), " --short");
        assert_eq!(word("rc/main.rs"), "rc/");

        let h = parse_history(b": 1700000000:0;ls -la\n: 1700000001:0;echo a\\\nb\n#1700000002\ngit push\ncaf\xc3\xa9 \xe2\x80\x83\xb4\n");
        assert_eq!(h, vec!["ls -la", "git push", "caf\u{e9} \u{2014}"]);
    }
}
