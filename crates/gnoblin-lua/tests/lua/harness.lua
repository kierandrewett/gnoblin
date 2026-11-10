-- tests/lua/harness.lua: a fresh node runtime over the fake core, and assertions. Plain Lua.
-- The runner sets GNOBLIN_LUA_DIR to data/lua/gnoblin and puts tests/lua on package.path.
local H = {}

-- Loads node.lua over a new fake core and returns core, host. Each case loads its own copy: node.lua keeps its
-- classes and the globals `root`, `Node`, `await` in the Lua state it runs in.
function H.fresh()
    local core, host = require("fake_core")()
    H.boot(core, host)
    return core, host
end

-- Loads node.lua over `core` as a fresh runtime. A restart does this again over the same core, so the native
-- tree (the windows, the screen, the seat) survives it, as it does in Gnoblin.
function H.boot(core, host)
    host.tick = assert(loadfile(GNOBLIN_LUA_DIR .. "/node.lua"))(core)
end

-- The stock shell: ui, then floating and strip (before init, which makes every Desk workspace a Floating), then
-- init. `sys` is the host's system table; the stub is inert. `H.floating` is the module floating.lua returns, so the
-- pure placement function is `H.floating.place`; `H.strip` is strip.lua's module (`H.strip.ring`, `H.strip.column`).
local shell_path_set = false
function H.load_shell()
    sys = sys or {
        spawn = function() end,
        date = function()
            return ""
        end,
    }
    if not shell_path_set then -- the shipped Lua root: require("gnoblin.shell") finds shell/init.lua
        package.path = GNOBLIN_LUA_DIR .. "/../?.lua;" .. GNOBLIN_LUA_DIR .. "/../?/init.lua;" .. package.path
        shell_path_set = true
    end
    -- A reload runs over a new node.lua runtime, so the shell's classes must be defined again.
    for name in pairs(package.loaded) do
        if name == "gnoblin.shell" or name:find("^gnoblin%.shell%.") then
            package.loaded[name] = nil
        end
    end
    require("gnoblin.shell")
    H.floating = require("gnoblin.shell.floating")
    H.strip = require("gnoblin.shell.strip")
    Focus = require("gnoblin.shell.focus")
    Snap = require("gnoblin.shell.snap")
    Rules = require("gnoblin.shell.rules")
end

-- The shell starts: its first frame emits `ready` for every node already in the tree, parents first, so every
-- Screen gets `ready` before the Windows it places. The Desk is built with the screen's size, so it places windows
-- in its active workspace.
function H.start_shell(host)
    host.frame(0, {})
end

-- One frame at time `now` with the given events. Returns the host's wake time.
function H.frame(host, now, events)
    return host.frame(now or 0, events or {})
end

function H.check(cond, msg)
    if not cond then
        error(msg or "check failed", 2)
    end
end

-- Element-wise identity of two lists.
function H.same(got, want)
    H.check(#got == #want, ("got %d elements, want %d"):format(#got, #want))
    for i = 1, #want do
        H.check(got[i] == want[i], ("element %d differs"):format(i))
    end
end

return H
