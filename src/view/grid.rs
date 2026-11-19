//! The folder as a grid of tiles, as Nautilus shows it: each item's thumbnail (a photo, a PDF's
//! first page, a video frame) or its icon-theme picture, over its name. Rows are virtual, so a
//! folder of thousands builds only the tiles near the screen.

use std::borrow::Cow;

use rune::{Align, El, GrowthSpec, PointerPress, ScrollSpec, SurfaceFit, Tx, col, row, text};
use rune_kit::Kit;

use crate::app::{App, GRID_SIZES, MenuFor, Thumb};
use crate::model::Entry;
use crate::ops::Mode;
use crate::preview::{Kind, kind_of};
use crate::view::list::file_icon;

/// Space around the grid and between tiles.
const PAD: f32 = 12.0;
const GAP: f32 = 4.0;

/// Tiles to a row in `width` points with icons `icon` points tall.
pub fn columns(width: f32, icon: f32) -> usize {
    let tile = icon + 56.0;
    (((width - 2.0 * PAD + GAP) / (tile + GAP)).floor() as usize).max(1)
}

/// A row's height before it's measured: the icon and two lines of name.
fn row_estimate(icon: f32) -> f32 {
    icon + 70.0
}

pub fn content(s: &App, k: &Kit, frame: El, size: Option<[f32; 2]>) -> El {
    let icon = GRID_SIZES[s.grid_size];
    let cols = size.map_or(s.grid_cols, |[w, _]| columns(w, icon));
    let n = s.shown.len();
    let rows = n.div_ceil(cols);
    let kit = k.clone();
    let scroll = s.grid_scroll.clone();

    let body = col()
        .key("rows")
        .w_full()
        .px(PAD)
        .py(PAD)
        .growth(GrowthSpec::default())
        .virtual_rows(
            rows,
            row_estimate(icon),
            // A row is named by its first item, so revealing an item finds its row.
            move |s: &App, r| match s.shown.get(r * cols) {
                Some(&i) => Cow::Borrowed(s.entries[i].name.as_str()),
                None => Cow::Owned(format!("row-{r}")),
            },
            move |s: &App, r| grid_row(s, &kit, r, cols, icon),
        )
        .on_visible_rows(|s: &mut App, r| s.grid_visible(r));

    let viewport = col()
        .key("viewport")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .on_scroll_offset(move |at| scroll.set(at.offset[1]))
        .on_layout_tx(move |s: &mut App, r: [f32; 4], tx: &mut Tx| {
            let page = (r[3] / row_estimate(icon)).floor() as usize;
            if !s.grid_layout(columns(r[2], icon), page) {
                tx.live_only();
            }
        })
        // Sideways swipes and clicks between tiles: the scroll view would take them otherwise.
        .on_wheel(|s: &mut App, e, tx| s.swipe(&e, tx))
        .on_press(|s: &mut App| s.clear_selection())
        .child(body);

    let mut root = col()
        .key("files")
        .role("grid")
        .label("Files")
        .focusable(true)
        // The selection and the cursor's outline show focus, as in the table.
        .no_focus_ring()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .clip_soft(0.0)
        .on_key(|s: &mut App, ev: &rune::KeyEvent, tx: &mut Tx| s.grid_key(ev, tx));
    if let Some(c) = &s.grid.cursor {
        root = root.active_descendant(format!("tile:{c}"));
    }
    frame.child(root.child(viewport))
}

fn grid_row(s: &App, k: &Kit, r: usize, cols: usize, icon: f32) -> El {
    let mut out = row().w_full().gap(GAP).items_start();
    for j in 0..cols {
        let at = r * cols + j;
        out = match s.shown.get(at) {
            Some(&i) => out.child(tile(s, k, &s.entries[i], icon)),
            // Keeps a short last row's tiles the width of the others.
            None => out.child(rune::el().w(0.0).grow(1.0).shrink()),
        };
    }
    out.pb(GAP).on_wheel(|s: &mut App, e, tx| s.swipe(&e, tx))
}

fn tile(s: &App, k: &Kit, e: &Entry, icon: f32) -> El {
    let selected = s.grid.is_selected(&e.name);
    let cut = s.clip.as_ref().is_some_and(|c| c.mode == Mode::Move && c.dir == s.loc && c.names.contains(&e.name));
    let style = rune_kit::TableStyle::from_kit(k);
    let label = text(e.name.clone())
        .font(k.ty.ui)
        .color(if selected { k.c.text_base } else { k.c.text_base.alpha(0.92) })
        .align(Align::Center)
        .wrap_anywhere()
        // The whole name once selected, as Nautilus does.
        .max_lines(if selected { 8 } else { 3 })
        .w_full();
    let picture = rune::el().key("picture").size(icon, icon).center().child(picture(s, k, e, icon));
    let picture = if e.link { picture.child(link_badge(k, icon)) } else { picture };
    let mut t = col()
        .key(format!("tile:{}", e.name))
        .role("gridcell")
        .label(e.name.clone())
        .focus_selected(selected)
        .w(0.0)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .px(6.0)
        .pt(8.0)
        .pb(8.0)
        .gap(6.0)
        .items_center()
        .radius(k.radius.lg)
        .child(picture)
        .child(label);
    t = if selected { t.fill(style.selected) } else { t.hover(k.c.hover) };
    if selected && s.grid.selected.len() > 1 && s.grid.cursor.as_deref() == Some(e.name.as_str()) {
        t = t.outline(1.0, -1.0, k.c.border_focus);
    }
    if cut {
        t = t.opacity(0.45);
    }
    let (name, open, menu) = (e.name.clone(), e.name.clone(), e.name.clone());
    t.on_press_tx(move |s: &mut App, tx: &mut Tx| {
        let m = tx.mods();
        s.grid_click(&name, m.cmd || m.ctrl, m.shift);
        tx.focus("files", false);
    })
    .on_double_press(move |s: &mut App| s.open(&open))
    .on_secondary_press(move |s: &mut App, p: PointerPress, _tx: &mut Tx| {
        if !s.grid.is_selected(&menu) {
            s.grid.select(menu.clone());
        }
        s.menu = Some((p, MenuFor::Items));
    })
    .on_wheel(|s: &mut App, e, tx| s.swipe(&e, tx))
}

/// The thumbnail when there is one, else the icon theme's picture, else a line icon.
fn picture(s: &App, k: &Kit, e: &Entry, box_: f32) -> El {
    if let Some((_, _, Thumb::Image(_, img))) = s.thumbs.get(&e.name)
        && img.is_ready()
        && let Some([w, h]) = img.size().filter(|[w, h]| *w > 0 && *h > 0)
    {
        let (w, h) = (w as f32, h as f32);
        // Fit the box, but don't blow a tiny picture up past twice its size.
        let scale = (box_ / w).min(box_ / h).min(2.0 * box_ / 96.0);
        let (dw, dh) = ((w * scale).max(1.0), (h * scale).max(1.0));
        let pic = rune::el().size(dw, dh).pixels_fit(img.pixels(), SurfaceFit::Fit);
        let opaque = matches!(crate::model::extension(&e.name).as_deref(), Some("jpg" | "jpeg" | "heic" | "heif" | "avif" | "bmp" | "tif" | "tiff"));
        return match kind_of(&e.name) {
            // Photos and covers with rounded corners and a shadow; pictures that may be
            // transparent (PNG, WebP, GIF) as they are, or the shadow would draw a box.
            Kind::Image if !opaque => pic,
            Kind::Image | Kind::Audio => pic.radius(4.0).clip_rounded(4.0).shadow(1.0, 4.0, rune::Color::BLACK.alpha(0.25)),
            Kind::Video => pic.radius(2.0).clip_rounded(2.0).shadow(1.0, 4.0, rune::Color::BLACK.alpha(0.3)),
            _ => pic.border(1.0, rune::Color::BLACK.alpha(0.12)).shadow(1.0, 4.0, rune::Color::BLACK.alpha(0.25)),
        };
    }
    if let Some(img) = s.theme_icon(e) {
        return rune::el().size(box_, box_).pixels_fit(img.pixels(), SurfaceFit::Fit);
    }
    let ink = if e.is_dir() { k.c.icon_accent } else { k.c.icon_muted };
    rune::icon(file_icon(e), box_ * 0.62).color(ink)
}

/// A small arrow in the picture's corner for a symbolic link.
fn link_badge(k: &Kit, icon: f32) -> El {
    let d = (icon * 0.26).clamp(16.0, 28.0);
    rune::el()
        .key("link")
        .pin(None, None, Some(0.0), Some(0.0))
        .size(d, d)
        .radius(d / 2.0)
        .center()
        .fill(k.c.surface_raised_base)
        .shadow(1.0, 3.0, rune::Color::BLACK.alpha(0.3))
        .child(rune::icon(&crate::icons::LINK, d * 0.6).color(k.c.icon_base))
}
