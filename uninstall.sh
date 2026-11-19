#!/bin/sh
# Removes what install.sh put in place and gives folders back to Nautilus when it's installed.
set -eu
APP=io.github.amanmprojects.Files
BIN="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"

rm -f "$BIN/files" "$DATA/applications/$APP.desktop" "$DATA/icons/hicolor/scalable/apps/$APP.svg"
rm -f "$DATA/dbus-1/services/org.freedesktop.FileManager1.service"
if [ -f /usr/share/applications/org.gnome.Nautilus.desktop ]; then
  xdg-mime default org.gnome.Nautilus.desktop inode/directory application/x-gnome-saved-search
fi
update-desktop-database "$DATA/applications" 2>/dev/null || true
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 || true
echo "Removed. Default for folders: $(xdg-mime query default inode/directory)"
