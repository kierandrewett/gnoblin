//! What `xdg-decoration` negotiates for a window: what its client asked and who draws the frame.

/// What a window's client asked of `xdg-decoration`, as the window node's `decoration_wanted` publishes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecorationWanted {
    /// The client never bound the protocol, asked for no preference, or is an X11 window.
    #[default]
    Unset,
    /// The client asked to draw its own frame.
    Client,
    /// The client asked for the compositor to draw its frame.
    Server,
}

impl DecorationWanted {
    /// The text the window node's `decoration_wanted` holds.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unset => "unset",
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

/// Who draws a window's frame, as the compositor negotiated it and the window node's `decoration` publishes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowDecoration {
    /// The client draws its own title bar and borders.
    #[default]
    Client,
    /// The shell draws the frame.
    Server,
    /// Nobody does: an X11 window while no shell declares decorations.
    None,
}

impl WindowDecoration {
    /// The text the window node's `decoration` holds.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
            Self::None => "none",
        }
    }
}
