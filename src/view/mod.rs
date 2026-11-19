//! The window: sidebar, toolbar, the file table, the transfers pane and the dialogs.

mod dialogs;
mod list;
mod preview;
mod sidebar;
mod terminal;
mod toolbar;
mod transfers;

pub use list::pick_upload;
pub use toolbar::start_path_edit;

use rune::{Cx, El, PointerButton, col, row, text};
use rune_kit::shell::{SidebarToggle, SplitView};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, Spinner, context_menu, glyphs};

use crate::app::{App, MenuFor, plural};
use crate::model::human_size;

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    cx.zoom(s.zoom);
    // Omarchy's theme when there is one (its background, text and accent), else neutral.
    let k = match &s.omarchy {
        Some(t) => Kit::from_palette(rune_kit::PaletteSeed { bg: t.bg, fg: t.fg, accent: t.accent }),
        None => Kit::neutral(cx.system_dark()),
    };
    let mut layer = Vec::new();

    let content = list::content(s, &k, cx.layout_size("content").map(|r| r[1]));
    let menu = context_menu("ctx", content, s.menu.map(|m| m.0)).min_width(230.0).on_close(|s: &mut App| s.menu = None);
    let menu = match s.menu.map(|m| m.1) {
        Some(MenuFor::Items) => list::item_menu(s, menu),
        Some(MenuFor::Background) => list::background_menu(s, menu),
        Some(MenuFor::Host) => sidebar::host_menu(s, menu),
        None => menu,
    };
    let content = menu.build(&k, &mut layer);

    let mut main = col().key("main").w_full().h_full().min_h(0.0);
    if s.tabs.len() > 1 {
        main = main.child(tab_strip(s, &k, cx.layout_size("tabs").map(|z| z[0])));
    }
    if let Some(n) = &s.notice {
        main = main.child(notice(&k, &n.text, n.error));
    }
    main = main.child(content.grow(1.0).min_h(0.0));
    if s.term_open {
        let theme = s.omarchy.clone().unwrap_or_else(|| crate::theme::fallback(k.dark, k.c.bg_base, k.c.text_base));
        main = main.child(terminal::panel(s, &k, &theme));
    }
    main = main.child(status_bar(s, &k));

    // The window's height in view points, so tall dialogs scroll instead of overflowing.
    let window = cx.layout_size("kit-root").unwrap_or([880.0, 560.0]);
    dialogs::build(s, &k, &mut layer, window);

    // What's left for the path between the history buttons and the trailing controls, from
    // last frame's layout: the toolbar's width less the trailing group, the three history
    // buttons, the paddings and, with the sidebar closed, its toggle.
    let bar = cx.layout_size("toolbar").map(|t| t[0]);
    let room = match (bar, cx.layout_size("toolbar/trailing")) {
        (Some(w), Some(t)) => Some(w - t[0] - 130.0 - if s.sidebar.is_open() { 0.0 } else { 44.0 }),
        _ => None,
    };

    let split = SplitView::new(main)
        .sidebar(&s.sidebar, |s: &mut App| &mut s.sidebar, sidebar::build(s, &k))
        .inspector(&s.activity, |s: &mut App| &mut s.activity, side_pane(s, &k))
        .toolbar(toolbar::build(s, &k, room, bar))
        .titlebar_inset(cx.titlebar_inset())
        .sidebar_toggle(SidebarToggle::new().size(ControlSize::Regular))
        .reduced_motion(cx.is_reduced_motion())
        .build(&k);
    if s.swipe.active() {
        layer.push(swipe_arrow(s, &k));
    }
    if let Some(list) = toolbar::completions(s, &k, cx.layout("path-field")) {
        layer.push(list);
    }
    rune_kit::root(split, layer)
        .on_wheel(|s: &mut App, e, tx| s.swipe(&e, tx))
        // A mouse's back and forward buttons move through history anywhere in the window.
        .pointer_buttons([PointerButton::Back, PointerButton::Forward])
        .on_press_tx(|s: &mut App, tx: &mut rune::Tx| match tx.button() {
            PointerButton::Back => s.go_back(),
            PointerButton::Forward => s.go_forward(),
            _ => {}
        })
        .on_drop_files(|s: &mut App, files, _tx| s.upload_paths(files))
        .on_magnify(|s: &mut App, e, _tx| {
            // Ctrl+scroll arrives here on Linux, as a pinch.
            s.set_zoom(s.zoom * (1.0 + e.delta));
            true
        })
}

/// Browser-style tabs over the file list, shown once there are two or more.
fn tab_strip(s: &App, k: &Kit, width: Option<f32>) -> El {
    use rune_kit::shell::{TabItem, TabOrientation, TabStrip};
    let items = (0..s.tabs.len()).map(|i| {
        let loc = s.tab_loc(i);
        let icon: &'static rune::Icon = match loc {
            crate::model::Location::Remote { .. } => &crate::icons::SERVER,
            crate::model::Location::Archive { inner, .. } if inner.is_empty() => &crate::icons::ARCHIVE,
            _ => &rune_kit::glyphs::FOLDER,
        };
        TabItem::new(s.tabs[i].id.as_str(), loc.name()).icon(icon).closable(true)
    });
    let plus = IconButton::new(&crate::icons::PLUS)
        .variant(IconButtonVariant::GhostMuted)
        .label("New tab")
        .help("New tab (Ctrl+N)")
        .on_press(|s: &mut App| s.new_tab(None))
        .build(k)
        .key("new-tab");
    let strip = TabStrip::new("tabs", TabOrientation::Horizontal)
        .tabs(items.collect::<Vec<_>>())
        .active(Some(s.tabs[s.active].id.as_str()))
        .viewport_width(width)
        .drag_state(&s.tab_drag, |s: &mut App| &mut s.tab_drag)
        .trailing_action(plus)
        .close_icon(&rune_kit::glyphs::CLOSE)
        .label("Tabs")
        .on_select(|s: &mut App, id, _| {
            if let Some(i) = s.tab_index(id) {
                s.switch_tab(i);
            }
        })
        .on_close(|s: &mut App, id, tx| {
            if let Some(i) = s.tab_index(id)
                && !s.close_tab(i)
            {
                tx.close_window();
            }
        })
        .on_reorder(|s: &mut App, r, _| s.move_tab(r.from, r.to))
        .build(k);
    row().key("tab-bar").w_full().px(6.0).pt(4.0).child(strip.grow(1.0).min_w(0.0))
}

/// The right-hand pane: the selection's preview, or the transfers.
fn side_pane(s: &App, k: &Kit) -> El {
    use crate::app::Side;
    let tabs = rune_kit::SegmentedControl::new(&["Preview", "Transfers"], Some(if s.side == Side::Preview { 0 } else { 1 }))
        .label("Pane")
        .on_pick(|s: &mut App, i, _| s.side = if i == 0 { Side::Preview } else { Side::Transfers })
        .build(k)
        .key("side-tabs");
    let content = match s.side {
        Side::Preview => col().key("preview-scroll").w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(preview::pane(s, k)),
        Side::Transfers => transfers::build(s, k).grow(1.0).min_h(0.0),
    };
    col().w_full().h_full().min_h(0.0).child(row().w_full().px(12.0).pt(8.0).pb(4.0).child(tabs)).child(content)
}

/// The arrow a sideways swipe pulls in from the edge, filled once letting go would navigate.
fn swipe_arrow(s: &App, k: &Kit) -> El {
    let [x, y, w, h] = s.content_at;
    let back = s.swipe.dx > 0.0;
    let p = (s.swipe.dx.abs() / crate::app::SWIPE_DISTANCE).min(1.0);
    let ready = p >= 1.0;
    let size = 44.0;
    let travel = -size + p * (size + 16.0);
    let left = if back { x + travel } else { x + w - size - travel };
    rune::el()
        .key("swipe-arrow")
        .absolute(left, y + h / 2.0 - size / 2.0)
        .size(size, size)
        .radius(size / 2.0)
        .center()
        .opacity(0.35 + 0.65 * p)
        .fill(if ready { k.c.icon_accent } else { k.c.surface_raised_base })
        .shadow(4.0, 12.0, rune::Color::BLACK.alpha(0.25))
        .child(rune::icon(if back { &crate::icons::BACK } else { &crate::icons::FORWARD }, 20.0).color(if ready { rune::Color::WHITE } else { k.c.icon_base }))
}

fn notice(k: &Kit, msg: &str, error: bool) -> El {
    let ink = if error { k.c.danger.fg } else { k.c.text_base };
    let fill = if error { k.c.danger.bg } else { k.c.layer_02 };
    row()
        .key("notice")
        .w_full()
        .px(12.0)
        .py(6.0)
        .gap(8.0)
        .center_y()
        .fill(fill)
        .child(text(msg.to_owned()).font(k.ty.small).color(ink).wrap().grow(1.0).shrink())
        .child(
            IconButton::new(&glyphs::CLOSE)
                .variant(IconButtonVariant::GhostMuted)
                .label("Dismiss")
                .on_press(|s: &mut App| s.notice = None)
                .build(k)
                .key("notice-close"),
        )
}

fn status_bar(s: &App, k: &Kit) -> El {
    let total = s.shown.len();
    let sel = s.selection();
    let mut left = format!("{total} item{}", plural(total));
    if !sel.is_empty() {
        let bytes: u64 = sel.iter().filter_map(|n| s.entry(n)).filter_map(|e| e.size).sum();
        left.push_str(&format!(" · {} selected", sel.len()));
        if bytes > 0 {
            left.push_str(&format!(" ({})", human_size(bytes)));
        }
    }
    let busy = s.loading || s.connecting.is_some();
    let mut bar = row()
        .key("status")
        .w_full()
        .h(28.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_weak)
        .child(text(left).font(k.ty.text_small).color(k.c.text_muted));
    if busy {
        let what = match &s.connecting {
            Some(h) => format!("Connecting to {h}…"),
            None => "Loading…".into(),
        };
        bar = bar.child(Spinner::new().size(12.0).build(k)).child(text(what).font(k.ty.text_small).color(k.c.text_faint));
    }
    bar = bar.child(el_spacer());
    if let Some(c) = &s.clip {
        let verb = if c.mode == crate::ops::Mode::Copy { "copy" } else { "move" };
        bar = bar.child(text(format!("{} item{} to {verb} · Ctrl+V pastes", c.names.len(), plural(c.names.len()))).font(k.ty.text_small).color(k.c.text_faint));
    }
    let active = s.jobs.active();
    if active > 0 {
        bar = bar.child(text(format!("{active} transfer{} running", plural(active))).font(k.ty.text_small).color(k.c.text_muted));
    }
    if (s.zoom - crate::app::DEFAULT_ZOOM).abs() > 0.001 {
        bar = bar.child(
            rune::el()
                .key("zoom")
                .px(4.0)
                .radius(k.radius.sm)
                .hover(k.c.hover)
                .help("Zoom (Ctrl+0 resets)")
                .on_press(|s: &mut App| s.zoom_step(0))
                .child(text(format!("{:.0}%", s.zoom / crate::app::DEFAULT_ZOOM * 100.0)).font(k.ty.text_small).color(k.c.text_muted)),
        );
    }
    if s.read_only() {
        bar = bar.child(text("Archive · read-only").font(k.ty.text_small).color(k.c.text_faint));
    }
    if s.show_hidden {
        bar = bar.child(text("Hidden files shown").font(k.ty.text_small).color(k.c.text_faint));
    }
    bar
}

fn el_spacer() -> El {
    rune::el().grow(1.0)
}
