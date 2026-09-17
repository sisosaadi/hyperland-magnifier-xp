# What this runs on

Two separate questions people mix up:

1. **Which distro?** (Debian, Arch, Fedora…) — decides how it *installs*.
   Packaging solves this. Easy.
2. **Which desktop/compositor?** (Hyprland, KDE, GNOME, XFCE, Cosmic…) —
   decides whether it *works at all*. Packaging can't solve this.

The magnifier needs two things from your desktop:

- **Docking** — a way to pin a bar to a screen edge and push other windows
  out of the way. This is the `wlr-layer-shell` protocol.
- **Capture** — a way to read the screen's pixels, fast, every frame.

## The desktop matrix (verified September 2026)

| Desktop | Docking (layer-shell) | Capture | Status |
|---|---|---|---|
| **Hyprland** | yes | yes (wlr-screencopy) | **Works today** — this is the tested build |
| **Sway / river / Wayfire / labwc** | yes | yes (wlr-screencopy) | Should work; needs a small IPC change (see below) |
| **KDE Plasma (KWin)** | yes | yes (ext-image-copy-capture, KWin 6.4+) | Very achievable — see note |
| **Cosmic** | yes (Smithay) | yes (ext-image-copy-capture) | Very achievable — same path as KDE |
| **niri** | yes | yes | Should work, same path |
| **GNOME** | **NO** | yes (Mutter 48+) | **Blocked** — see below |
| **XFCE** | **NO** (X11) | n/a (X11) | Would be a separate program — see below |

## Why GNOME is blocked

GNOME's compositor (Mutter) has declined to implement `wlr-layer-shell` for
years — the request has been open since 2019. Capture is fine on GNOME;
it's the *docking* half that has no path. Without layer-shell there's no way
for a normal app to reserve screen space and push windows around.

A GNOME version wouldn't be a port of this program. It would be a **GNOME
Shell extension**, written in JavaScript, running inside GNOME Shell itself
— which is also exactly how GNOME's own built-in magnifier works. Different
language, different architecture, no shared code with this project.

GNOME does already ship a built-in magnifier with a docked "lens" mode
(Settings → Accessibility → Zoom). If you end up on GNOME, that's the
realistic answer rather than this program.

## Why XFCE is different

XFCE is X11, not Wayland. Neither protocol above exists there. On X11 the
same features are done completely differently: `XComposite`/`XShm` for
capture, and `_NET_WM_STRUT_PARTIAL` for docking. That's a full second
backend — genuinely a separate program sharing only the general idea.

(XFCE has an early Wayland compositor, `xfwl4`, in preview as of 4.21. Too
early to target.)

## The good news: KDE and Cosmic got much easier

The original KDE scaffold in this project used xdg-desktop-portal +
PipeWire, because KWin didn't support `wlr-screencopy`. That was correct
at the time but is now the hard way round.

Since then, a **new standard capture protocol** — `ext-image-copy-capture-v1`
— has been adopted across nearly every compositor: Mutter 48+, KWin 6.4,
Hyprland 0.48+, Cosmic, Sway 1.11, niri, Wayfire, labwc, Weston, Mir.

That means a single capture backend written against `ext-image-copy-capture-v1`
would replace both the wlr-screencopy path *and* the untested portal/PipeWire
scaffold, and cover KDE, Cosmic, Hyprland, Sway and niri at once. No
permission dialog, no PipeWire dependency.

**This is the highest-value next step for this project** — much more so than
more packaging. It would turn three half-projects into one program.

Two caveats, honestly:

- There's still a per-compositor piece that won't generalise: the cursor
  position and focused-window geometry come from *Hyprland's* IPC socket.
  KDE and Cosmic each need their own equivalent, and I don't yet know what
  those look like.
- I can't test any of it here. It'd be the same back-and-forth debugging
  loop as the Hyprland build, on each desktop you actually have.

## Sway and other wlroots compositors

The Wayland half (docking + capture) is already compositor-agnostic within
wlroots. The only Hyprland-specific parts are the two IPC calls: cursor
position, and active window geometry. On Sway those become `swaymsg -t
get_tree` equivalents. Small change, but untested — flag it if you want it.
