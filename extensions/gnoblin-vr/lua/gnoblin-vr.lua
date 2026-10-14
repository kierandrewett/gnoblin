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

--- Parse the "key=value" lines printed by gnoblin-vr --status.
function M.parse_status(text)
    local fields = {}
    for key, value in tostring(text):gmatch("([%w_]+)=([^\n]*)") do
        fields[key] = value
    end
    return fields
end

--- Build an outputs-style record for the headset from parsed status fields. The record has
--- kind = "xr" and per-eye views instead of a logical desktop rectangle. It is not part of
--- gnoblin.outputs.list(): the compositor's output list comes from Mutter's monitor manager.
function M.output_record(fields)
    local views = {}
    for index = 0, (tonumber(fields.view_count) or 0) - 1 do
        views[#views + 1] = {
            width = tonumber(fields["view_" .. index .. "_width"]),
            height = tonumber(fields["view_" .. index .. "_height"]),
        }
    end
    return {
        id = "xr:0",
        kind = "xr",
        name = "OpenXR headset",
        enabled = fields.state == "running",
        primary = false,
        state = fields.state or "absent",
        diagnostic = fields.diagnostic,
        views = views,
    }
end

--- Read the headset as an outputs-style record. callback receives the record, or nil and an
--- error when gnoblin-vr cannot be run.
function M.output(callback)
    local operation = M.status()
    operation:on_complete(function(value, err)
        if err or type(value) ~= "table" then
            callback(nil, err or "gnoblin-vr returned no status")
            return
        end
        callback(M.output_record(M.parse_status(value.stdout or "")))
    end)
end

return M
