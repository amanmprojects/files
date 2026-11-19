//! Connect, trust a host key, new folder, rename, delete and name conflicts.

use rune::{El, TextEdit, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, Dialog as KitDialog, Kit, Spinner, TextInput};

use crate::app::{App, Dialog, Resolve, plural};

fn close(s: &mut App) {
    s.dialog = None;
}

fn label(k: &Kit, t: &str) -> El {
    text(t.to_owned()).font(k.ty.small_medium).color(k.c.text_muted)
}

fn error(k: &Kit, e: &Option<String>) -> Option<El> {
    e.as_ref().map(|e| text(e.clone()).key("dialog-error").font(k.ty.small).color(k.c.danger.icon).wrap())
}

/// A dialog's body, inset as the title is.
fn body() -> El {
    col().w_full().px(16.0).pb(16.0).pt(4.0)
}

fn actions(items: Vec<El>) -> El {
    items.into_iter().fold(row().w_full().gap(8.0).justify_end().pt(8.0), |r, b| r.child(b))
}

fn cancel(k: &Kit) -> El {
    Button::new("Cancel").variant(ButtonVariant::Ghost).on_press(close).build(k).key("cancel")
}

/// The open dialog's field, for `on_edit`: the dialog can change under a queued edit, so a
/// stale one lands in a spare.
fn field(s: &mut App, which: u8) -> &mut TextEdit {
    let spare = &mut s.filter;
    match &mut s.dialog {
        Some(Dialog::Connect { server, .. }) if which == 0 => server,
        Some(Dialog::Connect { password, .. }) if which == 1 => password,
        Some(Dialog::Connect { key, .. }) if which == 2 => key,
        Some(Dialog::NewFolder { name, .. } | Dialog::Rename { name, .. }) => name,
        _ => spare,
    }
}

pub fn build(s: &App, k: &Kit, layer: &mut Vec<El>, window: [f32; 2]) {
    let height = window[1];
    let Some(d) = &s.dialog else { return };
    let dialog = match d {
        Dialog::Connect { server, password, key, error: err } => {
            let busy = s.connecting.is_some();
            let mut body = col()
                .w_full()
                .gap(6.0)
                .child(label(k, "Server"))
                .child(
                    TextInput::new(server)
                        .placeholder("user@host, host:2222 or an ~/.ssh/config alias")
                        .label("Server")
                        .on_edit(|s: &mut App| field(s, 0))
                        .on_submit(|s: &mut App, _: &mut Tx| s.submit_connect())
                        .build(k)
                        .key("server")
                        .w_full(),
                )
                .child(label(k, "Password").pt(6.0))
                .child(
                    TextInput::new(password)
                        .secure()
                        .placeholder("Leave empty to use your SSH keys")
                        .label("Password")
                        .on_edit(|s: &mut App| field(s, 1))
                        .on_submit(|s: &mut App, _: &mut Tx| s.submit_connect())
                        .build(k)
                        .key("password")
                        .w_full(),
                )
                .child(label(k, "Private key file").pt(6.0))
                .child(
                    TextInput::new(key)
                        .placeholder("Optional, such as ~/.ssh/server.pem")
                        .label("Private key file")
                        .on_edit(|s: &mut App| field(s, 2))
                        .on_submit(|s: &mut App, _: &mut Tx| s.submit_connect())
                        .build(k)
                        .key("identity")
                        .w_full(),
                );
            if let Some(e) = error(k, err) {
                body = body.child(e.pt(4.0));
            }
            let mut buttons = vec![cancel(k)];
            if busy {
                buttons.insert(0, Spinner::new().size(14.0).build(k));
            }
            buttons.push(
                Button::new("Connect")
                    .variant(ButtonVariant::Contrast)
                    .disabled(busy)
                    .on_press(|s: &mut App| s.submit_connect())
                    .build(k)
                    .key("connect-submit")
                    .default_button(),
            );
            KitDialog::new("connect-dialog", "Connect to Server")
                .description("Browse a server over SSH. Files copy to and from it with scp.")
                .width(440.0)
                .child(self::body().child(body.child(actions(buttons))))
        }
        Dialog::Trust { target, fingerprint, .. } => {
            let fp: El = match fingerprint {
                Some(f) => text(f.clone()).key("fingerprint").font(k.ty.mono).color(k.c.text_base).wrap(),
                None => row()
                    .gap(8.0)
                    .center_y()
                    .child(Spinner::new().size(12.0).build(k))
                    .child(text("Reading the host key…").font(k.ty.small).color(k.c.text_muted)),
            };
            KitDialog::new("trust-dialog", format!("Trust {}?", target.id())).width(460.0).child(
                self::body()
                    .gap(10.0)
                    .child(
                        text("This is the first connection to this server. Check that its key fingerprint matches the one its owner gave you.")
                            .font(k.ty.body)
                            .color(k.c.text_muted)
                            .wrap(),
                    )
                    .child(col().w_full().padding(10.0).radius(k.radius.md).fill(k.c.layer_02).child(fp))
                    .child(actions(vec![
                        cancel(k),
                        Button::new("Trust and Connect")
                            .variant(ButtonVariant::Contrast)
                            .on_press(|s: &mut App| s.trust_and_connect())
                            .build(k)
                            .key("trust")
                            .default_button(),
                    ])),
            )
        }
        Dialog::NewFolder { name, error: err, file } => {
            let mut body = col().w_full().gap(6.0).child(
                TextInput::new(name)
                    .label(if *file { "File name" } else { "Folder name" })
                    .on_edit(|s: &mut App| field(s, 3))
                    .on_submit(|s: &mut App, _: &mut Tx| s.submit_new_folder())
                    .build(k)
                    .key("folder-name")
                    .w_full(),
            );
            if let Some(e) = error(k, err) {
                body = body.child(e);
            }
            KitDialog::new("new-folder-dialog", if *file { "New File" } else { "New Folder" }).width(380.0).child(self::body().child(body.child(actions(
                vec![
                cancel(k),
                Button::new("Create").variant(ButtonVariant::Contrast).on_press(|s: &mut App| s.submit_new_folder()).build(k).key("create").default_button(),
            ],
            ))))
        }
        Dialog::Rename { from, name, error: err } => {
            let mut body = col().w_full().gap(6.0).child(
                TextInput::new(name)
                    .label("New name")
                    .on_edit(|s: &mut App| field(s, 3))
                    .on_submit(|s: &mut App, _: &mut Tx| s.submit_rename())
                    .build(k)
                    .key("rename-field")
                    .w_full(),
            );
            if let Some(e) = error(k, err) {
                body = body.child(e);
            }
            KitDialog::new("rename-dialog", format!("Rename “{from}”")).width(400.0).child(self::body().child(body.child(actions(vec![
                cancel(k),
                Button::new("Rename").variant(ButtonVariant::Contrast).on_press(|s: &mut App| s.submit_rename()).build(k).key("rename").default_button(),
            ]))))
        }
        Dialog::Delete { names, trash } => {
            let n = names.len();
            let what = if n == 1 { format!("“{}”", names[0]) } else { format!("{n} items") };
            let names = names.clone();
            let trash = *trash;
            let why = if s.loc.is_remote() {
                "This deletes it from the server for good. It can't be undone."
            } else {
                "This deletes it for good instead of moving it to the Trash. It can't be undone."
            };
            KitDialog::new("delete-dialog", format!("Delete {what}?")).width(420.0).child(
                self::body().gap(4.0).child(text(why).font(k.ty.body).color(k.c.text_muted).wrap()).child(actions(vec![
                    cancel(k),
                    Button::new("Delete")
                        .variant(ButtonVariant::Danger)
                        .on_press(move |s: &mut App| s.delete(names.clone(), trash))
                        .build(k)
                        .key("confirm-delete")
                        .default_button(),
                ])),
            )
        }
        Dialog::Conflict { conflicts, .. } => {
            let n = conflicts.len();
            let mut list = col().w_full().gap(2.0);
            for c in conflicts.iter().take(5) {
                list = list.child(text(c.clone()).font(k.ty.small).color(k.c.text_base).truncate());
            }
            if n > 5 {
                list = list.child(text(format!("and {} more", n - 5)).font(k.ty.small).color(k.c.text_faint));
            }
            let pick = |label: &str, key: &str, v: ButtonVariant, how: Option<Resolve>| {
                let b = Button::new(label).variant(v).on_press(move |s: &mut App| s.resolve_conflict(how)).build(k).key(key.to_owned());
                // Return keeps both: the choice that loses nothing.
                if how == Some(Resolve::KeepBoth) { b.default_button() } else { b }
            };
            KitDialog::new("conflict-dialog", format!("{n} item{} already exist{} here", plural(n), if n == 1 { "s" } else { "" }))
                .description("Replace them, keep both (the new ones get “(copy)” names), or skip them.")
                .width(460.0)
                .child(self::body().gap(10.0).child(list).child(actions(vec![
                    pick("Cancel", "conflict-cancel", ButtonVariant::Ghost, None),
                    pick("Skip", "skip", ButtonVariant::Outline, Some(Resolve::Skip)),
                    pick("Keep Both", "keep-both", ButtonVariant::Outline, Some(Resolve::KeepBoth)),
                    pick("Replace", "replace", ButtonVariant::Danger, Some(Resolve::Replace)),
                ])))
        }
        Dialog::Properties { names } => properties(s, k, names),
        Dialog::Shortcuts => shortcuts(k, height),
        Dialog::QuickLook => super::preview::quick_look(s, k, window[0], height),
    };
    dialog.fit().open(true).on_close(close).build(k, layer);
}

fn field_row(k: &Kit, label: &str, value: String) -> El {
    row()
        .w_full()
        .gap(12.0)
        .child(text(label.to_owned()).font(k.ty.small).color(k.c.text_faint).w(96.0))
        .child(text(value).font(k.ty.small).color(k.c.text_base).wrap().selectable(true).grow(1.0).shrink().min_w(0.0))
}

/// Name, kind, size, dates and permissions of the selection (or of this folder).
fn properties(s: &App, k: &Kit, names: &[String]) -> KitDialog {
    use crate::model::{format_time, human_size, kind_label, mode_string};
    let entries: Vec<&crate::model::Entry> = names.iter().filter_map(|n| s.entry(n)).collect();
    let mut body = self::body().gap(8.0);
    let title = match entries.as_slice() {
        [e] => {
            let path = s.loc_label(&s.loc.join(&e.name));
            body = body.child(field_row(k, "Kind", kind_label(e)));
            if let Some(b) = e.size {
                body = body.child(field_row(k, "Size", format!("{} ({b} bytes)", human_size(b))));
            }
            if let Some(t) = e.modified {
                body = body.child(field_row(k, "Modified", format_time(t, s.now, s.utc_offset)));
            }
            if let Some(m) = e.mode {
                body = body.child(field_row(k, "Permissions", format!("{} ({m:o})", mode_string(m))));
            }
            if e.link {
                body =
                    body.child(field_row(k, "Link", if e.kind == crate::model::Kind::Other { "Broken symbolic link".into() } else { "Symbolic link".into() }));
            }
            body = body.child(field_row(k, "Location", path));
            e.name.clone()
        }
        [] => {
            let n = s.entries.len();
            body = body.child(field_row(k, "Location", s.loc_label(&s.loc))).child(field_row(k, "Contains", format!("{n} item{}", plural(n))));
            s.loc.name()
        }
        many => {
            let files = many.iter().filter(|e| !e.is_dir()).count();
            let bytes: u64 = many.iter().filter_map(|e| e.size).sum();
            body = body
                .child(field_row(k, "Contains", format!("{files} file{}, {} folder{}", plural(files), many.len() - files, plural(many.len() - files))))
                .child(field_row(k, "Size", format!("{} of files (folders not counted)", human_size(bytes))))
                .child(field_row(k, "Location", s.loc_label(&s.loc)));
            format!("{} items", many.len())
        }
    };
    let done = Button::new("Close").variant(ButtonVariant::Contrast).on_press(close).build(k).key("close-properties").default_button();
    KitDialog::new("properties-dialog", title).width(440.0).child(body.child(actions(vec![done])))
}

/// Every shortcut, from `keys::SHORTCUTS`, in two columns.
fn shortcuts(k: &Kit, height: f32) -> KitDialog {
    let section = |(title, rows): &(&str, &[(&str, &str)])| {
        rows.iter().fold(
            col().w_full().gap(5.0).pb(14.0).child(text(title.to_string()).font(k.ty.ui_medium).color(k.c.text_base).pb(2.0)),
            |c, (keys, what)| {
                c.child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .child(text(keys.to_string()).font(k.ty.small_medium).color(k.c.text_base).w(176.0).wrap())
                        .child(text(what.to_string()).font(k.ty.small).color(k.c.text_muted).wrap().grow(1.0).shrink().min_w(0.0)),
                )
            },
        )
    };
    let all = crate::keys::SHORTCUTS;
    let half = all.len().div_ceil(2);
    let column = |part: &[(&str, &[(&str, &str)])]| part.iter().fold(col().grow(1.0).shrink().min_w(0.0), |c, sct| c.child(section(sct)));
    let grid = row().w_full().gap(28.0).child(column(&all[..half])).child(column(&all[half..]));
    let done = Button::new("Close").variant(ButtonVariant::Contrast).on_press(close).build(k).key("close-shortcuts").default_button();
    KitDialog::new("shortcuts-dialog", "Keyboard Shortcuts").width(820.0).child(
        self::body()
            .child(col().w_full().h((height - 190.0).clamp(160.0, 640.0)).scroll(rune::ScrollSpec { stick_bottom: false }).child(grid))
            .child(actions(vec![done])),
    )
}
