//! The explorer's state and everything a handler can do to it.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rune::{Latest, PointerPress, TextEdit, Tx, Waker, WheelEvent};
use rune_kit::SortDir;
use rune_kit::TableState;
use rune_kit::shell::PaneState;

use crate::archive::{self, Archive};
use crate::jobs::{After, Direction, Job, JobState, Jobs};
use crate::local;
use crate::model::{self, Entry, Location, SortKey};
use crate::ops::{self, End, Mode};
use crate::preview::{self, Preview};
use crate::ssh::{self, SshError, Target};

/// A server the sidebar lists: from `~/.ssh/config`, or connected to by address.
pub struct Host {
    pub id: String,
    pub target: Target,
    /// The remote home folder, once connected.
    pub home: Option<String>,
    /// The folder last shown on it, to return to.
    pub last: Option<String>,
    pub from_config: bool,
}

pub struct Clip {
    pub mode: Mode,
    pub dir: Location,
    pub names: Vec<String>,
}

#[allow(clippy::large_enum_variant)]
pub enum Dialog {
    Connect {
        server: TextEdit,
        password: TextEdit,
        key: TextEdit,
        error: Option<String>,
    },
    /// First contact with a host whose key isn't in known_hosts.
    Trust {
        target: Target,
        path: Option<String>,
        fingerprint: Option<String>,
    },
    /// A new folder, or with `file` a new empty file.
    NewFolder {
        name: TextEdit,
        error: Option<String>,
        file: bool,
    },
    Properties {
        names: Vec<String>,
    },
    Shortcuts,
    /// A large preview of the selection (Space); ↑ ↓ step through files while it's open.
    QuickLook,
    Rename {
        from: String,
        name: TextEdit,
        error: Option<String>,
    },
    Delete {
        names: Vec<String>,
        trash: bool,
    },
    Conflict {
        plan: Paste,
        conflicts: Vec<String>,
    },
}

/// A paste waiting on the user's answer about names that already exist.
#[derive(Clone)]
pub struct Paste {
    pub mode: Mode,
    pub from: Location,
    pub names: Vec<String>,
    pub to: Location,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Resolve {
    Replace,
    KeepBoth,
    Skip,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuFor {
    Items,
    Background,
    /// A server in the sidebar (`App::menu_host`).
    Host,
}

pub struct Notice {
    pub text: String,
    pub error: bool,
}

/// A listing as it lands: where, what, and for an archive opened for the first time its index.
type Listed = (Location, Result<Vec<Entry>, String>, Option<(Location, Arc<Archive>)>);

/// A trackpad's two-finger sideways swipe, browser style: past `SWIPE_DISTANCE` points to the
/// right goes back, to the left forward.
#[derive(Default)]
pub struct Swipe {
    /// Points travelled sideways in this gesture (positive: fingers moving right).
    pub dx: f32,
    /// Decided on the gesture's first delta: sideways (ours) or not (the list scrolls).
    sideways: Option<bool>,
    /// When the last delta came, on Rune's clock.
    last: Option<f64>,
}

impl Swipe {
    pub fn active(&self) -> bool {
        self.sideways == Some(true)
    }
}

pub const SWIPE_DISTANCE: f32 = 140.0;

/// What the right-hand pane shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Preview,
    Transfers,
}
/// A gesture with no delta for this long, in seconds, has ended (for platforms that don't say).
const SWIPE_IDLE: f64 = 0.22;
type Connected = (Target, Option<String>, Result<String, SshError>);

/// A sidebar place: its label, icon and folder.
pub type Place = (&'static str, &'static rune::Icon, PathBuf);

/// A tab while another one shows: its folder, history, listing, selection and fields. The
/// showing tab's live in `App` itself, so everything else works on "the current tab".
pub struct Tab {
    /// Stable across reordering, for the tab strip.
    pub id: String,
    pub loc: Location,
    back: Vec<Location>,
    forward: Vec<Location>,
    entries: Vec<Entry>,
    shown: Vec<usize>,
    table: TableState,
    error: Option<String>,
    filter: TextEdit,
}

impl Tab {
    fn new(loc: Location) -> Tab {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = format!("tab{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        Tab {
            id,
            loc,
            back: Vec::new(),
            forward: Vec::new(),
            entries: Vec::new(),
            shown: Vec::new(),
            table: new_table(),
            error: None,
            filter: TextEdit::new(""),
        }
    }
}

fn new_table() -> TableState {
    let mut t = TableState::new().key("files").multi_select(true);
    t.set_sort("name", SortDir::Ascending);
    t
}

pub struct App {
    pub loc: Location,
    /// Every tab; the showing one (`active`) is a placeholder, its state being `App`'s own.
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// Closed tabs' folders, for Ctrl+Shift+T.
    closed: Vec<Location>,
    pub tab_drag: rune_kit::shell::TabDragState,
    /// Read once at start, so drawing the sidebar touches no files.
    pub places: Vec<Place>,
    pub back: Vec<Location>,
    pub forward: Vec<Location>,
    /// The folder's entries as listed; `shown` indexes the visible ones in display order.
    pub entries: Vec<Entry>,
    pub shown: Vec<usize>,
    pub table: TableState,
    pub sort: (SortKey, bool),
    pub loading: bool,
    pub error: Option<String>,
    pub show_hidden: bool,
    pub filter: TextEdit,
    pub path_edit: Option<TextEdit>,
    /// Completions for the path field, and the highlighted one.
    pub completions: Vec<String>,
    pub completion_pick: Option<usize>,
    complete_req: Latest<(String, Vec<String>)>,
    pub hosts: Vec<Host>,
    pub connecting: Option<String>,
    pub clip: Option<Clip>,
    pub jobs: Jobs,
    pub menu: Option<(PointerPress, MenuFor)>,
    /// The server a `MenuFor::Host` menu is for.
    pub menu_host: String,
    /// Servers pinned to the top of the sidebar, in pin order. Saved between launches.
    pub pinned: Vec<String>,
    /// The unpinned servers' section is open.
    pub more_servers_open: bool,
    /// Dragging a pinned server to reorder the pins.
    pub pin_reorder: rune_kit::shell::Reorder,
    pub dialog: Option<Dialog>,
    pub notice: Option<Notice>,
    /// UI zoom, as a browser's page zoom: 1 is Rune's native size. Saved between launches.
    pub zoom: f32,
    pub sidebar: PaneState,
    pub activity: PaneState,
    /// Move keyboard focus to the file table once the next listing lands (after navigating).
    want_focus: bool,
    /// An element to focus on the next frame: the table after navigating, Quick Look's body.
    pub focus_req: Option<&'static str>,
    /// Letters typed to jump to a name, and when the last one came.
    typed: (String, Option<f64>),
    /// Where the file area is, for menus opened from the keyboard.
    pub content_at: [f32; 4],
    pub now: f64,
    pub utc_offset: i64,
    pub swipe: Swipe,
    /// The bottom terminal panel.
    pub terminal: Option<crate::term::Terminal>,
    pub term_open: bool,
    pub term_h: f32,
    term_h_start: f32,
    /// One character cell of the terminal's font, measured by the view.
    pub term_cell: [f32; 2],
    /// The terminal's drawing area, to fit again when the cell size is measured.
    term_area: [f32; 2],
    /// The server the terminal is logged in to, if it was started on one.
    term_host: Option<String>,
    /// Omarchy's theme, when there is one; otherwise the kit's neutral look.
    pub omarchy: Option<crate::theme::Theme>,
    theme_changed: Arc<std::sync::atomic::AtomicBool>,
    waker: Option<Waker>,
    /// The window title last set (the current folder's name).
    pub title: String,
    /// How far the file list is scrolled, worked out from where its rows are drawn.
    pub scroll_y: f32,
    /// Each tab's folders' scroll when left, to put back on Back, Forward, Up or a tab switch.
    scroll_memory: HashMap<(String, Location), f32>,
    /// The folder being entered should get its remembered scroll back.
    restore_scroll: bool,
    /// A scroll for the list to jump to on the next frame.
    pub scroll_req: Option<f32>,
    pub side: Side,
    /// The selection's preview, once made, and its picture.
    pub preview: Option<Preview>,
    pub preview_image: Option<rune::Image>,
    /// The picture finished decoding (or failed) and the view has been told.
    preview_image_seen: bool,
    /// What the last preview request was for (None: nothing selected, or no preview shown).
    preview_for: Option<Location>,
    preview_req: Latest<Preview>,
    images: Option<rune::ImageCache>,
    /// Recent folders' entries, shown at once when revisited while a fresh listing loads.
    listings: HashMap<Location, Vec<Entry>>,
    listing_order: VecDeque<Location>,
    /// Indexes of opened archives, by the archive file's location.
    archives: HashMap<Location, Arc<Archive>>,
    listing: Latest<Listed>,
    connect_req: Latest<Connected>,
    fingerprint_req: Latest<(String, Option<String>)>,
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

impl App {
    pub fn new(start: Option<String>) -> App {
        let home = local::home();
        let mut hosts: Vec<Host> = ssh::config_hosts(std::env::var("FILES_SSH_CONFIG").ok().filter(|f| !f.is_empty()).map(PathBuf::from))
            .into_iter()
            .map(|h| Host { id: h.clone(), target: Target { dest: h, ..Target::default() }, home: None, last: None, from_config: true })
            .collect();
        for id in load_recent() {
            if hosts.iter().all(|h| h.id != id)
                && let Ok((t, _)) = Target::parse(&id)
            {
                hosts.push(Host { id, target: t, home: None, last: None, from_config: false });
            }
        }
        let table = new_table();
        let mut app = App {
            loc: Location::Local(home.clone()),
            tabs: vec![Tab::new(Location::Local(home.clone()))],
            active: 0,
            closed: Vec::new(),
            tab_drag: rune_kit::shell::TabDragState::new(),
            places: places(),
            back: Vec::new(),
            forward: Vec::new(),
            entries: Vec::new(),
            shown: Vec::new(),
            table,
            sort: (SortKey::Name, true),
            loading: false,
            error: None,
            show_hidden: false,
            filter: TextEdit::new(""),
            path_edit: None,
            completions: Vec::new(),
            completion_pick: None,
            complete_req: Latest::default().debounce(std::time::Duration::from_millis(40)),
            hosts,
            connecting: None,
            clip: None,
            jobs: Jobs::default(),
            menu: None,
            dialog: None,
            menu_host: String::new(),
            pinned: load_pinned(),
            more_servers_open: false,
            pin_reorder: rune_kit::shell::Reorder::new(),
            notice: None,
            zoom: load_zoom(),
            want_focus: true,
            focus_req: None,
            typed: (String::new(), None),
            content_at: [0.0; 4],
            sidebar: PaneState::new(220.0).range(160.0, 360.0),
            activity: PaneState::new(300.0).range(240.0, 480.0).collapsed(),
            now: now(),
            utc_offset: local::utc_offset(),
            swipe: Swipe::default(),
            terminal: None,
            term_open: false,
            term_h: 260.0,
            term_h_start: 260.0,
            term_cell: [8.0, 17.0],
            term_area: [0.0, 0.0],
            term_host: None,
            omarchy: crate::theme::load(),
            theme_changed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            waker: None,
            title: String::new(),
            scroll_y: 0.0,
            scroll_memory: HashMap::new(),
            restore_scroll: false,
            scroll_req: None,
            side: Side::Preview,
            preview: None,
            preview_image: None,
            preview_image_seen: false,
            preview_for: None,
            preview_req: Latest::default().debounce(std::time::Duration::from_millis(90)),
            images: None,
            listings: HashMap::new(),
            listing_order: VecDeque::new(),
            archives: HashMap::new(),
            listing: Latest::default(),
            connect_req: Latest::default(),
            fingerprint_req: Latest::default(),
        };
        match start.as_deref() {
            Some(s) if s.contains("://") || (!s.starts_with('/') && !s.starts_with('.') && s.contains(':')) => match Target::parse(s) {
                Ok((t, path)) => app.connect(t, path),
                Err(e) => app.fail(e),
            },
            Some(p) => {
                let p = std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
                if p.is_file() && archive::is_archive_name(&p.to_string_lossy()) {
                    app.loc = Location::Archive { file: Box::new(Location::Local(p)), inner: String::new() };
                    app.reload();
                } else {
                    app.loc = Location::Local(p);
                    app.list_now();
                }
            }
            None => app.list_now(),
        }
        std::thread::spawn(|| prune_cache(&local::cache_dir(), CACHE_LIMIT));
        app
    }

    pub fn set_waker(&mut self, w: Waker) {
        self.waker = Some(w.clone());
        crate::theme::watch(w.clone(), self.theme_changed.clone());
        self.images = Some(rune::ImageCache::new(w.clone()));
        self.preview_req.set_waker(w.clone());
        self.complete_req.set_waker(w.clone());
        self.listing.set_waker(w.clone());
        self.connect_req.set_waker(w.clone());
        self.fingerprint_req.set_waker(w.clone());
        self.jobs.set_waker(w);
    }

    /// Lists a local folder on this thread: only before the first frame, so the window opens full.
    fn list_now(&mut self) {
        if let Location::Local(p) = &self.loc {
            let r = local::list(p);
            self.apply_listing(self.loc.clone(), r, None);
        }
    }

    // ---- Navigation ----------------------------------------------------------------------------

    /// Opens a folder fresh: at the top, with history.
    pub fn go(&mut self, loc: Location) {
        self.navigate(loc, false);
    }

    fn navigate(&mut self, loc: Location, restore: bool) {
        if loc == self.loc {
            return self.reload();
        }
        self.remember_scroll();
        self.back.push(std::mem::replace(&mut self.loc, loc));
        self.forward.clear();
        self.restore_scroll = restore;
        self.entered();
    }

    pub fn go_back(&mut self) {
        if let Some(loc) = self.back.pop() {
            self.remember_scroll();
            self.forward.push(std::mem::replace(&mut self.loc, loc));
            self.restore_scroll = true;
            self.entered();
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(loc) = self.forward.pop() {
            self.remember_scroll();
            self.back.push(std::mem::replace(&mut self.loc, loc));
            self.restore_scroll = true;
            self.entered();
        }
    }

    /// The enclosing folder, scrolled as it was left and with the folder we came from selected.
    pub fn go_up(&mut self) {
        if let Some(p) = self.loc.parent() {
            let child = self.loc.name();
            self.navigate(p, true);
            self.table.select(child);
        }
    }

    // ---- Scroll memory -------------------------------------------------------------------------

    fn remember_scroll(&mut self) {
        if self.scroll_memory.len() > 500 {
            self.scroll_memory.clear();
        }
        let key = (self.tabs[self.active].id.clone(), self.loc.clone());
        self.scroll_memory.insert(key, self.scroll_y);
    }

    /// A row of the list was drawn at window `y` (row `i`; rows are `row_h` tall under a
    /// `header_h` header at the top of the file area): how far the list is scrolled.
    pub fn track_scroll(&mut self, i: usize, y: f32, header_h: f32, row_h: f32) {
        let top = self.content_at[1] + header_h;
        self.scroll_y = (i as f32 * row_h - (y - top)).max(0.0);
    }

    /// Asks for the remembered scroll of the folder shown, when it's being returned to.
    fn request_scroll(&mut self) {
        if self.restore_scroll
            && let Some(y) = self.scroll_memory.get(&(self.tabs[self.active].id.clone(), self.loc.clone())).copied()
            && y > 0.0
            && !self.entries.is_empty()
        {
            self.scroll_req = Some(y);
        }
    }

    fn entered(&mut self) {
        self.want_focus = true;
        self.preview_for = None;
        self.preview = None;
        self.preview_image = None;
        self.typed = (String::new(), None);
        self.filter = TextEdit::new("");
        self.path_edit = None;
        self.menu = None;
        self.table.clear_selection();
        self.entries.clear();
        self.shown.clear();
        self.error = None;
        if let Location::Remote { host, path } = &self.loc
            && let Some(h) = self.hosts.iter_mut().find(|h| &h.id == host)
        {
            h.last = Some(path.clone());
        }
        self.scroll_y = 0.0;
        // Seen recently: show it now; the listing below refreshes it.
        if let Some(cached) = self.listings.get(&self.loc) {
            self.entries = cached.clone();
            self.refilter();
            self.request_scroll();
        }
        self.reload();
    }

    /// Reload as the user asks for it (F5): archives are read again too.
    pub fn refresh(&mut self) {
        if let Location::Archive { file, .. } = &self.loc {
            self.archives.remove(file.as_ref());
        }
        self.reload();
    }

    pub fn reload(&mut self) {
        self.loading = true;
        let loc = self.loc.clone();
        match &loc {
            Location::Local(p) => {
                let p = p.clone();
                self.listing.request(loc, move |loc| (loc, local::list(&p), None));
            }
            Location::Remote { host, path } => {
                let Some(t) = self.target(host) else {
                    self.loading = false;
                    self.error = Some("Not connected".into());
                    return;
                };
                let path = path.clone();
                self.listing.request(loc, move |loc| (loc, ssh::list(&t, &path).map_err(|e| e.message()), None));
            }
            Location::Archive { file, inner } => {
                if let Some(a) = self.archives.get(file.as_ref()) {
                    let r = a.list(inner);
                    return self.apply_listing(loc.clone(), r, None);
                }
                let Some(local) = self.archive_local(file) else {
                    self.loading = false;
                    self.error = Some("Can't open this archive here".into());
                    return;
                };
                let (file, inner, offset, now) = ((**file).clone(), inner.clone(), self.utc_offset, self.now);
                self.listing.request(loc, move |loc| match archive::index(&local, offset, now) {
                    Ok(a) => (loc, a.list(&inner), Some((file, Arc::new(a)))),
                    Err(e) => (loc, Err(e), None),
                });
            }
        }
    }

    /// The archive file on this machine: itself, or for one on a server its cached download.
    fn archive_local(&self, file: &Location) -> Option<PathBuf> {
        match file {
            Location::Local(p) => Some(p.clone()),
            Location::Remote { host, path } => Some(ops::open_cache_path(&local::cache_dir(), &self.target(host)?, path)),
            Location::Archive { .. } => None,
        }
    }

    fn apply_listing(&mut self, loc: Location, r: Result<Vec<Entry>, String>, index: Option<(Location, Arc<Archive>)>) {
        if let Some((file, a)) = index {
            if self.archives.len() >= 8 {
                self.archives.clear();
            }
            self.archives.insert(file, a);
        }
        if let Ok(entries) = &r {
            if !self.listings.contains_key(&loc) {
                self.listing_order.push_back(loc.clone());
                if self.listing_order.len() > LISTING_CACHE
                    && let Some(old) = self.listing_order.pop_front()
                {
                    self.listings.remove(&old);
                }
            }
            self.listings.insert(loc.clone(), entries.clone());
        }
        if loc != self.loc {
            return;
        }
        self.loading = false;
        self.now = now();
        if std::mem::take(&mut self.want_focus) {
            self.focus_req = Some("files");
        }
        match r {
            Ok(entries) => {
                self.entries = entries;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(e);
            }
        }
        self.refilter();
        // Returning here: put the scroll back once the rows are in, then stop.
        self.request_scroll();
        self.restore_scroll = false;
    }

    /// Rebuilds `shown` from the entries, the hidden-files switch, the filter and the sort.
    pub fn refilter(&mut self) {
        let needle = self.filter.text.to_lowercase();
        let mut shown: Vec<usize> = (0..self.entries.len())
            .filter(|&i| {
                let e = &self.entries[i];
                (self.show_hidden || !e.hidden() || !needle.is_empty() && needle.starts_with('.'))
                    && (needle.is_empty() || e.name.to_lowercase().contains(&needle))
            })
            .collect();
        let (key, asc) = self.sort;
        shown.sort_by(|&a, &b| model::compare(&self.entries[a], &self.entries[b], key, asc));
        self.shown = shown;
    }

    pub fn set_sort(&mut self, column: &str, dir: SortDir) {
        self.sort = (SortKey::from_column(column), dir == SortDir::Ascending);
        self.refilter();
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.refilter();
    }

    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// The selected names that still exist, in display order.
    pub fn selection(&self) -> Vec<String> {
        self.shown.iter().map(|&i| &self.entries[i].name).filter(|n| self.table.is_selected(n)).cloned().collect()
    }

    /// Opens a folder; an archive as a folder; a file in its default app (downloading it from a
    /// server, or extracting it from an archive, first).
    pub fn open(&mut self, name: &str) {
        self.open_as(name, true);
    }

    /// Opens a file in its default app even when it's an archive.
    pub fn open_external(&mut self, name: &str) {
        self.open_as(name, false);
    }

    fn open_as(&mut self, name: &str, browse_archives: bool) {
        let Some(e) = self.entry(name).cloned() else { return };
        let item = self.loc.join(name);
        if e.is_dir() {
            return self.go(item);
        }
        let browse = browse_archives && archive::is_archive_name(name);
        if let Location::Archive { file, inner } = &self.loc {
            let Some(a) = self.archives.get(file.as_ref()).cloned() else { return };
            let inside = if inner.is_empty() { name.to_owned() } else { format!("{inner}/{name}") };
            let dir = local::cache_dir().join("extract").join(digest(&(a.local.clone(), inside.clone())));
            let target = a.extracted(&dir, &inside);
            let after = if browse {
                After::Browse(Location::Archive { file: Box::new(Location::Local(target.clone())), inner: String::new() })
            } else {
                After::Open(target.clone())
            };
            if target.exists() {
                return self.then(after);
            }
            let job = Job::new(format!("Extracting {name}"), Direction::Local, archive::extract(&a.local, &[a.member(&inside)], &dir))
                .done_title(format!("Extracted {name}"))
                .detail(format!("from {}", file.name()))
                .then(after);
            self.jobs.push(job);
            return;
        }
        match self.end(&item) {
            Some(End::Local(p)) if browse => {
                let _ = p;
                self.go(Location::Archive { file: Box::new(item), inner: String::new() });
            }
            Some(End::Local(p)) => xdg_open(&p),
            Some(src @ End::Remote(..)) => {
                let End::Remote(t, path) = &src else { return };
                let dest = ops::open_cache_path(&local::cache_dir(), t, path);
                let after =
                    if browse { After::Browse(Location::Archive { file: Box::new(item.clone()), inner: String::new() }) } else { After::Open(dest.clone()) };
                // Downloaded before and unchanged on the server: no scp at all.
                if cache_fresh(&dest, &e) {
                    return self.then(after);
                }
                if let Some(dir) = dest.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                // A new download replaces a stale copy, and any index read from it.
                self.archives.remove(&item);
                let tr = ops::transfer(&src, &End::Local(dest.clone()), Mode::Copy, true, &self.stage());
                let job = Job::new(format!("Downloading {}", e.name), tr.direction, tr.steps)
                    .done_title(format!("Downloaded {}", e.name))
                    .cleanup(tr.cleanup)
                    .detail(src.label())
                    .then(after)
                    .transfer();
                self.jobs.push(job);
            }
            None => {}
        }
    }

    fn then(&mut self, after: After) {
        match after {
            After::Open(p) => xdg_open(&p),
            After::Browse(loc) => self.go(loc),
        }
    }

    /// Unpacks the selected archives beside themselves, each into a folder named after it.
    pub fn extract_here(&mut self) {
        if self.read_only() {
            return self.fail("Extract the archive from the folder it's in".into());
        }
        let names: Vec<String> = self.selection().into_iter().filter(|n| archive::is_archive_name(n) && self.entry(n).is_some_and(|e| !e.is_dir())).collect();
        if names.is_empty() {
            return self.fail("Select an archive to extract".into());
        }
        let mut taken: Vec<String> = self.entries.iter().map(|e| e.name.clone()).collect();
        for name in names {
            let stem = archive::stem(&name);
            let folder = if taken.contains(&stem) { model::unique_copy_name(&stem, &|c| taken.iter().any(|t| t == c)) } else { stem };
            taken.push(folder.clone());
            let (Some(src), Some(dst)) = (self.end(&self.loc.join(&name)), self.end(&self.loc.join(&folder))) else { continue };
            let steps = match (&src, &dst) {
                (End::Local(a), End::Local(d)) => archive::extract(a, &[], d),
                (End::Remote(t, a), End::Remote(_, d)) => vec![crate::jobs::Step::remote(t, archive::remote_extract_script(a, d, &name))],
                _ => continue,
            };
            let job = Job::new(format!("Extracting {name}"), Direction::Local, steps)
                .done_title(format!("Extracted {name}"))
                .detail(format!("into {folder}"))
                .transfer()
                .refresh(self.loc.clone());
            self.jobs.push(job);
        }
    }

    /// Inside an archive nothing can be changed.
    pub fn read_only(&self) -> bool {
        self.loc.is_archive()
    }

    fn refuse_read_only(&mut self) -> bool {
        if self.read_only() {
            self.fail("Archives are read-only here: copy items out (Ctrl+C, then paste in a folder) or extract it.".into());
        }
        self.read_only()
    }

    /// How to copy `name` from `from` to `dst`: extracted when it's in an archive, else a copy
    /// or move on one machine or over scp.
    fn copy_out(&self, from: &Location, name: &str, dst: &End, mode: Mode, replace: bool) -> Option<ops::Transfer> {
        if let Location::Archive { file, inner } = from {
            let a = self.archives.get(file.as_ref())?;
            let inside = if inner.is_empty() { name.to_owned() } else { format!("{inner}/{name}") };
            let stage = self.stage();
            let mut steps = archive::extract(&a.local, &[a.member(&inside)], &stage);
            // The extracted copy is ours to move: on this machine that's a rename.
            let tr = ops::transfer(&End::Local(a.extracted(&stage, &inside)), dst, Mode::Move, replace, &self.stage());
            steps.extend(tr.steps);
            let mut cleanup = tr.cleanup;
            cleanup.push(crate::jobs::Step::Func(Box::new(move || {
                let _ = std::fs::remove_dir_all(&stage);
                Ok(())
            })));
            return Some(ops::Transfer { steps, cleanup, direction: tr.direction });
        }
        let src = self.end(&from.join(name))?;
        Some(ops::transfer(&src, dst, mode, replace, &self.stage()))
    }

    // ---- Hosts ---------------------------------------------------------------------------------

    pub fn target(&self, host: &str) -> Option<Target> {
        self.hosts.iter().find(|h| h.id == host).map(|h| h.target.clone())
    }

    /// The place a location names, with its host's login.
    pub fn end(&self, loc: &Location) -> Option<End> {
        match loc {
            Location::Local(p) => Some(End::Local(p.clone())),
            Location::Remote { host, path } => Some(End::Remote(self.target(host)?, path.clone())),
            Location::Archive { .. } => None,
        }
    }

    pub fn connect(&mut self, target: Target, path: Option<String>) {
        self.connecting = Some(target.id());
        self.connect_req.request((target, path), |(t, p)| {
            let r = ssh::connect(&t);
            (t, p, r)
        });
    }

    /// Opens a host from the sidebar: where it was left, else its home; connects first if needed.
    pub fn open_host(&mut self, id: &str) {
        let Some(h) = self.hosts.iter().find(|h| h.id == id) else { return };
        match (&h.home, &h.last) {
            (Some(home), last) => {
                let path = last.clone().unwrap_or_else(|| home.clone());
                self.go(Location::Remote { host: id.to_owned(), path });
            }
            (None, _) => {
                let t = h.target.clone();
                self.connect(t, None);
            }
        }
    }

    // ---- Pinned servers ----------------------------------------------------------------------

    pub fn is_pinned(&self, id: &str) -> bool {
        self.pinned.iter().any(|p| p == id)
    }

    /// Pins a server to the top of the sidebar, or unpins it.
    pub fn toggle_pin(&mut self, id: &str) {
        if self.is_pinned(id) {
            self.pinned.retain(|p| p != id);
        } else {
            self.pinned.push(id.to_owned());
        }
        save_pinned(&self.pinned);
    }

    /// Moves a pinned server from one place in the pinned list to another (drag to reorder).
    pub fn move_pin(&mut self, from: usize, to: usize) {
        if from < self.pinned.len() && to < self.pinned.len() {
            let id = self.pinned.remove(from);
            self.pinned.insert(to, id);
            save_pinned(&self.pinned);
        }
    }

    /// Drops a server typed in by address from the sidebar (`~/.ssh/config` hosts stay).
    pub fn forget_host(&mut self, id: &str) {
        if self.hosts.iter().any(|h| h.id == id && h.home.is_some()) {
            self.disconnect(id);
        }
        self.hosts.retain(|h| h.id != id || h.from_config);
        if self.is_pinned(id) && !self.hosts.iter().any(|h| h.id == id) {
            self.toggle_pin(id);
        }
        save_recent(&self.hosts);
    }

    pub fn disconnect(&mut self, id: &str) {
        let Some(h) = self.hosts.iter_mut().find(|h| h.id == id) else { return };
        let t = h.target.clone();
        h.home = None;
        h.last = None;
        h.target.password = None;
        std::thread::spawn(move || t.close_master());
        for (i, t) in self.tabs.iter_mut().enumerate() {
            if i != self.active && t.loc.host() == Some(id) {
                *t = Tab { id: t.id.clone(), ..Tab::new(Location::Local(local::home())) };
            }
        }
        if self.loc.host() == Some(id) {
            self.back.retain(|l| l.host() != Some(id));
            self.forward.retain(|l| l.host() != Some(id));
            self.loc = Location::Local(local::home());
            self.entered();
        }
    }

    pub fn open_connect_dialog(&mut self, server: &str, error: Option<String>) {
        let mut s = TextEdit::new(server);
        let mut password = TextEdit::new("").secure();
        if server.is_empty() || error.is_none() {
            s.focused = true;
        } else {
            password.focused = true;
        }
        s.anchor = s.text.len();
        s.focus = s.text.len();
        self.dialog = Some(Dialog::Connect { server: s, password, key: TextEdit::new(""), error });
    }

    pub fn submit_connect(&mut self) {
        let Some(Dialog::Connect { server, password, key, error }) = &mut self.dialog else { return };
        match Target::parse(&server.text) {
            Ok((mut t, path)) => {
                let known = self.hosts.iter().find(|h| h.id == t.id()).map(|h| h.target.clone());
                if let Some(k) = known {
                    t.config = k.config;
                    t.identity = k.identity;
                }
                if !password.text.is_empty() {
                    t.password = Some(password.text.clone());
                }
                let k = key.text.trim();
                if !k.is_empty() {
                    t.identity = Some(expand_tilde(k));
                }
                *error = None;
                self.connect(t, path);
            }
            Err(e) => *error = Some(e),
        }
    }

    pub fn trust_and_connect(&mut self) {
        if let Some(Dialog::Trust { target, path, .. }) = self.dialog.take() {
            let mut t = target;
            t.accept_new = true;
            self.connect(t, path);
        }
    }

    fn connected(&mut self, t: Target, path: Option<String>, home: String) {
        let id = t.id();
        let start = path.map(|p| if let Some(rest) = p.strip_prefix('~') { format!("{home}{rest}") } else { p }).unwrap_or_else(|| home.clone());
        match self.hosts.iter_mut().find(|h| h.id == id) {
            Some(h) => {
                h.target = Target { accept_new: false, ..t };
                h.home = Some(home);
            }
            None => {
                self.hosts.push(Host { id: id.clone(), target: Target { accept_new: false, ..t }, home: Some(home), last: None, from_config: false });
                save_recent(&self.hosts);
            }
        }
        if matches!(self.dialog, Some(Dialog::Connect { .. } | Dialog::Trust { .. })) {
            self.dialog = None;
        }
        self.go(Location::Remote { host: id, path: start });
    }

    // ---- Selection actions ---------------------------------------------------------------------

    /// Copy or cut the selection: kept here for paste, and put on the system clipboard as
    /// files (local ones, which other apps paste) or as paths.
    pub fn copy(&mut self, mode: Mode, tx: &mut Tx) {
        let names = self.selection();
        if names.is_empty() {
            return;
        }
        if mode == Mode::Move && self.refuse_read_only() {
            return;
        }
        match &self.loc {
            Location::Local(dir) => tx.copy_rich(rune::ClipboardContent::default().with_files(names.iter().map(|n| dir.join(n)))),
            _ => tx.copy(names.iter().map(|n| self.loc_label(&self.loc.join(n))).collect::<Vec<_>>().join("\n")),
        }
        self.clip = Some(Clip { mode, dir: self.loc.clone(), names });
    }

    /// Ctrl+V: files another app copied paste in (uploaded on a server); otherwise what was
    /// copied here.
    pub fn paste_clipboard(&mut self, tx: &mut Tx) {
        tx.read_clipboard(|s: &mut App, content: &rune::ClipboardContent, _tx: &mut Tx| {
            let files: Vec<PathBuf> = content.files().to_vec();
            let ours: Vec<PathBuf> = match &s.clip {
                Some(Clip { dir: Location::Local(d), names, .. }) => names.iter().map(|n| d.join(n)).collect(),
                _ => Vec::new(),
            };
            if !files.is_empty() && files != ours {
                s.upload_paths(&files);
            } else {
                s.paste();
            }
        });
    }

    pub fn paste(&mut self) {
        if self.clip.is_none() || self.refuse_read_only() {
            return;
        }
        let Some(clip) = &self.clip else { return };
        let plan = Paste { mode: clip.mode, from: clip.dir.clone(), names: clip.names.clone(), to: self.loc.clone() };
        if plan.mode == Mode::Move && plan.from == plan.to {
            return;
        }
        if plan.from.same_side(&plan.to) {
            for n in &plan.names {
                let inside = plan.from.join(n);
                if plan.to == inside || plan.to.path_str().starts_with(&format!("{}/", inside.path_str())) {
                    return self.fail(format!("Can't put “{n}” inside itself"));
                }
            }
        }
        let conflicts: Vec<String> = if plan.from == plan.to { Vec::new() } else { plan.names.iter().filter(|n| self.entry(n).is_some()).cloned().collect() };
        if conflicts.is_empty() {
            self.run_paste(plan, Resolve::KeepBoth);
        } else {
            self.dialog = Some(Dialog::Conflict { plan, conflicts });
        }
    }

    pub fn resolve_conflict(&mut self, how: Option<Resolve>) {
        if let Some(Dialog::Conflict { plan, .. }) = self.dialog.take()
            && let Some(how) = how
        {
            self.run_paste(plan, how);
        }
    }

    fn run_paste(&mut self, plan: Paste, how: Resolve) {
        let Some(to) = self.end(&plan.to) else {
            return self.fail("That server isn't connected any more".into());
        };
        let mut taken: Vec<String> = self.entries.iter().map(|e| e.name.clone()).collect();
        let mut queued = 0;
        for name in &plan.names {
            let exists = taken.contains(name);
            let (dest_name, replace) = match (exists, how) {
                (false, _) => (name.clone(), false),
                (true, Resolve::Skip) => continue,
                (true, Resolve::Replace) if plan.from != plan.to => (name.clone(), true),
                (true, _) => (model::unique_copy_name(name, &|c| taken.iter().any(|t| t == c)), false),
            };
            taken.push(dest_name.clone());
            let Some(tr) = self.copy_out(&plan.from, name, &to.join(&dest_name), plan.mode, replace) else {
                return self.fail("Open the archive or server again, then paste".into());
            };
            let (verb, done) = match (plan.mode, tr.direction) {
                _ if plan.from.is_archive() => ("Extracting", "Extracted"),
                (Mode::Move, _) => ("Moving", "Moved"),
                (_, Direction::Upload) => ("Uploading", "Uploaded"),
                (_, Direction::Download) => ("Downloading", "Downloaded"),
                _ => ("Copying", "Copied"),
            };
            let job = Job::new(format!("{verb} {name}"), tr.direction, tr.steps)
                .done_title(format!("{done} {name}"))
                .cleanup(tr.cleanup)
                .detail(format!("to {}", to.label()))
                .transfer()
                .refresh(plan.to.clone())
                .refresh(plan.from.clone());
            self.jobs.push(job);
            queued += 1;
        }
        if plan.mode == Mode::Move {
            self.clip = None;
        }
        if queued > 0 && plan.from.host() != plan.to.host() {
            self.show_transfers();
        }
    }

    pub fn ask_delete(&mut self, permanent: bool) {
        let names = self.selection();
        if names.is_empty() || self.refuse_read_only() {
            return;
        }
        let trash = !permanent && !self.loc.is_remote() && local::has_program("gio");
        if trash {
            self.delete(names, true);
        } else {
            self.dialog = Some(Dialog::Delete { names, trash: false });
        }
    }

    pub fn delete(&mut self, names: Vec<String>, trash: bool) {
        self.dialog = None;
        let n = names.len();
        let items: Vec<End> = names.iter().filter_map(|name| self.end(&self.loc.join(name))).collect();
        let step = if trash {
            let paths: Vec<PathBuf> = items.iter().map(|e| PathBuf::from(e.path())).collect();
            Some(ops::trash(&paths))
        } else {
            ops::delete(&items)
        };
        let Some(step) = step else { return };
        let what = if n == 1 { format!("“{}”", names[0]) } else { format!("{n} items") };
        let title = if trash { format!("Moving {what} to Trash") } else { format!("Deleting {what}") };
        self.jobs.push(Job::new(title, Direction::Local, vec![step]).refresh(self.loc.clone()));
        self.table.clear_selection();
    }

    pub fn start_new_folder(&mut self) {
        if self.refuse_read_only() {
            return;
        }
        let mut name = TextEdit::new("New Folder");
        name.focused = true;
        name.anchor = 0;
        name.focus = name.text.len();
        self.dialog = Some(Dialog::NewFolder { name, error: None, file: false });
    }

    pub fn start_new_file(&mut self) {
        if self.refuse_read_only() {
            return;
        }
        let mut name = TextEdit::new("New File.txt");
        name.focused = true;
        name.anchor = 0;
        name.focus = "New File".len();
        self.dialog = Some(Dialog::NewFolder { name, error: None, file: true });
    }

    pub fn submit_new_folder(&mut self) {
        let Some(Dialog::NewFolder { name, error, file }) = &mut self.dialog else { return };
        let file = *file;
        let n = name.text.trim().to_owned();
        if let Err(e) = model::valid_name(&n) {
            *error = Some(e.into());
            return;
        }
        if self.entries.iter().any(|e| e.name == n) {
            *error = Some(format!("“{n}” already exists"));
            return;
        }
        let Some(at) = self.end(&self.loc.join(&n)) else { return };
        self.dialog = None;
        let step = if file { ops::touch(&at) } else { ops::mkdir(&at) };
        self.jobs.push(Job::new(format!("Creating “{n}”"), Direction::Local, vec![step]).refresh(self.loc.clone()));
        self.table.select(n);
    }

    /// Opens the selection: every selected file in its app, or the selected folder.
    pub fn open_selection(&mut self) {
        let sel = self.selection();
        let (dirs, files): (Vec<String>, Vec<String>) = sel.into_iter().partition(|n| self.entry(n).is_some_and(Entry::is_dir));
        if files.is_empty() {
            if let Some(d) = dirs.first() {
                self.open(&d.clone());
            }
        } else {
            for f in files {
                self.open(&f);
            }
        }
    }

    /// Copies the selection beside itself: `a.txt` → `a (copy).txt`.
    pub fn duplicate(&mut self) {
        if self.refuse_read_only() {
            return;
        }
        let names = self.selection();
        if !names.is_empty() {
            self.run_paste(Paste { mode: Mode::Copy, from: self.loc.clone(), names, to: self.loc.clone() }, Resolve::KeepBoth);
        }
    }

    /// Another window on this folder (a server's shares this one's connection).
    pub fn new_window(&mut self) {
        let Ok(exe) = std::env::current_exe() else { return };
        let arg = match &self.loc {
            Location::Local(p) => p.display().to_string(),
            Location::Remote { host, path } => format!("ssh://{host}{path}"),
            // A local archive opens again; one on a server, its folder.
            Location::Archive { file, .. } => match file.as_ref() {
                Location::Local(p) => p.display().to_string(),
                Location::Remote { host, path } => format!("ssh://{host}{}", model::posix_parent(path).unwrap_or_else(|| "/".into())),
                Location::Archive { .. } => return,
            },
        };
        let mut cmd = std::process::Command::new(exe);
        // Its own process: a plain launch would hand the folder to this one as a tab.
        cmd.arg("--new-instance").arg(arg);
        spawn_detached(cmd);
    }

    /// Sorts by a column; asking again for the same one flips the direction.
    pub fn sort_by(&mut self, column: &str) {
        let dir = self.table.next_sort(column);
        self.table.set_sort(column, dir);
        self.set_sort(column, dir);
    }

    /// Type-to-select: letters typed within a second of each other jump to the first name
    /// starting with them; a repeated single letter cycles through names starting with it.
    pub fn type_select(&mut self, t: &str, tx: &mut Tx) -> bool {
        // On Rune's clock, so scripted runs time it as a window does.
        let now = tx.clock();
        if self.typed.1.is_none_or(|at| now - at > 1.0) {
            self.typed.0.clear();
        }
        self.typed.0.push_str(&t.to_lowercase());
        self.typed.1 = Some(now);
        let q = self.typed.0.clone();
        let names: Vec<&str> = self.shown.iter().map(|&i| self.entries[i].name.as_str()).collect();
        let starts = |n: &str| n.to_lowercase().starts_with(&q);
        let cycling = q.chars().count() > 1 && q.chars().all(|c| q.starts_with(c));
        let hit = if cycling {
            let c = q.chars().next().unwrap_or_default().to_string();
            let cur = self.table.cursor().and_then(|id| names.iter().position(|n| *n == id));
            let after = cur.map_or(0, |i| i + 1);
            names.iter().skip(after).chain(names.iter().take(after)).find(|n| n.to_lowercase().starts_with(&c)).copied()
        } else {
            names.iter().find(|n| starts(n)).copied()
        };
        match hit.map(str::to_owned) {
            Some(id) => {
                self.table.reveal(id, tx);
                true
            }
            None => false,
        }
    }

    pub fn show_properties(&mut self) {
        let names = self.selection();
        self.dialog = Some(Dialog::Properties { names });
    }

    /// The context menu from the keyboard (Menu key, Shift+F10), by the selection.
    pub fn menu_from_keyboard(&mut self) {
        let [x, y, w, h] = self.content_at;
        let sel = self.selection();
        let what = if sel.is_empty() { MenuFor::Background } else { MenuFor::Items };
        // Under the cursor row when it's on screen: rows are 28 points below a 36-point header.
        let row = self.table.cursor().and_then(|id| self.shown.iter().position(|&i| self.entries[i].name == id)).unwrap_or(0);
        let py = (y + 36.0 + (row as f32 + 1.0) * 28.0).min(y + h - 40.0).max(y + 40.0);
        self.menu = Some((PointerPress { x: x + (w * 0.25).min(220.0), y: py }, what));
    }

    pub fn start_rename(&mut self) {
        if self.refuse_read_only() {
            return;
        }
        let sel = self.selection();
        let Some(from) = sel.first().filter(|_| sel.len() == 1).cloned() else { return };
        let mut name = TextEdit::new(from.clone());
        name.focused = true;
        name.anchor = 0;
        // Select the name without its extension, as file managers do.
        name.focus = match from.rfind('.') {
            Some(i) if i > 0 && !self.entry(&from).is_some_and(Entry::is_dir) => i,
            _ => from.len(),
        };
        self.dialog = Some(Dialog::Rename { from, name, error: None });
    }

    pub fn submit_rename(&mut self) {
        let Some(Dialog::Rename { from, name, error }) = &mut self.dialog else { return };
        let to = name.text.trim().to_owned();
        if let Err(e) = model::valid_name(&to) {
            *error = Some(e.into());
            return;
        }
        let from = from.clone();
        if to == from {
            self.dialog = None;
            return;
        }
        if self.entries.iter().any(|e| e.name == to) {
            *error = Some(format!("“{to}” already exists"));
            return;
        }
        let (Some(a), Some(b)) = (self.end(&self.loc.join(&from)), self.end(&self.loc.join(&to))) else { return };
        self.dialog = None;
        self.jobs.push(Job::new(format!("Renaming “{from}”"), Direction::Local, vec![ops::rename(&a, &b)]).refresh(self.loc.clone()));
        self.table.select(to);
    }

    /// Downloads the selection from a host into the Downloads folder.
    pub fn download_selection(&mut self) {
        let downloads = local::user_dir("DOWNLOAD", "Downloads");
        let _ = std::fs::create_dir_all(&downloads);
        let names = self.selection();
        let mut taken: Vec<String> = Vec::new();
        let (verb, done) = if self.loc.is_archive() { ("Extracting", "Extracted") } else { ("Downloading", "Downloaded") };
        for name in &names {
            let exists = |c: &str| downloads.join(c).exists() || taken.iter().any(|t| t == c);
            let dest = if exists(name) { model::unique_copy_name(name, &exists) } else { name.clone() };
            taken.push(dest.clone());
            let Some(tr) = self.copy_out(&self.loc, name, &End::Local(downloads.join(&dest)), Mode::Copy, false) else { continue };
            let job = Job::new(format!("{verb} {name}"), tr.direction, tr.steps)
                .done_title(format!("{done} {name}"))
                .cleanup(tr.cleanup)
                .detail(format!("to {}", downloads.display()))
                .transfer()
                .refresh(Location::Local(downloads.clone()));
            self.jobs.push(job);
        }
        if !names.is_empty() {
            self.show_transfers();
        }
    }

    /// Uploads local files (from the file chooser or a drop) into the current folder.
    pub fn upload_paths(&mut self, paths: &[PathBuf]) {
        if self.refuse_read_only() {
            return;
        }
        let Some(to) = self.end(&self.loc) else { return };
        let mut taken: Vec<String> = self.entries.iter().map(|e| e.name.clone()).collect();
        for p in paths {
            let src = End::Local(p.clone());
            let name = src.name();
            let dest = if taken.contains(&name) { model::unique_copy_name(&name, &|c| taken.iter().any(|t| t == c)) } else { name.clone() };
            taken.push(dest.clone());
            let tr = ops::transfer(&src, &to.join(&dest), Mode::Copy, false, &self.stage());
            let (verb, done) = if tr.direction == Direction::Upload { ("Uploading", "Uploaded") } else { ("Copying", "Copied") };
            let job = Job::new(format!("{verb} {name}"), tr.direction, tr.steps)
                .done_title(format!("{done} {name}"))
                .cleanup(tr.cleanup)
                .detail(format!("to {}", to.label()))
                .transfer()
                .refresh(self.loc.clone());
            self.jobs.push(job);
        }
        if self.loc.is_remote() && !paths.is_empty() {
            self.show_transfers();
        }
    }

    pub fn copy_path(&mut self, tx: &mut Tx) {
        let sel = self.selection();
        let locs: Vec<String> = if sel.is_empty() { vec![self.loc_label(&self.loc)] } else { sel.iter().map(|n| self.loc_label(&self.loc.join(n))).collect() };
        let what = if locs.len() == 1 { locs[0].clone() } else { format!("{} paths", locs.len()) };
        tx.copy(locs.join("\n"));
        self.notice = Some(Notice { text: format!("Copied {what}"), error: false });
    }

    /// `/home/a/x` or `user@host:/srv/x`.
    pub fn loc_label(&self, loc: &Location) -> String {
        match loc {
            Location::Local(p) => p.display().to_string(),
            Location::Remote { host, path } => format!("{host}:{path}"),
            Location::Archive { file, inner } if inner.is_empty() => self.loc_label(file),
            Location::Archive { file, inner } => format!("{}/{inner}", self.loc_label(file)),
        }
    }

    /// A terminal in this folder: a shell here, or ssh to the host in this folder.
    pub fn open_terminal(&mut self) {
        let launcher = ["xdg-terminal-exec", "x-terminal-emulator"]
            .into_iter()
            .find(|p| local::has_program(p))
            .map(str::to_owned)
            .or_else(|| std::env::var("TERMINAL").ok());
        let Some(term) = launcher else { return self.fail("No terminal found: install xdg-terminal-exec or set $TERMINAL".into()) };
        let mut cmd = std::process::Command::new(term);
        // Inside an archive: the folder the archive is in.
        let mut at = self.loc.clone();
        while let Location::Archive { file, .. } = &at {
            match file.parent() {
                Some(p) => at = p,
                None => return,
            }
        }
        match &at {
            Location::Archive { .. } => return,
            Location::Local(p) => {
                cmd.current_dir(p);
            }
            Location::Remote { host, path } => {
                let Some(t) = self.target(host) else { return };
                cmd.current_dir(local::home());
                cmd.arg("ssh").arg("-t");
                if let Some(c) = t.config_file() {
                    cmd.args(["-F", &c]);
                }
                if let Some(i) = &t.identity {
                    cmd.args(["-i", i]);
                }
                if let Some(u) = &t.user {
                    cmd.args(["-l", u]);
                }
                if let Some(p) = t.port {
                    cmd.args(["-p", &p.to_string()]);
                }
                cmd.arg(&t.dest).arg(format!("cd {} && exec \"$SHELL\" -l", ssh::quote(path)));
            }
        }
        spawn_detached(cmd);
    }

    // ---- Background results --------------------------------------------------------------------

    /// Lands finished work; true when anything visible changed.
    pub fn poll(&mut self, clock: f64) -> bool {
        let mut changed = false;
        if let Some((loc, r, index)) = self.listing.take() {
            self.apply_listing(loc, r, index);
            changed = true;
        }
        if let Some((input, list)) = self.complete_req.take()
            && self.path_edit.as_ref().is_some_and(|e| e.text == input)
        {
            self.completions = list;
            self.completion_pick = None;
            changed = true;
        }
        if self.terminal.as_ref().is_some_and(|t| t.take_dirty()) {
            changed = true;
        }
        if self.theme_changed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.omarchy = crate::theme::load();
            changed = true;
        }
        if let Some(p) = self.preview_req.take() {
            changed |= self.land_preview(p);
        }
        // A picture decodes on Rune's worker, which wakes the window; redraw once it's done.
        if !self.preview_image_seen && self.preview_image.as_ref().is_some_and(|i| i.is_ready() || i.error().is_some()) {
            self.preview_image_seen = true;
            changed = true;
        }
        self.sync_preview();
        if self.swipe.active() && self.swipe.last.is_some_and(|t| clock - t >= SWIPE_IDLE) {
            self.finish_swipe();
            changed = true;
        }
        if let Some((t, path, r)) = self.connect_req.take() {
            changed = true;
            self.connecting = None;
            match r {
                Ok(home) => self.connected(t, path, home),
                Err(SshError::UnknownHostKey) => {
                    let id = t.id();
                    let probe = t.clone();
                    self.fingerprint_req.request(id, move |id| (id, host_fingerprint(&probe)));
                    self.dialog = Some(Dialog::Trust { target: t, path, fingerprint: None });
                }
                Err(SshError::Auth(msg)) => {
                    self.open_connect_dialog(&t.id(), Some(msg));
                    if let Some(Dialog::Connect { key, .. }) = &mut self.dialog
                        && let Some(i) = &t.identity
                    {
                        *key = TextEdit::new(i.clone());
                    }
                }
                Err(e) => match &mut self.dialog {
                    Some(Dialog::Connect { error, .. }) => *error = Some(e.message()),
                    _ => self.open_connect_dialog(&t.id(), Some(e.message())),
                },
            }
        }
        if let Some((id, fp)) = self.fingerprint_req.take()
            && let Some(Dialog::Trust { target, fingerprint, .. }) = &mut self.dialog
            && target.id() == id
        {
            *fingerprint = Some(fp.unwrap_or_else(|| "Couldn't read the key's fingerprint.".into()));
            changed = true;
        }
        let ended = self.jobs.poll();
        let mut reload = false;
        let mut afters = Vec::new();
        for id in &ended {
            changed = true;
            let Some(job) = self.jobs.list.iter().find(|j| j.id == *id) else { continue };
            if job.refresh.contains(&self.loc) {
                reload = true;
            }
            match &job.state {
                JobState::Done => {
                    if let Some(a) = job.after.clone() {
                        afters.push(a);
                    }
                }
                JobState::Failed(e) => {
                    let msg = format!("{} failed: {e}", job.title);
                    self.notice = Some(Notice { text: msg, error: true });
                }
                _ => {}
            }
        }
        // Quiet work that succeeded leaves the list; transfers stay until cleared.
        if !ended.is_empty() {
            self.jobs.list.retain(|j| j.transfer || !j.state.finished());
        }
        if reload {
            self.reload();
        }
        for a in afters {
            self.then(a);
        }
        changed
    }

    // ---- Path completion -------------------------------------------------------------------

    /// Completions for what the path field holds: folders under the typed folder that start
    /// with the typed name (on this machine, or the server shown), or server names.
    pub fn request_completions(&mut self) {
        let Some(input) = self.path_edit.as_ref().map(|e| e.text.clone()) else { return };
        if input.is_empty() {
            self.completions.clear();
            return;
        }
        // A server name: `user@ho` → the hosts in the sidebar.
        if !input.starts_with('/') && !input.starts_with('~') && !input.contains('/') {
            let low = input.to_lowercase();
            self.completions =
                self.hosts.iter().filter(|h| h.id.to_lowercase().starts_with(&low) && h.id != input).map(|h| format!("{}:", h.id)).take(12).collect();
            self.completion_pick = None;
            return;
        }
        let (dir, prefix) = match input.rfind('/') {
            Some(i) => (input[..=i].to_owned(), input[i + 1..].to_owned()),
            None => (format!("{input}/"), String::new()),
        };
        let typed_dir = dir.clone();
        let pick = move |names: Vec<(String, bool)>| -> Vec<String> {
            let low = prefix.to_lowercase();
            let mut out: Vec<String> = names
                .into_iter()
                .filter(|(n, dir)| *dir && n.to_lowercase().starts_with(&low) && (prefix.starts_with('.') || !n.starts_with('.')))
                .map(|(n, _)| format!("{typed_dir}{n}/"))
                .collect();
            out.sort_by(|a, b| model::natural_cmp(a, b));
            out.truncate(12);
            out
        };
        match self.loc.host().map(str::to_owned) {
            Some(host) => {
                // On a server: `~` is its home; listings seen before answer at once.
                let home = self.hosts.iter().find(|h| h.id == host).and_then(|h| h.home.clone()).unwrap_or_default();
                let abs = if let Some(rest) = dir.strip_prefix('~') { format!("{home}{rest}") } else { dir.clone() };
                let loc = Location::Remote { host: host.clone(), path: abs.trim_end_matches('/').to_owned().max("/".into()) };
                if let Some(cached) = self.listings.get(&loc) {
                    self.completions = pick(cached.iter().map(|e| (e.name.clone(), e.is_dir())).collect());
                    self.completion_pick = None;
                    return;
                }
                let Some(t) = self.target(&host) else { return };
                self.complete_req.request(input, move |input| {
                    let names = ssh::list(&t, &abs).map(|v| v.into_iter().map(|e| (e.name.clone(), e.is_dir())).collect()).unwrap_or_default();
                    (input, pick(names))
                });
            }
            None => {
                let home = local::home();
                let abs = if let Some(rest) = dir.strip_prefix('~') { format!("{}{rest}", home.display()) } else { dir.clone() };
                self.complete_req.request(input, move |input| {
                    let names = local::list(Path::new(&abs)).map(|v| v.into_iter().map(|e| (e.name.clone(), e.is_dir())).collect()).unwrap_or_default();
                    (input, pick(names))
                });
            }
        }
    }

    /// Puts a completion in the field (keeping it open, to go on typing or completing).
    pub fn accept_completion(&mut self, i: usize) {
        let Some(c) = self.completions.get(i).cloned() else { return };
        if let Some(e) = &mut self.path_edit {
            e.replace_text(c.clone());
            e.anchor = c.len();
            e.focus = c.len();
            e.focused = true;
        }
        self.completions.clear();
        self.completion_pick = None;
        self.request_completions();
    }

    /// ↑ ↓ through the completions.
    pub fn move_completion(&mut self, delta: isize) -> bool {
        if self.completions.is_empty() {
            return false;
        }
        let n = self.completions.len() as isize;
        let next = match self.completion_pick {
            Some(i) => (i as isize + delta).rem_euclid(n),
            None if delta > 0 => 0,
            None => n - 1,
        };
        self.completion_pick = Some(next as usize);
        true
    }

    // ---- Terminal panel ------------------------------------------------------------------------

    /// Ctrl+T: opens the panel and focuses the terminal; from the terminal, hides the panel.
    pub fn toggle_terminal(&mut self, tx: &mut Tx) {
        let in_term = tx.focused() == Some("terminal");
        if self.term_open && in_term {
            self.term_open = false;
            tx.focus("files", false);
            return;
        }
        self.term_open = true;
        if self.terminal.as_ref().is_none_or(|t| t.exited()) {
            self.start_terminal();
        }
        tx.focus("terminal", false);
    }

    /// A shell in this folder: here, or on the server over ssh (sharing its connection).
    pub fn start_terminal(&mut self) {
        let Some(waker) = self.waker.clone() else { return };
        // Inside an archive: the folder the archive is in.
        let mut at = self.loc.clone();
        while let Location::Archive { file, .. } = &at {
            match file.parent() {
                Some(p) => at = p,
                None => break,
            }
        }
        let size = self.terminal.as_ref().map_or(crate::term::Size { cols: 100, rows: 12 }, |t| t.size);
        let (cwd, program, host) = match &at {
            Location::Remote { host, path } => {
                let Some(t) = self.target(host) else { return };
                let mut args = Vec::new();
                if let Some(c) = t.config_file() {
                    args.extend(["-F".into(), c]);
                }
                if let Some(i) = &t.identity {
                    args.extend(["-i".into(), i.clone()]);
                }
                if let Some(u) = &t.user {
                    args.extend(["-l".into(), u.clone()]);
                }
                if let Some(p) = t.port {
                    args.extend(["-p".into(), p.to_string()]);
                }
                args.extend(["-o".into(), "ControlMaster=auto".into(), "-o".into(), format!("ControlPath={}", ssh::control_path()), "-t".into()]);
                args.push(t.dest.clone());
                args.push(format!("cd {} && exec \"$SHELL\" -l", ssh::quote(path)));
                (local::home(), Some(("ssh".to_owned(), args)), Some(host.clone()))
            }
            Location::Local(p) => (p.clone(), None, None),
            Location::Archive { .. } => (local::home(), None, None),
        };
        match crate::term::Terminal::spawn(cwd, program, size, waker) {
            Ok(t) => {
                self.terminal = Some(t);
                self.term_host = host;
            }
            Err(e) => self.fail(format!("Couldn't start a terminal: {e}")),
        }
    }

    /// Sends `cd <this folder>` to the terminal (when it's on the same machine).
    pub fn terminal_cd_here(&mut self) {
        let Some(t) = &self.terminal else { return };
        let path = match &self.loc {
            Location::Local(p) if self.term_host.is_none() => p.display().to_string(),
            Location::Remote { host, path } if self.term_host.as_deref() == Some(host) => path.clone(),
            _ => return self.fail("The terminal is on another machine: restart it here instead".into()),
        };
        t.write(format!(" cd {}\r", ssh::quote(&path)).into_bytes());
    }

    /// A key while the terminal has focus: true when the terminal took it.
    pub fn terminal_key(&mut self, ev: &rune::KeyEvent, tx: &mut Tx) -> bool {
        use rune::Key;
        let m = ev.mods;
        let ch = match &ev.key {
            Key::Command(c) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        // Left to the app: the panel toggle, Ctrl+Shift chords, zoom, tab switching, and
        // Ctrl+Alt+B (the right sidebar), which arrives as text with Ctrl and Alt held.
        let ctrl_alt_b = m.cmd && m.alt && (ch == Some('b') || matches!(&ev.key, Key::Text(t) if t.eq_ignore_ascii_case("b")));
        if (m.cmd && !m.shift && ch == Some('t')) || (m.cmd && m.shift && ch != Some('v')) || ctrl_alt_b {
            return false;
        }
        if m.cmd && matches!(ch, Some('=' | '+' | '-' | '0')) {
            return false;
        }
        if m.cmd && matches!(ev.key, Key::Tab | Key::PageUp | Key::PageDown) {
            return false;
        }
        let Some(t) = &self.terminal else { return false };
        if t.exited() {
            if ev.key == Key::Enter {
                self.start_terminal();
            }
            return true;
        }
        if m.cmd && m.shift && ch == Some('v') {
            tx.read_clipboard(|s: &mut App, content: &rune::ClipboardContent, _tx: &mut Tx| {
                if let Some(t) = &s.terminal {
                    t.paste(content.text());
                }
            });
            return true;
        }
        match ev.key {
            Key::PageUp if m.shift => t.scroll(t.size.rows as i32 - 1),
            Key::PageDown if m.shift => t.scroll(-(t.size.rows as i32 - 1)),
            _ => match t.key_bytes(ev) {
                Some(bytes) => {
                    t.scroll(i32::MIN / 2);
                    t.write(bytes);
                }
                None => return false,
            },
        }
        true
    }

    /// The terminal's area changed: fit the terminal to it.
    pub fn terminal_area(&mut self, w: f32, h: f32) {
        self.term_area = [w, h];
        let [cw, ch] = self.term_cell;
        if cw <= 0.0 || ch <= 0.0 {
            return;
        }
        let size = crate::term::Size { cols: ((w / cw).floor() as u16).max(10), rows: ((h / ch).floor() as u16).max(2) };
        if let Some(t) = &mut self.terminal {
            t.resize(size);
        }
    }

    /// The view measured the terminal font's cell.
    pub fn terminal_cell(&mut self, cell: [f32; 2]) {
        self.term_cell = cell;
        let [w, h] = self.term_area;
        if w > 0.0 {
            self.terminal_area(w, h);
        }
    }

    /// Dragging the panel's top edge.
    pub fn drag_terminal(&mut self, d: &rune::Drag) {
        if d.phase == rune::Phase::Down {
            self.term_h_start = self.term_h;
        }
        self.term_h = (self.term_h_start - d.dy).clamp(80.0, 900.0);
    }

    // ---- Tabs ----------------------------------------------------------------------------------

    /// The showing tab's state, moved out of `App` (which keeps empty fields until `put_tab`).
    fn take_tab(&mut self) -> Tab {
        self.remember_scroll();
        let filter = std::mem::replace(&mut self.filter, TextEdit::new(""));
        let mut t = Tab::new(self.loc.clone());
        t.id = self.tabs[self.active].id.clone();
        t.back = std::mem::take(&mut self.back);
        t.forward = std::mem::take(&mut self.forward);
        t.entries = std::mem::take(&mut self.entries);
        t.shown = std::mem::take(&mut self.shown);
        t.table = std::mem::replace(&mut self.table, new_table());
        t.error = self.error.take();
        t.filter = filter;
        t
    }

    fn put_tab(&mut self, t: Tab) {
        self.restore_scroll = true;
        self.scroll_y = 0.0;
        self.loc = t.loc;
        self.back = t.back;
        self.forward = t.forward;
        self.entries = t.entries;
        self.shown = t.shown;
        self.table = t.table;
        self.error = t.error;
        self.filter = t.filter;
        self.path_edit = None;
        self.menu = None;
        self.swipe = Swipe::default();
        self.typed = (String::new(), None);
        self.preview_for = None;
        self.preview = None;
        self.preview_image = None;
        self.want_focus = true;
        self.focus_req = Some("files");
        self.refilter();
        // Shown as it was left; a fresh listing replaces it.
        self.reload();
    }

    /// A tab's folder, the showing one's included.
    pub fn tab_loc(&self, i: usize) -> &Location {
        if i == self.active { &self.loc } else { &self.tabs[i].loc }
    }

    pub fn switch_tab(&mut self, i: usize) {
        if i == self.active || i >= self.tabs.len() {
            return;
        }
        let cur = self.take_tab();
        self.tabs[self.active] = cur;
        let mut placeholder = Tab::new(Location::Local(PathBuf::from("/")));
        placeholder.id = self.tabs[i].id.clone();
        let next = std::mem::replace(&mut self.tabs[i], placeholder);
        self.active = i;
        self.put_tab(next);
    }

    /// A new tab after this one, on `loc` (this tab's folder when None), and shows it.
    pub fn new_tab(&mut self, loc: Option<Location>) {
        let loc = loc.unwrap_or_else(|| self.loc.clone());
        let cur = self.take_tab();
        self.tabs[self.active] = cur;
        self.active += 1;
        let mut t = Tab::new(loc.clone());
        let mut placeholder = Tab::new(loc.clone());
        placeholder.id = t.id.clone();
        self.tabs.insert(self.active, placeholder);
        if let Some(cached) = self.listings.get(&loc) {
            t.entries = cached.clone();
        }
        self.put_tab(t);
    }

    /// Closes a tab; false when it was the last one (the window should close).
    pub fn close_tab(&mut self, i: usize) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        self.closed.push(self.tab_loc(i).clone());
        if i == self.active {
            let to = if i + 1 < self.tabs.len() { i + 1 } else { i - 1 };
            self.switch_tab(to);
        }
        self.tabs.remove(i);
        if self.active > i {
            self.active -= 1;
        }
        true
    }

    pub fn tab_index(&self, id: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// A tab dragged along the strip.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return;
        }
        let t = self.tabs.remove(from);
        self.tabs.insert(to, t);
        self.active = if self.active == from {
            to
        } else if from < self.active && to >= self.active {
            self.active - 1
        } else if from > self.active && to <= self.active {
            self.active + 1
        } else {
            self.active
        };
    }

    pub fn reopen_tab(&mut self) {
        if let Some(loc) = self.closed.pop() {
            self.new_tab(Some(loc));
        }
    }

    pub fn cycle_tab(&mut self, delta: isize) {
        let n = self.tabs.len() as isize;
        if n > 1 {
            self.switch_tab((self.active as isize + delta).rem_euclid(n) as usize);
        }
    }

    /// Opens a folder entry in a new tab (middle click, the menu).
    /// The only tab, still on home with no history: a request to show a folder replaces it
    /// rather than opening beside it.
    fn untouched(&self) -> bool {
        self.tabs.len() == 1 && self.back.is_empty() && self.forward.is_empty() && self.loc == Location::Local(local::home())
    }

    /// Locations from the command line, a second launch or D-Bus (`ShowFolders`…): each in a
    /// tab. Files open their folder with them selected.
    pub fn open_targets(&mut self, targets: &[String], how: crate::ipc::Show) {
        use crate::ipc::Show;
        for t in targets {
            let Some((path, is_dir)) = crate::ipc::resolve(t) else {
                match Target::parse(t) {
                    Ok((target, path)) => {
                        if !self.untouched() {
                            self.new_tab(None);
                        }
                        self.connect(target, path);
                    }
                    Err(e) => self.fail(e),
                }
                continue;
            };
            let archive = !is_dir && archive::is_archive_name(&path.to_string_lossy());
            let (loc, select) = match (how, is_dir) {
                (Show::Folder, true) => (Location::Local(path), None),
                (Show::Folder, false) if archive => (Location::Archive { file: Box::new(Location::Local(path)), inner: String::new() }, None),
                _ => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                    (Location::Local(path.parent().map_or_else(|| PathBuf::from("/"), Path::to_path_buf)), name)
                }
            };
            if self.untouched() {
                self.go(loc);
                self.back.clear();
            } else {
                self.new_tab(Some(loc));
            }
            if let Some(name) = select {
                self.table.select(name);
                if how == Show::Properties {
                    self.side = Side::Preview;
                    self.activity.set_open(true);
                }
            }
        }
    }

    /// Opens the selected folders in new tabs (Ctrl+Enter, the menu).
    pub fn open_selection_in_tabs(&mut self) {
        for n in self.selection() {
            self.open_in_tab(&n);
        }
    }

    pub fn open_in_tab(&mut self, name: &str) {
        match self.entry(name) {
            Some(e) if e.is_dir() => {
                let loc = self.loc.join(name);
                self.new_tab(Some(loc));
            }
            Some(e) if archive::is_archive_name(&e.name) => {
                let loc = Location::Archive { file: Box::new(self.loc.join(name)), inner: String::new() };
                if self.loc.is_remote() { self.open(name) } else { self.new_tab(Some(loc)) }
            }
            _ => {}
        }
    }

    // ---- Previews ------------------------------------------------------------------------------

    /// The preview pane or Quick Look is showing.
    pub fn previewing(&self) -> bool {
        (self.activity.is_open() && self.side == Side::Preview) || matches!(self.dialog, Some(Dialog::QuickLook))
    }

    /// Asks for the preview of the one selected item when it changed and a preview shows.
    pub fn sync_preview(&mut self) {
        if !self.previewing() {
            return;
        }
        let sel = self.selection();
        let want = match (sel.len(), self.table.cursor()) {
            (1, _) => Some(sel[0].clone()),
            (n, Some(c)) if n > 1 && sel.iter().any(|x| x == c) => Some(c.to_owned()),
            _ => None,
        };
        let loc = want.as_ref().map(|n| self.loc.join(n));
        if loc == self.preview_for {
            return;
        }
        self.preview_for = loc.clone();
        let (Some(loc), Some(name)) = (loc, want) else {
            self.preview = None;
            self.preview_image = None;
            self.preview_req.cancel();
            return;
        };
        let Some(entry) = self.entry(&name).cloned() else { return };
        let source = match &self.loc {
            Location::Local(p) => preview::Source::Local(p.join(&name)),
            Location::Remote { host, path } => {
                let Some(t) = self.target(host) else { return };
                preview::Source::Remote(t, model::posix_join(path, &name))
            }
            Location::Archive { file, inner } => {
                let Some(a) = self.archives.get(file.as_ref()) else { return };
                let inside = if inner.is_empty() { name.clone() } else { format!("{inner}/{name}") };
                let dir = local::cache_dir().join("extract").join(digest(&(a.local.clone(), inside.clone())));
                preview::Source::Member { archive: a.local.clone(), member: a.member(&inside), extracted: a.extracted(&dir, &inside), dir }
            }
        };
        let cache = local::cache_dir();
        self.preview_req.request((loc, entry, source), move |(loc, e, src)| preview::make(loc, e, src, cache));
    }

    fn land_preview(&mut self, p: Preview) -> bool {
        if self.preview_for.as_ref() != Some(&p.loc) {
            return false;
        }
        self.preview_image = match (&p.body, &self.images) {
            (preview::Body::Image(path), Some(cache)) => Some(cache.file(path, 900.0, 2.0)),
            _ => None,
        };
        self.preview_image_seen = false;
        self.preview = Some(p);
        true
    }

    /// Shows the preview pane on a tab, or hides it when it already shows that tab.
    pub fn toggle_side(&mut self, side: Side) {
        if self.activity.is_open() && self.side == side {
            self.activity.set_open(false);
        } else {
            self.side = side;
            self.activity.set_open(true);
        }
    }

    pub fn show_transfers(&mut self) {
        self.side = Side::Transfers;
        self.activity.set_open(true);
    }

    pub fn toggle_quick_look(&mut self) {
        if matches!(self.dialog, Some(Dialog::QuickLook)) {
            self.dialog = None;
        } else if self.dialog.is_none() && !self.selection().is_empty() {
            self.dialog = Some(Dialog::QuickLook);
            self.focus_req = Some("quick-look-body");
            self.sync_preview();
        }
    }

    /// Moves the selection one row (Quick Look's ↑ ↓).
    pub fn step_selection(&mut self, delta: isize, tx: &mut Tx) {
        let names: Vec<&str> = self.shown.iter().map(|&i| self.entries[i].name.as_str()).collect();
        if names.is_empty() {
            return;
        }
        let cur = self.table.cursor().and_then(|c| names.iter().position(|n| *n == c));
        let next = match cur {
            Some(i) => (i as isize + delta).clamp(0, names.len() as isize - 1) as usize,
            None => 0,
        };
        let id = names[next].to_owned();
        self.table.reveal(id, tx);
        self.sync_preview();
    }

    // ---- Swipes --------------------------------------------------------------------------------

    /// A wheel delta: a trackpad's sideways swipe is ours (back or forward when it ends); any
    /// other gesture, and mouse wheels, scroll as usual.
    pub fn swipe(&mut self, e: &WheelEvent, tx: &mut Tx) -> bool {
        if !e.precise || e.mods.cmd || e.mods.shift {
            return false;
        }
        let now = tx.clock();
        let idle = self.swipe.last.is_none_or(|t| now - t >= SWIPE_IDLE);
        if e.phase == rune::GesturePhase::Began || idle {
            if self.swipe.active() {
                self.finish_swipe();
            }
            self.swipe = Swipe::default();
        }
        self.swipe.last = Some(now);
        if e.momentum {
            return self.swipe.active();
        }
        if e.phase == rune::GesturePhase::Ended {
            let ours = self.swipe.active();
            self.finish_swipe();
            return ours;
        }
        if self.swipe.sideways.is_none() {
            if e.dx == 0.0 && e.dy == 0.0 {
                return false;
            }
            // Sideways only if it's mostly sideways and there's history that way.
            let possible = if e.dx > 0.0 { !self.back.is_empty() } else { !self.forward.is_empty() };
            self.swipe.sideways = Some(e.dx.abs() > e.dy.abs() * 1.5 && possible);
        }
        if !self.swipe.active() {
            return false;
        }
        // Deltas come in view points; finger travel shouldn't depend on the zoom.
        self.swipe.dx += e.dx * self.zoom;
        // Ends on the gesture's Ended phase, or after a pause where the platform sends none.
        tx.wake_after(std::time::Duration::from_secs_f64(SWIPE_IDLE));
        true
    }

    fn finish_swipe(&mut self) {
        let dx = self.swipe.dx;
        self.swipe = Swipe::default();
        if dx >= SWIPE_DISTANCE {
            self.go_back();
        } else if dx <= -SWIPE_DISTANCE {
            self.go_forward();
        }
    }

    /// Drops cached downloads, extractions and listings.
    pub fn clear_cache(&mut self) {
        self.listings.clear();
        self.listing_order.clear();
        self.archives.clear();
        let cache = local::cache_dir();
        std::thread::spawn(move || {
            for d in ["open", "extract"] {
                let _ = std::fs::remove_dir_all(cache.join(d));
            }
        });
        self.notice = Some(Notice { text: "Cleared downloaded and extracted files".into(), error: false });
    }

    /// A fresh staging folder for one host-to-host copy.
    fn stage(&self) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        local::cache_dir().join("stage").join(format!("{}-{n}", std::process::id()))
    }

    /// One step up or down the zoom ladder (Ctrl+= / Ctrl+-), or back to the default (Ctrl+0).
    pub fn zoom_step(&mut self, dir: i32) {
        let next = match dir {
            0 => DEFAULT_ZOOM,
            d if d > 0 => ZOOM_STEPS.iter().copied().find(|z| *z > self.zoom + 0.001).unwrap_or(MAX_ZOOM),
            _ => ZOOM_STEPS.iter().rev().copied().find(|z| *z < self.zoom - 0.001).unwrap_or(MIN_ZOOM),
        };
        self.set_zoom(next);
    }

    /// Sets the zoom (Ctrl+scroll and pinches set it smoothly) and saves it.
    pub fn set_zoom(&mut self, z: f32) {
        let z = z.clamp(MIN_ZOOM, MAX_ZOOM);
        if (z - self.zoom).abs() < 0.0005 {
            return;
        }
        self.zoom = z;
        let text = format!("{z:.3}\n");
        std::thread::spawn(move || {
            let f = zoom_file();
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(f, text);
        });
    }

    pub fn fail(&mut self, text: String) {
        self.notice = Some(Notice { text, error: true });
    }
}

/// Folders whose listings are kept for instant revisits.
const LISTING_CACHE: usize = 100;
/// Downloaded and extracted files kept on disk; past it, the least recently used go first.
const CACHE_LIMIT: u64 = 2 << 30;

/// A download in the cache is still the server's file: same size and modification time
/// (`scp -p` keeps the time).
pub fn cache_fresh(path: &Path, e: &Entry) -> bool {
    let Ok(m) = std::fs::metadata(path) else { return false };
    let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs_f64());
    Some(m.len()) == e.size && matches!((mtime, e.modified), (Some(a), Some(b)) if (a - b).abs() < 1.0)
}

/// Keeps the download and extraction caches under `limit` bytes, removing least recently
/// used files first.
fn prune_cache(cache: &Path, limit: u64) {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64, std::time::SystemTime)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(m) = e.path().symlink_metadata() else { continue };
            if m.is_dir() {
                walk(&e.path(), out);
            } else {
                let used = m.accessed().or_else(|_| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                out.push((e.path(), m.len(), used));
            }
        }
    }
    let mut files = Vec::new();
    for d in ["open", "extract"] {
        walk(&cache.join(d), &mut files);
    }
    // Staging folders left by a crash.
    let _ = std::fs::remove_dir_all(cache.join("stage"));
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.2);
    for (path, size, _) in files {
        if total <= limit {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= size;
        }
    }
}

/// A short stable name for cache folders.
pub fn digest(v: &impl std::hash::Hash) -> String {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    format!("{:016x}", h.finish())
}

pub fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn expand_tilde(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => local::home().join(rest).display().to_string(),
        None => p.to_owned(),
    }
}

/// Opens a file in its default app (terminal apps in a terminal: see `open.rs`).
pub fn xdg_open(p: &std::path::Path) {
    if rune::tooling::headless_run().is_some() {
        return;
    }
    crate::open::open(p);
}

/// Starts another program (an app, a terminal, a window) on its own. Never in headless runs:
/// scripted tests must not open things on the desktop.
pub fn spawn_detached(mut cmd: std::process::Command) {
    if rune::tooling::headless_run().is_some() {
        return;
    }
    use std::os::unix::process::CommandExt;
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).process_group(0);
    if let Ok(mut child) = cmd.spawn() {
        // Reap it so it doesn't linger as a zombie.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// `SHA256:…` for the host's ed25519 (else first) key, through `ssh -G` so aliases resolve.
fn host_fingerprint(t: &Target) -> Option<String> {
    use std::process::Command;
    let mut g = Command::new("ssh");
    if let Some(c) = t.config_file() {
        g.args(["-F", &c]);
    }
    if let Some(u) = &t.user {
        g.args(["-l", u]);
    }
    if let Some(p) = t.port {
        g.args(["-p", &p.to_string()]);
    }
    let out = g.arg("-G").arg("--").arg(&t.dest).output().ok()?;
    let conf = String::from_utf8_lossy(&out.stdout);
    let get = |k: &str| conf.lines().find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_owned()));
    let host = get("hostname ")?;
    let port = get("port ").unwrap_or_else(|| "22".into());
    let scan = Command::new("ssh-keyscan").args(["-T", "5", "-p", &port, "-t", "ed25519,ecdsa,rsa", "--", &host]).output().ok()?;
    let mut keygen =
        Command::new("ssh-keygen").args(["-l", "-f", "-"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().ok()?;
    {
        use std::io::Write;
        keygen.stdin.take()?.write_all(&scan.stdout).ok()?;
    }
    let out = keygen.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    let pick = lines.iter().find(|l| l.contains("ED25519")).or_else(|| lines.first())?;
    // "256 SHA256:abc… host (ED25519)" → "SHA256:abc… (ED25519)"
    let mut parts = pick.split_whitespace();
    let _bits = parts.next();
    let fp = parts.next()?;
    let kind = pick.rsplit(' ').next().unwrap_or("");
    Some(format!("{fp} {kind}"))
}

pub const DEFAULT_ZOOM: f32 = 1.25;
const MIN_ZOOM: f32 = 0.5;
const MAX_ZOOM: f32 = 3.0;
const ZOOM_STEPS: [f32; 13] = [0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

fn zoom_file() -> PathBuf {
    local::config_dir().join("zoom")
}

fn load_zoom() -> f32 {
    std::fs::read_to_string(zoom_file())
        .ok()
        .and_then(|t| t.trim().parse::<f32>().ok())
        .filter(|z| z.is_finite())
        .map_or(DEFAULT_ZOOM, |z| z.clamp(MIN_ZOOM, MAX_ZOOM))
}

fn pinned_file() -> PathBuf {
    local::config_dir().join("pinned")
}

fn load_pinned() -> Vec<String> {
    std::fs::read_to_string(pinned_file()).map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()).unwrap_or_default()
}

fn save_pinned(ids: &[String]) {
    let text: String = ids.iter().map(|id| format!("{id}\n")).collect();
    std::thread::spawn(move || {
        let f = pinned_file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, text);
    });
}

fn recent_file() -> PathBuf {
    local::config_dir().join("servers")
}

fn load_recent() -> Vec<String> {
    std::fs::read_to_string(recent_file()).map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()).unwrap_or_default()
}

/// Remembers servers connected to by address (never passwords), off the UI thread.
fn save_recent(hosts: &[Host]) {
    let text: String = hosts.iter().filter(|h| !h.from_config).map(|h| format!("{}\n", h.id)).collect();
    std::thread::spawn(move || {
        let f = recent_file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, text);
    });
}

fn places() -> Vec<Place> {
    let home = local::home();
    let mut v = vec![("Home", &crate::icons::HOME, home.clone())];
    for (key, fallback, label, icon) in [
        ("DESKTOP", "Desktop", "Desktop", &crate::icons::DESKTOP),
        ("DOCUMENTS", "Documents", "Documents", &crate::icons::DOCUMENTS),
        ("DOWNLOAD", "Downloads", "Downloads", &crate::icons::DOWNLOADS),
        ("PICTURES", "Pictures", "Pictures", &crate::icons::PICTURES),
        ("MUSIC", "Music", "Music", &crate::icons::MUSIC),
        ("VIDEOS", "Videos", "Videos", &crate::icons::VIDEOS),
    ] {
        let p = local::user_dir(key, fallback);
        if p != home && p.is_dir() {
            v.push((label, icon, p));
        }
    }
    v.push(("File System", &crate::icons::DRIVE, PathBuf::from("/")));
    v
}
