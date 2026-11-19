//! The local filesystem: listing and well-known folders.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::model::{Entry, Kind};

pub fn list(dir: &Path) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("Can't open {}: {}", dir.display(), plain_io(&e)))?;
    let mut out = Vec::new();
    for item in rd.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        let Ok(lmeta) = item.path().symlink_metadata() else { continue };
        let link = lmeta.file_type().is_symlink();
        let meta = if link { std::fs::metadata(item.path()).ok() } else { Some(lmeta.clone()) };
        let kind = match &meta {
            Some(m) if m.is_dir() => Kind::Dir,
            Some(m) if m.is_file() => Kind::File,
            _ => Kind::Other,
        };
        let m = meta.as_ref().unwrap_or(&lmeta);
        out.push(Entry {
            name,
            kind,
            link,
            size: (kind == Kind::File).then_some(m.len()),
            modified: m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64()),
            mode: Some(m.permissions().mode() & 0o7777),
        });
    }
    Ok(out)
}

/// An io::Error without its "(os error N)" suffix.
pub fn plain_io(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such folder".into(),
        std::io::ErrorKind::PermissionDenied => "Permission denied".into(),
        _ => {
            let s = e.to_string();
            s.split(" (os error").next().unwrap_or(&s).to_owned()
        }
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// An XDG user folder (`DESKTOP`, `DOWNLOAD`, …) from `user-dirs.dirs`, else `~/<fallback>`.
pub fn user_dir(key: &str, fallback: &str) -> PathBuf {
    let home = home();
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = std::fs::read_to_string(config.join("user-dirs.dirs")) {
        let want = format!("XDG_{key}_DIR=");
        for line in text.lines() {
            if let Some(v) = line.trim().strip_prefix(&want) {
                let v = v.trim_matches('"');
                let path = match v.strip_prefix("$HOME") {
                    Some(rest) => home.join(rest.trim_start_matches('/')),
                    None => PathBuf::from(v),
                };
                return path;
            }
        }
    }
    home.join(fallback)
}

pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache")).join("files")
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config")).join("files")
}

/// `$PATH` has this program.
pub fn has_program(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// Seconds east of UTC for local time, from `date +%z` (std has no time zone support).
pub fn utc_offset() -> i64 {
    let out = std::process::Command::new("date").arg("+%z").output();
    let Ok(out) = out else { return 0 };
    let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if s.len() != 5 {
        return 0;
    }
    let sign = if s.starts_with('-') { -1 } else { 1 };
    let h: i64 = s[1..3].parse().unwrap_or(0);
    let m: i64 = s[3..5].parse().unwrap_or(0);
    sign * (h * 3600 + m * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_a_folder() {
        let dir = std::env::temp_dir().join(format!("files-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::os::unix::fs::symlink(dir.join("sub"), dir.join("to-sub")).unwrap();
        std::os::unix::fs::symlink(dir.join("gone"), dir.join("dead")).unwrap();
        let mut e = list(&dir).unwrap();
        e.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<_> = e.iter().map(|e| (e.name.as_str(), e.kind, e.link, e.size)).collect();
        assert_eq!(
            names,
            [("a.txt", Kind::File, false, Some(5)), ("dead", Kind::Other, true, None), ("sub", Kind::Dir, false, None), ("to-sub", Kind::Dir, true, None)]
        );
        assert!(list(&dir.join("missing")).unwrap_err().contains("No such folder"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
