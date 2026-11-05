//! The file names a cursor-shape-v1 shape may be stored under.
//!
//! Clients ask for shapes by the protocol's names (`default`, `pointer`,
//! `nwse-resize`). Themes store them under the Xcursor names (`left_ptr`,
//! `hand2`, `bd_double_arrow`) or under the CSS names, and some use both. A
//! provider tries the candidates in order and the first file it finds wins.

/// The names tried last, for any shape a theme does not have.
const FALLBACK: [&str; 3] = ["left_ptr", "default", "arrow"];

/// Alternate names, tried before the generic spellings of the shape.
const ALIASES: &[(&str, &[&str])] = &[
    ("default", &["left_ptr"]),
    ("context-menu", &["left_ptr_menu"]),
    ("help", &["question_arrow", "whats_this"]),
    ("pointer", &["hand2", "hand1"]),
    ("progress", &["left_ptr_watch", "half-busy"]),
    ("wait", &["watch"]),
    ("cell", &["plus"]),
    ("crosshair", &["cross"]),
    ("text", &["xterm", "ibeam"]),
    ("vertical-text", &["xterm"]),
    ("alias", &["link"]),
    ("no-drop", &["crossed_circle"]),
    ("not-allowed", &["crossed_circle"]),
    ("grab", &["openhand", "hand1"]),
    ("grabbing", &["closedhand", "fleur"]),
    ("move", &["fleur"]),
    ("all-scroll", &["fleur"]),
    ("e-resize", &["right_side"]),
    ("n-resize", &["top_side"]),
    ("s-resize", &["bottom_side"]),
    ("w-resize", &["left_side"]),
    ("ne-resize", &["top_right_corner"]),
    ("nw-resize", &["top_left_corner"]),
    ("se-resize", &["bottom_right_corner"]),
    ("sw-resize", &["bottom_left_corner"]),
    ("ew-resize", &["h_double_arrow", "sb_h_double_arrow"]),
    ("ns-resize", &["v_double_arrow", "sb_v_double_arrow"]),
    ("nesw-resize", &["fd_double_arrow"]),
    ("nwse-resize", &["bd_double_arrow"]),
    ("col-resize", &["sb_h_double_arrow"]),
    ("row-resize", &["sb_v_double_arrow"]),
    ("dnd_ask", &["dnd-ask", "context-menu"]),
    ("all_resize", &["all-resize", "move"]),
];

/// The file names to try for a shape, most specific first, without repeats.
///
/// The order is the alias table, then the shape name as given, then its dash
/// and underscore spellings, then [`FALLBACK`]. A shape the table does not know
/// still gets its own spellings and the fallbacks.
#[must_use]
pub fn candidates(shape: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(8);
    let aliases = ALIASES
        .iter()
        .find(|(name, _)| *name == shape)
        .map(|(_, aliases)| *aliases)
        .unwrap_or(&[]);
    let spellings = [shape.to_owned(), shape.replace('-', "_"), shape.replace('_', "-")];
    for name in aliases
        .iter()
        .map(|name| (*name).to_owned())
        .chain(spellings)
        .chain(FALLBACK.iter().map(|name| (*name).to_owned()))
    {
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::candidates;

    #[test]
    fn default_is_left_ptr_then_the_generic_names() {
        assert_eq!(candidates("default"), ["left_ptr", "default", "arrow"]);
    }

    #[test]
    fn dnd_ask_and_all_resize_alias_to_the_older_names() {
        assert_eq!(candidates("dnd_ask")[..3], ["dnd-ask", "context-menu", "dnd_ask"]);
        assert_eq!(candidates("all_resize")[..3], ["all-resize", "move", "all_resize"]);
    }

    #[test]
    fn unknown_shapes_still_try_their_spellings_and_the_fallbacks() {
        assert_eq!(
            candidates("novel-shape"),
            ["novel-shape", "novel_shape", "left_ptr", "default", "arrow"]
        );
    }
}
