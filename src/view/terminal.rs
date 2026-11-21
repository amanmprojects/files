//! The bottom terminal panel (Ctrl+J), VS Code style: a header with the shell's title and
//! actions, a draggable top edge, and the terminal's screen.

use rune::{Cursor, El, PointerPress, Tx, col, row, text};
use rune_kit::{IconButton, IconButtonVariant, Kit, glyphs};

use crate::app::App;
use crate::icons;
use crate::theme::Theme;

pub fn panel(s: &App, k: &Kit, theme: &Theme) -> El {
    let title = s.terminal.as_ref().map(|t| t.title()).filter(|t| !t.is_empty()).unwrap_or_else(|| "Terminal".into());
    let exited = s.terminal.as_ref().is_none_or(|t| t.exited());
    let button = |icon: &'static rune::Icon, label: &str, key: &str, f: fn(&mut App, &mut Tx)| {
        IconButton::new(icon).variant(IconButtonVariant::GhostMuted).label(label).help(label).on_press_tx(f).build(k).key(key.to_owned())
    };
    let header = row()
        .key("term-header")
        .w_full()
        .h(30.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .child(rune::icon(&icons::TERMINAL, 14.0).color(k.c.icon_muted))
        .child(text(title).font(k.ty.small_medium).color(k.c.text_muted).truncate().shrink().min_w(0.0))
        .child(rune::el().grow(1.0))
        .child(button(&icons::FOLDER_GO, "Go to this folder", "term-cd", |s, tx| {
            s.terminal_cd_here();
            tx.focus("terminal", false);
        }))
        .child(button(&icons::REFRESH, "New shell here", "term-restart", |s, tx| {
            s.start_terminal();
            tx.focus("terminal", false);
        }))
        .child(button(&glyphs::CLOSE, "Hide (Ctrl+J)", "term-close", |s, tx| {
            s.term_open = false;
            tx.focus("files", false);
        }));

    let [_, ch] = s.term_cell;
    let rows = match &s.terminal {
        Some(t) => t.rows(theme, true),
        None => Vec::new(),
    };
    let mut screen = col().key("term-rows").w_full();
    for (i, r) in rows.into_iter().enumerate() {
        screen = screen.child(rune::rich(r.text, r.spans).key(format!("term-row-{i}")).font(k.ty.mono).color(theme.fg).h(ch));
    }
    if exited && s.terminal.is_some() {
        screen = screen.child(text("The shell exited. Press Enter for a new one.").font(k.ty.mono).color(theme.fg.alpha(0.6)).pt(6.0));
    }
    let body = col()
        .key("terminal")
        .focusable(true)
        .focus_ring(0.0, 0.0, rune::Color::TRANSPARENT)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(10.0)
        .py(4.0)
        .fill(theme.bg)
        .clip_rounded(0.0)
        .cursor(Cursor::Text)
        .on_key(|s: &mut App, ev: &rune::KeyEvent, tx: &mut Tx| s.terminal_key(ev, tx))
        .on_wheel(|s: &mut App, e, _tx| {
            let Some(t) = &s.terminal else { return false };
            let lines = (e.dy / s.term_cell[1].max(1.0)).round() as i32;
            if lines == 0 {
                return true;
            }
            if t.mode().contains(alacritty_terminal::term::TermMode::ALT_SCREEN) {
                // Full-screen programs (less, vim) scroll by arrow keys.
                let key = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
                for _ in 0..lines.unsigned_abs().min(10) {
                    t.write(key.to_vec());
                }
            } else {
                t.scroll(lines);
            }
            true
        })
        .on_layout_tx(|s: &mut App, r: [f32; 4], tx: &mut Tx| {
            let before = s.terminal.as_ref().map(|t| t.size);
            // Less the horizontal and vertical padding.
            s.terminal_area(r[2] - 20.0, r[3] - 8.0);
            if s.terminal.as_ref().map(|t| t.size) == before {
                tx.live_only();
            }
        })
        .on_secondary_press(|_s: &mut App, _p: PointerPress, tx: &mut Tx| tx.focus("terminal", false))
        .child(screen);

    // One line of the terminal's font, laid out off to the side, gives the cell size.
    let measure = text("MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM")
        .key("term-measure")
        .font(k.ty.mono)
        .absolute(-10_000.0, 0.0)
        .opacity(0.0)
        .inert()
        .on_layout_tx(|s: &mut App, r: [f32; 4], tx: &mut Tx| {
            let cell = [r[2] / 100.0, r[3]];
            if cell == s.term_cell {
                tx.live_only();
            } else {
                s.terminal_cell(cell);
            }
        });

    let edge = rune::el().key("term-edge").absolute(0.0, -3.0).w_full().h(6.0).cursor(Cursor::ResizeRow).on_drag(|s: &mut App, d, _tx| s.drag_terminal(d));

    col().key("term-panel").w_full().h(s.term_h).border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_base).child(header).child(body).child(measure).child(edge)
}
