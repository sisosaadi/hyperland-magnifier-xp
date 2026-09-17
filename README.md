# hypr-xp-magnifier

A docked screen magnifier for Hyprland, modeled on the Magnifier built into
Windows XP.

It pins a magnified strip to one edge of your screen and pushes your other
windows out of the way, rather than floating on top of them. The strip
shows a live, zoomed view of whatever is under your mouse cursor — and
optionally, of whatever has keyboard focus.

<!-- Add a screenshot here:
![screenshot](docs/screenshot.png)
-->

## Features

- **Docks to any screen edge** — top, bottom, left or right. Other windows
  resize around it instead of being covered.
- **Live view at ~75 fps** on a release build.
- **Follows the mouse cursor** in real time.
- **Optionally follows keyboard focus** — Tab through buttons or type in a
  text field and the view moves with you, via the Linux accessibility bus.
- **Live settings** — a small GUI adjusts zoom, size, position and refresh
  rate while the magnifier is running. No restart.
- **Multi-monitor aware** — clamps to the output the cursor is on.

## Requirements

- **Hyprland.** The Wayland parts (`wlr-layer-shell`, `wlr-screencopy`)
  work on any wlroots compositor, but cursor position and window geometry
  come from Hyprland's IPC socket specifically. See
  [Compatibility](#compatibility) before trying it elsewhere.
- **Rust** (stable) and `cargo`.
- **`at-spi2-core`** — only if you want the follow-keyboard-focus feature.

## Install

### Arch

```sh
cd packaging
makepkg -si
```

### Debian / Ubuntu

```sh
sudo apt install build-essential cargo pkg-config \
    libwayland-dev libxkbcommon-dev libgl1-mesa-dev libegl1-mesa-dev

cd packaging
./build-deb.sh
sudo apt install ./build/hypr-xp-magnifier_0.1.0_amd64.deb
```

### Fedora

```sh
sudo dnf install cargo rust make pkgconf-pkg-config \
    wayland-devel libxkbcommon-devel mesa-libGL-devel

rpmbuild -bb --build-in-place packaging/hypr-xp-magnifier.spec
```

### Any distro, no package

```sh
cd packaging
make build
sudo make install
```

### From source, without installing

```sh
cargo build --release
./target/release/hypr-xp-magnifier
```

> **Always build with `--release`.** A debug build runs at roughly half the
> frame rate and feels noticeably laggy.

## Usage

Start the magnifier:

```sh
hypr-xp-magnifier
```

Adjust it while it runs:

```sh
magnifier-settings
```

Both also appear in your application launcher.

To quit the magnifier: click the bar once to focus it, then press `Escape`
— or `Ctrl+C` in the terminal that started it.

To start it automatically with Hyprland, add to `~/.config/hypr/hyprland.conf`:

```
exec-once = hypr-xp-magnifier
```

## Settings

Settings live in `~/.config/hypr-xp-magnifier/settings.json` and are
re-read a few times a second, so you can edit the file directly if you
prefer — changes apply immediately either way.

| Setting | Default | Range | What it does |
|---|---|---|---|
| `zoom_factor` | `2.5` | 1.0 – 6.0 | How much to magnify |
| `thickness` | `200` | 40 – 800 | Bar height when docked top/bottom, width when docked left/right (px) |
| `position` | `Top` | Top / Bottom / Left / Right | Which edge to dock to |
| `refresh_rate_hz` | `60.0` | 10 – 165 | How often to capture the screen. Lower it to cut CPU use |
| `sharpness` | `1.0` | 0.0 – 1.0 | `0.0` = smooth (bilinear), `1.0` = sharp and blocky (nearest-neighbour) |
| `follow_keypress` | `false` | on/off | Follow keyboard focus as well as the mouse |
| `focus_y_correction` | `40` | -100 – 100 | Vertical nudge for focus tracking — see below |

## Following keyboard focus

With `follow_keypress` enabled, the magnifier tracks whichever moved most
recently: your mouse, or the keyboard focus. Tab between buttons or click
into a text field and the view follows.

This uses **AT-SPI2**, the Linux accessibility bus. Check it's running:

```sh
busctl --user call org.a11y.Bus /org/a11y/bus org.a11y.Bus GetAddress
```

If that prints an address you're set. If it errors, install and start
`at-spi2-registryd`.

Three things worth knowing:

- **It only sees apps that publish accessibility information.** Most
  GTK/Qt/Electron apps do. Custom-drawn UIs and games usually don't.
- **Chromium-based apps need a flag.** Chrome and Electron apps don't build
  their accessibility tree unless started with
  `--force-renderer-accessibility`, or unless a screen reader is already
  running.
- **`focus_y_correction` usually needs tuning by eye.** Toolkits report
  positions relative to their own window *excluding* the title bar, while
  the compositor's window bounds *include* it. The result is focus tracking
  that lands consistently too high or too low by a fixed amount. Nudge the
  slider until it lines up; the default of 40 suits typical GTK apps.

## Compatibility

Installing is the easy part — whether it runs depends on your compositor,
not your distro. The magnifier needs two things: a way to dock a bar and
reserve screen space (`wlr-layer-shell`), and a way to read screen pixels
quickly.

| Desktop | Status |
|---|---|
| **Hyprland** | Works — this is what it's developed and tested on |
| Sway, river, Wayfire, labwc | Wayland side should work; needs the Hyprland IPC calls swapped for that compositor's equivalent |
| KDE Plasma, Cosmic, niri | Achievable — layer-shell is supported, and capture would use `ext-image-copy-capture-v1` instead of wlr-screencopy. Not implemented yet |
| **GNOME** | **Not possible as-is.** Mutter has declined to implement `wlr-layer-shell` since 2019, so no ordinary app can dock and reserve space. A GNOME version would have to be a GNOME Shell extension. GNOME does ship its own docked magnifier under Settings → Accessibility → Zoom |
| **XFCE** and other X11 desktops | **Not supported.** X11 has neither protocol; capture and docking work completely differently there |

## Known limitations

- **Focus tracking centres on the whole field, not the exact character.**
  While typing, the view re-centres on the focused text box rather than
  following the caret. This is a limitation of GTK's accessibility bridge,
  not a missing feature here — both AT-SPI methods for per-character
  position (`GetCharacterExtents` and `GetRangeExtents`) return
  `NotSupported` on every GTK widget tested.
- **Bar size is set via the settings GUI**, not by dragging its edge.
- **Hyprland-specific IPC** for cursor and window position, as described
  above.

## How it works

Wayland deliberately hides global cursor position and window geometry from
ordinary applications, so a few things have to be sourced indirectly:

- **Docking** uses `wlr-layer-shell` with an exclusive zone, which is what
  makes other windows move aside rather than sit underneath.
- **Capture** uses `wlr-screencopy`, reusing a cached buffer between frames
  and skipping capture entirely when the tracked point is over the bar
  itself (otherwise you get an infinite mirror).
- **Cursor position** comes from Hyprland's IPC socket, since Wayland won't
  tell a client where the pointer is globally.
- **Focus position** combines two sources: AT-SPI gives coordinates
  relative to the focused app's own window, and Hyprland's IPC gives that
  window's position on screen. Adding them produces absolute coordinates.

GNOME's magnifier and Windows Magnifier don't need any of this, because
they're privileged: GNOME's runs inside the compositor, and Windows' UI
Automation hands out absolute screen coordinates by design. This one is an
ordinary unprivileged client, which is why the workarounds exist.

## License

MIT — see [LICENSE](LICENSE).
