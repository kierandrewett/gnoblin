//! Plumbing the Lua test runners share: a Lua state with the stock tree and the case directory on
//! its path, and the runner for case files. Each integration test is its own crate, so each one
//! pulls this in with `mod common;`.

use mlua::{Function, Lua, Table};
use std::path::PathBuf;

pub const LUA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/lua/gnoblin");
pub const TEST_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/lua");

/// A Lua state with the stock tree (`GNOBLIN_LUA_DIR`) and the case directory on its path.
pub fn lua_state() -> mlua::Result<Lua> {
    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("GNOBLIN_LUA_DIR", LUA_DIR)?;
    globals.set("GNOBLIN_TEST_DIR", TEST_DIR)?;
    let package: Table = globals.get("package")?;
    let path: String = package.get("path")?;
    package.set("path", format!("{TEST_DIR}/?.lua;{path}"))?;
    Ok(lua)
}

/// Runs every case of one case file, appending a line per failing case to `failures`.
fn run_file(lua: &Lua, file: &str, failures: &mut Vec<String>, count: &mut usize) -> mlua::Result<()> {
    let path = PathBuf::from(TEST_DIR).join(file);
    let source = std::fs::read(&path).map_err(mlua::Error::external)?;
    let cases: Table = lua.load(source).set_name(path.display().to_string()).eval()?;
    for case in cases.sequence_values::<Table>() {
        let case = case?;
        let name: String = case.get(1)?;
        let body: Function = case.get(2)?;
        *count += 1;
        if let Err(err) = body.call::<()>(()) {
            failures.push(format!("{name}: {err}"));
        }
    }
    Ok(())
}

/// Runs the named case files in order and asserts that every case passes. Reports every failing
/// case, not only the first.
pub fn run_case_files(lua: &Lua, files: &[&str]) -> mlua::Result<()> {
    let mut failures = Vec::new();
    let mut count = 0;
    for file in files {
        run_file(lua, file, &mut failures, &mut count)?;
    }
    assert!(count > 0, "no Lua cases were loaded");
    assert!(
        failures.is_empty(),
        "{} of {count} Lua cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}
