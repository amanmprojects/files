#!/bin/sh
# Builds Files and installs it for this user as the desktop's file manager:
#   - the binary in ~/.local/bin, a desktop entry and an icon;
#   - the default app for folders (xdg-mime inode/directory);
#   - org.freedesktop.FileManager1 on D-Bus ("Show in folder" in browsers and editors starts it).
# uninstall.sh undoes all of it.
set -eu
cd "$(dirname "$0")"

APP=io.github.amanmprojects.Files
BIN="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"

cargo build --release

install -Dm755 target/release/files "$BIN/files"
install -Dm644 assets/icon.svg "$DATA/icons/hicolor/scalable/apps/$APP.svg"
mkdir -p "$DATA/applications" "$DATA/dbus-1/services"
sed "s|^Exec=files|Exec=$BIN/files|" "packaging/$APP.desktop" > "$DATA/applications/$APP.desktop"

# D-Bus looks in the user's data dir before /usr/share, so this replaces Nautilus's service.
cat > "$DATA/dbus-1/services/org.freedesktop.FileManager1.service" <<SERVICE
[D-BUS Service]
Name=org.freedesktop.FileManager1
Exec=$BIN/files --service
SERVICE

xdg-mime default "$APP.desktop" inode/directory application/x-gnome-saved-search
update-desktop-database "$DATA/applications" 2>/dev/null || true
gtk-update-icon-cache -q -t "$DATA/icons/hicolor" 2>/dev/null || true
# Let the session bus see the new service file now rather than at next login.
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 || true

echo "Installed: $BIN/files"
echo "Default for folders: $(xdg-mime query default inode/directory)"
