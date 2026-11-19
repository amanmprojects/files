//! Places on this machine, then servers: `~/.ssh/config` hosts and ones connected to by address.

use rune::{El, PointerPress, Tx};
use rune_kit::shell::{Sidebar, SidebarRow, SidebarSection};
use rune_kit::{IconButton, IconButtonVariant, Kit};

use crate::app::{App, Host, MenuFor};
use crate::icons;
use crate::model::Location;

pub fn build(s: &App, k: &Kit) -> El {
    let places = s.places.clone();
    // The place the current folder is in: the longest one that contains it.
    let current = match &s.loc {
        Location::Local(p) => places.iter().filter(|(_, _, root)| p.starts_with(root)).max_by_key(|(_, _, root)| root.as_os_str().len()).map(|(l, _, _)| *l),
        Location::Remote { .. } | Location::Archive { .. } => None,
    };
    let place_rows = places.into_iter().map(|(label, icon, path)| {
        SidebarRow::new(label)
            .icon(icon)
            .height(30.0)
            .selected(current == Some(label) || drop_on(s, &format!("place-{label}")))
            .on_select(move |s: &mut App, _| s.go(Location::Local(path.clone())))
            .build(k)
            .key(format!("place-{label}"))
    });

    // Pinned servers first, in pin order (drag to reorder), then connected ones; the rest go
    // in "More servers", folded away once anything is pinned.
    let pinned: Vec<&Host> = s.pinned.iter().filter_map(|id| s.hosts.iter().find(|h| &h.id == id)).collect();
    let connected: Vec<&Host> = s.hosts.iter().filter(|h| !s.is_pinned(&h.id) && (h.home.is_some() || s.loc.host() == Some(h.id.as_str()))).collect();
    let rest: Vec<&Host> = s.hosts.iter().filter(|h| !s.is_pinned(&h.id) && !connected.iter().any(|c| c.id == h.id)).collect();
    let n_pinned = pinned.len();
    let mut top: Vec<El> = pinned.iter().enumerate().map(|(i, h)| host_row(s, k, h, Some((i, n_pinned)))).collect();
    top.extend(connected.iter().map(|h| host_row(s, k, h, None)));
    let more: Vec<El> = rest.iter().map(|h| host_row(s, k, h, None)).collect();

    let add = IconButton::new(&icons::CONNECT)
        .variant(IconButtonVariant::GhostMuted)
        .label("Connect to server")
        .help("Connect to server (Ctrl+K)")
        .on_press(|s: &mut App| s.open_connect_dialog("", None))
        .build(k)
        .key("add-server");

    let mut sidebar = Sidebar::new().label("Places").section(SidebarSection::new("Places").key("places").rows(place_rows)).section(
        SidebarSection::new("Servers").key("servers").action(add).rows(if n_pinned == 0 {
            top.into_iter().chain(more.clone()).collect::<Vec<_>>()
        } else {
            top
        }),
    );
    if n_pinned > 0 && !more.is_empty() {
        sidebar = sidebar.section(
            SidebarSection::new("More servers")
                .key("more-servers")
                .count(more.len())
                .collapsed(!s.more_servers_open)
                .on_toggle(|s: &mut App, _| s.more_servers_open = !s.more_servers_open)
                .rows(more),
        );
    }
    sidebar
        .footer(
            SidebarRow::new("Connect to Server…")
                .icon(&icons::CONNECT)
                .height(30.0)
                .on_select(|s: &mut App, _| s.open_connect_dialog("", None))
                .build(k)
                .key("connect"),
        )
        .build(k)
}

/// A server's row: pinned ones show a pin and can be dragged among the pins (`pin`: its
/// index and the pin count); a dot marks a live connection; right-click for its menu.
fn host_row(s: &App, k: &Kit, h: &Host, pin: Option<(usize, usize)>) -> El {
    let id = h.id.clone();
    let mut r = SidebarRow::new(h.id.clone())
        .icon(if pin.is_some() { &icons::PIN } else { &icons::SERVER })
        .height(30.0)
        .selected(s.loc.host() == Some(h.id.as_str()) || drop_on(s, &format!("host-{}", h.id)));
    if h.home.is_some() {
        let close = id.clone();
        r = r.trailing(rune::icon(&icons::DOT, 8.0).color(k.c.success.icon)).on_close(move |s: &mut App, _| s.disconnect(&close));
    } else if s.connecting.as_deref() == Some(h.id.as_str()) {
        r = r.trailing(rune_kit::Spinner::new().size(12.0).build(k));
    }
    if let Some((i, n)) = pin {
        r = r.reorder(&s.pin_reorder, i, n, |s: &mut App| &mut s.pin_reorder, |s: &mut App, from, to| s.move_pin(from, to));
    }
    let menu_id = id.clone();
    r.on_select(move |s: &mut App, _| s.open_host(&id)).build(k).key(format!("host-{}", h.id)).on_secondary_press(
        move |s: &mut App, p: PointerPress, _tx: &mut Tx| {
            s.menu_host = menu_id.clone();
            s.menu = Some((p, MenuFor::Host));
        },
    )
}

/// The right-click menu of a server in the sidebar.
pub fn host_menu(s: &App, menu: rune_kit::Menu) -> rune_kit::Menu {
    use rune_kit::MenuItem;
    let id = s.menu_host.clone();
    let Some(h) = s.hosts.iter().find(|h| h.id == id) else { return menu };
    let connected = h.home.is_some();
    let pinned = s.is_pinned(&id);
    let close = |f: fn(&mut App, &str)| {
        let id = id.clone();
        move |s: &mut App| {
            s.menu = None;
            f(s, &id);
        }
    };
    let mut m = menu
        .item(MenuItem::new(if connected { "Open" } else { "Connect" }).on_select(close(|s, id| s.open_host(id))))
        .item(MenuItem::new("Open in New Tab").disabled(!connected).on_select(close(|s, id| {
            if let Some(h) = s.hosts.iter().find(|h| h.id == id) {
                let path = h.last.clone().or_else(|| h.home.clone()).unwrap_or_else(|| "/".into());
                s.new_tab(Some(crate::model::Location::Remote { host: id.to_owned(), path }));
            }
        })))
        .separator()
        .item(MenuItem::new(if pinned { "Unpin" } else { "Pin to Top" }).icon(&icons::PIN).on_select(close(|s, id| s.toggle_pin(id))));
    let copy_id = id.clone();
    m = m.item(MenuItem::new("Copy Address").on_select_tx(move |s: &mut App, tx: &mut Tx| {
        s.menu = None;
        tx.copy(copy_id.clone());
    }));
    if connected {
        m = m.item(MenuItem::new("Disconnect").on_select(close(|s, id| s.disconnect(id))));
    }
    if !h.from_config {
        m = m.separator().item(MenuItem::new("Forget Server").danger(true).on_select(close(|s, id| s.forget_host(id))));
    }
    m
}

/// A drag of files would drop on the element keyed `key`.
fn drop_on(s: &App, key: &str) -> bool {
    s.file_drag.as_ref().and_then(|d| d.target.as_ref()).is_some_and(|t| t.key == key)
}
