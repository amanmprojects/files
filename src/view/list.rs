//! The folder's contents as sortable rows (or, through `grid`, tiles), its empty and error
//! states, and its menus.

use std::borrow::Cow;

use rune::{Align, El, GrowthSpec, Icon, PointerPress, ScrollSpec, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, Kit, Menu, MenuItem, TableStyle, glyphs};

use crate::app::{App, GRID_SIZE_NAMES, MenuFor, ViewMode};
use crate::icons;
use crate::model::{Entry, Kind, extension, format_time, human_size, kind_label, mode_string};
use crate::ops::Mode;

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

/// A column: id, title, width (None: the name, which takes the rest), bounds, alignment.
type Column = (&'static str, &'static str, Option<f32>, f32, f32, Align);

const COLUMNS: [Column; 4] = [
    ("name", "Name", None, 180.0, f32::INFINITY, Align::Start),
    ("modified", "Modified", Some(170.0), 120.0, 260.0, Align::Start),
    ("size", "Size", Some(90.0), 64.0, 160.0, Align::End),
    ("kind", "Kind", Some(120.0), 80.0, 240.0, Align::Start),
];

/// A column's width: as the user dragged it, else its own.
fn col_width(s: &App, id: &'static str, default: f32, min: f32, max: f32) -> f32 {
    s.col_widths.get(id).copied().unwrap_or(default).clamp(min, max)
}

/// The list or grid, or what stands in for it (an error, an empty folder).
/// `size`: the file area's size last frame, to lay out the grid.
pub fn content(s: &App, k: &Kit, size: Option<[f32; 2]>) -> El {
    let frame = col()
        .key("content")
        .w_full()
        .min_h(0.0)
        .fill(k.c.bg_base)
        // Space for Quick Look: focus navigation would take it as a press before the window's
        // keys see it, but an ancestor of the focused list hears it first.
        .on_key(|s: &mut App, ev: &rune::KeyEvent, _tx: &mut Tx| {
            let m = ev.mods;
            if ev.key == rune::Key::Text(" ".into()) && !m.cmd && !m.ctrl && !m.alt {
                s.toggle_quick_look();
                return true;
            }
            false
        })
        .on_layout_tx(|s: &mut App, r: [f32; 4], tx: &mut Tx| {
            // Bookkeeping for keyboard-opened menus and the swipe arrow.
            s.content_at = r;
            tx.live_only();
        })
        .on_secondary_press(|s: &mut App, p: PointerPress, _tx: &mut Tx| {
            s.clear_selection();
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
    if s.view_mode == ViewMode::Grid {
        return super::grid::content(s, k, frame, size);
    }

    let style = TableStyle::from_kit(k);
    let kit = k.clone();
    let row_h = style.row_height;
    let rows = col()
        .key("rows")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(s.shown.len(), row_h, |s: &App, i| row_id(s, i), move |s: &App, i| list_row(s, &kit, i))
        .on_visible_rows(|s: &mut App, r| s.grid_visible(r));
    let scroll = s.grid_scroll.clone();
    let viewport = col()
        .key("viewport")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .on_scroll_offset(move |at| scroll.set(at.offset[1]))
        .on_layout_tx(move |s: &mut App, r: [f32; 4], tx: &mut Tx| {
            if !s.grid_layout(1, (r[3] / row_h).floor() as usize) {
                tx.live_only();
            }
        })
        // Sideways swipes anywhere in the list; a drag on the empty part draws a selection
        // box, and a click there clears the selection.
        .on_wheel(|s: &mut App, e, tx| s.swipe(&e, tx))
        .on_drag(|s: &mut App, d: &mut rune::Drag, tx: &mut Tx| s.band_drag(d, tx))
        .child(rows);
    let mut root = col()
        .key("files")
        .role("grid")
        .label("Files")
        .focusable(true)
        // The selected rows and the cursor's outline show focus.
        .no_focus_ring()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .clip_soft(0.0)
        .on_key(|s: &mut App, ev: &rune::KeyEvent, tx: &mut Tx| s.nav_key(ev, tx));
    if let Some(c) = &s.sel.cursor {
        root = root.active_descendant(c.clone());
    }
    frame.child(root.child(header(s, k, &style)).child(viewport))
}

/// The column titles: a press sorts by one (again flips it); a column's right edge drags to
/// widen it, and a double click there puts it back.
fn header(s: &App, k: &Kit, style: &TableStyle) -> El {
    let sort = s.sort;
    let mut h = row()
        .key("header")
        .role("row")
        .w_full()
        .h(style.header_height)
        .child(rune::el().key("rule").pin(Some(0.0), None, Some(0.0), Some(0.0)).h(1.0).fill(style.rule).a11y_hidden());
    for (j, (id, title, width, min, max, align)) in COLUMNS.into_iter().enumerate() {
        let sorted = (crate::model::SortKey::from_column(id) == sort.0).then_some(sort.1);
        let mut inner = row()
            .gap(2.0)
            .center_y()
            .min_w(0.0)
            .shrink()
            .child(text(title).font(style.header_font).color(if sorted.is_some() { style.text } else { style.header_text }).truncate().shrink().min_w(0.0));
        if let Some(asc) = sorted {
            inner = inner.child(rune::icon(if asc { k.glyph.chevron_up } else { k.glyph.chevron_down }, 14.0).color(style.text));
        }
        let mut cell = row()
            .key(id)
            .role("columnheader")
            .label(title)
            .sorted(sorted)
            .h_full()
            .px(style.cell_pad)
            .center_y()
            .cursor(rune::Cursor::Pointer)
            .hover(k.c.hover)
            .on_press(move |s: &mut App| s.sort_by(id))
            .child(inner);
        cell = if align == Align::End { cell.justify_end() } else { cell };
        cell = match width {
            None => cell.w(0.0).grow(1.0).shrink().min_w(min),
            Some(w) => cell.w(col_width(s, id, w, min, max)),
        };
        if let Some(w) = width {
            // The edge between this column and the one before it: dragging it right widens
            // this one (the name column takes up the difference).
            let handle = rune::el()
                .key("resize")
                .role("separator")
                .label(format!("Resize {title}"))
                .pin(Some(-4.0), Some(6.0), None, Some(6.0))
                .w(9.0)
                .center()
                .cursor(rune::Cursor::ResizeColumn)
                .child(rune::el().w(1.0).h_full().fill(style.rule))
                .on_drag(move |s: &mut App, d: &mut rune::Drag, _tx: &mut Tx| {
                    if d.phase == rune::Phase::Down {
                        d.data = col_width(s, id, w, min, max);
                    }
                    if d.cancelled() {
                        s.col_widths.remove(id);
                    } else {
                        s.col_widths.insert(id, (d.data - d.dx).clamp(min, max));
                    }
                })
                .on_double_press(move |s: &mut App| {
                    s.col_widths.remove(id);
                });
            cell = cell.child(handle);
        } else if j + 1 < COLUMNS.len() {
            cell = cell.child(rune::el().pin(None, Some(8.0), Some(0.0), Some(8.0)).w(1.0).fill(style.rule).a11y_hidden());
        }
        h = h.child(cell);
    }
    h
}

fn row_id(s: &App, i: usize) -> Cow<'_, str> {
    match s.shown.get(i) {
        Some(&e) => Cow::Borrowed(s.entries[e].name.as_str()),
        None => Cow::Owned(format!("row-{i}")),
    }
}

/// Row `i`: its cells, and what pressing, dragging and right-clicking it does.
fn list_row(s: &App, k: &Kit, i: usize) -> El {
    let Some(&at) = s.shown.get(i) else { return rune::el() };
    let e = &s.entries[at];
    let style = TableStyle::from_kit(k);
    let selected = s.is_selected(&e.name);
    let drop_here = s.file_drag.as_ref().and_then(|d| d.target.as_ref()).is_some_and(|t| t.key.strip_prefix("item:") == Some(e.name.as_str()));
    let mut r = row().role("row").label(e.name.clone()).focus_selected(selected).w_full().h(style.row_height).center_y();
    r = if drop_here {
        r.fill(k.c.icon_accent.alpha(0.25)).outline(2.0, -2.0, k.c.icon_accent)
    } else if selected {
        r.fill(style.selected)
    } else {
        r.hover(style.stripe.mix(style.selected, 0.5))
    };
    if selected && s.sel.selected.len() > 1 && s.sel.cursor.as_deref() == Some(e.name.as_str()) {
        r = r.outline(1.0, -1.0, k.c.border_focus);
    }
    for (id, _, width, min, max, align) in COLUMNS {
        let c = row().key(id).role("gridcell").h_full().px(style.cell_pad).center_y().clip_soft(0.0).child(cell(s, k, e, id));
        let c = if align == Align::End { c.justify_end() } else { c };
        r = r.child(match width {
            None => c.w(0.0).grow(1.0).shrink().min_w(min),
            Some(w) => c.w(col_width(s, id, w, min, max)),
        });
    }
    let (name, open, menu, dragged, rect) = (e.name.clone(), e.name.clone(), e.name.clone(), e.name.clone(), e.name.clone());
    r.on_press_tx(move |s: &mut App, tx: &mut Tx| {
        let m = tx.mods();
        s.click_item(&name, m.cmd || m.ctrl, m.shift);
        tx.focus("files", false);
    })
    .on_double_press(move |s: &mut App| s.open(&open))
    .on_drag(move |s: &mut App, d: &mut rune::Drag, tx: &mut Tx| {
        s.drag_item(&dragged, d, tx);
    })
    .on_secondary_press(move |s: &mut App, p: PointerPress, _tx: &mut Tx| {
        if !s.is_selected(&menu) {
            s.select_only(menu.clone());
        }
        s.menu = Some((p, MenuFor::Items));
    })
    .on_layout_tx(move |s: &mut App, r: [f32; 4], tx: &mut Tx| {
        s.tile_rects.insert(rect.clone(), r);
        tx.live_only();
    })
    // Rows sit inside the list's scroll view, which a sideways swipe never reaches past.
    .on_wheel(|s: &mut App, e, tx| s.swipe(&e, tx))
}

/// What row `e` shows in column `col`.
fn cell(s: &App, k: &Kit, e: &Entry, col: &str) -> El {
    let cut = s.clip.as_ref().is_some_and(|c| c.mode == Mode::Move && c.dir == s.loc && c.names.contains(&e.name));
    let ink = if cut { k.c.text_faint } else { k.c.text_base };
    match col {
        "name" => {
            let glyph = if e.is_dir() { k.c.icon_accent } else { k.c.icon_muted };
            let mut r = row()
                .gap(8.0)
                .center_y()
                .min_w(0.0)
                .shrink()
                .child(rune::icon(file_icon(e), 16.0).color(if cut { k.c.icon_faint } else { glyph }))
                .child(text(e.name.clone()).font(k.ty.ui).color(ink).truncate().shrink().help_when_truncated(e.name.clone()));
            if e.link {
                r = r.child(rune::icon(&icons::LINK, 12.0).color(k.c.icon_faint).help(if e.kind == Kind::Other { "Broken link" } else { "Link" }));
            }
            r
        }
        "modified" => text(e.modified.map(|t| format_time(t, s.now, s.utc_offset)).unwrap_or_default()).font(k.ty.ui).color(k.c.text_muted).truncate().shrink(),
        "size" => text(e.size.map(human_size).unwrap_or_else(|| if e.is_dir() { "—".into() } else { String::new() })).font(k.ty.mono).color(k.c.text_muted),
        _ => {
            let t = text(kind_label(e)).font(k.ty.ui).color(k.c.text_muted).truncate().shrink();
            match e.mode {
                Some(m) => t.help(mode_string(m)),
                None => t,
            }
        }
    }
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
    let menu = menu
        .item(
            MenuItem::new("New Folder…")
                .disabled(inside)
                .icon(&icons::FOLDER_PLUS)
                .shortcut("Ctrl+Shift+N")
                .on_select(close_then(|s: &mut App| s.start_new_folder())),
        )
        .item(MenuItem::new("New File…").disabled(inside).shortcut("Ctrl+Alt+N").on_select(close_then(|s: &mut App| s.start_new_file())))
        .item(MenuItem::new("Paste").shortcut("Ctrl+V").disabled(s.clip.is_none() || inside).on_select(close_then(|s: &mut App| s.paste())))
        .item(
            MenuItem::new("Upload Files…")
                .icon(&icons::UPLOAD)
                .shortcut("Ctrl+U")
                .disabled(!s.loc.is_remote() || inside)
                .on_select_tx(close_then_tx(pick_upload)),
        )
        .separator()
        .item(MenuItem::new("View as Grid").checked(s.view_mode == ViewMode::Grid).shortcut("Ctrl+G").on_select(close_then(|s: &mut App| s.toggle_view())));
    let menu = if s.view_mode == ViewMode::Grid {
        GRID_SIZE_NAMES.iter().enumerate().fold(menu, |m, (i, name)| {
            m.item(MenuItem::new(format!("{name} Icons")).checked(s.grid_size == i).on_select(close_then(move |s: &mut App| s.set_grid_size(i))))
        })
    } else {
        menu
    };
    menu.separator()
        .item(MenuItem::new("Show Hidden Files").checked(s.show_hidden).shortcut("Ctrl+H").on_select(close_then(|s: &mut App| s.toggle_hidden())))
        .item(MenuItem::new("Reload").shortcut("F5").on_select(close_then(|s: &mut App| s.refresh())))
        .item(MenuItem::new("Copy Path").shortcut("Ctrl+Shift+C").on_select_tx(close_then_tx(|s: &mut App, tx: &mut Tx| s.copy_path(tx))))
        .item(MenuItem::new("Open Terminal Here").icon(&icons::TERMINAL).shortcut("F4").on_select(close_then(|s: &mut App| s.open_terminal())))
        .item(MenuItem::new("New Tab").shortcut("Ctrl+T").on_select(close_then(|s: &mut App| s.new_tab(None))))
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
