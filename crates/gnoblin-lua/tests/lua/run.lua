-- Local runner for the Lua tests. From the repository root:
--   lua crates/gnoblin-lua/tests/lua/run.lua
-- Prints PASS or FAIL per case and exits non-zero if any case failed. The Rust runner (tests/node_class.rs) runs the
-- same case files on the vendored Lua 5.5.
local dir = arg[0]:match("^(.*)/[^/]*$") or "."
GNOBLIN_TEST_DIR = dir
GNOBLIN_LUA_DIR = dir .. "/../../../../data/lua/gnoblin"
package.path = dir .. "/?.lua;" .. package.path

local failed = 0
for _, file in ipairs({ "node_test", "shell_test", "floating_test", "tiling_test" }) do
    for _, c in ipairs(dofile(dir .. "/" .. file .. ".lua")) do
        local ok, err = pcall(c[2])
        print((ok and "PASS " or "FAIL ") .. c[1] .. (ok and "" or ": " .. tostring(err)))
        if not ok then
            failed = failed + 1
        end
    end
end
os.exit(failed == 0 and 0 or 1)
