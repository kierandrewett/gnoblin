//! Where `libgnoblin-std.so.1` is, for the two places that open it: a native shell that links it, and the compositor of a
//! session with no shell.
//!
//! The library is looked for beside the core (`<libdir>/libgnoblin-std.so.1`, where an install and a build tree put it), then
//! in `$GNOBLIN_STD_DIR`, then by its name through the dynamic loader's own search ([`candidates`]).
//!
//! A shell that uses std records `NEEDED libgnoblin-std.so.1` and, being a shell, no run path. The launcher finds the core
//! through its own run path, but the loader does not use an executable's run path for the dependencies of a library that is
//! `dlopen`ed, so under a prefix its default search does not reach (`/usr/local`, `/opt`) the shell would fail to load for want
//! of std. [`needs_std`] reads the shell's `NEEDED` list from the file, so the host can open std where the install put it
//! before it opens the shell, and a shell that links the core only never causes std to be opened.

use std::fs::File;
use std::os::unix::fs::FileExt as _;
use std::path::{Path, PathBuf};

use crate::runtime::library_containing;

/// The file name of the std library, which a std shell records as `NEEDED`.
pub const SONAME: &str = "libgnoblin-std.so.1";

/// The environment variable that names the directory std is looked for in first.
pub const DIR_VARIABLE: &str = "GNOBLIN_STD_DIR";

/// The places std is looked for, in order: `$GNOBLIN_STD_DIR`, the directory of the library this code runs in (the core's), then
/// the bare name, which the dynamic loader searches by its own rules.
#[must_use]
pub fn candidates() -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Some(dir) = std::env::var_os(DIR_VARIABLE).filter(|dir| !dir.is_empty()) {
        found.push(PathBuf::from(dir).join(SONAME));
    }
    // The loader reports the core by the path its run path expanded to (`<prefix>/bin/../lib`): name the directory plainly, as
    // a message that says where std was looked for has to.
    if let Some(core) = library_containing(candidates as fn() -> Vec<PathBuf> as *const ())
        && let Some(dir) = core.parent()
    {
        found.push(dir.canonicalize().unwrap_or_else(|_| dir.to_owned()).join(SONAME));
    }
    found.push(PathBuf::from(SONAME));
    found
}

/// Whether the ELF file at `module` lists the std library among its `NEEDED` libraries. `false` for a file that cannot be read
/// or is not 64-bit little-endian ELF: the loader then reports what is wrong with it.
#[must_use]
pub fn needs_std(module: &Path) -> bool {
    File::open(module)
        .ok()
        .and_then(|file| needed(&file))
        .is_some_and(|names| names.iter().any(|name| name == SONAME))
}

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_STRTAB: u64 = 5;
const DT_STRSZ: u64 = 10;

/// What a program header says about one segment.
struct ProgramHeader {
    kind: u32,
    offset: u64,
    vaddr: u64,
    file_size: u64,
}

fn read_at(file: &File, offset: u64, len: u64) -> Option<Vec<u8>> {
    // A header that claims a table larger than any shell is corrupt; refuse it rather than allocate for it.
    const LIMIT: u64 = 16 << 20;
    if len > LIMIT {
        return None;
    }
    let mut bytes = vec![0; usize::try_from(len).ok()?];
    file.read_exact_at(&mut bytes, offset).ok()?;
    Some(bytes)
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// The `DT_NEEDED` names of a 64-bit little-endian ELF file, read the way the loader does: the dynamic segment named by the
/// program headers, and its string table found through the load segments. Only those pieces of the file are read.
fn needed(file: &File) -> Option<Vec<String>> {
    let header = read_at(file, 0, 64)?;
    if header.get(..6)? != [0x7f, b'E', b'L', b'F', 2, 1] {
        return None;
    }
    let table = u64_at(&header, 32)?;
    let entry = u64::from(u16_at(&header, 54)?);
    let count = u64::from(u16_at(&header, 56)?);
    if entry < 56 {
        return None;
    }
    let raw = read_at(file, table, entry.checked_mul(count)?)?;
    let mut segments = Vec::new();
    for index in 0..usize::try_from(count).ok()? {
        let at = index * usize::try_from(entry).ok()?;
        segments.push(ProgramHeader {
            kind: u32_at(&raw, at)?,
            offset: u64_at(&raw, at + 8)?,
            vaddr: u64_at(&raw, at + 16)?,
            file_size: u64_at(&raw, at + 32)?,
        });
    }
    let dynamic = segments.iter().find(|segment| segment.kind == PT_DYNAMIC)?;
    let entries = read_at(file, dynamic.offset, dynamic.file_size)?;
    let (mut names, mut strtab, mut strsz) = (Vec::new(), None, None);
    for pair in entries.as_chunks::<16>().0 {
        let (tag, value) = (u64_at(pair, 0)?, u64_at(pair, 8)?);
        match tag {
            DT_NULL => break,
            DT_NEEDED => names.push(value),
            DT_STRTAB => strtab = Some(value),
            DT_STRSZ => strsz = Some(value),
            _ => {}
        }
    }
    if names.is_empty() {
        return Some(Vec::new());
    }
    // `DT_STRTAB` is a virtual address: the file offset is where the load segment that holds it starts, plus the distance in.
    let strtab = strtab?;
    let holder = segments.iter().find(|segment| {
        segment.kind == PT_LOAD && strtab >= segment.vaddr && strtab - segment.vaddr < segment.file_size
    })?;
    let strings = read_at(file, holder.offset + (strtab - holder.vaddr), strsz?)?;
    names
        .into_iter()
        .map(|at| {
            let rest = strings.get(usize::try_from(at).ok()?..)?;
            let end = rest.iter().position(|&byte| byte == 0)?;
            Some(String::from_utf8_lossy(&rest[..end]).into_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_needed_list_of_a_real_executable_is_read_from_its_headers() {
        let exe = File::open(std::env::current_exe().expect("this test's executable")).expect("it opens");
        let names = needed(&exe).expect("a Linux test binary is 64-bit little-endian ELF");
        assert!(
            names.iter().all(|name| name.contains(".so")),
            "every entry is a library name: {names:?}"
        );
        // The test binary links std and the C library dynamically on a glibc target; a static or musl one lists less. What
        // matters is that a list that is there is read whole, so compare with the loader's own account when it can give one.
        if let Ok(output) = std::process::Command::new("readelf")
            .arg("-d")
            .arg(std::env::current_exe().expect("this test's executable"))
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            let expected: Vec<String> = text
                .lines()
                .filter(|line| line.contains("(NEEDED)"))
                .filter_map(|line| line.split('[').nth(1)?.split(']').next().map(str::to_owned))
                .collect();
            assert_eq!(names, expected, "readelf -d");
        }
    }

    #[test]
    fn a_file_that_is_not_elf_or_is_cut_short_needs_nothing() {
        let dir = tempfile::tempdir().expect("a directory");
        let text = dir.path().join("shell.so");
        std::fs::write(
            &text,
            "not an elf file at all, but longer than an ELF header is: ".repeat(4),
        )
        .expect("written");
        assert!(!needs_std(&text));
        let exe = std::fs::read(std::env::current_exe().expect("this test's executable")).expect("readable");
        let cut = dir.path().join("cut.so");
        std::fs::write(&cut, &exe[..100]).expect("written");
        assert!(!needs_std(&cut));
        assert!(!needs_std(&dir.path().join("missing.so")));
    }
}
