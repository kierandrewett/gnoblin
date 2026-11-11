//! `GNOBLIN_XR_DUMP_DIR`: a test-only copy of the pictures the extension uploads.
//!
//! When the variable names a directory, each picture that is copied into a swapchain is also written there as a
//! binary PPM (`P6`), so a script can compare a pixel with what the client drew. Per View node `N`:
//!
//! - `view-N-first.ppm`: the first picture uploaded for that View, written once.
//! - `view-N-latest.ppm`: the picture of the most recent upload, rewritten each time.
//!
//! It exists for `scripts/xr-smoke.sh` and is not a feature: nothing reads the files back, a failure to write is
//! logged and ignored, and with the variable unset or empty the extension does nothing extra.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use xr_capture::wayland::Frame;

/// The environment variable that names the dump directory.
pub const DUMP_VARIABLE: &str = "GNOBLIN_XR_DUMP_DIR";

/// Where uploaded pictures are written, and which Views have had their first picture written.
pub struct Dump {
    dir: PathBuf,
    first: BTreeSet<u64>,
}

impl Dump {
    /// The dump for the variable's value, or `None` when it is unset or empty. The directory is created.
    pub fn from_value(value: Option<String>) -> Option<Self> {
        let dir = PathBuf::from(value.filter(|value| !value.is_empty())?);
        if let Err(error) = std::fs::create_dir_all(&dir) {
            eprintln!("gnoblin-xr: {DUMP_VARIABLE}: cannot create {}: {error}", dir.display());
            return None;
        }
        Some(Self {
            dir,
            first: BTreeSet::new(),
        })
    }

    /// Writes the picture just uploaded for `view`.
    pub fn record(&mut self, view: u64, frame: &Frame) {
        let bytes = ppm(frame);
        if self.first.insert(view) {
            write(&self.dir.join(format!("view-{view}-first.ppm")), &bytes);
        }
        write(&self.dir.join(format!("view-{view}-latest.ppm")), &bytes);
    }
}

fn write(path: &Path, bytes: &[u8]) {
    if let Err(error) = std::fs::write(path, bytes) {
        eprintln!("gnoblin-xr: {DUMP_VARIABLE}: cannot write {}: {error}", path.display());
    }
}

/// `frame` as a binary PPM. Both `wl_shm` formats are little-endian words, so a pixel's bytes are blue, green, red,
/// then alpha or padding; the alpha is dropped.
pub fn ppm(frame: &Frame) -> Vec<u8> {
    let (width, height) = (frame.width as usize, frame.height as usize);
    let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
    out.reserve(width * height * 3);
    for row in 0..height {
        let start = row * frame.stride as usize;
        let Some(pixels) = frame.bytes.get(start..start + width * 4) else {
            break;
        };
        for &[blue, green, red, _] in pixels.as_chunks::<4>().0 {
            out.extend_from_slice(&[red, green, blue]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use xr_capture::ShmFormat;

    use super::*;

    fn frame(width: u32, height: u32, stride: u32, bytes: Vec<u8>) -> Frame {
        Frame {
            width,
            height,
            stride,
            format: ShmFormat::Xrgb8888,
            bytes,
            serial: 1,
        }
    }

    #[test]
    fn a_picture_becomes_rgb_triples_and_the_row_padding_is_dropped() {
        // Two pixels per row, a stride of 12 bytes (4 of padding): blue-first words.
        let bytes = vec![
            1, 2, 3, 255, 4, 5, 6, 0, 9, 9, 9, 9, //
            7, 8, 9, 255, 10, 11, 12, 0, 9, 9, 9, 9,
        ];
        let out = ppm(&frame(2, 2, 12, bytes));
        assert_eq!(&out[..11], b"P6\n2 2\n255\n");
        assert_eq!(&out[11..], &[3, 2, 1, 6, 5, 4, 9, 8, 7, 12, 11, 10]);
    }

    #[test]
    fn a_short_buffer_gives_a_short_picture_and_not_a_panic() {
        let out = ppm(&frame(2, 2, 8, vec![0; 8]));
        assert_eq!(out.len(), 11 + 6);
    }

    #[test]
    fn the_first_picture_is_kept_and_the_latest_follows() {
        let dir = std::env::temp_dir().join(format!("xr-dump-test-{}", std::process::id()));
        let mut dump = Dump::from_value(Some(dir.to_string_lossy().into_owned())).expect("a dump");
        dump.record(7, &frame(1, 1, 4, vec![0, 0, 255, 255]));
        dump.record(7, &frame(1, 1, 4, vec![255, 0, 0, 255]));
        let first = std::fs::read(dir.join("view-7-first.ppm")).expect("first");
        let latest = std::fs::read(dir.join("view-7-latest.ppm")).expect("latest");
        std::fs::remove_dir_all(&dir).expect("cleanup");
        assert_eq!(&first[first.len() - 3..], &[255, 0, 0]);
        assert_eq!(&latest[latest.len() - 3..], &[0, 0, 255]);
    }

    #[test]
    fn an_unset_or_empty_variable_dumps_nothing() {
        assert!(Dump::from_value(None).is_none());
        assert!(Dump::from_value(Some(String::new())).is_none());
    }
}
