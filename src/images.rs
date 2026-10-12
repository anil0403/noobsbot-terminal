//! Inline images: the kitty graphics protocol and sixel. Decoded pictures are
//! kept here; where they show is marked on the grid by tagged cells (like
//! prompts), so images scroll with the text and go when it does.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use alacritty_terminal::Term;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Line;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Cell;
use base64::Engine;

/// Hyperlink id prefix (and URI) of image cells; the id ends in `<placement>-<row>`.
pub const IMAGE_TAG: &str = "noobsbot-image-";

/// Largest side we accept; also what every GPU can hold in one texture.
const MAX_SIDE: u32 = 8192;
/// Memory for kitty images transmitted for later placement.
const KITTY_QUOTA: usize = 256 << 20;

/// A decoded picture, RGBA8.
pub struct Pic {
    /// Unique for the life of the program, for the renderer's texture cache.
    pub uid: u64,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Pic {
    fn new(w: u32, h: u32, rgba: Vec<u8>) -> Result<Arc<Self>, String> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(format!("EINVAL:bad size {w}x{h}"));
        }
        Ok(Arc::new(Self { uid: NEXT.fetch_add(1, Ordering::Relaxed), w, h, rgba }))
    }
}

/// An image on the grid.
pub struct Placed {
    pub pic: Arc<Pic>,
    kitty_id: Option<u32>,
    /// Left column, and the size in cells (fractional: images keep their pixel size).
    pub col: usize,
    pub cols: f32,
    pub rows: f32,
}

/// Room to make on the grid for a new image.
pub struct Place {
    n: u64,
    cols: usize,
    rows: usize,
    /// Leave the cursor after the image (kitty `C=0`), or where it was.
    move_cursor: bool,
    /// Sixel: the cursor goes to the line below.
    newline: bool,
}

#[derive(Default)]
pub struct Images {
    /// Cell size in pixels when the image arrives, to size it in cells.
    pub cell: (f32, f32),
    pub placed: HashMap<u64, Placed>,
    next: u64,
    /// Transmitted kitty images by id, with when they came (for the quota).
    kitty: HashMap<u32, (Arc<Pic>, u64)>,
    /// A kitty command still arriving in chunks (`m=1`): its keys and data.
    chunk: Option<(Keys, Vec<u8>)>,
}

impl Images {
    /// Run a kitty graphics command (the APC after `G`). Returns room to make
    /// on the grid, and the reply for the program.
    pub fn kitty(&mut self, cmd: &[u8], col: usize) -> (Option<Place>, Option<String>) {
        let (ctrl, payload) = match cmd.iter().position(|&b| b == b';') {
            Some(i) => (&cmd[..i], &cmd[i + 1..]),
            None => (cmd, &b""[..]),
        };
        let keys = Keys::parse(&String::from_utf8_lossy(ctrl));
        let more = keys.num('m') == Some(1);
        let (keys, payload) = match self.chunk.take() {
            Some((first, mut data)) => {
                if data.len() < 400 << 20 {
                    data.extend_from_slice(payload);
                }
                if more {
                    self.chunk = Some((first, data));
                    return (None, None);
                }
                (first, data)
            }
            None if more => {
                self.chunk = Some((keys, payload.to_vec()));
                return (None, None);
            }
            None => (keys, payload.to_vec()),
        };
        let result = self.run(&keys, &payload, col);
        // Reply only when the program gave an id, and didn't ask for quiet.
        let quiet = keys.num('q').unwrap_or(0);
        let (id, number) = (keys.num('i'), keys.num('I'));
        let reply = match &result {
            _ if id.is_none() && number.is_none() || keys.char('a') == Some('d') => None,
            Ok(_) if quiet >= 1 => None,
            Err(_) if quiet >= 2 => None,
            result => {
                let mut r = String::from("\x1b_G");
                let _ = write!(r, "i={}", id.or(number).unwrap_or(0));
                if let Some(n) = number {
                    let _ = write!(r, ",I={n}");
                }
                let msg = result.as_ref().err().map_or("OK", |e| e.as_str());
                let _ = write!(r, ";{msg}\x1b\\");
                Some(r)
            }
        };
        (result.unwrap_or(None), reply)
    }

    fn run(&mut self, keys: &Keys, payload: &[u8], col: usize) -> Result<Option<Place>, String> {
        // ponytail: `I` (image number) is treated as the id, not given a fresh one.
        let id = keys.num('i').or(keys.num('I'));
        match keys.char('a').unwrap_or('t') {
            'd' => {
                match (keys.char('d').unwrap_or('a'), id) {
                    (d @ ('i' | 'I' | 'n' | 'N'), Some(id)) => {
                        self.placed.retain(|_, p| p.kitty_id != Some(id));
                        if d.is_ascii_uppercase() {
                            self.kitty.remove(&id);
                        }
                    }
                    // ponytail: every other target (cursor, cell, z-index…) clears all placements.
                    (d, _) => {
                        self.placed.clear();
                        if d.is_ascii_uppercase() {
                            self.kitty.clear();
                        }
                    }
                }
                Ok(None)
            }
            'p' => {
                let pic = self.kitty.get(&id.ok_or("EINVAL:no id")?).ok_or("ENOENT:no such image")?.0.clone();
                Ok(Some(self.place(pic, id, keys, col)))
            }
            'q' => load(keys, payload).map(|_| None),
            'T' => {
                let pic = load(keys, payload)?;
                if let Some(id) = id {
                    self.store(id, pic.clone());
                }
                Ok(Some(self.place(pic, id, keys, col)))
            }
            't' => {
                self.store(id.ok_or("EINVAL:no id")?, load(keys, payload)?);
                Ok(None)
            }
            a => Err(format!("EINVAL:action {a}")),
        }
    }

    fn store(&mut self, id: u32, pic: Arc<Pic>) {
        self.next += 1;
        self.kitty.insert(id, (pic, self.next));
        while self.kitty.values().map(|(p, _)| p.rgba.len()).sum::<usize>() > KITTY_QUOTA {
            let oldest = *self.kitty.iter().min_by_key(|(_, (_, at))| at).unwrap().0;
            self.kitty.remove(&oldest);
        }
    }

    fn place(&mut self, pic: Arc<Pic>, kitty_id: Option<u32>, keys: &Keys, col: usize) -> Place {
        let (cw, ch) = self.cell;
        let (w, h) = (pic.w as f32, pic.h as f32);
        // `c`/`r`: fit that many columns/rows, keeping the aspect if only one is given.
        let (cols, rows) = match (keys.num('c').filter(|&c| c > 0), keys.num('r').filter(|&r| r > 0)) {
            (Some(c), Some(r)) => (c as f32, r as f32),
            (Some(c), None) => (c as f32, c as f32 * cw * h / w / ch),
            (None, Some(r)) => (r as f32 * ch * w / h / cw, r as f32),
            (None, None) => (w / cw, h / ch),
        };
        let place = self.add(pic, kitty_id, col, cols, rows);
        Place { move_cursor: keys.num('C') != Some(1), ..place }
    }

    fn add(&mut self, pic: Arc<Pic>, kitty_id: Option<u32>, col: usize, cols: f32, rows: f32) -> Place {
        self.next += 1;
        self.placed.insert(self.next, Placed { pic, kitty_id, col, cols, rows });
        Place { n: self.next, cols: cols.ceil().max(1.0) as usize, rows: rows.ceil().max(1.0) as usize, move_cursor: true, newline: false }
    }

    /// Decode a sixel image (the DCS after `P`) and make room for it.
    pub fn sixel(&mut self, dcs: &[u8], col: usize) -> Option<Place> {
        let pic = sixel(dcs).map_err(|e| log::warn!("sixel: {e}")).ok()?;
        let (cols, rows) = (pic.w as f32 / self.cell.0, pic.h as f32 / self.cell.1);
        Some(Place { newline: true, ..self.add(pic, None, col, cols, rows) })
    }

    /// Forget images whose cells are all gone (scrolled out, cleared, overwritten).
    pub fn evict<T>(&mut self, term: &Term<T>) {
        // The other screen's cells can't be seen from here; wait until we're back.
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        let mut live = HashSet::new();
        for line in term.topmost_line().0..=term.bottommost_line().0 {
            for cell in &term.grid()[Line(line)][..] {
                if let Some((n, _)) = tag(cell) {
                    live.insert(n);
                }
            }
        }
        self.placed.retain(|n, _| live.contains(n));
    }
}

/// Placement number and image row of an image cell.
pub fn tag(cell: &Cell) -> Option<(u64, usize)> {
    if cell.c != ' ' {
        return None;
    }
    let link = cell.hyperlink()?;
    let (n, row) = link.id().strip_prefix(IMAGE_TAG)?.split_once('-')?;
    Some((n.parse().ok()?, row.parse().ok()?))
}

/// What to feed the terminal to make room for `place` at the cursor: rows of
/// blank cells tagged with the image, and the cursor where it goes after.
pub fn room<T>(term: &Term<T>, place: &Place) -> String {
    let col = cursor_col(term);
    let width = place.cols.min(term.columns() - col).max(1);
    let mut s = String::new();
    if !place.move_cursor {
        s.push_str("\x1b7");
    }
    for row in 0..place.rows {
        if row > 0 {
            s.push_str("\n\r");
            if col > 0 {
                let _ = write!(s, "\x1b[{col}C");
            }
        }
        let _ = write!(s, "\x1b]8;id={IMAGE_TAG}{}-{row};{IMAGE_TAG}\x1b\\{}\x1b]8;;\x1b\\", place.n, " ".repeat(width));
    }
    if place.newline {
        s.push_str("\r\n");
    }
    if !place.move_cursor {
        s.push_str("\x1b8");
    }
    s
}

/// The cursor column, for placing an image there.
pub fn cursor_col<T>(term: &Term<T>) -> usize {
    term.grid().cursor.point.column.0.min(term.columns() - 1)
}

/// Kitty control keys: `a=T,f=100,i=3`.
struct Keys(Vec<(char, String)>);

impl Keys {
    fn parse(s: &str) -> Self {
        Self(s.split(',').filter_map(|kv| kv.split_once('=')).filter_map(|(k, v)| Some((k.chars().next()?, v.to_owned()))).collect())
    }

    fn get(&self, k: char) -> Option<&str> {
        self.0.iter().find(|(key, _)| *key == k).map(|(_, v)| v.as_str())
    }

    fn num(&self, k: char) -> Option<u32> {
        self.get(k)?.parse().ok()
    }

    fn char(&self, k: char) -> Option<char> {
        self.get(k)?.chars().next()
    }
}

/// Decode a kitty image from the payload, wherever it says the data is.
fn load(keys: &Keys, payload: &[u8]) -> Result<Arc<Pic>, String> {
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new().with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let payload: Vec<u8> = payload.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    let data = engine.decode(payload).map_err(|e| format!("EINVAL:base64: {e}"))?;
    let data = match keys.char('t').unwrap_or('d') {
        'd' => data,
        medium @ ('f' | 't' | 's') => read_file(medium, &String::from_utf8_lossy(&data), keys)?,
        m => return Err(format!("EINVAL:medium {m}")),
    };
    let data = match keys.char('o') {
        Some('z') => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&data, (MAX_SIDE * MAX_SIDE * 4) as usize).map_err(|e| format!("EINVAL:zlib: {e:?}"))?,
        _ => data,
    };
    match keys.num('f').unwrap_or(32) {
        100 => png(&data),
        f @ (24 | 32) => {
            let (w, h) = (keys.num('s').unwrap_or(0), keys.num('v').unwrap_or(0));
            let bpp = f as usize / 8;
            if data.len() != w as usize * h as usize * bpp {
                return Err(format!("EINVAL:{} bytes for {w}x{h}", data.len()));
            }
            let rgba = if bpp == 4 { data } else { data.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect() };
            Pic::new(w, h, rgba)
        }
        f => Err(format!("EINVAL:format {f}")),
    }
}

/// Image data sent as a file (`t=f`), temp file (`t=t`, deleted after) or
/// shared memory (`t=s`). Like kitty, refuse anything that isn't a plain file
/// or sits in /proc, /sys or /dev: programs only get to show files, not probe devices.
fn read_file(medium: char, name: &str, keys: &Keys) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let path = match medium {
        's' if !name.contains('/') => std::path::PathBuf::from("/dev/shm").join(name.trim_start_matches('/')),
        's' => return Err("EINVAL:bad shm name".into()),
        _ => std::path::PathBuf::from(name),
    };
    let path = path.canonicalize().map_err(|e| format!("EBADF:{e}"))?;
    let banned = ["/proc", "/sys", "/dev"].iter().any(|d| path.starts_with(d)) && !path.starts_with("/dev/shm");
    if banned || !path.is_file() {
        return Err("EBADF:not a regular file".into());
    }
    let mut file = std::fs::File::open(&path).map_err(|e| format!("EBADF:{e}"))?;
    file.seek(SeekFrom::Start(keys.num('O').unwrap_or(0) as u64)).map_err(|e| format!("EBADF:{e}"))?;
    let mut data = Vec::new();
    let size = keys.num('S').filter(|&s| s > 0).map_or(400 << 20, |s| s as u64);
    file.take(size).read_to_end(&mut data).map_err(|e| format!("EBADF:{e}"))?;
    // Only delete what the protocol says is ours to delete: shared memory, and
    // temp files with kitty's marker in the name.
    let temp = path.starts_with(std::env::temp_dir()) || path.starts_with("/tmp") || path.starts_with("/dev/shm");
    if medium == 's' || medium == 't' && temp && path.to_string_lossy().contains("tty-graphics-protocol") {
        let _ = std::fs::remove_file(&path);
    }
    Ok(data)
}

fn png(data: &[u8]) -> Result<Arc<Pic>, String> {
    let limits = png::Limits { bytes: (MAX_SIDE * MAX_SIDE * 4) as usize };
    let mut decoder = png::Decoder::new_with_limits(std::io::Cursor::new(data), limits);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("EINVAL:png: {e}"))?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("EINVAL:png too big")?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("EINVAL:png: {e}"))?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("EINVAL:png palette".into()),
    };
    Pic::new(info.width, info.height, rgba)
}

/// Decode sixel: `P1;P2;P3 q` then the data. Pixels no color was drawn on
/// stay transparent.
fn sixel(dcs: &[u8]) -> Result<Arc<Pic>, String> {
    let q = dcs.iter().position(|&b| b == b'q').ok_or("no q")?;
    let body = &dcs[q + 1..];
    // Two passes: find the size, then paint.
    let (mut w, mut h) = (0u32, 0u32);
    let (pw, ph) = run_sixel(body, |x, y, _| {
        w = w.max(x + 1);
        h = h.max(y + 1);
    })?;
    let (w, h) = (w.max(pw), h.max(ph));
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(format!("bad size {w}x{h}"));
    }
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    run_sixel(body, |x, y, c| {
        let i = ((y * w + x) * 4) as usize;
        rgba[i..i + 4].copy_from_slice(&c);
    })?;
    Pic::new(w, h, rgba)
}

/// Walk sixel data, calling `plot` for every pixel set. Returns the declared size.
fn run_sixel(body: &[u8], mut plot: impl FnMut(u32, u32, [u8; 4])) -> Result<(u32, u32), String> {
    let mut raster = (0, 0);
    // VT340 defaults; images nearly always define their own.
    let mut palette = [[0u8; 4]; 256];
    for (i, &(r, g, b)) in [(0, 0, 0), (20, 20, 80), (80, 13, 13), (20, 80, 20), (80, 20, 80), (20, 80, 80), (80, 80, 20), (53, 53, 53), (26, 26, 26), (33, 33, 60), (60, 26, 26), (33, 60, 33), (60, 33, 60), (33, 60, 60), (60, 60, 33), (80, 80, 80)].iter().enumerate() {
        palette[i] = [pct(r), pct(g), pct(b), 255];
    }
    let (mut x, mut y, mut color, mut repeat) = (0u32, 0u32, 0usize, 1u32);
    let mut i = 0;
    // Numbers after a command byte: `#1;2;100;0;0`.
    let nums = |i: &mut usize| -> Vec<u32> {
        let mut out = Vec::new();
        loop {
            let start = *i;
            while *i < body.len() && body[*i].is_ascii_digit() {
                *i += 1;
            }
            let n = std::str::from_utf8(&body[start..*i]).ok().and_then(|s| s.parse().ok()).unwrap_or(0u32);
            out.push(n.min(1 << 20));
            if *i < body.len() && body[*i] == b';' {
                *i += 1;
            } else {
                return out;
            }
        }
    };
    while i < body.len() {
        let b = body[i];
        i += 1;
        match b {
            b'"' => {
                let n = nums(&mut i);
                if let [_, _, pw, ph, ..] = n[..] {
                    if pw > MAX_SIDE || ph > MAX_SIDE {
                        return Err(format!("bad size {pw}x{ph}"));
                    }
                    raster = (pw, ph);
                }
            }
            b'#' => {
                let n = nums(&mut i);
                color = n[0] as usize % 256;
                match n[..] {
                    [_, 2, r, g, b] => palette[color] = [pct(r), pct(g), pct(b), 255],
                    [_, 1, h, l, s] => palette[color] = hls(h, l, s),
                    _ => {}
                }
            }
            b'!' => repeat = nums(&mut i)[0].clamp(1, MAX_SIDE),
            b'$' => x = 0,
            b'-' => {
                x = 0;
                y += 6;
            }
            b'?'..=b'~' => {
                let bits = b - b'?';
                if x + repeat > MAX_SIDE || y + 6 > MAX_SIDE + 6 {
                    return Err("too big".into());
                }
                for dx in 0..repeat {
                    for k in 0..6 {
                        if bits >> k & 1 == 1 && y + k < MAX_SIDE {
                            plot(x + dx, y + k, palette[color]);
                        }
                    }
                }
                x += repeat;
                repeat = 1;
            }
            _ => {}
        }
    }
    Ok(raster)
}

fn pct(v: u32) -> u8 {
    (v.min(100) * 255 / 100) as u8
}

/// DEC HLS (hue 0 = blue) to RGBA.
fn hls(h: u32, l: u32, s: u32) -> [u8; 4] {
    let (h, l, s) = (((h + 240) % 360) as f32, l.min(100) as f32 / 100.0, s.min(100) as f32 / 100.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f32| ((v + m) * 255.0).round() as u8;
    [to(r), to(g), to(b), 255]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn images() -> Images {
        Images { cell: (10.0, 20.0), ..Default::default() }
    }

    #[test]
    fn sixel_decodes() {
        // Red 2x6 column pair, then a green pixel on the next band's first row.
        let pic = sixel(b"0;1;0q\"1;1;3;7#1;2;100;0;0!2~-#2;2;0;100;0@").unwrap();
        assert_eq!((pic.w, pic.h), (3, 7)); // raster size wins over painted
        assert_eq!(&pic.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&pic.rgba[(5 * 3 + 1) * 4..][..4], &[255, 0, 0, 255]);
        assert_eq!(&pic.rgba[2 * 4..][..4], &[0, 0, 0, 0]); // never painted: clear
        assert_eq!(&pic.rgba[6 * 3 * 4..][..4], &[0, 255, 0, 255]);
        assert!(sixel(b"q!8000~!8000~").is_err());
    }

    #[test]
    fn kitty_raw_chunks_and_replies() {
        let mut im = images();
        // 2x1 RGB, sent in two chunks, then placed by id.
        let data = base64::engine::general_purpose::STANDARD.encode([255, 0, 0, 0, 0, 255]);
        let (a, b) = data.split_at(4);
        assert!(im.kitty(format!("a=t,f=24,s=2,v=1,i=7,m=1;{a}").as_bytes(), 0).1.is_none());
        let (place, reply) = im.kitty(format!("m=0;{b}").as_bytes(), 0);
        assert!(place.is_none());
        assert_eq!(reply.as_deref(), Some("\x1b_Gi=7;OK\x1b\\"));
        let (place, _) = im.kitty(b"a=p,i=7,r=2", 3);
        let place = place.unwrap();
        assert_eq!((place.cols, place.rows), (8, 2)); // 2 rows = 40px tall, so 80px wide
        assert_eq!(im.placed[&place.n].col, 3);
        let (_, reply) = im.kitty(b"a=p,i=9", 0);
        assert!(reply.unwrap().contains("ENOENT"));
        im.kitty(b"a=d,d=I,i=7", 0);
        assert!(im.placed.is_empty() && im.kitty.is_empty());
        // Bad sizes and data are refused, quietly if asked.
        assert!(im.kitty(b"a=T,f=24,s=2,v=2,i=1;AAAA", 0).1.unwrap().contains("EINVAL"));
        assert!(im.kitty(b"a=T,f=24,s=2,v=2,i=1,q=2;AAAA", 0).1.is_none());
    }

    #[test]
    fn refuses_device_files() {
        let keys = Keys::parse("t=f");
        assert!(read_file('f', "/proc/self/environ", &keys).is_err());
        assert!(read_file('f', "/dev/zero", &keys).is_err());
        assert!(read_file('s', "../etc/passwd", &keys).is_err());
    }
}
