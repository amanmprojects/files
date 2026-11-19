//! Omarchy's current theme (`~/.local/state/omarchy/current/theme/colors.toml`): the app's
//! background, text and accent, and the terminal's 16 colours. Re-read when the theme changes.

use std::collections::HashMap;
use std::path::PathBuf;

use rune::Color;

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub bg: Color,
    pub fg: Color,
    pub accent: Color,
    pub selection: Color,
    /// black, red, green, yellow, blue, magenta, cyan, white, then the bright eight.
    pub ansi: [Color; 16],
}

pub fn colors_file() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| crate::local::home().join(".local/state"));
    state.join("omarchy/current/theme/colors.toml")
}

/// The current Omarchy theme, if this is Omarchy.
pub fn load() -> Option<Theme> {
    parse(&std::fs::read_to_string(colors_file()).ok()?)
}

/// `key = "#rrggbb"` lines (the file is flat TOML).
pub fn parse(text: &str) -> Option<Theme> {
    let mut v: HashMap<&str, &str> = HashMap::new();
    for line in text.lines() {
        if let Some((k, val)) = line.split_once('=') {
            v.insert(k.trim(), val.trim().trim_matches('"'));
        }
    }
    let c = |k: &str| v.get(k).and_then(|h| rune::color::hex(h));
    let bg = c("background")?;
    let fg = c("foreground")?;
    let pick = |keys: &[&str], fallback: Color| keys.iter().find_map(|k| c(k)).unwrap_or(fallback);
    let black = pick(&["black", "dark_background"], bg);
    let white = pick(&["white", "foreground"], fg);
    let ansi = [
        black,
        pick(&["red"], fg),
        pick(&["green"], fg),
        pick(&["yellow"], fg),
        pick(&["blue"], fg),
        pick(&["magenta"], fg),
        pick(&["cyan"], fg),
        white,
        pick(&["bright_black", "muted", "dark_foreground"], fg),
        pick(&["bright_red", "red"], fg),
        pick(&["bright_green", "green"], fg),
        pick(&["bright_yellow", "yellow"], fg),
        pick(&["bright_blue", "blue"], fg),
        pick(&["bright_magenta", "magenta"], fg),
        pick(&["bright_cyan", "cyan"], fg),
        pick(&["bright_white", "bright_foreground", "foreground"], fg),
    ];
    Some(Theme {
        dark: v.get("mode").is_none_or(|m| *m != "light"),
        bg,
        fg,
        accent: pick(&["accent", "blue"], fg),
        selection: pick(&["selection"], fg.alpha(0.25)),
        ansi,
    })
}

/// A theme for the terminal when this isn't Omarchy: xterm's colours on the kit's background.
pub fn fallback(dark: bool, bg: Color, fg: Color) -> Theme {
    let h = |s: &str| rune::color::hex(s).unwrap_or(fg);
    Theme {
        dark,
        bg,
        fg,
        accent: h("#4f8cff"),
        selection: fg.alpha(0.25),
        ansi: [
            h("#1d1f21"),
            h("#cc6666"),
            h("#8fbf6a"),
            h("#e5c07b"),
            h("#61afef"),
            h("#c678dd"),
            h("#56b6c2"),
            h("#c5c8c6"),
            h("#666666"),
            h("#f07178"),
            h("#a5d6a7"),
            h("#ffd580"),
            h("#82b1ff"),
            h("#d7a7f5"),
            h("#89ddff"),
            h("#ffffff"),
        ],
    }
}

/// Watches the theme file on a thread (a stat every 2 s) and wakes the window when it changes.
pub fn watch(waker: rune::Waker, changed: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    std::thread::Builder::new()
        .name("theme-watch".into())
        .spawn(move || {
            let stamp = || std::fs::metadata(colors_file()).and_then(|m| m.modified()).ok();
            let mut last = stamp();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let now = stamp();
                if now != last {
                    last = now;
                    changed.store(true, std::sync::atomic::Ordering::SeqCst);
                    waker();
                }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_omarchy_colors() {
        let t = super::parse(
            "mode = \"dark\"\naccent = \"#798186\"\nbackground = \"#101315\"\nforeground = \"#cacccc\"\nred = \"#565d60\"\nbright_red = \"#de6145\"\n",
        )
        .unwrap();
        assert!(t.dark);
        assert_eq!(t.bg, rune::color::hex("#101315").unwrap());
        assert_eq!(t.ansi[9], rune::color::hex("#de6145").unwrap());
        assert!(super::parse("nothing = 1").is_none());
    }
}
