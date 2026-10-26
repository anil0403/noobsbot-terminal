//! What's going on in a terminal right now, for picking a theme.
//! Everything comes from /proc and the filesystem; no subprocesses.

use std::os::fd::RawFd;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    /// Foreground program name, e.g. "bash", "ssh", "vim".
    pub process: String,
    /// Effective user of the foreground program ("root" under sudo).
    pub user: String,
    pub ssh_host: Option<String>,
    pub cwd: Option<PathBuf>,
    pub git_branch: Option<String>,
    /// Local hour, 0-23.
    pub hour: u32,
}

/// Inspect the foreground process group of the PTY (falls back to the shell).
pub fn detect(pty_fd: RawFd, shell_pid: u32) -> Context {
    let pgrp = unsafe { libc::tcgetpgrp(pty_fd) };
    let pid = if pgrp > 0 { pgrp as u32 } else { shell_pid };
    let proc = |file: &str| std::fs::read_to_string(format!("/proc/{pid}/{file}")).unwrap_or_default();

    let process = proc("comm").trim().to_owned();
    let args: Vec<String> = proc("cmdline").split('\0').filter(|a| !a.is_empty()).map(String::from).collect();
    let user = effective_uid(&proc("status")).and_then(|uid| username(&std::fs::read_to_string("/etc/passwd").unwrap_or_default(), uid)).unwrap_or_default();
    let ssh_host = (process == "ssh").then(|| ssh_host(&args)).flatten();
    let cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).or_else(|_| std::fs::read_link(format!("/proc/{shell_pid}/cwd"))).ok();
    let git_branch = cwd.as_deref().and_then(git_branch);

    Context { process, user, ssh_host, cwd, git_branch, hour: crate::welcome::now().0 }
}

/// The program in the PTY's foreground when it isn't the shell itself (what
/// closing the pane would end), by name.
pub fn running(pty_fd: RawFd, shell_pid: u32) -> Option<String> {
    let pgrp = unsafe { libc::tcgetpgrp(pty_fd) };
    if pgrp <= 0 || pgrp as u32 == shell_pid {
        return None;
    }
    let name = std::fs::read_to_string(format!("/proc/{pgrp}/comm")).unwrap_or_default().trim().to_owned();
    Some(if name.is_empty() { "a program".into() } else { name })
}

fn effective_uid(status: &str) -> Option<u32> {
    // "Uid:\treal\teffective\tsaved\tfs"
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(2)?.parse().ok()
}

fn username(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        (fields.nth(1)?.parse::<u32>().ok()? == uid).then(|| name.to_owned())
    })
}

/// Destination host from an ssh command line: `ssh -p 22 me@prod-db` → "prod-db".
fn ssh_host(args: &[String]) -> Option<String> {
    const TAKES_VALUE: &str = "BbcDEeFIiJLlmOoPpQRSWw";
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if let Some(flags) = arg.strip_prefix('-').filter(|f| !f.is_empty()) {
            // `-p22` carries its value; `-p 22` takes the next arg.
            if let Some(pos) = flags.find(|c| TAKES_VALUE.contains(c))
                && pos + 1 == flags.len()
            {
                iter.next();
            }
            continue;
        }
        let dest = arg.strip_prefix("ssh://").unwrap_or(arg);
        let host = dest.rsplit('@').next()?;
        let host = if arg.starts_with("ssh://") { host.split(':').next()? } else { host };
        return Some(host.to_owned());
    }
    None
}

/// Current branch of the repo containing `dir`, or a short commit id when detached.
fn git_branch(dir: &Path) -> Option<String> {
    let dot_git = dir.ancestors().map(|d| d.join(".git")).find(|p| p.exists())?;
    let git_dir = if dot_git.is_file() {
        // Worktrees and submodules: ".git" is a file saying "gitdir: <path>".
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let target = PathBuf::from(text.strip_prefix("gitdir:")?.trim());
        if target.is_absolute() { target } else { dot_git.parent()?.join(target) }
    } else {
        dot_git
    };
    parse_head(&std::fs::read_to_string(git_dir.join("HEAD")).ok()?)
}

fn parse_head(head: &str) -> Option<String> {
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        Some(r) => Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_owned()),
        None => head.get(..7).map(String::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn finds_ssh_host() {
        assert_eq!(ssh_host(&args("ssh prod-db")), Some("prod-db".into()));
        assert_eq!(ssh_host(&args("ssh -p 2222 -i key.pem admin@prod-db.example.com uptime")), Some("prod-db.example.com".into()));
        assert_eq!(ssh_host(&args("ssh -p2222 -vA web1")), Some("web1".into()));
        assert_eq!(ssh_host(&args("ssh -o StrictHostKeyChecking=no -J bastion me@db")), Some("db".into()));
        assert_eq!(ssh_host(&args("ssh ssh://me@box:2200")), Some("box".into()));
        assert_eq!(ssh_host(&args("ssh -V")), None);
    }

    #[test]
    fn parses_proc_and_git() {
        assert_eq!(effective_uid("Name:\tsudo\nUid:\t1000\t0\t0\t0\n"), Some(0));
        let passwd = "root:x:0:0::/root:/bin/bash\ndevil:x:1000:1000::/home/devil:/bin/bash\n";
        assert_eq!(username(passwd, 0), Some("root".into()));
        assert_eq!(username(passwd, 1000), Some("devil".into()));
        assert_eq!(username(passwd, 42), None);
        assert_eq!(parse_head("ref: refs/heads/feature/x\n"), Some("feature/x".into()));
        assert_eq!(parse_head("3b18e512dba79e4c8300dd08aeb37f8e728b8dad\n"), Some("3b18e51".into()));
    }

    #[test]
    fn finds_this_repos_branch() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        assert!(git_branch(&here).is_some());
    }
}
