#!/usr/bin/env bash
#
# Builds a .deb you can install with `sudo apt install ./hypr-xp-magnifier_*.deb`
#
# Run it from this packaging/ directory:
#     ./build-deb.sh
#
# This is the pragmatic route: build with cargo, lay the files out in a
# staging directory, then hand that to dpkg-deb. It needs no debhelper and
# no Rust-crates-as-Debian-packages, which is what makes official Debian
# Rust packaging painful. The result is a perfectly normal .deb — it just
# isn't suitable for upload to Debian proper. For personal use that's fine.

set -euo pipefail

VERSION="0.1.0"
ARCH="$(dpkg --print-architecture)"
PKGNAME="hypr-xp-magnifier"
STAGING="$(pwd)/build/${PKGNAME}_${VERSION}_${ARCH}"

echo "==> Cleaning any previous staging directory"
rm -rf "$(pwd)/build"
mkdir -p "$STAGING/DEBIAN"

echo "==> Building release binaries (this takes a few minutes the first time)"
make build

echo "==> Installing into staging directory"
make install DESTDIR="$STAGING" PREFIX=/usr

echo "==> Writing package metadata"
INSTALLED_SIZE="$(du -sk "$STAGING" | cut -f1)"

cat > "$STAGING/DEBIAN/control" <<EOF
Package: $PKGNAME
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Installed-Size: $INSTALLED_SIZE
Depends: libc6, libwayland-client0, libxkbcommon0, libgl1, libegl1, libdbus-1-3, fontconfig
Recommends: at-spi2-core
Maintainer: Mohamed <mohamed@localhost>
Description: Docked screen magnifier in the style of Windows XP
 A docked magnifier bar that pins to a screen edge and pushes other
 windows aside, mirroring the behaviour of the Magnifier built into
 Windows XP. Follows the mouse cursor and, optionally, keyboard focus.
 .
 Requires a Wayland compositor supporting wlr-layer-shell and
 wlr-screencopy. Tested on Hyprland. Does NOT work on GNOME (no
 layer-shell support) or on X11 desktops such as XFCE. See
 /usr/share/doc/hypr-xp-magnifier/COMPATIBILITY.md
EOF

# Register the settings file as a conffile-free user config — nothing to do
# here, since settings live in the user's ~/.config and are created on first
# run. Listed for clarity that this is deliberate, not an oversight.

echo "==> Building the .deb"
dpkg-deb --root-owner-group --build "$STAGING"

OUTPUT="$(pwd)/build/${PKGNAME}_${VERSION}_${ARCH}.deb"
echo
echo "Done: $OUTPUT"
echo
echo "Install it with:"
echo "    sudo apt install $OUTPUT"
echo
echo "Remove it later with:"
echo "    sudo apt remove $PKGNAME"
