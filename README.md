THIS ENTIRE APP IS DONT USING CLAUDE I HAVE 0 CODING SKILLS

# hypr-xp-magnifier — phase 4

Two programs, in one project:

- `hypr-xp-magnifier` — the magnifier itself.
- `magnifier-settings` — a GUI that edits the magnifier's settings live via
  a shared file at `~/.config/hypr-xp-magnifier/settings.json`.

New this round: **follow keyboard focus** (e.g. Tab-highlighted buttons),
via Linux's accessibility stack (AT-SPI2) instead of more Wayland/screencopy
work — there's no Wayland-level way to ask "what's focused in some other
app."

## Before building: check the prerequisite

This feature needs the AT-SPI accessibility bus running. Check with:

```
busctl --user call org.a11y.Bus /org/a11y/bus org.a11y.Bus GetAddress
```

If that prints an address, you're set. If it errors, "follow keyboard
focus" can't work until `at-spi2-registryd` is installed/running — that's a
system setup issue, not something in this code.

Also worth knowing: this only sees focus changes in apps that publish
accessibility info. Most GTK/Qt/Electron/browser apps do; custom-drawn UIs
and games often don't.

## Build and run

This is the least-verified part of the whole project — the `atspi` Rust
crate's API has shifted across versions in ways I could only partially
confirm without a live system to test against. **If it doesn't compile,
that's expected — paste the full error back and we'll fix it.** The most
likely spots, if something's wrong, are the `use atspi::...` import lines
near the top of `src/focus_tracker.rs`.

```
cargo build --release
cargo run --release --bin hypr-xp-magnifier
```

```
cargo run --release --bin magnifier-settings
```

## What's new in the GUI

A checkbox: **"Follow keyboard focus (Tab, etc.)"**. When on, if the
keyboard-focused element changes more recently than the mouse last moved,
the magnifier centers on that element instead of the cursor. Move the
mouse again and it switches back to following that.

## What to report back

- Whether it compiles at all (see above).
- If it does: tab through a normal app's buttons (a GTK or Qt app is the
  safest first test) with the checkbox on, and see whether the magnifier
  jumps to each one.
- Whether mouse-follow still works normally with the checkbox off.

## Not in this version

- "Selection follow" (a second, related AT-SPI event for things like
  listbox/menu selection) — deliberately held back until keyboard-focus
  tracking is confirmed working, since it's a similarly new subsystem.
- Live drag-to-resize with the mouse.
- Multi-monitor awareness beyond "don't leak into the second monitor."

