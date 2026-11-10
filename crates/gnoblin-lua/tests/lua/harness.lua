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
function H.load_shell()
    sys = sys or {
        spawn = function() end,
        date = function()
            return ""
        end,
    }
    dofile(GNOBLIN_LUA_DIR .. "/shell/ui.lua")
    H.floating = dofile(GNOBLIN_LUA_DIR .. "/shell/floating.lua")
    H.strip = dofile(GNOBLIN_LUA_DIR .. "/shell/strip.lua")
    dofile(GNOBLIN_LUA_DIR .. "/shell/init.lua")
    dofile(GNOBLIN_LUA_DIR .. "/shell/panel.lua")
    dofile(GNOBLIN_LUA_DIR .. "/shell/lock.lua")
    dofile(GNOBLIN_LUA_DIR .. "/shell/overview.lua")
    dofile(GNOBLIN_LUA_DIR .. "/shell/decorations.lua")
    dofile(GNOBLIN_LUA_DIR .. "/shell/launcher.lua")
    dofile(GNOBLIN_LUA_DIR .. "/shell/workspaces.lua")
end

-- The shell starts: every Screen gets `ready` first, so the Desk exists; then every Window, as a shell that comes
-- up after its windows have mapped would see them. The Desk is built with the screen's size, so it places windows
-- in its active workspace.
function H.start_shell(host)
    local screens, windows = {}, {}
    for _, id in ipairs(host.ids("Screen")) do
        screens[#screens + 1] = { type = "ready", target = id }
    end
    host.frame(0, screens)
    for _, id in ipairs(host.ids("Window")) do
        windows[#windows + 1] = { type = "ready", target = id }
    end
    host.frame(0, windows)
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
