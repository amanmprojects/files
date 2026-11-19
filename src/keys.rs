//! Every keyboard shortcut. Keys the focused control takes first (a text field's editing, the
//! list's arrows, Home/End, Page Up/Down, Shift-extend, Ctrl+A and Enter) arrive here only when
//! it passes them on.

use rune::{Key, KeyEvent, Tx};

use crate::app::{App, Dialog};
use crate::ops::Mode;

/// What the shortcuts dialog (F1) lists, by section.
pub const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigate",
        &[
            ("Enter, Ctrl+O, Alt+↓", "Open (archives open as folders)"),
            ("Backspace, Alt+↑", "Enclosing folder"),
            ("Alt+← / Alt+→", "Back / Forward"),
            ("Mouse back / forward", "Back / Forward"),
            ("Two-finger swipe right / left", "Back / Forward (trackpad)"),
            ("Alt+Home", "Home folder"),
            ("Ctrl+L, Alt+D, F6", "Type a location"),
            ("/ or ~", "Type a location starting there"),
            ("F5, Ctrl+R", "Reload"),
        ],
    ),
    (
        "Select",
        &[
            ("Arrows, Home, End, Page Up/Down", "Move"),
            ("Shift + those, Shift+click", "Extend the selection"),
            ("Ctrl+click", "Add or remove one"),
            ("Drag across empty space", "Select with a box (Ctrl adds)"),
            ("Ctrl+A", "Select all"),
            ("Type a name", "Jump to it"),
            ("Ctrl+F", "Filter this folder"),
            ("Escape", "Clear the filter, clipboard or selection"),
        ],
    ),
    (
        "Files",
        &[
            ("Ctrl+C / Ctrl+X / Ctrl+V", "Copy / Cut / Paste (scp between machines)"),
            ("Drag onto a folder, place, server, tab or breadcrumb", "Move there (Ctrl copies; between machines it copies)"),
            ("Ctrl+D", "Duplicate"),
            ("Ctrl+E", "Extract the selected archives here"),
            ("F2", "Rename"),
            ("Delete", "Move to Trash (delete on a server)"),
            ("Shift+Delete", "Delete permanently"),
            ("Ctrl+Shift+N", "New folder"),
            ("Ctrl+Alt+N", "New empty file"),
            ("Ctrl+Shift+C", "Copy path"),
            ("Alt+Enter, Ctrl+I", "Properties"),
            ("Space", "Quick Look (↑ ↓ step through files, Space closes)"),
            ("Menu, Shift+F10", "Context menu"),
        ],
    ),
    (
        "Servers",
        &[
            ("Ctrl+K", "Connect to a server"),
            ("Ctrl+Shift+D", "Download (or extract from an archive) to Downloads"),
            ("Ctrl+U", "Upload files here"),
            ("Ctrl+Shift+K", "Disconnect from this server"),
        ],
    ),
    (
        "View",
        &[
            ("Ctrl+= / Ctrl+- / Ctrl+0", "Zoom in / out / reset"),
            ("Ctrl+scroll", "Zoom"),
            ("Ctrl+G", "Grid or list view"),
            ("Ctrl+Alt+= / Ctrl+Alt+-", "Bigger / smaller grid icons"),
            ("Ctrl+H, Alt+.", "Show hidden files"),
            ("Ctrl+1 / 2 / 3 / 4", "Sort by name / modified / size / kind (again flips)"),
            ("Ctrl+B, F9", "Sidebar"),
            ("Ctrl+Alt+B", "Right sidebar (preview or transfers)"),
            ("Alt+P", "Preview pane"),
            ("Ctrl+J", "Transfers"),
        ],
    ),
    (
        "Tabs",
        &[
            ("Ctrl+N", "New tab here"),
            ("Ctrl+Enter", "Open the selected folders in new tabs"),
            ("Ctrl+W", "Close the tab (the last one closes the window)"),
            ("Ctrl+Shift+T", "Reopen the last closed tab"),
            ("Ctrl+Tab / Ctrl+Shift+Tab", "Next / previous tab"),
            ("Ctrl+Page Down / Up", "Next / previous tab"),
            ("Alt+1 … Alt+8, Alt+9", "Go to tab 1 … 8, the last tab"),
            ("Middle-click a tab", "Close it"),
        ],
    ),
    (
        "Window",
        &[
            ("Ctrl+T", "Terminal panel: open and focus; from it, hide"),
            ("Ctrl+Shift+V (in the terminal)", "Paste"),
            ("Shift+Page Up / Down (in the terminal)", "Scroll back"),
            ("F4, Ctrl+Alt+T", "Terminal window here"),
            ("Ctrl+Q", "Quit"),
            ("F1, Ctrl+/", "Keyboard shortcuts"),
        ],
    ),
];

pub fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let m = ev.mods;
    // The character of a Ctrl chord. Ctrl+letter arrives as `Command`; Ctrl+Alt+letter arrives
    // as `Text` (Linux treats Ctrl+Alt as AltGr), so match both.
    let ch = match &ev.key {
        Key::Command(c) => Some(c.to_ascii_lowercase()),
        Key::Text(t) if m.cmd && t.chars().count() == 1 => t.chars().next().map(|c| c.to_ascii_lowercase()),
        _ => None,
    };
    let cmd = |c: char| m.cmd && !m.shift && !m.alt && ch == Some(c);
    let cmd_shift = |c: char| m.cmd && m.shift && !m.alt && ch == Some(c);
    let cmd_alt = |c: char| m.cmd && m.alt && !m.shift && ch == Some(c);
    let plain = !m.cmd && !m.ctrl && !m.alt && !m.logo;

    // Grid icon size: Ctrl+Alt with the zoom keys, so checked before them.
    if m.cmd && m.alt && matches!(ch, Some('=' | '+' | '-' | '_')) {
        s.grid_zoom(if matches!(ch, Some('-' | '_')) { -1 } else { 1 });
        return true;
    }
    // These work everywhere, dialogs and text fields included.
    if ev.is_command('=') || ev.is_command('+') {
        s.zoom_step(1);
        return true;
    }
    if ev.is_command('-') || ev.is_command('_') {
        s.zoom_step(-1);
        return true;
    }
    if cmd('0') {
        s.zoom_step(0);
        return true;
    }
    if cmd('q') {
        tx.quit();
        return true;
    }
    if cmd('w') {
        // Closes the tab; the last tab closes the window.
        if !s.close_tab(s.active) {
            tx.close_window();
        }
        return true;
    }
    if ev.key == Key::Function(1) || ev.is_command('/') || ev.is_command('?') {
        s.dialog = Some(Dialog::Shortcuts);
        return true;
    }
    // Quick Look: Space opens and closes it; ↑ ↓ step through the folder under it.
    if matches!(s.dialog, Some(Dialog::QuickLook)) {
        match &ev.key {
            Key::Text(t) if t == " " && plain => s.toggle_quick_look(),
            Key::Up | Key::Left if !m.alt => s.step_selection(-1, tx),
            Key::Down | Key::Right if !m.alt => s.step_selection(1, tx),
            _ => return false,
        }
        return true;
    }
    if s.dialog.is_some() {
        return false;
    }

    // In the table or with nothing focused, a typed character jumps to a name.
    let in_list = tx.focused().is_none_or(|f| f == "files");

    match &ev.key {
        Key::Text(t) if plain && in_list && (t == "/" || t == "~") => {
            let mut e = rune::TextEdit::new(t.clone());
            e.focused = true;
            e.anchor = t.len();
            e.focus = t.len();
            s.path_edit = Some(e);
        }
        Key::Text(t) if m.alt && !m.cmd && t == "." => s.toggle_hidden(),
        Key::Text(t) if m.alt && !m.cmd && t.len() == 1 && t.as_bytes()[0].is_ascii_digit() && t != "0" => {
            let n = (t.as_bytes()[0] - b'1') as usize;
            // Alt+9 is the last tab, as in browsers.
            s.switch_tab(if t == "9" { s.tabs.len() - 1 } else { n });
        }
        Key::Text(t) if m.alt && !m.cmd && t == "p" => s.toggle_side(crate::app::Side::Preview),
        Key::Text(t) if plain && in_list && t == " " => s.toggle_quick_look(),
        Key::Text(t) if plain && in_list && !t.trim().is_empty() => return s.type_select(t, tx),
        Key::Enter if m.alt => s.show_properties(),
        // The table opens its cursor row itself; this covers a selection it isn't focused on.
        Key::Enter if plain && in_list => s.open_selection(),
        Key::Down if m.alt => s.open_selection(),
        Key::Up if m.alt => s.go_up(),
        Key::Left if m.alt => s.go_back(),
        Key::Right if m.alt => s.go_forward(),
        Key::Home if m.alt => s.go(crate::model::Location::Local(crate::local::home())),
        Key::Backspace if in_list && plain => s.go_up(),
        Key::Delete => s.ask_delete(m.shift),
        Key::ContextMenu => s.menu_from_keyboard(),
        Key::Function(10) if m.shift => s.menu_from_keyboard(),
        Key::Function(2) => s.start_rename(),
        Key::Function(4) => s.open_terminal(),
        Key::Function(5) => s.refresh(),
        Key::Function(6) => crate::view::start_path_edit(s),
        Key::Function(9) => s.sidebar.toggle(),
        Key::Tab if m.cmd && !m.shift => s.cycle_tab(1),
        Key::Tab if m.cmd && m.shift => s.cycle_tab(-1),
        Key::PageDown if m.cmd => s.cycle_tab(1),
        Key::PageUp if m.cmd => s.cycle_tab(-1),
        Key::Enter if m.cmd && !m.shift && !m.alt => s.open_selection_in_tabs(),
        Key::Escape => {
            if s.menu.is_some() {
                s.menu = None;
            } else if !s.filter.text.is_empty() {
                s.filter = rune::TextEdit::new("");
                s.refilter();
            } else if s.clip.is_some() {
                s.clip = None;
            } else {
                s.clear_selection();
            }
        }
        _ if rune_kit::shell::is_sidebar_toggle(ev) => s.sidebar.toggle(),
        _ if cmd('c') => s.copy(Mode::Copy, tx),
        _ if cmd('x') => s.copy(Mode::Move, tx),
        _ if cmd('v') => s.paste_clipboard(tx),
        _ if cmd('d') => s.duplicate(),
        _ if cmd('o') => s.open_selection(),
        _ if cmd('i') => s.show_properties(),
        _ if cmd('h') => s.toggle_hidden(),
        _ if cmd('g') => s.toggle_view(),
        _ if cmd('l') => crate::view::start_path_edit(s),
        _ if cmd('r') => s.refresh(),
        _ if cmd('e') => s.extract_here(),
        _ if cmd('k') => s.open_connect_dialog("", None),
        _ if cmd('n') => s.new_tab(None),
        _ if cmd('b') => s.sidebar.toggle(),
        _ if cmd('t') => s.toggle_terminal(tx),
        _ if cmd_shift('t') => s.reopen_tab(),
        _ if cmd('j') => s.toggle_side(crate::app::Side::Transfers),
        _ if cmd('u') => {
            if s.loc.is_remote() {
                crate::view::pick_upload(s, tx);
            }
        }
        _ if cmd('f') => {
            // Shows the filter on a tight toolbar too, then focuses it.
            s.filter.focused = true;
            tx.focus("filter", true);
        }
        _ if cmd('1') => s.sort_by("name"),
        _ if cmd('2') => s.sort_by("modified"),
        _ if cmd('3') => s.sort_by("size"),
        _ if cmd('4') => s.sort_by("kind"),
        _ if cmd_shift('n') => s.start_new_folder(),
        _ if cmd_shift('c') => s.copy_path(tx),
        _ if cmd_shift('d') => {
            if s.loc.is_remote() || s.read_only() {
                s.download_selection();
            }
        }
        _ if cmd_shift('k') => {
            if let Some(h) = s.loc.host().map(str::to_owned) {
                s.disconnect(&h);
            }
        }
        _ if cmd_alt('n') => s.start_new_file(),
        _ if cmd_alt('b') => s.activity.toggle(),
        _ if cmd_alt('t') => s.open_terminal(),
        _ if m.alt && !m.cmd && ev.key == Key::Text("d".into()) => crate::view::start_path_edit(s),
        _ => return false,
    }
    true
}
