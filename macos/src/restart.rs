//! Restarting noobsbot: a fresh instance in place of a running one. Nothing
//! carries over; shells in the old window end with it.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

/// Environment variables, as (KEY, value).
type Env = Vec<(OsString, OsString)>;

/// Start noobsbot with `args`, detached from this terminal; `env` replaces ours if given.
pub fn launch(args: &[OsString], env: Option<Env>) -> std::io::Result<()> {
    spawn(args, env, None)
}

/// Another noobsbot window, its first shell in `dir` (else where this one started).
pub fn new_window(dir: Option<&Path>) -> std::io::Result<()> {
    spawn(&[], None, dir)
}

fn spawn(args: &[OsString], env: Option<Env>, dir: Option<&Path>) -> std::io::Result<()> {
    // After an update the bundle's binary is a new file at the same path.
    let mut cmd = Command::new(std::env::current_exe().unwrap_or_else(|_| "noobsbot".into()));
    cmd.args(args);
    if let Some(dir) = dir {
        cmd.current_dir(dir); // the first shell starts in noobsbot's own folder
    }
    if let Some(env) = env {
        cmd.env_clear().envs(env);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe; it frees the new noobsbot from this terminal.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().map(drop)
}

/// `noobsbot --restart`: the noobsbot this shell runs in, or all of yours when
/// run elsewhere. Each comes back as it was started (arguments, environment).
pub fn cli() -> i32 {
    let targets = match std::env::var("NOOBSBOT_PID").ok().and_then(|p| p.parse::<u32>().ok()) {
        Some(pid) => vec![pid],
        None => running(),
    };
    if targets.is_empty() {
        eprintln!("no noobsbot is running");
        return 1;
    }
    let mut status = 0;
    for pid in targets {
        let Some((args, env)) = procargs(pid) else {
            eprintln!("noobsbot {pid} is gone");
            status = 1;
            continue;
        };
        match launch(&args, Some(env)) {
            // SAFETY: plain kill on a pid we just read. Its shells get SIGHUP as it closes.
            Ok(()) => unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            },
            Err(e) => {
                eprintln!("can't start noobsbot: {e}");
                status = 1;
            }
        }
    }
    status
}

/// Your other noobsbot processes.
fn running() -> Vec<u32> {
    let me = unsafe { libc::getuid() }.to_string();
    let Ok(out) = Command::new("pgrep").args(["-x", "-U", &me, "noobsbot"]).output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse().ok()).filter(|&pid| pid != std::process::id()).collect()
}

/// A process's arguments (without the program) and environment, from sysctl(KERN_PROCARGS2).
fn procargs(pid: u32) -> Option<(Vec<OsString>, Env)> {
    let mut max: libc::c_int = 0;
    let mut len = std::mem::size_of_val(&max);
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: sysctl writes at most `len` bytes into `max`.
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 2, (&raw mut max).cast(), &mut len, std::ptr::null_mut(), 0) } != 0 {
        return None;
    }
    let mut buf = vec![0u8; max as usize];
    let mut len = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    // SAFETY: as above, into `buf`.
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) } != 0 {
        return None;
    }
    parse_procargs(&buf[..len])
}

/// KERN_PROCARGS2 layout: argc (i32), the exec path, NUL padding, argv, env,
/// then an empty field before Apple's own strings.
fn parse_procargs(buf: &[u8]) -> Option<(Vec<OsString>, Env)> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?) as usize;
    let rest = &buf[4..];
    let path_end = rest.iter().position(|&b| b == 0)?;
    let start = path_end + rest[path_end..].iter().position(|&b| b != 0)?;
    let mut fields = rest[start..].split(|&b| b == 0).map(|f| OsString::from_vec(f.to_vec()));
    let args: Vec<OsString> = fields.by_ref().take(argc).skip(1).collect();
    let env = fields.take_while(|f| !f.is_empty()).filter_map(key_value).collect();
    Some((args, env))
}

/// "KEY=value" -> (KEY, value).
fn key_value(kv: OsString) -> Option<(OsString, OsString)> {
    let kv = kv.into_vec();
    let at = kv.iter().position(|&b| b == b'=')?;
    Some((OsString::from_vec(kv[..at].to_vec()), OsString::from_vec(kv[at + 1..].to_vec())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_procargs() {
        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend(b"/x/noobsbot\0\0\0\0noobsbot\0--quake\0A=b=c\0novalue\0PATH=/bin\0\0ptr_munge=\0");
        let (args, env) = parse_procargs(&buf).unwrap();
        assert_eq!(args, ["--quake"]);
        assert_eq!(env, [("A".into(), "b=c".into()), ("PATH".into(), "/bin".into())]);
        // Our own process reads back the same way the restart reads another's.
        let (_, env) = procargs(std::process::id()).unwrap();
        assert!(env.iter().any(|(k, _)| k == "PATH"));
    }
}
