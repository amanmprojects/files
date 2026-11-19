//! Opening a file in its default app. `xdg-open` on a desktop it doesn't know (Hyprland)
//! starts a terminal app (`Terminal=true`, such as nvim) with no terminal, so it runs unseen.
//! This looks the app up itself and runs terminal apps through `xdg-terminal-exec`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Opens `path` in its default app, off the UI thread.
pub fn open(path: &Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let cmd = match terminal_command(&path) {
            Some(argv) => {
                let mut c = Command::new("xdg-terminal-exec");
                c.args(argv);
                c
            }
            None => {
                let mut c = Command::new("xdg-open");
                c.arg(&path);
                c
            }
        };
        crate::app::spawn_detached(cmd);
    });
}

fn output(cmd: &mut Command) -> Option<String> {
    let out = cmd.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !s.is_empty()).then_some(s)
}

/// The command for `path`'s default app when that app runs in a terminal.
fn terminal_command(path: &Path) -> Option<Vec<String>> {
    if !crate::local::has_program("xdg-terminal-exec") {
        return None;
    }
    let mime = output(Command::new("xdg-mime").args(["query", "filetype"]).arg(path))?;
    let desktop = output(Command::new("xdg-mime").args(["query", "default", &mime]))?;
    let text = std::fs::read_to_string(find_desktop_file(&desktop)?).ok()?;
    let (exec, terminal) = parse_entry(&text);
    if !terminal {
        return None;
    }
    Some(expand_exec(&exec?, path))
}

fn find_desktop_file(id: &str) -> Option<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| crate::local::home().join(".local/share"));
    let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    std::iter::once(home).chain(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from)).map(|d| d.join("applications").join(id)).find(|p| p.is_file())
}

/// `Exec=` and `Terminal=` from a desktop entry's main group.
pub fn parse_entry(text: &str) -> (Option<String>, bool) {
    let mut in_main = false;
    let (mut exec, mut terminal) = (None, false);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some(v) = line.strip_prefix("Exec=") {
            exec = Some(v.to_owned());
        } else if let Some(v) = line.strip_prefix("Terminal=") {
            terminal = v.trim() == "true";
        }
    }
    (exec, terminal)
}

/// An `Exec=` line as arguments, with the file in place of `%f %F %u %U` (added at the end
/// when there's none) and the other field codes dropped.
pub fn expand_exec(exec: &str, path: &Path) -> Vec<String> {
    let p = path.to_string_lossy().into_owned();
    let mut out = Vec::new();
    let mut placed = false;
    for word in split(exec) {
        match word.as_str() {
            "%f" | "%F" | "%u" | "%U" => {
                out.push(p.clone());
                placed = true;
            }
            "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
            w => out.push(w.replace("%%", "%")),
        }
    }
    if !placed {
        out.push(p);
    }
    out
}

/// Splits on spaces, honouring double quotes and backslash escapes (the desktop entry spec).
fn split(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted, mut any) = (Vec::new(), String::new(), false, false);
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_lines() {
        let (exec, term) = parse_entry("[Desktop Entry]\nName=Neovim\nExec=nvim %F\nTerminal=true\n[Desktop Action new]\nExec=other\n");
        assert_eq!((exec.as_deref(), term), (Some("nvim %F"), true));
        assert_eq!(expand_exec("nvim %F", Path::new("/a b/c.md")), ["nvim", "/a b/c.md"]);
        assert_eq!(expand_exec("\"my app\" --x %%d %i", Path::new("/f")), ["my app", "--x", "%d", "/f"]);
    }
}
