//! A Wayland client that maps one toplevel and keeps it for a while.
//!
//! `scripts/contract-record.py --drive` needs something that makes the session emit its window events, and the
//! session under test is a headless Mutter with no applications installed. This is the same client the scenarios use,
//! so what the recorder sees is a window the parity runner's own peer made.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::target::TargetError;
use crate::wayland_peer::WaylandPeer;

/// The name the target is given in errors.
const TARGET: &str = "hold-window";
/// The app id and title of the window, as the recorder's samples show them.
const APP_ID: &str = "org.gnoblin.parity.hold";
const TITLE: &str = "parity-hold";

/// Connects to the Wayland socket at `socket`, maps a toplevel, keeps answering the compositor for `hold`, then
/// destroys the toplevel, so the session emits a window creation, a focus change and a close.
pub fn hold_window(socket: &Path, hold: Duration) -> Result<(), TargetError> {
    let mut peer = WaylandPeer::connect(TARGET, socket)?;
    let window = peer.map_toplevel(APP_ID, TITLE)?;
    let deadline = Instant::now() + hold;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        peer.dispatch_for(left.min(Duration::from_millis(100)))?;
    }
    peer.destroy_toplevel(window)
}
