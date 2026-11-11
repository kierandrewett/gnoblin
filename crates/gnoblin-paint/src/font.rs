//! The font source: system fonts found by family, and one embedded face.
//!
//! # Where fonts come from
//!
//! [`FontSet::system`] scans the standard font directories (`/usr/share/fonts`, `/usr/local/share/fonts`,
//! `~/.local/share/fonts`, `~/.fonts`, `$XDG_DATA_HOME/fonts` and each `$XDG_DATA_DIRS/fonts`) for `.ttf`, `.otf`,
//! `.ttc` and `.otc` files. The scan reads only each file's table directory, `name` table and `OS/2` header, so a
//! machine with thousands of fonts is indexed without loading them; a face's bytes are read the first time a
//! glyph of it is drawn or measured. [`FontSet::scan`] does the same for directories the caller names, which is
//! how a test supplies its own fonts.
//!
//! Every set also holds the embedded face, a subset of Droid Sans (Apache-2.0, `fonts/README.md`) covering Basic
//! Latin, Latin-1 and common punctuation. It is the last resort when no system font matches and the only face of
//! [`FontSet::embedded`], the set the golden tests draw with, so a test gives the same pixels on every machine.
//!
//! # Choosing a face
//!
//! [`FontSet::resolve`] takes the `font` string of a text op. A family name matches case-insensitively against
//! the typographic family (`name` id 16, else id 1). Among a family's faces the upright one closest to regular
//! weight wins. `sans`, `serif` and `monospace` are generic names: each walks a list of well-known families and
//! takes the first one installed. A name that matches nothing, and a generic name with no installed family,
//! resolves to the embedded face. A glyph the chosen face lacks is taken from the embedded face, else drawn as
//! the chosen face's `.notdef`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use skrifa::prelude::{FontRef, GlyphId, MetadataProvider};

use crate::log;

/// The embedded face's bytes.
const EMBEDDED: &[u8] = include_bytes!("../fonts/DroidSans-subset.ttf");

/// The embedded face's family name.
pub const EMBEDDED_FAMILY: &str = "Droid Sans";

/// How deep a font directory scan descends.
const SCAN_DEPTH: usize = 8;

/// The most bytes read from one font file's `name` table; a larger one is not read.
const MAX_NAME_TABLE: u32 = 1 << 20;

/// The well-known families a generic name tries, in order.
const SANS: &[&str] = &[
    "Adwaita Sans",
    "Cantarell",
    "Inter",
    "Noto Sans",
    "DejaVu Sans",
    "Liberation Sans",
    "Roboto",
    "Open Sans",
    "Arimo",
];
const SERIF: &[&str] = &[
    "Noto Serif",
    "DejaVu Serif",
    "Liberation Serif",
    "FaceSource Serif 4",
    "Tinos",
];
const MONOSPACE: &[&str] = &[
    "Adwaita Mono",
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "FaceSource Code Pro",
    "Cousine",
];

/// A face of a [`FontSet`], by position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FaceId(usize);

#[derive(Debug)]
enum FaceSource {
    Embedded,
    File(PathBuf),
}

#[derive(Debug)]
struct Face {
    source: FaceSource,
    /// The face's index within a collection, 0 for a plain font file.
    index: u32,
    family: String,
    weight: u16,
    italic: bool,
    /// The file's bytes, read the first time they are needed. `None` records a read that failed.
    data: OnceLock<Option<Arc<Vec<u8>>>>,
}

impl Face {
    fn bytes(&self) -> Option<&Arc<Vec<u8>>> {
        self.data
            .get_or_init(|| match &self.source {
                FaceSource::Embedded => Some(Arc::new(EMBEDDED.to_vec())),
                FaceSource::File(path) => match std::fs::read(path) {
                    Ok(bytes) => Some(Arc::new(bytes)),
                    Err(error) => {
                        log::warn!(area = "paint", path = %path.display(), %error, "a font file could not be read");
                        None
                    }
                },
            })
            .as_ref()
    }
}

/// The fonts a painter draws with.
#[derive(Debug)]
pub struct FontSet {
    faces: Vec<Face>,
    embedded: FaceId,
}

impl FontSet {
    /// A set holding only the embedded face. Every family resolves to it, so text drawn with it is the same on every
    /// machine.
    #[must_use]
    pub fn embedded() -> Self {
        Self::scan(&[])
    }

    /// The embedded face and every font in the standard system directories.
    #[must_use]
    pub fn system() -> Self {
        Self::scan(&system_directories())
    }

    /// The embedded face and every font under `directories`, which are walked in name order.
    #[must_use]
    pub fn scan(directories: &[PathBuf]) -> Self {
        let mut faces = Vec::new();
        let mut files = Vec::new();
        for directory in directories {
            collect_files(directory, 0, &mut files);
        }
        files.sort();
        files.dedup();
        for file in files {
            for found in read_faces(&file) {
                faces.push(Face {
                    source: FaceSource::File(file.clone()),
                    index: found.index,
                    family: found.family,
                    weight: found.weight,
                    italic: found.italic,
                    data: OnceLock::new(),
                });
            }
        }
        faces.push(Face {
            source: FaceSource::Embedded,
            index: 0,
            family: EMBEDDED_FAMILY.to_owned(),
            weight: 400,
            italic: false,
            data: OnceLock::new(),
        });
        let embedded = FaceId(faces.len() - 1);
        Self { faces, embedded }
    }

    /// The embedded face.
    #[must_use]
    pub const fn embedded_face(&self) -> FaceId {
        self.embedded
    }

    /// The family names of every face, in scan order with the embedded face last. A family with several faces
    /// appears once per face.
    pub fn families(&self) -> impl Iterator<Item = &str> {
        self.faces.iter().map(|face| face.family.as_str())
    }

    /// The family name of a face.
    #[must_use]
    pub fn family(&self, face: FaceId) -> &str {
        &self.faces[face.0].family
    }

    /// The face a text op's `font` string names. See the module documentation for the rules.
    #[must_use]
    pub fn resolve(&self, font: &str) -> FaceId {
        let wanted = normalise(font);
        let list: &[&str] = match wanted.as_str() {
            "sans" | "sans-serif" | "system-ui" | "ui" | "" => SANS,
            "serif" => SERIF,
            "monospace" | "mono" => MONOSPACE,
            _ => {
                return self
                    .family_face(&wanted)
                    .or_else(|| self.generic(SANS))
                    .unwrap_or(self.embedded);
            }
        };
        self.generic(list).unwrap_or(self.embedded)
    }

    fn generic(&self, families: &[&str]) -> Option<FaceId> {
        families.iter().find_map(|family| self.family_face(&normalise(family)))
    }

    /// The upright face of a normalised family nearest regular weight.
    fn family_face(&self, family: &str) -> Option<FaceId> {
        self.faces
            .iter()
            .enumerate()
            .filter(|(_, face)| normalise(&face.family) == family)
            .min_by_key(|(position, face)| (face.italic, face.weight.abs_diff(400), *position))
            .map(|(position, _)| FaceId(position))
    }

    /// Runs `f` on the parsed face, reading its file first when this is its first use. `None` when the file cannot be
    /// read or parsed.
    pub fn with_font<R>(&self, face: FaceId, f: impl FnOnce(&FontRef<'_>) -> R) -> Option<R> {
        let entry = &self.faces[face.0];
        let bytes = entry.bytes()?;
        match FontRef::from_index(bytes.as_slice(), entry.index) {
            Ok(font) => Some(f(&font)),
            Err(error) => {
                log::warn!(area = "paint", family = %entry.family, %error, "a font could not be parsed");
                None
            }
        }
    }

    /// The glyph for `character`: the chosen face's, else the embedded face's, else the chosen face's `.notdef`.
    #[must_use]
    pub fn glyph(&self, face: FaceId, character: char) -> Option<(FaceId, GlyphId)> {
        let mapped = self
            .with_font(face, |font| font.charmap().map(character))
            .flatten()
            .filter(|glyph| glyph.to_u32() != 0);
        if let Some(glyph) = mapped {
            return Some((face, glyph));
        }
        if face != self.embedded
            && let Some(glyph) = self
                .with_font(self.embedded, |font| font.charmap().map(character))
                .flatten()
                .filter(|glyph| glyph.to_u32() != 0)
        {
            return Some((self.embedded, glyph));
        }
        self.with_font(face, |_| (face, GlyphId::NOTDEF))
    }
}

/// A family name as compared: lower case, with runs of space, `-` and `_` collapsed to one space.
fn normalise(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for character in name.trim().chars() {
        if character.is_whitespace() || character == '_' {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.extend(character.to_lowercase());
        }
    }
    out
}

/// The directories the system scan covers.
fn system_directories() -> Vec<PathBuf> {
    let mut directories = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(home) = &home {
        directories.push(home.join(".fonts"));
    }
    match std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        Some(data) => directories.push(PathBuf::from(data).join("fonts")),
        None => {
            if let Some(home) = &home {
                directories.push(home.join(".local/share/fonts"));
            }
        }
    }
    if let Some(dirs) = std::env::var_os("XDG_DATA_DIRS") {
        for data in std::env::split_paths(&dirs) {
            if !data.as_os_str().is_empty() {
                directories.push(data.join("fonts"));
            }
        }
    }
    directories
}

fn collect_files(directory: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            collect_files(&path, depth + 1, out);
        } else if metadata.is_file() && is_font_file(&path) {
            out.push(path);
        }
    }
}

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["ttf", "otf", "ttc", "otc"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

/// One face found in a font file.
struct Found {
    index: u32,
    family: String,
    weight: u16,
    italic: bool,
}

/// The faces of one font file, or none when it is not an sfnt font this reader understands.
fn read_faces(path: &Path) -> Vec<Found> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let mut header = [0_u8; 12];
    if file.read_exact(&mut header).is_err() {
        return Vec::new();
    }
    let offsets: Vec<u32> = if &header[0..4] == b"ttcf" {
        let count = u32::from_be_bytes([header[8], header[9], header[10], header[11]]).min(256);
        let mut table = vec![0_u8; count as usize * 4];
        if file.read_exact(&mut table).is_err() {
            return Vec::new();
        }
        table
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_be_bytes(*chunk))
            .collect()
    } else {
        vec![0]
    };
    offsets
        .into_iter()
        .enumerate()
        .filter_map(|(index, offset)| {
            let (family, weight, italic) = read_face(&mut file, offset)?;
            Some(Found {
                index: u32::try_from(index).ok()?,
                family,
                weight,
                italic,
            })
        })
        .collect()
}

/// The family, weight and italic flag of the sfnt at `offset`.
fn read_face(file: &mut File, offset: u32) -> Option<(String, u16, bool)> {
    file.seek(SeekFrom::Start(u64::from(offset))).ok()?;
    let mut directory = [0_u8; 12];
    file.read_exact(&mut directory).ok()?;
    if !matches!(&directory[0..4], [0, 1, 0, 0] | b"OTTO" | b"true") {
        return None;
    }
    let tables = usize::from(u16::from_be_bytes([directory[4], directory[5]]));
    let mut records = vec![0_u8; tables * 16];
    file.read_exact(&mut records).ok()?;
    let table = |tag: &[u8; 4]| {
        records
            .as_chunks::<16>()
            .0
            .iter()
            .find(|record| &record[0..4] == tag)
            .map(|record| {
                (
                    u32::from_be_bytes([record[8], record[9], record[10], record[11]]),
                    u32::from_be_bytes([record[12], record[13], record[14], record[15]]),
                )
            })
    };
    let (name_offset, name_length) = table(b"name")?;
    if name_length > MAX_NAME_TABLE {
        return None;
    }
    file.seek(SeekFrom::Start(u64::from(name_offset))).ok()?;
    let mut name = vec![0_u8; name_length as usize];
    file.read_exact(&mut name).ok()?;
    let family = family_name(&name)?;
    let (mut weight, mut italic) = (400_u16, false);
    if let Some((os2_offset, os2_length)) = table(b"OS/2")
        && os2_length >= 64
    {
        let mut os2 = [0_u8; 64];
        file.seek(SeekFrom::Start(u64::from(os2_offset))).ok()?;
        file.read_exact(&mut os2).ok()?;
        weight = u16::from_be_bytes([os2[4], os2[5]]);
        let selection = u16::from_be_bytes([os2[62], os2[63]]);
        italic = selection & 1 != 0 || selection & (1 << 9) != 0;
    }
    Some((family, weight, italic))
}

/// The typographic family (id 16), else the family (id 1), from a `name` table.
fn family_name(table: &[u8]) -> Option<String> {
    let count = usize::from(u16::from_be_bytes([*table.get(2)?, *table.get(3)?]));
    let strings = usize::from(u16::from_be_bytes([*table.get(4)?, *table.get(5)?]));
    let record = |index: usize| -> Option<[u16; 6]> {
        let start = 6 + index * 12;
        let bytes = table.get(start..start + 12)?;
        let mut fields = [0_u16; 6];
        for (field, chunk) in fields.iter_mut().zip(bytes.as_chunks::<2>().0) {
            *field = u16::from_be_bytes([chunk[0], chunk[1]]);
        }
        Some(fields)
    };
    for wanted in [16_u16, 1] {
        let mut best: Option<(u8, String)> = None;
        for index in 0..count {
            let Some([platform, encoding, language, id, length, offset]) = record(index) else {
                break;
            };
            if id != wanted {
                continue;
            }
            let start = strings + usize::from(offset);
            let Some(raw) = table.get(start..start + usize::from(length)) else {
                continue;
            };
            // Rank: Windows English, other Windows, Mac Roman.
            let (rank, text) = match (platform, encoding) {
                (3, 1 | 10) => {
                    let units: Vec<u16> = raw
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| u16::from_be_bytes(*pair))
                        .collect();
                    (if language == 0x0409 { 0 } else { 1 }, String::from_utf16_lossy(&units))
                }
                (1, 0) => (2, raw.iter().map(|byte| char::from(*byte)).collect()),
                _ => continue,
            };
            if !text.is_empty() && best.as_ref().is_none_or(|(held, _)| rank < *held) {
                best = Some((rank, text));
            }
        }
        if let Some((_, family)) = best {
            return Some(family);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_set_resolves_every_name_to_the_embedded_face() {
        let fonts = FontSet::embedded();
        for name in ["sans", "serif", "monospace", "Droid Sans", "No Such Family", ""] {
            assert_eq!(fonts.resolve(name), fonts.embedded_face(), "{name}");
        }
        assert_eq!(fonts.family(fonts.embedded_face()), EMBEDDED_FAMILY);
    }

    #[test]
    fn the_embedded_face_parses_and_covers_latin() {
        let fonts = FontSet::embedded();
        for character in ['A', 'z', '0', ' ', 'é', '…'] {
            let (face, glyph) = fonts
                .glyph(fonts.embedded_face(), character)
                .expect("the embedded face loads");
            assert_eq!(face, fonts.embedded_face());
            assert_ne!(glyph.to_u32(), 0, "{character} has a glyph");
        }
        let (_, notdef) = fonts.glyph(fonts.embedded_face(), '界').expect("the face loads");
        assert_eq!(notdef.to_u32(), 0, "a character outside the subset is .notdef");
    }

    #[test]
    fn family_names_compare_without_case_or_spacing() {
        assert_eq!(normalise("  DejaVu   Sans "), "dejavu sans");
        assert_eq!(normalise("Noto_Sans"), "noto sans");
    }

    #[test]
    fn a_scan_of_a_directory_with_a_font_finds_its_family() {
        let directory = std::env::temp_dir().join(format!("gnoblin-paint-fonts-{}", std::process::id()));
        std::fs::create_dir_all(directory.join("nested")).expect("a scratch directory");
        std::fs::write(directory.join("nested/Droid.TTF"), EMBEDDED).expect("a copy of the font");
        std::fs::write(directory.join("notes.txt"), b"not a font").expect("a stray file");
        std::fs::write(directory.join("broken.ttf"), b"junk").expect("a broken font");
        let fonts = FontSet::scan(std::slice::from_ref(&directory));
        let families: Vec<&str> = fonts.families().collect();
        assert_eq!(families, [EMBEDDED_FAMILY, EMBEDDED_FAMILY]);
        let found = fonts.resolve("droid  sans");
        assert_ne!(
            found,
            fonts.embedded_face(),
            "the scanned face is found before the embedded one"
        );
        assert!(fonts.glyph(found, 'A').is_some_and(|(face, _)| face == found));
        std::fs::remove_dir_all(&directory).ok();
    }
}
