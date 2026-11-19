//! File types and their icons from the desktop's icon theme, as Nautilus shows them: a type
//! from the name (shared-mime-info's globs), then the theme's picture for that type (a Word
//! page, a spreadsheet, a music note) or for a special folder (Downloads, Pictures).
//!
//! Both tables load on a worker at start and again when the theme changes; until then the
//! grid draws its own line icons.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::model::Entry;

pub struct Lookup {
    /// Lower-case suffix after a dot ("pdf", "tar.gz") → (weight, MIME type).
    exts: HashMap<String, (u32, String)>,
    /// Whole lower-case names ("makefile") → MIME type.
    names: HashMap<String, String>,
    /// A MIME type's own icon name, when it isn't the type with `/` as `-`.
    icon_names: HashMap<String, String>,
    /// A MIME type's generic icon ("x-office-document").
    generic: HashMap<String, String>,
    /// Icon name → the theme's best file for it.
    icons: HashMap<String, PathBuf>,
    /// Special folders → their icon name.
    places: Vec<(PathBuf, &'static str)>,
}

static LOOKUP: RwLock<Option<Arc<Lookup>>> = RwLock::new(None);

/// The tables, once loaded.
pub fn get() -> Option<Arc<Lookup>> {
    LOOKUP.read().ok()?.clone()
}

/// Loads (or reloads, for a new icon theme) the tables on a worker, then wakes the window.
pub fn load(wake: rune::Waker) {
    std::thread::spawn(move || {
        let l = Arc::new(Lookup::build());
        if let Ok(mut g) = LOOKUP.write() {
            *g = Some(l);
        }
        wake();
    });
}

impl Lookup {
    fn build() -> Lookup {
        let mut l = Lookup {
            exts: HashMap::new(),
            names: HashMap::new(),
            icon_names: HashMap::new(),
            generic: HashMap::new(),
            icons: HashMap::new(),
            places: Vec::new(),
        };
        for dir in data_dirs().iter().rev() {
            let mime = dir.join("mime");
            if let Ok(t) = std::fs::read_to_string(mime.join("globs2")) {
                l.add_globs(&t);
            }
            for (file, map) in [("icons", &mut l.icon_names), ("generic-icons", &mut l.generic)] {
                if let Ok(t) = std::fs::read_to_string(mime.join(file)) {
                    for (m, i) in t.lines().filter_map(|line| line.split_once(':')) {
                        map.insert(m.to_owned(), i.to_owned());
                    }
                }
            }
        }
        l.icons = theme_icons(&icon_theme());
        let home = crate::local::home();
        l.places = vec![
            (crate::local::user_dir("DESKTOP", "Desktop"), "user-desktop"),
            (crate::local::user_dir("DOCUMENTS", "Documents"), "folder-documents"),
            (crate::local::user_dir("DOWNLOAD", "Downloads"), "folder-download"),
            (crate::local::user_dir("MUSIC", "Music"), "folder-music"),
            (crate::local::user_dir("PICTURES", "Pictures"), "folder-pictures"),
            (crate::local::user_dir("VIDEOS", "Videos"), "folder-videos"),
            (crate::local::user_dir("TEMPLATES", "Templates"), "folder-templates"),
            (crate::local::user_dir("PUBLICSHARE", "Public"), "folder-publicshare"),
        ];
        // An unset XDG folder falls back to home itself, which keeps its own icon.
        l.places.retain(|(p, _)| *p != home);
        l.places.push((home, "user-home"));
        l
    }

    /// `weight:type:glob[:flags]` lines. Only `*.suffix` and plain names are kept; the
    /// handful of other patterns (`README*`) aren't worth a matcher.
    fn add_globs(&mut self, text: &str) {
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut it = line.splitn(4, ':');
            let (Some(w), Some(mime), Some(glob)) = (it.next(), it.next(), it.next()) else { continue };
            let w: u32 = w.parse().unwrap_or(50);
            let glob = glob.to_lowercase();
            if let Some(ext) = glob.strip_prefix("*.") {
                if ext.contains(['*', '?', '[']) {
                    continue;
                }
                let slot = self.exts.entry(ext.to_owned()).or_insert((0, String::new()));
                if w > slot.0 {
                    *slot = (w, mime.to_owned());
                }
            } else if !glob.contains(['*', '?', '[']) {
                self.names.entry(glob).or_insert_with(|| mime.to_owned());
            }
        }
    }

    /// The MIME type a name says, if any.
    pub fn mime_of(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        if let Some(m) = self.names.get(&lower) {
            return Some(m);
        }
        // Every suffix after a dot, longest first; the heaviest wins, the longer on a tie.
        let mut best: Option<&(u32, String)> = None;
        for (i, _) in lower.match_indices('.') {
            if let Some(hit) = self.exts.get(&lower[i + 1..])
                && best.is_none_or(|b| hit.0 > b.0)
            {
                best = Some(hit);
            }
        }
        best.map(|b| b.1.as_str())
    }

    /// The icon theme's picture for `e`, which is in folder `dir` on this machine (None on a
    /// server or in an archive: no special folders there).
    pub fn icon(&self, e: &Entry, dir: Option<&Path>) -> Option<&Path> {
        if e.is_dir() {
            let special = dir.map(|d| d.join(&e.name)).and_then(|p| self.places.iter().find(|(q, _)| *q == p).map(|(_, i)| *i));
            return special.into_iter().chain(["folder", "inode-directory"]).find_map(|n| self.icons.get(n)).map(PathBuf::as_path);
        }
        let mime = self.mime_of(&e.name);
        let mut names: Vec<String> = Vec::new();
        if let Some(m) = mime {
            if let Some(i) = self.icon_names.get(m) {
                names.push(i.clone());
            }
            names.push(m.replace('/', "-"));
            if let Some(g) = self.generic.get(m) {
                names.push(g.clone());
            }
            if let Some((media, _)) = m.split_once('/') {
                names.push(format!("{media}-x-generic"));
            }
        } else if e.mode.is_some_and(|m| m & 0o111 != 0) && !e.name.contains('.') {
            names.push("application-x-executable".into());
        }
        names.extend(["text-x-generic".into(), "application-x-generic".into(), "unknown".into()]);
        names.iter().find_map(|n| self.icons.get(n)).map(PathBuf::as_path)
    }
}

/// `$XDG_DATA_HOME` first, then `$XDG_DATA_DIRS`.
fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| crate::local::home().join(".local/share"));
    let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    std::iter::once(home).chain(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from)).collect()
}

/// The desktop's icon theme: GNOME's setting (Omarchy sets it with each theme), else GTK's.
fn icon_theme() -> String {
    let gsettings = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_owned())
        .filter(|t| !t.is_empty());
    gsettings
        .or_else(|| {
            let ini = std::fs::read_to_string(crate::local::config_dir().join("gtk-3.0/settings.ini")).ok()?;
            ini.lines().find_map(|l| l.trim().strip_prefix("gtk-icon-theme-name").map(|v| v.trim_start_matches([' ', '=']).trim().to_owned()))
        })
        .unwrap_or_else(|| "Adwaita".into())
}

fn icon_bases() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = data_dirs().into_iter().map(|d| d.join("icons")).collect();
    v.insert(1, crate::local::home().join(".icons"));
    v
}

/// How well a theme folder's pictures suit a 64–200 point tile: 128–256 pixel bitmaps first,
/// then scalable ones, then bigger bitmaps (slower to decode), then small ones.
fn fitness(size: u32, scale: u32, scalable: bool) -> u32 {
    let px = size * scale.max(1);
    match () {
        _ if (128..=256).contains(&px) => 2000 + px,
        _ if scalable => 1500,
        _ if px > 256 => 1000 + 1000 * 256 / px,
        _ => px,
    }
}

/// Every type and place icon in `theme` and what it inherits (then hicolor), by name: the
/// first theme in that order that has a name wins, with its best-fitting size.
fn theme_icons(theme: &str) -> HashMap<String, PathBuf> {
    let bases = icon_bases();
    let mut order: Vec<String> = Vec::new();
    let mut queue = vec![theme.to_owned()];
    while let Some(t) = queue.pop() {
        if order.contains(&t) {
            continue;
        }
        let inherits = bases
            .iter()
            .find_map(|b| std::fs::read_to_string(b.join(&t).join("index.theme")).ok())
            .and_then(|txt| txt.lines().find_map(|l| l.strip_prefix("Inherits=").map(str::to_owned)))
            .unwrap_or_default();
        order.push(t);
        // Depth first, in the listed order.
        for i in inherits.split(',').map(str::trim).filter(|i| !i.is_empty()).rev() {
            queue.push(i.to_owned());
        }
    }
    for fallback in ["Adwaita", "AdwaitaLegacy", "hicolor"] {
        if !order.iter().any(|t| t == fallback) {
            order.push(fallback.into());
        }
    }
    let mut out: HashMap<String, PathBuf> = HashMap::new();
    for t in &order {
        let mut own: HashMap<String, (u32, PathBuf)> = HashMap::new();
        for base in &bases {
            let root = base.join(t);
            let Ok(index) = std::fs::read_to_string(root.join("index.theme")) else { continue };
            for (dir, fit) in theme_dirs(&index) {
                let Ok(rd) = std::fs::read_dir(root.join(&dir)) else { continue };
                for f in rd.flatten() {
                    let name = f.file_name().to_string_lossy().into_owned();
                    let Some(stem) = name.strip_suffix(".svg").or_else(|| name.strip_suffix(".png")) else { continue };
                    if stem.ends_with("-symbolic") {
                        continue;
                    }
                    let slot = own.entry(stem.to_owned()).or_insert((0, PathBuf::new()));
                    if fit > slot.0 {
                        *slot = (fit, f.path());
                    }
                }
            }
        }
        for (name, (_, path)) in own {
            out.entry(name).or_insert(path);
        }
    }
    out
}

/// A theme's type and place folders and how well each fits, from its `index.theme`.
fn theme_dirs(index: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let (mut size, mut scale, mut scalable, mut wanted) = (0, 1, false, false);
    let mut flush = |cur: &Option<String>, size: u32, scale: u32, scalable: bool, wanted: bool| {
        if let Some(d) = cur
            && wanted
        {
            out.push((d.clone(), fitness(size, scale, scalable)));
        }
    };
    for line in index.lines().map(str::trim) {
        if let Some(sec) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            flush(&cur, size, scale, scalable, wanted);
            cur = (sec != "Icon Theme").then(|| sec.to_owned());
            (size, scale, scalable, wanted) = (0, 1, false, false);
        } else if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "Size" => size = v.trim().parse().unwrap_or(0),
                "Scale" => scale = v.trim().parse().unwrap_or(1),
                "Type" => scalable = v.trim() == "Scalable",
                "Context" => wanted = matches!(v.trim(), "MimeTypes" | "Places"),
                _ => {}
            }
        }
    }
    flush(&cur, size, scale, scalable, wanted);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, dir: bool) -> Entry {
        Entry {
            name: name.into(),
            kind: if dir { crate::model::Kind::Dir } else { crate::model::Kind::File },
            link: false,
            size: None,
            modified: None,
            mode: None,
        }
    }

    #[test]
    fn globs() {
        let mut l = Lookup {
            exts: HashMap::new(),
            names: HashMap::new(),
            icon_names: HashMap::new(),
            generic: HashMap::new(),
            icons: HashMap::new(),
            places: Vec::new(),
        };
        l.add_globs("50:application/pdf:*.pdf\n50:application/gzip:*.gz\n50:application/x-compressed-tar:*.tar.gz\n10:application/x-genesis-rom:*.md\n50:text/markdown:*.md\n50:text/x-makefile:makefile\n50:text/x-readme:README*\n");
        assert_eq!(l.mime_of("Paper.PDF"), Some("application/pdf"));
        assert_eq!(l.mime_of("x.tar.gz"), Some("application/x-compressed-tar"));
        assert_eq!(l.mime_of("a.gz"), Some("application/gzip"));
        assert_eq!(l.mime_of("notes.md"), Some("text/markdown"));
        assert_eq!(l.mime_of("Makefile"), Some("text/x-makefile"));
        assert_eq!(l.mime_of("README"), None);

        l.generic.insert("application/pdf".into(), "x-office-document".into());
        l.icons.insert("x-office-document".into(), "/i/doc.png".into());
        l.icons.insert("folder".into(), "/i/folder.png".into());
        l.icons.insert("folder-download".into(), "/i/dl.png".into());
        l.places.push(("/home/u/Downloads".into(), "folder-download"));
        assert_eq!(l.icon(&entry("a.pdf", false), None), Some(Path::new("/i/doc.png")));
        assert_eq!(l.icon(&entry("Downloads", true), Some(Path::new("/home/u"))), Some(Path::new("/i/dl.png")));
        assert_eq!(l.icon(&entry("Downloads", true), None), Some(Path::new("/i/folder.png")));
    }

    #[test]
    fn index_dirs() {
        let idx = "[Icon Theme]\nName=T\nDirectories=48x48/places,256x256/mimetypes,scalable/apps\n\n[48x48/places]\nContext=Places\nSize=48\nType=Fixed\n\n[256x256/mimetypes]\nContext=MimeTypes\nSize=256\n\n[scalable/apps]\nContext=Applications\nSize=64\nType=Scalable\n";
        let d = theme_dirs(idx);
        assert_eq!(d, vec![("48x48/places".into(), 48), ("256x256/mimetypes".into(), 2256)]);
    }
}
