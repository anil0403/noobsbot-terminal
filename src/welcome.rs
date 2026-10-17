//! The welcome screen (terminal.welcome): a text-drawn cat beside the
//! machine's specs, shown in a new window before the shell's first prompt.
//! Everything is read from /proc and /etc; nothing is spawned.

use std::fs::read_to_string;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

/// `:` and `.` are the fur, `$` and `#` the outline and stripes, `-` and `=` the
/// whiskers; `O` (the eyes) and `v` (the nose) are colored where it's drawn.
const CAT: &str = r###"
       ^-.             .-^
      :/`'.           .'"\:
      $ ##. ':::::::' ,## |
      $.#~'.:::::::::."##.|
    .#'::::::::::::::::::::.:
    $;::::   :::.:::   ::::;$
  --=:: :. (O) :.: (O) .: ::=--
 -----=: :.   :.v :.    :=-----
 --===--:.   \_/ \_/  .:--===--
       /#:..         ..:#\
      /#..###------##...$\
     /$...###########:...$
    /#...:############...#.
   /##...:############...#.
   $/.#....:=======...#..\$
  //.###.......#:....###...\)   ##.
  #..####......#..../###....$   ##|
  #..#####.....#....####....$  ,##|
  `..:#####....#.../###:....( /##'
   $...####....#...###:.....?;#$:|
   \#....####..:.####...:  :##:
    `##::######..####:=#:::::::;
"###;
const CAT_W: usize = 35;
/// Narrower than this, the cat is left out.
const MIN_COLS: usize = CAT_W + 3 + 57;
const BAR: usize = 20;
/// Where the latest release is looked up, and how often (the answer is cached).
const RELEASES: &str = "https://api.github.com/repos/anil0403/noobsbot-terminal/releases/latest";
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// A release newer than this build, once the check has found one.
static NEWER: Mutex<Option<String>> = Mutex::new(None);

/// The screen as terminal output, for a pane `cols` wide. Colors are palette
/// indexes, so themes recolor it.
pub fn screen(cols: usize, gpu: &str, shell: &str) -> String {
    let cat: Vec<&str> = if cols >= MIN_COLS { CAT.lines().skip(1).collect() } else { Vec::new() };
    let info = stretch(info(gpu, shell), cat.len());
    let mut out = String::from("\r\n");
    // The shorter column sits in the middle of the taller one.
    let rows = cat.len().max(info.len());
    let (cat_top, info_top) = ((rows - cat.len()) / 2, (rows - info.len()) / 2);
    for row in 0..rows {
        let mut line = String::new();
        if !cat.is_empty() {
            let cat = row.checked_sub(cat_top).and_then(|r| cat.get(r)).copied().unwrap_or("");
            line.push_str("\x1b[1;38;5;15m"); // bright white
            line.push_str(&paint(cat));
            line.push_str(&" ".repeat(CAT_W + 3 - cat.len()));
        } else {
            line.push(' ');
        }
        line.push_str("\x1b[0m");
        line.push_str(row.checked_sub(info_top).and_then(|r| info.get(r)).map_or("", String::as_str));
        out.push_str(&fit(&line, cols));
        out.push_str("\x1b[0m\r\n");
    }
    out.push_str("\r\n");
    out
}

/// Lines the screen takes: where the shell's prompt starts below it.
pub fn rows(screen: &str) -> usize {
    screen.matches("\r\n").count()
}

/// The line cut to `cols` visible characters (escapes kept), so it never wraps.
fn fit(line: &str, cols: usize) -> String {
    let (mut out, mut shown, mut chars) = (String::new(), 0, line.chars());
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            // A CSI runs to its final byte.
            if let Some(d) = chars.next() {
                out.push(d);
                if d == '[' {
                    for e in chars.by_ref() {
                        out.push(e);
                        if ('@'..='~').contains(&e) {
                            break;
                        }
                    }
                }
            }
        } else if shown < cols {
            out.push(c);
            shown += 1;
        }
    }
    out
}

/// Draws `screen` over the one drawn `rows` tall at the top, keeping the cursor (and
/// what's below) with the text. A different height moves what's below up or down
/// (`rows` is then the new one); an empty screen takes it away. Only while it's all
/// still there: nothing scrolled off, no full-screen app, the prompt below it, and
/// room below for it to grow. True if it did.
pub fn redraw<T: EventListener>(term: &mut Term<T>, screen: &str, rows: &mut usize) -> bool {
    let (new, cols, grid) = (self::rows(screen), term.columns(), term.grid());
    let shift = new as i32 - *rows as i32;
    let line = grid.cursor.point.line.0;
    if grid.history_size() > 0
        || grid.display_offset() > 0
        || term.mode().contains(TermMode::ALT_SCREEN)
        || line < *rows as i32
        || line + shift >= grid.screen_lines() as i32
    {
        return false;
    }
    let (mut cursor, mut saved) = (grid.cursor.clone(), grid.saved_cursor.clone());
    // Lines put in (or taken out) under the old screen move the prompt with them.
    let mut text = match shift {
        0 => String::new(),
        1.. => format!("\x1b[{};1H\x1b[{shift}L", *rows + 1),
        _ => format!("\x1b[{};1H\x1b[{}M", new + 1, -shift),
    };
    // Each line cut to the pane, so none wraps onto the prompt, and erased to its end:
    // a value can get shorter.
    text.push_str("\x1b[H");
    for line in screen.split_inclusive("\r\n") {
        text.push_str(&fit(line.trim_end_matches("\r\n"), cols));
        text.push_str(if line.ends_with("\r\n") { "\x1b[K\r\n" } else { "" });
    }
    Processor::<StdSyncHandler>::new().advance(term, text.as_bytes());
    // Lines taken out at the top go to the scrollback, empty before: they were ours.
    term.grid_mut().clear_history();
    cursor.point.line.0 += shift;
    if saved.point.line.0 >= *rows as i32 {
        saved.point.line.0 += shift;
    }
    let grid = term.grid_mut();
    (grid.cursor, grid.saved_cursor) = (cursor, saved);
    *rows = new;
    true
}

/// Starts the update check: the cached answer now, and a fresh one from GitHub in
/// the background once that's stale. The screen shows a newer release when drawn.
pub fn check_for_update() {
    let Some(cache) = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .map(|state| state.join("noobsbot/latest-release"))
    else {
        return;
    };
    let fresh = std::fs::metadata(&cache).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < CHECK_EVERY);
    found(read_to_string(&cache).ok());
    if fresh {
        return;
    }
    std::thread::spawn(move || {
        let Ok(out) = std::process::Command::new("curl").args(["-fsSL", "-m", "10", RELEASES]).stderr(std::process::Stdio::null()).output() else { return };
        let Some(tag) = tag_name(&String::from_utf8_lossy(&out.stdout)) else { return };
        if let Some(dir) = cache.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&cache, &tag);
        found(Some(tag));
    });
}

fn found(tag: Option<String>) {
    *NEWER.lock().unwrap() = tag.map(|t| t.trim().to_owned()).filter(|t| newer(t, env!("CARGO_PKG_VERSION")));
}

/// `"tag_name": "v0.2.1"` in GitHub's answer → `v0.2.1`.
fn tag_name(json: &str) -> Option<String> {
    let rest = &json[json.find("\"tag_name\"")? + 10..];
    let rest = &rest[rest.find(':')? + 1..];
    let rest = &rest[rest.find('"')? + 1..];
    Some(rest[..rest.find('"')?].to_owned()).filter(|t| !t.is_empty())
}

/// `tag` (`v0.2.1`) is a later version than `ours` (`0.2.0`).
fn newer(tag: &str, ours: &str) -> bool {
    let parts = |v: &str| v.trim_start_matches('v').split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(tag) > parts(ours)
}

/// The last CPU reading, (busy, total) ticks, and the share it gave.
static CPU: Mutex<Option<((u64, u64), f64)>> = Mutex::new(None);

/// Share of the CPU in use, all cores, since the last call (the screen refreshes
/// every 2 s); the first call watches 100 ms.
fn cpu_use() -> Option<f64> {
    let now = cpu_ticks()?;
    let mut last = CPU.lock().unwrap();
    let share = match *last {
        // Called again within a tick (a resize right after a refresh): the last share stands.
        Some((prev, share)) => busy_share(prev, now).unwrap_or(share),
        None => {
            std::thread::sleep(Duration::from_millis(100));
            let later = cpu_ticks()?;
            let share = busy_share(now, later).unwrap_or(0.0);
            *last = Some((later, share));
            return Some(share);
        }
    };
    *last = Some((now, share));
    Some(share)
}

/// Busy share of the ticks between two (busy, total) readings.
fn busy_share(prev: (u64, u64), now: (u64, u64)) -> Option<f64> {
    let total = now.1.checked_sub(prev.1).filter(|&t| t > 0)?;
    Some(now.0.saturating_sub(prev.0) as f64 / total as f64)
}

/// A line of the cat, bright white with yellow eyes and a pink nose.
fn paint(line: &str) -> String {
    line.replace('O', "\x1b[38;5;11mO\x1b[38;5;15m").replace('v', "\x1b[38;5;13mv\x1b[38;5;15m")
}

/// Widens the blank gaps (middle ones first) so the text is as tall as the cat.
fn stretch(lines: Vec<String>, height: usize) -> Vec<String> {
    let mut gaps: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].is_empty()).collect();
    gaps.sort_by_key(|&i| i.abs_diff(lines.len() / 2));
    let mut extra = vec![0; lines.len()];
    for k in 0..height.saturating_sub(lines.len()) {
        if let Some(&i) = gaps.get(k % gaps.len().max(1)) {
            extra[i] += 1;
        }
    }
    let mut out = Vec::new();
    for (line, n) in lines.into_iter().zip(extra) {
        out.extend(std::iter::repeat_n(String::new(), n));
        out.push(line);
    }
    out
}

/// The one shortcut worth knowing first: it leads to every other.
const TIP: &str = "Ctrl+Shift+P: the command palette, every action and theme";

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
        String::new(),
        format!("\x1b[1;38;5;11m{}\x1b[0;1m, {user}!\x1b[0m Welcome to \x1b[1;38;5;14mnoobsbot\x1b[0m.", greeting(now.0)),
        String::new(),
    ];
    let mut row = |label: &str, value: String| {
        if !value.is_empty() {
            lines.push(format!("\x1b[1;38;5;12m{label:<10}\x1b[0m{value}"));
        }
    };
    row("OS", format!("{} {}", os_name(), std::env::consts::ARCH));
    row("Kernel", first_line("/proc/sys/kernel/osrelease"));
    row("Uptime", uptime());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().trim_start_matches("X-").to_owned();
    row("Desktop", [desktop, packages()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "));
    row("Shell", shell.to_owned());
    row("Terminal", format!("noobsbot {}", env!("CARGO_PKG_VERSION")));
    row("CPU", cpu());
    row("GPU", tidy(gpu));
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    let mem = memory();
    let gib = |kib: f64| kib / 1024.0 / 1024.0;
    lines.push(String::new());
    let mut meter = |label: &str, bar: String, text: String| {
        lines.push(format!("\x1b[1;38;5;12m{label:<10}\x1b[0m{bar} {text}"));
    };
    // What the CPU is doing now; the load average (runnable threads, high on few cores
    // even when they're mostly idle) is kept as text, and is the meter only without it.
    match (cpu_use(), load()) {
        (Some(busy), load) => meter("CPU use", bar(busy, false), load.map_or(String::new(), |l| format!("load {l:.2}"))),
        (None, Some(load)) => meter("Load", bar(load / cores, false), format!("{load:.2} on {cores} cores")),
        (None, None) => {}
    }
    if let Some((total, free)) = mem.ram {
        meter("Memory", bar(1.0 - free / total, false), format!("{:.1} / {:.1} GiB", gib(total - free), gib(total)));
    }
    // Always listed after memory, so the screen keeps its height: `0.0 / 0.0 GiB` with
    // none set up (macOS makes swap files only once it needs them).
    let (total, free) = mem.swap.unwrap_or_default();
    let used = if total > 0.0 { 1.0 - free / total } else { 0.0 };
    meter("Swap", bar(used, false), format!("{:.1} / {:.1} GiB", gib(total - free), gib(total)));
    if let Some((total, free)) = disk("/") {
        meter("Disk", bar(1.0 - free / total, false), format!("{:.0} / {:.0} GiB", gib(total - free), gib(total)));
    }
    if let Some((level, status)) = battery() {
        meter("Battery", bar(level, true), status.to_lowercase());
    }
    if let Some(tag) = NEWER.lock().unwrap().as_deref() {
        lines.push(format!("\x1b[1;38;5;10m{:<10}\x1b[0m{tag} is out \x1b[38;5;8m·\x1b[0m run \x1b[1mnoobsbot --update\x1b[0m", "Update"));
    }
    lines.push(String::new());
    lines.push(format!("\x1b[38;5;8m{TIP}"));
    lines.push(String::new());
    lines.push("\x1b[38;5;11m\u{e0b6}\x1b[1;48;5;11;38;5;0m Designed & Developed by Noobsbot Team \x1b[0;38;5;11m\u{e0b4}\x1b[0m".into());
    lines
}

/// A meter for `level` (0..=1): green, yellow from 60 %, red from 85 %.
/// `full_is_good` (a battery): red up to 15 %, yellow up to 40 %.
fn bar(level: f64, full_is_good: bool) -> String {
    let level = level.clamp(0.0, 1.0);
    let filled = (level * BAR as f64).round() as usize;
    let badness = if full_is_good { 1.0 - level } else { level };
    let color = if badness >= 0.85 {
        1
    } else if badness >= 0.6 {
        3
    } else {
        2
    };
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
pub fn now() -> (u32, String) {
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

fn load() -> Option<f64> {
    first_line("/proc/loadavg").split(' ').next()?.parse().ok()
}

/// (busy, total) CPU ticks since boot, all cores.
fn cpu_ticks() -> Option<(u64, u64)> {
    proc_stat(&read_to_string("/proc/stat").ok()?)
}

/// `cpu  user nice system idle iowait irq softirq steal …` → (busy, total);
/// idle and iowait aren't busy.
fn proc_stat(text: &str) -> Option<(u64, u64)> {
    let ticks: Vec<u64> = text.lines().next()?.strip_prefix("cpu ")?.split_whitespace().take(8).map(|t| t.parse().ok()).collect::<Option<_>>()?;
    let total: u64 = ticks.iter().sum();
    let idle = ticks.get(3)? + ticks.get(4).unwrap_or(&0);
    Some((total.saturating_sub(idle), total))
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
        // Two eyes and one nose, colored without changing the width.
        assert_eq!((CAT.matches('O').count(), CAT.matches('v').count()), (2, 1));
        assert!(paint(":O:v:").contains("38;5;11mO") && paint(":O:v:").contains("38;5;13mv"));
        // Nothing beside the cat may wrap in the narrowest window that shows it.
        assert!(TIP.len() <= MIN_COLS - CAT_W - 3);
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
        assert!(screen(120, "gpu", "zsh").contains("$/.#....:======"));
        assert!(!screen(80, "gpu", "zsh").contains("$/.#....:======"));
        // Text stretched to the cat's height through its middle gaps; never shrunk.
        let s = |v: &[&str]| v.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        assert_eq!(stretch(s(&["a", "", "b", "", "c"]), 7), s(&["a", "", "", "b", "", "", "c"]));
        assert_eq!(stretch(s(&["a", "b"]), 4), s(&["a", "b"]));
        assert_eq!((greeting(8), greeting(14), greeting(19), greeting(2)), ("Good morning", "Good afternoon", "Good evening", "Up late"));
    }

    #[test]
    fn cpu_meter() {
        assert_eq!(busy_share((100, 400), (150, 500)), Some(0.5));
        assert_eq!(busy_share((100, 400), (100, 400)), None);
        let (busy, total) = cpu_ticks().unwrap();
        assert!(busy <= total && total > 0);
        let share = cpu_use().unwrap();
        assert!((0.0..=1.0).contains(&share));
        // Both meters always there, swap right after memory.
        let s = screen(120, "gpu", "zsh");
        let (cpu, mem, swap) = (s.find("CPU use").unwrap(), s.find("Memory").unwrap(), s.find("Swap").unwrap());
        assert!(cpu < mem && mem < swap);
        assert!(s[swap..].contains(" GiB"));
        assert_eq!(proc_stat("cpu  10 0 20 60 10 0 0 0 0 0\ncpu0 1 2 3 4\n"), Some((30, 100)));
        assert_eq!(proc_stat("intr 1 2 3\n"), None);
    }

    #[test]
    fn update_check() {
        assert_eq!(tag_name(r#"{"url": "x", "tag_name": "v0.2.1", "name": "v0.2.1"}"#).as_deref(), Some("v0.2.1"));
        assert_eq!(tag_name(r#"{"message": "API rate limit exceeded"}"#), None);
        assert!(newer("v0.2.1", "0.2.0") && newer("v0.10.0", "0.9.5") && newer("1.0", "0.9.9"));
        assert!(!newer("v0.2.0", "0.2.0") && !newer("v0.1.9", "0.2.0") && !newer("bogus", "0.2.0"));
    }

    #[test]
    fn redraws_in_place() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::index::{Column, Line, Point};
        use alacritty_terminal::term::Config;
        use alacritty_terminal::term::test::TermSize;
        let text = |term: &Term<VoidListener>, line: i32| (0..20).map(|c| term.grid()[Line(line)][Column(c)].c).collect::<String>().trim_end().to_owned();
        let mut term = Term::new(Config::default(), &TermSize::new(20, 8), VoidListener);
        let mut parser: Processor = Processor::new();
        let old = "\r\nup 5 min\r\nload 0.50\r\n\r\n";
        parser.advance(&mut term, old.as_bytes());
        parser.advance(&mut term, b"$ ls\x1b7");
        let cursor = term.grid().cursor.point;
        assert_eq!(cursor, Point::new(Line(4), Column(4)));
        // Shorter values leave nothing behind; the prompt, cursor and saved cursor stay.
        let mut rows = self::rows(old);
        assert!(redraw(&mut term, "\r\nup 6 min\r\nload 1\r\n\r\n", &mut rows));
        assert_eq!((text(&term, 1), text(&term, 2), text(&term, 4)), ("up 6 min".into(), "load 1".into(), "$ ls".into()));
        assert_eq!(term.grid().cursor.point, cursor);
        assert_eq!(term.grid().saved_cursor.point, cursor);
        // The prompt not below it, or no room below to grow: left alone.
        assert!(!redraw(&mut term, "\r\nx\r\ny\r\nz\r\nw\r\nv\r\n", &mut 5));
        assert!(!redraw(&mut term, "\r\n1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n\r\n", &mut rows));
        // Scrolled off the top, or under a full-screen app: left alone too.
        parser.advance(&mut term, b"\x1b[?1049h");
        assert!(!redraw(&mut term, old, &mut rows));
        parser.advance(&mut term, b"\x1b[?1049l\r\n\n\n\n\n\n");
        assert!(!redraw(&mut term, old, &mut rows));
        // Three lines scrolled up: the prompt moved with them, nothing was drawn over it.
        assert_eq!(text(&term, 1), "$ ls");
    }

    #[test]
    fn redraws_at_a_new_height() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::index::{Column, Line, Point};
        use alacritty_terminal::term::Config;
        use alacritty_terminal::term::test::TermSize;
        let text = |term: &Term<VoidListener>, line: i32| (0..20).map(|c| term.grid()[Line(line)][Column(c)].c).collect::<String>().trim_end().to_owned();
        let mut term = Term::new(Config::default(), &TermSize::new(20, 8), VoidListener);
        let mut parser: Processor = Processor::new();
        let old = "\r\nup 5 min\r\n\r\n";
        parser.advance(&mut term, old.as_bytes());
        parser.advance(&mut term, b"$ ls\x1b7");
        let mut rows = self::rows(old);
        // The update line arrives after the first draw: the prompt moves down with it.
        assert!(redraw(&mut term, "\r\nup 5 min\r\nv9 is out\r\n\r\n", &mut rows));
        assert_eq!((rows, text(&term, 2), text(&term, 4)), (4, "v9 is out".into(), "$ ls".into()));
        assert_eq!(term.grid().cursor.point, Point::new(Line(4), Column(4)));
        assert_eq!(term.grid().saved_cursor.point, Point::new(Line(4), Column(4)));
        // And back up when it's shorter; an empty screen takes it all away.
        assert!(redraw(&mut term, old, &mut rows));
        assert_eq!((rows, text(&term, 1), text(&term, 2), text(&term, 3)), (3, "up 5 min".into(), "".into(), "$ ls".into()));
        assert!(redraw(&mut term, "", &mut rows));
        assert_eq!((rows, text(&term, 0), term.grid().cursor.point), (0, "$ ls".into(), Point::new(Line(0), Column(4))));
        assert_eq!(term.grid().history_size(), 0);
        // The real screen at its real heights.
        found(None);
        let mut term = Term::new(Config::default(), &TermSize::new(121, 40), VoidListener);
        let before = screen(121, "gpu", "zsh");
        parser.advance(&mut term, before.as_bytes());
        parser.advance(&mut term, b"$ ");
        let mut rows = self::rows(&before);
        found(Some("v99.0.0".into()));
        let after = screen(121, "gpu", "zsh");
        found(None);
        assert!(self::rows(&after) > rows);
        assert!(redraw(&mut term, &after, &mut rows));
        assert_eq!(rows, self::rows(&after));
        assert_eq!(term.grid().cursor.point.line.0 as usize, rows);
    }

    #[test]
    fn never_wraps() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::index::{Column, Line};
        use alacritty_terminal::term::Config;
        use alacritty_terminal::term::test::TermSize;
        assert_eq!(fit("\x1b[1mabcdef\x1b[0m", 3), "\x1b[1mabc\x1b[0m");
        for cols in [2, 20, 40, 60, 94, 95, 121] {
            let s = screen(cols, "Intel(R) UHD Graphics 617", "zsh");
            let mut term = Term::new(Config::default(), &TermSize::new(cols, 60), VoidListener);
            let mut parser: Processor = Processor::new();
            parser.advance(&mut term, s.as_bytes());
            assert_eq!(term.grid().cursor.point.line.0 as usize, self::rows(&s), "{cols} columns: a line wrapped");
        }
        // A refresh with a longer line is cut too, and leaves the prompt alone.
        let mut term = Term::new(Config::default(), &TermSize::new(20, 10), VoidListener);
        let mut parser: Processor = Processor::new();
        let old = format!("\r\n{}\r\n\r\n", "a".repeat(10));
        parser.advance(&mut term, old.as_bytes());
        parser.advance(&mut term, b"$ ls");
        let mut rows = self::rows(&old);
        assert!(redraw(&mut term, &format!("\r\n{}\r\n\r\n", "b".repeat(45)), &mut rows));
        let text = |line: i32| (0..20).map(|c| term.grid()[Line(line)][Column(c)].c).collect::<String>().trim_end().to_owned();
        assert_eq!((text(1), text(3)), ("b".repeat(20), "$ ls".into()));
    }

    #[test]
    fn resize_keeps_what_the_shell_printed() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::index::{Column, Line};
        use alacritty_terminal::term::Config;
        use alacritty_terminal::term::test::TermSize;
        let text =
            |term: &Term<VoidListener>, line: i32| (0..term.columns()).map(|c| term.grid()[Line(line)][Column(c)].c).collect::<String>().trim_end().to_owned();
        let mut term = Term::new(Config::default(), &TermSize::new(60, 50), VoidListener);
        let mut parser: Processor = Processor::new();
        let before = screen(60, "gpu", "zsh");
        parser.advance(&mut term, before.as_bytes());
        parser.advance(&mut term, b"Last login: Fri\r\n$ ");
        let mut rows = self::rows(&before);
        // What the app does on a resize: take it out, reflow the rest, put it back.
        assert!(redraw(&mut term, "", &mut rows));
        term.resize(TermSize::new(30, 50));
        let after = screen(30, "gpu", "zsh");
        assert!(redraw(&mut term, &after, &mut rows));
        assert_eq!(rows, self::rows(&after));
        assert_eq!((text(&term, rows as i32), text(&term, rows as i32 + 1)), ("Last login: Fri".into(), "$".into()));
        assert_eq!(term.grid().cursor.point.line.0 as usize, rows + 1);
    }
}
