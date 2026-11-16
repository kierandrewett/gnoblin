//! The main seat's input state the shell reads and writes: the grab it holds and the cursor it shows.
//!
//! A move out of `compose.rs`: these are methods of [`Composer`] and read its fields. The compositor owns the grab
//! (`gnoblin-wayland`'s `seat_grab`); what lives here is the part that is the tree's: the read-only `grabbed` value the
//! compositor publishes, and the `cursor_*` values a shell writes and the compositor reads each frame.

use super::*;

/// The cursor a shell asks for on the main seat: what it wrote in the seat's `cursor_*` values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellCursor {
    /// The named theme cursor, or empty for none.
    pub shape: String,
    /// The encoded image, or empty for none.
    pub image: Vec<u8>,
    /// The image's hotspot, in image pixels.
    pub hotspot: (i64, i64),
    /// Whether the shell hides the cursor.
    pub hidden: bool,
}

impl ShellCursor {
    /// Whether the shell has no say in the cursor: the client's cursor shows.
    #[must_use]
    pub fn is_unset(&self) -> bool {
        !self.hidden && self.image.is_empty() && self.shape.is_empty()
    }
}

/// The seat values that make up a [`ShellCursor`], with the value each has when the shell has no say.
const CURSOR_KEYS: [(&str, TreeValue); 5] = [
    ("cursor_shape", TreeValue::Text(String::new())),
    ("cursor_image", TreeValue::Bytes(Vec::new())),
    ("cursor_hotspot_x", TreeValue::Int(0)),
    ("cursor_hotspot_y", TreeValue::Int(0)),
    ("cursor_hidden", TreeValue::Bool(false)),
];

impl Composer {
    /// The main seat node, once it exists.
    #[must_use]
    pub fn main_seat_node(&self) -> Option<NodeId> {
        self.main_seat
    }

    /// Whether `node` is a node of the shell's own that can hold an input grab: one the shell made and still has, that
    /// is not a window.
    #[must_use]
    pub fn is_shell_node(&self, node: NodeId) -> bool {
        self.scene
            .node(node)
            .is_some_and(|entry| entry.owner() == gnoblin_render::Actor::Shell)
            && self.window_of_node(node).is_none()
    }

    /// Whether `node` is part of the shell's lock screen while the shell has it up: the `lock` node is `engaged` and
    /// `node` is one of its descendants. The one place a node of the shell may take input while the session is locked.
    #[must_use]
    pub fn lock_screen_holds(&self, node: NodeId) -> bool {
        self.lock_ui_engaged() && self.under_lock_node(node)
    }

    /// Publishes the scope of the input grab the shell holds, or empty for none. Only a change is published.
    pub fn publish_main_seat_grab(&mut self, scope: &str) {
        let Some(seat) = self.main_seat else {
            return;
        };
        if self.scene.text(seat, "grabbed").unwrap_or("") == scope {
            return;
        }
        let value = TreeValue::Text(scope.to_owned());
        if self.scene.set_value(seat, "grabbed", value.clone()).is_ok() {
            self.tree_changes.push(TreeChange::Set {
                node: seat,
                key: "grabbed".to_owned(),
                value,
            });
        }
    }

    /// What the shell asked the cursor to be, read from the main seat's `cursor_*` values. Nothing set reads as
    /// [`ShellCursor::is_unset`], and so does a seat that does not exist.
    #[must_use]
    pub fn main_seat_cursor(&self) -> ShellCursor {
        let Some(seat) = self.main_seat else {
            return ShellCursor::default();
        };
        let number = |key: &str| match self.scene.value(seat, key) {
            Some(TreeValue::Int(value)) => *value,
            _ => 0,
        };
        ShellCursor {
            shape: self.scene.text(seat, "cursor_shape").unwrap_or("").to_owned(),
            image: match self.scene.value(seat, "cursor_image") {
                Some(TreeValue::Bytes(bytes)) => bytes.clone(),
                _ => Vec::new(),
            },
            hotspot: (number("cursor_hotspot_x"), number("cursor_hotspot_y")),
            hidden: matches!(self.scene.value(seat, "cursor_hidden"), Some(TreeValue::Bool(true))),
        }
    }

    /// Whether the main seat's `cursor_*` values still say what `cursor` records, compared where they stand so a frame
    /// with nothing new copies no image.
    #[must_use]
    pub fn main_seat_cursor_is(&self, cursor: &ShellCursor) -> bool {
        let Some(seat) = self.main_seat else {
            return cursor.is_unset();
        };
        let image_is = match self.scene.value(seat, "cursor_image") {
            Some(TreeValue::Bytes(bytes)) => *bytes == cursor.image,
            _ => cursor.image.is_empty(),
        };
        let number = |key: &str| match self.scene.value(seat, key) {
            Some(TreeValue::Int(value)) => *value,
            _ => 0,
        };
        image_is
            && self.scene.text(seat, "cursor_shape").unwrap_or("") == cursor.shape
            && (number("cursor_hotspot_x"), number("cursor_hotspot_y")) == cursor.hotspot
            && matches!(self.scene.value(seat, "cursor_hidden"), Some(TreeValue::Bool(true))) == cursor.hidden
    }

    /// Gives the cursor back to the client under the pointer: clears every `cursor_*` value the shell wrote on the main
    /// seat. Published like any other change, so the next shell starts from a seat nobody set a cursor on. The shell
    /// that wrote them no longer holds the tree, and no shell is left to take the cursor back from an invisible one.
    pub fn release_shell_cursor(&mut self) {
        let Some(seat) = self.main_seat else {
            return;
        };
        for (key, default) in CURSOR_KEYS {
            if self.scene.value(seat, key).is_none_or(|value| *value == default) {
                continue;
            }
            if self.scene.set_value(seat, key, default.clone()).is_ok() {
                self.tree_changes.push(TreeChange::Set {
                    node: seat,
                    key: key.to_owned(),
                    value: default,
                });
            }
        }
    }
}
