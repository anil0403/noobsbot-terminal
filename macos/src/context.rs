//! What's going on in a terminal right now, for picking a theme.
//! Everything comes from libproc, sysctl and the filesystem; no subprocesses.

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
    let info = bsd_info(pid as i32);
    let process =
        info.as_ref().map(|i| c_text(&i.pbi_name)).filter(|n| !n.is_empty()).or_else(|| info.as_ref().map(|i| c_text(&i.pbi_comm))).unwrap_or_default();
    let user = info.and_then(|i| username(i.pbi_uid)).unwrap_or_default();
    let ssh_host = (process == "ssh").then(|| ssh_host(&args(pid as i32))).flatten();
    let cwd = proc_cwd(pid as i32).or_else(|| proc_cwd(shell_pid as i32));
    let git_branch = cwd.as_deref().and_then(git_branch);

    Context { process, user, ssh_host, cwd, git_branch, hour: crate::welcome::now().0 }
}

/// Text of a NUL-terminated C char array.
fn c_text(chars: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = chars.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Name, effective user and more of a process (proc_pidinfo).
fn bsd_info(pid: i32) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (n == size).then_some(info)
}

/// A process's working directory.
pub fn proc_cwd(pid: i32) -> Option<PathBuf> {
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, (&raw mut info).cast(), size) };
    let path = c_text(info.pvi_cdir.vip_path.as_flattened());
    (n == size && !path.is_empty()).then(|| PathBuf::from(path))
}

/// The user name for a uid, cached (it doesn't change while we run).
fn username(uid: u32) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static NAMES: Mutex<Option<HashMap<u32, String>>> = Mutex::new(None);
    let mut names = NAMES.lock().unwrap();
    if let Some(name) = names.get_or_insert_default().get(&uid) {
        return Some(name.clone());
    }
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = [0 as libc::c_char; 2048];
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut found) };
    if found.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }.to_string_lossy().into_owned();
    names.get_or_insert_default().insert(uid, name.clone());
    Some(name)
}

/// A process's arguments (sysctl KERN_PROCARGS2).
fn args(pid: i32) -> Vec<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut buf = vec![0u8; 64 * 1024];
    let mut len = buf.len();
    let ok = unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) } == 0;
    if ok { parse_procargs(&buf[..len]) } else { Vec::new() }
}

/// KERN_PROCARGS2: argc (4 bytes), the executable's path, NUL padding, then argv.
fn parse_procargs(buf: &[u8]) -> Vec<String> {
    let Some(argc) = buf.get(..4).map(|b| i32::from_ne_bytes(b.try_into().unwrap()) as usize) else { return Vec::new() };
    let rest = &buf[4..];
    let Some(path_end) = rest.iter().position(|&b| b == 0) else { return Vec::new() };
    let rest = &rest[path_end..];
    let start = rest.iter().position(|&b| b != 0).unwrap_or(rest.len());
    rest[start..].split(|&b| b == 0).take(argc).map(|a| String::from_utf8_lossy(a).into_owned()).collect()
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
        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/bin/ssh\0\0\0\0ssh\0prod-db\0PATH=/usr/bin\0");
        assert_eq!(parse_procargs(&buf), ["ssh", "prod-db"]);
        assert_eq!(username(0), Some("root".into()));
        let me = std::process::id() as i32;
        assert_eq!(proc_cwd(me), std::env::current_dir().ok());
        assert_eq!(super::args(me).first().map(|a| a.contains("noobsbot")), Some(true));
        assert_eq!(parse_head("ref: refs/heads/feature/x\n"), Some("feature/x".into()));
        assert_eq!(parse_head("3b18e512dba79e4c8300dd08aeb37f8e728b8dad\n"), Some("3b18e51".into()));
    }

    #[test]
    fn finds_this_repos_branch() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        assert!(git_branch(&here).is_some());
    }
}
