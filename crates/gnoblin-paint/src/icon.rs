//! The built-in icon set: the symbolic icons the stock shell asks for, drawn as vector strokes.
//!
//! An icon op names an icon (`window-close-symbolic`). The painter does not search an icon theme: the stock
//! titlebar must not depend on one being installed, so the icons the stock UI uses are built in, each a few strokes
//! on a 16 by 16 grid scaled to the op's size. The `-symbolic` suffix is optional. A name that is not here draws
//! nothing and is reported in [`crate::Rendered::missing_icons`].
//!
//! | Name | Drawing |
//! |---|---|
//! | `window-close` | a cross |
//! | `window-minimize` | a bar on the baseline |
//! | `window-maximize` | a square outline |
//! | `window-restore` | a square outline with a second one behind it |
//! | `list-add` | a plus |
//! | `list-remove` | a minus |
//! | `open-menu` | three bars |

use tiny_skia::{LineCap, LineJoin, Path, PathBuilder, Stroke};

/// The side of the grid every icon is drawn on.
pub const GRID: f32 = 16.0;

/// One stroked path of an icon, in grid units.
pub struct IconStroke {
    /// The path.
    pub path: Path,
    /// The stroke, with its width in grid units.
    pub stroke: Stroke,
}

fn piece(build: impl FnOnce(&mut PathBuilder), width: f32, cap: LineCap) -> Option<IconStroke> {
    let mut builder = PathBuilder::new();
    build(&mut builder);
    Some(IconStroke {
        path: builder.finish()?,
        stroke: Stroke {
            width,
            line_cap: cap,
            line_join: LineJoin::Miter,
            ..Stroke::default()
        },
    })
}

fn polyline(points: &[(f32, f32)], close: bool, width: f32, cap: LineCap) -> Option<IconStroke> {
    piece(
        |builder| {
            let (first, rest) = points.split_first().expect("a polyline has points");
            builder.move_to(first.0, first.1);
            for point in rest {
                builder.line_to(point.0, point.1);
            }
            if close {
                builder.close();
            }
        },
        width,
        cap,
    )
}

/// The strokes of the icon `name`, or `None` when it is not built in.
#[must_use]
pub fn pieces(name: &str) -> Option<Vec<IconStroke>> {
    let name = name.strip_suffix("-symbolic").unwrap_or(name);
    let drawing = match name {
        "window-close" => vec![
            polyline(&[(4.0, 4.0), (12.0, 12.0)], false, 1.5, LineCap::Butt),
            polyline(&[(12.0, 4.0), (4.0, 12.0)], false, 1.5, LineCap::Butt),
        ],
        "window-minimize" => vec![polyline(&[(3.0, 12.0), (13.0, 12.0)], false, 2.0, LineCap::Butt)],
        "window-maximize" => vec![polyline(
            &[(4.0, 4.0), (12.0, 4.0), (12.0, 12.0), (4.0, 12.0)],
            true,
            2.0,
            LineCap::Butt,
        )],
        "window-restore" => vec![
            polyline(
                &[(3.75, 6.75), (9.25, 6.75), (9.25, 12.25), (3.75, 12.25)],
                true,
                1.5,
                LineCap::Butt,
            ),
            polyline(
                &[(6.75, 6.0), (6.75, 3.75), (12.25, 3.75), (12.25, 9.25), (10.0, 9.25)],
                false,
                1.5,
                LineCap::Butt,
            ),
        ],
        "list-add" => vec![
            polyline(&[(3.0, 8.0), (13.0, 8.0)], false, 2.0, LineCap::Butt),
            polyline(&[(8.0, 3.0), (8.0, 13.0)], false, 2.0, LineCap::Butt),
        ],
        "list-remove" => vec![polyline(&[(3.0, 8.0), (13.0, 8.0)], false, 2.0, LineCap::Butt)],
        "open-menu" => vec![
            polyline(&[(3.0, 4.0), (13.0, 4.0)], false, 2.0, LineCap::Butt),
            polyline(&[(3.0, 8.0), (13.0, 8.0)], false, 2.0, LineCap::Butt),
            polyline(&[(3.0, 12.0), (13.0, 12.0)], false, 2.0, LineCap::Butt),
        ],
        _ => return None,
    };
    Some(drawing.into_iter().flatten().collect())
}

/// The names of the built-in icons, without the `-symbolic` suffix.
pub const NAMES: &[&str] = &[
    "window-close",
    "window-minimize",
    "window-maximize",
    "window-restore",
    "list-add",
    "list-remove",
    "open-menu",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_name_has_a_drawing_with_or_without_the_suffix() {
        for name in NAMES {
            assert!(pieces(name).is_some_and(|drawing| !drawing.is_empty()), "{name}");
            assert!(pieces(&format!("{name}-symbolic")).is_some(), "{name}-symbolic");
        }
    }

    #[test]
    fn an_unknown_name_has_no_drawing() {
        assert!(pieces("no-such-icon-symbolic").is_none());
        assert!(pieces("").is_none());
    }
}
