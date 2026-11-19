//! History buttons, the breadcrumb path (or a path field), the filter and folder actions.

use rune::{El, TextEdit, Tx, row, text};
use rune_kit::shell::Toolbar;
use rune_kit::{Badge, Button, ButtonVariant, IconButton, IconButtonVariant, Kit, TextInput, glyphs};

use crate::app::App;
use crate::icons;
use crate::model::Location;
use crate::ssh::Target;

/// `room`: points the path can take, from last frame's layout (None before the first).
/// `bar`: the toolbar's width last frame. Narrow bars get a shorter filter; tight ones drop
/// the filter (Ctrl+F brings it back), new folder and reload (in the menu and on F5).
pub fn build(s: &App, k: &Kit, room: Option<f32>, bar: Option<f32>) -> Toolbar {
    let narrow = bar.is_some_and(|w| w < 680.0);
    let tight = bar.is_some_and(|w| w < 520.0);
    let nav = |icon: &'static rune::Icon, label: &str, enabled: bool, f: fn(&mut App)| {
        IconButton::new(icon).variant(IconButtonVariant::GhostMuted).label(label).help(label).disabled(!enabled).on_press(f).build(k)
    };
    let leading = row()
        .gap(2.0)
        .center_y()
        .shrink()
        .min_w(0.0)
        .child(nav(&icons::BACK, "Back", !s.back.is_empty(), App::go_back).key("back"))
        .child(nav(&icons::FORWARD, "Forward", !s.forward.is_empty(), App::go_forward).key("forward"))
        .child(nav(&icons::UP, "Enclosing folder", s.loc.parent().is_some(), App::go_up).key("up"));

    let center = match &s.path_edit {
        Some(edit) => TextInput::new(edit)
            .placeholder("/path, ~/path or user@host:/path")
            .label("Location")
            .on_edit(|s: &mut App| s.path_edit.get_or_insert_with(|| TextEdit::new("")))
            .on_change(|s: &mut App, _| s.request_completions())
            .on_submit(|s: &mut App, _tx: &mut Tx| {
                if let Some(i) = s.completion_pick {
                    s.accept_completion(i);
                }
                s.completions.clear();
                submit_path(s)
            })
            .on_key(|s: &mut App, ev, _| {
                match ev.key {
                    rune::Key::Escape if !s.completions.is_empty() => s.completions.clear(),
                    rune::Key::Escape => s.path_edit = None,
                    rune::Key::Down => return s.move_completion(1),
                    rune::Key::Up => return s.move_completion(-1),
                    // Tab takes the highlighted completion, else the first.
                    rune::Key::Tab if !ev.mods.shift && !s.completions.is_empty() => s.accept_completion(s.completion_pick.unwrap_or(0)),
                    _ => return false,
                }
                true
            })
            .width(Some(room.map_or(440.0, |r| r.clamp(160.0, 440.0))))
            .build(k)
            .key("path-field"),
        None => crumbs(s, k, room),
    };

    let filter = TextInput::new(&s.filter)
        .placeholder("Filter")
        .label("Filter this folder")
        .leading_icon(&glyphs::SEARCH)
        .width(Some(if narrow { 110.0 } else { 170.0 }))
        .on_edit(|s: &mut App| &mut s.filter)
        .on_change(|s: &mut App, _| s.refilter())
        .on_key(|s: &mut App, ev, tx| {
            if ev.key == rune::Key::Escape {
                s.filter = TextEdit::new("");
                s.refilter();
                tx.focus("files", false);
                return true;
            }
            if matches!(ev.key, rune::Key::Down | rune::Key::Enter) {
                tx.focus("files", true);
                return true;
            }
            false
        })
        .build(k)
        .key("filter");

    let active = s.jobs.active();
    let transfers = IconButton::new(&icons::TRANSFERS)
        .variant(IconButtonVariant::GhostMuted)
        .label("Transfers")
        .help("Transfers")
        .expanded(s.activity.is_open() && s.side == crate::app::Side::Transfers)
        .on_press(|s: &mut App| s.toggle_side(crate::app::Side::Transfers))
        .build(k)
        .key("transfers-toggle");
    let transfers = if active > 0 { row().center_y().gap(2.0).child(transfers).child(Badge::new(active.to_string()).build(k)) } else { row().child(transfers) };

    let mut trailing = row().gap(2.0).center_y();
    if !tight || !s.filter.text.is_empty() || s.filter.focused {
        trailing = trailing.child(filter);
    }
    trailing = trailing.child(
        IconButton::new(if s.show_hidden { &glyphs::EYE } else { &glyphs::EYE_OFF })
            .variant(IconButtonVariant::GhostMuted)
            .label(if s.show_hidden { "Hide hidden files" } else { "Show hidden files" })
            .help("Hidden files (Ctrl+H)")
            .expanded(s.show_hidden)
            .on_press(|s: &mut App| s.toggle_hidden())
            .build(k)
            .key("hidden"),
    );
    if !tight {
        trailing = trailing
            .child(
                IconButton::new(&icons::FOLDER_PLUS)
                    .variant(IconButtonVariant::GhostMuted)
                    .label("New folder")
                    .help("New folder (Ctrl+Shift+N)")
                    .on_press(|s: &mut App| s.start_new_folder())
                    .build(k)
                    .key("new-folder"),
            )
            .child(
                IconButton::new(&icons::REFRESH)
                    .variant(IconButtonVariant::GhostMuted)
                    .label("Reload")
                    .help("Reload (F5)")
                    .on_press(|s: &mut App| s.refresh())
                    .build(k)
                    .key("reload"),
            );
    }
    let showing = s.activity.is_open() && s.side == crate::app::Side::Preview;
    let trailing = trailing
        .child(
            IconButton::new(&rune_kit::shell::INSPECTOR)
                .variant(IconButtonVariant::GhostMuted)
                .label("Preview pane")
                .help("Preview pane (Alt+P)")
                .expanded(showing)
                .on_press(|s: &mut App| s.toggle_side(crate::app::Side::Preview))
                .build(k)
                .key("preview-toggle"),
        )
        .child(transfers);

    Toolbar::new().leading(leading.child(center.shrink().min_w(0.0))).trailing(trailing)
}

/// Estimated width of a crumb button with its separator; text averages about 7 points a
/// character in the UI font.
fn crumb_width(label: &str) -> f32 {
    label.chars().count() as f32 * 7.0 + 22.0 + 14.0
}

/// The path as buttons, one per folder; a press on the empty part edits it as text. When the
/// toolbar is tight, the leading folders fold into "…" so the current one stays in view.
fn crumbs(s: &App, k: &Kit, room: Option<f32>) -> El {
    let mut bar = row().key("crumbs").h(28.0).gap(0.0).center_y().radius(k.radius.md).px(2.0).shrink().min_w(0.0).clip_rounded(k.radius.md);
    let mut budget = room.unwrap_or(f32::INFINITY) - 30.0;
    if let Some(host) = s.loc.host() {
        budget -= host.chars().count() as f32 * 7.0 + 36.0;
        bar = bar.child(
            row()
                .key("host-chip")
                .gap(4.0)
                .px(6.0)
                .h(22.0)
                .center_y()
                .radius(k.radius.sm)
                .fill(k.c.layer_02)
                .child(rune::icon(&icons::SERVER, 13.0).color(k.c.icon_accent))
                .child(text(host.to_owned()).font(k.ty.small_medium).color(k.c.text_base).truncate()),
        );
    }
    let all = s.loc.crumbs();
    // Fit from the current folder back; it always shows, even cut short.
    let mut skip = all.len().saturating_sub(1);
    let mut used = all.last().map_or(0.0, |(l, _)| crumb_width(l));
    while skip > 0 {
        let w = crumb_width(&all[skip - 1].0);
        let ellipsis = if skip > 1 { 24.0 } else { 0.0 };
        if used + w + ellipsis > budget {
            break;
        }
        used += w;
        skip -= 1;
    }
    if skip > 0 {
        let parent = all[skip - 1].1.clone();
        bar = bar.child(
            Button::new("…")
                .variant(ButtonVariant::GhostMuted)
                .help("Enclosing folders")
                .on_press(move |s: &mut App| s.go(parent.clone()))
                .build(k)
                .key("crumb-more"),
        );
    }
    let last = all.len().saturating_sub(1);
    for (i, (label, loc)) in all.into_iter().enumerate().skip(skip) {
        if i > 0 {
            bar = bar.child(rune::icon(&glyphs::CARET_RIGHT, 10.0).color(k.c.icon_faint));
        }
        let current = i == last;
        let b = Button::new(label)
            .variant(if current { ButtonVariant::Ghost } else { ButtonVariant::GhostMuted })
            .on_press(move |s: &mut App| s.go(loc.clone()))
            .build(k)
            .key(format!("crumb-{i}"));
        bar = bar.child(if current { b.shrink().min_w(0.0) } else { b });
    }
    bar.child(rune::el().key("crumb-space").w(32.0).h_full().on_press(|s: &mut App| start_path_edit(s)).help("Click to type a location (Ctrl+L)"))
}

/// The completions under the path field.
pub fn completions(s: &App, k: &Kit, at: Option<[f32; 4]>) -> Option<El> {
    let [x, y, w, h] = at?;
    if s.path_edit.is_none() || s.completions.is_empty() {
        return None;
    }
    let rows = s.completions.iter().enumerate().fold(rune::col().w_full().padding(4.0).gap(1.0), |c, (i, path)| {
        let picked = s.completion_pick == Some(i);
        let name = path.trim_end_matches(['/', ':']).rsplit('/').next().unwrap_or(path).to_owned();
        c.child(
            row()
                .key(format!("completion-{i}"))
                .w_full()
                .h(26.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(k.radius.sm)
                .fill(if picked { k.c.icon_accent.alpha(0.22) } else { rune::Color::TRANSPARENT })
                .hover(k.c.hover)
                .on_press(move |s: &mut App| s.accept_completion(i))
                .child(rune::icon(if path.ends_with(':') { &icons::SERVER } else { &glyphs::FOLDER }, 14.0).color(k.c.icon_accent))
                .child(text(name).font(k.ty.small_medium).color(k.c.text_base))
                .child(text(path.clone()).font(k.ty.text_small).color(k.c.text_faint).truncate().shrink().min_w(0.0)),
        )
    });
    Some(
        rune::el()
            .key("completions")
            .absolute(x, y + h + 4.0)
            .w(w.max(320.0))
            .radius(k.radius.md)
            .fill(k.c.surface_raised_base)
            .border(1.0, k.c.border_base)
            .shadow(6.0, 18.0, rune::Color::BLACK.alpha(0.3))
            .child(rows),
    )
}

pub fn start_path_edit(s: &mut App) {
    let mut e = TextEdit::new(s.loc_label(&s.loc));
    e.focused = true;
    e.anchor = 0;
    e.focus = e.text.len();
    s.path_edit = Some(e);
}

fn submit_path(s: &mut App) {
    let Some(edit) = s.path_edit.take() else { return };
    let input = edit.text.trim().to_owned();
    if input.is_empty() {
        return;
    }
    let home = crate::local::home();
    // A typed path stays on the machine shown: the server's, or this one (also from inside an
    // archive). A local archive file opens as a folder.
    let local = |p: std::path::PathBuf| {
        if p.is_file() && crate::archive::is_archive_name(&p.to_string_lossy()) {
            Location::Archive { file: Box::new(Location::Local(p)), inner: String::new() }
        } else {
            Location::Local(p)
        }
    };
    let host = s.loc.host().map(str::to_owned);
    if let Some(rest) = input.strip_prefix('~').filter(|r| r.is_empty() || r.starts_with('/')) {
        let dest = match host {
            Some(host) => {
                let h = s.hosts.iter().find(|h| h.id == host).and_then(|h| h.home.clone()).unwrap_or_default();
                Location::Remote { host, path: format!("{h}{rest}") }
            }
            None => local(home.join(rest.trim_start_matches('/'))),
        };
        return s.go(dest);
    }
    if input.starts_with('/') {
        let dest = match host {
            Some(host) => Location::Remote { host, path: input },
            None => local(input.into()),
        };
        return s.go(dest);
    }
    match Target::parse(&input) {
        Ok((t, path)) => {
            if let Some(h) = s.hosts.iter().find(|h| h.id == t.id() && h.home.is_some()) {
                let path = path.or_else(|| h.home.clone()).unwrap_or_else(|| "/".into());
                let host = h.id.clone();
                s.go(Location::Remote { host, path });
            } else {
                s.connect(t, path);
            }
        }
        Err(e) => s.fail(e),
    }
}
