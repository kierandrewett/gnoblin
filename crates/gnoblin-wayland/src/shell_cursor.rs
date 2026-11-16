//! The cursor a shell chooses: a named theme cursor, an image of its own or none at all, over the cursor the client
//! under the pointer asked for.
//!
//! The shell writes the main seat's `cursor_shape`, `cursor_image`, `cursor_hotspot_x`, `cursor_hotspot_y` and
//! `cursor_hidden` values (`design/node-core.md`, "Input grabs and the cursor"). Each frame the compositor reads them
//! after the shell's transactions have applied, decodes a changed image once, and draws by this order: hidden, the
//! image, the named shape, the client's cursor. A shell that writes none of them, or clears them, leaves the client's
//! cursor as it was; nothing here changes what the client asked for, only what is drawn.
//!
//! # Failure modes
//!
//! 1. **The shell detaches with the cursor hidden.** Nobody is left to show it again, so the compositor clears the
//!    shell's values the frame it notices and the client's cursor is back:
//!    `a_shell_that_hides_the_cursor_shows_none_and_a_shell_that_detaches_gives_it_back`.
//! 2. **The session locks.** `State::frame` draws no pointer on a locked output (the pointer would sit over the cover),
//!    and that rule comes before this module: nothing the shell wrote is drawn, a shell can neither show a cursor over
//!    the cover nor leave the session with one it hid, and the values are kept for the unlock. There is no test that
//!    reads a locked output's cursor back, because a locked session refuses the capture that would show it.
//! 3. **An image the compositor cannot decode.** It is logged once and not drawn; the shape, or the client's cursor, is
//!    drawn instead, so a bad image never means no pointer: `an_image_that_is_not_a_png_draws_the_next_cursor`.
//! 4. **A name the theme lacks.** The cursor chain answers the built-in arrow, so the pointer is never invisible.
//! 5. **A pointer grab with no cursor of the shell's.** The client that had the pointer has left, and the seat resets the
//!    cursor it asked for to the default arrow when its pointer focus goes, so the arrow is drawn until the shell
//!    chooses one or the pointer enters a client, which sets its cursor on entering as the protocol asks:
//!    `a_pointer_grab_takes_the_clients_cursor_away_and_the_client_has_to_set_it_again`.

use gnoblin_backend::render::compose::ShellCursor;
use gnoblin_cursor::CursorImages;

use crate::State;
use crate::log;

/// What the shell asked for, and what its image decoded to.
#[derive(Debug, Default)]
pub(crate) struct ShellCursorState {
    /// The values the shell wrote, as last read.
    asked: ShellCursor,
    /// The image `asked` carries, decoded, when it decodes.
    image: Option<CursorImages>,
}

/// Which cursor is drawn.
pub(crate) enum CursorChoice<'a> {
    /// No cursor.
    Hidden,
    /// The shell's own image.
    Image(&'a CursorImages),
    /// A theme cursor by name.
    Named(&'a str),
    /// The client's: the compositor draws what it asked for.
    Client,
}

impl State {
    /// Reads the shell's cursor values and keeps what is drawn in line with them. A shell that no longer holds the tree
    /// has its values cleared. A change damages the cursor so it repaints where it stands.
    pub(crate) fn follow_shell_cursor(&mut self) {
        if !self.composer.holder_attached() {
            self.composer.release_shell_cursor();
        }
        if self.composer.main_seat_cursor_is(&self.shell_cursor.asked) {
            return;
        }
        let asked = self.composer.main_seat_cursor();
        let previous = &self.shell_cursor.asked;
        if asked.image != previous.image || asked.hotspot != previous.hotspot {
            self.shell_cursor.image = if asked.image.is_empty() {
                None
            } else {
                match gnoblin_cursor::decode_image(&asked.image, asked.hotspot) {
                    Ok(images) => Some(images),
                    Err(error) => {
                        log::warn!(area = "cursor", %error, "the shell's cursor image is not drawn");
                        None
                    }
                }
            };
        }
        self.shell_cursor.asked = asked;
        self.update_cursor(true);
    }

    /// The cursor to draw: the shell's choice while one stands, else the client's. Never called for a locked session,
    /// which draws no pointer.
    pub(crate) fn cursor_choice(&self) -> CursorChoice<'_> {
        let asked = &self.shell_cursor.asked;
        if asked.hidden {
            return CursorChoice::Hidden;
        }
        if let Some(images) = &self.shell_cursor.image {
            return CursorChoice::Image(images);
        }
        if !asked.shape.is_empty() {
            return CursorChoice::Named(&asked.shape);
        }
        CursorChoice::Client
    }
}
