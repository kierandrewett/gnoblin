//! Runs the plain-Lua class system and the stock shell (`data/lua/gnoblin/node.lua`,
//! `data/lua/gnoblin/shell/`) under the vendored Lua 5.5, against the fake native core.
//!
//! The case files are the same ones `crates/gnoblin-lua/tests/lua/run.lua` runs under plain Lua
//! for quick iteration: `node_test.lua` and `shell_test.lua`. Each returns `{ name, fn }` pairs;
//! a case fails by raising. This runner reports every failing case, not only the first.

use mlua::{Function, Lua, Table};
use std::path::PathBuf;

const LUA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/lua/gnoblin");
const TEST_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/lua");

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

#[test]
fn node_class_and_shell_cases_pass_on_lua_55() -> mlua::Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("GNOBLIN_LUA_DIR", LUA_DIR)?;
    globals.set("GNOBLIN_TEST_DIR", TEST_DIR)?;
    let package: Table = globals.get("package")?;
    let path: String = package.get("path")?;
    package.set("path", format!("{TEST_DIR}/?.lua;{path}"))?;

    let mut failures = Vec::new();
    let mut count = 0;
    run_file(&lua, "node_test.lua", &mut failures, &mut count)?;
    run_file(&lua, "shell_test.lua", &mut failures, &mut count)?;

    assert!(count > 0, "no Lua cases were loaded");
    assert!(
        failures.is_empty(),
        "{} of {count} Lua cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}
