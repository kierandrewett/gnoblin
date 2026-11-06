//! Xcursor themes: `<search root>/<theme>/cursors/<shape>` files, read through
//! [`CursorFiles`], following the `Inherits=` links of `index.theme`.
//!
//! The file format is parsed by the `xcursor` crate. Frames are kept at the
//! nominal size [`nominal_for`] picks for the physical size, then resampled per
//! axis to it, so an animated cursor keeps every frame and its delay. Themes
//! are searched in inheritance order, breadth first, and within a theme the
//! shape names are tried in the order [`candidates`] gives.

use std::collections::VecDeque;

use super::{
    CursorFiles, CursorFrame, CursorImages, CursorProvider, CursorRequest, FileError, Lookup, MAX_FRAMES, ProviderKind,
    SearchPath, candidates, is_contained, nominal_for, premultiplied, scale_hotspot, scale_length,
};

/// The deepest `Inherits=` chain followed from the requested theme.
const MAX_INHERIT_DEPTH: usize = 8;

/// The most themes one lookup visits, counting the requested theme.
const MAX_THEMES: usize = 32;

/// The Xcursor provider.
#[derive(Clone, Copy, Debug, Default)]
pub struct Xcursor;

impl CursorProvider for Xcursor {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Xcursor
    }

    fn lookup(&self, request: &CursorRequest<'_>, files: &dyn CursorFiles) -> Lookup {
        if !is_contained(request.theme) {
            return Lookup::Miss;
        }
        let themes = match inheritance(files, request.theme) {
            Ok(themes) => themes,
            Err(reason) => return Lookup::Failed(reason),
        };
        let names = candidates(request.shape);
        for theme in &themes {
            for name in &names {
                let relative = format!("cursors/{name}");
                let label = format!("{theme}/{relative}");
                match files.read(SearchPath::Xcursor, theme, &relative) {
                    Ok(None) => {}
                    Ok(Some(bytes)) => {
                        return match load(&bytes, request.physical, &label) {
                            Ok(images) => Lookup::Found(images),
                            Err(reason) => Lookup::Failed(reason),
                        };
                    }
                    Err(error) => return Lookup::Failed(file_failure(&label, &error)),
                }
            }
        }
        Lookup::Miss
    }
}

/// The requested theme followed by every theme it inherits from, breadth first.
/// A name that is not a plain relative path, or one already listed, is skipped,
/// which breaks cycles. The walk stops at [`MAX_INHERIT_DEPTH`] and [`MAX_THEMES`].
fn inheritance(files: &dyn CursorFiles, theme: &str) -> Result<Vec<String>, String> {
    let mut order = vec![theme.to_owned()];
    let mut frontier = VecDeque::from([(theme.to_owned(), 0_usize)]);
    while let Some((current, depth)) = frontier.pop_front() {
        if depth >= MAX_INHERIT_DEPTH {
            continue;
        }
        let index = match files.read(SearchPath::Xcursor, &current, "index.theme") {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(error) => return Err(file_failure(&format!("{current}/index.theme"), &error)),
        };
        for parent in inherits(&index) {
            if order.len() >= MAX_THEMES {
                return Ok(order);
            }
            if !is_contained(&parent) || order.contains(&parent) {
                continue;
            }
            order.push(parent.clone());
            frontier.push_back((parent, depth + 1));
        }
    }
    Ok(order)
}

/// The theme names on the `Inherits=` lines of an `index.theme` file.
fn inherits(index: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(index);
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("Inherits")?.trim_start().strip_prefix('='))
        .flat_map(|value| value.split([',', ';']))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Parses one Xcursor file and builds its images at `physical` pixels.
fn load(bytes: &[u8], physical: u32, label: &str) -> Result<CursorImages, String> {
    let images = xcursor::parser::parse_xcursor(bytes).ok_or_else(|| format!("{label}: not an Xcursor file"))?;
    let nominal = nominal_for(physical, images.iter().map(|image| image.size))
        .ok_or_else(|| format!("{label}: the file has no images"))?;
    let chosen: Vec<_> = images.iter().filter(|image| image.size == nominal).collect();
    if chosen.len() > MAX_FRAMES {
        return Err(format!("{label}: {} frames, more than {MAX_FRAMES}", chosen.len()));
    }
    let Some(&first) = chosen.first() else {
        return Err(format!("{label}: the file has no images"));
    };

    let mut frames = Vec::with_capacity(chosen.len());
    for image in &chosen {
        // The format stores straight-alpha bytes in A, R, G, B order.
        let pixels = image
            .pixels_argb
            .as_chunks::<4>()
            .0
            .iter()
            .map(|px| premultiplied(px[1], px[2], px[3], px[0]))
            .collect();
        let frame = CursorFrame::new(image.width, image.height, pixels, image.delay)
            .map_err(|error| format!("{label}: {error}"))?;
        frames.push(frame);
    }
    let native = CursorImages::new((0, 0), frames).map_err(|error| format!("{label}: {error}"))?;

    let (width, height) = (first.width, first.height);
    let (target_width, target_height) = (
        scale_length(width, nominal, physical),
        scale_length(height, nominal, physical),
    );
    let resized = native
        .resampled(target_width, target_height)
        .map_err(|error| format!("{label}: {error}"))?;
    let hotspot = (
        i64::from(scale_hotspot(first.xhot, width, target_width)),
        i64::from(scale_hotspot(first.yhot, height, target_height)),
    );
    CursorImages::new(hotspot, resized.frames().to_vec()).map_err(|error| format!("{label}: {error}"))
}

/// The reason a file could not be read, naming the file.
fn file_failure(label: &str, error: &FileError) -> String {
    format!("{label}: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherits_splits_comma_and_semicolon_lists() {
        let index = b"[Icon Theme]\nName=Test\nInherits=Adwaita, hicolor;core\n";
        assert_eq!(inherits(index), vec!["Adwaita", "hicolor", "core"]);
    }

    #[test]
    fn inherits_ignores_absent_or_empty_keys() {
        assert!(inherits(b"[Icon Theme]\nName=Test\n").is_empty());
        assert!(inherits(b"Inherits=\n").is_empty());
    }
}
