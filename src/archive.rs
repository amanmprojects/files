//! Archives as read-only folders. `bsdtar` (libarchive) lists and extracts zip, tar (plain, gz,
//! bz2, xz, zst, lz4), 7z, rar, iso, cpio, deb and rpm. A listing is read once into a tree of
//! folders; members are extracted one at a time, when opened or copied out.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::jobs::Step;
use crate::model::{Entry, Kind};

const MULTI: [&str; 8] = [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tar.lz4", ".tar.lz", ".tar.lzma", ".tar.z"];
const SINGLE: [&str; 22] = [
    ".zip", ".jar", ".war", ".ear", ".apk", ".xpi", ".cbz", ".epub", ".tar", ".tgz", ".tbz", ".tbz2", ".txz", ".tzst", ".7z", ".rar", ".cbr", ".iso", ".cpio",
    ".deb", ".rpm", ".whl",
];

/// The archive extension `name` ends with, if any (`.tar.gz` before `.gz`).
fn archive_ext(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    MULTI.iter().chain(SINGLE.iter()).find(|e| lower.ends_with(*e) && lower.len() > e.len()).copied()
}

pub fn is_archive_name(name: &str) -> bool {
    archive_ext(name).is_some()
}

/// The folder an archive extracts into: `photos.tar.gz` → `photos`.
pub fn stem(name: &str) -> String {
    match archive_ext(name) {
        Some(e) => name[..name.len() - e.len()].to_owned(),
        None => name.to_owned(),
    }
}

/// Zip-family archives, which `unzip` extracts on hosts without bsdtar.
pub fn is_zip_family(name: &str) -> bool {
    matches!(archive_ext(name), Some(".zip" | ".jar" | ".war" | ".ear" | ".apk" | ".xpi" | ".cbz" | ".epub" | ".whl"))
}

/// An archive's listing: every folder's entries, by the folder's path inside (`""` is the top).
pub struct Archive {
    /// The archive file on this machine (a download, for one on a server).
    pub local: PathBuf,
    dirs: HashMap<String, Vec<Entry>>,
    /// The member name to extract for each path, as the archive stores it (`./a/b`, `a/b/`).
    raw: HashMap<String, String>,
}

impl Archive {
    /// Every file and folder inside, at any depth.
    pub fn count(&self) -> usize {
        self.dirs.values().map(Vec::len).sum()
    }

    pub fn list(&self, inner: &str) -> Result<Vec<Entry>, String> {
        self.dirs.get(inner.trim_matches('/')).cloned().ok_or_else(|| format!("No folder “{inner}” in this archive"))
    }

    /// The pattern `bsdtar -x` takes for a path inside: the stored name, or for a folder only
    /// implied by its contents, the path in the archive's own style.
    pub fn member(&self, inner: &str) -> String {
        let inner = inner.trim_matches('/');
        if let Some(r) = self.raw.get(inner) {
            return r.trim_end_matches('/').to_owned();
        }
        let dotted = self.raw.values().next().is_some_and(|r| r.starts_with("./"));
        if dotted { format!("./{inner}") } else { inner.to_owned() }
    }

    /// Where `member(inner)` lands under the folder it's extracted into.
    pub fn extracted(&self, dir: &Path, inner: &str) -> PathBuf {
        dir.join(self.member(inner).trim_start_matches("./"))
    }
}

/// Reads an archive's listing (`bsdtar -tv`).
pub fn index(local: &Path, utc_offset: i64, now: f64) -> Result<Archive, String> {
    let out = Command::new("bsdtar").env("LC_ALL", "C").arg("-tvf").arg(local).stdin(Stdio::null()).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { "Viewing archives needs bsdtar (pacman -S libarchive)".to_owned() } else { e.to_string() }
    })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unreadable");
        return Err(format!("Can't read this archive: {}", line.trim_start_matches("bsdtar: ")));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(build(local.to_path_buf(), parse(&text, utc_offset, now)))
}

/// One `bsdtar -tv` line: `drwxr-xr-x  0 user group  0 Nov 19 12:00 name`, with `-> target`
/// after a symlink's name. Dates within six months show the time, older ones the year.
pub struct Member {
    pub raw: String,
    pub kind: Kind,
    pub link: bool,
    pub size: Option<u64>,
    pub modified: Option<f64>,
    pub mode: Option<u32>,
}

pub fn parse(text: &str, utc_offset: i64, now: f64) -> Vec<Member> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line;
        let mut fields = Vec::with_capacity(8);
        for _ in 0..8 {
            rest = rest.trim_start_matches(' ');
            let end = rest.find(' ').unwrap_or(rest.len());
            fields.push(&rest[..end]);
            rest = &rest[end..];
        }
        let Some(name) = rest.strip_prefix(' ') else { continue };
        let perms = fields[0];
        let (kind, link, name) = match perms.chars().next() {
            Some('d') => (Kind::Dir, false, name),
            Some('l') => (Kind::Other, true, name.split(" -> ").next().unwrap_or(name)),
            Some('-') | Some('h') => (Kind::File, false, name.split(" link to ").next().unwrap_or(name)),
            _ => (Kind::Other, false, name),
        };
        if name.is_empty() || name == "." || name == "./" {
            continue;
        }
        out.push(Member {
            raw: name.to_owned(),
            kind,
            link,
            size: if kind == Kind::File { fields[4].parse().ok() } else { None },
            modified: parse_date(fields[5], fields[6], fields[7], utc_offset, now),
            mode: mode_bits(perms),
        });
    }
    out
}

fn mode_bits(perms: &str) -> Option<u32> {
    let p: Vec<char> = perms.chars().collect();
    if p.len() < 10 {
        return None;
    }
    let mut m = 0;
    for (i, c) in p[1..10].iter().enumerate() {
        if *c != '-' {
            m |= 1 << (8 - i);
        }
    }
    Some(m)
}

fn parse_date(mon: &str, day: &str, time_or_year: &str, utc_offset: i64, now: f64) -> Option<f64> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let m = MONTHS.iter().position(|x| *x == mon)? as u32 + 1;
    let d: u32 = day.parse().ok()?;
    let (year, secs) = match time_or_year.split_once(':') {
        Some((h, mi)) => {
            // Recent: this year, unless that would be in the future (then last year).
            let now_days = ((now as i64) + utc_offset).div_euclid(86_400);
            let this_year = crate::model::civil_year(now_days);
            let secs = h.parse::<i64>().ok()? * 3600 + mi.parse::<i64>().ok()? * 60;
            let y = if crate::model::days_from_civil(this_year, m, d) > now_days + 1 { this_year - 1 } else { this_year };
            (y, secs)
        }
        None => (time_or_year.parse().ok()?, 0),
    };
    Some((crate::model::days_from_civil(year, m, d) * 86_400 + secs - utc_offset) as f64)
}

fn build(local: PathBuf, members: Vec<Member>) -> Archive {
    let mut dirs: HashMap<String, Vec<Entry>> = HashMap::new();
    let mut raw = HashMap::new();
    dirs.insert(String::new(), Vec::new());
    for m in members {
        let path = m.raw.trim_start_matches("./").trim_matches('/').to_owned();
        if path.is_empty() || path.split('/').any(|c| c == "..") {
            continue;
        }
        // Folders above it, even ones the archive doesn't list.
        let parts: Vec<&str> = path.split('/').collect();
        for i in 1..parts.len() {
            let (parent, dir) = (parts[..i - 1].join("/"), parts[..i].join("/"));
            if !dirs.contains_key(&dir) {
                dirs.insert(dir.clone(), Vec::new());
                let name = parts[i - 1].to_owned();
                let list = dirs.entry(parent).or_default();
                if !list.iter().any(|e| e.name == name) {
                    list.push(Entry { name, kind: Kind::Dir, link: false, size: None, modified: None, mode: None });
                }
            }
        }
        let parent = parts[..parts.len() - 1].join("/");
        let name = parts[parts.len() - 1].to_owned();
        let entry = Entry { name: name.clone(), kind: m.kind, link: m.link, size: m.size, modified: m.modified, mode: m.mode };
        if m.kind == Kind::Dir {
            dirs.entry(path.clone()).or_default();
        }
        let list = dirs.entry(parent).or_default();
        match list.iter_mut().find(|e| e.name == name) {
            // An implied folder, listed later with its own details.
            Some(e) => *e = entry,
            None => list.push(entry),
        }
        raw.insert(path, m.raw);
    }
    Archive { local, dirs, raw }
}

/// `bsdtar -x` of some members into `dir` (made first). bsdtar refuses `..` and absolute paths.
pub fn extract(archive: &Path, members: &[String], dir: &Path) -> Vec<Step> {
    let d = dir.to_path_buf();
    let mkdir = Step::Func(Box::new(move || std::fs::create_dir_all(&d).map_err(|e| e.to_string())));
    let mut cmd = Command::new("bsdtar");
    cmd.arg("-xf").arg(archive).arg("-C").arg(dir);
    if !members.is_empty() {
        cmd.arg("--").args(members);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    vec![mkdir, Step::cmd(cmd)]
}

/// A script extracting an archive on a server into `dir`, with whichever tool it has.
pub fn remote_extract_script(archive: &str, dir: &str, name: &str) -> String {
    use crate::ssh::quote;
    let (a, d) = (quote(archive), quote(dir));
    let fallback = if is_zip_family(name) { format!("unzip -q {a} -d {d}") } else { format!("tar -xf {a} -C {d}") };
    format!("set -e\nmkdir -p -- {d}\nif command -v bsdtar >/dev/null 2>&1; then bsdtar -xf {a} -C {d}; else {fallback}; fi\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_stems() {
        assert!(is_archive_name("a.tar.gz") && is_archive_name("X.ZIP") && is_archive_name("b.7z"));
        assert!(!is_archive_name("notes.gz") && !is_archive_name(".zip") && !is_archive_name("photo.jpg"));
        assert_eq!(stem("photos.tar.gz"), "photos");
        assert_eq!(stem("a.b.zip"), "a.b");
    }

    #[test]
    fn parses_bsdtar_lines() {
        let now = 1_763_500_000.0; // 2025-11-18
        let text = "drwxr-xr-x  0 aman   aman        0 Nov 18 10:00 ./src/\n\
                    -rw-r--r--  0 aman   aman     1234 Nov 18 10:05 ./src/my file.rs\n\
                    lrwxrwxrwx  0 aman   aman        0 Mar  3  2024 ./link -> src\n\
                    -rw-r--r--  0 0      0          42 Dec 31 23:59 ./deep/er/x.txt\n";
        let m = parse(text, 0, now);
        assert_eq!(m.len(), 4);
        assert_eq!((m[1].raw.as_str(), m[1].size, m[1].mode), ("./src/my file.rs", Some(1234), Some(0o644)));
        assert_eq!(m[2].raw, "./link");
        assert!(m[2].link);
        // "Dec 31 23:59" with no year is in the future this year, so it's last year's.
        assert!(m[3].modified.unwrap() < now);
        let a = build("/x.tar".into(), m);
        let top: Vec<_> = a.list("").unwrap().into_iter().map(|e| (e.name, e.kind)).collect();
        assert_eq!(top, [("src".to_owned(), Kind::Dir), ("link".to_owned(), Kind::Other), ("deep".to_owned(), Kind::Dir)]);
        assert_eq!(a.list("deep/er").unwrap()[0].name, "x.txt");
        assert_eq!(a.member("deep"), "./deep");
        assert_eq!(a.member("src/my file.rs"), "./src/my file.rs");
        assert!(a.list("nope").is_err());
    }

    #[test]
    fn real_archives() {
        if Command::new("bsdtar").arg("--version").output().is_err() {
            return;
        }
        let d = std::env::temp_dir().join(format!("files-archive-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src/a dir")).unwrap();
        std::fs::write(d.join("src/a dir/it's.txt"), b"inside").unwrap();
        std::fs::write(d.join("src/top.md"), b"top").unwrap();
        for (name, flags) in [("t.tar.gz", "-czf"), ("t.zip", "--format=zip -cf")] {
            let arch = d.join(name);
            let mut c = Command::new("bsdtar");
            c.args(flags.split(' ')).arg(&arch).arg("-C").arg(d.join("src")).arg(".");
            assert!(c.status().unwrap().success());
            let a = index(&arch, 0, 1.9e9).unwrap();
            let mut top: Vec<_> = a.list("").unwrap().into_iter().map(|e| e.name).collect();
            top.sort();
            assert_eq!(top, ["a dir", "top.md"], "{name}");
            let file = a.list("a dir").unwrap();
            assert_eq!((file[0].name.as_str(), file[0].size), ("it's.txt", Some(6)));
            // Extract one folder, then check what landed.
            let out = d.join(format!("out-{name}"));
            crate::jobs::run_steps(extract(&arch, &[a.member("a dir")], &out), &std::sync::atomic::AtomicBool::new(false)).unwrap();
            assert_eq!(std::fs::read(a.extracted(&out, "a dir").join("it's.txt")).unwrap(), b"inside");
            assert!(!a.extracted(&out, "top.md").exists());
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
