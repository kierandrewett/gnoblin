//! Compiled Hyprcursor themes, in the layout `hyprcursor-util --create` writes
//! (the layout `scripts/build-adwaita-hyprcursor.py` produces).
//!
//! Implemented, and only this:
//! - `manifest.hl` at the theme root. Its `cursors_directory` key names the
//!   directory of shape archives; it defaults to `hyprcursors`. A theme with
//!   no manifest is a miss.
//! - `<cursors_directory>/<shape>.hlc`, a zip archive holding `meta.hl` and the
//!   frame members it names. Shape names are tried in [`shapes::candidates`]
//!   order, so the shape's aliases are covered.
//! - `meta.hl`: `hotspot_x` and `hotspot_y` as fractions of the cursor size,
//!   and `define_size = <size>, <file>, <delay ms>` lines. The nominal size is
//!   chosen with [`nominal_for`]; only that size's frames are kept, in the
//!   order the file lists them.
//! - SVG and PNG frames. SVG is rasterised with [`svg::rasterise`]; PNG is
//!   decoded with [`png::rasterise`] (8-bit only, see that module). Each archive
//!   member is read through [`MAX_FILE_BYTES`], so a member that inflates past
//!   the cap is a named `Failed`.
//!
//! Not implemented, each a named `Failed` or a miss rather than a guess:
//! - Palette and 16-bit PNG frames. They are `Failed` by name and the chain
//!   moves on.
//! - `define_override` aliases. The shape candidates cover the common aliases;
//!   the override table itself is ignored.
//! - Archive version, hash and per-size interpolation beyond the nominal size.

use std::io::{Cursor, Read};

use zip::ZipArchive;

use super::files::{CursorFiles, FileError, MAX_FILE_BYTES, SearchPath, is_contained};
use super::image::{CursorFrame, CursorImages};
use super::{CursorProvider, CursorRequest, Lookup, MAX_FRAMES, ProviderKind, nominal_for, png, shapes, svg};

/// The directory of shape archives when `manifest.hl` does not name one.
const DEFAULT_DIRECTORY: &str = "hyprcursors";

/// The compiled Hyprcursor provider.
pub struct Hyprcursor;

/// One `define_size` line: a frame of one nominal size.
struct Define {
    size: u32,
    file: String,
    delay: u32,
}

/// The parts of `meta.hl` the provider reads.
struct HyprcursorMeta {
    hotspot: (f64, f64),
    defines: Vec<Define>,
}

impl CursorProvider for Hyprcursor {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Hyprcursor
    }

    fn lookup(&self, request: &CursorRequest<'_>, files: &dyn CursorFiles) -> Lookup {
        let manifest_label = format!("{}/manifest.hl", request.theme);
        let manifest = match files.read(SearchPath::Hyprcursor, request.theme, "manifest.hl") {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Lookup::Miss,
            Err(error) => return Lookup::Failed(file_failure(&manifest_label, &error)),
        };
        let directory = match cursors_directory(&manifest) {
            Ok(directory) => directory,
            Err(reason) => return Lookup::Failed(format!("{manifest_label}: {reason}")),
        };
        for name in shapes::candidates(request.shape) {
            let relative = format!("{directory}/{name}.hlc");
            let label = format!("{}/{relative}", request.theme);
            match files.read(SearchPath::Hyprcursor, request.theme, &relative) {
                Ok(None) => {}
                Ok(Some(bytes)) => {
                    return match load(&bytes, request, &label) {
                        Ok(images) => Lookup::Found(images),
                        Err(reason) => Lookup::Failed(reason),
                    };
                }
                Err(error) => return Lookup::Failed(file_failure(&label, &error)),
            }
        }
        Lookup::Miss
    }
}

/// Opens one shape archive and draws its frame set at the request's size.
fn load(bytes: &[u8], request: &CursorRequest<'_>, label: &str) -> Result<CursorImages, String> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|error| format!("{label}: not a zip archive: {error}"))?;
    let meta_bytes = member(&mut archive, "meta.hl", label)?;
    let meta_text = std::str::from_utf8(&meta_bytes).map_err(|error| format!("{label}/meta.hl: not UTF-8: {error}"))?;
    let meta = parse_meta(meta_text).map_err(|reason| format!("{label}/meta.hl: {reason}"))?;

    let size = nominal_for(request.physical, meta.defines.iter().map(|define| define.size))
        .ok_or_else(|| format!("{label}: meta.hl defines no sizes"))?;
    let mut images = Vec::new();
    for define in meta.defines.iter().filter(|define| define.size == size) {
        let bytes = member(&mut archive, &define.file, label)?;
        let frame = frame_for(&define.file, &bytes, size, request.physical)
            .map_err(|error| format!("{label}/{}: {error}", define.file))?;
        images.push(frame.with_delay(define.delay));
    }
    let hotspot = (
        px(meta.hotspot.0, request.physical),
        px(meta.hotspot.1, request.physical),
    );
    CursorImages::new(hotspot, images).map_err(|error| format!("{label}: {error}"))
}

/// Reads one archive member, refusing one that inflates past [`MAX_FILE_BYTES`].
fn member(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str, label: &str) -> Result<Vec<u8>, String> {
    if !is_contained(name) {
        return Err(format!("{label}: member {name:?} is outside the archive"));
    }
    let failure = |error: &dyn std::fmt::Display| format!("{label}/{name}: {error}");
    let mut file = archive.by_name(name).map_err(|error| failure(&error))?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| failure(&error))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("{label}/{name}: larger than {MAX_FILE_BYTES} bytes"));
    }
    Ok(bytes)
}

/// The `cursors_directory` the manifest names, checked to stay inside the theme.
fn cursors_directory(manifest: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(manifest).map_err(|error| format!("not UTF-8: {error}"))?;
    let directory = settings(text)
        .find(|(key, _)| *key == "cursors_directory")
        .map_or(DEFAULT_DIRECTORY, |(_, value)| value);
    if directory.is_empty() || !is_contained(directory) {
        return Err(format!("cursors_directory {directory:?} is outside the theme"));
    }
    Ok(directory.to_owned())
}

/// Parses `meta.hl`, keeping the hotspot and every `define_size` in order.
fn parse_meta(text: &str) -> Result<HyprcursorMeta, String> {
    let mut hotspot = (0.0, 0.0);
    let mut defines = Vec::new();
    for (key, value) in settings(text) {
        match key {
            "hotspot_x" => hotspot.0 = fraction(key, value)?,
            "hotspot_y" => hotspot.1 = fraction(key, value)?,
            "define_size" => defines.push(parse_define(value)?),
            _ => {}
        }
    }
    if defines.len() > MAX_FRAMES {
        return Err(format!("{} frames, more than {MAX_FRAMES}", defines.len()));
    }
    Ok(HyprcursorMeta { hotspot, defines })
}

/// One `define_size` value: `size, file, delay`. The frame must be an SVG or a PNG.
fn parse_define(value: &str) -> Result<Define, String> {
    let mut parts = value.split(',').map(str::trim);
    let (Some(size), Some(file), Some(delay), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(format!("define_size {value:?} is not size, file, delay"));
    };
    let size = size
        .parse::<u32>()
        .ok()
        .filter(|size| *size > 0)
        .ok_or_else(|| format!("define_size {value:?}: the size is not a positive integer"))?;
    let delay = delay
        .parse::<u32>()
        .map_err(|_| format!("define_size {value:?}: the delay is not a whole number of milliseconds"))?;
    if !file.ends_with(".svg") && !file.ends_with(".png") {
        return Err(format!("frame {file} is neither an SVG nor a PNG"));
    }
    if !is_contained(file) {
        return Err(format!("frame {file} is outside the archive"));
    }
    Ok(Define {
        size,
        file: file.to_owned(),
        delay,
    })
}

/// Rasterises one frame by its file extension, onto the physical canvas.
fn frame_for(file: &str, bytes: &[u8], nominal: u32, physical: u32) -> Result<CursorFrame, String> {
    if file.ends_with(".png") {
        png::rasterise(bytes, physical).map_err(|error| error.to_string())
    } else {
        svg::rasterise(bytes, nominal, physical).map_err(|error| error.to_string())
    }
}

/// A hotspot fraction, which must lie between zero and one.
fn fraction(key: &str, value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|fraction| fraction.is_finite() && (0.0..=1.0).contains(fraction))
        .ok_or_else(|| format!("{key} is not a fraction between 0 and 1"))
}

/// A hotspot fraction in physical pixels, rounded to the nearest pixel.
fn px(fraction: f64, physical: u32) -> i64 {
    (fraction * f64::from(physical)).round() as i64
}

/// The `key = value` lines of a manifest or `meta.hl`. Blank and `#` lines are
/// skipped; a line without `=` is skipped.
fn settings(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        Some((key.trim(), value.trim()))
    })
}

fn file_failure(label: &str, error: &FileError) -> String {
    format!("{label}: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::MemoryFiles;

    #[test]
    fn the_manifest_names_the_cursor_directory() {
        assert_eq!(
            cursors_directory(b"name = x\ncursors_directory = hyprcursors\n").expect("named directory"),
            "hyprcursors"
        );
        assert_eq!(
            cursors_directory(b"name = x\n").expect("default directory"),
            "hyprcursors"
        );
        assert!(cursors_directory(b"cursors_directory = ../escape\n").is_err());
    }

    #[test]
    fn meta_lists_the_hotspot_and_each_frame() {
        let meta = parse_meta(
            "resize_algorithm = bilinear\nhotspot_x = 0.5\nhotspot_y = 0.25\n\
             define_size = 24, 0001.svg, 50\ndefine_size = 48, 0001.svg, 50\n",
        )
        .expect("a well formed meta.hl parses");
        assert_eq!(meta.hotspot, (0.5, 0.25));
        assert_eq!(meta.defines.len(), 2);
        assert_eq!(meta.defines[0].size, 24);
        assert_eq!(meta.defines[0].delay, 50);
    }

    /// A theme holding one shape archive whose single frame is `frame` named `0001.png`.
    fn png_theme(frame: &[u8]) -> MemoryFiles {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let meta = "hotspot_x = 0.5\nhotspot_y = 0.25\ndefine_size = 2, 0001.png, 50\n";
        archive.start_file("meta.hl", options).expect("meta member");
        std::io::Write::write_all(&mut archive, meta.as_bytes()).expect("meta bytes");
        archive.start_file("0001.png", options).expect("frame member");
        std::io::Write::write_all(&mut archive, frame).expect("frame bytes");
        let shape = archive.finish().expect("archive").into_inner();
        MemoryFiles::new()
            .with_file(
                SearchPath::Hyprcursor,
                "Test/manifest.hl",
                b"cursors_directory = hyprcursors\n".to_vec(),
            )
            .with_file(SearchPath::Hyprcursor, "Test/hyprcursors/default.hlc", shape)
    }

    #[test]
    fn a_png_frame_decodes_with_its_hotspot() {
        let frame = super::super::png::tests::encode(2, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &[255; 16]);
        let files = png_theme(&frame);
        let request = CursorRequest {
            theme: "Test",
            shape: "default",
            physical: 2,
        };
        let Lookup::Found(images) = Hyprcursor.lookup(&request, &files) else {
            panic!("the PNG theme is found");
        };
        assert_eq!(images.hotspot(), (1, 1));
        assert_eq!(images.frames().len(), 1);
        assert_eq!(images.frames()[0].pixels(), &[0xFFFF_FFFF; 4]);
    }

    #[test]
    fn a_truncated_png_frame_fails_naming_the_file() {
        let frame = super::super::png::tests::encode(2, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &[255; 16]);
        let files = png_theme(&frame[..frame.len() / 2]);
        let request = CursorRequest {
            theme: "Test",
            shape: "default",
            physical: 2,
        };
        let Lookup::Failed(reason) = Hyprcursor.lookup(&request, &files) else {
            panic!("a truncated frame is a named failure");
        };
        assert!(reason.contains("0001.png"), "{reason}");
    }

    #[test]
    fn unknown_frames_and_out_of_range_hotspots_fail() {
        assert!(parse_meta("define_size = 24, 0001.gif, 50\n").is_err());
        assert!(parse_meta("hotspot_x = 2\n").is_err());
    }
}
