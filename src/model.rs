//! What the explorer shows: a location (a local path or a path on an SSH host) and its entries.

use std::cmp::Ordering;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Dir,
    File,
    /// A symlink whose target is missing, or a socket, fifo or device.
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub link: bool,
    pub size: Option<u64>,
    /// Seconds since the Unix epoch.
    pub modified: Option<f64>,
    /// Permission bits (the low 12 bits of `st_mode`).
    pub mode: Option<u32>,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }
    pub fn hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

/// Where a listing comes from. Remote locations name a host by its id in the app's host table.
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum Location {
    Local(PathBuf),
    Remote {
        host: String,
        path: String,
    },
    /// A folder inside an archive file (`inner` is `""` at its top), read-only.
    Archive {
        file: Box<Location>,
        inner: String,
    },
}

impl Location {
    pub fn is_remote(&self) -> bool {
        match self {
            Location::Remote { .. } => true,
            Location::Local(_) => false,
            Location::Archive { file, .. } => file.is_remote(),
        }
    }

    pub fn is_archive(&self) -> bool {
        matches!(self, Location::Archive { .. })
    }

    pub fn host(&self) -> Option<&str> {
        match self {
            Location::Remote { host, .. } => Some(host),
            Location::Local(_) => None,
            Location::Archive { file, .. } => file.host(),
        }
    }

    /// The path as text; inside an archive, the archive's path then the path inside it.
    pub fn path_str(&self) -> String {
        match self {
            Location::Local(p) => p.to_string_lossy().into_owned(),
            Location::Remote { path, .. } => path.clone(),
            Location::Archive { file, inner } if inner.is_empty() => file.path_str(),
            Location::Archive { file, inner } => format!("{}/{inner}", file.path_str()),
        }
    }

    pub fn join(&self, name: &str) -> Location {
        match self {
            Location::Local(p) => Location::Local(p.join(name)),
            Location::Remote { host, path } => Location::Remote { host: host.clone(), path: posix_join(path, name) },
            Location::Archive { file, inner } => {
                Location::Archive { file: file.clone(), inner: if inner.is_empty() { name.to_owned() } else { format!("{inner}/{name}") } }
            }
        }
    }

    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Local(p) => p.parent().map(|p| Location::Local(p.to_path_buf())),
            Location::Remote { host, path } => posix_parent(path).map(|p| Location::Remote { host: host.clone(), path: p }),
            Location::Archive { file, inner } if inner.is_empty() => file.parent(),
            Location::Archive { file, inner } => {
                Some(Location::Archive { file: file.clone(), inner: inner.rsplit_once('/').map_or(String::new(), |(p, _)| p.to_owned()) })
            }
        }
    }

    /// The last component, or `/` for the root.
    pub fn name(&self) -> String {
        match self {
            Location::Local(p) => p.file_name().map_or_else(|| "/".to_owned(), |n| n.to_string_lossy().into_owned()),
            Location::Remote { path, .. } => match path.trim_end_matches('/').rsplit('/').next() {
                Some(n) if !n.is_empty() => n.to_owned(),
                _ => "/".to_owned(),
            },
            Location::Archive { file, inner } if inner.is_empty() => file.name(),
            Location::Archive { inner, .. } => inner.rsplit('/').next().unwrap_or(inner).to_owned(),
        }
    }

    /// Each ancestor from the root down to this location, with its label, for breadcrumbs.
    pub fn crumbs(&self) -> Vec<(String, Location)> {
        let mut out = Vec::new();
        let mut cur = Some(self.clone());
        while let Some(loc) = cur {
            cur = loc.parent();
            out.push((loc.name(), loc));
        }
        out.reverse();
        out
    }

    /// Same machine: both local, or both on the same host. Archives are never the same side:
    /// what leaves one is extracted.
    pub fn same_side(&self, other: &Location) -> bool {
        match (self, other) {
            (Location::Local(_), Location::Local(_)) => true,
            (Location::Remote { host: a, .. }, Location::Remote { host: b, .. }) => a == b,
            _ => false,
        }
    }
}

pub fn posix_join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

pub fn posix_parent(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.rfind('/') {
        Some(0) => Some("/".to_owned()),
        Some(i) => Some(trimmed[..i].to_owned()),
        None => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Size,
    Modified,
    Kind,
}

impl SortKey {
    pub fn from_column(id: &str) -> SortKey {
        match id {
            "size" => SortKey::Size,
            "modified" => SortKey::Modified,
            "kind" => SortKey::Kind,
            _ => SortKey::Name,
        }
    }
}

/// Folders first, then by the key; ties fall back to the name in natural order.
pub fn compare(a: &Entry, b: &Entry, key: SortKey, ascending: bool) -> Ordering {
    let dirs = b.is_dir().cmp(&a.is_dir());
    if dirs != Ordering::Equal {
        return dirs;
    }
    let by_name = || natural_cmp(&a.name, &b.name);
    let o = match key {
        SortKey::Name => by_name(),
        SortKey::Size => a.size.cmp(&b.size).then_with(by_name),
        SortKey::Modified => a.modified.partial_cmp(&b.modified).unwrap_or(Ordering::Equal).then_with(by_name),
        SortKey::Kind => kind_label(a).cmp(&kind_label(b)).then_with(by_name),
    };
    if ascending { o } else { o.reverse() }
}

/// Case-insensitive, with digit runs compared by value (`file2` before `file10`).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (mt, nt) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let o = mt.len().cmp(&nt.len()).then_with(|| mt.cmp(nt));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

pub fn extension(name: &str) -> Option<String> {
    let stem = name.trim_start_matches('.');
    stem.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).filter(|e| !e.is_empty())
}

pub fn kind_label(e: &Entry) -> String {
    match e.kind {
        Kind::Dir => "Folder".to_owned(),
        Kind::Other if e.link => "Broken link".to_owned(),
        Kind::Other => "Special".to_owned(),
        Kind::File => match extension(&e.name) {
            Some(ext) => format!("{} file", ext.to_ascii_uppercase()),
            None => "File".to_owned(),
        },
    }
}

pub fn human_size(b: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if b < 1000 {
        return format!("{b} B");
    }
    let mut v = b as f64 / 1000.0;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if v < 10.0 { format!("{v:.1} {}", UNITS[u]) } else { format!("{v:.0} {}", UNITS[u]) }
}

/// `rwxr-xr-x` from permission bits.
pub fn mode_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    s
}

/// A modification time relative to `now` (both Unix seconds): "Today 14:05", "Yesterday 09:12",
/// "3 Mar 2026 10:00".
pub fn format_time(t: f64, now: f64, utc_offset: i64) -> String {
    let local = |secs: f64| (secs as i64) + utc_offset;
    let (day, now_day) = (local(t).div_euclid(86_400), local(now).div_euclid(86_400));
    let sod = local(t).rem_euclid(86_400);
    let hm = format!("{:02}:{:02}", sod / 3600, (sod % 3600) / 60);
    match now_day - day {
        0 => format!("Today {hm}"),
        1 => format!("Yesterday {hm}"),
        _ => {
            let (y, m, d) = civil_from_days(day);
            const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            format!("{d} {} {y} {hm}", MONTHS[(m - 1) as usize])
        }
    }
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's days_from_civil).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The year a day number falls in.
pub fn civil_year(days: i64) -> i64 {
    civil_from_days(days).0
}

/// Howard Hinnant's days-to-civil.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A name for a copy that doesn't collide with `taken`: `a.txt` → `a (copy).txt`, `a (copy 2).txt`, …
pub fn unique_copy_name(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1.. {
        let candidate = if n == 1 { format!("{stem} (copy){ext}") } else { format!("{stem} (copy {n}){ext}") };
        if !taken(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// A file or folder name the user typed is usable: not empty, no slash, not `.` or `..`.
pub fn valid_name(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err("Name can't be empty");
    }
    if name.contains('/') || name.contains('\0') {
        return Err("Name can't contain “/”");
    }
    if name == "." || name == ".." {
        return Err("That name is reserved");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_parents_and_names() {
        assert_eq!(posix_parent("/home/a/b"), Some("/home/a".into()));
        assert_eq!(posix_parent("/home"), Some("/".into()));
        assert_eq!(posix_parent("/"), None);
        assert_eq!(posix_join("/", "etc"), "/etc");
        assert_eq!(posix_join("/etc", "x"), "/etc/x");
        let loc = Location::Remote { host: "h".into(), path: "/srv/www".into() };
        assert_eq!(loc.name(), "www");
        let crumbs: Vec<_> = loc.crumbs().into_iter().map(|c| c.0).collect();
        assert_eq!(crumbs, ["/", "srv", "www"]);
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "File2", "file1", "a"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a", "file1", "File2", "file10"]);
    }

    #[test]
    fn copy_names() {
        let taken = |n: &str| n == "a (copy).txt";
        assert_eq!(unique_copy_name("a.txt", &taken), "a (copy 2).txt");
        assert_eq!(unique_copy_name(".bashrc", &|_| false), ".bashrc (copy)");
        assert_eq!(unique_copy_name("dir", &|_| false), "dir (copy)");
    }

    #[test]
    fn sizes_and_modes() {
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1_500), "1.5 KB");
        assert_eq!(human_size(25_000_000), "25 MB");
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
    }

    #[test]
    fn times() {
        let now = 1_763_500_000.0; // 2025-11-18 21:06:40 UTC
        assert_eq!(format_time(now - 60.0, now, 0), "Today 21:05");
        assert_eq!(format_time(now - 86_400.0, now, 0), "Yesterday 21:06");
        assert_eq!(format_time(0.0, now, 0), "1 Jan 1970 00:00");
        assert_eq!(days_from_civil(2025, 11, 18), 20_410);
        assert_eq!(civil_year(20_410), 2025);
        let a = Location::Archive { file: Box::new(Location::Local("/d/x.zip".into())), inner: "a/b".into() };
        let names: Vec<_> = a.crumbs().into_iter().map(|c| c.0).collect();
        assert_eq!(names, ["/", "d", "x.zip", "a", "b"]);
    }
}
