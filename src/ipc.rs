//! The desktop's ways in, over the session D-Bus:
//!
//! - One instance: a second `files PATH` hands its locations to the running window (as new
//!   tabs) through `io.github.amanmprojects.Files.Open` and exits.
//! - `org.freedesktop.FileManager1` (`ShowFolders`, `ShowItems`, `ShowItemProperties`): what
//!   browsers, editors and portals call for "Show in folder". Taken over from another file
//!   manager while this one runs; D-Bus starts this one for it when none does (see the
//!   service file the install script writes).

use std::path::PathBuf;

use rune::AppHandle;
use zbus::fdo::RequestNameFlags;

use crate::app::App;

pub const APP_ID: &str = "io.github.amanmprojects.Files";
const PATH: &str = "/io/github/amanmprojects/Files";
const FM1: &str = "org.freedesktop.FileManager1";
const FM1_PATH: &str = "/org/freedesktop/FileManager1";

/// How an outside request wants a location shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    /// Open the folder.
    Folder,
    /// Open the folder holding it, with it selected.
    Item,
    /// As `Item`, then its properties.
    Properties,
}

/// A `file://` URI as a path (percent-decoded); anything else (a path, `ssh://…`) unchanged.
pub fn uri_to_arg(s: &str) -> String {
    let Some(rest) = s.strip_prefix("file://") else { return s.to_owned() };
    // `file://host/path`: only the local host is ours.
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(i) if matches!(&rest[..i], "localhost") => &rest[i..],
        _ => rest,
    };
    percent_decode(path)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn runtime() -> Option<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().ok()
}

/// Hands `targets` to an instance that's already running; true when one took them.
pub fn handoff(targets: &[String]) -> bool {
    let Some(rt) = runtime() else { return false };
    let token = std::env::var("XDG_ACTIVATION_TOKEN").unwrap_or_default();
    rt.block_on(async {
        let Ok(conn) = zbus::Connection::session().await else { return false };
        let body = (targets.to_vec(), "folder", token);
        let call = conn.call_method(Some(APP_ID), PATH, Some(APP_ID), "Open", &body);
        matches!(tokio::time::timeout(std::time::Duration::from_secs(2), call).await, Ok(Ok(_)))
    })
}

struct Ours {
    app: AppHandle<App>,
}

#[zbus::interface(name = "io.github.amanmprojects.Files")]
impl Ours {
    /// Opens locations (paths, `file://` or `ssh://` URIs) in new tabs. `how`: `folder`,
    /// `item` or `properties`. `token`: an XDG activation token, to raise the window.
    async fn open(&self, targets: Vec<String>, how: String, token: String) {
        let how = match how.as_str() {
            "item" => Show::Item,
            "properties" => Show::Properties,
            _ => Show::Folder,
        };
        deliver(&self.app, targets, how, token);
    }
}

struct FileManager1 {
    app: AppHandle<App>,
}

#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl FileManager1 {
    async fn show_folders(&self, uris: Vec<String>, startup_id: String) {
        deliver(&self.app, uris, Show::Folder, startup_id);
    }
    async fn show_items(&self, uris: Vec<String>, startup_id: String) {
        deliver(&self.app, uris, Show::Item, startup_id);
    }
    async fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
        deliver(&self.app, uris, Show::Properties, startup_id);
    }
}

fn deliver(app: &AppHandle<App>, targets: Vec<String>, how: Show, token: String) {
    let targets: Vec<String> = targets.iter().map(|t| uri_to_arg(t)).collect();
    app.update(move |s, _tx| s.open_targets(&targets, how));
    if token.is_empty() { app.open() } else { app.activate(token) }
}

/// Serves both interfaces on a thread for as long as the app runs.
pub fn serve(app: AppHandle<App>) {
    std::thread::Builder::new()
        .name("dbus".into())
        .spawn(move || {
            let Some(rt) = runtime() else { return };
            rt.block_on(async move {
                let built = zbus::connection::Builder::session()
                    .name(APP_ID)
                    .serve_at(PATH, Ours { app: app.clone() })
                    .serve_at(FM1_PATH, FileManager1 { app: app.clone() })
                    .build()
                    .await;
                // Another instance owns the name (a `--new-instance` window): serve nothing.
                let Ok(conn) = built else { return };
                // The desktop's file manager while this runs; another may take it back later.
                let flags = RequestNameFlags::REPLACE_EXISTING | RequestNameFlags::ALLOW_REPLACEMENT | RequestNameFlags::DO_NOT_QUEUE;
                let _ = conn.request_name_with_flags(FM1, flags).await;
                std::future::pending::<()>().await;
            });
        })
        .ok();
}

/// A location argument as the app opens it: a path (absolute or relative to here) or a server.
pub fn resolve(arg: &str) -> Option<(PathBuf, bool)> {
    let arg = uri_to_arg(arg);
    if arg.contains("://") || (!arg.starts_with('/') && !arg.starts_with('.') && !arg.starts_with('~') && arg.contains(':')) {
        return None;
    }
    let p = std::fs::canonicalize(&arg).unwrap_or_else(|_| PathBuf::from(&arg));
    let is_dir = p.is_dir();
    Some((p, is_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris() {
        assert_eq!(uri_to_arg("file:///home/a/My%20Files/x%23y"), "/home/a/My Files/x#y");
        assert_eq!(uri_to_arg("file://localhost/etc"), "/etc");
        assert_eq!(uri_to_arg("/plain/path"), "/plain/path");
        assert_eq!(uri_to_arg("ssh://u@h/srv"), "ssh://u@h/srv");
    }
}
