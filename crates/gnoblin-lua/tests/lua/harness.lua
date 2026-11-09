-- tests/lua/harness.lua: a fresh node runtime over the fake core, and assertions. Plain Lua.
-- The runner sets GNOBLIN_LUA_DIR to data/lua/gnoblin and puts tests/lua on package.path.
local H = {}

-- Loads node.lua over a new fake core and returns core, host. Each case loads its own copy: node.lua keeps its
-- classes and the globals `root`, `Node`, `await` in the Lua state it runs in.
function H.fresh()
    local core, host = require("fake_core")()
    host.tick = assert(loadfile(GNOBLIN_LUA_DIR .. "/node.lua"))(core)
    return core, host
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
