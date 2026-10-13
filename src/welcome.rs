//! The welcome screen (terminal.welcome): a text-drawn cat beside the
//! machine's specs, shown in a new window before the shell's first prompt.
//! Everything is read from /proc and /etc; nothing is spawned.

use std::fs::read_to_string;

/// Colons are the fur (dense enough to read on black); dashes the inner ears,
/// stripes, eyes, whiskers, mouth, collar and toes.
const CAT: &str = r"
    ::              ::
   ::-::          ::-::
   ::---::::::::::---::
  ::::::::::::::::::::::
  :::::- o -::- o -:::::
  ::::::::::vv::::::::::
--- :::::::-::-::::::: ---
 ---::::::::::::::::::---
      ::::::::::::::
    ::::::::::::::::::   ::
   ::::::::::::::::::::  :-:
   :::-::-:::  :::-::-::::
    ------------------:::
";
const CAT_W: usize = 28;
/// Narrower than this, the cat is left out.
const MIN_COLS: usize = CAT_W + 3 + 57;
const BAR: usize = 20;

/// The screen as terminal output, for a pane `cols` wide. Colors are palette
/// indexes, so themes recolor it.
pub fn screen(cols: usize, gpu: &str, shell: &str) -> String {
    let info = info(gpu, shell);
    let cat: Vec<&str> = if cols >= MIN_COLS { CAT.lines().skip(1).collect() } else { Vec::new() };
    let mut out = String::from("\r\n");
    // The shorter column sits in the middle of the taller one.
    let rows = cat.len().max(info.len());
    let (cat_top, info_top) = ((rows - cat.len()) / 2, (rows - info.len()) / 2);
    for row in 0..rows {
        if !cat.is_empty() {
            let line = row.checked_sub(cat_top).and_then(|r| cat.get(r)).copied().unwrap_or("");
            out.push_str("\x1b[1;38;5;15m"); // bright white
            out.push_str(line);
            out.push_str(&" ".repeat(CAT_W + 3 - line.len()));
        } else {
            out.push(' ');
        }
        out.push_str("\x1b[0m");
        out.push_str(row.checked_sub(info_top).and_then(|r| info.get(r)).map_or("", String::as_str));
        out.push_str("\x1b[0m\r\n");
    }
    out.push_str("\r\n");
    out
}

fn info(gpu: &str, shell: &str) -> Vec<String> {
    let user = std::env::var("USER").unwrap_or_else(|_| "you".into());
    let now = now();
    // Like the pretty prompt: user, host, date. The terminal draws these separators itself.
    let header = format!(
        "\x1b[38;5;11m\u{e0b6}\x1b[48;5;11;38;5;0m {user} \x1b[38;5;11;48;5;12m\u{e0b0}\x1b[38;5;15m {} \x1b[38;5;12;48;5;14m\u{e0b0}\x1b[38;5;0m {} \x1b[0;38;5;14m\u{e0b4}",
        first_line("/proc/sys/kernel/hostname"),
        now.1
    );
    let mut lines = vec![
        header,
        format!("\x1b[1;38;5;11m{}\x1b[0;1m, {user}!\x1b[0m Welcome to \x1b[1;38;5;14mnoobsbot\x1b[0m.", greeting(now.0)),
    ];
    // Two or three facts a line, the empty ones left out.
    let mut row = |label: &str, values: &[String]| {
        let value = values.iter().filter(|v| !v.is_empty()).map(String::as_str).collect::<Vec<_>>().join(" \x1b[38;5;8m·\x1b[0m ");
        if !value.is_empty() {
            lines.push(format!("\x1b[1;38;5;12m{label:<10}\x1b[0m{value}"));
        }
    };
    row("OS", &[format!("{} {}", os_name(), std::env::consts::ARCH), first_line("/proc/sys/kernel/osrelease")]);
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().trim_start_matches("X-").to_owned();
    row("Desktop", &[desktop, shell.to_owned(), format!("noobsbot {}", env!("CARGO_PKG_VERSION"))]);
    row("Uptime", &[uptime(), packages()]);
    row("CPU", &[cpu()]);
    row("GPU", &[tidy(gpu)]);
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    let mem = memory();
    let gib = |kib: f64| kib / 1024.0 / 1024.0;
    let mut meter = |label: &str, bar: String, text: String| {
        lines.push(format!("\x1b[1;38;5;12m{label:<10}\x1b[0m{bar} {text}"));
    };
    if let Some(load) = first_line("/proc/loadavg").split(' ').next().and_then(|l| l.parse::<f64>().ok()) {
        meter("Load", bar(load / cores, false), format!("{load:.2} on {cores} cores"));
    }
    if let Some((total, free)) = mem.ram {
        meter("Memory", bar(1.0 - free / total, false), format!("{:.1} / {:.1} GiB", gib(total - free), gib(total)));
    }
    if let Some((total, free)) = mem.swap.filter(|s| s.0 > 0.0) {
        meter("Swap", bar(1.0 - free / total, false), format!("{:.1} / {:.1} GiB", gib(total - free), gib(total)));
    }
    if let Some((total, free)) = disk("/") {
        meter("Disk", bar(1.0 - free / total, false), format!("{:.0} / {:.0} GiB", gib(total - free), gib(total)));
    }
    if let Some((level, status)) = battery() {
        meter("Battery", bar(level, true), status.to_lowercase());
    }
    lines.push("\x1b[38;5;11m\u{e0b6}\x1b[1;48;5;11;38;5;0m Designed & Developed by Noobsbot Team \x1b[0;38;5;11m\u{e0b4}\x1b[0m".into());
    lines
}

/// A meter for `level` (0..=1): green, yellow from 60 %, red from 85 %.
/// `full_is_good` (a battery): red up to 15 %, yellow up to 40 %.
fn bar(level: f64, full_is_good: bool) -> String {
    let level = level.clamp(0.0, 1.0);
    let filled = (level * BAR as f64).round() as usize;
    let badness = if full_is_good { 1.0 - level } else { level };
    let color = if badness >= 0.85 { 1 } else if badness >= 0.6 { 3 } else { 2 };
    format!("\x1b[38;5;{color}m{}\x1b[38;5;8m{}\x1b[0m {:>3.0}%", "■".repeat(filled), "■".repeat(BAR - filled), level * 100.0)
}

fn greeting(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=21 => "Good evening",
        _ => "Up late",
    }
}

/// The local hour and "Tue 13 Oct 00:13".
fn now() -> (u32, String) {
    let mut text = [0u8; 64];
    // SAFETY: plain libc calls on our own values; strftime gets the buffer's size.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        let n = libc::strftime(text.as_mut_ptr().cast(), text.len(), c"%a %d %b %H:%M".as_ptr(), &tm);
        (tm.tm_hour as u32, String::from_utf8_lossy(&text[..n]).into_owned())
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

/// "AMD Ryzen 7 5800X 8-Core (16)".
fn cpu() -> String {
    let text = read_to_string("/proc/cpuinfo").unwrap_or_default();
    let Some(model) = text.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split_once(':')) else { return String::new() };
    let threads = text.lines().filter(|l| l.starts_with("processor")).count();
    format!("{} ({threads})", tidy(model.1))
}

/// Hardware names without the legal and filler words:
/// "Intel(R) Core(TM) i7-8550U CPU @ 1.80GHz" -> "Intel Core i7-8550U".
fn tidy(name: &str) -> String {
    let name = name.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    let name = name.split(" @ ").next().unwrap_or("");
    name.split_whitespace().filter(|w| !matches!(*w, "CPU" | "Processor")).collect::<Vec<_>>().join(" ")
}

/// Installed packages, where counting them is one directory listing.
fn packages() -> String {
    let count = |dir: &str, ext: &str| std::fs::read_dir(dir).map_or(0, |d| d.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(ext)).count());
    match (count("/var/lib/pacman/local", "").saturating_sub(1), count("/var/lib/dpkg/info", ".list")) {
        (0, 0) => String::new(),
        (0, n) => format!("{n} packages (dpkg)"),
        (n, _) => format!("{n} packages (pacman)"), // minus ALPM_DB_VERSION
    }
}

/// The first battery's charge (0..=1) and "Charging" / "Discharging" / "Full".
fn battery() -> Option<(f64, String)> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?.flatten().find(|e| e.file_name().to_string_lossy().starts_with("BAT"))?.path();
    let read = |name: &str| read_to_string(dir.join(name)).ok().map(|s| s.trim().to_owned());
    Some((read("capacity")?.parse::<f64>().ok()? / 100.0, read("status").unwrap_or_default()))
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
    fn cat_fits() {
        let lines: Vec<&str> = CAT.lines().skip(1).collect();
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
        assert!(bar(0.5, false).contains(&"■".repeat(10)) && bar(0.5, false).contains("38;5;2m"));
        assert!(bar(0.9, false).contains("38;5;1m") && bar(7.0, false).contains("100%"));
        // A battery is red when empty, green when full.
        assert!(bar(0.1, true).contains("38;5;1m") && bar(0.9, true).contains("38;5;2m"));
        assert_eq!(tidy("Intel(R) Core(TM) i7-8550U CPU @ 1.80GHz"), "Intel Core i7-8550U");
        assert_eq!(tidy("AMD Ryzen 7 5800X 8-Core Processor"), "AMD Ryzen 7 5800X 8-Core");
        // Wide: every line starts with the cat's column. Narrow: no cat.
        assert!(screen(120, "gpu", "zsh").contains("::-::"));
        assert!(!screen(80, "gpu", "zsh").contains("::-::"));
        assert_eq!((greeting(8), greeting(14), greeting(19), greeting(2)), ("Good morning", "Good afternoon", "Good evening", "Up late"));
    }
}
