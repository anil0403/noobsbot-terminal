//! The welcome screen (terminal.welcome): a dot-and-dash cat beside the
//! machine's specs, shown in a new window before the shell's first prompt.
//! Everything is read from /proc and /etc; nothing is spawned.

use std::fs::read_to_string;

/// Dots are the fur, dashes the eyes, whiskers, mouth and paws.
const CAT: &str = r"
    ..             ..
   ....           ....
   .....         .....
   ...................
  .....................
  .....--.......--.....
  .....--.......--.....
 -- .................. --
   --.......--.......--
 -- ......-....-...... --
    ......------......
     ...............
    .................
   ...................
  .....................
  .....................    ..
  .....................   ....
   ...................   ...
    ....--.....--....  ....
     ---------------.....
";
const CAT_W: usize = 30;
/// Narrower than this, the cat is left out.
const MIN_COLS: usize = CAT_W + 3 + 57;
const BAR: usize = 20;

/// The screen as terminal output, for a pane `cols` wide. Colors are palette
/// indexes, so themes recolor it.
pub fn screen(cols: usize, gpu: &str, shell: &str) -> String {
    let info = info(gpu, shell);
    let cat: Vec<&str> = if cols >= MIN_COLS { CAT.lines().skip(1).collect() } else { Vec::new() };
    let mut out = String::from("\r\n");
    for row in 0..cat.len().max(info.len()) {
        if !cat.is_empty() {
            let line = cat.get(row).copied().unwrap_or("");
            for c in line.chars() {
                match c {
                    '.' => out.push_str("\x1b[1;38;5;11m."),
                    '-' => out.push_str("\x1b[1;38;5;14m-"),
                    c => out.push(c),
                }
            }
            out.push_str(&" ".repeat(CAT_W + 3 - line.len()));
        } else {
            out.push(' ');
        }
        out.push_str("\x1b[0m");
        out.push_str(info.get(row).map_or("", String::as_str));
        out.push_str("\x1b[0m\r\n");
    }
    out.push_str("\r\n");
    out
}

fn info(gpu: &str, shell: &str) -> Vec<String> {
    let user = std::env::var("USER").unwrap_or_else(|_| "you".into());
    let host = first_line("/proc/sys/kernel/hostname");
    let mut lines = vec![
        format!("\x1b[1;38;5;11m{}\x1b[0;1m, {user}!\x1b[0m Welcome to \x1b[1;38;5;14mnoobsbot\x1b[0m.", greeting(local_hour())),
        format!("\x1b[38;5;8m{}", "-".repeat(40)),
    ];
    let mut row = |label: &str, value: String| {
        if !value.is_empty() {
            lines.push(format!("\x1b[1;38;5;12m{label:<9}\x1b[0m{value}"));
        }
    };
    row("Host", format!("{user}@{host}"));
    row("OS", format!("{} {}", os_name(), std::env::consts::ARCH));
    row("Kernel", first_line("/proc/sys/kernel/osrelease"));
    row("Uptime", uptime());
    row("Desktop", std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().trim_start_matches("X-").to_owned());
    row("Shell", shell.to_owned());
    row("Terminal", format!("noobsbot {}", env!("CARGO_PKG_VERSION")));
    row("CPU", cpu());
    row("GPU", gpu.to_owned());
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    let mem = memory();
    let gib = |kib: f64| kib / 1024.0 / 1024.0;
    lines.push(String::new());
    let mut meter = |label: &str, used: f64, text: String| {
        lines.push(format!("\x1b[1;38;5;12m{label:<9}\x1b[0m{} {text}", bar(used)));
    };
    if let Some(load) = first_line("/proc/loadavg").split(' ').next().and_then(|l| l.parse::<f64>().ok()) {
        meter("Load", load / cores, format!("{load:.2} on {cores} cores"));
    }
    if let Some((total, free)) = mem.ram {
        meter("Memory", 1.0 - free / total, format!("{:.1} / {:.1} GiB", gib(total - free), gib(total)));
    }
    if let Some((total, free)) = mem.swap.filter(|s| s.0 > 0.0) {
        meter("Swap", 1.0 - free / total, format!("{:.1} / {:.1} GiB", gib(total - free), gib(total)));
    }
    if let Some((total, free)) = disk("/") {
        meter("Disk /", 1.0 - free / total, format!("{:.0} / {:.0} GiB", gib(total - free), gib(total)));
    }
    lines.push(String::new());
    // The theme's 16 colors.
    for half in [0..8, 8..16] {
        lines.push(half.map(|i| format!("\x1b[48;5;{i}m   ")).collect::<String>());
    }
    lines.push(String::new());
    lines.push("\x1b[38;5;8mCtrl+Shift+P: every command and theme".into());
    lines
}

/// A meter for `used` (0..=1): green, yellow from 60 %, red from 85 %.
fn bar(used: f64) -> String {
    let used = used.clamp(0.0, 1.0);
    let filled = (used * BAR as f64).round() as usize;
    let color = if used >= 0.85 { 1 } else if used >= 0.6 { 3 } else { 2 };
    format!("\x1b[38;5;{color}m{}\x1b[38;5;8m{}\x1b[0m {:>3.0}%", "■".repeat(filled), "■".repeat(BAR - filled), used * 100.0)
}

fn greeting(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=21 => "Good evening",
        _ => "Up late",
    }
}

fn local_hour() -> u32 {
    // SAFETY: plain libc calls on our own values.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm.tm_hour as u32
    }
}

fn first_line(path: &str) -> String {
    read_to_string(path).unwrap_or_default().lines().next().unwrap_or("").trim().to_owned()
}

/// `PRETTY_NAME` from os-release: "Arch Linux", "Ubuntu 24.04.1 LTS".
fn os_name() -> String {
    let text = read_to_string("/etc/os-release").or_else(|_| read_to_string("/usr/lib/os-release")).unwrap_or_default();
    os_release(&text).unwrap_or_else(|| "Linux".into())
}

fn os_release(text: &str) -> Option<String> {
    let value = text.lines().find_map(|l| l.strip_prefix("PRETTY_NAME="))?;
    Some(value.trim().trim_matches('"').to_owned())
}

fn uptime() -> String {
    let Some(secs) = first_line("/proc/uptime").split(' ').next().and_then(|s| s.parse::<f64>().ok()) else { return String::new() };
    duration(secs as u64)
}

fn duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60);
    match (d, h) {
        (0, 0) => format!("{m} min"),
        (0, _) => format!("{h} h {m} min"),
        _ => format!("{d} d {h} h"),
    }
}

/// "AMD Ryzen 7 5800X 8-Core Processor (16)".
fn cpu() -> String {
    let text = read_to_string("/proc/cpuinfo").unwrap_or_default();
    let Some(model) = text.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split_once(':')) else { return String::new() };
    let threads = text.lines().filter(|l| l.starts_with("processor")).count();
    format!("{} ({threads})", model.1.split_whitespace().collect::<Vec<_>>().join(" "))
}

#[derive(Default, Debug, PartialEq)]
struct Memory {
    /// (total, available) in KiB.
    ram: Option<(f64, f64)>,
    swap: Option<(f64, f64)>,
}

fn memory() -> Memory {
    meminfo(&read_to_string("/proc/meminfo").unwrap_or_default())
}

fn meminfo(text: &str) -> Memory {
    let get = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(':')?.split_whitespace().next()?.parse::<f64>().ok());
    Memory { ram: get("MemTotal").zip(get("MemAvailable")), swap: get("SwapTotal").zip(get("SwapFree")) }
}

/// (total, free for us) of the filesystem holding `path`, in KiB.
fn disk(path: &str) -> Option<(f64, f64)> {
    let path = std::ffi::CString::new(path).ok()?;
    // SAFETY: `path` is a valid C string and `stat` is ours to fill.
    let stat = unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        (libc::statvfs(path.as_ptr(), &mut stat) == 0).then_some(stat)?
    };
    let kib = |blocks: u64| blocks as f64 * stat.f_frsize as f64 / 1024.0;
    (stat.f_blocks > 0).then(|| (kib(stat.f_blocks as u64), kib(stat.f_bavail as u64)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cat_is_dots_and_dashes_and_fits() {
        let lines: Vec<&str> = CAT.lines().skip(1).collect();
        assert!(lines.iter().all(|l| l.chars().all(|c| matches!(c, '.' | '-' | ' '))));
        assert_eq!(lines.iter().map(|l| l.len()).max(), Some(CAT_W));
    }

    #[test]
    fn parses_system_files() {
        assert_eq!(os_release("NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\n"), Some("Arch Linux".into()));
        assert_eq!(os_release("ID=void\n"), None);
        let mem = meminfo("MemTotal:       32000000 kB\nMemFree: 1 kB\nMemAvailable:   24000000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n");
        assert_eq!(mem, Memory { ram: Some((32000000.0, 24000000.0)), swap: Some((0.0, 0.0)) });
        assert_eq!(meminfo(""), Memory::default());
        assert_eq!((duration(59), duration(3 * 3600 + 120), duration(2 * 86400 + 3600)), ("0 min".into(), "3 h 2 min".into(), "2 d 1 h".into()));
    }

    #[test]
    fn meters_and_layout() {
        assert!(bar(0.5).contains(&"■".repeat(10)) && bar(0.5).contains("38;5;2m"));
        assert!(bar(0.9).contains("38;5;1m") && bar(7.0).contains("100%"));
        // Wide: every line starts with the cat's column. Narrow: no cat.
        assert!(screen(120, "gpu", "zsh").contains("1;38;5;14m-"));
        assert!(!screen(80, "gpu", "zsh").contains("1;38;5;14m-"));
        assert_eq!((greeting(8), greeting(14), greeting(19), greeting(2)), ("Good morning", "Good afternoon", "Good evening", "Up late"));
    }
}
