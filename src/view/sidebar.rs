//! Places on this machine, then servers: `~/.ssh/config` hosts and ones connected to by address.

use rune::El;
use rune_kit::shell::{Sidebar, SidebarRow, SidebarSection};
use rune_kit::{IconButton, IconButtonVariant, Kit};

use crate::app::App;
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
            .selected(current == Some(label))
            .on_select(move |s: &mut App, _| s.go(Location::Local(path.clone())))
            .build(k)
            .key(format!("place-{label}"))
    });

    let host_rows = s.hosts.iter().map(|h| {
        let id = h.id.clone();
        let mut r = SidebarRow::new(h.id.clone()).icon(&icons::SERVER).height(30.0).selected(s.loc.host() == Some(h.id.as_str()));
        if h.home.is_some() {
            let close = id.clone();
            r = r.trailing(rune::icon(&icons::DOT, 8.0).color(k.c.success.icon)).on_close(move |s: &mut App, _| s.disconnect(&close));
        } else if s.connecting.as_deref() == Some(h.id.as_str()) {
            r = r.trailing(rune_kit::Spinner::new().size(12.0).build(k));
        }
        r.on_select(move |s: &mut App, _| s.open_host(&id)).build(k).key(format!("host-{}", h.id))
    });

    let add = IconButton::new(&icons::CONNECT)
        .variant(IconButtonVariant::GhostMuted)
        .label("Connect to server")
        .help("Connect to server (Ctrl+K)")
        .on_press(|s: &mut App| s.open_connect_dialog("", None))
        .build(k)
        .key("add-server");

    Sidebar::new()
        .label("Places")
        .section(SidebarSection::new("Places").key("places").rows(place_rows))
        .section(SidebarSection::new("Servers").key("servers").action(add).rows(host_rows))
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
