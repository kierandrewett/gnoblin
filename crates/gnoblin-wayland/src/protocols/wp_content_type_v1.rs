//! XML protocol `content_type_v1` (`<protocol name="content_type_v1">`); interfaces served: `wp_content_type_v1`.
//!
//! `wp_content_type_v1`: a client says whether its surface is video or game content.
//!
//! The protocol carries one bit of intent and no mechanism: a video player says its
//! surface is `video`, a game says `game`, and everything else says nothing. The bit
//! matters to whatever decides to treat a full-screen video differently from a game — a
//! tear-free path, a delay before blanking, a frame-rate policy — and it must be decided
//! by the client, because only the client knows what it is drawing.
//!
//! Smithay stores the value in the surface's cached state and applies it on commit, so
//! this module is the global plus the one thing a compositor has to add: a value read at
//! the point the frame is composed. [`State::content_type`] answers it, so a policy that
//! wants the bit (the presentation path, a VRR choice) reads one function rather than
//! reaching into a data map.
//!
//! # Failure modes
//!
//! 1. **A content type changes without a commit.** The value is written to pending state
//!    and applied on commit, so a client that sets a type and never commits does not
//!    change what the compositor draws.
//! 2. **A content type outlives its surface.** It is stored in the surface's state, so it
//!    is dropped with the surface and cannot be read for a window that has none.

use smithay::reexports::wayland_protocols::wp::content_type::v1::server::wp_content_type_v1;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::with_states;
use smithay::wayland::content_type::{ContentTypeState, ContentTypeSurfaceCachedState};

use crate::State;

/// Creates the `wp_content_type_manager_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> ContentTypeState {
    ContentTypeState::new::<State>(display)
}

/// What a client says its surface shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WpContentTypeV1Kind {
    /// The client said nothing.
    #[default]
    None,
    /// Video content, which benefits from a tear-free presentation.
    Video,
    /// Game content, which benefits from the lowest latency.
    Game,
}

impl State {
    /// What a surface's client last committed as its content type.
    #[must_use]
    pub fn content_type(&self, surface: &WlSurface) -> WpContentTypeV1Kind {
        with_states(surface, |states| {
            let mut cached = states.cached_state.get::<ContentTypeSurfaceCachedState>();
            let state = cached.current();
            // The generated enum is `Type::None` by default, so the match is total over
            // what this protocol version can say.
            match state.content_type() {
                wp_content_type_v1::Type::Video => WpContentTypeV1Kind::Video,
                wp_content_type_v1::Type::Game => WpContentTypeV1Kind::Game,
                _ => WpContentTypeV1Kind::None,
            }
        })
    }
}
