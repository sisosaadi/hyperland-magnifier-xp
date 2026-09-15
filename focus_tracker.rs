//! Tracks system-wide keyboard focus changes via AT-SPI2, Linux's
//! accessibility framework — the same plumbing screen readers use. This is
//! the only real way for one app to learn "what got keyboard focus in a
//! different app" on Linux; there's no Wayland protocol for it.
//!
//! Needs the AT-SPI bus running:
//!   busctl --user call org.a11y.Bus /org/a11y/bus org.a11y.Bus GetAddress
//! should print an address. If it errors, this feature can't work until
//! that's fixed (installing/starting `at-spi2-registryd`) — that's a system
//! setup issue, not something this code can work around.
//!
//! Only sees focus changes in apps that publish accessibility info — most
//! GTK/Qt/Electron/browser apps do; custom-drawn UIs and games often don't.
//!
//! Also tracks the text caret while typing (not just Tab-focus changes),
//! but re-centers on the whole focused field rather than the exact
//! character — GTK's AT-SPI implementation returned "NotSupported" for
//! both per-character position methods tried (GetCharacterExtents and
//! GetRangeExtents) on every widget tested, so field-level tracking is the
//! practical ceiling here, not a shortcut.
//!
//! This is the newest, least-verified part of the project: the `atspi`
//! crate's exact API has shifted across versions, so if this file doesn't
//! compile as-is, that's expected — paste the error back.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use atspi::{
    connection::AccessibilityConnection,
    events::{FocusEvents, ObjectEvents},
    CoordType, Event,
};
use atspi_proxies::component::ComponentProxy;
use futures_lite::StreamExt;
use serde::Deserialize;

/// A focused element's bounding box, in screen pixel coordinates (the same
/// global coordinate space Hyprland reports the cursor in).
#[derive(Clone, Copy, Debug)]
pub struct FocusRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Spawns a dedicated thread running its own small async runtime (AT-SPI
/// requires one; everything else in this project is synchronous, so this
/// stays fully isolated from the rest of the program). Reads
/// `settings_enabled` each time an event arrives to decide whether to
/// bother doing anything, so this has near-zero cost while the feature is
/// switched off. Returns a shared slot holding the most recent focused
/// element's rectangle plus when it was recorded.
pub fn spawn(follow_enabled: Arc<Mutex<bool>>) -> Arc<Mutex<Option<(FocusRect, Instant)>>> {
    let focus_rect = Arc::new(Mutex::new(None));
    let writer = Arc::clone(&focus_rect);

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(err) => {
                eprintln!("Couldn't start the accessibility listener's runtime: {err}");
                return;
            }
        };

        runtime.block_on(async move {
            if let Err(err) = run(writer, follow_enabled).await {
                eprintln!(
                    "Accessibility (AT-SPI) listener stopped: {err}. \"Follow keyboard focus\" \
                     won't work until this is fixed — check that at-spi2-registryd is running \
                     (see the check at the top of focus_tracker.rs)."
                );
            }
        });
    });

    focus_rect
}

async fn run(
    focus_rect: Arc<Mutex<Option<(FocusRect, Instant)>>>,
    follow_enabled: Arc<Mutex<bool>>,
) -> atspi::Result<()> {
    let connection = AccessibilityConnection::new().await?;
    connection.register_event::<FocusEvents>().await?;
    connection.register_event::<ObjectEvents>().await?;

    eprintln!("[focus_tracker] connected to AT-SPI, waiting for focus events...");

    let events = connection.event_stream();
    tokio::pin!(events);

    while let Some(result) = events.next().await {
        let Ok(event) = result else { continue };

        let enabled = follow_enabled.lock().map(|g| *g).unwrap_or(false);
        if !enabled {
            continue; // feature switched off — skip the extra D-Bus round trip
        }

        let (object_ref, caret_offset) = match &event {
            Event::Focus(FocusEvents::Focus(e)) => (Some(e.item.clone()), None),
            Event::Object(ObjectEvents::StateChanged(e))
                if e.state == atspi::State::Focused && e.enabled =>
            {
                (Some(e.item.clone()), None)
            }
            Event::Object(ObjectEvents::TextCaretMoved(e)) => {
                (Some(e.item.clone()), Some(e.position))
            }
            _ => (None, None),
        };

        let Some(object_ref) = object_ref else { continue };

        eprintln!("[focus_tracker] focus/caret moved to {object_ref:?} (caret: {caret_offset:?})");

        // Note: GTK's AT-SPI implementation doesn't support per-character
        // position lookups for this widget type — both GetCharacterExtents
        // and GetRangeExtents returned "NotSupported" in testing. So caret
        // moves fall back to the same whole-field position as focus moves,
        // same as everything else.
        let extents = fetch_extents(&connection, &object_ref).await;

        match extents {
            Some(rect) => {
                eprintln!("[focus_tracker] extents: {rect:?}");
                if let Ok(mut guard) = focus_rect.lock() {
                    *guard = Some((rect, Instant::now()));
                }
            }
            None => eprintln!("[focus_tracker] couldn't get extents for {object_ref:?}"),
        }
    }

    Ok(())
}

/// Given a reference to the newly-focused accessible object, works out its
/// position *on screen* — even though GTK (and likely other toolkits) can't
/// report that directly under Wayland; `CoordType::Screen` always comes
/// back as (0, 0) in testing. Instead, we ask for the widget's position
/// *relative to its own window*, separately ask Hyprland (the same IPC
/// socket used for the cursor) where that window currently sits on screen,
/// and add the two together.
async fn fetch_extents(
    connection: &AccessibilityConnection,
    object_ref: &atspi::ObjectRefOwned,
) -> Option<FocusRect> {
    let name = object_ref.name()?.to_owned();
    let path = object_ref.path().to_owned();

    let component: ComponentProxy = ComponentProxy::builder(connection.connection())
        .destination(name)
        .ok()?
        .path(path)
        .ok()?
        .build()
        .await
        .ok()?;

    let (rel_x, rel_y, w, h) = component.get_extents(CoordType::Window).await.ok()?;
    let (win_x, win_y, _win_w, _win_h) = query_active_window_geometry()?;

    Some(FocusRect {
        x: win_x + rel_x,
        y: win_y + rel_y,
        width: w,
        height: h,
    })
}

#[derive(Deserialize)]
struct HyprlandActiveWindow {
    at: [i32; 2],
    size: [i32; 2],
}

/// Asks Hyprland (over the same IPC socket used for the cursor position)
/// where the currently active window sits on screen. Since a
/// keyboard-focus change almost always happens within whichever window is
/// currently active, this is what lets us turn the widget's window-relative
/// position into an actual screen position.
fn query_active_window_geometry() -> Option<(i32, i32, i32, i32)> {
    let socket_path = crate::hyprland_socket_path()?;
    let mut stream = UnixStream::connect(socket_path).ok()?;
    stream.write_all(b"j/activewindow").ok()?;
    stream.shutdown(std::net::Shutdown::Write).ok();
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    let window: HyprlandActiveWindow = serde_json::from_str(&response).ok()?;
    Some((window.at[0], window.at[1], window.size[0], window.size[1]))
}
