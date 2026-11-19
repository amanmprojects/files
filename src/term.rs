//! The terminal panel's terminal: a shell on a PTY, parsed by Alacritty's emulator
//! (`alacritty_terminal`), drawn by Rune as rows of rich text, with keys turned into the bytes
//! a terminal sends.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb};
use rune::{Color, Key, KeyEvent, Span};

use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }
    fn screen_lines(&self) -> usize {
        self.rows as usize
    }
    fn columns(&self) -> usize {
        self.cols as usize
    }
}

fn window_size(s: Size) -> WindowSize {
    WindowSize { num_lines: s.rows, num_cols: s.cols, cell_width: 8, cell_height: 16 }
}

/// Hears the emulator: new output wakes the window, the shell exiting is noted, and replies
/// the terminal must send (cursor position reports) go back to the PTY.
#[derive(Clone)]
struct Proxy {
    waker: rune::Waker,
    dirty: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    title: Arc<Mutex<String>>,
    pty: Arc<Mutex<Option<EventLoopSender>>>,
}

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => {}
            Event::Exit | Event::ChildExit(_) => self.exited.store(true, Ordering::SeqCst),
            Event::Title(t) => {
                if let Ok(mut title) = self.title.lock() {
                    *title = t;
                }
            }
            Event::ResetTitle => {
                if let Ok(mut title) = self.title.lock() {
                    title.clear();
                }
            }
            Event::PtyWrite(text) => {
                if let Some(tx) = self.pty.lock().ok().and_then(|p| p.clone()) {
                    let _ = tx.send(Msg::Input(Cow::Owned(text.into_bytes())));
                }
                return;
            }
            _ => return,
        }
        self.dirty.store(true, Ordering::SeqCst);
        (self.waker)();
    }
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Proxy>>>,
    tx: EventLoopSender,
    pub size: Size,
    dirty: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    title: Arc<Mutex<String>>,
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
    }
}

/// A drawn cell: character, ink, background (None: the terminal's own) and boldness.
type Cell = (char, Color, Option<Color>, bool);

/// One drawn row: its text and the colour runs over it.
pub struct Row {
    pub text: String,
    pub spans: Vec<Span>,
}

impl Terminal {
    /// Starts `program` (the login shell when None) in `cwd` at `size`.
    pub fn spawn(cwd: PathBuf, program: Option<(String, Vec<String>)>, size: Size, waker: rune::Waker) -> std::io::Result<Terminal> {
        let mut env = std::collections::HashMap::new();
        env.insert("TERM".to_owned(), "xterm-256color".to_owned());
        env.insert("COLORTERM".to_owned(), "truecolor".to_owned());
        let options = tty::Options { shell: program.map(|(p, a)| tty::Shell::new(p, a)), working_directory: Some(cwd), drain_on_exit: true, env };
        let pty = tty::new(&options, window_size(size), 0)?;
        let proxy = Proxy {
            waker,
            dirty: Arc::new(AtomicBool::new(true)),
            exited: Arc::new(AtomicBool::new(false)),
            title: Arc::new(Mutex::new(String::new())),
            pty: Arc::new(Mutex::new(None)),
        };
        let term = Arc::new(FairMutex::new(Term::new(Config::default(), &size, proxy.clone())));
        let event_loop = EventLoop::new(term.clone(), proxy.clone(), pty, true, false)?;
        let tx = event_loop.channel();
        if let Ok(mut p) = proxy.pty.lock() {
            *p = Some(tx.clone());
        }
        event_loop.spawn();
        Ok(Terminal { term, tx, size, dirty: proxy.dirty, exited: proxy.exited, title: proxy.title })
    }

    /// New output since the last call.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::SeqCst)
    }

    pub fn exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    pub fn title(&self) -> String {
        self.title.lock().map(|t| t.clone()).unwrap_or_default()
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.tx.send(Msg::Input(bytes.into()));
    }

    pub fn resize(&mut self, size: Size) {
        if size == self.size || size.cols < 2 || size.rows < 1 {
            return;
        }
        self.size = size;
        self.term.lock().resize(size);
        let _ = self.tx.send(Msg::Resize(window_size(size)));
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    /// Scrollback: positive lines go back in history.
    pub fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
        self.dirty.store(true, Ordering::SeqCst);
    }

    /// Text, as typed or pasted (bracketed when the program asked for it).
    pub fn paste(&self, text: &str) {
        let bracketed = self.mode().contains(TermMode::BRACKETED_PASTE);
        let body = text.replace("\r\n", "\r").replace('\n', "\r");
        let s = if bracketed { format!("\x1b[200~{}\x1b[201~", body.replace('\x1b', "")) } else { body };
        self.term.lock().scroll_display(Scroll::Bottom);
        self.write(s.into_bytes());
    }

    /// The screen as rows of text with colour spans, the cursor drawn as an inverted cell.
    pub fn rows(&self, theme: &Theme, focused: bool) -> Vec<Row> {
        let term = self.term.lock();
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let rows = term.screen_lines();
        let colors = content.colors;
        let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0;
        let cursor = content.cursor.point;
        let resolve = |c: AnsiColor, default_fg: bool| -> Color {
            let rgb = |r: Rgb| rgb8(r.r, r.g, r.b);
            match c {
                AnsiColor::Spec(r) => rgb(r),
                AnsiColor::Indexed(i) => colors[i as usize].map(rgb).unwrap_or_else(|| indexed(theme, i)),
                AnsiColor::Named(n) => match n {
                    NamedColor::Foreground | NamedColor::BrightForeground => theme.fg,
                    NamedColor::DimForeground => theme.fg.alpha(0.7),
                    NamedColor::Background => theme.bg,
                    NamedColor::Cursor => theme.fg,
                    n if (n as usize) < 16 => colors[n as usize].map(rgb).unwrap_or(theme.ansi[n as usize]),
                    n if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&(n as usize)) => {
                        theme.ansi[n as usize - NamedColor::DimBlack as usize].alpha(0.7)
                    }
                    _ => {
                        if default_fg {
                            theme.fg
                        } else {
                            theme.bg
                        }
                    }
                },
            }
        };
        let mut lines: Vec<Vec<Cell>> = vec![Vec::new(); rows];
        for cell in content.display_iter {
            let row = cell.point.line.0 + offset;
            if row < 0 || row as usize >= rows {
                continue;
            }
            let flags = cell.flags;
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let (mut fg, mut bg) = (resolve(cell.fg, true), resolve(cell.bg, false));
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if flags.contains(Flags::DIM) {
                fg = fg.alpha(0.7);
            }
            let at_cursor = show_cursor && cell.point == cursor;
            let (fg, bg) = if at_cursor && focused {
                (theme.bg, Some(theme.fg))
            } else if at_cursor {
                (fg, Some(theme.fg.alpha(0.35)))
            } else {
                (fg, (bg != theme.bg).then_some(bg))
            };
            let c = if cell.c == '\0' || cell.c == '\t' { ' ' } else { cell.c };
            lines[row as usize].push((c, fg, bg, flags.contains(Flags::BOLD)));
        }
        lines
            .into_iter()
            .map(|cells| {
                // Trailing blanks with nothing behind them draw nothing; drop them.
                let keep = cells.iter().rposition(|(c, _, bg, _)| *c != ' ' || bg.is_some()).map_or(0, |i| i + 1);
                let mut text = String::new();
                let mut spans: Vec<Span> = Vec::new();
                for (c, fg, bg, bold) in cells.into_iter().take(keep) {
                    let start = text.len();
                    text.push(c);
                    let end = text.len();
                    match spans.last_mut() {
                        Some(s) if s.end as usize == start && s.color == Some(fg) && s.bg == bg && (s.weight.is_some() == bold) => s.end = end as u32,
                        _ => {
                            let mut s = Span::new(start, end).color(fg);
                            s.bg = bg;
                            if bold {
                                s = s.bold();
                            }
                            spans.push(s);
                        }
                    }
                }
                Row { text, spans }
            })
            .collect()
    }

    /// The bytes a key sends, or None for keys the terminal leaves to the app.
    pub fn key_bytes(&self, ev: &KeyEvent) -> Option<Vec<u8>> {
        let m = ev.mods;
        let app_cursor = self.mode().contains(TermMode::APP_CURSOR);
        // xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
        let modp = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * (m.ctrl || m.cmd) as u8;
        let csi = |letter: char| -> Vec<u8> {
            if modp > 1 {
                format!("\x1b[1;{modp}{letter}").into_bytes()
            } else if app_cursor {
                format!("\x1bO{letter}").into_bytes()
            } else {
                format!("\x1b[{letter}").into_bytes()
            }
        };
        let tilde = |n: u8| -> Vec<u8> { if modp > 1 { format!("\x1b[{n};{modp}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() } };
        let alt = |mut b: Vec<u8>| -> Vec<u8> {
            if m.alt {
                b.insert(0, 0x1b);
            }
            b
        };
        Some(match &ev.key {
            Key::Text(t) => alt(t.as_bytes().to_vec()),
            Key::Command(c) => {
                let c = c.to_ascii_lowercase();
                let code = match c {
                    'a'..='z' => c as u8 & 0x1f,
                    '@' | ' ' | '2' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '/' | '7' => 0x1f,
                    '8' => 0x7f,
                    _ => return None,
                };
                alt(vec![code])
            }
            Key::Enter => alt(b"\r".to_vec()),
            Key::Backspace if m.ctrl || m.cmd => vec![0x08],
            Key::Backspace => alt(vec![0x7f]),
            Key::Tab if m.shift => b"\x1b[Z".to_vec(),
            Key::Tab => b"\t".to_vec(),
            Key::Escape => vec![0x1b],
            Key::Up => csi('A'),
            Key::Down => csi('B'),
            Key::Right => csi('C'),
            Key::Left => csi('D'),
            Key::Home => csi('H'),
            Key::End => csi('F'),
            Key::Insert => tilde(2),
            Key::Delete => tilde(3),
            Key::PageUp => tilde(5),
            Key::PageDown => tilde(6),
            Key::Function(n @ 1..=4) => {
                let l = b"PQRS"[*n as usize - 1];
                if modp > 1 { format!("\x1b[1;{modp}{}", l as char).into_bytes() } else { vec![0x1b, b'O', l] }
            }
            Key::Function(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][*n as usize - 5]),
            _ => return None,
        })
    }
}

/// A colour from 8-bit sRGB channels (Rune keeps colours in linear light).
fn rgb8(r: u8, g: u8, b: u8) -> Color {
    let c = |v: u8| rune::color::srgb_decode(v as f32 / 255.0);
    Color { r: c(r), g: c(g), b: c(b), a: 1.0 }
}

/// The 256-colour palette: the theme's 16, a 6×6×6 cube, then 24 greys.
fn indexed(theme: &Theme, i: u8) -> Color {
    match i {
        0..=15 => theme.ansi[i as usize],
        16..=231 => {
            let i = i - 16;
            let v = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
            rgb8(v(i / 36), v((i / 6) % 6), v(i % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            rgb8(g, g, g)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_a_shell_and_reads_its_output() {
        let waker: rune::Waker = Arc::new(|| {});
        let t = Terminal::spawn(
            std::env::temp_dir(),
            Some(("sh".into(), vec!["-c".into(), "printf 'hello \\033[31mred\\033[0m'; sleep 5".into()])),
            Size { cols: 40, rows: 4 },
            waker,
        )
        .unwrap();
        let theme = crate::theme::fallback(true, Color::BLACK, Color::WHITE);
        let mut text = String::new();
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(30));
            let rows = t.rows(&theme, false);
            text = rows[0].text.clone();
            if text.contains("red") {
                let red = rows[0].spans.iter().find(|s| &rows[0].text[s.start as usize..s.end as usize] == "red").unwrap();
                assert_eq!(red.color, Some(theme.ansi[1]));
                return;
            }
        }
        panic!("no output: {text:?}");
    }

    #[test]
    fn keys_become_bytes() {
        let waker: rune::Waker = Arc::new(|| {});
        let t = Terminal::spawn(std::env::temp_dir(), Some(("sh".into(), vec!["-c".into(), "sleep 2".into()])), Size { cols: 20, rows: 4 }, waker).unwrap();
        let ev = |key: Key, ctrl: bool| KeyEvent { key, mods: rune::Mods { ctrl, cmd: ctrl, ..Default::default() } };
        assert_eq!(t.key_bytes(&ev(Key::Command('c'), true)), Some(vec![3]));
        assert_eq!(t.key_bytes(&ev(Key::Up, false)), Some(b"\x1b[A".to_vec()));
        assert_eq!(t.key_bytes(&ev(Key::Left, true)), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(t.key_bytes(&ev(Key::Text("é".into()), false)), Some("é".as_bytes().to_vec()));
    }
}
