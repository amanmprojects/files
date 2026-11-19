//! Files: a native Linux file explorer built with Rune. Browses local folders and SSH servers;
//! copies between them with scp.
//!
//! `files [PATH | user@host:/path | ssh://user@host:port/path]`

mod app;
mod archive;
mod icons;
mod ipc;
mod jobs;
mod keys;
mod local;
mod model;
mod open;
mod ops;
mod preview;
mod ssh;
mod term;
mod theme;
mod view;

#[cfg(test)]
mod remote_tests;

use rune::{AppSpec, Tx};

use app::App;

/// Locations named on the command line (paths, `file://` or `ssh://` URIs, `user@host:/path`).
fn targets() -> Vec<String> {
    rune::extra_args().iter().filter(|a| !a.starts_with('-')).cloned().collect()
}

fn init() -> App {
    let mut app = App::new(None);
    app.open_targets(&targets(), ipc::Show::Folder);
    app
}

fn setup(s: &mut App, waker: rune::Waker) {
    s.set_waker(waker);
}

fn poll(s: &mut App, t: f64, tx: &mut Tx) -> bool {
    let changed = s.poll(t);
    if let Some(key) = s.focus_req.take() {
        tx.focus(key, false);
    }
    // The window's title names the folder, as the taskbar and window switchers show it.
    let title = format!("{} — Files", s.loc.name());
    if title != s.title {
        tx.set_title(title.clone());
        s.title = title;
    }
    changed
}

/// `SSH_ASKPASS` mode: ssh runs this binary to ask for the password it was given. A host-key
/// question gets "no": hosts are trusted only through the app's own dialog.
fn askpass() -> bool {
    let Ok(secret) = std::env::var(ssh::ASKPASS_ENV) else { return false };
    let prompt = std::env::args().nth(1).unwrap_or_default();
    use std::io::Write;
    let mut out = std::io::stdout();
    if prompt.contains("continue connecting") || prompt.contains("fingerprint") {
        let _ = writeln!(out, "no");
    } else {
        let _ = writeln!(out, "{secret}");
    }
    true
}

/// Named starting states for headless snapshots (`--scene`).
type Scene = (&'static str, fn(&mut App));

static SCENES: &[Scene] = &[
    ("transfers", |s| s.show_transfers()),
    ("preview", |s| s.toggle_side(app::Side::Preview)),
    ("connect", |s| s.open_connect_dialog("", None)),
    ("hidden", |s| s.toggle_hidden()),
];

fn main() {
    if askpass() {
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Headless runs (snapshots, scripts) stand alone: no handoff, no D-Bus names.
    const HEADLESS: [&str; 9] = ["--snap", "--script", "--tree", "--trace", "--video", "--out", "--list-scenes", "--strip", "--first-pixels"];
    let headless = args.iter().any(|a| HEADLESS.contains(&a.as_str()));
    let alone = headless || args.iter().any(|a| a == "--new-instance");
    // Already running: hand it the locations (it opens them as tabs) and stop here.
    if !alone && ipc::handoff(&targets()) {
        return;
    }
    rune::set_app_id(ipc::APP_ID);
    rune::prewarm_gpu();
    let app = rune::App::new(AppSpec {
        min_size: Some((640.0, 420.0)),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(keys::on_key),
        scenes: SCENES,
        // Monospace text (the terminal, sizes, code previews) in the system's monospace font:
        // fontconfig's, which Omarchy's font setting chooses (a Nerd Font, so icons draw).
        families: ("Inter", "ui-monospace"),
        ..AppSpec::new("Files", (1100.0, 700.0), init, view::view)
    });
    if !alone {
        ipc::serve(app.handle());
    }
    app.run();
}
