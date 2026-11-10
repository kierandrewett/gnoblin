//! Runs the plain-Lua class system and the stock shell (`data/lua/gnoblin/node.lua`,
//! `data/lua/gnoblin/shell/`) under the vendored Lua 5.5, against the fake native core.
//!
//! The case files are the same ones `crates/gnoblin-lua/tests/lua/run.lua` runs under plain Lua
//! for quick iteration: `node_test.lua` and `shell_test.lua`. Each returns `{ name, fn }` pairs;
//! a case fails by raising. This runner reports every failing case, not only the first.

mod common;

#[test]
fn node_class_and_shell_cases_pass_on_lua_55() -> mlua::Result<()> {
    let lua = common::lua_state()?;
    common::run_case_files(
        &lua,
        &["node_test.lua", "shell_test.lua", "tiling_test.lua", "strip_test.lua"],
    )
}
