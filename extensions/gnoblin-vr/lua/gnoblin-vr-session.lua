-- Default headset policy for the optional VR extension: plug a headset in and it comes up.
--
-- gnoblin-vr --watch publishes every headset it sees. The compositor turns that into xr outputs
-- and sends gnoblin.vr.headset-changed. This module reads those records and decides what to
-- start. Copy it next to gnoblin-vr.lua, then in the configuration:
--
--   require("gnoblin-vr-session").auto()
--
-- Replace or wrap apply() to change the policy; nothing here is known to the compositor.
local M = {}

local PRIORITY = { running = 4, connected = 3, ready = 2, detected = 1 }

local function systemctl(verb, unit)
    return gnoblin.commands.run({ "systemctl", "--user", verb, unit })
end

local function notify(summary, body)
    gnoblin.commands.run({ "notify-send", "-a", "Gnoblin VR", "-i", "video-display", summary, body or "" })
end

--- Pick the headset to use. A live session wins, then a connected stream, then a tethered
--- display that can be opened, then a standalone headset on USB that still needs its link.
function M.choose(headsets)
    local best
    for _, headset in ipairs(headsets or {}) do
        local rank = PRIORITY[headset.state] or 0
        if rank > 0 and (not best or rank > (PRIORITY[best.state] or 0)) then
            best = headset
        end
    end
    return best
end

local connecting = {}

--- Bring up the link to a standalone headset on USB (adb reverse, then start the app).
function M.connect_usb(headset)
    if connecting[headset.device_id] then
        return
    end
    connecting[headset.device_id] = true
    gnoblin.commands
        .capture({ argv = { "gnoblin-vr", "--connect", headset.device_id }, timeout = 30 })
        :on_complete(function(result, err)
            if err or not result or result.exit_code ~= 0 then
                local lines = tostring(result and result.stdout or err or "")
                local reason = lines:match("gnoblin%-vr: ([^\n]+)") or "the USB link could not be started"
                notify(headset.name .. " is not connected", reason)
            end
        end)
end

--- React to the current list of xr output records.
--- opts.streamer: "always" keeps the Wi-Fi/USB streamer running so a standalone headset can
--- find the computer; "usb" starts it only while a headset is plugged in.
function M.apply(headsets, opts)
    opts = opts or {}
    local streamer = opts.streamer or "always"
    local target = M.choose(headsets)
    local present = {}
    for _, headset in ipairs(headsets or {}) do
        present[headset.device_id] = true
    end
    for id in pairs(connecting) do
        if not present[id] then
            connecting[id] = nil
        end
    end

    if streamer == "always" or (target and target.backend == "wivrn") then
        systemctl("start", "gnoblin-vr-runtime@wivrn.service")
    else
        systemctl("stop", "gnoblin-vr-runtime@wivrn.service")
    end

    if not target then
        systemctl("stop", "gnoblin-vr@monado.service")
        systemctl("stop", "gnoblin-vr@wivrn.service")
        systemctl("stop", "gnoblin-vr-runtime@monado.service")
        return
    end

    local other = target.backend == "monado" and "wivrn" or "monado"
    systemctl("stop", "gnoblin-vr@" .. other .. ".service")
    if target.backend == "monado" then
        systemctl("start", "gnoblin-vr-runtime@monado.service")
    end

    if target.state == "detected" and target.transport == "usb" and target.backend == "wivrn" then
        M.connect_usb(target)
    elseif target.state == "ready" or target.state == "connected" or target.state == "running" then
        connecting[target.device_id] = nil
        systemctl("start", "gnoblin-vr@" .. target.backend .. ".service")
    end
end

--- Start following headsets. Call once from the configuration.
function M.auto(opts)
    gnoblin.on("gnoblin.vr.headset-changed", function(event)
        M.apply(event.headsets, opts)
    end)
    M.apply(M.current(), opts)
end

--- The xr records of gnoblin.outputs.list().
function M.current()
    local headsets = {}
    for _, output in ipairs(gnoblin.outputs.list()) do
        if output.kind == "xr" then
            headsets[#headsets + 1] = output
        end
    end
    return headsets
end

return M
