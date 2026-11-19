//! Thumbnails for the grid: photos, PDF first pages, video frames (on a film strip), album
//! covers, and whatever the desktop's thumbnailers make (office documents through
//! `gsf-office-thumbnailer`, for one).
//!
//! Thumbnails Nautilus and other apps already made (`~/.cache/thumbnails`) are used as they
//! are. New ones go to Files' own cache, keyed by path, size and date, and a file that can't be
//! thumbnailed is remembered so it isn't tried again. Workers take the newest request first, so
//! the tiles on screen fill before the ones scrolled past.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::model::{Entry, Location};
use crate::preview::{Kind, kind_of};
use crate::ssh::Target;

/// Pixels on a side.
const SIZE: u32 = 256;
/// Files on a server larger than this keep their icon rather than download for a thumbnail.
const REMOTE_LIMIT: u64 = 8_000_000;
/// Requests beyond this many waiting drop the oldest (rows long scrolled past).
const QUEUE: usize = 400;
const WORKERS: usize = 3;
/// A thumbnailer that takes longer is stopped.
const TIMEOUT: Duration = Duration::from_secs(20);

pub enum Src {
    Local(PathBuf),
    Remote(Target, String),
}

pub struct Request {
    /// The item, to match the result to its tile.
    pub item: Location,
    pub entry: Entry,
    pub mime: Option<String>,
    pub src: Src,
}

/// A finished request: the item and its thumbnail, or None when it has none.
pub type Done = (Location, Option<PathBuf>);

#[derive(Default)]
struct Shared {
    queue: Mutex<(VecDeque<Request>, usize)>,
    ready: Condvar,
    done: Mutex<Vec<Done>>,
    waker: Mutex<Option<rune::Waker>>,
}

#[derive(Clone, Default)]
pub struct Pool(Arc<Shared>);

impl Pool {
    pub fn set_waker(&self, w: rune::Waker) {
        if let Ok(mut g) = self.0.waker.lock() {
            *g = Some(w);
        }
    }

    pub fn request(&self, r: Request) {
        let Ok(mut q) = self.0.queue.lock() else { return };
        q.0.push_back(r);
        if q.0.len() > QUEUE {
            q.0.pop_front();
        }
        // Workers start with the first request: a session in list view runs none.
        if q.1 < WORKERS {
            q.1 += 1;
            let shared = self.0.clone();
            std::thread::spawn(move || work(shared));
        }
        self.0.ready.notify_one();
    }

    /// Drops what's waiting (another folder is showing); work under way still finishes.
    pub fn clear(&self) {
        if let Ok(mut q) = self.0.queue.lock() {
            q.0.clear();
        }
    }

    pub fn take(&self) -> Vec<Done> {
        self.0.done.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default()
    }
}

fn work(shared: Arc<Shared>) {
    let cache = crate::local::cache_dir();
    loop {
        let job = {
            let Ok(mut q) = shared.queue.lock() else { return };
            loop {
                if let Some(j) = q.0.pop_back() {
                    break j;
                }
                q = match shared.ready.wait(q) {
                    Ok(q) => q,
                    Err(_) => return,
                };
            }
        };
        let out = make(&job, &cache);
        if let Ok(mut d) = shared.done.lock() {
            d.push((job.item, out));
        }
        if let Some(w) = shared.waker.lock().ok().and_then(|w| w.clone()) {
            w();
        }
    }
}

/// Whether a file of this name and type can get a thumbnail (on a server: `remote`).
pub fn wants(e: &Entry, mime: Option<&str>, remote: bool) -> bool {
    if e.is_dir() || e.size == Some(0) {
        return false;
    }
    match kind_of(&e.name) {
        Kind::Image | Kind::Pdf => !remote || e.size.is_some_and(|s| s <= REMOTE_LIMIT),
        Kind::Video | Kind::Audio => !remote,
        _ => mime.is_some_and(|m| thumbnailer(m).is_some()) && (!remote || e.size.is_some_and(|s| s <= REMOTE_LIMIT)),
    }
}

fn make(r: &Request, cache: &Path) -> Option<PathBuf> {
    let ours = cache.join("thumbs");
    let (path, key) = match &r.src {
        Src::Local(p) => {
            let mtime = std::fs::metadata(p).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            if let Some(t) = shared_thumbnail(p, mtime) {
                return Some(t);
            }
            (p.clone(), crate::app::digest(&(p, r.entry.size, mtime, "grid")))
        }
        Src::Remote(t, path) => {
            let stamp = r.entry.modified.map(|m| m as i64);
            let key = crate::app::digest(&(t.id(), path, r.entry.size, stamp, "grid"));
            if let Some(found) = cached(&ours, &key) {
                return found;
            }
            let dest = crate::ops::open_cache_path(cache, t, path);
            if !crate::app::cache_fresh(&dest, &r.entry) {
                std::fs::create_dir_all(dest.parent()?).ok()?;
                let mut cmd = t.scp();
                cmd.arg("--").arg(t.remote_arg(path)).arg(&dest);
                if !run(&mut cmd) {
                    return None;
                }
            }
            (dest, key)
        }
    };
    if let Some(found) = cached(&ours, &key) {
        return found;
    }
    let ext = crate::model::extension(&r.entry.name).unwrap_or_default();
    // Rune draws SVG itself, at the size it's shown.
    if matches!(ext.as_str(), "svg" | "svgz") {
        return Some(path);
    }
    let _ = std::fs::create_dir_all(&ours);
    let out = ours.join(format!("grid-{key}.png"));
    let tmp = ours.join(format!("grid-{key}.{}.tmp.png", std::process::id()));
    let ok = generate(&path, &ext, r.mime.as_deref(), &tmp) && std::fs::metadata(&tmp).is_ok_and(|m| m.len() > 0);
    if ok && std::fs::rename(&tmp, &out).is_ok() {
        return Some(out);
    }
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::write(ours.join(format!("grid-{key}.fail")), b"");
    None
}

/// A thumbnail made before: Some(Some(path)), Some(None) for one that failed, None if never tried.
fn cached(dir: &Path, key: &str) -> Option<Option<PathBuf>> {
    let png = dir.join(format!("grid-{key}.png"));
    if png.exists() {
        return Some(Some(png));
    }
    dir.join(format!("grid-{key}.fail")).exists().then_some(None)
}

fn generate(path: &Path, ext: &str, mime: Option<&str>, out: &Path) -> bool {
    let size = SIZE.to_string();
    match kind_of(&path.to_string_lossy()) {
        Kind::Image => {
            let mut c = Command::new("magick");
            if matches!(ext, "jpg" | "jpeg") {
                // Decodes the JPEG at a fraction of its size: a 24 MP photo in a tenth of the time.
                c.args(["-define", &format!("jpeg:size={}x{}", SIZE * 2, SIZE * 2)]);
            }
            c.arg(format!("{}[0]", path.display())).args(["-auto-orient", "-thumbnail", &format!("{SIZE}x{SIZE}>")]).arg(format!("png:{}", out.display()));
            run(&mut c)
        }
        Kind::Pdf => {
            // pdftoppm adds ".png" to the name it's given.
            let base = out.to_string_lossy().trim_end_matches(".png").to_owned();
            run(Command::new("pdftoppm").args(["-png", "-singlefile", "-f", "1", "-l", "1", "-scale-to", &size]).arg(path).arg(&base))
        }
        // `-f`: sprocket holes down the sides, so a video reads as one at a glance.
        Kind::Video => run(Command::new("ffmpegthumbnailer").arg("-i").arg(path).arg("-o").arg(out).args(["-s", &size, "-t", "10%", "-f", "-c", "png"])),
        Kind::Audio => run(Command::new("ffmpegthumbnailer").arg("-i").arg(path).arg("-o").arg(out).args(["-s", &size, "-c", "png"])),
        _ => {
            let Some(exec) = mime.and_then(thumbnailer) else { return false };
            let uri = file_uri(path);
            let mut argv = Vec::new();
            for word in exec.split_whitespace() {
                argv.push(match word {
                    "%i" => path.to_string_lossy().into_owned(),
                    "%u" => uri.clone(),
                    "%o" => out.to_string_lossy().into_owned(),
                    "%s" => size.clone(),
                    w => w.replace("%%", "%"),
                });
            }
            let Some((prog, args)) = argv.split_first() else { return false };
            run(Command::new(prog).args(args))
        }
    }
}

/// Runs a thumbnailer, stopping it after `TIMEOUT`.
fn run(cmd: &mut Command) -> bool {
    let Ok(mut child) = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return false };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(_) => return false,
        }
    }
}

/// The desktop's thumbnailers (`/usr/share/thumbnailers`): MIME type → `Exec=` line.
fn thumbnailer(mime: &str) -> Option<&'static str> {
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = HashMap::new();
        let home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| crate::local::home().join(".local/share"));
        let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        for dir in std::iter::once(home).chain(dirs.split(':').map(PathBuf::from)) {
            let Ok(rd) = std::fs::read_dir(dir.join("thumbnailers")) else { continue };
            for f in rd.flatten() {
                let Ok(text) = std::fs::read_to_string(f.path()) else { continue };
                let get = |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().to_owned()));
                let (Some(exec), Some(types)) = (get("Exec="), get("MimeType=")) else { continue };
                let prog = get("TryExec=").or_else(|| exec.split_whitespace().next().map(str::to_owned)).unwrap_or_default();
                if !(Path::new(&prog).is_absolute() && Path::new(&prog).exists() || crate::local::has_program(&prog)) {
                    continue;
                }
                for m in types.split(';').filter(|m| !m.is_empty()) {
                    t.entry(m.to_owned()).or_insert_with(|| exec.clone());
                }
            }
        }
        t
    });
    table.get(mime).map(String::as_str)
}

/// `file://` and the path, escaped as GLib escapes it (the shared cache's names hash this).
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// A thumbnail another app made for `path` (modified at `mtime`) in the shared cache, if it's
/// still current.
fn shared_thumbnail(path: &Path, mtime: u64) -> Option<PathBuf> {
    use md5::Digest;
    let root = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| crate::local::home().join(".cache")).join("thumbnails");
    let hash = md5::Md5::digest(file_uri(path).as_bytes());
    let name: String = hash.iter().map(|b| format!("{b:02x}")).collect::<String>() + ".png";
    ["large", "x-large", "normal", "xx-large"].iter().map(|d| root.join(d).join(&name)).find(|t| png_mtime(t).is_some_and(|m| m == mtime))
}

/// A thumbnail's `Thumb::MTime`: the source file's modification time it was made from.
fn png_mtime(path: &Path) -> Option<u64> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    // Text chunks come before the pixels, in the first few kilobytes.
    f.by_ref().take(64 * 1024).read_to_end(&mut buf).ok()?;
    let mut at = 8;
    while at + 8 <= buf.len() {
        let len = u32::from_be_bytes(buf[at..at + 4].try_into().ok()?) as usize;
        let kind = &buf[at + 4..at + 8];
        let data = buf.get(at + 8..at + 8 + len)?;
        if kind == b"IDAT" {
            return None;
        }
        if kind == b"tEXt"
            && let Some(v) = data.strip_prefix(b"Thumb::MTime\0")
        {
            return std::str::from_utf8(v).ok()?.trim().parse().ok();
        }
        at += 12 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris() {
        assert_eq!(file_uri(Path::new("/home/a/Chapter # 6 QAS n S.pdf")), "file:///home/a/Chapter%20%23%206%20QAS%20n%20S.pdf");
        assert_eq!(file_uri(Path::new("/x/é(1).png")), "file:///x/%C3%A9(1).png");
    }

    #[test]
    fn reads_thumb_mtime() {
        // A PNG signature, a tEXt chunk, then IDAT.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let text = b"Thumb::MTime\x001700000000";
        png.extend((text.len() as u32).to_be_bytes());
        png.extend(b"tEXt");
        png.extend(text);
        png.extend([0; 4]);
        png.extend(0u32.to_be_bytes());
        png.extend(b"IDAT");
        png.extend([0; 4]);
        let p = std::env::temp_dir().join(format!("files-thumb-test-{}.png", std::process::id()));
        std::fs::write(&p, &png).unwrap();
        assert_eq!(png_mtime(&p), Some(1_700_000_000));
        let _ = std::fs::remove_file(&p);
    }
}
