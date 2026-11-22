//! Language dispatch: which module a path is, which runtime runs a script, and the runtime libraries the loader keeps open.
//!
//! A path is a native shell or a script, and a script is run by a
//! runtime library that exports `gnoruntime_entry_v1`:
//!
//! | step | test                                                    | result                                                     |
//! | ---- | ------------------------------------------------------- | ---------------------------------------------------------- |
//! | 1    | a language the caller named (`language = "..."`)        | wins: `native`, or the name of a language                  |
//! | 2    | the first four bytes are `\x7fELF`                      | native                                                     |
//! | 3    | the extension is `.so`                                  | native (an unreadable file is a load error, not a language) |
//! | 4    | `.py` `.js` `.mjs` `.ts` `.mts`                         | the runtime of that language                               |
//! | 5    | any other extension `.X`                                | `libgnoblin-runtime-X.so`, if a runtime directory has it   |
//!
//! A runtime directory is `$GNOBLIN_RUNTIME_DIR` and then `<core libdir>/gnoblin/`, found by asking the dynamic loader where the
//! code that is running lives, so a relocated prefix works. There is no other search and no registry.
//!
//! A runtime library is opened once, from a private copy as a native shell is, and stays open for the life of the process:
//! its interpreter is the shell instance's, and a reload of a script is a new generation inside it, not a new library.
#![allow(unsafe_code)]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::{CStr, CString};
use std::fs::{self, File};
use std::io::Read as _;
use std::os::unix::ffi::OsStrExt as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};

use gnoblin_core_abi::{self as abi, GnoblinRuntimeEntry};

use crate::log;
use crate::native::{LoadError, MissingRuntime, NativeLoader, RTLD_LOCAL, RTLD_NOW, dl_error, dlopen, dlsym};

/// The variable that names a directory searched first for runtime libraries.
pub const RUNTIME_DIR_VARIABLE: &str = "GNOBLIN_RUNTIME_DIR";

/// The extensions the loader knows, with the language each one means. The first column of the runtime's file name is the
/// language (`libgnoblin-runtime-<language>.so`).
const KNOWN: &[(&str, &str)] = &[
    ("py", "python"),
    ("js", "js"),
    ("mjs", "js"),
    ("ts", "js"),
    ("mts", "js"),
];

/// What a path is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleKind {
    /// A shared library that exports `gnoshell_entry_v1`.
    Native,
    /// A script that the runtime of `language` runs (`libgnoblin-runtime-<language>.so`), with the extension that chose it.
    Script {
        /// The runtime's name, the middle of its file name.
        language: String,
        /// The script's extension without the dot (or the language the caller named, when it named one).
        extension: String,
    },
}

/// Why a path is neither a native shell nor a script a runtime is installed for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownKind {
    /// The path.
    pub path: PathBuf,
}

/// Decides what `path` is: the language the caller named, else the content, else the extension.
///
/// `head` is the first bytes of the file (at least four when the file has that many), or empty when it could not be read.
///
/// # Errors
///
/// [`UnknownKind`]: no extension and not a library, so nothing says what to run it with.
pub fn detect(path: &Path, head: &[u8], language: Option<&str>) -> Result<ModuleKind, UnknownKind> {
    if let Some(named) = language.map(str::trim).filter(|named| !named.is_empty()) {
        if named == "native" {
            return Ok(ModuleKind::Native);
        }
        return Ok(script_of(named, named));
    }
    if head.starts_with(b"\x7fELF") {
        return Ok(ModuleKind::Native);
    }
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    match extension {
        "so" => Ok(ModuleKind::Native),
        "" => Err(UnknownKind { path: path.to_owned() }),
        other => Ok(script_of(other, other)),
    }
}

/// The script kind for a name that is either a known extension or a language name.
fn script_of(name: &str, extension: &str) -> ModuleKind {
    let language = KNOWN
        .iter()
        .find(|(ext, _)| *ext == name)
        .map_or(name, |(_, language)| *language);
    ModuleKind::Script {
        language: language.to_owned(),
        extension: extension.to_owned(),
    }
}

/// The extensions the loader names without looking for a runtime, for an error message: `.so .py .js .mjs .ts .mts`.
#[must_use]
pub fn known_extensions() -> String {
    let mut text = String::from(".so");
    for (extension, _) in KNOWN {
        text.push_str(" .");
        text.push_str(extension);
    }
    text
}

/// The `GnoblinCtxInfo.language` a script generation of `extension` is labelled with.
#[must_use]
pub fn language_constant(language: &str, extension: &str) -> u32 {
    match (language, extension) {
        ("python", _) => abi::LANG_PYTHON,
        ("js", "ts" | "mts") => abi::LANG_TS,
        ("js", _) => abi::LANG_JS,
        _ => abi::LANG_OTHER,
    }
}

/// The first bytes of `path`, up to four, or nothing when it cannot be read.
pub(crate) fn head_of(path: &Path) -> Vec<u8> {
    let mut head = vec![0; 4];
    let read = File::open(path).and_then(|mut file| file.read(&mut head)).unwrap_or(0);
    head.truncate(read);
    head
}

/// `Dl_info` of `dladdr`.
#[repr(C)]
struct DlInfo {
    file: *const c_char,
    base: *mut c_void,
    symbol_name: *const c_char,
    symbol_address: *mut c_void,
}

unsafe extern "C" {
    fn dladdr(address: *const c_void, info: *mut DlInfo) -> c_int;
}

/// The file of the mapping that holds the code at `address`: the shared library it is in, or the executable. `None` when the
/// dynamic loader cannot say. `address` is not read, so any function or static of the caller will do:
/// `library_containing(some_function as fn() as *const ())`.
///
/// A library asks this about itself, to find the installation it belongs to without assuming which process loaded it.
#[must_use]
pub fn library_containing(address: *const ()) -> Option<PathBuf> {
    let mut info = DlInfo {
        file: core::ptr::null(),
        base: core::ptr::null_mut(),
        symbol_name: core::ptr::null(),
        symbol_address: core::ptr::null_mut(),
    };
    // SAFETY: `dladdr` only looks the address up among the process's mappings and does not read it; `info` is a live local the
    // call fills in.
    let found = unsafe { dladdr(address.cast::<c_void>(), &raw mut info) };
    if found == 0 || info.file.is_null() {
        return None;
    }
    // SAFETY: `dladdr` returned a NUL-terminated path that lives as long as the mapping.
    let file = unsafe { CStr::from_ptr(info.file) };
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(file.to_bytes())))
}

/// The directory `<libdir>/gnoblin/` beside the library this code runs in: the core's, in a shipped build. `None` when the
/// dynamic loader cannot say.
fn sibling_directory() -> Option<PathBuf> {
    let library = library_containing(sibling_directory as fn() -> Option<PathBuf> as *const ())?;
    Some(library.parent()?.join("gnoblin"))
}

/// The directories runtimes are looked for in, in order.
#[must_use]
pub fn runtime_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os(RUNTIME_DIR_VARIABLE).filter(|dir| !dir.is_empty()) {
        dirs.push(PathBuf::from(dir));
    }
    dirs.extend(sibling_directory());
    dirs
}

/// A runtime library the loader has open: its mapping, its entry descriptor and what it said about itself.
pub struct LoadedRuntime {
    handle: usize,
    entry: &'static GnoblinRuntimeEntry,
    /// Where the library was found.
    pub path: PathBuf,
    /// The runtime's name (`js`).
    pub name: String,
    /// The extensions it runs.
    pub extensions: Vec<String>,
    /// Its version string.
    pub version: String,
}

impl LoadedRuntime {
    /// The runtime's entry descriptor.
    pub(crate) fn entry(&self) -> &'static GnoblinRuntimeEntry {
        self.entry
    }

    /// The `dlopen` handle of the mapping.
    pub(crate) fn handle(&self) -> usize {
        self.handle
    }
}

fn string_of(text: abi::GnoblinStr) -> String {
    if text.ptr.is_null() || text.len == 0 {
        return String::new();
    }
    // SAFETY: a runtime's strings are `len` bytes of string literals in its mapping, valid for the life of the module.
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(text.ptr, text.len) }).into_owned()
}

impl<E: 'static> NativeLoader<E> {
    /// The runtime library for `language`, loaded now when it was not yet.
    ///
    /// # Errors
    ///
    /// [`LoadError::NoRuntime`] when no runtime directory holds it, and the errors of opening a library and reading its entry.
    pub(crate) fn runtime_for(&'static self, language: &str, script: &Path) -> Result<Arc<LoadedRuntime>, LoadError> {
        let mut open = self.runtimes.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(found) = open
            .iter()
            .find(|runtime| runtime.name == language || runtime.extensions.iter().any(|ext| ext == language))
        {
            return Ok(Arc::clone(found));
        }
        let file = abi::runtime_file_name(language);
        let dirs = runtime_directories();
        let Some(library) = dirs
            .iter()
            .map(|dir| dir.join(&file))
            .find(|candidate| candidate.is_file())
        else {
            return Err(LoadError::NoRuntime(Box::new(MissingRuntime {
                path: script.to_owned(),
                language: language.to_owned(),
                file,
                searched: dirs,
            })));
        };
        let runtime = Arc::new(self.open_runtime(&library)?);
        log::info!(
            "runtime {} {} ({}) opened from {}",
            runtime.name,
            runtime.version,
            runtime.extensions.join(" "),
            runtime.path.display()
        );
        open.push(Arc::clone(&runtime));
        Ok(runtime)
    }

    /// Opens a runtime library from a private copy and reads its entry.
    fn open_runtime(&'static self, library: &Path) -> Result<LoadedRuntime, LoadError> {
        let target = self.unique_copy(library)?;
        let c_path = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| LoadError::Copy(format!("{}: the path contains a NUL byte", target.display())))?;
        // SAFETY: `c_path` is a valid NUL-terminated string for the call. `dlopen` runs the library's constructors, which is
        // what loading a runtime means.
        let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        let opened = if handle.is_null() { Err(dl_error()) } else { Ok(handle) };
        let _ = fs::remove_file(&target);
        let handle = opened.map_err(|message| LoadError::Dlopen {
            path: library.to_owned(),
            message: message.replace(&target.display().to_string(), &library.display().to_string()),
        })?;
        // SAFETY: `handle` is a live handle from `dlopen`, and the symbol name is a NUL-terminated literal.
        let symbol = unsafe { dlsym(handle, c"gnoruntime_entry_v1".as_ptr()) };
        if symbol.is_null() {
            return Err(LoadError::NotARuntime {
                path: library.to_owned(),
            });
        }
        // SAFETY: a runtime's one export is `extern "C" fn() -> *const GnoblinRuntimeEntry` (`gnoruntime_entry_v1`).
        let entry_fn = unsafe {
            core::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *const GnoblinRuntimeEntry>(symbol)
        };
        // SAFETY: calling the runtime's entry function; it returns a pointer to a static descriptor in its mapping.
        let entry = catch_unwind(AssertUnwindSafe(|| unsafe { entry_fn() })).unwrap_or(core::ptr::null());
        // SAFETY: the descriptor is a `static` in the runtime's mapping, which is never unmapped (a runtime stays open).
        let Some(entry) = (unsafe { entry.as_ref() }) else {
            return Err(LoadError::NotARuntime {
                path: library.to_owned(),
            });
        };
        let needed = core::mem::offset_of!(GnoblinRuntimeEntry, activate);
        if entry.abi_major != abi::ABI_MAJOR || (entry.size as usize) < needed {
            return Err(LoadError::RuntimeAbiMajor {
                path: library.to_owned(),
                built: entry.abi_major,
                built_minor: entry.abi_minor_built,
            });
        }
        Ok(LoadedRuntime {
            handle: handle as usize,
            entry,
            path: library.to_owned(),
            name: string_of(entry.name),
            extensions: string_of(entry.extensions)
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            version: string_of(entry.version),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(path: &str, head: &[u8], language: Option<&str>) -> Result<ModuleKind, UnknownKind> {
        detect(Path::new(path), head, language)
    }

    fn script(language: &str, extension: &str) -> ModuleKind {
        ModuleKind::Script {
            language: language.to_owned(),
            extension: extension.to_owned(),
        }
    }

    #[test]
    fn a_library_is_native_by_content_whatever_it_is_called() {
        assert_eq!(kind("/x/shell.js", b"\x7fELF", None), Ok(ModuleKind::Native));
        assert_eq!(kind("/x/libshell.so.1", b"\x7fELF", None), Ok(ModuleKind::Native));
        assert_eq!(kind("/x/noextension", b"\x7fELF", None), Ok(ModuleKind::Native));
    }

    #[test]
    fn an_unreadable_or_foreign_so_is_a_native_load_error_not_a_language_error() {
        assert_eq!(kind("/x/libshell.so", b"", None), Ok(ModuleKind::Native));
        assert_eq!(kind("/x/libshell.so", b"text", None), Ok(ModuleKind::Native));
    }

    #[test]
    fn the_known_extensions_pick_their_languages() {
        assert_eq!(kind("/x/a.js", b"//", None), Ok(script("js", "js")));
        assert_eq!(kind("/x/a.mjs", b"//", None), Ok(script("js", "mjs")));
        assert_eq!(kind("/x/a.ts", b"//", None), Ok(script("js", "ts")));
        assert_eq!(kind("/x/a.mts", b"//", None), Ok(script("js", "mts")));
        assert_eq!(kind("/x/a.py", b"#!", None), Ok(script("python", "py")));
    }

    #[test]
    fn any_other_extension_names_a_runtime_for_it() {
        assert_eq!(kind("/x/init.rb", b"--", None), Ok(script("rb", "rb")));
    }

    #[test]
    fn a_named_language_wins_over_content_and_extension() {
        assert_eq!(kind("/x/shell.txt", b"\x7fELF", Some("js")), Ok(script("js", "js")));
        assert_eq!(kind("/x/shell.js", b"//", Some("native")), Ok(ModuleKind::Native));
        assert_eq!(
            kind("/x/shell.txt", b"//", Some("python")),
            Ok(script("python", "python"))
        );
        assert_eq!(kind("/x/shell.js", b"//", Some("  ")), Ok(script("js", "js")));
    }

    #[test]
    fn no_extension_and_no_library_says_nothing_about_how_to_run_it() {
        assert_eq!(
            kind("/x/shell", b"abcd", None),
            Err(UnknownKind {
                path: PathBuf::from("/x/shell")
            })
        );
    }

    #[test]
    fn a_script_is_labelled_with_the_language_constant_of_its_extension() {
        assert_eq!(language_constant("js", "js"), abi::LANG_JS);
        assert_eq!(language_constant("js", "mjs"), abi::LANG_JS);
        assert_eq!(language_constant("js", "ts"), abi::LANG_TS);
        assert_eq!(language_constant("js", "mts"), abi::LANG_TS);
        assert_eq!(language_constant("python", "py"), abi::LANG_PYTHON);
        assert_eq!(language_constant("rb", "rb"), abi::LANG_OTHER);
    }

    #[test]
    fn the_known_extensions_are_listed_for_the_error_text() {
        assert_eq!(known_extensions(), ".so .py .js .mjs .ts .mts");
    }
}
