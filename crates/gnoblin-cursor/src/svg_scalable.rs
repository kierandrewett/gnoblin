//! `cursors_scalable` themes (the KDE and GNOME layout).
//!
//! A theme has `cursors_scalable/<shape>/metadata.json`, a JSON array of frames
//! in play order. Each frame is an object with `filename` (an SVG beside the
//! metadata), `hotspot_x` and `hotspot_y` (in the frame's nominal pixels),
//! `nominal_size` (the size the artwork was drawn at) and `delay` (milliseconds,
//! zero for a static cursor). Every frame is rasterised at the physical size
//! through [`svg::rasterise`], and its hotspot is scaled from nominal to
//! physical with [`scale_hotspot`]. The first frame's hotspot is the cursor's.
//!
//! Not implemented: `index.theme` inheritance. These themes do not inherit, so
//! a shape missing from one is a miss, and the chain asks the next provider.

use serde_json::Value;

use crate::files::{CursorFiles, FileError, SearchPath, is_contained};
use crate::image::{CursorFrame, CursorImages};
use crate::shapes;
use crate::{CursorProvider, CursorRequest, Lookup, MAX_FRAMES, ProviderKind, scale_hotspot, svg};

/// The `cursors_scalable` provider.
pub struct KdeGnomeSvg;

/// One frame as the metadata names it, already validated.
struct SvgFrameEntry {
    filename: String,
    hotspot: (u32, u32),
    nominal: u32,
    delay: u32,
}

impl CursorProvider for KdeGnomeSvg {
    fn kind(&self) -> ProviderKind {
        ProviderKind::KdeGnomeSvg
    }

    fn lookup(&self, request: &CursorRequest<'_>, files: &dyn CursorFiles) -> Lookup {
        for name in shapes::candidates(request.shape) {
            let directory = format!("cursors_scalable/{name}");
            let label = format!("{}/{directory}/metadata.json", request.theme);
            match files.read(
                SearchPath::Scalable,
                request.theme,
                &format!("{directory}/metadata.json"),
            ) {
                Ok(None) => {}
                Ok(Some(bytes)) => {
                    return match load(files, request, &directory, &bytes) {
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

/// Reads the frames `metadata.json` names and rasterises each one.
fn load(
    files: &dyn CursorFiles,
    request: &CursorRequest<'_>,
    directory: &str,
    bytes: &[u8],
) -> Result<CursorImages, String> {
    let label = format!("{}/{directory}/metadata.json", request.theme);
    let frames = parse_metadata(bytes).map_err(|reason| format!("{label}: {reason}"))?;

    let mut hotspot = None;
    let mut images = Vec::with_capacity(frames.len());
    for frame in &frames {
        let path = format!("{directory}/{}", frame.filename);
        let path_label = format!("{}/{path}", request.theme);
        let svg_bytes = files
            .read(SearchPath::Scalable, request.theme, &path)
            .map_err(|error| file_failure(&path_label, &error))?
            .ok_or_else(|| format!("{path_label}: listed in metadata.json but missing"))?;
        let raster: CursorFrame = svg::rasterise(&svg_bytes, frame.nominal, request.physical)
            .map_err(|error| format!("{path_label}: {error}"))?;
        hotspot.get_or_insert_with(|| {
            (
                i64::from(scale_hotspot(frame.hotspot.0, frame.nominal, request.physical)),
                i64::from(scale_hotspot(frame.hotspot.1, frame.nominal, request.physical)),
            )
        });
        images.push(raster.with_delay(frame.delay));
    }
    let hotspot = hotspot.unwrap_or((0, 0));
    CursorImages::new(hotspot, images).map_err(|error| format!("{label}: {error}"))
}

/// Checks the metadata and returns its frames in order. Every named file must
/// stay inside the theme, and the frame count is bounded by [`MAX_FRAMES`].
fn parse_metadata(bytes: &[u8]) -> Result<Vec<SvgFrameEntry>, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| format!("not JSON: {error}"))?;
    let entries = value.as_array().ok_or("not a JSON array of frames")?;
    if entries.is_empty() {
        return Err("the array has no frames".to_owned());
    }
    if entries.len() > MAX_FRAMES {
        return Err(format!("{} frames, more than {MAX_FRAMES}", entries.len()));
    }
    entries.iter().map(parse_frame).collect()
}

/// Reads one frame entry of `metadata.json`. Every number must be present and
/// fit in a u32. A zero nominal size names no cursor size, so it is refused here
/// rather than surfacing later as a broken cursor.
fn parse_frame(entry: &Value) -> Result<SvgFrameEntry, String> {
    let filename = entry
        .get("filename")
        .and_then(Value::as_str)
        .ok_or("a frame has no filename")?;
    if !is_contained(filename) {
        return Err(format!("frame filename {filename:?} is outside the theme"));
    }
    let number = |key: &str| -> Result<u32, String> {
        entry
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| format!("frame {filename}: {key} is not a non-negative integer"))
    };
    let nominal = number("nominal_size")?;
    if nominal == 0 {
        return Err(format!("frame {filename}: nominal_size is zero"));
    }
    Ok(SvgFrameEntry {
        filename: filename.to_owned(),
        hotspot: (number("hotspot_x")?, number("hotspot_y")?),
        nominal,
        delay: number("delay")?,
    })
}

/// The label prefix every file failure carries, so a log line names the file.
fn file_failure(label: &str, error: &FileError) -> String {
    format!("{label}: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_must_be_an_array_of_frames() {
        assert!(parse_metadata(b"{}").is_err());
        assert!(parse_metadata(b"[]").is_err());
        assert!(parse_metadata(b"not json").is_err());
    }

    #[test]
    fn a_frame_must_name_a_file_inside_the_theme() {
        let escape = br#"[{"filename":"../x.svg","hotspot_x":0,"hotspot_y":0,"nominal_size":24,"delay":0}]"#;
        assert!(parse_metadata(escape).is_err());
    }

    #[test]
    fn a_frame_needs_every_field() {
        let no_delay = br#"[{"filename":"a.svg","hotspot_x":1,"hotspot_y":2,"nominal_size":24}]"#;
        assert!(parse_metadata(no_delay).is_err());
    }

    #[test]
    fn a_complete_frame_parses() {
        let good = br#"[{"filename":"a.svg","hotspot_x":1,"hotspot_y":2,"nominal_size":24,"delay":50}]"#;
        let frames = parse_metadata(good).expect("a complete frame parses");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].hotspot, (1, 2));
        assert_eq!(frames[0].nominal, 24);
        assert_eq!(frames[0].delay, 50);
    }
}
