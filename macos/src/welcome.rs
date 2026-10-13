//! The welcome screen (terminal.welcome): a text-drawn cat beside the
//! machine's specs, shown in a new window before the shell's first prompt.
//! Read with sysctl and friends; only the battery spawns `pmset`.

use std::fs::read_to_string;

/// Colons are the fur (dense enough to read on black); dashes the inner ears,
/// stripes, eyes, whiskers, mouth, collar and toes.
const CAT: &str = r"
     ::                  ::
    ::-::              ::-::
    ::---::          ::---::
    ::----:::-:--:-:::----::
   :::::::::::-::-:::::::::::
  ::::::::::::::::::::::::::::
  :::::-----::::::::-----:::::
  :::::- o -::::::::- o -:::::
  :::::-----::::::::-----:::::
  :::::::::::::vv:::::::::::::
---- ::::::::::--:::::::::: ----
 ----::::::::--::--::::::::----
---- ::::::---::::---:::::: ----
       ::::::::::::::::::
        -------::-------
     ::::::::::::::::::::::
    ---::::::::::::::::::---
   ::::::::::::::::::::::::::
  ---::::::::::::::::::::::---    :::
  ::::::::::::::::::::::::::::   :--::
  ---::::::::::::::::::::::---  :::::
   ::::::::::::  :::::::::::: ::--:
    ::-::-::-:    :-::-::-:: :::::
     ----------------------::::
";
const CAT_W: usize = 38;
/// Narrower than this, the cat is left out.
const MIN_COLS: usize = CAT_W + 3 + 57;
const BAR: usize = 20;

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
const TIP: &str = "Cmd+Shift+P: the command palette, every action and theme";

fn info(gpu: &str, shell: &str) -> Vec<String> {
    let user = std::env::var("USER").unwrap_or_else(|_| "you".into());
    let now = now();
    // Like the pretty prompt: user, host, date. The terminal draws these separators itself.
    let header = format!(
        "\x1b[38;5;11m\u{e0b6}\x1b[48;5;11;38;5;0m {user} \x1b[38;5;11;48;5;12m\u{e0b0}\x1b[38;5;15m {} \x1b[38;5;12;48;5;14m\u{e0b0}\x1b[38;5;0m {} \x1b[0;38;5;14m\u{e0b4}",
        hostname(),
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
    row("Kernel", kernel());
    row("Uptime", uptime());
    row("Desktop", ["Aqua".to_owned(), packages()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "));
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
    if let Some(load) = load() {
        meter("Load", bar(load / cores, false), format!("{load:.2} on {cores} cores"));
    }
    if let Some((total, free)) = mem.ram {
        meter("Memory", bar(1.0 - free / total, false), format!("{:.1} / {:.1} GiB", gib(total - free), gib(total)));
    }
    if let Some((total, free)) = mem.swap.filter(|s| s.0 > 0.0) {
        meter("Swap", bar(1.0 - free / total, false), format!("{:.1} / {:.1} GiB", gib(total - free), gib(total)));
    }
    if let Some((total, free)) = disk("/System/Volumes/Data").or_else(|| disk("/")) {
        meter("Disk", bar(1.0 - free / total, false), format!("{:.0} / {:.0} GiB", gib(total - free), gib(total)));
    }
    if let Some((level, status)) = battery() {
        meter("Battery", bar(level, true), status.to_lowercase());
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

/// This Mac's name, without ".local".
pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: gethostname writes at most buf.len() bytes.
    let n = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    let name = if n == 0 { c_str(&buf) } else { String::new() };
    name.strip_suffix(".local").unwrap_or(&name).to_owned()
}

fn c_str(buf: &[u8]) -> String {
    String::from_utf8_lossy(&buf[..buf.iter().position(|&b| b == 0).unwrap_or(buf.len())]).into_owned()
}

/// A sysctl value, raw: `T` must be plain old data.
fn sysctl<T: Copy>(name: &std::ffi::CStr) -> Option<T> {
    let mut value = std::mem::MaybeUninit::<T>::uninit();
    let mut len = size_of::<T>();
    // SAFETY: the kernel writes at most `len` bytes into `value`.
    let ok = unsafe { libc::sysctlbyname(name.as_ptr(), value.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) == 0 };
    // SAFETY: filled in full, checked by `len`.
    (ok && len == size_of::<T>()).then(|| unsafe { value.assume_init() })
}

fn sysctl_text(name: &std::ffi::CStr) -> String {
    let mut buf = [0u8; 256];
    let mut len = buf.len();
    // SAFETY: as above.
    let ok = unsafe { libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) == 0 };
    if ok { c_str(&buf[..len.min(buf.len())]) } else { String::new() }
}

/// "macOS 15.1".
fn os_name() -> String {
    let text = read_to_string("/System/Library/CoreServices/SystemVersion.plist").unwrap_or_default();
    plist_string(&text, "ProductVersion").map_or("macOS".into(), |v| format!("macOS {v}"))
}

/// The <string> after <key>`key`</key> in an XML plist.
fn plist_string(text: &str, key: &str) -> Option<String> {
    let rest = &text[text.find(&format!("<key>{key}</key>"))?..];
    let start = rest.find("<string>")? + "<string>".len();
    Some(rest[start..start + rest[start..].find("</string>")?].trim().to_owned())
}

/// "Darwin 24.1.0".
fn kernel() -> String {
    // SAFETY: uname fills the struct.
    let name = unsafe {
        let mut name: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut name) != 0 {
            return String::new();
        }
        name
    };
    let text = |f: &[libc::c_char]| c_str(&f.iter().map(|&c| c as u8).collect::<Vec<_>>());
    format!("{} {}", text(&name.sysname), text(&name.release))
}

fn load() -> Option<f64> {
    let mut load = [0f64; 3];
    // SAFETY: room for the one value asked for.
    (unsafe { libc::getloadavg(load.as_mut_ptr(), 1) } == 1).then_some(load[0])
}

fn uptime() -> String {
    let Some(boot) = sysctl::<libc::timeval>(c"kern.boottime") else { return String::new() };
    // SAFETY: time with a null pointer just returns it.
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    duration((now - boot.tv_sec).max(0) as u64)
}

fn duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60);
    match (d, h) {
        (0, 0) => format!("{m} min"),
        (0, _) => format!("{h} h {m} min"),
        _ => format!("{d} d {h} h"),
    }
}

/// "Apple M2 Pro (12)".
fn cpu() -> String {
    let model = sysctl_text(c"machdep.cpu.brand_string");
    let threads = sysctl::<i32>(c"hw.ncpu").unwrap_or(0);
    if model.is_empty() { String::new() } else { format!("{} ({threads})", tidy(&model)) }
}

/// Hardware names without the legal and filler words:
/// "Intel(R) Core(TM) i7-8550U CPU @ 1.80GHz" -> "Intel Core i7-8550U".
fn tidy(name: &str) -> String {
    let name = name.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    let name = name.split(" @ ").next().unwrap_or("");
    name.split_whitespace().filter(|w| !matches!(*w, "CPU" | "Processor")).collect::<Vec<_>>().join(" ")
}

/// Homebrew formulae and casks, one directory listing each.
fn packages() -> String {
    let count = |dir: &str| std::fs::read_dir(dir).map_or(0, |d| d.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).count());
    let prefix = if std::path::Path::new("/opt/homebrew/Cellar").is_dir() { "/opt/homebrew" } else { "/usr/local" };
    match count(&format!("{prefix}/Cellar")) + count(&format!("{prefix}/Caskroom")) {
        0 => String::new(),
        n => format!("{n} packages (brew)"),
    }
}

/// The internal battery's charge (0..=1) and "charging" / "discharging" / "charged".
fn battery() -> Option<(f64, String)> {
    let out = std::process::Command::new("/usr/bin/pmset").args(["-g", "batt"]).stderr(std::process::Stdio::null()).output().ok()?;
    pmset(&String::from_utf8_lossy(&out.stdout))
}

fn pmset(text: &str) -> Option<(f64, String)> {
    // " -InternalBattery-0 (id=123)\t87%; discharging; 3:12 remaining present: true"
    let line = text.lines().find(|l| l.contains("InternalBattery"))?;
    let mut fields = line.split('\t').nth(1)?.split(';').map(str::trim);
    let level = fields.next()?.strip_suffix('%')?.parse::<f64>().ok()? / 100.0;
    Some((level, fields.next().unwrap_or("").to_owned()))
}

#[derive(Default, Debug, PartialEq)]
struct Memory {
    /// (total, available) in KiB.
    ram: Option<(f64, f64)>,
    swap: Option<(f64, f64)>,
}

/// Used memory counted as Activity Monitor does: app + wired + compressed.
fn memory() -> Memory {
    let total = sysctl::<u64>(c"hw.memsize").map(|b| b as f64 / 1024.0);
    let page = sysctl::<u64>(c"hw.pagesize").unwrap_or(16384) as f64 / 1024.0;
    // SAFETY: host_statistics64 fills at most `count` integers of `vm`.
    #[allow(deprecated)]
    let used = unsafe {
        let mut vm: libc::vm_statistics64 = std::mem::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        (libc::host_statistics64(libc::mach_host_self(), libc::HOST_VM_INFO64, (&raw mut vm).cast(), &mut count) == 0)
            .then(|| (vm.internal_page_count.saturating_sub(vm.purgeable_count) as f64 + vm.wire_count as f64 + vm.compressor_page_count as f64) * page)
    };
    let swap = sysctl::<libc::xsw_usage>(c"vm.swapusage").map(|s| (s.xsu_total as f64 / 1024.0, s.xsu_avail as f64 / 1024.0));
    Memory { ram: total.zip(used).map(|(t, u)| (t, (t - u).max(0.0))), swap }
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
        // Nothing beside the cat may wrap in the narrowest window that shows it.
        assert!(TIP.len() <= MIN_COLS - CAT_W - 3);
    }

    #[test]
    fn parses_system_files() {
        let plist = "<dict>\n\t<key>ProductName</key>\n\t<string>macOS</string>\n\t<key>ProductVersion</key>\n\t<string>15.1</string>\n</dict>";
        assert_eq!(plist_string(plist, "ProductVersion").as_deref(), Some("15.1"));
        assert_eq!(plist_string(plist, "BuildVersion"), None);
        let batt = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1234)\t87%; discharging; 3:12 remaining present: true\n";
        assert_eq!(pmset(batt), Some((0.87, "discharging".into())));
        assert_eq!(pmset("Now drawing from 'AC Power'\n"), None);
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
        // Text stretched to the cat's height through its middle gaps; never shrunk.
        let s = |v: &[&str]| v.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        assert_eq!(stretch(s(&["a", "", "b", "", "c"]), 7), s(&["a", "", "", "b", "", "", "c"]));
        assert_eq!(stretch(s(&["a", "b"]), 4), s(&["a", "b"]));
        assert_eq!((greeting(8), greeting(14), greeting(19), greeting(2)), ("Good morning", "Good afternoon", "Good evening", "Up late"));
    }
}
