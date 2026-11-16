# Modular framework: proof spike

Companion to `design/modular-framework.md`. A throwaway Cargo workspace (`.tmp-spike/` in the Architect worktree, never committed) was
built and run on an Agent37 box (Debian 12, rustc 1.99.1, binutils 2.40, gcc 12.2, python 3.11.2) on 2026-11-16, with `nice -n 19` and
`-j2`, in `/home/node/spike` with its own target directories. The owner of the box's lane (`DwindleHypr`) lent it between gates with Main's
permission; everything under `/home/node/spike*` and `/tmp/gnoblin-spike-*` was removed afterwards.

The spike has no crates.io dependencies, so build times show only the orchestration cost of the mechanisms, not the cost of the real
compositor. `dlopen`, `dlsym` and `dlerror` are declared by hand against libc.

## Results

| #   | Claim                                                                                                                      | Result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Evidence (section below) |
| --- | -------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------ |
| 1   | A cdylib `libgnoblin_core.so.1` with SONAME builds and exports only `gnoblin_*`                                            | Yes. `SONAME libgnoblin_core.so.1`, `FLAGS_1 NODELETE`, `-z defs` accepted; `nm -D --defined-only` lists exactly `gnoblin_abi_version`, `gnoblin_get_api`, `gnoblin_launch`. **A second, hand-written version script is not needed and is harmful:** `rust-lld` warns `attempt to reassign symbol 'gnoblin_get_api' of VER_NDX_GLOBAL to version 'GNOBLIN_1'` because rustc already passes its own script (`local: *` plus the `#[no_mangle]` exports), and the symbols stay unversioned. Without our script (`SPIKE_NO_VERSION_SCRIPT=1`) the result is the same three symbols, zero non-`gnoblin_*` symbols and no warning.            | (1), (1b)                |
| 2   | The launcher links the core dynamically, NEEDED once, and cargo ordering is solved                                         | Yes. `NEEDED libgnoblin_core.so.1` once, `RUNPATH $ORIGIN`, `ldd` resolves it to the staged core. Mechanism: a generated stub library with the SONAME, so no build ordering exists (`cargo build -p gnoblin` alone succeeds in 0.5 s and then fails at run time with the loader's `libgnoblin_core.so.1: cannot open shared object file`). Alternative B, a normal dependency edge on the `cdylib`+`rlib` crate, makes cargo build the cdylib too (`target/debug/deps/libgnoblin_core_impl.so`, not uplifted, not named by SONAME) but compiles the implementation as an rlib for the dependent as well. Stub cost: one `cc` run, 18 ms. | (2), order               |
| 3   | A shell cdylib linking `-lgnoblin_core` has NEEDED core only, defines no `gnoblin_*`, and one core copy exists at run time | Yes. NEEDED is `libgnoblin_core.so.1` plus libgcc_s, libc, ld.so; `nm -D --defined-only` shows only `gnoshell_entry_v1`; undefined `gnoblin_abi_version`, `gnoblin_get_api`. `/proc/self/maps` shows `core copies = 1` with the shell loaded. Negative control: a shell that depends on the implementation crate as an rlib defines all three `gnoblin_*` symbols, which is what the `nm` check catches.                                                                                                                                                                                                                                 | (3), (3b)                |
| 4   | A panic inside a shell callback is contained and logged by the core                                                        | Yes, in two places. SDK trampoline `catch_unwind` -> `report_panic`: core logs `PANIC in shell generation 3: boom from the shell callback` and `callback of registration 2 panicked; contained`, the next event is delivered. Core export `selftest_panic` returns `-2` with `last_error` `panic in core export selftest_panic: ...`. Note: the shell's own libstd also prints its default panic message first; the SDK must install a hook that routes to `report_panic`.                                                                                                                                                               | (4)                      |
| 5   | Hot reload: unique copy, swap, retire without `dlclose`, callbacks removed by generation, no stale call                    | Yes. Same path twice returns one handle (`handles equal = true`); unique copies give distinct handles. v1 -> v2: old generation `regs=1 calls=1 drops=1` after retire, **0 stale calls** across 5 further events, new generation `calls=5`; maps show two shell copies and one core; a call into the core from the retired generation returns `reg=0` with `E_RETIRED`; a failed reload (`init` error) keeps generation 5 serving.                                                                                                                                                                                                       | (5)                      |
| 6   | ABI upgrade: a shell built against 1.0 runs unchanged on a rebuilt core 1.1                                                | Yes. Launcher and `shell10.so` have identical sha256 under both cores; table size 88 -> 96 bytes. A shell built against 1.1 on core 1.0 loads and `uptime_ms()` returns `None` instead of crashing.                                                                                                                                                                                                                                                                                                                                                                                                                                      | (6)                      |
| 7   | Tools on the box                                                                                                           | `readelf`, `nm`, `objdump`, `ldd`, `python3` 3.11.2 (with `python3-dev`: `/usr/include/python3.11`, `libpython3.11.so`, `python-3.11-embed.pc`), `pkg-config`, `cc`/`gcc` present. `cbindgen` and `patchelf` absent.                                                                                                                                                                                                                                                                                                                                                                                                                     | tools                    |
| -   | `[lib] name` with a hyphen                                                                                                 | Refused: `library target names cannot contain hyphens: gnoblin-core`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | hyphen                   |

Not covered by the spike (design only): the shell -> std -> core chain (example 9), TLS destructors after retire, `CARGO_CFG_PANIC=abort`
detection in the SDK build script, an in-process compositor and real threads of a shell.

Driver quirk worth knowing: `a37 exec ID -- bash -c 'a; b'` splits at the semicolon on the remote side, and `a37 push` replaces the target
directory, so each step that needs state runs from a script file.

## Output

```text

=== tools
readelf: /usr/bin/readelf
nm: /usr/bin/nm
objdump: /usr/bin/objdump
ldd: /usr/bin/ldd
python3: /usr/bin/python3
pkg-config: /usr/bin/pkg-config
cbindgen: MISSING
cc: /usr/bin/cc
gcc: /usr/bin/gcc
patchelf: MISSING
rustc 1.99.1 (58cf8f9c3 2026-10-13)
cargo 1.99.1 (5f94df478 2026-08-27)
cc (Debian 12.2.0-14+deb12u1) 12.2.0
GNU readelf (GNU Binutils for Debian) 2.40
Python 3.11.2
-I/usr/include/python3.11 -I/usr/include/python3.11
/usr/include/python3.11
libpython3.11.a
libpython3.11.so
libpython3.11.so.1
PyImath                        PyImath - Python bindings for the Imath libraries
python-3.11-embed              Python - Embed Python into an application
python-3.11                    Python - Build a C extension for Python
PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
NAME="Debian GNU/Linux"

=== build core 1.0 + launcher + shell-min (cold)
warning: linker stderr: rust-lld: attempt to reassign symbol 'gnoblin_abi_version' of VER_NDX_GLOBAL to version 'GNOBLIN_1'
         rust-lld: attempt to reassign symbol 'gnoblin_get_api' of VER_NDX_GLOBAL to version 'GNOBLIN_1'
         rust-lld: attempt to reassign symbol 'gnoblin_launch' of VER_NDX_GLOBAL to version 'GNOBLIN_1'
  |
  = note: `#[warn(linker_messages)]` on by default

warning: `gnoblin-core-impl` (lib) generated 1 warning
    Finished `dev` profile [unoptimized] target(s) in 0.69s

real	0m0.732s
user	0m0.960s
sys	0m0.310s

=== stage: real core named by its SONAME next to the launcher (cargo xtask stage in the real tree)
total 1773
-rwxr-xr-x 1 node node 474288 Nov 16 18:35 gnoblin
-rwxr-xr-x 1 node node 866200 Nov 16 18:35 libgnoblin_core.so.1
-rwxr-xr-x 1 node node 474184 Nov 16 18:35 libshell_min.so

=== (1) core: SONAME, flags, version, exported symbols
 0x0000000000000001 (NEEDED)             Shared library: [libgcc_s.so.1]
 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
 0x0000000000000001 (NEEDED)             Shared library: [ld-linux-x86-64.so.2]
 0x000000000000000e (SONAME)             Library soname: [libgnoblin_core.so.1]
 0x000000000000001e (FLAGS)              BIND_NOW
 0x000000006ffffffb (FLAGS_1)            Flags: NOW NODELETE
--- nm -D --defined-only:
000000000004a6a0 T gnoblin_abi_version
000000000004a760 T gnoblin_get_api
000000000004a8c0 T gnoblin_launch
--- anything not gnoblin_*? (expect nothing):
none
--- symbol versions (readelf --dyn-syms):
gnoblin_abi_version
gnoblin_get_api
gnoblin_launch
--- version definitions:
  000000: Rev: 1  Flags: BASE  Index: 1  Cnt: 1  Name: libgnoblin_core.so.1
  0x001c: Rev: 1  Flags: none  Index: 2  Cnt: 1  Name: GNOBLIN_1

=== (2) launcher: NEEDED, RUNPATH
 0x000000000000001d (RUNPATH)            Library runpath: [$ORIGIN]
 0x0000000000000001 (NEEDED)             Shared library: [libgnoblin_core.so.1]
 0x0000000000000001 (NEEDED)             Shared library: [libgcc_s.so.1]
 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
 0x0000000000000001 (NEEDED)             Shared library: [ld-linux-x86-64.so.2]
--- ldd gnoblin:
	libgnoblin_core.so.1 => /home/node/spike/stage/./libgnoblin_core.so.1 (0x00007eeb305c0000)
--- NEEDED gnoblin count: 1

=== (3) shell: NEEDED, defined symbols, undefined gnoblin_*
 0x0000000000000001 (NEEDED)             Shared library: [libgnoblin_core.so.1]
 0x0000000000000001 (NEEDED)             Shared library: [libgcc_s.so.1]
 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
 0x0000000000000001 (NEEDED)             Shared library: [ld-linux-x86-64.so.2]
--- nm -D --defined-only:
0000000000018740 T gnoshell_entry_v1
--- defined gnoblin_* (expect none): 0
--- undefined gnoblin_*:
                 U gnoblin_abi_version
                 U gnoblin_get_api
--- shell ldd:
	linux-vdso.so.1 (0x00007edca8ac8000)
	libgnoblin_core.so.1 => not found
	libgcc_s.so.1 => /lib/x86_64-linux-gnu/libgcc_s.so.1 (0x00007edca8a44000)
	libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007edca8862000)
	/lib64/ld-linux-x86-64.so.2 (0x000055c73e438000)

=== (3) run: launcher dlopens the shell, one core copy
loaded shell "shell-min" generation 3
[shell-min] client_created client-1
maps: core copies = 1 {"/home/node/spike/stage/libgnoblin_core.so.1"}; shell mappings = {"/tmp/gnoblin-spike-17407/gen2-libshell_min.so"}
stats: (2, 1, 0)
exit=0
[shell L1] shell-min init (core only)

=== (4) panic containment
[shell-min] client_created after-boom
selftest_panic rc=-2 last_error="panic in core export selftest_panic: selftest panic inside the core export"
still alive; shell calls after panic: 2
exit=0
[shell L1] shell-min init (core only)

thread '<unnamed>' (17416) panicked at shell-min/src/lib.rs:6:28:
boom from the shell callback
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
[gnoblin-core] PANIC in shell generation 3: boom from the shell callback
[gnoblin-core] callback of registration 2 panicked; contained, compositor continues

thread '<unnamed>' (17416) panicked at core/src/lib.rs:194:41:
selftest panic inside the core export
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
[gnoblin-core] panic in core export selftest_panic: selftest panic inside the core export

=== (5) hot reload: build shell-hot as v1, v2, bad
    Finished `dev` profile [unoptimized] target(s) in 0.14s
    Finished `dev` profile [unoptimized] target(s) in 0.14s
    Finished `dev` profile [unoptimized] target(s) in 0.21s

=== (5a) same path twice vs unique copies
same path twice: handles equal = true
unique copies: handles distinct = true (94385488774064 vs 94385488778256)
maps: core copies = 1 {"/home/node/spike/stage/libgnoblin_core.so.1"}; shell mappings = {"/home/node/spike/stage/shells/v1/libshell_hot.so", "/tmp/gnoblin-spike-17510/gen4-libshell_hot.so", "/tmp/gnoblin-spike-17510/gen5-libshell_hot.so"}

=== (5b) hot reload v1 -> v2, then failed reload of bad
gen 3 active: v1
[shell-hot v1] client_created A
old stats before swap: regs=1 calls=1 drops=0
gen 5 initialised: v2
old gen 3 (v1) after retire: regs=1 calls=1 drops=1
[shell-hot v2] client_created B0
[shell-hot v2] client_created B1
[shell-hot v2] client_created B2
[shell-hot v2] client_created B3
[shell-hot v2] client_created B4
STALE CALLS into retired gen: 0
call from retired gen 3: reg=0 last_error="E_RETIRED: generation 3 has been retired; on_event did nothing" stale_rejected=1
new gen 5 (v2) calls=5 regs=1
maps: core copies = 1 {"/home/node/spike/stage/libgnoblin_core.so.1"}; shell mappings = {"/tmp/gnoblin-spike-17512/gen2-libshell_hot.so", "/tmp/gnoblin-spike-17512/gen4-libshell_hot.so"}
retired list (never dlclosed): 1 entries
failed reload kept gen 5: ./shells/bad/libshell_hot.so: init failed (-5): E_RETIRED: generation 3 has been retired; on_event did nothing
[shell-hot v2] client_created C
after failed reload, gen 5 calls=6 (still serving)
exit=0
[shell L1] shell-hot v1 init
[shell L1] shell-hot v2 init
[gnoblin-core] retired generation 3 (v1): 1 registration(s) removed, library NOT dlclosed
[shell L1] shell init error: simulated init failure (bad shell)

=== (6) ABI upgrade: shell built against 1.0 runs on core 1.0 and a rebuilt core 1.1
warning: `gnoblin-core-impl` (lib) generated 1 warning
    Finished `dev` profile [unoptimized] target(s) in 0.48s
--- core 1.0 table/abi:
core ABI 1.0 table size 88; shell "shell-min" built against ABI minor 0
[shell-min] client_created c
--- SAME shell10.so (not rebuilt) on core 1.1:
core ABI 1.1 table size 96; shell "shell-min" built against ABI minor 0
[shell-min] client_created c
--- shell built against 1.1 on core 1.1:
core ABI 1.1 table size 96; shell "shell-min" built against ABI minor 1
[shell-min] client_created c
--- shell built against 1.1 on core 1.0 (degrades, no crash):
core ABI 1.0 table size 88; shell "shell-min" built against ABI minor 1
[shell-min] client_created c
--- sha256 of launcher and shell10.so identical across both cores:
98cdb94af7c8eeba  /home/node/spike/up10/gnoblin
98cdb94af7c8eeba  /home/node/spike/up11/gnoblin
c782b245d260c064  /home/node/spike/up10/shell10.so
c782b245d260c064  /home/node/spike/up11/shell10.so
--- core sizes/ SONAME:
 0x000000000000000e (SONAME)             Library soname: [libgnoblin_core.so.1]
[shell L1] shell-min init (core only)
[shell L1] shell-min init (core only)
[shell L1] shell-min init (core only)
[shell L1] uptime_ms via ABI 1.1 = Some(0)
[shell L1] shell-min init (core only)
[shell L1] uptime_ms via ABI 1.1 = None

=== build-order experiments
--- B: launcher-b has a normal dependency on the cdylib+rlib crate (cargo build -p gnoblin-b)
   Compiling gnoblin-b v1.0.0 (/home/node/spike/launcher-b)
    Finished `dev` profile [unoptimized] target(s) in 0.79s
/home/node/spike-target-b/debug/deps/libgnoblin_core_impl.rlib
/home/node/spike-target-b/debug/deps/libgnoblin_core_impl.so
--- C: cargo build -p gnoblin only, cold target; time, then try to run
    Finished `dev` profile [unoptimized] target(s) in 0.49s
real	0m0.528s
/home/node/spike-target-c/debug/gnoblin: error while loading shared libraries: libgnoblin_core.so.1: cannot open shared object file: No such file or directory
--- cold full build of core+launcher+shell (time)
    Finished `dev` profile [unoptimized] target(s) in 1.38s
real	0m1.411s
--- stub generation cost: cc of the stub (3 symbols)
real	0m0.018s

=== (1b) core WITHOUT our own version script (rustc's export control only)
   Compiling gnoblin-core-abi v1.0.0 (/home/node/spike/abi)
   Compiling gnoblin-core-impl v1.0.0 (/home/node/spike/core)
    Finished `dev` profile [unoptimized] target(s) in 0.61s
 0x000000000000000e (SONAME)             Library soname: [libgnoblin_core.so.1]
 0x000000006ffffffb (FLAGS_1)            Flags: NOW NODELETE
--- nm -D --defined-only:
000000000004a660 T gnoblin_abi_version
000000000004a720 T gnoblin_get_api
000000000004a880 T gnoblin_launch
--- non gnoblin_* defined:
0

=== can a [lib] name contain a hyphen?
error: failed to parse manifest at `/home/node/spike-hy/Cargo.toml`

Caused by:
  library target names cannot contain hyphens: gnoblin-core
ls: cannot access '/home/node/spike-hy/t/debug': No such file or directory

=== (3b) negative control: a shell that depends on the implementation crate as an rlib
    Finished `dev` profile [unoptimized] target(s) in 0.14s
 0x0000000000000001 (NEEDED)             Shared library: [libgcc_s.so.1]
 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
 0x0000000000000001 (NEEDED)             Shared library: [ld-linux-x86-64.so.2]
--- defined gnoblin_* (a second copy of the core's statics): 3
0000000000032380 T gnoblin_abi_version
0000000000032440 T gnoblin_get_api
00000000000325a0 T gnoblin_launch
```

## Sources

The sources are reproduced exactly as run (`.tmp-spike/`).

### `Cargo.toml`

```toml
[workspace]
resolver = "3"
members = ["abi", "core", "sdk", "launcher", "launcher-b", "shell-min", "shell-hot", "shell-bad"]

[profile.dev]
panic = "unwind"
debug = 0
[profile.release]
panic = "unwind"
```

### `abi/Cargo.toml`

```toml
[package]
name = "gnoblin-core-abi"
version = "1.0.0"
edition = "2024"

[features]
# ABI 1.1 adds one function to the table (`uptime_ms`).
v1_1 = []
```

### `abi/exports.txt`

```text
gnoblin_abi_version
gnoblin_get_api
gnoblin_launch
```

### `abi/src/lib.rs`

```rust
//! Spike ABI crate: repr(C) types only, no logic. Real crate: generated from gnoblin-schema.
use std::ffi::{c_char, c_void};

pub const ABI_MAJOR: u32 = 1;
pub const ABI_MINOR: u32 = if cfg!(feature = "v1_1") { 1 } else { 0 };

pub const E_OK: i32 = 0;
pub const E_PANIC: i32 = -2;
pub const E_UNSUPPORTED: i32 = -3;
pub const E_ABI: i32 = -4;
pub const E_LOAD: i32 = -5;

pub type EventFn = unsafe extern "C" fn(user: *mut c_void, payload: *const c_char, len: usize) -> i32;
pub type DropFn = unsafe extern "C" fn(user: *mut c_void);

/// Versioned function table. `size` is the byte size of the table the core filled in; a field is present iff
/// `offset_of(field) + size_of(field) <= size`. New fields are only ever appended.
#[repr(C)]
pub struct GnoblinApi {
    pub size: u32,
    pub major: u32,
    pub minor: u32,
    pub reserved: u32,
    pub log: unsafe extern "C" fn(level: u32, msg: *const c_char, len: usize),
    pub last_error: unsafe extern "C" fn(buf: *mut c_char, cap: usize) -> usize,
    pub on_event: unsafe extern "C" fn(owner: u64, name: *const c_char, cb: EventFn, user: *mut c_void, drop_user: Option<DropFn>) -> u64,
    pub remove: unsafe extern "C" fn(reg: u64) -> i32,
    pub remove_owner: unsafe extern "C" fn(owner: u64) -> u32,
    pub post_event: unsafe extern "C" fn(name: *const c_char, payload: *const c_char, len: usize) -> i32,
    pub dispatch: unsafe extern "C" fn(timeout_ms: u32) -> i32,
    pub report_panic: unsafe extern "C" fn(owner: u64, msg: *const c_char, len: usize),
    /// Test hook: the core panics inside this export; the export wrapper must contain it.
    pub selftest_panic: unsafe extern "C" fn() -> i32,
    #[cfg(feature = "v1_1")]
    pub uptime_ms: unsafe extern "C" fn() -> u64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct ShellStats {
    pub registered: u64,
    pub calls: u64,
    pub drops: u64,
}

/// What a shell module exports as its single symbol `gnoshell_entry_v1`.
#[repr(C)]
pub struct ShellEntry {
    pub abi_major: u32,
    pub abi_minor_built: u32,
    pub init: unsafe extern "C" fn(api: *const GnoblinApi, owner: u64) -> i32,
    pub shutdown: unsafe extern "C" fn(owner: u64),
    pub stats: Option<unsafe extern "C" fn(out: *mut ShellStats)>,
    pub label: *const c_char,
}
unsafe impl Sync for ShellEntry {}

pub const ENTRY_SYMBOL: &str = "gnoshell_entry_v1";
```

### `core/Cargo.toml`

```toml
[package]
name = "gnoblin-core-impl"
version = "1.0.0"
edition = "2024"
build = "build.rs"

[lib]
name = "gnoblin_core_impl"
crate-type = ["cdylib", "rlib"]

[features]
v1_1 = ["gnoblin-core-abi/v1_1"]

[dependencies]
gnoblin-core-abi = { path = "../abi" }
```

### `core/build.rs`

```rust
// SONAME + version script + nodelete for the core. The export list is the single file abi/exports.txt.
use std::{env, fs, path::PathBuf};
fn main() {
    let exports = fs::read_to_string("../abi/exports.txt").unwrap();
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut map = String::from("GNOBLIN_1 {\n  global:\n");
    for s in exports.lines().filter(|l| !l.trim().is_empty()) {
        map.push_str(&format!("    {s};\n"));
    }
    map.push_str("  local:\n    *;\n};\n");
    let path = out.join("gnoblin_core.map");
    fs::write(&path, map).unwrap();
    println!("cargo:rerun-if-changed=../abi/exports.txt");
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libgnoblin_core.so.1");
    println!("cargo:rerun-if-env-changed=SPIKE_NO_VERSION_SCRIPT");
    if std::env::var_os("SPIKE_NO_VERSION_SCRIPT").is_none() {
        println!("cargo:rustc-cdylib-link-arg=-Wl,--version-script={}", path.display());
    }
    println!("cargo:rustc-cdylib-link-arg=-Wl,-z,nodelete");
    println!("cargo:rustc-cdylib-link-arg=-Wl,-z,defs");
}
```

### `core/src/lib.rs`

```rust
//! Spike core: registry, event queue, panic containment, loader with unique copies and a retire list.
#![allow(unsafe_op_in_unsafe_fn, clippy::missing_safety_doc)]
use gnoblin_core_abi::*;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

// ---- logging and last error ------------------------------------------------------------------------------------
fn clog(msg: &str) {
    eprintln!("[gnoblin-core] {msg}");
}
thread_local!(static LAST: RefCell<String> = const { RefCell::new(String::new()) });
fn set_err(s: impl Into<String>) {
    LAST.with(|l| *l.borrow_mut() = s.into());
}
fn payload_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "non-string panic".into())
}
/// Every export runs through this: a panic never crosses the ABI.
fn guard<T>(on_panic: T, what: &str, f: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(v) => v,
        Err(p) => {
            let m = format!("panic in core export {what}: {}", payload_text(&p));
            clog(&m);
            set_err(m);
            on_panic
        }
    }
}

// ---- registry --------------------------------------------------------------------------------------------------
struct Reg {
    id: u64,
    owner: u64,
    name: String,
    cb: EventFn,
    user: usize,
    drop_user: Option<DropFn>,
    dead: bool,
}
struct State {
    regs: Vec<Reg>,
    next: u64,
    queue: VecDeque<(String, String)>,
    in_flight: Option<u64>,
}
static STATE: Mutex<State> = Mutex::new(State { regs: Vec::new(), next: 1, queue: VecDeque::new(), in_flight: None });
static CV: Condvar = Condvar::new();
static RETIRED_OWNERS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
static STALE_REJECTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn st() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}
unsafe fn run_drop(r: &Reg) {
    if let Some(d) = r.drop_user {
        d(r.user as *mut c_void);
    }
}

unsafe extern "C" fn api_log(level: u32, msg: *const c_char, len: usize) {
    guard((), "log", || {
        let s = String::from_utf8_lossy(std::slice::from_raw_parts(msg as *const u8, len)).into_owned();
        eprintln!("[shell L{level}] {s}");
    })
}
unsafe extern "C" fn api_last_error(buf: *mut c_char, cap: usize) -> usize {
    LAST.with(|l| {
        let l = l.borrow();
        let n = l.len().min(cap.saturating_sub(1));
        if cap > 0 {
            std::ptr::copy_nonoverlapping(l.as_ptr(), buf as *mut u8, n);
            *buf.add(n) = 0;
        }
        l.len()
    })
}
unsafe extern "C" fn api_on_event(owner: u64, name: *const c_char, cb: EventFn, user: *mut c_void, drop_user: Option<DropFn>) -> u64 {
    guard(0, "on_event", || {
        if RETIRED_OWNERS.lock().unwrap().contains(&owner) {
            STALE_REJECTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            set_err(format!("E_RETIRED: generation {owner} has been retired; on_event did nothing"));
            return 0;
        }
        let name = CStr::from_ptr(name).to_string_lossy().into_owned();
        let mut s = st();
        let id = s.next;
        s.next += 1;
        s.regs.push(Reg { id, owner, name, cb, user: user as usize, drop_user, dead: false });
        id
    })
}
fn remove_where(pred: impl Fn(&Reg) -> bool) -> u32 {
    let mut gone = Vec::new();
    let mut s = st();
    let in_flight = s.in_flight;
    let mut i = 0;
    while i < s.regs.len() {
        if pred(&s.regs[i]) {
            if Some(s.regs[i].id) == in_flight {
                s.regs[i].dead = true; // its callback is running: drop after it returns
                i += 1;
            } else {
                gone.push(s.regs.remove(i));
            }
        } else {
            i += 1;
        }
    }
    drop(s);
    let n = gone.len() as u32;
    for r in &gone {
        unsafe { run_drop(r) };
    }
    n
}
unsafe extern "C" fn api_remove(reg: u64) -> i32 {
    guard(-1, "remove", || if remove_where(|r| r.id == reg) > 0 { 0 } else { set_err("no such registration"); -1 })
}
unsafe extern "C" fn api_remove_owner(owner: u64) -> u32 {
    guard(0, "remove_owner", || remove_where(|r| r.owner == owner))
}
unsafe extern "C" fn api_post_event(name: *const c_char, payload: *const c_char, len: usize) -> i32 {
    guard(-1, "post_event", || {
        let name = CStr::from_ptr(name).to_string_lossy().into_owned();
        let p = String::from_utf8_lossy(std::slice::from_raw_parts(payload as *const u8, len)).into_owned();
        st().queue.push_back((name, p));
        CV.notify_all();
        0
    })
}
unsafe extern "C" fn api_dispatch(timeout_ms: u32) -> i32 {
    guard(-1, "dispatch", || {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
        let mut s = st();
        while s.queue.is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return 0;
            }
            s = CV.wait_timeout(s, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        let events: Vec<_> = s.queue.drain(..).collect();
        drop(s);
        let mut called = 0;
        for (name, payload) in events {
            let ids: Vec<u64> = st().regs.iter().filter(|r| r.name == name && !r.dead).map(|r| r.id).collect();
            for id in ids {
                let (cb, user) = {
                    let mut s = st();
                    match s.regs.iter().find(|r| r.id == id && !r.dead) {
                        Some(r) => {
                            let t = (r.cb, r.user);
                            s.in_flight = Some(id);
                            t
                        }
                        None => continue, // removed by an earlier callback of this event: never call it
                    }
                };
                let rc = match catch_unwind(AssertUnwindSafe(|| unsafe { cb(user as *mut c_void, payload.as_ptr() as *const c_char, payload.len()) })) {
                    Ok(rc) => rc,
                    Err(p) => {
                        clog(&format!("callback unwound across the ABI (reg {id}): {}", payload_text(&p)));
                        E_PANIC
                    }
                };
                called += 1;
                if rc == E_PANIC {
                    clog(&format!("callback of registration {id} panicked; contained, compositor continues"));
                }
                let dead = {
                    let mut s = st();
                    s.in_flight = None;
                    s.regs.iter().position(|r| r.id == id && r.dead).map(|i| s.regs.remove(i))
                };
                if let Some(r) = dead {
                    unsafe { run_drop(&r) };
                }
            }
        }
        called
    })
}
unsafe extern "C" fn api_report_panic(owner: u64, msg: *const c_char, len: usize) {
    guard((), "report_panic", || {
        let s = String::from_utf8_lossy(std::slice::from_raw_parts(msg as *const u8, len)).into_owned();
        clog(&format!("PANIC in shell generation {owner}: {s}"));
    })
}
unsafe extern "C" fn api_selftest_panic() -> i32 {
    guard(E_PANIC, "selftest_panic", || panic!("selftest panic inside the core export"))
}
#[cfg(feature = "v1_1")]
unsafe extern "C" fn api_uptime_ms() -> u64 {
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_millis() as u64
}

static API: GnoblinApi = GnoblinApi {
    size: std::mem::size_of::<GnoblinApi>() as u32,
    major: ABI_MAJOR,
    minor: ABI_MINOR,
    reserved: 0,
    log: api_log,
    last_error: api_last_error,
    on_event: api_on_event,
    remove: api_remove,
    remove_owner: api_remove_owner,
    post_event: api_post_event,
    dispatch: api_dispatch,
    report_panic: api_report_panic,
    selftest_panic: api_selftest_panic,
    #[cfg(feature = "v1_1")]
    uptime_ms: api_uptime_ms,
};

// ---- the only exported symbols (exports.txt) ---------------------------------------------------------------------
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnoblin_abi_version(major: *mut u32, minor: *mut u32) {
    *major = ABI_MAJOR;
    *minor = ABI_MINOR;
}
/// Hands out the table. Refuses a different major; a shell built against a newer minor still gets the table and
/// sees through `size` which functions are missing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnoblin_get_api(major: u32, _built_minor: u32, out: *mut *const GnoblinApi) -> i32 {
    if major != ABI_MAJOR {
        set_err(format!("gnoblin ABI major mismatch: shell wants {major}, installed core is {ABI_MAJOR}"));
        return E_ABI;
    }
    *out = &API;
    E_OK
}

// ---- loader ------------------------------------------------------------------------------------------------------
unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}
const RTLD_NOW: c_int = 2;
const RTLD_LOCAL: c_int = 0;
fn dl_err() -> String {
    unsafe { let e = dlerror(); if e.is_null() { "unknown dl error".into() } else { CStr::from_ptr(e).to_string_lossy().into_owned() } }
}
struct Gen {
    owner: u64,
    handle: *mut c_void,
    entry: &'static ShellEntry,
    label: String,
}
static RETIRED: Mutex<Vec<(u64, usize)>> = Mutex::new(Vec::new()); // (owner, handle): never dlclosed
static NEXT_GEN: Mutex<u64> = Mutex::new(1);

fn open_copy(path: &str, same_path: bool) -> Result<*mut c_void, String> {
    let gen_n = { let mut n = NEXT_GEN.lock().unwrap(); *n += 1; *n };
    let use_path = if same_path {
        path.to_string()
    } else {
        let dir = std::env::temp_dir().join(format!("gnoblin-spike-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let name = std::path::Path::new(path).file_name().unwrap().to_string_lossy().into_owned();
        let dst = dir.join(format!("gen{gen_n}-{name}"));
        std::fs::copy(path, &dst).map_err(|e| format!("copy {path}: {e}"))?;
        dst.to_string_lossy().into_owned()
    };
    let c = CString::new(use_path.clone()).unwrap();
    let h = unsafe { dlopen(c.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
    if !same_path {
        let _ = std::fs::remove_file(&use_path); // the mapping keeps the inode alive
    }
    if h.is_null() { Err(format!("dlopen {path}: {}", dl_err())) } else { Ok(h) }
}
fn load(path: &str, unique: bool) -> Result<Gen, String> {
    let handle = open_copy(path, !unique)?;
    let sym = CString::new(ENTRY_SYMBOL).unwrap();
    let p = unsafe { dlsym(handle, sym.as_ptr()) };
    if p.is_null() {
        return Err(format!("{path}: no `{ENTRY_SYMBOL}` export (not a gnoblin shell): {}", dl_err()));
    }
    let f: unsafe extern "C" fn() -> *const ShellEntry = unsafe { std::mem::transmute(p) };
    let entry: &'static ShellEntry = unsafe { &*f() };
    if entry.abi_major != ABI_MAJOR {
        return Err(format!("{path}: built for gnoblin ABI {}.x, installed core is {ABI_MAJOR}.{ABI_MINOR}", entry.abi_major));
    }
    let owner = { let mut n = NEXT_GEN.lock().unwrap(); *n += 1; *n };
    let label = unsafe { CStr::from_ptr(entry.label).to_string_lossy().into_owned() };
    let rc = match catch_unwind(AssertUnwindSafe(|| unsafe { (entry.init)(&API, owner) })) {
        Ok(rc) => rc,
        Err(p) => {
            return Err(format!("{path}: init unwound across the ABI: {}", payload_text(&p)));
        }
    };
    if rc != E_OK {
        // partial init: whatever it registered goes away with its owner
        api_remove_owner_rust(owner);
        let mut buf = [0u8; 256];
        let n = unsafe { api_last_error(buf.as_mut_ptr() as *mut c_char, buf.len()) }.min(255);
        return Err(format!("{path}: init failed ({rc}): {}", String::from_utf8_lossy(&buf[..n])));
    }
    Ok(Gen { owner, handle, entry, label })
}
fn api_remove_owner_rust(owner: u64) -> u32 {
    remove_where(|r| r.owner == owner)
}
fn retire(g: Gen) {
    unsafe { (g.entry.shutdown)(g.owner) };
    let n = api_remove_owner_rust(g.owner);
    clog(&format!("retired generation {} ({}): {n} registration(s) removed, library NOT dlclosed", g.owner, g.label));
    RETIRED_OWNERS.lock().unwrap().push(g.owner);
    RETIRED.lock().unwrap().push((g.owner, g.handle as usize));
}
fn stats(g: &Gen) -> ShellStats {
    let mut s = ShellStats::default();
    if let Some(f) = g.entry.stats {
        unsafe { f(&mut s) };
    }
    s
}
fn maps() -> String {
    let m = std::fs::read_to_string("/proc/self/maps").unwrap_or_default();
    let mut core = std::collections::BTreeSet::new();
    let mut shells = std::collections::BTreeSet::new();
    for l in m.lines() {
        if let Some(p) = l.split_whitespace().nth(5) {
            if p.contains("libgnoblin_core") || p.contains("libgnoblin_core_impl") { core.insert(p.to_string()); }
            if p.contains("libshell") { shells.insert(p.trim_end_matches(" (deleted)").to_string()); }
        }
    }
    format!("maps: core copies = {} {:?}; shell mappings = {:?}", core.len(), core, shells)
}
fn pump(n: u32) {
    for _ in 0..n {
        unsafe { api_dispatch(50) };
    }
}
fn post(name: &str, payload: &str) {
    let n = CString::new(name).unwrap();
    unsafe { api_post_event(n.as_ptr(), payload.as_ptr() as *const c_char, payload.len()) };
}

// ---- launcher entry: scenarios run by the spike tests --------------------------------------------------------------
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnoblin_launch(argc: c_int, argv: *const *const c_char) -> c_int {
    guard(101, "launch", || {
        let args: Vec<String> = (0..argc as usize).map(|i| CStr::from_ptr(*argv.add(i)).to_string_lossy().into_owned()).collect();
        // The "compositor thread": posts events from another thread; the shell never runs on it.
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_millis(5));
            post("tick", "");
        });
        let scenario = args.get(1).map(String::as_str).unwrap_or("");
        match scenario {
            "run" => scen_run(&args[2]),
            "panic" => scen_panic(&args[2]),
            "hot" => scen_hot(&args[2], &args[3], args.get(4).map(String::as_str)),
            "dup" => scen_dup(&args[2]),
            "upgrade" => scen_upgrade(&args[2]),
            _ => { eprintln!("usage: gnoblin <run|panic|hot|dup|upgrade> ..."); 2 }
        }
    })
}
fn scen_run(p: &str) -> c_int {
    let g = match load(p, true) { Ok(g) => g, Err(e) => { println!("LOAD FAILED: {e}"); return 1 } };
    println!("loaded shell {:?} generation {}", g.label, g.owner);
    post("client_created", "client-1");
    pump(3);
    println!("{}", maps());
    println!("stats: {:?}", { let s = stats(&g); (s.registered, s.calls, s.drops) });
    0
}
fn scen_panic(p: &str) -> c_int {
    let g = match load(p, true) { Ok(g) => g, Err(e) => { println!("LOAD FAILED: {e}"); return 1 } };
    post("boom", "x");
    post("client_created", "after-boom");
    pump(3);
    let rc = unsafe { api_selftest_panic() };
    let mut buf = [0u8; 200];
    let n = unsafe { api_last_error(buf.as_mut_ptr() as *mut c_char, buf.len()) }.min(199);
    println!("selftest_panic rc={rc} last_error={:?}", String::from_utf8_lossy(&buf[..n]));
    println!("still alive; shell calls after panic: {}", stats(&g).calls);
    0
}
fn scen_dup(p: &str) -> c_int {
    let a = open_copy(p, true).unwrap();
    let b = open_copy(p, true).unwrap();
    println!("same path twice: handles equal = {}", a == b);
    let c = open_copy(p, false).unwrap();
    let d = open_copy(p, false).unwrap();
    println!("unique copies: handles distinct = {} ({} vs {})", c != d && c != a, c as usize, d as usize);
    println!("{}", maps());
    0
}
fn scen_upgrade(p: &str) -> c_int {
    let g = match load(p, true) { Ok(g) => g, Err(e) => { println!("LOAD FAILED: {e}"); return 1 } };
    println!("core ABI {}.{} table size {}; shell {:?} built against ABI minor {}", ABI_MAJOR, ABI_MINOR, API.size, g.label, g.entry.abi_minor_built);
    post("client_created", "c");
    pump(2);
    0
}
fn scen_hot(v1: &str, v2: &str, bad: Option<&str>) -> c_int {
    let mut cur = match load(v1, true) { Ok(g) => g, Err(e) => { println!("LOAD FAILED: {e}"); return 1 } };
    println!("gen {} active: {}", cur.owner, cur.label);
    post("client_created", "A");
    pump(2);
    let before = stats(&cur);
    println!("old stats before swap: regs={} calls={} drops={}", before.registered, before.calls, before.drops);
    // reload: unique copy, init the new one first, then retire the old
    let new = match load(v2, true) { Ok(g) => g, Err(e) => { println!("reload failed, keeping old: {e}"); return 1 } };
    println!("gen {} initialised: {}", new.owner, new.label);
    let old = std::mem::replace(&mut cur, new);
    let old_stats_ptr = old.entry;
    let (old_owner, old_label) = (old.owner, old.label.clone());
    retire(old);
    let mut s0 = ShellStats::default();
    if let Some(f) = old_stats_ptr.stats { unsafe { f(&mut s0) } }
    println!("old gen {old_owner} ({old_label}) after retire: regs={} calls={} drops={}", s0.registered, s0.calls, s0.drops);
    for i in 0..5 { post("client_created", &format!("B{i}")); }
    pump(5);
    let mut s1 = ShellStats::default();
    if let Some(f) = old_stats_ptr.stats { unsafe { f(&mut s1) } }
    println!("STALE CALLS into retired gen: {}", s1.calls - s0.calls);
    unsafe extern "C" fn dummy(_u: *mut c_void, _p: *const c_char, _l: usize) -> i32 { 0 }
    let nm = CString::new("client_created").unwrap();
    let reg = unsafe { api_on_event(old_owner, nm.as_ptr(), dummy, std::ptr::null_mut(), None) };
    let mut eb = [0u8; 160];
    let en = unsafe { api_last_error(eb.as_mut_ptr() as *mut c_char, eb.len()) }.min(159);
    println!("call from retired gen {old_owner}: reg={reg} last_error={:?} stale_rejected={}", String::from_utf8_lossy(&eb[..en]), STALE_REJECTED.load(std::sync::atomic::Ordering::SeqCst));
    let sn = stats(&cur);
    println!("new gen {} ({}) calls={} regs={}", cur.owner, cur.label, sn.calls, sn.registered);
    println!("{}", maps());
    println!("retired list (never dlclosed): {} entries", RETIRED.lock().unwrap().len());
    if let Some(b) = bad {
        match load(b, true) {
            Ok(_) => println!("UNEXPECTED: bad shell loaded"),
            Err(e) => println!("failed reload kept gen {}: {e}", cur.owner),
        }
        post("client_created", "C");
        pump(2);
        let s2 = stats(&cur);
        println!("after failed reload, gen {} calls={} (still serving)", cur.owner, s2.calls);
    }
    0
}
```

### `sdk/Cargo.toml`

```toml
[package]
name = "gnoblin-core"
version = "1.0.0"
edition = "2024"
build = "build.rs"

[features]
v1_1 = ["gnoblin-core-abi/v1_1"]

[dependencies]
gnoblin-core-abi = { path = "../abi" }
```

### `sdk/build.rs`

```rust
// Link-time stub: a tiny shared object named libgnoblin_core.so with SONAME libgnoblin_core.so.1 that defines the
// exported symbols of the core. The SDK links it, so the shell records NEEDED libgnoblin_core.so.1 and nothing else
// of ours; at run time the loader resolves that NEEDED to the real installed core. If a real dev library is
// present (GNOBLIN_CORE_LIB_DIR, standing in for pkg-config here) it is used instead.
use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-env-changed=GNOBLIN_CORE_LIB_DIR");
    println!("cargo:rerun-if-changed=../abi/exports.txt");
    if let Ok(dir) = env::var("GNOBLIN_CORE_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=dylib=gnoblin_core");
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let exports = fs::read_to_string("../abi/exports.txt").unwrap();
    let mut c = String::new();
    for s in exports.lines().filter(|l| !l.trim().is_empty()) {
        c.push_str(&format!("void {s}(void) {{}}\n"));
    }
    fs::write(out.join("stub.c"), c).unwrap();
    let st = Command::new("cc")
        .args(["-shared", "-fPIC", "-Wl,-soname,libgnoblin_core.so.1", "-o"])
        .arg(out.join("libgnoblin_core.so"))
        .arg(out.join("stub.c"))
        .status()
        .expect("cc");
    assert!(st.success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=dylib=gnoblin_core");
}
```

### `sdk/src/lib.rs`

```rust
//! Spike SDK: extern-C declarations plus a safe wrapper. Never contains core code.
use gnoblin_core_abi::*;
use std::ffi::{CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, AtomicPtr, Ordering};

pub use gnoblin_core_abi as abi;

unsafe extern "C" {
    fn gnoblin_abi_version(major: *mut u32, minor: *mut u32);
    fn gnoblin_get_api(major: u32, built_minor: u32, out: *mut *const GnoblinApi) -> i32;
}

pub static REGS: AtomicU64 = AtomicU64::new(0);
pub static CALLS: AtomicU64 = AtomicU64::new(0);
pub static DROPS: AtomicU64 = AtomicU64::new(0);
static API: AtomicPtr<GnoblinApi> = AtomicPtr::new(std::ptr::null_mut());

pub struct Gnoblin {
    api: &'static GnoblinApi,
    owner: u64,
}
pub struct Registration(pub u64);

struct Slot<F> {
    f: F,
    owner: u64,
}

unsafe extern "C" fn tramp<F: FnMut(&str) + 'static>(user: *mut c_void, payload: *const c_char, len: usize) -> i32 {
    let slot = unsafe { &mut *(user as *mut Slot<F>) };
    CALLS.fetch_add(1, Ordering::SeqCst);
    let text = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(payload as *const u8, len)) };
    match catch_unwind(AssertUnwindSafe(|| (slot.f)(text))) {
        Ok(()) => E_OK,
        Err(p) => {
            let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
            let api = unsafe { &*API.load(Ordering::SeqCst) };
            unsafe { (api.report_panic)(slot.owner, msg.as_ptr() as *const c_char, msg.len()) };
            E_PANIC
        }
    }
}
unsafe extern "C" fn drop_slot<F>(user: *mut c_void) {
    DROPS.fetch_add(1, Ordering::SeqCst);
    drop(unsafe { Box::from_raw(user as *mut Slot<F>) });
}

impl Gnoblin {
    pub fn log(&self, s: &str) {
        unsafe { (self.api.log)(1, s.as_ptr() as *const c_char, s.len()) }
    }
    pub fn on_event<F: FnMut(&str) + 'static>(&self, name: &str, f: F) -> Registration {
        let slot = Box::into_raw(Box::new(Slot { f, owner: self.owner }));
        let n = CString::new(name).unwrap();
        REGS.fetch_add(1, Ordering::SeqCst);
        Registration(unsafe { (self.api.on_event)(self.owner, n.as_ptr(), tramp::<F>, slot as *mut c_void, Some(drop_slot::<F>)) })
    }
    pub fn core_abi_minor(&self) -> u32 {
        self.api.minor
    }
    /// Present only when the installed core is at least ABI 1.1 *and* this SDK was built knowing the field.
    #[cfg(feature = "v1_1")]
    pub fn uptime_ms(&self) -> Option<u64> {
        let end = std::mem::offset_of!(GnoblinApi, uptime_ms) + std::mem::size_of::<unsafe extern "C" fn() -> u64>();
        if self.api.size as usize >= end { Some(unsafe { (self.api.uptime_ms)() }) } else { None }
    }
}

#[doc(hidden)]
pub unsafe fn __init(user: fn(&Gnoblin) -> Result<(), String>, owner: u64) -> i32 {
    let mut api: *const GnoblinApi = std::ptr::null();
    let rc = unsafe { gnoblin_get_api(ABI_MAJOR, ABI_MINOR, &mut api) };
    if rc != E_OK {
        return rc;
    }
    let (mut ma, mut mi) = (0, 0);
    unsafe { gnoblin_abi_version(&mut ma, &mut mi) };
    API.store(api as *mut _, Ordering::SeqCst);
    let g = Gnoblin { api: unsafe { &*api }, owner };
    match catch_unwind(AssertUnwindSafe(|| user(&g))) {
        Ok(Ok(())) => E_OK,
        Ok(Err(e)) => {
            g.log(&format!("shell init error: {e}"));
            E_LOAD
        }
        Err(_) => E_PANIC,
    }
}
#[doc(hidden)]
pub unsafe extern "C" fn __stats(out: *mut ShellStats) {
    unsafe {
        *out = ShellStats { registered: REGS.load(Ordering::SeqCst), calls: CALLS.load(Ordering::SeqCst), drops: DROPS.load(Ordering::SeqCst) };
    }
}

/// `export_shell!(label, init_fn)` is the stand-in for `#[gnoblin::main]`.
#[macro_export]
macro_rules! export_shell {
    ($label:expr, $init:path) => {
        unsafe extern "C" fn __gn_init(_api: *const $crate::abi::GnoblinApi, owner: u64) -> i32 {
            unsafe { $crate::__init($init, owner) }
        }
        unsafe extern "C" fn __gn_shutdown(_owner: u64) {}
        static __GN_ENTRY: $crate::abi::ShellEntry = $crate::abi::ShellEntry {
            abi_major: $crate::abi::ABI_MAJOR,
            abi_minor_built: $crate::abi::ABI_MINOR,
            init: __gn_init,
            shutdown: __gn_shutdown,
            stats: Some($crate::__stats),
            label: concat!($label, "\0").as_ptr() as *const ::std::ffi::c_char,
        };
        #[unsafe(no_mangle)]
        pub extern "C" fn gnoshell_entry_v1() -> *const $crate::abi::ShellEntry {
            &__GN_ENTRY
        }
    };
}
```

### `launcher/Cargo.toml`

```toml
[package]
name = "gnoblin"
version = "1.0.0"
edition = "2024"
build = "build.rs"

[[bin]]
name = "gnoblin"
path = "src/main.rs"
```

### `launcher/build.rs`

```rust
// Launcher link strategy "stub": link a generated libgnoblin_core.so (SONAME .so.1) so the cargo build needs no
// ordering against the core crate; `$ORIGIN` rpath finds the real one beside the binary at run time.
use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=../abi/exports.txt");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let exports = fs::read_to_string("../abi/exports.txt").unwrap();
    let c: String = exports.lines().filter(|l| !l.trim().is_empty()).map(|s| format!("void {s}(void) {{}}\n")).collect();
    fs::write(out.join("stub.c"), c).unwrap();
    let ok = Command::new("cc")
        .args(["-shared", "-fPIC", "-Wl,-soname,libgnoblin_core.so.1", "-o"])
        .arg(out.join("libgnoblin_core.so"))
        .arg(out.join("stub.c"))
        .status()
        .unwrap()
        .success();
    assert!(ok);
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=dylib=gnoblin_core");
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
}
```

### `launcher/src/main.rs`

```rust
use std::ffi::{CString, c_char, c_int};
unsafe extern "C" {
    fn gnoblin_launch(argc: c_int, argv: *const *const c_char) -> c_int;
}
fn main() {
    let args: Vec<CString> = std::env::args().map(|a| CString::new(a).unwrap()).collect();
    let argv: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
    std::process::exit(unsafe { gnoblin_launch(argv.len() as c_int, argv.as_ptr()) });
}
```

### `launcher-b/Cargo.toml`

```toml
[package]
name = "gnoblin-b"
version = "1.0.0"
edition = "2024"

[[bin]]
name = "gnoblin-b"
path = "src/main.rs"

[dependencies]
# Ordering experiment: does a normal dependency edge on a cdylib+rlib crate make cargo build the cdylib?
gnoblin-core-impl = { path = "../core" }
```

### `launcher-b/src/main.rs`

```rust
use gnoblin_core_impl as _;
fn main() {}
```

### `shell-min/Cargo.toml`

```toml
[package]
name = "shell-min"
version = "0.1.0"
edition = "2024"
build = "build.rs"

[lib]
name = "shell_min"
crate-type = ["cdylib"]

[features]
v1_1 = ["gnoblin-core/v1_1"]

[dependencies]
gnoblin-core = { path = "../sdk" }
```

### `shell-min/build.rs`

```rust
// What the SDK's build contributes to every shell: export only the entry symbol, fail on unresolved symbols.
fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    let map = format!("{out}/shell.map");
    std::fs::write(&map, "{ global: gnoshell_entry_v1; local: *; };\n").unwrap();
    println!("cargo:rustc-cdylib-link-arg=-Wl,--version-script={map}");
    println!("cargo:rustc-cdylib-link-arg=-Wl,-z,defs");
    println!("cargo:rerun-if-env-changed=GEN_LABEL");
    let label = std::env::var("GEN_LABEL").unwrap_or_else(|_| "shell-min".into());
    println!("cargo:rustc-env=SHELL_LABEL={label}");
}
```

### `shell-min/src/lib.rs`

```rust
use gnoblin_core::Gnoblin;

fn init(g: &Gnoblin) -> Result<(), String> {
    g.log("shell-min init (core only)");
    g.on_event("client_created", |p| println!("[shell-min] client_created {p}"));
    g.on_event("boom", |_| panic!("boom from the shell callback"));
    #[cfg(feature = "v1_1")]
    {
        g.log(&format!("uptime_ms via ABI 1.1 = {:?}", g.uptime_ms()));
    }
    Ok(())
}
gnoblin_core::export_shell!(env!("SHELL_LABEL"), init);
```

### `shell-hot/Cargo.toml`

```toml
[package]
name = "shell-hot"
version = "0.1.0"
edition = "2024"
build = "build.rs"

[lib]
name = "shell_hot"
crate-type = ["cdylib"]

[dependencies]
gnoblin-core = { path = "../sdk" }
```

### `shell-hot/src/lib.rs`

```rust
use gnoblin_core::Gnoblin;

fn init(g: &Gnoblin) -> Result<(), String> {
    let label = env!("SHELL_LABEL");
    if label == "bad" {
        return Err("simulated init failure (bad shell)".into());
    }
    g.log(&format!("shell-hot {label} init"));
    g.on_event("client_created", move |p| println!("[shell-hot {label}] client_created {p}"));
    Ok(())
}
gnoblin_core::export_shell!(env!("SHELL_LABEL"), init);
```

### `shell-bad/Cargo.toml`

```toml
[package]
name = "shell-bad"
version = "0.1.0"
edition = "2024"

[lib]
name = "shell_bad"
crate-type = ["cdylib"]

[dependencies]
# The mistake the SDK exists to prevent: depending on the implementation crate as an rlib.
gnoblin-core-impl = { path = "../core" }
```

### `shell-bad/src/lib.rs`

```rust
// Negative control: this shell compiles the core into itself.
pub fn keep() -> u32 {
    let (mut a, mut b) = (0, 0);
    unsafe { gnoblin_core_impl::gnoblin_abi_version(&mut a, &mut b) };
    a
}
#[unsafe(no_mangle)]
pub extern "C" fn gnoshell_entry_v1() -> u32 { keep() }
```

### `run.sh`

```bash
#!/bin/bash
# Spike driver, runs on the box. Output is recorded in design/modular-framework-spike.md.
set -u
export PATH=/usr/local/cargo/bin:/usr/bin:/bin RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo RUSTUP_TOOLCHAIN=1.99.1 CARGO_INCREMENTAL=0 CARGO_TERM_COLOR=never
S=/home/node/spike
cd $S
step() { echo; echo "=== $*"; }
B() { nice -n 19 cargo build -j2 "$@" 2>&1 | grep -v '^\s*\(Compiling\|Fresh\)' | tail -${TAILN:-6}; }
case "${1:-all}" in
tools)
  step "tools"
  for t in readelf nm objdump ldd python3 pkg-config cbindgen cc gcc patchelf; do printf '%s: ' $t; command -v $t || echo MISSING; done
  rustc --version; cargo --version; cc --version | head -1; readelf --version | head -1; python3 --version
  python3-config --includes 2>&1 | head -1; ls -d /usr/include/python3* 2>&1 | head -2; ls /usr/lib/x86_64-linux-gnu 2>/dev/null | grep -E '^libpython' | head -3
  pkg-config --list-all 2>/dev/null | grep -i -E 'python' | head -3; cat /etc/os-release | head -2
  ;;
build10)
  export CARGO_TARGET_DIR=/home/node/spike-target
  step "build core 1.0 + launcher + shell-min (cold)"
  touch core/src/lib.rs launcher/src/main.rs sdk/src/lib.rs abi/src/lib.rs shell-min/src/lib.rs shell-hot/src/lib.rs
  ( time B -p gnoblin-core-impl -p gnoblin -p shell-min ) 2>&1 | tail -12
  ;;
stage)
  export CARGO_TARGET_DIR=/home/node/spike-target
  step "stage: real core named by its SONAME next to the launcher (cargo xtask stage in the real tree)"
  rm -rf $S/stage; mkdir -p $S/stage
  T=$CARGO_TARGET_DIR/debug
  cp $T/libgnoblin_core_impl.so $S/stage/libgnoblin_core.so.1; cp $T/gnoblin $S/stage/; cp $T/libshell_min.so $S/stage/
  ls -l $S/stage
  ;;
check)
  cd $S/stage
  step "(1) core: SONAME, flags, version, exported symbols"
  readelf -d libgnoblin_core.so.1 | grep -E 'SONAME|FLAGS|NEEDED'
  echo "--- nm -D --defined-only:"; nm -D --defined-only libgnoblin_core.so.1
  echo "--- anything not gnoblin_*? (expect nothing):"; nm -D --defined-only libgnoblin_core.so.1 | awk '{print $3}' | grep -v '^gnoblin_' || echo none
  echo "--- symbol versions (readelf --dyn-syms):"; readelf --dyn-syms -W libgnoblin_core.so.1 | grep -E 'gnoblin_' | awk '{print $8, $9}'
  echo "--- version definitions:"; readelf -V libgnoblin_core.so.1 | grep -A3 'Version def' | grep -E 'Name|Rev' | head
  step "(2) launcher: NEEDED, RUNPATH"
  readelf -d gnoblin | grep -E 'NEEDED|RUNPATH|RPATH'
  echo "--- ldd gnoblin:"; ldd ./gnoblin | grep -E 'gnoblin|not found'
  echo "--- NEEDED gnoblin count: $(readelf -d gnoblin | grep -c 'libgnoblin_core.so.1')"
  step "(3) shell: NEEDED, defined symbols, undefined gnoblin_*"
  readelf -d libshell_min.so | grep -E 'NEEDED|SONAME'
  echo "--- nm -D --defined-only:"; nm -D --defined-only libshell_min.so
  echo "--- defined gnoblin_* (expect none): $(nm -D --defined-only libshell_min.so | grep -c ' gnoblin_')"
  echo "--- undefined gnoblin_*:"; nm -D --undefined-only libshell_min.so | grep gnoblin_
  echo "--- shell ldd:"; ldd libshell_min.so
  ;;
run)
  cd $S/stage
  step "(3) run: launcher dlopens the shell, one core copy"
  ./gnoblin run ./libshell_min.so; echo "exit=$?"
  ;;
panic)
  cd $S/stage
  step "(4) panic containment"
  ./gnoblin panic ./libshell_min.so; echo "exit=$?"
  ;;
hot)
  export CARGO_TARGET_DIR=/home/node/spike-target
  T=$CARGO_TARGET_DIR/debug
  step "(5) hot reload: build shell-hot as v1, v2, bad"
  mkdir -p $S/stage/shells
  for l in v1 v2 bad; do
    touch $S/shell-hot/src/lib.rs; GEN_LABEL=$l nice -n 19 cargo build -j2 -p shell-hot 2>&1 | tail -1
    mkdir -p $S/stage/shells/$l; cp $T/libshell_hot.so $S/stage/shells/$l/libshell_hot.so
  done
  cd $S/stage
  step "(5a) same path twice vs unique copies"
  ./gnoblin dup ./shells/v1/libshell_hot.so
  step "(5b) hot reload v1 -> v2, then failed reload of bad"
  ./gnoblin hot ./shells/v1/libshell_hot.so ./shells/v2/libshell_hot.so ./shells/bad/libshell_hot.so; echo "exit=$?"
  ;;
upgrade)
  step "(6) ABI upgrade: shell built against 1.0 runs on core 1.0 and a rebuilt core 1.1"
  export CARGO_TARGET_DIR=/home/node/spike-target
  T10=$CARGO_TARGET_DIR/debug
  export CARGO_TARGET_DIR=/home/node/spike-target11
  T11=$CARGO_TARGET_DIR/debug
  nice -n 19 cargo build -j2 --features gnoblin-core-impl/v1_1 -p gnoblin-core-impl -p shell-min --features shell-min/v1_1 2>&1 | tail -2
  rm -rf $S/up10 $S/up11 $S/up11shell; mkdir -p $S/up10 $S/up11 $S/up11shell
  cp $T10/libgnoblin_core_impl.so $S/up10/libgnoblin_core.so.1;  cp $T10/gnoblin $S/up10/; cp $T10/libshell_min.so $S/up10/shell10.so
  cp $T11/libgnoblin_core_impl.so $S/up11/libgnoblin_core.so.1;  cp $T10/gnoblin $S/up11/; cp $T10/libshell_min.so $S/up11/shell10.so
  cp $T11/libshell_min.so $S/up10/shell11.so; cp $T11/libshell_min.so $S/up11/shell11.so
  echo "--- core 1.0 table/abi:"; (cd $S/up10; ./gnoblin upgrade ./shell10.so)
  echo "--- SAME shell10.so (not rebuilt) on core 1.1:"; (cd $S/up11; ./gnoblin upgrade ./shell10.so)
  echo "--- shell built against 1.1 on core 1.1:"; (cd $S/up11; ./gnoblin upgrade ./shell11.so)
  echo "--- shell built against 1.1 on core 1.0 (degrades, no crash):"; (cd $S/up10; ./gnoblin upgrade ./shell11.so)
  echo "--- sha256 of launcher and shell10.so identical across both cores:"; sha256sum $S/up10/gnoblin $S/up11/gnoblin $S/up10/shell10.so $S/up11/shell10.so | cut -c1-16,65-
  echo "--- core sizes/ SONAME:"; readelf -d $S/up11/libgnoblin_core.so.1 | grep SONAME
  ;;
order)
  step "build-order experiments"
  for v in b c; do rm -rf /home/node/spike-target-$v; done
  echo "--- B: launcher-b has a normal dependency on the cdylib+rlib crate (cargo build -p gnoblin-b)"
  CARGO_TARGET_DIR=/home/node/spike-target-b nice -n 19 cargo build -j2 -p gnoblin-b 2>&1 | tail -2
  find /home/node/spike-target-b/debug -maxdepth 2 \( -name '*.so' -o -name 'libgnoblin_core_impl*' \) | sort
  echo "--- C: cargo build -p gnoblin only, cold target; time, then try to run"
  ( time CARGO_TARGET_DIR=/home/node/spike-target-c nice -n 19 cargo build -j2 -p gnoblin 2>&1 | tail -1 ) 2>&1 | grep -E 'Finished|real'
  find /home/node/spike-target-c/debug -maxdepth 1 -name '*.so'
  /home/node/spike-target-c/debug/gnoblin run x 2>&1 | head -2
  echo "--- cold full build of core+launcher+shell (time)"
  rm -rf /home/node/spike-target-d
  ( time CARGO_TARGET_DIR=/home/node/spike-target-d nice -n 19 cargo build -j2 -p gnoblin-core-impl -p gnoblin -p shell-min 2>&1 | tail -1 ) 2>&1 | grep -E 'Finished|real'
  echo "--- stub generation cost: cc of the stub (3 symbols)"
  ( time cc -shared -fPIC -Wl,-soname,libgnoblin_core.so.1 -o /tmp/stub.so $S/launcher/../abi/exports.txt 2>/dev/null ) 2>&1 | grep real
  ;;
syms)
  cd $S/stage; readelf --dyn-syms -W libgnoblin_core.so.1 | grep gnoblin_; readelf -V libgnoblin_core.so.1 | sed -n '/Version symbols/,/Version needs/p' | head -8
  ;;
nover)
  step "(1b) core WITHOUT our own version script (rustc's export control only)"
  export CARGO_TARGET_DIR=/home/node/spike-target-nv
  touch $S/core/src/lib.rs
  SPIKE_NO_VERSION_SCRIPT=1 nice -n 19 cargo build -j2 -p gnoblin-core-impl 2>&1 | tail -3
  L=$CARGO_TARGET_DIR/debug/libgnoblin_core_impl.so
  readelf -d $L | grep -E 'SONAME|FLAGS_1'; echo "--- nm -D --defined-only:"; nm -D --defined-only $L
  echo "--- non gnoblin_* defined:"; nm -D --defined-only $L | awk '{print $3}' | grep -vc '^gnoblin_'
  ;;
hyphen)
  step "can a [lib] name contain a hyphen?"
  rm -rf /home/node/spike-hy; mkdir -p /home/node/spike-hy/src; cd /home/node/spike-hy
  printf '[package]\nname="hy"\nversion="0.1.0"\nedition="2024"\n[lib]\nname="gnoblin-core"\ncrate-type=["cdylib"]\n' > Cargo.toml
  echo 'pub fn f(){}' > src/lib.rs
  CARGO_TARGET_DIR=/home/node/spike-hy/t cargo build 2>&1 | tail -4; ls /home/node/spike-hy/t/debug | head
  cd /; rm -rf /home/node/spike-hy
  ;;
negative)
  step "(3b) negative control: a shell that depends on the implementation crate as an rlib"
  export CARGO_TARGET_DIR=/home/node/spike-target
  touch $S/shell-bad/src/lib.rs
  nice -n 19 cargo build -j2 -p shell-bad 2>&1 | tail -1
  L=$CARGO_TARGET_DIR/debug/libshell_bad.so
  readelf -d $L | grep NEEDED
  echo "--- defined gnoblin_* (a second copy of the core's statics): $(nm -D --defined-only $L | grep -c ' gnoblin_')"; nm -D --defined-only $L | grep gnoblin_
  ;;
esac
```
