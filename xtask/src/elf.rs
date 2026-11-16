//! A small reader for the dynamic facts of an ELF shared object or executable.
//!
//! `dist-check` asks of a file: what is its SONAME, what does it NEED, what does it export and import, is it linked
//! `NODELETE`, and what is the newest glibc symbol version it requires. `readelf` and `nm` answer the same questions, but
//! parsing their text output is fragile and a machine without binutils cannot run the check. This module reads the ELF
//! structures directly (64-bit little endian, which is every target Gnoblin builds for) and the tests compare it with
//! `readelf` and `nm` on real files, so it cannot drift from them.

use std::fs;
use std::path::Path;

const SHT_DYNAMIC: u32 = 6;
const SHT_DYNSYM: u32 = 11;
const SHT_GNU_VERNEED: u32 = 0x6fff_fffe;
const DT_NEEDED: i64 = 1;
const DT_SONAME: i64 = 14;
const DT_RPATH: i64 = 15;
const DT_RUNPATH: i64 = 29;
const DT_FLAGS_1: i64 = 0x6fff_fffb;
/// `DF_1_NODELETE` in `DT_FLAGS_1`.
pub const DF_1_NODELETE: u64 = 0x8;
const SHN_UNDEF: u16 = 0;

/// The dynamic facts of one ELF file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Elf {
    /// `DT_SONAME`.
    pub soname: Option<String>,
    /// `DT_NEEDED` entries, in order.
    pub needed: Vec<String>,
    /// `DT_RUNPATH`.
    pub runpath: Option<String>,
    /// `DT_RPATH`.
    pub rpath: Option<String>,
    /// `DT_FLAGS_1`.
    pub flags_1: u64,
    /// Names of dynamic symbols this file defines (version suffixes removed), sorted.
    pub defined: Vec<String>,
    /// Names of dynamic symbols this file needs from elsewhere, sorted.
    pub undefined: Vec<String>,
    /// The newest `GLIBC_x.y` symbol version the file requires.
    pub glibc: Option<(u32, u32)>,
    /// `e_type`: 2 for an executable, 3 for a shared object or position independent executable.
    pub elf_type: u16,
}

impl Elf {
    /// True when the file is linked so it is never unloaded.
    pub fn is_nodelete(&self) -> bool {
        self.flags_1 & DF_1_NODELETE != 0
    }
}

struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn u16(&self, at: usize) -> Result<u16, String> {
        let slice = self.0.get(at..at + 2).ok_or("truncated ELF")?;
        Ok(u16::from_le_bytes([slice[0], slice[1]]))
    }

    fn u32(&self, at: usize) -> Result<u32, String> {
        let slice = self.0.get(at..at + 4).ok_or("truncated ELF")?;
        Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    }

    fn u64(&self, at: usize) -> Result<u64, String> {
        let slice = self.0.get(at..at + 8).ok_or("truncated ELF")?;
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(slice);
        Ok(u64::from_le_bytes(bytes))
    }

    /// The NUL-terminated string at `at` in the string table that starts at `table`.
    fn string(&self, table: usize, at: usize) -> Result<String, String> {
        let start = table.checked_add(at).ok_or("bad string offset")?;
        let rest = self.0.get(start..).ok_or("string offset past the end of the file")?;
        let end = rest.iter().position(|&byte| byte == 0).ok_or("unterminated string")?;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
}

struct Section {
    kind: u32,
    offset: usize,
    size: usize,
    link: usize,
}

fn sections(file: &Bytes<'_>) -> Result<Vec<Section>, String> {
    let table = usize::try_from(file.u64(40)?).map_err(|_| "bad section table offset")?;
    let entry_size = usize::from(file.u16(58)?);
    let count = usize::from(file.u16(60)?);
    if count > 0 && entry_size < 64 {
        return Err("ELF section header entries are too small".to_owned());
    }
    (0..count)
        .map(|index| {
            let at = table + index * entry_size;
            Ok(Section {
                kind: file.u32(at + 4)?,
                offset: usize::try_from(file.u64(at + 24)?).map_err(|_| "bad section offset")?,
                size: usize::try_from(file.u64(at + 32)?).map_err(|_| "bad section size")?,
                link: file.u32(at + 40)? as usize,
            })
        })
        .collect()
}

/// Reads the dynamic facts of the ELF file `bytes`.
pub fn parse(bytes: &[u8]) -> Result<Elf, String> {
    if bytes.get(..4) != Some(b"\x7fELF") {
        return Err("not an ELF file".to_owned());
    }
    if bytes.get(4) != Some(&2) || bytes.get(5) != Some(&1) {
        return Err("only 64-bit little-endian ELF is supported".to_owned());
    }
    let file = Bytes(bytes);
    let mut elf = Elf {
        elf_type: file.u16(16)?,
        ..Elf::default()
    };
    let sections = sections(&file)?;
    let strtab = |section: &Section| {
        sections
            .get(section.link)
            .map(|table| table.offset)
            .ok_or("bad string table link")
    };

    for section in &sections {
        match section.kind {
            SHT_DYNAMIC => {
                let names = strtab(section)?;
                for entry in 0..section.size / 16 {
                    let at = section.offset + entry * 16;
                    let tag = file.u64(at)? as i64;
                    let value = file.u64(at + 8)?;
                    match tag {
                        0 => break,
                        DT_NEEDED => elf.needed.push(file.string(names, value as usize)?),
                        DT_SONAME => elf.soname = Some(file.string(names, value as usize)?),
                        DT_RUNPATH => elf.runpath = Some(file.string(names, value as usize)?),
                        DT_RPATH => elf.rpath = Some(file.string(names, value as usize)?),
                        DT_FLAGS_1 => elf.flags_1 = value,
                        _ => {}
                    }
                }
            }
            SHT_DYNSYM => {
                let names = strtab(section)?;
                for entry in 1..section.size / 24 {
                    let at = section.offset + entry * 24;
                    let name = file.string(names, file.u32(at)? as usize)?;
                    if name.is_empty() {
                        continue;
                    }
                    if file.u16(at + 6)? == SHN_UNDEF {
                        elf.undefined.push(name);
                    } else {
                        elf.defined.push(name);
                    }
                }
            }
            SHT_GNU_VERNEED => {
                let names = strtab(section)?;
                let mut need = section.offset;
                loop {
                    let aux_count = usize::from(file.u16(need + 2)?);
                    let mut aux = need + file.u32(need + 8)? as usize;
                    for _ in 0..aux_count {
                        let version = file.string(names, file.u32(aux + 8)? as usize)?;
                        let newer = version
                            .strip_prefix("GLIBC_")
                            .and_then(parse_glibc)
                            .filter(|parsed| elf.glibc.is_none_or(|known| *parsed > known));
                        if newer.is_some() {
                            elf.glibc = newer;
                        }
                        aux += file.u32(aux + 12)? as usize;
                    }
                    let next = file.u32(need + 12)? as usize;
                    if next == 0 {
                        break;
                    }
                    need += next;
                }
            }
            _ => {}
        }
    }
    elf.defined.sort();
    elf.undefined.sort();
    Ok(elf)
}

/// `2.34` or `2.2.5` as `(2, 34)`; versions with a third component keep the first two.
fn parse_glibc(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// Reads the dynamic facts of the ELF file at `path`.
pub fn read(path: &Path) -> Result<Elf, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    parse(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn binutils(tool: &str, args: &[&str], path: &Path) -> String {
        let output = Command::new(tool)
            .args(args)
            .arg(path)
            .output()
            .unwrap_or_else(|error| panic!("`{tool}` is needed to cross-check the ELF reader (binutils): {error}"));
        assert!(
            output.status.success(),
            "{tool}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn bracketed(line: &str) -> String {
        let start = line.find('[').unwrap();
        let end = line.rfind(']').unwrap();
        line[start + 1..end].to_owned()
    }

    #[test]
    fn a_file_that_is_not_elf_is_refused() {
        assert!(parse(b"hello").is_err());
        assert!(parse(b"\x7fELF\x01\x01\x01").unwrap_err().contains("64-bit"));
        assert!(
            parse(b"\x7fELF\x02\x01\x01\x00").is_err(),
            "a header cut short is an error, not a panic"
        );
    }

    #[test]
    fn glibc_versions_compare_by_number_not_text() {
        assert_eq!(parse_glibc("2.34"), Some((2, 34)));
        assert_eq!(parse_glibc("2.2.5"), Some((2, 2)));
        assert!(parse_glibc("2.34").unwrap() > parse_glibc("2.5").unwrap());
        assert_eq!(parse_glibc("PRIVATE"), None);
    }

    /// The test binary is a real dynamically linked executable: compare with `readelf` and `nm` on it.
    #[test]
    fn the_reader_agrees_with_readelf_and_nm_on_a_real_executable() {
        let exe = std::env::current_exe().unwrap();
        let elf = read(&exe).unwrap();
        assert!(!elf.needed.is_empty());

        let dynamic = binutils("readelf", &["-d", "-W"], &exe);
        let needed: Vec<String> = dynamic
            .lines()
            .filter(|line| line.contains("(NEEDED)"))
            .map(bracketed)
            .collect();
        assert_eq!(elf.needed, needed);
        let runpath = dynamic.lines().find(|line| line.contains("(RUNPATH)")).map(bracketed);
        assert_eq!(elf.runpath, runpath);

        let undefined: std::collections::BTreeSet<String> = binutils("nm", &["-D", "--undefined-only"], &exe)
            .lines()
            .filter_map(|line| line.split_whitespace().last())
            .map(|name| name.split('@').next().unwrap().to_owned())
            .collect();
        let ours: std::collections::BTreeSet<String> = elf.undefined.iter().cloned().collect();
        assert_eq!(ours, undefined);

        // The newest glibc version readelf lists as needed.
        let version_info = binutils("readelf", &["-V", "-W"], &exe);
        let newest = version_info
            .lines()
            .filter_map(|line| line.split("Name: GLIBC_").nth(1))
            .filter_map(|rest| parse_glibc(rest.split_whitespace().next()?))
            .max();
        assert_eq!(elf.glibc, newest);
    }
}
