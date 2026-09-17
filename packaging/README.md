# Packaging

Four ways to install this, depending on your distro. **All of them install
the same two programs** to `/usr/bin`:

- `hypr-xp-magnifier` — the magnifier
- `magnifier-settings` — the settings GUI

After installing either way, both also show up in your app launcher.

**Before you pick one, read `COMPATIBILITY.md`.** Installing is the easy
part; whether it runs depends on your *desktop*, not your distro. Short
version: works on Hyprland, should work on Sway/KDE/Cosmic with small
changes, cannot work on GNOME or XFCE.

---

## Arch (this is the one to try first — it's your machine)

```
cd packaging
makepkg -si
```

That builds and installs it. To remove later:

```
sudo pacman -R hypr-xp-magnifier
```

## Debian / Ubuntu / Mint / Pop!_OS

First install the build tools (one time only):

```
sudo apt update
sudo apt install build-essential cargo pkg-config \
    libwayland-dev libxkbcommon-dev libgl1-mesa-dev libegl1-mesa-dev
```

Then build the package:

```
cd packaging
chmod +x build-deb.sh
./build-deb.sh
```

It prints the path to the finished `.deb` at the end. Install it with the
command it gives you, which will look like:

```
sudo apt install ./build/hypr-xp-magnifier_0.1.0_amd64.deb
```

To remove later:

```
sudo apt remove hypr-xp-magnifier
```

## Fedora / openSUSE

```
sudo dnf install cargo rust make pkgconf-pkg-config \
    wayland-devel libxkbcommon-devel mesa-libGL-devel

rpmbuild -bb --build-in-place packaging/hypr-xp-magnifier.spec
```

(Run that last command from the project root, not from `packaging/`.)
It prints where the `.rpm` landed; install it with `sudo dnf install <path>`.

## Any distro — plain install, no package

If your distro isn't listed, or packaging is being annoying:

```
cd packaging
make build
sudo make install
```

To remove:

```
cd packaging
sudo make uninstall
```

This installs to `/usr/local` instead of `/usr`, which is the conventional
place for things your package manager doesn't know about.

---

## A note on what I could and couldn't verify

I wrote these without being able to run them — there's no network or Rust
toolchain in my environment, so none of these have actually been built.

They're much lower-risk than compiled code, though: package files are
mostly declarative, and the formats are stable and well-established. The
likeliest thing to go wrong is a **dependency name being slightly off** for
your distro, since those differ between Debian and Fedora and change
between releases. If a build complains about a missing package, paste the
error and it's usually a one-word fix.

The Arch `PKGBUILD` is the one most likely to work first try, because
that's the distro you're actually on and the dependency names there I'm
most confident about.
