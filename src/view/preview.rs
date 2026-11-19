//! The preview pane's Preview tab and Quick Look: the selection's picture, text, Markdown or
//! contents, then its details.

use rune::{El, ScrollSpec, SurfaceFit, col, row, text};
use rune_kit::{Button, ButtonVariant, Dialog as KitDialog, Kit, Markdown, Spinner, glyphs};

use crate::app::{App, plural};
use crate::model::{Entry, format_time, human_size, kind_label};
use crate::preview::{Body, Preview};
use crate::view::list::file_icon;

/// How much of a text file to draw: the pane shows less than Quick Look.
fn text_budget(large: bool) -> (usize, usize) {
    if large { (1200, 64 * 1024) } else { (240, 16 * 1024) }
}

fn clip_text(t: &str, large: bool) -> (&str, bool) {
    let (max_lines, max_bytes) = text_budget(large);
    let mut end = t.len().min(max_bytes);
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    if let Some((i, _)) = t[..end].match_indices('\n').nth(max_lines.saturating_sub(1)) {
        end = i;
    }
    (&t[..end], end < t.len())
}

/// The preview's main part: picture, text, Markdown, listing, or an icon and a reason.
pub fn body(s: &App, k: &Kit, p: &Preview, max_w: f32, max_h: f32, large: bool) -> El {
    let frame = |e: El| col().w_full().radius(k.radius.md).fill(k.c.layer_01).border(1.0, k.c.border_weak).clip_rounded(k.radius.md).child(e);
    match &p.body {
        Body::Image(_) => match &s.preview_image {
            Some(img) if img.error().is_some() => message(k, &glyphs::FILE_IMAGE, "This image couldn't be decoded"),
            Some(img) => match img.size() {
                Some([w, h]) if w > 0 && h > 0 => {
                    let ratio = w as f32 / h as f32;
                    el_image(img, ratio, max_w, max_h, k)
                }
                _ => col().w_full().h(max_h.min(220.0)).center().child(Spinner::new().size(18.0).build(k)),
            },
            None => message(k, &glyphs::FILE_IMAGE, "No preview"),
        },
        Body::Text { text: t, lang, truncated } => {
            let (shown, cut) = clip_text(t, large);
            let ranges = rune_kit::syntax::highlight(*lang, shown);
            let spans = rune_kit::syntax::SyntaxTheme::from_kit(k).spans(&ranges, 0);
            let mut c = col()
                .w_full()
                .padding(10.0)
                .gap(6.0)
                .child(rune::rich(shown.to_owned(), spans).font(k.ty.mono).color(k.c.text_base).wrap_anywhere().selectable(true));
            if cut || *truncated {
                c = c.child(text("… open the file to see the rest").font(k.ty.text_small).color(k.c.text_faint));
            }
            frame(col().w_full().max_h(max_h).scroll(ScrollSpec { stick_bottom: false }).child(c))
        }
        Body::Markdown(src) => {
            let (shown, _) = clip_text(src, large);
            frame(
                col()
                    .w_full()
                    .max_h(max_h)
                    .scroll(ScrollSpec { stick_bottom: false })
                    .child(col().w_full().padding(14.0).child(Markdown::new(shown).selectable(true).build(k))),
            )
        }
        Body::Archive { names, total } => {
            let limit = if large { 200 } else { 40 };
            let mut c = col().w_full().padding(8.0).gap(2.0);
            for (name, dir) in names.iter().take(limit) {
                let e = Entry {
                    name: name.clone(),
                    kind: if *dir { crate::model::Kind::Dir } else { crate::model::Kind::File },
                    link: false,
                    size: None,
                    modified: None,
                    mode: None,
                };
                c = c.child(
                    row()
                        .gap(8.0)
                        .h(24.0)
                        .center_y()
                        .child(rune::icon(file_icon(&e), 14.0).color(if *dir { k.c.icon_accent } else { k.c.icon_muted }))
                        .child(text(name.clone()).font(k.ty.small).color(k.c.text_base).truncate()),
                );
            }
            if names.len() > limit {
                c = c.child(text(format!("and {} more at the top level", names.len() - limit)).font(k.ty.text_small).color(k.c.text_faint).pt(4.0));
            }
            let _ = total;
            frame(col().w_full().max_h(max_h).scroll(ScrollSpec { stick_bottom: false }).child(c))
        }
        Body::Folder { folders, files, bytes } => {
            let mut line = format!("{folders} folder{}, {files} file{}", plural(*folders), plural(*files));
            if *bytes > 0 {
                line.push_str(&format!(" · {} in files", human_size(*bytes)));
            }
            message(k, &glyphs::FOLDER, &line)
        }
        Body::Nothing(why) => {
            let icon = match crate::preview::kind_of(&p.name) {
                crate::preview::Kind::Audio => &crate::icons::MUSIC,
                crate::preview::Kind::Video => &crate::icons::VIDEOS,
                _ => s.entry(&p.name).map_or(&glyphs::FILE, file_icon),
            };
            message(k, icon, why)
        }
    }
}

fn el_image(img: &rune::Image, ratio: f32, max_w: f32, max_h: f32, k: &Kit) -> El {
    // The largest box at the image's shape inside max_w × max_h, sized here so whatever holds
    // it (a fitted dialog) measures the shown size, not the image's own.
    let w = max_w.min(max_h * ratio).max(1.0);
    let h = w / ratio;
    row().w_full().justify_center().child(rune::el().size(w, h).radius(k.radius.md).pixels_fit(img.pixels(), SurfaceFit::Fit))
}

fn message(k: &Kit, icon: &'static rune::Icon, msg: &str) -> El {
    col()
        .w_full()
        .py(28.0)
        .gap(10.0)
        .center()
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .child(rune::icon(icon, 40.0).color(k.c.icon_faint))
        .child(text(msg.to_owned()).font(k.ty.small).color(k.c.text_muted).wrap().align(rune::Align::Center).px(16.0))
}

/// Name, kind, size, date, then what the preview found (dimensions, duration, tags…).
fn details(s: &App, k: &Kit, p: &Preview) -> El {
    let e = s.entry(&p.name);
    let mut rows: Vec<(String, String)> = Vec::new();
    if let Some(e) = e {
        let kind =
            if e.is_dir() { "Folder".to_owned() } else { format!("{} ({})", crate::preview::kind_word(crate::preview::kind_of(&e.name)), kind_label(e)) };
        rows.push(("Kind".into(), kind));
        if let Some(b) = e.size {
            rows.push(("Size".into(), human_size(b)));
        }
        if let Some(t) = e.modified {
            rows.push(("Modified".into(), format_time(t, s.now, s.utc_offset)));
        }
    }
    if let (Some(img), Body::Image(_)) = (&s.preview_image, &p.body)
        && let Some([w, h]) = img.size()
        && !p.facts.iter().any(|f| f.0 == "Dimensions" || f.0 == "Resolution")
        && matches!(crate::preview::kind_of(&p.name), crate::preview::Kind::Image)
    {
        rows.push(("Dimensions".into(), format!("{w} × {h}")));
    }
    rows.extend(p.facts.iter().map(|(k, v)| (k.to_string(), v.clone())));
    rows.into_iter().fold(col().w_full().gap(5.0), |c, (label, value)| {
        c.child(
            row()
                .w_full()
                .gap(10.0)
                .child(text(label).font(k.ty.text_small).color(k.c.text_faint).w(76.0))
                .child(text(value).font(k.ty.text_small).color(k.c.text_base).wrap().grow(1.0).shrink().min_w(0.0)),
        )
    })
}

/// The Preview tab.
pub fn pane(s: &App, k: &Kit) -> El {
    let Some(p) = &s.preview else {
        let hint = if s.selection().is_empty() { "Select a file to preview it" } else { "Loading the preview…" };
        return col()
            .w_full()
            .padding(16.0)
            .gap(8.0)
            .child(text(hint).font(k.ty.ui).color(k.c.text_faint))
            .child(text("Space shows a larger preview; ↑ ↓ step through the folder while it's open.").font(k.ty.text_small).color(k.c.text_faint).wrap());
    };
    let name = p.name.clone();
    col()
        .key("preview")
        .w_full()
        .padding(12.0)
        .gap(12.0)
        .child(body(s, k, p, (s.activity.width() - 24.0).max(120.0), 360.0, false))
        .child(text(p.name.clone()).font(k.ty.ui_medium).color(k.c.text_base).wrap_anywhere())
        .child(details(s, k, p))
        .child(
            row()
                .gap(8.0)
                .child(Button::new("Open").variant(ButtonVariant::Outline).on_press(move |s: &mut App| s.open(&name)).build(k).key("preview-open"))
                .child(
                    Button::new("Quick Look")
                        .variant(ButtonVariant::Ghost)
                        .help("Space")
                        .on_press(|s: &mut App| s.toggle_quick_look())
                        .build(k)
                        .key("preview-quick-look"),
                ),
        )
}

/// Quick Look's dialog, sized to the window.
pub fn quick_look(s: &App, k: &Kit, width: f32, height: f32) -> KitDialog {
    let w = (width - 120.0).clamp(360.0, 1100.0);
    let body_h = (height - 230.0).max(160.0);
    let title = s.preview.as_ref().map_or_else(|| s.selection().first().cloned().unwrap_or_default(), |p| p.name.clone());
    let content = match &s.preview {
        Some(p) => col().w_full().gap(10.0).child(body(s, k, p, w - 32.0, body_h, true)).child(
            row().w_full().gap(16.0).child(summary(s, k, p)).child(rune::el().grow(1.0)).child(
                Button::new("Open")
                    .variant(ButtonVariant::Contrast)
                    .on_press(|s: &mut App| {
                        if let Some(n) = s.preview.as_ref().map(|p| p.name.clone()) {
                            s.dialog = None;
                            s.open(&n);
                        }
                    })
                    .build(k)
                    .key("quick-look-open")
                    .default_button(),
            ),
        ),
        None => col().w_full().h(body_h.min(240.0)).center().child(Spinner::new().size(20.0).build(k)),
    };
    // Focused when Quick Look opens, so its keys come here before the dialog's buttons'.
    let body = col().key("quick-look-body").focusable(true).focus_ring(0.0, 0.0, rune::Color::TRANSPARENT).w_full().px(16.0).pb(16.0).child(content).on_key(
        |s: &mut App, ev: &rune::KeyEvent, tx: &mut rune::Tx| {
            let m = ev.mods;
            if m.cmd || m.alt {
                return false;
            }
            match &ev.key {
                rune::Key::Text(t) if t == " " => s.toggle_quick_look(),
                rune::Key::Up | rune::Key::Left => s.step_selection(-1, tx),
                rune::Key::Down | rune::Key::Right => s.step_selection(1, tx),
                _ => return false,
            }
            true
        },
    );
    KitDialog::new("quick-look", title).width(w).child(body)
}

/// One line of facts under a Quick Look.
fn summary(s: &App, k: &Kit, p: &Preview) -> El {
    let mut parts = Vec::new();
    if let Some(e) = s.entry(&p.name) {
        if let Some(b) = e.size {
            parts.push(human_size(b));
        }
        if let Some(t) = e.modified {
            parts.push(format_time(t, s.now, s.utc_offset));
        }
    }
    parts.extend(p.facts.iter().take(4).map(|(k, v)| format!("{k} {v}")));
    text(parts.join("  ·  ")).font(k.ty.text_small).color(k.c.text_muted).wrap().shrink().min_w(0.0)
}
