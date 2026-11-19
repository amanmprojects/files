//! The folder's contents as a sortable table, its empty and error states, and its menus.

use std::borrow::Cow;

use rune::{Align, El, Icon, PointerPress, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, Cell, Kit, Menu, MenuItem, Table, TableColumn, glyphs};

use crate::app::{App, MenuFor};
use crate::icons;
use crate::model::{Entry, Kind, extension, format_time, human_size, kind_label, mode_string};
use crate::ops::Mode;

fn row_id(s: &App, i: usize) -> Cow<'_, str> {
    Cow::Borrowed(s.entries[s.shown[i]].name.as_str())
}

pub fn file_icon(e: &Entry) -> &'static Icon {
    if e.is_dir() {
        return &glyphs::FOLDER;
    }
    match extension(&e.name).as_deref() {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" | "avif" | "heic" | "tiff") => &glyphs::FILE_IMAGE,
        Some(
            "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "c" | "h" | "cpp" | "hpp" | "java" | "kt" | "rb" | "php" | "sh" | "bash" | "zsh" | "fish"
            | "lua" | "swift" | "cs" | "html" | "css" | "scss" | "json" | "toml" | "yaml" | "yml" | "xml" | "sql" | "zig",
        ) => &glyphs::FILE_CODE,
        Some("zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "zst" | "7z" | "rar" | "deb" | "rpm" | "iso") => &icons::ARCHIVE,
        Some("txt" | "md" | "log" | "csv" | "pdf" | "doc" | "docx" | "odt" | "rtf" | "conf" | "ini" | "cfg") => &glyphs::FILE_TEXT,
        _ => &glyphs::FILE,
    }
}

/// The table, or what stands in for it (an error, an empty folder).
/// `avail`: the file area's height last frame, to fit a short table to its rows.
pub fn content(s: &App, k: &Kit, avail: Option<f32>) -> El {
    let frame = col()
        .key("content")
        .w_full()
        .min_h(0.0)
        .fill(k.c.bg_base)
        // Space for Quick Look: focus navigation would take it as a press before the window's
        // keys see it, but an ancestor of the focused table hears it first.
        .on_key(|s: &mut App, ev: &rune::KeyEvent, _tx: &mut Tx| {
            let m = ev.mods;
            if ev.key == rune::Key::Text(" ".into()) && !m.cmd && !m.ctrl && !m.alt {
                s.toggle_quick_look();
                return true;
            }
            false
        })
        .on_layout_tx(|s: &mut App, r: [f32; 4], tx: &mut Tx| {
            // Bookkeeping for keyboard-opened menus; nothing on screen depends on it.
            s.content_at = r;
            tx.live_only();
        })
        .on_secondary_press(|s: &mut App, p: PointerPress, _tx: &mut Tx| {
            s.table.clear_selection();
            s.menu = Some((p, MenuFor::Background));
        });
    if let Some(err) = &s.error {
        return frame
            .center()
            .gap(12.0)
            .padding(24.0)
            .child(text(err.clone()).font(k.ty.body).color(k.c.text_muted).wrap().align(Align::Center).max_w(520.0))
            .child(
                row()
                    .gap(8.0)
                    .child(Button::new("Try Again").variant(ButtonVariant::Outline).on_press(|s: &mut App| s.refresh()).build(k).key("retry"))
                    .child(Button::new("Go Back").variant(ButtonVariant::Ghost).on_press(|s: &mut App| s.go_back()).build(k).key("error-back")),
            );
    }
    if s.shown.is_empty() && !s.loading {
        let msg = if !s.filter.text.is_empty() {
            "No items match the filter"
        } else if !s.entries.is_empty() {
            "Only hidden items here (Ctrl+H shows them)"
        } else {
            "This folder is empty"
        };
        return frame.center().child(text(msg).font(k.ty.body).color(k.c.text_faint));
    }

    let kit = k.clone();
    let table = Table::new(|s: &App| &s.table, |s: &mut App| &mut s.table, |s: &App| s.shown.len(), row_id)
        .name("Files")
        .column(TableColumn::new("name", "Name").flex(3.0).min(180.0).sortable())
        .column(TableColumn::new("modified", "Modified").width(170.0).min(120.0).max(240.0).sortable())
        .column(TableColumn::new("size", "Size").width(90.0).min(64.0).max(160.0).align(Align::End).sortable())
        .column(TableColumn::new("kind", "Kind").width(120.0).min(80.0).max(220.0).sortable())
        .cell(move |s: &App, i, col| cell(s, &kit, i, &s.entries[s.shown[i]], col))
        .on_sort(|s: &mut App, col, dir, _| s.set_sort(col, dir))
        .on_open(|s: &mut App, id, _| {
            let id = id.to_owned();
            s.open(&id);
        })
        .build(s, k);
    // A table shorter than the area ends at its last row, so the space below is ours: inside
    // the table it would be its scroll view, which swipes and clicks never reach past.
    let style = rune_kit::TableStyle::from_kit(k);
    let needed = style.header_height + s.shown.len() as f32 * style.row_height + 2.0;
    match avail {
        Some(h) if needed < h => frame.child(table.h(needed)).child(
            rune::el().key("below-rows").w_full().grow(1.0).on_wheel(|s: &mut App, e, tx| s.swipe(&e, tx)).on_press(|s: &mut App| s.table.clear_selection()),
        ),
        _ => frame.child(table.grow(1.0).min_h(0.0)),
    }
}

fn cell(s: &App, k: &Kit, i: usize, e: &Entry, col: &str) -> Cell {
    let cut = s.clip.as_ref().is_some_and(|c| c.mode == Mode::Move && c.dir == s.loc && c.names.contains(&e.name));
    let ink = if cut { k.c.text_faint } else { k.c.text_base };
    let content = match col {
        "name" => {
            let glyph = if e.is_dir() { k.c.icon_accent } else { k.c.icon_muted };
            let mut r = row()
                .gap(8.0)
                .center_y()
                .min_w(0.0)
                .child(rune::icon(file_icon(e), 16.0).color(if cut { k.c.icon_faint } else { glyph }))
                .child(text(e.name.clone()).font(k.ty.ui).color(ink).truncate().shrink());
            if e.link {
                r = r.child(rune::icon(&icons::LINK, 12.0).color(k.c.icon_faint).help(if e.kind == Kind::Other { "Broken link" } else { "Link" }));
            }
            r
        }
        "modified" => {
            row().child(text(e.modified.map(|t| format_time(t, s.now, s.utc_offset)).unwrap_or_default()).font(k.ty.ui).color(k.c.text_muted).truncate())
        }
        "size" => row()
            .justify_end()
            .child(text(e.size.map(human_size).unwrap_or_else(|| if e.is_dir() { "—".into() } else { String::new() })).font(k.ty.mono).color(k.c.text_muted)),
        _ => {
            let label = kind_label(e);
            let t = text(label).font(k.ty.ui).color(k.c.text_muted).truncate();
            match e.mode {
                Some(m) => row().child(t.help(mode_string(m))),
                None => row().child(t),
            }
        }
    };
    // The name cell reports where its row is drawn, which says how far the list is scrolled
    // (for putting it back on Back). Bookkeeping only: nothing on screen changes.
    let content = if col == "name" {
        let style = rune_kit::TableStyle::from_kit(k);
        let (header_h, row_h) = (style.header_height, style.row_height);
        content.on_layout_tx(move |s: &mut App, r: [f32; 4], tx: &mut Tx| {
            s.track_scroll(i, r[1], header_h, row_h);
            tx.live_only();
        })
    } else {
        content
    };
    let name = e.name.clone();
    // Rows sit inside the list's scroll view, which a sideways swipe never reaches past.
    Cell::el(content.w_full().h_full().center_y().on_wheel(|s: &mut App, e, tx| s.swipe(&e, tx)).on_secondary_press(
        move |s: &mut App, p: PointerPress, _tx: &mut Tx| {
            if !s.table.is_selected(&name) {
                s.table.select(name.clone());
            }
            s.menu = Some((p, MenuFor::Items));
        },
    ))
}

fn close_then(f: impl Fn(&mut App) + 'static) -> impl Fn(&mut App) + 'static {
    move |s: &mut App| {
        s.menu = None;
        f(s);
    }
}

fn close_then_tx(f: impl Fn(&mut App, &mut Tx) + 'static) -> impl Fn(&mut App, &mut Tx) + 'static {
    move |s: &mut App, tx: &mut Tx| {
        s.menu = None;
        f(s, tx);
    }
}

pub fn item_menu(s: &App, menu: Menu) -> Menu {
    let sel = s.selection();
    let one = sel.len() == 1;
    let dir = one && s.entry(&sel[0]).is_some_and(Entry::is_dir);
    let archives = sel.iter().any(|n| crate::archive::is_archive_name(n) && s.entry(n).is_some_and(|e| !e.is_dir()));
    let inside = s.read_only();
    let remote = s.loc.is_remote();
    let open = if dir {
        "Open"
    } else if one && archives {
        "Browse Archive"
    } else if inside {
        "Open (extract a copy)"
    } else if remote {
        "Open (download a copy)"
    } else {
        "Open"
    };
    let mut m = menu.item(MenuItem::new(open).shortcut("Enter").on_select(close_then(|s: &mut App| s.open_selection())));
    if sel.iter().any(|n| s.entry(n).is_some_and(Entry::is_dir)) {
        m = m.item(MenuItem::new("Open in New Tab").shortcut("Ctrl+Enter").on_select(close_then(|s: &mut App| s.open_selection_in_tabs())));
    }
    if one && archives {
        m = m.item(MenuItem::new("Open With Default App").on_select(close_then(|s: &mut App| {
            if let Some(n) = s.selection().first().cloned() {
                s.open_external(&n);
            }
        })));
    }
    if archives && !inside {
        m = m.item(MenuItem::new("Extract Here").icon(&icons::ARCHIVE).shortcut("Ctrl+E").on_select(close_then(|s: &mut App| s.extract_here())));
    }
    if remote || inside {
        let label = if inside { "Extract to Downloads" } else { "Download to Downloads" };
        m = m.item(MenuItem::new(label).icon(&icons::DOWNLOADS).shortcut("Ctrl+Shift+D").on_select(close_then(|s: &mut App| s.download_selection())));
    }
    m = m.separator();
    if !inside {
        m = m.item(MenuItem::new("Cut").shortcut("Ctrl+X").on_select_tx(close_then_tx(|s: &mut App, tx: &mut Tx| s.copy(Mode::Move, tx))));
    }
    m = m.item(MenuItem::new("Copy").shortcut("Ctrl+C").on_select_tx(close_then_tx(|s: &mut App, tx: &mut Tx| s.copy(Mode::Copy, tx))));
    if inside {
        return m
            .separator()
            .item(MenuItem::new("Copy Path").shortcut("Ctrl+Shift+C").on_select_tx(close_then_tx(|s: &mut App, tx: &mut Tx| s.copy_path(tx))))
            .item(MenuItem::new("Properties").shortcut("Alt+Enter").on_select(close_then(|s: &mut App| s.show_properties())));
    }
    m = m
        .item(MenuItem::new("Paste").shortcut("Ctrl+V").disabled(s.clip.is_none()).on_select(close_then(|s: &mut App| s.paste())))
        .separator()
        .item(MenuItem::new("Duplicate").shortcut("Ctrl+D").on_select(close_then(|s: &mut App| s.duplicate())))
        .item(MenuItem::new("Rename…").shortcut("F2").disabled(!one).on_select(close_then(|s: &mut App| s.start_rename())))
        .item(MenuItem::new("Copy Path").shortcut("Ctrl+Shift+C").on_select_tx(close_then_tx(|s: &mut App, tx: &mut Tx| s.copy_path(tx))))
        .item(MenuItem::new("Properties").shortcut("Alt+Enter").on_select(close_then(|s: &mut App| s.show_properties())))
        .separator();
    let trash = !remote && crate::local::has_program("gio");
    m.item(
        MenuItem::new(if trash { "Move to Trash" } else { "Delete…" })
            .icon(&icons::TRASH)
            .shortcut("Delete")
            .danger(true)
            .on_select(close_then(|s: &mut App| s.ask_delete(false))),
    )
}

pub fn background_menu(s: &App, menu: Menu) -> Menu {
    let inside = s.read_only();
    menu.item(
        MenuItem::new("New Folder…")
            .disabled(inside)
            .icon(&icons::FOLDER_PLUS)
            .shortcut("Ctrl+Shift+N")
            .on_select(close_then(|s: &mut App| s.start_new_folder())),
    )
    .item(MenuItem::new("New File…").disabled(inside).shortcut("Ctrl+Alt+N").on_select(close_then(|s: &mut App| s.start_new_file())))
    .item(MenuItem::new("Paste").shortcut("Ctrl+V").disabled(s.clip.is_none() || inside).on_select(close_then(|s: &mut App| s.paste())))
    .item(
        MenuItem::new("Upload Files…").icon(&icons::UPLOAD).shortcut("Ctrl+U").disabled(!s.loc.is_remote() || inside).on_select_tx(close_then_tx(pick_upload)),
    )
    .separator()
    .item(MenuItem::new("Show Hidden Files").checked(s.show_hidden).shortcut("Ctrl+H").on_select(close_then(|s: &mut App| s.toggle_hidden())))
    .item(MenuItem::new("Reload").shortcut("F5").on_select(close_then(|s: &mut App| s.refresh())))
    .item(MenuItem::new("Copy Path").shortcut("Ctrl+Shift+C").on_select_tx(close_then_tx(|s: &mut App, tx: &mut Tx| s.copy_path(tx))))
    .item(MenuItem::new("Open Terminal Here").icon(&icons::TERMINAL).shortcut("F4").on_select(close_then(|s: &mut App| s.open_terminal())))
    .item(MenuItem::new("New Tab").shortcut("Ctrl+N").on_select(close_then(|s: &mut App| s.new_tab(None))))
    .item(MenuItem::new("New Window").on_select(close_then(|s: &mut App| s.new_window())))
    .item(MenuItem::new("Clear Download Cache").on_select(close_then(|s: &mut App| s.clear_cache())))
    .separator()
    .item(MenuItem::new("Keyboard Shortcuts").shortcut("F1").on_select(close_then(|s: &mut App| s.dialog = Some(crate::app::Dialog::Shortcuts))))
}

/// Picks a local file with the desktop's chooser and uploads it here.
pub fn pick_upload(_: &mut App, tx: &mut Tx) {
    let options = rune::FileDialogOptions { title: "Upload to this folder".into(), ..Default::default() };
    tx.open_file(options, |s: &mut App, result, _tx| match result {
        Ok(Some(path)) => s.upload_paths(&[path]),
        Ok(None) => {}
        Err(e) => s.fail(format!("Couldn't open the file chooser: {e}")),
    });
}
