-- Runs the headset policy against a stub gnoblin table: lua tests/vr-session.test.lua
local commands = {}
gnoblin = {
    commands = {
        run = function(argv)
            commands[#commands + 1] = table.concat(argv, " ")
            return { on_complete = function() end }
        end,
        capture = function(request)
            commands[#commands + 1] = table.concat(request.argv, " ")
            return { on_complete = function() end }
        end,
    },
}
package.path = "extensions/gnoblin-vr/lua/?.lua;" .. package.path
local session = require("gnoblin-vr-session")

local function has(text)
    for _, command in ipairs(commands) do
        if command:find(text, 1, true) then
            return true
        end
    end
    return false
end

local function expect(condition, message)
    if not condition then
        error("FAIL: " .. message, 2)
    end
    print("PASS: " .. message)
end

local index = {
    device_id = "drm:DP-2",
    backend = "monado",
    connection = "tethered",
    transport = "drm-lease",
    state = "ready",
    name = "Valve Index",
}
local quest = {
    device_id = "usb:2833:0186:X",
    backend = "wivrn",
    connection = "standalone",
    transport = "usb",
    state = "detected",
    name = "Quest 3",
}

expect(session.choose({ quest, index }) == index, "a ready tethered display beats a standalone headset with no stream")
expect(session.choose({}) == nil, "no headsets chooses nothing")

session.apply({ index })
expect(has("start gnoblin-vr-runtime@monado.service"), "a tethered headset starts the monado runtime")
expect(has("start gnoblin-vr@monado.service"), "and its presentation client")
expect(has("stop gnoblin-vr@wivrn.service"), "and stops the other backend's client")

commands = {}
session.apply({ quest })
expect(has("start gnoblin-vr-runtime@wivrn.service"), "a Quest on USB starts the wivrn streamer")
expect(has("gnoblin-vr --connect usb:2833:0186:X"), "and brings up the USB link")
expect(not has("start gnoblin-vr@wivrn.service"), "but waits for the stream before starting the client")

commands = {}
quest.state = "connected"
session.apply({ quest })
expect(has("start gnoblin-vr@wivrn.service"), "a connected stream starts the client")

commands = {}
session.apply({}, { streamer = "usb" })
expect(has("stop gnoblin-vr-runtime@wivrn.service"), "with streamer = usb and no headset the streamer stops")
expect(has("stop gnoblin-vr-runtime@monado.service"), "and so does the monado runtime")

commands = {}
session.apply({})
expect(
    has("start gnoblin-vr-runtime@wivrn.service"),
    "by default the streamer stays up so a headset can find the computer"
)
print("ok")
