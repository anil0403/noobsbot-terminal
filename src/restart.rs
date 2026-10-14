//! Restarting noobsbot: a fresh instance in place of a running one. Nothing
//! carries over; shells in the old window end with it.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// This binary, also after an update replaced the file ("…/noobsbot (deleted)").
pub fn exe() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| "noobsbot".into());
    exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")).map_or(exe.clone(), PathBuf::from)
}

/// Start noobsbot with `args`, detached from this terminal; `env` replaces ours if given.
pub fn launch(args: &[OsString], env: Option<Vec<(OsString, OsString)>>) -> std::io::Result<()> {
    let mut cmd = Command::new("/bin/sh");
    // $$ is sh's pid, which exec keeps: the dock ties the window to its launcher by it.
    cmd.args(["-c", r#"[ -n "$GIO_LAUNCHED_DESKTOP_FILE" ] && export GIO_LAUNCHED_DESKTOP_FILE_PID=$$; exec "$0" "$@""#]).arg(exe()).args(args);
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
        let read = |name: &str| std::fs::read(format!("/proc/{pid}/{name}")).ok();
        let (Some(cmdline), Some(environ)) = (read("cmdline"), read("environ")) else {
            eprintln!("noobsbot {pid} is gone");
            status = 1;
            continue;
        };
        let args: Vec<OsString> = split(&cmdline).skip(1).collect();
        let env = split(&environ).filter_map(key_value).collect();
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
    let me = unsafe { libc::getuid() };
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok().filter(|_| e.metadata().is_ok_and(|m| m.uid() == me)))
        .filter(|&pid| pid != std::process::id() && std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim() == "noobsbot"))
        .collect()
}

/// NUL-separated fields, as in /proc/<pid>/cmdline and environ.
fn split(bytes: &[u8]) -> impl Iterator<Item = OsString> + '_ {
    bytes.split(|&b| b == 0).filter(|f| !f.is_empty()).map(|f| OsString::from_vec(f.to_vec()))
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
    fn parses_proc_fields() {
        let args: Vec<OsString> = split(b"/usr/bin/noobsbot\0--quake\0").collect();
        assert_eq!(args, ["/usr/bin/noobsbot", "--quake"]);
        assert_eq!(key_value("A=b=c".into()), Some(("A".into(), "b=c".into())));
        assert_eq!(key_value("novalue".into()), None);
        // Our own environment reads back the same way the restart reads another's.
        let env: Vec<_> = split(&std::fs::read("/proc/self/environ").unwrap()).filter_map(key_value).collect();
        assert!(env.iter().any(|(k, _)| k == "PATH"));
    }
}
