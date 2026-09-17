# RPM spec for Fedora / openSUSE / RHEL-likes.
#
# Quick local build (from the project's parent directory):
#     rpmbuild -bb --build-in-place packaging/hypr-xp-magnifier.spec
#
# --build-in-place skips the usual tarball-in-SOURCES dance and builds
# straight from the working tree, which is what you want for a personal
# package.

Name:           hypr-xp-magnifier
Version:        0.1.0
Release:        1%{?dist}
Summary:        Docked screen magnifier in the style of Windows XP's Magnifier

License:        MIT
URL:            https://github.com/yourname/hypr-xp-magnifier

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  pkgconfig
BuildRequires:  make
BuildRequires:  pkgconfig(wayland-client)
BuildRequires:  pkgconfig(xkbcommon)

Requires:       wayland
Requires:       libxkbcommon
Recommends:     at-spi2-core

%description
A docked magnifier bar that pins to a screen edge and pushes other windows
aside, mirroring the behaviour of the Magnifier built into Windows XP.
Follows the mouse cursor and, optionally, keyboard focus.

Requires a Wayland compositor supporting wlr-layer-shell and
wlr-screencopy. Tested on Hyprland. Does not work on GNOME (which does not
implement layer-shell) or on X11 desktops such as XFCE.

%build
cargo build --release

%install
make -C packaging install DESTDIR=%{buildroot} PREFIX=%{_prefix}

%files
%{_bindir}/hypr-xp-magnifier
%{_bindir}/magnifier-settings
%{_datadir}/applications/hypr-xp-magnifier.desktop
%{_datadir}/applications/magnifier-settings.desktop
%doc %{_datadir}/doc/hypr-xp-magnifier/README.md
%doc %{_datadir}/doc/hypr-xp-magnifier/COMPATIBILITY.md

%changelog
* Wed Sep 16 2026 Mohamed <mohamed@localhost> - 0.1.0-1
- Initial package
