-- Optional helper module. Load from user Lua configuration with require().
local M = {}

function M.start()
    return gnoblin.commands.run({ "systemctl", "--user", "start", "gnoblin-vr.service" })
end

function M.stop()
    return gnoblin.commands.run({ "systemctl", "--user", "stop", "gnoblin-vr.service" })
end

function M.status()
    return gnoblin.commands.capture({ argv = { "gnoblin-vr", "--status" } })
end

return M
